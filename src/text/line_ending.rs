#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LineEnding {
    Lf,
    Crlf,
    /// Classic Mac line ending. Detected when reading legacy files; never the
    /// default for a new buffer.
    Cr,
}

impl LineEnding {
    /// Default line ending for new buffers on this platform.
    pub(crate) fn platform_default() -> Self {
        if cfg!(target_os = "windows") {
            Self::Crlf
        } else {
            Self::Lf
        }
    }

    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
            Self::Cr => "\r",
        }
    }

    /// Short token used in debug labels and the session manifest.
    #[allow(dead_code)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Cr => "CR",
        }
    }

    /// Notepad++-style status bar label.
    pub(crate) fn status_label(self) -> &'static str {
        match self {
            Self::Lf => "Unix (LF)",
            Self::Crlf => "Windows (CR LF)",
            Self::Cr => "Macintosh (CR)",
        }
    }

    /// Cycle to the next mode. Status-bar click handler uses this.
    /// Order matches the NPP cycle: Windows → Unix → Macintosh → Windows.
    pub(crate) fn cycle_next(self) -> Self {
        match self {
            Self::Crlf => Self::Lf,
            Self::Lf => Self::Cr,
            Self::Cr => Self::Crlf,
        }
    }

    /// Pick the line ending a freshly loaded file is opened with.
    ///
    /// Notepad++ takes the format of the first line break in the file
    /// (`FileManager::loadFileData` → `getEOLFormatForm`); a file with no
    /// line break gets the new-document default. We do the same, with one
    /// guard: the rope is LF-internal, so a CRLF or CR file can only be
    /// normalized if converting back on save gives the same bytes. A file
    /// that mixes line endings in a way that wouldn't survive that trip
    /// (a lone LF in a CRLF file, any LF in a CR file, CR CR LF) opens in
    /// LF mode instead, with its CRs kept in the rope, so open + save is
    /// byte-identical. The editor already strips `\r` from line ends for
    /// display, and ropey counts CR and CRLF as line breaks.
    pub(crate) fn detect(text: &str) -> Self {
        let bytes = text.as_bytes();
        let Some(first) = bytes.iter().position(|&b| b == b'\r' || b == b'\n') else {
            return Self::platform_default();
        };
        if bytes.get(first) == Some(&b'\n') {
            return Self::Lf;
        }
        if bytes.get(first.saturating_add(1)) == Some(&b'\n') {
            if crlf_round_trips(bytes) {
                Self::Crlf
            } else {
                Self::Lf
            }
        } else if bytes.contains(&b'\n') {
            Self::Lf
        } else {
            Self::Cr
        }
    }
}

/// True when every LF in `bytes` is the tail of a CR LF pair whose CR is
/// not itself preceded by another CR. Exactly the files where
/// `normalize_to_lf(.., Crlf)` followed by `rope_to_bytes(.., Crlf)` gives
/// back the input: a lone LF would come back as CR LF, and CR CR LF would
/// come back as CR LF.
fn crlf_round_trips(bytes: &[u8]) -> bool {
    bytes.iter().enumerate().all(|(i, &b)| {
        if b != b'\n' {
            return true;
        }
        let prev = i.checked_sub(1).and_then(|p| bytes.get(p));
        let prev2 = i.checked_sub(2).and_then(|p| bytes.get(p));
        prev == Some(&b'\r') && prev2 != Some(&b'\r')
    })
}

/// Convert any line ending in `text` to plain LF — Notepad Studio's rope is
/// always LF-internal, the EOL on disk is reapplied at save time. `eol`
/// should come from `LineEnding::detect` on the same text (or use
/// `split_line_endings`), which guarantees the conversion is reversible.
pub(crate) fn normalize_to_lf(text: String, eol: LineEnding) -> String {
    match eol {
        LineEnding::Lf => text,
        LineEnding::Crlf => text.replace("\r\n", "\n"),
        LineEnding::Cr => text.replace('\r', "\n"),
    }
}

/// `detect` + `normalize_to_lf` in one step: the line ending to open the
/// file with and the text to put in the rope.
pub(crate) fn split_line_endings(text: String) -> (LineEnding, String) {
    let eol = LineEnding::detect(&text);
    (eol, normalize_to_lf(text, eol))
}

/// Serialize the rope into the on-disk byte stream using the buffer's
/// chosen line ending.
///
/// - `Lf` writes the rope as-is. A file opened with mixed line endings
///   lives in this mode, so its bytes survive a save untouched.
/// - `Crlf` turns every LF that isn't already part of a CR LF pair into
///   CR LF. Pairs already in the rope (from a mixed file the user switched
///   to Windows, or a `\r\n` typed through Extended replace) stay single.
/// - `Cr` turns CR LF and lone LF into CR.
///
/// Walks the rope's chunks instead of flattening it first, so a save holds
/// one copy of the document plus the output rather than three.
pub(crate) fn rope_to_bytes(rope: &ropey::Rope, eol: LineEnding) -> Vec<u8> {
    let extra = match eol {
        LineEnding::Crlf => rope.len_lines(),
        _ => 0,
    };
    let mut out = Vec::with_capacity(rope.len_bytes().saturating_add(extra));
    // Whether the last byte written came from a `\r`, carried across chunk
    // boundaries so a CR LF split between two chunks is still one pair.
    let mut prev_cr = false;
    for chunk in rope.chunks() {
        let bytes = chunk.as_bytes();
        if eol == LineEnding::Lf {
            out.extend_from_slice(bytes);
        } else {
            let mut start = 0usize;
            for (i, _) in chunk.match_indices('\n') {
                let after_cr = match i.checked_sub(1) {
                    Some(p) => bytes.get(p) == Some(&b'\r'),
                    None => prev_cr,
                };
                if let Some(seg) = bytes.get(start..i) {
                    out.extend_from_slice(seg);
                }
                match eol {
                    LineEnding::Crlf => {
                        if !after_cr {
                            out.push(b'\r');
                        }
                        out.push(b'\n');
                    }
                    // CR LF collapses to the CR already written; a lone LF
                    // becomes CR.
                    _ => {
                        if !after_cr {
                            out.push(b'\r');
                        }
                    }
                }
                start = i.saturating_add(1);
            }
            if let Some(seg) = bytes.get(start..) {
                out.extend_from_slice(seg);
            }
        }
        if let Some(&last) = bytes.last() {
            prev_cr = last == b'\r';
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open + save without edits, the way `io.rs` does it.
    fn round_trip(src: &str) -> (LineEnding, String) {
        let eol = LineEnding::detect(src);
        let rope = ropey::Rope::from_str(&normalize_to_lf(src.to_string(), eol));
        let bytes = rope_to_bytes(&rope, eol);
        (eol, String::from_utf8(bytes).unwrap_or_default())
    }

    #[test]
    fn lf_file_is_detected_as_lf_on_every_platform() {
        // Regression: a pure-LF file fell through to `platform_default()`,
        // which is CRLF on Windows, so every LF file came back CRLF on save.
        assert_eq!(LineEnding::detect("a\nb\n"), LineEnding::Lf);
        let (_, out) = round_trip("a\nb\n");
        assert_eq!(out, "a\nb\n");
    }

    #[test]
    fn first_line_longer_than_4k_still_detects() {
        let src = format!("{}\r\nb\r\n", "x".repeat(10_000));
        assert_eq!(LineEnding::detect(&src), LineEnding::Crlf);
        let src = format!("{}\nb\n", "x".repeat(10_000));
        assert_eq!(LineEnding::detect(&src), LineEnding::Lf);
    }

    #[test]
    fn pure_files_keep_their_mode() {
        assert_eq!(LineEnding::detect("a\r\nb\r\n"), LineEnding::Crlf);
        assert_eq!(LineEnding::detect("a\rb\r"), LineEnding::Cr);
        assert_eq!(LineEnding::detect("a\nb"), LineEnding::Lf);
    }

    #[test]
    fn open_save_is_byte_identical_for_every_eol_shape() {
        let cases = [
            "",
            "no newline at all",
            "a\nb\n",
            "a\nb",
            "a\r\nb\r\n",
            "a\r\nb",
            "a\rb\r",
            "a\rb",
            "\n",
            "\r\n",
            "\r",
            "\n\n\r\n",
            // Mixed: CRLF first, then a lone LF.
            "a\r\nb\nc\r\n",
            // Mixed: CR first, then LF and CRLF.
            "a\rb\nc\r\nd",
            // Mixed: LF first, then CRLF and CR.
            "a\nb\r\nc\rd",
            // Lone CR inside a CRLF file.
            "a\r\nb\rc\r\n",
            // Doubled CR before LF (a file converted twice).
            "a\r\r\nb\r\r\n",
            "a\r\r\r\nb",
            // Multi-byte text around the breaks.
            "é\r\n中\n😀\r",
        ];
        for src in cases {
            let (eol, out) = round_trip(src);
            assert_eq!(out, src, "round trip changed {src:?} (detected {eol:?})");
        }
    }

    #[test]
    fn switching_a_mixed_buffer_to_crlf_does_not_double_cr() {
        // A mixed file loads in LF mode with its CRs kept in the rope. If
        // the user then picks Windows (CR LF), the CRLF pairs already in the
        // rope must not turn into CR CR LF.
        let src = "a\r\nb\nc\r\n";
        let eol = LineEnding::detect(src);
        let rope = ropey::Rope::from_str(&normalize_to_lf(src.to_string(), eol));
        let crlf = rope_to_bytes(&rope, LineEnding::Crlf);
        assert_eq!(crlf, b"a\r\nb\r\nc\r\n");
        let cr = rope_to_bytes(&rope, LineEnding::Cr);
        assert_eq!(cr, b"a\rb\rc\r");
    }

    #[test]
    fn crlf_split_across_rope_chunks() {
        // ropey keeps chunks under ~1 KiB; build a document long enough to
        // span many chunks with a CRLF near every boundary.
        let mut src = String::new();
        for i in 0..5000 {
            src.push_str(&"y".repeat(i % 7));
            src.push_str("\r\n");
        }
        let (eol, out) = round_trip(&src);
        assert_eq!(eol, LineEnding::Crlf);
        assert_eq!(out, src);
    }
}
