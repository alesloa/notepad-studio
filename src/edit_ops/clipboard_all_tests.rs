use std::{collections::BTreeSet, ops::Range, path::PathBuf};

use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

use crate::{
    actions::{CopyBookmarkedLines, EditCopyAllFilePaths, EditCopyAllFilenames, MarkCopy},
    buffer::{serialize_undo_stack, ContentRevision, EditOp, UndoEntry},
    tab::Tab,
    text::{encoding::Encoding, line_ending::LineEnding},
    workspace::Workspace,
};

pub(crate) fn first_path() -> PathBuf {
    PathBuf::from("samples").join("first").join("café.txt")
}

pub(crate) fn second_path() -> PathBuf {
    PathBuf::from("samples").join("second").join("café.txt")
}

pub(crate) fn sample_tabs() -> Vec<Tab> {
    [
        Some(first_path()),
        None,
        Some(second_path()),
        Some(first_path()),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, path)| {
        let mut tab = Tab::empty();
        tab.buffer.file_path = path;
        tab.buffer.new_label = Some("new Ω 7".into());
        tab.buffer.rope = ropey::Rope::from_str("\u{FEFF}café😀\r\n中");
        tab.buffer.line_ending = LineEnding::Crlf;
        tab.buffer.encoding = if i % 2 == 0 {
            Encoding::Utf8Bom
        } else {
            Encoding::Utf16Le
        };
        tab.buffer.dirty = i != 1;
        tab.buffer.read_only = i == 0;
        tab.buffer.bookmarks.insert(0);
        tab.buffer.marks.push(3..8);
        tab.selection.anchor = 8;
        tab.selection.head = 3;
        tab.begin_select_marker = Some(3);
        let undo = UndoEntry::Single(EditOp {
            before_revision: ContentRevision::default(),
            after_revision: tab.buffer.raw_revision.clone(),
            byte_start: 3,
            removed: "old".into(),
            inserted: "café".into(),
            sel_before_anchor: 3,
            sel_before_head: 3,
        });
        tab.undo_stack.push(undo.clone());
        tab.redo_stack.push(undo);
        tab
    })
    .collect()
}

pub(crate) fn fixture(
    cx: &mut TestAppContext,
    tabs: Vec<Tab>,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            1.,
            WindowAppearance::Dark,
        );
        w.tabs = tabs;
        w.active = 1;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    (workspace, visual)
}

#[derive(Debug, PartialEq)]
pub(crate) struct EditState {
    active: usize,
    autosave_generation: u64,
    tabs: Vec<TabState>,
}

#[derive(Debug, PartialEq)]
struct TabState {
    text: String,
    path: Option<PathBuf>,
    label: Option<String>,
    dirty: bool,
    read_only: bool,
    encoding: Encoding,
    eol: LineEnding,
    selection: (usize, usize),
    undo: Vec<u8>,
    redo: Vec<u8>,
    bookmarks: BTreeSet<usize>,
    marks: Vec<Range<usize>>,
    revision: ContentRevision,
    session_id: uuid::Uuid,
    begin_select_marker: Option<usize>,
    transaction_open: bool,
}

pub(crate) fn edit_state(w: &Workspace) -> EditState {
    EditState {
        active: w.active,
        autosave_generation: w.autosave_generation,
        tabs: w
            .tabs
            .iter()
            .map(|tab| TabState {
                text: tab.buffer.rope.to_string(),
                path: tab.buffer.file_path.clone(),
                label: tab.buffer.new_label.as_ref().map(ToString::to_string),
                dirty: tab.buffer.dirty,
                read_only: tab.buffer.read_only,
                encoding: tab.buffer.encoding,
                eol: tab.buffer.line_ending,
                selection: (tab.selection.anchor, tab.selection.head),
                undo: serialize_undo_stack(&tab.undo_stack),
                redo: serialize_undo_stack(&tab.redo_stack),
                bookmarks: tab.buffer.bookmarks.clone(),
                marks: tab.buffer.marks.clone(),
                revision: tab.buffer.raw_revision.clone(),
                session_id: tab.session_id,
                begin_select_marker: tab.begin_select_marker,
                transaction_open: tab.edit_transaction.is_some(),
            })
            .collect(),
    }
}

pub(crate) fn clipboard(cx: &mut gpui::App) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

pub(crate) fn expected_paths() -> String {
    format!(
        "{}\r\nnew Ω 7\r\n{}\r\n{}\r\n",
        first_path().display(),
        second_path().display(),
        first_path().display()
    )
}

#[test]
fn clipboard_all_registered_actions_keep_duplicate_tabs_and_edit_state() {
    let _settings = crate::settings::override_for_test(Default::default());
    let _recorder = crate::macros::override_execution_for_test(false);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, sample_tabs());
    let before = visual.update(|_, cx| edit_state(workspace.read(cx)));
    visual.dispatch_action(EditCopyAllFilenames);
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert_eq!(
            clipboard(cx),
            "café.txt\r\nnew Ω 7\r\ncafé.txt\r\ncafé.txt\r\n"
        );
        assert_eq!(edit_state(workspace.read(cx)), before);
    });
    visual.dispatch_action(EditCopyAllFilePaths);
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert_eq!(clipboard(cx), expected_paths());
        assert_eq!(edit_state(workspace.read(cx)), before);
    });

    visual.update(|_, cx| {
        workspace.update(cx, |w, _| {
            w.tabs.rotate_left(2);
            w.active = 3;
        })
    });
    let reordered = visual.update(|_, cx| edit_state(workspace.read(cx)));
    visual.dispatch_action(EditCopyAllFilenames);
    visual.update(|_, cx| {
        assert_eq!(
            clipboard(cx),
            "café.txt\r\ncafé.txt\r\ncafé.txt\r\nnew Ω 7\r\n"
        );
        assert_eq!(edit_state(workspace.read(cx)), reordered);
    });
    visual.dispatch_action(EditCopyAllFilePaths);
    visual.update(|_, cx| {
        assert_eq!(
            clipboard(cx),
            format!(
                "{}\r\n{}\r\n{}\r\nnew Ω 7\r\n",
                second_path().display(),
                first_path().display(),
                first_path().display()
            )
        );
        assert_eq!(edit_state(workspace.read(cx)), reordered);
    });
    cx.quit();
}

#[test]
fn clipboard_all_empty_workspace_keeps_existing_clipboard() {
    let _settings = crate::settings::override_for_test(Default::default());
    let _recorder = crate::macros::override_execution_for_test(false);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, sample_tabs());
    visual.update(|window, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("keep 😀".into()));
        workspace.update(cx, |w, cx| {
            let tabs = std::mem::take(&mut w.tabs);
            w.copy_all_filenames(&EditCopyAllFilenames, window, cx);
            assert_eq!(clipboard(cx), "keep 😀");
            w.copy_all_file_paths(&EditCopyAllFilePaths, window, cx);
            assert_eq!(clipboard(cx), "keep 😀");
            w.tabs = tabs;
        });
    });
    cx.quit();
}

#[test]
fn clipboard_all_new_copy_retires_pending_bookmark_clipboard_work() {
    for kind in 0..3 {
        let _settings = crate::settings::override_for_test(Default::default());
        let _recorder = crate::macros::override_execution_for_test(false);
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, sample_tabs());
        let (release, gate) = futures::channel::oneshot::channel();
        visual.update_window_entity(&workspace, |w, _, _| {
            w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&"x\n".repeat(140_000));
            if kind == 2 {
                w.active_tab_mut().buffer.marks.clear();
                w.active_tab_mut().buffer.marks.push(0..1);
            }
            w.bookmark_copy_test_gate = Some(gate);
        });
        let before = visual.update(|_, cx| edit_state(workspace.read(cx)));
        visual.dispatch_action(CopyBookmarkedLines);
        visual.update(|_, cx| assert!(workspace.read(cx).pending_bookmark_copy.is_some()));
        match kind {
            0 => visual.dispatch_action(EditCopyAllFilenames),
            1 => visual.dispatch_action(EditCopyAllFilePaths),
            _ => visual.dispatch_action(MarkCopy),
        }
        visual.update(|_, cx| {
            assert!(workspace.read(cx).pending_bookmark_copy.is_none(), "new clipboard command must retire earlier work");
            assert_eq!(edit_state(workspace.read(cx)), before);
        });
        release.send(()).unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| {
            assert_eq!(clipboard(cx), match kind {
                0 => "café.txt\r\nnew Ω 7\r\ncafé.txt\r\ncafé.txt\r\n".into(),
                1 => expected_paths(),
                _ => "x\n".into(),
            });
            assert_eq!(edit_state(workspace.read(cx)), before);
        });
        cx.quit();
    }
}

#[test]
fn clipboard_all_untitled_labels_keep_stored_titles_until_an_edit() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        untitled_uses_first_line: true,
        ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _recorder = crate::macros::override_execution_for_test(false);
    let _clipboard = crate::clipboard_ownership::override_for_test();
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, sample_tabs());
    let expected = "café.txt\r\nnew Ω 7\r\ncafé.txt\r\ncafé.txt\r\n";
    let before = visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(super::tab_labels(&w.tabs, false).unwrap().as_deref(), Some(expected));
        edit_state(w)
    });
    visual.dispatch_action(EditCopyAllFilenames); visual.run_until_parked();
    visual.update(|_, cx| {
        assert_eq!(clipboard(cx), expected);
        assert_eq!(edit_state(workspace.read(cx)), before);
    });
    cx.quit();
}

#[test]
fn clipboard_all_crlf_terminated_and_empty_labels_follow_oracle_aggregation() {
    let _settings = crate::settings::override_for_test(Default::default());
    let mut tab = Tab::empty();
    tab.buffer.new_label = Some("終わり\r\n".into());
    assert_eq!(
        super::tab_labels(&[tab], false).unwrap(),
        Some("終わり\r\n".into())
    );
    assert_eq!(super::tab_labels(&[], true).unwrap(), None);
    let mut empty = Tab::empty();
    empty.buffer.new_label = Some("".into());
    assert_eq!(super::tab_labels(&[empty], false).unwrap(), None);
}
