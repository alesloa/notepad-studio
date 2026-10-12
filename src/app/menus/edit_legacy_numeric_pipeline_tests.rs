// Numeric error and geometry policy from NppCommands.cpp:1004–1063.
use super::*;

#[test]
fn legacy_invalid_decimal_fidelity_corpus_preserves_whole_draft_geometry_history() {
    let source="10\n-2\n3,5\n3.5\n-.\n.\n-1\n";
    for case in CASES.iter().take(8).skip(4) {
        for rectangle in [false,true] {
            with_source(source,|workspace,visual| {
                visual.update_window_entity(workspace,|w,_,cx| {
                    w.replace_byte_range(0..2,"10",cx); w.replace_byte_range(0..0,"redo",cx);
                });
                visual.dispatch_action(EditUndo); visual.run_until_parked();
                visual.update_window_entity(workspace,|w,_,_| {
                    w.tabs[0].selection.anchor=source.len(); w.tabs[0].selection.head=0;
                    if rectangle { w.tabs[0].block_selection=Some(BlockSelection { anchor_line:7,head_line:0,
                        anchor_col:0,head_col:0,is_dragging:false }); }
                    w.tabs[0].buffer.bookmarks.insert(0); w.tabs[0].buffer.marks.push(0..2); w.tabs[0].begin_select_marker=Some(1);
                });
                let before=state(workspace,visual); assert_eq!((before.undo,before.redo),(1,1));
                invoke(case,visual); assert!(state(workspace,visual)==before);
                visual.update(|_,cx| assert!(workspace.read(cx).status_message.as_deref().unwrap().contains("line 5")));
            });
        }
    }
}

#[test]
fn legacy_phase2_numeric_context_refusals_preserve_seeded_history_and_clipboard() {
    for context in 0..10 { refusal_cases(context,&[&CASES[2],&CASES[3],&CASES[4],&CASES[5],&CASES[6],&CASES[7]]); }
}

#[test]
fn legacy_phase2_numeric_rectangle_keys_geometry_and_undo() {
    for (index,case) in CASES.iter().enumerate().take(8).skip(2) {
        with_source("outside\nx 1\nz 2\ny 1\nend",|workspace,visual| {
            visual.update_window_entity(workspace,|w,_,_| w.tabs[0].block_selection=Some(BlockSelection {
                anchor_line:1,head_line:3,anchor_col:2,head_col:3,is_dragging:false }));
            let before=state(workspace,visual); invoke(case,visual);
            let expected=if index.is_multiple_of(2) { "outside\nx 1\ny 1\nz 2\nend" } else { "outside\nz 2\nx 1\ny 1\nend" };
            assert_eq!(text(workspace,visual),expected);
            assert_eq!(state(workspace,visual).block,Some((1,1,0,0,false)));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            let after=state(workspace,visual); assert_eq!(after.text,before.text); assert_eq!(after.block,before.block);
            assert_eq!(after.selection,before.selection); assert_eq!(after.revision,before.revision);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace,visual),expected);
            assert_eq!(state(workspace,visual).block,Some((1,1,0,0,false)));
        });
    }
}

#[test]
fn legacy_phase2_numeric_failure_records_once_and_preserves_draft_geometry_history() {
    for index in [4,5,6,7] {
        with_source("prefix\nx 2\nx --1\nx 1\nsuffix", |workspace,visual| {
            visual.update_window_entity(workspace,|w,_,cx| {
                w.replace_byte_range(0..0,"seed",cx); w.replace_byte_range(0..0,"redo",cx);
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update_window_entity(workspace,|w,_,_| {
                w.tabs[0].block_selection=Some(BlockSelection { anchor_line:1,head_line:3,anchor_col:2,head_col:5,is_dragging:true });
                w.tabs[0].selection.anchor=21; w.tabs[0].selection.head=12;
                w.tabs[0].buffer.bookmarks.insert(0); w.tabs[0].buffer.marks.push(0..2); w.tabs[0].begin_select_marker=Some(2);
            });
            visual.dispatch_action(MacroStartRecording); let before=state(workspace,visual);
            invoke(&CASES[index],visual); assert!(state(workspace,visual)==before);
            visual.update(|_,cx| assert!(workspace.read(cx).status_message.as_deref().unwrap().contains("line 3")));
            visual.dispatch_action(MacroStopRecording);
            assert_eq!(macros::current().len(),1); assert_eq!(format!("{:?}",macros::current()[0]),CASES[index].event);
        });
    }
}

#[test]
fn legacy_phase2_saved_failed_numeric_macro_stops_before_next_event() {
    for index in [4,5,6,7] {
        with_source("2\n--1\n1",|workspace,visual| {
            visual.dispatch_action(MacroStartRecording); invoke(&CASES[index],visual); visual.dispatch_action(MacroStopRecording);
            let directory=crate::persist::TempDir::new("legacy-numeric-error-macro");
            let mut events=macros::current(); events.push(macros::MacroEvent::InsertText("must not appear".into()));
            let saved=macros::saved_round_trip_for_test(&directory.join("macros.toml"), &BTreeMap::from([("bad".into(),events)])).unwrap();
            let before=state(workspace,visual);
            visual.update_window_entity(workspace,|w,window,cx| { w.replay_macro(saved.get("bad").unwrap(),1,window,cx); });
            visual.run_until_parked(); assert!(state(workspace,visual)==before);
            visual.update(|_,cx| { let w=workspace.read(cx); assert!(w.active_macro_playback_id().is_none());
                assert!(w.status_message.as_deref().unwrap().contains("line 2")); });
        });
    }
}

#[test]
fn legacy_phase2_numeric_equal_keys_and_empty_keys_are_stable() {
    for (index,source,expected) in [
        (6,"2b\n1.0a\nwordA\n1.00b\nwordB\n1c","wordA\nwordB\n1.0a\n1.00b\n1c\n2b"),
        (7,"wordA\n1.0a\n2b\n1.00b\nwordB\n1c","2b\n1.0a\n1.00b\n1c\nwordA\nwordB"),
        (4,"2b\n1,0a\nwordA\n1,00b\nwordB\n1c","wordA\nwordB\n1,0a\n1,00b\n1c\n2b"),
        (5,"wordA\n1,0a\n2b\n1,00b\nwordB\n1c","2b\n1,0a\n1,00b\n1c\nwordA\nwordB"),
    ] {
        with_source(source,|workspace,visual| { invoke(&CASES[index],visual); assert_eq!(text(workspace,visual),expected);
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace,visual),source); });
    }
}

#[test]
fn legacy_phase2_registered_default_nearest_even_subnormal_and_long_tail_controls() {
    let vectors:serde_json::Value=serde_json::from_str(include_str!("../../edit_ops/legacy_decimal_vectors.json")).unwrap();
    for vector in vectors["vectors"].as_array().unwrap() {
        let number=vector["input"].as_str().unwrap(); let source=format!("2\n{number}\n1");
        with_source(&source,|workspace,visual| {
            let before=state(workspace,visual); invoke(&CASES[6],visual);
            if vector["source_inferred_status"]=="OK" {
                let bits=u64::from_str_radix(vector["source_inferred_bits"].as_str().unwrap(),16).unwrap();
                let expected=if f64::from_bits(bits)>1. { format!("1\n{number}\n2") } else { format!("{number}\n1\n2") };
                assert_eq!(text(workspace,visual),expected,"{}",vector["id"]);
                visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace,visual),source);
            } else { assert!(state(workspace,visual)==before,"{}",vector["id"]); }
        });
    }
}
