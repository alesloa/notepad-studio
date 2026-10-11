// Bounded exact capture: no file text or unbounded history is cloned on the GUI thread.
use std::{path::PathBuf, sync::Arc, io::Write};
use crate::{buffer::{ContentRevision, SavePoint, UndoEntry, EditOp}, tab::{Tab, TabViewMode}};
const MAX_NODES: usize = 65_536;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const LIMIT: &str = "Reload cannot capture more than 8 MiB or 65,536 history/annotation records; save your work and reopen the file";
const MEMORY: &str = "Not enough memory to prepare Reload";

pub(super) struct Guard {
    instance: Arc<()>, activity: ContentRevision, rope: ropey::Rope, revision: ContentRevision, path: Option<PathBuf>,
    saved: Option<SavePoint>, dirty: bool, encoding: crate::text::encoding::Encoding,
    eol: crate::text::line_ending::LineEnding, language: Option<crate::syntax::LanguageId>,
    read_only: bool, color: Option<u8>, changelog: bool, view: TabViewMode,
    bookmarks: Vec<usize>, marks: Vec<std::ops::Range<usize>>, marker: Option<usize>,
    undo: Vec<UndoEntry>, redo: Vec<UndoEntry>, rich: Option<Rich>,
    remaining_nodes: usize, remaining_bytes: usize,
}
struct Budget { nodes: usize, bytes: usize }
impl Budget {
    fn add(&mut self, nodes: usize, bytes: usize) -> Result<(), &'static str> {
        self.nodes = self.nodes.checked_add(nodes).filter(|n| *n <= MAX_NODES).ok_or(LIMIT)?;
        self.bytes = self.bytes.checked_add(bytes).filter(|n| *n <= MAX_BYTES).ok_or(LIMIT)?; Ok(())
    }
}
fn scan(stack: &[UndoEntry], budget: &mut Budget) -> Result<(), &'static str> {
    budget.add(stack.len(), stack.len().checked_mul(std::mem::size_of::<UndoEntry>()).ok_or(LIMIT)?)?;
    for entry in stack {
        let ops = match entry { UndoEntry::Single(op) => std::slice::from_ref(op),
            UndoEntry::Group { ops, .. } => ops, UndoEntry::Prepared(_) => continue };
        budget.add(ops.len(), ops.len().checked_mul(std::mem::size_of::<EditOp>()).ok_or(LIMIT)?)?;
        for op in ops { budget.add(0, op.removed.len().checked_add(op.inserted.len()).ok_or(LIMIT)?)?; }
    }
    Ok(())
}
fn copy_text(text: &str) -> Result<String, &'static str> {
    let mut result = String::new(); result.try_reserve_exact(text.len()).map_err(|_| MEMORY)?;
    result.push_str(text); Ok(result)
}
fn copy_op(op: &EditOp) -> Result<EditOp, &'static str> {
    Ok(EditOp { before_revision: op.before_revision.clone(), after_revision: op.after_revision.clone(), byte_start: op.byte_start,
        removed: copy_text(&op.removed)?, inserted: copy_text(&op.inserted)?, sel_before_anchor: op.sel_before_anchor, sel_before_head: op.sel_before_head })
}
fn copy_stack(stack: &[UndoEntry]) -> Result<Vec<UndoEntry>, &'static str> {
    let mut result = Vec::new(); result.try_reserve_exact(stack.len()).map_err(|_| MEMORY)?;
    for entry in stack {
        result.push(match entry {
            UndoEntry::Single(op) => UndoEntry::Single(copy_op(op)?),
            UndoEntry::Prepared(group) => UndoEntry::Prepared(group.clone()),
            UndoEntry::Group { ops, sel_before_anchor, sel_before_head } => {
                let mut copied = Vec::new(); copied.try_reserve_exact(ops.len()).map_err(|_| MEMORY)?;
                for op in ops { copied.push(copy_op(op)?); }
                UndoEntry::Group { ops: copied, sel_before_anchor: *sel_before_anchor, sel_before_head: *sel_before_head }
            }
        });
    }
    Ok(result)
}
fn op_equal(a: &EditOp, b: &EditOp) -> bool {
    a.before_revision == b.before_revision && a.after_revision == b.after_revision && a.byte_start == b.byte_start
        && a.removed == b.removed && a.inserted == b.inserted && a.sel_before_anchor == b.sel_before_anchor && a.sel_before_head == b.sel_before_head
}
fn stacks_equal(a: &[UndoEntry], b: &[UndoEntry]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a,b)| match (a,b) {
        (UndoEntry::Single(a), UndoEntry::Single(b)) => op_equal(a,b),
        (UndoEntry::Prepared(a), UndoEntry::Prepared(b)) => Arc::ptr_eq(a,b),
        (UndoEntry::Group { ops:a, sel_before_anchor:aa, sel_before_head:ah }, UndoEntry::Group { ops:b, sel_before_anchor:ba, sel_before_head:bh }) =>
            aa == ba && ah == bh && a.len() == b.len() && a.iter().zip(b).all(|(a,b)| op_equal(a,b)), _ => false
    })
}
struct Rich { original: Arc<Vec<u8>>, revision: ContentRevision, history: Vec<ContentRevision>, semantic: Vec<u8> }
pub(super) struct Capped { pub(super) bytes: Vec<u8>, pub(super) maximum: usize }
impl Write for Capped {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let length = self.bytes.len().checked_add(input.len()).filter(|n| *n <= self.maximum)
            .ok_or_else(|| std::io::Error::other(LIMIT))?;
        if length > self.bytes.capacity() {
            let capacity = length.checked_next_power_of_two().unwrap_or(self.maximum).min(self.maximum);
            self.bytes.try_reserve_exact(capacity.saturating_sub(self.bytes.len())).map_err(|_| std::io::Error::other(MEMORY))?;
        }
        self.bytes.extend_from_slice(input); Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
fn rich_bytes(payload: &crate::rich_doc::RichDocPayload, maximum: usize) -> Result<Vec<u8>, &'static str> {
    let mut writer = Capped { bytes: Vec::new(), maximum };
    // Revision tokens are captured separately; every semantic undo/redo field is encoded here.
    serde_json::to_writer(&mut writer, &(&payload.format, &payload.ast, &payload.raw_view_text, &payload.edit_log,
        &payload.headers, &payload.footers, payload.edit_group_start)).map_err(|_| LIMIT)?;
    for stack in [&payload.undo_stack, &payload.redo_stack] {
        for snap in stack { serde_json::to_writer(&mut writer, &(&snap.ast, snap.selection.map(|s| (s.anchor.block,s.anchor.run,s.anchor.byte,s.cursor.block,s.cursor.run,s.cursor.byte)), snap.active_block, &snap.edit_log)).map_err(|_| LIMIT)?; }
        writer.write_all(b"|").map_err(|_| LIMIT)?;
    }
    Ok(writer.bytes)
}
fn budget(tab: &Tab) -> Result<Budget, &'static str> {
    if tab.edit_transaction.is_some() { return Err("Finish the current edit transaction before reloading"); }
    let buffer = &tab.buffer;
    let mut budget = Budget { nodes: 0, bytes: 0 };
    let annotations = buffer.bookmarks.len().checked_add(buffer.marks.len()).ok_or(LIMIT)?;
    budget.add(annotations, annotations.checked_mul(2 * std::mem::size_of::<usize>()).ok_or(LIMIT)?)?;
    scan(&tab.undo_stack, &mut budget)?; scan(&tab.redo_stack, &mut budget)?;
    if let Some(payload) = &buffer.rich_doc {
        let count = payload.undo_stack.len().checked_add(payload.redo_stack.len()).ok_or(LIMIT)?;
        budget.add(count, count.checked_mul(std::mem::size_of::<ContentRevision>()).ok_or(LIMIT)?)?;
    }
    Ok(budget)
}
impl Guard {
    pub(super) fn capture(tab: &Tab) -> Result<Self, &'static str> {
        let budget = budget(tab)?; let buffer = &tab.buffer;
        let mut bookmarks = Vec::new(); bookmarks.try_reserve_exact(buffer.bookmarks.len()).map_err(|_| MEMORY)?;
        bookmarks.extend(buffer.bookmarks.iter().copied());
        let mut marks = Vec::new(); marks.try_reserve_exact(buffer.marks.len()).map_err(|_| MEMORY)?;
        marks.extend(buffer.marks.iter().cloned());
        let rich = if let Some(payload) = &buffer.rich_doc {
            let semantic = rich_bytes(payload, MAX_BYTES.saturating_sub(budget.bytes))?;
            let mut history = Vec::new(); history.try_reserve_exact(payload.undo_stack.len() + payload.redo_stack.len()).map_err(|_| MEMORY)?;
            history.extend(payload.undo_stack.iter().chain(&payload.redo_stack).map(|s| s.revision.clone()));
            Some(Rich { original: payload.original_bytes.clone(), revision: payload.revision.clone(), history, semantic })
        } else { None };
        let used_rich = rich.as_ref().map_or(0, |r| r.semantic.capacity());
        Ok(Self { instance: tab.instance.clone(), activity: tab.content_activity.clone(), remaining_nodes: MAX_NODES.saturating_sub(budget.nodes), remaining_bytes: MAX_BYTES.saturating_sub(budget.bytes).saturating_sub(used_rich), rope: buffer.rope.clone(), revision: buffer.raw_revision.clone(), path: buffer.file_path.clone(),
            saved: buffer.savepoint.clone(), dirty: buffer.dirty, encoding: buffer.encoding, eol: buffer.line_ending,
            language: buffer.language, read_only: buffer.read_only, color: buffer.tab_color, changelog: buffer.is_changelog,
            view: tab.view_mode, bookmarks, marks, marker: tab.begin_select_marker,
            undo: copy_stack(&tab.undo_stack)?, redo: copy_stack(&tab.redo_stack)?, rich })
    }
    pub(super) fn remaining(&self) -> (usize, usize) { (self.remaining_nodes, self.remaining_bytes) }
    pub(super) fn valid(&self, tab: &Tab) -> bool {
        let b = &tab.buffer;
        if !Arc::ptr_eq(&self.instance, &tab.instance) || self.activity != tab.content_activity || b.loading.is_some() || tab.csv_draft.is_some() || tab.edit_transaction.is_some()
            || !b.rope.is_instance(&self.rope) || b.raw_revision != self.revision || b.file_path != self.path
            || b.savepoint != self.saved || b.dirty != self.dirty || b.encoding != self.encoding || b.line_ending != self.eol
            || b.language != self.language || b.read_only != self.read_only || b.tab_color != self.color || b.is_changelog != self.changelog
            || tab.view_mode != self.view || tab.begin_select_marker != self.marker { return false; }
        let Ok(budget) = budget(tab) else { return false; };
        if !b.bookmarks.iter().copied().eq(self.bookmarks.iter().copied()) || b.marks != self.marks
            || !stacks_equal(&tab.undo_stack, &self.undo) || !stacks_equal(&tab.redo_stack, &self.redo) { return false; }
        match (&self.rich, &b.rich_doc) {
            (None, None) => true,
            (Some(saved), Some(payload)) => Arc::ptr_eq(&saved.original, &payload.original_bytes) && saved.revision == payload.revision
                && payload.undo_stack.iter().chain(&payload.redo_stack).map(|s| &s.revision).eq(saved.history.iter())
                && rich_bytes(payload, MAX_BYTES.saturating_sub(budget.bytes)).is_ok_and(|bytes| bytes == saved.semantic),
            _ => false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_guard_combined_rich_and_csv_storage_charges_reserved_capacity() {
        let fixture = crate::persist::TempDir::new("reload-combined-capacity"); let path = fixture.join("neutral.rtf");
        std::fs::write(&path,format!("{{\\rtf1 {}}}","x".repeat(4000))).expect("fixture");
        let mut tab = Tab::empty(); tab.buffer = crate::io::read_buffer(&path).expect("rich fixture");
        let guard = Guard::capture(&tab).expect("guard"); let rich = guard.rich.as_ref().expect("rich capture");
        assert!(rich.semantic.capacity() > rich.semantic.len() + 1024);
        let budget = budget(&tab).expect("budget"); let (nodes,remaining) = guard.remaining();
        let source_bytes = 2*std::mem::size_of::<Option<(ContentRevision,ropey::Rope)>>();
        let mut model = crate::ui::csv_doc::CsvModel::empty(b','); model.has_header = false;
        model.rows = vec![vec!["x".repeat(MAX_BYTES-budget.bytes-rich.semantic.len()-source_bytes-1024)]];
        assert!(super::super::reload_csv::Guard::capture(&model,nodes,remaining).is_err());
        model.rows = vec![vec!["x".repeat(remaining-source_bytes-1024)]];
        let csv = super::super::reload_csv::Guard::capture(&model,nodes,remaining).expect("bounded combined capture");
        assert!(budget.bytes+rich.semantic.capacity()+csv.retained_capacity() <= MAX_BYTES);
    }
}
