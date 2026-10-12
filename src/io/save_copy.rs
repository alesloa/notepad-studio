// PowerEditor/src/NppIO.cpp:1994-2094; NppCommands.cpp:453-454.
use std::{path::PathBuf, sync::{Arc, atomic::Ordering}};
use gpui::{AppContext, Context, Window};
use crate::{actions::FileSaveCopyAs, buffer::{ContentRevision, SavePoint}, rich_doc::DocFormat,
    tab::Tab, text::{encoding::Encoding, line_ending::LineEnding}, workspace::Workspace};

struct Target {
    id: uuid::Uuid, instance: Arc<()>, rope: ropey::Rope, raw: ContentRevision, activity: ContentRevision,
    path: Option<PathBuf>, encoding: Encoding, eol: LineEnding, dirty: bool, read_only: bool,
    saved: Option<SavePoint>, rich: Option<(ContentRevision, DocFormat, Arc<Vec<u8>>)>,
    cleanup: gpui::BackgroundExecutor,
}

impl Target {
    fn capture(tab: &Tab, cleanup: gpui::BackgroundExecutor) -> Self {
        let buffer = &tab.buffer;
        Self { id: tab.session_id, instance: tab.instance.clone(), rope: buffer.rope.clone(),
            raw: buffer.raw_revision.clone(), activity: tab.content_activity.clone(), path: buffer.file_path.clone(),
            encoding: buffer.encoding, eol: buffer.line_ending, dirty: buffer.dirty, read_only: buffer.read_only,
            saved: buffer.savepoint.clone(), rich: buffer.rich_doc.as_ref().map(|payload| {
                (payload.revision.clone(), payload.format, payload.original_bytes.clone())
            }), cleanup }
    }

    fn matches(&self, tab: &Tab) -> bool {
        let buffer = &tab.buffer;
        self.id == tab.session_id && Arc::ptr_eq(&self.instance, &tab.instance)
            && self.activity == tab.content_activity && self.raw == buffer.raw_revision
            && self.rope.is_instance(&buffer.rope) && self.path == buffer.file_path
            && self.encoding == buffer.encoding && self.eol == buffer.line_ending
            && self.dirty == buffer.dirty && self.read_only == buffer.read_only && self.saved == buffer.savepoint
            && buffer.loading.is_none() && tab.csv_draft.is_none() && tab.edit_transaction.is_none()
            && match (&self.rich, &buffer.rich_doc) {
                (None, None) => true,
                (Some((revision, format, original)), Some(payload)) => *revision == payload.revision
                    && *format == payload.format && Arc::ptr_eq(original, &payload.original_bytes)
                    && payload.edit_group_start.is_none(),
                _ => false,
            }
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        // After a tab closes, these can be the last references to a large document.
        let rope = std::mem::replace(&mut self.rope, ropey::Rope::new()); let rich = self.rich.take();
        self.cleanup.spawn(async move { drop((rope, rich)); }).detach();
    }
}

// Capture the test answer before the spawned task can poll on another thread.
#[cfg_attr(not(test), allow(clippy::manual_async_fn))]
fn choose_open_copy() -> impl std::future::Future<Output = Option<bool>> {
    #[cfg(test)]
    let answer = tests::open_copy_answer();
    async move {
        #[cfg(test)]
        if let Some(answer) = answer { return answer; }
        let answer = rfd::AsyncMessageDialog::new().set_title("Save a Copy As")
            .set_description("Open the copy in Notepad Studio after saving?\n\nYes opens the copy. No keeps the original document active. Cancel stops saving the copy.")
            .set_buttons(rfd::MessageButtons::YesNoCancel).show().await;
        match answer { rfd::MessageDialogResult::Yes => Some(true), rfd::MessageDialogResult::No => Some(false), _ => None }
    }
}

fn unfinished(w: &Workspace, index: usize) -> bool {
    w.tabs.get(index).is_none_or(|tab| tab.buffer.loading.is_some() || tab.csv_draft.is_some()
        || tab.edit_transaction.is_some() || tab.buffer.rich_doc.as_ref().is_some_and(|payload| payload.edit_group_start.is_some())
        || w.csv_models.get(&tab.session_id).is_some_and(|model| model.cell_input.is_some()
            || model.detached_draft.is_some() || model.editing_cell.is_some() || model.col_drag.is_some()
            || model.row_drag.is_some() || model.col_resize.is_some() || model.fill_drag.is_some()))
}

impl Workspace {
    pub(crate) fn stop_file_copy(&mut self) {
        if let Some(progress) = self.file_copy.take() { progress.cancel.store(true, Ordering::Relaxed); }
        self.file_copy_request = None;
    }

    pub(crate) fn cancel_file_copy(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file_copy.is_none() { return false; }
        self.stop_file_copy(); self.set_status("Save a Copy As cancelled"); cx.notify(); true
    }

    pub(crate) fn file_copy_chip(&self) -> Option<crate::ui::chrome::LoadingChip> {
        let progress = self.file_copy.as_ref()?;
        let written = progress.written.load(Ordering::Relaxed); let total = progress.total.load(Ordering::Relaxed);
        Some(crate::ui::chrome::LoadingChip { progress_text: if total == 0 { "Preparing copy…".into() }
            else { format!("Saving copy {} / {}", crate::io_stream::format_bytes(written), crate::io_stream::format_bytes(total)) },
            cancel: progress.cancel.clone() })
    }

    pub(crate) fn save_copy_as(&mut self, _: &FileSaveCopyAs, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        self.stop_file_copy();
        if self.buffer().file_path.is_none() && self.defer_automatic_title_active("save-copy", window, cx, |w, window, cx| w.save_copy_as(&FileSaveCopyAs, window, cx)) { return; }
        if unfinished(self, self.active) {
            self.set_status("Save a Copy As cancelled: finish the file load or current document edit first"); cx.notify(); return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else { return; };
        tab.observe_content_activity();
        let target = Target::capture(tab, cx.background_executor().clone());
        let active_dir = target.path.as_ref().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
        let starting = crate::settings::resolve_starting_dir(active_dir.as_deref()).or(active_dir)
            .or_else(dirs::document_dir).or_else(dirs::home_dir);
        let suggested = super::suggested_save_name(&tab.buffer);
        let open_copy = choose_open_copy();
        let selected = super::save_as_dialog::pick_with_title(starting, suggested, "Save a Copy As");
        let request = uuid::Uuid::new_v4();
        let window_id = self.session_window_id;
        let native_window = window.window_handle();
        self.stop_file_copy(); self.file_copy_request = Some(request);
        cx.spawn_in(window, async move |this, cx| {
            let answer = match open_copy.await {
                Some(open_copy) => selected.await.map(|path| (open_copy, path)),
                None => None,
            };
            let Some((open_copy, path)) = answer else {
                let _ = this.update(cx, |w, _| { if w.file_copy_request == Some(request) { w.file_copy_request = None; } });
                return;
            };
            let _ = this.update_in(cx, |w, window, cx| {
                if w.file_copy_request != Some(request) { return; }
                if w.window_closed || w.session_window_id != window_id || window.window_handle() != native_window { w.file_copy_request = None; return; }
                let index = w.tabs.iter().position(|tab| tab.session_id == target.id);
                let Some(index) = index.filter(|index| !unfinished(w, *index)
                    && w.tabs.get(*index).is_some_and(|tab| target.matches(tab))) else {
                    w.file_copy_request = None;
                    w.set_status("Save a Copy As cancelled: the document changed while the dialog was open"); cx.notify(); return;
                };
                if w.file_destination_open_elsewhere(&path, index, window.window_handle(), cx) {
                    w.file_copy_request = None;
                    w.set_status(crate::workspace::already_open_message()); cx.notify(); return;
                }
                w.write_save_copy(target, path, open_copy, request, window, cx);
            });
        }).detach();
    }

    fn write_save_copy(&mut self, target: Target, path: PathBuf, open_copy: bool,
        request: uuid::Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|tab| target.matches(tab)) else { return; };
        let executor = cx.background_executor().clone();
        let mut snapshot = super::BackgroundDrop::new(super::copy_snapshot::Snapshot::new(&tab.buffer), executor.clone());
        let backup = super::copy_stage::Backup::capture();
        let window_id = self.session_window_id; let native_window = window.window_handle();
        let progress = Arc::new(super::CopyProgress::default()); self.file_copy = Some(progress.clone());
        let worker_progress = progress.clone(); let destination = path.clone();
        #[cfg(test)]
        let (prepared_gate, committed_gate) = tests::take_worker_gates();
        #[cfg(test)]
        let result_gate = tests::take_result_gate();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let step = this.update_in(cx, |w, window, cx| {
                    if w.file_copy_request != Some(request) || progress.cancel.load(Ordering::Relaxed)
                        || w.window_closed || w.session_window_id != window_id || window.window_handle() != native_window {
                        if w.file_copy_request == Some(request) { w.stop_file_copy(); w.set_status("Save a Copy As cancelled"); cx.notify(); }
                        return None;
                    }
                    let Some(tab) = w.tabs.iter().find(|tab| target.matches(tab)) else {
                        w.stop_file_copy(); w.set_status("Save a Copy As cancelled: the document changed while capturing the copy"); cx.notify(); return None;
                    };
                    let snapshot = snapshot.as_mut()?;
                    match snapshot.step(&tab.buffer) {
                        Ok(done) => Some(done),
                        Err(error) => { w.stop_file_copy(); w.set_status(format!("Save a Copy As failed: {error}")); cx.notify(); None }
                    }
                }).ok().flatten();
                match step { Some(true) => break, Some(false) => {}, None => return }
                cx.background_executor().timer(std::time::Duration::from_millis(1)).await;
            }
            let Some(snapshot) = snapshot.take() else { return; };
            let cleanup_executor = executor.clone();
            let work = cx.background_spawn(async move {
                let result = snapshot.finish().and_then(|buffer| super::copy_stage::Stage::prepare(buffer, destination, &worker_progress));
                #[cfg(test)]
                if let Some(gate) = prepared_gate { let _ = gate.await; }
                super::BackgroundDrop::new(result, cleanup_executor)
            });
            let mut prepared = work.await;
            let authorized = this.update_in(cx, |w, window, cx| {
                if w.file_copy_request != Some(request) || w.file_copy.as_ref().is_none_or(|current| !Arc::ptr_eq(current, &progress)) { return None; }
                if w.window_closed || w.session_window_id != window_id || window.window_handle() != native_window { w.stop_file_copy(); return None; }
                let valid_index = w.tabs.iter().position(|tab| target.matches(tab));
                let index = match valid_index.filter(|index| !unfinished(w, *index)) {
                    Some(index) => index,
                    None => { w.stop_file_copy(); w.set_status("Save a Copy As cancelled: the document changed while preparing the copy"); cx.notify(); return None; }
                };
                if w.file_destination_open_elsewhere(&path, index, window.window_handle(), cx) {
                    w.stop_file_copy(); w.set_status(crate::workspace::already_open_message()); cx.notify(); return None;
                }
                if progress.cancel.load(Ordering::Relaxed) {
                    w.stop_file_copy(); w.set_status("Save a Copy As cancelled"); cx.notify(); return None;
                }
                if let Some(Err(error)) = prepared.as_ref() {
                    w.stop_file_copy(); w.set_status(format!("Save a Copy As failed: {error}")); cx.notify(); return None;
                }
                Some(())
            }).ok().flatten();
            if authorized.is_none() { return; }
            let Some(Ok(stage)) = prepared.take() else { return; };
            let stage = super::BackgroundDrop::new(stage, executor);
            let worker_progress = progress.clone();
            let commit = cx.background_spawn(async move {
                #[cfg(test)]
                if let Some(gate) = committed_gate { let _ = gate.await; }
                let result = stage.as_ref().ok_or_else(|| super::copy_stage::CommitError::Preparation("copy staging file is unavailable".into()))?
                    .commit(&backup, &worker_progress);
                #[cfg(test)]
                let critical_log = if let Some((gate, log)) = result_gate { let _ = gate.await; Some(log) } else { None };
                if let Err(error) = &result { if error.critical() {
                    eprintln!("[Save a Copy As] {error}");
                    #[cfg(test)]
                    if let Some(log) = critical_log { if let Err(error) = log.send(error.to_string()) { eprintln!("[test critical log] {error}"); } }
                } }
                result
            });
            let result = commit.await;
            let _ = this.update_in(cx, |w, window, cx| {
                let current = w.file_copy_request == Some(request) && w.file_copy.as_ref().is_some_and(|current| Arc::ptr_eq(current, &progress));
                if !current && result.as_ref().err().is_none_or(|error| !error.critical()) { return; }
                if current { w.file_copy = None; w.file_copy_request = None; }
                if w.window_closed || w.session_window_id != window_id || window.window_handle() != native_window { return; }
                if let Err(error) = result {
                    w.set_status(match error { super::copy_stage::CommitError::Cancelled => "Save a Copy As cancelled".into(),
                        error => format!("Save a Copy As failed: {error}") }); cx.notify(); return;
                }
                if w.tabs.iter().all(|tab| !target.matches(tab)) {
                    w.set_status("Copy saved; the original changed while saving, so the copy was not opened");
                    cx.notify(); return;
                }
                crate::settings::record_last_used_directory(&path);
                w.set_status(format!("Saved copy {}", path.display()));
                if open_copy { w.open_path(path, cx); }
                cx.notify();
            });
        }).detach();
        let progress = self.file_copy.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                let alive = this.update(cx, |w, cx| {
                    let alive = match (&w.file_copy, &progress) { (Some(current), Some(progress)) => Arc::ptr_eq(current, progress), _ => false };
                    if alive { cx.notify(); }
                    alive
                }).unwrap_or(false);
                if !alive { break; }
            }
        }).detach();
    }
}

#[cfg(test)]
#[path = "save_copy_tests.rs"]
pub(super) mod tests;
