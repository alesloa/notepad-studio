use gpui::{CursorStyle, Div, Entity, MouseButton, SharedString, div, prelude::*, rems, rgb};

use crate::theme::Theme;
use crate::workspace::{Workspace, proofreading::CorrectionMenu};

pub(crate) fn append_rows(
    mut menu: Div,
    theme: &Theme,
    correction: Option<CorrectionMenu>,
    workspace: Entity<Workspace>,
) -> Div {
    let Some(correction) = correction else { return menu; };
    menu = menu.child(div()
        .id("proofreading-explanation")
        .px(rems(12. / 16.)).py(rems(4. / 16.))
        .max_w(rems(320. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(correction.issue.message.clone())));
    for (index, suggestion) in correction.issue.suggestions.iter().enumerate() {
        let ws = workspace.clone();
        let id = correction.id;
        menu = menu.child(div()
            .id(("proofreading-suggestion", index))
            .when(index == 0, |row| row.debug_selector(|| "proofreading-first-suggestion".into()))
            .px(rems(12. / 16.)).py(rems(4. / 16.))
            .max_w(rems(320. / 16.)).overflow_hidden()
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme.toggle_active)))
            .child(SharedString::from(if suggestion.is_empty() {
                "Remove".to_string()
            } else { suggestion.clone() }))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                ws.update(cx, |w, cx| w.apply_proofreading_suggestion(id, index, cx));
            }));
    }
    if correction.issue.kind == crate::proofreading::IssueKind::Spelling {
        let id = correction.id;
        let ws = workspace.clone();
        menu = menu.child(div()
            .id("proofreading-add-to-dictionary")
            .px(rems(12. / 16.)).py(rems(4. / 16.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme.toggle_active)))
            .child("Add to Dictionary")
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                ws.update(cx, |w, cx| w.add_proofreading_word_to_dictionary(id, cx));
            }));
        menu = menu.child(div()
            .id("proofreading-ignore")
            .px(rems(12. / 16.)).py(rems(4. / 16.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme.toggle_active)))
            .child("Ignore Word for This Session")
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                workspace.update(cx, |w, cx| w.ignore_proofreading_word(id, cx));
            }));
    }
    menu.child(div().h(rems(1. / 16.)).my(rems(4. / 16.))
        .mx(rems(8. / 16.)).bg(rgb(theme.border)))
}
