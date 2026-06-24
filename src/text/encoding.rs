// Per-buffer text encoding. The rope is always UTF-8 / LF-internal — encoding
// only affects how bytes are decoded on load and re-encoded on save.
//
// v1 trade-off: switching encoding via the menu / status bar updates the save
// target but does NOT re-decode the original raw bytes. If a user opened a
// Latin-1 file and then flips to UTF-16 LE, the next save writes the
// already-decoded text re-encoded as UTF-16 LE. We do not attempt to undo the
// initial decode.

use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1252};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Encoding {
    /// UTF-8 without a byte-order mark.
    Utf8,
    /// UTF-8 with a leading EF BB BF BOM.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Windows-1252 / ISO-8859-1 superset — what Notepad++ surfaces as
    /// "ANSI" on Windows builds and what we label "Latin-1" cross-platform.
    Latin1,
}

impl Encoding {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8-BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Latin1 => "Latin-1",
        }
    }

    /// Cycle order used by the status-bar click. UTF-8 → UTF-8 BOM →
    /// UTF-16 LE → UTF-16 BE → Latin-1 → UTF-8.
    pub(crate) fn cycle_next(self) -> Self {
        match self {
            Self::Utf8 => Self::Utf8Bom,
            Self::Utf8Bom => Self::Utf16Le,
            Self::Utf16Le => Self::Utf16Be,
            Self::Utf16Be => Self::Latin1,
            Self::Latin1 => Self::Utf8,
        }
    }
}

const BOM_UTF8: &[u8] = &[0xEF, 0xBB, 0xBF];
const BOM_UTF16_LE: &[u8] = &[0xFF, 0xFE];
const BOM_UTF16_BE: &[u8] = &[0xFE, 0xFF];

/// Sniff a leading BOM and decode the entire byte stream into a UTF-8 String.
/// Returns the chosen encoding and the decoded text.
///
/// Decoding ladder:
/// 1. BOM present → use it.
/// 2. No BOM → try strict UTF-8.
/// 3. UTF-8 had invalid bytes → fall back to Latin-1 (Windows-1252).
pub(crate) fn decode(raw: &[u8]) -> (Encoding, String) {
    if raw.starts_with(BOM_UTF8) {
        let body = &raw[BOM_UTF8.len()..];
        // The BOM guarantees UTF-8 intent; replace any subsequent invalid
        // bytes rather than silently downgrading to Latin-1.
        let text = String::from_utf8_lossy(body).into_owned();
        return (Encoding::Utf8Bom, text);
    }
    if raw.starts_with(BOM_UTF16_LE) {
        let body = &raw[BOM_UTF16_LE.len()..];
        let (cow, _, _) = UTF_16LE.decode(body);
        return (Encoding::Utf16Le, cow.into_owned());
    }
    if raw.starts_with(BOM_UTF16_BE) {
        let body = &raw[BOM_UTF16_BE.len()..];
        let (cow, _, _) = UTF_16BE.decode(body);
        return (Encoding::Utf16Be, cow.into_owned());
    }
    // No BOM. Strict UTF-8 first.
    match std::str::from_utf8(raw) {
        Ok(s) => (Encoding::Utf8, s.to_string()),
        Err(_) => {
            // Invalid UTF-8 bytes — fall back to Latin-1. Windows-1252 is a
            // superset of ISO-8859-1, decodes every possible byte without
            // error, and matches Notepad++'s ANSI fallback.
            let (cow, _, _) = WINDOWS_1252.decode(raw);
            (Encoding::Latin1, cow.into_owned())
        }
    }
}

/// Encode an in-memory UTF-8 string back into the on-disk byte stream for the
/// given encoding, prepending the BOM where the encoding requires one.
pub(crate) fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => {
            let mut out = Vec::with_capacity(BOM_UTF8.len() + text.len());
            out.extend_from_slice(BOM_UTF8);
            out.extend_from_slice(text.as_bytes());
            out
        }
        Encoding::Utf16Le => {
            let mut out = Vec::with_capacity(BOM_UTF16_LE.len() + text.len() * 2);
            out.extend_from_slice(BOM_UTF16_LE);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
        Encoding::Utf16Be => {
            let mut out = Vec::with_capacity(BOM_UTF16_BE.len() + text.len() * 2);
            out.extend_from_slice(BOM_UTF16_BE);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
            out
        }
        Encoding::Latin1 => {
            // encoding_rs maps unrepresentable code points to numeric NCRs
            // ("&#NNN;"). Acceptable for v1 — matches what most editors do
            // when forced to a narrower encoding.
            let (cow, _, _) = WINDOWS_1252.encode(text);
            cow.into_owned()
        }
    }
}
