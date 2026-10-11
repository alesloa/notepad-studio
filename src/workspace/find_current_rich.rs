// Current-tab Doc View Find All: incremental capture, worker matching and proven endpoints.
use std::{ops::Range, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use futures::{channel::mpsc, FutureExt, StreamExt};
use gpui::{AppContext, Context, Task};
use uuid::Uuid;
use crate::{buffer::ContentRevision, find_block_map::FindBlockMap, find_in_files::FindResult, rich_doc::TextSelection};
use super::{Workspace, find::SearchFlags, find_current_budget::{Credit, Ledger, Owned, Pool},
    find_current_controller::{Controller, Request}, find_current_endpoints::{EndpointResolver, ResolvedTargets},
    find_current_pattern::{copy_text, QUERY_BYTES}, find_current_pipe::{self, Producer},
    find_current_projection::{ProjectionBuilder, ProjectionError, RETAINED_BYTES},
    find_current_worker::{self, Failure, Prepared, RowCredits}};

const METADATA_BYTES: usize = 1024 * 1024;
const FRAME_DELAY: Duration = Duration::from_millis(16);
type Actor = Controller<Job, Output, Retired>;

struct Guard {
    tab: Uuid,
    window: Uuid,
    instance: Arc<()>,
    activity: ContentRevision,
    raw: ContentRevision,
    rich: ContentRevision,
    owner: Arc<()>,
    query: Owned<String>,
    path: Owned<PathBuf>,
    label: Owned<String>,
    flags: SearchFlags,
    scoped: bool,
    scope: Option<Range<usize>>,
    tab_hint: usize,
    _credit: Credit,
}
struct Job {
    guard: Arc<Guard>,
    #[cfg(test)]
    hold: Option<futures::channel::oneshot::Receiver<()>>,
}
struct Output { guard: Arc<Guard>, result: Result<Prepared, Failure> }
// Three inline slots are explicitly credited; boxing would add separate
// allocation/admission and destruction paths for the empty producer handoff.
#[allow(clippy::large_enum_variant)]
enum Notice { Started(Uuid, Arc<Guard>, Producer), Completed(Uuid), Wake, Tick }
struct NoticeSender { sender: Mutex<mpsc::Sender<Notice>>, wake_pending: AtomicBool }
type Notices = Arc<NoticeSender>;
const NOTICE_SLOTS: usize = 3;

struct Visible {
    guard: Arc<Guard>,
    targets: ResolvedTargets,
    _row_credits: Owned<Vec<RowCredits>>,
    _rows_credit: Credit,
    _matches_credit: Credit,
    _marks_credit: Credit,
    _query_credit: Credit,
}
// Actual externally visible data is destroyed before its allocation leases.
#[derive(Default)]
struct CacheBatch {
    _rows: Vec<FindResult>,
    _matches: Vec<Range<usize>>,
    _marks: Vec<Range<usize>>,
    _map: Option<Arc<FindBlockMap>>,
    _query: String,
    _visible: Option<Visible>,
}
#[derive(Default)]
struct Retired {
    _caches: CacheBatch,
    scratch: Option<EndpointResolver>,
    // Window close can join visible data with completion scratch without a
    // second queue slot or allocation. This slot is filled at most once.
    closing: Option<CacheBatch>,
}
struct Publication {
    rows: Vec<FindResult>, matches: Vec<Range<usize>>, marks: Vec<Range<usize>>,
    visible: Visible, query: String, truncated: bool,
}

pub(super) struct Runtime {
    ledger: Ledger,
    metadata: Pool,
    owner: Arc<()>,
    actor: Option<Arc<Actor>>,
    notices: Option<Notices>,
    pending: Option<Job>,
    current: Option<(Uuid, Arc<Guard>)>,
    capture: Option<Producer>,
    completion: Option<Uuid>,
    visible: Option<Visible>,
    task: Option<Task<()>>,
    _credit: Option<Credit>,
    #[cfg(test)]
    pub(super) hold: Option<futures::channel::oneshot::Receiver<()>>,
    #[cfg(test)]
    pub(super) hold_notices: Option<futures::channel::oneshot::Receiver<()>>,
    #[cfg(test)]
    pub(super) frame_max_bytes: usize,
    #[cfg(test)]
    pub(super) first_scheduled_capture_bytes: Option<usize>,
    #[cfg(test)]
    pub(super) capture_elapsed_override: Option<Duration>,
    #[cfg(test)]
    pub(super) trace: Option<Arc<super::find_current_schedule_tests::Trace>>,
    #[cfg(test)]
    pub(super) frame_max_nodes: usize,
}
impl Default for Runtime {
    fn default() -> Self {
        let ledger = Ledger::default(); let metadata = ledger.pool(METADATA_BYTES);
        Self { ledger, metadata, owner: Arc::new(()), actor: None, notices: None,
            pending: None, current: None, capture: None, completion: None, visible: None,
            task: None, _credit: None,
            #[cfg(test)] hold: None,
            #[cfg(test)] hold_notices: None,
            #[cfg(test)] frame_max_bytes: 0,
            #[cfg(test)] first_scheduled_capture_bytes: None,
            #[cfg(test)] capture_elapsed_override: None,
            #[cfg(test)] trace: None,
            #[cfg(test)] frame_max_nodes: 0,
        }
    }
}
#[cfg(test)]
impl Runtime {
    pub(super) fn wake_for_test(&self) {
        if let Some(notices) = &self.notices { let _ = send(notices, Notice::Wake); }
    }
    pub(super) fn waiting_for_retirement_for_test(&self) -> bool { self.pending.is_some() && self.current.is_none() }
    pub(super) fn owned_bytes(&self) -> usize { self.ledger.used() }
    pub(super) fn owned_peak(&self) -> usize { self.ledger.peak() }
    pub(super) fn control_bytes(&self) -> usize {
        std::mem::size_of::<Runtime>() + NOTICE_SLOTS * std::mem::size_of::<Notice>()
            + std::mem::size_of::<NoticeSender>() + std::mem::size_of::<Actor>()
    }
}
impl Drop for Runtime {
    fn drop(&mut self) { if let Some(actor) = &self.actor { actor.close(); } }
}

fn send(notices: &Notices, notice: Notice) -> bool {
    let wake = matches!(notice, Notice::Wake);
    if wake && notices.wake_pending.swap(true, Ordering::AcqRel) { return true; }
    let sent = notices.sender.lock().is_ok_and(|mut sender| sender.try_send(notice).is_ok());
    if wake && !sent { notices.wake_pending.store(false, Ordering::Release); }
    sent
}

impl Guard {
    fn matches(&self, workspace: &mut Workspace, active: bool) -> bool {
        if workspace.window_closed || workspace.session_window_id != self.window
            || !Arc::ptr_eq(&workspace.rich_find_current.owner, &self.owner)
            || workspace.find_state.query != self.query.data || workspace.find_flags() != self.flags
            || workspace.find_state.in_selection != self.scoped
            || (if self.scoped { workspace.find_state.in_selection_scope.as_ref() } else { None }) != self.scope.as_ref() {
            return false;
        }
        let index = if active {
            if workspace.tabs.get(workspace.active).is_none_or(|tab| tab.session_id != self.tab) { return false; }
            workspace.active
        } else {
            let Some(index) = workspace.tabs.iter().position(|tab| tab.session_id == self.tab) else { return false; };
            index
        };
        let Some(tab) = workspace.tabs.get_mut(index) else { return false; };
        tab.observe_content_activity();
        tab.buffer.loading.is_none() && super::find_view::uses_rich_text(tab)
            && Arc::ptr_eq(&tab.instance, &self.instance) && tab.content_activity == self.activity
            && tab.buffer.raw_revision == self.raw
            && tab.buffer.rich_doc.as_ref().is_some_and(|payload| payload.revision == self.rich)
    }
}

impl Workspace {
    fn take_rich_find_caches(&mut self) -> CacheBatch {
        self.find_state.current = None;
        CacheBatch {
            _rows: std::mem::take(&mut self.find_in_files.results),
            _matches: std::mem::take(&mut self.find_state.matches),
            _marks: std::mem::take(&mut self.mark_state.marks),
            _map: self.find_state.rich_doc_block_offsets.take(),
            _query: std::mem::take(&mut self.find_in_files.last_search_query),
            _visible: self.rich_find_current.visible.take(),
        }
    }
    pub(super) fn cancel_rich_find_current(&mut self) -> bool {
        let running = self.rich_find_current.pending.is_some() || self.rich_find_current.current.is_some();
        self.rich_find_current.owner = Arc::new(());
        self.rich_find_current.pending = None;
        self.rich_find_current.capture = None;
        self.rich_find_current.current = None;
        self.rich_find_current.completion = None;
        if running { self.find_in_files.searching = false; }
        let mut admitted = true;
        if let Some(actor) = self.rich_find_current.actor.clone() {
            actor.cancel();
            if self.rich_find_current.visible.is_some() {
                admitted = actor.retire_or_extend_using(
                    |previous| previous.is_none_or(|batch| batch.closing.is_none()),
                    |previous| {
                        let caches = self.take_rich_find_caches();
                        if let Some(mut previous) = previous {
                            previous.closing = Some(caches);
                            previous
                        } else { Retired { _caches: caches, ..Default::default() } }
                    },
                );
            }
        }
        if let Some(notices) = &self.rich_find_current.notices { let _ = send(notices, Notice::Wake); }
        admitted
    }
    pub(super) fn close_rich_find_current(&mut self) {
        let _ = self.cancel_rich_find_current();
        if let Some(actor) = &self.rich_find_current.actor { actor.close(); }
        self.rich_find_current.task = None;
    }
    fn ensure_rich_find_driver(&mut self, cx: &mut Context<Self>) -> Result<(), ProjectionError> {
        if self.rich_find_current.actor.is_some() { return Ok(()); }
        let credit = self.rich_find_current.metadata.charge(std::mem::size_of::<Runtime>()
            .saturating_add(NOTICE_SLOTS * std::mem::size_of::<Notice>())
            .saturating_add(std::mem::size_of::<NoticeSender>()))?;
        let (actor, wakes) = Actor::new(&self.rich_find_current.metadata)?;
        // Exactly one Sender: buffer2 plus its reserved slot retain at most
        // three notices: previous completion, next producer, one coalesced Wake.
        // The next completion needs that producer consumed and captured first.
        let (sender, mut receiver) = mpsc::channel(NOTICE_SLOTS - 1);
        let notices = Arc::new(NoticeSender { sender: Mutex::new(sender), wake_pending: AtomicBool::new(false) });
        let worker_notices = Arc::clone(&notices);
        let completion_notices = Arc::clone(&notices);
        let worker_actor = Arc::clone(&actor);
        let ledger = self.rich_find_current.ledger.clone();
        let metadata = self.rich_find_current.metadata.clone();
        #[cfg(test)]
        let driver_trace = self.rich_find_current.trace.clone();
        #[cfg(test)]
        let worker_trace = driver_trace.clone();
        cx.background_spawn(async move {
            worker_actor.run(wakes, move |request: Request<Job>| {
                let notices = Arc::clone(&worker_notices);
                let ledger = ledger.clone(); let metadata = metadata.clone();
                #[cfg(test)]
                let trace = worker_trace.clone();
                async move {
                    let guard = request.value.guard;
                    let result = async {
                        let projection_pool = ledger.pool(RETAINED_BYTES);
                        let (producer, mut consumer) = find_current_pipe::pipe(&projection_pool, &metadata)?;
                        if !send(&notices, Notice::Started(request.id, Arc::clone(&guard), producer)) {
                            return Err(Failure::Cancelled);
                        }
                        #[cfg(test)]
                        if let Some(hold) = request.value.hold { let _ = hold.await; }
                        let mut builder = ProjectionBuilder::new(&projection_pool)?;
                        loop {
                            let step = consumer.next().await.ok_or(Failure::Cancelled)?;
                            if request.cancel.load(Ordering::Acquire) { return Err(Failure::Cancelled); }
                            let done = step.done;
                            #[cfg(test)] let push_started = Instant::now();
                            builder.push(step.chunk)?;
                            #[cfg(test)] if let Some(trace) = &trace { trace.projection(push_started.elapsed()); }
                            if done { break; }
                        }
                        #[cfg(test)] let finish_started = Instant::now();
                        let projection = builder.finish()?;
                        #[cfg(test)] if let Some(trace) = &trace { trace.projection(finish_started.elapsed()); }
                        #[cfg(test)] let prepare_started = Instant::now();
                        let prepared = find_current_worker::prepare(projection, &guard.query.data, &guard.flags,
                            guard.scope.as_ref(), &guard.path.data, &guard.label.data, guard.tab_hint,
                            &ledger, &request.cancel);
                        #[cfg(test)] if let Some(trace) = &trace { trace.prepare(prepare_started.elapsed()); }
                        prepared
                    }.await;
                    Output { guard, result }
                }
            }, move |id| { let _ = send(&completion_notices, Notice::Completed(id)); }).await;
        }).detach();
        #[cfg(test)]
        let hold_notices = self.rich_find_current.hold_notices.take();
        let ui_notices = Arc::clone(&notices);
        let task = cx.spawn(async move |this, cx| {
            #[cfg(test)]
            if let Some(hold) = hold_notices { let _ = hold.await; }
            let mut busy = false;
            loop {
                let notice = if busy {
                    let timer = cx.background_executor().timer(FRAME_DELAY).fuse();
                    let next = receiver.next().fuse();
                    futures::pin_mut!(timer, next);
                    futures::select! { value = next => value, _ = timer => Some(Notice::Tick) }
                } else { receiver.next().await };
                let Some(notice) = notice else { break; };
                if matches!(notice, Notice::Wake) { ui_notices.wake_pending.store(false, Ordering::Release); }
                let tick = matches!(notice, Notice::Tick);
                let callback_started = Instant::now();
                #[cfg(test)] if let Some(trace) = &driver_trace { trace.begin(); }
                let updated = this.update(cx, |workspace, cx| {
                    if workspace.window_closed { return false; }
                    match notice {
                        Notice::Started(id, guard, producer) => {
                            if workspace.rich_find_current.current.as_ref().is_some_and(|(current, _)| *current == id)
                                && guard.matches(workspace, true) {
                                workspace.rich_find_current.capture = Some(producer);
                            }
                        }
                        Notice::Completed(id) => {
                            if workspace.rich_find_current.current.as_ref().is_some_and(|(current, _)| *current == id) {
                                workspace.rich_find_current.completion = Some(id);
                            }
                        }
                        Notice::Wake | Notice::Tick => {}
                    }
                    if tick { workspace.step_rich_find_current(cx, callback_started, true); }
                    workspace.rich_find_current.pending.is_some() || workspace.rich_find_current.current.is_some()
                        || workspace.rich_find_current.capture.is_some()
                        || workspace.rich_find_current.completion.is_some()
                });
                #[cfg(test)] if let Some(trace) = &driver_trace { trace.finish(callback_started, tick); }
                let Ok(updated) = updated else { break; };
                busy = updated;
            }
        });
        self.rich_find_current.actor = Some(actor);
        self.rich_find_current.notices = Some(notices);
        self.rich_find_current.task = Some(task);
        self.rich_find_current._credit = Some(credit);
        Ok(())
    }
    pub(super) fn start_rich_find_current(&mut self, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        if !self.cancel_rich_find_current() {
            self.find_state.status = Some("Search cleanup is in progress; try again".into()); cx.notify(); return;
        }
        let captured = (|| {
            self.ensure_rich_find_driver(cx)?;
            let pool = &self.rich_find_current.metadata;
            let query = copy_text(&self.find_state.query, pool, QUERY_BYTES)?;
            let flags = self.find_flags();
            let Some(tab) = self.tabs.get_mut(self.active) else { return Err(ProjectionError::SourceChanged); };
            tab.observe_content_activity();
            if tab.buffer.loading.is_some() || !super::find_view::uses_rich_text(tab) { return Err(ProjectionError::SourceChanged); }
            let payload = tab.buffer.rich_doc.as_ref().ok_or(ProjectionError::SourceChanged)?;
            let source_path = tab.buffer.file_path.as_deref().unwrap_or(std::path::Path::new(""));
            let mut path = Owned::<PathBuf>::new(pool)?;
            path.reserve(source_path.as_os_str().len())?; path.data.push(source_path);
            if path.data.capacity() != source_path.as_os_str().len() || path.data.as_os_str() != source_path.as_os_str() {
                return Err(ProjectionError::Limit);
            }
            let label = source_path.file_name().and_then(|name| name.to_str())
                .or_else(|| tab.buffer.new_label.as_ref().map(|name| name.as_ref())).unwrap_or("Document");
            let label = copy_text(label, pool, METADATA_BYTES)?;
            let guard = Arc::new(Guard { tab: tab.session_id, window: self.session_window_id,
                instance: Arc::clone(&tab.instance), activity: tab.content_activity.clone(),
                raw: tab.buffer.raw_revision.clone(), rich: payload.revision.clone(),
                owner: Arc::clone(&self.rich_find_current.owner), query, path, label, flags,
                scoped: self.find_state.in_selection,
                scope: if self.find_state.in_selection { self.find_state.in_selection_scope.clone() } else { None },
                tab_hint: self.active, _credit: pool.charge(std::mem::size_of::<Guard>())? });
            Ok(Job { guard, #[cfg(test)] hold: self.rich_find_current.hold.take() })
        })();
        match captured {
            Ok(job) => {
                self.rich_find_current.pending = Some(job);
                self.find_in_files.searching = true;
                self.find_in_files.open = true;
                self.find_state.status = Some("Searching current document…".into());
                self.find_in_files.status = Some("Searching current document…".into());
                self.find_in_files.error = None;
                self.step_rich_find_current(cx, Instant::now(), false);
                if let Some(notices) = &self.rich_find_current.notices { let _ = send(notices, Notice::Wake); }
            }
            Err(_) => {
                self.find_in_files.searching = false;
                self.find_state.status = Some("Current document search exceeds its memory budget".into());
            }
        }
        cx.notify();
    }
    fn stop_stale_rich_find_current(&mut self, cx: &mut Context<Self>) {
        self.cancel_rich_find_current();
        self.find_in_files.searching = false;
        let status = "Document or search changed; run the search again";
        self.find_state.status = Some(status.into());
        self.find_in_files.status = Some(status.into());
        cx.notify();
    }
    fn step_rich_find_current(&mut self, cx: &mut Context<Self>, callback_started: Instant, _scheduled: bool) {
        let Some(actor) = self.rich_find_current.actor.clone() else { return; };
        if self.rich_find_current.pending.is_some() {
            if !actor.retire_using(|| Retired { _caches: self.take_rich_find_caches(), ..Default::default() }) { return; }
            let Some(job) = self.rich_find_current.pending.take() else { return; };
            if !job.guard.matches(self, true) { self.stop_stale_rich_find_current(cx); return; }
            let guard = Arc::clone(&job.guard);
            self.rich_find_current.current = actor.request(job).map(|id| (id, guard));
            if self.rich_find_current.current.is_none() {
                self.find_in_files.searching = false;
                self.find_state.status = Some("Search is shutting down; run the search again".into()); cx.notify();
            }
            return;
        }
        let Some((id, guard)) = self.rich_find_current.current.clone() else { return; };
        #[cfg(test)] let guard_started = Instant::now();
        let valid = guard.matches(self, true);
        #[cfg(test)] if let Some(trace) = &self.rich_find_current.trace { trace.guard(guard_started.elapsed()); }
        if !valid {
            actor.reject(id); self.stop_stale_rich_find_current(cx); return;
        }
        if let Some(mut producer) = self.rich_find_current.capture.take() {
            let Some(payload) = self.tabs.get(self.active).and_then(|tab| tab.buffer.rich_doc.as_ref()) else { actor.reject(id); self.cancel_rich_find_current(); cx.notify(); return; };
            #[cfg(test)]
            let progress = if let Some(elapsed) = self.rich_find_current.capture_elapsed_override {
                producer.capture_frame_with_elapsed(payload, || elapsed)
            } else { producer.capture_frame(payload, callback_started) };
            #[cfg(not(test))]
            let progress = producer.capture_frame(payload, callback_started);
            match progress {
                Ok(progress) => {
                    if progress.nodes > 1024 || progress.bytes > find_current_pipe::FRAME_COPY_BYTES || progress.attempts > 16 || (progress.blocked && progress.done) {
                        actor.reject(id); self.cancel_rich_find_current();
                        self.find_state.status = Some("Document capture failed; run the search again".into()); cx.notify(); return;
                    }
                    #[cfg(test)] {
                        if let Some(trace) = &self.rich_find_current.trace { trace.capture(progress.bytes, progress.nodes, progress.attempts); }
                        if _scheduled { self.rich_find_current.first_scheduled_capture_bytes.get_or_insert(progress.bytes); }
                        self.rich_find_current.frame_max_bytes = self.rich_find_current.frame_max_bytes.max(progress.bytes);
                        self.rich_find_current.frame_max_nodes = self.rich_find_current.frame_max_nodes.max(progress.nodes);
                    }
                    if !progress.done { self.rich_find_current.capture = Some(producer); }
                }
                Err(_) => {
                    actor.reject(id); self.cancel_rich_find_current(); self.find_in_files.searching = false;
                    self.find_state.status = Some("Current document search exceeds its memory budget".into()); cx.notify();
                }
            }
            return;
        }
        // Completion notices only wake the driver. The actor remains the
        // authority, so a lost hint cannot strand a finished request.
        if self.rich_find_current.completion != Some(id) {
            if actor.with_output(id, |_| ()).is_none() { return; }
            self.rich_find_current.completion = Some(id);
        }
        let Some(payload) = self.tabs.get(self.active).and_then(|tab| tab.buffer.rich_doc.as_ref()) else { return; };
        let status = actor.with_output(id, |output| {
            match &mut output.result {
                Ok(prepared) => prepared.targets.step(&mut prepared.endpoints, payload)
                    .map(|_| prepared.targets.done()).map_err(|_| "Document changed; run the search again"),
                Err(Failure::Regex(error)) => Err(if matches!(error, regex::Error::CompiledTooBig(_)) {
                    "Regex exceeds its compilation budget"
                } else { "Invalid regex; check the search pattern" }),
                Err(Failure::Budget(error)) => Err(match error {
                    ProjectionError::SourceChanged => "Document changed; run the search again",
                    ProjectionError::InvalidChunk => "Document capture failed; run the search again",
                    ProjectionError::Allocation | ProjectionError::Limit => "Current document search exceeds its memory budget",
                }),
                Err(Failure::Cancelled) => Err("Document or search changed; run the search again"),
            }
        });
        match status {
            Some(Ok(false)) | None => return,
            Some(Err(message)) => {
                actor.reject(id); self.cancel_rich_find_current(); self.find_in_files.searching = false;
                self.find_state.status = Some(message.into()); cx.notify(); return;
            }
            Some(Ok(true)) => {}
        }
        // This bounded query copy is separately leased until its visible
        // header is retired; the guard keeps its independent ownership copy.
        let query = match copy_text(&guard.query.data, &self.rich_find_current.metadata, QUERY_BYTES) {
            Ok(query) => query,
            Err(_) => {
                actor.reject(id); self.cancel_rich_find_current();
                self.find_state.status = Some("Current document search exceeds its memory budget".into()); cx.notify(); return;
            }
        };
        // No controller mutation occurs inside with_output. Only moves occur in
        // this separate atomic acceptance/retirement transaction.
        let publication = actor.accept_retiring(id, |output| {
            let mut retired = Retired { _caches: self.take_rich_find_caches(), ..Default::default() };
            let publication = output.result.ok().map(|prepared| {
                let Prepared { rows, row_credits, matches, marks, map: _, endpoints, targets, truncated } = prepared;
                retired.scratch = Some(endpoints);
                let (rows, rows_credit) = rows.into_parts();
                let (matches, matches_credit) = matches.into_parts();
                let (marks, marks_credit) = marks.into_parts();
                let (query, query_credit) = query.into_parts();
                Publication { rows, matches, marks, query, truncated, visible: Visible {
                    guard: output.guard, targets, _row_credits: row_credits,
                    _rows_credit: rows_credit, _matches_credit: matches_credit, _marks_credit: marks_credit,
                    _query_credit: query_credit,
                } }
            });
            (publication, retired)
        });
        let Some(Some(publication)) = publication else { return; };
        self.find_state.rich_doc_block_offsets = Some(publication.visible.targets.map());
        self.find_in_files.results = publication.rows;
        self.find_state.matches = publication.matches;
        self.mark_state.marks = publication.marks;
        self.find_in_files.truncated = publication.truncated;
        self.find_in_files.searching = false;
        self.find_in_files.last_search_query = publication.query;
        self.find_in_files.last_search_mode = super::find_in_files::describe_search_mode(self.find_state.regex, self.find_state.extended);
        self.find_in_files.last_search_files_searched = 1;
        let total = self.find_in_files.results.len();
        let message = if total == 0 { "No matches in current document".into() }
            else { format!("{total} match{} in current document{}", if total == 1 { "" } else { "es" }, if publication.truncated { " (truncated)" } else { "" }) };
        self.find_in_files.status = Some(message);
        self.find_state.status = self.find_in_files.status.clone();
        self.rich_find_current.visible = Some(publication.visible);
        self.rich_find_current.current = None; self.rich_find_current.completion = None;
        cx.notify();
    }
    pub(super) fn jump_to_rich_find_result(&mut self, row: usize, cx: &mut Context<Self>) -> bool {
        let Some(visible) = self.rich_find_current.visible.as_ref() else { return false; };
        let guard = Arc::clone(&visible.guard);
        let positions = self.find_in_files.results.get(row)
            .and_then(|result| visible.targets.get(row, &result.identity));
        let Some((anchor, cursor, _range)) = positions else { return false; };
        if !guard.matches(self, false) {
            self.set_status("This search result is out of date; run the search again"); cx.notify(); return true;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.session_id == guard.tab) else { return true; };
        if index != self.active { self.set_active_tab(index, cx); }
        if self.active != index { return true; }
        if !guard.matches(self, false) { return true; }
        if let Some(payload) = self.tabs.get_mut(index).and_then(|tab| tab.buffer.rich_doc.as_mut()) {
            payload.selection = Some(TextSelection { anchor, cursor }); payload.active_block = Some(cursor.block);
            payload.pending_drag_anchor = None; payload.drag_active = false;
        }
        self.caret_visible = true;
        self.rich_doc_scroll_match_into_view(anchor.block, cx); cx.notify(); true
    }
}
