//! Phase 2g.K — modal that asks the user how to paste rich content into a
//! rich-doc body. Mirrors the visual + dismissal behavior of `about_dialog`
//! so the workspace's deferred-layer stack stays consistent.
//!
//! Three actions:
//!   • "Keep Formatting" — splice the pre-decoded blocks via
//!     `confirm_paste_format_formatted`
//!   • "Paste as Plain Text" — drop formatting, use plain-text path via
//!     `confirm_paste_format_plain`
//!   • "Cancel" / Esc / click-outside — `hide_paste_format_dialog`
//!
//! State lives on `Workspace::paste_format_pending`. The dialog is mounted
//! by `workspace/render.rs` when that field is `Some(_)`.

use gpui::{
    Entity, FontWeight, IntoElement, MouseButton, ParentElement, anchored, deferred, div,
    prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

const PANEL_WIDTH_PX: f32 = 460.;
const PANEL_HEIGHT_PX: f32 = 220.;
const BUTTON_HEIGHT_PX: f32 = 28.;
const BUTTON_MIN_WIDTH_PX: f32 = 130.;

pub(crate) fn paste_format_dialog(theme: &Theme, workspace: Entity<Workspace>) -> impl IntoElement {
    let scrim_ws = workspace.clone();
    let cancel_ws = workspace.clone();
    let plain_ws = workspace.clone();
    let keep_ws = workspace.clone();

    let scrim = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(gpui::hsla(0., 0., 0., 0.45))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_ws.update(cx, |w, cx| w.hide_paste_format_dialog(cx));
        });

    let panel = div()
        .w(px(PANEL_WIDTH_PX))
        .h(px(PANEL_HEIGHT_PX))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .font_family("Menlo")
        .text_size(rems(13. / 16.))
        .on_mouse_down(MouseButton::Left, |_e, _w, _cx| {})
        .flex()
        .flex_col()
        .child(panel_body(theme))
        .child(panel_footer(theme, cancel_ws, plain_ws, keep_ws));

    let centered = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(panel);

    deferred(
        anchored().child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(scrim)
                .child(centered),
        ),
    )
    .with_priority(2)
}

fn panel_body(theme: &Theme) -> impl IntoElement {
    div()
        .flex_1()
        .px(px(20.))
        .pt(px(20.))
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .text_color(rgb(theme.text))
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(rems(14. / 16.))
                .child("Paste with formatting?"),
        )
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .child(
                    "The clipboard contains formatted content from another app. \
                     Keep the original formatting (fonts, bold, links, lists) or \
                     paste as plain text and adopt this document's style?",
                ),
        )
}

fn panel_footer(
    theme: &Theme,
    cancel_ws: Entity<Workspace>,
    plain_ws: Entity<Workspace>,
    keep_ws: Entity<Workspace>,
) -> impl IntoElement {
    div()
        .h(px(BUTTON_HEIGHT_PX + 24.))
        .px(px(16.))
        .pb(px(12.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .child(button(
            theme,
            "Cancel",
            false,
            move |_e, _w, cx| {
                cancel_ws.update(cx, |w, cx| w.hide_paste_format_dialog(cx));
            },
        ))
        .child(button(
            theme,
            "Paste as Plain Text",
            false,
            move |_e, _w, cx| {
                plain_ws.update(cx, |w, cx| w.confirm_paste_format_plain(cx));
            },
        ))
        .child(button(
            theme,
            "Keep Formatting",
            true,
            move |_e, _w, cx| {
                keep_ws.update(cx, |w, cx| w.confirm_paste_format_formatted(cx));
            },
        ))
}

/// Render one of the three footer buttons. `is_default` paints the accent
/// fill used for the recommended action (Keep Formatting) so the dialog
/// communicates which choice the Enter key would trigger on a keyboard-
/// driven OS — even though we don't currently bind Enter to it.
fn button<F>(theme: &Theme, label: &'static str, is_default: bool, on_click: F) -> impl IntoElement
where
    F: Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    let (bg, fg, border) = if is_default {
        (theme.toggle_active, theme.bg, theme.toggle_active)
    } else {
        (theme.button_bg, theme.text, theme.border)
    };
    let hover_bg = if is_default { theme.toggle_active } else { theme.button_hover_bg };
    div()
        .min_w(px(BUTTON_MIN_WIDTH_PX))
        .h(px(BUTTON_HEIGHT_PX))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .border_1()
        .border_color(rgb(border))
        .id(label)
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(label)
}
