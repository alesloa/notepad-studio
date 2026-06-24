// Tab management — open, close, restore, cycle, and direct goto-N actions.

use std::collections::HashSet;

use gpui::{Context, Window};
use uuid::Uuid;

use crate::actions::{
    FileCloseAllTabs, FileCloseTab, FileRestoreClosedTab, TabGoto1, TabGoto2, TabGoto3, TabGoto4,
    TabGoto5, TabGoto6, TabGoto7, TabGoto8, TabGoto9, TabNext, TabNextSequential, TabPrev,
    TabPrevSequential,
};
use crate::tab::{CLOSED_TAB_LIMIT, ClosedTab, CloseDecision, Tab};
use crate::workspace::Workspace;
use crate::io::{save_tab_for_close, show_dirty_close_dialog};

impl Workspace {
    pub(crate) fn set_active_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() || idx == self.active {
            return;
        }
        self.active = idx;
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }

    /// Move the currently-active tab to the front of the document-switcher
    /// MRU and drop any stale session_ids whose tabs no longer exist. Called
    /// from every external tab-activation path (mouse click on a tab,
    /// goto-tab-N, open file, restore closed tab, session restore). Cycling
    /// (Ctrl+Tab) deliberately does NOT call this — the held-key sequence
    /// walks the MRU without scrambling its order; the next external action
    /// commits the chosen tab.
    pub(crate) fn touch_mru(&mut self) {
        let Some(active_id) = self.tabs.get(self.active).map(|t| t.session_id) else {
            self.tab_mru.clear();
            return;
        };
        let live: HashSet<Uuid> = self.tabs.iter().map(|t| t.session_id).collect();
        self.tab_mru.retain(|id| live.contains(id) && *id != active_id);
        self.tab_mru.push_front(active_id);
    }

    pub(crate) fn new_tab(&mut self, cx: &mut Context<Self>) {
        let label = crate::buffer::next_new_label_for(
            self.tabs.iter().map(|t| t.buffer.new_label.as_ref()),
        );
        let mut tab = Tab::empty();
        tab.buffer.new_label = Some(label);
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }

    pub(crate) fn close_tab_at(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }

        if !self.tabs[idx].buffer.dirty {
            self.finish_close(idx, window, cx);
            return;
        }

        let name = self.tabs[idx].buffer.display_short_name().to_string();
        cx.spawn_in(window, async move |this, cx| {
            let decision = show_dirty_close_dialog(name).await;
            match decision {
                CloseDecision::Cancel => {}
                CloseDecision::DontSave => {
                    this.update_in(cx, |w, window, cx| w.finish_close(idx, window, cx))
                        .ok();
                }
                CloseDecision::Save => {
                    let saved = save_tab_for_close(&this, cx, idx).await;
                    this.update_in(cx, |w, window, cx| match saved {
                        Ok(()) => w.finish_close(idx, window, cx),
                        Err(err) => {
                            w.set_status(format!("Save failed, keeping tab open: {err}"));
                            cx.notify();
                        }
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    /// Remove the tab at `idx`, push it onto the closed-tab stack, and adjust
    /// the active-tab index. Closes the window if the last tab is gone, or —
    /// when NPP `Exit on close the last tab` is unset — replaces the last tab
    /// with a fresh blank one so the window stays open.
    /// Caller has already settled any dirty-prompt save flow before calling.
    fn finish_close(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let removed = self.tabs.remove(idx);
        self.remember_closed(removed);
        self.settle_after_tab_removal(idx, window, cx);
    }

    /// Active-index, MRU, and empty-window settling after a tab has already
    /// been removed from `self.tabs` at `idx`. Shared between `finish_close`
    /// (Ctrl+W path — caller has just pushed to `closed_tabs`) and the
    /// cross-window Move path (caller has just handed the tab to a new
    /// window — does NOT push to `closed_tabs`).
    pub(crate) fn settle_after_tab_removal(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.is_empty() {
            if crate::settings::snapshot().tab_exit_on_last_close {
                // Persist the now-empty open-tab list (closed_tabs still carries
                // the buffer for Ctrl+Shift+T). Without this, the manifest keeps
                // claiming the just-closed file is still open, and next launch
                // would resurrect it as a phantom tab.
                self.save_session();
                window.remove_window();
                return;
            } else {
                // NPP "Exit on close the last tab" unchecked — keep the window
                // open with a fresh blank tab in place of the just-closed one.
                self.new_tab(cx);
                return;
            }
        }

        if idx < self.active {
            self.active -= 1;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }

    /// Append a tab handed over from another window (cross-window Move /
    /// Open in New Instance) and focus it. Does NOT push the source's
    /// welcome tab aside — the user sees `[welcome] [transferred]`, which
    /// matches NPP's "Open in New Instance" behaviour.
    pub(crate) fn adopt_transferred_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }

    pub(crate) fn close_active_tab(
        &mut self,
        _: &FileCloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let idx = self.active;
        self.close_tab_at(idx, window, cx);
    }

    /// Notepad++ `Ctrl+Shift+W` — close every tab in the window. Notepad++
    /// keeps the window open with one fresh blank doc (it never auto-quits
    /// just because every doc was closed); we match that. Dirty tabs trigger
    /// the same Save / Don't Save / Cancel prompt as a single close — Cancel
    /// at any tab aborts the entire Close All so partial state is never lost.
    pub(crate) fn close_all_tabs(
        &mut self,
        _: &FileCloseAllTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            // Walk rightmost-first so already-removed indices don't shift the
            // ones we still have to handle. Cancel at any tab aborts the
            // entire batch — tabs already removed stay removed (matches
            // Notepad++).
            loop {
                let next = this
                    .update(cx, |w, _| {
                        if w.tabs.is_empty() {
                            return None;
                        }
                        let idx = w.tabs.len() - 1;
                        let dirty = w.tabs[idx].buffer.dirty;
                        let name = w.tabs[idx].buffer.display_short_name().to_string();
                        Some((idx, dirty, name))
                    })
                    .ok()
                    .flatten();
                let Some((idx, dirty, name)) = next else { break };

                if dirty {
                    match show_dirty_close_dialog(name).await {
                        CloseDecision::Cancel => {
                            this.update(cx, |w, cx| {
                                w.set_status("Close All cancelled".to_string());
                                cx.notify();
                            })
                            .ok();
                            return;
                        }
                        CloseDecision::DontSave => {}
                        CloseDecision::Save => {
                            if let Err(err) = save_tab_for_close(&this, cx, idx).await {
                                this.update(cx, |w, cx| {
                                    w.set_status(format!(
                                        "Close All aborted — save failed: {err}"
                                    ));
                                    cx.notify();
                                })
                                .ok();
                                return;
                            }
                        }
                    }
                }

                let last = this
                    .update(cx, |w, _| {
                        if idx >= w.tabs.len() {
                            return true;
                        }
                        let removed = w.tabs.remove(idx);
                        w.remember_closed(removed);
                        w.tabs.is_empty()
                    })
                    .unwrap_or(true);
                if last {
                    break;
                }
            }

            this.update(cx, |w, cx| {
                let label = crate::buffer::next_new_label_for(
                    w.tabs.iter().map(|t| t.buffer.new_label.as_ref()),
                );
                let mut tab = Tab::empty();
                tab.buffer.new_label = Some(label);
                w.tabs.push(tab);
                w.active = 0;
                w.touch_mru();
                w.last_lines.clear();
                w.caret_visible = true;
                w.save_session();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Close every tab whose `session_id` is in `targets`, prompting for
    /// dirty saves with the same Save / Don't Save / Cancel flow as
    /// `close_all_tabs`. Cancel at any tab aborts the batch — tabs already
    /// removed stay removed. Used by the tab right-click menu's Close All
    /// BUT This / Close All to the Left / Close All to the Right / Close
    /// All Unchanged entries. Final fallback if every tab gets closed: a
    /// fresh blank tab is left behind so the window doesn't disappear.
    fn close_tab_batch(
        &mut self,
        targets: Vec<uuid::Uuid>,
        leave_blank_if_empty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            for id in targets.into_iter().rev() {
                let next = this
                    .update(cx, |w, _| {
                        let idx = w.tabs.iter().position(|t| t.session_id == id)?;
                        let dirty = w.tabs[idx].buffer.dirty;
                        let name = w.tabs[idx].buffer.display_short_name().to_string();
                        Some((idx, dirty, name))
                    })
                    .ok()
                    .flatten();
                let Some((idx, dirty, name)) = next else { continue };

                if dirty {
                    match show_dirty_close_dialog(name).await {
                        CloseDecision::Cancel => {
                            this.update(cx, |w, cx| {
                                w.set_status("Close cancelled");
                                cx.notify();
                            })
                            .ok();
                            return;
                        }
                        CloseDecision::DontSave => {}
                        CloseDecision::Save => {
                            if let Err(err) = save_tab_for_close(&this, cx, idx).await {
                                this.update(cx, |w, cx| {
                                    w.set_status(format!("Close aborted — save failed: {err}"));
                                    cx.notify();
                                })
                                .ok();
                                return;
                            }
                        }
                    }
                }

                this.update(cx, |w, _| {
                    let Some(idx) = w.tabs.iter().position(|t| t.session_id == id) else {
                        return;
                    };
                    let removed = w.tabs.remove(idx);
                    w.remember_closed(removed);
                    if idx < w.active {
                        w.active = w.active.saturating_sub(1);
                    } else if w.active >= w.tabs.len() {
                        w.active = w.tabs.len().saturating_sub(1);
                    }
                })
                .ok();
            }

            this.update_in(cx, |w, window, cx| {
                if w.tabs.is_empty() {
                    if leave_blank_if_empty {
                        let label = crate::buffer::next_new_label_for(
                            w.tabs.iter().map(|t| t.buffer.new_label.as_ref()),
                        );
                        let mut tab = Tab::empty();
                        tab.buffer.new_label = Some(label);
                        w.tabs.push(tab);
                        w.active = 0;
                    } else {
                        // Same reason as finish_close: persist the cleared
                        // tab list before tearing the window down.
                        w.save_session();
                        window.remove_window();
                        return;
                    }
                }
                w.touch_mru();
                w.last_lines.clear();
                w.caret_visible = true;
                w.save_session();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Notepad++ Close Multiple Tabs ▸ Close All BUT This. Closes every tab
    /// except the one whose menu was invoked.
    pub(crate) fn close_all_but(
        &mut self,
        keep_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keep_idx >= self.tabs.len() {
            return;
        }
        let keep_id = self.tabs[keep_idx].session_id;
        let targets: Vec<uuid::Uuid> = self
            .tabs
            .iter()
            .filter(|t| t.session_id != keep_id)
            .map(|t| t.session_id)
            .collect();
        self.close_tab_batch(targets, true, window, cx);
    }

    /// Close all tabs to the left of `pivot_idx` (exclusive).
    pub(crate) fn close_all_to_left(
        &mut self,
        pivot_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if pivot_idx == 0 || pivot_idx >= self.tabs.len() {
            return;
        }
        let targets: Vec<uuid::Uuid> = self.tabs[..pivot_idx]
            .iter()
            .map(|t| t.session_id)
            .collect();
        self.close_tab_batch(targets, true, window, cx);
    }

    /// Close all tabs to the right of `pivot_idx` (exclusive).
    pub(crate) fn close_all_to_right(
        &mut self,
        pivot_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if pivot_idx + 1 >= self.tabs.len() {
            return;
        }
        let targets: Vec<uuid::Uuid> = self.tabs[pivot_idx + 1..]
            .iter()
            .map(|t| t.session_id)
            .collect();
        self.close_tab_batch(targets, true, window, cx);
    }

    /// Close every tab whose buffer is clean (not dirty). Notepad++'s "Close
    /// All Unchanged" — typically used after a long editing session where
    /// the user only wants the tabs they actually touched.
    pub(crate) fn close_all_unchanged(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<uuid::Uuid> = self
            .tabs
            .iter()
            .filter(|t| !t.buffer.dirty)
            .map(|t| t.session_id)
            .collect();
        self.close_tab_batch(targets, true, window, cx);
    }

    /// Pull `idx` out of the strip and reinsert at position 0. Active-tab
    /// index follows the move so the user keeps looking at the same buffer.
    pub(crate) fn move_tab_to_start(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx == 0 || idx >= self.tabs.len() {
            return;
        }
        self.move_tab(idx, 0, cx);
    }

    /// Pull `idx` to the rightmost position.
    pub(crate) fn move_tab_to_end(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len().saturating_sub(1) {
            return;
        }
        let last = self.tabs.len() - 1;
        self.move_tab(idx, last, cx);
    }

    /// Apply / clear an NPP color stripe on tab `idx`. `color = None` removes
    /// the stripe.
    pub(crate) fn set_tab_color(&mut self, idx: usize, color: Option<u8>, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        let normalized = color.filter(|&c| (1..=5).contains(&c));
        self.tabs[idx].buffer.tab_color = normalized;
        self.save_session();
        cx.notify();
    }

    /// Toggle the Read-Only flag on tab `idx`. `on` lets callers force an
    /// explicit state when the menu user clicked the "Clear Read-Only Flag"
    /// row vs the toggle row.
    pub(crate) fn set_tab_read_only(&mut self, idx: usize, on: bool, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs[idx].buffer.read_only = on;
        self.save_session();
        cx.notify();
    }

    /// "Clear Read-Only Flag" — toggle the in-memory flag off AND chmod the
    /// underlying file writable so re-saving doesn't trip an OS permission
    /// error. No-op for untitled buffers.
    pub(crate) fn clear_tab_read_only_flag(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs[idx].buffer.read_only = false;
        if let Some(path) = self.tabs[idx].buffer.file_path.clone() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&path) {
                    let mut perms = meta.permissions();
                    let mode = perms.mode() | 0o200;
                    perms.set_mode(mode);
                    if let Err(err) = std::fs::set_permissions(&path, perms) {
                        self.set_status(format!(
                            "Cleared in-memory flag; chmod failed: {err}"
                        ));
                    } else {
                        self.set_status("Read-only flag cleared");
                    }
                }
            }
            #[cfg(windows)]
            {
                if let Ok(meta) = std::fs::metadata(&path) {
                    let mut perms = meta.permissions();
                    perms.set_readonly(false);
                    if let Err(err) = std::fs::set_permissions(&path, perms) {
                        self.set_status(format!(
                            "Cleared in-memory flag; OS attribute clear failed: {err}"
                        ));
                    } else {
                        self.set_status("Read-only flag cleared");
                    }
                }
            }
            #[cfg(not(any(unix, windows)))]
            {
                let _ = path;
                self.set_status("Read-only flag cleared (in-memory only)");
            }
        }
        self.save_session();
        cx.notify();
    }

    /// Reload tab `idx` from disk. Drops in-memory edits — caller is
    /// expected to have warned the user via the menu (NPP doesn't dirty-
    /// prompt on Reload either; symmetry).
    pub(crate) fn reload_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        let Some(path) = self.tabs[idx].buffer.file_path.clone() else {
            self.set_status("Cannot reload — tab has no file path");
            cx.notify();
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let (enc, raw) = crate::text::encoding::decode(&bytes);
                let line_ending = crate::text::line_ending::LineEnding::detect(&raw);
                let normalized = crate::text::line_ending::normalize_to_lf(raw, line_ending);
                let rope = ropey::Rope::from_str(&normalized);
                let language = crate::syntax::language_for_path(&path)
                    .or_else(|| crate::syntax::language_for_shebang(&normalized));
                let buf = &mut self.tabs[idx].buffer;
                buf.rope = rope;
                buf.invalidate_flat_text();
                buf.dirty = false;
                buf.encoding = enc;
                buf.line_ending = line_ending;
                buf.language = language;
                buf.bookmarks.clear();
                buf.marks.clear();
                buf.reparse_highlights();
                self.tabs[idx].selection = crate::buffer::Selection::default();
                self.tabs[idx].undo_stack.clear();
                self.tabs[idx].redo_stack.clear();
                self.tabs[idx].scroll_top = gpui::px(0.);
                self.tabs[idx].scroll_left = gpui::px(0.);
                self.set_status(format!("Reloaded {}", path.display()));
            }
            Err(err) => {
                self.set_status(format!("Reload failed: {err}"));
            }
        }
        self.last_lines.clear();
        self.save_session();
        cx.notify();
    }

    /// Rename tab `idx`'s underlying file on disk via `std::fs::rename`.
    /// `new_name` is the basename only (parent dir stays the same). Updates
    /// the buffer's `file_path` on success and refreshes the language / MRU.
    /// Untitled tabs without a path are a no-op (caller should disable the
    /// menu entry).
    pub(crate) fn rename_tab_file(
        &mut self,
        idx: usize,
        new_name: String,
        cx: &mut Context<Self>,
    ) {
        if idx >= self.tabs.len() {
            return;
        }
        let Some(old_path) = self.tabs[idx].buffer.file_path.clone() else {
            self.set_status("Cannot rename — tab has no file path");
            cx.notify();
            return;
        };
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            self.set_status("Rename cancelled — empty name");
            cx.notify();
            return;
        }
        // Reject path separators so a Rename can't accidentally move the file.
        if trimmed.contains('/') || trimmed.contains('\\') {
            self.set_status("Rename failed — name contains path separator");
            cx.notify();
            return;
        }
        let parent = match old_path.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                self.set_status("Rename failed — file has no parent dir");
                cx.notify();
                return;
            }
        };
        let new_path = parent.join(trimmed);
        if new_path == old_path {
            return;
        }
        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                let buf = &mut self.tabs[idx].buffer;
                buf.file_path = Some(new_path.clone());
                buf.language = crate::syntax::language_for_path(&new_path).or(buf.language);
                buf.reparse_highlights();
                self.set_status(format!("Renamed to {}", new_path.display()));
            }
            Err(err) => {
                self.set_status(format!("Rename failed: {err}"));
            }
        }
        self.save_session();
        cx.notify();
    }

    /// Move the underlying file to the OS trash via the `trash` crate, then
    /// remove the tab from the strip. No dirty-prompt — Notepad++ doesn't
    /// double-confirm: the menu already showed "Move to Trash" and the tab's
    /// next state is gone.
    pub(crate) fn move_tab_to_trash(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if idx >= self.tabs.len() {
            return;
        }
        let Some(path) = self.tabs[idx].buffer.file_path.clone() else {
            self.set_status("Cannot move to trash — tab has no file path");
            cx.notify();
            return;
        };
        match trash::delete(&path) {
            Ok(()) => {
                self.set_status(format!("Moved {} to trash", path.display()));
                // Drop the tab without remembering it on the closed-tab stack
                // — the file's gone, restoring the buffer would orphan it.
                let removed = self.tabs.remove(idx);
                drop(removed);
                if self.tabs.is_empty() {
                    // Same reason as finish_close: persist the cleared
                    // tab list before tearing the window down.
                    self.save_session();
                    window.remove_window();
                    return;
                }
                if idx < self.active {
                    self.active = self.active.saturating_sub(1);
                } else if self.active >= self.tabs.len() {
                    self.active = self.tabs.len() - 1;
                }
                self.touch_mru();
                self.last_lines.clear();
                self.caret_visible = true;
                self.save_session();
            }
            Err(err) => {
                self.set_status(format!("Move to trash failed: {err}"));
            }
        }
        cx.notify();
    }

    /// Save tab `idx` to its current path. If untitled, the caller is
    /// expected to route to Save As — this helper asserts a path is set so
    /// the menu's "Save" entry can be greyed out for untitled.
    pub(crate) fn save_tab_at(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs[idx].buffer.file_path.is_none() {
            self.set_status("Tab has no path — use Save As");
            cx.notify();
            return;
        }
        let prev = self.active;
        self.active = idx;
        match self.write_active_to_disk() {
            Ok(()) => {
                let name = self.tabs[idx].buffer.display_short_name().to_string();
                self.set_status(format!("Saved {name}"));
            }
            Err(err) => {
                self.set_status(format!("Save failed: {err}"));
            }
        }
        self.active = prev;
        self.save_session();
        cx.notify();
    }

    fn remember_closed(&mut self, tab: Tab) {
        if tab.buffer.rope.len_bytes() == 0 && tab.buffer.file_path.is_none() {
            return;
        }
        self.closed_tabs.push(ClosedTab {
            buffer: tab.buffer,
            session_id: tab.session_id,
        });
        self.trim_closed_tabs();
    }

    fn trim_closed_tabs(&mut self) {
        let overflow = self.closed_tabs.len().saturating_sub(CLOSED_TAB_LIMIT);
        if overflow > 0 {
            self.closed_tabs.drain(0..overflow);
        }
    }

    /// Notepad++ `Ctrl+Shift+T` — pop the most-recently-closed tab back onto
    /// the strip at the rightmost position and focus it.
    pub(crate) fn restore_closed_tab(
        &mut self,
        _: &FileRestoreClosedTab,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(closed) = self.closed_tabs.pop() else {
            return;
        };
        let mut tab = Tab::empty();
        tab.buffer = closed.buffer;
        tab.session_id = closed.session_id;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }

    /// Visual-order tab cycling (NPP `Ctrl+PageUp` / `Ctrl+PageDown`). Walks
    /// the tab strip in order, wrapping at the ends. Independent of MRU
    /// history — pure positional navigation.
    fn cycle_tab_visual(&mut self, delta: i32, cx: &mut Context<Self>) {
        if !crate::settings::snapshot().document_switcher_enabled {
            return;
        }
        let len = self.tabs.len();
        if len < 2 {
            return;
        }
        let cur = self.active as i32;
        let next = ((cur + delta).rem_euclid(len as i32)) as usize;
        self.set_active_tab(next, cx);
    }

    /// Most-recently-used tab cycling (NPP `Ctrl+Tab` / `Ctrl+Shift+Tab`).
    /// Steps through `tab_mru` from the position of the currently-active tab
    /// without re-fronting the visited tab — so a held-Ctrl Tab-tap sequence
    /// can walk through several recently-used tabs in MRU order. The next
    /// external action (mouse click, edit, goto-N) commits the chosen tab via
    /// `touch_mru`. Stale session_ids (closed tabs) are pruned and any open
    /// tab missing from the MRU is back-filled at the end before walking.
    fn cycle_tab_mru(&mut self, delta: i32, cx: &mut Context<Self>) {
        let snap = crate::settings::snapshot();
        if !snap.document_switcher_enabled {
            return;
        }
        // NPP-parity: when "Enable MRU behaviour" is off, Ctrl+Tab falls
        // through to positional cycling (same handler as Ctrl+PageDown/Up).
        if !snap.document_switcher_mru {
            self.cycle_tab_visual(delta, cx);
            return;
        }
        let live: HashSet<Uuid> = self.tabs.iter().map(|t| t.session_id).collect();
        self.tab_mru.retain(|id| live.contains(id));
        for tab in &self.tabs {
            if !self.tab_mru.iter().any(|id| *id == tab.session_id) {
                self.tab_mru.push_back(tab.session_id);
            }
        }
        let len = self.tab_mru.len();
        if len < 2 {
            return;
        }
        let active_id = self.tabs[self.active].session_id;
        let cur_pos = self
            .tab_mru
            .iter()
            .position(|id| *id == active_id)
            .unwrap_or(0);
        let next_pos = ((cur_pos as i32) + delta).rem_euclid(len as i32) as usize;
        let target = self.tab_mru[next_pos];
        let Some(target_idx) = self.tabs.iter().position(|t| t.session_id == target) else {
            return;
        };
        if target_idx == self.active {
            return;
        }
        self.active = target_idx;
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }

    pub(crate) fn next_tab(&mut self, _: &TabNext, _w: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab_mru(1, cx);
    }

    pub(crate) fn prev_tab(&mut self, _: &TabPrev, _w: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab_mru(-1, cx);
    }

    pub(crate) fn next_tab_sequential(
        &mut self,
        _: &TabNextSequential,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab_visual(1, cx);
    }

    pub(crate) fn prev_tab_sequential(
        &mut self,
        _: &TabPrevSequential,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab_visual(-1, cx);
    }

    /// Jump to the Nth tab (zero-indexed). Notepad++ binds these to
    /// `Ctrl+NumPad1..9`; we use the top-row digits so Mac keyboards without
    /// a NumPad still get the shortcut. Out-of-range indices are no-ops.
    fn goto_tab_index(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.tabs.len() {
            self.set_active_tab(idx, cx);
        }
    }

    pub(crate) fn goto_tab_1(&mut self, _: &TabGoto1, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(0, cx);
    }
    pub(crate) fn goto_tab_2(&mut self, _: &TabGoto2, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(1, cx);
    }
    pub(crate) fn goto_tab_3(&mut self, _: &TabGoto3, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(2, cx);
    }
    pub(crate) fn goto_tab_4(&mut self, _: &TabGoto4, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(3, cx);
    }
    pub(crate) fn goto_tab_5(&mut self, _: &TabGoto5, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(4, cx);
    }
    pub(crate) fn goto_tab_6(&mut self, _: &TabGoto6, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(5, cx);
    }
    pub(crate) fn goto_tab_7(&mut self, _: &TabGoto7, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(6, cx);
    }
    pub(crate) fn goto_tab_8(&mut self, _: &TabGoto8, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(7, cx);
    }
    pub(crate) fn goto_tab_9(&mut self, _: &TabGoto9, _w: &mut Window, cx: &mut Context<Self>) {
        self.goto_tab_index(8, cx);
    }

    /// Reorder tabs: pull the tab at `from` and reinsert it at `to`. Notepad++
    /// drag-reorder semantics — the dragged tab snaps into the slot occupied
    /// by the drop target. The previously-active tab stays active (its index
    /// is updated to wherever the move lands it). Out-of-range indices and
    /// no-op moves (`from == to`) are silent no-ops.
    pub(crate) fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let len = self.tabs.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);

        // Track the active tab through the shuffle so the user keeps looking
        // at the same buffer. Three cases: the moved tab was active, the
        // active tab sat between source and destination (gets pushed by 1),
        // or active sat outside the [from,to] span (unchanged).
        self.active = if self.active == from {
            to
        } else if from < self.active && self.active <= to {
            self.active - 1
        } else if to <= self.active && self.active < from {
            self.active + 1
        } else {
            self.active
        };

        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session();
        cx.notify();
    }
}
