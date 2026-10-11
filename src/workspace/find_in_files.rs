// Find-in-Files handlers on Workspace — open / close / run / pick-dir /
// toggle, plus the click-to-jump path that opens the result's file at the
// right line and selects the match.
//
// The actual filesystem walk lives in `crate::find_in_files` (the engine
// module). This file orchestrates: gather inputs, dispatch the engine on
// `cx.background_executor`, and apply results back on the main thread.

use std::path::PathBuf;

use gpui::{Context, Window};

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
use crate::find_in_files::{FindInFilesQuery, build_pattern};
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
        if !self.directory_operations.busy() && !self.cancel_rich_find_current() {
            self.set_status("Search cleanup is in progress; try again"); cx.notify(); return;
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
        if self.window_closed || self.directory_operations.busy() || self.find_in_files.searching { return; }
        if self.find_in_files.query.len() > 1024 * 1024 || self.find_in_files.filter.len() > 1024 * 1024 {
            self.find_in_files.error = Some("Directory query or filter exceeds 1 MiB".into()); cx.notify(); return;
        }
        if self.find_in_files.directory_text.len() > 1024 * 1024 {
            self.find_in_files.error = Some("Directory path exceeds 1 MiB".into()); cx.notify(); return;
        }
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
    pub(super) fn commit_directory_text(&mut self) {
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
        self.start_directory_operation(super::directory_operations::Route::Files, false, window, cx);
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
        self.start_directory_operation(super::directory_operations::Route::Files, true, window, cx);
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
    /// Open-buffer results resolve their original runtime document identity;
    /// stored indices and paths cannot identify a moved, closed or reopened tab.
    pub(crate) fn jump_to_find_result(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.jump_to_rich_find_result(idx, cx) { return; }
        let Some(result) = self.find_in_files.results.get(idx).cloned() else {
            return;
        };
        let path = result.path.clone();

        let prev_tab = self.active;
        let tab_idx = if let Some(source) = &result.source {
            let Some(i) = self.tabs.iter().position(|tab| tab.session_id == source.tab) else {
                self.set_status("This search result's document is closed; run the search again");
                cx.notify(); return;
            };
            let Some(tab) = self.tabs.get_mut(i) else { return; };
            tab.observe_content_activity();
            if !source.matches(tab) {
                self.set_status("This search result is out of date; run the search again");
                cx.notify(); return;
            }
            if i != self.active { self.set_active_tab(i, cx); }
            i
        } else if result.tab_idx.is_some() {
            self.set_status("This search result has no current document; run the search again");
            cx.notify(); return;
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
                // `open_path` reports its own failure (deleted, unreadable,
                // bad rich-doc) and leaves `active` where it was; the
                // selection must not then land in that unrelated tab.
                let opened = self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| t.buffer.file_path.as_deref() == Some(path.as_path()));
                if !opened {
                    return;
                }
                self.active
            }
        };

        // A pending CSV draft can refuse tab activation; retain that refusal
        // and leave the destination selection untouched.
        if self.active != tab_idx { return; }

        // Raw search rows carry rope coordinates, never rendered Doc View positions.
        if self.tabs.get(tab_idx).is_some_and(super::find_view::uses_rich_text) {
            self.set_status("This result is from Raw View; switch to Raw View to select it");
            cx.notify();
            return;
        }

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

        if tab_idx == self.active { self.settle_source_input_edit_caret(); }
        let Some(tab) = self.tabs.get_mut(tab_idx) else {
            return;
        };
        let range = result_range_in_rope(&tab.buffer.rope, result.line, &result.match_in_line);
        tab.selection.anchor = range.start;
        tab.selection.head = range.end;
        tab.selection.is_dragging = false;
        tab.desired_col_x = None;
        if tab_idx == self.active { self.sync_source_input_caret(); }
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

    pub(crate) fn find_in_projects_run_action(
        &mut self,
        _: &FindInProjectsRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.window_closed || self.directory_operations.busy() || self.find_in_files.searching { return; }
        if self.find_in_files.query.len() > 1024 * 1024 || self.find_in_files.filter.len() > 1024 * 1024 {
            self.find_in_files.error = Some("Directory query or filter exceeds 1 MiB".into()); cx.notify(); return;
        }
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
        self.start_directory_operation(super::directory_operations::Route::Projects, false, window, cx);
    }

    /// "Replace in Projects" — same destructive on-disk rewrite as Replace
    /// in Files, but fans out over every checked project-panel root.
    pub(crate) fn find_in_projects_replace(
        &mut self,
        _: &FindInProjectsReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_directory_operation(super::directory_operations::Route::Projects, true, window, cx);
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

/// Absolute byte range in `rope` of a result at `line`, `span` being its
/// offsets within that line. The file may have changed since the search,
/// so the line is clamped to the rope, the span to the line's content (any
/// ropey line break excluded, not only CR/LF), and both ends floored to a
/// char boundary: a mid-char selection panics the next `byte_slice`.
fn result_range_in_rope(
    rope: &ropey::Rope,
    line: usize,
    span: &std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let line = line.min(rope.len_lines().saturating_sub(1));
    let line_start = rope.line_to_byte(line);
    let slice = rope.line(line);
    let chars = slice.len_chars();
    let last = chars.checked_sub(1).and_then(|i| slice.get_char(i));
    let before_last = chars.checked_sub(2).and_then(|i| slice.get_char(i));
    let break_len = match (before_last, last) {
        (Some('\r'), Some('\n')) => 2,
        (_, Some(c @ ('\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'))) => {
            c.len_utf8()
        }
        _ => 0,
    };
    let content_len = slice.len_bytes().saturating_sub(break_len);
    let floor = |local: usize| {
        let byte = line_start + local.min(content_len);
        rope.char_to_byte(rope.byte_to_char(byte))
    };
    let start = floor(span.start);
    start..floor(span.end).max(start)
}

/// Outcome of a successful Replace-in-Files run.
#[derive(Default)]
pub(super) struct ReplaceInFilesReport {
    pub(super) cancelled: bool,
    pub(super) stopped: Option<super::project_file_visits::VisitStop>,
    pub(super) duplicate_visits: usize,
    pub(super) files_changed: usize,
    pub(super) replacements: usize,
    pub(super) failures: usize,
    pub(super) first_failure: Option<String>,
}

impl ReplaceInFilesReport {
    pub(super) fn failure_status(&self) -> String {
        format!(" · {} failed{}", self.failures,
            self.first_failure.as_ref().map_or(String::new(), |reason| format!(": {reason}")))
    }
}

/// Walk `query.directory`, regex-replace every match with `replacement`, and
/// rewrite each affected file in place. Lives off the UI thread — caller is
/// responsible for keeping it that way.
///
/// Uses the same `build_pattern` helper as the search path so every flag
/// (extended-escape decoding, dot-matches-newline, case, whole-word) lands
/// with identical semantics on both sides.
#[cfg(test)]
fn run_replace_in_files(
    query: &FindInFilesQuery,
    replacement: &str,
) -> Result<ReplaceInFilesReport, String> {
    run_replace_in_files_with_reader(query, replacement, crate::find_in_files::read_capped)
}

#[cfg(test)]
fn run_replace_in_files_with_reader(
    query: &FindInFilesQuery,
    replacement: &str,
    read: impl FnMut(&std::path::Path, u64) -> std::io::Result<Option<Vec<u8>>>,
) -> Result<ReplaceInFilesReport, String> {
    run_replace_in_files_with_reader_cancelled(query, replacement, read, &std::sync::atomic::AtomicBool::new(false))
}

#[cfg(test)]
pub(super) fn run_replace_in_files_with_reader_cancelled(
    query: &FindInFilesQuery, replacement: &str,
    read: impl FnMut(&std::path::Path, u64) -> std::io::Result<Option<Vec<u8>>>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<ReplaceInFilesReport, String> {
    run_replace_in_files_with_visits(query, replacement, read, cancelled, &mut super::project_file_visits::FileVisits::default())
}

pub(super) fn run_replace_in_files_with_visits(
    query: &FindInFilesQuery, replacement: &str,
    mut read: impl FnMut(&std::path::Path, u64) -> std::io::Result<Option<Vec<u8>>>,
    cancelled: &std::sync::atomic::AtomicBool,
    visits: &mut super::project_file_visits::FileVisits,
) -> Result<ReplaceInFilesReport, String> {
    use walkdir::WalkDir;
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(ReplaceInFilesReport { cancelled: true, ..ReplaceInFilesReport::default() });
    }
    if !query.directory.is_dir() {
        return Err("Directory not found".to_string());
    }
    let re = build_pattern(query).map_err(|e| format!("Invalid regex: {e}"))?;
    // Only Regular expression mode expands `$1` / `${name}` / `$$`. Normal
    // and Extended insert the text as typed (Extended after decoding its
    // escapes, as the Find tab does); expanding there turned `$5` into "".
    if replacement.len() > crate::text::regex_replacement::MAX_TEMPLATE_BYTES {
        return Err("Replacement template exceeds the size limit".into());
    }
    let replacement = if query.extended && !query.regex {
        crate::text::extended_escape::interpret_extended(replacement)
    } else {
        replacement.to_string()
    };
    let regex_template = if query.regex {
        Some(crate::text::regex_replacement::Template::parse(&replacement, &re)?)
    } else { None };

    let mut files_changed = 0usize;
    let mut replacements = 0usize;
    let mut failures = 0usize;
    let mut first_failure = None;
    let mut stopped = None; let mut duplicate_visits = 0usize;

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
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
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
        let limit = crate::find_in_files::FIND_IN_FILES_FILE_SIZE_LIMIT;
        if metadata.len() > limit {
            continue;
        }
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
        match visits.admit(path) {
            Ok(true) => {}
            Ok(false) => { duplicate_visits = duplicate_visits.saturating_add(1); continue; }
            Err(super::project_file_visits::VisitError::File(error)) => {
                failures = failures.saturating_add(1);
                if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
                continue;
            }
            Err(super::project_file_visits::VisitError::Stop(reason)) => { stopped = Some(reason); break; }
        }
        // Bounded read: a file that grew past the limit since the stat is
        // skipped whole, never rewritten from a truncated copy.
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
        let bytes = match read(path, limit) {
            Ok(Some(b)) => b,
            Ok(None) => continue,
            Err(error) => {
                failures += 1;
                if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
                continue;
            }
        };
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
        let decoded = match crate::text::search_file::decode(&bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                failures += 1;
                if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
                continue;
            }
        };
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
        let content = decoded.text.as_ref();
        let (replaced, count_in_file) = if let Some(template) = &regex_template {
            let result = template.plan(&re, content, None, crate::text::regex_replacement::MAX_CACHED_MATCHES)
                .and_then(|plan| crate::text::regex_replacement::apply_to_string(content, &plan).map(|text| (text, plan.len())));
            match result {
                Ok((text, count)) => (std::borrow::Cow::Owned(text), count),
                Err(error) => {
                    failures += 1;
                    if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
                    continue;
                }
            }
        } else {
            match crate::text::literal_replacement::replace(&re, content, &replacement) {
                Ok(result) => result,
                Err(error) => {
                    failures += 1;
                    if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
                    continue;
                }
            }
        };
        if count_in_file == 0 { continue; }
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
        let new_content = match crate::text::encoding::encode_checked(&replaced, decoded.encoding) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures += 1;
                if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
                continue;
            }
        };
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { break; }
        match crate::io::write_file_safely(path, &new_content) {
            Ok(()) => {
                files_changed += 1;
                replacements += count_in_file;
            }
            Err(error) => {
                failures += 1;
                if first_failure.is_none() { first_failure = Some(format!("{}: {error}", path.display())); }
            }
        }
    }

    Ok(ReplaceInFilesReport {
        cancelled: cancelled.load(std::sync::atomic::Ordering::Acquire),
        stopped, duplicate_visits,
        files_changed,
        replacements,
        failures,
        first_failure,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn lifecycle_worker_cancelled_before_read_preserves_utf16_bom() {
        let dir = tempfile::tempdir().unwrap(); let file = dir.path().join("neutral.txt");
        let bytes = utf16_sample(); std::fs::write(&file, &bytes).unwrap();
        let token = std::sync::atomic::AtomicBool::new(true);
        let report = run_replace_in_files_with_reader_cancelled(&query(dir.path(), "hit", false, false),
            "done", |_, _| panic!("cancelled worker must not read"), &token).unwrap();
        assert!(report.cancelled); assert_eq!((report.files_changed, report.replacements, report.failures), (0, 0, 0));
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }

    fn utf16_sample() -> Vec<u8> {
        let mut bytes = vec![0xff, 0xfe];
        for unit in "hit\r\n\u{e9}".encode_utf16() { bytes.extend_from_slice(&unit.to_le_bytes()); }
        bytes
    }

    fn held_worker(cancel_at: usize, first_read_fails: bool) {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("neutral-one.txt"); let second = dir.path().join("neutral-two.txt");
        let bytes = utf16_sample();
        std::fs::write(&first, &bytes).unwrap(); std::fs::write(&second, &bytes).unwrap();
        let q = query(dir.path(), "hit", false, false);
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)); let worker_token = token.clone();
        let (entered, waiting) = std::sync::mpsc::channel(); let (release, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut reads = 0;
            run_replace_in_files_with_reader_cancelled(&q, "done", |path, limit| {
                reads += 1;
                if first_read_fails && reads == 1 {
                    return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "synthetic read refusal"));
                }
                let actual = crate::find_in_files::read_capped(path, limit)?;
                if reads == cancel_at { entered.send(()).unwrap(); released.recv().unwrap(); }
                Ok(actual)
            }, &worker_token).unwrap()
        });
        let reached = waiting.recv_timeout(std::time::Duration::from_secs(5));
        token.store(true, std::sync::atomic::Ordering::Release); release.send(()).unwrap();
        let report = worker.join().unwrap(); reached.unwrap(); assert!(report.cancelled);
        assert_eq!(report.files_changed, usize::from(cancel_at == 2 && !first_read_fails));
        assert_eq!(report.replacements, report.files_changed);
        assert_eq!(report.failures, usize::from(first_read_fails));
        let written: Vec<_> = [first, second].iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(written.iter().filter(|b| **b == bytes).count(), 2 - report.files_changed);
        for changed in written.iter().filter(|b| **b != bytes) {
            let mut expected = vec![0xff, 0xfe];
            for unit in "done\r\n\u{e9}".encode_utf16() { expected.extend_from_slice(&unit.to_le_bytes()); }
            assert_eq!(changed, &expected);
        }
        if first_read_fails { assert!(report.first_failure.as_deref().unwrap().contains("synthetic read refusal")); }
    }

    #[test] fn lifecycle_worker_held_read_cancels_before_first_write() { held_worker(1, false); }
    #[test] fn lifecycle_worker_held_second_read_preserves_written_prefix() { held_worker(2, false); }
    #[test] fn lifecycle_worker_cancelled_after_read_failure_preserves_counts() { held_worker(2, true); }

    #[test]
    fn file_encoding_replace_ansi_fallback_preserves_every_unmatched_byte() {
        let dir = temp_dir();
        let all: Vec<u8> = (0..=255).collect();
        let prefixes = [all, b"\xef\xbb\xbfcaf\xe9".to_vec()];
        for (index, prefix) in prefixes.iter().enumerate() {
            let mut bytes = prefix.clone(); bytes.extend_from_slice(b" hit\r\n");
            std::fs::write(dir.join(format!("neutral{index}.txt")), bytes).unwrap();
        }
        let report = run_replace_in_files(&query(&dir, "hit", false, false), "done").unwrap();
        assert_eq!((report.files_changed, report.replacements, report.failures), (2, 2, 0));
        for (index, prefix) in prefixes.into_iter().enumerate() {
            let mut expected = prefix; expected.extend_from_slice(b" done\r\n");
            assert_eq!(std::fs::read(dir.join(format!("neutral{index}.txt"))).unwrap(), expected);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_encoding_replace_preserves_bom_encoding_eol_and_no_match_bytes() {
        use crate::text::encoding::{encode_checked, Encoding};
        let dir = temp_dir();
        let encodings = [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1];
        for (index, encoding) in encodings.into_iter().enumerate() {
            std::fs::write(dir.join(format!("neutral{index}.txt")), encode_checked("caf\u{e9} hit\r\nhit\nlast\r", encoding).unwrap()).unwrap();
            std::fs::write(dir.join(format!("unchanged{index}.txt")), encode_checked("caf\u{e9} unchanged\r\n", encoding).unwrap()).unwrap();
        }
        let report = run_replace_in_files(&query(&dir, "hit", false, false), "done").unwrap();
        assert_eq!((report.files_changed, report.replacements, report.failures), (5, 10, 0));
        for (index, encoding) in encodings.into_iter().enumerate() {
            assert_eq!(std::fs::read(dir.join(format!("neutral{index}.txt"))).unwrap(), encode_checked("caf\u{e9} done\r\ndone\nlast\r", encoding).unwrap());
            assert_eq!(std::fs::read(dir.join(format!("unchanged{index}.txt"))).unwrap(), encode_checked("caf\u{e9} unchanged\r\n", encoding).unwrap());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_encoding_replace_utf16_regex_preserves_captures_and_bom() {
        use crate::text::encoding::{encode_checked, Encoding};
        let dir = temp_dir(); let path = dir.join("neutral.txt");
        std::fs::write(&path, encode_checked("hit\r\n", Encoding::Utf16Be).unwrap()).unwrap();
        let report = run_replace_in_files(&query(&dir, "(?P<word>hit)", true, false), "${word}-caf\u{e9}").unwrap();
        assert_eq!((report.files_changed, report.replacements, report.failures), (1, 1, 0));
        assert_eq!(std::fs::read(&path).unwrap(), encode_checked("hit-caf\u{e9}\r\n", Encoding::Utf16Be).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_encoding_replace_refuses_malformed_utf16_without_skipping_valid_sibling() {
        use crate::text::encoding::{encode_checked, Encoding};
        let dir = temp_dir(); let bad = dir.join("neutral-bad.txt"); let good = dir.join("neutral-good.txt");
        let mut bytes = encode_checked("hit\r\n", Encoding::Utf16Le).unwrap(); bytes.push(0x41);
        std::fs::write(&bad, &bytes).unwrap(); std::fs::write(&good, "hit\r\n").unwrap();
        let report = run_replace_in_files(&query(&dir, "hit", false, false), "done").unwrap();
        assert_eq!((report.files_changed, report.replacements, report.failures), (1, 1, 1));
        assert!(report.first_failure.as_deref().is_some_and(|error| error.contains("neutral-bad.txt")));
        assert_eq!(std::fs::read(&bad).unwrap(), bytes); assert_eq!(std::fs::read(&good).unwrap(), b"done\r\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_encoding_replace_refuses_unmappable_latin1_without_skipping_utf8() {
        let dir = temp_dir(); let bad = dir.join("neutral-bad.txt"); let good = dir.join("neutral-good.txt");
        let bytes = b"caf\xe9 hit\r\n";
        std::fs::write(&bad, bytes).unwrap(); std::fs::write(&good, "hit\r\n").unwrap();
        let report = run_replace_in_files(&query(&dir, "hit", false, false), "\u{4e2d}").unwrap();
        assert_eq!((report.files_changed, report.replacements, report.failures), (1, 1, 1));
        assert!(report.first_failure.as_deref().is_some_and(|error| error.contains("neutral-bad.txt")));
        assert_eq!(std::fs::read(&bad).unwrap(), bytes); assert_eq!(std::fs::read(&good).unwrap(), "\u{4e2d}\r\n".as_bytes());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn literal_file_expansion_refuses_whole_file_before_write() {
        let dir = temp_dir();
        let path = dir.join("neutral.txt");
        let original = "a".repeat(4096);
        std::fs::write(&path, &original).expect("fixture");
        let report = run_replace_in_files(&query(&dir, "a", false, false), &"b".repeat(4097)).expect("report");
        assert_eq!(report.files_changed, 0);
        assert_eq!(report.failures, 1);
        assert!(report.first_failure.as_deref().is_some_and(|error| error.contains("neutral.txt")));
        assert_eq!(std::fs::read(&path).expect("read"), original.as_bytes());
        let sibling = dir.join("sibling.txt");
        std::fs::write(&sibling, "a").expect("sibling fixture");
        let report = run_replace_in_files(&query(&dir, "a", false, false), &"b".repeat(4097)).expect("report");
        assert_eq!((report.files_changed, report.replacements, report.failures), (1, 1, 1));
        assert_eq!(std::fs::read_to_string(&sibling).expect("sibling"), "b".repeat(4097));
        assert_eq!(std::fs::read(&path).expect("read"), original.as_bytes());
        let report = run_replace_in_files(&query(&dir, "a", false, true), &"b".repeat(4097)).expect("extended report");
        assert_eq!((report.files_changed, report.failures), (0, 1));
        assert_eq!(std::fs::read(&path).expect("read"), original.as_bytes());
        let _ = std::fs::remove_dir_all(dir);
    }

    use super::*;
    use std::path::Path;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notepad-studio-fif-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn query(dir: &Path, text: &str, regex: bool, extended: bool) -> FindInFilesQuery {
        FindInFilesQuery {
            text: text.to_string(),
            directory: dir.to_path_buf(),
            filter: "*".to_string(),
            match_case: true,
            whole_word: false,
            regex,
            extended,
            dot_matches_newline: false,
            in_subfolders: true,
            in_hidden_folders: false,
        }
    }

    /// Write `content` to a fresh file, run Replace in Files over its
    /// directory and return the file's new bytes.
    fn replace_in(content: &[u8], text: &str, regex: bool, extended: bool, replacement: &str) -> Vec<u8> {
        let dir = temp_dir();
        let path = dir.join("f.txt");
        std::fs::write(&path, content).unwrap();
        let report = run_replace_in_files(&query(&dir, text, regex, extended), replacement);
        let out = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(report.is_ok());
        out
    }

    #[test]
    fn stale_result_never_selects_mid_char() {
        // The file changed after the search: the stored span now cuts
        // through "中", or through a U+2028 line separator ending the line.
        let cases: [(&str, usize, std::ops::Range<usize>); 4] = [
            ("中\n", 0, 1..2),
            ("ab\u{2028}cd", 0, 3..4),
            ("x\r\n😀", 9, 0..3),
            ("", 4, 2..9),
        ];
        for (text, line, span) in cases {
            let rope = ropey::Rope::from_str(text);
            let r = result_range_in_rope(&rope, line, &span);
            assert!(r.start <= r.end, "{text:?}");
            assert!(rope.get_byte_slice(r.clone()).is_some(), "{text:?} -> {r:?}");
        }
    }

    #[test]
    fn result_range_lands_on_the_match() {
        let rope = ropey::Rope::from_str("one\r\ntwo é three\u{2028}four");
        // Line 1 is "two é three", the match "three" sits at 7..12 in it.
        let r = result_range_in_rope(&rope, 1, &(7..12));
        assert_eq!(rope.byte_slice(r).to_string(), "three");
        // A span past the line end is clipped to the content, before the LS.
        let r = result_range_in_rope(&rope, 1, &(7..40));
        assert_eq!(rope.byte_slice(r).to_string(), "three");
    }

    #[test]
    fn normal_mode_replacement_is_literal() {
        // `$5` names a group that doesn't exist; expanding it wrote "".
        assert_eq!(replace_in(b"a-b", "-", false, false, "$5"), b"a$5b");
        assert_eq!(replace_in(b"a-b", "-", false, false, "$$"), b"a$$b");
        assert_eq!(replace_in(b"a-b", "-", false, false, "${x}"), b"a${x}b");
    }

    #[test]
    fn extended_mode_decodes_replacement_escapes() {
        assert_eq!(replace_in(b"a-b", "-", false, true, r"\t"), b"a\tb");
        assert_eq!(replace_in(b"a-b", "-", false, true, "$1"), b"a$1b");
    }

    #[test]
    fn regex_mode_expands_groups() {
        assert_eq!(replace_in(b"ab", "(a)(?<n>b)", true, false, "${n}$1"), b"ba");
        assert_eq!(replace_in(b"a", "(a)(b)?", true, false, "[$2]"), b"[]");
        assert_eq!(replace_in(b"ab", "(a)b", true, false, "$$"), b"$");
        assert_eq!(replace_in(b"ab", "(a)b", true, false, r"\1"), b"a");
        assert_eq!(replace_in(b"ab", "(?<n>a)b", true, false, "$+{n}"), b"a");
    }

    #[test]
    fn unsupported_regex_replacement_leaves_all_files_intact() {
        let dir = temp_dir();
        let path = dir.join("f.txt");
        std::fs::write(&path, b"a a").unwrap();
        for value in ["$2", r"\U$1\E", "?1yes:no", r"\0", r"\0101"] {
            assert!(run_replace_in_files(&query(&dir, "(a)", true, false), value).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"a a");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn discovered_file_read_failure_reports_missing_path_without_writing() {
        let dir = temp_dir();
        let missing = dir.join("removed-after-discovery.txt");
        let control = dir.join("unchanged.txt");
        let original = "\u{FEFF}中 café\r\n".as_bytes();
        std::fs::write(&missing, b"replace-me\r\n").unwrap();
        std::fs::write(&control, original).unwrap();
        let mut read_error = None;
        let mut reads = 0;
        let report = run_replace_in_files_with_reader(
            &query(&dir, "replace-me", false, false), "changed",
            |path, limit| {
                reads += 1;
                if path == missing {
                    // Simulate a file disappearing between discovery/stat and open.
                    std::fs::remove_file(path).unwrap();
                }
                let result = crate::find_in_files::read_capped(path, limit);
                if let Err(error) = &result {
                    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
                    read_error = Some(error.to_string());
                }
                result
            },
        ).unwrap();
        let control_after = std::fs::read(&control).unwrap();
        let missing_recreated = missing.exists();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(reads, 2);
        assert_eq!(report.files_changed, 0);
        assert_eq!(report.replacements, 0);
        assert_eq!(report.failures, 1);
        let reason = format!("{}: {}", missing.display(), read_error.unwrap());
        assert_eq!(report.first_failure.as_deref(), Some(reason.as_str()));
        assert_eq!(report.failure_status(), format!(" · 1 failed: {reason}"));
        assert!(!missing_recreated);
        assert_eq!(control_after, original);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn discovered_file_read_failure_reports_locked_path_without_source_loss() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = temp_dir();
        let path = dir.join("locked.txt");
        let original = "\u{FEFF}replace-me 中 café\r\n".as_bytes();
        std::fs::write(&path, original).unwrap();
        // Deny sharing only for this synthetic file; metadata still discovers it.
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let error = crate::find_in_files::read_capped(
            &path, crate::find_in_files::FIND_IN_FILES_FILE_SIZE_LIMIT,
        ).unwrap_err();
        let report = run_replace_in_files(&query(&dir, "replace-me", false, false), "changed").unwrap();
        drop(lock);
        let after = std::fs::read(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(report.files_changed, 0);
        assert_eq!(report.replacements, 0);
        assert_eq!(report.failures, 1);
        let reason = format!("{}: {error}", path.display());
        assert_eq!(report.first_failure.as_deref(), Some(reason.as_str()));
        assert_eq!(report.failure_status(), format!(" · 1 failed: {reason}"));
        assert_eq!(after, original);
    }

    #[test]
    fn regex_replace_keeps_crlf_line_endings() {
        // `.` used to match the `\r` of a CRLF file on disk, so `a.*` ate it
        // and the file came back with bare LF endings.
        assert_eq!(replace_in(b"a1\r\na2\r\n", "a.*", true, false, "b"), b"b\r\nb\r\n");
        assert_eq!(replace_in(b"a1\r\na2\r\n", "[0-9]$", true, false, "#"), b"a#\r\na#\r\n");
    }

    #[test]
    fn replacement_containing_the_query_runs_once() {
        assert_eq!(replace_in(b"aa", "a", false, false, "aa"), b"aaaa");
    }

    #[test]
    fn bom_and_crlf_survive_a_replace() {
        let out = replace_in("\u{FEFF}x\r\ny".as_bytes(), "y", false, false, "z");
        assert_eq!(out, "\u{FEFF}x\r\nz".as_bytes());
        // Find in Files skips the BOM, so `^x` matches the first line here too.
        let out = replace_in("\u{FEFF}x".as_bytes(), "^x", true, false, "z");
        assert_eq!(out, "\u{FEFF}z".as_bytes());
    }

    #[test]
    fn oversize_and_binary_files_are_left_alone() {
        let dir = temp_dir();
        let big = "x".repeat(crate::find_in_files::FIND_IN_FILES_FILE_SIZE_LIMIT as usize + 1);
        std::fs::write(dir.join("big.txt"), &big).unwrap();
        // An odd UTF-16 body is malformed; unmarked high bytes are valid ANSI text.
        let malformed = [0xFF, 0xFE, b'x', 0x00, 0xC3];
        std::fs::write(dir.join("bin.dat"), malformed).unwrap();
        let report = run_replace_in_files(&query(&dir, "x", false, false), "y").ok().unwrap();
        let big_after = std::fs::read(dir.join("big.txt")).unwrap();
        let bin_after = std::fs::read(dir.join("bin.dat")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(report.files_changed, 0);
        assert_eq!(report.failures, 1);
        assert!(report.first_failure.as_deref().is_some_and(|error| error.contains("bin.dat")));
        assert_eq!(big_after, big.as_bytes());
        assert_eq!(bin_after, malformed);
    }
}
