use super::*;
use gpui::{TestAppContext, VisualContext};
use crate::workspace::Workspace;
use std::sync::mpsc::{channel, Sender, Receiver};
use futures::channel::oneshot;

type Inputs = (Vec<WindowEntry>, Option<SessionManifest>, Uuid);
struct Fixture {
    inputs: Option<Inputs>,
    gate: Option<oneshot::Receiver<()>>,
    snapshots: Sender<SessionManifest>,
    failures: Sender<String>,
    fail_open: bool,
    retirement_gate: Option<oneshot::Receiver<()>>,
    retained_bytes: Option<usize>,
    close_destination: bool,
    phases: Option<Sender<(&'static str, std::time::Duration)>>,
}
impl Global for Fixture {}

pub(super) fn phase_sink(cx: &App) -> Option<Sender<(&'static str, std::time::Duration)>> {
    cx.try_global::<Fixture>()?.phases.clone()
}
pub(super) fn record_phase(cx: &App, phase: &'static str, elapsed: std::time::Duration) {
    if let Some(phases) = phase_sink(cx) { let _ = phases.send((phase, elapsed)); }
}

pub(super) fn take_inputs(cx: &mut App) -> Option<Inputs> {
    cx.try_global::<Fixture>()?;
    cx.global_mut::<Fixture>().inputs.take()
}
pub(super) fn take_gate(cx: &mut App) -> Option<oneshot::Receiver<()>> {
    cx.try_global::<Fixture>()?;
    cx.global_mut::<Fixture>().gate.take()
}
pub(super) fn take_retirement_gate(cx: &mut App) -> Option<oneshot::Receiver<()>> {
    cx.try_global::<Fixture>()?;
    cx.global_mut::<Fixture>().retirement_gate.take()
}
pub(in crate::app) fn check_open(cx: &App) -> anyhow::Result<()> {
    if cx.try_global::<Fixture>().is_some_and(|fixture| fixture.fail_open) {
        anyhow::bail!("injected native window creation failure");
    }
    Ok(())
}
pub(in crate::app) fn close_transfer_destination(handle: gpui::WindowHandle<Workspace>, cx: &mut App) {
    if cx.try_global::<Fixture>().is_some_and(|fixture| fixture.close_destination) {
        handle.update(cx, |_, window, _| window.remove_window()).unwrap();
    }
}
pub(in crate::app) fn capture_retained(restored: Option<&RestoredSession>, cx: &mut App) {
    if cx.try_global::<Fixture>().is_none() { return; }
    cx.global_mut::<Fixture>().retained_bytes = restored.map(|session| session.tabs.iter().map(|tab| tab.buffer.rope.len_bytes()).sum());
}
pub(super) fn capture_failure(cx: &App, message: &str) -> bool {
    let Some(fixture) = cx.try_global::<Fixture>() else { return false; };
    let _ = fixture.failures.send(message.to_owned());
    true
}
pub(in crate::app) fn configure_workspace(workspace: &mut Workspace, cx: &App) {
    if let Some(fixture) = cx.try_global::<Fixture>() {
        workspace.session_snapshot_sink = Some(fixture.snapshots.clone());
        workspace.session_registration_sink = Some(fixture.snapshots.clone());
        workspace.session_final_save_result = Some(true);
    }
}
fn install(cx: &mut TestAppContext, manifest: SessionManifest, welcome: Uuid) -> (oneshot::Sender<()>, Receiver<SessionManifest>, Receiver<String>) {
    let (release, gate) = oneshot::channel();
    let (snapshots, saves) = channel();
    let (failures, errors) = channel();
    cx.update(|cx| {
        crate::shortcuts::register_catalog(cx, &Default::default());
        cx.set_global(Fixture { inputs: Some((Vec::new(), Some(manifest), welcome)),
            gate: Some(gate), snapshots, failures, fail_open: false, retirement_gate: None, retained_bytes: None, close_destination: false, phases: None });
    });
    (release, saves, errors)
}
fn entry(path: &std::path::Path, id: Uuid) -> crate::session::SessionTab {
    let mut entry: crate::session::SessionTab = toml::from_str(&format!("id = \"{id}\"" )).expect("entry");
    entry.path = Some(path.to_path_buf());
    entry.language = Some(crate::syntax::LanguageId::Rust);
    entry
}
fn workspace(cx: &mut TestAppContext) -> gpui::WindowHandle<Workspace> {
    let handle = cx.update(|cx| cx.windows().into_iter().find_map(|window| window.downcast::<Workspace>()).expect("published editor"));
    cx.update(|cx| handle.update(cx, |workspace, window, cx| {
        window.activate_window(); window.focus(&workspace.focus_handle, cx); cx.notify();
    }).unwrap());
    cx.run_until_parked();
    handle
}

#[test]
fn startup_controller_waits_before_first_registration_and_preserves_duplicate_sources() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-before-window");
    let first = dir.join("first.rs"); let second = dir.join("second.rs");
    std::fs::write(&first, "fn first() {}\n").unwrap(); std::fs::write(&second, "fn second() {}\n").unwrap();
    let id = Uuid::new_v4();
    let mut cx = TestAppContext::single();
    let (release, saves, errors) = install(&mut cx, SessionManifest { tabs: vec![entry(&first, id), entry(&second, id)], active: 1, ..Default::default() }, Uuid::new_v4());
    cx.update(|cx| begin(cx, false)); cx.run_until_parked();
    cx.update(|cx| { assert!(cx.windows().is_empty()); assert!(blocks_editor(cx)); assert!(protects_placement(cx)); });
    assert!(saves.try_recv().is_err(), "no placeholder registration or save");
    release.send(()).unwrap(); cx.run_until_parked();
    let registered = saves.try_recv().expect("first registration");
    assert_eq!(registered.tabs.len(), 2); assert_ne!(registered.tabs[0].id, registered.tabs[1].id); assert_eq!(registered.active, 1);
    let handle = workspace(&mut cx);
    cx.update(|cx| handle.update(cx, |w, _, _| {
        assert_eq!(w.tabs[0].buffer.rope.to_string(), "fn first() {}\n");
        assert_eq!(w.tabs[1].buffer.rope.to_string(), "fn second() {}\n");
        assert_eq!(w.active, 1); assert!(w.status_message.as_deref().unwrap().contains("repaired 1 duplicate"));
        w.save_session_now().unwrap();
    }).unwrap());
    assert_eq!(saves.try_recv().expect("actual final-save caller").tabs.len(), 2);
    assert!(errors.try_recv().is_err());
    cx.quit();
}

#[test]
fn startup_controller_quit_discards_held_worker_and_protects_placement() {
    let mut cx = TestAppContext::single();
    let (release, saves, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| begin(cx, false)); cx.run_until_parked();
    cx.update(super::super::quit::quit_with_recovery);
    // The real Quit path invalidates before its empty-editor final capture.
    assert!(release.send(()).is_ok()); cx.run_until_parked();
    cx.update(|cx| { assert!(cx.windows().is_empty()); assert!(protects_placement(cx)); });
    assert!(saves.try_recv().is_err());
    cx.quit();
}

#[test]
fn startup_controller_native_open_failure_keeps_recovery_unregistered_and_retry_works() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-window-refusal");
    let path = dir.join("source.rs"); std::fs::write(&path, "fn retained() {}\n").unwrap();
    let manifest = SessionManifest { tabs: vec![entry(&path, Uuid::new_v4())], ..Default::default() };
    let mut cx = TestAppContext::single();
    let (release, saves, errors) = install(&mut cx, manifest.clone(), Uuid::new_v4());
    cx.update(|cx| { cx.global_mut::<Fixture>().fail_open = true; begin(cx, false); });
    cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked();
    assert!(errors.try_recv().unwrap().contains("native window creation")); assert!(saves.try_recv().is_err());
    cx.update(|cx| {
        assert!(cx.windows().is_empty()); assert!(protects_placement(cx));
        let fixture = cx.global_mut::<Fixture>(); fixture.fail_open = false;
        fixture.inputs = Some((Vec::new(), Some(manifest), Uuid::new_v4()));
        let failure = cx.global::<Startup>().failure.clone().expect("failure choice");
        answer_failure(&failure, true, cx);
    });
    cx.run_until_parked(); assert_eq!(saves.try_recv().unwrap().tabs.len(), 1);
    let handle = workspace(&mut cx);
    cx.update(|cx| handle.update(cx, |w, _, _| assert_eq!(w.buffer().rope.to_string(), "fn retained() {}\n")).unwrap());
    cx.quit();
}

#[test]
fn startup_controller_retry_storm_waits_for_physical_refusal_retirement() {
    let dir = crate::persist::TempDir::new("startup-held-retirement");
    let path = dir.join("large.txt"); std::fs::write(&path, vec![b'a'; 12 * 1024 * 1024]).unwrap();
    let mut cx = TestAppContext::single();
    let (release, saves, errors) = install(&mut cx, SessionManifest { tabs: vec![entry(&path, Uuid::new_v4())], ..Default::default() }, Uuid::new_v4());
    let (retired, retirement_gate) = oneshot::channel();
    cx.update(|cx| {
        let fixture = cx.global_mut::<Fixture>(); fixture.fail_open = true;
        fixture.retirement_gate = Some(retirement_gate);
        begin(cx, false);
    });
    cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked();
    assert!(errors.try_recv().is_err(), "Retry dialog waits for physical disposal");
    cx.update(|cx| {
        assert_eq!(cx.global::<Fixture>().retained_bytes, Some(12 * 1024 * 1024));
        let request = cx.global::<Startup>().request.clone().expect("retirement occupies the slot");
        cx.global_mut::<Fixture>().inputs = Some((Vec::new(), Some(SessionManifest::default()), Uuid::new_v4()));
        for _ in 0..100 { super::super::window::restore_windows_on_boot(cx); }
        assert!(Arc::ptr_eq(cx.global::<Startup>().request.as_ref().unwrap(), &request));
        assert!(cx.global::<Fixture>().inputs.is_some(), "no next manifest admitted");
        assert!(cx.windows().is_empty());
    });
    assert!(saves.try_recv().is_err());
    retired.send(()).unwrap(); cx.run_until_parked();
    assert!(errors.try_recv().unwrap().contains("native window"));
    cx.update(|cx| assert!(cx.global::<Startup>().request.is_none()));
    cx.quit();
}

#[test]
fn startup_controller_late_worker_does_not_overwrite_a_new_editor() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-late-editor");
    let path = dir.join("later.rs"); std::fs::write(&path, "fn later() {}\n").unwrap();
    let mut cx = TestAppContext::single();
    let (release, saves, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    let (retired, retirement_gate) = oneshot::channel();
    cx.update(|cx| { cx.global_mut::<Fixture>().retirement_gate = Some(retirement_gate); begin(cx, false); });
    cx.run_until_parked();
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut workspace = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        workspace.tabs[0].buffer.rope = ropey::Rope::from_str("new unsaved text");
        workspace.tabs[0].buffer.dirty = true;
        configure_workspace(&mut workspace, cx);
        workspace
    });
    release.send(()).unwrap(); visual.run_until_parked();
    visual.update_window_entity(&entity, |w, _, _| {
        assert_eq!(w.buffer().rope.to_string(), "new unsaved text"); assert!(w.buffer().dirty);
    });
    visual.cx.update(|cx| {
        assert_eq!(cx.windows().len(), 1); assert!(cx.global::<Startup>().request.is_some());
        assert!(!blocks_editor(cx), "existing editor remains usable while old metadata is retired");
        super::super::window::open_path_in_active_or_new_window(path.clone(), cx);
        super::super::window::open_editor_window(cx);
        assert_eq!(cx.windows().len(), 2);
    });
    visual.run_until_parked();
    visual.update_window_entity(&entity, |w, _, _| {
        assert_eq!(w.tabs[0].buffer.rope.to_string(), "new unsaved text"); assert!(w.tabs[0].buffer.dirty);
        assert!(w.tabs.iter().any(|tab| tab.buffer.file_path.as_ref() == Some(&path)));
    });
    assert!(saves.try_recv().is_ok(), "later explicit Open/New still reach their real callers");
    retired.send(()).unwrap(); visual.run_until_parked();
    visual.update(|_, cx| assert!(cx.global::<Startup>().request.is_none()));
    cx.quit();
}

#[test]
fn startup_controller_open_and_new_intents_are_retained_until_publication() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-incoming-path");
    let path = dir.join("incoming.rs"); std::fs::write(&path, "fn incoming() {}\n").unwrap();
    let mut cx = TestAppContext::single();
    let (release, saves, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| {
        begin(cx, false);
        super::super::window::open_editor_window_with_initial_path(path.clone(), cx).expect("retained queued path");
        super::super::window::open_editor_window(cx);
        assert!(crate::single_instance::take_ready_open_paths(cx).is_empty());
        assert_eq!(cx.global::<Startup>().pending_new, 1);
        assert!(cx.windows().is_empty());
    });
    cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked();
    let registered = saves.try_recv().unwrap(); assert_eq!(registered.tabs.len(), 0, "clean untitled fallback is intentionally not persisted");
    assert!(saves.try_recv().is_ok(), "queued New receives its own editor");
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 2);
        assert_eq!(crate::single_instance::take_ready_open_paths(cx), vec![path]);
    });
    cx.quit();
}

#[test]
fn startup_controller_open_welcome_collision_uses_reserved_scratch_identity() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, startup_open_blank: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-open-welcome-id");
    let path = dir.join("saved.rs"); std::fs::write(&path, "fn saved() {}\n").unwrap();
    let welcome = Uuid::new_v4(); let mut cx = TestAppContext::single();
    let (release, saves, _) = install(&mut cx, SessionManifest { tabs: vec![entry(&path, welcome)], ..Default::default() }, welcome);
    cx.update(|cx| begin(cx, false)); cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked();
    let registered = saves.try_recv().unwrap();
    assert_eq!(registered.tabs.len(), 1, "clean scratch is omitted by existing persistence policy");
    assert_eq!(registered.tabs[0].id, welcome);
    let handle = workspace(&mut cx);
    let scratch = cx.update(|cx| handle.update(cx, |w, _, _| {
        assert_eq!(w.tabs.len(), 2); assert_eq!(w.active, 1);
        assert_eq!(w.tabs[0].session_id, welcome); assert_ne!(w.tabs[1].session_id, welcome);
        w.tabs[1].session_id
    }).unwrap());
    cx.simulate_input(handle.into(), "scratch"); cx.run_until_parked();
    cx.update(|cx| handle.update(cx, |w, _, _| {
        assert_eq!(w.tabs[0].buffer.rope.to_string(), "fn saved() {}\n");
        assert_eq!(w.tabs[1].buffer.rope.to_string(), "scratch");
        assert_eq!(w.tabs[1].session_id, scratch);
        w.save_session_now().unwrap();
    }).unwrap());
    assert!(saves.try_iter().any(|snapshot| snapshot.tabs.iter().any(|entry| entry.id == scratch && entry.dirty)));
    cx.quit();
}

#[test]
fn startup_controller_os_quit_rejects_late_result_without_save_or_placement_reset() {
    let mut cx = TestAppContext::single();
    let (release, saves, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| begin(cx, false)); cx.run_until_parked();
    cx.update(|cx| {
        cancel_for_app_quit(cx);
        super::super::quit::save_final_recovery(cx);
        assert!(protects_placement(cx));
    });
    release.send(()).unwrap(); cx.run_until_parked();
    cx.update(|cx| { assert!(cx.windows().is_empty()); begin(cx, false); assert!(cx.global::<Startup>().request.is_none()); });
    assert!(saves.try_recv().is_err());
    cx.quit();
}

#[test]
fn startup_controller_closed_only_welcome_collision_preserves_one_fallback_and_history() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, startup_open_blank: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-closed-welcome-id");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let welcome = Uuid::new_v4();
    std::fs::write(dir.join(&format!("{welcome}.bak")), "closed recovery").unwrap();
    let closed: crate::session::SessionClosedTab = toml::from_str(&format!("id = \"{welcome}\"\ndirty = true")).unwrap();
    let mut cx = TestAppContext::single();
    let (release, saves, _) = install(&mut cx, SessionManifest {
        tabs: vec![entry(&dir.join("missing.rs"), Uuid::new_v4())], closed_tabs: vec![closed],
        theme: Some(crate::theme::ThemeMode::Light), show_whitespace: true, word_wrap: true, ..Default::default()
    }, welcome);
    cx.update(|cx| begin(cx, false)); cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked();
    let registered = saves.try_recv().unwrap(); assert_eq!(registered.closed_tabs.len(), 1); assert_eq!(registered.closed_tabs[0].id, welcome);
    let handle = workspace(&mut cx);
    cx.update(|cx| handle.update(cx, |w, _, _| {
        assert_eq!(w.tabs.len(), 1); assert_eq!(w.active, 0); assert_ne!(w.tabs[0].session_id, welcome);
        assert_eq!(w.closed_tabs.len(), 1); assert_eq!(w.closed_tabs[0].session_id, welcome);
        assert_eq!(w.closed_tabs[0].buffer.rope.to_string(), "closed recovery");
        assert_eq!(w.theme_mode, crate::theme::ThemeMode::Light); assert!(w.show_whitespace && w.word_wrap);
        assert!(w.status_message.as_deref().unwrap().contains("1 couldn't be reopened"));
    }).unwrap());
    cx.simulate_input(handle.into(), "unsaved welcome"); cx.run_until_parked();
    cx.update(|cx| handle.update(cx, |w, _, _| w.save_session_now().unwrap()).unwrap());
    assert!(saves.try_iter().any(|snapshot| snapshot.tabs.iter().any(|entry| entry.dirty && entry.id != welcome)));
    cx.quit();
}

#[test]
fn startup_controller_failure_exit_choice_cannot_start_a_late_retry() {
    let mut cx = TestAppContext::single();
    let (release, saves, errors) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| { cx.global_mut::<Fixture>().fail_open = true; begin(cx, false); });
    cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked(); assert!(errors.try_recv().is_ok());
    cx.update(|cx| {
        let failure = cx.global::<Startup>().failure.clone().expect("failure choice");
        answer_failure(&failure, false, cx);
        assert!(protects_placement(cx));
        answer_failure(&failure, true, cx); begin(cx, false);
        assert!(cx.global::<Startup>().request.is_none()); assert!(cx.windows().is_empty());
    });
    assert!(saves.try_recv().is_err()); cx.quit();
}

#[test]
fn startup_controller_no_manifest_fallback_registers_only_after_metadata_completion() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let mut cx = TestAppContext::single(); let welcome = Uuid::new_v4();
    let (release, saves, errors) = install(&mut cx, SessionManifest::default(), welcome);
    cx.update(|cx| { cx.global_mut::<Fixture>().inputs = Some((Vec::new(), None, welcome)); begin(cx, false); });
    cx.run_until_parked(); assert!(saves.try_recv().is_err());
    release.send(()).unwrap(); cx.run_until_parked();
    assert!(saves.try_recv().unwrap().tabs.is_empty());
    let handle = workspace(&mut cx);
    cx.update(|cx| handle.update(cx, |w, _, _| {
        assert_eq!(w.tabs.len(), 1); assert_eq!(w.tabs[0].session_id, welcome);
        assert!(w.closed_tabs.is_empty()); assert_eq!(w.active, 0); assert!(!w.buffer().read_only);
    }).unwrap());
    assert!(errors.try_recv().is_err()); cx.quit();
}

#[test]
fn startup_controller_cancelled_ordinary_quit_keeps_new_and_open_available() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-cancelled-quit"); let path = dir.join("ordinary.rs");
    std::fs::write(&path, "fn ordinary() {}\n").unwrap();
    let mut cx = TestAppContext::single();
    let (release, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| begin(cx, false)); cx.run_until_parked(); release.send(()).unwrap(); cx.run_until_parked();
    let handle = workspace(&mut cx);
    cx.update(|cx| handle.update(cx, |w, _, _| w.session_final_save_result = Some(false)).unwrap());
    cx.update(super::super::quit::quit_with_recovery); cx.run_until_parked();
    cx.update(|cx| {
        assert!(!blocks_editor(cx)); assert!(!cx.global::<Startup>().quitting);
        handle.update(cx, |w, _, _| {
            assert!(w.status_message.as_deref().unwrap().contains("Exit cancelled"));
            w.session_final_save_result = Some(true);
        }).unwrap();
        super::super::window::open_path_in_active_or_new_window(path.clone(), cx);
        super::super::window::open_editor_window(cx);
        assert_eq!(cx.windows().len(), 2);
    });
    cx.run_until_parked();
    cx.update(|cx| handle.update(cx, |w, _, _| assert!(w.tabs.iter().any(|tab| tab.buffer.file_path.as_ref() == Some(&path)))).unwrap());
    cx.quit();
}

fn add_transfer_source(cx: &mut TestAppContext) -> (gpui::Entity<Workspace>, &mut gpui::VisualTestContext) {
    cx.add_window_view(|window, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs = ["keep source", "transferred edits"].into_iter().map(|text| {
            let mut tab = crate::tab::Tab::empty(); tab.buffer.rope = ropey::Rope::from_str(text);
            tab.buffer.dirty = true; tab
        }).collect();
        configure_workspace(&mut w, cx); window.activate_window(); window.focus(&w.focus_handle, cx); w
    })
}

#[test]
fn startup_transfer_actual_move_borrowed_source_targets_the_fresh_window() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    let (source, visual) = add_transfer_source(&mut cx);
    let id = visual.update_window_entity(&source, |w, window, cx| {
        let id = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, true, window, cx);
        id
    });
    visual.run_until_parked();
    let owners = visual.cx.update(|cx| {
        let handles: Vec<_> = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).collect();
        handles.into_iter().map(|handle| handle.update(cx, |w, _, _| w.tabs.iter().filter(|tab| tab.session_id == id && tab.buffer.rope == "transferred edits").count()).unwrap()).sum::<usize>()
    });
    assert_eq!(owners, 1, "actual Move must retain the owned tab in its new window");
    visual.update_window_entity(&source, |w, _, _| assert_eq!(w.tabs.len(), 1));
    cx.quit();
}

#[test]
fn startup_transfer_actual_native_refusal_returns_document_to_source() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| cx.global_mut::<Fixture>().fail_open = true);
    let (source, visual) = add_transfer_source(&mut cx);
    visual.update_window_entity(&source, |w, window, cx| {
        let id = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, true, window, cx);
        assert!(w.tabs.iter().any(|tab| tab.session_id == id && tab.buffer.rope == "transferred edits"), "native refusal must return the exact dirty document to the source");
    });
    cx.quit();
}

#[test]
fn startup_transfer_actual_move_during_held_metadata_preserves_document() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let mut cx = TestAppContext::single(); let (release, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| begin(cx, false)); cx.run_until_parked();
    let (source, visual) = add_transfer_source(&mut cx);
    let id = visual.update_window_entity(&source, |w, window, cx| {
        let id = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, true, window, cx); id
    });
    let owners = visual.cx.update(|cx| {
        let handles: Vec<_> = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).collect();
        handles.into_iter().map(|handle| handle.update(cx, |w, _, _| w.tabs.iter().filter(|tab| tab.session_id == id).count()).unwrap()).sum::<usize>()
    });
    assert_eq!(owners, 1, "startup gating must not drop a moved dirty tab");
    release.send(()).unwrap(); visual.run_until_parked(); cx.quit();
}

#[test]
fn startup_transfer_actual_update_refusal_returns_exact_document_to_source() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| cx.global_mut::<Fixture>().close_destination = true);
    let (source, visual) = add_transfer_source(&mut cx);
    visual.update_window_entity(&source, |w, window, cx| {
        let id = w.tabs[1].session_id; let rope = w.tabs[1].buffer.rope.clone();
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, true, window, cx);
        let returned = w.tabs.iter().find(|tab| tab.session_id == id).expect("refused destination returns document");
        assert!(returned.buffer.rope.is_instance(&rope)); assert!(returned.buffer.dirty);
        assert_eq!(returned.buffer.rope.to_string(), "transferred edits");
        assert!(w.status_message.as_deref().unwrap().contains("Document kept"));
    });
    cx.quit();
}

#[test]
fn startup_transfer_actual_open_copy_preserves_source_and_forks_uuid() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    let (source, visual) = add_transfer_source(&mut cx);
    let original = visual.update_window_entity(&source, |w, window, cx| {
        let original = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, false, window, cx);
        assert_eq!(w.tabs.len(), 2); assert_eq!(w.tabs[1].session_id, original);
        assert_eq!(w.tabs[1].buffer.rope.to_string(), "transferred edits"); original
    });
    let ids = visual.cx.update(|cx| {
        let handles: Vec<_> = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).collect();
        handles.into_iter().flat_map(|handle| handle.update(cx, |w, _, _| w.tabs.iter().filter(|tab| tab.buffer.rope == "transferred edits").map(|tab| tab.session_id).collect::<Vec<_>>()).unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(ids.len(), 2); assert!(ids.contains(&original)); assert_ne!(ids[0], ids[1]); cx.quit();
}

#[test]
fn startup_transfer_no_editor_refusal_retains_owned_bytes_and_uuid() {
    let mut cx = TestAppContext::single(); let (release, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| begin(cx, false)); cx.run_until_parked();
    let mut tab = crate::tab::Tab::empty(); tab.buffer.rope = ropey::Rope::from_str("owned unsaved transfer"); tab.buffer.dirty = true;
    let id = tab.session_id; let rope = tab.buffer.rope.clone();
    let returned = cx.update(|cx| {
        let (tab, message) = super::super::window::open_tab_in_new_window(tab, cx).expect("caller retains unsupported transfer");
        assert!(cx.windows().is_empty()); assert!(message.contains("recovery")); tab
    });
    assert_eq!(returned.session_id, id); assert!(returned.buffer.rope.is_instance(&rope)); assert!(returned.buffer.dirty);
    assert_eq!(returned.buffer.rope.to_string(), "owned unsaved transfer");
    cx.update(cancel_for_app_quit); release.send(()).unwrap(); cx.run_until_parked(); cx.quit();
}

#[test]
fn startup_transfer_actual_path_open_targets_fresh_window_without_changing_source() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-path-transfer"); let path = dir.join("disk.rs");
    std::fs::write(&path, "fn disk() {}\n").unwrap();
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    let (source, visual) = add_transfer_source(&mut cx);
    let original = visual.update_window_entity(&source, |w, window, cx| {
        w.tabs[1].buffer.file_path = Some(path.clone()); let original = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, false, window, cx);
        assert_eq!(w.tabs[1].session_id, original); assert_eq!(w.tabs[1].buffer.rope.to_string(), "transferred edits");
        assert!(w.tabs[1].buffer.dirty); original
    });
    visual.run_until_parked();
    let destinations = visual.cx.update(|cx| {
        let handles: Vec<_> = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).collect();
        handles.into_iter().map(|handle| handle.update(cx, |w, _, _| w.tabs.iter().filter(|tab| tab.session_id != original && tab.buffer.file_path.as_ref() == Some(&path) && tab.buffer.rope == "fn disk() {}\n").count()).unwrap()).sum::<usize>()
    });
    assert_eq!(std::fs::read(&path).unwrap(), b"fn disk() {}\n");
    assert_eq!(destinations, 1, "path-backed Open must reach the fresh destination while source is borrowed"); cx.quit();
}

#[test]
fn startup_transfer_actual_path_native_refusal_reports_to_unchanged_source() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let dir = crate::persist::TempDir::new("startup-path-refusal"); let path = dir.join("disk.rs");
    std::fs::write(&path, "fn disk() {}\n").unwrap();
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| cx.global_mut::<Fixture>().fail_open = true);
    let (source, visual) = add_transfer_source(&mut cx);
    visual.update_window_entity(&source, |w, window, cx| {
        w.tabs[1].buffer.file_path = Some(path.clone()); let original = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, false, window, cx);
        assert_eq!(w.tabs[1].session_id, original); assert_eq!(w.tabs[1].buffer.rope.to_string(), "transferred edits"); assert!(w.tabs[1].buffer.dirty);
        assert!(w.status_message.as_deref().is_some_and(|status| status.contains("Could not open")), "native path refusal must reach the still-live source status");
    });
    assert_eq!(std::fs::read(&path).unwrap(), b"fn disk() {}\n"); cx.quit();
}

#[test]
fn startup_transfer_actual_path_update_refusal_reports_to_unchanged_source() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let dir = crate::persist::TempDir::new("startup-path-update-refusal"); let path = dir.join("disk.rs");
    std::fs::write(&path, "fn disk() {}\n").unwrap();
    let mut cx = TestAppContext::single(); let (_unused, _, _) = install(&mut cx, SessionManifest::default(), Uuid::new_v4());
    cx.update(|cx| cx.global_mut::<Fixture>().close_destination = true);
    let (source, visual) = add_transfer_source(&mut cx);
    visual.update_window_entity(&source, |w, window, cx| {
        w.tabs[1].buffer.file_path = Some(path.clone()); let original = w.tabs[1].session_id;
        crate::ui::tab_context_menu::transfer_for_startup_test(w, 1, false, window, cx);
        assert_eq!(w.tabs[1].session_id, original); assert_eq!(w.tabs[1].buffer.rope.to_string(), "transferred edits"); assert!(w.tabs[1].buffer.dirty);
        assert!(w.status_message.as_deref().is_some_and(|status| status.contains("Could not open")), "destination update refusal must reach the still-live source status");
    });
    assert_eq!(std::fs::read(&path).unwrap(), b"fn disk() {}\n"); cx.quit();
}

#[test]
#[ignore = "optimized after-only startup phase evidence"]
fn startup_controller_release_phase_evidence() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings { toolbar_hidden: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let dir = crate::persist::TempDir::new("startup-release-phases"); let path = dir.join("sample.txt");
    let unit = "alpha \u{e9} \u{1f980}\r\n";
    let source = unit.repeat((12 * 1024 * 1024) / unit.len() + 1);
    std::fs::write(&path, source.as_bytes()).unwrap();
    println!("fixture_bytes={} unit_bytes={} repetitions={} scope=actual_controller_test_platform_no_native_render_loaders_excluded", source.len(), unit.len(), source.len() / unit.len());
    for sample in 0..3 {
        let mut cx = TestAppContext::single();
        let mut saved = entry(&path, Uuid::new_v4()); saved.language = None;
        let (release, saves, errors) = install(&mut cx, SessionManifest { tabs: vec![saved], ..Default::default() }, Uuid::new_v4());
        let (phases, timings) = channel(); cx.update(|cx| cx.global_mut::<Fixture>().phases = Some(phases));
        cx.update(|cx| begin(cx, false)); cx.run_until_parked();
        assert!(saves.try_recv().is_err());
        let started = std::time::Instant::now(); release.send(()).unwrap(); cx.run_until_parked();
        let total = started.elapsed(); assert!(saves.try_recv().is_ok()); assert!(errors.try_recv().is_err());
        cx.update(|cx| {
            let handle = cx.windows().into_iter().find_map(|h| h.downcast::<Workspace>()).unwrap();
            handle.update(cx, |w, _, _| { assert_eq!(w.tabs.len(), 1); assert_eq!(w.tabs[0].buffer.rope.len_bytes(), source.len()); assert!(!w.tabs[0].buffer.dirty); }).unwrap();
        });
        for (phase, elapsed) in timings.try_iter() { println!("sample={sample} {phase}_ms={:.3}", elapsed.as_secs_f64() * 1000.); }
        println!("sample={sample} release_to_test_publication_ms={:.3}", total.as_secs_f64() * 1000.);
        cx.quit();
    }
}
