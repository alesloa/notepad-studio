// Delimiter selection — NPP `Edit ▸ Begin/End Select` style "select between
// delimiters" triggered by Ctrl+double-click. Mirrors
// `NppGUI::_leftmostDelimiter` / `_rightmostDelimiter` /
// `_delimiterSelectionOnEntireDocument` (Parameters.h:769-771).
//
// Algorithm:
//   1. From the click byte offset, walk backward looking for `left`. When
//      `left != right`, track depth so nested pairs are skipped.
//   2. Walk forward looking for `right` with the same depth tracking.
//   3. Return the inner range (exclusive of the delimiters). On `whole_doc =
//      false` either scan stops at the surrounding line bounds.
//
// Pure function; no GPUI/Workspace deps so the unit tests can exercise it
// directly against a `ropey::Rope`.

use std::ops::Range;

use ropey::Rope;

use crate::edit_ops::util::floor_char_boundary;

/// Find the inner range between matching delimiters around `cursor`.
/// Returns `None` if either side is missing or if the delimiters scan past
/// the allowed bound (line ends when `whole_doc == false`).
pub(crate) fn select_between_delimiters(
    rope: &Rope,
    cursor: usize,
    left: char,
    right: char,
    whole_doc: bool,
) -> Option<Range<usize>> {
    let total = rope.len_bytes();
    // A click offset off a char boundary would panic the slices below.
    let cursor = floor_char_boundary(rope, cursor);

    let (back_limit, fwd_limit) = if whole_doc {
        (0, total)
    } else {
        line_bounds(rope, cursor)
    };

    let nested = left != right;

    let open_byte = scan_back(rope, cursor, back_limit, left, right, nested)?;
    let close_byte = scan_fwd(rope, cursor, fwd_limit, left, right, nested)?;

    let inner_start = open_byte + left.len_utf8();
    if close_byte < inner_start {
        return None;
    }
    Some(inner_start..close_byte)
}

/// Inclusive byte bounds of the line containing `byte` — `(line_start,
/// line_end_excluding_terminator)`. Used when `whole_doc == false` to clamp
/// the delimiter scan to the current line.
fn line_bounds(rope: &Rope, byte: usize) -> (usize, usize) {
    let line = rope.byte_to_line(byte.min(rope.len_bytes()));
    let start = rope.line_to_byte(line);
    let next_start = if line + 1 < rope.len_lines() {
        rope.line_to_byte(line + 1)
    } else {
        rope.len_bytes()
    };
    let end = trim_line_terminator(rope, start, next_start);
    (start, end)
}

/// Strip a trailing CRLF / LF / CR (in that order) from `[start, end)`.
/// Reads single bytes: the char before the line end may be multi-byte, so
/// slicing the last two bytes could cut into it.
fn trim_line_terminator(rope: &Rope, start: usize, end: usize) -> usize {
    if end <= start {
        return end;
    }
    let last = end.checked_sub(1).and_then(|i| rope.get_byte(i));
    let before = end
        .checked_sub(2)
        .filter(|&i| i >= start)
        .and_then(|i| rope.get_byte(i));
    match (before, last) {
        (Some(b'\r'), Some(b'\n')) => end - 2,
        (_, Some(b'\n' | b'\r')) => end - 1,
        _ => end,
    }
}

/// Walk backward from `cursor` looking for `left`. With `nested == true`,
/// every encountered `right` increments the depth so a matching outer
/// `left` is required to clear it. Returns the byte offset of `left`.
fn scan_back(
    rope: &Rope,
    cursor: usize,
    limit: usize,
    left: char,
    right: char,
    nested: bool,
) -> Option<usize> {
    let slice = rope.get_byte_slice(limit..cursor)?;
    let mut depth: usize = 0;
    // Walk the chars in place, back to front. Collecting them first cost 16
    // bytes per byte scanned, gigabytes on a large file with "match entire
    // document" on.
    let mut b = slice.len_bytes();
    let mut chars = slice.chars_at(slice.len_chars());
    while let Some(ch) = chars.prev() {
        b = b.saturating_sub(ch.len_utf8());
        if nested && ch == right {
            depth += 1;
            continue;
        }
        if ch == left {
            if depth == 0 {
                return Some(limit + b);
            }
            depth = depth.saturating_sub(1);
        }
    }
    None
}

/// Walk forward from `cursor` looking for `right`. With `nested == true`,
/// every encountered `left` increments the depth so a matching outer
/// `right` is required to clear it.
fn scan_fwd(
    rope: &Rope,
    cursor: usize,
    limit: usize,
    left: char,
    right: char,
    nested: bool,
) -> Option<usize> {
    if cursor >= limit {
        return None;
    }
    let slice = rope.get_byte_slice(cursor..limit)?;
    let mut depth: usize = 0;
    let mut byte = 0;
    for ch in slice.chars() {
        if nested && ch == left {
            depth += 1;
        } else if ch == right {
            if depth == 0 {
                return Some(cursor + byte);
            }
            depth = depth.saturating_sub(1);
        }
        byte += ch.len_utf8();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn finds_simple_pair() {
        let rope = r("foo (bar) baz");
        let range = select_between_delimiters(&rope, 6, '(', ')', false).unwrap();
        assert_eq!(&rope.byte_slice(range).to_string(), "bar");
    }

    #[test]
    fn handles_nested_pairs() {
        let rope = r("a (b (c) d) e");
        // cursor inside outer, between nested
        let range = select_between_delimiters(&rope, 9, '(', ')', false).unwrap();
        assert_eq!(&rope.byte_slice(range).to_string(), "b (c) d");
    }

    #[test]
    fn nested_inner_when_cursor_inside() {
        let rope = r("a (b (c) d) e");
        // cursor between 'c' and ')' of inner pair (offset 6 = inside (c))
        let range = select_between_delimiters(&rope, 6, '(', ')', false).unwrap();
        assert_eq!(&rope.byte_slice(range).to_string(), "c");
    }

    #[test]
    fn returns_none_when_left_missing() {
        let rope = r("foo bar) baz");
        assert!(select_between_delimiters(&rope, 5, '(', ')', false).is_none());
    }

    #[test]
    fn returns_none_when_right_missing() {
        let rope = r("foo (bar baz");
        assert!(select_between_delimiters(&rope, 6, '(', ')', false).is_none());
    }

    #[test]
    fn line_scope_blocks_cross_line_match() {
        let rope = r("foo (\nbar\n) baz");
        // bar on its own line — left is on prior line, right on next.
        // whole_doc = false should fail.
        assert!(select_between_delimiters(&rope, 7, '(', ')', false).is_none());
        // whole_doc = true should succeed.
        let range = select_between_delimiters(&rope, 7, '(', ')', true).unwrap();
        assert_eq!(&rope.byte_slice(range).to_string(), "\nbar\n");
    }

    #[test]
    fn same_char_delimiter_no_nesting() {
        let rope = r("'foo' 'bar'");
        // cursor inside first pair
        let range = select_between_delimiters(&rope, 2, '\'', '\'', false).unwrap();
        assert_eq!(&rope.byte_slice(range).to_string(), "foo");
    }

    #[test]
    fn empty_pair_is_empty_range() {
        let rope = r("()");
        let range = select_between_delimiters(&rope, 1, '(', ')', false).unwrap();
        assert_eq!(range.start, range.end);
    }

    #[test]
    fn multi_byte_char_before_line_end() {
        // The line-end check sliced the last two bytes, which cut into `中`.
        let rope = r("(a)中\nx");
        assert_eq!(select_between_delimiters(&rope, 1, '(', ')', false), Some(1..2));
        let rope = r("x\r\n(a)中");
        assert_eq!(select_between_delimiters(&rope, 4, '(', ')', false), Some(4..5));
        let rope = r("(é)\r\n(👨‍👩‍👧)🇯🇵\r");
        assert_eq!(select_between_delimiters(&rope, 7, '(', ')', false), Some(7..25));
    }

    #[test]
    fn backward_scan_reports_byte_offsets_across_multi_byte_chars() {
        // 中=0..3 (=3 é=4..6 (=6 x=7 )=8 ü=9..11 )=11 🇯🇵=12..20
        let rope = r("中(é(x)ü)🇯🇵");
        assert_eq!(select_between_delimiters(&rope, 9, '(', ')', true), Some(4..11));
        assert_eq!(select_between_delimiters(&rope, 7, '(', ')', true), Some(7..8));
        assert_eq!(select_between_delimiters(&rope, 12, '(', ')', true), None);
    }

    #[test]
    fn cursor_inside_a_char_or_past_the_end() {
        // Byte 2 is inside `中` (1..4): floored to 1.
        let rope = r("(中)");
        assert_eq!(select_between_delimiters(&rope, 2, '(', ')', false), Some(1..4));
        assert_eq!(select_between_delimiters(&rope, 99, '(', ')', false), None);
        assert_eq!(select_between_delimiters(&r(""), 3, '(', ')', true), None);
    }

    #[test]
    fn cursor_on_left_delim_finds_pair_after() {
        // Quirk: cursor BYTE = position of `(`. Backward scan starts before
        // it so it won't see the `(` at cursor — falls through to None.
        // This matches NPP behavior for click-on-delimiter; the user is
        // expected to click between delimiters.
        let rope = r("(hi)");
        assert!(select_between_delimiters(&rope, 0, '(', ')', false).is_none());
    }
}
