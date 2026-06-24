// Clipboard serialization + parsing for the CSV grid.
//
// Excel and Google Sheets exchange grid fragments as TSV — tab between
// fields, newline between rows, with the same RFC-4180 quoting rules
// CSV uses (quote any field that contains a tab, newline, or quote;
// double embedded quotes). We use the `csv` crate's WriterBuilder for
// both because it already handles every edge case correctly.
//
// Parsing accepts TSV first (what we emit) and falls back to CSV when
// the payload has no tabs. That covers the "I copied a column from a
// CSV file" case as well as native Excel / Sheets payloads.

use std::collections::BTreeSet;

use super::model::{CellRange, CsvModel};

/// Source of the current grid selection, used to pick what `copy_selection`
/// serialises. Priority order matches what the user most-recently set:
/// active cell range beats whole rows / cols beats single active cell.
#[derive(Clone, Debug)]
pub(crate) enum CopyKind {
    Rect(CellRange),
    Rows(BTreeSet<usize>),
    Cols(BTreeSet<usize>),
    Cell((usize, usize)),
}

/// Inspect the model and decide which selection is "live" for copy/cut.
/// Returns None when nothing is selected.
pub(crate) fn current_copy_kind(model: &CsvModel) -> Option<CopyKind> {
    if let Some(range) = model.cell_range {
        return Some(CopyKind::Rect(range));
    }
    if !model.selected_rows.is_empty() {
        return Some(CopyKind::Rows(model.selected_rows.clone()));
    }
    if !model.selected_cols.is_empty() {
        return Some(CopyKind::Cols(model.selected_cols.clone()));
    }
    if let Some(cell) = model.active_cell {
        return Some(CopyKind::Cell(cell));
    }
    None
}

/// Serialize the chosen selection as a TSV string. Empty selection
/// returns the empty string (caller should treat as "nothing to copy").
pub(crate) fn serialize_selection(model: &CsvModel, kind: &CopyKind) -> String {
    match kind {
        CopyKind::Rect(range) => serialize_rect(model, *range),
        CopyKind::Rows(rows) => serialize_rows(model, rows),
        CopyKind::Cols(cols) => serialize_cols(model, cols),
        CopyKind::Cell((r, c)) => serialize_cell(model, *r, *c),
    }
}

fn serialize_rect(model: &CsvModel, range: CellRange) -> String {
    let row_count = model.row_count();
    let col_count = model.col_count();
    if row_count == 0 || col_count == 0 {
        return String::new();
    }
    let r0 = range.min_row();
    let r1 = range.max_row().min(row_count.saturating_sub(1));
    let c0 = range.min_col();
    let c1 = range.max_col().min(col_count.saturating_sub(1));
    let mut rows: Vec<Vec<&str>> = Vec::with_capacity(r1.saturating_sub(r0) + 1);
    for r in r0..=r1 {
        let mut row: Vec<&str> = Vec::with_capacity(c1.saturating_sub(c0) + 1);
        for c in c0..=c1 {
            row.push(model.cell(r, c));
        }
        rows.push(row);
    }
    write_tsv(&rows)
}

fn serialize_rows(model: &CsvModel, rows: &BTreeSet<usize>) -> String {
    if rows.is_empty() || model.col_count() == 0 {
        return String::new();
    }
    let col_count = model.col_count();
    let mut out: Vec<Vec<&str>> = Vec::with_capacity(rows.len());
    for &r in rows {
        if r >= model.row_count() {
            continue;
        }
        let mut row: Vec<&str> = Vec::with_capacity(col_count);
        for c in 0..col_count {
            row.push(model.cell(r, c));
        }
        out.push(row);
    }
    write_tsv(&out)
}

fn serialize_cols(model: &CsvModel, cols: &BTreeSet<usize>) -> String {
    if cols.is_empty() || model.row_count() == 0 {
        return String::new();
    }
    let mut out: Vec<Vec<&str>> = Vec::with_capacity(model.row_count());
    for r in 0..model.row_count() {
        let mut row: Vec<&str> = Vec::with_capacity(cols.len());
        for &c in cols {
            if c >= model.col_count() {
                continue;
            }
            row.push(model.cell(r, c));
        }
        out.push(row);
    }
    write_tsv(&out)
}

fn serialize_cell(model: &CsvModel, r: usize, c: usize) -> String {
    if r >= model.row_count() || c >= model.col_count() {
        return String::new();
    }
    write_tsv(&[vec![model.cell(r, c)]])
}

fn write_tsv(rows: &[Vec<&str>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(b'\t')
            .quote_style(csv::QuoteStyle::Necessary)
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(&mut buf);
        for row in rows {
            let _ = wtr.write_record(row.iter().copied());
        }
        let _ = wtr.flush();
    }
    // Strip trailing newline — pastes into other apps look cleaner
    // without it and the parser handles either form on the way back in.
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    String::from_utf8(buf).unwrap_or_default()
}

/// Parse clipboard text into a 2-D vector of cell strings. Tries TSV
/// first because that's the format we emit; falls back to CSV when the
/// payload has no tabs. Empty input → empty result.
pub(crate) fn parse_clipboard(text: &str) -> Vec<Vec<String>> {
    if text.is_empty() {
        return Vec::new();
    }
    let delim = if text.contains('\t') { b'\t' } else { b',' };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delim)
        .from_reader(text.as_bytes());
    let mut out: Vec<Vec<String>> = Vec::new();
    for record in reader.records().flatten() {
        let row: Vec<String> = record.iter().map(|f| f.to_string()).collect();
        out.push(row);
    }
    // Plain single-line text with no delimiter at all comes through as
    // a one-cell record. That's the right shape for a single-cell paste.
    if out.is_empty() && !text.is_empty() {
        out.push(vec![text.to_string()]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_from(src: &str) -> CsvModel {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m
    }

    #[test]
    fn serialize_rect_basic() {
        let m = model_from("a,b,c\n1,2,3\n4,5,6\n");
        let range = CellRange::new((0, 0), (1, 2));
        let out = serialize_selection(&m, &CopyKind::Rect(range));
        assert_eq!(out, "1\t2\t3\n4\t5\t6");
    }

    #[test]
    fn serialize_rect_with_tab_quotes_field() {
        // A tab inside a cell forces the writer to quote that field.
        let mut m = model_from("h\nrow1\nrow2\n");
        m.set_cell(0, 0, "a\tb".into());
        m.set_cell(1, 0, "x".into());
        let range = CellRange::new((0, 0), (1, 0));
        let out = serialize_selection(&m, &CopyKind::Rect(range));
        // First row's cell has a tab, must be quoted.
        assert!(out.contains("\"a\tb\""), "expected quoted tab cell: {out:?}");
        assert!(out.contains("x"));
    }

    #[test]
    fn serialize_rect_with_quote_doubles_it() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("h\nv\n", 1);
        m.set_cell(0, 0, "a\"b".into());
        let range = CellRange::new((0, 0), (0, 0));
        let out = serialize_selection(&m, &CopyKind::Rect(range));
        // RFC-4180 doubles embedded quotes.
        assert_eq!(out, "\"a\"\"b\"");
    }

    #[test]
    fn serialize_rect_with_newline_quotes_field() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("h\nv\n", 1);
        m.set_cell(0, 0, "line1\nline2".into());
        let range = CellRange::new((0, 0), (0, 0));
        let out = serialize_selection(&m, &CopyKind::Rect(range));
        assert!(out.starts_with('"') && out.ends_with('"'));
        assert!(out.contains("line1\nline2"));
    }

    #[test]
    fn serialize_rows_includes_all_columns() {
        let m = model_from("a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
        let rows: BTreeSet<usize> = [0, 2].into_iter().collect();
        let out = serialize_selection(&m, &CopyKind::Rows(rows));
        assert_eq!(out, "1\t2\t3\n7\t8\t9");
    }

    #[test]
    fn serialize_cols_includes_all_rows() {
        let m = model_from("a,b,c\n1,2,3\n4,5,6\n");
        let cols: BTreeSet<usize> = [0, 2].into_iter().collect();
        let out = serialize_selection(&m, &CopyKind::Cols(cols));
        assert_eq!(out, "1\t3\n4\t6");
    }

    #[test]
    fn serialize_single_cell() {
        let m = model_from("a,b\n1,2\n");
        let out = serialize_selection(&m, &CopyKind::Cell((0, 1)));
        assert_eq!(out, "2");
    }

    #[test]
    fn parse_tsv_basic() {
        let out = parse_clipboard("1\t2\t3\n4\t5\t6");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], vec!["1", "2", "3"]);
        assert_eq!(out[1], vec!["4", "5", "6"]);
    }

    #[test]
    fn parse_tsv_csv_fallback() {
        // No tabs but commas → CSV path.
        let out = parse_clipboard("a,b,c\n1,2,3");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], vec!["a", "b", "c"]);
        assert_eq!(out[1], vec!["1", "2", "3"]);
    }

    #[test]
    fn parse_tsv_quoted_fields() {
        let out = parse_clipboard("\"a\tb\"\t\"c\"\n1\t2");
        assert_eq!(out[0], vec!["a\tb", "c"]);
        assert_eq!(out[1], vec!["1", "2"]);
    }

    #[test]
    fn parse_handles_quoted_newline() {
        let out = parse_clipboard("\"x\nyz\"\t1\n2\t3");
        assert_eq!(out[0], vec!["x\nyz", "1"]);
        assert_eq!(out[1], vec!["2", "3"]);
    }

    #[test]
    fn parse_plain_text_no_delimiters_single_cell() {
        let out = parse_clipboard("hello");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], vec!["hello"]);
    }
}
