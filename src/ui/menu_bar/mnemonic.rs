// Win32 menu access keys. `&` marks the key, `&&` is a literal ampersand;
// PowerEditor/src/Notepad_plus.rc (IDR_M30_MENU) writes every label this way
// ("&File", "E&ncoding", "Se&ttings", "&?").

use std::ops::Range;

/// A menu label with its `&` markers resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Label {
    /// Display text, markers removed.
    pub(crate) text: String,
    /// Byte range in `text` of the access key, for the underline.
    pub(crate) underline: Option<Range<usize>>,
    /// The `&`-marked key, lowercased.
    pub(crate) key: Option<char>,
}

pub(crate) fn parse(label: &str) -> Label {
    let mut text = String::with_capacity(label.len());
    let mut underline = None;
    let mut key = None;
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('&') => text.push('&'),
            Some(next) => {
                // Win32 honours the first marker only.
                if key.is_none() {
                    let start = text.len();
                    underline = Some(start..start + next.len_utf8());
                    key = Some(lower(next));
                }
                text.push(next);
            }
            // A trailing `&` marks nothing.
            None => {}
        }
    }
    Label {
        text,
        underline,
        key,
    }
}

/// The key that selects `label` from the keyboard: its `&` key, else its
/// first letter or digit (Win32 matches the first character when an item
/// carries no marker).
pub(crate) fn access_key(label: &Label) -> Option<char> {
    label
        .key
        .or_else(|| label.text.chars().find(|c| c.is_alphanumeric()).map(lower))
}

/// Quote text that is not a label (a file path, a plugin or macro name) so
/// its `&` shows as written.
pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&&")
}

/// Label text with the markers removed: what macOS shows in its native menu
/// bar, which has no access keys.
pub(crate) fn strip(label: &str) -> String {
    parse(label).text
}

pub(crate) fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_the_key_and_drops_the_marker() {
        let l = parse("Se&ttings");
        assert_eq!(l.text, "Settings");
        assert_eq!(l.key, Some('t'));
        assert_eq!(l.underline, Some(2..3));
    }

    #[test]
    fn double_ampersand_is_literal() {
        let l = parse("Save && Close");
        assert_eq!(l.text, "Save & Close");
        assert_eq!(l.key, None);
        assert_eq!(l.underline, None);
    }

    #[test]
    fn literal_ampersand_before_the_marker_keeps_offsets_right() {
        let l = parse("A&&B &Go");
        assert_eq!(l.text, "A&B Go");
        assert_eq!(l.key, Some('g'));
        assert_eq!(&l.text[l.underline.clone().unwrap()], "G");
    }

    #[test]
    fn first_marker_wins_and_later_ones_are_dropped() {
        let l = parse("&One &Two");
        assert_eq!(l.text, "One Two");
        assert_eq!(l.key, Some('o'));
    }

    #[test]
    fn trailing_marker_is_dropped() {
        let l = parse("Odd&");
        assert_eq!(l.text, "Odd");
        assert_eq!(l.key, None);
    }

    #[test]
    fn question_mark_menu() {
        let l = parse("&?");
        assert_eq!(l.text, "?");
        assert_eq!(l.key, Some('?'));
    }

    #[test]
    fn multibyte_key_underlines_whole_char() {
        let l = parse("caf&é");
        assert_eq!(l.text, "café");
        assert_eq!(l.key, Some('é'));
        assert_eq!(&l.text[l.underline.clone().unwrap()], "é");
    }

    #[test]
    fn uppercase_key_is_lowercased() {
        assert_eq!(parse("E&ncoding").key, Some('n'));
        assert_eq!(parse("&File").key, Some('f'));
    }

    #[test]
    fn access_key_falls_back_to_first_alphanumeric() {
        assert_eq!(access_key(&parse("Open Recent")), Some('o'));
        assert_eq!(access_key(&parse("…  9 lives")), Some('9'));
        assert_eq!(access_key(&parse("--")), None);
        assert_eq!(access_key(&parse("Re&do")), Some('d'));
    }

    #[test]
    fn escape_round_trips_through_parse() {
        let raw = "C:\\R&D\\a&&b.txt";
        assert_eq!(parse(&escape(raw)).text, raw);
        assert_eq!(parse(&escape(raw)).key, None);
    }

    #[test]
    fn strip_matches_parse_text() {
        assert_eq!(strip("&Macro"), "Macro");
        assert_eq!(strip("Tom && Jerry"), "Tom & Jerry");
        assert_eq!(strip(""), "");
    }
}
