// Remove Lines Containing — plain substring (not regex), case-sensitive.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::theme::Theme;

use super::widgets::{caption_text, label_text, render_inline_text_field, render_text_area};
use super::{FieldId, MultiFieldId, TextToolsDialog};

pub(super) fn transform(pattern: &str, input: &str) -> (String, usize) {
    if pattern.is_empty() {
        return (input.to_string(), 0);
    }
    let total = input.split('\n').count();
    let kept: Vec<&str> = input
        .split('\n')
        .filter(|l| !l.contains(pattern))
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
        .child(label_text(
            theme,
            "Remove every line of the input that contains the pattern.",
        ))
        .child(render_inline_text_field(
            theme,
            weak,
            dialog,
            FieldId::Pattern,
            "Pattern:",
            rems(280. / 16.),
        ))
        .child(caption_text(
            theme,
            "Plain substring match — not a regular expression. Case-sensitive.",
        ))
        .child(render_text_area(
            theme,
            weak,
            dialog,
            MultiFieldId::InputText,
            "Input:",
            rems(260. / 16.),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::transform;

    #[test]
    fn empty_pattern_is_noop() {
        let (out, n) = transform("", "a\nb");
        assert_eq!(out, "a\nb");
        assert_eq!(n, 0);
    }

    #[test]
    fn drops_matching_lines() {
        let (out, n) = transform("DEBUG", "INFO ok\nDEBUG x\nDEBUG y\nWARN z");
        assert_eq!(out, "INFO ok\nWARN z");
        assert_eq!(n, 2);
    }

    #[test]
    fn case_sensitive() {
        let (out, n) = transform("err", "err 1\nERR 2\nerror 3");
        assert_eq!(out, "ERR 2");
        assert_eq!(n, 2);
    }
}
