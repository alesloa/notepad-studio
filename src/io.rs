mod save_as_dialog;
mod save_copy;
mod copy_writer;
mod copy_stage;
mod copy_snapshot;
mod background_drop;
mod safety_copy;
mod restore_copy;
use safety_copy::make_safety_copy;
pub(crate) use background_drop::BackgroundDrop;
pub(crate) use copy_stage::Progress as CopyProgress;
pub(crate) use save_as_dialog::pick_rename as pick_rename_path;

mod save_all;
#[cfg(test)]
pub(crate) use save_as_dialog::Override as SaveAsDialogOverrideForTest;
#[cfg(test)]
pub(crate) use save_as_dialog::RequestCapture as SaveDialogRequestCaptureForTest;
#[cfg(test)]
mod save_as_identity_tests;
#[cfg(test)]
mod save_all_tests;
#[cfg(all(test, target_os = "windows"))]
#[path = "io/save_durability_tests.rs"]
mod save_durability_tests;
#[cfg(all(test, target_os = "windows"))]
#[path = "io/save_durability_caller_tests.rs"]
mod save_durability_caller_tests;

// File I/O — open / save / save-as flows plus the async dialog helpers
// invoked from the close-tab path.
//
// Dialog helpers MUST stay async. Sync `rfd::MessageDialog` / `FileDialog`
// block on the AppKit run loop, which drains gpui's dispatch queue while
// an outer `entity.update` is still holding the `AppCell` borrow. Any
// timer or task that fires during the dialog (e.g. caret blink) re-enters
// that borrow and panics with "already borrowed". Always go through
// `cx.spawn` / `cx.spawn_in` and use the `Async*Dialog` variants.

use std::path::{Path, PathBuf};

use gpui::{AsyncWindowContext, Context, WeakEntity, Window};
use uuid::Uuid;

use crate::actions::{
    FileNew, FileOpen, FileOpenFolderAsWorkspace, FileSave, FileSaveAll, FileSaveAs,
};
use crate::app::menus::rebuild_menus;
use crate::buffer::Buffer;
use crate::consts::APP_NAME;
use crate::recent;
use crate::tab::{CloseDecision, Tab, TabViewMode};
use crate::text::encoding::{self, Encoding};
use crate::text::line_ending::{rope_to_bytes, split_line_endings};
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn open_new_tab(&mut self, _: &FileNew, _w: &mut Window, cx: &mut Context<Self>) {
        self.new_tab(cx);
    }

    /// True when the active tab is a brand-new empty buffer that the user
    /// hasn't typed into. Opening into such a tab replaces it instead of
    /// stacking another empty next to it — matches Notepad++.
    fn active_is_pristine_empty(&self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        tab.buffer.rope.len_bytes() == 0
            && tab.buffer.file_path.is_none()
            && !tab.buffer.dirty
    }

    /// Replace the active tab's buffer with `buffer` (used after Open lands a
    /// file in a pristine empty tab). Everything tied to the old text goes,
    /// so the load is a clean slate.
    fn replace_active_buffer(&mut self, buffer: Buffer) {
        reset_tab_with(self.active_tab_mut(), buffer);
    }

    /// Run `f` with the tab whose `session_id` is `id` made active, then
    /// make the tab that was active before active again. `None` when the
    /// tab was closed, e.g. while a Save As dialog was open: the tab list
    /// can change under an await, so an index captured before it may now
    /// name another tab.
    fn with_tab_active<R>(&mut self, id: Uuid, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let idx = self.tabs.iter().position(|t| t.session_id == id)?;
        let prev = self.tabs.get(self.active).map(|t| t.session_id);
        self.active = idx;
        let result = f(self);
        let back = prev.and_then(|p| self.tabs.iter().position(|t| t.session_id == p));
        if let Some(back) = back {
            self.active = back;
        }
        Some(result)
    }

    fn append_buffer_as_tab(&mut self, buffer: Buffer) {
        let mut tab = Tab::empty();
        tab.buffer = buffer;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.touch_mru();
    }

    /// Load `path` from disk, detect line ending, and route into either the
    /// active pristine tab (replace) or a new tab (append). Status bar
    /// reflects success or failure.
    ///
    /// Files larger than `STREAM_THRESHOLD_BYTES` route through the
    /// streaming loader (`crate::io_stream`) instead of the synchronous
    /// `std::fs::read` path so the UI stays responsive while the rope
    /// fills in chunk by chunk. Streaming requires a `Window` — use
    /// `open_path_window` from any `&mut Window` call site, or stick with
    /// this function and accept a synchronous load up to the threshold.
    ///
    /// On a successful load `path` is also pushed to the front of the global
    /// MRU list (`File ▸ Open Recent`) and the menu bar is rebuilt so the
    /// recents submenu reflects the change immediately. A failed read does
    /// NOT pollute the MRU — recents only ever lists paths that opened OK.
    pub(crate) fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.activate_open_tab_for(&path, cx) {
            return;
        }
        let display = path.display().to_string();

        // Stat first — files larger than the streaming threshold go through
        // `io_stream` so the UI never waits on a multi-hundred-MB read.
        // Rich documents (DOCX, ODT, RTF) are containers, not text: they
        // always take the parser below, whatever their size.
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file()
                && meta.len() > crate::io_stream::STREAM_THRESHOLD_BYTES
                && crate::rich_doc::detect_format(&path).is_none()
            {
                crate::io_stream::open_path_streaming(self, path, meta.len(), cx);
                cx.notify();
                return;
            }
        }

        let buffer = match read_buffer(&path) {
            Ok(buffer) => buffer,
            Err(err) => {
                self.set_status(format!("Failed to open {display}: {err}"));
                cx.notify();
                return;
            }
        };
        let is_rich_doc = buffer.is_rich_doc();
        let lossy_decode = buffer.read_only;
        let enc = buffer.encoding;

        let basename = buffer.display_short_name().to_string();
        let is_csv = crate::ui::csv_doc::is_csv_path(Some(&path));
        if self.active_is_pristine_empty() {
            if let Some(id) = self.tabs.get(self.active).map(|tab| tab.session_id) {
                self.retire_syntax_highlights(id, cx);
            }
            self.replace_active_buffer(buffer);
            if is_rich_doc {
                self.active_tab_mut().view_mode = TabViewMode::RichDocView;
            } else if is_csv {
                self.active_tab_mut().view_mode = TabViewMode::CsvPreview;
            }
        } else {
            self.append_buffer_as_tab(buffer);
            if is_rich_doc {
                self.active_tab_mut().view_mode = TabViewMode::RichDocView;
            } else if is_csv {
                self.active_tab_mut().view_mode = TabViewMode::CsvPreview;
            }
        }

        if lossy_decode {
            self.set_status(lossy_decode_status(&basename, enc));
        } else {
            self.set_status(format!("Opened {basename}"));
        }
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
        // Update MRU memory before rebuilding the menu. Disk persistence is
        // coalesced on the background executor and does not delay this open.
        recent::record_opened(path);
        rebuild_menus(&mut *cx);
        if let Some(id) = self.tabs.get(self.active).map(|tab| tab.session_id) {
            self.request_syntax_refresh_for_tab(id, std::time::Duration::ZERO, cx);
        }
        cx.notify();
    }

    /// Open the bundled `change.log` as a read-only ephemeral tab. Used by
    /// the first-launch / post-update auto-open in `crate::changelog`. Does
    /// NOT push to recents, does NOT call `save_session`, and only steals
    /// focus when the workspace has nothing real loaded.
    ///
    /// Replaces any existing changelog tab in place — a session-restored
    /// copy from the prior version's auto-open would otherwise stack
    /// alongside the freshly-loaded one after an update.
    pub(crate) fn open_changelog_silent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let raw_bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(err) => {
                self.set_status(format!("Failed to open change.log: {err}"));
                cx.notify();
                return;
            }
        };
        let (enc, raw) = encoding::decode(&raw_bytes);
        let (line_ending, normalized) = split_line_endings(raw);
        let rope = ropey::Rope::from_str(&normalized);

        let mut buffer = Buffer {
            rope,
            file_path: Some(path),
            dirty: false,
            line_ending,
            encoding: enc,
            bookmarks: std::collections::BTreeSet::new(),
            marks: Vec::new(),
            language: None,
            highlights: std::sync::Arc::new(Vec::new()),
            loading: None,
            new_label: None,
            new_label_is_custom: false,
            read_only: true,
            tab_color: None,
            is_changelog: true,
            rich_doc: None,
            flat_text_cache: None,
            raw_revision: crate::buffer::ContentRevision::default(),
            savepoint: None,
        };
        buffer.mark_saved();

        // Replace an existing changelog tab in place — happens on a
        // version bump where the prior version's changelog is still in
        // the restored session.
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.buffer.is_changelog) {
            reset_tab_with(tab, buffer);
        } else if self.active_is_pristine_empty() && self.tabs.len() == 1 {
            // Steal focus only when the workspace currently has nothing
            // the user cares about — a single pristine empty tab.
            self.replace_active_buffer(buffer);
        } else {
            // Otherwise append silently so the user's restored session
            // stays foregrounded.
            let prev_active = self.active;
            self.append_buffer_as_tab(buffer);
            self.active = prev_active;
        }
        self.last_lines.clear();
        self.caret_visible = true;
        cx.notify();
    }

    pub(crate) fn open_dialog(&mut self, _: &FileOpen, window: &mut Window, cx: &mut Context<Self>) {
        let active_dir = self
            .buffer()
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let starting = crate::settings::resolve_starting_dir(active_dir.as_deref())
            .or(active_dir)
            .or_else(dirs::document_dir)
            .or_else(dirs::home_dir);

        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Open File");
            if let Some(dir) = starting {
                dialog = dialog.set_directory(dir);
            }
            let Some(handle) = dialog.pick_file().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            crate::settings::record_last_used_directory(&path);
            this.update(cx, |w, cx| w.open_path(path, cx)).ok();
        })
        .detach();
    }

    /// `File ▸ Open Folder as Workspace…`. Spawns the OS folder picker;
    /// the picked directory is appended to the persisted `folder_panel_roots`
    /// list and the panel is auto-shown. Mirrors `open_dialog`'s async
    /// pattern — sync `rfd::FileDialog::pick_folder` would block the gpui
    /// main thread.
    pub(crate) fn open_folder_as_workspace(
        &mut self,
        _: &FileOpenFolderAsWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_dir = self
            .buffer()
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let starting = crate::settings::resolve_starting_dir(active_dir.as_deref())
            .or(active_dir)
            .or_else(dirs::document_dir)
            .or_else(dirs::home_dir);

        cx.spawn_in(window, async move |this, cx| {
            let mut dialog =
                rfd::AsyncFileDialog::new().set_title("Open Folder as Workspace");
            if let Some(dir) = starting {
                dialog = dialog.set_directory(dir);
            }
            let Some(handle) = dialog.pick_folder().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            crate::settings::record_last_used_directory(&path);
            this.update(cx, |w, cx| {
                let n = crate::ui::folder_panel::add_root(path.clone());
                crate::ui::folder_panel::set_visible(true);
                w.set_status(format!(
                    "Added '{}' as workspace root ({n} total)",
                    path.display()
                ));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Drop-to-open handler. Called when the user drags one or more files
    /// from Finder / Explorer onto the editor window. Each file path goes
    /// through `open_path`, which lands the first one in the active pristine
    /// tab (if any) and stacks the rest as new tabs. Directories are filtered
    /// out and reported through an async info dialog — Notepad++ rejects
    /// folder drops, and we match that.
    pub(crate) fn handle_external_paths_drop(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (files, dir_count): (Vec<PathBuf>, usize) = {
            let mut files = Vec::new();
            let mut dirs = 0usize;
            for p in paths {
                if p.is_dir() {
                    dirs += 1;
                } else {
                    files.push(p);
                }
            }
            (files, dirs)
        };

        for path in files {
            self.open_path(path, cx);
        }

        if dir_count > 0 {
            let msg = if dir_count == 1 {
                "Cannot open folder. Drop a file instead.".to_string()
            } else {
                format!("Cannot open {dir_count} folders. Drop files instead.")
            };
            cx.spawn_in(window, async move |_this, _cx| {
                rfd::AsyncMessageDialog::new()
                    .set_level(rfd::MessageLevel::Info)
                    .set_title(APP_NAME)
                    .set_description(msg)
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show()
                    .await;
            })
            .detach();
        }
    }

    /// Write the active tab's rope to its `file_path`, returning whether the
    /// write succeeded. Caller is expected to have ensured `file_path` is
    /// already set (via Save As if needed).
    ///
    /// A tab flagged read-only is refused, as Notepad++'s `fileSave` does
    /// (NppIO.cpp:1777 "The file is read-only and cannot be saved"). Save As
    /// goes through `write_active_to_disk_as`, which writes a new file and
    /// so is allowed.
    pub(crate) fn write_active_to_disk(&mut self) -> Result<(), String> {
        self.write_active_inner(false)
    }

    /// `write_active_to_disk` for Save As: the target is a path the user
    /// just picked, so the tab's read-only flag doesn't block it.
    pub(crate) fn write_active_to_disk_as(&mut self) -> Result<(), String> {
        self.write_active_inner(true)
    }

    fn write_active_inner(&mut self, save_as: bool) -> Result<(), String> {
        let active = self.active;
        let Some(tab) = self.tabs.get_mut(active) else {
            return Err("no active tab".to_string());
        };
        let Some(path) = tab.buffer.file_path.clone() else {
            return Err("no path".to_string());
        };
        // A file still streaming in is only partly in the rope; writing it
        // would cut the file short.
        if tab.buffer.loading.is_some() {
            return Err(format!("{} is still loading", path.display()));
        }
        if tab.buffer.read_only && !save_as {
            return Err(format!(
                "{} is read-only in Notepad Studio; clear Read-Only or use Save As",
                path.display()
            ));
        }
        copy_writer::write_buffer(&tab.buffer, &path)?;
        tab.buffer.mark_saved();
        Ok(())
    }

    /// Save As core: point the active tab at `path` and write it. If the
    /// write fails the old path and "new N" label go back, so the tab never
    /// claims a file that was not written.
    fn save_active_to_new_path(&mut self, path: PathBuf) -> Result<(), String> {
        let (old_path, old_label) = {
            let buf = &mut self.active_tab_mut().buffer;
            (buf.file_path.replace(path), buf.new_label.take())
        };
        let result = self.write_active_to_disk_as();
        if result.is_err() {
            let buf = &mut self.active_tab_mut().buffer;
            buf.file_path = old_path;
            buf.new_label = old_label;
        }
        result
    }

    pub(crate) fn save_active(&mut self, _: &FileSave, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        if self.buffer().file_path.is_none() {
            self.save_active_as(&FileSaveAs, window, cx);
            return;
        }
        // Legacy `.doc` can't be re-serialized — re-route Cmd+S to Save As
        // .docx so the user gets a useful action instead of a save error.
        if let Some(payload) = self.buffer().rich_doc.as_ref() {
            if payload.format == crate::rich_doc::DocFormat::Doc {
                self.rich_doc_save_as_docx(
                    &crate::actions::rich_doc::RichDocSaveAsDocx,
                    window,
                    cx,
                );
                return;
            }
        }
        match self.write_active_to_disk() {
            Ok(()) => {
                let name = self.buffer().display_short_name().to_string();
                self.set_status(format!("Saved {name}"));
            }
            Err(err) => {
                self.set_status(format!("Save failed: {err}"));
            }
        }
        self.save_session(cx);
        cx.notify();
    }

    pub(crate) fn save_active_as(&mut self, _: &FileSaveAs, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        // PowerEditor/src/NppIO.cpp:1994-2090 saves the requesting buffer.
        // Async replies also need its incarnation: restoring a closed tab
        // retains the session ID but must not inherit the old dialog.
        let Some((tab_id, tab_instance)) = self.tabs.get(self.active).map(|t| (t.session_id, t.instance.clone())) else {
            return;
        };
        let active_dir = self
            .buffer()
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let starting = crate::settings::resolve_starting_dir(active_dir.as_deref())
            .or(active_dir)
            .or_else(dirs::document_dir)
            .or_else(dirs::home_dir);

        let suggested = suggested_save_name(self.buffer());

        let selected = save_as_dialog::pick(starting, suggested);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = selected.await else { return; };
            let Some(refused) = this
                .update(cx, |w, cx| {
                    if w.window_closed { return None; }
                    let saving = w.tabs.iter().position(|t| t.session_id == tab_id
                        && std::sync::Arc::ptr_eq(&t.instance, &tab_instance))?;
                    Some(w.save_target_open_elsewhere(&path, saving, cx))
                })
                .ok().flatten() else { return; };
            if refused {
                crate::workspace::show_already_open_error().await;
                return;
            }
            this.update(cx, |w, cx| {
                if w.window_closed || !w.tabs.iter().any(|t| t.session_id == tab_id
                    && std::sync::Arc::ptr_eq(&t.instance, &tab_instance)) { return; }
                if crate::ui::csv_doc::actions::commit_cell_edit_for_tab(w, tab_id, cx).is_err() { return; }
                crate::settings::record_last_used_directory(&path);
                let outcome = w.with_tab_active(tab_id, |w| {
                    w.save_active_to_new_path(path.clone())
                        .map(|()| w.buffer().display_short_name().to_string())
                });
                let saved = match outcome {
                    Some(Ok(name)) => {
                        w.set_status(format!("Saved {name}"));
                        true
                    }
                    Some(Err(err)) => {
                        w.set_status(format!("Save failed: {err}"));
                        false
                    }
                    None => {
                        w.set_status("Not saved: the tab was closed".to_string());
                        false
                    }
                };
                w.save_session(cx);
                if saved {
                    recent::record_opened(path);
                    rebuild_menus(&mut *cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn save_all(&mut self, _: &FileSaveAll, window: &mut Window, cx: &mut Context<Self>) {
        save_all::start(self, window, cx);
    }

    /// Commit every pending CSV cell before choosing the dirty documents.
    fn commit_csv_drafts_for_save_all(&mut self, cx: &mut Context<Self>) -> bool {
        let ids: Vec<_> = self.tabs.iter().map(|tab| tab.session_id).collect();
        for id in ids {
            if crate::ui::csv_doc::actions::commit_cell_edit_for_tab(self, id, cx).is_err() { return false; }
        }
        true
    }

    fn run_save_all(&mut self, cx: &mut Context<Self>) {
        if !self.commit_csv_drafts_for_save_all(cx) { return; }
        let mut saved = 0usize;
        let mut failures: Vec<String> = Vec::new();
        let prev_active = self.active;
        for idx in 0..self.tabs.len() {
            // Notepad++'s Save All saves only modified documents
            // (`fileSaveAll` → `fileSave`, which returns early unless
            // `isDirty()`). Rewriting clean files would bump their mtime and
            // overwrite any change made on disk since they were opened.
            let Some(tab) = self.tabs.get(idx) else { continue };
            if tab.buffer.file_path.is_none() || !tab.buffer.dirty {
                continue;
            }
            self.active = idx;
            match self.write_active_to_disk() {
                Ok(()) => saved += 1,
                Err(err) => {
                    let name = self
                        .tabs
                        .get(idx)
                        .map(|t| t.buffer.display_short_name().to_string())
                        .unwrap_or_default();
                    failures.push(format!("{name}: {err}"));
                }
            }
        }
        self.active = prev_active;

        let status = if failures.is_empty() {
            format!("Saved {saved} file(s)")
        } else {
            format!(
                "Saved {saved}, {} failed: {}",
                failures.len(),
                failures.join("; ")
            )
        };
        self.set_status(status);
        self.save_session(cx);
        cx.notify();
    }
}

/// Why `write_file_safely` didn't write. Every variant leaves the original
/// file as it was, except `NotRestored`, which names where its copy is.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SaveError {
    #[error("{} is read-only; clear the read-only attribute or use Save As", path.display())]
    ReadOnly { path: PathBuf },
    #[error("{} is a folder", path.display())]
    IsDirectory { path: PathBuf },
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{}: couldn't keep a copy of the file before writing ({source}); nothing was changed",
        path.display()
    )]
    NoSafetyCopy {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("writing {} failed ({source}); the file on disk was left as it was", path.display())]
    Restored {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "writing {} failed ({source}) and putting the original back also failed ({restore}); {}",
        path.display(),
        match copy {
            Some(c) => format!("the original is saved at {}", c.display()),
            None => "the original could not be kept".to_string(),
        }
    )]
    NotRestored {
        path: PathBuf,
        #[source]
        source: std::io::Error,
        restore: std::io::Error,
        copy: Option<PathBuf>,
    },
}

/// Save `bytes` to `path`, handing the file's previous version to `backup`
/// first. Backup ▸ Backup on save copies the version about to be
/// overwritten (NppIO.cpp:1863, `CopyFileEx(fn, fn_bak)` runs ahead of the
/// save); copying afterwards would back up the new content instead.
fn save_bytes_with_backup(
    path: &Path,
    bytes: &[u8],
    backup: impl FnOnce(&Path),
) -> Result<(), SaveError> {
    if path.is_file() {
        backup(path);
    }
    write_file_safely(path, bytes)
}

/// Write `bytes` to `path`, keeping its previous version recoverable first.
///
/// The file is rewritten in place, as Notepad++ does (Buffer.cpp
/// `FileManager::saveBuffer` opens the existing file and writes over it),
/// so hard links, ACLs, ownership, extended attributes and the Windows
/// hidden attribute all survive. Writing to a temp file and renaming it
/// over the original would lose those, and on Unix would also write
/// through a read-only file's permissions.
///
/// Before the first byte changes, the original is copied next to itself
/// (`<name>.notepad-studio-save`) and synced. If that cannot be done, the
/// write is refused. If the write fails part-way (disk full, device gone, I/O error)
/// the original bytes go back and the copy is removed. Only when putting
/// them back also fails does the copy stay, and the error names it.
pub(crate) fn write_file_safely(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    write_file_safely_with(path, bytes, |file, data| {
        use std::io::Write;
        file.write_all(data)
    })
}

/// `write_file_safely` with the data write injectable, so tests can make
/// it fail part-way.
fn write_file_safely_with(
    path: &Path,
    bytes: &[u8],
    write: impl Fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
) -> Result<(), SaveError> {
    write_file_safely_with_restore(path, bytes, write, restore_copy::restore)
}

fn write_file_safely_with_restore(
    path: &Path,
    bytes: &[u8],
    write: impl Fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
    restore: impl Fn(&mut std::fs::File, &Path) -> std::io::Result<()>,
) -> Result<(), SaveError> {

    let io_err = |source: std::io::Error| SaveError::Io {
        path: path.to_path_buf(),
        source,
    };
    let meta = match std::fs::metadata(path) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err(e)),
    };
    let new_len = u64::try_from(bytes.len()).map_err(|_| {
        io_err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "document too large to save on this platform",
        ))
    })?;

    let Some(meta) = meta else {
        // New file: nothing to protect. Don't leave a partial file behind.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io_err)?;
        let result = write(&mut file, bytes).and_then(|()| file.sync_all());
        drop(file);
        return match result {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(path);
                Err(io_err(e))
            }
        };
    };
    if meta.is_dir() {
        return Err(SaveError::IsDirectory {
            path: path.to_path_buf(),
        });
    }
    if meta.permissions().readonly() {
        return Err(SaveError::ReadOnly {
            path: path.to_path_buf(),
        });
    }

    // Keep the original before touching it.
    let copy = make_safety_copy(path).map_err(|source| SaveError::NoSafetyCopy {
        path: path.to_path_buf(), source,
    })?;

    // Open without truncating so the existing file and its hard links keep
    // their identity. `set_len` trims any tail after the data write.
    let mut file = match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(f) => f,
        Err(e) => {
            safety_copy::discard(&copy);
            return Err(io_err(e));
        }
    };
    let result = write(&mut file, bytes)
        .and_then(|()| file.set_len(new_len))
        .and_then(|()| file.sync_all());
    let Err(source) = result else {
        drop(file);
        safety_copy::discard(&copy);
        return Ok(());
    };

    // Put the original back.
    let restored = restore(&mut file, &copy);
    drop(file);
    match restored {
        Ok(()) => {
            safety_copy::discard(&copy);
            Err(SaveError::Restored {
                path: path.to_path_buf(),
                source,
            })
        }
        Err(restore) => Err(SaveError::NotRestored {
            path: path.to_path_buf(),
            source,
            restore,
            copy: Some(copy),
        }),
    }
}

/// Status line for a file whose bytes didn't decode cleanly. Shared by the
/// synchronous and streaming open paths.
pub(crate) fn lossy_decode_status(name: &str, enc: Encoding) -> String {
    format!(
        "Opened {name} read-only: it has malformed {} data, shown as U+FFFD. \
         Saving would replace those bytes; clear Read-Only to edit anyway.",
        enc.label()
    )
}

/// Hook for `Backup ▸ Backup on save`. Reads the current settings snapshot,
/// resolves the custom-dir override, and writes a `.bak` next to (or under)
/// the saved file. Errors are logged to stderr only — a backup-write failure
/// must never mask the underlying save success.
fn run_save_backup(saved_path: &Path) {
    let snap = crate::settings::snapshot();
    if matches!(snap.backup_on_save_mode, crate::settings::BackupOnSaveMode::None) {
        return;
    }
    let custom = if snap.backup_use_custom_dir && !snap.backup_custom_dir.trim().is_empty() {
        Some(PathBuf::from(snap.backup_custom_dir.trim()))
    } else {
        None
    };
    if let Err(err) = crate::session::write_save_backup(
        saved_path,
        snap.backup_on_save_mode,
        custom.as_deref(),
    ) {
        eprintln!("[backup-on-save] {err}");
    }
}

fn suggested_save_name(buffer: &Buffer) -> String {
    if let Some(name) = buffer.file_path.as_ref().and_then(|path| path.file_name()) {
        return name.to_string_lossy().into_owned();
    }
    let extension = match buffer.rich_doc.as_ref().map(|payload| payload.format) {
        Some(crate::rich_doc::DocFormat::Docx | crate::rich_doc::DocFormat::Doc) => "docx",
        Some(crate::rich_doc::DocFormat::Odt) => "odt",
        Some(crate::rich_doc::DocFormat::Rtf) => "rtf",
        None => "txt",
    };
    // NppIO.cpp:2013 suggests the current name, including renamed untitled tabs.
    if let Some(label) = buffer.new_label.as_ref().filter(|label| !label.is_empty()) {
        if buffer.rich_doc.is_none() { return label.to_string(); }
        let mut name = PathBuf::from(label.as_ref()); name.set_extension(extension);
        return name.to_string_lossy().into_owned();
    }
    format!("Untitled.{extension}")
}

pub(crate) async fn show_dirty_close_dialog(name: String) -> CloseDecision {
    let result = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(APP_NAME)
        .set_description(format!(
            "{name} has unsaved changes. Save before closing?"
        ))
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
            "Save".to_string(),
            "Don't Save".to_string(),
            "Cancel".to_string(),
        ))
        .show()
        .await;
    match result {
        rfd::MessageDialogResult::Custom(ref s) if s == "Save" => CloseDecision::Save,
        rfd::MessageDialogResult::Custom(ref s) if s == "Don't Save" => CloseDecision::DontSave,
        rfd::MessageDialogResult::Yes => CloseDecision::Save,
        rfd::MessageDialogResult::No => CloseDecision::DontSave,
        _ => CloseDecision::Cancel,
    }
}

/// Drive a Save flow for `tabs[idx]` from inside an async closure: write
/// directly if the tab already has a path, otherwise pop a Save As dialog
/// to pick one. Returns `Ok(())` on a successful write, `Err` if the dialog
/// was cancelled or the write failed.
pub(crate) async fn save_tab_for_close(
    workspace: &WeakEntity<Workspace>,
    cx: &mut AsyncWindowContext,
    idx: usize,
) -> Result<(), String> {
    let snap = workspace
        .update(cx, |w, cx| {
            if w.window_closed { return Err("window closed".to_string()); }
            let id = w.tabs.get(idx).map(|tab| tab.session_id).ok_or("tab gone")?;
            crate::ui::csv_doc::actions::commit_cell_edit_for_tab(w, id, cx)?;
            let tab = w.tabs.iter().find(|tab| tab.session_id == id).ok_or("tab gone")?;
            let active_dir = tab
                .buffer
                .file_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(Path::to_path_buf);
            let starting = crate::settings::resolve_starting_dir(active_dir.as_deref())
                .or(active_dir)
                .or_else(dirs::document_dir)
                .or_else(dirs::home_dir);
            let suggested = suggested_save_name(&tab.buffer);
            Ok::<_, String>((tab.session_id, tab.instance.clone(), tab.buffer.file_path.is_some(), starting, suggested))
        })
        .map_err(|e| e.to_string())??;
    let (tab_id, tab_instance, has_path, starting, suggested) = snap;

    let new_path = if has_path {
        None
    } else {
        let Some(picked) = save_as_dialog::pick(starting, suggested).await else {
            return Err("Save As cancelled".to_string());
        };
        Some(picked)
    };

    workspace
        .update(cx, |w, cx| {
            if w.window_closed { return Err("window closed".to_string()); }
            // Restore keeps a closed tab's UUID but creates a new runtime
            // incarnation. Only the original requester may use this reply.
            let Some(now) = w.tabs.iter().position(|t| t.session_id == tab_id
                && std::sync::Arc::ptr_eq(&t.instance, &tab_instance)) else {
                return Err("tab gone".to_string());
            };
            if let Some(p) = new_path.as_deref() {
                if w.open_tab_index_for(p).is_some_and(|other| other != now) {
                    return Err(crate::workspace::already_open_message());
                }
            }
            crate::ui::csv_doc::actions::commit_cell_edit_for_tab(w, tab_id, cx)?;
            if let Some(path) = new_path.as_deref() { crate::settings::record_last_used_directory(path); }
            let recorded = new_path.clone();
            let r = w
                .with_tab_active(tab_id, |w| match new_path {
                    Some(p) => w.save_active_to_new_path(p),
                    None => w.write_active_to_disk(),
                })
                .unwrap_or_else(|| Err("tab gone".to_string()));
            if r.is_ok() {
                if let Some(p) = recorded {
                    recent::record_opened(p);
                    rebuild_menus(&mut *cx);
                }
            }
            r
        })
        .map_err(|e| e.to_string())?
}

/// Point `tab` at a freshly loaded `buffer`, dropping everything tied to
/// the text it held: carets, column selection, undo and redo, an open edit
/// group, scroll, wrap geometry, the Begin/End Select marker. The tab keeps
/// its identity (`session_id`, the autosave name).
pub(crate) fn reset_tab_with(tab: &mut Tab, buffer: Buffer) {
    let session_id = tab.session_id;
    *tab = Tab::empty();
    tab.session_id = session_id;
    tab.buffer = buffer;
}

/// Open and Reload share decoding, container parsing and lossy-input protection.
pub(crate) fn read_buffer(path: &Path) -> Result<Buffer, String> {
    let rich_doc = if crate::rich_doc::detect_format(path).is_some() {
        Some(crate::rich_doc::open(path).map_err(|err| err.to_string())?)
    } else {
        None
    };
    let (text, enc, lossy) = if let Some(payload) = rich_doc.as_ref() {
        (payload.raw_view_text.clone(), Encoding::Utf8, false)
    } else {
        let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
        let decoded = encoding::decode_bytes(&bytes);
        (decoded.text, decoded.encoding, decoded.lossy)
    };
    let (line_ending, normalized) = split_line_endings(text);
    let language = crate::syntax::language_for_path(path).or_else(|| {
        if rich_doc.is_none() {
            crate::syntax::language_for_shebang(&normalized)
        } else {
            None
        }
    });
    let mut buffer = Buffer::empty();
    buffer.rope = ropey::Rope::from_str(&normalized);
    buffer.file_path = Some(path.to_path_buf());
    buffer.line_ending = line_ending;
    buffer.encoding = enc;
    buffer.language = language;
    buffer.read_only = lossy;
    buffer.rich_doc = rich_doc;
    buffer.mark_saved();
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use gpui::VisualContext;

    fn with_saved_file(
        name: &str,
        test: impl FnOnce(&mut Workspace, &mut gpui::Window, &mut Context<Workspace>, &Path, bool),
    ) {
        let dir = TempDir::new("savepoint-handler");
        let path = dir.join(name);
        let rich = !name.ends_with(".txt");
        let buffer = if rich {
            rich_save_fixture(&path)
        } else {
            std::fs::write(&path, encoding::encode_checked("caf\u{00E9}\r\n\u{4E2D}\u{1F600}", Encoding::Utf16Le).expect("fixture encoding")).expect("text fixture");
            read_buffer(&path).expect("text buffer")
        };
        let mut cx = gpui::TestAppContext::single();
        let entity = rich_save_workspace(buffer, &mut cx);
        {
            let cx = cx.add_empty_window();
            cx.update_window_entity(&entity, |workspace, window, cx| {
                let focus = if rich { workspace.ensure_rich_doc_body_focus(cx) } else { workspace.focus_handle.clone() };
                window.focus(&focus, cx);
                if let Some(payload) = workspace.active_tab_mut().buffer.rich_doc.as_mut() {
                    let start = payload.block_start(0);
                    payload.active_block = Some(0);
                    payload.selection = Some(crate::rich_doc::TextSelection { anchor: start, cursor: start });
                }
                workspace.write_active_to_disk().expect("save initial content");
                test(workspace, window, cx, &path, rich);
            });
        }
        cx.quit();
    }

    fn insert_savepoint_text(workspace: &mut Workspace, rich: bool, text: &str, cx: &mut Context<Workspace>) {
        if rich { workspace.rich_doc_insert_text(text, cx); }
        else { workspace.replace_byte_range(0..0, text, cx); }
    }

    fn assert_savepoint_history(name: &str, redo_to_save: bool) {
        with_saved_file(name, |workspace, window, cx, path, rich| {
            if rich {
                workspace.rich_doc_insert_text("draft \u{4E2D} ", cx);
            } else {
                workspace.replace_byte_range(0..0, "draft \u{4E2D} ", cx);
            }
            assert!(workspace.buffer().dirty);
            if redo_to_save {
                workspace.write_active_to_disk().expect("save edited content");
            }
            let saved = std::fs::read(path).expect("saved bytes");
            workspace.undo(&crate::actions::edit::EditUndo, window, cx);
            if redo_to_save {
                assert!(workspace.buffer().dirty, "{name}: undo must leave saved content");
                workspace.redo(&crate::actions::edit::EditRedo, window, cx);
            }
            assert!(!workspace.buffer().dirty, "{name}: history returned to the actual successful save");
            assert_eq!(std::fs::read(path).expect("history leaves disk alone"), saved);
        });
    }

    #[test]
    fn plain_undo_returns_to_the_saved_state() { assert_savepoint_history("source.txt", false); }

    #[test]
    fn plain_redo_returns_to_the_saved_state() { assert_savepoint_history("source.txt", true); }

    #[test]
    fn rich_undo_returns_to_the_saved_state() {
        for name in ["source.docx", "source.odt", "source.rtf"] { assert_savepoint_history(name, false); }
    }

    #[test]
    fn rich_redo_returns_to_the_saved_state() {
        for name in ["source.docx", "source.odt", "source.rtf"] { assert_savepoint_history(name, true); }
    }

    #[test]
    fn branching_history_does_not_reuse_a_saved_stack_depth() {
        for name in ["branch.txt", "branch.rtf"] {
            with_saved_file(name, |w, window, cx, _, rich| {
                insert_savepoint_text(w, rich, "saved ", cx);
                w.write_active_to_disk().expect("save edited branch");
                w.undo(&crate::actions::edit::EditUndo, window, cx);
                assert!(w.buffer().dirty);
                insert_savepoint_text(w, rich, "different ", cx);
                assert!(w.buffer().dirty, "same history depth is a different state");
                w.redo(&crate::actions::edit::EditRedo, window, cx);
                assert!(w.buffer().dirty, "discarded redo cannot reach the savepoint");
                w.undo(&crate::actions::edit::EditUndo, window, cx);
                assert!(w.buffer().dirty, "original state differs from the edited save");
            });
        }
    }

    #[test]
    fn grouped_history_crosses_and_returns_to_saved_states() {
        for name in ["group.txt", "group.rtf"] {
            with_saved_file(name, |w, window, cx, _, rich| {
                if rich { w.rich_doc_paste("\u{00E9}\n\u{4E2D}", cx); }
                else {
                    w.begin_edit_group();
                    w.replace_byte_range(0..0, "\u{00E9}", cx);
                    w.replace_byte_range(0..0, "\u{4E2D}", cx);
                    w.end_edit_group(cx);
                }
                assert!(w.buffer().dirty);
                w.undo(&crate::actions::edit::EditUndo, window, cx);
                assert!(!w.buffer().dirty);
                w.redo(&crate::actions::edit::EditRedo, window, cx);
                assert!(w.buffer().dirty);
                w.write_active_to_disk().expect("save group");
                w.undo(&crate::actions::edit::EditUndo, window, cx);
                assert!(w.buffer().dirty);
                w.redo(&crate::actions::edit::EditRedo, window, cx);
                assert!(!w.buffer().dirty);
            });
        }
    }

    #[test]
    fn history_does_not_hide_unsaved_encoding_or_line_endings() {
        with_saved_file("metadata.txt", |w, window, cx, _, rich| {
            assert!(!rich);
            w.eol_unix(&crate::actions::EditEolUnix, window, cx);
            insert_savepoint_text(w, false, "draft ", cx);
            w.undo(&crate::actions::edit::EditUndo, window, cx);
            assert!(w.buffer().dirty, "LF still differs from saved CRLF");
            w.eol_windows(&crate::actions::EditEolWindows, window, cx);
            assert!(!w.buffer().dirty);
            w.encoding_utf8(&crate::actions::EncodingUtf8, window, cx);
            insert_savepoint_text(w, false, "draft ", cx);
            w.undo(&crate::actions::edit::EditUndo, window, cx);
            assert!(w.buffer().dirty, "UTF-8 still differs from saved UTF-16 BOM");
            w.encoding_utf16_le(&crate::actions::EncodingUtf16Le, window, cx);
            assert!(!w.buffer().dirty);
        });
    }

    #[test]
    fn raw_and_doc_view_history_keep_independent_unsaved_states() {
        with_saved_file("mixed.docx", |w, window, cx, _, _| {
            insert_savepoint_text(w, true, "AST draft ", cx);
            window.focus(&w.focus_handle, cx);
            w.replace_byte_range(0..0, "raw draft ", cx);
            w.undo(&crate::actions::edit::EditUndo, window, cx);
            assert!(w.buffer().dirty, "undoing raw text must retain AST edits");
            let focus = w.ensure_rich_doc_body_focus(cx);
            window.focus(&focus, cx);
            w.undo(&crate::actions::edit::EditUndo, window, cx);
            assert!(!w.buffer().dirty, "both views now match the saved state");
            window.focus(&w.focus_handle, cx);
            w.redo(&crate::actions::edit::EditRedo, window, cx);
            assert!(w.buffer().dirty, "raw redo is independent of the clean AST");
            window.focus(&focus, cx);
            w.redo(&crate::actions::edit::EditRedo, window, cx);
            w.undo(&crate::actions::edit::EditUndo, window, cx);
            assert!(w.buffer().dirty, "undoing AST must retain the raw draft");
        });
    }

    #[test]
    fn failed_save_and_save_as_keep_the_previous_marker() {
        for name in ["failed.txt", "failed.rtf"] {
            with_saved_file(name, |w, window, cx, path, rich| {
                let saved = std::fs::read(path).expect("saved bytes");
                insert_savepoint_text(w, rich, "draft ", cx);
                w.active_tab_mut().buffer.read_only = true;
                assert!(w.write_active_to_disk().is_err());
                w.active_tab_mut().buffer.read_only = false;
                assert!(w.save_active_to_new_path(path.parent().expect("fixture directory").to_owned()).is_err());
                assert_eq!(w.buffer().file_path.as_deref(), Some(path));
                assert!(w.buffer().dirty);
                w.undo(&crate::actions::edit::EditUndo, window, cx);
                assert!(!w.buffer().dirty, "failed writes did not move the savepoint");
                assert_eq!(std::fs::read(path).expect("original file"), saved);
            });
        }
    }

    #[test]
    fn stale_history_cannot_reach_a_saved_token_or_replace_unrelated_text() {
        for redo in [false, true] {
            for damage in ["utf8", "overflow", "text", "revision", "group"] {
                with_saved_file("stale.txt", |w, window, cx, _, _| {
                    let saved_revision = w.buffer().raw_revision.clone();
                    w.replace_byte_range(0..0, "d", cx);
                    let crate::buffer::UndoEntry::Single(mut op) = w.active_tab_mut().undo_stack.pop().expect("recorded edit") else { panic!("single") };
                    if redo {
                        std::mem::swap(&mut op.before_revision, &mut op.after_revision);
                        std::mem::swap(&mut op.removed, &mut op.inserted);
                    }
                    match damage {
                        "utf8" => { op.byte_start = 5; op.removed = "x".into(); op.inserted = "x".into(); }
                        "overflow" => { op.byte_start = usize::MAX; }
                        "text" => { op.removed = "x".into(); op.inserted = "x".into(); }
                        "revision" => {
                            if redo { op.before_revision = crate::buffer::ContentRevision::default(); }
                            else { op.after_revision = crate::buffer::ContentRevision::default(); }
                        }
                        _ => {}
                    }
                    if redo { op.after_revision = saved_revision; } else { op.before_revision = saved_revision; }
                    let entry = if damage == "group" {
                        let mut bad = op.clone();
                        bad.byte_start = usize::MAX;
                        // A later failing op must refuse the whole group before the valid one runs.
                        let ops = if redo { vec![op, bad] } else { vec![bad, op] };
                        crate::buffer::UndoEntry::Group { ops, sel_before_anchor: 0, sel_before_head: 0 }
                    } else { crate::buffer::UndoEntry::Single(op) };
                    if redo { w.active_tab_mut().redo_stack.push(entry); }
                    else { w.active_tab_mut().undo_stack.push(entry); }
                    let original = w.buffer().rope.to_string();
                    if redo { w.redo(&crate::actions::edit::EditRedo, window, cx); }
                    else { w.undo(&crate::actions::edit::EditUndo, window, cx); }
                    assert!(w.buffer().dirty, "{redo}/{damage}: malformed replay cannot restore a saved identity");
                    assert_eq!(w.buffer().rope.to_string(), original, "{redo}/{damage}: refuse before changing unrelated text");
                    assert!(w.status_message.as_ref().is_some_and(|message| message.contains("history")));
                });
            }
        }
    }

    fn rich_save_fixture(path: &Path) -> Buffer {
        use std::io::Write;
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("docx") => {
                let xml = "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\r\n<w:body><w:p><w:r><w:t>café</w:t></w:r></w:p></w:body></w:document>";
                let mut zip = zip::ZipWriter::new(std::fs::File::create(path).expect("DOCX fixture"));
                for (name, contents) in [
                    ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
                    ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
                    ("word/document.xml", xml),
                ] {
                    zip.start_file(name, zip::write::SimpleFileOptions::default()).expect("DOCX entry");
                    zip.write_all(contents.as_bytes()).expect("DOCX XML");
                }
                zip.finish().expect("finish DOCX");
            }
            Some("odt") => {
                let xml = "<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\r\n<office:body><office:text><text:p>café</text:p></office:text></office:body></office:document-content>";
                let mut zip = zip::ZipWriter::new(std::fs::File::create(path).expect("ODT fixture"));
                zip.start_file("content.xml", zip::write::SimpleFileOptions::default()).expect("content entry");
                zip.write_all(xml.as_bytes()).expect("content XML");
                zip.finish().expect("finish ODT");
            }
            Some("rtf") => std::fs::write(path, br"{\rtf1\ansi caf\'e9}").expect("RTF fixture"),
            _ => panic!("unsupported fixture format"),
        }
        read_buffer(path).expect("open native fixture")
    }

    fn rich_save_workspace(buffer: Buffer, cx: &mut gpui::TestAppContext) -> gpui::Entity<Workspace> {
        use gpui::AppContext;
        cx.new(move |cx| {
            let mut workspace = Workspace::new(
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                1., gpui::WindowAppearance::Dark,
            );
            let mut tab = Tab::empty();
            tab.buffer = buffer;
            workspace.tabs = vec![tab];
            workspace.active = 0;
            workspace.window_closed = true;
            workspace
        })
    }

    fn edit_rich_source(buffer: &mut Buffer, edit_doc_view: bool) {
        if edit_doc_view {
            buffer.rich_doc.as_mut().expect("native payload").apply_edit(
                crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: true });
        }
        buffer.rope.insert(0, "unsaved raw 中 ");
        buffer.dirty = true;
    }

    #[test]
    fn native_save_refuses_source_edits_it_cannot_preserve() {
        let dir = TempDir::new("rich-raw-save");
        for name in ["source.docx", "source.odt", "source.rtf"] {
            let path = dir.join(name);
            let mut buffer = rich_save_fixture(&path);
            let original = std::fs::read(&path).expect("original bytes");
            edit_rich_source(&mut buffer, name.ends_with("rtf"));
            let raw = buffer.rope.to_string();
            let result = crate::rich_doc::save(buffer.rich_doc.as_ref().expect("payload"), &path, Some(&raw));
            assert!(result.is_err(), "{name}: Save silently discarded edited raw source");
            assert_eq!(std::fs::read(&path).expect("untouched source"), original, "{name}");
        }
    }

    #[test]
    fn refused_native_save_and_save_as_keep_the_dirty_source_and_path() {
        let dir = TempDir::new("rich-raw-save-handler");
        for name in ["source.docx", "source.odt", "source.rtf"] {
            let path = dir.join(name);
            let target = dir.join(&format!("copy-{name}"));
            let mut buffer = rich_save_fixture(&path);
            let original = std::fs::read(&path).expect("original bytes");
            edit_rich_source(&mut buffer, name.ends_with("rtf"));
            let raw = buffer.rope.to_string();
            let mut cx = gpui::TestAppContext::single();
            let entity = rich_save_workspace(buffer, &mut cx);
            entity.update(&mut cx, |workspace, _| {
                assert!(workspace.write_active_to_disk().is_err(), "{name}: native Save must fail");
                assert!(workspace.buffer().dirty);
                assert_eq!(workspace.buffer().rope.to_string(), raw);
                assert!(workspace.save_active_to_new_path(target.clone()).is_err(), "{name}: native Save As must fail");
                assert!(workspace.buffer().dirty);
                assert_eq!(workspace.buffer().file_path.as_deref(), Some(path.as_path()));
                assert_eq!(workspace.buffer().rope.to_string(), raw);
                assert!(!target.exists());
            });
            cx.quit();
            assert_eq!(std::fs::read(&path).expect("untouched source"), original, "{name}");
        }
    }

    #[test]
    fn native_doc_view_save_accepts_normalized_source_without_discarding_edits() {
        let dir = TempDir::new("rich-doc-view-save");
        for name in ["source.docx", "source.odt", "source.rtf"] {
            let path = dir.join(name);
            let target = dir.join(&format!("saved-{name}"));
            let mut buffer = rich_save_fixture(&path);
            let original = std::fs::read(&path).expect("original bytes");
            let payload = buffer.rich_doc.as_mut().expect("native payload");
            payload.apply_edit(crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: true });
            let expected = payload.block_text(0);
            buffer.dirty = true;
            let mut cx = gpui::TestAppContext::single();
            let entity = rich_save_workspace(buffer, &mut cx);
            entity.update(&mut cx, |workspace, _| {
                workspace.save_active_to_new_path(target.clone()).expect("save Doc View edits");
                assert!(!workspace.buffer().dirty);
                assert_eq!(workspace.buffer().file_path.as_deref(), Some(target.as_path()));
            });
            cx.quit();
            let reopened = crate::rich_doc::open(&target).expect("reopen saved native file");
            assert_eq!(reopened.block_text(0), expected);
            assert!(reopened.block_is_bold(0));
            assert_eq!(std::fs::read(&path).expect("original source"), original);
        }
    }

    #[test]
    fn native_rtf_save_preserves_source_only_edits() {
        let dir = TempDir::new("rich-rtf-source-save");
        let source = dir.join("source.rtf");
        let target = dir.join("edited.rtf");
        let mut buffer = rich_save_fixture(&source);
        let raw = r"{\rtf1\ansi edited caf\'e9 \u20013?}";
        buffer.rope = ropey::Rope::from_str(raw);
        buffer.dirty = true;
        let mut cx = gpui::TestAppContext::single();
        let entity = rich_save_workspace(buffer, &mut cx);
        entity.update(&mut cx, |workspace, _| {
            workspace.save_active_to_new_path(target.clone()).expect("save RTF source");
            assert!(!workspace.buffer().dirty);
        });
        cx.quit();
        assert_eq!(std::fs::read_to_string(&target).expect("saved source"), raw);
        assert!(crate::rich_doc::open(&target).expect("reopen RTF").block_text(0).contains("中"));
    }

    #[test]
    fn recovered_native_documents_keep_their_format_in_save_as_names() {
        let mut buffer = crate::buffer::Buffer::empty();
        assert_eq!(super::suggested_save_name(&buffer), "Untitled.txt");
        let payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi test}".to_vec()).expect("rtf");
        buffer.rich_doc = Some(payload);
        for (format, name) in [(crate::rich_doc::DocFormat::Rtf, "Untitled.rtf"), (crate::rich_doc::DocFormat::Docx, "Untitled.docx"), (crate::rich_doc::DocFormat::Odt, "Untitled.odt")] {
            buffer.rich_doc.as_mut().expect("payload").format = format;
            assert_eq!(super::suggested_save_name(&buffer), name);
        }
        buffer.file_path = Some(std::path::PathBuf::from("source.odt"));
        assert_eq!(super::suggested_save_name(&buffer), "source.odt");
    }

    #[test]
    fn save_names_follow_untitled_labels_and_preserve_native_extensions() {
        let mut buffer = crate::buffer::Buffer::empty();
        for label in ["new 1", "named café中😀", "custom.txt"] {
            buffer.new_label = Some(label.to_owned().into());
            assert_eq!(super::suggested_save_name(&buffer), label);
        }
        buffer.rich_doc = Some(crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi test}".to_vec()).expect("rtf"));
        for (format, extension) in [(crate::rich_doc::DocFormat::Rtf, "rtf"), (crate::rich_doc::DocFormat::Docx, "docx"),
            (crate::rich_doc::DocFormat::Doc, "docx"), (crate::rich_doc::DocFormat::Odt, "odt")] {
            buffer.rich_doc.as_mut().expect("payload").format = format;
            assert_eq!(super::suggested_save_name(&buffer), format!("custom.{extension}"));
            buffer.new_label = Some("named café中😀".into());
            assert_eq!(super::suggested_save_name(&buffer), format!("named café中😀.{extension}"));
            buffer.new_label = Some("custom.txt".into());
        }
        buffer.file_path = Some("source.original.rtf".into());
        assert_eq!(super::suggested_save_name(&buffer), "source.original.rtf");
    }

    use super::*;
    use crate::persist::TempDir;

    #[test]
    fn reusing_a_tab_drops_state_tied_to_its_old_text() {
        let mut tab = Tab::empty();
        let id = tab.session_id;
        tab.selection = crate::buffer::Selection { anchor: 3, head: 9, is_dragging: false };
        tab.block_selection = Some(crate::buffer::BlockSelection {
            anchor_line: 4,
            head_line: 7,
            ..Default::default()
        });
        tab.begin_select_marker = Some(40);
        tab.undo_stack.push(crate::buffer::UndoEntry::Group {
            ops: Vec::new(),
            sel_before_anchor: 3,
            sel_before_head: 9,
        });
        let mut loaded = Buffer::empty();
        loaded.rope = ropey::Rope::from_str("ab");
        reset_tab_with(&mut tab, loaded);
        assert_eq!(tab.session_id, id);
        assert_eq!(tab.buffer.rope.len_bytes(), 2);
        assert_eq!((tab.selection.anchor, tab.selection.head), (0, 0));
        assert!(tab.begin_select_marker.is_none());
        assert!(tab.undo_stack.is_empty());
        assert!(tab.wrap_cache.is_none());
        assert!(tab.block_selection.is_none());
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.contains(".notepad-studio-save"))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn writes_a_new_file() {
        let dir = TempDir::new("save-new");
        let p = dir.join("new.txt");
        assert!(write_file_safely(&p, b"hello").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"hello".to_vec()));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn overwrites_shorter_and_longer_without_leftovers() {
        let dir = TempDir::new("save-overwrite");
        let p = dir.join("f.txt");
        let _ = std::fs::write(&p, b"0123456789");
        assert!(write_file_safely(&p, b"abc").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"abc".to_vec()));
        assert!(write_file_safely(&p, b"a much longer body").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"a much longer body".to_vec()));
        assert!(write_file_safely(&p, b"").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(Vec::new()));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn failed_write_puts_the_original_back() {
        // Regression: `fs::write` truncated the file before writing, so an
        // error part-way through (disk full, device unplugged) left the
        // user's file empty or half-written.
        let dir = TempDir::new("save-fail");
        let p = dir.join("f.txt");
        let original = b"original contents\r\nline two\r\n".repeat(100);
        let _ = std::fs::write(&p, &original);
        let r = write_file_safely_with(&p, &b"new".repeat(5000), |file, data| {
            use std::io::Write;
            let half = data.get(..data.len() / 2).unwrap_or(data);
            file.write_all(half)?;
            Err(std::io::Error::other("disk full"))
        });
        assert!(matches!(r, Err(SaveError::Restored { .. })), "{r:?}");
        assert_eq!(std::fs::read(&p).ok(), Some(original));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn backup_on_save_sees_the_previous_version() {
        // Regression: the backup ran after the write, so `.bak` held the
        // new content and the previous version was lost.
        let dir = TempDir::new("save-backup");
        let p = dir.join("f.txt");
        let _ = std::fs::write(&p, b"version 1");
        let mut seen = None;
        let r = save_bytes_with_backup(&p, b"version 2", |path| {
            seen = std::fs::read(path).ok();
        });
        assert!(r.is_ok());
        assert_eq!(seen, Some(b"version 1".to_vec()));
        assert_eq!(std::fs::read(&p).ok(), Some(b"version 2".to_vec()));
        // A new file has no previous version to back up.
        let mut called = false;
        let r = save_bytes_with_backup(&dir.join("new.txt"), b"x", |_| called = true);
        assert!(r.is_ok());
        assert!(!called);
    }

    #[test]
    fn failed_new_file_is_not_left_behind() {
        let dir = TempDir::new("save-fail-new");
        let p = dir.join("new.txt");
        let r = write_file_safely_with(&p, b"data", |_, _| Err(std::io::Error::other("boom")));
        assert!(matches!(r, Err(SaveError::Io { .. })), "{r:?}");
        assert!(!p.exists());
    }

    #[test]
    fn read_only_file_is_refused_and_untouched() {
        let dir = TempDir::new("save-ro");
        let p = dir.join("ro.txt");
        let _ = std::fs::write(&p, b"keep me");
        let Ok(meta) = std::fs::metadata(&p) else {
            panic!("metadata");
        };
        let mut perms = meta.permissions();
        perms.set_readonly(true);
        let _ = std::fs::set_permissions(&p, perms.clone());
        let r = write_file_safely(&p, b"overwrite");
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(&p, perms);
        assert!(matches!(r, Err(SaveError::ReadOnly { .. })), "{r:?}");
        assert_eq!(std::fs::read(&p).ok(), Some(b"keep me".to_vec()));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn directory_and_vanished_parent_are_errors() {
        let dir = TempDir::new("save-dir");
        let sub = dir.join("sub");
        let _ = std::fs::create_dir(&sub);
        assert!(matches!(
            write_file_safely(&sub, b"x"),
            Err(SaveError::IsDirectory { .. })
        ));
        let gone = dir.join("gone").join("f.txt");
        let r = write_file_safely(&gone, b"x");
        assert!(matches!(r, Err(SaveError::Io { .. })), "{r:?}");
        assert!(!r.err().map(|e| e.to_string()).unwrap_or_default().is_empty());
    }

    #[test]
    fn an_existing_safety_copy_is_never_overwritten() {
        // A copy left by an earlier save that could not restore may be the
        // only good version of the file.
        let dir = TempDir::new("save-keep");
        let p = dir.join("f.txt");
        let stale = dir.join("f.txt.notepad-studio-save");
        let _ = std::fs::write(&p, b"current");
        let _ = std::fs::write(&stale, b"older rescue copy");
        let r = write_file_safely_with(&p, b"next", |_, _| Err(std::io::Error::other("boom")));
        assert!(matches!(r, Err(SaveError::Restored { .. })), "{r:?}");
        assert_eq!(std::fs::read(&stale).ok(), Some(b"older rescue copy".to_vec()));
        assert_eq!(std::fs::read(&p).ok(), Some(b"current".to_vec()));
        assert!(write_file_safely(&p, b"next").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"next".to_vec()));
        assert_eq!(leftovers(dir.path()), vec!["f.txt.notepad-studio-save".to_string()]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn hidden_file_on_windows_saves() {
        // Windows refuses CREATE_ALWAYS on a file with the hidden
        // attribute; the in-place open must not trip over it.
        let dir = TempDir::new("save-hidden");
        let p = dir.join("hidden.txt");
        let _ = std::fs::write(&p, b"old");
        let status = std::process::Command::new("attrib")
            .arg("+h")
            .arg(&p)
            .status();
        assert!(matches!(status, Ok(s) if s.success()));
        assert!(write_file_safely(&p, b"new").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"new".to_vec()));
        let _ = std::process::Command::new("attrib").arg("-h").arg(&p).status();
    }
}

#[cfg(test)]
#[path = "ui/csv_doc/pending_tests.rs"]
mod csv_pending_tests;

#[cfg(test)]
#[path = "ui/csv_doc/recovery_tests.rs"]
mod csv_recovery_tests;
