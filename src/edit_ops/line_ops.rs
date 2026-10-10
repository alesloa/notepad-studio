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
//   Notepad_plus.cpp:1759                 — removeDuplicateLines (consecutive)
//   ScintillaEditView.cpp:4474            — sortLines
//   ScintillaEditView.cpp:4806            — removeAnyDuplicateLines
//
// All ops operate on the line range covered by the selection if a selection
// exists, else on the whole buffer. NPP's selection-line-range rule: if the
// selection ends exactly at the start of line N (i.e. just past line N-1's
// terminator), line N is *not* included. Sort and dedup additionally bail
// when the selection covers only a single line. Each op produces one
// `replace_byte_range` call so it lands as a single undo entry.
//
// Line ends: each op works on the block's own terminators (`edit_ops::util`)
// and never inserts the buffer's on-disk EOL, because the rope is
// LF-internal. Sort, reverse, randomize and any-dedupe follow NPP `sortLines`
// / `removeAnyDuplicateLines`: line i of the result takes the terminator line
// i had, and a block that reaches the end of the document keeps its final
// (possibly empty) line in the set, so the block never grows or gains a line.

use std::cmp::Ordering;
use std::ops::Range;

use gpui::{Context, Window};
use ropey::Rope;

use crate::actions::*;
use crate::edit_ops::util::{first_terminator, split_line_terminator, split_lines_eol};
use crate::tab::InputFocus;
use crate::workspace::Workspace;

/// Span of full lines selected for a line op, plus the byte offsets that
/// frame those lines. `whole_buffer` is `true` when the user had no
/// selection — NPP uses this to gate single-line-only sort early-outs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSpan {
    /// Inclusive line index of the first selected line.
    first_line: usize,
    /// Inclusive line index of the last selected line.
    last_line: usize,
    /// Byte offset of the start of `first_line`.
    block_start: usize,
    /// Byte offset just past `last_line`'s terminator (or buffer end if none).
    block_end: usize,
    /// True when the user had no selection — op covers the whole buffer.
    whole_buffer: bool,
    /// True when `last_line` is the document's last line (NPP
    /// `sortEntireDocument`): the block's final line stays in the set even
    /// when it is the empty line after a trailing line end.
    reaches_doc_end: bool,
    /// True when the byte before `block_start` is a lone CR, so a result that
    /// starts with LF would fuse with it into one CRLF.
    after_cr: bool,
}

/// Compute the line span for `selection` (`None` = no selection). Selection
/// ends past the end of the rope are clamped, so a stale selection can't
/// panic `byte_to_line`. Returns `None` for an empty block.
fn line_span_in(rope: &Rope, selection: Option<Range<usize>>) -> Option<LineSpan> {
    let total_lines = rope.len_lines();
    if total_lines == 0 {
        return None;
    }
    let total_bytes = rope.len_bytes();

    let (first_line, last_line, whole_buffer) = match selection {
        None => (0, total_lines.saturating_sub(1), true),
        Some(r) => {
            let start = r.start.min(r.end).min(total_bytes);
            let end = r.start.max(r.end).min(total_bytes);
            let first = rope.byte_to_line(start);
            let last_raw = rope.byte_to_line(end);
            // NPP rule: a selection that ends exactly at the start of a line
            // does not include that line.
            let last = if end > start && last_raw > first && rope.line_to_byte(last_raw) == end {
                last_raw - 1
            } else {
                last_raw
            };
            (first, last, false)
        }
    };

    // ropey also ends lines at VT, FF, NEL, LS and PS; the ops split only on
    // CR and LF, as Notepad++ does. Widen the block to whole CR/LF lines so
    // it never cuts one: a cut line's tail would fuse with whatever the op
    // moved last.
    let block_start = crlf_line_start(rope, rope.line_to_byte(first_line));
    let rope_end = if last_line + 1 < total_lines {
        rope.line_to_byte(last_line + 1)
    } else {
        total_bytes
    };
    let block_end = crlf_line_end(rope, rope_end);
    if block_start >= block_end {
        return None;
    }
    let reaches_doc_end = if block_end == rope_end {
        last_line + 1 >= total_lines
    } else {
        // Widened: the block's last line is the document's last line only
        // when no line end follows it.
        block_end == total_bytes
            && !matches!(rope.get_byte(block_end.saturating_sub(1)), Some(b'\n' | b'\r'))
    };
    let after_cr = block_start
        .checked_sub(1)
        .and_then(|i| rope.get_byte(i))
        == Some(b'\r');
    Some(LineSpan {
        first_line,
        last_line,
        block_start,
        block_end,
        whole_buffer,
        reaches_doc_end,
        after_cr,
    })
}

/// `at` moved back to the start of its CR/LF line.
fn crlf_line_start(rope: &Rope, at: usize) -> usize {
    let Some(mut bytes) = rope.get_bytes_at(at) else {
        return at;
    };
    let mut start = at;
    while let Some(b) = bytes.prev() {
        if b == b'\n' || b == b'\r' {
            break;
        }
        start = start.saturating_sub(1);
    }
    start
}

/// `at` moved forward past the end of its CR/LF line, terminator included.
/// An offset that already follows a CR or LF is a line start and stays.
fn crlf_line_end(rope: &Rope, at: usize) -> usize {
    let Some(bytes) = rope.get_bytes_at(at) else {
        return at;
    };
    let Some(before) = at.checked_sub(1) else {
        return at;
    };
    if matches!(rope.get_byte(before), Some(b'\n' | b'\r')) {
        return at;
    }
    let mut end = at;
    let mut prev_cr = false;
    for b in bytes {
        if prev_cr {
            if b == b'\n' {
                end += 1;
            }
            break;
        }
        end += 1;
        if b == b'\n' {
            break;
        }
        prev_cr = b == b'\r';
    }
    end
}

/// The lines a permuting or filtering op works on. Like NPP, the empty piece
/// after the block's last line end is dropped unless the block reaches the
/// end of the document.
fn block_pieces(block: &str, reaches_doc_end: bool) -> Vec<(&str, &str)> {
    let mut pieces = split_lines_eol(block);
    if !reaches_doc_end && pieces.len() > 1 && pieces.last() == Some(&("", "")) {
        pieces.pop();
    }
    pieces
}

/// Rebuild a block from `bodies` after a permutation or filter: line i takes
/// terminator i, the last line takes the block's last terminator. Returns
/// `None` when a lone CR would land right before an empty LF line (or after
/// a CR that precedes the block): the pair would read back as one CRLF and
/// swallow a line. Only a block that mixes CR and LF line ends can hit this.
fn rebuild_positional(bodies: &[&str], eols: &[&str], after_cr: bool) -> Option<String> {
    let n = bodies.len();
    let mut out = String::with_capacity(bodies.iter().map(|b| b.len() + 2).sum());
    let mut prev_cr = after_cr;
    for (i, body) in bodies.iter().enumerate() {
        let eol = if i + 1 == n { eols.last() } else { eols.get(i) };
        let eol = *eol?;
        if prev_cr && body.is_empty() && eol.starts_with('\n') {
            return None;
        }
        out.push_str(body);
        out.push_str(eol);
        if !(body.is_empty() && eol.is_empty()) {
            prev_cr = eol == "\r";
        }
    }
    Some(out)
}

/// Rebuild after `reorder` permutes the bodies of `block`.
fn permute_block(
    block: &str,
    span: &LineSpan,
    reorder: impl FnOnce(&mut Vec<&str>),
) -> Option<String> {
    let pieces = block_pieces(block, span.reaches_doc_end);
    let eols: Vec<&str> = pieces.iter().map(|p| p.1).collect();
    let mut bodies: Vec<&str> = pieces.iter().map(|p| p.0).collect();
    reorder(&mut bodies);
    rebuild_positional(&bodies, &eols, span.after_cr)
}

fn sort_block(
    block: &str,
    span: &LineSpan,
    descending: bool,
    mut cmp: impl FnMut(&str, &str) -> Ordering,
) -> Option<String> {
    permute_block(block, span, |bodies| {
        if descending {
            merge_sort_by(bodies, |a, b| cmp(b, a));
        } else {
            merge_sort_by(bodies, |a, b| cmp(a, b));
        }
    })
}

/// NPP `NumericSorter::sort` shape: filter into "empty after preparation"
/// vs "parses as f64", stable-sort the numerics, then put empties before
/// numerics (ascending) or after (descending).
fn sort_decimal_block(
    block: &str,
    span: &LineSpan,
    descending: bool,
    sep: DecimalSep,
) -> Option<String> {
    permute_block(block, span, |bodies| {
        let admissable: &[char] = match sep {
            DecimalSep::Comma => &[' ', '\t', '\r', '\n', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ',', '-'],
            DecimalSep::Dot => &[' ', '\t', '\r', '\n', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', '-'],
        };
        let mut numerics: Vec<(&str, f64)> = Vec::new();
        let mut empties: Vec<&str> = Vec::new();
        for line in bodies.iter().copied() {
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
            merge_sort_by(&mut numerics, |a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        } else {
            merge_sort_by(&mut numerics, |a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        }
        bodies.clear();
        if descending {
            bodies.extend(numerics.into_iter().map(|(l, _)| l));
            bodies.extend(empties);
        } else {
            bodies.extend(empties);
            bodies.extend(numerics.into_iter().map(|(l, _)| l));
        }
    })
}

fn reverse_block(block: &str, span: &LineSpan) -> Option<String> {
    permute_block(block, span, |bodies| bodies.reverse())
}

fn shuffle_block(block: &str, span: &LineSpan) -> Option<String> {
    permute_block(block, span, |bodies| fisher_yates_shuffle(bodies))
}

/// NPP `removeAnyDuplicateLines`: keep the first copy of each line.
fn dedupe_any_block(block: &str, span: &LineSpan) -> Option<String> {
    let pieces = block_pieces(block, span.reaches_doc_end);
    let eols: Vec<&str> = pieces.iter().map(|p| p.1).collect();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let bodies: Vec<&str> = pieces
        .iter()
        .map(|p| p.0)
        .filter(|body| seen.insert(*body))
        .collect();
    rebuild_positional(&bodies, &eols, span.after_cr)
}

/// NPP `removeDuplicateLines`: drop a line equal to the one above it. NPP
/// compares lines with their line end, so the last line (no line end) never
/// matches the one before it. Kept lines keep their own terminator.
fn dedupe_consecutive_block(block: &str, span: &LineSpan) -> String {
    let pieces = block_pieces(block, span.reaches_doc_end);
    let mut out = String::with_capacity(block.len());
    let mut prev: Option<(&str, &str)> = None;
    for piece in pieces {
        if prev == Some(piece) {
            continue;
        }
        out.push_str(piece.0);
        out.push_str(piece.1);
        prev = Some(piece);
    }
    out
}

/// Wrap every line of the block at `edge` columns. New breaks inside a line
/// reuse that line's terminator (or the block's first one, for a last line
/// that has none).
fn split_block(block: &str, edge: usize) -> String {
    let fallback = first_terminator(block);
    let mut out = String::with_capacity(block.len());
    for (body, eol) in split_lines_eol(block) {
        let mut segments = Vec::new();
        wrap_line_at_column(body, edge, &mut segments);
        let inner = if eol.is_empty() { fallback } else { eol };
        let n = segments.len();
        for (i, seg) in segments.iter().enumerate() {
            out.push_str(seg);
            if i + 1 < n {
                out.push_str(inner);
            }
        }
        out.push_str(eol);
    }
    out
}

/// Join the block's lines: every line-end run becomes one space. The block's
/// own final line end (if any) is kept so the rest of the buffer is left
/// alone.
fn join_block(block: &str) -> String {
    let (body, trailing) = split_line_terminator(block);
    let mut joined = String::with_capacity(block.len());
    let mut in_eol_run = false;
    for c in body.chars() {
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
    joined.push_str(trailing);
    joined
}

/// Drop empty lines (or, with `blank_too`, lines of only spaces and tabs).
/// Kept lines keep their own terminator; the last kept line takes the
/// block's final line end, so the block's trailing-newline state survives.
fn remove_empty_block(block: &str, blank_too: bool) -> String {
    let (_, trailing) = split_line_terminator(block);
    let kept: Vec<(&str, &str)> = split_lines_eol(block)
        .into_iter()
        .filter(|(body, _)| {
            if blank_too {
                !body.chars().all(|c| c == ' ' || c == '\t')
            } else {
                !body.is_empty()
            }
        })
        .collect();
    let n = kept.len();
    let mut out = String::with_capacity(block.len());
    for (i, (body, eol)) in kept.into_iter().enumerate() {
        out.push_str(body);
        out.push_str(if i + 1 == n { trailing } else { eol });
    }
    out
}

impl Workspace {
    /// Compute the line span the op should act on. Returns `None` if the
    /// buffer is empty.
    fn line_span(&self) -> Option<LineSpan> {
        let sel = self.selection();
        let selection = (!sel.is_empty()).then(|| sel.ordered());
        line_span_in(&self.buffer().rope, selection)
    }

    /// Run `op` over the selected line block and replace the block with its
    /// result. `op` returns `None` to leave the buffer alone.
    fn run_line_op(
        &mut self,
        cx: &mut Context<Self>,
        op: impl FnOnce(&str, &LineSpan) -> LineOpResult,
    ) {
        let Some(span) = self.line_span() else {
            return;
        };
        let Some(block) = self
            .buffer()
            .rope
            .get_byte_slice(span.block_start..span.block_end)
            .map(|s| s.to_string())
        else {
            return;
        };
        let new_block = match op(&block, &span) {
            LineOpResult::Replace(text) => text,
            LineOpResult::Skip => return,
            LineOpResult::MixedLineEnds => {
                self.set_status(
                    "Line operation skipped: the selection mixes CR and LF line endings \
                     and the result would merge two lines",
                );
                cx.notify();
                return;
            }
        };
        self.replace_byte_range(span.block_start..span.block_end, &new_block, cx);
        let tab = self.active_tab_mut();
        tab.selection.collapse_to(span.block_start);
        tab.desired_col_x = None;
    }

    // ---- Sort variants ---------------------------------------------------------------------

    fn sort_with(
        &mut self,
        descending: bool,
        cmp: fn(&str, &str) -> Ordering,
        cx: &mut Context<Self>,
    ) {
        self.run_line_op(cx, |block, span| {
            // NPP gates sort to multi-line selections only when a selection
            // exists (whole-buffer sort is fine even for single-line files —
            // the sort is a no-op but harmless).
            if !span.whole_buffer && span.first_line == span.last_line {
                return LineOpResult::Skip;
            }
            sort_block(block, span, descending, cmp).into()
        });
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

    fn sort_decimal(&mut self, descending: bool, sep: DecimalSep, cx: &mut Context<Self>) {
        self.run_line_op(cx, |block, span| {
            if !span.whole_buffer && span.first_line == span.last_line {
                return LineOpResult::Skip;
            }
            sort_decimal_block(block, span, descending, sep).into()
        });
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
        self.run_line_op(cx, |block, span| reverse_block(block, span).into());
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
        self.run_line_op(cx, |block, span| shuffle_block(block, span).into());
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
        self.run_line_op(cx, |block, span| {
            // NPP gate: dedup needs at least two lines.
            if span.first_line == span.last_line {
                return LineOpResult::Skip;
            }
            dedupe_any_block(block, span).into()
        });
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
        self.run_line_op(cx, |block, span| {
            if span.first_line == span.last_line {
                return LineOpResult::Skip;
            }
            LineOpResult::Replace(dedupe_consecutive_block(block, span))
        });
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
        self.run_line_op(cx, |block, _| LineOpResult::Replace(split_block(block, edge)));
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
        if self.selection().is_empty() {
            return;
        }
        self.run_line_op(cx, |block, span| {
            if span.first_line == span.last_line {
                return LineOpResult::Skip;
            }
            LineOpResult::Replace(join_block(block))
        });
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
        self.run_line_op(cx, |block, _| LineOpResult::Replace(remove_empty_block(block, false)));
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
        self.run_line_op(cx, |block, _| LineOpResult::Replace(remove_empty_block(block, true)));
    }
}

/// What a line op decided to do with the block.
enum LineOpResult {
    Replace(String),
    /// NPP gate (single-line selection etc.): leave the buffer alone.
    Skip,
    /// The rebuilt block would fuse a lone CR with an LF; leave it alone and
    /// tell the user why.
    MixedLineEnds,
}

impl From<Option<String>> for LineOpResult {
    fn from(result: Option<String>) -> Self {
        match result {
            Some(text) => LineOpResult::Replace(text),
            None => LineOpResult::MixedLineEnds,
        }
    }
}

#[derive(Clone, Copy)]
enum DecimalSep {
    Comma,
    Dot,
}

/// Stable bottom-up merge sort. std's `sort_by` may panic when the
/// comparator is not a total order (Rust 1.81+), and `natural_cmp` mirrors
/// NPP's IntegerSorter, which is not one once `-` is involved (`"-."` equals
/// `"."`, yet `"-." < "-1" < "."`). This sort only ever compares and moves,
/// so any comparator yields some permutation of the input.
fn merge_sort_by<T: Copy>(items: &mut Vec<T>, mut cmp: impl FnMut(&T, &T) -> Ordering) {
    let mut width = 1usize;
    while width < items.len() {
        let mut merged = Vec::with_capacity(items.len());
        for chunk in items.chunks(width.saturating_mul(2)) {
            let (left, right) = chunk.split_at(width.min(chunk.len()));
            let mut l = left.iter().peekable();
            let mut r = right.iter().peekable();
            loop {
                match (l.peek(), r.peek()) {
                    (Some(a), Some(b)) => {
                        // Take from the right only when strictly less: stable.
                        if cmp(b, a) == Ordering::Less {
                            merged.push(**b);
                            r.next();
                        } else {
                            merged.push(**a);
                            l.next();
                        }
                    }
                    (Some(a), None) => {
                        merged.push(**a);
                        l.next();
                    }
                    (None, Some(b)) => {
                        merged.push(**b);
                        r.next();
                    }
                    (None, None) => break,
                }
            }
        }
        *items = merged;
        width = width.saturating_mul(2);
    }
}

/// NPP's IntegerSorter is a *natural* sort — it walks two strings in lockstep,
/// comparing numeric runs as numbers (length-first, then digit-by-digit) and
/// non-numeric runs as text. Negative numbers are handled by detecting a `-`
/// followed by a digit. Leading-zero counts break ties so `"01"` sorts after
/// `"1"`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Rope contents every op runs over.
    const INPUTS: &[&str] = &[
        "",
        "abc",
        "b\na\nc",
        "b\na\nc\n",
        "b\r\na\r\nc\r\n",
        "b\ra\rc\r",
        "b\r\na\nc\rd",
        "a\r\n\nb\r\r\n",
        "a\rb\n\n",
        "x\ra\n\nz",
        "é\n中\n👨‍👩‍👧\n🇯🇵🇫🇷\nß İ\n",
        "a\n \t \nb\n",
        "x\n\n\nx\n",
        "10\n-2\n3,5\n3.5\n-.\n.\n-1\n",
        "aaaa bbbb cccc\r\n中中中中中中\rdd",
        // ropey also breaks lines on VT, FF, NEL, LS and PS.
        "b\u{2028}a\nc\u{85}d\u{b}e\u{c}f\u{2029}g\n",
        "\u{2028}\r\n\u{85}",
    ];

    type Op = fn(&str, &LineSpan) -> Option<String>;

    fn ops() -> Vec<(&'static str, Op, bool)> {
        // (name, op, permutes): a permuting op keeps every line and terminator.
        vec![
            ("sort_lex_asc", |b, s| sort_block(b, s, false, |x, y| x.cmp(y)), true),
            ("sort_lex_desc", |b, s| sort_block(b, s, true, |x, y| x.cmp(y)), true),
            ("sort_int_asc", |b, s| sort_block(b, s, false, natural_cmp), true),
            ("sort_int_desc", |b, s| sort_block(b, s, true, natural_cmp), true),
            ("sort_dec_comma_asc", |b, s| sort_decimal_block(b, s, false, DecimalSep::Comma), true),
            ("sort_dec_comma_desc", |b, s| sort_decimal_block(b, s, true, DecimalSep::Comma), true),
            ("sort_dec_dot_asc", |b, s| sort_decimal_block(b, s, false, DecimalSep::Dot), true),
            ("sort_dec_dot_desc", |b, s| sort_decimal_block(b, s, true, DecimalSep::Dot), true),
            ("reverse", reverse_block, true),
            ("shuffle", shuffle_block, true),
            ("dedupe_any", dedupe_any_block, false),
            ("dedupe_consecutive", |b, s| Some(dedupe_consecutive_block(b, s)), false),
            ("split_80", |b, _| Some(split_block(b, 80)), false),
            ("split_3", |b, _| Some(split_block(b, 3)), false),
            ("split_1", |b, _| Some(split_block(b, 1)), false),
            ("join", |b, _| Some(join_block(b)), false),
            ("remove_empty", |b, _| Some(remove_empty_block(b, false)), false),
            ("remove_blank", |b, _| Some(remove_empty_block(b, true)), false),
        ]
    }

    /// Selections to try on `text`: none, whole document, ending at column 0
    /// of line 1, caret at EOF, mid-char, and stale past EOF.
    fn selections(rope: &Rope) -> Vec<Option<Range<usize>>> {
        let len = rope.len_bytes();
        let mut out = vec![None, Some(0..len), Some(len..len), Some(len + 3..len + 9), Some(len..0)];
        if rope.len_lines() > 1 {
            out.push(Some(0..rope.line_to_byte(1)));
        }
        if rope.len_lines() > 2 {
            out.push(Some(rope.line_to_byte(1)..rope.line_to_byte(2) + 1));
        }
        if len > 2 {
            out.push(Some(1..len - 1));
        }
        out
    }

    fn eol_kinds(text: &str) -> BTreeSet<&str> {
        split_lines_eol(text).into_iter().map(|p| p.1).filter(|e| !e.is_empty()).collect()
    }

    fn sorted_pieces<'a>(text: &'a str, span: &LineSpan) -> (Vec<&'a str>, Vec<&'a str>) {
        let pieces = block_pieces(text, span.reaches_doc_end);
        let mut bodies: Vec<&str> = pieces.iter().map(|p| p.0).collect();
        let mut eols: Vec<&str> = pieces.iter().map(|p| p.1).collect();
        bodies.sort();
        eols.sort();
        (bodies, eols)
    }

    fn span_of(text: &str, sel: Option<Range<usize>>) -> (Rope, LineSpan, String) {
        let rope = Rope::from_str(text);
        let span = line_span_in(&rope, sel).expect("non-empty block");
        let block = rope
            .get_byte_slice(span.block_start..span.block_end)
            .expect("block sits on line starts")
            .to_string();
        (rope, span, block)
    }

    #[test]
    fn every_op_keeps_text_valid_and_line_ends_intact() {
        for input in INPUTS {
            let rope = Rope::from_str(input);
            for sel in selections(&rope) {
                let Some(span) = line_span_in(&rope, sel.clone()) else {
                    // Only an empty buffer, or a selection inside the empty
                    // line after a trailing line end, has no lines to act on.
                    let last_start = rope.line_to_byte(rope.len_lines() - 1);
                    let sel_start = sel.as_ref().map_or(0, |r| r.start.min(r.end).min(rope.len_bytes()));
                    assert!(
                        input.is_empty() || (last_start == rope.len_bytes() && sel_start == last_start),
                        "no span for {input:?} {sel:?}"
                    );
                    continue;
                };
                let block = rope
                    .get_byte_slice(span.block_start..span.block_end)
                    .expect("block sits on line starts")
                    .to_string();
                for (name, op, permutes) in ops() {
                    let ctx = format!("{name} on {input:?} sel {sel:?}");
                    let Some(out) = op(&block, &span) else {
                        // Only a block that mixes a lone CR with LF may bail.
                        let has_lone_cr = split_lines_eol(&block).iter().any(|p| p.1 == "\r")
                            || span.after_cr;
                        assert!(has_lone_cr && block.contains('\n'), "{ctx}: bailed");
                        continue;
                    };
                    let mut spliced = String::new();
                    spliced.push_str(&input[..span.block_start]);
                    spliced.push_str(&out);
                    spliced.push_str(&input[span.block_end..]);
                    let new_rope = Rope::from_str(&spliced);
                    assert_eq!(new_rope.len_bytes(), spliced.len(), "{ctx}");

                    let before = eol_kinds(&block);
                    let after = eol_kinds(&out);
                    if before.is_empty() {
                        assert!(after.iter().all(|e| *e == "\n"), "{ctx}: {after:?}");
                    } else {
                        assert!(after.is_subset(&before), "{ctx}: {after:?} not in {before:?}");
                    }
                    if !span.reaches_doc_end && !out.is_empty() {
                        assert_eq!(
                            split_line_terminator(&out).1,
                            split_line_terminator(&block).1,
                            "{ctx}: block end changed"
                        );
                    }
                    if permutes {
                        assert_eq!(out.len(), block.len(), "{ctx}");
                        assert_eq!(new_rope.len_lines(), rope.len_lines(), "{ctx}");
                        assert_eq!(sorted_pieces(&out, &span), sorted_pieces(&block, &span), "{ctx}");
                    }
                }
            }
        }
    }

    #[test]
    fn line_span_clamps_stale_selection() {
        // Selection left over from a longer buffer: used to panic in
        // `byte_to_line`.
        let rope = Rope::from_str("ab\ncd");
        let span = line_span_in(&rope, Some(10..50)).expect("span");
        assert_eq!((span.first_line, span.last_line), (1, 1));
        assert_eq!((span.block_start, span.block_end), (3, 5));
        let span = line_span_in(&rope, Some(1..50)).expect("span");
        assert_eq!((span.first_line, span.last_line), (0, 1));
    }

    #[test]
    fn block_never_cuts_a_line_at_a_unicode_line_break() {
        // ropey ends a line at NEL, so the block used to stop after
        // "a\u{85}" and sorting fused "b" onto "rest".
        let text = "x\nb\na\u{85}rest";
        let (_, span, block) = span_of(text, Some(2..7));
        assert_eq!(block, "b\na\u{85}rest");
        assert!(span.reaches_doc_end);
        let out = sort_block(&block, &span, false, |x, y| x.cmp(y)).expect("sorted");
        assert_eq!(out, "a\u{85}rest\nb");

        // The start snaps back the same way, past LS, VT and FF.
        let text = "p\u{b}q\u{2028}r\u{c}s\nt\n";
        let (_, span, block) = span_of(text, Some(6..12));
        assert_eq!(block, "p\u{b}q\u{2028}r\u{c}s\nt\n");
        assert!(!span.reaches_doc_end);
        assert!(!span.after_cr);
        // A block that already sits on CR/LF line starts is unchanged.
        let (_, span, block) = span_of("a\r\nb\u{2029}c\rd", Some(0..3));
        assert_eq!((span.block_start, span.block_end), (0, 3));
        assert_eq!(block, "a\r\n");
    }

    #[test]
    fn line_span_excludes_line_at_column_zero() {
        let rope = Rope::from_str("a\r\nb\r\nc");
        let span = line_span_in(&rope, Some(0..3)).expect("span");
        assert_eq!((span.first_line, span.last_line), (0, 0));
        assert_eq!((span.block_start, span.block_end), (0, 3));
        assert!(!span.reaches_doc_end);
        assert!(line_span_in(&Rope::new(), None).is_none());
    }

    #[test]
    fn join_crlf_block_with_multibyte_last_char() {
        // Used to slice the block at `len - buffer_eol.len()`, which cut into
        // `é` when the buffer's EOL was CRLF but the rope held LF.
        let (_, span, block) = span_of("a\né\n", Some(0..5));
        assert_eq!((span.first_line, span.last_line), (0, 1));
        assert_eq!(join_block(&block), "a é\n");
        assert_eq!(join_block("a\r\nb\r\n\r\nc"), "a b c");
        assert_eq!(join_block("a\rb\r"), "a b\r");
    }

    #[test]
    fn int_sort_survives_inconsistent_comparator() {
        // `natural_cmp` is not a total order ("-." == "." but
        // "-." < "-1" < "."). std `sort_by` panics on this input.
        let lines = [
            "1a", ".", "1", "a", "-.1", "1", "-0", "1", "-1", "-a", "-0", "1a", "-.", "a", "1",
            "-", "a1", "-.", "-a", "a", "a", "-a", "a", "-.",
        ];
        let text = lines.join("\n");
        let (_, span, block) = span_of(&text, None);
        let out = sort_block(&block, &span, false, natural_cmp).expect("no CR");
        let mut got: Vec<&str> = out.split('\n').collect();
        let mut want = lines.to_vec();
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn merge_sort_is_stable_and_sorted() {
        let mut items: Vec<(u8, usize)> = [3u8, 1, 2, 3, 1, 0, 2, 3, 1]
            .iter()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();
        let mut want = items.clone();
        want.sort_by_key(|p| p.0);
        merge_sort_by(&mut items, |a, b| a.0.cmp(&b.0));
        assert_eq!(items, want);
        let mut empty: Vec<u8> = Vec::new();
        merge_sort_by(&mut empty, |a, b| a.cmp(b));
        assert!(empty.is_empty());
    }

    #[test]
    fn sort_matches_notepad_plus_plus() {
        // Whole document: the empty last line joins the sort and no line end
        // is added (NPP `sortLines`, toLine == lineCount - 1).
        let (_, span, block) = span_of("b\na\n", None);
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)).as_deref(), Some("\na\nb"));
        // A selection keeps its trailing line end in place.
        let (_, span, block) = span_of("b\na\nc\n", Some(0..4));
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)).as_deref(), Some("a\nb\n"));
        // Raw CRLF stays CRLF; it is never swapped for the buffer's EOL.
        let (_, span, block) = span_of("b\r\na\r\nz", Some(0..6));
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)).as_deref(), Some("a\r\nb\r\n"));
        // Line ends stay where they were.
        let (_, span, block) = span_of("c\r\nb\na\r", None);
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)).as_deref(), Some("\r\na\nb\rc"));
        let (_, span, block) = span_of("10\n9\n-2\n1", None);
        assert_eq!(sort_block(&block, &span, false, natural_cmp).as_deref(), Some("-2\n1\n9\n10"));
        let (_, span, block) = span_of("1,5\nx\n-3\n1,25", None);
        assert_eq!(
            sort_decimal_block(&block, &span, false, DecimalSep::Comma).as_deref(),
            Some("x\n-3\n1,25\n1,5")
        );
        let (_, span, block) = span_of("a\nb\n", None);
        assert_eq!(reverse_block(&block, &span).as_deref(), Some("\nb\na"));
    }

    #[test]
    fn permutation_that_would_merge_cr_and_lf_is_refused() {
        // "a\r" then an empty LF line would read back as one CRLF.
        let (_, span, block) = span_of("a\rb\n\n", None);
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)), None);
        // Same at the block's start, after a lone CR above the selection.
        let (_, span, block) = span_of("x\ra\n\nz", Some(2..5));
        assert!(span.after_cr);
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)), None);
        // Without the CR the same sort goes through.
        let (_, span, block) = span_of("x\na\n\nz", Some(2..5));
        assert_eq!(sort_block(&block, &span, false, |x, y| x.cmp(y)).as_deref(), Some("\na\n"));
    }

    #[test]
    fn dedupe_matches_notepad_plus_plus() {
        let (_, span, block) = span_of("a\na\n", None);
        assert_eq!(dedupe_any_block(&block, &span).as_deref(), Some("a\n"));
        let (_, span, block) = span_of("b\r\na\r\nb\r\nz", Some(0..9));
        assert_eq!(dedupe_any_block(&block, &span).as_deref(), Some("b\r\na\r\n"));
        // Consecutive dedupe compares line ends too: the last line has none.
        let (_, span, block) = span_of("a\na", None);
        assert_eq!(dedupe_consecutive_block(&block, &span), "a\na");
        let (_, span, block) = span_of("a\n\n\n", None);
        assert_eq!(dedupe_consecutive_block(&block, &span), "a\n\n");
        let (_, span, block) = span_of("é\r\né\r\n中", None);
        assert_eq!(dedupe_consecutive_block(&block, &span), "é\r\n中");
    }

    #[test]
    fn split_reuses_the_line_terminator() {
        assert_eq!(split_block("aaaa bbbb\r\ncc", 5), "aaaa \r\nbbbb\r\ncc");
        assert_eq!(split_block("aaaaaa", 3), "aaa\naaa");
        assert_eq!(split_block("x\rcccccc", 3), "x\rccc\rccc");
        assert_eq!(split_block("中中中中", 2), "中中\n中中");
        assert_eq!(split_block("", 3), "");
    }

    #[test]
    fn remove_empty_keeps_the_block_end() {
        assert_eq!(remove_empty_block("a\n\n\nb\n", false), "a\nb\n");
        assert_eq!(remove_empty_block("a\r\n\r\nb", false), "a\r\nb");
        assert_eq!(remove_empty_block("a\n \t\nb", false), "a\n \t\nb");
        assert_eq!(remove_empty_block("a\n \t\nb", true), "a\nb");
        // A block of only empty lines goes away entirely.
        assert_eq!(remove_empty_block("\n\n", false), "");
        assert_eq!(remove_empty_block("", false), "");
    }
}