// SPDX-License-Identifier: Apache-2.0
//
// Inline confirmation overlay shown when the user tries to switch themes
// while the in-memory document has unsaved edits. Three buttons:
//
//   * Save           — write current edits, then proceed with switch.
//   * Discard        — restore from snapshot (lose edits), then switch.
//   * Cancel switch  — leave the dialog on the current theme; do nothing.
//
// The overlay is a centred card layered on top of the dialog body via
// `position: absolute`. Mirrors the visual treatment of the dialog's
// other surfaces — square edges, group-box border, NPP-faithful look.

use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, Entity, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;

use super::StyleConfiguratorDialog;

pub(super) fn render(
    theme: &Theme,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let scrim = div()
        .id("style-cfg-prompt-scrim")
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .bg(rgb(0x000000))
        .opacity(0.45);

    let card = div()
        .absolute()
        .top(rems(120. / 16.))
        .left(rems(180. / 16.))
        .min_w(rems(400. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .p(rems(12. / 16.))
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(
            div()
                .text_color(rgb(theme.text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(
                    "Unsaved changes will be lost. Save, discard, or cancel the switch?",
                )),
        )
        .child(buttons_row(theme, entity));

    div()
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .child(scrim)
        .child(card)
        .into_any_element()
}

fn buttons_row(
    theme: &Theme,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let save_entity = entity.clone();
    let save_btn = prompt_button(theme, "Save", true, move |cx| {
        // Save current edits, then complete the pending switch.
        let (result, pending) = save_entity.update(cx, |this, cx| {
            let result = this.save_active_theme();
            cx.notify();
            (result, this.state.take_pending_switch())
        });
        match result {
            Ok(_) => {
                super::notify_all_workspaces(cx);
                if let Some(idx) = pending {
                    save_entity.update(cx, |this, cx| {
                        this.state.switch_theme(idx);
                        this.sync_user_ext_from_state();
                        this.refresh_preview(cx);
                    });
                }
            }
            Err(msg) => {
                // Save failed — surface the error and re-arm the prompt
                // so the user can retry or pick Discard / Cancel switch.
                save_entity.update(cx, |this, cx| {
                    this.state.last_save_error = Some(msg);
                    if let Some(idx) = pending {
                        this.state.switch_pending = Some(idx);
                        this.state.prompt_visible = true;
                    }
                    cx.notify();
                });
            }
        }
    });

    let discard_entity = entity.clone();
    let discard_btn = prompt_button(theme, "Discard", false, move |cx| {
        discard_entity.update(cx, |this, cx| {
            this.state.restore_from_snapshot();
            if let Some(idx) = this.state.take_pending_switch() {
                this.state.switch_theme(idx);
            }
            this.sync_user_ext_from_state();
            this.refresh_preview(cx);
        });
    });

    let cancel_entity = entity.clone();
    let cancel_btn = prompt_button(theme, "Cancel switch", false, move |cx| {
        cancel_entity.update(cx, |this, cx| {
            this.state.cancel_pending_switch();
            cx.notify();
        });
    });

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(rems(8. / 16.))
        .child(save_btn)
        .child(discard_btn)
        .child(cancel_btn)
        .into_any_element()
}

fn prompt_button<F>(
    theme: &Theme,
    label: &'static str,
    primary: bool,
    handler: F,
) -> AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let bg = theme.button_bg;
    let border = if primary {
        theme.checkbox_fill
    } else {
        theme.group_box_border
    };
    let hover_bg = theme.button_hover_bg;
    let pressed_bg = theme.button_pressed_bg;
    let handler = Rc::new(handler);

    div()
        .id(SharedString::from(format!("sc_prompt_btn::{label}")))
        .flex()
        .items_center()
        .justify_center()
        .h(rems(24. / 16.))
        .min_w(rems(96. / 16.))
        .px(rems(10. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(11.5 / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(rgb(hover_bg)))
        .active(move |s| s.bg(rgb(pressed_bg)))
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            (handler)(cx);
        })
        .into_any_element()
}
