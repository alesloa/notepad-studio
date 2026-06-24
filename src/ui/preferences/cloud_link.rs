// Cloud & Link panel — NPP `Cloud & Link Settings`.
//
// Two groups:
//   * "Clickable Link Settings" — parent toggle (URL underline + Cmd-click)
//     plus indented "No underline" sub-toggle. Drives
//     `settings::clickable_urls_enabled` and `clickable_urls_no_underline`.
//   * "Cloud" — placeholder block. NPP integrates with vendor sync (Dropbox,
//     OneDrive, etc.); Notepad Studio does not. Renders an explanatory note rather
//     than a fake control.

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings;
use crate::theme::Theme;

use super::PreferencesDialog;
use super::widgets::{npp_checkbox, npp_checkbox_disabled, render_group_box};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    let weak_clickable = weak.clone();
    let parent_enabled = snap.clickable_urls_enabled;

    let parent_check = npp_checkbox(
        theme,
        "prefs-clickable-urls",
        "Enable Clickable Link",
        parent_enabled,
        move |checked, cx| {
            let _ = weak_clickable.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.clickable_urls_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Clickable URLs enabled".to_string()
                } else {
                    "Clickable URLs disabled".to_string()
                });
                cx.notify();
            });
        },
    );

    // NPP-style indented sub-checkbox. Greys out when the parent toggle is
    // off — clicking has nothing to act on, so the live variant would be
    // misleading.
    let no_underline_row = if parent_enabled {
        let weak_no_under = weak.clone();
        div()
            .pl(rems(28. / 16.))
            .child(npp_checkbox(
                theme,
                "prefs-clickable-urls-no-underline",
                "No underline",
                snap.clickable_urls_no_underline,
                move |checked, cx| {
                    let _ = weak_no_under.update(cx, |this, cx| {
                        let mut next = settings::snapshot();
                        next.clickable_urls_no_underline = checked;
                        settings::replace_and_save(next);
                        this.last_action = Some(if checked {
                            "Clickable URL underline hidden".to_string()
                        } else {
                            "Clickable URL underline shown".to_string()
                        });
                        cx.notify();
                    });
                },
            ))
            .into_any_element()
    } else {
        div()
            .pl(rems(28. / 16.))
            .child(npp_checkbox_disabled(
                theme,
                "No underline",
                snap.clickable_urls_no_underline,
            ))
            .into_any_element()
    };

    let clickable_group = render_group_box(
        theme,
        "Clickable Link Settings",
        vec![parent_check, no_underline_row],
    );

    let cloud_group = render_group_box(
        theme,
        "Cloud",
        vec![
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(
                    "Cloud sync is not yet implemented in Notepad Studio.",
                ))
                .into_any_element(),
        ],
    );

    body.child(
        div()
            .flex()
            .flex_col()
            .gap(rems(16. / 16.))
            .child(clickable_group)
            .child(cloud_group),
    )
    .child(
        div()
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .child(SharedString::from(
                "URLs (http://, https://, ftp://, file://) become underlined when clickable links are on. Cmd+click opens in your default browser. Turn on No underline to drop the line while keeping the link colour.",
            )),
    )
    .into_any_element()
}
