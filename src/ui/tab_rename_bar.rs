// Inline tab-rename bar — shown above the editor when the user picks
// Rename behaviour: PowerEditor/src/NppCommands.cpp:404.
// Rename from the tab right-click menu. Mirrors macro_prompt_bar in shape:
// label, single-line text input, close button. Enter commits via
// `Workspace::commit_tab_rename`; Esc closes via `EditFindClose`.

use gpui::{CursorStyle, Entity, IntoElement, MouseButton, div, prelude::*, rems, rgb};

use crate::find_state::TabRenameState;
use crate::input_field::{InputFieldElement, rename_prompt_matches};
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

pub(crate) fn tab_rename_bar(
    theme: &Theme,
    state: &TabRenameState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let click_workspace = workspace.clone();
    let close_workspace = workspace.clone();
    let prompt_id = state.prompt_id;
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(rems(30. / 16.))
        .px(rems(8. / 16.))
        .gap_2()
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border))
        .child(
            div()
                .text_size(rems(11. / 16.))
                .text_color(rgb(theme.chrome_text))
                .child("Rename file"),
        )
        .child(
            div()
                .debug_selector(|| "tab-rename-input".into())
                .flex()
                .flex_1()
                .max_w(rems(360. / 16.))
                .min_w(rems(180. / 16.))
                .h(rems(22. / 16.))
                .px(rems(6. / 16.))
                .items_center()
                .bg(rgb(theme.bg))
                .border_1()
                .border_color(rgb(theme.border))
                .text_size(rems(12. / 16.))
                .text_color(rgb(theme.text))
                .cursor(CursorStyle::IBeam)
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    let focused = click_workspace.update(cx, |w, cx| {
                        if !rename_prompt_matches(w, InputFocus::TabRename, Some(prompt_id)) {
                            return false;
                        }
                        w.focus_input(InputFocus::TabRename, window, cx);
                        true
                    });
                    if focused { window.prevent_default(); }
                })
                .child(InputFieldElement {
                    workspace: workspace.clone(),
                    target: InputFocus::TabRename,
                    placeholder: Some("New filename".to_string()),
                    rename_prompt: Some(prompt_id),
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(rems(22. / 16.))
                .w(rems(22. / 16.))
                .text_size(rems(13. / 16.))
                .text_color(rgb(theme.chrome_text))
                .cursor(CursorStyle::PointingHand)
                .child("×")
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    close_workspace.update(cx, |w, cx| {
                        if w.tab_rename.as_ref().is_none_or(|state| state.prompt_id != prompt_id) {
                            return;
                        }
                        let restore_focus = window.is_window_active()
                            && w.tab_rename_focus.as_ref().is_some_and(|handle| handle.is_focused(window));
                        w.close_tab_rename(cx);
                        if restore_focus { window.focus(&w.focus_handle, cx); }
                    });
                }),
        )
}
