use futures::channel::oneshot;

pub(super) async fn confirmation(
    receiver: Option<oneshot::Receiver<rfd::MessageDialogResult>>,
    _query: &str, _replacement: &str, _scope: &str,
) -> rfd::MessageDialogResult {
    match receiver {
        Some(receiver) => receiver.await.unwrap_or(rfd::MessageDialogResult::No),
        None => rfd::MessageDialogResult::No,
    }
}

use gpui::{Entity, TestAppContext, VisualTestContext, WindowAppearance};
use super::Workspace;
use crate::{actions::*, settings::Settings};
use std::path::Path;

#[derive(Clone, Copy)]
enum Route { Files, Projects }

fn settings(root: &Path) -> crate::settings::TestSettingsGuard {
    crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        folder_panel_roots: vec![root.to_path_buf()],
        ..Settings::default()
    })
}

fn fixture(cx: &mut TestAppContext, root: &Path, route: Route)
    -> (Entity<Workspace>, VisualTestContext, VisualTestContext)
{
    let (entity, editor) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., WindowAppearance::Dark,
        );
        w.find_in_files.directory = Some(root.to_path_buf());
        w.find_in_files.directory_text = root.display().to_string();
        w.find_in_files.query = "hit".into();
        w.find_in_files.replacement = "done".into();
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    let mut editor = editor.clone();
    editor.update(|window, cx| {
        window.activate_window();
        let close = entity.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            close.update(cx, crate::app::quit::save_before_window_close).unwrap_or(false)
        });
        entity.update(cx, |w, cx| match route {
            Route::Files => w.open_find_in_files(&SearchFindInFiles, window, cx),
            Route::Projects => w.open_find_in_projects(&SearchFindInProjects, window, cx),
        });
    });
    editor.run_until_parked();
    let handle = cx.update(|cx| entity.read(cx).find_dialog_window.unwrap());
    let dialog = VisualTestContext::from_window(handle.into(), cx);
    (entity, editor, dialog)
}

fn request(entity: &Entity<Workspace>, dialog: &mut VisualTestContext, route: Route)
    -> oneshot::Sender<rfd::MessageDialogResult>
{
    let (sender, receiver) = oneshot::channel();
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        w.replace_audit_confirmations.push_back(receiver);
        match route {
            Route::Files => w.find_in_files_replace(&FindInFilesReplace, window, cx),
            Route::Projects => w.find_in_projects_replace(&FindInProjectsReplace, window, cx),
        }
    }));
    dialog.run_until_parked();
    sender
}

fn neutral(root: &Path) -> std::path::PathBuf {
    let file = root.join("neutral.txt");
    std::fs::write(&file, b"hit\r\n").unwrap();
    file
}

fn positive(route: Route, yes: bool) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    let sender = request(&entity, &mut dialog, route);
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n");
    sender.send(if yes { rfd::MessageDialogResult::Yes } else { rfd::MessageDialogResult::No }).unwrap();
    dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), if yes { b"done\r\n".as_slice() } else { b"hit\r\n".as_slice() },
        "status: {:?}", dialog.cx.update(|cx| entity.read(cx).find_in_files.status.clone()));
    dialog.cx.update(|cx| assert!(!entity.read(cx).find_in_files.searching));
    cx.quit();
}

#[test] fn files_valid_yes_writes_once() { positive(Route::Files, true); }
#[test] fn projects_valid_yes_writes_once() { positive(Route::Projects, true); }
#[test] fn files_no_leaves_private_file_unchanged() { positive(Route::Files, false); }
#[test] fn projects_no_leaves_private_file_unchanged() { positive(Route::Projects, false); }

fn closed(route: Route, drop_entity: bool, saved: bool) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, mut dialog) = fixture(&mut cx, dir.path(), route);
    editor.update(|_, cx| entity.update(cx, |w, _| w.session_final_save_result = Some(saved)));
    let sender = request(&entity, &mut dialog, route);
    let weak = entity.downgrade();
    assert_eq!(editor.simulate_close(), saved);
    if saved {
        editor.update(|window, _| window.remove_window());
        dialog.update(|window, _| window.remove_window());
        dialog.run_until_parked();
    }
    if drop_entity {
        drop(entity);
        dialog.run_until_parked();
        assert!(weak.upgrade().is_none(), "dropped fixture must release Workspace");
    } else {
        dialog.cx.update(|cx| assert_eq!(entity.read(cx).window_closed, saved));
    }
    let sent = sender.send(rfd::MessageDialogResult::Yes);
    if !saved { sent.unwrap(); }
    dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), if saved { b"hit\r\n".as_slice() } else { b"done\r\n".as_slice() });
    cx.quit();
}

#[test] fn files_closed_retained_workspace_refuses_late_yes() { closed(Route::Files, false, true); }
#[test] fn projects_closed_retained_workspace_refuses_late_yes() { closed(Route::Projects, false, true); }
#[test] fn files_dropped_workspace_refuses_late_yes() { closed(Route::Files, true, true); }
#[test] fn projects_dropped_workspace_refuses_late_yes() { closed(Route::Projects, true, true); }
#[test] fn files_failed_close_preserves_live_confirmation() { closed(Route::Files, false, false); }
#[test] fn projects_failed_close_preserves_live_confirmation() { closed(Route::Projects, false, false); }

fn dialog_closed(route: Route) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    let sender = request(&entity, &mut dialog, route);
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        crate::ui::find_dialog::close_find_dialog(w, window, cx);
    }));
    dialog.run_until_parked();
    dialog.cx.update(|cx| {
        let w = entity.read(cx);
        assert!(!w.find_state.open);
        assert_eq!(w.find_in_files.status.as_deref(), Some("Directory replacement cancelled"));
    });
    let _ = sender.send(rfd::MessageDialogResult::Yes);
    dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n");
    cx.quit();
}

#[test] fn files_dialog_close_refuses_late_yes() { dialog_closed(Route::Files); }
#[test] fn projects_dialog_close_refuses_late_yes() { dialog_closed(Route::Projects); }

fn old_no(route: Route) {
    let dir = tempfile::tempdir().unwrap(); let _file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, mut dialog) = fixture(&mut cx, dir.path(), route);
    let first = request(&entity, &mut dialog, route);
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        crate::ui::find_dialog::close_find_dialog(w, window, cx);
    }));
    editor.update(|window, cx| entity.update(cx, |w, cx| match route {
        Route::Files => w.open_find_in_files(&SearchFindInFiles, window, cx),
        Route::Projects => w.open_find_in_projects(&SearchFindInProjects, window, cx),
    }));
    editor.run_until_parked();
    let handle = cx.update(|cx| entity.read(cx).find_dialog_window.unwrap());
    dialog = VisualTestContext::from_window(handle.into(), &cx);
    let second = request(&entity, &mut dialog, route);
    let status = dialog.cx.update(|cx| entity.read(cx).find_in_files.status.clone());
    let _ = first.send(rfd::MessageDialogResult::No);
    dialog.run_until_parked();
    dialog.cx.update(|cx| assert_eq!(entity.read(cx).find_in_files.status, status,
        "older cancellation must not overwrite the newer pending request"));
    second.send(rfd::MessageDialogResult::No).unwrap(); dialog.run_until_parked(); cx.quit();
}

#[test] fn files_old_no_preserves_new_confirmation_status() { old_no(Route::Files); }
#[test] fn projects_old_no_preserves_new_confirmation_status() { old_no(Route::Projects); }

fn busy_search(route: Route) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    let (_sender, receiver) = oneshot::channel();
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        match route {
            Route::Files => w.run_find_in_files(window, cx),
            Route::Projects => w.run_find_in_projects(window, cx),
        }
        assert!(w.find_in_files.searching, "actual search must own the busy state");
        let status = w.find_in_files.status.clone();
        w.replace_audit_confirmations.push_back(receiver);
        match route {
            Route::Files => w.find_in_files_replace(&FindInFilesReplace, window, cx),
            Route::Projects => w.find_in_projects_replace(&FindInProjectsReplace, window, cx),
        }
        assert_eq!(w.replace_audit_confirmations.len(), 1,
            "replacement must not launch confirmation while the actual search is busy");
        assert_eq!(w.find_in_files.status, status);
    }));
    dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n"); cx.quit();
}

#[test] fn files_busy_search_refuses_replace_confirmation() { busy_search(Route::Files); }
#[test] fn projects_busy_search_refuses_replace_confirmation() { busy_search(Route::Projects); }

fn changed_scope(route: Route, change: impl FnOnce(&mut Workspace)) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    let sender = request(&entity, &mut dialog, route);
    dialog.update(|_, cx| entity.update(cx, |w, _| change(w)));
    sender.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n");
    dialog.cx.update(|cx| {
        assert!(!entity.read(cx).directory_operations.busy());
        assert!(!entity.read(cx).find_in_files.searching);
    }); cx.quit();
}

#[test] fn files_changed_query_refuses_yes() { changed_scope(Route::Files, |w| w.find_in_files.query = "other".into()); }
#[test] fn projects_changed_query_refuses_yes() { changed_scope(Route::Projects, |w| w.find_in_files.query = "other".into()); }
#[test] fn files_changed_template_refuses_yes() { changed_scope(Route::Files, |w| w.find_in_files.replacement = "other".into()); }
#[test] fn projects_changed_options_refuse_yes() { changed_scope(Route::Projects, |w| w.find_in_files.match_case = !w.find_in_files.match_case); }
#[test] fn files_changed_directory_text_refuses_yes() { changed_scope(Route::Files, |w| w.find_in_files.directory_text.clear()); }
#[test] fn projects_changed_selected_roots_refuse_yes() { changed_scope(Route::Projects, |w| w.find_in_files.project_panel_1 = false); }

#[test]
fn files_snapshot_limit_refuses_before_confirmation() {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Files);
    dialog.update(|_, cx| entity.update(cx, |w, _| w.find_in_files.query = "x".repeat(1024 * 1024 + 1)));
    let _sender = request(&entity, &mut dialog, Route::Files);
    dialog.cx.update(|cx| {
        let w = entity.read(cx); assert!(!w.directory_operations.busy());
        assert_eq!(w.replace_audit_confirmations.len(), 1); assert!(w.find_in_files.error.is_some());
    });
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n"); cx.quit();
}

fn held_completion(route: Route) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, mut dialog) = fixture(&mut cx, dir.path(), route);
    let (ack, receiver) = oneshot::channel();
    dialog.update(|_, cx| entity.update(cx, |w, _| w.replace_audit_ack = Some(receiver)));
    let yes = request(&entity, &mut dialog, route);
    yes.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"done\r\n", "actual worker must finish before held acknowledgement");
    dialog.update(|window, cx| entity.update(cx, |w, cx| crate::ui::find_dialog::close_find_dialog(w, window, cx)));
    editor.run_until_parked();
    editor.update(|window, cx| entity.update(cx, |w, cx| {
        assert!(w.directory_operations.busy()); assert!(w.find_in_files.searching);
        match route {
            Route::Files => w.open_find_in_files(&SearchFindInFiles, window, cx),
            Route::Projects => w.open_find_in_projects(&SearchFindInProjects, window, cx),
        }
    })); editor.run_until_parked();
    let handle = cx.update(|cx| entity.read(cx).find_dialog_window.unwrap());
    dialog = VisualTestContext::from_window(handle.into(), &cx);
    let _refused = request(&entity, &mut dialog, route);
    dialog.cx.update(|cx| assert_eq!(entity.read(cx).replace_audit_confirmations.len(), 1));
    ack.send(()).unwrap(); dialog.run_until_parked();
    dialog.cx.update(|cx| {
        let w = entity.read(cx); assert!(!w.directory_operations.busy()); assert!(!w.find_in_files.searching);
        assert_eq!(w.find_in_files.status.as_deref(), Some("Cancelled after replacing 1 occurrence in 1 file"));
    });
    assert_eq!(std::fs::read(&file).unwrap(), b"done\r\n", "completed write is not rolled back"); cx.quit();
}

#[test] fn files_cancelling_slot_waits_for_actual_worker_ack() { held_completion(Route::Files); }
#[test] fn projects_cancelling_slot_waits_for_actual_worker_ack() { held_completion(Route::Projects); }

fn editor_origin(route: Route) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, mut dialog) = fixture(&mut cx, dir.path(), route);
    let yes = request(&entity, &mut editor, route);
    let pending = editor.cx.update(|cx| entity.read(cx).find_in_files.status.clone());
    dialog.update(|window, cx| entity.update(cx, |w, cx| crate::ui::find_dialog::close_find_dialog(w, window, cx)));
    editor.run_until_parked();
    editor.cx.update(|cx| {
        assert!(entity.read(cx).directory_operations.busy());
        assert_eq!(entity.read(cx).find_in_files.status, pending);
    });
    yes.send(rfd::MessageDialogResult::Yes).unwrap(); editor.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"done\r\n", "closing an unrelated dialog must preserve editor-origin confirmation");
    editor.cx.update(|cx| assert!(!entity.read(cx).directory_operations.busy())); cx.quit();
}

#[test] fn files_editor_origin_survives_unrelated_dialog_close() { editor_origin(Route::Files); }
#[test] fn projects_editor_origin_survives_unrelated_dialog_close() { editor_origin(Route::Projects); }

fn replaced_editor_root(route: Route, no: bool) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, _) = fixture(&mut cx, dir.path(), route);
    let yes = request(&entity, &mut editor, route);
    let before = editor.cx.update(|cx| entity.read(cx).find_in_files.status.clone());
    let replacement = editor.update(|window, cx| window.replace_root(cx, |_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true); w
    }));
    assert_ne!(replacement.entity_id(), entity.entity_id());
    editor.cx.update(|cx| assert!(!entity.read(cx).window_closed));
    let _ = yes.send(if no { rfd::MessageDialogResult::No } else { rfd::MessageDialogResult::Yes }); editor.run_until_parked();
    if no { editor.cx.update(|cx| assert_eq!(entity.read(cx).find_in_files.status, before)); }
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n");
    editor.cx.update(|cx| assert!(!entity.read(cx).directory_operations.busy())); cx.quit();
}

#[test] fn files_replaced_live_editor_root_refuses_old_workspace_confirmation() { replaced_editor_root(Route::Files, false); }
#[test] fn projects_replaced_live_editor_root_refuses_old_workspace_confirmation() { replaced_editor_root(Route::Projects, false); }

fn replaced_dialog_root(route: Route, no: bool) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    let yes = request(&entity, &mut dialog, route);
    let before = dialog.cx.update(|cx| entity.read(cx).find_in_files.status.clone());
    let previous = dialog.update(|window, _| window.root::<crate::ui::find_dialog::FindDialog>().flatten().unwrap());
    let weak = entity.downgrade();
    let replacement = dialog.update(|window, cx| window.replace_root(cx,
        |window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx)));
    assert_ne!(previous.entity_id(), replacement.entity_id());
    yes.send(if no { rfd::MessageDialogResult::No } else { rfd::MessageDialogResult::Yes }).unwrap(); dialog.run_until_parked();
    if no { dialog.cx.update(|cx| assert_eq!(entity.read(cx).find_in_files.status, before)); }
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n");
    dialog.cx.update(|cx| assert!(!entity.read(cx).directory_operations.busy()));
    drop(previous); cx.quit();
}

#[test] fn files_replaced_live_dialog_root_refuses_old_confirmation() { replaced_dialog_root(Route::Files, false); }
#[test] fn projects_replaced_live_dialog_root_refuses_old_confirmation() { replaced_dialog_root(Route::Projects, false); }

fn oversized_search_action(route: Route, filter: bool) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    let before = crate::settings::snapshot().find_history;
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        if filter { w.find_in_files.filter = "x".repeat(1024 * 1024 + 1); }
        else { w.find_in_files.query = "x".repeat(1024 * 1024 + 1); }
        match route {
            Route::Files => w.find_in_files_run_action(&FindInFilesRun, window, cx),
            Route::Projects => w.find_in_projects_run_action(&FindInProjectsRun, window, cx),
        }
        assert!(!w.directory_operations.busy()); assert!(w.find_in_files.error.is_some());
    })); dialog.run_until_parked();
    let after = crate::settings::snapshot().find_history;
    assert_eq!(after.fif_query, before.fif_query); assert_eq!(after.fif_filter, before.fif_filter);
    assert_eq!(after.fif_directory, before.fif_directory);
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n"); cx.quit();
}

#[test] fn files_oversized_query_action_does_not_publish_history() { oversized_search_action(Route::Files, false); }
#[test] fn files_oversized_filter_action_does_not_publish_history() { oversized_search_action(Route::Files, true); }
#[test] fn projects_oversized_query_action_does_not_publish_history() { oversized_search_action(Route::Projects, false); }
#[test] fn projects_oversized_filter_action_does_not_publish_history() { oversized_search_action(Route::Projects, true); }

fn completed_root_swap(route: Route, editor_origin: bool) {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, mut dialog) = fixture(&mut cx, dir.path(), route);
    let (ack, receiver) = oneshot::channel();
    dialog.update(|_, cx| entity.update(cx, |w, _| w.replace_audit_ack = Some(receiver)));
    let yes = if editor_origin { request(&entity, &mut editor, route) } else { request(&entity, &mut dialog, route) };
    yes.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"done\r\n");
    let before = dialog.cx.update(|cx| entity.read(cx).find_in_files.status.clone());
    let retained_dialog = if editor_origin {
        editor.update(|window, cx| window.replace_root(cx, |_, cx| {
            let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
            w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
            w.session_final_save_result = Some(true); w
        })); None
    } else {
        let previous = dialog.update(|window, _| window.root::<crate::ui::find_dialog::FindDialog>().flatten().unwrap());
        let weak = entity.downgrade();
        dialog.update(|window, cx| window.replace_root(cx,
            |window, cx| crate::ui::find_dialog::FindDialog::new(weak, window, cx)));
        Some(previous)
    };
    ack.send(()).unwrap(); dialog.run_until_parked();
    dialog.cx.update(|cx| {
        let w = entity.read(cx); assert!(!w.directory_operations.busy()); assert!(!w.find_in_files.searching);
        assert_eq!(w.find_in_files.status, before, "detached root must not receive completion publication");
    });
    drop(retained_dialog); cx.quit();
}

#[test] fn files_completed_editor_root_swap_retires_without_publication() { completed_root_swap(Route::Files, true); }
#[test] fn projects_completed_editor_root_swap_retires_without_publication() { completed_root_swap(Route::Projects, true); }
#[test] fn files_completed_dialog_root_swap_retires_without_publication() { completed_root_swap(Route::Files, false); }
#[test] fn projects_completed_dialog_root_swap_retires_without_publication() { completed_root_swap(Route::Projects, false); }

#[test]
fn files_dialog_entity_lease_releases_before_async_owner_read() {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Files);
    let (yes, receiver) = oneshot::channel();
    dialog.update(|window, cx| {
        let root = window.root::<crate::ui::find_dialog::FindDialog>().flatten().unwrap();
        root.update(cx, |_, cx| entity.update(cx, |w, cx| {
            w.replace_audit_confirmations.push_back(receiver);
            w.find_in_files_replace(&FindInFilesReplace, window, cx);
        }));
    }); dialog.run_until_parked();
    yes.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"done\r\n"); cx.quit();
}

#[test]
fn files_origin_removed_without_close_callback_refuses_yes_and_retires_owner() {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, mut editor, dialog) = fixture(&mut cx, dir.path(), Route::Files);
    let yes = request(&entity, &mut editor, Route::Files);
    editor.update(|window, _| window.remove_window()); dialog.run_until_parked();
    dialog.cx.update(|cx| assert!(!entity.read(cx).window_closed));
    let _ = yes.send(rfd::MessageDialogResult::Yes); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"hit\r\n");
    dialog.cx.update(|cx| assert!(!entity.read(cx).directory_operations.busy())); cx.quit();
}

#[test]
fn files_pending_replace_blocks_actual_search_without_status_change() {
    let dir = tempfile::tempdir().unwrap(); let _file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Files);
    let yes = request(&entity, &mut dialog, Route::Files);
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        let status = w.find_in_files.status.clone();
        w.run_find_in_files(window, cx); assert_eq!(w.find_in_files.status, status);
        w.run_find_in_projects(window, cx); assert_eq!(w.find_in_files.status, status);
    }));
    yes.send(rfd::MessageDialogResult::No).unwrap(); dialog.run_until_parked(); cx.quit();
}

#[test]
fn files_hiding_results_panel_preserves_confirmation() {
    let dir = tempfile::tempdir().unwrap(); let file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Files);
    let yes = request(&entity, &mut dialog, Route::Files);
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        w.find_in_files.open = true; w.close_find_in_files(&FindInFilesClose, window, cx);
        assert!(!w.find_in_files.open); assert!(w.directory_operations.busy());
    }));
    yes.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"done\r\n"); cx.quit();
}

#[test]
fn projects_overlapping_roots_do_not_replace_one_file_twice() {
    let dir = tempfile::tempdir().unwrap(); let child = dir.path().join("neutral-sub");
    std::fs::create_dir(&child).unwrap(); let file = neutral(&child);
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        folder_panel_roots: vec![dir.path().to_path_buf(), child.clone()],
        ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Projects);
    dialog.update(|_, cx| entity.update(cx, |w, _| {
        w.find_in_files.project_panel_2 = true;
        w.find_in_files.replacement = "hit!".into();
    }));
    let sender = request(&entity, &mut dialog, Route::Projects);
    sender.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    assert_eq!(std::fs::read(&file).unwrap(), b"hit!\r\n");
    cx.quit();
}

#[test] fn files_no_replaced_editor_root_retires_silently() { replaced_editor_root(Route::Files, true); }
#[test] fn projects_no_replaced_editor_root_retires_silently() { replaced_editor_root(Route::Projects, true); }
#[test] fn files_no_replaced_dialog_root_retires_silently() { replaced_dialog_root(Route::Files, true); }
#[test] fn projects_no_replaced_dialog_root_retires_silently() { replaced_dialog_root(Route::Projects, true); }

fn pending_directory_owner_preserves_panel(route: Route, all_opened: bool) {
    let dir = tempfile::tempdir().unwrap(); let _file = neutral(dir.path());
    let _settings = settings(dir.path()); let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), route);
    dialog.update(|_, cx| entity.update(cx, |w, _| {
        w.tabs[0].buffer.rope = ropey::Rope::from_str("hit\n"); w.find_state.query = "hit".into();
    }));
    let no = request(&entity, &mut dialog, route);
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        let status = w.find_in_files.status.clone();
        assert!(w.directory_operations.busy()); assert!(w.find_in_files.results.is_empty());
        if all_opened { w.find_all_in_all_opened(&EditFindAllOpened, window, cx); }
        else { w.find_all_current(&EditFindAllCurrent, window, cx); }
        assert!(w.find_in_files.searching, "open-document search must preserve directory busy state");
        assert_eq!(w.find_in_files.status, status);
        assert!(w.find_in_files.results.is_empty(), "pending directory owner keeps its panel");
    }));
    no.send(rfd::MessageDialogResult::No).unwrap(); dialog.run_until_parked();
    dialog.update(|window, cx| entity.update(cx, |w, cx| {
        if all_opened { w.find_all_in_all_opened(&EditFindAllOpened, window, cx); }
        else { w.find_all_current(&EditFindAllCurrent, window, cx); }
        assert!(!w.find_in_files.searching); assert_eq!(w.find_in_files.results.len(), 1);
    })); cx.quit();
}

#[test] fn files_pending_owner_preserves_panel_on_find_all_current() { pending_directory_owner_preserves_panel(Route::Files, false); }
#[test] fn projects_pending_owner_preserves_panel_on_find_all_current() { pending_directory_owner_preserves_panel(Route::Projects, false); }
#[test] fn files_pending_owner_preserves_panel_on_find_all_opened() { pending_directory_owner_preserves_panel(Route::Files, true); }
#[test] fn projects_pending_owner_preserves_panel_on_find_all_opened() { pending_directory_owner_preserves_panel(Route::Projects, true); }

fn project_overlap_case(case: &str) {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join(if case == "hidden" { ".neutral-hidden" } else if case == "pruned" { "target" } else { "neutral-sub" });
    std::fs::create_dir(&child).unwrap(); let file = neutral(&child);
    let mut roots = vec![dir.path().to_path_buf(), child.clone()];
    if case == "reverse" { roots.reverse(); }
    if case == "identical" { roots = vec![child.clone(), child.clone()]; }
    if case == "hardlink" {
        std::fs::hard_link(&file, child.join("neutral-alias.txt")).unwrap(); roots = vec![child.clone()];
    }
    if case == "case" {
        let alias = std::path::PathBuf::from(child.to_str().unwrap().to_uppercase());
        assert!(alias.is_dir(), "controlled filesystem must support this case alias");
        roots = vec![child.clone(), alias];
    }
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, folder_panel_roots: roots.clone(), ..Settings::default()
    });
    let mut cx = TestAppContext::single();
    let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Projects);
    dialog.update(|_, cx| entity.update(cx, |w, _| {
        w.find_in_files.project_panel_2 = roots.len() > 1;
        w.find_in_files.replacement = "hit!".into();
        if case == "norecurse" { w.find_in_files.in_subfolders = false; }
    }));
    request(&entity, &mut dialog, Route::Projects).send(rfd::MessageDialogResult::Yes).unwrap();
    dialog.run_until_parked(); assert_eq!(std::fs::read(&file).unwrap(), b"hit!\r\n"); cx.quit();
}

#[test] fn project_overlap_child_before_parent_replaces_once() { project_overlap_case("reverse"); }
#[test] fn project_overlap_identical_roots_replace_once() { project_overlap_case("identical"); }
#[test] fn project_overlap_hardlinks_replace_once() { project_overlap_case("hardlink"); }
#[cfg(windows)]
#[test] fn project_overlap_case_alias_replace_once() { project_overlap_case("case"); }
#[test] fn project_overlap_explicit_hidden_root_is_not_suppressed() { project_overlap_case("hidden"); }
#[test] fn project_overlap_explicit_pruned_root_is_not_suppressed() { project_overlap_case("pruned"); }
#[test] fn project_overlap_projects_recurse_with_files_toggle_off() { project_overlap_case("norecurse"); }
#[test]
#[ignore = "Matched release benchmark on synthetic 16 MiB project"]
fn project_overlap_release_benchmark() {
    let dir = tempfile::tempdir().unwrap(); let child = dir.path().join("neutral-sub");
    std::fs::create_dir(&child).unwrap();
    let mut bytes = vec![b'x'; 256 * 1024]; bytes.get_mut(..4).unwrap().copy_from_slice(b"hit\n");
    for i in 0..64 { std::fs::write(child.join(format!("neutral-{i}.txt")), &bytes).unwrap(); }
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false,
        folder_panel_roots: vec![dir.path().to_path_buf(), child], ..Settings::default()
    });
    let mut cx = TestAppContext::single(); let (entity, _, mut dialog) = fixture(&mut cx, dir.path(), Route::Projects);
    dialog.update(|_, cx| entity.update(cx, |w, _| {
        w.find_in_files.project_panel_2 = true; w.find_in_files.replacement = "hit!".into();
    }));
    let yes = request(&entity, &mut dialog, Route::Projects); let start = std::time::Instant::now();
    yes.send(rfd::MessageDialogResult::Yes).unwrap(); dialog.run_until_parked();
    dialog.cx.update(|cx| {
        let w=entity.read(cx); assert!(!w.directory_operations.busy()); assert!(w.find_in_files.error.is_none());
        eprintln!("BENCH project16MiB files64 roots2 elapsed_ms={} status={:?}", start.elapsed().as_millis(), w.find_in_files.status);
        assert_eq!(w.find_in_files.status.as_deref(), Some("Replaced 64 occurrences in 64 files"));
    });
    // Correctness checks are outside the matched worker timing.
    use std::io::Read;
    for i in 0..64 {
        let mut file = std::fs::File::open(dir.path().join("neutral-sub").join(format!("neutral-{i}.txt"))).unwrap();
        let mut prefix = [0; 5]; file.read_exact(&mut prefix).unwrap(); assert_eq!(&prefix, b"hit!\n");
    }
    cx.quit();
}
