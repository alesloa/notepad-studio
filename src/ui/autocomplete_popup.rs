// Auto-Completion popup. Anchored to a window-space point (caret position +
// one line), shows a vertical list of word candidates. Click-to-insert, with
// the currently-selected row tinted by `theme.toggle_active`. Keyboard
// navigation (Up/Down/Tab/Enter/Esc) is driven by the workspace handlers
// gated on `Workspace::autocomplete.is_some()` — this widget only paints the
// list and dispatches click events.

use gpui::{
    Corner, CursorStyle, Entity, IntoElement, MouseButton, Pixels, Point, anchored, deferred, div,
    prelude::*, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

/// Pixel width of the popup body. Tracks the average longest candidate at
/// Menlo 12pt — 240 px holds a ~28-char identifier without horizontal clip;
/// any longer candidate is truncated by GPUI's text shaper at paint time
/// rather than expanding the popup beyond a comfortable visual width.
const POPUP_WIDTH: f32 = 240.0;

/// Max rows rendered at once. NPP's popup also caps at ~8 — anything taller
/// dominates the editor pane. Scrollability isn't implemented yet; if the
/// candidate list exceeds this, the keyboard nav still walks the full set,
/// just only the rows around the selected index are visible.
const MAX_VISIBLE_ROWS: usize = 8;

const ROW_HEIGHT_REM: f32 = 22.0 / 16.0;

pub(crate) fn autocomplete_popup(
    theme: &Theme,
    anchor: Point<Pixels>,
    candidates: &[String],
    selected: usize,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let dismiss_workspace = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
            dismiss_workspace.update(cx, |w, cx| w.close_autocomplete(cx));
        })
        .on_mouse_down(MouseButton::Right, {
            let dismiss_workspace = workspace.clone();
            move |_e, _window, cx| {
                dismiss_workspace.update(cx, |w, cx| w.close_autocomplete(cx));
            }
        });

    let bg = theme.chrome;
    let border = theme.border;
    let text = theme.text;
    let hover = theme.toggle_active;

    let (visible_start, visible_end) = visible_window(candidates.len(), selected);

    let mut menu = div()
        .flex()
        .flex_col()
        .py(rems(2. / 16.))
        .w(gpui::px(POPUP_WIDTH))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(text));

    for (idx, candidate) in candidates
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_end.saturating_sub(visible_start))
    {
        let is_selected = idx == selected;
        let row_text_color = if is_selected { theme.bg } else { text };
        let row_workspace = workspace.clone();
        let pick = candidate.clone();
        let mut row = div()
            .flex()
            .items_center()
            .h(rems(ROW_HEIGHT_REM))
            .px(rems(10. / 16.))
            .text_color(rgb(row_text_color))
            .cursor(CursorStyle::PointingHand)
            .child(candidate.clone());
        if is_selected {
            row = row.bg(rgb(hover));
        } else {
            row = row.hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)));
        }
        row = row.on_mouse_down(MouseButton::Left, move |_e, window, cx| {
            let chosen = pick.clone();
            row_workspace.update(cx, |w, cx| {
                w.accept_autocomplete_with(&chosen, window, cx);
            });
        });
        menu = menu.child(row);
    }

    deferred(
        div().absolute().top_0().left_0().size_full().child(
            div()
                .size_full()
                .child(backdrop)
                .child(
                    anchored()
                        .position(anchor)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(4.))
                        .child(menu),
                ),
        ),
    )
    .with_priority(2)
}

/// Rows `[start, end)` of a `len`-row list to paint so `selected` stays in
/// view: the first `MAX_VISIBLE_ROWS`, bottom-anchored on `selected` once it
/// walks past them.
fn visible_window(len: usize, selected: usize) -> (usize, usize) {
    let selected = selected.min(len.saturating_sub(1));
    let visible_start = if len <= MAX_VISIBLE_ROWS {
        0
    } else {
        (selected + 1).saturating_sub(MAX_VISIBLE_ROWS)
    };
    let visible_end = visible_start.saturating_add(MAX_VISIBLE_ROWS).min(len);
    (visible_start, visible_end)
}

#[cfg(test)]
mod tests {
    use super::{MAX_VISIBLE_ROWS, visible_window};

    #[test]
    fn window_follows_selection() {
        assert_eq!(visible_window(0, 0), (0, 0));
        assert_eq!(visible_window(3, 2), (0, 3));
        assert_eq!(visible_window(50, 0), (0, MAX_VISIBLE_ROWS));
        assert_eq!(visible_window(50, 7), (0, 8));
        assert_eq!(visible_window(50, 8), (1, 9));
        assert_eq!(visible_window(50, 49), (42, 50));
    }

    #[test]
    fn selection_past_the_list_stays_in_range() {
        // A selected index past the list used to put start past end, and
        // `end - start` underflowed.
        for (len, selected) in [(50, 60), (9, 100), (9, usize::MAX), (0, usize::MAX)] {
            let (start, end) = visible_window(len, selected);
            assert!(start <= end && end <= len, "{len} {selected} -> {start}..{end}");
        }
    }
}