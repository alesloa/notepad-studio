// Find-in-Files handlers on Workspace — open / close / run / pick-dir /
// toggle, plus the click-to-jump path that opens the result's file at the
// right line and selects the match.
//
// The actual filesystem walk lives in `crate::find_in_files` (the engine
// module). This file orchestrates: gather inputs, dispatch the engine on
// `cx.background_executor`, and apply results back on the main thread.

use std::path::PathBuf;

use gpui::{AppContext, Context, Window};

use crate::actions::{
    FindInFilesClose, FindInFilesPickDir, FindInFilesReplace, FindInFilesRun, FindInFilesSwap,
    FindInFilesToggleCase, FindInFilesToggleExtended, FindInFilesToggleFollow,
    FindInFilesToggleHidden, FindInFilesToggleRegex, FindInFilesToggleSubfolders,
    FindInFilesToggleTransparency, FindInFilesToggleWord, FindInFilesTransparencyAlways,
    FindInFilesTransparencyDec, FindInFilesTransparencyInc, FindInFilesTransparencyOnFocusLoss,
    FindInProjectsReplace, FindInProjectsRun, FindInProjectsSwap, FindInProjectsTogglePanel1,
    FindInProjectsTogglePanel2, FindInProjectsTogglePanel3, SearchFindInFiles,
    SearchFindInProjects,
};
use crate::find_in_files::{FindInFilesOutcome, FindInFilesQuery, build_pattern, run_search};
use crate::find_state::DialogTab;
use crate::ui::find_dialog::open_or_focus_find_dialog;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn open_find_in_files(
        &mut self,
        _: &SearchFindInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Default the search dir to the active buffer's parent if known
        // (or follow-current-doc demands it), else home. Persisted across
        // dialog close/reopen.
        let needs_dir = self.find_in_files.directory.is_none()
            || self.find_in_files.follow_current_doc;
        if needs_dir {
            let starting = self
                .buffer()
                .file_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(PathBuf::from)
                .or_else(dirs::home_dir);
            if let Some(dir) = starting {
                self.find_in_files.directory_text = dir.display().to_string();
                self.find_in_files.directory_text_cursor =
                    self.find_in_files.directory_text.len();
                self.find_in_files.directory = Some(dir);
            }
        }

        // Pre-fill the query from a single-line selection — same UX as the
        // other tabs.
        if self.find_in_files.query.is_empty() && !self.selection().is_empty() {
            let r = self.selection().ordered();
            let text = self.buffer().slice_bytes(r);
            if !text.is_empty() && !text.contains('\n') && text.len() < 256 {
                self.find_in_files.query = text;
                self.find_in_files.query_cursor = self.find_in_files.query.len();
            }
        }

        open_or_focus_find_dialog(self, DialogTab::FindInFiles, window, cx);
    }

    /// Close just the inline results panel — the dialog has its own close
    /// path. Lets the `×` chip on the panel keep working.
    pub(crate) fn close_find_in_files(
        &mut self,
        _: &FindInFilesClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find_in_files.open {
            return;
        }
        self.find_in_files.open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(crate) fn find_in_files_run_action(
        &mut self,
        _: &FindInFilesRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Promote the typed directory text into the validated `directory`
        // field before kicking off the walk.
        self.commit_directory_text();
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifQuery,
            &self.find_in_files.query,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifFilter,
            &self.find_in_files.filter,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifDirectory,
            &self.find_in_files.directory_text,
        );
        // Open the inline results panel so the user can click results.
        self.find_in_files.open = true;
        self.run_find_in_files(window, cx);
    }

    /// Sync `directory_text` (what the user typed) into `directory` (the
    /// validated PathBuf). No-op when the field matches the cached path.
    fn commit_directory_text(&mut self) {
        let trimmed = self.find_in_files.directory_text.trim();
        if trimmed.is_empty() {
            return;
        }
        let path = PathBuf::from(trimmed);
        let same = self
            .find_in_files
            .directory
            .as_ref()
            .map(|p| p == &path)
            .unwrap_or(false);
        if !same {
            self.find_in_files.directory = Some(path);
        }
    }

    /// Kick off the search on the background executor. UI thread stays
    /// responsive — when the walk finishes, the result is applied back via
    /// `update`.
    pub(crate) fn run_find_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_in_files.searching {
            return;
        }
        if self.find_in_files.query.trim().is_empty() {
            self.find_in_files.error = Some("Enter a search query".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        }
        let Some(dir) = self.find_in_files.directory.clone() else {
            self.find_in_files.error = Some("Pick a directory first".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        };

        let query = FindInFilesQuery {
            text: self.find_in_files.query.clone(),
            directory: dir,
            filter: self.find_in_files.filter.clone(),
            match_case: self.find_in_files.match_case,
            whole_word: self.find_in_files.whole_word,
            regex: self.find_in_files.regex,
            extended: self.find_in_files.extended,
            dot_matches_newline: self.find_in_files.dot_matches_newline,
            in_subfolders: self.find_in_files.in_subfolders,
            in_hidden_folders: self.find_in_files.in_hidden_folders,
        };

        self.find_in_files.searching = true;
        self.find_in_files.error = None;
        self.find_in_files.status = Some("Searching…".to_string());
        self.find_in_files.results.clear();
        self.find_in_files.truncated = false;
        self.find_in_files.last_search_query = self.find_in_files.query.clone();
        self.find_in_files.last_search_mode = describe_search_mode(
            self.find_in_files.regex,
            self.find_in_files.extended,
        );
        self.find_in_files.last_search_files_searched = 0;
        cx.notify();

        let bg_task = cx
            .background_executor()
            .spawn(async move { run_search(&query) });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = bg_task.await;
            this.update(cx, |w, cx| {
                w.find_in_files.searching = false;
                if !w.find_in_files.open {
                    return;
                }
                match outcome {
                    FindInFilesOutcome::Ok {
                        results,
                        searched_files,
                        truncated,
                        elapsed_ms,
                    } => {
                        let result_count = results.len();
                        w.find_in_files.results = results;
                        w.find_in_files.truncated = truncated;
                        w.find_in_files.error = None;
                        w.find_in_files.last_search_files_searched = searched_files;
                        w.find_in_files.status = Some(format!(
                            "{result_count} match{} in {searched_files} file{} · {elapsed_ms}ms{}",
                            if result_count == 1 { "" } else { "es" },
                            if searched_files == 1 { "" } else { "s" },
                            if truncated { " (truncated)" } else { "" },
                        ));
                    }
                    FindInFilesOutcome::InvalidRegex(msg) => {
                        w.find_in_files.error = Some(msg);
                        w.find_in_files.status = None;
                    }
                    FindInFilesOutcome::DirectoryMissing => {
                        w.find_in_files.error = Some("Directory not found".to_string());
                        w.find_in_files.status = None;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Replace in Files" — runs a fresh search, then rewrites every matched
    /// file on disk with the replacement applied. Walk + write happens on the
    /// background executor; results are surfaced as a status string.
    ///
    /// Confirms the operation through an OS-native message dialog before
    /// touching any file, since this is destructive and the matches live on
    /// disk (not in editor buffers, so undo isn't available). The dialog
    /// shows the query, the replacement, and the directory so the user can
    /// double-check before committing.
    ///
    /// Files that fail to rewrite (permission denied, I/O error) are
    /// reported in the status without rolling back successful writes —
    /// Notepad++'s behaviour. The user gets a count and a list of failures.
    pub(crate) fn find_in_files_replace(
        &mut self,
        _: &FindInFilesReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_directory_text();
        if self.find_in_files.query.trim().is_empty() {
            self.find_in_files.error = Some("Enter a search query".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        }
        let Some(dir) = self.find_in_files.directory.clone() else {
            self.find_in_files.error = Some("Pick a directory first".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        };
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifQuery,
            &self.find_in_files.query,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifReplacement,
            &self.find_in_files.replacement,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifFilter,
            &self.find_in_files.filter,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifDirectory,
            &self.find_in_files.directory_text,
        );
        let query = FindInFilesQuery {
            text: self.find_in_files.query.clone(),
            directory: dir.clone(),
            filter: self.find_in_files.filter.clone(),
            match_case: self.find_in_files.match_case,
            whole_word: self.find_in_files.whole_word,
            regex: self.find_in_files.regex,
            extended: self.find_in_files.extended,
            dot_matches_newline: self.find_in_files.dot_matches_newline,
            in_subfolders: self.find_in_files.in_subfolders,
            in_hidden_folders: self.find_in_files.in_hidden_folders,
        };
        let replacement = self.find_in_files.replacement.clone();
        let prompt_query = self.find_in_files.query.clone();
        let prompt_replacement = replacement.clone();

        self.find_in_files.error = None;
        self.find_in_files.status = Some("Confirming Replace in Files…".to_string());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            // OS-native confirm. AsyncMessageDialog returns a button enum
            // — "Yes" / "No" mapped onto Notepad++'s Apply / Cancel.
            let confirm = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Replace in Files")
                .set_description(format!(
                    "Replace every match of\n  {prompt_query}\nwith\n  {prompt_replacement}\nin {} ?\n\nThis rewrites every matching file on disk and cannot be undone from inside Notepad Studio.",
                    dir.display()
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            if !matches!(confirm, rfd::MessageDialogResult::Yes) {
                this.update(cx, |w, cx| {
                    w.find_in_files.status = Some("Replace in Files cancelled".to_string());
                    cx.notify();
                })
                .ok();
                return;
            }
            this.update(cx, |w, cx| {
                w.find_in_files.searching = true;
                w.find_in_files.status = Some("Replacing…".to_string());
                cx.notify();
            })
            .ok();
            let report =
                cx.background_spawn(async move { run_replace_in_files(&query, &replacement) })
                    .await;
            this.update(cx, |w, cx| {
                w.find_in_files.searching = false;
                match report {
                    Ok(report) => {
                        w.find_in_files.status = Some(format!(
                            "Replaced {} occurrence{} in {} file{}{}",
                            report.replacements,
                            if report.replacements == 1 { "" } else { "s" },
                            report.files_changed,
                            if report.files_changed == 1 { "" } else { "s" },
                            if report.failures > 0 {
                                format!(" · {} failed", report.failures)
                            } else {
                                String::new()
                            },
                        ));
                        w.find_in_files.error = None;
                    }
                    Err(msg) => {
                        w.find_in_files.error = Some(msg);
                        w.find_in_files.status = None;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn find_in_files_pick_dir(
        &mut self,
        _: &FindInFilesPickDir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_dir = self.find_in_files.directory.clone();
        let starting = active_dir
            .clone()
            .or_else(|| crate::settings::resolve_starting_dir(None));
        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Find in Folder");
            if let Some(dir) = starting {
                dialog = dialog.set_directory(dir);
            }
            let Some(handle) = dialog.pick_folder().await else {
                return;
            };
            let path = handle.path().to_path_buf();
            crate::settings::record_last_used_directory(&path);
            this.update(cx, |w, cx| {
                w.find_in_files.directory_text = path.display().to_string();
                w.find_in_files.directory_text_cursor =
                    w.find_in_files.directory_text.len();
                w.find_in_files.directory = Some(path);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn find_in_files_toggle_case(
        &mut self,
        _: &FindInFilesToggleCase,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.match_case = !self.find_in_files.match_case;
        cx.notify();
    }

    pub(crate) fn find_in_files_toggle_word(
        &mut self,
        _: &FindInFilesToggleWord,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.whole_word = !self.find_in_files.whole_word;
        cx.notify();
    }

    pub(crate) fn find_in_files_toggle_regex(
        &mut self,
        _: &FindInFilesToggleRegex,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.regex = !self.find_in_files.regex;
        // Mirror Notepad++'s mutual exclusion — turning regex on clears
        // extended; turning it off leaves extended alone.
        if self.find_in_files.regex {
            self.find_in_files.extended = false;
        }
        cx.notify();
    }

    /// Toggle Notepad++'s "Extended" radio. Mirrors mutual exclusion with
    /// `regex` — turning Extended on clears `regex`. Caller is the FIF tab's
    /// Extended radio; the dialog also flips this directly when switching
    /// to Normal mode.
    pub(crate) fn find_in_files_toggle_extended(
        &mut self,
        _: &FindInFilesToggleExtended,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.extended = !self.find_in_files.extended;
        if self.find_in_files.extended {
            self.find_in_files.regex = false;
        }
        cx.notify();
    }

    pub(crate) fn find_in_files_toggle_follow(
        &mut self,
        _: &FindInFilesToggleFollow,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.follow_current_doc = !self.find_in_files.follow_current_doc;
        if self.find_in_files.follow_current_doc {
            if let Some(dir) = self
                .buffer()
                .file_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
            {
                self.find_in_files.directory_text = dir.display().to_string();
                self.find_in_files.directory_text_cursor =
                    self.find_in_files.directory_text.len();
                self.find_in_files.directory = Some(dir);
            }
        }
        cx.notify();
    }

    pub(crate) fn find_in_files_toggle_subfolders(
        &mut self,
        _: &FindInFilesToggleSubfolders,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.in_subfolders = !self.find_in_files.in_subfolders;
        cx.notify();
    }

    pub(crate) fn find_in_files_toggle_hidden(
        &mut self,
        _: &FindInFilesToggleHidden,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.in_hidden_folders = !self.find_in_files.in_hidden_folders;
        cx.notify();
    }

    /// Open the result's file — focus the existing tab if already open, or
    /// load it via `open_path`. Then place the selection on the match span
    /// and scroll into view. Panel stays open for follow-up jumps.
    ///
    /// "Find All in All Opened Documents" results carry an explicit
    /// `tab_idx` so we skip the path lookup (which would fail on Untitled
    /// buffers and is racy if the user renamed the file since the search).
    pub(crate) fn jump_to_find_result(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(result) = self.find_in_files.results.get(idx).cloned() else {
            return;
        };
        let path = result.path.clone();

        let prev_tab = self.active;
        let tab_idx = if let Some(i) = result.tab_idx {
            // All-opened result — index is authoritative. Bounds-check so a
            // tab closed between search and click can't panic.
            if i >= self.tabs.len() {
                return;
            }
            if i != self.active {
                self.set_active_tab(i, cx);
            }
            i
        } else {
            let existing = self
                .tabs
                .iter()
                .position(|t| t.buffer.file_path.as_deref() == Some(path.as_path()));
            if let Some(i) = existing {
                self.set_active_tab(i, cx);
                i
            } else {
                self.open_path(path.clone(), cx);
                self.active
            }
        };

        // Rebuild `find_state.matches` against the newly-active buffer.
        // Without this, the editor's match-highlight overlay would keep the
        // byte ranges computed for the previously-active buffer and paint
        // them in the new buffer, lighting up arbitrary 3-letter substrings
        // at those stale offsets (the "ing" search lighting up `clea`,
        // `uffer`, `\0` bug). `refresh_matches` re-runs the current Find
        // query against the now-active rope, so the overlay tracks the file
        // the user just jumped into.
        if tab_idx != prev_tab {
            self.refresh_matches();
        }

        let buffer = &self.tabs[tab_idx].buffer;
        let total_lines = buffer.rope.len_lines().max(1);
        let line = result.line.min(total_lines - 1);
        let line_start = buffer.rope.line_to_byte(line);
        let line_slice = buffer.rope.line(line);
        let line_str = line_slice.to_string();
        let line_str_no_eol = line_str.trim_end_matches(['\n', '\r']);
        let local_start = result.match_in_line.start.min(line_str_no_eol.len());
        let local_end = result.match_in_line.end.min(line_str_no_eol.len());
        let abs_start = line_start + local_start;
        let abs_end = line_start + local_end;

        let tab = &mut self.tabs[tab_idx];
        tab.selection.anchor = abs_start;
        tab.selection.head = abs_end;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        self.last_lines.clear();
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    // ---- Find in Projects ---------------------------------------------------

    /// Open the dialog targeted at the Find in Projects tab. Shares
    /// query/replace/filter/options state with FIF — only the search scope
    /// differs (sidebar `folder_panel_roots` instead of an explicit
    /// Directory). Mirrors NPP's `enableFindInProjectsFunc` entry point.
    pub(crate) fn open_find_in_projects(
        &mut self,
        _: &SearchFindInProjects,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_in_files.query.is_empty() && !self.selection().is_empty() {
            let r = self.selection().ordered();
            let text = self.buffer().slice_bytes(r);
            if !text.is_empty() && !text.contains('\n') && text.len() < 256 {
                self.find_in_files.query = text;
                self.find_in_files.query_cursor = self.find_in_files.query.len();
            }
        }
        open_or_focus_find_dialog(self, DialogTab::FindInProject, window, cx);
    }

    /// Folder-panel roots that drive Project Panel 1/2/3 in the dialog. NPP
    /// caps at three so we mirror that — extra sidebar roots beyond index 2
    /// can't be searched from this tab (the user can switch to Find in
    /// Files for those).
    pub(crate) fn find_in_projects_selected_roots(&self) -> Vec<PathBuf> {
        let snap = crate::settings::snapshot();
        let mut out = Vec::new();
        let roots = &snap.folder_panel_roots;
        let flags = [
            self.find_in_files.project_panel_1,
            self.find_in_files.project_panel_2,
            self.find_in_files.project_panel_3,
        ];
        for (i, on) in flags.iter().enumerate() {
            if *on {
                if let Some(p) = roots.get(i) {
                    out.push(p.clone());
                }
            }
        }
        out
    }

    pub(crate) fn find_in_projects_run_action(
        &mut self,
        _: &FindInProjectsRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifQuery,
            &self.find_in_files.query,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifFilter,
            &self.find_in_files.filter,
        );
        self.find_in_files.open = true;
        self.run_find_in_projects(window, cx);
    }

    /// Fan out the FIF engine over every checked project-panel root and
    /// merge the per-root results into one list. Background-executor-friendly
    /// — same async wiring as `run_find_in_files`.
    pub(crate) fn run_find_in_projects(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_in_files.searching {
            return;
        }
        if self.find_in_files.query.trim().is_empty() {
            self.find_in_files.error = Some("Enter a search query".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        }
        let roots = self.find_in_projects_selected_roots();
        if roots.is_empty() {
            self.find_in_files.error =
                Some("Tick a Project Panel with an open folder".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        }
        let text = self.find_in_files.query.clone();
        let filter = self.find_in_files.filter.clone();
        let match_case = self.find_in_files.match_case;
        let whole_word = self.find_in_files.whole_word;
        let regex = self.find_in_files.regex;
        let extended = self.find_in_files.extended;
        let dot_matches_newline = self.find_in_files.dot_matches_newline;

        self.find_in_files.searching = true;
        self.find_in_files.error = None;
        self.find_in_files.status = Some("Searching projects…".to_string());
        self.find_in_files.results.clear();
        self.find_in_files.truncated = false;
        cx.notify();

        let bg_task = cx.background_executor().spawn(async move {
            let mut merged_results = Vec::new();
            let mut merged_files = 0usize;
            let mut merged_truncated = false;
            let mut merged_elapsed = 0u64;
            let mut invalid_regex: Option<String> = None;
            for dir in &roots {
                let q = FindInFilesQuery {
                    text: text.clone(),
                    directory: dir.clone(),
                    filter: filter.clone(),
                    match_case,
                    whole_word,
                    regex,
                    extended,
                    dot_matches_newline,
                    in_subfolders: true,
                    in_hidden_folders: false,
                };
                match run_search(&q) {
                    FindInFilesOutcome::Ok {
                        results,
                        searched_files,
                        truncated,
                        elapsed_ms,
                    } => {
                        merged_results.extend(results);
                        merged_files += searched_files;
                        merged_truncated |= truncated;
                        merged_elapsed += elapsed_ms;
                    }
                    FindInFilesOutcome::InvalidRegex(msg) => {
                        invalid_regex = Some(msg);
                        break;
                    }
                    FindInFilesOutcome::DirectoryMissing => {
                        // Skip missing roots silently — user may have one
                        // panel pointing at a folder that's been moved.
                    }
                }
            }
            if let Some(msg) = invalid_regex {
                FindInFilesOutcome::InvalidRegex(msg)
            } else {
                FindInFilesOutcome::Ok {
                    results: merged_results,
                    searched_files: merged_files,
                    truncated: merged_truncated,
                    elapsed_ms: merged_elapsed,
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = bg_task.await;
            this.update(cx, |w, cx| {
                w.find_in_files.searching = false;
                if !w.find_in_files.open {
                    return;
                }
                match outcome {
                    FindInFilesOutcome::Ok {
                        results,
                        searched_files,
                        truncated,
                        elapsed_ms,
                    } => {
                        let n = results.len();
                        w.find_in_files.results = results;
                        w.find_in_files.truncated = truncated;
                        w.find_in_files.error = None;
                        w.find_in_files.status = Some(format!(
                            "{n} match{} in {searched_files} file{} · {elapsed_ms}ms{}",
                            if n == 1 { "" } else { "es" },
                            if searched_files == 1 { "" } else { "s" },
                            if truncated { " (truncated)" } else { "" },
                        ));
                    }
                    FindInFilesOutcome::InvalidRegex(msg) => {
                        w.find_in_files.error = Some(msg);
                        w.find_in_files.status = None;
                    }
                    FindInFilesOutcome::DirectoryMissing => {
                        w.find_in_files.error = Some("Folder not found".to_string());
                        w.find_in_files.status = None;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Replace in Projects" — same destructive on-disk rewrite as Replace
    /// in Files, but fans out over every checked project-panel root.
    pub(crate) fn find_in_projects_replace(
        &mut self,
        _: &FindInProjectsReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_in_files.query.trim().is_empty() {
            self.find_in_files.error = Some("Enter a search query".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        }
        let roots = self.find_in_projects_selected_roots();
        if roots.is_empty() {
            self.find_in_files.error =
                Some("Tick a Project Panel with an open folder".to_string());
            self.find_in_files.status = None;
            cx.notify();
            return;
        }
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifQuery,
            &self.find_in_files.query,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifReplacement,
            &self.find_in_files.replacement,
        );
        crate::settings::push_find_history(
            crate::settings::FindHistorySlot::FifFilter,
            &self.find_in_files.filter,
        );
        let text = self.find_in_files.query.clone();
        let filter = self.find_in_files.filter.clone();
        let match_case = self.find_in_files.match_case;
        let whole_word = self.find_in_files.whole_word;
        let regex = self.find_in_files.regex;
        let extended = self.find_in_files.extended;
        let dot_matches_newline = self.find_in_files.dot_matches_newline;
        let replacement = self.find_in_files.replacement.clone();
        let prompt_query = self.find_in_files.query.clone();
        let prompt_replacement = replacement.clone();
        let dirs_list = roots
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n");

        self.find_in_files.error = None;
        self.find_in_files.status = Some("Confirming Replace in Projects…".to_string());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let confirm = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Replace in Projects")
                .set_description(format!(
                    "Replace every match of\n  {prompt_query}\nwith\n  {prompt_replacement}\nin:\n{dirs_list}\n\nThis rewrites every matching file on disk and cannot be undone from inside Notepad Studio."
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            if !matches!(confirm, rfd::MessageDialogResult::Yes) {
                this.update(cx, |w, cx| {
                    w.find_in_files.status = Some("Replace in Projects cancelled".to_string());
                    cx.notify();
                })
                .ok();
                return;
            }
            this.update(cx, |w, cx| {
                w.find_in_files.searching = true;
                w.find_in_files.status = Some("Replacing…".to_string());
                cx.notify();
            })
            .ok();
            let report = cx
                .background_spawn(async move {
                    let mut total = ReplaceInFilesReport {
                        files_changed: 0,
                        replacements: 0,
                        failures: 0,
                    };
                    let mut last_err: Option<String> = None;
                    for dir in &roots {
                        let q = FindInFilesQuery {
                            text: text.clone(),
                            directory: dir.clone(),
                            filter: filter.clone(),
                            match_case,
                            whole_word,
                            regex,
                            extended,
                            dot_matches_newline,
                            in_subfolders: true,
                            in_hidden_folders: false,
                        };
                        match run_replace_in_files(&q, &replacement) {
                            Ok(r) => {
                                total.files_changed += r.files_changed;
                                total.replacements += r.replacements;
                                total.failures += r.failures;
                            }
                            Err(msg) => {
                                last_err = Some(msg);
                            }
                        }
                    }
                    if total.files_changed == 0
                        && total.replacements == 0
                        && total.failures == 0
                        && last_err.is_some()
                    {
                        Err(last_err.unwrap())
                    } else {
                        Ok(total)
                    }
                })
                .await;
            this.update(cx, |w, cx| {
                w.find_in_files.searching = false;
                match report {
                    Ok(report) => {
                        w.find_in_files.status = Some(format!(
                            "Replaced {} occurrence{} in {} file{}{}",
                            report.replacements,
                            if report.replacements == 1 { "" } else { "s" },
                            report.files_changed,
                            if report.files_changed == 1 { "" } else { "s" },
                            if report.failures > 0 {
                                format!(" · {} failed", report.failures)
                            } else {
                                String::new()
                            },
                        ));
                        w.find_in_files.error = None;
                    }
                    Err(msg) => {
                        w.find_in_files.error = Some(msg);
                        w.find_in_files.status = None;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Swap the Find what / Replace with strings — NPP's small ↑↓ button to
    /// the right of the input column. Mirrors `IDD_FINDREPLACE_SWAP_BUTTON`.
    pub(crate) fn find_in_projects_swap(
        &mut self,
        _: &FindInProjectsSwap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        std::mem::swap(
            &mut self.find_in_files.query,
            &mut self.find_in_files.replacement,
        );
        std::mem::swap(
            &mut self.find_in_files.query_cursor,
            &mut self.find_in_files.replacement_cursor,
        );
        std::mem::swap(
            &mut self.find_in_files.query_anchor,
            &mut self.find_in_files.replacement_anchor,
        );
        cx.notify();
    }

    pub(crate) fn find_in_projects_toggle_panel_1(
        &mut self,
        _: &FindInProjectsTogglePanel1,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.project_panel_1 = !self.find_in_files.project_panel_1;
        cx.notify();
    }

    pub(crate) fn find_in_projects_toggle_panel_2(
        &mut self,
        _: &FindInProjectsTogglePanel2,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.project_panel_2 = !self.find_in_files.project_panel_2;
        cx.notify();
    }

    pub(crate) fn find_in_projects_toggle_panel_3(
        &mut self,
        _: &FindInProjectsTogglePanel3,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.project_panel_3 = !self.find_in_files.project_panel_3;
        cx.notify();
    }

    pub(crate) fn find_in_files_swap(
        &mut self,
        _: &FindInFilesSwap,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        std::mem::swap(
            &mut self.find_in_files.query,
            &mut self.find_in_files.replacement,
        );
        std::mem::swap(
            &mut self.find_in_files.query_cursor,
            &mut self.find_in_files.replacement_cursor,
        );
        std::mem::swap(
            &mut self.find_in_files.query_anchor,
            &mut self.find_in_files.replacement_anchor,
        );
        cx.notify();
    }

    pub(crate) fn find_in_files_toggle_transparency(
        &mut self,
        _: &FindInFilesToggleTransparency,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.transparency_enabled = !self.find_in_files.transparency_enabled;
        cx.notify();
    }

    pub(crate) fn find_in_files_transparency_on_focus_loss(
        &mut self,
        _: &FindInFilesTransparencyOnFocusLoss,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.transparency_always = false;
        cx.notify();
    }

    pub(crate) fn find_in_files_transparency_always(
        &mut self,
        _: &FindInFilesTransparencyAlways,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_in_files.transparency_always = true;
        cx.notify();
    }

    /// Slider step — the slider thumb in NPP slides through a coarse range
    /// (≈25–95 by default). We step in 5-point jumps and clamp into the
    /// sensible band so the dialog never disappears entirely.
    pub(crate) fn find_in_files_transparency_dec(
        &mut self,
        _: &FindInFilesTransparencyDec,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lvl = self.find_in_files.transparency_level.saturating_sub(5);
        self.find_in_files.transparency_level = lvl.max(25);
        cx.notify();
    }

    pub(crate) fn find_in_files_transparency_inc(
        &mut self,
        _: &FindInFilesTransparencyInc,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lvl = self.find_in_files.transparency_level.saturating_add(5);
        self.find_in_files.transparency_level = lvl.min(95);
        cx.notify();
    }
}

/// Format the search-mode label used in the NPP-style results panel header
/// (`[Normal]` / `[Extended]` / `[Regular expression]`). Mirrors the radio
/// labels in the Find dialog so the two read the same.
pub(crate) fn describe_search_mode(regex: bool, extended: bool) -> String {
    if regex {
        "Regular expression".to_string()
    } else if extended {
        "Extended".to_string()
    } else {
        "Normal".to_string()
    }
}

/// Outcome of a successful Replace-in-Files run.
struct ReplaceInFilesReport {
    files_changed: usize,
    replacements: usize,
    failures: usize,
}

/// Walk `query.directory`, regex-replace every match with `replacement`, and
/// rewrite each affected file in place. Lives off the UI thread — caller is
/// responsible for keeping it that way.
///
/// Uses the same `build_pattern` helper as the search path so every flag
/// (extended-escape decoding, dot-matches-newline, case, whole-word) lands
/// with identical semantics on both sides.
fn run_replace_in_files(
    query: &FindInFilesQuery,
    replacement: &str,
) -> Result<ReplaceInFilesReport, String> {
    use walkdir::WalkDir;
    if !query.directory.is_dir() {
        return Err("Directory not found".to_string());
    }
    let re = build_pattern(query).map_err(|e| format!("Invalid regex: {e}"))?;

    let mut files_changed = 0usize;
    let mut replacements = 0usize;
    let mut failures = 0usize;

    let in_subfolders = query.in_subfolders;
    let in_hidden = query.in_hidden_folders;
    let walker = WalkDir::new(&query.directory)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |e| {
            if e.depth() == 0 {
                return true;
            }
            // Honour the "in sub-folders" toggle — prune every directory
            // below the root so file walking stays at depth 1.
            if !in_subfolders && e.file_type().is_dir() && e.depth() >= 1 {
                return false;
            }
            let Some(name) = e.file_name().to_str() else {
                return false;
            };
            if e.file_type().is_dir() {
                if name.starts_with('.') && !in_hidden {
                    return false;
                }
                if matches!(
                    name,
                    ".git"
                        | "node_modules"
                        | "target"
                        | ".next"
                        | "dist"
                        | "build"
                        | ".venv"
                        | "venv"
                        | "__pycache__"
                        | ".cache"
                ) {
                    return false;
                }
            }
            true
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !crate::find_in_files::matches_filter(filename, &query.filter) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.len() > crate::find_in_files::FIND_IN_FILES_FILE_SIZE_LIMIT {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => {
                failures += 1;
                continue;
            }
        };
        let content = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let count_in_file = re.find_iter(content).count();
        if count_in_file == 0 {
            continue;
        }
        let new_content = re.replace_all(content, replacement).into_owned();
        match std::fs::write(path, new_content.as_bytes()) {
            Ok(()) => {
                files_changed += 1;
                replacements += count_in_file;
            }
            Err(_) => {
                failures += 1;
            }
        }
    }

    Ok(ReplaceInFilesReport {
        files_changed,
        replacements,
        failures,
    })
}

