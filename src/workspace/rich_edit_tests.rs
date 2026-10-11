// Headless handler/state tests; these do not verify native rendering.

use gpui::{AppContext, Context, Entity, TestAppContext, WindowAppearance};

use super::{HeadingShift, RichDocStyleAxis, Workspace};
use crate::rich_doc::{TextSelection, ast::{Alignment, Block, BlockProps, InlineRun}, edits::Edit};
use crate::tab::{Tab, TabViewMode};

#[derive(Clone, Copy, Debug)]
enum Mutation {
    Text,
    ParagraphBreak,
    DeleteForward,
    DeleteBackward,
    DeleteSelection,
    MergeNext,
    MergePrevious,
    DeleteAcrossBlocks,
    PlainPaste,
    FormattedPaste,
    Heading,
    List,
    Alignment,
    Indent,
    BlockStyle,
    RangeStyle,
    Color,
    ClearColor,
    Link,
    Unlink,
}

impl Mutation {
    const ALL: [Self; 20] = [
        Self::Text, Self::ParagraphBreak, Self::DeleteForward, Self::DeleteBackward,
        Self::DeleteSelection, Self::MergeNext, Self::MergePrevious, Self::DeleteAcrossBlocks,
        Self::PlainPaste, Self::FormattedPaste, Self::Heading, Self::List,
        Self::Alignment, Self::Indent, Self::BlockStyle, Self::RangeStyle,
        Self::Color, Self::ClearColor, Self::Link, Self::Unlink,
    ];

    fn prepare(self, w: &mut Workspace) {
        let payload = w.active_tab_mut().buffer.rich_doc.as_mut().expect("rich fixture");
        if matches!(self, Self::MergeNext | Self::MergePrevious | Self::DeleteAcrossBlocks) {
            payload.apply_edit(Edit::SplitBlock { block: 0, run: 0, byte: 2 });
        }
        let (anchor, cursor) = match self {
            Self::DeleteBackward => { let end = payload.block_end(0); (end, end) }
            Self::DeleteSelection | Self::RangeStyle => (payload.block_start(0), payload.block_end(0)),
            Self::MergeNext => { let end = payload.block_end(0); (end, end) }
            Self::MergePrevious => { let start = payload.block_start(1); (start, start) }
            Self::DeleteAcrossBlocks => (payload.block_start(0), payload.block_end(1)),
            _ => { let start = payload.block_start(0); (start, start) }
        };
        payload.active_block = Some(cursor.block);
        payload.selection = Some(TextSelection { anchor, cursor });
        match self {
            Self::Color | Self::ClearColor => {
                if matches!(self, Self::ClearColor) {
                    payload.apply_edit(Edit::SetBlockColor { block: 0, color: Some(0x123456) });
                }
                w.rich_doc_color_bar = Some(crate::find_state::RichDocColorBarState::new(0, "00ff00", None));
            }
            Self::Link | Self::Unlink => {
                if matches!(self, Self::Unlink) {
                    payload.apply_edit(Edit::SetBlockLink { block: 0, url: Some("https://example.invalid".into()) });
                }
                w.rich_doc_link_bar = Some(crate::find_state::RichDocLinkBarState::new(0, "https://example.invalid", false));
            }
            _ => {}
        }
        w.active_tab_mut().buffer.dirty = false;
    }

    fn apply(self, w: &mut Workspace, cx: &mut Context<Workspace>) {
        match self {
            Self::Text => w.rich_doc_insert_text("draft 中 ", cx),
            Self::ParagraphBreak => w.rich_doc_insert_paragraph_break(cx),
            Self::DeleteForward | Self::MergeNext => w.rich_doc_delete(true, cx),
            Self::DeleteBackward | Self::DeleteSelection | Self::MergePrevious | Self::DeleteAcrossBlocks => w.rich_doc_delete(false, cx),
            Self::PlainPaste => w.rich_doc_paste("draft 中\r\nnext", cx),
            Self::FormattedPaste => w.rich_doc_paste_formatted(vec![Block::Paragraph {
                runs: vec![InlineRun::plain("draft 中 ")], props: BlockProps::default(),
            }], cx),
            Self::Heading => w.rich_doc_shift_heading(HeadingShift::Promote, cx),
            Self::List => w.rich_doc_toggle_list(false, cx),
            Self::Alignment => w.rich_doc_set_alignment(Alignment::Center, cx),
            Self::Indent => w.rich_doc_apply_indent_delta(1, cx),
            Self::BlockStyle | Self::RangeStyle => w.rich_doc_toggle_style(RichDocStyleAxis::Bold, cx),
            Self::Color => w.apply_rich_doc_color_buffer(cx),
            Self::ClearColor => w.rich_doc_color_clear(cx),
            Self::Link => w.commit_rich_doc_link_bar(cx),
            Self::Unlink => w.rich_doc_link_unlink(cx),
        }
    }
}

fn workspace(cx: &mut TestAppContext) -> Entity<Workspace> {
    let path = std::env::temp_dir().join(format!("notepad-studio-rich-edit-{}.rtf", uuid::Uuid::new_v4()));
    std::fs::write(&path, br"{\rtf1\ansi caf\'e9}").expect("write neutral synthetic RTF");
    let mut buffer = crate::io::read_buffer(&path).expect("read synthetic RTF");
    std::fs::remove_file(&path).expect("remove synthetic fixture");
    buffer.file_path = None;
    cx.new(move |cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., WindowAppearance::Dark,
        );
        let mut tab = Tab::empty();
        tab.buffer = buffer;
        tab.view_mode = TabViewMode::RichDocView;
        w.tabs = vec![tab];
        w.active = 0;
        w
    })
}

#[test]
fn rich_document_mutations_schedule_recovery() {
    for mutation in Mutation::ALL {
        let mut cx = TestAppContext::single();
        let entity = workspace(&mut cx);
        entity.update(&mut cx, |w, cx| {
            mutation.prepare(w);
            let before = w.buffer().rich_doc.as_ref().expect("rich fixture").ast.clone();
            let generation = w.autosave_generation;
            mutation.apply(w, cx);
            assert_ne!(w.buffer().rich_doc.as_ref().expect("rich fixture").ast, before, "{mutation:?}");
            assert!(w.buffer().dirty, "{mutation:?} must mark the document unsaved");
            assert!(w.autosave_generation > generation, "{mutation:?} must schedule recovery");
            // Never let a synthetic test workspace write user recovery state.
            w.window_closed = true;
        });
        cx.quit();
    }
}

#[test]
fn read_only_rich_document_mutations_preserve_content_and_history() {
    for mutation in Mutation::ALL {
        let mut cx = TestAppContext::single();
        let entity = workspace(&mut cx);
        entity.update(&mut cx, |w, cx| {
            mutation.prepare(w);
            w.active_tab_mut().buffer.read_only = true;
            let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
            let before = payload.ast.clone();
            let (undo, redo) = (payload.undo_stack.len(), payload.redo_stack.len());
            let generation = w.autosave_generation;
            mutation.apply(w, cx);
            let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
            assert_eq!(payload.ast, before, "{mutation:?}");
            assert_eq!((payload.undo_stack.len(), payload.redo_stack.len()), (undo, redo), "{mutation:?}");
            assert!(!w.buffer().dirty, "{mutation:?}");
            assert_eq!(w.autosave_generation, generation, "{mutation:?}");
            w.window_closed = true;
        });
        cx.quit();
    }
}

#[test]
fn no_op_rich_document_actions_do_not_schedule_recovery() {
    let mut cx = TestAppContext::single();
    let entity = workspace(&mut cx);
    entity.update(&mut cx, |w, cx| {
        Mutation::Text.prepare(w);
        let generation = w.autosave_generation;
        w.rich_doc_insert_text("", cx);
        w.rich_doc_delete(false, cx); // Backspace at document start.
        w.rich_doc_shift_heading(HeadingShift::Demote, cx); // Plain paragraph.
        w.rich_doc_paste_formatted(Vec::new(), cx);
        let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
        w.find_state.rich_doc_block_offsets = Some(payload.flat_search_text().1);
        assert_eq!(w.replace_rich_doc_ranges(&[], "中", cx), 0);
        assert_eq!(w.replace_rich_doc_ranges(std::slice::from_ref(&(0..0)), "", cx), 0);
        let payload = w.active_tab_mut().buffer.rich_doc.as_mut().expect("rich fixture");
        payload.selection = None;
        payload.active_block = None;
        w.rich_doc_insert_text("ignored without a caret", cx);
        w.rich_doc_toggle_style(RichDocStyleAxis::Bold, cx);
        assert!(!w.buffer().dirty);
        assert_eq!(w.autosave_generation, generation);
        w.window_closed = true;
    });
    cx.quit();
}

#[test]
fn rich_replace_schedules_one_recovery_for_the_batch() {
    let single = std::iter::once(0..3).collect::<Vec<_>>();
    for ranges in [single, vec![2..3, 0..2]] {
        let mut cx = TestAppContext::single();
        let entity = workspace(&mut cx);
        entity.update(&mut cx, |w, cx| {
            let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
            w.find_state.rich_doc_block_offsets = Some(payload.flat_search_text().1);
            let generation = w.autosave_generation;
            assert_eq!(w.replace_rich_doc_ranges(&ranges, "中", cx), ranges.len());
            let expected = if ranges.len() == 1 { "中é" } else { "中中é" };
            assert_eq!(w.buffer().rich_doc.as_ref().expect("rich fixture").block_text(0), expected);
            assert!(w.buffer().dirty);
            assert_eq!(w.autosave_generation, generation.wrapping_add(1));
            w.window_closed = true;
        });
        cx.quit();
    }
}

#[test]
fn read_only_rich_replace_preserves_the_document() {
    let single = std::iter::once(0..3).collect::<Vec<_>>();
    for ranges in [single, vec![2..3, 0..2]] {
        let mut cx = TestAppContext::single();
        let entity = workspace(&mut cx);
        entity.update(&mut cx, |w, cx| {
            w.active_tab_mut().buffer.read_only = true;
            let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
            let before = payload.ast.clone();
            w.find_state.rich_doc_block_offsets = Some(payload.flat_search_text().1);
            let generation = w.autosave_generation;
            assert_eq!(w.replace_rich_doc_ranges(&ranges, "中", cx), 0);
            let payload = w.buffer().rich_doc.as_ref().expect("rich fixture");
            assert_eq!(payload.ast, before);
            assert!(payload.undo_stack.is_empty());
            assert!(payload.redo_stack.is_empty());
            assert!(!w.buffer().dirty);
            assert_eq!(w.autosave_generation, generation);
            w.window_closed = true;
        });
        cx.quit();
    }
}
