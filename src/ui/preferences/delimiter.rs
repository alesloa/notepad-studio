// NPP `Delimiter` panel. NPP exposes free-character entry per delimiter side
// via Win32 EDITTEXT controls (single-char) plus an "Allow on entire
// document" checkbox. Notepad Studio's settings layer accepts the same character
// pair via `delimiter_left` / `delimiter_right` strings (first char wins),
// but the dialog renders six common pairs as a click-cycle to avoid
// per-field IME plumbing for what is effectively single-byte ASCII input.
// Users who need non-standard delimiters can edit the persisted
// `~/.config/Notepad Studio/settings.toml` directly and the change still drives
// `select_between_delimiters` correctly. Caveat noted in UNTESTED.md.
//
// Layout:
//   ┌── Delimiter selection settings ──────────────────────┐
//   │ [Open: ( ]   ( … )   [Close: ) ]                     │
//   │ [ ] Allow on entire document                          │
//   └───────────────────────────────────────────────────────┘
//
// Behavior:
//   - Each delimiter button shows the current char centered. Click cycles
//     through the six standard pairs in lockstep — picking `[` on the left
//     button rotates the right to `]`. NPP allows asymmetric pairs;
//     Notepad Studio's button cycles them paired so users cannot persist a
//     mismatched ASCII pair through the UI.
//   - The middle preview shows `(`+`current_left`+` … `+`current_right`+`)`
//     so the chosen pair is visible at a glance.
//   - Match-entire-document checkbox toggles `delimiter_match_entire_document`
//     — when off, `select_between_delimiters` clamps the scan to the
//     surrounding line (matches NPP `_delimiterSelectionOnEntireDocument`).

use gpui::{
    App, CursorStyle, IntoElement, MouseButton, ParentElement, SharedString, Styled, WeakEntity,
    div, prelude::*, rems, rgb,
};

use crate::settings;
use crate::theme::Theme;

use super::PreferencesDialog;
use super::widgets::{npp_checkbox, render_group_box};

/// Standard delimiter pairs offered by the click-cycle. Index N on left
/// always pairs with index N on right.
const PAIRS: &[(&str, &str)] = &[
    ("(", ")"),
    ("[", "]"),
    ("{", "}"),
    ("<", ">"),
    ("'", "'"),
    ("\"", "\""),
];

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let current_idx = current_pair_idx(&snap.delimiter_left, &snap.delimiter_right);

    let weak_left = weak.clone();
    let left_btn = pair_button(
        theme,
        "prefs-delimiter-left",
        snap.delimiter_left.chars().next().unwrap_or('('),
        move |cx| cycle_pair(&weak_left, current_idx, cx),
    );

    let weak_right = weak.clone();
    let right_btn = pair_button(
        theme,
        "prefs-delimiter-right",
        snap.delimiter_right.chars().next().unwrap_or(')'),
        move |cx| cycle_pair(&weak_right, current_idx, cx),
    );

    let preview_text = format!(
        "{}  …  {}",
        snap.delimiter_left, snap.delimiter_right
    );
    let preview = div()
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(13. / 16.))
        .child(SharedString::from(preview_text));

    let pair_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(14. / 16.))
        .child(label(theme, "Open:"))
        .child(left_btn)
        .child(preview)
        .child(label(theme, "Close:"))
        .child(right_btn);

    let weak_whole = weak.clone();
    let whole_doc = npp_checkbox(
        theme,
        "prefs-delimiter-whole-doc",
        "Allow on entire document",
        snap.delimiter_match_entire_document,
        move |checked, cx| {
            let _ = weak_whole.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.delimiter_match_entire_document = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Delimiter scope: entire document".to_string()
                } else {
                    "Delimiter scope: current line".to_string()
                });
                cx.notify();
            });
        },
    );

    let hint = div()
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(
            "Trigger: Ctrl + double-click inside a delimiter pair.",
        ));

    body.child(render_group_box(
        theme,
        "Delimiter selection settings",
        vec![pair_row.into_any_element(), whole_doc, hint.into_any_element()],
    ))
    .into_any_element()
}

fn current_pair_idx(left: &str, right: &str) -> usize {
    PAIRS
        .iter()
        .position(|(l, r)| *l == left && *r == right)
        .unwrap_or(0)
}

fn cycle_pair(weak: &WeakEntity<PreferencesDialog>, current_idx: usize, cx: &mut App) {
    let next_idx = (current_idx + 1) % PAIRS.len();
    let (l, r) = PAIRS[next_idx];
    let _ = weak.update(cx, |this, cx| {
        let mut next = settings::snapshot();
        next.delimiter_left = l.to_string();
        next.delimiter_right = r.to_string();
        settings::replace_and_save(next);
        this.last_action = Some(format!("Delimiter pair: {l}  {r}"));
        cx.notify();
    });
}

fn pair_button<F>(
    theme: &Theme,
    id: &'static str,
    ch: char,
    on_click: F,
) -> gpui::AnyElement
where
    F: Fn(&mut App) + 'static,
{
    div()
        .id(id)
        .w(rems(36. / 16.))
        .h(rems(24. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from(ch.to_string()))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            on_click(cx);
        })
        .into_any_element()
}

fn label(theme: &Theme, text: &'static str) -> gpui::AnyElement {
    div()
        .text_color(rgb(theme.text))
        .child(SharedString::from(text))
        .into_any_element()
}
