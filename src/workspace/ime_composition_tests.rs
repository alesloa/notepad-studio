// Exercises GPUI's OS input protocol in rendered neutral fixture windows.
use gpui::{ElementInputHandler, Entity, InputHandler, VisualContext, VisualTestContext};
use crate::{tab::InputFocus, workspace::Workspace};
use crate::ui::csv_doc::actions::grid_focus_tests::{press_action, with_grid};

fn with_source(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    with_grid(|entity, visual, path| {
        std::fs::write(path, crate::text::encoding::encode("a😀中bc\r\n", crate::text::encoding::Encoding::Utf8Bom)).unwrap();
        let buffer = crate::io::read_buffer(path).unwrap();
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs[1].buffer = buffer;
            w.tabs[1].selection.collapse_to(1);
            cx.notify();
        });
        press_action(visual, "TabGoto2");
        visual.update(|window, cx| assert!(entity.read(cx).focus_handle.is_focused(window)));
        test(entity, visual);
    });
}

#[test]
fn ime_composition_source_repeated_preedit_replaces_then_commits_once_and_undoes_once() {
    with_source(|entity, visual| {
        let original = visual.update(|_, cx| entity.read(cx).buffer().rope.to_string());
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
            handler.replace_text_in_range(None, "日本😀", window, cx);
        });
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a日本😀😀中bc\n"));
        press_action(visual, "FileSave");
        let path = visual.update(|_, cx| entity.read(cx).buffer().file_path.clone().unwrap());
        let reopened = crate::io::read_buffer(&path).unwrap();
        assert_eq!(reopened.encoding, crate::text::encoding::Encoding::Utf8Bom);
        assert_eq!(reopened.line_ending, crate::text::line_ending::LineEnding::Crlf);
        assert_eq!(reopened.rope.to_string(), "a日本😀😀中bc\n");
        press_action(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), original, "composition is one Undo"));
        press_action(visual, "EditRedo");
        visual.update(|_, cx| {
            assert_eq!(entity.read(cx).buffer().rope.to_string(), "a日本😀😀中bc\n");
            assert!(!entity.read(cx).buffer().dirty);
        });
    });
}

#[test]
fn ime_composition_source_reports_utf16_mark_and_relative_selection() {
    with_source(|entity, visual| {
        visual.update_window_entity(entity, |w, _, _| {
            w.tabs[1].selection.anchor = 1; w.tabs[1].selection.head = 8;
        });
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "日本😀", Some(2..4), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), Some(1..5));
            assert_eq!(handler.selected_text_range(false, window, cx).unwrap().range, 3..5);
            let mut actual = None;
            assert_eq!(handler.text_for_range(1..5, &mut actual, window, cx).as_deref(), Some("日本😀"));
            assert_eq!(actual, Some(1..5));
            handler.unmark_text(window, cx);
            assert_eq!(handler.marked_text_range(window, cx), None);
        });
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a日本😀bc\n"));
        press_action(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a😀中bc\n"));
    });
}

#[test]
fn ime_composition_csv_grid_repeated_preedit_commits_cell_once_and_undoes_once() {
    with_grid(|entity, visual, path| {
        let original = visual.update(|_, cx| entity.read(cx).buffer().rope.to_string());
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
            handler.replace_text_in_range(None, "日本😀", window, cx);
            assert_eq!(handler.marked_text_range(window, cx), None);
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.input_focus(window) == InputFocus::CsvCell);
            assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "日本😀");
            assert_eq!(w.buffer().rope.to_string(), original);
        });
        press_action(visual, "FileSave");
        let reopened = crate::io::read_buffer(&path.with_file_name("grid.csv")).unwrap();
        assert!(reopened.rope.to_string().contains("日本😀,1"));
        assert_eq!(reopened.encoding, crate::text::encoding::Encoding::Utf8Bom);
        assert_eq!(reopened.line_ending, crate::text::line_ending::LineEnding::Crlf);
        press_action(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), original));
    });
}

#[test]
fn ime_composition_csv_selected_utf16_text_tracks_mark_and_relative_selection() {
    with_grid(|entity, visual, _| {
        visual.simulate_input("a😀中z");
        visual.update_window_entity(entity, |w, _, _| {
            let input = w.csv_models.get_mut(&w.tabs[0].session_id).unwrap().cell_input.as_mut().unwrap();
            input.anchor = 1; input.cursor = 8;
        });
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "日", Some(1..1), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), Some(1..2));
            handler.replace_and_mark_text_in_range(Some(1..2), "日本😀", Some(2..4), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), Some(1..5));
            assert_eq!(handler.selected_text_range(false, window, cx).unwrap().range, 3..5);
            handler.unmark_text(window, cx);
            assert_eq!(handler.marked_text_range(window, cx), None);
        });
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "a日本😀z");
        });
    });
}

#[test]
fn ime_composition_source_empty_preedit_removes_mark_without_deleting_neighbors() {
    with_source(|entity, visual| {
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            handler.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), None);
        });
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a😀中bc\n"));
    });
}

#[test]
fn ime_composition_csv_escape_discards_preedit_then_new_cell_input_keeps_new_text() {
    with_grid(|entity, visual, _| {
        let original = visual.update(|_, cx| entity.read(cx).buffer().rope.to_string());
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
        });
        visual.run_until_parked();
        visual.simulate_keystrokes("escape");
        visual.simulate_input("新");
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            assert_eq!(handler.marked_text_range(window, cx), None);
            let w = entity.read(cx);
            assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "新");
            assert_eq!(w.buffer().rope.to_string(), original);
        });
    });
}

#[test]
fn ime_composition_source_focus_tab_and_preview_transitions_retire_old_mark() {
    for transition in 0..3 {
        with_source(|entity, visual| {
            visual.update(|window, cx| {
                ElementInputHandler::new(window.bounds(), entity.clone())
                    .replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            });
            visual.run_until_parked();
            match transition {
                0 => {
                    visual.update_window_entity(entity, |w, window, cx| {
                        w.goto_state.open = true; window.focus(&w.goto_focus, cx); cx.notify();
                    });
                    visual.run_until_parked();
                    visual.simulate_input("7");
                    visual.update(|_, cx| assert_eq!(entity.read(cx).goto_state.text, "7"));
                    visual.update_window_entity(entity, |w, window, cx| {
                        w.goto_state.open = false; window.focus(&w.focus_handle, cx); cx.notify();
                    });
                }
                1 => { press_action(visual, "TabGoto1"); press_action(visual, "TabGoto2"); }
                _ => {
                    visual.update_window_entity(entity, |w, _, cx| {
                        w.tabs[1].buffer.file_path = w.tabs[1].buffer.file_path.as_ref().map(|path| path.with_extension("md"));
                        cx.notify();
                    });
                    visual.dispatch_action(crate::actions::ViewMarkdownPreview);
                    visual.run_until_parked();
                    visual.dispatch_action(crate::actions::ViewSource);
                }
            }
            visual.run_until_parked();
            visual.update(|window, cx| {
                assert_eq!(ElementInputHandler::new(window.bounds(), entity.clone()).marked_text_range(window, cx), None);
            });
            visual.simulate_input("新");
            visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "aに新😀中bc\n", "transition {transition}"));
        });
    }
}

#[test]
fn ime_composition_source_readonly_and_external_edits_never_reuse_old_history() {
    for readonly in [false, true] {
        with_source(|entity, visual| {
            visual.update(|window, cx| {
                ElementInputHandler::new(window.bounds(), entity.clone())
                    .replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            });
            visual.update_window_entity(entity, |w, _, cx| {
                if readonly { w.tabs[1].buffer.read_only = true; cx.notify(); }
                else { w.replace_byte_range(0..1, "x", cx); }
            });
            visual.run_until_parked();
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
                assert_eq!(handler.marked_text_range(window, cx), None);
                handler.replace_and_mark_text_in_range(None, "新", Some(1..1), window, cx);
                handler.unmark_text(window, cx);
            });
            if readonly {
                visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "aに😀中bc\n"));
            } else {
                visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "x新に😀中bc\n"));
                press_action(visual, "EditUndo");
                visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "xに😀中bc\n"));
                press_action(visual, "EditUndo");
                visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "aに😀中bc\n", "unrelated edit has its own Undo"));
            }
        });
    }
}

#[test]
fn ime_composition_source_cancel_restores_clean_identity_and_prior_redo() {
    with_source(|entity, visual| {
        visual.simulate_input("x"); press_action(visual, "EditUndo");
        visual.update(|_, cx| assert!(!entity.read(cx).buffer().dirty));
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            handler.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx);
            handler.unmark_text(window, cx);
        });
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(!w.buffer().dirty);
            assert!(w.tabs[1].undo_stack.is_empty());
        });
        press_action(visual, "EditRedo");
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "ax😀中bc\n"));
    });
}

#[test]
fn ime_composition_source_macro_emits_only_final_or_unmarked_text_and_replays() {
    for unmark in [false, true] {
        with_source(|entity, visual| {
            let (sink, events) = std::sync::mpsc::channel();
            visual.update_window_entity(entity, |w, _, _| w.ime_record_sink = Some(sink));
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
                handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                handler.replace_and_mark_text_in_range(None, "日本😀", Some(4..4), window, cx);
                assert!(events.try_recv().is_err(), "preedit is never recorded");
                if unmark { handler.unmark_text(window, cx); }
                else { handler.replace_text_in_range(None, "日本😀", window, cx); }
                handler.unmark_text(window, cx);
            });
            let crate::macros::MacroEvent::InsertText(text) = events.try_recv().unwrap() else { panic!("wrong event"); };
            assert_eq!(text, "日本😀");
            assert!(events.try_recv().is_err(), "final text is recorded once");
            press_action(visual, "EditUndo");
            visual.update(|window, cx| {
                ElementInputHandler::new(window.bounds(), entity.clone()).replace_text_in_range(None, &text, window, cx);
            });
            visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a日本😀😀中bc\n"));
        });
    }
}

#[test]
fn ime_composition_source_cancel_and_blur_never_record_preedit_in_another_tab() {
    for cancel in [false, true] {
        with_source(|entity, visual| {
            let (sink, events) = std::sync::mpsc::channel();
            visual.update_window_entity(entity, |w, _, _| w.ime_record_sink = Some(sink));
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
                handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                if cancel { handler.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx); }
            });
            press_action(visual, "TabGoto1");
            visual.update(|window, cx| ElementInputHandler::new(window.bounds(), entity.clone()).unmark_text(window, cx));
            assert!(events.try_recv().is_err());
        });
    }
}

#[test]
fn ime_composition_source_partial_marked_update_keeps_one_exact_undo() {
    with_source(|entity, visual| {
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            handler.replace_and_mark_text_in_range(None, "日本😀", Some(4..4), window, cx);
            handler.replace_and_mark_text_in_range(Some(2..3), "中", Some(1..1), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), Some(2..3));
            handler.unmark_text(window, cx);
        });
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a日中😀😀中bc\n"));
        press_action(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a😀中bc\n"));
        press_action(visual, "EditRedo");
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a日中😀😀中bc\n"));
    });
}

#[test]
fn ime_composition_csv_focus_tab_preview_and_new_cell_retire_old_mark() {
    for transition in 0..4 {
        with_grid(|entity, visual, _| {
            visual.update(|window, cx| {
                ElementInputHandler::new(window.bounds(), entity.clone())
                    .replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            });
            visual.run_until_parked();
            match transition {
                0 => {
                    visual.update_window_entity(entity, |w, window, cx| {
                        w.goto_state.open = true; window.focus(&w.goto_focus, cx); cx.notify();
                    });
                    visual.run_until_parked();
                    visual.simulate_input("7");
                    visual.update(|_, cx| assert_eq!(entity.read(cx).goto_state.text, "7"));
                }
                1 => { press_action(visual, "TabGoto2"); }
                2 => { visual.dispatch_action(crate::actions::ViewCsvSource); }
                _ => {
                    visual.simulate_keystrokes("escape");
                    visual.simulate_input("新");
                }
            }
            visual.run_until_parked();
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
                assert_eq!(handler.marked_text_range(window, cx), None);
                handler.unmark_text(window, cx);
                if transition == 3 {
                    let w = entity.read(cx);
                    assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "新");
                }
            });
        });
    }
}

#[test]
fn ime_composition_csv_readonly_or_stale_draft_remains_editable_and_recoverable() {
    for readonly in [false, true] {
        with_grid(|entity, visual, _| {
            let raw = visual.update_window_entity(entity, |w, _, _| {
                w.tabs[0].buffer.read_only = readonly; w.buffer().rope.to_string()
            });
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
                handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                handler.replace_and_mark_text_in_range(None, "日本😀", Some(4..4), window, cx);
                handler.unmark_text(window, cx);
            });
            if !readonly {
                visual.update_window_entity(entity, |w, _, cx| {
                    w.csv_models.get_mut(&w.tabs[0].session_id).unwrap().source_state = None;
                    cx.notify();
                });
            }
            press_action(visual, "FileSave");
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert!(w.input_focus(window) == InputFocus::CsvCell);
                assert_eq!(w.buffer().rope.to_string(), raw);
                assert_eq!(w.session_snapshot(crate::session::BakPolicy::Full).tabs[0].csv_draft.as_ref().unwrap().value, "日本😀");
            });
        });
    }
}

#[test]
#[allow(clippy::reversed_empty_ranges)]
fn ime_composition_source_surrogate_reversed_and_huge_ranges_remain_on_char_boundaries() {
    for range in [2..2, 4..1, usize::MAX..usize::MAX] {
        with_source(|entity, visual| {
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
                handler.replace_and_mark_text_in_range(Some(range.clone()), "😀中", Some(1..usize::MAX), window, cx);
                let mark = handler.marked_text_range(window, cx).unwrap();
                assert_eq!(mark.end - mark.start, 3);
                let mut actual = None;
                assert_eq!(handler.text_for_range(mark, &mut actual, window, cx).as_deref(), Some("😀中"));
                let selection = handler.selected_text_range(false, window, cx).unwrap().range;
                assert_eq!(selection.end - selection.start, 1, "split surrogate selected offset is clamped");
                handler.unmark_text(window, cx);
            });
            press_action(visual, "EditUndo");
            visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "a😀中bc\n"));
        });
    }
}

#[test]
fn ime_composition_csv_changed_mark_refuses_stale_replacement_without_losing_text() {
    with_grid(|entity, visual, _| {
        visual.update(|window, cx| {
            ElementInputHandler::new(window.bounds(), entity.clone())
                .replace_and_mark_text_in_range(None, "日本😀", Some(4..4), window, cx);
        });
        visual.update_window_entity(entity, |w, _, cx| {
            let input = w.csv_models.get_mut(&w.tabs[0].session_id).unwrap().cell_input.as_mut().unwrap();
            input.buffer.replace_range(0..6, "中国");
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), entity.clone());
            assert_eq!(handler.marked_text_range(window, cx), None);
            handler.replace_text_in_range(None, "新", window, cx);
            let w = entity.read(cx);
            assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap().buffer, "中国😀");
            assert!(w.status_message.as_ref().unwrap().contains("restart input"));
        });
    });
}
