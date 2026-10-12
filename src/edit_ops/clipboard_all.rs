// PowerEditor/src/NppCommands.cpp:1529-1549 — Copy All Filenames / File Paths.
// MISC/Common/Common.cpp:1088-1106 — clipboard rows end in CRLF.
// NppCommands.cpp:4590-4591 — both commands are macro-recordable.
// Notepad Studio has one tab strip per window; use its visual tab order.
// Distinct tabs retain duplicate names and paths, as distinct oracle buffers do.

use gpui::{ClipboardItem, Context, Window};

use crate::actions::{EditCopyAllFilePaths, EditCopyAllFilenames};
use crate::{macros::MacroEvent, tab::Tab, workspace::Workspace};

#[cfg(test)]
#[path = "clipboard_all_tests.rs"]
pub(crate) mod tests;

fn tab_labels(tabs: &[Tab], full_paths: bool) -> Result<Option<String>, &'static str> {
    let mut output = String::new();
    for tab in tabs {
        let label = if full_paths {
            tab.buffer
                .file_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| tab.buffer.display_short_name().to_string())
        } else {
            tab.buffer.display_short_name().to_string()
        };
        let additional = label
            .len()
            .checked_add(2)
            .ok_or("Too many document names to copy")?;
        output
            .try_reserve(additional)
            .map_err(|_| "Not enough memory to copy document names")?;
        output.push_str(&label);
        if !output.is_empty() && !output.ends_with("\r\n") {
            output.push_str("\r\n");
        }
    }
    Ok((!output.is_empty()).then_some(output))
}

impl Workspace {
    pub(crate) fn copy_all_filenames(
        &mut self,
        _: &EditCopyAllFilenames,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_all_tab_labels(false, cx);
    }

    pub(crate) fn copy_all_file_paths(
        &mut self,
        _: &EditCopyAllFilePaths,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_all_tab_labels(true, cx);
    }

    pub(crate) fn copy_all_tab_labels(&mut self, full_paths: bool, cx: &mut Context<Self>) -> bool {
        let request = crate::clipboard_ownership::claim(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
        self.cancel_bookmark_copy(cx);
        crate::macros::record(if full_paths {
            MacroEvent::CopyAllFilePaths
        } else {
            MacroEvent::CopyAllFilenames
        });
        if let Some(ready) = self.wait_for_automatic_titles(cx) {
            let stamp = self.automatic_title_consumer_stamp();
            let playback = self.begin_title_clipboard_wait();
            cx.spawn(async move |this, cx| {
                let _ = ready.await;
                let _ = this.update(cx, |w, cx| {
                    w.finish_title_clipboard_wait(playback, cx, move |w, cx| {
                        if w.window_closed || !crate::clipboard_ownership::is_current(&request) { return false; }
                        if w.automatic_title_consumer_stamp() != stamp {
                            w.set_status("Document names changed while waiting to copy them; repeat the command for the current names"); cx.notify(); return false;
                        }
                        w.publish_all_tab_labels(full_paths, &request, cx)
                    });
                });
            }).detach(); return true;
        }
        self.publish_all_tab_labels(full_paths, &request, cx)
    }

    fn publish_all_tab_labels(&mut self, full_paths: bool, request: &crate::clipboard_ownership::Request, cx: &mut Context<Self>) -> bool {
        match tab_labels(&self.tabs, full_paths) {
            Ok(Some(payload)) => {
                if !crate::clipboard_ownership::publish(request, cx, ClipboardItem::new_string(payload)) { return false; }
                self.set_status(if full_paths {
                    "Copied all file paths"
                } else {
                    "Copied all filenames"
                });
                cx.notify();
                true
            }
            Ok(None) => true,
            Err(error) => {
                self.set_status(error);
                cx.notify();
                false
            }
        }
    }
}
