// Bottom-docked preview pane for Text Tools — read-only text view that
// shows the live transformed output produced by the active tool while
// the Text Tools modal is open. Rendered conditionally on
// `Workspace::preview_pane`.

use gpui::{
    Bounds, CursorStyle, DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, ScrollWheelEvent, SharedString, Style,
    Styled, WeakEntity, Window, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::{
    PREVIEW_PANE_HANDLE_HEIGHT, PreviewPaneDrag, PreviewPaneState, Workspace,
};

const HEADER_HEIGHT: f32 = 26.0;
const VISIBLE_LINES_HINT: usize = 12;

pub(crate) fn text_tools_preview_panel(
    theme: &Theme,
    state: &PreviewPaneState,
    workspace: Entity<Workspace>,
    height: Pixels,
) -> impl IntoElement {
    let title = SharedString::from(state.title.clone());
    let line_count = state.content.lines().count();
    let scroll_top = state.scroll_top_lines.min(line_count.saturating_sub(1));
    let visible: Vec<SharedString> = state
        .content
        .lines()
        .skip(scroll_top)
        .take(VISIBLE_LINES_HINT * 4)
        .map(|line| {
            if line.is_empty() {
                SharedString::from(" ")
            } else {
                SharedString::from(line.to_string())
            }
        })
        .collect();
    let scroll_ws = workspace.clone();

    let body: gpui::AnyElement = if state.content.is_empty() {
        div()
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .px(px(10.))
            .py(px(8.))
            .child(SharedString::from(
                "Preview will appear here as you type. Apply to Tab writes the preview to the active tab.",
            ))
            .into_any_element()
    } else {
        let mut col = div().flex().flex_col().px(px(10.)).py(px(6.));
        for line in visible {
            col = col.child(
                div()
                    .text_color(rgb(theme.text))
                    .text_size(rems(12. / 16.))
                    .child(line),
            );
        }
        col.into_any_element()
    };

    let close_ws = workspace.clone();
    let close_btn = div()
        .id("tt-preview-close")
        .px(px(8.))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.chrome_text))
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.text_color(rgb(theme.text)))
        .child("✕")
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            close_ws.update(cx, |ws, cx| ws.close_preview(cx));
        });

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(HEADER_HEIGHT))
        .px(px(10.))
        .gap_2()
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border))
        .child(
            div()
                .flex_grow()
                .text_color(rgb(theme.text))
                .text_size(rems(12. / 16.))
                .child(title),
        )
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(format!(
                    "{} line{}",
                    line_count,
                    if line_count == 1 { "" } else { "s" }
                ))),
        )
        .child(close_btn);

    div()
        .id("tt-preview-pane")
        .flex()
        .flex_col()
        .flex_none()
        .h(height)
        .bg(rgb(theme.bg))
        .border_t_1()
        .border_color(rgb(theme.border))
        .child(header)
        .child(
            div()
                .flex_grow()
                .min_h(px(0.))
                .overflow_hidden()
                .child(body),
        )
        .on_scroll_wheel(move |ev: &ScrollWheelEvent, _w, cx| {
            // Coarse line-step scroll. One wheel notch ≈ ±3 lines.
            let dy_px = ev.delta.pixel_delta(px(20.)).y;
            let dy = f32::from(dy_px);
            if dy.abs() < 1.0 {
                return;
            }
            let lines = if dy > 0.0 { -3 } else { 3 };
            scroll_ws.update(cx, |ws, cx| ws.preview_scroll_by_lines(lines, cx));
        })
}

/// Splitter handle between the editor pane and the preview pane. Custom
/// `Element` so its `paint` can register window-global mouse handlers —
/// without those, a drag would cancel the moment the cursor left the
/// 6-px hit area. Mirrors `SidebarResizeHandle` in `ui::preferences::widgets`.
pub(crate) struct PreviewPaneResizeHandle {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) theme_chrome: u32,
    pub(crate) theme_border: u32,
}

impl IntoElement for PreviewPaneResizeHandle {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PreviewPaneResizeHandle {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = px(PREVIEW_PANE_HANDLE_HEIGHT).into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        // Paint the handle: chrome bar with a top border so it visually
        // separates the editor from the preview body, plus the
        // resize-up-down hover cursor over the entire hit area.
        window.set_cursor_style(CursorStyle::ResizeUpDown, hitbox);
        window.paint_quad(gpui::fill(bounds, rgb(self.theme_chrome)));
        let top_line = Bounds::new(
            bounds.origin,
            gpui::size(bounds.size.width, px(1.)),
        );
        window.paint_quad(gpui::fill(top_line, rgb(self.theme_border)));

        let viewport_h = window.viewport_size().height;

        let ws_for_down = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            let _ = ws_for_down.update(cx, |w, cx| {
                w.preview_pane_drag = Some(PreviewPaneDrag {
                    start_mouse_y: event.position.y,
                    start_height: w.preview_pane_height,
                });
                cx.notify();
            });
        });

        let ws_for_move = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let _ = ws_for_move.update(cx, |w, cx| {
                let Some(drag) = w.preview_pane_drag else {
                    return;
                };
                // Drag UP → cursor.y decreases → height grows. So new height
                // = start_height + (start_y - current_y).
                let dy = drag.start_mouse_y - event.position.y;
                let next = drag.start_height + dy;
                w.set_preview_pane_height(next, viewport_h, cx);
            });
        });

        let ws_for_up = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let _ = ws_for_up.update(cx, |w, cx| {
                if w.preview_pane_drag.is_some() {
                    w.preview_pane_drag = None;
                    cx.notify();
                }
            });
        });
    }
}
