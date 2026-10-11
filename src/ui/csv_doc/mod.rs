// CSV / TSV viewer + editor.
//
// Mirrors the markdown preview architecture: source mode reuses the
// regular editor pane (the rope already holds the raw delimited text),
// preview mode swaps in a spreadsheet-style grid built from a per-tab
// `CsvModel`. The toolbar toggle gates on `is_csv_path` and the active
// tab's `view_mode == CsvPreview`.
//
// Model lives on `Workspace::csv_models` keyed by `Tab::session_id`.
// It's lazily built when preview mode first renders and rebuilt
// whenever the buffer's edit generation changes (typing in source view
// invalidates the model on the next preview paint).
//
// Cell edits propagate back to the rope as ONE `replace_byte_range`
// covering the whole buffer — single undo entry per commit so the
// undo grain matches user expectation. Row / column drag-reorder and
// the "has header" / delimiter / add-row / add-column controls all
// route through the same emit path.

pub(crate) mod actions;
pub(crate) mod cell_input;
pub(crate) mod clipboard;
pub(crate) mod context_menu;
pub(crate) mod fill_series;
pub(crate) mod filter_dropdown;
pub(crate) mod find_bar;
pub(crate) mod format_bar;
pub(crate) mod grid;
mod grid_input;
pub(crate) mod model;
pub(crate) mod status_bar;
pub(crate) mod undo;

pub(crate) use context_menu::{CsvContextMenuState, csv_context_menu};
pub(crate) use filter_dropdown::{FilterDropdownSnapshot, csv_filter_dropdown};
pub(crate) use format_bar::{csv_delimiter_menu, csv_format_bar};
pub(crate) use grid::{CsvGridSnapshot, csv_preview_pane};
pub(crate) use model::{CsvModel, detect_delimiter};

/// Returns true when the buffer at `path` should offer the CSV preview
/// toggle. Looks at the file extension only — content sniffing would
/// require reading the full file, which we already do separately for
/// syntax detection.
pub(crate) fn is_csv_path(path: Option<&std::path::Path>) -> bool {
    path.and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "csv" || e == "tsv"
        })
        .unwrap_or(false)
}

/// Default delimiter inferred from the file extension. `.tsv` opens with
/// TAB pre-selected even on empty buffers; everything else falls back to
/// comma. Used as the seed before `detect_delimiter` looks at the first
/// data line.
pub(crate) fn default_delimiter_for_path(path: Option<&std::path::Path>) -> u8 {
    let ext = path
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if ext.as_deref() == Some("tsv") { b'\t' } else { b',' }
}

#[cfg(test)]
mod grid_input_tests;
