// Large manual prefix commands prepare off-thread and publish one Rope state.
use super::{history::HistoryDirection, Workspace};
use crate::{buffer::{ContentRevision, SavePoint, UndoEntry, prepared::{Metadata, PreparedGroup}},
    edit_ops::menu_line_commands::prepare_indentation, tab::{InputFocus, TabViewMode}};
use gpui::{AppContext, Context, Window};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use uuid::Uuid;

// Bound the one foreground metadata capture before cloning either collection.
pub(crate) const MAX_PREPARED_ANNOTATIONS: usize = 65_536;

struct Stamp {
    tab: Uuid,
    source_epoch: super::source_input::SourceInputToken,
    input_context: gpui::InputContextToken,
    revision: ContentRevision,
    rope: ropey::Rope,
    rich: Option<ContentRevision>,
    path: Option<PathBuf>,
    encoding: crate::text::encoding::Encoding,
    eol: crate::text::line_ending::LineEnding,
    saved: Option<SavePoint>,
    dirty: bool,
    selection: (usize, usize, bool),
    metadata: Arc<Metadata>,
    undo: Boundary,
    redo: Boundary,
    autosave: u64,
    post_edit: u64,
    indent: (bool, usize),
}

#[derive(PartialEq)]
struct Boundary {
    len: usize,
    first: Option<ContentRevision>,
    last: Option<ContentRevision>,
}

impl Boundary {
    fn capture(stack: &[UndoEntry]) -> Self {
        let endpoints = stack.last().and_then(|entry| match entry {
            UndoEntry::Single(op) => Some((op.before_revision.clone(), op.after_revision.clone())),
            UndoEntry::Group { ops, .. } => Some((ops.first()?.before_revision.clone(), ops.last()?.after_revision.clone())),
            UndoEntry::Prepared(group) => Some((group.expected(false).clone(), group.expected(true).clone())),
        });
        Self { len: stack.len(), first: endpoints.as_ref().map(|p| p.0.clone()), last: endpoints.map(|p| p.1) }
    }
}

impl Stamp {
    fn capture(w: &Workspace, window: &Window) -> Result<Self, &'static str> {
        if !source_context(w, window) { return Err("Focus an editable Source or Raw View before changing line indentation"); }
        let Some(tab) = w.tabs.get(w.active) else { return Err("No active editor"); };
        let buffer = &tab.buffer;
        if buffer.bookmarks.len().saturating_add(buffer.marks.len()) > MAX_PREPARED_ANNOTATIONS {
            return Err("Clear marks or bookmarks to leave at most 65,536 annotations before preparing line indentation");
        }
        let mut marks = Vec::new();
        marks.try_reserve_exact(buffer.marks.len()).map_err(|_| "Not enough memory to prepare line indentation")?;
        marks.extend(buffer.marks.iter().cloned());
        Ok(Self { tab: tab.session_id, source_epoch: w.source_input_token(), input_context: window.input_context_token(),
            revision: buffer.raw_revision.clone(),
            rope: buffer.rope.clone(),
            rich: buffer.rich_doc.as_ref().map(|doc| doc.revision.clone()), path: buffer.file_path.clone(),
            encoding: buffer.encoding, eol: buffer.line_ending, saved: buffer.savepoint.clone(), dirty: buffer.dirty,
            selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            metadata: Arc::new(Metadata { bookmarks: buffer.bookmarks.clone(), marks, marker: tab.begin_select_marker }),
            undo: Boundary::capture(&tab.undo_stack), redo: Boundary::capture(&tab.redo_stack),
            autosave: w.autosave_generation, post_edit: w.post_edit_generation,
            indent: crate::settings::indent_for_lang(buffer.language.map(|language| language.key())) })
    }

    fn context_valid(&self, w: &Workspace, window: &Window) -> bool {
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        source_context(w, window) && tab.session_id == self.tab && tab.buffer.raw_revision == self.revision
            && w.source_input_token() == self.source_epoch && window.input_context_token() == self.input_context
            && tab.buffer.rope.is_instance(&self.rope)
            && (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging) == self.selection
            && w.autosave_generation == self.autosave && w.post_edit_generation == self.post_edit
    }

    fn valid(&self, w: &Workspace, window: &Window) -> bool {
        if !self.context_valid(w, window) { return false; }
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        let buffer = &tab.buffer;
        buffer.rich_doc.as_ref().map(|doc| &doc.revision) == self.rich.as_ref()
            && buffer.file_path == self.path && buffer.encoding == self.encoding && buffer.line_ending == self.eol
            && buffer.savepoint == self.saved && buffer.dirty == self.dirty
            && buffer.bookmarks == self.metadata.bookmarks && buffer.marks == self.metadata.marks
            && tab.begin_select_marker == self.metadata.marker
            && Boundary::capture(&tab.undo_stack) == self.undo && Boundary::capture(&tab.redo_stack) == self.redo
            && crate::settings::indent_for_lang(buffer.language.map(|language| language.key())) == self.indent
    }
}

fn source_context(w: &Workspace, window: &Window) -> bool {
    let Some(tab) = w.tabs.get(w.active) else { return false; };
    window.is_window_active() && w.focus_handle.is_focused(window) && w.input_focus(window) == InputFocus::Editor
        && tab.view_mode == TabViewMode::Edit && !tab.buffer.read_only && tab.buffer.loading.is_none()
        && tab.block_selection.is_none() && tab.edit_transaction.is_none() && !w.is_composing_text(window)
        && !crate::macros::execution_active()
}

pub(crate) struct PendingPreparedEdit {
    id: Uuid,
    stamp: Option<Stamp>,
    cancel: Arc<AtomicBool>,
    executor: gpui::BackgroundExecutor,
    #[cfg(test)]
    capture: std::time::Duration,
}

impl Drop for PendingPreparedEdit {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(Stamp { rope, metadata, .. }) = self.stamp.take() {
            dispose_on_worker((rope, metadata), &self.executor);
        }
    }
}

enum Operation { Indent, Replay { group: Arc<PreparedGroup>, undo: bool } }
struct ResultState {
    group: Arc<PreparedGroup>, metadata: Metadata,
    #[cfg(test)]
    worker: std::time::Duration,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) capture: std::time::Duration,
    pub(crate) worker: std::time::Duration,
    pub(crate) publish: std::time::Duration,
}

impl Workspace {
    pub(crate) fn cancel_prepared_edit(&mut self, _cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_prepared_edit.take() {
            pending.cancel.store(true, Ordering::Relaxed);
            drop(pending);
        }
    }

    pub(crate) fn retire_prepared_edit(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.pending_prepared_edit.as_ref().is_some_and(|pending| pending.stamp.as_ref().is_none_or(|stamp| !stamp.context_valid(self, window))) {
            self.cancel_prepared_edit(cx);
            if self.status_message.as_deref() == Some("Preparing line indentation; Escape cancels") {
                self.set_status("Line indentation cancelled because the editor changed; no changes applied");
                cx.notify();
            }
        }
    }

    pub(crate) fn start_prepared_indent(&mut self, increase: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.start_prepared(Operation::Indent, increase, window, cx)
    }

    /// Route before popping history: cancellation leaves both stacks intact.
    pub(super) fn start_prepared_replay(&mut self, direction: HistoryDirection, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let undo = matches!(direction, HistoryDirection::Undo);
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        let stack = if undo { &tab.undo_stack } else { &tab.redo_stack };
        let Some(UndoEntry::Prepared(group)) = stack.last() else { return false; };
        let group = group.clone();
        if tab.buffer.raw_revision != *group.expected(undo) {
            self.set_status("Indentation history no longer matches this document; no changes applied");
            cx.notify();
            return true;
        }
        self.start_prepared(Operation::Replay { group, undo }, false, window, cx);
        true
    }

    fn start_prepared(&mut self, operation: Operation, increase: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        #[cfg(test)]
        let capture_start = std::time::Instant::now();
        self.cancel_prepared_edit(cx);
        let stamp = match Stamp::capture(self, window) {
            Ok(stamp) => stamp,
            Err(message) => { self.set_status(message); cx.notify(); return false; }
        };
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let metadata = stamp.metadata.clone();
        let rope = self.buffer().rope.clone();
        let revision = stamp.revision.clone();
        let selection = self.selection();
        let (tabs, width) = stamp.indent;
        let undo = matches!(operation, Operation::Replay { undo: true, .. });
        #[cfg(test)]
        let gate = self.prepared_test_gate.take();
        #[cfg(test)]
        let publish_gate = self.prepared_test_publish_gate.take();
        let work = cx.background_spawn(async move {
            #[cfg(test)]
            if let Some(gate) = gate { let _ = gate.await; }
            #[cfg(test)]
            let worker_start = std::time::Instant::now();
            let group = match &operation {
                Operation::Indent => Arc::new(prepare_indentation(rope, revision, selection, tabs, width, increase, &worker_cancel)?),
                Operation::Replay { group, undo } => {
                    if !group.matches_rope(&rope, *undo, &worker_cancel)? {
                        return Err("Indentation history no longer matches this document; no changes applied");
                    }
                    group.clone()
                }
            };
            let mapped = group.remap(&metadata, undo, &worker_cancel)?;
            Ok::<_, &'static str>((operation, ResultState { group, metadata: mapped,
                #[cfg(test)] worker: worker_start.elapsed() }))
        });
        self.pending_prepared_edit = Some(PendingPreparedEdit { id, stamp: Some(stamp), cancel, executor: cx.background_executor().clone(),
            #[cfg(test)] capture: std::time::Duration::ZERO });
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.set_status("Preparing line indentation; Escape cancels");
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let mut result = Some(work.await);
            #[cfg(test)]
            if let Some(gate) = publish_gate { let _ = gate.await; }
            let updated = this.update_in(cx, |w, window, cx| {
                if let Some(result) = result.take() { w.finish_prepared(id, result, window, cx); }
            });
            if updated.is_err() {
                let _ = this.update(cx, |w, cx| {
                    if w.pending_prepared_edit.as_ref().is_some_and(|pending| pending.id == id) { w.cancel_prepared_edit(cx); }
                });
            }
            if let Some(result) = result { dispose_on_worker(result, cx.background_executor()); }
        }).detach();
        #[cfg(test)]
        if let Some(pending) = self.pending_prepared_edit.as_mut() { pending.capture = capture_start.elapsed(); }
        true
    }

    fn finish_prepared(&mut self, id: Uuid, result: Result<(Operation, ResultState), &'static str>, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(test)]
        let publish_start = std::time::Instant::now();
        let Some(pending) = self.pending_prepared_edit.as_ref() else { drop_on_worker(result, cx); return; };
        if pending.id != id { drop_on_worker(result, cx); return; }
        let valid = pending.stamp.as_ref().is_some_and(|stamp| stamp.valid(self, window));
        #[cfg(test)]
        let capture = pending.capture;
        self.cancel_prepared_edit(cx);
        if !valid {
            drop_on_worker(result, cx);
            if self.status_message.as_deref() == Some("Preparing line indentation; Escape cancels") {
                self.set_status("Line indentation cancelled because the editor changed; no changes applied");
                cx.notify();
            }
            return;
        }
        let (operation, state) = match result {
            Ok(state) => state,
            Err(message) => { self.set_status(message); cx.notify(); return; }
        };
        if state.group.ops().is_empty() {
            drop_on_worker(state, cx);
            if self.status_message.as_deref() == Some("Preparing line indentation; Escape cancels") { self.status_message = None; }
            cx.notify(); return;
        }
        let undo = matches!(operation, Operation::Replay { undo: true, .. });
        #[cfg(test)]
        if self.prepared_fail_reserve { drop_on_worker(state, cx); self.set_status("Not enough memory to publish line indentation"); cx.notify(); return; }
        let tab = self.active_tab_mut();
        let destination = if undo { &mut tab.redo_stack } else { &mut tab.undo_stack };
        if destination.try_reserve(1).is_err() { drop_on_worker(state, cx); self.set_status("Not enough memory to publish line indentation"); cx.notify(); return; }
        let mut old_redo = None;
        match operation {
            Operation::Indent => {
                tab.undo_stack.push(UndoEntry::Prepared(state.group.clone()));
                old_redo = Some(std::mem::take(&mut tab.redo_stack));
            }
            Operation::Replay { group, undo } => {
                let origin = if undo { &mut tab.undo_stack } else { &mut tab.redo_stack };
                if !matches!(origin.last(), Some(UndoEntry::Prepared(top)) if Arc::ptr_eq(top, &group)) {
                    drop_on_worker(state, cx);
                    self.set_status("Indentation history changed; no changes applied"); cx.notify(); return;
                }
                let Some(entry) = origin.pop() else { return; };
                if undo { tab.redo_stack.push(entry); } else { tab.undo_stack.push(entry); }
            }
        }
        let (rope, revision, selection) = state.group.target(undo);
        tab.buffer.rope = rope;
        tab.buffer.raw_revision = revision;
        let old_metadata = (
            std::mem::replace(&mut tab.buffer.bookmarks, state.metadata.bookmarks),
            std::mem::replace(&mut tab.buffer.marks, state.metadata.marks),
        );
        tab.begin_select_marker = state.metadata.marker;
        tab.selection = selection;
        tab.desired_col_x = None;
        tab.wrap_cache = None;
        tab.buffer.invalidate_flat_text();
        tab.buffer.refresh_dirty();
        drop_on_worker((old_redo, old_metadata), cx);
        self.after_edit_flush(cx);
        self.commit_caret(cx);
        // Find's existing refresh scans synchronously in this prerequisite.
        // Drop stale ranges without flattening the large new document here.
        let stale_find = (std::mem::take(&mut self.find_state.matches), self.find_state.rich_doc_block_offsets.take());
        self.find_state.current = None;
        if self.find_state.open && !self.find_state.query.is_empty() {
            self.find_state.status = Some("Document changed; run Find again to refresh matches".into());
        }
        drop_on_worker(stale_find, cx);
        #[cfg(test)]
        self.prepared_timings.push(Timing { capture, worker: state.worker, publish: publish_start.elapsed() });
    }
}

// Releasing a discarded plan or metadata snapshot is linear in its size.
// It belongs on the worker even when the request was cancelled by typing.
fn drop_on_worker<T: Send + 'static>(value: T, cx: &mut Context<Workspace>) {
    dispose_on_worker(value, cx.background_executor());
}

fn dispose_on_worker<T: Send + 'static>(value: T, executor: &gpui::BackgroundExecutor) {
    executor.spawn(async move {
        #[cfg(test)]
        let start = std::time::Instant::now();
        drop(value);
        #[cfg(test)]
        if std::env::var_os("NOTEPAD_STUDIO_BULK_BENCH").is_some() {
            println!("bulk disposal type={} worker={:.3}ms", std::any::type_name::<T>(), start.elapsed().as_secs_f64()*1000.);
        }
    }).detach();
}
