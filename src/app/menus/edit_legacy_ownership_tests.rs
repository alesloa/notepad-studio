use super::*;
use crate::edit_ops::legacy_lines::override_seed;

#[test]
fn legacy_registered_type_then_undo_aba_never_publishes_a_stale_candidate() {
    for case in &CASES {
        let _seed=override_seed(1);
        with_source(case.source,|workspace,visual| {
            let (send,receive)=futures::channel::oneshot::channel();
            visual.update_window_entity(workspace,|w,_,_| w.prepared_test_gate=Some(receive));
            let original=state(workspace,visual); invoke(case,visual);
            visual.update(|_,cx| assert!(workspace.read(cx).pending_prepared_edit.is_some()));
            visual.simulate_input("x"); visual.run_until_parked(); assert_eq!(text(workspace,visual),format!("x{}",case.source));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            let current=state(workspace,visual); assert_eq!(current.text,original.text); assert_eq!(current.revision,original.revision);
            send.send(()).unwrap(); visual.run_until_parked(); assert!(state(workspace,visual)==current,"{}",case.id);
            visual.update(|_,cx| assert!(workspace.read(cx).pending_prepared_edit.is_none()));
        });
    }
}

#[test]
fn legacy_saved_rectangle_macro_mixed_history_restores_geometry_with_one_undo() {
    for (index,case) in CASES.iter().enumerate() {
        let _seed=override_seed(1);
        let source=if index>=10 { "outside\nx 1\nx 1\ny 2\nend" } else { "outside\nx 2\nz 3\ny 1\nend" };
        with_source(source,|workspace,visual| {
            visual.update_window_entity(workspace,|w,_,_| {
                w.tabs[0].block_selection=Some(BlockSelection { anchor_line:1,head_line:3,anchor_col:2,head_col:3,is_dragging:false });
                w.tabs[0].selection.anchor=10; w.tabs[0].selection.head=19;
            });
            let before=state(workspace,visual);
            visual.dispatch_action(MacroStartRecording); invoke(case,visual); visual.dispatch_action(MacroStopRecording);
            let transformed=text(workspace,visual); let events=macros::current(); assert_eq!(events.len(),1);
            let directory=crate::persist::TempDir::new("legacy-rectangle-macro");
            let saved=macros::saved_round_trip_for_test(&directory.join("macros.toml"),&BTreeMap::from([("rect".into(),events)])).unwrap();
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace,visual),source);
            assert_eq!(state(workspace,visual).block,before.block);
            // A scalar edit after the rectangle command exercises the common
            // prepared/legacy merger and its initial/final geometry capture.
            let mut events=saved.get("rect").unwrap().clone(); events.push(macros::MacroEvent::InsertText("!".into()));
            visual.update_window_entity(workspace,|w,window,cx| { w.replay_macro(&events,1,window,cx); });
            visual.run_until_parked(); let completed=state(workspace,visual);
            assert!(completed.text.contains('!')); assert_ne!(completed.text,transformed); assert_eq!(completed.undo,1);
            visual.dispatch_action(EditUndo); visual.run_until_parked(); let undone=state(workspace,visual);
            assert_eq!(undone.text,before.text); assert_eq!(undone.block,before.block); assert_eq!(undone.selection,before.selection);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace,visual),completed.text);
            assert_eq!(state(workspace,visual).block,completed.block);
        });
    }
}

#[test]
fn legacy_all_twelve_hostile_mid_character_and_past_eof_selections_keep_exact_rows() {
    for (index,case) in CASES.iter().enumerate() {
        let _seed=override_seed(1);
        let source=case.source.split('\n').map(|row|format!("{row}é")).collect::<Vec<_>>().join("\n");
        let base=if index==9 { "a\nb\nc" } else { case.expected };
        let expected=base.split('\n').map(|row|format!("{row}é")).collect::<Vec<_>>().join("\n");
        let middle=source.split('\n').next().unwrap().len()-1;
        for backward in [false,true] {
            with_source(&source,|workspace,visual| {
                visual.update_window_entity(workspace,|w,_,_| {
                    w.tabs[0].selection.anchor=if backward {usize::MAX}else{middle};
                    w.tabs[0].selection.head=if backward {middle}else{usize::MAX};
                });
                invoke(case,visual); assert_eq!(text(workspace,visual),expected,"{}",case.id);
                visual.update(|_,cx| { let w=workspace.read(cx); let selection=w.selection();
                    assert_eq!(crate::buffer::floor_char_boundary(&w.buffer().rope,selection.anchor),selection.anchor);
                    assert_eq!(crate::buffer::floor_char_boundary(&w.buffer().rope,selection.head),selection.head); });
                visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace,visual),source);
            });
        }
    }
}

#[test]
fn legacy_hostile_rectangle_rows_and_overflowing_corners_preserve_draft_and_history() {
    for case in &CASES {
        for block in [BlockSelection { anchor_line:usize::MAX,head_line:1,anchor_col:0,head_col:1,is_dragging:false },
            BlockSelection { anchor_line:0,head_line:2,anchor_col:usize::MAX,head_col:usize::MAX,is_dragging:false }] {
            with_source(case.source,|workspace,visual| {
                visual.update_window_entity(workspace,|w,_,cx| { w.replace_byte_range(0..0,"seed",cx); w.replace_byte_range(0..0,"redo",cx); });
                visual.dispatch_action(EditUndo); visual.run_until_parked();
                visual.update_window_entity(workspace,|w,_,_| w.tabs[0].block_selection=Some(block));
                visual.dispatch_action(MacroStartRecording);
                let before=state(workspace,visual); invoke(case,visual); assert!(state(workspace,visual)==before,"{}",case.id);
                visual.update(|_,cx| assert!(workspace.read(cx).status_message.is_some()));
                visual.dispatch_action(MacroStopRecording);
                if block.anchor_line==usize::MAX { assert!(macros::current().is_empty()); }
                else { assert_eq!(macros::current().len(),1); assert_eq!(format!("{:?}",macros::current()[0]),case.event); }
            });
        }
    }
}
