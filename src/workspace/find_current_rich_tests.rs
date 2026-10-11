use super::*;
use crate::actions::{MoveDown, MoveLineStart, MoveDocStart, SelectLineEnd, SelectRight, MoveRight};
use crate::{settings::Settings, tab::{Tab, TabViewMode}};
use gpui::TestAppContext;

fn painted_current(rich: bool, scoped: bool, count: bool, anchored: bool) {
    painted_current_impl(rich, scoped, count, anchored, false);
}

fn painted_current_impl(rich: bool, scoped: bool, count: bool, anchored: bool, click: bool) {
    painted_current_navigation(rich, scoped, count, anchored, click, false);
}
fn painted_current_navigation(rich: bool, scoped: bool, count: bool, anchored: bool, click: bool, cross_tab: bool) {
    painted_current_owned_row(rich, scoped, count, anchored, click, cross_tab, false);
}
fn painted_current_owned_row(rich: bool, scoped: bool, count: bool, anchored: bool, click: bool, cross_tab: bool, stale_row: bool) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., gpui::WindowAppearance::Dark,
        );
        let mut tab = Tab::empty();
        if rich {
            let mut payload = crate::rich_doc::rtf::open_bytes(
                br"{\rtf1\ansi outside hit\par inside hit\par outside hit}".to_vec(),
            ).unwrap();
            let pos = payload.block_start(0);
            payload.selection = Some(crate::rich_doc::TextSelection { anchor: pos, cursor: pos });
            payload.active_block = Some(0);
            tab.buffer.rope = ropey::Rope::from_str(&payload.raw_view_text);
            tab.buffer.rich_doc = Some(payload);
            tab.view_mode = TabViewMode::RichDocView;
        } else {
            tab.buffer.rope = ropey::Rope::from_str("outside hit\r\n\u{4e2d} hit caf\u{e9}\r\noutside hit");
        }
        w.tabs = vec![tab];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.find_state.query = if rich { if click { "inside" } else { "newhit" } } else if anchored { "^hit" } else { "hit" }.into();
        w.find_state.regex = anchored;
        w.dialog_tab = DialogTab::Find;
        w.find_state.open = true;
        w
    });
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        window.activate_window();
        let focus = if rich { w.ensure_rich_doc_body_focus(cx) } else { w.focus_handle.clone() };
        window.focus(&focus, cx);
    }));
    visual.run_until_parked();
    if rich {
        let mut handler = visual.update(|window, _| window.take_registered_input_handler().unwrap());
        visual.update(|window, cx| handler.replace_text_in_range(None, "newhit ", window, cx));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(!w.buffer().rope.to_string().contains("newhit"));
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().flat_search_text().0,
                "newhit outside hit\ninside hit\noutside hit");
            assert_eq!(w.buffer().rich_doc.as_ref().unwrap().undo_stack.len(), 1);
        });
        if scoped {
            visual.dispatch_action(MoveDocStart);
            for _ in 0..6 { visual.dispatch_action(SelectRight); }
        }
    } else if scoped {
        visual.dispatch_action(MoveDown);
        visual.dispatch_action(MoveLineStart);
        if anchored {
            visual.dispatch_action(MoveRight);
            visual.dispatch_action(MoveRight);
            for _ in 0..3 { visual.dispatch_action(SelectRight); }
        } else {
            visual.dispatch_action(SelectLineEnd);
        }
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().slice_bytes(w.selection().ordered()), if anchored { "hit" } else { "\u{4e2d} hit caf\u{e9}" });
        });
    }
    let before = visual.update(|_, cx| {
        let w = entity.read(cx);
        (w.buffer().rope.clone(), w.buffer().raw_revision.clone(), w.buffer().dirty,
            w.selection(), w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len(),
            w.buffer().rich_doc.clone())
    });
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| {
        crate::ui::find_dialog::FindDialog::new(weak, window, cx)
    });
    dialog.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).find_focus.clone(), cx);
    });
    dialog.run_until_parked();
    if scoped {
        let checkbox = dialog.debug_bounds("dialog-checkbox-In selection").unwrap();
        dialog.simulate_click(checkbox.center(), gpui::Modifiers::none());
        dialog.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.find_state.in_selection);
            assert!(w.find_state.in_selection_scope.is_some());
            if rich {
                assert_eq!(w.find_state.in_selection_scope, Some(0..6));
            }
        });
    }
    let button = dialog.debug_bounds(if count { "dialog-button-Count" } else {
        "dialog-button-Find All in Current Document"
    }).unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    if rich && !count {
        for _ in 0..20 {
            visual.cx.executor().advance_clock(std::time::Duration::from_millis(16));
            visual.run_until_parked();
        }
    }
    visual.update(|_, cx| {
        let w = entity.read(cx);
        assert!(w.buffer().rope.is_instance(&before.0));
        assert_eq!(w.buffer().raw_revision, before.1);
        assert_eq!(w.buffer().dirty, before.2);
        assert_eq!(w.selection().anchor, before.3.anchor);
        assert_eq!(w.selection().head, before.3.head);
        assert_eq!(w.selection().is_dragging, before.3.is_dragging);
        assert_eq!(w.tabs[0].undo_stack.len(), before.4);
        assert_eq!(w.tabs[0].redo_stack.len(), before.5);
        if let Some(old) = &before.6 {
            let current = w.buffer().rich_doc.as_ref().unwrap();
            assert_eq!(current.ast, old.ast);
            assert_eq!(current.revision, old.revision);
            assert_eq!(current.selection, old.selection);
            assert_eq!(current.edit_log, old.edit_log);
            assert_eq!(current.undo_stack.len(), old.undo_stack.len());
            assert_eq!(current.redo_stack.len(), old.redo_stack.len());
        }
        let expected = if anchored { 0 } else if rich || scoped { 1 } else { 3 };
        eprintln!("painted current: rich={rich} scoped={scoped} count={count} matches={} panel={} status={:?}",
            w.find_state.matches.len(), w.find_in_files.results.len(), w.find_state.status);
        if count {
            assert_eq!(w.find_state.status.as_deref(), Some(if anchored { "0 matches" } else { "1 match" }));
        } else {
            assert_eq!(w.find_in_files.results.len(), expected);
            if scoped && !rich && !anchored {
                let hit = &w.find_in_files.results[0];
                assert_eq!(hit.line, 1);
                assert_eq!(hit.match_in_line, 4..7);
                assert_eq!(hit.line_text, "\u{4e2d} hit caf\u{e9}");
            } else if rich && !click {
                let hit = &w.find_in_files.results[0];
                assert_eq!(hit.line, 0);
                assert_eq!(hit.match_in_line, 0..6);
                assert_eq!(hit.line_text, "newhit outside hit");
            }
        }
    });
    if click {
        let target = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        if cross_tab {
            visual.update(|_, cx| entity.update(cx, |workspace, cx| {
                workspace.tabs.push(Tab::empty());
                workspace.set_active_tab(1, cx);
                workspace.tabs.swap(0, 1);
                workspace.active = 0;
                workspace.tabs[1].buffer.new_label = Some("renamed".into());
            }));
        }
        visual.run_until_parked();
        let row = visual.debug_bounds("fif-result-0").unwrap();
        if stale_row {
            visual.update(|window, cx| {
                let before = entity.read(cx).buffer().rich_doc.as_ref().unwrap().selection;
                entity.update(cx, |workspace, cx| {
                    workspace.find_state.query = "newhit".into();
                    workspace.find_all_current(&crate::actions::EditFindAllCurrent, window, cx);
                });
                window.simulate_mouse_down(gpui::MouseDownEvent { position: row.center(), ..Default::default() }, cx);
                assert_eq!(entity.read(cx).buffer().rich_doc.as_ref().unwrap().selection, before);
            });
            for _ in 0..20 {
                visual.cx.executor().advance_clock(std::time::Duration::from_millis(16)); visual.run_until_parked();
            }
        }
        let row = visual.debug_bounds("fif-result-0").unwrap();
        visual.simulate_click(row.center(), gpui::Modifiers::none());
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[w.active].session_id, target);
            let payload = w.buffer().rich_doc.as_ref().unwrap();
            let (flat, offsets) = payload.flat_search_text();
            let selection = payload.selection.unwrap();
            let a = payload.global_flat_for_pos(&offsets, selection.anchor);
            let b = payload.global_flat_for_pos(&offsets, selection.cursor);
            assert_eq!(flat.get(a.min(b)..a.max(b)), Some(if stale_row { "newhit" } else { "inside" }));
            assert_eq!(payload.ast, before.6.as_ref().unwrap().ast);
            assert_eq!(payload.revision, before.6.as_ref().unwrap().revision);
            assert_eq!(payload.undo_stack.len(), before.6.as_ref().unwrap().undo_stack.len());
            assert_eq!(w.selection().anchor, before.3.anchor);
            assert_eq!(w.selection().head, before.3.head);
        });
    }
    cx.quit();
}

#[test]
fn painted_source_find_all_current_limits_results_to_real_selected_line() {
    painted_current(false, true, false, false);
}
#[test]
fn painted_source_find_all_current_full_document_control() {
    painted_current(false, false, false, false);
}
#[test]
fn painted_source_count_real_selected_line_control() {
    painted_current(false, true, true, false);
}
#[test]
fn painted_rich_find_all_current_finds_actual_committed_native_input() {
    painted_current(true, false, false, false);
}
#[test]
fn painted_rich_count_actual_committed_native_input_control() {
    painted_current(true, false, true, false);
}
#[test]
fn painted_rich_find_all_current_finds_actual_selected_native_input() {
    painted_current(true, true, false, false);
}
#[test]
fn painted_rich_count_actual_selected_native_input_control() {
    painted_current(true, true, true, false);
}

#[test]
fn painted_source_find_all_current_preserves_document_regex_anchor_context() {
    painted_current(false, true, false, true);
}
#[test]
fn painted_source_count_scope_relative_regex_anchor_control() {
    painted_current(false, true, true, true);
}

#[test]
fn painted_rich_find_all_current_result_click_selects_visible_ast_text() {
    painted_current_impl(true, false, false, false, true);
}

#[test]
fn painted_rich_row_navigates_after_unrelated_activation_reorder_and_rename() {
    painted_current_navigation(true, false, false, false, true, true);
}

#[test]
fn painted_old_rich_row_refuses_replacement_then_new_row_navigates() {
    painted_current_owned_row(true, false, false, false, true, false, true);
}
