// Editing 1 panel — Caret Settings → Blink rate, plus the right-column
// `Enable scrolling beyond last line` toggle. The other NPP groups (Current
// Line Indicator, Caret Width, Line Wrap, the eight remaining right-column
// toggles) all back features Notepad Studio doesn't have yet — kept off-screen
// per the no-fake-controls rule.

use gpui::{IntoElement, ParentElement, WeakEntity, rems};

use crate::settings;
use crate::theme::Theme;

use super::widgets::{npp_checkbox, render_field, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let weak_for_scroll = weak.clone();
    let weak_for_copy_cut = weak.clone();
    let weak_for_dnd = weak.clone();
    let weak_for_current_line = weak.clone();
    body.child(render_group_box(
        theme,
        "Caret Settings",
        vec![render_field(
            theme,
            weak,
            FieldId::CaretBlinkRateMs,
            "Blink rate (ms):",
            rems(60. / 16.),
        )],
    ))
    .child(npp_checkbox(
        theme,
        "prefs-editing1-scroll-beyond-last-line",
        "Enable scrolling beyond last line",
        snap.scroll_beyond_last_line,
        move |checked, cx| {
            let _ = weak_for_scroll.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.scroll_beyond_last_line = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Scroll beyond last line enabled".to_string()
                } else {
                    "Scroll beyond last line disabled".to_string()
                });
                cx.notify();
            });
        },
    ))
    .child(npp_checkbox(
        theme,
        "prefs-editing1-copy-cut-line",
        "Enable Copy/Cut Line without selection",
        snap.copy_cut_line_without_selection,
        move |checked, cx| {
            let _ = weak_for_copy_cut.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.copy_cut_line_without_selection = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Copy/Cut Line without selection: on".to_string()
                } else {
                    "Copy/Cut Line without selection: off".to_string()
                });
                cx.notify();
            });
        },
    ))
    .child(npp_checkbox(
        theme,
        "prefs-editing1-disable-dnd",
        "Disable selected text drag-and-drop",
        snap.disable_selected_text_drag_drop,
        move |checked, cx| {
            let _ = weak_for_dnd.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.disable_selected_text_drag_drop = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Selected-text drag-and-drop disabled".to_string()
                } else {
                    "Selected-text drag-and-drop enabled".to_string()
                });
                cx.notify();
            });
        },
    ))
    .child(npp_checkbox(
        theme,
        "prefs-editing1-highlight-current-line",
        "Highlight current line",
        snap.highlight_current_line,
        move |checked, cx| {
            let _ = weak_for_current_line.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.highlight_current_line = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Highlight current line: on".to_string()
                } else {
                    "Highlight current line: off".to_string()
                });
                cx.notify();
            });
        },
    ))
    .into_any_element()
}
