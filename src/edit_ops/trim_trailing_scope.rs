// PowerEditor/src/NppCommands.cpp:2218–2225; Notepad_plus.cpp:1651–1704.
// Trim trailing space on expanded selected physical lines, or the whole
// document with a collapsed caret. Rectangular selections are unsupported.
use gpui::Context;

use crate::edit_ops::util::{floor_char_boundary, trim_selected_line_range};
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn trim_trailing_ws_scoped(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        if tab.block_selection.is_some() { return; }
        if tab.view_mode != crate::tab::TabViewMode::Edit {
            self.set_status("Switch to Source or Raw View to trim trailing space");
            cx.notify();
            return;
        }
        if tab.buffer.read_only {
            self.set_status("Buffer is read-only");
            cx.notify();
            return;
        }
        let selection = tab.selection.clamped(&tab.buffer.rope);
        let whole_document = selection.is_empty();
        let range = if whole_document {
            if !tab.selection.is_empty() { return; }
            0..tab.buffer.rope.len_bytes()
        } else {
            trim_selected_line_range(&tab.buffer.rope, selection.ordered())
        };
        let Some(slice) = tab.buffer.rope.get_byte_slice(range.clone()) else { return; };
        let out = match trim_trailing(slice) {
            Ok(out) => out,
            Err(_) => {
                self.set_status("Trim Trailing Space skipped: not enough memory");
                cx.notify();
                return;
            }
        };
        if slice.chars().eq(out.chars()) { return; }
        let head_was = self.selection().head;
        self.replace_byte_range(range.clone(), &out, cx);
        let head = floor_char_boundary(&self.buffer().rope, head_was);
        let tab = self.active_tab_mut();
        if whole_document {
            tab.selection.collapse_to(head);
        } else {
            tab.selection.anchor = range.start;
            tab.selection.head = range.start.saturating_add(out.len());
        }
        tab.desired_col_x = None;
        self.commit_caret(cx);
    }
}

fn trim_trailing(slice: ropey::RopeSlice<'_>) -> Result<String, std::collections::TryReserveError> {
    let mut out = String::new();
    out.try_reserve(slice.len_bytes())?;
    let mut trailing_start = 0;
    for ch in slice.chars() {
        if matches!(ch, '\r' | '\n') {
            out.truncate(trailing_start);
            out.push(ch);
            trailing_start = out.len();
        } else {
            out.push(ch);
            if !matches!(ch, ' ' | '\t') { trailing_start = out.len(); }
        }
    }
    out.truncate(trailing_start);
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn trim_only_ascii_tail_space_and_keep_each_physical_terminator() {
        for (source, expected) in [
            ("", ""),
            (" \t", ""),
            (" café  \r\n中\t\n  b \r", " café\r\n中\n  b\r"),
            ("\u{FEFF}a\t \r\n\t\r\nlast  ", "\u{FEFF}a\r\n\r\nlast"),
            ("a \u{85} b \u{2028} c \u{2029} d\u{a0}\t ", "a \u{85} b \u{2028} c \u{2029} d\u{a0}"),
        ] {
            let rope = ropey::Rope::from_str(source);
            assert_eq!(super::trim_trailing(rope.slice(..)).unwrap(), expected);
        }
    }
}
