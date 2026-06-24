// NPP `Search Engine` panel — picks the URL template `Edit ▸ Search on
// Internet` uses to open the browser with the current selection. NPP source:
// `PowerEditor/src/WinControls/Preference/preferenceDlg.cpp` (CloudAndLinkSubDlg
// owns the radios in NPP 8.x). Notepad Studio stores the choice + custom URL in
// `Settings::search_engine` / `search_engine_custom_url` (see `settings.rs`).

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings::{self, SearchEngineChoice};
use crate::theme::Theme;

use super::widgets::{DialogTextInput, npp_radio, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let choice = snap.search_engine;
    let is_custom = matches!(choice, SearchEngineChoice::Custom);

    let radios: Vec<gpui::AnyElement> = vec![
        engine_radio(theme, weak, "prefs-se-duckduckgo", "DuckDuckGo", choice, SearchEngineChoice::DuckDuckGo),
        engine_radio(theme, weak, "prefs-se-google", "Google", choice, SearchEngineChoice::Google),
        engine_radio(theme, weak, "prefs-se-bing", "Bing", choice, SearchEngineChoice::Bing),
        engine_radio(theme, weak, "prefs-se-yahoo", "Yahoo!", choice, SearchEngineChoice::Yahoo),
        engine_radio(
            theme,
            weak,
            "prefs-se-stackoverflow",
            "Stack Overflow",
            choice,
            SearchEngineChoice::StackOverflow,
        ),
        engine_radio(theme, weak, "prefs-se-custom", "Custom", choice, SearchEngineChoice::Custom),
    ];

    let custom_row = custom_url_row(theme, weak, is_custom);
    let hint = div()
        .pl(rems(28. / 16.))
        .text_size(rems(11. / 16.))
        .text_color(rgb(if is_custom { theme.chrome_text } else { theme.disabled_text }))
        .child(SharedString::from(
            "Use $(CURRENT_WORD) as a placeholder for the searched term.",
        ));

    let mut children: Vec<gpui::AnyElement> = radios;
    children.push(custom_row);
    children.push(hint.into_any_element());

    body.child(render_group_box(theme, "Search Engine", children))
        .into_any_element()
}

fn engine_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: SearchEngineChoice,
    target: SearchEngineChoice,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.search_engine = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!("Search engine: {}", radio_status(target)));
            cx.notify();
        });
    })
}

fn radio_status(c: SearchEngineChoice) -> &'static str {
    match c {
        SearchEngineChoice::DuckDuckGo => "DuckDuckGo",
        SearchEngineChoice::Google => "Google",
        SearchEngineChoice::Bing => "Bing",
        SearchEngineChoice::Yahoo => "Yahoo!",
        SearchEngineChoice::StackOverflow => "Stack Overflow",
        SearchEngineChoice::Custom => "custom URL",
    }
}

fn custom_url_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    enabled: bool,
) -> gpui::AnyElement {
    // The text field renders unconditionally so the layout stays stable when
    // the user toggles between Custom and a preset; gating editability is via
    // the focus handle (only the Custom radio routes IME to it through the
    // panel's focus order). Visually we dim the box when not on Custom so it
    // reads as "disabled" without removing the placeholder slot.
    let border_color = if enabled { theme.border } else { theme.disabled_text };
    let bg = if enabled { theme.bg } else { theme.chrome };
    let input = DialogTextInput {
        dialog: weak.clone(),
        field_id: FieldId::SearchEngineCustomUrl,
    };
    let input_box = div()
        .w(rems(420. / 16.))
        .h(rems(24. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border_color))
        .px(rems(6. / 16.))
        .child(input);

    div()
        .pl(rems(28. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .child(input_box)
        .into_any_element()
}
