// Auto-Completion panel — NPP `Auto-Completion` parity.
//
// Two halves:
//   * Auto-completion (mode + N-th char + 5 toggles) — UI-only persist;
//     the consuming completion engine is deferred. NPP source:
//     `Parameters.h:803-810`.
//   * Auto-Insert (6 pair toggles + user-defined slots) — the 6 pair toggles
//     are LIVE and drive `Workspace::replace_text_in_range` matched-pair
//     behaviour (see `workspace/input_handler.rs`). User-defined slots
//     persist via TOML only — list editor UI deferred.
//
// User-pair persistence works via the schema `auto_insert_user_pairs:
// Vec<(String, String)>` so users can hand-edit `settings.toml` today and
// the runtime hook will pick the entries up next launch.

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings::{self, AutocompletionMode};
use crate::theme::Theme;

use super::widgets::{
    npp_checkbox, npp_checkbox_disabled, npp_radio, render_field, render_field_disabled,
    render_group_box,
};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
    user_pair_count: usize,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    body.child(autocompletion_group(theme, weak, &snap))
        .child(auto_insert_group(theme, weak, &snap, user_pair_count))
        .into_any_element()
}

fn autocompletion_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let mode = snap.autocompletion_mode;

    let modes_row = div()
        .flex()
        .flex_row()
        .gap(rems(20. / 16.))
        .child(mode_radio(
            theme,
            weak,
            "prefs-ac-mode-none",
            "Disable",
            mode,
            AutocompletionMode::None,
        ))
        .child(mode_radio(
            theme,
            weak,
            "prefs-ac-mode-function",
            "Function completion",
            mode,
            AutocompletionMode::Function,
        ))
        .child(mode_radio(
            theme,
            weak,
            "prefs-ac-mode-word",
            "Word completion",
            mode,
            AutocompletionMode::Word,
        ))
        .child(mode_radio(
            theme,
            weak,
            "prefs-ac-mode-both",
            "Function and word completion",
            mode,
            AutocompletionMode::FunctionAndWord,
        ))
        .into_any_element();

    // NPP `setEnableForAutocompletion` (preferenceDlg.cpp) greys every
    // popup-related child when the AC master is off (= our `None` mode), and
    // greys `brief` further when the mode isn't `Function`/`FunctionAndWord`.
    // `Function parameter hint` is independent on NPP — left LIVE here too.
    let ac_on = mode != AutocompletionMode::None;
    let brief_on = matches!(
        mode,
        AutocompletionMode::Function | AutocompletionMode::FunctionAndWord
    );

    let from_chars_field = if ac_on {
        render_field(
            theme,
            weak,
            FieldId::AutocompletionFromChars,
            "From N-th character (1..9):",
            rems(60. / 16.),
        )
    } else {
        render_field_disabled(
            theme,
            FieldId::AutocompletionFromChars,
            "From N-th character (1..9):",
            &snap.autocompletion_from_chars.to_string(),
            rems(60. / 16.),
        )
    };

    let brief_toggle = if brief_on {
        let weak_brief = weak.clone();
        npp_checkbox(
            theme,
            "prefs-ac-brief",
            "Make auto-completion list brief",
            snap.autocompletion_brief,
            move |checked, cx| {
                let _ = weak_brief.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.autocompletion_brief = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Auto-completion list: brief".to_string()
                    } else {
                        "Auto-completion list: detailed".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Make auto-completion list brief",
            snap.autocompletion_brief,
        )
    };

    let weak_funcparams = weak.clone();
    let funcparams_toggle = npp_checkbox(
        theme,
        "prefs-ac-funcparams",
        "Function parameter hint on input",
        snap.autocompletion_func_params_hint,
        move |checked, cx| {
            let _ = weak_funcparams.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.autocompletion_func_params_hint = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Function parameter hints: on".to_string()
                } else {
                    "Function parameter hints: off".to_string()
                });
                cx.notify();
            });
        },
    );

    let ignore_toggle = if ac_on {
        let weak_ignore = weak.clone();
        npp_checkbox(
            theme,
            "prefs-ac-ignore-numbers",
            "Ignore numbers",
            snap.autocompletion_ignore_numbers,
            move |checked, cx| {
                let _ = weak_ignore.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.autocompletion_ignore_numbers = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Auto-completion: ignore numbers".to_string()
                    } else {
                        "Auto-completion: include numbers".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(theme, "Ignore numbers", snap.autocompletion_ignore_numbers)
    };

    let tab_toggle = if ac_on {
        let weak_tab = weak.clone();
        npp_checkbox(
            theme,
            "prefs-ac-insert-tab",
            "Insert selection with TAB",
            snap.autocompletion_insert_with_tab,
            move |checked, cx| {
                let _ = weak_tab.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.autocompletion_insert_with_tab = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Insert with TAB: on".to_string()
                    } else {
                        "Insert with TAB: off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Insert selection with TAB",
            snap.autocompletion_insert_with_tab,
        )
    };

    let enter_toggle = if ac_on {
        let weak_enter = weak.clone();
        npp_checkbox(
            theme,
            "prefs-ac-insert-enter",
            "Insert selection with ENTER",
            snap.autocompletion_insert_with_enter,
            move |checked, cx| {
                let _ = weak_enter.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.autocompletion_insert_with_enter = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Insert with ENTER: on".to_string()
                    } else {
                        "Insert with ENTER: off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Insert selection with ENTER",
            snap.autocompletion_insert_with_enter,
        )
    };

    let caveat = caveat(
        theme,
        "Engine deferred — these settings persist now and will apply when the auto-completion engine ships.",
    );

    render_group_box(
        theme,
        "Auto-completion",
        vec![
            modes_row,
            from_chars_field,
            brief_toggle,
            funcparams_toggle,
            ignore_toggle,
            tab_toggle,
            enter_toggle,
            caveat,
        ],
    )
    .into_any_element()
}

fn auto_insert_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
    user_pair_count: usize,
) -> gpui::AnyElement {
    let parens = pair_toggle(
        theme,
        weak,
        "prefs-ai-parens",
        "(  )",
        snap.auto_insert_parens,
        |s| &mut s.auto_insert_parens,
    );
    let brackets = pair_toggle(
        theme,
        weak,
        "prefs-ai-brackets",
        "[  ]",
        snap.auto_insert_brackets,
        |s| &mut s.auto_insert_brackets,
    );
    let braces = pair_toggle(
        theme,
        weak,
        "prefs-ai-braces",
        "{  }",
        snap.auto_insert_braces,
        |s| &mut s.auto_insert_braces,
    );
    let quotes = pair_toggle(
        theme,
        weak,
        "prefs-ai-quotes",
        "'  '",
        snap.auto_insert_quotes,
        |s| &mut s.auto_insert_quotes,
    );
    let dquotes = pair_toggle(
        theme,
        weak,
        "prefs-ai-dquotes",
        "\"  \"",
        snap.auto_insert_double_quotes,
        |s| &mut s.auto_insert_double_quotes,
    );
    let html = pair_toggle(
        theme,
        weak,
        "prefs-ai-html",
        "<  >  (HTML/XML angle brackets)",
        snap.auto_insert_html_xml_tag,
        |s| &mut s.auto_insert_html_xml_tag,
    );
    // Row count comes from the dialog entity (caller passes it in) — an
    // in-progress add of a brand-new row hasn't been mirrored to settings yet
    // when the panel re-renders mid-frame.
    let user_pair_block =
        super::user_pair_input::render_block(theme, weak, user_pair_count);
    render_group_box(
        theme,
        "Auto-Insert",
        vec![
            parens,
            brackets,
            braces,
            quotes,
            dquotes,
            html,
            user_pair_block,
        ],
    )
    .into_any_element()
}

fn pair_toggle(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    checked: bool,
    field: fn(&mut settings::Settings) -> &mut bool,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_checkbox(theme, id, label, checked, move |new, cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            *field(&mut next) = new;
            settings::replace_and_save(next);
            this.last_action = Some(if new {
                format!("Auto-insert {label}: on")
            } else {
                format!("Auto-insert {label}: off")
            });
            cx.notify();
        });
    })
}

fn mode_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: AutocompletionMode,
    target: AutocompletionMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.autocompletion_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!("Auto-completion mode: {} (engine pending)", mode_label(target)));
            cx.notify();
        });
    })
}

fn mode_label(m: AutocompletionMode) -> &'static str {
    match m {
        AutocompletionMode::None => "off",
        AutocompletionMode::Function => "function",
        AutocompletionMode::Word => "word",
        AutocompletionMode::FunctionAndWord => "function + word",
    }
}

fn caveat(theme: &Theme, text: &'static str) -> gpui::AnyElement {
    div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(text))
        .into_any_element()
}
