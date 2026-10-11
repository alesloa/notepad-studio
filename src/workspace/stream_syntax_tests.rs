// Exercise the same completion callback used by the streaming file reader.
use super::super::syntax_refresh_tests::with_sources;
use crate::buffer::LoadProgress;
use crate::io_stream::{StreamCompletion, finish_streaming};
use crate::syntax::LanguageId;
use crate::text::{encoding::Encoding, line_ending::LineEnding};
use gpui::{Entity, VisualTestContext};
use std::sync::{Arc, atomic::AtomicBool};

fn completion(entity: &Entity<super::Workspace>, visual: &mut VisualTestContext,
    extension: &str, lossy: bool) -> StreamCompletion
{
    let path = std::path::PathBuf::from(format!("neutral-stream.{extension}"));
    let cancel = Arc::new(AtomicBool::new(false));
    visual.update(|_, cx| entity.update(cx, |w, _| {
        w.hold_syntax_refresh(true);
        let tab = &mut w.tabs[1];
        tab.buffer.file_path = Some(path.clone());
        tab.buffer.rope = ropey::Rope::from_str("fn caf\u{e9}() { let value = \"\u{4e2d}\u{1f600}\"; }\n");
        tab.buffer.language = None;
        tab.buffer.highlights.clear();
        tab.buffer.read_only = true;
        tab.buffer.loading = Some(LoadProgress {
            bytes_read: 100, total_bytes: 100, cancel: cancel.clone(),
        });
    }));
    StreamCompletion {
        path, total_bytes: 100, cancel,
        encoding: Encoding::Utf16Le, line_ending: LineEnding::Crlf, lossy,
    }
}

#[test]
fn stream_syntax_completion_defers_parsing_the_inactive_document() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let untouched = w.tabs[0].buffer.rope.clone();
            let source = w.tabs[1].buffer.rope.clone();
            let revision = w.tabs[1].buffer.raw_revision.clone();
            let uuid = w.tabs[1].session_id;
            let selection = (w.tabs[1].selection.anchor, w.tabs[1].selection.head);
            crate::buffer::highlight_tests::reset_flatten_calls();
            finish_streaming(w, completed, cx);
            assert_eq!(crate::buffer::highlight_tests::flatten_calls(), 0, "EOF must not flatten on the UI thread");
            assert_eq!(w.active, 0);
            assert!(w.tabs[0].buffer.rope.is_instance(&untouched));
            assert!(w.tabs[0].buffer.highlights.is_empty());
            let tab = &w.tabs[1];
            assert_eq!(tab.session_id, uuid);
            assert!(tab.buffer.rope.is_instance(&source));
            assert_eq!(tab.buffer.raw_revision, revision);
            assert_eq!((tab.selection.anchor, tab.selection.head), selection);
            assert!(tab.buffer.loading.is_none());
            assert!(!tab.buffer.read_only);
            assert!(!tab.buffer.dirty);
            assert_eq!(tab.buffer.encoding, Encoding::Utf16Le);
            assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
            assert_eq!(tab.buffer.language, Some(LanguageId::Rust));
            assert!(tab.buffer.highlights.is_empty(), "EOF must not parse on the UI thread");
            assert_eq!(w.syntax_pending_for_test().0, 1);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert!(w.syntax_pending_for_test().2);
            assert!(w.tabs[1].buffer.highlights.is_empty());
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[1].buffer.highlights.is_empty());
            assert!(w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.active, 0);
        }));
    });
}

#[test]
fn stream_syntax_unknown_language_preserves_lossy_read_only_completion() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "txt", true);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            finish_streaming(w, completed, cx);
            assert!(w.tabs[1].buffer.read_only);
            assert!(!w.tabs[1].buffer.dirty);
            assert!(w.tabs[1].buffer.loading.is_none());
            assert!(w.tabs[1].buffer.language.is_none());
            assert!(w.tabs[1].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert_eq!(w.active, 0);
            assert!(w.status_message.as_deref().is_some_and(|s| s.contains("read-only")));
        }));
    });
}

#[test]
fn stream_syntax_old_load_cannot_complete_a_new_load_on_the_same_uuid() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let newer = Arc::new(AtomicBool::new(false));
            w.tabs[1].buffer.loading.as_mut().unwrap().cancel = newer.clone();
            w.set_status("newer load status");
            let source = w.tabs[1].buffer.rope.clone();
            finish_streaming(w, completed, cx);
            assert!(w.tabs[1].buffer.rope.is_instance(&source));
            assert!(w.tabs[1].buffer.read_only);
            assert!(w.tabs[1].buffer.language.is_none());
            assert!(Arc::ptr_eq(&w.tabs[1].buffer.loading.as_ref().unwrap().cancel, &newer));
            assert_eq!(w.status_message.as_deref(), Some("newer load status"));
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        }));
    });
}

#[test]
fn stream_syntax_successful_window_close_retires_the_original_load() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        assert!(visual.simulate_close());
        visual.update(|window, _| window.remove_window());
        visual.cx.update(|cx| {
            entity.update(cx, |w, cx| {
                assert!(w.window_closed);
                let source = w.tabs[1].buffer.rope.clone();
                let revision = w.tabs[1].buffer.raw_revision.clone();
                w.set_status("closed window status");
                finish_streaming(w, completed, cx);
                let tab = &w.tabs[1];
                assert!(tab.buffer.rope.is_instance(&source));
                assert_eq!(tab.buffer.raw_revision, revision);
                assert!(tab.buffer.loading.is_some(), "closed editor must not publish EOF");
                assert!(tab.buffer.read_only);
                assert!(tab.buffer.language.is_none());
                assert!(tab.buffer.highlights.is_empty());
                assert_eq!(w.status_message.as_deref(), Some("closed window status"));
                assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            });
        });
    });
}

#[test]
fn stream_syntax_failed_window_close_keeps_the_original_load_usable() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        visual.update(|_, cx| entity.update(cx, |w, _| w.session_final_save_result = Some(false)));
        assert!(!visual.simulate_close());
        visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                assert!(!w.window_closed);
                finish_streaming(w, completed, cx);
                assert!(w.tabs[1].buffer.loading.is_none());
                assert!(!w.tabs[1].buffer.read_only);
                assert_eq!(w.tabs[1].buffer.language, Some(LanguageId::Rust));
            });
        });
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert!(w.syntax_pending_for_test().2);
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[1].buffer.highlights.is_empty());
            w.session_final_save_result = Some(true);
        }));
    });
}

#[test]
fn stream_syntax_pending_result_follows_uuid_through_tab_reorder() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        visual.update(|_, cx| entity.update(cx, |w, cx| finish_streaming(w, completed, cx)));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let parsed_id = w.tabs[1].session_id;
            w.tabs.swap(0, 1);
            w.active = 1;
            w.set_status("newer active document status");
            w.release_syntax_refresh(cx);
            assert_eq!(w.tabs[0].session_id, parsed_id);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert!(w.tabs[1].buffer.highlights.is_empty());
            assert_eq!(w.active, 1);
            assert_eq!(w.status_message.as_deref(), Some("newer active document status"));
        }));
    });
}

#[test]
fn stream_syntax_pending_result_refuses_an_intervening_edit() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        visual.update(|_, cx| entity.update(cx, |w, cx| finish_streaming(w, completed, cx)));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs[1].buffer.rope.insert(0, "// newer text\n");
            w.tabs[1].buffer.raw_revision = crate::buffer::ContentRevision::default();
            w.set_status("newer edit status");
            let edited = w.tabs[1].buffer.rope.clone();
            w.release_syntax_refresh(cx);
            assert!(w.tabs[1].buffer.rope.is_instance(&edited));
            assert!(w.tabs[1].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert_eq!(w.status_message.as_deref(), Some("newer edit status"));
        }));
    });
}

#[test]
fn stream_syntax_uuid_capture_does_not_redirect_a_missing_target() {
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.set_status("unrelated active status");
            w.request_syntax_refresh_for_tab(uuid::Uuid::new_v4(), std::time::Duration::ZERO, cx);
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert!(w.tabs.iter().all(|tab| tab.buffer.highlights.is_empty()));
            assert_eq!(w.status_message.as_deref(), Some("unrelated active status"));
        }));
    });
}

#[test]
fn stream_syntax_configured_size_gate_refuses_without_a_worker() {
    with_sources(|entity, visual| {
        let completed = completion(entity, visual, "rs", false);
        let mut settings = crate::settings::snapshot();
        settings.large_file_restriction_enabled = true;
        settings.large_file_disable_syntax = true;
        settings.large_file_threshold_bytes = 1;
        let _restricted = crate::settings::override_for_test(settings);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            finish_streaming(w, completed, cx);
            assert_eq!(w.tabs[1].buffer.language, Some(LanguageId::Rust));
            assert!(w.tabs[1].buffer.highlights.is_empty());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert!(w.tabs[1].buffer.loading.is_none());
            assert!(!w.tabs[1].buffer.read_only);
        }));
    });
}

#[test]
fn stream_syntax_closed_editor_refuses_a_new_stream() {
    with_sources(|entity, visual| {
        assert!(visual.simulate_close());
        visual.update(|window, _| window.remove_window());
        visual.cx.update(|cx| entity.update(cx, |w, cx| {
            let ids: Vec<_> = w.tabs.iter().map(|tab| tab.session_id).collect();
            w.set_status("closed editor status");
            crate::io_stream::open_path_streaming(w, "neutral-late.rs".into(),
                crate::io_stream::STREAM_THRESHOLD_BYTES + 1, cx);
            assert_eq!(w.tabs.iter().map(|tab| tab.session_id).collect::<Vec<_>>(), ids);
            assert!(w.tabs.iter().all(|tab| tab.buffer.loading.is_none()));
            assert_eq!(w.status_message.as_deref(), Some("closed editor status"));
        }));
    });
}

#[test]
fn stream_syntax_real_large_file_defers_eof_after_switching_documents() {
    let directory = crate::persist::TempDir::new("stream-syntax");
    let path = directory.join("neutral.rs");
    let raw = format!("\u{feff}fn source() {{ let payload = r#\"{}\"#; }}\r\n",
        "\u{4e2d}\u{1f600}".repeat(1_800_000));
    assert!(raw.len() as u64 > crate::io_stream::STREAM_THRESHOLD_BYTES);
    std::fs::write(&path, raw.as_bytes()).unwrap();
    with_sources(|entity, visual| {
        let loaded_id = visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.hold_syntax_refresh(true);
            w.open_path(path.clone(), cx);
            assert_eq!(w.tabs.len(), 3);
            assert!(w.tabs[2].buffer.loading.is_some());
            let id = w.tabs[2].session_id;
            w.set_active_tab(0, cx);
            assert_eq!(w.active, 0);
            id
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.active, 0);
            let tab = w.tabs.iter().find(|tab| tab.session_id == loaded_id).unwrap();
            assert!(tab.buffer.loading.is_none());
            assert_eq!(tab.buffer.file_path.as_deref(), Some(path.as_path()));
            assert_eq!(tab.buffer.encoding, Encoding::Utf8Bom);
            assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
            assert!(!tab.buffer.dirty);
            assert!(!tab.buffer.read_only);
            assert!(tab.buffer.highlights.is_empty(), "the actual reader must defer EOF parsing");
            assert_eq!(tab.buffer.rope.to_string(), raw.strip_prefix('\u{feff}').unwrap().replace("\r\n", "\n"));
            assert!(w.syntax_pending_for_test().2);
            w.release_syntax_refresh(cx);
            assert!(!w.tabs.iter().find(|tab| tab.session_id == loaded_id).unwrap().buffer.highlights.is_empty());
            assert!(w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.active, 0);
        }));
    });
    assert_eq!(std::fs::read(&path).unwrap(), raw.as_bytes());
}
