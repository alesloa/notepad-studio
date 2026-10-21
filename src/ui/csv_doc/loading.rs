// Large CSV preparation owns a rope snapshot. Publication requires the same
// tab incarnation, revision and request; cancelled/stale buffers drop off-thread.
use crate::{buffer::ContentRevision, workspace::Workspace};
use gpui::{Context, Window};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

const FOREGROUND_BYTES: usize = 64 * 1024;
pub(super) struct Job {
    id: Uuid,
    revision: ContentRevision,
    delimiter: u8,
    has_header: bool,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(super) fn cancel(model: &mut super::CsvModel) {
    if model.parse_job.take().is_some() {
        model.source_state = None;
        model.reparse_requested = true;
        model.parse_error = None;
    }
}

pub(super) fn start_if_large(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(tab) = w.tabs.get(w.active) else {
        return false;
    };
    let id = tab.session_id;
    let revision = tab.buffer.raw_revision.clone();
    let loading = tab.buffer.loading.is_some();
    if w.csv_models.get(&id).is_some_and(|model| {
        !model.reparse_requested
            && model.parse_job.as_ref().is_some_and(|job| {
                job.revision == revision
                    && job.delimiter == model.delimiter
                    && job.has_header == model.has_header
            })
    }) {
        return true;
    }
    if !loading && tab.buffer.rope.len_bytes() <= FOREGROUND_BYTES {
        return false;
    }
    if !loading
        && w.csv_models.get(&id).is_some_and(|model| {
            !model.reparse_requested
                && model
                    .source_state
                    .as_ref()
                    .is_some_and(|source| source.revision == revision)
        })
    {
        return false;
    }
    if w.csv_models
        .get(&id)
        .is_some_and(|model| model.cell_input.is_some())
    {
        return false;
    }
    let rope = tab.buffer.rope.clone();
    let instance = tab.instance.clone();
    let default = super::default_delimiter_for_path(tab.buffer.file_path.as_deref());
    let recovered = tab.csv_draft.as_ref();
    let explicit_delimiter = recovered
        .and_then(|draft| draft.delimiter)
        .filter(|byte| byte.is_ascii() && !matches!(byte, b'\n' | b'\r' | b'"'));
    // Delimiter inference only needs a bounded initial sample; full decoding
    // and CSV parsing happen on the background executor.
    let sample: String = rope.chars().take(1024).collect();
    let delimiter = explicit_delimiter.unwrap_or_else(|| super::detect_delimiter(&sample, default));
    let has_header = recovered.is_none_or(|draft| draft.has_header);
    let handle = cx.focus_handle();
    let created = !w.csv_models.contains_key(&id);
    let model = w.csv_models.entry(id).or_insert_with(|| {
        let mut model = super::CsvModel::empty(delimiter);
        model.has_header = has_header;
        model.infer_delimiter = explicit_delimiter.is_none();
        model.grid_focus = Some(handle.clone());
        model
    });
    let delimiter = model.delimiter;
    let has_header = model.has_header;
    let infer_delimiter = model.infer_delimiter;
    cancel(model);
    if loading {
        model.install_parsed(Err("CSV source is still loading"), w.autosave_generation);
        model.bind_source(&rope, &revision);
        model.reparse_requested = true;
        if created {
            super::actions::register_grid_blur_listener(w, id, &handle, window, cx);
        }
        return true;
    }
    if rope.len_bytes() > super::limits::MAX_SOURCE_BYTES {
        model.install_parsed(Err(super::limits::TOO_LARGE), w.autosave_generation);
        model.bind_source(&rope, &revision);
        let draft = w
            .tabs
            .get_mut(w.active)
            .and_then(|tab| tab.csv_draft.take());
        if let Some(status) = super::actions::restore_draft(model, draft, false, window, cx) {
            w.set_status(status);
        }
        cx.notify();
        return true;
    }
    let request = Uuid::new_v4();
    let cancel = Arc::new(AtomicBool::new(false));
    model.parse_job = Some(Job {
        id: request,
        revision: revision.clone(),
        delimiter,
        has_header,
        cancel: cancel.clone(),
    });
    // This label reflects the active local worker, not an estimated wait.
    model.parse_error = Some("Preparing CSV grid…");
    model.bind_source(&rope, &revision);
    if created {
        super::actions::register_grid_blur_listener(w, id, &handle, window, cx);
    }
    let generation = w.autosave_generation;
    let executor = cx.background_executor().clone();
    let cleanup = executor.clone();
    #[cfg(test)]
    let gate = tests::take_gate();
    let worker = executor.spawn(async move {
        #[cfg(test)]
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        let result = (|| {
            let mut source = String::new();
            source
                .try_reserve_exact(rope.len_bytes())
                .map_err(|_| super::limits::TOO_LARGE)?;
            for chunk in rope.chunks() {
                if cancel.load(Ordering::Relaxed) {
                    return Err("CSV grid preparation cancelled");
                }
                source.push_str(chunk);
            }
            let delimiter = if infer_delimiter {
                super::detect_delimiter(&source, delimiter)
            } else {
                delimiter
            };
            super::limits::Parsed::new(&source, delimiter, has_header, &cancel)
                .map(|parsed| (parsed, delimiter))
        })();
        crate::io::BackgroundDrop::new(result, cleanup)
    });
    cx.spawn_in(window, async move |this, cx| {
        let mut result = worker.await;
        let _ = this.update_in(cx, |w, window, cx| {
            let Some(index) = w.tabs.iter().position(|tab| {
                tab.session_id == id
                    && Arc::ptr_eq(&tab.instance, &instance)
                    && tab.buffer.raw_revision == revision
            }) else {
                return;
            };
            let Some(model) = w.csv_models.get_mut(&id).filter(|model| {
                !model.reparse_requested
                    && model.delimiter == delimiter
                    && model.has_header == has_header
                    && model
                        .parse_job
                        .as_ref()
                        .is_some_and(|job| job.id == request)
            }) else {
                return;
            };
            let Some(parsed) = result.take() else {
                return;
            };
            let parsed = parsed.map(|(parsed, delimiter)| {
                model.delimiter = delimiter;
                parsed
            });
            model.infer_delimiter = false;
            model.install_parsed(parsed, generation);
            model.bind_source(&w.tabs[index].buffer.rope, &revision);
            let draft = w.tabs[index].csv_draft.take();
            let focused = w.active == index
                && window.is_window_active()
                && (w.focus_handle.is_focused(window)
                    || model
                        .grid_focus
                        .as_ref()
                        .is_some_and(|handle| handle.is_focused(window))
                    || window.focused(cx).is_none());
            if let Some(status) = super::actions::restore_draft(model, draft, focused, window, cx) {
                w.set_status(status);
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
    true
}

#[cfg(test)]
#[path = "loading_tests.rs"]
mod tests;
