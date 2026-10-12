use super::*;
use crate::edit_ops::legacy_lines::override_seed;
use std::{path::Path, time::{Duration, Instant}};

const UNIQUE: usize = 45_000;

fn fixture_row(value: usize, payload: &str) -> String { format!("{value:06} {payload}") }

fn large_source(payload: &str, copies: usize) -> String {
    let mut rows=Vec::new();
    for index in 0..UNIQUE {
        let value=index*193%UNIQUE;
        for _ in 0..copies { rows.push(fixture_row(value,payload)); }
    }
    rows.join("\n")
}

// Fixed-width nonnegative keys have the same independent order for the eight
// lexical/numeric commands; duplicate rows contain identical full bodies.
fn golden(index: usize, source: &str, payload: &str) -> Option<String> {
    match index {
        0..=7 => {
            let mut rows=Vec::new();
            for at in 0..UNIQUE {
                let value=if index.is_multiple_of(2) { at } else { UNIQUE-1-at };
                for _ in 0..2 { rows.push(fixture_row(value,payload)); }
            }
            Some(rows.join("\n"))
        }
        8 => Some(source.split('\n').rev().collect::<Vec<_>>().join("\n")),
        9 => None,
        10 | 11 => Some(large_source(payload,1)),
        _ => panic!("unknown fixture command"),
    }
}

fn assert_rows(actual: &str, expected: Option<&str>, source: &str, payload: &str) {
    if let Some(expected)=expected { assert_eq!(actual,expected); }
    else {
        assert_ne!(actual,source);
        let mut counts=vec![0usize;UNIQUE];
        for row in actual.split('\n') {
            let (number,body)=row.split_once(' ').unwrap();
            assert_eq!(number.len(),6); assert_eq!(body,payload);
            let value=number.parse::<usize>().unwrap(); *counts.get_mut(value).unwrap()+=1;
        }
        assert!(counts.iter().all(|&count|count==2));
    }
}

fn open_large(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, path: &Path, source: &str) {
    visual.update_window_entity(workspace,|w,_,cx| {
        w.open_path(path.to_owned(),cx);
        assert!(w.buffer().loading.is_some(),"actual streaming path starts before EOF");
    });
    visual.run_until_parked();
    visual.update(|_,cx| { let w=workspace.read(cx); assert!(w.buffer().loading.is_none());
        assert_eq!(w.active,0); assert_eq!(w.tabs.len(),1); assert_eq!(w.buffer().rope.to_string(),source);
        assert_eq!(w.buffer().encoding,crate::text::encoding::Encoding::Utf8); });
}

fn dispatch(case: &Case, visual: &mut VisualTestContext) -> Duration {
    let start=Instant::now();
    visual.update(|window,cx| {
        assert!(cx.background_executor().is_main_thread());
        window.dispatch_action(action(case.label),cx);
    });
    start.elapsed()
}

fn report(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, case: &Case,
    stage: &str, dispatch: Duration, wall: Duration)
{
    visual.update(|_,cx| {
        let w=workspace.read(cx); let timing=w.prepared_timings.last().unwrap();
        assert_eq!((timing.capture_main_thread,timing.worker_main_thread,timing.publish_main_thread),(true,false,true));
        println!("legacy-large id={} stage={stage} dispatch_ms={:.3} capture_ms={:.3} worker_ms={:.3} publish_ms={:.3} wall_ms={:.3} gpui_capture_foreground={} gpui_worker_foreground={} gpui_publish_foreground={}",
            case.id,dispatch.as_secs_f64()*1000.,timing.capture.as_secs_f64()*1000.,timing.worker.as_secs_f64()*1000.,
            timing.publish.as_secs_f64()*1000.,wall.as_secs_f64()*1000.,timing.capture_main_thread,timing.worker_main_thread,timing.publish_main_thread);
    });
}

#[test]
#[ignore = "default-release >10 MiB installed legacy callers, history and saved mixed macros"]
fn legacy_release_large_callers_saved_macros_and_one_undo() {
    let payload="é🙂payload ".repeat(9); let source=large_source(&payload,2);
    assert_eq!(source.len(),12_059_999); assert!(source.len()>crate::io_stream::STREAM_THRESHOLD_BYTES as usize);
    let directory=crate::persist::TempDir::new("legacy-large-callers"); let path=directory.join("neutral.txt");
    std::fs::write(&path,source.as_bytes()).unwrap();
    println!("legacy-large fixture bytes={} rows=90000 unique_rows={UNIQUE} encoding=UTF8 eol=LF executor=GPUI-TestDispatcher",source.len());
    for (index,case) in CASES.iter().enumerate() {
        let _seed=override_seed(1); let expected=golden(index,&source,&payload);
        with_source("",|workspace,visual| {
            open_large(workspace,visual,&path,&source); assert_eq!(std::fs::read(&path).unwrap(),source.as_bytes());
            let before=state(workspace,visual);
            let before_rope=visual.update(|_,cx| workspace.read(cx).buffer().rope.clone());
            visual.dispatch_action(MacroStartRecording);
            let start=Instant::now(); let foreground=dispatch(case,visual);
            visual.update(|_,cx| { let w=workspace.read(cx); assert!(w.pending_prepared_edit.is_some());
                assert!(w.buffer().rope.is_instance(&before_rope)); assert!(w.tabs[0].undo_stack.is_empty()); });
            visual.run_until_parked(); let wall=start.elapsed();
            report(workspace,visual,case,"action",foreground,wall);
            visual.dispatch_action(MacroStopRecording);
            let transformed=text(workspace,visual); assert_rows(&transformed,expected.as_deref(),&source,&payload);
            assert_eq!(state(workspace,visual).undo,1); assert_eq!(macros::current().len(),1);
            let mut events=macros::current(); assert_eq!(format!("{:?}",events[0]),case.event);
            events.extend([macros::MacroEvent::MoveDocEnd,macros::MacroEvent::InsertText("\nTAIL".into())]);
            let saved=macros::saved_round_trip_for_test(&directory.join("macros.toml"),&BTreeMap::from([("large".into(),events)])).unwrap();
            for undo in [true,false,true] {
                let start=Instant::now();
                if undo { visual.dispatch_action(EditUndo); } else { visual.dispatch_action(EditRedo); }
                let foreground=start.elapsed(); visual.run_until_parked(); let wall=start.elapsed();
                report(workspace,visual,case,if undo {"undo"} else {"redo"},foreground,wall);
                let after=state(workspace,visual); assert_eq!(after.text,if undo {source.as_str()} else {transformed.as_str()});
                assert_eq!((after.encoding,after.eol),(before.encoding,before.eol));
                if undo { assert_eq!(after.selection,before.selection); assert_eq!(after.block,before.block); }
            }
            // Keep a real earlier scalar history entry while combining the
            // prepared command and saved scalar continuation into one step.
            visual.update_window_entity(workspace,|w,_,cx| { w.replace_byte_range(0..1,"0",cx); w.macro_capture_turns.clear(); });
            let prior=state(workspace,visual); assert_eq!(prior.text,source); assert_eq!(prior.undo,1);
            let start=Instant::now();
            visual.update_window_entity(workspace,|w,window,cx| {
                w.replay_macro(saved.get("large").unwrap(),1,window,cx); assert!(w.pending_prepared_edit.is_some());
            });
            let foreground=start.elapsed(); visual.run_until_parked(); let wall=start.elapsed();
            report(workspace,visual,case,"macro",foreground,wall);
            let completed=state(workspace,visual); assert_eq!(completed.text,format!("{transformed}\nTAIL")); assert_eq!(completed.undo,2);
            visual.update(|_,cx| { let w=workspace.read(cx); assert!(w.pending_prepared_edit.is_none() && w.active_macro_playback_id().is_none());
                assert!(!w.macro_capture_turns.is_empty()); assert!(w.macro_capture_turns.iter().all(|&(bytes,ops)|bytes<=64*1024 && ops<=256));
                println!("legacy-large id={} stage=macro-complete wall_ms={:.3} capture_turns={} prefix_undo=1 macro_undo=1",case.id,wall.as_secs_f64()*1000.,w.macro_capture_turns.len()); });
            visual.dispatch_action(EditUndo); visual.run_until_parked(); let undone=state(workspace,visual);
            assert_eq!(undone.text,prior.text); assert_eq!(undone.selection,prior.selection); assert_eq!(undone.block,prior.block);
            assert!(undone.history.0==prior.history.0); assert_eq!(undone.undo,1);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace,visual),completed.text);
            assert_eq!(state(workspace,visual).undo,2);
            assert_eq!(std::fs::read(&path).unwrap(),source.as_bytes());
        });
    }
}

#[test]
#[ignore = "default-release >10 MiB legacy cancel/stale controls before worker and before publication"]
fn legacy_release_large_cancel_and_typed_undo_aba() {
    let source=large_source(&"é🙂payload ".repeat(9),2); assert_eq!(source.len(),12_059_999);
    let directory=crate::persist::TempDir::new("legacy-large-refusals"); let path=directory.join("neutral.txt");
    std::fs::write(&path,source.as_bytes()).unwrap();
    for case in &CASES {
        for publish in [false,true] {
            for cancel in [false,true] {
                let _seed=override_seed(1);
                with_source("",|workspace,visual| {
                    open_large(workspace,visual,&path,&source);
                    let (send,receive)=futures::channel::oneshot::channel();
                    visual.update_window_entity(workspace,|w,_,_| {
                        if publish { w.prepared_test_publish_gate=Some(receive); } else { w.prepared_test_gate=Some(receive); }
                    });
                    visual.dispatch_action(MacroStartRecording); dispatch(case,visual); visual.run_until_parked();
                    visual.update(|_,cx| assert!(workspace.read(cx).pending_prepared_edit.is_some()));
                    assert_eq!(text(workspace,visual),source);
                    if cancel { visual.dispatch_action(EditFindClose); visual.run_until_parked(); }
                    else {
                        visual.simulate_input("x"); visual.run_until_parked(); assert_eq!(text(workspace,visual),format!("x{source}"));
                        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace,visual),source);
                    }
                    let before=state(workspace,visual); let start=Instant::now();
                    send.send(()).unwrap(); visual.run_until_parked(); let wall=start.elapsed();
                    assert!(state(workspace,visual)==before); assert_eq!(std::fs::read(&path).unwrap(),source.as_bytes());
                    visual.update(|_,cx| assert!(workspace.read(cx).pending_prepared_edit.is_none()));
                    visual.dispatch_action(MacroStopRecording);
                    let events=macros::current(); assert_eq!(format!("{:?}",events[0]),case.event);
                    // Input/Undo may record their own events after the accepted
                    // operation; neither cancellation nor refusal records it twice.
                    assert_eq!(events.iter().filter(|event|format!("{event:?}")==case.event).count(),1);
                    println!("legacy-large-refusal id={} stage={} control={} release_wall_ms={:.3} unchanged=true",case.id,
                        if publish {"before-publish"} else {"before-worker"},if cancel {"escape"} else {"typed-undo-aba"},wall.as_secs_f64()*1000.);
                });
            }
        }
    }
}
