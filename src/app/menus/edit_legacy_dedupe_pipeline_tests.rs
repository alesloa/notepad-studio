// ScintillaEditView.cpp:4806–4871; Notepad_plus.cpp:1759–1842.
// Geometry is inferred from Scintilla target deletion, not native screenshots.
use super::*;

#[test]
fn legacy_phase3_any_dedupe_rectangle_replacement_collapses_and_undo_restores_corners() {
    with_source("outside\nx\nx\nx\ny\nend",|workspace,visual| {
        visual.update_window_entity(workspace,|w,_,_| {
            w.tabs[0].block_selection=Some(BlockSelection { anchor_line:1,head_line:4,anchor_col:1,head_col:1,is_dragging:false });
            w.tabs[0].selection.anchor=9; w.tabs[0].selection.head=15;
        });
        let before=state(workspace,visual); invoke(&CASES[10],visual);
        assert_eq!(text(workspace,visual),"outside\nx\ny\nend");
        let after=state(workspace,visual); assert_eq!(after.block,Some((1,1,0,0,false))); assert_eq!(after.selection,(8,8,false));
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        let after=state(workspace,visual); assert_eq!(after.text,before.text); assert_eq!(after.block,before.block);
        assert_eq!(after.selection,before.selection); assert_eq!(after.revision,before.revision);
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace,visual),"outside\nx\ny\nend");
        assert_eq!(state(workspace,visual).block,Some((1,1,0,0,false)));
    });
}

#[test]
fn legacy_phase3_consecutive_dedupe_maps_both_rectangle_directions_through_deletions() {
    for reversed in [false,true] {
        with_source("outside\nx\nx\nx\ny\nend",|workspace,visual| {
            visual.update_window_entity(workspace,|w,_,_| {
                w.tabs[0].block_selection=Some(BlockSelection { anchor_line:if reversed {4}else{1},head_line:if reversed {1}else{4},
                    anchor_col:1,head_col:1,is_dragging:false });
                w.tabs[0].selection.anchor=if reversed {15}else{9}; w.tabs[0].selection.head=if reversed {9}else{15};
            });
            let before=state(workspace,visual); invoke(&CASES[11],visual); assert_eq!(text(workspace,visual),"outside\nx\ny\nend");
            let after=state(workspace,visual);
            assert_eq!(after.block,Some((if reversed {2}else{1},if reversed {1}else{2},1,1,false)));
            assert_eq!(after.selection,if reversed {(11,9,false)}else{(9,11,false)});
            visual.dispatch_action(EditUndo); visual.run_until_parked(); let after=state(workspace,visual);
            assert_eq!(after.text,before.text); assert_eq!(after.block,before.block); assert_eq!(after.selection,before.selection);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace,visual),"outside\nx\ny\nend");
        });
    }
}

#[test]
fn legacy_phase3_consecutive_eof_corner_mapping_and_virtual_space() {
    with_source("outside\nx\nx",|workspace,visual| {
        visual.update_window_entity(workspace,|w,_,_| {
            w.tabs[0].block_selection=Some(BlockSelection { anchor_line:1,head_line:2,anchor_col:5,head_col:6,is_dragging:false });
            w.tabs[0].selection.anchor=9; w.tabs[0].selection.head=11;
        });
        let before=state(workspace,visual); invoke(&CASES[11],visual); assert_eq!(text(workspace,visual),"outside\nx");
        // The removed row loses virtual space; the retained EOF corner keeps it.
        assert_eq!(state(workspace,visual).block,Some((1,1,0,6,false)));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(state(workspace,visual).block,before.block);
    });
}

#[test]
fn legacy_phase3_noop_rectangles_preserve_geometry_history_and_selection() {
    for index in [10,11] {
        with_source("outside\nx\ny\nend",|workspace,visual| {
            visual.update_window_entity(workspace,|w,_,_| w.tabs[0].block_selection=Some(BlockSelection {
                anchor_line:1,head_line:2,anchor_col:1,head_col:5,is_dragging:true }));
            let before=state(workspace,visual); invoke(&CASES[index],visual); assert!(state(workspace,visual)==before);
        });
    }
}

#[test]
fn legacy_phase3_dedupe_context_refusals_preserve_seeded_history_and_clipboard() {
    for context in 0..10 { refusal_cases(context,&[&CASES[10],&CASES[11]]); }
}

#[test]
fn legacy_phase3_disjoint_consecutive_deletions_keep_backward_stream_endpoints() {
    with_source("b\nb\na\nc\nc\nd",|workspace,visual| {
        visual.update_window_entity(workspace,|w,_,_| { w.tabs[0].selection.anchor=11; w.tabs[0].selection.head=1; });
        let before=state(workspace,visual); invoke(&CASES[11],visual); assert_eq!(text(workspace,visual),"b\na\nc\nd");
        assert_eq!(state(workspace,visual).selection,(7,1,false));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(state(workspace,visual).selection,before.selection);
    });
}
