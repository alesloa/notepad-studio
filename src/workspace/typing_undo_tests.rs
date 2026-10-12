// scintilla/src/UndoHistory.cxx:253-350; Editor.cxx:2087-2094.
// Installed Source callbacks and registered history actions, not simulate_input.
use crate::{actions::{EditPaste, EditRedo, EditUndo}, buffer::SavePoint,
    macros, settings::{AutoIndentMode, AutocompletionMode, Settings}, tab::Tab,
    text::{encoding::Encoding, line_ending::LineEnding}, workspace::Workspace};
use gpui::{ClipboardItem, Entity, InputHandler, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

fn with_source(test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        auto_indent_mode: AutoIndentMode::None, autocompletion_mode: AutocompletionMode::None,
        auto_insert_parens: false, auto_insert_brackets: false, auto_insert_braces: false,
        auto_insert_quotes: false, auto_insert_double_quotes: false, auto_insert_html_xml_tag: false,
        auto_insert_user_pairs: Vec::new(), ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let _macros = macros::override_playback_for_test(Vec::new(), Default::default());
    let _execution = macros::override_execution_for_test(false);
    let _clipboard = crate::clipboard_ownership::override_for_test();
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str("BASE");
        tab.buffer.encoding = Encoding::Utf8Bom; tab.buffer.line_ending = LineEnding::Crlf;
        tab.buffer.bookmarks.insert(0); tab.buffer.marks.push(0..2); tab.begin_select_marker = Some(1);
        tab.buffer.mark_saved(); tab.selection.collapse_to(4);
        w.tabs = vec![tab]; w.active = 0; w.word_wrap = false;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked(); visual.write_to_clipboard(ClipboardItem::new_string("typing sentinel".into()));
    test(&workspace, visual); cx.quit();
}

fn capture(visual: &mut VisualTestContext) -> Box<dyn InputHandler> {
    visual.update(|window, cx| { window.refresh(); let _ = window.draw(cx); window.take_registered_input_handler().expect("painted Source handler") })
}
fn type_chunks(handler: &mut dyn InputHandler, chunks: &[&str], visual: &mut VisualTestContext) {
    for chunk in chunks {
        visual.update(|window, cx| { assert!(handler.accepts_text_input(window, cx)); handler.replace_text_in_range(None, chunk, window, cx); });
        visual.run_until_parked();
    }
}
fn text(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> String {
    visual.update(|_, cx| workspace.read(cx).buffer().rope.to_string())
}
fn assert_metadata(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, saved: &Option<SavePoint>) {
    visual.update(|window, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[0];
        assert_eq!(w.active, 0); assert!(!w.window_closed); assert!(window.is_window_active());
        assert!(w.focus_handle.is_focused(window)); assert!(!w.is_composing_text(window));
        assert!(tab.buffer.savepoint == *saved); assert_eq!(tab.buffer.encoding, Encoding::Utf8Bom);
        assert_eq!(tab.buffer.line_ending, LineEnding::Crlf); assert_eq!(tab.buffer.bookmarks.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(tab.buffer.marks, vec![0..2]); assert_eq!(tab.begin_select_marker, Some(1));
        assert!(tab.block_selection.is_none()); assert!(tab.edit_transaction.is_none()); assert!(!tab.buffer.read_only);
    });
}
fn assert_run_undo(chunks: &[&str], suffix: &str) {
    with_source(|workspace, visual| {
        let (before_revision, saved) = visual.update(|_, cx| { let buffer = workspace.read(cx).buffer(); (buffer.raw_revision.clone(), buffer.savepoint.clone()) });
        let mut handler = capture(visual); type_chunks(handler.as_mut(), chunks, visual);
        assert_eq!(text(workspace, visual), format!("BASE{suffix}")); assert_metadata(workspace, visual, &saved);
        let after_revision = visual.update(|_, cx| {
            let w = workspace.read(cx); assert!(w.buffer().dirty);
            assert_eq!((w.selection().anchor, w.selection().head), (4 + suffix.len(), 4 + suffix.len()));
            eprintln!("installed callback chunks={} undo_slots={}", chunks.len(), w.tabs[0].undo_stack.len());
            w.buffer().raw_revision.clone()
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        let undone = visual.update(|_, cx| { let w = workspace.read(cx); (w.buffer().rope.to_string(), w.buffer().raw_revision.clone(), w.buffer().dirty, w.selection().anchor, w.selection().head) });
        assert_metadata(workspace, visual, &saved);
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        assert_eq!(text(workspace, visual), format!("BASE{suffix}"));
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().raw_revision, after_revision); assert!(w.buffer().dirty); });
        assert_metadata(workspace, visual, &saved);
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("typing sentinel"));
        assert_eq!(undone.0, "BASE", "one registered Undo must remove the complete adjacent insertion run");
        assert_eq!(undone.1, before_revision); assert!(!undone.2); assert_eq!((undone.3, undone.4), (4, 4));
    });
}

#[test]
fn typing_undo_g1_one_installed_callback_is_one_reversible_edit() { assert_run_undo(&["abc"], "abc"); }

#[test]
fn typing_undo_g1_adjacent_installed_callbacks_coalesce_across_turns() { assert_run_undo(&["a", "b", "c"], "abc"); }

#[test]
fn typing_undo_g1_adjacent_unicode_installed_callbacks_coalesce() { assert_run_undo(&["é", "中", "😀"], "é中😀"); }

// UndoHistory.cxx:281-282,391-393 — an established savepoint separates runs.
// This applies the buffer savepoint primitive, not a native file-save observation.
#[test]
fn typing_undo_g1_savepoint_separates_installed_insertions() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        visual.update_window_entity(workspace, |w, _, _| w.active_tab_mut().buffer.mark_saved());
        let saved = visual.update(|_, cx| workspace.read(cx).buffer().savepoint.clone());
        type_chunks(handler.as_mut(), &["b"], visual);
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty)); assert_metadata(workspace, visual, &saved);
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().dirty));
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEab");
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().dirty)); assert_metadata(workspace, visual, &saved);
    });
}

// ScintillaWin.cxx:3171-3175 and UndoHistory.cxx:335-350 — Paste is isolated.
#[test]
fn typing_undo_g1_registered_paste_separates_typing_runs() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        visual.write_to_clipboard(ClipboardItem::new_string("X".into()));
        visual.dispatch_action(EditPaste); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEaX");
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["b"], visual);
        assert_eq!(text(workspace, visual), "BASEaXb");
        for expected in ["BASEaX", "BASEa", "BASE"] { visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), expected); }
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        for expected in ["BASEa", "BASEaX", "BASEaXb"] { visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), expected); }
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("X"));
    });
}

#[test]
fn typing_undo_after_redo_resumes_the_same_insertion_run() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "b"], visual);
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        visual.dispatch_action(EditRedo); visual.run_until_parked();
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["c"], visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1);
            let crate::buffer::UndoEntry::Group { ops, sel_before_anchor, sel_before_head } = &w.tabs[0].undo_stack[0] else { panic!("typing group"); };
            assert_eq!(ops.len(), 3); assert_eq!((*sel_before_anchor, *sel_before_head), (4, 4));
            for pair in ops.windows(2) { assert_eq!(pair[0].after_revision, pair[1].before_revision); }
            assert_eq!(ops.iter().map(|op| op.inserted.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEabc");
    });
}

#[test]
fn typing_undo_group_reserve_refusal_falls_back_to_separate_history() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].typing_undo.fail_group_reserve = true);
        type_chunks(handler.as_mut(), &["b", "c"], visual);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 2));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
    });
}

#[test]
fn typing_undo_essential_reserve_refusal_preserves_draft_history_and_generations() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        visual.dispatch_action(EditUndo); visual.run_until_parked();
        let before = visual.update(|window, cx| {
            let w = workspace.read(cx); (super::typing_undo::HistoryStamp::capture(&w.tabs[0]),
                w.buffer().savepoint.clone(), w.selection(), w.autosave_generation, w.post_edit_generation,
                window.input_context_token())
        });
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].typing_undo.fail_history_reserve = true);
        let _recording = macros::override_execution_for_test(true);
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["x"], visual);
        assert!(!macros::stop_recording());
        visual.update(|window, cx| {
            let w = workspace.read(cx); assert!(before.0.matches(&w.tabs[0]));
            assert!(w.buffer().savepoint == before.1);
            assert_eq!((w.selection().anchor, w.selection().head), (before.2.anchor, before.2.head));
            assert_eq!((w.autosave_generation, w.post_edit_generation), (before.3, before.4));
            assert_eq!(window.input_context_token(), before.5); assert!(macros::recorded_events_for_test().expect("scoped macro execution").is_empty());
            assert!(w.status_message.as_deref().is_some_and(|s| s.contains("Cannot reserve typing Undo history")));
        });
        assert_eq!(text(workspace, visual), "BASE"); assert_metadata(workspace, visual, &before.1);
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("typing sentinel"));
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
    });
}

// Captured Save API/settlement model only; actual async Save caller controls belong to File.
#[test]
fn typing_undo_captured_save_seal_retains_success_and_failure_boundaries() {
    for success in [false, true] {
        with_source(|workspace, visual| {
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
            let captured = visual.update_window_entity(workspace, |w, _, _| {
                let tab = &mut w.tabs[0]; let revision = tab.buffer.raw_revision.clone();
                let stamp = super::typing_undo::HistoryStamp::capture(tab);
                assert!(!tab.seal_typing_undo_revision(&Default::default())); assert!(stamp.matches(tab));
                assert!(tab.seal_typing_undo_revision(&revision)); assert!(stamp.matches(tab));
                let mut saved_model = crate::buffer::Buffer::empty();
                saved_model.raw_revision = revision; saved_model.encoding = tab.buffer.encoding;
                saved_model.line_ending = tab.buffer.line_ending; saved_model.mark_saved();
                saved_model.savepoint
            });
            type_chunks(handler.as_mut(), &["b", "c"], visual);
            visual.update_window_entity(workspace, |w, _, _| {
                assert_eq!(w.tabs[0].undo_stack.len(), 2);
                if success { w.tabs[0].buffer.savepoint = captured; w.tabs[0].buffer.refresh_dirty(); }
            });
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().dirty, !success));
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().dirty, success));
            for expected in ["BASEa", "BASEabc"] { visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), expected); }
        });
    }
}

#[test]
fn typing_undo_exact_revision_seal_succeeds_without_an_open_run() {
    let mut tab = Tab::empty(); let revision = tab.buffer.raw_revision.clone();
    let before = super::typing_undo::HistoryStamp::capture(&tab);
    assert!(tab.seal_typing_undo_revision(&revision)); assert!(before.matches(&tab));
    assert!(!tab.seal_typing_undo_revision(&Default::default())); assert!(before.matches(&tab));
}

#[test]
fn typing_undo_history_stamp_rejects_same_depth_extension_and_revision_aba() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "b"], visual);
        let before = visual.update(|_, cx| super::typing_undo::HistoryStamp::capture(&workspace.read(cx).tabs[0]));
        type_chunks(handler.as_mut(), &["c"], visual);
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(!before.matches(&w.tabs[0])); });
        let before = visual.update(|_, cx| super::typing_undo::HistoryStamp::capture(&workspace.read(cx).tabs[0]));
        let revision = visual.update(|_, cx| workspace.read(cx).buffer().raw_revision.clone());
        visual.dispatch_action(EditUndo); visual.run_until_parked(); visual.dispatch_action(EditRedo); visual.run_until_parked();
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().raw_revision, revision); assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(!before.matches(&w.tabs[0])); });
    });
}

#[test]
fn typing_undo_copy_and_caret_round_trip_do_not_close_an_adjacent_run() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        visual.dispatch_action(crate::actions::EditCopy);
        visual.dispatch_action(crate::actions::MoveLeft); visual.dispatch_action(crate::actions::MoveRight);
        visual.run_until_parked();
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["b"], visual);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
    });
}

#[test]
fn typing_undo_nonadjacent_insertion_and_empty_explicit_group_start_new_entries() {
    for empty_group in [false, true] {
        with_source(|workspace, visual| {
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
            if empty_group {
                visual.update_window_entity(workspace, |w, _, cx| { w.begin_edit_group(); w.end_edit_group(cx); });
            } else { visual.dispatch_action(crate::actions::MoveLeft); }
            visual.run_until_parked();
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &["b"], visual);
            assert_eq!(text(workspace, visual), if empty_group { "BASEab" } else { "BASEba" });
            visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 2));
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        });
    }
}

#[test]
fn typing_undo_each_repainted_callback_flushes_and_retains_its_revision() {
    with_source(|workspace, visual| {
        let mut previous = visual.update(|_, cx| { let w = workspace.read(cx); (w.buffer().raw_revision.clone(),
            w.tabs[0].content_activity.clone(), w.autosave_generation, w.post_edit_generation) });
        for chunk in ["a", "é", "\n", "😀"] {
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &[chunk], visual);
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_ne!(w.buffer().raw_revision, previous.0);
                assert_ne!(w.tabs[0].content_activity, previous.1);
                assert_eq!(w.autosave_generation, previous.2 + 1); assert_eq!(w.post_edit_generation, previous.3 + 1);
                previous = (w.buffer().raw_revision.clone(), w.tabs[0].content_activity.clone(), w.autosave_generation, w.post_edit_generation);
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].edit_transaction.is_none());
            });
        }
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEaé\n😀");
    });
}

#[test]
fn typing_undo_registered_held_sort_rejects_same_depth_extension_aba_and_cancel() {
    for case in 0..3 {
        with_source(|workspace, visual| {
            visual.update_window_entity(workspace, |w, _, _| {
                w.tabs[0].buffer.rope = ropey::Rope::from_str("z\nBASE"); w.tabs[0].buffer.mark_saved();
                w.tabs[0].selection.collapse_to(6);
            });
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "b"], visual);
            let before = visual.update(|_, cx| super::typing_undo::HistoryStamp::capture(&workspace.read(cx).tabs[0]));
            let (send, receive) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
            visual.dispatch_action(crate::actions::EditSortLinesLexIgnoreCaseAsc); visual.run_until_parked();
            visual.update(|_, cx| assert!(workspace.read(cx).pending_prepared_edit.is_some()));
            match case {
                0 => type_chunks(handler.as_mut(), &["c"], visual),
                1 => { visual.dispatch_action(EditUndo); visual.run_until_parked(); visual.dispatch_action(EditRedo); visual.run_until_parked(); }
                _ => { visual.dispatch_action(crate::actions::EditFindClose); visual.run_until_parked(); }
            }
            let _ = send.send(()); visual.run_until_parked();
            visual.update(|_, cx| {
                let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), if case == 0 { "z\nBASEabc" } else { "z\nBASEab" });
                assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.tabs[0].redo_stack.is_empty());
                assert!(w.pending_prepared_edit.is_none()); assert_eq!(before.matches(&w.tabs[0]), case == 2);
            });
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("typing sentinel"));
            visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "z\nBASE");
        });
    }
}

#[test]
fn typing_undo_registered_held_bookmark_copy_rejects_same_depth_extension() {
    with_source(|workspace, visual| {
        let original = format!("BASE\n{}", "row\n".repeat(70000));
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].buffer.rope = ropey::Rope::from_str(&original); w.tabs[0].buffer.mark_saved();
            w.tabs[0].selection.collapse_to(original.len());
        });
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "b"], visual);
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, _, _| w.bookmark_copy_test_gate = Some(receive));
        visual.dispatch_action(crate::actions::CopyBookmarkedLines); visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).pending_bookmark_copy.is_some()));
        type_chunks(handler.as_mut(), &["c"], visual); let _ = send.send(()); visual.run_until_parked();
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.pending_bookmark_copy.is_none()); });
        assert_eq!(text(workspace, visual), format!("{original}abc"));
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("typing sentinel"));
    });
}

fn disk_bytes(w: &Workspace) -> Vec<u8> {
    let buffer = w.buffer();
    let raw = crate::text::line_ending::rope_to_bytes(&buffer.rope, buffer.line_ending);
    crate::text::encoding::encode_checked(std::str::from_utf8(&raw).expect("Rope UTF-8"), buffer.encoding).expect("representable fixture")
}

#[test]
fn typing_undo_disk_loaded_encodings_and_eols_restore_exact_bytes_and_savepoint() {
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for eol in [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr] {
            with_source(|workspace, visual| {
                let folder = tempfile::tempdir().unwrap(); let path = folder.path().join("typing-source.txt");
                let body = if encoding == Encoding::Latin1 { "café€\nfin" } else { "café€\n中😀" };
                let original = crate::text::encoding::encode_checked(&body.replace('\n', eol.as_str()), encoding).unwrap();
                std::fs::write(&path, &original).unwrap();
                visual.update_window_entity(workspace, |w, _, _| {
                    w.tabs[0].buffer = crate::io::read_buffer(&path).unwrap();
                    let end = w.tabs[0].buffer.rope.len_bytes(); w.tabs[0].selection.collapse_to(end);
                    w.tabs[0].buffer.bookmarks.insert(0); w.tabs[0].buffer.marks.push(0..2);
                });
                let saved = visual.update(|_, cx| workspace.read(cx).buffer().savepoint.clone());
                let mut handler = capture(visual); type_chunks(handler.as_mut(), &["é", "\n", "x"], visual);
                visual.update(|_, cx| {
                    let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1);
                    assert_eq!(w.buffer().encoding, encoding); assert_eq!(w.buffer().line_ending, eol);
                    assert!(w.buffer().savepoint == saved); assert!(w.buffer().dirty);
                });
                visual.dispatch_action(EditUndo); visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx); assert_eq!(disk_bytes(w), original); assert!(!w.buffer().dirty);
                    assert!(w.buffer().savepoint == saved); assert_eq!(w.buffer().marks, vec![0..2]);
                    assert_eq!(w.buffer().bookmarks.iter().copied().collect::<Vec<_>>(), vec![0]);
                });
                visual.dispatch_action(EditRedo); visual.run_until_parked();
                visual.update(|_, cx| {
                    let w = workspace.read(cx); let expected = crate::text::encoding::encode_checked(
                        &format!("{body}é\nx").replace('\n', eol.as_str()), encoding).unwrap();
                    assert_eq!(disk_bytes(w), expected); assert!(w.buffer().dirty);
                });
            });
        }
    }
}

// Existing f62 synchronous File Save caller, separate from File's future captured-Save jobs.
#[test]
fn typing_undo_registered_synchronous_save_is_a_clean_reachable_boundary() {
    with_source(|workspace, visual| {
        let folder = tempfile::tempdir().unwrap(); let path = folder.path().join("typing-save.txt");
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].buffer.file_path = Some(path.clone()));
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        visual.dispatch_action(crate::actions::FileSave); visual.run_until_parked();
        let saved = std::fs::read(&path).unwrap(); assert_eq!(saved, b"\xef\xbb\xbfBASEa");
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["b", "c"], visual);
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        visual.update(|_, cx| assert!(workspace.read(cx).buffer().dirty));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    });
}

#[test]
fn typing_undo_group_survives_legacy_codec_without_restoring_runtime_eligibility() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "é", "😀"], visual);
        visual.update_window_entity(workspace, |w, _, _| {
            let tab = &mut w.tabs[0]; let bytes = crate::buffer::serialize_undo_stack(&tab.undo_stack);
            let mut history = crate::buffer::deserialize_undo_stack(&bytes);
            assert_eq!(history.len(), 1); crate::buffer::rebind_undo_revisions(&mut history, &tab.buffer.raw_revision);
            tab.undo_stack = history; tab.typing_undo.seal(); tab.buffer.forget_savepoint();
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEaé😀");
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["x"], visual);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 2));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEaé😀");
    });
}

#[test]
fn typing_undo_required_text_storage_refusal_preserves_both_histories_and_input_state() {
    for remove in [false, true] {
        with_source(|workspace, visual| {
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "b"], visual);
            visual.write_to_clipboard(ClipboardItem::new_string("X".into()));
            visual.dispatch_action(EditPaste); visual.run_until_parked();
            visual.dispatch_action(EditUndo); visual.run_until_parked();
            visual.update_window_entity(workspace, |w, _, _| {
                if remove { w.tabs[0].selection.anchor = 6; w.tabs[0].selection.head = 4; w.sync_source_input_caret(); }
                w.find_state.error = Some("existing search error".into());
            });
            let mut handler = capture(visual);
            let before = visual.update(|window, cx| {
                let w = workspace.read(cx); let tab = &w.tabs[0];
                (super::typing_undo::HistoryStamp::capture(tab), crate::buffer::serialize_undo_stack(&tab.undo_stack),
                    crate::buffer::serialize_undo_stack(&tab.redo_stack), tab.buffer.savepoint.clone(), tab.selection,
                    w.autosave_generation, w.post_edit_generation, window.input_context_token(), w.source_input_token())
            });
            visual.update_window_entity(workspace, |w, _, _| {
                if remove { w.tabs[0].typing_undo.fail_remove_reserve = true; }
                else { w.tabs[0].typing_undo.fail_insert_reserve = true; }
            });
            let _recording = macros::override_execution_for_test(true);
            type_chunks(handler.as_mut(), &["x"], visual);
            assert!(!macros::stop_recording());
            visual.update(|window, cx| {
                let w = workspace.read(cx); let tab = &w.tabs[0];
                assert!(before.0.matches(tab)); assert_eq!(crate::buffer::serialize_undo_stack(&tab.undo_stack), before.1);
                assert_eq!(crate::buffer::serialize_undo_stack(&tab.redo_stack), before.2);
                assert!(tab.buffer.savepoint == before.3); assert_eq!((tab.selection.anchor, tab.selection.head), (before.4.anchor, before.4.head));
                assert_eq!((w.autosave_generation, w.post_edit_generation), (before.5, before.6));
                assert_eq!(window.input_context_token(), before.7); assert!(w.source_input_token() == before.8);
                assert_eq!(w.buffer().rope.to_string(), "BASEab"); assert!(w.buffer().dirty);
                assert_eq!(w.buffer().marks, vec![0..2]); assert_eq!(tab.begin_select_marker, Some(1));
                assert_eq!(w.find_state.error.as_deref(), Some("existing search error"));
                assert!(macros::recorded_events_for_test().unwrap().is_empty());
                assert!(w.status_message.as_deref().is_some_and(|s| s.contains(if remove { "removed typing Undo text" } else { "inserted typing Undo text" })));
            });
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("X"));
            visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEabX");
        });
    }
}

// Savepoint settlement model: a saved earlier endpoint does not break newer typing.
#[test]
fn typing_undo_settling_an_earlier_captured_save_preserves_the_later_run() {
    with_source(|workspace, visual| {
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a"], visual);
        let captured = visual.update_window_entity(workspace, |w, _, _| {
            let tab = &mut w.tabs[0]; let revision = tab.buffer.raw_revision.clone();
            assert!(tab.seal_typing_undo_revision(&revision));
            let mut saved_model = crate::buffer::Buffer::empty();
            saved_model.raw_revision = revision; saved_model.encoding = tab.buffer.encoding;
            saved_model.line_ending = tab.buffer.line_ending; saved_model.mark_saved(); saved_model.savepoint
        });
        type_chunks(handler.as_mut(), &["b", "c"], visual);
        visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].buffer.savepoint = captured; w.tabs[0].buffer.refresh_dirty(); });
        type_chunks(handler.as_mut(), &["d"], visual);
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 2); assert!(w.buffer().dirty); });
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEa");
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASEabcd");
    });
}

#[test]
fn typing_undo_raw_view_coalesces_without_changing_native_history_or_revision() {
    with_source(|workspace, visual| {
        let (raw, ast, native_revision) = visual.update_window_entity(workspace, |w, _, _| {
            let payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi seed}".to_vec()).unwrap();
            let raw = payload.raw_view_text.clone(); let ast = payload.ast.clone(); let revision = payload.revision.clone();
            w.tabs[0].buffer.rope = ropey::Rope::from_str(&raw); w.tabs[0].buffer.rich_doc = Some(payload);
            w.tabs[0].buffer.mark_saved(); w.tabs[0].selection.collapse_to(raw.len());
            (raw, ast, revision)
        });
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "b", "c"], visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx); let native = w.buffer().rich_doc.as_ref().unwrap();
            assert_eq!(w.tabs[0].undo_stack.len(), 1); assert_eq!(native.ast, ast); assert_eq!(native.revision, native_revision);
            assert!(native.edit_log.is_empty()); assert!(native.undo_stack.is_empty()); assert!(native.redo_stack.is_empty());
        });
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), raw);
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        visual.dispatch_action(EditRedo); visual.run_until_parked(); assert_eq!(text(workspace, visual), format!("{raw}abc"));
    });
}

#[test]
fn typing_undo_successful_callbacks_keep_existing_insert_text_record_and_playback() {
    with_source(|workspace, visual| {
        let events = {
            let _recording = macros::override_execution_for_test(true);
            let mut handler = capture(visual); type_chunks(handler.as_mut(), &["a", "é", "😀"], visual);
            visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
            let events = macros::recorded_events_for_test().unwrap();
            assert!(matches!(events.as_slice(), [macros::MacroEvent::InsertText(text)] if text == "aé😀")); events
        };
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
        let _playback = macros::override_playback_for_test(events, Default::default());
        visual.dispatch_action(crate::actions::MacroPlayback); visual.run_until_parked();
        assert_eq!(text(workspace, visual), "BASEaé😀");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
    });
}

#[test]
fn typing_undo_macro_payload_refusal_is_atomic_and_off_recording_skips_storage() {
    with_source(|workspace, visual| {
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].typing_undo.fail_macro_reserve = true);
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["x"], visual);
        visual.update(|_, cx| assert!(workspace.read(cx).tabs[0].typing_undo.fail_macro_reserve));
        visual.dispatch_action(crate::actions::MacroStartRecording); visual.run_until_parked();
        let before = visual.update(|window, cx| {
            let w = workspace.read(cx); (super::typing_undo::HistoryStamp::capture(&w.tabs[0]), w.selection(),
                w.autosave_generation, w.post_edit_generation, w.source_input_token(), window.input_context_token(), w.buffer().savepoint.clone())
        });
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["y"], visual);
        visual.update(|window, cx| {
            let w = workspace.read(cx); assert!(before.0.matches(&w.tabs[0]));
            assert_eq!(w.buffer().rope.to_string(), "BASEx"); assert!(w.buffer().savepoint == before.6);
            assert_eq!((w.selection().anchor, w.selection().head), (before.1.anchor, before.1.head));
            assert_eq!((w.autosave_generation, w.post_edit_generation), (before.2, before.3));
            assert!(w.source_input_token() == before.4); assert_eq!(window.input_context_token(), before.5);
            assert_eq!(w.buffer().marks, vec![0..2]); assert_eq!(w.tabs[0].begin_select_marker, Some(1));
            assert_eq!(w.status_message.as_deref(), Some("Cannot reserve recorded typing text; no changes applied"));
        });
        visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
        assert!(macros::recorded_events_for_test().unwrap().is_empty());
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("typing sentinel"));
        let mut handler = capture(visual); type_chunks(handler.as_mut(), &["y"], visual);
        assert_eq!(text(workspace, visual), "BASExy");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1));
        visual.dispatch_action(EditUndo); visual.run_until_parked(); assert_eq!(text(workspace, visual), "BASE");
    });
}
