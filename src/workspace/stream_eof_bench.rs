use super::super::syntax_refresh_tests::with_sources;
use crate::buffer::LoadProgress;
use crate::io_stream::{StreamCompletion, finish_streaming};
use crate::text::{encoding::Encoding, line_ending::LineEnding};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
#[ignore = "release timing for the real streamed-file completion callback"]
fn stream_eof_release_completion_sample() {
    let source = format!("fn café() {{ let payload = r#\"{}\"#; }}\n", "中😀".repeat(1_800_000));
    assert!(source.len() as u64 > crate::io_stream::STREAM_THRESHOLD_BYTES);
    with_sources(|entity, visual| {
        for sample in 0..3 {
            let cancel = Arc::new(AtomicBool::new(false));
            let loaded = ropey::Rope::from_str(&source);
            let path = std::path::PathBuf::from("neutral-release.rs");
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.hold_syntax_refresh(true);
                w.tabs[1].buffer.rope = loaded.clone();
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.language = None;
                w.tabs[1].buffer.highlights.clear();
                w.tabs[1].buffer.read_only = true;
                w.tabs[1].buffer.loading = Some(LoadProgress {
                    bytes_read: source.len() as u64, total_bytes: source.len() as u64,
                    cancel: cancel.clone(),
                });
            }));
            let (callback, deferred) = visual.update(|_, cx| entity.update(cx, |w, cx| {
                let start = std::time::Instant::now();
                finish_streaming(w, StreamCompletion {
                    path, total_bytes: source.len() as u64, cancel,
                    encoding: Encoding::Utf8Bom, line_ending: LineEnding::Crlf, lossy: false,
                }, cx);
                (start.elapsed(), w.syntax_pending_for_test().0 != 0)
            }));
            visual.run_until_parked();
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.release_syntax_refresh(cx);
                assert!(w.tabs[1].buffer.rope.is_instance(&loaded));
                assert!(w.tabs[1].buffer.loading.is_none());
                assert!(!w.tabs[1].buffer.dirty);
                assert!(!w.tabs[1].buffer.highlights.is_empty());
                assert_eq!(w.active, 0);
                println!("stream EOF sample={sample} bytes={} spans={} deferred={deferred} callback_ms={:.6} capture_ms={:.6} worker_ms={:.6} install_ms={:.6}",
                    source.len(), w.tabs[1].buffer.highlights.len(), callback.as_secs_f64()*1000.,
                    w.syntax_refresh.capture_time.as_secs_f64()*1000.,
                    w.syntax_refresh.worker_time.as_secs_f64()*1000.,
                    w.syntax_refresh.install_time.as_secs_f64()*1000.);
            }));
        }
    });
}
