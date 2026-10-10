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

use crate::actions::{
    FileNew, FileOpen, FileOpenFolderAsWorkspace, FileSave, FileSaveAll, FileSaveAs,
};
use crate::app::menus::rebuild_menus;
use crate::buffer::{Buffer, Selection};
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
        let tab = &self.tabs[self.active];
        tab.buffer.rope.len_bytes() == 0
            && tab.buffer.file_path.is_none()
            && !tab.buffer.dirty
    }

    /// Replace the active tab's buffer with `buffer` (used after Open lands a
    /// file in a pristine empty tab). Resets selection and undo history so
    /// the load is a clean slate.
    fn replace_active_buffer(&mut self, buffer: Buffer) {
        let tab = self.active_tab_mut();
        tab.buffer = buffer;
        tab.selection = Selection::default();
        tab.desired_col_x = None;
        tab.undo_stack.clear();
        tab.redo_stack.clear();
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
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() && meta.len() > crate::io_stream::STREAM_THRESHOLD_BYTES {
                crate::io_stream::open_path_streaming(self, path, meta.len(), cx);
                cx.notify();
                return;
            }
        }

        // Rich-document formats (DOCX, …) parse through their own path before
        // the generic byte-decode below — the file is a zip/binary container,
        // not raw text, so we extract the inner document.xml for Raw View and
        // build the AST that drives Doc View.
        let rich_doc_payload = if crate::rich_doc::detect_format(&path).is_some() {
            match crate::rich_doc::open(&path) {
                Ok(p) => Some(p),
                Err(err) => {
                    self.set_status(format!("Failed to open {display}: {err}"));
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };

        let mut lossy_decode = false;
        let (rope, line_ending, enc, language) = if let Some(p) = rich_doc_payload.as_ref() {
            // Raw View shows the extracted XML — already UTF-8, no BOM sniff needed.
            let (le, normalized) = split_line_endings(p.raw_view_text.clone());
            (
                ropey::Rope::from_str(&normalized),
                le,
                Encoding::Utf8,
                crate::syntax::language_for_path(&path),
            )
        } else {
            let raw_bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(err) => {
                    self.set_status(format!("Failed to open {display}: {err}"));
                    cx.notify();
                    return;
                }
            };
            // Sniff BOM and decode into a UTF-8 String, recording which encoding
            // the bytes came from so subsequent saves round-trip correctly.
            let decoded = encoding::decode_bytes(&raw_bytes);
            drop(raw_bytes);
            lossy_decode = decoded.lossy;
            let enc = decoded.encoding;
            let (le, normalized) = split_line_endings(decoded.text);
            let language = crate::syntax::language_for_path(&path)
                .or_else(|| crate::syntax::language_for_shebang(&normalized));
            (ropey::Rope::from_str(&normalized), le, enc, language)
        };

        let is_rich_doc = rich_doc_payload.is_some();
        let mut buffer = Buffer {
            rope,
            file_path: Some(path.clone()),
            dirty: false,
            line_ending,
            encoding: enc,
            bookmarks: std::collections::BTreeSet::new(),
            marks: Vec::new(),
            language,
            highlights: Vec::new(),
            loading: None,
            new_label: None,
            // A lossy decode can't be saved back without rewriting the bad
            // bytes; open read-only so that takes a deliberate step.
            read_only: lossy_decode,
            tab_color: None,
            is_changelog: false,
            rich_doc: rich_doc_payload,
            flat_text_cache: None,
        };
        buffer.reparse_highlights();

        let basename = buffer.display_short_name().to_string();
        let is_csv = crate::ui::csv_doc::is_csv_path(Some(&path));
        if self.active_is_pristine_empty() {
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
        self.save_session();
        // Push to MRU and rebuild the File ▸ Open Recent submenu. Order
        // matters: `record_opened` writes the new TOML before the menu
        // rebuild reads the global snapshot, so the new entry is the first
        // item in the submenu by the time the OS picks up the new menu tree.
        recent::record_opened(path);
        rebuild_menus(&mut *cx);
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

        let buffer = Buffer {
            rope,
            file_path: Some(path),
            dirty: false,
            line_ending,
            encoding: enc,
            bookmarks: std::collections::BTreeSet::new(),
            marks: Vec::new(),
            language: None,
            highlights: Vec::new(),
            loading: None,
            new_label: None,
            read_only: true,
            tab_color: None,
            is_changelog: true,
            rich_doc: None,
            flat_text_cache: None,
        };

        // Replace an existing changelog tab in place — happens on a
        // version bump where the prior version's changelog is still in
        // the restored session.
        if let Some(idx) = self.tabs.iter().position(|t| t.buffer.is_changelog) {
            let tab = &mut self.tabs[idx];
            tab.buffer = buffer;
            tab.selection = Selection::default();
            tab.desired_col_x = None;
            tab.undo_stack.clear();
            tab.redo_stack.clear();
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
    pub(crate) fn write_active_to_disk(&mut self) -> Result<(), String> {
        let active = self.active;
        let tab = &mut self.tabs[active];
        let Some(path) = tab.buffer.file_path.clone() else {
            return Err("no path".to_string());
        };
        // Rich-doc buffers go through their native save path (e.g. zip
        // container preservation for DOCX). Falling through to the rope
        // serializer would write `word/document.xml` (the raw-view text)
        // back to disk as the entire file and destroy the container.
        if let Some(payload) = tab.buffer.rich_doc.as_ref() {
            // For RTF, the rope IS the editable raw view — pass it through so
            // a save with no toolbar edits but with raw-text edits writes the
            // user's typed source instead of the original file bytes. DOCX
            // ignores this since raw XML editing inside a zip is unsafe in v1.
            let raw_override: Option<String> =
                if matches!(payload.format, crate::rich_doc::DocFormat::Rtf) && tab.buffer.dirty {
                    Some(tab.buffer.rope.to_string())
                } else {
                    None
                };
            match crate::rich_doc::save(payload, &path, raw_override.as_deref()) {
                Ok(()) => {
                    tab.buffer.dirty = false;
                    run_save_backup(&path);
                    return Ok(());
                }
                Err(err) => return Err(format!("rich-doc save failed: {err}")),
            }
        }
        // Two-stage serialization: rope (LF-internal UTF-8) → EOL-applied
        // UTF-8 bytes → encoded bytes per the buffer's chosen encoding (with
        // BOM where applicable).
        let eol_bytes = rope_to_bytes(&tab.buffer.rope, tab.buffer.line_ending);
        let utf8 = match std::str::from_utf8(&eol_bytes) {
            Ok(s) => s,
            Err(err) => return Err(format!("internal: rope was not valid UTF-8: {err}")),
        };
        let bytes = encoding::encode(utf8, tab.buffer.encoding);
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                tab.buffer.dirty = false;
                run_save_backup(&path);
                Ok(())
            }
            Err(err) => Err(err.to_string()),
        }
    }

    pub(crate) fn save_active(&mut self, _: &FileSave, window: &mut Window, cx: &mut Context<Self>) {
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
        self.save_session();
        cx.notify();
    }

    pub(crate) fn save_active_as(&mut self, _: &FileSaveAs, window: &mut Window, cx: &mut Context<Self>) {
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

        let suggested = self
            .buffer()
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.txt".to_string());

        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Save As")
                .set_file_name(&suggested);
            if let Some(dir) = starting {
                dialog = dialog.set_directory(dir);
            }
            let Some(handle) = dialog.save_file().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            crate::settings::record_last_used_directory(&path);
            let refused = this
                .update(cx, |w, cx| {
                    let active = w.active;
                    w.save_target_open_elsewhere(&path, active, cx)
                })
                .unwrap_or(false);
            if refused {
                crate::workspace::show_already_open_error().await;
                return;
            }
            this.update(cx, |w, cx| {
                let buf = &mut w.active_tab_mut().buffer;
                buf.file_path = Some(path.clone());
                buf.new_label = None;
                let saved = match w.write_active_to_disk() {
                    Ok(()) => {
                        let name = w.buffer().display_short_name().to_string();
                        w.set_status(format!("Saved {name}"));
                        true
                    }
                    Err(err) => {
                        w.set_status(format!("Save failed: {err}"));
                        false
                    }
                };
                w.save_session();
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

    /// Notepad++ `Ctrl+Shift+S` (Linux/Win), `Cmd+Opt+S` on macOS to leave
    /// `Cmd+Shift+S` free for the standard Mac Save As. Skips tabs without a
    /// path silently — the user invokes Save As individually for those.
    ///
    /// When `Settings.save_all_confirm` is true (NPP `MISC. ▸ Enable Save
    /// All confirm dialog`), an OS-native Yes/No prompt fires first; the
    /// save loop only runs on Yes. The dirty-count is computed up-front so
    /// the prompt body can name a count.
    pub(crate) fn save_all(&mut self, _: &FileSaveAll, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = crate::settings::snapshot();
        if !prefs.save_all_confirm {
            self.run_save_all(cx);
            return;
        }

        let dirty_with_path = self
            .tabs
            .iter()
            .filter(|t| t.buffer.file_path.is_some() && t.buffer.dirty)
            .count();
        if dirty_with_path == 0 {
            // Nothing to save — skip the prompt entirely; matches NPP.
            self.run_save_all(cx);
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            let confirm = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title(APP_NAME)
                .set_description(format!(
                    "Save {dirty_with_path} unsaved file(s)?"
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            if !matches!(confirm, rfd::MessageDialogResult::Yes) {
                let _ = this.update(cx, |w, cx| {
                    w.set_status("Save All cancelled".to_string());
                    cx.notify();
                });
                return;
            }
            let _ = this.update(cx, |w, cx| {
                w.run_save_all(cx);
            });
        })
        .detach();
    }

    /// The actual save loop — extracted so both the immediate path and the
    /// post-confirm async path call the same code.
    fn run_save_all(&mut self, cx: &mut Context<Self>) {
        let mut saved = 0usize;
        let mut failures: Vec<String> = Vec::new();
        let prev_active = self.active;
        for idx in 0..self.tabs.len() {
            if self.tabs[idx].buffer.file_path.is_none() {
                continue;
            }
            self.active = idx;
            match self.write_active_to_disk() {
                Ok(()) => saved += 1,
                Err(err) => {
                    let name = self.tabs[idx].buffer.display_short_name().to_string();
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
        self.save_session();
        cx.notify();
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
        .update(cx, |w, _| {
            let tab = w.tabs.get(idx)?;
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
            let suggested = tab
                .buffer
                .file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled.txt".to_string());
            Some((tab.buffer.file_path.is_some(), starting, suggested))
        })
        .map_err(|e| e.to_string())?;
    let Some((has_path, starting, suggested)) = snap else {
        return Err("tab gone".to_string());
    };

    let new_path = if has_path {
        None
    } else {
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title("Save As")
            .set_file_name(&suggested);
        if let Some(dir) = starting {
            dialog = dialog.set_directory(dir);
        }
        let Some(handle) = dialog.save_file().await else {
            return Err("Save As cancelled".to_string());
        };
        let picked = handle.path().to_path_buf();
        crate::settings::record_last_used_directory(&picked);
        Some(picked)
    };

    workspace
        .update(cx, |w, cx| {
            if let Some(p) = new_path.as_deref() {
                if w.open_tab_index_for(p).is_some_and(|other| other != idx) {
                    return Err(crate::workspace::already_open_message());
                }
            }
            let prev = w.active;
            w.active = idx;
            let recorded = if let Some(p) = new_path {
                let buf = &mut w.active_tab_mut().buffer;
                buf.file_path = Some(p.clone());
                buf.new_label = None;
                Some(p)
            } else {
                None
            };
            let r = w.write_active_to_disk();
            w.active = prev;
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
