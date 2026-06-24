// SPDX-License-Identifier: Apache-2.0
//
// Custom text input element for the Style Configurator's User-ext field.
//
// Mirrors `colour_picker::HexInput` (same pattern as preferences'
// `DialogTextInput`): wrapper div carries `.id() + .track_focus()` so the
// focus_id anchors to a dedicated dispatch-tree node, then this Element's
// `paint` calls `window.handle_input(...)` so typed chars / paste / IME
// route to the dialog's `EntityInputHandler::replace_text_in_range`.
// The handler dispatches by focus to either `picker_buffer` or
// `user_ext_buffer` — see `mod.rs::impl EntityInputHandler`.
//
// Visuals: buffer rendered as-is (no leading prefix, unlike HexInput's
// `#`). Empty buffer shows a dimmed placeholder ("e.g. log cfg") so the
// user knows what the field is for. Selection highlight + caret only when
// focused. Mouse-down sets / extends selection; window-global mouse-move
// + mouse-up handle drag-to-select past the field's edges.

use gpui::{
    App, Bounds, ElementInputHandler, FocusHandle, Hsla, IntoElement, MouseButton, Pixels, Rgba,
    SharedString, TextRun, WeakEntity, Window, fill, point, px, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;

use super::StyleConfiguratorDialog;

const PLACEHOLDER: &str = "(no user extensions)";

pub(super) struct UserExtInput {
    pub(super) dialog: WeakEntity<StyleConfiguratorDialog>,
}

impl IntoElement for UserExtInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for UserExtInput {
    type RequestLayoutState = ();
    type PrepaintState =
        Option<(FocusHandle, Option<gpui::ShapedLine>, usize, usize, String, bool)>;

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
        let buffer = dialog_ref.user_ext_buffer.clone();
        let cursor = dialog_ref.user_ext_cursor.min(buffer.len());
        let anchor = dialog_ref.user_ext_anchor.min(buffer.len());
        let handle = dialog_ref.user_ext_focus.clone();
        let theme = dialog_ref.theme();
        let is_empty = buffer.is_empty();

        let display_text = if is_empty {
            PLACEHOLDER.to_string()
        } else {
            buffer.clone()
        };

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let color: Hsla = if is_empty {
            Rgba::from(rgb(theme.disabled_text)).into()
        } else {
            Rgba::from(rgb(theme.text)).into()
        };
        let runs = vec![TextRun {
            len: display_text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let shaped = Some(window.text_system().shape_line(
            SharedString::from(display_text),
            font_size,
            &runs,
            None,
        ));

        Some((handle, shaped, cursor, anchor, buffer, is_empty))
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
        let Some((handle, shaped, cursor, anchor, buffer, is_empty)) = prepaint.as_mut() else {
            return;
        };
        let theme = dialog.read(cx).theme();
        let line_height = window.line_height();
        let baseline_y = bounds.top() + (bounds.size.height - line_height).max(px(0.)) / 2.0;

        // Selection rect. Only paints when the buffer is non-empty —
        // selecting the placeholder text would be nonsense.
        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        if hi > lo && !*is_empty {
            if let Some(line) = shaped.as_ref() {
                let lo_disp = lo.min(line.len);
                let hi_disp = hi.min(line.len);
                let x_lo = bounds.left() + line.x_for_index(lo_disp);
                let x_hi = bounds.left() + line.x_for_index(hi_disp);
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

        // Caret — only when focused. Position at cursor byte (or 0 when
        // buffer is empty, so the caret sits at the start of the field).
        if handle.is_focused(window) {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let caret_text = if *is_empty {
                String::new()
            } else {
                buffer.clone()
            };
            let caret_color: Hsla = Rgba::from(rgb(theme.text)).into();
            let runs = vec![TextRun {
                len: caret_text.len(),
                font: style.font(),
                color: caret_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let line = window.text_system().shape_line(
                SharedString::from(caret_text.clone()),
                font_size,
                &runs,
                None,
            );
            let cx_idx = if *is_empty {
                0
            } else {
                (*cursor).min(line.len)
            };
            let cx_pos = bounds.left() + line.x_for_index(cx_idx);
            window.paint_quad(fill(
                Bounds::new(
                    point(cx_pos, baseline_y),
                    size(px(CARET_WIDTH_PX), line_height),
                ),
                caret_color,
            ));
        }

        // IME / OS text input router. Without this typed chars / paste /
        // dead-key composition do nothing.
        window.handle_input(handle, ElementInputHandler::new(bounds, dialog.clone()), cx);

        // Mouse-down — focus the field, place the cursor at the click x,
        // start a drag. Double-click selects the entire buffer (parity
        // with native fields where double-click selects the word; for an
        // ext list "the word" is the whole value).
        let dialog_for_down = dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            cx.stop_propagation();
            let click_count = event.click_count;
            dialog_for_down.update(cx, |this, cx| {
                let handle = this.user_ext_focus.clone();
                window.focus(&handle, cx);
                if click_count >= 2 {
                    this.user_ext_anchor = 0;
                    this.user_ext_cursor = this.user_ext_buffer.len();
                    this.user_ext_drag_active = false;
                } else {
                    let click_x = event.position.x - bounds.left();
                    let buf_byte = byte_for_x_in_buffer(&this.user_ext_buffer, click_x, window);
                    this.user_ext_cursor = buf_byte;
                    this.user_ext_anchor = buf_byte;
                    this.user_ext_drag_active = true;
                }
                cx.notify();
            });
        });

        // Mouse-move — window-global so the drag tracks past the field
        // edges. Extends cursor only; anchor stays pinned at mouse-down.
        let dialog_for_move = dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            if !event.dragging() {
                return;
            }
            dialog_for_move.update(cx, |this, cx| {
                if !this.user_ext_drag_active {
                    return;
                }
                let click_x = event.position.x - bounds.left();
                let buf_byte = byte_for_x_in_buffer(&this.user_ext_buffer, click_x, window);
                if this.user_ext_cursor != buf_byte {
                    this.user_ext_cursor = buf_byte;
                    cx.notify();
                }
            });
        });

        // Mouse-up — end any in-progress drag. Window-global so a release
        // outside the field bounds still terminates the drag.
        let dialog_for_up = dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            dialog_for_up.update(cx, |this, _cx| {
                this.user_ext_drag_active = false;
            });
        });
    }
}

/// Map a click x-offset (relative to the input bounds) to a byte position
/// inside the user-ext buffer. Returns 0 on click before the first char,
/// `len` on click past the end. Identical shape to
/// `colour_picker::byte_for_x_in_buffer` minus the `#` prefix arithmetic.
fn byte_for_x_in_buffer(buffer: &str, x: Pixels, window: &mut Window) -> usize {
    if buffer.is_empty() {
        return 0;
    }
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let runs = vec![TextRun {
        len: buffer.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window.text_system().shape_line(
        SharedString::from(buffer.to_string()),
        font_size,
        &runs,
        None,
    );
    let local = if x < px(0.) { px(0.) } else { x };
    line.closest_index_for_x(local).min(buffer.len())
}
