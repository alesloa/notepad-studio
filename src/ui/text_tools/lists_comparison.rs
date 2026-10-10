// Lists Comparison — set difference / intersection of List A and List B.
// Empty / whitespace-only lines are filtered before set ops.
//
// Modal shows ONLY:
//   * Apply target radio: ( ) Only in A   ( ) Only in B   ( ) In Both
// List A = the active tab. List B = the `[Aux 1]` tab opened in the tab
// strip when the user enters this tool. The user types into both tabs
// directly. The bottom preview pane shows the chosen result, live.

use std::collections::HashSet;

use gpui::{
    InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, WeakEntity, div, rems,
    rgb,
};

use crate::edit_ops::util::split_lines_eol;
use crate::theme::Theme;

use super::eol::join_kept;
use super::widgets::{caption_text, label_text, render_text_area};
use super::{MultiFieldId, TextToolsDialog};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompareResult {
    OnlyInA,
    OnlyInB,
    InBoth,
}

impl Default for CompareResult {
    fn default() -> Self {
        CompareResult::OnlyInA
    }
}

/// Lines of `list` that hold text, the first occurrence of each only, with
/// their line ends. Lines compare without their line end.
fn distinct_lines(list: &str) -> Vec<(&str, &str)> {
    let mut seen: HashSet<&str> = HashSet::new();
    split_lines_eol(list)
        .into_iter()
        .filter(|&(body, _)| !body.trim().is_empty() && seen.insert(body))
        .collect()
}

/// `(only in A, only in B, in both)`, each in the order its lines first
/// appear (A's order for "in both") and keeping their line ends. A part is
/// `None` when it would merge two lines (`eol::join_kept`).
pub(super) fn transform(
    list_a: &str,
    list_b: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let a = distinct_lines(list_a);
    let b = distinct_lines(list_b);
    let in_a: HashSet<&str> = a.iter().map(|&(body, _)| body).collect();
    let in_b: HashSet<&str> = b.iter().map(|&(body, _)| body).collect();
    let pick = |lines: &[(&str, &str)], other: &HashSet<&str>, want_in_other: bool| {
        let kept: Vec<(&str, &str)> = lines
            .iter()
            .copied()
            .filter(|(body, _)| other.contains(body) == want_in_other)
            .collect();
        join_kept(&kept)
    };
    (pick(&a, &in_b, false), pick(&b, &in_a, false), pick(&a, &in_b, true))
}

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
) -> gpui::AnyElement {
    let target_label = |target: CompareResult| match target {
        CompareResult::OnlyInA => "Only in A",
        CompareResult::OnlyInB => "Only in B",
        CompareResult::InBoth => "In Both",
    };
    let active_target = dialog.compare_target;

    let mut radios = div().flex().flex_row().gap(rems(14. / 16.));
    for target in [
        CompareResult::OnlyInA,
        CompareResult::OnlyInB,
        CompareResult::InBoth,
    ] {
        let label = target_label(target);
        let selected = active_target == target;
        let dot_color = if selected { theme.checkbox_fill } else { theme.bg };
        let weak_click = weak.clone();
        let id = match target {
            CompareResult::OnlyInA => "tt-cmp-target-a",
            CompareResult::OnlyInB => "tt-cmp-target-b",
            CompareResult::InBoth => "tt-cmp-target-both",
        };
        let radio = div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(rems(6. / 16.))
            .cursor(gpui::CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                let _ = weak_click.update(cx, |this, cx| {
                    this.compare_target = target;
                    cx.notify();
                });
            })
            .child(
                div()
                    .w(rems(13. / 16.))
                    .h(rems(13. / 16.))
                    .rounded(rems(13. / 32.))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.bg))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(rems(7. / 16.))
                            .h(rems(7. / 16.))
                            .rounded(rems(7. / 32.))
                            .bg(rgb(dot_color)),
                    ),
            )
            .child(
                div()
                    .text_color(rgb(theme.text))
                    .text_size(rems(12. / 16.))
                    .child(label),
            );
        radios = radios.child(radio);
    }

    let lists = div()
        .flex()
        .flex_row()
        .gap(rems(10. / 16.))
        .child(
            div()
                .flex_1()
                .child(render_text_area(
                    theme,
                    weak,
                    dialog,
                    MultiFieldId::InputText,
                    "List A:",
                    rems(220. / 16.),
                )),
        )
        .child(
            div()
                .flex_1()
                .child(render_text_area(
                    theme,
                    weak,
                    dialog,
                    MultiFieldId::ListB,
                    "List B:",
                    rems(220. / 16.),
                )),
        );

    div()
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(label_text(
            theme,
            "Compare List A against List B. Apply replaces the active tab with the chosen result. Cmd+Z to undo. No tab open ⇒ new tab.",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(rems(10. / 16.))
                .child(label_text(theme, "Default Apply target:"))
                .child(radios),
        )
        .child(caption_text(
            theme,
            "Set semantics — duplicates within a list collapse. Lines come out in the order they first appear.",
        ))
        .child(lists)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::transform;
    use std::collections::HashSet;

    fn set(s: &Option<String>) -> HashSet<String> {
        s.as_deref()
            .unwrap_or_default()
            .split('\n')
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn basic_diff() {
        let (a, b, both) = transform("apple\nbanana\ncherry", "banana\ncherry\ndate");
        assert_eq!(set(&a), set(&some("apple")));
        assert_eq!(set(&b), set(&some("date")));
        assert_eq!(set(&both), set(&some("banana\ncherry")));
    }

    #[test]
    fn drops_blank_lines() {
        let (a, b, both) = transform("x\n\n  \ny", "y");
        assert_eq!(set(&a), set(&some("x")));
        assert_eq!(set(&b), HashSet::new());
        assert_eq!(set(&both), set(&some("y")));
    }

    #[test]
    fn duplicates_collapse() {
        let (_, _, both) = transform("a\na\nb", "a\na");
        assert_eq!(set(&both), set(&some("a")));
    }

    #[test]
    fn keeps_first_appearance_order_and_line_ends() {
        let cases: &[(&str, &str, (&str, &str, &str))] = &[
            ("", "", ("", "", "")),
            ("c\nb\na", "a", ("c\nb", "", "a")),
            ("x\r\ny\r\nz\r\n", "y\n", ("x\r\nz", "", "y")),
            ("x\ry\rz", "z\r\nw", ("x\ry", "w", "z")),
            // Lines compare without their line end.
            ("k\r\nk\nj", "k", ("j", "", "k")),
            ("é\r\n中\n👨‍👩‍👧\r🇯🇵", "中\r\n🇯🇵", ("é\r\n👨‍👩‍👧", "", "中\n🇯🇵")),
            ("  \r\n\t\nq", "", ("q", "", "")),
        ];
        for (a, b, (only_a, only_b, both)) in cases {
            let got = transform(a, b);
            let want = (some(only_a), some(only_b), some(both));
            assert_eq!(got, want, "lists {a:?} {b:?}");
        }
    }
}
