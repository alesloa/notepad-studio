use crate::actions::*;
use super::bookmark_edits::*;
use crate::{macros, settings::Settings, text::{encoding::Encoding, line_ending::LineEnding}, workspace::Workspace, buffer::ContentRevision};
use gpui::{ClipboardItem, Entity, TestAppContext, VisualTestContext, VisualContext, ElementInputHandler, InputHandler};
use std::sync::{Arc, atomic::AtomicBool};

fn with_source(source: &str, bookmarks: &[usize], test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings::default());
    let _execution = macros::override_execution_for_test(false);
    let mut cx = TestAppContext::single();
    let (workspace, visual) = super::bookmark_clipboard::tests::fixture(&mut cx, source, bookmarks);
    visual.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    test(&workspace, visual);
    cx.quit();
}

fn clipboard(visual: &VisualTestContext) -> String { visual.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default() }

fn dispatch(command: BookmarkCommand, visual: &mut VisualTestContext) {
    match command {
        BookmarkCommand::Cut => visual.dispatch_action(CutBookmarkedLines),
        BookmarkCommand::Paste => visual.dispatch_action(PasteToBookmarkedLines),
        BookmarkCommand::Remove => visual.dispatch_action(RemoveBookmarkedLines),
        BookmarkCommand::RemoveNon => visual.dispatch_action(RemoveNonBookmarkedLines),
        BookmarkCommand::Inverse => visual.dispatch_action(InverseBookmarks),
    }
    visual.run_until_parked();
}

#[test]
fn bookmark_edits_menu_has_every_oracle_action() {
    let gpui::MenuItem::Submenu(menu) = crate::app::menus::bookmarks::build() else { panic!("Bookmark submenu"); };
    let labels: Vec<_> = menu.items.iter().map(|item| match item {
        gpui::MenuItem::Action { name, .. } => name.as_ref(),
        _ => "unexpected separator or submenu",
    }).collect();
    assert_eq!(labels, ["Toggle Bookmark", "Next Bookmark", "Previous Bookmark", "Clear All Bookmarks",
        "Cut Bookmarked Lines", "Copy Bookmarked Lines", "Paste to (Replace) Bookmarked Lines",
        "Remove Bookmarked Lines", "Remove Non-Bookmarked Lines", "Inverse Bookmarks"]);
}

#[test]
fn bookmark_edits_menu_actions_and_catalog_run_real_handlers() {
    let gpui::MenuItem::Submenu(menu) = crate::app::menus::bookmarks::build() else { panic!("Bookmark menu"); };
    for (label, id, npp, expected) in [
        ("Cut Bookmarked Lines", "CutBookmarkedLines", "IDM_SEARCH_CUTMARKEDLINES", "keep\n"),
        ("Paste to (Replace) Bookmarked Lines", "PasteToBookmarkedLines", "IDM_SEARCH_PASTEMARKEDLINES", "sentinel\nkeep\nsentinel"),
        ("Remove Bookmarked Lines", "RemoveBookmarkedLines", "IDM_SEARCH_DELETEMARKEDLINES", "keep\n"),
        ("Remove Non-Bookmarked Lines", "RemoveNonBookmarkedLines", "IDM_SEARCH_DELETEUNMARKEDLINES", "é😀\n中"),
        ("Inverse Bookmarks", "InverseBookmarks", "IDM_SEARCH_INVERSEMARKS", "é😀\nkeep\n中"),
    ] {
        let row = crate::shortcuts::catalog().iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.npp, Some(npp)); assert!(row.keys.is_empty() && row.mac_keys.is_empty());
        let action = menu.items.iter().find_map(|item| match item {
            gpui::MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action.boxed_clone()), _ => None,
        }).unwrap();
        assert_eq!(action.name(), (row.action)().name());
        with_source("é😀\nkeep\n中", &[0, "é😀\nkeep\n".len()], |workspace, visual| {
            visual.update(|window, cx| window.dispatch_action(action, cx)); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        });
    }
}

#[test]
fn bookmark_edits_cut_remove_and_inverse_cover_physical_lines_and_eof() {
    for (source, bookmarks, removed, kept, copied) in [
        ("a\r\n\r\né😀", vec![0, 5], "\r\n", "a\r\né😀", "a\r\né😀"),
        ("a\rb\n中", vec![0, 4], "b\n", "a\r中", "a\r中"),
        ("a\nb\n", vec![2, 4], "a\n", "b\n", "b\n"),
        ("é\u{2028}😀\u{85}b\nq", vec![1, 3, 7], "q", "é\u{2028}😀\u{85}b\n", "é\u{2028}😀\u{85}b\n"),
        ("é😀", vec![1, usize::MAX], "", "é😀", "é😀"),
        ("", vec![0], "", "", ""),
        ("a\nb", vec![], "a\nb", "", ""),
        ("a\0hidden\nb\n", vec![0, 9], "", "a\0hidden\nb\n", "ab\n"),
    ] {
        for (command, expected) in [(BookmarkCommand::Cut, removed), (BookmarkCommand::Remove, removed), (BookmarkCommand::RemoveNon, kept)] {
            with_source(source, &bookmarks, |workspace, visual| {
                dispatch(command, visual);
                assert_eq!(clipboard(visual), if command == BookmarkCommand::Cut { copied } else { "sentinel" });
                visual.update(|_, cx| {
                    let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected, "{source:?}");
                    assert_eq!(w.buffer().encoding, Encoding::Utf16Le); assert_eq!(w.buffer().line_ending, LineEnding::Lf);
                    if expected != source { assert_eq!(w.tabs[0].undo_stack.len(), 1); }
                    if command != BookmarkCommand::RemoveNon { assert!(w.buffer().bookmarks.iter().all(|&at| at <= expected.len())); }
                });
                if expected != source {
                    visual.dispatch_action(EditUndo); visual.run_until_parked();
                    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
                    visual.dispatch_action(EditRedo); visual.run_until_parked();
                    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
                }
            });
        }
    }
    with_source("a\r\nb\r中\n", &[0, 5], |workspace, visual| {
        dispatch(BookmarkCommand::Inverse, visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().bookmarks, [3, 9].into_iter().collect());
            assert!(w.tabs[0].undo_stack.is_empty() && !w.buffer().dirty);
        });
        dispatch(BookmarkCommand::Inverse, visual);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().bookmarks, [0, 5].into_iter().collect()));
    });
}

#[test]
fn bookmark_edits_paste_repeats_exact_clipboard_and_preserves_original_terminators() {
    for paste in ["X", "é😀\r\n中\n", "", "\r", "\0after"] {
        with_source("a\r\nkeep\rb\n", &[0, 8, 10], |workspace, visual| {
            visual.write_to_clipboard(ClipboardItem::new_string(paste.into()));
            dispatch(BookmarkCommand::Paste, visual);
            let inserted = paste.split('\0').next().unwrap();
            let expected = format!("{inserted}\r\nkeep\r{inserted}\n{inserted}");
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected, "paste={paste:?}, status={:?}", w.status_message);
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].edit_transaction.is_none());
            });
            assert_eq!(clipboard(visual), paste);
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\r\nkeep\rb\n"));
        });
    }
}

#[test]
fn bookmark_edits_annotations_selection_and_one_undo_preserve_existing_history() {
    with_source("a\nremove\nkeep\nlast", &[2], |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            w.replace_byte_range(0..0, "x", cx); w.replace_byte_range(0..0, "y", cx); w.undo(&EditUndo, window, cx);
            let tab = w.active_tab_mut(); tab.buffer.marks = vec![0..1, 3..5, 10..14]; tab.begin_select_marker = Some(11);
            tab.selection.anchor = 15; tab.selection.head = 1; tab.buffer.mark_saved();
        });
        dispatch(BookmarkCommand::Remove, visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "xa\nkeep\nlast");
            assert!(w.buffer().bookmarks.is_empty()); assert_eq!(w.buffer().marks, vec![0..1, 3..7]);
            assert_eq!(w.tabs[0].begin_select_marker, Some(4));
            assert_eq!((w.selection().anchor, w.selection().head), (8, 1));
            assert_eq!((w.tabs[0].undo_stack.len(), w.tabs[0].redo_stack.len()), (2, 0)); assert!(w.buffer().dirty);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "xa\nremove\nkeep\nlast");
            assert_eq!((w.selection().anchor, w.selection().head), (15, 1)); assert!(!w.buffer().dirty);
        });
    });
}

#[test]
fn bookmark_edits_paste_preparation_moves_single_text_and_stops_at_logical_first_nul() {
    use gpui::{ClipboardEntry, ClipboardString};
    let text = "é😀\r\n".repeat(70_000); let address = text.as_ptr();
    let result = prepare_paste(vec![ClipboardEntry::String(ClipboardString::new(text))], &AtomicBool::new(false)).unwrap();
    assert_eq!(result.as_ptr(), address);
    let entries = vec![ClipboardEntry::String(ClipboardString::new("é".into())),
        ClipboardEntry::Image(gpui::Image::empty()), ClipboardEntry::String(ClipboardString::new("😀\0after".into())),
        ClipboardEntry::String(ClipboardString::new("ignored".into()))];
    assert_eq!(prepare_paste(entries, &AtomicBool::new(false)).unwrap(), "é😀");
    assert!(prepare_paste(vec![ClipboardEntry::String(ClipboardString::new(result))], &AtomicBool::new(true)).is_err());
    assert!(prepare_paste(vec![ClipboardEntry::Image(gpui::Image::empty()); 4097], &AtomicBool::new(false)).is_err());
}

#[test]
fn bookmark_edits_guards_refuse_changes_and_preserve_clipboard_and_history() {
    for case in 0..11 {
        with_source("é😀\n中", &[0], |workspace, visual| {
            visual.update_window_entity(workspace, |w, window, cx| match case {
                0 => w.active_tab_mut().buffer.read_only = true,
                1 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::MarkdownPreview,
                2 => { w.active_tab_mut().view_mode = crate::tab::TabViewMode::CsvPreview; w.active_tab_mut().buffer.file_path = Some("neutral.csv".into()); },
                3 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::RichDocView,
                4 => w.focus_input(crate::tab::InputFocus::FindQuery, window, cx),
                5 => window.focus(&cx.focus_handle(), cx),
                6 => w.active_tab_mut().buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 0, total_bytes: 10, cancel: Arc::new(AtomicBool::new(false)) }),
                7 => w.begin_edit_group(),
                8 => w.active_tab_mut().block_selection = Some(crate::buffer::BlockSelection { anchor_line: 0, head_line: 1, anchor_col: 0, head_col: 1, is_dragging: false }),
                9 => w.window_closed = true,
                _ => {},
            });
            if case == 10 {
                visual.update(|window, cx| {
                    let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                    handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                    assert!(workspace.read(cx).is_composing_text(window));
                });
            }
            let before = visual.update(|_, cx| { let w = workspace.read(cx); (w.buffer().rope.to_string(), w.selection(), w.tabs[0].undo_stack.len()) });
            for command in [BookmarkCommand::Cut, BookmarkCommand::Paste, BookmarkCommand::Remove, BookmarkCommand::RemoveNon] {
                visual.update_window_entity(workspace, |w, window, cx| assert!(!w.run_bookmark_command(command, window, cx), "guard={case}, command={}", command as usize));
                visual.run_until_parked(); assert_eq!(clipboard(visual), "sentinel");
                visual.update(|_, cx| {
                    let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), before.0);
                    assert_eq!((w.selection().anchor, w.selection().head), (before.1.anchor, before.1.head));
                    assert_eq!(w.tabs[0].undo_stack.len(), before.2); assert!(w.pending_prepared_edit.is_none());
                });
            }
        });
    }
    with_source("a\nb\n", &[0], |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer.read_only = true);
        dispatch(BookmarkCommand::Inverse, visual);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().bookmarks, [2, 4].into_iter().collect()));
    });
}

#[test]
fn bookmark_edits_cancel_and_stale_ownership_do_not_publish_any_part() {
    for case in 0..16 {
        with_source("é😀\nkeep\n中", &[0], |workspace, visual| {
            let (send, receive) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
            visual.dispatch_action(CutBookmarkedLines); visual.run_until_parked();
            visual.update_window_entity(workspace, |w, window, cx| {
                assert!(w.pending_prepared_edit.is_some());
                match case {
                    0 => w.close_find(&EditFindClose, window, cx),
                    1 => w.active_tab_mut().instance = Arc::new(()),
                    2 => w.active_tab_mut().buffer.raw_revision = ContentRevision::default(),
                    3 => w.active_tab_mut().buffer.bookmarks.clear(),
                    4 => w.active_tab_mut().buffer.marks.push(0..2),
                    5 => w.active_tab_mut().selection.collapse_to(2),
                    6 => w.active_tab_mut().buffer.read_only = true,
                    7 => w.focus_input(crate::tab::InputFocus::FindQuery, window, cx),
                    8 => { w.focus_input(crate::tab::InputFocus::FindQuery, window, cx); w.focus_input(crate::tab::InputFocus::Editor, window, cx); },
                    9 => w.active_tab_mut().buffer.encoding = Encoding::Utf8Bom,
                    10 => w.active_tab_mut().buffer.line_ending = LineEnding::Cr,
                    11 => w.active_tab_mut().buffer.forget_savepoint(),
                    12 => { w.active_tab_mut().buffer.file_path = Some("neutral.txt".into()); },
                    13 => w.active_tab_mut().view_mode = crate::tab::TabViewMode::MarkdownPreview,
                    14 => w.window_closed = true,
                    15 => { w.active_tab_mut().selection.anchor = 0; w.active_tab_mut().selection.head = 2; w.copy(&EditCopy, window, cx); },
                    _ => {},
                }
            });
            send.send(()).unwrap(); visual.run_until_parked();
            assert_eq!(clipboard(visual), if case == 15 { "é" } else { "sentinel" });
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "é😀\nkeep\n中");
                assert!(w.pending_prepared_edit.is_none() && w.tabs[0].undo_stack.is_empty() && w.tabs[0].redo_stack.is_empty());
            });
        });
    }
}

#[test]
fn bookmark_edits_disk_loaded_encodings_endings_save_undo_and_redo_are_faithful() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        for ending in [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr] {
            let source = format!("é😀{}keep{}中", ending.as_str(), ending.as_str());
            let dir = crate::persist::TempDir::new("bookmark-edit-fidelity"); let path = dir.join("neutral.txt");
            let original = crate::text::encoding::encode(&source, encoding);
            std::fs::write(&path, &original).unwrap();
            let buffer = crate::io::read_buffer(&path).unwrap();
            let normalized = buffer.rope.to_string();
            with_source(&normalized, &[], |workspace, visual| {
                visual.update_window_entity(workspace, |w, _, _| {
                    let tab = w.active_tab_mut(); tab.buffer = buffer; tab.buffer.bookmarks.insert(0);
                    tab.selection.collapse_to(0); tab.observe_content_activity();
                });
                dispatch(BookmarkCommand::Cut, visual);
                assert_eq!(clipboard(visual), format!("é😀{}", ending.as_str()));
                visual.update_window_entity(workspace, |w, _, _| {
                    assert_eq!(w.buffer().encoding, encoding); assert_eq!(w.buffer().line_ending, ending);
                    w.write_active_to_disk().unwrap();
                });
                assert_eq!(std::fs::read(&path).unwrap(), crate::text::encoding::encode(&format!("keep{}中", ending.as_str()), encoding));
                visual.dispatch_action(EditUndo); visual.run_until_parked();
                visual.update_window_entity(workspace, |w, _, _| { assert!(w.buffer().dirty); w.write_active_to_disk().unwrap(); assert!(!w.buffer().dirty); });
                assert_eq!(std::fs::read(&path).unwrap(), original);
                visual.dispatch_action(EditRedo); visual.run_until_parked();
                visual.update(|_, cx| assert!(workspace.read(cx).buffer().dirty));
            });
        }
    }
}

#[test]
fn bookmark_edits_newer_cut_retires_pending_copy_and_no_bookmarks_clear_clipboard() {
    let source = format!("{}\nkeep", "é😀".repeat(50_000));
    with_source(&source, &[0], |workspace, visual| {
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, _, _| w.bookmark_copy_test_gate = Some(receive));
        visual.dispatch_action(CopyBookmarkedLines); visual.run_until_parked();
        dispatch(BookmarkCommand::Cut, visual); assert_eq!(clipboard(visual), format!("{}\n", "é😀".repeat(50_000)));
        visual.write_to_clipboard(ClipboardItem::new_string("newer".into()));
        send.send(()).unwrap(); visual.run_until_parked(); assert_eq!(clipboard(visual), "newer");
        dispatch(BookmarkCommand::Cut, visual); assert_eq!(clipboard(visual), "");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
    });
}

#[test]
fn bookmark_edits_history_codec_retains_arbitrary_edits_after_restore() {
    with_source("é😀\r\nkeep\r\n中", &[0], |workspace, visual| {
        dispatch(BookmarkCommand::Remove, visual);
        let (mut restored, rope, revision) = visual.update(|_, cx| {
            let w = workspace.read(cx);
            (crate::buffer::deserialize_undo_stack(&crate::buffer::serialize_undo_stack(&w.tabs[0].undo_stack)), w.buffer().rope.clone(), w.buffer().raw_revision.clone())
        });
        restored = crate::buffer::retain_replayable_undo(restored, &rope);
        crate::buffer::rebind_undo_revisions(&mut restored, &revision);
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().undo_stack = restored);
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "é😀\r\nkeep\r\n中"));
    });
}

#[test]
fn bookmark_edits_identical_nonempty_paste_changes_saved_identity_and_has_one_undo() {
    for (encoding, ending) in [(Encoding::Utf8Bom, LineEnding::Crlf), (Encoding::Utf16Be, LineEnding::Cr)] {
        let text = format!("é😀{}é😀", ending.as_str());
        let original = crate::text::encoding::encode(&text, encoding);
        let dir = crate::persist::TempDir::new("bookmark-identical-paste"); let path = dir.join("neutral.txt");
        std::fs::write(&path, &original).unwrap(); let buffer = crate::io::read_buffer(&path).unwrap(); let loaded = buffer.rope.to_string();
        with_source(&loaded, &[], |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, cx| {
                let tab = w.active_tab_mut(); tab.buffer = buffer; tab.selection.collapse_to(0); tab.observe_content_activity();
                tab.buffer.bookmarks.extend([0, tab.buffer.rope.line_to_byte(1)]);
                w.replace_byte_range(0..0, "x", cx);
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert!(!w.buffer().dirty); assert_eq!(w.tabs[0].redo_stack.len(), 1); });
            visual.write_to_clipboard(ClipboardItem::new_string("é😀".into())); dispatch(BookmarkCommand::Paste, visual);
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), loaded); assert!(w.buffer().dirty);
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].redo_stack.is_empty());
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), loaded); assert!(!w.buffer().dirty); });
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update_window_entity(workspace, |w, _, _| { assert!(w.buffer().dirty); w.write_active_to_disk().unwrap(); assert!(!w.buffer().dirty); });
            assert_eq!(std::fs::read(&path).unwrap(), original);
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), loaded); assert!(w.buffer().dirty); });
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        });
    }
}

#[test]
#[ignore = "manual optimized Windows caller timing and controls"]
fn bookmark_edits_release_large_callers_and_cancel_stale_controls() {
    let source = "0123456789012345678901234567890123456789é😀\r\n".repeat(262_145);
    assert!(source.len() > 10 * 1024 * 1024);
    let bookmarks: Vec<_> = (0..262_145).step_by(26).map(|row| row * 48).collect();
    for command in [BookmarkCommand::Cut, BookmarkCommand::Paste, BookmarkCommand::Remove, BookmarkCommand::RemoveNon] {
        with_source(&source, &bookmarks, |workspace, visual| {
            visual.write_to_clipboard(ClipboardItem::new_string("Pé😀\r\nQ".into()));
            visual.update_window_entity(workspace, |w, _, _| {
                for row in (1..262_145).step_by(26) { let start = row * 48; w.active_tab_mut().buffer.marks.push(start..start+4); }
            });
            dispatch(command, visual);
            let after = visual.update(|_, cx| workspace.read(cx).buffer().rope.to_string());
            assert_ne!(after, source);
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
            visual.dispatch_action(EditRedo); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), after);
                assert_eq!(w.prepared_timings.len(), 3);
                for (operation, timing) in ["edit", "undo", "redo"].into_iter().zip(&w.prepared_timings) {
                    println!("bookmark command={} {operation} source={} output={} capture={:.3}ms worker={:.3}ms publish={:.3}ms clipboard={:.3}ms",
                        command as usize, source.len(), after.len(), timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000., timing.clipboard.as_secs_f64()*1000.);
                }
            });
        });
        for stale in [false, true] {
            with_source(&source, &bookmarks, |workspace, visual| {
                let (send, receive) = futures::channel::oneshot::channel();
                visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
                match command {
                    BookmarkCommand::Cut => visual.dispatch_action(CutBookmarkedLines),
                    BookmarkCommand::Paste => visual.dispatch_action(PasteToBookmarkedLines),
                    BookmarkCommand::Remove => visual.dispatch_action(RemoveBookmarkedLines),
                    BookmarkCommand::RemoveNon => visual.dispatch_action(RemoveNonBookmarkedLines),
                    BookmarkCommand::Inverse => {},
                }
                visual.run_until_parked();
                visual.update_window_entity(workspace, |w, window, cx| {
                    let start = std::time::Instant::now();
                    if stale { w.active_tab_mut().instance = Arc::new(()); } else { w.close_find(&EditFindClose, window, cx); }
                    println!("bookmark command={} stale={stale} control={:.3}ms", command as usize, start.elapsed().as_secs_f64()*1000.);
                });
                send.send(()).unwrap(); visual.run_until_parked();
                assert_eq!(clipboard(visual), "sentinel");
                visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), source); assert!(w.tabs[0].undo_stack.is_empty()); });
            });
        }
    }
    let row = "0123456789".repeat(100) + "é😀\r\n";
    let source = row.repeat(12_000); let bookmarks: Vec<_> = (0..12_000).map(|line| line * row.len()).collect();
    with_source(&source, &bookmarks, |workspace, visual| {
        dispatch(BookmarkCommand::Cut, visual); assert_eq!(clipboard(visual), source);
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.len_bytes(), 0);
            let timing = w.prepared_timings.last().unwrap();
            println!("bookmark full-copy Cut source={} capture={:.3}ms worker={:.3}ms publish={:.3}ms clipboard_test_platform={:.3}ms",
                source.len(), timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000., timing.clipboard.as_secs_f64()*1000.);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked(); visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
    });
    let paste = "é😀\n".repeat(1_500_000);
    with_source(&source, &[0], |workspace, visual| {
        visual.write_to_clipboard(ClipboardItem::new_string(paste.clone()));
        let started = std::time::Instant::now(); visual.dispatch_action(PasteToBookmarkedLines); let dispatch = started.elapsed(); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert_eq!(w.buffer().rope.len_bytes(), source.len() - row.len() + 2 + paste.len());
            assert_eq!(w.buffer().rope.byte_slice(0..paste.len()).to_string(), paste);
            let timing = w.prepared_timings.last().unwrap();
            println!("bookmark large Clipboard Paste source={} clipboard={} dispatch_including_test_platform_read={:.3}ms capture={:.3}ms worker={:.3}ms publish={:.3}ms",
                source.len(), paste.len(), dispatch.as_secs_f64()*1000., timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000.);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked(); visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
    });
}
