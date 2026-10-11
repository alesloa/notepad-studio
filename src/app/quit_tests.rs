use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};
use crate::{actions::Quit, session::SessionManifest, tab::Tab, workspace::Workspace};
use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::mpsc::{channel, Receiver, Sender}};

thread_local! { static QUIT_SINK: RefCell<Option<Sender<()>>> = const { RefCell::new(None) }; }
pub(super) fn capture_quit() -> bool {
    QUIT_SINK.with(|slot| match slot.borrow().as_ref() {
        Some(sink) => { let _ = sink.send(()); true }
        None => false,
    })
}
struct QuitCapture { previous: Option<Sender<()>>, _local: PhantomData<Rc<()>> }
impl QuitCapture {
    fn new() -> (Self, Receiver<()>) {
        let (sink, calls) = channel();
        (Self { previous: QUIT_SINK.with(|slot| slot.replace(Some(sink))), _local: PhantomData }, calls)
    }
}
impl Drop for QuitCapture {
    fn drop(&mut self) { QUIT_SINK.with(|slot| slot.replace(self.previous.take())); }
}

fn add_workspace(
    cx: &mut TestAppContext, succeeds: bool, pending_csv: bool,
) -> (Entity<Workspace>, &mut VisualTestContext, Receiver<SessionManifest>) {
    let (sink, snapshots) = channel();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str("before café\n中😀");
        tab.buffer.dirty = true;
        if pending_csv {
            tab.csv_draft = Some(crate::session::CsvDraft { value: "pending 日本😀".into(),
                cursor: 8, anchor: 0, ..Default::default() });
        }
        w.tabs = vec![tab]; w.active = 0;
        w.window_closed = true; // The fixture never reads or writes user recovery files.
        w.session_snapshot_sink = Some(sink);
        w.session_final_save_result = Some(succeeds);
        w.goto_state.open = true;
        w.goto_state.text = "3".into();
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).goto_focus.clone(), cx);
    });
    visual.run_until_parked();
    (workspace, visual, snapshots)
}

#[test]
fn exit_refuses_failed_recovery_and_keeps_documents_drafts_and_current_field() {
    for pending_csv in [false, true] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let (_capture, quits) = QuitCapture::new();
        let mut cx = TestAppContext::single();
        cx.update(super::super::menus::install_global_actions);
        let (workspace, visual, snapshots) = add_workspace(&mut cx, false, pending_csv);
        visual.dispatch_action(Quit);
        visual.run_until_parked();
        assert!(quits.try_recv().is_err(), "failed recovery must not exit the platform");
        let snapshot = snapshots.try_recv().expect("attempt final full recovery first");
        assert_eq!(snapshot.tabs.len(), 1);
        assert_eq!(snapshot.tabs[0].csv_draft.is_some(), pending_csv);
        if pending_csv { assert_eq!(snapshot.tabs[0].csv_draft.as_ref().unwrap().value, "pending 日本😀"); }
        assert!(snapshots.try_recv().is_err());
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.buffer().rope.to_string(), "before café\n中😀");
            assert!(w.buffer().dirty);
            assert_eq!(w.tabs[0].csv_draft.is_some(), pending_csv);
            assert!(w.goto_focus.is_focused(window));
            assert_eq!(w.goto_state.text, "3");
            assert_eq!(w.status_message.as_deref(), Some("Exit cancelled: session recovery could not be saved: injected final recovery failure; save your documents or retry Exit"));
        });
        cx.quit();
    }
}

#[test]
fn exit_saves_every_workspace_before_quitting_and_can_retry_after_failure() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (first, _, first_saves) = add_workspace(&mut cx, true, true);
    let (second, visual, second_saves) = add_workspace(&mut cx, false, false);
    visual.dispatch_action(Quit);
    visual.run_until_parked();
    assert!(quits.try_recv().is_err());
    // Window iteration order need not match creation order. A failed preflight
    // keeps every window, even if a preceding window's backup already succeeded.
    let _ = first_saves.try_recv();
    assert!(second_saves.try_recv().is_ok());
    visual.update(|_, cx| {
        assert_eq!(first.read(cx).buffer().rope.to_string(), "before café\n中😀");
        assert_eq!(second.read(cx).buffer().rope.to_string(), "before café\n中😀");
        second.update(cx, |w, _| w.session_final_save_result = Some(true));
    });
    visual.dispatch_action(Quit);
    visual.run_until_parked();
    assert!(first_saves.try_recv().is_ok(), "retry saves the first workspace again");
    assert!(second_saves.try_recv().is_ok());
    assert!(quits.try_recv().is_ok(), "all recovery writes succeeded");
    assert!(quits.try_recv().is_err());
    cx.quit();
}

#[test]
fn exit_with_no_workspaces_quits_once() {
    let (_capture, quits) = QuitCapture::new();
    let cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    cx.update(|cx| cx.dispatch_action(&Quit));
    assert!(quits.try_recv().is_ok());
    assert!(quits.try_recv().is_err());
    cx.quit();
}

#[test]
fn native_window_close_refuses_recovery_failure_without_releasing_drafts() {
    for succeeds in [false, true] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let mut cx = TestAppContext::single();
        let (workspace, visual, snapshots) = add_workspace(&mut cx, succeeds, true);
        visual.update_window_entity(&workspace, |w, window, cx| {
            w.window_closed = false;
            assert_eq!(super::save_before_window_close(w, cx), succeeds);
            assert_eq!(w.window_closed, succeeds);
            assert_eq!(w.tabs[0].csv_draft.as_ref().unwrap().value, "pending 日本😀");
            assert_eq!(w.buffer().rope.to_string(), "before café\n中😀");
            assert!(w.goto_focus.is_focused(window));
            if !succeeds {
                assert_eq!(w.status_message.as_deref(), Some("Window close cancelled: session recovery could not be saved: injected final recovery failure; save your documents or retry closing"));
            }
        });
        assert_eq!(snapshots.try_recv().unwrap().tabs[0].csv_draft.as_ref().unwrap().value, "pending 日本😀");
        assert!(snapshots.try_recv().is_err());
        cx.quit();
    }
}

#[test]
fn failed_last_tab_recovery_keeps_a_live_strip_and_closed_document() {
    let settings = crate::settings::Settings { tab_exit_on_last_close: true, ..Default::default() };
    let _settings = crate::settings::override_for_test(settings);
    let mut cx = TestAppContext::single();
    let (workspace, visual, snapshots) = add_workspace(&mut cx, false, false);
    let id = visual.update_window_entity(&workspace, |w, window, cx| {
        w.window_closed = false;
        w.tabs[0].buffer.dirty = false;
        window.focus(&w.focus_handle, cx);
        w.tabs[0].session_id
    });
    visual.dispatch_action(crate::actions::FileCloseTab);
    visual.run_until_parked();
    let snapshot = snapshots.try_recv().expect("final recovery before removing the window");
    assert!(snapshot.tabs.is_empty());
    assert_eq!(snapshot.closed_tabs.len(), 1);
    visual.update_window_entity(&workspace, |w, _window, _cx| {
        assert!(!w.window_closed);
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.active, 0);
        assert_eq!(w.buffer().rope.len_bytes(), 0);
        assert_eq!(w.closed_tabs.len(), 1);
        assert_eq!(w.closed_tabs[0].session_id, id);
        assert_eq!(w.closed_tabs[0].buffer.rope.to_string(), "before café\n中😀");
        assert_eq!(w.status_message.as_deref(), Some("Window close cancelled: session recovery could not be saved: injected final recovery failure; save your documents or retry closing"));
    });
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    cx.quit();
}
