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
        crate::clipboard_ownership::supersede();
        self.cancel_bookmark_copy(cx);
        let buffer = self.buffer();
        let payload = match buffer.file_path.as_ref() {
            Some(p) => p.display().to_string(),
            None => buffer.display_short_name().to_string(),
        };
        crate::clipboard_ownership::write_now(cx, ClipboardItem::new_string(payload));
        self.set_status("Copied full path");
    }

    pub(crate) fn copy_filename(
        &mut self,
        _: &EditCopyFilename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::clipboard_ownership::supersede();
        self.cancel_bookmark_copy(cx);
        let buffer = self.buffer();
        let payload = match buffer
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
        {
            Some(name) => name.to_string_lossy().into_owned(),
            None => buffer.display_short_name().to_string(),
        };
        crate::clipboard_ownership::write_now(cx, ClipboardItem::new_string(payload));
        self.set_status("Copied filename");
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
