// Startup syntax uses the existing parser after the window mounts.
// Reads, recovery and undo-history restoration remain in session.rs.
use super::*;

const INITIAL_JOBS: usize = 4;
const SCAN_BUDGET: usize = 64;

#[derive(Default)]
pub(super) struct Pump {
    enabled: bool,
    cursor: usize,
    remaining: usize,
    seen_pending: bool,
    pub(super) task: Option<Task<()>>,
    #[cfg(test)]
    pub(super) max_outstanding: usize,
    #[cfg(test)]
    pub(super) max_scanned: usize,
    #[cfg(test)]
    pub(super) callback_time: Duration,
}

impl Pump {
    pub(super) fn close(&mut self) {
        self.enabled = false;
        self.remaining = 0;
        self.task = None;
    }
}

impl Workspace {
    pub(crate) fn seed_initial_syntax(&mut self, cursor: usize) {
        self.syntax_refresh.initial.enabled = true;
        self.syntax_refresh.initial.cursor = cursor;
        self.syntax_refresh.initial.remaining = self.tabs.len();
        self.syntax_refresh.initial.seen_pending = false;
    }

    pub(crate) fn start_initial_syntax(&mut self, cx: &mut Context<Self>) {
        self.activate_initial_syntax(cx);
    }

    pub(crate) fn activate_initial_syntax(&mut self, cx: &mut Context<Self>) {
        if !self.syntax_refresh.initial.enabled || self.window_closed { return; }
        self.prioritize_initial_syntax(cx);
        self.wake_initial_syntax(cx);
    }

    pub(super) fn initial_outstanding(&self) -> usize {
        self.syntax_refresh.queued.iter().filter(|r| r.origin == Origin::Initial).count()
            + usize::from(self.syntax_refresh.running.as_ref().is_some_and(|r| r.origin == Origin::Initial))
    }

    // The scan budget bounds entries visited here, not all callback work:
    // capture and validity retain the controller's linear UUID/closed-tab lookups.
    pub(super) fn prioritize_initial_syntax(&mut self, cx: &mut Context<Self>) {
        if !self.syntax_refresh.initial.enabled || self.window_closed { return; }
        let Some(tab) = self.tabs.get(self.active).filter(|tab| tab.initial_syntax_pending) else { return; };
        let id = tab.session_id;
        let regular = self.syntax_refresh.delayed.contains_key(&id)
            || self.syntax_refresh.queued.iter().any(|r| r.tab == id && r.origin == Origin::Regular)
            || self.syntax_refresh.running.as_ref().is_some_and(|r| r.tab == id && r.origin == Origin::Regular);
        if regular { return; }
        let inactive = self.syntax_refresh.running.as_ref()
            .filter(|r| r.origin == Origin::Initial && r.tab != id && r.cancel.load(Ordering::Relaxed) == 0)
            .map(|r| r.tab);
        if let Some(inactive) = inactive { self.retire_syntax_tab(inactive, cx); }
        if let Some(index) = self.syntax_refresh.queued.iter().position(|r| r.tab == id && r.origin == Origin::Initial) {
            if let Some(request) = self.syntax_refresh.queued.remove(index) {
                self.syntax_refresh.queued.push_front(request);
            }
        } else if !self.syntax_refresh.latest.contains_key(&id)
            && self.initial_outstanding() < INITIAL_JOBS
            && self.syntax_refresh.latest.len() < MAX_PENDING_TABS
        {
            self.capture_syntax_refresh(id, Duration::ZERO, Origin::Initial, cx);
        }
    }

    pub(super) fn wake_initial_syntax(&mut self, cx: &mut Context<Self>) {
        if !self.syntax_refresh.initial.enabled || self.window_closed { return; }
        self.syntax_refresh.initial.remaining = self.tabs.len();
        self.syntax_refresh.initial.seen_pending = false;
        self.schedule_initial_pump(cx);
    }

    fn schedule_initial_pump(&mut self, cx: &mut Context<Self>) {
        if !self.syntax_refresh.initial.enabled || self.syntax_refresh.initial.task.is_some() || self.window_closed
            || self.syntax_refresh.initial.remaining == 0
            || self.initial_outstanding() >= INITIAL_JOBS
            || self.syntax_refresh.latest.len() >= MAX_PENDING_TABS { return; }
        let timer = cx.background_executor().timer(Duration::from_millis(1));
        self.syntax_refresh.initial.task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.syntax_refresh.initial.task = None;
                workspace.pump_initial_syntax(cx);
            });
        }));
    }

    fn pump_initial_syntax(&mut self, cx: &mut Context<Self>) {
        let started = Instant::now();
        if self.window_closed { self.syntax_refresh.close(); return; }
        self.prioritize_initial_syntax(cx);
        let mut scanned = 0;
        let limit = self.syntax_refresh.initial.remaining.min(self.tabs.len()).min(SCAN_BUDGET);
        while scanned < limit && self.initial_outstanding() < INITIAL_JOBS
            && self.syntax_refresh.latest.len() < MAX_PENDING_TABS
        {
            let len = self.tabs.len();
            if len == 0 { break; }
            let index = self.syntax_refresh.initial.cursor % len;
            self.syntax_refresh.initial.cursor = if index + 1 == len { 0 } else { index + 1 };
            scanned += 1;
            self.syntax_refresh.initial.remaining = self.syntax_refresh.initial.remaining.saturating_sub(1);
            if self.tabs.get(index).is_some_and(|tab| tab.initial_syntax_pending) {
                self.syntax_refresh.initial.seen_pending = true;
            }
            let candidate = self.tabs.get(index).filter(|tab| tab.initial_syntax_pending)
                .filter(|tab| !self.syntax_refresh.latest.contains_key(&tab.session_id))
                .filter(|tab| tab.buffer.loading.is_none()).map(|tab| tab.session_id);
            if let Some(id) = candidate {
                self.capture_syntax_refresh(id, Duration::ZERO, Origin::Initial, cx);
            }
        }
        #[cfg(test)] {
            let outstanding = self.initial_outstanding();
            let pump = &mut self.syntax_refresh.initial;
            pump.max_outstanding = pump.max_outstanding.max(outstanding);
            pump.max_scanned = pump.max_scanned.max(scanned);
            pump.callback_time = pump.callback_time.max(started.elapsed());
        }
        #[cfg(not(test))] let _ = started;
        if self.syntax_refresh.initial.remaining == 0 && !self.syntax_refresh.initial.seen_pending {
            self.syntax_refresh.initial.enabled = false;
        }
        self.schedule_initial_pump(cx);
    }
}
