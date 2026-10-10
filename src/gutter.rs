// --------------------------------------------------------------------------------------------
// GutterElement — paints right-aligned line numbers for the visible window.
// Reads the active tab's `scroll_top` so it stays glued to the editor pane.

use gpui::{
    App, Bounds, Element, ElementId, Entity, GlobalElementId, Hsla, IntoElement, LayoutId, Pixels,
    Point, Rgba, ShapedLine, SharedString, Style, TextRun, Window, fill, point, relative, rgb,
    size,
};

use crate::workspace::Workspace;

pub(crate) struct GutterElement {
    pub(crate) workspace: Entity<Workspace>,
}

pub(crate) struct GutterPrepaint {
    labels: Vec<(ShapedLine, Point<Pixels>)>,
    /// Filled-circle glyphs marking bookmarked visible lines. Painted left
    /// of the line numbers in the theme's accent (`toggle_active`) color.
    bookmark_glyphs: Vec<Bounds<Pixels>>,
    bookmark_color: Hsla,
    /// Thin vertical bar on the gutter's left edge marking the pending
    /// Begin/End Select anchor line. At most one per gutter — `Option`
    /// because the marker may not be set or its line may be off-screen.
    begin_select_glyph: Option<Bounds<Pixels>>,
    begin_select_color: Hsla,
    /// Backdrop band painted across the full gutter width for the row
    /// containing the caret. Mirrors the editor pane's current-line
    /// highlight so the line number, bookmark glyph, and editor body all
    /// share the same "this is the active line" cue. `None` when the
    /// toggle is off or the caret's line is off-screen.
    current_line_quad: Option<Bounds<Pixels>>,
    current_line_color: Hsla,
}

impl IntoElement for GutterElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GutterElement {
    type RequestLayoutState = ();
    type PrepaintState = GutterPrepaint;

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
        let workspace = self.workspace.read(cx);
        let theme = workspace.theme();
        let style = window.text_style();
        // 12px at 16px rem; scales with the active rem_size so window-level
        // zoom flows through automatically.
        let font_size = window.rem_size() * (12. / 16.);
        let line_height = window.line_height();
        let scroll_top = workspace.scroll_top();
        let buffer = workspace.buffer();
        let total_lines = buffer.rope.len_lines().max(1);
        let gutter_color: Hsla = Rgba::from(rgb(theme.gutter_text)).into();
        let bookmark_color: Hsla = Rgba::from(rgb(theme.toggle_active)).into();
        // Begin/End Select marker uses the bracket-match accent — distinct
        // from `toggle_active` so the two glyphs never read as the same
        // signal, and already theme-coupled so it tracks light/dark mode.
        let begin_select_color: Hsla = Rgba::from(rgb(theme.bracket_match)).into();
        let prefs = crate::settings::snapshot();
        let show_line_numbers = prefs.show_line_numbers;
        let show_bookmarks = prefs.show_bookmark_glyph;
        let highlight_current_line = prefs.highlight_current_line;
        let current_line_color: Hsla = Rgba::from(rgb(theme.current_line_bg)).into();
        let begin_select_line = workspace.begin_select_marker_line();

        // Resolve the caret's logical line up-front. Block-mode selection
        // tracks `block.head_line`; otherwise use the head byte's line.
        let caret_line: Option<usize> = if highlight_current_line {
            if let Some(block) = workspace
                .block_selection()
                .filter(|b| !b.is_empty() && !workspace.word_wrap)
            {
                Some(block.head_line.min(total_lines.saturating_sub(1)))
            } else {
                let head = workspace.selection().head;
                Some(buffer.rope.byte_to_line(head.min(buffer.rope.len_bytes())))
            }
        } else {
            None
        };

        let first_line = (scroll_top / line_height).floor() as usize;
        // A zero line height saturates both casts to usize::MAX.
        let visible_count = ((bounds.size.height / line_height).ceil() as usize).saturating_add(1);
        let last_line = first_line.saturating_add(visible_count).min(total_lines);

        // Glyph geometry — small filled circle in the gutter's left margin.
        // Sized in rems so it scales with Cmd+= zoom alongside the gutter font.
        let glyph_diameter = window.rem_size() * (8. / 16.);
        let glyph_pad_left = window.rem_size() * (3. / 16.);
        // Begin/End Select glyph — a thin vertical bar on the gutter's left
        // edge spanning the full line height. Same width as the bookmark
        // padding so it occupies the leftmost gutter strip without colliding
        // with the bookmark circle further in.
        let begin_select_width = window.rem_size() * (2. / 16.);

        let mut labels = Vec::with_capacity(last_line.saturating_sub(first_line));
        let mut bookmark_glyphs = Vec::new();
        let mut begin_select_glyph: Option<Bounds<Pixels>> = None;
        let mut current_line_quad: Option<Bounds<Pixels>> = None;
        for line_idx in first_line..last_line {
            let line_top = bounds.top() + line_height * (line_idx as f32) - scroll_top;

            if Some(line_idx) == caret_line {
                current_line_quad = Some(Bounds::new(
                    point(bounds.left(), line_top),
                    size(bounds.size.width, line_height),
                ));
            }

            if show_line_numbers {
                let label: SharedString = format!("{}", line_idx + 1).into();
                let runs = vec![TextRun {
                    len: label.len(),
                    font: style.font(),
                    color: gutter_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let shaped = window
                    .text_system()
                    .shape_line(label.clone(), font_size, &runs, None);
                // Right-align inside the gutter bounds.
                let label_left = bounds.right() - shaped.width;
                labels.push((shaped, point(label_left, line_top)));
            }

            if show_bookmarks && buffer.is_line_bookmarked(line_idx) {
                let glyph_top = line_top + (line_height - glyph_diameter) * 0.5;
                let glyph_left = bounds.left() + glyph_pad_left;
                bookmark_glyphs.push(Bounds::new(
                    point(glyph_left, glyph_top),
                    size(glyph_diameter, glyph_diameter),
                ));
            }

            if Some(line_idx) == begin_select_line {
                begin_select_glyph = Some(Bounds::new(
                    point(bounds.left(), line_top),
                    size(begin_select_width, line_height),
                ));
            }
        }

        GutterPrepaint {
            labels,
            bookmark_glyphs,
            bookmark_color,
            begin_select_glyph,
            begin_select_color,
            current_line_quad,
            current_line_color,
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
        let line_height = window.line_height();
        let glyph_color = prepaint.bookmark_color;
        let glyphs: Vec<Bounds<Pixels>> = prepaint.bookmark_glyphs.drain(..).collect();
        let begin_select_glyph = prepaint.begin_select_glyph.take();
        let begin_select_color = prepaint.begin_select_color;
        let current_line_quad = prepaint.current_line_quad.take();
        let current_line_color = prepaint.current_line_color;
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            // Current-line backdrop paints first so the begin/end-select bar,
            // bookmark circles, and line-number text all layer above it.
            if let Some(band) = current_line_quad {
                window.paint_quad(fill(band, current_line_color));
            }
            // Begin/End Select bar paints first (left edge, full height) so
            // bookmark circles and the line-number text always layer above it.
            if let Some(bar) = begin_select_glyph {
                window.paint_quad(fill(bar, begin_select_color));
            }
            // Bookmarks paint next so the line-number text is never occluded
            // (the glyph sits in the left margin so they shouldn't overlap
            // anyway, but layering this way is the safe default).
            for glyph_bounds in glyphs {
                let radius = glyph_bounds.size.width * 0.5;
                window.paint_quad(fill(glyph_bounds, glyph_color).corner_radii(radius));
            }
            for (shaped, origin) in prepaint.labels.drain(..) {
                let _ = shaped.paint(
                    origin,
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });
    }
}
