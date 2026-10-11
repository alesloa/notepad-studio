use super::super::syntax_refresh_tests::with_sources;
use crate::session::{SessionManifest, SessionTab};
use std::time::{Duration, Instant};

fn manifest(path: &std::path::Path, count: usize) -> SessionManifest {
    SessionManifest { tabs: (0..count).map(|_| {
        let mut entry: SessionTab = toml::from_str(&format!("id = \"{}\"", uuid::Uuid::new_v4())).expect("entry");
        entry.path = Some(path.to_path_buf());
        entry.language = Some(crate::syntax::LanguageId::Rust);
        entry
    }).collect(), active: count.saturating_sub(1), ..Default::default() }
}

fn sample(source: &str, count: usize) {
    let dir = crate::persist::TempDir::new("session-release");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, source).expect("fixture");
    std::fs::create_dir_all("log").expect("log");
    if count == 1 { std::fs::write("log/session-syntax-release-fixture.rs", source).expect("frozen fixture"); }
    let manifest = manifest(&path, count);
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let started = Instant::now();
            w.apply_session(manifest);
            let restored = started.elapsed();
            let started = Instant::now();
            w.start_initial_syntax(cx);
            println!("SESSION_SAMPLE bytes={} tabs={} restore={restored:?} post_mount={:?}", source.len(), count, started.elapsed());
        }));
        for _ in 0..400 {
            visual.run_until_parked();
            let done = visual.update(|_, cx| entity.read(cx).tabs.iter().all(|tab| !tab.buffer.highlights.is_empty()));
            if done { break; }
            visual.cx.executor().advance_clock(Duration::from_millis(2));
        }
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs.iter().all(|tab| !tab.buffer.highlights.is_empty()));
            assert_eq!(w.active, count - 1);
            assert!(w.tabs.iter().all(|tab| !tab.buffer.dirty));
            let (capture, worker, publication, callback) = w.session_syntax_bench_times();
            println!("SESSION_PHASES tabs={count} capture={capture:?} worker={worker:?} publication={publication:?} max_pump_callback={callback:?}");
        });
    });
}

#[test]
#[ignore = "optimized actual-handler timing"]
fn session_syntax_release_large_restore_sample() {
    let mut source = String::from("fn caf\u{e9}() {}\n");
    source.push_str(&"// neutral startup filler abcdefghijklmnopqrstuvwxyz\n".repeat(260_000));
    assert!(source.len() > crate::io_stream::STREAM_THRESHOLD_BYTES as usize);
    for _ in 0..3 { sample(&source, 1); }
}

#[test]
#[ignore = "optimized whole-pump callback timing"]
fn session_syntax_release_300_tabs_sample() {
    sample("fn caf\u{e9}() {}\n", 300);
}
