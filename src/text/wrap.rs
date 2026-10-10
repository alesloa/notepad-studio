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
// chars without re-shaping. `ShapedLine::x_for_index` scans every glyph
// from the start, so asking it once per char made wrapping one line
// quadratic: a 100 KB minified line took seconds per repaint. `GlyphXs`
// answers the same question by binary search.

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
    if max_width <= Pixels::from(0.) || line_text.is_empty() || shaped.width <= max_width {
        return Vec::new();
    }
    let glyphs = shaped
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.index, glyph.position.x));
    let xs = GlyphXs::new(glyphs, shaped.width);
    breaks_with(line_text, |index| xs.x_for_index(index), max_width)
}

/// The greedy break loop, measuring with `x_for_index` (the x where the
/// char at a byte index starts, or the line width past the end).
fn breaks_with(
    line_text: &str,
    x_for_index: impl Fn(usize) -> Pixels,
    max_width: Pixels,
) -> Vec<usize> {
    let mut breaks: Vec<usize> = Vec::new();
    let line_len = line_text.len();

    // Indices we consult on the shaped line are byte indices into the
    // *original* text — `shape_line` keeps that mapping. Walk char boundaries
    // (using `char_indices`) so we never split a multi-byte codepoint.
    let mut row_start: usize = 0;
    let mut last_ws_end: Option<usize> = None;
    // The x-offset where the current row begins. Subtracting from
    // `shaped.x_for_index(i)` gives the row-local width, which is what we
    // compare against `max_width`.
    let mut row_x_start: Pixels = x_for_index(0);

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
        let x_end = x_for_index(next_byte);
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
            row_x_start = x_for_index(break_at);
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

/// Glyph x-positions indexed for lookup by byte offset. Gives the same
/// answer as `ShapedLine::x_for_index` (the x of the first glyph, in
/// shaping order, whose byte index is at or past the one asked for; the
/// line width when there is none) in O(log n) instead of O(n).
struct GlyphXs {
    /// Largest byte index among glyphs `0..=k`. Never decreases, so the
    /// first glyph reaching an index is found by binary search even if
    /// shaping ever emits glyphs out of byte order.
    reach: Vec<usize>,
    xs: Vec<Pixels>,
    width: Pixels,
}

impl GlyphXs {
    fn new(glyphs: impl Iterator<Item = (usize, Pixels)>, width: Pixels) -> Self {
        let (lower, _) = glyphs.size_hint();
        let mut reach = Vec::with_capacity(lower);
        let mut xs = Vec::with_capacity(lower);
        let mut furthest = 0usize;
        for (index, x) in glyphs {
            furthest = furthest.max(index);
            reach.push(furthest);
            xs.push(x);
        }
        Self { reach, xs, width }
    }

    fn x_for_index(&self, index: usize) -> Pixels {
        let k = self.reach.partition_point(|&r| r < index);
        self.xs.get(k).copied().unwrap_or(self.width)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    /// Glyphs for `text` laid out left to right: one glyph per char at its
    /// byte offset, ASCII 7 px, tab 28 px, anything wider 14 px.
    fn layout(text: &str) -> (Vec<(usize, Pixels)>, Pixels) {
        let mut x = 0.0f32;
        let mut glyphs = Vec::new();
        for (i, c) in text.char_indices() {
            glyphs.push((i, px(x)));
            x += match c {
                '\t' => 28.0,
                c if c.is_ascii() => 7.0,
                _ => 14.0,
            };
        }
        (glyphs, px(x))
    }

    /// `ShapedLine::x_for_index` as gpui writes it: scan from the start.
    fn linear_x(glyphs: &[(usize, Pixels)], width: Pixels, index: usize) -> Pixels {
        glyphs
            .iter()
            .find(|(i, _)| *i >= index)
            .map(|(_, x)| *x)
            .unwrap_or(width)
    }

    fn sample_text(len: usize, seed: u64) -> String {
        let pieces = ["word", " ", "\t", "é", "日本", "a", "  ", "longerword", "😀"];
        let mut state = seed;
        let mut out = String::new();
        while out.len() < len {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let pick = (state >> 33) as usize % pieces.len();
            out.push_str(pieces.get(pick).copied().unwrap_or("a"));
        }
        out
    }

    #[test]
    fn indexed_lookup_matches_the_linear_scan() {
        let text = sample_text(400, 7);
        let (glyphs, width) = layout(&text);
        let xs = GlyphXs::new(glyphs.iter().copied(), width);
        for index in 0..=text.len() + 2 {
            assert_eq!(xs.x_for_index(index), linear_x(&glyphs, width, index), "index {index}");
        }
        // Out-of-order glyphs (never seen from gpui, but the contract is
        // "first in shaping order", so hold to it).
        let shuffled = [(0, px(0.)), (8, px(10.)), (4, px(20.)), (12, px(30.))];
        let xs = GlyphXs::new(shuffled.iter().copied(), px(40.));
        for index in 0..14 {
            assert_eq!(xs.x_for_index(index), linear_x(&shuffled, px(40.), index), "index {index}");
        }
    }

    #[test]
    fn breaks_are_unchanged_by_the_indexed_lookup() {
        for seed in 1..20u64 {
            let text = sample_text(300, seed);
            let (glyphs, width) = layout(&text);
            let xs = GlyphXs::new(glyphs.iter().copied(), width);
            for max in [5.0f32, 7.0, 20.0, 63.0, 100.0, 333.0] {
                let fast = breaks_with(&text, |i| xs.x_for_index(i), px(max));
                let slow = breaks_with(&text, |i| linear_x(&glyphs, width, i), px(max));
                assert_eq!(fast, slow, "seed {seed}, width {max}");
                assert!(fast.iter().all(|&b| text.is_char_boundary(b)));
                assert!(fast.windows(2).all(|w| w.first() < w.get(1)));
            }
        }
    }

    #[test]
    fn rows_cover_the_line_once() {
        let text = "alpha beta gamma\tdelta";
        let (glyphs, width) = layout(text);
        let xs = GlyphXs::new(glyphs.iter().copied(), width);
        let breaks = breaks_with(text, |i| xs.x_for_index(i), px(50.));
        assert_eq!(breaks, vec![6, 11, 17]);
        let rows = rows_from_breaks(text.len(), &breaks);
        assert_eq!(rows, vec![0..6, 6..11, 11..17, 17..text.len()]);
    }

    /// Wrapping one long line, linear-scan lookup vs indexed lookup. Run
    /// with:
    /// `cargo test --release -p notepad-studio --bin notepad-studio wrap_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn wrap_cost() {
        for len in [10_000usize, 50_000, 200_000, 1_000_000] {
            let text = sample_text(len, 3);
            let (glyphs, width) = layout(&text);
            let t = std::time::Instant::now();
            let xs = GlyphXs::new(glyphs.iter().copied(), width);
            let fast = breaks_with(&text, |i| xs.x_for_index(i), px(800.));
            let indexed = t.elapsed();
            if len <= 50_000 {
                let t = std::time::Instant::now();
                let slow = breaks_with(&text, |i| linear_x(&glyphs, width, i), px(800.));
                let linear = t.elapsed();
                assert_eq!(fast, slow);
                println!("{len:>8} bytes: linear {linear:?}, indexed {indexed:?}");
            } else {
                println!("{len:>8} bytes: linear (skipped), indexed {indexed:?}");
            }
        }
    }
}