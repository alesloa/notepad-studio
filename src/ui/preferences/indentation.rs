// Indentation panel — Indent size + Tab/Space radio + Auto-indent radio,
// plus a per-language listbox that materialises overrides into
// `Settings::language_indent_overrides`.
//
// Auto-indent (None / Basic / Advanced) drives the Enter key in the editor.
// "Advanced" is bracket-only (`{` `[` `(`) — NPP's full advanced behavior
// includes language-aware keywords (`if`, `while`, etc.); Notepad Studio
// only does brackets.
//
// The per-language section mirrors NPP `Indentation ▸ language listbox`.
// Clicking a row in the listbox selects which language the radios + width
// cycler operate on. The map is empty by default — the resolver
// `settings::indent_for_lang` falls back to the global pair when no
// override exists. First interaction with the per-lang radios / width
// materialises an entry; the "Reset to default" button removes it.

use gpui::{
    CursorStyle, IntoElement, MouseButton, ParentElement, SharedString, Styled, WeakEntity, div,
    prelude::*, rems, rgb,
};

use crate::settings::{self, AutoIndentMode, IndentOverride};
use crate::syntax::LanguageId;
use crate::theme::Theme;

use super::widgets::{npp_button, npp_button_disabled, npp_radio, render_field, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
    selected_indent_lang_idx: usize,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let uses_tabs = snap.indent_uses_tabs;
    let auto_mode = snap.auto_indent_mode;

    body.child(render_group_box(
        theme,
        "Indent Settings",
        vec![
            render_field(
                theme,
                weak,
                FieldId::TabWidth,
                "Indent size:",
                rems(60. / 16.),
            ),
            indent_radio(theme, weak, "prefs-indent-tab", "Tab character", uses_tabs, true),
            indent_radio(
                theme,
                weak,
                "prefs-indent-space",
                "Space character(s)",
                !uses_tabs,
                false,
            ),
        ],
    ))
    .child(render_group_box(
        theme,
        "Per-language overrides",
        vec![lang_override_section(theme, weak, &snap, selected_indent_lang_idx)],
    ))
    .child(render_group_box(
        theme,
        "Auto-indent",
        vec![
            auto_indent_radio(theme, weak, "prefs-autoindent-none", "None", auto_mode, AutoIndentMode::None),
            auto_indent_radio(theme, weak, "prefs-autoindent-basic", "Basic", auto_mode, AutoIndentMode::Basic),
            auto_indent_radio(
                theme,
                weak,
                "prefs-autoindent-advanced",
                "Advanced",
                auto_mode,
                AutoIndentMode::Advanced,
            ),
        ],
    ))
    .into_any_element()
}

fn indent_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    selected: bool,
    target_uses_tabs: bool,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, selected, move |cx| {
        let mut next = settings::snapshot();
        next.indent_uses_tabs = target_uses_tabs;
        settings::replace_and_save(next);
        let _ = weak.update(cx, |this, cx| {
            this.last_action = Some(if target_uses_tabs {
                "Indent: tab character".to_string()
            } else {
                "Indent: space character(s)".to_string()
            });
            cx.notify();
        });
    })
}

fn auto_indent_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: AutoIndentMode,
    target: AutoIndentMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    let selected = current == target;
    npp_radio(theme, id, label, selected, move |cx| {
        let mut next = settings::snapshot();
        next.auto_indent_mode = target;
        settings::replace_and_save(next);
        let _ = weak.update(cx, |this, cx| {
            this.last_action = Some(match target {
                AutoIndentMode::None => "Auto-indent: none".to_string(),
                AutoIndentMode::Basic => "Auto-indent: basic".to_string(),
                AutoIndentMode::Advanced => "Auto-indent: advanced".to_string(),
            });
            cx.notify();
        });
    })
}

// --- Per-language override section ---------------------------------------

fn lang_override_section(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
    selected_idx_raw: usize,
) -> gpui::AnyElement {
    let langs = LanguageId::all();
    let selected_idx = selected_idx_raw.min(langs.len().saturating_sub(1));
    let selected_lang = langs[selected_idx];
    let key = selected_lang.key();
    let override_entry = snap.language_indent_overrides.get(key).copied();
    let has_override = override_entry.is_some();
    let effective = override_entry.unwrap_or(IndentOverride {
        uses_tabs: snap.indent_uses_tabs,
        width: snap.tab_width.max(1),
    });

    let listbox = render_lang_listbox(theme, weak, selected_idx);

    let radios_row = div()
        .flex()
        .flex_col()
        .gap(rems(2. / 16.))
        .child(lang_indent_radio(
            theme,
            weak,
            selected_lang,
            "prefs-lang-indent-tab",
            "Tab character",
            effective.uses_tabs,
            true,
        ))
        .child(lang_indent_radio(
            theme,
            weak,
            selected_lang,
            "prefs-lang-indent-space",
            "Space character(s)",
            !effective.uses_tabs,
            false,
        ));

    let width_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(6. / 16.))
        .child(
            div()
                .min_w(rems(80. / 16.))
                .text_color(rgb(theme.text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from("Indent size:")),
        )
        .child(lang_width_cycler(theme, weak, selected_lang, effective.width));

    let reset_row = if has_override {
        let weak_reset = weak.clone();
        let lang_for_reset = selected_lang;
        npp_button(theme, "prefs-lang-indent-reset", "Reset to default", move |cx| {
            let mut next = settings::snapshot();
            next.language_indent_overrides.remove(lang_for_reset.key());
            settings::replace_and_save(next);
            let _ = weak_reset.update(cx, |this, cx| {
                this.last_action = Some(format!(
                    "{}: reset to default indent",
                    lang_for_reset.label()
                ));
                cx.notify();
            });
        })
    } else {
        npp_button_disabled(theme, "Reset to default")
    };

    let status = div()
        .text_color(rgb(theme.disabled_text))
        .text_size(rems(10.5 / 16.))
        .child(SharedString::from(if has_override {
            format!("{} uses its own indent.", selected_lang.label())
        } else {
            format!("{} follows the global default.", selected_lang.label())
        }));

    let right = div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .flex_1()
        .child(status)
        .child(radios_row)
        .child(width_row)
        .child(reset_row);

    div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(listbox)
        .child(right)
        .into_any_element()
}

fn render_lang_listbox(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    selected_idx: usize,
) -> gpui::AnyElement {
    let mut list = div()
        .id("prefs-indent-lang-list")
        .flex()
        .flex_col()
        .h(rems(140. / 16.))
        .min_w(rems(130. / 16.))
        .max_w(rems(150. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border));

    for (i, lang) in LanguageId::all().iter().copied().enumerate() {
        let is_active = i == selected_idx;
        let weak_row = weak.clone();
        let row_id = SharedString::from(format!("prefs-indent-lang-row-{i}"));
        let row = div()
            .id(row_id)
            .px(rems(6. / 16.))
            .py(rems(2. / 16.))
            .text_color(rgb(theme.text))
            .text_size(rems(11.5 / 16.))
            .when(is_active, |s| s.bg(rgb(theme.button_pressed_bg)))
            .when(!is_active, |s| {
                s.cursor(CursorStyle::PointingHand)
                    .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
            })
            .child(SharedString::from(lang.label()))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                let _ = weak_row.update(cx, |this, cx| {
                    this.selected_indent_lang_idx = i;
                    cx.notify();
                });
            });
        list = list.child(row);
    }

    list.into_any_element()
}

fn lang_indent_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    lang: LanguageId,
    id: &'static str,
    label: &'static str,
    selected: bool,
    target_uses_tabs: bool,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, selected, move |cx| {
        let mut next = settings::snapshot();
        let key = lang.key();
        let mut entry = next
            .language_indent_overrides
            .get(key)
            .copied()
            .unwrap_or(IndentOverride {
                uses_tabs: next.indent_uses_tabs,
                width: next.tab_width.max(1),
            });
        entry.uses_tabs = target_uses_tabs;
        next.language_indent_overrides
            .insert(key.to_string(), entry);
        settings::replace_and_save(next);
        let _ = weak.update(cx, |this, cx| {
            this.last_action = Some(if target_uses_tabs {
                format!("{}: tab character", lang.label())
            } else {
                format!("{}: space character(s)", lang.label())
            });
            cx.notify();
        });
    })
}

/// Clickable width cycler — 1 → 2 → 3 → … → 8 → 1. NPP uses a numeric
/// spinner; Notepad Studio has no spinner primitive yet, so the cycler
/// keeps a single-click interaction that still reaches every width in
/// 1..=8. First click materialises a per-language override.
fn lang_width_cycler(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    lang: LanguageId,
    current_width: usize,
) -> gpui::AnyElement {
    let weak_click = weak.clone();
    let label = format!("{current_width}");
    div()
        .id("prefs-lang-indent-width")
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .min_w(rems(32. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let mut next = settings::snapshot();
            let key = lang.key();
            let mut entry = next
                .language_indent_overrides
                .get(key)
                .copied()
                .unwrap_or(IndentOverride {
                    uses_tabs: next.indent_uses_tabs,
                    width: next.tab_width.max(1),
                });
            entry.width = if entry.width >= 8 { 1 } else { entry.width + 1 };
            let new_width = entry.width;
            next.language_indent_overrides
                .insert(key.to_string(), entry);
            settings::replace_and_save(next);
            let _ = weak_click.update(cx, |this, cx| {
                this.last_action = Some(format!("{}: indent size {}", lang.label(), new_width));
                cx.notify();
            });
        })
        .into_any_element()
}
