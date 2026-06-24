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

use gpui::{Context, Window};

use crate::actions::*;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn trim_leading_ws(
        &mut self,
        _: &EditTrimLeadingWs,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        run_full_buffer_transform(self, cx, trim_leading);
    }

    pub(crate) fn trim_leading_and_trailing_ws(
        &mut self,
        _: &EditTrimLeadingAndTrailingWs,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        run_full_buffer_transform(self, cx, trim_both);
    }

    pub(crate) fn eol_to_space(
        &mut self,
        _: &EditEolToSpace,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        run_full_buffer_transform(self, cx, eol_to_space_lines_join);
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
            eol_to_space_lines_join(&trimmed)
        });
    }

    pub(crate) fn tab_to_space(
        &mut self,
        _: &EditTabToSpace,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = crate::settings::snapshot().tab_width.max(1);
        run_full_buffer_transform(self, cx, |s| tab_to_space(s, width));
    }

    pub(crate) fn space_to_tab_all(
        &mut self,
        _: &EditSpaceToTabAll,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = crate::settings::snapshot().tab_width.max(1);
        run_full_buffer_transform(self, cx, |s| space_to_tab(s, width, false));
    }

    pub(crate) fn space_to_tab_leading(
        &mut self,
        _: &EditSpaceToTabLeading,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = crate::settings::snapshot().tab_width.max(1);
        run_full_buffer_transform(self, cx, |s| space_to_tab(s, width, true));
    }
}

/// Apply a pure-text transform to the active buffer's full contents as a
/// single undoable replacement, then collapse the selection at the clamped
/// caret position. Mirrors `trim_trailing_ws`'s pattern.
fn run_full_buffer_transform<F>(ws: &mut Workspace, cx: &mut Context<Workspace>, transform: F)
where
    F: FnOnce(&str) -> String,
{
    let total_bytes = ws.buffer().rope.len_bytes();
    if total_bytes == 0 {
        return;
    }
    let text = ws.buffer().rope.slice(..).to_string();
    let out = transform(&text);
    if out == text {
        return;
    }
    let head_was = ws.selection().head;
    ws.replace_byte_range(0..total_bytes, &out, cx);
    let clamped = head_was.min(ws.buffer().rope.len_bytes());
    let tab = ws.active_tab_mut();
    tab.selection.collapse_to(clamped);
    tab.desired_col_x = None;
    ws.commit_caret(cx);
}

// ---- Pure transforms -----------------------------------------------------

fn trim_leading(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, term) = split_line_terminator(line);
        out.push_str(body.trim_start_matches([' ', '\t']));
        out.push_str(term);
    }
    out
}

fn trim_both(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, term) = split_line_terminator(line);
        out.push_str(body.trim_matches([' ', '\t']));
        out.push_str(term);
    }
    out
}

/// Strip CRLF, LF, or CR off the end of a line slice from `split_inclusive`.
fn split_line_terminator(line: &str) -> (&str, &str) {
    if let Some(s) = line.strip_suffix("\r\n") {
        (s, "\r\n")
    } else if let Some(s) = line.strip_suffix('\n') {
        (s, "\n")
    } else if let Some(s) = line.strip_suffix('\r') {
        (s, "\r")
    } else {
        (line, "")
    }
}

/// Emulate Scintilla `SCI_LINESJOIN`: walk the buffer and delete every EOL
/// byte (\n, \r, or CRLF pair). When deleting an EOL whose previous non-EOL
/// byte was a non-whitespace char, emit a single space. Otherwise skip.
/// Net effect — every line-ending run collapses with adjacent whitespace into
/// a single space, so `"foo \n\n bar"` becomes `"foo bar"`.
fn eol_to_space_lines_join(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut prev_non_ws = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            if prev_non_ws {
                out.push(' ');
            }
            prev_non_ws = false;
            i += 1;
        } else if b == b'\r' {
            if prev_non_ws {
                out.push(' ');
            }
            prev_non_ws = false;
            // Consume optional LF in CRLF.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
        } else {
            out.push(b as char);
            // UTF-8 continuation bytes don't toggle prev_non_ws on their own;
            // ASCII path covers space/tab vs. non-WS. For multi-byte chars
            // (always non-ASCII whitespace candidates excluded here), treat
            // any non-EOL non-ASCII byte as non-whitespace.
            if b == b' ' || b == b'\t' {
                prev_non_ws = false;
            } else if b < 0x80 {
                prev_non_ws = true;
            } else {
                // Non-ASCII leading or continuation byte.
                prev_non_ws = true;
            }
            i += 1;
        }
    }
    out
}

/// Column-aware TAB → spaces. Each `\t` expands to enough spaces to reach the
/// next tab stop (column % tab_width == 0). UTF-8 continuation bytes don't
/// advance the column.
fn tab_to_space(text: &str, tab_width: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut column = 0usize;
    for &b in bytes {
        match b {
            b'\t' => {
                let fill = tab_width - (column % tab_width);
                for _ in 0..fill {
                    out.push(' ');
                }
                column += fill;
            }
            b'\n' | b'\r' => {
                out.push(b as char);
                column = 0;
            }
            _ => {
                out.push(b as char);
                if b < 0x80 || (b & 0xC0) != 0x80 {
                    column += 1;
                }
            }
        }
    }
    out
}

/// Column-aware Space → TAB. Mirror of NPP `wsTabConvert(space2Tab*)`. Walks
/// each line tracking the visual column. Runs of spaces that cross a tab
/// stop boundary collapse to `\t`. Spaces "absorbed" by a trailing tab also
/// become tab. With `only_leading`, conversion stops once a non-space/tab
/// char is seen on the line (so inline spacing in code stays untouched).
fn space_to_tab(text: &str, tab_width: usize, only_leading: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut column = 0usize;
    // Distance to the next tab stop, counted from zero (mirrors NPP `tabStop`).
    let mut tab_stop = tab_width.saturating_sub(1);
    let mut nonspace_seen = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];

        // Handle leading-only gating: once a non-space/tab is hit on this
        // line, fall through to a copy-only path until EOL.
        if only_leading && nonspace_seen {
            out.push(b as char);
            match b {
                b'\n' | b'\r' => {
                    column = 0;
                    tab_stop = tab_width.saturating_sub(1);
                    nonspace_seen = false;
                }
                _ => {
                    if b < 0x80 || (b & 0xC0) != 0x80 {
                        column += 1;
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
                let probe_byte = i + counter;
                if probe_byte >= bytes.len() || bytes[probe_byte] != b' ' {
                    break;
                }
                if column + counter == tab_stop {
                    tab_stop += tab_width;
                    if counter >= 1 {
                        out.push('\t');
                        i += counter; // i now sits on the boundary space
                        column += counter + 1;
                        // Consume that boundary space as part of the tab.
                        i += 1;
                        emitted = true;
                        break;
                    } else if i + 1 < bytes.len()
                        && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
                    {
                        // Single space at a tab stop followed by ws → tab.
                        out.push('\t');
                        i += 1;
                        column += 1;
                        emitted = true;
                        break;
                    } else {
                        // Single space at boundary, not followed by ws.
                        out.push(' ');
                        column += 1;
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
            if i + counter < bytes.len() && bytes[i + counter] == b'\t' {
                out.push('\t');
                i += counter + 1;
                column = tab_stop + 1;
                tab_stop += tab_width;
                continue;
            }

            // Plain space, no boundary in sight — emit as-is and advance.
            out.push(' ');
            column += 1;
            i += 1;
            continue;
        }

        // Non-space char.
        if only_leading && b != b'\t' && b != b'\n' && b != b'\r' {
            nonspace_seen = true;
        }

        match b {
            b'\n' | b'\r' => {
                out.push(b as char);
                column = 0;
                tab_stop = tab_width.saturating_sub(1);
                nonspace_seen = false;
            }
            b'\t' => {
                out.push('\t');
                column = tab_stop + 1;
                tab_stop += tab_width;
            }
            _ => {
                out.push(b as char);
                if b < 0x80 || (b & 0xC0) != 0x80 {
                    column += 1;
                    if column > 0 && column % tab_width == 0 {
                        tab_stop += tab_width;
                    }
                }
            }
        }
        i += 1;
    }
    out
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
        assert_eq!(tab_to_space("\tfoo", 4), "    foo");
        assert_eq!(tab_to_space("ab\tcd", 4), "ab  cd");
        assert_eq!(tab_to_space("abcd\tef", 4), "abcd    ef");
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
    fn space_to_tab_leading_only_indent() {
        assert_eq!(space_to_tab("    foo    bar", 4, true), "\tfoo    bar");
        assert_eq!(space_to_tab("foo    bar", 4, true), "foo    bar");
    }
}
