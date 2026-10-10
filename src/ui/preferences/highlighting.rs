// Highlighting panel — NPP `Highlighting`.
//
// Wired:
//   * Smart Highlighting master + Match Case + Match whole word only +
//     Use Find dialog settings. Backed by `Workspace::refresh_smart_highlight`.
//   * Mark All Occurrences defaults — feed `MarkState::new` so a fresh
//     workspace picks up the user's saved match-case / whole-word toggles.
//   * Highlight Matching Tags (Enable) — driven by `Workspace::refresh_tag_match`
//     and the tag-pair scanner in `workspace::tag_match`.
//   * Highlight tag attributes — `tag_match::find_attribute_ranges` collects
//     every attribute name+value range inside the open tag containing caret;
//     painted in the smart-highlight colour alongside the tag-name pair.
//   * Highlight comment/PHP/ASP zone — `tag_match::find_zone_pair` resolves
//     the surrounding `<!-- -->`, `<![CDATA[ ]]>`, `<? ?>`, or `<% %>` zone
//     and paints both delimiters in the smart-highlight colour.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::settings;
use crate::theme::Theme;

use super::widgets::{npp_checkbox, npp_checkbox_disabled, render_group_box};
use super::PreferencesDialog;

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    // --- Smart Highlighting -------------------------------------------------
    // NPP setEnableForSmartHilite (preferenceDlg.cpp): master off greys all
    // three sub-toggles. `Use Find dialog settings` on greys `Match case` +
    // `Match whole word only` (Find dialog values win).
    let smart_master = snap.smart_highlight_enabled;
    let use_find = snap.smart_highlight_use_find_settings;
    let case_word_live = smart_master && !use_find;
    let use_find_live = smart_master;

    let weak_smart = weak.clone();
    let smart_enable = npp_checkbox(
        theme,
        "prefs-smart-hilite-enable",
        "Enable",
        smart_master,
        move |checked, cx| {
            let _ = weak_smart.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.smart_highlight_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Smart highlighting on".to_string()
                } else {
                    "Smart highlighting off".to_string()
                });
                cx.notify();
            });
        },
    );

    let smart_case = if case_word_live {
        let weak_smart_case = weak.clone();
        npp_checkbox(
            theme,
            "prefs-smart-hilite-case",
            "Match case",
            snap.smart_highlight_case_sensitive,
            move |checked, cx| {
                let _ = weak_smart_case.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.smart_highlight_case_sensitive = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Smart hilite: match case on".to_string()
                    } else {
                        "Smart hilite: match case off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(theme, "Match case", snap.smart_highlight_case_sensitive)
    };

    let smart_word = if case_word_live {
        let weak_smart_word = weak.clone();
        npp_checkbox(
            theme,
            "prefs-smart-hilite-word",
            "Match whole word only",
            snap.smart_highlight_whole_word,
            move |checked, cx| {
                let _ = weak_smart_word.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.smart_highlight_whole_word = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Smart hilite: whole word on".to_string()
                    } else {
                        "Smart hilite: whole word off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Match whole word only",
            snap.smart_highlight_whole_word,
        )
    };

    let smart_use_find = if use_find_live {
        let weak_smart_find = weak.clone();
        npp_checkbox(
            theme,
            "prefs-smart-hilite-use-find",
            "Use Find dialog settings",
            snap.smart_highlight_use_find_settings,
            move |checked, cx| {
                let _ = weak_smart_find.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.smart_highlight_use_find_settings = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Smart hilite follows Find dialog".to_string()
                    } else {
                        "Smart hilite uses its own toggles".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Use Find dialog settings",
            snap.smart_highlight_use_find_settings,
        )
    };

    let smart_group = render_group_box(
        theme,
        "Smart Highlighting",
        vec![smart_enable, smart_case, smart_word, smart_use_find],
    );

    // --- Mark All Occurrences ----------------------------------------------
    let weak_mark_case = weak.clone();
    let weak_mark_word = weak.clone();

    let mark_group = render_group_box(
        theme,
        "Mark All Occurrences",
        vec![
            npp_checkbox(
                theme,
                "prefs-mark-all-case",
                "Match case",
                snap.mark_all_case_sensitive,
                move |checked, cx| {
                    let _ = weak_mark_case.update(cx, |this, cx| {
                        let mut next = settings::snapshot();
                        next.mark_all_case_sensitive = checked;
                        settings::replace_and_save(next);
                        this.last_action = Some(if checked {
                            "Mark All default: match case on".to_string()
                        } else {
                            "Mark All default: match case off".to_string()
                        });
                        cx.notify();
                    });
                },
            ),
            npp_checkbox(
                theme,
                "prefs-mark-all-word",
                "Match whole word only",
                snap.mark_all_whole_word,
                move |checked, cx| {
                    let _ = weak_mark_word.update(cx, |this, cx| {
                        let mut next = settings::snapshot();
                        next.mark_all_whole_word = checked;
                        settings::replace_and_save(next);
                        this.last_action = Some(if checked {
                            "Mark All default: whole word on".to_string()
                        } else {
                            "Mark All default: whole word off".to_string()
                        });
                        cx.notify();
                    });
                },
            ),
        ],
    );

    // --- Match Tags --------------------------------------------------------
    // NPP setEnableForTagsMatchHilite: master off greys both sub-toggles.
    let tags_master = snap.match_tags_enabled;

    let weak_tags = weak.clone();
    let tags_enable = npp_checkbox(
        theme,
        "prefs-match-tags",
        "Enable",
        tags_master,
        move |checked, cx| {
            let _ = weak_tags.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.match_tags_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Match tags on".to_string()
                } else {
                    "Match tags off".to_string()
                });
                cx.notify();
            });
        },
    );

    let tag_attrs = if tags_master {
        let weak_attrs = weak.clone();
        npp_checkbox(
            theme,
            "prefs-match-tag-attrs",
            "Highlight tag attributes",
            snap.match_tag_attrs_enabled,
            move |checked, cx| {
                let _ = weak_attrs.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.match_tag_attrs_enabled = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Tag attribute hilite on".to_string()
                    } else {
                        "Tag attribute hilite off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Highlight tag attributes",
            snap.match_tag_attrs_enabled,
        )
    };

    let tag_zone = if tags_master {
        let weak_zone = weak.clone();
        npp_checkbox(
            theme,
            "prefs-match-tags-comment-zone",
            "Highlight comment/PHP/ASP zone",
            snap.match_tags_in_comment_zone,
            move |checked, cx| {
                let _ = weak_zone.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.match_tags_in_comment_zone = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Tag hilite in comment zones on".to_string()
                    } else {
                        "Tag hilite in comment zones off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Highlight comment/PHP/ASP zone",
            snap.match_tags_in_comment_zone,
        )
    };

    let tags_group = render_group_box(
        theme,
        "Highlight Matching Tags",
        vec![tags_enable, tag_attrs, tag_zone],
    );

    body.child(
        div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(smart_group)
            .child(mark_group)
            .child(tags_group),
    )
    .into_any_element()
}
