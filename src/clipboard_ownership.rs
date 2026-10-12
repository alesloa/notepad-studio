// Application-side ordering for delayed clipboard work. External OS writers
// are outside this token; all application writes use write_now or publish.
use std::{cell::Cell, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use gpui::{App, ClipboardItem};

#[derive(Clone)]
pub(crate) struct Request { marker: Arc<()>, cancel: Arc<AtomicBool> }
thread_local! {
    static CURRENT: Cell<Option<Request>> = const { Cell::new(None) };
}
pub(crate) fn claim(cancel: Arc<AtomicBool>) -> Request {
    let request = Request { marker: Arc::new(()), cancel };
    if CURRENT.try_with(|slot| {
        if let Some(previous) = slot.replace(Some(request.clone())) { previous.cancel.store(true, Ordering::Relaxed); }
    }).is_err() { request.cancel.store(true, Ordering::Relaxed); }
    request
}
pub(crate) fn supersede() { claim(Arc::new(AtomicBool::new(false))); }
pub(crate) fn is_current(request: &Request) -> bool {
    CURRENT.try_with(|slot| {
        let current = slot.take();
        let valid = current.as_ref().is_some_and(|current| Arc::ptr_eq(&current.marker, &request.marker));
        slot.set(current); valid
    }).unwrap_or(false)
}
pub(crate) fn write_now(cx: &mut App, item: ClipboardItem) {
    supersede(); cx.write_to_clipboard(item);
}
pub(crate) fn publish(request: &Request, cx: &mut App, item: ClipboardItem) -> bool {
    if !is_current(request) { return false; }
    write_now(cx, item); true
}

#[cfg(test)]
pub(crate) struct TestScope { previous: Option<Request> }
#[cfg(test)]
pub(crate) fn override_for_test() -> TestScope {
    TestScope { previous: CURRENT.try_with(Cell::take).ok().flatten() }
}
#[cfg(test)]
impl Drop for TestScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _ = CURRENT.try_with(|slot| {
            if let Some(current) = slot.replace(previous) { current.cancel.store(true, Ordering::Relaxed); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_ownership_new_intent_cancels_previous_work_and_completion_does_not_revive_it() {
        let _scope = override_for_test(); let first_cancel = Arc::new(AtomicBool::new(false));
        let first = claim(first_cancel.clone()); assert!(is_current(&first));
        let second = claim(Arc::new(AtomicBool::new(false)));
        assert!(first_cancel.load(Ordering::Relaxed)); assert!(!is_current(&first)); assert!(is_current(&second));
        supersede(); assert!(!is_current(&second)); assert!(!is_current(&first));
    }
    #[test]
    fn clipboard_ownership_scoped_tests_restore_outer_owner_without_cancelling_it() {
        let _scope = override_for_test(); let outer_cancel = Arc::new(AtomicBool::new(false));
        let outer = claim(outer_cancel.clone());
        { let _nested = override_for_test(); let nested = claim(Arc::new(AtomicBool::new(false))); assert!(is_current(&nested)); assert!(!is_current(&outer)); }
        assert!(is_current(&outer)); assert!(!outer_cancel.load(Ordering::Relaxed));
    }
    #[test]
    fn clipboard_ownership_finished_worker_flag_is_separate_from_publication_owner() {
        let _scope = override_for_test(); let cancel = Arc::new(AtomicBool::new(false)); let request = claim(cancel.clone());
        cancel.store(true, Ordering::Relaxed); assert!(is_current(&request));
    }
}
