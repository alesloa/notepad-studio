// NppCommands.cpp:3595-3617, 3674-3729; Scintilla Editor.cxx:4505-4525.
// Hash the active editor's physical bytes; focus in Find does not change source.
use std::{ops::Range, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};
use gpui::{App, Context, WeakEntity, Window, WindowHandle};
use uuid::Uuid;
use crate::{buffer::ContentRevision, clipboard_ownership, hash_tools::{self, Algorithm, EditorFormat},
    tab::TabViewMode, workspace::Workspace};
use super::jobs::{self, Budget};

pub(crate) struct Pending { id: Uuid, cancel: Arc<AtomicBool>, pub(crate) progress: Arc<AtomicU64> }
impl Drop for Pending { fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed); } }
struct Stamp {
    owner: WeakEntity<Workspace>,
    rope: jobs::Delivery<ropey::Rope>,
    tab: Uuid, instance: Arc<()>, revision: ContentRevision, activity: ContentRevision,
    selection: (usize, usize, bool), block: Option<(usize, usize, usize, usize, bool)>,
    format: EditorFormat, source: crate::workspace::source_input::SourceInputToken,
    context: gpui::InputContextToken,
    origin: Option<Origin>,
}
struct Origin {
    handle: WindowHandle<crate::ui::find_dialog::FindDialog>,
    dialog: WeakEntity<crate::ui::find_dialog::FindDialog>,
    workspace: WeakEntity<Workspace>, context: gpui::InputContextToken,
}
fn block_key(w: &Workspace) -> Option<(usize, usize, usize, usize, bool)> {
    w.tabs.get(w.active)?.block_selection.map(|b| (b.anchor_line, b.head_line, b.anchor_col, b.head_col, b.is_dragging))
}
impl Stamp {
    fn valid(&self, w: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> bool {
        let origin_valid = self.origin.as_ref().map(|origin| {
            origin.handle.update(cx, |dialog, window, cx| window.is_window_active() && window.input_context_token() == origin.context
                && dialog.workspace == origin.workspace && cx.entity() == origin.dialog).unwrap_or(false)
        }).unwrap_or_else(|| window.is_window_active());
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        !w.window_closed && origin_valid && window.root::<Workspace>().flatten().is_some_and(|owner| owner == self.owner)
            && tab.view_mode == TabViewMode::Edit
            && tab.buffer.loading.is_none() && tab.edit_transaction.is_none() && !w.is_composing_text(window)
            && tab.session_id == self.tab && Arc::ptr_eq(&tab.instance, &self.instance)
            && tab.buffer.raw_revision == self.revision && tab.content_activity == self.activity
            && self.rope.as_ref().is_some_and(|rope| tab.buffer.rope.is_instance(rope))
            && (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging) == self.selection
            && block_key(w) == self.block && tab.buffer.encoding == self.format.encoding
            && tab.buffer.line_ending == self.format.ending && w.source_input_token() == self.source
            && window.input_context_token() == self.context
    }
}
fn range(w: &Workspace) -> Result<Option<Range<usize>>, &'static str> {
    let Some(tab) = w.tabs.get(w.active) else { return Ok(None); };
    if let Some(block) = tab.block_selection {
        if block.anchor_line != block.head_line { return Ok(None); }
        let row = crate::workspace::column_plan::row(&tab.buffer.rope, block.head_line)?;
        let start = row.byte(block.anchor_col.min(block.head_col))?;
        let end = row.byte(block.anchor_col.max(block.head_col))?;
        return Ok((start < end).then_some(start..end));
    }
    if tab.selection.anchor > tab.buffer.rope.len_bytes() || tab.selection.head > tab.buffer.rope.len_bytes() {
        return Err("Hash selection is outside the current document; select the text again");
    }
    let range = crate::edit_ops::util::clamp_byte_range(&tab.buffer.rope, tab.selection.ordered());
    Ok((range.start < range.end).then_some(range))
}
impl Workspace {
    pub(crate) fn cancel_hash_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if self.pending_hash_selection.take().is_none() { return false; }
        self.set_status("Hash cancelled; clipboard unchanged"); cx.notify(); true
    }
    pub(crate) fn hash_selection(&mut self, algorithm: Algorithm, window: &mut Window, cx: &mut Context<Self>) {
        self.hash_selection_from(algorithm, None, window, cx);
    }
    fn hash_selection_from(&mut self, algorithm: Algorithm, origin: Option<Origin>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        if self.window_closed || (origin.is_none() && !window.is_window_active()) || tab.view_mode != TabViewMode::Edit
            || tab.buffer.loading.is_some() || tab.edit_transaction.is_some() || self.is_composing_text(window) {
            self.set_status("Switch to a loaded Source or Raw View and finish composition before hashing a selection"); cx.notify(); return;
        }
        let selected = match range(self) {
            Ok(Some(range)) => range, Ok(None) => return,
            Err(message) => { self.set_status(message); cx.notify(); return; }
        };
        self.sync_source_input_caret();
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let stamp = Stamp { owner: cx.entity().downgrade(), rope: jobs::Delivery::new(tab.buffer.rope.clone(), cx.background_executor().clone()),
            tab: tab.session_id, instance: tab.instance.clone(), revision: tab.buffer.raw_revision.clone(),
            activity: tab.content_activity.clone(), selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            block: block_key(self), format: EditorFormat { encoding: tab.buffer.encoding, ending: tab.buffer.line_ending },
            source: self.source_input_token(), context: window.input_context_token(), origin };
        let rope = tab.buffer.rope.clone(); let format = stamp.format;
        let budget = cx.default_global::<Budget>().clone();
        let permit = match budget.reserve(2 * hash_tools::CHUNK + 4096) {
            Ok(permit) => permit, Err(message) => { self.set_status(message); cx.notify(); return; }
        };
        self.cancel_bookmark_copy(cx);
        self.pending_hash_selection = None;
        let cancel = Arc::new(AtomicBool::new(false)); let progress = Arc::new(AtomicU64::new(0));
        let request = clipboard_ownership::claim(cancel.clone()); let owner = request.clone();
        let worker_cancel = cancel.clone(); let worker_progress = progress.clone(); let id = Uuid::new_v4();
        let weak = cx.entity().downgrade(); let handle = window.window_handle(); let executor = cx.background_executor().clone();
        let result = jobs::run(cx, cancel.clone(), move || {
            let _permit = permit;
            let text = hash_tools::selection_digest(algorithm, &rope, selected, format, &worker_cancel, &worker_progress)?;
            text.map(|text| crate::clipboard_history::prepare_copy(&text, &worker_cancel)
                .map(|history| (text, history)).map_err(|_| hash_tools::HashError::Memory)).transpose()
        }, move |result, cx| {
            let mut delivery = jobs::Delivery::new(result, executor.clone());
            let updated = handle.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |w, cx| {
                    if !w.pending_hash_selection.as_ref().is_some_and(|pending| pending.id == id) { return; }
                    w.pending_hash_selection = None;
                    if !stamp.valid(w, window, cx) || !clipboard_ownership::is_current(&owner) {
                        w.set_status("Hash cancelled because the editor or clipboard request changed; clipboard unchanged"); cx.notify(); return;
                    }
                    match delivery.take() {
                        Some(Ok(Some((text, history)))) => {
                            jobs::dispose(crate::clipboard_history::record_copy_owned(history), &executor);
                            clipboard_ownership::publish(&owner, cx, gpui::ClipboardItem::new_string(text));
                            w.set_status(format!("Copied {} selection hash", algorithm.label()));
                        }
                        Some(Err(error)) => w.set_status(error.to_string()),
                        _ => {},
                    }
                    cx.notify();
                });
            });
            if updated.is_err() {
                let _ = weak.update(cx, |w, cx| { if w.pending_hash_selection.as_ref().is_some_and(|pending| pending.id == id) { w.cancel_hash_selection(cx); } });
            }
        });
        if let Err(message) = result { self.set_status(message); cx.notify(); return; }
        self.pending_hash_selection = Some(Pending { id, cancel, progress });
        self.set_status(format!("Hashing {} selection; Escape cancels", algorithm.label())); cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                let active = this.update(cx, |w, cx| {
                    if let Some(pending) = w.pending_hash_selection.as_ref().filter(|pending| pending.id == id) {
                        let bytes = pending.progress.load(Ordering::Relaxed);
                        w.set_status(format!("Hashing {} selection: {bytes} bytes; Escape cancels", algorithm.label())); cx.notify(); true
                    } else { false }
                }).unwrap_or(false);
                if !active { break; }
            }
        }).detach();
    }
}

pub(crate) fn route(algorithm: Algorithm, cx: &mut App) {
    let Some(origin) = cx.active_window() else { return; };
    cx.defer(move |cx| {
        if cx.active_window() != Some(origin) { return; }
        if let Some(handle) = origin.downcast::<Workspace>() {
            let _ = handle.update(cx, |w, window, cx| w.hash_selection(algorithm, window, cx)); return;
        }
        let Some(find) = origin.downcast::<crate::ui::find_dialog::FindDialog>() else { return; };
        let Some(workspace) = find.read(cx).ok().and_then(|dialog| dialog.workspace.upgrade()) else { return; };
        let Some(context) = origin.update(cx, |_, window, _| window.is_window_active().then(|| window.input_context_token())).ok().flatten() else { return; };
        let source = cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>())
            .find(|handle| handle.entity(cx).ok().is_some_and(|entity| entity == workspace));
        let Some(dialog) = find.entity(cx).ok().map(|dialog| dialog.downgrade()) else { return; };
        let origin = Origin { handle: find, dialog, workspace: workspace.downgrade(), context };
        if let Some(source) = source { let _ = source.update(cx, |w, window, cx| w.hash_selection_from(algorithm, Some(origin), window, cx)); }
    });
}
