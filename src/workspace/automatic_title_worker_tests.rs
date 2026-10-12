use super::*;

pub(super) fn hold_title(entity: &gpui::Entity<Workspace>, visual: &mut gpui::VisualTestContext) -> futures::channel::oneshot::Sender<()> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    visual.update(|_, cx| entity.update(cx, |w, _| w.automatic_title.gate = Some(receiver)));
    sender
}

#[test]
fn automatic_title_worker_retains_admitted_intermediate_before_final_empty_line() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        let first = format!("{}intermediate", "/".repeat(10000));
        automatic_title_paste(visual, &first);
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        visual.dispatch_action(crate::actions::EditSelectAll); visual.run_until_parked();
        automatic_title_paste(visual, "\n");
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        release.send(()).expect("held worker"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "intermediate");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "\n"); assert_eq!(w.tabs[0].undo_stack.len(), 2);
            assert!(w.automatic_title.worker_main_thread.iter().all(|main| !*main));
            assert!(w.automatic_title.context_turn_bytes.iter().all(|bytes| *bytes <= 16 * 1024));
        });
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "intermediate");
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "intermediate");
    });
}

#[test]
fn automatic_title_worker_latest_input_and_other_tab_collision_effects_are_fifo() {
    with_automatic_title(|entity, visual| {
        let release = hold_title(entity, visual);
        automatic_title_paste(visual, &format!("{}shared", "/".repeat(10000)));
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.replace_byte_range(0..w.buffer().rope.len_bytes(), "different", cx);
            w.set_active_tab(1, cx);
            w.replace_byte_range(0..w.buffer().rope.len_bytes(), "shared", cx);
        }));
        assert_eq!(automatic_title_name(entity, visual), "new 1");
        release.send(()).expect("held worker"); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.tabs[0].buffer.display_short_name().as_ref(), "different");
            assert_eq!(w.tabs[1].buffer.display_short_name().as_ref(), "shared");
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "different"); assert_eq!(w.tabs[1].buffer.rope.to_string(), "shared");
        });
    });
}

#[test]
fn automatic_title_worker_preference_and_complete_context_aba_refuse_stale_names() {
    for preference in [false, true] {
        with_automatic_title(|entity, visual| {
            let release = hold_title(entity, visual);
            automatic_title_paste(visual, &format!("{}delayed", "/".repeat(10000)));
            if preference {
                let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = false;
                { let _off = crate::settings::override_for_test(settings); }
            } else {
                visual.update(|_, cx| entity.update(cx, |w, cx| {
                    let saved = w.tabs[1].buffer.new_label.clone();
                    w.tabs[1].buffer.new_label = Some("changed beyond a bounded prefix".into()); w.automatic_name_context_changed(cx);
                    w.tabs[1].buffer.new_label = saved; w.automatic_name_context_changed(cx);
                }));
            }
            release.send(()).expect("held worker"); visual.run_until_parked();
            assert_eq!(automatic_title_name(entity, visual), "new 1");
            visual.update(|_, cx| assert!(entity.read(cx).tabs[0].buffer.rope.to_string().ends_with("delayed")));
        });
    }
}

#[test]
fn automatic_title_worker_wide_context_is_bounded_and_does_not_copy_full_custom_names() {
    with_automatic_title(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.tabs[1].buffer.new_label = Some(format!("CUSTOM{}", "中".repeat(4 * 1024 * 1024)).into());
            w.tabs[1].buffer.new_label_is_custom = true;
        }));
        crate::buffer::automatic_title_work::reset();
        automatic_title_paste(visual, &format!("{}bounded", "/".repeat(10000)));
        assert_eq!(automatic_title_name(entity, visual), "bounded");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.automatic_title.context_turn_bytes.iter().all(|bytes| *bytes <= 16 * 1024));
        });
        let (_, scans) = crate::buffer::automatic_title_work::snapshot();
        assert!(scans < 30000, "wide custom label must not be scanned: {scans}");
    });
}

#[test]
fn automatic_title_prepared_crlf_inverse_undo_then_redo_uses_cached_direction() {
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str("first\r\nlater");
    buffer.new_label = Some("retained".into());
    let group = crate::buffer::prepared::PreparedGroup::build_line_edits(buffer.rope.clone(), buffer.raw_revision.clone(), Default::default(),
        vec![crate::buffer::prepared::PrefixEdit { range: 6..7, text: String::new() }], false, &std::sync::atomic::AtomicBool::new(false)).expect("prepared deletion of LF");
    assert!(!group.has_title_notifications(false)); assert!(group.has_title_notifications(true));
    buffer.rope = group.expected_rope(true).clone(); buffer.raw_revision = group.expected(true).clone(); buffer.refresh_dirty();
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _settings = crate::settings::override_for_test(settings);
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.tabs[0].undo_stack.push(crate::buffer::UndoEntry::Prepared(std::sync::Arc::new(group)));
        }));
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "first");
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), "first\r\nlater"));
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "first");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), "first\rlater");
            assert!(!w.automatic_titles_pending()); assert!(w.status_message.is_none());
        });
    });
}

#[test]
fn automatic_title_large_registered_indent_below_row_zero_has_no_title_job_or_warning() {
    let before = format!("retained\n{}", "z".repeat(11 * 1024 * 1024));
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(&before); buffer.new_label = Some("retained".into());
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _settings = crate::settings::override_for_test(settings);
        visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].selection.collapse_to(9)));
        crate::buffer::automatic_title_work::reset();
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent); visual.run_until_parked();
        let changed = format!("retained\n    {}", "z".repeat(11 * 1024 * 1024));
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), changed);
            let Some(crate::buffer::UndoEntry::Prepared(group)) = w.tabs[0].undo_stack.last() else { panic!("registered prepared indentation"); };
            assert!(!group.has_title_notifications(false)); assert!(!group.has_title_notifications(true));
            assert!(!w.automatic_titles_pending()); assert!(w.automatic_title.worker_main_thread.is_empty()); assert!(!w.automatic_title.suspended);
        });
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), before));
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), changed);
            assert!(!w.automatic_titles_pending()); assert!(!w.automatic_title.suspended);
            assert!(w.status_message.as_deref().is_none_or(|status| !status.contains("title")));
        });
        assert_eq!(automatic_title_name(entity, visual), "retained");
        assert_eq!(crate::buffer::automatic_title_work::snapshot().1, 0);
    });
}

#[test]
pub(super) fn automatic_title_actual_aggregate_scan_exhaustion_keeps_prepared_history_and_save() {
    let before = format!("{}admitted", "/".repeat(20 * 1024 * 1024));
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(&before); buffer.mark_saved();
    buffer.new_label = Some("retained".into());
    let edits = (1..10).map(|at| crate::buffer::prepared::PrefixEdit { range: at..at + 1, text: "/".to_owned() }).collect();
    let group = crate::buffer::prepared::PreparedGroup::build_line_edits(buffer.rope.clone(), buffer.raw_revision.clone(), Default::default(),
        edits, false, &std::sync::atomic::AtomicBool::new(false)).expect("nine real sealed operations");
    let charge = group.expected_rope(false).len_bytes() + group.expected_rope(true).len_bytes() + group.retained_title_bytes().expect("cached charge");
    assert!(charge < 128 * 1024 * 1024); assert!(group.has_title_notifications(false) && group.has_title_notifications(true));
    buffer.rope = group.expected_rope(true).clone(); buffer.raw_revision = group.expected(true).clone(); buffer.refresh_dirty();
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _settings = crate::settings::override_for_test(settings);
        let disposed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.automatic_title.disposal_observer = Some(disposed.clone());
            w.tabs[0].undo_stack.push(crate::buffer::UndoEntry::Prepared(std::sync::Arc::new(group)));
        }));
        crate::buffer::automatic_title_work::reset();
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        let scans = crate::buffer::automatic_title_work::snapshot().1;
        assert_eq!(scans, crate::automatic_title::history::WORK_LIMIT);
        assert_eq!(automatic_title_name(entity, visual), "retained");
        visual.update(|_, cx| {
            let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), before);
            assert_eq!(w.tabs[0].undo_stack.len(), 0); assert_eq!(w.tabs[0].redo_stack.len(), 1);
            assert!(w.automatic_title.suspended); assert!(!w.automatic_titles_pending()); assert!(w.automatic_title_credits_available());
            assert!(w.status_message.as_deref().is_some_and(|status| status.contains("128 MiB")));
            assert!(!w.automatic_title.worker_main_thread.is_empty());
            assert!(w.automatic_title.worker_main_thread.iter().all(|main| !main));
        });
        let disposed = disposed.lock().expect("context disposal observer").clone();
        assert!(!disposed.is_empty()); assert!(disposed.iter().all(|main| !main));
        let directory = crate::persist::TempDir::new("automatic-title-budget-save"); let path = directory.join("neutral.txt");
        let requests = crate::io::SaveDialogRequestCaptureForTest::new();
        let (reply, selected) = futures::channel::oneshot::channel(); let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
        visual.dispatch_action(FileSave); visual.run_until_parked();
        assert_eq!(requests.requests(), vec![("Save As", "retained".to_owned())]);
        reply.send(Some(path.clone())).expect("save admitted current document"); visual.run_until_parked();
        assert_eq!(std::fs::read(&path).expect("saved current bytes"), before.as_bytes());
        println!("automatic-title-budget charged_source_bytes={charge} actual_scan_bytes={scans} history_ops=9 document_bytes={} context_rows_disposed_on_background={}", before.len(), disposed.len());
    });
}

#[test]
fn automatic_title_mounted_seed_context_receipt_keeps_first_crlf_failure_unclassified() {
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str("first\r\nlater"); buffer.new_label = Some("retained".into());
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _settings = crate::settings::override_for_test(settings);
        let (sender, receiver) = futures::channel::oneshot::channel();
        let context = visual.update(|window, cx| entity.update(cx, |w, _| {
            let source = w.source_input_token(); let input = window.input_context_token();
            let tab = &mut w.tabs[0]; let activity = tab.content_activity.clone(); let raw = tab.buffer.raw_revision.clone(); let rope = tab.buffer.rope.clone();
            let group = crate::buffer::prepared::PreparedGroup::build_line_edits(tab.buffer.rope.clone(), raw.clone(), tab.selection,
                vec![crate::buffer::prepared::PrefixEdit { range: 6..7, text: String::new() }], false, &std::sync::atomic::AtomicBool::new(false)).expect("original mounted fixture");
            tab.buffer.rope = group.expected_rope(true).clone(); tab.buffer.raw_revision = group.expected(true).clone(); tab.buffer.refresh_dirty();
            println!("automatic-title-mounted-seed uuid={} incarnation={} initial_revision_changed={} initial_rope_replaced={} activity_unchanged={} selection={}:{} raw_matches_history={}",
                tab.session_id, std::sync::Arc::as_ptr(&tab.instance) as usize, tab.buffer.raw_revision != raw, !tab.buffer.rope.is_instance(&rope),
                tab.content_activity == activity, tab.selection.anchor, tab.selection.head, tab.buffer.raw_revision == *group.expected(true));
            tab.undo_stack.push(crate::buffer::UndoEntry::Prepared(std::sync::Arc::new(group))); w.prepared_test_gate = Some(receiver);
            (source, input, activity, w.autosave_generation, w.post_edit_generation)
        }));
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx); println!("automatic-title-mounted-held source_epoch_unchanged={} focus_epoch_unchanged={} activity_unchanged={} autosave_unchanged={} post_edit_unchanged={} pending={} status={:?}",
                w.source_input_token() == context.0, window.input_context_token() == context.1, w.tabs[0].content_activity == context.2,
                w.autosave_generation == context.3, w.post_edit_generation == context.4, w.pending_prepared_edit.is_some(), w.status_message);
        });
        sender.send(()).expect("held original fixture worker"); visual.run_until_parked();
        visual.update(|window, cx| {
            let w = entity.read(cx); let tab = &w.tabs[0];
            println!("automatic-title-mounted-terminal rope={:?} name={} undo={} redo={} activity_unchanged={} source_epoch_unchanged={} focus_epoch_unchanged={} status={:?}",
                tab.buffer.rope.to_string(), tab.buffer.display_short_name(), tab.undo_stack.len(), tab.redo_stack.len(), tab.content_activity == context.2,
                w.source_input_token() == context.0, window.input_context_token() == context.1, w.status_message);
            assert_eq!(tab.undo_stack.len() + tab.redo_stack.len(), 1);
            assert!(matches!(tab.buffer.rope.to_string().as_str(), "first\rlater" | "first\r\nlater"));
        });
    });
}
