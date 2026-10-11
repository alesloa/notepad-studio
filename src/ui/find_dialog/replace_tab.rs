// Replace tab — same Notepad++-style layout as the Find tab, with an extra
// "Replace with:" row under "Find what:" and a Replace-flavoured right-side
// button stack (Find Next, Replace, Replace All, Replace All in All Opened
// Documents, Close).
//
// Every checkbox / radio / button routes through a real workspace handler:
// `Backward direction` controls the primary button and Enter; the ▲
// button next to Find Next is a one-shot Find
// Previous that does NOT mutate the persistent `backward` toggle.
// `In selection` captures the live linear-selection range as the scope —
// Replace All only walks matches inside that scope, and the scope's end
// edge slides by the cumulative byte delta after each replacement so the
// scope stays correct across runs. `. matches newline` flows into the
// live regex builder when `regex` is on. The query-history chevron stays
// disabled — no recall ring buffer yet.

use gpui::{AnyElement, Entity, div, prelude::*, rems};

use crate::actions::{
    EditFindAllCurrent, EditFindCount, EditFindPrev, EditFindToggleCase,
    EditFindToggleExtended, EditFindToggleRegex, EditFindToggleWord, EditFindToggleWrap,
    EditReplaceAll, EditReplaceAllOpened, EditReplaceCurrent,
};
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

use super::close::close_button_handler;
use super::widgets::{
    checkbox, dialog_action_button, dialog_action_button_compact, direction_arrow_button,
    group_box, labeled_row, radio, text_input_with_chevron,
};

pub(super) fn render(
    theme: &Theme,
    workspace: &Workspace,
    workspace_entity: Entity<Workspace>,
) -> AnyElement {
    let state = &workspace.find_state;
    let pad_x = rems(12. / 16.);

    let query_row = labeled_row(
        theme,
        "Find what:",
        text_input_with_chevron(
            theme,
            workspace,
            workspace_entity.clone(),
            workspace.find_focus.clone(),
            InputFocus::FindQuery,
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
            workspace.replace_focus.clone(),
            InputFocus::Replacement,
            false,
            state
                .replacement
                .is_empty()
                .then(|| "(empty deletes match)".to_string()),
        ),
    );

    // Right-side button stack.
    let next_ws = workspace_entity.clone();
    let replace_ws = workspace_entity.clone();
    let replace_all_ws = workspace_entity.clone();
    let replace_all_opened_ws = workspace_entity.clone();
    let close_ws = workspace_entity.clone();
    let prev_ws = workspace_entity.clone();
    let in_sel_ws = workspace_entity.clone();
    let count_ws = workspace_entity.clone();
    let all_ws = workspace_entity.clone();

    // ▲ is a one-shot Find Previous button. It performs a single backward
    // search regardless of the persistent `find_state.backward` checkbox in
    // the left column. The checkbox controls the *default* direction of the
    // regular Find Next; ▲ is a momentary override.
    let direction_toggle = direction_arrow_button(theme, false, true, move |w, cx| {
        prev_ws.update(cx, |ws, cx| ws.find_prev(&EditFindPrev, w, cx));
    });
    let find_next = dialog_action_button(theme, "Find Next", true, true, move |w, cx| {
        next_ws.update(cx, |ws, cx| ws.find_dialog_step(w.modifiers().shift, cx));
    });
    let find_next_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(4. / 16.))
        .child(direction_toggle)
        .child(find_next);

    // Notepad++'s Replace tab right column, top-down:
    //   ▲ + Find Next  ·  Replace  ·  Replace All
    //   [untitled group: In selection | Count, Find All in Current Document]
    //   Replace All in All Opened Documents (disabled)
    //   Close
    let replace_button = dialog_action_button(
        theme,
        "Replace",
        false,
        true,
        move |w, cx| {
            replace_ws.update(cx, |ws, cx| {
                ws.replace_current_match(&EditReplaceCurrent, w, cx)
            });
        },
    );
    let replace_all_button = dialog_action_button(
        theme,
        "Replace All",
        false,
        true,
        move |w, cx| {
            replace_all_ws.update(cx, |ws, cx| {
                ws.replace_all_matches(&EditReplaceAll, w, cx)
            });
        },
    );

    let in_selection_checkbox = checkbox(
        theme,
        "In selection",
        state.in_selection,
        true,
        move |_w, cx| {
            in_sel_ws.update(cx, |ws, cx| {
                ws.toggle_find_in_selection(cx);
            });
        },
    );

    // Same width constants as find_tab.rs — see the comment there for the
    // arithmetic. Replace tab uses the identical 300px right column and
    // identical group, so the inner button + checkbox widths must match.
    let in_sel_btn_w = rems(160. / 16.);
    let in_sel_check_w = rems(110. / 16.);
    let in_selection_buttons = div()
        .flex()
        .flex_col()
        .gap(rems(4. / 16.))
        .w(in_sel_btn_w)
        .flex_shrink_0()
        .child(dialog_action_button_compact(
            theme,
            "Count",
            in_sel_btn_w,
            false,
            true,
            move |w, cx| {
                count_ws.update(cx, |ws, cx| ws.find_count(&EditFindCount, w, cx));
            },
        ))
        .child(dialog_action_button_compact(
            theme,
            "Find All in Current Document",
            in_sel_btn_w,
            false,
            true,
            move |w, cx| {
                all_ws.update(cx, |ws, cx| {
                    ws.find_all_current(&EditFindAllCurrent, w, cx)
                });
            },
        ));

    let in_selection_body = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(
            div()
                .w(in_sel_check_w)
                .flex_shrink_0()
                .overflow_hidden()
                .child(in_selection_checkbox),
        )
        .child(in_selection_buttons)
        .into_any_element();
    // Empty title → unlabeled group, matching Notepad++'s Replace dialog.
    let in_selection = group_box(theme, "", in_selection_body);

    let replace_all_all_docs = dialog_action_button(
        theme,
        "Replace All in All Opened Documents",
        false,
        true,
        move |w, cx| {
            replace_all_opened_ws.update(cx, |ws, cx| {
                ws.replace_all_in_all_opened(&EditReplaceAllOpened, w, cx)
            });
        },
    );
    let close_button = dialog_action_button(
        theme,
        "Close",
        false,
        true,
        close_button_handler(close_ws),
    );

    // Same column-width rationale as find_tab.rs — see the comment there.
    let right_column = div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .w(rems(300. / 16.))
        .flex_shrink_0()
        .overflow_hidden()
        .child(find_next_row)
        .child(replace_button)
        .child(replace_all_button)
        .child(in_selection)
        .child(replace_all_all_docs)
        .child(close_button);

    // Left-side checkbox column + Search Mode group.
    let backward_ws = workspace_entity.clone();
    let left_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(4. / 16.))
        .child(checkbox(
            theme,
            "Backward direction",
            state.backward,
            true,
            move |_w, cx| {
                backward_ws.update(cx, |ws, cx| {
                    ws.toggle_find_backward(cx);
                });
            },
        ))
        .child(checkbox(
            theme,
            "Match whole word only",
            state.whole_word,
            true,
            {
                let ws = workspace_entity.clone();
                move |w, cx| {
                    ws.update(cx, |ws, cx| {
                        ws.toggle_find_word(&EditFindToggleWord, w, cx)
                    });
                }
            },
        ))
        .child(checkbox(theme, "Match case", state.match_case, true, {
            let ws = workspace_entity.clone();
            move |w, cx| {
                ws.update(cx, |ws, cx| {
                    ws.toggle_find_case(&EditFindToggleCase, w, cx)
                });
            }
        }))
        .child(checkbox(theme, "Wrap around", state.wrap_around, true, {
            let ws = workspace_entity.clone();
            move |w, cx| {
                ws.update(cx, |ws, cx| {
                    ws.toggle_find_wrap(&EditFindToggleWrap, w, cx)
                });
            }
        }))
        .child(group_box(
            theme,
            "Search Mode",
            search_mode_group(theme, workspace_entity.clone(), state),
        ));

    let body = div()
        .flex()
        .flex_row()
        .gap(rems(1.))
        .px(pad_x)
        .py(rems(8. / 16.))
        .child(left_column)
        .child(right_column);

    div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .pt(rems(8. / 16.))
        .px(pad_x)
        .child(query_row)
        .child(replace_row)
        .child(body)
        .into_any_element()
}

fn search_mode_group(
    theme: &Theme,
    ws: Entity<Workspace>,
    state: &crate::find_state::FindState,
) -> AnyElement {
    let normal_selected = !state.regex && !state.extended;
    let extended_selected = state.extended;
    let regex_selected = state.regex;

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
                if ws.find_state.regex {
                    ws.toggle_find_regex(&EditFindToggleRegex, w, cx);
                }
                if ws.find_state.extended {
                    ws.toggle_find_extended(&EditFindToggleExtended, w, cx);
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
                    if !ws.find_state.extended {
                        ws.toggle_find_extended(&EditFindToggleExtended, w, cx);
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
                    if !ws.find_state.regex {
                        ws.toggle_find_regex(&EditFindToggleRegex, w, cx);
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
                    ws.find_state.dot_matches_newline =
                        !ws.find_state.dot_matches_newline;
                    ws.refresh_matches();
                    cx.notify();
                });
            },
        )))
        .into_any_element()
}
