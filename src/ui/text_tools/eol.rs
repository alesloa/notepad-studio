// Line output shared by the Text Tools transforms.
//
// The transforms split their input on CRLF, LF and CR
// (`edit_ops::util::split_lines_eol`) and give every output line a line end
// taken from the input, so CRLF and mixed-EOL text comes back with the line
// ends it went in with. Joining on a bare "\n" turned "a\r\nb" into "a\r" +
// suffix + "\n", splitting a CRLF around the suffix.

/// Status shown when a result would merge two lines (see `LineWriter`).
pub(super) const MERGED_LINES: &str = "Skipped: a line ending in CR would join the LF-only line after it. \
     Make the line endings match first (Edit > EOL Conversion).";

/// Builds a transform's output line by line. Dropping or reordering lines
/// can put a CR line end right before an LF one, which then reads back as a
/// single CRLF and merges two lines; `finish` returns `None` in that case.
#[derive(Default)]
pub(super) struct LineWriter {
    out: String,
    merged: bool,
}

impl LineWriter {
    fn push(&mut self, s: &str) {
        if self.out.ends_with('\r') && s.starts_with('\n') {
            self.merged = true;
        }
        self.out.push_str(s);
    }

    /// Append `body` followed by its line end `eol` (`""` for none).
    pub(super) fn line(&mut self, body: &str, eol: &str) {
        self.push(body);
        self.push(eol);
    }

    pub(super) fn finish(self) -> Option<String> {
        (!self.merged).then_some(self.out)
    }
}

/// Join the lines a filter kept, each with its own line end except the
/// last, which gets none (the old `join("\n")` shape). A kept line with no
/// line end in a middle position falls back to LF, the rope's separator.
pub(super) fn join_kept(kept: &[(&str, &str)]) -> Option<String> {
    let mut w = LineWriter::default();
    let last = kept.len().saturating_sub(1);
    for (i, &(body, eol)) in kept.iter().enumerate() {
        let eol = match (i == last, eol) {
            (true, _) => "",
            (false, "") => "\n",
            (false, eol) => eol,
        };
        w.line(body, eol);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_kept_keeps_each_line_end_and_refuses_a_merge() {
        assert_eq!(join_kept(&[]).as_deref(), Some(""));
        assert_eq!(join_kept(&[("a", "\r\n"), ("b", "\n")]).as_deref(), Some("a\r\nb"));
        assert_eq!(join_kept(&[("a", ""), ("b", "\r")]).as_deref(), Some("a\nb"));
        assert_eq!(join_kept(&[("a", "\r"), ("é", "\r\n"), ("中", "")]).as_deref(), Some("a\ré\r\n中"));
        // CR then an empty LF line: "a\r\n" would be one line.
        assert_eq!(join_kept(&[("a", "\r"), ("", "\n"), ("b", "")]), None);
        assert_eq!(join_kept(&[("a", "\r"), ("", "\r\n"), ("b", "")]).as_deref(), Some("a\r\r\nb"));
    }
}
