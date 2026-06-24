// Find in Files tab — Notepad++ visual fidelity. Mirrors the NPP screenshot
// layout exactly:
//
//   * Top row: three labeled input rows (Find what / Replace with /
//     Filters) on the left, a narrow ⇅ swap button column in the middle,
//     a vertical action button stack on the right (Find All / Replace in
//     Files / Close).
//   * Directory row sits below the top row, full-width, with a small
//     `…` browse button at the end. NPP draws the browse glyph as three
//     dots, not the word "Browse".
//   * Below the directory row, a two-column checkbox section:
//       left column  → Match whole word only / Match case
//       right column → Follow current doc. / In all sub-folders /
//                      In hidden folders
//   * Bottom section is two side-by-side group boxes:
//       Search Mode (Normal / Extended / Regular expression / .matches
//       newline) and Transparency (enable checkbox + On losing focus /
//       Always radios + a coarse +/- slider).
//
// Every toggle plumbs through to real state. The Transparency group is
// wired into `focus_aware_bg` so changing the enable / mode / level
// actually affects the dialog window's alpha — there's no mock state.
// The slider here is stepped (5-point clicks) rather than a continuous
// drag thumb; see UNTESTED.md for the smoke-test entry.

use gpui::{AnyElement, Entity, SharedString, div, prelude::*, rems, rgb};

use crate::actions::{
    FindInFilesPickDir, FindInFilesReplace, FindInFilesRun, FindInFilesSwap,
    FindInFilesToggleCase, FindInFilesToggleExtended, FindInFilesToggleFollow,
    FindInFilesToggleHidden, FindInFilesToggleRegex, FindInFilesToggleSubfolders,
    FindInFilesToggleTransparency, FindInFilesToggleWord, FindInFilesTransparencyAlways,
    FindInFilesTransparencyDec, FindInFilesTransparencyInc, FindInFilesTransparencyOnFocusLoss,
};
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

use super::close::close_button_handler;
use super::widgets::{
    checkbox, dialog_action_button, dialog_action_button_compact, group_box, labeled_row, radio,
    text_input_with_chevron,
};

pub(super) fn render(
    theme: &Theme,
    workspace: &Workspace,
    workspace_entity: Entity<Workspace>,
) -> AnyElement {
    let state = &workspace.find_in_files;
    let pad_x = rems(12. / 16.);

    let fif_replacement_handle = workspace
        .fif_replacement_focus
        .clone()
        .unwrap_or_else(|| workspace.focus_handle.clone());
    let fif_directory_handle = workspace
        .fif_directory_focus
        .clone()
        .unwrap_or_else(|| workspace.focus_handle.clone());

    let query_row = labeled_row(
        theme,
        "Find what:",
        text_input_with_chevron(
            theme,
            workspace,
            workspace_entity.clone(),
            workspace.fif_query_focus.clone(),
            InputFocus::FifQuery,
            state.error.is_some(),
            state.query.is_empty().then(|| "Search…".to_string()),
        ),
    );
    let replace_row = labeled_row(
        theme,
        "Replace with:",
        text_input_with_chevron(
            theme,
            workspace,
            workspace_entity.clone(),
            fif_replacement_handle,
            InputFocus::FifReplacement,
            false,
            state
                .replacement
                .is_empty()
                .then(|| "(empty deletes match)".to_string()),
        ),
    );
    let filter_row = labeled_row(
        theme,
        "Filters:",
        text_input_with_chevron(
            theme,
            workspace,
            workspace_entity.clone(),
            workspace.fif_filter_focus.clone(),
            InputFocus::FifFilter,
            false,
            Some("*.rs;*.toml".to_string()),
        ),
    );

    let inputs_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(6. / 16.))
        .child(query_row)
        .child(replace_row)
        .child(filter_row);

    // ⇅ swap column — matches NPP's compact swap button between the input
    // column and the right action stack. Uses the compact widget so it
    // doesn't hit the standard button's 160px min-width floor.
    let swap_ws = workspace_entity.clone();
    let swap_column = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_start()
        .pt(rems(2. / 16.))
        .child(dialog_action_button_compact(
            theme,
            "\u{21C5}",
            rems(36. / 16.),
            false,
            true,
            move |w, cx| {
                swap_ws.update(cx, |ws, cx| {
                    ws.find_in_files_swap(&FindInFilesSwap, w, cx)
                });
            },
        ));

    let run_ws = workspace_entity.clone();
    let replace_ws = workspace_entity.clone();
    let close_ws = workspace_entity.clone();
    let right_column = div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .w(rems(200. / 16.))
        .flex_shrink_0()
        .overflow_hidden()
        .child(dialog_action_button(
            theme,
            "Find All",
            true,
            true,
            move |w, cx| {
                run_ws.update(cx, |ws, cx| {
                    ws.find_in_files_run_action(&FindInFilesRun, w, cx)
                });
            },
        ))
        .child(dialog_action_button(
            theme,
            "Replace in Files",
            false,
            true,
            move |w, cx| {
                replace_ws.update(cx, |ws, cx| {
                    ws.find_in_files_replace(&FindInFilesReplace, w, cx)
                });
            },
        ))
        .child(dialog_action_button(
            theme,
            "Close",
            false,
            true,
            close_button_handler(close_ws),
        ));

    let top_row = div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(inputs_column)
        .child(swap_column)
        .child(right_column);

    // Directory row — full width below the top section. The trailing `…`
    // button opens the OS folder picker; NPP draws it as a small square
    // with three dots, so use the compact variant with a fixed width.
    let pick_ws = workspace_entity.clone();
    let dir_input = text_input_with_chevron(
        theme,
        workspace,
        workspace_entity.clone(),
        fif_directory_handle,
        InputFocus::FifDirectory,
        false,
        Some("(pick a folder or type a path)".to_string()),
    );
    let dir_row = labeled_row(
        theme,
        "Directory:",
        div()
            .flex()
            .flex_row()
            .flex_1()
            .gap(rems(6. / 16.))
            .child(dir_input)
            .child(dialog_action_button_compact(
                theme,
                "…",
                rems(28. / 16.),
                false,
                true,
                move |w, cx| {
                    pick_ws.update(cx, |ws, cx| {
                        ws.find_in_files_pick_dir(&FindInFilesPickDir, w, cx)
                    });
                },
            ))
            .into_any_element(),
    );

    // Two-column checkboxes section.
    let word_ws = workspace_entity.clone();
    let case_ws = workspace_entity.clone();
    let left_options = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(4. / 16.))
        .child(checkbox(
            theme,
            "Match whole word only",
            state.whole_word,
            true,
            move |w, cx| {
                word_ws.update(cx, |ws, cx| {
                    ws.find_in_files_toggle_word(&FindInFilesToggleWord, w, cx)
                });
            },
        ))
        .child(checkbox(
            theme,
            "Match case",
            state.match_case,
            true,
            move |w, cx| {
                case_ws.update(cx, |ws, cx| {
                    ws.find_in_files_toggle_case(&FindInFilesToggleCase, w, cx)
                });
            },
        ));

    let follow_ws = workspace_entity.clone();
    let sub_ws = workspace_entity.clone();
    let hidden_ws = workspace_entity.clone();
    let right_options = div()
        .flex()
        .flex_col()
        .w(rems(200. / 16.))
        .flex_shrink_0()
        .gap(rems(4. / 16.))
        .child(checkbox(
            theme,
            "Follow current doc.",
            state.follow_current_doc,
            true,
            move |w, cx| {
                follow_ws.update(cx, |ws, cx| {
                    ws.find_in_files_toggle_follow(&FindInFilesToggleFollow, w, cx)
                });
            },
        ))
        .child(checkbox(
            theme,
            "In all sub-folders",
            state.in_subfolders,
            true,
            move |w, cx| {
                sub_ws.update(cx, |ws, cx| {
                    ws.find_in_files_toggle_subfolders(&FindInFilesToggleSubfolders, w, cx)
                });
            },
        ))
        .child(checkbox(
            theme,
            "In hidden folders",
            state.in_hidden_folders,
            true,
            move |w, cx| {
                hidden_ws.update(cx, |ws, cx| {
                    ws.find_in_files_toggle_hidden(&FindInFilesToggleHidden, w, cx)
                });
            },
        ));

    let options_row = div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(left_options)
        .child(right_options);

    // Two side-by-side group boxes at the bottom — Search Mode + Transparency.
    let groups_row = div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(
            div()
                .flex_1()
                .child(group_box(
                    theme,
                    "Search Mode",
                    search_mode_group(theme, workspace_entity.clone(), state),
                )),
        )
        .child(
            div()
                .w(rems(200. / 16.))
                .flex_shrink_0()
                .child(group_box(
                    theme,
                    "Transparency",
                    transparency_group(theme, workspace_entity.clone(), state),
                )),
        );

    let body = div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .px(pad_x)
        .py(rems(8. / 16.))
        .child(options_row)
        .child(groups_row);

    div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .pt(rems(8. / 16.))
        .px(pad_x)
        .child(top_row)
        .child(dir_row)
        .child(body)
        .into_any_element()
}

fn search_mode_group(
    theme: &Theme,
    ws: Entity<Workspace>,
    state: &crate::find_in_files::FindInFilesState,
) -> AnyElement {
    let regex_selected = state.regex;
    let extended_selected = state.extended;
    let normal_selected = !regex_selected && !extended_selected;
    let normal_ws = ws.clone();
    let ext_ws = ws.clone();
    let regex_ws = ws.clone();
    let dot_ws = ws.clone();

    div()
        .flex()
        .flex_col()
        .gap(rems(4. / 16.))
        .child(radio(theme, "Normal", normal_selected, true, move |w, cx| {
            normal_ws.update(cx, |ws, cx| {
                if ws.find_in_files.regex {
                    ws.find_in_files_toggle_regex(&FindInFilesToggleRegex, w, cx);
                }
                if ws.find_in_files.extended {
                    ws.find_in_files_toggle_extended(&FindInFilesToggleExtended, w, cx);
                }
            });
        }))
        .child(radio(
            theme,
            "Extended (\\n, \\r, \\t, \\0, \\x...)",
            extended_selected,
            true,
            move |w, cx| {
                ext_ws.update(cx, |ws, cx| {
                    if !ws.find_in_files.extended {
                        ws.find_in_files_toggle_extended(&FindInFilesToggleExtended, w, cx);
                    }
                });
            },
        ))
        .child(radio(
            theme,
            "Regular expression",
            regex_selected,
            true,
            move |w, cx| {
                regex_ws.update(cx, |ws, cx| {
                    if !ws.find_in_files.regex {
                        ws.find_in_files_toggle_regex(&FindInFilesToggleRegex, w, cx);
                    }
                });
            },
        ))
        .child(div().pl(rems(20. / 16.)).child(checkbox(
            theme,
            ". matches newline",
            state.dot_matches_newline,
            state.regex,
            move |_w, cx| {
                dot_ws.update(cx, |ws, cx| {
                    ws.find_in_files.dot_matches_newline =
                        !ws.find_in_files.dot_matches_newline;
                    cx.notify();
                });
            },
        )))
        .into_any_element()
}

fn transparency_group(
    theme: &Theme,
    ws: Entity<Workspace>,
    state: &crate::find_in_files::FindInFilesState,
) -> AnyElement {
    let enabled = state.transparency_enabled;
    let on_focus_loss_selected = !state.transparency_always;
    let always_selected = state.transparency_always;

    let toggle_ws = ws.clone();
    let focus_ws = ws.clone();
    let always_ws = ws.clone();
    let dec_ws = ws.clone();
    let inc_ws = ws.clone();

    // Slider — left/right step buttons flanking a thin bar with a thumb
    // anchored at the current level. NPP's slider is a continuous drag
    // thumb; we step in 5-point jumps via the +/- buttons for now (see
    // UNTESTED.md). The bar visualises the current level so the user can
    // see where the alpha sits.
    let level = state.transparency_level.clamp(10, 100);
    let track_width_rems = 120. / 16.;
    let thumb_offset_rems = (track_width_rems * ((level.saturating_sub(10)) as f32 / 90.0)) - 4. / 16.;

    let slider = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(4. / 16.))
        .child(dialog_action_button_compact(
            theme,
            "−",
            rems(20. / 16.),
            false,
            enabled,
            move |w, cx| {
                dec_ws.update(cx, |ws, cx| {
                    ws.find_in_files_transparency_dec(&FindInFilesTransparencyDec, w, cx)
                });
            },
        ))
        .child(
            div()
                .id(SharedString::from("fif-transparency-track"))
                .relative()
                .w(rems(track_width_rems))
                .h(rems(6. / 16.))
                .bg(rgb(if enabled {
                    theme.group_box_border
                } else {
                    theme.disabled_text
                }))
                .border_1()
                .border_color(rgb(theme.group_box_border))
                .child(
                    div()
                        .absolute()
                        .top(rems(-3. / 16.))
                        .left(rems(thumb_offset_rems))
                        .w(rems(8. / 16.))
                        .h(rems(12. / 16.))
                        .bg(rgb(if enabled {
                            theme.checkbox_fill
                        } else {
                            theme.disabled_text
                        }))
                        .border_1()
                        .border_color(rgb(theme.group_box_border)),
                ),
        )
        .child(dialog_action_button_compact(
            theme,
            "+",
            rems(20. / 16.),
            false,
            enabled,
            move |w, cx| {
                inc_ws.update(cx, |ws, cx| {
                    ws.find_in_files_transparency_inc(&FindInFilesTransparencyInc, w, cx)
                });
            },
        ));

    div()
        .flex()
        .flex_col()
        .gap(rems(4. / 16.))
        .child(checkbox(
            theme,
            "Transparency",
            enabled,
            true,
            move |w, cx| {
                toggle_ws.update(cx, |ws, cx| {
                    ws.find_in_files_toggle_transparency(
                        &FindInFilesToggleTransparency,
                        w,
                        cx,
                    )
                });
            },
        ))
        .child(div().pl(rems(16. / 16.)).child(radio(
            theme,
            "On losing focus",
            on_focus_loss_selected,
            enabled,
            move |w, cx| {
                focus_ws.update(cx, |ws, cx| {
                    ws.find_in_files_transparency_on_focus_loss(
                        &FindInFilesTransparencyOnFocusLoss,
                        w,
                        cx,
                    )
                });
            },
        )))
        .child(div().pl(rems(16. / 16.)).child(radio(
            theme,
            "Always",
            always_selected,
            enabled,
            move |w, cx| {
                always_ws.update(cx, |ws, cx| {
                    ws.find_in_files_transparency_always(&FindInFilesTransparencyAlways, w, cx)
                });
            },
        )))
        .child(div().pl(rems(16. / 16.)).pt(rems(2. / 16.)).child(slider))
        .into_any_element()
}
