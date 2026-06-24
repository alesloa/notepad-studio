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

    /// Detect the dominant line ending in a freshly-loaded file.
    /// Scans up to the first 4 KiB so loading a large file stays cheap.
    pub(crate) fn detect(text: &str) -> Self {
        let scan_end = text
            .char_indices()
            .take_while(|(idx, _)| *idx < 4096)
            .last()
            .map(|(idx, c)| idx + c.len_utf8())
            .unwrap_or(0);
        let head = &text[..scan_end];
        if head.contains("\r\n") {
            Self::Crlf
        } else if head.contains('\r') {
            Self::Cr
        } else {
            Self::platform_default()
        }
    }
}

/// Convert any line ending in `text` to plain LF — Notepad Studio's rope is
/// always LF-internal, the EOL on disk is reapplied at save time.
pub(crate) fn normalize_to_lf(text: String, eol: LineEnding) -> String {
    match eol {
        LineEnding::Lf => text,
        LineEnding::Crlf => text.replace("\r\n", "\n"),
        LineEnding::Cr => text.replace('\r', "\n"),
    }
}

/// Serialize the LF-internal rope into the on-disk byte stream using the
/// buffer's chosen line ending.
pub(crate) fn rope_to_bytes(rope: &ropey::Rope, eol: LineEnding) -> Vec<u8> {
    let s = rope.to_string();
    match eol {
        LineEnding::Lf => s.into_bytes(),
        LineEnding::Crlf => s.replace('\n', "\r\n").into_bytes(),
        LineEnding::Cr => s.replace('\n', "\r").into_bytes(),
    }
}
