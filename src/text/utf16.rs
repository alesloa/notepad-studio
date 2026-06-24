use std::ops::Range;

use gpui::UTF16Selection;

pub(crate) fn prev_char_byte_in_str(s: &str, byte: usize) -> usize {
    if byte == 0 || byte > s.len() {
        return byte.min(s.len()).saturating_sub(0);
    }
    let mut iter = s[..byte].char_indices();
    iter.next_back().map(|(i, _)| i).unwrap_or(0)
}

pub(crate) fn next_char_byte_in_str(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let rest = &s[byte..];
    let mut chars = rest.char_indices();
    chars.next();
    if let Some((i, _)) = chars.next() {
        byte + i
    } else {
        s.len()
    }
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
            let lo = byte_cursor.min(byte_anchor).min(target.len());
            let hi = byte_cursor.max(byte_anchor).min(target.len());
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

pub(crate) fn rope_byte_to_utf16(rope: &ropey::Rope, byte_offset: usize) -> usize {
    let total = rope.len_bytes();
    let target = byte_offset.min(total);
    let mut bytes_seen = 0usize;
    let mut utf16 = 0usize;
    for c in rope.chars() {
        if bytes_seen >= target {
            break;
        }
        utf16 += c.len_utf16();
        bytes_seen += c.len_utf8();
    }
    utf16
}

pub(crate) fn rope_utf16_to_byte(rope: &ropey::Rope, utf16_offset: usize) -> usize {
    let mut utf16 = 0usize;
    let mut bytes = 0usize;
    for c in rope.chars() {
        if utf16 >= utf16_offset {
            break;
        }
        utf16 += c.len_utf16();
        bytes += c.len_utf8();
    }
    bytes
}
