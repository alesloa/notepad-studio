use std::ops::Range;

use gpui::Context;

use crate::buffer::{EditOp, EditTransaction, UndoEntry};
use crate::workspace::Workspace;
use super::history::{HistoryDirection, text_history_matches};

impl Workspace {
    // ---- Buffer mutation -------------------------------------------------------------------

    pub(crate) fn apply_edit(
        &mut self,
        byte_range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> EditOp {
        let active = self.active;
        let tab = &mut self.tabs[active];
        // Callers derive ranges from find results, undo history (possibly
        // restored from disk), macros and plugins. Clamp to the rope and land
        // both ends on a char boundary so the removal, the recorded undo op
        // and the shifted bookmarks/marks all agree on the same bytes.
        let Range { start, end } =
            crate::buffer::char_boundary_range(&tab.buffer.rope, byte_range);

        let removed = tab.buffer.slice_bytes(start..end);
        let marker = tab.begin_select_marker.filter(|&byte| byte <= tab.buffer.rope.len_bytes()
            && crate::buffer::floor_char_boundary(&tab.buffer.rope, byte) == byte);
        let before_revision = tab.buffer.raw_revision.clone();
        let sel_before_anchor = tab.selection.anchor;
        let sel_before_head = tab.selection.head;

        let char_start = tab.buffer.rope.byte_to_char(start);
        let char_end = tab.buffer.rope.byte_to_char(end);
        if char_start != char_end {
            tab.buffer.rope.remove(char_start..char_end);
        }
        if !new_text.is_empty() {
            tab.buffer.rope.insert(char_start, new_text);
        }
        // Rope just mutated — drop the find / smart-highlight scan cache so
        // the next `flat_text()` call rebuilds against the new content.
        tab.buffer.invalidate_flat_text();

        // Track bookmarks across the edit: shift offsets that sit past the
        // edit, drop any that sat inside the deleted region, then snap each
        // surviving bookmark byte to its current line start so lookups by
        // line-start-byte stay exact-match.
        tab.buffer
            .shift_bookmarks_for_edit(start, end, new_text.len());
        tab.buffer.normalize_bookmarks();
        // Track Mark layer ranges the same way — overlapping marks drop, marks
        // past the edit shift by the size delta. Keeps "Mark All" highlights
        // glued to their text across typing / paste / undo.
        tab.buffer.shift_marks_for_edit(start, end, new_text.len());

        // Begin/End Select marker stays anchored to its character across
        // edits. Mirrors NPP `ScintillaEditView::updateBeginEndSelectPosition`:
        // edits before the marker shift it by the size delta, edits that
        // straddle the marker drop it.
        tab.begin_select_marker = marker.and_then(|marker| {
            if marker >= start && marker <= end {
                None
            } else if marker > end {
                marker.checked_sub(end - start).and_then(|byte| byte.checked_add(new_text.len()))
            } else {
                Some(marker)
            }
        });

        // Syntax-highlight refresh is deferred to `after_edit_flush` so a
        // batched Replace All / multi-cursor run reparses ONCE at the end
        // instead of once per edit — a full reparse per edit is O(n²).

        if removed != new_text {
            tab.buffer.raw_revision = crate::buffer::ContentRevision::default();
        }
        tab.buffer.refresh_dirty();
        // Any rope mutation invalidates the cached wrap layout — visual rows
        // depend directly on byte content. Cache is rebuilt lazily on the
        // next paint when `word_wrap` is on.
        tab.wrap_cache = None;
        let new_pos = start + new_text.len();
        tab.selection.collapse_to(new_pos);
        tab.desired_col_x = None;
        let after_revision = tab.buffer.raw_revision.clone();

        // Inside a Replace-All / multi-cursor transaction (`edit_transaction`)
        // or an undo/redo group replay (`bulk_edit`), defer the heavyweight
        // post-edit work — full syntax reparse, caret-visibility scroll,
        // repaint, and the synchronous session + autosave disk writes — to a
        // single `after_edit_flush` once the whole batch lands. Running it per
        // edit turned an N-match Replace All into N disk writes + N full
        // reparses (O(n²)) — the cause of ~15s replaces across ~100 lines.
        let in_bulk = self.bulk_edit || self.tabs[active].edit_transaction.is_some();

        let op = EditOp {
            before_revision,
            after_revision,
            byte_start: start,
            removed,
            inserted: new_text.to_string(),
            sel_before_anchor,
            sel_before_head,
        };

        if !in_bulk {
            self.after_edit_flush(cx);
        }

        op
    }

    /// Heavyweight post-edit work shared by a single edit and the tail of a
    /// batched run (Replace All, multi-cursor, block ops, undo/redo of a
    /// composite group). Runs ONCE per logical change: refresh syntax
    /// highlights, scroll the caret into view, repaint, flush the session
    /// manifest, and schedule the debounced autosave. Keeping this out of the
    /// per-edit hot loop is what makes batched edits fast.
    pub(crate) fn after_edit_flush(&mut self, cx: &mut Context<Self>) {
        self.retire_source_input();
        self.cancel_prepared_edit(cx);
        // The synchronous edit-commit path stays CHEAP so the edit, caret, and
        // repaint land this frame and the UI thread never stalls. The two
        // heavyweight post-edit jobs — a full syntax reparse and session
        // persistence — are pushed to debounced async tasks below.
        //
        // Why: a 580-match Replace All (or its undo) used to run BOTH of these
        // synchronously right here — `reparse_highlights()` re-stringifies and
        // tree-sitter-parses the whole document, and `save_session_eager()`
        // serializes the entire undo stack (grows with op count) plus writes
        // the full doc to disk. On a 5k-line buffer that blocked the UI thread
        // for seconds → the loading spinner. A single keystroke tolerated it;
        // a batched run did not. Zed keeps highlighting async/incremental and
        // persistence off the edit path for exactly this reason.
        self.refresh_autocomplete_after_edit(cx);
        self.invalidate_function_hint_after_edit(cx);
        self.last_lines.clear();
        self.caret_visible = true;
        self.status_message = None;
        self.ensure_cursor_visible(cx);
        cx.notify();
        // Debounced (~100ms) syntax reparse + eager untitled persistence —
        // both off the synchronous edit path so batched edits commit instantly.
        self.schedule_reparse(cx);
        // Debounced (250ms) full save — already async; flushes path-backed
        // dirty `.bak`s and refreshes the manifest. Persists untitled buffers
        // too via BakPolicy::Full.
        self.schedule_autosave(cx);
    }

    pub(crate) fn replace_byte_range(
        &mut self,
        byte_range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) {
        if byte_range.start == byte_range.end && new_text.is_empty() {
            return;
        }
        if self.buffer().read_only {
            self.set_status("Buffer is read-only");
            cx.notify();
            return;
        }
        let op = self.apply_edit(byte_range, new_text, cx);
        let in_transaction;
        {
            let tab = self.active_tab_mut();
            in_transaction = tab.edit_transaction.is_some();
            if let Some(txn) = tab.edit_transaction.as_mut() {
                // Inside an explicit `begin_edit_group`. Buffer the op until the
                // matching `end_edit_group` flushes them as one composite entry.
                txn.ops.push(op);
            } else {
                tab.undo_stack.push(UndoEntry::Single(op));
            }
            // Any fresh edit invalidates the redo timeline — including individual
            // ops appended inside an open transaction.
            tab.redo_stack.clear();
        }
        // Buffer mutation invalidates the find-match cache. Skip inside an
        // open transaction (Replace All wraps its loop and calls
        // `refresh_matches` once at the end — refreshing per-op would be
        // O(n²)).
        if !in_transaction && self.find_state.open && !self.find_state.query.is_empty() {
            self.refresh_matches();
        }
    }

    /// Open an undo transaction on the active tab. Until the matching
    /// `end_edit_group` call, every `replace_byte_range` op is collected into
    /// the same composite undo entry instead of pushing a fresh per-edit
    /// entry. Balanced nested groups leave the outer transaction in charge
    /// until its matching end, including commands called during macro replay.
    pub(crate) fn begin_edit_group(&mut self) {
        let tab = self.active_tab_mut();
        if let Some(txn) = tab.edit_transaction.as_mut() {
            txn.depth = txn.depth.saturating_add(1);
            return;
        }
        let anchor = tab.selection.anchor;
        let head = tab.selection.head;
        tab.edit_transaction = Some(EditTransaction {
            depth: 1,
            ops: Vec::new(),
            sel_before_anchor: anchor,
            sel_before_head: head,
        });
    }

    /// Close the active tab's open edit-group transaction and push it to the
    /// undo stack as a single composite entry. Empty transactions (no ops
    /// happened between begin/end) just clear without leaving a stack entry,
    /// so wrapping a Replace All that finds zero matches does not waste an
    /// undo press.
    pub(crate) fn end_edit_group(&mut self, cx: &mut Context<Self>) {
        {
            let tab = self.active_tab_mut();
            if let Some(txn) = tab.edit_transaction.as_mut() {
                if txn.depth > 1 {
                    txn.depth -= 1;
                    return;
                }
            }
            let Some(txn) = tab.edit_transaction.take() else {
                return;
            };
            if txn.ops.is_empty() {
                return;
            }
            if txn.ops.len() == 1 {
                // Degenerate group — only one op landed, so collapse to Single
                // for symmetry with the per-edit case. Keeps the undo history
                // tidy and avoids an extra layer of indirection at undo time.
                //
                // Navigation before the sole edit still belongs to the group.
                // Undo returns to the selection where the group opened.
                let mut ops = txn.ops;
                if let Some(mut op) = ops.pop() {
                    op.sel_before_anchor = txn.sel_before_anchor;
                    op.sel_before_head = txn.sel_before_head;
                    tab.undo_stack.push(UndoEntry::Single(op));
                }
            } else {
                tab.undo_stack.push(UndoEntry::Group {
                    ops: txn.ops,
                    sel_before_anchor: txn.sel_before_anchor,
                    sel_before_head: txn.sel_before_head,
                });
            }
        }
        // The batched edits ran with their per-edit flush suppressed (see
        // `apply_edit`); do the single deferred reparse + scroll + repaint +
        // disk flush now that the whole group has landed.
        self.after_edit_flush(cx);
    }

    /// Undo a single `EditOp` by replaying its inverse. Pulled out of the
    /// per-edit `undo` action so the same primitive can walk an entire
    /// `Group` in reverse for composite-entry undo.
    fn replay_undo_op(&mut self, op: &EditOp, cx: &mut Context<Self>) {
        // `byte_start` may come from an undo file on disk: no overflow.
        let undo_range = op.byte_start..op.byte_start.saturating_add(op.inserted.len());
        let _ = self.apply_edit(undo_range, &op.removed, cx);
        let buffer = &mut self.active_tab_mut().buffer;
        buffer.raw_revision = op.before_revision.clone();
        buffer.refresh_dirty();
    }

    /// Redo a single `EditOp` by replaying it forward. Same role as
    /// `replay_undo_op` but the other direction.
    fn replay_redo_op(&mut self, op: &EditOp, cx: &mut Context<Self>) {
        let redo_range = op.byte_start..op.byte_start.saturating_add(op.removed.len());
        let _ = self.apply_edit(redo_range, &op.inserted, cx);
        let buffer = &mut self.active_tab_mut().buffer;
        buffer.raw_revision = op.after_revision.clone();
        buffer.refresh_dirty();
    }

    /// Replay an entire `UndoEntry` in reverse. Used by the `EditUndo`
    /// handler. Composite groups walk their ops back-to-front so each
    /// inverse lands on a buffer that still matches the byte offsets the op
    /// was recorded against. Caret is restored to the pre-entry position
    /// the entry stored at record time.
    pub(crate) fn replay_undo_entry(
        &mut self,
        entry: &UndoEntry,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(entry, UndoEntry::Prepared(_)) {
            self.set_status("Use the Undo action to prepare this large indentation change");
            return false;
        }
        if !text_history_matches(self.buffer(), entry, HistoryDirection::Undo) {
            self.active_tab_mut().buffer.forget_savepoint();
            self.set_status("Undo history no longer matches this document; no changes applied");
            cx.notify();
            return false;
        }
        match entry {
            UndoEntry::Prepared(_) => return false,
            UndoEntry::Single(op) => {
                self.replay_undo_op(op, cx);
                let tab = self.active_tab_mut();
                tab.selection.anchor = op.sel_before_anchor;
                tab.selection.head = op.sel_before_head;
                tab.selection = tab.selection.clamped(&tab.buffer.rope);
            }
            UndoEntry::Group {
                ops,
                sel_before_anchor,
                sel_before_head,
            } => {
                // Replay every inverse op under the bulk guard so each one
                // skips the per-edit reparse + disk write; flush once after
                // the group lands. Without this, undoing a Replace All was as
                // slow as the Replace All itself (O(n²)).
                self.bulk_edit = true;
                for op in ops.iter().rev() {
                    self.replay_undo_op(op, cx);
                }
                self.bulk_edit = false;
                let tab = self.active_tab_mut();
                tab.selection.anchor = *sel_before_anchor;
                tab.selection.head = *sel_before_head;
                tab.selection = tab.selection.clamped(&tab.buffer.rope);
                self.after_edit_flush(cx);
            }
        }
        true
    }

    /// Replay an entire `UndoEntry` forward. Used by `EditRedo`. Groups walk
    /// their ops front-to-back so each replay lands on the same byte offsets
    /// the original edit ran against. Caret is left wherever `apply_edit`
    /// places it after the last op — matches the post-edit caret the user
    /// saw the moment the original action committed.
    pub(crate) fn replay_redo_entry(
        &mut self,
        entry: &UndoEntry,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(entry, UndoEntry::Prepared(_)) {
            self.set_status("Use the Redo action to prepare this large indentation change");
            return false;
        }
        if !text_history_matches(self.buffer(), entry, HistoryDirection::Redo) {
            self.active_tab_mut().buffer.forget_savepoint();
            self.set_status("Redo history no longer matches this document; no changes applied");
            cx.notify();
            return false;
        }
        match entry {
            UndoEntry::Prepared(_) => return false,
            UndoEntry::Single(op) => {
                self.replay_redo_op(op, cx);
            }
            UndoEntry::Group { ops, .. } => {
                // Same bulk guard as the undo path — replay forward with the
                // per-edit flush suppressed, then flush once.
                self.bulk_edit = true;
                for op in ops.iter() {
                    self.replay_redo_op(op, cx);
                }
                self.bulk_edit = false;
                self.after_edit_flush(cx);
            }
        }
        true
    }

    // ---- Navigation helpers ----------------------------------------------------------------

    // Every helper below clamps `byte` to the rope first: callers pass carets
    // and offsets that can outlive a shrinking edit, and ropey panics on an
    // out-of-range index.

    pub(crate) fn prev_char_byte(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let byte = byte.min(rope.len_bytes());
        if byte == 0 {
            return 0;
        }
        let char_idx = rope.byte_to_char(byte);
        if char_idx == 0 {
            return 0;
        }
        rope.char_to_byte(char_idx - 1)
    }

    pub(crate) fn next_char_byte(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let total_bytes = rope.len_bytes();
        if byte >= total_bytes {
            return total_bytes;
        }
        let char_idx = rope.byte_to_char(byte);
        let total_chars = rope.len_chars();
        if char_idx + 1 > total_chars {
            return total_bytes;
        }
        rope.char_to_byte(char_idx + 1)
    }

    pub(crate) fn line_start_byte(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let line = rope.byte_to_line(byte.min(rope.len_bytes()));
        rope.line_to_byte(line)
    }

    pub(crate) fn line_end_byte(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let line = rope.byte_to_line(byte.min(rope.len_bytes()));
        let line_start = rope.line_to_byte(line);
        let line_slice = rope.line(line);
        let raw = line_slice.to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        line_start + trimmed.len()
    }

    pub(crate) fn prev_word_boundary(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let mut idx = rope.byte_to_char(byte.min(rope.len_bytes()));
        if idx == 0 {
            return 0;
        }
        let mut entered_word = false;
        while idx > 0 {
            let c = rope.char(idx - 1);
            if c.is_whitespace() {
                if entered_word {
                    break;
                }
                idx -= 1;
            } else {
                entered_word = true;
                idx -= 1;
            }
        }
        rope.char_to_byte(idx)
    }

    pub(crate) fn next_word_boundary(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let total = rope.len_chars();
        let mut idx = rope.byte_to_char(byte.min(rope.len_bytes()));
        let mut entered_word = false;
        while idx < total {
            let c = rope.char(idx);
            if c.is_whitespace() {
                if entered_word {
                    break;
                }
                idx += 1;
            } else {
                entered_word = true;
                idx += 1;
            }
        }
        rope.char_to_byte(idx)
    }

    /// Word-class for a single char. Notepad++/Scintilla default classifier:
    /// alphanumeric + `_` are word chars; whitespace is its own class;
    /// everything else is "punctuation". Double-click expands across a
    /// contiguous run of the same class, so clicking a `,` selects only the
    /// comma run, clicking inside `foo` selects `foo`, etc.
    fn word_class(c: char) -> u8 {
        if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            2
        } else {
            3
        }
    }

    /// Range of the word under `byte` for double-click word selection.
    /// Returns the contiguous run of same-class chars containing `byte`
    /// (or starting at `byte` when the cursor sits between two chars of
    /// different classes — favors the left char to match Notepad++).
    pub(crate) fn word_range_at(&self, byte: usize) -> Range<usize> {
        let rope = &self.buffer().rope;
        let total_bytes = rope.len_bytes();
        if total_bytes == 0 {
            return 0..0;
        }
        let total_chars = rope.len_chars();
        let clamped = byte.min(total_bytes);
        let char_idx = rope.byte_to_char(clamped);
        // Pick the char whose class drives the run. Prefer the char at the
        // cursor; if the cursor is at the very end of the doc, fall back to
        // the previous char.
        let pivot = if char_idx >= total_chars {
            if total_chars == 0 {
                return 0..0;
            }
            total_chars - 1
        } else {
            char_idx
        };
        let pivot_char = rope.char(pivot);
        let target_class = Self::word_class(pivot_char);

        // Don't auto-select runs of whitespace on double-click — feels wrong
        // and matches NPP. Caller will collapse to caret when len == 0.
        if target_class == 2 {
            let byte_at_pivot = rope.char_to_byte(pivot);
            return byte_at_pivot..byte_at_pivot;
        }

        let mut start_char = pivot;
        while start_char > 0 && Self::word_class(rope.char(start_char - 1)) == target_class {
            start_char -= 1;
        }
        let mut end_char = pivot + 1;
        while end_char < total_chars && Self::word_class(rope.char(end_char)) == target_class {
            end_char += 1;
        }
        rope.char_to_byte(start_char)..rope.char_to_byte(end_char)
    }

    /// Range of the line under `byte` for triple-click line selection. Spans
    /// the line content but excludes the trailing CR/LF — matches Notepad++.
    pub(crate) fn line_range_at(&self, byte: usize) -> Range<usize> {
        let start = self.line_start_byte(byte);
        let end = self.line_end_byte(byte);
        start..end
    }

    pub(crate) fn move_to_byte(&mut self, byte: usize, extend: bool, reset_col: bool) {
        let before = (self.selection().anchor, self.selection().head, self.block_selection().is_some());
        // Linear cursor movement collapses any in-flight block selection.
        // Block-aware actions call `block_extend` directly — they bypass
        // this funnel and keep the block alive.
        self.clear_block_selection();
        let target = crate::buffer::floor_char_boundary(&self.buffer().rope, byte);
        let tab = self.active_tab_mut();
        if extend {
            tab.selection.head = target;
        } else {
            tab.selection.collapse_to(target);
        }
        if reset_col {
            tab.desired_col_x = None;
        }
        if before != (tab.selection.anchor, tab.selection.head, false) {
            self.retire_source_input();
        }
        self.caret_visible = true;
    }

    /// Helper used by every cursor-moving action handler in place of a
    /// trailing `cx.notify()`. Auto-scrolls the new cursor into view, then
    /// refreshes the smart-highlight band so the green halo on the word at
    /// cursor stays in sync, then notifies, in one call.
    pub(crate) fn commit_caret(&mut self, cx: &mut Context<Self>) {
        self.retire_source_input();
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.ensure_cursor_visible(cx);
        self.refresh_smart_highlight();
        self.refresh_tag_match();
        cx.notify();
    }

    pub(crate) fn locate_line_for_byte(&self, byte: usize) -> Option<usize> {
        self.last_lines
            .iter()
            .position(|l| byte >= l.line_byte_start && byte <= l.line_byte_start + l.line_byte_len)
    }

    pub(crate) fn move_vertical(&mut self, delta: i32, extend: bool, cx: &mut Context<Self>) {
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.clear_block_selection();
        let head = self.selection().head;

        // Path A: word wrap on — move by *visual* row. The visible visual
        // rows live in `last_lines`; locate the source row, compute the
        // target row index, and either land on a visible row (precise X
        // preservation) or fall back to the cache's row ranges (approximate
        // — drops to the row's start byte until the next paint).
        if self.word_wrap {
            let total_visual = self.total_visual_rows() as i32;
            let cur_visual_idx = self.locate_line_for_byte(head);
            // Convert source row → absolute visual-row index in the doc, so
            // we can clamp before consulting the cache.
            let cur_doc_row = match cur_visual_idx {
                Some(_) => self.visual_row_for_byte(head),
                None => self.visual_row_for_byte(head),
            }
            .unwrap_or_else(|| self.buffer().rope.byte_to_line(head)) as i32;
            let target_doc_row = cur_doc_row + delta;
            if target_doc_row < 0 {
                self.move_to_byte(0, extend, true);
                self.ensure_cursor_visible(cx);
                cx.notify();
                return;
            }
            if target_doc_row >= total_visual {
                let total = self.buffer().rope.len_bytes();
                self.move_to_byte(total, extend, true);
                self.ensure_cursor_visible(cx);
                cx.notify();
                return;
            }

            // Try to land precisely on a visible visual row using shaped X.
            let desired_x = match self.tabs[self.active].desired_col_x {
                Some(x) => Some(x),
                None => cur_visual_idx.map(|idx| {
                    let cur = &self.last_lines[idx];
                    let local = head.saturating_sub(cur.line_byte_start);
                    cur.shaped.x_for_index(local.min(cur.line_byte_len))
                }),
            };
            if let Some(x) = desired_x {
                self.active_tab_mut().desired_col_x = Some(x);
            }

            // Pull the target row from the wrap cache (always present when
            // word_wrap is true and the cache has been built — at least one
            // paint has happened by now in any realistic scenario).
            let target_row = self
                .tabs[self.active]
                .wrap_cache
                .as_ref()
                .and_then(|c| c.rows.get(target_doc_row as usize).copied());

            let new_byte = if let Some(row) = target_row {
                // If the target row happens to be on screen, use its shaped
                // line for an exact X match. Otherwise drop the cursor at
                // the row's start and let the next paint scroll it in.
                let on_screen = self
                    .last_lines
                    .iter()
                    .find(|l| l.line_byte_start == row.byte_start as usize);
                if let (Some(line), Some(x)) = (on_screen, desired_x) {
                    let local = if line.line_byte_len == 0 {
                        0
                    } else {
                        line.shaped
                            .closest_index_for_x(x)
                            .min(line.line_byte_len)
                    };
                    line.line_byte_start + local
                } else {
                    row.byte_start as usize
                }
            } else {
                // Cache not yet built — fall back to logical-line move so
                // arrow keys still work on the very first frame after wrap
                // toggled on.
                let rope = &self.buffer().rope;
                let total_lines = rope.len_lines().max(1) as i32;
                let cur_logical = rope.byte_to_line(head) as i32;
                let target_logical = (cur_logical + delta).clamp(0, total_lines - 1);
                same_byte_column(rope, head, target_logical as usize)
            };
            let new_byte = crate::buffer::floor_char_boundary(&self.buffer().rope, new_byte);

            let tab = self.active_tab_mut();
            if extend {
                tab.selection.head = new_byte;
            } else {
                tab.selection.collapse_to(new_byte);
            }
            self.caret_visible = true;
            self.ensure_cursor_visible(cx);
            cx.notify();
            return;
        }

        // Path B: word wrap off — original logical-line movement.
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines().max(1) as i32;
        let cur_logical = rope.byte_to_line(head) as i32;
        let target_logical = cur_logical + delta;
        if target_logical < 0 {
            self.move_to_byte(0, extend, true);
            self.ensure_cursor_visible(cx);
            cx.notify();
            return;
        }
        if target_logical >= total_lines {
            let total = rope.len_bytes();
            self.move_to_byte(total, extend, true);
            self.ensure_cursor_visible(cx);
            cx.notify();
            return;
        }

        // When both source and target lines are inside the visible window
        // we can preserve the column via shaped.x_for_index. Otherwise fall
        // back to a logical-only move (same byte-offset within the line)
        // and rely on `ensure_cursor_visible` to scroll in. The fallback
        // drops desired_col_x precision for that one keypress; still leaves
        // the cursor in a sensible spot.
        let (cur_visible_idx, tgt_visible_idx) = (
            self.locate_line_for_byte(head),
            self.last_lines
                .iter()
                .position(|l| l.line_byte_start == self.buffer().rope.line_to_byte(target_logical as usize)),
        );

        let new_byte = if let (Some(cur_idx), Some(tgt_idx)) = (cur_visible_idx, tgt_visible_idx) {
            let desired_x = match self.tabs[self.active].desired_col_x {
                Some(x) => x,
                None => {
                    let cur = &self.last_lines[cur_idx];
                    let local = head.saturating_sub(cur.line_byte_start);
                    cur.shaped.x_for_index(local.min(cur.line_byte_len))
                }
            };
            self.active_tab_mut().desired_col_x = Some(desired_x);
            let target_line = &self.last_lines[tgt_idx];
            let local = if target_line.line_byte_len == 0 {
                0
            } else {
                target_line
                    .shaped
                    .closest_index_for_x(desired_x)
                    .min(target_line.line_byte_len)
            };
            target_line.line_byte_start + local
        } else {
            // Fallback: same column-byte-offset within target line, clamped
            // to the line's printable length.
            same_byte_column(&self.buffer().rope, head, target_logical as usize)
        };
        let new_byte = crate::buffer::floor_char_boundary(&self.buffer().rope, new_byte);

        let tab = self.active_tab_mut();
        if extend {
            tab.selection.head = new_byte;
        } else {
            tab.selection.collapse_to(new_byte);
        }
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }
}

/// Byte offset on `target_line` at the same byte column as `head` on its own
/// line, capped at the target line's printable length (CR/LF excluded) and
/// floored to a char boundary. A byte column carried onto a line with
/// different multi-byte characters can land inside one; a caret there made
/// the next copy or delete panic in ropey's `byte_slice`.
fn same_byte_column(rope: &ropey::Rope, head: usize, target_line: usize) -> usize {
    let head = head.min(rope.len_bytes());
    let target_line = target_line.min(rope.len_lines().saturating_sub(1));
    let cur_line_start = rope.line_to_byte(rope.byte_to_line(head));
    let col_bytes = head.saturating_sub(cur_line_start);
    let tgt_line_start = rope.line_to_byte(target_line);
    let tgt_line_len = {
        let raw = rope.line(target_line).to_string();
        raw.trim_end_matches(['\n', '\r']).len()
    };
    crate::buffer::floor_char_boundary(rope, tgt_line_start + col_bytes.min(tgt_line_len))
}

#[cfg(test)]
mod tests {
    use super::same_byte_column;
    use crate::buffer::{Buffer, char_boundary_range};
    use ropey::Rope;

    #[test]
    fn vertical_move_never_lands_inside_a_char() {
        // Caret at byte column 1 ("a|bc…") moving down onto "éx": byte 1 of
        // that line is the middle of 'é'.
        let rope = Rope::from_str("abcdefgh\néx\n中文\r\n😀\n");
        let line1 = rope.line_to_byte(1);
        assert_eq!(same_byte_column(&rope, 1, 1), line1);
        // Column 2 onto "中文" (3-byte chars) floors to the start of '中'.
        let line2 = rope.line_to_byte(2);
        assert_eq!(same_byte_column(&rope, 2, 2), line2);
        // Column 3 onto "😀" (4 bytes) floors to the line start.
        let line3 = rope.line_to_byte(3);
        assert_eq!(same_byte_column(&rope, 3, 3), line3);
        // Column past the end of a short line caps at its printable end,
        // before the CRLF.
        assert_eq!(same_byte_column(&rope, 7, 2), line2 + 6);
        // Every result is a char boundary the rope can slice at.
        for head in 0..=rope.len_bytes() + 2 {
            for line in 0..rope.len_lines() + 1 {
                let b = same_byte_column(&rope, head, line);
                let _ = rope.byte_slice(b..b);
            }
        }
    }

    #[test]
    fn same_byte_column_on_empty_rope() {
        let rope = Rope::new();
        assert_eq!(same_byte_column(&rope, 10, 3), 0);
    }

    /// Table of ranges `apply_edit` used to pass straight to ropey: past the
    /// end, reversed, and mid-char in each multi-byte width. The normalized
    /// range must slice cleanly and stay inside the rope.
    #[test]
    fn edit_ranges_are_normalized_before_touching_the_rope() {
        let texts = ["", "x", "aé中😀", "a\r\nb", "a\rb", "👨\u{200d}👩\u{200d}👧"];
        for text in texts {
            let rope = Rope::from_str(text);
            let len = rope.len_bytes();
            for start in 0..len + 3 {
                for end in 0..len + 3 {
                    let r = char_boundary_range(&rope, start..end);
                    assert!(r.start <= r.end && r.end <= len, "{text:?} {start}..{end}");
                    let _ = rope.byte_slice(r.clone());
                    let mut buf = Buffer::empty();
                    buf.rope = rope.clone();
                    assert_eq!(buf.slice_bytes(start..end), rope.byte_slice(r).to_string());
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "annotation_edit_tests.rs"]
mod annotation_tests;
