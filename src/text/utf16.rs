use std::ops::Range;

use gpui::UTF16Selection;

/// Start of the char before `byte`. A `byte` inside a char moves to that
/// char's start; past the end it clamps to `s.len()`.
pub(crate) fn prev_char_byte_in_str(s: &str, byte: usize) -> usize {
    if byte == 0 || byte > s.len() {
        return byte.min(s.len());
    }
    floor_char_boundary(s, byte - 1)
}

/// Start of the char after the one at `byte`. A `byte` inside a char moves
/// to the end of that char.
pub(crate) fn next_char_byte_in_str(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    ceil_char_boundary(s, byte + 1)
}

/// Largest char boundary at or below `byte` (clamped to `s.len()`).
fn floor_char_boundary(s: &str, byte: usize) -> usize {
    let mut i = byte.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary at or above `byte` (clamped to `s.len()`).
fn ceil_char_boundary(s: &str, byte: usize) -> usize {
    let mut i = byte.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub(crate) fn str_byte_to_utf16(s: &str, byte_offset: usize) -> usize {
    let target = byte_offset.min(s.len());
    let mut bytes = 0usize;
    let mut utf16 = 0usize;
    for c in s.chars() {
        if bytes >= target {
            break;
        }
        utf16 += c.len_utf16();
        bytes += c.len_utf8();
    }
    utf16
}

pub(crate) fn str_utf16_to_byte(s: &str, utf16_offset: usize) -> usize {
    let mut utf16 = 0usize;
    let mut bytes = 0usize;
    for c in s.chars() {
        if utf16 >= utf16_offset {
            break;
        }
        utf16 += c.len_utf16();
        bytes += c.len_utf8();
    }
    bytes
}

pub(crate) fn slice_str_chars_utf16(
    s: &str,
    range_utf16: Range<usize>,
    actual_range: &mut Option<Range<usize>>,
) -> String {
    let start_byte = str_utf16_to_byte(s, range_utf16.start).min(s.len());
    let end_byte = str_utf16_to_byte(s, range_utf16.end).min(s.len());
    if start_byte > end_byte {
        return String::new();
    }
    actual_range.replace(str_byte_to_utf16(s, start_byte)..str_byte_to_utf16(s, end_byte));
    s[start_byte..end_byte].to_string()
}

/// Selection variant — returns the UTF-16 range spanning anchor..cursor with
/// `reversed = true` when the cursor sits left of the anchor. This is what
/// the OS asks for via `selected_text_range`; it then passes the same range
/// back through `replace_text_in_range` when the user types over a selection.
pub(crate) fn input_selection_range_utf16(
    s: &str,
    byte_cursor: usize,
    byte_anchor: usize,
) -> UTF16Selection {
    let lo = byte_cursor.min(byte_anchor);
    let hi = byte_cursor.max(byte_anchor);
    let lo_u = str_byte_to_utf16(s, lo);
    let hi_u = str_byte_to_utf16(s, hi);
    UTF16Selection {
        range: lo_u..hi_u,
        reversed: byte_cursor < byte_anchor,
    }
}

/// Apply an IME replace_text_in_range update to one of the String-backed
/// inputs. `byte_cursor` / `byte_anchor` describe the current selection;
/// on return both are updated to a collapsed caret at the end of the
/// inserted text. If `range_utf16` is `None` we replace the live selection
/// (anchor..cursor) — i.e. type-over-selection from the OS path.
pub(crate) fn replace_input_utf16(
    target: &mut String,
    cursor_field: &mut usize,
    anchor_field: &mut usize,
    byte_cursor: usize,
    byte_anchor: usize,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
) {
    let (start_byte, end_byte) = match range_utf16 {
        Some(ru) => (
            str_utf16_to_byte(target, ru.start).min(target.len()),
            str_utf16_to_byte(target, ru.end).min(target.len()),
        ),
        None => {
            // A stale caret can sit inside a char; widen to whole chars
            // rather than let `replace_range` panic.
            let lo = floor_char_boundary(target, byte_cursor.min(byte_anchor));
            let hi = ceil_char_boundary(target, byte_cursor.max(byte_anchor));
            (lo, hi)
        }
    };
    let (lo, hi) = if start_byte <= end_byte {
        (start_byte, end_byte)
    } else {
        (end_byte, start_byte)
    };
    target.replace_range(lo..hi, new_text);
    *cursor_field = lo + new_text.len();
    *anchor_field = *cursor_field;
}

/// UTF-16 offset of `byte_offset` in `rope`; a byte inside a char counts
/// that whole char. The platform input handler asks for this on every
/// keystroke, so it goes through ropey's O(log n) indices: a scan from the
/// start of the rope made typing at the end of a large file crawl.
pub(crate) fn rope_byte_to_utf16(rope: &ropey::Rope, byte_offset: usize) -> usize {
    let target = byte_offset.min(rope.len_bytes());
    let mut char_idx = rope.byte_to_char(target);
    if rope.char_to_byte(char_idx) < target {
        char_idx += 1;
    }
    rope.char_to_utf16_cu(char_idx)
}

/// Byte offset of `utf16_offset` in `rope`, clamped to the end. An offset
/// between the two halves of a surrogate pair counts that whole char.
pub(crate) fn rope_utf16_to_byte(rope: &ropey::Rope, utf16_offset: usize) -> usize {
    let target = utf16_offset.min(rope.len_utf16_cu());
    let mut char_idx = rope.utf16_cu_to_char(target);
    if rope.char_to_utf16_cu(char_idx) < target {
        char_idx += 1;
    }
    rope.char_to_byte(char_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The linear scans these functions replaced; the new ones must agree.
    fn byte_to_utf16_scan(rope: &ropey::Rope, byte_offset: usize) -> usize {
        let target = byte_offset.min(rope.len_bytes());
        let (mut bytes, mut utf16) = (0usize, 0usize);
        for c in rope.chars() {
            if bytes >= target {
                break;
            }
            utf16 += c.len_utf16();
            bytes += c.len_utf8();
        }
        utf16
    }

    fn utf16_to_byte_scan(rope: &ropey::Rope, utf16_offset: usize) -> usize {
        let (mut utf16, mut bytes) = (0usize, 0usize);
        for c in rope.chars() {
            if utf16 >= utf16_offset {
                break;
            }
            utf16 += c.len_utf16();
            bytes += c.len_utf8();
        }
        bytes
    }

    const MIXED: &str = "a\u{e9}\r\n\u{1F600}x\u{4E2D}\u{1F468}\u{200D}\u{1F469}\n\u{feff}z";

    #[test]
    fn rope_offsets_match_the_linear_scan_everywhere() {
        let rope = ropey::Rope::from_str(MIXED);
        for byte in 0..=rope.len_bytes() + 3 {
            assert_eq!(rope_byte_to_utf16(&rope, byte), byte_to_utf16_scan(&rope, byte), "byte {byte}");
        }
        for u in 0..=rope.len_utf16_cu() + 3 {
            assert_eq!(rope_utf16_to_byte(&rope, u), utf16_to_byte_scan(&rope, u), "utf16 {u}");
        }
        let empty = ropey::Rope::new();
        assert_eq!(rope_byte_to_utf16(&empty, 5), 0);
        assert_eq!(rope_utf16_to_byte(&empty, 5), 0);
    }

    #[test]
    fn char_steps_survive_offsets_inside_a_char() {
        let s = "a\u{e9}\u{1F600}";
        // 'a' 0..1, 'é' 1..3, '😀' 3..7
        assert_eq!(prev_char_byte_in_str(s, 2), 1);
        assert_eq!(prev_char_byte_in_str(s, 5), 3);
        assert_eq!(prev_char_byte_in_str(s, 3), 1);
        assert_eq!(prev_char_byte_in_str(s, 0), 0);
        assert_eq!(prev_char_byte_in_str(s, 99), s.len());
        assert_eq!(next_char_byte_in_str(s, 2), 3);
        assert_eq!(next_char_byte_in_str(s, 4), 7);
        assert_eq!(next_char_byte_in_str(s, 1), 3);
        assert_eq!(next_char_byte_in_str(s, 7), 7);
    }

    #[test]
    fn replacing_a_selection_that_ends_inside_a_char_takes_the_whole_char() {
        let mut text = String::from("a\u{e9}b");
        let (mut cursor, mut anchor) = (0, 0);
        replace_input_utf16(&mut text, &mut cursor, &mut anchor, 2, 0, None, "X");
        assert_eq!(text, "Xb");
        assert_eq!((cursor, anchor), (1, 1));
    }

    /// `cargo test --release -p notepad-studio --bin notepad-studio utf16_offset_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn utf16_offset_cost() {
        let line = "fn main() { println!(\"h\u{e9}llo \u{1F600}\"); }\r\n";
        let rope = ropey::Rope::from_str(&line.repeat(64 * 1024 * 1024 / line.len()));
        let end = rope.len_bytes();
        let t = std::time::Instant::now();
        let scan = byte_to_utf16_scan(&rope, end);
        let before = t.elapsed();
        let rounds = 10_000u32;
        let t = std::time::Instant::now();
        let mut fast = 0;
        for _ in 0..rounds {
            fast = rope_byte_to_utf16(&rope, std::hint::black_box(end));
        }
        let after = t.elapsed() / rounds;
        assert_eq!(scan, fast);
        println!("{} MB rope, caret at the end: scan {before:?}, ropey index {after:?}", end >> 20);
    }
}
