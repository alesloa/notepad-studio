// PowerEditor/src/NppIO.cpp:2346-2387 — confirm before recycling and closing.
// PowerEditor/src/Notepad_plus.cpp:2619-2628 — modal OK/Cancel and the full path.
// PowerEditor/src/ScintillaComponent/Buffer.cpp:1174-1197 — recyclable deletion.
use std::{path::{Path, PathBuf}, sync::Arc};
use gpui::{AnyWindowHandle, Context, Window};
use uuid::Uuid;
use crate::{buffer::{ContentRevision, SavePoint}, rich_doc::DocFormat, tab::Tab,
    text::{encoding::Encoding, line_ending::LineEnding}};
use super::Workspace;

#[derive(Default)]
pub(super) struct Runtime { pending: Option<Uuid> }

// Rope clones share storage. The activity token also detects Edit -> Undo ABA.
struct Target {
    tab: Uuid, window: Uuid, native_window: AnyWindowHandle, instance: Arc<()>,
    rope: ropey::Rope, raw: ContentRevision, activity: ContentRevision,
    path: PathBuf, encoding: Encoding, eol: LineEnding, dirty: bool, read_only: bool,
    saved: Option<SavePoint>, rich: Option<RichTarget>,
}
struct RichTarget { revision: ContentRevision, format: DocFormat, original: Arc<Vec<u8>> }

impl Target {
    fn capture(tab: &Tab, path: PathBuf, window: Uuid, native_window: AnyWindowHandle) -> Self {
        Self { tab: tab.session_id, window, native_window, instance: tab.instance.clone(),
            rope: tab.buffer.rope.clone(), raw: tab.buffer.raw_revision.clone(), activity: tab.content_activity.clone(),
            path, encoding: tab.buffer.encoding, eol: tab.buffer.line_ending, dirty: tab.buffer.dirty,
            read_only: tab.buffer.read_only, saved: tab.buffer.savepoint.clone(),
            rich: tab.buffer.rich_doc.as_ref().map(|payload| RichTarget {
                revision: payload.revision.clone(), format: payload.format, original: payload.original_bytes.clone(),
            }) }
    }

    fn matches(&self, tab: &Tab) -> bool {
        let buffer = &tab.buffer;
        Arc::ptr_eq(&self.instance, &tab.instance) && self.activity == tab.content_activity
            && self.raw == buffer.raw_revision && buffer.rope.is_instance(&self.rope)
            && buffer.file_path.as_ref() == Some(&self.path) && self.encoding == buffer.encoding
            && self.eol == buffer.line_ending && self.dirty == buffer.dirty && self.read_only == buffer.read_only
            && self.saved == buffer.savepoint && buffer.loading.is_none() && tab.csv_draft.is_none()
            && tab.edit_transaction.is_none() && match (&self.rich, &buffer.rich_doc) {
                (None, None) => true,
                (Some(saved), Some(payload)) => saved.revision == payload.revision && saved.format == payload.format
                    && Arc::ptr_eq(&saved.original, &payload.original_bytes) && payload.edit_group_start.is_none(),
                _ => false,
            }
    }
}

fn unfinished(workspace: &Workspace, id: Uuid) -> bool {
    workspace.tabs.iter().find(|tab| tab.session_id == id).is_none_or(|tab| {
        tab.buffer.loading.is_some() || tab.csv_draft.is_some() || tab.edit_transaction.is_some()
            || tab.buffer.rich_doc.as_ref().is_some_and(|payload| payload.edit_group_start.is_some())
    }) || workspace.csv_models.get(&id).is_some_and(|model| {
        model.cell_input.is_some() || model.detached_draft.is_some() || model.editing_cell.is_some()
            || model.col_drag.is_some() || model.row_drag.is_some() || model.col_resize.is_some()
            || model.fill_drag.is_some() || model.open_filter_dropdown.is_some()
    })
}

impl Workspace {
    pub(super) fn confirm_tab_trash(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        let Some(tab) = self.tabs.get(idx) else { return; };
        let id = tab.session_id;
        let Some(path) = tab.buffer.file_path.clone() else {
            self.set_status("Cannot move to trash: this document has no file path"); cx.notify(); return;
        };
        if unfinished(self, id) {
            self.set_status("Move to trash cancelled: finish the file load or current document edit first"); cx.notify(); return;
        }
        let Some(tab) = self.tabs.get_mut(idx) else { return; };
        tab.observe_content_activity();
        let target = Target::capture(tab, path, self.session_window_id, window.window_handle());
        let request = Uuid::new_v4();
        self.trash.pending = Some(request);
        let reply = confirmation(&target.path, target.dirty, window);
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = reply.await;
            let _ = this.update_in(cx, |workspace, window, cx| {
                if workspace.trash.pending != Some(request) { return; }
                workspace.trash.pending = None;
                if !confirmed { return; }
                if workspace.window_closed || workspace.session_window_id != target.window
                    || window.window_handle() != target.native_window { return; }
                let Some(idx) = workspace.tabs.iter().position(|tab| tab.session_id == target.tab) else { return; };
                if unfinished(workspace, target.tab) || !workspace.tabs.get(idx).is_some_and(|tab| target.matches(tab)) {
                    workspace.set_status("Move to trash cancelled: the document changed while confirmation was open");
                    cx.notify(); return;
                }
                workspace.trash_confirmed(idx, &target.path, window, cx);
            });
        }).detach();
    }

    fn trash_confirmed(&mut self, idx: usize, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let exit = self.tabs.len() == 1 && crate::settings::snapshot().tab_exit_on_last_close;
        // Closing the final window requires a recovery write. Refuse the filesystem
        // effect if even the current live document cannot be preserved first.
        if exit {
            if let Err(error) = save_recovery(self) {
                self.set_status(format!("Move to trash cancelled: session recovery could not be saved: {error}"));
                cx.notify(); return;
            }
        }
        // Keep the existing synchronous OS backend. No UI edit can interleave
        // between the ownership check, filesystem effect and document removal.
        if let Err(error) = delete(path) {
            self.set_status(format!("Move to trash failed: {error}")); cx.notify(); return;
        }
        self.automatic_name_context_changed(cx);
        let mut removed = self.tabs.remove(idx);
        if exit {
            self.active = 0;
            if let Err(error) = save_recovery(self) {
                // The file is already in trash. Keep this exact Tab (including its
                // UUID, incarnation and history) and require Save As to write it again.
                removed.buffer.new_label = path.file_name().map(|name| name.to_string_lossy().into_owned().into());
                removed.buffer.file_path = None;
                removed.buffer.read_only = false;
                removed.buffer.is_changelog = false;
                removed.buffer.forget_savepoint();
                self.tabs.push(removed);
                self.last_lines.clear(); self.caret_visible = true;
                self.set_status(format!("File moved to trash, but session recovery could not be saved: {error}; the document is still open, use Save As"));
                self.save_session(cx); cx.notify(); return;
            }
            self.release_session_window();
            self.tabs.push(Tab::empty());
            window.remove_window();
            return;
        }
        self.set_status(format!("Moved {} to trash", path.display()));
        self.settle_after_tab_removal(idx, window, cx);
    }
}

fn save_recovery(workspace: &mut Workspace) -> std::io::Result<()> {
    #[cfg(test)]
    if let Some(result) = test_support::next_recovery_result() {
        let previous = workspace.session_final_save_result.replace(result);
        let result = workspace.save_session_now();
        workspace.session_final_save_result = previous;
        return result;
    }
    workspace.save_session_now()
}

fn confirmation_description(path: &Path, dirty: bool) -> String {
    let destination = if cfg!(target_os = "windows") { "Recycle Bin" } else { "Trash" };
    let warning = if dirty { "\n\nUnsaved changes in this document will be discarded." } else { "" };
    format!("Move this file to the {destination} and close its document?\n\n{}{warning}", path.display())
}

fn confirmation(path: &Path, dirty: bool, window: &Window) -> impl std::future::Future<Output = bool> {
    let description = confirmation_description(path, dirty);
    #[cfg(test)]
    {
        let _ = window;
        let receiver = test_support::prompt(path, dirty, description);
        async move { match receiver { Some(receiver) => receiver.await.unwrap_or(false), None => false } }
    }
    #[cfg(not(test))]
    {
        let dialog = rfd::AsyncMessageDialog::new().set_parent(window).set_title("Delete file")
            .set_description(description).set_level(if dirty { rfd::MessageLevel::Warning } else { rfd::MessageLevel::Info })
            .set_buttons(rfd::MessageButtons::OkCancel);
        async move { matches!(dialog.show().await, rfd::MessageDialogResult::Ok) }
    }
}

fn delete(path: &Path) -> Result<(), String> {
    #[cfg(not(test))]
    { trash::delete(path).map_err(|error| error.to_string()) }
    #[cfg(test)]
    { test_support::delete(path) }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{cell::RefCell, collections::VecDeque, path::{Path, PathBuf}, rc::Rc};
    use futures::channel::oneshot;
    thread_local! { static BACKEND: RefCell<Option<Rc<RefCell<Backend>>>> = const { RefCell::new(None) }; }
    pub(crate) struct Prompt { pub(crate) path: PathBuf, pub(crate) dirty: bool, pub(crate) description: String }
    struct Backend {
        root: PathBuf, destination: PathBuf, calls: Vec<PathBuf>, fail: bool,
        prompts: Vec<Prompt>, replies: VecDeque<oneshot::Sender<bool>>, recovery: VecDeque<bool>,
    }
    pub(crate) struct Control { backend: Rc<RefCell<Backend>>, previous: Option<Rc<RefCell<Backend>>> }
    pub(crate) fn install(root: &Path) -> Control {
        let backend = Rc::new(RefCell::new(Backend { root: root.to_path_buf(), destination: root.join("simulated-trash.txt"),
            calls: Vec::new(), fail: false, prompts: Vec::new(), replies: VecDeque::new(), recovery: VecDeque::new() }));
        let previous = BACKEND.with(|current| current.replace(Some(backend.clone())));
        Control { backend, previous }
    }
    impl Control {
        pub(crate) fn calls(&self) -> Vec<PathBuf> { self.backend.borrow().calls.clone() }
        pub(crate) fn destination(&self) -> PathBuf { self.backend.borrow().destination.clone() }
        pub(crate) fn fail(&self) { self.backend.borrow_mut().fail = true; }
        pub(crate) fn recovery(&self, results: impl IntoIterator<Item = bool>) { self.backend.borrow_mut().recovery.extend(results); }
        pub(crate) fn answer(&self, confirmed: bool) {
            assert!(self.backend.borrow_mut().replies.pop_front().expect("pending dialog").send(confirmed).is_ok());
        }
        pub(crate) fn drop_reply(&self) { self.backend.borrow_mut().replies.pop_front().expect("pending dialog"); }
        pub(crate) fn prompts(&self) -> Vec<(PathBuf, bool, String)> {
            self.backend.borrow().prompts.iter().map(|prompt| (prompt.path.clone(), prompt.dirty, prompt.description.clone())).collect()
        }
    }
    impl Drop for Control { fn drop(&mut self) { BACKEND.with(|current| current.replace(self.previous.take())); } }
    pub(super) fn prompt(path: &Path, dirty: bool, description: String) -> Option<oneshot::Receiver<bool>> {
        let backend = BACKEND.with(|current| current.borrow().clone())?;
        let (sender, receiver) = oneshot::channel();
        let mut backend = backend.borrow_mut();
        backend.prompts.push(Prompt { path: path.to_path_buf(), dirty, description }); backend.replies.push_back(sender);
        Some(receiver)
    }
    pub(super) fn next_recovery_result() -> Option<bool> {
        BACKEND.with(|current| current.borrow().as_ref().and_then(|backend| backend.borrow_mut().recovery.pop_front()))
    }
    pub(super) fn delete(path: &Path) -> Result<(), String> {
        let backend = BACKEND.with(|current| current.borrow().clone()).ok_or_else(|| "No test trash backend installed".to_string())?;
        let mut backend = backend.borrow_mut();
        if path.parent() != Some(backend.root.as_path()) { return Err("Test trash refuses paths outside its fixture directory".to_string()); }
        backend.calls.push(path.to_path_buf());
        if backend.fail { return Err("injected trash failure".to_string()); }
        std::fs::rename(path, &backend.destination).map_err(|error| error.to_string())
    }
}
