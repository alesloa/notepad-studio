// NppCommands.cpp:933–1063 and Scintilla Selection.cxx:54–80,131–143.
// Endpoint geometry is source-inferred; this is registered GPUI caller proof.
use super::*;
use crate::edit_ops::legacy_lines::override_seed;

fn block_tuple(block: BlockSelection) -> (usize, usize, usize, usize) {
    (block.anchor_line, block.head_line, block.anchor_col, block.head_col)
}

#[test]
fn legacy_phase1_rectangle_geometry_undo_and_redo() {
    for (index, expected) in [(0, "outside\nx a\ny a\nz b\nend"), (1, "outside\nz b\nx a\ny a\nend"),
        (8, "outside\ny a\nz b\nx a\nend"), (9, "outside\ny a\nz b\nx a\nend")] {
        let _seed = override_seed(1);
        let source = "outside\nx a\nz b\ny a\nend";
        with_source(source, |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| w.tabs[0].block_selection = Some(BlockSelection {
                anchor_line: 1, head_line: 3, anchor_col: 2, head_col: 3, is_dragging: false }));
            let before = state(workspace, visual);
            invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected);
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(block_tuple(w.tabs[0].block_selection.unwrap()), (1,1,0,0));
                assert_eq!((w.selection().anchor, w.selection().head), (8,8)); assert_eq!(w.tabs[0].undo_stack.len(), 1); });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            let undone = state(workspace, visual); assert_eq!(undone.text, source); assert_eq!(undone.block, before.block);
            assert_eq!(undone.selection, before.selection); assert_eq!(undone.revision, before.revision);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), expected);
            visual.update(|_, cx| assert_eq!(block_tuple(workspace.read(cx).tabs[0].block_selection.unwrap()), (1,1,0,0)));
        });
    }
}

#[test]
fn legacy_phase1_noop_rectangle_preserves_geometry_and_history() {
    with_source("x\ny", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].block_selection = Some(BlockSelection {
            anchor_line: 0, head_line: 1, anchor_col: 0, head_col: 1, is_dragging: true }));
        let before = state(workspace, visual); invoke(&CASES[0], visual); assert!(state(workspace, visual) == before);
    });
}

#[test]
fn legacy_phase1_saved_macro_continues_and_undoes_once() {
    saved_macro_continuation(&[0,1,8,9]);
}

#[test]
fn legacy_phase2_saved_numeric_macros_continue_and_undo_once() { saved_macro_continuation(&[2,3,4,5,6,7]); }

#[test]
fn legacy_phase3_saved_dedupe_macros_continue_and_undo_once() { saved_macro_continuation(&[10,11]); }

fn saved_macro_continuation(indices: &[usize]) {
    for &index in indices {
        let _seed = override_seed(1); let case = &CASES[index];
        with_source(case.source, |workspace, visual| {
            visual.dispatch_action(MacroStartRecording); invoke(case, visual); visual.dispatch_action(MacroStopRecording);
            let expected = text(workspace, visual); let events = macros::current(); assert_eq!(events.len(),1);
            let directory = crate::persist::TempDir::new("legacy-phase1-macro");
            let mut saved = macros::saved_round_trip_for_test(&directory.join("macros.toml"), &BTreeMap::from([("sort".into(),events)])).unwrap();
            let events = saved.get_mut("sort").unwrap(); events.push(macros::MacroEvent::InsertText("!".into()));
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), case.source);
            visual.update_window_entity(workspace, |w, window, cx| { w.replay_macro(events,1,window,cx); assert!(w.pending_prepared_edit.is_some()); });
            visual.run_until_parked(); assert_eq!(text(workspace, visual), format!("!{expected}"));
            visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(),1));
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), case.source);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), format!("!{expected}"));
        });
    }
}

#[test]
fn legacy_phase1_context_refusals_preserve_seeded_history_and_clipboard() {
    for context in 0..10 { refusal_cases(context, &[&CASES[0],&CASES[1],&CASES[8],&CASES[9]]); }
}

#[test]
fn legacy_phase1_cancel_and_stale_candidates_do_not_publish() {
    candidate_refusals(&[0,1,8,9]);
}

#[test]
fn legacy_phase2_cancel_and_stale_numeric_candidates_do_not_publish() { candidate_refusals(&[2,3,4,5,6,7]); }

#[test]
fn legacy_phase3_cancel_and_stale_dedupe_candidates_do_not_publish() { candidate_refusals(&[10,11]); }

fn candidate_refusals(indices: &[usize]) {
    for &index in indices {
        for change in ["escape","selection","block","activity"] {
            for publish in [false,true] {
            let _seed = override_seed(1); let case = &CASES[index];
            with_source(case.source, |workspace, visual| {
                let (send,receive) = futures::channel::oneshot::channel();
                visual.update_window_entity(workspace, |w, _, _| {
                    if publish { w.prepared_test_publish_gate=Some(receive); } else { w.prepared_test_gate=Some(receive); }
                });
                invoke(case,visual); let original = text(workspace,visual);
                visual.update(|_,cx| assert!(workspace.read(cx).pending_prepared_edit.is_some()));
                if change == "escape" { visual.dispatch_action(EditFindClose); visual.run_until_parked(); }
                else { visual.update_window_entity(workspace, |w, _, _| match change {
                    "selection" => w.tabs[0].selection.collapse_to(1),
                    "block" => w.tabs[0].block_selection = Some(BlockSelection { head_line: 1, ..BlockSelection::default() }),
                    _ => w.tabs[0].content_activity = ContentRevision::default(),
                }); }
                let before = state(workspace,visual); send.send(()).unwrap(); visual.run_until_parked();
                assert_eq!(text(workspace,visual),original); assert!(state(workspace,visual)==before,"{change}");
                visual.update(|_,cx| assert!(workspace.read(cx).pending_prepared_edit.is_none()));
            });
            }
        }
    }
}

#[test]
fn legacy_phase1_newer_sort_corner_policy_is_unchanged() {
    with_source("outside\nx a\nz b\ny a\nend", |workspace,visual| {
        visual.update_window_entity(workspace, |w,_,_| w.tabs[0].block_selection=Some(BlockSelection {
            anchor_line:1,head_line:3,anchor_col:2,head_col:3,is_dragging:false }));
        let before=state(workspace,visual);
        visual.dispatch_action(EditSortLinesLexIgnoreCaseAsc); visual.run_until_parked();
        assert_eq!(text(workspace,visual),"outside\nx a\ny a\nz b\nend"); assert_eq!(state(workspace,visual).block,before.block);
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(state(workspace,visual).block,before.block);
    });
}
