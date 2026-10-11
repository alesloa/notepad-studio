// Notepad++ column-mode (block / rectangular) selection — model-side
// helpers and edit funnels. Paint lives in `editor_element.rs`; mouse
// alt-drag wiring lives in `mouse.rs` and the per-frame mouse-move
// handler in `editor_element.rs`. This module is the place every
// block-aware code path bottoms out — typing, backspace, delete, cut,
// copy, paste, and the Alt+Shift arrow extensions.

use gpui::{ClipboardItem, Context, Window};

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
        let rope = &self.buffer().rope;
        if line_idx >= rope.len_lines() {
            return 0;
        }
        let line = rope.line(line_idx);
        let raw = line.to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        trimmed.chars().count()
    }

    /// Byte offset into the active buffer for the char-column `col` on
    /// `line`. Clamps `col` to the line's char count — virtual columns
    /// past EOL all map to the line's printable end byte (caller pads
    /// with spaces when materializing virtual-column space). Lines past
    /// EOF map to the rope's end byte.
    pub(crate) fn byte_at_line_col(&self, line_idx: usize, col_chars: usize) -> usize {
        let rope = &self.buffer().rope;
        if line_idx >= rope.len_lines() {
            return rope.len_bytes();
        }
        let line_start = rope.line_to_byte(line_idx);
        let line_slice = rope.line(line_idx);
        let raw = line_slice.to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        let max_chars = trimmed.chars().count();
        let target = col_chars.min(max_chars);
        let mut byte_in_line = 0usize;
        for (i, c) in trimmed.chars().enumerate() {
            if i == target {
                break;
            }
            byte_in_line += c.len_utf8();
        }
        line_start + byte_in_line
    }

    /// Convert an absolute byte offset to (line, char_col). char_col is
    /// the count of chars from the line's start, not the byte offset, so
    /// it can drive the column field of a `BlockSelection`.
    pub(crate) fn line_col_at_byte(&self, byte: usize) -> (usize, usize) {
        let rope = &self.buffer().rope;
        let total = rope.len_bytes();
        let clamped = byte.min(total);
        let line = rope.byte_to_line(clamped);
        let line_start = rope.line_to_byte(line);
        let local = clamped.saturating_sub(line_start);
        let raw = rope.line(line).to_string();
        (line, char_col_in_line(&raw, local))
    }

    /// True when the active tab has a non-empty block selection.
    pub(crate) fn has_block_selection(&self) -> bool {
        self.tabs[self.active]
            .block_selection
            .as_ref()
            .is_some_and(|b| !b.is_empty())
    }

    pub(crate) fn block_selection(&self) -> Option<BlockSelection> {
        self.tabs[self.active].block_selection
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

    /// Apply a column-mode edit: delete the block's rectangle and (when
    /// `insert` is non-empty) insert `insert` at min_col on every spanned
    /// line. Walks bottom-to-top so byte offsets on earlier lines stay
    /// stable across each per-line replace. Wraps the whole batch in one
    /// `EditTransaction` so undo reverts the entire column edit in one
    /// step. Updates the block to a thin column at the post-insert
    /// caret.
    pub(crate) fn block_replace(&mut self, insert: &str, cx: &mut Context<Self>) {
        let Some(block) = self.block_selection() else {
            return;
        };
        let min_line = block.min_line();
        let max_line = block.max_line();
        let min_col = block.min_col();
        let max_col = block.max_col();

        // Snapshot the original head/anchor cols so we can rebuild the
        // post-edit thin block at the correct end-corner.
        let post_col = min_col.saturating_add(insert.chars().count());

        self.begin_edit_group();

        // Walk bottom-to-top so per-line edits don't shift the byte
        // offsets we've already computed for higher (lower-index) lines.
        let total_lines = self.buffer().rope.len_lines();
        let last = max_line.min(total_lines.saturating_sub(1));
        for line_idx in (min_line..=last).rev() {
            let line_chars = self.line_char_count(line_idx);
            let del_start_byte = self.byte_at_line_col(line_idx, min_col);
            let del_end_byte = self.byte_at_line_col(line_idx, max_col);

            // Pad with virtual spaces when the line is shorter than
            // `min_col` so the inserted text lands at the user's chosen
            // column rather than colliding with the EOL.
            let pad = if !insert.is_empty() && line_chars < min_col {
                " ".repeat(min_col - line_chars)
            } else {
                String::new()
            };

            let mut payload = pad;
            payload.push_str(insert);

            if del_start_byte == del_end_byte && payload.is_empty() {
                continue;
            }
            self.replace_byte_range(del_start_byte..del_end_byte, &payload, cx);
        }

        self.end_edit_group(cx);

        // Collapse the block to a thin column at the post-edit position.
        // For pure-delete (insert.is_empty()), the block collapses to
        // a thin caret at min_col on every spanned line. For typing,
        // it advances by the inserted char count.
        let new_block = BlockSelection {
            anchor_line: min_line,
            anchor_col: post_col,
            head_line: max_line,
            head_col: post_col,
            is_dragging: false,
        };
        if new_block.is_empty() {
            // Fully collapsed → drop the block entirely. The linear
            // selection (set by the last `replace_byte_range`) lands at
            // min_line/min_col on the last edited line, which is the
            // expected post-delete caret.
            self.set_block_selection(None);
        } else {
            self.set_block_selection(Some(new_block));
            // Sync the linear caret to the head corner so the rest of
            // the codebase (status bar, ensure_cursor_visible, etc.)
            // sees a sensible caret position.
            let head_byte = self.byte_at_line_col(new_block.head_line, new_block.head_col);
            let active = self.active;
            self.tabs[active].selection.anchor = head_byte;
            self.tabs[active].selection.head = head_byte;
        }
    }

    /// Serialize a block selection to a multi-line string with `\n` row
    /// separators. Each row is the slice [min_col..max_col] of its
    /// logical line, padded with spaces if the line is shorter than
    /// max_col. Used by Cut and Copy.
    pub(crate) fn block_serialize(&self) -> Option<String> {
        let block = self.block_selection()?;
        if block.is_empty() {
            return None;
        }
        let min_line = block.min_line();
        let max_line = block.max_line();
        let min_col = block.min_col();
        let max_col = block.max_col();
        let total_lines = self.buffer().rope.len_lines();
        let last = max_line.min(total_lines.saturating_sub(1));

        let mut out = String::new();
        for line_idx in min_line..=last {
            let s_byte = self.byte_at_line_col(line_idx, min_col);
            let e_byte = self.byte_at_line_col(line_idx, max_col);
            let slice = self.buffer().slice_bytes(s_byte..e_byte);
            let line_chars = self.line_char_count(line_idx);
            // If the line stops short of max_col, pad the row with
            // spaces so the clipboard preserves the rectangular shape.
            if line_chars < max_col {
                let pad_count = (max_col - line_chars.max(min_col)).min(max_col - min_col);
                let mut padded = slice;
                padded.push_str(&" ".repeat(pad_count));
                out.push_str(&padded);
            } else {
                out.push_str(&slice);
            }
            if line_idx != last {
                out.push('\n');
            }
        }
        Some(out)
    }

    /// Apply a column paste. `clip_lines` is the row-split clipboard
    /// payload (sentinel already stripped). Pastes `clip_lines[i]` at
    /// the caret/block's column on consecutive lines, padding earlier
    /// lines with spaces when they don't reach the target column.
    /// Walks bottom-to-top so prior-line byte offsets stay stable.
    pub(crate) fn block_paste(&mut self, clip_lines: &[&str], cx: &mut Context<Self>) {
        if clip_lines.is_empty() {
            return;
        }

        // Determine target rectangle: if a block is active, replace
        // its content row-by-row; else the linear caret defines the
        // top-left corner.
        let (start_line, paste_col, replace_block) = if let Some(block) = self.block_selection() {
            (block.min_line(), block.min_col(), Some(block))
        } else {
            let head = self.selection().head;
            let (line, col) = self.line_col_at_byte(head);
            (line, col, None)
        };

        let total_lines = self.buffer().rope.len_lines();
        // Cap at last existing line — extending past EOF is deferred.
        let lines_avail = total_lines.saturating_sub(start_line);
        let pasted_count = clip_lines.len().min(lines_avail);
        if pasted_count == 0 {
            return;
        }

        self.begin_edit_group();

        // If we're replacing an existing block, first delete the block's
        // rectangle on rows beyond the clipboard length so the row-by-row
        // map cleanly. Walk bottom-to-top.
        if let Some(block) = replace_block {
            let max_line = block.max_line().min(total_lines.saturating_sub(1));
            let min_col = block.min_col();
            let max_col = block.max_col();
            let block_height = max_line - block.min_line() + 1;
            let extra_rows = block_height.saturating_sub(pasted_count);
            for line_idx in
                ((start_line + pasted_count)..(start_line + pasted_count + extra_rows)).rev()
            {
                if line_idx > max_line {
                    continue;
                }
                let s = self.byte_at_line_col(line_idx, min_col);
                let e = self.byte_at_line_col(line_idx, max_col);
                if s != e {
                    self.replace_byte_range(s..e, "", cx);
                }
            }
        }

        // Paste each clipboard row onto its target row, bottom-to-top.
        for i in (0..pasted_count).rev() {
            let line_idx = start_line + i;
            let line_chars = self.line_char_count(line_idx);

            // For block replace, the existing row's [min_col..max_col]
            // gets replaced. For linear paste, just insert at paste_col.
            let (del_s, del_e) = if let Some(block) = replace_block {
                let max_col = block.max_col();
                (
                    self.byte_at_line_col(line_idx, paste_col),
                    self.byte_at_line_col(line_idx, max_col),
                )
            } else {
                let s = self.byte_at_line_col(line_idx, paste_col);
                (s, s)
            };

            let pad = if line_chars < paste_col {
                " ".repeat(paste_col - line_chars)
            } else {
                String::new()
            };
            let mut payload = pad;
            payload.push_str(clip_lines[i]);
            self.replace_byte_range(del_s..del_e, &payload, cx);
        }

        self.end_edit_group(cx);

        // Place the block selection around the freshly pasted region so
        // the user can keep editing rectangularly.
        let widest = clip_lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let end_col = paste_col + widest;
        let end_line = start_line + pasted_count - 1;
        let new_block = BlockSelection {
            anchor_line: start_line,
            anchor_col: paste_col,
            head_line: end_line,
            head_col: end_col,
            is_dragging: false,
        };
        if new_block.is_empty() {
            self.set_block_selection(None);
        } else {
            self.set_block_selection(Some(new_block));
            let head_byte = self.byte_at_line_col(end_line, end_col);
            let active = self.active;
            self.tabs[active].selection.anchor = head_byte;
            self.tabs[active].selection.head = head_byte;
        }
    }

    /// Notepad++-style block Cut: write the block to the clipboard with
    /// the sentinel marker and delete the rectangle in one undo step.
    pub(crate) fn block_cut(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.block_serialize() else {
            return;
        };
        // History stores the unwrapped text so a paste-from-history lands
        // as plain content, not as a block-paste with the sentinel header.
        crate::clipboard_history::record_copy(&text);
        let mut payload = String::from(BLOCK_CLIPBOARD_SENTINEL);
        payload.push_str(&text);
        cx.write_to_clipboard(ClipboardItem::new_string(payload));
        self.block_replace("", cx);
    }

    /// Notepad++-style block Copy.
    pub(crate) fn block_copy(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.block_serialize() else {
            return;
        };
        crate::clipboard_history::record_copy(&text);
        let mut payload = String::from(BLOCK_CLIPBOARD_SENTINEL);
        payload.push_str(&text);
        cx.write_to_clipboard(ClipboardItem::new_string(payload));
    }

    /// Try a block-aware paste. Returns `true` when the clipboard had
    /// the block sentinel and the paste was applied as column paste.
    /// Returns `false` for plain text (caller falls through to the
    /// existing linear paste).
    pub(crate) fn try_block_paste(&mut self, raw: &str, cx: &mut Context<Self>) -> bool {
        let Some(rest) = raw.strip_prefix(BLOCK_CLIPBOARD_SENTINEL) else {
            return false;
        };
        let lines: Vec<&str> = rest.split('\n').collect();
        self.block_paste(&lines, cx);
        true
    }

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
