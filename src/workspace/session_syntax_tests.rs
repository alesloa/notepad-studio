use super::super::syntax_refresh_tests::with_sources;
use crate::{session::{SessionManifest, SessionTab}, syntax::LanguageId};

fn entry(path: &std::path::Path) -> SessionTab {
    let mut entry: SessionTab = toml::from_str(&format!("id = \"{}\"", uuid::Uuid::new_v4())).expect("entry");
    entry.path = Some(path.to_path_buf());
    entry.language = Some(LanguageId::Rust);
    entry
}

#[test]
fn session_syntax_actual_restore_does_not_parse_before_mount() {
    let dir = crate::persist::TempDir::new("session-syntax");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, "fn caf\u{e9}() {}\r\n").expect("fixture");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            crate::buffer::highlight_tests::reset_flatten_calls();
            w.apply_session(SessionManifest { tabs: vec![entry(&path)], ..Default::default() });
            assert_eq!(crate::buffer::highlight_tests::flatten_calls(), 0);
            assert!(w.tabs[0].buffer.highlights.is_empty());
        }));
    });
}

fn drain(visual: &mut gpui::VisualTestContext, entity: &gpui::Entity<super::Workspace>) {
    for _ in 0..400 {
        visual.cx.executor().advance_clock(std::time::Duration::from_millis(2));
        visual.run_until_parked();
        let done = visual.update(|_, cx| {
            let w = entity.read(cx);
            w.syntax_refresh.initial.task.is_none() && (w.syntax_refresh.held.is_some()
                || w.syntax_refresh.latest.is_empty())
        });
        if done { return; }
    }
}

#[test]
fn session_syntax_active_first_and_all_300_tabs_refill() {
    let dir = crate::persist::TempDir::new("session-refill");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("fixture");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: (0..300).map(|_| entry(&path)).collect(), active: 270, ..Default::default() });
            w.hold_syntax_refresh(true);
            w.start_initial_syntax(cx);
            assert_eq!(w.syntax_refresh.running.as_ref().expect("worker").tab, w.tabs[270].session_id);
            assert!(w.tabs.iter().all(|tab| tab.buffer.highlights.is_empty()));
        }));
        drain(visual, entity);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.initial_outstanding(), 4);
            assert_eq!(w.syntax_refresh.latest.len(), 4);
            assert_eq!(w.active, 270);
            w.release_syntax_refresh(cx);
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending && !tab.buffer.highlights.is_empty()));
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
            assert!(w.syntax_refresh.initial.task.is_none());
            assert!(w.syntax_refresh.initial.max_outstanding <= 4);
            assert!(w.syntax_refresh.initial.max_scanned <= 64);
            println!("session pump 300: max callback {:?}, max scanned {}, max outstanding {}", w.syntax_refresh.initial.callback_time, w.syntax_refresh.initial.max_scanned, w.syntax_refresh.initial.max_outstanding);
            assert_eq!(w.active, 270);
            assert_eq!(w.status_message.as_deref(), Some("Restored 300 tab(s)"));
        });
    });
}

#[test]
fn session_syntax_startup_blank_and_policy_skips_preserve_status() {
    let dir = crate::persist::TempDir::new("session-blank");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("fixture");
    with_sources(|entity, visual| {
        let _settings = crate::settings::override_for_test(crate::settings::Settings {
            startup_open_blank: true, large_file_restriction_enabled: true,
            large_file_disable_syntax: true, large_file_threshold_bytes: 1,
            ..Default::default()
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let mut normal = entry(&path); normal.language = None;
            w.apply_session(SessionManifest { tabs: vec![entry(&path), normal], ..Default::default() });
            assert_eq!(w.active, 2);
            assert!(!w.tabs[2].initial_syntax_pending);
            w.start_initial_syntax(cx);
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.active, 2);
            assert_eq!(w.syntax_refresh.started, 0);
            assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending));
            assert_eq!(w.status_message.as_deref(), Some("Restored 2 tab(s)"));
            assert!(w.syntax_refresh.initial.task.is_none());
        });
    });
}

#[test]
fn session_syntax_activation_preempts_initial_but_keeps_regular_debounce() {
    let dir = crate::persist::TempDir::new("session-priority");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("fixture");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: (0..8).map(|_| entry(&path)).collect(), ..Default::default() });
            w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
        }));
        drain(visual, entity);
        visual.dispatch_action(crate::actions::TabGoto2);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.active, 1);
            assert_eq!(w.syntax_refresh.queued.front().expect("promoted").tab, w.tabs[1].session_id);
            assert_ne!(w.syntax_refresh.running.as_ref().expect("retiring").cancel.load(std::sync::atomic::Ordering::Relaxed), 0);
            w.release_syntax_refresh(cx);
            assert_eq!(w.syntax_refresh.running.as_ref().expect("new active").tab, w.tabs[1].session_id);
            w.hold_syntax_refresh(true);
        }));
        visual.run_until_parked();
        visual.simulate_input("// edit\n");
        visual.dispatch_action(crate::actions::TabGoto1);
        visual.dispatch_action(crate::actions::TabGoto2);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let delayed = w.syntax_refresh.delayed.get(&w.tabs[1].session_id).expect("regular debounce retained");
            assert_eq!(delayed.request.origin, super::Origin::Regular);
            w.release_syntax_refresh(cx);
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending && !tab.buffer.highlights.is_empty()));
            assert!(w.initial_outstanding() <= 4);
        });
    });
}

#[test]
fn session_syntax_actual_restore_preserves_encoding_selection_and_savepoint() {
    use crate::text::{encoding::Encoding, line_ending::LineEnding};
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let dir = crate::persist::TempDir::new("session-metadata");
        let path = dir.join("neutral.rs");
        let text = "fn caf\u{e9}() { let value = \"\u{4e2d}\u{1f600}\"; }\r\n";
        let bytes = crate::text::encoding::encode(text, encoding);
        std::fs::write(&path, &bytes).expect("fixture");
        with_sources(|entity, visual| {
            let before = std::cell::RefCell::new(None);
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                let mut tab = entry(&path);
                tab.cursor_anchor = 7; tab.cursor_head = 8; tab.read_only = true; tab.tab_color = Some(3);
                w.apply_session(SessionManifest { tabs: vec![tab], ..Default::default() });
                let tab = &w.tabs[0];
                assert_eq!(tab.buffer.encoding, encoding);
                assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
                assert_eq!(tab.buffer.rope.to_string(), text.replace("\r\n", "\n"));
                assert!(tab.buffer.read_only); assert!(!tab.buffer.dirty);
                assert_eq!(tab.buffer.tab_color, Some(3));
                before.replace(Some((tab.buffer.rope.clone(), tab.buffer.raw_revision.clone(), tab.selection)));
                w.start_initial_syntax(cx);
            }));
            drain(visual, entity);
            visual.update(|_, cx| {
                let w = entity.read(cx); let tab = &w.tabs[0];
                let before = before.borrow(); let (rope, revision, selection) = before.as_ref().expect("snapshot");
                assert!(tab.buffer.rope.is_instance(rope)); assert_eq!(&tab.buffer.raw_revision, revision);
                assert_eq!(tab.selection.anchor, selection.anchor); assert_eq!(tab.selection.head, selection.head);
                assert!(tab.buffer.read_only); assert!(!tab.buffer.dirty);
                assert!(!tab.buffer.highlights.is_empty()); assert!(!tab.initial_syntax_pending);
                assert_eq!(std::fs::read(&path).expect("unchanged"), bytes);
            });
        });
    }
}

#[test]
fn session_syntax_stale_policy_and_same_uuid_new_incarnation_retry() {
    let dir = crate::persist::TempDir::new("session-stale");
    let path = dir.join("neutral.rs");
    std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("fixture");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: vec![entry(&path)], ..Default::default() });
            w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let id = w.tabs[0].session_id;
            let rope = w.tabs[0].buffer.rope.clone();
            let revision = w.tabs[0].buffer.raw_revision.clone();
            let mut fresh = crate::tab::Tab::empty();
            fresh.session_id = id; fresh.buffer.rope = rope; fresh.buffer.raw_revision = revision;
            fresh.buffer.language = Some(LanguageId::Rust); fresh.initial_syntax_pending = true;
            w.tabs[0] = fresh;
            w.release_syntax_refresh(cx);
            assert!(w.tabs[0].buffer.highlights.is_empty());
            assert!(w.tabs[0].initial_syntax_pending);
        }));
        drain(visual, entity);
        visual.update(|_, cx| assert!(!entity.read(cx).tabs[0].initial_syntax_pending));
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs[0].initial_syntax_pending = true; w.seed_initial_syntax(0);
            w.retire_syntax_highlights(w.tabs[0].session_id, cx);
            w.hold_syntax_refresh(true); w.set_active_tab(0, cx);
        }));
        visual.run_until_parked();
        let _policy = crate::settings::override_for_test(crate::settings::Settings {
            large_file_restriction_enabled: true, large_file_disable_syntax: true,
            large_file_threshold_bytes: 1, ..Default::default()
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.release_syntax_refresh(cx);
            assert!(w.tabs[0].buffer.highlights.is_empty());
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx); assert!(!w.tabs[0].initial_syntax_pending);
            assert!(w.tabs[0].buffer.highlights.is_empty());
        });
    });
}

#[test]
fn session_syntax_dirty_history_survives_publication_and_real_undo() {
    use crate::buffer::{ContentRevision, EditOp, UndoEntry, serialize_undo_stack};
    let dir = crate::persist::TempDir::new("session-history");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let mut saved = entry(&dir.join("missing.rs")); saved.dirty = true;
    let source = "fn caf\u{e9}() {}\n";
    std::fs::write(dir.join(&format!("{}.bak", saved.id)), source).expect("backup");
    let history = vec![UndoEntry::Single(EditOp {
        byte_start: 0, removed: "fn prior() {}\n".into(), inserted: source.into(),
        sel_before_anchor: 0, sel_before_head: 0,
        before_revision: ContentRevision::default(), after_revision: ContentRevision::default(),
    })];
    std::fs::write(dir.join(&format!("{}.undo", saved.id)), serialize_undo_stack(&history)).expect("history");
    with_sources(|entity, visual| {
        let before = std::cell::RefCell::new(None);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: vec![saved.clone()], ..Default::default() });
            let tab = &w.tabs[0]; assert!(tab.buffer.dirty); assert_eq!(tab.undo_stack.len(), 1);
            before.replace(Some((tab.buffer.rope.clone(), tab.buffer.raw_revision.clone(), serialize_undo_stack(&tab.undo_stack))));
            w.start_initial_syntax(cx);
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[0];
            let before = before.borrow(); let (rope, revision, history) = before.as_ref().expect("before");
            assert!(tab.buffer.rope.is_instance(rope)); assert_eq!(&tab.buffer.raw_revision, revision);
            assert_eq!(&serialize_undo_stack(&tab.undo_stack), history);
            assert!(tab.buffer.dirty); assert!(!tab.buffer.highlights.is_empty());
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), "fn prior() {}\n");
            assert!(w.tabs[0].buffer.dirty);
        });
    });
}

#[test]
fn session_syntax_noop_caret_and_unrelated_tab_keep_initial_result_valid() {
    let dir = crate::persist::TempDir::new("session-noop");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: vec![entry(&path), entry(&path)], ..Default::default() });
            w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.bulk_edit = true; w.apply_edit(0..0, "", cx); w.bulk_edit = false;
            w.tabs[0].selection.collapse_to(2);
            w.active = 1; w.replace_byte_range(0..0, "// other\n", cx); w.active = 0;
            assert_eq!(w.syntax_refresh.held.as_ref().expect("initial").0.origin, super::Origin::Initial);
            w.release_syntax_refresh(cx);
            assert!(!w.tabs[0].initial_syntax_pending);
            assert!(!w.tabs[0].buffer.highlights.is_empty());
        }));
    });
}
#[test]

fn session_syntax_running_activity_guard_rejects_real_prepared_undo_redo_aba() {

    use crate::buffer::{ContentRevision, Selection, UndoEntry, prepared::{PreparedGroup, PrefixEdit}};

    let _execution = crate::macros::override_execution_for_test(false);

    let held = std::cell::RefCell::new(None);

    with_sources(|entity, visual| {

        visual.update(|_, cx| entity.update(cx, |w, cx| {

            let group = std::sync::Arc::new(PreparedGroup::build(

                ropey::Rope::from_str("fn caf\u{e9}() {}\n"), ContentRevision::default(),

                Selection::default(), Selection::default(),

                vec![PrefixEdit { range: 0..0, text: "    ".into() }],

                &std::sync::atomic::AtomicBool::new(false)).expect("prepared fixture"));

            let (rope, revision, selection) = group.target(false);

            let tab = &mut w.tabs[0];

            tab.buffer.rope = rope; tab.buffer.raw_revision = revision; tab.selection = selection;

            tab.undo_stack = vec![UndoEntry::Prepared(group)]; tab.buffer.mark_saved();

            w.hold_syntax_refresh(true);

            w.tabs[0].initial_syntax_pending = true;

            w.seed_initial_syntax(0); w.start_initial_syntax(cx);

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

            assert_eq!(request.origin, super::Origin::Initial);

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
fn session_syntax_terminal_error_and_zero_spans_terminate_without_losing_recovery_status() {
    let dir = crate::persist::TempDir::new("session-errors");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    let empty = dir.join("empty.rs"); std::fs::write(&empty, "").expect("empty");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: vec![entry(&path), entry(&empty), entry(&dir.join("missing.rs"))], ..Default::default() });
            w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let mut parsed = w.syntax_refresh.held.take().expect("held initial");
            parsed.1 = Err(crate::syntax::HighlightError::DepthLimit);
            w.syntax_refresh.hold = false;
            w.finish_syntax_refresh(parsed, cx);
            assert!(!w.tabs[0].initial_syntax_pending);
            let status = w.status_message.as_deref().expect("status");
            assert!(status.starts_with("Restored 2 tab(s); 1 couldn't be reopened; syntax:"));
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending));
            assert!(w.tabs[1].buffer.highlights.is_empty());
            assert!(w.syntax_refresh.initial.task.is_none());
            assert_eq!(w.syntax_refresh.started, 2);
        });
    });
}

#[test]
fn session_syntax_failed_and_successful_window_close_handle_pump_and_worker() {
    let dir = crate::persist::TempDir::new("session-close");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    for succeeds in [false, true] {
        with_sources(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.apply_session(SessionManifest { tabs: (0..8).map(|_| entry(&path)).collect(), ..Default::default() });
                w.session_final_save_result = Some(succeeds);
                w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
            }));
            drain(visual, entity);
            assert_eq!(visual.simulate_close(), succeeds);
            if succeeds {
                visual.update(|window, _| window.remove_window());
                visual.cx.update(|cx| entity.update(cx, |w, cx| {
                    assert!(w.window_closed);
                    assert!(w.syntax_refresh.initial.task.is_none());
                    assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
                    w.release_syntax_refresh(cx); w.start_initial_syntax(cx);
                    assert!(w.tabs.iter().all(|tab| tab.buffer.highlights.is_empty()));
                }));
            } else {
                visual.update(|_, cx| entity.update(cx, |w, cx| {
                    assert!(!w.window_closed);
                    w.release_syntax_refresh(cx);
                }));
                drain(visual, entity);
                visual.update(|_, cx| {
                    let w = entity.read(cx);
                    assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending));
                    assert!(w.status_message.as_deref().expect("newer error").contains("recovery could not be saved"));
                });
            }
        });
    }
}

#[test]
fn session_syntax_regular_queue_backlog_eventually_refills_without_refusal() {
    let dir = crate::persist::TempDir::new("session-backlog");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: (0..260).map(|_| entry(&path)).collect(), active: 259, ..Default::default() });
            w.hold_syntax_refresh(true);
            for index in 0..256 {
                w.request_syntax_refresh_for_tab(w.tabs[index].session_id, std::time::Duration::ZERO, cx);
            }
            assert_eq!(w.syntax_refresh.latest.len(), 256);
            w.start_initial_syntax(cx);
            assert_eq!(w.initial_outstanding(), 0);
            assert!(w.syntax_refresh.refused.is_none());
            assert_eq!(w.status_message.as_deref(), Some("Restored 260 tab(s)"));
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending && !tab.buffer.highlights.is_empty()));
            assert!(w.syntax_refresh.refused.is_none());
            assert_eq!(w.syntax_pending_for_test(), (0, 0, false));
        });
    });
}

#[test]
fn session_syntax_mru_reorder_close_reopen_and_transfer_activation_keep_markers() {
    let dir = crate::persist::TempDir::new("session-list");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let entries: Vec<_> = (0..8).map(|_| entry(&path)).collect();
            let mru = entries.iter().rev().map(|entry| entry.id).collect();
            w.apply_session(SessionManifest { tabs: entries, mru, ..Default::default() });
            w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
        }));
        drain(visual, entity);
        visual.dispatch_action(crate::actions::TabNext);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert_ne!(w.active, 0);
            let active = w.tabs[w.active].session_id;
            w.move_tab(w.active, 6, cx);
            assert_eq!(w.tabs[w.active].session_id, active);
            w.release_syntax_refresh(cx);
            assert_eq!(w.syntax_refresh.running.as_ref().expect("active initial").tab, active);
            w.hold_syntax_refresh(true);
        }));
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::FileCloseTab);
        visual.dispatch_action(crate::actions::FileRestoreClosedTab);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.release_syntax_refresh(cx);
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx); assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending));
            assert!(w.tabs.iter().all(|tab| !tab.buffer.highlights.is_empty()));
        });
    });
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert_eq!(w.initial_outstanding(), 0);
            let crate::session::RestoreOutcome::Restored(tab) = crate::session::restore_csv_fixture(&entry(&path), dir.path()) else { panic!("restore") };
            assert!(tab.initial_syntax_pending);
            w.adopt_transferred_tab(tab, cx);
            assert_eq!(w.syntax_refresh.running.as_ref().expect("destination activated").tab, w.tabs[w.active].session_id);
        }));
        drain(visual, entity);
        visual.update(|_, cx| {
            let w = entity.read(cx); assert!(!w.tabs[w.active].initial_syntax_pending);
            assert!(!w.tabs[w.active].buffer.highlights.is_empty());
        });
    });
}

#[test]
fn session_syntax_loading_and_language_changes_never_publish_stale_result() {
    let dir = crate::persist::TempDir::new("session-context");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    for loading in [false, true] {
        with_sources(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.apply_session(SessionManifest { tabs: vec![entry(&path)], ..Default::default() });
                w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
            }));
            visual.run_until_parked();
            visual.update(|window, cx| entity.update(cx, |w, cx| {
                if loading {
                    w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress {
                        bytes_read: 0, total_bytes: 100,
                        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    });
                } else { w.language_normal_text(&crate::actions::LanguageNormalText, window, cx); }
                w.set_status("newer operation"); w.release_syntax_refresh(cx);
                assert!(w.tabs[0].buffer.highlights.is_empty());
            }));
            drain(visual, entity);
            visual.update(|_, cx| entity.update(cx, |w, cx| {
                assert!(w.tabs[0].buffer.highlights.is_empty());
                assert_eq!(w.status_message.as_deref(), Some("newer operation"));
                if loading {
                    assert!(w.tabs[0].initial_syntax_pending);
                    w.tabs[0].buffer.loading = None; w.set_active_tab(0, cx);
                } else { assert!(!w.tabs[0].initial_syntax_pending); }
            }));
            if loading {
                drain(visual, entity);
                visual.update(|_, cx| assert!(!entity.read(cx).tabs[0].initial_syntax_pending));
            }
        });
    }
}

#[test]
fn session_syntax_completed_pump_sleeps_through_later_regular_parse() {
    let dir = crate::persist::TempDir::new("session-idle");
    let path = dir.join("neutral.rs"); std::fs::write(&path, "fn caf\u{e9}() {}\n").expect("source");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: vec![entry(&path)], ..Default::default() });
            w.start_initial_syntax(cx);
        }));
        drain(visual, entity);
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            assert!(!w.tabs[0].initial_syntax_pending);
            assert!(w.syntax_refresh.initial.task.is_none());
            w.hold_syntax_refresh(true);
            w.request_syntax_refresh_for_tab(w.tabs[0].session_id, std::time::Duration::ZERO, cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.release_syntax_refresh(cx);
            assert!(w.syntax_refresh.initial.task.is_none(), "completed startup must not resume scanning on a later regular parse");
        }));
    });
}
