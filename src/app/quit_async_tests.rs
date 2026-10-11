use gpui::{Entity, Global, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};
use crate::{actions::Quit, session::SessionManifest, tab::Tab, workspace::Workspace};
use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::mpsc::{channel, Receiver, Sender}};
use futures::channel::oneshot;

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

pub(super) struct WriteGate {
    pub(super) finished: Option<oneshot::Receiver<std::io::Result<()>>>,
    pub(super) captures: Sender<Vec<SessionManifest>>,
}
impl Global for WriteGate {}

struct FinalBarrierFixture { failure: Option<String>, calls: Option<Sender<Vec<bool>>> }
impl Global for FinalBarrierFixture {}

pub(super) fn final_barrier(cx: &mut gpui::App) -> Option<std::io::Result<()>> {
    let fixture = cx.try_global::<FinalBarrierFixture>()?;
    let failure = fixture.failure.clone(); let calls = fixture.calls.clone();
    if let Some(calls) = calls {
        let live_flags = cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>())
            .map(|handle| handle.update(cx, |w, _, _| w.window_closed).unwrap_or(true)).collect();
        let _ = calls.send(live_flags);
    }
    Some(match failure { Some(message) => Err(std::io::Error::other(message)), None => Ok(()) })
}

pub(super) fn take_gate(cx: &mut gpui::App, snapshots: Vec<SessionManifest>) -> Option<oneshot::Receiver<std::io::Result<()>>> {
    cx.try_global::<WriteGate>()?;
    let gate = cx.global_mut::<WriteGate>();
    let receiver = gate.finished.take()?;
    let _ = gate.captures.send(snapshots);
    Some(receiver)
}

fn gate(cx: &mut TestAppContext) -> (oneshot::Sender<std::io::Result<()>>, Receiver<Vec<SessionManifest>>) {
    cx.update(gate_in_app)
}

fn gate_in_app(cx: &mut gpui::App) -> (oneshot::Sender<std::io::Result<()>>, Receiver<Vec<SessionManifest>>) {
    let (release, finished) = oneshot::channel();
    let (captures, jobs) = channel();
    cx.set_global(WriteGate { finished: Some(finished), captures });
    cx.set_global(FinalBarrierFixture { failure: None, calls: None });
    (release, jobs)
}

fn add_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.add_window_view(|window, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str("before café\n中😀");
        tab.buffer.dirty = true;
        w.tabs = vec![tab]; w.active = 0;
        w.window_closed = true;
        w.session_final_save_result = Some(true);
        window.activate_window(); window.focus(&w.focus_handle, cx);
        w
    })
}

#[test]
fn exit_waits_for_one_immutable_worker_without_blocking_editor_input() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (release, jobs) = gate(&mut cx);
    let (workspace, visual) = add_workspace(&mut cx);
    visual.dispatch_action(Quit); visual.run_until_parked();
    assert!(quits.try_recv().is_err(), "Exit must await the background recovery write");
    let captures = jobs.try_recv().expect("one immutable recovery job");
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].tabs.len(), 1);
    visual.simulate_input("日本"); visual.run_until_parked();
    visual.update_window_entity(&workspace, |w, _, _| assert_eq!(w.buffer().rope.to_string(), "日本before café\n中😀"));
    let dir = crate::persist::TempDir::new("quit-immutable-source");
    crate::session::write_csv_fixture(captures[0].clone(), &dir.join("session.toml"), dir.path()).unwrap();
    let backup = dir.join(&format!("{}.bak", captures[0].tabs[0].id));
    assert_eq!(std::fs::read_to_string(backup).unwrap(), "before café\n中😀", "worker retains the exact earlier rope");
    visual.dispatch_action(Quit); visual.run_until_parked();
    assert!(jobs.try_recv().is_err(), "a second Exit must not duplicate the in-flight write");
    release.send(Ok(())).expect("release writer"); visual.run_until_parked();
    assert!(quits.try_recv().is_err(), "input after capture cancels Exit");
    visual.update_window_entity(&workspace, |w, _, _| {
        assert!(w.status_message.as_deref().is_some_and(|status| status.contains("changed")));
        assert_eq!(w.buffer().rope.to_string(), "日本before café\n中😀");
    });
    cx.quit();
}

#[test]
fn exit_unchanged_worker_quits_once_after_completion() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (release, jobs) = gate(&mut cx);
    let (_workspace, visual) = add_workspace(&mut cx);
    visual.dispatch_action(Quit); visual.run_until_parked();
    assert!(quits.try_recv().is_err());
    assert_eq!(jobs.try_recv().expect("capture").len(), 1);
    release.send(Ok(())).expect("release"); visual.run_until_parked();
    assert!(quits.try_recv().is_ok()); assert!(quits.try_recv().is_err());
    cx.quit();
}

#[test]
fn exit_failure_preserves_details_and_retry_can_complete() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (release, jobs) = gate(&mut cx);
    let (workspace, visual) = add_workspace(&mut cx);
    visual.dispatch_action(Quit); visual.run_until_parked();
    assert_eq!(jobs.try_recv().unwrap().len(), 1);
    release.send(Err(std::io::Error::other("fixture disk full"))).unwrap(); visual.run_until_parked();
    assert!(quits.try_recv().is_err());
    visual.update_window_entity(&workspace, |w, window, _| {
        assert_eq!(w.buffer().rope.to_string(), "before café\n中😀");
        assert!(w.focus_handle.is_focused(window));
        assert_eq!(w.status_message.as_deref(), Some("Exit cancelled: session recovery could not be saved: fixture disk full; save your documents or retry Exit"));
    });
    let (release, jobs) = visual.update(|_, cx| gate_in_app(cx));
    visual.dispatch_action(Quit); visual.run_until_parked();
    assert_eq!(jobs.try_recv().unwrap().len(), 1);
    release.send(Ok(())).unwrap(); visual.run_until_parked();
    assert!(quits.try_recv().is_ok()); assert!(quits.try_recv().is_err());
    cx.quit();
}

#[test]
fn exit_refuses_streaming_load_without_starting_a_recovery_job() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (_release, jobs) = gate(&mut cx);
    let (workspace, visual) = add_workspace(&mut cx);
    visual.update_window_entity(&workspace, |w, _, _| {
        w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 1024, total_bytes: 16_777_216,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) });
    });
    visual.dispatch_action(Quit); visual.run_until_parked();
    assert!(quits.try_recv().is_err()); assert!(jobs.try_recv().is_err());
    visual.update_window_entity(&workspace, |w, _, _| {
        assert!(w.status_message.as_deref().is_some_and(|message| message.contains("still loading")));
        assert_eq!(w.buffer().loading.as_ref().unwrap().bytes_read, 1024);
    });
    cx.quit();
}

#[test]
fn exit_checks_raw_revision_metadata_and_closed_identities_without_a_debounce_capture() {
    for change in 0..8 {
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let (_capture, quits) = QuitCapture::new();
        let mut cx = TestAppContext::single();
        cx.update(super::super::menus::install_global_actions);
        let (release, jobs) = gate(&mut cx);
        let (workspace, visual) = add_workspace(&mut cx);
        visual.dispatch_action(Quit); visual.run_until_parked(); jobs.try_recv().unwrap();
        visual.update_window_entity(&workspace, |w, _, _| match change {
            0 => w.tabs[0].buffer.raw_revision = crate::buffer::ContentRevision::default(),
            1 => w.tabs[0].buffer.encoding = crate::text::encoding::Encoding::Utf16Be,
            2 => w.tabs[0].buffer.line_ending = match w.tabs[0].buffer.line_ending {
                crate::text::line_ending::LineEnding::Lf => crate::text::line_ending::LineEnding::Crlf,
                _ => crate::text::line_ending::LineEnding::Lf,
            },
            3 => w.tabs[0].buffer.file_path = Some(std::env::temp_dir().join("notepad-studio-neutral-exit.txt")),
            4 => w.tabs[0].buffer.mark_saved(),
            5 => w.closed_tabs.push(crate::tab::ClosedTab { buffer: crate::buffer::Buffer::untitled(), session_id: uuid::Uuid::new_v4() }),
            6 => w.tabs[0].session_id = uuid::Uuid::new_v4(),
            _ => w.tabs.push(Tab::empty()),
        });
        release.send(Ok(())).unwrap(); visual.run_until_parked();
        assert!(quits.try_recv().is_err(), "change {change} cannot exit against an earlier capture");
        visual.update_window_entity(&workspace, |w, _, _| {
            assert!(w.status_message.as_deref().is_some_and(|status| status.contains("changed")));
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "before café\n中😀");
        });
        cx.quit();
    }
}

#[test]
fn exit_captures_both_windows_once_and_rejects_a_reopened_window() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (release, jobs) = gate(&mut cx);
    let (first, _) = add_workspace(&mut cx);
    let (_second, visual) = add_workspace(&mut cx);
    visual.dispatch_action(Quit); visual.run_until_parked();
    let snapshots = jobs.try_recv().unwrap();
    assert_eq!(snapshots.len(), 2); assert!(jobs.try_recv().is_err());
    visual.update(|window, _| window.remove_window());
    cx.run_until_parked();
    let (reopened, visual) = add_workspace(&mut cx);
    release.send(Ok(())).unwrap(); visual.run_until_parked();
    assert!(quits.try_recv().is_err());
    visual.update(|_, cx| {
        assert_ne!(first.read(cx).session_window_id, reopened.read(cx).session_window_id);
        assert_eq!(first.read(cx).buffer().rope.to_string(), "before café\n中😀");
        assert_eq!(reopened.read(cx).buffer().rope.to_string(), "before café\n中😀");
    });
    cx.quit();
}

#[test]
fn exit_late_platform_hook_skips_only_a_fresh_successful_capture() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    assert!(!cx.update(super::has_current_exit_recovery));
    let (release, jobs) = gate(&mut cx);
    let (_workspace, visual) = add_workspace(&mut cx);
    visual.dispatch_action(Quit); visual.run_until_parked(); jobs.try_recv().unwrap();
    assert!(!visual.cx.update(super::has_current_exit_recovery));
    release.send(Ok(())).unwrap(); visual.run_until_parked();
    assert!(quits.try_recv().is_ok());
    assert!(visual.cx.update(super::has_current_exit_recovery));
    visual.simulate_input("新"); visual.run_until_parked();
    assert!(!visual.cx.update(super::has_current_exit_recovery));
    assert!(jobs.try_recv().is_err());
    cx.quit();
}

#[test]
fn exit_pending_csv_unicode_replacement_and_escape_keep_the_current_draft_choice() {
    use gpui::{ElementInputHandler, InputHandler};
    for cancel in [false, true] {
        let (_capture, quits) = QuitCapture::new();
        crate::ui::csv_doc::actions::grid_focus_tests::with_grid(|workspace, visual, _| {
            visual.update(|_, cx| super::super::menus::install_global_actions(cx));
            visual.update_window_entity(workspace, |w, _, _| w.session_final_save_result = Some(true));
            let (release, jobs) = visual.update(|_, cx| gate_in_app(cx));
            visual.simulate_input("日本😀"); visual.run_until_parked();
            let raw = visual.update(|_, cx| workspace.read(cx).buffer().rope.to_string());
            visual.dispatch_action(Quit); visual.run_until_parked();
            let snapshots = jobs.try_recv().unwrap();
            assert_eq!(snapshots[0].tabs[0].csv_draft.as_ref().unwrap().value, "日本😀");
            if cancel { visual.simulate_keystrokes("escape"); } else {
                visual.update(|window, cx| ElementInputHandler::new(window.bounds(), workspace.clone())
                    .replace_text_in_range(Some(0..4), "中文😀", window, cx));
            }
            visual.run_until_parked(); release.send(Ok(())).unwrap(); visual.run_until_parked();
            assert!(quits.try_recv().is_err());
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), raw);
                let draft = w.session_snapshot(crate::session::BakPolicy::Full).tabs[0].csv_draft.clone();
                if cancel { assert!(draft.is_none()); } else { assert_eq!(draft.unwrap().value, "中文😀"); }
            });
        });
    }
}

#[test]
fn exit_intervening_rich_text_is_retained_and_refuses_the_old_ast_capture() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let dir = crate::persist::TempDir::new("quit-rich");
    let path = dir.join("neutral.rtf");
    std::fs::write(&path, br"{\rtf1\ansi before}").unwrap();
    let mut rich = crate::io::read_buffer(&path).unwrap(); rich.file_path = None;
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (release, jobs) = gate(&mut cx);
    let (workspace, visual) = add_workspace(&mut cx);
    visual.update_window_entity(&workspace, |w, _, cx| {
        w.tabs[0].buffer = rich; w.tabs[0].view_mode = crate::tab::TabViewMode::RichDocView;
        let payload = w.tabs[0].buffer.rich_doc.as_mut().unwrap();
        let start = payload.block_start(0); payload.active_block = Some(0);
        payload.selection = Some(crate::rich_doc::TextSelection { anchor: start, cursor: start });
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        window.focus(&workspace.read(cx).rich_doc_body_focus.clone().unwrap(), cx);
        assert!(workspace.read(cx).input_focus(window) == crate::tab::InputFocus::RichDocBody);
    });
    visual.run_until_parked(); visual.dispatch_action(Quit); visual.run_until_parked(); jobs.try_recv().unwrap();
    visual.simulate_input("日本😀"); visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rich_doc.as_ref().unwrap().block_text(0), "日本😀before"));
    release.send(Ok(())).unwrap(); visual.run_until_parked();
    assert!(quits.try_recv().is_err());
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rich_doc.as_ref().unwrap().block_text(0), "日本😀before"));
    cx.quit();
}

#[test]
fn exit_catalog_chord_waits_and_preserves_bom_crlf_metadata() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        super::super::menus::install_global_actions(cx);
        crate::shortcuts::register_catalog(cx, &Default::default());
    });
    let (release, jobs) = gate(&mut cx);
    let (workspace, visual) = add_workspace(&mut cx);
    visual.update_window_entity(&workspace, |w, _, _| {
        w.tabs[0].buffer.encoding = crate::text::encoding::Encoding::Utf8Bom;
        w.tabs[0].buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
    });
    let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == "Quit").unwrap();
    visual.simulate_keystrokes(crate::shortcuts::platform_default_keys(entry)[0]); visual.run_until_parked();
    assert!(quits.try_recv().is_err());
    let mut snapshots = jobs.try_recv().unwrap();
    let dir = crate::persist::TempDir::new("quit-catalog");
    crate::session::write_csv_fixture(snapshots.remove(0), &dir.join("session.toml"), dir.path()).unwrap();
    let manifest: SessionManifest = toml::from_str(&std::fs::read_to_string(dir.join("session.toml")).unwrap()).unwrap();
    assert_eq!(manifest.tabs[0].encoding.into_encoding(), crate::text::encoding::Encoding::Utf8Bom);
    assert_eq!(manifest.tabs[0].line_ending.into_line_ending(), crate::text::line_ending::LineEnding::Crlf);
    release.send(Ok(())).unwrap(); visual.run_until_parked();
    assert!(quits.try_recv().is_ok()); assert!(quits.try_recv().is_err());
    cx.quit();
}

#[test]
fn exit_actual_new_close_and_restore_actions_invalidate_the_captured_document_set() {
    for restore in [false, true] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings { tab_exit_on_last_close: false, ..Default::default() });
        let (_capture, quits) = QuitCapture::new();
        let mut cx = TestAppContext::single();
        cx.update(super::super::menus::install_global_actions);
        let (release, jobs) = gate(&mut cx);
        let (workspace, visual) = add_workspace(&mut cx);
        let original = visual.update_window_entity(&workspace, |w, _, _| {
            w.tabs[0].buffer.mark_saved(); w.tabs[0].session_id
        });
        visual.dispatch_action(Quit); visual.run_until_parked(); jobs.try_recv().unwrap();
        if restore {
            visual.dispatch_action(crate::actions::FileCloseTab); visual.run_until_parked();
            visual.dispatch_action(crate::actions::FileRestoreClosedTab);
        } else { visual.dispatch_action(crate::actions::FileNew); }
        visual.run_until_parked(); release.send(Ok(())).unwrap(); visual.run_until_parked();
        assert!(quits.try_recv().is_err());
        visual.update_window_entity(&workspace, |w, _, _| {
            if restore {
                assert!(w.tabs.iter().any(|tab| tab.session_id == original));
                assert_eq!(w.tabs.iter().find(|tab| tab.session_id == original).unwrap().buffer.rope.to_string(), "before café\n中😀");
            } else { assert_eq!(w.tabs.len(), 2); }
        });
        cx.quit();
    }
}

#[test]
fn exit_late_release_suppresses_final_writes_only_after_fresh_publication() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let (_capture, quits) = QuitCapture::new();
    let mut cx = TestAppContext::single();
    cx.update(super::super::menus::install_global_actions);
    let (release, jobs) = gate(&mut cx);
    let (first, _) = add_workspace(&mut cx);
    let (second, visual) = add_workspace(&mut cx);
    let (sink, snapshots) = channel();
    visual.update(|_, cx| {
        for entity in [&first, &second] {
            entity.update(cx, |w, _| { w.window_closed = false; w.session_snapshot_sink = Some(sink.clone()); });
        }
    });
    visual.dispatch_action(Quit); visual.run_until_parked(); jobs.try_recv().unwrap();
    assert!(!visual.cx.update(super::finish_exit_recovery_if_current));
    visual.update(|_, cx| { assert!(!first.read(cx).window_closed); assert!(!second.read(cx).window_closed); });
    assert!(snapshots.try_recv().is_ok()); assert!(snapshots.try_recv().is_ok()); assert!(snapshots.try_recv().is_err());
    release.send(Ok(())).unwrap(); visual.run_until_parked(); assert!(quits.try_recv().is_ok());
    assert!(visual.cx.update(super::finish_exit_recovery_if_current));
    assert!(!visual.cx.update(super::finish_exit_recovery_if_current), "the teardown consumes its proof once");
    visual.update(|_, cx| {
        for entity in [&first, &second] {
            entity.update(cx, |w, _| {
                assert!(w.window_closed);
                // Remove the synthetic failure override to exercise the real
                // closed-window fast path used by the release callback.
                w.session_final_save_result = None;
                w.save_session_now().unwrap();
            });
        }
    });
    assert!(snapshots.try_recv().is_err(), "release cannot queue another recovery snapshot");
    cx.quit();
}

#[test]
fn final_app_quit_orders_fresh_proof_barrier_and_release_or_keeps_sync_fallback() {
    for mode in ["fresh", "stale", "failed-barrier"] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let (_capture, quits) = QuitCapture::new();
        let mut cx = TestAppContext::single();
        cx.update(super::super::menus::install_global_actions);
        let (release, jobs) = gate(&mut cx);
        let (first, _) = add_workspace(&mut cx);
        let (second, visual) = add_workspace(&mut cx);
        let (sink, snapshots) = channel(); let (barrier_calls, barriers) = channel();
        visual.update(|_, cx| {
            for entity in [&first, &second] {
                entity.update(cx, |w, _| { w.window_closed = false; w.session_snapshot_sink = Some(sink.clone()); });
            }
            cx.set_global(FinalBarrierFixture { failure: (mode == "failed-barrier").then(|| "fixture writer poison".to_owned()), calls: Some(barrier_calls) });
        });
        visual.dispatch_action(Quit); visual.run_until_parked(); jobs.try_recv().unwrap();
        assert!(barriers.try_recv().is_err(), "preflight must not retire queued writes or release live windows");
        assert!(snapshots.try_recv().is_ok()); assert!(snapshots.try_recv().is_ok()); assert!(snapshots.try_recv().is_err());
        release.send(Ok(())).unwrap(); visual.run_until_parked(); assert!(quits.try_recv().is_ok());
        if mode == "stale" { visual.simulate_input("新"); visual.run_until_parked(); }
        let (observe, observed) = channel();
        cx.update(move |cx| {
            cx.on_app_quit(move |cx| {
                super::save_final_recovery(cx);
                let flags = cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>())
                    .map(|handle| handle.update(cx, |w, _, _| w.window_closed).unwrap()).collect::<Vec<_>>();
                let _ = observe.send(flags);
                async {}
            }).detach();
        });
        cx.quit(); // Exercise the same observer stack as the main.rs final hook.
        assert_eq!(observed.recv().unwrap(), vec![mode == "fresh"; 2]);
        if mode == "stale" { assert!(barriers.try_recv().is_err()); }
        else { assert_eq!(barriers.try_recv().unwrap(), vec![false, false], "all windows remain live until barrier success"); }
        if mode == "fresh" { assert!(snapshots.try_recv().is_err(), "no duplicate final snapshot or writer"); }
        else { assert!(snapshots.try_recv().is_ok()); assert!(snapshots.try_recv().is_ok()); }
    }
}

#[test]
fn exit_snapshot_and_write_timing_above_streaming_threshold() {
    use std::time::Instant;
    fn middle(samples: &mut [f64]) -> (f64, f64) {
        samples.sort_by(f64::total_cmp);
        (samples[samples.len() / 2], samples[(samples.len() * 95 / 100).min(samples.len() - 1)])
    }
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    for large in [false, true] {
        let dir = crate::persist::TempDir::new("quit-timing");
        let mut cx = TestAppContext::single();
        let (workspace, visual) = add_workspace(&mut cx);
        let line = "0123456789 café 中😀\n";
        let text = line.repeat(if large { 16 * 1024 * 1024 / line.len() } else { 1024 / line.len() });
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.tabs[0].buffer.rope = ropey::Rope::from_str(&text);
            w.tabs[0].buffer.raw_revision = crate::buffer::ContentRevision::default();
            w.bulk_edit = true;
            for _ in 0..if large { 2000 } else { 1 } { w.replace_byte_range(0..0, "x", cx); }
            w.bulk_edit = false;
            if large {
                w.begin_edit_group();
                for _ in 0..16 { w.replace_byte_range(0..0, "café中😀", cx); }
                w.end_edit_group(cx);
            }
        });
        let iterations = if cfg!(debug_assertions) { 3 } else { 20 };
        let mut before = Vec::new(); let mut after = Vec::new();
        let mut captures = Vec::new(); let mut writes = Vec::new();
        for run in 0..iterations + 2 {
            let start = Instant::now();
            let stamp = visual.update(|_, cx| super::state::WorkspaceStamp::capture(workspace.read(cx)));
            let stamp_ms = start.elapsed().as_secs_f64() * 1000.;
            let start = Instant::now();
            let (snapshot, size) = visual.update(|_, cx| {
                let w = workspace.read(cx);
                (w.session_snapshot(crate::session::BakPolicy::Full), w.buffer().rope.len_bytes())
            });
            let capture_ms = start.elapsed().as_secs_f64() * 1000.;
            let id = snapshot.tabs[0].id;
            let start = Instant::now();
            crate::session::write_csv_fixture(snapshot, &dir.join("session.toml"), dir.path()).unwrap();
            let write_ms = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(std::fs::metadata(dir.join(&format!("{id}.bak"))).unwrap().len(), size as u64);
            if large { assert!(size > crate::io_stream::STREAM_THRESHOLD_BYTES as usize); }
            drop(stamp);
            if run >= 2 {
                before.push(capture_ms + write_ms); after.push(stamp_ms + capture_ms);
                captures.push(capture_ms); writes.push(write_ms);
            }
        }
        let (before, before_p95) = middle(&mut before);
        let (after, after_p95) = middle(&mut after);
        let (capture, _) = middle(&mut captures); let (write, _) = middle(&mut writes);
        eprintln!("[quit timing] large={large}, iterations={iterations}, capture_ms={capture:.3}, write_ms={write:.3}, foreground_sync_ms={before:.3}/{before_p95:.3}p95, foreground_async_ms={after:.3}/{after_p95:.3}p95");
        cx.quit();
    }
}
