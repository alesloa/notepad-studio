//! One worker, latest metadata request, one retired batch and one output slot.
//! Only a generation token reaches the UI; large rejected/closed outputs drop
//! inside run on the background executor, before another request activates.
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use futures::{channel::mpsc, StreamExt};
use uuid::Uuid;
use super::{find_current_budget::{Credit, Pool}, find_current_projection::ProjectionError};

pub(super) struct Request<R> {
    pub(super) id: Uuid,
    pub(super) value: R,
    pub(super) cancel: Arc<AtomicBool>,
}
struct Active { id: Uuid, cancel: Arc<AtomicBool> }
struct State<R, O, D> {
    latest: Option<Request<R>>,
    active: Option<Active>,
    output: Option<(Uuid, O)>,
    retired: Option<D>,
    closed: bool,
}
// Data precedes its fixed-control credit. Request/output buffers own their own
// capacity credits; framework channel/Arc allocator internals are excluded.
pub(super) struct Controller<R, O, D> {
    state: Mutex<State<R, O, D>>,
    wake: Mutex<mpsc::Sender<()>>,
    _control_credit: Credit,
}
enum Work<R, O, D> { Request(Request<R>), Retire(D), Discard(O), Park, Stop }

impl<R: Send + 'static, O: Send + 'static, D: Send + 'static> Controller<R, O, D> {
    pub(super) fn new(pool: &Pool) -> Result<(Arc<Self>, mpsc::Receiver<()>), ProjectionError> {
        let credit = pool.charge(std::mem::size_of::<Self>())?;
        // futures mpsc adds one reserved slot per sender. There is exactly one
        // sender and buffer0, so retained wake tokens are bounded to one.
        let (sender, receiver) = mpsc::channel(0);
        Ok((Arc::new(Self {
            state: Mutex::new(State { latest: None, active: None, output: None, retired: None, closed: false }),
            wake: Mutex::new(sender), _control_credit: credit,
        }), receiver))
    }
    fn signal(&self) {
        if let Ok(mut sender) = self.wake.lock() {
            // A full slot already guarantees a subsequent state inspection.
            let _ = sender.try_send(());
        }
    }
    pub(super) fn request(&self, value: R) -> Option<Uuid> {
        let id = Uuid::new_v4();
        {
            let Ok(mut state) = self.state.lock() else { return None; };
            if state.closed { return None; }
            if let Some(active) = state.active.as_ref() { active.cancel.store(true, Ordering::Release); }
            // R is bounded metadata only. Never put a captured projection here.
            state.latest = Some(Request { id, value, cancel: Arc::new(AtomicBool::new(false)) });
        }
        self.signal();
        Some(id)
    }
    pub(super) fn retire(&self, batch: D) -> Result<(), D> {
        {
            let Ok(mut state) = self.state.lock() else { return Err(batch); };
            if state.closed || state.retired.is_some() { return Err(batch); }
            state.retired = Some(batch);
        }
        self.signal();
        Ok(())
    }
    pub(super) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.latest = None;
            if let Some(active) = state.active.as_ref() { active.cancel.store(true, Ordering::Release); }
        }
        self.signal();
    }
    fn next(&self) -> Work<R, O, D> {
        let Ok(mut state) = self.state.lock() else { return Work::Stop; };
        if let Some(batch) = state.retired.take() { return Work::Retire(batch); }
        let invalid = state.closed || state.latest.is_some()
            || state.active.as_ref().is_some_and(|a| a.cancel.load(Ordering::Acquire));
        if invalid {
            if let Some((_, output)) = state.output.take() {
                state.active = None;
                return Work::Discard(output);
            }
        }
        if state.closed { return Work::Stop; }
        if state.active.is_some() { return Work::Park; }
        let Some(request) = state.latest.take() else { return Work::Park; };
        state.active = Some(Active { id: request.id, cancel: Arc::clone(&request.cancel) });
        Work::Request(request)
    }
    fn complete(&self, id: Uuid, output: O) -> Result<(), O> {
        let Ok(mut state) = self.state.lock() else { return Err(output); };
        let valid = !state.closed && state.latest.is_none() && state.output.is_none()
            && state.active.as_ref().is_some_and(|a| a.id == id && !a.cancel.load(Ordering::Acquire));
        if valid { state.output = Some((id, output)); return Ok(()); }
        if state.active.as_ref().is_some_and(|a| a.id == id) { state.active = None; }
        Err(output)
    }
    /// The caller checks full source/query/view ownership before bounded work.
    pub(super) fn with_output<T>(&self, id: Uuid, update: impl FnOnce(&mut O) -> T) -> Option<T> {
        let Ok(mut state) = self.state.lock() else { return None; };
        if state.closed || state.latest.is_some()
            || state.active.as_ref().is_none_or(|a| a.id != id || a.cancel.load(Ordering::Acquire)) {
            return None;
        }
        let (owner, output) = state.output.as_mut()?;
        (*owner == id).then(|| update(output))
    }
    pub(super) fn accept(&self, id: Uuid) -> Option<O> {
        let output = {
            let Ok(mut state) = self.state.lock() else { return None; };
            if state.closed || state.latest.is_some()
                || state.active.as_ref().is_none_or(|a| a.id != id || a.cancel.load(Ordering::Acquire)) {
                return None;
            }
            if state.output.as_ref().is_none_or(|(owner, _)| *owner != id) { return None; }
            state.active = None;
            state.output.take().map(|(_, output)| output)
        };
        self.signal();
        output
    }
    pub(super) fn reject(&self, id: Uuid) {
        if let Ok(state) = self.state.lock() {
            if let Some(active) = state.active.as_ref().filter(|a| a.id == id) {
                active.cancel.store(true, Ordering::Release);
            }
        }
        self.signal();
    }
    pub(super) async fn run<F, Fut>(
        &self, mut wakes: mpsc::Receiver<()>, mut execute: F, mut notify: impl FnMut(Uuid),
    ) where F: FnMut(Request<R>) -> Fut, Fut: std::future::Future<Output = O> {
        while wakes.next().await.is_some() {
            loop {
                match self.next() {
                    Work::Retire(batch) => drop(batch),
                    Work::Discard(output) => drop(output),
                    Work::Request(request) => {
                        let id = request.id;
                        let output = execute(request).await;
                        match self.complete(id, output) {
                            Ok(()) => notify(id),
                            Err(output) => drop(output),
                        }
                    }
                    Work::Park => break,
                    Work::Stop => return,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::find_current_budget::Ledger;
    use futures::channel::oneshot;
    use std::{thread, time::Duration};

    struct Dropped(std::sync::mpsc::Sender<thread::ThreadId>);
    impl Drop for Dropped { fn drop(&mut self) { let _ = self.0.send(thread::current().id()); } }
    fn fixture() -> (Arc<Controller<usize, Dropped, Dropped>>, mpsc::Receiver<()>) {
        Controller::new(&Ledger::default().pool(1024 * 1024)).unwrap()
    }
    #[test]
    fn held_worker_coalesces_storm_and_drops_cancelled_output_and_retired_on_worker() {
        let (controller, wakes) = fixture();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (drop_tx, drop_rx) = std::sync::mpsc::channel();
        let (notice_tx, notice_rx) = std::sync::mpsc::channel();
        let (release, hold) = oneshot::channel();
        let worker_controller = Arc::clone(&controller);
        let output_drop = drop_tx.clone();
        let worker = thread::spawn(move || {
            let mut hold = Some(hold);
            futures::executor::block_on(worker_controller.run(wakes, move |request| {
                let hold = hold.take(); let started = started_tx.clone(); let dropped = output_drop.clone();
                async move {
                    started.send(request.value).unwrap();
                    if let Some(hold) = hold { let _ = hold.await; }
                    Dropped(dropped)
                }
            }, |id| { notice_tx.send(id).unwrap(); }));
        });
        controller.request(0).unwrap();
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        controller.retire(Dropped(drop_tx.clone())).ok().unwrap();
        let second = controller.retire(Dropped(drop_tx.clone())).err().unwrap();
        // Failed admission returns ownership; test disposes its intentionally
        // extra synthetic batch here. Production checks its single slot first.
        drop(second); let _ = drop_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut latest = Uuid::nil();
        for value in 1..=1000 { latest = controller.request(value).unwrap(); }
        {
            let state = controller.state.lock().unwrap();
            assert_eq!(state.latest.as_ref().unwrap().value, 1000);
            assert!(state.active.is_some()); assert!(state.retired.is_some()); assert!(state.output.is_none());
        }
        release.send(()).unwrap();
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1000);
        assert_eq!(notice_rx.recv_timeout(Duration::from_secs(5)).unwrap(), latest);
        assert!(started_rx.try_recv().is_err());
        for _ in 0..2 { assert_eq!(drop_rx.recv_timeout(Duration::from_secs(5)).unwrap(), worker.thread().id()); }
        assert!(controller.accept(Uuid::new_v4()).is_none());
        controller.reject(latest);
        assert_eq!(drop_rx.recv_timeout(Duration::from_secs(5)).unwrap(), worker.thread().id());
        controller.close(); worker.join().unwrap();
    }
    #[test]
    fn gpui_background_driver_holds_old_work_and_accepts_only_latest_output() {
        use gpui::AppContext;
        let mut cx = gpui::TestAppContext::single();
        let ledger = Ledger::default();
        let (controller, wakes) = Controller::<usize, usize, usize>::new(&ledger.pool(1024 * 1024)).unwrap();
        let actor = Arc::clone(&controller);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let worker_trace = Arc::clone(&trace);
        let notices = Arc::new(Mutex::new(Vec::new()));
        let worker_notices = Arc::clone(&notices);
        let (release, hold) = oneshot::channel();
        cx.background_spawn(async move {
            let mut hold = Some(hold);
            actor.run(wakes, move |request| {
                let hold = hold.take(); let trace = Arc::clone(&worker_trace);
                async move {
                    trace.lock().unwrap().push(request.value);
                    if let Some(hold) = hold { let _ = hold.await; }
                    request.value
                }
            }, move |id| worker_notices.lock().unwrap().push(id)).await;
        }).detach();
        let old = controller.request(1).unwrap(); cx.run_until_parked();
        assert_eq!(*trace.lock().unwrap(), vec![1]);
        let latest = controller.request(2).unwrap(); cx.run_until_parked();
        assert!(controller.with_output(old, |_| ()).is_none());
        assert!(notices.lock().unwrap().is_empty());
        release.send(()).unwrap(); cx.run_until_parked();
        assert_eq!(*trace.lock().unwrap(), vec![1, 2]);
        assert_eq!(*notices.lock().unwrap(), vec![latest]);
        assert_eq!(controller.with_output(latest, |value| { *value += 1; *value }), Some(3));
        assert_eq!(controller.accept(latest), Some(3));
        assert!(controller.accept(latest).is_none());
        controller.close(); cx.run_until_parked(); drop(controller);
        assert_eq!(ledger.used(), 0); cx.quit();
    }
    #[test]
    fn window_close_keeps_held_job_cleanup_off_the_ui_thread() {
        let (controller, wakes) = fixture();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (drop_tx, drop_rx) = std::sync::mpsc::channel();
        let (release, hold) = oneshot::channel();
        let actor = Arc::clone(&controller);
        let worker = thread::spawn(move || {
            let mut hold = Some(hold);
            futures::executor::block_on(actor.run(wakes, move |_| {
                let hold = hold.take(); let started = started_tx.clone(); let drop_tx = drop_tx.clone();
                async move { started.send(()).unwrap(); if let Some(hold) = hold { let _ = hold.await; } Dropped(drop_tx) }
            }, |_| panic!("closed output must not publish")));
        });
        controller.request(1).unwrap(); started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        controller.close(); assert!(controller.request(2).is_none());
        release.send(()).unwrap();
        assert_eq!(drop_rx.recv_timeout(Duration::from_secs(5)).unwrap(), worker.thread().id());
        worker.join().unwrap();
    }
}
