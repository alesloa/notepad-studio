// Color picker popover for Dark Mode → Tones → Customized slot overrides.
//
// Mirrors NPP's standard Win32 ChooseColor popover: an 8×6 grid of preset
// swatches the user can click to assign a color to a customized slot. The
// popover is rendered as an overlay above the dialog body when the user
// clicks an enabled swatch in the Tones group.
//
// State lives on `PreferencesDialog::color_picker` (this struct). The user
// closes the picker by clicking a swatch (commits the color) or by clicking
// outside the popover (dismisses without change). Closure flows through
// `PreferencesDialog::close_color_picker`.

use gpui::{
    App, Bounds, CursorStyle, ElementInputHandler, FocusHandle, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Rgba, SharedString, Styled, TextRun,
    WeakEntity, Window, div, fill, point, prelude::*, px, rems, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::theme::Theme;

use super::PreferencesDialog;

/// The slot the user is currently picking a color for, plus where the
/// picker popover should anchor (top-left in dialog-local coordinates).
#[derive(Clone, Copy, Debug)]
pub(super) struct ColorPickerState {
    pub(super) slot_idx: usize,
    pub(super) anchor_x: Pixels,
    pub(super) anchor_y: Pixels,
}

/// 8 columns × 6 rows = 48 standard preset colors. Mirrors the Win32 basic
/// + extended color palette shown by `ChooseColor` (the dialog NPP opens
/// when you click a Customized slot). Order: each row goes from dark to
/// light, top-to-bottom transitions hue family to hue family.
pub(super) const COLOR_GRID: &[u32] = &[
    // Row 1 — dark/saturated primaries
    0x000000, 0x800000, 0x8B4513, 0x808000, 0x008000, 0x008080, 0x000080, 0x4B0082,
    // Row 2 — pure primaries
    0x696969, 0xFF0000, 0xFFA500, 0xFFFF00, 0x00FF00, 0x00CED1, 0x0000FF, 0x8A2BE2,
    // Row 3 — bright tertiaries
    0x808080, 0xFF1493, 0xFFD700, 0xADFF2F, 0x00FA9A, 0x00FFFF, 0x1E90FF, 0xBA55D3,
    // Row 4 — pastels
    0xA9A9A9, 0xFF69B4, 0xFFB347, 0xFFFFE0, 0x90EE90, 0xAFEEEE, 0x87CEEB, 0xDDA0DD,
    // Row 5 — light pastels
    0xD3D3D3, 0xFFC0CB, 0xFFE4B5, 0xFFFACD, 0xF0FFF0, 0xE0FFFF, 0xE6F2FF, 0xF8F0FF,
    // Row 6 — neutrals
    0xFFFFFF, 0xFAFAFA, 0xF5F5F5, 0xEEEEEE, 0xDDDDDD, 0xCCCCCC, 0xBBBBBB, 0xAAAAAA,
];

/// Render the picker overlay. Backdrop covers the entire dialog and
/// dismisses on click; the popover sits at the picker's anchor offset
/// from dialog origin.
pub(super) fn render(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    state: ColorPickerState,
) -> gpui::AnyElement {
    let weak_for_backdrop = weak.clone();
    let backdrop = div()
        .id("prefs-color-picker-backdrop")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_for_backdrop.update(cx, |this, cx| {
                this.close_color_picker(cx);
            });
        });

    // 32-px swatches with 1-px gap → 8 cols × 33 = 264 px wide; +12 px
    // popover padding × 2 → 288 px. Anchor at click point but clamp left
    // to keep popover on-screen via dialog body's overflow.
    let swatch_size = rems(28. / 16.);
    let swatch_gap = rems(2. / 16.);

    let mut rows = Vec::with_capacity(6);
    for row in 0..6 {
        let mut cells = Vec::with_capacity(8);
        for col in 0..8 {
            let idx = row * 8 + col;
            let color = COLOR_GRID[idx];
            let weak_for_pick = weak.clone();
            let id_str: SharedString = format!("prefs-picker-cell-{}", idx).into();
            let cell = div()
                .id(gpui::ElementId::Name(id_str))
                .w(swatch_size)
                .h(swatch_size)
                .bg(rgb(color))
                .border_1()
                .border_color(rgb(theme.border))
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.border_color(rgb(theme.toggle_active)))
                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                    let _ = weak_for_pick.update(cx, |this, cx| {
                        this.commit_color_pick(color, cx);
                    });
                })
                .into_any_element();
            cells.push(cell);
        }
        rows.push(
            div()
                .flex()
                .flex_row()
                .gap(swatch_gap)
                .children(cells)
                .into_any_element(),
        );
    }

    // "More Colours…" row: # prefix + 6-char hex input + Apply button.
    let hex_row = render_hex_row(theme, weak);

    // HSV surface (SV gradient + hue strip) reused from Style
    // Configurator. Sits above the swatch grid so the user can drag a
    // free-form colour without having to type a hex. Mounted via the
    // strong entity (upgrade from weak — when the weak is dead the
    // popover is about to tear down anyway, so an empty fallback row
    // is fine for the one frame between dismissal and rebuild).
    let hsv_row: gpui::AnyElement = match weak.upgrade() {
        Some(entity) => crate::ui::style_configurator::hsv_surface::render(
            theme, "prefs-color-picker", &entity,
        ),
        None => div().into_any_element(),
    };

    let popover = div()
        .absolute()
        .left(state.anchor_x)
        .top(state.anchor_y)
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .p(rems(8. / 16.))
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .child(hsv_row)
        .children(rows)
        .child(hex_row);

    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(backdrop)
        .child(popover)
        .into_any_element()
}

/// "More Colours…" row underneath the preset grid. `[ # ][ 6-char hex input ][ Apply ]`
/// — typing a 6-char hex + clicking Apply (or pressing Enter while focused)
/// commits the color to the slot and closes the picker. Invalid hex shows a
/// status footer message instead of committing.
fn render_hex_row(theme: &Theme, weak: &WeakEntity<PreferencesDialog>) -> gpui::AnyElement {
    let prefix = div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from("#"));

    let input = PickerHexInput {
        dialog: weak.clone(),
    };
    let input_box = div()
        .w(rems(60. / 16.))
        .h(rems(20. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .px(rems(6. / 16.))
        .child(input);

    let weak_apply = weak.clone();
    let apply_btn = div()
        .id("prefs-picker-hex-apply")
        .px(rems(8. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from("Apply"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_apply.update(cx, |this, cx| {
                this.commit_picker_hex(cx);
            });
        });

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(4. / 16.))
        .pt(rems(4. / 16.))
        .child(prefix)
        .child(input_box)
        .child(apply_btn)
        .into_any_element()
}

/// Hex input element — mirrors `widgets::DialogTextInput` but reads/writes
/// `dialog.picker_hex` instead of `dialog.fields`. IME route in
/// `EntityInputHandler` filters to hex + caps at 6 chars.
pub(super) struct PickerHexInput {
    pub(super) dialog: WeakEntity<PreferencesDialog>,
}

impl IntoElement for PickerHexInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for PickerHexInput {
    type RequestLayoutState = ();
    type PrepaintState = Option<(FocusHandle, Option<gpui::ShapedLine>, usize, usize)>;

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
        let f = &dialog_ref.picker_hex;
        let handle = dialog_ref.picker_hex_focus.clone();
        let text = f.text.clone();
        let cursor = f.cursor.min(text.len());
        let anchor = f.anchor.min(text.len());

        let display: SharedString = text.clone().into();
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
            let dialog_ref = dialog.read(cx);
            let f_text = dialog_ref.picker_hex.text.clone();
            let font_size = window
                .text_style()
                .font_size
                .to_pixels(window.rem_size());
            let display: SharedString = f_text.clone().into();
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
                let line = window
                    .text_system()
                    .shape_line(display, font_size, &runs, None);
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
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            dialog_for_click.update(cx, |this, cx| {
                let handle = this.picker_hex_focus.clone();
                window.focus(&handle, cx);
                let click_x = event.position.x - bounds.left();
                let byte = byte_for_x_in_text(&this.picker_hex.text, click_x, window);
                this.picker_hex.cursor = byte;
                this.picker_hex.anchor = byte;
                cx.notify();
            });
        });
    }
}

/// Shape one plain line into glyphs at the current text style. Used by the
/// hex input's click handler to map a local x back to a byte offset.
fn byte_for_x_in_text(text: &str, x: Pixels, window: &mut Window) -> usize {
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
    let line = window
        .text_system()
        .shape_line(display, font_size, &runs, None);
    let local = if x < px(0.) { px(0.) } else { x };
    line.closest_index_for_x(local).min(line.len)
}

/// Anchor coordinates used when no measured position is available. Falls
/// back to a sensible offset inside the content panel.
pub(super) fn fallback_anchor() -> (Pixels, Pixels) {
    (px(260.), px(240.))
}
