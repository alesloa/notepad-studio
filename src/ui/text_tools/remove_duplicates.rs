// Remove Duplicates — keeps first occurrence, preserves original order.
// Exact match: case-sensitive, whitespace-sensitive (no trim).

use std::collections::HashSet;

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::theme::Theme;

use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

pub(super) fn transform(input: &str) -> (String, usize) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut kept: Vec<&str> = Vec::new();
    let total = input.split('\n').count();
    for line in input.split('\n') {
        if seen.insert(line.to_string()) {
            kept.push(line);
        }
    }
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
        .child(label_text(theme, "Remove duplicate lines from the input."))
        .child(caption_text(
            theme,
            "Keeps the first occurrence. Exact match: case-sensitive, whitespace-sensitive (no trim).",
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
    fn keeps_first_occurrence() {
        let (out, n) = transform("a\nb\na\nc\nb");
        assert_eq!(out, "a\nb\nc");
        assert_eq!(n, 2);
    }

    #[test]
    fn case_sensitive() {
        let (out, n) = transform("a\nA\na");
        assert_eq!(out, "a\nA");
        assert_eq!(n, 1);
    }

    #[test]
    fn no_trim() {
        let (out, n) = transform("a\n a\na");
        assert_eq!(out, "a\n a");
        assert_eq!(n, 1);
    }
}
