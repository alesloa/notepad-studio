// SPDX-License-Identifier: Apache-2.0
//
// Right column of the Style Configurator dialog.
//
// Three vertical sections: a description label, the "Colour Style" group
// (Foreground / Background swatches), and the "Font Style" group (Font
// name + Bold / Italic / Underline + Font size). The colour-picker
// popover floats above the group boxes when active.
//
// Disable rules (UI spec §6 + §2.4 / §2.5 / §2.6):
//   * No style selected — every control disabled.
//   * "Selected text colour" — fg swatch disabled (NPP only enables it
//     when the `_selectedTextForegroundSingleColor` pref is on; we don't
//     ship that pref yet, so the swatch stays greyed). bg stays editable.
//
// Phase 4 hooks every edit through `state.notify_modified()` so the
// footer's Save & Close button enables on the first change. The actual
// XML write lands in Phase 5.

use gpui::{
    AnyElement, CursorStyle, Entity, FontStyle, FontWeight, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, div, prelude::*, px, rems, rgb,
};

use crate::style_model::style::FontFlags;
use crate::theme::Theme;

use super::colour_picker::{self, PickerTarget};
use super::font_picker;
use super::state::OverrideField;
use super::StyleConfiguratorDialog;

const SELECTED_TEXT_COLOUR_NAME: &str = "Selected text colour";
const CURRENT_LINE_BG_COLOUR_NAME: &str = "Current line background colour";
const GLOBAL_OVERRIDE_STYLE_NAME: &str = "Global override";

pub(super) fn render(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let style = dialog.state.current_style();
    let fg_disabled = is_fg_disabled(style.map(|s| s.name.as_str()));
    let bg_disabled = false;
    let controls_disabled = style.is_none();

    let description = description_label(theme, dialog);
    let preview = preview_strip(theme, dialog);
    let colour_group = colour_group(
        theme,
        dialog,
        entity,
        controls_disabled,
        fg_disabled,
        bg_disabled,
    );
    let font_group = font_group(theme, dialog, entity, controls_disabled);

    let show_override_panel = is_global_override_selected(dialog);

    let mut col = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(10. / 16.))
        .child(description)
        .child(preview)
        .child(
            div()
                .flex()
                .flex_row()
                .gap(rems(12. / 16.))
                .child(colour_group)
                .child(font_group),
        );

    if show_override_panel {
        col = col.child(global_override_panel(theme, dialog, entity));
    }

    // Popovers (colour picker, font name dropdown, font size dropdown)
    // are NOT rendered here anymore — they're hoisted to the dialog
    // root in `mod.rs::render` so each gets a full-window dismiss scrim
    // (clicking anywhere outside the popover closes it). Rendering here
    // would clip the scrim to the right-column bounds and let clicks
    // outside that column slip through to other controls.

    col.into_any_element()
}

/// Live preview of the currently-selected style. Renders a fixed sample
/// string with the style's `fg_color`, `bg_color`, `font_name`, `font_size`,
/// and `font_style` flags applied, so the user sees what the style looks
/// like in the editor before they Save & Close.
///
/// Falls back to theme bg/text + Menlo 13 when the style has no value for
/// the corresponding field — matches NPP's rendering rule that an unset
/// per-style attribute inherits from `Default Style` / the editor base.
fn preview_strip(theme: &Theme, dialog: &StyleConfiguratorDialog) -> AnyElement {
    let style = dialog.state.current_style();
    // Fallback to the active theme's "Default Style" so the preview reads
    // sensible defaults when the picked style itself didn't override fg/bg.
    let default_style = dialog
        .state
        .document
        .global_styles
        .iter()
        .find(|s| s.name == "Default Style");
    let fallback_fg = default_style.and_then(|s| s.fg_color).unwrap_or(theme.text);
    let fallback_bg = default_style.and_then(|s| s.bg_color).unwrap_or(theme.bg);

    let fg = style.and_then(|s| s.fg_color).unwrap_or(fallback_fg);
    let bg = style.and_then(|s| s.bg_color).unwrap_or(fallback_bg);
    let flags = style
        .and_then(|s| s.font_style)
        .unwrap_or_else(FontFlags::empty);
    let font_name = style
        .and_then(|s| s.font_name.as_ref())
        .filter(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| crate::fonts::mono().to_string());
    let font_size = style
        .and_then(|s| s.font_size)
        .filter(|n| *n > 0)
        .unwrap_or(13);

    let mut sample = div()
        .flex()
        .items_center()
        .h(rems(36. / 16.))
        .px(rems(10. / 16.))
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .font_family(SharedString::from(font_name))
        .text_size(px(font_size as f32))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .child(SharedString::from("AaBbCc 123  if (x) { return \"hi\"; } // sample"));

    if flags.bold() {
        sample = sample.font_weight(FontWeight::BOLD);
    }
    if flags.italic() {
        sample.text_style().font_style = Some(FontStyle::Italic);
    }
    sample.into_any_element()
}

fn description_label(theme: &Theme, dialog: &StyleConfiguratorDialog) -> AnyElement {
    let lang_name = if dialog.state.selected_lang_idx == 0 {
        "Global Styles".to_string()
    } else {
        dialog
            .state
            .current_lexer()
            .map(|l| l.lexer_desc.clone())
            .unwrap_or_default()
    };
    let style_name = dialog
        .state
        .current_style()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "(no style)".to_string());

    div()
        .text_color(rgb(theme.text))
        .text_size(rems(12. / 16.))
        .child(SharedString::from(format!("{lang_name}: {style_name}")))
        .into_any_element()
}

fn colour_group(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
    all_disabled: bool,
    fg_disabled: bool,
    bg_disabled: bool,
) -> AnyElement {
    let style = dialog.state.current_style();
    let fg_value = style.and_then(|s| s.fg_color);
    let bg_value = style.and_then(|s| s.bg_color);

    let fg_row = swatch_row(
        theme,
        "Foreground colour",
        fg_value,
        all_disabled || fg_disabled,
        PickerTarget::Foreground,
        entity,
    );
    let bg_row = swatch_row(
        theme,
        "Background colour",
        bg_value,
        all_disabled || bg_disabled,
        PickerTarget::Background,
        entity,
    );

    group_box(theme, "Colour Style", div().flex().flex_col().gap(rems(8. / 16.)).child(fg_row).child(bg_row))
}

fn font_group(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
    all_disabled: bool,
) -> AnyElement {
    let style = dialog.state.current_style();
    let font_flags = style
        .and_then(|s| s.font_style)
        .unwrap_or_else(FontFlags::empty);

    let name_row = labeled_row(
        theme,
        "Font name:",
        font_picker::name_combo_button(theme, dialog, entity, !all_disabled),
    );
    let size_row = labeled_row(
        theme,
        "Font size:",
        font_picker::size_combo_button(theme, dialog, entity, !all_disabled),
    );

    let bold_box = checkbox(
        theme,
        "style-cfg-bold",
        "Bold",
        font_flags.bold(),
        !all_disabled,
        entity,
        FontFlags::BOLD,
    );
    let italic_box = checkbox(
        theme,
        "style-cfg-italic",
        "Italic",
        font_flags.italic(),
        !all_disabled,
        entity,
        FontFlags::ITALIC,
    );
    let underline_box = checkbox(
        theme,
        "style-cfg-underline",
        "Underline",
        font_flags.underline(),
        !all_disabled,
        entity,
        FontFlags::UNDERLINE,
    );

    let body = div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .child(name_row)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(4. / 16.))
                .child(bold_box)
                .child(italic_box)
                .child(underline_box),
        )
        .child(size_row);

    group_box(theme, "Font Style", body)
}

fn group_box<E>(theme: &Theme, title: &'static str, body: E) -> AnyElement
where
    E: IntoElement,
{
    // Title-on-border NPP-style group box. The header is positioned
    // absolutely so it overlaps the top border line.
    div()
        .relative()
        .flex_1()
        .pt(rems(14. / 16.))
        .child(
            div()
                .border_1()
                .border_color(rgb(theme.group_box_border))
                .px(rems(10. / 16.))
                .pt(rems(12. / 16.))
                .pb(rems(10. / 16.))
                .child(body),
        )
        .child(
            div()
                .absolute()
                .top(rems(6. / 16.))
                .left(rems(14. / 16.))
                .px(rems(4. / 16.))
                .bg(rgb(theme.bg))
                .text_color(rgb(theme.text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(title)),
        )
        .into_any_element()
}

fn labeled_row(theme: &Theme, label: &'static str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(6. / 16.))
        .child(
            div()
                .min_w(rems(60. / 16.))
                .text_color(rgb(theme.text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(label)),
        )
        .child(div().flex_1().child(control))
        .into_any_element()
}

fn swatch_row(
    theme: &Theme,
    label: &'static str,
    value: Option<u32>,
    disabled: bool,
    target: PickerTarget,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let label_text = div()
        .min_w(rems(110. / 16.))
        .text_color(rgb(if disabled {
            theme.disabled_text
        } else {
            theme.text
        }))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(label));

    let swatch_id = match target {
        PickerTarget::Foreground => "style-cfg-swatch-fg",
        PickerTarget::Background => "style-cfg-swatch-bg",
    };

    let swatch = swatch_box(theme, swatch_id, value, disabled, entity, target);

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(label_text)
        .child(swatch)
        .into_any_element()
}

fn swatch_box(
    theme: &Theme,
    id: &'static str,
    value: Option<u32>,
    disabled: bool,
    entity: &Entity<StyleConfiguratorDialog>,
    target: PickerTarget,
) -> AnyElement {
    let border = if disabled {
        theme.disabled_text
    } else {
        theme.group_box_border
    };

    if disabled {
        return div()
            .id(id)
            .w(rems(28. / 16.))
            .h(rems(20. / 16.))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(theme.chrome))
            .cursor(CursorStyle::OperationNotAllowed)
            .into_any_element();
    }

    let entity = entity.clone();
    let mut node = div()
        .id(id)
        .w(rems(28. / 16.))
        .h(rems(20. / 16.))
        .border_1()
        .border_color(rgb(border));

    match value {
        Some(v) => {
            node = node.bg(rgb(v));
        }
        None => {
            // Unset sentinel — bordered chrome cell with a small "/" so
            // the swatch is visually distinct from a real black fill.
            node = node.bg(rgb(theme.chrome)).child(
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(theme.error))
                    .text_size(rems(9. / 16.))
                    .child(SharedString::from("/")),
            );
        }
    }

    node.cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = entity.update(cx, |this, cx| {
                // Close any other open popover before showing the picker
                // so we never stack two floating panels. Mirrors the
                // pattern in the dropdown open-handlers.
                this.theme_dropdown_open = false;
                this.font_dropdown_open = false;
                this.font_size_dropdown_open = false;
                colour_picker::open(this, target);
                cx.notify();
            });
        })
        .into_any_element()
}

fn checkbox(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    checked: bool,
    enabled: bool,
    entity: &Entity<StyleConfiguratorDialog>,
    bit: u8,
) -> AnyElement {
    let box_size = rems(13. / 16.);
    let inner_size = rems(7. / 16.);
    let border_color = if enabled {
        theme.border
    } else {
        theme.disabled_text
    };
    let fill_color = if enabled {
        theme.checkbox_fill
    } else {
        theme.disabled_text
    };
    let label_color = if enabled {
        theme.text
    } else {
        theme.disabled_text
    };

    let mark = div()
        .w(box_size)
        .h(box_size)
        .border_1()
        .border_color(rgb(border_color))
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |this| {
            this.child(div().w(inner_size).h(inner_size).bg(rgb(fill_color)))
        });

    let entity = entity.clone();
    let row = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(mark)
        .child(
            div()
                .text_color(rgb(label_color))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(label)),
        );

    if enabled {
        row.cursor(CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                let _ = entity.update(cx, |this, cx| {
                    set_font_bit(this, bit, !checked);
                    this.refresh_preview(cx);
                });
            })
            .into_any_element()
    } else {
        row.cursor(CursorStyle::OperationNotAllowed).into_any_element()
    }
}

/// Toggle a font-flag bit on the currently-selected style and mark the
/// dialog modified. Initialises `font_style` from `Some(empty())` if it
/// was `None` so the bit can land on a real bitmask.
fn set_font_bit(dialog: &mut StyleConfiguratorDialog, bit: u8, on: bool) {
    if let Some(style) = dialog.state.current_style_mut() {
        let mut flags = style.font_style.unwrap_or_else(FontFlags::empty);
        match bit {
            FontFlags::BOLD => flags.set_bold(on),
            FontFlags::ITALIC => flags.set_italic(on),
            FontFlags::UNDERLINE => flags.set_underline(on),
            _ => return,
        }
        if flags.is_empty() {
            style.font_style = None;
        } else {
            style.font_style = Some(flags);
        }
        dialog.state.notify_modified();
    }
}

fn is_fg_disabled(style_name: Option<&str>) -> bool {
    matches!(
        style_name,
        Some(SELECTED_TEXT_COLOUR_NAME) | Some(CURRENT_LINE_BG_COLOUR_NAME),
    )
}

/// True when "Global Styles ▸ Global override" is the active selection.
/// The seven `Enable global …` checkboxes only render in this case;
/// every other lang/style combination shows the regular Colour/Font
/// controls only.
fn is_global_override_selected(dialog: &StyleConfiguratorDialog) -> bool {
    if dialog.state.selected_lang_idx != 0 {
        return false;
    }
    matches!(
        dialog.state.current_style().map(|s| s.name.as_str()),
        Some(GLOBAL_OVERRIDE_STYLE_NAME)
    )
}

/// Two-column "Enable global …" panel: foreground/background on the left,
/// font / size / bold / italic / underline on the right. Mirrors NPP's
/// layout (UI spec §1, §3) — wraps in the same title-on-border group box
/// so it visually sits next to the Colour Style / Font Style groups.
fn global_override_panel(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let left = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(4. / 16.))
        .child(override_checkbox(
            theme,
            "style-cfg-override-fg",
            "Enable global foreground colour",
            OverrideField::Fg,
            dialog,
            entity,
        ))
        .child(override_checkbox(
            theme,
            "style-cfg-override-bg",
            "Enable global background colour",
            OverrideField::Bg,
            dialog,
            entity,
        ));

    let right = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(4. / 16.))
        .child(override_checkbox(
            theme,
            "style-cfg-override-font",
            "Enable global font",
            OverrideField::Font,
            dialog,
            entity,
        ))
        .child(override_checkbox(
            theme,
            "style-cfg-override-font-size",
            "Enable global font size",
            OverrideField::FontSize,
            dialog,
            entity,
        ))
        .child(override_checkbox(
            theme,
            "style-cfg-override-bold",
            "Enable global bold font style",
            OverrideField::Bold,
            dialog,
            entity,
        ))
        .child(override_checkbox(
            theme,
            "style-cfg-override-italic",
            "Enable global italic font style",
            OverrideField::Italic,
            dialog,
            entity,
        ))
        .child(override_checkbox(
            theme,
            "style-cfg-override-underline",
            "Enable global underline font style",
            OverrideField::Underline,
            dialog,
            entity,
        ));

    let body = div()
        .flex()
        .flex_row()
        .gap(rems(12. / 16.))
        .child(left)
        .child(right);

    group_box(theme, "Global override", body)
}

fn override_checkbox(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    field: OverrideField,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    let checked = dialog.state.override_flag(field);
    let box_size = rems(13. / 16.);
    let inner_size = rems(7. / 16.);

    let mark = div()
        .w(box_size)
        .h(box_size)
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |this| {
            this.child(
                div()
                    .w(inner_size)
                    .h(inner_size)
                    .bg(rgb(theme.checkbox_fill)),
            )
        });

    let entity = entity.clone();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .cursor(CursorStyle::PointingHand)
        .child(mark)
        .child(
            div()
                .text_color(rgb(theme.text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(label)),
        )
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.state.toggle_override(field);
                this.refresh_preview(cx);
            });
        })
        .into_any_element()
}
