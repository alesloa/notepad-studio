use super::*;
use crate::{actions::*, buffer::BlockSelection, workspace::with_column_editor};
use gpui::{ClipboardItem, ElementInputHandler, InputHandler, VisualContext};

fn rectangle(workspace: &gpui::Entity<crate::workspace::Workspace>, visual: &mut gpui::VisualTestContext, column: usize) {
    visual.update_window_entity(workspace, |w, _, _| w.set_block_selection(Some(BlockSelection {
        anchor_line: 1, head_line: 0, anchor_col: column + 1, head_col: column, is_dragging: true,
    })));
}

#[test]
fn column_safety_record_save_reload_replays_text_and_classic_case_rectangles() {
    let dir = crate::persist::TempDir::new("column-macros"); let path = dir.join("macros.toml");
    let mut saved = BTreeMap::new();
    for (name, source, expected) in [
        ("text", "aéc\nx😀y", "a中c\nx中y"),
        ("upper", "aßc\nxßy", "aSSc\nxSSy"),
        ("lower", "AİC\nXİY", "Ai\u{307}C\nXi\u{307}Y"),
    ] {
        with_column_editor(source, |workspace, visual| {
            rectangle(workspace, visual, 1); let _recording = override_execution_for_test(true);
            match name {
                "upper" => visual.dispatch_action(EditUppercase), "lower" => visual.dispatch_action(EditLowercase),
                _ => visual.update(|window, cx| ElementInputHandler::new(window.bounds(), workspace.clone()).replace_text_in_range(None, "中", window, cx)),
            }
            visual.run_until_parked();
            assert!(!stop_recording()); let events = recorded_events_for_test().unwrap(); assert_eq!(events.len(), 1, "{name}");
            assert!(matches!((&events[0], name), (MacroEvent::Uppercase, "upper") | (MacroEvent::Lowercase, "lower"))
                || matches!(&events[0], MacroEvent::InsertText(text) if name == "text" && text == "中"));
            saved.insert(name.to_owned(), events);
        });
        persist_at(&path, &saved).unwrap();
        with_column_editor(source, |workspace, visual| {
            rectangle(workspace, visual, 1);
            let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
            visual.dispatch_action(MacroRunSaved { name: name.into() }); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected);
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].edit_transaction.is_none());
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        });
    }
    assert!(!dir.join("macros.toml.corrupt").exists());
}

#[test]
fn column_safety_saved_rectangle_refusal_preserves_draft_and_stops_tail() {
    let dir = crate::persist::TempDir::new("column-refused-macro"); let path = dir.join("macros.toml");
    for (name, event) in [("text", MacroEvent::InsertText("中".into())), ("upper", MacroEvent::Uppercase), ("lower", MacroEvent::Lowercase)] {
        let saved = BTreeMap::from([(name.into(), vec![event, MacroEvent::InsertText("tail".into())])]);
        persist_at(&path, &saved).unwrap();
        for guard in 0..3 {
            let source = if guard == 2 && name != "text" { format!("x\n{}", "é".repeat(7000)) } else { "aßc\nxİy".into() };
            with_column_editor(&source, |workspace, visual| {
                rectangle(workspace, visual, 1);
                visual.update_window_entity(workspace, |w, _, _| match guard {
                    0 => w.active_tab_mut().buffer.read_only = true,
                    1 => w.active_tab_mut().block_selection.as_mut().unwrap().head_line = usize::MAX,
                    _ => { let block = w.active_tab_mut().block_selection.as_mut().unwrap(); block.anchor_col = if name == "text" { usize::MAX } else { 7000 }; block.head_col = if name == "text" { usize::MAX } else { 0 }; },
                });
                let before = visual.update(|_, cx| {
                    let w = workspace.read(cx); let tab = &w.tabs[0];
                    (tab.buffer.rope.to_string(), (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
                        tab.block_selection.map(|b| (b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging)),
                        crate::buffer::serialize_undo_stack(&tab.undo_stack), crate::buffer::serialize_undo_stack(&tab.redo_stack),
                        tab.buffer.raw_revision.clone(), tab.buffer.savepoint.clone())
                });
                let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
                visual.dispatch_action(MacroRunSaved { name: name.into() }); visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx); let tab = &w.tabs[0];
                    let after = (tab.buffer.rope.to_string(), (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
                        tab.block_selection.map(|b| (b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging)),
                        crate::buffer::serialize_undo_stack(&tab.undo_stack), crate::buffer::serialize_undo_stack(&tab.redo_stack),
                        tab.buffer.raw_revision.clone(), tab.buffer.savepoint.clone());
                    assert!(after == before, "saved refusal must preserve all draft and revision identities"); assert!(tab.edit_transaction.is_none()); assert!(w.pending_prepared_edit.is_none());
                });
                assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
            });
        }
    }
}

#[test]
fn column_safety_saved_linear_events_keep_the_existing_selection_semantics() {
    for (source, event, expected) in [("aéz", MacroEvent::InsertText("中".into()), "a中z"),
        ("aßc", MacroEvent::Uppercase, "aSSc"), ("AİC", MacroEvent::Lowercase, "Ai\u{307}C")] {
        let dir = crate::persist::TempDir::new("column-linear-macro"); let path = dir.join("macros.toml");
        persist_at(&path, &BTreeMap::from([("linear".into(), vec![event])])).unwrap();
        with_column_editor(source, |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| { w.active_tab_mut().selection.anchor = 1; w.active_tab_mut().selection.head = 3; });
            let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
            visual.dispatch_action(MacroRunSaved { name: "linear".into() }); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
        });
    }
}

#[test]
fn column_safety_mixed_legacy_prepared_macro_merges_column_edits_into_one_undo() {
    with_column_editor("a\nb", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.set_block_selection(Some(BlockSelection {
            anchor_line: 1, head_line: 0, anchor_col: 1, head_col: 1, is_dragging: true,
        })));
        let _playback = override_playback_for_test(vec![MacroEvent::InsertText("中".into()), MacroEvent::InverseBookmarks,
            MacroEvent::InsertText("!".into())], BTreeMap::new());
        visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
        visual.dispatch_action(MacroPlayback); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "a中!\nb中!");
            assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.pending_prepared_edit.is_none() && w.tabs[0].edit_transaction.is_none());
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nb"));
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a中!\nb中!"));
    });
}
#[test]
fn column_safety_saved_word_refusal_stops_tail_before_geometry_or_history_changes() {
    let dir = crate::persist::TempDir::new("column-word-macro"); let path = dir.join("macros.toml");
    persist_at(&path, &BTreeMap::from([("word".into(), vec![MacroEvent::DeleteWordRight, MacroEvent::InsertText("tail".into())])])).unwrap();
    with_column_editor("word\nend", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.set_block_selection(Some(BlockSelection {
            anchor_line: usize::MAX, head_line: 0, anchor_col: 1, head_col: 0, is_dragging: true,
        })));
        let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
        visual.dispatch_action(MacroRunSaved { name: "word".into() }); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "word\nend");
            assert_eq!(w.block_selection().unwrap().anchor_line, usize::MAX);
            assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty() && w.tabs[0].edit_transaction.is_none());
        });
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("sentinel"));
    });
}
#[test]
fn column_safety_saved_plain_multiline_paste_uses_current_clipboard_and_one_undo() {
    let dir = crate::persist::TempDir::new("column-plain-paste"); let path = dir.join("macros.toml");
    persist_at(&path, &BTreeMap::from([("paste".into(), vec![MacroEvent::Paste])])).unwrap();
    with_column_editor("abcde", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| { w.active_tab_mut().selection.anchor = 4; w.active_tab_mut().selection.head = 1; });
        visual.write_to_clipboard(ClipboardItem::new_string("é\r\n中".into()));
        let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
        visual.dispatch_action(MacroRunSaved { name: "paste".into() }); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "aé\n中e");
            assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].edit_transaction.is_none());
        });
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("é\r\n中"));
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "abcde");
            assert_eq!((w.selection().anchor, w.selection().head), (4, 1)); assert!(!w.buffer().dirty);
        });
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "aé\n中e"));
    });
}

#[test]
fn column_safety_saved_empty_paste_continues_and_readonly_plain_paste_refuses_the_macro() {
    let dir = crate::persist::TempDir::new("column-empty-paste"); let path = dir.join("macros.toml");
    persist_at(&path, &BTreeMap::from([("paste".into(), vec![MacroEvent::Paste, MacroEvent::InsertText("!".into())])])).unwrap();
    for readonly in [false, true] {
        with_column_editor("abc", |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| { w.active_tab_mut().selection.collapse_to(3); w.active_tab_mut().buffer.read_only = readonly; });
            visual.write_to_clipboard(ClipboardItem::new_string(String::new()));
            let _playback = override_playback_for_test(Vec::new(), load_saved_at(&path));
            visual.dispatch_action(MacroRunSaved { name: "paste".into() }); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), if readonly { "abc" } else { "abc!" });
                assert_eq!(w.tabs[0].undo_stack.len(), usize::from(!readonly)); assert!(w.tabs[0].edit_transaction.is_none());
                if readonly { assert_eq!(w.selection().head, 3); }
            });
        });
    }
}