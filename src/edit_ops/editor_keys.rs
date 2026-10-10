// Scintilla editor keys that Notepad++ binds without a menu item.
//
// PowerEditor/src/Parameters.cpp scintKeyDefs[]:
//   :564 — SCI_EDITTOGGLEOVERTYPE  Insert
//   :569 — SCI_LINESCROLLDOWN      Ctrl+Down
//   :573 — SCI_LINESCROLLUP        Ctrl+Up
//   :634 — SCI_DELWORDLEFT         Ctrl+Backspace
//   :635 — SCI_DELWORDRIGHT        Ctrl+Delete
//   :636 — SCI_DELLINELEFT         Ctrl+Shift+Backspace
//   :637 — SCI_DELLINERIGHT        Ctrl+Shift+Delete
//
// Behaviour follows Scintilla, which Notepad++ embeds:
//   scintilla/src/Editor.cxx:3870-3937 (DelWordOrLine): the deleted range
//     runs from the caret, not the selection; the anchor stays put.
//   scintilla/src/Document.cxx:2126 (NextWordStart): leftwards skips
//     blanks then one run of a single character class; rightwards skips
//     one run of a class then blanks. Line breaks are their own class, so
//     Ctrl+Backspace at a line start joins the line to the one above.
//   scintilla/src/Editor.cxx:3956/3975 (LineScrollDown/Up): scroll one
//     whole line, then pull the caret back inside the view.
//   scintilla/src/Editor.cxx:4067 (EditToggleOvertype) and InsertCharacter:
//     in overtype a typed character replaces the next one unless the caret
//     sits at the end of its line.
//
// All seven are macro-recordable, as in Scintilla's NotifyMacroRecord list
// (Editor.cxx:3020-3101).

use std::ops::Range;

use gpui::{Context, Window};
use ropey::Rope;

use crate::actions::{
    EditDeleteLineLeft, EditDeleteLineRight, EditDeleteWordLeft, EditDeleteWordRight,
    EditScrollLineDown, EditScrollLineUp, EditToggleOvertype,
};
use crate::buffer::BlockSelection;
use crate::macros::{self, MacroEvent};
use crate::tab::InputFocus;
use crate::workspace::Workspace;

/// The part of the line around the caret a delete key removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaretPart {
    WordLeft,
    WordRight,
    LineLeft,
    LineRight,
}

impl CaretPart {
    fn macro_event(self) -> MacroEvent {
        match self {
            Self::WordLeft => MacroEvent::DeleteWordLeft,
            Self::WordRight => MacroEvent::DeleteWordRight,
            Self::LineLeft => MacroEvent::DeleteLineLeft,
            Self::LineRight => MacroEvent::DeleteLineRight,
        }
    }
}

/// Scintilla's default character classes (CharClassify.cxx:22 for ASCII,
/// Document.cxx:2025 for the rest of Unicode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Space,
    NewLine,
    Word,
    Punctuation,
}

fn char_class(c: char) -> CharClass {
    match c {
        '\r' | '\n' | '\u{2028}' | '\u{2029}' => CharClass::NewLine,
        _ if c.is_whitespace() || c.is_control() => CharClass::Space,
        _ if c.is_alphanumeric() || c == '_' => CharClass::Word,
        // Scintilla also counts combining marks as word characters.
        _ if !c.is_ascii() && is_combining_mark(c) => CharClass::Word,
        _ => CharClass::Punctuation,
    }
}

/// The main combining-mark blocks (Unicode Mn/Mc/Me). Rust's std has no
/// general-category lookup; these blocks cover accents typed or pasted in
/// practice.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Bytes Ctrl+Backspace removes. `before` yields the characters left of
/// the caret, nearest first.
pub(crate) fn word_start_before(before: impl Iterator<Item = char>) -> usize {
    let mut chars = before.peekable();
    let mut len = 0usize;
    while let Some(&c) = chars.peek() {
        if char_class(c) != CharClass::Space {
            break;
        }
        len += c.len_utf8();
        chars.next();
    }
    let Some(&first) = chars.peek() else {
        return len;
    };
    let class = char_class(first);
    for c in chars {
        if char_class(c) != class {
            break;
        }
        len += c.len_utf8();
    }
    len
}

/// Bytes Ctrl+Delete removes. `after` yields the characters right of the
/// caret, nearest first.
pub(crate) fn word_start_after(after: impl Iterator<Item = char>) -> usize {
    let mut chars = after.peekable();
    let mut len = 0usize;
    let Some(&first) = chars.peek() else {
        return 0;
    };
    let class = char_class(first);
    while let Some(&c) = chars.peek() {
        if char_class(c) != class {
            break;
        }
        len += c.len_utf8();
        chars.next();
    }
    for c in chars {
        if char_class(c) != CharClass::Space {
            break;
        }
        len += c.len_utf8();
    }
    len
}

/// Bytes that `typed_chars` overtyped characters replace: the next
/// characters after the caret, stopping at the end of the line.
pub(crate) fn overtype_len(after: impl Iterator<Item = char>, typed_chars: usize) -> usize {
    after
        .take_while(|c| *c != '\n' && *c != '\r')
        .take(typed_chars)
        .map(char::len_utf8)
        .sum()
}

/// Where the anchor lands once `deleted` is cut out of the text.
pub(crate) fn anchor_after_delete(anchor: usize, deleted: &Range<usize>) -> usize {
    if anchor <= deleted.start {
        anchor
    } else if anchor >= deleted.end {
        anchor - (deleted.end - deleted.start)
    } else {
        deleted.start
    }
}

/// The range a delete key removes from a single-line input field.
pub(crate) fn input_part_range(text: &str, cursor: usize, part: CaretPart) -> Range<usize> {
    let cursor = floor_char_boundary(text, cursor);
    let (left, right) = text.split_at(cursor);
    match part {
        CaretPart::WordLeft => cursor - word_start_before(left.chars().rev())..cursor,
        CaretPart::WordRight => cursor..cursor + word_start_after(right.chars()),
        CaretPart::LineLeft => 0..cursor,
        CaretPart::LineRight => cursor..text.len(),
    }
}

fn floor_char_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// `byte` clamped to the rope and moved back onto a character start.
fn rope_char_boundary(rope: &Rope, byte: usize) -> usize {
    let byte = byte.min(rope.len_bytes());
    rope.char_to_byte(rope.byte_to_char(byte))
}

/// Scintilla `LineScrollUp/Down`: the new scroll offset once the top row
/// moves by `delta` whole rows. A part-scrolled top row counts as the row
/// it shows, so the result always lands on a row boundary.
pub(crate) fn line_scroll_top(scroll_top: f32, line_height: f32, delta: i64) -> f32 {
    if line_height <= 0.0 {
        return scroll_top;
    }
    let top_row = (scroll_top / line_height + 1e-3).floor() as i64;
    (top_row + delta).max(0) as f32 * line_height
}

/// Scintilla `MoveCaretInsideView`: rows the caret has to move so it sits
/// on a fully visible row after a scroll. Positive moves down.
pub(crate) fn caret_rows_into_view(
    scroll_top: f32,
    line_height: f32,
    view_height: f32,
    caret_row: usize,
) -> i64 {
    if line_height <= 0.0 || view_height <= 0.0 {
        return 0;
    }
    let first = (scroll_top / line_height - 1e-3).ceil().max(0.0) as i64;
    let last = (((scroll_top + view_height) / line_height + 1e-3).floor() as i64 - 1).max(first);
    let row = caret_row as i64;
    if row < first {
        first - row
    } else if row > last {
        last - row
    } else {
        0
    }
}

impl Workspace {
    pub(crate) fn delete_word_left(
        &mut self,
        _: &EditDeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_caret_part(CaretPart::WordLeft, window, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &EditDeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_caret_part(CaretPart::WordRight, window, cx);
    }

    pub(crate) fn delete_line_left(
        &mut self,
        _: &EditDeleteLineLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_caret_part(CaretPart::LineLeft, window, cx);
    }

    pub(crate) fn delete_line_right(
        &mut self,
        _: &EditDeleteLineRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_caret_part(CaretPart::LineRight, window, cx);
    }

    fn delete_caret_part(&mut self, part: CaretPart, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.input_focus(window);
        if focus != InputFocus::Editor {
            self.input_delete_part(focus, part);
            cx.notify();
            return;
        }
        macros::record(part.macro_event());
        if self.has_block_selection() {
            match part {
                CaretPart::LineLeft | CaretPart::LineRight => {
                    self.block_delete_line_part(part, cx);
                    return;
                }
                // A column selection would end ragged after a per-line
                // word delete, which `BlockSelection` can't hold. Drop it
                // and delete at the caret, which sits at the head corner.
                CaretPart::WordLeft | CaretPart::WordRight => {
                    self.clear_block_selection();
                }
            }
        }
        let rope = &self.buffer().rope;
        let len_before = rope.len_bytes();
        let sel = self.selection();
        let head = rope_char_boundary(rope, sel.head);
        let range = match part {
            CaretPart::WordLeft => {
                let removed = word_start_before(rope.chars_at(rope.byte_to_char(head)).reversed());
                head - removed..head
            }
            CaretPart::WordRight => {
                head..head + word_start_after(rope.chars_at(rope.byte_to_char(head)))
            }
            CaretPart::LineLeft => self.line_start_byte(head)..head,
            CaretPart::LineRight => head..self.line_end_byte(head).max(head),
        };
        if range.is_empty() {
            return;
        }
        let anchor = sel.anchor.min(len_before);
        self.replace_byte_range(range.clone(), "", cx);
        // Read-only buffers refuse the edit; leave the selection alone then.
        if self.buffer().rope.len_bytes() == len_before {
            return;
        }
        let new_anchor = rope_char_boundary(&self.buffer().rope, anchor_after_delete(anchor, &range));
        let tab = self.active_tab_mut();
        tab.selection.anchor = new_anchor;
        tab.selection.head = range.start;
        cx.notify();
    }

    /// Ctrl+Shift+Backspace / Delete on a column selection: delete from
    /// the caret column to the start or end of every spanned line.
    fn block_delete_line_part(&mut self, part: CaretPart, cx: &mut Context<Self>) {
        let Some(block) = self.block_selection() else {
            return;
        };
        let (min_line, max_line) = (block.min_line(), block.max_line());
        let (from_col, to_col) = if part == CaretPart::LineLeft {
            (0, block.head_col)
        } else {
            let widest = (min_line..=max_line)
                .map(|line| self.line_char_count(line))
                .max()
                .unwrap_or(0);
            (block.head_col, widest.max(block.head_col))
        };
        self.set_block_selection(Some(BlockSelection {
            anchor_line: min_line,
            anchor_col: from_col,
            head_line: max_line,
            head_col: to_col,
            is_dragging: false,
        }));
        self.block_replace("", cx);
    }

    fn input_delete_part(&mut self, focus: InputFocus, part: CaretPart) {
        let Some((text, cursor, anchor)) = self.input_field_mut(focus) else {
            return;
        };
        let range = input_part_range(text, *cursor, part);
        if range.is_empty() {
            return;
        }
        let new_anchor = anchor_after_delete(*anchor, &range);
        text.replace_range(range.clone(), "");
        *cursor = range.start;
        *anchor = floor_char_boundary(text, new_anchor);
        if focus == InputFocus::FindQuery {
            self.refresh_matches();
        }
    }

    pub(crate) fn scroll_line_up(
        &mut self,
        _: &EditScrollLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_by_line(-1, window, cx);
    }

    pub(crate) fn scroll_line_down(
        &mut self,
        _: &EditScrollLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_by_line(1, window, cx);
    }

    fn scroll_by_line(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_focus(window) != InputFocus::Editor || self.is_in_markdown_preview() {
            return;
        }
        macros::record(if delta < 0 {
            MacroEvent::ScrollLineUp
        } else {
            MacroEvent::ScrollLineDown
        });
        let line_height = f32::from(self.viewport.line_height);
        let view_height = f32::from(self.viewport.bounds.size.height);
        if line_height <= 0.0 || view_height <= 0.0 {
            return;
        }
        let target = line_scroll_top(f32::from(self.scroll_top()), line_height, delta);
        self.set_scroll_top(gpui::px(target), cx);
        // `set_scroll_top` clamps, so measure the caret against where the
        // view actually landed.
        let top = f32::from(self.scroll_top());
        let head = self.selection().head.min(self.buffer().rope.len_bytes());
        let caret_row = if self.word_wrap {
            self.visual_row_for_byte(head)
                .unwrap_or_else(|| self.buffer().rope.byte_to_line(head))
        } else {
            self.buffer().rope.byte_to_line(head)
        };
        let rows = caret_rows_into_view(top, line_height, view_height, caret_row);
        if rows != 0 {
            let rows = i32::try_from(rows).unwrap_or(if rows < 0 { i32::MIN } else { i32::MAX });
            self.move_vertical(rows, false, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_overtype(
        &mut self,
        _: &EditToggleOvertype,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        macros::record(MacroEvent::ToggleOvertype);
        self.set_overtype(!self.overtype, cx);
    }

    /// Notepad++ also flips the mode from a click on the status bar's
    /// INS / OVR field (NppNotification.cpp:941), which Scintilla does not
    /// record in a macro.
    pub(crate) fn set_overtype(&mut self, on: bool, cx: &mut Context<Self>) {
        self.overtype = on;
        self.caret_visible = true;
        cx.notify();
    }

    /// End of the range that `typed` replaces when inserted at `caret`.
    /// Equal to `caret` unless overtype is on; a typed line break never
    /// overwrites.
    pub(crate) fn overtype_end(&self, caret: usize, typed: &str) -> usize {
        if !self.overtype || typed.contains(['\n', '\r']) {
            return caret;
        }
        let rope = &self.buffer().rope;
        let caret = rope_char_boundary(rope, caret);
        let after = rope.chars_at(rope.byte_to_char(caret));
        caret + overtype_len(after, typed.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn left(text: &str, caret: usize) -> String {
        let r = input_part_range(text, caret, CaretPart::WordLeft);
        format!("{}|{}", &text[..r.start], &text[r.end..])
    }

    fn right(text: &str, caret: usize) -> String {
        let r = input_part_range(text, caret, CaretPart::WordRight);
        format!("{}|{}", &text[..r.start], &text[r.end..])
    }

    #[test]
    fn delete_word_left_follows_scintilla_classes() {
        assert_eq!(left("foo bar", 7), "foo |");
        assert_eq!(left("foo bar  ", 9), "foo |");
        assert_eq!(left("foo.bar", 7), "foo.|");
        assert_eq!(left("foo...", 6), "foo|");
        assert_eq!(left("snake_case", 10), "|");
        assert_eq!(left("", 0), "|");
        assert_eq!(left("   ", 3), "|");
        assert_eq!(left("foo bar", 0), "|foo bar");
    }

    #[test]
    fn delete_word_left_eats_a_whole_crlf_at_line_start() {
        let text = "foo\r\nbar";
        assert_eq!(left(text, 5), "foo|bar");
        // Blank lines are one run of line-break characters.
        assert_eq!(left("foo\r\n\r\n", 7), "foo|");
        assert_eq!(left("foo\n\nbar", 5), "foo|bar");
    }

    #[test]
    fn delete_word_right_follows_scintilla_classes() {
        assert_eq!(right("foo bar", 0), "|bar");
        assert_eq!(right("foo.bar", 3), "foo|bar");
        assert_eq!(right("foo   ", 0), "|");
        assert_eq!(right("foo\r\nbar", 3), "foo|bar");
        assert_eq!(right("foo", 3), "foo|");
    }

    #[test]
    fn word_deletes_stay_on_char_boundaries_in_multibyte_text() {
        // Accented letters, CJK and emoji are all multi-byte.
        assert_eq!(left("café olé", "café olé".len()), "café |");
        assert_eq!(left("日本語 テキスト", "日本語 テキスト".len()), "日本語 |");
        assert_eq!(right("naïve—text", 0), "|—text");
        assert_eq!(left("a 😀😀", "a 😀😀".len()), "a |");
        // e + combining acute stays one word.
        assert_eq!(left("cafe\u{301}", "cafe\u{301}".len()), "|");
        // A caret inside a multi-byte char snaps back to its start.
        let text = "aé";
        let r = input_part_range(text, 2, CaretPart::WordLeft);
        assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        assert_eq!(r, 0..1);
    }

    #[test]
    fn rope_and_str_paths_agree() {
        let text = "fn main() {\r\n    let x = \"héllo\";\r\n}\r\n";
        let rope = Rope::from_str(text);
        for (byte, _) in text.char_indices().chain([(text.len(), ' ')]) {
            let idx = rope.byte_to_char(byte);
            assert_eq!(
                word_start_before(rope.chars_at(idx).reversed()),
                word_start_before(text[..byte].chars().rev()),
                "left at {byte}"
            );
            assert_eq!(
                word_start_after(rope.chars_at(idx)),
                word_start_after(text[byte..].chars()),
                "right at {byte}"
            );
        }
    }

    #[test]
    fn line_parts_in_inputs() {
        assert_eq!(input_part_range("hello world", 6, CaretPart::LineLeft), 0..6);
        assert_eq!(input_part_range("hello world", 6, CaretPart::LineRight), 6..11);
        assert_eq!(input_part_range("héllo", 99, CaretPart::LineRight), 6..6);
        assert_eq!(input_part_range("héllo", 2, CaretPart::LineLeft), 0..1);
    }

    #[test]
    fn rope_char_boundary_clamps_and_snaps() {
        let rope = Rope::from_str("one\r\ntwo\r\n");
        assert_eq!(rope_char_boundary(&rope, 7), 7);
        assert_eq!(rope_char_boundary(&rope, 99), 10);
        let rope = Rope::from_str("aé😀");
        assert_eq!(rope_char_boundary(&rope, 2), 1);
        assert_eq!(rope_char_boundary(&rope, 5), 3);
        assert_eq!(rope_char_boundary(&Rope::new(), 3), 0);
    }

    #[test]
    fn anchor_tracks_the_deleted_range() {
        assert_eq!(anchor_after_delete(2, &(4..7)), 2);
        assert_eq!(anchor_after_delete(9, &(4..7)), 6);
        assert_eq!(anchor_after_delete(5, &(4..7)), 4);
        assert_eq!(anchor_after_delete(7, &(4..7)), 4);
    }

    #[test]
    fn overtype_replaces_up_to_the_line_end() {
        assert_eq!(overtype_len("abc".chars(), 1), 1);
        assert_eq!(overtype_len("éa".chars(), 1), 2);
        assert_eq!(overtype_len("ab\r\ncd".chars(), 5), 2);
        assert_eq!(overtype_len("\nabc".chars(), 1), 0);
        assert_eq!(overtype_len("".chars(), 1), 0);
        assert_eq!(overtype_len("日本".chars(), 3), 6);
    }

    #[test]
    fn line_scroll_moves_whole_rows() {
        assert_eq!(line_scroll_top(0.0, 20.0, 1), 20.0);
        assert_eq!(line_scroll_top(0.0, 20.0, -1), 0.0);
        assert_eq!(line_scroll_top(50.0, 20.0, 1), 60.0);
        assert_eq!(line_scroll_top(50.0, 20.0, -1), 20.0);
        // Float noise just under a row boundary counts as that row.
        assert_eq!(line_scroll_top(59.99999, 20.0, 1), 80.0);
        assert_eq!(line_scroll_top(40.0, 0.0, 1), 40.0);
    }

    #[test]
    fn caret_is_pulled_back_inside_the_view() {
        // Rows 3..=7 fully visible (top 60, 100px tall, 20px rows).
        assert_eq!(caret_rows_into_view(60.0, 20.0, 100.0, 2), 1);
        assert_eq!(caret_rows_into_view(60.0, 20.0, 100.0, 3), 0);
        assert_eq!(caret_rows_into_view(60.0, 20.0, 100.0, 7), 0);
        assert_eq!(caret_rows_into_view(60.0, 20.0, 100.0, 8), -1);
        // A part-visible row does not count.
        assert_eq!(caret_rows_into_view(50.0, 20.0, 100.0, 2), 1);
        assert_eq!(caret_rows_into_view(50.0, 20.0, 100.0, 7), -1);
        // A view shorter than a row still keeps the caret on the top row.
        assert_eq!(caret_rows_into_view(60.0, 20.0, 10.0, 9), -6);
        assert_eq!(caret_rows_into_view(60.0, 0.0, 100.0, 9), 0);
    }
}
