// Scoped persistence sink: failed settings controls must not write user files.
use std::{cell::{Cell, RefCell}, rc::Rc};

thread_local! {
    static SINK: RefCell<Option<Rc<Cell<usize>>>> = const { RefCell::new(None) };
}

pub(crate) struct Guard {
    current: Rc<Cell<usize>>,
    previous: Option<Rc<Cell<usize>>>,
}

pub(crate) fn observe() -> Guard {
    let current = Rc::new(Cell::new(0));
    let previous = SINK.with(|slot| slot.replace(Some(current.clone())));
    Guard { current, previous }
}

impl Guard {
    pub(crate) fn count(&self) -> usize { self.current.get() }
}

impl Drop for Guard {
    fn drop(&mut self) { SINK.with(|slot| *slot.borrow_mut() = self.previous.take()); }
}

pub(super) fn intercept() -> bool {
    SINK.with(|slot| {
        let current = slot.borrow();
        let Some(current) = current.as_ref() else { return false; };
        current.set(current.get().saturating_add(1));
        true
    })
}

#[test]
fn nested_sinks_restore_and_stay_inactive_without_a_guard() {
    assert!(!intercept());
    let outer = observe();
    assert!(intercept());
    {
        let inner = observe();
        assert!(intercept()); assert!(intercept());
        assert_eq!(inner.count(), 2); assert_eq!(outer.count(), 1);
    }
    assert!(intercept()); assert_eq!(outer.count(), 2);
    drop(outer);
    assert!(!intercept());
}

#[test]
fn scoped_settings_save_updates_the_override_without_disk_writes() {
    use super::{Settings, override_for_test, replace_and_save, snapshot};
    let writes = observe();
    let _outer = override_for_test(Settings { proofread_spelling: false, proofread_grammar: false, ..Default::default() });
    {
        let _inner = override_for_test(Settings::default());
        let mut next = snapshot();
        next.proofread_spelling = false;
        next.proofread_grammar = true;
        next.tab_width = usize::MAX;
        replace_and_save(next);
        let next = snapshot();
        assert!(!next.proofread_spelling); assert!(next.proofread_grammar);
        assert_eq!(next.tab_width, super::TAB_WIDTH_MAX);
        assert_eq!(writes.count(), 0);
    }
    let restored = snapshot();
    assert!(!restored.proofread_spelling); assert!(!restored.proofread_grammar);
    assert_eq!(writes.count(), 0);
}
