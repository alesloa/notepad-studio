// CSV preview status bar.
//
// Thin strip below the grid that mirrors the format_bar styling. Reports
// row × column counts plus a short context phrase that tracks the
// current editing / selection state — Excel-style A1 cell reference
// when an active cell is set, "N rows selected" / "N columns selected"
// when an axis selection is live, otherwise just the dimensions.
//
// When the selection includes 2+ numeric cells, the right side shows
// Sum / Avg / Count aggregates (Excel parity).

use std::collections::BTreeSet;

use gpui::{IntoElement, ParentElement, SharedString, Styled, div, px, rems, rgb};

use crate::theme::Theme;

use super::grid::CsvGridSnapshot;
use super::model::{col_letter, format_aggregate_number, parse_numeric};

/// Numeric aggregates for the status bar. Computed against the active
/// selection (rect / rows / cols). `None` when the selection holds
/// fewer than 2 numeric cells — Excel parity (single-cell aggregates
/// are redundant with the cell value itself).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AggregateSummary {
    pub sum: f64,
    pub avg: f64,
    pub count: usize,
}

/// Iterate the cells inside the active selection and produce a
/// numeric aggregate when 2+ cells parse. Headers are excluded.
/// Empty cells and non-numeric cells are skipped silently.
pub(crate) fn compute_aggregate(
    cells: &[Vec<String>],
    active_cell: Option<(usize, usize)>,
    cell_range: Option<super::model::CellRange>,
    selected_rows: &BTreeSet<usize>,
    selected_cols: &BTreeSet<usize>,
) -> Option<AggregateSummary> {
    let row_count = cells.len();
    let col_count = cells.first().map(|r| r.len()).unwrap_or(0);

    let cell_at = |r: usize, c: usize| -> Option<&str> {
        cells.get(r).and_then(|row| row.get(c)).map(|s| s.as_str())
    };

    let mut sum = 0.0_f64;
    let mut count = 0usize;

    if let Some(range) = cell_range {
        for r in range.min_row()..=range.max_row() {
            for c in range.min_col()..=range.max_col() {
                let Some(s) = cell_at(r, c) else { continue };
                let Some(n) = parse_numeric(s) else { continue };
                sum += n;
                count += 1;
            }
        }
    } else if !selected_rows.is_empty() {
        for &r in selected_rows {
            for c in 0..col_count {
                let Some(s) = cell_at(r, c) else { continue };
                let Some(n) = parse_numeric(s) else { continue };
                sum += n;
                count += 1;
            }
        }
    } else if !selected_cols.is_empty() {
        for &c in selected_cols {
            for r in 0..row_count {
                let Some(s) = cell_at(r, c) else { continue };
                let Some(n) = parse_numeric(s) else { continue };
                sum += n;
                count += 1;
            }
        }
    } else if active_cell.is_some() {
        // Single active cell → don't show aggregates. Threshold check
        // below would catch this anyway; explicit branch keeps the
        // intent clear.
        return None;
    } else {
        return None;
    }

    if count < 2 {
        return None;
    }
    let avg = sum / count as f64;
    Some(AggregateSummary { sum, avg, count })
}

/// Compose the human-readable status string. Pure — no theme / GPUI
/// dependencies so the formatting logic is unit-testable.
fn compose_status(
    row_count: usize,
    col_count: usize,
    active_cell: Option<(usize, usize)>,
    editing_cell: Option<(usize, usize)>,
    selected_rows: usize,
    selected_cols: usize,
    hidden_rows: usize,
) -> String {
    let visible_rows = row_count.saturating_sub(hidden_rows);
    let base = if hidden_rows > 0 {
        format!(
            "{visible_rows} of {row_count} rows × {col_count} columns  ·  {hidden_rows} hidden"
        )
    } else {
        format!("{row_count} rows × {col_count} columns")
    };
    // Header rename uses the row == usize::MAX sentinel — surface it as
    // the header cell coordinate rather than a body row reference.
    let detail = if let Some((row, col)) = editing_cell {
        if row == usize::MAX {
            Some(format!("Editing header {}", col_letter(col)))
        } else {
            Some(format!("Editing {}{}", col_letter(col), row + 1))
        }
    } else if let Some((row, col)) = active_cell {
        Some(format!("{}{}", col_letter(col), row + 1))
    } else if selected_rows > 0 {
        let label = if selected_rows == 1 { "row" } else { "rows" };
        Some(format!("{selected_rows} {label} selected"))
    } else if selected_cols > 0 {
        let label = if selected_cols == 1 { "column" } else { "columns" };
        Some(format!("{selected_cols} {label} selected"))
    } else {
        None
    };
    match detail {
        Some(d) => format!("{base}  ·  {d}"),
        None => base,
    }
}

/// Format an aggregate for display, e.g. `Sum: 12  Avg: 2.4  Count: 5`.
/// Returns empty string when the aggregate is `None`.
pub(crate) fn format_aggregate(agg: Option<&AggregateSummary>) -> String {
    match agg {
        Some(a) => format!(
            "Sum: {}  Avg: {}  Count: {}",
            format_aggregate_number(a.sum),
            format_aggregate_number(a.avg),
            a.count
        ),
        None => String::new(),
    }
}

pub(crate) fn csv_status_bar(theme: &Theme, snap: &CsvGridSnapshot) -> impl IntoElement {
    let bg = theme.toolbar_bg;
    let border = theme.border;
    let text = theme.toolbar_icon;

    let text_value = compose_status(
        snap.row_count,
        snap.col_count,
        snap.active_cell,
        snap.editing_cell,
        snap.selected_rows.len(),
        snap.selected_cols.len(),
        snap.hidden_rows.len(),
    );

    let agg = compute_aggregate(
        &snap.cells,
        snap.active_cell,
        snap.cell_range,
        &snap.selected_rows,
        &snap.selected_cols,
    );
    let agg_text = format_aggregate(agg.as_ref());

    let mut bar = div()
        .h(rems(24. / 16.))
        .flex_shrink_0()
        .px(rems(8. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .overflow_hidden()
        .bg(rgb(bg))
        .border_t_1()
        .border_color(rgb(border))
        .text_size(rems(11.5 / 16.))
        .text_color(rgb(text))
        .child(div().flex_grow().child(SharedString::from(text_value)));
    if !agg_text.is_empty() {
        bar = bar.child(
            div()
                .flex_shrink_0()
                .text_color(rgb(text))
                .child(SharedString::from(agg_text)),
        );
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::csv_doc::model::CellRange;

    #[test]
    fn base_string_when_no_state() {
        assert_eq!(compose_status(10, 4, None, None, 0, 0, 0), "10 rows × 4 columns");
    }

    #[test]
    fn active_cell_uses_a1_notation() {
        assert_eq!(
            compose_status(50, 30, Some((0, 0)), None, 0, 0, 0),
            "50 rows × 30 columns  ·  A1"
        );
        assert_eq!(
            compose_status(50, 30, Some((4, 2)), None, 0, 0, 0),
            "50 rows × 30 columns  ·  C5"
        );
        assert_eq!(
            compose_status(50, 30, Some((26, 27)), None, 0, 0, 0),
            "50 rows × 30 columns  ·  AB27"
        );
    }

    #[test]
    fn editing_cell_overrides_active() {
        assert_eq!(
            compose_status(50, 30, Some((4, 2)), Some((4, 2)), 0, 0, 0),
            "50 rows × 30 columns  ·  Editing C5"
        );
    }

    #[test]
    fn editing_header_cell_is_labeled() {
        assert_eq!(
            compose_status(50, 30, None, Some((usize::MAX, 1)), 0, 0, 0),
            "50 rows × 30 columns  ·  Editing header B"
        );
    }

    #[test]
    fn selected_rows_singular_and_plural() {
        assert_eq!(
            compose_status(50, 30, None, None, 1, 0, 0),
            "50 rows × 30 columns  ·  1 row selected"
        );
        assert_eq!(
            compose_status(50, 30, None, None, 4, 0, 0),
            "50 rows × 30 columns  ·  4 rows selected"
        );
    }

    #[test]
    fn selected_columns_singular_and_plural() {
        assert_eq!(
            compose_status(50, 30, None, None, 0, 1, 0),
            "50 rows × 30 columns  ·  1 column selected"
        );
        assert_eq!(
            compose_status(50, 30, None, None, 0, 3, 0),
            "50 rows × 30 columns  ·  3 columns selected"
        );
    }

    #[test]
    fn empty_file_renders_safely() {
        assert_eq!(compose_status(0, 0, None, None, 0, 0, 0), "0 rows × 0 columns");
    }

    #[test]
    fn hidden_count_renders_with_visible_total() {
        assert_eq!(
            compose_status(50, 4, None, None, 0, 0, 12),
            "38 of 50 rows × 4 columns  ·  12 hidden"
        );
    }

    #[test]
    fn hidden_count_can_combine_with_active_cell() {
        assert_eq!(
            compose_status(50, 4, Some((0, 0)), None, 0, 0, 3),
            "47 of 50 rows × 4 columns  ·  3 hidden  ·  A1"
        );
    }

    fn cells_from(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    fn rect(r0: usize, c0: usize, r1: usize, c1: usize) -> CellRange {
        CellRange {
            anchor: (r0, c0),
            focus: (r1, c1),
        }
    }

    #[test]
    fn aggregate_sum_avg_count_basic() {
        let cells = cells_from(&[&["1", "2"], &["3", ""]]);
        let agg = compute_aggregate(
            &cells,
            None,
            Some(rect(0, 0, 1, 1)),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(agg.count, 3);
        assert_eq!(agg.sum, 6.0);
        assert_eq!(agg.avg, 2.0);
    }

    #[test]
    fn aggregate_skips_non_numeric() {
        let cells = cells_from(&[&["1", "foo", "2"]]);
        let agg = compute_aggregate(
            &cells,
            None,
            Some(rect(0, 0, 0, 2)),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(agg.count, 2);
        assert_eq!(agg.sum, 3.0);
        assert_eq!(agg.avg, 1.5);
    }

    #[test]
    fn aggregate_skips_empty() {
        let cells = cells_from(&[&["", "1", "", "2"]]);
        let agg = compute_aggregate(
            &cells,
            None,
            Some(rect(0, 0, 0, 3)),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(agg.count, 2);
        assert_eq!(agg.sum, 3.0);
    }

    #[test]
    fn aggregate_below_threshold_returns_none() {
        let cells = cells_from(&[&["5"]]);
        let agg = compute_aggregate(
            &cells,
            None,
            Some(rect(0, 0, 0, 0)),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert!(agg.is_none());
    }

    #[test]
    fn aggregate_zero_numeric_returns_none() {
        let cells = cells_from(&[&["foo", "bar"]]);
        let agg = compute_aggregate(
            &cells,
            None,
            Some(rect(0, 0, 0, 1)),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert!(agg.is_none());
    }

    #[test]
    fn aggregate_handles_negatives_and_decimals() {
        let cells = cells_from(&[&["-1.5", "2.5", "1"]]);
        let agg = compute_aggregate(
            &cells,
            None,
            Some(rect(0, 0, 0, 2)),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(agg.count, 3);
        assert_eq!(agg.sum, 2.0);
        let diff = (agg.avg - (2.0 / 3.0)).abs();
        assert!(diff < 1e-9, "avg should be 0.666..., got {}", agg.avg);
    }

    #[test]
    fn aggregate_active_cell_only_returns_none() {
        let cells = cells_from(&[&["5"]]);
        let agg = compute_aggregate(
            &cells,
            Some((0, 0)),
            None,
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert!(agg.is_none());
    }

    #[test]
    fn aggregate_selected_rows_includes_all_cols() {
        let cells = cells_from(&[&["1", "2"], &["3", "4"], &["5", "6"]]);
        let mut rows = BTreeSet::new();
        rows.insert(0);
        rows.insert(2);
        let agg =
            compute_aggregate(&cells, None, None, &rows, &BTreeSet::new()).unwrap();
        // Row 0: 1+2=3; Row 2: 5+6=11; total 14, count 4.
        assert_eq!(agg.count, 4);
        assert_eq!(agg.sum, 14.0);
    }

    #[test]
    fn aggregate_selected_cols_includes_all_rows() {
        let cells = cells_from(&[&["1", "2"], &["3", "4"], &["5", "6"]]);
        let mut cols = BTreeSet::new();
        cols.insert(1);
        let agg =
            compute_aggregate(&cells, None, None, &BTreeSet::new(), &cols).unwrap();
        // Col 1: 2+4+6=12, count 3.
        assert_eq!(agg.count, 3);
        assert_eq!(agg.sum, 12.0);
    }

    #[test]
    fn format_number_trims_trailing_zeros() {
        assert_eq!(format_aggregate_number(5.0), "5");
        assert_eq!(format_aggregate_number(5.5), "5.5");
        assert_eq!(format_aggregate_number(5.55), "5.55");
        assert_eq!(format_aggregate_number(-1.5), "-1.5");
        assert_eq!(format_aggregate_number(0.0), "0");
    }

    #[test]
    fn format_aggregate_full_string() {
        let agg = AggregateSummary {
            sum: 6.0,
            avg: 2.0,
            count: 3,
        };
        assert_eq!(format_aggregate(Some(&agg)), "Sum: 6  Avg: 2  Count: 3");
        assert_eq!(format_aggregate(None), "");
    }
}
