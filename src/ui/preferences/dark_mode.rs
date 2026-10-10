// Dark Mode panel — mirrors NPP's `Dark Mode` panel structurally:
//   1. Top-left: ungrouped vertical Light / Dark / Follow Windows radios.
//   2. Below: `Tones` group box. Left column = 8 tone radios stacked
//      vertically (Black, Red, …, Customized). Right side = 4×3 grid of
//      12 color swatches, one per NPP customizable slot. A bottom-right
//      `Reset` button clears every customized-slot override.
//
// All controls are live:
//   - Light / Dark / Follow Windows write `Settings.theme_mode` and
//     propagate via `propagate_theme_to_all_workspaces`. FollowSystem
//     resolves through the window-appearance observer installed in
//     `app/window.rs` and `preferences/mod.rs`.
//   - Tone radios (Black..Olive, Customized) are LIVE only when the
//     active theme mode is `FollowSystem` (NPP-parity rule from
//     `theme::Theme::resolve` — plain Dark/Light force vanilla
//     rendering). Otherwise rendered greyed and inert.
//   - Color swatches are LIVE only when `mode == FollowSystem` AND
//     `palette == TonePalette::Customized`. Otherwise rendered greyed
//     and inert (still preview their resolved color so the user can
//     scan each tone's preset palette without switching to Customized).
//     Clicking a live swatch opens the `color_picker` popover.
//   - Reset clears every customized-slot override and switches the
//     active tone back to Black. NPP doesn't confirm; neither do we.

use gpui::{
    CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rems, rgb,
};

use crate::settings;
use crate::theme::{
    PaletteColors, SLOT_ACTIVE, SLOT_COUNT, SLOT_DARKER_TEXT, SLOT_DISABLED_TEXT, SLOT_EDGE,
    SLOT_EDGE_DISABLED, SLOT_EDGE_HIGHLIGHT, SLOT_ERROR, SLOT_LINK, SLOT_MAIN,
    SLOT_MENU_HOT_TRACK, SLOT_TEXT, SLOT_TOP, Theme, ThemeMode, TonePalette, palette_for_tone,
};

use super::PreferencesDialog;
use super::widgets::{
    npp_color_swatch, npp_radio, npp_radio_disabled, render_group_box,
};

const TONE_LABELS: &[(TonePalette, &str)] = &[
    (TonePalette::Black, "Black"),
    (TonePalette::Red, "Red"),
    (TonePalette::Green, "Green"),
    (TonePalette::Blue, "Blue"),
    (TonePalette::Purple, "Purple"),
    (TonePalette::Cyan, "Cyan"),
    (TonePalette::Olive, "Olive"),
    (TonePalette::Customized, "Customized"),
];

const SLOT_LABELS: [(usize, &str); SLOT_COUNT] = [
    (SLOT_TOP, "Top"),
    (SLOT_MENU_HOT_TRACK, "Menu hot track"),
    (SLOT_ACTIVE, "Active"),
    (SLOT_MAIN, "Main"),
    (SLOT_ERROR, "Error"),
    (SLOT_TEXT, "Text"),
    (SLOT_DARKER_TEXT, "Darker text"),
    (SLOT_DISABLED_TEXT, "Disabled text"),
    (SLOT_LINK, "Link"),
    (SLOT_EDGE, "Edge"),
    (SLOT_EDGE_HIGHLIGHT, "Edge highlight"),
    (SLOT_EDGE_DISABLED, "Edge disabled"),
];

pub(super) fn render(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let active_mode = snap.theme_mode;

    let mode_radios = div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .child(theme_radio(
            theme,
            weak,
            "prefs-dark-mode-light",
            "Light mode",
            active_mode == ThemeMode::Light,
            ThemeMode::Light,
        ))
        .child(theme_radio(
            theme,
            weak,
            "prefs-dark-mode-dark",
            "Dark mode",
            active_mode == ThemeMode::Dark,
            ThemeMode::Dark,
        ))
        .child(theme_radio(
            theme,
            weak,
            "prefs-dark-mode-follow",
            "Follow Windows",
            active_mode == ThemeMode::FollowSystem,
            ThemeMode::FollowSystem,
        ));

    body.child(mode_radios)
        .child(div().h(rems(12. / 16.)))
        .child(render_tones_group(theme, weak, &snap, active_mode))
        .into_any_element()
}

fn theme_radio(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    selected: bool,
    target: ThemeMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, selected, move |cx| {
        let mut next = settings::snapshot();
        next.theme_mode = target;
        settings::replace_and_save(next);
        settings::propagate_theme_to_all_workspaces(target, cx);
        let _ = weak.update(cx, |this, cx| {
            this.set_theme_mode(target);
            this.last_action = Some(format!("Theme: {}", target.label()));
            cx.notify();
        });
    })
}

fn render_tones_group(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    snap: &crate::settings::Settings,
    active_mode: ThemeMode,
) -> gpui::AnyElement {
    let active_tone = snap.tone_palette;
    let slot_array = snap.customized_slots_array();
    // NPP-parity: tones only meaningful under Follow Windows. Plain
    // Light / Dark force vanilla rendering (see `Theme::resolve`) so the
    // whole group is inert when those modes are active.
    let group_enabled = matches!(active_mode, ThemeMode::FollowSystem);
    // Customized slot swatches: live only on Customized tone (NPP's
    // override model only layers slots on top of the Customized base).
    let slots_enabled = group_enabled && matches!(active_tone, TonePalette::Customized);

    // The resolved palette the swatches preview. When the group is live
    // we read the user's selected tone; otherwise show the vanilla Black
    // tone so the user can still see slot defaults.
    let preview_palette = if group_enabled {
        let base = palette_for_tone(active_tone);
        if matches!(active_tone, TonePalette::Customized) {
            crate::theme::apply_customized_overrides(base, &slot_array)
        } else {
            base
        }
    } else {
        PaletteColors::BLACK
    };

    let tone_radios = div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .min_w(rems(140. / 16.))
        .children(TONE_LABELS.iter().map(|(tone, label)| {
            tone_radio_or_disabled(
                theme,
                weak,
                *tone,
                label,
                *tone == active_tone,
                group_enabled,
            )
        }));

    let slot_grid = div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .children((0..3).map(|row| {
            let start = row * 4;
            let end = (start + 4).min(SLOT_LABELS.len());
            div()
                .flex()
                .flex_row()
                .gap(rems(1.))
                .children(SLOT_LABELS[start..end].iter().map(|(slot, label)| {
                    let color = slot_color(*slot, &preview_palette);
                    div().flex_1().child(slot_swatch(
                        theme,
                        weak,
                        *slot,
                        label,
                        color,
                        slots_enabled,
                    ))
                }))
        }));

    let row = div()
        .flex()
        .flex_row()
        .gap(rems(20. / 16.))
        .child(tone_radios)
        .child(slot_grid);

    let caveat = if !group_enabled {
        Some(SharedString::from(
            "Tones apply only when Follow Windows is selected",
        ))
    } else if !slots_enabled {
        Some(SharedString::from(
            "Select Customized to override individual slot colors",
        ))
    } else {
        None
    };

    let footer = div()
        .flex()
        .flex_row()
        .justify_between()
        .items_center()
        .child(
            div()
                .flex_1()
                .text_size(rems(11. / 16.))
                .text_color(rgb(theme.disabled_text))
                .child(caveat.unwrap_or_default()),
        )
        .child(reset_button(theme, weak, group_enabled));

    render_group_box(
        theme,
        "Tones",
        vec![row.into_any_element(), footer.into_any_element()],
    )
    .into_any_element()
}

fn slot_color(slot: usize, palette: &PaletteColors) -> u32 {
    match slot {
        SLOT_TOP => palette.top,
        SLOT_MENU_HOT_TRACK => palette.menu_hot_track,
        SLOT_ACTIVE => palette.active,
        SLOT_MAIN => palette.main,
        SLOT_ERROR => palette.error,
        SLOT_TEXT => palette.text,
        SLOT_DARKER_TEXT => palette.darker_text,
        SLOT_DISABLED_TEXT => palette.disabled_text,
        SLOT_LINK => palette.link,
        SLOT_EDGE => palette.edge,
        SLOT_EDGE_HIGHLIGHT => palette.edge_highlight,
        SLOT_EDGE_DISABLED => palette.edge_disabled,
        _ => 0x808080,
    }
}

fn tone_radio_or_disabled(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    target: TonePalette,
    label: &'static str,
    selected: bool,
    enabled: bool,
) -> gpui::AnyElement {
    if enabled {
        tone_radio(theme, weak, target, label, selected)
    } else {
        npp_radio_disabled(theme, label, selected)
    }
}

fn tone_radio(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    target: TonePalette,
    label: &'static str,
    selected: bool,
) -> gpui::AnyElement {
    let id: &'static str = match target {
        TonePalette::Black => "prefs-tone-black",
        TonePalette::Red => "prefs-tone-red",
        TonePalette::Green => "prefs-tone-green",
        TonePalette::Blue => "prefs-tone-blue",
        TonePalette::Purple => "prefs-tone-purple",
        TonePalette::Cyan => "prefs-tone-cyan",
        TonePalette::Olive => "prefs-tone-olive",
        TonePalette::Customized => "prefs-tone-customized",
    };
    let weak = weak.clone();
    npp_radio(theme, id, label, selected, move |cx| {
        let mut next = settings::snapshot();
        next.tone_palette = target;
        settings::replace_and_save(next);
        // Re-render every workspace + this dialog so the new palette
        // resolves on the next paint.
        for handle in cx.windows() {
            if let Some(ws) = handle.downcast::<crate::workspace::Workspace>() {
                let _ = ws.update(cx, |_w, _, cx| cx.notify());
            }
        }
        let _ = weak.update(cx, |this, cx| {
            this.last_action = Some(format!("Tone: {}", target.label()));
            cx.notify();
        });
    })
}

fn slot_swatch(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    slot: usize,
    label: &'static str,
    color: u32,
    enabled: bool,
) -> gpui::AnyElement {
    let id: &'static str = match slot {
        SLOT_TOP => "prefs-slot-top",
        SLOT_MENU_HOT_TRACK => "prefs-slot-menu-hot-track",
        SLOT_ACTIVE => "prefs-slot-active",
        SLOT_MAIN => "prefs-slot-main",
        SLOT_ERROR => "prefs-slot-error",
        SLOT_TEXT => "prefs-slot-text",
        SLOT_DARKER_TEXT => "prefs-slot-darker-text",
        SLOT_DISABLED_TEXT => "prefs-slot-disabled-text",
        SLOT_LINK => "prefs-slot-link",
        SLOT_EDGE => "prefs-slot-edge",
        SLOT_EDGE_HIGHLIGHT => "prefs-slot-edge-highlight",
        SLOT_EDGE_DISABLED => "prefs-slot-edge-disabled",
        _ => "prefs-slot-unknown",
    };
    let weak = weak.clone();
    npp_color_swatch(theme, id, label, color, enabled, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            // Anchor the picker near the right side of the dialog body
            // so it doesn't clip off the right edge. Hard-coded fallback
            // because we don't measure the swatch bounds here.
            let (ax, ay) = super::color_picker::fallback_anchor();
            this.open_color_picker(slot, ax, ay, cx);
            // Echo the slot in the footer for confirmation.
            this.last_action = Some(format!("Pick color for: {}", label));
        });
    })
}

fn reset_button(
    theme: &Theme,
    weak: &gpui::WeakEntity<PreferencesDialog>,
    enabled: bool,
) -> gpui::AnyElement {
    // NPP-parity Reset split-button: main body keeps the default "reset
    // all" behaviour (clears slot overrides + tone → Black), and the
    // caret on the right opens a dropdown offering slot-only vs.
    // slot+tone variants.
    if !enabled {
        let border = rgb(theme.disabled_text);
        let text_color = rgb(theme.disabled_text);
        let main = div()
            .px(rems(10. / 16.))
            .py(rems(2. / 16.))
            .bg(rgb(theme.button_bg))
            .border_1()
            .border_color(border)
            .text_size(rems(11. / 16.))
            .text_color(text_color)
            .child(SharedString::from("Reset"));
        let caret = div()
            .px(rems(6. / 16.))
            .py(rems(2. / 16.))
            .bg(rgb(theme.button_bg))
            .border_t_1()
            .border_b_1()
            .border_r_1()
            .border_color(border)
            .text_size(rems(11. / 16.))
            .text_color(text_color)
            .child(SharedString::from("▾"));
        return div()
            .flex()
            .flex_row()
            .child(main)
            .child(caret)
            .into_any_element();
    }

    let border = rgb(theme.border);
    let text_color = rgb(theme.text);
    let weak_main = weak.clone();
    let main = div()
        .id("prefs-tones-reset")
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(border)
        .text_size(rems(11. / 16.))
        .text_color(text_color)
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from("Reset"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let mut next = settings::snapshot();
            next.tone_palette = TonePalette::Black;
            next.customized_slots = vec![None; SLOT_COUNT];
            settings::replace_and_save(next);
            for handle in cx.windows() {
                if let Some(ws) = handle.downcast::<crate::workspace::Workspace>() {
                    let _ = ws.update(cx, |_w, _, cx| cx.notify());
                }
            }
            let _ = weak_main.update(cx, |this, cx| {
                this.last_action = Some("Tones reset".to_string());
                cx.notify();
            });
        });

    let weak_caret = weak.clone();
    let caret = div()
        .id("prefs-tones-reset-caret")
        .px(rems(6. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_t_1()
        .border_b_1()
        .border_r_1()
        .border_color(border)
        .text_size(rems(11. / 16.))
        .text_color(text_color)
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from("▾"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_caret.update(cx, |this, cx| {
                if this.reset_dropdown_open() {
                    this.close_reset_dropdown(cx);
                } else {
                    // Anchor below the right-edge button group. Fixed
                    // offsets — measuring the actual button bounds from a
                    // click handler is non-trivial here and the dialog
                    // body is fixed-size enough that an approximate
                    // anchor lands well visually.
                    this.open_reset_dropdown(px(380.), px(360.), cx);
                }
            });
        });

    div()
        .flex()
        .flex_row()
        .child(main)
        .child(caret)
        .into_any_element()
}

