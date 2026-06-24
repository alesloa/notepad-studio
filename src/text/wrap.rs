// Word-wrap algorithm. Given a shaped line and a max pixel width, returns a
// list of byte offsets (within the line) at which to start each visual row.
// The first row always starts at 0 (not included in the returned vec when
// the line fits in one row — empty vec means "no wrap").
//
// Algorithm: greedy, whitespace-preferring. Walks the line char by char,
// tracking the last whitespace boundary seen. When the cumulative width
// exceeds the target, breaks at the last whitespace; if no whitespace has
// been seen since the previous break (a single "word" wider than the
// viewport), hard-breaks at the char that overflowed. Matches NPP's "wrap
// at word boundary, hard-break unbreakables".
//
// The shaped line carries x-offsets per byte index, so we get accurate
// pixel-width measurements for tabs, mixed-width glyphs, and combining
// chars without re-shaping. Pure function — no allocations beyond the
// returned vec.

use gpui::{Pixels, ShapedLine};

/// Compute visual-row break byte offsets for `line_text` (already shaped as
/// `shaped`), given a maximum pixel width. Returns the byte offsets at which
/// each *non-first* visual row begins. If the line fits in `max_width`, the
/// returned vec is empty.
///
/// `max_width` of zero or negative is treated as "no wrap" (empty result) so
/// callers don't have to guard against pre-paint bounds.
pub(crate) fn compute_wrap_breaks(
    line_text: &str,
    shaped: &ShapedLine,
    max_width: Pixels,
) -> Vec<usize> {
    let mut breaks: Vec<usize> = Vec::new();
    if max_width <= Pixels::from(0.) {
        return breaks;
    }
    let line_len = line_text.len();
    if line_len == 0 || shaped.width <= max_width {
        return breaks;
    }

    // Indices we consult on the shaped line are byte indices into the
    // *original* text — `shape_line` keeps that mapping. Walk char boundaries
    // (using `char_indices`) so we never split a multi-byte codepoint.
    let mut row_start: usize = 0;
    let mut last_ws_end: Option<usize> = None;
    // The x-offset where the current row begins. Subtracting from
    // `shaped.x_for_index(i)` gives the row-local width, which is what we
    // compare against `max_width`.
    let mut row_x_start: Pixels = shaped.x_for_index(0);

    let mut iter = line_text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        // End-byte of this char (start of next char, or line_len).
        let next_byte = match iter.peek() {
            Some(&(ni, _)) => ni,
            None => line_len,
        };

        // Track the last whitespace boundary we could break after. NPP wraps
        // *after* the whitespace run so the leading edge of the next visual
        // row starts on a real word.
        if c == ' ' || c == '\t' {
            // Walk forward through any contiguous whitespace so the break
            // point sits past the entire run, not in the middle of it.
            // (Handles "    ". The next iteration of the outer loop will
            // observe the post-ws char.)
            //
            // Actually simpler: just record the *end* of the current ws char.
            // Multiple consecutive ws chars will each update last_ws_end to
            // the next iteration's start, which is correct.
            last_ws_end = Some(next_byte);
        }

        // Width consumed by everything up to and including this char.
        let x_end = shaped.x_for_index(next_byte);
        let row_w = x_end - row_x_start;

        if row_w > max_width && next_byte > row_start {
            // Overflow. Break preferentially at the last whitespace in this
            // row; if no whitespace was seen since `row_start`, hard-break
            // at the start of *this* char.
            let break_at = match last_ws_end {
                Some(ws) if ws > row_start && ws <= next_byte => ws,
                _ => {
                    // Hard break — start the next row with the overflowing
                    // char. If `i == row_start` (the very first char of the
                    // row already overflows the row, e.g. a tab wider than
                    // viewport), advance past it so we make progress.
                    if i > row_start { i } else { next_byte }
                }
            };
            breaks.push(break_at);
            row_start = break_at;
            row_x_start = shaped.x_for_index(break_at);
            last_ws_end = None;

            // The current char might still belong to this new row. If we
            // hard-broke at `i`, the char IS this row's first char and is
            // already accounted for in `row_x_start`. If we soft-broke at a
            // whitespace earlier, we need to re-test the current char
            // against the new row width — but in practice the next loop
            // iteration handles it via x_for_index against the new
            // row_x_start, so just continue.
            //
            // One subtle bit: if `break_at == next_byte` (we broke right
            // after this char), the char is on the previous row and the
            // next row starts empty. That's also handled — the next
            // iteration measures from row_x_start which equals
            // x_for_index(next_byte).
            continue;
        }
    }

    breaks
}

/// Given the wrap break byte offsets produced by `compute_wrap_breaks`, plus
/// the line's full byte length, return the inclusive byte ranges of each
/// visual row. Always returns at least one range covering the whole line.
pub(crate) fn rows_from_breaks(line_len: usize, breaks: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut rows = Vec::with_capacity(breaks.len() + 1);
    let mut start = 0usize;
    for &b in breaks {
        if b <= start {
            continue;
        }
        rows.push(start..b);
        start = b;
    }
    rows.push(start..line_len);
    rows
}
