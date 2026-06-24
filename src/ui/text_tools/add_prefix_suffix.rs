// Add Prefix/Suffix — wraps each line of the in-modal Input area:
//   format!("{prefix}{line}{suffix}").
// Apply replaces the active tab with the result (Cmd+Z to undo); falls
// back to opening a new untitled tab when no tab is open.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::theme::Theme;

use super::widgets::{caption_text, label_text, render_inline_text_field, render_text_area};
use super::{FieldId, MultiFieldId, TextToolsDialog};

pub(super) fn transform(prefix: &str, suffix: &str, input: &str) -> String {
    input
        .split('\n')
        .map(|line| format!("{prefix}{line}{suffix}"))
        .collect::<Vec<_>>()
        .join("\n")
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
        assert_eq!(out, ">>a<<\n>>b<<\n>>c<<");
    }

    #[test]
    fn empty_prefix_suffix_passthrough() {
        assert_eq!(transform("", "", "x\ny"), "x\ny");
    }

    #[test]
    fn preserves_blank_lines() {
        assert_eq!(transform("[", "]", "a\n\nb"), "[a]\n[]\n[b]");
    }
}
