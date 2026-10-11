// PowerEditor/src/NppCommands.cpp:83, 106-125 — macro playback targets the editor document.
// Macro recorder hooks + replay engine + action handlers.
//
// Recording side: every editor action handler calls `record_event(...)` near
// its top when the input focus is the editor body. The record helper drops
// the event silently if no recording is in flight, so the cost on the
// non-recording hot path is one branch and a mutex-lock attempt.
//
// Replay side: dispatch each event by calling the matching `Workspace`
// method directly with a default-constructed action marker. Replay is
// wrapped in a `replaying` guard so the recorder doesn't append the replayed
// events back into a fresh recording.

use gpui::{Context, Window};

use crate::actions::{
    EditBackspace, EditBlockComment, EditCopy, EditCut, EditDelete, EditDeleteLine,
    EditDeleteLineLeft, EditDeleteLineRight, EditDeleteWordLeft, EditDeleteWordRight,
    EditDuplicateLine, EditInsertBlankLineAbove, EditInsertBlankLineBelow,
    EditInsertDateTimeCustomized, EditInsertDateTimeLong, EditInsertDateTimeShort,
    EditInsertNewline, EditInsertTab, EditLowercase, EditMoveLineDown, EditMoveLineUp,
    EditPageDown, EditPageUp, EditPaste, EditRedo, EditScrollLineDown, EditScrollLineUp,
    EditSelectAll, EditStreamComment, EditToggleOvertype, EditTrimTrailingWs, EditUndo,
    EditUppercase, MacroPlayback, MacroRunMultipleTimes,
    EditTrimLeadingWs, EditTrimLeadingAndTrailingWs, EditEolToSpace,
    EditRemoveUnnecessaryBlankAndEol, EditTabToSpace, EditSpaceToTabAll, EditSpaceToTabLeading,
    MacroRunSaved, MacroSaveCurrent, MacroStartRecording, MacroStartStopRecording,
    MacroStopRecording, MoveDocEnd, MoveDocStart, MoveDown, MoveLeft, MoveLineEnd, MoveLineStart,
    MoveRight, MoveUp, MoveWordLeft, MoveWordRight, SelectDocEnd, SelectDocStart, SelectDown,
    SelectLeft, SelectLineEnd, SelectLineStart, SelectPageDown, SelectPageUp, SelectRight,
    SelectUp, SelectWordLeft, SelectWordRight,
};
use crate::find_state::{MacroPromptMode, MacroPromptState};
use crate::macros::{self, MacroEvent, set_replaying};
use crate::tab::{InputFocus, TabViewMode};
use crate::workspace::Workspace;

#[path = "completion_macro.rs"]
mod completion_macro;

#[path = "search_macro.rs"]
pub(super) mod search_macro;

#[cfg(test)]
#[path = "search_macro_tests.rs"]
mod search_macro_tests;

fn unsupported_macro_operation(event: &MacroEvent) -> Option<&'static str> {
    match event {
        MacroEvent::FindNext => Some("Find Next"),
        MacroEvent::FindPrev => Some("Find Previous"),
        MacroEvent::ReplaceCurrent => Some("Replace Current"),
        MacroEvent::ReplaceAll => Some("Replace All"),
        MacroEvent::Save => Some("Save"),
        _ => None,
    }
}

impl Workspace {
    /// Replay `events` `times` times against the active editor. Recording is
    /// suspended for the duration so a replay-while-recording session
    /// captures one entry for the playback (the surrounding `MacroPlayback`
    /// action) instead of duplicating every event the playback emits.
    pub(crate) fn replay_macro(
        &mut self,
        events: &[MacroEvent],
        times: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if events.is_empty() || times == 0 {
            return false;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            self.set_status("No active editor for macro playback");
            cx.notify();
            return false;
        };
        let refusal = if tab.view_mode != TabViewMode::Edit {
            Some("Switch to Source or Raw View to run a macro")
        } else if !window.is_window_active()
            || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor
        {
            Some("Focus the active Source or Raw View editor to run a macro")
        } else if tab.buffer.loading.is_some() {
            Some("Wait for the file to finish loading before running a macro")
        } else if tab.buffer.read_only && !events.iter().all(|event| matches!(event, MacroEvent::InverseBookmarks)) {
            Some("Buffer is read-only")
        } else if self.is_composing_text(window) {
            Some("Finish the active composition before running a macro")
        } else {
            None
        };
        if let Some(message) = refusal {
            self.set_status(message);
            cx.notify();
            return false;
        }
        // Legacy unit events contain no captured search/save parameters. Refuse
        // the entire macro instead of silently skipping an operation after edits.
        if let Some(operation) = events.iter().find_map(unsupported_macro_operation) {
            self.set_status(format!("Macro playback does not support {operation} yet"));
            cx.notify();
            return false;
        }
        if self.regex_replacement.macro_playback.is_some() {
            self.set_status("A macro replacement is preparing; finish or cancel it first");
            cx.notify();
            return false;
        }
        if events.iter().any(search_macro::needs_background) {
            return self.start_search_macro_playback(events, times, window, cx);
        }
        if events.iter().any(search_macro::is_search) {
            self.find_state.matches.clear();
            self.find_state.current = None;
            self.find_state.in_selection_scope = None;
            self.find_state.in_selection_scope_owner = None;
        }
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        // Old saved macros can contain history commands; replay them on their
        // existing timeline instead of placing Undo/Redo inside an open group.
        let grouped = !events.iter().any(|event| matches!(event, MacroEvent::Undo | MacroEvent::Redo));
        if grouped { self.begin_edit_group(); }
        let prev = set_replaying(true);
        let mut completed = 0usize;
        for _ in 0..times {
            for ev in events {
                if !self.replay_event(ev, window, cx) {
                    let reason = self.status_message.as_deref().unwrap_or("event rejected").to_string();
                    if grouped { self.end_edit_group(cx); }
                    set_replaying(prev);
                    if completed > 0 {
                        let unit = if completed == 1 { "event" } else { "events" };
                        self.set_status(format!("Macro stopped after {completed} {unit}: {reason}"));
                    } else {
                        self.set_status(reason);
                    }
                    cx.notify();
                    return false;
                }
                completed = completed.saturating_add(1);
            }
        }
        if grouped { self.end_edit_group(cx); }
        set_replaying(prev);
        true
    }

    pub(super) fn replay_event(&mut self, event: &MacroEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let prepared = match event {
            MacroEvent::Undo => self.tabs.get(self.active).and_then(|tab| tab.undo_stack.last()),
            MacroEvent::Redo => self.tabs.get(self.active).and_then(|tab| tab.redo_stack.last()),
            _ => None,
        };
        if matches!(prepared, Some(crate::buffer::UndoEntry::Prepared(group)) if !group.is_line_edit()) {
            self.set_status("Large indentation Undo/Redo is unavailable during macro playback");
            cx.notify();
            return false;
        }
        match event {
            MacroEvent::InsertText(text) => {
                let r = self.selection().ordered();
                self.replace_byte_range(r, text, cx);
            }
            MacroEvent::CompleteWord { prefix, candidate } => {
                return self.replay_completion_word(prefix, candidate, window, cx);
            }
            MacroEvent::CorrectText { original, replacement } => {
                return self.replay_proofreading_correction(original, replacement, cx);
            }
            MacroEvent::Backspace => self.backspace(&EditBackspace, window, cx),
            MacroEvent::Delete => self.delete(&EditDelete, window, cx),
            MacroEvent::InsertNewline => self.insert_newline(&EditInsertNewline, window, cx),
            MacroEvent::InsertTab => self.insert_tab(&EditInsertTab, window, cx),
            MacroEvent::IncreaseLineIndent => return self.run_menu_line_command(crate::edit_ops::menu_line_commands::MenuLineCommand::Increase, window, cx),
            MacroEvent::DecreaseLineIndent => return self.run_menu_line_command(crate::edit_ops::menu_line_commands::MenuLineCommand::Decrease, window, cx),
            MacroEvent::DuplicateCurrentLine => return self.run_menu_line_command(crate::edit_ops::menu_line_commands::MenuLineCommand::Duplicate, window, cx),
            MacroEvent::SortLinesLexIgnoreCaseAsc => return self.run_line_sort(crate::edit_ops::line_sort::SortCommand::IgnoreCaseAsc, window, cx),
            MacroEvent::SortLinesLexIgnoreCaseDesc => return self.run_line_sort(crate::edit_ops::line_sort::SortCommand::IgnoreCaseDesc, window, cx),
            MacroEvent::SortLinesLocaleAsc => return self.run_line_sort(crate::edit_ops::line_sort::SortCommand::LocaleAsc, window, cx),
            MacroEvent::SortLinesLocaleDesc => return self.run_line_sort(crate::edit_ops::line_sort::SortCommand::LocaleDesc, window, cx),
            MacroEvent::SortLinesLengthAsc => return self.run_line_sort(crate::edit_ops::line_sort::SortCommand::LengthAsc, window, cx),
            MacroEvent::SortLinesLengthDesc => return self.run_line_sort(crate::edit_ops::line_sort::SortCommand::LengthDesc, window, cx),
            MacroEvent::DuplicateLine => self.duplicate_line(&EditDuplicateLine, window, cx),
            MacroEvent::MoveLineUp => self.move_line_up(&EditMoveLineUp, window, cx),
            MacroEvent::MoveLineDown => self.move_line_down(&EditMoveLineDown, window, cx),
            MacroEvent::DeleteLine => self.delete_line(&EditDeleteLine, window, cx),
            MacroEvent::Uppercase => self.uppercase_selection(&EditUppercase, window, cx),
            MacroEvent::Lowercase => self.lowercase_selection(&EditLowercase, window, cx),
            MacroEvent::BlockComment => self.block_comment(&EditBlockComment, window, cx),
            MacroEvent::StreamComment => self.stream_comment(&EditStreamComment, window, cx),
            MacroEvent::InsertBlankLineAbove => {
                self.insert_blank_line_above(&EditInsertBlankLineAbove, window, cx)
            }
            MacroEvent::InsertBlankLineBelow => {
                self.insert_blank_line_below(&EditInsertBlankLineBelow, window, cx)
            }
            MacroEvent::TrimTrailingWs => self.trim_trailing_ws(&EditTrimTrailingWs, window, cx),
            MacroEvent::TrimLeadingWs => self.trim_leading_ws(&EditTrimLeadingWs, window, cx),
            MacroEvent::TrimLeadingAndTrailingWs => {
                self.trim_leading_and_trailing_ws(&EditTrimLeadingAndTrailingWs, window, cx)
            }
            MacroEvent::EolToSpace => self.eol_to_space(&EditEolToSpace, window, cx),
            MacroEvent::RemoveUnnecessaryBlankAndEol => {
                self.remove_unnecessary_blank_and_eol(&EditRemoveUnnecessaryBlankAndEol, window, cx)
            }
            MacroEvent::TabToSpace => self.tab_to_space(&EditTabToSpace, window, cx),
            MacroEvent::SpaceToTabAll => self.space_to_tab_all(&EditSpaceToTabAll, window, cx),
            MacroEvent::SpaceToTabLeading => self.space_to_tab_leading(&EditSpaceToTabLeading, window, cx),
            MacroEvent::Cut => self.cut(&EditCut, window, cx),
            MacroEvent::Copy => self.copy(&EditCopy, window, cx),
            MacroEvent::CopyAllFilenames => return self.copy_all_tab_labels(false, cx),
            MacroEvent::CopyAllFilePaths => return self.copy_all_tab_labels(true, cx),
            MacroEvent::CopyBookmarkedLines => return self.run_copy_bookmarked_lines(window, cx),
            MacroEvent::CutBookmarkedLines => return self.run_bookmark_command(crate::edit_ops::bookmark_edits::BookmarkCommand::Cut, window, cx),
            MacroEvent::PasteToBookmarkedLines => return self.run_bookmark_command(crate::edit_ops::bookmark_edits::BookmarkCommand::Paste, window, cx),
            MacroEvent::RemoveBookmarkedLines => return self.run_bookmark_command(crate::edit_ops::bookmark_edits::BookmarkCommand::Remove, window, cx),
            MacroEvent::RemoveNonBookmarkedLines => return self.run_bookmark_command(crate::edit_ops::bookmark_edits::BookmarkCommand::RemoveNon, window, cx),
            MacroEvent::InverseBookmarks => return self.run_bookmark_command(crate::edit_ops::bookmark_edits::BookmarkCommand::Inverse, window, cx),
            MacroEvent::Paste => self.paste(&EditPaste, window, cx),
            MacroEvent::SelectAll => self.select_all(&EditSelectAll, window, cx),
            MacroEvent::Undo => self.undo(&EditUndo, window, cx),
            MacroEvent::Redo => self.redo(&EditRedo, window, cx),
            MacroEvent::MoveLeft => self.move_left(&MoveLeft, window, cx),
            MacroEvent::MoveRight => self.move_right(&MoveRight, window, cx),
            MacroEvent::MoveUp => self.move_up(&MoveUp, window, cx),
            MacroEvent::MoveDown => self.move_down(&MoveDown, window, cx),
            MacroEvent::SelectLeft => self.select_left(&SelectLeft, window, cx),
            MacroEvent::SelectRight => self.select_right(&SelectRight, window, cx),
            MacroEvent::SelectUp => self.select_up(&SelectUp, window, cx),
            MacroEvent::SelectDown => self.select_down(&SelectDown, window, cx),
            MacroEvent::PageUp => self.page_up(&EditPageUp, window, cx),
            MacroEvent::PageDown => self.page_down(&EditPageDown, window, cx),
            MacroEvent::SelectPageUp => self.select_page_up(&SelectPageUp, window, cx),
            MacroEvent::SelectPageDown => self.select_page_down(&SelectPageDown, window, cx),
            MacroEvent::MoveLineStart => self.move_line_start(&MoveLineStart, window, cx),
            MacroEvent::MoveLineEnd => self.move_line_end(&MoveLineEnd, window, cx),
            MacroEvent::SelectLineStart => self.select_line_start(&SelectLineStart, window, cx),
            MacroEvent::SelectLineEnd => self.select_line_end(&SelectLineEnd, window, cx),
            MacroEvent::MoveDocStart => self.move_doc_start(&MoveDocStart, window, cx),
            MacroEvent::MoveDocEnd => self.move_doc_end(&MoveDocEnd, window, cx),
            MacroEvent::SelectDocStart => self.select_doc_start(&SelectDocStart, window, cx),
            MacroEvent::SelectDocEnd => self.select_doc_end(&SelectDocEnd, window, cx),
            MacroEvent::MoveWordLeft => self.move_word_left(&MoveWordLeft, window, cx),
            MacroEvent::MoveWordRight => self.move_word_right(&MoveWordRight, window, cx),
            MacroEvent::SelectWordLeft => self.select_word_left(&SelectWordLeft, window, cx),
            MacroEvent::SelectWordRight => self.select_word_right(&SelectWordRight, window, cx),
            MacroEvent::FindNextWithOptions(search)
            | MacroEvent::FindPrevWithOptions(search)
            | MacroEvent::ReplaceCurrentWithOptions(search)
            | MacroEvent::ReplaceAllWithOptions(search) => {
                return self.replay_search_event(event, search, window, cx);
            }
            // Checked before any edit by replay_macro; never report a skipped
            // legacy operation as successful playback.
            MacroEvent::FindNext
            | MacroEvent::FindPrev
            | MacroEvent::ReplaceCurrent
            | MacroEvent::ReplaceAll
            | MacroEvent::Save => return false,
            MacroEvent::InsertDateTimeShort => {
                self.insert_datetime_short(&EditInsertDateTimeShort, window, cx)
            }
            MacroEvent::InsertDateTimeLong => {
                self.insert_datetime_long(&EditInsertDateTimeLong, window, cx)
            }
            MacroEvent::InsertDateTimeCustomized => {
                self.insert_datetime_customized(&EditInsertDateTimeCustomized, window, cx)
            }
            MacroEvent::DeleteWordLeft => self.delete_word_left(&EditDeleteWordLeft, window, cx),
            MacroEvent::DeleteWordRight => self.delete_word_right(&EditDeleteWordRight, window, cx),
            MacroEvent::DeleteLineLeft => self.delete_line_left(&EditDeleteLineLeft, window, cx),
            MacroEvent::DeleteLineRight => self.delete_line_right(&EditDeleteLineRight, window, cx),
            MacroEvent::ScrollLineUp => self.scroll_line_up(&EditScrollLineUp, window, cx),
            MacroEvent::ScrollLineDown => self.scroll_line_down(&EditScrollLineDown, window, cx),
            MacroEvent::ToggleOvertype => self.toggle_overtype(&EditToggleOvertype, window, cx),
        }
        true
    }

    // ---- Macro menu actions ---------------------------------------------------------------

    pub(crate) fn macro_start_stop_recording(
        &mut self,
        _: &MacroStartStopRecording,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now_recording = macros::toggle_recording();
        let msg = if now_recording {
            "Recording macro — press the same chord to stop"
        } else {
            "Macro recorded"
        };
        self.set_status(msg);
        cx.notify();
    }

    /// Explicit Start. Used by the toolbar's "Start Recording" button.
    /// No-op (with a status nudge) if recording is already in flight, so
    /// double-clicking the button can't reset the in-progress capture.
    pub(crate) fn macro_start_recording(
        &mut self,
        _: &MacroStartRecording,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if macros::is_recording() {
            self.set_status("Already recording — press Stop to finish");
        } else {
            macros::start_recording();
            self.set_status("Recording macro — press Stop to finish");
        }
        cx.notify();
    }

    /// Explicit Stop. Used by the toolbar's "Stop Recording" button.
    /// No-op (with a status nudge) if no recording is active.
    pub(crate) fn macro_stop_recording(
        &mut self,
        _: &MacroStopRecording,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if macros::is_recording() {
            macros::stop_recording();
            let count = macros::current().len();
            self.set_status(format!("Macro recorded ({count} events)"));
        } else {
            self.set_status("Not recording");
        }
        cx.notify();
    }

    pub(crate) fn macro_playback(
        &mut self,
        _: &MacroPlayback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = macros::current();
        if events.is_empty() {
            self.set_status("No macro to play — record one first");
            cx.notify();
            return;
        }
        if self.replay_macro(&events, 1, window, cx) {
            self.set_status(format!("Played macro ({} events)", events.len()));
        }
        cx.notify();
    }

    pub(crate) fn macro_run_multiple_times(
        &mut self,
        _: &MacroRunMultipleTimes,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = macros::current();
        if events.is_empty() {
            self.set_status("No macro to play — record one first");
            cx.notify();
            return;
        }
        self.open_macro_prompt(MacroPromptMode::RunCount, "10", window, cx);
    }

    pub(crate) fn macro_save_current(
        &mut self,
        _: &MacroSaveCurrent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = macros::current();
        if events.is_empty() {
            self.set_status("Nothing to save — record a macro first");
            cx.notify();
            return;
        }
        // Suggest a sensible default name: "Macro N" where N is one past the
        // current saved count.
        let next_idx = macros::saved_snapshot().len() + 1;
        let initial = format!("Macro {next_idx}");
        self.open_macro_prompt(MacroPromptMode::SaveName, &initial, window, cx);
    }

    pub(crate) fn macro_run_saved(
        &mut self,
        action: &MacroRunSaved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(events) = macros::saved_events(&action.name) else {
            self.set_status(format!("Saved macro '{}' is missing", action.name));
            cx.notify();
            return;
        };
        if self.replay_macro(&events, 1, window, cx) {
            self.set_status(format!("Ran macro '{}'", action.name));
        }
        cx.notify();
    }

    // ---- Macro prompt bar (shared Run-N + Save-name inline input) ---------------------------

    fn open_macro_prompt(
        &mut self,
        mode: MacroPromptMode,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.macro_focus.is_none() {
            self.macro_focus = Some(cx.focus_handle());
        }
        self.macro_prompt = Some(MacroPromptState::new(mode, initial));
        // Pre-select the entire initial text so a user typing N or a name
        // immediately replaces it (mirrors NPP's "select-on-focus" behavior).
        if let Some(state) = self.macro_prompt.as_mut() {
            state.text_anchor = 0;
            state.text_cursor = state.text.len();
        }
        self.focus_input(InputFocus::MacroPrompt, window, cx);
        cx.notify();
    }

    pub(crate) fn close_macro_prompt(&mut self, cx: &mut Context<Self>) {
        if self.macro_prompt.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn commit_macro_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.macro_prompt.take() else {
            return;
        };
        // Hand focus back to the editor before any replay runs so editor-side
        // handlers see Editor input focus.
        self.focus_input(InputFocus::Editor, window, cx);
        cx.notify();
        match state.mode {
            MacroPromptMode::RunCount => {
                let trimmed = state.text.trim();
                let Ok(n) = trimmed.parse::<usize>() else {
                    self.set_status("Macro count must be a positive integer");
                    return;
                };
                if n == 0 {
                    self.set_status("Macro count must be greater than zero");
                    return;
                }
                let n = n.min(9999);
                let events = macros::current();
                if events.is_empty() {
                    self.set_status("No macro to play");
                    return;
                }
                if self.replay_macro(&events, n, window, cx) {
                    self.set_status(format!("Ran macro {n} times"));
                }
            }
            MacroPromptMode::SaveName => {
                let name = state.text.trim().to_string();
                if name.is_empty() {
                    self.set_status("Macro name can't be empty");
                    return;
                }
                if macros::save_macro(name.clone()) {
                    self.set_status(format!("Saved macro '{name}'"));
                    crate::app::menus::rebuild_menus(&mut *cx);
                } else {
                    self.set_status("Nothing to save");
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "completion_macro_tests.rs"]
mod completion_macro_tests;

#[cfg(test)]
#[path = "macro_context_tests.rs"]
mod macro_context_tests;

#[cfg(test)]
#[path = "macro_undo_tests.rs"]
mod macro_undo_tests;

#[cfg(test)]
#[path = "legacy_macro_tests.rs"]
mod legacy_macro_tests;
