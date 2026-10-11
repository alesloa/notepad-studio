// Process-wide MRU persistence. One writer drains a coalesced latest snapshot.
use std::{path::PathBuf, sync::{Arc, Mutex}};
use gpui::BackgroundExecutor;
use super::RecentFiles;

const SNAPSHOT_BYTES: usize = 1024 * 1024;
#[derive(Default)]
struct State { pending: Option<RecentFiles>, running: bool, retired: bool }
pub(super) struct Writer {
    executor: BackgroundExecutor,
    destination: PathBuf,
    state: Mutex<State>,
    disk: Mutex<()>,
    #[cfg(test)]
    pub(super) probe: Probe,
}
impl Writer {
    pub(super) fn new(executor: BackgroundExecutor, destination: PathBuf) -> Arc<Self> {
        Arc::new(Self { executor, destination, state: Mutex::new(State::default()), disk: Mutex::new(()),
            #[cfg(test)] probe: Probe::default() })
    }
    /// Must be called in the same MRU critical section as its mutation.
    pub(super) fn enqueue(&self, snapshot: RecentFiles) -> bool {
        let Ok(mut state) = self.state.lock() else {
            eprintln!("[recent] persistence queue lock was poisoned");
            return false;
        };
        if state.retired { return false; }
        state.pending = Some(snapshot);
        if state.running { false } else { state.running = true; true }
    }
    pub(super) fn start(self: &Arc<Self>) {
        let writer = self.clone();
        self.executor.spawn(async move { writer.drain().await; }).detach();
    }
    async fn drain(self: Arc<Self>) {
        loop {
            let snapshot = {
                let Ok(mut state) = self.state.lock() else {
                    eprintln!("[recent] persistence queue lock was poisoned");
                    return;
                };
                if state.retired { state.running = false; return; }
                let Some(snapshot) = state.pending.take() else {
                    #[cfg(test)]
                    if let Some(gate) = self.probe.idle_gate.lock().unwrap().take() { gate(); }
                    state.running = false;
                    return;
                };
                snapshot
            };
            #[cfg(test)]
            self.probe.before_write(&snapshot).await;
            let Ok(_disk) = self.disk.lock() else {
                eprintln!("[recent] persistence write lock was poisoned");
                if let Ok(mut state) = self.state.lock() { state.running = false; }
                return;
            };
            let retired = self.state.lock().map(|state| state.retired).unwrap_or(true);
            if retired { return; }
            #[cfg(test)]
            if let Some(gate) = self.probe.disk_gate.lock().unwrap().take() { gate(); }
            #[cfg(test)]
            let _context = WriteContext::enter();
            #[cfg(test)]
            let result = if self.probe.fail_next.swap(false, std::sync::atomic::Ordering::AcqRel) {
                Err(std::io::Error::other("synthetic recent write failure"))
            } else { super::save_at(&self.destination, &snapshot) };
            #[cfg(not(test))]
            let result = super::save_at(&self.destination, &snapshot);
            if let Err(error) = &result {
                eprintln!("[recent] couldn't save {}: {error}", self.destination.display());
            }
            #[cfg(test)]
            self.probe.completed.lock().unwrap().push((snapshot.paths, result.is_ok()));
        }
    }
    /// Only the final app-quit observer calls this synchronous barrier.
    /// The disk lock prevents an older worker write from following the final one.
    pub(super) fn finish(&self, latest: impl FnOnce() -> Result<RecentFiles, String>) {
        let Ok(_disk) = self.disk.lock() else {
            eprintln!("[recent] final persistence write lock was poisoned");
            return;
        };
        let pending = {
            let Ok(mut state) = self.state.lock() else {
                eprintln!("[recent] final persistence queue lock was poisoned");
                return;
            };
            state.retired = true;
            state.running = false;
            state.pending.take()
        };
        drop(pending);
        #[cfg(test)]
        let _context = WriteContext::enter();
        let result = latest().map_err(std::io::Error::other)
            .and_then(|snapshot| super::save_at(&self.destination, &snapshot));
        if let Err(error) = &result {
            eprintln!("[recent] final recent-file save failed: {error}");
        }
    }
    #[cfg(test)]
    pub(super) fn run_worker_for_test(self: &Arc<Self>) {
        futures::executor::block_on(self.clone().drain());
    }
    #[cfg(test)]
    pub(super) fn destination(&self) -> &std::path::Path { &self.destination }
    #[cfg(test)]
    pub(super) fn status(&self) -> (bool, bool, Option<Vec<PathBuf>>) {
        let state = self.state.lock().unwrap();
        (state.running, state.retired, state.pending.as_ref().map(|recent| recent.paths.clone()))
    }

}

pub(super) fn bounded_snapshot(recent: &RecentFiles) -> Result<RecentFiles, String> {
    let mut bytes = 0usize;
    if recent.paths.len() > crate::settings::RECENT_FILES_MAX_HARD_CAP as usize {
        return Err("recent-file list exceeds 30 entries".into());
    }
    for path in &recent.paths {
        bytes = bytes.checked_add(path.as_os_str().len()).ok_or("recent path lengths overflow")?;
        if bytes > SNAPSHOT_BYTES { return Err("recent-file paths exceed the 1 MiB persistence limit".into()); }
    }
    let mut paths = Vec::new();
    paths.try_reserve_exact(recent.paths.len()).map_err(|_| "could not allocate recent-file snapshot")?;
    let mut retained = 0usize;
    for source in &recent.paths {
        let mut path = PathBuf::new();
        path.try_reserve_exact(source.as_os_str().len()).map_err(|_| "could not copy recent-file path")?;
        path.push(source);
        retained = retained.checked_add(path.capacity()).ok_or("recent path capacities overflow")?;
        if retained > SNAPSHOT_BYTES { return Err("recent-file snapshot capacity exceeds the 1 MiB limit".into()); }
        paths.push(path);
    }
    Ok(RecentFiles { paths })
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct Probe {
    pub(super) gate: Mutex<Option<futures::channel::oneshot::Receiver<()>>>,
    pub(super) disk_gate: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub(super) idle_gate: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub(super) started: Mutex<Vec<Vec<PathBuf>>>,
    pub(super) completed: Mutex<Vec<(Vec<PathBuf>, bool)>>,
    pub(super) inline: std::sync::atomic::AtomicUsize,
    pub(super) fail_next: std::sync::atomic::AtomicBool,
}
#[cfg(test)]
impl Probe {
    async fn before_write(&self, recent: &RecentFiles) {
        self.started.lock().unwrap().push(recent.paths.clone());
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate { let _ = gate.await; }
    }
}

pub(super) fn queue(writer: &Arc<Writer>, recent: &RecentFiles) -> Option<Arc<Writer>> {
    match bounded_snapshot(recent) {
        Ok(snapshot) => writer.enqueue(snapshot).then(|| writer.clone()),
        Err(error) => { eprintln!("[recent] could not queue persistence: {error}"); None }
    }
}

#[cfg(test)]
thread_local! { static CONTROLLED_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
pub(super) fn controlled_write() -> bool { CONTROLLED_WRITE.with(std::cell::Cell::get) }
#[cfg(test)]
struct WriteContext(bool);
#[cfg(test)]
impl WriteContext {
    fn enter() -> Self { Self(CONTROLLED_WRITE.with(|slot| slot.replace(true))) }
}
#[cfg(test)]
impl Drop for WriteContext {
    fn drop(&mut self) { CONTROLLED_WRITE.with(|slot| slot.set(self.0)); }
}
