use std::ops::Range;

use gpui::Context;

use crate::buffer::{EditOp, EditTransaction, UndoEntry};
use crate::workspace::Workspace;

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
        let total = tab.buffer.rope.len_bytes();
        let start = byte_range.start.min(total);
        let end = byte_range.end.min(total).max(start);

        let removed = tab.buffer.slice_bytes(start..end);
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
        if let Some(marker) = tab.begin_select_marker {
            let removed_len = end - start;
            let inserted_len = new_text.len();
            if marker >= start && marker <= end {
                tab.begin_select_marker = None;
            } else if marker > end {
                let shifted = marker as isize - removed_len as isize + inserted_len as isize;
                tab.begin_select_marker = Some(shifted.max(0) as usize);
            }
        }

        // Syntax-highlight refresh is deferred to `after_edit_flush` so a
        // batched Replace All / multi-cursor run reparses ONCE at the end
        // instead of once per edit — a full reparse per edit is O(n²).

        tab.buffer.dirty = true;
        // Any rope mutation invalidates the cached wrap layout — visual rows
        // depend directly on byte content. Cache is rebuilt lazily on the
        // next paint when `word_wrap` is on.
        tab.wrap_cache = None;
        let new_pos = start + new_text.len();
        tab.selection.collapse_to(new_pos);
        tab.desired_col_x = None;

        // Inside a Replace-All / multi-cursor transaction (`edit_transaction`)
        // or an undo/redo group replay (`bulk_edit`), defer the heavyweight
        // post-edit work — full syntax reparse, caret-visibility scroll,
        // repaint, and the synchronous session + autosave disk writes — to a
        // single `after_edit_flush` once the whole batch lands. Running it per
        // edit turned an N-match Replace All into N disk writes + N full
        // reparses (O(n²)) — the cause of ~15s replaces across ~100 lines.
        let in_bulk = self.bulk_edit || self.tabs[active].edit_transaction.is_some();

        let op = EditOp {
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
    /// entry. Nested `begin_edit_group` calls are a no-op (the outer
    /// transaction stays in charge), so callers are free to wrap helpers
    /// that themselves open transactions without doubling up.
    pub(crate) fn begin_edit_group(&mut self) {
        let tab = self.active_tab_mut();
        if tab.edit_transaction.is_some() {
            return;
        }
        let anchor = tab.selection.anchor;
        let head = tab.selection.head;
        tab.edit_transaction = Some(EditTransaction {
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
                // Note: the Single variant uses the op's own `sel_before_*` for
                // caret restore, which equals the transaction's saved values
                // because the transaction snapshots before the first op runs and
                // each `apply_edit` records `sel_before_*` from the same caret.
                let mut ops = txn.ops;
                tab.undo_stack
                    .push(UndoEntry::Single(ops.pop().expect("len==1")));
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
        let undo_range = op.byte_start..op.byte_start + op.inserted.len();
        let _ = self.apply_edit(undo_range, &op.removed, cx);
    }

    /// Redo a single `EditOp` by replaying it forward. Same role as
    /// `replay_undo_op` but the other direction.
    fn replay_redo_op(&mut self, op: &EditOp, cx: &mut Context<Self>) {
        let redo_range = op.byte_start..op.byte_start + op.removed.len();
        let _ = self.apply_edit(redo_range, &op.inserted, cx);
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
    ) {
        match entry {
            UndoEntry::Single(op) => {
                self.replay_undo_op(op, cx);
                let tab = self.active_tab_mut();
                tab.selection.anchor = op.sel_before_anchor;
                tab.selection.head = op.sel_before_head;
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
                self.after_edit_flush(cx);
            }
        }
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
    ) {
        match entry {
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
    }

    // ---- Navigation helpers ----------------------------------------------------------------

    pub(crate) fn prev_char_byte(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
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
        let line = rope.byte_to_line(byte);
        rope.line_to_byte(line)
    }

    pub(crate) fn line_end_byte(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let line = rope.byte_to_line(byte);
        let line_start = rope.line_to_byte(line);
        let line_slice = rope.line(line);
        let raw = line_slice.to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        line_start + trimmed.len()
    }

    pub(crate) fn prev_word_boundary(&self, byte: usize) -> usize {
        let rope = &self.buffer().rope;
        let mut idx = rope.byte_to_char(byte);
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
        let mut idx = rope.byte_to_char(byte);
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
        // Linear cursor movement collapses any in-flight block selection.
        // Block-aware actions call `block_extend` directly — they bypass
        // this funnel and keep the block alive.
        self.clear_block_selection();
        let total = self.buffer().rope.len_bytes();
        let target = byte.min(total);
        let tab = self.active_tab_mut();
        if extend {
            tab.selection.head = target;
        } else {
            tab.selection.collapse_to(target);
        }
        if reset_col {
            tab.desired_col_x = None;
        }
        self.caret_visible = true;
    }

    /// Helper used by every cursor-moving action handler in place of a
    /// trailing `cx.notify()`. Auto-scrolls the new cursor into view, then
    /// refreshes the smart-highlight band so the green halo on the word at
    /// cursor stays in sync, then notifies, in one call.
    pub(crate) fn commit_caret(&mut self, cx: &mut Context<Self>) {
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
                let tgt_line_start = rope.line_to_byte(target_logical as usize);
                let tgt_line_len = {
                    let line_slice = rope.line(target_logical as usize);
                    let raw = line_slice.to_string();
                    raw.trim_end_matches(['\n', '\r']).len()
                };
                let cur_line_start = rope.line_to_byte(cur_logical as usize);
                let col_bytes = head.saturating_sub(cur_line_start);
                tgt_line_start + col_bytes.min(tgt_line_len)
            };

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
                    let local = head - cur.line_byte_start;
                    cur.shaped.x_for_index(local)
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
            let rope = &self.buffer().rope;
            let cur_line_start = rope.line_to_byte(cur_logical as usize);
            let col_bytes = head - cur_line_start;
            let tgt_line_start = rope.line_to_byte(target_logical as usize);
            let tgt_line_len = {
                let line_slice = rope.line(target_logical as usize);
                let raw = line_slice.to_string();
                raw.trim_end_matches(['\n', '\r']).len()
            };
            tgt_line_start + col_bytes.min(tgt_line_len)
        };

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
