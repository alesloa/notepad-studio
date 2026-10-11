// Scintilla Document.cxx:169,263–287 and CellBuffer.cxx:578–593 — default
// line ends are CR/LF, even when a lexer supports Unicode line endings.
use super::Workspace;
use crate::{actions::*, buffer::BlockSelection, macros, settings::Settings,
    tab::Tab, text::line_ending::LineEnding};
use gpui::{Action, Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

fn with_source(text: &str, eol: LineEnding, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _recording = macros::override_execution_for_test(false);
    let _settings = crate::settings::override_for_test(Settings::default());
    assert!(crate::recent::snapshot().is_empty());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.line_ending = eol;
        tab.buffer.mark_saved();
        tab.selection.collapse_to(0);
        w.tabs = vec![tab]; w.active = 0; w.word_wrap = false;
        let (sink, _) = std::sync::mpsc::channel();
        w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| {
        window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked(); test(&workspace, visual); cx.quit();
}

fn caret(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, byte: usize) {
    visual.update_window_entity(workspace, |w, _, cx| {
        w.tabs[0].selection.collapse_to(byte); w.commit_caret(cx);
    });
    visual.run_until_parked();
}

fn text(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> String {
    visual.update(|_, cx| workspace.read(cx).buffer().rope.to_string())
}

fn movement_separator(separator: &str) {
    let source = format!("x{separator}y\nz");
    with_source(&source, LineEnding::Lf, |workspace, visual| {
        visual.dispatch_action(MoveDown); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source);
            assert_eq!(w.selection().head, source.len() - 1, "MoveDown must step over one physical row: {separator:?}");
        });
    });
}

fn bookmark_separator(separator: &str) {
    let source = format!("x{separator}y\nz");
    with_source(&source, LineEnding::Lf, |workspace, visual| {
        let head = 1 + separator.len(); caret(workspace, visual, head);
        visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source);
            assert_eq!(w.selection().head, head);
            assert_eq!(w.buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![0],
                "BookmarkToggle must anchor the containing physical row: {separator:?}");
        });
    });
}

fn delete_separator(separator: &str) {
    let source = format!("x{separator}y\nz");
    with_source(&source, LineEnding::Lf, |workspace, visual| {
        let head = 1 + separator.len(); caret(workspace, visual, head);
        visual.dispatch_action(EditDeleteLine); visual.run_until_parked();
        let deleted = text(workspace, visual);
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        assert_eq!(text(workspace, visual), source);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, head));
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        assert_eq!(text(workspace, visual), deleted);
        assert_eq!(deleted, "z", "EditDeleteLine must remove the containing physical row: {separator:?}");
    });
}

fn sort_action(mode: usize) -> Box<dyn Action> {
    match mode {
        0 => Box::new(EditSortLinesLexIgnoreCaseAsc),
        1 => Box::new(EditSortLinesLexIgnoreCaseDesc),
        2 => Box::new(EditSortLinesLengthAsc),
        3 => Box::new(EditSortLinesLengthDesc),
        4 => Box::new(EditSortLinesLocaleAsc),
        5 => Box::new(EditSortLinesLocaleDesc),
        _ => panic!("unknown sort fixture"),
    }
}

fn rectangle_sorts(separator: &str, eol: LineEnding) {
    let ending = eol.as_str();
    let first = format!("p B{separator}BBB");
    let source = format!("{first}{ending}q A{ending}r AA");
    let ascending = format!("q A{ending}r AA{ending}{first}");
    let descending = format!("{first}{ending}r AA{ending}q A");
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for mode in 0..if cfg!(windows) { 6 } else { 4 } {
        with_source(&source, eol, |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                w.tabs[0].block_selection = Some(BlockSelection {
                    anchor_line: 0, head_line: 2, anchor_col: 2, head_col: 2, is_dragging: false,
                });
            });
            visual.update(|window, cx| window.dispatch_action(sort_action(mode), cx));
            visual.run_until_parked();
            let sorted = text(workspace, visual);
            let history = visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.pending_prepared_edit.is_none());
                let block = w.block_selection().expect("rectangle preserved");
                assert_eq!((block.anchor_line, block.head_line, block.anchor_col, block.head_col), (0, 2, 2, 2));
                w.tabs[0].undo_stack.len()
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), source);
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), sorted);
            println!("physical rectangle separator={separator:?} eol={ending:?} mode={mode} actual={sorted:?} undo={history}");
            actual.push((sorted, history));
            expected.push((if mode.is_multiple_of(2) { ascending.clone() } else { descending.clone() }, 1));
        });
    }
    assert_eq!(actual, expected, "registered rectangle sorts must include physical rows 0..2: {separator:?}");
}

macro_rules! separators {
    ($check:ident; $($name:ident, $separator:literal;)*) => {
        $(#[test] fn $name() { $check($separator); })*
    };
}

separators!(movement_separator;
    movement_vt, "\u{b}"; movement_ff, "\u{c}"; movement_nel, "\u{85}";
    movement_ls, "\u{2028}"; movement_ps, "\u{2029}";);
separators!(bookmark_separator;
    bookmark_vt, "\u{b}"; bookmark_ff, "\u{c}"; bookmark_nel, "\u{85}";
    bookmark_ls, "\u{2028}"; bookmark_ps, "\u{2029}";);
separators!(delete_separator;
    delete_vt, "\u{b}"; delete_ff, "\u{c}"; delete_nel, "\u{85}";
    delete_ls, "\u{2028}"; delete_ps, "\u{2029}";);

fn rectangle_separator(separator: &str) { rectangle_sorts(separator, LineEnding::Lf); }
separators!(rectangle_separator;
    rectangle_vt, "\u{b}"; rectangle_ff, "\u{c}"; rectangle_nel, "\u{85}";
    rectangle_ls, "\u{2028}"; rectangle_ps, "\u{2029}";);

const EOLS: [LineEnding; 3] = [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr];

#[test]
fn movement_crlf_lf_cr_and_eof_controls() {
    for eol in EOLS {
        let source = format!("x{}z", eol.as_str());
        with_source(&source, eol, |workspace, visual| {
            visual.dispatch_action(MoveDown); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, source.len() - 1));
            visual.dispatch_action(MoveDown); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, source.len()));
            assert_eq!(text(workspace, visual), source);
        });
    }
    for source in ["", "x", "x\n", "x\n\n"] {
        with_source(source, LineEnding::Lf, |workspace, visual| {
            caret(workspace, visual, source.len());
            visual.dispatch_action(MoveDown); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, source.len()));
            assert_eq!(text(workspace, visual), source);
        });
    }
}

#[test]
fn bookmark_crlf_lf_cr_and_eof_controls() {
    for eol in EOLS {
        let source = format!("xy{}z", eol.as_str());
        with_source(&source, eol, |workspace, visual| {
            caret(workspace, visual, 1);
            visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
            caret(workspace, visual, source.len());
            visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![0, source.len() - 1]));
            assert_eq!(text(workspace, visual), source);
        });
    }
    for source in ["", "x", "x\n", "x\n\n"] {
        with_source(source, LineEnding::Lf, |workspace, visual| {
            caret(workspace, visual, source.len());
            visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
            let start = source.rfind('\n').map_or(0, |byte| byte + 1);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![start]));
            assert_eq!(text(workspace, visual), source);
        });
    }
}

#[test]
fn delete_crlf_lf_cr_and_empty_controls() {
    for eol in EOLS {
        let source = format!("xy{}z", eol.as_str());
        with_source(&source, eol, |workspace, visual| {
            caret(workspace, visual, 1);
            visual.dispatch_action(EditDeleteLine); visual.run_until_parked();
            assert_eq!(text(workspace, visual), "z");
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), source);
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), "z");
        });
    }
    with_source("", LineEnding::Lf, |workspace, visual| {
        visual.dispatch_action(EditDeleteLine); visual.run_until_parked();
        assert_eq!(text(workspace, visual), "");
        visual.update(|_, cx| assert!(workspace.read(cx).tabs[0].undo_stack.is_empty()));
    });
}

#[test]
fn rectangle_crlf_lf_cr_controls() {
    for eol in EOLS { rectangle_sorts("", eol); }
}

fn search_file(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, path: &std::path::Path, query: &str) {
    visual.update_window_entity(workspace, |w, window, cx| {
        w.tabs[0].buffer.file_path = Some(path.to_path_buf());
        w.find_in_files.query = query.into();
        w.find_in_files.filter = "*.txt".into();
        w.find_in_files.directory_text = path.parent().unwrap().display().to_string();
        w.find_in_files.match_case = true;
        w.find_in_files.regex = true;
        // The actual Find-in-Files button callback (fif_tab.rs:154).
        w.find_in_files_run_action(&FindInFilesRun, window, cx);
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        let state = &workspace.read(cx).find_in_files;
        assert!(!state.searching);
        assert!(state.error.is_none(), "{:?}", state.error);
    });
}

#[test]
fn find_result_rows_keep_unicode_body_and_navigate_to_the_match() {
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for separator in ["\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"] {
        let source = format!("x{separator}y{separator}");
        let directory = crate::persist::TempDir::new("physical-fif-body");
        let path = directory.join("fixture.txt"); std::fs::write(&path, &source).unwrap();
        with_source(&source, LineEnding::Lf, |workspace, visual| {
            search_file(workspace, visual, &path, "y");
            let row = visual.update(|_, cx| {
                let results = &workspace.read(cx).find_in_files.results;
                assert_eq!(results.len(), 1);
                let result = &results[0];
                (result.line, result.line_text.clone(), result.match_in_line.clone())
            });
            visual.update_window_entity(workspace, |w, _, cx| w.jump_to_find_result(0, cx));
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.selection().ordered(), 1 + separator.len()..2 + separator.len());
                assert_eq!(w.buffer().rope.to_string(), source);
                assert!(w.tabs[0].undo_stack.is_empty());
            });
            println!("physical find-result separator={separator:?} actual={row:?}");
            actual.push(row);
            expected.push((0, source.clone(), 1 + separator.len()..2 + separator.len()));
        });
    }
    assert_eq!(actual, expected, "Find All must use the editor's physical row coordinates and complete body");
}

#[test]
fn find_result_can_select_a_unicode_separator_at_eof() {
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for separator in ["\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"] {
        let source = format!("x{separator}y{separator}");
        let directory = crate::persist::TempDir::new("physical-fif-eof");
        let path = directory.join("fixture.txt"); std::fs::write(&path, &source).unwrap();
        with_source(&source, LineEnding::Lf, |workspace, visual| {
            let query = format!("\\x{{{:x}}}", separator.chars().next().unwrap() as u32);
            search_file(workspace, visual, &path, &query);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).find_in_files.results.len(), 2));
            visual.update_window_entity(workspace, |w, _, cx| w.jump_to_find_result(1, cx));
            visual.run_until_parked();
            let selection = visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), source);
                assert!(w.tabs[0].undo_stack.is_empty());
                w.selection().ordered()
            });
            println!("physical EOF find-result separator={separator:?} actual={selection:?}");
            actual.push(selection);
            expected.push(source.len() - separator.len()..source.len());
        });
    }
    assert_eq!(actual, expected, "A Unicode body character is selectable at EOF");
}

fn saved_bytes(buffer: &crate::buffer::Buffer) -> Vec<u8> {
    let bytes = crate::text::line_ending::rope_to_bytes(&buffer.rope, buffer.line_ending);
    let source = String::from_utf8(bytes).expect("rope serializes as UTF8");
    crate::text::encoding::encode_checked(&source, buffer.encoding).expect("fixture is representable")
}

#[test]
fn all_encodings_and_eols_open_save_and_action_undo_keep_exact_bytes() {
    use crate::text::encoding::{self, Encoding};
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        let body = if encoding == Encoding::Latin1 { "é€\u{b}\u{c}body" }
            else { "é中😀\u{b}\u{c}\u{85}\u{2028}\u{2029}\u{feff}body" };
        for eol in EOLS {
            let ending = eol.as_str();
            let source = format!("{body}{ending}tail{ending}");
            let raw = encoding::encode_checked(&source, encoding).unwrap();
            let directory = crate::persist::TempDir::new("physical-encoded-source");
            let path = directory.join("fixture.txt"); std::fs::write(&path, &raw).unwrap();
            let buffer = crate::io::read_buffer(&path).unwrap();
            assert_eq!((buffer.encoding, buffer.line_ending), (encoding, eol));
            assert_eq!(buffer.rope.len_lines(), 3);
            assert_eq!(saved_bytes(&buffer), raw);
            let normalized = buffer.rope.to_string();
            with_source("", LineEnding::Lf, |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, cx| {
                    w.tabs[0] = Tab::with_buffer(buffer); w.commit_caret(cx);
                });
                visual.run_until_parked();
                visual.dispatch_action(FileSave); visual.run_until_parked();
                assert_eq!(std::fs::read(&path).unwrap(), raw);
                visual.dispatch_action(EditDuplicateCurrentLine); visual.run_until_parked();
                visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
                visual.dispatch_action(FileSave); visual.run_until_parked();
                let duplicated = encoding::encode_checked(&format!("{body}{ending}{body}{ending}tail{ending}"), encoding).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), duplicated);
                visual.dispatch_action(EditUndo); visual.run_until_parked();
                assert_eq!(text(workspace, visual), normalized);
                visual.dispatch_action(FileSave); visual.run_until_parked();
                assert_eq!(std::fs::read(&path).unwrap(), raw);
                visual.dispatch_action(EditRedo); visual.run_until_parked();
                visual.dispatch_action(FileSave); visual.run_until_parked();
                assert_eq!(std::fs::read(&path).unwrap(), duplicated);
            });
        }
    }
    assert!(encoding::encode_checked("\u{85}\u{2028}\u{2029}", Encoding::Latin1).is_err(),
        "unrepresentable ANSI body characters must not be silently replaced");
}

#[test]
fn old_session_byte_offsets_floor_bookmarks_and_replay_legacy_history() {
    use crate::buffer::{ContentRevision, EditOp, UndoEntry};
    let base = "x\u{b}y\u{c}z\u{85}q\u{2028}r\u{2029}s\nlast";
    let source = format!("{base}!");
    let directory = crate::persist::TempDir::new("physical-old-session");
    let path = directory.join("fixture.txt"); std::fs::write(&path, &source).unwrap();
    let mut tab = Tab::with_buffer(crate::io::read_buffer(&path).unwrap());
    tab.selection.anchor = source.len(); tab.selection.head = 2;
    let mut entry = crate::session::snapshot_tab_with_policy(&tab, crate::session::BakPolicy::UntitledOnly);
    let last_row = source.rfind('\n').unwrap() + 1;
    entry.bookmarks = vec![0, 2, 4, 7, 11, 15, last_row];
    let legacy = toml::to_string(&entry).unwrap();
    let entry: crate::session::SessionTab = toml::from_str(&legacy).unwrap();
    assert_eq!((entry.cursor_anchor, entry.cursor_head), (source.len(), 2));
    let history = vec![UndoEntry::Single(EditOp {
        before_revision: ContentRevision::default(), after_revision: ContentRevision::default(),
        byte_start: base.len(), removed: String::new(), inserted: "!".into(),
        sel_before_anchor: base.len(), sel_before_head: 2,
    })];
    std::fs::write(directory.join(&format!("{}.undo", entry.id)), crate::buffer::serialize_undo_stack(&history)).unwrap();
    let crate::session::RestoreOutcome::Restored(restored) = crate::session::restore_csv_fixture(&entry, directory.path())
        else { panic!("plain saved source restored") };
    assert_eq!(restored.buffer.rope.to_string(), source);
    assert_eq!((restored.selection.anchor, restored.selection.head), (source.len(), 2));
    assert_eq!(restored.buffer.bookmarks.iter().copied().collect::<Vec<_>>(), vec![0, last_row]);
    assert_eq!(restored.undo_stack.len(), 1);
    with_source("", LineEnding::Lf, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, cx| { w.tabs[0] = restored; w.commit_caret(cx); });
        visual.run_until_parked();
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        assert_eq!(text(workspace, visual), base);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!((w.selection().anchor, w.selection().head), (base.len(), 2));
            assert_eq!(w.buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![0, last_row]);
        });
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        assert_eq!(text(workspace, visual), source);
    });
    let mut hostile = entry;
    hostile.cursor_anchor = usize::MAX; hostile.cursor_head = 10;
    hostile.bookmarks.push(usize::MAX);
    let crate::session::RestoreOutcome::Restored(restored) = crate::session::restore_csv_fixture(&hostile, directory.path())
        else { panic!("hostile offsets still restore source") };
    assert_eq!((restored.selection.anchor, restored.selection.head), (source.len(), 8));
    assert_eq!(restored.buffer.rope.to_string(), source);
}

#[test]
fn physical_bookmark_edit_saved_macros_keep_unicode_body_and_one_undo() {
    for separator in ["\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"] {
        let source = format!("x{separator}y\nz\nq");
        with_source(&source, LineEnding::Lf, |workspace, visual| {
            caret(workspace, visual, 1 + separator.len());
            visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
            visual.dispatch_action(MacroStartRecording);
            visual.dispatch_action(RemoveBookmarkedLines); visual.run_until_parked();
            visual.dispatch_action(MacroStopRecording);
            let events = macros::current(); assert_eq!(events.len(), 1);
            assert!(matches!(events[0], macros::MacroEvent::RemoveBookmarkedLines));
            assert_eq!(text(workspace, visual), "z\nq");
            let directory = crate::persist::TempDir::new("physical-bookmark-macro");
            let saved = macros::saved_round_trip_for_test(&directory.join("macros.toml"),
                &std::collections::BTreeMap::from([("physical".into(), events)])).unwrap();
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), source);
            // Removing a row deletes its marker; Undo restores only its text.
            visual.update(|_, cx| assert!(workspace.read(cx).buffer().bookmarks.is_empty()));
            visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![0]));
            let _playback = macros::override_playback_for_test(Vec::new(), saved);
            visual.dispatch_action(MacroRunSaved { name: "physical".into() });
            visual.run_until_parked();
            assert_eq!(text(workspace, visual), "z\nq");
            visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), source);
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), "z\nq");
        });
    }
}

#[test]
fn reader_rich_and_csv_views_keep_structural_content_and_raw_bytes() {
    use crate::tab::TabViewMode;
    for separator in ["\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"] {
        let scalar = separator.chars().next().unwrap() as u32;
        for (name, source, mode) in [
            ("fixture.md", format!("left{separator}right\n\nlast"), TabViewMode::MarkdownPreview),
            ("fixture.csv", format!("label,value\n\"left{separator}right\ninside\",1\nlast,2\n"), TabViewMode::CsvPreview),
            ("fixture.rtf", format!("{{\\rtf1\\ansi left\\u{scalar}?right\\par last}}"), TabViewMode::RichDocView),
        ] {
            let directory = crate::persist::TempDir::new("physical-view-source");
            let path = directory.join(name); std::fs::write(&path, &source).unwrap();
            let buffer = crate::io::read_buffer(&path).unwrap();
            let original = buffer.rope.to_string();
            with_source("", LineEnding::Lf, |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, cx| {
                    w.tabs[0] = Tab::with_buffer(buffer); w.commit_caret(cx);
                });
                visual.run_until_parked();
                visual.dispatch_action(ViewPreview); visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx);
                    assert_eq!(w.tabs[0].view_mode, mode);
                    assert_eq!(w.buffer().rope.to_string(), original);
                    match mode {
                        TabViewMode::CsvPreview => {
                            let model = w.csv_models.get(&w.tabs[0].session_id).unwrap();
                            assert_eq!(model.headers, ["label", "value"]);
                            assert_eq!(model.rows.len(), 2);
                            assert_eq!(model.rows[0], [format!("left{separator}right\ninside"), "1".into()]);
                            assert_eq!(model.rows[1], ["last", "2"]);
                        }
                        TabViewMode::RichDocView => {
                            let payload = w.buffer().rich_doc.as_ref().unwrap();
                            assert_eq!(payload.block_text(0), format!("left{separator}right"));
                            assert_eq!(payload.block_text(1), "last");
                            assert_eq!(payload.original_bytes.as_slice(), source.as_bytes());
                        }
                        _ => {}
                    }
                    assert!(w.tabs[0].undo_stack.is_empty());
                });
                visual.dispatch_action(ViewSource); visual.run_until_parked();
                visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].view_mode, TabViewMode::Edit));
                assert_eq!(text(workspace, visual), original);
                visual.dispatch_action(FileSave); visual.run_until_parked();
                assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
            });
        }
    }
}

#[test]
#[ignore = "default-release >10 MiB Unicode streaming/open/caret/save control"]
fn physical_lines_release_unicode_streaming_callers() {
    use crate::text::encoding::{self, Encoding};
    use std::time::Instant;
    const ROWS: usize = 160_000;
    let body = format!("record {} é中😀\u{b}\u{c}\u{85}\u{2028}\u{2029} end", "payload".repeat(8));
    let source = format!("{body}\r\n").repeat(ROWS);
    let normalized = format!("{body}\n").repeat(ROWS);
    assert!(source.len() > crate::io_stream::STREAM_THRESHOLD_BYTES as usize);
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Be] {
        let raw = encoding::encode_checked(&source, encoding).unwrap();
        let directory = crate::persist::TempDir::new("physical-streaming");
        let path = directory.join("fixture.txt"); std::fs::write(&path, &raw).unwrap();
        with_source("", LineEnding::Lf, |workspace, visual| {
            let started = Instant::now();
            visual.update_window_entity(workspace, |w, _, cx| {
                w.open_path(path.clone(), cx); assert!(w.buffer().loading.is_some());
            });
            visual.run_until_parked();
            let open_ms = started.elapsed().as_secs_f64() * 1000.;
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.buffer().loading.is_none());
                assert_eq!((w.buffer().encoding, w.buffer().line_ending), (encoding, LineEnding::Crlf));
                assert_eq!(w.buffer().rope.len_lines(), ROWS + 1);
                assert_eq!(w.buffer().rope.to_string(), normalized);
            });
            let started = Instant::now();
            visual.dispatch_action(MoveDown); visual.run_until_parked();
            let movement_ms = started.elapsed().as_secs_f64() * 1000.;
            visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, body.len() + 1));
            visual.dispatch_action(BookmarkToggle); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![body.len() + 1]));
            let started = Instant::now();
            visual.dispatch_action(FileSave); visual.run_until_parked();
            let save_ms = started.elapsed().as_secs_f64() * 1000.;
            assert_eq!(std::fs::read(&path).unwrap(), raw);
            visual.dispatch_action(EditDuplicateCurrentLine); visual.run_until_parked();
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            assert_eq!(text(workspace, visual), normalized);
            visual.dispatch_action(FileSave); visual.run_until_parked();
            assert_eq!(std::fs::read(&path).unwrap(), raw);
            println!("physical-streaming encoding={encoding:?} disk_bytes={} rope_bytes={} physical_rows={} open={open_ms:.3}ms movement={movement_ms:.3}ms save={save_ms:.3}ms exact_bytes=true undo=true",
                raw.len(), normalized.len(), ROWS + 1);
        });
    }
}
