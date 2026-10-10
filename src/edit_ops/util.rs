// Shared helpers for the Edit-submenu ops and Text Tools: splitting text into
// lines that keep their own terminator, and clamping byte offsets that came
// from a selection and may be stale.
//
// The rope is LF-internal (`text::line_ending`), but a mixed-EOL file can
// still carry a lone `\r`, and a raw `\r\n` survives when `LineEnding::detect`
// (first 4 KiB only) picked LF. Ops that rebuild lines therefore reuse the
// terminators they found instead of inserting the buffer's on-disk EOL, which
// the save path would expand a second time (`\r\n` -> `\r\r\n`).
//
// Line ends are CRLF, LF and CR, the three Scintilla recognises.

use std::ops::Range;

use ropey::Rope;

/// Split one line into its body and its trailing CRLF, LF or CR (`""` when
/// it has none).
pub(crate) fn split_line_terminator(line: &str) -> (&str, &str) {
    if let Some(s) = line.strip_suffix("\r\n") {
        (s, "\r\n")
    } else if let Some(s) = line.strip_suffix('\n') {
        (s, "\n")
    } else if let Some(s) = line.strip_suffix('\r') {
        (s, "\r")
    } else {
        (line, "")
    }
}

/// Split `text` into `(body, terminator)` pieces on CRLF, LF and CR. Every
/// piece but the last has a terminator; the last never does. So text that
/// ends with a line end yields a final `("", "")`, and empty text yields one
/// `("", "")`, the same piece count as `str::split('\n')` on LF-only text.
pub(crate) fn split_lines_eol(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(['\r', '\n']) {
        let eol_len = if rest.get(pos..pos + 2) == Some("\r\n") { 2 } else { 1 };
        // `pos` and `pos + eol_len` sit on ASCII bytes, so every `get` below
        // succeeds; on the impossible miss the remainder stays one piece.
        let (Some(body), Some(eol), Some(tail)) = (
            rest.get(..pos),
            rest.get(pos..pos + eol_len),
            rest.get(pos + eol_len..),
        ) else {
            break;
        };
        out.push((body, eol));
        rest = tail;
    }
    out.push((rest, ""));
    out
}

/// First line terminator in `text`, or LF, the rope's own separator, when the
/// text has none. Used where an op must insert a new line end.
pub(crate) fn first_terminator(text: &str) -> &str {
    split_lines_eol(text)
        .into_iter()
        .map(|(_, eol)| eol)
        .find(|eol| !eol.is_empty())
        .unwrap_or("\n")
}

/// Clamp `byte` to the rope and move it back to the start of the char it
/// falls in.
pub(crate) fn floor_char_boundary(rope: &Rope, byte: usize) -> usize {
    let b = byte.min(rope.len_bytes());
    rope.char_to_byte(rope.byte_to_char(b))
}

/// `range` ordered, clamped to the rope and floored to char boundaries, so
/// it is always safe to slice or replace.
pub(crate) fn clamp_byte_range(rope: &Rope, range: Range<usize>) -> Range<usize> {
    let a = floor_char_boundary(rope, range.start);
    let b = floor_char_boundary(rope, range.end);
    a.min(b)..a.max(b)
}

/// Clamp `byte` to `s` and move it back to the start of the char it falls in.
pub(crate) fn floor_str_boundary(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_eol_keeps_each_terminator() {
        let cases: &[(&str, &[(&str, &str)])] = &[
            ("", &[("", "")]),
            ("a", &[("a", "")]),
            ("a\n", &[("a", "\n"), ("", "")]),
            ("a\r\nb", &[("a", "\r\n"), ("b", "")]),
            ("a\rb\r", &[("a", "\r"), ("b", "\r"), ("", "")]),
            ("a\r\nb\nc\rd", &[("a", "\r\n"), ("b", "\n"), ("c", "\r"), ("d", "")]),
            ("\r\r\n\n", &[("", "\r"), ("", "\r\n"), ("", "\n"), ("", "")]),
            ("é\n👨‍👩‍👧\r\n🇯🇵", &[("é", "\n"), ("👨‍👩‍👧", "\r\n"), ("🇯🇵", "")]),
        ];
        for (input, want) in cases {
            assert_eq!(split_lines_eol(input), want.to_vec(), "input {input:?}");
            let rebuilt: String = split_lines_eol(input)
                .iter()
                .map(|(b, e)| format!("{b}{e}"))
                .collect();
            assert_eq!(&rebuilt, input);
        }
    }

    #[test]
    fn first_terminator_falls_back_to_lf() {
        assert_eq!(first_terminator("a\r\nb\n"), "\r\n");
        assert_eq!(first_terminator("a\rb"), "\r");
        assert_eq!(first_terminator("abc"), "\n");
        assert_eq!(first_terminator(""), "\n");
    }

    #[test]
    fn floor_char_boundary_clamps_and_snaps() {
        let rope = Rope::from_str("aé中");
        // a=0, é=1..3, 中=3..6
        assert_eq!(floor_char_boundary(&rope, 0), 0);
        assert_eq!(floor_char_boundary(&rope, 2), 1);
        assert_eq!(floor_char_boundary(&rope, 4), 3);
        assert_eq!(floor_char_boundary(&rope, 6), 6);
        assert_eq!(floor_char_boundary(&rope, 99), 6);
        // Reversed range, as an anchor-after-head selection gives.
        assert_eq!(clamp_byte_range(&rope, Range { start: 5, end: 2 }), 1..3);
        assert_eq!(clamp_byte_range(&rope, 2..99), 1..6);
        let empty = Rope::new();
        assert_eq!(clamp_byte_range(&empty, 3..7), 0..0);
    }

    #[test]
    fn floor_str_boundary_clamps_and_snaps() {
        let s = "aé中";
        assert_eq!(floor_str_boundary(s, 2), 1);
        assert_eq!(floor_str_boundary(s, 5), 3);
        assert_eq!(floor_str_boundary(s, 99), 6);
        assert_eq!(floor_str_boundary("", 3), 0);
    }
}
