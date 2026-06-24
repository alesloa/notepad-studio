// New Document → Default language popover dropdown.
//
// Replaces the prior cycle-button picker (clicking advanced through the list
// one entry at a time). The popover lists "(auto-detect)" plus every
// `LanguageId::all()` entry, with the current selection highlighted. Excluded
// languages (Settings ▸ Language panel) are dimmed but still selectable —
// excluding from the *cycler* and excluding from the *picker* are different
// surfaces and we don't want a user to discover their pinned default suddenly
// became unselectable because they excluded it.
//
// Anchored below the trigger button via the existing find-dialog-style
// `relative()` + `absolute()` pattern; the scrim covers the whole dialog so
// any outside click closes it cleanly without firing on the underlying
// control.

use gpui::{
    CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, deferred, div, px, rems, rgb,
};

use crate::settings;
use crate::syntax::LanguageId;
use crate::theme::Theme;

use super::PreferencesDialog;

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    current: Option<LanguageId>,
    excluded: &[LanguageId],
) -> gpui::AnyElement {
    let weak_scrim = weak.clone();
    let scrim = div()
        .id("prefs-newdoc-default-lang-scrim")
        // Window-sized rect anchored at the trigger row's box. Pinned with
        // generous negative offsets so any click anywhere in the dialog
        // outside the panel falls onto the scrim — same pattern as
        // `find_dialog::widgets::history_dropdown`.
        .absolute()
        .top(px(-9999.))
        .left(px(-9999.))
        .w(px(20000.))
        .h(px(20000.))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_scrim.update(cx, |this, cx| {
                this.close_default_lang_dropdown(cx);
            });
        });

    let mut panel = div()
        .id("prefs-newdoc-default-lang-panel")
        .absolute()
        // Trigger button is rendered at row height ~22/16rem; drop the
        // panel just below with 1/16rem breathing room.
        .top(rems(24. / 16.))
        .left(px(0.))
        .min_w(rems(220. / 16.))
        .max_h(rems(240. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .flex()
        .flex_col()
        // Block clicks from punching through to the scrim — picking a row
        // would otherwise fire both the row handler and the scrim's close
        // and the close would win on later frames.
        .occlude();

    panel = panel.child(row(theme, weak, None, current, false));
    for lang in LanguageId::all().iter().copied() {
        let is_excluded = excluded.contains(&lang);
        panel = panel.child(row(theme, weak, Some(lang), current, is_excluded));
    }

    deferred(div().absolute().child(scrim).child(panel))
        .with_priority(2)
        .into_any_element()
}

fn row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    target: Option<LanguageId>,
    current: Option<LanguageId>,
    is_excluded: bool,
) -> gpui::AnyElement {
    let selected = target == current;
    let label: SharedString = match target {
        Some(lang) => lang.label().into(),
        None => "(auto-detect by extension)".into(),
    };
    let id = match target {
        Some(lang) => SharedString::from(format!("prefs-newdoc-lang-row::{}", lang.label())),
        None => SharedString::from("prefs-newdoc-lang-row::none"),
    };
    let text_color = if is_excluded {
        theme.disabled_text
    } else {
        theme.text
    };
    let bg = if selected {
        Some(theme.button_pressed_bg)
    } else {
        None
    };

    let weak = weak.clone();
    let mut row = div()
        .id(id)
        .px(rems(10. / 16.))
        .py(rems(4. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .text_size(rems(12. / 16.))
        .text_color(rgb(text_color))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .child(label);
    if let Some(bg_color) = bg {
        row = row.bg(rgb(bg_color));
    }
    row.on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
        let mut next = settings::snapshot();
        next.default_language = target;
        settings::replace_and_save(next);
        let _ = weak.update(cx, |this, cx| {
            this.close_default_lang_dropdown(cx);
            this.last_action = Some(match target {
                Some(lang) => format!("Default language: {}", lang.label()),
                None => "Default language: (auto-detect)".to_string(),
            });
            cx.notify();
        });
    })
    .into_any_element()
}
