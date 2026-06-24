// Shared close-button click handler factory for the find-dialog tabs.
//
// Why this lives in its own module: the click handler can't synchronously
// invoke `close_find_dialog`. The mouse event runs inside the dialog window's
// own `update_window_id` slot — gpui has *taken* the window out of
// `App::windows` for the duration of the event dispatch. Calling
// `handle.update(cx, |_, window, _| window.remove_window())` from that
// context tries to take the same window slot a second time, which silently
// returns `Err` (the slot is `None`), so the dialog never closes. The user
// observes a dead Close button.
//
// Fix: `window.defer(cx, ...)` enqueues the close to run after the current
// event cycle, by which point the dialog window is back in its slot and
// `handle.update` can succeed. Same pattern the open path uses to dodge the
// matching double-lease-on-Workspace panic.

use gpui::{App, Entity, Window};

use crate::workspace::Workspace;

use super::close_find_dialog;

/// Build the click closure used by every "Close" button in the find-dialog
/// tabs. Returns `Fn(&mut Window, &mut App)` so it slots straight into
/// `dialog_action_button`'s handler parameter.
pub(super) fn close_button_handler(
    workspace: Entity<Workspace>,
) -> impl Fn(&mut Window, &mut App) + 'static {
    move |window, cx| {
        let ws = workspace.clone();
        // Defer past the current event cycle: the dialog window is currently
        // taken out of `App::windows` to dispatch this click, so a
        // synchronous `handle.update` inside `close_find_dialog` would
        // return `Err` and the close would silently no-op.
        window.defer(cx, move |window, cx| {
            ws.update(cx, |w, cx| close_find_dialog(w, window, cx));
        });
    }
}
