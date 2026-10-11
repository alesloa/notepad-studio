use gpui::{ElementInputHandler, InputHandler, VisualContext};
use crate::tab::{InputFocus, TabViewMode};
use crate::text::encoding::Encoding;
use super::actions::grid_focus_tests::{press_action, with_grid};

#[test]
fn csv_grid_input_first_character_recovers_and_subsequent_text_commits_once() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        with_grid(|entity, visual, source_path| {
            let (raw, generation) = visual.update_window_entity(entity, |w, _, cx| {
                w.tabs[0].buffer.encoding = encoding;
                w.write_active_to_disk_as().unwrap();
                cx.notify();
                (w.buffer().rope.to_string(), w.autosave_generation)
            });
            visual.run_until_parked();
            visual.simulate_input("中");
            visual.update(|window, cx| {
                let w = entity.read(cx);
                let input = w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().expect("first typed text must open the cell");
                assert_eq!(input.buffer, "中");
                assert!(input.focus_handle.is_focused(window));
                assert_eq!(w.buffer().rope.to_string(), raw);
                assert!(w.autosave_generation > generation, "one-character drafts need recovery");
                let snapshot = w.session_snapshot(crate::session::BakPolicy::Full);
                assert_eq!(snapshot.tabs[0].csv_draft.as_ref().unwrap().value, "中");
            });
            visual.simulate_input("😀é");
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "中😀é");
            });
            press_action(visual, "FileSave");
            let reopened = crate::io::read_buffer(&source_path.with_file_name("grid.csv")).unwrap();
            assert_eq!(reopened.encoding, encoding);
            assert_eq!(reopened.line_ending, crate::text::line_ending::LineEnding::Crlf);
            assert!(reopened.rope.to_string().contains("中😀é,1"));
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert!(w.input_focus(window) == InputFocus::CsvGrid);
                assert!(!w.buffer().dirty);
            });
            press_action(visual, "EditUndo");
            visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), raw));
            press_action(visual, "EditRedo");
            visual.update(|_, cx| assert!(!entity.read(cx).buffer().dirty));
        });
    }
}

#[test]
fn csv_grid_input_marked_utf16_replacements_continue_in_cell_without_raw_fallthrough() {
    with_grid(|entity, visual, _| {
        let raw = visual.update(|_, cx| entity.read(cx).buffer().rope.to_string());
        visual.simulate_input("に");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.is_some(), "paint registered grid input");
        });
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(Some(0..1), "日本😀", Some(4..4), window, cx);
            handler.replace_and_mark_text_in_range(Some(0..4), "日本語😀", Some(5..5), window, cx);
            handler.unmark_text(window, cx);
        });
        visual.run_until_parked();
        visual.simulate_input("!");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.input_focus(window) == InputFocus::CsvCell);
            assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "日本語😀!");
            assert_eq!(w.buffer().rope.to_string(), raw);
        });
        press_action(visual, "FileSave");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().contains("日本語😀!,1")));
    });
}

#[test]
fn csv_grid_input_first_header_character_captures_and_commits_without_duplicate_text() {
    with_grid(|entity, visual, _| {
        let generation = visual.update_window_entity(entity, |w, _, cx| {
            let model = w.csv_models.get_mut(&w.tabs[0].session_id).unwrap();
            model.selected_cols.insert(0); model.selected_rows.clear(); cx.notify();
            w.autosave_generation
        });
        visual.run_until_parked();
        visual.simulate_input("新");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let model = w.csv_models.get(&w.tabs[0].session_id).unwrap();
            assert_eq!(model.editing_cell, Some((usize::MAX, 0)));
            assert_eq!(model.cell_input.as_ref().unwrap().buffer, "新");
            assert!(w.autosave_generation > generation);
            assert_eq!(w.session_snapshot(crate::session::BakPolicy::Full).tabs[0].csv_draft.as_ref().unwrap().value, "新");
        });
        visual.simulate_input("😀");
        press_action(visual, "FileSave");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("新😀,value\n")));
    });
}

#[test]
fn csv_grid_input_newer_inline_field_keeps_text_input() {
    with_grid(|entity, visual, _| {
        let raw = visual.update_window_entity(entity, |w, window, cx| {
            w.goto_state.open = true; window.focus(&w.goto_focus, cx); cx.notify();
            w.buffer().rope.to_string()
        });
        visual.run_until_parked();
        visual.simulate_input("7");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.goto_focus.is_focused(window));
            assert_eq!(w.goto_state.text, "7");
            assert_eq!(w.buffer().rope.to_string(), raw);
            assert!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.is_none());
        });
    });
}

#[test]
fn csv_grid_input_hidden_grid_does_not_receive_source_text() {
    with_grid(|entity, visual, _| {
        visual.dispatch_action(crate::actions::ViewCsvSource);
        press_action(visual, "EditSelectAll");
        visual.simulate_input("source input 😀");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[0].view_mode, TabViewMode::Edit);
            assert!(w.focus_handle.is_focused(window));
            assert_eq!(w.buffer().rope.to_string(), "source input 😀");
            assert!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.is_none());
        });
    });
}

#[test]
fn csv_grid_input_inactive_window_does_not_begin_edit() {
    with_grid(|entity, visual, _| {
        let raw = visual.update(|_, cx| entity.read(cx).buffer().rope.to_string());
        visual.deactivate_window();
        visual.update_window_entity(entity, |_, _, cx| cx.notify());
        visual.run_until_parked();
        visual.simulate_input("中");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(!window.is_window_active());
            assert_eq!(w.buffer().rope.to_string(), raw);
            assert!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.is_none());
        });
    });
}

#[test]
fn csv_grid_input_readonly_or_stale_source_refuses_commit_and_retains_typed_draft() {
    for readonly in [false, true] {
        with_grid(|entity, visual, _| {
            let raw = visual.update_window_entity(entity, |w, _, _| {
                w.tabs[0].buffer.read_only = readonly;
                w.buffer().rope.to_string()
            });
            visual.simulate_input("draft 😀");
            if !readonly {
                visual.update_window_entity(entity, |w, _, cx| {
                    w.csv_models.get_mut(&w.tabs[0].session_id).unwrap().source_state = None;
                    cx.notify();
                });
                visual.run_until_parked();
            }
            press_action(visual, "FileSave");
            visual.update(|window, cx| {
                let w = entity.read(cx);
                let input = w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap();
                assert_eq!(input.buffer, "draft 😀");
                assert!(input.focus_handle.is_focused(window));
                assert_eq!(w.buffer().rope.to_string(), raw);
                assert!(!w.buffer().dirty);
                assert_eq!(w.session_snapshot(crate::session::BakPolicy::Full).tabs[0].csv_draft.as_ref().unwrap().value, "draft 😀");
            });
        });
    }
}

#[test]
fn csv_grid_input_empty_schema_keeps_source_intact() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |w, _, cx| {
            let total = w.buffer().rope.len_bytes();
            w.replace_byte_range(0..total, "", cx);
        });
        visual.run_until_parked();
        visual.simulate_input("中");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().rope.len_bytes(), 0);
            assert!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.is_none());
        });
    });
}

#[test]
fn csv_grid_input_single_seed_reaches_actual_debounced_recovery() {
    for header in [false, true] {
        with_grid(|entity, visual, _| {
            let (sink, captures) = std::sync::mpsc::channel();
            visual.update_window_entity(entity, |w, _, cx| {
                w.session_snapshot_sink = Some(sink);
                if header {
                    let model = w.csv_models.get_mut(&w.tabs[0].session_id).unwrap();
                    model.selected_cols.insert(0); model.selected_rows.clear();
                    cx.notify();
                }
            });
            visual.run_until_parked();
            visual.simulate_input("中");
            assert!(captures.try_recv().is_err());
            // Autosave uses the production wall-clock timer.
            std::thread::sleep(std::time::Duration::from_millis(350));
            visual.run_until_parked();
            let snapshot = captures.try_recv().expect("first grid input must reach recovery without another edit");
            let draft = snapshot.tabs[0].csv_draft.as_ref().unwrap();
            assert_eq!(draft.value, "中");
            assert_eq!(draft.is_header, header);
            assert!(captures.try_recv().is_err());
        });
    }
}
