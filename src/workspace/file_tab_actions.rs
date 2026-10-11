// PowerEditor/src/Notepad_plus.rc:449-473; Parameters.cpp:110-127 — current-file commands.
// Menu wrappers reuse the tab helpers' UUID-aware async reload/close flows.
use std::path::{Path, PathBuf};

use crate::{actions::*, workspace::Workspace};
use gpui::{Context, FocusHandle, Window};

// Capture the test response on the caller thread, before spawn_in can poll elsewhere.
pub(super) fn confirm_reload(name: String) -> impl std::future::Future<Output = bool> {
    #[cfg(test)]
    {
        let response = file_tab_action_tests::reload_response();
        async move {
            match response {
                Some(response) => response,
                None => show_reload_confirmation(name).await,
            }
        }
    }
    #[cfg(not(test))]
    {
        show_reload_confirmation(name)
    }
}

async fn show_reload_confirmation(name: String) -> bool {
    let answer = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(crate::consts::APP_NAME)
        .set_description(format!("Reload {name} and lose its unsaved changes?"))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        .await;
    matches!(answer, rfd::MessageDialogResult::Yes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileLaunch {
    Reveal,
    Viewer,
}

fn launch_file(kind: FileLaunch, path: &Path) -> Result<(), String> {
    #[cfg(test)]
    if let Some(result) = file_tab_action_tests::capture_launch(kind, path) {
        return result;
    }
    match kind {
        FileLaunch::Reveal => crate::shell::reveal_in_file_manager(path),
        FileLaunch::Viewer => crate::shell::open_with_default_viewer(path),
    }
}

impl Workspace {
    pub(super) fn csv_focus_for_reload(&self, index: usize, window: &Window) -> Option<FocusHandle> {
        if index != self.active { return None; }
        let tab = self.tabs.get(index)?;
        if tab.view_mode != crate::tab::TabViewMode::CsvPreview {
            return None;
        }
        let model = self.csv_models.get(&tab.session_id)?;
        model
            .cell_input
            .as_ref()
            .map(|input| input.focus_handle.clone())
            .filter(|handle| handle.is_focused(window))
            .or_else(|| {
                model
                    .grid_focus
                    .clone()
                    .filter(|handle| handle.is_focused(window))
            })
    }

    pub(super) fn restore_reloaded_csv_focus(
        &mut self,
        id: uuid::Uuid,
        handle: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() || !handle.is_focused(window) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.session_id != id || tab.view_mode != crate::tab::TabViewMode::CsvPreview {
            return;
        }
        crate::ui::csv_doc::actions::ensure_model(self, window, cx);
        if let Some(focus) = self
            .csv_models
            .get(&id)
            .and_then(|model| model.grid_focus.clone())
        {
            window.focus(&focus, cx);
        }
    }

    fn file_action_tab_index(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        if self.tabs.get(self.active).is_some() {
            return Some(self.active);
        }
        self.set_status("No active document for this File command");
        cx.notify();
        None
    }

    fn file_action_path(&mut self, cx: &mut Context<Self>) -> Option<PathBuf> {
        let index = self.file_action_tab_index(cx)?;
        let Some(path) = self
            .tabs
            .get(index)
            .and_then(|tab| tab.buffer.file_path.as_ref())
        else {
            self.set_status("Current document has no file path");
            cx.notify();
            return None;
        };
        if path.as_os_str().to_string_lossy().contains('\0') {
            self.set_status("Current document has an invalid file path");
            cx.notify();
            return None;
        }
        match std::path::absolute(path) {
            Ok(path) => Some(path),
            Err(error) => {
                self.set_status(format!("Cannot resolve document path: {error}"));
                cx.notify();
                None
            }
        }
    }

    pub(crate) fn file_open_containing_folder(
        &mut self,
        _: &FileOpenContainingFolder,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.file_action_path(cx) else {
            return;
        };
        match launch_file(FileLaunch::Reveal, &path) {
            Ok(()) => self.set_status(format!("Revealing {}", path.display())),
            Err(error) => self.set_status(format!("Open containing folder failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn file_open_default_viewer(
        &mut self,
        _: &FileOpenDefaultViewer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.file_action_path(cx) else {
            return;
        };
        match launch_file(FileLaunch::Viewer, &path) {
            Ok(()) => self.set_status(format!("Opening saved file {}", path.display())),
            Err(error) => self.set_status(format!("Open in default viewer failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn file_reload_from_disk(
        &mut self,
        _: &FileReloadFromDisk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.file_action_tab_index(cx) else {
            return;
        };
        self.reload_tab(index, window, cx);
    }

    pub(crate) fn file_close_all_but_active(
        &mut self,
        _: &FileCloseAllButActive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.file_action_tab_index(cx) else {
            return;
        };
        self.close_autocomplete(cx);
        self.close_all_but(index, window, cx);
    }

    pub(crate) fn file_close_all_to_left(
        &mut self,
        _: &FileCloseAllToLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.file_action_tab_index(cx) else {
            return;
        };
        self.close_autocomplete(cx);
        self.close_all_to_left(index, window, cx);
    }

    pub(crate) fn file_close_all_to_right(
        &mut self,
        _: &FileCloseAllToRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.file_action_tab_index(cx) else {
            return;
        };
        self.close_autocomplete(cx);
        self.close_all_to_right(index, window, cx);
    }

    pub(crate) fn file_close_all_unchanged(
        &mut self,
        _: &FileCloseAllUnchanged,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_action_tab_index(cx).is_none() {
            return;
        }
        let ids: Vec<_> = self.tabs.iter().map(|tab| tab.session_id).collect();
        for id in ids {
            if crate::ui::csv_doc::actions::commit_cell_edit_for_tab(self, id, cx).is_err() {
                return;
            }
        }
        self.close_autocomplete(cx);
        self.close_all_unchanged(window, cx);
    }
}

#[cfg(test)]
#[path = "file_tab_action_tests.rs"]
mod file_tab_action_tests;
