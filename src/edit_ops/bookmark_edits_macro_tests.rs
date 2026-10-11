use super::*;
use crate::{actions::*, edit_ops::bookmark_clipboard::tests::fixture, settings::Settings};
use gpui::{Action, ClipboardItem, TestAppContext, VisualContext};

#[test]
fn bookmark_edits_record_save_reload_and_fresh_editor_replay_execute_every_handler() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let dir = crate::persist::TempDir::new("bookmark-edit-macros");
    let path = dir.join("macros.toml");
    let mut saved = BTreeMap::new();
    for (name, action) in [
        ("cut", CutBookmarkedLines.boxed_clone()),
        ("paste", PasteToBookmarkedLines.boxed_clone()),
        ("remove", RemoveBookmarkedLines.boxed_clone()),
        ("keep", RemoveNonBookmarkedLines.boxed_clone()),
        ("inverse", InverseBookmarks.boxed_clone()),
    ] {
        let _recorder = override_execution_for_test(true);
        let mut cx = TestAppContext::single();
        let (_, visual) = fixture(&mut cx, "a\nkeep\n中", &[0, 7]);
        visual.write_to_clipboard(ClipboardItem::new_string("P\r\nQ".into()));
        visual.update(|window, cx| window.dispatch_action(action, cx)); visual.run_until_parked();
        assert!(!stop_recording()); let events = current(); assert_eq!(events.len(), 1);
        saved.insert(name.to_owned(), events); cx.quit();
    }
    persist_at(&path, &saved).unwrap(); let loaded = load_saved_at(&path); assert_eq!(loaded.len(), 5);
    assert!(!dir.join("macros.toml.corrupt").exists());
    let _execution = override_execution_for_test(true);
    let _playback = override_playback_for_test(Vec::new(), loaded);
    for (name, expected) in [
        ("cut", "fresh\n"), ("paste", "P\r\nQ\nfresh\nP\r\nQ"),
        ("remove", "fresh\n"), ("keep", "é😀\n新"), ("inverse", "é😀\nfresh\n新"),
    ] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "é😀\nfresh\n新", &[0, "é😀\nfresh\n".len()]);
        visual.write_to_clipboard(ClipboardItem::new_string("P\r\nQ".into()));
        visual.dispatch_action(MacroRunSaved { name: name.into() }); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected, "{name}");
            assert!(w.pending_prepared_edit.is_none() && w.tabs[0].edit_transaction.is_none());
            assert_eq!(w.tabs[0].undo_stack.len(), usize::from(name != "inverse"));
        });
        cx.quit();
    }
    assert!(!stop_recording()); assert!(current().is_empty(), "Replay must not record itself");
}

#[test]
fn bookmark_edits_mixed_legacy_and_prepared_macro_waits_and_has_one_undo() {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _execution = override_execution_for_test(false);
    let events = vec![MacroEvent::InsertText("x".into()), MacroEvent::PasteToBookmarkedLines, MacroEvent::InsertText("!".into())];
    let _playback = override_playback_for_test(events, BTreeMap::new());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "a\nb\nc", &[2]);
    let (send, receive) = futures::channel::oneshot::channel();
    visual.update_window_entity(&workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(0); w.prepared_test_gate = Some(receive); });
    visual.write_to_clipboard(ClipboardItem::new_string("P".into()));
    visual.dispatch_action(MacroPlayback); visual.run_until_parked();
    visual.update(|_, cx| {
        let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "xa\nb\nc");
        assert!(w.pending_prepared_edit.is_some() && w.tabs[0].edit_transaction.is_none());
    });
    send.send(()).unwrap(); visual.run_until_parked();
    visual.update(|_, cx| {
        let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "x!a\nP\nc");
        assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.pending_prepared_edit.is_none());
    });
    visual.dispatch_action(EditUndo); visual.run_until_parked();
    visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "a\nb\nc"); assert_eq!((w.selection().anchor, w.selection().head), (0, 0)); });
    visual.dispatch_action(EditRedo); visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "x!a\nP\nc"));
    cx.quit();
}

#[test]
fn bookmark_edits_macro_cancel_during_worker_keeps_prefix_history_and_stops_tail() {
    let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
    let _playback = override_playback_for_test(vec![MacroEvent::InsertText("x".into()), MacroEvent::CutBookmarkedLines, MacroEvent::InsertText("!".into())], BTreeMap::new());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
    let (send, receive) = futures::channel::oneshot::channel();
    visual.update_window_entity(&workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(0); w.prepared_test_gate = Some(receive); });
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
    visual.dispatch_action(EditFindClose); visual.run_until_parked(); send.send(()).unwrap(); visual.run_until_parked();
    visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "xa\nb"); assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.pending_prepared_edit.is_none()); });
    assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel")); cx.quit();
}

#[test]
fn bookmark_edits_macro_large_single_capture_yields_and_cancels_without_losing_history() {
    for cancel in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
        let prefix = "é😀".repeat(70_000);
        let _playback = override_playback_for_test(vec![MacroEvent::InsertText(prefix.clone()), MacroEvent::PasteToBookmarkedLines], BTreeMap::new());
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(&workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(0); w.macro_capture_test_gate = Some(receive); });
        visual.write_to_clipboard(ClipboardItem::new_string("P".into())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_some());
            assert_eq!(w.macro_capture_turns.len(), 1); assert_eq!(w.macro_capture_turns[0].0, 64 * 1024 - 2);
            assert_eq!(w.tabs[0].undo_stack.len(), 2); assert_eq!(w.buffer().rope.to_string(), format!("{prefix}a\nP"));
        });
        if cancel { visual.dispatch_action(EditFindClose); visual.run_until_parked(); }
        send.send(()).unwrap(); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_none());
            assert_eq!(w.tabs[0].undo_stack.len(), if cancel { 2 } else { 1 });
            assert!(w.macro_capture_turns.iter().all(|&(bytes, ops)| bytes <= 64 * 1024 && ops <= 256));
            if !cancel { assert!(w.macro_capture_turns.len() > 1); }
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        if cancel { visual.dispatch_action(EditUndo); visual.run_until_parked(); }
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nb"));
        cx.quit();
    }
}

#[test]
fn bookmark_edits_macro_large_group_capture_yields_revalidates_and_bounds_operation_turns() {
    for stale in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
        let prefix = "é😀".repeat(70_000);
        let mut events = vec![MacroEvent::InsertText(prefix.clone())];
        events.extend((0..600).map(|_| MacroEvent::InsertText("x".into())));
        events.push(MacroEvent::PasteToBookmarkedLines);
        let _playback = override_playback_for_test(events, BTreeMap::new());
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(&workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(0); w.macro_capture_test_gate = Some(receive); });
        visual.write_to_clipboard(ClipboardItem::new_string("P".into())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.macro_capture_turns, [(64 * 1024 - 2, 0)]);
            assert_eq!(w.tabs[0].undo_stack.len(), 2);
            assert!(matches!(&w.tabs[0].undo_stack[0], crate::buffer::UndoEntry::Group { ops, .. } if ops.len() == 601));
        });
        if stale { visual.update_window_entity(&workspace, |w, _, _| w.active_tab_mut().buffer.bookmarks.clear()); }
        send.send(()).unwrap(); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_none());
            assert_eq!(w.tabs[0].undo_stack.len(), if stale { 2 } else { 1 });
            assert!(w.macro_capture_turns.iter().all(|&(bytes, ops)| bytes <= 64 * 1024 && ops <= 256));
            if !stale { assert!(w.macro_capture_turns.iter().any(|&(_, ops)| ops == 256)); }
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        if stale { visual.dispatch_action(EditUndo); visual.run_until_parked(); }
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nb"));
        cx.quit();
    }
}

#[test]
fn bookmark_edits_macro_prepared_undo_redo_events_execute_and_preserve_history() {
    let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
    let _playback = override_playback_for_test(vec![MacroEvent::PasteToBookmarkedLines, MacroEvent::Undo, MacroEvent::Redo,
        MacroEvent::InsertText("!".into())], BTreeMap::new());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
    visual.update_window_entity(&workspace, |w, _, _| w.active_tab_mut().selection.collapse_to(0));
    visual.write_to_clipboard(ClipboardItem::new_string("P".into())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
    visual.update(|_, cx| {
        let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "!a\nP");
        assert!(w.pending_prepared_edit.is_none()); assert_eq!(w.tabs[0].undo_stack.len(), 2);
        assert_eq!(w.status_message.as_deref(), Some("Played macro (4 events)"));
    });
    visual.dispatch_action(EditUndo); visual.run_until_parked(); visual.dispatch_action(EditUndo); visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nb")); cx.quit();
}

fn assert_history_entry_preserved(left: &crate::buffer::UndoEntry, right: &crate::buffer::UndoEntry) {
    use crate::buffer::UndoEntry;
    fn op(left: &crate::buffer::EditOp, right: &crate::buffer::EditOp) {
        assert_eq!(left.before_revision, right.before_revision); assert_eq!(left.after_revision, right.after_revision);
        assert_eq!(left.byte_start, right.byte_start); assert_eq!(left.removed, right.removed); assert_eq!(left.inserted, right.inserted);
        assert_eq!((left.sel_before_anchor, left.sel_before_head), (right.sel_before_anchor, right.sel_before_head));
    }
    match (left, right) {
        (UndoEntry::Single(left), UndoEntry::Single(right)) => op(left, right),
        (UndoEntry::Prepared(left), UndoEntry::Prepared(right)) => assert!(std::sync::Arc::ptr_eq(left, right)),
        (UndoEntry::Group { ops: left, sel_before_anchor: la, sel_before_head: lh },
            UndoEntry::Group { ops: right, sel_before_anchor: ra, sel_before_head: rh }) => {
            assert_eq!((la, lh), (ra, rh)); assert_eq!(left.len(), right.len());
            for (left, right) in left.iter().zip(right) { op(left, right); }
        }
        _ => panic!("History entry changed its representation"),
    }
}

#[test]
fn bookmark_edits_macro_full_history_capture_keeps_large_mixed_prefix_and_publishes_one_suffix() {
    for outcome in ["complete", "cancel", "stale", "budget"] {
        let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
        let _playback = override_playback_for_test(vec![MacroEvent::InsertText("!".into()), MacroEvent::PasteToBookmarkedLines], BTreeMap::new());
        let large = "é😀\n".repeat(70_000);
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
        visual.update_window_entity(&workspace, |w, _, cx| { w.active_tab_mut().selection.collapse_to(0); w.replace_byte_range(0..0, &large, cx); });
        visual.write_to_clipboard(ClipboardItem::new_string("first".into())); visual.dispatch_action(PasteToBookmarkedLines); visual.run_until_parked();
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.begin_edit_group(); w.replace_byte_range(0..0, "[", cx); w.replace_byte_range(0..0, "]", cx); w.end_edit_group(cx);
            w.active_tab_mut().selection.collapse_to(0);
        });
        let (before, prefix) = visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 3);
            (w.buffer().rope.to_string(), w.tabs[0].undo_stack.clone())
        });
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(&workspace, |w, _, _| {
            w.macro_capture_test_gate = Some(receive);
            if outcome == "budget" { w.macro_capture_test_budget = Some(128 * 1024 * 1024); }
        });
        visual.write_to_clipboard(ClipboardItem::new_string("P".into())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
        if outcome != "budget" {
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.macro_capture_turns.len(), 1);
                assert!(w.macro_capture_turns[0].0 > 60_000); assert_eq!(w.tabs[0].undo_stack.len(), 5);
            });
            if outcome == "cancel" { visual.dispatch_action(EditFindClose); visual.run_until_parked(); }
            if outcome == "stale" { visual.update_window_entity(&workspace, |w, _, _| w.active_tab_mut().buffer.bookmarks.clear()); }
            send.send(()).unwrap(); visual.run_until_parked();
        }
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_none()); assert_eq!(w.tabs[0].undo_stack.len(), if outcome == "complete" { 4 } else { 5 });
            for (old, new) in prefix.iter().zip(&w.tabs[0].undo_stack) { assert_history_entry_preserved(old, new); }
            assert!(w.tabs[0].redo_stack.is_empty()); assert!(w.macro_capture_turns.iter().all(|&(bytes, ops)| bytes <= 64 * 1024 && ops <= 256));
            if outcome == "budget" { assert!(w.status_message.as_deref().is_some_and(|status| status.contains("full Undo timeline exceeds"))); }
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        if outcome != "complete" { visual.dispatch_action(EditUndo); visual.run_until_parked(); }
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), before));
        for _ in 0..3 { visual.dispatch_action(EditUndo); visual.run_until_parked(); }
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nb")); cx.quit();
    }
}

#[test]
fn bookmark_edits_metadata_and_empty_paste_macros_keep_existing_undo_and_redo() {
    for read_only in [false, true] {
        let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
        let events = if read_only { vec![MacroEvent::InverseBookmarks, MacroEvent::InverseBookmarks] } else { vec![MacroEvent::PasteToBookmarkedLines] };
        let _playback = override_playback_for_test(events, BTreeMap::new());
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\n", &[2]);
        visual.update_window_entity(&workspace, |w, _, cx| { w.replace_byte_range(0..0, "x", cx); w.replace_byte_range(0..0, "y", cx); });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        let (undo, redo) = visual.update_window_entity(&workspace, |w, _, _| {
            w.active_tab_mut().buffer.read_only = read_only;
            (w.tabs[0].undo_stack.clone(), w.tabs[0].redo_stack.clone())
        });
        visual.write_to_clipboard(ClipboardItem::new_string(String::new())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "xa\n"); assert_eq!(w.buffer().bookmarks, [3].into_iter().collect());
            assert_eq!((w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len()), (undo.len(), redo.len()));
            for (old, new) in undo.iter().zip(&w.tabs[0].undo_stack).chain(redo.iter().zip(&w.tabs[0].redo_stack)) { assert_history_entry_preserved(old, new); }
            assert!(w.macro_capture_turns.is_empty() && w.pending_prepared_edit.is_none());
        }); cx.quit();
    }
}

#[test]
fn bookmark_edits_saved_large_copy_waits_before_paste_and_combines_real_tail_history() {
    let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
    let dir = crate::persist::TempDir::new("bookmark-large-copy-macro"); let path = dir.join("macros.toml");
    let saved = BTreeMap::from([("copy-and-paste".into(), vec![MacroEvent::CopyBookmarkedLines, MacroEvent::PasteToBookmarkedLines,
        MacroEvent::InsertText("!".into())])]);
    persist_at(&path, &saved).unwrap(); let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
    let body = "é😀".repeat(50_000); let source = format!("{body}\nkeep");
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, &source, &[0]);
    let (send, receive) = futures::channel::oneshot::channel();
    visual.update_window_entity(&workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(0); w.bookmark_copy_test_gate = Some(receive); });
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    visual.dispatch_action(MacroRunSaved { name: "copy-and-paste".into() }); visual.run_until_parked();
    visual.update(|_, cx| { let w = workspace.read(cx); assert!(w.pending_bookmark_copy.is_some()); assert_eq!(w.buffer().rope.to_string(), source); });
    assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
    send.send(()).unwrap(); visual.run_until_parked();
    assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).unwrap(), format!("{body}\n"));
    visual.update(|_, cx| {
        let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), format!("!{body}\n\nkeep"));
        assert!(w.pending_bookmark_copy.is_none() && w.pending_prepared_edit.is_none()); assert_eq!(w.tabs[0].undo_stack.len(), 1);
    });
    visual.dispatch_action(EditUndo); visual.run_until_parked(); visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
    cx.quit();
}

#[test]
fn bookmark_edits_saved_inverse_only_macro_runs_readonly_and_mixed_macro_refuses_atomically() {
    let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
    let dir = crate::persist::TempDir::new("readonly-bookmark-macro"); let path = dir.join("macros.toml");
    persist_at(&path, &BTreeMap::from([
        ("inverse".into(), vec![MacroEvent::InverseBookmarks]),
        ("mixed".into(), vec![MacroEvent::InverseBookmarks, MacroEvent::InsertText("x".into())]),
    ])).unwrap();
    let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
    for name in ["inverse", "mixed"] {
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "é😀\r\n中\r\n", &[0]);
        visual.update_window_entity(&workspace, |w, _, _| w.active_tab_mut().buffer.read_only = true);
        visual.dispatch_action(MacroRunSaved { name: name.into() }); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "é😀\r\n中\r\n");
            assert_eq!(w.buffer().bookmarks, if name == "inverse" { [8, 13].into_iter().collect() } else { [0].into_iter().collect() });
            assert!(w.tabs[0].undo_stack.is_empty() && w.pending_prepared_edit.is_none());
            assert_eq!(w.status_message.as_deref(), Some(if name == "inverse" { "Played macro (1 events)" } else { "Buffer is read-only" }));
        }); cx.quit();
    }
}

#[test]
fn bookmark_edits_paste_without_clipboard_text_still_records_the_command() {
    let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(true);
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
    visual.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::empty()));
    visual.dispatch_action(PasteToBookmarkedLines); visual.run_until_parked(); stop_recording();
    assert!(matches!(current().as_slice(), [MacroEvent::PasteToBookmarkedLines]));
    visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "a\nb"); assert!(w.tabs[0].undo_stack.is_empty()); });
    visual.write_to_clipboard(ClipboardItem::new_string("P".into())); visual.dispatch_action(MacroPlayback); visual.run_until_parked();
    visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "a\nP"); assert_eq!(w.tabs[0].undo_stack.len(), 1); }); cx.quit();
}

#[test]
fn bookmark_edits_inverse_manual_and_saved_macro_preserve_rectangles_and_readonly_history() {
    let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
    let dir = crate::persist::TempDir::new("rectangle-inverse-macro"); let path = dir.join("macros.toml");
    persist_at(&path, &BTreeMap::from([("inverse".into(), vec![MacroEvent::InverseBookmarks])])).unwrap();
    let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
    for (read_only, saved) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "é😀\r\n中\r\n", &[0]);
        visual.update_window_entity(&workspace, |w, _, cx| { w.replace_byte_range(0..0, "x", cx); w.replace_byte_range(0..0, "y", cx); });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        let (text, selection, block, undo, redo) = visual.update_window_entity(&workspace, |w, _, _| {
            let tab = w.active_tab_mut(); tab.buffer.read_only = read_only;
            tab.selection = crate::buffer::Selection { anchor: 14, head: 1, is_dragging: true };
            tab.block_selection = Some(crate::buffer::BlockSelection { anchor_line: 2, head_line: 0, anchor_col: 2, head_col: 7, is_dragging: true });
            (tab.buffer.rope.to_string(), tab.selection, tab.block_selection.unwrap(), tab.undo_stack.clone(), tab.redo_stack.clone())
        });
        if saved { visual.dispatch_action(MacroRunSaved { name: "inverse".into() }); } else { visual.dispatch_action(InverseBookmarks); }
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); let tab = &w.tabs[0]; assert_eq!(w.buffer().rope.to_string(), text); assert_eq!(w.buffer().bookmarks, [9, 14].into_iter().collect());
            assert_eq!((tab.selection.anchor, tab.selection.head, tab.selection.is_dragging), (selection.anchor, selection.head, selection.is_dragging));
            let after = tab.block_selection.unwrap();
            assert_eq!((after.anchor_line, after.head_line, after.anchor_col, after.head_col, after.is_dragging),
                (block.anchor_line, block.head_line, block.anchor_col, block.head_col, block.is_dragging));
            assert_eq!((tab.undo_stack.len(), tab.redo_stack.len()), (undo.len(), redo.len()));
            for (old, new) in undo.iter().zip(&tab.undo_stack).chain(redo.iter().zip(&tab.redo_stack)) { assert_history_entry_preserved(old, new); }
            assert!(w.pending_prepared_edit.is_none());
        }); cx.quit();
    }
}

#[test]
#[ignore = "optimized large mixed macro capture phase evidence"]
fn bookmark_edits_release_large_macro_single_and_group_capture_turns() {
    for (name, grouped, existing_prefix) in [("single", false, false), ("group", true, false), ("existing-prefix", false, true)] {
        let _settings = crate::settings::override_for_test(Settings::default()); let _execution = override_execution_for_test(false);
        let prefix = "é😀\n".repeat(1_500_000); assert!(prefix.len() > 10 * 1024 * 1024);
        let mut events = vec![MacroEvent::InsertText(if existing_prefix { "!".into() } else { prefix.clone() })];
        if grouped { events.extend((0..600).map(|_| MacroEvent::InsertText("x".into()))); }
        events.push(MacroEvent::PasteToBookmarkedLines); let _playback = override_playback_for_test(events, BTreeMap::new());
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
        visual.update_window_entity(&workspace, |w, _, cx| {
            if existing_prefix { w.replace_byte_range(0..0, &prefix, cx); }
            w.active_tab_mut().selection.collapse_to(0);
        });
        visual.write_to_clipboard(ClipboardItem::new_string("P".into()));
        let started = std::time::Instant::now(); visual.dispatch_action(MacroPlayback); let dispatch = started.elapsed(); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1 + usize::from(existing_prefix)); assert!(w.pending_prepared_edit.is_none());
            assert!(w.macro_capture_turns.len() > 100);
            assert!(w.macro_capture_turns.iter().all(|&(bytes, ops)| bytes <= 64 * 1024 && ops <= 256));
            let maximum = w.macro_capture_timings.iter().copied().max().unwrap();
            println!("bookmark macro {name} payload={} dispatch_legacy_insert={:.3}ms total={:.3}ms capture_turns={} max_capture={:.3}ms",
                prefix.len(), dispatch.as_secs_f64()*1000., started.elapsed().as_secs_f64()*1000., w.macro_capture_turns.len(), maximum.as_secs_f64()*1000.);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        if existing_prefix { visual.dispatch_action(EditUndo); visual.run_until_parked(); }
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nb")); cx.quit();
    }
}
