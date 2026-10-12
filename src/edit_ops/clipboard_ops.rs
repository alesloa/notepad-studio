// Copy to Clipboard ▶ submenu — Current Full Path, Filename, Dir Path.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:1458 — IDM_EDIT_FULLPATHTOCLIP /
//                                          IDM_EDIT_CURRENTDIRTOCLIP /
//                                          IDM_EDIT_FILENAMETOCLIP
//
// NPP `Buffer::getFullPathName()` returns the document title for untitled
// docs (e.g. "new 1"), so on an untitled tab full-path and filename both
// land as the label and dir-path becomes empty (pathRemoveFileSpec on a
// bare title yields ""). We mirror that exactly using `display_short_name`
// for the untitled fallback.

use gpui::{ClipboardItem, Context, Window};

use crate::actions::*;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn copy_full_path(
        &mut self,
        _: &EditCopyFullPath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_current_name(true, cx);
    }

    pub(crate) fn copy_filename(
        &mut self,
        _: &EditCopyFilename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_current_name(false, cx);
    }

    fn copy_current_name(&mut self, full: bool, cx: &mut Context<Self>) {
        let request = crate::clipboard_ownership::claim(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
        self.cancel_bookmark_copy(cx);
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let original = self.active;
        let id = tab.session_id; let instance = tab.instance.clone();
        let revision = tab.buffer.raw_revision.clone(); let activity = tab.content_activity.clone();
        let path = tab.buffer.file_path.as_ref().map(|path| { let bytes = path.as_os_str().as_encoded_bytes(); (bytes.as_ptr() as usize, bytes.len()) });
        let custom = tab.buffer.new_label_is_custom;
        if let Some(ready) = self.wait_for_automatic_titles(cx) {
            cx.spawn(async move |this, cx| {
                let _ = ready.await;
                let _ = this.update(cx, |w, cx| {
                    if w.window_closed || !crate::clipboard_ownership::is_current(&request) { return; }
                    let valid = |tab: &crate::tab::Tab| tab.session_id == id && std::sync::Arc::ptr_eq(&tab.instance, &instance)
                        && tab.buffer.raw_revision == revision && tab.content_activity == activity && tab.buffer.new_label_is_custom == custom
                        && tab.buffer.file_path.as_ref().map(|path| { let bytes = path.as_os_str().as_encoded_bytes(); (bytes.as_ptr() as usize, bytes.len()) }) == path;
                    let index = if w.tabs.get(original).is_some_and(valid) { Some(original) }
                        else { w.tabs.iter().take(4096).position(valid) };
                    let Some(index) = index else {
                        w.set_status("Document changed while waiting to copy its name; repeat the command for the current document"); cx.notify(); return;
                    };
                    w.publish_current_name(index, full, &request, cx);
                });
            }).detach();
        } else { self.publish_current_name(self.active, full, &request, cx); }
    }

    fn publish_current_name(&mut self, index: usize, full: bool, request: &crate::clipboard_ownership::Request, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return; }; let buffer = &tab.buffer;
        let payload = if full { buffer.file_path.as_ref().map(|p| p.display().to_string()) }
            else { buffer.file_path.as_ref().and_then(|p| p.file_name()).map(|name| name.to_string_lossy().into_owned()) }
            .unwrap_or_else(|| buffer.display_short_name().to_string());
        if crate::clipboard_ownership::publish(request, cx, ClipboardItem::new_string(payload)) {
            self.set_status(if full { "Copied full path" } else { "Copied filename" }); cx.notify();
        }
    }

    pub(crate) fn copy_dir_path(
        &mut self,
        _: &EditCopyDirPath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::clipboard_ownership::supersede();
        self.cancel_bookmark_copy(cx);
        let buffer = self.buffer();
        let payload = buffer
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        crate::clipboard_ownership::write_now(cx, ClipboardItem::new_string(payload));
        self.set_status("Copied directory path");
    }
}
