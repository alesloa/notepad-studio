use super::super::syntax_refresh_tests::with_sources;
use crate::{actions::FileRestoreClosedTab, syntax::LanguageId, tab::{ClosedTab, Tab},
    text::{encoding::Encoding, line_ending::LineEnding}};

#[test]
fn initial_syntax_open_defers_actual_handler_and_preserves_saved_metadata() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let dir = crate::persist::TempDir::new("initial-open");
        let path = dir.join("neutral.rs");
        let text = "fn café() { let value = \"中😀\"; }\r\n";
        let bytes = crate::text::encoding::encode(text, encoding);
        std::fs::write(&path, &bytes).expect("fixture");
        with_sources(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.hold_syntax_refresh(true);
                crate::buffer::highlight_tests::reset_flatten_calls();
                w.open_path(path.clone(), cx);
                assert_eq!(crate::buffer::highlight_tests::flatten_calls(), 0);
                let tab = &w.tabs[w.active];
                assert_eq!(tab.buffer.encoding, encoding);
                assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
                assert_eq!(tab.buffer.language, Some(LanguageId::Rust));
                assert!(!tab.buffer.dirty);
                assert!(!tab.buffer.read_only);
                assert_eq!(tab.buffer.rope.to_string(), text.replace("\r\n", "\n"));
                assert!(tab.buffer.highlights.is_empty());
                w.active = 0;
            }));
            visual.run_until_parked();
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                assert!(w.syntax_pending_for_test().2);
                w.release_syntax_refresh(cx);
                assert!(!w.tabs[2].buffer.highlights.is_empty());
                assert!(w.tabs[0].buffer.highlights.is_empty());
                assert_eq!(w.active, 0);
                assert!(!w.tabs[2].buffer.dirty);
                assert_eq!(std::fs::read(&path).expect("disk unchanged"), bytes);
            }));
        });
    }
}

#[test]
fn initial_syntax_reopen_defers_actual_handler_and_preserves_snapshot() {
    with_sources(|entity, visual| {
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.hold_syntax_refresh(true);
            let mut tab = Tab::empty();
            tab.buffer.rope = ropey::Rope::from_str("fn café() {}\n");
            tab.buffer.language = Some(LanguageId::Rust);
            tab.buffer.encoding = Encoding::Utf16Le;
            tab.buffer.line_ending = LineEnding::Crlf;
            tab.buffer.mark_saved();
            tab.buffer.highlights = crate::syntax::highlight(LanguageId::Rust, &tab.buffer.rope.to_string());
            assert!(!tab.buffer.highlights.is_empty());
            let uuid = tab.session_id;
            let source = tab.buffer.rope.clone();
            let revision = tab.buffer.raw_revision.clone();
            let incarnation = tab.instance.clone();
            w.closed_tabs.push(ClosedTab { buffer: tab.buffer, session_id: uuid });
            crate::buffer::highlight_tests::reset_flatten_calls();
            w.restore_closed_tab(&FileRestoreClosedTab, window, cx);
            assert_eq!(crate::buffer::highlight_tests::flatten_calls(), 0);
            let restored = &w.tabs[w.active];
            assert_eq!(restored.session_id, uuid);
            assert!(!std::sync::Arc::ptr_eq(&restored.instance, &incarnation));
            assert!(restored.buffer.rope.is_instance(&source));
            assert_eq!(restored.buffer.raw_revision, revision);
            assert_eq!(restored.buffer.encoding, Encoding::Utf16Le);
            assert_eq!(restored.buffer.line_ending, LineEnding::Crlf);
            assert!(!restored.buffer.dirty);
            assert!(restored.buffer.highlights.is_empty());
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[w.active].buffer.highlights.is_empty());
            assert!(!w.tabs[w.active].buffer.dirty);
        }));
    });
}

#[test]
fn initial_syntax_rename_defers_actual_inactive_handler() {
    let dir = crate::persist::TempDir::new("initial-rename");
    let path = dir.join("neutral.txt");
    std::fs::write(&path, crate::text::encoding::encode("fn café() {}\r\n", Encoding::Utf8Bom)).expect("fixture");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.hold_syntax_refresh(true);
            let tab = &mut w.tabs[1];
            tab.buffer = crate::io::read_buffer(&path).expect("neutral source");
            let source = tab.buffer.rope.clone();
            let revision = tab.buffer.raw_revision.clone();
            crate::buffer::highlight_tests::reset_flatten_calls();
            w.rename_tab_file(1, "renamed.rs".into(), cx);
            assert_eq!(crate::buffer::highlight_tests::flatten_calls(), 0);
            assert_eq!(w.active, 0);
            let tab = &w.tabs[1];
            assert_eq!(tab.buffer.file_path.as_ref(), Some(&dir.join("renamed.rs")));
            assert_eq!(tab.buffer.language, Some(LanguageId::Rust));
            assert!(tab.buffer.rope.is_instance(&source));
            assert_eq!(tab.buffer.raw_revision, revision);
            assert!(!tab.buffer.dirty);
            assert_eq!(tab.buffer.encoding, Encoding::Utf8Bom);
            assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
            assert!(tab.buffer.highlights.is_empty());
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[1].buffer.highlights.is_empty());
            assert!(w.tabs[0].buffer.highlights.is_empty());
            assert_eq!(w.active, 0);
            assert!(!w.tabs[1].buffer.dirty);
        }));
    });
}

#[test]
fn initial_syntax_running_result_rejects_same_uuid_reopened_incarnation() {
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.hold_syntax_refresh(true);
            w.request_syntax_refresh_for_tab(w.tabs[0].session_id, std::time::Duration::ZERO, cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let old = w.tabs.remove(0);
            let uuid = old.session_id;
            let rope = old.buffer.rope.clone();
            let revision = old.buffer.raw_revision.clone();
            let mut reopened = Tab::from_closed(ClosedTab { buffer: old.buffer, session_id: uuid });
            reopened.buffer.highlights.clear();
            assert!(reopened.buffer.rope.is_instance(&rope));
            assert_eq!(reopened.buffer.raw_revision, revision);
            w.tabs.insert(0, reopened);
            w.release_syntax_refresh(cx);
            assert!(w.tabs[0].buffer.highlights.is_empty(), "old incarnation must not publish");
        }));
    });
}

#[test]
#[ignore = "optimized >10 MiB actual Reopen/Rename handler timing"]
fn initial_syntax_release_handler_sample() {
    for rename in [false, true] {
        for sample in 0..3 {
            let dir = crate::persist::TempDir::new("initial-syntax-timing");
            let path = dir.join("neutral.txt");
            let source = "// neutral filler\n".repeat(800_000) + "fn café() {}\n";
            assert!(source.len() as u64 > crate::io_stream::STREAM_THRESHOLD_BYTES);
            std::fs::write(&path, &source).expect("fixture");
            with_sources(|entity, visual| {
                visual.update(|window, cx| entity.update(cx, |w, cx| {
                    w.hold_syntax_refresh(true);
                    let mut tab = Tab::empty();
                    tab.buffer.rope = ropey::Rope::from_str(&source);
                    tab.buffer.file_path = Some(path.clone());
                    tab.buffer.language = Some(LanguageId::Rust);
                    tab.buffer.mark_saved();
                    if rename { w.tabs.push(tab); }
                    else { w.closed_tabs.push(ClosedTab { buffer: tab.buffer, session_id: tab.session_id }); }
                    let started = std::time::Instant::now();
                    if rename { w.rename_tab_file(2, "renamed.rs".into(), cx); }
                    else { w.restore_closed_tab(&FileRestoreClosedTab, window, cx); }
                    println!("INITIAL_SYNTAX_HANDLER rename={rename} sample={sample} bytes={} elapsed_us={:.3}", source.len(), started.elapsed().as_secs_f64()*1e6);
                }));
                visual.run_until_parked();
                visual.update(|_, cx| entity.update(cx, |w, cx| {
                    w.release_syntax_refresh(cx);
                    assert!(!w.tabs[2].buffer.highlights.is_empty());
                    println!("INITIAL_SYNTAX_PHASE rename={rename} sample={sample} worker_ms={:.3} publication_ms={:.3}", w.syntax_refresh.worker_time.as_secs_f64()*1e3, w.syntax_refresh.install_time.as_secs_f64()*1e3);
                }));
            });
        }
    }
}

#[test]
fn initial_syntax_running_activity_guard_rejects_real_prepared_undo_redo_aba() {
    use crate::buffer::{ContentRevision, Selection, UndoEntry, prepared::{PreparedGroup, PrefixEdit}};
    let _execution = crate::macros::override_execution_for_test(false);
    let held = std::cell::RefCell::new(None);
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let group = std::sync::Arc::new(PreparedGroup::build(
                ropey::Rope::from_str("fn café() {}\n"), ContentRevision::default(),
                Selection::default(), Selection::default(),
                vec![PrefixEdit { range: 0..0, text: "    ".into() }],
                &std::sync::atomic::AtomicBool::new(false)).expect("prepared fixture"));
            let (rope, revision, selection) = group.target(false);
            let tab = &mut w.tabs[0];
            tab.buffer.rope = rope; tab.buffer.raw_revision = revision; tab.selection = selection;
            tab.undo_stack = vec![UndoEntry::Prepared(group)]; tab.buffer.mark_saved();
            w.hold_syntax_refresh(true);
            w.request_syntax_refresh_for_tab(w.tabs[0].session_id, std::time::Duration::ZERO, cx);
        }));
        visual.run_until_parked();
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            held.replace(w.syntax_refresh.held.take());
            w.undo(&crate::actions::EditUndo, window, cx);
        }));
        visual.run_until_parked();
        visual.update(|window, cx| entity.update(cx, |w, cx| w.redo(&crate::actions::EditRedo, window, cx)));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, _| {
            let parsed = held.borrow();
            let (request, _, _) = parsed.as_ref().expect("original running request");
            assert!(w.tabs[0].buffer.rope.is_instance(&request.rope));
            assert_eq!(w.tabs[0].buffer.raw_revision, request.revision);
            // Isolate the content guard from the independently protective latest/cancel guards.
            w.syntax_refresh.latest.insert(request.tab, request.id);
            request.cancel.store(0, std::sync::atomic::Ordering::Relaxed);
            assert!(w.syntax_source_valid(request));
            assert!(!w.syntax_request_valid(request), "running activity cannot rebind after real Undo/Redo");
        }));
    });
}

#[test]
fn initial_syntax_running_noop_caret_and_unrelated_edit_keep_activity_valid() {
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.hold_syntax_refresh(true);
            w.request_syntax_refresh_for_tab(w.tabs[0].session_id, std::time::Duration::ZERO, cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.bulk_edit = true;
            w.apply_edit(0..0, "", cx);
            w.bulk_edit = false;
            w.tabs[0].selection.collapse_to(2);
            w.active = 1;
            w.replace_byte_range(0..0, "// other tab\n", cx);
            w.active = 0;
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
        }));
    });
}

#[test]
fn initial_syntax_open_pristine_replacement_gets_a_fresh_incarnation() {
    let dir = crate::persist::TempDir::new("initial-pristine");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, "fn café() {}\n").expect("fixture");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs[0] = Tab::empty();
            let uuid = w.tabs[0].session_id;
            let instance = w.tabs[0].instance.clone();
            w.hold_syntax_refresh(true);
            crate::buffer::highlight_tests::reset_flatten_calls();
            w.open_path(path.clone(), cx);
            assert_eq!(crate::buffer::highlight_tests::flatten_calls(), 0);
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[0].session_id, uuid);
            assert!(!std::sync::Arc::ptr_eq(&w.tabs[0].instance, &instance));
            assert!(!w.tabs[0].buffer.dirty);
            assert!(w.tabs[0].buffer.highlights.is_empty());
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
            assert!(w.tabs[1].buffer.highlights.is_empty());
        }));
    });
}
