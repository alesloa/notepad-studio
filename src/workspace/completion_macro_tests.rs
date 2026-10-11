// Real replay handlers on GPUI TestWindow; no global recording or user persistence.
use super::Workspace;
use crate::{
    actions::*,
    macros::{MacroEvent, MacroRecorder, completion_event},
    tab::Tab,
};
use gpui::{AppContext, Context, TestAppContext, VisualContext, Window, WindowAppearance};

fn with_workspace(
    text: &str,
    test: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    with_workspace_window(text, true, test);
}

fn with_workspace_window(
    text: &str,
    active: bool,
    test: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    let mut cx = TestAppContext::single();
    let text = text.to_string();
    let entity = cx.new(move |cx| {
        let mut w = Workspace::new(
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            1.,
            WindowAppearance::Dark,
        );
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(&text);
        tab.selection.collapse_to(text.len());
        w.tabs = vec![tab];
        w.active = 0;
        w.window_closed = true;
        w
    });
    {
        let cx = cx.add_empty_window();
        if active {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
        }
        cx.update_window_entity(&entity, |w, window, cx| {
            window.focus(&w.focus_handle, cx);
            test(w, window, cx);
            w.window_closed = true;
        });
    }
    cx.quit();
}

#[test]
fn replayed_completion_replaces_the_recorded_prefix_and_restores_caret_on_undo() {
    for (prefix, candidate) in [
        ("an", "android"),
        ("AN", "android"),
        ("café", "caféine"),
        ("中文", "中文词"),
        ("", "android"),
    ] {
        with_workspace("\u{FEFF}before\r\n", |w, window, cx| {
            w.replay_macro(&[MacroEvent::InsertText(prefix.to_string())], 1, window, cx);
            let caret = w.selection().head;
            let before = w.buffer().rope.to_string();
            w.replay_macro(&[completion_event(prefix, candidate)], 1, window, cx);
            assert_eq!(
                w.buffer().rope.to_string(),
                format!("\u{FEFF}before\r\n{candidate}")
            );
            assert!(w.selection().is_empty());
            w.undo(&EditUndo, window, cx);
            assert_eq!(w.buffer().rope.to_string(), before);
            assert!(w.selection().is_empty());
            assert_eq!(w.selection().head, caret);
            w.redo(&EditRedo, window, cx);
            assert_eq!(
                w.buffer().rope.to_string(),
                format!("\u{FEFF}before\r\n{candidate}")
            );
        });
    }
}

#[test]
fn completion_replay_requires_the_matching_prefix_and_editor_context() {
    for bad in 0..8 {
        with_workspace(if bad == 0 { "ban" } else { "an" }, |w, window, cx| {
            match bad {
                1 => w.active_tab_mut().buffer.read_only = true,
                2 => w.active_tab_mut().selection.anchor = 0,
                3 => window.focus(&w.find_focus, cx),
                4 => w.active_tab_mut().selection.collapse_to(usize::MAX),
                5 => w.active_tab_mut().selection.collapse_to(1),
                7 => {
                    w.active_tab_mut().block_selection = Some(crate::buffer::BlockSelection {
                        anchor_line: 0,
                        head_line: 0,
                        anchor_col: 2,
                        head_col: 2,
                        is_dragging: false,
                    })
                }
                _ => {}
            }
            let before = w.buffer().rope.to_string();
            let event = if bad == 6 {
                completion_event("zz", "zzebra")
            } else {
                completion_event("an", "android")
            };
            w.replay_macro(&[event], 1, window, cx);
            assert_eq!(
                w.buffer().rope.to_string(),
                before,
                "invalid replay context {bad}"
            );
            assert!(w.active_tab_mut().undo_stack.is_empty());
        });
    }
}

#[test]
fn recorded_typing_and_completion_replay_without_a_popup() {
    with_workspace("", |w, window, cx| {
        let mut recorder = MacroRecorder::recording_for_test();
        recorder.record_event(MacroEvent::InsertText("a".into()));
        recorder.record_event(MacroEvent::InsertText("n".into()));
        recorder.record_event(completion_event("an", "android"));
        assert!(
            matches!(recorder.captured_events(), [MacroEvent::InsertText(prefix), MacroEvent::CompleteWord { .. }] if prefix == "an")
        );
        recorder.replaying = true;
        recorder.record_event(completion_event("an", "android"));
        recorder.replaying = false;
        recorder.recording = false;
        recorder.record_event(completion_event("an", "android"));
        assert_eq!(
            recorder.captured_events().len(),
            2,
            "disabled/replaying captures nothing"
        );
        w.replay_macro(recorder.captured_events(), 1, window, cx);
        assert_eq!(w.buffer().rope.to_string(), "android");
        assert!(w.autocomplete.is_none());
    });
}

#[test]
fn live_completion_and_recorded_replay_have_the_same_undo_caret() {
    with_workspace("android andromeda\nan", |w, window, cx| {
        let initial_caret = w.selection().head;
        w.autocomplete_word(&EditAutocompleteWord, window, cx);
        assert!(w.accept_autocomplete_with("android", window, cx));
        let live = w.buffer().rope.to_string();
        w.undo(&EditUndo, window, cx);
        assert!(w.selection().is_empty());
        assert_eq!(w.selection().head, initial_caret);
        w.replay_macro(&[completion_event("an", "android")], 1, window, cx);
        assert_eq!(w.buffer().rope.to_string(), live);
        w.undo(&EditUndo, window, cx);
        assert!(w.selection().is_empty());
        assert_eq!(w.selection().head, initial_caret);
    });
}


#[test]
fn completion_replay_does_not_edit_hidden_preview_source() {
    for view in [
        crate::tab::TabViewMode::MarkdownPreview,
        crate::tab::TabViewMode::CsvPreview,
        crate::tab::TabViewMode::RichDocView,
    ] {
        with_workspace("an", |w, window, cx| {
            w.active_tab_mut().view_mode = view;
            w.replay_macro(&[completion_event("an", "android")], 1, window, cx);
            assert_eq!(w.buffer().rope.to_string(), "an", "preview {view:?}");
            assert!(w.active_tab_mut().undo_stack.is_empty());
        });
    }
}

#[test]
fn completion_replay_does_not_edit_an_inactive_window() {
    with_workspace_window("an", false, |w, window, cx| {
        assert!(!window.is_window_active());
        w.replay_macro(&[completion_event("an", "android")], 1, window, cx);
        assert_eq!(w.buffer().rope.to_string(), "an");
        assert!(w.active_tab_mut().undo_stack.is_empty());
    });
}
