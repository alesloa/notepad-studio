use gpui::{Entity, VisualContext, VisualTestContext};
use crate::tab::{InputFocus, Tab, TabViewMode};
use crate::workspace::Workspace;
use super::{ensure_model, open_cell_editor};

pub(crate) fn press_action(visual: &mut VisualTestContext, action: &str) {
    let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == action).unwrap();
    visual.simulate_keystrokes(crate::shortcuts::platform_default_keys(entry)[0]);
}

pub(crate) fn with_grid(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, &std::path::Path)) {
    with_grid_mode(false, test);
}

fn with_grid_mode(vertical: bool, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, &std::path::Path)) {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false, tab_vertical: vertical,
        ..Default::default()
    });
    let dir = crate::persist::TempDir::new("csv-grid-focus");
    let csv_path = dir.join("grid.csv");
    let source_path = dir.join("text.txt");
    std::fs::write(&csv_path, crate::text::encoding::encode("label,value\r\ncaf\u{00E9},1\r\n中😀,2\r\n", crate::text::encoding::Encoding::Utf8Bom)).unwrap();
    std::fs::write(&source_path, "source 中😀\n").unwrap();
    let csv = crate::io::read_buffer(&csv_path).unwrap();
    let source = crate::io::read_buffer(&source_path).unwrap();
    let mut cx = gpui::TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs = vec![Tab::empty(), Tab::empty()];
        w.tabs[0].buffer = csv; w.tabs[0].view_mode = TabViewMode::CsvPreview;
        w.tabs[1].buffer = source; w.active = 0; w.touch_mru();
        w.window_closed = true;
        w
    });
    visual.update_window_entity(&entity, |w, window, cx| {
        window.activate_window(); ensure_model(w, window, cx);
        let handle = w.csv_models.get(&w.tabs[0].session_id).unwrap().grid_focus.clone().unwrap();
        window.focus(&handle, cx);
    });
    visual.run_until_parked();
    press_action(visual, "EditSelectAll");
    visual.update(|window, cx| {
        let w = entity.read(cx);
        assert!(w.input_focus(window) == InputFocus::CsvGrid);
        assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().selected_rows.len(), 2, "CATALOG reached the rendered grid");
        assert!(!w.buffer().dirty);
    });
    visual.simulate_keystrokes("escape");
    test(&entity, visual, &source_path);
    cx.quit();
}

fn assert_source_input(entity: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.run_until_parked();
    press_action(visual, "EditSelectAll");
    visual.simulate_input("visible edit 😀");
    visual.update(|window, cx| {
        let w = entity.read(cx);
        assert_eq!(w.tabs[w.active].view_mode, TabViewMode::Edit);
        assert!(w.focus_handle.is_focused(window), "visible source must own focus");
        assert_eq!(w.buffer().rope.to_string(), "visible edit 😀", "typed text reached the visible source");
    });
}

#[test]
fn csv_grid_focus_existing_source_tab_catalog_routes() {
    for action in ["TabGoto2", "TabNextSequential", "TabNext"] {
        with_grid(|entity, visual, _| {
            press_action(visual, action);
            visual.update(|_, cx| assert_eq!(entity.read(cx).active, 1, "{action}"));
            assert_source_input(entity, visual);
        });
    }
}

#[test]
fn csv_grid_focus_new_tab_catalog_routes_to_visible_source() {
    with_grid(|entity, visual, _| {
        press_action(visual, "FileNew");
        visual.update(|_, cx| assert_eq!(entity.read(cx).active, 2));
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_close_routes_to_visible_source() {
    with_grid(|entity, visual, _| {
        press_action(visual, "FileCloseTab");
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs.len(), 1));
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_close_all_routes_to_visible_source() {
    with_grid(|entity, visual, _| {
        press_action(visual, "FileCloseAllTabs");
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs.len(), 1));
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_close_unchanged_routes_to_visible_source() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |w, _, _| w.tabs[1].buffer.dirty = true);
        visual.dispatch_action(crate::actions::FileCloseAllUnchanged);
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs.len(), 1));
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_restored_tab_catalog_routes_to_visible_source() {
    with_grid(|entity, visual, source_path| {
        visual.update_window_entity(entity, |w, _, _| {
            w.closed_tabs.push(crate::tab::ClosedTab { buffer: crate::io::read_buffer(source_path).unwrap(), session_id: uuid::Uuid::new_v4() });
        });
        press_action(visual, "FileRestoreClosedTab");
        visual.update(|_, cx| assert_eq!(entity.read(cx).active, 2));
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_source_view_actions_already_route_correctly() {
    for universal in [false, true] {
        with_grid(|entity, visual, _| {
            if universal { visual.dispatch_action(crate::actions::ViewSource); }
            else { visual.dispatch_action(crate::actions::ViewCsvSource); }
            assert_source_input(entity, visual);
        });
    }
}

#[test]
fn csv_grid_focus_tab_change_keeps_newer_goto_field() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |w, window, cx| {
            w.set_active_tab(1, cx);
            w.goto_state.open = true; window.focus(&w.goto_focus, cx); cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.goto_focus.is_focused(window));
            assert_eq!(w.buffer().rope.to_string(), "source 中😀\n");
        });
    });
}

#[test]
fn csv_grid_focus_readonly_destination_receives_focus_without_editing() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |w, _, _| w.tabs[1].buffer.read_only = true);
        press_action(visual, "TabGoto2");
        visual.simulate_input("must refuse");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.focus_handle.is_focused(window));
            assert_eq!(w.buffer().rope.to_string(), "source 中😀\n");
            assert!(!w.buffer().dirty);
        });
    });
}

#[test]
fn csv_grid_focus_refused_draft_stays_on_original_tab() {
    for action in ["TabGoto2", "FileNew", "FileCloseTab", "FileCloseAllTabs", "FileCloseAllUnchanged", "FileRestoreClosedTab"] {
        with_grid(|entity, visual, source_path| {
            visual.update_window_entity(entity, |w, window, cx| {
                w.closed_tabs.push(crate::tab::ClosedTab { buffer: crate::io::read_buffer(source_path).unwrap(), session_id: uuid::Uuid::new_v4() });
                open_cell_editor(w, 0, 0, window, cx);
                let model = w.csv_models.get_mut(&w.tabs[0].session_id).unwrap();
                model.cell_input.as_mut().unwrap().buffer = "refused 😀".into();
                w.tabs[0].buffer.read_only = true; cx.notify();
            });
            visual.run_until_parked();
            if action == "FileCloseAllUnchanged" { visual.dispatch_action(crate::actions::FileCloseAllUnchanged); }
            else { press_action(visual, action); }
            visual.update(|window, cx| {
                let w = entity.read(cx);
                assert_eq!(w.active, 0, "{action}");
                let input = w.csv_models.get(&w.tabs[0].session_id).unwrap().cell_input.as_ref().unwrap();
                assert_eq!(input.buffer, "refused 😀");
                assert!(input.focus_handle.is_focused(window));
                assert!(!w.buffer().dirty);
            });
        });
    }
}

#[test]
fn csv_grid_focus_horizontal_and_vertical_mouse_tabs_route_to_source() {
    for vertical in [false, true] {
        with_grid_mode(vertical, |entity, visual, _| {
            let bounds = visual.debug_bounds(if vertical { "vtab-1" } else { "tab-1" }).unwrap();
            visual.simulate_click(bounds.center(), gpui::Modifiers::default());
            visual.update(|_, cx| assert_eq!(entity.read(cx).active, 1));
            assert_source_input(entity, visual);
        });
    }
}

#[test]
fn csv_grid_focus_inactive_window_defers_handoff_until_activation() {
    with_grid(|entity, visual, _| {
        visual.deactivate_window();
        visual.update_window_entity(entity, |w, _, cx| w.set_active_tab(1, cx));
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(!window.is_window_active());
            assert!(!w.focus_handle.is_focused(window));
        });
        visual.update_window_entity(entity, |_, window, _| window.activate_window());
        visual.run_until_parked();
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_visible_grid_keeps_ownership_after_rerender() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |_, _, cx| cx.notify());
        visual.run_until_parked();
        press_action(visual, "EditSelectAll");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.input_focus(window) == InputFocus::CsvGrid);
            assert_eq!(w.csv_models.get(&w.tabs[0].session_id).unwrap().selected_rows.len(), 2);
            assert!(!w.buffer().dirty);
        });
    });
}

#[test]
fn csv_grid_focus_loaded_transferred_source_routes_to_visible_editor() {
    with_grid(|entity, visual, source_path| {
        let mut loaded = Tab::empty();
        loaded.buffer = crate::io::read_buffer(source_path).unwrap();
        visual.update_window_entity(entity, |w, _, cx| w.adopt_transferred_tab(loaded, cx));
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_rich_doc_target_edits_ast_and_preserves_raw_source() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |w, _, cx| {
            let mut payload = crate::rich_doc::rtf::open_bytes(b"{\\rtf1\\ansi rich text}".to_vec()).unwrap();
            payload.active_block = Some(0);
            w.tabs[1].buffer.rope = ropey::Rope::from_str(&payload.raw_view_text);
            w.tabs[1].buffer.rich_doc = Some(payload);
            w.tabs[1].view_mode = TabViewMode::RichDocView;
            cx.notify();
        });
        press_action(visual, "TabGoto2");
        press_action(visual, "EditSelectAll");
        visual.simulate_input("visible edit 😀");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.rich_doc_body_focus.as_ref().unwrap().is_focused(window));
            let payload = w.buffer().rich_doc.as_ref().unwrap();
            assert_eq!(payload.block_text(0), "visible edit 😀");
            assert_eq!(w.buffer().rope.to_string(), payload.raw_view_text);
        });
    });
}

#[test]
fn csv_grid_focus_markdown_preview_target_can_return_to_editable_source() {
    with_grid(|entity, visual, source_path| {
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs[1].buffer.file_path = Some(source_path.with_extension("md"));
            w.tabs[1].view_mode = TabViewMode::MarkdownPreview;
            cx.notify();
        });
        press_action(visual, "TabGoto2");
        visual.run_until_parked();
        visual.update(|window, cx| assert!(entity.read(cx).focus_handle.is_focused(window)));
        visual.dispatch_action(crate::actions::ViewSource);
        assert_source_input(entity, visual);
    });
}

#[test]
fn csv_grid_focus_another_csv_preview_target_keeps_grid_input_routing() {
    with_grid(|entity, visual, source_path| {
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs[1].buffer.file_path = Some(source_path.with_extension("csv"));
            w.tabs[1].buffer.rope = ropey::Rope::from_str("label,value\nsecond,3\n");
            w.tabs[1].view_mode = TabViewMode::CsvPreview;
            cx.notify();
        });
        press_action(visual, "TabGoto2");
        visual.run_until_parked();
        press_action(visual, "EditSelectAll");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.input_focus(window) == InputFocus::CsvGrid);
            assert_eq!(w.csv_models.get(&w.tabs[1].session_id).unwrap().selected_rows.len(), 1);
            assert!(!w.buffer().dirty);
        });
    });
}

#[test]
fn csv_grid_focus_horizontal_and_vertical_mouse_close_route_to_source() {
    for vertical in [false, true] {
        with_grid_mode(vertical, |entity, visual, _| {
            let bounds = visual.debug_bounds(if vertical { "vtab-0" } else { "tab-0" }).unwrap();
            visual.simulate_click(gpui::point(bounds.right() - gpui::px(12.), bounds.center().y), gpui::Modifiers::default());
            visual.update(|_, cx| assert_eq!(entity.read(cx).tabs.len(), 1));
            assert_source_input(entity, visual);
        });
    }
}

#[test]
fn csv_grid_close_unchanged_all_clean_never_renders_empty_tabs() {
    with_grid(|entity, visual, _| {
        visual.update_window_entity(entity, |w, window, cx| w.close_all_unchanged(window, cx));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs.len(), 1);
            assert!(w.active < w.tabs.len());
        });
    });
}

#[test]
fn csv_grid_focus_non_csv_save_as_returns_to_visible_source() {
    with_grid(|entity, visual, source_path| {
        let destination = source_path.with_file_name("grid-saved.txt");
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs[0].buffer.file_path = Some(destination.clone());
            w.write_active_to_disk_as().unwrap();
            cx.notify();
        });
        assert!(std::fs::read(&destination).unwrap().starts_with(&[0xef, 0xbb, 0xbf]));
        visual.run_until_parked();
        visual.update(|window, cx| assert!(entity.read(cx).focus_handle.is_focused(window), "non-CSV Save As displays Source"));
        visual.simulate_input("visible edit 😀");
        visual.update(|_, cx| assert!(entity.read(cx).buffer().rope.to_string().contains("visible edit 😀")));
    });
}

#[test]
fn csv_grid_focus_background_close_and_reorder_preserve_visible_grid() {
    with_grid(|entity, visual, _| {
        let id = visual.update_window_entity(entity, |w, _, cx| {
            let id = w.tabs[0].session_id;
            w.move_tab(0, 1, cx);
            id
        });
        visual.run_until_parked();
        visual.update_window_entity(entity, |w, window, cx| w.close_tab_at(0, window, cx));
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[w.active].session_id, id);
            assert!(w.input_focus(window) == InputFocus::CsvGrid);
        });
    });
}

#[test]
fn csv_grid_focus_inactive_tab_change_keeps_newer_field_on_activation() {
    with_grid(|entity, visual, _| {
        visual.deactivate_window();
        visual.update_window_entity(entity, |w, window, cx| {
            w.set_active_tab(1, cx); w.goto_state.open = true;
            window.focus(&w.goto_focus, cx); cx.notify();
        });
        visual.run_until_parked();
        visual.update_window_entity(entity, |_, window, _| window.activate_window());
        visual.run_until_parked();
        visual.update(|window, cx| assert!(entity.read(cx).goto_focus.is_focused(window)));
    });
}
