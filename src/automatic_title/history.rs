// NppNotification.cpp:52-61 — Undo/Redo also emit delete/insert notifications.

use std::sync::atomic::{AtomicBool, Ordering};

use ropey::Rope;

use crate::buffer::{EditOp, prepared::PreparedGroup};
use super::{Cursor, Scan, Summary};

pub(crate) const WORK_LIMIT: usize = 128 * 1024 * 1024;
pub(crate) const WORK_LIMIT_ERROR: &str = "Automatic title updates paused: naming work reached the 128 MiB limit; the last accepted name is retained";

pub(crate) fn scan(rope: &Rope, mut cursor: Cursor, cancel: &AtomicBool, budget: &mut usize) -> Result<Option<super::Candidate>, &'static str> {
    loop {
        let mut visits = 0;
        let result = cursor.advance_counted(rope, (*budget).min(65536), cancel, &mut visits)?;
        *budget = budget.checked_sub(visits).ok_or(WORK_LIMIT_ERROR)?;
        match result {
            Scan::Complete(name) => return Ok(name),
            Scan::Pending(next) => {
                if visits == 0 { return Err(WORK_LIMIT_ERROR); }
                cursor = next;
            }
        }
    }
}

fn notification(rope: &Rope, position: usize, summary: &mut Summary, conflicts: &[Vec<u8>],
    cancel: &AtomicBool, budget: &mut usize) -> Result<(), &'static str>
{
    if !super::row_zero_notification(rope, position) { return Ok(()); }
    let Some(candidate) = scan(rope, Cursor::new(), cancel, budget)? else { return Ok(()); };
    for conflict in conflicts {
        if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
        if super::same_full_name(candidate.as_str()?, conflict) { return Ok(()); }
    }
    summary.append(candidate)
}

fn apply(rope: &mut Rope, op: &EditOp, undo: bool, summary: &mut Summary, conflicts: &[Vec<u8>],
    cancel: &AtomicBool, budget: &mut usize) -> Result<(), &'static str>
{
    if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
    let (removed, inserted) = if undo { (&op.inserted, &op.removed) } else { (&op.removed, &op.inserted) };
    let end = op.byte_start.checked_add(removed.len()).ok_or("Invalid automatic-name history range")?;
    let range = op.byte_start..end;
    if crate::buffer::char_boundary_range(rope, range.clone()) != range {
        return Err("Automatic-name history no longer matches its document");
    }
    let source = rope.get_byte_slice(range).ok_or("Invalid automatic-name history range")?;
    let mut matched = 0usize;
    for chunk in source.chunks() {
        if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
        let end = matched.checked_add(chunk.len()).ok_or("Invalid automatic-name history range")?;
        if removed.get(matched..end) != Some(chunk) { return Err("Automatic-name history no longer matches its document"); }
        matched = end;
    }
    if matched != removed.len() { return Err("Automatic-name history no longer matches its document"); }
    let start = rope.byte_to_char(op.byte_start);
    if !removed.is_empty() {
        rope.remove(start..rope.byte_to_char(end));
        notification(rope, op.byte_start, summary, conflicts, cancel, budget)?;
    }
    if !inserted.is_empty() {
        rope.insert(start, inserted);
        notification(rope, op.byte_start, summary, conflicts, cancel, budget)?;
    }
    Ok(())
}

pub(crate) fn summarize(group: &PreparedGroup, undo: bool, conflicts: &[Vec<u8>],
    cancel: &AtomicBool, budget: &mut usize) -> Result<Summary, &'static str>
{
    let mut rope = group.expected_rope(undo).clone();
    let mut summary = Summary::Identity;
    if undo {
        for op in group.ops().iter().rev() { apply(&mut rope, op, true, &mut summary, conflicts, cancel, budget)?; }
    } else {
        for op in group.ops() { apply(&mut rope, op, false, &mut summary, conflicts, cancel, budget)?; }
    }
    if !group.matches_rope(&rope, !undo, cancel)? { return Err("Automatic-name history has a different final document"); }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{ContentRevision, Selection, UndoEntry, prepared::PrefixEdit};

    fn group(text: &str, range: std::ops::Range<usize>, insert: &str) -> PreparedGroup {
        PreparedGroup::build_line_edits(Rope::from_str(text), ContentRevision::default(), Selection::default(),
            vec![PrefixEdit { range, text: insert.to_owned() }], false, &AtomicBool::new(false)).expect("valid sealed group")
    }

    fn accepted(summary: Summary, before: &str) -> String {
        summary.resolve(before).expect("valid summary").map(|name| name.to_string().expect("bounded name")).unwrap_or_else(|| before.to_owned())
    }

    fn effects(group: &PreparedGroup, undo: bool, conflicts: &[Vec<u8>]) -> Summary {
        let mut budget = WORK_LIMIT;
        summarize(group, undo, conflicts, &AtomicBool::new(false), &mut budget).expect("admitted history")
    }

    #[test]
    fn prepared_forward_and_inverse_keep_separate_delete_insert_effects() {
        let group = group("original\nintermediate\n", 0..9, "\n");
        assert_eq!(group.target(false).0.to_string(), "\nintermediate\n");
        assert_eq!(accepted(effects(&group, false, &[]), "original"), "intermediate");
        assert_eq!(accepted(effects(&group, true, &[]), "intermediate"), "original");
        assert_eq!(accepted(effects(&group, false, &[b"INTERMEDIATE\0hidden".to_vec()]), "original"), "original");
        assert_eq!(group.expected_rope(false).to_string(), "original\nintermediate\n");
    }

    #[test]
    fn real_merged_prepared_and_scalar_history_preserves_intermediate_name() {
        let first = group("first\nsecond\n", 0..6, "");
        let after_revision = ContentRevision::default();
        let scalar = EditOp { byte_start: 0, removed: String::new(), inserted: "\n".to_owned(),
            before_revision: first.expected(true).clone(), after_revision: after_revision.clone(),
            sel_before_anchor: 0, sel_before_head: 0 };
        let combined = PreparedGroup::combine_history(&[UndoEntry::Prepared(std::sync::Arc::new(first)), UndoEntry::Single(scalar)],
            Rope::from_str("\nsecond\n"), after_revision, Selection::default(), Selection::default(), &AtomicBool::new(false)).expect("real mixed timeline");
        assert_eq!(accepted(effects(&combined, false, &[]), "first"), "second");
        assert_eq!(accepted(effects(&combined, true, &[]), "second"), "first");
        assert_eq!(combined.expected_rope(false).to_string(), "first\nsecond\n");
        assert_eq!(combined.expected_rope(true).to_string(), "\nsecond\n");
    }

    #[test]
    fn budget_and_cancel_leave_sealed_document_and_history_exact() {
        let text = format!("{}name\nsecond", "/".repeat(10000));
        let group = group(&text, 0..0, "/");
        let before = group.expected_rope(false).clone(); let after = group.expected_rope(true).clone();
        let mut budget = 31;
        assert_eq!(summarize(&group, false, &[], &AtomicBool::new(false), &mut budget).expect_err("work budget"), WORK_LIMIT_ERROR);
        assert_eq!(budget, 0);
        let mut budget = WORK_LIMIT;
        assert!(summarize(&group, false, &[], &AtomicBool::new(true), &mut budget).is_err());
        assert_eq!(group.expected_rope(false), &before); assert_eq!(group.expected_rope(true), &after);
        assert_eq!(group.ops().len(), 1);
    }
    #[test]
    fn cached_post_notification_rows_are_directional_at_crlf_boundaries() {
        for (before, range, inserted, forward, inverse) in [
            ("first\r\nlater", 6..7, "", false, true),
            ("first\r\nlater", 5..6, "", true, true),
            ("first\rlater", 6..6, "\n", true, false),
            ("first\nlater", 6..6, "x", false, false),
            ("first\rlater", 6..7, "x", false, false),
            ("first\r\nlater", 7..8, "x", false, false),
        ] {
            let group = group(before, range, inserted);
            assert_eq!(group.has_title_notifications(false), forward, "{before:?} forward");
            assert_eq!(group.has_title_notifications(true), inverse, "{before:?} inverse");
        }
    }

    #[test]
    fn mixed_history_charge_includes_large_intermediate_op_payload() {
        let text = "/".repeat(131072);
        let first = group("a", 0..1, &text);
        let after_revision = ContentRevision::default();
        let scalar = EditOp { byte_start: 0, removed: text.clone(), inserted: "b".to_owned(),
            before_revision: first.expected(true).clone(), after_revision: after_revision.clone(),
            sel_before_anchor: 0, sel_before_head: 0 };
        let combined = PreparedGroup::combine_history(&[UndoEntry::Prepared(std::sync::Arc::new(first)), UndoEntry::Single(scalar)],
            Rope::from_str("b"), after_revision, Selection::default(), Selection::default(), &AtomicBool::new(false)).expect("mixed history");
        assert_eq!(combined.expected_rope(false).len_bytes(), 1);
        assert_eq!(combined.expected_rope(true).len_bytes(), 1);
        assert!(combined.retained_title_bytes().expect("cached charge") >= 4 * text.len());
        assert!(combined.has_title_notifications(false) && combined.has_title_notifications(true));
        assert_eq!(accepted(effects(&combined, false, &[]), "a"), "b");
        assert_eq!(accepted(effects(&combined, true, &[]), "b"), "a");
    }

}
