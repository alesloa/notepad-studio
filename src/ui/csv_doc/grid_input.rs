// CSV preview does not mount the source editor's input element. Register
// the grid's own handle so its first text/IME input can open a cell editor.
use gpui::{AnyElement, ElementInputHandler, Entity, FocusHandle, IntoElement, canvas, prelude::*, px};
use crate::workspace::Workspace;

pub(crate) fn grid_input_bridge(workspace: Entity<Workspace>, focus: FocusHandle) -> AnyElement {
    canvas(|_, _, _| (), move |bounds, _, window, cx| {
        if window.is_window_active() {
            window.handle_input(&focus, ElementInputHandler::new(bounds, workspace), cx);
        }
    }).absolute().top(px(0.)).left(px(0.)).size_full().into_any_element()
}
