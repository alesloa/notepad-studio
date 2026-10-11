// Blank Operations ▶ submenu — Trim leading / both, EOL→Space, Remove
// blank+EOL, TAB↔Space. Trim Trailing already exists in `workspace::edits`.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:2179 — IDM_EDIT_TRIMLINEHEAD (leading)
//   PowerEditor/src/NppCommands.cpp:2189 — IDM_EDIT_TRIM_BOTH
//   PowerEditor/src/NppCommands.cpp:2199 — IDM_EDIT_EOL2WS
//   PowerEditor/src/NppCommands.cpp:2169 — IDM_EDIT_TRIMTRAILING (existing)
//   PowerEditor/src/NppCommands.cpp:2205 — IDM_EDIT_TRIMALL (= doTrim(both)+eol2ws)
//   PowerEditor/src/NppCommands.cpp:2218 — IDM_EDIT_TAB2SW (TAB→Space)
//   PowerEditor/src/NppCommands.cpp:2222 — IDM_EDIT_SW2TAB_LEADING
//   PowerEditor/src/NppCommands.cpp:2226 — IDM_EDIT_SW2TAB_ALL
//   Notepad_plus.cpp:1343 wsTabConvert (column-aware tab/space conversion)
//   Notepad_plus.cpp:1628 doTrim (regex-driven trim)
//   Notepad_plus.cpp:1684 eol2ws  (calls SCI_LINESJOIN — see below)
//
// SCI_LINESJOIN semantics (Scintilla `Editor::LinesJoin`): walks bytes,
// removes each EOL char in turn; if the previous non-EOL byte was a
// non-whitespace char, inserts a single space, otherwise nothing. Net:
// every EOL run collapses with surrounding whitespace into a single space.
//
// The byte-walking transforms copy every byte they keep into a `Vec<u8>`, so
// multi-byte UTF-8 passes through whole; they only add or drop ASCII bytes.

use gpui::{Context, Window};

use crate::actions::*;
use crate::edit_ops::util::{floor_char_boundary, split_lines_eol, trim_selected_line_range};
use crate::workspace::Workspace;

#[cfg(test)]
#[path = "trim_leading_scope_tests.rs"]
mod trim_leading_scope_tests;

#[path = "trim_both_scope.rs"]
mod trim_both_scope;
#[cfg(test)]
#[path = "trim_both_scope_tests.rs"]
mod trim_both_scope_tests;

impl Workspace {
    pub(crate) fn trim_leading_ws(
        &mut self,
        _: &EditTrimLeadingWs,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // PowerEditor/src/Notepad_plus.cpp:1651–1704 — trim only expanded
        // selected physical lines; rectangular selection is unsupported.
        let Some(tab) = self.tabs.get(self.active) else { return; };
        if tab.block_selection.is_some() { return; }
        if tab.view_mode != crate::tab::TabViewMode::Edit {
            self.set_status("Switch to Source or Raw View to trim leading space");
            cx.notify();
            return;
        }
        if tab.buffer.read_only {
            self.set_status("Buffer is read-only");
            cx.notify();
            return;
        }
        let selection = tab.selection.clamped(&tab.buffer.rope);
        if selection.is_empty() {
            if !tab.selection.is_empty() { return; }
            run_full_buffer_transform(self, cx, |s| Ok(trim_leading(s)));
            return;
        }
        let range = trim_selected_line_range(&tab.buffer.rope, selection.ordered());
        let Some(slice) = tab.buffer.rope.get_byte_slice(range.clone()) else { return; };
        let mut out = String::new();
        if out.try_reserve(range.end.saturating_sub(range.start)).is_err() {
            self.set_status("Trim Leading Space skipped: not enough memory");
            cx.notify();
            return;
        }
        let mut leading = true;
        for ch in slice.chars() {
            if leading && matches!(ch, ' ' | '\t') { continue; }
            out.push(ch);
            leading = matches!(ch, '\r' | '\n');
        }
        if slice.chars().eq(out.chars()) { return; }
        self.replace_byte_range(range.clone(), &out, cx);
        let tab = self.active_tab_mut();
        tab.selection.anchor = range.start;
        tab.selection.head = range.start.saturating_add(out.len());
        tab.desired_col_x = None;
        self.commit_caret(cx);
    }

    pub(crate) fn trim_leading_and_trailing_ws(
        &mut self,
        _: &EditTrimLeadingAndTrailingWs,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trim_both_scope::apply(self, cx);
    }

    pub(crate) fn eol_to_space(
        &mut self,
        _: &EditEolToSpace,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        run_full_buffer_transform(self, cx, |s| Ok(eol_to_space_lines_join(s)));
    }

    pub(crate) fn remove_unnecessary_blank_and_eol(
        &mut self,
        _: &EditRemoveUnnecessaryBlankAndEol,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // NPP IDM_EDIT_TRIMALL: trim leading+trailing per line, then apply
        // SCI_LINESJOIN. Composed transform.
        run_full_buffer_transform(self, cx, |s| {
            let trimmed = trim_both(s);
            Ok(eol_to_space_lines_join(&trimmed))
        });
    }

    pub(crate) fn tab_to_space(
        &mut self,
        _: &EditTabToSpace,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = crate::settings::snapshot().tab_width.max(1);
        run_full_buffer_transform(self, cx, |s| {
            tab_to_space(s, width).ok_or_else(|| {
                format!("TAB to Space skipped: not enough memory to expand tabs to width {width}")
            })
        });
    }

    pub(crate) fn space_to_tab_all(
        &mut self,
        _: &EditSpaceToTabAll,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = crate::settings::snapshot().tab_width.max(1);
        run_full_buffer_transform(self, cx, |s| Ok(space_to_tab(s, width, false)));
    }

    pub(crate) fn space_to_tab_leading(
        &mut self,
        _: &EditSpaceToTabLeading,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = crate::settings::snapshot().tab_width.max(1);
        run_full_buffer_transform(self, cx, |s| Ok(space_to_tab(s, width, true)));
    }
}

/// Apply a pure-text transform to the active buffer's full contents as a
/// single undoable replacement, then collapse the selection at the clamped
/// caret position. Mirrors `trim_trailing_ws`'s pattern. A transform that
/// fails leaves the buffer alone and reports why in the status bar.
fn run_full_buffer_transform<F>(ws: &mut Workspace, cx: &mut Context<Workspace>, transform: F)
where
    F: FnOnce(&str) -> Result<String, String>,
{
    let total_bytes = ws.buffer().rope.len_bytes();
    if total_bytes == 0 {
        return;
    }
    let text = ws.buffer().rope.slice(..).to_string();
    let out = match transform(&text) {
        Ok(out) => out,
        Err(msg) => {
            ws.set_status(msg);
            cx.notify();
            return;
        }
    };
    if out == text {
        return;
    }
    let head_was = ws.selection().head;
    ws.replace_byte_range(0..total_bytes, &out, cx);
    // The old caret offset can fall inside a multi-byte char of the new text.
    let clamped = floor_char_boundary(&ws.buffer().rope, head_was);
    let tab = ws.active_tab_mut();
    tab.selection.collapse_to(clamped);
    tab.desired_col_x = None;
    ws.commit_caret(cx);
}

// ---- Pure transforms -----------------------------------------------------

fn trim_leading(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (body, term) in split_lines_eol(text) {
        out.push_str(body.trim_start_matches([' ', '\t']));
        out.push_str(term);
    }
    out
}

fn trim_both(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (body, term) in split_lines_eol(text) {
        out.push_str(body.trim_matches([' ', '\t']));
        out.push_str(term);
    }
    out
}

/// Emulate Scintilla `SCI_LINESJOIN`: walk the buffer and delete every EOL
/// byte (\n, \r, or CRLF pair). When deleting an EOL whose previous non-EOL
/// byte was a non-whitespace char, emit a single space. Otherwise skip.
/// Net effect — every line-ending run collapses with adjacent whitespace into
/// a single space, so `"foo \n\n bar"` becomes `"foo bar"`.
fn eol_to_space_lines_join(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    let mut prev_non_ws = false;
    let mut iter = bytes.iter().copied().peekable();
    while let Some(b) = iter.next() {
        if b == b'\n' || b == b'\r' {
            if prev_non_ws {
                out.push(b' ');
            }
            prev_non_ws = false;
            // Consume the LF of a CRLF pair.
            if b == b'\r' && iter.peek() == Some(&b'\n') {
                iter.next();
            }
        } else {
            out.push(b);
            // Space and tab are whitespace; every other byte, including the
            // bytes of a multi-byte char, counts as non-whitespace.
            prev_non_ws = b != b' ' && b != b'\t';
        }
    }
    // Only ASCII EOL bytes were dropped and ASCII spaces added, so `out` is
    // valid UTF-8; the fallback leaves the text unchanged.
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// Column-aware TAB → spaces. Each `\t` expands to enough spaces to reach the
/// next tab stop (column % tab_width == 0). UTF-8 continuation bytes don't
/// advance the column. Returns `None` when the expanded text can't be
/// allocated (a huge tab width from settings).
fn tab_to_space(text: &str, tab_width: usize) -> Option<String> {
    let tab_width = tab_width.max(1);
    let mut out: Vec<u8> = Vec::new();
    out.try_reserve(text.len()).ok()?;
    let mut column = 0usize;
    for &b in text.as_bytes() {
        match b {
            b'\t' => {
                let fill = tab_width - (column % tab_width);
                out.try_reserve(fill).ok()?;
                out.resize(out.len().saturating_add(fill), b' ');
                column = column.saturating_add(fill);
            }
            b'\n' | b'\r' => {
                out.push(b);
                column = 0;
            }
            _ => {
                out.push(b);
                if b < 0x80 || (b & 0xC0) != 0x80 {
                    column = column.saturating_add(1);
                }
            }
        }
    }
    // Tabs became ASCII spaces and every other byte was copied, so `out` is
    // valid UTF-8; the fallback leaves the text unchanged.
    Some(String::from_utf8(out).unwrap_or_else(|_| text.to_string()))
}

/// Column-aware Space → TAB. Mirror of NPP `wsTabConvert(space2Tab*)`. Walks
/// each line tracking the visual column. Runs of spaces that cross a tab
/// stop boundary collapse to `\t`. Spaces "absorbed" by a trailing tab also
/// become tab. With `only_leading`, conversion stops once a non-space/tab
/// char is seen on the line (so inline spacing in code stays untouched).
fn space_to_tab(text: &str, tab_width: usize, only_leading: bool) -> String {
    let tab_width = tab_width.max(1);
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    let mut column = 0usize;
    // Distance to the next tab stop, counted from zero (mirrors NPP `tabStop`).
    let mut tab_stop = tab_width.saturating_sub(1);
    let mut nonspace_seen = false;
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        // Handle leading-only gating: once a non-space/tab is hit on this
        // line, fall through to a copy-only path until EOL.
        if only_leading && nonspace_seen {
            out.push(b);
            match b {
                b'\n' | b'\r' => {
                    column = 0;
                    tab_stop = tab_width.saturating_sub(1);
                    nonspace_seen = false;
                }
                _ => {
                    if b < 0x80 || (b & 0xC0) != 0x80 {
                        column = column.saturating_add(1);
                    }
                }
            }
            i += 1;
            continue;
        }

        if b == b' ' {
            // Try to consume a run of spaces that crosses a tab stop.
            let mut counter = 0usize;
            let mut emitted = false;
            loop {
                if bytes.get(i + counter) != Some(&b' ') {
                    break;
                }
                if column.saturating_add(counter) == tab_stop {
                    tab_stop = tab_stop.saturating_add(tab_width);
                    if counter >= 1 {
                        out.push(b'\t');
                        // Skip the run up to and including the boundary space.
                        i += counter + 1;
                        column = column.saturating_add(counter + 1);
                        emitted = true;
                        break;
                    } else if matches!(bytes.get(i + 1), Some(b' ') | Some(b'\t')) {
                        // Single space at a tab stop followed by ws → tab.
                        out.push(b'\t');
                        i += 1;
                        column = column.saturating_add(1);
                        emitted = true;
                        break;
                    } else {
                        // Single space at boundary, not followed by ws.
                        out.push(b' ');
                        column = column.saturating_add(1);
                        i += 1;
                        emitted = true;
                        break;
                    }
                }
                counter += 1;
            }

            if emitted {
                continue;
            }

            // Spaces "absorbed" by a trailing tab on the right.
            if bytes.get(i + counter) == Some(&b'\t') {
                out.push(b'\t');
                i += counter + 1;
                column = tab_stop.saturating_add(1);
                tab_stop = tab_stop.saturating_add(tab_width);
                continue;
            }

            // Plain space, no boundary in sight — emit as-is and advance.
            out.push(b' ');
            column = column.saturating_add(1);
            i += 1;
            continue;
        }

        // Non-space char.
        if only_leading && b != b'\t' && b != b'\n' && b != b'\r' {
            nonspace_seen = true;
        }

        match b {
            b'\n' | b'\r' => {
                out.push(b);
                column = 0;
                tab_stop = tab_width.saturating_sub(1);
                nonspace_seen = false;
            }
            b'\t' => {
                out.push(b'\t');
                column = tab_stop.saturating_add(1);
                tab_stop = tab_stop.saturating_add(tab_width);
            }
            _ => {
                out.push(b);
                if b < 0x80 || (b & 0xC0) != 0x80 {
                    column = column.saturating_add(1);
                    if column.is_multiple_of(tab_width) {
                        tab_stop = tab_stop.saturating_add(tab_width);
                    }
                }
            }
        }
        i += 1;
    }
    // Only ASCII spaces were swapped for tabs, so `out` is valid UTF-8; the
    // fallback leaves the text unchanged.
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_leading_strips_spaces_and_tabs() {
        assert_eq!(trim_leading("  hi\n\tworld  \n"), "hi\nworld  \n");
    }

    #[test]
    fn trim_both_strips_both_sides() {
        assert_eq!(trim_both("  hi  \n\tworld\t\n"), "hi\nworld\n");
    }

    #[test]
    fn eol_to_space_collapses_blank_lines() {
        assert_eq!(eol_to_space_lines_join("foo\n\nbar"), "foo bar");
        assert_eq!(eol_to_space_lines_join("foo \n\n bar"), "foo  bar");
        assert_eq!(eol_to_space_lines_join("a\r\nb\r\nc"), "a b c");
    }

    #[test]
    fn trimall_removes_blanks() {
        // Match NPP TRIMALL exactly: trim both per line, then SCI_LINESJOIN.
        // The final trailing newline becomes a trailing space (Scintilla keeps
        // it because the target range covers the full doc including the
        // closing EOL — same observable behavior as NPP).
        let out = eol_to_space_lines_join(&trim_both("foo  \n\n  bar  \n"));
        assert_eq!(out, "foo bar ");
        // Without the trailing newline the result is clean.
        let clean = eol_to_space_lines_join(&trim_both("foo  \n\n  bar"));
        assert_eq!(clean, "foo bar");
    }

    #[test]
    fn tab_to_space_column_aware() {
        // tab at col 0 → 4 spaces; ab + tab → 2 spaces to fill to col 4.
        assert_eq!(tab_to_space("\tfoo", 4).as_deref(), Some("    foo"));
        assert_eq!(tab_to_space("ab\tcd", 4).as_deref(), Some("ab  cd"));
        assert_eq!(tab_to_space("abcd\tef", 4).as_deref(), Some("abcd    ef"));
    }

    #[test]
    fn space_to_tab_all_collapses_runs() {
        // 4 leading spaces from col 0 collapse to one tab.
        assert_eq!(space_to_tab("    foo", 4, false), "\tfoo");
        // 2 spaces at col 2-3 hit tab stop 3 → tab; remaining 2 spaces don't
        // reach the next stop (col 7), stay as spaces. Matches NPP wsTabConvert.
        assert_eq!(space_to_tab("ab    cd", 4, false), "ab\t  cd");
        // 2 spaces at col 2-3 fully consumed by the tab.
        assert_eq!(space_to_tab("ab  cd", 4, false), "ab\tcd");
    }

    #[test]
    fn non_ascii_text_survives_blank_ops() {
        assert_eq!(tab_to_space("é\tx", 4).as_deref(), Some("é   x"));
        assert_eq!(eol_to_space_lines_join("é\n中"), "é 中");
        assert_eq!(space_to_tab("中    x", 4, false), "中\t x");
    }

    #[test]
    fn huge_tab_width_does_not_overflow() {
        assert_eq!(space_to_tab("\t\t\t", usize::MAX, false), "\t\t\t");
        assert_eq!(space_to_tab("a  \t  b", usize::MAX, true), "a  \t  b");
        // Expanding to usize::MAX columns can't be allocated: refused, not
        // attempted.
        assert_eq!(tab_to_space("a\tb", usize::MAX), None);
        // A zero width is treated as one column, not a division by zero.
        assert_eq!(tab_to_space("a\tb", 0).as_deref(), Some("a b"));
        assert_eq!(space_to_tab("a    b", 0, false), "a\t\t\t b");
    }

    /// Every blank op over the shared edge-case inputs: valid UTF-8, and the
    /// line-end sequence is untouched (EOL→Space removes it, by design).
    #[test]
    fn blank_ops_keep_line_ends_on_edge_inputs() {
        use crate::edit_ops::util::split_lines_eol;
        let inputs = [
            "",
            "abc",
            "  a\n\tb\n",
            " a \r\n\t b\t\r\n",
            "  a\r  b\r",
            " a\r\n b\n c\r d",
            "é\t中 \n 👨‍👩‍👧\t🇯🇵\r\n  ß İ  ",
            "\t   \n    \n",
            "x",
            // ropey also breaks lines on VT, FF, NEL, LS and PS.
            " a\u{2028}\tb\n c\u{85} d\u{b}e\u{c}f\u{2029} g\n",
        ];
        let eols = |s: &str| -> Vec<String> {
            split_lines_eol(s).into_iter().map(|p| p.1.to_string()).collect()
        };
        for input in inputs {
            let outs = [
                ("trim_leading", trim_leading(input)),
                ("trim_both", trim_both(input)),
                ("tab_to_space", tab_to_space(input, 4).unwrap_or_default()),
                ("space_to_tab_all", space_to_tab(input, 4, false)),
                ("space_to_tab_leading", space_to_tab(input, 4, true)),
            ];
            for (name, out) in outs {
                assert_eq!(eols(&out), eols(input), "{name} on {input:?} -> {out:?}");
                // Only spaces and tabs change.
                let strip = |s: &str| s.replace([' ', '\t'], "");
                assert_eq!(strip(&out), strip(input), "{name} on {input:?}");
            }
            let joined = eol_to_space_lines_join(input);
            assert!(!joined.contains(['\r', '\n']), "eol_to_space on {input:?}");
            assert_eq!(
                joined.replace(' ', ""),
                input.replace([' ', '\r', '\n'], ""),
                "eol_to_space on {input:?}"
            );
        }
    }

    #[test]
    fn trim_handles_cr_lines() {
        assert_eq!(trim_leading("  a\r  b\r"), "a\rb\r");
    }

    #[test]
    fn space_to_tab_leading_only_indent() {
        assert_eq!(space_to_tab("    foo    bar", 4, true), "\tfoo    bar");
        assert_eq!(space_to_tab("foo    bar", 4, true), "foo    bar");
    }
}
