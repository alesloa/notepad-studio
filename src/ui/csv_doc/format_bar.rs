// CSV format bar — secondary toolbar shown directly below the main
// toolbar when the active tab is a CSV in preview mode.

use gpui::{
    Corner, CursorStyle, Entity, IntoElement, MouseButton, ParentElement, Pixels, Point,
    SharedString, Styled, anchored, deferred, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

fn delimiter_label(delim: u8) -> &'static str {
    match delim {
        b',' => "Comma (,)",
        b';' => "Semicolon (;)",
        b'\t' => "Tab",
        b'|' => "Pipe (|)",
        _ => "Comma (,)",
    }
}

const DELIMITER_CHOICES: &[(u8, &str)] = &[
    (b',', "Comma (,)"),
    (b';', "Semicolon (;)"),
    (b'\t', "Tab"),
    (b'|', "Pipe (|)"),
];

pub(crate) fn csv_format_bar(
    theme: &Theme,
    workspace: Entity<Workspace>,
    delimiter: u8,
) -> impl IntoElement {
    let bg = theme.toolbar_bg;
    let hover_bg = theme.toolbar_hover_bg;
    let border = theme.border;
    let text = theme.toolbar_icon;

    let ws_delim = workspace.clone();
    let delim_trigger = div()
        .id("csv-delim-trigger")
        .px(px(10.))
        .h(px(22.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(2.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(text))
        .hover(|s| s.bg(rgb(hover_bg)))
        .child(SharedString::from(delimiter_label(delimiter)))
        .child(SharedString::from("▾"))
        .on_mouse_down(MouseButton::Left, move |e, _w, cx| {
            let pos = Point::new(e.position.x, e.position.y + px(4.));
            ws_delim.update(cx, |w, cx| {
                w.show_csv_delimiter_menu(pos, cx);
            });
        });

    let ws_add_row = workspace.clone();
    let add_row = div()
        .id("csv-add-row")
        .px(px(10.))
        .h(px(22.))
        .flex()
        .items_center()
        .rounded(px(2.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(text))
        .hover(|s| s.bg(rgb(hover_bg)))
        .child(SharedString::from("+ Row"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            ws_add_row.update(cx, |w, cx| {
                if !super::actions::commit_cell_edit(w, cx) { return; }
                let tab_id = w.tabs[w.active].session_id;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    let target_row = model
                        .selected_rows
                        .iter()
                        .copied()
                        .max()
                        .or_else(|| model.cell_range.as_ref().map(|r| r.max_row()))
                        .or_else(|| model.active_cell.map(|(r, _)| r));
                    match target_row {
                        Some(r) => model.insert_row_at(r, false),
                        None => model.add_row(),
                    }
                }
                super::actions::emit_to_source(w, cx);
            });
        });

    let ws_add_col = workspace;
    let add_col = div()
        .id("csv-add-col")
        .px(px(10.))
        .h(px(22.))
        .flex()
        .items_center()
        .rounded(px(2.))
        .cursor(CursorStyle::PointingHand)
        .text_color(rgb(text))
        .hover(|s| s.bg(rgb(hover_bg)))
        .child(SharedString::from("+ Column"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            ws_add_col.update(cx, |w, cx| {
                if !super::actions::commit_cell_edit(w, cx) { return; }
                let tab_id = w.tabs[w.active].session_id;
                if let Some(model) = w.csv_models.get_mut(&tab_id) {
                    let target_col = model
                        .selected_cols
                        .iter()
                        .copied()
                        .max()
                        .or_else(|| model.cell_range.as_ref().map(|r| r.max_col()))
                        .or_else(|| model.active_cell.map(|(_, c)| c));
                    match target_col {
                        Some(c) => model.insert_col_at(c, false),
                        None => model.add_column(),
                    }
                }
                super::actions::emit_to_source(w, cx);
            });
        });

    let separator = |color: u32| {
        div()
            .w(px(1.))
            .h(rems(1.))
            .mx(px(6.))
            .bg(rgb(color))
    };

    div()
        .h(rems(28. / 16.))
        .flex_shrink_0()
        .px(rems(8. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.))
        .overflow_hidden()
        .bg(rgb(bg))
        .border_b_1()
        .border_color(rgb(border))
        .text_size(rems(11.5 / 16.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .text_color(rgb(theme.disabled_text))
                .child(SharedString::from("Delimiter:")),
        )
        .child(delim_trigger)
        .child(separator(theme.toolbar_separator))
        .child(add_row)
        .child(add_col)
}

/// Anchored dropdown listing delimiter choices. Rendered from the
/// workspace alongside the context menus when
/// `workspace.csv_delimiter_menu` is `Some`. Backdrop dismisses on
/// outside click. Item click sets the delimiter on the active model
/// and forces a re-parse on the next render.
pub(crate) fn csv_delimiter_menu(
    theme: &Theme,
    position: Point<Pixels>,
    current: u8,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let dismiss_ws = workspace.clone();
    let dismiss_ws_right = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            dismiss_ws.update(cx, |w, cx| w.hide_csv_delimiter_menu(cx));
        })
        .on_mouse_down(MouseButton::Right, move |_e, _w, cx| {
            dismiss_ws_right.update(cx, |w, cx| w.hide_csv_delimiter_menu(cx));
        });

    let bg = theme.chrome;
    let border = theme.border;
    let text = theme.text;
    let hover = theme.toggle_active;

    let mut menu = div()
        .flex()
        .flex_col()
        .py(rems(4. / 16.))
        .min_w(rems(160. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(text));

    for &(delim, label) in DELIMITER_CHOICES {
        let is_current = delim == current;
        let row_ws = workspace.clone();
        let row = div()
            .flex()
            .items_center()
            .h(rems(22. / 16.))
            .px(rems(12. / 16.))
            .gap(px(8.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
            .child(SharedString::from(if is_current { "✓" } else { " " }))
            .child(SharedString::from(label))
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                row_ws.update(cx, |w, cx| {
                    w.hide_csv_delimiter_menu(cx);
                    if !super::actions::commit_cell_edit(w, cx) { return; }
                    let tab_id = w.tabs[w.active].session_id;
                    if let Some(model) = w.csv_models.get_mut(&tab_id) {
                        model.set_delimiter(delim);
                    }
                    super::actions::ensure_model(w, window, cx);
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
                        .position(position)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(4.))
                        .child(menu),
                ),
        ),
    )
    .with_priority(2)
}
