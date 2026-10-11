use super::*;
use crate::{actions::{FileOpenRecent, FileClearRecent}, settings::Settings, workspace::Workspace};
use gpui::TestAppContext;
use std::sync::atomic::Ordering;

fn held_ui_route(clear: bool) {
    let _settings = crate::settings::override_for_test(Settings { toolbar_hidden: true,
        folder_panel_visible: false, ..Default::default() });
    let dir = crate::persist::TempDir::new("recent-held-ui");
    let path = dir.join("neutral-cafe.txt");
    std::fs::write(&path, "\u{FEFF}中 café\r\n").unwrap();
    let _recent = override_snapshot_for_test(vec![dir.join("older.txt")]);
    let mut cx = TestAppContext::single();
    let writer = writer::Writer::new(cx.background_executor.clone(), dir.join("recent.toml"));
    let (release, gate) = futures::channel::oneshot::channel();
    *writer.probe.gate.lock().unwrap() = Some(gate);
    let _writer = test_snapshot::override_writer(writer.clone());
    cx.update(|cx| { crate::app::menus::install_global_actions(cx); crate::shortcuts::register_catalog(cx, &Default::default()); });
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    if clear { visual.dispatch_action(FileClearRecent); }
    else { visual.dispatch_action(FileOpenRecent { path: path.clone() }); }
    visual.run_until_parked();
    if clear { assert!(snapshot().is_empty()); }
    else {
        assert_eq!(snapshot().first(), Some(&path));
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "中 café\n"));
    }
    let attempts = writer.probe.inline.load(Ordering::Acquire);
    let started = writer.probe.started.lock().unwrap().len();
    let completed = writer.probe.completed.lock().unwrap().len();
    let _ = release.send(());
    visual.run_until_parked();
    cx.quit();
    assert_eq!(attempts, 0, "actual UI route attempted synchronous MRU persistence");
    assert_eq!(started, 1, "production writer should be held before disk publication");
    assert_eq!(completed, 0, "route must complete while background persistence is held");
}
#[test]
fn real_source_open_recent_finishes_while_recent_writer_is_held() { held_ui_route(false); }
#[test]
fn real_clear_recent_finishes_while_recent_writer_is_held() { held_ui_route(true); }

fn recent(paths: &[&str]) -> RecentFiles {
    RecentFiles { paths: paths.iter().map(PathBuf::from).collect() }
}
fn queue(writer: &std::sync::Arc<writer::Writer>, paths: &[&str]) {
    if let Some(start) = writer::queue(writer, &recent(paths)) { start.start(); }
}
#[test]
fn held_writer_coalesces_open_clear_and_remove_to_the_newest_snapshot() {
    let dir = crate::persist::TempDir::new("recent-order");
    let mut cx = TestAppContext::single();
    let destination = dir.join("recent.toml");
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    let (release, gate) = futures::channel::oneshot::channel();
    *writer.probe.gate.lock().unwrap() = Some(gate);
    queue(&writer, &["a"]);
    cx.run_until_parked();
    assert_eq!(writer.probe.started.lock().unwrap().as_slice(), &[vec![PathBuf::from("a")]]);
    queue(&writer, &["b", "a"]);
    queue(&writer, &[]);
    queue(&writer, &["c"]);
    queue(&writer, &[]);
    assert_eq!(writer.status(), (true, false, Some(Vec::new())));
    assert!(writer.probe.completed.lock().unwrap().is_empty());
    release.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(writer.probe.started.lock().unwrap().len(), 2);
    assert_eq!(load_paths_at(&destination), Vec::<PathBuf>::new());
    assert_eq!(writer.status(), (false, false, None));
    queue(&writer, &["after-idle"]);
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("after-idle")]);
    cx.quit();
}
#[test]
fn failed_write_keeps_previous_file_and_a_later_snapshot_retries() {
    let dir = crate::persist::TempDir::new("recent-failure");
    let destination = dir.join("recent.toml");
    save_at(&destination, &recent(&["previous"])).unwrap();
    let mut cx = TestAppContext::single();
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    writer.probe.fail_next.store(true, Ordering::Release);
    queue(&writer, &["failed"]);
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("previous")]);
    assert_eq!(writer.status(), (false, false, None));
    queue(&writer, &["newest"]);
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("newest")]);
    assert_eq!(writer.probe.completed.lock().unwrap().iter().map(|(_, ok)| *ok)
        .collect::<Vec<_>>(), vec![false, true]);
    cx.quit();
}
#[test]
fn failed_running_write_still_drains_the_newer_pending_snapshot() {
    let dir = crate::persist::TempDir::new("recent-running-failure");
    let mut cx = TestAppContext::single();
    let destination = dir.join("recent.toml");
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    let (release, gate) = futures::channel::oneshot::channel();
    *writer.probe.gate.lock().unwrap() = Some(gate);
    writer.probe.fail_next.store(true, Ordering::Release);
    queue(&writer, &["failed"]);
    cx.run_until_parked();
    queue(&writer, &["newest"]);
    release.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("newest")]);
    assert_eq!(writer.probe.completed.lock().unwrap().iter().map(|(_, ok)| *ok)
        .collect::<Vec<_>>(), vec![false, true]);
    cx.quit();
}
#[test]
fn shutdown_barrier_retires_a_captured_old_write_before_final_publication() {
    let dir = crate::persist::TempDir::new("recent-final");
    let mut cx = TestAppContext::single();
    let destination = dir.join("recent.toml");
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    let (release, gate) = futures::channel::oneshot::channel();
    *writer.probe.gate.lock().unwrap() = Some(gate);
    queue(&writer, &["old"]);
    cx.run_until_parked();
    queue(&writer, &["latest"]);
    writer.finish(|| Ok(recent(&["latest"])));
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("latest")]);
    release.send(()).unwrap();
    cx.run_until_parked();
    assert!(writer.probe.completed.lock().unwrap().is_empty());
    assert_eq!(writer.status(), (false, true, None));
    queue(&writer, &["must-not-write"]);
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("latest")]);
    cx.quit();
}
#[test]
fn bounded_snapshot_preserves_unicode_and_refuses_whole_hostile_lists() {
    let original = recent(&["C:\\neutral\\中😀.rs", "café.txt"]);
    assert_eq!(writer::bounded_snapshot(&original).unwrap().paths, original.paths);
    let oversized = RecentFiles { paths: vec![PathBuf::from("x".repeat(1024 * 1024 + 1))] };
    assert!(writer::bounded_snapshot(&oversized).is_err());
    let too_many = RecentFiles { paths: vec![PathBuf::from("x"); 31] };
    assert!(writer::bounded_snapshot(&too_many).is_err());
    assert!(writer::bounded_snapshot(&RecentFiles::default()).unwrap().paths.is_empty());
}
#[test]
fn failed_real_atomic_write_does_not_replace_previous_recent_file() {
    let dir = crate::persist::TempDir::new("recent-atomic-failure");
    let destination = dir.join("recent.toml");
    save_at(&destination, &recent(&["previous"])).unwrap();
    std::fs::create_dir(dir.join("recent.toml.tmp")).unwrap();
    let mut cx = TestAppContext::single();
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    queue(&writer, &["newest"]);
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("previous")]);
    assert!(!writer.probe.completed.lock().unwrap()[0].1);
    cx.quit();
}

#[test]
fn actual_held_disk_write_leaves_queue_free_and_shutdown_waits_for_serialization() {
    let dir = crate::persist::TempDir::new("recent-held-disk");
    let cx = TestAppContext::single();
    let destination = dir.join("recent.toml");
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *writer.probe.disk_gate.lock().unwrap() = Some(Box::new(move || {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    }));
    assert!(writer.enqueue(recent(&["old"])));
    let worker = writer.clone();
    let worker = std::thread::spawn(move || worker.run_worker_for_test());
    started_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    for index in 0..20 {
        assert!(!writer.enqueue(recent(&[&format!("latest-{index}")])));
    }
    assert_eq!(writer.status(), (true, false, Some(vec![PathBuf::from("latest-19")])));
    let (finish_started_tx, finish_started_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let final_writer = writer.clone();
    let final_worker = std::thread::spawn(move || {
        finish_started_tx.send(()).unwrap();
        final_writer.finish(|| Ok(recent(&["latest-19"])));
        finished_tx.send(()).unwrap();
    });
    finish_started_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(matches!(finished_rx.recv_timeout(std::time::Duration::from_millis(30)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    final_worker.join().unwrap();
    finished_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("latest-19")]);
    assert_eq!(writer.status(), (false, true, None));
    cx.quit();
}
#[test]
fn enqueue_racing_atomic_idle_retirement_starts_the_next_writer() {
    let dir = crate::persist::TempDir::new("recent-idle-race");
    let mut cx = TestAppContext::single();
    let destination = dir.join("recent.toml");
    let writer = writer::Writer::new(cx.background_executor.clone(), destination.clone());
    let (idle_tx, idle_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *writer.probe.idle_gate.lock().unwrap() = Some(Box::new(move || {
        idle_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    }));
    // Reach the idle handoff without making the gate depend on filesystem latency.
    writer.probe.fail_next.store(true, Ordering::Release);
    assert!(writer.enqueue(recent(&["first"])));
    let worker = writer.clone();
    let worker = std::thread::spawn(move || worker.run_worker_for_test());
    idle_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let (enqueue_tx, enqueue_rx) = std::sync::mpsc::channel();
    let next_writer = writer.clone();
    let next = std::thread::spawn(move || {
        enqueue_tx.send(()).unwrap();
        next_writer.enqueue(recent(&["newest"]))
    });
    enqueue_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(next.join().unwrap(), "idle retirement must hand off spawn ownership");
    writer.start();
    cx.run_until_parked();
    assert_eq!(load_paths_at(&destination), vec![PathBuf::from("newest")]);
    assert_eq!(writer.probe.completed.lock().unwrap().iter().map(|(_, ok)| *ok)
        .collect::<Vec<_>>(), vec![false, true]);
    assert_eq!(writer.status(), (false, false, None));
    cx.quit();
}

#[test]
fn absent_remove_does_not_start_a_persistence_write() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let dir = crate::persist::TempDir::new("recent-noop-remove");
    let _recent = override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single();
    let writer = writer::Writer::new(cx.background_executor.clone(), dir.join("recent.toml"));
    let _writer = test_snapshot::override_writer(writer.clone());
    record_opened("keep".into());
    cx.run_until_parked();
    remove_path(Path::new("absent"));
    cx.run_until_parked();
    assert_eq!(snapshot(), vec![PathBuf::from("keep")]);
    assert_eq!(writer.probe.started.lock().unwrap().len(), 1);
    cx.quit();
}
#[test]
fn nested_persistence_fixtures_restore_owned_destinations_and_mru_state() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let outer_dir = crate::persist::TempDir::new("recent-outer-writer");
    let inner_dir = crate::persist::TempDir::new("recent-inner-writer");
    let _outer_recent = override_snapshot_for_test(Vec::new());
    let mut cx = TestAppContext::single();
    let outer_path = outer_dir.join("recent.toml");
    let inner_path = inner_dir.join("recent.toml");
    let outer_writer = writer::Writer::new(cx.background_executor.clone(), outer_path.clone());
    let _outer_writer = test_snapshot::override_writer(outer_writer);
    record_opened("outer".into());
    cx.run_until_parked();
    {
        let _inner_recent = override_snapshot_for_test(Vec::new());
        let inner_writer = writer::Writer::new(cx.background_executor.clone(), inner_path.clone());
        let _inner_writer = test_snapshot::override_writer(inner_writer);
        record_opened("inner".into());
        cx.run_until_parked();
    }
    record_opened("outer-newest".into());
    cx.run_until_parked();
    assert_eq!(load_paths_at(&outer_path), vec![PathBuf::from("outer-newest"), PathBuf::from("outer")]);
    assert_eq!(load_paths_at(&inner_path), vec![PathBuf::from("inner")]);
    cx.quit();
}
