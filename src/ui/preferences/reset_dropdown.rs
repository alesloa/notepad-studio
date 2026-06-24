// Reset split-button dropdown for Dark Mode → Tones.
//
// Mirrors NPP's Dark Mode `Reset` split-button: a small caret beside the
// main button opens a menu offering "Reset slot overrides only" vs.
// "Reset all (slots + tone)". Main button click keeps the prior NPP
// behaviour (full reset) so existing muscle memory is preserved.
//
// State lives on `PreferencesDialog::reset_dropdown` as anchor pixels.
// Dismissal: clicking a row, clicking the backdrop, or clicking the
// caret again.

use gpui::{
    CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, SharedString,
    Styled, div, rems, rgb,
};

use crate::settings;
use crate::theme::{SLOT_COUNT, Theme, TonePalette};

use super::PreferencesDialog;

#[derive(Clone, Copy, Debug)]
pub(super) struct ResetDropdownState {
    pub(super) anchor_x: Pixels,
    pub(super) anchor_y: Pixels,
}

const MENU_WIDTH_REM: f32 = 220. / 16.;
const ROW_HEIGHT_REM: f32 = 24. / 16.;

pub(super) fn render(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    state: ResetDropdownState,
) -> gpui::AnyElement {
    let weak_for_backdrop = weak.clone();
    let backdrop = div()
        .id("prefs-reset-dropdown-backdrop")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_for_backdrop.update(cx, |this, cx| {
                this.close_reset_dropdown(cx);
            });
        });

    let menu = div()
        .absolute()
        .left(state.anchor_x)
        .top(state.anchor_y)
        .w(rems(MENU_WIDTH_REM))
        .bg(rgb(theme.chrome))
        .border_1()
        .border_color(rgb(theme.border))
        .flex()
        .flex_col()
        .child(menu_row(
            theme,
            weak,
            "prefs-reset-dropdown-slots",
            "Reset slot overrides only",
            reset_slots_only,
        ))
        .child(menu_row(
            theme,
            weak,
            "prefs-reset-dropdown-all",
            "Reset all (slots + tone)",
            reset_all,
        ));

    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(backdrop)
        .child(menu)
        .into_any_element()
}

fn menu_row(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    action: fn(&mut gpui::App),
) -> gpui::AnyElement {
    let weak = weak.clone();
    div()
        .id(id)
        .px(rems(10. / 16.))
        .h(rems(ROW_HEIGHT_REM))
        .flex()
        .items_center()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            action(cx);
            let _ = weak.update(cx, |this, cx| {
                this.close_reset_dropdown(cx);
                this.last_action = Some(label.to_string());
                cx.notify();
            });
        })
        .into_any_element()
}

fn reset_slots_only(cx: &mut gpui::App) {
    let mut next = settings::snapshot();
    next.customized_slots = vec![None; SLOT_COUNT];
    settings::replace_and_save(next);
    nudge_workspaces(cx);
}

fn reset_all(cx: &mut gpui::App) {
    let mut next = settings::snapshot();
    next.tone_palette = TonePalette::Black;
    next.customized_slots = vec![None; SLOT_COUNT];
    settings::replace_and_save(next);
    nudge_workspaces(cx);
}

fn nudge_workspaces(cx: &mut gpui::App) {
    for handle in cx.windows() {
        if let Some(ws) = handle.downcast::<crate::workspace::Workspace>() {
            let _ = ws.update(cx, |_w, _, cx| cx.notify());
        }
    }
}
