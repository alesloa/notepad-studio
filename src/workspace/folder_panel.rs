// Workspace methods backing the folder-as-workspace left dock. Click-to-
// open, splitter drag bookkeeping, right-click context-menu state, and
// directory-cache invalidation. The actual rendering lives in
// `ui::folder_panel`.

use std::path::PathBuf;

use gpui::{Context, Pixels, Point, Window};

use crate::find_state::{FolderInlineCreateKind, FolderInlineCreateState};
use crate::tab::InputFocus;
use crate::ui::folder_panel::{
    self, SplitterDrag,
    context_menu::{FolderContextMenuState, RowKind},
};
use crate::workspace::Workspace;

impl Workspace {
    /// Click handler for any tree row. Folders toggle expansion; files
    /// route to `open_path` (same path as File ▸ Open). The clicked path
    /// is also saved on `folder_panel.selected` for the highlight.
    pub(crate) fn handle_folder_panel_click(
        &mut self,
        path: PathBuf,
        is_dir: bool,
        _kind: RowKind,
        cx: &mut Context<Self>,
    ) {
        self.folder_panel.selected = Some(path.clone());
        if is_dir {
            let now_open = folder_panel::toggle_expanded(&path);
            if !now_open {
                self.folder_panel.invalidate(&path);
            }
            cx.notify();
        } else {
            self.open_path(path, cx);
        }
    }

    /// Open the right-click context menu for a folder-panel row at the
    /// pointer position. Phase 4 wires the actual menu surface; this just
    /// stashes the state so render() can pick it up.
    pub(crate) fn show_folder_panel_context_menu(
        &mut self,
        path: PathBuf,
        kind: RowKind,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.folder_panel.context_menu = Some(FolderContextMenuState {
            target: path,
            position,
            kind,
        });
        cx.notify();
    }

    /// Called when the splitter handle's mouse-down lands. Captures the
    /// initial mouse x and the persisted width so subsequent moves can
    /// compute the new width as `start_width + (current_x - start_x)`.
    pub(crate) fn begin_folder_panel_drag(&mut self, start_x: Pixels, cx: &mut Context<Self>) {
        let snap = crate::settings::snapshot();
        self.folder_panel.drag = Some(SplitterDrag {
            start_mouse_x: start_x,
            start_width: snap.folder_panel_width,
        });
        cx.notify();
    }

    /// Apply an in-flight splitter drag — caller passes the current pointer
    /// x; the panel width is derived from the captured drag state. Called
    /// from the `FolderPanelSplitter` element's mouse-move handler.
    pub(crate) fn drag_folder_panel(&mut self, current_x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.folder_panel.drag else {
            return;
        };
        let dx = f32::from(current_x) - f32::from(drag.start_mouse_x);
        let next = drag.start_width + dx;
        folder_panel::set_width(next);
        cx.notify();
    }

    /// End an in-flight splitter drag. No-op when no drag is in progress.
    pub(crate) fn end_folder_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.folder_panel.drag.take().is_some() {
            cx.notify();
        }
    }

    /// Hide the folder-panel right-click context menu.
    pub(crate) fn hide_folder_panel_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.folder_panel.context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Open the inline-create draft inside `parent` (auto-expanding it if it
    /// isn't already). `kind` decides whether the entry will become a file
    /// or a folder on commit. Lazily creates the focus handle on first open
    /// and focuses it so typed characters route in immediately.
    pub(crate) fn begin_folder_inline_create(
        &mut self,
        parent: PathBuf,
        kind: FolderInlineCreateKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !parent.is_dir() {
            self.set_status("Cannot create here — parent is not a directory");
            return;
        }
        // Ensure the parent shows its children so the draft row is visible.
        let snap = crate::settings::snapshot();
        if !folder_panel::is_expanded(&snap.folder_panel_expanded, &parent) {
            folder_panel::toggle_expanded(&parent);
        }
        if self.folder_inline_create_focus.is_none() {
            self.folder_inline_create_focus = Some(cx.focus_handle());
        }
        self.folder_inline_create = Some(FolderInlineCreateState::new(parent, kind));
        self.focus_input(InputFocus::FolderInlineCreate, window, cx);
        cx.notify();
    }

    /// Dismiss the inline-create draft without touching the filesystem.
    pub(crate) fn cancel_folder_inline_create(&mut self, cx: &mut Context<Self>) {
        if self.folder_inline_create.take().is_some() {
            cx.notify();
        }
    }

    /// Validate the typed name and create the file/folder on disk. On success
    /// the directory cache is invalidated, the new path is selected, and (for
    /// files) the buffer opens in a tab. On validation failure the draft
    /// stays open with `error` populated so the user can correct the name.
    pub(crate) fn commit_folder_inline_create(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.folder_inline_create.as_ref() else {
            return;
        };
        let parent = draft.parent_dir.clone();
        let kind = draft.kind;
        let raw_name = draft.text.trim().to_string();

        if let Err(err) = validate_entry_name(&raw_name) {
            if let Some(d) = self.folder_inline_create.as_mut() {
                d.error = Some(err);
            }
            cx.notify();
            return;
        }

        let target = parent.join(&raw_name);
        if target.exists() {
            if let Some(d) = self.folder_inline_create.as_mut() {
                d.error = Some(format!("'{raw_name}' already exists"));
            }
            cx.notify();
            return;
        }

        let create_result = match kind {
            FolderInlineCreateKind::File => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map(|_| ()),
            FolderInlineCreateKind::Folder => std::fs::create_dir(&target),
        };
        if let Err(err) = create_result {
            if let Some(d) = self.folder_inline_create.as_mut() {
                d.error = Some(format!("{err}"));
            }
            cx.notify();
            return;
        }

        self.folder_inline_create = None;
        folder_panel::cache_invalidate(&parent);
        self.folder_panel.selected = Some(target.clone());
        match kind {
            FolderInlineCreateKind::File => {
                self.open_path(target.clone(), cx);
                self.set_status(format!("Created file '{}'", target.display()));
            }
            FolderInlineCreateKind::Folder => {
                self.set_status(format!("Created folder '{}'", target.display()));
            }
        }
        cx.notify();
    }
}

/// Reject names that the OS will choke on or that would escape the parent
/// directory. We don't try to enforce every OS-specific quirk — `OpenOptions`
/// surfaces those at create time and the error bubbles into `d.error`.
fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name cannot be empty".into());
    }
    if name == "." || name == ".." {
        return Err("Reserved name".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("Name cannot contain '/' or '\\'".into());
    }
    if name.contains('\0') {
        return Err("Name contains a null byte".into());
    }
    Ok(())
}
