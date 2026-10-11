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
        } else if tab.buffer.read_only {
            Some("Buffer is read-only")
        } else {
            None
        };
        if let Some(message) = refusal {
            self.set_status(message);
            cx.notify();
            return false;
        }
        self.close_autocomplete(cx);
        let prev = set_replaying(true);
        let mut completed = 0usize;
        for _ in 0..times {
            for ev in events {
                if !self.replay_event(ev, window, cx) {
                    set_replaying(prev);
                    if completed > 0 {
                        let reason = self.status_message.as_deref().unwrap_or("event rejected");
                        let unit = if completed == 1 { "event" } else { "events" };
                        self.set_status(format!("Macro stopped after {completed} {unit}: {reason}"));
                        cx.notify();
                    }
                    return false;
                }
                completed = completed.saturating_add(1);
            }
        }
        set_replaying(prev);
        true
    }

    fn replay_event(&mut self, event: &MacroEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match event {
            MacroEvent::InsertText(text) => {
                let r = self.selection().ordered();
                self.replace_byte_range(r, text, cx);
            }
            MacroEvent::CompleteWord { prefix, candidate } => {
                return self.replay_completion_word(prefix, candidate, window, cx);
            }
            MacroEvent::Backspace => self.backspace(&EditBackspace, window, cx),
            MacroEvent::Delete => self.delete(&EditDelete, window, cx),
            MacroEvent::InsertNewline => self.insert_newline(&EditInsertNewline, window, cx),
            MacroEvent::InsertTab => self.insert_tab(&EditInsertTab, window, cx),
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
            MacroEvent::Cut => self.cut(&EditCut, window, cx),
            MacroEvent::Copy => self.copy(&EditCopy, window, cx),
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
            // Find/Replace/Save aren't replayed in v1 — recording skips them
            // too, so these arms are only reached for macros loaded from
            // disk that predate the field. Treat them as no-ops.
            MacroEvent::FindNext
            | MacroEvent::FindPrev
            | MacroEvent::ReplaceCurrent
            | MacroEvent::ReplaceAll
            | MacroEvent::Save => {}
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
