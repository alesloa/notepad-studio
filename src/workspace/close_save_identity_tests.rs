use gpui::{Entity, TestAppContext, VisualTestContext};
use crate::{actions::{FileCloseAllTabs, FileCloseTab, FileRestoreClosedTab}, settings::Settings, tab::{CloseDecision, Tab}, workspace::Workspace};

fn with_untitled(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        default_directory_mode: crate::settings::DirectoryMode::FollowDocument, ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut source = Tab::empty();
        source.buffer.rope = ropey::Rope::from_str("untitled caf\u{e9}\n");
        source.buffer.language = None;
        source.buffer.dirty = true;
        source.buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
        source.buffer.encoding = crate::text::encoding::Encoding::Utf8Bom;
        let mut sibling = Tab::empty();
        sibling.buffer.rope = ropey::Rope::from_str("unsaved sibling \u{4e2d}\u{1f600}");
        sibling.buffer.dirty = true;
        w.tabs = vec![source, sibling];
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

fn begin_close_save(visual: &mut VisualTestContext) {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let _prompt = super::close_async_tests::PromptOverride::new(receiver);
    visual.dispatch_action(FileCloseTab);
    visual.run_until_parked();
    assert!(sender.send(CloseDecision::Save).is_ok());
    visual.run_until_parked();
}

#[test]
fn close_save_actual_two_close_restore_cannot_write_restored_incarnation() {
    let dir = crate::persist::TempDir::new("close-save-incarnation");
    let target = dir.join("existing-target.txt");
    std::fs::write(&target, b"unrelated target\r\n").expect("target");
    with_untitled(|entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = crate::io::SaveAsDialogOverrideForTest::new(receiver);
        let original = visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0]; (tab.session_id, tab.instance.clone())
        });
        begin_close_save(visual);
        let (second_sender, second_receiver) = futures::channel::oneshot::channel();
        let _second_prompt = super::close_async_tests::PromptOverride::new(second_receiver);
        visual.dispatch_action(FileCloseTab);
        visual.run_until_parked();
        assert!(second_sender.send(CloseDecision::DontSave).is_ok());
        visual.run_until_parked();
        visual.update(|_, cx| assert!(!entity.read(cx).tabs.iter().any(|tab| tab.session_id == original.0),
            "the second registered Close route must remove the first requester"));
        visual.dispatch_action(FileRestoreClosedTab);
        visual.run_until_parked();
        visual.simulate_input("late \u{4e2d}\u{1f600} ");
        let before = visual.update(|_, cx| {
            let w = entity.read(cx);
            let tab = &w.tabs[w.active];
            assert_eq!(tab.session_id, original.0);
            assert!(!std::sync::Arc::ptr_eq(&tab.instance, &original.1));
            (w.active, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.buffer.dirty,
                tab.undo_stack.len(), tab.redo_stack.len(), w.closed_tabs.len(),
                w.tabs.iter().find(|tab| tab.session_id != original.0).expect("sibling").buffer.rope.to_string())
        });
        let recent_before = crate::recent::snapshot();
        assert!(sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("untouched target"), b"unrelated target\r\n",
            "a pending save-for-close must not write a restored tab's edits");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let tab = &w.tabs[w.active];
            assert_eq!(tab.session_id, original.0, "the old close must not remove the restored tab");
            assert_eq!(before, (w.active, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(), tab.buffer.dirty,
                tab.undo_stack.len(), tab.redo_stack.len(), w.closed_tabs.len(),
                w.tabs.iter().find(|tab| tab.session_id != original.0).expect("sibling").buffer.rope.to_string()));
        });
        assert_eq!(crate::recent::snapshot(), recent_before);
    });
}

#[test]
fn close_save_actual_original_live_edits_reorder_and_switch_still_save_and_close() {
    let dir = crate::persist::TempDir::new("close-save-live");
    let target = dir.join("chosen-target.txt");
    with_untitled(|entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = crate::io::SaveAsDialogOverrideForTest::new(receiver);
        let original_id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        begin_close_save(visual);
        visual.simulate_input("latest \u{4e2d}\u{1f600} ");
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
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.tabs[w.active].session_id, active_id);
            assert_eq!(w.tabs[w.active].buffer.rope.to_string(), "unsaved sibling \u{4e2d}\u{1f600}");
            assert!(w.tabs[w.active].buffer.dirty);
            assert!(!w.tabs.iter().any(|tab| tab.session_id == original_id));
            let closed = w.closed_tabs.last().expect("saved closed tab");
            assert_eq!(closed.session_id, original_id);
            assert!(!closed.buffer.dirty);
            assert_eq!(closed.buffer.file_path.as_deref(), Some(target.as_path()));
        });
        assert_eq!(std::fs::read(&target).expect("saved target"),
            "\u{feff}latest \u{4e2d}\u{1f600} untitled caf\u{e9}\r\n".as_bytes());
    });
}

#[test]
fn close_save_actual_cancel_keeps_original_dirty_tab_open() {
    with_untitled(|entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = crate::io::SaveAsDialogOverrideForTest::new(receiver);
        let original_id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        begin_close_save(visual);
        assert!(sender.send(None).is_ok());
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[w.active].session_id, original_id);
            assert!(w.tabs[w.active].buffer.dirty);
            assert!(w.tabs[w.active].buffer.file_path.is_none());
            assert!(w.closed_tabs.is_empty());
            assert!(w.status_message.as_deref().is_some_and(|message| message.contains("Save As cancelled")));
        });
        assert!(crate::recent::snapshot().is_empty());
    });
}

#[test]
fn close_save_actual_old_dirty_prompt_cannot_adopt_unchanged_restored_tab() {
    let dir = crate::persist::TempDir::new("close-save-old-confirmation");
    let target = dir.join("existing-target.txt");
    std::fs::write(&target, b"unrelated target\r\n").expect("target");
    with_untitled(|entity, visual| {
        let (path_sender, path_receiver) = futures::channel::oneshot::channel();
        let _dialog = crate::io::SaveAsDialogOverrideForTest::new(path_receiver);
        let (first_sender, first_receiver) = futures::channel::oneshot::channel();
        let _first_prompt = super::close_async_tests::PromptOverride::new(first_receiver);
        let original = visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0]; (tab.session_id, tab.instance.clone(), tab.buffer.raw_revision.clone())
        });
        visual.dispatch_action(FileCloseTab);
        visual.run_until_parked();
        let (second_sender, second_receiver) = futures::channel::oneshot::channel();
        let _second_prompt = super::close_async_tests::PromptOverride::new(second_receiver);
        visual.dispatch_action(FileCloseTab);
        visual.run_until_parked();
        assert!(second_sender.send(CloseDecision::DontSave).is_ok());
        visual.run_until_parked();
        visual.dispatch_action(FileRestoreClosedTab);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[entity.read(cx).active];
            assert_eq!(tab.session_id, original.0);
            assert!(!std::sync::Arc::ptr_eq(&tab.instance, &original.1));
            assert_eq!(tab.buffer.raw_revision, original.2, "restoring retains the captured content revision");
        });
        assert!(first_sender.send(CloseDecision::Save).is_ok());
        visual.run_until_parked();
        assert!(path_sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("target"), b"unrelated target\r\n",
            "the original dirty-close prompt belongs to the removed incarnation");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let tab = &w.tabs[w.active];
            assert_eq!(tab.session_id, original.0);
            assert!(tab.buffer.dirty);
            assert!(tab.buffer.file_path.is_none());
            assert!(w.closed_tabs.is_empty());
            assert_eq!(tab.buffer.rope.to_string(), "untitled caf\u{e9}\n");
        });
        assert!(crate::recent::snapshot().is_empty());
    });
}

#[test]
fn close_save_actual_accepted_window_close_discards_held_path_reply() {
    let dir = crate::persist::TempDir::new("close-save-window");
    let target = dir.join("existing-target.txt");
    std::fs::write(&target, b"unrelated target\r\n").expect("target");
    with_untitled(|entity, visual| {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _dialog = crate::io::SaveAsDialogOverrideForTest::new(receiver);
        begin_close_save(visual);
        assert!(visual.simulate_close());
        let before = visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.window_closed);
            let tab = &w.tabs[0];
            (w.tabs.len(), tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(),
                tab.buffer.dirty, tab.undo_stack.len(), w.closed_tabs.len())
        });
        assert!(sender.send(Some(target.clone())).is_ok());
        visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("target"), b"unrelated target\r\n");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let tab = &w.tabs[0];
            assert_eq!(before, (w.tabs.len(), tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(),
                tab.buffer.dirty, tab.undo_stack.len(), w.closed_tabs.len()));
        });
        assert!(crate::recent::snapshot().is_empty());
        visual.update(|window, _| window.remove_window());
        visual.run_until_parked();
    });
}

fn check_accepted_close_dont_save(batch: bool) {
    with_untitled(|entity, visual| {
        let (sink, recovered) = std::sync::mpsc::channel();
        visual.update(|_, cx| entity.update(cx, |w, _| w.session_snapshot_sink = Some(sink)));
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _prompt = super::close_async_tests::PromptOverride::new(receiver);
        if batch { visual.dispatch_action(FileCloseAllTabs); }
        else { visual.dispatch_action(FileCloseTab); }
        visual.run_until_parked();
        assert!(visual.simulate_close());
        let before = visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.window_closed);
            (w.active, w.closed_tabs.len(), w.tabs.iter().map(|tab| {
                (tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(),
                    tab.buffer.dirty, tab.undo_stack.len(), tab.redo_stack.len())
            }).collect::<Vec<_>>())
        });
        let recovery = recovered.try_iter().last().expect("accepted-close recovery");
        assert_eq!(recovery.tabs.len(), 2, "accepted close captures both original documents");
        // The test sink intercepts before the production closed-window
        // check. Detach it so late callbacks use that real write guard.
        visual.update(|_, cx| entity.update(cx, |w, _| w.session_snapshot_sink = None));
        assert!(sender.send(CloseDecision::DontSave).is_ok());
        visual.run_until_parked();
        assert!(recovered.try_iter().next().is_none(), "a late prompt cannot rewrite accepted-close recovery");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(before, (w.active, w.closed_tabs.len(), w.tabs.iter().map(|tab| {
                (tab.session_id, tab.buffer.rope.to_string(), tab.buffer.file_path.clone(),
                    tab.buffer.dirty, tab.undo_stack.len(), tab.redo_stack.len())
            }).collect::<Vec<_>>()), "late Don't Save must leave retained documents intact: batch={batch}");
        });
        visual.update(|window, _| window.remove_window());
        visual.run_until_parked();
    });
}

#[test]
fn close_save_actual_accepted_window_close_ignores_held_dont_save_prompt() {
    check_accepted_close_dont_save(false);
}

#[test]
fn close_save_actual_accepted_window_close_ignores_held_dont_save_prompt_batch() {
    check_accepted_close_dont_save(true);
}
