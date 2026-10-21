// Existing File > Save As .docx conversion, with the same ownership and
// destination rules as Save a Copy As. Serialization/staging run off-thread.
use super::Workspace;
use crate::{
    actions::rich_doc::RichDocSaveAsDocx,
    buffer::ContentRevision,
    io::{BackgroundDrop, CopyBackup, CopyProgress, CopyStage, ExportSnapshot},
    rich_doc::DocFormat,
    tab::Tab,
    text::{encoding::Encoding, line_ending::LineEnding},
};
use gpui::{AppContext, Context, Window};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};
use uuid::Uuid;

struct Target {
    id: Uuid,
    instance: Arc<()>,
    revision: ContentRevision,
    activity: ContentRevision,
    rich: ContentRevision,
    path: Option<PathBuf>,
    encoding: Encoding,
    eol: LineEnding,
}
impl Target {
    fn capture(tab: &Tab) -> Option<Self> {
        let payload = tab
            .buffer
            .rich_doc
            .as_ref()
            .filter(|payload| payload.format == DocFormat::Doc)?;
        Some(Self {
            id: tab.session_id,
            instance: tab.instance.clone(),
            revision: tab.buffer.raw_revision.clone(),
            activity: tab.content_activity.clone(),
            rich: payload.revision.clone(),
            path: tab.buffer.file_path.clone(),
            encoding: tab.buffer.encoding,
            eol: tab.buffer.line_ending,
        })
    }
    fn matches(&self, tab: &Tab) -> bool {
        self.id == tab.session_id
            && Arc::ptr_eq(&self.instance, &tab.instance)
            && self.revision == tab.buffer.raw_revision
            && self.activity == tab.content_activity
            && self.path == tab.buffer.file_path
            && self.encoding == tab.buffer.encoding
            && self.eol == tab.buffer.line_ending
            && tab.buffer.loading.is_none()
            && tab.edit_transaction.is_none()
            && tab.buffer.rich_doc.as_ref().is_some_and(|payload| {
                payload.format == DocFormat::Doc
                    && payload.revision == self.rich
                    && payload.edit_group_start.is_none()
            })
    }
}
struct CancelOnDrop(Arc<CopyProgress>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel.store(true, Ordering::Relaxed);
    }
}

fn same_file(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
        || super::file_routes::file_identity(left)
            .zip(super::file_routes::file_identity(right))
            .is_some_and(|(left, right)| left == right)
}

impl Workspace {
    fn export_destination(
        &mut self,
        owner: &Target,
        request: Uuid,
        destination: &Path,
        window: &Window,
        native: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if self.window_closed
            || window.window_handle() != native
            || self.file_copy_request != Some(request)
        {
            return None;
        }
        if self
            .file_copy
            .as_ref()
            .is_some_and(|progress| progress.cancel.load(Ordering::Relaxed))
        {
            self.stop_file_copy();
            self.set_status("Save As .docx cancelled");
            cx.notify();
            return None;
        }
        let Some(index) = self.tabs.iter().position(|tab| owner.matches(tab)) else {
            self.stop_file_copy();
            self.set_status("Save As .docx cancelled: the document changed");
            cx.notify();
            return None;
        };
        if owner
            .path
            .as_ref()
            .is_some_and(|source| same_file(source, destination))
            || self.file_destination_open_elsewhere(destination, index, native, cx)
        {
            self.stop_file_copy();
            self.set_status(super::already_open_message());
            cx.notify();
            return None;
        }
        Some(index)
    }

    pub(crate) fn rich_doc_save_as_docx(
        &mut self,
        _: &RichDocSaveAsDocx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.window_closed {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        tab.observe_content_activity();
        let Some(owner) = Target::capture(tab) else {
            return;
        };
        let starting = owner
            .path
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf);
        let suggested = owner
            .path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| format!("{}.docx", stem.to_string_lossy()))
            .unwrap_or_else(|| "Untitled.docx".into());
        let picker = crate::io::pick_docx_path(starting, suggested);
        let request = Uuid::new_v4();
        let native = window.window_handle();
        self.stop_file_copy();
        self.file_copy_request = Some(request);
        #[cfg(test)]
        let gate = tests::take_gate();
        cx.spawn_in(window, async move |this, cx| {
            let Some(destination) = picker.await else {
                let _ = this.update(cx, |w, _| {
                    if w.file_copy_request == Some(request) {
                        w.stop_file_copy();
                    }
                });
                return;
            };
            let progress = Arc::new(CopyProgress::default());
            let _cancel = CancelOnDrop(progress.clone());
            let executor = cx.background_executor().clone();
            let admitted = this
                .update_in(cx, |w, window, cx| {
                    let index =
                        w.export_destination(&owner, request, &destination, window, native, cx)?;
                    w.file_copy = Some(progress.clone());
                    cx.notify();
                    Some((
                        ExportSnapshot::for_export(&w.tabs[index].buffer),
                        CopyBackup::capture(),
                    ))
                })
                .ok()
                .flatten();
            let Some((snapshot, backup)) = admitted else {
                return;
            };
            let mut snapshot = BackgroundDrop::new(snapshot, executor.clone());
            loop {
                let step = this
                    .update_in(cx, |w, window, cx| {
                        let index = w.export_destination(
                            &owner,
                            request,
                            &destination,
                            window,
                            native,
                            cx,
                        )?;
                        match snapshot.as_mut()?.step(&w.tabs[index].buffer) {
                            Ok(done) => Some(done),
                            Err(error) => {
                                w.stop_file_copy();
                                w.set_status(format!("Save As .docx failed: {error}"));
                                cx.notify();
                                None
                            }
                        }
                    })
                    .ok()
                    .flatten();
                match step {
                    Some(true) => break,
                    Some(false) => {}
                    None => return,
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(1))
                    .await;
            }
            let Some(snapshot) = snapshot.take() else {
                return;
            };
            let selected = destination.clone();
            let work_progress = progress.clone();
            let cleanup = executor.clone();
            let mut prepared = cx
                .background_spawn(async move {
                    let result = (|| {
                        let buffer = snapshot.finish()?;
                        let payload = buffer
                            .rich_doc
                            .as_ref()
                            .ok_or("native document is unavailable")?;
                        let raw = buffer.dirty.then(|| buffer.rope.to_string());
                        crate::rich_doc::validate_raw_view(payload, raw.as_deref())
                            .map_err(|error| error.to_string())?;
                        let bytes = crate::rich_doc::doc::docx_bytes(payload)
                            .map_err(|error| error.to_string())?;
                        CopyStage::prepare_bytes(&bytes, selected, &work_progress)
                    })();
                    #[cfg(test)]
                    if let Some(gate) = gate {
                        let _ = gate.await;
                    }
                    BackgroundDrop::new(result, cleanup)
                })
                .await;
            let authorized = this
                .update_in(cx, |w, window, cx| {
                    w.export_destination(&owner, request, &destination, window, native, cx)?;
                    if progress.cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    if let Some(Err(error)) = prepared.as_ref() {
                        w.stop_file_copy();
                        w.set_status(format!("Save As .docx failed: {error}"));
                        cx.notify();
                        return None;
                    }
                    Some(())
                })
                .ok()
                .flatten();
            if authorized.is_none() {
                return;
            }
            let Some(Ok(stage)) = prepared.take() else {
                return;
            };
            let stage = BackgroundDrop::new(stage, executor);
            let work_progress = progress.clone();
            let result = cx
                .background_spawn(async move {
                    stage
                        .as_ref()
                        .ok_or_else(|| ("export staging file is unavailable".to_owned(), false))?
                        .commit(&backup, &work_progress)
                        .map_err(|error| {
                            let critical = error.critical();
                            if critical {
                                eprintln!("[Save As .docx] {error}");
                            }
                            (error.to_string(), critical)
                        })
                })
                .await;
            let _ = this.update(cx, |w, cx| {
                let current = w.file_copy_request == Some(request);
                if !current && result.as_ref().err().is_none_or(|(_, critical)| !critical) {
                    return;
                }
                if current {
                    w.stop_file_copy();
                }
                match result {
                    Ok(()) => {
                        crate::settings::record_last_used_directory(&destination);
                        let name = destination
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        w.set_status(format!("Saved {name}"));
                        if !w.window_closed {
                            w.open_path(destination, cx);
                        }
                    }
                    Err((error, _)) => w.set_status(format!("Save As .docx failed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
#[path = "rich_export_tests.rs"]
mod tests;
