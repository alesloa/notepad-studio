// Local grid admission policy. Refusals describe these checks; Source View
// keeps the original document, encoding and line endings unchanged.
use super::model::CsvModel;

pub(super) const MAX_SOURCE_BYTES: usize = 10 * 1024 * 1024;
pub(super) const MAX_CELLS: usize = 256 * 1024;
pub(super) const MAX_COLUMNS: usize = 1024;
pub(super) const MAX_CELL_BYTES: usize = 1024 * 1024;
pub(super) const TOO_LARGE: &str = "CSV grid is too large; use Source View";
pub(super) const EDIT_TOO_LARGE: &str =
    "CSV grid edit exceeds its memory limit; no changes applied";

pub(super) fn shape(rows: usize, columns: usize) -> Result<(), &'static str> {
    if columns > MAX_COLUMNS
        || rows
            .checked_mul(columns.max(1))
            .is_none_or(|cells| cells > MAX_CELLS)
    {
        return Err(TOO_LARGE);
    }
    Ok(())
}

pub(super) fn read_rows(source: &str, delimiter: u8) -> Result<Vec<Vec<String>>, &'static str> {
    read_rows_cancellable(
        source,
        delimiter,
        &std::sync::atomic::AtomicBool::new(false),
    )
}

fn read_rows_cancellable(
    source: &str,
    delimiter: u8,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<Vec<String>>, &'static str> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(TOO_LARGE);
    }
    if !delimiter.is_ascii() || matches!(delimiter, b'\n' | b'\r' | b'"') {
        return Err("Invalid CSV delimiter");
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(source.as_bytes());
    let mut rows = Vec::new();
    let mut columns = 0;
    for record in reader.records() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("CSV grid preparation cancelled");
        }
        let record = record.map_err(|_| "CSV could not be parsed; use Source View")?;
        columns = columns.max(record.len());
        shape(rows.len().saturating_add(1), columns)?;
        let mut row = Vec::new();
        row.try_reserve_exact(record.len()).map_err(|_| TOO_LARGE)?;
        for field in &record {
            if field.len() > MAX_CELL_BYTES {
                return Err(TOO_LARGE);
            }
            let mut value = String::new();
            value
                .try_reserve_exact(field.len())
                .map_err(|_| TOO_LARGE)?;
            value.push_str(field);
            row.push(value);
        }
        rows.try_reserve(1).map_err(|_| TOO_LARGE)?;
        rows.push(row);
    }
    Ok(rows)
}

pub(super) struct Parsed {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub line_terminator: super::model::LineTerminator,
    pub trailing_newline: bool,
    pub had_bom: bool,
}

impl Parsed {
    pub(super) fn new(
        source: &str,
        delimiter: u8,
        has_header: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self, &'static str> {
        let (had_bom, source) = super::model::strip_bom(source);
        let mut rows = read_rows_cancellable(source, delimiter, cancel)?;
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        shape(rows.len().saturating_add(usize::from(!has_header)), columns)?;
        let mut headers = if has_header && !rows.is_empty() {
            rows.remove(0)
        } else {
            (0..columns)
                .map(|col| format!("Column {}", col + 1))
                .collect()
        };
        for row in std::iter::once(&mut headers).chain(rows.iter_mut()) {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("CSV grid preparation cancelled");
            }
            row.try_reserve_exact(columns.saturating_sub(row.len()))
                .map_err(|_| TOO_LARGE)?;
            row.resize(columns, String::new());
        }
        Ok(Self {
            headers,
            rows,
            had_bom,
            line_terminator: super::model::detect_line_terminator(source),
            trailing_newline: source.ends_with(['\n', '\r']),
        })
    }
}

impl CsvModel {
    pub(super) fn data_bytes(&self) -> usize {
        self.headers
            .iter()
            .chain(self.rows.iter().flatten())
            .fold(0usize, |total, value| total.saturating_add(value.len()))
    }

    pub(super) fn admit_shape(&mut self, rows: usize, columns: usize) -> bool {
        if self.parse_error.is_some() {
            return false;
        }
        if shape(rows.saturating_add(1), columns).is_err() {
            self.edit_error = Some(EDIT_TOO_LARGE);
            return false;
        }
        true
    }

    pub(super) fn admit_replacement(&mut self, removed: usize, added: usize) -> bool {
        if self.parse_error.is_some() {
            return false;
        }
        if self
            .data_bytes()
            .saturating_sub(removed)
            .checked_add(added)
            .is_none_or(|bytes| bytes > MAX_SOURCE_BYTES)
        {
            self.edit_error = Some(EDIT_TOO_LARGE);
            return false;
        }
        true
    }

    pub(super) fn admit_bytes(&mut self, extra: usize) -> bool {
        if self
            .data_bytes()
            .checked_add(extra)
            .is_none_or(|bytes| bytes > MAX_SOURCE_BYTES)
        {
            self.edit_error = Some(EDIT_TOO_LARGE);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_ragged_csv_is_refused_before_rectangular_expansion() {
        let source = format!("{}\n{}", ",".repeat(999), "x\n".repeat(1000));
        assert_eq!(source.len(), 3000);
        assert_eq!(read_rows(&source, b','), Err(TOO_LARGE));
        let mut model = CsvModel::empty(b',');
        model.rebuild_from_source(&source, 1);
        assert_eq!(model.parse_error, Some(TOO_LARGE));
        assert!(model.rows.is_empty() && model.emit_source().is_none());
    }

    #[test]
    fn growth_and_repeated_paste_are_refused_without_changes_or_history() {
        let mut model = CsvModel::empty(b',');
        model.rebuild_from_source("a,b\nx,y\n", 1);
        let before = model.emit_source();
        assert!(!model.paste_at(usize::MAX - 1, 0, &[vec!["z".into()]], None));
        assert_eq!(model.emit_source(), before);
        assert!(model.undo_stack.is_empty());
        let value = "x".repeat(MAX_CELL_BYTES);
        let range = super::super::model::CellRange::new((0, 0), (100, 100));
        assert!(!model.paste_at(0, 0, &[vec![value]], Some(range)));
        assert_eq!(model.emit_source(), before);
        assert!(model.undo_stack.is_empty());
    }
}

#[cfg(test)]
mod edit_budget_tests {
    use super::super::model::CellRange;
    use super::*;

    #[test]
    fn budget_counts_replacements_and_ignores_unrelated_selection() {
        let mut model = CsvModel::empty(b',');
        model.rebuild_from_source(
            &format!("a,b\n{},{}\n", "x".repeat(600_000), "y".repeat(600_000)),
            1,
        );
        // Replacing a long existing value does not add its entire size twice.
        let value = "z".repeat(600_000);
        assert!(model.paste_at(0, 0, &[vec![value.clone()]], None));
        assert!(model.fill_right_range(CellRange::new((0, 0), (0, 1))));
        // Non-multiple selections do not participate in an anchor-only paste.
        let unrelated = CellRange::new((0, 0), (9_999_999, 4));
        assert!(model.paste_at(
            0,
            0,
            &[vec!["small".into(), "block".into(), "third".into()]],
            Some(unrelated)
        ));
        assert_eq!(model.cell(0, 2), "third");
    }

    #[test]
    fn large_series_fill_is_refused_atomically() {
        let mut model = CsvModel::empty(b',');
        model.rebuild_from_source(
            &format!("h\n{}\n{}", "x".repeat(600_000), "empty\n".repeat(100)),
            1,
        );
        let before = model.emit_source();
        let range = CellRange::new((0, 0), (0, 0));
        model.cell_range = Some(range);
        model.begin_fill_drag(gpui::point(gpui::px(0.), gpui::px(0.)));
        if let Some(drag) = &mut model.fill_drag {
            drag.current_target = CellRange::new((0, 0), (100, 0));
        }
        assert!(!model.commit_fill_drag());
        assert_eq!(model.emit_source(), before);
        assert!(model.undo_stack.is_empty());
    }
}

#[cfg(test)]
mod release_probe {
    use super::*;

    #[test]
    #[ignore = "release performance diagnostic; run with --release --ignored --nocapture"]
    fn csv_grid_release_admission_and_snapshot() {
        let source = format!(
            "header\n{}",
            format!("{}\n", "x".repeat(119)).repeat(105_000)
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.csv");
        std::fs::write(&path, &source).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > crate::io_stream::STREAM_THRESHOLD_BYTES);
        // Previous parser's read/rectangular-pad algorithm, retained here only
        // to measure the same already-decoded source against admission checks.
        let before = std::time::Instant::now();
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(source.as_bytes());
        let mut rows: Vec<Vec<String>> = reader
            .records()
            .map(|record| record.unwrap().iter().map(str::to_owned).collect())
            .collect();
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        for row in &mut rows {
            row.resize(cols, String::new());
        }
        let old_ms = before.elapsed().as_secs_f64() * 1000.;
        assert_eq!(rows.len(), 105_001);
        drop(rows);
        let before = std::time::Instant::now();
        let mut model = CsvModel::empty(b',');
        model.rebuild_from_source(&source, 1);
        let new_ms = before.elapsed().as_secs_f64() * 1000.;
        assert_eq!(model.parse_error, Some(TOO_LARGE));
        assert!(model.rows.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        let small_source = format!("h\n{}", "1\n".repeat(100_000));
        model.rebuild_from_source(&small_source, 2);
        let before = std::time::Instant::now();
        let snap = super::super::grid::CsvGridSnapshot::from_model(&model);
        let snapshot_ms = before.elapsed().as_secs_f64() * 1000.;
        assert!(snap.cells.len() <= 256);
        println!(
            "CSV release probe: file_bytes={} old_parse_ms={old_ms:.3} refused_parse_ms={new_ms:.3} snapshot_100k_ms={snapshot_ms:.3} copied_rows={}",
            source.len(),
            snap.cells.len()
        );
    }
}

#[cfg(test)]
mod commit_tests {
    use super::super::actions::{commit_cell_edit, open_cell_editor, open_header_editor};
    use super::*;
    use crate::text::encoding::Encoding;

    #[test]
    fn oversized_cell_or_header_commit_preserves_draft_source_and_history() {
        for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
            for header in [false, true] {
                super::super::actions::savepoint_tests::with_csv(
                    encoding,
                    |w, window, cx, path| {
                        let source = w.buffer().rope.to_string();
                        let revision = w.buffer().raw_revision.clone();
                        let disk = std::fs::read(path).unwrap();
                        if header {
                            open_header_editor(w, 0, window, cx);
                        } else {
                            open_cell_editor(w, 0, 0, window, cx);
                        }
                        let id = w.tabs[0].session_id;
                        let value = "é".repeat(MAX_CELL_BYTES / 2 + 1);
                        w.csv_models
                            .get_mut(&id)
                            .unwrap()
                            .cell_input
                            .as_mut()
                            .unwrap()
                            .buffer = value.clone();
                        assert!(!commit_cell_edit(w, cx));
                        let model = w.csv_models.get(&id).unwrap();
                        assert_eq!(model.cell_input.as_ref().unwrap().buffer, value);
                        assert!(model.undo_stack.is_empty() && model.redo_stack.is_empty());
                        assert_eq!(w.buffer().rope.to_string(), source);
                        assert_eq!(w.buffer().raw_revision, revision);
                        assert!(!w.buffer().dirty);
                        assert_eq!(std::fs::read(path).unwrap(), disk);
                        w.csv_models
                            .get_mut(&id)
                            .unwrap()
                            .cell_input
                            .as_mut()
                            .unwrap()
                            .buffer = "smaller 中😀".into();
                        assert!(commit_cell_edit(w, cx));
                        assert!(w.buffer().rope.to_string().contains("smaller 中😀"));
                    },
                );
            }
        }
    }
}
