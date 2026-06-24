// Reverse Text — reverses LINE ORDER only (does not reverse characters).

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::theme::Theme;

use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

pub(super) fn transform(input: &str) -> String {
    input
        .lines()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(label_text(theme, "Reverse the line order of the input."))
        .child(caption_text(
            theme,
            "Lines come out in reverse order. Characters within a line are not reversed.",
        ))
        .child(render_text_area(
            theme,
            weak,
            dialog,
            MultiFieldId::InputText,
            "Input:",
            rems(280. / 16.),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::transform;

    #[test]
    fn reverses_lines() {
        assert_eq!(transform("a\nb\nc"), "c\nb\na");
    }

    #[test]
    fn empty_passthrough() {
        assert_eq!(transform(""), "");
    }

    #[test]
    fn does_not_reverse_chars() {
        assert_eq!(transform("hello\nworld"), "world\nhello");
    }
}
