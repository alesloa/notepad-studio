// Find in Projects tab — NPP's `FINDINPROJECTS_DLG` layout. Shares state
// with the Find in Files tab (query / replace / filter / options) so the
// settings carry across when the user toggles tabs; only the scope and the
// right-side controls differ.
//
// Layout mirrors the NPP reference screenshot:
//   * Three labeled input rows (Find what / Replace with / Filters), each
//     with a history-dropdown chevron. No Directory row — scope is the
//     folder-panel root(s) instead.
//   * A narrow ↑↓ swap button between the input column and the right
//     button stack — swaps the Find what / Replace with strings.
//   * Right column: Find All / Replace in Projects / Close, followed by
//     three Project Panel checkboxes. Each checkbox maps to one
//     `folder_panel_roots[i]`. Panels without a backing root render
//     greyed-out (NPP does the same when no project file is loaded).
//   * Below the right column: Match whole word only / Match case +
//     Search Mode group on the left side.
//
// Find All / Replace in Projects are enabled only when at least one
// project panel is checked AND that panel has a backing root — matches
// NPP's `enable` gate in `enableFindInProjectsFunc`.

use gpui::{AnyElement, Entity, div, prelude::*, rems};

use crate::actions::{
    FindInFilesToggleCase, FindInFilesToggleExtended, FindInFilesToggleRegex,
    FindInFilesToggleWord, FindInProjectsReplace, FindInProjectsRun, FindInProjectsSwap,
    FindInProjectsTogglePanel1, FindInProjectsTogglePanel2, FindInProjectsTogglePanel3,
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
    let snap = crate::settings::snapshot();
    let roots = &snap.folder_panel_roots;

    let fif_replacement_handle = workspace
        .fif_replacement_focus
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

    // Stack of the three labeled input rows.
    let inputs_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(6. / 16.))
        .child(query_row)
        .child(replace_row)
        .child(filter_row);

    // ⇅ swap column — narrow vertical strip with one short button. Sits
    // between the input column and the right button stack. NPP's swap
    // button is roughly square (~36px wide × 28px tall); we mirror that
    // with the compact variant + an explicit width.
    let swap_ws = workspace_entity.clone();
    let swap_column = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_start()
        .pt(rems(2. / 16.))
        .child(dialog_action_button_compact(
            theme,
            "\u{21C5}", // ⇅ — vertical two-way arrow, matches NPP's swap glyph
            rems(36. / 16.),
            false,
            true,
            move |w, cx| {
                swap_ws.update(cx, |ws, cx| {
                    ws.find_in_projects_swap(&FindInProjectsSwap, w, cx)
                });
            },
        ));

    // Right column: action button stack + Project Panel checkboxes.
    //
    // Find/Replace enable when at least one panel is checked AND that
    // panel has a backing root. Mirrors NPP's
    // `enable = panel1 || panel2 || panel3` gate.
    let any_active_panel = (state.project_panel_1 && roots.first().is_some())
        || (state.project_panel_2 && roots.get(1).is_some())
        || (state.project_panel_3 && roots.get(2).is_some());

    let run_ws = workspace_entity.clone();
    let replace_run_ws = workspace_entity.clone();
    let close_ws = workspace_entity.clone();
    let p1_ws = workspace_entity.clone();
    let p2_ws = workspace_entity.clone();
    let p3_ws = workspace_entity.clone();

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
            any_active_panel,
            move |w, cx| {
                run_ws.update(cx, |ws, cx| {
                    ws.find_in_projects_run_action(&FindInProjectsRun, w, cx)
                });
            },
        ))
        .child(dialog_action_button(
            theme,
            "Replace in Projects",
            false,
            any_active_panel,
            move |w, cx| {
                replace_run_ws.update(cx, |ws, cx| {
                    ws.find_in_projects_replace(&FindInProjectsReplace, w, cx)
                });
            },
        ))
        .child(dialog_action_button(
            theme,
            "Close",
            false,
            true,
            close_button_handler(close_ws),
        ))
        .child(div().h(rems(6. / 16.))) // small gap between button stack and panel checkboxes
        .child(checkbox(
            theme,
            "Project Panel 1",
            state.project_panel_1,
            roots.first().is_some(),
            move |w, cx| {
                p1_ws.update(cx, |ws, cx| {
                    ws.find_in_projects_toggle_panel_1(&FindInProjectsTogglePanel1, w, cx)
                });
            },
        ))
        .child(checkbox(
            theme,
            "Project Panel 2",
            state.project_panel_2,
            roots.get(1).is_some(),
            move |w, cx| {
                p2_ws.update(cx, |ws, cx| {
                    ws.find_in_projects_toggle_panel_2(&FindInProjectsTogglePanel2, w, cx)
                });
            },
        ))
        .child(checkbox(
            theme,
            "Project Panel 3",
            state.project_panel_3,
            roots.get(2).is_some(),
            move |w, cx| {
                p3_ws.update(cx, |ws, cx| {
                    ws.find_in_projects_toggle_panel_3(&FindInProjectsTogglePanel3, w, cx)
                });
            },
        ));

    let top_row = div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(inputs_column)
        .child(swap_column)
        .child(right_column);

    // Left side of body — Match whole word + Match case + Search Mode.
    let case_ws = workspace_entity.clone();
    let word_ws = workspace_entity.clone();
    let left_column = div()
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
        ))
        .child(group_box(
            theme,
            "Search Mode",
            search_mode_group(theme, workspace_entity.clone(), state),
        ));

    let body = div()
        .flex()
        .flex_row()
        .gap(rems(16. / 16.))
        .px(pad_x)
        .py(rems(8. / 16.))
        .child(left_column);

    div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .pt(rems(8. / 16.))
        .px(pad_x)
        .child(top_row)
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
