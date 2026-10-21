use super::*;
use super::automatic_title_worker_tests::hold_title;

fn clipboard(visual: &mut VisualTestContext) -> String {
    visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default())
}

pub(super) fn source_callback(visual: &mut VisualTestContext, text: &str) {
    let mut handler = visual.update(|window, cx| {
        window.refresh(); let _ = window.draw(cx);
        window.take_registered_input_handler().expect("painted Source handler")
    });
    visual.update(|window, cx| {
        assert!(handler.accepts_text_input(window, cx)); handler.replace_text_in_range(None, text, window, cx);
    }); visual.run_until_parked();
}

#[test]
fn automatic_title_held_save_consumers_wait_for_original_tab_suggestion() {
    for mode in 0..3 {
        with_automatic_title(|entity, visual| {
            let release = hold_title(entity, visual);
            automatic_title_paste(visual, &format!("{}suggested café中", "/".repeat(10000)));
            let requests = crate::io::SaveDialogRequestCaptureForTest::new();
            let (reply, selected) = futures::channel::oneshot::channel();
            let _copy_choice = crate::io::SaveCopyOptionsForTest::new(Some(false));
            let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
            match mode { 0 => visual.dispatch_action(FileSave), 1 => visual.dispatch_action(crate::actions::FileSaveAs),
                _ => visual.dispatch_action(crate::actions::FileSaveCopyAs) }
            visual.run_until_parked(); assert!(requests.requests().is_empty());
            visual.update(|_, cx| entity.update(cx, |w, cx| w.set_active_tab(1, cx)));
            release.send(()).expect("held title worker"); visual.run_until_parked();
            assert_eq!(requests.requests().len(), 1);
            assert_eq!(requests.requests()[0].1, "suggested café中");
            visual.update(|_, cx| assert_eq!(entity.read(cx).active, 1));
            reply.send(None).expect("cancel controlled picker"); visual.run_until_parked();
            visual.update(|_, cx| {
                let tab = &entity.read(cx).tabs[0]; assert!(tab.buffer.file_path.is_none());
                assert!(tab.buffer.rope.to_string().ends_with("suggested café中")); assert_eq!(tab.undo_stack.len(), 1);
            });
        });
    }
}

#[test]
fn automatic_title_cancelled_metadata_keeps_save_available_with_accepted_name() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}delayed", "/".repeat(10000)));
        let requests = crate::io::SaveDialogRequestCaptureForTest::new();
        let (reply, selected) = futures::channel::oneshot::channel();
        let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
        visual.dispatch_action(FileSave); visual.run_until_parked(); assert!(requests.requests().is_empty());
        visual.update(|_, cx| entity.update(cx, |w, _| w.cancel_automatic_title_worker_for_test()));
        release.send(()).expect("held title worker"); visual.run_until_parked();
        assert_eq!(requests.requests(), vec![("Save As", "new 1".to_owned())]);
        reply.send(None).expect("controlled picker"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert!(w.tabs[0].buffer.rope.to_string().ends_with("delayed"));
            assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(!w.automatic_titles_pending());
        });
    });
}

#[test]
fn automatic_title_deferred_clipboard_name_cannot_overwrite_newer_copy() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}delayed", "/".repeat(10000)));
        visual.dispatch_action(crate::actions::EditCopyFilename); visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditCopyDirPath); visual.run_until_parked();
        assert_eq!(clipboard(visual), "");
        release.send(()).expect("held title worker"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "delayed"); assert_eq!(clipboard(visual), "");
        visual.dispatch_action(crate::actions::EditCopyFilename); visual.run_until_parked();
        assert_eq!(clipboard(visual), "delayed");
    });
}

#[test]
fn automatic_title_deferred_clipboard_refuses_edit_then_undo_aba() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        let source = format!("{}delayed", "/".repeat(10000)); automatic_title_paste(visual, &source);
        visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".to_owned())));
        visual.dispatch_action(crate::actions::EditCopyFilename); visual.run_until_parked();
        visual.simulate_input("x"); visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        release.send(()).expect("held title worker"); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), source));
        assert_eq!(clipboard(visual), "sentinel"); assert_eq!(automatic_title_name(entity, visual), "delayed");
    });
}

#[test]
fn automatic_title_context_and_name_payload_dispose_on_background_after_window_removal() {
    with_automatic_title(|entity, visual| {
        let observer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.automatic_title.disposal_observer = Some(observer.clone());
            w.tabs[1].buffer.new_label = Some(format!("custom{}", "中".repeat(1024 * 1024)).into());
            w.tabs[1].buffer.new_label_is_custom = true;
        }));
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}delayed", "/".repeat(10000)));
        visual.update(|window, _| window.remove_window()); visual.run_until_parked();
        release.send(()).expect("worker retained through window removal"); visual.run_until_parked();
        let drops = observer.lock().expect("observer").clone(); assert!(!drops.is_empty());
        assert!(drops.iter().all(|main| !main), "captured rows must dispose on the GPUI background executor: {drops:?}");
    });
}

#[test]
fn automatic_title_admission_exhaustion_never_refuses_grouped_text_history() {
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.begin_edit_group();
            for index in 0..4097 { let end = w.buffer().rope.len_bytes(); w.replace_byte_range(0..end, &format!("v{index}"), cx); }
            w.end_edit_group(cx);
            assert_eq!(w.buffer().rope.to_string(), "v4096"); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.automatic_title.suspended); assert!(!w.automatic_titles_pending());
            assert!(w.automatic_title.admission_turn_bytes.iter().all(|bytes| *bytes <= 32 * 1024));
        })); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        visual.update(|_, cx| assert!(entity.read(cx).automatic_title_credits_available()));
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), ""));
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "v4096"));
    });
}

#[test]
fn automatic_title_cross_name_cycle_publishes_atomically_with_actual_move_cost() {
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            for index in 2..70 { let mut tab = Tab::empty(); tab.buffer.language = None;
                tab.buffer.new_label = Some(format!("old{index}").into()); w.tabs.push(tab); }
            w.tabs[0].buffer.new_label = Some("old0".into()); w.tabs[1].buffer.new_label = Some("old1".into());
        }));
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}temporary0", "/".repeat(10000)));
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            for index in 1..70 { w.active = index; w.replace_byte_range(0..0, &format!("temporary{index}"), cx); }
            for index in 0..70 { w.active = index; let end = w.buffer().rope.len_bytes();
                w.replace_byte_range(0..end, &format!("old{}", (index + 1) % 70), cx); }
        }));
        release.send(()).expect("held title worker"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            for index in 0..70 { assert_eq!(w.tabs[index].buffer.display_short_name().as_ref(), format!("old{}", (index + 1) % 70)); }
            assert_eq!(w.automatic_title.commits.len(), 1); assert_eq!(w.automatic_title.commits[0].0, 70);
            assert!(w.automatic_title.context_turn_bytes.iter().all(|bytes| *bytes <= 16 * 1024));
            println!("automatic-title-atomic shared_string_bytes={} optional_label_bytes={} changed={} logical_move_bytes={} commit_ns={}",
                std::mem::size_of::<gpui::SharedString>(), std::mem::size_of::<Option<gpui::SharedString>>(),
                w.automatic_title.commits[0].0, w.automatic_title.commits[0].1, w.automatic_title.commits[0].2.as_nanos());
        });
    });
}

#[test]
fn automatic_title_saved_mixed_macro_waits_for_names_then_keeps_one_undo() {
    let source = format!("{}body", "/".repeat(10000));
    let mut recorded = None;
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        visual.dispatch_action(crate::actions::MacroStartRecording); visual.run_until_parked();
        source_callback(visual, &source);
        visual.dispatch_action(crate::actions::EditCopyAllFilenames); visual.run_until_parked();
        release.send(()).expect("held record worker"); visual.run_until_parked();
        assert_eq!(clipboard(visual), "body\r\nnew\r\n");
        visual.dispatch_action(crate::actions::EditPaste); visual.run_until_parked();
        visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "bodybody");
        recorded = Some(crate::macros::current());
    });
    let events = recorded.expect("recorded"); assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], crate::macros::MacroEvent::InsertText(text) if text == &source));
    assert!(matches!(events[1], crate::macros::MacroEvent::CopyAllFilenames));
    let directory = crate::persist::TempDir::new("automatic-title-macro"); let path = directory.join("macros.toml");
    let saved = std::collections::BTreeMap::from([("title macro".to_owned(), events)]);
    let loaded = crate::macros::saved_round_trip_for_test(&path, &saved).expect("save/reload recorded macro");
    with_automatic_title(|entity, visual| {
        let _playback = crate::macros::override_playback_for_test(Vec::new(), loaded);
        let release = hold_title(entity, visual);
        visual.dispatch_action(crate::actions::MacroRunSaved { name: "title macro".to_owned() }); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), source);
            assert!(w.title_clipboard_pending()); assert!(w.regex_replacement.macro_playback.is_some());
        });
        release.send(()).expect("held replay worker"); visual.run_until_parked();
        let expected = format!("{source}body\nnew\n");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), expected);
            assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(!w.title_clipboard_pending());
            assert!(w.regex_replacement.macro_playback.is_none());
        });
        assert_eq!(automatic_title_name(entity, visual), "bodybody");
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), ""));
        assert_eq!(automatic_title_name(entity, visual), "body");
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), expected));
        assert_eq!(automatic_title_name(entity, visual), "bodybody"); assert!(crate::macros::current().is_empty());
    });
}

#[test]
fn automatic_title_character_input_macro_receipt_preserves_capacity_failure_context() {
    let source = format!("{}body", "/".repeat(10000));
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        visual.dispatch_action(crate::actions::MacroStartRecording); visual.run_until_parked();
        visual.simulate_input(&source); visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditCopyAllFilenames); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            println!("automatic-title-character-macro bytes={} pending={} suspended={} name={} undo={} redo={} status={:?}",
                w.buffer().rope.len_bytes(), w.automatic_titles_pending(), w.automatic_title.suspended,
                w.buffer().display_short_name(), w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len(), w.status_message);
            assert_eq!(w.buffer().rope.to_string(), source);
        });
        let held = release.send(()); println!("automatic-title-character-macro held_receiver_alive={}", held.is_ok());
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
        let events = crate::macros::current();
        println!("automatic-title-character-macro recorded_events={} clipboard={:?}", events.len(), clipboard(visual));
        assert!(matches!(&events[0], crate::macros::MacroEvent::InsertText(text) if text == &source));
        assert!(matches!(events[1], crate::macros::MacroEvent::CopyAllFilenames));
    });
}

#[test]
fn automatic_title_wide_installed_inputs_bound_latest_reservation_and_notice_work() {
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            for index in 2..512 { let mut tab = Tab::empty(); tab.buffer.language = None;
                tab.buffer.new_label = Some(format!("old{index}").into()); w.tabs.push(tab); }
        }));
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}first", "/".repeat(10000)));
        for index in 1..512 {
            visual.update(|_, cx| entity.update(cx, |w, cx| w.set_active_tab(index, cx)));
            let text = format!("name{index}");
            source_callback(visual, &text);
        }
        let maximum = visual.update(|_, cx| {
            let w = entity.read(cx);
            for (old, new, moved, bytes) in &w.automatic_title.latest_reserve_work {
                println!("automatic-title-latest old_capacity={old} new_capacity={new} existing_entries_moved={moved} logical_reservation_write_bytes={bytes}");
            }
            assert!(w.automatic_title.admission_turn_bytes.iter().all(|bytes| *bytes <= 32 * 1024));
            assert!(w.automatic_titles_pending()); assert!(!w.automatic_title.suspended);
            for index in 1..512 { assert_eq!(w.tabs[index].buffer.rope.to_string(), format!("name{index}")); assert_eq!(w.tabs[index].undo_stack.len(), 1); }
            w.automatic_title.latest_reserve_work.iter().map(|entry| entry.3).max().expect("observed reserve")
        });
        release.send(()).expect("admitted worker"); visual.run_until_parked();
        assert!(maximum <= 32 * 1024, "Latest reservation exceeds admission accounting: {maximum} bytes");
    });
}

#[test]
fn automatic_title_registered_escape_preserves_text_history_and_recovery_fallback() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        let source = format!("{}delayed", "/".repeat(10000)); automatic_title_paste(visual, &source);
        let snapshot = visual.update(|_, cx| entity.read(cx).session_snapshot(crate::session::BakPolicy::Full));
        assert_eq!(snapshot.tabs[0].new_label.as_deref(), Some("new 1"));
        visual.simulate_keystrokes("escape"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), source);
            assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(!w.automatic_titles_pending());
        });
        release.send(()).expect("worker retained until cancellation returns"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        let directory = crate::persist::TempDir::new("automatic-title-held-recovery"); let path = directory.join("session.toml");
        crate::session::write_csv_fixture(snapshot, &path, directory.path()).expect("actual isolated recovery Writer");
        let saved: crate::session::SessionManifest = toml::from_str(&std::fs::read_to_string(&path).expect("manifest")).expect("persisted schema");
        let crate::session::RestoreOutcome::Restored(tab) = crate::session::restore_csv_fixture(&saved.tabs[0], directory.path()) else { panic!("recovery"); };
        assert_eq!(tab.buffer.rope.to_string(), source); assert_eq!(tab.buffer.new_label.as_ref().map(|label| label.as_ref()), Some("new 1"));
    });
}

#[test]
fn automatic_title_aggregate_shared_wide_context_refuses_metadata_then_retries_after_disposal() {
    with_automatic_title(|entity, visual| {
        let wide: gpui::SharedString = format!("custom{}", "中".repeat(4 * 1024 * 1024)).into();
        let observer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.automatic_title.disposal_observer = Some(observer.clone());
            for index in 1..13 {
                let mut tab = Tab::empty(); tab.buffer.language = None; tab.buffer.new_label = Some(wide.clone()); tab.buffer.new_label_is_custom = true;
                if index == 1 { w.tabs[1] = tab; } else { w.tabs.push(tab); }
            }
        }));
        let source = format!("{}admitted", "/".repeat(10000)); automatic_title_paste(visual, &source);
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), source); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.automatic_title.suspended); assert!(!w.automatic_titles_pending()); assert!(w.automatic_title_credits_available());
            assert!(w.automatic_title.context_turn_bytes.iter().all(|bytes| *bytes <= 16 * 1024));
        });
        let drops = observer.lock().expect("observer").clone(); assert!(!drops.is_empty()); assert!(drops.iter().all(|main| !main));
        let requests = crate::io::SaveDialogRequestCaptureForTest::new();
        let (reply, selected) = futures::channel::oneshot::channel(); let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
        visual.dispatch_action(FileSave); visual.run_until_parked(); assert_eq!(requests.requests(), vec![("Save As", "new 1".to_owned())]);
        reply.send(None).expect("cancel controlled picker"); visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, cx| { w.tabs.truncate(2); w.automatic_name_context_changed(cx); }));
        visual.simulate_input("x"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "admittedx");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), format!("{source}x")); assert_eq!(w.tabs[0].undo_stack.len(), 2);
            assert!(!w.automatic_title.suspended); assert!(w.automatic_title_credits_available());
        });
        println!("automatic-title-context logical_label_occurrences=12 bytes_per_occurrence={} shared_arc=true cap_bytes={}", wide.len(), 128 * 1024 * 1024);
    });
}

#[test]
fn automatic_title_cancel_keeps_context_credit_until_background_source_disposal() {
    with_automatic_title(|entity, visual| {
        let observer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.automatic_title.disposal_observer = Some(observer.clone());
            w.tabs[1].buffer.new_label = Some(format!("custom{}", "中".repeat(4 * 1024 * 1024)).into()); w.tabs[1].buffer.new_label_is_custom = true;
        }));
        let release = hold_title(entity, visual);
        let source = format!("{}admitted", "/".repeat(10000)); automatic_title_paste(visual, &source);
        visual.simulate_keystrokes("escape"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert!(!w.automatic_titles_pending()); assert!(!w.automatic_title_credits_available());
            assert_eq!(w.buffer().rope.to_string(), source); assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
        visual.simulate_input("x"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        release.send(()).expect("held cancelled context"); visual.run_until_parked();
        visual.update(|_, cx| assert!(entity.read(cx).automatic_title_credits_available()));
        let drops = observer.lock().expect("observer").clone(); assert!(!drops.is_empty()); assert!(drops.iter().all(|main| !main));
        visual.simulate_input("y"); visual.run_until_parked(); assert_eq!(automatic_title_name(entity, visual), "admittedxy");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), format!("{source}xy")); assert!(w.automatic_title_credits_available());
        });
    });
}

#[test]
fn automatic_title_yielded_validation_rechecks_input_and_complete_context_aba() {
    for mode in ["below-row-zero", "input-undo-aba", "preference-aba", "name-aba"] {
        with_automatic_title(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, _| {
                for index in 2..70 { let mut tab = Tab::empty(); tab.buffer.language = None;
                    tab.buffer.new_label = Some(format!("old{index}").into()); w.tabs.push(tab); }
            }));
            let worker = hold_title(entity, visual);
            let (release, gate) = futures::channel::oneshot::channel();
            visual.update(|_, cx| entity.update(cx, |w, _| w.automatic_title.validation_gate = Some(gate)));
            let source = format!("{}stable\nsecond", "/".repeat(10000)); automatic_title_paste(visual, &source);
            worker.send(()).expect("worker"); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx); assert!(w.automatic_title.commits.is_empty());
                assert_eq!(w.automatic_title.validation_offsets.len(), 1);
                assert!((1..70).contains(&w.automatic_title.validation_offsets[0]));
            });
            match mode {
                "below-row-zero" | "input-undo-aba" => {
                    source_callback(visual, " tail");
                    if mode == "input-undo-aba" { visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked(); }
                },
                "preference-aba" => {
                    let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = false;
                    let disabled = crate::settings::override_for_test(settings); drop(disabled);
                },
                _ => visual.update(|_, cx| entity.update(cx, |w, cx| {
                    let old = w.tabs[1].buffer.new_label.clone(); w.tabs[1].buffer.new_label = Some("temporary collision context".into());
                    w.automatic_name_context_changed(cx); w.tabs[1].buffer.new_label = old; w.automatic_name_context_changed(cx);
                })),
            }
            release.send(()).expect("validation held after earlier rows"); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx);
                assert_eq!(w.buffer().rope.to_string(), if mode == "below-row-zero" { format!("{source} tail") } else { source.clone() });
                assert!(!w.automatic_titles_pending()); assert!(w.automatic_title_credits_available());
                if mode == "below-row-zero" || mode == "input-undo-aba" {
                    assert_eq!(w.buffer().display_short_name().as_ref(), "stable"); assert!(!w.automatic_title.suspended);
                    assert_eq!(w.automatic_title.worker_main_thread.len(), 1);
                    assert!(w.automatic_title.validation_offsets.len() >= 3);
                    assert_eq!(w.automatic_title.validation_offsets[0], w.automatic_title.validation_offsets[1]);
                } else { assert_eq!(w.buffer().display_short_name().as_ref(), "new 1"); assert!(w.automatic_title.commits.is_empty()); }
            });
        });
    }
}

#[test]
fn automatic_title_absent_latest_slot_checks_captured_revision_and_activity() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        let source = format!("{}admitted", "/".repeat(10000)); automatic_title_paste(visual, &source);
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.tabs[1].buffer.raw_revision = Default::default(); w.tabs[1].observe_content_activity();
        }));
        release.send(()).expect("worker"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), source); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert_eq!(w.buffer().display_short_name().as_ref(), "new 1"); assert!(w.automatic_title.commits.is_empty());
            assert!(w.status_message.as_deref().is_some_and(|status| status.contains("document changed")));
            assert!(w.automatic_title_credits_available());
        });
    });
}

#[test]
fn automatic_title_command_and_waiter_capacity_keeps_actual_save_available() {
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            for _ in 2..256 { let mut tab = Tab::empty(); tab.buffer.language = None; w.tabs.push(tab); }
        }));
        let release = hold_title(entity, visual);
        let source = format!("{}admitted", "/".repeat(10000)); automatic_title_paste(visual, &source);
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            for index in 0..256 {
                let completed = completed.clone();
                assert!(w.defer_automatic_title_tab(index, "capacity-control", window, cx, move |_, _, _, _| {
                    completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }));
            }
            assert_eq!(w.automatic_title_command_count(), 256);
        }));
        let directory = crate::persist::TempDir::new("automatic-title-command-save"); let path = directory.join("neutral.txt");
        let requests = crate::io::SaveDialogRequestCaptureForTest::new();
        let (reply, selected) = futures::channel::oneshot::channel(); let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
        visual.dispatch_action(FileSave); visual.run_until_parked();
        assert_eq!(requests.requests(), vec![("Save As", "new 1".to_owned())]);
        assert_eq!(completed.load(std::sync::atomic::Ordering::SeqCst), 256);
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.automatic_title_command_count(), 0);
            assert!(w.automatic_title.admission_turn_bytes.iter().all(|bytes| *bytes <= 32 * 1024));
            println!("automatic-title-command maximum_joint_notice_latest_waiter_request_bytes={}", w.automatic_title.admission_turn_bytes.iter().max().expect("observed work"));
        });
        reply.send(Some(path.clone())).expect("Save current bytes"); visual.run_until_parked();
        release.send(()).expect("cancelled held worker"); visual.run_until_parked();
        assert_eq!(std::fs::read(&path).expect("saved exact bytes"), source.as_bytes());
        visual.update(|_, cx| { let w = entity.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.automatic_title_credits_available()); assert!(!w.buffer().dirty); });
    });
}

#[test]
fn automatic_title_maximum_atomic_publication_measures_actual_label_moves() {
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            for index in 2..4096 { let mut tab = Tab::empty(); tab.buffer.language = None;
                tab.buffer.new_label = Some(format!("old{index}").into()); w.tabs.push(tab); }
        }));
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}first", "/".repeat(10000)));
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            for index in 1..4096 { w.active = index; w.replace_byte_range(0..0, &format!("name{index}"), cx); }
        }));
        release.send(()).expect("maximum admitted job"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.display_short_name().as_ref(), "first");
            for index in 1..4096 { assert_eq!(w.tabs[index].buffer.display_short_name().as_ref(), format!("name{index}")); }
            assert_eq!(w.automatic_title.commits.len(), 1); assert_eq!(w.automatic_title.commits[0].0, 4096);
            assert_eq!(w.automatic_title.commits[0].1, 589824);
            assert!(w.automatic_title.admission_turn_bytes.iter().all(|bytes| *bytes <= 32 * 1024));
            assert!(w.automatic_title.context_turn_bytes.iter().all(|bytes| *bytes <= 16 * 1024));
            println!("automatic-title-maximum changed={} logical_label_move_bytes={} label_assignment_ns={}",
                w.automatic_title.commits[0].0, w.automatic_title.commits[0].1, w.automatic_title.commits[0].2.as_nanos());
        });
    });
}

#[test]
fn automatic_title_lost_entity_releases_owned_rows_sources_and_credit_on_background() {
    let _settings = crate::settings::override_for_test(Settings { toolbar_hidden: true, folder_panel_visible: false,
        tab_exit_on_last_close: false, untitled_uses_first_line: true, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _macros = crate::macros::override_execution_for_test(false); let _clipboard = crate::clipboard_ownership::override_for_test();
    let mut app = TestAppContext::single(); app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let observer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut first = Tab::empty(); first.buffer.language = None; first.buffer.new_label = Some("new 1".into());
        let mut second = Tab::empty(); second.buffer.language = None;
        second.buffer.new_label = Some(format!("custom{}", "中".repeat(4 * 1024 * 1024)).into()); second.buffer.new_label_is_custom = true;
        w.tabs = vec![first, second]; w.active = 0; w.automatic_title.disposal_observer = Some(observer.clone());
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true); w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx); }); visual.run_until_parked();
    let check_credit = visual.update(|_, cx| entity.read(cx).automatic_title_credit_checker());
    let weak = entity.downgrade(); let release = hold_title(&entity, visual);
    automatic_title_paste(visual, &format!("{}admitted", "/".repeat(10000))); assert!(!check_credit());
    visual.update(|window, _| { drop(window.take_registered_input_handler()); window.remove_window(); });
    visual.run_until_parked(); drop(entity); visual.run_until_parked();
    assert!(weak.upgrade().is_none(), "fixture must actually lose the Workspace");
    let _ = release.send(()); visual.run_until_parked(); assert!(check_credit());
    let drops = observer.lock().expect("observer").clone(); assert!(!drops.is_empty()); assert!(drops.iter().all(|main| !main));
    app.quit();
}

#[test]
fn automatic_title_close_waits_then_restores_accepted_name_and_exact_clean_document() {
    let source = format!("{}body", "/".repeat(10000));
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(&source); buffer.mark_saved(); buffer.new_label = Some("new 1".into());
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _settings = crate::settings::override_for_test(settings);
        visual.dispatch_action(crate::actions::MoveDocEnd); visual.run_until_parked();
        let release = hold_title(entity, visual); source_callback(visual, "x");
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        let original = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; assert!(!tab.buffer.dirty); (tab.session_id, tab.instance.clone()) });
        visual.dispatch_action(FileCloseTab); visual.run_until_parked();
        visual.update(|_, cx| assert!(entity.read(cx).tabs.iter().any(|tab| tab.session_id == original.0)));
        release.send(()).expect("close name barrier"); visual.run_until_parked();
        visual.update(|_, cx| assert!(!entity.read(cx).tabs.iter().any(|tab| tab.session_id == original.0)));
        visual.dispatch_action(FileRestoreClosedTab); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[w.active]; assert_eq!(tab.session_id, original.0);
            assert!(!std::sync::Arc::ptr_eq(&tab.instance, &original.1)); assert_eq!(tab.buffer.rope.to_string(), source);
            assert_eq!(tab.buffer.new_label.as_ref().map(|label| label.as_ref()), Some("body")); assert!(!tab.buffer.new_label_is_custom);
            assert!(!tab.buffer.dirty); assert!(tab.undo_stack.is_empty()); assert!(tab.redo_stack.is_empty());
        });
    });
}

#[test]
fn automatic_title_actual_cross_window_transfer_waits_for_accepted_name() {
    for move_it in [true, false] {
        with_automatic_title(|entity, visual| {
            visual.cx.update(crate::app::startup::tests::isolate_fresh_windows_for_test);
            let release = hold_title(entity, visual);
            let source = format!("{}transferred", "/".repeat(10000)); automatic_title_paste(visual, &source);
            let original = visual.update(|window, cx| entity.update(cx, |w, cx| {
                let original = w.tabs[0].session_id;
                crate::ui::tab_context_menu::transfer_for_startup_test(w, 0, move_it, window, cx);
                assert_eq!(w.tabs[0].session_id, original); original
            }));
            visual.run_until_parked(); assert_eq!(visual.cx.update(|cx| cx.windows().len()), 1);
            release.send(()).expect("transfer name barrier"); visual.run_until_parked();
            visual.cx.update(|cx| {
                let handles = cx.windows(); assert_eq!(handles.len(), 2);
                let mut matches = Vec::new();
                for handle in handles {
                    if let Some(handle) = handle.downcast::<Workspace>() { handle.update(cx, |w, _, _| {
                        for tab in &w.tabs { if tab.buffer.rope == source.as_str() { matches.push((tab.session_id, tab.buffer.new_label.clone(), tab.buffer.dirty, tab.buffer.new_label_is_custom)); } }
                    }).expect("live window"); }
                }
                assert_eq!(matches.len(), if move_it { 1 } else { 2 });
                for (_, label, dirty, custom) in &matches { assert_eq!(label.as_ref().map(|label| label.as_ref()), Some("transferred")); assert!(*dirty); assert!(!*custom); }
                assert!(matches.iter().any(|item| item.0 == original));
                if !move_it { assert!(matches.iter().any(|item| item.0 != original)); }
            });
        });
    }
}

#[test]
fn automatic_title_cancelled_ime_keeps_intermediate_name_and_authoritative_history_identity() {
    with_automatic_title(|entity, visual| {
        source_callback(visual, "prior"); visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        let before = visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0]; assert!(!tab.buffer.dirty); assert_eq!(tab.redo_stack.len(), 1);
            (tab.buffer.raw_revision.clone(), tab.selection.anchor, tab.selection.head)
        });
        let release = hold_title(entity, visual);
        let source = format!("{}intermediate", "/".repeat(10000));
        let mut handler = visual.update(|window, cx| {
            window.refresh(); let _ = window.draw(cx); window.take_registered_input_handler().expect("painted Source handler")
        });
        visual.update(|window, cx| {
            assert!(handler.accepts_text_input(window, cx));
            handler.replace_and_mark_text_in_range(None, &source, Some(source.len()..source.len()), window, cx);
        }); visual.run_until_parked();
        visual.update(|window, cx| {
            handler.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx); handler.unmark_text(window, cx);
        }); visual.run_until_parked();
        release.send(()).expect("held IME name notifications"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[0]; assert_eq!(tab.buffer.rope.to_string(), "");
            assert_eq!(tab.buffer.raw_revision, before.0); assert_eq!((tab.selection.anchor, tab.selection.head), (before.1, before.2));
            assert!(!tab.buffer.dirty); assert_eq!(tab.undo_stack.len(), 0); assert_eq!(tab.redo_stack.len(), 1);
            assert!(!w.automatic_titles_pending()); assert!(w.automatic_title_credits_available());
        });
        assert_eq!(automatic_title_name(entity, visual), "intermediate");
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).buffer().rope.to_string(), "prior"));
        assert_eq!(automatic_title_name(entity, visual), "prior");
    });
}

#[test]
fn automatic_title_window_close_preflight_waits_or_uses_current_recovery_fallback() {
    for cancel_metadata in [false, true] {
        with_automatic_title(|entity, visual| {
            let (snapshots, received) = std::sync::mpsc::channel();
            visual.update(|window, cx| {
                entity.update(cx, |w, _| w.session_snapshot_sink = Some(snapshots));
                let owner = entity.clone();
                window.on_window_should_close(cx, move |_, cx| owner.update(cx, crate::app::quit::save_before_window_close));
            });
            let release = hold_title(entity, visual);
            let source = format!("{}admitted", "/".repeat(10000)); automatic_title_paste(visual, &source);
            assert!(!visual.simulate_close(), "admitted names must settle before final close capture");
            if cancel_metadata { visual.update(|_, cx| entity.update(cx, |w, cx| w.cancel_automatic_titles_for_recovery(cx))); }
            else { release.send(()).expect("held close naming"); }
            visual.run_until_parked();
            let saved = received.try_iter().last().expect("actual central final Full capture");
            assert_eq!(saved.tabs[0].new_label.as_deref(), Some(if cancel_metadata { "new 1" } else { "admitted" }));
            let directory = crate::persist::TempDir::new("automatic-title-final-recovery"); let path = directory.join("session.toml");
            crate::session::write_csv_fixture(saved, &path, directory.path()).expect("isolated Writer");
            let manifest: crate::session::SessionManifest = toml::from_str(&std::fs::read_to_string(&path).expect("manifest")).expect("schema");
            let crate::session::RestoreOutcome::Restored(tab) = crate::session::restore_csv_fixture(&manifest.tabs[0], directory.path()) else { panic!("restored current text"); };
            assert_eq!(tab.buffer.rope.to_string(), source);
            visual.cx.update(|cx| { let w = entity.read(cx); assert!(w.window_closed); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
        });
    }
}

#[test]
fn automatic_title_other_tab_csv_undo_keeps_authoritative_stamps_and_exact_saved_view() {
    let directory = crate::persist::TempDir::new("automatic-title-csv"); let path = directory.join("neutral.csv");
    let disk = b"\xef\xbb\xbflabel,value\r\ncaf\xc3\xa9,1\r\n\xe4\xb8\xad\xf0\x9f\x98\x80,2\r\n";
    std::fs::write(&path, disk).expect("neutral CSV");
    let saved = crate::io::read_buffer(&path).expect("actual saved CSV");
    let original = saved.rope.to_string(); let revision = saved.raw_revision.clone();
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs[1] = Tab::with_buffer(saved); w.tabs[1].buffer.language = None;
            w.tabs[1].view_mode = crate::tab::TabViewMode::CsvPreview; w.automatic_name_context_changed(cx);
        }));
        let release = hold_title(entity, visual);
        let source = format!("{}admitted", "/".repeat(10000)); automatic_title_paste(visual, &source);
        visual.update(|window, cx| entity.update(cx, |w, cx| {
            w.set_active_tab(1, cx); crate::ui::csv_doc::actions::ensure_model(w, window, cx);
            let grid = w.csv_models.get(&w.tabs[1].session_id).expect("saved grid").grid_focus.clone().expect("grid focus");
            window.focus(&grid, cx);
        })); visual.run_until_parked();
        visual.simulate_input("中"); visual.simulate_keystrokes("enter"); visual.run_until_parked();
        visual.update(|window, cx| {
            let tab = &entity.read(cx).tabs[1]; assert!(tab.buffer.dirty); assert_ne!(tab.buffer.raw_revision, revision);
            assert_eq!(tab.view_mode, crate::tab::TabViewMode::CsvPreview);
            assert!(entity.read(cx).input_focus(window) == crate::tab::InputFocus::CsvGrid);
        });
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[1]; assert_eq!(tab.buffer.rope.to_string(), original);
            assert_eq!(tab.buffer.raw_revision, revision); assert!(!tab.buffer.dirty);
            assert_eq!((tab.buffer.encoding, tab.buffer.line_ending), (crate::text::encoding::Encoding::Utf8Bom, crate::text::line_ending::LineEnding::Crlf));
        });
        assert_eq!(std::fs::read(&path).expect("saved bytes"), disk);
        release.send(()).expect("held other-tab naming"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), source); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert_eq!(w.active, 1); assert!(!w.automatic_titles_pending()); assert!(w.automatic_title_credits_available());
        });
        assert_eq!(automatic_title_name(entity, visual), "admitted");
    });
}
