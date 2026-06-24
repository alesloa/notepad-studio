// Line Operations ▶ submenu — sort / reverse / randomize / dedupe / split /
// join / remove empty.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:885   — IDM_EDIT_SORTLINES_*
//   PowerEditor/src/NppCommands.cpp:2040  — IDM_EDIT_REMOVE_CONSECUTIVE_DUP_LINES
//   PowerEditor/src/NppCommands.cpp:2046  — IDM_EDIT_REMOVE_ANY_DUP_LINES
//   PowerEditor/src/NppCommands.cpp:2052  — IDM_EDIT_SPLIT_LINES
//   PowerEditor/src/NppCommands.cpp:2083  — IDM_EDIT_JOIN_LINES
//   PowerEditor/src/NppCommands.cpp:2105  — IDM_EDIT_REMOVEEMPTYLINES
//   PowerEditor/src/NppCommands.cpp:2111  — IDM_EDIT_REMOVEEMPTYLINESWITHBLANK
//   MISC/Common/Sorters.h                 — IntegerSorter / DecimalCommaSorter
//                                           / DecimalDotSorter / Reverse / Random
//   Notepad_plus.cpp:1736                 — removeDuplicateLines (consecutive)
//   ScintillaEditView.cpp:4798            — removeAnyDuplicateLines
//
// All ops operate on the line range covered by the selection if a selection
// exists, else on the whole buffer. NPP's selection-line-range rule: if the
// selection ends exactly at the start of line N (i.e. just past line N-1's
// terminator), line N is *not* included. Sort and dedup additionally bail
// when the selection covers only a single line. Each op produces one
// `replace_byte_range` call so it lands as a single undo entry.

use gpui::{Context, Window};

use crate::actions::*;
use crate::tab::InputFocus;
use crate::workspace::Workspace;

/// Span of full lines selected for a line op, plus the byte offsets that
/// frame those lines and a flag for whether the trailing line ended with a
/// terminator. `whole_buffer` is `true` when the user had no selection — NPP
/// uses this to gate single-line-only sort early-outs.
struct LineSpan {
    /// Inclusive line index of the first selected line.
    first_line: usize,
    /// Inclusive line index of the last selected line.
    last_line: usize,
    /// Byte offset of the start of `first_line`.
    block_start: usize,
    /// Byte offset just past `last_line`'s terminator (or buffer end if none).
    block_end: usize,
    /// True if the block ends with a `\n` / `\r` terminator. Lets us preserve
    /// the original trailing-newline state when rebuilding the replacement.
    has_terminator: bool,
    /// True when the user had no selection — op covers the whole buffer.
    whole_buffer: bool,
}

impl Workspace {
    /// Compute the line span the op should act on. Returns `None` if the
    /// buffer is empty.
    fn line_span(&self) -> Option<LineSpan> {
        let sel = self.selection();
        let rope = &self.buffer().rope;
        let total_lines = rope.len_lines();
        if total_lines == 0 {
            return None;
        }
        let total_bytes = rope.len_bytes();

        let (first_line, last_line, whole_buffer) = if sel.is_empty() {
            (0, total_lines.saturating_sub(1), true)
        } else {
            let r = sel.ordered();
            let first = rope.byte_to_line(r.start);
            let last_raw = rope.byte_to_line(r.end);
            // NPP rule: a selection that ends exactly at the start of a line
            // does not include that line.
            let last = if r.end > r.start
                && last_raw > first
                && rope.line_to_byte(last_raw) == r.end
            {
                last_raw - 1
            } else {
                last_raw
            };
            (first, last, false)
        };

        let block_start = rope.line_to_byte(first_line);
        let block_end = if last_line + 1 < total_lines {
            rope.line_to_byte(last_line + 1)
        } else {
            total_bytes
        };
        if block_start == block_end {
            return None;
        }
        let block_text = rope.byte_slice(block_start..block_end).to_string();
        let has_terminator = block_text.ends_with('\n') || block_text.ends_with('\r');
        Some(LineSpan {
            first_line,
            last_line,
            block_start,
            block_end,
            has_terminator,
            whole_buffer,
        })
    }

    /// Pull every line in `span` into a `Vec<String>` with the per-line
    /// terminator stripped. Mirrors NPP's `stringSplit` over the EOL string.
    fn extract_lines(&self, span: &LineSpan) -> Vec<String> {
        let rope = &self.buffer().rope;
        let mut out = Vec::with_capacity(span.last_line - span.first_line + 1);
        for line_idx in span.first_line..=span.last_line {
            let line_start = rope.line_to_byte(line_idx);
            let line_end = if line_idx + 1 < rope.len_lines() {
                rope.line_to_byte(line_idx + 1)
            } else {
                rope.len_bytes()
            };
            let mut text = rope.byte_slice(line_start..line_end).to_string();
            // Strip any trailing CR / LF — preserve only the body of the line.
            while text.ends_with('\n') || text.ends_with('\r') {
                text.pop();
            }
            out.push(text);
        }
        out
    }

    /// Rejoin `lines` with the buffer's line ending and replace the original
    /// span. Re-attaches a trailing terminator if the original block had one.
    fn replace_lines(
        &mut self,
        span: &LineSpan,
        lines: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let ending = self.buffer().line_ending.as_str().to_string();
        let mut joined = lines.join(&ending);
        if span.has_terminator {
            joined.push_str(&ending);
        }
        self.replace_byte_range(span.block_start..span.block_end, &joined, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(span.block_start);
        tab.desired_col_x = None;
    }

    // ---- Sort variants ---------------------------------------------------------------------

    fn sort_with<F>(&mut self, descending: bool, cmp: F, cx: &mut Context<Self>)
    where
        F: FnMut(&String, &String) -> std::cmp::Ordering,
    {
        let Some(span) = self.line_span() else {
            return;
        };
        // NPP gates sort to multi-line selections only when a selection exists
        // (whole-buffer sort is fine even for single-line files — the sort is
        // a no-op but harmless).
        if !span.whole_buffer && span.first_line == span.last_line {
            return;
        }
        let mut lines = self.extract_lines(&span);
        let mut cmp = cmp;
        if descending {
            lines.sort_by(|a, b| cmp(b, a));
        } else {
            lines.sort_by(|a, b| cmp(a, b));
        }
        self.replace_lines(&span, lines, cx);
    }

    pub(crate) fn sort_lines_lex_asc(
        &mut self,
        _: &EditSortLinesLexAsc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_with(false, |a, b| a.cmp(b), cx);
    }

    pub(crate) fn sort_lines_lex_desc(
        &mut self,
        _: &EditSortLinesLexDesc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_with(true, |a, b| a.cmp(b), cx);
    }

    pub(crate) fn sort_lines_int_asc(
        &mut self,
        _: &EditSortLinesIntAsc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_with(false, natural_cmp, cx);
    }

    pub(crate) fn sort_lines_int_desc(
        &mut self,
        _: &EditSortLinesIntDesc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_with(true, natural_cmp, cx);
    }

    pub(crate) fn sort_lines_dec_comma_asc(
        &mut self,
        _: &EditSortLinesDecCommaAsc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_decimal(false, DecimalSep::Comma, cx);
    }

    pub(crate) fn sort_lines_dec_comma_desc(
        &mut self,
        _: &EditSortLinesDecCommaDesc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_decimal(true, DecimalSep::Comma, cx);
    }

    pub(crate) fn sort_lines_dec_dot_asc(
        &mut self,
        _: &EditSortLinesDecDotAsc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_decimal(false, DecimalSep::Dot, cx);
    }

    pub(crate) fn sort_lines_dec_dot_desc(
        &mut self,
        _: &EditSortLinesDecDotDesc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.sort_decimal(true, DecimalSep::Dot, cx);
    }

    /// NPP `NumericSorter::sort` shape: filter into "empty after preparation"
    /// vs "parses as f64", stable-sort the numerics, then put empties before
    /// numerics (ascending) or after (descending).
    fn sort_decimal(&mut self, descending: bool, sep: DecimalSep, cx: &mut Context<Self>) {
        let Some(span) = self.line_span() else {
            return;
        };
        if !span.whole_buffer && span.first_line == span.last_line {
            return;
        }
        let lines = self.extract_lines(&span);
        let admissable: &[char] = match sep {
            DecimalSep::Comma => &[' ', '\t', '\r', '\n', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ',', '-'],
            DecimalSep::Dot => &[' ', '\t', '\r', '\n', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', '-'],
        };
        let mut numerics: Vec<(String, f64)> = Vec::new();
        let mut empties: Vec<String> = Vec::new();
        for line in lines {
            let prefix: String = line.chars().take_while(|c| admissable.contains(c)).collect();
            let normalized = match sep {
                DecimalSep::Comma => prefix.replace(',', "."),
                DecimalSep::Dot => prefix.clone(),
            };
            let trimmed = normalized.trim();
            if trimmed.is_empty() {
                empties.push(line);
                continue;
            }
            match trimmed.parse::<f64>() {
                Ok(n) => numerics.push((line, n)),
                Err(_) => empties.push(line),
            }
        }
        if descending {
            numerics.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            numerics.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        let mut output: Vec<String> = Vec::with_capacity(numerics.len() + empties.len());
        if descending {
            output.extend(numerics.into_iter().map(|(l, _)| l));
            output.extend(empties);
        } else {
            output.extend(empties);
            output.extend(numerics.into_iter().map(|(l, _)| l));
        }
        self.replace_lines(&span, output, cx);
    }

    // ---- Reverse / Randomize ---------------------------------------------------------------

    pub(crate) fn reverse_line_order(
        &mut self,
        _: &EditReverseLineOrder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let Some(span) = self.line_span() else {
            return;
        };
        let mut lines = self.extract_lines(&span);
        lines.reverse();
        self.replace_lines(&span, lines, cx);
    }

    pub(crate) fn randomize_line_order(
        &mut self,
        _: &EditRandomizeLineOrder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let Some(span) = self.line_span() else {
            return;
        };
        let mut lines = self.extract_lines(&span);
        fisher_yates_shuffle(&mut lines);
        self.replace_lines(&span, lines, cx);
    }

    // ---- Dedup -----------------------------------------------------------------------------

    pub(crate) fn remove_duplicate_lines(
        &mut self,
        _: &EditRemoveDuplicateLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let Some(span) = self.line_span() else {
            return;
        };
        // NPP gate: dedup needs at least two lines.
        if span.first_line == span.last_line {
            return;
        }
        let lines = self.extract_lines(&span);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(lines.len());
        for line in lines {
            if seen.insert(line.clone()) {
                out.push(line);
            }
        }
        self.replace_lines(&span, out, cx);
    }

    pub(crate) fn remove_consecutive_duplicate_lines(
        &mut self,
        _: &EditRemoveConsecutiveDuplicateLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let Some(span) = self.line_span() else {
            return;
        };
        if span.first_line == span.last_line {
            return;
        }
        let lines = self.extract_lines(&span);
        let mut out: Vec<String> = Vec::with_capacity(lines.len());
        for line in lines {
            if out.last().map(|l| l != &line).unwrap_or(true) {
                out.push(line);
            }
        }
        self.replace_lines(&span, out, cx);
    }

    // ---- Split / Join ----------------------------------------------------------------------

    pub(crate) fn split_lines(
        &mut self,
        _: &EditSplitLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        // 0 in NPP means "use current view width". Notepad Studio has no pane-width
        // accessor yet, so 0 in settings.toml falls back to 80 (NPP default).
        // Real values from the Preferences dialog flow through unchanged.
        let edge_raw = self.settings().split_lines_edge_column;
        let edge = if edge_raw == 0 { 80 } else { edge_raw };
        let Some(span) = self.line_span() else {
            return;
        };
        let lines = self.extract_lines(&span);
        let mut wrapped: Vec<String> = Vec::with_capacity(lines.len());
        for line in lines {
            wrap_line_at_column(&line, edge, &mut wrapped);
        }
        self.replace_lines(&span, wrapped, cx);
    }

    pub(crate) fn join_lines(
        &mut self,
        _: &EditJoinLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        // NPP only joins when the selection covers ≥2 lines. With no
        // selection, the op is a no-op.
        let sel = self.selection();
        if sel.is_empty() {
            return;
        }
        let Some(span) = self.line_span() else {
            return;
        };
        if span.first_line == span.last_line {
            return;
        }
        // Slice the selected line block, replace every newline run with a
        // single space, drop any trailing newline so the join leaves the
        // rest of the buffer alone.
        let rope = &self.buffer().rope;
        let original = rope.byte_slice(span.block_start..span.block_end).to_string();
        let trailing_newline = if span.has_terminator {
            self.buffer().line_ending.as_str().to_string()
        } else {
            String::new()
        };
        let body_end = original.len() - trailing_newline.len();
        let body = &original[..body_end];

        let mut joined = String::with_capacity(body.len());
        let mut chars = body.chars().peekable();
        let mut in_eol_run = false;
        while let Some(c) = chars.next() {
            if c == '\r' || c == '\n' {
                if !in_eol_run {
                    joined.push(' ');
                    in_eol_run = true;
                }
            } else {
                joined.push(c);
                in_eol_run = false;
            }
        }
        joined.push_str(&trailing_newline);
        self.replace_byte_range(span.block_start..span.block_end, &joined, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(span.block_start);
        tab.desired_col_x = None;
    }

    // ---- Empty-line removal ----------------------------------------------------------------

    pub(crate) fn remove_empty_lines(
        &mut self,
        _: &EditRemoveEmptyLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.remove_empty(false, cx);
    }

    pub(crate) fn remove_empty_lines_containing_blank(
        &mut self,
        _: &EditRemoveEmptyLinesContainingBlank,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.remove_empty(true, cx);
    }

    fn remove_empty(&mut self, blank_too: bool, cx: &mut Context<Self>) {
        let Some(span) = self.line_span() else {
            return;
        };
        let lines = self.extract_lines(&span);
        let kept: Vec<String> = lines
            .into_iter()
            .filter(|line| {
                if blank_too {
                    !line.chars().all(|c| c == ' ' || c == '\t')
                } else {
                    !line.is_empty()
                }
            })
            .collect();
        self.replace_lines(&span, kept, cx);
    }
}

#[derive(Clone, Copy)]
enum DecimalSep {
    Comma,
    Dot,
}

/// NPP's IntegerSorter is a *natural* sort — it walks two strings in lockstep,
/// comparing numeric runs as numbers (length-first, then digit-by-digit) and
/// non-numeric runs as text. Negative numbers are handled by detecting a `-`
/// followed by a digit. Leading-zero counts break ties so `"01"` sorts after
/// `"1"`.
fn natural_cmp(a: &String, b: &String) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let ab: Vec<char> = a.chars().collect();
    let bb: Vec<char> = b.chars().collect();
    let mut ai = 0usize;
    let mut bi = 0usize;
    loop {
        if ai >= ab.len() || bi >= bb.len() {
            // One side exhausted — fall through to a tail-compare on the
            // remaining slices. Match NPP's `a.compare(min(ai,len), npos, b,
            // min(bi,len), npos)`.
            let a_tail: String = ab[ai.min(ab.len())..].iter().collect();
            let b_tail: String = bb[bi.min(bb.len())..].iter().collect();
            return a_tail.cmp(&b_tail);
        }

        let mut a_is_num = ab[ai].is_ascii_digit();
        let mut b_is_num = bb[bi].is_ascii_digit();
        let mut a_sign = 1i32;
        let mut b_sign = 1i32;
        if !a_is_num && ai + 1 < ab.len() && ab[ai] == '-' && ab[ai + 1].is_ascii_digit() {
            a_is_num = true;
            a_sign = -1;
        }
        if !b_is_num && bi + 1 < bb.len() && bb[bi] == '-' && bb[bi + 1].is_ascii_digit() {
            b_is_num = true;
            b_sign = -1;
        }

        if a_is_num != b_is_num {
            // One is num, one isn't — NPP returns the raw char diff and
            // bumps both indices.
            let res = (ab[ai] as i32) - (bb[bi] as i32);
            ai += 1;
            bi += 1;
            if res < 0 {
                return Less;
            } else if res > 0 {
                return Greater;
            } else {
                continue;
            }
        } else if a_is_num {
            if a_sign != b_sign {
                return if a_sign == 1 { Greater } else { Less };
            }
            if a_sign == -1 {
                ai += 1;
                bi += 1;
            }
            // Skip leading zeros and remember how many we ate so equal-length
            // tail digit runs still tiebreak by zero-count.
            let mut a_zeros = 0;
            while ai < ab.len() && ab[ai] == '0' {
                a_zeros += 1;
                ai += 1;
            }
            let mut b_zeros = 0;
            while bi < bb.len() && bb[bi] == '0' {
                b_zeros += 1;
                bi += 1;
            }
            // Find end of digit run on each side.
            let mut a_end = ai;
            while a_end < ab.len() && ab[a_end].is_ascii_digit() {
                a_end += 1;
            }
            let mut b_end = bi;
            while b_end < bb.len() && bb[b_end].is_ascii_digit() {
                b_end += 1;
            }
            let a_len = a_end - ai;
            let b_len = b_end - bi;
            if a_len != b_len {
                let cmp = a_len.cmp(&b_len);
                return if a_sign == -1 { cmp.reverse() } else { cmp };
            }
            // Equal-length digit runs — compare digit by digit.
            let mut digit_cmp = Equal;
            for _ in 0..a_len {
                let d = (ab[ai] as i32) - (bb[bi] as i32);
                ai += 1;
                bi += 1;
                if d != 0 {
                    digit_cmp = if d < 0 { Less } else { Greater };
                    if a_sign == -1 {
                        digit_cmp = digit_cmp.reverse();
                    }
                    break;
                }
            }
            if digit_cmp != Equal {
                // Skip remaining digits so the next iteration realigns.
                while ai < ab.len() && ab[ai].is_ascii_digit() {
                    ai += 1;
                }
                while bi < bb.len() && bb[bi].is_ascii_digit() {
                    bi += 1;
                }
                return digit_cmp;
            }
            // Equal digit runs — break tie on leading-zero count.
            if a_zeros != b_zeros {
                return b_zeros.cmp(&a_zeros);
            }
            // Otherwise loop continues into the post-number region.
        } else {
            // Both are non-numeric runs. Skip a leading `-` on each and
            // compare the chunk up to the next digit / minus.
            if ab[ai] == '-' {
                ai += 1;
            }
            if bb[bi] == '-' {
                bi += 1;
            }
            let mut a_end = ai;
            while a_end < ab.len()
                && !ab[a_end].is_ascii_digit()
                && ab[a_end] != '-'
            {
                a_end += 1;
            }
            let mut b_end = bi;
            while b_end < bb.len()
                && !bb[b_end].is_ascii_digit()
                && bb[b_end] != '-'
            {
                b_end += 1;
            }
            let a_chunk: String = ab[ai..a_end].iter().collect();
            let b_chunk: String = bb[bi..b_end].iter().collect();
            ai = a_end;
            bi = b_end;
            let chunk_cmp = a_chunk.cmp(&b_chunk);
            if chunk_cmp != Equal {
                return chunk_cmp;
            }
        }
    }
}

/// Wrap one logical line at `edge` columns, NPP `Editor::LinesSplit` style:
/// look back for the nearest whitespace within ~20% of `edge`; if none, hard
/// break at the column. Empty / short lines pass through unchanged. Column
/// is char-count (Unicode scalars), not bytes — matches Scintilla's
/// pre-pixel-aware column model.
fn wrap_line_at_column(line: &str, edge: usize, out: &mut Vec<String>) {
    if line.chars().count() <= edge {
        out.push(line.to_string());
        return;
    }
    let lookback = (edge / 5).max(1);
    let chars: Vec<char> = line.chars().collect();
    let mut start = 0;
    while chars.len() - start > edge {
        let hard_break = start + edge;
        let mut break_at = hard_break;
        let lookback_floor = hard_break.saturating_sub(lookback).max(start + 1);
        for i in (lookback_floor..hard_break).rev() {
            if chars[i].is_whitespace() {
                break_at = i + 1;
                break;
            }
        }
        out.push(chars[start..break_at].iter().collect());
        // Skip the whitespace we broke on so the next segment doesn't start
        // with the space — matches NPP's wrap output.
        start = break_at;
        while start < chars.len() && chars[start] == ' ' {
            start += 1;
        }
    }
    if start < chars.len() {
        out.push(chars[start..].iter().collect());
    }
}

/// Fisher-Yates shuffle seeded from the system clock. Avoids pulling in
/// `rand` for one op. Same shape as NPP's `RandomSorter` (which seeds
/// `default_random_engine` from `time(NULL)`).
fn fisher_yates_shuffle<T>(slice: &mut [T]) {
    use std::time::SystemTime;
    let mut state: u64 = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| {
            // Mix nanos and secs so two shuffles in the same second still
            // diverge.
            d.as_nanos() as u64 ^ d.as_secs().wrapping_mul(0x9E3779B97F4A7C15)
        })
        .unwrap_or(0xDEADBEEFCAFEF00D);
    if state == 0 {
        state = 0xA5A5_5A5A_A5A5_5A5A;
    }
    let n = slice.len();
    for i in (1..n).rev() {
        // xorshift64 — fine for shuffling line order.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state as usize) % (i + 1);
        slice.swap(i, j);
    }
}
