// PowerEditor/src/Notepad_plus.cpp:2034-2077, 2270-2298 - disk search uses decoded buffers.
// Keep physical line endings here: replacement must preserve untouched bytes.
use std::borrow::Cow;
use super::encoding::{decode_bytes, Encoding};

pub(crate) struct SearchFile<'a> {
    pub(crate) text: Cow<'a, str>,
    pub(crate) encoding: Encoding,
}

pub(crate) fn decode(raw: &[u8]) -> Result<SearchFile<'_>, String> {
    let limit = crate::find_in_files::FIND_IN_FILES_FILE_SIZE_LIMIT as usize;
    if raw.len() > limit { return Err("File exceeds the search size limit".into()); }
    if let Ok(text) = std::str::from_utf8(raw) {
        let (encoding, text) = match text.strip_prefix('\u{FEFF}') {
            Some(body) => (Encoding::Utf8Bom, body),
            None => (Encoding::Utf8, text),
        };
        return Ok(SearchFile { text: Cow::Borrowed(text), encoding });
    }
    let decoded = decode_bytes(raw);
    if decoded.lossy {
        return Err(format!("Malformed {} data; replacement would change the original bytes", decoded.encoding.label()));
    }
    let decoded_limit = limit.checked_mul(3).ok_or("Decoded file size overflow")?;
    if decoded.text.len() > decoded_limit || decoded.text.capacity() > decoded_limit {
        return Err("Decoded file exceeds the search size limit".into());
    }
    Ok(SearchFile { text: Cow::Owned(decoded.text), encoding: decoded.encoding })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::encoding::{encode_checked, Encoding};

    #[test]
    fn file_encoding_decoder_borrows_utf8_and_preserves_literal_second_bom() {
        let decoded = decode("\u{FEFF}\u{FEFF}caf\u{e9}\r\n".as_bytes()).unwrap();
        assert_eq!(decoded.encoding, Encoding::Utf8Bom);
        assert_eq!(decoded.text, "\u{FEFF}caf\u{e9}\r\n");
        assert!(matches!(decoded.text, Cow::Borrowed(_)));
    }

    #[test]
    fn file_encoding_decoder_keeps_invalid_utf8_bom_in_lossless_ansi_fallback() {
        let raw = b"\xef\xbb\xbfcaf\xe9 hit\r\n";
        let decoded = decode(raw).unwrap();
        assert_eq!(decoded.encoding, Encoding::Latin1);
        assert_eq!(encode_checked(&decoded.text, decoded.encoding).unwrap(), raw);
    }

    #[test]
    fn file_encoding_decoder_checks_raw_limit_and_utf16_loss() {
        assert!(decode(&vec![b'a'; crate::find_in_files::FIND_IN_FILES_FILE_SIZE_LIMIT as usize + 1]).is_err());
        assert!(decode(&[0xff, 0xfe, 0x3d, 0xd8]).is_err());
        let text = "\u{4e2d}\u{1f600}\r\n";
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
            assert_eq!(decode(&encode_checked(text, encoding).unwrap()).unwrap().text, text);
        }
    }
}
