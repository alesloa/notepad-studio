// Auto-Insert ▸ user-defined matched-pair list editor.
//
// A pair-per-row table inside the Auto-Insert group:
//   [ open input ] → [ close input ] [ × delete ]
//   [ open input ] → [ close input ] [ × delete ]
//   ...
//   [ ＋ Add pair ]
//
// Each input is a custom gpui Element wired to the dialog's
// `EntityInputHandler` via per-buffer focus handles. Edits persist immediately
// — every keystroke / paste / delete mirrors `user_pairs` back into
// `Settings::auto_insert_user_pairs` and triggers a save. Empty pairs are
// preserved as-is in settings; the matched-pair lookup at
// `workspace/input_handler.rs::pair_close_for` silently skips entries whose
// open or close half is empty, so a half-typed row is harmless.

use gpui::{
    App, Bounds, CursorStyle, ElementInputHandler, FocusHandle, Hsla, IntoElement, MouseButton,
    ParentElement, Pixels, Rgba, SharedString, Styled, TextRun, WeakEntity, Window, div, fill,
    point, prelude::*, px, rems, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::theme::Theme;

use super::widgets::nudge_workspaces;
use super::{FieldState, PreferencesDialog};

/// Which half of a user-defined pair the input is editing. Used by routing
/// helpers and the input element to pick which `FieldState` / `FocusHandle`
/// inside the buffer to read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PairHalf {
    Open,
    Close,
}

/// Per-row backing storage. The two `FieldState`s drive the input glyphs +
/// selection / caret; the two `FocusHandle`s wire each input into the dialog's
/// IME + keyboard router.
pub(crate) struct UserPairBuffer {
    pub(super) open: FieldState,
    pub(super) close: FieldState,
    pub(super) open_focus: FocusHandle,
    pub(super) close_focus: FocusHandle,
}

impl UserPairBuffer {
    pub(super) fn new(open: String, close: String, cx: &mut gpui::App) -> Self {
        Self {
            open: FieldState::from_text(open),
            close: FieldState::from_text(close),
            open_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
        }
    }

    pub(super) fn buffer(&self, half: PairHalf) -> &FieldState {
        match half {
            PairHalf::Open => &self.open,
            PairHalf::Close => &self.close,
        }
    }

    pub(super) fn buffer_mut(&mut self, half: PairHalf) -> &mut FieldState {
        match half {
            PairHalf::Open => &mut self.open,
            PairHalf::Close => &mut self.close,
        }
    }

    pub(super) fn focus(&self, half: PairHalf) -> FocusHandle {
        match half {
            PairHalf::Open => self.open_focus.clone(),
            PairHalf::Close => self.close_focus.clone(),
        }
    }
}

/// Shape one line of plain text into glyphs at the current text style. Used
/// by the click handler to map a local x back to a byte offset.
fn byte_for_x(text: &str, x: Pixels, window: &mut Window) -> usize {
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

/// The custom Element. One per (row, half). Mirrors `widgets::DialogTextInput`,
/// except the FieldState lives in `dialog.user_pairs[idx]` instead of
/// `dialog.fields`.
pub(super) struct UserPairInput {
    pub(super) dialog: WeakEntity<PreferencesDialog>,
    pub(super) idx: usize,
    pub(super) half: PairHalf,
}

impl IntoElement for UserPairInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for UserPairInput {
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
        let buf = dialog_ref.user_pairs.get(self.idx)?;
        let theme = dialog_ref.theme();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let f = buf.buffer(self.half);
        let handle = buf.focus(self.half);
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
            if let Some(buf) = dialog_ref.user_pairs.get(self.idx) {
                let f = buf.buffer(self.half);
                let f_text = f.text.clone();
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
        }

        window.handle_input(handle, ElementInputHandler::new(bounds, dialog.clone()), cx);

        let dialog_for_click = dialog.clone();
        let idx = self.idx;
        let half = self.half;
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            dialog_for_click.update(cx, |this, cx| {
                let Some(buf) = this.user_pairs.get_mut(idx) else {
                    return;
                };
                let handle = buf.focus(half);
                window.focus(&handle, cx);
                let f = buf.buffer_mut(half);
                let click_x = event.position.x - bounds.left();
                let byte = byte_for_x(&f.text, click_x, window);
                f.collapse_to(byte);
                cx.notify();
            });
        });
    }
}

/// Render the whole "User-defined pairs" sub-block (header + N rows + Add).
pub(super) fn render_block(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    count: usize,
) -> gpui::AnyElement {
    let header = div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(
            "User-defined matched pairs — empty rows are ignored.",
        ));

    let mut rows = div().flex().flex_col().gap(rems(4. / 16.));
    for i in 0..count {
        rows = rows.child(render_row(theme, weak, i));
    }

    let weak_add = weak.clone();
    let add_btn = div()
        .id("prefs-userpair-add")
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
        .child(SharedString::from("＋ Add pair"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_add.update(cx, |this, cx| {
                this.add_user_pair(cx);
            });
            nudge_workspaces(cx);
        });

    let add_row = div().flex().flex_row().child(add_btn);

    div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .child(header)
        .child(rows)
        .child(add_row)
        .into_any_element()
}

fn render_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    idx: usize,
) -> gpui::AnyElement {
    let open_input = labelled_input(theme, weak, idx, PairHalf::Open);
    let close_input = labelled_input(theme, weak, idx, PairHalf::Close);

    let arrow = div()
        .px(rems(4. / 16.))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from("→"));

    let weak_del = weak.clone();
    let delete_id: SharedString = format!("prefs-userpair-del-{idx}").into();
    let delete_btn = div()
        .id(gpui::ElementId::from(delete_id))
        .w(rems(20. / 16.))
        .h(rems(20. / 16.))
        .ml(rems(6. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from("×"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_del.update(cx, |this, cx| {
                this.remove_user_pair(idx, cx);
            });
            nudge_workspaces(cx);
        });

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(4. / 16.))
        .child(open_input)
        .child(arrow)
        .child(close_input)
        .child(delete_btn)
        .into_any_element()
}

fn labelled_input(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    idx: usize,
    half: PairHalf,
) -> gpui::AnyElement {
    let input = UserPairInput {
        dialog: weak.clone(),
        idx,
        half,
    };
    div()
        .w(rems(48. / 16.))
        .h(rems(22. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .px(rems(6. / 16.))
        .child(input)
        .into_any_element()
}
