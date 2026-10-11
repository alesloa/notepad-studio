// Post-edit syntax refresh: original scheduling around tree-sitter's cancellable parser.
// Language selection follows PowerEditor/src/NppCommands.cpp:4008-4017.
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use gpui::{AppContext, BackgroundExecutor, Context, Task};
use ropey::Rope;
use uuid::Uuid;

use super::Workspace;
use crate::buffer::ContentRevision;
use crate::syntax::{HighlightError, HighlightSpan, LanguageId};

const MAX_PENDING_TABS: usize = 256;
const DEBOUNCE: Duration = Duration::from_millis(100);
const QUEUE_STATUS: &str = "Syntax highlighting queue limit (256 documents) reached; select this language again after parsing finishes to retry";

#[derive(Clone, Copy, PartialEq, Eq)]
struct Policy {
    restricted: bool,
    disabled: bool,
    threshold: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_syntax_cap_cannot_be_disabled_and_smaller_user_cap_is_honored() {
        let hard = crate::io_stream::HIGHLIGHT_SIZE_LIMIT as usize;
        let unrestricted = Policy {
            restricted: false,
            disabled: false,
            threshold: 1,
        };
        assert!(!unrestricted.skips(hard));
        assert!(unrestricted.skips(hard + 1));
        let restricted = Policy {
            restricted: true,
            disabled: true,
            threshold: 10,
        };
        assert!(!restricted.skips(9));
        assert!(restricted.skips(10));
        assert!(restricted.skips(hard));
        let _settings = crate::settings::override_for_test(crate::settings::Settings {
            large_file_restriction_enabled: false,
            large_file_disable_syntax: false,
            ..Default::default()
        });
        assert!(crate::syntax::should_skip_syntax(
            hard as u64 + 1,
            &crate::settings::snapshot()
        ));
        assert!(!crate::syntax::should_skip_syntax(
            hard as u64,
            &crate::settings::snapshot()
        ));
    }

    #[test]
    fn restricted_and_cancelled_requests_refuse_before_flattening() {
        let rope = Rope::from_str("fn café() {}\r\n");
        let mut request = Request {
            id: Uuid::new_v4(),
            tab: Uuid::new_v4(),
            revision: ContentRevision::default(),
            rope,
            language: LanguageId::Rust,
            policy: Policy {
                restricted: true,
                disabled: true,
                threshold: 1,
            },
            cancel: Arc::new(AtomicUsize::new(0)),
            status: None,
        };
        assert!(matches!(parse(&request), Err(HighlightError::SizeLimit)));
        request.policy.restricted = false;
        request.cancel.store(1, Ordering::Relaxed);
        assert!(matches!(parse(&request), Err(HighlightError::Cancelled)));
    }
}

impl Policy {
    fn current() -> Self {
        let settings = crate::settings::snapshot();
        Self {
            restricted: settings.large_file_restriction_enabled,
            disabled: settings.large_file_disable_syntax,
            threshold: settings.large_file_threshold_bytes,
        }
    }

    fn skips(self, size: usize) -> bool {
        size as u64 > crate::io_stream::HIGHLIGHT_SIZE_LIMIT
            || (self.restricted && self.disabled && size as u64 >= self.threshold)
    }
}

struct Request {
    id: Uuid,
    tab: Uuid,
    revision: ContentRevision,
    rope: Rope,
    language: LanguageId,
    policy: Policy,
    cancel: Arc<AtomicUsize>,
    status: Option<String>,
}

struct Delayed {
    request: Request,
    _timer: Task<()>,
}

struct Running {
    id: Uuid,
    tab: Uuid,
    cancel: Arc<AtomicUsize>,
}

type Parsed = (
    Request,
    Result<Vec<HighlightSpan>, HighlightError>,
    Duration,
);

#[derive(Default)]
pub(super) struct Runtime {
    latest: HashMap<Uuid, Uuid>,
    delayed: HashMap<Uuid, Delayed>,
    queued: VecDeque<Request>,
    running: Option<Running>,
    executor: Option<BackgroundExecutor>,
    refused: Option<Uuid>,
    #[cfg(test)]
    hold: bool,
    #[cfg(test)]
    held: Option<Parsed>,
    #[cfg(test)]
    pub(super) started: usize,
    #[cfg(test)]
    pub(super) capture_time: Duration,
    #[cfg(test)]
    pub(super) worker_time: Duration,
    #[cfg(test)]
    pub(super) install_time: Duration,
}

fn dispose<T: Send + 'static>(executor: &BackgroundExecutor, value: T) {
    executor
        .spawn(async move {
            drop(value);
        })
        .detach();
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.close();
    }
}

impl Runtime {
    pub(super) fn close(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.store(1, Ordering::Relaxed);
        }
        self.latest.clear();
        self.refused = None;
        let requests: Vec<_> = self
            .delayed
            .drain()
            .map(|(_, delayed)| delayed.request)
            .chain(self.queued.drain(..))
            .collect();
        if let Some(executor) = &self.executor {
            dispose(executor, requests);
            #[cfg(test)]
            dispose(executor, self.held.take());
        }
    }
}

fn parse(request: &Request) -> Result<Vec<HighlightSpan>, HighlightError> {
    if request.cancel.load(Ordering::Relaxed) != 0 {
        return Err(HighlightError::Cancelled);
    }
    if request.policy.skips(request.rope.len_bytes()) {
        return Err(HighlightError::SizeLimit);
    }
    let source = request.rope.to_string();
    crate::syntax::highlight_cancellable(request.language, &source, Some(&request.cancel))
}

impl Workspace {
    /// Syntax jobs debounce per tab. Recovery keeps its independent workspace debounce.
    pub(crate) fn schedule_reparse(&mut self, cx: &mut Context<Self>) {
        if self.window_closed {
            self.syntax_refresh.close();
            return;
        }
        self.request_syntax_refresh(DEBOUNCE, cx);
        self.post_edit_generation = self.post_edit_generation.wrapping_add(1);
        let generation = self.post_edit_generation;
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(DEBOUNCE).await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.post_edit_generation == generation {
                    workspace.save_session_eager(cx);
                }
            });
        })
        .detach();
    }

    fn syntax_source_valid(&self, request: &Request) -> bool {
        !self.window_closed
            && self.syntax_refresh.latest.get(&request.tab) == Some(&request.id)
            && request.cancel.load(Ordering::Relaxed) == 0
            && request.policy == Policy::current()
            && self
                .tabs
                .iter()
                .find(|tab| tab.session_id == request.tab)
                .is_some_and(|tab| {
                    tab.buffer.loading.is_none()
                        && tab.buffer.language == Some(request.language)
                        && tab.buffer.rope.is_instance(&request.rope)
                })
    }

    fn syntax_request_valid(&self, request: &Request) -> bool {
        self.syntax_source_valid(request)
            && self
                .tabs
                .iter()
                .find(|tab| tab.session_id == request.tab)
                .is_some_and(|tab| tab.buffer.raw_revision == request.revision)
    }

    pub(super) fn retire_syntax_tab(&mut self, tab: Uuid, cx: &mut Context<Self>) {
        self.syntax_refresh.latest.remove(&tab);
        if let Some(running) = self
            .syntax_refresh
            .running
            .as_ref()
            .filter(|running| running.tab == tab)
        {
            running.cancel.store(1, Ordering::Relaxed);
        }
        let executor = cx.background_executor();
        if let Some(delayed) = self.syntax_refresh.delayed.remove(&tab) {
            dispose(executor, delayed.request);
        }
        let mut retained = VecDeque::new();
        for request in self.syntax_refresh.queued.drain(..) {
            if request.tab == tab {
                dispose(executor, request);
            } else {
                retained.push_back(request);
            }
        }
        self.syntax_refresh.queued = retained;
    }

    /// Root's tab-list integration calls this immediately after closing/restoring tabs.
    pub(crate) fn retire_closed_syntax_refresh(&mut self, cx: &mut Context<Self>) {
        let closed: Vec<_> = self
            .syntax_refresh
            .latest
            .keys()
            .copied()
            .filter(|id| !self.tabs.iter().any(|tab| tab.session_id == *id))
            .collect();
        for id in closed {
            self.retire_syntax_tab(id, cx);
        }
    }

    pub(super) fn request_syntax_refresh(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.window_closed { self.syntax_refresh.close(); return; }
        let Some(tab) = self.tabs.get(self.active) else { return; };
        self.request_syntax_refresh_for_tab(tab.session_id, delay, cx);
    }

    /// File loading can finish after another document becomes active.
    pub(crate) fn request_syntax_refresh_for_tab(&mut self, id: Uuid, delay: Duration, cx: &mut Context<Self>) {
        if self.window_closed {
            self.syntax_refresh.close();
            return;
        }
        let capture_started = Instant::now();
        self.syntax_refresh.executor = Some(cx.background_executor().clone());
        self.retire_closed_syntax_refresh(cx);
        let Some(index) = self.tabs.iter().position(|tab| tab.session_id == id) else { return; };
        let Some(tab) = self.tabs.get(index) else { return; };
        let language = tab.buffer.language;
        let policy = Policy::current();
        let size = tab.buffer.rope.len_bytes();
        let skip = tab.buffer.loading.is_some() || language.is_none() || policy.skips(size);
        if skip {
            self.retire_syntax_tab(id, cx);
            if let Some(tab) = self.tabs.get_mut(index) {
                dispose(
                    cx.background_executor(),
                    std::mem::take(&mut tab.buffer.highlights),
                );
            }
            if index == self.active && delay.is_zero() && size as u64 > crate::io_stream::HIGHLIGHT_SIZE_LIMIT {
                self.set_status(HighlightError::SizeLimit.to_string());
            }
            cx.notify();
            return;
        }
        if !self.syntax_refresh.latest.contains_key(&id)
            && self.syntax_refresh.latest.len() >= MAX_PENDING_TABS
        {
            self.syntax_refresh.refused = Some(id);
            if index == self.active { self.set_status(QUEUE_STATUS); }
            if let Some(tab) = self.tabs.get_mut(index) {
                dispose(
                    cx.background_executor(),
                    std::mem::take(&mut tab.buffer.highlights),
                );
            }
            cx.notify();
            return;
        }
        let Some(language) = language else {
            return;
        };
        let mut request = Request {
            id: Uuid::new_v4(),
            tab: id,
            revision: tab.buffer.raw_revision.clone(),
            rope: tab.buffer.rope.clone(),
            language,
            policy,
            cancel: Arc::new(AtomicUsize::new(0)),
            status: self.status_message.clone(),
        };
        if self.syntax_refresh.refused == Some(id) {
            self.syntax_refresh.refused = None;
            if self.status_message.as_deref() == Some(QUEUE_STATUS) {
                self.status_message = None;
                cx.notify();
            }
        }
        request.status = self.status_message.clone();
        self.retire_syntax_tab(id, cx);
        self.syntax_refresh.latest.insert(id, request.id);
        if delay.is_zero() {
            self.syntax_refresh.queued.push_back(request);
            self.start_next_syntax_refresh(cx);
        } else {
            let request_id = request.id;
            let timer = cx.background_executor().timer(delay);
            let task = cx.spawn(async move |this, cx| {
                timer.await;
                let _ = this.update(cx, |workspace, cx| {
                    workspace.enqueue_syntax_refresh(id, request_id, cx)
                });
            });
            self.syntax_refresh.delayed.insert(
                id,
                Delayed {
                    request,
                    _timer: task,
                },
            );
        }
        #[cfg(test)]
        {
            self.syntax_refresh.capture_time = capture_started.elapsed();
        }
        #[cfg(not(test))]
        let _ = capture_started;
    }

    fn enqueue_syntax_refresh(&mut self, tab: Uuid, id: Uuid, cx: &mut Context<Self>) {
        if self.syntax_refresh.latest.get(&tab) != Some(&id) {
            return;
        }
        let Some(mut delayed) = self.syntax_refresh.delayed.remove(&tab) else {
            return;
        };
        // Single-entry Undo/Redo restores its canonical history revision after
        // apply_edit schedules this debounce. Finalize identity only while the
        // exact captured Rope/context still matches; running results never retarget.
        if !self.syntax_source_valid(&delayed.request) {
            self.syntax_refresh.latest.remove(&tab);
            dispose(cx.background_executor(), delayed.request);
            self.start_next_syntax_refresh(cx);
            return;
        }
        if let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.session_id == delayed.request.tab)
        {
            delayed.request.revision = tab.buffer.raw_revision.clone();
        }
        self.syntax_refresh.queued.push_back(delayed.request);
        self.start_next_syntax_refresh(cx);
    }

    fn start_next_syntax_refresh(&mut self, cx: &mut Context<Self>) {
        if self.window_closed {
            self.syntax_refresh.close();
            return;
        }
        if self.syntax_refresh.running.is_some() {
            return;
        }
        while let Some(request) = self.syntax_refresh.queued.pop_front() {
            if !self.syntax_request_valid(&request) {
                if self.syntax_refresh.latest.get(&request.tab) == Some(&request.id) {
                    self.syntax_refresh.latest.remove(&request.tab);
                }
                dispose(cx.background_executor(), request);
                continue;
            }
            self.syntax_refresh.running = Some(Running {
                id: request.id,
                tab: request.tab,
                cancel: request.cancel.clone(),
            });
            #[cfg(test)]
            {
                self.syntax_refresh.started += 1;
            }
            let executor = cx.background_executor().clone();
            let task = cx.background_spawn(async move {
                let started = Instant::now();
                let result = parse(&request);
                (request, result, started.elapsed())
            });
            cx.spawn(async move |this, cx| {
                let mut parsed = Some(task.await);
                let _ = this.update(cx, |workspace, cx| {
                    let Some(parsed) = parsed.take() else {
                        return;
                    };
                    #[cfg(test)]
                    if workspace.syntax_refresh.hold && !workspace.window_closed {
                        workspace.syntax_refresh.held = Some(parsed);
                        return;
                    }
                    workspace.finish_syntax_refresh(parsed, cx);
                });
                // Entity/window teardown must not destroy a large stale result on the UI thread.
                if let Some(parsed) = parsed {
                    dispose(&executor, parsed);
                }
            })
            .detach();
            break;
        }
    }

    fn finish_syntax_refresh(&mut self, parsed: Parsed, cx: &mut Context<Self>) {
        let install_started = Instant::now();
        let (request, result, duration) = parsed;
        let current = self.syntax_request_valid(&request);
        if self
            .syntax_refresh
            .running
            .as_ref()
            .is_some_and(|running| running.id == request.id)
        {
            self.syntax_refresh.running = None;
        }
        if self.syntax_refresh.latest.get(&request.tab) == Some(&request.id) {
            self.syntax_refresh.latest.remove(&request.tab);
        }
        if current {
            match result {
                Ok(highlights) => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.session_id == request.tab)
                    {
                        let retired = super::language::install_highlights(
                            &mut tab.buffer,
                            request.language,
                            &request.rope,
                            highlights,
                        );
                        dispose(cx.background_executor(), retired);
                        self.last_lines.clear();
                        cx.notify();
                    } else {
                        dispose(cx.background_executor(), highlights);
                    }
                }
                Err(HighlightError::Cancelled) => {}
                Err(error) => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.session_id == request.tab)
                    {
                        dispose(
                            cx.background_executor(),
                            std::mem::take(&mut tab.buffer.highlights),
                        );
                        self.last_lines.clear();
                        cx.notify();
                    }
                    if self
                        .tabs
                        .get(self.active)
                        .is_some_and(|tab| tab.session_id == request.tab)
                        && self.status_message == request.status
                    {
                        self.set_status(error.to_string());
                        cx.notify();
                    }
                }
            }
        } else {
            dispose(cx.background_executor(), result);
        }
        dispose(cx.background_executor(), request);
        #[cfg(test)]
        {
            self.syntax_refresh.worker_time = duration;
            self.syntax_refresh.install_time = install_started.elapsed();
        }
        #[cfg(not(test))]
        let _ = (duration, install_started);
        self.start_next_syntax_refresh(cx);
    }

    #[cfg(test)]
    pub(super) fn hold_syntax_refresh(&mut self, hold: bool) {
        self.syntax_refresh.hold = hold;
    }

    #[cfg(test)]
    pub(super) fn syntax_pending_for_test(&self) -> (usize, usize, bool) {
        (
            self.syntax_refresh.latest.len(),
            self.syntax_refresh.queued.len(),
            self.syntax_refresh.held.is_some(),
        )
    }

    #[cfg(test)]
    pub(super) fn release_syntax_refresh(&mut self, cx: &mut Context<Self>) {
        self.syntax_refresh.hold = false;
        if let Some(parsed) = self.syntax_refresh.held.take() {
            self.finish_syntax_refresh(parsed, cx);
        }
    }
}

#[cfg(test)]
#[path = "stream_syntax_tests.rs"]
mod stream_tests;

#[cfg(test)]
#[path = "stream_eof_bench.rs"]
mod stream_bench;
