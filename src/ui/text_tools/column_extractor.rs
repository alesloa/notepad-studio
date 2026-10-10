// Column Extractor — split each line by `delimiter`, pick columns by index,
// rejoin with the same delimiter.
//
// Layout (Zed-style):
//   [ Columns delimited by this text: input ]   [ What columns to extract: input ]   separated by commas
//   [ Input (editable) ]
//   [ Preview ]

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::LineWriter;
use super::widgets::{caption_text, label_text, render_inline_text_field, render_text_area};
use super::{FieldId, MultiFieldId, TextToolsDialog};

pub(super) fn parse_columns(spec: &str) -> Option<Vec<usize>> {
    let mut out: Vec<usize> = Vec::new();
    for tok in spec.split(',') {
        let trimmed = tok.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.parse::<usize>() {
            Ok(n) => out.push(n),
            Err(_) => return None,
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The picked columns of every line, each line keeping its own line end.
/// `None` when an emptied line would merge two lines (`eol::LineWriter`).
pub(super) fn transform(delimiter: &str, columns_spec: &str, input: &str) -> Option<String> {
    if delimiter.is_empty() {
        return Some(input.to_string());
    }
    let Some(indices) = parse_columns(columns_spec) else {
        return Some(input.to_string());
    };
    let mut w = LineWriter::default();
    for (body, eol) in split_lines_eol(input) {
        let parts: Vec<&str> = body.split(delimiter).collect();
        let picked: Vec<&str> = indices
            .iter()
            .filter_map(|&i| parts.get(i).copied())
            .collect();
        w.line(&picked.join(delimiter), eol);
    }
    w.finish()
}

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
) -> gpui::AnyElement {
    let inputs_row = div()
        .flex()
        .flex_row()
        .gap(rems(14. / 16.))
        .items_center()
        .child(render_inline_text_field(
            theme,
            weak,
            dialog,
            FieldId::Delimiter,
            "Columns delimited by this text:",
            rems(80. / 16.),
        ))
        .child(render_inline_text_field(
            theme,
            weak,
            dialog,
            FieldId::Columns,
            "What columns to extract:",
            rems(120. / 16.),
        ))
        .child(caption_text(theme, "separated by commas"));

    div()
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(label_text(
            theme,
            "Extract specific columns from each line of the input. Out-of-range indices are silently skipped.",
        ))
        .child(inputs_row)
        .child(caption_text(
            theme,
            "Empty delimiter or unparseable column list → output equals input.",
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
    use super::{parse_columns, transform};

    #[test]
    fn parse_columns_basic() {
        assert_eq!(parse_columns("0,2"), Some(vec![0, 2]));
        assert_eq!(parse_columns("0, 1 ,2"), Some(vec![0, 1, 2]));
    }

    #[test]
    fn parse_columns_invalid() {
        assert_eq!(parse_columns(""), None);
        assert_eq!(parse_columns("a,b"), None);
        assert_eq!(parse_columns("0,abc"), None);
    }

    #[test]
    fn extract_basic() {
        let out = transform(",", "0,2", "a,b,c\n1,2,3");
        assert_eq!(out.as_deref(), Some("a,c\n1,3"));
    }

    #[test]
    fn empty_delim_noop() {
        let out = transform("", "0", "a,b\nc,d");
        assert_eq!(out.as_deref(), Some("a,b\nc,d"));
    }

    #[test]
    fn out_of_range_skipped() {
        let out = transform(",", "0,5", "a,b,c");
        assert_eq!(out.as_deref(), Some("a"));
    }

    #[test]
    fn keeps_every_line_end() {
        let cases: &[(&str, Option<&str>)] = &[
            ("", Some("")),
            ("a,b", Some("b")),
            ("a,b\n", Some("b\n")),
            ("a,b\r\nc,d\r\n", Some("b\r\nd\r\n")),
            ("a,b\rc,d\r", Some("b\rd\r")),
            ("a,b\r\nc,d\ne,f\rg,h", Some("b\r\nd\nf\rh")),
            ("é,中\r\n👨‍👩‍👧,🇯🇵", Some("中\r\n🇯🇵")),
            ("  \r\n,x", Some("\r\nx")),
            // The second line has no column 1, so "a\r" meets the empty
            // LF line and the result would read as one CRLF line.
            ("x,a\rb\nc,d", None),
        ];
        for (input, want) in cases {
            assert_eq!(transform(",", "1", input).as_deref(), *want, "input {input:?}");
        }
    }
}
