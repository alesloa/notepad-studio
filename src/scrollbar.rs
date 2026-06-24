// --------------------------------------------------------------------------------------------
// VerticalScrollbarElement + HorizontalScrollbarElement — paint a thumb on
// the right / bottom edges of the editor pane. Drag-to-scroll, click-track
// -to-page. Hide when content fits or (h-scroll only) when word wrap is on.

use gpui::{
    App, Bounds, DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hsla, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Style, Window,
    fill, point, px, relative, rgba, size,
};

use crate::workspace::Workspace;

pub(crate) const SCROLLBAR_THICKNESS: f32 = 10.;
const SCROLLBAR_MIN_THUMB: f32 = 30.;

pub(crate) struct VerticalScrollbarElement {
    pub(crate) workspace: Entity<Workspace>,
}

pub(crate) struct VerticalScrollbarPrepaint {
    /// Thumb bounds in window-space, or `None` when content fits the viewport
    /// and no scrollbar should render.
    thumb: Option<Bounds<Pixels>>,
    track_bounds: Bounds<Pixels>,
    max_scroll: Pixels,
    movable: Pixels,
}

impl IntoElement for VerticalScrollbarElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for VerticalScrollbarElement {
    type RequestLayoutState = ();
    type PrepaintState = VerticalScrollbarPrepaint;

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
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let workspace = self.workspace.read(cx);
        let max_scroll = workspace.max_scroll_top();
        if max_scroll <= px(0.) {
            return VerticalScrollbarPrepaint {
                thumb: None,
                track_bounds: bounds,
                max_scroll: px(0.),
                movable: px(0.),
            };
        }
        let line_height = workspace.viewport.line_height.max(px(1.));
        let total_lines = workspace.viewport.total_lines as f32;
        let total_content = line_height * total_lines + line_height; // matches max_scroll padding
        let viewport_h = bounds.size.height;
        let viewport_h_f = f32::from(viewport_h);
        let total_content_f = f32::from(total_content).max(1.);
        let raw_thumb = viewport_h_f * (viewport_h_f / total_content_f);
        let thumb_h = px(raw_thumb.max(SCROLLBAR_MIN_THUMB).min(viewport_h_f));
        let movable = (viewport_h - thumb_h).max(px(0.));
        let scroll_top = workspace.scroll_top();
        let ratio = if max_scroll > px(0.) {
            (scroll_top / max_scroll).clamp(0., 1.)
        } else {
            0.
        };
        let thumb_top = bounds.top() + movable * ratio;
        let thumb = Bounds::new(point(bounds.left(), thumb_top), size(bounds.size.width, thumb_h));
        VerticalScrollbarPrepaint {
            thumb: Some(thumb),
            track_bounds: bounds,
            max_scroll,
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
        _cx: &mut App,
    ) {
        let Some(thumb_bounds) = prepaint.thumb else {
            return;
        };

        let theme = self.workspace.read(_cx).theme();
        let dragging = self.workspace.read(_cx).v_scroll_drag.is_some();
        let color: Hsla = if dragging {
            rgba(theme.scrollbar_hover).into()
        } else {
            rgba(theme.scrollbar).into()
        };
        // Slightly inset thumb from track edges so it reads as a thumb not a fill.
        let inset = px(2.);
        let visible_thumb = Bounds::new(
            point(thumb_bounds.left() + inset, thumb_bounds.top()),
            size(
                (thumb_bounds.size.width - inset * 2.).max(px(1.)),
                thumb_bounds.size.height,
            ),
        );
        window.paint_quad(fill(visible_thumb, color));

        // Mouse-down on thumb → start drag. Mouse-down on track but not thumb
        // → page-step toward the click. Both wired here, on the prepaint
        // bounds.
        let track_bounds = prepaint.track_bounds;
        let max_scroll = prepaint.max_scroll;
        let movable = prepaint.movable;
        let thumb_h = thumb_bounds.size.height;

        let workspace_for_down = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !track_bounds.contains(&event.position) {
                return;
            }
            if thumb_bounds.contains(&event.position) {
                // Start dragging — capture offset within thumb.
                let offset = event.position.y - thumb_bounds.top();
                workspace_for_down.update(cx, |w, cx| {
                    w.v_scroll_drag = Some(offset);
                    cx.notify();
                });
            } else {
                // Page-step toward the click.
                let viewport_h = track_bounds.size.height;
                let direction = if event.position.y < thumb_bounds.top() {
                    -1.
                } else {
                    1.
                };
                workspace_for_down.update(cx, |w, cx| {
                    let new_top = w.scroll_top() + viewport_h * direction;
                    w.set_scroll_top(new_top, cx);
                });
            }
        });

        let workspace_for_move = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            workspace_for_move.update(cx, |w, cx| {
                let Some(grab_offset) = w.v_scroll_drag else { return };
                if movable <= px(0.) || max_scroll <= px(0.) {
                    return;
                }
                let target_top_y = event.position.y - grab_offset;
                let local = (target_top_y - track_bounds.top()).clamp(px(0.), movable);
                let ratio = (local / movable).clamp(0., 1.);
                let new_top = max_scroll * ratio;
                w.set_scroll_top(new_top, cx);
            });
        });

        let workspace_for_up = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            workspace_for_up.update(cx, |w, cx| {
                if w.v_scroll_drag.is_some() {
                    w.v_scroll_drag = None;
                    cx.notify();
                }
            });
        });

        // Suppress unused warning on `thumb_h` (kept for future ergonomic
        // tweaks like edge-snap thresholds).
        let _ = thumb_h;
    }
}

// --------------------------------------------------------------------------------------------
// HorizontalScrollbarElement — paints a thumb along the bottom edge of the
// editor pane when word wrap is OFF and at least one line is wider than the
// viewport. Drag the thumb to scroll horizontally; click the track outside
// the thumb to page-step. Mirrors the vertical scrollbar's interaction model
// so users who learn one know the other.

pub(crate) struct HorizontalScrollbarElement {
    pub(crate) workspace: Entity<Workspace>,
}

pub(crate) struct HorizontalScrollbarPrepaint {
    thumb: Option<Bounds<Pixels>>,
    track_bounds: Bounds<Pixels>,
    max_scroll: Pixels,
    movable: Pixels,
}

impl IntoElement for HorizontalScrollbarElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for HorizontalScrollbarElement {
    type RequestLayoutState = ();
    type PrepaintState = HorizontalScrollbarPrepaint;

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
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let workspace = self.workspace.read(cx);
        // Hide entirely when wrap is on — no horizontal extent under wrap.
        if workspace.word_wrap {
            return HorizontalScrollbarPrepaint {
                thumb: None,
                track_bounds: bounds,
                max_scroll: px(0.),
                movable: px(0.),
            };
        }
        let max_scroll = workspace.max_scroll_left();
        if max_scroll <= px(0.) {
            return HorizontalScrollbarPrepaint {
                thumb: None,
                track_bounds: bounds,
                max_scroll: px(0.),
                movable: px(0.),
            };
        }

        // Total content width = widest known line + the right-pad
        // `max_scroll_left` adds. Reconstruct it instead of plumbing a new
        // field so this stays self-contained.
        let viewport_w = workspace.viewport.bounds.size.width;
        let total_content = workspace.viewport.max_line_width + px(20.);
        let viewport_w_f = f32::from(viewport_w).max(1.);
        let total_content_f = f32::from(total_content).max(1.);
        let raw_thumb = viewport_w_f * (viewport_w_f / total_content_f);
        let thumb_w = px(raw_thumb.max(SCROLLBAR_MIN_THUMB).min(viewport_w_f));
        let movable = (bounds.size.width - thumb_w).max(px(0.));
        let scroll_left = workspace.scroll_left();
        let ratio = if max_scroll > px(0.) {
            (scroll_left / max_scroll).clamp(0., 1.)
        } else {
            0.
        };
        let thumb_left = bounds.left() + movable * ratio;
        let thumb = Bounds::new(point(thumb_left, bounds.top()), size(thumb_w, bounds.size.height));
        HorizontalScrollbarPrepaint {
            thumb: Some(thumb),
            track_bounds: bounds,
            max_scroll,
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
        _cx: &mut App,
    ) {
        let Some(thumb_bounds) = prepaint.thumb else {
            return;
        };

        let theme = self.workspace.read(_cx).theme();
        let dragging = self.workspace.read(_cx).h_scroll_drag.is_some();
        let color: Hsla = if dragging {
            rgba(theme.scrollbar_hover).into()
        } else {
            rgba(theme.scrollbar).into()
        };
        let inset = px(2.);
        let visible_thumb = Bounds::new(
            point(thumb_bounds.left(), thumb_bounds.top() + inset),
            size(
                thumb_bounds.size.width,
                (thumb_bounds.size.height - inset * 2.).max(px(1.)),
            ),
        );
        window.paint_quad(fill(visible_thumb, color));

        let track_bounds = prepaint.track_bounds;
        let max_scroll = prepaint.max_scroll;
        let movable = prepaint.movable;

        // Mouse-down: drag-thumb or page-step.
        let workspace_for_down = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !track_bounds.contains(&event.position) {
                return;
            }
            if thumb_bounds.contains(&event.position) {
                let offset = event.position.x - thumb_bounds.left();
                workspace_for_down.update(cx, |w, cx| {
                    w.h_scroll_drag = Some(offset);
                    cx.notify();
                });
            } else {
                let viewport_w = track_bounds.size.width;
                let direction = if event.position.x < thumb_bounds.left() {
                    -1.
                } else {
                    1.
                };
                workspace_for_down.update(cx, |w, cx| {
                    let new_left = w.scroll_left() + viewport_w * direction;
                    w.set_scroll_left(new_left, cx);
                });
            }
        });

        // Mouse-move while dragging: translate pointer x into scroll_left.
        let workspace_for_move = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            workspace_for_move.update(cx, |w, cx| {
                let Some(grab_offset) = w.h_scroll_drag else { return };
                if movable <= px(0.) || max_scroll <= px(0.) {
                    return;
                }
                let target_left_x = event.position.x - grab_offset;
                let local = (target_left_x - track_bounds.left()).clamp(px(0.), movable);
                let ratio = (local / movable).clamp(0., 1.);
                let new_left = max_scroll * ratio;
                w.set_scroll_left(new_left, cx);
            });
        });

        // Mouse-up: release the drag.
        let workspace_for_up = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            workspace_for_up.update(cx, |w, cx| {
                if w.h_scroll_drag.is_some() {
                    w.h_scroll_drag = None;
                    cx.notify();
                }
            });
        });
    }
}
