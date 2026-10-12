// EditorElement — paints text + selection rectangles + caret.

#[cfg(test)]
#[path = "source_input_origin_tests.rs"]
pub(crate) mod source_input_origin_tests;

use std::ops::Range;

use gpui::{
    App, BorderStyle, Bounds, DispatchPhase, ElementId, Entity, Font,
    GlobalElementId, Hsla, IntoElement, LayoutId, MouseButton, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Rgba, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun,
    UnderlineStyle, Window, fill, outline, point, px, relative, rgb, rgba, size,
};

use crate::buffer::LineGeo;
use crate::consts::{CARET_WIDTH_PX, OVERSTRIKE_CARET_HEIGHT_PX, OVERSTRIKE_CARET_MIN_WIDTH_PX};
use crate::syntax::{self, HighlightSpan};
#[cfg(test)]
use crate::syntax::HighlightKind;
use crate::text::{url_scan, wrap};
use crate::theme::SyntaxTheme;
use crate::workspace::{WrapCache, WrapRow, Workspace};

pub(crate) struct EditorElement {
    pub(crate) workspace: Entity<Workspace>,
}

#[path = "editor_proofreading.rs"]
mod proofreading;

pub(crate) struct EditorPrepaint {
    lines: Vec<EditorLineLayout>,
    selection_quads: Vec<PaintQuad>,
    /// NPP "Highlight current line" backdrop. One quad per *visual row* that
    /// belongs to the logical line containing the caret (wrap-on flows
    /// produce multiple rows per line, all of which get the backdrop).
    /// Painted *under* every other overlay so find / mark / smart / selection
    /// quads still read on top. Empty when the toggle is off.
    current_line_quads: Vec<PaintQuad>,
    /// Find-match highlight quads. Painted under selection so an in-progress
    /// drag-select still reads as the selected range, while idle matches show
    /// through clearly.
    match_quads: Vec<PaintQuad>,
    /// "Mark All" persistent highlights. Painted *below* `match_quads` so a
    /// live find highlight on the same byte still reads on top.
    mark_quads: Vec<PaintQuad>,
    /// Outlines for the cached before-caret or at-caret brace and its match.
    /// Quads are rebuilt for the visible lines; matching runs on a worker.
    bracket_quads: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    /// "Show whitespace" overlay glyphs. Empty when the toggle is off.
    /// Built additively in `prepaint_whitespace_glyphs` so the existing
    /// per-line paint logic above stays untouched. Each entry is the origin
    /// at which to paint the corresponding `ShapedLine`.
    whitespace_glyphs: Vec<WhitespaceGlyph>,
    /// `Editing 2 ▸ EOL` overlay glyphs (CRLF / LF / CR), painted at the
    /// right edge of each visible line whose source contains a line ending.
    /// Empty when the toggle is off or the visible region has no terminated
    /// line. Each entry carries an optional rounded-rect bg that paints
    /// behind the text when `eol_display_mode = RoundedRectangle`.
    eol_glyphs: Vec<EolGlyph>,
    /// `Editing 2 ▸ Non-Printing Characters` overlay glyphs (C0 control
    /// character abbreviations). Empty when the toggle is off or the visible
    /// region has no control characters. One entry per byte hit.
    npc_glyphs: Vec<WhitespaceGlyph>,
    /// One hitbox per URL on each visible line (pixel bounds match the
    /// underlined glyph cells). Paint phase switches the cursor to
    /// `PointingHand` over each when the platform/command modifier is held,
    /// matching the Cmd+click handler in `workspace::mouse::on_mouse_down`.
    url_hitboxes: Vec<gpui::Hitbox>,
}

/// One glyph from the "show whitespace" overlay. The `shaped` lines for the
/// dot and arrow are shaped once per prepaint and cloned (cheap — they're
/// `Arc`-backed) so each visible whitespace cell only stores its origin.
struct WhitespaceGlyph {
    shaped: ShapedLine,
    origin: gpui::Point<Pixels>,
}

/// One EOL-overlay glyph (CRLF / LF / CR). When `bg` is Some, it paints a
/// rounded rectangle behind the text — NPP's "Rounded Rectangle" display
/// mode. When None, it's the bare "Plain Text" mode.
struct EolGlyph {
    shaped: ShapedLine,
    origin: gpui::Point<Pixels>,
    bg: Option<PaintQuad>,
}

struct EditorLineLayout {
    shaped: ShapedLine,
    line_byte_start: usize,
    line_byte_len: usize,
    bounds: Bounds<Pixels>,
}

/// Build the per-tab visual-row cache from scratch by walking every line in
/// the rope, shaping it, and computing wrap break offsets. Inserted into
/// `tab.wrap_cache` for the next paint to consume. O(N_lines) work — each
/// line is shaped once and the resulting `ShapedLine` is dropped (only the
/// row metadata persists in the cache). For the buffer sizes Notepad++
/// targets (a few thousand lines) this is sub-millisecond.
fn rebuild_wrap_cache(
    rope: &ropey::Rope,
    font: Font,
    font_size: Pixels,
    rem_size: Pixels,
    text_color: Hsla,
    max_width: Pixels,
    window: &mut Window,
) -> WrapCache {
    let total_lines = rope.len_lines().max(1);
    let total_bytes = rope.len_bytes() as u32;
    let mut rows: Vec<WrapRow> = Vec::with_capacity(total_lines);
    let make_run = |len: usize, font: &Font| TextRun {
        len,
        font: font.clone(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    for line_idx in 0..total_lines {
        let line_byte_start = rope.line_to_byte(line_idx) as u32;
        let line_slice = rope.line(line_idx);
        let raw = line_slice.to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        let line_byte_len = trimmed.len() as u32;

        if line_byte_len == 0 {
            // Empty line — one visual row covering byte_start..byte_start.
            rows.push(WrapRow {
                logical_line: line_idx as u32,
                byte_start: line_byte_start,
                byte_end: line_byte_start,
            });
            continue;
        }

        // Shape just to measure widths. The resulting ShapedLine is dropped
        // after we extract break offsets — only row metadata persists.
        let display: SharedString = trimmed.to_string().into();
        let runs = vec![make_run(trimmed.len(), &font)];
        let shaped = window
            .text_system()
            .shape_line(display, font_size, &runs, None);

        if shaped.width <= max_width {
            // No wrap — one row spans the whole line.
            rows.push(WrapRow {
                logical_line: line_idx as u32,
                byte_start: line_byte_start,
                byte_end: line_byte_start.saturating_add(line_byte_len),
            });
            continue;
        }

        let breaks = wrap::compute_wrap_breaks(trimmed, &shaped, max_width);
        let segments = wrap::rows_from_breaks(trimmed.len(), &breaks);
        for seg in segments {
            rows.push(WrapRow {
                logical_line: line_idx as u32,
                byte_start: line_byte_start.saturating_add(seg.start as u32),
                byte_end: line_byte_start.saturating_add(seg.end as u32),
            });
        }
    }

    WrapCache {
        viewport_width: max_width,
        rem_size,
        font_family: font.family.clone(),
        total_rows: rows.len() as u32,
        byte_len: total_bytes,
        rows,
    }
}

/// One cached wrap row's text within its trimmed logical line, with the
/// row's byte offset into that line. Wrap breaks sit on char boundaries
/// when the cache is built, but a cache that outlived a change to the rope
/// (a reload, a streamed chunk) can hold offsets past the line or inside a
/// multi-byte char; those are clamped and snapped back to a boundary.
fn wrap_row_text<'a>(trimmed: &'a str, line_byte_start: usize, row: &WrapRow) -> (usize, &'a str) {
    let start = (row.byte_start as usize).saturating_sub(line_byte_start);
    let end = (row.byte_end as usize).saturating_sub(line_byte_start);
    let start = trimmed.floor_char_boundary(start);
    let end = trimmed.floor_char_boundary(end).max(start);
    (start, trimmed.get(start..end).unwrap_or(""))
}

/// One visual row prepared for paint. Same shape under both wrap modes:
/// when wrap is off, each logical line maps to exactly one
/// `VisibleRowSpec` whose offset/length cover the full printable line.
struct VisibleRowSpec {
    /// Byte offset into the buffer where this visual row begins.
    seg_byte_start: usize,
    /// Length in bytes of this visual row's text (printable, no EOL).
    seg_byte_len: usize,
    /// First-printable-byte offset within the logical line. Zero for the
    /// first visual row of a line; >0 for subsequent rows.
    seg_offset_in_line: usize,
    /// Total printable byte length of the underlying logical line.
    full_line_byte_len: usize,
    /// Buffer-byte offset of the logical line's start. Used so selection /
    /// match / mark range comparisons stay anchored to the logical line
    /// even when the visual row covers a sub-range.
    full_line_byte_start: usize,
    /// Document-relative top of this visual row (pre-scroll).
    line_top: Pixels,
    /// Already-shaped text for this segment.
    shaped: ShapedLine,
    /// URL byte ranges within this segment's text (offsets relative to
    /// `seg_byte_start`, so `shaped.x_for_index` accepts them directly).
    /// Empty when clickable URLs are disabled or this segment has none —
    /// the hover-cursor pass uses this to install per-URL hitboxes.
    url_ranges_in_seg: Vec<Range<usize>>,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

/// Slice the buffer's highlight spans down to the byte range owned by one
/// visible line, then build the sequence of `TextRun`s the shaper expects.
/// Gaps between highlighted spans get the default text color; spans
/// extending past the line end clamp to it. The sum of `run.len` equals
/// `text.len()`, which is what `Window::shape_line` requires.
fn build_line_runs(
    line_start: usize,
    text: &str,
    spans: &[HighlightSpan],
    font: Font,
    default_color: Hsla,
    syntax: &SyntaxTheme,
) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let line_len = text.len();
    let line_end = line_start + line_len;
    let mut local_pos: usize = 0;

    let push = |runs: &mut Vec<TextRun>, len: usize, color: Hsla, font: &Font| {
        if len == 0 {
            return;
        }
        runs.push(TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    };

    let first = spans.partition_point(|s| s.range.end <= line_start);
    for span in spans[first..].iter() {
        if span.range.start >= line_end {
            break;
        }
        let s = span.range.start.max(line_start);
        let e = span.range.end.min(line_end);
        if e <= s {
            continue;
        }
        // Spans lag edits until the debounced reparse lands, so a span edge
        // can fall inside a multi-byte char of the current text. The
        // platform shapers slice the text at run edges (`&text[a..b]` in
        // DirectWrite, `split_at` in CoreText) and panic there, so snap
        // both edges back to a char boundary.
        let local_s = text.floor_char_boundary(s - line_start);
        let local_e = text.floor_char_boundary(e - line_start);
        if local_e <= local_s {
            continue;
        }

        if local_s < local_pos {
            // Overlapping region — already covered by the previous run.
            // tree-sitter-highlight emits flat events, so this is rare,
            // but defensive.
            continue;
        }
        if local_s > local_pos {
            push(&mut runs, local_s - local_pos, default_color, &font);
        }
        let color: Hsla =
            Rgba::from(rgb(syntax::kind_color(span.kind, syntax))).into();
        push(&mut runs, local_e - local_s, color, &font);
        local_pos = local_e;
    }

    if local_pos < line_len {
        push(&mut runs, line_len - local_pos, default_color, &font);
    }

    runs
}

/// Splice URL underline overlays into a line's `TextRun` sequence.
///
/// `runs` is the syntax-coloured run list as produced by `build_line_runs`;
/// `seg_text` is the exact UTF-8 text those runs cover (its length must equal
/// the sum of `run.len`). For each URL detected by `url_scan::find_urls`,
/// any run intersecting the URL byte range is split so the URL portion gets
/// its own run with `underline = Some(...)` and `color = link_color`. Runs
/// outside URL ranges are left untouched. The total `run.len` sum is
/// preserved.
fn apply_url_underlines(
    runs: &mut Vec<TextRun>,
    seg_text: &str,
    link_color: Hsla,
    with_underline: bool,
) {
    if runs.is_empty() || seg_text.is_empty() {
        return;
    }
    let urls = url_scan::find_urls(seg_text);
    if urls.is_empty() {
        return;
    }
    // Cloud & Link ▸ No underline drops the line under the URL while keeping
    // the link-colour swap so users can still see what's clickable.
    let style = if with_underline {
        Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(link_color),
            wavy: false,
        })
    } else {
        None
    };

    let mut out: Vec<TextRun> = Vec::with_capacity(runs.len() + urls.len() * 2);
    let mut cursor: usize = 0;
    let runs_in = std::mem::take(runs);
    for run in runs_in {
        let run_start = cursor;
        let run_end = cursor + run.len;
        cursor = run_end;

        // Build the (possibly split) ranges within this run.
        let mut pos = run_start;
        for url in urls.iter() {
            if url.end <= run_start {
                continue;
            }
            if url.start >= run_end {
                break;
            }
            let s = url.start.max(run_start);
            let e = url.end.min(run_end);
            if pos < s {
                let mut piece = run.clone();
                piece.len = s - pos;
                out.push(piece);
            }
            if e > s {
                let mut piece = run.clone();
                piece.len = e - s;
                piece.color = link_color;
                piece.underline = style;
                out.push(piece);
            }
            pos = e;
        }
        if pos < run_end {
            let mut piece = run;
            piece.len = run_end - pos;
            out.push(piece);
        }
    }
    *runs = out;
}

/// The rope text of one painted row, `len` bytes from `start`. `None` when
/// the range doesn't fit the rope or splits a char, which a wrap row from a
/// stale cache can do.
fn row_text(rope: &ropey::Rope, start: usize, len: usize) -> Option<String> {
    let end = start.checked_add(len)?;
    rope.get_byte_slice(start..end).map(|s| s.to_string())
}

/// Build the per-glyph layout for the "show whitespace" overlay. Shapes the
/// dot (`·`) and arrow (`→`) once each at file scope, then walks every
/// visible line's text and emits one entry per space / tab. Each entry's
/// `origin` is the window-space point at which the shaped glyph paints — the
/// caller renders them with `ShapedLine::paint` during the existing line
/// paint phase, so the glyphs land **inside** the line text rendering
/// rather than as extra quads.
///
/// Pure additive pass — never reads or mutates any of the existing prepaint
/// state (selection quads, match quads, line layouts) beyond what's needed
/// to compute glyph positions, so it composes safely with concurrent paint
/// changes elsewhere in this file.
fn prepaint_whitespace_glyphs(
    lines: &[EditorLineLayout],
    buffer: &crate::buffer::Buffer,
    font_size: Pixels,
    font: Font,
    color: Hsla,
    line_left: Pixels,
    window: &mut Window,
) -> Vec<WhitespaceGlyph> {
    if lines.is_empty() {
        return Vec::new();
    }

    // Shape the two reference glyphs once. ShapedLine is Arc-backed so cloning
    // it for every emitted position is cheap.
    let glyph_run = |c: char| -> Vec<TextRun> {
        let s = c.to_string();
        vec![TextRun {
            len: s.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }]
    };
    let dot_text: SharedString = "·".into();
    let arrow_text: SharedString = "→".into();
    let dot_runs = glyph_run('·');
    let arrow_runs = glyph_run('→');
    let dot_shaped = window
        .text_system()
        .shape_line(dot_text.clone(), font_size, &dot_runs, None);
    let arrow_shaped = window
        .text_system()
        .shape_line(arrow_text.clone(), font_size, &arrow_runs, None);
    let dot_width = dot_shaped.width;
    let arrow_width = arrow_shaped.width;

    let mut out: Vec<WhitespaceGlyph> = Vec::new();

    for layout in lines {
        let line_byte_start = layout.line_byte_start;
        let line_byte_len = layout.line_byte_len;
        if line_byte_len == 0 {
            continue;
        }
        // Pull *just this segment's* text from the rope. Under wrap-on,
        // each `EditorLineLayout` covers a sub-range of its logical line;
        // walk only those bytes so glyph-cell indices line up with the
        // segment-level `shaped` layout.
        let Some(segment_string) = row_text(&buffer.rope, line_byte_start, line_byte_len) else {
            continue;
        };

        let line_top = layout.bounds.top();
        let bytes = segment_string.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b' ' => {
                    let cell_left = line_left + layout.shaped.x_for_index(i);
                    let cell_right = line_left + layout.shaped.x_for_index(i + 1);
                    if cell_right <= cell_left {
                        continue;
                    }
                    // Center the dot horizontally inside the space cell.
                    let cell_w = cell_right - cell_left;
                    let glyph_x = cell_left + (cell_w - dot_width) / 2.;
                    out.push(WhitespaceGlyph {
                        shaped: dot_shaped.clone(),
                        origin: point(glyph_x, line_top),
                    });
                }
                b'\t' => {
                    let cell_left = line_left + layout.shaped.x_for_index(i);
                    let cell_right = line_left + layout.shaped.x_for_index(i + 1);
                    if cell_right <= cell_left {
                        continue;
                    }
                    // Place the arrow at the start of the tab cell, padded a
                    // hair so it doesn't kiss the previous glyph. Arrow size
                    // stays one glyph wide regardless of how far the tab
                    // expands — matches NPP's lightweight overlay style.
                    let cell_w = cell_right - cell_left;
                    let pad = (cell_w - arrow_width).max(px(0.)) / 2.;
                    let glyph_x = cell_left + pad;
                    out.push(WhitespaceGlyph {
                        shaped: arrow_shaped.clone(),
                        origin: point(glyph_x, line_top),
                    });
                }
                _ => {}
            }
        }
    }

    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EolMark {
    Crlf,
    Lf,
    Cr,
}

/// The line ending that starts at byte `end`, the first byte past a row's
/// printable text. The wrap cache trims `\n`/`\r`, so these are the raw
/// EOL bytes when the row ends its line. Reads bytes rather than slicing:
/// `end + 2` lands inside the next char whenever an LF is followed by a
/// multi-byte char, or a wrapped row ends between two CJK chars.
fn eol_mark_at(rope: &ropey::Rope, end: usize) -> Option<EolMark> {
    let mut bytes = rope.get_bytes_at(end)?;
    match (bytes.next(), bytes.next()) {
        (Some(b'\r'), Some(b'\n')) => Some(EolMark::Crlf),
        (Some(b'\n'), _) => Some(EolMark::Lf),
        (Some(b'\r'), _) => Some(EolMark::Cr),
        _ => None,
    }
}

/// Build the per-line EOL marker overlay (CRLF / LF / CR). For each visible
/// line layout, peeks at the bytes that follow the printable region in the
/// rope — those bytes ARE the line ending (one or two bytes). Skips lines
/// that don't terminate (the buffer's final line when EOF isn't a newline).
///
/// Style is driven by `mode`: `Plain` paints just the text glyph; `Rounded`
/// paints a 1px-bordered rounded rectangle behind the text in the same
/// faint color. Matches NPP's `_eolMode` switch.
fn prepaint_eol_glyphs(
    lines: &[EditorLineLayout],
    buffer: &crate::buffer::Buffer,
    font_size: Pixels,
    font: Font,
    color: Hsla,
    line_left: Pixels,
    rounded: bool,
    window: &mut Window,
) -> Vec<EolGlyph> {
    if lines.is_empty() {
        return Vec::new();
    }
    let line_height = window.line_height();
    let total_bytes = buffer.rope.len_bytes();

    let make_run = |len: usize| -> Vec<TextRun> {
        vec![TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }]
    };
    let shape_token = |window: &mut Window, label: &'static str| -> ShapedLine {
        let s: SharedString = label.into();
        let runs = make_run(label.len());
        window.text_system().shape_line(s, font_size, &runs, None)
    };
    let lf = shape_token(window, "LF");
    let crlf = shape_token(window, "CRLF");
    let cr = shape_token(window, "CR");

    let mut out: Vec<EolGlyph> = Vec::new();
    for layout in lines {
        let end = layout.line_byte_start + layout.line_byte_len;
        if end >= total_bytes {
            continue;
        }
        let glyph = match eol_mark_at(&buffer.rope, end) {
            Some(EolMark::Crlf) => &crlf,
            Some(EolMark::Lf) => &lf,
            Some(EolMark::Cr) => &cr,
            None => continue,
        };
        // Anchor the glyph just past the last printable byte's x. The
        // wrap-cache trims trailing whitespace from `line_byte_len`, so
        // `x_for_index(line_byte_len)` is the rightmost printable x.
        let x_after = line_left + layout.shaped.x_for_index(layout.line_byte_len);
        let origin = point(x_after + px(4.), layout.bounds.top());
        let bg = if rounded {
            // Rounded rectangle hugs the text with 3px horizontal padding
            // and 2px vertical inset. 1px border in the same faint colour;
            // no fill (read-through to the editor background).
            let pad_x = px(3.);
            let inset_y = px(1.);
            let glyph_w = glyph.width;
            let rect = Bounds::new(
                point(origin.x - pad_x, layout.bounds.top() + inset_y),
                size(glyph_w + pad_x * 2., line_height - inset_y * 2.),
            );
            Some(outline(rect, color, BorderStyle::Solid))
        } else {
            None
        };
        out.push(EolGlyph {
            shaped: glyph.clone(),
            origin,
            bg,
        });
    }
    out
}

/// Build the Non-Printing Characters (NPC) overlay. Walks every byte in
/// every visible line looking for C0 control characters (0x00..=0x1F, 0x7F)
/// excluding `\t` (already covered by the whitespace arrow glyph). Each hit
/// gets an abbreviation glyph ("NUL", "SOH", "ESC", "DEL", …) painted at
/// the character cell. Codepoint mode renders the hex form ("0x00", …).
///
/// The C1 + Unicode EOL extension (`Apply Appearance to C0, C1 & Unicode EOL`)
/// is honored only for the C0 range here — C1 (0x80..=0x9F) and Unicode-EOL
/// (U+2028 LSEP, U+2029 PSEP, U+0085 NEL) require codepoint decoding and
/// are deferred so the common case stays cheap.
fn prepaint_npc_glyphs(
    lines: &[EditorLineLayout],
    buffer: &crate::buffer::Buffer,
    font_size: Pixels,
    font: Font,
    color: Hsla,
    line_left: Pixels,
    codepoint_mode: bool,
    include_c1_unicode_eol: bool,
    window: &mut Window,
) -> Vec<WhitespaceGlyph> {
    if lines.is_empty() {
        return Vec::new();
    }
    let make_run = |len: usize| -> Vec<TextRun> {
        vec![TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }]
    };
    let shape = |window: &mut Window, label: &str| -> ShapedLine {
        let s: SharedString = label.to_string().into();
        let runs = make_run(label.len());
        window.text_system().shape_line(s, font_size, &runs, None)
    };
    let c0_label_for = |b: u8| -> &'static str {
        if codepoint_mode {
            CODEPOINT_LABELS[b as usize]
        } else {
            ABBREV_LABELS[b as usize]
        }
    };

    let mut out: Vec<WhitespaceGlyph> = Vec::new();
    for layout in lines {
        if layout.line_byte_len == 0 {
            continue;
        }
        let Some(text) = row_text(&buffer.rope, layout.line_byte_start, layout.line_byte_len) else {
            continue;
        };
        let line_top = layout.bounds.top();
        // Pass 1: C0 + DEL byte scan. Cheap and the common case.
        for (i, &b) in text.as_bytes().iter().enumerate() {
            if !((b <= 0x1F && b != b'\t' && b != b'\n' && b != b'\r') || b == 0x7F) {
                continue;
            }
            let label = c0_label_for(b);
            if label.is_empty() {
                continue;
            }
            let cell_left = line_left + layout.shaped.x_for_index(i);
            let shaped = shape(window, label);
            out.push(WhitespaceGlyph {
                shaped,
                origin: point(cell_left, line_top),
            });
        }
        // Pass 2: C1 + Unicode-EOL char scan. Only when the
        // `Apply appearance to C0, C1 & Unicode EOL` checkbox is on.
        // Walks chars so the cp values are correctly decoded.
        if include_c1_unicode_eol {
            for (i, ch) in text.char_indices() {
                let cp = ch as u32;
                let is_c1 = (0x80..=0x9F).contains(&cp);
                let is_unicode_eol = cp == 0x85 || cp == 0x2028 || cp == 0x2029;
                if !is_c1 && !is_unicode_eol {
                    continue;
                }
                let label = if codepoint_mode {
                    format!("U+{cp:04X}")
                } else {
                    match cp {
                        0x85 => "NEL".to_string(),
                        0x2028 => "LSEP".to_string(),
                        0x2029 => "PSEP".to_string(),
                        // C1 controls — well-known abbreviations from
                        // ISO/IEC 6429. Fall back to U+ form for any code
                        // point without a standard mnemonic in the table.
                        _ => c1_abbrev_for(cp)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("U+{cp:04X}")),
                    }
                };
                let cell_left = line_left + layout.shaped.x_for_index(i);
                let shaped = shape(window, &label);
                out.push(WhitespaceGlyph {
                    shaped,
                    origin: point(cell_left, line_top),
                });
            }
        }
    }
    out
}

/// ISO/IEC 6429 C1 control-character mnemonics. Returns `None` for code
/// points outside the C1 range or without a standard abbreviation.
fn c1_abbrev_for(cp: u32) -> Option<&'static str> {
    Some(match cp {
        0x80 => "PAD", 0x81 => "HOP", 0x82 => "BPH", 0x83 => "NBH",
        0x84 => "IND", 0x85 => "NEL", 0x86 => "SSA", 0x87 => "ESA",
        0x88 => "HTS", 0x89 => "HTJ", 0x8A => "VTS", 0x8B => "PLD",
        0x8C => "PLU", 0x8D => "RI",  0x8E => "SS2", 0x8F => "SS3",
        0x90 => "DCS", 0x91 => "PU1", 0x92 => "PU2", 0x93 => "STS",
        0x94 => "CCH", 0x95 => "MW",  0x96 => "SPA", 0x97 => "EPA",
        0x98 => "SOS", 0x99 => "SGCI", 0x9A => "SCI", 0x9B => "CSI",
        0x9C => "ST",  0x9D => "OSC", 0x9E => "PM",  0x9F => "APC",
        _ => return None,
    })
}

/// NPC abbreviation table (NPP `_npcMode = abbreviation`). Index by byte
/// value 0..=255. Empty string for non-control bytes.
const ABBREV_LABELS: [&str; 256] = build_abbrev_labels();
const fn build_abbrev_labels() -> [&'static str; 256] {
    let mut t = [""; 256];
    t[0x00] = "NUL"; t[0x01] = "SOH"; t[0x02] = "STX"; t[0x03] = "ETX";
    t[0x04] = "EOT"; t[0x05] = "ENQ"; t[0x06] = "ACK"; t[0x07] = "BEL";
    t[0x08] = "BS";  t[0x09] = "HT";  t[0x0A] = "LF";  t[0x0B] = "VT";
    t[0x0C] = "FF";  t[0x0D] = "CR";  t[0x0E] = "SO";  t[0x0F] = "SI";
    t[0x10] = "DLE"; t[0x11] = "DC1"; t[0x12] = "DC2"; t[0x13] = "DC3";
    t[0x14] = "DC4"; t[0x15] = "NAK"; t[0x16] = "SYN"; t[0x17] = "ETB";
    t[0x18] = "CAN"; t[0x19] = "EM";  t[0x1A] = "SUB"; t[0x1B] = "ESC";
    t[0x1C] = "FS";  t[0x1D] = "GS";  t[0x1E] = "RS";  t[0x1F] = "US";
    t[0x7F] = "DEL";
    t
}

/// NPC codepoint table (NPP `_npcMode = codepoint`). Each byte's index maps
/// to its `U+00XX` form so the renderer can use it without re-formatting.
const CODEPOINT_LABELS: [&str; 256] = build_codepoint_labels();
const fn build_codepoint_labels() -> [&'static str; 256] {
    let mut t = [""; 256];
    // Hard-coded for the C0 range + DEL. Outside that range stays empty —
    // matches the same filter as `prepaint_npc_glyphs`'s byte check.
    t[0x00] = "U+0000"; t[0x01] = "U+0001"; t[0x02] = "U+0002"; t[0x03] = "U+0003";
    t[0x04] = "U+0004"; t[0x05] = "U+0005"; t[0x06] = "U+0006"; t[0x07] = "U+0007";
    t[0x08] = "U+0008"; t[0x09] = "U+0009"; t[0x0A] = "U+000A"; t[0x0B] = "U+000B";
    t[0x0C] = "U+000C"; t[0x0D] = "U+000D"; t[0x0E] = "U+000E"; t[0x0F] = "U+000F";
    t[0x10] = "U+0010"; t[0x11] = "U+0011"; t[0x12] = "U+0012"; t[0x13] = "U+0013";
    t[0x14] = "U+0014"; t[0x15] = "U+0015"; t[0x16] = "U+0016"; t[0x17] = "U+0017";
    t[0x18] = "U+0018"; t[0x19] = "U+0019"; t[0x1A] = "U+001A"; t[0x1B] = "U+001B";
    t[0x1C] = "U+001C"; t[0x1D] = "U+001D"; t[0x1E] = "U+001E"; t[0x1F] = "U+001F";
    t[0x7F] = "U+007F";
    t
}

/// Build 1px-outline quads for a bracket byte position if it falls on a
/// visible line. Pushed into `out`. Skips when the byte sits past the
/// printable end of its line (e.g. EOL) — bracket chars never do, so this
/// is purely defensive.
fn bracket_outline_for_byte(
    target_byte: usize,
    lines: &[EditorLineLayout],
    line_left: Pixels,
    line_height: Pixels,
    color: Hsla,
    out: &mut Vec<PaintQuad>,
) {
    for layout in lines {
        let line_start = layout.line_byte_start;
        let line_end = line_start + layout.line_byte_len;
        if target_byte < line_start || target_byte >= line_end {
            continue;
        }
        let local = target_byte - line_start;
        // Bracket chars are single-byte ASCII so `local + 1` is a valid
        // grapheme boundary in the shaped line.
        let x_start = line_left + layout.shaped.x_for_index(local);
        let x_end = line_left + layout.shaped.x_for_index(local + 1);
        if x_end <= x_start {
            return;
        }
        let bounds = Bounds::new(
            point(x_start, layout.bounds.top()),
            size(x_end - x_start, line_height),
        );
        out.push(outline(bounds, color, BorderStyle::default()));
        return;
    }
}

impl gpui::Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // Pull theme/style snapshots first via a non-mutating read so we can
        // compute the wrap cache in a separate `update` borrow without a
        // double-borrow conflict on the workspace entity.
        let style = window.text_style();
        let mut font: Font = style.font();
        let font_size: Pixels = style.font_size.to_pixels(window.rem_size());
        let line_height: Pixels = window.line_height();
        let rem_size: Pixels = window.rem_size();
        let (theme, text_color, selection_color, caret_color, match_color, match_current_color, mark_color, smart_color, current_line_color, word_wrap): (
            crate::theme::Theme,
            Hsla,
            Hsla,
            Hsla,
            Hsla,
            Hsla,
            Hsla,
            Hsla,
            Hsla,
            bool,
        ) = {
            let workspace = self.workspace.read(cx);
            let theme = workspace.theme();
            (
                theme,
                Rgba::from(rgb(theme.text)).into(),
                rgba(theme.selection).into(),
                Rgba::from(rgb(theme.text)).into(),
                rgba(theme.match_highlight).into(),
                rgba(theme.match_highlight_current).into(),
                rgba(theme.mark_highlight).into(),
                rgba(theme.smart_highlight).into(),
                Rgba::from(rgb(theme.current_line_bg)).into(),
                workspace.word_wrap,
            )
        };
        font.family = crate::fonts::editor_family(theme.editor_font);

        // Clickable-URL underline colour. Resolved once so per-line paint can
        // splice an underline overlay without re-reading settings or theme.
        let url_link_color: Hsla = Rgba::from(rgb(theme.link)).into();
        let (url_underline_enabled, url_with_underline, highlight_current_line) = {
            let snap = crate::settings::snapshot();
            (
                snap.clickable_urls_enabled,
                !snap.clickable_urls_no_underline,
                snap.highlight_current_line,
            )
        };

        // Wrap-target width inside the editor pane. Subtract a small inset so
        // wrapped rows don't kiss the right edge — matches NPP's behaviour
        // and keeps room for the caret on the last char of a row. Floor at
        // 1px so we never produce a zero-width wrap target on a vanishingly
        // small viewport.
        let wrap_max_width = (bounds.size.width - px(8.)).max(px(1.));

        // Step 1 — refresh the active tab's wrap cache when wrap is on. The
        // cache is keyed implicitly by viewport width and rem size; if either
        // changed since the last build, drop and rebuild. This is the only
        // place that mutates the workspace inside `prepaint`.
        if word_wrap {
            // Decide whether we need a (re)build outside the workspace
            // borrow so we don't hold a mutable handle longer than needed.
            let needs_rebuild = {
                let w = self.workspace.read(cx);
                let buffer = w.buffer();
                // Edits drop the cache in `apply_edit`; the byte length
                // catches a rope replaced without one (reload, open into
                // the pristine tab). Not while a streamed load is still
                // appending chunks: that would rebuild the whole cache per
                // chunk. The first paint after the load finishes catches up.
                let len_changed = |c: &WrapCache| {
                    buffer.loading.is_none() && c.byte_len != buffer.rope.len_bytes() as u32
                };
                w.tabs[w.active].wrap_cache.as_ref().is_none_or(|c| {
                    c.viewport_width != wrap_max_width
                        || c.rem_size != rem_size
                        || c.font_family != font.family
                        || len_changed(c)
                })
            };
            if needs_rebuild {
                // Snapshot rope into a temporary so the cache builder can
                // shape lines via the window without holding the workspace
                // borrow across the shape calls.
                let rope = self.workspace.read(cx).buffer().rope.clone();
                let cache = rebuild_wrap_cache(
                    &rope,
                    font.clone(),
                    font_size,
                    rem_size,
                    text_color,
                    wrap_max_width,
                    window,
                );
                self.workspace.update(cx, |w, _| {
                    let active = w.active;
                    w.tabs[active].wrap_cache = Some(cache);
                });
            }
        }

        self.workspace.update(cx, |w, cx| w.refresh_brace_match(window, cx));

        // Step 2 — read the workspace for the rest of the prepaint pass.
        let workspace = self.workspace.read(cx);
        let buffer = workspace.buffer();
        let selection = workspace.selection();
        let total_lines = buffer.rope.len_lines().max(1);
        let selection_range = selection.ordered();
        let head = selection.head;
        let selection_empty = selection.is_empty();
        let caret_visible = workspace.caret_visible;
        let overtype = workspace.overtype;
        let scroll_top = workspace.scroll_top();
        let scroll_left = if word_wrap { px(0.) } else { workspace.scroll_left() };
        // Notepad++ column-mode (block) selection. When `Some` AND wrap
        // is off, paint switches to per-line rectangles and the linear
        // selection rect is suppressed. Block + wrap together is a v2
        // feature — when wrap is on, block paint is skipped.
        let block_active = workspace
            .block_selection()
            .filter(|b| !b.is_empty() && !word_wrap);

        // Snapshot match data so we can paint the find highlights without
        // re-borrowing the workspace per line.
        let match_ranges: Vec<Range<usize>> = if workspace.find_state.open {
            workspace.find_state.matches.clone()
        } else {
            Vec::new()
        };
        let current_match_idx = workspace.find_state.current;

        // "Mark All" highlights. Source-of-truth lives on the active buffer's
        // `marks` layer (per-buffer, edit-tracked); MarkState.marks is the
        // cache the dialog reads. Paint a union of both so the legacy "Find
        // All in Current Document" path that writes only into MarkState still
        // shows highlights on the active buffer.
        let mut mark_ranges: Vec<Range<usize>> = buffer.marks.clone();
        for r in &workspace.mark_state.marks {
            if !mark_ranges
                .iter()
                .any(|m| m.start == r.start && m.end == r.end)
            {
                mark_ranges.push(r.clone());
            }
        }

        // Smart Highlighting band — every other occurrence of the word at
        // cursor. Empty when smart-hilite is off, when the cursor isn't on a
        // word, or when the buffer exceeds the smart-hilite size cutoff.
        let smart_ranges: Vec<Range<usize>> = workspace.smart_match_ranges.clone();
        // HTML/XML matching-tag highlight — both halves' tag-name ranges
        // get a fill quad. Empty when the cursor isn't inside a tag, when
        // the buffer isn't HTML/XML, or when match_tags_enabled is off.
        // Sub-toggles fold in too: attribute name+value ranges (when
        // "Highlight tag attributes" is on) and comment/CDATA/PI/ASP zone
        // delimiters (when "Highlight comment/PHP/ASP zone" is on) share
        // the same paint path so they get the same smart-highlight colour.
        let mut tag_match_ranges: Vec<Range<usize>> = Vec::new();
        if let Some(p) = workspace.tag_match_pair.as_ref() {
            tag_match_ranges.push(p.open.clone());
            tag_match_ranges.push(p.close.clone());
        }
        tag_match_ranges.extend(workspace.tag_match_attrs.iter().cloned());
        if let Some(p) = workspace.tag_match_zone.as_ref() {
            tag_match_ranges.push(p.open.clone());
            tag_match_ranges.push(p.close.clone());
        }

        // Step 3 — assemble the list of *visual rows* that fall inside the
        // viewport. Wrap-off mode produces exactly one entry per logical
        // line; wrap-on mode reads the cached `WrapRow`s. Both modes feed
        // the same per-row paint loop below.
        // Float-to-int `as` saturates, so a zero line height gives
        // usize::MAX here; keep the sums below from overflowing.
        let first_visual_row = (scroll_top / line_height).floor() as usize;
        let visible_count = ((bounds.size.height / line_height).ceil() as usize).saturating_add(1);

        let total_visual_rows = if word_wrap {
            workspace
                .tabs[workspace.active]
                .wrap_cache
                .as_ref()
                .map(|c| c.total_rows as usize)
                .unwrap_or(total_lines)
        } else {
            total_lines
        };
        let last_visual_row = first_visual_row.saturating_add(visible_count).min(total_visual_rows);
        let first_visual_row = first_visual_row.min(total_visual_rows.saturating_sub(1));

        let style_font = font.clone();
        let mut row_specs: Vec<VisibleRowSpec> = Vec::with_capacity(
            last_visual_row.saturating_sub(first_visual_row).max(1),
        );
        let mut max_line_width = px(0.);

        if word_wrap {
            // Pull the slice of cached rows that overlaps the viewport.
            let wrap_rows: Vec<WrapRow> = workspace
                .tabs[workspace.active]
                .wrap_cache
                .as_ref()
                .map(|c| {
                    c.rows
                        .get(first_visual_row..last_visual_row)
                        .map(|s| s.to_vec())
                        .unwrap_or_default()
                })
                .unwrap_or_default();

            for (i, row) in wrap_rows.iter().enumerate() {
                let row_idx = first_visual_row + i;
                let logical_line = row.logical_line as usize;
                // A row from a cache that outlived the rope can name a line
                // that no longer exists.
                let Some(line_slice) = buffer.rope.get_line(logical_line) else {
                    continue;
                };
                let line_byte_start = buffer.rope.line_to_byte(logical_line);
                let raw = line_slice.to_string();
                let trimmed = raw.trim_end_matches(['\n', '\r']);
                let full_line_byte_len = trimmed.len();

                // Slice the trimmed line down to just this row's bytes.
                let (seg_offset_in_line, seg_text) =
                    wrap_row_text(trimmed, line_byte_start, row);
                let seg_byte_start = line_byte_start + seg_offset_in_line;
                let seg_byte_len = seg_text.len();

                let mut runs: Vec<TextRun> = if seg_text.is_empty() {
                    Vec::new()
                } else {
                    // Build runs against the absolute byte range of this
                    // segment so syntax highlights still line up with the
                    // wrapped sub-text.
                    build_line_runs(
                        line_byte_start + seg_offset_in_line,
                        seg_text,
                        &buffer.highlights,
                        style_font.clone(),
                        text_color,
                        &theme.syntax,
                    )
                };
                proofreading::apply(&mut runs, seg_text, seg_byte_start, workspace.proofreading_issues(), &theme);
                let url_ranges_in_seg: Vec<Range<usize>> =
                    if url_underline_enabled && !seg_text.is_empty() {
                        let urls = url_scan::find_urls(seg_text);
                        apply_url_underlines(&mut runs, seg_text, url_link_color, url_with_underline);
                        urls
                    } else {
                        Vec::new()
                    };
                let display: SharedString = seg_text.to_string().into();
                let shaped = window
                    .text_system()
                    .shape_line(display, font_size, &runs, None);
                if shaped.width > max_line_width {
                    max_line_width = shaped.width;
                }
                let line_top = bounds.top() + line_height * (row_idx as f32) - scroll_top;
                row_specs.push(VisibleRowSpec {
                    seg_byte_start,
                    seg_byte_len,
                    seg_offset_in_line,
                    full_line_byte_len,
                    full_line_byte_start: line_byte_start,
                    line_top,
                    shaped,
                    url_ranges_in_seg,
                });
            }
        } else {
            // Wrap-off: one visual row per logical line in the viewport.
            for line_idx in first_visual_row..last_visual_row {
                let line_byte_start = buffer.rope.line_to_byte(line_idx);
                let line_slice = buffer.rope.line(line_idx);
                let raw = line_slice.to_string();
                let trimmed = raw.trim_end_matches(['\n', '\r']);
                let line_byte_len = trimmed.len();

                let display: SharedString = if line_byte_len == 0 {
                    "".into()
                } else {
                    trimmed.to_string().into()
                };

                let mut runs: Vec<TextRun> = if display.is_empty() {
                    Vec::new()
                } else {
                    build_line_runs(
                        line_byte_start,
                        trimmed,
                        &buffer.highlights,
                        style_font.clone(),
                        text_color,
                        &theme.syntax,
                    )
                };
                proofreading::apply(&mut runs, trimmed, line_byte_start, workspace.proofreading_issues(), &theme);
                let url_ranges_in_seg: Vec<Range<usize>> =
                    if url_underline_enabled && !display.is_empty() {
                        let urls = url_scan::find_urls(trimmed);
                        apply_url_underlines(&mut runs, trimmed, url_link_color, url_with_underline);
                        urls
                    } else {
                        Vec::new()
                    };
                let shaped = window
                    .text_system()
                    .shape_line(display, font_size, &runs, None);
                if shaped.width > max_line_width {
                    max_line_width = shaped.width;
                }
                let line_top = bounds.top() + line_height * (line_idx as f32) - scroll_top;
                row_specs.push(VisibleRowSpec {
                    seg_byte_start: line_byte_start,
                    seg_byte_len: line_byte_len,
                    seg_offset_in_line: 0,
                    full_line_byte_len: line_byte_len,
                    full_line_byte_start: line_byte_start,
                    line_top,
                    shaped,
                    url_ranges_in_seg,
                });
            }
        }

        // Pre-allocate output containers.
        let mut lines = Vec::with_capacity(row_specs.len());
        let mut selection_quads = Vec::new();
        let mut match_quads: Vec<PaintQuad> = Vec::new();
        let mut mark_quads: Vec<PaintQuad> = Vec::new();
        let mut current_line_quads: Vec<PaintQuad> = Vec::new();
        let mut caret: Option<PaintQuad> = None;
        let mut url_hitboxes: Vec<gpui::Hitbox> = Vec::new();

        // Logical-line-start byte of the line containing the caret. Used by
        // the current-line backdrop to mark every visual row of the wrapped
        // logical line (wrap-off → exactly one row, wrap-on → N rows). For
        // column-mode block selection the caret follows `block.head_line`
        // instead of `head`, so the highlight tracks the block's top-right
        // corner the same way the linear caret tracks the head byte.
        let current_line_start: Option<usize> = if highlight_current_line {
            if let Some(block) = block_active.as_ref() {
                let line_idx = block.head_line.min(total_lines.saturating_sub(1));
                Some(buffer.rope.line_to_byte(line_idx))
            } else {
                let line_idx = buffer.rope.byte_to_line(head.min(buffer.rope.len_bytes()));
                Some(buffer.rope.line_to_byte(line_idx))
            }
        } else {
            None
        };

        let line_left_base = bounds.left() - scroll_left;

        // Width of one virtual-column space — needed when a column-mode
        // block extends past a line's printable end, and as the overtype
        // caret's width at a line end. Shape once outside the row loop.
        // Costs nothing when neither is active.
        let virtual_space_width: Pixels = if block_active.is_some() || overtype {
            let space_runs = vec![TextRun {
                len: 1,
                font: font.clone(),
                color: text_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let shaped = window
                .text_system()
                .shape_line(" ".into(), font_size, &space_runs, None);
            shaped.width
        } else {
            px(0.)
        };

        for spec in row_specs.into_iter() {
            let shaped = spec.shaped;
            let seg_start = spec.seg_byte_start;
            let seg_len = spec.seg_byte_len;
            let seg_end = seg_start + seg_len;
            let seg_offset = spec.seg_offset_in_line;
            let full_line_start = spec.full_line_byte_start;
            let full_line_byte_len = spec.full_line_byte_len;
            let line_top = spec.line_top;
            let url_ranges_in_seg = spec.url_ranges_in_seg;
            let line_bounds = Bounds::new(
                point(line_left_base, line_top),
                size(bounds.size.width, line_height),
            );

            // Current-line backdrop — emit one quad per visual row whose
            // logical line equals the caret's. Spans the full viewport
            // width (anchored at `bounds.left()`, not `line_left_base`, so
            // horizontal scroll doesn't slide the band off the left edge).
            if let Some(line_start) = current_line_start {
                if full_line_start == line_start {
                    current_line_quads.push(fill(
                        Bounds::new(
                            point(bounds.left(), line_top),
                            size(bounds.size.width, line_height),
                        ),
                        current_line_color,
                    ));
                }
            }

            // URL hover hitboxes — one per URL on this visual row. Paint
            // phase uses them to flip the cursor to `PointingHand` whenever
            // the platform modifier (Cmd on macOS / Ctrl elsewhere) is held.
            for url in url_ranges_in_seg {
                let lo = url.start.min(seg_len);
                let hi = url.end.min(seg_len);
                if hi <= lo {
                    continue;
                }
                let hx_start = line_left_base + shaped.x_for_index(lo);
                let hx_end = line_left_base + shaped.x_for_index(hi);
                if hx_end <= hx_start {
                    continue;
                }
                let hb_bounds = Bounds::new(
                    point(hx_start, line_top),
                    size(hx_end - hx_start, line_height),
                );
                url_hitboxes.push(
                    window.insert_hitbox(hb_bounds, gpui::HitboxBehavior::Normal),
                );
            }

            // Mark highlights — clamp each range to this row's byte slice.
            for r in &mark_ranges {
                if r.start >= seg_end || r.end <= seg_start {
                    continue;
                }
                let local_start = r.start.saturating_sub(seg_start).min(seg_len);
                let local_end = (r.end.min(seg_end)).saturating_sub(seg_start);
                if local_end <= local_start {
                    continue;
                }
                let mx_start = line_left_base + shaped.x_for_index(local_start);
                let mx_end = line_left_base + shaped.x_for_index(local_end);
                if mx_end <= mx_start {
                    continue;
                }
                mark_quads.push(fill(
                    Bounds::new(
                        point(mx_start, line_top),
                        size(mx_end - mx_start, line_height),
                    ),
                    mark_color,
                ));
            }

            // Smart Highlighting band — same clamp/paint pattern as marks.
            for r in &smart_ranges {
                if r.start >= seg_end || r.end <= seg_start {
                    continue;
                }
                let local_start = r.start.saturating_sub(seg_start).min(seg_len);
                let local_end = (r.end.min(seg_end)).saturating_sub(seg_start);
                if local_end <= local_start {
                    continue;
                }
                let mx_start = line_left_base + shaped.x_for_index(local_start);
                let mx_end = line_left_base + shaped.x_for_index(local_end);
                if mx_end <= mx_start {
                    continue;
                }
                mark_quads.push(fill(
                    Bounds::new(
                        point(mx_start, line_top),
                        size(mx_end - mx_start, line_height),
                    ),
                    smart_color,
                ));
            }

            // HTML/XML matching-tag highlight — same clamp/paint pattern.
            for r in &tag_match_ranges {
                if r.start >= seg_end || r.end <= seg_start {
                    continue;
                }
                let local_start = r.start.saturating_sub(seg_start).min(seg_len);
                let local_end = (r.end.min(seg_end)).saturating_sub(seg_start);
                if local_end <= local_start {
                    continue;
                }
                let mx_start = line_left_base + shaped.x_for_index(local_start);
                let mx_end = line_left_base + shaped.x_for_index(local_end);
                if mx_end <= mx_start {
                    continue;
                }
                mark_quads.push(fill(
                    Bounds::new(
                        point(mx_start, line_top),
                        size(mx_end - mx_start, line_height),
                    ),
                    smart_color,
                ));
            }

            // Find-match highlights.
            for (idx, m) in match_ranges.iter().enumerate() {
                if m.start >= seg_end || m.end <= seg_start {
                    continue;
                }
                let local_start = m.start.saturating_sub(seg_start).min(seg_len);
                let local_end = (m.end.min(seg_end)).saturating_sub(seg_start);
                if local_end <= local_start {
                    continue;
                }
                let mx_start = line_left_base + shaped.x_for_index(local_start);
                let mx_end = line_left_base + shaped.x_for_index(local_end);
                if mx_end <= mx_start {
                    continue;
                }
                let color = if Some(idx) == current_match_idx {
                    match_current_color
                } else {
                    match_color
                };
                match_quads.push(fill(
                    Bounds::new(
                        point(mx_start, line_top),
                        size(mx_end - mx_start, line_height),
                    ),
                    color,
                ));
            }

            // Notepad++ column-mode block paint. Only paint on the first
            // visual row of a logical line (`seg_offset == 0`), so wrap-on
            // mode (which has multiple visual rows per logical line) just
            // skips block paint — gated on `!word_wrap` via `block_active`
            // already, but the seg_offset guard keeps this robust if a
            // future tweak feeds wrap rows in.
            if let Some(block) = block_active.as_ref() {
                let logical_line = buffer.rope.byte_to_line(full_line_start);
                if logical_line >= block.min_line()
                    && logical_line <= block.max_line()
                    && seg_offset == 0
                {
                    let line_chars = workspace.line_char_count(logical_line);
                    let block_min_col = block.min_col();
                    let block_max_col = block.max_col();

                    // Real (non-virtual) byte offsets within the line.
                    let real_min_col = block_min_col.min(line_chars);
                    let real_max_col = block_max_col.min(line_chars);
                    let byte_min = workspace.byte_at_line_col(logical_line, real_min_col);
                    let byte_max = workspace.byte_at_line_col(logical_line, real_max_col);
                    let local_min = byte_min.saturating_sub(seg_start).min(seg_len);
                    let local_max = byte_max.saturating_sub(seg_start).min(seg_len);

                    let mut x_start = line_left_base + shaped.x_for_index(local_min);
                    let mut x_end = line_left_base + shaped.x_for_index(local_max);

                    // Extend into virtual column space when the block
                    // reaches past EOL on either end.
                    if block_min_col > line_chars {
                        let extra = block_min_col - line_chars;
                        x_start += virtual_space_width * (extra as f32);
                    }
                    if block_max_col > line_chars {
                        let virt_start = line_chars.max(block_min_col);
                        let extra = block_max_col - virt_start;
                        let line_end_x = line_left_base + shaped.x_for_index(seg_len);
                        let virt_base = if block_min_col > line_chars {
                            x_start
                        } else {
                            line_end_x
                        };
                        x_end = virt_base + virtual_space_width * (extra as f32);
                    }

                    if x_end > x_start {
                        selection_quads.push(fill(
                            Bounds::new(
                                point(x_start, line_top),
                                size(x_end - x_start, line_height),
                            ),
                            selection_color,
                        ));
                    }
                }
            }

            // Linear selection rectangle (skipped when a block is active —
            // block paint above takes over). The "extends past EOL" pad
            // only applies to the *last* visual row of a wrapped logical
            // line — middle rows always end in the middle of the line so
            // the selection shouldn't get the right-edge pad.
            let line_with_nl_end = full_line_start + full_line_byte_len;
            let is_last_row_of_line = seg_offset + seg_len == full_line_byte_len;
            if block_active.is_none()
                && selection_range.start < line_with_nl_end
                && selection_range.end > full_line_start
                && selection_range.start < seg_end
                && selection_range.end > seg_start
            {
                let local_start = selection_range
                    .start
                    .saturating_sub(seg_start)
                    .min(seg_len);
                let sel_extends_past_seg = selection_range.end > seg_end;
                let local_end = if sel_extends_past_seg {
                    seg_len
                } else {
                    selection_range.end - seg_start
                };

                let x_start = line_left_base + shaped.x_for_index(local_start);
                let x_end_in_line = line_left_base + shaped.x_for_index(local_end);
                let x_end = if is_last_row_of_line
                    && selection_range.end > full_line_start + full_line_byte_len
                {
                    let pad = px(8.);
                    (x_end_in_line + pad).min(bounds.right())
                } else if sel_extends_past_seg {
                    // Selection continues onto the next visual row — extend
                    // to the row's right edge so the wrap is visually
                    // continuous.
                    bounds.right()
                } else {
                    x_end_in_line
                };

                if x_end > x_start {
                    selection_quads.push(fill(
                        Bounds::new(
                            point(x_start, line_top),
                            size(x_end - x_start, line_height),
                        ),
                        selection_color,
                    ));
                }
            }

            // Block-mode caret. Paints at the block's head corner so a
            // virtual column past EOL still shows the caret in its
            // virtual position. Runs before the linear caret check so
            // `caret.is_none()` skips the linear path below.
            if let Some(block) = block_active.as_ref() {
                let logical_line = buffer.rope.byte_to_line(full_line_start);
                if caret_visible
                    && caret.is_none()
                    && logical_line == block.head_line
                    && seg_offset == 0
                {
                    let line_chars = workspace.line_char_count(logical_line);
                    let head_col = block.head_col;
                    let real_col = head_col.min(line_chars);
                    let byte_at_real = workspace.byte_at_line_col(logical_line, real_col);
                    let local = byte_at_real.saturating_sub(seg_start).min(seg_len);
                    let mut cx_pos = line_left_base + shaped.x_for_index(local);
                    if head_col > line_chars {
                        cx_pos += virtual_space_width * ((head_col - line_chars) as f32);
                    }
                    caret = Some(fill(
                        Bounds::new(
                            point(cx_pos, line_top),
                            size(px(CARET_WIDTH_PX), line_height),
                        ),
                        caret_color,
                    ));
                }
            }

            // Linear caret. Only emit on the visual row that actually
            // contains the head. For wrapped lines the head sits on the
            // row whose [seg_start, seg_end) range covers it; the row
            // whose seg_end *equals* head also wins for end-of-row
            // positioning, except when there's a following row of the
            // same logical line — in that case the head paints at the
            // start of the next row so a visual line break never leaves
            // the caret hanging off the right edge with no glyph.
            if caret_visible
                && selection_empty
                && caret.is_none()
                && head >= seg_start
                && head <= seg_end
            {
                let head_at_seg_end = head == seg_end;
                let row_continues_line = !is_last_row_of_line && seg_len > 0;
                // Skip on this row only if the next row of the same line
                // exists AND the head is exactly at the wrap point (seg_end).
                // If head equals the very end of the line, paint here.
                let head_at_logical_end = head == full_line_start + full_line_byte_len;
                let skip_for_wrap_handoff =
                    head_at_seg_end && row_continues_line && !head_at_logical_end;
                if !skip_for_wrap_handoff {
                    let local_x = head - seg_start;
                    let head_x = shaped.x_for_index(local_x);
                    let cx_pos = line_left_base + head_x;
                    // The overtype bar spans the character a keystroke will
                    // replace; at a row end Scintilla falls back to an
                    // average character width.
                    let overstrike_width = overtype.then(|| {
                        if head < seg_end {
                            let rope = &buffer.rope;
                            let next = rope
                                .char_to_byte(rope.byte_to_char(head) + 1)
                                .min(seg_end);
                            shaped.x_for_index(next - seg_start) - head_x
                        } else {
                            virtual_space_width
                        }
                    });
                    caret = Some(fill(
                        caret_bounds(cx_pos, line_top, line_height, overstrike_width),
                        caret_color,
                    ));
                }
            }

            lines.push(EditorLineLayout {
                shaped,
                line_byte_start: seg_start,
                line_byte_len: seg_len,
                bounds: line_bounds,
            });
        }

        // Edge case: head sits past the last shaped row (rope ends with a
        // newline and `head == rope.len_bytes()`). Only emit the trailing
        // caret if the visible window includes that synthetic last position.
        if caret_visible
            && selection_empty
            && caret.is_none()
            && head == buffer.rope.len_bytes()
            && last_visual_row == total_visual_rows
        {
            if let Some(last) = lines.last() {
                let cx_pos = line_left_base + last.shaped.x_for_index(0);
                let line_top = last.bounds.top() + line_height;
                caret = Some(fill(
                    caret_bounds(
                        cx_pos,
                        line_top,
                        line_height,
                        overtype.then_some(virtual_space_width),
                    ),
                    caret_color,
                ));
            }
        }

        let whitespace_glyphs = if workspace.show_whitespace {
            prepaint_whitespace_glyphs(
                &lines,
                buffer,
                font_size,
                font.clone(),
                Rgba::from(rgb(theme.whitespace_glyph)).into(),
                bounds.left() - scroll_left,
                window,
            )
        } else {
            Vec::new()
        };

        // Editing 2 panel toggles. Snapshot once per prepaint to avoid the
        // cross-process settings poller racing with paint.
        let snap = crate::settings::snapshot();
        let glyph_color: Hsla = Rgba::from(rgb(theme.whitespace_glyph)).into();
        let eol_glyphs = if snap.eol_visible {
            prepaint_eol_glyphs(
                &lines,
                buffer,
                font_size,
                font.clone(),
                glyph_color,
                bounds.left() - scroll_left,
                matches!(
                    snap.eol_display_mode,
                    crate::settings::EolDisplayMode::RoundedRect
                ),
                window,
            )
        } else {
            Vec::new()
        };
        let npc_glyphs = if snap.npc_visible {
            prepaint_npc_glyphs(
                &lines,
                buffer,
                font_size,
                font.clone(),
                glyph_color,
                bounds.left() - scroll_left,
                matches!(
                    snap.npc_display_mode,
                    crate::settings::NpcDisplayMode::Codepoint
                ),
                snap.npc_include_cc_uni_eol,
                window,
            )
        } else {
            Vec::new()
        };

        // Drawing consumes the worker cache, including cached unmatched results.
        let mut bracket_quads: Vec<PaintQuad> = Vec::new();
        if let Some((left, right)) = workspace.cached_brace_pair(window) {
            let bracket_color: Hsla = Rgba::from(rgb(theme.bracket_match)).into();
            let line_left = bounds.left() - scroll_left;
            bracket_outline_for_byte(
                left,
                &lines,
                line_left,
                line_height,
                bracket_color,
                &mut bracket_quads,
            );
            bracket_outline_for_byte(
                right,
                &lines,
                line_left,
                line_height,
                bracket_color,
                &mut bracket_quads,
            );
        }

        // Deferred popups prepaint before the editor paints. Publish this
        // frame's geometry now, after edits have cleared the old cache.
        let hit_geo = lines.iter().map(|line| LineGeo {
            bounds: line.bounds,
            line_byte_start: line.line_byte_start,
            line_byte_len: line.line_byte_len,
            shaped: line.shaped.clone(),
        }).collect();
        self.workspace.update(cx, |w, _| {
            w.last_lines = hit_geo;
            w.viewport.bounds = bounds;
            w.viewport.line_height = line_height;
            w.viewport.total_lines = total_lines;
            w.viewport.max_line_width = w.viewport.max_line_width.max(max_line_width);
        });

        EditorPrepaint {
            lines,
            selection_quads,
            current_line_quads,
            match_quads,
            mark_quads,
            bracket_quads,
            caret,
            whitespace_glyphs,
            eol_glyphs,
            npc_glyphs,
            url_hitboxes,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.workspace.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            crate::workspace::source_input::SourceInputHandler::new(bounds, self.workspace.clone(), window, cx),
            cx,
        );

        // Clip child painting to the editor pane. Without this, lines that
        // partially overflow the bottom (during a partial-line scroll) bleed
        // into the status bar.
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            // Current-line backdrop paints first so every other overlay (mark
            // / find / smart / tag-match / selection / bracket frame) reads
            // on top of it. Empty when `highlight_current_line` is off.
            for quad in prepaint.current_line_quads.drain(..) {
                window.paint_quad(quad);
            }
            // Mark highlights paint first — sit beneath find matches and
            // selection so a live find tint or drag-select visually wins.
            for quad in prepaint.mark_quads.drain(..) {
                window.paint_quad(quad);
            }
            // Find-match highlights paint next so the selection layers on top.
            for quad in prepaint.match_quads.drain(..) {
                window.paint_quad(quad);
            }

            for quad in prepaint.selection_quads.drain(..) {
                window.paint_quad(quad);
            }

            // Bracket-pair outlines paint above the selection layer so the
            // 1px frame stays readable even while a selection covers the
            // bracket cell, and below the line glyphs so the bracket char
            // reads through the (transparent-fill) frame untouched. Empty
            // when the caret isn't sitting next to a bracket — costs zero.
            for quad in prepaint.bracket_quads.drain(..) {
                window.paint_quad(quad);
            }

            let line_height = window.line_height();
            for line in &prepaint.lines {
                let _ = line.shaped.paint(
                    line.bounds.origin,
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            // Show-whitespace overlay. Paint glyphs immediately after the
            // line text so they read as belonging to the line (same paint
            // phase, same clip mask) without re-arranging the existing
            // selection / match / mark layering above.
            for glyph in prepaint.whitespace_glyphs.drain(..) {
                let _ = glyph.shaped.paint(
                    glyph.origin,
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            // EOL overlay (Editing 2 → Show EOL). Bg paints first when the
            // rounded-rectangle mode is on; text paints over.
            for glyph in prepaint.eol_glyphs.drain(..) {
                if let Some(bg) = glyph.bg {
                    window.paint_quad(bg);
                }
                let _ = glyph.shaped.paint(
                    glyph.origin,
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            // NPC overlay (Editing 2 → Show Non-Printing Characters).
            for glyph in prepaint.npc_glyphs.drain(..) {
                let _ = glyph.shaped.paint(
                    glyph.origin,
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            if focus_handle.is_focused(window) {
                if let Some(caret) = prepaint.caret.take() {
                    window.paint_quad(caret);
                }
            }
        });

        // URL hover cursor — only when the same modifier the click handler
        // checks (`event.modifiers.platform` in `workspace::mouse`) is held.
        // The hitboxes were created in prepaint; binding `PointingHand` to
        // each only re-routes the OS cursor when the pointer is over it, so
        // mass-installing them costs nothing per non-hovered frame.
        let cmd_held = window.modifiers().platform;
        if cmd_held {
            for hitbox in prepaint.url_hitboxes.iter() {
                window.set_cursor_style(gpui::CursorStyle::PointingHand, hitbox);
            }
        }
        // Re-paint when the modifier toggles so the cursor flips even when
        // the user is stationary over a URL — without this the cursor would
        // only update on the next mouse-move.
        let modkey_workspace = self.workspace.clone();
        let prev_cmd_held = cmd_held;
        window.on_modifiers_changed(move |event, _window, cx| {
            if event.modifiers.platform != prev_cmd_held {
                modkey_workspace.update(cx, |_, cx| cx.notify());
            }
        });

        // Window-level mouse capture: keeps drag-to-select alive when the cursor
        // wanders outside the editor's div (or even outside the window). Listeners
        // are re-registered every frame, so they don't accumulate.
        let drag_workspace = self.workspace.clone();
        let drag_bounds = bounds;
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            drag_workspace.update(cx, |w, cx| {
                // Selected-text drag-and-drop. If we have a pending text drag
                // and the pointer has cleared the start threshold, promote
                // it to Active. While Active, update drop_target on every
                // move. Block this BEFORE the regular selection drag so we
                // don't extend the selection underneath the drag.
                if let Some(state) = w.text_drag.clone() {
                    match state.phase {
                        crate::workspace::TextDragPhase::Pending => {
                            let dx = f32::from(event.position.x - state.start_position.x).abs();
                            let dy = f32::from(event.position.y - state.start_position.y).abs();
                            if dx >= crate::workspace::TEXT_DRAG_THRESHOLD_PX
                                || dy >= crate::workspace::TEXT_DRAG_THRESHOLD_PX
                            {
                                let offset = w.offset_at_position(event.position);
                                if let Some(td) = w.text_drag.as_mut() {
                                    td.phase = crate::workspace::TextDragPhase::Active;
                                    td.drop_target = offset;
                                }
                                w.caret_visible = true;
                                cx.notify();
                            }
                            return;
                        }
                        crate::workspace::TextDragPhase::Active => {
                            let offset = w.offset_at_position(event.position);
                            if let Some(td) = w.text_drag.as_mut() {
                                if td.drop_target != offset {
                                    td.drop_target = offset;
                                    w.caret_visible = true;
                                    cx.notify();
                                }
                            }
                            return;
                        }
                    }
                }
                // Block-mode (alt-drag) — track head_line/head_col on the
                // active tab's block_selection. Skip the linear path
                // entirely so a block-drag doesn't also extend the linear
                // selection.
                let block_dragging = w
                    .block_selection()
                    .as_ref()
                    .is_some_and(|b| b.is_dragging);
                if block_dragging {
                    if w.last_lines.is_empty() {
                        return;
                    }
                    let above = event.position.y - drag_bounds.top();
                    let below = event.position.y - drag_bounds.bottom();
                    let viewport_h = drag_bounds.size.height;
                    if above < px(0.) {
                        let dist = -f32::from(above);
                        let delta = px((dist * 0.4).min(f32::from(viewport_h)).max(2.));
                        let new_top = w.scroll_top() - delta;
                        w.set_scroll_top(new_top, cx);
                    } else if below > px(0.) {
                        let dist = f32::from(below);
                        let delta = px((dist * 0.4).min(f32::from(viewport_h)).max(2.));
                        let new_top = w.scroll_top() + delta;
                        w.set_scroll_top(new_top, cx);
                    }
                    let offset = w.offset_at_position(event.position);
                    let (line, col) = w.line_col_at_byte(offset);
                    if let Some(mut block) = w.block_selection() {
                        if block.head_line != line || block.head_col != col {
                            block.head_line = line;
                            block.head_col = col;
                            w.set_block_selection(Some(block));
                            let active = w.active;
                            w.tabs[active].selection.anchor = offset;
                            w.tabs[active].selection.head = offset;
                            w.retire_source_input();
                            w.caret_visible = true;
                            cx.notify();
                        }
                    }
                    return;
                }
                if !w.selection().is_dragging || w.last_lines.is_empty() {
                    return;
                }
                // Edge auto-scroll: if the user's drag has wandered past the
                // top or bottom of the editor, nudge the viewport so more
                // content comes into view. Speed scales with how far past
                // the edge the cursor is, capped at one viewport-height
                // per event so a fast flick doesn't teleport the document.
                let above = event.position.y - drag_bounds.top();
                let below = event.position.y - drag_bounds.bottom();
                let viewport_h = drag_bounds.size.height;
                if above < px(0.) {
                    let dist = -f32::from(above);
                    let delta = px((dist * 0.4).min(f32::from(viewport_h)).max(2.));
                    let new_top = w.scroll_top() - delta;
                    w.set_scroll_top(new_top, cx);
                } else if below > px(0.) {
                    let dist = f32::from(below);
                    let delta = px((dist * 0.4).min(f32::from(viewport_h)).max(2.));
                    let new_top = w.scroll_top() + delta;
                    w.set_scroll_top(new_top, cx);
                }
                let offset = w.offset_at_position(event.position);
                // Drag mode picked at mouse-down by click count. Word/Line
                // modes snap the head to the outside edge of the word/line
                // under the pointer relative to the anchor word/line so the
                // selection always covers whole units, NPP-style.
                let (new_anchor, new_head) = match (w.drag_mode, w.drag_anchor_range.clone()) {
                    (crate::workspace::DragMode::Word, Some(anchor)) => {
                        let cur = w.word_range_at(offset);
                        if cur.start >= anchor.end {
                            (anchor.start, cur.end)
                        } else if cur.end <= anchor.start {
                            (anchor.end, cur.start)
                        } else {
                            (anchor.start, anchor.end)
                        }
                    }
                    (crate::workspace::DragMode::Line, Some(anchor)) => {
                        let cur = w.line_range_at(offset);
                        if cur.start >= anchor.end {
                            (anchor.start, cur.end)
                        } else if cur.end <= anchor.start {
                            (anchor.end, cur.start)
                        } else {
                            (anchor.start, anchor.end)
                        }
                    }
                    _ => (w.selection().anchor, offset),
                };
                let cur_sel = w.selection();
                if cur_sel.head != new_head || cur_sel.anchor != new_anchor {
                    let tab = w.active_tab_mut();
                    tab.selection.anchor = new_anchor;
                    tab.selection.head = new_head;
                    w.retire_source_input();
                    w.caret_visible = true;
                    cx.notify();
                }
            });
        });

        let release_workspace = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            release_workspace.update(cx, |w, cx| {
                // Selected-text drag-and-drop release. Pending = treat as
                // plain click (collapse the selection to the click point).
                // Active = perform the move / copy.
                if let Some(state) = w.text_drag.take() {
                    match state.phase {
                        crate::workspace::TextDragPhase::Pending => {
                            // No movement — collapse selection at the click.
                            let tab = w.active_tab_mut();
                            tab.selection.anchor = state.drop_target;
                            tab.selection.head = state.drop_target;
                            tab.selection.is_dragging = false;
                            w.retire_source_input();
                            w.caret_visible = true;
                            cx.notify();
                        }
                        crate::workspace::TextDragPhase::Active => {
                            w.commit_text_drag(state, event.modifiers.platform, cx);
                        }
                    }
                    return;
                }
                if let Some(mut block) = w.block_selection() {
                    if block.is_dragging {
                        block.is_dragging = false;
                        w.set_block_selection(Some(block));
                        if w.block_selection().as_ref().is_some_and(|b| b.is_empty()) {
                            // Click-without-drag in alt mode → no rectangle.
                            // Drop the block so the linear caret takes over.
                            w.set_block_selection(None);
                        }
                        cx.notify();
                    }
                }
                if w.selection().is_dragging {
                    w.active_tab_mut().selection.is_dragging = false;
                    w.sync_source_input_caret();
                    w.drag_mode = crate::workspace::DragMode::Char;
                    w.drag_anchor_range = None;
                    cx.notify();
                }
            });
        });

        // Mouse-wheel + trackpad scrolling. gpui convention is `scroll_offset
        // += delta`, where positive delta.y means content moves DOWN (and
        // scroll_top should DECREASE). We invert that to drive scroll_top
        // directly.
        let wheel_workspace = self.workspace.clone();
        let wheel_line_height = window.line_height();
        let editor_bounds = bounds;
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !editor_bounds.contains(&event.position) {
                return;
            }
            let delta = event.delta.pixel_delta(wheel_line_height);
            wheel_workspace.update(cx, |w, cx| {
                if delta.y != px(0.) {
                    let new_top = w.scroll_top() - delta.y;
                    w.set_scroll_top(new_top, cx);
                }
                if delta.x != px(0.) && !w.word_wrap {
                    let new_left = w.scroll_left() - delta.x;
                    w.set_scroll_left(new_left, cx);
                }
            });
        });
    }
}

/// The caret rectangle at `x` on the row that starts at `line_top`. In
/// overtype, `overstrike_width` is the width of the character the next
/// keystroke replaces, and the caret becomes Scintilla's overstrike bar:
/// 2px tall at the row bottom, 1px right of the caret, as wide as that
/// character (at least 3px) less 1px (scintilla/src/EditView.cxx:1533-1551).
fn caret_bounds(
    x: Pixels,
    line_top: Pixels,
    line_height: Pixels,
    overstrike_width: Option<Pixels>,
) -> Bounds<Pixels> {
    let Some(width) = overstrike_width else {
        return Bounds::new(point(x, line_top), size(px(CARET_WIDTH_PX), line_height));
    };
    let width = width.max(px(OVERSTRIKE_CARET_MIN_WIDTH_PX)) - px(1.);
    let height = px(OVERSTRIKE_CARET_HEIGHT_PX);
    Bounds::new(point(x + px(1.), line_top + line_height - height), size(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_mode_caret_is_a_full_height_bar() {
        let b = caret_bounds(px(10.), px(40.), px(20.), None);
        assert_eq!(b.origin, point(px(10.), px(40.)));
        assert_eq!(b.size, size(px(CARET_WIDTH_PX), px(20.)));
    }

    #[test]
    fn overtype_caret_underlines_the_next_character() {
        let b = caret_bounds(px(10.), px(40.), px(20.), Some(px(8.)));
        assert_eq!(b.origin, point(px(11.), px(58.)));
        assert_eq!(b.size, size(px(7.), px(2.)));
        // A zero-width or narrow character still gets a visible bar.
        let b = caret_bounds(px(10.), px(40.), px(20.), Some(px(0.)));
        assert_eq!(b.size, size(px(2.), px(2.)));
    }

    fn span(range: Range<usize>, kind: HighlightKind) -> HighlightSpan {
        HighlightSpan { range, kind }
    }

    /// Slice `text` at every run boundary, as the platform shapers do
    /// (`&text[a..b]` in DirectWrite, `split_at` in CoreText).
    fn slice_like_shaper<'a>(text: &'a str, runs: &[TextRun]) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut at = 0;
        for run in runs {
            out.push(&text[at..at + run.len]);
            at += run.len;
        }
        assert_eq!(at, text.len(), "runs must cover the whole line");
        out
    }

    #[test]
    fn stale_spans_never_split_a_char() {
        // Spans lag edits until the debounced reparse lands: `foo` was
        // highlighted as 0..3, then `é` was typed after `fo`.
        let text = "fo\u{e9}o(x)";
        let spans = [
            span(0..3, HighlightKind::Keyword),
            span(4..6, HighlightKind::Function),
            span(6..40, HighlightKind::String),
        ];
        let theme = crate::theme::Theme::for_mode(crate::theme::ThemeMode::Dark);
        let runs = build_line_runs(0, text, &spans, gpui::font("Zed Mono"), gpui::black(), &theme.syntax);
        let pieces = slice_like_shaper(text, &runs);
        assert_eq!(pieces.concat(), text);

        // A comment ending in a multi-byte char, with one byte typed in
        // front of it: the stale end lands inside the `é`.
        let text = "// xcaf\u{e9}";
        let spans = [span(0..8, HighlightKind::Comment)];
        let runs = build_line_runs(0, text, &spans, gpui::font("Zed Mono"), gpui::black(), &theme.syntax);
        assert_eq!(slice_like_shaper(text, &runs).concat(), text);
    }

    #[test]
    fn line_runs_cover_text_on_a_mid_document_line() {
        let text = "\u{4e2d}\u{6587} ok";
        let spans = [span(10..14, HighlightKind::Comment), span(15..17, HighlightKind::Number)];
        let theme = crate::theme::Theme::for_mode(crate::theme::ThemeMode::Dark);
        let runs = build_line_runs(10, text, &spans, gpui::font("Zed Mono"), gpui::black(), &theme.syntax);
        assert_eq!(slice_like_shaper(text, &runs).concat(), text);
    }

    #[test]
    fn eol_mark_reads_bytes_without_splitting_chars() {
        // LF followed by a multi-byte char: `end..end + 2` splits `中`.
        let rope = ropey::Rope::from_str("abc\n\u{4e2d}");
        assert_eq!(eol_mark_at(&rope, 3), Some(EolMark::Lf));
        // A wrapped row that ends between two CJK chars.
        let rope = ropey::Rope::from_str("\u{4e2d}\u{6587}");
        assert_eq!(eol_mark_at(&rope, 3), None);
        let rope = ropey::Rope::from_str("a\r\nb");
        assert_eq!(eol_mark_at(&rope, 1), Some(EolMark::Crlf));
        let rope = ropey::Rope::from_str("a\rb");
        assert_eq!(eol_mark_at(&rope, 1), Some(EolMark::Cr));
        assert_eq!(eol_mark_at(&rope, 3), None);
        assert_eq!(eol_mark_at(&rope, 99), None);
    }

    #[test]
    fn row_text_rejects_ranges_that_split_a_char() {
        let rope = ropey::Rope::from_str("a\u{4e2d}b");
        assert_eq!(row_text(&rope, 0, 4).as_deref(), Some("a\u{4e2d}"));
        assert_eq!(row_text(&rope, 0, 2), None);
        assert_eq!(row_text(&rope, 2, 1), None);
        assert_eq!(row_text(&rope, 3, 9), None);
        assert_eq!(row_text(&rope, usize::MAX, 2), None);
    }

    #[test]
    fn stale_wrap_rows_slice_on_char_boundaries() {
        let row = |start: u32, end: u32| WrapRow {
            logical_line: 0,
            byte_start: start,
            byte_end: end,
        };
        // Row cached for "ab" (0..2); the line is now "a\u{e9}".
        assert_eq!(wrap_row_text("a\u{e9}", 0, &row(0, 2)), (0, "a"));
        // Row starts inside the `é`.
        assert_eq!(wrap_row_text("a\u{e9}z", 0, &row(2, 4)), (1, "\u{e9}z"));
        // Row past the end of a line that got shorter.
        assert_eq!(wrap_row_text("ab", 10, &row(20, 25)), (2, ""));
        // Row from before the line start (lines above shrank).
        assert_eq!(wrap_row_text("abc", 10, &row(5, 12)), (0, "ab"));
        // Valid row is untouched.
        assert_eq!(wrap_row_text("\u{4e2d}\u{6587}x", 7, &row(10, 14)), (3, "\u{6587}x"));
    }
}

#[cfg(test)]
mod caret_geometry_tests {
    use super::*;
    use gpui::{AppContext, Element, TestAppContext, WindowAppearance};

    struct Probe {
        editor: gpui::AnyElement,
        workspace: Entity<Workspace>,
    }

    impl IntoElement for Probe {
        type Element = Self;
        fn into_element(self) -> Self {
            self
        }
    }

    impl Element for Probe {
        type RequestLayoutState = ();
        type PrepaintState = ();
        fn id(&self) -> Option<ElementId> {
            None
        }
        fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
            None
        }
        fn request_layout(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&gpui::InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, ()) {
            (self.editor.request_layout(window, cx), ())
        }
        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&gpui::InspectorElementId>,
            bounds: Bounds<Pixels>,
            _: &mut (),
            window: &mut Window,
            cx: &mut App,
        ) {
            self.editor.prepaint(window, cx);
            let w = self.workspace.read(cx);
            assert!(
                !w.last_lines.is_empty(),
                "caret geometry must exist before deferred popups prepaint"
            );
            assert_eq!(w.last_lines[0].line_byte_len, "alpha".len());
            assert_eq!(w.viewport.bounds, bounds);
        }
        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&gpui::InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut (),
            _: &mut (),
            window: &mut Window,
            cx: &mut App,
        ) {
            self.editor.paint(window, cx);
        }
    }

    #[test]
    fn edited_caret_geometry_is_ready_before_popup_prepaint() {
        let mut cx = TestAppContext::single();
        let workspace = cx.new(|cx| {
            let mut w = Workspace::new(
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                1.,
                WindowAppearance::Dark,
            );
            w.active_tab_mut().buffer.rope = ropey::Rope::from_str("alpha");
            w.active_tab_mut().selection = crate::buffer::Selection {
                anchor: 5,
                head: 5,
                ..Default::default()
            };
            w.last_lines.clear();
            w.window_closed = true;
            w
        });
        let visual = cx.add_empty_window();
        visual.draw(
            point(px(80.), px(110.)),
            size(px(400.), px(150.)),
            |_, _| Probe {
                editor: EditorElement {
                    workspace: workspace.clone(),
                }
                .into_any_element(),
                workspace: workspace.clone(),
            },
        );
        cx.quit();
    }
}
