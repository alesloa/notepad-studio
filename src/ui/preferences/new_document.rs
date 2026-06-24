// New Document panel — defaults applied to every fresh `Buffer::empty()` /
// `Buffer::untitled()`. Mirrors NPP's "New Document" preference panel:
// line-ending radio, encoding radio, default-language picker, plus two
// startup/title-naming checkboxes.
//
// Encoding is fully backed: `text/encoding.rs` already implements all five
// variants (UTF-8, UTF-8 BOM, UTF-16 LE, UTF-16 BE, Latin-1) — switching the
// default just pins what fresh buffers save as. The encoding cycle in the
// status-bar chip uses the same set.
//
// "Apply to opened ANSI files" (NPP's UTF-8 sub-checkbox) is omitted —
// Notepad Studio's loader does not re-decode bytes after the initial load, per the
// trade-off documented in `text/encoding.rs`.

use gpui::{
    InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, WeakEntity, div, rems, rgb,
};

use crate::settings;
use crate::text::encoding::Encoding;
use crate::text::line_ending::LineEnding;
use crate::theme::Theme;

use super::PreferencesDialog;
use super::widgets::{npp_checkbox, npp_radio, render_group_box};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
    default_lang_dropdown_open: bool,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    let line_ending_group = render_group_box(
        theme,
        "Format (Line ending)",
        line_ending_radios(theme, weak, &snap),
    );

    let encoding_group =
        render_group_box(theme, "Encoding", encoding_radios(theme, weak, &snap));

    body.child(
        div()
            .flex()
            .flex_col()
            .gap(rems(14. / 16.))
            .child(line_ending_group)
            .child(encoding_group)
            .child(default_language_row(theme, weak, &snap, default_lang_dropdown_open))
            .child(startup_blank_row(theme, weak, &snap))
            .child(untitled_first_line_row(theme, weak, &snap)),
    )
    .into_any_element()
}

fn line_ending_radios(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> Vec<gpui::AnyElement> {
    let active = snap
        .default_line_ending
        .unwrap_or_else(LineEnding::platform_default);
    vec![
        line_ending_radio(theme, weak, "prefs-newdoc-eol-crlf", "Windows (CR LF)", active, LineEnding::Crlf),
        line_ending_radio(theme, weak, "prefs-newdoc-eol-lf", "Unix (LF)", active, LineEnding::Lf),
        line_ending_radio(theme, weak, "prefs-newdoc-eol-cr", "Macintosh (CR)", active, LineEnding::Cr),
    ]
}

fn line_ending_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    active: LineEnding,
    target: LineEnding,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, active == target, move |cx| {
        let mut next = settings::snapshot();
        next.default_line_ending = Some(target);
        settings::replace_and_save(next);
        let _ = weak.update(cx, |this, cx| {
            this.last_action = Some(format!("Default EOL: {}", target.status_label()));
            cx.notify();
        });
    })
}

fn encoding_radios(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> Vec<gpui::AnyElement> {
    let active = snap.default_encoding.unwrap_or(Encoding::Utf8);
    vec![
        encoding_radio(theme, weak, "prefs-newdoc-enc-utf8", "UTF-8", active, Encoding::Utf8),
        encoding_radio(theme, weak, "prefs-newdoc-enc-utf8-bom", "UTF-8 with BOM", active, Encoding::Utf8Bom),
        encoding_radio(theme, weak, "prefs-newdoc-enc-utf16-le", "UTF-16 LE with BOM", active, Encoding::Utf16Le),
        encoding_radio(theme, weak, "prefs-newdoc-enc-utf16-be", "UTF-16 BE with BOM", active, Encoding::Utf16Be),
        encoding_radio(theme, weak, "prefs-newdoc-enc-latin1", "Latin-1 (ANSI)", active, Encoding::Latin1),
    ]
}

fn encoding_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    active: Encoding,
    target: Encoding,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, active == target, move |cx| {
        let mut next = settings::snapshot();
        next.default_encoding = Some(target);
        settings::replace_and_save(next);
        let _ = weak.update(cx, |this, cx| {
            this.last_action = Some(format!("Default encoding: {}", target.label()));
            cx.notify();
        });
    })
}

/// Popover-dropdown picker for the default-language. Clicking the trigger
/// opens an anchored panel listing "(auto-detect)" + every `LanguageId`. The
/// popover state lives on `PreferencesDialog::default_lang_dropdown_open`
/// and is threaded through `render()`; the panel itself paints via
/// `deferred(...).with_priority(2)` so it lands on top of every sibling
/// regardless of paint order.
fn default_language_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
    dropdown_open: bool,
) -> gpui::AnyElement {
    let current = snap.default_language;
    let label_text: SharedString = match current {
        Some(lang) => lang.label().into(),
        None => "(auto-detect by extension)".into(),
    };

    let weak_btn = weak.clone();
    let trigger = div()
        .id("prefs-newdoc-default-lang")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(rems(10. / 16.))
        .py(rems(3. / 16.))
        .min_w(rems(220. / 16.))
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.button_bg))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.text))
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(label_text)
        .child(SharedString::from("▾"))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, cx| {
            let _ = weak_btn.update(cx, |this, cx| {
                if this.default_lang_dropdown_is_open() {
                    this.close_default_lang_dropdown(cx);
                } else {
                    this.open_default_lang_dropdown(cx);
                }
            });
        });

    let weak_clear = weak.clone();
    let clear_btn = div()
        .id("prefs-newdoc-default-lang-clear")
        .ml(rems(8. / 16.))
        .px(rems(8. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(gpui::CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from("Clear"))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, cx| {
            let mut next = settings::snapshot();
            next.default_language = None;
            settings::replace_and_save(next);
            let _ = weak_clear.update(cx, |this, cx| {
                this.last_action = Some("Default language: (auto-detect)".to_string());
                cx.notify();
            });
        });

    let mut picker_box = div().relative().flex().items_center().child(trigger);
    if dropdown_open {
        picker_box = picker_box.child(super::default_lang_dropdown::render(
            theme,
            weak,
            current,
            &snap.excluded_languages,
        ));
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(
            div()
                .min_w(rems(140. / 16.))
                .text_color(rgb(theme.text))
                .text_size(rems(12. / 16.))
                .child(SharedString::from("Default language:")),
        )
        .child(picker_box)
        .child(clear_btn)
        .into_any_element()
}

fn startup_blank_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_checkbox(
        theme,
        "prefs-newdoc-startup-blank",
        "Always open a new document in addition at startup",
        snap.startup_open_blank,
        move |checked, cx| {
            let _ = weak.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.startup_open_blank = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Startup blank tab: on".to_string()
                } else {
                    "Startup blank tab: off".to_string()
                });
                cx.notify();
            });
        },
    )
}

fn untitled_first_line_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_checkbox(
        theme,
        "prefs-newdoc-untitled-first-line",
        "Use the first line of document as untitled tab name",
        snap.untitled_uses_first_line,
        move |checked, cx| {
            let _ = weak.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.untitled_uses_first_line = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Untitled tab uses first line".to_string()
                } else {
                    "Untitled tab uses 'new N'".to_string()
                });
                cx.notify();
            });
        },
    )
}

