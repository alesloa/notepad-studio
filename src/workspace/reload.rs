// PowerEditor/src/NppIO.cpp:588 — Reload confirms discarding dirty edits.
// Original background loading and atomic publication, shared by File and tab menus.
use std::{sync::{Arc, atomic::Ordering}, time::Duration};
use gpui::{AppContext, BackgroundExecutor, Context, FocusHandle, Window};
use uuid::Uuid;
use super::{Workspace, reload_guard::Guard};
use crate::{io_reload::{Error, Progress, Request}, buffer::Buffer, tab::Tab};

struct Pending {
    id: Uuid, tab: Uuid, guard: Guard, request: Request,
    csv_focus: Option<FocusHandle>, csv: Option<super::reload_csv::Guard>, status: String,
}
struct Running { id: Uuid, progress: Arc<Progress> }
#[derive(Default)]
pub(super) struct Runtime {
    pending: Option<Pending>, running: Option<Running>, executor: Option<BackgroundExecutor>,
    #[cfg(test)] hold: bool,
    #[cfg(test)] held: Option<(Uuid, Result<Buffer, Error>)>,
    #[cfg(test)] huge_prompt_sink: Option<std::sync::mpsc::Sender<u64>>,
}
impl Runtime {
    pub(super) fn close(&mut self) {
        if let Some(running) = &self.running { running.progress.stop(); }
        if let Some(pending) = &self.pending { pending.request.progress.stop(); }
        if let Some(executor) = &self.executor { dispose(executor, self.pending.take()); }
        #[cfg(test)] if let Some(executor) = &self.executor { dispose(executor, self.held.take()); }
    }
}
impl Drop for Runtime { fn drop(&mut self) { self.close(); } }
fn dispose<T: Send + 'static>(executor: &BackgroundExecutor, value: T) { executor.spawn(async move { drop(value); }).detach(); }
fn dispose_csv(executor: &BackgroundExecutor, model: crate::ui::csv_doc::CsvModel) {
    // ScrollHandle is UI-only; detach all file-sized owned data before its final drop.
    dispose(executor, (model.headers, model.rows, model.col_widths, model.col_order, model.row_order,
        model.source_state, model.pending_source, model.selected_rows, model.selected_cols,
        model.undo_stack, model.redo_stack, model.find_matches, model.col_filters,
        model.find_query, model.replace_text, model.replace_summary, model.fill_drag, model.open_filter_dropdown,
        model.find_input.map(|input| input.buffer), model.replace_input.map(|input| input.buffer)));
}
fn draft(w: &Workspace, id: Uuid) -> bool {
    w.tabs.iter().find(|tab| tab.session_id == id).is_none_or(|tab| tab.csv_draft.is_some())
        || w.csv_models.get(&id).is_some_and(|model| model.cell_input.is_some() || model.detached_draft.is_some()
            || model.editing_cell.is_some() || model.col_drag.is_some() || model.row_drag.is_some()
            || model.col_resize.is_some() || model.fill_drag.is_some() || model.open_filter_dropdown.is_some())
}
impl Workspace {
    pub(super) fn retire_closed_reloads(&mut self) {
        if let Some(pending) = &self.reload.pending {
            if self.tabs.iter().all(|tab| tab.session_id != pending.tab) { pending.request.progress.stop(); }
        }
    }
    pub(super) fn begin_reload(&mut self, id: Uuid, confirmed: bool, csv_focus: Option<FocusHandle>, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        let Some(tab) = self.tabs.iter().find(|tab| tab.session_id == id) else { return; };
        if tab.buffer.dirty && !confirmed { return; }
        if tab.buffer.loading.is_some() || draft(self, id) { self.set_status("Reload cancelled: finish the current file load or CSV draft first"); cx.notify(); return; }
        let Some(path) = tab.buffer.file_path.clone() else { return; };
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.session_id == id) else { return; };
        tab.observe_content_activity();
        let guard = match Guard::capture(tab) { Ok(guard) => guard, Err(error) => { self.set_status(error); cx.notify(); return; } };
        let csv = match self.csv_models.get(&id).map(|model| { let (nodes,bytes) = guard.remaining(); super::reload_csv::Guard::capture(model,nodes,bytes) }).transpose() {
            Ok(csv) => csv, Err(error) => { self.set_status(error); cx.notify(); return; }
        };
        let executor = cx.background_executor().clone();
        self.reload.executor = Some(executor.clone());
        if let Some(running) = &self.reload.running { running.progress.stop(); }
        if let Some(pending) = &self.reload.pending { pending.request.progress.stop(); }
        dispose(&executor, self.reload.pending.take());
        let request = Request { path, settings: crate::settings::snapshot(), progress: Arc::new(Progress::default()), allow_huge: false };
        let status = "Reloading document…".to_string(); self.set_status(status.clone());
        self.reload.pending = Some(Pending { id: Uuid::new_v4(), tab: id, guard, request, csv_focus, csv, status });
        self.start_reload_worker(window, cx); cx.notify();
    }
    pub(super) fn reload_chip(&self) -> Option<crate::ui::chrome::LoadingChip> {
        let pending = self.reload.pending.as_ref()?;
        if self.tabs.get(self.active)?.session_id != pending.tab { return None; }
        let progress = &pending.request.progress;
        let bytes = progress.bytes.load(Ordering::Relaxed); let total = progress.total.load(Ordering::Relaxed);
        let text = if progress.cancel.load(Ordering::Relaxed) { "Cancelling reload…".to_string() }
            else if self.reload.running.as_ref().is_none_or(|running| running.id != pending.id) { "Waiting to reload…".to_string() }
            else { format!("Reloading {} / {}", crate::io_stream::format_bytes(bytes), crate::io_stream::format_bytes(total)) };
        Some(crate::ui::chrome::LoadingChip { progress_text: text, cancel: progress.cancel.clone() })
    }
    fn start_reload_worker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed || self.reload.running.is_some() { return; }
        let Some(pending) = &self.reload.pending else { return; };
        if !self.reload_pending_valid(pending) {
            let id = pending.id;
            self.finish_reload_without_worker(id, Err(Error::Cancelled), window, cx); return;
        }
        let id = pending.id; let progress = pending.request.progress.clone();
        let request = Request { path: pending.request.path.clone(), settings: pending.request.settings.clone(),
            progress: progress.clone(), allow_huge: pending.request.allow_huge };
        self.reload.running = Some(Running { id, progress: progress.clone() });
        let task = cx.background_spawn(async move {
            #[cfg(test)] let started = std::time::Instant::now();
            #[cfg(test)] let progress = request.progress.clone();
            let result = crate::io_reload::read(request);
            #[cfg(test)] progress.elapsed_nanos.store(started.elapsed().as_nanos().min(u64::MAX as u128) as u64, Ordering::Relaxed);
            result
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = Arc::new(std::sync::Mutex::new(Some(task.await)));
            let executor = cx.background_executor().clone();
            let reply = result.clone();
            let _ = this.update_in(cx, |w, window, cx| {
                let Some(result) = reply.lock().ok().and_then(|mut value| value.take()) else { return; };
                #[cfg(test)] if w.reload.hold && !w.window_closed { w.reload.held = Some((id, result)); return; }
                w.finish_reload_worker(id, result, window, cx);
            });
            dispose(&executor, result);
        }).detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let alive = this.update_in(cx, |w, _, cx| {
                    if w.window_closed { w.reload.close(); return false; }
                    if w.reload.running.as_ref().is_none_or(|running| running.id != id) { return false; }
                    if progress.cancel.load(Ordering::Relaxed) { progress.stop(); }
                    cx.notify(); true
                }).unwrap_or(false);
                if !alive { break; }
            }
        }).detach();
    }
    fn reload_pending_valid(&self, pending: &Pending) -> bool {
        let current = crate::settings::snapshot();
        let captured = &pending.request.settings;
        !self.window_closed && !pending.request.progress.cancel.load(Ordering::Relaxed)
            && current.large_file_restriction_enabled == captured.large_file_restriction_enabled
            && current.large_file_disable_syntax == captured.large_file_disable_syntax
            && current.large_file_threshold_bytes == captured.large_file_threshold_bytes
            && !draft(self, pending.tab)
            && match (&pending.csv, self.csv_models.get(&pending.tab)) { (None,None) => true, (Some(saved),Some(model)) => saved.valid(model), _ => false }
            && self.tabs.iter().find(|tab| tab.session_id == pending.tab).is_some_and(|tab| pending.guard.valid(tab))
    }
    fn finish_reload_worker(&mut self, id: Uuid, result: Result<Buffer, Error>, window: &mut Window, cx: &mut Context<Self>) {
        if self.reload.running.as_ref().is_none_or(|running| running.id != id) {
            dispose(cx.background_executor(), result); return;
        }
        self.reload.running = None;
        if self.reload.pending.as_ref().is_none_or(|pending| pending.id != id) {
            dispose(cx.background_executor(), result); self.start_reload_worker(window, cx); return;
        }
        if self.window_closed { self.reload.close(); dispose(cx.background_executor(), result); return; }
        if matches!(result, Err(Error::Huge(_))) {
            if !self.reload.pending.as_ref().is_some_and(|pending| self.reload_pending_valid(pending)) {
                self.finish_reload_without_worker(id, result, window, cx); return;
            }
            let size = match result { Err(Error::Huge(size)) => size, _ => return };
            #[cfg(test)] if let Some(sink) = &self.reload.huge_prompt_sink { let _ = sink.send(size); return; }
            cx.spawn_in(window, async move |this, cx| {
                let answer = rfd::AsyncMessageDialog::new().set_title(crate::consts::APP_NAME)
                    .set_level(rfd::MessageLevel::Warning).set_description(format!("Reload a {} document? This may use substantial memory.", crate::io_stream::format_bytes(size)))
                    .set_buttons(rfd::MessageButtons::YesNo).show().await;
                let _ = this.update_in(cx, |w, window, cx| {
                    if w.reload.pending.as_ref().is_none_or(|pending| pending.id != id) { return; }
                    if matches!(answer, rfd::MessageDialogResult::Yes) {
                        if let Some(pending) = w.reload.pending.as_mut() { pending.request.allow_huge = true; }
                        w.start_reload_worker(window, cx);
                    } else { w.finish_reload_without_worker(id, Err(Error::Cancelled), window, cx); }
                });
            }).detach(); return;
        }
        self.finish_reload_without_worker(id, result, window, cx);
    }
    fn finish_reload_without_worker(&mut self, id: Uuid, result: Result<Buffer, Error>, window: &mut Window, cx: &mut Context<Self>) {
        if self.reload.pending.as_ref().is_none_or(|pending| pending.id != id) { dispose(cx.background_executor(), result); return; }
        let valid = self.reload.pending.as_ref().is_some_and(|pending| self.reload_pending_valid(pending));
        let Some(pending) = self.reload.pending.take() else { dispose(cx.background_executor(), result); return; };
        let owned_status = self.status_message.as_deref() == Some(pending.status.as_str());
        let index = self.tabs.iter().position(|tab| tab.session_id == pending.tab);
        if !valid {
            if !self.window_closed && owned_status { self.set_status("Reload cancelled: document changed or reload was cancelled; original contents kept"); }
            dispose(cx.background_executor(), (pending, result)); cx.notify(); return;
        }
        match result {
            Ok(mut buffer) => {
                let Some(index) = index else { dispose(cx.background_executor(), (pending, buffer)); return; };
                let Some(old) = self.tabs.get(index) else { dispose(cx.background_executor(), (pending, buffer)); return; };
                buffer.read_only |= old.buffer.read_only; buffer.tab_color = old.buffer.tab_color; buffer.is_changelog = old.buffer.is_changelog;
                let mut replacement = Tab::empty(); replacement.session_id = old.session_id; replacement.view_mode = old.view_mode; replacement.buffer = buffer;
                let Some(target) = self.tabs.get_mut(index) else { dispose(cx.background_executor(), (pending, replacement)); return; };
                let old = std::mem::replace(target, replacement);
                self.retire_syntax_tab(pending.tab, cx);
                let model = self.csv_models.remove(&pending.tab);
                dispose(cx.background_executor(), old);
                if let Some(model) = model { dispose_csv(cx.background_executor(), model); }
                if index == self.active { self.cancel_regex_replacement(); }
                if index == self.active { self.retire_source_input(); self.last_lines.clear(); self.refresh_matches(); self.refresh_smart_highlight(); self.refresh_tag_match(); self.caret_visible = true; }
                if owned_status { self.set_status(if self.tabs.get(index).is_some_and(|tab| tab.buffer.read_only) { "Reloaded document read-only" } else { "Reloaded document" }); }
                if let Some(handle) = pending.csv_focus.clone() { self.restore_reloaded_csv_focus(pending.tab, handle, window, cx); }
                self.save_session(cx);
            }
            Err(Error::Cancelled) => { if owned_status { self.set_status("Reload cancelled; original contents kept"); } }
            Err(Error::Failed(error)) => { if owned_status { self.set_status(format!("Reload failed: {error}")); } }
            Err(Error::Huge(_)) => {}
        }
        dispose(cx.background_executor(), pending); cx.notify();
    }
}
#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;
