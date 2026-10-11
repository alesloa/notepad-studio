// PowerEditor/src/NppIO.cpp:1795–1820, 1907–1990 — Save All includes
// dirty untitled documents and leaves clean files alone.
use std::path::Path;

use crate::{actions::*, settings::Settings, tab::Tab, workspace::Workspace};
use gpui::{Entity, TestAppContext, VisualTestContext};

fn untitled(text: &str) -> Tab {
    let mut tab = Tab::empty();
    tab.buffer.rope = ropey::Rope::from_str(text);
    tab.buffer.dirty = true;
    tab.buffer.language = None;
    tab.buffer.encoding = crate::text::encoding::Encoding::Utf16Le;
    tab.buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
    tab
}

fn named(path: &Path, replacement: Option<&str>) -> Tab {
    let mut tab = Tab::with_buffer(super::read_buffer(path).unwrap());
    tab.buffer.language = None;
    if let Some(text) = replacement {
        tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.dirty = true;
    }
    tab
}

fn with_editor(tabs: Vec<Tab>, active: usize, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        default_directory_mode: crate::settings::DirectoryMode::FollowDocument,
        save_all_confirm: false, ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs = tabs;
        w.active = active;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.touch_mru();
        w
    });
    visual.update(|window, cx| {
        let close_entity = entity.clone();
        window.on_window_should_close(cx, move |_, cx| close_entity.update(cx, crate::app::quit::save_before_window_close));
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&entity, visual);
    app.quit();
}

#[test]
fn save_all_actual_saves_dirty_untitled_with_native_encoding() {
    let dir = crate::persist::TempDir::new("save-all-untitled");
    let target = dir.join("chosen.txt");
    let source = "café 中\n";
    with_editor(vec![untitled(source), Tab::empty()], 0, |entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(receiver);
        visual.dispatch_action(FileSaveAll);
        visual.run_until_parked();
        assert!(sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        let mut bytes = vec![0xff, 0xfe];
        for word in "café 中\r\n".encode_utf16() { bytes.extend_from_slice(&word.to_le_bytes()); }
        assert_eq!(std::fs::read(&target).ok(), Some(bytes), "Save All must Save As dirty untitled text");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[0].buffer.file_path.as_deref(), Some(target.as_path()));
            assert!(!w.tabs[0].buffer.dirty);
            assert!(w.tabs[1].buffer.file_path.is_none());
            assert!(!w.tabs[1].buffer.dirty);
            assert_eq!(w.active, 0);
        });
    });
}

#[test]
fn save_all_actual_named_control_preserves_clean_disk_bytes_and_mtime() {
    let dir = crate::persist::TempDir::new("save-all-named");
    let clean = dir.join("clean.txt");
    let dirty = dir.join("dirty.txt");
    std::fs::write(&clean, b"original clean\r\n").unwrap();
    std::fs::write(&dirty, b"old dirty\r\n").unwrap();
    let tabs = vec![named(&clean, None), named(&dirty, Some("latest café\n")), Tab::empty()];
    std::fs::write(&clean, b"external changes stay\r\n").unwrap();
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    std::fs::OpenOptions::new().write(true).open(&clean).unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(time)).unwrap();
    let before = std::fs::metadata(&clean).unwrap().modified().unwrap();
    with_editor(tabs, 0, |entity, visual| {
        visual.dispatch_action(FileSaveAll);
        visual.run_until_parked();
        assert_eq!(std::fs::read(&clean).unwrap(), b"external changes stay\r\n");
        assert_eq!(std::fs::metadata(&clean).unwrap().modified().unwrap(), before);
        assert_eq!(std::fs::read(&dirty).unwrap(), "latest café\r\n".as_bytes());
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.active, 0);
            assert!(!w.tabs[1].buffer.dirty);
        });
    });
}

#[test]
fn save_all_actual_cancel_one_untitled_continues_named_and_next_dialog() {
    let dir = crate::persist::TempDir::new("save-all-sequential");
    let named_path = dir.join("named.txt");
    let chosen = dir.join("second.txt");
    std::fs::write(&named_path, b"old\r\n").unwrap();
    with_editor(vec![untitled("cancel me\n"), named(&named_path, Some("named latest\n")), untitled("second 中\n"), Tab::empty()], 3, |entity, visual| {
        let (first, reply) = futures::channel::oneshot::channel();
        let _first = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll);
        visual.run_until_parked();
        assert_eq!(std::fs::read(&named_path).unwrap(), b"old\r\n", "tab order waits for the first picker");
        let (second, reply) = futures::channel::oneshot::channel();
        let _second = super::save_as_dialog::Override::new(reply);
        first.send(None).unwrap();
        visual.run_until_parked();
        assert_eq!(std::fs::read(&named_path).unwrap(), b"named latest\r\n");
        assert!(!chosen.exists());
        second.send(Some(chosen.clone())).unwrap();
        visual.run_until_parked();
        assert_eq!(super::read_buffer(&chosen).unwrap().rope.to_string(), "second 中\n");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs[0].buffer.dirty); assert!(w.tabs[0].buffer.file_path.is_none());
            assert!(!w.tabs[1].buffer.dirty); assert!(!w.tabs[2].buffer.dirty);
            assert_eq!(w.active, 3);
            assert!(w.status_message.as_ref().unwrap().contains("1 cancelled"));
        });
    });
}

#[test]
fn save_all_actual_live_edits_reordering_switching_and_new_tabs_keep_original_batch() {
    let dir = crate::persist::TempDir::new("save-all-live");
    let first_path = dir.join("first.txt");
    let second_path = dir.join("second.txt");
    with_editor(vec![untitled("first café\n"), untitled("second 中\n"), Tab::empty()], 0, |entity, visual| {
        let (first, reply) = futures::channel::oneshot::channel();
        let _first = super::save_as_dialog::Override::new(reply);
        let first_id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        visual.dispatch_action(FileSaveAll);
        visual.run_until_parked();
        visual.simulate_input("latest 中😀 ");
        let (active_id, history) = visual.update(|_, cx| entity.update(cx, |w, cx| {
            let history = w.tabs[0].undo_stack.len();
            w.tabs.swap(0, 2); w.active = 0;
            w.tabs.push(untitled("created later"));
            w.touch_mru(); cx.notify();
            (w.tabs[0].session_id, history)
        }));
        let (second, reply) = futures::channel::oneshot::channel();
        let _second = super::save_as_dialog::Override::new(reply);
        first.send(Some(first_path.clone())).unwrap();
        visual.run_until_parked();
        assert_eq!(super::read_buffer(&first_path).unwrap().rope.to_string(), "latest 中😀 first café\n");
        second.send(Some(second_path.clone())).unwrap();
        visual.run_until_parked();
        assert_eq!(super::read_buffer(&second_path).unwrap().rope.to_string(), "second 中\n");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[w.active].session_id, active_id);
            let first = w.tabs.iter().find(|tab| tab.session_id == first_id).unwrap();
            assert!(!first.buffer.dirty); assert_eq!(first.undo_stack.len(), history);
            assert!(w.tabs[3].buffer.dirty); assert!(w.tabs[3].buffer.file_path.is_none());
        });
    });
}

#[test]
fn save_all_actual_closed_restored_instance_cannot_adopt_old_picker() {
    let dir = crate::persist::TempDir::new("save-all-instance");
    let target = dir.join("untouched.txt");
    std::fs::write(&target, b"unrelated target\r\n").unwrap();
    with_editor(vec![untitled("source café\n"), Tab::empty()], 0, |entity, visual| {
        let (sender, reply) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(reply);
        let (id, instance) = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; (tab.session_id, tab.instance.clone()) });
        visual.dispatch_action(FileSaveAll);
        visual.run_until_parked();
        // Close a clean fixture without invoking the independent dirty-close dialog.
        visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.dirty = false));
        visual.dispatch_action(FileCloseTab); visual.run_until_parked();
        visual.dispatch_action(FileRestoreClosedTab); visual.run_until_parked();
        visual.simulate_input("restored 中😀 ");
        let before = visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[w.active];
            assert_eq!(tab.session_id, id); assert!(!std::sync::Arc::ptr_eq(&tab.instance, &instance));
            (tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.buffer.dirty, tab.undo_stack.len())
        });
        let recents = crate::recent::snapshot();
        sender.send(Some(target.clone())).unwrap(); visual.run_until_parked();
        assert_eq!(std::fs::read(&target).unwrap(), b"unrelated target\r\n");
        assert_eq!(crate::recent::snapshot(), recents);
        visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[w.active];
            assert_eq!(before, (tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.buffer.dirty, tab.undo_stack.len()));
        });
    });
}

#[test]
fn save_all_actual_accepted_window_close_rejects_late_picker() {
    let dir = crate::persist::TempDir::new("save-all-window-close");
    let target = dir.join("untouched.txt");
    std::fs::write(&target, b"unrelated target\r\n").unwrap();
    with_editor(vec![untitled("unsaved café\n"), Tab::empty()], 0, |entity, visual| {
        let (sender, reply) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert!(visual.simulate_close());
        assert!(visual.update(|_, cx| entity.read(cx).window_closed));
        let recents = crate::recent::snapshot();
        sender.send(Some(target.clone())).unwrap(); visual.run_until_parked();
        assert_eq!(std::fs::read(&target).unwrap(), b"unrelated target\r\n");
        assert_eq!(crate::recent::snapshot(), recents);
        visual.update(|window, _| window.remove_window()); visual.run_until_parked();
    });
}

#[test]
fn save_all_actual_other_save_as_repoints_original_before_old_picker_reply() {
    let dir = crate::persist::TempDir::new("save-all-repoint");
    let fresh = dir.join("fresh.txt"); let stale = dir.join("stale.txt");
    with_editor(vec![untitled("original\n"), Tab::empty()], 0, |entity, visual| {
        let (old, reply) = futures::channel::oneshot::channel();
        let _old = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        let (new, reply) = futures::channel::oneshot::channel();
        let _new = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAs); visual.run_until_parked();
        new.send(Some(fresh.clone())).unwrap(); visual.run_until_parked();
        visual.simulate_input("later ");
        let recents = crate::recent::snapshot();
        old.send(Some(stale.clone())).unwrap(); visual.run_until_parked();
        assert!(!stale.exists()); assert_eq!(crate::recent::snapshot(), recents);
        assert_eq!(super::read_buffer(&fresh).unwrap().rope.to_string(), "original\n");
        visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0];
            assert_eq!(tab.buffer.file_path.as_deref(), Some(fresh.as_path()));
            assert_eq!(tab.buffer.rope.to_string(), "later original\n"); assert!(tab.buffer.dirty);
        });
    });
}

#[test]
fn save_all_actual_write_failure_and_duplicate_target_continue_without_switching() {
    let dir = crate::persist::TempDir::new("save-all-failures");
    let open = dir.join("open.txt"); let named_path = dir.join("named.txt");
    std::fs::write(&open, b"open original\n").unwrap(); std::fs::write(&named_path, b"old\n").unwrap();
    with_editor(vec![untitled("first"), untitled("second"), named(&open, None), named(&named_path, Some("latest\n"))], 2, |entity, visual| {
        let (first, reply) = futures::channel::oneshot::channel();
        let _first = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        let (second, reply) = futures::channel::oneshot::channel();
        let _second = super::save_as_dialog::Override::new(reply);
        first.send(Some(dir.join("missing/subdir/file.txt"))).unwrap(); visual.run_until_parked();
        second.send(Some(open.clone())).unwrap(); visual.run_until_parked();
        assert_eq!(std::fs::read(&open).unwrap(), b"open original\n");
        assert_eq!(std::fs::read(&named_path).unwrap(), b"latest\n");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.active, 2);
            assert!(w.tabs[0].buffer.file_path.is_none()); assert!(w.tabs[0].buffer.dirty);
            assert!(w.tabs[1].buffer.file_path.is_none()); assert!(w.tabs[1].buffer.dirty);
            assert!(w.status_message.as_ref().unwrap().contains("2 failed"));
        });
    });
}

#[test]
fn save_all_actual_read_only_changed_during_picker_blocks_write_but_continues() {
    let dir = crate::persist::TempDir::new("save-all-readonly");
    let target = dir.join("target.txt"); let later = dir.join("later.txt");
    std::fs::write(&later, b"old\n").unwrap();
    with_editor(vec![untitled("draft"), named(&later, Some("latest\n"))], 0, |entity, visual| {
        let (sender, reply) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        visual.dispatch_action(EditToggleReadOnly); visual.run_until_parked();
        sender.send(Some(target.clone())).unwrap(); visual.run_until_parked();
        assert!(!target.exists()); assert_eq!(std::fs::read(&later).unwrap(), b"latest\n");
        visual.update(|_, cx| { let w = entity.read(cx); assert!(w.tabs[0].buffer.dirty); assert!(w.tabs[0].buffer.read_only); });
    });
}

#[test]
fn save_all_actual_confirmation_bypasses_sole_active_dirty_untitled() {
    let dir = crate::persist::TempDir::new("save-all-sole"); let target = dir.join("target.txt");
    with_editor(vec![untitled("only dirty"), Tab::empty()], 0, |_, visual| {
        let _prefs = crate::settings::override_for_test(Settings { save_all_confirm: true, ..crate::settings::snapshot() });
        let (confirm, reply) = futures::channel::oneshot::channel();
        let confirm_dialog = super::save_all::Override::new(reply);
        let (sender, reply) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        sender.send(Some(target.clone())).unwrap(); visual.run_until_parked();
        assert!(target.is_file());
        assert_eq!(confirm_dialog.count(), None);
        assert!(confirm.send(super::save_all::Decision::No).is_ok(), "the confirmation receiver was never consumed");
        assert!(crate::settings::snapshot().save_all_confirm);
    });
}

#[test]
fn save_all_actual_confirmation_no_counts_untitled_and_writes_nothing() {
    let dir = crate::persist::TempDir::new("save-all-no"); let path = dir.join("named.txt");
    std::fs::write(&path, b"old\n").unwrap();
    with_editor(vec![untitled("first"), named(&path, Some("latest\n")), Tab::empty()], 2, |entity, visual| {
        let _prefs = crate::settings::override_for_test(Settings { save_all_confirm: true, ..crate::settings::snapshot() });
        let (sender, reply) = futures::channel::oneshot::channel();
        let confirm_dialog = super::save_all::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert_eq!(confirm_dialog.count(), Some(2));
        sender.send(super::save_all::Decision::No).unwrap(); visual.run_until_parked();
        assert_eq!(std::fs::read(&path).unwrap(), b"old\n");
        visual.update(|_, cx| { let w = entity.read(cx); assert!(w.tabs[0].buffer.dirty); assert!(w.tabs[1].buffer.dirty); assert_eq!(w.active, 2); });
    });
}

#[test]
fn save_all_actual_always_yes_saves_and_disables_later_confirmation_locally() {
    let dir = crate::persist::TempDir::new("save-all-always-yes"); let path = dir.join("named.txt");
    std::fs::write(&path, b"old\n").unwrap();
    with_editor(vec![named(&path, Some("latest\n")), Tab::empty()], 1, |entity, visual| {
        let writes = crate::settings::persistence_fixture::observe();
        let _prefs = crate::settings::override_for_test(Settings { save_all_confirm: true, ..crate::settings::snapshot() });
        let (sender, reply) = futures::channel::oneshot::channel();
        let confirm_dialog = super::save_all::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert_eq!(confirm_dialog.count(), Some(1), "one dirty inactive document still confirms");
        sender.send(super::save_all::Decision::AlwaysYes).unwrap(); visual.run_until_parked();
        assert_eq!(std::fs::read(&path).unwrap(), b"latest\n");
        assert!(!crate::settings::snapshot().save_all_confirm); assert_eq!(writes.count(), 0);
        visual.update(|_, cx| entity.update(cx, |w, _| { w.tabs[0].buffer.rope = ropey::Rope::from_str("again\n"); w.tabs[0].buffer.dirty = true; }));
        let (unused, reply) = futures::channel::oneshot::channel();
        let _unused = super::save_all::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert_eq!(std::fs::read(&path).unwrap(), b"again\n"); assert_eq!(writes.count(), 0);
        assert!(unused.send(super::save_all::Decision::No).is_ok());
    });
}

#[test]
fn save_all_actual_late_always_yes_after_accepted_close_cannot_change_settings() {
    let dir = crate::persist::TempDir::new("save-all-confirm-close"); let path = dir.join("named.txt");
    std::fs::write(&path, b"old\n").unwrap();
    with_editor(vec![named(&path, Some("latest\n")), Tab::empty()], 1, |entity, visual| {
        let writes = crate::settings::persistence_fixture::observe();
        let _prefs = crate::settings::override_for_test(Settings { save_all_confirm: true, ..crate::settings::snapshot() });
        let (sender, reply) = futures::channel::oneshot::channel();
        let _confirm = super::save_all::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert!(visual.simulate_close()); assert!(visual.update(|_, cx| entity.read(cx).window_closed));
        sender.send(super::save_all::Decision::AlwaysYes).unwrap(); visual.run_until_parked();
        assert_eq!(std::fs::read(&path).unwrap(), b"old\n");
        assert!(crate::settings::snapshot().save_all_confirm); assert_eq!(writes.count(), 0);
        visual.update(|window, _| window.remove_window()); visual.run_until_parked();
    });
}

fn csv_draft(entity: &Entity<Workspace>, visual: &mut VisualTestContext, index: usize, read_only: bool) {
    visual.update(|window, cx| entity.update(cx, |w, cx| {
        w.active = index;
        w.set_csv_view_mode(true, cx);
        crate::ui::csv_doc::actions::ensure_model(w, window, cx);
        crate::ui::csv_doc::actions::open_cell_editor(w, 0, 0, window, cx);
        gpui::EntityInputHandler::replace_text_in_range(w, Some(0..4), "draft 中😀", window, cx);
        w.tabs[index].buffer.read_only = read_only;
    }));
}

#[test]
fn save_all_actual_commits_previously_clean_csv_before_dirty_snapshot() {
    let dir = crate::persist::TempDir::new("save-all-csv"); let path = dir.join("source.csv");
    let text = "label,value\r\ncafé,1\r\n中,2\r\n";
    std::fs::write(&path, crate::text::encoding::encode_checked(text, crate::text::encoding::Encoding::Utf8Bom).unwrap()).unwrap();
    with_editor(vec![named(&path, None), Tab::empty()], 0, |entity, visual| {
        csv_draft(entity, visual, 0, false);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        let bytes = std::fs::read(&path).unwrap(); assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(super::read_buffer(&path).unwrap().rope.to_string().contains("draft 中😀"));
        visual.update(|_, cx| { let w = entity.read(cx); assert!(!w.tabs[0].buffer.dirty); assert!(w.csv_models[&w.tabs[0].session_id].cell_input.is_none()); });
    });
}

#[test]
fn save_all_actual_csv_refusal_before_save_keeps_draft_and_every_file() {
    let dir = crate::persist::TempDir::new("save-all-csv-refusal");
    let csv = dir.join("source.csv"); let later = dir.join("later.txt");
    let original = b"label,value\r\ncafe,1\r\n";
    std::fs::write(&csv, original).unwrap(); std::fs::write(&later, b"old\n").unwrap();
    with_editor(vec![named(&csv, None), named(&later, Some("latest\n"))], 0, |entity, visual| {
        csv_draft(entity, visual, 0, true);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert_eq!(std::fs::read(&csv).unwrap(), original); assert_eq!(std::fs::read(&later).unwrap(), b"old\n");
        visual.update(|_, cx| {
            let w = entity.read(cx); let input = w.csv_models[&w.tabs[0].session_id].cell_input.as_ref().unwrap();
            assert_eq!(input.buffer, "draft 中😀"); assert_eq!(w.active, 0);
            assert!(w.status_message.as_ref().unwrap().contains("Cell edit not committed"));
        });
    });
}

#[test]
fn save_all_actual_csv_refusal_added_during_picker_blocks_deferred_and_later_writes() {
    let dir = crate::persist::TempDir::new("save-all-csv-late");
    let csv = dir.join("source.csv"); let later = dir.join("later.txt"); let chosen = dir.join("chosen.txt");
    let original = b"label,value\r\ncafe,1\r\n";
    std::fs::write(&csv, original).unwrap(); std::fs::write(&later, b"old\n").unwrap();
    with_editor(vec![untitled("first"), named(&csv, None), named(&later, Some("latest\n"))], 0, |entity, visual| {
        let (sender, reply) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        csv_draft(entity, visual, 1, true);
        sender.send(Some(chosen.clone())).unwrap(); visual.run_until_parked();
        assert!(!chosen.exists()); assert_eq!(std::fs::read(&csv).unwrap(), original); assert_eq!(std::fs::read(&later).unwrap(), b"old\n");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.active, 1); assert!(w.tabs[0].buffer.dirty); assert!(w.tabs[2].buffer.dirty);
            assert_eq!(w.csv_models[&w.tabs[1].session_id].cell_input.as_ref().unwrap().buffer, "draft 中😀");
            assert!(w.status_message.as_ref().unwrap().contains("Cell edit not committed"));
        });
    });
}

#[test]
fn save_all_actual_read_only_and_loading_untitled_skip_picker_and_continue() {
    let dir = crate::persist::TempDir::new("save-all-loading"); let path = dir.join("later.txt");
    std::fs::write(&path, b"old\n").unwrap();
    let mut readonly = untitled("readonly"); readonly.buffer.read_only = true;
    let mut loading = untitled("partial"); loading.buffer.loading = Some(crate::buffer::LoadProgress {
        bytes_read: 1, total_bytes: 2, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    with_editor(vec![readonly, loading, named(&path, Some("latest\n")), Tab::empty()], 3, |entity, visual| {
        let (unused, reply) = futures::channel::oneshot::channel(); let _unused = super::save_as_dialog::Override::new(reply);
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        assert!(unused.send(None).is_ok()); assert_eq!(std::fs::read(&path).unwrap(), b"latest\n");
        visual.update(|_, cx| { let w = entity.read(cx); assert_eq!(w.active, 3); assert!(w.tabs[0].buffer.dirty); assert!(w.tabs[1].buffer.dirty); assert!(w.status_message.as_ref().unwrap().contains("2 failed")); });
    });
}

#[test]
fn save_all_native_confirmation_close_is_distinct_from_always_yes_button() {
    use super::save_all::{Decision, decision};
    use rfd::MessageDialogResult as Result;
    assert_eq!(decision(Result::Yes), Decision::Yes);
    assert_eq!(decision(Result::Custom("Yes".into())), Decision::Yes);
    assert_eq!(decision(Result::Custom("Always Yes".into())), Decision::AlwaysYes);
    for closed in [Result::Cancel, Result::No, Result::Custom("No".into()), Result::Custom("unexpected".into())] {
        assert_eq!(decision(closed), Decision::No, "native close or an unknown response must not save or disable confirmation");
    }
}
