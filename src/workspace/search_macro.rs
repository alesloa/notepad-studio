// PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:4328-4389 captures
// search parameters; NppCommands.cpp:83-149 groups playback Undo actions.
use super::{MacroEvent, InputFocus, TabViewMode, Workspace, set_replaying};
use crate::{actions::*, buffer::{Selection, UndoEntry}, macros::{self, SearchMacro}};
use gpui::{Context, Window};
use std::sync::Arc;

pub(super) fn needs_background(event: &MacroEvent) -> bool {
    matches!(event, MacroEvent::ReplaceCurrentWithOptions(search)
        | MacroEvent::ReplaceAllWithOptions(search) if search.regex)
        || matches!(event, MacroEvent::CutBookmarkedLines | MacroEvent::PasteToBookmarkedLines
            | MacroEvent::RemoveBookmarkedLines | MacroEvent::RemoveNonBookmarkedLines | MacroEvent::InverseBookmarks
            | MacroEvent::GoToMatchingBrace | MacroEvent::SelectInBetween
            | MacroEvent::CopyBookmarkedLines | MacroEvent::Undo | MacroEvent::Redo | MacroEvent::SortLinesLexIgnoreCaseAsc | MacroEvent::SortLinesLexIgnoreCaseDesc
            | MacroEvent::SortLinesLexAsc | MacroEvent::SortLinesLexDesc | MacroEvent::ReverseLineOrder | MacroEvent::RandomizeLineOrder
            | MacroEvent::SortLinesIntAsc | MacroEvent::SortLinesIntDesc | MacroEvent::SortLinesDecCommaAsc | MacroEvent::SortLinesDecCommaDesc
            | MacroEvent::SortLinesDecDotAsc | MacroEvent::SortLinesDecDotDesc
            | MacroEvent::RemoveDuplicateLines | MacroEvent::RemoveConsecutiveDuplicateLines
            | MacroEvent::SortLinesLocaleAsc | MacroEvent::SortLinesLocaleDesc | MacroEvent::SortLinesLengthAsc | MacroEvent::SortLinesLengthDesc)
}

pub(super) fn allows_read_only(event: &MacroEvent) -> bool {
    matches!(event, MacroEvent::InverseBookmarks | MacroEvent::GoToMatchingBrace | MacroEvent::SelectInBetween)
}

pub(super) fn is_search(event: &MacroEvent) -> bool {
    matches!(event, MacroEvent::FindNextWithOptions(_) | MacroEvent::FindPrevWithOptions(_)
        | MacroEvent::ReplaceCurrentWithOptions(_) | MacroEvent::ReplaceAllWithOptions(_))
}

pub(in crate::workspace) struct Playback {
    pub(in crate::workspace) id: uuid::Uuid,
    pub(in crate::workspace) window: gpui::AnyWindowHandle,
    pub(in crate::workspace) completed: usize,
    events: Vec<MacroEvent>,
    index: usize,
    remaining: usize,
    tab: uuid::Uuid,
    instance: Arc<()>,
    input: gpui::InputContextToken,
    metadata_only: bool,
    history: crate::workspace::typing_undo::HistoryStamp,
    pub(in crate::workspace) undo_base: usize,
    pub(in crate::workspace) undo_len: usize,
    pub(in crate::workspace) redo_len: usize,
    pub(in crate::workspace) grouped: bool,
    pub(in crate::workspace) selection: Selection,
    pub(in crate::workspace) block: Option<crate::buffer::BlockSelection>,
}

impl Playback {
    pub(in crate::workspace) fn stopped(&self, reason: &str) -> String {
        let unit = if self.completed == 1 { "event" } else { "events" };
        format!("Macro stopped after {} {unit}: {reason}", self.completed)
    }

    pub(in crate::workspace) fn refresh_history(&mut self, tab: &crate::tab::Tab) {
        self.undo_len = tab.undo_stack.len(); self.redo_len = tab.redo_stack.len();
        self.history = crate::workspace::typing_undo::HistoryStamp::capture(tab);
    }

    pub(in crate::workspace) fn valid(&self, w: &Workspace, window: &Window) -> bool {
        w.tabs.get(w.active).is_some_and(|tab| {
            tab.session_id == self.tab && Arc::ptr_eq(&tab.instance, &self.instance)
                && tab.view_mode == TabViewMode::Edit && (!tab.buffer.read_only || self.metadata_only)
                && tab.buffer.loading.is_none() && tab.edit_transaction.is_none()
                && self.history.matches(tab)
                && tab.undo_stack.len() == self.undo_len && tab.redo_stack.len() == self.redo_len
        }) && !w.window_closed && window.is_window_active()
            && window.input_context_token() == self.input
            && w.focus_handle.is_focused(window) && w.input_focus(window) == InputFocus::Editor
            && !w.is_composing_text(window)
    }
}

impl Workspace {
    pub(in crate::workspace) fn replacement_dont_move_to_next(&self) -> bool {
        self.regex_replacement.macro_dont_move
            .unwrap_or_else(|| crate::settings::snapshot().replace_dont_move_to_next)
    }

    pub(in crate::workspace) fn record_search_macro(
        &mut self, event: fn(SearchMacro) -> MacroEvent, cx: &mut Context<Self>,
    ) {
        if !macros::is_recording() { return; }
        if !self.tabs.get(self.active).is_some_and(|tab| tab.view_mode == TabViewMode::Edit) {
            self.set_status("Search macro recording requires Source or Raw View");
            cx.notify();
            return;
        }
        if self.find_state.query.len() > crate::text::regex_replacement::MAX_TEMPLATE_BYTES
            || self.find_state.replacement.len() > crate::text::regex_replacement::MAX_TEMPLATE_BYTES {
            self.set_status("Search macro not recorded: search and replacement must each be at most 1 MiB");
            cx.notify();
            return;
        }
        let state = &self.find_state;
        macros::record(event(SearchMacro {
            query: state.query.clone(), replacement: state.replacement.clone(),
            match_case: state.match_case, whole_word: state.whole_word,
            regex: state.regex, extended: state.extended,
            dot_matches_newline: state.dot_matches_newline, wrap_around: state.wrap_around,
            backward: state.backward, in_selection: state.in_selection,
            dont_move_to_next: self.replacement_dont_move_to_next(),
        }));
    }

    fn apply_search_macro(&mut self, search: &SearchMacro) {
        let state = &mut self.find_state;
        let changed = state.query != search.query || state.match_case != search.match_case
            || state.whole_word != search.whole_word || state.regex != search.regex
            || state.extended != search.extended || state.dot_matches_newline != search.dot_matches_newline
            || state.in_selection != search.in_selection;
        state.query = search.query.clone();
        state.query_cursor = state.query.len(); state.query_anchor = state.query_cursor;
        state.replacement = search.replacement.clone();
        state.replacement_cursor = state.replacement.len(); state.replacement_anchor = state.replacement_cursor;
        state.match_case = search.match_case; state.whole_word = search.whole_word;
        state.regex = search.regex; state.extended = search.extended;
        state.dot_matches_newline = search.dot_matches_newline; state.wrap_around = search.wrap_around;
        state.backward = search.backward; state.in_selection = search.in_selection;
        state.error = None; state.status = None;
        if changed { state.matches.clear(); state.current = None; }
        if search.in_selection && state.in_selection_scope_owner.is_none() {
            let tab = self.active_tab_mut();
            let scope = tab.selection.clamped(&tab.buffer.rope).ordered();
            let owner = crate::find_state::FindScopeOwner {
                tab: tab.session_id, instance: tab.instance.clone(), view: tab.view_mode,
            };
            self.find_state.in_selection_scope = (!scope.is_empty()).then_some(scope);
            self.find_state.in_selection_scope_owner = Some(owner);
        } else if !search.in_selection {
            self.find_state.in_selection_scope = None;
            self.find_state.in_selection_scope_owner = None;
        }
    }

    pub(in crate::workspace) fn replay_search_event(
        &mut self, event: &MacroEvent, search: &SearchMacro, window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        if search.query.len() > crate::text::regex_replacement::MAX_TEMPLATE_BYTES
            || search.replacement.len() > crate::text::regex_replacement::MAX_TEMPLATE_BYTES {
            self.set_status("Macro search and replacement must each be at most 1 MiB");
            cx.notify(); return false;
        }
        if self.has_block_selection() && search.in_selection {
            self.set_status("Search macro In Selection requires a linear selection");
            cx.notify(); return false;
        }
        self.apply_search_macro(search);
        self.regex_replacement.macro_dont_move = Some(search.dont_move_to_next);
        match event {
            MacroEvent::FindNextWithOptions(_) => self.find_next(&EditFindNext, window, cx),
            MacroEvent::FindPrevWithOptions(_) => self.find_prev(&EditFindPrev, window, cx),
            MacroEvent::ReplaceCurrentWithOptions(_) => self.replace_current_match(&EditReplaceCurrent, window, cx),
            MacroEvent::ReplaceAllWithOptions(_) => self.replace_all_matches(&EditReplaceAll, window, cx),
            _ => {}
        }
        self.regex_replacement.macro_dont_move = None;
        if let Some(error) = self.find_state.error.clone() {
            self.set_status(error); cx.notify(); return false;
        }
        if search.regex && matches!(event, MacroEvent::ReplaceCurrentWithOptions(_)
            | MacroEvent::ReplaceAllWithOptions(_)) && self.regex_replacement.pending.is_none()
            && !search.query.is_empty() {
            self.set_status("Macro replacement could not start"); cx.notify(); return false;
        }
        true
    }

    pub(in crate::workspace) fn start_search_macro_playback(
        &mut self, events: &[MacroEvent], times: usize, window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        if tab.edit_transaction.is_some() || self.is_composing_text(window) {
            self.set_status("Finish the active edit or composition before playing a search macro");
            cx.notify(); return false;
        }
        let playback = Playback {
            id: uuid::Uuid::new_v4(), window: window.window_handle(), completed: 0,
            events: events.to_vec(), index: 0, remaining: times, tab: tab.session_id,
            instance: tab.instance.clone(), input: window.input_context_token(),
            metadata_only: events.iter().all(allows_read_only),
            history: crate::workspace::typing_undo::HistoryStamp::capture(tab),
            undo_base: tab.undo_stack.len(), undo_len: tab.undo_stack.len(), redo_len: tab.redo_stack.len(),
            grouped: !events.iter().any(|ev| matches!(ev, MacroEvent::Undo | MacroEvent::Redo)),
            selection: tab.selection, block: tab.block_selection,
        };
        self.cancel_regex_replacement();
        self.close_autocomplete(cx); self.close_function_hint(cx);
        self.find_state.matches.clear(); self.find_state.current = None;
        self.find_state.in_selection_scope = None; self.find_state.in_selection_scope_owner = None;
        self.continue_search_macro(playback, window, cx)
    }

    // No edit transaction survives a worker wait. A real keystroke or Undo
    // therefore owns its history normally and invalidates the pending request.
    pub(in crate::workspace) fn continue_search_macro(
        &mut self, playback: Playback, window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        let grouped = playback.grouped;
        let mut group_open = grouped;
        if group_open { self.begin_edit_group(); }
        let previous = set_replaying(true);
        self.regex_replacement.macro_playback = Some(playback);
        loop {
            let event = self.regex_replacement.macro_playback.as_mut().and_then(|playback| {
                if playback.index >= playback.events.len() {
                    playback.remaining = playback.remaining.saturating_sub(1); playback.index = 0;
                }
                if playback.remaining == 0 { return None; }
                let event = playback.events.get(playback.index)?.clone();
                playback.index = playback.index.saturating_add(1);
                Some(event)
            });
            let Some(event) = event else { break; };
            // Flush the completed prefix before the replacement captures its
            // source stamp. The flush retires the source-input epoch.
            if needs_background(&event) && group_open {
                self.end_edit_group(cx); group_open = false;
                if let Some(playback) = self.regex_replacement.macro_playback.take() {
                    self.merge_search_macro_undo(&playback);
                    self.regex_replacement.macro_playback = Some(playback);
                }
            }
            if !self.replay_event(&event, window, cx) {
                if group_open { self.end_edit_group(cx); }
                set_replaying(previous);
                if let Some(playback) = self.regex_replacement.macro_playback.take() {
                    self.merge_search_macro_undo(&playback);
                    let reason = self.status_message.clone().unwrap_or_else(|| "event rejected".into());
                    self.set_status(playback.stopped(&reason));
                }
                cx.notify(); return false;
            }
            if self.regex_replacement.pending.is_some() || self.pending_prepared_edit.is_some() || self.pending_bookmark_copy.is_some() || self.brace_navigation_pending() {
                if group_open { self.end_edit_group(cx); }
                set_replaying(previous);
                if let Some(mut playback) = self.regex_replacement.macro_playback.take() {
                    self.merge_search_macro_undo(&playback);
                    if let Some(tab) = self.tabs.get(self.active) {
                        playback.refresh_history(tab);
                    }
                    self.regex_replacement.macro_playback = Some(playback);
                }
                if self.regex_replacement.pending.is_some() { self.set_status("Preparing macro replacement; Escape cancels"); }
                cx.notify(); return false;
            }
            if let Some(playback) = self.regex_replacement.macro_playback.as_mut() {
                playback.completed = playback.completed.saturating_add(1);
            }
            if grouped && !group_open { self.begin_edit_group(); group_open = true; }
        }
        if group_open { self.end_edit_group(cx); }
        set_replaying(previous);
        if let Some(playback) = self.regex_replacement.macro_playback.take() {
            if playback.grouped && self.tabs.get(self.active).is_some_and(|tab| {
                tab.undo_stack.len().saturating_sub(playback.undo_base) > 1
                    && tab.undo_stack.iter().skip(playback.undo_base).any(|entry| matches!(entry, UndoEntry::Prepared(_)))
            }) {
                return self.start_prepared_macro_merge(playback, window, cx);
            }
            self.merge_search_macro_undo(&playback);
            self.set_status(format!("Played macro ({} events)", playback.completed));
        }
        cx.notify(); true
    }

    pub(in crate::workspace) fn merge_search_macro_undo(&mut self, playback: &Playback) {
        if !playback.grouped { return; }
        let Some(tab) = self.tabs.get_mut(self.active) else { return; };
        if tab.session_id != playback.tab || !Arc::ptr_eq(&tab.instance, &playback.instance)
            || tab.undo_stack.len() <= playback.undo_base { return; }
        // Only entries created by uninterrupted replay can reach this path.
        // Prepared history is never produced by its synchronous handlers.
        if tab.undo_stack.iter().skip(playback.undo_base).any(|entry| matches!(entry, UndoEntry::Prepared(_))) { return; }
        let Some(first) = tab.undo_stack.get_mut(playback.undo_base) else { return; };
        let first = std::mem::replace(first, UndoEntry::Group { ops: Vec::new(),
            sel_before_anchor: playback.selection.anchor, sel_before_head: playback.selection.head });
        // Reuse the accumulated group's capacity rather than copying the
        // whole history vector again at every background replacement.
        let mut ops = match first {
            UndoEntry::Single(op) => vec![op],
            UndoEntry::Group { ops, .. } => ops,
            UndoEntry::Prepared(_) => return,
        };
        for entry in tab.undo_stack.drain(playback.undo_base.saturating_add(1)..) {
            match entry {
                UndoEntry::Single(op) => ops.push(op),
                UndoEntry::Group { ops: group, .. } => ops.extend(group),
                UndoEntry::Prepared(_) => {}
            }
        }
        if let Some(first) = tab.undo_stack.get_mut(playback.undo_base) {
            *first = UndoEntry::Group { ops,
                sel_before_anchor: playback.selection.anchor, sel_before_head: playback.selection.head };
        }
    }
}
