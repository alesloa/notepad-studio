// Mark tab — Notepad++ layout.
//
// Top: a single "Find what:" row stretching the full body width with a small
// history-dropdown chevron painted on the right edge of the input.
//
// Below the query, the body splits into two columns:
//
//   * Left column: Match case / Match whole word only / Wrap around / Bookmark
//     Line / Purge for each search / Clear all marks, then a `Search Mode`
//     group box (Normal / Extended / Regular expression + indented `. matches
//     newline`).
//   * Right column: a vertical button stack — Mark All / Clear all marks /
//     Copy Marked Text / Close.
//
// Every interactive control is wired:
//   * Wrap around — re-orders Mark All hits so the first listed match is the
//     next one at-or-after the caret, wrapping to the buffer top.
//   * Purge for each search — when on, Mark All replaces the existing mark
//     layer; when off, new hits append (deduped) to it.
//   * Match case / Match whole word only / `. matches newline` plumb through
//     to the regex builder used by Mark All.
//   * Search Mode (Normal / Extended / Regular expression) — Extended decodes
//     `\n` `\r` `\t` `\0` `\\` literal escapes before the regex builds.
//   * Bookmark Line — Notepad++'s checkbox companion: when on, every
//     Mark All run also bookmarks every line containing a hit.
//   * Mark All / Clear all marks / Copy Marked Text / Close — full button
//     stack on the right.
//
// The "Clear all marks" left-column checkbox stays painted-disabled because
// Notepad++ uses it as a UI mode toggle the right-column button doesn't have
// — leaving it as the right-column button only is the simpler match.
//
// History-dropdown chevron on the search input is painted but unwired
// (no recall buffer yet — same as the other dialog tabs).

use gpui::{AnyElement, Entity, div, prelude::*, rems};

use crate::actions::{
    MarkAll, MarkClearAll, MarkCopy, MarkToggleBookmarkLine, MarkToggleCase, MarkToggleExtended,
    MarkTogglePurge, MarkToggleRegex, MarkToggleWord, MarkToggleWrap,
};
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

use super::close::close_button_handler;
use super::widgets::{
    checkbox, dialog_action_button, group_box, labeled_row, radio, text_input_with_chevron,
};

pub(super) fn render(
    theme: &Theme,
    workspace: &Workspace,
    workspace_entity: Entity<Workspace>,
) -> AnyElement {
    let state = &workspace.mark_state;
    let pad_x = rems(12. / 16.);

    // Lazy mark focus handle: created by `ensure_dialog_focus_handles` before
    // first render. Fall back to the editor focus_handle defensively.
    let mark_handle = workspace
        .mark_focus
        .clone()
        .unwrap_or_else(|| workspace.focus_handle.clone());

    let query_row = labeled_row(
        theme,
        "Find what:",
        text_input_with_chevron(
            theme,
            workspace,
            workspace_entity.clone(),
            mark_handle,
            InputFocus::MarkQuery,
            state.error.is_some(),
            state.query.is_empty().then(|| "Search…".to_string()),
        ),
    );

    // Right-side button stack.
    let mark_ws = workspace_entity.clone();
    let clear_ws = workspace_entity.clone();
    let copy_ws = workspace_entity.clone();
    let close_ws = workspace_entity.clone();
    let right_column = div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .w(rems(300. / 16.))
        .flex_shrink_0()
        .overflow_hidden()
        .child(dialog_action_button(
            theme,
            "Mark All",
            true,
            true,
            move |w, cx| {
                mark_ws.update(cx, |ws, cx| ws.mark_all(&MarkAll, w, cx));
            },
        ))
        .child(dialog_action_button(
            theme,
            "Clear all marks",
            false,
            true,
            move |w, cx| {
                clear_ws.update(cx, |ws, cx| ws.mark_clear_all(&MarkClearAll, w, cx));
            },
        ))
        .child(dialog_action_button(
            theme,
            "Copy Marked Text",
            false,
            true,
            move |w, cx| {
                copy_ws.update(cx, |ws, cx| ws.mark_copy(&MarkCopy, w, cx));
            },
        ))
        .child(dialog_action_button(
            theme,
            "Close",
            false,
            true,
            close_button_handler(close_ws),
        ));

    // Left-side checkbox column + Search Mode group.
    let case_ws = workspace_entity.clone();
    let word_ws = workspace_entity.clone();
    let bm_ws = workspace_entity.clone();
    let wrap_ws = workspace_entity.clone();
    let purge_ws = workspace_entity.clone();
    let left_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rems(4. / 16.))
        .child(checkbox(theme, "Match case", state.match_case, true, move |w, cx| {
            case_ws.update(cx, |ws, cx| ws.mark_toggle_case(&MarkToggleCase, w, cx));
        }))
        .child(checkbox(theme, "Match whole word only", state.whole_word, true, move |w, cx| {
            word_ws.update(cx, |ws, cx| ws.mark_toggle_word(&MarkToggleWord, w, cx));
        }))
        .child(checkbox(theme, "Wrap around", state.wrap_around, true, move |w, cx| {
            wrap_ws.update(cx, |ws, cx| ws.mark_toggle_wrap(&MarkToggleWrap, w, cx));
        }))
        .child(checkbox(
            theme,
            "Bookmark Line",
            state.bookmark_line,
            true,
            move |w, cx| {
                bm_ws.update(cx, |ws, cx| {
                    ws.mark_toggle_bookmark_line(&MarkToggleBookmarkLine, w, cx)
                });
            },
        ))
        .child(checkbox(
            theme,
            "Purge for each search",
            state.purge_for_each_search,
            true,
            move |w, cx| {
                purge_ws.update(cx, |ws, cx| ws.mark_toggle_purge(&MarkTogglePurge, w, cx));
            },
        ))
        // "Clear all marks" left-column checkbox stays disabled — Notepad++
        // paints this as a UI-mode toggle that pairs with Purge in their
        // engine, which we model directly via the right-column button. No
        // additional state needed here.
        .child(checkbox(theme, "Clear all marks", false, false, |_, _| {}))
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

fn search_mode_group(
    theme: &Theme,
    ws: Entity<Workspace>,
    state: &crate::find_state::MarkState,
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
                if ws.mark_state.regex {
                    ws.mark_toggle_regex(&MarkToggleRegex, w, cx);
                }
                if ws.mark_state.extended {
                    ws.mark_toggle_extended(&MarkToggleExtended, w, cx);
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
                    if !ws.mark_state.extended {
                        ws.mark_toggle_extended(&MarkToggleExtended, w, cx);
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
                    if !ws.mark_state.regex {
                        ws.mark_toggle_regex(&MarkToggleRegex, w, cx);
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
                    ws.mark_state.dot_matches_newline =
                        !ws.mark_state.dot_matches_newline;
                    cx.notify();
                });
            },
        )))
        .into_any_element()
}
