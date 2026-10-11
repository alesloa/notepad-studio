use super::*;
use crate::buffer::{Buffer, UndoEntry};
use std::sync::Arc;

fn group(text: &str, changes: &[(Range<usize>, &str)]) -> PreparedGroup {
    let rope = Rope::from_str(text);
    PreparedGroup::build(rope, ContentRevision::default(), Selection::default(), Selection::default(),
        changes.iter().map(|(range, text)| PrefixEdit { range: range.clone(), text: (*text).into() }).collect(),
        &AtomicBool::new(false)).unwrap()
}

fn legacy(group: &PreparedGroup, metadata: Metadata, undo: bool) -> Metadata {
    let mut buffer = Buffer::empty();
    buffer.rope = if undo { group.body.after.clone() } else { group.body.before.clone() };
    buffer.bookmarks = metadata.bookmarks;
    buffer.marks = metadata.marks;
    let mut marker = metadata.marker;
    let ops: Vec<_> = if undo { group.body.ops.iter().rev().collect() } else { group.body.ops.iter().collect() };
    for op in ops {
        let (removed, inserted) = if undo { (&op.inserted, &op.removed) } else { (&op.removed, &op.inserted) };
        let start = op.byte_start;
        let end = start + removed.len();
        let chars = buffer.rope.byte_to_char(start)..buffer.rope.byte_to_char(end);
        buffer.rope.remove(chars);
        buffer.rope.insert(buffer.rope.byte_to_char(start), inserted);
        buffer.shift_bookmarks_for_edit(start, end, inserted.len());
        buffer.normalize_bookmarks();
        buffer.shift_marks_for_edit(start, end, inserted.len());
        marker = marker.and_then(|marker| {
            if marker >= start && marker <= end { None }
            else if marker > end { Some(marker - removed.len() + inserted.len()) }
            else { Some(marker) }
        });
    }
    Metadata { bookmarks: buffer.bookmarks, marks: buffer.marks, marker }
}

#[test]
fn prefix_mapper_matches_per_edit_metadata_affinities_in_both_directions() {
    for group in [
        group("é😀\r\n \tx\r  y\nz\u{2028}q", &[(0..0, "    "), (8..10, "\t\t"), (12..14, ""), (16..16, " ")]),
        group("  a\n\tb\r\n c", &[(0..2, ""), (4..5, "    "), (8..9, "\t")]),
    ] {
        for undo in [false, true] {
            let source = if undo { &group.body.after } else { &group.body.before };
            let total = source.len_bytes();
            let bookmarks: BTreeSet<_> = (0..=total).collect();
            for marker in 0..=total {
                if crate::buffer::floor_char_boundary(source, marker) != marker { continue; }
                let mut marks = Vec::new();
                for start in 0..total { for end in start+1..=total {
                    if crate::buffer::char_boundary_range(source, start..end) == (start..end) { marks.push(start..end); }
                } }
                let metadata = Metadata { bookmarks: bookmarks.clone(), marks, marker: Some(marker) };
                let got = group.remap(&metadata, undo, &AtomicBool::new(false)).unwrap();
                let want = legacy(&group, metadata, undo);
                assert_eq!(got.bookmarks, want.bookmarks, "undo={undo} marker={marker}");
                assert_eq!(got.marks, want.marks, "undo={undo} marker={marker}");
                assert_eq!(got.marker, want.marker, "undo={undo} marker={marker}");
            }
        }
    }
}

#[test]
fn prepared_history_clones_share_and_legacy_codec_restores_text() {
    let group = Arc::new(group(" a\r\n\té😀", &[(0..1, "    "), (4..5, "")]));
    let entry = UndoEntry::Prepared(group.clone());
    let cloned = entry.clone();
    assert!(matches!(&cloned, UndoEntry::Prepared(other) if Arc::ptr_eq(&group, other)));
    let encoded = crate::buffer::serialize_undo_stack(&[entry]);
    let legacy_entry = UndoEntry::Group { ops: group.body.ops.clone(),
        sel_before_anchor: group.body.before_selection.anchor, sel_before_head: group.body.before_selection.head };
    assert_eq!(encoded, crate::buffer::serialize_undo_stack(&[legacy_entry]));
    let restored = crate::buffer::deserialize_undo_stack(&encoded);
    assert_eq!(restored.len(), 1);
    let retained = crate::buffer::retain_replayable_undo(restored, &group.body.after);
    assert_eq!(retained.len(), 1);
    let mut rope = group.body.after.clone();
    let mut entry = retained.into_iter().next().unwrap();
    assert!(crate::buffer::undo_entry_on(&mut rope, &mut entry));
    assert_eq!(rope, group.body.before);
}

#[test]
fn rebinding_shared_prepared_endpoints_keeps_sealed_history_shared() {
    let original = Arc::new(group(" a\n b", &[(0..1, "    "), (3..4, "")]));
    let before_revision = original.expected(false).clone();
    let after_revision = original.expected(true).clone();
    let ops = original.ops().as_ptr();
    let encoded = crate::buffer::serialize_undo_stack(&[UndoEntry::Prepared(original.clone())]);
    let terminal = ContentRevision::default();
    let mut stack = vec![UndoEntry::Prepared(original.clone())];
    crate::buffer::rebind_undo_revisions(&mut stack, &terminal);
    let UndoEntry::Prepared(rebound) = &mut stack[0] else { panic!("prepared entry required"); };
    assert_eq!(rebound.ops().as_ptr(), ops, "revision rebinding must not copy the sealed operations");
    assert_eq!(rebound.expected(true), &terminal);
    assert_ne!(rebound.expected(false), &before_revision);
    assert_eq!(original.expected(false), &before_revision);
    assert_eq!(original.expected(true), &after_revision);
    assert_eq!(crate::buffer::serialize_undo_stack(&stack), encoded);

    let next = ContentRevision::default();
    let UndoEntry::Prepared(rebound) = &mut stack[0] else { panic!("prepared entry required"); };
    Arc::make_mut(rebound).rebind_after(next.clone());
    assert_eq!(rebound.ops().as_ptr(), ops);
    assert_eq!(rebound.expected(true), &next);
    assert_eq!(original.expected(true), &after_revision);
    assert_eq!(crate::buffer::serialize_undo_stack(&stack), encoded);
}

#[test]
fn hostile_prefix_plans_and_offsets_are_refused_or_clamped() {
    for changes in [vec![(1..2, " ")], vec![(0..usize::MAX, " ")], vec![(0..0, "x")],
        vec![(0..0, " "), (0..0, " ")], vec![(0..0, "\n")]] {
        let result = PreparedGroup::build(Rope::from_str("😀\n"), ContentRevision::default(),
            Selection::default(), Selection::default(), changes.into_iter().map(|(range, text)|
                PrefixEdit { range, text: text.into() }).collect(), &AtomicBool::new(false));
        assert!(result.is_err());
    }
    let group = group("é\nq", &[(0..0, " ")]);
    let metadata = Metadata { bookmarks: [usize::MAX].into_iter().collect(),
        marks: vec![0..usize::MAX, usize::MAX..usize::MAX], marker: Some(usize::MAX) };
    let mapped = group.remap(&metadata, false, &AtomicBool::new(false)).unwrap();
    assert_eq!(mapped.bookmarks, [4].into_iter().collect());
    assert!(mapped.marks.is_empty());
    assert_eq!(mapped.marker, None);
    assert!(group.remap(&metadata, false, &AtomicBool::new(true)).is_err());
}

#[test]
fn hostile_mid_codepoint_marks_and_marker_drop_before_prefix_mapping() {
    let group = group("é😀\nq", &[(0..0, " ")]);
    for undo in [false, true] {
        let start = usize::from(undo);
        let metadata = Metadata { bookmarks: [start+1].into_iter().collect(),
            marks: vec![start+1..start+2, start+2..start+4], marker: Some(start+1) };
        let mapped = group.remap(&metadata, undo, &AtomicBool::new(false)).unwrap();
        assert!(mapped.marks.is_empty());
        assert_eq!(mapped.marker, None);
    }
}

#[test]
fn prepared_budget_rejects_hostile_counts_and_caps_growing_plan_storage() {
    assert!(history_record_budget(usize::MAX, 0).is_err());
    assert!(history_record_budget(0, usize::MAX).is_err());
    let per_record = history_record_budget(1, 2).unwrap();
    let records = MAX_HISTORY_BYTES / per_record;
    assert!(history_record_budget(records, records * 2).is_ok());
    assert!(history_record_budget(records + 1, (records + 1) * 2).is_err());
    let group = group(" a\n b", &[(0..1, ""), (3..4, "")]);
    assert!(group.matches_rope(group.expected_rope(true), true, &AtomicBool::new(true)).is_err());
    assert!(group.matches_rope(&Rope::from_str(&group.body.after.to_string()), true, &AtomicBool::new(false)).unwrap());
}
