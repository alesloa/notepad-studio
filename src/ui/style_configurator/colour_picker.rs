// SPDX-License-Identifier: Apache-2.0
//
// Colour picker popover for the Style Configurator.
//
// Shipped surface (this file):
//
//   ┌────────────────────────────────────────────┐
//   │ Foreground colour                          │
//   │                                            │
//   │ ┌────┐ ┌────┐  Hex:  F8F8F2                │
//   │ │new │ │was │                              │
//   │ └────┘ └────┘                              │
//   │                                            │
//   │ ┌───────────────────────┐┌─┐               │
//   │ │                       ││ │               │
//   │ │   SV gradient (HSV)   ││ │  Hue strip    │
//   │ │   draggable thumb     ││ │  vertical     │
//   │ │                       ││ │               │
//   │ └───────────────────────┘└─┘               │
//   │                                            │
//   │ ┌─┬─┬─┬─┬─┬─┬─┬─┬─┬─┐                      │
//   │ │ │ │ │ │ │ │ │ │ │ │                      │
//   │ ├─┼─┼─┼─┼─┼─┼─┼─┼─┼─┤   swatch grid        │
//   │ │ │ │ │ │ │ │ │ │ │ │   (quick-pick)       │
//   │ ├─┼─┼─┼─┼─┼─┼─┼─┼─┼─┤                      │
//   │ ...                                        │
//   │ └─┴─┴─┴─┴─┴─┴─┴─┴─┴─┘                      │
//   │                                            │
//   │                    [ OK ] [ Cancel ]       │
//   └────────────────────────────────────────────┘
//
// Three picking surfaces, all writing to the same `picker_buffer`:
//
//   * HSV row (`hsv_surface.rs`) — saturation × value gradient + hue
//     strip on the right. Drag for arbitrary smooth picks.
//   * Swatch grid — quick-access hue × lightness palette + greyscale
//     column. Single click picks a curated colour.
//   * Hex input — typed 6-digit hex for exact values.
//
// Hex input is the canonical source — every other surface mutates the
// buffer and the buffer drives the live editor preview. HSV state on
// the dialog (`picker_hue/sat/val`) is derived from the buffer via
// `hsv_surface::sync_from_buffer` after a non-HSV write so the SV thumb
// and hue tick track typed / swatched changes too.
//
// OK commits the picked hex onto the active style; Cancel discards the
// in-progress edit. Click-outside (scrim) keeps the user's last pick
// (NPP soft-commit behaviour).
//
// The popover is hoisted to the dialog root by `mod.rs::render` so the
// scrim added by `overlay()` covers the whole window. Clicking outside
// the popover closes it without falling through to whatever underlying
// control would have been hit.

use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, ElementInputHandler, Entity, FocusHandle, Hsla,
    IntoElement, MouseButton, ParentElement, Pixels, Rgba, SharedString, Styled, TextRun,
    WeakEntity, Window, div, fill, point, prelude::*, px, rems, rgb, rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::style_model::style::{format_hex_rgb, parse_hex_rgb};
use crate::theme::Theme;

use super::StyleConfiguratorDialog;
use super::hsv_surface;

/// Which channel of the active style the picker is editing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum PickerTarget {
    Foreground,
    Background,
}

impl PickerTarget {
    pub(super) fn label(self) -> &'static str {
        match self {
            PickerTarget::Foreground => "Foreground",
            PickerTarget::Background => "Background",
        }
    }
}

/// Open the picker for `target`, seeded from the current style's value.
/// Called from the right-column swatch click handlers. The seed colour
/// is captured so Cancel / click-outside can revert any swatch-click
/// previews back to the original value.
pub(super) fn open(dialog: &mut StyleConfiguratorDialog, target: PickerTarget) {
    let seed = dialog.state.current_style().and_then(|s| match target {
        PickerTarget::Foreground => s.fg_color,
        PickerTarget::Background => s.bg_color,
    });
    let buffer = seed.map(format_hex_rgb).unwrap_or_default();
    dialog.picker_target = Some(target);
    dialog.picker_cursor = buffer.len();
    dialog.picker_anchor = buffer.len();
    dialog.picker_buffer = buffer;
    dialog.picker_seed = seed;
    // Seed HSV from the starting hex so SV / hue thumbs land on the
    // current colour the moment the picker opens.
    hsv_surface::sync_from_buffer(dialog);
}

/// Called from `EntityInputHandler::replace_text_in_range` and
/// `handle_picker_key_down` after `picker_buffer` mutates. Pushes the
/// new buffer onto the active style for the live preview, refreshes the
/// editor, and re-renders the popover.
pub(super) fn on_buffer_changed(
    dialog: &mut StyleConfiguratorDialog,
    cx: &mut Context<StyleConfiguratorDialog>,
) {
    // Skip HSV resync when an HSV drag is the source — the drag handler
    // already mutated h/s/v before writing the buffer, and `rgb_to_hsv`
    // can't recover the user's hue from a grey colour (it returns 0,
    // which would snap the hue thumb every time the user dragged
    // through the grey midline).
    if dialog.picker_hsv_drag == hsv_surface::HsvDrag::None {
        hsv_surface::sync_from_buffer(dialog);
    }
    apply_buffer_to_style(dialog);
    dialog.refresh_preview(cx);
    cx.notify();
}

/// Hoisted overlay (scrim + popover) for the colour picker. Returns
/// `None` when no picker is open. The scrim sits below the popover so
/// clicks outside the popover close it without falling through.
pub(super) fn overlay(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> Option<AnyElement> {
    let popover = render(theme, dialog, entity)?;
    let scrim_entity = entity.clone();
    let scrim = div()
        .id("style-cfg-colour-picker-scrim")
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_entity.update(cx, |this, cx| {
                this.close_colour_picker();
                cx.notify();
            });
        });
    Some(
        div()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .size_full()
            .child(scrim)
            .child(popover)
            .into_any_element(),
    )
}

/// Render the popover when `dialog.picker_target` is `Some`. Otherwise
/// returns `None` so the caller can skip layout entirely.
pub(super) fn render(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> Option<AnyElement> {
    let target = dialog.picker_target?;
    let buffer = dialog.picker_buffer.clone();
    let new_color = parse_hex_rgb(&buffer);
    let prior_color = dialog.state.current_style().and_then(|s| match target {
        PickerTarget::Foreground => s.fg_color,
        PickerTarget::Background => s.bg_color,
    });

    let title = div()
        .text_color(rgb(theme.text))
        .text_size(rems(12. / 16.))
        .child(SharedString::from(format!("{} colour", target.label())));

    let new_swatch_label = div()
        .text_color(rgb(theme.disabled_text))
        .text_size(rems(9.5 / 16.))
        .child(SharedString::from("New"));
    let was_swatch_label = div()
        .text_color(rgb(theme.disabled_text))
        .text_size(rems(9.5 / 16.))
        .child(SharedString::from("Was"));

    let swatch_box = |fill: Option<u32>, theme: &Theme| {
        let mut node = div()
            .w(rems(36. / 16.))
            .h(rems(28. / 16.))
            .border_1()
            .border_color(rgb(theme.group_box_border));
        if let Some(rgb_v) = fill {
            node = node.bg(rgb(rgb_v));
        } else {
            node = node.bg(rgb(theme.chrome));
        }
        node
    };
    let new_swatch = swatch_box(new_color, theme);
    let was_swatch = swatch_box(prior_color, theme);

    let hex_label = div()
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from("Hex:"));

    // Custom Element wrapped in a div with `.id()` + `.track_focus()` —
    // track_focus is mandatory here (GPUI focus-handle caveat). The
    // Element calls `window.handle_input(...)` so typed chars / paste /
    // IME flow through `EntityInputHandler::replace_text_in_range`.
    let hex_input = div()
        .id("style-cfg-hex-input")
        .track_focus(&dialog.hex_input_focus)
        .occlude()
        .flex()
        .items_center()
        .h(rems(22. / 16.))
        .min_w(rems(80. / 16.))
        .px(rems(6. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(if new_color.is_none() && !buffer.is_empty() {
            theme.error
        } else {
            theme.group_box_border
        }))
        .text_color(rgb(theme.text))
        .text_size(rems(11.5 / 16.))
        .cursor(CursorStyle::IBeam)
        .child(HexInput {
            dialog: entity.downgrade(),
        });

    let header_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(2. / 16.))
                .child(new_swatch_label)
                .child(new_swatch),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(2. / 16.))
                .child(was_swatch_label)
                .child(was_swatch),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(rems(6. / 16.))
                .ml(rems(8. / 16.))
                .child(hex_label)
                .child(hex_input),
        );

    let hsv_row = hsv_surface::render(theme, "style-cfg", entity);
    let grid = swatch_grid(theme, entity);
    let buttons = action_row(theme, entity);

    Some(
        div()
            .id("style-cfg-colour-picker")
            .absolute()
            // Popover sits roughly centred under the colour-style group.
            // Coords are root-relative because the overlay is hoisted to
            // the dialog root. Anchored to top 32 (was 80) — below the
            // header strip's 50-px chrome but high enough to leave room
            // for the title + HSV row + swatch grid + OK/Cancel inside
            // the 380-px dialog window.
            .top(rems(32. / 16.))
            .right(rems(40. / 16.))
            .w(rems(380. / 16.))
            // Cap popover height + scroll if content still overruns.
            // The HSV row added ~100 px; without the cap, OK / Cancel
            // would be pushed off the bottom edge on the default dialog
            // size and only become reachable via the OS scroll wheel
            // (which is jarring inside a modal popover).
            .max_h(rems(320. / 16.))
            .overflow_y_scroll()
            .bg(rgb(theme.bg))
            .border_1()
            .border_color(rgb(theme.group_box_border))
            .p(rems(12. / 16.))
            // Mark this hitbox opaque so clicks on the popover (including
            // the hex input and swatch grid) can't fall through to the
            // dismiss-scrim sibling sitting under it. Without this, GPUI
            // forwards every mouse_down to every overlapping element with
            // a handler — the scrim's close-on-click handler then fires
            // alongside the input's focus handler and the popover snaps
            // shut as soon as the user tries to interact with it.
            .occlude()
            .flex()
            .flex_col()
            .gap(rems(10. / 16.))
            .child(title)
            .child(header_row)
            .child(hsv_row)
            .child(grid)
            .child(buttons)
            .into_any_element(),
    )
}

/// Visual swatch grid — the picker's primary surface. Generates a
/// hue × lightness grid (9 hues × 7 lightnesses) plus a trailing column
/// of greyscales (8 levels). Click a swatch → update buffer + live
/// preview the colour on the editor.
fn swatch_grid(theme: &Theme, entity: &Entity<StyleConfiguratorDialog>) -> AnyElement {
    // Hue values cover the colour wheel at even intervals so the grid
    // reads as a rainbow. 9 hues — enough variety without making each
    // cell too small for a comfortable click target.
    const HUES: [f32; 9] = [
        0.0,        // red
        30.0 / 360.0,  // orange
        60.0 / 360.0,  // yellow
        100.0 / 360.0, // lime
        140.0 / 360.0, // green
        180.0 / 360.0, // cyan
        220.0 / 360.0, // blue
        270.0 / 360.0, // purple
        315.0 / 360.0, // magenta
    ];
    // Lightness values for the hue rows. 7 levels — light to dark —
    // cover most editor-style use cases (light backgrounds, mid-tones,
    // dark accent text). Avoids the extremes (1.0, 0.0) which collapse
    // to white/black on every hue.
    const LIGHTNESSES: [f32; 7] = [0.92, 0.78, 0.64, 0.50, 0.38, 0.26, 0.14];
    // Greyscale column — 8 levels including pure white and pure black.
    const GREYS: [u32; 8] = [
        0xFFFFFF, 0xE0E0E0, 0xC0C0C0, 0x808080, 0x606060, 0x404040, 0x202020, 0x000000,
    ];

    let mut grid = div()
        .flex()
        .flex_col()
        .gap(rems(2. / 16.));

    for (row_idx, &lightness) in LIGHTNESSES.iter().enumerate() {
        let mut row = div()
            .flex()
            .flex_row()
            .gap(rems(2. / 16.));
        for (col_idx, &hue) in HUES.iter().enumerate() {
            let rgb_val = hsl_to_rgb(hue, 0.85, lightness);
            row = row.child(swatch_cell(
                theme,
                entity,
                format!("style-cfg-swatch-h{col_idx}-l{row_idx}"),
                rgb_val,
            ));
        }
        // 8th-grey-row: only paint a grey cell for rows with a matching
        // grey index. We have 8 greys but 7 lightness rows — pad with
        // white at the top and black at the bottom by mapping row_idx
        // directly into the GREYS slice (7 of 8 grey shades show up
        // alongside the hues, the 8th — pure black — sits in the row
        // appended below).
        if let Some(&g) = GREYS.get(row_idx) {
            row = row.child(swatch_cell(
                theme,
                entity,
                format!("style-cfg-swatch-grey-{row_idx}"),
                g,
            ));
        }
        grid = grid.child(row);
    }
    // Trailing pure-black row aligned under the grey column.
    let [.., black] = GREYS;
    let last_row = div()
        .flex()
        .flex_row()
        .justify_end()
        .child(swatch_cell(
            theme,
            entity,
            "style-cfg-swatch-black".to_string(),
            black,
        ));
    grid = grid.child(last_row);

    grid.into_any_element()
}

fn swatch_cell(
    theme: &Theme,
    entity: &Entity<StyleConfiguratorDialog>,
    id: String,
    rgb_val: u32,
) -> AnyElement {
    let entity = entity.clone();
    div()
        .id(SharedString::from(id))
        .w(rems(22. / 16.))
        .h(rems(22. / 16.))
        .bg(rgb(rgb_val))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.border_color(rgb(theme.text)))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            entity.update(cx, |this, cx| {
                let buf = format_hex_rgb(rgb_val);
                this.picker_cursor = buf.len();
                this.picker_anchor = buf.len();
                this.picker_buffer = buf;
                // Sync HSV so the SV thumb / hue tick jump to the
                // swatch's colour. Safe to call unconditionally — swatch
                // clicks never happen during an HSV drag (drag would
                // own the mouse), so the drag-guard branch in
                // `on_buffer_changed` isn't load-bearing here.
                hsv_surface::sync_from_buffer(this);
                // Apply immediately so the editor previews the new
                // colour. The popover stays open so the user can fine-
                // tune by picking a different swatch or typing hex.
                apply_buffer_to_style(this);
                this.refresh_preview(cx);
            });
        })
        .into_any_element()
}

/// HSL → RGB packed into a u32 (0xRRGGBB). Standard formula. `h` in
/// [0, 1), `s` and `l` in [0, 1].
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h * 6.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let r = ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    let g = ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    let b = ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (r << 16) | (g << 8) | b
}

fn action_row(theme: &Theme, entity: &Entity<StyleConfiguratorDialog>) -> AnyElement {
    let ok_entity = entity.clone();
    let cancel_entity = entity.clone();

    let ok_btn = popover_button(theme, "ok", "OK", true, move |cx| {
        let _ = ok_entity.update(cx, |this, cx| {
            commit(this);
            this.refresh_preview(cx);
        });
    });
    let cancel_btn = popover_button(theme, "cancel", "Cancel", false, move |cx| {
        let _ = cancel_entity.update(cx, |this, cx| {
            // Cancel — revert the in-progress preview to whatever the
            // current style's saved colour was, then close the picker.
            // `cancel_colour_picker` is the dedicated revert path; the
            // generic `close_colour_picker` keeps the user's pick
            // committed (NPP soft-commit behaviour for click-outside).
            this.cancel_colour_picker();
            this.refresh_preview(cx);
        });
    });

    div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(rems(6. / 16.))
        .child(ok_btn)
        .child(cancel_btn)
        .into_any_element()
}

fn popover_button<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    primary: bool,
    handler: F,
) -> AnyElement
where
    F: Fn(&mut gpui::App) + 'static,
{
    let bg = theme.button_bg;
    let hover_bg = theme.button_hover_bg;
    let border = if primary {
        theme.checkbox_fill
    } else {
        theme.group_box_border
    };

    div()
        .id(SharedString::from(format!("style-cfg-picker-{id}")))
        .flex()
        .items_center()
        .justify_center()
        .h(rems(24. / 16.))
        .min_w(rems(80. / 16.))
        .px(rems(10. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_color(rgb(theme.text))
        .text_size(rems(11.5 / 16.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(rgb(hover_bg)))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            handler(cx);
        })
        .into_any_element()
}

/// Push the current `picker_buffer` onto the active style without
/// closing the popover. Used by the swatch-click handler to make
/// picking a swatch instantly visible in the editor preview.
fn apply_buffer_to_style(dialog: &mut StyleConfiguratorDialog) {
    let Some(target) = dialog.picker_target else {
        return;
    };
    let Some(value) = parse_hex_rgb(&dialog.picker_buffer) else {
        return;
    };
    if let Some(style) = dialog.state.current_style_mut() {
        match target {
            PickerTarget::Foreground => style.fg_color = Some(value),
            PickerTarget::Background => style.bg_color = Some(value),
        }
        dialog.state.notify_modified();
    }
}

/// Apply the current `picker_buffer` to the active style and close the
/// popover. If the buffer doesn't parse as 6-digit hex the picker stays
/// open (the user can fix it). On a successful commit we clear the
/// seed FIRST so `close_colour_picker` doesn't revert what we just
/// applied.
pub(super) fn commit(dialog: &mut StyleConfiguratorDialog) {
    apply_buffer_to_style(dialog);
    if parse_hex_rgb(&dialog.picker_buffer).is_some() {
        dialog.picker_seed = None;
        dialog.close_colour_picker();
    }
}

// Custom text input element for the colour-picker hex field.
//
// Mirrors the `DialogTextInput` pattern from `preferences/widgets.rs`:
// the wrapper div carries `.id() + .track_focus()` so the focus_id
// anchors to a dedicated dispatch-tree node; this Element's `paint`
// calls `window.handle_input(handle, ElementInputHandler::new(bounds,
// dialog), cx)` so typed chars / paste / IME route to the dialog's
// `EntityInputHandler` impl. Rendering: the buffer is shown with a
// leading `#` (greyed if empty), with a selection highlight and caret
// when this field is focused.
pub(super) struct HexInput {
    pub(super) dialog: WeakEntity<StyleConfiguratorDialog>,
}

impl IntoElement for HexInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for HexInput {
    type RequestLayoutState = ();
    type PrepaintState = Option<(FocusHandle, Option<gpui::ShapedLine>, usize, usize, String)>;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let dialog = self.dialog.upgrade()?;
        let dialog_ref = dialog.read(cx);
        let buffer = dialog_ref.picker_buffer.clone();
        let cursor = dialog_ref.picker_cursor.min(buffer.len());
        let anchor = dialog_ref.picker_anchor.min(buffer.len());
        let handle = dialog_ref.hex_input_focus.clone();
        let theme = dialog_ref.theme();

        let display_text = if buffer.is_empty() {
            String::from("#------")
        } else {
            format!("#{buffer}")
        };

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let color: Hsla = if buffer.is_empty() {
            Rgba::from(rgb(theme.disabled_text)).into()
        } else {
            Rgba::from(rgb(theme.text)).into()
        };
        let runs = vec![TextRun {
            len: display_text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let shaped = Some(window.text_system().shape_line(
            SharedString::from(display_text.clone()),
            font_size,
            &runs,
            None,
        ));

        Some((handle, shaped, cursor, anchor, buffer))
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let Some((handle, shaped, cursor, anchor, buffer)) = prepaint.as_mut() else {
            return;
        };
        let theme = dialog.read(cx).theme();
        let line_height = window.line_height();
        let baseline_y = bounds.top() + (bounds.size.height - line_height).max(px(0.)) / 2.0;
        // Display string starts with `#`; selection / caret indices are
        // into the BUFFER (no `#`) — shift by 1 char to account for the
        // leading hash when mapping to display offsets.
        let display_offset = 1usize;

        // Selection rect.
        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        if hi > lo && !buffer.is_empty() {
            if let Some(line) = shaped.as_ref() {
                let lo_disp = (lo + display_offset).min(line.len);
                let hi_disp = (hi + display_offset).min(line.len);
                let x_lo = bounds.left() + line.x_for_index(lo_disp);
                let x_hi = bounds.left() + line.x_for_index(hi_disp);
                let sel_color: Hsla = rgba(theme.selection).into();
                window.paint_quad(fill(
                    Bounds::new(point(x_lo, baseline_y), size(x_hi - x_lo, line_height)),
                    sel_color,
                ));
            }
        }

        // Glyphs.
        if let Some(line) = shaped.take() {
            let _ = line.paint(
                point(bounds.left(), baseline_y),
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        // Caret — only when focused.
        if handle.is_focused(window) {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let display_text = if buffer.is_empty() {
                String::from("#------")
            } else {
                format!("#{buffer}")
            };
            let caret_color: Hsla = Rgba::from(rgb(theme.text)).into();
            let runs = vec![TextRun {
                len: display_text.len(),
                font: style.font(),
                color: caret_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let line = window.text_system().shape_line(
                SharedString::from(display_text),
                font_size,
                &runs,
                None,
            );
            let cx_idx = if buffer.is_empty() {
                display_offset
            } else {
                (*cursor + display_offset).min(line.len)
            };
            let cx_pos = bounds.left() + line.x_for_index(cx_idx);
            window.paint_quad(fill(
                Bounds::new(
                    point(cx_pos, baseline_y),
                    size(px(CARET_WIDTH_PX), line_height),
                ),
                caret_color,
            ));
        }

        // IME / OS text input router. Without this call typed chars /
        // paste / dead-key composition do nothing — the bug we're fixing.
        window.handle_input(handle, ElementInputHandler::new(bounds, dialog.clone()), cx);

        // Mouse-down — focus the field, place the cursor at the click x,
        // start a drag (anchor=cursor=click position). Double-click
        // selects the entire buffer (parity with native macOS / GTK
        // text fields where double-click selects the word — for a 6-
        // char hex value the "word" is the whole buffer).
        let dialog_for_down = dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            cx.stop_propagation();
            let click_count = event.click_count;
            dialog_for_down.update(cx, |this, cx| {
                let handle = this.hex_input_focus.clone();
                window.focus(&handle, cx);
                if click_count >= 2 {
                    this.picker_anchor = 0;
                    this.picker_cursor = this.picker_buffer.len();
                    this.picker_drag_active = false;
                } else {
                    let click_x = event.position.x - bounds.left();
                    let buf_byte =
                        byte_for_x_in_buffer(&this.picker_buffer, click_x, window);
                    this.picker_cursor = buf_byte;
                    this.picker_anchor = buf_byte;
                    this.picker_drag_active = true;
                }
                cx.notify();
            });
        });

        // Mouse-move — while a drag started inside the input is active,
        // extend the cursor to the current x position. Anchor stays
        // pinned at the original mouse-down byte. Window-global so the
        // drag tracks even after the cursor leaves the input bounds.
        let dialog_for_move = dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            if !event.dragging() {
                return;
            }
            dialog_for_move.update(cx, |this, cx| {
                if !this.picker_drag_active {
                    return;
                }
                let click_x = event.position.x - bounds.left();
                let buf_byte = byte_for_x_in_buffer(&this.picker_buffer, click_x, window);
                if this.picker_cursor != buf_byte {
                    this.picker_cursor = buf_byte;
                    cx.notify();
                }
            });
        });

        // Mouse-up — end any in-progress drag. Window-global so a release
        // outside the input bounds still terminates the drag.
        let dialog_for_up = dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            dialog_for_up.update(cx, |this, _cx| {
                this.picker_drag_active = false;
            });
        });
    }
}

/// Map a click x-offset (relative to the input bounds) to a byte
/// position inside the hex buffer (no leading `#`). Returns 0 on click
/// before the first char, `len` on click past the end.
fn byte_for_x_in_buffer(buffer: &str, x: Pixels, window: &mut Window) -> usize {
    let display_text = if buffer.is_empty() {
        String::from("#------")
    } else {
        format!("#{buffer}")
    };
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let runs = vec![TextRun {
        len: display_text.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window.text_system().shape_line(
        SharedString::from(display_text),
        font_size,
        &runs,
        None,
    );
    let local = if x < px(0.) { px(0.) } else { x };
    let display_idx = line.closest_index_for_x(local).min(line.len);
    // Subtract the leading `#` (1 byte). Clamp to buffer length.
    display_idx.saturating_sub(1).min(buffer.len())
}
