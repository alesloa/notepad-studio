// Tab management — open, close, restore, cycle, and direct goto-N actions.

use std::collections::HashSet;

use gpui::{Context, FocusHandle, Window};
use uuid::Uuid;

use crate::actions::{
    FileCloseAllTabs, FileCloseTab, FileRestoreClosedTab, TabGoto1, TabGoto2, TabGoto3, TabGoto4,
    TabGoto5, TabGoto6, TabGoto7, TabGoto8, TabGoto9, TabNext, TabNextSequential, TabPrev,
    TabPrevSequential,
};
use crate::tab::{CLOSED_TAB_LIMIT, ClosedTab, CloseDecision, Tab};
use crate::workspace::Workspace;
use crate::io::{save_tab_for_close, show_dirty_close_dialog};

struct ClosePrompts {
    #[cfg(test)]
    responses: Option<close_async_tests::PromptSource>,
}

impl ClosePrompts {
    fn capture() -> Self {
        Self { #[cfg(test)] responses: close_async_tests::prompt_source() }
    }

    fn confirm(&self, name: String) -> impl std::future::Future<Output = CloseDecision> {
        #[cfg(test)]
        let response = self.responses.as_ref().map(|source| {
            source.lock().ok().and_then(|mut queue| queue.pop_front())
        });
        async move {
            #[cfg(test)]
            if let Some(response) = response {
                return match response {
                    Some(response) => response.await.unwrap_or(CloseDecision::Cancel),
                    None => CloseDecision::Cancel,
                };
            }
            show_dirty_close_dialog(name).await
        }
    }
}

impl Workspace {
    pub(crate) fn set_active_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() { return; }
        if idx == self.active {
            self.activate_initial_syntax(cx);
            return;
        }
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        self.active = idx;
        self.activate_initial_syntax(cx);
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
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
        self.sync_source_input_tab();
        if self.autocomplete.as_ref().is_some_and(|state| {
            !self.tabs.get(self.active).is_some_and(|tab| tab.session_id == state.tab_id)
        }) {
            self.autocomplete = None;
            self.autocomplete_blur_subscription = None;
            self.autocomplete_activation_subscription = None;
        }
        let Some(active_id) = self.tabs.get(self.active).map(|t| t.session_id) else {
            self.tab_mru.clear();
            return;
        };
        let live: HashSet<Uuid> = self.tabs.iter().map(|t| t.session_id).collect();
        self.tab_mru.retain(|id| live.contains(id) && *id != active_id);
        self.tab_mru.push_front(active_id);
    }

    pub(crate) fn new_tab(&mut self, cx: &mut Context<Self>) {
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        self.append_blank_tab(cx);
    }

    // Closing has already settled drafts and may have removed the last tab.
    // Create its replacement without asking an absent source to commit again.
    fn append_blank_tab(&mut self, cx: &mut Context<Self>) {
        let label = crate::buffer::next_new_label_for(
            self.tabs.iter().map(|t| t.buffer.new_label.as_ref()),
        );
        let mut tab = Tab::empty();
        tab.buffer.new_label = Some(label);
        self.tabs.push(tab);
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        self.active = self.tabs.len() - 1;
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
        cx.notify();
    }

    pub(crate) fn close_tab_at(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }

        let id = self.tabs[idx].session_id;
        if crate::ui::csv_doc::actions::commit_cell_edit_for_tab(self, id, cx).is_err() { return; }
        if !self.tabs[idx].buffer.dirty {
            self.finish_close(idx, window, cx);
            return;
        }

        // The prompt is not modal: tabs can close or move while it is up, so
        // `idx` goes stale across the await. Re-find the tab by its id.
        let Some(tab) = self.tabs.get(idx) else { return };
        let id = tab.session_id;
        let name = tab.buffer.display_short_name().to_string();
        let snapshot = DocumentSnapshot::capture(tab);
        let confirmation = ClosePrompts::capture().confirm(name);
        cx.spawn_in(window, async move |this, cx| {
            let decision = confirmation.await;
            match decision {
                CloseDecision::Cancel => {}
                CloseDecision::DontSave => {
                    this.update_in(cx, |w, window, cx| {
                        if let Ok(Some(idx)) = w.close_confirmation_index(id, &snapshot, false, cx) {
                            w.finish_close(idx, window, cx);
                        }
                    })
                        .ok();
                }
                CloseDecision::Save => {
                    let Some(idx) = this
                        .update(cx, |w, cx| w.close_confirmation_index(id, &snapshot, false, cx))
                        .ok()
                        .and_then(Result::ok)
                        .flatten()
                    else {
                        return;
                    };
                    let saved = save_tab_for_close(&this, cx, idx).await;
                    this.update_in(cx, |w, window, cx| match saved {
                        Ok(()) => {
                            if let Ok(Some(idx)) = w.close_confirmation_index(id, &snapshot, true, cx) {
                                w.finish_close(idx, window, cx);
                            }
                        }
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

    fn close_confirmation_index(
        &mut self, id: Uuid, snapshot: &DocumentSnapshot, saved: bool, cx: &mut Context<Self>,
    ) -> Result<Option<usize>, ()> {
        let Some(idx) = tab_index_of(&self.tabs, id) else { return Ok(None) };
        let Some(tab) = self.tabs.get(idx) else { return Ok(None) };
        // Cell text can change without advancing the source revision.
        if tab.csv_draft.is_some() || self.csv_models.get(&id).is_some_and(|model| {
            model.cell_input.is_some() || model.detached_draft.is_some()
        }) {
            self.set_status("Close cancelled: a CSV cell edit started while confirmation was open; commit or cancel it before closing");
            cx.notify();
            return Err(());
        }
        // Save As may legitimately change the path. After a successful save,
        // close only if no further unsaved content has appeared.
        if if saved { tab.buffer.dirty || tab.buffer.loading.is_some() } else { !snapshot.matches(tab) } {
            self.set_status("Close cancelled: document changed while confirmation was open; close again to confirm the current contents");
            cx.notify();
            return Err(());
        }
        Ok(Some(idx))
    }

    /// Remove the tab at `idx`, push it onto the closed-tab stack, and adjust
    /// the active-tab index. Closes the window if the last tab is gone, or —
    /// when NPP `Exit on close the last tab` is unset — replaces the last tab
    /// with a fresh blank one so the window stays open.
    /// Caller has already settled any dirty-prompt save flow before calling.
    fn finish_close(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
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
        self.retire_closed_reloads();
        self.retire_closed_syntax_refresh(cx);
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        if self.tabs.is_empty() {
            if crate::settings::snapshot().tab_exit_on_last_close {
                self.close_window_after_last_tab(window, cx);
                return;
            } else {
                // NPP "Exit on close the last tab" unchecked — keep the window
                // open with a fresh blank tab in place of the just-closed one.
                self.append_blank_tab(cx);
                return;
            }
        }

        self.active = active_after_removal(self.active, idx, self.tabs.len());
        self.activate_initial_syntax(cx);
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
        cx.notify();
    }

    /// Persist the now-empty open-tab list, then tear the window down.
    /// Without the save, the manifest keeps claiming the just-closed file is
    /// still open and the next launch resurrects it as a phantom tab
    /// (closed_tabs still carries the buffer for Ctrl+Shift+T).
    ///
    /// An open Preferences or Text Tools window holds this entity, so it can
    /// outlive the window. A blank tab is parked in the strip so late timers
    /// (the debounced reparse) and async closures still find an active tab,
    /// and `window_closed` stops them persisting it.
    fn close_window_after_last_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.save_session_now() {
            park_closed_strip(&mut self.tabs, &mut self.active);
            self.set_status(format!("Window close cancelled: session recovery could not be saved: {err}; save your documents or retry closing"));
            cx.notify();
            return;
        }
        self.release_session_window();
        park_closed_strip(&mut self.tabs, &mut self.active);
        window.remove_window();
    }

    /// Append a tab handed over from another window (cross-window Move /
    /// Open in New Instance) and focus it. Does NOT push the source's
    /// welcome tab aside — the user sees `[welcome] [transferred]`, which
    /// matches NPP's "Open in New Instance" behaviour.
    pub(crate) fn adopt_transferred_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let pending = tab.initial_syntax_pending;
        self.tabs.push(tab);
        if pending { self.seed_initial_syntax(self.tabs.len().saturating_sub(1)); }
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        self.active = self.tabs.len() - 1;
        self.activate_initial_syntax(cx);
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
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
        // The prompt is asynchronous: documents opened after this command
        // belong to the next interaction, not to the captured close batch.
        let targets = self.tabs.iter().map(|tab| tab.session_id).collect();
        self.close_tab_batch(targets, true, window, cx);
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
            if self.tabs.is_empty() { self.append_blank_tab(cx); }
            return;
        }
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        let prompts = ClosePrompts::capture();
        cx.spawn_in(window, async move |this, cx| {
            for id in targets.into_iter().rev() {
                let next = this
                    .update(cx, |w, cx| {
                        if tab_index_of(&w.tabs, id).is_none() { return Ok(None); }
                        crate::ui::csv_doc::actions::commit_cell_edit_for_tab(w, id, cx)?;
                        let tab = w.tabs.iter().find(|tab| tab.session_id == id).ok_or("tab gone")?;
                        let name = tab.buffer.display_short_name().to_string();
                        Ok::<_, String>(Some((tab.buffer.dirty, name, DocumentSnapshot::capture(tab))))
                    });
                let Some((dirty, name, snapshot)) = (match next { Ok(Ok(next)) => next, _ => return }) else { continue };
                let mut saved = false;

                if dirty {
                    match prompts.confirm(name).await {
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
                            // Re-find the tab: the strip may have changed
                            // while the (non-modal) prompt was up.
                            let Some(idx) = this
                                .update(cx, |w, cx| w.close_confirmation_index(id, &snapshot, false, cx))
                                .ok()
                            else { return };
                            let idx = match idx { Ok(Some(idx)) => idx, Ok(None) => continue, Err(()) => return };
                            if let Err(err) = save_tab_for_close(&this, cx, idx).await {
                                this.update(cx, |w, cx| {
                                    w.set_status(format!("Close aborted — save failed: {err}"));
                                    cx.notify();
                                })
                                .ok();
                                return;
                            }
                            saved = true;
                        }
                    }
                }

                let closed = this.update_in(cx, |w, window, cx| {
                    let idx = match w.close_confirmation_index(id, &snapshot, saved, cx) {
                        Ok(Some(idx)) => idx,
                        Ok(None) => return true,
                        Err(()) => return false,
                    };
                    let removed = w.tabs.remove(idx);
                    w.retire_closed_reloads();
                    w.retire_closed_syntax_refresh(cx);
                    w.remember_closed(removed);
                    w.active = active_after_removal(w.active, idx, w.tabs.len());
                    if w.tabs.is_empty() {
                        // No render or observer may see an empty strip between
                        // this removal and the batch's final update.
                        if leave_blank_if_empty { w.append_blank_tab(cx); }
                        else { w.close_window_after_last_tab(window, cx); }
                    } else {
                        w.activate_initial_syntax(cx);
                        w.touch_mru();
                        w.last_lines.clear();
                        cx.notify();
                    }
                    true
                })
                .unwrap_or(false);
                if !closed { return; }
            }

            this.update_in(cx, |w, window, cx| {
                if w.tabs.is_empty() {
                    if leave_blank_if_empty {
                        w.append_blank_tab(cx);
                    } else {
                        w.close_window_after_last_tab(window, cx);
                        return;
                    }
                }
                w.activate_initial_syntax(cx);
                w.touch_mru();
                w.last_lines.clear();
                w.caret_visible = true;
                w.save_session(cx);
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
        self.save_session(cx);
        cx.notify();
    }

    /// Toggle the Read-Only flag on tab `idx`. `on` lets callers force an
    /// explicit state when the menu user clicked the "Clear Read-Only Flag"
    /// row vs the toggle row.
    pub(crate) fn set_tab_read_only(&mut self, idx: usize, on: bool, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        if on && idx == self.active { self.close_autocomplete(cx); self.close_function_hint(cx); }
        if on { self.cancel_regex_replacement(); }
        self.tabs[idx].buffer.read_only = on;
        self.save_session(cx);
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
        self.save_session(cx);
        cx.notify();
    }

    // PowerEditor/src/NppIO.cpp:588 — doReload confirms discarding dirty edits.
    pub(crate) fn reload_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        if tab.buffer.loading.is_some() {
            self.set_status("Cannot reload while the file is still loading");
            cx.notify();
            return;
        }
        if tab.buffer.file_path.is_none() {
            self.set_status("Cannot reload — tab has no file path");
            cx.notify();
            return;
        }
        if tab.csv_draft.is_some() {
            self.set_status("Open the recovered CSV grid, copy any needed cell text, then commit or cancel the edit before reloading");
            cx.notify();
            return;
        }
        let id = tab.session_id;
        let csv_focus = self.csv_focus_for_reload(idx, window);
        if crate::ui::csv_doc::actions::commit_cell_edit_for_tab(self, id, cx).is_err() {
            return;
        }
        let Some(idx) = tab_index_of(&self.tabs, id) else { return };
        if idx == self.active { self.close_autocomplete(cx); self.close_function_hint(cx); }
        self.cancel_regex_replacement();
        let Some(tab) = self.tabs.get(idx) else { return };
        if !tab.buffer.dirty {
            self.finish_reload(id, false, csv_focus, window, cx);
            return;
        }
        let name = tab.buffer.display_short_name().to_string();
        let snapshot = DocumentSnapshot::capture(tab);
        let confirmation = super::file_tab_actions::confirm_reload(name);
        cx.spawn_in(window, async move |this, cx| {
            if confirmation.await {
                this.update_in(cx, |w, window, cx| {
                    if w.reload_confirmation_is_current(id, &snapshot, cx) {
                        w.finish_reload(id, true, csv_focus, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn reload_confirmation_is_current(
        &mut self,
        id: Uuid,
        snapshot: &DocumentSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.iter().find(|tab| tab.session_id == id) else {
            return false;
        };
        // A pending cell's text does not advance the source revision until commit.
        if tab.csv_draft.is_some() || self.csv_models.get(&id).is_some_and(|model| {
            model.cell_input.is_some() || model.detached_draft.is_some()
        }) {
            self.set_status("Reload cancelled: a CSV cell edit started while confirmation was open; commit or cancel it before reloading");
            cx.notify();
            return false;
        }
        if !snapshot.matches(tab) {
            self.set_status("Reload cancelled: document changed while confirmation was open; reload again to confirm the current contents");
            cx.notify();
            return false;
        }
        true
    }

    fn finish_reload(
        &mut self,
        id: Uuid,
        confirmed: bool,
        csv_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_reload(id, confirmed, csv_focus, window, cx);
    }

    /// Rename tab `idx`'s underlying file without replacing an existing path.
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
        if trimmed.contains('\0') || old_path.to_string_lossy().contains('\0') {
            self.set_status("Rename failed — file path contains a null character");
            cx.notify();
            return;
        }
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
        // renamore uses native no-replace rename on Windows, macOS and Linux.
        // Unsupported filesystems refuse; never fall back to an overwrite.
        match renamore::rename_exclusive(&old_path, &new_path) {
            Ok(()) => {
                let Some(tab) = self.tabs.get_mut(idx) else { return; };
                let id = tab.session_id;
                let buf = &mut tab.buffer;
                buf.file_path = Some(new_path.clone());
                buf.language = crate::syntax::language_for_path(&new_path).or(buf.language);
                self.retire_syntax_highlights(id, cx);
                self.set_status(format!("Renamed to {}", new_path.display()));
                self.request_syntax_refresh_for_tab(id, std::time::Duration::ZERO, cx);
            }
            Err(err) => {
                self.set_status(format!("Rename failed: {err}"));
            }
        }
        self.save_session(cx);
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
                self.retire_closed_reloads();
                drop(removed);
                if self.tabs.is_empty() {
                    self.close_window_after_last_tab(window, cx);
                    return;
                }
                self.active = active_after_removal(self.active, idx, self.tabs.len());
                self.activate_initial_syntax(cx);
                self.touch_mru();
                self.last_lines.clear();
                self.caret_visible = true;
                self.save_session(cx);
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
        let id = self.tabs[idx].session_id;
        if crate::ui::csv_doc::actions::commit_cell_edit_for_tab(self, id, cx).is_err() { return; }
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
        self.save_session(cx);
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
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        let Some(closed) = self.closed_tabs.pop() else {
            return;
        };
        let recovered_raw = closed.buffer.rich_doc.is_none() && closed.buffer.new_label.as_ref()
            .is_some_and(|label| label.starts_with("Recovered raw source "));
        self.tabs.push(Tab::from_closed(closed));
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        self.active = self.tabs.len() - 1;
        self.touch_mru();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
        if recovered_raw {
            self.set_status("Reopened recovered raw source; Doc View recovery was missing or damaged. Save As keeps the original native document safe.");
        }
        if let Some(id) = self.tabs.get(self.active).map(|tab| tab.session_id) {
            self.retire_syntax_highlights(id, cx);
            self.request_syntax_refresh_for_tab(id, std::time::Duration::ZERO, cx);
        }
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
        if !crate::ui::csv_doc::actions::commit_cell_edit(self, cx) { return; }
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.cancel_regex_replacement();
        self.active = target_idx;
        self.activate_initial_syntax(cx);
        self.sync_source_input_tab();
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
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

        self.activate_initial_syntax(cx);
        self.last_lines.clear();
        self.caret_visible = true;
        self.save_session(cx);
        cx.notify();
    }
}

/// Position of the tab whose `session_id` is `id`, if it is still open.
fn tab_index_of(tabs: &[Tab], id: Uuid) -> Option<usize> {
    tabs.iter().position(|t| t.session_id == id)
}

// Confirmation applies to the captured document, not to later edits or Save As.
struct DocumentSnapshot {
    raw: crate::buffer::ContentRevision,
    rich: Option<crate::buffer::ContentRevision>,
    path: Option<std::path::PathBuf>,
    encoding: crate::text::encoding::Encoding,
    line_ending: crate::text::line_ending::LineEnding,
}

impl DocumentSnapshot {
    fn capture(tab: &Tab) -> Self {
        Self {
            raw: tab.buffer.raw_revision.clone(),
            rich: tab.buffer.rich_doc.as_ref().map(|payload| payload.revision.clone()),
            path: tab.buffer.file_path.clone(),
            encoding: tab.buffer.encoding,
            line_ending: tab.buffer.line_ending,
        }
    }

    fn matches(&self, tab: &Tab) -> bool {
        tab.buffer.loading.is_none()
            && self.raw == tab.buffer.raw_revision
            && self.rich.as_ref() == tab.buffer.rich_doc.as_ref().map(|payload| &payload.revision)
            && self.path == tab.buffer.file_path
            && self.encoding == tab.buffer.encoding
            && self.line_ending == tab.buffer.line_ending
    }
}

#[cfg(test)]
fn reload_tab_by_id(tabs: &mut [Tab], id: Uuid, confirmed: bool) -> Result<Option<usize>, String> {
    let Some(idx) = tab_index_of(tabs, id) else {
        return Ok(None);
    };
    let Some(tab) = tabs.get_mut(idx) else {
        return Ok(None);
    };
    if tab.buffer.dirty && !confirmed {
        return Ok(None);
    }
    reload_tab_from_disk(tab)?;
    Ok(Some(idx))
}

#[cfg(test)]
fn reload_tab_from_disk(tab: &mut Tab) -> Result<(), String> {
    if tab.buffer.loading.is_some() {
        return Err("file is still loading".to_string());
    }
    let path = tab.buffer.file_path.as_ref().ok_or("tab has no file path")?;
    let mut buffer = crate::io::read_buffer(path)?;
    buffer.read_only |= tab.buffer.read_only;
    buffer.tab_color = tab.buffer.tab_color;
    buffer.is_changelog = tab.buffer.is_changelog;
    let view_mode = tab.view_mode;
    crate::io::reset_tab_with(tab, buffer);
    tab.view_mode = view_mode;
    Ok(())
}

/// Active-tab index after the tab at `removed` has been taken out of a strip
/// that now holds `len` tabs. The same buffer stays active when it is still
/// open; otherwise its right neighbour (or the new last tab) takes over.
/// Returns 0 for an empty strip; callers refill or close the window then.
pub(crate) fn active_after_removal(active: usize, removed: usize, len: usize) -> usize {
    let shifted = if removed < active { active - 1 } else { active };
    shifted.min(len.saturating_sub(1))
}

/// Refill a strip whose window is being torn down so `tabs[active]` still
/// indexes a tab. The entity can outlive its window, and its timers and async
/// closures go through accessors that index the active tab.
fn park_closed_strip(tabs: &mut Vec<Tab>, active: &mut usize) {
    if tabs.is_empty() {
        tabs.push(Tab::empty());
    }
    *active = (*active).min(tabs.len().saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use super::{active_after_removal, park_closed_strip, reload_tab_by_id, reload_tab_from_disk};
    use crate::tab::Tab;

    #[test]
    fn reload_preserves_mixed_line_endings_and_encoding() {
        use crate::text::{encoding, line_ending::rope_to_bytes};
        let dir = crate::persist::TempDir::new("reload-mixed");
        for enc in [encoding::Encoding::Utf8, encoding::Encoding::Utf8Bom,
            encoding::Encoding::Utf16Le, encoding::Encoding::Utf16Be,
            encoding::Encoding::Latin1] {
            let path = dir.join("mixed.txt");
            let bytes = encoding::encode_checked("café\r\nsecond\nthird\rfourth", enc).expect("fixture");
            std::fs::write(&path, &bytes).expect("write fixture");
            let mut tab = Tab::empty();
            tab.buffer.file_path = Some(path);
            reload_tab_from_disk(&mut tab).expect("reload");
            let text = rope_to_bytes(&tab.buffer.rope, tab.buffer.line_ending);
            let saved = encoding::encode_checked(std::str::from_utf8(&text).expect("UTF-8"), tab.buffer.encoding).expect("encode");
            assert_eq!(saved, bytes, "{enc:?}");
        }
    }

    #[test]
    fn reload_malformed_utf16_is_read_only() {
        let dir = crate::persist::TempDir::new("reload-lossy");
        let path = dir.join("broken.txt");
        std::fs::write(&path, [0xff, 0xfe, 0x00, 0xd8]).expect("write fixture");
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path);
        reload_tab_from_disk(&mut tab).expect("reload");
        assert!(tab.buffer.read_only, "reload must protect bytes that could not be decoded");
    }

    #[test]
    fn reload_replaces_rich_document_payload() {
        let dir = crate::persist::TempDir::new("reload-rtf");
        let path = dir.join("document.rtf");
        std::fs::write(&path, b"{\\rtf1 old}").expect("write fixture");
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path.clone());
        tab.buffer.rich_doc = Some(crate::rich_doc::open(&path).expect("open"));
        std::fs::write(&path, b"{\\rtf1 changed}").expect("update fixture");
        reload_tab_from_disk(&mut tab).expect("reload");
        assert_eq!(tab.buffer.rich_doc.as_ref().expect("payload").block_text(0), "changed");
    }

    #[test]
    fn reload_clears_positions_and_open_transactions() {
        let dir = crate::persist::TempDir::new("reload-state");
        let path = dir.join("short.txt");
        std::fs::write(&path, "é").expect("write fixture");
        let mut tab = Tab::empty();
        let id = tab.session_id;
        tab.buffer.file_path = Some(path);
        tab.buffer.read_only = true;
        tab.buffer.tab_color = Some(3);
        tab.view_mode = crate::tab::TabViewMode::Edit;
        tab.begin_select_marker = Some(40);
        tab.block_selection = Some(crate::buffer::BlockSelection {
            anchor_line: 10, head_line: 20, ..Default::default()
        });
        tab.edit_transaction = Some(crate::buffer::EditTransaction {
            depth: 1, ops: Vec::new(), sel_before_anchor: 40, sel_before_head: 60,
        });
        reload_tab_from_disk(&mut tab).expect("reload");
        assert_eq!(tab.session_id, id);
        assert!(tab.buffer.read_only);
        assert_eq!(tab.buffer.tab_color, Some(3));
        assert!(tab.begin_select_marker.is_none());
        assert!(tab.block_selection.is_none());
        assert!(tab.edit_transaction.is_none());
    }

    #[test]
    fn failed_reload_keeps_unsaved_document() {
        let dir = crate::persist::TempDir::new("reload-fail");
        let path = dir.join("broken.rtf");
        std::fs::write(&path, b"{\\rtf1 old}").expect("write fixture");
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path.clone());
        tab.buffer.rich_doc = Some(crate::rich_doc::open(&path).expect("open"));
        tab.buffer.rope = ropey::Rope::from_str("unsaved source");
        tab.buffer.dirty = true;
        tab.selection.collapse_to(7);
        // A directory cannot be parsed or read as a document on any platform.
        tab.buffer.file_path = Some(dir.path().to_path_buf());
        assert!(reload_tab_from_disk(&mut tab).is_err());
        assert_eq!(tab.buffer.rope.to_string(), "unsaved source");
        assert_eq!(tab.buffer.rich_doc.as_ref().expect("payload").block_text(0), "old");
        assert!(tab.buffer.dirty);
        assert_eq!(tab.selection.head, 7);
    }

    #[test]
    fn reload_does_not_replace_a_streaming_buffer() {
        let dir = crate::persist::TempDir::new("reload-streaming");
        let path = dir.join("loading.txt");
        std::fs::write(&path, "complete").expect("write fixture");
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path);
        tab.buffer.rope = ropey::Rope::from_str("partial");
        tab.buffer.loading = Some(crate::buffer::LoadProgress {
            bytes_read: 7, total_bytes: 20,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        assert!(reload_tab_from_disk(&mut tab).is_err());
        assert_eq!(tab.buffer.rope.to_string(), "partial");
        assert!(tab.buffer.loading.is_some());
    }

    #[test]
    fn declined_reload_keeps_dirty_text() {
        let dir = crate::persist::TempDir::new("reload-declined");
        let path = dir.join("file.txt");
        std::fs::write(&path, "on disk").expect("write fixture");
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path);
        tab.buffer.rope = ropey::Rope::from_str("unsaved edits");
        tab.buffer.dirty = true;
        let id = tab.session_id;
        let mut tabs = vec![tab];
        assert_eq!(reload_tab_by_id(&mut tabs, id, false).expect("decline"), None);
        assert_eq!(tabs[0].buffer.rope.to_string(), "unsaved edits");
        assert!(tabs[0].buffer.dirty);
    }

    #[test]
    fn confirmed_reload_tracks_tab_after_reorder() {
        let dir = crate::persist::TempDir::new("reload-reorder");
        let path = dir.join("file.txt");
        std::fs::write(&path, "on disk").expect("write fixture");
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path);
        tab.buffer.rope = ropey::Rope::from_str("unsaved edits");
        tab.buffer.dirty = true;
        let id = tab.session_id;
        let mut tabs = vec![tab, Tab::empty()];
        let other_id = tabs[1].session_id;
        tabs.swap(0, 1);
        assert_eq!(reload_tab_by_id(&mut tabs, id, true).expect("confirm"), Some(1));
        assert_eq!(tabs[1].buffer.rope.to_string(), "on disk");
        assert!(!tabs[1].buffer.dirty);
        assert_eq!(tabs[0].session_id, other_id);
        assert_eq!(tabs[0].buffer.rope.len_bytes(), 0);
        assert_eq!(reload_tab_by_id(&mut tabs, uuid::Uuid::new_v4(), true).expect("closed"), None);
    }

    /// A closed window's strip must still answer `tabs[active]`: the
    /// debounced reparse indexed it after the window was gone.
    #[test]
    fn closed_strip_still_has_an_active_tab() {
        let mut tabs: Vec<Tab> = Vec::new();
        let mut active = active_after_removal(0, 0, 0);
        park_closed_strip(&mut tabs, &mut active);
        assert!(tabs.get(active).is_some());

        let mut tabs = vec![Tab::empty(), Tab::empty()];
        let kept = tabs[1].session_id;
        let mut active = 9;
        park_closed_strip(&mut tabs, &mut active);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.get(active).map(|t| t.session_id), Some(kept));
    }

    /// Replays the Close All loop (rightmost tab first) and checks after
    /// every removal that `active` still indexes a tab, which is what render
    /// does while the next dirty prompt is up.
    #[test]
    fn close_all_keeps_active_in_range_after_each_removal() {
        for len in 1..6usize {
            for start_active in 0..len {
                let mut tabs: Vec<usize> = (0..len).collect();
                let mut active = start_active;
                let want = tabs[active];
                while let Some(idx) = tabs.len().checked_sub(1) {
                    tabs.remove(idx);
                    active = active_after_removal(active, idx, tabs.len());
                    if tabs.is_empty() {
                        break;
                    }
                    assert!(active < tabs.len(), "len {len} active {start_active}");
                    if want < tabs.len() {
                        assert_eq!(tabs[active], want, "active buffer changed");
                    }
                }
            }
        }
    }

    #[test]
    fn active_follows_its_buffer_or_neighbour() {
        // [A, B, C], active C, close A -> C now at 1.
        assert_eq!(active_after_removal(2, 0, 2), 1);
        // active B, close B -> C slides into slot 1.
        assert_eq!(active_after_removal(1, 1, 2), 1);
        // active C, close C -> B.
        assert_eq!(active_after_removal(2, 2, 2), 1);
        // active A, close C -> A.
        assert_eq!(active_after_removal(0, 2, 2), 0);
        // last tab closed.
        assert_eq!(active_after_removal(0, 0, 0), 0);
        // stale active beyond the strip is pulled back in.
        assert_eq!(active_after_removal(7, 5, 3), 2);
    }
}

#[cfg(test)]
#[path = "close_batch_tests.rs"]
mod close_batch_tests;

#[cfg(test)]
#[path = "close_async_tests.rs"]
mod close_async_tests;
