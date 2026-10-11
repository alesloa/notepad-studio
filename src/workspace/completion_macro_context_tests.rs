// Actual completion acceptance, recorder and replay; per-thread fixtures only.
use crate::workspace::autocomplete::completion_api_tests::{fixture, preferences, press};
use crate::{actions::*, macros::MacroEvent, workspace::Workspace};
use gpui::{Context, TestAppContext, VisualContext, Window};

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedEvents {
    events: Vec<MacroEvent>,
}

fn replay_and_compare(
    w: &mut Workspace,
    completion: (&str, &str, &str, &str),
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let (initial, live, prefix, candidate) = completion;
    assert!(!crate::macros::stop_recording());
    let events = crate::macros::current();
    assert!(
        matches!(events.as_slice(), [MacroEvent::CompleteWord { prefix: p, candidate: c }]
        if p == prefix && c == candidate)
    );
    let serialized = toml::to_string(&SavedEvents { events }).unwrap();
    let saved: SavedEvents = toml::from_str(&serialized).unwrap();
    assert_eq!(w.buffer().rope.to_string(), live);
    assert_eq!(
        (w.selection().anchor, w.selection().head),
        (live.len(), live.len())
    );
    assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
    w.undo(&EditUndo, window, cx);
    assert_eq!(w.buffer().rope.to_string(), initial);
    assert_eq!(
        (w.selection().anchor, w.selection().head),
        (initial.len(), initial.len())
    );
    assert!(w.active_tab_mut().undo_stack.is_empty());
    assert_eq!(w.active_tab_mut().redo_stack.len(), 1);
    w.completion_api = Default::default();
    assert!(w.loaded_api_snapshot(w.buffer().language).is_none());
    w.replay_macro(&saved.events, 1, window, cx);
    assert_eq!(
        w.buffer().rope.to_string(),
        live,
        "recorded live completion must replay identically"
    );
    assert_eq!(
        (w.selection().anchor, w.selection().head),
        (live.len(), live.len())
    );
    assert_eq!(w.active_tab_mut().undo_stack.len(), 1);
    assert!(w.active_tab_mut().redo_stack.is_empty());
    w.undo(&EditUndo, window, cx);
    assert_eq!(w.buffer().rope.to_string(), initial);
    assert_eq!(
        (w.selection().anchor, w.selection().head),
        (initial.len(), initial.len())
    );
    w.redo(&EditRedo, window, cx);
    assert_eq!(w.buffer().rope.to_string(), live);
    assert_eq!(
        (w.selection().anchor, w.selection().head),
        (live.len(), live.len())
    );
}

#[test]
fn recorded_api_additional_word_character_completion_replays_like_live_acceptance() {
    for candidate in ["object.method", "object.method member"] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings {
            autocompletion_brief: true,
            ..preferences()
        });
        let _recording = crate::macros::override_execution_for_test(false);
        let directory = crate::persist::TempDir::new("completion-macro-word-char");
        std::fs::write(directory.join("normal.xml"),
        format!("<NotepadPlus><AutoComplete><Environment additionalWordChar='.'/><KeyWord name='{candidate}' func='yes'/></AutoComplete></NotepadPlus>")).unwrap();
        let _directory = crate::workspace::completion_api::override_directory_for_test(
            directory.path().to_owned(),
        );
        let initial = "\u{FEFF}日本\r\nobject.me";
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, initial);
        visual.update_window_entity(&workspace, |w, _, _| {
            w.active_tab_mut().buffer.language = None
        });
        press(visual, "EditAutocompleteFunc");
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            let popup = w.autocomplete.as_ref().unwrap();
            assert_eq!(popup.replace_start, "\u{FEFF}日本\r\n".len());
            assert_eq!(popup.candidates, [candidate]);
        });
        assert!(crate::macros::start_recording());
        visual.simulate_keystrokes("tab");
        std::fs::remove_file(directory.join("normal.xml")).unwrap();
        let live = format!("\u{FEFF}日本\r\n{candidate}");
        visual.update_window_entity(&workspace, |w, window, cx| {
            replay_and_compare(w, (initial, &live, "object.me", candidate), window, cx)
        });
        cx.quit();
    }
}

#[test]
fn recorded_initial_manual_function_non_prefix_member_replays_like_live_acceptance() {
    let _settings = crate::settings::override_for_test(preferences());
    let _recording = crate::macros::override_execution_for_test(false);
    let initial = "\u{FEFF}日本\r\nret";
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, initial);
    press(visual, "EditAutocompleteFunc");
    press(visual, "MoveLineStart");
    visual.update(|_, cx| {
        let popup = workspace.read(cx).autocomplete.as_ref().unwrap();
        assert_eq!(
            popup.candidates.get(popup.selected).map(String::as_str),
            Some("Arc")
        );
    });
    assert!(crate::macros::start_recording());
    visual.simulate_keystrokes("enter");
    visual.update_window_entity(&workspace, |w, window, cx| {
        replay_and_compare(
            w,
            (initial, "\u{FEFF}日本\r\nArc", "ret", "Arc"),
            window,
            cx,
        )
    });
    cx.quit();
}

#[test]
fn replayed_api_completion_refuses_suffixes_changed_text_and_invalid_editor_context() {
    for refusal in 0..8 {
        let _settings = crate::settings::override_for_test(preferences());
        let _recording = crate::macros::override_execution_for_test(false);
        let initial = match refusal {
            0 => "\u{FEFF}日本\r\n.object.me",
            1 => "\u{FEFF}日本\r\nxobject.me",
            2 => "\u{FEFF}日本\r\nobject.mx",
            _ => "\u{FEFF}日本\r\nobject.me",
        };
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, initial);
        visual.update_window_entity(&workspace, |w, window, cx| {
            match refusal {
                3 => w
                    .active_tab_mut()
                    .selection
                    .collapse_to("\u{FEFF}日".len() - 1),
                4 => w.active_tab_mut().selection.anchor = 0,
                5 => w.active_tab_mut().buffer.read_only = true,
                6 => window.focus(&w.find_focus, cx),
                7 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::MarkdownPreview,
                _ => {}
            }
            let selection = w.selection();
            w.replay_macro(
                &[crate::macros::completion_event(
                    "object.me",
                    "object.method",
                )],
                1,
                window,
                cx,
            );
            assert_eq!(w.buffer().rope.to_string(), initial, "refusal {refusal}");
            assert_eq!(
                (w.selection().anchor, w.selection().head),
                (selection.anchor, selection.head)
            );
            assert!(w.active_tab_mut().undo_stack.is_empty());
            assert!(w.active_tab_mut().redo_stack.is_empty());
        });
        cx.quit();
    }
}

#[test]
fn persisted_completion_range_rejects_controls_and_whitespace_prefixes() {
    for (source, prefix, candidate) in [
        ("日本\r\nobject.me", "\r\nobject.me", "other"),
        ("日本 object.me", " object.me", "other"),
        ("日本\tobject.me", "\tobject.me", "other"),
        ("日本\0object.me", "\0object.me", "other"),
        ("object.me", "object.me", "other\nline"),
        ("object.me", "object.me", "other\0line"),
    ] {
        let rope = ropey::Rope::from_str(source);
        assert_eq!(
            super::completion_range(&rope, prefix, candidate, rope.len_bytes()),
            None
        );
    }
    let rope = ropey::Rope::from_str("object.me");
    assert_eq!(
        super::completion_range(&rope, "object.me", "other member", rope.len_bytes()),
        Some(0..9)
    );
}
