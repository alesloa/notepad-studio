use std::{collections::VecDeque, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}};
use gpui::{App, BackgroundExecutor, Global};

const MAX_WORKERS: usize = 2;
const MAX_QUEUE: usize = 64;
pub(super) const MAX_MEMORY: usize = 128 * 1024 * 1024;
pub(super) const MEMORY: &str = "Hash tools exceed the shared 128 MiB memory limit; clear a large hash input or choose fewer files";

#[derive(Default)]
pub(super) struct Budget(Arc<AtomicUsize>);
impl Clone for Budget { fn clone(&self) -> Self { Self(self.0.clone()) } }
impl Global for Budget {}
pub(super) struct Permit { budget: Budget, bytes: usize }
impl Permit {
    pub(super) fn shrink(&mut self, bytes: usize) {
        if bytes < self.bytes { self.budget.0.fetch_sub(self.bytes - bytes, Ordering::Relaxed); self.bytes = bytes; }
    }
}
impl Budget {
    #[cfg(test)]
    pub(super) fn used(&self) -> usize { self.0.load(Ordering::Relaxed) }
    pub(super) fn reserve(&self, bytes: usize) -> Result<Permit, &'static str> {
        self.0.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(bytes).filter(|&n| n <= MAX_MEMORY))
            .map_err(|_| MEMORY)?;
        Ok(Permit { budget: self.clone(), bytes })
    }
}
impl Drop for Permit { fn drop(&mut self) { self.budget.0.fetch_sub(self.bytes, Ordering::Relaxed); } }

pub(super) struct Delivery<T: Send + 'static> { value: Option<T>, executor: BackgroundExecutor }
impl<T: Send + 'static> Delivery<T> {
    pub(super) fn new(value: T, executor: BackgroundExecutor) -> Self { Self { value: Some(value), executor } }
    pub(super) fn take(&mut self) -> Option<T> { self.value.take() }
    pub(super) fn as_ref(&self) -> Option<&T> { self.value.as_ref() }
}
impl<T: Send + 'static> Drop for Delivery<T> {
    fn drop(&mut self) { if let Some(value) = self.value.take() { dispose(value, &self.executor); } }
}
pub(super) fn dispose<T: Send + 'static>(value: T, executor: &BackgroundExecutor) {
    executor.spawn(async move { drop(value); }).detach();
}

struct Queued { cancel: Arc<AtomicBool>, start: Box<dyn FnOnce(&mut App)> }
#[derive(Default)]
struct Queue { active: usize, waiting: VecDeque<Queued> }
impl Global for Queue {}

// Slots cover capture, hashing and delivery; cancelled closures are also retired
// off-thread because they can own the last reference to a large Rope.
pub(super) fn run<T: Send + 'static>(cx: &mut App, cancel: Arc<AtomicBool>,
    work: impl FnOnce() -> T + Send + 'static, finish: impl FnOnce(T, &mut App) + 'static) -> Result<(), &'static str> {
    let executor = cx.background_executor().clone();
    let queue = cx.default_global::<Queue>();
    let mut index = 0;
    while index < queue.waiting.len() {
        if queue.waiting.get(index).is_some_and(|job| job.cancel.load(Ordering::Relaxed)) {
            queue.waiting.remove(index);
        } else { index += 1; }
    }
    if queue.waiting.len() >= MAX_QUEUE || queue.waiting.try_reserve(1).is_err() {
        dispose(work, &executor); return Err("Hash queue is full; cancel a pending hash and try again");
    }
    #[cfg(test)]
    let gate = { let state = cx.default_global::<TestGate>(); state.1 += 1; state.0.take() };
    #[cfg(test)]
    let publish_gate = cx.default_global::<TestGate>().2.take();
    let mut work = Delivery::new(work, executor.clone());
    let start = Box::new(move |cx: &mut App| {
        let Some(work) = work.take() else { return; };
        let disposer = executor.clone();
        let worker = executor.spawn(async move {
            #[cfg(test)]
            if let Some(gate) = gate { let _ = gate.await; }
            Delivery::new(work(), disposer)
        });
        cx.spawn(async move |cx| {
            let mut delivery = worker.await;
            #[cfg(test)]
            if let Some(gate) = publish_gate { let _ = gate.await; }
            cx.update(|cx| {
                if let Some(value) = delivery.take() { finish(value, cx); }
                let queue = cx.default_global::<Queue>(); queue.active = queue.active.saturating_sub(1);
                pump(cx);
            });
        }).detach();
    });
    cx.default_global::<Queue>().waiting.push_back(Queued { cancel, start });
    pump(cx); Ok(())
}
fn pump(cx: &mut App) {
    loop {
        let queue = cx.default_global::<Queue>();
        if queue.active >= MAX_WORKERS { return; }
        let Some(job) = queue.waiting.pop_front() else { return; };
        if job.cancel.load(Ordering::Relaxed) { continue; }
        queue.active += 1; (job.start)(cx);
    }
}
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestGate(pub(crate) Option<futures::channel::oneshot::Receiver<()>>, pub(crate) usize,
    pub(crate) Option<futures::channel::oneshot::Receiver<()>>);
#[cfg(test)]
impl Global for TestGate {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_tools_budget_checks_cumulative_cost_and_releases_exact_lease() {
        let budget = Budget::default(); let lease = budget.reserve(MAX_MEMORY - 1).unwrap();
        assert!(budget.reserve(2).is_err()); assert!(budget.reserve(usize::MAX).is_err());
        let last = budget.reserve(1).unwrap(); drop(lease);
        assert!(budget.reserve(MAX_MEMORY - 1).is_ok()); drop(last);
        assert_eq!(budget.0.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn hash_tools_queue_bounds_active_workers_and_disposes_cancelled_queued_work() {
        crate::app::menus::hash_tools_tests::with_editor(|_,visual| {
            let mut releases=Vec::new();let cancelled=Arc::new(AtomicBool::new(false));
            let budget=Budget::default();let ran=Arc::new(AtomicUsize::new(0));
            for index in 0..3 {
                let(tx,rx)=futures::channel::oneshot::channel();releases.push(tx);let permit=budget.reserve(4096).unwrap();let ran=ran.clone();
                let cancel=if index==2 {cancelled.clone()}else{Arc::new(AtomicBool::new(false))};
                visual.update(|_,cx|{cx.default_global::<TestGate>().0=Some(rx);run(cx,cancel,move||{drop(permit);ran.fetch_add(1,Ordering::Relaxed);},|_,_|{}).unwrap();});
            }
            visual.run_until_parked();visual.update(|_,cx|{let queue=cx.default_global::<Queue>();assert_eq!((queue.active,queue.waiting.len()),(2,1));});
            assert_eq!(budget.used(),3*4096);cancelled.store(true,Ordering::Relaxed);
            visual.update(|_,cx|run(cx,Arc::new(AtomicBool::new(false)),||{},|_,_|{}).unwrap());visual.run_until_parked();assert_eq!(budget.used(),2*4096);
            for release in releases.into_iter().take(2) {release.send(()).unwrap();}visual.run_until_parked();assert_eq!(ran.load(Ordering::Relaxed),2);assert_eq!(budget.used(),0);
            visual.update(|_,cx|{let queue=cx.default_global::<Queue>();assert_eq!((queue.active,queue.waiting.len()),(0,0));});
        });
    }
}
