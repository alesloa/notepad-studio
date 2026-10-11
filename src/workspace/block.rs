// Notepad++ column-mode (block / rectangular) selection — model-side
// helpers and edit funnels. Paint lives in `editor_element.rs`; mouse
// alt-drag wiring lives in `mouse.rs` and the per-frame mouse-move
// handler in `editor_element.rs`. This module is the place every
// block-aware code path bottoms out — typing, backspace, delete, cut,
// copy, paste, and the Alt+Shift arrow extensions.

use gpui::{AppContext, ClipboardItem, Context, Window};

use crate::actions::{BlockExtendDown, BlockExtendLeft, BlockExtendRight, BlockExtendUp};
use crate::buffer::BlockSelection;
use crate::tab::InputFocus;
use crate::workspace::Workspace;

/// Sentinel embedded at the start of a clipboard payload that originated
/// from a block-mode Cut/Copy. Lets a same-session Notepad Studio→Notepad Studio paste
/// detect "this came from a column selection" and re-apply it as a column
/// paste. Cross-application paste loses the marker, which is fine — the
/// content still pastes as a normal multi-line string.
pub(crate) const BLOCK_CLIPBOARD_SENTINEL: &str = "\u{0001}NOTEPAD_STUDIO_BLOCK\u{0001}\n";

/// Char column of byte offset `local` inside one rope line (`line_text`
/// still carries its CR/LF terminator). Offsets on or past the terminator
/// map to the printable end of the line; an offset inside a multi-byte char
/// counts as that char's start.
#[cfg(test)]
fn char_col_in_line(line_text: &str, local: usize) -> usize {
    let trimmed = line_text.trim_end_matches(['\n', '\r']);
    let max_local = super::edits::floor_char_boundary_in(trimmed, local);
    trimmed.get(..max_local).map_or(0, |s| s.chars().count())
}

/// Move a block's head corner by `delta_line` rows and `delta_col` chars,
/// keeping the row inside `0..total_lines` and the column at or above 0.
fn step_block_head(
    line: usize,
    col: usize,
    delta_line: i32,
    delta_col: i32,
    total_lines: usize,
) -> (usize, usize) {
    let new_line = line
        .saturating_add_signed(delta_line as isize)
        .min(total_lines.saturating_sub(1));
    let new_col = col.saturating_add_signed(delta_col as isize);
    (new_line, new_col)
}

impl Workspace {
    /// Total char count of `line_idx` in the active buffer (excluding
    /// CR/LF). Used by every block helper that needs to know how far a
    /// given line reaches before falling off into virtual-column space.
    pub(crate) fn line_char_count(&self, line_idx: usize) -> usize {
        super::column_plan::row(&self.buffer().rope, line_idx).map_or(0, |row| row.body.len_chars())
    }

    pub(crate) fn byte_at_line_col(&self, line_idx: usize, col_chars: usize) -> usize {
        crate::edit_ops::util::byte_at_line_col(&self.buffer().rope, line_idx, col_chars)
    }

    /// Convert an absolute byte offset to (line, char_col). char_col is
    /// the count of chars from the line's start, not the byte offset, so
    /// it can drive the column field of a `BlockSelection`.
    pub(crate) fn line_col_at_byte(&self, byte: usize) -> (usize, usize) {
        let rope = &self.buffer().rope;
        let clamped = crate::buffer::floor_char_boundary(rope, byte);
        let line = rope.try_byte_to_line(clamped).unwrap_or(0);
        let Ok(row) = super::column_plan::row(rope, line) else { return (line, 0); };
        let local = clamped.saturating_sub(row.start).min(row.body.len_bytes());
        (line, row.body.try_byte_to_char(local).unwrap_or(0))
    }

    /// True when the active tab has a non-empty block selection.
    pub(crate) fn has_block_selection(&self) -> bool {
        self.tabs.get(self.active)
            .and_then(|tab| tab.block_selection)
            .as_ref()
            .is_some_and(|b| !b.is_empty())
    }

    pub(crate) fn block_selection(&self) -> Option<BlockSelection> {
        self.tabs.get(self.active).and_then(|tab| tab.block_selection)
    }

    pub(crate) fn set_block_selection(&mut self, block: Option<BlockSelection>) {
        self.settle_source_input_edit_caret();
        let Some(tab) = self.tabs.get_mut(self.active) else { return; };
        tab.block_selection = block;
        self.sync_source_input_caret();
    }

    pub(crate) fn clear_block_selection(&mut self) -> bool {
        self.settle_source_input_edit_caret();
        let Some(tab) = self.tabs.get_mut(self.active) else { return false; };
        if tab.block_selection.take().is_some() {
            self.sync_source_input_caret();
            return true;
        }
        false
    }

    pub(crate) fn column_context(&mut self, edit: bool, cx: &mut Context<Self>) -> bool {
        let refusal = if self.window_closed { Some("The editor window is closed") }
            else if let Some(tab) = self.tabs.get(self.active) {
                if tab.view_mode != crate::tab::TabViewMode::Edit { Some("Switch to Source or Raw View to use column operations") }
                else if tab.buffer.loading.is_some() { Some("Wait for the file to finish loading before a column operation") }
                else if edit && tab.buffer.read_only { Some("Buffer is read-only") }
                else { None }
            } else { Some("No active document for the column operation") };
        if let Some(message) = refusal { self.set_status(message); cx.notify(); return false; }
        true
    }

    fn column_history_allocation(&self, rows: usize) -> Result<usize, &'static str> {
        let tab = self.tabs.get(self.active).ok_or("No active column document")?;
        let mut bytes = 0usize;
        if tab.undo_stack.len() == tab.undo_stack.capacity() {
            bytes = tab.undo_stack.len().checked_add(1).and_then(|n| n.checked_mul(std::mem::size_of::<crate::buffer::UndoEntry>()))
                .ok_or(super::column_plan::ALLOCATION_LIMIT)?;
        }
        if let Some(txn) = &tab.edit_transaction {
            if txn.depth == 0 || txn.depth.checked_add(1).is_none() { return Err("The edit group is invalid; finish or cancel it before a column edit"); }
            if txn.ops.len().checked_add(rows).ok_or(super::column_plan::ALLOCATION_LIMIT)? > txn.ops.capacity() {
                bytes = bytes.checked_add(txn.ops.len().checked_mul(std::mem::size_of::<crate::buffer::EditOp>())
                    .ok_or(super::column_plan::ALLOCATION_LIMIT)?).ok_or(super::column_plan::ALLOCATION_LIMIT)?;
            }
        }
        if bytes > super::column_plan::MAX_ALLOCATION { return Err(super::column_plan::ALLOCATION_LIMIT); }
        Ok(bytes)
    }

    fn column_error(&mut self, message: &'static str, cx: &mut Context<Self>) -> bool {
        self.set_status(message); cx.notify(); false
    }

    fn prepare_column_history(&mut self, count: usize) -> Result<Option<Vec<crate::buffer::EditOp>>, &'static str> {
        let tab = self.tabs.get_mut(self.active).ok_or("No active column document")?;
        // Reserve every legacy history slot before the first row changes.
        let mut fresh = None;
        if let Some(txn) = &mut tab.edit_transaction {
            txn.ops.try_reserve_exact(count).map_err(|_| "Not enough memory for column edit history; no changes applied")?;
        } else {
            let mut ops = Vec::new(); ops.try_reserve_exact(count).map_err(|_| "Not enough memory for column edit history; no changes applied")?;
            fresh = Some(ops);
        }
        tab.undo_stack.try_reserve_exact(1).map_err(|_| "Not enough memory for column Undo; no changes applied")?;
        Ok(fresh)
    }

    fn apply_column_plan(&mut self, plan: super::column_plan::Plan, cx: &mut Context<Self>) -> bool {
        if !plan.edits.is_empty() {
            let fresh = match self.prepare_column_history(plan.edits.len()) {
                Ok(fresh) => fresh, Err(message) => return self.column_error(message, cx),
            };
            let old_redo = std::mem::take(&mut self.active_tab_mut().redo_stack);
            cx.background_spawn(async move { drop(old_redo); }).detach();
            if let Some(ops) = fresh {
                let tab = self.active_tab_mut();
                tab.edit_transaction = Some(crate::buffer::EditTransaction { depth: 1, ops,
                    sel_before_anchor: tab.selection.anchor, sel_before_head: tab.selection.head });
            } else { self.begin_edit_group(); }
            for edit in plan.edits { self.replace_byte_range(edit.range, &edit.text, cx); }
            self.end_edit_group(cx);
        }
        if plan.update_geometry {
            self.set_block_selection(plan.block);
            let head = self.byte_at_line_col(plan.head.0, plan.head.1);
            let tab = self.active_tab_mut(); tab.selection.collapse_to(head); tab.desired_col_x = None;
            cx.notify();
        }
        true
    }

    pub(crate) fn block_replace_geometry(&mut self, block: BlockSelection, insert: &str, input_work: usize, cx: &mut Context<Self>) -> bool {
        if !self.column_context(true, cx) { return false; }
        let extra = match super::column_plan::geometry(&self.buffer().rope, &block).and_then(|rows| self.column_history_allocation(rows)) {
            Ok(extra) => extra, Err(message) => return self.column_error(message, cx),
        };
        match super::column_plan::replace(&self.buffer().rope, block, insert, extra, input_work) {
            Ok(plan) => self.apply_column_plan(plan, cx), Err(message) => self.column_error(message, cx),
        }
    }

    pub(crate) fn block_delete(&mut self, backwards: bool, cx: &mut Context<Self>) -> bool {
        let Some(mut block) = self.block_selection() else { return true; };
        if !self.column_context(true, cx) { return false; }
        if block.min_col() == block.max_col() {
            if backwards {
                if block.min_col() > 0 { block.anchor_col = block.min_col() - 1; }
            } else {
                let Some(next) = block.max_col().checked_add(1) else { return self.column_error(super::column_plan::LIMIT, cx); };
                block.head_col = next;
            }
        }
        self.block_replace_geometry(block, "", 0, cx)
    }

    pub(crate) fn block_delete_stream_range(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) -> bool {
        let extra = match self.column_history_allocation(1) {
            Ok(extra) => extra, Err(message) => return self.column_error(message, cx),
        };
        match super::column_plan::delete_stream(&self.buffer().rope, range, extra) {
            Ok(plan) => self.apply_column_plan(plan, cx), Err(message) => self.column_error(message, cx),
        }
    }
    pub(crate) fn block_case(&mut self, block: BlockSelection, transform: &impl Fn(&str) -> String, cx: &mut Context<Self>) -> bool {
        if !self.column_context(true, cx) { return false; }
        let extra = match super::column_plan::geometry(&self.buffer().rope, &block).and_then(|rows| self.column_history_allocation(rows)) {
            Ok(extra) => extra, Err(message) => return self.column_error(message, cx),
        };
        match super::column_plan::case(&self.buffer().rope, block, transform, extra) {
            Ok(plan) => self.apply_column_plan(plan, cx), Err(message) => self.column_error(message, cx),
        }
    }

    pub(crate) fn run_block_paste(&mut self, raw: &str, cx: &mut Context<Self>) -> bool {
        if !self.column_context(true, cx) { return false; }
        let Some(rest) = raw.strip_prefix(BLOCK_CLIPBOARD_SENTINEL) else { return false; };
        let Some(input_work) = raw.len().checked_mul(2).filter(|_| raw.len().checked_mul(5).is_some_and(|n| n <= super::column_plan::MAX_WORK))
            else { return self.column_error(super::column_plan::LIMIT, cx); };
        let extra = match self.column_history_allocation(super::column_plan::MAX_ROWS) {
            Ok(extra) => extra, Err(message) => return self.column_error(message, cx),
        };
        if extra.checked_add(input_work).is_none_or(|n| n > super::column_plan::MAX_ALLOCATION) {
            return self.column_error(super::column_plan::ALLOCATION_LIMIT, cx);
        }
        let clip = crate::text::eol_convert::to_rope_eols(rest);
        match super::column_plan::paste(&self.buffer().rope, self.block_selection(), self.selection(), &clip, extra, input_work) {
            Ok(plan) => self.apply_column_plan(plan, cx), Err(message) => self.column_error(message, cx),
        }
    }

    fn column_clipboard(&mut self, cut: bool, cx: &mut Context<Self>) -> bool {
        let Some(block) = self.block_selection().filter(|block| !block.is_empty()) else { return true; };
        if !self.column_context(cut, cx) { return false; }
        let extra = match super::column_plan::geometry(&self.buffer().rope, &block).and_then(|rows| if cut { self.column_history_allocation(rows) } else { Ok(0) }) {
            Ok(extra) => extra, Err(message) => return self.column_error(message, cx),
        };
        let plan = match super::column_plan::copy(&self.buffer().rope, block, cut, extra) {
            Ok(plan) => plan, Err(message) => return self.column_error(message, cx),
        };
        if let Some(cut) = plan.cut { if !self.apply_column_plan(cut, cx) { return false; } }
        let evicted = crate::clipboard_history::record_copy_owned(plan.history);
        cx.background_spawn(async move { drop(evicted); }).detach();
        cx.write_to_clipboard(ClipboardItem::new_string(plan.payload)); true
    }

    pub(crate) fn block_cut(&mut self, cx: &mut Context<Self>) -> bool { self.column_clipboard(true, cx) }
    pub(crate) fn block_copy(&mut self, cx: &mut Context<Self>) -> bool { self.column_clipboard(false, cx) }

    pub(crate) fn block_extend_up(
        &mut self,
        _: &BlockExtendUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.block_extend(-1, 0, cx);
    }

    pub(crate) fn block_extend_down(
        &mut self,
        _: &BlockExtendDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.block_extend(1, 0, cx);
    }

    pub(crate) fn block_extend_left(
        &mut self,
        _: &BlockExtendLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.block_extend(0, -1, cx);
    }

    pub(crate) fn block_extend_right(
        &mut self,
        _: &BlockExtendRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.block_extend(0, 1, cx);
    }

    /// Extend the block in `delta_line` rows / `delta_col` chars. When no
    /// block is active, seed one anchored at the current caret. Used by
    /// the BlockExtend* keyboard actions.
    pub(crate) fn block_extend(&mut self, delta_line: i32, delta_col: i32, cx: &mut Context<Self>) {
        let total_lines = self.buffer().rope.len_lines().max(1);
        let mut block = self.block_selection().unwrap_or_else(|| {
            let head = self.selection().head;
            let (line, col) = self.line_col_at_byte(head);
            BlockSelection {
                anchor_line: line,
                anchor_col: col,
                head_line: line,
                head_col: col,
                is_dragging: false,
            }
        });
        let (new_line, new_col) =
            step_block_head(block.head_line, block.head_col, delta_line, delta_col, total_lines);
        block.head_line = new_line;
        block.head_col = new_col;
        if block.is_empty() {
            // Movement that ends back at the anchor → clear the block,
            // restore linear caret at that position.
            self.set_block_selection(None);
            let byte = self.byte_at_line_col(new_line, new_col);
            let active = self.active;
            self.tabs[active].selection.anchor = byte;
            self.tabs[active].selection.head = byte;
        } else {
            self.set_block_selection(Some(block));
            let byte = self.byte_at_line_col(new_line, new_col);
            let active = self.active;
            self.tabs[active].selection.anchor = byte;
            self.tabs[active].selection.head = byte;
        }
        self.sync_source_input_caret();
        self.caret_visible = true;
        self.ensure_cursor_visible(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{char_col_in_line, step_block_head};

    #[test]
    fn char_col_counts_chars_not_bytes() {
        let cases: &[(&str, usize, usize)] = &[
            ("", 0, 0),
            ("abc\n", 2, 2),
            ("abc\r\n", 3, 3),
            ("abc\r\n", 4, 3),
            ("abc\r", 9, 3),
            ("héllo\n", 3, 2),
            ("中文", 6, 2),
            ("a👍b", 5, 2),
            ("👨‍👩‍👧\n", 18, 5),
        ];
        for &(line, local, want) in cases {
            assert_eq!(char_col_in_line(line, local), want, "{line:?} @ {local}");
        }
    }

    #[test]
    fn char_col_mid_char_offset_does_not_panic() {
        // A hit-test or stale caret byte that lands inside a multi-byte
        // char used to slice the line there.
        let cases: &[(&str, usize, usize)] = &[
            ("é\n", 1, 0),
            ("中文", 4, 1),
            ("a👍b", 2, 1),
            ("🇫🇷\r\n", 6, 1),
        ];
        for &(line, local, want) in cases {
            assert_eq!(char_col_in_line(line, local), want, "{line:?} @ {local}");
        }
    }

    #[test]
    fn step_block_head_clamps() {
        assert_eq!(step_block_head(0, 0, -1, -1, 5), (0, 0));
        assert_eq!(step_block_head(4, 3, 1, 1, 5), (4, 4));
        assert_eq!(step_block_head(2, 0, 0, 0, 0), (0, 0));
        assert_eq!(step_block_head(1, 7, 1, -1, 5), (2, 6));
    }

    #[test]
    fn step_block_head_large_values_do_not_overflow() {
        let big = i32::MAX as usize;
        assert_eq!(step_block_head(big, big, 1, 1, usize::MAX), (big + 1, big + 1));
        assert_eq!(step_block_head(usize::MAX, usize::MAX, 1, 1, usize::MAX), (usize::MAX - 1, usize::MAX));
    }
}

#[cfg(test)]
#[path = "block_safety_tests.rs"]
pub(super) mod safety_tests;
