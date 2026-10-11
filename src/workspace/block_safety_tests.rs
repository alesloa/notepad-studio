use crate::{actions::*, buffer::{BlockSelection, UndoEntry}, macros, settings::Settings, tab::Tab, workspace::Workspace};
use gpui::{ClipboardItem, Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub(crate) fn with_editor(source: &str, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _execution = macros::override_execution_for_test(false);
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _playback = macros::override_playback_for_test(Vec::new(), Default::default());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty(); tab.buffer.rope = ropey::Rope::from_str(source); tab.buffer.mark_saved();
        w.tabs = vec![tab]; w.active = 0;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked();
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    test(&workspace, visual); cx.quit();
}

fn with_source(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    with_editor("aé\nb😀", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, cx| {
            w.replace_byte_range(0..0, "!", cx); w.replace_byte_range(0..0, "?", cx);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update_window_entity(workspace, |w, _, _| {
            let tab = w.active_tab_mut(); tab.selection.anchor = tab.buffer.rope.len_bytes(); tab.selection.head = 0;
            tab.buffer.bookmarks.insert(0); tab.buffer.marks.push(1..2);
        });
        test(workspace, visual);
    });
}
#[derive(Debug, PartialEq, Eq)]
struct Proof {
    text: String,
    selection: (usize, usize, bool),
    block: Option<(usize, usize, usize, usize, bool)>,
    history: (Vec<u8>, Vec<u8>),
    dirty: bool,
    transaction: Option<(usize, Vec<u8>)>,
    bookmarks: Vec<usize>,
    marks: Vec<std::ops::Range<usize>>,
}

fn proof(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Proof {
    visual.update(|_, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[w.active];
        Proof { text: tab.buffer.rope.to_string(), selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            block: tab.block_selection.map(|b| (b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging)),
            history: (crate::buffer::serialize_undo_stack(&tab.undo_stack), crate::buffer::serialize_undo_stack(&tab.redo_stack)),
            dirty: tab.buffer.dirty, transaction: tab.edit_transaction.as_ref().map(|txn| (txn.depth, crate::buffer::serialize_undo_stack(&[UndoEntry::Group { ops: txn.ops.clone(), sel_before_anchor: txn.sel_before_anchor, sel_before_head: txn.sel_before_head }]))),
            bookmarks: tab.buffer.bookmarks.iter().copied().collect(), marks: tab.buffer.marks.clone() }
    })
}

#[derive(PartialEq, Eq)]
struct Identities {
    revision: crate::buffer::ContentRevision,
    savepoint: Option<crate::buffer::SavePoint>,
    undo: Vec<(crate::buffer::ContentRevision, crate::buffer::ContentRevision)>,
    redo: Vec<(crate::buffer::ContentRevision, crate::buffer::ContentRevision)>,
    transaction: Vec<(crate::buffer::ContentRevision, crate::buffer::ContentRevision)>,
}

fn identities(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Identities {
    fn revisions(stack: &[UndoEntry]) -> Vec<(crate::buffer::ContentRevision, crate::buffer::ContentRevision)> {
        let mut out = Vec::new();
        for entry in stack {
            match entry {
                UndoEntry::Single(op) => out.push((op.before_revision.clone(), op.after_revision.clone())),
                UndoEntry::Group { ops, .. } => out.extend(ops.iter().map(|op| (op.before_revision.clone(), op.after_revision.clone()))),
                UndoEntry::Prepared(group) => out.push((group.target(true).1, group.target(false).1)),
            }
        }
        out
    }
    visual.update(|_, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[w.active];
        Identities { revision: tab.buffer.raw_revision.clone(), savepoint: tab.buffer.savepoint.clone(),
            undo: revisions(&tab.undo_stack), redo: revisions(&tab.redo_stack),
            transaction: tab.edit_transaction.as_ref().map_or_else(Vec::new, |txn| txn.ops.iter().map(|op| (op.before_revision.clone(), op.after_revision.clone())).collect()) }
    })
}

fn set_block(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, rows: (usize, usize), cols: (usize, usize)) {
    visual.update_window_entity(workspace, |w, _, _| w.set_block_selection(Some(BlockSelection {
        anchor_line: rows.0, head_line: rows.1, anchor_col: cols.0, head_col: cols.1, is_dragging: true,
    })));
}

fn assert_refused(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, before: Proof, identity: Identities) {
    assert_eq!(proof(workspace, visual), before, "refused column edit must preserve the whole draft");
    assert!(identities(workspace, visual) == identity, "history and savepoint identities must remain unchanged");
    assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).unwrap(), "sentinel");
}

#[test]
fn column_safety_delete_extreme_thin_column_refuses_before_geometry_or_history_changes() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (0, 1), (usize::MAX, usize::MAX));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        let result = catch_unwind(AssertUnwindSafe(|| visual.dispatch_action(EditDelete)));
        assert!(result.is_ok(), "registered Delete overflowed the thin column");
        assert_refused(workspace, visual, before, identity);
    });
}

#[test]
fn column_safety_copy_extreme_width_refuses_without_allocating_or_changing_clipboard() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (1, 0), (usize::MAX, 0));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        // usize::MAX padding exceeds Rust's isize allocation bound, so the
        // baseline fails immediately with capacity overflow before allocating.
        let result = catch_unwind(AssertUnwindSafe(|| visual.dispatch_action(EditCopy)));
        assert!(result.is_ok(), "registered Copy overflowed its virtual-space allocation");
        assert_refused(workspace, visual, before, identity);
    });
}

#[test]
fn column_safety_paste_extreme_virtual_column_refuses_before_any_row_mutation() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (0, 1), (usize::MAX, usize::MAX));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.write_to_clipboard(ClipboardItem::new_string(format!("{}X", super::BLOCK_CLIPBOARD_SENTINEL)));
        let clipboard = visual.read_from_clipboard().and_then(|item| item.text()).unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| visual.dispatch_action(EditPaste)));
        assert!(result.is_ok(), "registered Paste overflowed its virtual-space allocation");
        assert_eq!(proof(workspace, visual), before);
        assert!(identities(workspace, visual) == identity, "history and savepoint identities must remain unchanged");
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).unwrap(), clipboard);
    });
}

#[test]
fn column_safety_copy_stale_rows_refuses_whole_rectangle_without_clipboard_change() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (0, 9), (0, 2));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.dispatch_action(EditCopy);
        assert_refused(workspace, visual, before, identity);
    });
}

#[test]
fn column_safety_read_only_cut_preserves_clipboard_and_both_selection_geometries() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (1, 0), (2, 0));
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer.read_only = true);
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.dispatch_action(EditCut);
        assert_refused(workspace, visual, before, identity);
    });
}

fn classic_case_rectangle(upper: bool) {
    let source = if upper { "aßc\nxßy" } else { "AİC\nXİY" };
    with_editor(source, |workspace, visual| {
        set_block(workspace, visual, (1, 0), (2, 1));
        if upper { visual.dispatch_action(EditUppercase); } else { visual.dispatch_action(EditLowercase); }
        let expected = if upper { "aSSc\nxSSy" } else { "Ai\u{307}C\nXi\u{307}Y" };
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected);
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            let block = w.block_selection().unwrap(); assert_eq!((block.anchor_line, block.head_line), (1, 0));
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
    });
}

#[test]
fn column_safety_classic_uppercase_rectangle_consistency() { classic_case_rectangle(true); }
#[test]
fn column_safety_classic_lowercase_rectangle_consistency() { classic_case_rectangle(false); }
#[test]
fn column_safety_copy_work_limit_and_virtual_space_boundary() {
    let width = (super::super::column_plan::MAX_WORK - super::BLOCK_CLIPBOARD_SENTINEL.len()) / 2;
    for accepted in [true, false] {
        with_editor("", |workspace, visual| {
            set_block(workspace, visual, (0, 0), (0, width + usize::from(!accepted)));
            let before = proof(workspace, visual); let identity = identities(workspace, visual);
            visual.dispatch_action(EditCopy);
            if accepted {
                assert_eq!(proof(workspace, visual), before);
                let copied = visual.read_from_clipboard().and_then(|item| item.text()).unwrap();
                assert_eq!(copied.strip_prefix(super::BLOCK_CLIPBOARD_SENTINEL).unwrap().len(), width);
                assert!(copied.ends_with(&" ".repeat(width)));
            } else { assert_refused(workspace, visual, before, identity); }
        });
    }
}

#[test]
fn column_safety_copy_row_limit_accepts_4096_and_refuses_4097() {
    for rows in [4096, 4097] {
        let source = "\n".repeat(rows - 1);
        with_editor(&source, |workspace, visual| {
            set_block(workspace, visual, (rows - 1, 0), (0, 0));
            let before = proof(workspace, visual); let identity = identities(workspace, visual);
            visual.dispatch_action(EditCopy);
            if rows == 4096 {
                assert_eq!(proof(workspace, visual), before);
                assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).unwrap(), format!("{}{source}", super::BLOCK_CLIPBOARD_SENTINEL));
            } else { assert_refused(workspace, visual, before, identity); }
        });
    }
}

#[test]
fn column_safety_late_row_work_refusal_preserves_the_entire_draft() {
    let source = format!("x\n{}", "é".repeat(70_000));
    with_editor(&source, |workspace, visual| {
        set_block(workspace, visual, (1, 0), (70_000, 0));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.dispatch_action(EditCut);
        assert_refused(workspace, visual, before, identity);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_none());
            assert!(w.status_message.as_deref().unwrap().contains("128 KiB"));
        });
    });
}

#[test]
fn column_safety_existing_transaction_metadata_is_charged_before_row_one() {
    with_editor(&"\n".repeat(4095), |workspace, visual| {
        set_block(workspace, visual, (4095, 0), (0, 0));
        visual.update_window_entity(workspace, |w, _, _| {
            w.begin_edit_group();
            let revision = w.buffer().raw_revision.clone();
            let txn = w.active_tab_mut().edit_transaction.as_mut().unwrap();
            txn.ops = (0..10_000).map(|_| crate::buffer::EditOp {
                before_revision: revision.clone(), after_revision: revision.clone(), byte_start: 0,
                removed: String::new(), inserted: String::new(), sel_before_anchor: 0, sel_before_head: 0,
            }).collect();
            txn.ops.shrink_to_fit();
        });
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.update_window_entity(workspace, |w, _, cx| {
            assert!(!w.block_replace_geometry(w.block_selection().unwrap(), "x", 0, cx));
        });
        assert_refused(workspace, visual, before, identity);
        visual.update(|_, cx| assert!(workspace.read(cx).status_message.as_deref().unwrap().contains("1 MiB")));
    });
}

#[test]
fn column_safety_unicode_cut_paste_delete_keep_row_direction_and_one_undo() {
    for reverse in [false, true] {
        with_editor("aéz\nx😀y", |workspace, visual| {
            set_block(workspace, visual, if reverse { (1, 0) } else { (0, 1) }, if reverse { (2, 1) } else { (1, 2) });
            visual.dispatch_action(EditCut);
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "az\nxy");
                assert_eq!(w.tabs[0].undo_stack.len(), 1);
                let block = w.block_selection().unwrap(); assert_eq!((block.anchor_line, block.head_line), if reverse { (1, 0) } else { (0, 1) });
            });
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).unwrap(), format!("{}é\n😀", super::BLOCK_CLIPBOARD_SENTINEL));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "aéz\nx😀y"));
            set_block(workspace, visual, (1, 0), (1, 1));
            visual.write_to_clipboard(ClipboardItem::new_string(format!("{}中\né", super::BLOCK_CLIPBOARD_SENTINEL)));
            visual.dispatch_action(EditPaste);
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "a中éz\nxé😀y"); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            set_block(workspace, visual, (1, 0), (2, 2)); visual.dispatch_action(EditBackspace);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "az\nxy"));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            set_block(workspace, visual, (1, 0), (1, 1)); visual.dispatch_action(EditDelete);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "az\nxy"));
        });
    }
}

#[test]
fn column_safety_native_input_virtual_space_and_column_ime_refusal() {
    use gpui::{ElementInputHandler, InputHandler};
    with_editor("é\n😀", |workspace, visual| {
        set_block(workspace, visual, (1, 0), (3, 3));
        visual.update(|window, cx| ElementInputHandler::new(window.bounds(), workspace.clone()).replace_text_in_range(None, "中", window, cx));
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "é  中\n😀  中");
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            let block = w.block_selection().unwrap(); assert_eq!((block.anchor_line, block.head_line, block.anchor_col, block.head_col), (1, 0, 4, 4));
        });
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
            handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
            assert_eq!(handler.marked_text_range(window, cx), None);
        });
        assert_refused(workspace, visual, before, identity);
        set_block(workspace, visual, (1, 0), (usize::MAX, usize::MAX));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.update(|window, cx| ElementInputHandler::new(window.bounds(), workspace.clone()).replace_text_in_range(None, "中", window, cx));
        assert_refused(workspace, visual, before, identity);
    });
}

#[test]
fn column_safety_ordinary_stream_input_and_ime_keep_single_undo() {
    use gpui::{ElementInputHandler, InputHandler};
    for marked in [false, true] {
        with_editor("a😀z", |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().selection.collapse_to(1));
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                if marked {
                    handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                    handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                }
                handler.replace_text_in_range(None, "日本", window, cx);
            });
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "a日本😀z"); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a😀z"));
        });
    }
}

#[test]
fn column_safety_stale_readonly_loading_and_view_refusals_do_not_record_events() {
    use gpui::Action;
    for guard in 0..4 {
        for action in [EditBackspace.boxed_clone(), EditDelete.boxed_clone(), EditCut.boxed_clone(), EditDeleteLineLeft.boxed_clone(), EditDeleteLineRight.boxed_clone(), EditUppercase.boxed_clone(), EditLowercase.boxed_clone()] {
            with_source(|workspace, visual| {
                set_block(workspace, visual, (1, 0), (2, 0));
                visual.update_window_entity(workspace, |w, _, _| match guard {
                    0 => w.active_tab_mut().block_selection.as_mut().unwrap().head_line = usize::MAX,
                    1 => w.active_tab_mut().buffer.read_only = true,
                    2 => w.active_tab_mut().buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 0, total_bytes: 100, cancel: Default::default() }),
                    _ => w.active_tab_mut().view_mode = crate::tab::TabViewMode::MarkdownPreview,
                });
                let before = proof(workspace, visual); let identity = identities(workspace, visual);
                let _recording = macros::override_execution_for_test(true);
                visual.update(|window, cx| window.dispatch_action(action, cx));
                assert_refused(workspace, visual, before, identity);
                assert!(!macros::stop_recording()); assert!(macros::recorded_events_for_test().unwrap().is_empty(), "refusal must not become a saved event");
            });
        }
    }
}

#[test]
fn column_safety_readonly_copy_remains_available_and_find_focus_edits_only_its_field() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (1, 0), (2, 1));
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer.read_only = true);
        let before = proof(workspace, visual); visual.dispatch_action(EditCopy); assert_eq!(proof(workspace, visual), before);
        assert!(visual.read_from_clipboard().and_then(|item| item.text()).unwrap().starts_with(super::BLOCK_CLIPBOARD_SENTINEL));
        visual.update_window_entity(workspace, |w, window, cx| {
            w.find_state.query = "query".into(); w.find_state.query_cursor = 5; w.find_state.query_anchor = 5;
            w.focus_input(crate::tab::InputFocus::FindQuery, window, cx);
        });
        let before = proof(workspace, visual);
        visual.update_window_entity(workspace, |w, window, cx| w.backspace(&EditBackspace, window, cx));
        assert_eq!(proof(workspace, visual), before);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).find_state.query, "quer"));
    });
}
#[test]
fn column_safety_word_delete_readonly_rectangle_refuses_before_clearing_raw_block() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (1, 0), (2, 1));
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer.read_only = true);
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.dispatch_action(EditDeleteWordLeft);
        assert_refused(workspace, visual, before, identity);
    });
}

#[test]
fn column_safety_word_delete_stale_rectangle_refuses_before_clearing_raw_block() {
    with_source(|workspace, visual| {
        set_block(workspace, visual, (usize::MAX, 0), (2, 1));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.dispatch_action(EditDeleteWordRight);
        assert_refused(workspace, visual, before, identity);
    });
}
#[test]
fn column_safety_word_fallback_keeps_ordinary_stream_semantics_and_one_undo() {
    for left in [true, false] {
        with_editor("one two\nend", |workspace, visual| {
            set_block(workspace, visual, (1, 0), (2, 1));
            visual.update_window_entity(workspace, |w, _, _| {
                w.active_tab_mut().selection.anchor = if left { 7 } else { 0 };
                w.active_tab_mut().selection.head = if left { 3 } else { 4 };
            });
            if left { visual.dispatch_action(EditDeleteWordLeft); } else { visual.dispatch_action(EditDeleteWordRight); }
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), if left { " two\nend" } else { "one \nend" });
                assert!(w.block_selection().is_none()); assert_eq!(w.tabs[0].undo_stack.len(), 1);
                assert_eq!((w.selection().anchor, w.selection().head), if left { (4, 0) } else { (0, 4) });
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "one two\nend"));
        });
    }
}

#[test]
fn column_safety_word_fallback_limit_refusal_keeps_histories_clipboard_and_recording_empty() {
    with_editor(&format!("{}\nx", "a".repeat(super::super::column_plan::MAX_WORK + 1)), |workspace, visual| {
        set_block(workspace, visual, (1, 0), (1, 0));
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        let _recording = macros::override_execution_for_test(true);
        visual.dispatch_action(EditDeleteWordRight); visual.run_until_parked();
        assert_refused(workspace, visual, before, identity);
        assert!(!macros::stop_recording()); assert!(macros::recorded_events_for_test().unwrap().is_empty());
    });
}

#[test]
fn column_safety_extended_case_actions_use_the_same_refusal_preflight() {
    use gpui::Action;
    for action in [EditProperCaseForce.boxed_clone(), EditProperCaseBlend.boxed_clone(), EditSentenceCaseForce.boxed_clone(),
        EditSentenceCaseBlend.boxed_clone(), EditInvertCase.boxed_clone(), EditRandomCase.boxed_clone()] {
        with_source(|workspace, visual| {
            set_block(workspace, visual, (usize::MAX, 0), (2, 0));
            let before = proof(workspace, visual); let identity = identities(workspace, visual);
            visual.update(|window, cx| window.dispatch_action(action, cx)); visual.run_until_parked();
            assert_refused(workspace, visual, before, identity);
        });
    }
}
#[test]
fn column_safety_disk_loaded_bom_utf16_crlf_and_unicode_edits_save_and_undo() {
    use crate::text::{encoding::{encode, Encoding}, line_ending::LineEnding};
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let dir = crate::persist::TempDir::new("column-encoded-source"); let path = dir.join("source.txt");
        std::fs::write(&path, encode("aéz\r\nx😀y\r\n", encoding)).unwrap();
        let buffer = crate::io::read_buffer(&path).unwrap();
        with_editor("", |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer = buffer);
            set_block(workspace, visual, (1, 0), (2, 1)); visual.dispatch_action(EditDelete); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "az\nxy\n");
                assert_eq!(w.buffer().encoding, encoding); assert_eq!(w.buffer().line_ending, LineEnding::Crlf);
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.buffer().dirty);
            });
            visual.dispatch_action(FileSave); visual.run_until_parked();
            assert_eq!(std::fs::read(&path).unwrap(), encode("az\r\nxy\r\n", encoding));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "aéz\nx😀y\n"); assert!(w.buffer().dirty); });
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        });
    }
}
#[test]
fn column_safety_non_editor_primitive_paste_keeps_field_routing_and_plain_readonly_behavior() {
    with_source(|workspace, visual| {
        visual.write_to_clipboard(ClipboardItem::new_string("é\r\n中".into()));
        visual.update_window_entity(workspace, |w, window, cx| {
            w.find_state.query = "query".into(); w.find_state.query_anchor = 0; w.find_state.query_cursor = 5;
            w.focus_input(crate::tab::InputFocus::FindQuery, window, cx);
        });
        let before = proof(workspace, visual);
        visual.update_window_entity(workspace, |w, window, cx| assert!(w.replay_event(&macros::MacroEvent::Paste, window, cx)));
        assert_eq!(proof(workspace, visual), before);
        visual.update_window_entity(workspace, |w, window, cx| {
            assert_eq!(w.find_state.query, "é中"); w.focus_input(crate::tab::InputFocus::Editor, window, cx);
            w.active_tab_mut().buffer.read_only = true;
        });
        let before = proof(workspace, visual); let identity = identities(workspace, visual);
        visual.dispatch_action(EditPaste); visual.run_until_parked();
        assert_eq!(proof(workspace, visual), before); assert!(identities(workspace, visual) == identity);
    });
}