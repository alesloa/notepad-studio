// CSV in-memory model — parse, hold, mutate, serialize.
//
// The model is the single source of truth for the grid view. Source
// text → model only happens through `rebuild_from_source`; every
// grid-side mutation (cell commit, column / row reorder, add row /
// column, delete row / column) re-emits the entire CSV via the `csv`
// crate's `WriterBuilder` and asks the workspace to replace the rope's
// byte range with the freshly serialized text. Round-tripping through
// the csv crate normalizes quoting, so the source view will show
// canonical RFC-4180 output after the first commit — a known and
// acceptable behaviour for an editor that exposes both views.

use std::collections::{BTreeSet, HashMap, HashSet};

use gpui::{FocusHandle, Pixels, Point, ScrollHandle, px};

/// Default width of a freshly-parsed column. User can drag the right edge
/// of any column header to resize beyond this; the value persists in
/// `col_widths` for the lifetime of the model (i.e. until the buffer is
/// closed or re-parsed).
pub(crate) const DEFAULT_COL_WIDTH_PX: f32 = 140.0;

/// Lower bound for column resize. Anything narrower than this hides the
/// content; matches the size of the resize hit-zone so dragging never
/// makes the column disappear.
pub(crate) const MIN_COL_WIDTH_PX: f32 = 48.0;

/// Upper bound for auto-fit. Drag-resize is unbounded, but double-click
/// auto-fit must stop somewhere — a single 5,000-char cell shouldn't
/// expand a column to that width.
pub(crate) const MAX_AUTO_FIT_WIDTH_PX: f32 = 600.0;

/// Padding added to measured text width when auto-fitting. Sums the
/// header cell's actual horizontal chrome — 8px left pad + 8px right pad
/// (px_2 in render_header) + 1px right border + 6px resize handle reserved
/// on the right edge — plus a 5px safety margin for antialiasing slop and
/// the gap between shape_line's measured advance and the rendered glyph
/// extent. Without enough margin the truncation kicks in and `…` shows.
pub(crate) const AUTO_FIT_PADDING_PX: f32 = 28.0;

/// Minimum width to fall back to when a column has no header text —
/// wider than `MIN_COL_WIDTH_PX` so the resize handle stays comfortably
/// grabbable after auto-fit.
pub(crate) const AUTO_FIT_EMPTY_HEADER_PX: f32 = 80.0;

/// Pixel height of every grid row including the header. Fixed so the
/// virtual scroller can compute visible rows from `scroll_top / row_h`
/// without measuring text — large CSVs (10k+ rows) need O(visible)
/// paint cost, not O(total).
pub(crate) const ROW_HEIGHT_PX: f32 = 24.0;

/// Width of the row-gutter column at the left. Carries the row number
/// and the row drag-reorder handle.
pub(crate) const ROW_GUTTER_WIDTH_PX: f32 = 56.0;

/// Width of the resize hit-zone on the right edge of each column header.
/// Mouse-down anywhere inside this band starts a column resize drag
/// instead of clicking the header for sort / drag-reorder.
pub(crate) const RESIZE_HANDLE_WIDTH_PX: f32 = 6.0;

/// Pixel distance a header / row-gutter mousedown must travel before it
/// flips from "this is a click that picks a selection" to "this is a
/// drag that reorders." Anything under this on mouseup is treated as a
/// click; anything past triggers reorder. 4 px matches what the OS
/// double-click jitter tolerance uses on macOS — feels right for
/// committing to a drag.
pub(crate) const DRAG_THRESHOLD_PX: f32 = 4.0;

#[derive(Clone, Debug)]
pub(crate) struct ColDragState {
    /// Snapshot of the logical column indices that will travel as a
    /// contiguous block on commit. Sorted ascending. When the user
    /// mouses-down on a column that is part of an existing multi-
    /// selection, the whole selection is captured here so the drag
    /// moves the group together. Single-column drags hold `[primary]`.
    pub(crate) dragged: Vec<usize>,
    /// The logical column directly under the cursor at mousedown — the
    /// anchor for drop-target calculation. Always a member of
    /// [`dragged`].
    pub(crate) primary: usize,
    pub(crate) start_mouse_x: Pixels,
    pub(crate) current_mouse_x: Pixels,
    pub(crate) drop_logical: usize,
    /// True once the cursor has moved at least [`DRAG_THRESHOLD_PX`] from
    /// the start position. While false, mouseup is treated as a plain
    /// click (selection). Once true, mouseup commits a reorder.
    pub(crate) crossed_threshold: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RowDragState {
    pub(crate) dragged: Vec<usize>,
    pub(crate) primary: usize,
    pub(crate) start_mouse_y: Pixels,
    pub(crate) current_mouse_y: Pixels,
    pub(crate) drop_logical: usize,
    pub(crate) crossed_threshold: bool,
}

/// In-flight drag state for cell-rectangle selection. Set on the
/// mousedown that landed on a body cell; updated by window-global
/// mousemove until mouseup commits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CellRangeDrag {
    pub(crate) anchor: (usize, usize),
    pub(crate) start_mouse_x: Pixels,
    pub(crate) start_mouse_y: Pixels,
    pub(crate) crossed_threshold: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ColResizeState {
    pub(crate) col_logical: usize,
    pub(crate) start_mouse_x: Pixels,
    pub(crate) start_width: Pixels,
}

/// Line terminator detected at parse time. The csv writer uses this on
/// emit so a CRLF file stays CRLF after round-trip. Default LF when the
/// source is empty or has no embedded line break yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineTerminator {
    Lf,
    Crlf,
}

impl LineTerminator {
    /// Byte sequence written between records by the csv writer.
    pub(crate) fn bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::Crlf => b"\r\n",
        }
    }
}

/// Inclusive rectangle of body cells selected via drag or Shift-click.
/// Stored as anchor (where the gesture started) + focus (where it
/// currently extends to). Either pair may hold the larger or smaller
/// coordinate — `min_*` / `max_*` normalise. Headers and the row gutter
/// are NOT part of cell ranges; whole-row / whole-column selections live
/// on the separate `selected_rows` / `selected_cols` axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellRange {
    pub(crate) anchor: (usize, usize),
    pub(crate) focus: (usize, usize),
}

impl CellRange {
    pub(crate) fn new(anchor: (usize, usize), focus: (usize, usize)) -> Self {
        Self { anchor, focus }
    }

    pub(crate) fn min_row(&self) -> usize {
        self.anchor.0.min(self.focus.0)
    }
    pub(crate) fn max_row(&self) -> usize {
        self.anchor.0.max(self.focus.0)
    }
    pub(crate) fn min_col(&self) -> usize {
        self.anchor.1.min(self.focus.1)
    }
    pub(crate) fn max_col(&self) -> usize {
        self.anchor.1.max(self.focus.1)
    }
    pub(crate) fn contains(&self, r: usize, c: usize) -> bool {
        r >= self.min_row()
            && r <= self.max_row()
            && c >= self.min_col()
            && c <= self.max_col()
    }
    pub(crate) fn is_single_cell(&self) -> bool {
        self.anchor == self.focus
    }
}

/// Transient state for an Excel-style fill-series drag. Lives on the
/// model only while the user is mid-drag on the cell-range's fill
/// handle. `source_range` is the original selection; `current_target`
/// expands as the cursor moves and is rendered as a dashed preview.
/// `commit_fill_drag` applies the pattern projection to cells outside
/// `source_range` and clears the state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FillDragState {
    pub(crate) source_range: CellRange,
    pub(crate) current_target: CellRange,
    pub(crate) anchor_window_pos: Point<Pixels>,
}

/// Pending state for an open column-filter dropdown. Lives on the model
/// only while the dropdown is visible — `commit_filter_dropdown` moves
/// `staged_hidden` onto `col_filters`, `cancel_filter_dropdown` drops it.
/// Same gesture-only state pattern as `CsvContextMenuState`.
#[derive(Clone, Debug)]
pub(crate) struct FilterDropdownState {
    /// Logical column index the dropdown is anchored to.
    pub(crate) col: usize,
    /// Window-space coordinate at which to mount the dropdown's top-left.
    pub(crate) anchor: Point<Pixels>,
    /// Pending unchecked-values set — UI mutates this freely; commit
    /// pushes it to `col_filters[col]`, cancel discards it.
    pub(crate) staged_hidden: HashSet<String>,
    /// Pre-computed sorted unique values for the column, excluding the
    /// header. Empty string is included when any row has a blank cell —
    /// rendered as `(Blanks)` by the dropdown UI.
    pub(crate) unique_values: Vec<String>,
}

/// Anchor + cursor for the cell editor's text input. Same shape as the
/// dialog inputs across the rest of the app — see
/// `style_configurator::colour_picker::HexInput` for the full pattern.
pub(crate) struct CsvCellInput {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) buffer: String,
    pub(crate) cursor: usize,
    pub(crate) anchor: usize,
    pub(crate) drag_active: bool,
}

impl CsvCellInput {
    pub(crate) fn new(focus_handle: FocusHandle, seed: String) -> Self {
        let len = seed.len();
        Self {
            focus_handle,
            buffer: seed,
            cursor: len,
            anchor: len,
            drag_active: false,
        }
    }
}

pub(crate) struct CsvModel {
    /// Header row — extracted from row 0 when `has_header` is true, or an
    /// empty vec (column count derived from the first data row) otherwise.
    pub(crate) headers: Vec<String>,
    /// Body rows. When `has_header` is false, this includes what would
    /// otherwise be row 0 of the file.
    pub(crate) rows: Vec<Vec<String>>,
    pub(crate) col_widths: Vec<Pixels>,
    /// Logical column order. Drag-reorder swaps entries here instead of
    /// mutating `headers` / `rows` directly so a drag in flight can render
    /// the columns in the new order without losing the source mapping if
    /// the user releases mid-drag. On commit (mouseup), `apply_col_order`
    /// permutes the underlying data and resets this to identity.
    pub(crate) col_order: Vec<usize>,
    /// Logical row order. Same mechanism as `col_order` but for rows.
    pub(crate) row_order: Vec<usize>,
    /// Currently-highlighted cell (no editor open). Click on a cell sets
    /// this; arrow keys while no editor is open can navigate from here.
    pub(crate) active_cell: Option<(usize, usize)>,
    /// Cell currently in edit mode. `Some((r, c))` mounts the inline
    /// text input over the (r, c) cell; `None` means no inline editor.
    pub(crate) editing_cell: Option<(usize, usize)>,
    pub(crate) cell_input: Option<CsvCellInput>,
    pub(crate) delimiter: u8,
    pub(crate) has_header: bool,
    /// Local parse/mutation version, retained for the model's own callers.
    /// Workspace invalidation uses the bound raw revision below.
    pub(crate) source_version: u64,
    pub(crate) source_state: Option<super::undo::CsvSourceState>,
    /// Snapshot awaiting an Undo/Redo write; source_state still identifies
    /// the current rope so a newer raw edit can refuse the stale write.
    pub(crate) pending_source: Option<super::undo::CsvSourceState>,
    pub(crate) reparse_requested: bool,
    pub(crate) col_drag: Option<ColDragState>,
    pub(crate) row_drag: Option<RowDragState>,
    pub(crate) col_resize: Option<ColResizeState>,
    pub(crate) scroll_handle: ScrollHandle,
    /// Logical row indices currently highlighted by the user. Click on a
    /// row gutter sets this to `{clicked}`; Cmd-click toggles membership;
    /// Shift-click sets the inclusive range between [`last_clicked_row`]
    /// and the new click. Selecting any row clears `selected_cols`.
    pub(crate) selected_rows: BTreeSet<usize>,
    pub(crate) selected_cols: BTreeSet<usize>,
    /// Body-cell rectangle selection — created by mouse drag from cell to
    /// cell or by Shift-click on a cell with an `active_cell` already
    /// set. Mutually exclusive with `selected_rows` / `selected_cols`:
    /// setting one axis clears the other. A single-cell range is
    /// allowed mid-drag but discarded on mouse-up if the cursor never
    /// crossed [`DRAG_THRESHOLD_PX`].
    pub(crate) cell_range: Option<CellRange>,
    /// True while the user is mid drag-select on a cell. Set on
    /// mousedown, flipped to `crossed_threshold = true` once movement
    /// exceeds [`DRAG_THRESHOLD_PX`]. Mouseup discards the range when
    /// `crossed_threshold` stayed false (it was just a click).
    pub(crate) cell_range_drag: Option<CellRangeDrag>,
    /// Anchor index for shift-click range selection on the row axis. Set
    /// by plain-click and cmd-click; left alone by shift-click so the
    /// user can swing the range back the other way without picking a
    /// new anchor first.
    pub(crate) last_clicked_row: Option<usize>,
    pub(crate) last_clicked_col: Option<usize>,
    /// Line terminator detected at parse time. Re-emitted on every write
    /// so a CRLF file stays CRLF and an LF file stays LF.
    pub(crate) line_terminator: LineTerminator,
    /// True when the original source ended with a line terminator. The
    /// csv writer always emits one after the last record; we strip the
    /// trailing bytes when this is false so the file's terminator
    /// presence round-trips.
    pub(crate) trailing_newline: bool,
    /// True when the original source began with a UTF-8 BOM. Stripped on
    /// parse, re-prepended on emit so files round-trip byte-identical
    /// when nothing changed.
    pub(crate) has_bom: bool,
    /// Stable focus handle for the grid container. Anchored on the
    /// outer wrapper via `.track_focus(...)` in `csv_preview_pane` so the
    /// workspace can detect "grid focused, no cell editor" and route
    /// arrow keys / F2 / Cmd+A / typed chars to grid-level handlers.
    /// `None` until the model is first built — the workspace's
    /// `ensure_csv_grid_focus` allocates it the first time the grid
    /// renders and stashes it here.
    pub(crate) grid_focus: Option<FocusHandle>,
    /// Per-tab undo stack — independent of the rope's text undo. Each
    /// snapshot captures headers, rows, col_widths, active_cell and
    /// axis selections so undo restores both data AND the surrounding
    /// view state. Pushed by mutating model methods (set_cell,
    /// delete_rows, sort_by_col, drag-reorder commit, column resize
    /// start, etc.) and consumed by `undo()` / `redo()`.
    pub(crate) undo_stack: Vec<crate::ui::csv_doc::undo::CsvSnapshot>,
    pub(crate) redo_stack: Vec<crate::ui::csv_doc::undo::CsvSnapshot>,
    /// Current Find-in-CSV query. Empty when the find bar isn't open or
    /// no query has been entered yet — in that case `find_matches` is
    /// also empty.
    pub(crate) find_query: String,
    pub(crate) find_case_sensitive: bool,
    /// When true, a cell counts as a match only when its value equals the
    /// query exactly (after case folding). When false (default), substring
    /// matches qualify.
    pub(crate) find_whole_cell: bool,
    /// Match list, in logical row × column order (top-to-bottom,
    /// left-to-right). Includes header row matches as `(usize::MAX, col)`
    /// so they round-trip the same coordinate space the editor uses.
    pub(crate) find_matches: Vec<(usize, usize)>,
    /// Index into [`find_matches`] of the currently-highlighted match.
    /// `None` when `find_matches` is empty.
    pub(crate) find_current_match: Option<usize>,
    /// True while the inline find bar is visible. Decoupled from
    /// `find_query` so the user can close the bar (Esc) without losing
    /// the query for next time.
    pub(crate) find_bar_open: bool,
    /// Stable focus handle for the find bar's text input. `None` until
    /// the bar first opens; allocated by the workspace when first shown.
    pub(crate) find_input_handle: Option<FocusHandle>,
    /// Inline-editor state for the find bar's query input.
    pub(crate) find_input: Option<CsvCellInput>,
    /// When true, the find bar renders a second row with the replace
    /// input + Replace / Replace All buttons. Cmd+H sets this; Cmd+F
    /// clears it. Persists across bar open/close so re-opening with
    /// Cmd+H restores the replace row.
    pub(crate) find_show_replace: bool,
    /// Persistent replace string — survives close/reopen the same way
    /// find_query does. Mirrored into `replace_input.buffer` while the
    /// replace input is active.
    pub(crate) replace_text: String,
    /// Stable focus handle for the find bar's replace text input. `None`
    /// until the replace row is first shown; allocated when needed.
    pub(crate) replace_input_handle: Option<FocusHandle>,
    /// Inline-editor state for the replace input. Mirrors `find_input`.
    pub(crate) replace_input: Option<CsvCellInput>,
    /// Last replace-all summary — shown briefly in the find bar counter
    /// slot after a Replace All completes. Cleared on next query change.
    pub(crate) replace_summary: Option<String>,
    /// When true, every column header renders a funnel chevron that opens
    /// the AutoFilter dropdown. Toggled globally via Cmd+Shift+L. The
    /// dropdown still works when off; this just hides the trigger.
    pub(crate) filters_visible: bool,
    /// Per-column filter — col_logical → set of values to HIDE. Absent
    /// entry (or empty set) means no filter on that column.
    pub(crate) col_filters: HashMap<usize, HashSet<String>>,
    /// Currently-open filter dropdown, or `None` when none is up. Mounted
    /// as a deferred anchored panel from `render.rs`.
    pub(crate) open_filter_dropdown: Option<FilterDropdownState>,
    /// In-flight Excel-style fill-series drag. Set on mousedown over the
    /// fill handle that sits at the bottom-right corner of `cell_range`;
    /// cleared by `commit_fill_drag` / `cancel_fill_drag` on mouseup.
    pub(crate) fill_drag: Option<FillDragState>,
}

impl CsvModel {
    pub(crate) fn empty(delimiter: u8) -> Self {
        Self {
            headers: Vec::new(),
            rows: Vec::new(),
            col_widths: Vec::new(),
            col_order: Vec::new(),
            row_order: Vec::new(),
            active_cell: None,
            editing_cell: None,
            cell_input: None,
            delimiter,
            has_header: true,
            source_version: 0,
            source_state: None,
            pending_source: None,
            reparse_requested: false,
            col_drag: None,
            row_drag: None,
            col_resize: None,
            scroll_handle: ScrollHandle::new(),
            selected_rows: BTreeSet::new(),
            selected_cols: BTreeSet::new(),
            cell_range: None,
            cell_range_drag: None,
            last_clicked_row: None,
            last_clicked_col: None,
            line_terminator: LineTerminator::Lf,
            trailing_newline: true,
            has_bom: false,
            grid_focus: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            find_query: String::new(),
            find_case_sensitive: false,
            find_whole_cell: false,
            find_matches: Vec::new(),
            find_current_match: None,
            find_bar_open: false,
            find_input_handle: None,
            find_input: None,
            find_show_replace: false,
            replace_text: String::new(),
            replace_input_handle: None,
            replace_input: None,
            replace_summary: None,
            filters_visible: false,
            col_filters: HashMap::new(),
            open_filter_dropdown: None,
            fill_drag: None,
        }
    }

    pub(crate) fn clear_axis_selections(&mut self) {
        self.selected_rows.clear();
        self.selected_cols.clear();
        self.last_clicked_row = None;
        self.last_clicked_col = None;
        // Active cell highlight is also a "selection" piece — clicking
        // elsewhere on the grid must drop it alongside row/col axis
        // state, otherwise a stale single-cell highlight survives a row
        // gutter click or a dead-space click and looks broken.
        self.active_cell = None;
        // Cell rectangle selection is a peer of row/col selection —
        // clear it on the same blur/clear paths so a dangling range
        // doesn't out-live its parent gesture.
        self.cell_range = None;
        self.cell_range_drag = None;
        // Drag / resize state from a previous gesture must not survive a
        // structural mutation (delete, insert, source rebuild). Leftover
        // col_drag pinning the next mouseup to a stale logical index was
        // the root cause of "after deleting a column, row clicks don't
        // select" — the body's mouseup saw row_drag = None but the
        // header's mouseup machinery was still poised, dragging the
        // next row-gutter click into a header-side codepath.
        self.col_drag = None;
        self.row_drag = None;
        self.col_resize = None;
        // Fill-series drag is anchored to a cell_range that we just
        // dropped — must clear too or the next paint would render a
        // dangling preview rect with no underlying selection.
        self.fill_drag = None;
    }

    /// True when the inline editor is anchored to the header row (the
    /// row == usize::MAX sentinel). Lets the grid render-path distinguish
    /// "header rename in progress" from a normal body-cell edit without
    /// every consumer having to repeat the magic-row check.
    pub(crate) fn editing_header_col(&self) -> Option<usize> {
        match self.editing_cell {
            Some((row, col)) if row == usize::MAX => Some(col),
            _ => None,
        }
    }

    /// Plain-click selection on the row gutter. Replaces the row
    /// selection with `{row}`, clears any column selection, and updates
    /// the shift-click anchor.
    pub(crate) fn select_row_plain(&mut self, row: usize) {
        self.selected_cols.clear();
        self.last_clicked_col = None;
        self.active_cell = None;
        self.cell_range = None;
        self.selected_rows.clear();
        self.selected_rows.insert(row);
        self.last_clicked_row = Some(row);
    }

    /// Cmd / Ctrl-click selection on the row gutter. Toggles membership
    /// of `row` and updates the shift-click anchor to it. Also clears
    /// any column selection (the two axes are mutually exclusive).
    pub(crate) fn select_row_toggle(&mut self, row: usize) {
        self.selected_cols.clear();
        self.last_clicked_col = None;
        self.active_cell = None;
        self.cell_range = None;
        if !self.selected_rows.insert(row) {
            self.selected_rows.remove(&row);
        }
        self.last_clicked_row = Some(row);
    }

    /// Shift-click selection on the row gutter. Sets the selection to
    /// the inclusive range between [`last_clicked_row`] and `row` (or
    /// just `{row}` when no anchor is set). Leaves the anchor alone so
    /// the user can swing the range back the other way.
    pub(crate) fn select_row_range(&mut self, row: usize) {
        self.selected_cols.clear();
        self.last_clicked_col = None;
        self.active_cell = None;
        self.cell_range = None;
        let anchor = self.last_clicked_row.unwrap_or(row);
        let (lo, hi) = if anchor <= row { (anchor, row) } else { (row, anchor) };
        self.selected_rows = (lo..=hi).collect();
    }

    pub(crate) fn select_col_plain(&mut self, col: usize) {
        self.selected_rows.clear();
        self.last_clicked_row = None;
        self.active_cell = None;
        self.cell_range = None;
        self.selected_cols.clear();
        self.selected_cols.insert(col);
        self.last_clicked_col = Some(col);
    }

    pub(crate) fn select_col_toggle(&mut self, col: usize) {
        self.selected_rows.clear();
        self.last_clicked_row = None;
        self.active_cell = None;
        self.cell_range = None;
        if !self.selected_cols.insert(col) {
            self.selected_cols.remove(&col);
        }
        self.last_clicked_col = Some(col);
    }

    pub(crate) fn select_col_range(&mut self, col: usize) {
        self.selected_rows.clear();
        self.last_clicked_row = None;
        self.active_cell = None;
        self.cell_range = None;
        let anchor = self.last_clicked_col.unwrap_or(col);
        let (lo, hi) = if anchor <= col { (anchor, col) } else { (col, anchor) };
        self.selected_cols = (lo..=hi).collect();
    }

    /// Begin a cell-range selection with `anchor` as the top-left
    /// (logical) corner. Clears row + column axis selections — the three
    /// are mutually exclusive. Caller is responsible for setting
    /// `active_cell` and `cell_range_drag` if this is a mousedown.
    pub(crate) fn select_cell_range(&mut self, anchor: (usize, usize), focus: (usize, usize)) {
        self.selected_rows.clear();
        self.selected_cols.clear();
        self.last_clicked_row = None;
        self.last_clicked_col = None;
        self.cell_range = Some(CellRange::new(anchor, focus));
    }

    /// Update the focus corner of an active range without changing the
    /// anchor. No-op when no range is set.
    pub(crate) fn extend_cell_range_focus(&mut self, focus: (usize, usize)) {
        if let Some(range) = self.cell_range.as_mut() {
            range.focus = focus;
        }
    }

    pub(crate) fn clear_cell_range(&mut self) {
        self.cell_range = None;
        self.cell_range_drag = None;
    }

    pub(crate) fn bind_source(&mut self, rope: &ropey::Rope, revision: &crate::buffer::ContentRevision) {
        self.source_state = Some(super::undo::CsvSourceState { revision: revision.clone(), rope: rope.clone() });
        self.pending_source = None;
        self.reparse_requested = false;
    }

    /// Re-parse `source` into headers + rows. Preserves user-set column
    /// widths when the column count is unchanged; otherwise resets every
    /// width to `DEFAULT_COL_WIDTH_PX`. Resets `col_order` / `row_order`
    /// to identity — any in-flight drag is implicitly cancelled because
    /// the source-side edit that triggered the rebuild is authoritative.
    pub(crate) fn rebuild_from_source(&mut self, source: &str, generation: u64) {
        self.source_state = None;
        self.pending_source = None;
        self.reparse_requested = false;
        let preserved_widths = self.col_widths.clone();

        // Detect and strip UTF-8 BOM. csv crate handles BOM internally but
        // the round-trip needs the model to remember whether one was
        // present so emit can re-prepend.
        let (had_bom, stripped) = strip_bom(source);
        self.has_bom = had_bom;

        // Detect line terminator from the first occurrence. CRLF wins
        // over LF when both are present at the same offset (a CRLF is a
        // `\r` immediately followed by `\n`, never the other way).
        self.line_terminator = detect_line_terminator(stripped);

        // Trailing newline preservation. csv writer always emits one; we
        // trim on emit when this is false.
        self.trailing_newline =
            stripped.ends_with('\n') || stripped.ends_with('\r');

        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(self.delimiter)
            .from_reader(stripped.as_bytes());

        let mut all_rows: Vec<Vec<String>> = Vec::new();
        for record in reader.records().flatten() {
            let row: Vec<String> = record.iter().map(|f| f.to_string()).collect();
            all_rows.push(row);
        }

        let col_count = all_rows
            .iter()
            .map(|r| r.len())
            .max()
            .unwrap_or(0);

        if self.has_header && !all_rows.is_empty() {
            let mut headers = all_rows.remove(0);
            normalize_row_len(&mut headers, col_count);
            self.headers = headers;
        } else {
            self.headers = (0..col_count)
                .map(|i| format!("Column {}", i + 1))
                .collect();
        }
        for row in &mut all_rows {
            normalize_row_len(row, col_count);
        }
        self.rows = all_rows;

        if preserved_widths.len() == col_count {
            self.col_widths = preserved_widths;
        } else {
            self.col_widths = vec![px(DEFAULT_COL_WIDTH_PX); col_count];
        }
        self.col_order = (0..col_count).collect();
        self.row_order = (0..self.rows.len()).collect();
        self.source_version = generation;
        // Source-side edit invalidates the CSV undo history — old
        // snapshots referenced row/col shapes that no longer match the
        // rope, so applying them could produce nonsense. The rope has
        // its own text-undo for source-mode edits.
        self.undo_stack.clear();
        self.redo_stack.clear();
        // AutoFilter state also references stale row contents / col
        // indices after a source-side rebuild. Drop both filter map
        // and any open dropdown; `filters_visible` is a UI preference
        // and survives.
        self.col_filters.clear();
        self.open_filter_dropdown = None;
        // Source-side edits can re-arrange or remove rows/cols arbitrarily;
        // any prior axis selection no longer maps cleanly to the new shape.
        self.clear_axis_selections();
        if let Some((r, c)) = self.active_cell {
            if r >= self.rows.len() || c >= col_count {
                self.active_cell = None;
            }
        }
        if let Some((r, c)) = self.editing_cell {
            if r >= self.rows.len() || c >= col_count {
                self.editing_cell = None;
                self.cell_input = None;
            }
        }
        if let Some(range) = self.cell_range {
            if range.max_row() >= self.rows.len() || range.max_col() >= col_count {
                self.cell_range = None;
            }
        }
    }

    /// Number of columns currently visible (after drag-reorder is laid
    /// out on top of the underlying data — same value pre- and post-
    /// commit because reorder is purely a permutation).
    pub(crate) fn col_count(&self) -> usize {
        self.col_order.len()
    }

    pub(crate) fn row_count(&self) -> usize {
        self.row_order.len()
    }

    /// Read a cell value using logical (display) coordinates. Returns an
    /// empty string for out-of-range cells so the grid render path never
    /// has to special-case missing values.
    pub(crate) fn cell(&self, row_logical: usize, col_logical: usize) -> &str {
        let Some(&row_orig) = self.row_order.get(row_logical) else {
            return "";
        };
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return "";
        };
        self.rows
            .get(row_orig)
            .and_then(|r| r.get(col_orig))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    pub(crate) fn header(&self, col_logical: usize) -> &str {
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return "";
        };
        self.headers
            .get(col_orig)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Set a cell value using logical (display) coordinates. Returns
    /// true when the value actually changed so the caller can skip the
    /// re-emit when the commit is a no-op. Pushes undo only when the
    /// cell actually changes — typing the same value as the existing
    /// one is a no-op for the user and should not bury history.
    pub(crate) fn set_cell(
        &mut self,
        row_logical: usize,
        col_logical: usize,
        value: String,
    ) -> bool {
        let Some(&row_orig) = self.row_order.get(row_logical) else {
            return false;
        };
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return false;
        };
        {
            let Some(row) = self.rows.get(row_orig) else {
                return false;
            };
            if row.get(col_orig).map(|s| s.as_str()) == Some(value.as_str()) {
                return false;
            }
        }
        self.push_undo();
        // push_undo only snapshots, so the row found above is still here.
        let Some(row) = self.rows.get_mut(row_orig) else {
            self.pop_undo_no_restore();
            return false;
        };
        if row.len() <= col_orig {
            row.resize(col_orig + 1, String::new());
        }
        row[col_orig] = value;
        true
    }

    pub(crate) fn set_header(&mut self, col_logical: usize, value: String) -> bool {
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return false;
        };
        if self.headers.get(col_orig).map(|s| s.as_str()) == Some(value.as_str()) {
            return false;
        }
        self.push_undo();
        if self.headers.len() <= col_orig {
            self.headers.resize(col_orig + 1, String::new());
        }
        self.headers[col_orig] = value;
        true
    }

    pub(crate) fn col_width(&self, col_logical: usize) -> Pixels {
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return px(DEFAULT_COL_WIDTH_PX);
        };
        self.col_widths
            .get(col_orig)
            .copied()
            .unwrap_or(px(DEFAULT_COL_WIDTH_PX))
    }

    pub(crate) fn set_col_width(&mut self, col_logical: usize, w: Pixels) {
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return;
        };
        let clamped = w.max(px(MIN_COL_WIDTH_PX));
        if self.col_widths.len() <= col_orig {
            self.col_widths.resize(col_orig + 1, px(DEFAULT_COL_WIDTH_PX));
        }
        self.col_widths[col_orig] = clamped;
    }

    /// Resize `col_logical` so the header text fits plus padding,
    /// clamped to [MIN, MAX]. Body cell widths are ignored — a single
    /// huge cell could otherwise stretch a column off-screen and make
    /// the resize handle unreachable. Empty headers fall back to
    /// `AUTO_FIT_EMPTY_HEADER_PX` so the handle stays grabbable. Pass
    /// in a text-width measurer so this method stays usable from unit
    /// tests (no `Window` in scope there); the grid handler hands over
    /// a closure backed by `window.text_system().shape_line(...)`.
    /// Pushes one undo entry per call.
    pub(crate) fn auto_fit_column(
        &mut self,
        col_logical: usize,
        measure: impl Fn(&str) -> f32,
    ) {
        if col_logical >= self.col_count() {
            return;
        }
        self.push_undo();
        self.auto_fit_column_inner(col_logical, measure);
    }

    /// Bare auto-fit — no undo push. Used internally by `auto_fit_all_columns`
    /// so all columns roll into a single undo grain.
    fn auto_fit_column_inner(
        &mut self,
        col_logical: usize,
        measure: impl Fn(&str) -> f32,
    ) {
        if col_logical >= self.col_count() {
            return;
        }
        let header = self.header(col_logical);
        let fitted = if header.is_empty() {
            AUTO_FIT_EMPTY_HEADER_PX
        } else {
            (measure(header) + AUTO_FIT_PADDING_PX)
                .clamp(MIN_COL_WIDTH_PX, MAX_AUTO_FIT_WIDTH_PX)
        };
        self.set_col_width(col_logical, px(fitted));
    }

    /// Auto-fit every visible column using the same measurer. One
    /// undo grain for the whole sweep, not one per column.
    pub(crate) fn auto_fit_all_columns(&mut self, measure: impl Fn(&str) -> f32) {
        if self.col_count() == 0 {
            return;
        }
        self.push_undo();
        for c in 0..self.col_count() {
            self.auto_fit_column_inner(c, &measure);
        }
    }

    /// Total content width — sum of all visible column widths. Used for
    /// horizontal scroll extent calculation. Excludes the row gutter.
    pub(crate) fn total_content_width(&self) -> Pixels {
        let mut total = px(0.);
        for i in 0..self.col_count() {
            total += self.col_width(i);
        }
        total
    }

    /// Commit a column drag — physically swap the underlying column
    /// indices so the source emit reflects the new ordering.
    pub(crate) fn apply_col_order(&mut self) {
        let order = self.col_order.clone();
        let n = order.len();
        if order.iter().enumerate().all(|(i, &v)| i == v) {
            return;
        }
        // Active filters reference logical column indices; re-key them
        // before the underlying data permutation so filter entries
        // continue to point at the same column they did before.
        self.remap_filters_after_col_reorder(&order);
        let new_headers: Vec<String> = (0..n)
            .map(|i| {
                self.headers
                    .get(order[i])
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        let new_widths: Vec<Pixels> = (0..n)
            .map(|i| {
                self.col_widths
                    .get(order[i])
                    .copied()
                    .unwrap_or(px(DEFAULT_COL_WIDTH_PX))
            })
            .collect();
        let new_rows: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|row| {
                (0..n)
                    .map(|i| row.get(order[i]).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
        self.headers = new_headers;
        self.col_widths = new_widths;
        self.rows = new_rows;
        self.col_order = (0..n).collect();
    }

    pub(crate) fn apply_row_order(&mut self) {
        let order = self.row_order.clone();
        let n = order.len();
        if order.iter().enumerate().all(|(i, &v)| i == v) {
            return;
        }
        let new_rows: Vec<Vec<String>> = (0..n)
            .map(|i| self.rows.get(order[i]).cloned().unwrap_or_default())
            .collect();
        self.rows = new_rows;
        self.row_order = (0..n).collect();
    }

    /// Sort the visible rows by the values in `col_logical`, ascending
    /// when `ascending` is true. Comparison is smart: when every cell in
    /// the column parses as `f64` the sort is numeric; otherwise it
    /// falls back to case-insensitive string compare. Stable: equal
    /// keys keep their original relative order. Mutates `row_order`
    /// only — the underlying `rows` keep their original byte positions
    /// so subsequent emit honours the new sequence without disturbing
    /// row identities. Clears `selected_rows` + `active_cell` because
    /// their logical indices would otherwise reference stale positions
    /// after the permutation. Bumps `source_version` so the caller's
    /// emit pipeline picks the change up.
    pub(crate) fn sort_by_col(&mut self, col_logical: usize, ascending: bool) {
        if col_logical >= self.col_count() || self.row_count() == 0 {
            return;
        }
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return;
        };
        // Push undo up-front, pop later if the sort turns out to be a
        // no-op (rows were already in the requested order).
        self.push_undo();
        // Phase 1: pre-compute parsed values so the comparator stays
        // pure. f64 parse is attempted for every cell; numeric path
        // wins only when EVERY cell yields Some — mixed columns fall
        // back to string compare. Trim whitespace before parsing so
        // " 12 " still counts as numeric.
        let mut all_numeric = true;
        let mut numeric: Vec<f64> = Vec::with_capacity(self.row_order.len());
        let mut text: Vec<String> = Vec::with_capacity(self.row_order.len());
        for &row_orig in &self.row_order {
            let cell = self
                .rows
                .get(row_orig)
                .and_then(|r| r.get(col_orig))
                .map(|s| s.as_str())
                .unwrap_or("");
            text.push(cell.to_string());
            if all_numeric {
                match cell.trim().parse::<f64>() {
                    Ok(v) => numeric.push(v),
                    Err(_) => all_numeric = false,
                }
            }
        }
        // Phase 2: build a permutation index over (logical_pos, original
        // row), sort with the chosen comparator. sort_by is stable in
        // std — equal keys preserve their input order.
        let mut indices: Vec<usize> = (0..self.row_order.len()).collect();
        if all_numeric {
            indices.sort_by(|&a, &b| {
                let ord = numeric[a]
                    .partial_cmp(&numeric[b])
                    .unwrap_or(std::cmp::Ordering::Equal);
                if ascending { ord } else { ord.reverse() }
            });
        } else {
            indices.sort_by(|&a, &b| {
                let ord = text[a].to_lowercase().cmp(&text[b].to_lowercase());
                if ascending { ord } else { ord.reverse() }
            });
        }
        let new_order: Vec<usize> = indices.iter().map(|&i| self.row_order[i]).collect();
        let changed = new_order != self.row_order;
        self.row_order = new_order;
        // emit_source walks `self.rows` directly, so the permutation has
        // to land in the underlying data — mirror what drag-reorder
        // does on commit. apply_row_order resets row_order to identity
        // afterwards so subsequent index math stays straightforward.
        if changed {
            self.apply_row_order();
        }
        // Selection invalidation: row indices change meaning after the
        // permutation — clearing avoids "row 3 was foo, now bar".
        self.selected_rows.clear();
        self.last_clicked_row = None;
        self.active_cell = None;
        if changed {
            self.source_version = self.source_version.wrapping_add(1).max(1);
        } else {
            // No-op sort — drop the speculative undo entry we pushed
            // above so the user's history isn't littered with right-
            // click-sort actions that didn't move anything.
            self.pop_undo_no_restore();
        }
    }

    /// Move the logical rows in `dragged` so they land as a contiguous
    /// block at `target_logical` (the position the user dropped on, in
    /// the CURRENT logical order). Handles both single-row and multi-
    /// row moves; `dragged` must be sorted ascending. Updates row_order,
    /// permutes the underlying data, sets `selected_rows` to the new
    /// positions of the moved block, and returns `(new_primary, changed)`
    /// — `changed` is true when the move actually moved anything (used
    /// by callers to skip the source re-emit on no-op drops, which would
    /// otherwise dirty the file for no reason).
    ///
    /// Algorithm: pull `extracted` (the original row indices behind each
    /// dragged entry) out of `row_order`, splice them at the target
    /// (adjusted down by the count of dragged entries that sat before
    /// the target — since those slots disappeared from the filtered
    /// list). Then `apply_row_order` permutes `self.rows` and resets
    /// `row_order` to identity, so subsequent indices line up.
    pub(crate) fn move_row_block(
        &mut self,
        dragged: &[usize],
        primary: usize,
        target_logical: usize,
    ) -> (usize, bool) {
        if dragged.is_empty() {
            return (primary, false);
        }
        // Push undo before the move; pop after if no_op.
        self.push_undo();
        let dragged_set: BTreeSet<usize> = dragged.iter().copied().collect();
        let extracted: Vec<usize> = dragged
            .iter()
            .filter_map(|&i| self.row_order.get(i).copied())
            .collect();
        let mut new_order: Vec<usize> = self
            .row_order
            .iter()
            .enumerate()
            .filter(|(i, _)| !dragged_set.contains(i))
            .map(|(_, &v)| v)
            .collect();
        let removed_before = dragged.iter().filter(|&&i| i < target_logical).count();
        let mut drop = target_logical.saturating_sub(removed_before);
        drop = drop.min(new_order.len());
        new_order.splice(drop..drop, extracted.iter().copied());
        // No-op detection — if the proposed order equals the current
        // order, the user dropped the block exactly where it started.
        // Skip the data mutation and tell the caller it doesn't need to
        // re-emit.
        let did_change = new_order != self.row_order;
        self.row_order = new_order;
        if did_change {
            self.apply_row_order();
        } else {
            self.row_order = (0..self.rows.len()).collect();
            // Drag ended exactly where it started — drop the undo
            // entry pushed at the top of this function.
            self.pop_undo_no_restore();
        }
        let new_selected: BTreeSet<usize> = (drop..drop + dragged.len()).collect();
        self.selected_rows = new_selected;
        let primary_offset = dragged.iter().position(|&i| i == primary).unwrap_or(0);
        let new_primary = drop + primary_offset;
        self.last_clicked_row = Some(new_primary);
        (new_primary, did_change)
    }

    pub(crate) fn move_col_block(
        &mut self,
        dragged: &[usize],
        primary: usize,
        target_logical: usize,
    ) -> (usize, bool) {
        if dragged.is_empty() {
            return (primary, false);
        }
        self.push_undo();
        let dragged_set: BTreeSet<usize> = dragged.iter().copied().collect();
        let extracted: Vec<usize> = dragged
            .iter()
            .filter_map(|&i| self.col_order.get(i).copied())
            .collect();
        let mut new_order: Vec<usize> = self
            .col_order
            .iter()
            .enumerate()
            .filter(|(i, _)| !dragged_set.contains(i))
            .map(|(_, &v)| v)
            .collect();
        let removed_before = dragged.iter().filter(|&&i| i < target_logical).count();
        let mut drop = target_logical.saturating_sub(removed_before);
        drop = drop.min(new_order.len());
        new_order.splice(drop..drop, extracted.iter().copied());
        let did_change = new_order != self.col_order;
        self.col_order = new_order;
        if did_change {
            self.apply_col_order();
        } else {
            self.col_order = (0..self.headers.len()).collect();
            self.pop_undo_no_restore();
        }
        let new_selected: BTreeSet<usize> = (drop..drop + dragged.len()).collect();
        self.selected_cols = new_selected;
        let primary_offset = dragged.iter().position(|&i| i == primary).unwrap_or(0);
        let new_primary = drop + primary_offset;
        self.last_clicked_col = Some(new_primary);
        (new_primary, did_change)
    }

    pub(crate) fn add_row(&mut self) {
        self.push_undo();
        let col_count = self.col_count().max(1);
        self.rows.push(vec![String::new(); col_count]);
        self.row_order.push(self.row_order.len());
    }

    pub(crate) fn add_column(&mut self) {
        self.push_undo();
        let new_orig = self.headers.len();
        let label = format!("Column {}", new_orig + 1);
        self.headers.push(label);
        for row in &mut self.rows {
            row.push(String::new());
        }
        self.col_widths.push(px(DEFAULT_COL_WIDTH_PX));
        self.col_order.push(new_orig);
    }

    /// Delete-key handler when the grid (not a cell) is focused.
    /// Priority: cell range → col selection → row selection → active
    /// cell clear → no-op. Returns true when something changed so the
    /// caller emits source.
    ///
    /// Excel-style: Delete on a cell range BLANKS the cells inside the
    /// rectangle rather than removing them. Whole-row / whole-column
    /// selections still delete the row / column entirely — that's the
    /// existing behaviour, preserved here.
    pub(crate) fn handle_delete_key(&mut self) -> bool {
        if let Some(range) = self.cell_range {
            if !range.is_single_cell() {
                return self.clear_range(range);
            }
        }
        if !self.selected_cols.is_empty() && self.selected_rows.is_empty() {
            let sel = self.selected_cols.clone();
            self.delete_cols(&sel);
            return true;
        }
        if !self.selected_rows.is_empty() && self.selected_cols.is_empty() {
            let sel = self.selected_rows.clone();
            self.delete_rows(&sel);
            return true;
        }
        if let Some((r, c)) = self.active_cell {
            return self.set_cell(r, c, String::new());
        }
        false
    }

    pub(crate) fn delete_row(&mut self, row_logical: usize) {
        let Some(&row_orig) = self.row_order.get(row_logical) else {
            return;
        };
        if row_orig >= self.rows.len() {
            return;
        }
        self.rows.remove(row_orig);
        // Re-index `row_order` so existing entries pointing past the
        // removed row shift down by one, and drop the entry that
        // referenced the deleted row.
        self.row_order = (0..self.rows.len()).collect();
        if let Some((r, _)) = self.active_cell {
            if r >= self.rows.len() {
                self.active_cell = None;
            }
        }
        if let Some((r, _)) = self.editing_cell {
            if r >= self.rows.len() {
                self.editing_cell = None;
                self.cell_input = None;
            }
        }
    }

    pub(crate) fn delete_column(&mut self, col_logical: usize) {
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return;
        };
        if col_orig >= self.headers.len() && self.rows.iter().all(|r| col_orig >= r.len()) {
            return;
        }
        if col_orig < self.headers.len() {
            self.headers.remove(col_orig);
        }
        for row in &mut self.rows {
            if col_orig < row.len() {
                row.remove(col_orig);
            }
        }
        if col_orig < self.col_widths.len() {
            self.col_widths.remove(col_orig);
        }
        let new_col_count = self.headers.len().max(
            self.rows.iter().map(|r| r.len()).max().unwrap_or(0),
        );
        self.col_order = (0..new_col_count).collect();
        if let Some((_, c)) = self.active_cell {
            if c >= new_col_count {
                self.active_cell = None;
            }
        }
        if let Some((_, c)) = self.editing_cell {
            if c >= new_col_count {
                self.editing_cell = None;
                self.cell_input = None;
            }
        }
    }

    /// Delete every logical row in `rows`. Sorted descending so earlier
    /// indices don't shift later ones during the per-row removal. Re-
    /// emits row_order at the end so the model is back to identity.
    /// One undo grain for the whole batch.
    pub(crate) fn delete_rows(&mut self, rows: &BTreeSet<usize>) {
        if rows.is_empty() {
            return;
        }
        self.push_undo();
        let mut sorted: Vec<usize> = rows.iter().copied().collect();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        for row in sorted {
            self.delete_row(row);
        }
        self.clear_axis_selections();
    }

    pub(crate) fn delete_cols(&mut self, cols: &BTreeSet<usize>) {
        if cols.is_empty() {
            return;
        }
        self.push_undo();
        let mut sorted: Vec<usize> = cols.iter().copied().collect();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        for col in sorted {
            self.delete_column(col);
        }
        // Drop / shift filter keys for the deleted columns. Snapshot is
        // already pushed above so the pre-state is preserved.
        self.remap_filters_after_col_delete(cols);
        self.clear_axis_selections();
    }

    /// Insert a blank row before `row_logical` (when `above` is true) or
    /// after it. Resets `row_order` to identity afterwards so the new
    /// row sits in the expected display position.
    pub(crate) fn insert_row_at(&mut self, row_logical: usize, above: bool) {
        self.push_undo();
        let col_count = self.col_count().max(1);
        let total = self.row_order.len();
        // Resolve the underlying row index. When the model has no rows
        // yet we just push an empty row and rebuild row_order.
        if total == 0 {
            self.rows.push(vec![String::new(); col_count]);
            self.row_order = (0..self.rows.len()).collect();
            return;
        }
        // Resolve to underlying. Easiest: materialise the current order
        // into the rows vec, then insert at the logical index.
        self.apply_row_order();
        let target = if above { row_logical } else { row_logical.saturating_add(1) };
        let target = target.min(self.rows.len());
        self.rows.insert(target, vec![String::new(); col_count]);
        self.row_order = (0..self.rows.len()).collect();
    }

    pub(crate) fn insert_col_at(&mut self, col_logical: usize, left: bool) {
        let total = self.col_order.len();
        if total == 0 {
            // add_column already pushes undo — don't double up here.
            self.add_column();
            return;
        }
        self.push_undo();
        self.apply_col_order();
        let target = if left { col_logical } else { col_logical.saturating_add(1) };
        let target = target.min(self.headers.len());
        // Slide filters above the insert point up by one so they stay
        // glued to the same data column they were tracking.
        self.remap_filters_after_col_insert(target);
        let label = format!("Column {}", target + 1);
        self.headers.insert(target, label);
        for row in &mut self.rows {
            if row.len() >= target {
                row.insert(target, String::new());
            } else {
                row.resize(target, String::new());
                row.push(String::new());
            }
        }
        if self.col_widths.len() >= target {
            self.col_widths.insert(target, px(DEFAULT_COL_WIDTH_PX));
        } else {
            self.col_widths
                .resize(target, px(DEFAULT_COL_WIDTH_PX));
            self.col_widths.push(px(DEFAULT_COL_WIDTH_PX));
        }
        self.col_order = (0..self.headers.len()).collect();
    }

    /// Blank out every cell on the given logical rows. Headers are left
    /// untouched even if the row selection covers row 0 of the body.
    /// Pushes one undo grain; pops it if no cells actually changed.
    pub(crate) fn clear_rows(&mut self, rows: &BTreeSet<usize>) -> bool {
        self.push_undo();
        let mut changed = false;
        for &r in rows {
            let Some(&orig) = self.row_order.get(r) else {
                continue;
            };
            let Some(row) = self.rows.get_mut(orig) else {
                continue;
            };
            for cell in row.iter_mut() {
                if !cell.is_empty() {
                    cell.clear();
                    changed = true;
                }
            }
        }
        if !changed {
            self.pop_undo_no_restore();
        }
        changed
    }

    /// Blank out every cell on the given logical columns, plus their
    /// headers when `has_header` is true.
    pub(crate) fn clear_cols(&mut self, cols: &BTreeSet<usize>) -> bool {
        self.push_undo();
        let mut changed = false;
        for &c in cols {
            let Some(&orig) = self.col_order.get(c) else {
                continue;
            };
            if self.has_header {
                if let Some(h) = self.headers.get_mut(orig) {
                    if !h.is_empty() {
                        h.clear();
                        changed = true;
                    }
                }
            }
            for row in &mut self.rows {
                if let Some(cell) = row.get_mut(orig) {
                    if !cell.is_empty() {
                        cell.clear();
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            self.pop_undo_no_restore();
        }
        changed
    }

    /// Blank every cell inside the given rectangle. Used by Cut on a
    /// cell range — emptied cells stay in place rather than being
    /// removed (that's `delete_rows` / `delete_cols`). One undo grain.
    pub(crate) fn clear_range(&mut self, range: CellRange) -> bool {
        let row_count = self.row_count();
        let col_count = self.col_count();
        if row_count == 0 || col_count == 0 {
            return false;
        }
        self.push_undo();
        let r0 = range.min_row();
        let r1 = range.max_row().min(row_count - 1);
        let c0 = range.min_col();
        let c1 = range.max_col().min(col_count - 1);
        let mut changed = false;
        for r in r0..=r1 {
            let Some(&row_orig) = self.row_order.get(r) else {
                continue;
            };
            let Some(row) = self.rows.get_mut(row_orig) else {
                continue;
            };
            for c in c0..=c1 {
                let Some(&col_orig) = self.col_order.get(c) else {
                    continue;
                };
                if col_orig >= row.len() {
                    continue;
                }
                if !row[col_orig].is_empty() {
                    row[col_orig].clear();
                    changed = true;
                }
            }
        }
        if !changed {
            self.pop_undo_no_restore();
        }
        changed
    }

    /// Paste a 2-D block of cell values starting at logical
    /// `(anchor_row, anchor_col)`. Grows the grid (extra rows / cols)
    /// when the data extends past the current dimensions — matches
    /// Excel's "extend the sheet" paste behaviour. Returns true if any
    /// cell value changed (so the caller knows whether to emit source).
    /// One undo grain.
    ///
    /// `fill_target` is honoured when the source block is exactly 1×1
    /// and `fill_target` is `Some(rect)`: the single value is written
    /// to every cell inside `rect`. This implements the Excel /
    /// Sheets "paste one cell into a selection fills the selection"
    /// behaviour.
    pub(crate) fn paste_at(
        &mut self,
        anchor_row: usize,
        anchor_col: usize,
        data: &[Vec<String>],
        fill_target: Option<CellRange>,
    ) -> bool {
        if data.is_empty() {
            return false;
        }
        let src_rows = data.len();
        let src_cols = data.iter().map(|r| r.len()).max().unwrap_or(0);
        if src_cols == 0 {
            return false;
        }

        self.push_undo();

        // 1×1 fill — broadcast the single source cell across every
        // target cell in the supplied range.
        if src_rows == 1 && src_cols == 1 {
            if let Some(range) = fill_target {
                let value = data[0].first().cloned().unwrap_or_default();
                let r0 = range.min_row();
                let r1 = range.max_row();
                let c0 = range.min_col();
                let c1 = range.max_col();
                self.grow_to_accommodate(r1, c1);
                // Growth stops at the last index `usize` can count, so
                // walk only the cells that now exist.
                let r1 = r1.min(self.row_count().saturating_sub(1));
                let c1 = c1.min(self.col_count().saturating_sub(1));
                let mut changed = false;
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        if self.write_cell_raw(r, c, &value) {
                            changed = true;
                        }
                    }
                }
                if !changed {
                    self.pop_undo_no_restore();
                }
                return changed;
            }
        }

        // Repeat-tile — multi-cell source pasted into a target range
        // whose dimensions are an exact multiple of the source. Matches
        // Excel: copy 1×2, select 1×6, paste → 3 tiled copies. Plain
        // anchor-only paste falls through when the shapes don't fit.
        if let Some(range) = fill_target {
            let r0 = range.min_row();
            let r1 = range.max_row();
            let c0 = range.min_col();
            let c1 = range.max_col();
            let tgt_rows = (r1 - r0).saturating_add(1);
            let tgt_cols = (c1 - c0).saturating_add(1);
            let row_fits = tgt_rows > src_rows && tgt_rows % src_rows == 0;
            let col_fits = tgt_cols > src_cols && tgt_cols % src_cols == 0;
            let row_match = tgt_rows == src_rows;
            let col_match = tgt_cols == src_cols;
            // Tile only when at least one axis is a strict multiple and
            // the other axis either matches or is also a strict multiple.
            if (row_fits || row_match) && (col_fits || col_match) && (row_fits || col_fits) {
                self.grow_to_accommodate(r1, c1);
                let r1 = r1.min(self.row_count().saturating_sub(1));
                let c1 = c1.min(self.col_count().saturating_sub(1));
                let mut changed = false;
                for r in r0..=r1 {
                    let src_r = (r - r0) % src_rows;
                    let src_row = &data[src_r];
                    if src_row.is_empty() {
                        continue;
                    }
                    let src_row_cols = src_row.len();
                    for c in c0..=c1 {
                        let src_c = (c - c0) % src_cols;
                        let value = if src_c < src_row_cols {
                            src_row[src_c].as_str()
                        } else {
                            ""
                        };
                        if self.write_cell_raw(r, c, value) {
                            changed = true;
                        }
                    }
                }
                if !changed {
                    self.pop_undo_no_restore();
                }
                return changed;
            }
        }

        // Block paste — write source[i][j] to (anchor + i, anchor + j),
        // growing the model as needed. Empty source rows (csv parsed an
        // empty trailing line) are skipped.
        let (Some(target_max_row), Some(target_max_col)) = (
            anchor_row.checked_add(src_rows.saturating_sub(1)),
            anchor_col.checked_add(src_cols.saturating_sub(1)),
        ) else {
            // The block would end past the last index `usize` can count.
            self.pop_undo_no_restore();
            return false;
        };
        self.grow_to_accommodate(target_max_row, target_max_col);

        let mut changed = false;
        for (i, row) in data.iter().enumerate() {
            if row.is_empty() {
                continue;
            }
            for (j, value) in row.iter().enumerate() {
                let r = anchor_row + i;
                let c = anchor_col + j;
                if self.write_cell_raw(r, c, value) {
                    changed = true;
                }
            }
        }
        if !changed {
            self.pop_undo_no_restore();
        }
        changed
    }

    /// Cmd+D fill-down: copy the top row of `range` into every row
    /// below it within `range`. One undo grain. Returns true if any
    /// cell actually changed.
    pub(crate) fn fill_down_range(&mut self, range: CellRange) -> bool {
        let row_count = self.row_count();
        let col_count = self.col_count();
        if row_count == 0 || col_count == 0 {
            return false;
        }
        let r0 = range.min_row();
        let r1 = range.max_row().min(row_count.saturating_sub(1));
        let c0 = range.min_col();
        let c1 = range.max_col().min(col_count.saturating_sub(1));
        if r1 <= r0 {
            return false;
        }
        let src: Vec<String> = (c0..=c1).map(|c| self.cell(r0, c).to_string()).collect();
        self.push_undo();
        let mut changed = false;
        for r in (r0 + 1)..=r1 {
            for (i, c) in (c0..=c1).enumerate() {
                if self.write_cell_raw(r, c, &src[i]) {
                    changed = true;
                }
            }
        }
        if !changed {
            self.pop_undo_no_restore();
        }
        changed
    }

    /// Cmd+R fill-right: copy the left column of `range` into every
    /// column to its right within `range`. One undo grain. Returns true
    /// if any cell actually changed.
    pub(crate) fn fill_right_range(&mut self, range: CellRange) -> bool {
        let row_count = self.row_count();
        let col_count = self.col_count();
        if row_count == 0 || col_count == 0 {
            return false;
        }
        let r0 = range.min_row();
        let r1 = range.max_row().min(row_count.saturating_sub(1));
        let c0 = range.min_col();
        let c1 = range.max_col().min(col_count.saturating_sub(1));
        if c1 <= c0 {
            return false;
        }
        let src: Vec<String> = (r0..=r1).map(|r| self.cell(r, c0).to_string()).collect();
        self.push_undo();
        let mut changed = false;
        for (i, r) in (r0..=r1).enumerate() {
            for c in (c0 + 1)..=c1 {
                if self.write_cell_raw(r, c, &src[i]) {
                    changed = true;
                }
            }
        }
        if !changed {
            self.pop_undo_no_restore();
        }
        changed
    }

    /// Internal helper: extend `rows` and `headers` so logical
    /// coordinates `(target_row, target_col)` are addressable. Used by
    /// paste to grow the sheet rather than clipping at the existing
    /// boundary.
    fn grow_to_accommodate(&mut self, target_row: usize, target_col: usize) {
        // An axis at `usize::MAX` has no count after it and is left as
        // it is; callers clamp their writes to the cells that exist.
        // Add columns first so freshly-added rows have the right width.
        if let Some(needed_cols) = target_col.checked_add(1) {
            while self.col_count() < needed_cols {
                self.add_column_silent();
            }
        }
        if let Some(needed_rows) = target_row.checked_add(1) {
            while self.row_count() < needed_rows {
                self.add_row_silent();
            }
        }
    }

    /// Same as `add_column` but skips the undo push — used by
    /// `grow_to_accommodate` which lives inside a single undo grain.
    fn add_column_silent(&mut self) {
        let new_orig = self.headers.len();
        let label = format!("Column {}", new_orig + 1);
        self.headers.push(label);
        for row in &mut self.rows {
            row.push(String::new());
        }
        self.col_widths.push(px(DEFAULT_COL_WIDTH_PX));
        self.col_order.push(new_orig);
    }

    /// Same as `add_row` but skips the undo push.
    fn add_row_silent(&mut self) {
        let col_count = self.col_count().max(1);
        self.rows.push(vec![String::new(); col_count]);
        self.row_order.push(self.row_order.len());
    }

    /// Internal: write a single cell value via logical coords, growing
    /// the underlying row if necessary. Returns true on actual change.
    /// Skips the push_undo / source_version handshake — called from
    /// inside larger mutations (paste) that own the undo grain.
    fn write_cell_raw(&mut self, row_logical: usize, col_logical: usize, value: &str) -> bool {
        let Some(&row_orig) = self.row_order.get(row_logical) else {
            return false;
        };
        let Some(&col_orig) = self.col_order.get(col_logical) else {
            return false;
        };
        let Some(row) = self.rows.get_mut(row_orig) else {
            return false;
        };
        if row.len() <= col_orig {
            row.resize(col_orig + 1, String::new());
        }
        if row[col_orig] == value {
            return false;
        }
        row[col_orig].clear();
        row[col_orig].push_str(value);
        true
    }

    // ---- Fill-series drag ----------------------------------------------
    //
    // Excel-style fill handle: the user drags the bottom-right corner of
    // a cell range, and on release the cells outside the original
    // selection are filled with a pattern projected from the source
    // values (numeric increment, date increment, weekday cycle, etc.).

    /// Arm a fill-series drag. Returns true when the drag was armed —
    /// requires an existing `cell_range`. Captures the source rectangle
    /// and seeds `current_target` to the source, so a zero-distance
    /// release produces no change.
    pub(crate) fn begin_fill_drag(&mut self, anchor_window_pos: Point<Pixels>) -> bool {
        let Some(range) = self.cell_range else {
            return false;
        };
        self.fill_drag = Some(FillDragState {
            source_range: range,
            current_target: range,
            anchor_window_pos,
        });
        true
    }

    /// Drop the in-flight fill drag without applying any cells. Used by
    /// Esc / mouse-up-outside / mouse-up-on-source.
    #[allow(dead_code)]
    pub(crate) fn cancel_fill_drag(&mut self) {
        self.fill_drag = None;
    }

    /// Update the preview target based on the cell the cursor is over.
    /// Picks the dominant axis (larger pixel delta) from the source's
    /// bottom-right corner and clamps the other axis to the source
    /// bounds — Excel only fills along one axis per drag.
    ///
    /// `hovered` is the (row, col) the mouse currently sits over. The
    /// caller has already clamped it to the visible grid bounds.
    /// `pixel_delta_x` / `pixel_delta_y` are the signed mouse-move
    /// distances from the drag-start window position, used to pick the
    /// dominant axis when the cursor leaves the source corner
    /// diagonally.
    pub(crate) fn update_fill_drag(
        &mut self,
        hovered: (usize, usize),
        pixel_delta_x: f32,
        pixel_delta_y: f32,
    ) {
        let Some(state) = self.fill_drag.as_mut() else {
            return;
        };
        let src = state.source_range;
        let (hr, hc) = hovered;

        // Determine which axis the cursor is extending along.
        let extends_vert = hr < src.min_row() || hr > src.max_row();
        let extends_horiz = hc < src.min_col() || hc > src.max_col();

        // Diagonal — pick the larger pixel delta. When both axes are
        // outside the source rect this resolves the ambiguity. When the
        // cursor is inside the source on one axis, that axis simply
        // contributes nothing — the other one wins.
        let prefer_vertical = match (extends_vert, extends_horiz) {
            (true, true) => pixel_delta_y.abs() >= pixel_delta_x.abs(),
            (true, false) => true,
            (false, true) => false,
            (false, false) => true, // inside source — collapse to source
        };

        let target = if prefer_vertical {
            // Extend rows; clamp cols to source bounds.
            let (top, bot) = if hr < src.min_row() {
                (hr, src.max_row())
            } else if hr > src.max_row() {
                (src.min_row(), hr)
            } else {
                (src.min_row(), src.max_row())
            };
            CellRange::new((top, src.min_col()), (bot, src.max_col()))
        } else {
            // Extend cols; clamp rows to source bounds.
            let (left, right) = if hc < src.min_col() {
                (hc, src.max_col())
            } else if hc > src.max_col() {
                (src.min_col(), hc)
            } else {
                (src.min_col(), src.max_col())
            };
            CellRange::new((src.min_row(), left), (src.max_row(), right))
        };

        state.current_target = target;
    }

    /// Apply the fill — write projected values into every cell in
    /// `current_target` that is NOT inside `source_range`. Pushes one
    /// undo grain, sets `cell_range` to the target (Excel parity), and
    /// clears `fill_drag`. Returns true when at least one cell changed.
    pub(crate) fn commit_fill_drag(&mut self) -> bool {
        let Some(state) = self.fill_drag.take() else {
            return false;
        };
        let src = state.source_range;
        let target = state.current_target;
        // No expansion — keep the source selection, no fill.
        if target == src {
            return false;
        }

        // Determine direction of expansion. Only one of these is true
        // at a time after `update_fill_drag`'s clamp.
        let extend_down = target.max_row() > src.max_row();
        let extend_up = target.min_row() < src.min_row();
        let extend_right = target.max_col() > src.max_col();
        let extend_left = target.min_col() < src.min_col();

        self.push_undo();
        // Always restore cell_range to the target on commit so even a
        // no-change fill (e.g. cyclic copy that happened to match) ends
        // up with the new selection.
        self.cell_range = Some(target);

        let mut changed = false;

        if extend_down || extend_up {
            // Vertical extension — each source column is its own
            // pattern. Walk columns; build the source column slice; run
            // the pattern; project N new cells.
            for col in src.min_col()..=src.max_col() {
                let source_cells: Vec<String> = (src.min_row()..=src.max_row())
                    .map(|r| self.cell(r, col).to_string())
                    .collect();
                let pattern = super::fill_series::detect(&source_cells);
                let count = if extend_down {
                    target.max_row() - src.max_row()
                } else {
                    src.min_row() - target.min_row()
                };
                let projected = super::fill_series::project_with_source_len(
                    &pattern,
                    source_cells.len(),
                    count,
                    extend_up,
                );
                if extend_down {
                    let start_row = src.max_row() + 1;
                    for (i, v) in projected.into_iter().enumerate() {
                        // None: the pattern has no value for this cell (a
                        // date past chrono's calendar, a non-finite
                        // number), so the cell keeps what it had.
                        let Some(v) = v else { continue };
                        if self.write_cell_raw(start_row + i, col, &v) {
                            changed = true;
                        }
                    }
                } else {
                    let start_row = target.min_row();
                    for (i, v) in projected.into_iter().enumerate() {
                        let Some(v) = v else { continue };
                        if self.write_cell_raw(start_row + i, col, &v) {
                            changed = true;
                        }
                    }
                }
            }
        } else if extend_right || extend_left {
            // Horizontal extension — each source row is its own pattern.
            for row in src.min_row()..=src.max_row() {
                let source_cells: Vec<String> = (src.min_col()..=src.max_col())
                    .map(|c| self.cell(row, c).to_string())
                    .collect();
                let pattern = super::fill_series::detect(&source_cells);
                let count = if extend_right {
                    target.max_col() - src.max_col()
                } else {
                    src.min_col() - target.min_col()
                };
                let projected = super::fill_series::project_with_source_len(
                    &pattern,
                    source_cells.len(),
                    count,
                    extend_left,
                );
                if extend_right {
                    let start_col = src.max_col() + 1;
                    for (i, v) in projected.into_iter().enumerate() {
                        let Some(v) = v else { continue };
                        if self.write_cell_raw(row, start_col + i, &v) {
                            changed = true;
                        }
                    }
                } else {
                    let start_col = target.min_col();
                    for (i, v) in projected.into_iter().enumerate() {
                        let Some(v) = v else { continue };
                        if self.write_cell_raw(row, start_col + i, &v) {
                            changed = true;
                        }
                    }
                }
            }
        }

        if !changed {
            // The fill landed on cells that already held the projected
            // values — keep the new selection (matches Excel) but drop
            // the spurious undo entry.
            self.pop_undo_no_restore();
        } else {
            self.source_version = self.source_version.wrapping_add(1).max(1);
        }

        changed
    }

    pub(crate) fn toggle_has_header(&mut self) {
        self.push_undo();
        let col_count = self.col_count();
        if self.has_header {
            // Convert headers row back into a data row.
            self.has_header = false;
            let mut row = std::mem::take(&mut self.headers);
            normalize_row_len(&mut row, col_count);
            self.rows.insert(0, row);
            self.headers = (0..col_count)
                .map(|i| format!("Column {}", i + 1))
                .collect();
            self.row_order = (0..self.rows.len()).collect();
        } else {
            // Promote the first data row to headers.
            if self.rows.is_empty() {
                self.has_header = true;
                self.headers = (0..col_count).map(|_| String::new()).collect();
                return;
            }
            let promoted = self.rows.remove(0);
            self.headers = promoted;
            normalize_row_len(&mut self.headers, col_count);
            self.has_header = true;
            self.row_order = (0..self.rows.len()).collect();
        }
        if let Some((r, _)) = self.active_cell {
            if r >= self.rows.len() {
                self.active_cell = None;
            }
        }
        if let Some((r, _)) = self.editing_cell {
            if r >= self.rows.len() {
                self.editing_cell = None;
                self.cell_input = None;
            }
        }
    }

    pub(crate) fn set_delimiter(&mut self, delim: u8) {
        if self.delimiter == delim {
            return;
        }
        self.push_undo();
        self.delimiter = delim;
        // Delimiter selection reinterprets the same source bytes.
        self.reparse_requested = true;
        self.source_version = self.source_version.wrapping_add(1).max(1);
    }

    /// Serialize the model back to a CSV string using the current
    /// delimiter and the detected line terminator. Preserves BOM and the
    // ---- AutoFilter ------------------------------------------------------
    //
    // Filtering is purely a render-time concern. `rows` keeps the full
    // dataset; the grid skips rendering rows where `row_is_visible` is
    // false and `emit_source` always writes everything (Excel parity —
    // saving with a filter on does NOT drop hidden rows).

    /// True when the row at `row_logical` should be rendered. A row is
    /// hidden if any column with an active filter rejects its value. The
    /// header row is always visible. Out-of-range indices return true so
    /// callers don't have to special-case them.
    pub(crate) fn row_is_visible(&self, row_logical: usize) -> bool {
        if row_logical >= self.row_count() {
            return true;
        }
        if self.col_filters.is_empty() {
            return true;
        }
        for (&col, hidden) in &self.col_filters {
            if hidden.is_empty() {
                continue;
            }
            let v = self.cell(row_logical, col);
            if hidden.contains(v) {
                return false;
            }
        }
        true
    }

    /// Number of logical rows currently hidden by the active filters.
    /// O(rows × filtered_cols) — only called when the status bar needs
    /// to render the hidden count.
    pub(crate) fn hidden_row_count(&self) -> usize {
        if self.col_filters.is_empty() {
            return 0;
        }
        (0..self.row_count())
            .filter(|&r| !self.row_is_visible(r))
            .count()
    }

    /// True when at least one column has a non-empty filter set.
    pub(crate) fn any_filter_active(&self) -> bool {
        self.col_filters.values().any(|s| !s.is_empty())
    }

    /// Compute the unique cell values for `col`, considering rows that
    /// pass every OTHER column's filter (so the user sees the choices
    /// actually relevant to their current filter intersection). Sorted
    /// alphanumerically; numeric values come first in numeric order
    /// followed by non-numeric values in case-insensitive string order.
    /// Empty cells contribute the empty string — UI renders it as
    /// `(Blanks)`.
    pub(crate) fn unique_values_for_col(&self, col_logical: usize) -> Vec<String> {
        if col_logical >= self.col_count() {
            return Vec::new();
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        for r in 0..self.row_count() {
            // Apply every OTHER column's filter — the dropdown shows
            // what's *currently* relevant. Skipping the active column's
            // own filter is the same trick Excel uses.
            let mut visible = true;
            for (&other_col, hidden) in &self.col_filters {
                if other_col == col_logical || hidden.is_empty() {
                    continue;
                }
                if hidden.contains(self.cell(r, other_col)) {
                    visible = false;
                    break;
                }
            }
            if !visible {
                continue;
            }
            let v = self.cell(r, col_logical).to_string();
            if seen.insert(v.clone()) {
                out.push(v);
            }
        }
        // Sort: numerics first in numeric order, then strings case-
        // insensitively. Empty string sinks to the top of the strings
        // (rendered as (Blanks) by the UI).
        out.sort_by(|a, b| {
            let na = a.trim().parse::<f64>().ok().filter(|n| n.is_finite());
            let nb = b.trim().parse::<f64>().ok().filter(|n| n.is_finite());
            match (na, nb) {
                (Some(x), Some(y)) => {
                    x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.to_lowercase().cmp(&b.to_lowercase()),
            }
        });
        out
    }

    /// Open the filter dropdown for `col` at `anchor`. Stages the existing
    /// hidden set so re-opening shows the active filter; the user can
    /// edit and commit, or cancel to discard.
    pub(crate) fn open_filter_dropdown_for_col(
        &mut self,
        col: usize,
        anchor: Point<Pixels>,
    ) {
        if col >= self.col_count() {
            return;
        }
        let unique_values = self.unique_values_for_col(col);
        let staged_hidden = self
            .col_filters
            .get(&col)
            .cloned()
            .unwrap_or_default();
        self.open_filter_dropdown = Some(FilterDropdownState {
            col,
            anchor,
            staged_hidden,
            unique_values,
        });
    }

    /// Move the staged hidden set onto `col_filters`. Removes the entry
    /// when the set ends up empty so `any_filter_active` is accurate.
    /// Returns the column that was committed against, if any — caller
    /// uses this to refresh dependent state (e.g. recompute find).
    pub(crate) fn commit_filter_dropdown(&mut self) -> Option<usize> {
        let Some(state) = self.open_filter_dropdown.take() else {
            return None;
        };
        if state.staged_hidden.is_empty() {
            self.col_filters.remove(&state.col);
        } else {
            self.col_filters.insert(state.col, state.staged_hidden);
        }
        Some(state.col)
    }

    /// Drop the dropdown without applying changes.
    pub(crate) fn cancel_filter_dropdown(&mut self) {
        self.open_filter_dropdown = None;
    }

    /// Remove the filter on `col`. No-op when the column has no filter.
    pub(crate) fn clear_filter_for_col(&mut self, col: usize) {
        self.col_filters.remove(&col);
    }

    pub(crate) fn clear_all_filters(&mut self) {
        self.col_filters.clear();
    }

    pub(crate) fn toggle_filters_visible(&mut self) {
        self.filters_visible = !self.filters_visible;
        if !self.filters_visible {
            // Closing the filter row also dismisses any open dropdown —
            // its trigger just disappeared.
            self.open_filter_dropdown = None;
        }
    }

    /// Re-key `col_filters` after a column reorder. `permutation[i]` is
    /// the OLD logical index of the column now at position `i`. Used by
    /// the col-reorder commit path so filter entries follow their data.
    pub(crate) fn remap_filters_after_col_reorder(&mut self, permutation: &[usize]) {
        if self.col_filters.is_empty() {
            return;
        }
        let mut new_map: HashMap<usize, HashSet<String>> = HashMap::new();
        for (new_idx, &old_idx) in permutation.iter().enumerate() {
            if let Some(set) = self.col_filters.remove(&old_idx) {
                new_map.insert(new_idx, set);
            }
        }
        self.col_filters = new_map;
    }

    /// Shift filter keys down after deleting `deleted_cols`. Entries for
    /// deleted columns are dropped; later columns slide down by the
    /// count of removed predecessors. Sorted ascending input expected
    /// (caller of delete_cols is BTreeSet which iterates sorted).
    pub(crate) fn remap_filters_after_col_delete(&mut self, deleted_cols: &BTreeSet<usize>) {
        if self.col_filters.is_empty() || deleted_cols.is_empty() {
            return;
        }
        let mut new_map: HashMap<usize, HashSet<String>> = HashMap::new();
        for (&col, set) in &self.col_filters {
            if deleted_cols.contains(&col) {
                continue;
            }
            let shift = deleted_cols.iter().filter(|&&d| d < col).count();
            new_map.insert(col - shift, set.clone());
        }
        self.col_filters = new_map;
    }

    /// Shift filter keys up after inserting a column at `insert_at`.
    /// Entries at >= `insert_at` slide up by one.
    pub(crate) fn remap_filters_after_col_insert(&mut self, insert_at: usize) {
        if self.col_filters.is_empty() {
            return;
        }
        let mut new_map: HashMap<usize, HashSet<String>> = HashMap::new();
        for (&col, set) in &self.col_filters {
            let new_col = if col >= insert_at { col + 1 } else { col };
            new_map.insert(new_col, set.clone());
        }
        self.col_filters = new_map;
    }

    /// trailing-newline convention from the original source. Called on
    /// every cell commit / reorder / structural edit; the result is fed
    /// into a single `replace_byte_range(0..len, …)` against the active
    /// buffer. `None` means the writer failed or the output would not be
    /// valid UTF-8; the caller then leaves the document as it is.
    pub(crate) fn emit_source(&self) -> Option<String> {
        // Empty schema with empty rows = empty file. Skip the csv writer
        // entirely (which would emit a single quoted empty record).
        if self.headers.is_empty() && self.rows.is_empty() {
            return Some(if self.has_bom {
                String::from("\u{FEFF}")
            } else {
                String::new()
            });
        }

        let mut buf: Vec<u8> = Vec::new();
        let terminator = match self.line_terminator {
            LineTerminator::Lf => csv::Terminator::Any(b'\n'),
            LineTerminator::Crlf => csv::Terminator::CRLF,
        };
        {
            // Flexible: a row whose length differs from the first record
            // is otherwise refused after its fields are written but
            // before its terminator, which runs it into the next row.
            let mut writer = csv::WriterBuilder::new()
                .delimiter(self.delimiter)
                .terminator(terminator)
                .flexible(true)
                .from_writer(&mut buf);
            if self.has_header && !self.headers.is_empty() {
                writer.write_record(self.headers.iter().map(|s| s.as_str())).ok()?;
            }
            for row in &self.rows {
                if !row.is_empty() {
                    writer.write_record(row.iter().map(|s| s.as_str())).ok()?;
                }
            }
            writer.flush().ok()?;
        }

        // csv always writes a terminator after the last record. Strip it
        // when the source didn't have one.
        if !self.trailing_newline {
            let term = self.line_terminator.bytes();
            if buf.ends_with(term) {
                buf.truncate(buf.len() - term.len());
            }
        }

        // Cells are `String`s and every delimiter the UI sets is ASCII,
        // so this only fails if that stops holding; an empty string here
        // would replace the whole document.
        let mut out = String::from_utf8(buf).ok()?;
        // Re-prepend BOM when the original carried one.
        if self.has_bom {
            out.insert(0, '\u{FEFF}');
        }
        Some(out)
    }

    /// Recompute `find_matches` from the current cell contents. Header
    /// matches are recorded with `row == usize::MAX` so the coordinate
    /// space matches the editor sentinel. Iterates in visual order
    /// (header row first, then body rows top-to-bottom × cols
    /// left-to-right) so Next / Prev step in the on-screen direction.
    pub(crate) fn update_find_query(&mut self, q: String) {
        self.find_query = q;
        let q = self.find_query.clone();
        self.find_matches.clear();
        if q.is_empty() {
            self.find_current_match = None;
            return;
        }
        let needle = if self.find_case_sensitive { q.clone() } else { q.to_lowercase() };

        let cell_match = |cell: &str| -> bool {
            let hay_owned;
            let hay = if self.find_case_sensitive {
                cell
            } else {
                hay_owned = cell.to_lowercase();
                hay_owned.as_str()
            };
            if self.find_whole_cell {
                hay == needle
            } else {
                hay.contains(needle.as_str())
            }
        };

        // Header row first so its visual position drives the Next/Prev
        // ordering — Excel includes headers in Find scope when the sheet
        // has them.
        if self.has_header {
            for c in 0..self.col_count() {
                if cell_match(self.header(c)) {
                    self.find_matches.push((usize::MAX, c));
                }
            }
        }
        for r in 0..self.row_count() {
            for c in 0..self.col_count() {
                if cell_match(self.cell(r, c)) {
                    self.find_matches.push((r, c));
                }
            }
        }

        self.find_current_match = if self.find_matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Step to the next match. Wraps to 0 when past the end. Returns the
    /// (row, col) of the new current match, or `None` when there are no
    /// matches.
    pub(crate) fn next_match(&mut self) -> Option<(usize, usize)> {
        if self.find_matches.is_empty() {
            self.find_current_match = None;
            return None;
        }
        // Past the end (or a stale index) wraps to the first match.
        let len = self.find_matches.len();
        let idx = self
            .find_current_match
            .and_then(|i| i.checked_add(1))
            .filter(|&i| i < len)
            .unwrap_or(0);
        self.find_current_match = Some(idx);
        self.find_matches.get(idx).copied()
    }

    /// Step to the previous match. Wraps to the end when past zero.
    pub(crate) fn prev_match(&mut self) -> Option<(usize, usize)> {
        if self.find_matches.is_empty() {
            self.find_current_match = None;
            return None;
        }
        // `len >= 1` here. Before the first match, or from a stale index
        // past the end, step to the last match.
        let len = self.find_matches.len();
        let idx = match self.find_current_match {
            Some(i) if (1..=len).contains(&i) => i - 1,
            _ => len - 1,
        };
        self.find_current_match = Some(idx);
        self.find_matches.get(idx).copied()
    }

    /// Clear find state — bar closed, no query, no matches. Used when
    /// the user presses Escape in the find bar.
    pub(crate) fn clear_find(&mut self) {
        self.find_query.clear();
        self.find_matches.clear();
        self.find_current_match = None;
        self.find_bar_open = false;
        self.find_input = None;
        self.replace_input = None;
        self.replace_summary = None;
    }

    /// Replace the current find match with `replace_text` and advance
    /// to the next match. Substring mode replaces only the matched
    /// substring (case-insensitive find preserves the original cell
    /// outside the match span by using the case-sensitive index of the
    /// matched substring in the cell's lower-cased copy). Whole-cell
    /// mode replaces the entire cell value.
    ///
    /// Returns true when a replacement happened. False on empty query,
    /// no current match, or attempting to replace an evaporated match
    /// (cell was edited externally between hits).
    pub(crate) fn replace_current(&mut self) -> bool {
        if self.find_query.is_empty() {
            return false;
        }
        let Some(i) = self.find_current_match else {
            return false;
        };
        let Some((row, col)) = self.find_matches.get(i).copied() else {
            return false;
        };

        let original = if row == usize::MAX {
            self.header(col).to_string()
        } else {
            self.cell(row, col).to_string()
        };
        let Some(new_value) = self.compute_replacement(&original) else {
            return false;
        };
        // Header rename and body cell edit are separate methods; both
        // push undo + bail on no-op. clear_summary on the next replace.
        self.replace_summary = None;
        if row == usize::MAX {
            if !self.set_header(col, new_value) {
                return false;
            }
        } else {
            if !self.set_cell(row, col, new_value) {
                return false;
            }
        }
        // After mutation the match list is stale — refresh and try to
        // hold the cursor near the same position so Next advances to
        // the FOLLOWING match instead of the same one again. Whole-cell
        // mode drops the current cell from matches (its value changed);
        // substring may keep it if there's another occurrence inside.
        let prev_idx = i;
        self.refresh_find_matches();
        if self.find_matches.is_empty() {
            self.find_current_match = None;
        } else {
            // Hold idx; refresh_find_matches already clamps. Then step
            // forward so the next Next press moves further.
            self.find_current_match = Some(prev_idx.min(self.find_matches.len() - 1));
            self.next_match();
        }
        true
    }

    /// Replace every match with `replace_text` in one undo grain.
    /// Returns the number of replacements performed.
    pub(crate) fn replace_all(&mut self) -> usize {
        if self.find_query.is_empty() || self.find_matches.is_empty() {
            return 0;
        }
        // Capture the match list before mutating; refresh after.
        let targets: Vec<(usize, usize)> = self.find_matches.clone();
        // Single undo grain — push once, then do raw writes that skip
        // their own push_undo. Done via direct field writes for headers
        // and rows, mirroring what set_cell / set_header do internally.
        self.push_undo();
        let mut count = 0usize;
        for (row, col) in targets {
            let original = if row == usize::MAX {
                self.header(col).to_string()
            } else {
                self.cell(row, col).to_string()
            };
            let Some(new_value) = self.compute_replacement(&original) else {
                continue;
            };
            if new_value == original {
                continue;
            }
            // Write through the same col_order/row_order remap as the
            // pushing variants without re-pushing undo.
            if row == usize::MAX {
                let Some(&col_orig) = self.col_order.get(col) else { continue };
                if self.headers.len() <= col_orig {
                    self.headers.resize(col_orig + 1, String::new());
                }
                self.headers[col_orig] = new_value;
            } else {
                let Some(&row_orig) = self.row_order.get(row) else { continue };
                let Some(&col_orig) = self.col_order.get(col) else { continue };
                let row_vec = match self.rows.get_mut(row_orig) {
                    Some(r) => r,
                    None => continue,
                };
                if row_vec.len() <= col_orig {
                    row_vec.resize(col_orig + 1, String::new());
                }
                row_vec[col_orig] = new_value;
            }
            count += 1;
        }
        if count == 0 {
            // Spurious undo entry — pop without restoring.
            self.pop_undo_no_restore();
        } else {
            self.refresh_find_matches();
            self.replace_summary = Some(format!(
                "Replaced {count} match{}",
                if count == 1 { "" } else { "es" }
            ));
        }
        count
    }

    /// Compute the new value for a cell when applying the current
    /// replace_text against the current query + options. Returns
    /// `None` when no replacement applies (no match in substring mode,
    /// or cell didn't equal query in whole-cell mode).
    fn compute_replacement(&self, original: &str) -> Option<String> {
        if self.find_query.is_empty() {
            return None;
        }
        if self.find_whole_cell {
            // Same comparison `update_find_query` used to list the match,
            // so every whole-cell match it found can be replaced.
            let matches = if self.find_case_sensitive {
                original == self.find_query
            } else {
                original.to_lowercase() == self.find_query.to_lowercase()
            };
            if matches {
                Some(self.replace_text.clone())
            } else {
                None
            }
        } else {
            if self.find_case_sensitive {
                if !original.contains(self.find_query.as_str()) {
                    return None;
                }
                Some(original.replace(self.find_query.as_str(), &self.replace_text))
            } else {
                // Case-insensitive substring: find every occurrence
                // by lowering both haystack and needle, splice using
                // the original cell's byte ranges so the surrounding
                // case is preserved.
                let needle_lower = self.find_query.to_lowercase();
                let hay_lower = original.to_lowercase();
                if !hay_lower.contains(needle_lower.as_str()) {
                    return None;
                }
                if !original.is_ascii() {
                    return replace_lowered_matches(
                        original,
                        &hay_lower,
                        &needle_lower,
                        &self.replace_text,
                    );
                }
                // Lowercase-mapping is byte-equal for ASCII, so offsets
                // in the lowered copy are offsets in the original: every
                // index below stays under `original.len()`.
                let mut out = String::with_capacity(original.len());
                let mut cursor = 0usize;
                let bytes = original.as_bytes();
                let n = bytes.len();
                let needle_len = needle_lower.len();
                let mut i = 0usize;
                while i + needle_len <= n {
                    let slice = &hay_lower.as_bytes()[i..i + needle_len];
                    if slice == needle_lower.as_bytes() {
                        out.push_str(&original[cursor..i]);
                        out.push_str(&self.replace_text);
                        i += needle_len;
                        cursor = i;
                    } else {
                        i += 1;
                    }
                }
                out.push_str(&original[cursor..]);
                Some(out)
            }
        }
    }

    /// Re-evaluate `find_matches` against the current cell contents
    /// without changing the query, options, or open state. Called after
    /// any mutation that could invalidate prior match coordinates
    /// (set_cell, delete_*, insert_*, sort_by_col, drag-reorder commit).
    pub(crate) fn refresh_find_matches(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        let q = self.find_query.clone();
        // update_find_query overwrites find_current_match to Some(0) on
        // a fresh query; preserve the old current so the cursor doesn't
        // jump to the first match every time the user types in the grid.
        let prev = self.find_current_match;
        self.update_find_query(q);
        if let Some(i) = prev {
            self.find_current_match = if self.find_matches.is_empty() {
                None
            } else {
                Some(i.min(self.find_matches.len().saturating_sub(1)))
            };
        }
    }
}

/// Case-insensitive replace for a non-ASCII cell. Matches are found in
/// `hay_lower` (`original.to_lowercase()`, the string `update_find_query`
/// searched) and spliced back into `original`. `str::to_lowercase` lowers
/// one char at a time, so each original char owns one contiguous run of
/// `hay_lower`; a match that starts or ends inside a run (`İ` lowers to
/// two chars) is left alone. One forward pass over both strings, so a
/// long cell stays linear.
fn replace_lowered_matches(
    original: &str,
    hay_lower: &str,
    needle_lower: &str,
    replacement: &str,
) -> Option<String> {
    if needle_lower.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(original.len());
    let mut walker = LoweredOffsets::new(original);
    let mut copied = 0usize;
    let mut from = 0usize;
    while let Some(rel) = hay_lower.get(from..).and_then(|h| h.find(needle_lower)) {
        let start = from + rel;
        let end = start + needle_lower.len();
        let orig_start = walker.seek(start);
        let mut probe = walker.clone();
        if let (Some(a), Some(b)) = (orig_start, probe.seek(end)) {
            out.push_str(original.get(copied..a)?);
            out.push_str(replacement);
            copied = b;
            walker = probe;
            from = end;
        } else {
            let step = hay_lower
                .get(start..)
                .and_then(|s| s.chars().next())
                .map_or(1, char::len_utf8);
            from = start + step;
        }
    }
    out.push_str(original.get(copied..)?);
    Some(out)
}

/// Walks `original` one char at a time, tracking where each char's
/// lower-case form starts in `original.to_lowercase()`. The final-sigma
/// rule changes which sigma `Σ` lowers to, never its byte length, so
/// per-char lengths add up to the whole-string offsets.
#[derive(Clone)]
struct LoweredOffsets<'a> {
    chars: std::str::Chars<'a>,
    lower: usize,
    orig: usize,
}

impl<'a> LoweredOffsets<'a> {
    fn new(original: &'a str) -> Self {
        Self { chars: original.chars(), lower: 0, orig: 0 }
    }

    /// Advance to lowered offset `target` (never backwards). Returns the
    /// byte offset in `original` when `target` is where a char's
    /// lower-case form starts, or the end; `None` when it falls inside
    /// one char's form or past the end.
    fn seek(&mut self, target: usize) -> Option<usize> {
        while self.lower < target {
            let c = self.chars.next()?;
            self.lower += c.to_lowercase().map(char::len_utf8).sum::<usize>();
            self.orig += c.len_utf8();
        }
        (self.lower == target).then_some(self.orig)
    }
}

/// Strip a leading UTF-8 BOM if present. Returns the stripped slice and
/// whether a BOM was detected. UTF-8 BOM = 0xEF 0xBB 0xBF which encodes
/// the single char `\u{FEFF}`.
fn strip_bom(s: &str) -> (bool, &str) {
    if let Some(stripped) = s.strip_prefix('\u{FEFF}') {
        (true, stripped)
    } else {
        (false, s)
    }
}

/// Detect the dominant line terminator. Walks bytes left-to-right; the
/// first `\r\n` pair commits to CRLF, a bare `\n` before any `\r` commits
/// to LF. No line break at all → fall back to LF.
fn detect_line_terminator(s: &str) -> LineTerminator {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if bytes.get(i + 1).copied() == Some(b'\n') {
                    return LineTerminator::Crlf;
                }
                // Bare \r — rare but treat like LF for simplicity.
                return LineTerminator::Lf;
            }
            b'\n' => return LineTerminator::Lf,
            _ => i += 1,
        }
    }
    LineTerminator::Lf
}

/// Pad / truncate `row` so it has exactly `n` columns. Used to keep
/// every row in `CsvModel::rows` rectangular — the grid renderer assumes
/// uniform column counts when laying out cells.
fn normalize_row_len(row: &mut Vec<String>, n: usize) {
    if row.len() < n {
        row.resize(n, String::new());
    } else if row.len() > n {
        row.truncate(n);
    }
}

/// Convert a zero-based column index into the spreadsheet letter label
/// (`0 -> "A"`, `25 -> "Z"`, `26 -> "AA"`, `701 -> "ZZ"`, `702 -> "AAA"`).
/// Used by the status bar to display the active cell coordinate in
/// Excel-style A1 notation.
pub(crate) fn col_letter(idx: usize) -> String {
    let mut s = String::new();
    // u128 so `usize::MAX + 1` still fits.
    let mut n = idx as u128 + 1;
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        s.insert(0, (b'A' + rem) as char);
        n = (n - 1) / 26;
    }
    s
}

/// Sniff the most-likely delimiter from the first non-empty line of
/// `source`. Walks comma / semicolon / tab character counts on that
/// single line and picks the winner; falls back to `fallback` on tie
/// or empty input. Used at parse time to seed the model's delimiter
/// from existing content without making the user pick it manually.
pub(crate) fn detect_delimiter(source: &str, fallback: u8) -> u8 {
    let first = source
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return fallback;
    }
    let comma = first.bytes().filter(|&b| b == b',').count();
    let semi = first.bytes().filter(|&b| b == b';').count();
    let tab = first.bytes().filter(|&b| b == b'\t').count();
    let max = comma.max(semi).max(tab);
    if max == 0 {
        return fallback;
    }
    // Tie-break order: comma → tab → semicolon. Comma is the dominant
    // CSV separator; tab beats semicolon for files that didn't use the
    // `.tsv` extension (e.g. clipboard paste of TSV content).
    if comma == max {
        b','
    } else if tab == max {
        b'\t'
    } else {
        b';'
    }
}

/// True when `s` looks like a number — used by the grid render path to
/// right-align numeric cells (Excel parity). Trims surrounding whitespace
/// then delegates to `f64::parse`, which accepts integers, decimals,
/// scientific notation, and leading `+` / `-`. Empty / whitespace-only
/// strings are not numeric. Mixed values like `"123abc"` fail to parse
/// in full and stay left-aligned.
pub(crate) fn is_numeric(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    match t.parse::<f64>() {
        Ok(n) => n.is_finite(),
        Err(_) => false,
    }
}

/// Parse a cell as `f64` using the same predicate as [`is_numeric`].
/// Returns `None` for non-numeric / non-finite / empty cells.
pub(crate) fn parse_numeric(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    match t.parse::<f64>() {
        Ok(n) if n.is_finite() => Some(n),
        _ => None,
    }
}

/// Format a number with up to 2 decimal places, trimming trailing zeros
/// (`5.0` → `"5"`, `5.50` → `"5.5"`, `5.55` → `"5.55"`). Excel-style;
/// status bar shows aggregates this way. Negative zero collapses to
/// "0".
pub(crate) fn format_aggregate_number(n: f64) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    let raw = format!("{:.2}", n);
    // Trim trailing zeros AFTER the decimal point only; keep the
    // integral part intact.
    if raw.contains('.') {
        let trimmed = raw.trim_end_matches('0');
        let trimmed = trimmed.trim_end_matches('.');
        trimmed.to_string()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_comma() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3", b','), b',');
    }

    #[test]
    fn detects_tab() {
        assert_eq!(detect_delimiter("a\tb\tc", b','), b'\t');
    }

    #[test]
    fn detects_semicolon() {
        assert_eq!(detect_delimiter("a;b;c;d", b','), b';');
    }

    #[test]
    fn falls_back_on_empty() {
        assert_eq!(detect_delimiter("", b','), b',');
        assert_eq!(detect_delimiter("\n\n", b','), b',');
        assert_eq!(detect_delimiter("plain_word", b','), b',');
    }

    #[test]
    fn round_trip_preserves_data() {
        let src = "name,age,city\nAlice,30,NYC\nBob,25,LA\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert_eq!(m.headers, vec!["name", "age", "city"]);
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.cell(0, 0), "Alice");
        assert_eq!(m.cell(1, 2), "LA");
        let out = m.emit_source().unwrap();
        // The csv crate normalizes quoting but the field contents survive.
        let mut m2 = CsvModel::empty(b',');
        m2.rebuild_from_source(&out, 2);
        assert_eq!(m2.headers, m.headers);
        assert_eq!(m2.rows, m.rows);
    }

    #[test]
    fn column_reorder_permutes_data() {
        let src = "a,b,c\n1,2,3\n4,5,6\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        // Move column 0 to position 2.
        m.col_order = vec![1, 2, 0];
        m.apply_col_order();
        assert_eq!(m.headers, vec!["b", "c", "a"]);
        assert_eq!(m.rows[0], vec!["2", "3", "1"]);
    }

    #[test]
    fn row_reorder_permutes_data() {
        let src = "a,b\n1,2\n3,4\n5,6\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.row_order = vec![2, 0, 1];
        m.apply_row_order();
        assert_eq!(m.rows[0], vec!["5", "6"]);
        assert_eq!(m.rows[1], vec!["1", "2"]);
        assert_eq!(m.rows[2], vec!["3", "4"]);
    }

    #[test]
    fn add_row_and_column() {
        let src = "a,b\n1,2\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.add_column();
        assert_eq!(m.headers.len(), 3);
        assert_eq!(m.rows[0].len(), 3);
        m.add_row();
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.rows[1], vec!["", "", ""]);
    }

    #[test]
    fn delete_rows_batch_descending() {
        let src = "a,b\n1,2\n3,4\n5,6\n7,8\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let mut sel = BTreeSet::new();
        sel.insert(0);
        sel.insert(2);
        m.delete_rows(&sel);
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.rows[0], vec!["3", "4"]);
        assert_eq!(m.rows[1], vec!["7", "8"]);
        assert!(m.selected_rows.is_empty());
    }

    #[test]
    fn delete_cols_batch_descending() {
        let src = "a,b,c,d\n1,2,3,4\n5,6,7,8\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let mut sel = BTreeSet::new();
        sel.insert(0);
        sel.insert(2);
        m.delete_cols(&sel);
        assert_eq!(m.headers, vec!["b", "d"]);
        assert_eq!(m.rows[0], vec!["2", "4"]);
        assert!(m.selected_cols.is_empty());
    }

    #[test]
    fn insert_row_above_and_below() {
        let src = "a,b\n1,2\n3,4\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.insert_row_at(0, true);
        // rows: [blank, 1/2, 3/4]
        assert_eq!(m.rows.len(), 3);
        assert_eq!(m.rows[0], vec!["", ""]);
        assert_eq!(m.rows[1], vec!["1", "2"]);
        m.insert_row_at(1, false);
        // rows: [blank, 1/2, blank, 3/4]
        assert_eq!(m.rows.len(), 4);
        assert_eq!(m.rows[2], vec!["", ""]);
        assert_eq!(m.rows[3], vec!["3", "4"]);
    }

    #[test]
    fn insert_col_left_and_right() {
        let src = "a,b\n1,2\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.insert_col_at(0, true);
        assert_eq!(m.headers.len(), 3);
        assert_eq!(m.rows[0], vec!["", "1", "2"]);
        m.insert_col_at(2, false);
        assert_eq!(m.headers.len(), 4);
        assert_eq!(m.rows[0], vec!["", "1", "2", ""]);
    }

    #[test]
    fn clear_rows_blanks_cells_only() {
        let src = "a,b\n1,2\n3,4\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let mut sel = BTreeSet::new();
        sel.insert(0);
        let changed = m.clear_rows(&sel);
        assert!(changed);
        assert_eq!(m.rows[0], vec!["", ""]);
        assert_eq!(m.rows[1], vec!["3", "4"]);
        assert_eq!(m.headers, vec!["a", "b"]);
    }

    #[test]
    fn clear_cols_blanks_cells_and_headers() {
        let src = "a,b\n1,2\n3,4\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let mut sel = BTreeSet::new();
        sel.insert(0);
        let changed = m.clear_cols(&sel);
        assert!(changed);
        assert_eq!(m.headers, vec!["", "b"]);
        assert_eq!(m.rows[0], vec!["", "2"]);
    }

    #[test]
    fn select_row_range_uses_anchor() {
        let mut m = CsvModel::empty(b',');
        // Need rows for indices to be valid in higher-level callers, but
        // model selection methods themselves don't bounds-check; this
        // verifies the range logic only.
        m.select_row_plain(2);
        m.select_row_range(5);
        let got: Vec<usize> = m.selected_rows.iter().copied().collect();
        assert_eq!(got, vec![2, 3, 4, 5]);
        // Swing back across the anchor — anchor stays at 2.
        m.select_row_range(0);
        let got: Vec<usize> = m.selected_rows.iter().copied().collect();
        assert_eq!(got, vec![0, 1, 2]);
    }

    #[test]
    fn cmd_click_toggles_and_clears_cols() {
        let mut m = CsvModel::empty(b',');
        m.select_col_plain(1);
        assert_eq!(m.selected_cols.len(), 1);
        m.select_row_toggle(0);
        // Selecting a row must clear the column selection.
        assert!(m.selected_cols.is_empty());
        assert_eq!(m.selected_rows.iter().copied().collect::<Vec<_>>(), vec![0]);
        m.select_row_toggle(0);
        // Toggling the same row off empties the selection but keeps the
        // anchor.
        assert!(m.selected_rows.is_empty());
        assert_eq!(m.last_clicked_row, Some(0));
    }

    #[test]
    fn multi_row_drag_contiguous_to_end() {
        // 5 body rows. Select rows 0,1,2. Drop at end (target=5).
        let src = "h\n1\n2\n3\n4\n5\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let dragged = vec![0, 1, 2];
        let (_, changed) = m.move_row_block(&dragged, 0, 5);
        assert!(changed);
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        // Rows 3,4 stay first (preserving their relative order), then
        // the moved block lands at the end keeping its internal order.
        assert_eq!(cells, vec!["4", "5", "1", "2", "3"]);
        let selected: Vec<usize> = m.selected_rows.iter().copied().collect();
        assert_eq!(selected, vec![2, 3, 4]);
        assert_eq!(m.last_clicked_row, Some(2));
    }

    #[test]
    fn multi_row_drag_non_contiguous() {
        // 5 body rows. Select rows 1 and 3. Drop at target=4 (end).
        let src = "h\nA\nB\nC\nD\nE\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let dragged = vec![1, 3];
        let (_, changed) = m.move_row_block(&dragged, 1, 4);
        assert!(changed);
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        // Filtered (without rows 1,3): [A, C, E]. removed_before=2.
        // adjusted drop = 4 - 2 = 2. splice [B, D] at 2: [A, C, B, D, E].
        assert_eq!(cells, vec!["A", "C", "B", "D", "E"]);
        let selected: Vec<usize> = m.selected_rows.iter().copied().collect();
        // Moved block now occupies logical 2 and 3.
        assert_eq!(selected, vec![2, 3]);
    }

    #[test]
    fn multi_col_drag_contiguous() {
        // 5 cols. Select cols 0,1,2. Drop at end (target=5).
        let src = "a,b,c,d,e\n1,2,3,4,5\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let dragged = vec![0, 1, 2];
        let (_, changed) = m.move_col_block(&dragged, 0, 5);
        assert!(changed);
        assert_eq!(m.headers, vec!["d", "e", "a", "b", "c"]);
        assert_eq!(m.rows[0], vec!["4", "5", "1", "2", "3"]);
        let selected: Vec<usize> = m.selected_cols.iter().copied().collect();
        assert_eq!(selected, vec![2, 3, 4]);
    }

    #[test]
    fn single_row_drag_via_block_helper() {
        // Drop row 1 at target 4. Filtered [A,C,D,E], removed_before=1,
        // adjusted drop=3. splice [B] at 3 → [A,C,D,B,E].
        let src = "h\nA\nB\nC\nD\nE\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let (_, changed) = m.move_row_block(&[1], 1, 4);
        assert!(changed);
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        assert_eq!(cells, vec!["A", "C", "D", "B", "E"]);
        let selected: Vec<usize> = m.selected_rows.iter().copied().collect();
        assert_eq!(selected, vec![3]);
        assert_eq!(m.last_clicked_row, Some(3));
    }

    #[test]
    fn row_block_drop_at_same_position_no_op() {
        let src = "h\nA\nB\nC\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let before: Vec<String> = m.rows.iter().map(|r| r[0].clone()).collect();
        let (_, changed) = m.move_row_block(&[1], 1, 1);
        let after: Vec<String> = m.rows.iter().map(|r| r[0].clone()).collect();
        assert_eq!(before, after);
        assert!(!changed, "drop-at-self must report no-op");
    }

    #[test]
    fn multi_row_drop_at_block_start_no_op() {
        // Contiguous selection [1,2,3] dropped at target=1 → already
        // there, no change. did_change must be false so the grid skips
        // the emit and doesn't dirty the file.
        let src = "h\nA\nB\nC\nD\nE\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let before: Vec<String> = m.rows.iter().map(|r| r[0].clone()).collect();
        let (_, changed) = m.move_row_block(&[1, 2, 3], 1, 1);
        let after: Vec<String> = m.rows.iter().map(|r| r[0].clone()).collect();
        assert_eq!(before, after);
        assert!(!changed);
    }

    #[test]
    fn multi_col_drop_at_block_start_no_op() {
        let src = "a,b,c,d\n1,2,3,4\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let (_, changed) = m.move_col_block(&[0, 1], 0, 0);
        assert_eq!(m.headers, vec!["a", "b", "c", "d"]);
        assert!(!changed);
    }

    #[test]
    fn non_contiguous_drop_repacks_reports_change() {
        // Non-contiguous selection always changes order on commit, even
        // at a "natural" target — the gap between the two indices
        // collapses.
        let src = "h\nA\nB\nC\nD\nE\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let (_, changed) = m.move_row_block(&[1, 3], 1, 1);
        assert!(changed, "non-contiguous repack must dirty the file");
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        // [1,3] -> filtered [A,C,E], drop=1 (no adjustment since 1 == target
        // and no dragged < target), splice [B,D] at 1: [A,B,D,C,E].
        assert_eq!(cells, vec!["A", "B", "D", "C", "E"]);
    }

    #[test]
    fn toggle_header_promotes_and_demotes() {
        let src = "x,y\n1,2\n3,4\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert_eq!(m.headers, vec!["x", "y"]);
        m.toggle_has_header();
        assert!(!m.has_header);
        assert_eq!(m.rows.len(), 3);
        assert_eq!(m.rows[0], vec!["x", "y"]);
        m.toggle_has_header();
        assert!(m.has_header);
        assert_eq!(m.headers, vec!["x", "y"]);
    }

    // Test invariant: parse(emit(parse(s))).model_data == parse(s).model_data.
    // Verifies that any data the csv crate accepts on the way in survives
    // the WriterBuilder round-trip. Quoting normalization may make the
    // emitted text differ from the original but the parsed contents must
    // match across the round-trip.
    fn assert_round_trip(src: &str) {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let emitted = m.emit_source().unwrap();
        let mut m2 = CsvModel::empty(b',');
        m2.rebuild_from_source(&emitted, 2);
        assert_eq!(m.headers, m2.headers, "headers diverged: {emitted:?}");
        assert_eq!(m.rows, m2.rows, "rows diverged: {emitted:?}");
    }

    #[test]
    fn round_trip_field_with_comma() {
        // The csv crate auto-quotes a field that contains the delimiter.
        assert_round_trip("name,address\nAlice,\"123 Main, Apt 4\"\nBob,xyz\n");
    }

    #[test]
    fn round_trip_field_with_double_quote() {
        // RFC 4180 escapes embedded quotes by doubling.
        assert_round_trip("q,r\n\"a\"\"b\",ok\nnoquote,1\n");
    }

    #[test]
    fn round_trip_field_with_newline() {
        // Embedded newlines are legal inside quoted fields.
        assert_round_trip("note,priority\n\"line one\nline two\",hi\nplain,lo\n");
    }

    #[test]
    fn round_trip_leading_trailing_spaces_preserved() {
        // The csv crate preserves whitespace inside fields by default.
        assert_round_trip("a,b\n  spaced  ,trimmed\n");
    }

    #[test]
    fn round_trip_empty_cells() {
        // Three consecutive commas = three empty fields. Must round-trip.
        let src = "a,b,c,d\n,,,\nfoo,,,bar\n";
        assert_round_trip(src);
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert_eq!(m.rows[0], vec!["", "", "", ""]);
        assert_eq!(m.rows[1], vec!["foo", "", "", "bar"]);
    }

    #[test]
    fn round_trip_unicode_preserves_bytes() {
        // Multi-byte UTF-8 chars (CJK + emoji) must survive without
        // truncation or replacement.
        let src = "name,emoji\nЯлекс,🚀\n張三,😀\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        let emitted = m.emit_source().unwrap();
        let mut m2 = CsvModel::empty(b',');
        m2.rebuild_from_source(&emitted, 2);
        assert_eq!(m.rows, m2.rows);
        assert_eq!(m.cell(0, 1), "🚀");
        assert_eq!(m.cell(1, 0), "張三");
    }

    #[test]
    fn detect_lf_terminator() {
        assert_eq!(detect_line_terminator("a,b\n1,2\n"), LineTerminator::Lf);
    }

    #[test]
    fn detect_crlf_terminator() {
        assert_eq!(
            detect_line_terminator("a,b\r\n1,2\r\n"),
            LineTerminator::Crlf
        );
    }

    #[test]
    fn detect_terminator_empty_falls_back_to_lf() {
        assert_eq!(detect_line_terminator(""), LineTerminator::Lf);
        assert_eq!(detect_line_terminator("noline"), LineTerminator::Lf);
    }

    #[test]
    fn crlf_round_trips_through_emit() {
        let src = "a,b\r\n1,2\r\n3,4\r\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert_eq!(m.line_terminator, LineTerminator::Crlf);
        let emit = m.emit_source().unwrap();
        assert!(
            emit.contains("\r\n"),
            "CRLF terminator must round-trip, got {:?}",
            emit
        );
        assert!(!emit.contains("\n\n"), "no LF-only lines allowed");
    }

    #[test]
    fn lf_round_trips_through_emit() {
        let src = "a,b\n1,2\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert_eq!(m.line_terminator, LineTerminator::Lf);
        let emit = m.emit_source().unwrap();
        assert!(!emit.contains('\r'), "LF source must not pick up CR on emit");
    }

    #[test]
    fn trailing_newline_preserved_when_present() {
        let src = "a,b\n1,2\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert!(m.trailing_newline);
        let emit = m.emit_source().unwrap();
        assert!(emit.ends_with('\n'), "trailing newline lost: {emit:?}");
    }

    #[test]
    fn trailing_newline_absent_when_source_lacks_it() {
        let src = "a,b\n1,2";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        assert!(!m.trailing_newline);
        let emit = m.emit_source().unwrap();
        assert!(
            !emit.ends_with('\n'),
            "trailing newline injected where source had none: {emit:?}"
        );
    }

    #[test]
    fn bom_round_trips() {
        let with_bom = "\u{FEFF}a,b\n1,2\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(with_bom, 1);
        assert!(m.has_bom);
        assert_eq!(m.headers, vec!["a", "b"]);
        let emit = m.emit_source().unwrap();
        assert!(emit.starts_with('\u{FEFF}'), "BOM dropped on emit");
    }

    #[test]
    fn no_bom_means_no_bom_on_emit() {
        let without = "a,b\n1,2\n";
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(without, 1);
        assert!(!m.has_bom);
        assert!(!m.emit_source().unwrap().starts_with('\u{FEFF}'));
    }

    #[test]
    fn empty_source_doesnt_crash() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("", 1);
        assert_eq!(m.headers.len(), 0);
        assert_eq!(m.rows.len(), 0);
        // Emit must be safe and produce an empty string (no trailing
        // newline since there was none to begin with).
        assert_eq!(m.emit_source().unwrap(), "");
    }

    #[test]
    fn header_only_file_renders_empty_body() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("Name,Email\n", 1);
        assert_eq!(m.headers, vec!["Name", "Email"]);
        assert_eq!(m.rows.len(), 0);
        // Adding a row works on a header-only file.
        m.add_row();
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.rows[0], vec!["", ""]);
    }

    #[test]
    fn delete_every_row_keeps_headers() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        let mut sel = BTreeSet::new();
        sel.insert(0);
        sel.insert(1);
        m.delete_rows(&sel);
        assert_eq!(m.headers, vec!["a", "b"], "headers wiped by row deletion");
        assert_eq!(m.rows.len(), 0);
        // Emit yields just the header line.
        let emit = m.emit_source().unwrap();
        assert!(
            emit.starts_with("a,b"),
            "header missing after delete-all-rows: {emit:?}"
        );
    }

    #[test]
    fn delete_every_column_is_safe() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n", 1);
        let mut sel = BTreeSet::new();
        sel.insert(0);
        sel.insert(1);
        sel.insert(2);
        m.delete_cols(&sel);
        assert_eq!(m.headers.len(), 0);
        assert_eq!(m.col_count(), 0);
        // emit_source must not panic on an empty schema.
        let _ = m.emit_source().unwrap();
    }

    #[test]
    fn set_header_round_trips_through_emit() {
        // Renaming a header writes through set_header; the next emit must
        // place the new label on the first record of the serialized CSV.
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("name,age\nAlice,30\n", 1);
        let changed = m.set_header(0, "Full Name".to_string());
        assert!(changed);
        let emit = m.emit_source().unwrap();
        assert!(
            emit.starts_with("Full Name,age"),
            "renamed header missing from emit: {emit:?}"
        );
        assert!(emit.contains("Alice,30"), "body lost in emit: {emit:?}");
    }

    #[test]
    fn set_header_no_op_returns_false() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("name,age\n", 1);
        // Same value already in headers — no rewrite needed.
        assert!(!m.set_header(0, "name".to_string()));
    }

    #[test]
    fn clear_axis_selections_drops_drag_state() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n", 1);
        m.col_drag = Some(ColDragState {
            dragged: vec![1],
            primary: 1,
            start_mouse_x: px(0.),
            current_mouse_x: px(0.),
            drop_logical: 1,
            crossed_threshold: false,
        });
        m.row_drag = Some(RowDragState {
            dragged: vec![0],
            primary: 0,
            start_mouse_y: px(0.),
            current_mouse_y: px(0.),
            drop_logical: 0,
            crossed_threshold: false,
        });
        m.col_resize = Some(ColResizeState {
            col_logical: 0,
            start_mouse_x: px(0.),
            start_width: px(120.),
        });
        m.selected_rows.insert(0);
        m.selected_cols.insert(1);
        m.last_clicked_row = Some(0);
        m.last_clicked_col = Some(1);

        m.clear_axis_selections();

        assert!(m.col_drag.is_none(), "col_drag survived clear");
        assert!(m.row_drag.is_none(), "row_drag survived clear");
        assert!(m.col_resize.is_none(), "col_resize survived clear");
        assert!(m.selected_rows.is_empty());
        assert!(m.selected_cols.is_empty());
        assert!(m.last_clicked_row.is_none());
        assert!(m.last_clicked_col.is_none());
    }

    #[test]
    fn editing_header_col_recognises_sentinel() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        m.editing_cell = Some((0, 1));
        assert_eq!(m.editing_header_col(), None);
        m.editing_cell = Some((usize::MAX, 1));
        assert_eq!(m.editing_header_col(), Some(1));
        m.editing_cell = None;
        assert_eq!(m.editing_header_col(), None);
    }

    #[test]
    fn strip_bom_helper_detects_presence() {
        let (had, rest) = strip_bom("\u{FEFF}hello");
        assert!(had);
        assert_eq!(rest, "hello");
        let (had2, rest2) = strip_bom("hello");
        assert!(!had2);
        assert_eq!(rest2, "hello");
    }

    // Excel-style click semantics rely on `active_cell` (single-click
    // highlight) and `editing_cell` (double-click / F2 entry) being two
    // independent fields. The integration paths live in actions.rs and
    // need a Workspace harness to test directly, but the model-level
    // invariants below describe the state contract those actions write.

    #[test]
    fn active_cell_independent_of_editing_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        m.active_cell = Some((0, 1));
        assert_eq!(m.active_cell, Some((0, 1)));
        assert_eq!(m.editing_cell, None);
        m.editing_cell = Some((0, 1));
        m.active_cell = None;
        assert_eq!(m.active_cell, None);
        assert_eq!(m.editing_cell, Some((0, 1)));
    }

    #[test]
    fn select_col_plain_clears_rows() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_row_plain(0);
        m.select_col_plain(1);
        assert!(m.selected_rows.is_empty(), "row selection should clear");
        assert!(m.selected_cols.contains(&1));
        assert_eq!(m.last_clicked_col, Some(1));
    }

    #[test]
    fn select_row_plain_clears_cols() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_col_plain(1);
        m.select_row_plain(0);
        assert!(m.selected_cols.is_empty(), "col selection should clear");
        assert!(m.selected_rows.contains(&0));
        assert_eq!(m.last_clicked_row, Some(0));
    }

    #[test]
    fn single_selected_col_signals_header_rename_target() {
        // Mirrors the actions.rs check used by open_active_cell_editor
        // and begin_cell_edit_with_seed when F2 / Enter / typed-char
        // arrives on a focused grid: exactly one column selected and no
        // rows selected routes the keystroke to header rename.
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n", 1);
        m.select_col_plain(1);
        let is_header_rename = m.selected_cols.len() == 1 && m.selected_rows.is_empty();
        assert!(is_header_rename);

        m.select_col_toggle(2);
        let is_header_rename = m.selected_cols.len() == 1 && m.selected_rows.is_empty();
        assert!(!is_header_rename, "two cols selected → not header rename");

        m.clear_axis_selections();
        m.select_col_plain(0);
        m.select_row_toggle(0);
        let is_header_rename = m.selected_cols.len() == 1 && m.selected_rows.is_empty();
        assert!(!is_header_rename, "row also selected → not header rename");
    }

    #[test]
    fn plain_click_col_replaces_previous_selection() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c,d,e\n1,2,3,4,5\n", 1);
        m.select_col_toggle(0);
        m.select_col_toggle(2);
        assert_eq!(m.selected_cols.len(), 2);
        m.select_col_plain(4);
        assert_eq!(m.selected_cols.iter().copied().collect::<Vec<_>>(), vec![4]);
        assert_eq!(m.last_clicked_col, Some(4));
    }

    #[test]
    fn plain_click_row_replaces_previous_selection() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("h\n1\n2\n3\n4\n5\n", 1);
        m.select_row_toggle(0);
        m.select_row_toggle(2);
        assert_eq!(m.selected_rows.len(), 2);
        m.select_row_plain(4);
        assert_eq!(m.selected_rows.iter().copied().collect::<Vec<_>>(), vec![4]);
        assert_eq!(m.last_clicked_row, Some(4));
    }

    #[test]
    fn cmd_click_col_toggles() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c,d,e\n1,2,3,4,5\n", 1);
        m.select_col_toggle(0);
        m.select_col_toggle(2);
        m.select_col_toggle(4);
        assert_eq!(m.selected_cols.iter().copied().collect::<Vec<_>>(), vec![0, 2, 4]);
        m.select_col_toggle(0);
        assert_eq!(m.selected_cols.iter().copied().collect::<Vec<_>>(), vec![2, 4]);
        assert_eq!(m.last_clicked_col, Some(0));
    }

    #[test]
    fn shift_click_col_range_from_last_clicked() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c,d,e\n1,2,3,4,5\n", 1);
        m.select_col_plain(1);
        m.select_col_range(4);
        assert_eq!(m.selected_cols.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn plain_click_col_clears_active_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c,d,e\n1,2,3,4,5\n", 1);
        m.active_cell = Some((0, 3));
        m.select_col_plain(1);
        assert_eq!(m.active_cell, None);
        assert!(m.selected_cols.contains(&1));
    }

    #[test]
    fn plain_click_row_clears_active_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("h\n1\n2\n3\n4\n", 1);
        m.active_cell = Some((2, 0));
        m.select_row_plain(0);
        assert_eq!(m.active_cell, None);
        assert!(m.selected_rows.contains(&0));
    }

    #[test]
    fn cmd_and_shift_click_also_drop_active_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c,d,e\n1,2,3,4,5\n", 1);
        m.active_cell = Some((0, 2));
        m.select_col_toggle(0);
        assert_eq!(m.active_cell, None);

        m.active_cell = Some((0, 2));
        m.select_col_range(3);
        assert_eq!(m.active_cell, None);

        m.active_cell = Some((0, 1));
        m.select_row_toggle(0);
        assert_eq!(m.active_cell, None);

        m.active_cell = Some((0, 1));
        m.select_row_range(0);
        assert_eq!(m.active_cell, None);
    }

    #[test]
    fn delete_key_with_only_col_selection_deletes_cols() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c,d,e\n1,2,3,4,5\n", 1);
        m.selected_cols = [1, 3].into_iter().collect();
        assert!(m.handle_delete_key());
        assert_eq!(m.headers, vec!["a", "c", "e"]);
        assert_eq!(m.rows[0], vec!["1", "3", "5"]);
        assert!(m.selected_cols.is_empty());
    }

    #[test]
    fn delete_key_with_only_row_selection_deletes_rows() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n5,6\n", 1);
        m.selected_rows = [0, 2].into_iter().collect();
        assert!(m.handle_delete_key());
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.rows[0], vec!["3", "4"]);
        assert!(m.selected_rows.is_empty());
    }

    #[test]
    fn delete_key_with_active_cell_only_clears_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nhello,world\n", 1);
        m.active_cell = Some((0, 0));
        assert!(m.handle_delete_key());
        assert_eq!(m.cell(0, 0), "");
        assert_eq!(m.cell(0, 1), "world");
        assert_eq!(m.active_cell, Some((0, 0)));
    }

    #[test]
    fn delete_key_no_selection_no_active_cell_is_noop() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        let before = m.rows.clone();
        assert!(!m.handle_delete_key());
        assert_eq!(m.rows, before);
    }

    #[test]
    fn clear_axis_selections_drops_active_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_row_toggle(0);
        m.select_col_toggle(1);
        m.active_cell = Some((1, 0));
        m.last_clicked_row = Some(0);
        m.last_clicked_col = Some(1);
        m.clear_axis_selections();
        assert!(m.selected_rows.is_empty());
        assert!(m.selected_cols.is_empty());
        assert_eq!(m.active_cell, None);
        assert_eq!(m.last_clicked_row, None);
        assert_eq!(m.last_clicked_col, None);
    }

    /// Stub measurer used in auto-fit tests — pretends every glyph is
    /// 7px wide. Decouples the math from a real `TextSystem` so the
    /// model layer stays testable.
    fn stub_measure_7px(s: &str) -> f32 {
        s.chars().count() as f32 * 7.0
    }

    #[test]
    fn auto_fit_column_clamps_to_min() {
        // Empty header → falls back to the grabbable empty-header width,
        // not the absolute MIN — keeps the resize handle reachable. The
        // leading comma in row 1 gives column 0 an empty header field.
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(",b\nx,y\n", 1);
        m.auto_fit_column(0, stub_measure_7px);
        assert_eq!(m.col_width(0), px(AUTO_FIT_EMPTY_HEADER_PX));
    }

    #[test]
    fn auto_fit_column_clamps_to_max() {
        let mut m = CsvModel::empty(b',');
        let huge: String = "x".repeat(1000);
        // Header is the 1000-char string — auto-fit must clamp to MAX.
        let src = format!("{huge}\nshort\n");
        m.rebuild_from_source(&src, 1);
        m.auto_fit_column(0, stub_measure_7px);
        assert_eq!(m.col_width(0), px(MAX_AUTO_FIT_WIDTH_PX));
    }

    #[test]
    fn auto_fit_column_ignores_cell_content() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("Hi\nVeryLongCellValueThatShouldBeIgnored\nshort\n", 1);
        m.auto_fit_column(0, stub_measure_7px);
        // Header "Hi" = 2 * 7 = 14; +28 padding = 42 → clamps to MIN 48.
        // Body cells must NOT influence the result.
        assert_eq!(m.col_width(0), px(MIN_COL_WIDTH_PX));
    }

    #[test]
    fn auto_fit_column_uses_header_when_wider_than_cells() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("LongHeaderText\na\nb\n", 1);
        m.auto_fit_column(0, stub_measure_7px);
        // "LongHeaderText" = 14 * 7 = 98; +28 = 126.
        assert_eq!(m.col_width(0), px(126.0));
    }

    #[test]
    fn auto_fit_column_empty_header_falls_back_to_min_grabbable() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(",hdr\nval,val\n", 1);
        m.auto_fit_column(0, stub_measure_7px);
        assert!(m.col_width(0) >= px(AUTO_FIT_EMPTY_HEADER_PX));
        assert_eq!(m.col_width(0), px(AUTO_FIT_EMPTY_HEADER_PX));
    }

    #[test]
    fn auto_fit_all_columns_runs_per_column() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,LongHeader\nabcdefg,b\n", 1);
        m.auto_fit_all_columns(stub_measure_7px);
        // col 0: header "a" = 1 * 7 = 7; +28 = 35 → clamps to MIN 48.
        // Cells like "abcdefg" are ignored now.
        assert_eq!(m.col_width(0), px(MIN_COL_WIDTH_PX));
        // col 1: header "LongHeader" = 10 * 7 = 70; +28 = 98.
        assert_eq!(m.col_width(1), px(98.0));
    }

    #[test]
    fn auto_fit_all_columns_widens_all() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("aaa,bbbbb\n1,2\n", 1);
        let pre_0 = m.col_width(0);
        let pre_1 = m.col_width(1);
        m.auto_fit_all_columns(stub_measure_7px);
        // Both columns must have moved off the default — header-driven
        // auto-fit hits min-clamp for short headers, narrower than the
        // 140-px default; longer headers go wider. Either way ≠ default.
        assert_ne!(m.col_width(0), pre_0);
        assert_ne!(m.col_width(1), pre_1);
    }

    #[test]
    fn auto_fit_all_columns_one_undo_grain() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n", 1);
        let pre_widths: Vec<Pixels> = (0..m.col_count()).map(|c| m.col_width(c)).collect();
        m.auto_fit_all_columns(stub_measure_7px);
        let post_widths: Vec<Pixels> = (0..m.col_count()).map(|c| m.col_width(c)).collect();
        assert_ne!(pre_widths, post_widths);
        // A single undo must restore every column's pre-auto-fit width.
        assert!(m.undo());
        let restored: Vec<Pixels> = (0..m.col_count()).map(|c| m.col_width(c)).collect();
        assert_eq!(restored, pre_widths);
    }

    #[test]
    fn col_letter_first_block() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(1), "B");
        assert_eq!(col_letter(25), "Z");
    }

    #[test]
    fn col_letter_double_block() {
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(27), "AB");
        assert_eq!(col_letter(51), "AZ");
        assert_eq!(col_letter(52), "BA");
        assert_eq!(col_letter(701), "ZZ");
    }

    #[test]
    fn col_letter_triple_block() {
        assert_eq!(col_letter(702), "AAA");
    }

    #[test]
    fn sort_by_col_ascending_strings() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("name\ncharlie\nalpha\nbravo\n", 1);
        m.sort_by_col(0, true);
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        assert_eq!(cells, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn sort_by_col_descending_strings() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("name\ncharlie\nalpha\nbravo\n", 1);
        m.sort_by_col(0, false);
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        assert_eq!(cells, vec!["charlie", "bravo", "alpha"]);
    }

    #[test]
    fn sort_by_col_numeric_detects_mixed_columns() {
        // A column with "1", "10", "2" must sort as 1, 2, 10 (numeric),
        // not 1, 10, 2 (string compare).
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("n\n10\n1\n2\n", 1);
        m.sort_by_col(0, true);
        let cells: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 0)).collect();
        assert_eq!(cells, vec!["1", "2", "10"]);
    }

    #[test]
    fn sort_by_col_is_stable() {
        // Equal keys must preserve their original relative order.
        // Group by "a" — both rows have key "a"; the second-column tag
        // must keep its pre-sort order after the sort.
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("k,tag\nb,x\na,first\na,second\na,third\nb,y\n", 1);
        m.sort_by_col(0, true);
        // First three rows = a,first / a,second / a,third (stable),
        // followed by b,x / b,y in original order.
        let tags: Vec<&str> = (0..m.row_count()).map(|r| m.cell(r, 1)).collect();
        assert_eq!(tags, vec!["first", "second", "third", "x", "y"]);
    }

    #[test]
    fn sort_by_col_clears_row_selection() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("k\nc\nb\na\n", 1);
        m.selected_rows = [0, 1].into_iter().collect();
        m.last_clicked_row = Some(0);
        m.sort_by_col(0, true);
        assert!(m.selected_rows.is_empty());
        assert!(m.last_clicked_row.is_none());
    }

    #[test]
    fn sort_by_col_clears_active_cell() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("k\nc\nb\na\n", 1);
        m.active_cell = Some((1, 0));
        m.sort_by_col(0, true);
        assert!(m.active_cell.is_none());
    }

    #[test]
    fn sort_by_col_preserves_col_selection() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("k,v\nc,3\nb,2\na,1\n", 1);
        m.selected_cols = [0].into_iter().collect();
        m.last_clicked_col = Some(0);
        m.sort_by_col(0, true);
        assert_eq!(
            m.selected_cols.iter().copied().collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(m.last_clicked_col, Some(0));
    }

    #[test]
    fn sort_by_col_bumps_source_version() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("k\nc\nb\na\n", 1);
        let before = m.source_version;
        m.sort_by_col(0, true);
        assert_ne!(m.source_version, before);
    }

    #[test]
    fn sort_by_col_round_trips_through_emit() {
        // Sort permutes row_order — emit must walk the permuted order
        // so the saved file reflects the sort.
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("k,v\nc,3\na,1\nb,2\n", 1);
        m.sort_by_col(0, true);
        let emit = m.emit_source().unwrap();
        // Header first, then rows in sorted order.
        let mut lines = emit.lines();
        assert_eq!(lines.next(), Some("k,v"));
        assert_eq!(lines.next(), Some("a,1"));
        assert_eq!(lines.next(), Some("b,2"));
        assert_eq!(lines.next(), Some("c,3"));
    }

    // --- cell-range selection -------------------------------------------------

    #[test]
    fn cell_range_normalized_min_max() {
        // anchor and focus can be swapped — min/max are independent of
        // which corner the user dragged from.
        let a = CellRange::new((4, 3), (1, 0));
        assert_eq!(a.min_row(), 1);
        assert_eq!(a.max_row(), 4);
        assert_eq!(a.min_col(), 0);
        assert_eq!(a.max_col(), 3);
    }

    #[test]
    fn cell_range_contains() {
        let r = CellRange::new((1, 1), (3, 3));
        assert!(r.contains(2, 2));
        assert!(r.contains(1, 1)); // inclusive corners
        assert!(r.contains(3, 3));
        assert!(!r.contains(0, 2));
        assert!(!r.contains(4, 2));
    }

    #[test]
    fn cell_range_single_cell() {
        let r = CellRange::new((2, 2), (2, 2));
        assert!(r.is_single_cell());
        let r2 = CellRange::new((2, 2), (2, 3));
        assert!(!r2.is_single_cell());
    }

    #[test]
    fn setting_cell_range_clears_row_col_selection() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_row_plain(0);
        m.select_col_plain(0);
        // select_col_plain wipes selected_rows. Now drop col selection
        // by setting a range.
        m.select_cell_range((0, 0), (1, 1));
        assert!(m.selected_rows.is_empty());
        assert!(m.selected_cols.is_empty());
        assert!(m.cell_range.is_some());
    }

    #[test]
    fn setting_selected_rows_clears_cell_range() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_cell_range((0, 0), (1, 1));
        assert!(m.cell_range.is_some());
        m.select_row_plain(0);
        assert!(m.cell_range.is_none());
    }

    #[test]
    fn setting_selected_cols_clears_cell_range() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_cell_range((0, 0), (1, 1));
        m.select_col_plain(1);
        assert!(m.cell_range.is_none());
    }

    #[test]
    fn extend_cell_range_focus_keeps_anchor() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n4,5,6\n", 1);
        m.select_cell_range((0, 0), (0, 0));
        m.extend_cell_range_focus((1, 2));
        let range = m.cell_range.unwrap();
        assert_eq!(range.anchor, (0, 0));
        assert_eq!(range.focus, (1, 2));
    }

    #[test]
    fn clear_axis_selections_drops_cell_range() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.select_cell_range((0, 0), (1, 1));
        m.active_cell = Some((1, 1));
        m.clear_axis_selections();
        assert!(m.cell_range.is_none());
        assert!(m.active_cell.is_none());
    }

    // --- clear_range / paste --------------------------------------------------

    #[test]
    fn clear_range_blanks_cells_only() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n4,5,6\n7,8,9\n", 1);
        let range = CellRange::new((0, 0), (1, 1));
        assert!(m.clear_range(range));
        assert_eq!(m.cell(0, 0), "");
        assert_eq!(m.cell(0, 1), "");
        assert_eq!(m.cell(0, 2), "3"); // outside range — untouched
        assert_eq!(m.cell(1, 0), "");
        assert_eq!(m.cell(2, 0), "7"); // outside range
    }

    #[test]
    fn clear_range_pushes_undo() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n3,4\n", 1);
        m.undo_stack.clear();
        let range = CellRange::new((0, 0), (0, 0));
        assert!(m.clear_range(range));
        assert_eq!(m.undo_stack.len(), 1);
        assert!(m.undo());
        assert_eq!(m.cell(0, 0), "1");
    }

    #[test]
    fn paste_into_active_cell_writes_2x2_at_anchor() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n4,5,6\n", 1);
        let data = vec![
            vec!["X".to_string(), "Y".to_string()],
            vec!["P".to_string(), "Q".to_string()],
        ];
        assert!(m.paste_at(0, 1, &data, None));
        assert_eq!(m.cell(0, 1), "X");
        assert_eq!(m.cell(0, 2), "Y");
        assert_eq!(m.cell(1, 1), "P");
        assert_eq!(m.cell(1, 2), "Q");
        assert_eq!(m.cell(0, 0), "1");
    }

    #[test]
    fn paste_1x1_into_range_fills_range() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n4,5,6\n7,8,9\n", 1);
        let data = vec![vec!["Z".to_string()]];
        let range = CellRange::new((0, 0), (1, 1));
        assert!(m.paste_at(0, 0, &data, Some(range)));
        assert_eq!(m.cell(0, 0), "Z");
        assert_eq!(m.cell(0, 1), "Z");
        assert_eq!(m.cell(1, 0), "Z");
        assert_eq!(m.cell(1, 1), "Z");
        assert_eq!(m.cell(0, 2), "3"); // outside target
        assert_eq!(m.cell(2, 0), "7"); // outside target
    }

    #[test]
    fn paste_extends_rows_when_needed() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        assert_eq!(m.row_count(), 1);
        let data = vec![
            vec!["X".to_string()],
            vec!["Y".to_string()],
            vec!["Z".to_string()],
        ];
        assert!(m.paste_at(0, 0, &data, None));
        assert_eq!(m.row_count(), 3);
        assert_eq!(m.cell(0, 0), "X");
        assert_eq!(m.cell(2, 0), "Z");
    }

    #[test]
    fn paste_extends_cols_when_needed() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        assert_eq!(m.col_count(), 2);
        let data = vec![vec![
            "P".to_string(),
            "Q".to_string(),
            "R".to_string(),
            "S".to_string(),
        ]];
        assert!(m.paste_at(0, 0, &data, None));
        assert_eq!(m.col_count(), 4);
        assert_eq!(m.cell(0, 0), "P");
        assert_eq!(m.cell(0, 3), "S");
    }

    #[test]
    fn paste_pushes_undo() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        m.undo_stack.clear();
        let data = vec![vec!["X".to_string()]];
        assert!(m.paste_at(0, 0, &data, None));
        assert_eq!(m.undo_stack.len(), 1);
        assert!(m.undo());
        assert_eq!(m.cell(0, 0), "1");
    }

    #[test]
    fn paste_no_op_when_value_equals_existing() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        m.undo_stack.clear();
        let data = vec![vec!["1".to_string()]];
        // 1x1 source, no fill_target, value equals existing — should
        // be a true no-op and not push a useless undo entry.
        assert!(!m.paste_at(0, 0, &data, None));
        assert_eq!(m.undo_stack.len(), 0);
    }

    #[test]
    fn delete_key_on_range_clears_cells() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b,c\n1,2,3\n4,5,6\n", 1);
        m.select_cell_range((0, 0), (1, 1));
        assert!(m.handle_delete_key());
        assert_eq!(m.cell(0, 0), "");
        assert_eq!(m.cell(1, 1), "");
        assert_eq!(m.cell(0, 2), "3");
        assert_eq!(m.cell(1, 2), "6");
        // Row count unchanged — delete on a range BLANKS, doesn't
        // remove rows.
        assert_eq!(m.row_count(), 2);
    }

    #[test]
    fn delete_key_single_cell_range_clears_active_cell() {
        // A 1×1 "range" left behind from a click-without-drag should
        // not trip the range branch — the existing active-cell-clear
        // path still fires.
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\n1,2\n", 1);
        m.cell_range = Some(CellRange::new((0, 0), (0, 0)));
        m.active_cell = Some((0, 0));
        assert!(m.handle_delete_key());
        assert_eq!(m.cell(0, 0), "");
    }

    #[test]
    fn is_numeric_basic() {
        assert!(is_numeric("123"));
        assert!(is_numeric("0"));
    }

    #[test]
    fn is_numeric_negative() {
        assert!(is_numeric("-123"));
        assert!(is_numeric("-0.5"));
        assert!(is_numeric("+42"));
    }

    #[test]
    fn is_numeric_decimal() {
        assert!(is_numeric("3.14"));
        assert!(is_numeric(".5"));
        assert!(is_numeric("5."));
    }

    #[test]
    fn is_numeric_scientific() {
        assert!(is_numeric("1e5"));
        assert!(is_numeric("1.5E-3"));
        assert!(is_numeric("-2.71e10"));
    }

    #[test]
    fn is_numeric_with_leading_space() {
        assert!(is_numeric("  42  "));
        assert!(is_numeric("\t3.14\n"));
    }

    #[test]
    fn is_numeric_rejects_text() {
        assert!(!is_numeric("hello"));
        assert!(!is_numeric("NaN-ish"));
    }

    #[test]
    fn is_numeric_rejects_mixed() {
        assert!(!is_numeric("123abc"));
        assert!(!is_numeric("12 34"));
        assert!(!is_numeric("$100"));
    }

    #[test]
    fn is_numeric_empty_is_false() {
        assert!(!is_numeric(""));
        assert!(!is_numeric("   "));
        assert!(!is_numeric("\t\n"));
    }

    #[test]
    fn find_substring_case_insensitive_finds_all() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nApple,Banana\napricot,APPLE\n", 1);
        m.update_find_query("apple".to_string());
        // Body rows: (0,0)=Apple, (1,1)=APPLE. Both case-insensitive.
        // Header has no "apple" entry.
        assert_eq!(m.find_matches.len(), 2);
        assert!(m.find_matches.contains(&(0, 0)));
        assert!(m.find_matches.contains(&(1, 1)));
    }

    #[test]
    fn find_case_sensitive_skips_mismatched_case() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nApple,APPLE\napricot,banana\n", 1);
        m.find_case_sensitive = true;
        m.update_find_query("APPLE".to_string());
        assert_eq!(m.find_matches, vec![(0, 1)]);
    }

    #[test]
    fn find_whole_cell_excludes_substrings() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nfoo,foobar\nfoo,foo\n", 1);
        m.find_whole_cell = true;
        m.update_find_query("foo".to_string());
        // Only cells equal to "foo" — (0,0), (1,0), (1,1). "foobar" excluded.
        assert_eq!(m.find_matches.len(), 3);
        assert!(!m.find_matches.contains(&(0, 1)));
    }

    #[test]
    fn find_searches_headers_and_body() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("Name,age\nName,42\n", 1);
        m.update_find_query("name".to_string());
        // Header (usize::MAX, 0)="Name" and body (0,0)="Name" both match.
        assert_eq!(m.find_matches.len(), 2);
        assert_eq!(m.find_matches[0], (usize::MAX, 0));
        assert_eq!(m.find_matches[1], (0, 0));
    }

    #[test]
    fn find_next_wraps_at_end() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nx,y\nx,z\n", 1);
        m.update_find_query("x".to_string());
        assert_eq!(m.find_current_match, Some(0));
        m.next_match();
        assert_eq!(m.find_current_match, Some(1));
        m.next_match();
        // Wraps back to 0.
        assert_eq!(m.find_current_match, Some(0));
    }

    #[test]
    fn find_prev_wraps_at_start() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nx,y\nx,z\n", 1);
        m.update_find_query("x".to_string());
        assert_eq!(m.find_current_match, Some(0));
        m.prev_match();
        // Wraps to last.
        assert_eq!(m.find_current_match, Some(1));
    }

    #[test]
    fn find_empty_query_clears_matches() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nx,y\n", 1);
        m.update_find_query("x".to_string());
        assert_eq!(m.find_matches.len(), 1);
        m.update_find_query(String::new());
        assert!(m.find_matches.is_empty());
        assert_eq!(m.find_current_match, None);
    }

    #[test]
    fn find_after_edit_remains_consistent() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nx,y\nx,z\n", 1);
        m.update_find_query("x".to_string());
        // Replace the second "x" with "q".
        m.set_cell(1, 0, "q".to_string());
        m.refresh_find_matches();
        assert_eq!(m.find_matches, vec![(0, 0)]);
    }

    #[test]
    fn find_after_delete_invalidates_stale_cursor() {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source("a,b\nx,y\nx,z\n", 1);
        m.update_find_query("x".to_string());
        assert_eq!(m.find_matches.len(), 2);
        // Step to second match, then delete a row so the list shrinks.
        m.next_match();
        assert_eq!(m.find_current_match, Some(1));
        let mut sel = std::collections::BTreeSet::new();
        sel.insert(1usize);
        m.delete_rows(&sel);
        m.refresh_find_matches();
        // Only one "x" left — cursor must not point past the end.
        assert_eq!(m.find_matches.len(), 1);
        assert!(m.find_current_match.unwrap() < m.find_matches.len());
    }

    // ---- replace tests ------------------------------------------------------

    fn replace_model(src: &str) -> CsvModel {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.undo_stack.clear();
        m.redo_stack.clear();
        m
    }

    #[test]
    fn replace_current_substring_preserves_surrounding_text() {
        let mut m = replace_model("a,b\nhello world,c\n");
        m.update_find_query("world".to_string());
        m.replace_text = "there".to_string();
        assert!(m.replace_current());
        assert_eq!(m.cell(0, 0), "hello there");
    }

    #[test]
    fn replace_current_whole_cell_replaces_full() {
        let mut m = replace_model("a,b\nhello world,c\n");
        m.find_whole_cell = true;
        m.update_find_query("hello world".to_string());
        m.replace_text = "bye".to_string();
        assert!(m.replace_current());
        assert_eq!(m.cell(0, 0), "bye");
    }

    #[test]
    fn replace_current_advances_to_next_match() {
        let mut m = replace_model("a\nx\nx\nx\n");
        m.update_find_query("x".to_string());
        // Starts at first match (row 0). Replace → row 0 becomes "z";
        // current pointer should advance.
        m.replace_text = "z".to_string();
        assert_eq!(m.find_current_match, Some(0));
        assert!(m.replace_current());
        // After replace, find_matches has 2 entries (rows 1 and 2).
        assert_eq!(m.find_matches.len(), 2);
        // Pointer should be on the next match.
        assert!(m.find_current_match.is_some());
    }

    #[test]
    fn replace_all_substring_replaces_every_occurrence() {
        let mut m = replace_model("a\nfoo bar\nfoo baz\n");
        m.update_find_query("foo".to_string());
        m.replace_text = "qux".to_string();
        let n = m.replace_all();
        assert_eq!(n, 2);
        assert_eq!(m.cell(0, 0), "qux bar");
        assert_eq!(m.cell(1, 0), "qux baz");
    }

    #[test]
    fn replace_all_returns_count() {
        let mut m = replace_model("a\nx\ny\nx\nx\n");
        m.update_find_query("x".to_string());
        m.replace_text = "Z".to_string();
        assert_eq!(m.replace_all(), 3);
    }

    #[test]
    fn replace_all_is_one_undo_grain() {
        let mut m = replace_model("a\nx\nx\nx\nx\nx\n");
        m.update_find_query("x".to_string());
        m.replace_text = "Z".to_string();
        let n = m.replace_all();
        assert_eq!(n, 5);
        // One undo should restore all five cells.
        assert!(m.undo());
        for r in 0..5 {
            assert_eq!(m.cell(r, 0), "x", "row {r} should be restored");
        }
    }

    #[test]
    fn replace_all_with_empty_string_deletes() {
        let mut m = replace_model("a\nabc\nxyz\n");
        m.update_find_query("b".to_string());
        m.replace_text = String::new();
        m.replace_all();
        assert_eq!(m.cell(0, 0), "ac");
    }

    #[test]
    fn replace_current_with_empty_query_is_noop() {
        let mut m = replace_model("a\nx\n");
        m.find_query.clear();
        m.find_current_match = None;
        m.replace_text = "Z".to_string();
        assert!(!m.replace_current());
        assert_eq!(m.cell(0, 0), "x");
    }

    #[test]
    fn replace_case_insensitive_preserves_original_case_of_surrounding_text() {
        let mut m = replace_model("a\nHello WORLD!\n");
        m.find_case_sensitive = false;
        m.update_find_query("world".to_string());
        m.replace_text = "there".to_string();
        m.replace_current();
        // Surrounding case (Hello, !) preserved; matched substring
        // replaced.
        assert_eq!(m.cell(0, 0), "Hello there!");
    }

    #[test]
    fn replace_case_insensitive_long_non_ascii_cell_finishes() {
        // Lower-casing the rest of the cell once per char was quadratic:
        // a 40k-char non-ASCII cell took tens of seconds.
        let tail = "É".repeat(10);
        let cell = format!("{}X{tail}", "é".repeat(40_000));
        let mut m = replace_model(&format!("h\n{cell}\n"));
        m.find_case_sensitive = false;
        m.update_find_query("x".to_string());
        m.replace_text = "y".to_string();
        let started = std::time::Instant::now();
        assert_eq!(m.replace_all(), 1);
        let elapsed = started.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(2), "replace_all took {elapsed:?}");
        assert_eq!(m.cell(0, 0), format!("{}y{tail}", "é".repeat(40_000)));
    }

    #[test]
    fn replace_case_insensitive_non_ascii_keeps_surrounding_text() {
        let mut m = replace_model("h\nCafé CAFÉ café\n");
        m.find_case_sensitive = false;
        m.update_find_query("CAFÉ".to_string());
        m.replace_text = "bar".to_string();
        assert_eq!(m.replace_all(), 1);
        assert_eq!(m.cell(0, 0), "bar bar bar");

        // 'İ' lower-cases to two chars ("i\u{307}"), so lowered and
        // original offsets drift apart after it.
        let mut m = replace_model("h\nİx İX\n");
        m.find_case_sensitive = false;
        m.update_find_query("x".to_string());
        m.replace_text = "-".to_string();
        assert_eq!(m.replace_all(), 1);
        assert_eq!(m.cell(0, 0), "İ- İ-");
    }

    // ---- AutoFilter tests --------------------------------------------------

    fn filter_model(src: &str) -> CsvModel {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.undo_stack.clear();
        m.redo_stack.clear();
        m
    }

    #[test]
    fn row_is_visible_no_filters_returns_true() {
        let m = filter_model("a,b\n1,2\n3,4\n");
        assert!(m.row_is_visible(0));
        assert!(m.row_is_visible(1));
    }

    #[test]
    fn row_is_visible_hides_matching_value() {
        let mut m = filter_model("a,b\n1,2\n3,4\n5,6\n");
        let mut hidden = HashSet::new();
        hidden.insert("3".to_string());
        m.col_filters.insert(0, hidden);
        assert!(m.row_is_visible(0));
        assert!(!m.row_is_visible(1));
        assert!(m.row_is_visible(2));
    }

    #[test]
    fn row_is_visible_out_of_range_returns_true() {
        let m = filter_model("a\n1\n");
        assert!(m.row_is_visible(99));
    }

    #[test]
    fn row_is_visible_multi_column_intersection() {
        let mut m = filter_model("a,b\n1,X\n2,Y\n3,X\n");
        let mut h0 = HashSet::new();
        h0.insert("3".to_string());
        let mut h1 = HashSet::new();
        h1.insert("Y".to_string());
        m.col_filters.insert(0, h0);
        m.col_filters.insert(1, h1);
        // Row 0 (1,X) — passes both filters → visible
        // Row 1 (2,Y) — col 1 hides Y → hidden
        // Row 2 (3,X) — col 0 hides 3 → hidden
        assert!(m.row_is_visible(0));
        assert!(!m.row_is_visible(1));
        assert!(!m.row_is_visible(2));
    }

    #[test]
    fn hidden_row_count_reports_total() {
        let mut m = filter_model("a\n1\n2\n3\n4\n");
        let mut hidden = HashSet::new();
        hidden.insert("2".to_string());
        hidden.insert("4".to_string());
        m.col_filters.insert(0, hidden);
        assert_eq!(m.hidden_row_count(), 2);
    }

    #[test]
    fn any_filter_active_only_with_nonempty_set() {
        let mut m = filter_model("a\n1\n2\n");
        assert!(!m.any_filter_active());
        m.col_filters.insert(0, HashSet::new());
        assert!(!m.any_filter_active());
        let mut hidden = HashSet::new();
        hidden.insert("1".to_string());
        m.col_filters.insert(0, hidden);
        assert!(m.any_filter_active());
    }

    #[test]
    fn unique_values_returns_sorted_distinct() {
        let m = filter_model("a\nfoo\nbar\nfoo\nbaz\n");
        let v = m.unique_values_for_col(0);
        assert_eq!(v, vec!["bar".to_string(), "baz".to_string(), "foo".to_string()]);
    }

    #[test]
    fn unique_values_numeric_sort_then_text() {
        let m = filter_model("a\n10\nfoo\n2\nbar\n");
        let v = m.unique_values_for_col(0);
        // Numerics first in numeric order (2, 10), then non-numeric
        // in case-insensitive string order (bar, foo).
        assert_eq!(v, vec!["2".to_string(), "10".to_string(), "bar".to_string(), "foo".to_string()]);
    }

    #[test]
    fn unique_values_includes_empty_string() {
        let m = filter_model("a,b\n1,x\n,y\n2,\n");
        let v = m.unique_values_for_col(1);
        assert!(v.contains(&"".to_string()));
    }

    #[test]
    fn unique_values_excludes_rows_hidden_by_other_cols() {
        let mut m = filter_model("a,b\nfoo,1\nbar,2\nbaz,1\n");
        // Filter col 1 to hide value "2" — col 0's unique list should
        // exclude "bar" because that row is hidden by the other filter.
        let mut h1 = HashSet::new();
        h1.insert("2".to_string());
        m.col_filters.insert(1, h1);
        let v = m.unique_values_for_col(0);
        assert!(!v.contains(&"bar".to_string()));
        assert!(v.contains(&"foo".to_string()));
        assert!(v.contains(&"baz".to_string()));
    }

    #[test]
    fn open_filter_dropdown_stages_existing_hidden_set() {
        let mut m = filter_model("a\nfoo\nbar\nfoo\n");
        let mut hidden = HashSet::new();
        hidden.insert("foo".to_string());
        m.col_filters.insert(0, hidden.clone());
        m.open_filter_dropdown_for_col(0, gpui::point(px(0.), px(0.)));
        let state = m.open_filter_dropdown.as_ref().unwrap();
        assert_eq!(state.col, 0);
        assert_eq!(state.staged_hidden, hidden);
    }

    #[test]
    fn commit_filter_dropdown_moves_staged_to_col_filters() {
        let mut m = filter_model("a\nfoo\nbar\n");
        m.open_filter_dropdown_for_col(0, gpui::point(px(0.), px(0.)));
        let mut hidden = HashSet::new();
        hidden.insert("foo".to_string());
        m.open_filter_dropdown.as_mut().unwrap().staged_hidden = hidden.clone();
        assert_eq!(m.commit_filter_dropdown(), Some(0));
        assert_eq!(m.col_filters.get(&0).unwrap(), &hidden);
        assert!(m.open_filter_dropdown.is_none());
    }

    #[test]
    fn commit_empty_staged_removes_filter_entry() {
        let mut m = filter_model("a\nfoo\nbar\n");
        let mut existing = HashSet::new();
        existing.insert("foo".to_string());
        m.col_filters.insert(0, existing);
        m.open_filter_dropdown_for_col(0, gpui::point(px(0.), px(0.)));
        m.open_filter_dropdown.as_mut().unwrap().staged_hidden = HashSet::new();
        m.commit_filter_dropdown();
        assert!(!m.col_filters.contains_key(&0));
    }

    #[test]
    fn cancel_filter_dropdown_drops_state() {
        let mut m = filter_model("a\nfoo\n");
        m.open_filter_dropdown_for_col(0, gpui::point(px(0.), px(0.)));
        m.cancel_filter_dropdown();
        assert!(m.open_filter_dropdown.is_none());
    }

    #[test]
    fn clear_filter_for_col_removes_entry() {
        let mut m = filter_model("a,b\n1,2\n");
        let mut h = HashSet::new();
        h.insert("1".to_string());
        m.col_filters.insert(0, h);
        m.clear_filter_for_col(0);
        assert!(m.col_filters.is_empty());
    }

    #[test]
    fn clear_all_filters_empties_map() {
        let mut m = filter_model("a,b\n1,2\n");
        let mut h0 = HashSet::new();
        h0.insert("1".to_string());
        let mut h1 = HashSet::new();
        h1.insert("2".to_string());
        m.col_filters.insert(0, h0);
        m.col_filters.insert(1, h1);
        m.clear_all_filters();
        assert!(m.col_filters.is_empty());
    }

    #[test]
    fn toggle_filters_visible_flips_state() {
        let mut m = filter_model("a\n1\n");
        assert!(!m.filters_visible);
        m.toggle_filters_visible();
        assert!(m.filters_visible);
        m.toggle_filters_visible();
        assert!(!m.filters_visible);
    }

    #[test]
    fn toggling_filter_row_off_dismisses_open_dropdown() {
        let mut m = filter_model("a\n1\n");
        m.filters_visible = true;
        m.open_filter_dropdown_for_col(0, gpui::point(px(0.), px(0.)));
        m.toggle_filters_visible();
        assert!(m.open_filter_dropdown.is_none());
    }

    #[test]
    fn delete_col_drops_filter_entry() {
        let mut m = filter_model("a,b,c\n1,2,3\n4,5,6\n");
        let mut h = HashSet::new();
        h.insert("2".to_string());
        m.col_filters.insert(1, h.clone());
        let to_delete: BTreeSet<usize> = [1].into_iter().collect();
        m.delete_cols(&to_delete);
        // After deleting col 1, no entry should remain at index 1
        // (the original col 1's filter is gone).
        assert!(!m.col_filters.contains_key(&1));
    }

    #[test]
    fn delete_col_shifts_later_filter_keys_down() {
        let mut m = filter_model("a,b,c\n1,2,3\n4,5,6\n");
        // Filter on col 2 → after deleting col 0, should land at col 1.
        let mut h = HashSet::new();
        h.insert("3".to_string());
        m.col_filters.insert(2, h.clone());
        let to_delete: BTreeSet<usize> = [0].into_iter().collect();
        m.delete_cols(&to_delete);
        assert!(!m.col_filters.contains_key(&2));
        assert_eq!(m.col_filters.get(&1).unwrap(), &h);
    }

    #[test]
    fn insert_col_shifts_later_filter_keys_up() {
        let mut m = filter_model("a,b\n1,2\n3,4\n");
        let mut h = HashSet::new();
        h.insert("2".to_string());
        m.col_filters.insert(1, h.clone());
        m.insert_col_at(0, true);
        // Filter at col 1 should slide to col 2.
        assert_eq!(m.col_filters.get(&2).unwrap(), &h);
        assert!(!m.col_filters.contains_key(&1));
    }

    #[test]
    fn col_reorder_remaps_filter_keys() {
        let mut m = filter_model("a,b,c\n1,2,3\n");
        let mut h = HashSet::new();
        h.insert("3".to_string());
        m.col_filters.insert(2, h.clone());
        // Move col 2 to position 0 — apply_col_order with explicit
        // permutation.
        m.col_order = vec![2, 0, 1];
        m.apply_col_order();
        assert_eq!(m.col_filters.get(&0).unwrap(), &h);
        assert!(!m.col_filters.contains_key(&2));
    }

    #[test]
    fn source_rebuild_clears_filters() {
        let mut m = filter_model("a\nfoo\n");
        let mut h = HashSet::new();
        h.insert("foo".to_string());
        m.col_filters.insert(0, h);
        m.rebuild_from_source("a\nbar\n", 2);
        assert!(m.col_filters.is_empty());
    }

    // ---- Fill-series drag --------------------------------------------------

    fn fresh(src: &str) -> CsvModel {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.undo_stack.clear();
        m.redo_stack.clear();
        m
    }

    #[test]
    fn fill_drag_extends_numeric_down() {
        let mut m = fresh("n\n1\n2\n0\n0\n0\n");
        // Source = rows 0..=1 in col 0 (values 1, 2)
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        assert!(m.begin_fill_drag(gpui::point(px(0.), px(0.))));
        // Extend down to row 4 (3 new cells)
        m.update_fill_drag((4, 0), 0.0, 100.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.cell(2, 0), "3");
        assert_eq!(m.cell(3, 0), "4");
        assert_eq!(m.cell(4, 0), "5");
    }

    #[test]
    fn fill_drag_extends_numeric_right() {
        // 1 row × 5 cols, source = (0,0)..(0,1) = [1, 2], extend right
        let mut m = fresh("a,b,c,d,e\n1,2,0,0,0\n");
        m.cell_range = Some(CellRange::new((0, 0), (0, 1)));
        assert!(m.begin_fill_drag(gpui::point(px(0.), px(0.))));
        m.update_fill_drag((0, 4), 200.0, 0.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.cell(0, 2), "3");
        assert_eq!(m.cell(0, 3), "4");
        assert_eq!(m.cell(0, 4), "5");
    }

    #[test]
    fn fill_drag_extends_up_reverse() {
        let mut m = fresh("n\n0\n0\n5\n6\n");
        // Source = rows 2..=3 (values 5, 6). Extend up to row 0.
        m.cell_range = Some(CellRange::new((2, 0), (3, 0)));
        assert!(m.begin_fill_drag(gpui::point(px(0.), px(0.))));
        m.update_fill_drag((0, 0), 0.0, -100.0);
        assert!(m.commit_fill_drag());
        // Going up: cell[0] = 3, cell[1] = 4 (top-to-bottom).
        assert_eq!(m.cell(0, 0), "3");
        assert_eq!(m.cell(1, 0), "4");
    }

    #[test]
    fn fill_drag_constant_for_single_cell() {
        // Single-cell source = no increment, just copy.
        let mut m = fresh("n\n7\n0\n0\n0\n");
        m.cell_range = Some(CellRange::new((0, 0), (0, 0)));
        assert!(m.begin_fill_drag(gpui::point(px(0.), px(0.))));
        m.update_fill_drag((3, 0), 0.0, 100.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.cell(1, 0), "7");
        assert_eq!(m.cell(2, 0), "7");
        assert_eq!(m.cell(3, 0), "7");
    }

    #[test]
    fn fill_drag_weekday_cycle() {
        // Pre-seed with placeholder body cells so the parser keeps the
        // empty trailing rows (the csv reader collapses bare `\n\n`).
        let mut m = fresh("d\nMon\nTue\nx\nx\nx\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((4, 0), 0.0, 100.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.cell(2, 0), "Wed");
        assert_eq!(m.cell(3, 0), "Thu");
        assert_eq!(m.cell(4, 0), "Fri");
    }

    #[test]
    fn fill_drag_text_number_padded() {
        let mut m = fresh("x\nItem 01\nItem 02\nz\nz\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((3, 0), 0.0, 100.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.cell(2, 0), "Item 03");
        assert_eq!(m.cell(3, 0), "Item 04");
    }

    #[test]
    fn fill_drag_target_equals_source_no_change() {
        let mut m = fresh("n\n1\n2\n3\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((1, 0), 0.0, 0.0); // stay inside source
        assert!(!m.commit_fill_drag());
        // Original cells untouched.
        assert_eq!(m.cell(0, 0), "1");
        assert_eq!(m.cell(1, 0), "2");
        assert_eq!(m.cell(2, 0), "3");
    }

    #[test]
    fn fill_drag_commit_updates_cell_range_to_target() {
        let mut m = fresh("n\n1\n2\n0\n0\n0\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((4, 0), 0.0, 100.0);
        m.commit_fill_drag();
        let r = m.cell_range.expect("cell_range set on commit");
        assert_eq!(r.min_row(), 0);
        assert_eq!(r.max_row(), 4);
    }

    #[test]
    fn fill_drag_cancel_drops_state_without_writing() {
        let mut m = fresh("n\n1\n2\nplaceholder1\nplaceholder2\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((3, 0), 0.0, 100.0);
        m.cancel_fill_drag();
        assert!(m.fill_drag.is_none());
        // Cells outside the source were never written — placeholders
        // retained.
        assert_eq!(m.cell(2, 0), "placeholder1");
        assert_eq!(m.cell(3, 0), "placeholder2");
    }

    #[test]
    fn fill_drag_pushes_one_undo_grain() {
        let mut m = fresh("n\n1\n2\n0\n0\n");
        let before = m.undo_stack.len();
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((3, 0), 0.0, 100.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.undo_stack.len(), before + 1);
        // Undo restores the pre-fill state.
        assert!(m.undo());
        assert_eq!(m.cell(2, 0), "0");
    }

    #[test]
    fn fill_drag_picks_dominant_axis_diagonal() {
        // Source 2×2; cursor moves more in Y than X — should extend rows.
        let mut m = fresh("a,b\n1,2\n3,4\n0,0\n0,0\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 1)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        // Hover at (3, 3) — but 3 columns is beyond bounds (col_count=2),
        // so test with a smaller delta. Hover at (3, 1) with strong dy.
        m.update_fill_drag((3, 1), 5.0, 100.0);
        let target = m.fill_drag.unwrap().current_target;
        // Should have extended rows, not cols.
        assert_eq!(target.max_row(), 3);
        assert_eq!(target.max_col(), 1);
    }

    #[test]
    fn fill_drag_multi_col_source_extends_each_pattern() {
        // 2×2 numeric source — vertical extend fills each column with
        // its own pattern.
        let mut m = fresh("a,b\n1,10\n2,20\n0,0\n0,0\n");
        m.cell_range = Some(CellRange::new((0, 0), (1, 1)));
        m.begin_fill_drag(gpui::point(px(0.), px(0.)));
        m.update_fill_drag((3, 1), 0.0, 100.0);
        assert!(m.commit_fill_drag());
        assert_eq!(m.cell(2, 0), "3");
        assert_eq!(m.cell(3, 0), "4");
        assert_eq!(m.cell(2, 1), "30");
        assert_eq!(m.cell(3, 1), "40");
    }

    #[test]
    fn fill_drag_without_a_value_keeps_cells() {
        // The step overflows f64, so the pattern has no value for the
        // target cells; they keep their text and no undo grain is left.
        let mut m = fresh("n\n-1e308\n1.7e308\nkeep\nkeep\n");
        let before = m.undo_stack.len();
        m.cell_range = Some(CellRange::new((0, 0), (1, 0)));
        assert!(m.begin_fill_drag(gpui::point(px(0.), px(0.))));
        m.update_fill_drag((3, 0), 0.0, 100.0);
        assert!(!m.commit_fill_drag());
        assert_eq!(m.cell(2, 0), "keep");
        assert_eq!(m.cell(3, 0), "keep");
        assert_eq!(m.undo_stack.len(), before);
    }

    #[test]
    fn replace_whole_cell_case_insensitive_non_ascii() {
        // Find lowers with `to_lowercase`, so "CAFÉ" is a whole-cell
        // match for "café"; replace has to agree and rewrite it.
        let mut m = replace_model("h\nCAFÉ\ncafé x\n");
        m.find_case_sensitive = false;
        m.find_whole_cell = true;
        m.update_find_query("café".to_string());
        assert_eq!(m.find_matches, vec![(0, 0)]);
        m.replace_text = "tea".to_string();
        assert_eq!(m.replace_all(), 1);
        assert_eq!(m.cell(0, 0), "tea");
        assert_eq!(m.cell(1, 0), "café x");
    }

    // ---- Out-of-range indices -----------------------------------------------

    #[test]
    fn find_step_from_stale_index_stays_in_range() {
        let mut m = fresh("h\nx\nx\n");
        m.update_find_query("x".to_string());
        assert_eq!(m.find_matches, vec![(0, 0), (1, 0)]);
        m.find_current_match = Some(99);
        assert_eq!(m.prev_match(), Some((1, 0)));
        m.find_current_match = Some(99);
        assert_eq!(m.next_match(), Some((0, 0)));
        m.find_current_match = Some(usize::MAX);
        assert_eq!(m.next_match(), Some((0, 0)));
        assert_eq!(m.find_current_match, Some(0));
    }

    #[test]
    fn insert_after_last_index_appends() {
        let mut m = fresh("a,b\n1,2\n");
        m.insert_row_at(usize::MAX, false);
        assert_eq!(m.row_count(), 2);
        assert_eq!(m.cell(0, 0), "1");
        assert_eq!(m.cell(1, 0), "");
        m.insert_col_at(usize::MAX, false);
        assert_eq!(m.col_count(), 3);
        assert_eq!(m.header(1), "b");
        assert_eq!(m.cell(0, 1), "2");
    }

    #[test]
    fn col_letter_at_usize_max() {
        let last = col_letter(usize::MAX);
        assert!(!last.is_empty());
        assert!(last.bytes().all(|b| b.is_ascii_uppercase()));
        assert_ne!(last, col_letter(usize::MAX - 1));
    }

    #[test]
    fn paste_block_past_usize_max_is_refused() {
        let mut m = fresh("a\n1\n");
        let block = vec![vec!["x".to_string()], vec!["y".to_string()]];
        assert!(!m.paste_at(usize::MAX, 0, &block, None));
        assert!(!m.paste_at(0, usize::MAX, &[vec!["x".to_string(), "y".to_string()]], None));
        assert_eq!((m.row_count(), m.col_count()), (1, 1));
        assert_eq!(m.cell(0, 0), "1");
        assert!(m.undo_stack.is_empty());
    }

    #[test]
    fn paste_into_unbounded_range_fills_only_the_sheet() {
        let mut m = fresh("a,b\n1,2\n3,4\n");
        let whole = Some(CellRange::new((0, 0), (usize::MAX, usize::MAX)));
        assert!(m.paste_at(0, 0, &[vec!["z".to_string()]], whole));
        assert_eq!((m.row_count(), m.col_count()), (2, 2));
        assert!((0..2).all(|r| (0..2).all(|c| m.cell(r, c) == "z")));
        let tile = vec![vec!["p".to_string(), "q".to_string()]];
        assert!(m.paste_at(0, 0, &tile, whole));
        assert_eq!((m.row_count(), m.col_count()), (2, 2));
    }

    #[test]
    fn emit_that_cannot_be_utf8_does_not_blank_the_document() {
        // Every delimiter the UI offers is ASCII, but a byte past 0x7F
        // splits a multibyte cell into invalid UTF-8 on the way out.
        let mut m = fresh("a,b\nél,2\n");
        m.delimiter = 0xC3;
        assert_eq!(m.emit_source(), None);
    }

    #[test]
    fn emit_keeps_rows_longer_than_the_header_on_their_own_lines() {
        // Deleting every column leaves zero-length rows; Add Row then
        // gives each new row one cell, and Add Column widens all of them,
        // so the new rows end up one cell longer than the header.
        let mut m = fresh("a\n1\n");
        m.delete_cols(&BTreeSet::from([0]));
        m.add_row();
        m.add_row();
        m.add_column();
        assert!(m.set_cell(1, 0, "one".to_string()));
        assert!(m.set_cell(2, 0, "two".to_string()));
        let again = fresh(&m.emit_source().unwrap());
        assert_eq!(again.row_count(), 3);
        assert_eq!((again.cell(1, 0), again.cell(2, 0)), ("one", "two"));
    }

    // ---- Hostile input: parse matrix -----------------------------------------

    /// Emit `m` and parse the result with the same settings: the cells
    /// must come back unchanged, whatever shape the original source had.
    fn assert_emit_reparses(m: &CsvModel) {
        let mut again = CsvModel::empty(m.delimiter);
        again.has_header = m.has_header;
        again.rebuild_from_source(&m.emit_source().unwrap(), 2);
        assert_eq!(
            (&again.headers, &again.rows),
            (&m.headers, &m.rows),
            "emit of {:?} parsed differently",
            m.emit_source().unwrap()
        );
    }

    /// Sources a CSV file can hand the parser that are not a tidy grid.
    fn hostile_sources() -> Vec<String> {
        let mut sources: Vec<String> = [
            "",
            "\u{FEFF}",
            "x",
            "\"",
            "\"\"",
            "\"\"\"",
            "a,b\n\"open,1\n2,3\n",
            "a,b\nx\"y,z\n",
            "a,b\n1,\"\"",
            "\"a\"b\"c",
            "a\n1,2,3\n4,5\n\n6\n",
            "a,b\r1,2\r3,4\r",
            "a,b\r\n1,2\n3,4\r5,6",
            "a,b\r\n\"x\r\ny\",2\r\n",
            "\u{FEFF}\"a\",b\r\n1,2",
            "naïve,中文,👩‍👩‍👧\né,中,🏳️‍🌈\n",
            "a,b,",
            "a,b\n1,2,\n",
            "a,b\n1,2\n\n\n",
            "\n\n\r\n",
            "a;b\t|c\0d\n",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        sources.push(format!("{}\n1\n", ",".repeat(999)));
        sources
    }

    #[test]
    fn parse_matrix_reparses_under_every_delimiter_and_header_mode() {
        for src in hostile_sources() {
            for &delim in b",;\t|" {
                for has_header in [true, false] {
                    let mut m = CsvModel::empty(delim);
                    m.has_header = has_header;
                    m.rebuild_from_source(&src, 1);
                    let cols = m.col_count();
                    assert_eq!(m.headers.len(), cols, "{src:?}");
                    assert!(m.rows.iter().all(|r| r.len() == cols), "{src:?}");
                    assert_emit_reparses(&m);
                }
            }
        }
    }

    #[test]
    fn parse_empty_and_single_cell_round_trip_bytes() {
        for src in ["", "\u{FEFF}", "x", "x\n", "\u{FEFF}x\r\n"] {
            assert_eq!(fresh(src).emit_source().unwrap(), src);
        }
        let m = fresh("x");
        assert_eq!((m.row_count(), m.col_count()), (0, 1));
        assert_eq!(m.header(0), "x");
    }

    #[test]
    fn parse_unbalanced_quotes() {
        // An open quote that never closes runs to the end of the file.
        let m = fresh("a,b\n\"open,1\n2,3\n");
        assert_eq!(m.row_count(), 1);
        assert_eq!(m.cell(0, 0), "open,1\n2,3\n");
        // A quote inside an unquoted field is plain text.
        let m = fresh("a,b\nx\"y,z\n");
        assert_eq!((m.cell(0, 0), m.cell(0, 1)), ("x\"y", "z"));
        // An empty quoted field right at EOF.
        let m = fresh("a,b\n1,\"\"");
        assert_eq!((m.cell(0, 0), m.cell(0, 1)), ("1", ""));
    }

    #[test]
    fn parse_ragged_rows_pad_to_the_widest() {
        let m = fresh("a\n1,2,3\n4,5\n\n6\n");
        assert_eq!((m.row_count(), m.col_count()), (3, 3));
        assert_eq!(m.headers, ["a", "", ""]);
        assert_eq!((m.cell(1, 1), m.cell(1, 2), m.cell(2, 0)), ("5", "", "6"));
    }

    #[test]
    fn parse_million_columns_on_one_line() {
        let line = ",".repeat(999_999);
        let started = std::time::Instant::now();
        let m = fresh(&line);
        assert_eq!((m.row_count(), m.col_count()), (0, 1_000_000));
        assert_eq!(m.emit_source().unwrap(), line);
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_secs(20), "took {took:?}");
    }

    #[test]
    fn parse_cr_only_mixed_endings_and_quoted_breaks() {
        let m = fresh("a,b\r1,2\r3,4\r");
        assert_eq!(m.row_count(), 2);
        assert_eq!((m.cell(1, 0), m.cell(1, 1)), ("3", "4"));
        let m = fresh("a,b\r\n1,2\n3,4\r5,6");
        assert_eq!(m.row_count(), 3);
        assert_eq!(m.cell(2, 1), "6");
        let m = fresh("a,b\r\n\"x\r\ny\",2\r\n");
        assert_eq!(m.cell(0, 0), "x\r\ny");
        assert_eq!(m.emit_source().unwrap(), "a,b\r\n\"x\r\ny\",2\r\n");
        let m = fresh("\u{FEFF}\"a\",b\r\n1,2");
        assert_eq!((m.header(0), m.cell(0, 1)), ("a", "2"));
    }

    #[test]
    fn parse_multibyte_cells_round_trip_and_find() {
        let src = "naïve,中文,👩‍👩‍👧\né,中,🏳️‍🌈\n";
        let mut m = fresh(src);
        assert_eq!(m.header(2), "👩‍👩‍👧");
        assert_eq!(m.cell(0, 2), "🏳️‍🌈");
        assert_eq!(m.emit_source().unwrap(), src);
        m.find_case_sensitive = false;
        m.update_find_query("\u{200D}".to_string());
        assert_eq!(m.find_matches, vec![(usize::MAX, 2), (0, 2)]);
        m.replace_text = "+".to_string();
        assert_eq!(m.replace_all(), 2);
        assert_eq!(m.header(2), "👩+👩+👧");
        assert_eq!(m.cell(0, 2), "🏳️+🌈");
    }

    #[test]
    fn parse_delimiter_at_eof_and_trailing_blank_lines() {
        let m = fresh("a,b,");
        assert_eq!(m.headers, ["a", "b", ""]);
        assert_eq!(m.emit_source().unwrap(), "a,b,");
        let m = fresh("a,b\n1,2,\n");
        assert_eq!(m.col_count(), 3);
        let m = fresh("a,b\n1,2\n\n\n");
        assert_eq!(m.row_count(), 1);
        let m = fresh("\n\n\r\n");
        assert_eq!((m.row_count(), m.col_count()), (0, 0));
    }

    // ---- Hostile input: actions on empty, small, ragged and stale state -----

    /// What the user sees: the header row, then every body row, in
    /// logical order.
    fn grid_view(m: &CsvModel) -> Vec<Vec<String>> {
        let cols = m.col_count();
        let mut view = vec![(0..cols).map(|c| m.header(c).to_string()).collect()];
        view.extend(
            (0..m.row_count()).map(|r| (0..cols).map(|c| m.cell(r, c).to_string()).collect()),
        );
        view
    }

    fn arm_fill(m: &mut CsvModel, range: CellRange, hovered: (usize, usize), dx: f32, dy: f32) -> bool {
        m.cell_range = Some(range);
        if !m.begin_fill_drag(gpui::point(px(0.), px(0.))) {
            return false;
        }
        m.update_fill_drag(hovered, dx, dy);
        m.commit_fill_drag()
    }

    #[test]
    fn actions_on_an_empty_sheet_change_nothing() {
        let mut m = fresh("");
        let range = CellRange::new((0, 0), (5, 5));
        assert!(!m.handle_delete_key());
        assert!(!m.clear_range(range));
        assert!(!m.fill_down_range(range));
        assert!(!m.fill_right_range(range));
        assert!(!m.clear_rows(&BTreeSet::from([0, 9])));
        assert!(!m.clear_cols(&BTreeSet::from([0, 9])));
        m.delete_rows(&BTreeSet::new());
        m.delete_cols(&BTreeSet::new());
        m.delete_row(0);
        m.delete_column(usize::MAX);
        m.sort_by_col(0, true);
        assert!(m.unique_values_for_col(0).is_empty());
        m.open_filter_dropdown_for_col(0, gpui::point(px(0.), px(0.)));
        assert!(m.open_filter_dropdown.is_none());
        assert_eq!(m.hidden_row_count(), 0);
        assert!(!m.move_row_block(&[0], 0, 3).1);
        assert!(!m.move_col_block(&[0], 0, 3).1);
        m.update_find_query("x".to_string());
        assert_eq!((m.next_match(), m.prev_match()), (None, None));
        assert!(!m.replace_current());
        assert_eq!(m.replace_all(), 0);
        assert!(!m.set_cell(0, 0, "x".to_string()));
        assert!(!m.set_header(0, "x".to_string()));
        m.auto_fit_all_columns(stub_measure_7px);
        m.auto_fit_column(0, stub_measure_7px);
        assert!(!m.paste_at(0, 0, &[], None));
        assert!(!m.paste_at(0, 0, &[vec![], vec![]], Some(range)));
        assert!(!arm_fill(&mut m, CellRange::new((0, 0), (0, 0)), (4, 0), 0.0, 100.0));
        assert!(m.undo_stack.is_empty());
        m.toggle_has_header();
        m.toggle_has_header();
        assert_eq!((m.row_count(), m.col_count()), (0, 0));
        assert_eq!(m.emit_source().unwrap(), "");
    }

    #[test]
    fn stale_selection_past_the_end_after_deletes() {
        let mut m = fresh("a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
        let stale = CellRange::new((1, 1), (2, 2));
        m.delete_rows(&BTreeSet::from([1, 2]));
        m.delete_cols(&BTreeSet::from([1, 2]));
        assert_eq!(m.emit_source().unwrap(), "a\n1\n");
        let undo_depth = m.undo_stack.len();

        m.cell_range = Some(stale);
        m.active_cell = Some((2, 2));
        assert!(!m.handle_delete_key());
        assert!(!m.clear_range(stale));
        assert!(!m.fill_down_range(stale));
        assert!(!m.fill_right_range(stale));
        assert!(!m.set_cell(2, 2, "x".to_string()));
        assert!(!m.set_header(2, "x".to_string()));
        assert!(!m.clear_rows(&BTreeSet::from([5])));
        assert!(!m.clear_cols(&BTreeSet::from([5])));
        assert!(!m.move_row_block(&[5], 5, 0).1);
        assert!(!m.move_col_block(&[5], 5, 0).1);
        m.sort_by_col(5, true);
        assert!(!arm_fill(&mut m, stale, (0, 0), 0.0, -100.0));
        assert!(!arm_fill(&mut m, stale, (9, 2), 0.0, 100.0));
        assert!(!arm_fill(&mut m, stale, (2, 9), 100.0, 0.0));
        m.update_find_query("1".to_string());
        m.find_current_match = Some(7);
        assert!(!m.replace_current());
        assert_eq!(m.undo_stack.len(), undo_depth);
        assert_eq!(m.emit_source().unwrap(), "a\n1\n");
    }

    #[test]
    fn single_row_and_single_column_sheets() {
        let mut m = fresh("h\n7\n");
        m.sort_by_col(0, false);
        assert_eq!(m.unique_values_for_col(0), ["7"]);
        assert!(!m.fill_down_range(CellRange::new((0, 0), (0, 0))));
        assert!(!m.fill_right_range(CellRange::new((0, 0), (0, 0))));
        // The fill handle dragged past the last row writes nothing.
        assert!(!arm_fill(&mut m, CellRange::new((0, 0), (0, 0)), (3, 0), 0.0, 100.0));
        assert!(!arm_fill(&mut m, CellRange::new((0, 0), (0, 0)), (0, 3), 100.0, 0.0));
        m.update_find_query("7".to_string());
        assert_eq!(m.next_match(), Some((0, 0)));
        assert_eq!(m.next_match(), Some((0, 0)));
        assert_eq!(m.prev_match(), Some((0, 0)));
        assert_eq!(m.emit_source().unwrap(), "h\n7\n");

        let mut m = fresh("a,b,c\n");
        m.sort_by_col(1, true);
        assert!(m.unique_values_for_col(1).is_empty());
        assert!(!arm_fill(&mut m, CellRange::new((0, 0), (0, 1)), (0, 2), 100.0, 0.0));
        m.update_find_query("b".to_string());
        assert_eq!(m.find_matches, vec![(usize::MAX, 1)]);
        m.replace_text = "B".to_string();
        assert!(m.replace_current());
        assert_eq!(m.emit_source().unwrap(), "a,B,c\n");
    }

    #[test]
    fn fill_series_on_zero_and_one_source_rows() {
        let mut m = fresh("n\n1\nx\nx\n");
        assert!(arm_fill(&mut m, CellRange::new((0, 0), (0, 0)), (2, 0), 0.0, 100.0));
        assert_eq!((m.cell(1, 0), m.cell(2, 0)), ("1", "1"));
        // A source range over rows that no longer exist projects onto
        // nothing.
        let mut m = fresh("n\n");
        assert!(!arm_fill(&mut m, CellRange::new((0, 0), (1, 0)), (3, 0), 0.0, 100.0));
        assert!(!arm_fill(&mut m, CellRange::new((2, 0), (3, 0)), (0, 0), 0.0, -100.0));
        assert_eq!(m.row_count(), 0);
        assert!(m.undo_stack.is_empty());
    }

    #[test]
    fn sort_filter_find_on_ragged_rows() {
        let mut m = fresh("a\n3\n1,x,y\n2,z\n");
        let mut before: Vec<String> = m.rows.concat();
        before.sort();
        m.sort_by_col(2, true);
        m.sort_by_col(1, false);
        m.sort_by_col(0, true);
        assert_eq!((m.cell(0, 0), m.cell(1, 0), m.cell(2, 0)), ("1", "2", "3"));
        let mut after: Vec<String> = m.rows.concat();
        after.sort();
        assert_eq!(after, before);
        assert!(m.rows.iter().all(|r| r.len() == 3));

        assert_eq!(m.unique_values_for_col(2), ["", "y"]);
        m.col_filters.insert(2, HashSet::from([String::new()]));
        assert_eq!(m.hidden_row_count(), 2);
        assert!(m.row_is_visible(0));
        assert!(m.row_is_visible(usize::MAX));
        m.update_find_query("y".to_string());
        assert_eq!(m.find_matches, vec![(0, 2)]);
        m.replace_text = String::new();
        assert_eq!(m.replace_all(), 1);
        assert_eq!(m.hidden_row_count(), 3);
        assert_emit_reparses(&m);
    }

    #[test]
    fn insert_and_delete_at_both_ends() {
        let mut m = fresh("a,b\n1,2\n3,4\n");
        m.insert_row_at(0, true);
        m.insert_row_at(m.row_count() - 1, false);
        m.insert_col_at(0, true);
        m.insert_col_at(m.col_count() - 1, false);
        assert_eq!((m.row_count(), m.col_count()), (4, 4));
        assert_eq!(m.emit_source().unwrap(), "Column 1,a,b,Column 4\n,,,\n,1,2,\n,3,4,\n,,,\n");
        m.delete_row(0);
        m.delete_row(m.row_count() - 1);
        m.delete_column(0);
        m.delete_column(m.col_count() - 1);
        m.delete_row(99);
        m.delete_column(99);
        assert_eq!(m.emit_source().unwrap(), "a,b\n1,2\n3,4\n");

        let mut m = fresh("");
        m.insert_col_at(0, true);
        m.insert_row_at(0, false);
        assert_eq!((m.row_count(), m.col_count()), (1, 1));
        m.delete_column(0);
        m.delete_row(0);
        assert_eq!((m.row_count(), m.col_count()), (0, 0));
    }

    type Op = Box<dyn Fn(&mut CsvModel)>;

    #[test]
    fn undo_redo_walks_structural_changes_both_ways() {
        let mut m = fresh("a,b\n1,2\n3,4\n");
        let ops: Vec<Op> = vec![
            Box::new(|m| m.insert_row_at(0, true)),
            Box::new(|m| m.insert_col_at(1, false)),
            Box::new(|m| m.delete_rows(&BTreeSet::from([0]))),
            Box::new(|m| m.sort_by_col(0, false)),
            Box::new(|m| {
                m.move_row_block(&[0], 0, 2);
            }),
            Box::new(|m| m.delete_cols(&BTreeSet::from([2]))),
            Box::new(|m| {
                let ragged = vec![vec!["p".to_string(), "q".to_string()], vec!["r".to_string()]];
                m.paste_at(1, 1, &ragged, None);
            }),
            Box::new(|m| m.toggle_has_header()),
            Box::new(|m| {
                m.set_cell(0, 0, "z".to_string());
            }),
            Box::new(|m| {
                m.fill_down_range(CellRange::new((0, 0), (3, 0)));
            }),
            Box::new(|m| m.add_row()),
            Box::new(|m| m.add_column()),
            Box::new(|m| m.delete_cols(&BTreeSet::from([0, 1, 2, 3]))),
        ];
        let mut states = vec![grid_view(&m)];
        for (i, op) in ops.iter().enumerate() {
            let depth = m.undo_stack.len();
            op(&mut m);
            assert_eq!(m.undo_stack.len(), depth + 1, "op {i} should leave one undo grain");
            states.push(grid_view(&m));
        }
        for expected in states.iter().rev().skip(1) {
            assert!(m.undo());
            assert_eq!(&grid_view(&m), expected);
        }
        assert!(!m.undo());
        for expected in states.iter().skip(1) {
            assert!(m.redo());
            assert_eq!(&grid_view(&m), expected);
        }
        assert!(!m.redo());
    }

    #[test]
    fn paste_ragged_empty_and_oversized_blocks() {
        let mut m = fresh("a,b\n1,2\n");
        assert!(!m.paste_at(0, 0, &[], None));
        assert!(!m.paste_at(0, 0, &[vec![], vec![]], None));
        assert!(m.undo_stack.is_empty());

        // Short rows leave the cells to their right alone; an empty row
        // still counts toward the block's height.
        let ragged = vec![
            vec!["x".to_string()],
            vec![],
            vec!["y".to_string(), "z".to_string(), "w".to_string()],
        ];
        assert!(m.paste_at(0, 0, &ragged, None));
        assert_eq!((m.row_count(), m.col_count()), (3, 3));
        assert_eq!(m.emit_source().unwrap(), "a,b,Column 3\nx,2,\n,,\ny,z,w\n");

        // A block bigger than the sheet, anchored at its last cell.
        let big: Vec<Vec<String>> = (0..50)
            .map(|r| (0..40).map(|c| format!("{r}:{c}")).collect())
            .collect();
        assert!(m.paste_at(2, 2, &big, None));
        assert_eq!((m.row_count(), m.col_count()), (52, 42));
        assert_eq!(m.cell(51, 41), "49:39");
        assert_eq!(m.headers.len(), 42);
        assert!(m.rows.iter().all(|r| r.len() == 42));

        // A ragged block tiled across a range: missing cells tile as blanks.
        let tile = vec![vec!["t".to_string(), "u".to_string()], vec!["v".to_string()]];
        assert!(m.paste_at(0, 0, &tile, Some(CellRange::new((0, 0), (3, 3)))));
        assert_eq!((m.cell(0, 1), m.cell(1, 0), m.cell(1, 1), m.cell(2, 2)), ("u", "v", "", "t"));
        assert_emit_reparses(&m);
        assert!(m.undo());
        assert!(m.undo());
        assert!(m.undo());
        assert_eq!(m.emit_source().unwrap(), "a,b\n1,2\n");
    }
}
