// EntityInputHandler — IME / OS keyboard text routes through here.

// --- helpers for the find/replace/goto String inputs --------------------------------

use std::ops::Range;

use gpui::{
    Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window, point,
};

use crate::settings::{self, Settings};
use crate::tab::InputFocus;
use crate::text::utf16::{
    input_selection_range_utf16, rope_byte_to_utf16, rope_utf16_to_byte, slice_str_chars_utf16,
};
use crate::workspace::Workspace;

/// IME / typed text into one of the String-backed inputs. Wraps
/// [`crate::text::utf16::replace_input_utf16`], first pulling the stored
/// cursor and anchor onto char starts inside `target`: several openers
/// (Ctrl+F prefill, Go To reopen) replace the text but leave the anchor
/// stale, and the wrapped helper slices at those offsets.
fn replace_input_utf16(
    target: &mut String,
    cursor_field: &mut usize,
    anchor_field: &mut usize,
    byte_cursor: usize,
    byte_anchor: usize,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
) {
    let byte_cursor = super::edits::floor_char_boundary_in(target, byte_cursor);
    let byte_anchor = super::edits::floor_char_boundary_in(target, byte_anchor);
    crate::text::utf16::replace_input_utf16(
        target,
        cursor_field,
        anchor_field,
        byte_cursor,
        byte_anchor,
        range_utf16,
        new_text,
    );
}

/// NPP `Auto-Completion ▸ Auto-Insert` — what the matched-pair hook should do
/// in response to a single typed character. `None` means "fall through to the
/// normal insert path" (no pair flag matched, paste, multi-char input, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoPairAction {
    None,
    /// Closing-half typed and the next byte already holds it — advance the
    /// cursor instead of inserting a duplicate.
    SkipOver,
    /// Opening-half typed with no selection — insert `open` + `close`,
    /// position cursor between them.
    InsertPair { open: char, close: char },
    /// Opening-half typed with a selection — wrap the selection: replace
    /// `[ordered]` with `open + selected + close`, preserve inner selection.
    WrapSelection { open: char, close: char },
}

/// `typed` is the closer-half of an enabled pair (`)`, `]`, `}`, `>`, `'`,
/// `"`, or any user-defined close char). Combined with the next-char check in
/// `compute_auto_pair_action` to power the skip-over path.
fn is_pair_close(typed: char, snap: &Settings) -> bool {
    let builtin = match typed {
        ')' => snap.auto_insert_parens,
        ']' => snap.auto_insert_brackets,
        '}' => snap.auto_insert_braces,
        '>' => snap.auto_insert_html_xml_tag,
        '\'' => snap.auto_insert_quotes,
        '"' => snap.auto_insert_double_quotes,
        _ => false,
    };
    if builtin {
        return true;
    }
    snap.auto_insert_user_pairs
        .iter()
        .any(|(_open, close)| close.chars().next() == Some(typed))
}

/// `typed` is the opener-half of an enabled pair — return the matching
/// closer if so. Built-in pairs check the per-pair flag; user-defined pairs
/// don't have a flag (presence in the vec means "enabled").
fn pair_close_for(typed: char, snap: &Settings) -> Option<char> {
    let builtin = match typed {
        '(' if snap.auto_insert_parens => Some(')'),
        '[' if snap.auto_insert_brackets => Some(']'),
        '{' if snap.auto_insert_braces => Some('}'),
        '<' if snap.auto_insert_html_xml_tag => Some('>'),
        '\'' if snap.auto_insert_quotes => Some('\''),
        '"' if snap.auto_insert_double_quotes => Some('"'),
        _ => None,
    };
    if builtin.is_some() {
        return builtin;
    }
    snap.auto_insert_user_pairs.iter().find_map(|(open, close)| {
        let o = open.chars().next()?;
        let c = close.chars().next()?;
        (o == typed).then_some(c)
    })
}

/// Decide what to do for a single typed character. Pure — no editor state
/// touched, just the typed char + selection emptiness + the byte right after
/// the cursor + the current Settings snapshot. Skip-over wins over
/// insert-pair when both apply (e.g. `'` / `"` are their own closer), so
/// typing into an existing auto-pair advances instead of nesting.
fn compute_auto_pair_action(
    typed: char,
    sel_empty: bool,
    next_char: Option<char>,
    snap: &Settings,
) -> AutoPairAction {
    if sel_empty && next_char == Some(typed) && is_pair_close(typed, snap) {
        return AutoPairAction::SkipOver;
    }
    if let Some(close) = pair_close_for(typed, snap) {
        return if sel_empty {
            AutoPairAction::InsertPair { open: typed, close }
        } else {
            AutoPairAction::WrapSelection { open: typed, close }
        };
    }
    AutoPairAction::None
}

/// Returns the single character if `s` is exactly one Unicode scalar — used
/// to gate matched-pair behaviour on real keystrokes (paste / IME blocks
/// fall through with `None`).
fn single_char(s: &str) -> Option<char> {
    let mut iter = s.chars();
    let first = iter.next()?;
    if iter.next().is_some() {
        return None;
    }
    Some(first)
}

/// The char starting at byte `cursor`, or `None` at the end of the buffer.
/// Feeds the auto-pair skip-over check.
fn char_at_byte(rope: &ropey::Rope, cursor: usize) -> Option<char> {
    let cursor = super::edits::floor_rope_char_boundary(rope, cursor);
    rope.get_char(rope.byte_to_char(cursor))
}

/// Byte range for a UTF-16 range the OS handed to `replace_text_in_range`.
/// The OS can send offsets past the end, a reversed range, or one that
/// splits a surrogate pair; the result is clamped, ordered and on char
/// boundaries either way.
fn utf16_range_to_bytes(rope: &ropey::Rope, range_utf16: Range<usize>) -> Range<usize> {
    let a = super::edits::floor_rope_char_boundary(rope, rope_utf16_to_byte(rope, range_utf16.start));
    let b = super::edits::floor_rope_char_boundary(rope, rope_utf16_to_byte(rope, range_utf16.end));
    a.min(b)..a.max(b)
}

/// Line-local byte span of `byte_start..byte_end` on a laid-out line that
/// starts at `line_start` and is `line_len` bytes long, or `None` when the
/// range doesn't start on this line. Drives the IME candidate-window rect.
fn local_span_on_line(
    line_start: usize,
    line_len: usize,
    byte_start: usize,
    byte_end: usize,
) -> Option<(usize, usize)> {
    let (byte_start, byte_end) = (byte_start.min(byte_end), byte_start.max(byte_end));
    let line_end = line_start.saturating_add(line_len);
    if byte_start >= line_start && byte_start <= line_end {
        let local_start = byte_start - line_start;
        let local_end = (byte_end - line_start).min(line_len);
        return Some((local_start, local_end));
    }
    None
}

/// NPP-style C0-control bytes blocked when `prevent_c0_input` is on:
/// 0x00..=0x08, 0x0E..=0x1F, 0x7F. Tab (0x09), LF (0x0A), CR (0x0D) pass
/// through.
#[inline]
fn is_c0_control(b: u8) -> bool {
    matches!(b, 0x00..=0x08 | 0x0E..=0x1F | 0x7F)
}

fn strip_c0_controls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        let blocked = matches!(cp, 0x00..=0x08 | 0x0E..=0x1F | 0x7F);
        if !blocked {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{is_c0_control, strip_c0_controls};

    #[test]
    fn c0_control_predicate() {
        for b in 0x00u8..=0x08 {
            assert!(is_c0_control(b), "{b:#x} should be blocked");
        }
        for b in 0x0Eu8..=0x1F {
            assert!(is_c0_control(b), "{b:#x} should be blocked");
        }
        assert!(is_c0_control(0x7F));
        // Whitelisted formatting bytes
        assert!(!is_c0_control(b'\t'));
        assert!(!is_c0_control(b'\n'));
        assert!(!is_c0_control(b'\r'));
        // Printable ASCII
        assert!(!is_c0_control(b'a'));
        assert!(!is_c0_control(b' '));
    }

    #[test]
    fn strip_keeps_tab_lf_cr_and_text() {
        assert_eq!(strip_c0_controls("hello\tworld\nfoo\r\n"), "hello\tworld\nfoo\r\n");
    }

    #[test]
    fn strip_removes_c0_codes() {
        let raw = "a\x00b\x01c\x07d\x0Ee\x1Ff\x7Fg";
        assert_eq!(strip_c0_controls(raw), "abcdefg");
    }

    #[test]
    fn strip_preserves_unicode_above_c0() {
        // Multi-byte UTF-8 chars share bytes in the 0x80..=0xBF range; the
        // filter is char-based, not byte-based, so they survive intact.
        assert_eq!(strip_c0_controls("éπ漢"), "éπ漢");
    }

    #[test]
    fn strip_returns_empty_when_only_c0() {
        assert_eq!(strip_c0_controls("\x00\x01\x02"), "");
    }

    // --- Matched-pair helpers ----------------------------------------------

    use super::{
        AutoPairAction, compute_auto_pair_action, is_pair_close, pair_close_for, single_char,
    };
    use crate::settings::Settings;

    /// Default settings with all auto-insert flags off — start point for
    /// the matched-pair tests so each test only flips the flags it needs.
    fn flags_off() -> Settings {
        let mut s = Settings::default();
        s.auto_insert_parens = false;
        s.auto_insert_brackets = false;
        s.auto_insert_braces = false;
        s.auto_insert_quotes = false;
        s.auto_insert_double_quotes = false;
        s.auto_insert_html_xml_tag = false;
        s.auto_insert_user_pairs = Vec::new();
        s
    }

    #[test]
    fn pair_close_for_respects_flags() {
        let mut s = flags_off();
        assert_eq!(pair_close_for('(', &s), None);
        s.auto_insert_parens = true;
        assert_eq!(pair_close_for('(', &s), Some(')'));
    }

    #[test]
    fn pair_close_for_user_pairs() {
        let mut s = flags_off();
        s.auto_insert_user_pairs = vec![("«".into(), "»".into())];
        assert_eq!(pair_close_for('«', &s), Some('»'));
        assert_eq!(pair_close_for('»', &s), None);
    }

    #[test]
    fn is_pair_close_recognises_closers() {
        let mut s = flags_off();
        s.auto_insert_parens = true;
        assert!(is_pair_close(')', &s));
        assert!(!is_pair_close(']', &s));
    }

    #[test]
    fn opener_no_selection_inserts_pair() {
        let mut s = flags_off();
        s.auto_insert_parens = true;
        let action = compute_auto_pair_action('(', true, Some('a'), &s);
        assert_eq!(
            action,
            AutoPairAction::InsertPair { open: '(', close: ')' }
        );
    }

    #[test]
    fn opener_disabled_falls_through() {
        let s = flags_off(); // parens off
        assert_eq!(
            compute_auto_pair_action('(', true, None, &s),
            AutoPairAction::None
        );
    }

    #[test]
    fn opener_with_selection_wraps() {
        let mut s = flags_off();
        s.auto_insert_double_quotes = true;
        let action = compute_auto_pair_action('"', false, None, &s);
        assert_eq!(
            action,
            AutoPairAction::WrapSelection { open: '"', close: '"' }
        );
    }

    #[test]
    fn closer_over_existing_closer_skips() {
        let mut s = flags_off();
        s.auto_insert_parens = true;
        assert_eq!(
            compute_auto_pair_action(')', true, Some(')'), &s),
            AutoPairAction::SkipOver
        );
    }

    #[test]
    fn closer_without_match_passes_through() {
        let mut s = flags_off();
        s.auto_insert_parens = true;
        // Cursor at end of buffer (next_char = None) and typing `)` — fall
        // through to normal insert; nothing to skip over.
        assert_eq!(
            compute_auto_pair_action(')', true, None, &s),
            AutoPairAction::None
        );
        // Or next char is unrelated.
        assert_eq!(
            compute_auto_pair_action(')', true, Some('a'), &s),
            AutoPairAction::None
        );
    }

    #[test]
    fn quote_skips_over_existing_quote() {
        let mut s = flags_off();
        s.auto_insert_quotes = true;
        // Typing `'` right before an existing `'` (the closer of a previous
        // auto-pair) — advance the cursor instead of nesting.
        assert_eq!(
            compute_auto_pair_action('\'', true, Some('\''), &s),
            AutoPairAction::SkipOver
        );
    }

    #[test]
    fn quote_at_end_of_buffer_inserts_pair() {
        let mut s = flags_off();
        s.auto_insert_quotes = true;
        assert_eq!(
            compute_auto_pair_action('\'', true, None, &s),
            AutoPairAction::InsertPair { open: '\'', close: '\'' }
        );
    }

    #[test]
    fn user_pair_skips_over_close_char() {
        let mut s = flags_off();
        s.auto_insert_user_pairs = vec![("«".into(), "»".into())];
        assert_eq!(
            compute_auto_pair_action('»', true, Some('»'), &s),
            AutoPairAction::SkipOver
        );
        assert_eq!(
            compute_auto_pair_action('«', true, None, &s),
            AutoPairAction::InsertPair { open: '«', close: '»' }
        );
    }

    #[test]
    fn single_char_filters_multi_char_input() {
        assert_eq!(single_char(""), None);
        assert_eq!(single_char("a"), Some('a'));
        assert_eq!(single_char("ab"), None);
        assert_eq!(single_char("漢"), Some('漢'));
    }

    // --- Offset hardening for OS / IME supplied positions ------------------

    use super::{char_at_byte, local_span_on_line, utf16_range_to_bytes};
    use ropey::Rope;

    fn on_char_boundary(rope: &Rope, b: usize) -> bool {
        b <= rope.len_bytes() && rope.char_to_byte(rope.byte_to_char(b)) == b
    }

    #[test]
    fn char_at_byte_handles_eof_and_mid_char() {
        let rope = Rope::from_str("aé");
        assert_eq!(char_at_byte(&rope, 0), Some('a'));
        assert_eq!(char_at_byte(&rope, 1), Some('é'));
        assert_eq!(char_at_byte(&rope, 3), None);
        // Mid-char and past-EOF carets used to panic in `byte_slice`.
        assert_eq!(char_at_byte(&rope, 2), Some('é'));
        assert_eq!(char_at_byte(&rope, 99), None);
        assert_eq!(char_at_byte(&Rope::new(), 0), None);
    }

    #[test]
    fn utf16_ranges_from_the_os_are_clamped_and_ordered() {
        // a(1 unit) 👍(2 units, a surrogate pair) b(1 unit) \r\n 中
        let rope = Rope::from_str("a👍b\r\n中");
        let units = 1 + 2 + 1 + 2 + 1;
        for start in 0..=units + 3 {
            for end in 0..=units + 3 {
                let r = utf16_range_to_bytes(&rope, start..end);
                assert!(r.start <= r.end, "{start}..{end} -> {r:?}");
                assert!(on_char_boundary(&rope, r.start), "{start}..{end} -> {r:?}");
                assert!(on_char_boundary(&rope, r.end), "{start}..{end} -> {r:?}");
            }
        }
        // A range that splits the surrogate pair rounds out to whole chars.
        assert_eq!(utf16_range_to_bytes(&rope, 2..2), 5..5);
        // Reversed ranges are reordered rather than turned into a pure insert.
        assert_eq!(utf16_range_to_bytes(&rope, 4..1), 1..6);
        assert_eq!(utf16_range_to_bytes(&rope, 50..60), 11..11);
    }

    #[test]
    fn ime_bounds_span_handles_reversed_and_overlong_ranges() {
        assert_eq!(local_span_on_line(10, 5, 12, 14), Some((2, 4)));
        assert_eq!(local_span_on_line(10, 5, 12, 99), Some((2, 5)));
        assert_eq!(local_span_on_line(10, 5, 3, 4), None);
        // byte_end before the line start used to underflow.
        assert_eq!(local_span_on_line(10, 5, 12, 8), None);
        assert_eq!(local_span_on_line(0, 9, 12, 8), Some((8, 9)));
        assert_eq!(local_span_on_line(usize::MAX - 1, 5, usize::MAX, usize::MAX), Some((1, 1)));
    }

    use super::replace_input_utf16;

    #[test]
    fn input_ime_replace_with_stale_carets_does_not_panic() {
        // Ctrl+F prefill sets the query to "é" and the cursor to 2 but
        // leaves the anchor at 1, inside the char.
        let (mut t, mut c, mut a) = (String::from("é"), 2usize, 1usize);
        replace_input_utf16(&mut t, &mut c, &mut a, 2, 1, None, "x");
        assert!(t.is_char_boundary(c) && c == a && c <= t.len(), "{t:?} {c}");

        for base in ["", "abc", "é", "中文", "a👍b", "👨‍👩‍👧", "🇫🇷"] {
            for cur in 0..=base.len() + 2 {
                for anc in 0..=base.len() + 2 {
                    for range in [None, Some(0..1), Some(3..1), Some(1..99)] {
                        let (mut t, mut c, mut a) = (base.to_string(), cur, anc);
                        replace_input_utf16(&mut t, &mut c, &mut a, cur, anc, range.clone(), "é");
                        assert!(
                            c <= t.len() && t.is_char_boundary(c) && c == a,
                            "{base:?} cur={cur} anc={anc} {range:?} -> {t:?} {c}"
                        );
                    }
                }
            }
        }
    }
}

impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        match self.input_focus(window) {
            InputFocus::FindQuery => Some(slice_str_chars_utf16(
                &self.find_state.query,
                range_utf16,
                actual_range,
            )),
            InputFocus::Replacement => Some(slice_str_chars_utf16(
                &self.find_state.replacement,
                range_utf16,
                actual_range,
            )),
            InputFocus::Goto => Some(slice_str_chars_utf16(
                &self.goto_state.text,
                range_utf16,
                actual_range,
            )),
            InputFocus::FifQuery => Some(slice_str_chars_utf16(
                &self.find_in_files.query,
                range_utf16,
                actual_range,
            )),
            InputFocus::FifReplacement => Some(slice_str_chars_utf16(
                &self.find_in_files.replacement,
                range_utf16,
                actual_range,
            )),
            InputFocus::FifFilter => Some(slice_str_chars_utf16(
                &self.find_in_files.filter,
                range_utf16,
                actual_range,
            )),
            InputFocus::FifDirectory => Some(slice_str_chars_utf16(
                &self.find_in_files.directory_text,
                range_utf16,
                actual_range,
            )),
            InputFocus::MarkQuery => Some(slice_str_chars_utf16(
                &self.mark_state.query,
                range_utf16,
                actual_range,
            )),
            InputFocus::MacroPrompt => self.macro_prompt.as_ref().map(|s| {
                slice_str_chars_utf16(&s.text, range_utf16, actual_range)
            }),
            InputFocus::TabRename => self.tab_rename.as_ref().map(|s| {
                slice_str_chars_utf16(&s.text, range_utf16, actual_range)
            }),
            InputFocus::FolderInlineCreate => self.folder_inline_create.as_ref().map(|s| {
                slice_str_chars_utf16(&s.text, range_utf16, actual_range)
            }),
            InputFocus::RichDocLinkUrl => self.rich_doc_link_bar.as_ref().map(|s| {
                slice_str_chars_utf16(&s.text, range_utf16, actual_range)
            }),
            InputFocus::RichDocColorHex => self.rich_doc_color_bar.as_ref().map(|s| {
                slice_str_chars_utf16(&s.text, range_utf16, actual_range)
            }),
            InputFocus::RichDocBody => {
                let payload = self.tabs[self.active].buffer.rich_doc.as_ref()?;
                let sel = payload.selection?;
                if sel.anchor.block != sel.cursor.block {
                    return None;
                }
                let block_text = payload.block_text(sel.anchor.block);
                Some(slice_str_chars_utf16(&block_text, range_utf16, actual_range))
            }
            InputFocus::CsvCell => {
                let tab_id = self.tabs[self.active].session_id;
                let model = self.csv_models.get(&tab_id)?;
                let input = model.cell_input.as_ref()?;
                Some(slice_str_chars_utf16(
                    &input.buffer,
                    range_utf16,
                    actual_range,
                ))
            }
            InputFocus::CsvFindBar => {
                let tab_id = self.tabs[self.active].session_id;
                let model = self.csv_models.get(&tab_id)?;
                let input = model.find_input.as_ref()?;
                Some(slice_str_chars_utf16(
                    &input.buffer,
                    range_utf16,
                    actual_range,
                ))
            }
            InputFocus::CsvReplaceBar => {
                let tab_id = self.tabs[self.active].session_id;
                let model = self.csv_models.get(&tab_id)?;
                let input = model.replace_input.as_ref()?;
                Some(slice_str_chars_utf16(
                    &input.buffer,
                    range_utf16,
                    actual_range,
                ))
            }
            // Grid-focused (no cell edit) has no text content to slice.
            InputFocus::CsvGrid => None,
            InputFocus::Editor => {
                let rope = &self.buffer().rope;
                let total = rope.len_bytes();
                let byte_start = rope_utf16_to_byte(rope, range_utf16.start).min(total);
                let byte_end = rope_utf16_to_byte(rope, range_utf16.end).min(total);
                if byte_start > byte_end {
                    return None;
                }
                actual_range.replace(
                    rope_byte_to_utf16(rope, byte_start)
                        ..rope_byte_to_utf16(rope, byte_end),
                );
                Some(rope.byte_slice(byte_start..byte_end).to_string())
            }
        }
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        match self.input_focus(window) {
            InputFocus::FindQuery => Some(input_selection_range_utf16(
                &self.find_state.query,
                self.find_state.query_cursor,
                self.find_state.query_anchor,
            )),
            InputFocus::Replacement => Some(input_selection_range_utf16(
                &self.find_state.replacement,
                self.find_state.replacement_cursor,
                self.find_state.replacement_anchor,
            )),
            InputFocus::Goto => Some(input_selection_range_utf16(
                &self.goto_state.text,
                self.goto_state.text_cursor,
                self.goto_state.text_anchor,
            )),
            InputFocus::FifQuery => Some(input_selection_range_utf16(
                &self.find_in_files.query,
                self.find_in_files.query_cursor,
                self.find_in_files.query_anchor,
            )),
            InputFocus::FifReplacement => Some(input_selection_range_utf16(
                &self.find_in_files.replacement,
                self.find_in_files.replacement_cursor,
                self.find_in_files.replacement_anchor,
            )),
            InputFocus::FifFilter => Some(input_selection_range_utf16(
                &self.find_in_files.filter,
                self.find_in_files.filter_cursor,
                self.find_in_files.filter_anchor,
            )),
            InputFocus::FifDirectory => Some(input_selection_range_utf16(
                &self.find_in_files.directory_text,
                self.find_in_files.directory_text_cursor,
                self.find_in_files.directory_text_anchor,
            )),
            InputFocus::MarkQuery => Some(input_selection_range_utf16(
                &self.mark_state.query,
                self.mark_state.query_cursor,
                self.mark_state.query_anchor,
            )),
            InputFocus::MacroPrompt => self.macro_prompt.as_ref().map(|s| {
                input_selection_range_utf16(&s.text, s.text_cursor, s.text_anchor)
            }),
            InputFocus::TabRename => self.tab_rename.as_ref().map(|s| {
                input_selection_range_utf16(&s.text, s.text_cursor, s.text_anchor)
            }),
            InputFocus::FolderInlineCreate => self.folder_inline_create.as_ref().map(|s| {
                input_selection_range_utf16(&s.text, s.text_cursor, s.text_anchor)
            }),
            InputFocus::RichDocLinkUrl => self.rich_doc_link_bar.as_ref().map(|s| {
                input_selection_range_utf16(&s.text, s.text_cursor, s.text_anchor)
            }),
            InputFocus::RichDocColorHex => self.rich_doc_color_bar.as_ref().map(|s| {
                input_selection_range_utf16(&s.text, s.text_cursor, s.text_anchor)
            }),
            InputFocus::RichDocBody => {
                let payload = self.tabs[self.active].buffer.rich_doc.as_ref()?;
                let sel = payload.selection?;
                if sel.anchor.block != sel.cursor.block {
                    return None;
                }
                let block_text = payload.block_text(sel.anchor.block);
                let cursor_byte = payload.flat_byte_for(sel.cursor);
                let anchor_byte = payload.flat_byte_for(sel.anchor);
                Some(input_selection_range_utf16(
                    &block_text,
                    cursor_byte,
                    anchor_byte,
                ))
            }
            InputFocus::CsvCell => {
                let tab_id = self.tabs[self.active].session_id;
                let model = self.csv_models.get(&tab_id)?;
                let input = model.cell_input.as_ref()?;
                Some(input_selection_range_utf16(
                    &input.buffer,
                    input.cursor,
                    input.anchor,
                ))
            }
            InputFocus::CsvFindBar => {
                let tab_id = self.tabs[self.active].session_id;
                let model = self.csv_models.get(&tab_id)?;
                let input = model.find_input.as_ref()?;
                Some(input_selection_range_utf16(
                    &input.buffer,
                    input.cursor,
                    input.anchor,
                ))
            }
            InputFocus::CsvReplaceBar => {
                let tab_id = self.tabs[self.active].session_id;
                let model = self.csv_models.get(&tab_id)?;
                let input = model.replace_input.as_ref()?;
                Some(input_selection_range_utf16(
                    &input.buffer,
                    input.cursor,
                    input.anchor,
                ))
            }
            InputFocus::CsvGrid => None,
            InputFocus::Editor => {
                let sel = self.selection();
                let rope = &self.buffer().rope;
                let r = sel.ordered();
                let start_u16 = rope_byte_to_utf16(rope, r.start);
                let end_u16 = rope_byte_to_utf16(rope, r.end);
                Some(UTF16Selection {
                    range: start_u16..end_u16,
                    reversed: sel.head < sel.anchor,
                })
            }
        }
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.input_focus(window) {
            InputFocus::FindQuery => {
                let cursor = self.find_state.query_cursor;
                let anchor = self.find_state.query_anchor;
                replace_input_utf16(
                    &mut self.find_state.query,
                    &mut self.find_state.query_cursor,
                    &mut self.find_state.query_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                self.refresh_matches();
                cx.notify();
            }
            InputFocus::Replacement => {
                let cursor = self.find_state.replacement_cursor;
                let anchor = self.find_state.replacement_anchor;
                replace_input_utf16(
                    &mut self.find_state.replacement,
                    &mut self.find_state.replacement_cursor,
                    &mut self.find_state.replacement_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::Goto => {
                let cursor = self.goto_state.text_cursor;
                let anchor = self.goto_state.text_anchor;
                replace_input_utf16(
                    &mut self.goto_state.text,
                    &mut self.goto_state.text_cursor,
                    &mut self.goto_state.text_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::FifQuery => {
                let cursor = self.find_in_files.query_cursor;
                let anchor = self.find_in_files.query_anchor;
                replace_input_utf16(
                    &mut self.find_in_files.query,
                    &mut self.find_in_files.query_cursor,
                    &mut self.find_in_files.query_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::FifReplacement => {
                let cursor = self.find_in_files.replacement_cursor;
                let anchor = self.find_in_files.replacement_anchor;
                replace_input_utf16(
                    &mut self.find_in_files.replacement,
                    &mut self.find_in_files.replacement_cursor,
                    &mut self.find_in_files.replacement_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::FifFilter => {
                let cursor = self.find_in_files.filter_cursor;
                let anchor = self.find_in_files.filter_anchor;
                replace_input_utf16(
                    &mut self.find_in_files.filter,
                    &mut self.find_in_files.filter_cursor,
                    &mut self.find_in_files.filter_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::FifDirectory => {
                let cursor = self.find_in_files.directory_text_cursor;
                let anchor = self.find_in_files.directory_text_anchor;
                replace_input_utf16(
                    &mut self.find_in_files.directory_text,
                    &mut self.find_in_files.directory_text_cursor,
                    &mut self.find_in_files.directory_text_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::MarkQuery => {
                let cursor = self.mark_state.query_cursor;
                let anchor = self.mark_state.query_anchor;
                replace_input_utf16(
                    &mut self.mark_state.query,
                    &mut self.mark_state.query_cursor,
                    &mut self.mark_state.query_anchor,
                    cursor,
                    anchor,
                    range_utf16,
                    new_text,
                );
                cx.notify();
            }
            InputFocus::MacroPrompt => {
                if let Some(state) = self.macro_prompt.as_mut() {
                    let cursor = state.text_cursor;
                    let anchor = state.text_anchor;
                    replace_input_utf16(
                        &mut state.text,
                        &mut state.text_cursor,
                        &mut state.text_anchor,
                        cursor,
                        anchor,
                        range_utf16,
                        new_text,
                    );
                    cx.notify();
                }
            }
            InputFocus::TabRename => {
                if let Some(state) = self.tab_rename.as_mut() {
                    let cursor = state.text_cursor;
                    let anchor = state.text_anchor;
                    replace_input_utf16(
                        &mut state.text,
                        &mut state.text_cursor,
                        &mut state.text_anchor,
                        cursor,
                        anchor,
                        range_utf16,
                        new_text,
                    );
                    cx.notify();
                }
            }
            InputFocus::FolderInlineCreate => {
                if let Some(state) = self.folder_inline_create.as_mut() {
                    // Block path separators and the null byte at the input
                    // level — same set the on-commit validator checks, but
                    // catching them on keystroke prevents the user typing
                    // a path that will only fail at Enter time.
                    let filtered: String = new_text
                        .chars()
                        .filter(|c| *c != '/' && *c != '\\' && *c != '\0')
                        .collect();
                    if filtered.is_empty() && !new_text.is_empty() {
                        return;
                    }
                    let cursor = state.text_cursor;
                    let anchor = state.text_anchor;
                    replace_input_utf16(
                        &mut state.text,
                        &mut state.text_cursor,
                        &mut state.text_anchor,
                        cursor,
                        anchor,
                        range_utf16,
                        &filtered,
                    );
                    // Typing clears the last validation error so the user
                    // gets fresh feedback on the next Enter.
                    state.error = None;
                    cx.notify();
                }
            }
            InputFocus::RichDocLinkUrl => {
                if let Some(state) = self.rich_doc_link_bar.as_mut() {
                    let cursor = state.text_cursor;
                    let anchor = state.text_anchor;
                    replace_input_utf16(
                        &mut state.text,
                        &mut state.text_cursor,
                        &mut state.text_anchor,
                        cursor,
                        anchor,
                        range_utf16,
                        new_text,
                    );
                    cx.notify();
                }
            }
            InputFocus::RichDocColorHex => {
                // Filter to hex digits only (uppercase). Anything else is
                // dropped — same policy as the StyleConfigurator hex input
                // so paste of `"#ff00aa"` stays usable (the `#` filters out).
                let filtered: String = new_text
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .map(|c| c.to_ascii_uppercase())
                    .collect();
                if filtered.is_empty() && !new_text.is_empty() {
                    return;
                }
                if let Some(state) = self.rich_doc_color_bar.as_mut() {
                    let cursor = state.text_cursor;
                    let anchor = state.text_anchor;
                    replace_input_utf16(
                        &mut state.text,
                        &mut state.text_cursor,
                        &mut state.text_anchor,
                        cursor,
                        anchor,
                        range_utf16,
                        &filtered,
                    );
                    // Hard cap at 6 hex chars. If paste-of-larger-string
                    // lands, truncate from the end and clamp the caret.
                    if state.text.len() > 6 {
                        state.text.truncate(6);
                        state.text_cursor = state.text_cursor.min(6);
                        state.text_anchor = state.text_anchor.min(6);
                    }
                }
                // Sync HSV thumbs from the just-typed text so the More-colours
                // popover's SV / hue surfaces track. sync_from_buffer no-ops
                // on partial buffers (<6 chars don't parse), so the thumbs
                // only move once the user finishes typing a valid hex.
                crate::ui::style_configurator::hsv_surface::sync_from_buffer(self);
                self.apply_rich_doc_color_buffer(cx);
                cx.notify();
            }
            InputFocus::RichDocBody => {
                // Strip C0 controls. Newlines (\n / \r) are kept and become
                // paragraph splits via the segmented insert below — Phase 2g.B
                // supports multi-line paste / Enter as block boundaries.
                let cleaned: String = new_text
                    .chars()
                    .filter(|c| !is_c0_control(*c as u8))
                    .collect();
                if cleaned.is_empty() {
                    return;
                }
                // Normalize \r\n / \r to \n, then split on \n. Each segment
                // is inserted in order; between segments we issue a paragraph
                // break so a pasted "a\nb\nc" turns into three paragraphs.
                let normalized = cleaned.replace("\r\n", "\n").replace('\r', "\n");
                let mut first = true;
                for segment in normalized.split('\n') {
                    if !first {
                        self.rich_doc_insert_paragraph_break(cx);
                    }
                    if !segment.is_empty() {
                        self.rich_doc_insert_text(segment, cx);
                    }
                    first = false;
                }
            }
            InputFocus::CsvCell => {
                // Cell editor is a single-line free-text field. Strip
                // newlines / tabs (these are navigation keys at the
                // grid level, not literal characters) and route the rest
                // through the per-cell utf16 helper.
                let filtered: String = new_text
                    .chars()
                    .filter(|c| *c != '\n' && *c != '\r' && *c != '\t')
                    .collect();
                if filtered.is_empty() && !new_text.is_empty() {
                    return;
                }
                let tab_id = self.tabs[self.active].session_id;
                if let Some(model) = self.csv_models.get_mut(&tab_id) {
                    if let Some(input) = model.cell_input.as_mut() {
                        let cursor = input.cursor;
                        let anchor = input.anchor;
                        replace_input_utf16(
                            &mut input.buffer,
                            &mut input.cursor,
                            &mut input.anchor,
                            cursor,
                            anchor,
                            range_utf16,
                            &filtered,
                        );
                        cx.notify();
                    }
                }
            }
            InputFocus::CsvFindBar => {
                // Find-bar query input — strip newlines / tabs so typed
                // text always stays on a single line, then route through
                // the utf16 helper into model.find_input. After the
                // splice, recompute matches and jump to the first hit.
                let filtered: String = new_text
                    .chars()
                    .filter(|c| *c != '\n' && *c != '\r' && *c != '\t')
                    .collect();
                if filtered.is_empty() && !new_text.is_empty() {
                    return;
                }
                let tab_id = self.tabs[self.active].session_id;
                let mut new_query: Option<String> = None;
                if let Some(model) = self.csv_models.get_mut(&tab_id) {
                    if let Some(input) = model.find_input.as_mut() {
                        let cursor = input.cursor;
                        let anchor = input.anchor;
                        replace_input_utf16(
                            &mut input.buffer,
                            &mut input.cursor,
                            &mut input.anchor,
                            cursor,
                            anchor,
                            range_utf16,
                            &filtered,
                        );
                        new_query = Some(input.buffer.clone());
                    }
                }
                if let Some(q) = new_query {
                    if let Some(model) = self.csv_models.get_mut(&tab_id) {
                        model.update_find_query(q);
                        if let Some((r, c)) = model.find_current_match.and_then(|i| {
                            model.find_matches.get(i).copied()
                        }) {
                            // Header matches (row == usize::MAX) don't
                            // map to an editable active_cell — leave the
                            // active_cell alone for those.
                            if r != usize::MAX {
                                model.active_cell = Some((r, c));
                            }
                        }
                    }
                    cx.notify();
                }
            }
            InputFocus::CsvReplaceBar => {
                // Replace input — strip newlines/tabs (same as cell
                // editor) and persist the buffer back to model.replace_text
                // so close/reopen keeps the string.
                let filtered: String = new_text
                    .chars()
                    .filter(|c| *c != '\n' && *c != '\r' && *c != '\t')
                    .collect();
                if filtered.is_empty() && !new_text.is_empty() {
                    return;
                }
                let tab_id = self.tabs[self.active].session_id;
                if let Some(model) = self.csv_models.get_mut(&tab_id) {
                    if let Some(input) = model.replace_input.as_mut() {
                        let cursor = input.cursor;
                        let anchor = input.anchor;
                        replace_input_utf16(
                            &mut input.buffer,
                            &mut input.cursor,
                            &mut input.anchor,
                            cursor,
                            anchor,
                            range_utf16,
                            &filtered,
                        );
                        model.replace_text = input.buffer.clone();
                        // Replace-summary stale once user keeps typing.
                        model.replace_summary = None;
                        cx.notify();
                    }
                }
            }
            InputFocus::CsvGrid => {
                // Type-to-edit: the grid has focus but no cell editor is
                // open. Any printable input opens the editor on the
                // active cell, seeded with the typed text (matches
                // Excel — start typing on a highlighted cell to replace
                // its contents).
                let filtered: String = new_text
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if filtered.is_empty() {
                    return;
                }
                crate::ui::csv_doc::actions::begin_cell_edit_with_seed(
                    self, &filtered, window, cx,
                );
            }
            InputFocus::Editor => {
                // NPP `Editing 2 ▸ Prevent control character (C0 code) typing
                // into document` — strip 0x00..=0x08, 0x0E..=0x1F, 0x7F from
                // typed text. \t (0x09), \n (0x0A), \r (0x0D) are legitimate
                // and pass through. Empty result after strip → drop the edit.
                let filtered_owned;
                let new_text: &str = if settings::snapshot().prevent_c0_input
                    && new_text.bytes().any(is_c0_control)
                {
                    filtered_owned = strip_c0_controls(new_text);
                    if filtered_owned.is_empty() && range_utf16.is_none() {
                        return;
                    }
                    filtered_owned.as_str()
                } else {
                    new_text
                };

                // NPP `Auto-Completion ▸ HTML/XML close tag` — when the user
                // types `/` immediately after a `<` in an HTML/XML buffer,
                // splice the nearest unclosed open tag's name + `>` so the
                // pair `</tagname>` lands in one keystroke. Gated on the same
                // settings flag that drives `<>` auto-pair so the user has
                // one switch for "HTML/XML helpers".
                if range_utf16.is_none()
                    && !self.has_block_selection()
                    && new_text == "/"
                    && self.selection().is_empty()
                    && settings::snapshot().auto_insert_html_xml_tag
                {
                    let lang = self.buffer().language;
                    let is_html_or_xml = matches!(
                        lang,
                        Some(crate::syntax::LanguageId::Html)
                            | Some(crate::syntax::LanguageId::Xml)
                    );
                    if is_html_or_xml {
                        let cursor = super::edits::floor_rope_char_boundary(
                            &self.buffer().rope,
                            self.selection().head,
                        );
                        let html_mode = matches!(
                            lang,
                            Some(crate::syntax::LanguageId::Html)
                        );
                        if let Some(tag) = super::close_tag::close_tag_for_slash(
                            &self.buffer().rope,
                            cursor,
                            html_mode,
                        ) {
                            // Insert `/tag>` at the caret, then place the
                            // caret right after the closing `>` so the
                            // user can keep typing.
                            let payload = format!("/{tag}>");
                            crate::macros::record(
                                crate::macros::MacroEvent::InsertText(payload.clone()),
                            );
                            // Only the typed `/` overtypes; Notepad++
                            // inserts the tag name after it.
                            let end = self.overtype_end(cursor, "/");
                            self.replace_byte_range(cursor..end, &payload, cx);
                            if self.find_state.open
                                && !self.find_state.query.is_empty()
                            {
                                self.refresh_matches();
                            }
                            self.smart_match_word = None;
                            self.refresh_smart_highlight();
                            cx.notify();
                            return;
                        }
                    }
                }

                // NPP `Auto-Completion ▸ Auto-Insert` — single-char keystroke
                // dispatch. Skip-over advances the cursor when the typed char
                // sits right where the auto-pair would put its closer; the
                // opener path inserts `open + close` (or wraps selection).
                // Block-selection / paste / IME (range_utf16=Some) fall
                // through to the linear path below unchanged.
                let typed_for_pair = if range_utf16.is_none() && !self.has_block_selection() {
                    single_char(new_text)
                } else {
                    None
                };
                if let Some(typed) = typed_for_pair {
                    let snap = settings::snapshot();
                    let sel = self.selection();
                    let cursor =
                        super::edits::floor_rope_char_boundary(&self.buffer().rope, sel.head);
                    let next_char = char_at_byte(&self.buffer().rope, cursor);
                    match compute_auto_pair_action(typed, sel.is_empty(), next_char, &snap) {
                        AutoPairAction::SkipOver => {
                            let advance = typed.len_utf8();
                            let new_pos = cursor + advance;
                            self.active_tab_mut().selection.collapse_to(new_pos);
                            cx.notify();
                            return;
                        }
                        AutoPairAction::InsertPair { open, close } => {
                            let mut payload = String::with_capacity(open.len_utf8() + close.len_utf8());
                            payload.push(open);
                            payload.push(close);
                            crate::macros::record(crate::macros::MacroEvent::InsertText(
                                payload.clone(),
                            ));
                            // Only the typed opener overtypes; Notepad++
                            // inserts the closer after it.
                            let end = self.overtype_end(cursor, open.encode_utf8(&mut [0; 4]));
                            self.replace_byte_range(cursor..end, &payload, cx);
                            // `replace_byte_range` collapsed the cursor to
                            // `cursor + payload.len()`. Move it back between
                            // the two pair chars.
                            let between = cursor + open.len_utf8();
                            self.active_tab_mut().selection.collapse_to(between);
                            if self.find_state.open && !self.find_state.query.is_empty() {
                                self.refresh_matches();
                            }
                            cx.notify();
                            return;
                        }
                        AutoPairAction::WrapSelection { open, close } => {
                            let range = {
                                let rope = &self.buffer().rope;
                                let r = sel.ordered();
                                super::edits::floor_rope_char_boundary(rope, r.start)
                                    ..super::edits::floor_rope_char_boundary(rope, r.end)
                            };
                            let selected: String = self
                                .buffer()
                                .rope
                                .byte_slice(range.start..range.end)
                                .to_string();
                            let mut payload = String::with_capacity(
                                open.len_utf8() + selected.len() + close.len_utf8(),
                            );
                            payload.push(open);
                            payload.push_str(&selected);
                            payload.push(close);
                            crate::macros::record(crate::macros::MacroEvent::InsertText(
                                payload.clone(),
                            ));
                            self.replace_byte_range(range.clone(), &payload, cx);
                            // Restore the original selection between the new
                            // wrapping chars so the user can keep extending.
                            let inner_start = range.start + open.len_utf8();
                            let inner_end = inner_start + selected.len();
                            let tab = self.active_tab_mut();
                            tab.selection.anchor = inner_start;
                            tab.selection.head = inner_end;
                            if self.find_state.open && !self.find_state.query.is_empty() {
                                self.refresh_matches();
                            }
                            cx.notify();
                            return;
                        }
                        AutoPairAction::None => {}
                    }
                }

                // Markdown preview: a single `\n` inside source text is a
                // pulldown-cmark soft break and renders as a space, so every
                // newline run has to be `\n\n` (the paragraph separator) for
                // a real line break to appear in the preview. Collapse any
                // run of one or more newlines to exactly `\n\n` — handles
                // Enter keystrokes (just `\n`), trailing/leading newlines in
                // pastes, and consecutive newlines all in one pass. The
                // earlier split+filter+join approach dropped empty segments,
                // which silently ate Enter keystrokes and edge newlines.
                let md_paragraph_owned;
                let new_text: &str = if new_text.contains('\n') && {
                    let tab = &self.tabs[self.active];
                    matches!(tab.view_mode, crate::tab::TabViewMode::MarkdownPreview)
                        && crate::ui::markdown_view::is_markdown_path(
                            tab.buffer.file_path.as_deref(),
                        )
                } {
                    let normalized = new_text.replace("\r\n", "\n").replace('\r', "\n");
                    let mut out = String::with_capacity(normalized.len() + 2);
                    let mut in_newline_run = false;
                    for c in normalized.chars() {
                        if c == '\n' {
                            if !in_newline_run {
                                out.push_str("\n\n");
                                in_newline_run = true;
                            }
                        } else {
                            out.push(c);
                            in_newline_run = false;
                        }
                    }
                    md_paragraph_owned = out;
                    md_paragraph_owned.as_str()
                } else {
                    new_text
                };

                // Column-mode (block) selection: every text replace is a
                // rectangular edit. `block_replace` runs the per-line
                // delete + insert inside one undo group and updates the
                // block to a thin column at the post-insert position.
                if self.has_block_selection() {
                    if !new_text.is_empty() {
                        crate::macros::record(crate::macros::MacroEvent::InsertText(
                            new_text.to_string(),
                        ));
                    }
                    self.block_replace(new_text, cx);
                    if self.find_state.open && !self.find_state.query.is_empty() {
                        self.refresh_matches();
                    }
                    // Buffer changed under the smart-hilite cache; force the
                    // next caret-move to re-scan against the new bytes.
                    self.smart_match_word = None;
                    self.refresh_smart_highlight();
                    return;
                }
                // Capture pre-insert state for the auto-completion auto-trigger
                // gate below. A real point-insertion is `range_utf16=None` AND
                // an empty selection — IME range-replace and selection-replace
                // both turn this false so neither path ghost-opens the popup.
                // Computed before `byte_range` moves `range_utf16`.
                let plain_point_insert = range_utf16.is_none()
                    && self.selection().is_empty()
                    && !self.has_block_selection();
                let mut byte_range = if let Some(ru) = range_utf16 {
                    utf16_range_to_bytes(&self.buffer().rope, ru)
                } else {
                    self.selection().ordered()
                };
                // Overtype (Insert key): a typed character replaces the next
                // one. Selection replaces and IME edits never overtype.
                if plain_point_insert {
                    byte_range.end = self.overtype_end(byte_range.start, new_text);
                }
                if !new_text.is_empty() {
                    crate::macros::record(crate::macros::MacroEvent::InsertText(new_text.to_string()));
                }
                self.replace_byte_range(byte_range, new_text, cx);
                if self.find_state.open && !self.find_state.query.is_empty() {
                    self.refresh_matches();
                }
                // NPP `Auto-Completion ▸ From N-th character`. Auto-open the
                // popup when a single identifier char has just been typed at
                // the caret and the prefix has reached the threshold length.
                // Paste routes through `replace_byte_range` directly (see
                // `paste` in `edits.rs`) so it never reaches this branch;
                // multi-char IME commits return None from `single_char` and
                // bypass too. Mode + threshold + popup-already-open gating
                // lives in `maybe_auto_trigger_autocomplete`.
                if plain_point_insert {
                    if let Some(c) = single_char(new_text) {
                        if super::autocomplete::is_word_char(c) {
                            self.maybe_auto_trigger_autocomplete(cx);
                        }
                    }
                }
            }
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // No IME composition state yet; treat as a plain replace.
        self.replace_text_in_range(range_utf16, new_text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.input_focus(window) != InputFocus::Editor {
            return Some(bounds);
        }
        let rope = &self.buffer().rope;
        let byte_start = rope_utf16_to_byte(rope, range_utf16.start);
        let byte_end = rope_utf16_to_byte(rope, range_utf16.end);
        for line in &self.last_lines {
            if let Some((local_start, local_end)) = local_span_on_line(
                line.line_byte_start,
                line.line_byte_len,
                byte_start,
                byte_end,
            ) {
                let x_start = line.bounds.left() + line.shaped.x_for_index(local_start);
                let x_end = line.bounds.left() + line.shaped.x_for_index(local_end);
                return Some(Bounds::from_corners(
                    point(x_start, line.bounds.top()),
                    point(x_end, line.bounds.bottom()),
                ));
            }
        }
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        if self.input_focus(window) != InputFocus::Editor {
            return Some(0);
        }
        let byte_offset = self.offset_at_position(point);
        Some(rope_byte_to_utf16(&self.buffer().rope, byte_offset))
    }
}
