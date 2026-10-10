// Custom GPUI elements for the Text Tools dialog.
//
//   * `TextToolsInput`   — single-line bound to a `FieldId`.
//   * `MultiLineInput`   — multi-line bound to a `MultiFieldId`.
//   * helper functions   — buttons, labels, captions, label-wrapped inputs.
//
// Both inputs are CUSTOM `gpui::Element`s because they need direct text
// shaping, paint of caret/selection, hit-testing for click-to-position, and
// `window.handle_input` to route IME + typed characters back into the
// dialog's `EntityInputHandler` impl.

use std::ops::Range;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, ElementInputHandler, FocusHandle, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Rgba,
    ShapedLine, SharedString, StatefulInteractiveElement, Styled, TextRun, WeakEntity, Window, div,
    fill, point, px, rems, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::edit_ops::util::floor_str_boundary;
use crate::theme::Theme;

use super::{FieldId, MultiFieldId, TextToolsDialog};

// --- buttons + labels ----------------------------------------------------

pub(super) fn npp_button<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    on_click: F,
) -> gpui::AnyElement
where
    F: Fn(&mut App) + 'static,
{
    div()
        .id(id)
        .px(rems(12. / 16.))
        .py(rems(4. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            on_click(cx);
        })
        .into_any_element()
}

pub(super) fn npp_button_disabled(theme: &Theme, label: &'static str) -> gpui::AnyElement {
    div()
        .px(rems(12. / 16.))
        .py(rems(4. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.disabled_text))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.disabled_text))
        .child(SharedString::from(label))
        .into_any_element()
}

pub(super) fn npp_button_primary<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    on_click: F,
) -> gpui::AnyElement
where
    F: Fn(&mut App) + 'static,
{
    div()
        .id(id)
        .px(rems(14. / 16.))
        .py(rems(4. / 16.))
        .bg(rgb(theme.checkbox_fill))
        .border_1()
        .border_color(rgb(theme.checkbox_fill))
        .text_size(rems(12. / 16.))
        .text_color(rgb(0xffffff))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            on_click(cx);
        })
        .into_any_element()
}

pub(super) fn label_text(theme: &Theme, text: &str) -> gpui::AnyElement {
    div()
        .text_color(rgb(theme.text))
        .text_size(rems(12. / 16.))
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

pub(super) fn caption_text(theme: &Theme, text: &str) -> gpui::AnyElement {
    div()
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

#[allow(dead_code)]
pub(super) fn section_heading(theme: &Theme, text: &str) -> gpui::AnyElement {
    div()
        .text_color(rgb(theme.text))
        .text_size(rems(12. / 16.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

// --- single-line text field ----------------------------------------------

#[allow(dead_code)]
pub(super) fn render_text_field(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
    id: FieldId,
    label: &'static str,
    width: gpui::Rems,
) -> gpui::AnyElement {
    let handle = dialog.focus_for(id);
    let input_box = div()
        .id(id.container_id())
        .track_focus(&handle)
        .w(width)
        .h(rems(26. / 16.))
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .px(rems(6. / 16.))
        .py(rems(2. / 16.))
        .cursor(CursorStyle::IBeam)
        .child(TextToolsInput { dialog: weak.clone(), field_id: id });

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(
            div()
                .min_w(rems(110. / 16.))
                .text_color(rgb(theme.text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(label)),
        )
        .child(input_box)
        .into_any_element()
}

pub(super) fn render_inline_text_field(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
    id: FieldId,
    label: &'static str,
    width: gpui::Rems,
) -> gpui::AnyElement {
    let handle = dialog.focus_for(id);
    let focus_color = rgb(theme.checkbox_fill);
    let label_text = label.trim_end_matches(':').to_string();
    let mut input_box = div()
        .id(id.container_id())
        .track_focus(&handle)
        .h(rems(30. / 16.))
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .rounded(rems(4. / 16.))
        .px(rems(8. / 16.))
        .py(rems(4. / 16.))
        .cursor(CursorStyle::IBeam)
        .focus(|s| s.border_color(focus_color))
        .child(TextToolsInput { dialog: weak.clone(), field_id: id });
    if width.0 > 0.0 {
        input_box = input_box.w(width).flex_shrink_0();
    } else {
        input_box = input_box.w_full();
    }

    div()
        .flex()
        .flex_col()
        .gap(rems(4. / 16.))
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from(label_text)),
        )
        .child(input_box)
        .into_any_element()
}

// --- multi-line labelled text area ---------------------------------------

pub(super) fn render_text_area(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
    id: MultiFieldId,
    label: &'static str,
    height: gpui::Rems,
) -> gpui::AnyElement {
    let handle = dialog.focus_for_multi(id);
    let focus_color = rgb(theme.checkbox_fill);
    let label_text = label.trim_end_matches(':').to_string();
    let editor = div()
        .id(MultiFieldId::container_id(id))
        .track_focus(&handle)
        .w_full()
        .h(height)
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .rounded(rems(4. / 16.))
        .px(rems(8. / 16.))
        .py(rems(6. / 16.))
        .cursor(CursorStyle::IBeam)
        .overflow_y_scroll()
        .focus(|s| s.border_color(focus_color))
        .child(MultiLineInput {
            dialog: weak.clone(),
            field_id: id,
        });

    div()
        .flex()
        .flex_col()
        .gap(rems(4. / 16.))
        .w_full()
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from(label_text)),
        )
        .child(editor)
        .into_any_element()
}

// --- single-line custom Element ------------------------------------------

pub(super) struct TextToolsInput {
    pub(super) dialog: WeakEntity<TextToolsDialog>,
    pub(super) field_id: FieldId,
}

impl IntoElement for TextToolsInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for TextToolsInput {
    type RequestLayoutState = ();
    type PrepaintState = Option<(FocusHandle, Option<ShapedLine>, usize, usize)>;

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
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let dialog = self.dialog.upgrade()?;
        let dialog_ref = dialog.read(cx);
        let theme = dialog_ref.theme();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let f = dialog_ref.fields.get(self.field_id);
        let handle = dialog_ref.focus_for(self.field_id);
        let text = f.text.clone();
        let cursor = f.cursor.min(text.len());
        let anchor = f.anchor.min(text.len());

        let display: SharedString = text.into();
        let color: Hsla = Rgba::from(rgb(theme.text)).into();

        let shaped = if display.is_empty() {
            None
        } else {
            let runs = vec![TextRun {
                len: display.len(),
                font: text_style.font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            Some(window.text_system().shape_line(display, font_size, &runs, None))
        };

        Some((handle, shaped, cursor, anchor))
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
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let Some((handle, shaped, cursor, anchor)) = prepaint.as_mut() else {
            return;
        };
        let theme = dialog.read(cx).theme();
        let line_height = window.line_height();
        let baseline_y = bounds.top() + (bounds.size.height - line_height).max(px(0.)) / 2.0;

        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        if hi > lo {
            if let Some(line) = shaped.as_ref() {
                let lo_clamped = lo.min(line.len);
                let hi_clamped = hi.min(line.len);
                let x_lo = bounds.left() + line.x_for_index(lo_clamped);
                let x_hi = bounds.left() + line.x_for_index(hi_clamped);
                let sel_color: Hsla = rgba(theme.selection).into();
                window.paint_quad(fill(
                    Bounds::new(point(x_lo, baseline_y), size(x_hi - x_lo, line_height)),
                    sel_color,
                ));
            }
        }

        if let Some(line) = shaped.take() {
            let _ = line.paint(
                point(bounds.left(), baseline_y),
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        if handle.is_focused(window) {
            let f_text = dialog.read(cx).fields.get(self.field_id).text.clone();
            let font_size = window.text_style().font_size.to_pixels(window.rem_size());
            let display: SharedString = f_text.into();
            let caret_color: Hsla = Rgba::from(rgb(theme.text)).into();
            let cx_pos = if display.is_empty() {
                bounds.left()
            } else {
                let runs = vec![TextRun {
                    len: display.len(),
                    font: window.text_style().font(),
                    color: caret_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let line = window.text_system().shape_line(display, font_size, &runs, None);
                bounds.left() + line.x_for_index((*cursor).min(line.len))
            };
            window.paint_quad(fill(
                Bounds::new(
                    point(cx_pos, baseline_y),
                    size(px(CARET_WIDTH_PX), line_height),
                ),
                caret_color,
            ));
        }

        window.handle_input(handle, ElementInputHandler::new(bounds, dialog.clone()), cx);

        let dialog_for_click = dialog.clone();
        let field_id = self.field_id;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            dialog_for_click.update(cx, |this, cx| {
                let handle = this.focus_for(field_id);
                window.focus(&handle, cx);
                let f = this.fields.get_mut(field_id);
                let click_x = event.position.x - bounds.left();
                let byte = byte_for_x_singleline(&f.text, click_x, window);
                f.collapse_to(byte);
                cx.notify();
            });
        });
    }
}

fn byte_for_x_singleline(text: &str, x: Pixels, window: &mut Window) -> usize {
    if text.is_empty() {
        return 0;
    }
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let display: SharedString = text.to_string().into();
    let runs = vec![TextRun {
        len: display.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window.text_system().shape_line(display, font_size, &runs, None);
    line.closest_index_for_x(x).min(text.len())
}

// --- multi-line custom Element -------------------------------------------

pub(super) struct MultiLineInput {
    pub(super) dialog: WeakEntity<TextToolsDialog>,
    pub(super) field_id: MultiFieldId,
}

impl IntoElement for MultiLineInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub(super) struct MultiPrepaint {
    pub(super) handle: FocusHandle,
    /// Shaped rows `first_row..`, the ones inside the scroll clip; `None`
    /// for an empty row.
    pub(super) shaped_lines: Vec<Option<ShapedLine>>,
    pub(super) first_row: usize,
    /// Start byte of every row, visible or not.
    pub(super) line_offsets: Vec<usize>,
    pub(super) cursor: usize,
    pub(super) anchor: usize,
    pub(super) text_len: usize,
}

impl gpui::Element for MultiLineInput {
    type RequestLayoutState = ();
    type PrepaintState = Option<MultiPrepaint>;

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
        let lines = self
            .dialog
            .upgrade()
            .map(|d| {
                let r = d.read(cx);
                let s = &r.multi_fields.get(self.field_id).text;
                s.split('\n').count().max(1)
            })
            .unwrap_or(1);
        let line_h_px = window.line_height();
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::Length::Definite((line_h_px * lines as f32).into());
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let dialog = self.dialog.upgrade()?;
        let dialog_ref = dialog.read(cx);
        let theme = dialog_ref.theme();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let f = dialog_ref.multi_fields.get(self.field_id);
        let handle = dialog_ref.focus_for_multi(self.field_id);
        let text = f.text.as_str();
        let cursor = floor_str_boundary(text, f.cursor);
        let anchor = floor_str_boundary(text, f.anchor);
        let text_len = text.len();
        // The input opens seeded with the whole document. Shaping every row
        // each frame hangs on a large file, so shape only the clipped rows.
        let clip = window.content_mask().bounds;
        let visible = visible_rows(
            bounds.top().into(),
            clip.top().into(),
            clip.bottom().into(),
            window.line_height().into(),
        );

        let color: Hsla = Rgba::from(rgb(theme.text)).into();
        let mut shaped_lines: Vec<Option<ShapedLine>> = Vec::new();
        let mut line_offsets: Vec<usize> = Vec::new();
        let mut byte = 0usize;
        for (row, raw) in text.split('\n').enumerate() {
            line_offsets.push(byte);
            byte += raw.len() + 1;
            if !visible.contains(&row) {
                continue;
            }
            if raw.is_empty() {
                shaped_lines.push(None);
            } else {
                let display: SharedString = raw.to_string().into();
                let runs = vec![TextRun {
                    len: display.len(),
                    font: text_style.font(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                shaped_lines.push(Some(window.text_system().shape_line(
                    display, font_size, &runs, None,
                )));
            }
        }

        Some(MultiPrepaint {
            handle,
            shaped_lines,
            first_row: visible.start,
            line_offsets,
            cursor,
            anchor,
            text_len,
        })
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
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let Some(state) = prepaint.as_mut() else {
            return;
        };
        let theme = dialog.read(cx).theme();
        let line_height = window.line_height();

        let lo = state.cursor.min(state.anchor);
        let hi = state.cursor.max(state.anchor);
        let sel_color: Hsla = rgba(theme.selection).into();

        if hi > lo {
            let row_count = state.line_offsets.len();
            let lo_row = byte_to_row_col_offsets(&state.line_offsets, state.text_len, lo).0;
            for (i, shaped) in state.shaped_lines.iter().enumerate() {
                let row_idx = state.first_row.saturating_add(i);
                let Some(&line_start) = state.line_offsets.get(row_idx) else {
                    continue;
                };
                let line_text_len = shaped.as_ref().map(|l| l.len).unwrap_or(0);
                let line_end = line_start + line_text_len;
                let row_y = bounds.top() + line_height * row_idx as f32;
                let sel_lo = lo.max(line_start);
                let sel_hi = hi.min(line_end);
                let row_includes_newline = hi > line_end && row_idx + 1 < row_count;
                if sel_hi > sel_lo {
                    let local_lo = sel_lo - line_start;
                    let local_hi = sel_hi - line_start;
                    let x_lo = bounds.left()
                        + shaped
                            .as_ref()
                            .map(|l| l.x_for_index(local_lo.min(l.len)))
                            .unwrap_or(px(0.));
                    let x_hi = bounds.left()
                        + shaped
                            .as_ref()
                            .map(|l| l.x_for_index(local_hi.min(l.len)))
                            .unwrap_or(px(0.));
                    let extra_w = if row_includes_newline { px(8.) } else { px(0.) };
                    window.paint_quad(fill(
                        Bounds::new(point(x_lo, row_y), size(x_hi - x_lo + extra_w, line_height)),
                        sel_color,
                    ));
                } else if row_includes_newline && row_idx >= lo_row {
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.left(), row_y), size(px(8.), line_height)),
                        sel_color,
                    ));
                }
            }
        }

        let mut shaped_lines = std::mem::take(&mut state.shaped_lines);
        for (i, shaped) in shaped_lines.iter_mut().enumerate() {
            let row_y = bounds.top() + line_height * state.first_row.saturating_add(i) as f32;
            if let Some(line) = shaped.take() {
                let _ = line.paint(
                    point(bounds.left(), row_y),
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }

        if state.handle.is_focused(window) {
            let cursor = state.cursor;
            let (row, col_byte) =
                byte_to_row_col_offsets(&state.line_offsets, state.text_len, cursor);
            let row_y = bounds.top() + line_height * row as f32;
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let row_text = {
                let text = &dialog.read(cx).multi_fields.get(self.field_id).text;
                text.get(row_byte_range(text, row)).unwrap_or("").to_string()
            };
            let cx_pos = if row_text.is_empty() {
                bounds.left()
            } else {
                let display: SharedString = row_text.clone().into();
                let runs = vec![TextRun {
                    len: display.len(),
                    font: style.font(),
                    color: Rgba::from(rgb(theme.text)).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let line = window.text_system().shape_line(display, font_size, &runs, None);
                bounds.left() + line.x_for_index(col_byte.min(line.len))
            };
            let caret_color: Hsla = Rgba::from(rgb(theme.text)).into();
            window.paint_quad(fill(
                Bounds::new(
                    point(cx_pos, row_y),
                    size(px(CARET_WIDTH_PX), line_height),
                ),
                caret_color,
            ));
        }

        window.handle_input(
            &state.handle,
            ElementInputHandler::new(bounds, dialog.clone()),
            cx,
        );

        let dialog_for_click = dialog.clone();
        let field_id = self.field_id;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            let line_h: f32 = window.line_height().into();
            let local_y_px: f32 = (event.position.y - bounds.top()).into();
            let row = (local_y_px / line_h).floor().max(0.0) as usize;
            dialog_for_click.update(cx, |this, cx| {
                let handle = this.focus_for_multi(field_id);
                window.focus(&handle, cx);
                let f = this.multi_fields.get_mut(field_id);
                // Row bounds come from the text as it is now: an edit can
                // land between this frame's paint and the click.
                let range = row_byte_range(&f.text, row);
                let line_start = range.start;
                let row_text = f.text.get(range).unwrap_or("").to_string();
                let click_x = event.position.x - bounds.left();
                let local_byte = byte_for_x_singleline(&row_text, click_x, window);
                f.collapse_to(line_start + local_byte);
                cx.notify();
            });
        });
    }
}

/// Byte range of row `row` of `text`, split on `\n` and without it. A row
/// past the last maps to the last.
fn row_byte_range(text: &str, row: usize) -> Range<usize> {
    let mut start = 0usize;
    let mut range = 0..0;
    for (i, line) in text.split('\n').enumerate() {
        range = start..start + line.len();
        if i == row {
            break;
        }
        start = range.end + 1;
    }
    range
}

/// Rows of height `line_h` laid out from `top` that meet the clip
/// `clip_top..clip_bottom`, with one row of slack on each side.
fn visible_rows(top: f32, clip_top: f32, clip_bottom: f32, line_h: f32) -> Range<usize> {
    if !line_h.is_finite() || line_h <= 0.0 {
        return 0..0;
    }
    // Float-to-int `as` saturates and maps NaN to 0.
    let first = (((clip_top - top) / line_h).floor() - 1.0).max(0.0) as usize;
    let last = (((clip_bottom - top) / line_h).ceil() + 1.0).max(0.0) as usize;
    first..last.max(first)
}

/// `(row, column byte)` of `byte`, given each row's start byte.
pub(super) fn byte_to_row_col_offsets(
    line_offsets: &[usize],
    text_len: usize,
    byte: usize,
) -> (usize, usize) {
    let byte = byte.min(text_len);
    // Offsets ascend: the last row starting at or before `byte` holds it.
    let row = line_offsets.partition_point(|&off| off <= byte).saturating_sub(1);
    let line_start = line_offsets.get(row).copied().unwrap_or(0);
    (row, byte.saturating_sub(line_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_byte_range_follows_the_current_text() {
        // é=0..2 \n=2 x=3 \r=4 \n=5 中=6..9
        let s = "é\nx\r\n中";
        assert_eq!(row_byte_range(s, 0), 0..2);
        assert_eq!(row_byte_range(s, 1), 3..5);
        assert_eq!(row_byte_range(s, 2), 6..9);
        assert_eq!(row_byte_range(s, 99), 6..9);
        assert_eq!(row_byte_range("", 3), 0..0);
        assert_eq!(row_byte_range("a\n", 5), 2..2);
        for row in 0..4 {
            assert!(s.get(row_byte_range(s, row)).is_some(), "row {row}");
        }
    }

    #[test]
    fn row_col_lookup_survives_empty_offsets() {
        // An empty offset table indexed `line_offsets[0]`.
        assert_eq!(byte_to_row_col_offsets(&[], 0, 5), (0, 0));
        assert_eq!(byte_to_row_col_offsets(&[], 9, 5), (0, 5));
        let offsets = [0, 3, 4, 7];
        assert_eq!(byte_to_row_col_offsets(&offsets, 9, 0), (0, 0));
        assert_eq!(byte_to_row_col_offsets(&offsets, 9, 3), (1, 0));
        assert_eq!(byte_to_row_col_offsets(&offsets, 9, 6), (2, 2));
        assert_eq!(byte_to_row_col_offsets(&offsets, 9, 99), (3, 2));
    }

    #[test]
    fn only_rows_inside_the_clip_are_shaped() {
        // 20px rows from y=100, clipped to y=150..250: rows 2..8 with slack.
        assert_eq!(visible_rows(100.0, 150.0, 250.0, 20.0), 1..9);
        // Scrolled so the input starts far above the clip.
        assert_eq!(visible_rows(-1_000_000.0, 0.0, 400.0, 20.0), 49_999..50_021);
        // Clip above the input: nothing, or only the slack row.
        assert_eq!(visible_rows(500.0, 0.0, 100.0, 20.0), 0..0);
        assert_eq!(visible_rows(0.0, 0.0, 100.0, 0.0), 0..0);
        assert_eq!(visible_rows(0.0, 0.0, 100.0, f32::NAN), 0..0);
        assert_eq!(visible_rows(0.0, f32::NAN, f32::NAN, 20.0), 0..0);
    }
}
