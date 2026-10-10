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

/// A file's bytes decoded into the rope's UTF-8, plus what was lost doing it.
pub(crate) struct Decoded {
    pub(crate) encoding: Encoding,
    pub(crate) text: String,
    /// True when malformed input was replaced with U+FFFD, so saving the
    /// text back would not reproduce the file's bytes. Only malformed
    /// UTF-16 (an odd trailing byte, an unpaired surrogate) can set this:
    /// every other input decodes losslessly. Notepad++ is lossy here too
    /// (`Utf16_Iter` in Utf8_16.cpp drops or re-encodes bad units); the
    /// open path reports it and opens the tab read-only rather than letting
    /// a save rewrite the file silently.
    pub(crate) lossy: bool,
}

/// Sniff a leading BOM and decode the entire byte stream into a UTF-8 String.
/// Returns the chosen encoding and the decoded text.
pub(crate) fn decode(raw: &[u8]) -> (Encoding, String) {
    let d = decode_bytes(raw);
    (d.encoding, d.text)
}

/// Decoding ladder:
/// 1. UTF-8 BOM and a valid UTF-8 body → UTF-8 BOM.
/// 2. UTF-16 BOM → UTF-16 LE / BE. The body is decoded without a second BOM
///    sniff, so a doubled BOM or a body that happens to start with EF BB BF
///    stays text.
/// 3. No BOM → strict UTF-8.
/// 4. Anything else → Latin-1 (Windows-1252) over the whole file, BOM bytes
///    included. Windows-1252 maps all 256 byte values to distinct
///    characters and back, so the file saves byte-identical; this is
///    Notepad++'s ANSI fallback. A UTF-8 BOM followed by invalid UTF-8 lands
///    here too: Notepad++ keeps those raw bytes in Scintilla, which a Rust
///    `String` can't hold, and replacing them would corrupt the file.
pub(crate) fn decode_bytes(raw: &[u8]) -> Decoded {
    if let Some(body) = raw.strip_prefix(BOM_UTF8) {
        if let Ok(s) = std::str::from_utf8(body) {
            return Decoded {
                encoding: Encoding::Utf8Bom,
                text: s.to_owned(),
                lossy: false,
            };
        }
        return decode_latin1(raw);
    }
    if let Some(body) = raw.strip_prefix(BOM_UTF16_LE) {
        let (cow, had_errors) = UTF_16LE.decode_without_bom_handling(body);
        return Decoded {
            encoding: Encoding::Utf16Le,
            text: cow.into_owned(),
            lossy: had_errors,
        };
    }
    if let Some(body) = raw.strip_prefix(BOM_UTF16_BE) {
        let (cow, had_errors) = UTF_16BE.decode_without_bom_handling(body);
        return Decoded {
            encoding: Encoding::Utf16Be,
            text: cow.into_owned(),
            lossy: had_errors,
        };
    }
    match std::str::from_utf8(raw) {
        Ok(s) => Decoded {
            encoding: Encoding::Utf8,
            text: s.to_owned(),
            lossy: false,
        },
        Err(_) => decode_latin1(raw),
    }
}

fn decode_latin1(raw: &[u8]) -> Decoded {
    let (cow, _) = WINDOWS_1252.decode_without_bom_handling(raw);
    Decoded {
        encoding: Encoding::Latin1,
        text: cow.into_owned(),
        lossy: false,
    }
}

/// Why `encode_checked` refused to produce bytes.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EncodeError {
    #[error(
        "'{ch}' (U+{code:04X}) can't be written as {label}; choose UTF-8 in the Encoding menu and save again",
        code = *ch as u32,
        label = encoding.label()
    )]
    Unmappable { ch: char, encoding: Encoding },
}

/// `encode` for the save path: refuses instead of substituting when `text`
/// holds a character the target encoding can't represent. Only Latin-1 can
/// fail; `encode` would write such a character as an `&#NNN;` reference,
/// which silently changes the document.
pub(crate) fn encode_checked(text: &str, encoding: Encoding) -> Result<Vec<u8>, EncodeError> {
    if encoding == Encoding::Latin1 {
        let (cow, _, had_unmappable) = WINDOWS_1252.encode(text);
        if had_unmappable {
            let mut buf = [0u8; 4];
            let ch = text
                .chars()
                .find(|c| WINDOWS_1252.encode(c.encode_utf8(&mut buf)).2)
                .unwrap_or(char::REPLACEMENT_CHARACTER);
            return Err(EncodeError::Unmappable { ch, encoding });
        }
        return Ok(cow.into_owned());
    }
    Ok(encode(text, encoding))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(raw: &[u8]) -> Vec<u8> {
        let (enc, text) = decode(raw);
        encode(&text, enc)
    }

    #[test]
    fn plain_encodings_round_trip() {
        let cases: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"hello\r\nworld".to_vec(),
            "héllo 中 😀".as_bytes().to_vec(),
            [BOM_UTF8, "héllo 😀".as_bytes()].concat(),
            BOM_UTF8.to_vec(),
            BOM_UTF16_LE.to_vec(),
            BOM_UTF16_BE.to_vec(),
            encode("a😀b\r\n", Encoding::Utf16Le),
            encode("a😀b\r\n", Encoding::Utf16Be),
            b"nul\0byte".to_vec(),
        ];
        for raw in cases {
            assert_eq!(round_trip(&raw), raw, "round trip changed {raw:02X?}");
        }
    }

    #[test]
    fn every_byte_survives_the_latin1_fallback() {
        // Invalid UTF-8 without a BOM falls back to Windows-1252, which must
        // map all 256 byte values to distinct characters and back.
        let raw: Vec<u8> = (0u8..=255).collect();
        let (enc, _) = decode(&raw);
        assert_eq!(enc, Encoding::Latin1);
        assert_eq!(round_trip(&raw), raw);
    }

    #[test]
    fn utf8_bom_with_invalid_bytes_is_not_corrupted() {
        // Regression: the body went through `from_utf8_lossy`, so a stray
        // Latin-1 byte after a UTF-8 BOM became U+FFFD and was written back
        // as EF BF BD on the next save.
        let raw = [BOM_UTF8, b"caf\xE9 ok"].concat();
        assert_eq!(round_trip(&raw), raw);
    }

    #[test]
    fn utf16_body_is_not_bom_sniffed_twice() {
        // Regression: `UTF_16LE.decode(body)` sniffs the body for a BOM
        // again, so a second FF FE was swallowed and a body starting with
        // EF BB BF was decoded as UTF-8.
        let doubled = [BOM_UTF16_LE, BOM_UTF16_LE, b"A\0"].concat();
        assert_eq!(round_trip(&doubled), doubled);
        let utf8_lookalike = [BOM_UTF16_LE, &[0xEF, 0xBB, 0xBF, 0x00]].concat();
        assert_eq!(round_trip(&utf8_lookalike), utf8_lookalike);
        let be = [BOM_UTF16_BE, BOM_UTF16_BE, b"\0A"].concat();
        assert_eq!(round_trip(&be), be);
    }

    #[test]
    fn malformed_utf16_is_reported() {
        // Odd trailing byte.
        let odd = [BOM_UTF16_LE, b"A\0B"].concat();
        let d = decode_bytes(&odd);
        assert_eq!(d.encoding, Encoding::Utf16Le);
        assert!(d.lossy);
        assert!(d.text.starts_with('A'));
        // Lone high surrogate followed by an ordinary unit.
        let lone = [BOM_UTF16_LE, &[0x3D, 0xD8, 0x41, 0x00]].concat();
        let d = decode_bytes(&lone);
        assert!(d.lossy);
        // A well-formed surrogate pair is not lossy.
        let pair = encode("😀", Encoding::Utf16Le);
        let d = decode_bytes(&pair);
        assert!(!d.lossy);
        assert_eq!(d.text, "😀");
    }

    #[test]
    fn unmappable_latin1_save_is_refused() {
        assert_eq!(
            encode_checked("café €", Encoding::Latin1).ok(),
            Some(b"caf\xE9 \x80".to_vec())
        );
        let err = encode_checked("a中b", Encoding::Latin1).err();
        assert!(matches!(err, Some(EncodeError::Unmappable { ch: '中', .. })));
        assert!(encode_checked("a中b", Encoding::Utf16Le).is_ok());
    }
}
