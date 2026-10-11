// Actual playback/action handlers on synthetic GPUI windows, without user persistence.
use super::Workspace;
use crate::{
    actions::*,
    buffer::LoadProgress,
    macros::{MacroEvent, override_playback_for_test},
    settings::{Settings, override_for_test},
    tab::{InputFocus, Tab, TabViewMode},
};
use gpui::{AppContext, Context, TestAppContext, VisualContext, Window, WindowAppearance};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

const TEXT: &str = "\u{feff}café\r\nsource";

pub(super) fn with_workspace(
    active: bool,
    test: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    let _settings = override_for_test(Settings::default());
    let _execution = crate::macros::override_execution_for_test(false);
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let events = vec![MacroEvent::InsertText("é".into())];
    let _macro = override_playback_for_test(
        events.clone(),
        BTreeMap::from([("synthetic".into(), events)]),
    );
    let mut cx = TestAppContext::single();
    let entity = cx.new(|cx| {
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
        tab.buffer.rope = ropey::Rope::from_str(TEXT);
        tab.selection.collapse_to(TEXT.len());
        w.tabs = vec![tab];
        w.active = 0;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink);
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
        });
    }
    cx.quit();
}

fn attach_rich_payload(w: &mut Workspace) {
    let path = std::env::temp_dir().join(format!(
        "notepad-studio-macro-context-{}.rtf",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, br"{\rtf1\ansi\b seed}").expect("write neutral RTF");
    let mut buffer = crate::io::read_buffer(&path).expect("read neutral RTF");
    std::fs::remove_file(path).expect("remove neutral RTF");
    w.active_tab_mut().buffer.rich_doc = buffer.rich_doc.take();
}

fn assert_unchanged(w: &Workspace, selection: crate::buffer::Selection, generation: u64) {
    assert_eq!(
        w.buffer().rope.to_string(),
        TEXT,
        "playback must leave source unchanged"
    );
    assert_eq!(w.selection().head, selection.head);
    assert_eq!(w.selection().anchor, selection.anchor);
    assert!(w.tabs[0].undo_stack.is_empty());
    assert!(w.tabs[0].redo_stack.is_empty());
    assert!(!w.buffer().dirty);
    assert_eq!(w.autosave_generation, generation);
    assert!(
        w.status_message.as_ref().is_some_and(
            |message| !message.starts_with("Played macro") && !message.starts_with("Ran macro")
        ),
        "refusal must survive the action handler"
    );
}

#[test]
fn playback_rejects_preview_and_rendered_document_contexts() {
    for view in [
        TabViewMode::MarkdownPreview,
        TabViewMode::CsvPreview,
        TabViewMode::RichDocView,
    ] {
        with_workspace(true, |w, window, cx| {
            w.active_tab_mut().view_mode = view;
            let rich_before = if view == TabViewMode::RichDocView {
                attach_rich_payload(w);
                let focus = w.ensure_rich_doc_body_focus(cx);
                window.focus(&focus, cx);
                w.buffer()
                    .rich_doc
                    .as_ref()
                    .map(|payload| payload.ast.clone())
            } else {
                None
            };
            let selection = w.selection();
            let generation = w.autosave_generation;
            w.macro_playback(&MacroPlayback, window, cx);
            assert_unchanged(w, selection, generation);
            assert_eq!(
                w.buffer()
                    .rich_doc
                    .as_ref()
                    .map(|payload| payload.ast.clone()),
                rich_before
            );
        });
    }
}

#[test]
fn playback_rejects_focused_input_fields() {
    for focus in [
        InputFocus::FindQuery,
        InputFocus::Replacement,
        InputFocus::Goto,
        InputFocus::MacroPrompt,
    ] {
        with_workspace(true, |w, window, cx| {
            w.find_state.query = "query".into();
            w.find_state.replacement = "replacement".into();
            w.goto_state.text = "10".into();
            if focus == InputFocus::MacroPrompt {
                w.open_macro_prompt(
                    crate::find_state::MacroPromptMode::RunCount,
                    "2",
                    window,
                    cx,
                );
            } else {
                w.focus_input(focus, window, cx);
            }
            let selection = w.selection();
            let generation = w.autosave_generation;
            w.macro_playback(&MacroPlayback, window, cx);
            assert_unchanged(w, selection, generation);
            assert_eq!(w.find_state.query, "query");
            assert_eq!(w.find_state.replacement, "replacement");
            assert_eq!(w.goto_state.text, "10");
            if focus == InputFocus::MacroPrompt {
                assert_eq!(w.macro_prompt.as_ref().expect("prompt remains").text, "2");
            }
        });
    }
}

#[test]
fn playback_rejects_inactive_or_unfocused_editor_windows() {
    for active in [false, true] {
        with_workspace(active, |w, window, cx| {
            if active {
                window.focus(&cx.focus_handle(), cx);
            }
            let selection = w.selection();
            let generation = w.autosave_generation;
            w.macro_playback(&MacroPlayback, window, cx);
            assert_unchanged(w, selection, generation);
        });
    }
}

#[test]
fn playback_rejects_read_only_buffers() {
    with_workspace(true, |w, window, cx| {
        w.active_tab_mut().buffer.read_only = true;
        let selection = w.selection();
        let generation = w.autosave_generation;
        w.macro_playback(&MacroPlayback, window, cx);
        assert_unchanged(w, selection, generation);
        assert_eq!(w.status_message.as_deref(), Some("Buffer is read-only"));
    });
}

#[test]
fn playback_rejects_loading_buffers() {
    with_workspace(true, |w, window, cx| {
        w.active_tab_mut().buffer.loading = Some(LoadProgress {
            bytes_read: 1,
            total_bytes: 2,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let selection = w.selection();
        let generation = w.autosave_generation;
        w.macro_playback(&MacroPlayback, window, cx);
        assert_unchanged(w, selection, generation);
    });
}

#[test]
fn playback_leaves_csv_grid_and_cell_drafts_untouched() {
    use crate::ui::csv_doc::{CsvModel, model::CsvCellInput};
    for editing in [false, true] {
        with_workspace(true, |w, window, cx| {
            w.active_tab_mut().view_mode = TabViewMode::CsvPreview;
            let tab_id = w.tabs[0].session_id;
            let mut model = CsvModel::empty(b',');
            model.rows = vec![vec!["cell".into()]];
            model.active_cell = Some((0, 0));
            let focus = cx.focus_handle();
            if editing {
                model.editing_cell = Some((0, 0));
                model.cell_input = Some(CsvCellInput::new(focus.clone(), "draft".into()));
            } else {
                model.grid_focus = Some(focus.clone());
            }
            w.csv_models.insert(tab_id, model);
            window.focus(&focus, cx);
            let selection = w.selection();
            let generation = w.autosave_generation;
            w.macro_playback(&MacroPlayback, window, cx);
            assert_unchanged(w, selection, generation);
            let model = w.csv_models.get(&tab_id).expect("CSV model remains");
            assert_eq!(model.rows, vec![vec!["cell"]]);
            assert_eq!(model.active_cell, Some((0, 0)));
            assert!(model.undo_stack.is_empty());
            assert!(model.redo_stack.is_empty());
            if editing {
                assert_eq!(
                    model.cell_input.as_ref().expect("draft remains").buffer,
                    "draft"
                );
            }
        });
    }
}

#[test]
fn playback_handles_missing_active_tab_without_panicking() {
    for empty in [false, true] {
        with_workspace(true, |w, window, cx| {
            if empty {
                w.tabs.clear();
            } else {
                w.active = usize::MAX;
            }
            let generation = w.autosave_generation;
            w.macro_playback(&MacroPlayback, window, cx);
            assert_eq!(w.autosave_generation, generation);
            assert!(
                w.status_message
                    .as_ref()
                    .is_some_and(|message| !message.starts_with("Played macro"))
            );
        });
    }
}

#[test]
fn playback_runs_in_source_and_raw_views() {
    for rich_raw in [false, true] {
        with_workspace(true, |w, window, cx| {
            if rich_raw {
                attach_rich_payload(w);
            }
            let rich_before = w
                .buffer()
                .rich_doc
                .as_ref()
                .map(|payload| payload.ast.clone());
            w.macro_playback(&MacroPlayback, window, cx);
            assert_eq!(w.buffer().rope.to_string(), format!("{TEXT}é"));
            assert_eq!(w.selection().head, TEXT.len() + "é".len());
            assert!(w.selection().is_empty());
            assert!(w.buffer().dirty);
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert_eq!(
                w.buffer()
                    .rich_doc
                    .as_ref()
                    .map(|payload| payload.ast.clone()),
                rich_before
            );
        });
    }
}

#[test]
fn saved_macro_and_run_count_keep_refusal_status() {
    with_workspace(true, |w, window, cx| {
        w.active_tab_mut().view_mode = TabViewMode::MarkdownPreview;
        let selection = w.selection();
        let generation = w.autosave_generation;
        w.macro_run_saved(
            &MacroRunSaved {
                name: "synthetic".into(),
            },
            window,
            cx,
        );
        assert_unchanged(w, selection, generation);
        w.macro_run_multiple_times(&MacroRunMultipleTimes, window, cx);
        assert!(w.macro_prompt.is_some());
        w.macro_prompt.as_mut().expect("count prompt").text = "2".into();
        w.commit_macro_prompt(window, cx);
        assert!(w.macro_prompt.is_none());
        assert_unchanged(w, selection, generation);
    });
}

#[test]
fn run_count_hands_focus_back_to_the_source_editor() {
    with_workspace(true, |w, window, cx| {
        w.macro_run_multiple_times(&MacroRunMultipleTimes, window, cx);
        assert!(w.input_focus(window) == InputFocus::MacroPrompt);
        w.macro_prompt.as_mut().expect("count prompt").text = "2".into();
        w.commit_macro_prompt(window, cx);
        assert!(w.input_focus(window) == InputFocus::Editor);
        assert!(w.focus_handle.is_focused(window));
        assert_eq!(w.buffer().rope.to_string(), format!("{TEXT}éé"));
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
    });
}

#[test]
fn playback_tab_and_enter_do_not_accept_an_open_completion_popup() {
    for event in [MacroEvent::InsertTab, MacroEvent::InsertNewline] {
        with_workspace(true, |w, window, cx| {
            let source = "android\r\nan";
            w.active_tab_mut().buffer.rope = ropey::Rope::from_str(source);
            w.active_tab_mut().selection.collapse_to(source.len());
            w.autocomplete_word(&EditAutocompleteWord, window, cx);
            assert!(w.autocomplete.is_some(), "synthetic completion opens");
            let newline = matches!(event, MacroEvent::InsertNewline);
            let _macro = override_playback_for_test(vec![event.clone()], BTreeMap::new());
            w.macro_playback(&MacroPlayback, window, cx);
            assert!(w.autocomplete.is_none());
            let suffix = if newline { "\n" } else { "    " }; // Rope edits use LF internally.
            assert_eq!(w.buffer().rope.to_string(), format!("{source}{suffix}"));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
    }
}

#[test]
fn playback_callers_preserve_invalid_completion_refusals() {
    let event = MacroEvent::CompleteWord {
        prefix: "zz".into(),
        candidate: "zzebra".into(),
    };
    for caller in 0..3 {
        with_workspace(true, |w, window, cx| {
            let _macro = override_playback_for_test(
                vec![event.clone()],
                BTreeMap::from([("synthetic".into(), vec![event.clone()])]),
            );
            let selection = w.selection();
            let generation = w.autosave_generation;
            match caller {
                0 => w.macro_playback(&MacroPlayback, window, cx),
                1 => w.macro_run_saved(
                    &MacroRunSaved {
                        name: "synthetic".into(),
                    },
                    window,
                    cx,
                ),
                _ => {
                    w.macro_run_multiple_times(&MacroRunMultipleTimes, window, cx);
                    w.macro_prompt.as_mut().expect("count prompt").text = "2".into();
                    w.commit_macro_prompt(window, cx);
                }
            }
            assert_unchanged(w, selection, generation);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Macro completion skipped: prefix changed or event is invalid")
            );
        });
    }
}

#[test]
fn playback_stops_after_a_failed_event_without_repeating_prior_edits() {
    with_workspace(true, |w, window, cx| {
        let _macro = override_playback_for_test(
            vec![
                MacroEvent::InsertText("é".into()),
                MacroEvent::CompleteWord {
                    prefix: "zz".into(),
                    candidate: "zzebra".into(),
                },
                MacroEvent::InsertText("later".into()),
            ],
            BTreeMap::new(),
        );
        w.macro_run_multiple_times(&MacroRunMultipleTimes, window, cx);
        w.macro_prompt.as_mut().expect("count prompt").text = "2".into();
        w.commit_macro_prompt(window, cx);
        assert_eq!(w.buffer().rope.to_string(), format!("{TEXT}é"));
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
        assert_eq!(
            w.status_message.as_deref(),
            Some(
                "Macro stopped after 1 event: Macro completion skipped: prefix changed or event is invalid"
            )
        );
    });
}
