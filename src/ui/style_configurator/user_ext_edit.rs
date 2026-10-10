// SPDX-License-Identifier: Apache-2.0
//
// Caret and edit steps for the Style Configurator's User-ext field, kept
// free of gpui so they can be tested headless.
//
// PowerEditor/src/WinControls/ColourPicker/WordStyleDlg.rc:43 — IDC_USER_EXT_EDIT
//
// The field edits a byte-indexed `String`. An extension list typed through
// an IME or loaded from a theme can hold multi-byte characters (CJK,
// emoji), so every step lands on a char boundary rather than moving by one
// byte.

/// Hard cap on the field's length in bytes. Way past anything NPP ships
/// (e.g. the C lexer's default is ~40 chars); stops a paste-bomb from
/// blowing up the field.
pub(super) const MAX_BYTES: usize = 256;

/// `i` clamped to `s` and moved back to the start of the character it
/// falls inside.
fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}

/// Start of the character before `i`.
pub(super) fn prev_boundary(s: &str, i: usize) -> usize {
    let i = floor_boundary(s, i);
    s.get(..i)
        .and_then(|head| head.char_indices().next_back())
        .map_or(0, |(at, _)| at)
}

/// End of the character at `i`.
pub(super) fn next_boundary(s: &str, i: usize) -> usize {
    let i = floor_boundary(s, i);
    s.get(i..)
        .and_then(|tail| tail.chars().next())
        .map_or(s.len(), |c| i.saturating_add(c.len_utf8()))
}

/// The selection `lo..hi` between `cursor` and `anchor`, both on char
/// boundaries.
pub(super) fn selection(s: &str, cursor: usize, anchor: usize) -> (usize, usize) {
    let cursor = floor_boundary(s, cursor);
    let anchor = floor_boundary(s, anchor);
    (cursor.min(anchor), cursor.max(anchor))
}

/// The selected text, or `None` when the selection is empty.
pub(super) fn selected_text(s: &str, cursor: usize, anchor: usize) -> Option<String> {
    let (lo, hi) = selection(s, cursor, anchor);
    if hi > lo { s.get(lo..hi).map(str::to_string) } else { None }
}

/// Backspace: delete the selection, or the character before the caret.
/// Returns whether the text changed.
pub(super) fn backspace(s: &mut String, cursor: &mut usize, anchor: &mut usize) -> bool {
    let (lo, hi) = selection(s, *cursor, *anchor);
    let (lo, hi) = if hi > lo {
        (lo, hi)
    } else if lo > 0 {
        (prev_boundary(s, lo), lo)
    } else {
        return false;
    };
    s.replace_range(lo..hi, "");
    *cursor = lo;
    *anchor = lo;
    true
}

/// Delete: delete the selection, or the character after the caret.
/// Returns whether the text changed.
pub(super) fn delete_forward(s: &mut String, cursor: &mut usize, anchor: &mut usize) -> bool {
    let (lo, hi) = selection(s, *cursor, *anchor);
    let (lo, hi) = if hi > lo {
        (lo, hi)
    } else if lo < s.len() {
        (lo, next_boundary(s, lo))
    } else {
        return false;
    };
    s.replace_range(lo..hi, "");
    *cursor = lo;
    *anchor = lo;
    true
}

/// Cut: remove the selection and return it. `None` when nothing is
/// selected.
pub(super) fn cut(s: &mut String, cursor: &mut usize, anchor: &mut usize) -> Option<String> {
    let removed = selected_text(s, *cursor, *anchor)?;
    let (lo, hi) = selection(s, *cursor, *anchor);
    s.replace_range(lo..hi, "");
    *cursor = lo;
    *anchor = lo;
    Some(removed)
}

/// Trim the field to at most `MAX_BYTES`, cutting before any character
/// that would straddle the cap, and pull the caret back inside it.
pub(super) fn cap_length(s: &mut String, cursor: &mut usize, anchor: &mut usize) {
    if s.len() > MAX_BYTES {
        s.truncate(floor_boundary(s, MAX_BYTES));
        *cursor = floor_boundary(s, *cursor);
        *anchor = floor_boundary(s, *anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two CJK characters and an emoji: 3 + 3 + 4 bytes.
    const MIXED: &str = "日本😀";

    #[test]
    fn caret_steps_over_whole_characters() {
        assert_eq!(next_boundary(MIXED, 0), 3);
        assert_eq!(next_boundary(MIXED, 6), 10);
        assert_eq!(next_boundary(MIXED, 10), 10);
        assert_eq!(prev_boundary(MIXED, 10), 6);
        assert_eq!(prev_boundary(MIXED, 3), 0);
        assert_eq!(prev_boundary(MIXED, 0), 0);
    }

    #[test]
    fn backspace_removes_a_whole_multibyte_character() {
        let mut s = MIXED.to_string();
        let (mut cursor, mut anchor) = (s.len(), s.len());
        assert!(backspace(&mut s, &mut cursor, &mut anchor));
        assert_eq!(s, "日本");
        assert_eq!((cursor, anchor), (6, 6));
        assert!(backspace(&mut s, &mut cursor, &mut anchor));
        assert!(backspace(&mut s, &mut cursor, &mut anchor));
        assert_eq!(s, "");
        assert!(!backspace(&mut s, &mut cursor, &mut anchor));
    }

    #[test]
    fn delete_removes_a_whole_multibyte_character() {
        let mut s = MIXED.to_string();
        let (mut cursor, mut anchor) = (3, 3);
        assert!(delete_forward(&mut s, &mut cursor, &mut anchor));
        assert_eq!(s, "日😀");
        assert!(delete_forward(&mut s, &mut cursor, &mut anchor));
        assert_eq!(s, "日");
        assert!(!delete_forward(&mut s, &mut cursor, &mut anchor));
    }

    #[test]
    fn selection_edits_handle_multibyte_text() {
        let mut s = MIXED.to_string();
        let (mut cursor, mut anchor) = (10, 3);
        assert_eq!(selected_text(&s, cursor, anchor).as_deref(), Some("本😀"));
        assert_eq!(cut(&mut s, &mut cursor, &mut anchor).as_deref(), Some("本😀"));
        assert_eq!(s, "日");
        assert_eq!((cursor, anchor), (3, 3));
        assert_eq!(cut(&mut s, &mut cursor, &mut anchor), None);
    }

    /// A caret left mid-character snaps back to the character's start
    /// instead of panicking a slice.
    #[test]
    fn off_boundary_caret_snaps_to_character_start() {
        assert_eq!(selected_text(MIXED, 1, 5).as_deref(), Some("日"));
        assert_eq!(next_boundary(MIXED, 1), 3);
        assert_eq!(prev_boundary(MIXED, 4), 0);
        let mut s = MIXED.to_string();
        let (mut cursor, mut anchor) = (4, 4);
        assert!(backspace(&mut s, &mut cursor, &mut anchor));
        assert_eq!(s, "本😀");
        assert_eq!((cursor, anchor), (0, 0));
    }

    #[test]
    fn empty_field_edits_are_no_ops() {
        let mut s = String::new();
        let (mut cursor, mut anchor) = (0, 0);
        assert!(!backspace(&mut s, &mut cursor, &mut anchor));
        assert!(!delete_forward(&mut s, &mut cursor, &mut anchor));
        assert_eq!(cut(&mut s, &mut cursor, &mut anchor), None);
        assert_eq!(next_boundary(&s, 0), 0);
        assert_eq!(prev_boundary(&s, 0), 0);
    }

    /// 85 three-byte characters are 255 bytes; one more crosses the cap
    /// mid-character.
    #[test]
    fn cap_cuts_on_a_character_boundary() {
        let mut s = "語".repeat(86);
        let (mut cursor, mut anchor) = (s.len(), 0);
        cap_length(&mut s, &mut cursor, &mut anchor);
        assert_eq!(s, "語".repeat(85));
        assert_eq!((cursor, anchor), (255, 0));
    }

    #[test]
    fn cap_leaves_short_text_alone() {
        let mut s = "cpp hpp".to_string();
        let (mut cursor, mut anchor) = (3, 3);
        cap_length(&mut s, &mut cursor, &mut anchor);
        assert_eq!(s, "cpp hpp");
        assert_eq!((cursor, anchor), (3, 3));
    }
}
