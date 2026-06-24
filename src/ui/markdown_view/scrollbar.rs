// Markdown preview scrollbar — paints a draggable thumb on the right edge of
// the markdown preview pane. Reads scroll state from the persistent
// `ScrollHandle` on the workspace; the drag offset (offset within the
// thumb where the user grabbed) lives on `Workspace.md_preview_scroll_drag`
// so it survives across paints.

use gpui::{
    App, Bounds, DispatchPhase, Edges, Element, ElementId, Entity, GlobalElementId, Hsla,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Position, ScrollHandle, Size, Style, Window, fill, point, px, relative, size,
};

use crate::workspace::Workspace;

pub(crate) const MD_PREVIEW_SCROLLBAR_THICKNESS: f32 = 10.;
const MD_PREVIEW_SCROLLBAR_MIN_THUMB: f32 = 30.;

pub(crate) struct MarkdownPreviewScrollbar {
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) handle: ScrollHandle,
    pub(crate) color: Hsla,
    pub(crate) color_hover: Hsla,
}

pub(crate) struct MarkdownPreviewScrollbarPrepaint {
    thumb: Option<Bounds<Pixels>>,
    track: Bounds<Pixels>,
    max_offset_y: Pixels,
    movable: Pixels,
}

impl IntoElement for MarkdownPreviewScrollbar {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MarkdownPreviewScrollbar {
    type RequestLayoutState = ();
    type PrepaintState = MarkdownPreviewScrollbarPrepaint;

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
        // Absolute overlay pinned to the right edge of the (relative)
        // scroll-div parent. `inset` 0 on top/right/bottom + an explicit
        // width gives a strip from top to bottom of the parent, of fixed
        // width. This is Zed's pattern (with `inset = Edges::default()`
        // + `size = relative(1.)`) but anchored to the right edge so we
        // don't paint a full-width overlay on top of the content.
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.inset = Edges {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: gpui::Length::Auto,
        };
        style.size.width = px(MD_PREVIEW_SCROLLBAR_THICKNESS).into();
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
        // Always emit a thumb so the scrollbar strip is visible. When
        // content fits the viewport the thumb fills the track (it can't
        // be dragged — the mouse handlers below short-circuit on
        // `max_y <= 0`); when content overflows we shrink the thumb
        // proportionally and place it according to scroll offset.
        if max_y <= px(0.) {
            let thumb = Bounds::new(
                point(bounds.left(), bounds.top()),
                size(bounds.size.width, bounds.size.height),
            );
            return MarkdownPreviewScrollbarPrepaint {
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
        let thumb_h = px(raw_thumb.max(MD_PREVIEW_SCROLLBAR_MIN_THUMB).min(viewport_h_f));
        let movable = (viewport_h - thumb_h).max(px(0.));
        // Handle offset is negative going down (0 = top, -max_y = bottom).
        let offset_y = -self.handle.offset().y;
        let ratio = (f32::from(offset_y) / f32::from(max_y).max(1.)).clamp(0., 1.);
        let thumb_top = bounds.top() + movable * ratio;
        let thumb = Bounds::new(point(bounds.left(), thumb_top), size(bounds.size.width, thumb_h));
        MarkdownPreviewScrollbarPrepaint {
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

        let dragging = self.workspace.read(cx).md_preview_scroll_drag.is_some();
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
                    w.md_preview_scroll_drag = Some(offset);
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
                let Some(grab) = w.md_preview_scroll_drag else { return };
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
                if w.md_preview_scroll_drag.is_some() {
                    w.md_preview_scroll_drag = None;
                    cx.notify();
                }
            });
        });
    }
}
