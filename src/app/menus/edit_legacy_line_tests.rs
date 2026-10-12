// PowerEditor/src/NppCommands.cpp:933–1063,4413–4607; Sorters.h; and
// Notepad_plus.cpp:1759–1842 — installed legacy line-operation contracts.
use crate::{actions::*, buffer::{BlockSelection, ContentRevision, SavePoint}, macros,
    settings::Settings, tab::{Tab, TabViewMode}, workspace::Workspace};
use gpui::{Action, ClipboardItem, ElementInputHandler, Entity, InputHandler, MenuItem, Pixels, TestAppContext,
    VisualContext, VisualTestContext, WindowAppearance};
use std::{collections::BTreeMap, ops::Range};

struct Case { label: &'static str, id: &'static str, npp: &'static str, event: &'static str,
    source: &'static str, expected: &'static str }

const CASES: [Case; 12] = [
    Case { label: "Sort Lines Lexicographically Ascending", id: "EditSortLinesLexAsc", npp: "IDM_EDIT_SORTLINES_LEXICOGRAPHIC_ASCENDING", event: "SortLinesLexAsc", source: "c\nb\na", expected: "a\nb\nc" },
    Case { label: "Sort Lines Lexicographically Descending", id: "EditSortLinesLexDesc", npp: "IDM_EDIT_SORTLINES_LEXICOGRAPHIC_DESCENDING", event: "SortLinesLexDesc", source: "a\nb\nc", expected: "c\nb\na" },
    Case { label: "Sort Lines As Integers Ascending", id: "EditSortLinesIntAsc", npp: "IDM_EDIT_SORTLINES_INTEGER_ASCENDING", event: "SortLinesIntAsc", source: "10\n9\n-2\n01\n1", expected: "-2\n01\n1\n9\n10" },
    Case { label: "Sort Lines As Integers Descending", id: "EditSortLinesIntDesc", npp: "IDM_EDIT_SORTLINES_INTEGER_DESCENDING", event: "SortLinesIntDesc", source: "10\n9\n-2\n01\n1", expected: "10\n9\n1\n01\n-2" },
    Case { label: "Sort Lines As Decimals (Comma) Ascending", id: "EditSortLinesDecCommaAsc", npp: "IDM_EDIT_SORTLINES_DECIMALCOMMA_ASCENDING", event: "SortLinesDecCommaAsc", source: "1,5\nx\n-3\n1,25", expected: "x\n-3\n1,25\n1,5" },
    Case { label: "Sort Lines As Decimals (Comma) Descending", id: "EditSortLinesDecCommaDesc", npp: "IDM_EDIT_SORTLINES_DECIMALCOMMA_DESCENDING", event: "SortLinesDecCommaDesc", source: "1,5\nx\n-3\n1,25", expected: "1,5\n1,25\n-3\nx" },
    Case { label: "Sort Lines As Decimals (Dot) Ascending", id: "EditSortLinesDecDotAsc", npp: "IDM_EDIT_SORTLINES_DECIMALDOT_ASCENDING", event: "SortLinesDecDotAsc", source: "1.5\nx\n-3\n1.25", expected: "x\n-3\n1.25\n1.5" },
    Case { label: "Sort Lines As Decimals (Dot) Descending", id: "EditSortLinesDecDotDesc", npp: "IDM_EDIT_SORTLINES_DECIMALDOT_DESCENDING", event: "SortLinesDecDotDesc", source: "1.5\nx\n-3\n1.25", expected: "1.5\n1.25\n-3\nx" },
    Case { label: "Reverse Line Order", id: "EditReverseLineOrder", npp: "IDM_EDIT_SORTLINES_REVERSE_ORDER", event: "ReverseLineOrder", source: "c\nb\na", expected: "a\nb\nc" },
    Case { label: "Randomize Line Order", id: "EditRandomizeLineOrder", npp: "IDM_EDIT_SORTLINES_RANDOMLY", event: "RandomizeLineOrder", source: "c\nb\na", expected: "" },
    Case { label: "Remove Duplicate Lines", id: "EditRemoveDuplicateLines", npp: "IDM_EDIT_REMOVE_ANY_DUP_LINES", event: "RemoveDuplicateLines", source: "b\na\nb\nc", expected: "b\na\nc" },
    Case { label: "Remove Consecutive Duplicate Lines", id: "EditRemoveConsecutiveDuplicateLines", npp: "IDM_EDIT_REMOVE_CONSECUTIVE_DUP_LINES", event: "RemoveConsecutiveDuplicateLines", source: "b\nb\na\nc", expected: "b\na\nc" },
];

fn action(label: &str) -> Box<dyn Action> {
    super::line_operations_menu().items.into_iter().find_map(|item| match item {
        MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action.boxed_clone()),
        _ => None,
    }).expect("installed Line Operations action")
}

fn invoke(case: &Case, visual: &mut VisualTestContext) {
    visual.update(|window, cx| window.dispatch_action(action(case.label), cx));
    visual.run_until_parked();
}

fn with_source(source: &str, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _settings = crate::settings::override_for_test(Settings::default());
    let _recording = macros::override_execution_for_test(false);
    assert!(crate::recent::snapshot().is_empty());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(source);
        tab.buffer.line_ending = crate::text::line_ending::LineEnding::detect(source);
        tab.buffer.mark_saved();
        w.tabs = vec![tab]; w.active = 0; w.word_wrap = false;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked();
    visual.write_to_clipboard(ClipboardItem::new_string("clipboard sentinel".into()));
    test(&workspace, visual); cx.quit();
}

fn text(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> String {
    visual.update(|_, cx| workspace.read(cx).buffer().rope.to_string())
}

#[derive(PartialEq)]
struct HistoryOp { before: ContentRevision, after: ContentRevision, start: usize, removed: String, inserted: String, selection: (usize,usize) }

#[derive(PartialEq)]
enum HistoryState {
    Single(HistoryOp),
    Group(Vec<HistoryOp>,usize,usize),
    Prepared(usize,ContentRevision,ContentRevision),
}

fn history_state(entries: &[crate::buffer::UndoEntry]) -> Vec<HistoryState> {
    let op = |op: &crate::buffer::EditOp| HistoryOp { before:op.before_revision.clone(),after:op.after_revision.clone(),start:op.byte_start,
        removed:op.removed.clone(),inserted:op.inserted.clone(),selection:(op.sel_before_anchor,op.sel_before_head) };
    entries.iter().map(|entry| match entry {
        crate::buffer::UndoEntry::Single(edit) => HistoryState::Single(op(edit)),
        crate::buffer::UndoEntry::Group { ops,sel_before_anchor,sel_before_head } =>
            HistoryState::Group(ops.iter().map(op).collect(),*sel_before_anchor,*sel_before_head),
        crate::buffer::UndoEntry::Prepared(group) => HistoryState::Prepared(std::sync::Arc::as_ptr(group) as usize,
            group.expected(false).clone(),group.expected(true).clone()),
    }).collect()
}

#[derive(PartialEq)]
struct State {
    text: String, selection: (usize, usize, bool), block: Option<(usize, usize, usize, usize, bool)>,
    revision: ContentRevision, saved: Option<SavePoint>, dirty: bool, undo: usize, redo: usize,
    bookmarks: Vec<usize>, marks: Vec<Range<usize>>, marker: Option<usize>,
    desired_col_x: Option<Pixels>, encoding: crate::text::encoding::Encoding,
    eol: crate::text::line_ending::LineEnding, composing: bool,
    loading: Option<(u64, u64, bool)>, transaction: Option<(usize, usize)>,
    read_only: bool, view_mode: TabViewMode, active_window: bool, closed: bool,
    path: Option<std::path::PathBuf>,
    history: (Vec<HistoryState>,Vec<HistoryState>), rich: Option<(ContentRevision,String)>,
    autosave: u64, post_edit: u64, clipboard: Option<String>,
}

fn state(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> State {
    let clipboard = visual.read_from_clipboard().and_then(|item| item.text());
    visual.update(|window, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[0]; let buffer = &tab.buffer;
        State { text: buffer.rope.to_string(), selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            block: tab.block_selection.map(|b| (b.anchor_line, b.head_line, b.anchor_col, b.head_col, b.is_dragging)),
            revision: buffer.raw_revision.clone(), saved: buffer.savepoint.clone(), dirty: buffer.dirty,
            undo: tab.undo_stack.len(), redo: tab.redo_stack.len(), bookmarks: buffer.bookmarks.iter().copied().collect(),
            marks: buffer.marks.clone(), marker: tab.begin_select_marker,
            desired_col_x: tab.desired_col_x, encoding: buffer.encoding, eol: buffer.line_ending,
            composing: w.is_composing_text(window),
            loading: buffer.loading.as_ref().map(|load| (load.bytes_read, load.total_bytes,
                load.cancel.load(std::sync::atomic::Ordering::Relaxed))),
            transaction: tab.edit_transaction.as_ref().map(|group| (group.depth, group.ops.len())),
            read_only: buffer.read_only, view_mode: tab.view_mode, active_window: window.is_window_active(), closed: w.window_closed,
            path: buffer.file_path.clone(),
            history: (history_state(&tab.undo_stack),history_state(&tab.redo_stack)),
            rich: buffer.rich_doc.as_ref().map(|payload| (payload.revision.clone(),format!("{payload:?}"))),
            autosave: w.autosave_generation, post_edit: w.post_edit_generation, clipboard }
    })
}

#[test]
fn legacy_lines_installed_catalog_routes_and_ordinary_undo_controls() {
    for (index, case) in CASES.iter().enumerate() {
        let catalog = crate::shortcuts::catalog();
        let entry = catalog.iter().find(|entry| entry.id == case.id).expect("legacy CATALOG row");
        assert_eq!(entry.npp, Some(case.npp)); assert_eq!((entry.action)().name(), action(case.label).name());
        if index == 9 { continue; }
        with_source(case.source, |workspace, visual| {
            invoke(case, visual);
            assert_eq!(text(workspace, visual), case.expected, "{}", case.id);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), case.source);
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), case.expected);
        });
    }
}

#[test]
fn legacy_uniform_eol_transforms_preserve_encoding_metadata_and_undo_bytes() {
    use crate::text::encoding::{Encoding, encode_checked};
    for ending in ["\n", "\r\n", "\r"] {
        for case in CASES.iter().filter(|case| case.id != "EditRandomizeLineOrder") {
            let source = case.source.replace('\n', ending); let expected = case.expected.replace('\n', ending);
            with_source(&source, |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, _| w.tabs[0].buffer.encoding = Encoding::Utf16Be);
                let before = state(workspace, visual); let original_bytes = encode_checked(&source, before.encoding).unwrap();
                invoke(case, visual); let after = state(workspace, visual);
                assert_eq!(after.text, expected, "{} {ending:?}", case.id);
                assert_eq!((after.encoding, after.eol), (before.encoding, before.eol));
                assert_eq!(after.undo, 1);
                visual.dispatch_action(EditUndo); visual.run_until_parked();
                assert_eq!(encode_checked(&text(workspace, visual), before.encoding).unwrap(), original_bytes);
                visual.dispatch_action(EditRedo); visual.run_until_parked();
                assert_eq!(text(workspace, visual), expected);
            });
        }
    }
}

#[test]
fn legacy_selected_next_row_start_excludes_that_row() {
    for ending in ["\n", "\r\n", "\r"] {
        for index in [0, 2, 4, 6, 8] {
            let source = format!("2{ending}1{ending}0"); let expected = format!("1{ending}2{ending}0");
            with_source(&source, |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].selection.anchor = 0; w.tabs[0].selection.head = 2 * (1 + ending.len()); });
                invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected, "{}", CASES[index].id);
            });
        }
    }
}

#[test]
fn legacy_reverse_includes_the_final_empty_row_at_eof() {
    for ending in ["\n", "\r\n", "\r"] {
        let source = format!("a{ending}b{ending}"); let expected = format!("{ending}b{ending}a");
        with_source(&source, |workspace, visual| { invoke(&CASES[8], visual); assert_eq!(text(workspace, visual), expected); });
    }
}

#[test]
fn legacy_integer_digit_runs_are_not_limited_to_machine_integer_width() {
    let longer = "9".repeat(80); let shorter = "1".repeat(79);
    let source = format!("{longer}\n-{longer}\n{shorter}");
    for index in [2, 3] {
        let expected = if index == 2 { format!("-{longer}\n{shorter}\n{longer}") } else { format!("{longer}\n{shorter}\n-{longer}") };
        with_source(&source, |workspace, visual| { invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected); });
    }
}

#[test]
fn legacy_integer_non_total_comparison_has_deterministic_permutation_policy() {
    for prefix in ["", "0"] {
        let source = format!("{prefix}.\n{prefix}-1\n{prefix}-.");
        let expected = format!("{prefix}-.\n{prefix}-1\n{prefix}.");
        for _ in 0..2 {
            with_source(&source, |workspace, visual| { invoke(&CASES[2], visual); assert_eq!(text(workspace, visual), expected); });
        }
    }
}

#[test]
fn legacy_randomize_preserves_the_rows_and_one_undo_when_order_changes() {
    let source = (0..64).map(|i| format!("row{i}é")).collect::<Vec<_>>().join("\r\n");
    with_source(&source, |workspace, visual| {
        invoke(&CASES[9], visual); let after = text(workspace, visual);
        let mut before_rows = source.split("\r\n").collect::<Vec<_>>(); let mut after_rows = after.split("\r\n").collect::<Vec<_>>();
        before_rows.sort(); after_rows.sort(); assert_eq!(after_rows, before_rows);
        let expected_undo = usize::from(after != source);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), expected_undo));
        if expected_undo != 0 {
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), source);
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), after);
        }
    });
}

#[test]
fn legacy_mixed_eol_merge_refusal_preserves_all_state() {
    with_source("a\rb\n\n", |workspace, visual| {
        let before = state(workspace, visual); invoke(&CASES[0], visual);
        assert!(state(workspace, visual) == before);
        visual.update(|_, cx| assert!(workspace.read(cx).status_message.as_deref().unwrap().contains("merge")));
    });
}

#[test]
fn legacy_hostile_selection_bytes_still_operate_on_physical_rows() {
    for (anchor, head) in [(1, usize::MAX), (usize::MAX, 1)] {
        with_source("中1\né2", |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].selection.anchor = anchor; w.tabs[0].selection.head = head; });
            invoke(&CASES[0], visual); assert_eq!(text(workspace, visual), "é2\n中1");
            visual.update(|_, cx| { let w = workspace.read(cx); assert!(w.selection().head <= w.buffer().rope.len_bytes()); });
        });
    }
}

#[test]
fn legacy_embedded_nul_policy_preserves_the_entire_row() {
    for (index, source, expected) in [(0, "é\nz\0tail", "z\0tail\né"),
        (6, "2\n1\0tail", "1\0tail\n2"), (8, "é\nz\0tail", "z\0tail\né")] {
        with_source(source, |workspace, visual| { invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected); });
    }
}

fn macro_contract(index: usize) {
    let case = &CASES[index];
    with_source(case.source, |_, visual| {
        visual.dispatch_action(MacroStartRecording); invoke(case, visual);
        visual.dispatch_action(MacroStopRecording);
        let events = macros::current();
        assert_eq!(events.len(), 1, "unsupported macro contract: {}", case.id);
        assert_eq!(format!("{:?}", events[0]), case.event);
        let directory = crate::persist::TempDir::new("legacy-line-macro");
        let saved = macros::saved_round_trip_for_test(&directory.join("macros.toml"),
            &BTreeMap::from([("line operation".into(), events)])).unwrap();
        assert_eq!(format!("{:?}", saved.get("line operation").unwrap()[0]), case.event);
    });
}

macro_rules! caller_cases { ($helper:ident; $($name:ident => $index:expr),* $(,)?) => { $(#[test] fn $name() { $helper($index); })* }; }
caller_cases! { macro_contract;
    legacy_macro_lex_asc_contract => 0, legacy_macro_lex_desc_contract => 1,
    legacy_macro_int_asc_contract => 2, legacy_macro_int_desc_contract => 3,
    legacy_macro_comma_asc_contract => 4, legacy_macro_comma_desc_contract => 5,
    legacy_macro_dot_asc_contract => 6, legacy_macro_dot_desc_contract => 7,
    legacy_macro_reverse_contract => 8, legacy_macro_random_contract => 9,
    legacy_macro_dedupe_contract => 10, legacy_macro_consecutive_contract => 11,
}

fn utf16_contract(index: usize) {
    let source = "\u{e000}2\n\u{10000}2";
    let expected = if index.is_multiple_of(2) { "\u{10000}2\n\u{e000}2" } else { source };
    with_source(source, |workspace, visual| { invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected, "{}", CASES[index].id); });
}
caller_cases! { utf16_contract;
    legacy_lex_asc_windows_utf16 => 0, legacy_lex_desc_windows_utf16 => 1,
    legacy_int_asc_windows_utf16 => 2, legacy_int_desc_windows_utf16 => 3,
}

fn decimal_prefix_contract(index: usize) {
    let source = if index < 6 { "2\n1,2,3\n0" } else { "2\n1.2.3\n0" };
    let expected = match index {
        4 => "0\n1,2,3\n2", 5 => "2\n1,2,3\n0", 6 => "0\n1.2.3\n2", _ => "2\n1.2.3\n0",
    };
    with_source(source, |workspace, visual| { invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected, "{}", CASES[index].id); });
}
caller_cases! { decimal_prefix_contract;
    legacy_comma_asc_valid_numeric_prefix => 4, legacy_comma_desc_valid_numeric_prefix => 5,
    legacy_dot_asc_valid_numeric_prefix => 6, legacy_dot_desc_valid_numeric_prefix => 7,
}

fn decimal_refusal(index: usize, bad: &str) {
    let source = format!("2\n{bad}\n1");
    with_source(&source, |workspace, visual| {
        let before = state(workspace, visual); invoke(&CASES[index], visual);
        assert!(state(workspace, visual) == before, "numeric refusal changed state: {}", CASES[index].id);
        visual.update(|_, cx| assert!(workspace.read(cx).status_message.is_some(), "numeric error must be visible"));
    });
}
fn decimal_error_contract(index: usize) { decimal_refusal(index, "--1"); }
fn decimal_overflow_contract(index: usize) { decimal_refusal(index, &"9".repeat(400)); }
caller_cases! { decimal_error_contract;
    legacy_comma_asc_invalid_numeric_refusal => 4, legacy_comma_desc_invalid_numeric_refusal => 5,
    legacy_dot_asc_invalid_numeric_refusal => 6, legacy_dot_desc_invalid_numeric_refusal => 7,
}
caller_cases! { decimal_overflow_contract;
    legacy_comma_asc_numeric_overflow_refusal => 4, legacy_comma_desc_numeric_overflow_refusal => 5,
    legacy_dot_asc_numeric_overflow_refusal => 6, legacy_dot_desc_numeric_overflow_refusal => 7,
}

#[test]
fn legacy_consecutive_dedupe_removes_the_matching_unterminated_eof_row() {
    for ending in ["\n", "\r\n", "\r"] {
        let source = format!("a{ending}a");
        with_source(&source, |workspace, visual| { invoke(&CASES[11], visual); assert_eq!(text(workspace, visual), "a"); });
    }
}

fn selection_contract(index: usize) {
    let source = "prefix\n2\n1\nsuffix";
    with_source(source, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].selection.anchor = 10; w.tabs[0].selection.head = 8; });
        invoke(&CASES[index], visual);
        let expected = if index.is_multiple_of(2) { "prefix\n1\n2\nsuffix" } else { source };
        assert_eq!(text(workspace, visual), expected);
        visual.update(|_, cx| { let selection = workspace.read(cx).selection(); assert_eq!((selection.anchor, selection.head), (7, 10), "{}", CASES[index].id); });
    });
}
caller_cases! { selection_contract;
    legacy_lex_asc_forward_row_selection => 0, legacy_lex_desc_forward_row_selection => 1,
    legacy_int_asc_forward_row_selection => 2, legacy_int_desc_forward_row_selection => 3,
    legacy_comma_asc_forward_row_selection => 4, legacy_comma_desc_forward_row_selection => 5,
    legacy_dot_asc_forward_row_selection => 6, legacy_dot_desc_forward_row_selection => 7,
    legacy_reverse_forward_row_selection => 8,
}

fn rectangle_contract(index: usize) {
    let source = if index < 2 { "prefix\nx a\nz b\ny a\nsuffix" } else { "prefix\nx 1\nz 2\ny 1\nsuffix" };
    let expected = match index {
        0 => "prefix\nx a\ny a\nz b\nsuffix", 1 => "prefix\nz b\nx a\ny a\nsuffix",
        2 | 4 | 6 => "prefix\nx 1\ny 1\nz 2\nsuffix", _ => "prefix\nz 2\nx 1\ny 1\nsuffix",
    };
    with_source(source, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].block_selection = Some(BlockSelection {
            anchor_line: 1, head_line: 3, anchor_col: 2, head_col: 3, is_dragging: false,
        }); });
        invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), expected, "{}", CASES[index].id);
    });
}
caller_cases! { rectangle_contract;
    legacy_lex_asc_rectangle_stable_keys => 0, legacy_lex_desc_rectangle_stable_keys => 1,
    legacy_int_asc_rectangle_stable_keys => 2, legacy_int_desc_rectangle_stable_keys => 3,
    legacy_comma_asc_rectangle_stable_keys => 4, legacy_comma_desc_rectangle_stable_keys => 5,
    legacy_dot_asc_rectangle_stable_keys => 6, legacy_dot_desc_rectangle_stable_keys => 7,
}

fn rectangle_dedupe_contract(index: usize) {
    with_source("outside\noutside\nk\nk\nend", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].block_selection = Some(BlockSelection {
            anchor_line: 2, head_line: 3, anchor_col: 0, head_col: 1, is_dragging: false,
        }); });
        invoke(&CASES[index], visual); assert_eq!(text(workspace, visual), "outside\noutside\nk\nend");
    });
}
caller_cases! { rectangle_dedupe_contract;
    legacy_dedupe_rectangle_rows => 10, legacy_consecutive_rectangle_rows => 11,
}

#[test]
fn legacy_reverse_rectangle_permutates_only_its_corner_rows() {
    with_source("outside\na\nb\nc\nend", |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].block_selection = Some(BlockSelection {
            anchor_line: 1, head_line: 3, anchor_col: 0, head_col: 1, is_dragging: false,
        }); });
        invoke(&CASES[8], visual); assert_eq!(text(workspace, visual), "outside\nc\nb\na\nend");
    });
}

fn noop_contract(index: usize) {
    with_source("one", |workspace, visual| { let before = state(workspace, visual); invoke(&CASES[index], visual); assert!(state(workspace, visual) == before, "{}", CASES[index].id); });
}
caller_cases! { noop_contract;
    legacy_lex_asc_single_row_noop => 0, legacy_lex_desc_single_row_noop => 1,
    legacy_int_asc_single_row_noop => 2, legacy_int_desc_single_row_noop => 3,
    legacy_comma_asc_single_row_noop => 4, legacy_comma_desc_single_row_noop => 5,
    legacy_dot_asc_single_row_noop => 6, legacy_dot_desc_single_row_noop => 7,
    legacy_reverse_single_row_noop => 8, legacy_random_single_row_noop => 9,
    legacy_dedupe_single_row_noop => 10, legacy_consecutive_single_row_noop => 11,
}

fn refusal_contract(context: usize) { refusal_cases(context, &CASES.iter().collect::<Vec<_>>()); }

fn refusal_cases(context: usize, cases: &[&Case]) {
    for case in cases {
        let directory = crate::persist::TempDir::new("legacy-line-context");
        with_source(case.source, |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, cx| {
                w.replace_byte_range(0..0, "seed", cx); w.replace_byte_range(0..0, "redo", cx);
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let tab = &workspace.read(cx).tabs[0]; assert_eq!((tab.undo_stack.len(), tab.redo_stack.len()), (1, 1)); });
            visual.update_window_entity(workspace, |w, window, cx| {
                w.tabs[0].selection.anchor = w.buffer().rope.len_bytes(); w.tabs[0].selection.head = 1;
                w.tabs[0].buffer.bookmarks.insert(0); w.tabs[0].buffer.marks.push(0..1); w.tabs[0].begin_select_marker = Some(1);
                w.tabs[0].desired_col_x = Some(gpui::px(20.));
                match context {
                    0 => w.tabs[0].buffer.read_only = true,
                    1 => {
                        w.tabs[0].buffer.file_path=Some(directory.join("neutral.md"));
                        w.tabs[0].view_mode = TabViewMode::MarkdownPreview;
                    }
                    2 => window.focus(&w.find_focus, cx), 3 => w.window_closed = true,
                    5 => {
                        w.tabs[0].buffer.file_path=Some(directory.join("neutral.rtf"));
                        w.tabs[0].buffer.rich_doc=Some(crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi neutral preview}".to_vec()).unwrap());
                        w.tabs[0].view_mode = TabViewMode::RichDocView;
                        let focus=w.ensure_rich_doc_body_focus(cx); window.focus(&focus,cx);
                    }
                    6 => {
                        w.tabs[0].buffer.file_path = Some(directory.join("neutral.csv"));
                        w.tabs[0].view_mode = TabViewMode::CsvPreview;
                        crate::ui::csv_doc::actions::ensure_model(w,window,cx);
                        let focus = w.csv_models.get(&w.tabs[0].session_id).unwrap().grid_focus.clone().unwrap();
                        window.focus(&focus,cx);
                    }
                    8 => w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 1, total_bytes: 20, cancel: Default::default() }),
                    9 => w.begin_edit_group(), _ => {}
                }
            });
            if matches!(context,1|5|6) { visual.run_until_parked(); }
            if context == 7 { visual.deactivate_window(); }
            if context == 4 {
                visual.update(|window, cx| { let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                    handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                    assert!(workspace.read(cx).is_composing_text(window)); });
            }
            visual.dispatch_action(MacroStartRecording);
            let before = state(workspace, visual); invoke(case, visual);
            assert!(state(workspace, visual) == before, "refusal changed state: {} context={context}", case.id);
            visual.dispatch_action(MacroStopRecording);
            assert!(macros::current().is_empty(), "refused command was recorded: {} context={context}", case.id);
        });
    }
}
caller_cases! { refusal_contract;
    legacy_readonly_refusal_preserves_state => 0, legacy_preview_refusal_preserves_state => 1,
    legacy_field_focus_refusal_preserves_state => 2, legacy_closed_refusal_preserves_state => 3,
    legacy_ime_refusal_preserves_state => 4,
    legacy_rich_view_refusal_preserves_state => 5, legacy_csv_view_refusal_preserves_state => 6,
    legacy_inactive_refusal_preserves_state => 7, legacy_loading_refusal_preserves_state => 8,
    legacy_open_transaction_refusal_preserves_state => 9,
}

#[path = "edit_legacy_pipeline_tests.rs"]
mod pipeline_tests;

#[path = "edit_legacy_numeric_pipeline_tests.rs"]
mod numeric_pipeline_tests;

#[path = "edit_legacy_dedupe_pipeline_tests.rs"]
mod dedupe_pipeline_tests;

#[path = "edit_legacy_fidelity_tests.rs"]
mod fidelity_tests;

#[path = "edit_legacy_ownership_tests.rs"]
mod ownership_tests;

#[path = "edit_legacy_large_tests.rs"]
mod large_tests;
