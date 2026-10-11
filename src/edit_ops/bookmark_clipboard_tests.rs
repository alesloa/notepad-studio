use super::*;
use crate::{actions::*, macros, settings::Settings, tab::Tab, text::encoding::Encoding};
use gpui::{ElementInputHandler, Entity, InputHandler, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

pub(crate) fn fixture<'a>(cx: &'a mut TestAppContext, source: &str, bookmarks: &[usize]) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = Rope::from_str(source);
        tab.buffer.line_ending = LineEnding::Lf;
        tab.buffer.encoding = Encoding::Utf16Le;
        tab.buffer.bookmarks.extend(bookmarks.iter().copied());
        tab.buffer.mark_saved();
        tab.selection.anchor = source.len();
        tab.selection.head = 0;
        w.tabs = vec![tab];
        w.active = 0;
        let (sink, _) = std::sync::mpsc::channel();
        w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    (workspace, visual)
}

fn clipboard(visual: &VisualTestContext) -> String {
    visual.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default()
}

fn with_source(source: &str, bookmarks: &[usize], test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _execution = macros::override_execution_for_test(false);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, source, bookmarks);
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    test(&workspace, visual);
    cx.quit();
}

#[test]
fn bookmark_clipboard_menu_exposes_copy_bookmarked_lines() {
    let gpui::MenuItem::Submenu(menu) = crate::app::menus::bookmarks::build() else { panic!("Bookmark submenu"); };
    let labels: Vec<_> = menu.items.iter().map(|item| match item {
        gpui::MenuItem::Action { name, .. } => name.as_ref(),
        _ => "unexpected separator or submenu",
    }).collect();
    assert_eq!(labels, ["Toggle Bookmark", "Next Bookmark", "Previous Bookmark", "Clear All Bookmarks", "Copy Bookmarked Lines"]);
    let index = menu.items.iter().position(|item| matches!(item,
        gpui::MenuItem::Action { name, .. } if name.as_ref() == "Copy Bookmarked Lines"))
        .expect("Copy Bookmarked Lines must have a real Bookmark menu route");
    assert!(matches!(menu.items.get(index - 1), Some(gpui::MenuItem::Action { name, .. }) if name.as_ref() == "Clear All Bookmarks"));
    let row = crate::shortcuts::catalog().iter().find(|row| row.id == "CopyBookmarkedLines").unwrap();
    assert_eq!(row.npp, Some("IDM_SEARCH_COPYMARKEDLINES"));
    assert!(row.keys.is_empty() && row.mac_keys.is_empty());
    assert!((row.action)().as_any().is::<CopyBookmarkedLines>());
    with_source("é😀\nignored\n中", &[0, "é😀\nignored\n".len()], |workspace, visual| {
        let action = menu.items.into_iter().find_map(|item| match item {
            gpui::MenuItem::Action { name, action, .. } if name.as_ref() == "Copy Bookmarked Lines" => Some(action),
            _ => None,
        }).unwrap();
        visual.update(|window, cx| window.dispatch_action(action, cx));
        visual.run_until_parked();
        assert_eq!(clipboard(visual), "é😀\n中");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "é😀\nignored\n中");
            assert_eq!((w.selection().anchor, w.selection().head), ("é😀\nignored\n中".len(), 0));
            assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
            assert!(!w.buffer().dirty && !w.window_closed);
        });
    });
}

#[test]
fn bookmark_clipboard_dense_unicode_rows_lookup_the_physical_line_once() {
    let row = "é\u{2028}x";
    let source = format!("{}\n", row.repeat(4096));
    let bookmarks: Vec<_> = (0..4096).map(|index| index * row.len()).collect();
    PHYSICAL_LINE_LOOKUPS.with(|count| count.set(0));
    assert_eq!(bookmarked_text(&Rope::from_str(&source), &bookmarks, LineEnding::Lf,
        MAX_COPY_BYTES, &AtomicBool::new(false)).unwrap(), source);
    assert_eq!(PHYSICAL_LINE_LOOKUPS.with(|count| count.get()), 1);
    with_source(&source, &bookmarks, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer.read_only = true);
        PHYSICAL_LINE_LOOKUPS.with(|count| count.set(0));
        visual.dispatch_action(CopyBookmarkedLines);
        assert_eq!(clipboard(visual), source);
        assert_eq!(PHYSICAL_LINE_LOOKUPS.with(|count| count.get()), 1);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source);
            assert_eq!((w.selection().anchor, w.selection().head), (source.len(), 0));
            assert_eq!(w.buffer().bookmarks.iter().copied().collect::<Vec<_>>(), bookmarks);
            assert_eq!(w.buffer().encoding, Encoding::Utf16Le);
            assert_eq!(w.buffer().line_ending, LineEnding::Lf);
            assert!(w.buffer().read_only && !w.buffer().dirty);
            assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
        });
    });
}

#[test]
fn bookmark_clipboard_covered_offsets_preserve_eof_crlf_and_unsorted_inputs() {
    for (source, bookmarks, expected, lookups) in [
        ("é😀\r\nq\r\n", vec![0, 1, 3, 6, 7, 8, 9, 10, 11, 11, usize::MAX], "é😀\r\nq\r\n", 3),
        ("é😀", vec![1, 2, 5, 6, usize::MAX], "é😀", 2),
        ("", vec![0, 0, usize::MAX], "", 1),
        ("a\nb\nc", vec![2, 0], "b\na\n", 2),
        ("é😀\n\n", vec![1, 3, 6, 7, 8, 8, usize::MAX], "é😀\n\n", 3),
        ("é😀\n", vec![], "", 0),
    ] {
        PHYSICAL_LINE_LOOKUPS.with(|count| count.set(0));
        assert_eq!(bookmarked_text(&Rope::from_str(source), &bookmarks, LineEnding::Lf,
            MAX_COPY_BYTES, &AtomicBool::new(false)).unwrap(), expected);
        assert_eq!(PHYSICAL_LINE_LOOKUPS.with(|count| count.get()), lookups, "{source:?}");
    }
    PHYSICAL_LINE_LOOKUPS.with(|count| count.set(0));
    assert!(bookmarked_text(&Rope::from_str("é\u{2028}x\n"), &[0, 1, 2, 5], LineEnding::Lf,
        MAX_COPY_BYTES, &AtomicBool::new(true)).unwrap_err().contains("cancelled"));
    assert_eq!(PHYSICAL_LINE_LOOKUPS.with(|count| count.get()), 0);
}

#[test]
fn bookmark_clipboard_extraction_preserves_physical_lines_and_unterminated_eof() {
    for (source, bookmarks, ending, expected) in [
        ("a\r\n\r\né😀", vec![0, 3, 5], LineEnding::Lf, "a\r\n\r\né😀"),
        ("a\rb\n中", vec![0, 2, 4], LineEnding::Lf, "a\rb\n中"),
        ("a\nb\n", vec![2, 4], LineEnding::Crlf, "b\r\n"),
        ("a\nb", vec![0, 2], LineEnding::Cr, "a\rb"),
        ("a\u{2028}😀\u{85}b\nq", vec![0, 4, 12], LineEnding::Lf, "a\u{2028}😀\u{85}b\nq"),
        ("a\r\nb\rc\n", vec![0, 3, 5], LineEnding::Lf, "a\r\nb\rc\n"),
        ("a\r\nb\rc\n", vec![0, 3, 5], LineEnding::Cr, "a\rb\rc\r"),
        ("é😀", vec![1, usize::MAX], LineEnding::Lf, "é😀"),
        ("", vec![0], LineEnding::Crlf, ""),
        ("\u{feff}é😀\n中", vec![0, "\u{feff}é😀\n".len()], LineEnding::Lf, "\u{feff}é😀\n中"),
    ] {
        assert_eq!(bookmarked_text(&Rope::from_str(source), &bookmarks, ending, MAX_COPY_BYTES, &AtomicBool::new(false)).unwrap(), expected);
    }
}

#[test]
fn bookmark_clipboard_preserves_rectangular_selection_marks_and_existing_undo_redo() {
    with_source("café😀\n中", &[0], |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            w.replace_byte_range(0..0, "a", cx);
            let end = w.buffer().rope.len_bytes();
            w.replace_byte_range(end..end, "b", cx);
            w.undo(&EditUndo, window, cx);
            w.active_tab_mut().block_selection = Some(crate::buffer::BlockSelection {
                anchor_line: 0, head_line: 1, anchor_col: 1, head_col: 3, is_dragging: false,
            });
            w.active_tab_mut().buffer.marks = std::iter::once(0..2).collect();
        });
        let before = visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!((w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len()), (1, 1));
            (w.buffer().raw_revision.clone(), w.selection(), w.buffer().marks.clone(), w.autosave_generation)
        });
        visual.dispatch_action(CopyBookmarkedLines);
        assert_eq!(clipboard(visual), "acafé😀\n");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "acafé😀\n中");
            assert_eq!(w.buffer().raw_revision, before.0);
            assert_eq!((w.selection().anchor, w.selection().head), (before.1.anchor, before.1.head));
            assert_eq!(w.buffer().marks, before.2);
            assert_eq!(w.autosave_generation, before.3);
            let block = w.tabs[0].block_selection.unwrap();
            assert_eq!((block.anchor_line, block.head_line, block.anchor_col, block.head_col), (0, 1, 1, 3));
            assert_eq!((w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len()), (1, 1));
        });
    });
}

#[test]
fn bookmark_clipboard_limits_and_cancellation_do_not_need_large_allocations() {
    let rope = Rope::from_str("a\nb");
    assert_eq!(bookmarked_text(&rope, &[0, 2], LineEnding::Crlf, 4, &AtomicBool::new(false)).unwrap(), "a\r\nb");
    assert!(bookmarked_text(&rope, &[0, 2], LineEnding::Crlf, 3, &AtomicBool::new(false)).unwrap_err().contains("128 MiB"));
    assert!(bookmarked_text(&rope, &[0], LineEnding::Lf, MAX_COPY_BYTES, &AtomicBool::new(true)).unwrap_err().contains("cancelled"));
}

#[test]
fn bookmark_clipboard_disk_loaded_endings_encodings_and_read_only_leave_bytes_and_state_unchanged() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        for ending in [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr] {
            let source = format!("café😀{}{}中", ending.as_str(), ending.as_str());
            let dir = crate::persist::TempDir::new("bookmark-clipboard-fidelity");
            let path = dir.join("neutral.txt");
            let bytes = crate::text::encoding::encode(&source, encoding);
            std::fs::write(&path, &bytes).unwrap();
            let buffer = crate::io::read_buffer(&path).unwrap();
            let normalized = buffer.rope.to_string();
            with_source(&normalized, &[], |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, _| {
                    let tab = w.active_tab_mut();
                    tab.buffer = buffer;
                    tab.buffer.read_only = true;
                    for row in 0..tab.buffer.rope.len_lines() { tab.buffer.bookmarks.insert(tab.buffer.rope.line_to_byte(row)); }
                });
                let before = visual.update(|_, cx| {
                    let w = workspace.read(cx);
                    (w.buffer().raw_revision.clone(), w.tabs[0].content_activity.clone(), w.autosave_generation, w.buffer().bookmarks.clone())
                });
                visual.dispatch_action(CopyBookmarkedLines);
                assert_eq!(clipboard(visual), source);
                visual.update(|_, cx| {
                    let w = workspace.read(cx);
                    assert_eq!(w.buffer().rope.to_string(), normalized);
                    assert_eq!(w.buffer().encoding, encoding);
                    assert_eq!(w.buffer().line_ending, ending);
                    assert_eq!((&w.buffer().raw_revision, &w.tabs[0].content_activity, w.autosave_generation, &w.buffer().bookmarks), (&before.0, &before.1, before.2, &before.3));
                    assert_eq!((w.selection().anchor, w.selection().head), (normalized.len(), 0));
                    assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
                    assert!(!w.buffer().dirty && w.buffer().read_only);
                });
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            });
        }
    }
}

#[test]
fn bookmark_clipboard_no_bookmarks_or_empty_eof_clears_clipboard_and_records_command() {
    for bookmarks in [vec![], vec![2]] {
        with_source("a\n", &bookmarks, |workspace, visual| {
            let _record = macros::override_execution_for_test(true);
            visual.dispatch_action(CopyBookmarkedLines);
            assert_eq!(clipboard(visual), "");
            assert!(visual.read_from_clipboard().is_some());
            assert!(!macros::stop_recording());
            assert!(matches!(macros::current().as_slice(), [MacroEvent::CopyBookmarkedLines]));
            visual.update(|_, cx| assert!(workspace.read(cx).tabs[0].undo_stack.is_empty()));
        });
    }
}

#[test]
fn bookmark_clipboard_incompatible_views_focus_loading_and_ime_are_refused() {
    for case in 0..9 {
        with_source("é😀\n中", &[0], |workspace, visual| {
            visual.update_window_entity(workspace, |w, window, cx| match case {
                0 => w.active_tab_mut().view_mode = TabViewMode::MarkdownPreview,
                1 => w.active_tab_mut().view_mode = TabViewMode::CsvPreview,
                2 => w.active_tab_mut().view_mode = TabViewMode::RichDocView,
                3 => w.focus_input(InputFocus::FindQuery, window, cx),
                4 => w.focus_input(InputFocus::Replacement, window, cx),
                5 => w.focus_input(InputFocus::Goto, window, cx),
                6 => window.focus(&cx.focus_handle(), cx),
                7 => w.active_tab_mut().buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 0, total_bytes: 10, cancel: Arc::new(AtomicBool::new(false)) }),
                _ => {},
            });
            if case == 8 {
                visual.update(|window, cx| {
                    let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                    handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                    assert!(workspace.read(cx).is_composing_text(window));
                });
            }
            let before = visual.update(|_, cx| {
                let w = workspace.read(cx);
                (w.buffer().rope.to_string(), w.tabs[0].undo_stack.len(), w.selection())
            });
            let _record = macros::override_execution_for_test(true);
            visual.update_window_entity(workspace, |w, window, cx| assert!(!w.run_copy_bookmarked_lines(window, cx)));
            assert_eq!(clipboard(visual), "sentinel");
            macros::stop_recording();
            assert!(macros::current().is_empty());
            visual.update(|window, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), before.0);
                assert_eq!(w.tabs[0].undo_stack.len(), before.1);
                assert_eq!((w.selection().anchor, w.selection().head), (before.2.anchor, before.2.head));
                assert!(w.status_message.is_some());
                if case == 8 { assert!(w.is_composing_text(window)); }
            });
        });
    }
}

#[test]
fn bookmark_clipboard_background_completion_cancellation_and_stale_results_preserve_clipboard() {
    let source = format!("{}\n中", "é".repeat(FOREGROUND_BYTES));
    for case in 0..13 {
        with_source(&source, &[0], |workspace, visual| {
            if case == 12 {
                visual.update_window_entity(workspace, |w, _, cx| {
                    w.active_tab_mut().block_selection = Some(crate::buffer::BlockSelection {
                        anchor_line: 0, head_line: 1, anchor_col: 1, head_col: 3, is_dragging: false,
                    });
                    w.sync_source_input_caret();
                    cx.notify();
                });
                visual.run_until_parked();
            }
            let (send, receive) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, _, _| w.bookmark_copy_test_gate = Some(receive));
            visual.dispatch_action(CopyBookmarkedLines);
            assert_eq!(clipboard(visual), "sentinel");
            visual.update_window_entity(workspace, |w, window, cx| {
                assert!(w.pending_bookmark_copy.is_some());
                match case {
                    1 => w.close_find(&EditFindClose, window, cx),
                    2 => w.active_tab_mut().buffer.raw_revision = ContentRevision::default(),
                    3 => w.active_tab_mut().buffer.bookmarks.clear(),
                    4 => w.active_tab_mut().instance = Arc::new(()),
                    5 => w.focus_input(InputFocus::FindQuery, window, cx),
                    6 => w.active_tab_mut().buffer.line_ending = LineEnding::Cr,
                    7 => {
                        w.active_tab_mut().selection.anchor = 0;
                        w.active_tab_mut().selection.head = 2;
                        w.copy(&EditCopy, window, cx);
                    }
                    8 => {
                        w.focus_input(InputFocus::FindQuery, window, cx);
                        w.focus_input(InputFocus::Editor, window, cx);
                    }
                    9 => {
                        let selection = w.selection();
                        w.active_tab_mut().selection.collapse_to(2);
                        w.sync_source_input_caret();
                        w.active_tab_mut().selection = selection;
                        w.sync_source_input_caret();
                    }
                    10 => {
                        w.set_status("unrelated newer status");
                        w.active_tab_mut().selection.anchor = 0;
                        w.active_tab_mut().selection.head = 2;
                        w.copy(&EditCopy, window, cx);
                    }
                    11 => {
                        let _record = macros::override_execution_for_test(true);
                        assert!(!w.run_copy_bookmarked_lines(window, cx));
                    }
                    12 => w.close_find(&EditFindClose, window, cx),
                    _ => {},
                }
            });
            send.send(()).unwrap();
            visual.run_until_parked();
            assert_eq!(clipboard(visual), match case {
                0 => format!("{}\n", "é".repeat(FOREGROUND_BYTES)),
                7 | 10 => "é".into(),
                _ => "sentinel".into(),
            });
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.pending_bookmark_copy.is_none());
                assert_eq!(w.buffer().rope.to_string(), source);
                assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
                if case == 7 { assert!(w.status_message.is_none()); }
                if case == 10 { assert_eq!(w.status_message.as_deref(), Some("unrelated newer status")); }
            });
        });
    }
}

#[test]
#[ignore = "manual Windows release timing: cargo test --release bookmark_clipboard_release_manual_timing -- --ignored --nocapture"]
fn bookmark_clipboard_release_manual_timing() {
    let row = format!("{}\n", "café😀 ".repeat(2000));
    let source = row.repeat(600);
    assert!(source.len() > 10 * 1024 * 1024);
    let bookmarks: Vec<usize> = (0..600).map(|row_number| row_number * row.len()).collect();
    for case in 0..4 {
        with_source(&source, &bookmarks, |workspace, visual| {
            let (send, receive) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, _, _| w.bookmark_copy_test_gate = Some(receive));
            let started = std::time::Instant::now();
            visual.dispatch_action(CopyBookmarkedLines);
            let caller = started.elapsed();
            assert_eq!(clipboard(visual), "sentinel");
            visual.update_window_entity(workspace, |w, window, cx| match case {
                1 => w.close_find(&EditFindClose, window, cx),
                2 => w.active_tab_mut().buffer.raw_revision = ContentRevision::default(),
                3 => {
                    w.active_tab_mut().selection.anchor = 0;
                    w.active_tab_mut().selection.head = 1;
                    w.copy(&EditCopy, window, cx);
                }
                _ => {},
            });
            send.send(()).unwrap();
            visual.run_until_parked();
            eprintln!("bookmark copy caller: case={case}, source={} bytes, dispatch={caller:?}, total={:?}", source.len(), started.elapsed());
            assert_eq!(clipboard(visual), match case { 0 => source.clone(), 3 => "c".into(), _ => "sentinel".into() });
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.pending_bookmark_copy.is_none());
                assert_eq!(w.buffer().rope.to_string(), source);
                assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
            });
        });
    }
    // Keep the long physical row below the viewport. Its Ropey-only Unicode
    // boundary makes lookup widen backwards across more than 10 MiB.
    let prefix = "ordinary\n".repeat(200);
    let long_body = "é".repeat(6 * 1024 * 1024);
    let source = format!("{prefix}{long_body}\u{2028}😀tail\n");
    let bookmark = prefix.len() + long_body.len() + '\u{2028}'.len_utf8();
    with_source(&source, &[bookmark], |workspace, visual| {
        let started = std::time::Instant::now();
        visual.dispatch_action(CopyBookmarkedLines);
        let caller = started.elapsed();
        visual.run_until_parked();
        eprintln!("bookmark copy Unicode-row caller: source={} bytes, dispatch={caller:?}, total={:?}", source.len(), started.elapsed());
        assert_eq!(clipboard(visual), format!("{long_body}\u{2028}😀tail\n"));
        visual.update(|_, cx| assert!(workspace.read(cx).pending_bookmark_copy.is_none()));
    });
}

#[test]
fn bookmark_clipboard_large_macro_refuses_before_following_paste_or_clipboard_mutation() {
    let source = "x".repeat(FOREGROUND_BYTES + 1);
    with_source(&source, &[0], |workspace, visual| {
        let _record = macros::override_execution_for_test(true);
        visual.dispatch_action(CopyBookmarkedLines);
        macros::stop_recording();
        assert!(macros::current().is_empty());
        visual.update_window_entity(workspace, |w, window, cx| {
            assert!(!w.replay_macro(&[MacroEvent::CopyBookmarkedLines, MacroEvent::Paste], 1, window, cx));
            assert!(w.status_message.as_deref().is_some_and(|message| message.contains("Large Copy Bookmarked Lines")));
            assert!(w.tabs[0].undo_stack.is_empty() && w.tabs[0].edit_transaction.is_none());
            assert_eq!(w.buffer().rope.to_string(), source);
        });
        assert_eq!(clipboard(visual), "sentinel");
    });
}
