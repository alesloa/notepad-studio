use super::*;
use crate::{buffer::Buffer, settings::Settings, tab::Tab};
use gpui::{Entity, TestAppContext, VisualTestContext};

#[test]
fn actual_untitled_rename_rejects_overlong_utf16_label_without_truncation() {
    let accepted = format!("{}😀", "a".repeat(61));
    let rejected = format!("{}😀", "a".repeat(62));
    with_editor(Buffer::untitled(), |entity, visual| {
        visual.simulate_input("unsaved");
        let original_text = visual.update(|_, cx| entity.read(cx).tabs[0].buffer.rope.to_string());
        for name in [&accepted, &rejected] {
            visual.dispatch_action(FileRename); visual.run_until_parked();
            visual.simulate_input(name); visual.simulate_keystrokes("enter"); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.new_label.as_ref().map(|label| label.as_ref()), Some(accepted.as_str()));
                assert!(w.tabs[0].buffer.dirty); assert_eq!(w.tabs[0].buffer.rope.to_string(), original_text);
                if name == &rejected { assert!(w.status_message.as_deref().expect("status").contains("63")); }
            });
        }
    });
}

#[test]
fn actual_save_picker_suggests_current_untitled_label() {
    for label in ["new 1", "named café中😀.txt"] {
            let mut buffer = Buffer::untitled(); buffer.new_label = Some(label.to_owned().into());
            with_editor(buffer, |entity, visual| {
                visual.simulate_input("unsaved");
                let requests = crate::io::SaveDialogRequestCaptureForTest::new();
                let (reply, selected) = futures::channel::oneshot::channel();
                let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
                visual.dispatch_action(FileSave); visual.run_until_parked();
                assert_eq!(requests.requests(), vec![("Save As", label.to_owned())]);
                assert!(reply.send(None).is_ok()); visual.run_until_parked();
                visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; assert!(tab.buffer.file_path.is_none()); assert!(tab.buffer.dirty); });
            });
    }
}

thread_local! {
    static LAUNCHES: std::cell::RefCell<Option<(Vec<Launch>, Option<String>)>> = const { std::cell::RefCell::new(None) };
    static CROSS_DEVICE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    #[cfg(target_os = "windows")]
    static REGISTRY: std::cell::RefCell<Option<Result<OsString, String>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(target_os = "windows")]
pub(super) fn registry_answer() -> Option<Result<OsString, String>> { REGISTRY.with(|slot| slot.borrow().clone()) }
#[cfg(target_os = "windows")]
struct Registry(Option<Result<OsString, String>>);
#[cfg(target_os = "windows")]
impl Registry { fn new(answer: Result<OsString, String>) -> Self { Self(REGISTRY.with(|slot| slot.replace(Some(answer)))) } }
#[cfg(target_os = "windows")]
impl Drop for Registry { fn drop(&mut self) { REGISTRY.with(|slot| slot.replace(self.0.take())); } }
pub(super) fn force_cross_device() -> bool { CROSS_DEVICE.with(std::cell::Cell::get) }
struct CrossDevice(bool);
impl CrossDevice { fn new() -> Self { Self(CROSS_DEVICE.with(|slot| slot.replace(true))) } }
impl Drop for CrossDevice { fn drop(&mut self) { CROSS_DEVICE.with(|slot| slot.set(self.0)); } }
pub(super) fn capture_launch(request: &Launch) -> Option<Result<(), String>> {
    LAUNCHES.with(|slot| slot.borrow_mut().as_mut().map(|(requests, error)| {
        requests.push(request.clone()); error.clone().map_or(Ok(()), Err)
    }))
}
struct Launches;
impl Launches {
    fn new(error: Option<&str>) -> Self {
        LAUNCHES.with(|slot| slot.replace(Some((Vec::new(), error.map(str::to_owned))))); Self
    }
    fn requests(&self) -> Vec<Launch> { LAUNCHES.with(|slot| slot.borrow().as_ref().expect("fixture").0.clone()) }
}
impl Drop for Launches { fn drop(&mut self) { LAUNCHES.with(|slot| slot.replace(None)); } }

fn with_editor(buffer: Buffer, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    #[cfg(target_os = "windows")]
    let _registry = registry_answer().is_none().then(|| Registry::new(Ok("C:/controlled PowerShell/powershell.exe".into())));
    let _settings = crate::settings::override_for_test(Settings { toolbar_hidden: true,
        folder_panel_visible: false, tab_exit_on_last_close: false, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::with_buffer(buffer); tab.buffer.language = None;
        w.tabs = vec![tab, Tab::empty()]; w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true); w.touch_mru(); w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked(); test(&entity, visual); app.quit();
}

#[test]
fn actual_containing_folder_routes_use_argument_arrays_and_preserve_live_document() {
    let dir = crate::persist::TempDir::new("file-launch");
    let nested = dir.join("folder & café中 %PATH% (name)"); std::fs::create_dir(&nested).expect("fixture directory");
    let path = nested.join("source.txt"); std::fs::write(&path, b"disk\r\n").expect("fixture");
    let launches = Launches::new(None);
    with_editor(crate::io::read_buffer(&path).expect("read"), |entity, visual| {
        visual.simulate_input("unsaved ");
        let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
            (tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.buffer.dirty, tab.undo_stack.len()) });
        visual.dispatch_action(FileOpenContainingFolderCmd);
        visual.dispatch_action(FileOpenContainingFolderPowerShell);
        visual.dispatch_action(FileContainingFolderAsWorkspace); visual.run_until_parked();
        let requests = launches.requests(); assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], launch_request(ShellKind::CommandPrompt, nested.clone()).expect("cmd launch"));
        assert_eq!(requests[1], launch_request(ShellKind::PowerShell, nested.clone()).expect("PowerShell launch"));
        #[cfg(target_os = "windows")]
        { assert!(requests[0].arguments.is_empty()); assert!(requests[1].arguments.is_empty()); }
        assert!(crate::settings::snapshot().folder_panel_visible);
        assert!(crate::settings::snapshot().folder_panel_roots.contains(&nested));
        visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
            assert_eq!(before, (tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.buffer.dirty, tab.undo_stack.len())); });
        assert_eq!(std::fs::read(&path).expect("source"), b"disk\r\n");
    });
}

#[test]
fn actual_containing_folder_routes_report_untitled_invalid_missing_and_launch_error() {
    let dir = crate::persist::TempDir::new("file-launch-error"); let launches = Launches::new(Some("synthetic launch error"));
    with_editor(Buffer::untitled(), |entity, visual| {
        for path in [None, Some(PathBuf::from("bad\0path")), Some(dir.join("missing/source.txt")), Some(dir.join("source.txt"))] {
            visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.file_path = path.clone()));
            visual.dispatch_action(FileOpenContainingFolderCmd); visual.run_until_parked();
            visual.update(|_, cx| assert!(entity.read(cx).status_message.is_some()));
        }
        assert_eq!(launches.requests().len(), 1);
        visual.update(|_, cx| assert_eq!(entity.read(cx).status_message.as_deref(), Some("Open terminal failed: synthetic launch error")));
    });
}

fn begin_rename(visual: &mut VisualTestContext) -> futures::channel::oneshot::Sender<Option<PathBuf>> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let _dialog = crate::io::SaveAsDialogOverrideForTest::new(receiver);
    visual.dispatch_action(FileRename); visual.run_until_parked(); sender
}

#[test]
fn actual_destination_rename_moves_disk_bytes_and_keeps_dirty_history_identity_after_edit_and_reorder() {
    let dir = crate::persist::TempDir::new("file-rename"); let source = dir.join("source.txt");
    let destination_folder = dir.join("other café中😀"); std::fs::create_dir(&destination_folder).expect("directory");
    let destination = destination_folder.join("renamed.txt");
    let bytes = crate::text::encoding::encode_checked("disk café中😀\r\n", crate::text::encoding::Encoding::Utf16Be).expect("encoding");
    std::fs::write(&source, &bytes).expect("source"); std::fs::write(&destination, b"replace this chosen file").expect("destination");
    with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
        let sender = begin_rename(visual); visual.simulate_input("live ");
        let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
            (tab.session_id, tab.instance.clone(), tab.buffer.savepoint.clone(), tab.undo_stack.len(), tab.buffer.rope.to_string()) });
        visual.update(|_, cx| entity.update(cx, |w, cx| { w.tabs.swap(0, 1); w.active = 0; cx.notify(); }));
        assert!(sender.send(Some(destination.clone())).is_ok()); visual.run_until_parked();
        assert!(!source.exists()); assert_eq!(std::fs::read(&destination).expect("renamed bytes"), bytes);
        visual.update(|_, cx| { let w = entity.read(cx); let tab = &w.tabs[1];
            assert_eq!(tab.session_id, before.0); assert!(Arc::ptr_eq(&tab.instance, &before.1));
            assert!(tab.buffer.savepoint == before.2); assert_eq!(tab.undo_stack.len(), before.3);
            assert_eq!(tab.buffer.rope.to_string(), before.4); assert!(tab.buffer.dirty);
            assert_eq!(tab.buffer.file_path.as_ref(), Some(&destination)); assert_eq!(w.active, 0);
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| w.set_active_tab(1, cx)));
        for _ in 0..before.3 { visual.dispatch_action(EditUndo); }
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[1].buffer.rope.to_string(), "disk café中😀\n"));
    });
}

#[test]
fn actual_cross_device_rename_uses_staged_background_copy() {
    let dir = crate::persist::TempDir::new("rename-cross-device"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
    std::fs::write(&source, "café中😀\r\n").expect("source");
    with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
        let _cross_device = CrossDevice::new(); let sender = begin_rename(visual);
        assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        assert!(!source.exists()); assert_eq!(std::fs::read_to_string(&target).expect("target"), "café中😀\r\n");
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.file_path.as_ref(), Some(&target)));
        assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 1, "staging files removed");
    });
}

#[test]
fn actual_new_native_or_popup_rename_retires_an_earlier_staged_move() {
    for popup in [false, true] {
        let dir = crate::persist::TempDir::new("rename-stage-superseded");
        let source = dir.join("source.txt"); let earlier = dir.join("earlier.txt"); let latest = dir.join("latest.txt");
        std::fs::write(&source, b"source bytes\r\n").expect("source"); std::fs::write(&earlier, b"earlier destination").expect("earlier");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            visual.simulate_input("unsaved ");
            let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                (tab.session_id, tab.instance.clone(), tab.buffer.rope.to_string(), tab.undo_stack.len(), tab.buffer.savepoint.clone()) });
            let _cross_device = CrossDevice::new(); let (release, held) = futures::channel::oneshot::channel();
            let _gate = crate::workspace::file_move::test_support::Override::new(held);
            let first = begin_rename(visual); assert!(first.send(Some(earlier.clone())).is_ok()); visual.run_until_parked();
            let next = if popup { visual.update(|_, cx| entity.update(cx, |w, cx| w.open_tab_rename(0, cx))); None }
                else { Some(begin_rename(visual)) };
            visual.update(|_, cx| assert!(entity.read(cx).file_move_chip().is_none()));
            assert!(release.send(()).is_ok()); visual.run_until_parked();
            assert_eq!(std::fs::read(&source).expect("source"), b"source bytes\r\n");
            assert_eq!(std::fs::read(&earlier).expect("earlier"), b"earlier destination"); assert!(!latest.exists());
            assert_eq!(std::fs::read_dir(dir.path()).expect("discarded stage").count(), 2);
            if let Some(next) = next { assert!(next.send(Some(latest.clone())).is_ok()); visual.run_until_parked();
                assert!(!source.exists()); assert_eq!(std::fs::read(&latest).expect("latest"), b"source bytes\r\n"); }
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                assert_eq!(tab.session_id, before.0); assert!(Arc::ptr_eq(&tab.instance, &before.1));
                assert_eq!(tab.buffer.rope.to_string(), before.2); assert_eq!(tab.undo_stack.len(), before.3);
                assert!(tab.buffer.savepoint == before.4); assert!(tab.buffer.dirty);
            });
        });
    }
}

#[test]
fn actual_cross_device_rename_rejects_restore_path_save_and_closed_window_after_staging() {
    for change in ["restore", "path", "save", "window"] {
        let dir = crate::persist::TempDir::new("rename-cross-stale"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        std::fs::write(&source, b"original\r\n").expect("source");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let _cross_device = CrossDevice::new(); let (release, held) = futures::channel::oneshot::channel();
            let _gate = crate::workspace::file_move::test_support::Override::new(held);
            let sender = begin_rename(visual); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            assert!(source.exists()); assert!(!target.exists());
            match change {
                "restore" => { visual.dispatch_action(FileCloseTab); visual.run_until_parked(); visual.dispatch_action(FileRestoreClosedTab); },
                "save" => { visual.simulate_input("saved "); visual.dispatch_action(FileSave); },
                _ => visual.update(|_, cx| entity.update(cx, |w, _| {
                    if change == "path" { w.tabs[0].buffer.file_path = Some(dir.join("changed.txt")); }
                    else { w.window_closed = true; }
                })),
            }
            visual.run_until_parked(); assert!(release.send(()).is_ok()); visual.run_until_parked();
            assert!(source.exists()); assert!(!target.exists());
            assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 1, "cancelled staging file removed");
        });
    }
}

#[test]
fn actual_cross_device_move_progress_and_cancel_chip_or_escape_keep_both_files() {
    for click in [false, true] {
        let dir = crate::persist::TempDir::new("rename-cross-cancel"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        std::fs::write(&source, b"original source\r\n").expect("source"); std::fs::write(&target, b"original destination").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let _cross_device = CrossDevice::new(); let (release, held) = futures::channel::oneshot::channel();
            let _gate = crate::workspace::file_move::test_support::Override::new(held);
            let sender = begin_rename(visual); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            visual.update(|_, cx| { let chip = entity.read(cx).file_move_chip().expect("progress"); assert!(chip.progress_text.contains("Moving file")); });
            if click { let bounds = visual.debug_bounds("loading-cancel").expect("cancel chip"); visual.simulate_click(bounds.center(), gpui::Modifiers::none()); }
            else { visual.simulate_keystrokes("escape"); }
            assert!(release.send(()).is_ok()); visual.run_until_parked();
            assert_eq!(std::fs::read(&source).expect("source"), b"original source\r\n"); assert_eq!(std::fs::read(&target).expect("target"), b"original destination");
            assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 2);
            visual.update(|_, cx| assert!(entity.read(cx).file_move_chip().is_none()));
        });
    }
}

#[test]
fn actual_destination_rename_rejects_stale_restored_path_closed_and_error_replies() {
    for mode in ["restore", "path", "closed", "error", "cancel"] {
        let dir = crate::persist::TempDir::new("rename-stale"); let source = dir.join("source.txt");
        let destination = dir.join("renamed.txt"); std::fs::write(&source, b"original\r\n").expect("source");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let sender = begin_rename(visual);
            match mode {
                "restore" | "closed" => {
                    visual.dispatch_action(FileCloseTab); visual.run_until_parked();
                    if mode == "restore" { visual.dispatch_action(FileRestoreClosedTab); visual.run_until_parked(); }
                }
                "path" => visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.file_path = Some(dir.join("changed.txt")))),
                _ => (),
            }
            let chosen = match mode { "error" => Some(dir.join("missing/renamed.txt")), "cancel" => None, _ => Some(destination.clone()) };
            assert!(sender.send(chosen).is_ok()); visual.run_until_parked();
            assert_eq!(std::fs::read(&source).expect("original"), b"original\r\n"); assert!(!destination.exists());
            if mode == "error" || mode == "cancel" { visual.update(|_, cx| {
                let tab = &entity.read(cx).tabs[0]; assert_eq!(tab.buffer.file_path.as_ref(), Some(&source)); assert!(!tab.buffer.dirty);
            }); }
        });
    }
}

#[test]
fn actual_destination_rename_new_request_retires_old_native_picker_reply() {
    let dir = crate::persist::TempDir::new("rename-superseded"); let source = dir.join("source.txt");
    let old_target = dir.join("old.txt"); let latest_target = dir.join("latest.txt"); std::fs::write(&source, b"original").expect("source");
    with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
        let first = begin_rename(visual); let second = begin_rename(visual);
        assert!(first.send(Some(old_target.clone())).is_ok()); visual.run_until_parked();
        assert!(source.exists()); assert!(!old_target.exists());
        assert!(second.send(Some(latest_target.clone())).is_ok()); visual.run_until_parked();
        assert!(!source.exists()); assert_eq!(std::fs::read(&latest_target).expect("latest"), b"original");
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.file_path.as_ref(), Some(&latest_target)));
    });
}

#[test]
fn actual_untitled_and_missing_path_rename_changes_only_label_and_keeps_recovery_and_history() {
    for missing in [false, true] {
        let dir = crate::persist::TempDir::new("label-rename"); let mut buffer = Buffer::untitled();
        if missing { buffer.file_path = Some(dir.join("gone.txt")); }
        with_editor(buffer, |entity, visual| {
            visual.simulate_input("unsaved café中😀");
            let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; (tab.session_id, tab.instance.clone(), tab.undo_stack.len(), tab.buffer.rope.to_string()) });
            visual.dispatch_action(FileRename); visual.run_until_parked();
            visual.simulate_input("named café中😀.txt"); visual.simulate_keystrokes("enter"); visual.run_until_parked();
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                assert_eq!(tab.buffer.display_short_name().as_ref(), "named café中😀.txt");
                assert!(tab.buffer.file_path.is_none()); assert!(tab.buffer.dirty); assert_eq!(tab.session_id, before.0);
                assert!(Arc::ptr_eq(&tab.instance, &before.1)); assert_eq!(tab.undo_stack.len(), before.2); assert_eq!(tab.buffer.rope.to_string(), before.3);
            });
            assert!(!dir.join("named café中😀.txt").exists());
        });
    }
}

#[test]
fn inline_rename_prompt_rejects_restored_incarnation_even_with_same_uuid_and_path() {
    let dir = crate::persist::TempDir::new("inline-rename-stale"); let source = dir.join("source.txt");
    std::fs::write(&source, b"original").expect("source");
    with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
        let old_prompt = visual.update(|_, cx| entity.update(cx, |w, cx| { w.open_tab_rename(0, cx); w.tab_rename.clone().expect("prompt") }));
        visual.dispatch_action(FileCloseTab); visual.run_until_parked();
        visual.dispatch_action(FileRestoreClosedTab); visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let mut prompt = old_prompt; prompt.text = "new.txt".into(); w.tab_rename = Some(prompt); w.commit_tab_rename(cx);
        }));
        assert_eq!(std::fs::read(&source).expect("original"), b"original"); assert!(!dir.join("new.txt").exists());
        visual.update(|_, cx| assert!(entity.read(cx).status_message.as_deref().is_some_and(|status| status.contains("no longer open"))));
    });
}

#[test]
fn actual_file_trash_route_uses_guarded_confirmation_and_fixture_backend() {
    for confirmed in [false, true] {
        let dir = crate::persist::TempDir::new("file-trash-route"); let source = dir.join("source.txt");
        std::fs::write(&source, b"original\r\n").expect("source"); let control = crate::workspace::trash::test_support::install(dir.path());
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            visual.dispatch_action(FileMoveToTrash); visual.run_until_parked();
            assert!(control.calls().is_empty()); assert_eq!(control.prompts().len(), 1);
            control.answer(confirmed); visual.run_until_parked();
            assert_eq!(source.exists(), !confirmed); assert_eq!(control.calls().len(), usize::from(confirmed));
            visual.update(|_, cx| assert_eq!(entity.read(cx).tabs.len(), if confirmed { 1 } else { 2 }));
        });
    }
}

#[cfg(target_os = "windows")]
#[test]
fn actual_powershell_route_uses_registered_unicode_path_and_reports_registry_errors_without_launch() {
    let dir = crate::persist::TempDir::new("shell-registry"); let source = dir.join("source.txt"); std::fs::write(&source, b"source").expect("source");
    for answer in [Ok(OsString::from("C:/custom café中😀 & shell/powershell.exe")), Err("controlled registry read failure".into()), Ok(OsString::new()), Ok(OsString::from("bad\0path"))] {
        let _registry = Registry::new(answer.clone()); let launches = Launches::new(None);
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            visual.dispatch_action(FileOpenContainingFolderPowerShell); visual.run_until_parked();
            if let Ok(path) = &answer { if !path.is_empty() && !path.to_string_lossy().contains('\0') {
                let requests = launches.requests(); assert_eq!(requests.len(), 1); assert_eq!(&requests[0].executable, path); assert!(requests[0].arguments.is_empty()); return;
            } }
            assert!(launches.requests().is_empty());
            visual.update(|_, cx| assert!(entity.read(cx).status_message.as_deref().is_some_and(|status| status.starts_with("Open terminal failed:"))));
        });
    }
    let _registry = Registry::new(Ok("%SystemRoot%/controlled café中/powershell.exe".into())); let launches = Launches::new(None);
    with_editor(crate::io::read_buffer(&source).expect("read"), |_, visual| {
        visual.dispatch_action(FileOpenContainingFolderPowerShell); visual.run_until_parked();
        let mut expected = std::env::var_os("SystemRoot").expect("Windows directory"); expected.push("/controlled café中/powershell.exe");
        assert_eq!(launches.requests()[0].executable, expected);
    });
}

#[test]
fn actual_rename_refuses_an_open_destination_hardlink_before_any_file_effect() {
    let dir = crate::persist::TempDir::new("rename-open-alias"); let source = dir.join("source.txt"); let destination = dir.join("destination.txt"); let alias = dir.join("other-name.txt");
    std::fs::write(&source, b"source").expect("source"); std::fs::write(&destination, b"open destination").expect("destination"); std::fs::hard_link(&destination, &alias).expect("hardlink");
    with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[1] = Tab::with_buffer(crate::io::read_buffer(&alias).expect("open alias"))));
        let sender = begin_rename(visual); assert!(sender.send(Some(destination.clone())).is_ok()); visual.run_until_parked();
        assert_eq!(std::fs::read(&source).expect("source"), b"source"); assert_eq!(std::fs::read(&destination).expect("destination"), b"open destination");
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.file_path.as_ref(), Some(&source)));
    });
}

// NppNotification.cpp:58-60; NppIO.cpp:2186; Notepad_plus.cpp:6572.
#[test]
fn explicit_untitled_label_survives_first_line_title_preference() {
    for name in ["named café中😀.txt", "new 37"] {
        with_editor(Buffer::empty(), |entity, visual| {
            let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
            let _preference = crate::settings::override_for_test(settings);
            visual.simulate_input("first café中😀\nsecond line"); visual.run_until_parked();
            let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                (tab.buffer.rope.to_string(), crate::buffer::serialize_undo_stack(&tab.undo_stack),
                    tab.buffer.savepoint.clone(), tab.session_id, tab.instance.clone()) });
            visual.dispatch_action(FileRename); visual.run_until_parked();
            visual.simulate_input(name); visual.simulate_keystrokes("enter"); visual.run_until_parked();
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                assert_eq!(tab.buffer.new_label.as_ref().map(|s| s.as_ref()), Some(name));
                assert_eq!(tab.buffer.rope.to_string(), before.0);
                assert_eq!(crate::buffer::serialize_undo_stack(&tab.undo_stack), before.1);
                assert!(tab.buffer.savepoint == before.2);
                assert_eq!(tab.session_id, before.3); assert!(Arc::ptr_eq(&tab.instance, &before.4));
                assert!(tab.buffer.dirty && tab.buffer.file_path.is_none());
                assert_eq!(tab.buffer.display_short_name().as_ref(), name, "an explicit rename must stay visible");
                assert_eq!(tab.buffer.title_bar_label(), name);
            });
            for enabled in [false, true] {
                let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = enabled;
                let _toggle = crate::settings::override_for_test(settings);
                visual.update(|_, cx| {
                    assert_eq!(entity.read(cx).tabs[0].buffer.display_short_name().as_ref(), name);
                    assert_eq!(entity.read(cx).tabs[0].buffer.title_bar_label(), name);
                });
            }
            visual.simulate_keystrokes("ctrl-home");
            visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("changed ".into())));
            visual.dispatch_action(crate::actions::EditPaste); visual.run_until_parked();
            visual.update(|_, cx| {
                let buffer = &entity.read(cx).tabs[0].buffer;
                assert_eq!(buffer.rope.to_string(), "changed first café中😀\nsecond line");
                assert_eq!(buffer.display_short_name().as_ref(), name);
            });
            visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
            visual.update(|_, cx| {
                assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), before.0);
                assert_eq!(entity.read(cx).tabs[0].buffer.title_bar_label(), name);
            });
            visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
            visual.update(|_, cx| {
                assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), "changed first café中😀\nsecond line");
                assert_eq!(entity.read(cx).tabs[0].buffer.title_bar_label(), name);
            });
        });
    }
}

#[test]
fn explicit_untitled_label_survives_actual_recovery_write_and_restore() {
    let dir = crate::persist::TempDir::new("explicit-label-recovery");
    let path = dir.join("session.toml"); let name = "recovered café中😀.txt";
    with_editor(Buffer::empty(), |entity, visual| {
        visual.simulate_input("first café中😀\nsecond line"); visual.run_until_parked();
        visual.dispatch_action(FileRename); visual.run_until_parked();
        visual.simulate_input(name); visual.simulate_keystrokes("enter"); visual.run_until_parked();
        let manifest = visual.update(|_, cx| entity.read(cx).session_snapshot(crate::session::BakPolicy::Full));
        crate::session::write_csv_fixture(manifest, &path, dir.path()).expect("actual isolated recovery Writer");
        let persisted = std::fs::read_to_string(&path).expect("published manifest");
        let manifest: crate::session::SessionManifest = toml::from_str(&persisted).expect("current persisted schema");
        let entry = manifest.tabs.first().expect("persisted dirty tab");
        assert_eq!(entry.new_label.as_deref(), Some(name));
        let crate::session::RestoreOutcome::Restored(tab) = crate::session::restore_csv_fixture(entry, dir.path())
            else { panic!("actual recovery must restore the draft"); };
        assert_eq!(tab.buffer.rope.to_string(), "first café中😀\nsecond line");
        assert!(tab.buffer.dirty && tab.buffer.file_path.is_none());
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _preference = crate::settings::override_for_test(settings);
        assert_eq!(tab.buffer.display_short_name().as_ref(), name, "persisted explicit label must override first-line titles");
        assert_eq!(tab.buffer.title_bar_label(), name);
    });
}

#[test]
fn automatic_untitled_label_still_follows_first_line_title_preference() {
    with_editor(Buffer::empty(), |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _preference = crate::settings::override_for_test(settings);
        visual.simulate_input("first café中😀\nsecond line"); visual.run_until_parked();
        visual.update(|_, cx| {
            let buffer = &entity.read(cx).tabs[0].buffer;
            assert_eq!(buffer.display_short_name().as_ref(), "first café中😀");
            assert_eq!(buffer.title_bar_label(), "first café中😀");
        });
    });
}

#[test]
fn explicit_untitled_label_is_metadata_only_in_read_only_source() {
    with_editor(Buffer::empty(), |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _preference = crate::settings::override_for_test(settings);
        visual.simulate_input("first café中😀\nsecond line"); visual.run_until_parked();
        let before = visual.update(|_, cx| entity.update(cx, |workspace, _| {
            let tab = &mut workspace.tabs[0]; tab.buffer.read_only = true;
            (tab.buffer.rope.to_string(), crate::buffer::serialize_undo_stack(&tab.undo_stack), tab.buffer.savepoint.clone())
        }));
        visual.dispatch_action(FileRename); visual.run_until_parked();
        visual.simulate_input("read-only café中😀"); visual.simulate_keystrokes("enter"); visual.run_until_parked();
        visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0];
            assert_eq!(tab.buffer.display_short_name().as_ref(), "read-only café中😀");
            assert!(tab.buffer.read_only && tab.buffer.dirty && tab.buffer.file_path.is_none());
            assert_eq!(tab.buffer.rope.to_string(), before.0);
            assert_eq!(crate::buffer::serialize_undo_stack(&tab.undo_stack), before.1);
            assert!(tab.buffer.savepoint == before.2);
        });
    });
}
