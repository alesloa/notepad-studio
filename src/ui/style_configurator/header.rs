// SPDX-License-Identifier: Apache-2.0
//
// Header row of the Style Configurator dialog: "Select theme:" label + a
// click-to-open dropdown listing every discovered theme.
//
// The dropdown is intentionally simple — a button toggles a panel rendered
// below the row, and clicking a theme name swaps the dialog's `DialogState`
// onto that theme. Visual treatment matches `shortcut_mapper::action_button`
// so the dialog family looks consistent.

use gpui::{
    AnyElement, CursorStyle, Entity, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;

use super::StyleConfiguratorDialog;

pub(super) fn render(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let label = div()
        .min_w(rems(96. / 16.))
        .text_color(rgb(theme.text))
        .child(SharedString::from("Select theme:"));

    let combo = combo_button(theme, dialog, entity);

    div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(10. / 16.))
        .h(rems(32. / 16.))
        .px(rems(2. / 16.))
        .child(label)
        .child(combo)
        .into_any_element()
}

/// Theme-combo dropdown panel as a standalone overlay. Hoisted out of the
/// header row so the dialog's root render can stack it AFTER the body,
/// footer, and any other content — element paint order is the only z-axis
/// GPUI offers, so the dropdown has to be the last sibling of the root to
/// land on top of everything.
///
/// Coordinates are computed against the root's padding-box (the dialog has
/// `.relative()` + `.p(8/16)`): the combo sits inside the header row at
/// `(left = root.padding + label.min_w + gap, top = root.padding + (header.h
/// - combo.h) / 2)`. The dropdown drops just below.
pub(super) fn theme_dropdown_overlay(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> Option<AnyElement> {
    if !dialog.theme_dropdown_open {
        return None;
    }
    // Transparent full-window scrim BELOW the panel — clicks on the scrim
    // close the dropdown without firing whatever underlying control they
    // would have hit. Standard popover dismiss pattern. The panel paints
    // AFTER the scrim, so clicks on the panel hit the panel rows first.
    let scrim_entity = entity.clone();
    let scrim = div()
        .id("style-cfg-theme-dropdown-scrim")
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_entity.update(cx, |this, cx| {
                this.theme_dropdown_open = false;
                cx.notify();
            });
        });
    let panel = dropdown_panel(theme, dialog, entity);
    Some(
        div()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .size_full()
            .child(scrim)
            .child(panel)
            .into_any_element(),
    )
}

fn combo_button(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let entity = entity.clone();
    let open = dialog.theme_dropdown_open;
    let bg = theme.button_bg;
    let hover_bg = theme.button_hover_bg;
    let border = theme.group_box_border;

    div()
        .id("style-cfg-theme-combo")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(rems(24. / 16.))
        .min_w(rems(220. / 16.))
        .px(rems(8. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_color(rgb(theme.text))
        .text_size(rems(11.5 / 16.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(rgb(hover_bg)))
        .child(SharedString::from(active_label_text(dialog)))
        .child(SharedString::from(if open { "^" } else { "v" }))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            entity.update(cx, |this, cx| {
                let next = !this.theme_dropdown_open;
                // Opening the theme dropdown closes every other popover so
                // the user never sees two stacked dropdowns. Same when
                // toggling closed (idempotent on the others).
                this.theme_dropdown_open = next;
                this.font_dropdown_open = false;
                this.font_size_dropdown_open = false;
                this.close_colour_picker();
                cx.notify();
            });
        })
        .into_any_element()
}

fn active_label_text(dialog: &StyleConfiguratorDialog) -> String {
    dialog
        .state
        .themes
        .get(dialog.state.active_theme_idx)
        .map(|t| t.display_name.clone())
        .unwrap_or_else(|| "(no themes)".to_string())
}

fn dropdown_panel(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    // Coords are root-relative (the panel is hoisted out of the header
    // row so it paints on top of the body / footer). Math:
    //   top  = root.p (8/16) + header.h (32/16) - 4/16 visual gap = 36/16
    //   left = root.p (8/16) + label.min_w (96/16) + gap (10/16)   = 114/16
    let mut panel = div()
        .id("style-cfg-theme-dropdown")
        .absolute()
        .top(rems(36. / 16.))
        .left(rems(114. / 16.))
        .min_w(rems(220. / 16.))
        .max_h(rems(200. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .flex()
        .flex_col()
        // Block clicks from reaching the dismiss-scrim sibling so picking
        // a row registers cleanly without the scrim's close-on-click
        // handler firing alongside.
        .occlude();

    let active_idx = dialog.state.active_theme_idx;
    for (i, entry) in dialog.state.themes.iter().enumerate() {
        let is_active = i == active_idx;
        let entity_click = entity.clone();
        let row_id = SharedString::from(format!("style-cfg-theme-row-{i}"));
        let row = div()
            .id(row_id)
            .px(rems(8. / 16.))
            .py(rems(4. / 16.))
            .text_color(rgb(theme.text))
            .text_size(rems(11.5 / 16.))
            .when(is_active, |s| s.bg(rgb(theme.button_pressed_bg)))
            .when(!is_active, |s| {
                s.cursor(CursorStyle::PointingHand)
                    .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
            })
            .child(SharedString::from(entry.display_name.clone()))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                entity_click.update(cx, |this, cx| {
                    this.state.request_theme_switch(i);
                    this.theme_dropdown_open = false;
                    // Switching themes closes every transient popover —
                    // the foreground/background swatches the picker was
                    // editing belong to the OLD theme's selected style;
                    // keeping the picker open after the swap would be
                    // confusing (the user is now editing a different
                    // style entirely). Same for the font/font-size
                    // dropdowns — their selected row reflects the
                    // previous theme.
                    this.font_dropdown_open = false;
                    this.font_size_dropdown_open = false;
                    this.close_colour_picker();
                    // If the request actually switched (clean state) the
                    // document mutated — push it into the preview so the
                    // editor flips colours immediately. If the request
                    // pended (dirty state), document is unchanged and
                    // refresh_preview is a cheap no-op. The dirty-prompt
                    // handlers will refresh again once the user picks
                    // Save / Discard.
                    this.refresh_preview(cx);
                });
            });
        panel = panel.child(row);
    }

    panel.into_any_element()
}
