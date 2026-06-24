// MarkdownInputBridge — invisible absolute-fill overlay that bridges the
// OS text input subsystem to the workspace while the markdown preview
// pane is mounted. The source editor's `EditorElement::paint` is the
// usual host for `window.handle_input(...)`, but it doesn't run in
// preview mode. Without something here, typed characters / IME / paste
// have no input handler registered against the workspace's focus handle
// and silently drop — even when the handle is focused (which click
// handlers on every paragraph / heading line ensure).
//
// Why a custom Element instead of a div: `handle_input` must be called
// during the paint pass with the actual element bounds (used by the OS
// to position the IME composition rect). A div doesn't expose paint.
// The bridge owns no state and paints nothing — it exists purely to be
// a paint hook with bounds.

use gpui::{
    App, Bounds, Edges, Element, ElementId, ElementInputHandler, Entity, GlobalElementId,
    IntoElement, LayoutId, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Position, Style,
    Window, px, relative,
};

use crate::workspace::Workspace;

pub(crate) struct MarkdownInputBridge {
    pub(crate) workspace: Entity<Workspace>,
}

impl IntoElement for MarkdownInputBridge {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MarkdownInputBridge {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        // Absolute-positioned overlay filling the parent. The parent
        // (`markdown_preview_pane`'s outer wrapper) already has
        // `.relative()`, so `Position::Absolute` here resolves against
        // that. We don't paint anything visible, so opacity / z-order
        // are irrelevant.
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.inset = Edges {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        };
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.workspace.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.workspace.clone()),
            cx,
        );
        // Window-global mouse-up listener — terminates any in-flight
        // preview drag regardless of where the release lands. The
        // per-pane `.on_mouse_up` only fires when the release is inside
        // the pane's div bounds; if the user drags off the pane (into
        // padding, a sibling panel, or off the window entirely) the
        // drag flag would otherwise never clear and the next mouse
        // move inside any block would extend the selection. Capture
        // phase so we run before any child listeners that might stop
        // propagation.
        let ws = self.workspace.clone();
        window.on_mouse_event(
            move |event: &MouseUpEvent, phase, _window, cx| {
                if !phase.bubble() {
                    return;
                }
                if event.button != MouseButton::Left {
                    return;
                }
                ws.update(cx, |w, cx| {
                    if w.preview_is_dragging() {
                        w.preview_end_drag(cx);
                    }
                    // Drop the continuous-autoscroll timer too — even if the
                    // drag flag was already clear (e.g. preview_end_drag
                    // already ran via another path), the timer task must not
                    // outlive the gesture.
                    w.preview_stop_continuous_autoscroll(cx);
                });
            },
        );
        // Window-global mouse-move listener — keeps extending the drag
        // selection when the cursor passes through gaps between blocks
        // (per-block on_mouse_move only fires inside its own div bounds,
        // so dragging through the padding between two paragraphs would
        // otherwise freeze) or off the right margin / past the bottom
        // of the last block. Only runs when a drag is actually in
        // progress so it's a no-op for plain hover moves; bails early
        // when no Left button is held. Re-installed every paint, same
        // lifecycle as the mouse-up listener above.
        let ws = self.workspace.clone();
        window.on_mouse_event(
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if !phase.bubble() {
                    return;
                }
                if !event.dragging() {
                    return;
                }
                let position = event.position;
                ws.update(cx, |w, cx| {
                    if !w.preview_is_dragging() {
                        return;
                    }
                    // Auto-scroll the preview when the drag goes past the
                    // top or bottom of the viewport so the user can keep
                    // extending the selection into scrolled-off content.
                    // Done before the hit-test so the next paint reveals
                    // the freshly-scrolled blocks; the hit-test continues
                    // to extend selection against the current (about to
                    // be re-rendered) block layouts.
                    w.preview_edge_auto_scroll(position, cx);
                    w.preview_drag_to_pixel(position, true, false, cx);
                    // Kick (or refresh) the continuous-autoscroll timer.
                    // No-op when the cursor is inside the viewport or the
                    // task is already running; updates the captured
                    // last-drag-pos so the next tick sees the latest cursor.
                    w.preview_start_continuous_autoscroll(position, cx);
                });
            },
        );
    }
}
