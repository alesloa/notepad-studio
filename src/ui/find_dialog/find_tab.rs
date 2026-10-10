// Find tab — Notepad++-style layout.
//
// Top: a single "Find what:" row stretching the full body width with a small
// history-dropdown chevron painted on the right edge of the input that
// opens the per-input history menu (see `widgets::text_input_with_chevron`
// + the `Workspace::toggle_find_*_history_dropdown` handlers).
//
// Below the query, the body splits into two columns:
//
//   * Left column: a checkbox stack (Backward direction, Match whole word
//     only, Match case, Wrap around) followed by a `Search Mode` group box
//     containing three radios (Normal / Extended / Regular expression) and
//     an indented `. matches newline` checkbox under the Regular expression
//     option.
//   * Right column: a vertical button stack starting with a one-shot ▲
//     "Find Previous" button sitting next to "Find Next", then an
//     `In selection` group box wrapping the document-wide buttons (Count,
//     Find All in Current Document, Find All in All Opened Documents, Close).
//     ▲ does NOT toggle any persistent state — it always runs a single
//     backward search; the persistent direction lives on the left-column
//     `Backward direction` checkbox.
//
// Every checkbox / radio / button routes through a real workspace handler:
// `In selection` captures the live linear-selection range as the search
// scope (block selection falls back to full-buffer scan); `Backward
// direction` inverts both Find Next and Find Prev. The query-history
// chevron opens a per-input recall ring buffer (see
// `widgets::text_input_with_chevron` + `FindHistorySlot`).

use gpui::{AnyElement, Entity, div, prelude::*, rems};

use crate::actions::{
    EditFindAllCurrent, EditFindAllOpened, EditFindCount, EditFindNext, EditFindPrev,
    EditFindToggleCase, EditFindToggleExtended, EditFindToggleRegex, EditFindToggleWord,
    EditFindToggleWrap,
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

    // Top "Find what:" row + history chevron affordance.
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

    // Right-side button stack — Find Next + direction arrow on top, then
    // the "In selection" group box under it.
    let next_ws = workspace_entity.clone();
    let count_ws = workspace_entity.clone();
    let all_ws = workspace_entity.clone();
    let all_opened_ws = workspace_entity.clone();
    let close_ws = workspace_entity.clone();
    let prev_ws = workspace_entity.clone();
    let in_sel_ws = workspace_entity.clone();

    // ▲ is a one-shot Find Previous button. It performs a single backward
    // search regardless of the persistent `find_state.backward` checkbox in
    // the left column. The checkbox controls the *default* direction of the
    // regular Find Next; ▲ is a momentary override.
    let direction_toggle = direction_arrow_button(theme, false, true, move |w, cx| {
        prev_ws.update(cx, |ws, cx| ws.find_prev(&EditFindPrev, w, cx));
    });
    let find_next = dialog_action_button(theme, "Find Next", true, true, move |w, cx| {
        next_ws.update(cx, |ws, cx| ws.find_next(&EditFindNext, w, cx));
    });
    let find_next_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(4. / 16.))
        .child(direction_toggle)
        .child(find_next);

    // Notepad++ layout: an untitled group box pairs the "In selection"
    // checkbox (left) with a vertical stack of [Count, Find All in Current
    // Document] (right). `Find All in All Opened Documents` and `Close` sit
    // below the group, full-width in the right column.
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

    // Width for the inner buttons inside the unlabeled "In selection" group.
    // Right column is 300px (see comment near `right_column` below). The
    // group adds 1px borders + 10px horizontal padding on each side → 278px
    // inner width. Subtracting the 8px row gap and the In-selection checkbox
    // column (110px — fits "In selection" plus the 14px checkbox box) leaves
    // 160px for the buttons column. 160px also matches the `min_w` floor of
    // the standard `dialog_action_button`, so the in-selection buttons read
    // visually flush with the rest of the right column even though they're
    // narrower than the full-column button width.
    //
    // "Find All in Current Document" is wider than 160px on a single line,
    // so `dialog_action_button_compact` lets the label wrap onto two lines
    // inside the button (matches Notepad++ — its in-selection button group
    // shows that label across two lines at the same width as `Count`).
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
    // Empty title → unlabeled group, matching Notepad++'s Find dialog.
    let in_selection = group_box(theme, "", in_selection_body);

    let find_all_all_docs = dialog_action_button(
        theme,
        "Find All in All Opened Documents",
        false,
        true,
        move |w, cx| {
            all_opened_ws.update(cx, |ws, cx| {
                ws.find_all_in_all_opened(&EditFindAllOpened, w, cx)
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

    // Right column has to fit the longest button label ("Find All in All
    // Opened Documents", ~265px at 11.5px Menlo) plus the group-box's 10px
    // horizontal padding on each side without overflowing into the left
    // column. 300rem-units gives the right-column width 300px, enough for
    // the label. Inside the 600px window floor that leaves
    // 560 − 16 gap − 300 = 244px for the left column, which comfortably
    // holds the four checkboxes plus the Search Mode group.
    // `overflow_hidden` is a belt-and-braces guard if a future label grows
    // unexpectedly long — better to clip the button than to bleed across.
    let right_column = div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .w(rems(300. / 16.))
        .flex_shrink_0()
        .overflow_hidden()
        .child(find_next_row)
        .child(in_selection)
        .child(find_all_all_docs)
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
        .child(body)
        .into_any_element()
}

/// Build the contents of the Search Mode group: three radios (Normal /
/// Extended / Regular expression) plus an indented `. matches newline`
/// checkbox sitting under the Regular expression radio.
fn search_mode_group(theme: &Theme, ws: Entity<Workspace>, state: &crate::find_state::FindState) -> AnyElement {
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
                // "Normal" is the default mode — clears both Extended and
                // Regex via the existing toggle handlers so the matches
                // snapshot refreshes through the same path.
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
        .child(
            div()
                .pl(rems(20. / 16.))
                .child(checkbox(
                    theme,
                    ". matches newline",
                    state.dot_matches_newline,
                    // Only meaningful in regex mode — disable the box when
                    // `Normal` / `Extended` is selected so the user knows the
                    // flag wouldn't apply. We still keep the value when the
                    // user flips back to Regex.
                    state.regex,
                    move |_w, cx| {
                        dot_ws.update(cx, |ws, cx| {
                            ws.find_state.dot_matches_newline =
                                !ws.find_state.dot_matches_newline;
                            ws.refresh_matches();
                            cx.notify();
                        });
                    },
                )),
        )
        .into_any_element()
}
