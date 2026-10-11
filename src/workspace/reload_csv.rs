// Exact bounded grid/history capture, without UI focus or scroll handles.
use std::io::Write;
use crate::{buffer::ContentRevision, ui::csv_doc::{CsvModel, undo::{CsvSnapshot, CsvSourceState}}};
use super::reload_guard::Capped;
const LIMIT: &str = "Reload cannot capture this CSV history within the 8 MiB / 65,536-record limit; save your work and reopen the file";
pub(super) struct Guard { version: u64, bytes: Vec<u8>, sources: Vec<Option<(ContentRevision, ropey::Rope)>>, nodes: usize, maximum: usize }
fn source(value: &Option<CsvSourceState>) -> Option<(ContentRevision, ropey::Rope)> { value.as_ref().map(|s| (s.revision.clone(), s.rope.clone())) }
fn source_equal(a: &Option<CsvSourceState>, b: &Option<(ContentRevision, ropey::Rope)>) -> bool {
    match (a,b) { (None,None) => true, (Some(a),Some((rev,rope))) => &a.revision == rev && a.rope.is_instance(rope), _ => false }
}
fn count(rows: &[Vec<String>], headers: &[String], widths: usize, cols: usize, order: usize, nodes: &mut usize) -> Result<(), &'static str> {
    let n = rows.len().checked_add(headers.len()).and_then(|n| n.checked_add(widths)).and_then(|n| n.checked_add(cols)).and_then(|n| n.checked_add(order)).ok_or(LIMIT)?;
    *nodes = nodes.checked_sub(n).ok_or(LIMIT)?;
    for row in rows { *nodes = nodes.checked_sub(row.len()).ok_or(LIMIT)?; }
    Ok(())
}
fn encode(model: &CsvModel, maximum: usize, mut nodes: usize) -> Result<Vec<u8>, &'static str> {
    let source_count = model.undo_stack.len().checked_add(model.redo_stack.len()).and_then(|n| n.checked_add(2)).ok_or(LIMIT)?;
    let source_bytes = source_count.checked_mul(std::mem::size_of::<Option<(ContentRevision,ropey::Rope)>>()).ok_or(LIMIT)?;
    let filter_bytes = model.col_filters.len().checked_mul(2*std::mem::size_of::<usize>())
        .and_then(|n| model.col_filters.values().map(|set| set.len()).max().unwrap_or(0).checked_mul(std::mem::size_of::<&String>()).and_then(|m| n.checked_add(m))).ok_or(LIMIT)?;
    let maximum = maximum.checked_sub(source_bytes.checked_add(filter_bytes).ok_or(LIMIT)?).ok_or(LIMIT)?;
    count(&model.rows, &model.headers, model.col_widths.len(), model.col_order.len(), model.row_order.len(), &mut nodes)?;
    nodes = nodes.checked_sub(model.find_matches.len()).ok_or(LIMIT)?;
    nodes = nodes.checked_sub(model.col_filters.len()).ok_or(LIMIT)?;
    for hidden in model.col_filters.values() { nodes = nodes.checked_sub(hidden.len()).ok_or(LIMIT)?; }
    nodes = nodes.checked_sub(model.undo_stack.len().checked_add(model.redo_stack.len()).ok_or(LIMIT)?).ok_or(LIMIT)?;
    for snap in model.undo_stack.iter().chain(&model.redo_stack) {
        count(&snap.rows, &snap.headers, snap.col_widths.len(), snap.col_order.len(), snap.row_order.len(), &mut nodes)?;
        nodes = nodes.checked_sub(snap.selected_rows.len().checked_add(snap.selected_cols.len()).ok_or(LIMIT)?).ok_or(LIMIT)?;
    }
    let mut writer = Capped { bytes: Vec::new(), maximum };
    serde_json::to_writer(&mut writer, &(&model.rows, &model.headers, &model.col_order, &model.row_order,
        model.has_header, model.delimiter, model.trailing_newline, model.has_bom, model.reparse_requested)).map_err(|_| LIMIT)?;
    serde_json::to_writer(&mut writer, &(model.line_terminator.bytes(), &model.find_query, model.find_case_sensitive,
        model.find_whole_cell, &model.find_matches, model.find_current_match, model.find_bar_open,
        model.find_show_replace, &model.replace_text, &model.replace_summary, model.filters_visible,
        model.find_input.as_ref().map(|input| &input.buffer), model.replace_input.as_ref().map(|input| &input.buffer))).map_err(|_| LIMIT)?;
    // Hash iteration order is irrelevant; sorted references preserve exact filter contents.
    let mut columns = Vec::new(); columns.try_reserve_exact(model.col_filters.len()).map_err(|_| LIMIT)?;
    columns.extend(model.col_filters.iter()); columns.sort_unstable_by_key(|(column,_)| **column);
    for (column,hidden) in columns {
        let mut values = Vec::new(); values.try_reserve_exact(hidden.len()).map_err(|_| LIMIT)?;
        values.extend(hidden.iter()); values.sort_unstable();
        serde_json::to_writer(&mut writer, &(column,values)).map_err(|_| LIMIT)?;
    }
    for width in &model.col_widths { serde_json::to_writer(&mut writer, &f32::from(*width).to_bits()).map_err(|_| LIMIT)?; writer.write_all(b",").map_err(|_| LIMIT)?; }
    for stack in [&model.undo_stack, &model.redo_stack] {
        for snap in stack { snapshot(snap, &mut writer)?; }
        writer.write_all(b"|").map_err(|_| LIMIT)?;
    }
    Ok(writer.bytes)
}
fn snapshot(snap: &CsvSnapshot, writer: &mut Capped) -> Result<(), &'static str> {
    let range = snap.cell_range.map(|r| (r.anchor,r.focus));
    serde_json::to_writer(&mut *writer, &(&snap.rows, &snap.headers, &snap.col_order, &snap.row_order,
        snap.active_cell, &snap.selected_rows, &snap.selected_cols, range, snap.last_clicked_row, snap.last_clicked_col,
        snap.has_header, snap.delimiter)).map_err(|_| LIMIT)?;
    for width in &snap.col_widths { serde_json::to_writer(&mut *writer, &f32::from(*width).to_bits()).map_err(|_| LIMIT)?; writer.write_all(b",").map_err(|_| LIMIT)?; }
    writer.write_all(b";").map_err(|_| LIMIT)?; Ok(())
}
impl Guard {
    #[cfg(test)] pub(super) fn retained_capacity(&self) -> usize { self.bytes.capacity()+self.sources.capacity()*std::mem::size_of::<Option<(ContentRevision,ropey::Rope)>>() }
    pub(super) fn capture(model: &CsvModel, nodes: usize, maximum: usize) -> Result<Self, &'static str> {
        let bytes = encode(model, maximum, nodes)?;
        let count = model.undo_stack.len().checked_add(model.redo_stack.len()).and_then(|n| n.checked_add(2)).ok_or(LIMIT)?;
        let mut sources = Vec::new(); sources.try_reserve_exact(count).map_err(|_| "Not enough memory to capture CSV history for Reload")?;
        sources.push(source(&model.source_state)); sources.push(source(&model.pending_source));
        sources.extend(model.undo_stack.iter().chain(&model.redo_stack).map(|s| source(&s.source_state)));
        Ok(Self { version: model.source_version, bytes, sources, nodes, maximum })
    }
    pub(super) fn valid(&self, model: &CsvModel) -> bool {
        if self.version != model.source_version || self.sources.len() != model.undo_stack.len().saturating_add(model.redo_stack.len()).saturating_add(2) { return false; }
        let current = std::iter::once(&model.source_state).chain(std::iter::once(&model.pending_source))
            .chain(model.undo_stack.iter().chain(&model.redo_stack).map(|s| &s.source_state));
        current.zip(&self.sources).all(|(a,b)| source_equal(a,b))
            && encode(model, self.maximum, self.nodes).is_ok_and(|bytes| bytes == self.bytes)
    }
}
