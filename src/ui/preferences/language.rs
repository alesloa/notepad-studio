// Language panel — NPP "Available items" / "Disabled items" mirror, but
// rendered as a single check-list since the only practical effect inside
// Notepad Studio today is to remove an entry from the new-document language
// cycler (no Language menu yet). Checked = excluded from the cycler.

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings;
use crate::syntax::LanguageId;
use crate::theme::Theme;

use super::PreferencesDialog;
use super::widgets::{npp_checkbox, render_group_box};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let excluded = snap.excluded_languages.clone();

    let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(LanguageId::all().len());
    for lang in LanguageId::all() {
        let lang = *lang;
        let is_excluded = excluded.iter().any(|l| *l == lang);
        let weak_inner = weak.clone();
        rows.push(npp_checkbox(
            theme,
            lang.pref_id(),
            lang.label(),
            is_excluded,
            move |checked, cx| {
                let _ = weak_inner.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    if checked {
                        if !next.excluded_languages.iter().any(|l| *l == lang) {
                            next.excluded_languages.push(lang);
                        }
                    } else {
                        next.excluded_languages.retain(|l| *l != lang);
                    }
                    settings::replace_and_save(next);
                    this.last_action = Some(format!(
                        "{} {}",
                        lang.label(),
                        if checked { "excluded" } else { "included" }
                    ));
                    cx.notify();
                });
            },
        ));
    }

    let grid = grid_4col(rows);

    body.child(render_group_box(theme, "Default-language exclusions", vec![grid]))
        .child(
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(
                    "Checked languages are skipped by the New Document default-language cycler. \
The Language menu remains available for selecting a document language.",
                )),
        )
        .into_any_element()
}

fn grid_4col(rows: Vec<gpui::AnyElement>) -> gpui::AnyElement {
    let cols = 4;
    let total = rows.len();
    let per_col = total.div_ceil(cols);

    let mut row_iter = rows.into_iter();
    let mut columns: Vec<gpui::AnyElement> = Vec::with_capacity(cols);
    for _ in 0..cols {
        let mut col = div().flex().flex_col().gap(rems(6. / 16.)).min_w(rems(140. / 16.));
        for _ in 0..per_col {
            if let Some(row) = row_iter.next() {
                col = col.child(row);
            }
        }
        columns.push(col.into_any_element());
    }

    let mut grid = div().flex().flex_row().gap(rems(18. / 16.));
    for c in columns {
        grid = grid.child(c);
    }
    grid.into_any_element()
}
