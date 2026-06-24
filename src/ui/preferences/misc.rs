// Misc panel — Title Bar / Save / Document Switcher groups. Document
// Switcher has a master `Enable` plus a sub-toggle `Enable MRU
// behaviour`; when the master is off the Ctrl+TAB switcher never
// renders so the MRU sub-toggle is a silent no-op — grey it out to
// match the rest of the prefs panels' master/sub gating pattern.

use gpui::{IntoElement, ParentElement, WeakEntity};

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

    let weak_for_filename = weak.clone();
    let title_bar_group = render_group_box(
        theme,
        "Title Bar",
        vec![npp_checkbox(
            theme,
            "prefs-misc-title-filename-only",
            "Show only filename in title bar",
            snap.title_bar_filename_only,
            move |checked, cx| {
                let _ = weak_for_filename.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.title_bar_filename_only = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Title bar shows filename only".to_string()
                    } else {
                        "Title bar shows full path".to_string()
                    });
                    cx.notify();
                });
            },
        )],
    );

    let weak_for_save_all = weak.clone();
    let save_all_group = render_group_box(
        theme,
        "Save",
        vec![npp_checkbox(
            theme,
            "prefs-misc-save-all-confirm",
            "Enable Save All confirm dialog",
            snap.save_all_confirm,
            move |checked, cx| {
                let _ = weak_for_save_all.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.save_all_confirm = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Save All confirm dialog enabled".to_string()
                    } else {
                        "Save All confirm dialog disabled".to_string()
                    });
                    cx.notify();
                });
            },
        )],
    );

    let switcher_on = snap.document_switcher_enabled;

    let weak_for_switcher = weak.clone();
    let switcher_enable = npp_checkbox(
        theme,
        "prefs-misc-document-switcher-enabled",
        "Enable",
        switcher_on,
        move |checked, cx| {
            let _ = weak_for_switcher.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.document_switcher_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Document switcher enabled".to_string()
                } else {
                    "Document switcher disabled".to_string()
                });
                cx.notify();
            });
        },
    );

    let switcher_mru = if switcher_on {
        let weak_for_mru = weak.clone();
        npp_checkbox(
            theme,
            "prefs-misc-document-switcher-mru",
            "Enable MRU behaviour",
            snap.document_switcher_mru,
            move |checked, cx| {
                let _ = weak_for_mru.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.document_switcher_mru = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Ctrl+TAB uses MRU order".to_string()
                    } else {
                        "Ctrl+TAB uses positional order".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(theme, "Enable MRU behaviour", snap.document_switcher_mru)
    };

    let document_switcher_group = render_group_box(
        theme,
        "Document Switcher (Ctrl+TAB)",
        vec![switcher_enable, switcher_mru],
    );

    body.child(title_bar_group)
        .child(save_all_group)
        .child(document_switcher_group)
        .into_any_element()
}
