use std::cell::Cell;

use super::*;
use crate::settings::{Settings, override_for_test};
use crate::syntax::HighlightKind;

thread_local! {
    static FLATTEN_CALLS: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn record_flatten() {
    FLATTEN_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
}

pub(crate) fn reset_flatten_calls() {
    FLATTEN_CALLS.with(|count| count.set(0));
}

pub(crate) fn flatten_calls() -> usize {
    FLATTEN_CALLS.with(Cell::get)
}

fn calls(buffer: &mut Buffer) -> usize {
    reset_flatten_calls();
    buffer.reparse_highlights();
    flatten_calls()
}

fn policy(threshold: u64) -> Settings {
    Settings {
        large_file_restriction_enabled: true, large_file_disable_syntax: true,
        large_file_threshold_bytes: threshold, ..Default::default()
    }
}

fn fixture(text: &str) -> Buffer {
    let mut buffer = Buffer::empty();
    buffer.rope = ropey::Rope::from_str(text);
    buffer.language = Some(LanguageId::Rust);
    buffer.highlights = vec![HighlightSpan { range: 0..1, kind: HighlightKind::String }];
    buffer.mark_saved();
    buffer
}

fn has_keyword(buffer: &Buffer) -> bool {
    buffer.highlights.iter().any(|span| span.range == (0..2) && span.kind == HighlightKind::Keyword)
}

#[test]
fn configured_highlight_gate_does_not_flatten_the_buffer() {
    let _settings = override_for_test(policy(1));
    let mut buffer = fixture("fn main() { let value = \"é😀\"; }\r\n");
    assert_eq!(calls(&mut buffer), 0);
    assert!(buffer.highlights.is_empty(), "skipping must clear stale spans");
}

#[test]
fn no_language_clears_spans_without_flattening() {
    let _settings = override_for_test(policy(1));
    let mut buffer = fixture("fn main() {}\r\n"); buffer.language = None;
    assert_eq!(calls(&mut buffer), 0);
    assert!(buffer.highlights.is_empty());
}

#[test]
fn below_user_gate_still_parses_and_replaces_stale_spans() {
    let text = "fn main() { let value = \"é😀\"; }\r\n";
    let _settings = override_for_test(policy(text.len() as u64 + 1));
    let mut buffer = fixture(text);
    assert_eq!(calls(&mut buffer), 1);
    assert!(has_keyword(&buffer));
}

#[test]
fn threshold_uses_exact_utf8_bytes_and_is_inclusive() {
    let text = "//é😀\r\n";
    assert!(text.chars().count() < text.len());
    for threshold in [text.len() as u64, text.len() as u64 + 1] {
        let _settings = override_for_test(policy(threshold));
        let mut buffer = fixture(text);
        assert_eq!(calls(&mut buffer), usize::from(threshold > text.len() as u64));
        if threshold == text.len() as u64 { assert!(buffer.highlights.is_empty()); }
        else { assert!(buffer.highlights.iter().any(|span| span.kind == HighlightKind::Comment)); }
    }
}

#[test]
fn disabled_user_restriction_keeps_small_highlights() {
    let mut settings = policy(1); settings.large_file_restriction_enabled = false;
    let _settings = override_for_test(settings);
    let mut buffer = fixture("fn main() {}\n");
    assert_eq!(calls(&mut buffer), 1);
    assert!(has_keyword(&buffer));
}

#[test]
fn disabled_syntax_restriction_keeps_small_highlights() {
    let mut settings = policy(1); settings.large_file_disable_syntax = false;
    let _settings = override_for_test(settings);
    let mut buffer = fixture("fn main() {}\n");
    assert_eq!(calls(&mut buffer), 1);
    assert!(has_keyword(&buffer));
}

fn large_spaces(bytes: usize) -> ropey::Rope {
    let chunk = " ".repeat(1024);
    let mut builder = ropey::RopeBuilder::new();
    for _ in 0..bytes / chunk.len() { builder.append(&chunk); }
    builder.append(&chunk[..bytes % chunk.len()]);
    builder.finish()
}

#[test]
fn hard_highlight_ceiling_refuses_before_flatten_when_user_gate_is_disabled() {
    let mut settings = policy(1); settings.large_file_restriction_enabled = false;
    let _settings = override_for_test(settings);
    let mut buffer = fixture("");
    buffer.rope = large_spaces(crate::io_stream::HIGHLIGHT_SIZE_LIMIT as usize);
    buffer.rope.append(ropey::Rope::from_str("é"));
    let original = buffer.rope.clone(); let revision = buffer.raw_revision.clone();
    assert_eq!(calls(&mut buffer), 0);
    assert!(buffer.highlights.is_empty());
    assert!(buffer.rope.is_instance(&original));
    assert_eq!(buffer.raw_revision, revision);
}

#[test]
fn exact_hard_ceiling_still_allows_highlighting() {
    let mut settings = policy(1); settings.large_file_restriction_enabled = false;
    let _settings = override_for_test(settings);
    let mut buffer = fixture("");
    buffer.rope = large_spaces(crate::io_stream::HIGHLIGHT_SIZE_LIMIT as usize);
    assert_eq!(calls(&mut buffer), 1);
    assert!(buffer.highlights.is_empty(), "spaces have no syntax spans");
}

#[test]
fn skipped_highlighting_preserves_text_savepoint_metadata_and_history() {
    let _settings = override_for_test(policy(1));
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for dirty in [false, true] {
            let mut tab = crate::tab::Tab::empty();
            tab.buffer = fixture("fn café() {}\r\n// second\rthird\n");
            tab.buffer.encoding = encoding; tab.buffer.line_ending = LineEnding::Crlf;
            tab.buffer.bookmarks.insert(0); tab.buffer.marks.push(3..8);
            tab.buffer.flat_text_cache = Some("retained cache".into());
            tab.buffer.mark_saved();
            if dirty { tab.buffer.raw_revision = ContentRevision::default(); tab.buffer.refresh_dirty(); }
            let original = tab.buffer.rope.clone();
            let revision = tab.buffer.raw_revision.clone(); let saved = tab.buffer.savepoint.clone();
            let before_bytes = crate::text::encoding::encode_checked(&original.to_string(), encoding).unwrap();
            let op = EditOp {
                before_revision: ContentRevision::default(), after_revision: revision.clone(),
                byte_start: 0, removed: String::new(), inserted: "fn".into(),
                sel_before_anchor: 0, sel_before_head: 0,
            };
            tab.undo_stack.push(UndoEntry::Single(op.clone())); tab.redo_stack.push(UndoEntry::Single(op));
            let undo = serialize_undo_stack(&tab.undo_stack); let redo = serialize_undo_stack(&tab.redo_stack);
            let count = calls(&mut tab.buffer);
            assert!(tab.buffer.rope.is_instance(&original));
            assert_eq!(tab.buffer.raw_revision, revision); assert!(tab.buffer.savepoint == saved);
            assert_eq!(tab.buffer.dirty, dirty); assert_eq!(tab.buffer.encoding, encoding);
            assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
            assert_eq!(tab.buffer.bookmarks, BTreeSet::from([0]));
            assert_eq!(tab.buffer.marks.len(), 1); assert_eq!(tab.buffer.marks.first(), Some(&(3..8)));
            assert_eq!(tab.buffer.flat_text_cache.as_deref(), Some("retained cache"));
            assert_eq!(crate::text::encoding::encode_checked(&tab.buffer.rope.to_string(), encoding).unwrap(), before_bytes);
            assert_eq!(serialize_undo_stack(&tab.undo_stack), undo); assert_eq!(serialize_undo_stack(&tab.redo_stack), redo);
            assert!(tab.buffer.highlights.is_empty()); assert_eq!(count, 0);
        }
    }
}
