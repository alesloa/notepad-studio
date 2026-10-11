// CSV-specific undo/redo. Independent of the rope's text-side undo
// because the rope only sees the serialized output of `emit_source` —
// it can't restore selection, active_cell, or per-column widths. We
// keep two stacks of `CsvSnapshot` on the model, capped at MAX_DEPTH,
// and mutating methods call `push_undo` before they change anything.
//
// Width drag-resize is special-cased: the per-pixel `set_col_width`
// during mousemove does NOT push (would create one undo entry per
// pixel). The drag-start handler in `grid.rs` pushes once at mousedown.

use std::collections::BTreeSet;

use gpui::Pixels;

use super::model::{CellRange, CsvModel};

/// Maximum number of snapshots kept on each stack. Once exceeded, the
/// oldest entry is dropped on push. 200 covers a typical session
/// without ballooning memory — a 10k-row CSV snapshot is ~1 MB and
/// stacking 200 of those is still well under typical app overhead.
pub(crate) const MAX_DEPTH: usize = 200;

/// Exact source associated with a grid state. Rope clones share chunks;
/// retaining it lets Undo restore optional quoting as well as cell values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CsvSourceState {
    pub(crate) revision: crate::buffer::ContentRevision,
    pub(crate) rope: ropey::Rope,
}

/// Frozen copy of the CSV model state at one undo grain. Drag /
/// resize / edit-buffer fields are deliberately omitted — they are
/// ephemeral and would round-trip noisily through undo.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CsvSnapshot {
    pub(crate) source_state: Option<CsvSourceState>,
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<Vec<String>>,
    pub(crate) col_widths: Vec<Pixels>,
    pub(crate) col_order: Vec<usize>,
    pub(crate) row_order: Vec<usize>,
    pub(crate) active_cell: Option<(usize, usize)>,
    pub(crate) selected_rows: BTreeSet<usize>,
    pub(crate) selected_cols: BTreeSet<usize>,
    pub(crate) cell_range: Option<CellRange>,
    pub(crate) last_clicked_row: Option<usize>,
    pub(crate) last_clicked_col: Option<usize>,
    pub(crate) has_header: bool,
    pub(crate) delimiter: u8,
}

impl CsvModel {
    /// Build a snapshot of the current state. Cheap clone — no copies
    /// of focus handles, scroll handles or in-flight cell input.
    pub(crate) fn snapshot(&self) -> CsvSnapshot {
        CsvSnapshot {
            source_state: self.pending_source.as_ref().or(self.source_state.as_ref()).cloned(),
            headers: self.headers.clone(),
            rows: self.rows.clone(),
            col_widths: self.col_widths.clone(),
            col_order: self.col_order.clone(),
            row_order: self.row_order.clone(),
            active_cell: self.active_cell,
            selected_rows: self.selected_rows.clone(),
            selected_cols: self.selected_cols.clone(),
            cell_range: self.cell_range,
            last_clicked_row: self.last_clicked_row,
            last_clicked_col: self.last_clicked_col,
            has_header: self.has_header,
            delimiter: self.delimiter,
        }
    }

    /// Restore the model from a snapshot. Cancels any in-flight cell
    /// editor and bumps `source_version` so the workspace re-emits.
    pub(crate) fn restore(&mut self, snap: CsvSnapshot) {
        self.pending_source = snap.source_state;
        self.reparse_requested = false;
        self.headers = snap.headers;
        self.rows = snap.rows;
        self.col_widths = snap.col_widths;
        self.col_order = snap.col_order;
        self.row_order = snap.row_order;
        self.active_cell = snap.active_cell;
        self.selected_rows = snap.selected_rows;
        self.selected_cols = snap.selected_cols;
        self.cell_range = snap.cell_range;
        self.cell_range_drag = None;
        self.last_clicked_row = snap.last_clicked_row;
        self.last_clicked_col = snap.last_clicked_col;
        self.has_header = snap.has_header;
        self.delimiter = snap.delimiter;
        // In-flight cell editor / drag / resize state is meaningless
        // against the restored shape — drop it so the next paint shows
        // a clean grid.
        self.editing_cell = None;
        self.cell_input = None;
        self.col_drag = None;
        self.row_drag = None;
        self.col_resize = None;
        self.fill_drag = None;
        self.source_version = self.source_version.wrapping_add(1).max(1);
    }

    /// Push the current state onto the undo stack. Clears the redo
    /// stack — any future history is invalidated by a fresh mutation.
    /// Caps depth at MAX_DEPTH by dropping the oldest entry.
    pub(crate) fn push_undo(&mut self) {
        let snap = self.snapshot();
        self.pending_source = None;
        self.undo_stack.push(snap);
        if self.undo_stack.len() > MAX_DEPTH {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Pop the most recent undo snapshot without restoring it. Used by
    /// methods that push at the top but then discover the mutation was
    /// a no-op, so the undo entry would be spurious.
    pub(crate) fn pop_undo_no_restore(&mut self) {
        self.undo_stack.pop();
    }

    /// Restore the previous snapshot. Returns true when something was
    /// undone, false when the stack was empty.
    pub(crate) fn undo(&mut self) -> bool {
        let Some(snap) = self.undo_stack.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.restore(snap);
        self.redo_stack.push(current);
        if self.redo_stack.len() > MAX_DEPTH {
            self.redo_stack.remove(0);
        }
        true
    }

    /// Replay the most recently undone snapshot. Returns true when
    /// something was redone, false when the stack was empty.
    pub(crate) fn redo(&mut self) -> bool {
        let Some(snap) = self.redo_stack.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.restore(snap);
        self.undo_stack.push(current);
        if self.undo_stack.len() > MAX_DEPTH {
            self.undo_stack.remove(0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::csv_doc::model::CsvModel;
    use std::collections::BTreeSet;

    fn fresh_model(src: &str) -> CsvModel {
        let mut m = CsvModel::empty(b',');
        m.rebuild_from_source(src, 1);
        m.undo_stack.clear();
        m.redo_stack.clear();
        m
    }

    #[test]
    fn undo_restores_cell_value() {
        let mut m = fresh_model("a,b\n1,2\n3,4\n");
        assert!(m.set_cell(0, 0, "X".to_string()));
        assert_eq!(m.cell(0, 0), "X");
        assert!(m.undo());
        assert_eq!(m.cell(0, 0), "1");
    }

    #[test]
    fn redo_replays_cell_value() {
        let mut m = fresh_model("a,b\n1,2\n");
        m.set_cell(0, 0, "X".to_string());
        m.undo();
        assert_eq!(m.cell(0, 0), "1");
        assert!(m.redo());
        assert_eq!(m.cell(0, 0), "X");
    }

    #[test]
    fn undo_restores_deleted_rows() {
        let mut m = fresh_model("a,b\n1,2\n3,4\n5,6\n");
        let to_delete: BTreeSet<usize> = [1].into_iter().collect();
        m.delete_rows(&to_delete);
        assert_eq!(m.row_count(), 2);
        assert_eq!(m.cell(0, 0), "1");
        assert_eq!(m.cell(1, 0), "5");
        assert!(m.undo());
        assert_eq!(m.row_count(), 3);
        assert_eq!(m.cell(1, 0), "3");
    }

    #[test]
    fn undo_restores_deleted_columns() {
        let mut m = fresh_model("a,b,c\n1,2,3\n");
        let to_delete: BTreeSet<usize> = [1].into_iter().collect();
        m.delete_cols(&to_delete);
        assert_eq!(m.col_count(), 2);
        assert!(m.undo());
        assert_eq!(m.col_count(), 3);
        assert_eq!(m.header(1), "b");
    }

    #[test]
    fn undo_restores_inserted_row() {
        let mut m = fresh_model("a,b\n1,2\n3,4\n");
        m.insert_row_at(0, false);
        assert_eq!(m.row_count(), 3);
        assert!(m.undo());
        assert_eq!(m.row_count(), 2);
    }

    #[test]
    fn undo_restores_header_rename() {
        let mut m = fresh_model("a,b\n1,2\n");
        assert!(m.set_header(0, "NEW".to_string()));
        assert_eq!(m.header(0), "NEW");
        assert!(m.undo());
        assert_eq!(m.header(0), "a");
    }

    #[test]
    fn undo_restores_sort_order() {
        let mut m = fresh_model("n\n3\n1\n2\n");
        m.sort_by_col(0, true);
        assert_eq!(m.cell(0, 0), "1");
        assert_eq!(m.cell(1, 0), "2");
        assert_eq!(m.cell(2, 0), "3");
        assert!(m.undo());
        assert_eq!(m.cell(0, 0), "3");
        assert_eq!(m.cell(1, 0), "1");
        assert_eq!(m.cell(2, 0), "2");
    }

    #[test]
    fn undo_restores_col_width() {
        let mut m = fresh_model("a,b\n1,2\n");
        let original = m.col_width(0);
        m.push_undo();
        m.set_col_width(0, gpui::px(300.0));
        assert_eq!(f32::from(m.col_width(0)), 300.0);
        assert!(m.undo());
        assert_eq!(m.col_width(0), original);
    }

    #[test]
    fn undo_restores_row_drag_reorder() {
        let mut m = fresh_model("a\n1\n2\n3\n");
        let dragged = vec![0usize];
        m.move_row_block(&dragged, 0, 3);
        assert_eq!(m.cell(0, 0), "2");
        assert_eq!(m.cell(1, 0), "3");
        assert_eq!(m.cell(2, 0), "1");
        assert!(m.undo());
        assert_eq!(m.cell(0, 0), "1");
        assert_eq!(m.cell(1, 0), "2");
        assert_eq!(m.cell(2, 0), "3");
    }

    #[test]
    fn undo_restores_col_drag_reorder() {
        let mut m = fresh_model("a,b,c\n1,2,3\n");
        let dragged = vec![0usize];
        m.move_col_block(&dragged, 0, 3);
        assert_eq!(m.header(0), "b");
        assert_eq!(m.header(1), "c");
        assert_eq!(m.header(2), "a");
        assert!(m.undo());
        assert_eq!(m.header(0), "a");
        assert_eq!(m.header(1), "b");
        assert_eq!(m.header(2), "c");
    }

    #[test]
    fn undo_with_empty_stack_is_noop() {
        let mut m = fresh_model("a\n1\n");
        assert!(!m.undo());
    }

    #[test]
    fn redo_with_empty_stack_is_noop() {
        let mut m = fresh_model("a\n1\n");
        assert!(!m.redo());
    }

    #[test]
    fn undo_stack_caps_at_max_depth() {
        let mut m = fresh_model("a\n1\n");
        for i in 0..(MAX_DEPTH + 10) {
            m.set_cell(0, 0, format!("v{}", i));
        }
        assert_eq!(m.undo_stack.len(), MAX_DEPTH);
    }

    #[test]
    fn mutation_clears_redo_stack() {
        let mut m = fresh_model("a\n1\n");
        m.set_cell(0, 0, "X".to_string());
        m.undo();
        assert_eq!(m.redo_stack.len(), 1);
        m.set_cell(0, 0, "Y".to_string());
        assert_eq!(m.redo_stack.len(), 0);
    }

    #[test]
    fn selection_change_does_not_push_undo() {
        let mut m = fresh_model("a,b\n1,2\n3,4\n");
        assert_eq!(m.undo_stack.len(), 0);
        m.select_row_plain(0);
        m.select_row_toggle(1);
        m.select_row_range(2);
        m.select_col_plain(0);
        m.clear_axis_selections();
        assert_eq!(m.undo_stack.len(), 0);
    }

    #[test]
    fn active_cell_move_does_not_push_undo() {
        let mut m = fresh_model("a,b\n1,2\n");
        m.active_cell = Some((0, 0));
        m.active_cell = Some((0, 1));
        assert_eq!(m.undo_stack.len(), 0);
    }

    #[test]
    fn undo_cancels_in_flight_edit() {
        let mut m = fresh_model("a\n1\n");
        m.set_cell(0, 0, "X".to_string());
        m.editing_cell = Some((0, 0));
        assert!(m.undo());
        assert!(m.editing_cell.is_none());
        assert!(m.cell_input.is_none());
    }

    #[test]
    fn undo_bumps_source_version() {
        let mut m = fresh_model("a\n1\n");
        m.set_cell(0, 0, "X".to_string());
        let v = m.source_version;
        m.undo();
        assert!(m.source_version != v);
    }

    #[test]
    fn redo_after_undo_chains_correctly() {
        let mut m = fresh_model("a\n1\n");
        m.set_cell(0, 0, "X".to_string());
        m.set_cell(0, 0, "Y".to_string());
        m.set_cell(0, 0, "Z".to_string());
        assert_eq!(m.cell(0, 0), "Z");
        m.undo();
        assert_eq!(m.cell(0, 0), "Y");
        m.undo();
        assert_eq!(m.cell(0, 0), "X");
        m.redo();
        assert_eq!(m.cell(0, 0), "Y");
        m.redo();
        assert_eq!(m.cell(0, 0), "Z");
    }

    #[test]
    fn no_op_set_cell_does_not_push_undo() {
        let mut m = fresh_model("a\n1\n");
        // Setting a cell to its current value should not push undo.
        assert!(!m.set_cell(0, 0, "1".to_string()));
        assert_eq!(m.undo_stack.len(), 0);
    }

    #[test]
    fn no_op_set_header_does_not_push_undo() {
        let mut m = fresh_model("a,b\n1,2\n");
        assert!(!m.set_header(0, "a".to_string()));
        assert_eq!(m.undo_stack.len(), 0);
    }

    #[test]
    fn no_op_move_row_block_does_not_push_undo() {
        let mut m = fresh_model("a\n1\n2\n3\n");
        let dragged = vec![0usize];
        // Dropping at index 0 keeps row 0 at position 0 — no-op.
        m.move_row_block(&dragged, 0, 0);
        assert_eq!(m.undo_stack.len(), 0);
    }

    #[test]
    fn no_op_sort_does_not_push_undo() {
        let mut m = fresh_model("n\n1\n2\n3\n");
        // Already sorted ascending.
        m.sort_by_col(0, true);
        assert_eq!(m.undo_stack.len(), 0);
    }
}
