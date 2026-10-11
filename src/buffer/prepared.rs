// Immutable prefix edits. Only this builder can create a fast replay entry.
use super::{ContentRevision, EditOp, Selection};
use ropey::Rope;
use std::{collections::BTreeSet, ops::Range, sync::{Arc, atomic::{AtomicBool, Ordering}}};

const MAX_HISTORY_BYTES: usize = 128 * 1024 * 1024;
const MEMORY_ERROR: &str = "Not enough memory to change line indentation";

pub(crate) fn history_record_budget(count: usize, plan_capacity: usize) -> Result<usize, &'static str> {
    let cost = count.checked_mul(std::mem::size_of::<EditOp>() + 2 * std::mem::size_of::<Offset>() + 64)
        .and_then(|n| plan_capacity.checked_mul(std::mem::size_of::<PrefixEdit>()).and_then(|plan| n.checked_add(plan)))
        .ok_or(MEMORY_ERROR)?;
    if cost > MAX_HISTORY_BYTES { return Err("Select fewer lines to keep indentation history within 128 MiB"); }
    Ok(cost)
}

pub(crate) struct PrefixEdit {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

#[derive(Clone)]
struct Offset {
    start: usize,
    end: usize,
    removed: usize,
    inserted: usize,
}

#[derive(Clone)]
struct PrefixMap(Vec<Offset>);

impl PrefixMap {
    fn position(&self, position: usize, count: usize) -> usize {
        let Some(offset) = count.checked_sub(1).and_then(|i| self.0.get(i)) else { return position; };
        position.saturating_sub(offset.removed).saturating_add(offset.inserted)
    }

    fn metadata(&self, source: &Rope, target: &Rope, metadata: &Metadata, first: Option<Range<usize>>, cancel: &AtomicBool) -> Result<Metadata, &'static str> {
        let mut bookmarks = BTreeSet::new();
        for &bookmark in &metadata.bookmarks {
            if cancel.load(Ordering::Relaxed) { return Err("Line indentation cancelled"); }
            let bookmark = bookmark.min(source.len_bytes());
            if first.as_ref().is_some_and(|r| bookmark > r.start && bookmark < r.end) { continue; }
            // Prefixes contain no line breaks. After the first replacement,
            // legacy replay normalizes every surviving bookmark to its row.
            let row = source.byte_to_line(bookmark);
            bookmarks.insert(target.line_to_byte(row.min(target.len_lines().saturating_sub(1))));
        }
        let mut marks = Vec::new();
        marks.try_reserve_exact(metadata.marks.len()).map_err(|_| MEMORY_ERROR)?;
        for range in &metadata.marks {
            if cancel.load(Ordering::Relaxed) { return Err("Line indentation cancelled"); }
            if range.start >= range.end || super::char_boundary_range(source, range.clone()) != *range { continue; }
            let count = self.0.partition_point(|edit| edit.end <= range.start);
            if self.0.get(count).is_some_and(|edit| edit.start < range.end) { continue; }
            let mapped = self.position(range.start, count)..self.position(range.end, count);
            if super::char_boundary_range(target, mapped.clone()) == mapped { marks.push(mapped); }
        }
        let marker = metadata.marker.filter(|&marker| marker <= source.len_bytes()
            && super::floor_char_boundary(source, marker) == marker).and_then(|marker| {
            let count = self.0.partition_point(|edit| edit.end < marker);
            if self.0.get(count).is_some_and(|edit| edit.start <= marker) { None }
            else {
                let mapped = self.position(marker, count);
                (super::floor_char_boundary(target, mapped) == mapped).then_some(mapped)
            }
        });
        Ok(Metadata { bookmarks, marks, marker })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub(crate) bookmarks: BTreeSet<usize>,
    pub(crate) marks: Vec<Range<usize>>,
    pub(crate) marker: Option<usize>,
}

pub(crate) struct PreparedGroup {
    body: Arc<PreparedBody>,
    before_revision: ContentRevision,
    after_revision: ContentRevision,
}

// Endpoint identities can be rebound after session/CSV restoration. The
// document trees, maps and legacy codec data remain sealed and shared.
struct PreparedBody {
    before: Rope,
    after: Rope,
    before_selection: Selection,
    after_selection: Selection,
    ops: Vec<EditOp>,
    forward: PrefixMap,
    backward: PrefixMap,
}

impl Clone for PreparedGroup {
    fn clone(&self) -> Self {
        Self { body: self.body.clone(), before_revision: self.before_revision.clone(), after_revision: self.after_revision.clone() }
    }
}

impl PreparedGroup {
    pub(crate) fn build(before: Rope, revision: ContentRevision, before_selection: Selection,
        after_selection: Selection, edits: Vec<PrefixEdit>, cancel: &AtomicBool) -> Result<Self, &'static str>
    {
        let mut cost = history_record_budget(edits.len(), edits.capacity())?;
        let mut forward = Vec::new();
        let mut backward = Vec::new();
        let mut ops = Vec::new();
        forward.try_reserve_exact(edits.len()).map_err(|_| MEMORY_ERROR)?;
        backward.try_reserve_exact(edits.len()).map_err(|_| MEMORY_ERROR)?;
        ops.try_reserve_exact(edits.len()).map_err(|_| MEMORY_ERROR)?;
        let (mut removed_total, mut inserted_total, mut previous_end) = (0usize, 0usize, 0usize);
        for edit in &edits {
            if cancel.load(Ordering::Relaxed) { return Err("Line indentation cancelled"); }
            let range = &edit.range;
            if (forward.last().is_some() && range.start <= previous_end) || super::char_boundary_range(&before, range.clone()) != *range
                || !edit.text.bytes().all(|b| matches!(b, b' ' | b'\t'))
            { return Err("Line indentation plan no longer matches the document"); }
            let Some(prefix) = before.get_byte_slice(range.clone()) else { return Err("Invalid indentation range"); };
            if !prefix.bytes().all(|b| matches!(b, b' ' | b'\t'))
                || (range.start > 0 && !matches!(before.get_byte(range.start - 1), Some(b'\r' | b'\n')))
            { return Err("Invalid indentation prefix"); }
            cost = cost.checked_add(range.len()).and_then(|n| n.checked_add(edit.text.len())).ok_or(MEMORY_ERROR)?;
            if cost > MAX_HISTORY_BYTES { return Err("Select fewer lines to keep indentation history within 128 MiB"); }
            let new_start = range.start.checked_sub(removed_total).and_then(|n| n.checked_add(inserted_total)).ok_or(MEMORY_ERROR)?;
            let new_end = new_start.checked_add(edit.text.len()).ok_or(MEMORY_ERROR)?;
            removed_total = removed_total.checked_add(range.len()).ok_or(MEMORY_ERROR)?;
            inserted_total = inserted_total.checked_add(edit.text.len()).ok_or(MEMORY_ERROR)?;
            forward.push(Offset { start: range.start, end: range.end, removed: removed_total, inserted: inserted_total });
            backward.push(Offset { start: new_start, end: new_end, removed: inserted_total, inserted: removed_total });
            previous_end = range.end;
        }
        let before_selection = before_selection.clamped(&before);
        let mut after = before.clone();
        let mut current_revision = revision.clone();
        let mut selection = before_selection;
        for edit in edits.into_iter().rev() {
            if cancel.load(Ordering::Relaxed) { return Err("Line indentation cancelled"); }
            let mut removed = String::new();
            removed.try_reserve_exact(edit.range.len()).map_err(|_| MEMORY_ERROR)?;
            let Some(slice) = after.get_byte_slice(edit.range.clone()) else { return Err("Invalid indentation range"); };
            for chunk in slice.chunks() { removed.push_str(chunk); }
            let start = after.byte_to_char(edit.range.start);
            let end = after.byte_to_char(edit.range.end);
            after.remove(start..end);
            after.insert(start, &edit.text);
            let next = ContentRevision::default();
            ops.push(EditOp { before_revision: current_revision, after_revision: next.clone(),
                byte_start: edit.range.start, removed, inserted: edit.text,
                sel_before_anchor: selection.anchor, sel_before_head: selection.head });
            current_revision = next;
            if let Some(op) = ops.last() { selection.collapse_to(op.byte_start.saturating_add(op.inserted.len())); }
        }
        Ok(Self { before_revision: revision, after_revision: current_revision,
            body: Arc::new(PreparedBody { before, after: after.clone(),
            before_selection, after_selection: after_selection.clamped(&after), ops,
            forward: PrefixMap(forward), backward: PrefixMap(backward) }) })
    }

    pub(crate) fn ops(&self) -> &[EditOp] { &self.body.ops }
    pub(crate) fn before_selection(&self) -> Selection { self.body.before_selection }
    pub(crate) fn expected(&self, undo: bool) -> &ContentRevision { if undo { &self.after_revision } else { &self.before_revision } }
    pub(crate) fn expected_rope(&self, undo: bool) -> &Rope { if undo { &self.body.after } else { &self.body.before } }
    pub(crate) fn matches_rope(&self, rope: &Rope, undo: bool, cancel: &AtomicBool) -> Result<bool, &'static str> {
        if cancel.load(Ordering::Relaxed) { return Err("Line indentation cancelled"); }
        let expected = self.expected_rope(undo);
        if rope.is_instance(expected) { return Ok(true); }
        if rope.len_bytes() != expected.len_bytes() { return Ok(false); }
        // Single Undo can reconstruct an endpoint with different tree nodes.
        // Prove the bytes on the worker, polling between bounded Rope chunks.
        let (mut left, mut right) = (rope.chunks(), expected.chunks());
        let (mut a, mut b): (&[u8], &[u8]) = (&[], &[]);
        loop {
            if cancel.load(Ordering::Relaxed) { return Err("Line indentation cancelled"); }
            if a.is_empty() { a = left.next().map(str::as_bytes).unwrap_or_default(); }
            if b.is_empty() { b = right.next().map(str::as_bytes).unwrap_or_default(); }
            if a.is_empty() || b.is_empty() { return Ok(a.is_empty() && b.is_empty()); }
            let count = a.len().min(b.len());
            if a.get(..count) != b.get(..count) { return Ok(false); }
            a = a.get(count..).unwrap_or_default();
            b = b.get(count..).unwrap_or_default();
        }
    }
    pub(crate) fn target(&self, undo: bool) -> (Rope, ContentRevision, Selection) {
        if undo { (self.body.before.clone(), self.before_revision.clone(), self.body.before_selection) }
        else { (self.body.after.clone(), self.after_revision.clone(), self.body.after_selection) }
    }
    pub(crate) fn remap(&self, metadata: &Metadata, undo: bool, cancel: &AtomicBool) -> Result<Metadata, &'static str> {
        let (map, source, target, first) = if undo {
            (&self.body.backward, &self.body.after, &self.body.before, self.body.backward.0.first())
        } else { (&self.body.forward, &self.body.before, &self.body.after, self.body.forward.0.last()) };
        map.metadata(source, target, metadata, first.map(|offset| offset.start..offset.end), cancel)
    }
    pub(crate) fn rebind(&mut self, current: &mut ContentRevision) {
        self.after_revision = current.clone();
        if !self.body.ops.is_empty() { *current = ContentRevision::default(); }
        self.before_revision = current.clone();
    }
    pub(crate) fn rebind_after(&mut self, revision: ContentRevision) {
        self.after_revision = revision;
    }
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
