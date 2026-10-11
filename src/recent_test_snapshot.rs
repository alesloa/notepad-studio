// Neutral menu snapshots for tests; MRU mutation and persistence stay unchanged.
use std::{cell::RefCell, marker::PhantomData, path::PathBuf, rc::Rc};

thread_local! {
    static SNAPSHOT: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

pub(crate) struct SnapshotGuard {
    previous: Option<Vec<PathBuf>>,
    same_thread: PhantomData<Rc<()>>,
}

impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        SNAPSHOT.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

pub(crate) fn override_for_test(paths: Vec<PathBuf>) -> SnapshotGuard {
    SnapshotGuard {
        previous: SNAPSHOT.with(|slot| slot.replace(Some(paths))),
        same_thread: PhantomData,
    }
}

pub(super) fn snapshot() -> Option<Vec<PathBuf>> {
    SNAPSHOT.with(|slot| slot.borrow().clone())
}

#[test]
fn snapshot_guard_restores_nested_values_and_does_not_cross_threads() {
    assert!(snapshot().is_none());
    {
        let _outer = override_for_test(vec!["neutral-café.txt".into()]);
        assert_eq!(super::snapshot(), vec![PathBuf::from("neutral-café.txt")]);
        assert!(std::thread::spawn(|| snapshot().is_none()).join().unwrap());
        {
            let _inner = override_for_test(Vec::new());
            assert!(super::snapshot().is_empty());
        }
        assert_eq!(super::snapshot(), vec![PathBuf::from("neutral-café.txt")]);
    }
    // Inspect the fixture rather than loading the real MRU after restoration.
    assert!(snapshot().is_none());
}
