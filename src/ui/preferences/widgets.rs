// NPP-style preference dialog primitives shared across all panels.
//
// Every panel renders through these helpers so the visual fidelity rules
// (group box with title-on-border, square checkboxes, text input + Reset
// button row) are honored uniformly. Panels never duplicate styling.

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, ElementInputHandler, FocusHandle, Hsla, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Rgba,
    SharedString, Styled, TextRun, WeakEntity, Window, div, fill, point, prelude::*, px, rems, rgb,
    rgba, size,
};

use crate::consts::CARET_WIDTH_PX;
use crate::theme::Theme;

use super::{FieldId, PreferencesDialog, SIDEBAR_MIN_WIDTH_PX, SidebarDrag};

// --- helpers -------------------------------------------------------------

/// Force every open Workspace window to re-render. Called from Preferences
/// toggles (Status Bar Hide, etc.) so the change in `settings::snapshot()`
/// flows through to the next paint without waiting for unrelated user input.
pub(super) fn nudge_workspaces(cx: &mut App) {
    for handle in cx.windows() {
        if let Some(ws_handle) = handle.downcast::<crate::workspace::Workspace>() {
            let _ = ws_handle.update(cx, |_, _, cx| cx.notify());
        }
    }
}

pub(super) fn prev_grapheme(s: &str, byte: usize) -> usize {
    if byte == 0 {
        return 0;
    }
    let mut i = byte - 1;
    while !s.is_char_boundary(i) && i > 0 {
        i -= 1;
    }
    i
}

pub(super) fn next_grapheme(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let mut i = byte + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub(super) fn move_caret_or_select<F>(f: &mut super::FieldState, target: F, extend: bool)
where
    F: FnOnce(&str, usize) -> usize,
{
    let new = target(&f.text, f.cursor);
    if extend {
        f.cursor = new;
    } else {
        f.cursor = new;
        f.anchor = new;
    }
}

fn byte_for_x(text: &str, x: Pixels, window: &mut Window) -> usize {
    if text.is_empty() {
        return 0;
    }
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let display: SharedString = text.to_string().into();
    let runs = vec![TextRun {
        len: display.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let line = window
        .text_system()
        .shape_line(display, font_size, &runs, None);
    let local = if x < px(0.) { px(0.) } else { x };
    line.closest_index_for_x(local).min(line.len)
}

// --- NPP-style checkbox --------------------------------------------------

/// NPP-style checkbox — square outline, filled accent square when checked.
/// `on_toggle` receives the new checked value plus the App context, so the
/// callback can both mutate settings and nudge workspaces. Caller supplies
/// a stable element id for hover/active styling.
pub(super) fn npp_checkbox<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    checked: bool,
    on_toggle: F,
) -> gpui::AnyElement
where
    F: Fn(bool, &mut App) + 'static,
{
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

    // Click target sized to content (mark + label). Wrapped in an outer
    // flex_row so even when a flex_col parent stretches the wrapper to
    // full column width, the click handler stays bound to the inner row
    // (mark + gap + label) only — clicks in the empty space to the right
    // no longer fire the toggle.
    let click_target = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            on_toggle(!checked, cx);
            nudge_workspaces(cx);
        })
        .child(mark)
        .child(
            div()
                .text_color(rgb(theme.text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(label)),
        );

    div().flex().flex_row().child(click_target).into_any_element()
}

// --- NPP-style radio button ---------------------------------------------

/// NPP-style radio — circle outline, filled accent dot when selected. Drives
/// a `bool` "this is the chosen option" — caller is responsible for ensuring
/// only one radio in a group is `selected = true`. `on_select` fires when
/// the user clicks an unselected radio.
pub(super) fn npp_radio<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    selected: bool,
    on_select: F,
) -> gpui::AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let outer_size = rems(13. / 16.);
    let inner_size = rems(7. / 16.);
    // Half of `outer_size` keeps the rim a circle even at extreme zooms;
    // expressing it in rems ties the radius to the same scaling pipeline
    // used by every other dialog element.
    let outer_radius = rems(13. / 32.);
    let inner_radius = rems(7. / 32.);

    let mark = div()
        .w(outer_size)
        .h(outer_size)
        .border_1()
        .border_color(rgb(theme.border))
        .rounded(outer_radius)
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |this| {
            this.child(
                div()
                    .w(inner_size)
                    .h(inner_size)
                    .rounded(inner_radius)
                    .bg(rgb(theme.checkbox_fill)),
            )
        });

    // Click target sized to content (mark + label). Wrapped in an outer
    // flex_row so even when a flex_col parent stretches the wrapper to
    // full column width, the click handler stays bound to the inner row
    // (mark + gap + label) only — clicks beyond the label never fire the
    // selection.
    let click_target = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            if !selected {
                on_select(cx);
                nudge_workspaces(cx);
            }
        })
        .child(mark)
        .child(
            div()
                .text_color(rgb(theme.text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(label)),
        );

    div().flex().flex_row().child(click_target).into_any_element()
}

// --- disabled variants ---------------------------------------------------

/// Visual placeholder radio — same NPP look but greyed and non-interactive.
/// Used by Dark Mode's Tones group when the active theme mode is not
/// `FollowSystem` (NPP's parity rule: tones only meaningful when the OS
/// drives appearance).
pub(super) fn npp_radio_disabled(
    theme: &Theme,
    label: &'static str,
    selected: bool,
) -> gpui::AnyElement {
    let outer_size = rems(13. / 16.);
    let inner_size = rems(7. / 16.);
    let outer_radius = rems(13. / 32.);
    let inner_radius = rems(7. / 32.);

    let mark = div()
        .w(outer_size)
        .h(outer_size)
        .border_1()
        .border_color(rgb(theme.disabled_text))
        .rounded(outer_radius)
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |this| {
            this.child(
                div()
                    .w(inner_size)
                    .h(inner_size)
                    .rounded(inner_radius)
                    .bg(rgb(theme.disabled_text)),
            )
        });

    // Same content-sized layout as the live `npp_radio` so disabled and
    // enabled rows align on the same grid even though the disabled
    // variant has no click handler.
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(mark)
        .child(
            div()
                .text_color(rgb(theme.disabled_text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(label)),
        );

    div().flex().flex_row().child(row).into_any_element()
}

/// Visual placeholder checkbox — same NPP look as `npp_checkbox` but greyed
/// and non-interactive. Used when a master toggle gates the sub-control
/// (e.g. Performance ▸ `Disable syntax highlighting` is gated by
/// `Enable Large File Restriction`).
pub(super) fn npp_checkbox_disabled(
    theme: &Theme,
    label: &'static str,
    checked: bool,
) -> gpui::AnyElement {
    let box_size = rems(13. / 16.);
    let inner_size = rems(7. / 16.);

    let mark = div()
        .w(box_size)
        .h(box_size)
        .border_1()
        .border_color(rgb(theme.disabled_text))
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |this| {
            this.child(
                div()
                    .w(inner_size)
                    .h(inner_size)
                    .bg(rgb(theme.disabled_text)),
            )
        });

    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(mark)
        .child(
            div()
                .text_color(rgb(theme.disabled_text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(label)),
        );

    div().flex().flex_row().child(row).into_any_element()
}

/// Visual placeholder button — NPP look, greyed and non-interactive. Used
/// for the Tones group's `Reset` split-button slot until palette infra ships.
pub(super) fn npp_button_disabled(
    theme: &Theme,
    label: &'static str,
) -> gpui::AnyElement {
    div()
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.disabled_text))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.disabled_text))
        .child(SharedString::from(label))
        .into_any_element()
}

/// Live NPP-style button. Same shape/typography as `npp_button_disabled`
/// but interactive — hover / pressed states + click handler. Used by the
/// Dark Mode panel's `Reset` button to clear customized-slot overrides.
pub(super) fn npp_button<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    on_click: F,
) -> gpui::AnyElement
where
    F: Fn(&mut App) + 'static,
{
    div()
        .id(id)
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            on_click(cx);
            nudge_workspaces(cx);
        })
        .into_any_element()
}

// --- NPP-style color swatch ---------------------------------------------

/// NPP-style customized-slot swatch: a small filled square preview of the
/// slot's current color, with the slot label to the right. When `enabled`,
/// clicking opens the color picker via `on_click`. When disabled, no click
/// handler is attached and the swatch is rendered with a desaturated
/// border + dim label so the user can see the slot's preset color but
/// can't interact with it.
pub(super) fn npp_color_swatch<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    color: u32,
    enabled: bool,
    on_click: F,
) -> gpui::AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let swatch_size = rems(18. / 16.);

    let mut sw = div()
        .w(swatch_size)
        .h(swatch_size)
        .bg(rgb(color))
        .border_1();
    sw = if enabled {
        sw.border_color(rgb(theme.border))
    } else {
        sw.border_color(rgb(theme.disabled_text))
    };

    let label_color = if enabled {
        theme.text
    } else {
        theme.disabled_text
    };

    let label_div = div()
        .text_color(rgb(label_color))
        .text_size(rems(12. / 16.))
        .child(SharedString::from(label));

    if !enabled {
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(rems(8. / 16.))
            .child(sw)
            .child(label_div);
        return div().flex().flex_row().child(row).into_any_element();
    }

    // Click target sized to content (swatch + label). Wrapped in an outer
    // flex_row so when the parent grid cell stretches the wrapper to
    // fill its column, the click handler stays bound to the inner row
    // only — clicking the empty padding inside the cell no longer fires
    // a swatch open and so cannot accidentally hit a neighbouring tone
    // radio either.
    let click_target = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            on_click(cx);
        })
        .child(sw)
        .child(label_div);

    div().flex().flex_row().child(click_target).into_any_element()
}

// --- group box -----------------------------------------------------------

/// NPP-style group box: bordered frame with the title sitting on the top
/// edge of the border. The chip lives on a *sibling* layer (not a child of
/// the bordered box) so the bordered box paints completely first — border
/// and all — and the chip then paints on top, occluding the border line
/// behind the legend text. Painting child-first vs. sibling-after is the
/// fix; the previous "background-punch" approach tried to occlude the
/// border with a chip-sized bg fill but lost when gpui paints borders
/// after a parent's children.
pub(super) fn render_group_box(
    theme: &Theme,
    title: &'static str,
    children: Vec<gpui::AnyElement>,
) -> impl IntoElement {
    let chip_h = rems(14. / 16.);

    // Bordered content lives on its own; this is what the chip sits on top
    // of. Padding `pt` is `chip_h / 2 + content padding` so the first row
    // doesn't crash into the legend.
    let mut bordered = div()
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .px(rems(14. / 16.))
        .pt(rems(14. / 16.))
        .pb(rems(12. / 16.))
        .flex()
        .flex_col()
        .gap(rems(8. / 16.));
    for c in children {
        bordered = bordered.child(c);
    }

    let title_chip = div()
        .absolute()
        .top(-rems(7. / 16.))
        .left(rems(14. / 16.))
        .h(chip_h)
        .px(rems(6. / 16.))
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(title));

    div().relative().child(bordered).child(title_chip)
}

// --- deferral panel ------------------------------------------------------

/// Plain-text panel shown for NPP categories whose backing features don't
/// exist yet in Notepad Studio. Currently unused — every panel ships either a real
/// UI or its own bespoke "deferred" message — kept around so a future stub
/// panel has a one-liner shortcut.
#[allow(dead_code)]
pub(super) fn render_deferral(theme: &Theme, feature: &'static str) -> gpui::AnyElement {
    div()
        .flex_1()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(rems(40. / 16.))
        .child(
            div()
                .max_w(rems(420. / 16.))
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from(format!(
                    "This panel will be populated when {feature} is added to Notepad Studio."
                ))),
        )
        .into_any_element()
}

// --- labelled text field row --------------------------------------------

/// Label + text input + Reset button on a horizontal row. The input is a
/// `DialogTextInput` element wired to the dialog's IME plumbing; the Reset
/// button restores the field's default value via `PreferencesDialog::reset_field`.
pub(super) fn render_field(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: FieldId,
    label: &'static str,
    input_width: gpui::Rems,
) -> gpui::AnyElement {
    let input = DialogTextInput {
        dialog: weak.clone(),
        field_id: id,
    };

    let entity_for_reset = weak.clone();
    let reset_id = match id {
        FieldId::TabWidth => "prefs-reset-tab-width",
        FieldId::SplitColumn => "prefs-reset-split-column",
        FieldId::DateTimeFormat => "prefs-reset-datetime-format",
        FieldId::RecentFilesMax => "prefs-reset-recent-files-max",
        FieldId::CaretBlinkRateMs => "prefs-reset-caret-blink-rate",
        FieldId::DefaultDirectoryCustom => "prefs-reset-default-directory-custom",
        FieldId::LargeFileThresholdMb => "prefs-reset-large-file-threshold-mb",
        FieldId::SearchEngineCustomUrl => "prefs-reset-search-engine-custom-url",
        FieldId::BackupPeriodicIntervalMs => "prefs-reset-backup-interval-ms",
        FieldId::BackupCustomDir => "prefs-reset-backup-custom-dir",
        FieldId::PrintMarginTopMm => "prefs-reset-print-margin-top",
        FieldId::PrintMarginLeftMm => "prefs-reset-print-margin-left",
        FieldId::PrintMarginRightMm => "prefs-reset-print-margin-right",
        FieldId::PrintMarginBottomMm => "prefs-reset-print-margin-bottom",
        FieldId::PrintHeaderLeft => "prefs-reset-print-header-left",
        FieldId::PrintHeaderMiddle => "prefs-reset-print-header-middle",
        FieldId::PrintHeaderRight => "prefs-reset-print-header-right",
        FieldId::PrintHeaderFontName => "prefs-reset-print-header-font-name",
        FieldId::PrintHeaderFontSize => "prefs-reset-print-header-font-size",
        FieldId::PrintFooterLeft => "prefs-reset-print-footer-left",
        FieldId::PrintFooterMiddle => "prefs-reset-print-footer-middle",
        FieldId::PrintFooterRight => "prefs-reset-print-footer-right",
        FieldId::PrintFooterFontName => "prefs-reset-print-footer-font-name",
        FieldId::PrintFooterFontSize => "prefs-reset-print-footer-font-size",
        FieldId::AutocompletionFromChars => "prefs-reset-autocompletion-from-chars",
    };
    let reset_btn = div()
        .id(reset_id)
        .ml(rems(8. / 16.))
        .px(rems(8. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child("Reset")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = entity_for_reset.update(cx, |this, cx| {
                this.reset_field(id, cx);
            });
        });

    let input_box = div()
        .w(input_width)
        .h(rems(24. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .px(rems(6. / 16.))
        .child(input);

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .min_w(rems(170. / 16.))
                .text_color(rgb(theme.text))
                .child(SharedString::from(label)),
        )
        .child(input_box);
    if id.numeric_bounds().is_some() {
        row = row.child(spinner_column(theme, weak.clone(), id, reset_id));
    }
    row.child(reset_btn).into_any_element()
}

/// Disabled mirror of `render_field` — same row geometry, but the input is
/// a static label of the current value, the Reset button is greyed, and the
/// spinner column (if any) renders inert. Used when a master toggle gates
/// the field (e.g. Performance threshold while Large File Restriction is off).
pub(super) fn render_field_disabled(
    theme: &Theme,
    id: FieldId,
    label: &'static str,
    value: &str,
    input_width: gpui::Rems,
) -> gpui::AnyElement {
    let display = if value.is_empty() {
        SharedString::from(" ")
    } else {
        SharedString::from(value.to_string())
    };

    let input_box = div()
        .w(input_width)
        .h(rems(24. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.disabled_text))
        .px(rems(6. / 16.))
        .flex()
        .items_center()
        .child(
            div()
                .text_color(rgb(theme.disabled_text))
                .text_size(rems(12. / 16.))
                .child(display),
        );

    let reset_btn = div()
        .ml(rems(8. / 16.))
        .px(rems(8. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.disabled_text))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.disabled_text))
        .child(SharedString::from("Reset"));

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .min_w(rems(170. / 16.))
                .text_color(rgb(theme.disabled_text))
                .child(SharedString::from(label)),
        )
        .child(input_box);
    if id.numeric_bounds().is_some() {
        row = row.child(spinner_column_disabled(theme));
    }
    row.child(reset_btn).into_any_element()
}

/// Inert mirror of `spinner_column` — same geometry, greyed, no click.
fn spinner_column_disabled(theme: &Theme) -> impl IntoElement {
    let bg_color = theme.button_bg;
    let border_color = theme.disabled_text;
    let text_color = theme.disabled_text;
    let up = div()
        .flex()
        .items_center()
        .justify_center()
        .w(rems(1.))
        .h(rems(12. / 16.))
        .bg(rgb(bg_color))
        .border_1()
        .border_color(rgb(border_color))
        .text_size(rems(9. / 16.))
        .text_color(rgb(text_color))
        .child(SharedString::from("▲"));
    let down = div()
        .flex()
        .items_center()
        .justify_center()
        .w(rems(1.))
        .h(rems(12. / 16.))
        .bg(rgb(bg_color))
        .border_1()
        .border_color(rgb(border_color))
        .text_size(rems(9. / 16.))
        .text_color(rgb(text_color))
        .child(SharedString::from("▼"));
    div().flex().flex_col().gap(px(1.)).child(up).child(down)
}

/// Stacked ↑/↓ buttons for numeric fields. Click steps the field via
/// `PreferencesDialog::step_field`, which clamps + persists.
fn spinner_column(
    theme: &Theme,
    weak: WeakEntity<PreferencesDialog>,
    id: FieldId,
    reset_id: &'static str,
) -> impl IntoElement {
    let up_id: SharedString = format!("{reset_id}-spin-up").into();
    let down_id: SharedString = format!("{reset_id}-spin-down").into();
    let bg_color = theme.button_bg;
    let hover_color = theme.button_hover_bg;
    let pressed_color = theme.button_pressed_bg;
    let border_color = theme.border;
    let text_color = theme.text;
    let weak_up = weak.clone();
    let weak_down = weak;
    let up = div()
        .id(gpui::ElementId::from(up_id))
        .flex()
        .items_center()
        .justify_center()
        .w(rems(1.))
        .h(rems(12. / 16.))
        .bg(rgb(bg_color))
        .border_1()
        .border_color(rgb(border_color))
        .text_size(rems(9. / 16.))
        .text_color(rgb(text_color))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(rgb(hover_color)))
        .active(move |s| s.bg(rgb(pressed_color)))
        .child(SharedString::from("▲"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_up.update(cx, |this, cx| {
                this.step_field(id, 1, cx);
            });
        });
    let down = div()
        .id(gpui::ElementId::from(down_id))
        .flex()
        .items_center()
        .justify_center()
        .w(rems(1.))
        .h(rems(12. / 16.))
        .bg(rgb(bg_color))
        .border_1()
        .border_color(rgb(border_color))
        .text_size(rems(9. / 16.))
        .text_color(rgb(text_color))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(rgb(hover_color)))
        .active(move |s| s.bg(rgb(pressed_color)))
        .child(SharedString::from("▼"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_down.update(cx, |this, cx| {
                this.step_field(id, -1, cx);
            });
        });
    div().flex().flex_col().gap(px(1.)).child(up).child(down)
}

// --- text input element --------------------------------------------------

pub(super) struct DialogTextInput {
    pub(super) dialog: WeakEntity<PreferencesDialog>,
    pub(super) field_id: FieldId,
}

impl IntoElement for DialogTextInput {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for DialogTextInput {
    type RequestLayoutState = ();
    type PrepaintState = Option<(FocusHandle, Option<gpui::ShapedLine>, usize, usize)>;

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
        let Some(dialog) = self.dialog.upgrade() else {
            return None;
        };
        let dialog_ref = dialog.read(cx);
        let theme = dialog_ref.theme();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let f = dialog_ref.fields.get(self.field_id);
        let handle = dialog_ref.focus_for(self.field_id);
        let text = f.text.clone();
        let cursor = f.cursor.min(text.len());
        let anchor = f.anchor.min(text.len());

        let display: SharedString = text.clone().into();
        let color: Hsla = Rgba::from(rgb(theme.text)).into();

        let shaped = if display.is_empty() {
            None
        } else {
            let runs = vec![TextRun {
                len: display.len(),
                font: text_style.font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            Some(window.text_system().shape_line(display, font_size, &runs, None))
        };

        Some((handle, shaped, cursor, anchor))
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
        let Some((handle, shaped, cursor, anchor)) = prepaint.as_mut() else {
            return;
        };
        let theme = dialog.read(cx).theme();
        let line_height = window.line_height();
        let baseline_y = bounds.top() + (bounds.size.height - line_height).max(px(0.)) / 2.0;

        // Selection rect (if any).
        let lo = (*cursor).min(*anchor);
        let hi = (*cursor).max(*anchor);
        if hi > lo {
            if let Some(line) = shaped.as_ref() {
                let lo_clamped = lo.min(line.len);
                let hi_clamped = hi.min(line.len);
                let x_lo = bounds.left() + line.x_for_index(lo_clamped);
                let x_hi = bounds.left() + line.x_for_index(hi_clamped);
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

        // Caret — only when this field is focused.
        if handle.is_focused(window) {
            let f_text = dialog
                .read(cx)
                .fields
                .get(self.field_id)
                .text
                .clone();
            let font_size = window
                .text_style()
                .font_size
                .to_pixels(window.rem_size());
            let display: SharedString = f_text.clone().into();
            let caret_color: Hsla = Rgba::from(rgb(theme.text)).into();
            let cx_pos = if display.is_empty() {
                bounds.left()
            } else {
                let runs = vec![TextRun {
                    len: display.len(),
                    font: window.text_style().font(),
                    color: caret_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let line = window
                    .text_system()
                    .shape_line(display, font_size, &runs, None);
                bounds.left() + line.x_for_index((*cursor).min(line.len))
            };
            window.paint_quad(fill(
                Bounds::new(
                    point(cx_pos, baseline_y),
                    size(px(CARET_WIDTH_PX), line_height),
                ),
                caret_color,
            ));
        }

        // IME / OS text input router.
        window.handle_input(handle, ElementInputHandler::new(bounds, dialog.clone()), cx);

        // Click-to-focus handler.
        let dialog_for_click = dialog.clone();
        let field_id = self.field_id;
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            dialog_for_click.update(cx, |this, cx| {
                let handle = this.focus_for(field_id);
                window.focus(&handle, cx);
                let f = this.fields.get_mut(field_id);
                let click_x = event.position.x - bounds.left();
                let byte = byte_for_x(&f.text, click_x, window);
                f.collapse_to(byte);
                cx.notify();
            });
        });
    }
}

// --- sidebar resize handle ----------------------------------------------

/// 1-px-thin draggable divider on the right edge of the category list.
/// Lives as a custom Element so its `paint` can register window-global
/// `MouseMoveEvent` / `MouseUpEvent` handlers — without those, the drag
/// would be cancelled the moment the cursor left the 4-px hit area.
pub(super) struct SidebarResizeHandle {
    pub(super) dialog: WeakEntity<PreferencesDialog>,
}

impl IntoElement for SidebarResizeHandle {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for SidebarResizeHandle {
    type RequestLayoutState = ();
    type PrepaintState = gpui::Hitbox;

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
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // gpui only permits hitbox insertion during request_layout/prepaint.
        window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.set_cursor_style(CursorStyle::ResizeLeftRight, hitbox);

        let viewport_w = window.viewport_size().width;
        let dialog_for_down = self.dialog.clone();

        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            let _ = dialog_for_down.update(cx, |this, cx| {
                this.sidebar_drag = Some(SidebarDrag {
                    start_mouse_x: event.position.x,
                    start_width: this.sidebar_width,
                });
                cx.notify();
            });
        });

        let dialog_for_move = self.dialog.clone();
        let max_width = (viewport_w / 2.).max(px(SIDEBAR_MIN_WIDTH_PX));
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let _ = dialog_for_move.update(cx, |this, cx| {
                let Some(drag) = this.sidebar_drag else {
                    return;
                };
                let delta = event.position.x - drag.start_mouse_x;
                let next = (drag.start_width + delta)
                    .clamp(px(SIDEBAR_MIN_WIDTH_PX), max_width);
                if next != this.sidebar_width {
                    this.sidebar_width = next;
                    cx.notify();
                }
            });
        });

        let dialog_for_up = self.dialog.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let _ = dialog_for_up.update(cx, |this, cx| {
                if this.sidebar_drag.is_some() {
                    this.sidebar_drag = None;
                    this.save_now(cx);
                }
            });
        });
    }
}

