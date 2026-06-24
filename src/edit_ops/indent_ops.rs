// Indent ▶ submenu — Increase / Decrease line indent.
//
// Notepad++ source:
//   PowerEditor/src/menuCmdID.h:93 — IDM_EDIT_INS_TAB / IDM_EDIT_RMV_TAB
//
// Increase indent already exists in Notepad Studio via EditInsertTab + the Tab
// keybinding in `app/keymap/edit.rs`. Outdent (Shift+Tab) strips ONE
// leading indent step from every line in the selection (or the caret's
// line when no selection). An indent step is one literal `\t` if present,
// otherwise up to `settings.tab_width` leading spaces.

use gpui::{Context, Window};

use crate::actions::*;
use crate::tab::InputFocus;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn outdent(
        &mut self,
        _: &EditOutdent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.input_focus(window);
        // CSV grid Shift+Tab — symmetric to insert_tab. Moves the active
        // cell one column left with row wrap. Both edit-mode and
        // selected-mode route the same way.
        if focus == InputFocus::CsvCell || focus == InputFocus::CsvGrid {
            crate::ui::csv_doc::actions::move_active_cell_tab(self, false, cx);
            return;
        }
        // Shift+Tab in find/replace inputs cycles to the other input.
        if focus == InputFocus::CsvFindBar {
            crate::ui::csv_doc::actions::focus_replace_input(self, window, cx);
            return;
        }
        if focus == InputFocus::CsvReplaceBar {
            crate::ui::csv_doc::actions::focus_find_input(self, window, cx);
            return;
        }
        if focus != InputFocus::Editor {
            return;
        }
        let sel = self.selection();
        let r = sel.ordered();
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return;
        }
        let first = rope.byte_to_line(r.start);
        let last_raw = rope.byte_to_line(r.end);
        // If the selection ends exactly at a line start (and spans more than
        // one line), the trailing line is not visually selected — exclude it.
        let last = if r.end > r.start
            && last_raw > first
            && rope.line_to_byte(last_raw) == r.end
        {
            last_raw - 1
        } else {
            last_raw
        };
        let block_start = rope.line_to_byte(first);
        let block_end = if last + 1 < total_lines {
            rope.line_to_byte(last + 1)
        } else {
            rope.len_bytes()
        };
        let block_text = rope.byte_slice(block_start..block_end).to_string();
        let tab_width = crate::settings::snapshot().tab_width.max(1);

        let mut new_block = String::with_capacity(block_text.len());
        for line in block_text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\n', '\r']);
            let term = &line[body.len()..];
            let stripped = strip_one_indent(body, tab_width);
            new_block.push_str(stripped);
            new_block.push_str(term);
        }
        if new_block == block_text {
            return;
        }

        let anchor_was = sel.anchor;
        let head_was = sel.head;
        let new_block_len = new_block.len();
        let old_block_len = block_text.len();
        let added = new_block_len as i64 - old_block_len as i64;

        self.replace_byte_range(block_start..block_end, &new_block, cx);

        let adjust = |off: usize| -> usize {
            if off <= block_start {
                off
            } else if off >= block_end {
                ((off as i64) + added).max(block_start as i64) as usize
            } else {
                let rel = off - block_start;
                let new_end = block_start + new_block_len;
                (block_start + rel).min(new_end)
            }
        };
        let new_anchor = adjust(anchor_was);
        let new_head = adjust(head_was);
        let tab = self.active_tab_mut();
        tab.selection.anchor = new_anchor;
        tab.selection.head = new_head;
        tab.desired_col_x = None;
        cx.notify();
    }
}

fn strip_one_indent(body: &str, tab_width: usize) -> &str {
    if let Some(rest) = body.strip_prefix('\t') {
        return rest;
    }
    let mut bytes = 0;
    for (i, c) in body.char_indices() {
        if c == ' ' && i < tab_width {
            bytes = i + 1;
        } else {
            break;
        }
    }
    &body[bytes..]
}
