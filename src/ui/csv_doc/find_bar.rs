// Inline CSV find bar — rendered just above the status bar at the
// bottom of the preview pane when `model.find_bar_open` is true.
//
// Picked Option B (a CSV-specific inline bar) over extending the
// existing multi-tab find_dialog. Reasons: the find_dialog is wired
// to rope text via 7 tab files and a workspace-scoped state; adding a
// CsvGrid scope means threading a new variant through all of them.
// An inline bar reuses the CSV model's `find_input` / `find_query`
// state and the parameterised `CsvCellInputElement` (see
// `cell_input.rs`), so the only new surface is layout + buttons.
//
// Cmd+H expands the bar with a second row containing the replace
// input + Replace / Replace All buttons. The two rows share the same
// chrome styling; the bar's height doubles when replace mode is up.

use gpui::{
    CursorStyle, Entity, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

use super::cell_input::{find_input_element, replace_input_element};
use super::grid::CsvGridSnapshot;

/// Pixel height of one row inside the find bar (find row OR replace
/// row). The bar's outer height is one row in find-only mode, two
/// rows when find+replace is active.
pub(crate) const FIND_BAR_HEIGHT_PX: f32 = 28.0;

/// Render the find bar. Returns an empty element when the bar is
/// closed so the caller can mount unconditionally and the bar appears
/// / disappears on `model.find_bar_open`.
pub(crate) fn csv_find_bar(
    theme: &Theme,
    snap: &CsvGridSnapshot,
    workspace: Entity<Workspace>,
) -> Option<gpui::AnyElement> {
    if !snap.find_bar_open {
        return None;
    }
    let handle = snap.find_input_handle.as_ref()?;

    let total = snap.find_match_count;
    let current = snap.find_current_match.map(|i| i + 1).unwrap_or(0);
    let counter: SharedString = if let Some(summary) = snap.replace_summary.as_ref() {
        SharedString::from(summary.clone())
    } else if total == 0 && !snap.find_query.is_empty() {
        SharedString::from("No matches")
    } else if total == 0 {
        SharedString::from("")
    } else {
        SharedString::from(format!("{current} of {total}"))
    };

    let ws_for_prev = workspace.clone();
    let ws_for_next = workspace.clone();
    let ws_for_case = workspace.clone();
    let ws_for_whole = workspace.clone();
    let ws_for_close = workspace.clone();

    let case_active = snap.find_case_sensitive;
    let whole_active = snap.find_whole_cell;

    // Stable wrapper id — without it `track_focus` stamps the same
    // dispatch node every frame and typing into the input goes dead
    // (a GPUI focus-handle caveat).
    let input_box = div()
        .id("csv-find-input")
        .track_focus(handle)
        .cursor(CursorStyle::IBeam)
        .flex_grow()
        .h_full()
        .min_w(rems(120. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .px(px(6.))
        .child(find_input_element(
            theme,
            workspace.downgrade(),
            handle.clone(),
        ));

    let button = |label: &'static str, active: bool, theme: &Theme| {
        let bg = if active { theme.toggle_active } else { theme.chrome };
        let txt = if active { theme.bg } else { theme.chrome_text };
        div()
            .h_full()
            .px(rems(8. / 16.))
            .flex()
            .items_center()
            .bg(rgb(bg))
            .text_color(rgb(txt))
            .border_1()
            .border_color(rgb(theme.border))
            .text_size(rems(11. / 16.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme.toggle_active)).text_color(rgb(theme.bg)))
            .child(SharedString::from(label))
    };

    let find_row = div()
        .w_full()
        .h(px(FIND_BAR_HEIGHT_PX))
        .flex()
        .items_center()
        .px(px(6.))
        .gap(px(4.))
        .child(input_box)
        .child(
            div()
                .min_w(rems(64. / 16.))
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(counter),
        )
        .child(
            button("Prev", false, theme).on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                ws_for_prev.update(cx, |w, cx| {
                    super::actions::find_step(w, false, window, cx);
                });
            }),
        )
        .child(
            button("Next", false, theme).on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                ws_for_next.update(cx, |w, cx| {
                    super::actions::find_step(w, true, window, cx);
                });
            }),
        )
        .child(
            button("Aa", case_active, theme).on_mouse_down(
                MouseButton::Left,
                move |_e, _window, cx| {
                    ws_for_case.update(cx, |w, cx| {
                        let tab_id = w.tabs[w.active].session_id;
                        if let Some(model) = w.csv_models.get_mut(&tab_id) {
                            model.find_case_sensitive = !model.find_case_sensitive;
                            let q = model.find_query.clone();
                            if !q.is_empty() {
                                model.update_find_query(q);
                            }
                            cx.notify();
                        }
                    });
                },
            ),
        )
        .child(
            button("Whole", whole_active, theme).on_mouse_down(
                MouseButton::Left,
                move |_e, _window, cx| {
                    ws_for_whole.update(cx, |w, cx| {
                        let tab_id = w.tabs[w.active].session_id;
                        if let Some(model) = w.csv_models.get_mut(&tab_id) {
                            model.find_whole_cell = !model.find_whole_cell;
                            let q = model.find_query.clone();
                            if !q.is_empty() {
                                model.update_find_query(q);
                            }
                            cx.notify();
                        }
                    });
                },
            ),
        )
        .child(
            button("✕", false, theme).on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                ws_for_close.update(cx, |w, cx| {
                    super::actions::close_find_bar(w, window, cx);
                });
            }),
        );

    let replace_row = if snap.find_show_replace {
        let replace_handle = snap.replace_input_handle.clone();
        // Need a handle to render the input; if it isn't allocated
        // yet, skip the row — open_replace_bar / focus_replace_input
        // allocate before rendering.
        if let Some(rh) = replace_handle {
            let ws_for_repl = workspace.clone();
            let ws_for_repl_all = workspace.clone();
            let replace_input = div()
                .id("csv-replace-input")
                .track_focus(&rh)
                .cursor(CursorStyle::IBeam)
                .flex_grow()
                .h_full()
                .min_w(rems(120. / 16.))
                .bg(rgb(theme.bg))
                .border_1()
                .border_color(rgb(theme.border))
                .px(px(6.))
                .child(replace_input_element(
                    theme,
                    workspace.downgrade(),
                    rh.clone(),
                ));
            let row = div()
                .w_full()
                .h(px(FIND_BAR_HEIGHT_PX))
                .flex()
                .items_center()
                .px(px(6.))
                .gap(px(4.))
                .border_t_1()
                .border_color(rgb(theme.border))
                .child(replace_input)
                .child(
                    button("Replace", false, theme).on_mouse_down(
                        MouseButton::Left,
                        move |_e, _window, cx| {
                            ws_for_repl.update(cx, |w, cx| {
                                super::actions::replace_current_action(w, cx);
                            });
                        },
                    ),
                )
                .child(
                    button("Replace All", false, theme).on_mouse_down(
                        MouseButton::Left,
                        move |_e, _window, cx| {
                            ws_for_repl_all.update(cx, |w, cx| {
                                super::actions::replace_all_action(w, cx);
                            });
                        },
                    ),
                );
            Some(row)
        } else {
            None
        }
    } else {
        None
    };

    let mut bar = div()
        .w_full()
        .flex()
        .flex_col()
        .bg(rgb(theme.chrome))
        .border_t_1()
        .border_color(rgb(theme.border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.text))
        .child(find_row);
    if let Some(row) = replace_row {
        bar = bar.child(row);
    }

    Some(bar.into_any_element())
}
