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

use crate::theme::Theme;

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

pub(super) fn transform(list_a: &str, list_b: &str) -> (String, String, String) {
    let set_a: HashSet<&str> = list_a
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect();
    let set_b: HashSet<&str> = list_b
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect();
    let only_a: Vec<&&str> = set_a.difference(&set_b).collect();
    let only_b: Vec<&&str> = set_b.difference(&set_a).collect();
    let both: Vec<&&str> = set_a.intersection(&set_b).collect();
    let join = |v: Vec<&&str>| v.iter().map(|s| **s).collect::<Vec<&str>>().join("\n");
    (join(only_a), join(only_b), join(both))
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
            "Set semantics — duplicates within a list collapse. Output order is hash-iteration (not stable).",
        ))
        .child(lists)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::transform;
    use std::collections::HashSet;

    fn set(s: &str) -> HashSet<String> {
        s.split('\n').filter(|x| !x.is_empty()).map(String::from).collect()
    }

    #[test]
    fn basic_diff() {
        let (a, b, both) = transform("apple\nbanana\ncherry", "banana\ncherry\ndate");
        assert_eq!(set(&a), set("apple"));
        assert_eq!(set(&b), set("date"));
        assert_eq!(set(&both), set("banana\ncherry"));
    }

    #[test]
    fn drops_blank_lines() {
        let (a, b, both) = transform("x\n\n  \ny", "y");
        assert_eq!(set(&a), set("x"));
        assert_eq!(set(&b), HashSet::new());
        assert_eq!(set(&both), set("y"));
    }

    #[test]
    fn duplicates_collapse() {
        let (_, _, both) = transform("a\na\nb", "a\na");
        assert_eq!(set(&both), set("a"));
    }
}
