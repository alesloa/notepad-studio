// Reverse Text — reverses LINE ORDER only (does not reverse characters).

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::LineWriter;
use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

/// The lines in reverse order. Line ends stay where they were, so the last
/// line still has none and a final line end stays final. `str::lines` used
/// to drop that final line end and turn CRLF into LF. `None` when the
/// result would merge two lines (`eol::LineWriter`).
pub(super) fn transform(input: &str) -> Option<String> {
    let mut lines = split_lines_eol(input);
    // Text ending in a line end yields a final ("", ""): not a line to move.
    if lines.len() > 1 && lines.last() == Some(&("", "")) {
        lines.pop();
    }
    let mut w = LineWriter::default();
    let bodies = lines.iter().rev().map(|&(body, _)| body);
    let eols = lines.iter().map(|&(_, eol)| eol);
    for (body, eol) in bodies.zip(eols) {
        w.line(body, eol);
    }
    w.finish()
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
        assert_eq!(transform("a\nb\nc").as_deref(), Some("c\nb\na"));
    }

    #[test]
    fn empty_passthrough() {
        assert_eq!(transform("").as_deref(), Some(""));
    }

    #[test]
    fn does_not_reverse_chars() {
        assert_eq!(transform("hello\nworld").as_deref(), Some("world\nhello"));
    }

    #[test]
    fn line_ends_stay_in_place() {
        let cases: &[(&str, Option<&str>)] = &[
            ("a", Some("a")),
            ("\n", Some("\n")),
            ("a\nb\n", Some("b\na\n")),
            ("a\r\nb\r\n", Some("b\r\na\r\n")),
            ("a\rb", Some("b\ra")),
            ("a\r\nb\nc\rd", Some("d\r\nc\nb\ra")),
            ("é\r\n中\n👨‍👩‍👧\r🇯🇵", Some("🇯🇵\r\n👨‍👩‍👧\n中\ré")),
            ("  \r\n\t\n", Some("\t\r\n  \n")),
            ("\n\n", Some("\n\n")),
            // "r" takes the CR, then the empty line takes the LF after it:
            // "r\r\n" would read back as one line.
            ("p\rq\n\nr", None),
        ];
        for (input, want) in cases {
            assert_eq!(transform(input).as_deref(), *want, "input {input:?}");
        }
    }
}
