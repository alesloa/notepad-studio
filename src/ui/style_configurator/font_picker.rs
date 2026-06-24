// SPDX-License-Identifier: Apache-2.0
//
// Font name + Font size combo helpers for the Style Configurator's right
// column.
//
// Font names: pulled from `cx.text_system().all_font_names()` (GPUI's
// platform-native enumerator — CoreText on macOS, fontconfig on Linux,
// DirectWrite on Windows). Cached on the dialog after first paint so
// subsequent dropdown opens are zero-cost. No hard-coded font lists —
// the platform enumerator is the source of truth (no mock data).
//
// Font size: a fixed dropdown of NPP's standard size choices. Empty
// string at the top means "inherit from Default Style" (`None` in
// `Style.font_size`).

use gpui::{
    AnyElement, App, CursorStyle, Entity, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;

use super::StyleConfiguratorDialog;

/// NPP's `fontSizeStrs` list — empty entry = "inherit Default Style".
pub(super) const FONT_SIZES: &[&str] = &[
    "", "5", "6", "7", "8", "9", "10", "11", "12", "14", "16", "18", "20", "22", "24", "26", "28",
];

/// Ensure the dialog's `font_names` cache is populated. Idempotent.
pub(super) fn ensure_font_cache(dialog: &mut StyleConfiguratorDialog, cx: &App) {
    if !dialog.font_names.is_empty() {
        return;
    }
    let mut names = cx.text_system().all_font_names();
    // Drop any starting with `.` (Apple-private font families surface as
    // `.SF NS` etc. — unhelpful for end users picking a coding font).
    names.retain(|n| !n.starts_with('.'));
    names.sort();
    names.dedup();
    if names.is_empty() {
        // text_system() must always return at least the loaded fallback
        // stack; if it didn't, surface the symptom rather than silently
        // shipping an empty combo.
        eprintln!("style_configurator: font enumeration returned no families");
    }
    dialog.font_names = names;
}

/// Render the Font name combo button. Click toggles
/// `dialog.font_dropdown_open`; the popover panel is rendered separately
/// by `font_dropdown_panel` so the button can stay inside the Font Style
/// group while the panel floats above siblings.
pub(super) fn name_combo_button(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
    enabled: bool,
) -> AnyElement {
    let label = dialog
        .state
        .current_style()
        .and_then(|s| s.font_name.clone())
        .unwrap_or_default();
    let display = if label.is_empty() {
        "(inherit)".to_string()
    } else {
        label
    };
    combo_button(
        theme,
        "style-cfg-font-name",
        display,
        dialog.font_dropdown_open,
        enabled,
        {
            let entity = entity.clone();
            move |cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.font_dropdown_open = !this.font_dropdown_open;
                    if this.font_dropdown_open {
                        // Open one, close every other popover so the
                        // user only ever sees one floating panel at a
                        // time. Mirrors the theme dropdown handler.
                        this.font_size_dropdown_open = false;
                        this.theme_dropdown_open = false;
                        this.close_colour_picker();
                        ensure_font_cache(this, cx);
                    }
                    cx.notify();
                });
            }
        },
    )
}

/// Render the Font size combo button.
pub(super) fn size_combo_button(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
    enabled: bool,
) -> AnyElement {
    let label = dialog
        .state
        .current_style()
        .and_then(|s| s.font_size)
        .map(|n| n.to_string())
        .unwrap_or_default();
    let display = if label.is_empty() {
        "(inherit)".to_string()
    } else {
        label
    };
    combo_button(
        theme,
        "style-cfg-font-size",
        display,
        dialog.font_size_dropdown_open,
        enabled,
        {
            let entity = entity.clone();
            move |cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.font_size_dropdown_open = !this.font_size_dropdown_open;
                    if this.font_size_dropdown_open {
                        this.font_dropdown_open = false;
                        this.theme_dropdown_open = false;
                        this.close_colour_picker();
                    }
                    cx.notify();
                });
            }
        },
    )
}

fn combo_button<F>(
    theme: &Theme,
    id: &'static str,
    display: String,
    open: bool,
    enabled: bool,
    handler: F,
) -> AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let bg = if enabled {
        theme.button_bg
    } else {
        theme.chrome
    };
    let hover_bg = theme.button_hover_bg;
    let border = if enabled {
        theme.group_box_border
    } else {
        theme.disabled_text
    };
    let fg = if enabled {
        theme.text
    } else {
        theme.disabled_text
    };

    let node = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(rems(22. / 16.))
        .min_w(rems(140. / 16.))
        .px(rems(8. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_color(rgb(fg))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(display))
        .child(SharedString::from(if open { "^" } else { "v" }));

    if enabled {
        node.cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                handler(cx);
            })
            .into_any_element()
    } else {
        node.cursor(CursorStyle::OperationNotAllowed)
            .into_any_element()
    }
}

/// Hoisted overlay (scrim + panel) for the font-name dropdown. Returns
/// `None` when the dropdown is closed. Mirrors `header::theme_dropdown_overlay`
/// — clicking the scrim closes the dropdown without falling through to any
/// underlying control.
pub(super) fn font_dropdown_overlay(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> Option<AnyElement> {
    if !dialog.font_dropdown_open {
        return None;
    }
    let scrim_entity = entity.clone();
    let scrim = div()
        .id("style-cfg-font-dropdown-scrim")
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_entity.update(cx, |this, cx| {
                this.font_dropdown_open = false;
                cx.notify();
            });
        });
    let panel = font_dropdown_panel(theme, dialog, entity);
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

/// Float panel listing every font family. Rendered absolutely positioned
/// inside the right column when `dialog.font_dropdown_open == true`.
pub(super) fn font_dropdown_panel(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let active_name = dialog
        .state
        .current_style()
        .and_then(|s| s.font_name.clone())
        .unwrap_or_default();

    // Coords are root-relative (the panel is hoisted into the dialog
    // root by `mod.rs::render` so it paints on top of every body child).
    // The Font name combo sits inside the Font Style group box on the
    // right half of the body. Vertical math (760×380 window, fixed):
    //   root.p (8) + header.h (32) + root.gap (8)            = 48 (body top)
    //   description (20) + gap (10) + preview (36) + gap (10) = 76 (inner row top in body)
    //   group_box pt (14) + bordered box pt (12)              = 26 (above name_row)
    //   name_row.h (22)                                       = 22
    //   total to combo bottom: 48 + 76 + 26 + 22              = 172
    // Panel sits ~2px below combo: top = 174/16 rem.
    // Right-anchored at root.p + group_box.px = 8 + 10 = 18px,
    // visually flush with the combo's right edge.
    let mut panel = div()
        .id("style-cfg-font-name-panel")
        .absolute()
        .top(rems(174. / 16.))
        .right(rems(18. / 16.))
        .min_w(rems(220. / 16.))
        .max_h(rems(220. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .flex()
        .flex_col()
        // Block clicks from falling through to the dismiss-scrim sibling
        // so picking a row registers cleanly. Without this the scrim's
        // close-on-click handler fires alongside the row handler.
        .occlude();

    // First row: clear / inherit.
    let entity_clear = entity.clone();
    panel = panel.child(
        dropdown_row(
            theme,
            "style-cfg-font-row-inherit".into(),
            "(inherit Default Style)".to_string(),
            active_name.is_empty(),
            move |cx| {
                let _ = entity_clear.update(cx, |this, cx| {
                    if let Some(style) = this.state.current_style_mut() {
                        if style.font_name.is_some() {
                            style.font_name = None;
                            this.state.notify_modified();
                        }
                    }
                    this.font_dropdown_open = false;
                    this.refresh_preview(cx);
                });
            },
        ),
    );

    for (idx, name) in dialog.font_names.iter().enumerate() {
        let is_active = !active_name.is_empty() && active_name == *name;
        let entity_pick = entity.clone();
        let name_owned = name.clone();
        panel = panel.child(dropdown_row(
            theme,
            SharedString::from(format!("style-cfg-font-row-{idx}")),
            name.clone(),
            is_active,
            move |cx| {
                let new_name = name_owned.clone();
                let _ = entity_pick.update(cx, |this, cx| {
                    if let Some(style) = this.state.current_style_mut() {
                        if style.font_name.as_deref() != Some(new_name.as_str()) {
                            style.font_name = Some(new_name);
                            this.state.notify_modified();
                        }
                    }
                    this.font_dropdown_open = false;
                    this.refresh_preview(cx);
                });
            },
        ));
    }

    panel.into_any_element()
}

/// Hoisted overlay (scrim + panel) for the font-size dropdown. Returns
/// `None` when closed. Click on scrim → close.
pub(super) fn size_dropdown_overlay(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> Option<AnyElement> {
    if !dialog.font_size_dropdown_open {
        return None;
    }
    let scrim_entity = entity.clone();
    let scrim = div()
        .id("style-cfg-size-dropdown-scrim")
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_entity.update(cx, |this, cx| {
                this.font_size_dropdown_open = false;
                cx.notify();
            });
        });
    let panel = size_dropdown_panel(theme, dialog, entity);
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

/// Float panel listing the fixed font-size choices.
pub(super) fn size_dropdown_panel(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let active = dialog
        .state
        .current_style()
        .and_then(|s| s.font_size)
        .map(|n| n.to_string())
        .unwrap_or_default();

    // Vertical math (continued from font_dropdown_panel):
    //   name_row bottom = 172
    //   gap (8) + bold/italic/underline (47) + gap (8) + size_row.h (22) = 85
    //   total to size combo bottom: 172 + 85 = 257
    // Panel top sits ~3px below: 260/16 rem.
    let mut panel = div()
        .id("style-cfg-font-size-panel")
        .absolute()
        .top(rems(260. / 16.))
        .right(rems(18. / 16.))
        .min_w(rems(80. / 16.))
        .max_h(rems(220. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .flex()
        .flex_col()
        .occlude();

    for (idx, size) in FONT_SIZES.iter().enumerate() {
        let display = if size.is_empty() {
            "(inherit)".to_string()
        } else {
            size.to_string()
        };
        let is_active = active == *size;
        let entity_pick = entity.clone();
        let size_str = size.to_string();
        panel = panel.child(dropdown_row(
            theme,
            SharedString::from(format!("style-cfg-size-row-{idx}")),
            display,
            is_active,
            move |cx| {
                let parsed = if size_str.is_empty() {
                    None
                } else {
                    size_str.parse::<u32>().ok()
                };
                let _ = entity_pick.update(cx, |this, cx| {
                    if let Some(style) = this.state.current_style_mut() {
                        if style.font_size != parsed {
                            style.font_size = parsed;
                            this.state.notify_modified();
                        }
                    }
                    this.font_size_dropdown_open = false;
                    this.refresh_preview(cx);
                });
            },
        ));
    }

    panel.into_any_element()
}

fn dropdown_row<F>(
    theme: &Theme,
    id: SharedString,
    label: String,
    is_active: bool,
    handler: F,
) -> AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let row = div()
        .id(id)
        .px(rems(8. / 16.))
        .py(rems(3. / 16.))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .when(is_active, |s| s.bg(rgb(theme.button_pressed_bg)))
        .when(!is_active, |s| {
            s.cursor(CursorStyle::PointingHand)
                .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
        })
        .child(SharedString::from(label));

    if is_active {
        row.into_any_element()
    } else {
        row.on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            handler(cx);
        })
        .into_any_element()
    }
}
