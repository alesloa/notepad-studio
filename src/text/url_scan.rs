// URL detection over a single line of text.
//
// Matches the four schemes Notepad++ recognises for clickable links:
// `http://`, `https://`, `ftp://`, `file://`. A URL ends at the first
// whitespace, control byte, or end of input; a small set of trailing
// punctuation is trimmed so a URL that lives inside a sentence doesn't
// pull the period or closing bracket into the link.
//
// Pure byte-level scan — no regex dep, no allocation per match beyond
// the returned `Vec`. Returned ranges are byte offsets into the input.

use std::ops::Range;

const SCHEMES: &[&str] = &["https://", "http://", "ftp://", "file://"];

pub(crate) fn find_urls(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let mut scheme_len = 0usize;
        for s in SCHEMES {
            let b = s.as_bytes();
            if i + b.len() <= bytes.len() && &bytes[i..i + b.len()] == b {
                scheme_len = b.len();
                break;
            }
        }
        if scheme_len > 0 {
            let mut end = i + scheme_len;
            while end < bytes.len() {
                let b = bytes[end];
                if b <= 0x20 || b == 0x7F {
                    break;
                }
                end += 1;
            }
            while end > i + scheme_len
                && matches!(
                    bytes[end - 1],
                    b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}' | b'>' | b'"' | b'\''
                )
            {
                end -= 1;
            }
            if end > i + scheme_len {
                out.push(i..end);
                i = end;
                continue;
            }
        }
        i += utf8_step(bytes, i);
    }
    out
}

fn utf8_step(bytes: &[u8], i: usize) -> usize {
    let b = bytes[i];
    if b < 0x80 {
        1
    } else if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_https() {
        let r = find_urls("see https://example.com today");
        assert_eq!(r, vec![4..23]);
    }

    #[test]
    fn trims_trailing_punct() {
        let r = find_urls("visit https://example.com.");
        assert_eq!(r, vec![6..25]);
        let r2 = find_urls("see (https://example.com).");
        assert_eq!(r2, vec![5..24]);
    }

    #[test]
    fn multiple() {
        let r = find_urls("a https://x.io b http://y.io c");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn ftp_and_file() {
        let r = find_urls("ftp://srv/p file:///tmp/x");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn no_scheme_no_match() {
        let r = find_urls("example.com is not enough");
        assert!(r.is_empty());
    }

    #[test]
    fn bare_scheme_skipped() {
        let r = find_urls("https:// trailing");
        assert!(r.is_empty());
    }

    #[test]
    fn unicode_around_url() {
        let r = find_urls("→ https://例え.jp ←");
        assert_eq!(r.len(), 1);
    }
}
