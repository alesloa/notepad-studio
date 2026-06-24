// Inline rich-doc link-prompt bar — shown above the editor pane when the
// user clicks the toolbar Link button while a rich-doc block is active.
// Mirrors tab_rename_bar in shape but adds an Apply button (commit typed
// URL) and a contextual Unlink button (only when the active block already
// carries a link). Enter commits via `commit_rich_doc_link_bar`; Esc
// closes via the EditFindClose chain.

use gpui::{CursorStyle, Entity, IntoElement, MouseButton, div, prelude::*, px, rems, rgb};

use crate::actions::EditFindClose;
use crate::find_state::RichDocLinkBarState;
use crate::input_field::InputFieldElement;
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

pub(crate) fn rich_doc_link_bar(
    theme: &Theme,
    state: &RichDocLinkBarState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let click_workspace = workspace.clone();
    let apply_workspace = workspace.clone();
    let unlink_workspace = workspace.clone();
    let close_workspace = workspace.clone();
    let had_link = state.had_link;

    let mut row = div()
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
                .child("Hyperlink URL"),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .max_w(rems(420. / 16.))
                .min_w(rems(220. / 16.))
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
                    click_workspace.update(cx, |w, cx| {
                        w.focus_input(InputFocus::RichDocLinkUrl, window, cx)
                    });
                })
                .child(InputFieldElement {
                    workspace: workspace.clone(),
                    target: InputFocus::RichDocLinkUrl,
                    placeholder: Some("https://example.com".to_string()),
                }),
        )
        .child(action_button(theme, "rd-link-apply", "Apply", true, move |window, cx| {
            apply_workspace.update(cx, |w, cx| {
                w.commit_rich_doc_link_bar(cx);
                w.close_find(&EditFindClose, window, cx);
            });
        }));

    if had_link {
        row = row.child(action_button(
            theme,
            "rd-link-unlink",
            "Unlink",
            false,
            move |window, cx| {
                unlink_workspace.update(cx, |w, cx| {
                    w.rich_doc_link_unlink(cx);
                    w.close_find(&EditFindClose, window, cx);
                });
            },
        ));
    }

    row = row.child(close_button(theme, move |window, cx| {
        close_workspace.update(cx, |w, cx| {
            w.close_rich_doc_link_bar(cx);
            w.close_find(&EditFindClose, window, cx);
        });
    }));

    row
}

fn action_button<F>(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    primary: bool,
    handler: F,
) -> impl IntoElement
where
    F: Fn(&mut gpui::Window, &mut gpui::App) + 'static,
{
    let bg = theme.button_bg;
    let hover_bg = theme.button_hover_bg;
    let border = if primary {
        theme.checkbox_fill
    } else {
        theme.border
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(rems(22. / 16.))
        .min_w(rems(58. / 16.))
        .px(rems(8. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(rgb(hover_bg)))
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
            handler(window, cx);
        })
}

fn close_button<F>(theme: &Theme, handler: F) -> impl IntoElement
where
    F: Fn(&mut gpui::Window, &mut gpui::App) + 'static,
{
    div()
        .id("rd-link-close")
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
            handler(window, cx);
        })
        // Width-stable trailing pad so the row doesn't reflow when Unlink
        // appears/disappears next to it.
        .ml(px(0.))
}
