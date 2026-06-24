// Notepad++ "Extended" search-mode escape interpretation.
//
// Used by both `workspace::find` (Find/Replace + Mark) and `find_in_files`.
// When Extended mode is on (and Regex is off), the user's literal find/replace
// strings are run through `interpret_extended` before matching/inserting.
//
// Supported escapes (matching Notepad++'s extended mode):
//   \n  → 0x0A line feed
//   \r  → 0x0D carriage return
//   \t  → 0x09 tab
//   \0  → 0x00 NUL
//   \b  → 0x08 backspace
//   \\  → literal backslash
//   \xHH    → byte from two hex digits (e.g. \x1B = ESC)
//   \dDDD   → byte from three decimal digits (0..=255)
//   \oOOO   → byte from three octal digits  (0..=255)
//
// Anything else after a `\` (including a trailing `\` at EOL) is passed
// through verbatim — Notepad++ behaves the same way.

pub(crate) fn interpret_extended(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c != b'\\' {
            // Multi-byte UTF-8 char outside an escape — copy whole codepoint.
            let rest = &input[i..];
            let ch = rest.chars().next().expect("non-empty");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let next = match bytes.get(i + 1) {
            Some(b) => *b,
            None => {
                out.push('\\');
                i += 1;
                continue;
            }
        };
        match next {
            b'n' => { out.push('\n'); i += 2; }
            b'r' => { out.push('\r'); i += 2; }
            b't' => { out.push('\t'); i += 2; }
            b'0' => { out.push('\0'); i += 2; }
            b'b' => { out.push('\u{08}'); i += 2; }
            b'\\' => { out.push('\\'); i += 2; }
            b'x' => {
                if let Some(byte) = parse_hex_byte(bytes, i + 2) {
                    out.push(byte as char);
                    i += 4;
                } else {
                    out.push('\\');
                    out.push('x');
                    i += 2;
                }
            }
            b'd' => {
                if let Some(byte) = parse_dec_byte(bytes, i + 2) {
                    out.push(byte as char);
                    i += 5;
                } else {
                    out.push('\\');
                    out.push('d');
                    i += 2;
                }
            }
            b'o' => {
                if let Some(byte) = parse_oct_byte(bytes, i + 2) {
                    out.push(byte as char);
                    i += 5;
                } else {
                    out.push('\\');
                    out.push('o');
                    i += 2;
                }
            }
            _ => {
                out.push('\\');
                out.push(next as char);
                i += 2;
            }
        }
    }
    out
}

fn parse_hex_byte(bytes: &[u8], at: usize) -> Option<u8> {
    let h1 = hex_digit(*bytes.get(at)?)?;
    let h2 = hex_digit(*bytes.get(at + 1)?)?;
    Some((h1 * 16 + h2) as u8)
}

fn parse_dec_byte(bytes: &[u8], at: usize) -> Option<u8> {
    let a = dec_digit(*bytes.get(at)?)?;
    let b = dec_digit(*bytes.get(at + 1)?)?;
    let c = dec_digit(*bytes.get(at + 2)?)?;
    let v = a * 100 + b * 10 + c;
    if v <= 255 { Some(v as u8) } else { None }
}

fn parse_oct_byte(bytes: &[u8], at: usize) -> Option<u8> {
    let a = oct_digit(*bytes.get(at)?)?;
    let b = oct_digit(*bytes.get(at + 1)?)?;
    let c = oct_digit(*bytes.get(at + 2)?)?;
    let v = a * 64 + b * 8 + c;
    if v <= 255 { Some(v as u8) } else { None }
}

fn hex_digit(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}

fn dec_digit(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        _ => None,
    }
}

fn oct_digit(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'7' => Some((b - b'0') as u32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::interpret_extended;

    #[test]
    fn passthrough_when_no_backslash() {
        assert_eq!(interpret_extended("hello world"), "hello world");
    }

    #[test]
    fn basic_escapes() {
        assert_eq!(interpret_extended(r"\n\r\t\0\\"), "\n\r\t\0\\");
    }

    #[test]
    fn backspace() {
        assert_eq!(interpret_extended(r"\b"), "\u{08}");
    }

    #[test]
    fn hex_byte() {
        assert_eq!(interpret_extended(r"\x1b"), "\u{1B}");
        assert_eq!(interpret_extended(r"\x1B"), "\u{1B}");
        assert_eq!(interpret_extended(r"\x41\x42"), "AB");
    }

    #[test]
    fn decimal_byte() {
        assert_eq!(interpret_extended(r"\d065"), "A");
        assert_eq!(interpret_extended(r"\d255"), "\u{ff}");
    }

    #[test]
    fn octal_byte() {
        assert_eq!(interpret_extended(r"\o101"), "A");
        assert_eq!(interpret_extended(r"\o033"), "\u{1B}");
    }

    #[test]
    fn unknown_escape_passes_through() {
        assert_eq!(interpret_extended(r"\q"), r"\q");
        assert_eq!(interpret_extended(r"\z"), r"\z");
    }

    #[test]
    fn trailing_backslash_passes_through() {
        assert_eq!(interpret_extended(r"foo\"), r"foo\");
    }

    #[test]
    fn malformed_hex_passes_through() {
        assert_eq!(interpret_extended(r"\xZ"), r"\xZ");
    }

    #[test]
    fn decimal_out_of_range_passes_through() {
        assert_eq!(interpret_extended(r"\d999"), r"\d999");
    }

    #[test]
    fn unicode_passthrough_around_escape() {
        assert_eq!(interpret_extended("héllo\\nworld"), "héllo\nworld");
    }
}
