// Searching panel — only the two replace-related toggles are wired today.
// NPP exposes a much larger surface (Find dialog defaults, monospaced font,
// auto-fill behaviour, etc.) that doesn't have backing in Notepad Studio yet.

use gpui::{IntoElement, ParentElement, WeakEntity};

use crate::settings;
use crate::theme::Theme;

use super::PreferencesDialog;
use super::widgets::{npp_checkbox, render_group_box};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    let weak_dont_move = weak.clone();
    let dont_move = npp_checkbox(
        theme,
        "prefs-searching-dont-move",
        "Replace: Don't move to the following occurrence",
        snap.replace_dont_move_to_next,
        move |checked, cx| {
            let _ = weak_dont_move.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.replace_dont_move_to_next = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Replace: stay on current match".to_string()
                } else {
                    "Replace: move to next match".to_string()
                });
                cx.notify();
            });
        },
    );

    let weak_confirm = weak.clone();
    let confirm = npp_checkbox(
        theme,
        "prefs-searching-confirm-replace-all",
        "Confirm Replace All in All Opened Documents",
        snap.confirm_replace_all_in_all_docs,
        move |checked, cx| {
            let _ = weak_confirm.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.confirm_replace_all_in_all_docs = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Replace All in All: confirm enabled".to_string()
                } else {
                    "Replace All in All: confirm disabled".to_string()
                });
                cx.notify();
            });
        },
    );

    body.child(render_group_box(theme, "Replace", vec![dont_move, confirm]))
        .into_any_element()
}
