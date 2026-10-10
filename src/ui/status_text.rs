// Status-bar field text, built the way Notepad++ builds it:
//   PowerEditor/src/Notepad_plus.cpp:4519 — updateStatusBar: length/lines,
//     Ln/Col with Pos or Sel, INS/OVR
//   PowerEditor/src/Notepad_plus.cpp:3174 — setLangStatus: document type
//     (long names from ScintillaComponent/ScintillaEditView.cpp:99)
//   PowerEditor/src/Notepad_plus.cpp:3180 — setDisplayFormat: EOL field
//   PowerEditor/src/Notepad_plus.cpp:3194 — setUniModeText: encoding field
//   PowerEditor/src/ScintillaComponent/ScintillaEditView.cpp:3627, 4659 —
//     selection line range and char count
//
// Notepad++ reports positions and lengths in what Scintilla holds: the
// document's own line endings (CRLF counts two) and one unit per UTF-8 byte,
// or per character for ANSI. The rope holds LF-only UTF-8, so the
// functions here add the difference back. Every rope query is O(log n)
// except the column scan, which walks the caret's line prefix.

use ropey::Rope;

use crate::syntax::LanguageId;
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;

/// Integer with `,` thousands separators, as `commafyInt` prints it on an
/// English system locale.
pub(crate) fn commafy(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The document-type field: Notepad++'s long language name.
pub(crate) fn doc_type(language: Option<LanguageId>) -> &'static str {
    let Some(language) = language else {
        return "Normal text file";
    };
    match language {
        LanguageId::Rust => "Rust file",
        LanguageId::Python => "Python file",
        LanguageId::C => "C source file",
        LanguageId::CSharp => "C# source file",
        LanguageId::JavaScript => "JavaScript file",
        LanguageId::TypeScript => "TypeScript file",
        // Notepad++ has no TSX, SCSS or built-in Markdown lexer; these follow
        // its "<name> file" pattern.
        LanguageId::Tsx => "TSX file",
        LanguageId::Json => "JSON file",
        LanguageId::Markdown => "Markdown file",
        LanguageId::Bash => "Unix script file",
        LanguageId::PowerShell => "Windows PowerShell",
        LanguageId::Go => "Go source file",
        LanguageId::Java => "Java source file",
        LanguageId::Swift => "Swift file",
        LanguageId::Ruby => "Ruby file",
        LanguageId::Php => "PHP Hypertext Preprocessor file",
        LanguageId::Cpp => "C++ source file",
        LanguageId::Lua => "Lua source File",
        LanguageId::Html => "Hyper Text Markup Language file",
        LanguageId::Css => "Cascade Style Sheets File",
        LanguageId::Scss => "SCSS file",
        LanguageId::Yaml => "YAML Ain't Markup Language",
        LanguageId::Toml => "Tom's Obvious Minimal Language file",
        LanguageId::Xml => "eXtensible Markup Language file",
        LanguageId::Sql => "Structured Query Language file",
    }
}

/// The encoding field. Notepad++ shows its Encoding-menu item text; the
/// UTF-16 writers always emit a BOM, so those read "BOM".
pub(crate) fn encoding_text(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf8Bom => "UTF-8-BOM",
        Encoding::Utf16Le => "UTF-16 LE BOM",
        Encoding::Utf16Be => "UTF-16 BE BOM",
        Encoding::Latin1 => "ANSI",
    }
}

/// Extra units a CRLF document has over the LF-only rope: one per break.
fn crlf_extra(breaks: usize, eol: LineEnding) -> usize {
    if eol == LineEnding::Crlf { breaks } else { 0 }
}

/// Byte offset clamped into the rope and moved back onto a char boundary.
fn clamp_byte(rope: &Rope, byte: usize) -> usize {
    let byte = byte.min(rope.len_bytes());
    rope.char_to_byte(rope.byte_to_char(byte))
}

/// Scintilla units before `byte`: bytes for UTF-8 and UTF-16 (Scintilla
/// holds those as UTF-8), characters for single-byte ANSI.
fn units_before(rope: &Rope, byte: usize, encoding: Encoding) -> usize {
    if encoding == Encoding::Latin1 {
        rope.byte_to_char(byte)
    } else {
        byte
    }
}

/// `SCI_GETLENGTH`: the document length in Scintilla units.
pub(crate) fn doc_length(rope: &Rope, eol: LineEnding, encoding: Encoding) -> usize {
    let end = rope.len_bytes();
    units_before(rope, end, encoding)
        .saturating_add(crlf_extra(rope.len_lines().saturating_sub(1), eol))
}

/// `SCI_GETCURRENTPOS + 1`: the caret's 1-based position.
pub(crate) fn caret_pos(rope: &Rope, head: usize, eol: LineEnding, encoding: Encoding) -> usize {
    let head = clamp_byte(rope, head);
    let line = rope.byte_to_line(head);
    units_before(rope, head, encoding)
        .saturating_add(crlf_extra(line, eol))
        .saturating_add(1)
}

/// `SCI_GETCOLUMN + 1`: the caret's 1-based column, tabs advancing to the
/// next multiple of `tab_width`.
pub(crate) fn caret_column(rope: &Rope, head: usize, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    let head = clamp_byte(rope, head);
    let line_start = rope.line_to_byte(rope.byte_to_line(head));
    let prefix = rope.byte_slice(line_start..head);
    let mut col = 0usize;
    for chunk in prefix.chunks() {
        let mut segments = chunk.split('\t');
        if let Some(first) = segments.next() {
            col = col.saturating_add(first.chars().count());
        }
        for segment in segments {
            col = (col / tab_width).saturating_add(1).saturating_mul(tab_width);
            col = col.saturating_add(segment.chars().count());
        }
    }
    col.saturating_add(1)
}

/// Chars and lines of a stream selection, as `getSelectedCharsAndLinesCount`
/// counts one: line endings are characters, and a selection that ends at a
/// line's start doesn't count that line.
pub(crate) fn stream_selection(
    rope: &Rope,
    start: usize,
    end: usize,
    eol: LineEnding,
) -> (usize, usize) {
    let start = clamp_byte(rope, start);
    let end = clamp_byte(rope, end).max(start);
    let line1 = rope.byte_to_line(start);
    let mut line2 = rope.byte_to_line(end);
    let chars = rope.byte_to_char(end) - rope.byte_to_char(start);
    let chars = chars.saturating_add(crlf_extra(line2 - line1, eol));
    if line2 != line1 && rope.line_to_byte(line2) == end {
        line2 -= 1;
    }
    (chars, line2 - line1 + 1)
}

/// A rectangular selection summarised the way Notepad++ prints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RectSelection {
    pub(crate) rows: usize,
    pub(crate) widest: usize,
    /// Every row selects the same number of characters.
    pub(crate) uniform: bool,
    pub(crate) total: usize,
}

/// Chars of `line` without its line ending.
fn line_chars(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let mut n = slice.len_chars();
    if n > 0 && slice.char(n - 1) == '\n' {
        n -= 1;
        if n > 0 && slice.char(n - 1) == '\r' {
            n -= 1;
        }
    } else if n > 0 && slice.char(n - 1) == '\r' {
        n -= 1;
    }
    n
}

/// Count a column block spanning `lines` and char columns `cols`. Virtual
/// space past a line's end selects nothing on that line.
pub(crate) fn rect_selection(
    rope: &Rope,
    lines: std::ops::RangeInclusive<usize>,
    cols: std::ops::Range<usize>,
) -> RectSelection {
    let last = rope.len_lines().saturating_sub(1);
    let first = (*lines.start()).min(last);
    let end = (*lines.end()).min(last);
    let mut out = RectSelection { rows: 0, widest: 0, uniform: true, total: 0 };
    for line in first..=end {
        let len = line_chars(rope, line);
        let width = cols.end.min(len).saturating_sub(cols.start.min(len));
        if out.rows > 0 && width != out.widest {
            out.uniform = false;
        }
        out.widest = out.widest.max(width);
        out.total = out.total.saturating_add(width);
        out.rows += 1;
    }
    out
}

/// What the Ln/Col field shows after the column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionText {
    Pos(usize),
    Stream { chars: usize, lines: usize },
    Rect(RectSelection),
}

/// The document-size field.
pub(crate) fn doc_size_text(length: usize, lines: usize) -> String {
    format!("length: {}    lines: {}", commafy(length), commafy(lines))
}

/// The caret-position field.
pub(crate) fn cur_pos_text(line: usize, col: usize, selection: SelectionText) -> String {
    let tail = match selection {
        SelectionText::Pos(pos) => format!("Pos: {}", commafy(pos)),
        SelectionText::Stream { chars, lines } => {
            format!("Sel: {} | {}", commafy(chars), commafy(lines))
        }
        SelectionText::Rect(r) => format!(
            "Sel: {}x{}{}{}",
            commafy(r.rows),
            commafy(r.widest),
            if r.uniform { " = " } else { " -> " },
            commafy(r.total)
        ),
    };
    format!("Ln: {}    Col: {}    {tail}", commafy(line), commafy(col))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn commafy_groups_thousands() {
        assert_eq!(commafy(0), "0");
        assert_eq!(commafy(999), "999");
        assert_eq!(commafy(1_000), "1,000");
        assert_eq!(commafy(1_234_567), "1,234,567");
        assert_eq!(commafy(12_345_678), "12,345,678");
    }

    #[test]
    fn doc_length_counts_crlf_as_two_and_ansi_by_char() {
        let r = rope("ab\ncd\n");
        assert_eq!(doc_length(&r, LineEnding::Lf, Encoding::Utf8), 6);
        assert_eq!(doc_length(&r, LineEnding::Cr, Encoding::Utf8), 6);
        assert_eq!(doc_length(&r, LineEnding::Crlf, Encoding::Utf8), 8);
        let r = rope("é中\n😀");
        assert_eq!(doc_length(&r, LineEnding::Lf, Encoding::Utf8), 2 + 3 + 1 + 4);
        assert_eq!(doc_length(&r, LineEnding::Crlf, Encoding::Utf8Bom), 2 + 3 + 2 + 4);
        assert_eq!(doc_length(&r, LineEnding::Lf, Encoding::Latin1), 4);
        assert_eq!(doc_length(&rope(""), LineEnding::Crlf, Encoding::Utf8), 0);
    }

    #[test]
    fn caret_pos_is_one_based_and_counts_crlf() {
        let r = rope("ab\ncd");
        assert_eq!(caret_pos(&r, 0, LineEnding::Lf, Encoding::Utf8), 1);
        assert_eq!(caret_pos(&r, 3, LineEnding::Lf, Encoding::Utf8), 4);
        assert_eq!(caret_pos(&r, 3, LineEnding::Crlf, Encoding::Utf8), 5);
        assert_eq!(caret_pos(&r, 5, LineEnding::Crlf, Encoding::Utf8), 7);
    }

    #[test]
    fn caret_pos_survives_offsets_off_boundaries_and_past_the_end() {
        let r = rope("é\nx");
        // Byte 1 is inside "é": it lands back on 0.
        assert_eq!(caret_pos(&r, 1, LineEnding::Lf, Encoding::Utf8), 1);
        assert_eq!(caret_pos(&r, 999, LineEnding::Lf, Encoding::Utf8), 5);
        assert_eq!(caret_pos(&r, 999, LineEnding::Lf, Encoding::Latin1), 4);
        assert_eq!(caret_pos(&rope(""), 7, LineEnding::Crlf, Encoding::Utf8), 1);
    }

    #[test]
    fn caret_column_expands_tabs_to_stops() {
        let r = rope("\tab\n");
        assert_eq!(caret_column(&r, 0, 4), 1);
        assert_eq!(caret_column(&r, 1, 4), 5);
        assert_eq!(caret_column(&r, 3, 4), 7);
        let r = rope("ab\tc");
        assert_eq!(caret_column(&r, 3, 4), 5);
        assert_eq!(caret_column(&r, 3, 8), 9);
        let r = rope("abcd\t");
        assert_eq!(caret_column(&r, 5, 4), 9);
        // Zero width from a corrupt setting acts as one.
        assert_eq!(caret_column(&r, 5, 0), 6);
    }

    #[test]
    fn caret_column_counts_chars_not_bytes() {
        let r = rope("x\né中😀z");
        let head = r.len_bytes();
        assert_eq!(caret_column(&r, head, 4), 5);
        assert_eq!(caret_column(&r, 3, 4), 1);
        assert_eq!(caret_column(&r, 4, 4), 2);
    }

    #[test]
    fn caret_column_on_a_long_line_spanning_chunks() {
        let line = "a\t".repeat(50_000);
        let r = rope(&line);
        assert_eq!(caret_column(&r, r.len_bytes(), 4), 50_000 * 4 + 1);
    }

    #[test]
    fn stream_selection_counts_chars_and_lines() {
        let r = rope("abc\ndef\nghi");
        assert_eq!(stream_selection(&r, 0, 2, LineEnding::Lf), (2, 1));
        assert_eq!(stream_selection(&r, 1, 5, LineEnding::Lf), (4, 2));
        assert_eq!(stream_selection(&r, 1, 5, LineEnding::Crlf), (5, 2));
        // Ending at the start of a line leaves that line out.
        assert_eq!(stream_selection(&r, 0, 4, LineEnding::Lf), (4, 1));
        assert_eq!(stream_selection(&r, 0, 8, LineEnding::Crlf), (10, 2));
        assert_eq!(stream_selection(&r, 0, r.len_bytes(), LineEnding::Lf), (11, 3));
    }

    #[test]
    fn stream_selection_with_multibyte_and_stale_offsets() {
        let r = rope("é中\n😀");
        assert_eq!(stream_selection(&r, 0, r.len_bytes(), LineEnding::Lf), (4, 2));
        assert_eq!(stream_selection(&r, 1, 4, LineEnding::Lf), (1, 1));
        assert_eq!(stream_selection(&r, 50, 900, LineEnding::Lf), (0, 1));
        assert_eq!(stream_selection(&r, 9, 2, LineEnding::Lf), (0, 1));
    }

    #[test]
    fn rect_selection_ignores_virtual_space() {
        let r = rope("abcdef\nab\nabcdef\r\n");
        let s = rect_selection(&r, 0..=2, 1..4);
        assert_eq!(s, RectSelection { rows: 3, widest: 3, uniform: false, total: 3 + 1 + 3 });
        let s = rect_selection(&r, 0..=0, 1..4);
        assert_eq!(s, RectSelection { rows: 1, widest: 3, uniform: true, total: 3 });
        // Lines past the end are clamped to the last line.
        let s = rect_selection(&r, 2..=99, 0..2);
        assert_eq!(s.rows, 2);
        assert_eq!(rect_selection(&rope(""), 0..=5, 0..5).rows, 1);
    }

    #[test]
    fn field_text_matches_notepad_plus_plus() {
        assert_eq!(doc_size_text(1234, 56), "length: 1,234    lines: 56");
        assert_eq!(cur_pos_text(12, 5, SelectionText::Pos(1001)), "Ln: 12    Col: 5    Pos: 1,001");
        assert_eq!(
            cur_pos_text(3, 1, SelectionText::Stream { chars: 10, lines: 2 }),
            "Ln: 3    Col: 1    Sel: 10 | 2"
        );
        let rect = RectSelection { rows: 3, widest: 4, uniform: true, total: 12 };
        assert_eq!(cur_pos_text(1, 1, SelectionText::Rect(rect)), "Ln: 1    Col: 1    Sel: 3x4 = 12");
        let rect = RectSelection { uniform: false, total: 9, ..rect };
        assert_eq!(cur_pos_text(1, 1, SelectionText::Rect(rect)), "Ln: 1    Col: 1    Sel: 3x4 -> 9");
    }

    #[test]
    fn every_language_and_encoding_has_a_label() {
        assert_eq!(doc_type(None), "Normal text file");
        for lang in LanguageId::all() {
            assert!(!doc_type(Some(*lang)).is_empty());
        }
        assert_eq!(encoding_text(Encoding::Utf8Bom), "UTF-8-BOM");
        assert_eq!(encoding_text(Encoding::Latin1), "ANSI");
    }
}
