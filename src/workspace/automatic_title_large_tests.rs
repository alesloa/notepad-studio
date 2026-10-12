use super::*;
use super::{automatic_title_consumer_tests::source_callback, automatic_title_worker_tests::hold_title};
use std::time::Instant;

#[test]
#[ignore = "manual optimized test-harness control; Root quiet admission required"]
fn automatic_title_release_long_prefix_and_consumer_controls() {
    for cancelled in [false, true] {
        with_automatic_title(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.tabs[0].buffer.encoding = crate::text::encoding::Encoding::Utf8Bom;
                w.tabs[0].buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
            }));
            let source = format!("{}café中😀\nsecond", "/".repeat(12 * 1024 * 1024));
            assert_eq!(source.len(), 12_582_931);
            let release = hold_title(entity, visual);
            crate::buffer::automatic_title_work::reset();
            let started = Instant::now(); source_callback(visual, &source); let input = started.elapsed();
            let foreground_scan = crate::buffer::automatic_title_work::snapshot().1;
            assert_eq!(foreground_scan, crate::automatic_title::FOREGROUND_BYTES);
            assert_eq!(automatic_title_name(entity, visual), "new 1");
            visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into())));
            visual.dispatch_action(crate::actions::EditCopyFilename); visual.run_until_parked();
            assert_eq!(visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())), Some("sentinel".into()));
            let started = Instant::now();
            if cancelled { visual.simulate_keystrokes("escape"); visual.run_until_parked(); }
            release.send(()).expect("held naming source"); visual.run_until_parked(); let settle = started.elapsed();
            let accepted = if cancelled { "new 1" } else { "café中😀" };
            assert_eq!(automatic_title_name(entity, visual), accepted);
            assert_eq!(visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())), Some(accepted.into()));
            let total_scan = crate::buffer::automatic_title_work::snapshot().1;
            visual.update(|_, cx| {
                let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), source);
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert_eq!(w.selection().head, source.len());
                assert_eq!(w.selection().anchor, source.len()); assert!(w.tabs[0].block_selection.is_none());
                assert!(!w.buffer().new_label_is_custom); assert!(!w.automatic_titles_pending());
                assert!(w.automatic_title_credits_available()); assert_eq!(w.automatic_title.suspended, cancelled);
                assert!(!w.automatic_title.worker_main_thread.is_empty());
                assert!(w.automatic_title.worker_main_thread.iter().all(|main| !main));
                assert!(w.automatic_title.context_turn_bytes.iter().all(|bytes| *bytes <= 16 * 1024));
                assert!(w.automatic_title.admission_turn_bytes.iter().all(|bytes| *bytes <= 32 * 1024));
            });
            let getter_work = visual.update(|_, cx| {
                crate::buffer::automatic_title_work::reset(); let buffer = &entity.read(cx).tabs[0].buffer;
                for _ in 0..2 { assert_eq!(buffer.display_short_name().as_ref(), accepted); assert_eq!(buffer.title_bar_label(), accepted); }
                crate::buffer::automatic_title_work::snapshot()
            });
            assert_eq!(getter_work, (2 * accepted.len(), 0));
            let expected = if cancelled {
                source_callback(visual, "!"); format!("{source}!")
            } else { source.clone() };
            let directory = crate::persist::TempDir::new("automatic-title-large-save"); let path = directory.join("neutral.txt");
            let requests = crate::io::SaveDialogRequestCaptureForTest::new();
            let (reply, selected) = futures::channel::oneshot::channel(); let _dialog = crate::io::SaveAsDialogOverrideForTest::new(selected);
            let started = Instant::now(); visual.dispatch_action(FileSave); visual.run_until_parked();
            assert_eq!(requests.requests(), vec![("Save As", accepted.to_owned())]);
            reply.send(Some(path.clone())).expect("save current exact bytes"); visual.run_until_parked(); let save = started.elapsed();
            let mut disk = vec![0xef, 0xbb, 0xbf]; disk.extend_from_slice(expected.replace('\n', "\r\n").as_bytes());
            assert_eq!(std::fs::read(&path).expect("saved bytes"), disk);
            let revision = visual.update(|_, cx| {
                let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert!(!w.buffer().dirty);
                assert!(w.buffer().new_label.is_none()); assert_eq!(w.tabs[0].undo_stack.len(), 1);
                assert_eq!((w.buffer().encoding, w.buffer().line_ending), (crate::text::encoding::Encoding::Utf8Bom, crate::text::line_ending::LineEnding::Crlf));
                w.buffer().raw_revision.clone()
            });
            visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let w = entity.read(cx); assert_eq!(w.buffer().rope.len_bytes(), 0); assert!(w.buffer().dirty);
                assert_eq!(w.tabs[0].undo_stack.len(), 0); assert_eq!(w.tabs[0].redo_stack.len(), 1); });
            visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
            visual.update(|_, cx| { let w = entity.read(cx); assert_eq!(w.buffer().rope.to_string(), expected);
                assert_eq!(w.buffer().raw_revision, revision); assert!(!w.buffer().dirty); });
            assert_eq!(std::fs::read(&path).expect("disk after history"), disk);
            println!("automatic-title-large mode={} source_bytes={} foreground_scan_bytes={} total_scan_bytes={} four_getter_copied_bytes={} four_getter_scan_bytes={} input_capture_ns={} naming_consumer_settle_ns={} inherited_ec8_save_ns={}",
                if cancelled { "cancel-and-continue" } else { "accepted" }, source.len(), foreground_scan, total_scan, getter_work.0, getter_work.1,
                input.as_nanos(), settle.as_nanos(), save.as_nanos());
        });
    }
}

#[test]
#[ignore = "manual optimized test-harness control; Root quiet admission required"]
fn automatic_title_release_prepared_scan_limit_and_save_control() {
    let started = Instant::now();
    super::automatic_title_worker_tests::automatic_title_actual_aggregate_scan_exhaustion_keeps_prepared_history_and_save();
    println!("automatic-title-prepared complete_fixture_undo_naming_limit_save_ns={}", started.elapsed().as_nanos());
}
