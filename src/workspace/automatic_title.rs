// PowerEditor/src/NppNotification.cpp:52-61; NppIO.cpp:2213-2267.
use std::{sync::{Arc, atomic::{AtomicBool, Ordering}}, task::Poll};
use futures::channel::oneshot;
use gpui::{AppContext, Context};
use ropey::Rope;
use uuid::Uuid;
use crate::{automatic_title::{Candidate, Cursor, Scan, FOREGROUND_BYTES, history}, buffer::{ContentRevision, prepared::PreparedGroup}};
use super::Workspace;
use crate::io::BackgroundDrop as Owned;
#[path = "automatic_title_worker.rs"] mod worker;
#[path = "automatic_title_consumers.rs"] mod consumers;
use worker::{Credit, NameKey, Notice, Row, MAX_NOTICES};

const IMMEDIATE_TABS: usize = 64;
const CONTEXT_TURN_BYTES: usize = 16 * 1024;
const MAX_CONTEXT_TABS: usize = 4096;
const NOTICE_CHUNK: usize = 16;
const ADMISSION_TURN_BYTES: usize = 32 * 1024;
const LATEST_PAGE: usize = 16;
const MAX_COMMANDS: usize = 256;
const ACTIVITY_INIT_BYTES: usize = 6 * std::mem::size_of::<ContentRevision>() + 2 * std::mem::size_of::<usize>();
type Command = ((Uuid, &'static str), Uuid);
pub(super) const LIMIT_STATUS: &str = "Automatic title updates paused: naming metadata reached its limit; the last accepted name is retained";

pub(super) enum Effect {
    Ready(Candidate), Pending { rope: Rope, cursor: Cursor }, History { group: Arc<PreparedGroup>, undo: bool },
}
struct Running { id: Uuid, cancel: Arc<AtomicBool> }
impl Drop for Running { fn drop(&mut self) { self.cancel.store(true, Ordering::Release); } }
struct Latest { tab: Uuid, instance: Arc<()>, revision: ContentRevision, activity: ContentRevision }
pub(super) struct State {
    notices: Vec<Vec<Notice>>, credit: Arc<Credit>, running: Option<Running>, shadow: Option<Vec<Row>>,
    latest: Vec<Option<Vec<Option<Latest>>>>, input_epoch: ContentRevision, admission_bytes: usize,
    context: ContentRevision, policy: Option<crate::settings::TitlePolicy>, work_budget: usize,
    waiters: Vec<oneshot::Sender<()>>, executor: Option<gpui::BackgroundExecutor>, pub(super) suspended: bool,
    commands: Vec<Command>,
    title_clipboard: Option<Uuid>, window: Option<gpui::AnyWindowHandle>, closing: bool,
    #[cfg(test)] pub(super) gate: Option<oneshot::Receiver<()>>,
    #[cfg(test)] pub(super) validation_gate: Option<oneshot::Receiver<()>>,
    #[cfg(test)] pub(super) validation_offsets: Vec<usize>,
    #[cfg(test)] pub(super) worker_main_thread: Vec<bool>,
    #[cfg(test)] pub(super) admission_turn_bytes: Vec<usize>,
    #[cfg(test)] pub(super) latest_reserve_work: Vec<(usize, usize, usize, usize)>,
    #[cfg(test)] pub(super) context_turn_bytes: Vec<usize>,
    #[cfg(test)] pub(super) disposal_observer: Option<Arc<std::sync::Mutex<Vec<bool>>>>,
    #[cfg(test)] pub(super) commits: Vec<(usize, usize, std::time::Duration)>,
}
impl Default for State {
    fn default() -> Self {
        Self { notices: Vec::new(), credit: Arc::new(Credit::default()), running: None, shadow: None, latest: Vec::new(), input_epoch: Default::default(), admission_bytes: 0,
            context: Default::default(), policy: None, work_budget: history::WORK_LIMIT, waiters: Vec::new(), executor: None, suspended: false,
            commands: Default::default(),
            title_clipboard: None, window: None, closing: false,
            #[cfg(test)] gate: None, #[cfg(test)] validation_gate: None, #[cfg(test)] validation_offsets: Vec::new(), #[cfg(test)] worker_main_thread: Vec::new(), #[cfg(test)] admission_turn_bytes: Vec::new(), #[cfg(test)] latest_reserve_work: Vec::new(), #[cfg(test)] context_turn_bytes: Vec::new(), #[cfg(test)] disposal_observer: None, #[cfg(test)] commits: Vec::new() }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.running.take();
        let payload = (std::mem::take(&mut self.notices), self.shadow.take(), std::mem::take(&mut self.latest));
        if let Some(executor) = &self.executor { executor.spawn(async move { drop(payload); }).detach(); }
    }
}
async fn yield_turn() {
    let mut yielded = false;
    futures::future::poll_fn(|context| {
        if yielded { Poll::Ready(()) } else { yielded = true; context.waker().wake_by_ref(); Poll::Pending }
    }).await;
}
pub(super) fn capture(rope: &Rope, position: usize, budget: usize) -> Result<Option<Effect>, &'static str> {
    if !crate::automatic_title::row_zero_notification(rope, position) { return Ok(None); }
    match Cursor::new().advance(rope, budget, &AtomicBool::new(false))? {
        Scan::Complete(Some(candidate)) => Ok(Some(Effect::Ready(candidate))), Scan::Complete(None) => Ok(None),
        Scan::Pending(cursor) => Ok(Some(Effect::Pending { rope: rope.clone(), cursor })),
    }
}
fn path_identity(tab: &crate::tab::Tab) -> Option<(usize, usize)> {
    tab.buffer.file_path.as_ref().map(|path| { let bytes = path.as_os_str().as_encoded_bytes(); (bytes.as_ptr() as usize, bytes.len()) })
}
fn label_identity(label: Option<&gpui::SharedString>) -> Option<(usize, usize)> { label.map(|label| (label.as_ptr() as usize, label.len())) }
fn capture_row(tab: &crate::tab::Tab, index: usize, context_lease: worker::ContextLease,
    #[cfg(test)] probe: Option<worker::DropProbe>) -> Row {
    let full = if let Some(path) = &tab.buffer.file_path { path.as_os_str().as_encoded_bytes() }
        else { tab.buffer.new_label.as_ref().map(|label| label.as_bytes()).unwrap_or(b"new") };
    Row { index, tab: tab.session_id, instance: tab.instance.clone(), label: tab.buffer.new_label.clone(), path_identity: path_identity(tab),
        custom: tab.buffer.new_label_is_custom, revision: tab.buffer.raw_revision.clone(), activity: tab.content_activity.clone(), key: NameKey::new(full), accepted: None, publish: None,
        #[cfg(test)] _drop_probe: probe, _context_lease: Some(context_lease) }
}

impl Workspace {
    pub(super) fn begin_automatic_title_admission(&mut self) { self.automatic_title.admission_bytes = 0; }

    fn charge_automatic_title_admission(&mut self, bytes: usize) -> Result<(), &'static str> {
        let total = self.automatic_title.admission_bytes.checked_add(bytes).filter(|&total| total <= ADMISSION_TURN_BYTES).ok_or(LIMIT_STATUS)?;
        self.automatic_title.admission_bytes = total;
        #[cfg(test)] self.automatic_title.admission_turn_bytes.push(total);
        Ok(())
    }
    pub(crate) fn observe_automatic_title_policy(&mut self, cx: &mut Context<Self>) {
        if self.automatic_title.policy.as_ref().is_some_and(|policy| *policy != crate::settings::untitled_title_policy()) {
            self.discard_automatic_titles(None, cx);
        }
    }
    pub(crate) fn automatic_title_chip(&self) -> Option<crate::ui::chrome::LoadingChip> {
        self.automatic_title.running.as_ref().map(|run| crate::ui::chrome::LoadingChip {
            progress_text: "Updating untitled names; Cancel retains the accepted names".to_owned(), cancel: run.cancel.clone(),
        })
    }
    pub(crate) fn cancel_automatic_titles(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.automatic_titles_pending() { return false; }
        self.discard_automatic_titles(Some("Automatic title updates cancelled; the accepted names and current documents are retained"), cx); true
    }
    pub(super) fn automatic_title_foreground_budget(&self) -> usize {
        if self.bulk_edit || self.tabs.get(self.active).is_some_and(|tab| tab.edit_transaction.is_some())
            || self.automatic_titles_pending() || self.tabs.len() > IMMEDIATE_TABS { 0 } else { FOREGROUND_BYTES }
    }
    pub(super) fn automatic_title_capture_allowed(&mut self, cx: &mut Context<Self>) -> bool {
        let policy = crate::settings::untitled_title_policy();
        if self.automatic_title.policy.as_ref().is_some_and(|old| old != &policy) { self.discard_automatic_titles(None, cx); }
        if !policy.enabled { return false; }
        if self.automatic_title.suspended {
            if !self.automatic_title.credit.available() { return false; }
            self.automatic_title.suspended = false;
        }
        self.automatic_title.policy = Some(policy); true
    }
    pub(super) fn record_automatic_title_effects(&mut self, index: usize, effects: [Result<Option<Effect>, &'static str>; 2], cx: &mut Context<Self>) {
        if self.window_closed { return; }
        for effect in effects { match effect {
            Ok(Some(effect)) => self.enqueue_automatic_title(index, effect, cx), Ok(None) => {}, Err(error) => self.discard_automatic_titles(Some(error), cx),
        } }
    }
    fn enqueue_automatic_title(&mut self, index: usize, effect: Effect, cx: &mut Context<Self>) {
        if self.automatic_title.suspended { cx.background_spawn(async move { drop(effect); }).detach(); return; }
        let Some(tab) = self.tabs.get(index) else { return; };
        if tab.buffer.file_path.is_some() || tab.buffer.new_label_is_custom { return; }
        let id = tab.session_id; let instance = tab.instance.clone();
        if self.automatic_title.running.is_none() && self.automatic_title.notices.is_empty() && self.tabs.len() <= IMMEDIATE_TABS {
            if let Effect::Ready(candidate) = &effect {
                if let Err(error) = self.accept_automatic_candidate(index, candidate) { self.set_status(error); cx.notify(); } return;
            }
        }
        let bytes = match &effect { Effect::Ready(_) => Some(0), Effect::Pending { rope, .. } => Some(rope.len_bytes()), Effect::History { group, .. } => worker::history_charge(group) };
        let lease = bytes.and_then(|bytes| self.automatic_title.credit.admit(bytes).ok());
        if self.automatic_title.suspended || lease.is_none() || self.reserve_automatic_title_notice().is_err() {
            cx.background_spawn(async move { drop((effect, lease)); }).detach(); self.discard_automatic_titles(Some(LIMIT_STATUS), cx); return;
        }
        let Some(lease) = lease else { return; };
        self.automatic_title.executor = Some(cx.background_executor().clone());
        let notice = Notice { tab: id, instance, effect, _lease: lease };
        if let Some(chunk) = self.automatic_title.notices.last_mut() { chunk.push(notice); }
        else { cx.background_spawn(async move { drop(notice); }).detach(); self.discard_automatic_titles(Some(LIMIT_STATUS), cx); return; }
        self.observe_automatic_title_input(index, cx);
    }
    fn reserve_automatic_title_notice(&mut self) -> Result<(), &'static str> {
        if self.automatic_title.notices.last().is_some_and(|chunk| chunk.len() < NOTICE_CHUNK) {
            self.charge_automatic_title_admission(2 * std::mem::size_of::<Notice>())?;
            return Ok(());
        }
        let chunks = self.automatic_title.notices.len();
        if chunks >= MAX_NOTICES / NOTICE_CHUNK { return Err(LIMIT_STATUS); }
        let records = NOTICE_CHUNK.checked_mul(std::mem::size_of::<Notice>()).ok_or(LIMIT_STATUS)?;
        let outer = chunks.checked_add(chunks.saturating_add(1)).and_then(|count| count.checked_mul(std::mem::size_of::<Vec<Notice>>())).ok_or(LIMIT_STATUS)?;
        let cost = records.checked_add(outer).and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<Notice>() + 2 * std::mem::size_of::<Vec<Notice>>())).ok_or(LIMIT_STATUS)?;
        self.charge_automatic_title_admission(cost)?;
        let mut chunk = Vec::new(); chunk.try_reserve_exact(NOTICE_CHUNK).map_err(|_| LIMIT_STATUS)?;
        self.automatic_title.notices.try_reserve_exact(1).map_err(|_| LIMIT_STATUS)?;
        self.automatic_title.notices.push(chunk);
        Ok(())
    }
    pub(super) fn record_automatic_title_history(&mut self, index: usize, group: Arc<PreparedGroup>, undo: bool, cx: &mut Context<Self>) {
        self.begin_automatic_title_admission();
        if !group.has_title_notifications(undo) || !self.automatic_title_capture_allowed(cx) { return; }
        self.enqueue_automatic_title(index, Effect::History { group, undo }, cx);
    }
    pub(crate) fn observe_automatic_title_input(&mut self, index: usize, cx: &mut Context<Self>) {
        self.automatic_title.input_epoch = Default::default();
        if self.automatic_title.running.is_none() && self.automatic_title.notices.is_empty() { return; }
        let Some(tab) = self.tabs.get(index) else { return; };
        let latest = Latest { tab: tab.session_id, instance: tab.instance.clone(), revision: tab.buffer.raw_revision.clone(), activity: tab.content_activity.clone() };
        if self.set_automatic_latest(index, latest).is_err() { self.discard_automatic_titles(Some(LIMIT_STATUS), cx); }
    }
    fn set_automatic_latest(&mut self, index: usize, latest: Latest) -> Result<(), &'static str> {
        if index >= MAX_CONTEXT_TABS { return Err(LIMIT_STATUS); }
        let page = index / LATEST_PAGE;
        if self.automatic_title.latest.is_empty() {
            let count = MAX_CONTEXT_TABS / LATEST_PAGE;
            let bytes = count.checked_mul(2 * std::mem::size_of::<Option<Vec<Option<Latest>>>>()).ok_or(LIMIT_STATUS)?;
            self.charge_automatic_title_admission(bytes)?;
            // The capped directory is allocated once. Later tabs add one
            // page without relocating any accumulated Latest entries.
            self.automatic_title.latest.try_reserve_exact(count).map_err(|_| LIMIT_STATUS)?;
            self.automatic_title.latest.resize_with(count, || None);
            #[cfg(test)] self.automatic_title.latest_reserve_work.push((0, self.automatic_title.latest.capacity(), 0, bytes));
        }
        if self.automatic_title.latest.get(page).is_some_and(Option::is_none) {
            let bytes = LATEST_PAGE.checked_mul(2 * std::mem::size_of::<Option<Latest>>()).ok_or(LIMIT_STATUS)?;
            self.charge_automatic_title_admission(bytes)?;
            let mut slots = Vec::new(); slots.try_reserve_exact(LATEST_PAGE).map_err(|_| LIMIT_STATUS)?;
            slots.resize_with(LATEST_PAGE, || None);
            #[cfg(test)] self.automatic_title.latest_reserve_work.push((0, slots.capacity(), 0, bytes));
            let directory = self.automatic_title.latest.get_mut(page).ok_or(LIMIT_STATUS)?;
            *directory = Some(slots);
        }
        self.charge_automatic_title_admission(2 * std::mem::size_of::<Latest>())?;
        let slot = self.automatic_title.latest.get_mut(page).and_then(Option::as_mut)
            .and_then(|slots| slots.get_mut(index % LATEST_PAGE)).ok_or(LIMIT_STATUS)?;
        *slot = Some(latest); Ok(())
    }
    pub(super) fn drive_automatic_titles(&mut self, cx: &mut Context<Self>) {
        if self.window_closed { self.discard_automatic_titles(None, cx); return; }
        if self.automatic_title.running.is_some() || self.automatic_title.notices.is_empty() { return; }
        if !self.automatic_title_capture_allowed(cx) || self.automatic_title.notices.is_empty() { return; }
        let context = self.automatic_title.context.clone(); let Some(policy) = self.automatic_title.policy.clone() else { return; };
        let id = Uuid::new_v4(); let cancel = Arc::new(AtomicBool::new(false));
        self.automatic_title.running = Some(Running { id, cancel: cancel.clone() });
        let notices = std::mem::take(&mut self.automatic_title.notices); let shadow = self.automatic_title.shadow.take();
        let budget = self.automatic_title.work_budget; let executor = cx.background_executor().clone();
        #[cfg(test)] let gate = self.automatic_title.gate.take();
        cx.spawn(async move |this, cx| {
            let mut owned = Owned::new((notices, shadow), executor.clone());
            let shadow = owned.as_mut().and_then(|(_, shadow)| shadow.take());
            let mut rows = if let Some(rows) = shadow { Owned::new(rows, executor.clone()) } else {
                let count = this.update(cx, |w, _| w.tabs.len().min(MAX_CONTEXT_TABS));
                let count = count.unwrap_or(0);
                let reserve_executor = executor.clone();
                let reserved = executor.spawn(async move {
                    let mut rows = Vec::new();
                    let result = rows.try_reserve_exact(count).map(|_| rows).map_err(|_| "Not enough memory to prepare automatic names");
                    Owned::new(result, reserve_executor)
                }).await;
                let mut reserved = reserved;
                match reserved.take() {
                    Some(Ok(rows)) => Owned::new(rows, executor.clone()),
                    _ => { let _ = this.update(cx, |w, cx| w.finish_automatic_title_failure(id, LIMIT_STATUS, cx)); return; }
                }
            };
            let mut offset = rows.as_ref().filter(|rows| !rows.is_empty()).map_or(0usize, |_| usize::MAX);
            loop {
                if offset == usize::MAX { break; }
                let captured = this.update(cx, |w, cx| {
                    if w.window_closed || !w.automatic_title_window_valid(cx) || w.automatic_title.context != context || crate::settings::untitled_title_policy() != policy
                        || !w.automatic_title.running.as_ref().is_some_and(|run| run.id == id) { return Err("Automatic-name context changed"); }
                    if w.tabs.len() > MAX_CONTEXT_TABS { return Err(LIMIT_STATUS); }
                    let Some(rows) = rows.as_mut() else { return Err("Automatic-name context was released"); };
                    // No reservation or accumulated-vector move occurs on this turn.
                    let row_cost = std::mem::size_of::<Row>().checked_mul(4).and_then(|bytes| bytes.checked_add(2 * 190))
                        .and_then(|bytes| bytes.checked_add(ACTIVITY_INIT_BYTES)).ok_or(LIMIT_STATUS)?;
                    let count = (CONTEXT_TURN_BYTES / row_cost).min(IMMEDIATE_TABS);
                    if count == 0 { return Err(LIMIT_STATUS); }
                    let end = offset.saturating_add(count).min(w.tabs.len());
                    if end > rows.capacity() { return Err("Automatic-name tab list changed while capturing"); }
                    for index in offset..end {
                        let Some(tab) = w.tabs.get_mut(index) else { return Err("Automatic-name tab list changed"); };
                        tab.observe_content_activity();
                        // Charge each retained label occurrence, even shared Arcs,
                        // before cloning it. Native paths are only borrowed for
                        // the bounded comparison key and are not retained.
                        let context_lease = w.automatic_title.credit.retain_context(tab.buffer.new_label.as_ref().map_or(0, |label| label.len()))?;
                        rows.push(capture_row(tab, index, context_lease,
                        #[cfg(test)] w.automatic_title.disposal_observer.as_ref().map(|observed| worker::DropProbe { executor: cx.background_executor().clone(), observed: observed.clone() }))); }
                    #[cfg(test)] w.automatic_title.context_turn_bytes.push(end.saturating_sub(offset).saturating_mul(row_cost));
                    Ok((end, end == w.tabs.len()))
                });
                match captured {
                    Ok(Ok((end, done))) => { offset = end; if done { break; } },
                    _ => { let _ = this.update(cx, |w, cx| w.finish_automatic_title_failure(id, "Automatic-name context changed or exceeded its limit", cx)); return; }
                }
                yield_turn().await;
            }
            let Some((notices, _)) = owned.take() else { return; };
            let Some(rows) = rows.take() else { return; };
            let worker_executor = executor.clone();
            let worker = executor.spawn(async move {
                #[cfg(test)] if let Some(gate) = gate { let _ = gate.await; }
                #[cfg(test)] let main = worker_executor.is_main_thread();
                #[cfg(not(test))] let main = false;
                let result = worker::reduce(rows, notices, &cancel, budget);
                Owned::new((result, main), worker_executor)
            });
            let mut result = worker.await;
            let result = this.update(cx, |w, cx| {
                let (result, _main) = result.take()?;
                #[cfg(test)] w.automatic_title.worker_main_thread.push(_main);
                w.finish_automatic_title(id, &context, &policy, result, cx)
                    .map(|rows| Owned::new(rows, cx.background_executor().clone()))
            });
            if let Ok(Some(mut rows)) = result {
                let mut offset = 0usize; let mut input = None;
                loop {
                    let done = this.update(cx, |w, cx| {
                        if !w.automatic_title.running.as_ref().is_some_and(|run| run.id == id) { return true; }
                        if w.automatic_title.running.as_ref().is_some_and(|run| run.cancel.load(Ordering::Acquire)) {
                            w.finish_automatic_title_failure(id, "Automatic naming cancelled; the last accepted name is retained", cx); return true;
                        }
                        if !w.automatic_title.notices.is_empty() { w.automatic_title.shadow = rows.take(); w.automatic_title.running.take(); w.drive_automatic_titles(cx); return true; }
                        if w.automatic_title.context != context || crate::settings::untitled_title_policy() != policy || w.window_closed || !w.automatic_title_window_valid(cx) {
                            w.finish_automatic_title_failure(id, "Automatic-name context changed; the last accepted name is retained", cx); return true;
                        }
                        if input.as_ref() != Some(&w.automatic_title.input_epoch) { input = Some(w.automatic_title.input_epoch.clone()); offset = 0; }
                        let Some(rows) = rows.as_mut() else { return true; };
                        let count = (CONTEXT_TURN_BYTES / std::mem::size_of::<Row>()).min(IMMEDIATE_TABS);
                        let end = offset.saturating_add(count).min(rows.len());
                        for row in rows.get(offset..end).unwrap_or(&[]) {
                            if !w.automatic_row_valid(row) { w.finish_automatic_title_failure(id, "Automatic-name document changed; the last accepted name is retained", cx); return true; }
                        }
                        offset = end;
                        #[cfg(test)] w.automatic_title.validation_offsets.push(offset);
                        if offset != rows.len() { return false; }
                        #[cfg(test)] let started = std::time::Instant::now();
                        #[cfg(test)] let mut changed = 0usize;
                        // Names and capacity were prepared on BG. The cap limits
                        // this atomic commit to 4096 label assignments, with old
                        // labels kept by rows until their worker disposal.
                        for row in rows {
                            if let Some(name) = row.publish.take() {
                                if let Some(tab) = w.tabs.get_mut(row.index) { tab.buffer.new_label = Some(name); }
                                #[cfg(test)] { changed += 1; }
                            }
                        }
                        #[cfg(test)] {
                            let moves = 4 * std::mem::size_of::<Option<gpui::SharedString>>() + 2 * std::mem::size_of::<gpui::SharedString>();
                            w.automatic_title.commits.push((changed, changed.saturating_mul(moves), started.elapsed()));
                        }
                        w.complete_automatic_titles(cx); cx.notify(); true
                    });
                    if done.unwrap_or(true) { break; }
                    #[cfg(test)] if let Ok(Some(gate)) = this.update(cx, |w, _| w.automatic_title.validation_gate.take()) { let _ = gate.await; }
                    yield_turn().await;
                }
            }
        }).detach();
    }
    fn automatic_title_window_valid(&self, cx: &Context<Self>) -> bool {
        let Some(window) = self.automatic_title.window else { return false; };
        let owner = cx.entity().entity_id();
        window.read::<Workspace, _, _>(cx, |entity, _| entity.entity_id() == owner).unwrap_or(false)
    }
    #[cfg(test)]
    pub(super) fn automatic_title_credits_available(&self) -> bool { self.automatic_title.credit.available() }
    #[cfg(test)]
    pub(super) fn automatic_title_credit_checker(&self) -> impl Fn() -> bool + Send + 'static {
        let credit = self.automatic_title.credit.clone(); move || credit.available()
    }
    #[cfg(test)]
    pub(super) fn automatic_title_command_count(&self) -> usize { self.automatic_title.commands.len() }
    #[cfg(test)]
    pub(super) fn cancel_automatic_title_worker_for_test(&mut self) {
        if let Some(run) = &self.automatic_title.running { run.cancel.store(true, Ordering::Release); }
    }
    fn automatic_row_valid(&self, row: &Row) -> bool {
        let Some(tab) = self.tabs.get(row.index) else { return false; };
        let latest = self.automatic_title.latest.get(row.index / LATEST_PAGE).and_then(Option::as_ref)
            .and_then(|slots| slots.get(row.index % LATEST_PAGE)).and_then(Option::as_ref);
        if latest.is_some_and(|latest| latest.tab != row.tab || !Arc::ptr_eq(&latest.instance, &row.instance)) { return false; }
        let (revision, activity) = latest.map(|latest| (&latest.revision, &latest.activity)).unwrap_or((&row.revision, &row.activity));
        tab.session_id == row.tab && Arc::ptr_eq(&tab.instance, &row.instance)
            && tab.buffer.raw_revision == *revision && tab.content_activity == *activity
            && path_identity(tab) == row.path_identity && tab.buffer.new_label_is_custom == row.custom
            && label_identity(tab.buffer.new_label.as_ref()) == label_identity(row.label.as_ref())
    }
    fn finish_automatic_title(&mut self, id: Uuid, context: &ContentRevision, policy: &crate::settings::TitlePolicy,
        result: Result<(Vec<Row>, usize), &'static str>, cx: &mut Context<Self>) -> Option<Vec<Row>>
    {
        let valid = !self.window_closed && self.automatic_title_window_valid(cx) && self.automatic_title.context == *context && crate::settings::untitled_title_policy() == *policy
            && self.automatic_title.running.as_ref().is_some_and(|run| run.id == id && !run.cancel.load(Ordering::Acquire));
        if !valid { cx.background_spawn(async move { drop(result); }).detach(); self.finish_automatic_title_failure(id, "Automatic-name context changed; the last accepted name is retained", cx); return None; }
        match result {
            Err(error) => { self.finish_automatic_title_failure(id, error, cx); None },
            Ok((rows, budget)) => { self.automatic_title.work_budget = budget;
                if self.automatic_title.notices.is_empty() { Some(rows) } else { self.automatic_title.shadow = Some(rows); self.automatic_title.running.take(); self.drive_automatic_titles(cx); None }
            }
        }
    }
    fn finish_automatic_title_failure(&mut self, id: Uuid, error: &'static str, cx: &mut Context<Self>) {
        if self.automatic_title.running.as_ref().is_some_and(|run| run.id == id) { self.discard_automatic_titles(Some(error), cx); }
    }
    fn complete_automatic_titles(&mut self, cx: &mut Context<Self>) {
        self.automatic_title.running.take(); self.automatic_title.policy = None; self.automatic_title.work_budget = history::WORK_LIMIT;
        let latest = std::mem::take(&mut self.automatic_title.latest); cx.background_spawn(async move { drop(latest); }).detach();
        for waiter in std::mem::take(&mut self.automatic_title.waiters) { let _ = waiter.send(()); }
        self.schedule_autosave(cx);
    }
    fn discard_automatic_titles(&mut self, error: Option<&'static str>, cx: &mut Context<Self>) {
        self.automatic_title.running.take(); let discarded = (std::mem::take(&mut self.automatic_title.notices), self.automatic_title.shadow.take());
        cx.background_spawn(async move { drop(discarded); }).detach(); self.automatic_title.policy = None; self.automatic_title.suspended = error.is_some(); self.complete_automatic_titles(cx);
        if let Some(error) = error { self.set_status(error); cx.notify(); }
    }
    pub(crate) fn automatic_name_context_changed(&mut self, cx: &mut Context<Self>) {
        self.automatic_title.context = Default::default();
        if self.automatic_title.running.is_some() || !self.automatic_title.notices.is_empty() { self.discard_automatic_titles(Some("Automatic-name context changed; the last accepted name is retained"), cx); }
    }
    pub(crate) fn invalidate_automatic_name_context(&mut self) {
        self.automatic_title.context = Default::default();
        if let Some(running) = &self.automatic_title.running { running.cancel.store(true, Ordering::Release); }
    }
    pub(crate) fn wait_for_automatic_titles(&mut self, cx: &mut Context<Self>) -> Option<oneshot::Receiver<()>> {
        self.begin_automatic_title_admission();
        if self.automatic_title.running.is_none() && self.automatic_title.notices.is_empty() { return None; }
        let count = self.automatic_title.waiters.len();
        let bytes = if count == self.automatic_title.waiters.capacity() { count.saturating_mul(2).saturating_add(1).saturating_mul(std::mem::size_of::<oneshot::Sender<()>>()) } else { std::mem::size_of::<oneshot::Sender<()>>() };
        if count >= MAX_COMMANDS || self.charge_automatic_title_admission(bytes).is_err() || self.automatic_title.waiters.try_reserve_exact(1).is_err() {
            self.discard_automatic_titles(Some(LIMIT_STATUS), cx); return None;
        }
        let (sender, receiver) = oneshot::channel(); self.automatic_title.waiters.push(sender); self.drive_automatic_titles(cx); Some(receiver)
    }
    pub(crate) fn automatic_titles_pending(&self) -> bool { self.automatic_title.running.is_some() || !self.automatic_title.notices.is_empty() }
    pub(crate) fn automatic_title_consumer_stamp(&self) -> (ContentRevision, ContentRevision) {
        (self.automatic_title.input_epoch.clone(), self.automatic_title.context.clone())
    }
    pub(crate) fn automatic_title_state_stamp(&self) -> (ContentRevision, ContentRevision, bool) {
        (self.automatic_title.input_epoch.clone(), self.automatic_title.context.clone(), self.automatic_titles_pending())
    }
    fn accept_automatic_candidate(&mut self, index: usize, candidate: &Candidate) -> Result<(), &'static str> {
        let name = candidate.as_str()?;
        let conflict = self.tabs.iter().any(|tab| { let full = if let Some(path) = tab.buffer.file_path.as_ref() { path.as_os_str().as_encoded_bytes() }
            else { tab.buffer.new_label.as_ref().map(|label| label.as_bytes()).unwrap_or(b"new") }; crate::automatic_title::same_full_name(name, full) });
        if conflict { return Ok(()); }
        let Some(tab) = self.tabs.get_mut(index) else { return Ok(()); };
        if tab.buffer.file_path.is_some() || tab.buffer.new_label_is_custom { return Ok(()); }
        tab.buffer.new_label = Some(candidate.to_string()?.into()); self.automatic_title.context = Default::default(); Ok(())
    }
    pub(crate) fn set_automatic_title_window(&mut self, window: &gpui::Window) { self.automatic_title.window = Some(window.window_handle()); }
    pub(crate) fn cancel_automatic_titles_for_recovery(&mut self, cx: &mut Context<Self>) {
        if self.automatic_titles_pending() { self.discard_automatic_titles(Some("Automatic title work cancelled for final recovery; the accepted name and current document are retained"), cx); }
    }
    pub(crate) fn wait_automatic_titles_before_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.automatic_title.closing { return true; }
        let Some(window) = self.automatic_title.window else { self.cancel_automatic_titles_for_recovery(cx); return false; };
        let Some(ready) = self.wait_for_automatic_titles(cx) else { return false; };
        self.automatic_title.closing = true; let owner = cx.entity().entity_id();
        cx.spawn(async move |_, cx| {
            let _ = ready.await;
            let _ = window.update(cx, |root, window, cx| {
                let Ok(entity) = root.downcast::<Workspace>() else { return; };
                if entity.entity_id() != owner { return; }
                entity.update(cx, |w, cx| {
                    w.automatic_title.closing = false;
                    if !w.window_closed && crate::app::quit::save_before_window_close(w, cx) { window.remove_window(); }
                });
            });
        }).detach(); true
    }
    pub(super) fn show_automatic_title_limit(&mut self) { if self.automatic_title.suspended { self.set_status(LIMIT_STATUS); } }
}
