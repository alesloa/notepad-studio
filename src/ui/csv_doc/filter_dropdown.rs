// Excel-style AutoFilter dropdown.
//
// Mounts as a deferred anchored panel — same shape as `context_menu.rs`.
// The model owns the in-flight state (`open_filter_dropdown`); the
// workspace render path clones the relevant fields into a
// [`FilterDropdownSnapshot`] and hands them to `csv_filter_dropdown`
// before paint. Backdrop click = Cancel (drop staged edits); OK =
// commit staged edits to `col_filters`.
//
// Layout (~260 px wide, scrollable middle band):
//
//   ┌──────────────────────────────┐
//   │ Sort Ascending               │
//   │ Sort Descending              │
//   │ ─────────────────────────── │
//   │ [✓] (Select All)             │
//   │ [✓] value1                   │   ← scrollable
//   │ [ ] value2                   │
//   │ ...                          │
//   │ ─────────────────────────── │
//   │              [Cancel] [OK]   │
//   └──────────────────────────────┘

use std::collections::HashSet;

use gpui::{
    Corner, CursorStyle, Entity, IntoElement, MouseButton, ParentElement, Pixels, Point,
    SharedString, Styled, anchored, deferred, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

/// Clone of the open dropdown state, captured at the start of render so
/// the closures here don't need to re-borrow the workspace mid-paint.
/// Render path is the workspace; it reads
/// `model.open_filter_dropdown` and builds this before passing it down.
#[derive(Clone)]
pub(crate) struct FilterDropdownSnapshot {
    pub(crate) col: usize,
    pub(crate) anchor: Point<Pixels>,
    pub(crate) staged_hidden: HashSet<String>,
    pub(crate) unique_values: Vec<String>,
}

impl FilterDropdownSnapshot {
    pub(crate) fn from_model(model: &super::model::CsvModel) -> Option<Self> {
        let s = model.open_filter_dropdown.as_ref()?;
        Some(Self {
            col: s.col,
            anchor: s.anchor,
            staged_hidden: s.staged_hidden.clone(),
            unique_values: s.unique_values.clone(),
        })
    }
}

pub(crate) fn csv_filter_dropdown(
    theme: &Theme,
    snap: FilterDropdownSnapshot,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let bg = theme.chrome;
    let border = theme.border;
    let text = theme.text;
    let hover = theme.toggle_active;

    let cancel_workspace = workspace.clone();
    let cancel_workspace_right = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
            cancel_workspace.update(cx, |w, cx| {
                super::actions::cancel_filter_dropdown(w, cx);
            });
        })
        .on_mouse_down(MouseButton::Right, move |_e, _window, cx| {
            cancel_workspace_right.update(cx, |w, cx| {
                super::actions::cancel_filter_dropdown(w, cx);
            });
        });

    let col = snap.col;
    let ws_for_asc = workspace.clone();
    let ws_for_desc = workspace.clone();
    let ws_for_select_all = workspace.clone();
    let ws_for_ok = workspace.clone();
    let ws_for_cancel = workspace.clone();

    let unique_total = snap.unique_values.len();
    let staged_hidden_count = snap.staged_hidden.len();
    let checked_count = unique_total.saturating_sub(staged_hidden_count);
    let select_all_glyph: SharedString = if staged_hidden_count == 0 {
        SharedString::from("✓")
    } else if checked_count == 0 {
        SharedString::from(" ")
    } else {
        SharedString::from("−")
    };

    let sort_asc_row = div()
        .flex()
        .items_center()
        .h(rems(22. / 16.))
        .px(rems(12. / 16.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(text))
        .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
        .child(SharedString::from("Sort Ascending"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            ws_for_asc.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    model.sort_by_col(col, true);
                    model.cancel_filter_dropdown();
                }
                super::actions::emit_to_source(w, cx);
            });
        });

    let sort_desc_row = div()
        .flex()
        .items_center()
        .h(rems(22. / 16.))
        .px(rems(12. / 16.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(text))
        .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
        .child(SharedString::from("Sort Descending"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            ws_for_desc.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    model.sort_by_col(col, false);
                    model.cancel_filter_dropdown();
                }
                super::actions::emit_to_source(w, cx);
            });
        });

    let separator = || {
        div()
            .h(rems(1. / 16.))
            .my(rems(4. / 16.))
            .mx(rems(8. / 16.))
            .bg(rgb(border))
    };

    let select_all_row = div()
        .flex()
        .items_center()
        .h(rems(22. / 16.))
        .px(rems(12. / 16.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(text))
        .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .mr(px(6.))
                .border_1()
                .border_color(rgb(border))
                .flex()
                .items_center()
                .justify_center()
                .text_size(rems(12. / 16.))
                .child(select_all_glyph),
        )
        .child(SharedString::from("(Select All)"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            ws_for_select_all.update(cx, |w, cx| {
                let tab_id = w.tabs[w.active].session_id;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    if let Some(state) = model.open_filter_dropdown.as_mut() {
                        let fully_unchecked =
                            state.staged_hidden.len() == state.unique_values.len();
                        if fully_unchecked {
                            state.staged_hidden.clear();
                        } else {
                            state.staged_hidden =
                                state.unique_values.iter().cloned().collect();
                        }
                    }
                }
                cx.notify();
            });
        });

    let mut list = div()
        .id(SharedString::from(format!("csv-filter-list-{}", snap.col)))
        .flex()
        .flex_col()
        .max_h(rems(220. / 16.))
        .overflow_y_scroll();

    for value in &snap.unique_values {
        let is_hidden = snap.staged_hidden.contains(value);
        let checked = !is_hidden;
        let ws_for_row = workspace.clone();
        let value_for_handler = value.clone();
        let label: SharedString = if value.is_empty() {
            SharedString::from("(Blanks)")
        } else {
            SharedString::from(value.clone())
        };
        let glyph: SharedString = if checked {
            SharedString::from("✓")
        } else {
            SharedString::from(" ")
        };

        list = list.child(
            div()
                .flex()
                .items_center()
                .h(rems(22. / 16.))
                .px(rems(12. / 16.))
                .cursor(CursorStyle::PointingHand)
                .text_color(rgb(text))
                .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
                .child(
                    div()
                        .w(px(16.))
                        .h(px(16.))
                        .mr(px(6.))
                        .border_1()
                        .border_color(rgb(border))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(rems(12. / 16.))
                        .child(glyph),
                )
                .child(div().truncate().min_w_0().child(label))
                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                    let v = value_for_handler.clone();
                    ws_for_row.update(cx, |w, cx| {
                        let tab_id = w.tabs[w.active].session_id;
                        if let Some(model) = w.csv_models.get_mut(&tab_id) {
                            if let Some(state) = model.open_filter_dropdown.as_mut() {
                                if state.staged_hidden.contains(&v) {
                                    state.staged_hidden.remove(&v);
                                } else {
                                    state.staged_hidden.insert(v);
                                }
                            }
                        }
                        cx.notify();
                    });
                }),
        );
    }

    let footer_button = |label: &'static str, accent: bool, theme: &Theme| {
        let bg = if accent { theme.toggle_active } else { theme.chrome };
        let fg = if accent { theme.bg } else { theme.chrome_text };
        div()
            .px(rems(10. / 16.))
            .py(rems(3. / 16.))
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .border_1()
            .border_color(rgb(theme.border))
            .text_size(rems(11.5 / 16.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme.toggle_active)).text_color(rgb(theme.bg)))
            .child(SharedString::from(label))
    };

    let cancel_btn = footer_button("Cancel", false, theme).on_mouse_down(
        MouseButton::Left,
        move |_e, _w, cx| {
            ws_for_cancel.update(cx, |w, cx| {
                super::actions::cancel_filter_dropdown(w, cx);
            });
        },
    );
    let ok_btn = footer_button("OK", true, theme).on_mouse_down(
        MouseButton::Left,
        move |_e, _w, cx| {
            ws_for_ok.update(cx, |w, cx| {
                super::actions::apply_filter_dropdown(w, cx);
            });
        },
    );

    let footer = div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(px(6.))
        .px(rems(8. / 16.))
        .py(rems(4. / 16.))
        .child(cancel_btn)
        .child(ok_btn);

    let panel = div()
        .flex()
        .flex_col()
        .min_w(rems(240. / 16.))
        .max_w(rems(320. / 16.))
        .py(rems(4. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(text))
        .child(sort_asc_row)
        .child(sort_desc_row)
        .child(separator())
        .child(select_all_row)
        .child(list)
        .child(separator())
        .child(footer);

    deferred(
        div().absolute().top_0().left_0().size_full().child(
            div()
                .size_full()
                .child(backdrop)
                .child(
                    anchored()
                        .position(snap.anchor)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(4.))
                        .child(panel),
                ),
        ),
    )
    .with_priority(2)
}
