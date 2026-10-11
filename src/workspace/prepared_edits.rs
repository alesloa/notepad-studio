// Large manual prefix commands prepare off-thread and publish one Rope state.
use super::{history::HistoryDirection, Workspace};
use crate::{buffer::{ContentRevision, SavePoint, UndoEntry, prepared::{Metadata, PreparedGroup}},
    edit_ops::{menu_line_commands::prepare_indentation, bookmark_edits::{BookmarkCommand, BookmarkInput, prepare_bookmarks, prepare_paste}, line_sort::{SortCommand, SortFormat, prepare_sort}}, tab::{InputFocus, TabViewMode}};
use gpui::{AppContext, Context, Window};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use uuid::Uuid;

#[path = "prepared_macro_capture.rs"]
mod macro_capture;

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
    instance: Arc<()>,
    activity: ContentRevision,
    block: Option<crate::buffer::BlockSelection>,
    macro_allowed: bool,
    read_only_allowed: bool,
    block_allowed: bool,
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
    fn capture(w: &Workspace, window: &Window, macro_allowed: bool, read_only_allowed: bool, block_allowed: bool) -> Result<Self, &'static str> {
        if !source_context(w, window, macro_allowed, read_only_allowed, block_allowed) { return Err("Focus an editable Source or Raw View before preparing line edits; finish active edits or composition first"); }
        let Some(tab) = w.tabs.get(w.active) else { return Err("No active editor"); };
        let buffer = &tab.buffer;
        if buffer.bookmarks.len().saturating_add(buffer.marks.len()) > MAX_PREPARED_ANNOTATIONS {
            return Err("Clear marks or bookmarks to leave at most 65,536 annotations before preparing line edits");
        }
        let mut marks = Vec::new();
        marks.try_reserve_exact(buffer.marks.len()).map_err(|_| "Not enough memory to prepare line edits")?;
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
            indent: crate::settings::indent_for_lang(buffer.language.map(|language| language.key())),
            instance: tab.instance.clone(), activity: tab.content_activity.clone(), block: tab.block_selection,
            macro_allowed, read_only_allowed, block_allowed })
    }

    fn context_valid(&self, w: &Workspace, window: &Window) -> bool {
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        source_context(w, window, self.macro_allowed, self.read_only_allowed, self.block_allowed)
            && tab.session_id == self.tab && Arc::ptr_eq(&tab.instance, &self.instance)
            && tab.content_activity == self.activity && block_key(tab.block_selection) == block_key(self.block)
            && tab.buffer.raw_revision == self.revision
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

fn block_key(block: Option<crate::buffer::BlockSelection>) -> Option<(usize, usize, usize, usize, bool)> {
    block.map(|b| (b.anchor_line, b.head_line, b.anchor_col, b.head_col, b.is_dragging))
}

fn source_context(w: &Workspace, window: &Window, macro_allowed: bool, read_only_allowed: bool, block_allowed: bool) -> bool {
    let Some(tab) = w.tabs.get(w.active) else { return false; };
    window.is_window_active() && w.focus_handle.is_focused(window) && w.input_focus(window) == InputFocus::Editor
        && !w.window_closed && tab.view_mode == TabViewMode::Edit && (!tab.buffer.read_only || read_only_allowed) && tab.buffer.loading.is_none()
        && (tab.block_selection.is_none() || block_allowed) && tab.edit_transaction.is_none() && !w.is_composing_text(window)
        && (macro_allowed || !crate::macros::execution_active())
}

pub(crate) struct PendingPreparedEdit {
    id: Uuid,
    stamp: Option<Stamp>,
    cancel: Arc<AtomicBool>,
    executor: gpui::BackgroundExecutor,
    status: &'static str,
    clipboard: bool,
    playback: Option<Uuid>,
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

enum Operation {
    Indent, Replay { group: Arc<PreparedGroup>, undo: bool },
    Bookmarks { command: BookmarkCommand, paste: Option<gpui::ClipboardItem> },
    Sort(SortCommand),
    Merge { entries: futures::channel::oneshot::Receiver<Result<Vec<UndoEntry>, &'static str>>, base: usize, selection: crate::buffer::Selection },
}
struct ResultState {
    group: Arc<PreparedGroup>, metadata: Metadata,
    history: Option<Vec<UndoEntry>>,
    clipboard: Option<String>,
    clipboard_history: Option<String>,
    #[cfg(test)]
    worker: std::time::Duration,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) capture: std::time::Duration,
    pub(crate) worker: std::time::Duration,
    pub(crate) publish: std::time::Duration,
    pub(crate) clipboard: std::time::Duration,
}

impl Workspace {
    pub(crate) fn active_macro_playback_id(&self) -> Option<Uuid> { self.regex_replacement.macro_playback.as_ref().map(|playback| playback.id) }
    pub(crate) fn cancel_prepared_edit(&mut self, _cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_prepared_edit.take() {
            pending.cancel.store(true, Ordering::Relaxed);
            drop(pending);
        }
    }

    pub(crate) fn cancel_prepared_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_prepared_edit.as_ref().filter(|pending| pending.clipboard) {
            let (status, playback) = (pending.status, pending.playback);
            self.cancel_prepared_edit(cx);
            if self.status_message.as_deref() == Some(status) { self.status_message = None; }
            if playback.is_some() && playback == self.active_macro_playback_id() { self.stop_prepared_macro("a newer clipboard command took precedence"); }
            cx.notify();
        }
    }

    pub(crate) fn retire_prepared_edit(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.pending_prepared_edit.as_ref().is_some_and(|pending| pending.stamp.as_ref().is_none_or(|stamp| !stamp.context_valid(self, window))) {
            let status = self.pending_prepared_edit.as_ref().map(|pending| pending.status);
            self.cancel_prepared_edit(cx);
            if self.status_message.as_deref() == status {
                self.stop_prepared_macro("the editor changed");
                self.set_status("Line edits cancelled because the editor changed; no changes applied");
                cx.notify();
            }
        }
    }

    pub(crate) fn start_prepared_indent(&mut self, increase: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.start_prepared(Operation::Indent, increase, window, cx)
    }

    pub(crate) fn start_prepared_bookmarks(&mut self, command: BookmarkCommand, paste: Option<gpui::ClipboardItem>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.start_prepared(Operation::Bookmarks { command, paste }, false, window, cx)
    }

    pub(crate) fn bookmark_source_available(&mut self, command: BookmarkCommand, window: &Window, cx: &mut Context<Self>) -> bool {
        if source_context(self, window, true, command == BookmarkCommand::Inverse, command == BookmarkCommand::Inverse) { return true; }
        self.set_status("Focus an editable Source or Raw View before preparing line edits; finish active edits or composition first");
        cx.notify(); false
    }

    pub(crate) fn start_prepared_sort(&mut self, command: SortCommand, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.start_prepared(Operation::Sort(command), false, window, cx)
    }

    pub(crate) fn stop_prepared_macro(&mut self, reason: &str) {
        if let Some(playback) = self.regex_replacement.macro_playback.take() { self.set_status(playback.stopped(reason)); }
    }

    pub(in crate::workspace) fn start_prepared_macro_merge(&mut self, mut playback: super::macros::search_macro::Playback, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        let capture = match macro_capture::Capture::new(0, tab.undo_stack.len()) {
            Ok(capture) => capture,
            Err(message) => {
                self.set_status(message); cx.notify(); return false;
            }
        };
        let (sender, receiver) = futures::channel::oneshot::channel();
        if tab.undo_stack.len() < playback.undo_base {
            self.set_status("Not enough memory to combine macro history; Undo remains in separate steps"); cx.notify(); return false;
        }
        playback.undo_len = tab.undo_stack.len(); playback.redo_len = tab.redo_stack.len();
        let operation = Operation::Merge { entries: receiver, base: playback.undo_base, selection: playback.selection };
        self.regex_replacement.macro_playback = Some(playback);
        if !self.start_prepared(operation, false, window, cx) { self.stop_prepared_macro("Macro history could not start"); return false; }
        let Some(id) = self.pending_prepared_edit.as_ref().map(|pending| pending.id) else { return false; };
        macro_capture::start(capture, sender, id, window, cx);
        false
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
        self.settle_source_input_edit_caret();
        self.sync_source_input_caret();
        let macro_allowed = !matches!(operation, Operation::Indent) &&
            (!matches!(&operation, Operation::Replay { group, .. } if !group.is_line_edit()));
        let read_only_allowed = matches!(operation, Operation::Bookmarks { command: BookmarkCommand::Inverse, .. });
        let block_allowed = matches!(operation, Operation::Sort(_) | Operation::Merge { .. })
            || matches!(operation, Operation::Bookmarks { command: BookmarkCommand::Inverse, .. })
            || matches!(&operation, Operation::Replay { group, .. } if group.is_line_edit());
        let stamp = match Stamp::capture(self, window, macro_allowed, read_only_allowed, block_allowed) {
            Ok(stamp) => stamp,
            Err(message) => { drop_on_worker(operation, cx); self.set_status(message); cx.notify(); return false; }
        };
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let metadata = stamp.metadata.clone();
        let rope = self.buffer().rope.clone();
        let revision = stamp.revision.clone();
        let selection = self.selection();
        let block = stamp.block;
        let eol = stamp.eol;
        let encoding = stamp.encoding;
        let (tabs, width) = stamp.indent;
        let clipboard = matches!(operation, Operation::Bookmarks { command: BookmarkCommand::Cut, .. });
        let status = if matches!(operation, Operation::Indent)
            || matches!(&operation, Operation::Replay { group, .. } if !group.is_line_edit()) {
            "Preparing line indentation; Escape cancels"
        } else if matches!(operation, Operation::Replay { .. }) { "Preparing line history; Escape cancels"
        } else if matches!(operation, Operation::Sort(_)) { "Preparing line sorting; Escape cancels" }
        else if matches!(operation, Operation::Merge { .. }) { "Preparing macro history; Escape cancels" }
        else { "Preparing bookmark edits; Escape cancels" };
        let playback = self.regex_replacement.macro_playback.as_ref().map(|playback| playback.id);
        if let Operation::Bookmarks { command, .. } = &operation { crate::macros::record(command.event()); }
        if let Operation::Sort(command) = &operation { crate::macros::record(command.event()); }
        #[cfg(test)]
        let gate = self.prepared_test_gate.take();
        #[cfg(test)]
        let publish_gate = self.prepared_test_publish_gate.take();
        let work = cx.background_spawn(async move {
            let mut operation = operation;
            #[cfg(test)]
            if let Some(gate) = gate { let _ = gate.await; }
            #[cfg(test)]
            let worker_start = std::time::Instant::now();
            let (group, mapped, clipboard, history) = match &mut operation {
                Operation::Indent => {
                    let group = Arc::new(prepare_indentation(rope, revision, selection, tabs, width, increase, &worker_cancel)?);
                    let mapped = group.remap(&metadata, false, &worker_cancel)?;
                    (group, mapped, None, None)
                }
                Operation::Replay { group, undo } => {
                    if !group.matches_rope(&rope, *undo, &worker_cancel)? {
                        return Err("Indentation history no longer matches this document; no changes applied");
                    }
                    (group.clone(), group.remap(&metadata, *undo, &worker_cancel)?, None, None)
                }
                Operation::Bookmarks { command, paste } => {
                    let paste = paste.take().map(|item| prepare_paste(item.into_entries(), &worker_cancel)).transpose()?;
                    let result = prepare_bookmarks(rope, revision, selection, &metadata, *command, BookmarkInput { ending: eol, paste }, &worker_cancel)?;
                    (Arc::new(result.group), result.metadata, result.clipboard, None)
                }
                Operation::Sort(command) => {
                    let group = Arc::new(prepare_sort(rope, revision, selection, block, *command, SortFormat { eol: eol.as_str(), encoding }, &worker_cancel)?);
                    let mapped = group.remap(&metadata, false, &worker_cancel)?;
                    (group, mapped, None, None)
                }
                Operation::Merge { entries, base, selection: before_selection } => {
                    let mut entries = entries.await.map_err(|_| "Macro history capture cancelled")??;
                    let suffix = entries.get(*base..).ok_or("Macro history changed during capture")?;
                    let group = Arc::new(PreparedGroup::combine_history(suffix, rope, revision, *before_selection, selection, &worker_cancel)?);
                    entries.truncate(*base);
                    entries.try_reserve(1).map_err(|_| "Not enough memory to combine macro history; Undo remains in separate steps")?;
                    entries.push(UndoEntry::Prepared(group.clone()));
                    (group, (*metadata).clone(), None, Some(entries))
                }
            };
            let clipboard_history = clipboard.as_ref().map(|text| crate::clipboard_history::prepare_copy(text, &worker_cancel)).transpose()?;
            Ok::<_, &'static str>((operation, ResultState { group, metadata: mapped,
                clipboard, clipboard_history, history,
                #[cfg(test)] worker: worker_start.elapsed() }))
        });
        self.pending_prepared_edit = Some(PendingPreparedEdit { id, stamp: Some(stamp), cancel, executor: cx.background_executor().clone(), status, clipboard, playback,
            #[cfg(test)] capture: std::time::Duration::ZERO });
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.set_status(status);
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
        let playback_id = pending.playback;
        let status = pending.status;
        let valid = pending.stamp.as_ref().is_some_and(|stamp| stamp.valid(self, window))
            && playback_id.is_none_or(|id| self.regex_replacement.macro_playback.as_ref().is_some_and(|playback| playback.id == id && playback.valid(self, window)));
        #[cfg(test)]
        let capture = pending.capture;
        self.cancel_prepared_edit(cx);
        if !valid {
            drop_on_worker(result, cx);
            let preparing = self.status_message.as_deref() == Some(status);
            self.stop_prepared_macro("the editor changed; no changes applied");
            if preparing && self.active_macro_playback_id().is_none() && playback_id.is_none() {
                self.set_status("Line edits cancelled because the editor changed; no changes applied");
            }
            cx.notify();
            return;
        }
        let (operation, state) = match result {
            Ok(state) => state,
            Err(message) => { self.set_status(message); self.stop_prepared_macro(message); cx.notify(); return; }
        };
        if let Operation::Merge { .. } = &operation {
            let Some(history) = state.history else {
                drop_on_worker((operation, state.group, state.metadata, state.clipboard, state.clipboard_history), cx);
                self.stop_prepared_macro("Macro history preparation did not return a timeline; Undo remains in separate steps"); cx.notify(); return;
            };
            let old = std::mem::replace(&mut self.active_tab_mut().undo_stack, history);
            drop_on_worker((old, state.group, state.metadata, state.clipboard, state.clipboard_history), cx);
            drop_on_worker(operation, cx);
            if let Some(playback) = self.regex_replacement.macro_playback.take() {
                self.set_status(format!("Played macro ({} events)", playback.completed));
            }
            cx.notify(); return;
        }
        let changed = !state.group.ops().is_empty();
        let metadata_changed = self.tabs.get(self.active).is_some_and(|tab| {
            tab.buffer.bookmarks != state.metadata.bookmarks || tab.buffer.marks != state.metadata.marks
                || tab.begin_select_marker != state.metadata.marker
        });
        if !changed && !metadata_changed {
            if let Some(text) = state.clipboard {
                if let Some(history) = state.clipboard_history { drop_on_worker(crate::clipboard_history::record_copy_owned(history), cx); }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            let selection = state.group.target(false).2;
            if let Some(tab) = self.tabs.get_mut(self.active) { tab.selection = selection; }
            self.commit_caret(cx);
            if self.status_message.as_deref() == Some(status) { self.status_message = None; }
            self.resume_prepared_macro(playback_id, window, cx);
            drop_on_worker((operation, state.group, state.metadata), cx);
            cx.notify(); return;
        }
        let undo = matches!(operation, Operation::Replay { undo: true, .. });
        #[cfg(test)]
        if self.prepared_fail_reserve { drop_on_worker((operation, state), cx); self.set_status("Not enough memory to publish line indentation"); self.stop_prepared_macro("not enough memory to publish line edits"); cx.notify(); return; }
        let tab = self.active_tab_mut();
        let destination = if undo { &mut tab.redo_stack } else { &mut tab.undo_stack };
        if changed && destination.try_reserve(1).is_err() { drop_on_worker((operation, state), cx); self.set_status("Not enough memory to publish line edits"); self.stop_prepared_macro("not enough memory to publish line edits"); cx.notify(); return; }
        let mut old_redo = None;
        match &operation {
            Operation::Indent => {
                tab.undo_stack.push(UndoEntry::Prepared(state.group.clone()));
                old_redo = Some(std::mem::take(&mut tab.redo_stack));
            }
            Operation::Bookmarks { .. } | Operation::Sort(_) => {
                if changed {
                    tab.undo_stack.push(UndoEntry::Prepared(state.group.clone()));
                    old_redo = Some(std::mem::take(&mut tab.redo_stack));
                }
            }
            Operation::Merge { .. } => return,
            Operation::Replay { group, undo } => {
                let origin = if *undo { &mut tab.undo_stack } else { &mut tab.redo_stack };
                if !matches!(origin.last(), Some(UndoEntry::Prepared(top)) if Arc::ptr_eq(top, group)) {
                    drop_on_worker((operation, state), cx);
                    self.set_status("Indentation history changed; no changes applied"); cx.notify(); return;
                }
                let Some(entry) = origin.pop() else { return; };
                if *undo { tab.redo_stack.push(entry); } else { tab.undo_stack.push(entry); }
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
        if changed { self.after_edit_flush(cx); self.commit_caret(cx); }
        #[cfg(test)]
        let clipboard_start = std::time::Instant::now();
        #[cfg(test)]
        let has_clipboard = state.clipboard.is_some();
        if let Some(text) = state.clipboard {
            if let Some(history) = state.clipboard_history { drop_on_worker(crate::clipboard_history::record_copy_owned(history), cx); }
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        #[cfg(test)]
        let clipboard_time = if has_clipboard { clipboard_start.elapsed() } else { std::time::Duration::ZERO };
        // Find's existing refresh scans synchronously in this prerequisite.
        // Drop stale ranges without flattening the large new document here.
        if changed {
            let stale_find = (std::mem::take(&mut self.find_state.matches), self.find_state.rich_doc_block_offsets.take());
            self.find_state.current = None;
            if self.find_state.open && !self.find_state.query.is_empty() {
                self.find_state.status = Some("Document changed; run Find again to refresh matches".into());
            }
            drop_on_worker(stale_find, cx);
        }
        drop_on_worker(operation, cx);
        #[cfg(test)]
        self.prepared_timings.push(Timing { capture, worker: state.worker, publish: publish_start.elapsed(), clipboard: clipboard_time });
        if self.status_message.as_deref() == Some(status) { self.status_message = None; }
        self.resume_prepared_macro(playback_id, window, cx);
        cx.notify();
    }

    pub(crate) fn resume_prepared_macro(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = id else { return; };
        if !self.regex_replacement.macro_playback.as_ref().is_some_and(|playback| playback.id == id) { return; }
        let Some(mut playback) = self.regex_replacement.macro_playback.take() else { return; };
        playback.completed = playback.completed.saturating_add(1);
        self.merge_search_macro_undo(&playback);
        self.continue_search_macro(playback, window, cx);
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
