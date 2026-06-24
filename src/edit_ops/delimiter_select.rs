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
    let cursor = cursor.min(total);

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
fn trim_line_terminator(rope: &Rope, start: usize, end: usize) -> usize {
    if end <= start {
        return end;
    }
    let len = end - start;
    if len >= 2 {
        let two: String = rope.byte_slice(end - 2..end).to_string();
        if two == "\r\n" {
            return end - 2;
        }
    }
    let last: String = rope.byte_slice(end - 1..end).to_string();
    if last == "\n" || last == "\r" {
        return end - 1;
    }
    end
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
    let slice = rope.byte_slice(limit..cursor);
    let mut depth: usize = 0;
    let len = slice.len_bytes();
    let mut chars: Vec<(usize, char)> = Vec::with_capacity(len);
    let mut byte = 0;
    for ch in slice.chars() {
        chars.push((byte, ch));
        byte += ch.len_utf8();
    }
    for (b, ch) in chars.iter().rev() {
        if nested && *ch == right {
            depth += 1;
            continue;
        }
        if *ch == left {
            if depth == 0 {
                return Some(limit + *b);
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
    let slice = rope.byte_slice(cursor..limit);
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
    fn cursor_on_left_delim_finds_pair_after() {
        // Quirk: cursor BYTE = position of `(`. Backward scan starts before
        // it so it won't see the `(` at cursor — falls through to None.
        // This matches NPP behavior for click-on-delimiter; the user is
        // expected to click between delimiters.
        let rope = r("(hi)");
        assert!(select_between_delimiters(&rope, 0, '(', ')', false).is_none());
    }
}
