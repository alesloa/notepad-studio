//! Rendered Doc View pane for rich-document buffers (Phase 1: DOCX read-only).
//!
//! Walks Buffer.rich_doc.ast.blocks and renders each as a vertical stack of
//! paragraph / heading rows. Inline runs (bold/italic/underline) handled in
//! `render_runs`. Phase 2 adds click handlers that set the active-block on
//! the payload so the toolbar formatting buttons know what to target. Lists,
//! tables, images, color, sub-range selection, and click-to-position-caret
//! are deferred to later phases.

use std::path::Path;

use gpui::{
    div, fill, point, prelude::*, px, relative, rgb, rgba, size, AnyElement, App, Bounds,
    CursorStyle, DispatchPhase, Edges, Element, ElementId, ElementInputHandler, Entity,
    FocusHandle, FontWeight, GlobalElementId, Hsla, IntoElement, LayoutId, Length, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Position, Rgba, ScrollHandle,
    ShapedLine, SharedString, Size, StrikethroughStyle, Style, StyledText, TextRun,
    UnderlineStyle, WeakEntity, Window,
};

use crate::consts::CARET_WIDTH_PX;
use crate::rich_doc::ast::{
    Alignment, Block, BlockProps, ImageSource, InlineRun, InlineStyle, ListMarker, TableCell,
    TableRow,
};
use crate::tab::Tab;
use crate::theme::Theme;
use crate::workspace::Workspace;

fn body_font_family() -> SharedString {
    crate::fonts::mono()
}

/// Returns `true` when `path` is a rich-document extension Phase 1 understands.
/// Mirrors `is_markdown_path` in shape so callers can use one detection idiom.
pub(crate) fn is_rich_doc_path(path: Option<&Path>) -> bool {
    path.and_then(crate::rich_doc::detect_format).is_some()
}

/// `match_highlights`: per-block flat-byte ranges `(lo, hi, is_current)`
/// to paint as find-result backgrounds. Empty when the find dialog is
/// closed or no matches landed inside this document.
pub(crate) fn rich_doc_pane(
    theme: &Theme,
    tab: &Tab,
    workspace: Entity<Workspace>,
    body_focus: FocusHandle,
    match_highlights: std::collections::HashMap<usize, Vec<(usize, usize, bool)>>,
    scroll_handle: ScrollHandle,
) -> AnyElement {
    let payload = match tab.buffer.rich_doc.as_ref() {
        Some(p) => p,
        None => {
            return div()
                .h_full()
                .bg(rgb(theme.bg))
                .child("Doc View error: buffer has no rich_doc payload")
                .into_any_element();
        }
    };
    let active_block = payload.active_block;
    let active_bg = theme.chrome;
    let accent = theme.link;

    // Pane root catches clicks that miss every block row (the gap_3 strip
    // between rows, the p_6 padding around the column, and the empty area
    // below the last block). EditableInline calls `cx.stop_propagation()`
    // on text clicks and each row's `on_mouse_down` below also stops the
    // bubble, so this only fires for true background clicks. Matches "click
    // anywhere in the canvas to deselect and park the caret at doc end."
    //
    // `.track_focus(&body_focus)` here is the load-bearing piece for Cmd+A
    // working after a blank-area click: GPUI's div auto-registers a mouse
    // handler for any element with a tracked focus handle that steals focus
    // on click. The Workspace root has `.track_focus(&workspace.focus_handle)`
    // so without our own track_focus here, the click would bubble up and the
    // workspace's auto-focus would steal focus away from body_focus, leaving
    // input_focus(window) == Editor — and Cmd+A would no-op on the rich doc.
    // `stop_propagation()` on the explicit on_mouse_down halts the bubble
    // before the workspace auto-focus runs.
    let ws_for_pane_click = workspace.clone();
    let col = div()
        .id("rich-doc-pane")
        .track_focus(&body_focus)
        // `flex_1` instead of `h_full` — `h_full` makes Taffy measure
        // `bounds.size.height` against intrinsic content height, which
        // truncates `max_offset` to ~0 so the wheel scroll silently does
        // nothing. Same trap markdown_view documents at
        // src/ui/markdown_view/mod.rs:1522-1529.
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .track_scroll(&scroll_handle)
        .bg(rgb(theme.bg))
        .pt_6()
        .pb_6()
        .pl_6()
        // Right padding reserves space for the absolute-overlay scrollbar
        // so the last words of long lines don't slide under the thumb.
        .pr(px(RICH_DOC_SCROLLBAR_THICKNESS + 24.))
        .flex()
        .flex_col()
        .gap_3()
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            // Focus + deselect-in-place — clicking blank pane background does
            // NOT move the caret (matches Word / NPP). But ALSO arm a pending
            // drag anchor at the end of the last textual block so the first
            // mouse-move with the button held promotes it into a real
            // selection drag (Phase 2g.E). Pure click → release never
            // promotes (mouse-up wipes the pending anchor) so the caret
            // still stays exactly where it was.
            cx.stop_propagation();
            ws_for_pane_click.update(cx, |w, cx| {
                let anchor = w
                    .tabs
                    .get(w.active)
                    .and_then(|t| t.buffer.rich_doc.as_ref())
                    .and_then(|p| {
                        (0..p.ast.blocks.len())
                            .rev()
                            .find(|&b| p.block_runs(b).is_some())
                            .map(|b| p.block_end(b))
                    });
                w.focus_rich_doc_body_with_anchor(anchor, window, cx);
            });
        });
    // Right-click anywhere over the rich-doc pane opens the shared workspace
    // context menu (Undo / Redo / Cut / Copy / Paste / Delete / Select All /
    // Toggle Bookmark / Go to Line / Find). Mirrors the plain editor pane's
    // wiring at src/ui/editor_pane.rs. Each entry routes through the
    // workspace action handlers, which already short-circuit to the
    // rich_doc_* path when input_focus(window) == RichDocBody.
    let ws_for_right_click = workspace.clone();
    let col = col.on_mouse_down(MouseButton::Right, move |event, _window, cx| {
        ws_for_right_click.update(cx, |w, cx| w.show_context_menu(event.position, cx));
    });
    let mut col = col;
    // Navigation / clipboard / editing chords route through Workspace's
    // action handlers (select_all / copy / cut / paste / move_* / etc.).
    // Each handler short-circuits to its `rich_doc_*` sibling when
    // `input_focus(window) == RichDocBody`. Typed characters and IME flow
    // through `EntityInputHandler::replace_text_in_range`. So nothing here
    // needs to listen for raw key events — the on_key_down listener used
    // by earlier phases would have fired alongside the matching action and
    // duplicated every keystroke.

    let list_markers = compute_list_markers(&payload.ast.blocks);
    let cursor_block = payload.selection.as_ref().map(|s| s.cursor.block);
    // First / last textual (Paragraph or Heading) block indices — used by the
    // per-block drag-move handler so the topmost block also claims "cursor y
    // above me" and the bottommost block claims "cursor y below me." This is
    // what keeps drag-select extending when the user drags past the top or
    // bottom edge of the app window (mouse capture still delivers events
    // with the cursor outside the pane, but no block's bounds contain that y
    // without these extra claims). The first textual block also owns the
    // drag auto-scroll: when the user drags past the viewport edge, that
    // block's move handler nudges `scroll_handle`'s offset so content
    // scrolls toward the cursor (Phase 2g.G).
    let first_textual_idx: Option<usize> = (0..payload.ast.blocks.len())
        .find(|&i| matches!(payload.ast.blocks[i], Block::Paragraph { .. } | Block::Heading { .. }));
    let last_textual_idx: Option<usize> = (0..payload.ast.blocks.len())
        .rev()
        .find(|&i| matches!(payload.ast.blocks[i], Block::Paragraph { .. } | Block::Heading { .. }));
    for (idx, block) in payload.ast.blocks.iter().enumerate() {
        let is_active = active_block == Some(idx);
        let is_cursor_block = cursor_block == Some(idx);
        let marker = list_markers[idx].clone();
        let is_textual = matches!(block, Block::Paragraph { .. } | Block::Heading { .. });
        let is_first_textual = first_textual_idx == Some(idx);
        let is_last_textual = last_textual_idx == Some(idx);

        let highlights = match_highlights.get(&idx).cloned().unwrap_or_default();
        let inner: AnyElement = match block {
            Block::Paragraph { .. } | Block::Heading { .. } => render_editable_block(
                block,
                marker.as_deref(),
                idx,
                payload.selection,
                is_cursor_block,
                is_first_textual,
                is_last_textual,
                workspace.downgrade(),
                body_focus.clone(),
                theme,
                highlights,
                scroll_handle.clone(),
            ),
            Block::Table { .. } | Block::Image { .. } => render_block(block, marker.as_deref(), theme),
        };

        let mut row = div()
            .id(("rich-doc-block", idx))
            .px_2()
            .py_1()
            .border_l_2()
            .border_color(rgb(if is_active { accent } else { theme.bg }))
            .when(is_active, |this| this.bg(rgb(active_bg)))
            .cursor(CursorStyle::IBeam)
            .child(inner);
        // Track focus only on the cursor block so handle_input from the
        // EditableInline below maps to a dispatch node that actually receives
        // IME / typing. GPUI focus-handle caveat — `.id() + .track_focus()` on
        // ONE wrapper is mandatory; multiple tracks stomp earlier ones.
        if is_cursor_block {
            row = row.track_focus(&body_focus);
        }
        // Row-level click catches clicks in the row's padding (px_2 / py_1)
        // that miss EditableInline's text bounds. Focus + deselect-in-place
        // (no caret move) AND arm a pending drag anchor at the END of this
        // block, so a drag started from the row padding starts selecting
        // from this paragraph's end and extends across paragraphs as the
        // cursor moves. Pure click → release leaves the caret untouched.
        // stop_propagation prevents the pane root's and workspace's auto-
        // focus handlers from firing.
        if is_textual {
            let ws_for_row_click = workspace.clone();
            let block_idx = idx;
            row = row.on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                cx.stop_propagation();
                ws_for_row_click.update(cx, |w, cx| {
                    let anchor = w
                        .tabs
                        .get(w.active)
                        .and_then(|t| t.buffer.rich_doc.as_ref())
                        .map(|p| p.block_end(block_idx));
                    w.focus_rich_doc_body_with_anchor(anchor, window, cx);
                });
            });
        }
        col = col.child(row);
    }

    // Headers/footers render out-of-flow at the bottom of Doc View, separated
    // by a divider. Read-only — they're container parts (DOCX header*.xml /
    // ODT master-page entries) that toolbar B/I/U never targets.
    if !payload.headers.is_empty() {
        col = col.child(render_extra_section("Header", &payload.headers, theme));
    }
    if !payload.footers.is_empty() {
        col = col.child(render_extra_section("Footer", &payload.footers, theme));
    }

    // Outer wrapper: relative positioning so the scrollbar overlay can be
    // pinned to the right edge with `position: absolute`. `overflow_hidden`
    // keeps the scrollbar from being clipped by an over-eager ancestor.
    // `flex_col` so `flex_1` on the scroll-div child resolves vertically.
    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .relative()
        .overflow_hidden()
        .child(col)
        .child(RichDocScrollbar {
            workspace: workspace.clone(),
            handle: scroll_handle,
            color: rgba(theme.scrollbar).into(),
            color_hover: rgba(theme.scrollbar_hover).into(),
        })
        .into_any_element()
}

fn render_extra_section(label: &str, sections: &[Vec<Block>], theme: &Theme) -> AnyElement {
    let muted = theme.disabled_text;
    let mut wrap = div()
        .mt_4()
        .pt_3()
        .border_t_1()
        .border_color(rgb(theme.border))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(muted))
                .child(label.to_string()),
        );
    for (i, section) in sections.iter().enumerate() {
        let mut bucket = div()
            .px_2()
            .py_1()
            .text_color(rgb(muted))
            .text_size(px(12.))
            .flex()
            .flex_col()
            .gap_1();
        if sections.len() > 1 {
            bucket = bucket.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(muted))
                    .child(format!("[section {}]", i + 1)),
            );
        }
        let section_markers = compute_list_markers(section);
        for (b_idx, b) in section.iter().enumerate() {
            bucket = bucket.child(render_block(b, section_markers[b_idx].as_deref(), theme));
        }
        wrap = wrap.child(bucket);
    }
    wrap.into_any_element()
}

fn render_block(block: &Block, list_marker_text: Option<&str>, theme: &Theme) -> AnyElement {
    match block {
        Block::Paragraph { runs, props } => render_paragraph(runs, props, list_marker_text, theme),
        Block::Heading { level, runs, props } => {
            render_heading(*level, runs, props, list_marker_text, theme)
        }
        Block::Table { rows } => render_table(rows, theme),
        Block::Image { source, width_px, height_px, alt } => {
            render_image(source, *width_px, *height_px, alt.as_deref(), theme)
        }
    }
}

/// Walk `blocks` once and produce the visible list-marker prefix per block
/// (`"• "` for bullets, `"1. "`, `"2. "`, … for numbered, `None` for
/// non-list blocks). Numbered lists reset their counter whenever the
/// run breaks — a non-list block, a kind change (bullet ↔ number), or a
/// level change. Tables and images count as breaks even when they sit
/// between two list paragraphs of the same kind because numbering across
/// an interrupting block would surprise readers.
fn compute_list_markers(blocks: &[Block]) -> Vec<Option<String>> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut counter: u32 = 0;
    let mut prev: Option<ListMarker> = None;
    for b in blocks {
        let marker = block_list_marker(b);
        match marker {
            None => {
                out.push(None);
                counter = 0;
                prev = None;
            }
            Some(m) if !m.ordered => {
                out.push(Some(bullet_glyph(m.level).to_string() + " "));
                counter = 0;
                prev = Some(m);
            }
            Some(m) => {
                let same_run = matches!(prev, Some(p) if p.ordered && p.level == m.level);
                if same_run {
                    counter = counter.saturating_add(1);
                } else {
                    counter = 1;
                }
                out.push(Some(format!("{}. ", counter)));
                prev = Some(m);
            }
        }
    }
    out
}

fn block_list_marker(block: &Block) -> Option<ListMarker> {
    match block {
        Block::Paragraph { props, .. } | Block::Heading { props, .. } => props.list,
        Block::Table { .. } | Block::Image { .. } => None,
    }
}

/// Visual glyph for a bullet at nesting `level`. Mirrors Word / LibreOffice
/// default styles: filled disc, hollow circle, filled square, then cycles.
fn bullet_glyph(level: u8) -> &'static str {
    match level % 3 {
        0 => "•",
        1 => "◦",
        _ => "▪",
    }
}

/// Apply alignment + indent to a paragraph wrapper. Indent is 18px per level
/// (~half-inch at 36 DPI) — matches Word's default Increase Indent step
/// visually inside the Doc View's body font. List paragraphs get an extra
/// 24px per nesting level on top of any explicit indent so the bullet/number
/// glyph has room to breathe.
fn apply_block_props<E: gpui::Styled>(wrapper: E, props: &BlockProps) -> E {
    let extra_list_px = props.list.map(|m| 24.0 + (m.level as f32) * 24.0).unwrap_or(0.0);
    let total_left_px = (props.indent_level as f32) * 18.0 + extra_list_px;
    let wrapper = if total_left_px > 0.0 {
        wrapper.pl(px(total_left_px))
    } else {
        wrapper
    };
    match props.alignment {
        Alignment::Left => wrapper.text_align(gpui::TextAlign::Left),
        Alignment::Center => wrapper.text_align(gpui::TextAlign::Center),
        Alignment::Right => wrapper.text_align(gpui::TextAlign::Right),
        // GPUI's TextAlign doesn't have a dedicated Justify variant; the
        // styled-text path doesn't support real text-justification anyway
        // (it would need per-line glyph spacing), so we fall back to Left
        // for the render. Justify still serializes correctly on save —
        // editors that load the file will lay it out.
        Alignment::Justify => wrapper.text_align(gpui::TextAlign::Left),
    }
}

fn render_table(rows: &[TableRow], theme: &Theme) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .w_full()
        .border_1()
        .border_color(rgb(theme.border));
    for row in rows {
        col = col.child(render_table_row(row, theme));
    }
    col.into_any_element()
}

fn render_table_row(row: &TableRow, theme: &Theme) -> AnyElement {
    let mut r = div()
        .flex()
        .flex_row()
        .w_full()
        .border_b_1()
        .border_color(rgb(theme.border));
    for cell in &row.cells {
        r = r.child(render_table_cell(cell, theme));
    }
    r.into_any_element()
}

fn render_table_cell(cell: &TableCell, theme: &Theme) -> AnyElement {
    let mut c = div()
        .flex_1()
        .min_w(px(40.))
        .p_1()
        .border_r_1()
        .border_color(rgb(theme.border));
    for b in &cell.blocks {
        c = c.child(render_block(b, None, theme));
    }
    c.into_any_element()
}

fn render_image(
    source: &ImageSource,
    width_px: Option<u32>,
    height_px: Option<u32>,
    alt: Option<&str>,
    theme: &Theme,
) -> AnyElement {
    let ImageSource::Inline { bytes, mime } = source;
    let Some(format) = gpui::ImageFormat::from_mime_type(mime) else {
        return render_image_placeholder(bytes.len(), mime, alt, theme);
    };
    let image = std::sync::Arc::new(gpui::Image::from_bytes(format, (**bytes).clone()));
    let mut elem = gpui::img(image);
    if let Some(w) = width_px {
        elem = elem.w(px(w as f32));
    }
    if let Some(h) = height_px {
        elem = elem.h(px(h as f32));
    }
    div()
        .py_1()
        .child(elem)
        .into_any_element()
}

fn render_image_placeholder(
    bytes_len: usize,
    mime: &str,
    alt: Option<&str>,
    theme: &Theme,
) -> AnyElement {
    let label = match alt {
        Some(a) if !a.is_empty() => format!("[image: {a} — {bytes_len} bytes, {mime}]"),
        _ => format!("[image: {bytes_len} bytes, {mime}]"),
    };
    div()
        .py_1()
        .px_2()
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .text_size(px(12.))
        .child(label)
        .into_any_element()
}

fn render_paragraph(
    runs: &[InlineRun],
    props: &BlockProps,
    list_marker_text: Option<&str>,
    theme: &Theme,
) -> AnyElement {
    let wrap = div()
        .text_color(rgb(theme.text))
        .text_size(px(15.))
        .child(render_runs_with_prefix(
            list_marker_text,
            runs,
            theme,
            15.,
            FontWeight::NORMAL,
        ));
    apply_block_props(wrap, props).into_any_element()
}

fn render_heading(
    level: u8,
    runs: &[InlineRun],
    props: &BlockProps,
    list_marker_text: Option<&str>,
    theme: &Theme,
) -> AnyElement {
    let size_px: f32 = match level {
        1 => 28.,
        2 => 24.,
        3 => 20.,
        4 => 18.,
        5 => 16.,
        _ => 15.,
    };
    let wrap = div()
        .text_color(rgb(theme.text))
        .text_size(px(size_px))
        .font_weight(FontWeight::BOLD)
        .pt_4()
        .pb_1()
        .child(render_runs_with_prefix(
            list_marker_text,
            runs,
            theme,
            size_px,
            FontWeight::BOLD,
        ));
    apply_block_props(wrap, props).into_any_element()
}

/// Render the inline runs of a paragraph/heading, prefixing the list-marker
/// glyph (`"• "` / `"1. "`) when present. The marker piggybacks on the same
/// `StyledText` as the body so it inherits the paragraph's font size and
/// base weight — separate text would line up unevenly on the baseline.
fn render_runs_with_prefix(
    prefix: Option<&str>,
    runs: &[InlineRun],
    theme: &Theme,
    base_size: f32,
    base_weight: FontWeight,
) -> AnyElement {
    if prefix.is_none() {
        return render_runs(runs, theme, base_size, base_weight);
    }
    let mut combined = String::new();
    let mut text_runs: Vec<TextRun> = Vec::with_capacity(runs.len() + 1);
    if let Some(p) = prefix {
        combined.push_str(p);
        text_runs.push(text_run_for(
            &InlineStyle::default(),
            p.len(),
            theme,
            base_size,
            base_weight,
        ));
    }
    for r in runs {
        let len = r.text.len();
        if len == 0 {
            continue;
        }
        combined.push_str(&r.text);
        text_runs.push(text_run_for(&r.style, len, theme, base_size, base_weight));
    }
    if combined.is_empty() {
        return "".into_any_element();
    }
    StyledText::new(SharedString::from(combined))
        .with_runs(text_runs)
        .into_any_element()
}

fn render_runs(
    runs: &[InlineRun],
    theme: &Theme,
    base_size: f32,
    base_weight: FontWeight,
) -> AnyElement {
    if runs.is_empty() {
        return "".into_any_element();
    }
    let mut combined = String::new();
    let mut text_runs: Vec<TextRun> = Vec::with_capacity(runs.len());
    for r in runs {
        let len = r.text.len();
        if len == 0 {
            continue;
        }
        combined.push_str(&r.text);
        text_runs.push(text_run_for(&r.style, len, theme, base_size, base_weight));
    }
    if combined.is_empty() {
        return "".into_any_element();
    }
    StyledText::new(SharedString::from(combined))
        .with_runs(text_runs)
        .into_any_element()
}

fn text_run_for(
    style: &InlineStyle,
    len: usize,
    theme: &Theme,
    _base_size: f32,
    base_weight: FontWeight,
) -> TextRun {
    let weight = if style.bold {
        FontWeight::BOLD
    } else {
        base_weight
    };
    // Effective color: link runs paint in theme.link (and force an underline
    // so they look like links). Explicit color overrides — even on a link run
    // — because the user picked it deliberately. No color + no link falls
    // back to theme.text.
    let is_link = style.link.is_some();
    let effective_color: u32 = match (style.color, is_link) {
        (Some(c), _) => c,
        (None, true) => theme.link,
        (None, false) => theme.text,
    };
    let want_underline = style.underline || is_link;
    let underline = if want_underline {
        Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(rgb(effective_color).into()),
            wavy: false,
        })
    } else {
        None
    };
    let strikethrough = if style.strike {
        Some(StrikethroughStyle {
            thickness: px(1.5),
            color: Some(rgb(effective_color).into()),
        })
    } else {
        None
    };
    TextRun {
        len,
        font: gpui::Font {
            family: body_font_family(),
            features: Default::default(),
            weight,
            style: if style.italic {
                gpui::FontStyle::Italic
            } else {
                gpui::FontStyle::Normal
            },
            fallbacks: None,
        },
        color: rgb(effective_color).into(),
        background_color: None,
        underline,
        strikethrough,
    }
}

/// Render a paragraph / heading as an editable custom Element. Phase 2g.B —
/// EVERY top-level paragraph/heading uses this so selection paints across
/// block boundaries. Caret + IME registration + focus tracking gates on
/// `is_cursor_block` so only the cursor's home block receives typing input.
/// Falls back to read-only `render_block` for tables and images.
fn render_editable_block(
    block: &Block,
    list_marker_text: Option<&str>,
    block_idx: usize,
    selection: Option<crate::rich_doc::TextSelection>,
    is_cursor_block: bool,
    is_first_textual: bool,
    is_last_textual: bool,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    theme: &Theme,
    match_highlights: Vec<(usize, usize, bool)>,
    scroll_handle: ScrollHandle,
) -> AnyElement {
    let (runs, props, base_size_px, base_weight): (&[InlineRun], &BlockProps, f32, FontWeight) =
        match block {
            Block::Paragraph { runs, props } => (runs, props, 15.0, FontWeight::NORMAL),
            Block::Heading { level, runs, props } => {
                let s = match level {
                    1 => 28.0,
                    2 => 24.0,
                    3 => 20.0,
                    4 => 18.0,
                    5 => 16.0,
                    _ => 15.0,
                };
                (runs, props, s, FontWeight::BOLD)
            }
            Block::Table { .. } | Block::Image { .. } => {
                return render_block(block, list_marker_text, theme);
            }
        };
    let prefix_len = list_marker_text.map(|p| p.len()).unwrap_or(0);
    let elem = EditableInline {
        runs: runs.to_vec(),
        prefix: list_marker_text.map(|s| s.to_string()),
        prefix_len,
        block_idx,
        selection,
        is_cursor_block,
        is_first_textual,
        is_last_textual,
        workspace,
        focus,
        theme: theme.clone(),
        base_size_px,
        base_weight,
        match_highlights,
        scroll_handle,
    };
    let wrap = div()
        .text_color(rgb(theme.text))
        .text_size(px(base_size_px))
        .when(base_weight == FontWeight::BOLD, |w| {
            w.font_weight(FontWeight::BOLD).pt_4().pb_1()
        })
        .child(elem);
    apply_block_props(wrap, props).into_any_element()
}

/// Editable paragraph / heading element. Owns the run snapshot + selection
/// snapshot for the active block. Paints selection / glyphs / caret + wires
/// IME input + mouse handlers (click → caret, drag → selection extension,
/// double-click → word).
struct EditableInline {
    runs: Vec<InlineRun>,
    prefix: Option<String>,
    /// Byte length of `prefix` if present (already utf-8). Used to offset
    /// click x → byte mapping past the marker glyph.
    prefix_len: usize,
    block_idx: usize,
    selection: Option<crate::rich_doc::TextSelection>,
    /// True iff this block contains the selection cursor. Drives caret
    /// painting + IME handler registration + drag tracking. Non-cursor
    /// blocks still paint selection (if the selection spans into them)
    /// and accept mouse clicks (which move the cursor to them).
    is_cursor_block: bool,
    /// True iff this is the topmost paragraph/heading in the doc. The
    /// drag-move handler uses this to claim "cursor y above me" so dragging
    /// up past the top of the app window still extends to the doc start.
    is_first_textual: bool,
    /// True iff this is the bottommost paragraph/heading in the doc.
    /// Symmetric to `is_first_textual` — claims "cursor y below me" so a
    /// drag that escapes the bottom of the app window still extends to doc
    /// end.
    is_last_textual: bool,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    theme: Theme,
    base_size_px: f32,
    base_weight: FontWeight,
    /// Per-match (lo, hi, is_current) tuples inside this block's flat run
    /// text. Painted as a yellow background under the matched glyphs in
    /// `paint`. Empty when the find dialog is closed or no matches landed
    /// in this block.
    match_highlights: Vec<(usize, usize, bool)>,
    /// Pane scroll handle, cloned from `rich_doc_pane`. The first textual
    /// block's drag-move handler uses this to nudge the viewport when the
    /// cursor strays past the visible edge during a drag-select.
    scroll_handle: ScrollHandle,
}

impl IntoElement for EditableInline {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for EditableInline {
    type RequestLayoutState = ();
    type PrepaintState = Option<ShapedLine>;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        // Reserve at least the line height for the row so an empty paragraph
        // still has a clickable / caret-paintable rect.
        let line_h = window.line_height();
        style.min_size.height = line_h.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let (combined, text_runs) = build_combined(
            self.prefix.as_deref(),
            &self.runs,
            &self.theme,
            self.base_size_px,
            self.base_weight,
        );
        if combined.is_empty() {
            return None;
        }
        let font_size = px(self.base_size_px);
        Some(
            window
                .text_system()
                .shape_line(SharedString::from(combined), font_size, &text_runs, None),
        )
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = window.line_height();
        let baseline_y = bounds.top();
        let prefix_len = self.prefix_len;
        let body_total_bytes: usize = self.runs.iter().map(|r| r.text.len()).sum();

        // Selection rect — paint the slice of the selection that falls inside
        // this block. Cross-block selections paint a slice in every block
        // they touch: the start block runs from lo→end-of-block, middle
        // blocks fill end-to-end, the cursor block runs 0→cursor.
        if let (Some(shaped), Some(sel)) = (prepaint.as_ref(), self.selection.as_ref()) {
            if !sel.is_collapsed() {
                let (lo, hi) = sel.range_full();
                if self.block_idx >= lo.block && self.block_idx <= hi.block {
                    let body_lo = if self.block_idx == lo.block {
                        flat_byte_for_run_byte(&self.runs, lo.run, lo.byte)
                    } else {
                        0
                    };
                    let body_hi = if self.block_idx == hi.block {
                        flat_byte_for_run_byte(&self.runs, hi.run, hi.byte)
                    } else {
                        body_total_bytes
                    };
                    let lo_disp = (body_lo + prefix_len).min(shaped.len);
                    // When this block sits between lo and hi but is empty,
                    // still paint a visible sliver so the user can see the
                    // empty paragraph is selected. Same when the entire
                    // block has no content but is between the ends.
                    let hi_disp = if self.block_idx != hi.block && body_hi == 0 {
                        // Empty intermediate block — draw a narrow indicator
                        // 6px wide past the prefix so the row reads as
                        // "selected" even with no glyphs.
                        (lo_disp + 6).min(shaped.len.max(lo_disp + 6))
                    } else {
                        (body_hi + prefix_len).min(shaped.len)
                    };
                    let sel_color: Hsla = rgba(self.theme.selection).into();
                    if hi_disp > lo_disp {
                        let x_lo = bounds.left() + shaped.x_for_index(lo_disp);
                        let x_hi = bounds.left() + shaped.x_for_index(hi_disp);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(x_lo, baseline_y),
                                size(x_hi - x_lo, line_height),
                            ),
                            sel_color,
                        ));
                    } else if self.block_idx != lo.block && self.block_idx != hi.block {
                        // Truly-empty intermediate block: shaped is None
                        // (handled by the outer if-let). Paint a fallback
                        // sliver at the left edge of bounds.
                        let x_lo = bounds.left();
                        window.paint_quad(fill(
                            Bounds::new(point(x_lo, baseline_y), size(px(8.), line_height)),
                            sel_color,
                        ));
                    }
                }
            }
        } else if let Some(sel) = self.selection.as_ref() {
            // Empty block between selection ends — shaped is None because
            // there's no text. Still draw a sliver so the row reads selected.
            if !sel.is_collapsed() {
                let (lo, hi) = sel.range_full();
                if self.block_idx > lo.block && self.block_idx < hi.block {
                    let sel_color: Hsla = rgba(self.theme.selection).into();
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.left(), baseline_y), size(px(8.), line_height)),
                        sel_color,
                    ));
                }
            }
        }

        // Find-match highlights — paint a yellow background under each match
        // range that lands in this block. The "current" match gets a deeper
        // amber so the user can tell which one Find Next/Prev will land on.
        // Painted after the selection rect but before glyphs so the text
        // stays legible on top.
        if !self.match_highlights.is_empty() {
            let shaped_for_hl = window.text_system().shape_line(
                SharedString::from(build_combined(
                    self.prefix.as_deref(),
                    &self.runs,
                    &self.theme,
                    self.base_size_px,
                    self.base_weight,
                ).0),
                px(self.base_size_px),
                &build_combined(
                    self.prefix.as_deref(),
                    &self.runs,
                    &self.theme,
                    self.base_size_px,
                    self.base_weight,
                ).1,
                None,
            );
            // Notepad++ palette: pale yellow for all matches, amber for the
            // currently-targeted one. RGBA hex.
            let match_color: Hsla = rgba(0xfff39960).into();
            let current_color: Hsla = rgba(0xff8c4290).into();
            for &(lo_body, hi_body, is_current) in &self.match_highlights {
                let lo_disp = (lo_body + prefix_len).min(shaped_for_hl.len);
                let hi_disp = (hi_body + prefix_len).min(shaped_for_hl.len);
                if hi_disp <= lo_disp {
                    continue;
                }
                let x_lo = bounds.left() + shaped_for_hl.x_for_index(lo_disp);
                let x_hi = bounds.left() + shaped_for_hl.x_for_index(hi_disp);
                let color = if is_current { current_color } else { match_color };
                window.paint_quad(fill(
                    Bounds::new(point(x_lo, baseline_y), size(x_hi - x_lo, line_height)),
                    color,
                ));
            }
        }

        // Glyphs.
        if let Some(shaped) = prepaint.take() {
            let _ = shaped.paint(
                point(bounds.left(), baseline_y),
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        // Caret — only when this is the cursor's home block AND focused AND
        // the workspace's blink timer says the caret should be visible right
        // now. Mirrors the plain editor's 500ms blink (driven by
        // Workspace::caret_visible flipped on a gpui::Timer task). Without
        // this read, the rich-doc caret was painted as a solid non-blinking
        // bar — a visible "not a real editor" tell.
        let caret_visible = self
            .workspace
            .upgrade()
            .map(|ws| ws.read(cx).caret_visible)
            .unwrap_or(true);
        if self.is_cursor_block && self.focus.is_focused(window) && caret_visible {
            if let Some(sel) = self.selection.as_ref() {
                let cursor_flat = flat_byte_for_run_byte(
                    &self.runs,
                    sel.cursor.run,
                    sel.cursor.byte,
                );
                let (combined, text_runs) = build_combined(
                    self.prefix.as_deref(),
                    &self.runs,
                    &self.theme,
                    self.base_size_px,
                    self.base_weight,
                );
                let caret_color: Hsla = Rgba::from(rgb(self.theme.text)).into();
                let cx_pos = if combined.is_empty() {
                    bounds.left()
                } else {
                    let shaped = window.text_system().shape_line(
                        SharedString::from(combined),
                        px(self.base_size_px),
                        &text_runs,
                        None,
                    );
                    let cx_idx = (cursor_flat + prefix_len).min(shaped.len);
                    bounds.left() + shaped.x_for_index(cx_idx)
                };
                window.paint_quad(fill(
                    Bounds::new(
                        point(cx_pos, baseline_y),
                        size(px(CARET_WIDTH_PX), line_height),
                    ),
                    caret_color,
                ));
            }
        }

        // IME router — registered only on the cursor block so typing maps
        // to the right bounds and doesn't double-fire from sibling blocks.
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        if self.is_cursor_block {
            window.handle_input(
                &self.focus,
                ElementInputHandler::new(bounds, workspace.clone()),
                cx,
            );
        }

        // Mouse handlers — capture bounds + element-owned data.
        let block_idx = self.block_idx;
        let prefix_len_cap = prefix_len;
        let runs_for_hit = self.runs.clone();
        let prefix_for_hit = self.prefix.clone();
        let theme_for_hit = self.theme.clone();
        let base_size_for_hit = self.base_size_px;
        let base_weight_for_hit = self.base_weight;
        let ws_for_down = workspace.clone();
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            cx.stop_propagation();
            let click_count = event.click_count;
            let shift = event.modifiers.shift;
            let local_x = event.position.x - bounds.left();
            // Re-shape so we can hit-test at the click x. Cheap — one paragraph.
            let (combined, text_runs) = build_combined(
                prefix_for_hit.as_deref(),
                &runs_for_hit,
                &theme_for_hit,
                base_size_for_hit,
                base_weight_for_hit,
            );
            if combined.is_empty() {
                ws_for_down.update(cx, |w, cx| {
                    if click_count >= 4 {
                        w.rich_doc_select_all_document(window, cx);
                    } else if shift {
                        w.extend_rich_doc_selection_to(block_idx, 0, window, cx);
                    } else {
                        w.set_rich_doc_caret(block_idx, 0, window, cx);
                    }
                });
                return;
            }
            let shaped = window.text_system().shape_line(
                SharedString::from(combined),
                px(base_size_for_hit),
                &text_runs,
                None,
            );
            let local_x = if local_x < px(0.) { px(0.) } else { local_x };
            let display_idx = shaped.closest_index_for_x(local_x).min(shaped.len);
            // Strip the marker prefix from display offset to get the flat
            // body byte. Clicks landing inside the prefix collapse to 0.
            let flat_body = display_idx.saturating_sub(prefix_len_cap);
            ws_for_down.update(cx, |w, cx| {
                if click_count >= 4 {
                    w.rich_doc_select_all_document(window, cx);
                } else if click_count == 3 {
                    w.rich_doc_select_paragraph(block_idx, window, cx);
                } else if click_count == 2 {
                    w.select_rich_doc_word(block_idx, flat_body, window, cx);
                } else if shift {
                    w.extend_rich_doc_selection_to(block_idx, flat_body, window, cx);
                } else {
                    w.set_rich_doc_caret(block_idx, flat_body, window, cx);
                }
            });
        });

        let ws_for_move = workspace.clone();
        let runs_for_move = self.runs.clone();
        let prefix_for_move = self.prefix.clone();
        let theme_for_move = self.theme.clone();
        let base_size_for_move = self.base_size_px;
        let base_weight_for_move = self.base_weight;
        let is_first_textual_for_move = self.is_first_textual;
        let is_last_textual_for_move = self.is_last_textual;
        let scroll_for_move = self.scroll_handle.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || !event.dragging() {
                return;
            }
            // Phase 2g.G — drag auto-scroll. Only the first textual block runs
            // this branch (every block's move handler fires per event; gating
            // here keeps the per-event offset adjustment idempotent). When the
            // cursor strays past the viewport top/bottom during a drag, nudge
            // the pane's scroll offset toward the cursor. Distance into the
            // 24px edge zone determines the per-event delta (capped at 16px
            // per move event). GPUI's tracked scroll handle: offset.y is 0 at
            // the top and grows MORE NEGATIVE as the pane scrolls down, so
            // "scroll down" = subtract from offset.y, "scroll up" = add.
            if is_first_textual_for_move {
                let drag_active_for_scroll = {
                    let w = ws_for_move.read(cx);
                    w.tabs
                        .get(w.active)
                        .and_then(|t| t.buffer.rich_doc.as_ref())
                        .map(|p| p.drag_active)
                        .unwrap_or(false)
                };
                if drag_active_for_scroll {
                    let viewport = scroll_for_move.bounds();
                    let edge_zone = px(24.);
                    let max_step = px(16.);
                    let y = event.position.y;
                    let top_thresh = viewport.top() + edge_zone;
                    let bot_thresh = viewport.bottom() - edge_zone;
                    let mut dy = px(0.);
                    if y < top_thresh {
                        let depth = (top_thresh - y).max(px(0.));
                        let factor = (f32::from(depth.min(edge_zone)) / f32::from(edge_zone))
                            .clamp(0., 1.);
                        dy = max_step * factor;
                    } else if y > bot_thresh {
                        let depth = (y - bot_thresh).max(px(0.));
                        let factor = (f32::from(depth.min(edge_zone)) / f32::from(edge_zone))
                            .clamp(0., 1.);
                        dy = max_step * -factor;
                    }
                    if dy != px(0.) {
                        let max = scroll_for_move.max_offset();
                        let mut off = scroll_for_move.offset();
                        let new_y = (off.y + dy).min(px(0.)).max(-max.height);
                        if new_y != off.y {
                            off.y = new_y;
                            scroll_for_move.set_offset(off);
                            // Force a repaint so the new offset lands this
                            // frame; without this the viewport only updates
                            // on the NEXT mouse-move, which feels laggy.
                            ws_for_move.update(cx, |_, cx| cx.notify());
                        }
                    }
                    // Continuous-autoscroll fallback. Once the cursor passes
                    // the edge zone, kick a 30ms-tick timer that keeps
                    // re-firing edge auto-scroll against the captured
                    // cursor position. Without this, holding the cursor
                    // still past the edge stalls the scroll (no more
                    // MouseMoveEvents fire). Idempotent — no-op while the
                    // task is already running or the cursor is inside the
                    // viewport; the call always updates the captured
                    // last-drag-pos so the next tick sees the latest cursor.
                    let pos_for_timer = event.position;
                    ws_for_move.update(cx, |w, cx| {
                        w.rich_doc_start_continuous_autoscroll(pos_for_timer, cx);
                    });
                }
            }
            // Phase 2g.E — if a blank-area mouse-down armed a pending drag
            // anchor (clicking in the pane padding / row padding), the very
            // first move event with button held promotes it into a real
            // drag here. Idempotent — multiple blocks can call this on the
            // same event; only the first one actually flips the state.
            let pending_exists = {
                let w = ws_for_move.read(cx);
                w.tabs
                    .get(w.active)
                    .and_then(|t| t.buffer.rich_doc.as_ref())
                    .map(|p| p.pending_drag_anchor.is_some())
                    .unwrap_or(false)
            };
            if pending_exists {
                ws_for_move.update(cx, |w, cx| {
                    w.commit_pending_rich_doc_drag(cx);
                });
            }
            // Phase 2g.D — cross-block drag. Every textual block listens for
            // every drag-move event. The block whose vertical bounds contain
            // the cursor's y handles it, extending the selection to whatever
            // glyph the cursor's x lands on inside this block. When the
            // cursor is in the gap between two blocks (the gap_3 strip) or
            // outside the rich-doc pane, no block fires and the selection
            // simply doesn't extend that frame — next move event will land
            // on a block. `extend_rich_doc_caret` also updates active_block
            // so the toolbar / arrow keys follow the cursor across paras.
            let drag_active = {
                let w = ws_for_move.read(cx);
                w.tabs
                    .get(w.active)
                    .and_then(|t| t.buffer.rich_doc.as_ref())
                    .map(|p| p.drag_active)
                    .unwrap_or(false)
            };
            if !drag_active {
                return;
            }
            // Phase 2g.F — cursor outside the app window during drag. macOS /
            // gpui still deliver mouse-move events while the button is held,
            // so we keep firing — but the cursor's y may now be above the
            // first block's top or below the last block's bottom. The first
            // textual block also claims "y above me" (snap cursor to flat
            // byte 0 of this block); the last textual block claims "y below
            // me" (snap to end-of-block byte count). Otherwise the existing
            // y-in-bounds gate decides.
            let y = event.position.y;
            let flat_total: usize = runs_for_move.iter().map(|r| r.text.len()).sum();
            let (use_block, override_flat): (bool, Option<usize>) =
                if y < bounds.top() && is_first_textual_for_move {
                    (true, Some(0))
                } else if y > bounds.bottom() && is_last_textual_for_move {
                    (true, Some(flat_total))
                } else if y >= bounds.top() && y <= bounds.bottom() {
                    (true, None)
                } else {
                    (false, None)
                };
            if !use_block {
                return;
            }
            if let Some(flat_body) = override_flat {
                ws_for_move.update(cx, |w, cx| {
                    w.extend_rich_doc_caret(block_idx, flat_body, cx);
                });
                return;
            }
            let local_x = (event.position.x - bounds.left()).max(px(0.));
            let (combined, text_runs) = build_combined(
                prefix_for_move.as_deref(),
                &runs_for_move,
                &theme_for_move,
                base_size_for_move,
                base_weight_for_move,
            );
            if combined.is_empty() {
                return;
            }
            let shaped = window.text_system().shape_line(
                SharedString::from(combined),
                px(base_size_for_move),
                &text_runs,
                None,
            );
            let display_idx = shaped.closest_index_for_x(local_x).min(shaped.len);
            let flat_body = display_idx.saturating_sub(prefix_len_cap);
            ws_for_move.update(cx, |w, cx| {
                w.extend_rich_doc_caret(block_idx, flat_body, cx);
            });
        });

        let ws_for_up = workspace.clone();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            ws_for_up.update(cx, |w, cx| {
                w.end_rich_doc_drag(cx);
                // Drop the continuous-autoscroll timer too — even if
                // end_rich_doc_drag already ran via another path's mouse-up,
                // the timer task must not outlive the gesture.
                w.rich_doc_stop_continuous_autoscroll(cx);
            });
        });
    }
}

/// Build the combined display string (prefix + run texts) and the matching
/// TextRun array. Returns ("", vec![]) when nothing to render. Shared by
/// the EditableInline element's prepaint, paint (caret re-shape), and
/// mouse-down (hit-test) so the byte indices line up across all three.
fn build_combined(
    prefix: Option<&str>,
    runs: &[InlineRun],
    theme: &Theme,
    base_size: f32,
    base_weight: FontWeight,
) -> (String, Vec<TextRun>) {
    let mut combined = String::new();
    let mut text_runs: Vec<TextRun> = Vec::with_capacity(runs.len() + 1);
    if let Some(p) = prefix {
        if !p.is_empty() {
            combined.push_str(p);
            text_runs.push(text_run_for(
                &InlineStyle::default(),
                p.len(),
                theme,
                base_size,
                base_weight,
            ));
        }
    }
    for r in runs {
        let len = r.text.len();
        if len == 0 {
            continue;
        }
        combined.push_str(&r.text);
        text_runs.push(text_run_for(&r.style, len, theme, base_size, base_weight));
    }
    (combined, text_runs)
}

/// Flat byte offset (sum of preceding runs + intra-run byte) inside the
/// concatenated run text — does NOT include the marker prefix. Callers add
/// prefix_len themselves when mapping into the shaped display string.
fn flat_byte_for_run_byte(runs: &[InlineRun], run: usize, byte: usize) -> usize {
    let mut acc = 0usize;
    for (i, r) in runs.iter().enumerate() {
        if i < run {
            acc += r.text.len();
        } else if i == run {
            return acc + byte.min(r.text.len());
        } else {
            break;
        }
    }
    acc
}

/// Doc View scrollbar — mirrors `MarkdownPreviewScrollbar` shape, paints a
/// draggable thumb on the right edge of the rich-doc pane. Drag offset
/// (Pixels from the top of the thumb where the user grabbed) persists on
/// `Workspace.rich_doc_scroll_drag` so it survives across paints.
pub(crate) const RICH_DOC_SCROLLBAR_THICKNESS: f32 = 10.;
const RICH_DOC_SCROLLBAR_MIN_THUMB: f32 = 30.;

pub(crate) struct RichDocScrollbar {
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) handle: ScrollHandle,
    pub(crate) color: Hsla,
    pub(crate) color_hover: Hsla,
}

pub(crate) struct RichDocScrollbarPrepaint {
    thumb: Option<Bounds<Pixels>>,
    track: Bounds<Pixels>,
    max_offset_y: Pixels,
    movable: Pixels,
}

impl IntoElement for RichDocScrollbar {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RichDocScrollbar {
    type RequestLayoutState = ();
    type PrepaintState = RichDocScrollbarPrepaint;

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
        // Absolute overlay pinned to right edge of the (relative) parent.
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.inset = Edges {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: Length::Auto,
        };
        style.size.width = px(RICH_DOC_SCROLLBAR_THICKNESS).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let max_off: Size<Pixels> = self.handle.max_offset();
        let max_y = max_off.height;
        if max_y <= px(0.) {
            // Content fits — full-height thumb that can't move (mouse
            // handlers short-circuit on `max_y <= 0`).
            let thumb = Bounds::new(
                point(bounds.left(), bounds.top()),
                size(bounds.size.width, bounds.size.height),
            );
            return RichDocScrollbarPrepaint {
                thumb: Some(thumb),
                track: bounds,
                max_offset_y: px(0.),
                movable: px(0.),
            };
        }
        let viewport_h = bounds.size.height;
        let viewport_h_f = f32::from(viewport_h).max(1.);
        let total_content_f = viewport_h_f + f32::from(max_y);
        let raw_thumb = viewport_h_f * (viewport_h_f / total_content_f);
        let thumb_h = px(raw_thumb.max(RICH_DOC_SCROLLBAR_MIN_THUMB).min(viewport_h_f));
        let movable = (viewport_h - thumb_h).max(px(0.));
        let offset_y = -self.handle.offset().y;
        let ratio = (f32::from(offset_y) / f32::from(max_y).max(1.)).clamp(0., 1.);
        let thumb_top = bounds.top() + movable * ratio;
        let thumb = Bounds::new(point(bounds.left(), thumb_top), size(bounds.size.width, thumb_h));
        RichDocScrollbarPrepaint {
            thumb: Some(thumb),
            track: bounds,
            max_offset_y: max_y,
            movable,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(thumb_bounds) = prepaint.thumb else {
            return;
        };

        let dragging = self.workspace.read(cx).rich_doc_scroll_drag.is_some();
        let color = if dragging { self.color_hover } else { self.color };
        let inset = px(2.);
        let visible_thumb = Bounds::new(
            point(thumb_bounds.left() + inset, thumb_bounds.top()),
            size(
                (thumb_bounds.size.width - inset * 2.).max(px(1.)),
                thumb_bounds.size.height,
            ),
        );
        window.paint_quad(fill(visible_thumb, color));

        let track_bounds = prepaint.track;
        let max_y = prepaint.max_offset_y;
        let movable = prepaint.movable;
        let handle_down = self.handle.clone();
        let handle_move = self.handle.clone();
        let workspace_down = self.workspace.clone();
        let workspace_move = self.workspace.clone();
        let workspace_up = self.workspace.clone();

        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !track_bounds.contains(&event.position) {
                return;
            }
            if thumb_bounds.contains(&event.position) {
                let offset = event.position.y - thumb_bounds.top();
                workspace_down.update(cx, |w, cx| {
                    w.rich_doc_scroll_drag = Some(offset);
                    cx.notify();
                });
            } else {
                // Page-step toward the click.
                let viewport_h = track_bounds.size.height;
                let direction = if event.position.y < thumb_bounds.top() { -1. } else { 1. };
                let cur_y = -handle_down.offset().y;
                let new_y = (cur_y + viewport_h * direction).clamp(px(0.), max_y);
                handle_down.set_offset(point(handle_down.offset().x, -new_y));
                workspace_down.update(cx, |_w, cx| cx.notify());
            }
            cx.stop_propagation();
        });

        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            workspace_move.update(cx, |w, cx| {
                let Some(grab) = w.rich_doc_scroll_drag else { return };
                if movable <= px(0.) || max_y <= px(0.) {
                    return;
                }
                let target_top_y = event.position.y - grab;
                let local = (target_top_y - track_bounds.top()).clamp(px(0.), movable);
                let ratio = (f32::from(local) / f32::from(movable).max(1.)).clamp(0., 1.);
                let new_y = max_y * ratio;
                handle_move.set_offset(point(handle_move.offset().x, -new_y));
                cx.notify();
            });
        });

        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            workspace_up.update(cx, |w, cx| {
                if w.rich_doc_scroll_drag.is_some() {
                    w.rich_doc_scroll_drag = None;
                    cx.notify();
                }
            });
        });
    }
}
