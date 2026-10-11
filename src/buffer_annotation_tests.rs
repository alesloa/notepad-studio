use super::Buffer;
use crate::settings::{override_for_test, Settings};
use ropey::Rope;

#[test]
fn hostile_bookmark_offsets_cannot_overflow_during_insertion() {
    let _settings = override_for_test(Settings::default());
    let mut buffer = Buffer::empty();
    buffer.rope = Rope::from_str("xxaé\n中\n");
    buffer
        .bookmarks
        .extend([0, isize::MAX as usize, usize::MAX]);
    buffer.shift_bookmarks_for_edit(0, 0, 2);
    buffer.normalize_bookmarks();
    assert_eq!(
        buffer.bookmarks.into_iter().collect::<Vec<_>>(),
        [0, buffer.rope.len_bytes()]
    );
}

#[test]
fn hostile_mark_offsets_cannot_overflow_or_survive_an_edit() {
    let _settings = override_for_test(Settings::default());
    let mut buffer = Buffer::empty();
    buffer.rope = Rope::from_str("xxaé\n中\n");
    let huge = isize::MAX as usize;
    buffer.marks = vec![
        huge..huge + 1,
        usize::MAX - 1..usize::MAX,
        std::ops::Range { start: 1, end: 0 },
    ];
    buffer.shift_marks_for_edit(0, 0, 2);
    assert!(buffer.marks.is_empty());
}

#[test]
fn valid_annotation_insertion_and_deletion_affinities_stay_exact() {
    let _settings = override_for_test(Settings::default());
    for insert in [true, false] {
        let mut buffer = Buffer::empty();
        buffer.bookmarks.extend([0, 3, 4, 6]);
        buffer.marks = vec![0..2, 3..5, 6..8, 2..4];
        buffer.rope = Rope::from_str(if insert { "aa\nXYbb\ncc\n" } else { "aa\ncc\n" });
        let (end, length) = if insert { (3, 2) } else { (6, 0) };
        buffer.shift_bookmarks_for_edit(3, end, length);
        buffer.normalize_bookmarks();
        buffer.shift_marks_for_edit(3, end, length);
        if insert {
            assert_eq!(buffer.bookmarks.into_iter().collect::<Vec<_>>(), [0, 3, 8]);
            assert_eq!(buffer.marks, [0..2, 5..7, 8..10]);
        } else {
            assert_eq!(buffer.bookmarks.into_iter().collect::<Vec<_>>(), [0, 3]);
            assert_eq!(buffer.marks, [0..2, 3..5]);
        }
    }
}

#[test]
fn oversized_annotation_shift_lengths_never_use_signed_arithmetic() {
    let _settings = override_for_test(Settings::default());
    for (start, end, inserted) in [(0, usize::MAX, 2), (0, 0, usize::MAX), (usize::MAX, 0, 2)] {
        let mut buffer = Buffer::empty();
        buffer.rope = Rope::from_str("é\n中\n");
        buffer
            .bookmarks
            .extend([1, isize::MAX as usize, usize::MAX]);
        buffer.marks = vec![0..2, 1..usize::MAX];
        buffer.shift_bookmarks_for_edit(start, end, inserted);
        buffer.normalize_bookmarks();
        buffer.shift_marks_for_edit(start, end, inserted);
        assert!(buffer
            .bookmarks
            .iter()
            .all(|b| *b <= buffer.rope.len_bytes()));
        assert!(buffer
            .marks
            .iter()
            .all(|r| buffer.rope.get_byte_slice(r.clone()).is_some()));
    }
}
