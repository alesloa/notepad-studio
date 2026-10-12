// Retain the actual handler installed by EditorElement paint. Transitions
// below call the registered action handlers, rather than changing tab fields.
use gpui::{Entity, InputHandler, TestAppContext, VisualContext, VisualTestContext};
use crate::{actions::*, settings::Settings, tab::Tab, workspace::Workspace};

pub(crate) fn with_sources(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        ..Default::default()
    });
    let directory = crate::persist::TempDir::new("source-input-origin");
    let paths = [directory.join("first.md"), directory.join("second.txt")];
    std::fs::write(&paths[0], "first 中😀\r\n").unwrap();
    std::fs::write(&paths[1], "second café\r\n").unwrap();
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs = paths.iter().map(|path| {
            let mut tab = Tab::empty();
            tab.buffer = crate::io::read_buffer(path).unwrap();
            tab.selection.collapse_to(0);
            tab
        }).collect();
        w.active = 0;
        let (sink, _) = std::sync::mpsc::channel();
        w.session_snapshot_sink = Some(sink);
        w.touch_mru();
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&entity, visual);
    cx.quit();
}

fn capture(visual: &mut VisualTestContext) -> Box<dyn InputHandler> {
    visual.update(|window, _| window.take_registered_input_handler().expect("Source paint installed a native handler"))
}

fn assert_refused(handler: &mut dyn InputHandler, window: &mut gpui::Window, cx: &mut gpui::App) {
    let mut adjusted = Some(99..99);
    assert!(handler.text_for_range(0..1, &mut adjusted, window, cx).is_none());
    assert_eq!(adjusted, Some(99..99));
    assert!(handler.selected_text_range(true, window, cx).is_none());
    assert!(handler.marked_text_range(window, cx).is_none());
    assert!(handler.bounds_for_range(0..1, window, cx).is_none());
    assert!(handler.character_index_for_point(gpui::point(gpui::px(10.), gpui::px(10.)), window, cx).is_none());
    assert!(!handler.accepts_text_input(window, cx));
    handler.replace_text_in_range(None, "unsolicited edit", window, cx);
    handler.replace_and_mark_text_in_range(None, "preedit", None, window, cx);
    handler.unmark_text(window, cx);
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    tabs: Vec<(uuid::Uuid, String, usize, usize, usize, usize, bool)>,
    active: usize,
    status: Option<String>,
    query: String,
    replacement: String,
    goto: String,
    autosave: u64,
    rich: Vec<(uuid::Uuid, String)>,
    csv: Vec<(uuid::Uuid, String)>,
}

fn snapshot(entity: &Entity<Workspace>, visual: &mut VisualTestContext) -> Snapshot {
    visual.update(|_, cx| {
        let w = entity.read(cx);
        Snapshot {
            tabs: w.tabs.iter().map(|tab| (tab.session_id, tab.buffer.rope.to_string(), tab.selection.anchor, tab.selection.head, tab.undo_stack.len(), tab.redo_stack.len(), tab.buffer.dirty)).collect(),
            active: w.active, status: w.status_message.clone(), query: w.find_state.query.clone(),
            replacement: w.find_state.replacement.clone(), goto: w.goto_state.text.clone(), autosave: w.autosave_generation,
            rich: w.tabs.iter().filter_map(|tab| tab.buffer.rich_doc.as_ref().map(|payload| (tab.session_id, format!("{payload:?}")))).collect(),
            csv: {
                let mut models: Vec<_> = w.csv_models.iter().map(|(id, model)| (*id, format!("{:?}:{:?}:{:?}:{:?}:{}:{}",
                    model.cell_input.as_ref().map(|input| (&input.buffer, input.anchor, input.cursor)),
                    model.find_input.as_ref().map(|input| (&input.buffer, input.anchor, input.cursor)),
                    model.replace_input.as_ref().map(|input| (&input.buffer, input.anchor, input.cursor)),
                    model.editing_cell, model.undo_stack.len(), model.redo_stack.len()))).collect();
                models.sort_by_key(|(id, _)| *id);
                models
            },
        }
    })
}

#[test]
fn source_input_origin_painted_handler_does_not_read_a_new_tab() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.dispatch_action(TabGoto2);
        visual.run_until_parked();
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            let mut adjusted = Some(99..99);
            assert_eq!(old.text_for_range(0..6, &mut adjusted, window, cx), None);
            assert_eq!(adjusted, Some(99..99));
            assert!(old.selected_text_range(true, window, cx).is_none());
            assert!(old.marked_text_range(window, cx).is_none());
            assert!(old.bounds_for_range(0..1, window, cx).is_none());
            assert!(old.character_index_for_point(gpui::point(gpui::px(10.), gpui::px(10.)), window, cx).is_none());
            assert!(!old.accepts_text_input(window, cx));
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_painted_handler_does_not_write_a_new_tab() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.dispatch_action(TabGoto2);
        visual.run_until_parked();
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            old.replace_text_in_range(Some(0..6), "late final 😀", window, cx);
            old.replace_and_mark_text_in_range(None, "遅い", Some(2..2), window, cx);
            old.unmark_text(window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_same_frame_tab_aba_invalidates_the_painted_handler() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        // Both real transitions finish before a repaint can replace the handler.
        visual.update_window_entity(entity, |w, window, cx| {
            w.goto_tab_2(&TabGoto2, window, cx);
            w.goto_tab_1(&TabGoto1, window, cx);
        });
        let before = snapshot(entity, visual);
        visual.update(|window, cx| old.replace_text_in_range(None, "late ABA", window, cx));
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_registered_handler_accepts_successive_edits_and_preedit_across_frames() {
    with_sources(|entity, visual| {
        let mut handler = capture(visual);
        visual.update(|window, cx| handler.replace_text_in_range(None, "a", window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| handler.replace_text_in_range(None, "😀", window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| handler.replace_and_mark_text_in_range(None, "日本😀", Some(2..4), window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert_eq!(handler.marked_text_range(window, cx), Some(3..7));
            handler.replace_text_in_range(None, "日本😀", window, cx);
        });
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "a😀日本😀first 中😀\n");
            // Adjacent ordinary input shares one entry; IME retains its dedicated Single.
            assert_eq!(w.tabs[0].undo_stack.len(), 2);
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "second café\n");
        });
    });
}

fn history_aba(redo: bool) {
        with_sources(|entity, visual| {
            if redo { visual.simulate_input("a"); }
            let revision = visual.update(|_, cx| entity.read(cx).buffer().raw_revision.clone());
            let mut old = capture(visual);
            if !redo {
                // capture removed the platform slot; a real repaint installs
                // the fresh handler used for this intervening edit.
                visual.update(|window, _| window.refresh());
                visual.run_until_parked();
                visual.simulate_input("a");
                visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with('a')));
            }
            visual.dispatch_action(EditUndo);
            if redo { visual.dispatch_action(EditRedo); }
            visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().raw_revision, revision, "history deliberately restored the captured identity"));
            let before = snapshot(entity, visual);
            visual.update(|window, cx| old.replace_text_in_range(None, "late history", window, cx));
            assert_eq!(snapshot(entity, visual), before);
        });
}

#[test]
fn source_input_origin_typing_then_undo_does_not_revive_an_old_handler() { history_aba(false); }

#[test]
fn source_input_origin_undo_then_redo_does_not_revive_an_old_handler() { history_aba(true); }

fn selection_change(aba: bool) {
        with_sources(|entity, visual| {
            let mut old = capture(visual);
            visual.update_window_entity(entity, |w, window, cx| {
                w.select_right(&SelectRight, window, cx);
                if aba { w.move_left(&MoveLeft, window, cx); }
            });
            let before = snapshot(entity, visual);
            visual.update(|window, cx| {
                assert!(old.selected_text_range(true, window, cx).is_none());
                old.replace_text_in_range(None, "late selection", window, cx);
            });
            assert_eq!(snapshot(entity, visual), before);
            visual.run_until_parked();
            // Repainting publishes the new caret promptly for ordinary input.
            visual.simulate_input("fresh");
            visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
        });
}

#[test]
fn source_input_origin_selection_change_retires_retained_callbacks() { selection_change(false); }

#[test]
fn source_input_origin_same_frame_selection_aba_retires_retained_callbacks() { selection_change(true); }

fn context_aba(view: bool) {
        with_sources(|entity, visual| {
            let mut old = capture(visual);
            visual.update_window_entity(entity, |w, window, cx| {
                if view {
                    w.view_markdown_preview(&ViewMarkdownPreview, window, cx);
                    w.view_markdown_edit(&ViewMarkdownEdit, window, cx);
                } else {
                    w.focus_input(crate::tab::InputFocus::FindQuery, window, cx);
                    window.focus(&w.focus_handle, cx);
                }
            });
            let before = snapshot(entity, visual);
            visual.update(|window, cx| {
                old.replace_and_mark_text_in_range(None, "late", Some(4..4), window, cx);
                old.unmark_text(window, cx);
            });
            assert_eq!(snapshot(entity, visual), before);
        });
}

#[test]
fn source_input_origin_same_frame_focus_aba_refuses_late_protocol() { context_aba(false); }

#[test]
fn source_input_origin_same_frame_view_aba_refuses_late_protocol() { context_aba(true); }

#[test]
fn source_input_origin_old_source_protocol_preserves_a_new_find_field() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.update_window_entity(entity, |w, window, cx| w.focus_input(crate::tab::InputFocus::FindQuery, window, cx));
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            old.replace_and_mark_text_in_range(None, "old preedit", Some(3..3), window, cx);
            old.replace_text_in_range(None, "old final", window, cx);
            old.unmark_text(window, cx);
            assert!(entity.read(cx).find_focus.is_focused(window));
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_old_unmark_preserves_a_new_tabs_composition() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.dispatch_action(TabGoto2);
        visual.run_until_parked();
        let mut current = capture(visual);
        visual.update(|window, cx| current.replace_and_mark_text_in_range(None, "日本😀", Some(2..4), window, cx));
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert_eq!(current.marked_text_range(window, cx), Some(0..4));
            old.unmark_text(window, cx);
            assert_eq!(current.marked_text_range(window, cx), Some(0..4));
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_old_protocol_preserves_actual_rendered_goto_field() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.dispatch_action(EditGotoLine);
        visual.run_until_parked();
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert!(entity.read(cx).goto_focus.is_focused(window));
            old.replace_and_mark_text_in_range(None, "123", Some(3..3), window, cx);
            old.replace_text_in_range(None, "456", window, cx);
            old.unmark_text(window, cx);
            assert!(entity.read(cx).goto_focus.is_focused(window));
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_close_and_restore_does_not_revive_the_old_handler() {
    with_sources(|entity, visual| {
        let id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        let mut old = capture(visual);
        visual.dispatch_action(FileCloseTab);
        visual.dispatch_action(FileRestoreClosedTab);
        visual.run_until_parked();
        visual.update(|_, cx| { let w = entity.read(cx); assert_eq!(w.tabs[w.active].session_id, id); });
        let before = snapshot(entity, visual);
        visual.update(|window, cx| old.replace_text_in_range(None, "late restored", window, cx));
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_noop_focus_redraw_view_and_same_document_reorder_remain_usable() {
    with_sources(|entity, visual| {
        let mut handler = capture(visual);
        let token = visual.update(|window, _| window.input_context_token());
        visual.update_window_entity(entity, |w, window, cx| {
            window.focus(&w.focus_handle, cx);
            window.activate_window();
            w.set_markdown_view_mode(false, cx);
            w.move_tab(0, 1, cx);
            w.touch_mru();
            window.refresh();
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert_eq!(window.input_context_token(), token);
            assert!(handler.accepts_text_input(window, cx));
            handler.replace_text_in_range(None, "kept 😀", window, cx);
        });
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.active, 1);
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "kept 😀first 中😀\n");
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "second café\n");
        });
    });
}

#[test]
fn source_input_origin_window_deactivation_and_reactivation_never_revive_old_input() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.deactivate_window();
        let before = snapshot(entity, visual);
        visual.update(|window, cx| old.replace_text_in_range(None, "inactive", window, cx));
        assert_eq!(snapshot(entity, visual), before);
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        let before = snapshot(entity, visual);
        visual.update(|window, cx| old.replace_and_mark_text_in_range(None, "late activation", None, window, cx));
        assert_eq!(snapshot(entity, visual), before);
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
    });
}

#[test]
fn source_input_origin_actual_mouse_selection_aba_retires_the_native_handler() {
    with_sources(|entity, visual| {
        let (start, next) = visual.update(|_, cx| {
            let line = entity.read(cx).last_lines.first().unwrap();
            let y = line.bounds.center().y;
            (gpui::point(line.bounds.left() + gpui::px(1.), y),
                gpui::point(line.bounds.left() + line.shaped.x_for_index(1), y))
        });
        visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        let mut old = capture(visual);
        visual.simulate_mouse_move(next, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.update(|_, cx| assert_eq!(entity.read(cx).selection().head, 1));
        visual.simulate_mouse_move(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.simulate_mouse_up(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.update(|_, cx| assert_eq!(entity.read(cx).selection().head, 0));
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert!(old.selected_text_range(true, window, cx).is_none());
            old.replace_text_in_range(None, "old drag", window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
    });
}

#[test]
fn source_input_origin_old_protocol_preserves_a_rendered_csv_cell() {
    with_sources(|entity, visual| {
        let directory = crate::persist::TempDir::new("source-csv-origin");
        let path = directory.join("neutral.csv");
        std::fs::write(&path, "label,value\r\nseed,1\r\n").unwrap();
        visual.update_window_entity(entity, |w, _, cx| { w.tabs[1].buffer = crate::io::read_buffer(&path).unwrap(); cx.notify(); });
        visual.run_until_parked();
        let mut old = capture(visual);
        visual.dispatch_action(TabGoto2);
        visual.dispatch_action(ViewCsvPreview);
        visual.run_until_parked();
        // Open the genuine cell-editor route and paint its native field.
        visual.update_window_entity(entity, crate::ui::csv_doc::actions::open_active_cell_editor);
        visual.run_until_parked();
        visual.update(|window, cx| assert!(entity.read(cx).input_focus(window) == crate::tab::InputFocus::CsvCell));
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            old.replace_and_mark_text_in_range(None, "late", Some(4..4), window, cx);
            old.replace_text_in_range(None, "final", window, cx);
            old.unmark_text(window, cx);
            assert!(entity.read(cx).input_focus(window) == crate::tab::InputFocus::CsvCell);
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_old_protocol_preserves_the_rendered_rich_body() {
    with_sources(|entity, visual| {
        let directory = crate::persist::TempDir::new("source-rich-origin");
        let path = directory.join("neutral.rtf");
        std::fs::write(&path, br"{\rtf1\ansi seed\par}").unwrap();
        visual.update_window_entity(entity, |w, _, cx| { w.tabs[1].buffer = crate::io::read_buffer(&path).unwrap(); cx.notify(); });
        visual.run_until_parked();
        let mut old = capture(visual);
        visual.dispatch_action(TabGoto2);
        visual.dispatch_action(ViewPreview);
        visual.run_until_parked();
        let point = visual.update(|window, _| window.bounds().center());
        visual.simulate_click(point, gpui::Modifiers::none());
        visual.update(|window, cx| assert!(entity.read(cx).input_focus(window) == crate::tab::InputFocus::RichDocBody));
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            old.replace_and_mark_text_in_range(None, "late", Some(4..4), window, cx);
            old.replace_text_in_range(None, "final", window, cx);
            old.unmark_text(window, cx);
            assert!(entity.read(cx).input_focus(window) == crate::tab::InputFocus::RichDocBody);
        });
        assert_eq!(snapshot(entity, visual), before);
    });
}

#[test]
fn source_input_origin_fresh_production_welcome_survives_initial_mru_sync() {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    let mut handler = capture(visual);
    visual.update_window_entity(&entity, |w, _, _| w.touch_mru());
    visual.update(|window, cx| {
        assert!(handler.accepts_text_input(window, cx));
        handler.replace_text_in_range(None, "welcome 😀", window, cx);
        assert!(entity.read(cx).buffer().rope.to_string().starts_with("welcome 😀"));
    });
    cx.quit();
}

#[test]
fn source_input_origin_native_callback_cannot_join_an_open_edit_group() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.update_window_entity(entity, |w, _, _| w.begin_edit_group());
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert_refused(old.as_mut(), window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
        visual.update_window_entity(entity, |w, _, cx| {
            assert!(w.tabs[0].edit_transaction.as_ref().unwrap().ops.is_empty());
            w.end_edit_group(cx);
            cx.notify();
        });
        visual.run_until_parked();
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
    });
}

#[test]
fn source_input_origin_block_drag_release_retires_the_dragging_native_context() {
    with_sources(|entity, visual| {
        let (start, next) = visual.update(|_, cx| {
            let line = entity.read(cx).last_lines.first().unwrap();
            let y = line.bounds.center().y;
            (gpui::point(line.bounds.left() + gpui::px(1.), y),
                gpui::point(line.bounds.left() + line.shaped.x_for_index(1), y))
        });
        let modifiers = gpui::Modifiers { alt: true, ..gpui::Modifiers::none() };
        visual.simulate_mouse_down(start, gpui::MouseButton::Left, modifiers);
        visual.simulate_mouse_move(next, gpui::MouseButton::Left, modifiers);
        visual.update(|_, cx| assert!(entity.read(cx).block_selection().unwrap().is_dragging));
        let mut old = capture(visual);
        visual.simulate_mouse_up(next, gpui::MouseButton::Left, modifiers);
        visual.update(|_, cx| assert!(!entity.read(cx).block_selection().unwrap().is_dragging));
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert!(old.selected_text_range(true, window, cx).is_none());
            old.replace_text_in_range(None, "old block", window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
    });
}

#[test]
fn source_input_origin_folder_reveal_tab_aba_retires_the_painted_handler() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        visual.update_window_entity(entity, |w, _, cx| {
            let path = w.tabs[1].buffer.file_path.clone().unwrap();
            w.show_folder_panel_context_menu(path, crate::ui::folder_panel::context_menu::RowKind::File,
                gpui::point(gpui::px(20.), gpui::px(20.)), cx);
        });
        visual.run_until_parked();
        // The third painted file row is Reveal in Tab. Switch back before
        // running the foreground executor for the resulting frame.
        visual.simulate_mouse_down(gpui::point(gpui::px(60.), gpui::px(79.)), gpui::MouseButton::Left, gpui::Modifiers::none());
        visual.update_window_entity(entity, |w, window, cx| {
            assert_eq!(w.active, 1, "the actual painted Reveal row ran");
            w.goto_tab_1(&TabGoto1, window, cx);
        });
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert!(old.selected_text_range(true, window, cx).is_none());
            old.replace_text_in_range(None, "late folder activation", window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
        visual.run_until_parked();
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
    });
}

#[test]
fn source_input_origin_native_callback_cannot_join_bulk_edits() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        // Controller boundary control, rather than a fabricated GUI bug.
        visual.update_window_entity(entity, |w, _, _| w.bulk_edit = true);
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert_refused(old.as_mut(), window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
        visual.update_window_entity(entity, |w, _, cx| { w.bulk_edit = false; cx.notify(); });
        visual.run_until_parked();
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("fresh")));
    });
}

#[test]
fn source_input_origin_raw_rope_change_with_same_revision_refuses_old_protocol() {
    with_sources(|entity, visual| {
        let mut old = capture(visual);
        // Supplemental content guard: an untracked mutation cannot revive
        // a retained callback just because the revision stayed unchanged.
        visual.update_window_entity(entity, |w, _, cx| { w.tabs[0].buffer.rope.insert(0, "new "); cx.notify(); });
        let before = snapshot(entity, visual);
        visual.update(|window, cx| {
            assert!(old.selected_text_range(true, window, cx).is_none());
            old.replace_text_in_range(None, "old content", window, cx);
        });
        assert_eq!(snapshot(entity, visual), before);
        visual.run_until_parked();
        visual.simulate_input("fresh");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().starts_with("freshnew ")));
    });
}

#[test]
fn source_input_origin_csv_save_as_source_accepts_typing_and_composition() {
    with_sources(|entity, visual| {
        let directory = crate::persist::TempDir::new("source-csv-save-as");
        let path = directory.join("neutral.csv");
        let destination = directory.join("saved.txt");
        std::fs::write(&path, crate::text::encoding::encode("label,value\r\n中😀,1\r\n", crate::text::encoding::Encoding::Utf8Bom)).unwrap();
        visual.update_window_entity(entity, |w, _, cx| { w.tabs[1].buffer = crate::io::read_buffer(&path).unwrap(); cx.notify(); });
        visual.dispatch_action(TabGoto2);
        visual.run_until_parked();
        let mut old = capture(visual);
        visual.dispatch_action(ViewCsvPreview);
        visual.run_until_parked();
        visual.update_window_entity(entity, |w, window, cx| {
            let handle = w.csv_models.get(&w.tabs[w.active].session_id).unwrap().grid_focus.clone().unwrap();
            window.focus(&handle, cx);
            // Same real Save As core as the existing CSV focus regression;
            // the destination is isolated and no native dialog is opened.
            w.tabs[w.active].buffer.file_path = Some(destination.clone());
            w.write_active_to_disk_as().unwrap();
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert_eq!(entity.read(cx).tabs[1].view_mode, crate::tab::TabViewMode::Edit);
            assert!(entity.read(cx).focus_handle.is_focused(window));
        });
        let before = snapshot(entity, visual);
        visual.update(|window, cx| assert_refused(old.as_mut(), window, cx));
        assert_eq!(snapshot(entity, visual), before);
        let mut current = capture(visual);
        visual.update(|window, cx| current.replace_text_in_range(None, "fresh 😀", window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| current.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| current.replace_text_in_range(None, "日本😀", window, cx));
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.buffer().rope.to_string().starts_with("fresh 😀日本😀"));
            assert_eq!(w.buffer().encoding, crate::text::encoding::Encoding::Utf8Bom);
            assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
            assert_eq!(w.tabs[1].undo_stack.len(), 2);
        });
        assert!(std::fs::read(&destination).unwrap().starts_with(&[0xef, 0xbb, 0xbf]));
    });
}
