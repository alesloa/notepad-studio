// GPUI's InputHandler protocol uses UTF-16 document ranges and a selection
// relative to the new preedit. Runtime state belongs to one focused input.
use std::ops::Range;
use gpui::{Context, FocusHandle, Window};
use uuid::Uuid;
use crate::buffer::{ContentRevision, UndoEntry};
use crate::tab::{InputFocus, TabViewMode};
use crate::text::utf16::{str_byte_to_utf16, str_utf16_to_byte, rope_byte_to_utf16};
use super::Workspace;

enum ImeTarget {
    Source {
        revision: ContentRevision,
        before_revision: ContentRevision,
        undo_index: usize,
        start: usize,
        inserted_len: usize,
        prior_redo: Vec<UndoEntry>,
    },
    Csv { cell: Option<(usize, usize)>, len: usize },
}

pub(crate) struct ImeComposition {
    tab_id: Uuid,
    mode: TabViewMode,
    focus: FocusHandle,
    target: ImeTarget,
    marked: Range<usize>,
    selection: (usize, usize),
    text: String,
}

impl ImeComposition {
    fn context_valid(&self, w: &Workspace, window: &Window) -> bool {
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        if !window.is_window_active() || tab.session_id != self.tab_id
            || tab.view_mode != self.mode || !self.focus.is_focused(window)
        { return false; }
        match &self.target {
            ImeTarget::Source { revision, before_revision, undo_index, start, inserted_len, .. } => {
                let Some(end) = start.checked_add(*inserted_len) else { return false; };
                if tab.buffer.read_only || tab.buffer.loading.is_some() || tab.block_selection.is_some()
                    || tab.edit_transaction.is_some() || tab.buffer.raw_revision != *revision
                    || (tab.selection.anchor, tab.selection.head) != self.selection
                    || tab.undo_stack.len() != undo_index.saturating_add(1)
                    || self.marked.start < *start || self.marked.end > end
                    || self.marked.start > self.marked.end || end > tab.buffer.rope.len_bytes()
                { return false; }
                matches!(tab.undo_stack.get(*undo_index), Some(UndoEntry::Single(op))
                    if op.before_revision == *before_revision && op.after_revision == *revision
                        && op.byte_start == *start && op.inserted.len() == *inserted_len)
            }
            ImeTarget::Csv { cell, len } => {
                let Some(model) = w.csv_models.get(&self.tab_id) else { return false; };
                let Some(input) = model.cell_input.as_ref() else { return false; };
                input.focus_handle == self.focus && model.editing_cell == *cell
                    && input.buffer.len() == *len && (input.anchor, input.cursor) == self.selection
                    && input.buffer.get(self.marked.clone()).is_some()
            }
        }
    }

    fn valid(&self, w: &Workspace, window: &Window) -> bool {
        self.context_valid(w, window) && match self.target {
            ImeTarget::Source { .. } => true,
            ImeTarget::Csv { .. } => w.csv_models.get(&self.tab_id).and_then(|model| model.cell_input.as_ref())
                .and_then(|input| input.buffer.get(self.marked.clone())) == Some(self.text.as_str()),
        }
    }
}

impl Workspace {
    pub(crate) fn is_composing_text(&self, window: &Window) -> bool {
        self.ime_composition.as_ref().is_some_and(|state| state.context_valid(self, window))
    }

    pub(crate) fn retire_ime_composition(&mut self, window: &Window, _cx: &mut Context<Self>) {
        if self.ime_composition.as_ref().is_some_and(|state| !state.context_valid(self, window)) {
            // Preserve visible preedit and its Undo entry. Never finalize a
            // macro later in a different document or a newer field.
            self.ime_composition = None;
        }
    }

    pub(super) fn ime_marked_range(&self, window: &Window) -> Option<Range<usize>> {
        let state = self.ime_composition.as_ref().filter(|state| state.valid(self, window))?;
        if state.text.is_empty() { return None; }
        match state.target {
            ImeTarget::Source { .. } => {
                let rope = &self.tabs.get(self.active)?.buffer.rope;
                Some(rope_byte_to_utf16(rope, state.marked.start)..rope_byte_to_utf16(rope, state.marked.end))
            }
            ImeTarget::Csv { .. } => {
                let input = self.csv_models.get(&state.tab_id)?.cell_input.as_ref()?;
                Some(str_byte_to_utf16(&input.buffer, state.marked.start)..str_byte_to_utf16(&input.buffer, state.marked.end))
            }
        }
    }

    fn record_ime_text(&self, text: &str) {
        if text.is_empty() { return; }
        let event = crate::macros::MacroEvent::InsertText(text.to_string());
        #[cfg(test)]
        if let Some(sink) = &self.ime_record_sink {
            let _ = sink.send(event);
            return;
        }
        crate::macros::record(event);
    }

    fn finish_ime_state(&mut self, mut state: ImeComposition, allow_completion: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !state.valid(self, window) { return; }
        let ImeTarget::Source { before_revision, undo_index, prior_redo, .. } = &mut state.target else { return; };
        let Some(tab) = self.tabs.get_mut(self.active) else { return; };
        let Some(UndoEntry::Single(op)) = tab.undo_stack.get(*undo_index) else { return; };
        if op.removed == op.inserted {
            // The exact aggregate edit returned to its original bytes.
            // Restore identity and the redo branch lost during preedit.
            tab.buffer.raw_revision = before_revision.clone();
            tab.buffer.refresh_dirty();
            tab.undo_stack.pop();
            tab.redo_stack = std::mem::take(prior_redo);
            tab.observe_content_activity();
            self.observe_automatic_title_input(self.active, cx);
        } else {
            let text = op.inserted.clone();
            let complete = allow_completion && op.removed.is_empty() && !text.is_empty()
                && text.chars().all(super::autocomplete::is_word_char);
            self.record_ime_text(&text);
            if complete { self.maybe_auto_trigger_autocomplete(window, cx); }
            self.update_function_hint_after_input(&text, window, cx);
        }
        cx.notify();
    }

    pub(super) fn finish_ime_composition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.ime_composition.take() { self.finish_ime_state(state, true, window, cx); }
    }

    pub(super) fn replace_ime_text(
        &mut self, range_utf16: Option<Range<usize>>, text: &str,
        selected_utf16: Option<Range<usize>>, mark: bool,
        window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return true; };
        let initial_tab_id = tab.session_id;
        let focus = self.input_focus(window);
        let source = focus == InputFocus::Editor;
        if !matches!(focus, InputFocus::Editor | InputFocus::CsvCell | InputFocus::CsvGrid) { return false; }
        if !window.is_window_active() { return true; }
        if source && (!self.focus_handle.is_focused(window) || tab.view_mode != TabViewMode::Edit
            || tab.buffer.read_only || tab.buffer.loading.is_some() || tab.block_selection.is_some()
            || tab.edit_transaction.is_some() || self.bulk_edit)
        {
            self.ime_composition = None;
            self.set_status(if tab.buffer.read_only { "Buffer is read-only" } else { "IME composition is unavailable in this view" });
            cx.notify();
            return true;
        }
        let normalized = if source {
            let text = if crate::settings::snapshot().prevent_c0_input {
                super::input_handler::strip_c0_controls(text)
            } else { text.to_string() };
            crate::text::eol_convert::to_rope_eols(&text).into_owned()
        } else {
            text.chars().filter(|c| if focus == InputFocus::CsvGrid { !c.is_control() }
                else { *c != '\n' && *c != '\r' && *c != '\t' }).collect::<String>()
        };
        if normalized.is_empty() && !text.is_empty() { return true; }
        let mut previous = self.ime_composition.take();
        if previous.as_ref().is_some_and(|state| !state.valid(self, window)) {
            self.set_status("IME composition changed; restart input to continue");
            cx.notify();
            return true;
        }
        let mut byte_range = if source {
            range_utf16.map(|range| super::input_handler::utf16_range_to_bytes(&self.buffer().rope, range))
                .or_else(|| previous.as_ref().map(|state| state.marked.clone()))
                .unwrap_or_else(|| crate::buffer::char_boundary_range(&self.buffer().rope, self.selection().ordered()))
        } else if focus == InputFocus::CsvGrid {
            if normalized.is_empty() { return true; }
            crate::ui::csv_doc::actions::begin_cell_edit_with_seed(self, &normalized, window, cx);
            0..0
        } else {
            let Some(input) = self.csv_models.get(&initial_tab_id).and_then(|model| model.cell_input.as_ref()) else { return true; };
            range_utf16.map(|range| {
                let a = str_utf16_to_byte(&input.buffer, range.start);
                let b = str_utf16_to_byte(&input.buffer, range.end);
                a.min(b)..a.max(b)
            }).or_else(|| previous.as_ref().map(|state| state.marked.clone())).unwrap_or_else(|| {
                let a = super::edits::floor_char_boundary_in(&input.buffer, input.anchor);
                let b = super::edits::floor_char_boundary_in(&input.buffer, input.cursor);
                a.min(b)..a.max(b)
            })
        };
        if previous.as_ref().is_some_and(|state| byte_range.start < state.marked.start || byte_range.end > state.marked.end) {
            if let Some(state) = previous.take() { self.finish_ime_state(state, false, window, cx); }
        }
        let tab_id = self.tabs[self.active].session_id;
        let mode = self.tabs[self.active].view_mode;
        let target = if source {
            if let Some(ImeComposition { target: ImeTarget::Source { before_revision, undo_index, start, inserted_len, prior_redo, .. }, .. }) = previous {
                let Some(total) = inserted_len.checked_sub(byte_range.end - byte_range.start)
                    .and_then(|len| len.checked_add(normalized.len())) else { return true; };
                let Some(end) = start.checked_add(total) else { return true; };
                let op = self.apply_edit(byte_range.clone(), &normalized, cx);
                let Some(text) = self.buffer().rope.get_byte_slice(start..end).map(|slice| slice.to_string()) else { return true; };
                let Some(UndoEntry::Single(entry)) = self.tabs[self.active].undo_stack.get_mut(undo_index) else { return true; };
                entry.inserted = text;
                entry.after_revision = op.after_revision.clone();
                ImeTarget::Source { revision: op.after_revision, before_revision, undo_index, start, inserted_len: total, prior_redo }
            } else {
                if byte_range.is_empty() && normalized.is_empty() { return true; }
                let undo_index = self.tabs[self.active].undo_stack.len();
                let prior_redo = std::mem::take(&mut self.tabs[self.active].redo_stack);
                self.replace_byte_range(byte_range.clone(), &normalized, cx);
                let Some(UndoEntry::Single(op)) = self.tabs[self.active].undo_stack.get(undo_index) else { return true; };
                ImeTarget::Source {
                    revision: op.after_revision.clone(), before_revision: op.before_revision.clone(), undo_index,
                    start: op.byte_start, inserted_len: op.inserted.len(), prior_redo,
                }
            }
        } else {
            let Some(model) = self.csv_models.get_mut(&tab_id) else { return true; };
            let Some(input) = model.cell_input.as_mut() else { return true; };
            if focus != InputFocus::CsvGrid {
                let (cursor, anchor) = (input.cursor, input.anchor);
                let units = str_byte_to_utf16(&input.buffer, byte_range.start)..str_byte_to_utf16(&input.buffer, byte_range.end);
                super::input_handler::replace_input_utf16(&mut input.buffer, &mut input.cursor, &mut input.anchor, cursor, anchor, Some(units), &normalized);
            } else { byte_range = 0..0; }
            ImeTarget::Csv { cell: model.editing_cell, len: input.buffer.len() }
        };
        let marked = byte_range.start..byte_range.start.saturating_add(normalized.len());
        let relative = selected_utf16.unwrap_or_else(|| {
            let end = normalized.encode_utf16().count(); end..end
        });
        let anchor = marked.start.saturating_add(str_utf16_to_byte(&normalized, relative.start));
        let cursor = marked.start.saturating_add(str_utf16_to_byte(&normalized, relative.end));
        let handle = if source {
            let tab = &mut self.tabs[self.active];
            tab.selection.anchor = anchor; tab.selection.head = cursor;
            self.focus_handle.clone()
        } else {
            let Some(input) = self.csv_models.get_mut(&tab_id).and_then(|model| model.cell_input.as_mut()) else { return true; };
            input.anchor = anchor; input.cursor = cursor;
            input.focus_handle.clone()
        };
        let state = ImeComposition { tab_id, mode, focus: handle, target, marked, selection: (anchor, cursor), text: normalized };
        if mark && !state.text.is_empty() {
            self.ime_composition = Some(state);
        } else { self.finish_ime_state(state, true, window, cx); }
        self.schedule_autosave(cx);
        cx.notify();
        true
    }
}

#[cfg(test)]
#[path = "ime_completion_tests.rs"]
mod completion_tests;
