// General panel — matches the user-supplied NPP screenshot. Localization
// and Menu groups are omitted (no i18n, native menus across platforms).
// Toolbar group ships only the Hide checkbox; NPP's icon-set radios and
// colorization rows are skipped because Notepad Studio ships a single icon theme
// with theme-driven coloring (no alternate sets to switch). Status Bar
// Hide, Toolbar Hide, and the Tab Bar group are the implemented controls;
// the few NPP Tab Bar options without Notepad Studio backing (Multi-line,
// Vertical, Alternate icons) stay out of the dialog so we never ship
// checkboxes that don't drive real behavior.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

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

    // --- Status Bar group (left column) ---------------------------------
    let weak_status = weak.clone();
    let status_bar_group = render_group_box(
        theme,
        "Status Bar",
        vec![npp_checkbox(
            theme,
            "prefs-general-status-bar-hide",
            "Hide",
            snap.status_bar_hidden,
            move |checked, cx| {
                let _ = weak_status.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.status_bar_hidden = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Status bar hidden".to_string()
                    } else {
                        "Status bar shown".to_string()
                    });
                    cx.notify();
                });
            },
        )],
    );

    // --- Toolbar group (left column, below Status Bar) ------------------
    let weak_toolbar = weak.clone();
    let toolbar_group = render_group_box(
        theme,
        "Toolbar",
        vec![npp_checkbox(
            theme,
            "prefs-general-toolbar-hide",
            "Hide",
            snap.toolbar_hidden,
            move |checked, cx| {
                let _ = weak_toolbar.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.toolbar_hidden = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Toolbar hidden".to_string()
                    } else {
                        "Toolbar shown".to_string()
                    });
                    cx.notify();
                });
            },
        )],
    );

    // --- Tab Bar group (right column) -----------------------------------
    let tab_bar_group = render_group_box(theme, "Tab Bar", tab_bar_checkboxes(theme, weak, &snap));

    body.child(
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(rems(16. / 16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(16. / 16.))
                    .child(status_bar_group)
                    .child(toolbar_group),
            )
            .child(div().flex().flex_col().min_w(rems(260. / 16.)).child(tab_bar_group)),
    )
    .into_any_element()
}

fn tab_bar_checkboxes(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> Vec<gpui::AnyElement> {
    // NPP setEnableForTabBar (preferenceDlg.cpp): the `Hide` master greys
    // every other tab-bar option — none of them apply to a hidden strip.
    let live = !snap.tab_bar_hidden;

    vec![
        bool_toggle(
            theme,
            weak,
            "prefs-general-tab-bar-hide",
            "Hide",
            snap.tab_bar_hidden,
            "Tab bar hidden",
            "Tab bar shown",
            |s, v| s.tab_bar_hidden = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-reduce",
            "Reduce",
            snap.tab_reduce,
            "Tab bar height reduced",
            "Tab bar height restored",
            |s, v| s.tab_reduce = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-lock-drag",
            "Lock (no drag and drop)",
            snap.tab_lock_drag,
            "Tab drag-reorder locked",
            "Tab drag-reorder unlocked",
            |s, v| s.tab_lock_drag = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-multi-line",
            "Multi-line",
            snap.tab_multi_line,
            "Tab bar wraps to multiple rows",
            "Tab bar stays a single row",
            |s, v| s.tab_multi_line = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-vertical",
            "Vertical",
            snap.tab_vertical,
            "Tab bar shown as a left-side column",
            "Tab bar shown as a horizontal strip above the editor",
            |s, v| s.tab_vertical = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-darken-inactive",
            "Darken inactive tabs",
            snap.tab_darken_inactive,
            "Inactive tabs darkened",
            "Inactive tabs share active background",
            |s, v| s.tab_darken_inactive = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-active-bar",
            "Draw a coloured bar on active tab",
            snap.tab_active_bar,
            "Active-tab bar shown",
            "Active-tab bar hidden",
            |s, v| s.tab_active_bar = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-close-button",
            "Show close button on each tab",
            snap.tab_close_button_visible,
            "Tab close buttons shown",
            "Tab close buttons hidden",
            |s, v| s.tab_close_button_visible = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-double-click-close",
            "Double click to close document",
            snap.tab_double_click_to_close,
            "Double-click closes tabs",
            "Double-click does nothing on tabs",
            |s, v| s.tab_double_click_to_close = v,
        ),
        bool_toggle_gated(
            theme,
            weak,
            live,
            "prefs-general-tab-exit-last-close",
            "Exit on close the last tab",
            snap.tab_exit_on_last_close,
            "Window closes on last tab",
            "Window stays open with fresh blank tab",
            |s, v| s.tab_exit_on_last_close = v,
        ),
    ]
}

fn bool_toggle(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    checked: bool,
    on_msg: &'static str,
    off_msg: &'static str,
    set: fn(&mut settings::Settings, bool),
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_checkbox(theme, id, label, checked, move |checked, cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            set(&mut next, checked);
            settings::replace_and_save(next);
            this.last_action = Some(
                if checked { on_msg } else { off_msg }.to_string(),
            );
            cx.notify();
        });
    })
}

fn bool_toggle_gated(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    live: bool,
    id: &'static str,
    label: &'static str,
    checked: bool,
    on_msg: &'static str,
    off_msg: &'static str,
    set: fn(&mut settings::Settings, bool),
) -> gpui::AnyElement {
    if live {
        bool_toggle(theme, weak, id, label, checked, on_msg, off_msg, set)
    } else {
        super::widgets::npp_checkbox_disabled(theme, label, checked)
    }
}
