use super::Workspace;
use crate::{settings::Settings, tab::Tab};
use gpui::{AppContext, ElementInputHandler, InputHandler, Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

fn with_source(text: &str, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _execution = crate::macros::override_execution_for_test(false);
    let _settings = crate::settings::override_for_test(Settings {
        indent_uses_tabs: false, tab_width: 4, ..Default::default()
    });
    let mut cx = TestAppContext::single();
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.raw_revision = crate::buffer::ContentRevision::default();
        tab.buffer.forget_savepoint();
        tab.selection.head = text.len();
        w.tabs = vec![tab];
        w.active = 0;
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&workspace, visual);
    cx.quit();
}

#[test]
fn large_manual_indent_action_publishes_one_complete_undoable_change() {
    let text = "café😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "    café😀\r\n".repeat(5000));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.tabs[0].edit_transaction.is_none());
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), text));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "    café😀\r\n".repeat(5000)));
    });
}

#[test]
fn annotation_heavy_small_selection_prepares_before_mutating_the_source() {
    let text = "café😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].selection.head = 512 * 11;
            for row in 0..64 {
                w.tabs[0].buffer.bookmarks.insert(row * 11);
                w.tabs[0].buffer.marks.push(row * 11+1..row * 11+3);
            }
        });
        let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
        });
        send.send(()).unwrap(); visual.run_until_parked();
        visual.update(|_, cx| assert!(matches!(workspace.read(cx).tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Prepared(_)))));
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
    });
}

#[test]
fn oversized_annotation_capture_refuses_before_work_and_preserves_source() {
    let text = "a\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].buffer.marks = std::iter::repeat_n(0..1, 65_537).collect();
        });
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.pending_prepared_edit.is_none());
            assert!(w.tabs[0].undo_stack.is_empty());
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.status_message.as_deref().is_some_and(|message| message.contains("Clear marks or bookmarks")));
        });
    });
}

#[test]
fn oversized_current_annotations_refuse_prepared_replay_without_popping_history() {
    let text = "a\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        for undo in [true, false] {
            visual.update_window_entity(workspace, |w, _, _| {
                w.tabs[0].buffer.marks = std::iter::repeat_n(0..1,
                    super::prepared_edits::MAX_PREPARED_ANNOTATIONS + 1).collect();
            });
            if undo { visual.dispatch_action(crate::actions::EditUndo); }
            else { visual.dispatch_action(crate::actions::EditRedo); }
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.pending_prepared_edit.is_none());
                assert_eq!(w.tabs[0].undo_stack.len(), usize::from(undo));
                assert_eq!(w.tabs[0].redo_stack.len(), usize::from(!undo));
                let expected = if undo { "    a\n".repeat(5000) } else { text.clone() };
                assert!(w.buffer().rope.chars().eq(expected.chars()));
                assert_eq!(w.buffer().marks.len(), super::prepared_edits::MAX_PREPARED_ANNOTATIONS + 1);
                assert!(w.status_message.as_deref().is_some_and(|message| message.contains("Clear marks or bookmarks")));
            });
            visual.update_window_entity(workspace, |w, _, _| w.tabs[0].buffer.marks.clear());
            if undo { visual.dispatch_action(crate::actions::EditUndo); }
            else { visual.dispatch_action(crate::actions::EditRedo); }
            visual.run_until_parked();
        }
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq("    a\n".repeat(5000).chars())));
    });
}

#[test]
fn rebound_catalog_indent_key_handles_more_than_foreground_cap() {
    let text = "a\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        let mut overrides = crate::shortcuts::Overrides::new();
        overrides.insert("EditIncreaseLineIndent".into(), vec!["ctrl-alt-shift-i".into()]);
        visual.update(|_, cx| crate::shortcuts::reload_keymap(cx, &overrides));
        visual.simulate_keystrokes("ctrl-alt-shift-i");
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "    a\n".repeat(5000)));
    });
}

#[test]
fn prepared_controller_publishes_and_replays_sealed_candidate_before_cap_routing() {
    let text = "é😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| assert!(w.start_prepared_indent(true, window, cx)));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq("    é😀\r\n".repeat(5000).chars()));
            assert!(matches!(w.tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Prepared(_))));
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq("    é😀\r\n".repeat(5000).chars())));
    });
}

#[test]
fn prepared_controller_failed_publication_reserve_preserves_source_and_history() {
    let text = "é😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            w.prepared_fail_reserve = true;
            assert!(w.start_prepared_indent(true, window, cx));
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
            assert!(w.tabs[0].redo_stack.is_empty());
            assert!(w.status_message.as_deref().unwrap().contains("memory"));
        });
    });
}

fn hold_action(workspace: &Entity<Workspace>, visual: &mut VisualTestContext,
    action: impl gpui::Action) -> futures::channel::oneshot::Sender<()> {
    let (send, receive) = futures::channel::oneshot::channel();
    visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
    visual.dispatch_action(action);
    visual.run_until_parked();
    visual.update(|_, cx| assert!(workspace.read(cx).pending_prepared_edit.is_some()));
    send
}

#[test]
fn interleaved_real_typing_retires_indent_and_undo_without_losing_the_new_text() {
    for (replay, completed) in [(false, false), (true, false), (false, true), (true, true)] {
        let text = "é😀\n".repeat(5000);
        with_source(&text, |workspace, visual| {
            if replay {
                visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
                visual.run_until_parked();
            }
            let send = if completed {
                let (send, receive) = futures::channel::oneshot::channel();
                visual.update_window_entity(workspace, |w, _, _| w.prepared_test_publish_gate = Some(receive));
                if replay { visual.dispatch_action(crate::actions::EditUndo); }
                else { visual.dispatch_action(crate::actions::EditIncreaseLineIndent); }
                visual.run_until_parked();
                send
            } else if replay { hold_action(workspace, visual, crate::actions::EditUndo) }
                else { hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent) };
            visual.simulate_keystrokes("z");
            visual.run_until_parked();
            send.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), "z");
                assert_eq!(w.tabs[0].undo_stack.len(), if replay { 2 } else { 1 });
                assert!(w.tabs[0].redo_stack.is_empty());
                assert!(w.pending_prepared_edit.is_none());
                assert!(matches!(w.tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Single(_))));
            });
        });
    }
}

#[test]
fn changed_metadata_and_source_context_refuse_the_candidate_atomically() {
    for change in ["bookmarks", "marks", "marker", "encoding", "eol", "saved", "path", "selection", "readonly", "loading", "transaction", "tab", "preview", "field", "inactive", "settings"] {
        let text = "é😀\n".repeat(5000);
        with_source(&text, |workspace, visual| {
            let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
            visual.update_window_entity(workspace, |w, window, cx| {
                let tab = &mut w.tabs[0];
                match change {
                    "bookmarks" => { tab.buffer.bookmarks.insert(700); },
                    "marks" => tab.buffer.marks.push(700..706),
                    "marker" => tab.begin_select_marker = Some(702),
                    "encoding" => tab.buffer.encoding = crate::text::encoding::Encoding::Utf16Le,
                    "eol" => tab.buffer.line_ending = crate::text::line_ending::LineEnding::Cr,
                    "saved" => tab.buffer.mark_saved(),
                    "path" => tab.buffer.file_path = Some(std::env::temp_dir().join("neutral-new-path.txt")),
                    "selection" => tab.selection.collapse_to(2),
                    "readonly" => tab.buffer.read_only = true,
                    "loading" => tab.buffer.loading = Some(crate::buffer::LoadProgress {
                        bytes_read: 0, total_bytes: 12_000_000, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }),
                    "transaction" => w.begin_edit_group(),
                    "tab" => { w.tabs.push(Tab::empty()); w.active = 1; },
                    "preview" => tab.view_mode = crate::tab::TabViewMode::MarkdownPreview,
                    "field" => { w.goto_state.open = true; window.focus(&w.goto_focus, cx); },
                    _ => {},
                }
                cx.notify();
            });
            if change == "inactive" { visual.deactivate_window(); }
            let _changed_settings = (change == "settings").then(|| crate::settings::override_for_test(Settings {
                indent_uses_tabs: true, tab_width: 8, ..Default::default()
            }));
            send.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|window, cx| {
                let w = workspace.read(cx);
                assert!(w.tabs[0].buffer.rope.chars().eq(text.chars()), "{change}");
                assert!(w.tabs[0].undo_stack.is_empty(), "{change}");
                assert!(w.tabs[0].redo_stack.is_empty(), "{change}");
                if change == "bookmarks" { assert!(w.tabs[0].buffer.bookmarks.contains(&700)); }
                if change == "marks" { assert_eq!(w.tabs[0].buffer.marks, vec![700..706]); }
                if change == "marker" { assert_eq!(w.tabs[0].begin_select_marker, Some(702)); }
                if change == "field" { assert!(w.goto_focus.is_focused(window)); }
            });
        });
    }
}

#[test]
fn undo_and_redo_remap_bookmarks_and_marks_added_after_the_large_edit() {
    let text = "é😀\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        let after_selection = visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].buffer.bookmarks.insert(33000);
            w.tabs[0].buffer.marks.push(33004..33010);
            w.tabs[0].begin_select_marker = Some(33006);
            w.selection()
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().bookmarks, [21000].into_iter().collect());
            assert_eq!(w.buffer().marks, vec![21000..21006]);
            assert_eq!(w.tabs[0].begin_select_marker, Some(21002));
            assert_eq!((w.selection().anchor, w.selection().head), (0, text.len()));
        });
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().bookmarks, [33000].into_iter().collect());
            assert_eq!(w.buffer().marks, vec![33004..33010]);
            assert_eq!(w.tabs[0].begin_select_marker, Some(33006));
            assert_eq!((w.selection().anchor, w.selection().head), (after_selection.anchor, after_selection.head));
        });
    });
}

#[test]
fn large_noop_and_failed_replay_reserve_keep_the_existing_timeline() {
    let text = "é😀\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].selection.collapse_to(2));
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].selection.anchor = 0; w.tabs[0].selection.head = text.len(); });
        visual.dispatch_action(crate::actions::EditDecreaseLineIndent);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.tabs[0].undo_stack.is_empty());
            assert_eq!(w.tabs[0].redo_stack.len(), 1);
            assert!(w.buffer().rope.chars().eq(text.chars()));
        });
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update_window_entity(workspace, |w, _, _| w.prepared_fail_reserve = true);
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq("    é😀\n".repeat(5000).chars()));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.tabs[0].redo_stack.is_empty());
        });
        visual.update_window_entity(workspace, |w, _, _| w.prepared_fail_reserve = false);
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
    });
}

#[test]
fn large_and_annotation_costly_recording_group_and_macro_playback_refuse_atomically() {
    for costly_small in [false, true] {
        let text = "é😀\n".repeat(5000);
        with_source(&text, |workspace, visual| {
            if costly_small {
                visual.update_window_entity(workspace, |w, _, _| {
                    w.tabs[0].selection.head = 512 * 7;
                    for row in 0..64 {
                        w.tabs[0].buffer.bookmarks.insert(row * 7);
                        w.tabs[0].buffer.marks.push(row * 7..row * 7+2);
                    }
                });
            }
            visual.dispatch_action(crate::actions::MacroStartRecording);
            visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
            visual.dispatch_action(crate::actions::MacroStopRecording);
            visual.run_until_parked();
            assert!(crate::macros::current().is_empty());
            visual.update_window_entity(workspace, |w, window, cx| {
                w.begin_edit_group();
                assert!(!w.run_menu_line_command(crate::edit_ops::menu_line_commands::MenuLineCommand::Increase, window, cx));
                w.end_edit_group(cx);
                assert!(!w.replay_macro(&[crate::macros::MacroEvent::IncreaseLineIndent, crate::macros::MacroEvent::InsertText("later".into())], 1, window, cx));
                // A legacy macro opens no group: its actual replay state must
                // still refuse costly indentation after a harmless Undo.
                assert!(!w.replay_macro(&[crate::macros::MacroEvent::Undo, crate::macros::MacroEvent::IncreaseLineIndent, crate::macros::MacroEvent::InsertText("later".into())], 1, window, cx));
                assert!(w.buffer().rope.chars().eq(text.chars()));
                assert!(w.tabs[0].undo_stack.is_empty());
                assert!(w.pending_prepared_edit.is_none());
            });
            visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
            visual.run_until_parked();
            let rows = if costly_small { 512 } else { 5000 };
            let expected = format!("{}{}", "    é😀\n".repeat(rows), "é😀\n".repeat(5000 - rows));
            visual.update_window_entity(workspace, |w, window, cx| {
                assert!(!w.replay_macro(&[crate::macros::MacroEvent::Undo, crate::macros::MacroEvent::InsertText("later".into())], 1, window, cx));
                assert_eq!(w.tabs[0].undo_stack.len(), 1);
                assert!(w.tabs[0].redo_stack.is_empty());
                assert!(w.buffer().rope.chars().eq(expected.chars()));
            });
        });
    }
}

#[test]
fn foreground_annotation_budget_includes_boundary_and_keeps_modest_macros_synchronous() {
    let text = "é😀\n".repeat(4097);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].selection.head = 4096 * 7;
            w.tabs[0].buffer.bookmarks.extend([0, 7]);
            w.tabs[0].buffer.marks.extend([0..2, 7..9]);
        });
        visual.dispatch_action(crate::actions::MacroStartRecording);
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.dispatch_action(crate::actions::MacroStopRecording);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.pending_prepared_edit.is_none());
            assert!(matches!(w.tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Group { .. })));
            assert!(w.buffer().rope.chars().eq(format!("{}é😀\n", "    é😀\n".repeat(4096)).chars()));
        });
        assert!(matches!(crate::macros::current().as_slice(), [crate::macros::MacroEvent::IncreaseLineIndent]));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update_window_entity(workspace, |w, window, cx| {
            assert!(w.replay_macro(&[crate::macros::MacroEvent::IncreaseLineIndent], 1, window, cx));
            assert!(w.pending_prepared_edit.is_none());
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
    });
}

#[test]
fn source_ime_after_prepared_history_is_one_separate_unicode_edit() {
    let text = "é😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update_window_entity(workspace, |w, _, cx| { w.tabs[0].selection.collapse_to(4); cx.notify(); });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
            handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
            handler.replace_text_in_range(None, "日本😀", window, cx);
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.to_string().starts_with("    日本😀é😀\r\n"));
            assert_eq!(w.tabs[0].undo_stack.len(), 2);
            assert!(matches!(&w.tabs[0].undo_stack[0], crate::buffer::UndoEntry::Prepared(_)));
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq("    é😀\r\n".repeat(5000).chars())));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.to_string().starts_with("    日本😀é😀\r\n")));
    });
}

#[test]
fn prepared_saved_identity_and_same_depth_branch_follow_actual_file_save() {
    let directory = std::env::temp_dir().join(format!("notepad-studio-prepared-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("unicode.txt");
    let text = "é😀\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].buffer.file_path = Some(path.clone());
            w.tabs[0].buffer.encoding = crate::text::encoding::Encoding::Utf16Be;
            w.tabs[0].buffer.line_ending = crate::text::line_ending::LineEnding::Crlf;
        });
        visual.dispatch_action(crate::actions::FileSave);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().dirty));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::FileSave);
        visual.run_until_parked();
        let reopened = crate::io::read_buffer(&path).unwrap();
        assert!(reopened.rope.chars().eq("    é😀\n".repeat(5000).chars()));
        assert_eq!(reopened.encoding, crate::text::encoding::Encoding::Utf16Be);
        assert_eq!(reopened.line_ending, crate::text::line_ending::LineEnding::Crlf);
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().dirty));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.simulate_keystrokes("z");
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "z");
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.tabs[0].redo_stack.is_empty());
            assert!(w.buffer().dirty, "equal stack depth is a different saved identity");
        });
    });
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn hostile_selection_and_recording_started_during_worker_preserve_source() {
    let text = "é😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].selection.anchor = usize::MAX;
            w.tabs[0].selection.head = 1;
        });
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq("    é😀\r\n".repeat(5000).chars()));
            assert!(w.selection().anchor > w.selection().head);
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!((w.selection().anchor, w.selection().head), (text.len(), 0));
        });
        let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
        visual.dispatch_action(crate::actions::MacroStartRecording);
        visual.run_until_parked();
        send.send(()).unwrap();
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::MacroStopRecording);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
            assert_eq!(w.tabs[0].redo_stack.len(), 1);
        });
        assert!(crate::macros::current().is_empty());
    });
}

#[test]
fn prepared_large_file_timing_and_atomic_correctness() {
    let text = "0123456789012345678901234567890123456789é😀\r\n".repeat(262145);
    assert!(text.len() > 10 * 1024 * 1024);
    println!("bulk record budget: {} bytes for 262145 edits and 524288 plan slots; text bytes capped separately at 32 MiB; history total at 128 MiB",
        crate::buffer::prepared::history_record_budget(262145, 524288).unwrap());
    let samples = std::env::var("NOTEPAD_STUDIO_BULK_BENCH").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).clamp(1, 30);
    for annotations in [false, true] {
        with_source(&text, |workspace, visual| {
            if annotations {
                visual.update_window_entity(workspace, |w, _, _| {
                    for row in (0..262145).step_by(26) {
                        let start = row * 48;
                        w.tabs[0].buffer.bookmarks.insert(start);
                        w.tabs[0].buffer.marks.push(start..start+4);
                    }
                });
            }
            for _ in 0..samples {
                visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
                visual.run_until_parked();
                visual.dispatch_action(crate::actions::EditUndo);
                visual.run_until_parked();
                visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
                visual.dispatch_action(crate::actions::EditRedo);
                visual.run_until_parked();
                visual.dispatch_action(crate::actions::EditUndo);
                visual.run_until_parked();
            }
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                for (operation, index) in [("indent", 0), ("undo", 1), ("redo", 2)] {
                    let measurements: Vec<_> = w.prepared_timings.iter().skip(index).step_by(4).collect();
                    let report = |label: &str, field: fn(&super::prepared_edits::Timing) -> std::time::Duration| {
                        let mut times: Vec<f64> = measurements.iter().map(|timing| field(timing).as_secs_f64() * 1000.).collect();
                        times.sort_by(f64::total_cmp);
                        println!("bulk {operation} annotations={annotations} {label} median={:.3}ms p95={:.3}ms n={}",
                            times[times.len()/2], times[(times.len()*95).div_ceil(100).saturating_sub(1)], times.len());
                    };
                    report("capture", |t| t.capture);
                    report("worker", |t| t.worker);
                    report("publish", |t| t.publish);
                }
            });
            let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
            visual.update_window_entity(workspace, |w, _, cx| {
                let start = std::time::Instant::now();
                w.cancel_prepared_edit(cx);
                println!("bulk cancellation annotations={annotations} foreground={:.3}ms", start.elapsed().as_secs_f64()*1000.);
            });
            send.send(()).unwrap();
            visual.run_until_parked();
            let (send, recv) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, _, _| w.prepared_test_publish_gate = Some(recv));
            visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
            visual.run_until_parked(); // Complete the large candidate before cancellation.
            visual.update_window_entity(workspace, |w, _, cx| {
                let start = std::time::Instant::now();
                w.cancel_prepared_edit(cx);
                println!("bulk completed-candidate cancellation annotations={annotations} foreground={:.3}ms", start.elapsed().as_secs_f64()*1000.);
            });
            send.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
        });
    }
}

#[test]
fn bounded_small_indent_path_release_comparison_on_large_file() {
    if std::env::var_os("NOTEPAD_STUDIO_BULK_BENCH").is_none() { return; }
    let text = "0123456789012345678901234567890123456789é😀\r\n".repeat(262145);
    for annotations in [0, 4, 20_166] {
        with_source(&text, |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                w.tabs[0].selection.head = 4096 * 48;
                if annotations == 4 {
                    w.tabs[0].buffer.bookmarks.extend([0, 48]);
                    w.tabs[0].buffer.marks.extend([0..4, 48..52]);
                } else if annotations > 4 {
                    for row in (0..262145).step_by(26) {
                        w.tabs[0].buffer.bookmarks.insert(row * 48);
                        w.tabs[0].buffer.marks.push(row * 48..row * 48+4);
                    }
                }
            });
            for name in ["indent", "undo", "redo"] {
                visual.update_window_entity(workspace, |w, window, cx| {
                    let start = std::time::Instant::now();
                    match name {
                        "indent" => w.increase_line_indent(&crate::actions::EditIncreaseLineIndent, window, cx),
                        "undo" => w.undo(&crate::actions::EditUndo, window, cx),
                        "redo" => w.redo(&crate::actions::EditRedo, window, cx),
                        _ => unreachable!(),
                    }
                    println!("bulk bounded-small-path {name} annotations={annotations} foreground={:.3}ms", start.elapsed().as_secs_f64()*1000.);
                });
                visual.run_until_parked();
            }
        });
    }
}

#[test]
fn prepared_limit_and_long_line_release_publication_samples() {
    if std::env::var_os("NOTEPAD_STUDIO_BULK_BENCH").is_none() { return; }
    let _execution = crate::macros::override_execution_for_test(false);
    let _settings = crate::settings::override_for_test(Settings { indent_uses_tabs: false, tab_width: 4, ..Default::default() });
    for sample in ["annotation-limit", "mark-limit", "long-line"] {
        let mut text = "a\n".repeat(5000);
        text.push_str(&"x".repeat(12 * 1024 * 1024));
        let mut cx = TestAppContext::single();
        let workspace = cx.new(|cx| {
            let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
            w.tabs = vec![Tab::empty()]; w.active = 0; w.window_closed = true;
            w.tabs[0].buffer.rope = ropey::Rope::from_str(&text);
            w.tabs[0].buffer.raw_revision = crate::buffer::ContentRevision::default();
            w.tabs[0].buffer.forget_savepoint();
            w.tabs[0].selection.head = text.len();
            w
        });
        let visual = cx.add_empty_window(); // Measure the handler callback, excluding long-line shaping.
        visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
        visual.run_until_parked();
        if sample != "long-line" {
            let count = super::prepared_edits::MAX_PREPARED_ANNOTATIONS;
            visual.update_window_entity(&workspace, |w, _, _| {
                if sample == "annotation-limit" { w.tabs[0].buffer.bookmarks = (0..count).collect(); }
                else { w.tabs[0].buffer.marks = std::iter::repeat_n(0..1, count).collect(); }
            });
            let (send, recv) = futures::channel::oneshot::channel();
            visual.update_window_entity(&workspace, |w, window, cx| {
                w.prepared_test_gate = Some(recv);
                let start = std::time::Instant::now();
                w.increase_line_indent(&crate::actions::EditIncreaseLineIndent, window, cx);
                println!("bulk {sample} capture count={count} foreground={:.3}ms", start.elapsed().as_secs_f64()*1000.);
                assert!(w.pending_prepared_edit.is_some(), "{:?}", w.status_message);
                w.cancel_prepared_edit(cx);
            });
            send.send(()).unwrap(); visual.run_until_parked();
            for name in ["indent", "undo", "redo"] {
                visual.update_window_entity(&workspace, |w, window, cx| match name {
                    "indent" => w.increase_line_indent(&crate::actions::EditIncreaseLineIndent, window, cx),
                    "undo" => w.undo(&crate::actions::EditUndo, window, cx),
                    "redo" => w.redo(&crate::actions::EditRedo, window, cx),
                    _ => unreachable!(),
                });
                visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx);
                    let timing = w.prepared_timings.last().unwrap();
                    println!("bulk {sample} {name} capture={:.3}ms worker={:.3}ms publish={:.3}ms",
                        timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000.);
                    assert_eq!(w.buffer().rope.len_bytes(), text.len() + if name == "undo" { 0 } else { 5001 * 4 });
                    if sample == "mark-limit" { assert_eq!(w.buffer().marks.len(), count); }
                });
            }
        } else {
            visual.update_window_entity(&workspace, |w, window, cx| w.increase_line_indent(&crate::actions::EditIncreaseLineIndent, window, cx));
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                let timing = w.prepared_timings.last().unwrap();
                println!("bulk long-line handler-only capture={:.3}ms worker={:.3}ms publish={:.3}ms",
                    timing.capture.as_secs_f64()*1000., timing.worker.as_secs_f64()*1000., timing.publish.as_secs_f64()*1000.);
                assert_eq!(w.buffer().rope.len_bytes(), text.len() + 5001 * 4);
            });
        }
        cx.quit();
    }
}

#[test]
fn direct_rope_mutation_without_revision_cannot_publish_or_replay_a_prepared_edit() {
    for replay in [false, true] {
        let text = "a\n".repeat(5000);
        with_source(&text, |workspace, visual| {
            if replay {
                visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
                visual.run_until_parked();
            }
            let send = if replay { None } else { Some(hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent)) };
            visual.update_window_entity(workspace, |w, _, cx| {
                let rope = &mut w.tabs[0].buffer.rope;
                let start = if replay { 4 } else { 0 };
                rope.remove(start..start+1);
                rope.insert(start, "z");
                cx.notify();
            });
            if replay { visual.dispatch_action(crate::actions::EditUndo); }
            if let Some(send) = send { send.send(()).unwrap(); }
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.byte_slice(0..if replay { 5 } else { 1 }).to_string(), if replay { "    z" } else { "z" });
                assert_eq!(w.tabs[0].undo_stack.len(), usize::from(replay));
                assert!(w.tabs[0].redo_stack.is_empty());
            });
        });
    }
}

#[test]
fn caret_and_focus_aba_retire_pending_prepared_source_work() {
    for transition in ["caret", "focus", "tab"] {
        let text = "a\n".repeat(5000);
        with_source(&text, |workspace, visual| {
            let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
            visual.update_window_entity(workspace, |w, window, cx| {
                match transition {
                    "caret" => {
                        w.move_to_byte(0, false, true);
                        w.commit_caret(cx);
                        w.tabs[0].selection.anchor = 0;
                        w.tabs[0].selection.head = text.len();
                        w.commit_caret(cx);
                    }
                    "focus" => { window.focus(&w.goto_focus, cx); window.focus(&w.focus_handle, cx); }
                    "tab" => {
                        w.tabs.push(Tab::empty());
                        w.set_active_tab(1, cx);
                        w.set_active_tab(0, cx);
                    }
                    _ => unreachable!(),
                }
                cx.notify();
            });
            send.send(()).unwrap();
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.buffer().rope.chars().eq(text.chars()), "{transition}");
                assert!(w.tabs[0].undo_stack.is_empty(), "{transition}");
            });
        });
    }
}

#[test]
fn csv_identity_only_source_restore_preserves_shared_prepared_replay() {
    let text = "a,b\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        let shared = visual.update(|_, cx| {
            let crate::buffer::UndoEntry::Prepared(group) = &workspace.read(cx).tabs[0].undo_stack[0] else { panic!("prepared history"); };
            group.clone()
        });
        let expected = shared.expected(true).clone();
        visual.update_window_entity(workspace, |w, _, cx| {
            let mut model = crate::ui::csv_doc::CsvModel::empty(b',');
            model.bind_source(&w.buffer().rope, &w.buffer().raw_revision);
            model.pending_source = Some(crate::ui::csv_doc::undo::CsvSourceState {
                rope: w.buffer().rope.clone(), revision: crate::buffer::ContentRevision::default(),
            });
            w.csv_models.insert(w.tabs[0].session_id, model);
            crate::ui::csv_doc::actions::emit_to_source(w, cx);
            assert_eq!(w.tabs[0].undo_stack.len(), 2, "identity-only restoration records a Source op");
            assert!(matches!(w.tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Single(_))));
            let crate::buffer::UndoEntry::Prepared(group) = &w.tabs[0].undo_stack[0] else { panic!("prepared history"); };
            assert!(std::sync::Arc::ptr_eq(group, &shared));
            assert_eq!(shared.expected(true), &expected);
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 2));
        assert_eq!(shared.expected(true), &expected);
    });
}

#[test]
fn prepared_publication_invalidates_find_without_flattening_large_document() {
    let text = "café😀\r\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.find_state.open = true;
            w.find_state.query = "café".into();
            w.find_state.matches.clear();
            w.find_state.matches.push(0..5);
            w.find_state.current = Some(0);
        });
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().flat_text_cache.is_none());
            assert!(w.find_state.matches.is_empty());
            assert!(w.find_state.current.is_none());
            assert_eq!(w.find_state.status.as_deref(), Some("Document changed; run Find again to refresh matches"));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
    });
}

#[test]
fn closed_window_discards_pending_and_completed_indentation_without_history_changes() {
    for completed in [false, true] {
        let text = "a\n".repeat(5000);
        with_source(&text, |workspace, visual| {
            let send = if completed {
                let (send, recv) = futures::channel::oneshot::channel();
                visual.update_window_entity(workspace, |w, _, _| w.prepared_test_publish_gate = Some(recv));
                visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
                visual.run_until_parked();
                send
            } else { hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent) };
            visual.update(|window, _| window.remove_window());
            visual.cx.run_until_parked();
            send.send(()).unwrap();
            visual.cx.run_until_parked();
            visual.cx.update(|cx| {
                let w = workspace.read(cx);
                assert!(w.buffer().rope.chars().eq(text.chars()));
                assert!(w.tabs[0].undo_stack.is_empty());
                assert!(w.pending_prepared_edit.is_none());
            });
        });
    }
}

#[test]
fn source_escape_cancels_completed_prepared_candidate_without_changing_history() {
    let text = "a\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update(|_, cx| crate::shortcuts::reload_keymap(cx, &crate::shortcuts::Overrides::new()));
        let (send, recv) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, _, _| w.prepared_test_publish_gate = Some(recv));
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        send.send(()).unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
            assert!(w.pending_prepared_edit.is_none());
            assert_eq!(w.status_message.as_deref(), Some("Line indentation cancelled; no changes applied"));
        });
    });
}

#[test]
fn source_escape_closes_find_in_files_before_cancelling_indentation() {
    let text = "a\n".repeat(5000);
    with_source(&text, |workspace, visual| {
        visual.update(|_, cx| crate::shortcuts::reload_keymap(cx, &crate::shortcuts::Overrides::new()));
        let (send, recv) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, _, _| w.prepared_test_publish_gate = Some(recv));
        visual.dispatch_action(crate::actions::EditIncreaseLineIndent);
        visual.run_until_parked();
        visual.update_window_entity(workspace, |w, _, _| w.find_in_files.open = true);
        visual.run_until_parked();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(!w.find_in_files.open);
            assert!(w.pending_prepared_edit.is_some());
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
        });
        send.send(()).unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.pending_prepared_edit.is_none());
            assert_eq!(w.buffer().rope.to_string(), "    a\n".repeat(5000));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
    });
}

#[test]
fn long_prefixes_on_few_rows_prepare_before_changing_source() {
    let row = format!("{}é😀\r\n", " ".repeat(128 * 1024));
    let text = row.repeat(96);
    assert!(text.len() > 10 * 1024 * 1024);
    with_source(&text, |workspace, visual| {
        let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
        });
        send.send(()).unwrap();
        visual.run_until_parked();
        let expected = format!("    {row}").repeat(96);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(expected.chars()));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(matches!(w.tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Prepared(_))));
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(expected.chars())));
    });
}

#[test]
fn unicode_rope_row_inside_long_physical_line_prepares_before_changing_source() {
    let before = "x".repeat(512 * 1024);
    let text = format!("{before}\u{2028}é😀\u{2028}{}\r\n", "y".repeat(512 * 1024));
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].selection.collapse_to(before.len() + '\u{2028}'.len_utf8());
        });
        let send = hold_action(workspace, visual, crate::actions::EditIncreaseLineIndent);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.chars().eq(text.chars()));
            assert!(w.tabs[0].undo_stack.is_empty());
        });
        send.send(()).unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(format!("    {text}").chars())));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().rope.chars().eq(text.chars())));
    });
}

#[test]
fn small_selected_row_before_large_unicode_line_keeps_foreground_history() {
    let first = "\u{feff}é😀\r\n";
    let text = format!("{first}{}\u{2028}tail", "x".repeat(10 * 1024 * 1024));
    with_source(&text, |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            w.tabs[0].selection.head = first.len();
            w.increase_line_indent(&crate::actions::EditIncreaseLineIndent, window, cx);
            assert!(w.pending_prepared_edit.is_none());
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(!matches!(w.tabs[0].undo_stack.last(), Some(crate::buffer::UndoEntry::Prepared(_))));
            assert!(w.buffer().rope.chars().eq(format!("    {text}").chars()));
        });
    });
}

#[test]
fn few_row_large_prefix_release_comparison() {
    let Some(samples) = std::env::var("NOTEPAD_STUDIO_INDENT_BYTE_BENCH").ok()
        .and_then(|s| s.parse::<usize>().ok()).map(|n| n.clamp(1, 12)) else { return; };
    let row = format!("{}é😀\r\n", " ".repeat(128 * 1024));
    let text = row.repeat(96);
    assert!(text.len() > 10 * 1024 * 1024);
    for _ in 0..samples {
        with_source(&text, |workspace, visual| {
            visual.update_window_entity(workspace, |w, window, cx| {
                let started = std::time::Instant::now();
                w.increase_line_indent(&crate::actions::EditIncreaseLineIndent, window, cx);
                println!("few-row-prefix foreground={:.3}ms prepared={}",
                    started.elapsed().as_secs_f64() * 1000., w.pending_prepared_edit.is_some());
            });
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.buffer().rope.chars().eq(format!("    {row}").repeat(96).chars()));
                if let Some(timing) = w.prepared_timings.last() {
                    println!("few-row-prefix capture={:.3}ms worker={:.3}ms publish={:.3}ms",
                        timing.capture.as_secs_f64() * 1000., timing.worker.as_secs_f64() * 1000.,
                        timing.publish.as_secs_f64() * 1000.);
                }
            });
        });
    }
}
