// Margins/Border/Edge panel — the NPP categories that have backing in
// Notepad Studio today: the Vertical Edge wrap-column input (used by `Edit ▸ Line
// Operations ▸ Split Lines`), the line-number toggle (gates the gutter
// label paint), the Dynamic/Constant line-number width radio pair (controls
// whether the gutter widens with digit count), and the bookmark-glyph toggle
// (gates the F2 marker paint).
//
// Other NPP groups on this panel (Fold Margin Style, Border Width, Change
// History, Padding) need backing features Notepad Studio doesn't have yet —
// omitted per the no-fake-controls rule.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, prelude::FluentBuilder, rems};

use crate::settings;
use crate::theme::Theme;

use super::widgets::{npp_checkbox, npp_radio, render_field, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    let edge_group = render_group_box(
        theme,
        "Vertical Edge Settings",
        vec![render_field(
            theme,
            weak,
            FieldId::SplitColumn,
            "Edge column:",
            rems(60. / 16.),
        )],
    );

    let weak_lineno = weak.clone();
    let weak_dyn = weak.clone();
    let weak_const = weak.clone();
    let dyn_width = snap.line_number_width_dynamic;
    let line_number_group = render_group_box(
        theme,
        "Line Number",
        vec![
            npp_checkbox(
                theme,
                "prefs-margins-line-numbers",
                "Display",
                snap.show_line_numbers,
                move |checked, cx| {
                    let _ = weak_lineno.update(cx, |this, cx| {
                        let mut next = settings::snapshot();
                        next.show_line_numbers = checked;
                        settings::replace_and_save(next);
                        this.last_action = Some(if checked {
                            "Line numbers shown".to_string()
                        } else {
                            "Line numbers hidden".to_string()
                        });
                        cx.notify();
                    });
                },
            ),
            // Dynamic/Constant width is NPP's `IDC_RADIO_LNDYNAMICWIDTH` /
            // `IDC_RADIO_LNCONSTANTWIDTH` pair (preferenceDlg.cpp ~5400).
            // Single boolean backs both — `dyn_width` true = Dynamic, false = Constant.
            div()
                .flex()
                .flex_col()
                .gap(rems(4. / 16.))
                .pl(rems(20. / 16.))
                .when(!snap.show_line_numbers, |this: gpui::Div| this.opacity(0.5))
                .child(npp_radio(
                    theme,
                    "prefs-margins-lineno-dynamic",
                    "Dynamic width",
                    dyn_width,
                    move |cx| {
                        let _ = weak_dyn.update(cx, |this, cx| {
                            let mut next = settings::snapshot();
                            next.line_number_width_dynamic = true;
                            settings::replace_and_save(next);
                            this.last_action = Some("Gutter: dynamic width".to_string());
                            cx.notify();
                        });
                    },
                ))
                .child(npp_radio(
                    theme,
                    "prefs-margins-lineno-constant",
                    "Constant width",
                    !dyn_width,
                    move |cx| {
                        let _ = weak_const.update(cx, |this, cx| {
                            let mut next = settings::snapshot();
                            next.line_number_width_dynamic = false;
                            settings::replace_and_save(next);
                            this.last_action = Some("Gutter: constant width".to_string());
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
        ],
    );

    let weak_bookmark = weak.clone();
    let bookmark_checkbox = npp_checkbox(
        theme,
        "prefs-margins-bookmarks",
        "Display bookmark",
        snap.show_bookmark_glyph,
        move |checked, cx| {
            let _ = weak_bookmark.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.show_bookmark_glyph = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Bookmark glyphs shown".to_string()
                } else {
                    "Bookmark glyphs hidden".to_string()
                });
                cx.notify();
            });
        },
    );

    body.child(edge_group)
        .child(line_number_group)
        .child(bookmark_checkbox)
        .into_any_element()
}
