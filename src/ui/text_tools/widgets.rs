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

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, ElementInputHandler, FocusHandle, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Rgba,
    ShapedLine, SharedString, StatefulInteractiveElement, Styled, TextRun, WeakEntity, Window, div,
    fill, point, px, rems, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
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
    pub(super) shaped_lines: Vec<Option<ShapedLine>>,
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
        let f = dialog_ref.multi_fields.get(self.field_id);
        let handle = dialog_ref.focus_for_multi(self.field_id);
        let text = f.text.clone();
        let cursor = f.cursor.min(text.len());
        let anchor = f.anchor.min(text.len());
        let text_len = text.len();

        let color: Hsla = Rgba::from(rgb(theme.text)).into();
        let mut shaped_lines: Vec<Option<ShapedLine>> = Vec::new();
        let mut line_offsets: Vec<usize> = Vec::new();
        let mut byte = 0usize;
        for raw in text.split('\n') {
            line_offsets.push(byte);
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
            byte += raw.len() + 1;
        }

        Some(MultiPrepaint {
            handle,
            shaped_lines,
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
            for (row_idx, shaped) in state.shaped_lines.iter().enumerate() {
                let line_start = state.line_offsets[row_idx];
                let line_text_len = shaped.as_ref().map(|l| l.len).unwrap_or(0);
                let line_end = line_start + line_text_len;
                let row_y = bounds.top() + line_height * row_idx as f32;
                let sel_lo = lo.max(line_start);
                let sel_hi = hi.min(line_end);
                let row_includes_newline =
                    hi > line_end && row_idx + 1 < state.shaped_lines.len();
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
                } else if row_includes_newline && row_idx >= line_offset_of_byte(state, lo) {
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.left(), row_y), size(px(8.), line_height)),
                        sel_color,
                    ));
                }
            }
        }

        let mut shaped_lines = std::mem::take(&mut state.shaped_lines);
        for (row_idx, shaped) in shaped_lines.iter_mut().enumerate() {
            let row_y = bounds.top() + line_height * row_idx as f32;
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
            let row_text = dialog
                .read(cx)
                .multi_fields
                .get(self.field_id)
                .text
                .split('\n')
                .nth(row)
                .unwrap_or("")
                .to_string();
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
        let line_offsets_clone = state.line_offsets.clone();
        let text_len = state.text_len;
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
            let row = row.min(line_offsets_clone.len().saturating_sub(1));
            let line_start = line_offsets_clone[row];
            let line_end = line_offsets_clone
                .get(row + 1)
                .map(|n| n - 1)
                .unwrap_or(text_len);
            dialog_for_click.update(cx, |this, cx| {
                let handle = this.focus_for_multi(field_id);
                window.focus(&handle, cx);
                let f = this.multi_fields.get_mut(field_id);
                let row_text = f.text[line_start..line_end].to_string();
                let click_x = event.position.x - bounds.left();
                let local_byte = byte_for_x_singleline(&row_text, click_x, window);
                f.collapse_to(line_start + local_byte);
                cx.notify();
            });
        });
    }
}

fn line_offset_of_byte(state: &MultiPrepaint, byte: usize) -> usize {
    let mut idx = 0usize;
    for (i, off) in state.line_offsets.iter().enumerate() {
        if *off <= byte {
            idx = i;
        } else {
            break;
        }
    }
    idx
}

pub(super) fn byte_to_row_col_offsets(
    line_offsets: &[usize],
    text_len: usize,
    byte: usize,
) -> (usize, usize) {
    let byte = byte.min(text_len);
    let mut row = 0;
    for (i, off) in line_offsets.iter().enumerate() {
        if *off <= byte {
            row = i;
        } else {
            break;
        }
    }
    let line_start = line_offsets[row];
    (row, byte - line_start)
}
