// Line endings for text crossing the editor's edge: clipboard and typed
// input coming in, copied text going out.
//
// The rope is LF-internal: `io.rs` normalizes a file's line endings to `\n`
// on load and `line_ending::rope_to_bytes` puts the document's ending back on
// save. Text that enters the rope must follow the same rule, or a CRLF paste
// lands as `\r\n` and is written back as `\r\r\n`.
//
// scintilla/src/Editor.cxx:214 — `convertPastes = true`: Scintilla converts
// pasted line endings to the document's EOL mode, and Notepad++ never sends
// SCI_SETPASTECONVERTENDINGS to turn it off. Copy hands out the document's
// own bytes, so a copy from a Windows file carries CR LF.

use std::borrow::Cow;

use super::line_ending::LineEnding;

/// Text to insert into the rope: every CR LF and every lone CR becomes LF,
/// whatever the document's line ending, because the rope only holds LF.
/// Borrows when there is no CR to convert.
pub(crate) fn to_rope_eols(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Text copied out of the rope, with the document's line ending put back,
/// as `rope_to_bytes` does on save. A CR LF pair already in the rope (a
/// mixed file kept as it was loaded) stays one pair.
pub(crate) fn to_doc_eols(text: &str, eol: LineEnding) -> Cow<'_, str> {
    if eol == LineEnding::Lf || !text.contains('\n') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len().saturating_add(text.len() / 16));
    let mut prev_cr = false;
    for c in text.chars() {
        if c == '\n' {
            match eol {
                LineEnding::Crlf => {
                    if !prev_cr {
                        out.push('\r');
                    }
                    out.push('\n');
                }
                // CR LF collapses to the CR already written; a lone LF
                // becomes CR.
                _ => {
                    if !prev_cr {
                        out.push('\r');
                    }
                }
            }
        } else {
            out.push(c);
        }
        prev_cr = c == '\r';
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_crlf_and_cr_become_lf() {
        assert_eq!(to_rope_eols("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(to_rope_eols("a\rb\r"), "a\nb\n");
        assert_eq!(to_rope_eols("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(to_rope_eols("\r\r\n"), "\n\n");
        assert_eq!(to_rope_eols("\r"), "\n");
    }

    #[test]
    fn incoming_without_cr_is_borrowed() {
        assert!(matches!(to_rope_eols("a\nb"), Cow::Borrowed("a\nb")));
        assert!(matches!(to_rope_eols(""), Cow::Borrowed("")));
    }

    #[test]
    fn incoming_keeps_multibyte_text() {
        assert_eq!(to_rope_eols("é\r\n漢\r😀"), "é\n漢\n😀");
    }

    #[test]
    fn outgoing_matches_the_document_ending() {
        assert_eq!(to_doc_eols("a\nb\n", LineEnding::Crlf), "a\r\nb\r\n");
        assert_eq!(to_doc_eols("a\nb\n", LineEnding::Cr), "a\rb\r");
        assert!(matches!(to_doc_eols("a\nb", LineEnding::Lf), Cow::Borrowed("a\nb")));
        assert!(matches!(to_doc_eols("ab", LineEnding::Crlf), Cow::Borrowed("ab")));
    }

    #[test]
    fn outgoing_keeps_existing_pairs_single() {
        assert_eq!(to_doc_eols("a\r\nb\n", LineEnding::Crlf), "a\r\nb\r\n");
        assert_eq!(to_doc_eols("a\r\nb\n", LineEnding::Cr), "a\rb\r");
        assert_eq!(to_doc_eols("é\n漢", LineEnding::Crlf), "é\r\n漢");
    }

    #[test]
    fn pasted_crlf_saves_as_one_crlf() {
        // Regression: a CR LF paste into a Windows file used to go into the
        // rope as-is and save as CR CR LF.
        let rope = ropey::Rope::from_str(&to_rope_eols("a\r\nb\r\n"));
        let saved = crate::text::line_ending::rope_to_bytes(&rope, LineEnding::Crlf);
        assert_eq!(saved, b"a\r\nb\r\n");
        let rope = ropey::Rope::from_str(&to_rope_eols("a\rb"));
        assert_eq!(crate::text::line_ending::rope_to_bytes(&rope, LineEnding::Lf), b"a\nb");
    }

    #[test]
    fn pasting_a_copy_back_round_trips() {
        for eol in [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr] {
            let rope_text = "one\ntwo\n\nthree";
            let copied = to_doc_eols(rope_text, eol);
            assert_eq!(to_rope_eols(&copied), rope_text, "{eol:?}");
        }
    }
}
