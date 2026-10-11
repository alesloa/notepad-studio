use gpui::{Entity, TestAppContext, VisualTestContext};
use crate::{actions::{FileCloseTab, FileRestoreClosedTab, FileSaveAs}, settings::Settings, tab::Tab, workspace::Workspace};

fn with_editor(source: &std::path::Path, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        default_directory_mode: crate::settings::DirectoryMode::FollowDocument, ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let buffer = super::read_buffer(source).expect("fixture buffer");
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut source_tab = Tab::with_buffer(buffer);
        source_tab.buffer.language = None;
        let mut sibling = Tab::empty();
        sibling.buffer.rope = ropey::Rope::from_str("unsaved sibling \u{4e2d}\u{1f600}");
        sibling.buffer.dirty = true;
        w.tabs = vec![source_tab, sibling];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.touch_mru();
        w
    });
    visual.update(|window, cx| {
        let close_entity = entity.clone();
        window.on_window_should_close(cx, move |_, cx| {
            close_entity.update(cx, crate::app::quit::save_before_window_close)
        });
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&entity, visual);
    app.quit();
}

#[test]
fn save_as_actual_closed_then_reopened_uuid_cannot_adopt_old_dialog() {
    let dir = crate::persist::TempDir::new("save-as-incarnation");
    let source = dir.join("source.txt");
    let target = dir.join("chosen-target.txt");
    let source_bytes = "source caf\u{e9}\r\n".as_bytes();
    std::fs::write(&source, source_bytes).expect("source");
    std::fs::write(&target, b"unrelated target must survive\r\n").expect("target");
    with_editor(&source, |entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(receiver);
        let original = visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0]; (tab.session_id, tab.instance.clone())
        });
        visual.dispatch_action(FileSaveAs);
        visual.run_until_parked();
        visual.dispatch_action(FileCloseTab);
        visual.run_until_parked();
        visual.dispatch_action(FileRestoreClosedTab);
        visual.run_until_parked();
        visual.simulate_input("late \u{4e2d}\u{1f600} ");
        let before = visual.update(|_, cx| {
            let w = entity.read(cx);
            let tab = &w.tabs[w.active];
            assert_eq!(tab.session_id, original.0);
            assert!(!std::sync::Arc::ptr_eq(&tab.instance, &original.1));
            (tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.undo_stack.len(), tab.buffer.dirty,
                w.tabs.iter().find(|tab| tab.session_id != original.0).expect("sibling").buffer.rope.to_string())
        });
        let recent_before = crate::recent::snapshot();
        assert!(sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("untouched target"), b"unrelated target must survive\r\n",
            "the old Save As dialog must not write another tab incarnation");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let tab = &w.tabs[w.active];
            assert_eq!(before, (tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.undo_stack.len(), tab.buffer.dirty,
                w.tabs.iter().find(|tab| tab.session_id != original.0).expect("sibling").buffer.rope.to_string()));
        });
        assert_eq!(crate::recent::snapshot(), recent_before);
    });
    assert_eq!(std::fs::read(&source).expect("untouched source"), source_bytes);
}

#[test]
fn save_as_actual_live_edited_tab_keeps_ownership_after_reordering_and_switching() {
    let dir = crate::persist::TempDir::new("save-as-live");
    let source = dir.join("source.txt");
    let target = dir.join("chosen-target.txt");
    std::fs::write(&source, "source caf\u{e9}\r\n".as_bytes()).expect("source");
    with_editor(&source, |entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(receiver);
        let source_id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        visual.dispatch_action(FileSaveAs);
        visual.run_until_parked();
        visual.simulate_input("latest \u{4e2d}\u{1f600} ");
        let history = visual.update(|_, cx| entity.read(cx).tabs[0].undo_stack.len());
        let active_id = visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs.swap(0, 1);
            w.active = 0;
            w.touch_mru();
            cx.notify();
            w.tabs[0].session_id
        }));
        assert!(sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[w.active].session_id, active_id);
            let saved = w.tabs.iter().find(|t| t.session_id == source_id).expect("original tab");
            assert_eq!(saved.buffer.file_path.as_deref(), Some(target.as_path()));
            assert!(!saved.buffer.dirty);
            assert_eq!(saved.undo_stack.len(), history);
            assert_eq!(w.tabs[w.active].buffer.rope.to_string(), "unsaved sibling \u{4e2d}\u{1f600}");
            assert!(w.tabs[w.active].buffer.dirty);
        });
        assert_eq!(std::fs::read(&target).expect("saved target"), "latest \u{4e2d}\u{1f600} source caf\u{e9}\r\n".as_bytes());
    });
    assert_eq!(std::fs::read(&source).expect("original source"), "source caf\u{e9}\r\n".as_bytes());
}
#[test]
fn save_as_actual_accepted_window_close_cannot_write_late_dialog_reply() {
    let dir = crate::persist::TempDir::new("save-as-window-close");
    let source = dir.join("source.txt");
    let target = dir.join("chosen-target.txt");
    let original = b"original source\r\n";
    std::fs::write(&source, original).expect("source");
    std::fs::write(&target, b"existing target\r\n").expect("target");
    with_editor(&source, |entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = super::save_as_dialog::Override::new(receiver);
        visual.dispatch_action(FileSaveAs);
        visual.run_until_parked();
        assert!(visual.simulate_close());
        let before = visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.window_closed);
            let tab = &w.tabs[0];
            (tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(),
                tab.buffer.dirty, tab.undo_stack.len())
        });
        let recent_before = crate::recent::snapshot();
        // Accepted native closure precedes platform removal; a queued dialog
        // completion must not write through the still-held editor entity.
        assert!(sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("target"), b"existing target\r\n");
        visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0];
            assert_eq!(before, (tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(),
                tab.buffer.dirty, tab.undo_stack.len()));
        });
        assert_eq!(crate::recent::snapshot(), recent_before);
        visual.update(|window, _| window.remove_window());
        visual.run_until_parked();
    });
    assert_eq!(std::fs::read(&source).expect("source"), original);
}
