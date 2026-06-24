// Remove Empty Lines — drops lines that are empty or whitespace-only.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::theme::Theme;

use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

pub(super) fn transform(input: &str) -> (String, usize) {
    let total = input.split('\n').count();
    let kept: Vec<&str> = input
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect();
    let removed = total.saturating_sub(kept.len());
    (kept.join("\n"), removed)
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
        .child(label_text(theme, "Remove empty lines from the input."))
        .child(caption_text(
            theme,
            "Whitespace-only lines are also dropped (trim is applied for the empty check).",
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
    fn drops_empty() {
        let (out, n) = transform("a\n\nb\n\n\nc");
        assert_eq!(out, "a\nb\nc");
        assert_eq!(n, 3);
    }

    #[test]
    fn drops_whitespace_only() {
        let (out, n) = transform("a\n   \n\tb");
        assert_eq!(out, "a\n\tb");
        assert_eq!(n, 1);
    }
}
