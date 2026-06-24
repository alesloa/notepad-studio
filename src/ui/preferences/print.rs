// NPP `Preferences ▸ Print` panel — the layout matches NPP's full Print
// page (`preferenceDlg.cpp:6430-6560`):
//
//   ┌── Print line number ──────────────────────────┐
//   │ [✓] Print line number                         │
//   └───────────────────────────────────────────────┘
//   ┌── Colour mode ────────────────────────────────┐
//   │ ( ) WYSIWYG  ( ) Invert ( ) BlackOnWhite (•) ColourOnWhite │
//   └───────────────────────────────────────────────┘
//   ┌── Margins (mm) ───────────────────────────────┐
//   │ Left [ ]  Right [ ]  Top [ ]  Bottom [ ]      │
//   └───────────────────────────────────────────────┘
//   ┌── Header ─────────────────────────────────────┐
//   │ Left [______]  Center [______]  Right [______]│
//   │ Font [______]  Size [__]  [✓] Bold  [✓] Italic│
//   └───────────────────────────────────────────────┘
//   ┌── Footer ─────────────────────────────────────┐
//   │ ... same as Header                            │
//   └───────────────────────────────────────────────┘
//   $(FILE_NAME) $(SHORT_DATE) $(TIME) $(CURRENT_LINE) ...
//   Print pipeline pending — these settings persist now.
//
// All field plumbing (FieldId / FieldState / focus / commit_all / reset_field
// / cycle_focus) lives in `mod.rs`. This file only owns layout + the
// checkbox / radio click handlers.

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings::{self, PrintColorMode};
use crate::theme::Theme;

use super::widgets::{npp_checkbox, npp_radio, render_field, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    body.child(line_numbers_group(theme, weak, &snap))
        .child(color_mode_group(theme, weak, &snap))
        .child(margins_group(theme, weak))
        .child(header_group(theme, weak, &snap))
        .child(footer_group(theme, weak, &snap))
        .child(variable_hint(theme))
        .child(pipeline_caveat(theme))
        .into_any_element()
}

// --- groups --------------------------------------------------------------

fn line_numbers_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let weak_chk = weak.clone();
    let chk = npp_checkbox(
        theme,
        "prefs-print-line-numbers",
        "Print line number",
        snap.print_line_numbers,
        move |checked, cx| {
            let _ = weak_chk.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.print_line_numbers = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Print line numbers enabled".to_string()
                } else {
                    "Print line numbers disabled".to_string()
                });
                cx.notify();
            });
        },
    );
    render_group_box(theme, "Print line number", vec![chk]).into_any_element()
}

fn color_mode_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let mode = snap.print_color_mode;
    let row = div()
        .flex()
        .flex_row()
        .gap(rems(20. / 16.))
        .child(color_radio(
            theme,
            weak,
            "prefs-print-color-wysiwyg",
            "WYSIWYG",
            mode,
            PrintColorMode::Wysiwyg,
        ))
        .child(color_radio(
            theme,
            weak,
            "prefs-print-color-invert",
            "Invert",
            mode,
            PrintColorMode::InvertLight,
        ))
        .child(color_radio(
            theme,
            weak,
            "prefs-print-color-black-on-white",
            "Black on white",
            mode,
            PrintColorMode::BlackOnWhite,
        ))
        .child(color_radio(
            theme,
            weak,
            "prefs-print-color-colour-on-white",
            "Colour, no background",
            mode,
            PrintColorMode::ColourOnWhite,
        ));

    render_group_box(theme, "Colour mode", vec![row.into_any_element()])
        .into_any_element()
}

fn color_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: PrintColorMode,
    target: PrintColorMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.print_color_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!("Print colour mode: {}", color_label(target)));
            cx.notify();
        });
    })
}

fn color_label(m: PrintColorMode) -> &'static str {
    match m {
        PrintColorMode::Wysiwyg => "wysiwyg",
        PrintColorMode::InvertLight => "invert",
        PrintColorMode::BlackOnWhite => "black on white",
        PrintColorMode::ColourOnWhite => "colour on white",
    }
}

fn margins_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
) -> gpui::AnyElement {
    let field_w = rems(54. / 16.);
    let row1 = div()
        .flex()
        .flex_row()
        .gap(rems(14. / 16.))
        .child(render_field(
            theme,
            weak,
            FieldId::PrintMarginLeftMm,
            "Left:",
            field_w,
        ))
        .child(render_field(
            theme,
            weak,
            FieldId::PrintMarginRightMm,
            "Right:",
            field_w,
        ));
    let row2 = div()
        .flex()
        .flex_row()
        .gap(rems(14. / 16.))
        .child(render_field(
            theme,
            weak,
            FieldId::PrintMarginTopMm,
            "Top:",
            field_w,
        ))
        .child(render_field(
            theme,
            weak,
            FieldId::PrintMarginBottomMm,
            "Bottom:",
            field_w,
        ));

    render_group_box(
        theme,
        "Margins (mm)",
        vec![row1.into_any_element(), row2.into_any_element()],
    )
    .into_any_element()
}

fn header_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    text_block_group(
        theme,
        weak,
        snap,
        "Header",
        FieldId::PrintHeaderLeft,
        FieldId::PrintHeaderMiddle,
        FieldId::PrintHeaderRight,
        FieldId::PrintHeaderFontName,
        FieldId::PrintHeaderFontSize,
        snap.print_header_font_bold,
        snap.print_header_font_italic,
        "prefs-print-header-bold",
        "prefs-print-header-italic",
        true,
    )
}

fn footer_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    text_block_group(
        theme,
        weak,
        snap,
        "Footer",
        FieldId::PrintFooterLeft,
        FieldId::PrintFooterMiddle,
        FieldId::PrintFooterRight,
        FieldId::PrintFooterFontName,
        FieldId::PrintFooterFontSize,
        snap.print_footer_font_bold,
        snap.print_footer_font_italic,
        "prefs-print-footer-bold",
        "prefs-print-footer-italic",
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn text_block_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    _snap: &settings::Settings,
    title: &'static str,
    left_id: FieldId,
    middle_id: FieldId,
    right_id: FieldId,
    font_name_id: FieldId,
    font_size_id: FieldId,
    bold: bool,
    italic: bool,
    bold_id: &'static str,
    italic_id: &'static str,
    is_header: bool,
) -> gpui::AnyElement {
    let text_w = rems(120. / 16.);
    let row1 = div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(render_field(theme, weak, left_id, "Left:", text_w))
        .child(render_field(theme, weak, middle_id, "Centre:", text_w))
        .child(render_field(theme, weak, right_id, "Right:", text_w));

    let weak_bold = weak.clone();
    let bold_chk = npp_checkbox(theme, bold_id, "Bold", bold, move |checked, cx| {
        let _ = weak_bold.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            if is_header {
                next.print_header_font_bold = checked;
            } else {
                next.print_footer_font_bold = checked;
            }
            settings::replace_and_save(next);
            this.last_action = Some(format!(
                "{} bold {}",
                if is_header { "Header" } else { "Footer" },
                if checked { "on" } else { "off" }
            ));
            cx.notify();
        });
    });

    let weak_italic = weak.clone();
    let italic_chk = npp_checkbox(theme, italic_id, "Italic", italic, move |checked, cx| {
        let _ = weak_italic.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            if is_header {
                next.print_header_font_italic = checked;
            } else {
                next.print_footer_font_italic = checked;
            }
            settings::replace_and_save(next);
            this.last_action = Some(format!(
                "{} italic {}",
                if is_header { "Header" } else { "Footer" },
                if checked { "on" } else { "off" }
            ));
            cx.notify();
        });
    });

    let row2 = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(12. / 16.))
        .child(render_field(theme, weak, font_name_id, "Font:", text_w))
        .child(render_field(theme, weak, font_size_id, "Size:", rems(54. / 16.)))
        .child(bold_chk)
        .child(italic_chk);

    render_group_box(
        theme,
        title,
        vec![row1.into_any_element(), row2.into_any_element()],
    )
    .into_any_element()
}

fn variable_hint(theme: &Theme) -> gpui::AnyElement {
    let hint = "Variables: $(FULL_CURRENT_PATH) $(CURRENT_DIRECTORY) \
                $(FILE_NAME) $(NAME_PART) $(EXT_PART) $(CURRENT_WORD) \
                $(CURRENT_LINE) $(CURRENT_COLUMN) $(NPP_DIRECTORY) \
                $(NPP_FULL_FILE_PATH) $(SHORT_DATE) $(LONG_DATE) $(TIME)";
    div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(hint))
        .into_any_element()
}

fn pipeline_caveat(theme: &Theme) -> gpui::AnyElement {
    div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(
            "Print pipeline pending — these settings persist now, will apply when print is wired.",
        ))
        .into_any_element()
}
