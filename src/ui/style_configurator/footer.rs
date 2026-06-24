// Style Configurator footer — Save & Close + Cancel buttons + inline
// save-error notice.
//
// NPP layout (PowerEditor/src/WinControls/Preference/wordstyleDlg.rc) is
// roughly: Save & Close (primary, disabled until edits) on the left of
// the right-aligned cluster, Cancel on its right. Notepad Studio drops the
// Windows-only Transparency control (UI spec §5.14).
//
// Phase 5: Save & Close calls `StyleConfiguratorDialog::save_active_theme`
// which writes the active theme's XML to disk, refreshes the theme list,
// persists `Settings.theme_path`, and closes the window on success. On
// failure the dialog stays open and the error is surfaced inline above
// the button row. Cancel restores the snapshot and closes.

use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, Entity, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, prelude::*, rems, rgb,
};

use crate::theme::Theme;

use super::StyleConfiguratorDialog;

pub(super) fn render<C>(
    theme: &Theme,
    _dirty: bool,
    last_error: Option<&str>,
    entity: &Entity<StyleConfiguratorDialog>,
    _cx: &mut gpui::Context<C>,
) -> AnyElement {
    // Apply: write the current document + persist Settings.theme_path,
    // then refresh every editor window WITHOUT closing the dialog. Lets
    // the user iterate on a theme — pick a colour, click Apply, see the
    // editor change behind the floating dialog, adjust, click Apply
    // again. Closes nothing on success or failure.
    let apply_entity = entity.clone();
    let apply_btn = action_button(
        theme,
        "Apply",
        /* enabled */ true,
        /* primary */ false,
        move |cx| {
            eprintln!("style-cfg: Apply clicked");
            let result = apply_entity.update(cx, |this, cx| {
                let r = this.save_active_theme();
                cx.notify();
                r
            });
            match result {
                Ok(_path) => super::notify_all_workspaces(cx),
                Err(msg) => {
                    apply_entity.update(cx, |this, cx| {
                        this.state.last_save_error = Some(msg);
                        cx.notify();
                    });
                }
            }
        },
    );

    // Save & Close: write + persist + refresh editors + close. Always
    // enabled — clicking with no edits is a deliberate "re-apply current
    // theme" gesture (cheap on disk, useful when the editor cache fell
    // out of sync or the user just wants to commit the current pick).
    let save_entity = entity.clone();
    let save_btn = action_button(
        theme,
        "Save & Close",
        /* enabled */ true,
        /* primary */ true,
        move |cx| {
            eprintln!("style-cfg: Save & Close clicked");
            let result = save_entity.update(cx, |this, _| this.save_active_theme());
            match result {
                Ok(path) => {
                    eprintln!("style-cfg: save ok ({path:?}); deferring close");
                    crate::style_model::active_theme::set_preview(None);
                    crate::style_model::active_theme::invalidate();
                    super::notify_all_workspaces(cx);
                    // `WindowHandle::update` re-enters `update_window_id`,
                    // which `take()`s the window out of `cx.windows`. Our
                    // mouse handler is already running inside that very
                    // take period — calling close_window directly here
                    // would `take()` an already-taken slot and fail with
                    // "window not found". Defer until the current update
                    // unwinds and the window is back in the map.
                    cx.defer(|cx| super::close_window(cx));
                }
                Err(msg) => {
                    eprintln!("style-cfg: save failed: {msg}");
                    save_entity.update(cx, |this, cx| {
                        this.state.last_save_error = Some(msg);
                        cx.notify();
                    });
                }
            }
        },
    );

    let cancel_entity = entity.clone();
    let cancel_btn = action_button(
        theme,
        "Cancel",
        /* enabled */ true,
        /* primary */ false,
        move |cx| {
            eprintln!("style-cfg: Cancel clicked");
            cancel_entity.update(cx, |this, _cx| {
                if this.state.dirty {
                    this.state.restore_from_snapshot();
                    this.sync_user_ext_from_state();
                }
            });
            crate::style_model::active_theme::set_preview(None);
            crate::style_model::active_theme::invalidate();
            super::notify_all_workspaces(cx);
            // See Save & Close handler for why this is deferred — the
            // window is `take()`n during our mouse-handler dispatch and
            // a re-entrant `handle.update` would fail with "window not
            // found".
            cx.defer(|cx| super::close_window(cx));
        },
    );

    let buttons = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(rems(8. / 16.))
        .h(rems(36. / 16.))
        .px(rems(2. / 16.))
        .child(apply_btn)
        .child(save_btn)
        .child(cancel_btn);

    if let Some(msg) = last_error {
        div()
            .flex()
            .flex_col()
            .gap(rems(4. / 16.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .h(rems(20. / 16.))
                    .px(rems(2. / 16.))
                    .text_size(rems(11. / 16.))
                    .text_color(rgb(theme.error))
                    .child(SharedString::from(format!("Save failed: {msg}"))),
            )
            .child(buttons)
            .into_any_element()
    } else {
        buttons.into_any_element()
    }
}

/// NPP-style button for the configurator footer. Mirrors shortcut_mapper's
/// `action_button` so the visual treatment matches across our dialog
/// family (square edges, primary border = checkbox-fill blue, neutral
/// border = group-box grey, disabled = desaturated).
fn action_button<F>(
    theme: &Theme,
    label: &'static str,
    enabled: bool,
    primary: bool,
    handler: F,
) -> AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let bg: u32 = theme.button_bg;
    let border: u32 = if primary && enabled {
        theme.checkbox_fill
    } else if enabled {
        theme.group_box_border
    } else {
        theme.disabled_text
    };
    let fg: u32 = if enabled {
        theme.text
    } else {
        theme.disabled_text
    };
    let cursor = if enabled {
        CursorStyle::PointingHand
    } else {
        CursorStyle::OperationNotAllowed
    };
    let hover_bg = theme.button_hover_bg;
    let pressed_bg = theme.button_pressed_bg;
    let handler = Rc::new(handler);

    let node = div()
        .id(SharedString::from(format!("sc_button::{label}")))
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
        .text_color(rgb(fg))
        .cursor(cursor)
        .child(label);

    if enabled {
        node.hover(move |s| s.bg(rgb(hover_bg)))
            .active(move |s| s.bg(rgb(pressed_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                (handler)(cx);
            })
            .into_any_element()
    } else {
        node.into_any_element()
    }
}
