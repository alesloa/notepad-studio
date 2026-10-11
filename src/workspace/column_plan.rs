// Scintilla Editor.cxx:2336-2347 guards Cut/Paste before side effects.
// Bounded synchronous plans keep the existing column input/IME funnel.
use std::{mem::size_of, ops::Range};
use ropey::{Rope, RopeSlice};
use crate::buffer::{BlockSelection, EditOp, Selection};

pub(crate) const MAX_WORK: usize = 128 * 1024;
pub(crate) const MAX_ROWS: usize = 4096;
pub(crate) const MAX_ALLOCATION: usize = 1024 * 1024;
pub(crate) const LIMIT: &str = "Column operation exceeds the temporary 128 KiB / 4096-row limit; select a smaller rectangle";
pub(crate) const ALLOCATION_LIMIT: &str = "Column plan exceeds the 1 MiB allocation limit; reduce the rectangle or finish the current edit group";
const MEMORY: &str = "Not enough memory to prepare the column operation; no changes applied";
const GEOMETRY: &str = "Column selection is outside the current document; select the rectangle again";

pub(crate) struct Row<'a> { pub(crate) start: usize, pub(crate) body: RopeSlice<'a> }
impl Row<'_> {
    pub(crate) fn byte(&self, column: usize) -> Result<usize, &'static str> {
        self.start.checked_add(self.body.try_char_to_byte(column.min(self.body.len_chars())).map_err(|_| GEOMETRY)?).ok_or(GEOMETRY)
    }
}
pub(crate) fn row(rope: &Rope, index: usize) -> Result<Row<'_>, &'static str> {
    let line = rope.get_line(index).ok_or(GEOMETRY)?;
    let start = rope.try_line_to_byte(index).map_err(|_| GEOMETRY)?;
    let mut end = line.len_bytes();
    while end > 0 && matches!(line.get_byte(end - 1), Some(b'\r' | b'\n')) { end -= 1; }
    Ok(Row { start, body: line.get_byte_slice(..end).ok_or(GEOMETRY)? })
}
pub(crate) fn geometry(rope: &Rope, block: &BlockSelection) -> Result<usize, &'static str> {
    if block.anchor_line >= rope.len_lines() || block.head_line >= rope.len_lines() { return Err(GEOMETRY); }
    let height = block.max_line().checked_sub(block.min_line()).and_then(|n| n.checked_add(1)).ok_or(GEOMETRY)?;
    if height > MAX_ROWS { return Err(LIMIT); }
    Ok(height)
}

struct Budget { work: usize, allocation: usize }
impl Budget {
    fn new(extra: usize) -> Result<Self, &'static str> {
        let mut value = Self { work: 0, allocation: 0 }; value.allocate(extra)?; Ok(value)
    }
    fn work(&mut self, bytes: usize) -> Result<(), &'static str> {
        self.work = self.work.checked_add(bytes).ok_or(LIMIT)?;
        if self.work > MAX_WORK { return Err(LIMIT); } Ok(())
    }
    fn allocate(&mut self, bytes: usize) -> Result<(), &'static str> {
        self.allocation = self.allocation.checked_add(bytes).ok_or(ALLOCATION_LIMIT)?;
        if self.allocation > MAX_ALLOCATION { return Err(ALLOCATION_LIMIT); } Ok(())
    }
    fn rows<T>(&mut self, count: usize) -> Result<(), &'static str> {
        self.allocate(count.checked_mul(size_of::<T>()).ok_or(ALLOCATION_LIMIT)?)
    }
    fn text(&mut self, bytes: usize) -> Result<(), &'static str> { self.work(bytes)?; self.allocate(bytes) }
}
struct Span { range: Range<usize>, padding: usize }
pub(crate) struct Edit { pub(crate) range: Range<usize>, pub(crate) text: String }
pub(crate) struct Plan {
    pub(crate) edits: Vec<Edit>,
    pub(crate) block: Option<BlockSelection>,
    pub(crate) head: (usize, usize),
    pub(crate) update_geometry: bool,
}
pub(crate) struct ClipboardPlan { pub(crate) history: String, pub(crate) payload: String, pub(crate) cut: Option<Plan> }

fn reserve<T>(count: usize) -> Result<Vec<T>, &'static str> {
    let mut value = Vec::new(); value.try_reserve_exact(count).map_err(|_| MEMORY)?; Ok(value)
}
fn string(capacity: usize) -> Result<String, &'static str> {
    let mut value = String::new(); value.try_reserve_exact(capacity).map_err(|_| MEMORY)?; Ok(value)
}
fn thin(block: BlockSelection, column: usize) -> Option<BlockSelection> {
    let after = BlockSelection { anchor_col: column, head_col: column, is_dragging: false, ..block };
    (!after.is_empty()).then_some(after)
}
fn edit_budget(budget: &mut Budget, count: usize) -> Result<(), &'static str> {
    budget.rows::<Edit>(count)?; budget.rows::<[u8; 64]>(count)?; budget.rows::<EditOp>(count)
}
fn edit_work(budget: &mut Budget, range: &Range<usize>, inserted: usize) -> Result<(), &'static str> {
    // Payload and legacy inserted-history copy, plus removed-history copy.
    budget.text(inserted.checked_mul(2).and_then(|n| n.checked_add(range.end.saturating_sub(range.start))).ok_or(LIMIT)?)
}
fn payload(padding: usize, text: &str) -> Result<String, &'static str> {
    let mut value = string(padding.checked_add(text.len()).ok_or(LIMIT)?)?;
    value.extend(std::iter::repeat_n(' ', padding)); value.push_str(text); Ok(value)
}

pub(crate) fn replace(rope: &Rope, block: BlockSelection, insert: &str, extra: usize, input_work: usize) -> Result<Plan, &'static str> {
    let count = geometry(rope, &block)?;
    if insert.len() > MAX_WORK { return Err(LIMIT); }
    let column = block.min_col().checked_add(insert.chars().count()).ok_or(LIMIT)?;
    let mut budget = Budget::new(extra)?; budget.text(input_work)?; budget.rows::<Span>(count)?; edit_budget(&mut budget, count)?;
    let mut spans = reserve(count)?;
    for line in block.min_line()..=block.max_line() {
        let row = row(rope, line)?;
        let range = row.byte(block.min_col())?..row.byte(block.max_col())?;
        let padding = if insert.is_empty() { 0 } else { block.min_col().saturating_sub(row.body.len_chars()) };
        edit_work(&mut budget, &range, padding.checked_add(insert.len()).ok_or(LIMIT)?)?;
        spans.push(Span { range, padding });
    }
    let mut edits = reserve(count)?;
    for span in spans.into_iter().rev() {
        if !span.range.is_empty() || span.padding != 0 || !insert.is_empty() {
            edits.push(Edit { range: span.range, text: payload(span.padding, insert)? });
        }
    }
    Ok(Plan { edits, block: thin(block, column), head: (block.head_line, column), update_geometry: true })
}

pub(crate) fn delete_stream(rope: &Rope, range: Range<usize>, extra: usize) -> Result<Plan, &'static str> {
    let range = crate::buffer::char_boundary_range(rope, range);
    let mut budget = Budget::new(extra)?; edit_budget(&mut budget, 1)?; edit_work(&mut budget, &range, 0)?;
    let mut edits = reserve(1)?;
    if !range.is_empty() { edits.push(Edit { range, text: String::new() }); }
    Ok(Plan { edits, block: None, head: (0, 0), update_geometry: false })
}
pub(crate) fn copy(rope: &Rope, block: BlockSelection, cut: bool, extra: usize) -> Result<ClipboardPlan, &'static str> {
    let count = geometry(rope, &block)?;
    let mut budget = Budget::new(extra)?; budget.rows::<Span>(count)?;
    if cut { edit_budget(&mut budget, count)?; }
    let width = block.max_col().checked_sub(block.min_col()).ok_or(LIMIT)?;
    let mut spans = reserve(count)?; let mut bytes = 0usize;
    for line in block.min_line()..=block.max_line() {
        let row = row(rope, line)?; let range = row.byte(block.min_col())?..row.byte(block.max_col())?;
        let padding = block.max_col().saturating_sub(row.body.len_chars().max(block.min_col())).min(width);
        let selected = range.end.saturating_sub(range.start);
        let length = selected.checked_add(padding).and_then(|n| n.checked_add(usize::from(line != block.max_line()))).ok_or(LIMIT)?;
        bytes = bytes.checked_add(length).ok_or(LIMIT)?;
        budget.text(length.checked_mul(2).and_then(|n| n.checked_add(if cut { selected } else { 0 })).ok_or(LIMIT)?)?;
        spans.push(Span { range, padding });
    }
    let sentinel = super::block::BLOCK_CLIPBOARD_SENTINEL;
    budget.text(sentinel.len())?;
    let mut history = string(bytes)?; let mut edits = if cut { reserve(count)? } else { Vec::new() };
    for (index, span) in spans.into_iter().enumerate() {
        let selected = rope.get_byte_slice(span.range.clone()).ok_or(GEOMETRY)?;
        for chunk in selected.chunks() { history.push_str(chunk); }
        history.extend(std::iter::repeat_n(' ', span.padding));
        if index + 1 < count { history.push('\n'); }
        if cut && !span.range.is_empty() { edits.push(Edit { range: span.range, text: String::new() }); }
    }
    let mut payload = string(sentinel.len().checked_add(history.len()).ok_or(LIMIT)?)?;
    payload.push_str(sentinel); payload.push_str(&history); edits.reverse();
    let cut = cut.then_some(Plan { edits, block: thin(block, block.min_col()), head: (block.head_line, block.min_col()), update_geometry: true });
    Ok(ClipboardPlan { history, payload, cut })
}

pub(crate) fn paste(rope: &Rope, block: Option<BlockSelection>, selection: Selection, clip: &str, extra: usize, input_work: usize) -> Result<Plan, &'static str> {
    if clip.len() > MAX_WORK { return Err(LIMIT); }
    let (start, column) = if let Some(block) = block {
        geometry(rope, &block)?; (block.min_line(), block.min_col())
    } else {
        let head = crate::buffer::floor_char_boundary(rope, selection.head);
        let line = rope.try_byte_to_line(head).map_err(|_| GEOMETRY)?;
        let row = row(rope, line)?;
        let local = head.saturating_sub(row.start).min(row.body.len_bytes());
        (line, row.body.try_byte_to_char(local).map_err(|_| GEOMETRY)?)
    };
    let rows = clip.split('\n').count(); if rows > MAX_ROWS { return Err(LIMIT); }
    let count = rows.min(rope.len_lines().saturating_sub(start));
    if count == 0 { return Err(GEOMETRY); }
    let last = start.checked_add(count - 1).ok_or(GEOMETRY)?;
    let end = block.map_or(last, |b| b.max_line().max(last));
    let total = end.checked_sub(start).and_then(|n| n.checked_add(1)).ok_or(GEOMETRY)?;
    if total > MAX_ROWS { return Err(LIMIT); }
    let mut budget = Budget::new(extra)?; budget.rows::<Span>(total)?; budget.rows::<&str>(rows)?; edit_budget(&mut budget, total)?;
    budget.text(input_work)?; budget.work(clip.len())?;
    let mut lines = reserve(rows)?; lines.extend(clip.split('\n'));
    let widest = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
    let post_column = column.checked_add(widest).ok_or(LIMIT)?;
    let mut spans = reserve(total)?;
    for line in start..=end {
        let row = row(rope, line)?; let index = line - start;
        let text = if index < count { lines.get(index).copied().ok_or(GEOMETRY)? } else { "" };
        let range = row.byte(column)?..row.byte(block.map_or(column, |b| b.max_col()))?;
        let padding = if index < count { column.saturating_sub(row.body.len_chars()) } else { 0 };
        edit_work(&mut budget, &range, padding.checked_add(text.len()).ok_or(LIMIT)?)?;
        spans.push(Span { range, padding });
    }
    let mut edits = reserve(total)?;
    for (index, span) in spans.into_iter().enumerate().rev() {
        let text = if index < count { lines.get(index).copied().ok_or(GEOMETRY)? } else { "" };
        if !span.range.is_empty() || span.padding != 0 || !text.is_empty() {
            edits.push(Edit { range: span.range, text: payload(span.padding, text)? });
        }
    }
    let after = BlockSelection { anchor_line: start, anchor_col: column, head_line: last, head_col: post_column, is_dragging: false };
    Ok(Plan { edits, block: (!after.is_empty()).then_some(after), head: (last, post_column), update_geometry: true })
}

pub(crate) fn case(rope: &Rope, block: BlockSelection, transform: &impl Fn(&str) -> String, extra: usize) -> Result<Plan, &'static str> {
    let count = geometry(rope, &block)?;
    let mut budget = Budget::new(extra)?; budget.rows::<Range<usize>>(count)?; edit_budget(&mut budget, count)?;
    let mut ranges = reserve(count)?;
    for line in block.min_line()..=block.max_line() {
        let row = row(rope, line)?; let range = row.byte(block.min_col())?..row.byte(block.max_col())?;
        // The six case transforms can expand Unicode scalars. Charge a
        // conservative 20x bound before calling any transform or copying text.
        budget.text(range.len().checked_mul(20).ok_or(LIMIT)?)?; ranges.push(range);
    }
    let mut edits = reserve(count)?;
    for range in ranges.into_iter().rev().filter(|range| !range.is_empty()) {
        let selected = rope.get_byte_slice(range.clone()).ok_or(GEOMETRY)?;
        let mut before = string(selected.len_bytes())?; for chunk in selected.chunks() { before.push_str(chunk); }
        let after = transform(&before);
        if after.len() > before.len().checked_mul(4).ok_or(LIMIT)? { return Err(LIMIT); }
        if before != after { edits.push(Edit { range, text: after }); }
    }
    let changed = !edits.is_empty();
    Ok(Plan { edits, block: Some(block), head: (block.head_line, block.head_col), update_geometry: changed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn block(rows: (usize, usize), cols: (usize, usize)) -> BlockSelection {
        BlockSelection { anchor_line: rows.0, head_line: rows.1, anchor_col: cols.0, head_col: cols.1, is_dragging: true }
    }

    #[test]
    fn column_safety_budget_boundaries_and_overflow_are_checked() {
        let mut budget = Budget::new(MAX_ALLOCATION).unwrap();
        assert!(budget.allocate(1).is_err());
        let mut budget = Budget::new(0).unwrap(); budget.work(MAX_WORK).unwrap(); assert!(budget.work(1).is_err());
        let mut budget = Budget { work: usize::MAX, allocation: usize::MAX };
        assert!(budget.work(1).is_err()); assert!(budget.allocate(1).is_err());
        assert!(Budget::new(usize::MAX).is_err());
    }

    #[test]
    fn column_safety_row_mapping_clamps_unicode_and_trims_only_cr_lf() {
        let rope = Rope::from_str("é😀\r\n中\rx\n\u{2028}z");
        for index in 0..rope.len_lines() {
            let row = row(&rope, index).unwrap();
            for column in [0, 1, 2, usize::MAX] {
                let byte = row.byte(column).unwrap();
                assert!(byte <= rope.len_bytes()); assert_eq!(crate::buffer::floor_char_boundary(&rope, byte), byte);
                assert_eq!(crate::edit_ops::util::byte_at_line_col(&rope, index, column), byte);
            }
        }
        let first = row(&rope, 0).unwrap(); assert_eq!(first.body.to_string(), "é😀"); assert_eq!(first.byte(1).unwrap(), 2);
        assert_eq!(crate::edit_ops::util::byte_at_line_col(&rope, usize::MAX, usize::MAX), rope.len_bytes());
    }

    #[test]
    fn column_safety_case_refuses_later_row_before_running_any_transform() {
        let rope = Rope::from_str(&format!("x\n{}", "é".repeat(4000)));
        let called = Cell::new(0);
        let result = case(&rope, block((0, 1), (0, 4000)), &|text| { called.set(called.get() + 1); text.to_uppercase() }, 0);
        assert!(result.is_err()); assert_eq!(called.get(), 0); assert_eq!(rope.len_bytes(), 8002);
    }

    #[test]
    fn column_safety_replace_and_paste_preflight_all_virtual_rows() {
        let rope = Rope::from_str("a\nb");
        for col in [MAX_WORK, usize::MAX] {
            assert!(replace(&rope, block((1, 0), (col, col)), "x", 0, 0).is_err());
            assert!(paste(&rope, Some(block((1, 0), (col, col))), Selection::default(), "x\ny", 0, 0).is_err());
        }
        for b in [block((usize::MAX, 0), (0, 1)), block((0, 99), (0, 1))] {
            assert!(replace(&rope, b, "x", 0, 0).is_err()); assert!(copy(&rope, b, true, 0).is_err());
        }
    }

    #[test]
    fn column_safety_physical_eol_and_unterminated_empty_rows_keep_payload() {
        for eol in ["\n", "\r", "\r\n"] {
            let rope = Rope::from_str(&format!("é{eol}😀{eol}"));
            let b = block((2, 0), (0, 2));
            let plan = copy(&rope, b, false, 0).unwrap();
            assert_eq!(plan.history, "é \n😀 \n  "); assert!(plan.cut.is_none());
            let plan = replace(&rope, block((2, 0), (3, 3)), "中", 0, 0).unwrap();
            assert_eq!(plan.edits.len(), 3);
            assert_eq!(plan.edits[0].text, "   中");
            assert_eq!((plan.block.unwrap().anchor_line, plan.head), (2, (0, 4)));
        }
        let rope = Rope::from_str("é");
        assert_eq!(copy(&rope, block((0, 0), (0, 1)), false, 0).unwrap().history, "é");
        assert!(replace(&rope, block((0, 0), (0, 0)), "", 0, 0).unwrap().edits.is_empty());
    }
}