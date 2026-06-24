// Join/Merge — row-wise zip of N column streams with per-pair delimiters
// and a per-line prefix. Shorter columns pad empty.
//
// Modal shows ONLY:
//   * Line prefix
//   * Column count +/− (default 2, max 4)
//   * N−1 delimiter inputs
// The actual columns come from the workspace tab strip:
//   * Column 1 = active tab (whatever was active when the user opened
//     Text Tools).
//   * Column 2..N = aux tabs `[Aux 1] [Aux 2] …` opened in the strip
//     when the user picks Join/Merge or grows the column count.
// Live preview lands in the bottom Text Tools preview pane.

use gpui::{IntoElement, ParentElement, Styled, WeakEntity, div, rems};

use crate::theme::Theme;

use super::widgets::{
    caption_text, label_text, npp_button, npp_button_disabled, render_inline_text_field,
    render_text_area, render_text_field,
};
use super::{FieldId, MultiFieldId, TextToolsDialog};

/// v1 cap on number of columns. Reference impl is unbounded; we cap at 4 to
/// keep the FieldId enum + UI fixed-shape.
pub(super) const MAX_COLUMNS: usize = 4;
pub(super) const MIN_COLUMNS: usize = 2;

pub(super) fn transform(prefix: &str, columns: &[&str], delims: &[&str]) -> String {
    if columns.len() < 2 {
        return String::new();
    }
    let split: Vec<Vec<&str>> = columns.iter().map(|c| c.split('\n').collect()).collect();
    let max_lines = split.iter().map(|c| c.len()).max().unwrap_or(0);
    let mut out_lines: Vec<String> = Vec::with_capacity(max_lines);
    for i in 0..max_lines {
        let mut line = String::from(prefix);
        for (c, col) in split.iter().enumerate() {
            if c > 0 {
                let d = delims.get(c - 1).copied().unwrap_or("");
                line.push_str(d);
            }
            let cell = col.get(i).copied().unwrap_or("");
            line.push_str(cell);
        }
        out_lines.push(line);
    }
    out_lines.join("\n")
}

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<TextToolsDialog>,
    dialog: &TextToolsDialog,
) -> gpui::AnyElement {
    let n = dialog.join_column_count;

    let weak_add = weak.clone();
    let weak_remove = weak.clone();
    let add_btn = if n < MAX_COLUMNS {
        npp_button(theme, "tt-join-add-col", "Add column", move |cx| {
            let _ = weak_add.update(cx, |this, cx| {
                this.grow_join_columns(cx);
            });
        })
    } else {
        npp_button_disabled(theme, "Add column")
    };
    let remove_btn = if n > MIN_COLUMNS {
        npp_button(theme, "tt-join-remove-col", "Remove column", move |cx| {
            let _ = weak_remove.update(cx, |this, cx| {
                this.shrink_join_columns(cx);
            });
        })
    } else {
        npp_button_disabled(theme, "Remove column")
    };

    let count_label = format!("Columns: {n} (min {MIN_COLUMNS}, max {MAX_COLUMNS})");

    // Delimiter row — horizontal stack, only as many as needed.
    let mut delim_row = div().flex().flex_row().gap(rems(14. / 16.));
    if n >= 2 {
        delim_row = delim_row.child(render_inline_text_field(
            theme,
            weak,
            dialog,
            FieldId::JoinDelim1,
            "Delim 1↔2:",
            rems(90. / 16.),
        ));
    }
    if n >= 3 {
        delim_row = delim_row.child(render_inline_text_field(
            theme,
            weak,
            dialog,
            FieldId::JoinDelim2,
            "Delim 2↔3:",
            rems(90. / 16.),
        ));
    }
    if n >= 4 {
        delim_row = delim_row.child(render_inline_text_field(
            theme,
            weak,
            dialog,
            FieldId::JoinDelim3,
            "Delim 3↔4:",
            rems(90. / 16.),
        ));
    }

    let mut columns_grid = div().flex().flex_row().gap(rems(10. / 16.));
    columns_grid = columns_grid.child(
        div()
            .flex_1()
            .child(render_text_area(
                theme,
                weak,
                dialog,
                MultiFieldId::InputText,
                "Column 1:",
                rems(180. / 16.),
            )),
    );
    if n >= 2 {
        columns_grid = columns_grid.child(
            div()
                .flex_1()
                .child(render_text_area(
                    theme,
                    weak,
                    dialog,
                    MultiFieldId::JoinCol2,
                    "Column 2:",
                    rems(180. / 16.),
                )),
        );
    }
    if n >= 3 {
        columns_grid = columns_grid.child(
            div()
                .flex_1()
                .child(render_text_area(
                    theme,
                    weak,
                    dialog,
                    MultiFieldId::JoinCol3,
                    "Column 3:",
                    rems(180. / 16.),
                )),
        );
    }
    if n >= 4 {
        columns_grid = columns_grid.child(
            div()
                .flex_1()
                .child(render_text_area(
                    theme,
                    weak,
                    dialog,
                    MultiFieldId::JoinCol4,
                    "Column 4:",
                    rems(180. / 16.),
                )),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(label_text(
            theme,
            "Row-wise merge of multiple columns. Type each column's lines below; shorter columns pad empty.",
        ))
        .child(render_text_field(
            theme,
            weak,
            dialog,
            FieldId::JoinPrefix,
            "Line prefix:",
            rems(220. / 16.),
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(rems(12. / 16.))
                .child(label_text(theme, &count_label))
                .child(add_btn)
                .child(remove_btn),
        )
        .child(label_text(theme, "Delimiters between adjacent columns:"))
        .child(delim_row)
        .child(columns_grid)
        .child(caption_text(
            theme,
            "Shorter columns pad empty cells. Apply replaces the active tab with the merged result. Cmd+Z to undo. No tab open ⇒ new tab.",
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::transform;

    #[test]
    fn two_columns_basic() {
        let out = transform("", &["a\nb\nc", "1\n2\n3"], &[","]);
        assert_eq!(out, "a,1\nb,2\nc,3");
    }

    #[test]
    fn shorter_padded_empty() {
        let out = transform("", &["a\nb\nc", "1"], &[","]);
        assert_eq!(out, "a,1\nb,\nc,");
    }

    #[test]
    fn three_columns_two_delims() {
        let out = transform("", &["a", "b", "c"], &["-", "+"]);
        assert_eq!(out, "a-b+c");
    }

    #[test]
    fn line_prefix_applied_per_row() {
        let out = transform("> ", &["a\nb", "1\n2"], &[" "]);
        assert_eq!(out, "> a 1\n> b 2");
    }

    #[test]
    fn fewer_than_two_columns_empty() {
        assert_eq!(transform("", &["a"], &[]), "");
    }
}
