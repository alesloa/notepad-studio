// Remove Empty Lines — drops lines that are empty or whitespace-only.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::join_kept;
use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

/// `(result, lines removed)`; kept lines keep their line ends.
pub(super) fn transform(input: &str) -> Option<(String, usize)> {
    let lines = split_lines_eol(input);
    let kept: Vec<(&str, &str)> = lines
        .iter()
        .copied()
        .filter(|(body, _)| !body.trim().is_empty())
        .collect();
    let removed = lines.len().saturating_sub(kept.len());
    Some((join_kept(&kept)?, removed))
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

    fn run(input: &str) -> (String, usize) {
        transform(input).unwrap_or_else(|| panic!("merged lines for {input:?}"))
    }

    #[test]
    fn drops_empty() {
        let (out, n) = run("a\n\nb\n\n\nc");
        assert_eq!(out, "a\nb\nc");
        assert_eq!(n, 3);
    }

    #[test]
    fn drops_whitespace_only() {
        let (out, n) = run("a\n   \n\tb");
        assert_eq!(out, "a\n\tb");
        assert_eq!(n, 1);
    }

    #[test]
    fn keeps_line_ends_of_kept_lines() {
        // A blank line never survives, so no CR can meet an LF here.
        let cases: &[(&str, &str, usize)] = &[
            ("", "", 1),
            ("a", "a", 0),
            ("a\n", "a", 1),
            ("a\r\n\r\nb\r\n", "a\r\nb", 2),
            ("a\r\rb\r", "a\rb", 2),
            ("a\r\n \nb\r\n\rc", "a\r\nb\r\nc", 2),
            ("é\r\n\n中\r👨‍👩‍👧\n🇯🇵", "é\r\n中\r👨‍👩‍👧\n🇯🇵", 1),
            ("\r\n\t \r\n", "", 3),
        ];
        for (input, want, removed) in cases {
            assert_eq!(run(input), (want.to_string(), *removed), "input {input:?}");
        }
    }
}
