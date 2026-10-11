// PowerEditor/src/NppCommands.cpp:477-493 — Undo / Redo commands.
// scintilla/src/Document.cxx:1589-1594,1661-1666 — read-only history guard.

use crate::buffer::{Buffer, ContentRevision, UndoEntry};
use crate::tab::{InputFocus, Tab, TabViewMode};
use crate::ui::csv_doc::CsvModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HistoryTarget {
    Editor,
    RichDocument,
    Csv,
    Ignored,
    ReadOnly,
}

#[derive(Clone, Copy)]
pub(super) enum HistoryDirection {
    Undo,
    Redo,
}

pub(super) fn history_target(
    focus: InputFocus,
    read_only: bool,
    csv_active: bool,
) -> HistoryTarget {
    let target = match focus {
        InputFocus::RichDocBody => HistoryTarget::RichDocument,
        InputFocus::Editor if csv_active => HistoryTarget::Csv,
        InputFocus::Editor => HistoryTarget::Editor,
        InputFocus::CsvGrid | InputFocus::CsvCell if csv_active => HistoryTarget::Csv,
        _ => return HistoryTarget::Ignored,
    };
    if read_only {
        HistoryTarget::ReadOnly
    } else {
        target
    }
}

pub(super) fn apply_rich_history(buffer: &mut Buffer, direction: HistoryDirection) -> bool {
    if buffer.read_only {
        return false;
    }
    let Some(payload) = buffer.rich_doc.as_mut() else {
        return false;
    };
    let changed = match direction {
        HistoryDirection::Undo => payload.undo(),
        HistoryDirection::Redo => payload.redo(),
    };
    if changed {
        buffer.refresh_dirty();
    }
    changed
}

/// Check an entire entry before replay so stale file-derived ranges cannot
/// replace unrelated text or revive an identity for different contents.
pub(super) fn text_history_matches(buffer: &Buffer, entry: &UndoEntry, direction: HistoryDirection) -> bool {
    fn apply(rope: &mut ropey::Rope, revision: &mut ContentRevision, op: &crate::buffer::EditOp, direction: HistoryDirection) -> bool {
        let (expected, next, removed, inserted) = match direction {
            HistoryDirection::Undo => (&op.after_revision, &op.before_revision, &op.inserted, &op.removed),
            HistoryDirection::Redo => (&op.before_revision, &op.after_revision, &op.removed, &op.inserted),
        };
        if revision != expected { return false; }
        let Some(end) = op.byte_start.checked_add(removed.len()) else { return false; };
        let range = op.byte_start..end;
        if crate::buffer::char_boundary_range(rope, range.clone()) != range { return false; }
        let Some(slice) = rope.get_byte_slice(range) else { return false; };
        if slice != removed.as_str() { return false; }
        let start_char = rope.byte_to_char(op.byte_start);
        let end_char = rope.byte_to_char(end);
        rope.remove(start_char..end_char);
        rope.insert(start_char, inserted);
        *revision = next.clone();
        true
    }
    let mut rope = buffer.rope.clone();
    let mut revision = buffer.raw_revision.clone();
    match entry {
        UndoEntry::Prepared(group) => {
            let undo = matches!(direction, HistoryDirection::Undo);
            buffer.raw_revision == *group.expected(undo) && buffer.rope.is_instance(group.expected_rope(undo))
        }
        UndoEntry::Single(op) => apply(&mut rope, &mut revision, op, direction),
        UndoEntry::Group { ops, .. } => match direction {
            HistoryDirection::Undo => ops.iter().rev().all(|op| apply(&mut rope, &mut revision, op, direction)),
            HistoryDirection::Redo => ops.iter().all(|op| apply(&mut rope, &mut revision, op, direction)),
        },
    }
}

pub(super) fn history_availability(
    tab: &Tab,
    focus: InputFocus,
    csv: Option<&CsvModel>,
) -> (bool, bool) {
    let csv = csv.filter(|_| tab.view_mode == TabViewMode::CsvPreview);
    match history_target(focus, tab.buffer.read_only, csv.is_some()) {
        HistoryTarget::Editor => (!tab.undo_stack.is_empty(), !tab.redo_stack.is_empty()),
        HistoryTarget::RichDocument => tab.buffer.rich_doc.as_ref()
            .map(|payload| (!payload.undo_stack.is_empty(), !payload.redo_stack.is_empty()))
            .unwrap_or((false, false)),
        HistoryTarget::Csv => csv
            .map(|model| (!model.undo_stack.is_empty(), !model.redo_stack.is_empty()))
            .unwrap_or((false, false)),
        HistoryTarget::Ignored | HistoryTarget::ReadOnly => (false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_doc::edits::Edit;

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("notepad-studio-history-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).expect("create neutral fixture directory");
            Self(dir)
        }

        fn buffer(&self) -> Buffer {
            let path = self.0.join("history.rtf");
            std::fs::write(&path, br"{\rtf1\ansi caf\'e9}").expect("write synthetic RTF");
            crate::io::read_buffer(&path).expect("open synthetic RTF")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn dialog_history_does_not_target_document_history() {
        let inputs = [
            InputFocus::FindQuery, InputFocus::Replacement, InputFocus::Goto,
            InputFocus::FifQuery, InputFocus::FifReplacement, InputFocus::FifFilter,
            InputFocus::FifDirectory, InputFocus::MarkQuery, InputFocus::MacroPrompt,
            InputFocus::TabRename, InputFocus::FolderInlineCreate,
            InputFocus::RichDocLinkUrl, InputFocus::RichDocColorHex,
            InputFocus::CsvFindBar, InputFocus::CsvReplaceBar,
        ];
        for focus in inputs {
            for csv_active in [false, true] {
                for read_only in [false, true] {
                    assert_eq!(history_target(focus, read_only, csv_active), HistoryTarget::Ignored);
                }
            }
        }
    }

    #[test]
    fn read_only_history_does_not_target_any_document_view() {
        for focus in [InputFocus::Editor, InputFocus::RichDocBody, InputFocus::CsvGrid, InputFocus::CsvCell] {
            assert_eq!(history_target(focus, true, true), HistoryTarget::ReadOnly);
        }
        assert_eq!(history_target(InputFocus::Editor, true, false), HistoryTarget::ReadOnly);
    }

    #[test]
    fn writable_document_history_uses_the_focused_view() {
        assert_eq!(history_target(InputFocus::Editor, false, false), HistoryTarget::Editor);
        assert_eq!(history_target(InputFocus::RichDocBody, false, false), HistoryTarget::RichDocument);
        for focus in [InputFocus::Editor, InputFocus::CsvGrid, InputFocus::CsvCell] {
            assert_eq!(history_target(focus, false, true), HistoryTarget::Csv);
        }
        assert_eq!(history_target(InputFocus::CsvGrid, false, false), HistoryTarget::Ignored);
        assert_eq!(history_target(InputFocus::CsvCell, false, false), HistoryTarget::Ignored);
    }

    #[test]
    fn rich_history_marks_saved_buffers_dirty_for_undo_and_redo() {
        let fixture = Fixture::new();
        let mut buffer = fixture.buffer();
        let original = buffer.rich_doc.as_ref().expect("rich payload").block_text(0);
        buffer.rich_doc.as_mut().expect("rich payload").apply_edit(Edit::InsertText {
            block: 0, run: 0, byte: 0, text: "draft 中 ".into(),
        });
        let edited = buffer.rich_doc.as_ref().expect("rich payload").block_text(0);
        assert_ne!(edited, original);
        buffer.mark_saved(); // Simulate a save before undo.
        assert!(apply_rich_history(&mut buffer, HistoryDirection::Undo));
        assert_eq!(buffer.rich_doc.as_ref().expect("rich payload").block_text(0), original);
        assert!(buffer.dirty, "undo of a saved rich document must enter recovery");
        buffer.mark_saved(); // Simulate another save before redo.
        assert!(apply_rich_history(&mut buffer, HistoryDirection::Redo));
        assert_eq!(buffer.rich_doc.as_ref().expect("rich payload").block_text(0), edited);
        assert!(buffer.dirty, "redo of a saved rich document must enter recovery");
    }

    #[test]
    fn read_only_rich_history_preserves_content_and_stacks() {
        let fixture = Fixture::new();
        let mut buffer = fixture.buffer();
        buffer.rich_doc.as_mut().expect("rich payload").apply_edit(Edit::InsertText {
            block: 0, run: 0, byte: 0, text: "draft 中 ".into(),
        });
        buffer.read_only = true;
        buffer.dirty = false;
        let edited = buffer.rich_doc.as_ref().expect("rich payload").block_text(0);
        assert!(!apply_rich_history(&mut buffer, HistoryDirection::Undo));
        let payload = buffer.rich_doc.as_ref().expect("rich payload");
        assert_eq!(payload.block_text(0), edited);
        assert_eq!(payload.undo_stack.len(), 1);
        assert!(payload.redo_stack.is_empty());
        assert!(!buffer.dirty);

        buffer.read_only = false;
        assert!(apply_rich_history(&mut buffer, HistoryDirection::Undo));
        buffer.read_only = true;
        buffer.dirty = false;
        let restored = buffer.rich_doc.as_ref().expect("rich payload").block_text(0);
        assert!(!apply_rich_history(&mut buffer, HistoryDirection::Redo));
        let payload = buffer.rich_doc.as_ref().expect("rich payload");
        assert_eq!(payload.block_text(0), restored);
        assert!(payload.undo_stack.is_empty());
        assert_eq!(payload.redo_stack.len(), 1);
        assert!(!buffer.dirty);
    }

    #[test]
    fn empty_rich_history_leaves_clean_buffers_clean() {
        let fixture = Fixture::new();
        let mut buffer = fixture.buffer();
        assert!(!apply_rich_history(&mut buffer, HistoryDirection::Undo));
        assert!(!apply_rich_history(&mut buffer, HistoryDirection::Redo));
        assert!(!buffer.dirty);
        assert!(!apply_rich_history(&mut Buffer::empty(), HistoryDirection::Undo));
    }

    #[test]
    fn rich_history_controls_follow_rich_history_and_read_only_state() {
        let fixture = Fixture::new();
        let mut tab = Tab::empty();
        tab.buffer = fixture.buffer();
        tab.view_mode = TabViewMode::RichDocView;
        tab.buffer.rich_doc.as_mut().expect("rich payload").apply_edit(Edit::SetBlockBold { block: 0, on: true });
        assert_eq!(history_availability(&tab, InputFocus::RichDocBody, None), (true, false));
        assert!(apply_rich_history(&mut tab.buffer, HistoryDirection::Undo));
        assert_eq!(history_availability(&tab, InputFocus::RichDocBody, None), (false, true));
        tab.buffer.read_only = true;
        assert_eq!(history_availability(&tab, InputFocus::RichDocBody, None), (false, false));
        tab.buffer.read_only = false;
        assert_eq!(history_availability(&tab, InputFocus::FindQuery, None), (false, false));
        assert_eq!(history_availability(&tab, InputFocus::Editor, None), (false, false));
    }

    #[test]
    fn csv_history_controls_follow_the_grid_history() {
        let mut tab = Tab::empty();
        tab.view_mode = TabViewMode::CsvPreview;
        let mut model = CsvModel::empty(b',');
        model.rebuild_from_source("item,value\r\ncafé,1\r\n", 1);
        assert!(model.set_cell(0, 1, "2".into()));
        assert_eq!(history_availability(&tab, InputFocus::CsvGrid, Some(&model)), (true, false));
        assert!(model.undo());
        assert_eq!(history_availability(&tab, InputFocus::CsvCell, Some(&model)), (false, true));
        tab.buffer.read_only = true;
        assert_eq!(history_availability(&tab, InputFocus::CsvGrid, Some(&model)), (false, false));
        tab.buffer.read_only = false;
        assert_eq!(history_availability(&tab, InputFocus::CsvFindBar, Some(&model)), (false, false));
        tab.view_mode = TabViewMode::Edit;
        assert_eq!(history_availability(&tab, InputFocus::Editor, Some(&model)), (false, false));
    }
}
