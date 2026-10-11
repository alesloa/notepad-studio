use crate::{actions::*, buffer::BlockSelection, macros::{self, MacroEvent},
    settings::Settings, tab::{Tab, TabViewMode}, workspace::Workspace};
use gpui::{Action, ElementInputHandler, Entity, InputHandler, MenuItem, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};
use std::collections::BTreeMap;

const SOURCE: &str = "bb\nA\nddd\nc";

const CASES: [(&str, &str, &str, &str); 6] = [
    ("Sort Lines Lex. Ascending Ignoring Case", "EditSortLinesLexIgnoreCaseAsc", "IDM_EDIT_SORTLINES_LEXICO_CASE_INSENS_ASCENDING", "A\nbb\nc\nddd"),
    ("Sort Lines Lex. Descending Ignoring Case", "EditSortLinesLexIgnoreCaseDesc", "IDM_EDIT_SORTLINES_LEXICO_CASE_INSENS_DESCENDING", "ddd\nc\nbb\nA"),
    ("Sort Lines By Length Ascending", "EditSortLinesLengthAsc", "IDM_EDIT_SORTLINES_LENGTH_ASCENDING", "A\nc\nbb\nddd"),
    ("Sort Lines By Length Descending", "EditSortLinesLengthDesc", "IDM_EDIT_SORTLINES_LENGTH_DESCENDING", "ddd\nbb\nA\nc"),
    ("Sort Lines In Locale Order Ascending", "EditSortLinesLocaleAsc", "IDM_EDIT_SORTLINES_LOCALE_ASCENDING", "A\nbb\nc\nddd"),
    ("Sort Lines In Locale Order Descending", "EditSortLinesLocaleDesc", "IDM_EDIT_SORTLINES_LOCALE_DESCENDING", "ddd\nc\nbb\nA"),
];

fn menu_action(label: &str) -> Box<dyn Action> {
    let menu = super::line_operations_menu();
    menu.items.iter().find_map(|item| match item {
        MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action.boxed_clone()),
        _ => None,
    }).expect("real Line Operations action")
}

fn with_source(text: &str, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _recording = macros::override_execution_for_test(false);
    let _settings = crate::settings::override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.line_ending = crate::text::line_ending::LineEnding::Lf;
        tab.buffer.mark_saved();
        tab.selection.collapse_to(0);
        w.tabs = vec![tab]; w.active = 0;
        let (sink, _) = std::sync::mpsc::channel();
        w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked(); test(&workspace, visual); cx.quit();
}

fn invoke(label: &str, visual: &mut VisualTestContext) {
    let action = menu_action(label);
    visual.update(|window, cx| window.dispatch_action(action, cx));
    visual.run_until_parked();
}

#[test]
fn line_sort_actual_menu_catalog_and_registered_actions_produce_one_undo() {
    for (label, id, npp, expected) in CASES.into_iter().filter(|case| cfg!(windows) || !case.0.contains("Locale")) {
        let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == id).expect("CATALOG action");
        assert_eq!(entry.npp, Some(npp));
        assert_eq!((entry.action)().name(), menu_action(label).name());
        with_source(SOURCE, |workspace, visual| {
            invoke(label, visual);
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), expected);
                assert_eq!(w.tabs[0].undo_stack.len(), 1);
                assert!(w.pending_prepared_edit.is_none());
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), SOURCE));
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        });
    }
}

#[test]
fn line_sort_menu_order_matches_the_oracle() {
    let labels = super::line_operations_menu().items.iter().filter_map(|item| match item {
        MenuItem::Action { name, .. } => Some(name.to_string()), _ => None,
    }).collect::<Vec<_>>();
    let ascending = labels.iter().position(|name| name == "Sort Lines Lexicographically Ascending").unwrap();
    assert_eq!(&labels[ascending..], [
        "Sort Lines Lexicographically Ascending", "Sort Lines Lex. Ascending Ignoring Case",
        "Sort Lines In Locale Order Ascending", "Sort Lines As Integers Ascending",
        "Sort Lines As Decimals (Comma) Ascending", "Sort Lines As Decimals (Dot) Ascending",
        "Sort Lines By Length Ascending", "Sort Lines Lexicographically Descending",
        "Sort Lines Lex. Descending Ignoring Case", "Sort Lines In Locale Order Descending",
        "Sort Lines As Integers Descending", "Sort Lines As Decimals (Comma) Descending",
        "Sort Lines As Decimals (Dot) Descending", "Sort Lines By Length Descending",
    ]);
    assert_eq!(&labels[ascending - 2..ascending], ["Reverse Line Order", "Randomize Line Order"]);
}

#[test]
fn line_sort_record_save_reload_and_background_playback_have_one_undo() {
    for (label, id, _, expected) in CASES.into_iter().filter(|case| cfg!(windows) || !case.0.contains("Locale")) {
        with_source(SOURCE, |workspace, visual| {
            visual.dispatch_action(MacroStartRecording);
            invoke(label, visual);
            visual.dispatch_action(MacroStopRecording);
            let events = macros::current(); assert_eq!(events.len(), 1);
            let event = match id {
                "EditSortLinesLexIgnoreCaseAsc" => MacroEvent::SortLinesLexIgnoreCaseAsc,
                "EditSortLinesLexIgnoreCaseDesc" => MacroEvent::SortLinesLexIgnoreCaseDesc,
                "EditSortLinesLocaleAsc" => MacroEvent::SortLinesLocaleAsc,
                "EditSortLinesLocaleDesc" => MacroEvent::SortLinesLocaleDesc,
                "EditSortLinesLengthAsc" => MacroEvent::SortLinesLengthAsc,
                "EditSortLinesLengthDesc" => MacroEvent::SortLinesLengthDesc,
                _ => panic!("unknown action in fixture"),
            };
            assert_eq!(std::mem::discriminant(events.first().unwrap()), std::mem::discriminant(&event));
            let directory = crate::persist::TempDir::new("line-sort-macro");
            let saved = macros::saved_round_trip_for_test(&directory.join("macros.toml"),
                &BTreeMap::from([("line sort".into(), events)])).unwrap();
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update_window_entity(workspace, |w, window, cx| {
                w.replay_macro(saved.get("line sort").unwrap(), 1, window, cx);
                assert!(w.pending_prepared_edit.is_some());
            });
            visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), SOURCE));
        });
    }
}

#[cfg(windows)]
#[test]
fn line_sort_native_locale_actions_order_numbers_and_preserve_case() {
    for (label, expected) in [(CASES[4].0, "ITEM1\nitem2\nItem10"), (CASES[5].0, "Item10\nitem2\nITEM1")] {
        with_source("item2\nItem10\nITEM1", |workspace, visual| {
            invoke(label, visual);
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "item2\nItem10\nITEM1"));
        });
    }
}

#[test]
fn line_sort_rectangles_survive_dispatch_undo_and_redo() {
    for (case, expected) in [(0, "q A\np BBB\nr cc"), (1, "r cc\np BBB\nq A"),
        (2, "q A\nr cc\np BBB"), (3, "p BBB\nr cc\nq A"),
        (4, "q A\np BBB\nr cc"), (5, "r cc\np BBB\nq A")].into_iter().filter(|case| cfg!(windows) || case.0 < 4) {
        with_source("p BBB\nq A\nr cc", |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                w.tabs[0].block_selection = Some(BlockSelection { anchor_line: 0, head_line: 2, anchor_col: 2, head_col: 5, is_dragging: false });
            });
            visual.dispatch_action(MacroStartRecording);
            invoke(CASES[case].0, visual);
            visual.dispatch_action(MacroStopRecording);
            let events = macros::current(); assert_eq!(events.len(), 1);
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert!(w.tabs[0].block_selection.is_some()); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "p BBB\nq A\nr cc"); assert!(w.tabs[0].block_selection.is_some()); });
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update_window_entity(workspace, |w, window, cx| { w.replay_macro(&events, 1, window, cx); assert!(w.pending_prepared_edit.is_some()); });
            visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].block_selection.is_some()); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "p BBB\nq A\nr cc"));
        });
    }
}

#[test]
fn line_sort_worker_waits_without_mutation_and_rejects_selection_drift() {
    with_source(SOURCE, |workspace, visual| {
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
        invoke(CASES[0].0, visual);
        visual.update(|_, cx| { let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_some()); assert_eq!(w.buffer().rope.to_string(), SOURCE); assert!(w.tabs[0].undo_stack.is_empty()); });
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].selection.collapse_to(2));
        send.send(()).unwrap(); visual.run_until_parked();
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), SOURCE); assert!(w.tabs[0].undo_stack.is_empty()); });
    });
}

#[test]
fn line_sort_read_only_preview_and_find_focus_preserve_the_document() {
    for mode in 0..3 {
        with_source(SOURCE, |workspace, visual| {
            visual.update_window_entity(workspace, |w, window, cx| {
                match mode {
                    0 => w.tabs[0].buffer.read_only = true,
                    1 => w.tabs[0].view_mode = TabViewMode::MarkdownPreview,
                    _ => window.focus(&w.find_focus, cx),
                }
                w.sort_lines_lex_ignore_case_asc(&EditSortLinesLexIgnoreCaseAsc, window, cx);
            });
            visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), SOURCE); assert!(w.tabs[0].undo_stack.is_empty()); assert!(w.pending_prepared_edit.is_none()); });
        });
    }
}

#[test]
fn line_sort_loading_composition_and_pending_escape_leave_source_and_history_intact() {
    for mode in 0..3 {
        with_source(SOURCE, |workspace, visual| {
            if mode == 0 {
                visual.update_window_entity(workspace, |w, _, _| {
                    w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 0, total_bytes: 20,
                        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) });
                });
            } else if mode == 1 {
                visual.update(|window, cx| {
                    let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                    handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                    assert!(workspace.read(cx).is_composing_text(window));
                });
            }
            let before = visual.update(|_, cx| { let w = workspace.read(cx); (w.buffer().rope.to_string(), w.tabs[0].undo_stack.len()) });
            if mode == 2 {
                let (send, receive) = futures::channel::oneshot::channel();
                visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
                invoke(CASES[0].0, visual);
                visual.dispatch_action(EditFindClose); visual.run_until_parked();
                send.send(()).unwrap();
            } else { invoke(CASES[0].0, visual); }
            visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!((w.buffer().rope.to_string(), w.tabs[0].undo_stack.len()), before); assert!(w.pending_prepared_edit.is_none()); });
        });
    }
}

#[test]
fn line_sort_mixed_macro_waits_for_large_sort_and_merges_one_undo() {
    let text = "z9\n".repeat(5000) + "a1";
    with_source(&text, |workspace, visual| {
        let events = [MacroEvent::InsertText("B0\n".into()), MacroEvent::SortLinesLexIgnoreCaseAsc,
            MacroEvent::MoveDocEnd, MacroEvent::InsertText("\nTAIL".into())];
        visual.update_window_entity(workspace, |w, window, cx| { w.replay_macro(&events, 1, window, cx); assert!(w.pending_prepared_edit.is_some()); });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); let out = w.buffer().rope.to_string();
            assert!(out.starts_with("a1\nB0\n")); assert!(out.ends_with("z9\nTAIL"));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), text));
    });
}

#[test]
#[ignore = "optimized >10 MiB line-sort timing; run explicitly"]
fn line_sort_release_measurement_above_10mib() {
    use std::time::Instant;
    let mut text = String::new();
    for row in (0..180_000).rev() {
        text.push_str(&format!("record{row:06} {}\n", "payload".repeat(8)));
    }
    text.pop();
    assert!(text.len() > 10 * 1024 * 1024);
    let mut baseline = String::new();
    with_source(&text, |workspace, visual| {
        let start = Instant::now();
        visual.dispatch_action(EditSortLinesLexAsc);
        let foreground = start.elapsed();
        visual.run_until_parked();
        visual.update(|_, cx| baseline = workspace.read(cx).buffer().rope.to_string());
        println!("line-sort baseline bytes={} rows=180000 synchronous_action={:.3}ms", text.len(), foreground.as_secs_f64()*1000.);
    });
    with_source(&text, |workspace, visual| {
        let start = Instant::now();
        visual.dispatch_action(EditSortLinesLexIgnoreCaseAsc);
        let action = start.elapsed();
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), baseline);
            let timing = w.prepared_timings.last().expect("actual prepared worker timing");
            println!("line-sort optimized bytes={} rows=180000 dispatch={:.3}ms capture={:.3}ms worker={:.3}ms publish={:.3}ms wall={:.3}ms",
                text.len(), action.as_secs_f64()*1000., timing.capture.as_secs_f64()*1000.,
                timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000., start.elapsed().as_secs_f64()*1000.);
        });
        for undo in [true, false] {
            let start = Instant::now();
            if undo { visual.dispatch_action(EditUndo); } else { visual.dispatch_action(EditRedo); }
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                let timing = w.prepared_timings.last().unwrap();
                assert_eq!(w.buffer().rope.to_string(), if undo { text.as_str() } else { baseline.as_str() });
                println!("line-sort {} capture={:.3}ms worker={:.3}ms publish={:.3}ms wall={:.3}ms", if undo { "undo" } else { "redo" },
                    timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000., start.elapsed().as_secs_f64()*1000.);
            });
        }
    });
    #[cfg(windows)]
    with_source(&text, |workspace, visual| {
        let start = Instant::now();
        let events = [MacroEvent::SortLinesLocaleAsc, MacroEvent::MoveDocEnd, MacroEvent::InsertText("\nTAIL".into())];
        visual.update_window_entity(workspace, |w, window, cx| { w.replay_macro(&events, 1, window, cx); assert!(w.pending_prepared_edit.is_some()); });
        let action = start.elapsed(); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), baseline.clone() + "\nTAIL");
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.active_macro_playback_id().is_none() && w.pending_prepared_edit.is_none());
            let timing = w.prepared_timings.first().unwrap();
            println!("line-sort locale mixed-macro bytes={} rows=180000 dispatch={:.3}ms capture={:.3}ms sort_worker={:.3}ms publish={:.3}ms wall={:.3}ms",
                text.len(), action.as_secs_f64()*1000., timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000., start.elapsed().as_secs_f64()*1000.);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), text));
    });
}
