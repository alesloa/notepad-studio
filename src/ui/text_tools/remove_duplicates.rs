// Remove Duplicates — keeps first occurrence, preserves original order.
// Exact match: case-sensitive, whitespace-sensitive (no trim).

use std::collections::HashSet;

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::join_kept;
use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

/// `(result, lines removed)`. Lines compare without their line end and keep
/// it. `None` when the result would merge two lines (`eol::join_kept`).
pub(super) fn transform(input: &str) -> Option<(String, usize)> {
    let lines = split_lines_eol(input);
    let mut seen: HashSet<&str> = HashSet::new();
    let kept: Vec<(&str, &str)> = lines
        .iter()
        .copied()
        .filter(|&(body, _)| seen.insert(body))
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

    fn run(input: &str) -> (String, usize) {
        transform(input).unwrap_or_else(|| panic!("merged lines for {input:?}"))
    }

    #[test]
    fn keeps_first_occurrence() {
        let (out, n) = run("a\nb\na\nc\nb");
        assert_eq!(out, "a\nb\nc");
        assert_eq!(n, 2);
    }

    #[test]
    fn case_sensitive() {
        let (out, n) = run("a\nA\na");
        assert_eq!(out, "a\nA");
        assert_eq!(n, 1);
    }

    #[test]
    fn no_trim() {
        let (out, n) = run("a\n a\na");
        assert_eq!(out, "a\n a");
        assert_eq!(n, 1);
    }

    #[test]
    fn compares_lines_without_their_line_end() {
        let cases: &[(&str, Option<(&str, usize)>)] = &[
            ("", Some(("", 0))),
            ("a", Some(("a", 0))),
            ("a\na\n", Some(("a\n", 1))),
            ("a\r\nb\r\na\r\n", Some(("a\r\nb\r\n", 1))),
            ("a\rb\ra", Some(("a\rb", 1))),
            ("a\r\na\nb\ra", Some(("a\r\nb", 2))),
            ("é\r\n中\n👨‍👩‍👧\r中\n🇯🇵", Some(("é\r\n中\n👨‍👩‍👧\r🇯🇵", 1))),
            ("  \r\n  \n\t", Some(("  \r\n\t", 1))),
            // Dropping the second "a" leaves "x\r" before the empty LF line.
            ("a\nx\ra\n\nb", None),
        ];
        for (input, want) in cases {
            let got = transform(input);
            let got = got.as_ref().map(|(s, n)| (s.as_str(), *n));
            assert_eq!(got, *want, "input {input:?}");
        }
    }
}
