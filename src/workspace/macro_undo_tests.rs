// PowerEditor/src/NppCommands.cpp:83-149 — playback brackets document Undo actions.
use super::macro_context_tests::with_workspace;
use crate::{
    actions::{EditRedo, EditUndo},
    buffer::{BlockSelection, UndoEntry},
    macros::{MacroEvent, MacroRecorder},
    text::{encoding::Encoding, line_ending::LineEnding},
};

#[test]
fn playback_repetitions_restore_source_selection_and_savepoint_in_one_undo() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        with_workspace(true, |w, window, cx| {
            let before = w.buffer().rope.to_string();
            let caret = w.selection().head;
            w.active_tab_mut().buffer.encoding = encoding;
            w.active_tab_mut().buffer.line_ending = LineEnding::Crlf;
            w.active_tab_mut().buffer.mark_saved();
            let bytes = crate::text::encoding::encode(&before, encoding);
            assert!(w.replay_macro(&[
                MacroEvent::InsertText("é".into()),
                MacroEvent::InsertText("中🙂".into()),
            ], 3, window, cx));
            let after = w.buffer().rope.to_string();
            let after_caret = w.selection().head;
            assert_eq!(after, format!("{before}{}", "é中🙂".repeat(3)));
            assert_eq!(w.tabs[0].undo_stack.len(), 1, "one whole playback");
            assert!(w.buffer().dirty);
            assert!(w.tabs[0].edit_transaction.is_none());
            w.undo(&EditUndo, window, cx);
            assert_eq!(w.buffer().rope.to_string(), before);
            assert_eq!(w.selection().head, caret);
            assert!(w.selection().is_empty());
            assert!(!w.buffer().dirty);
            assert_eq!(crate::text::encoding::encode(&w.buffer().rope.to_string(), encoding), bytes);
            assert_eq!(w.buffer().encoding, encoding);
            assert_eq!(w.buffer().line_ending, LineEnding::Crlf);
            w.redo(&EditRedo, window, cx);
            assert_eq!(w.buffer().rope.to_string(), after);
            assert_eq!(w.selection().head, after_caret);
            assert!(w.buffer().dirty);
        });
    }
}

#[test]
fn one_edit_after_macro_navigation_restores_the_playback_start_caret() {
    with_workspace(true, |w, window, cx| {
        let before = w.buffer().rope.to_string();
        let caret = w.selection().head;
        assert!(w.replay_macro(&[
            MacroEvent::MoveDocStart,
            MacroEvent::InsertText("中".into()),
        ], 1, window, cx));
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), before);
        assert_eq!(w.selection().head, caret, "caret before navigation");
        assert!(w.selection().is_empty());
    });
}

#[test]
fn nested_source_transactions_finish_only_after_the_outer_end() {
    with_workspace(true, |w, window, cx| {
        let before = w.buffer().rope.to_string();
        w.begin_edit_group();
        w.begin_edit_group();
        w.replace_byte_range(w.selection().ordered(), "é", cx);
        w.begin_edit_group();
        w.replace_byte_range(w.selection().ordered(), "中", cx);
        w.end_edit_group(cx);
        w.end_edit_group(cx);
        assert!(w.tabs[0].edit_transaction.is_some());
        assert!(w.tabs[0].undo_stack.is_empty(), "outer still owns edits");
        w.replace_byte_range(w.selection().ordered(), "🙂", cx);
        w.end_edit_group(cx);
        assert!(w.tabs[0].edit_transaction.is_none());
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
        assert!(matches!(w.tabs[0].undo_stack[0], UndoEntry::Group { .. }));
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), before);
    });
}

#[test]
fn playback_keeps_an_existing_outer_transaction_open() {
    with_workspace(true, |w, window, cx| {
        let before = w.buffer().rope.to_string();
        w.begin_edit_group();
        w.replace_byte_range(w.selection().ordered(), "start", cx);
        assert!(w.replay_macro(&[
            MacroEvent::InsertText("é".into()),
            MacroEvent::InsertText("中".into()),
        ], 2, window, cx));
        assert!(w.tabs[0].edit_transaction.is_some());
        assert!(w.tabs[0].undo_stack.is_empty());
        w.end_edit_group(cx);
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), before);
    });
}

#[test]
fn nested_column_command_stays_inside_the_playback_undo() {
    with_workspace(true, |w, window, cx| {
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str("aa\nbb");
        w.active_tab_mut().selection.collapse_to(4);
        w.active_tab_mut().block_selection = Some(BlockSelection {
            anchor_line: 0, anchor_col: 0, head_line: 1, head_col: 1,
            is_dragging: false,
        });
        assert!(w.replay_macro(&[
            MacroEvent::Backspace,
            MacroEvent::InsertText("é".into()),
        ], 1, window, cx));
        let after = w.buffer().rope.to_string();
        assert_ne!(after, "aa\nbb");
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
        assert!(w.tabs[0].edit_transaction.is_none());
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), "aa\nbb");
        w.redo(&EditRedo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), after);
    });
}

#[test]
fn failed_playback_commits_earlier_edits_as_one_undo_and_keeps_the_reason() {
    with_workspace(true, |w, window, cx| {
        let before = w.buffer().rope.to_string();
        w.active_tab_mut().buffer.mark_saved();
        assert!(!w.replay_macro(&[
            MacroEvent::InsertText("a".into()),
            MacroEvent::InsertText("n".into()),
            MacroEvent::CompleteWord { prefix: "zz".into(), candidate: "zzebra".into() },
            MacroEvent::InsertText("should not appear".into()),
        ], 3, window, cx));
        assert_eq!(w.buffer().rope.to_string(), format!("{before}an"));
        assert!(w.status_message.as_deref().unwrap().starts_with("Macro stopped after 2 events:"));
        assert!(w.tabs[0].edit_transaction.is_none());
        assert_eq!(w.tabs[0].undo_stack.len(), 1);
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), before);
        assert!(!w.buffer().dirty);
    });
}

#[test]
fn noop_and_refused_playback_keep_redo_and_close_empty_groups() {
    with_workspace(true, |w, window, cx| {
        let before = w.buffer().rope.to_string();
        w.replace_byte_range(w.selection().ordered(), "é", cx);
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.tabs[0].redo_stack.len(), 1);
        for events in [vec![], vec![MacroEvent::InsertText(String::new())], vec![
            MacroEvent::MoveDocStart, MacroEvent::MoveDocEnd,
        ]] {
            w.replay_macro(&events, 1, window, cx);
            assert_eq!(w.buffer().rope.to_string(), before);
            assert_eq!(w.tabs[0].redo_stack.len(), 1);
            assert!(w.tabs[0].undo_stack.is_empty());
            assert!(w.tabs[0].edit_transaction.is_none());
        }
        assert!(!w.replay_macro(&[MacroEvent::CompleteWord {
            prefix: "zz".into(), candidate: "zzebra".into(),
        }], 1, window, cx));
        assert_eq!(w.tabs[0].redo_stack.len(), 1);
        assert!(w.tabs[0].edit_transaction.is_none());
        w.active_tab_mut().buffer.read_only = true;
        assert!(!w.replay_macro(&[MacroEvent::InsertText("bad".into())], 1, window, cx));
        assert_eq!(w.tabs[0].redo_stack.len(), 1);
        assert_eq!(w.buffer().rope.to_string(), before);
        assert!(w.tabs[0].edit_transaction.is_none());
    });
}

#[test]
fn legacy_macro_history_events_keep_their_existing_replay_behavior() {
    with_workspace(true, |w, window, cx| {
        let before = w.buffer().rope.to_string();
        assert!(w.replay_macro(&[
            MacroEvent::InsertText("é".into()), MacroEvent::Undo,
            MacroEvent::Redo, MacroEvent::InsertText("中".into()),
        ], 1, window, cx));
        assert_eq!(w.buffer().rope.to_string(), format!("{before}é中"));
        assert_eq!(w.tabs[0].undo_stack.len(), 2);
        assert!(w.tabs[0].edit_transaction.is_none());
        w.undo(&EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), format!("{before}é"));
    });
}

#[test]
fn new_recordings_exclude_history_commands_and_old_names_still_load() {
    let mut recorder = MacroRecorder::recording_for_test();
    recorder.record_event(MacroEvent::InsertText("é".into()));
    recorder.record_event(MacroEvent::Undo);
    recorder.record_event(MacroEvent::Redo);
    recorder.record_event(MacroEvent::InsertText("中".into()));
    assert!(matches!(recorder.captured_events(), [MacroEvent::InsertText(text)] if text == "é中"));
    assert!(matches!(serde_json::from_str::<MacroEvent>("\"Undo\"").unwrap(), MacroEvent::Undo));
    assert!(matches!(serde_json::from_str::<MacroEvent>("\"Redo\"").unwrap(), MacroEvent::Redo));
}
