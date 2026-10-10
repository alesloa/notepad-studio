// Recent Files History panel — NPP `Recent Files History`.
//
// Controls:
//   * Max. number of entries (numeric, 0–30; existing field).
//   * Check existence at launch (drop missing entries on next start).
//   * Display the full path (filename-only when off — NPP default).
//   * In Submenu (flat-list under File menu when off — NPP default).
//
// Toggling the latter two rebuilds the OS File menu so the change is
// visible immediately without a relaunch.
//
// When `Max. number of entries` is 0 the MRU list is disabled wholesale
// (see `recent::max_recent`), so the 3 Display checkboxes become no-ops
// — grey them out to match the rest of the prefs panels' master/sub
// gating pattern (Performance, Backup, Editing 2 etc.).

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::app::menus::rebuild_menus;
use crate::settings;
use crate::theme::Theme;

use super::widgets::{npp_checkbox, npp_checkbox_disabled, render_field, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    let max_group = render_group_box(
        theme,
        "Max. number of entries",
        vec![render_field(
            theme,
            weak,
            FieldId::RecentFilesMax,
            "Max. number of entries:",
            rems(60. / 16.),
        )],
    );

    // List disabled when max == 0 — the 3 display checkboxes become
    // no-ops, so grey them out (NPP-parity gating).
    let list_enabled = snap.recent_files_max > 0;

    let check_at_launch = if list_enabled {
        let weak_check = weak.clone();
        npp_checkbox(
            theme,
            "prefs-recent-check-at-launch",
            "Check existence at launch (drop missing entries)",
            snap.recent_files_check_at_launch,
            move |checked, cx| {
                let _ = weak_check.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.recent_files_check_at_launch = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Recent files: check at launch enabled".to_string()
                    } else {
                        "Recent files: check at launch disabled".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Check existence at launch (drop missing entries)",
            snap.recent_files_check_at_launch,
        )
    };

    let show_full_path = if list_enabled {
        let weak_full = weak.clone();
        npp_checkbox(
            theme,
            "prefs-recent-show-full-path",
            "Display the full path",
            snap.recent_files_show_full_path,
            move |checked, cx| {
                let _ = weak_full.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.recent_files_show_full_path = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Recent files: full path".to_string()
                    } else {
                        "Recent files: filename only".to_string()
                    });
                    cx.notify();
                });
                rebuild_menus(cx);
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Display the full path",
            snap.recent_files_show_full_path,
        )
    };

    let in_submenu = if list_enabled {
        let weak_sub = weak.clone();
        npp_checkbox(
            theme,
            "prefs-recent-in-submenu",
            "In Submenu",
            snap.recent_files_in_submenu,
            move |checked, cx| {
                let _ = weak_sub.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.recent_files_in_submenu = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Recent files: in submenu".to_string()
                    } else {
                        "Recent files: flat in File menu".to_string()
                    });
                    cx.notify();
                });
                rebuild_menus(cx);
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "In Submenu",
            snap.recent_files_in_submenu,
        )
    };

    let display_group = render_group_box(
        theme,
        "Display",
        vec![check_at_launch, show_full_path, in_submenu],
    );

    body.child(
        div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(max_group)
            .child(display_group),
    )
    .child(
        div()
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .child(SharedString::from(
                "Range: 0–30. 0 disables the recent-files list.",
            )),
    )
    .into_any_element()
}
