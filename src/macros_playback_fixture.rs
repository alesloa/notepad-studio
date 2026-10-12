// Per-thread playback data for handler tests. No disk or shared recorder changes.
use super::MacroEvent;
use std::{cell::RefCell, collections::BTreeMap, marker::PhantomData, rc::Rc};

struct Execution { recorder: super::MacroRecorder }

thread_local! {
    static EXECUTION: RefCell<Option<Execution>> = const { RefCell::new(None) };
}

pub(crate) struct ExecutionGuard { previous: Option<Execution>, same_thread: PhantomData<Rc<()>> }
impl Drop for ExecutionGuard {
    fn drop(&mut self) { EXECUTION.with(|slot| *slot.borrow_mut() = self.previous.take()); }
}

pub(crate) fn override_execution_for_test(recording: bool) -> ExecutionGuard {
    let recorder = super::MacroRecorder { recording, ..Default::default() };
    ExecutionGuard { previous: EXECUTION.with(|slot| slot.replace(Some(Execution { recorder }))),
        same_thread: PhantomData }
}

pub(super) fn execution_active() -> Option<bool> {
    EXECUTION.with(|slot| slot.borrow().as_ref().map(|execution| execution.recorder.recording || execution.recorder.replaying))
}

pub(super) fn set_replaying(value: bool) -> Option<bool> {
    EXECUTION.with(|slot| {
        let mut state = slot.borrow_mut();
        let execution = state.as_mut()?;
        let previous = execution.recorder.replaying;
        execution.recorder.replaying = value;
        Some(previous)
    })
}

pub(super) fn replaying() -> Option<bool> {
    EXECUTION.with(|slot| slot.borrow().as_ref().map(|execution| execution.recorder.replaying))
}

pub(super) fn recording() -> Option<bool> {
    EXECUTION.with(|slot| slot.borrow().as_ref().map(|execution| execution.recorder.recording))
}

pub(super) fn start_recording() -> Option<bool> {
    EXECUTION.with(|slot| {
        let mut state = slot.borrow_mut();
        let recorder = &mut state.as_mut()?.recorder;
        if !recorder.recording { recorder.in_progress.clear(); recorder.recording = true; }
        Some(recorder.recording)
    })
}

pub(super) fn stop_recording() -> Option<bool> {
    EXECUTION.with(|slot| {
        let mut state = slot.borrow_mut();
        let recorder = &mut state.as_mut()?.recorder;
        if recorder.recording {
            recorder.current = std::mem::take(&mut recorder.in_progress);
            recorder.recording = false;
        }
        Some(recorder.recording)
    })
}

pub(super) fn toggle_recording() -> Option<bool> {
    if recording()? { stop_recording() } else { start_recording() }
}

pub(super) fn record(event: &MacroEvent) -> bool {
    EXECUTION.with(|slot| {
        let mut state = slot.borrow_mut();
        let Some(execution) = state.as_mut() else { return false; };
        execution.recorder.record_event(event.clone());
        true
    })
}

pub(crate) fn execution_current() -> Option<Vec<MacroEvent>> {
    EXECUTION.with(|slot| slot.borrow().as_ref().map(|execution| execution.recorder.current.clone()))
}

#[test]
fn execution_fixture_restores_nested_state_and_is_thread_local() {
    trait AmbiguousIfSend<A> { fn check() {} }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
    let _ = <ExecutionGuard as AmbiguousIfSend<_>>::check;
    assert_eq!(execution_active(), None);
    {
        let _outer = override_execution_for_test(true);
        assert_eq!(execution_active(), Some(true));
        {
            let _inner = override_execution_for_test(false);
            assert_eq!(execution_active(), Some(false));
            assert!(!super::set_replaying(true));
            assert!(super::execution_active());
            assert!(super::set_replaying(false));
            assert!(super::start_recording());
            super::record(MacroEvent::InsertText("日本".into()));
            assert!(!super::stop_recording());
            assert!(matches!(super::current().as_slice(), [MacroEvent::InsertText(text)] if text == "日本"));
        }
        assert_eq!(execution_active(), Some(true));
        assert_eq!(std::thread::spawn(execution_active).join().unwrap(), None);
    }
    assert_eq!(execution_active(), None);
}

struct PlaybackData {
    current: Vec<MacroEvent>,
    saved: BTreeMap<String, Vec<MacroEvent>>,
}

thread_local! {
    static PLAYBACK: RefCell<Option<PlaybackData>> = const { RefCell::new(None) };
}

pub(crate) struct PlaybackGuard {
    previous: Option<PlaybackData>,
    same_thread: PhantomData<Rc<()>>,
}

impl Drop for PlaybackGuard {
    fn drop(&mut self) {
        PLAYBACK.with(|current| *current.borrow_mut() = self.previous.take());
    }
}

pub(crate) fn override_for_test(
    current: Vec<MacroEvent>,
    saved: BTreeMap<String, Vec<MacroEvent>>,
) -> PlaybackGuard {
    PlaybackGuard {
        previous: PLAYBACK.with(|slot| slot.replace(Some(PlaybackData { current, saved }))),
        same_thread: PhantomData,
    }
}

pub(super) fn current() -> Option<Vec<MacroEvent>> {
    PLAYBACK.with(|slot| slot.borrow().as_ref().map(|data| data.current.clone()))
}

pub(super) fn saved_events(name: &str) -> Option<Option<Vec<MacroEvent>>> {
    PLAYBACK.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|data| data.saved.get(name).cloned())
    })
}

pub(super) fn saved_snapshot() -> Option<Vec<(String, usize)>> {
    PLAYBACK.with(|slot| {
        slot.borrow().as_ref().map(|data| {
            data.saved
                .iter()
                .map(|(name, events)| (name.clone(), events.len()))
                .collect()
        })
    })
}

#[test]
fn playback_fixture_restores_nested_data_and_stays_on_its_thread() {
    assert!(current().is_none());
    {
        let _outer = override_for_test(
            vec![MacroEvent::InsertText("outer".into())],
            BTreeMap::new(),
        );
        assert!(
            matches!(current().and_then(|mut events| events.pop()), Some(MacroEvent::InsertText(text)) if text == "outer")
        );
        assert!(
            std::thread::spawn(|| current().is_none())
                .join()
                .expect("fixture thread")
        );
        {
            let saved =
                BTreeMap::from([("named".into(), vec![MacroEvent::InsertText("saved".into())])]);
            let _inner = override_for_test(vec![MacroEvent::InsertText("inner".into())], saved);
            assert!(
                matches!(current().and_then(|mut events| events.pop()), Some(MacroEvent::InsertText(text)) if text == "inner")
            );
            assert!(saved_events("named").flatten().is_some());
            assert!(saved_events("missing").is_some_and(|events| events.is_none()));
            assert_eq!(saved_snapshot(), Some(vec![("named".into(), 1)]));
        }
        assert!(
            matches!(current().and_then(|mut events| events.pop()), Some(MacroEvent::InsertText(text)) if text == "outer")
        );
    }
    assert!(current().is_none());
}
