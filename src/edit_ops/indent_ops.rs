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

use std::ops::Range;

use gpui::{Context, Window};
use ropey::Rope;

use crate::actions::*;
use crate::edit_ops::util::{floor_char_boundary, split_lines_eol};
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
        let tab_width = crate::settings::snapshot().tab_width.max(1);
        let rope = &self.buffer().rope;
        let Some(out) = outdent_block(rope, sel.ordered(), tab_width) else {
            return;
        };
        // Clamp first: a stale offset must not map past the new text.
        let anchor_was = floor_char_boundary(rope, sel.anchor);
        let head_was = floor_char_boundary(rope, sel.head);

        self.replace_byte_range(out.range.clone(), &out.text, cx);

        let new_anchor = out.map_offset(anchor_was);
        let new_head = out.map_offset(head_was);
        let tab = self.active_tab_mut();
        tab.selection.anchor = new_anchor;
        tab.selection.head = new_head;
        tab.desired_col_x = None;
        cx.notify();
    }
}

/// The result of outdenting a block of lines: the byte range replaced, its
/// new text, and where each line's removed indent sat so old offsets can be
/// mapped into the new text.
#[derive(Debug, PartialEq, Eq)]
struct Outdent {
    range: Range<usize>,
    text: String,
    /// `(old line start, bytes removed there)`, in line order.
    cuts: Vec<(usize, usize)>,
}

impl Outdent {
    /// Map an offset in the old text to the new one: a removed byte before it
    /// shifts it left, and an offset inside a removed indent lands where the
    /// indent was.
    fn map_offset(&self, off: usize) -> usize {
        let mut shift = 0usize;
        for &(start, removed) in &self.cuts {
            if start >= off {
                break;
            }
            shift += (off - start).min(removed);
        }
        off.saturating_sub(shift)
    }
}

/// Strip one indent step from every line `sel` touches. A selection ending
/// at column 0 of a line leaves that line out. The selection is clamped to
/// the rope, so a stale one can't panic. `None` when nothing changes.
fn outdent_block(rope: &Rope, sel: Range<usize>, tab_width: usize) -> Option<Outdent> {
    let total_lines = rope.len_lines();
    if total_lines == 0 {
        return None;
    }
    let len = rope.len_bytes();
    let (start, end) = (sel.start.min(sel.end).min(len), sel.start.max(sel.end).min(len));
    let first = rope.byte_to_line(start);
    let last_raw = rope.byte_to_line(end);
    // If the selection ends exactly at a line start (and spans more than
    // one line), the trailing line is not visually selected — exclude it.
    let last = if end > start && last_raw > first && rope.line_to_byte(last_raw) == end {
        last_raw - 1
    } else {
        last_raw
    };
    let block_start = rope.line_to_byte(first);
    let block_end = if last + 1 < total_lines {
        rope.line_to_byte(last + 1)
    } else {
        len
    };
    let block_text = rope.get_byte_slice(block_start..block_end)?.to_string();

    let mut text = String::with_capacity(block_text.len());
    let mut cuts = Vec::new();
    let mut line_start = block_start;
    for (body, term) in split_lines_eol(&block_text) {
        let stripped = strip_one_indent(body, tab_width);
        let removed = body.len() - stripped.len();
        if removed > 0 {
            cuts.push((line_start, removed));
        }
        text.push_str(stripped);
        text.push_str(term);
        line_start += body.len() + term.len();
    }
    if cuts.is_empty() {
        return None;
    }
    Some(Outdent {
        range: block_start..block_end,
        text,
        cuts,
    })
}

fn strip_one_indent(body: &str, tab_width: usize) -> &str {
    if let Some(rest) = body.strip_prefix('\t') {
        return rest;
    }
    // Leading spaces, at most one indent step: ASCII, so the slice below is
    // on a char boundary.
    let spaces = body.bytes().take(tab_width).take_while(|&b| b == b' ').count();
    body.get(spaces..).unwrap_or(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outdent(text: &str, sel: Range<usize>, tab_width: usize) -> Option<String> {
        let rope = Rope::from_str(text);
        let out = outdent_block(&rope, sel, tab_width)?;
        let mut s = text.to_string();
        s.replace_range(out.range.clone(), &out.text);
        Some(s)
    }

    #[test]
    fn outdent_strips_one_step_per_line() {
        assert_eq!(outdent("\t\ta\n    b\n  c", 0..13, 4).as_deref(), Some("\ta\nb\nc"));
        assert_eq!(outdent("      a", 0..0, 4).as_deref(), Some("  a"));
        assert_eq!(outdent("a\n  b", 0..1, 4), None);
        // Selection ending at column 0 of line 1 leaves line 1 alone.
        assert_eq!(outdent("  a\n  b", 0..4, 4).as_deref(), Some("a\n  b"));
    }

    #[test]
    fn outdent_keeps_every_line_end_kind() {
        // CR-only lines used to be one "line" to `split_inclusive('\n')`.
        assert_eq!(outdent("  a\r  b", 0..7, 4).as_deref(), Some("a\rb"));
        assert_eq!(outdent("\ta\r\n\tb\n\tc\r", 0..12, 4).as_deref(), Some("a\r\nb\nc\r"));
        assert_eq!(outdent("  é\n  👨‍👩‍👧\n  🇯🇵", 0..40, 4).as_deref(), Some("é\n👨‍👩‍👧\n🇯🇵"));
        assert_eq!(outdent("   \n", 0..4, 4).as_deref(), Some("\n"));
    }

    #[test]
    fn outdent_clamps_stale_selection() {
        // A selection left over from a longer buffer used to panic
        // `byte_to_line`.
        assert_eq!(outdent("  ab\n  cd", 10..50, 4).as_deref(), Some("  ab\ncd"));
        assert_eq!(outdent("", 3..9, 4), None);
    }

    #[test]
    fn outdent_survives_every_selection_and_line_end() {
        // ropey also breaks lines on VT, FF, NEL, LS and PS, which
        // `split_lines_eol` keeps inside a line.
        let inputs = [
            "",
            "  a",
            "\ta\r\n  b\r  c\n",
            "  é\n\t👨‍👩‍👧\r\n  🇯🇵",
            "  a\u{2028}  b\n\tc\u{85}\td\u{b} e\u{c}  f\u{2029}\tg\n",
            "\u{2028}\r\n\u{85}  ",
        ];
        let eols = |s: &str| -> Vec<String> {
            split_lines_eol(s).into_iter().map(|p| p.1.to_string()).collect()
        };
        for text in inputs {
            let len = text.len();
            for a in 0..=len + 2 {
                for b in [0, a, len, len + 2] {
                    let ctx = format!("{text:?} sel {a}..{b}");
                    let Some(out) = outdent(text, a..b, 4) else { continue };
                    assert_eq!(eols(&out), eols(text), "{ctx}");
                    let strip = |s: &str| s.replace([' ', '\t'], "");
                    assert_eq!(strip(&out), strip(text), "{ctx}");
                    assert_eq!(Rope::from_str(&out).len_lines(), Rope::from_str(text).len_lines(), "{ctx}");
                }
            }
        }
    }

    #[test]
    fn outdent_maps_offsets_onto_char_boundaries() {
        // Caret after the first `中` of "  中中" (byte 5). The old mapping kept
        // the in-block offset, landing inside the second `中` (bytes 3..6 of
        // "中中") once the indent was gone.
        let text = "  中中\n  x";
        let rope = Rope::from_str(text);
        let out = outdent_block(&rope, 0..text.len(), 4).expect("changed");
        assert_eq!(out.text, "中中\nx");
        assert_eq!(out.map_offset(5), 3);
        assert_eq!(out.map_offset(0), 0);
        // Inside the removed indent: lands at the line start.
        assert_eq!(out.map_offset(1), 0);
        assert_eq!(out.map_offset(9), 7);
        assert_eq!(out.map_offset(10), 7);
        assert_eq!(out.map_offset(text.len()), out.text.len());
        for off in 0..=text.len() {
            if text.is_char_boundary(off) {
                assert!(out.text.is_char_boundary(out.map_offset(off)), "offset {off}");
            }
        }
    }
}
