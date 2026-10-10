// Remove Lines Containing — plain substring (not regex), case-sensitive.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::join_kept;
use super::widgets::{caption_text, label_text, render_inline_text_field, render_text_area};
use super::{FieldId, MultiFieldId, TextToolsDialog};

/// `(result, lines removed)`; kept lines keep their line ends. `None` when
/// the result would merge two lines (`eol::join_kept`).
pub(super) fn transform(pattern: &str, input: &str) -> Option<(String, usize)> {
    if pattern.is_empty() {
        return Some((input.to_string(), 0));
    }
    let lines = split_lines_eol(input);
    let kept: Vec<(&str, &str)> = lines
        .iter()
        .copied()
        .filter(|(body, _)| !body.contains(pattern))
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

    fn run(pattern: &str, input: &str) -> (String, usize) {
        transform(pattern, input).unwrap_or_else(|| panic!("merged lines for {input:?}"))
    }

    #[test]
    fn empty_pattern_is_noop() {
        let (out, n) = run("", "a\nb");
        assert_eq!(out, "a\nb");
        assert_eq!(n, 0);
    }

    #[test]
    fn drops_matching_lines() {
        let (out, n) = run("DEBUG", "INFO ok\nDEBUG x\nDEBUG y\nWARN z");
        assert_eq!(out, "INFO ok\nWARN z");
        assert_eq!(n, 2);
    }

    #[test]
    fn case_sensitive() {
        let (out, n) = run("err", "err 1\nERR 2\nerror 3");
        assert_eq!(out, "ERR 2");
        assert_eq!(n, 2);
    }

    #[test]
    fn keeps_line_ends_of_kept_lines() {
        let cases: &[(&str, Option<(&str, usize)>)] = &[
            ("", Some(("", 0))),
            ("x", Some(("", 1))),
            ("a\n", Some(("a\n", 0))),
            ("a\r\nx\r\nb\r\n", Some(("a\r\nb\r\n", 1))),
            ("a\rx\rb", Some(("a\rb", 1))),
            ("a\r\nx\nb\rc", Some(("a\r\nb\rc", 1))),
            ("é\r\nx中\n👨‍👩‍👧\r🇯🇵", Some(("é\r\n👨‍👩‍👧\r🇯🇵", 1))),
            ("  \r\nx\n\t", Some(("  \r\n\t", 1))),
            // Dropping "x" leaves "a\r" right before the empty LF line.
            ("a\rx\n\nb", None),
        ];
        for (input, want) in cases {
            let got = transform("x", input);
            let got = got.as_ref().map(|(s, n)| (s.as_str(), *n));
            assert_eq!(got, *want, "input {input:?}");
        }
    }
}
