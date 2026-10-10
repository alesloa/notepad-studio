// Add Prefix/Suffix — wraps each line of the in-modal Input area:
//   format!("{prefix}{line}{suffix}").
// Apply replaces the active tab with the result (Cmd+Z to undo); falls
// back to opening a new untitled tab when no tab is open.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::LineWriter;
use super::widgets::{caption_text, label_text, render_inline_text_field, render_text_area};
use super::{FieldId, MultiFieldId, TextToolsDialog};

/// Every line wrapped, each keeping its own line end. `None` when the
/// result would merge two lines (`eol::LineWriter`).
pub(super) fn transform(prefix: &str, suffix: &str, input: &str) -> Option<String> {
    let mut w = LineWriter::default();
    for (body, eol) in split_lines_eol(input) {
        w.line(&format!("{prefix}{body}{suffix}"), eol);
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
        .w_full()
        .child(
            div()
                .flex_1()
                .min_w(rems(0.))
                .child(render_inline_text_field(
                    theme,
                    weak,
                    dialog,
                    FieldId::Prefix,
                    "Prefix",
                    rems(0.),
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w(rems(0.))
                .child(render_inline_text_field(
                    theme,
                    weak,
                    dialog,
                    FieldId::Suffix,
                    "Suffix",
                    rems(0.),
                )),
        );

    div()
        .flex()
        .flex_col()
        .gap(rems(12. / 16.))
        .child(label_text(
            theme,
            "Wrap every line of the input below with the Prefix and/or Suffix above.",
        ))
        .child(inputs_row)
        .child(caption_text(
            theme,
            "Empty prefix or suffix is allowed. Blank lines are wrapped too.",
        ))
        .child(render_text_area(
            theme,
            weak,
            dialog,
            MultiFieldId::InputText,
            "Input",
            rems(280. / 16.),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::transform;

    #[test]
    fn wraps_each_line() {
        let out = transform(">>", "<<", "a\nb\nc");
        assert_eq!(out.as_deref(), Some(">>a<<\n>>b<<\n>>c<<"));
    }

    #[test]
    fn empty_prefix_suffix_passthrough() {
        assert_eq!(transform("", "", "x\ny").as_deref(), Some("x\ny"));
    }

    #[test]
    fn preserves_blank_lines() {
        assert_eq!(transform("[", "]", "a\n\nb").as_deref(), Some("[a]\n[]\n[b]"));
    }

    #[test]
    fn keeps_every_line_end() {
        // The suffix used to land between the CR and the LF of a CRLF.
        let cases: &[(&str, &str)] = &[
            ("", "[]"),
            ("a", "[a]"),
            ("a\n", "[a]\n[]"),
            ("a\r\nb\r\n", "[a]\r\n[b]\r\n[]"),
            ("a\rb\r", "[a]\r[b]\r[]"),
            ("a\r\nb\nc\rd", "[a]\r\n[b]\n[c]\r[d]"),
            ("é\r\n中\n👨‍👩‍👧\r🇯🇵", "[é]\r\n[中]\n[👨‍👩‍👧]\r[🇯🇵]"),
            ("  \r\n\t", "[  ]\r\n[\t]"),
        ];
        for (input, want) in cases {
            assert_eq!(transform("[", "]", input).as_deref(), Some(*want), "input {input:?}");
            assert_eq!(transform("", "", input).as_deref(), Some(*input), "input {input:?}");
        }
    }
}
