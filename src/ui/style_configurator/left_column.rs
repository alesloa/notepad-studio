// SPDX-License-Identifier: Apache-2.0
//
// Left column of the Style Configurator dialog: Language listbox, Style
// listbox, and (for non-Global lexers) the Default-ext + User-ext rows.
//
// The Language listbox drives the cascade — clicking a row repopulates the
// Style listbox; clicking a Style row updates `selected_style_idx`. The
// User-ext row hosts a custom text input (`user_ext_input::UserExtInput`)
// so the user can edit the active lexer's `userExt` attribute live; the
// `.id() + .track_focus()` wrapping is required by the GPUI focus-handle
// caveat (multi-input dispatch-tree trap).

use gpui::{
    AnyElement, CursorStyle, Entity, FocusHandle, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, div, prelude::*, rems, rgb,
};

use crate::theme::Theme;

use super::StyleConfiguratorDialog;
use super::user_ext_input::UserExtInput;

pub(super) fn render(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let language_section = section(
        theme,
        "Language :",
        language_listbox(theme, dialog, entity),
    );
    let style_section = section(
        theme,
        "Style :",
        style_listbox(theme, dialog, entity),
    );

    let mut col = div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .min_w(rems(220. / 16.))
        .max_w(rems(280. / 16.))
        .h_full()
        .child(language_section)
        .child(style_section);

    // Default-ext / User-ext rows only show when a real lexer is selected.
    if dialog.state.current_lexer().is_some() {
        col = col.child(default_ext_row(theme, dialog));
        col = col.child(user_ext_row(theme, &dialog.user_ext_focus, entity));
    }

    col.into_any_element()
}

fn section(theme: &Theme, label: &'static str, body: AnyElement) -> AnyElement {
    let header = div()
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(label));

    div()
        .flex()
        .flex_col()
        .gap(rems(2. / 16.))
        .child(header)
        .child(body)
        .into_any_element()
}

fn language_listbox(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let names = dialog.state.lang_names();
    let active = dialog.state.selected_lang_idx;

    let mut list = div()
        .id("style-cfg-language-list")
        .flex()
        .flex_col()
        .h(rems(108. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border));

    for (i, name) in names.into_iter().enumerate() {
        let is_active = i == active;
        let entity_click = entity.clone();
        let row_id = SharedString::from(format!("style-cfg-lang-row-{i}"));
        let row = div()
            .id(row_id)
            .px(rems(6. / 16.))
            .py(rems(2. / 16.))
            .text_color(rgb(theme.text))
            .text_size(rems(11.5 / 16.))
            .when(is_active, |s| s.bg(rgb(theme.button_pressed_bg)))
            .when(!is_active, |s| {
                s.cursor(CursorStyle::PointingHand)
                    .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
            })
            .child(SharedString::from(name))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                entity_click.update(cx, |this, cx| {
                    this.state.switch_lang(i);
                    this.sync_user_ext_from_state();
                    cx.notify();
                });
            });
        list = list.child(row);
    }

    list.into_any_element()
}

fn style_listbox(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let names = dialog.state.style_names();
    let active = dialog.state.selected_style_idx;

    let mut list = div()
        .id("style-cfg-style-list")
        .flex()
        .flex_col()
        .h(rems(140. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border));

    if names.is_empty() {
        list = list.child(
            div()
                .px(rems(6. / 16.))
                .py(rems(4. / 16.))
                .text_color(rgb(theme.disabled_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from("(no styles)")),
        );
        return list.into_any_element();
    }

    for (i, name) in names.into_iter().enumerate() {
        let is_active = i == active;
        let entity_click = entity.clone();
        let row_id = SharedString::from(format!("style-cfg-style-row-{i}"));
        let row = div()
            .id(row_id)
            .px(rems(6. / 16.))
            .py(rems(2. / 16.))
            .text_color(rgb(theme.text))
            .text_size(rems(11.5 / 16.))
            .when(is_active, |s| s.bg(rgb(theme.button_pressed_bg)))
            .when(!is_active, |s| {
                s.cursor(CursorStyle::PointingHand)
                    .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
            })
            .child(SharedString::from(name))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                entity_click.update(cx, |this, cx| {
                    this.state.switch_style(i);
                    cx.notify();
                });
            });
        list = list.child(row);
    }

    list.into_any_element()
}

fn default_ext_row(theme: &Theme, dialog: &StyleConfiguratorDialog) -> AnyElement {
    let value = dialog
        .state
        .current_default_ext()
        .map(|s| s.to_string())
        .unwrap_or_default();

    let label = div()
        .min_w(rems(80. / 16.))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from("Default ext. :"));

    // Read-only; same look as the User-ext box but greyed text + no cursor.
    let value_box = div()
        .flex()
        .flex_1()
        .items_center()
        .h(rems(20. / 16.))
        .px(rems(6. / 16.))
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .text_color(rgb(theme.disabled_text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(value));

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(6. / 16.))
        .child(label)
        .child(value_box)
        .into_any_element()
}

/// User-ext editable row — live-edits the active lexer's `lexer_user_ext`
/// via the `UserExtInput` custom Element. Save & Close serialises the
/// value back into the theme XML's `userExt` attribute. The wrapper div
/// pairs `.id()` with `.track_focus()` per the GPUI focus-handle caveat so
/// the focus_id anchors to a dedicated dispatch-tree node (without that,
/// `handle_input` registration silently drops on the second-or-later
/// custom input in the dialog).
fn user_ext_row(
    theme: &Theme,
    focus: &FocusHandle,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let label = div()
        .min_w(rems(80. / 16.))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from("User ext. :"));

    let input_box = div()
        .id("style-cfg-user-ext")
        .track_focus(focus)
        .occlude()
        .flex()
        .flex_1()
        .items_center()
        .h(rems(20. / 16.))
        .px(rems(6. / 16.))
        .cursor(CursorStyle::IBeam)
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .child(UserExtInput {
            dialog: entity.downgrade(),
        });

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(6. / 16.))
        .child(label)
        .child(input_box)
        .into_any_element()
}
