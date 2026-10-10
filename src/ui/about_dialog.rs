// Help ▸ About modal. Centered ~420×280 panel over a dimmed scrim. Reuses
// the same `deferred(...).with_priority(2)` overlay strategy as the tab
// right-click menu so it stacks cleanly above every other workspace surface.
//
// Esc dismissal is handled in `Workspace::close_find` (the same chain that
// closes context menus and inline bars). Click-outside dismisses via the
// scrim's `on_mouse_down`. The OK button is the only in-panel dismiss.

use gpui::{
    Entity, FontWeight, IntoElement, MouseButton, ParentElement, anchored, deferred, div,
    prelude::*, px, rems, rgb,
};

use crate::consts::{APP_NAME, VERSION};
use crate::theme::Theme;
use crate::workspace::Workspace;

const PANEL_WIDTH_PX: f32 = 420.;
const PANEL_HEIGHT_PX: f32 = 280.;

const TAGLINE: &str = "A lightweight cross-platform text editor inspired by Notepad++";
const CREDIT: &str = "Built with gpui by Zed Industries";
const LICENSE: &str = "Apache-2.0";
const REPO: &str = "github.com/alesloa/notepad-studio";

pub(crate) fn about_dialog(theme: &Theme, workspace: Entity<Workspace>) -> impl IntoElement {
    let scrim_workspace = workspace.clone();
    let ok_workspace = workspace.clone();

    // Scrim covers the entire window with a dim wash; clicks anywhere
    // outside the inner panel close the dialog.
    let scrim = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(gpui::hsla(0., 0., 0., 0.45))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_workspace.update(cx, |w, cx| w.hide_about_dialog(cx));
        });

    // Centered panel — anchored to the window center via `anchored`.
    let panel = div()
        .w(px(PANEL_WIDTH_PX))
        .h(px(PANEL_HEIGHT_PX))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .font_family(crate::fonts::mono())
        .text_size(rems(13. / 16.))
        // Stop click propagation so a click inside the panel doesn't reach
        // the scrim's dismiss handler.
        .on_mouse_down(MouseButton::Left, |_e, _w, _cx| {})
        .flex()
        .flex_col()
        .child(panel_body(theme))
        .child(panel_footer(theme, ok_workspace));

    let centered = div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().child(panel);

    // `deferred(...).with_priority(2)` matches the tab context menu — paints
    // after every other layer so the modal floats above the toolbar / tab
    // strip / find dialog overlay.
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
        .flex_grow()
        .flex()
        .flex_col()
        .px(px(28.))
        .pt(px(28.))
        .pb(px(12.))
        .gap(px(10.))
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme.text))
                .child(APP_NAME),
        )
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .child(format!("Version {VERSION}")),
        )
        .child(div().h(px(6.)))
        .child(div().text_color(rgb(theme.text)).child(TAGLINE))
        .child(div().h(px(4.)))
        .child(div().text_color(rgb(theme.chrome_text)).child(CREDIT))
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .child(format!("License: {LICENSE}")),
        )
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .child(format!("Repo: {REPO}")),
        )
}

fn panel_footer(theme: &Theme, workspace: Entity<Workspace>) -> impl IntoElement {
    let ok = div()
        .id("about-ok")
        .px(px(20.))
        .py(px(4.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .text_size(rems(13. / 16.))
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child("OK")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            workspace.update(cx, |w, cx| w.hide_about_dialog(cx));
        });

    div()
        .h(px(48.))
        .border_t_1()
        .border_color(rgb(theme.border))
        .flex()
        .items_center()
        .justify_end()
        .px(px(20.))
        .child(ok)
}
