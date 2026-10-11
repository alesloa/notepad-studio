use super::*;
use crate::{settings::Settings, tab::TabViewMode};
use gpui::{Entity, VisualTestContext};
use std::time::Duration;

fn frames(visual: &mut VisualTestContext, count: usize) {
    for _ in 0..count {
        visual.cx.executor().advance_clock(Duration::from_millis(16));
        visual.run_until_parked();
    }
}

fn held(
    exercise: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, &mut VisualTestContext),
    expect_publication: bool,
) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        autocompletion_mode: crate::settings::AutocompletionMode::None, ..Default::default()
    });
    let mut cx = gpui::TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (release, gate) = futures::channel::oneshot::channel();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut workspace = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::empty();
        let input = format!("{{\\rtf1\\ansi {}\\par inside hit}}", "word ".repeat(70_000));
        let mut payload = crate::rich_doc::rtf::open_bytes(input.into_bytes()).unwrap();
        let start = payload.block_start(0);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor: start, cursor: start });
        payload.active_block = Some(0);
        tab.buffer.rope = ropey::Rope::from_str(&payload.raw_view_text);
        tab.buffer.rich_doc = Some(payload); tab.view_mode = TabViewMode::RichDocView;
        workspace.tabs = vec![tab]; workspace.active = 0;
        workspace.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        workspace.session_final_save_result = Some(true);
        workspace.rich_find_current.hold = Some(gate);
        workspace.find_state.query = "newhit".into(); workspace.find_state.open = true;
        workspace.dialog_tab = DialogTab::Find;
        workspace
    });
    visual.update(|window, cx| entity.update(cx, |workspace, cx| {
        window.activate_window(); let focus = workspace.ensure_rich_doc_body_focus(cx); window.focus(&focus, cx);
    }));
    visual.run_until_parked();
    let mut input = visual.update(|window, _| window.take_registered_input_handler().unwrap());
    visual.update(|window, cx| input.replace_text_in_range(None, "newhit ", window, cx));
    visual.run_until_parked();
    let weak = entity.downgrade();
    let (_, dialog) = visual.cx.add_window_view(|window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx));
    let mut dialog = dialog.clone();
    dialog.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).find_focus.clone(), cx); });
    dialog.run_until_parked();
    let button = dialog.debug_bounds("dialog-button-Find All in Current Document").unwrap();
    dialog.simulate_click(button.center(), gpui::Modifiers::none());
    visual.run_until_parked(); frames(visual, 8);
    visual.update(|_, cx| {
        let workspace = entity.read(cx);
        assert!(workspace.find_in_files.results.is_empty());
        assert!(workspace.find_in_files.searching);
        assert!(workspace.rich_find_current.frame_max_bytes <= 64 * 1024);
        assert!(workspace.rich_find_current.frame_max_nodes <= 1024);
        // The held real worker leaves four aggregate permits in the channel;
        // producer retries neither allocate nor copy further source text.
        assert!(workspace.rich_find_current.owned_bytes() < 1024 * 1024);
    });
    let used = visual.update(|_, cx| entity.read(cx).rich_find_current.owned_bytes());
    frames(visual, 10);
    assert_eq!(visual.update(|_, cx| entity.read(cx).rich_find_current.owned_bytes()), used);
    exercise(&entity, visual, &mut dialog);
    release.send(()).unwrap(); visual.run_until_parked(); frames(visual, 40);
    visual.update(|_, cx| {
        let workspace = entity.read(cx);
        assert_eq!(workspace.find_in_files.results.len(), usize::from(expect_publication));
        assert!(workspace.rich_find_current.owned_peak() <= 192 * 1024 * 1024);
    });
    visual.update(|_, cx| entity.update(cx, |workspace, _| workspace.release_session_window()));
    visual.run_until_parked();
    visual.update(|_, cx| {
        let workspace = entity.read(cx);
        assert_eq!(workspace.rich_find_current.owned_bytes(), workspace.rich_find_current.control_bytes());
    });
    cx.quit();
}

#[test]
fn actual_held_worker_keeps_capture_bounded_then_publishes_native_input() {
    held(|_, _, _| {}, true);
}
#[test]
fn actual_match_case_checkbox_invalidates_held_find_all() {
    held(|_, _, dialog| {
        let checkbox = dialog.debug_bounds("dialog-checkbox-Match case").unwrap();
        dialog.simulate_click(checkbox.center(), gpui::Modifiers::none());
    }, false);
}
#[test]
fn actual_doc_view_switch_invalidates_held_find_all() {
    held(|_, visual, _| { visual.dispatch_action(crate::actions::rich_doc::RichDocRaw); }, false);
}
#[test]
fn actual_native_edit_then_undo_cannot_revive_held_find_all() {
    held(|entity, visual, _| {
        let revision = visual.update(|_, cx| entity.read(cx).buffer().rich_doc.as_ref().unwrap().revision.clone());
        visual.update(|window, cx| entity.update(cx, |workspace, cx| {
            window.activate_window(); let focus = workspace.ensure_rich_doc_body_focus(cx); window.focus(&focus, cx);
        }));
        visual.run_until_parked();
        let mut input = visual.update(|window, _| window.take_registered_input_handler().unwrap());
        visual.update(|window, cx| input.replace_text_in_range(None, "x", window, cx));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rich_doc.as_ref().unwrap().revision, revision));
    }, false);
}
#[test]
fn same_uuid_reopened_incarnation_cannot_adopt_held_find_all() {
    held(|entity, visual, _| {
        visual.update(|_, cx| entity.update(cx, |workspace, _| {
            let uuid = workspace.tabs[0].session_id;
            let payload = workspace.tabs[0].buffer.rich_doc.clone();
            let mut replacement = Tab::empty(); replacement.session_id = uuid;
            replacement.buffer.rich_doc = payload; replacement.view_mode = TabViewMode::RichDocView;
            workspace.tabs[0] = replacement;
        }));
    }, false);
}
#[test]
fn window_release_cancels_the_actual_held_pipeline() {
    held(|entity, visual, _| {
        visual.update(|_, cx| entity.update(cx, |workspace, _| workspace.release_session_window()));
    }, false);
}

#[test]
fn held_notice_receiver_cannot_lose_started_or_completion() {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut cx = gpui::TestAppContext::single();
    let (release, gate) = futures::channel::oneshot::channel();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut workspace = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rich_doc = Some(crate::rich_doc::rtf::open_bytes(b"{\\rtf1\\ansi inside hit}".to_vec()).unwrap());
        tab.view_mode = TabViewMode::RichDocView;
        workspace.tabs = vec![tab]; workspace.active = 0;
        workspace.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        workspace.session_final_save_result = Some(true);
        workspace.find_state.query = "inside".into();
        workspace.rich_find_current.hold_notices = Some(gate);
        workspace
    });
    // Hold the real driver's receiver before any background polling and fill
    // its actual channel through the same Wake path used by editor actions.
    visual.update(|window, cx| entity.update(cx, |workspace, cx| {
        workspace.find_all_current(&crate::actions::search::EditFindAllCurrent, window, cx);
        workspace.rich_find_current.wake_for_test();
        workspace.rich_find_current.wake_for_test();
    }));
    visual.run_until_parked();
    release.send(()).unwrap();
    visual.run_until_parked(); frames(visual, 40);
    visual.update(|_, cx| {
        let workspace = entity.read(cx);
        assert!(!workspace.find_in_files.searching, "completion must be recoverable after notice saturation");
        assert_eq!(workspace.find_in_files.results.len(), 1);
    });
    visual.update(|_, cx| entity.update(cx, |workspace, _| workspace.release_session_window()));
    visual.run_until_parked(); cx.quit();
}

#[test]
fn changed_query_cannot_publish_held_find_all() {
    held(|entity, visual, _| {
        visual.update(|_, cx| entity.update(cx, |workspace, _| workspace.find_state.query = "word".into()));
    }, false);
}
#[test]
fn actual_in_selection_checkbox_cannot_adopt_held_whole_document_search() {
    held(|_, _, dialog| {
        let checkbox = dialog.debug_bounds("dialog-checkbox-In selection").unwrap();
        dialog.simulate_click(checkbox.center(), gpui::Modifiers::none());
    }, false);
}
#[test]
fn unrelated_active_tab_cancels_held_capture_without_changing_its_buffer() {
    held(|entity, visual, _| {
        visual.update(|_, cx| entity.update(cx, |workspace, cx| {
            workspace.tabs.push(Tab::empty()); workspace.set_active_tab(1, cx);
        }));
    }, false);
}
#[test]
fn closed_source_cannot_publish_held_result_into_successor() {
    held(|entity, visual, _| {
        visual.update(|_, cx| entity.update(cx, |workspace, _| {
            workspace.tabs.remove(0); workspace.tabs.push(Tab::empty()); workspace.active = 0;
        }));
    }, false);
}

#[test]
fn actual_close_button_preserves_held_document_search() {
    held(|entity, visual, dialog| {
        let handle = dialog.update(|window, _| {
            window.window_handle().downcast::<crate::ui::find_dialog::FindDialog>().unwrap()
        });
        visual.update(|_, cx| entity.update(cx, |workspace, _| {
            workspace.find_dialog_window = Some(handle);
        }));
        let button = dialog.debug_bounds("dialog-button-Close").unwrap();
        dialog.simulate_click(button.center(), gpui::Modifiers::none());
        visual.run_until_parked();
        visual.update(|_, cx| {
            let workspace = entity.read(cx);
            assert!(workspace.find_dialog_window.is_none());
            assert!(!workspace.find_state.open);
            assert!(workspace.find_in_files.searching, "closing an idle directory dialog must preserve the document search owner");
        });
    }, true);
}
