// Per-thread playback data for handler tests. No disk or shared recorder changes.
use super::MacroEvent;
use std::{cell::RefCell, collections::BTreeMap, marker::PhantomData, rc::Rc};

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
