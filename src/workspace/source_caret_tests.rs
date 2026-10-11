// Retained callbacks use the actual native handler installed by Source paint.
use super::Workspace;
use crate::{actions::*, editor_element::source_input_origin_tests::with_sources};
use gpui::{Entity, InputHandler, VisualContext, VisualTestContext};

fn capture(visual: &mut VisualTestContext) -> Box<dyn InputHandler> {
    visual.update(|window, _| window.take_registered_input_handler().unwrap())
}

fn proof(entity: &Entity<Workspace>, visual: &mut VisualTestContext) -> (String, usize, usize, usize, usize, bool) {
    visual.update(|_, cx| {
        let w = entity.read(cx); let tab = &w.tabs[w.active];
        (w.buffer().rope.to_string(), tab.selection.anchor, tab.selection.head,
            tab.undo_stack.len(), tab.redo_stack.len(), w.buffer().dirty)
    })
}

fn block_key(entity: &Entity<Workspace>, visual: &mut VisualTestContext) -> Option<(usize, usize, usize, usize, bool)> {
    visual.update(|_, cx| entity.read(cx).block_selection().map(|b| (b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging)))
}

fn press(visual: &mut VisualTestContext, id: &str) {
    let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == id).unwrap();
    visual.simulate_keystrokes(crate::shortcuts::platform_default_keys(entry).first().unwrap());
}

fn fresh_after_aba(entity: &Entity<Workspace>, visual: &mut VisualTestContext, old: &mut dyn InputHandler) {
    let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
    visual.run_until_parked();
    let mut fresh = capture(visual);
    visual.update_window_entity(entity, |w, _, cx| w.commit_caret(cx));
    visual.update(|window, cx| {
        assert_eq!(entity.read(cx).source_input_token(), epoch);
        assert!(!old.accepts_text_input(window, cx));
        assert!(fresh.accepts_text_input(window, cx));
        fresh.replace_text_in_range(None, "新", window, cx);
    });
    let edited = proof(entity, visual);
    assert!(edited.0.contains('新')); assert_eq!(edited.3, 1);
    visual.update(|window, cx| {
        old.replace_text_in_range(None, "late after fresh paint", window, cx);
        old.replace_and_mark_text_in_range(None, "遅い", None, window, cx);
        old.unmark_text(window, cx);
    });
    assert_eq!(proof(entity, visual), edited);
}

fn no_op(kind: u8) {
    with_sources(|entity, visual| {
        if kind == 1 { visual.dispatch_action(MoveDocEnd); visual.run_until_parked(); }
        let before = proof(entity, visual);
        let mut old = capture(visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update(|window, cx| match kind {
            0 => window.dispatch_action(Box::new(MoveLeft), cx),
            1 => window.dispatch_action(Box::new(MoveRight), cx),
            2 => window.dispatch_action(Box::new(MoveDocStart), cx),
            3 => window.dispatch_action(Box::new(MoveLineStart), cx),
            4 => window.dispatch_action(Box::new(MoveUp), cx),
            5 => window.dispatch_action(Box::new(EditPageUp), cx),
            _ => entity.update(cx, |w, cx| w.commit_caret(cx)),
        });
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| {
            assert_eq!(entity.read(cx).source_input_token(), epoch, "no caret change must preserve the captured Source identity");
            assert!(old.accepts_text_input(window, cx));
            old.replace_text_in_range(None, "入力😀", window, cx);
        });
        let after = proof(entity, visual);
        assert_eq!(after.0, format!("{}入力😀{}", &before.0[..before.2], &before.0[before.2..]));
        assert_eq!(after.3, 1);
    });
}

#[test] fn source_caret_noop_left_keeps_native_input() { no_op(0); }
#[test] fn source_caret_noop_right_at_eof_keeps_native_input() { no_op(1); }
#[test] fn source_caret_noop_doc_start_keeps_native_input() { no_op(2); }
#[test] fn source_caret_noop_line_start_keeps_native_input() { no_op(3); }
#[test] fn source_caret_noop_up_keeps_native_input() { no_op(4); }
#[test] fn source_caret_noop_page_up_keeps_native_input() { no_op(5); }
#[test] fn source_caret_noop_commit_keeps_native_input() { no_op(6); }

fn movement_aba(kind: u8) {
    with_sources(|entity, visual| {
        if kind >= 6 {
            visual.update_window_entity(entity, |w, _, cx| { w.word_wrap = true; cx.notify(); });
            visual.run_until_parked();
            visual.update(|_, cx| assert!(entity.read(cx).tabs[0].wrap_cache.is_some()));
        }
        let mut old = capture(visual);
        let before = proof(entity, visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update(|window, cx| {
            let (first, second): (Box<dyn gpui::Action>, Box<dyn gpui::Action>) = match kind {
                0 => (Box::new(MoveRight), Box::new(MoveLeft)),
                1 => (Box::new(MoveDown), Box::new(MoveUp)),
                2 => (Box::new(EditPageDown), Box::new(EditPageUp)),
                3 => (Box::new(SelectDown), Box::new(SelectUp)),
                4 => (Box::new(SelectPageDown), Box::new(SelectPageUp)),
                6 => (Box::new(MoveDown), Box::new(MoveUp)),
                7 => (Box::new(SelectDown), Box::new(SelectUp)),
                _ => (Box::new(BlockExtendRight), Box::new(BlockExtendLeft)),
            };
            window.dispatch_action(first, cx);
            let entity = entity.clone(); let start = (before.1, before.2);
            cx.defer(move |cx| {
                let moved = entity.read(cx).selection();
                assert_ne!((moved.anchor, moved.head), start, "the first registered action must actually move the selection");
            });
            window.dispatch_action(second, cx);
        });
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| {
            assert_ne!(entity.read(cx).source_input_token(), epoch, "real caret roundtrip must retire the captured Source identity");
            assert!(!old.accepts_text_input(window, cx));
            old.replace_text_in_range(None, "late movement", window, cx);
            old.replace_and_mark_text_in_range(None, "遅い", None, window, cx);
            old.unmark_text(window, cx);
        });
        assert_eq!(proof(entity, visual), before);
        fresh_after_aba(entity, visual, old.as_mut());
    });
}

#[test] fn source_caret_horizontal_aba_refuses_old_native_input() { movement_aba(0); }
#[test] fn source_caret_vertical_aba_refuses_old_native_input() { movement_aba(1); }
#[test] fn source_caret_page_aba_refuses_old_native_input() { movement_aba(2); }
#[test] fn source_caret_vertical_selection_aba_refuses_old_native_input() { movement_aba(3); }
#[test] fn source_caret_page_selection_aba_refuses_old_native_input() { movement_aba(4); }
#[test] fn source_caret_block_aba_refuses_old_native_input() { movement_aba(5); }
#[test] fn source_caret_wrapped_vertical_aba_refuses_old_native_input() { movement_aba(6); }
#[test] fn source_caret_wrapped_vertical_selection_aba_refuses_old_native_input() { movement_aba(7); }

#[test]
fn source_caret_block_corner_aba_refuses_old_native_input() {
    block_corner_aba(false);
}

#[test]
fn source_caret_virtual_block_corner_aba_refuses_old_native_input() {
    block_corner_aba(true);
}

fn block_corner_aba(virtual_column: bool) {
    with_sources(|entity, visual| {
        if virtual_column { visual.dispatch_action(MoveLineEnd); }
        visual.dispatch_action(BlockExtendRight); visual.run_until_parked();
        let before = proof(entity, visual);
        let block = block_key(entity, visual);
        assert!(block.is_some());
        let mut old = capture(visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update(|window, cx| {
            window.dispatch_action(Box::new(BlockExtendRight), cx);
            let entity = entity.clone();
            let start = (before.1, before.2);
            cx.defer(move |cx| {
                let w = entity.read(cx); let b = w.block_selection().unwrap();
                assert_ne!((b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging), block.unwrap());
                if virtual_column { assert_eq!((w.selection().anchor, w.selection().head), start); }
            });
            window.dispatch_action(Box::new(BlockExtendLeft), cx);
        });
        assert_eq!(proof(entity, visual), before);
        assert_eq!(block_key(entity, visual), block);
        visual.update(|window, cx| {
            assert_ne!(entity.read(cx).source_input_token(), epoch);
            assert!(!old.accepts_text_input(window, cx));
            old.replace_text_in_range(None, "late block", window, cx);
        });
        assert_eq!(proof(entity, visual), before);
        fresh_after_aba(entity, visual, old.as_mut());
    });
}

#[test]
fn source_caret_find_selection_aba_refuses_old_native_input() {
    with_sources(|entity, visual| {
        visual.update_window_entity(entity, |w, _, cx| {
            w.find_state.query = "first|中".into(); w.find_state.regex = true;
            w.find_state.match_case = true; w.refresh_matches(); cx.notify();
        });
        visual.dispatch_action(EditFindNext); visual.run_until_parked();
        let before = proof(entity, visual);
        assert_eq!((before.1, before.2), (6, 9));
        let mut old = capture(visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update(|window, cx| {
            window.dispatch_action(Box::new(EditFindNext), cx);
            let entity = entity.clone();
            cx.defer(move |cx| {
                let s = entity.read(cx).selection();
                assert_eq!((s.anchor, s.head), (0, 5));
            });
            window.dispatch_action(Box::new(EditFindPrev), cx);
        });
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| {
            assert_ne!(entity.read(cx).source_input_token(), epoch);
            assert!(!old.accepts_text_input(window, cx));
            old.replace_text_in_range(None, "late find", window, cx);
        });
        assert_eq!(proof(entity, visual), before);
        fresh_after_aba(entity, visual, old.as_mut());
    });
}

fn keyed_caret(first: &str, second: Option<&str>) {
    with_sources(|entity, visual| {
        visual.update(|_, cx| crate::shortcuts::reload_keymap(cx, &Default::default()));
        let mut old = capture(visual);
        let before = proof(entity, visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        press(visual, first);
        if let Some(second) = second {
            let moved = proof(entity, visual);
            assert_ne!((moved.1, moved.2), (before.1, before.2), "the production key must move before the return key");
            press(visual, second);
        }
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| {
            assert_eq!(entity.read(cx).source_input_token() == epoch, second.is_none());
            assert_eq!(old.accepts_text_input(window, cx), second.is_none());
            old.replace_text_in_range(None, "keyed", window, cx);
        });
        let after = proof(entity, visual);
        if second.is_none() { assert_eq!(after.0, format!("keyed{}", before.0)); }
        else { assert_eq!(after, before); }
    });
}

#[test] fn source_caret_catalog_left_noop_keeps_native_input() { keyed_caret("MoveLeft", None); }
#[test] fn source_caret_catalog_horizontal_aba_refuses_old_native_input() { keyed_caret("MoveRight", Some("MoveLeft")); }
#[test] fn source_caret_catalog_vertical_aba_refuses_old_native_input() { keyed_caret("MoveDown", Some("MoveUp")); }
#[test] fn source_caret_catalog_block_aba_refuses_old_native_input() { keyed_caret("BlockExtendRight", Some("BlockExtendLeft")); }

#[test]
fn source_caret_current_ime_handler_survives_unchanged_commit_after_selected_preedit() {
    with_sources(|entity, visual| {
        let mut handler = capture(visual);
        visual.update(|window, cx| {
            handler.replace_and_mark_text_in_range(None, "に😀本", Some(1..3), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), Some(0..4));
        });
        let before = proof(entity, visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update_window_entity(entity, |w, _, cx| w.commit_caret(cx));
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| {
            assert_eq!(entity.read(cx).source_input_token(), epoch);
            assert!(handler.accepts_text_input(window, cx));
            handler.replace_and_mark_text_in_range(None, "中国😀", Some(2..4), window, cx);
            handler.replace_text_in_range(None, "中国😀", window, cx);
        });
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "中国😀first 中😀\n");
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
    });
}

#[test]
fn source_caret_current_autopair_handler_survives_unchanged_commit() {
    with_sources(|entity, visual| {
        let mut settings = crate::settings::snapshot();
        settings.auto_insert_parens = true;
        let _settings = crate::settings::override_for_test(settings);
        let mut handler = capture(visual);
        visual.update(|window, cx| handler.replace_text_in_range(None, "(", window, cx));
        assert_eq!(proof(entity, visual).0, "()first 中😀\n");
        assert_eq!(proof(entity, visual).2, 1);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update_window_entity(entity, |w, _, cx| w.commit_caret(cx));
        visual.update(|window, cx| {
            assert_eq!(entity.read(cx).source_input_token(), epoch);
            assert!(handler.accepts_text_input(window, cx));
            handler.replace_text_in_range(None, ")", window, cx);
            handler.replace_text_in_range(None, "z", window, cx);
        });
        assert_eq!(proof(entity, visual).0, "()zfirst 中😀\n");
        assert_eq!(proof(entity, visual).3, 2);
    });
}

#[test]
fn source_caret_select_all_then_find_aba_retires_each_transition() {
    with_sources(|entity, visual| {
        visual.update_window_entity(entity, |w, _, cx| {
            w.find_state.query = "first".into(); w.find_state.regex = false;
            w.find_state.match_case = true; w.refresh_matches(); cx.notify();
        });
        visual.dispatch_action(EditFindNext);
        let mut old = capture(visual);
        let before = proof(entity, visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.dispatch_action(EditSelectAll);
        let selection = proof(entity, visual);
        assert_eq!((selection.1, selection.2), (0, before.0.len()));
        let selected_epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        assert_ne!(selected_epoch, epoch);
        visual.dispatch_action(EditFindNext);
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| {
            assert_ne!(entity.read(cx).source_input_token(), selected_epoch);
            assert!(!old.accepts_text_input(window, cx));
            old.replace_text_in_range(None, "late select all", window, cx);
        });
        assert_eq!(proof(entity, visual), before);
    });
}

#[test]
fn source_caret_begin_end_marker_keeps_identity_until_selection_changes() {
    with_sources(|entity, visual| {
        visual.dispatch_action(MoveRight);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.dispatch_action(EditBeginEndSelect);
        assert_eq!(visual.update(|_, cx| entity.read(cx).source_input_token()), epoch);
        visual.dispatch_action(MoveDocStart);
        let mut old = capture(visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.dispatch_action(EditBeginEndSelect);
        let selected = proof(entity, visual);
        assert_eq!((selected.1, selected.2), (1, 0));
        visual.update(|window, cx| {
            assert_ne!(entity.read(cx).source_input_token(), epoch);
            assert!(!old.accepts_text_input(window, cx));
        });
    });
}

#[test]
fn source_caret_actual_mouse_release_retires_drag_identity() {
    with_sources(|entity, visual| {
        let point = visual.update(|_, cx| {
            let line = entity.read(cx).last_lines.first().unwrap();
            gpui::point(line.bounds.left() + gpui::px(1.), line.bounds.center().y)
        });
        visual.simulate_mouse_down(point, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.update(|_, cx| assert!(entity.read(cx).selection().is_dragging));
        let mut old = capture(visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.simulate_mouse_up(point, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(!w.selection().is_dragging);
            assert_eq!((w.selection().anchor, w.selection().head), (0, 0));
            assert_ne!(w.source_input_token(), epoch);
            assert!(!old.accepts_text_input(window, cx));
        });
    });
}

#[test]
fn source_caret_post_edit_selection_correction_is_not_a_later_caret_transition() {
    with_sources(|entity, visual| {
        for _ in 0..5 { visual.dispatch_action(SelectRight); }
        assert_eq!((proof(entity, visual).1, proof(entity, visual).2), (0, 5));
        let retired = std::rc::Rc::new(std::cell::RefCell::new(None));
        visual.update(|window, cx| {
            window.dispatch_action(Box::new(EditUppercase), cx);
            let entity = entity.clone(); let retired = retired.clone();
            cx.defer(move |cx| {
                let w = entity.read(cx);
                assert_eq!(w.buffer().rope.to_string(), "FIRST 中😀\n");
                assert_eq!((w.selection().anchor, w.selection().head), (0, 5));
                *retired.borrow_mut() = Some(w.source_input_token());
            });
        });
        let after_edit = retired.borrow().clone().unwrap();
        visual.run_until_parked();
        assert_eq!(visual.update(|_, cx| entity.read(cx).source_input_token()), after_edit,
            "paint must align the final caret without retiring an already completed edit again");
    });
}

fn post_edit_worker(movement: u8) {
    let _execution = crate::macros::override_execution_for_test(false);
    with_sources(|entity, visual| {
        visual.update_window_entity(entity, |w, _, cx| {
            w.find_state.query = "FIRST".into(); w.find_state.match_case = true;
            w.tabs[w.active].buffer.marks = vec![6..9; 16_385]; cx.notify();
        });
        for _ in 0..5 { visual.dispatch_action(SelectRight); }
        let mut old = capture(visual);
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(entity, |w, _, _| w.prepared_test_gate = Some(receive));
        let requested = std::rc::Rc::new(std::cell::RefCell::new(None));
        visual.update(|window, cx| {
            window.dispatch_action(Box::new(EditUppercase), cx);
            window.dispatch_action(Box::new(EditIncreaseLineIndent), cx);
            let entity_probe = entity.clone(); let request_probe = requested.clone();
            cx.defer(move |cx| {
                let w = entity_probe.read(cx);
                assert!(w.pending_prepared_edit.is_some());
                assert_eq!((w.selection().anchor, w.selection().head), (0, 5));
                *request_probe.borrow_mut() = Some(w.source_input_token());
            });
            if movement == 2 {
                window.dispatch_action(Box::new(MoveRight), cx);
                let entity_probe = entity.clone(); let request_probe = requested.clone();
                cx.defer(move |cx| {
                    let w = entity_probe.read(cx);
                    assert_eq!((w.selection().anchor, w.selection().head), (5, 5));
                    assert_ne!(Some(w.source_input_token()), request_probe.borrow().clone(), "first real transition must retire despite the pending final edit correction");
                });
            }
            if movement == 3 {
                let entity_probe = entity.clone(); let request_probe = requested.clone();
                cx.defer(move |cx| entity_probe.update(cx, |w, cx| {
                    w.move_to_byte(5, true, false); w.commit_caret(cx);
                    assert_eq!((w.selection().anchor, w.selection().head), (0, 5));
                    assert_eq!(Some(w.source_input_token()), request_probe.borrow().clone());
                }));
            } else if movement != 0 { window.dispatch_action(Box::new(EditFindNext), cx); }
        });
        let request = requested.borrow().clone().unwrap();
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert_eq!((w.selection().anchor, w.selection().head), (0, 5));
            assert_eq!(w.source_input_token() == request, movement != 2);
            assert!(!old.accepts_text_input(window, cx));
        });
        send.send(()).unwrap(); visual.run_until_parked();
        let result = proof(entity, visual);
        if movement == 2 { assert_eq!(result.0, "FIRST 中😀\n"); assert_eq!(result.3, 1); }
        else { assert_eq!(result.0, "    FIRST 中😀\n"); assert_eq!(result.3, 2); }
    });
}

#[test] fn source_caret_completed_edit_worker_survives_next_paint() { post_edit_worker(0); }
#[test] fn source_caret_completed_edit_worker_survives_noop_find_before_paint() { post_edit_worker(1); }
#[test] fn source_caret_completed_edit_worker_refuses_selection_aba_before_paint() { post_edit_worker(2); }
#[test] fn source_caret_completed_edit_worker_survives_noop_movement_before_paint() { post_edit_worker(3); }

#[test]
fn source_caret_autopair_skip_delegate_retires_before_native_refresh() {
    with_sources(|entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.auto_insert_parens = true;
        let _settings = crate::settings::override_for_test(settings);
        let mut current = capture(visual);
        visual.update(|window, cx| current.replace_text_in_range(None, "(", window, cx));
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        // The delegate itself must finish retirement before hint scheduling
        // or the current native wrapper refresh can observe this new caret.
        visual.update(|window, cx| {
            let mut delegate = gpui::ElementInputHandler::new(gpui::Bounds::default(), entity.clone());
            delegate.replace_text_in_range(None, ")", window, cx);
            let w = entity.read(cx);
            assert_eq!(w.selection().head, 2);
            assert_eq!(w.buffer().rope.to_string(), "()first 中😀\n");
            assert_ne!(w.source_input_token(), epoch);
            assert!(!current.accepts_text_input(window, cx));
        });
    });
}

#[test]
fn source_caret_find_result_selection_aba_retires_immediately() {
    with_sources(|entity, visual| {
        visual.update_window_entity(entity, |w, _, cx| {
            let path = w.buffer().file_path.clone().unwrap();
            let source = crate::find_in_files::OpenBufferResultSource::capture(&mut w.tabs[0]);
            w.find_in_files.results = [0..5, 6..9].into_iter().map(|range| crate::find_in_files::FindResult {
                path: path.clone(), line: 0, line_text: "first 中😀".into(), match_in_line: range,
                tab_idx: Some(0), display_label: None,
                identity: std::sync::Arc::new(()), source: Some(source.clone()),
            }).collect();
            w.jump_to_find_result(0, cx);
        });
        visual.run_until_parked();
        let mut old = capture(visual);
        let before = proof(entity, visual);
        let epoch = visual.update(|_, cx| entity.read(cx).source_input_token());
        visual.update_window_entity(entity, |w, _, cx| {
            w.jump_to_find_result(1, cx);
            assert_eq!((w.selection().anchor, w.selection().head), (6, 9));
            assert_ne!(w.source_input_token(), epoch);
            w.jump_to_find_result(0, cx);
        });
        assert_eq!(proof(entity, visual), before);
        visual.update(|window, cx| assert!(!old.accepts_text_input(window, cx)));
        fresh_after_aba(entity, visual, old.as_mut());
    });
}

#[test]
fn source_caret_welcome_first_paint_does_not_retire_an_initial_unchanged_caret() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        toolbar_hidden: true, folder_panel_visible: false, ..Default::default()
    });
    let mut cx = gpui::TestAppContext::single();
    let initial = std::rc::Rc::new(std::cell::RefCell::new(None));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.window_closed = true; *initial.borrow_mut() = Some(w.source_input_token()); w
    });
    visual.update(|window, cx| {
        window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    assert_eq!(visual.update(|_, cx| entity.read(cx).source_input_token()), initial.borrow().clone().unwrap());
    let mut handler = capture(visual);
    visual.update_window_entity(&entity, |w, _, cx| w.commit_caret(cx));
    visual.update(|window, cx| assert!(handler.accepts_text_input(window, cx)));
    cx.quit();
}

fn missing_tab_block_funnel(clear: bool) {
    with_sources(|entity, visual| {
        visual.update_window_entity(entity, |w, _, _| {
            let tabs = std::mem::take(&mut w.tabs);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if clear { assert!(!w.clear_block_selection()); }
                else { w.set_block_selection(None); }
            }));
            w.tabs = tabs;
            w.sync_source_input_tab();
            assert!(result.is_ok(), "a missing tab must not panic in caret reconciliation");
        });
    });
}

#[test] fn source_caret_missing_tab_block_set_refuses_without_panicking() { missing_tab_block_funnel(false); }
#[test] fn source_caret_missing_tab_block_clear_refuses_without_panicking() { missing_tab_block_funnel(true); }

#[test]
fn source_caret_release_large_rope_sample() {
    if std::env::var_os("NOTEPAD_STUDIO_CARET_BENCH").is_none() { return; }
    with_sources(|entity, visual| {
        for repeats in [1, 1_300_000] {
            let text = "café😀\n".repeat(repeats);
            visual.update_window_entity(entity, |w, _, cx| {
                w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&text);
                w.active_tab_mut().buffer.language = None;
                w.active_tab_mut().buffer.raw_revision = crate::buffer::ContentRevision::default();
                w.active_tab_mut().selection.collapse_to(0); w.retire_source_input(); cx.notify();
            });
            visual.run_until_parked();
            let mut samples = Vec::new();
            for _ in 0..12 {
                let sync = visual.update_window_entity(entity, |w, _, _| {
                    let start = std::time::Instant::now();
                    for _ in 0..100_000 {
                        w.sync_source_input_caret(); std::hint::black_box(w.source_input_token());
                    }
                    start.elapsed().as_secs_f64() * 1000.
                });
                let paint_capture = visual.update(|window, cx| {
                    let bounds = entity.read(cx).viewport.bounds;
                    let start = std::time::Instant::now();
                    for _ in 0..1000 {
                        std::hint::black_box(super::source_input::SourceInputHandler::new(bounds, entity.clone(), window, cx));
                    }
                    start.elapsed().as_secs_f64() * 1000.
                });
                let commit = visual.update_window_entity(entity, |w, _, cx| {
                    let start = std::time::Instant::now();
                    for _ in 0..100 { w.commit_caret(cx); }
                    start.elapsed().as_secs_f64() * 1000.
                });
                let movement = visual.update_window_entity(entity, |w, _, _| {
                    let start = std::time::Instant::now();
                    for _ in 0..10_000 { w.move_to_byte(1, false, false); w.move_to_byte(0, false, false); }
                    start.elapsed().as_secs_f64() * 1000.
                });
                samples.push([sync, paint_capture, commit, movement]);
            }
            for (index, name) in ["sync x100000", "native capture x1000", "noop commit x100", "movement roundtrip x10000"].iter().enumerate() {
                let mut durations: Vec<_> = samples.iter().map(|sample| sample[index]).collect();
                durations.sort_by(|a, b| a.total_cmp(b));
                println!("caret bytes={} {name}: median={:.3}ms p95={:.3}ms", text.len(), durations[durations.len()/2], durations[durations.len()-1]);
            }
        }
    });
}
