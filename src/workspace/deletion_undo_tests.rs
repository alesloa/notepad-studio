// scintilla/src/UndoHistory.cxx:253-350; Editor.cxx:2415-2443,2496-2536.
// Catalog-bound installed key/action protocol, not native Windows input.
use crate::{actions::{EditRedo, EditUndo}, buffer::{ContentRevision, EditOp, UndoEntry},
    macros, settings::{AutoIndentMode, AutocompletionMode, Settings}, tab::Tab,
    text::{encoding::Encoding, line_ending::LineEnding}, workspace::Workspace};
use gpui::{ClipboardItem, Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

#[derive(Clone, Copy, Debug)]
enum Surface { Source(Encoding), Raw }

struct Outcome {
    label: String,
    original: String,
    undone: String,
    first_caret: usize,
    undo_caret: (usize, usize),
    original_revision: ContentRevision,
    undone_revision: ContentRevision,
    undo_dirty: bool,
    undo_bytes: Vec<u8>,
    original_bytes: Vec<u8>,
}

fn disk_bytes(w: &Workspace) -> Vec<u8> {
    let buffer = w.buffer();
    let bytes = crate::text::line_ending::rope_to_bytes(&buffer.rope, buffer.line_ending);
    crate::text::encoding::encode_checked(std::str::from_utf8(&bytes).unwrap(), buffer.encoding).unwrap()
}

fn last_op(w: &Workspace) -> &EditOp {
    match w.tabs[0].undo_stack.last().unwrap() {
        UndoEntry::Single(op) => op,
        UndoEntry::Group { ops, .. } => ops.last().unwrap(),
        UndoEntry::Prepared(_) => panic!("ordinary deletion unexpectedly prepared"),
    }
}

fn run(surface: Surface, backspace: bool, keys: usize) -> Outcome {
    let label = format!("{surface:?}/{}x{keys}", if backspace { "backspace" } else { "delete" });
    eprintln!("G1 BEGIN {label}");
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
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("deletion-source.txt");
    let encoding = match surface { Surface::Source(encoding) => encoding, Surface::Raw => Encoding::Utf8 };
    let source_bytes = crate::text::encoding::encode_checked("BASEabc\r\nEND", encoding).unwrap();
    std::fs::write(&path, &source_bytes).unwrap();
    let mut cx = TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (workspace, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        match surface {
            Surface::Source(_) => tab.buffer = crate::io::read_buffer(&path).unwrap(),
            Surface::Raw => {
                let payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi BASEabc}".to_vec()).unwrap();
                tab.buffer.rope = ropey::Rope::from_str(&payload.raw_view_text);
                tab.buffer.rich_doc = Some(payload);
            }
        }
        let text = tab.buffer.rope.to_string();
        let base = text.find("BASEabc").expect("neutral fixture target");
        tab.buffer.bookmarks.insert(0); tab.buffer.marks.push(0..2); tab.begin_select_marker = Some(1);
        tab.buffer.mark_saved(); tab.selection.collapse_to(base + if backspace { 7 } else { 4 });
        w.tabs = vec![tab]; w.active = 0; w.word_wrap = false;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink);
        w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked();
    visual.write_to_clipboard(ClipboardItem::new_string("deletion sentinel".into()));
    let handler = visual.update(|window, cx| {
        window.refresh(); let _ = window.draw(cx);
        let mut handler = window.take_registered_input_handler().expect("painted Source/Raw handler");
        assert!(handler.accepts_text_input(window, cx)); handler
    });
    drop(handler);
    visual.update(|window, cx| { window.refresh(); let _ = window.draw(cx); });
    let (original, original_revision, original_bytes, first_caret, saved, native) = visual.update(|window, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[0];
        assert!(w.input_focus(window) == crate::tab::InputFocus::Editor);
        assert!(w.focus_handle.is_focused(window)); assert!(window.is_window_active()); assert!(!w.window_closed);
        assert!(!w.is_composing_text(window)); assert!(tab.block_selection.is_none());
        assert!(tab.undo_stack.is_empty() && tab.redo_stack.is_empty() && tab.edit_transaction.is_none());
        assert!(!tab.buffer.dirty && !tab.buffer.read_only && tab.buffer.loading.is_none());
        if matches!(surface, Surface::Source(_)) {
            assert_eq!(disk_bytes(w), source_bytes); assert_eq!(tab.buffer.encoding, encoding);
            assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
        }
        (tab.buffer.rope.to_string(), tab.buffer.raw_revision.clone(), disk_bytes(w), tab.selection.head,
            tab.buffer.savepoint.clone(), tab.buffer.rich_doc.as_ref().map(|payload| (payload.ast.clone(), payload.revision.clone())))
    });
    eprintln!("G1 READY {label} first_caret={first_caret}");
    let mut previous = original_revision.clone();
    let mut previous_activity = visual.update(|_, cx| workspace.read(cx).tabs[0].content_activity.clone());
    for index in 0..keys {
        visual.simulate_keystrokes(if backspace { "backspace" } else { "delete" });
        visual.run_until_parked();
        visual.update(|window, cx| { window.refresh(); let _ = window.draw(cx); });
        visual.update(|_, cx| {
            let w = workspace.read(cx); let tab = &w.tabs[0]; let op = last_op(w);
            assert_eq!(op.before_revision, previous); assert_eq!(op.after_revision, tab.buffer.raw_revision);
            assert_ne!(op.before_revision, op.after_revision); assert!(op.inserted.is_empty());
            assert_eq!(op.removed, if backspace { ["c", "b", "a"][index] } else { ["a", "b", "c"][index] });
            assert_ne!(tab.content_activity, previous_activity);
            previous = tab.buffer.raw_revision.clone(); previous_activity = tab.content_activity.clone();
            eprintln!("G1 KEY {label} index={index} slots={}", tab.undo_stack.len());
        });
    }
    let expected = if keys == 3 { original.replacen("BASEabc", "BASE", 1) }
        else { original.replacen("BASEabc", if backspace { "BASEab" } else { "BASEbc" }, 1) };
    visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), expected); assert!(w.buffer().dirty); });
    visual.dispatch_action(EditUndo); visual.run_until_parked();
    let (undone, undone_revision, undo_dirty, undo_caret, undo_bytes) = visual.update(|_, cx| {
        let w = workspace.read(cx); (w.buffer().rope.to_string(), w.buffer().raw_revision.clone(), w.buffer().dirty,
            (w.selection().anchor, w.selection().head), disk_bytes(w))
    });
    visual.dispatch_action(EditRedo); visual.run_until_parked();
    visual.update(|window, cx| {
        let w = workspace.read(cx); let tab = &w.tabs[0];
        assert_eq!(w.buffer().rope.to_string(), expected); assert_eq!(w.buffer().raw_revision, previous);
        assert!(w.buffer().dirty); assert!(w.buffer().savepoint == saved);
        assert_eq!(tab.selection.head, first_caret - if backspace { keys } else { 0 });
        assert_eq!(tab.selection.anchor, tab.selection.head);
        assert_eq!(tab.buffer.marks, vec![0..2]); assert_eq!(tab.buffer.bookmarks.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(tab.begin_select_marker, Some(1)); assert!(tab.block_selection.is_none()); assert!(tab.edit_transaction.is_none());
        assert!(w.focus_handle.is_focused(window)); assert!(!w.is_composing_text(window));
        if let Some((ast, revision)) = native.as_ref() {
            let payload = tab.buffer.rich_doc.as_ref().unwrap();
            assert_eq!(&payload.ast, ast); assert_eq!(&payload.revision, revision);
            assert!(payload.edit_log.is_empty() && payload.undo_stack.is_empty() && payload.redo_stack.is_empty());
        }
    });
    assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("deletion sentinel"));
    eprintln!("G1 OBSERVED {label} undone={undone:?} undo_dirty={undo_dirty} undo_caret={undo_caret:?}");
    cx.quit();
    Outcome { label, original, undone, first_caret, undo_caret, original_revision, undone_revision,
        undo_dirty, undo_bytes, original_bytes }
}

fn assert_complete(outcomes: &[Outcome]) {
    // Collect both surfaces before checking the expected coalescence, so a Source
    // RED cannot hide an unexecuted Raw parameter.
    let mut failures = Vec::new();
    for value in outcomes {
        for (condition, contract) in [
            (value.undone == value.original, "one Undo restores the whole deletion run"),
            (value.undone_revision == value.original_revision, "captured revision"),
            (!value.undo_dirty, "captured savepoint is clean"),
            (value.undo_caret == (value.first_caret, value.first_caret), "first caret"),
            (value.undo_bytes == value.original_bytes, "exact encoded bytes"),
        ] {
            if !condition { failures.push(format!("{}: {contract}", value.label)); }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn deletion_undo_g1_catalog_backspace_coalesces_across_turns() {
    let outcomes = [run(Surface::Source(Encoding::Utf8Bom), true, 3), run(Surface::Raw, true, 3)];
    assert_complete(&outcomes);
}

#[test]
fn deletion_undo_g1_catalog_delete_coalesces_across_turns() {
    let outcomes = [run(Surface::Source(Encoding::Utf8Bom), false, 3), run(Surface::Raw, false, 3)];
    assert_complete(&outcomes);
}

#[test]
fn deletion_undo_g1_single_catalog_key_restores_exact_source_and_raw_state() {
    for backspace in [true, false] {
        for surface in [Surface::Source(Encoding::Utf8Bom), Surface::Source(Encoding::Utf16Le),
            Surface::Source(Encoding::Utf16Be), Surface::Raw] {
            assert_complete(&[run(surface, backspace, 1)]);
        }
    }
}

fn with_editor(body: &str, raw: bool, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext, usize)) {
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
        if raw {
            // A Raw draft can differ from the last successfully parsed native payload.
            tab.buffer.rich_doc = Some(crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi seed}".to_vec()).unwrap());
        }
        let text = if raw { format!("{{\\rtf1\\ansi {body}}}") } else { body.to_owned() };
        tab.buffer.rope = ropey::Rope::from_str(&text); tab.buffer.encoding = Encoding::Utf8Bom;
        tab.buffer.line_ending = LineEnding::Crlf; tab.buffer.bookmarks.insert(0); tab.buffer.marks.push(0..2);
        tab.begin_select_marker = Some(1); tab.buffer.mark_saved();
        tab.selection.collapse_to(if raw { text.len() - 1 } else { text.len() });
        w.tabs = vec![tab]; w.active = 0; w.word_wrap = false;
        let (sink, _) = std::sync::mpsc::channel(); w.session_snapshot_sink = Some(sink); w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked(); paint(visual);
    visual.write_to_clipboard(ClipboardItem::new_string("boundary sentinel".into()));
    let (base, native) = visual.update(|_, cx| {
        let w = workspace.read(cx); (w.buffer().rope.to_string().find("BASE").unwrap(),
            w.buffer().rich_doc.as_ref().map(|p| (p.ast.clone(), p.revision.clone())))
    });
    test(&workspace, visual, base);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        if let Some((ast, revision)) = native {
            let p = w.buffer().rich_doc.as_ref().unwrap(); assert_eq!(p.ast, ast); assert_eq!(p.revision, revision);
            assert!(p.edit_log.is_empty() && p.undo_stack.is_empty() && p.redo_stack.is_empty());
        }
    });
    cx.quit();
}

fn paint(visual: &mut VisualTestContext) { visual.update(|window, cx| { window.refresh(); let _ = window.draw(cx); }); }
fn press(visual: &mut VisualTestContext, key: &str) {
    let action = match key { "ctrl-z" => Some("EditUndo"), "ctrl-y" => Some("EditRedo"),
        "ctrl-delete" => Some("EditDeleteWordRight"), _ => None };
    let key = action.map(|id| {
        let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == id).expect("catalog action");
        *crate::shortcuts::platform_default_keys(entry).first().expect("default chord")
    }).unwrap_or(key);
    visual.simulate_keystrokes(key); visual.run_until_parked(); paint(visual);
}
fn text(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> String {
    visual.update(|_, cx| workspace.read(cx).buffer().rope.to_string())
}
fn caret(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, byte: usize) {
    visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].selection.collapse_to(byte); w.sync_source_input_caret(); }); paint(visual);
}
fn type_text(visual: &mut VisualTestContext, value: &str) {
    paint(visual);
    visual.update(|window, cx| {
        let mut handler = window.take_registered_input_handler().unwrap();
        assert!(handler.accepts_text_input(window, cx)); handler.replace_text_in_range(None, value, window, cx);
    });
    visual.run_until_parked(); paint(visual);
}
fn slots(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> usize {
    visual.update(|_, cx| workspace.read(cx).tabs[0].undo_stack.len())
}
fn replace_body(original: &str, body: &str) -> String { original.replacen("BASEa中b😀c", body, 1) }

#[test]
fn deletion_undo_unicode_new_width_and_unrestricted_previous_width_use_editing_bytes() {
    for raw in [false, true] {
        for backspace in [false, true] {
            with_editor("BASEa中b😀c", raw, |workspace, visual, base| {
                let original = text(workspace, visual); let bytes = visual.update(|_, cx| disk_bytes(workspace.read(cx)));
                if !backspace { caret(workspace, visual, base + 4); }
                for _ in 0..5 { press(visual, if backspace { "backspace" } else { "delete" }); }
                assert_eq!(slots(workspace, visual), 3);
                assert_eq!(text(workspace, visual), replace_body(&original, "BASE"));
                let expected = if backspace { ["BASEa中", "BASEa中b😀", "BASEa中b😀c"] }
                    else { ["BASE😀c", "BASE中b😀c", "BASEa中b😀c"] };
                for value in expected { press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), replace_body(&original, value)); }
                visual.update(|_, cx| { assert_eq!(disk_bytes(workspace.read(cx)), bytes); assert!(!workspace.read(cx).buffer().dirty); });
                for _ in 0..3 { press(visual, "ctrl-y"); }
                assert_eq!(text(workspace, visual), replace_body(&original, "BASE"));
            });
            with_editor("BASEéé", raw, |workspace, visual, base| {
                let original = text(workspace, visual); if !backspace { caret(workspace, visual, base + 4); }
                for _ in 0..2 { press(visual, if backspace { "backspace" } else { "delete" }); }
                assert_eq!(slots(workspace, visual), 1); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
            });
        }
    }
}

#[test]
fn deletion_undo_utf16_disk_widths_and_crlf_transport_round_trip() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        for backspace in [false, true] {
            with_editor("BASE", false, |workspace, visual, _| {
                let folder = tempfile::tempdir().unwrap(); let path = folder.path().join("unicode-delete.txt");
                let original = crate::text::encoding::encode_checked("BASEa中b😀c\r\nEND", encoding).unwrap();
                std::fs::write(&path, &original).unwrap();
                visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].buffer = crate::io::read_buffer(&path).unwrap(); });
                caret(workspace, visual, if backspace { "BASEa中b😀c".len() } else { 4 });
                for _ in 0..5 { press(visual, if backspace { "backspace" } else { "delete" }); }
                assert_eq!(slots(workspace, visual), 3);
                for _ in 0..3 { press(visual, "ctrl-z"); }
                visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(disk_bytes(w), original); assert!(!w.buffer().dirty);
                    assert_eq!(w.buffer().encoding, encoding); assert_eq!(w.buffer().line_ending, LineEnding::Crlf); });
                for _ in 0..3 { press(visual, "ctrl-y"); }
                assert_eq!(text(workspace, visual), "BASE\nEND");
            });
        }
    }
}

#[test]
fn deletion_undo_crlf_pair_lone_endings_and_direction_switches_join() {
    for raw in [false, true] {
        for body in ["BASE\r\nab", "BASE\nab", "BASE\rab"] {
            for backspace in [false, true] {
                with_editor(body, raw, |workspace, visual, base| {
                    let original = text(workspace, visual); if !backspace { caret(workspace, visual, base + 4); }
                    for _ in 0..3 { press(visual, if backspace { "backspace" } else { "delete" }); }
                    assert_eq!(slots(workspace, visual), 1); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
                });
            }
        }
        for keys in [["delete", "backspace"], ["backspace", "delete"]] {
            with_editor("BASEabc", raw, |workspace, visual, base| {
                let original = text(workspace, visual); caret(workspace, visual, base + 5);
                for key in keys { press(visual, key); }
                assert_eq!(slots(workspace, visual), 1); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
            });
        }
    }
}

#[test]
fn deletion_undo_movement_copy_redo_and_explicit_boundaries_preserve_runs() {
    for raw in [false, true] {
        for separate in [false, true] {
            with_editor("BASEabc", raw, |workspace, visual, base| {
                let original = text(workspace, visual); caret(workspace, visual, base + 4); press(visual, "delete");
                let first = text(workspace, visual);
                if separate { visual.update_window_entity(workspace, |w, _, cx| { w.begin_edit_group(); w.end_edit_group(cx); }); }
                else { press(visual, "left"); press(visual, "right"); visual.dispatch_action(crate::actions::EditCopy); visual.run_until_parked(); }
                press(visual, "delete"); assert_eq!(slots(workspace, visual), if separate { 2 } else { 1 });
                press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), if separate { first } else { original.clone() });
                if separate { press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original); }
            });
        }
        with_editor("BASEabc", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4); press(visual, "delete");
            press(visual, "ctrl-z"); press(visual, "ctrl-y"); press(visual, "delete");
            assert_eq!(slots(workspace, visual), 1); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
        });
        with_editor("BASEabc", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4); press(visual, "delete");
            let first = text(workspace, visual); for _ in 0..3 { press(visual, "left"); } press(visual, "delete");
            assert_eq!(slots(workspace, visual), 2); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), first);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
        });
    }
}

#[test]
fn deletion_undo_insertion_selection_and_word_helpers_remain_separate() {
    for raw in [false, true] {
        with_editor("BASE", raw, |workspace, visual, _| {
            let original = text(workspace, visual); type_text(visual, "ab"); press(visual, "backspace"); press(visual, "backspace");
            assert_eq!(slots(workspace, visual), 2); press(visual, "ctrl-z"); assert!(text(workspace, visual).contains("BASEab"));
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
        });
        with_editor("BASEabc", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4); press(visual, "delete");
            let first = text(workspace, visual);
            visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].selection.anchor = base + 4; w.tabs[0].selection.head = base + 5; });
            paint(visual); press(visual, "delete"); press(visual, "delete"); assert_eq!(slots(workspace, visual), 3);
            for _ in 0..2 { press(visual, "ctrl-z"); } assert_eq!(text(workspace, visual), first);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
        });
        with_editor("BASEone two three", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4); press(visual, "ctrl-delete");
            let first = text(workspace, visual); press(visual, "ctrl-delete"); assert_eq!(slots(workspace, visual), 2);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), first); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
        });
    }
}

#[test]
fn deletion_undo_current_save_and_retained_captured_save_fences_are_exact() {
    for success in [false, true] {
        with_editor("BASEabcd", false, |workspace, visual, base| {
            caret(workspace, visual, base + 4); press(visual, "delete");
            let first = text(workspace, visual);
            let saved = visual.update_window_entity(workspace, |w, _, _| {
                let tab = &mut w.tabs[0]; let revision = tab.buffer.raw_revision.clone();
                let stamp = super::typing_undo::HistoryStamp::capture(tab);
                assert!(!tab.seal_typing_undo_revision(&Default::default())); assert!(stamp.matches(tab));
                assert!(tab.seal_typing_undo_revision(&revision)); assert!(stamp.matches(tab));
                let mut marker = crate::buffer::Buffer::empty(); marker.raw_revision = revision;
                marker.encoding = tab.buffer.encoding; marker.line_ending = tab.buffer.line_ending; marker.mark_saved(); marker.savepoint
            });
            press(visual, "delete"); press(visual, "delete");
            if success { visual.update_window_entity(workspace, |w, _, _| { w.tabs[0].buffer.savepoint = saved; w.tabs[0].buffer.refresh_dirty(); }); }
            press(visual, "delete"); assert_eq!(slots(workspace, visual), 2);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), first);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().dirty, !success));
            press(visual, "ctrl-y"); assert_eq!(text(workspace, visual), "BASE");
        });
    }
    with_editor("BASEabc", false, |workspace, visual, base| {
        let folder = tempfile::tempdir().unwrap(); let path = folder.path().join("saved-delete.txt");
        std::fs::write(&path, b"BASEabc").unwrap();
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].buffer.file_path = Some(path.clone()));
        caret(workspace, visual, base + 4); press(visual, "delete");
        visual.dispatch_action(crate::actions::FileSave); visual.run_until_parked();
        visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        let saved = std::fs::read(&path).unwrap(); press(visual, "delete"); assert_eq!(slots(workspace, visual), 2);
        press(visual, "ctrl-z"); visual.update(|_, cx| { assert!(!workspace.read(cx).buffer().dirty); assert_eq!(disk_bytes(workspace.read(cx)), saved); });
    });
}

#[test]
fn deletion_undo_optional_group_failure_falls_back_and_required_refusal_is_atomic() {
    with_editor("BASEabc", false, |workspace, visual, base| {
        caret(workspace, visual, base + 4); press(visual, "delete");
        visual.update_window_entity(workspace, |w, _, _| w.tabs[0].typing_undo.fail_group_reserve = true);
        press(visual, "delete"); assert_eq!(slots(workspace, visual), 2);
        press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), "BASEbc"); press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), "BASEabc");
    });
    for remove_text in [false, true] {
        with_editor("BASEabcd", false, |workspace, visual, base| {
            caret(workspace, visual, base + 4); press(visual, "delete"); press(visual, "delete");
            visual.update_window_entity(workspace, |w, _, cx| { w.begin_edit_group(); w.end_edit_group(cx); });
            press(visual, "delete"); press(visual, "ctrl-z");
            visual.dispatch_action(crate::actions::MacroStartRecording); visual.run_until_parked();
            let before = visual.update(|window, cx| {
                let w = workspace.read(cx); let tab = &w.tabs[0];
                (super::typing_undo::HistoryStamp::capture(tab), crate::buffer::serialize_undo_stack(&tab.undo_stack),
                    crate::buffer::serialize_undo_stack(&tab.redo_stack), disk_bytes(w), tab.selection,
                    tab.buffer.savepoint.clone(), w.source_input_token(), window.input_context_token(), w.autosave_generation, w.post_edit_generation)
            });
            visual.update_window_entity(workspace, |w, _, _| {
                if remove_text { w.tabs[0].typing_undo.fail_remove_reserve = true; }
                else { w.tabs[0].typing_undo.fail_history_reserve = true; }
            });
            press(visual, "delete");
            visual.update(|window, cx| {
                let w = workspace.read(cx); let tab = &w.tabs[0]; assert!(before.0.matches(tab));
                assert_eq!(crate::buffer::serialize_undo_stack(&tab.undo_stack), before.1);
                assert_eq!(crate::buffer::serialize_undo_stack(&tab.redo_stack), before.2); assert_eq!(disk_bytes(w), before.3);
                assert_eq!((tab.selection.anchor, tab.selection.head), (before.4.anchor, before.4.head)); assert!(tab.buffer.savepoint == before.5);
                assert!(w.source_input_token() == before.6); assert_eq!(window.input_context_token(), before.7);
                assert_eq!((w.autosave_generation, w.post_edit_generation), (before.8, before.9));
                assert_eq!(tab.buffer.marks, vec![0..2]); assert_eq!(tab.buffer.bookmarks.iter().copied().collect::<Vec<_>>(), vec![0]);
                assert!(w.status_message.as_deref().is_some_and(|s| s.contains("Cannot reserve")));
            });
            visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
            assert!(macros::recorded_events_for_test().unwrap().is_empty());
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("boundary sentinel"));
            press(visual, "ctrl-y"); assert_eq!(text(workspace, visual), "BASEd");
        });
    }
}

fn saved_events(events: Vec<macros::MacroEvent>) -> Vec<macros::MacroEvent> {
    let folder = tempfile::tempdir().unwrap(); let path = folder.path().join("deletion-macros.toml");
    let mut saved = std::collections::BTreeMap::new(); saved.insert("deletion".to_owned(), events);
    macros::saved_round_trip_for_test(&path, &saved).unwrap().remove("deletion").unwrap()
}

#[test]
fn deletion_undo_required_text_refusal_keeps_the_eligible_run() {
    for raw in [false, true] {
        with_editor("BASEabc", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4); press(visual, "delete");
            let before = visual.update(|_, cx| super::typing_undo::HistoryStamp::capture(&workspace.read(cx).tabs[0]));
            visual.update_window_entity(workspace, |w, _, _| w.tabs[0].typing_undo.fail_remove_reserve = true);
            press(visual, "delete");
            visual.update(|_, cx| assert!(before.matches(&workspace.read(cx).tabs[0])));
            assert_eq!(text(workspace, visual), original.replacen("BASEabc", "BASEbc", 1));
            press(visual, "delete"); assert_eq!(slots(workspace, visual), 1);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
            press(visual, "ctrl-y"); assert_eq!(text(workspace, visual), original.replacen("BASEabc", "BASEc", 1));
        });
    }
}

#[test]
fn deletion_undo_groups_keep_every_operation_through_the_legacy_codec() {
    for raw in [false, true] {
        with_editor("BASEaéb😀cd", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4);
            for _ in 0..5 { press(visual, "delete"); }
            assert_eq!(text(workspace, visual), original.replacen("BASEaéb😀cd", "BASEd", 1));
            visual.update_window_entity(workspace, |w, _, _| {
                let tab = &mut w.tabs[0]; assert_eq!(tab.undo_stack.len(), 2);
                let ops: Vec<_> = tab.undo_stack.iter().flat_map(|entry| match entry {
                    UndoEntry::Group { ops, .. } => ops.iter(), _ => panic!("removal Group"),
                }).collect();
                assert_eq!(ops.iter().map(|op| op.removed.as_str()).collect::<Vec<_>>(), ["a", "é", "b", "😀", "c"]);
                assert!(ops.iter().all(|op| op.inserted.is_empty() && op.byte_start == base + 4));
                for pair in ops.windows(2) { assert_eq!(pair[0].after_revision, pair[1].before_revision); }
                let bytes = crate::buffer::serialize_undo_stack(&tab.undo_stack);
                let mut history = crate::buffer::deserialize_undo_stack(&bytes); assert_eq!(history.len(), 2);
                crate::buffer::rebind_undo_revisions(&mut history, &tab.buffer.raw_revision);
                tab.undo_stack = history; tab.typing_undo.seal(); tab.buffer.forget_savepoint();
            });
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original.replacen("BASEaéb😀cd", "BASE😀cd", 1));
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
            press(visual, "ctrl-y"); press(visual, "ctrl-y"); press(visual, "delete");
            assert_eq!(slots(workspace, visual), 3); press(visual, "ctrl-z");
            assert_eq!(text(workspace, visual), original.replacen("BASEaéb😀cd", "BASEd", 1));
        });
    }
}

#[test]
fn deletion_undo_saved_macro_stops_before_tail_on_essential_refusal() {
    for raw in [false, true] {
        with_editor("BASEabc", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4);
            let events = saved_events(vec![macros::MacroEvent::Delete, macros::MacroEvent::InsertText("tail".into())]);
            let _playback = macros::override_playback_for_test(events, Default::default());
            let before = visual.update(|window, cx| {
                let w = workspace.read(cx); (super::typing_undo::HistoryStamp::capture(&w.tabs[0]),
                    disk_bytes(w), w.selection(), w.source_input_token(), window.input_context_token(),
                    w.autosave_generation, w.post_edit_generation)
            });
            visual.update_window_entity(workspace, |w, _, _| w.tabs[0].typing_undo.fail_history_reserve = true);
            visual.dispatch_action(crate::actions::MacroPlayback); visual.run_until_parked();
            assert_eq!(text(workspace, visual), original);
            visual.update(|window, cx| {
                let w = workspace.read(cx); assert!(before.0.matches(&w.tabs[0])); assert_eq!(disk_bytes(w), before.1);
                assert_eq!((w.selection().anchor, w.selection().head), (before.2.anchor, before.2.head));
                assert!(w.source_input_token() == before.3); assert_eq!(window.input_context_token(), before.4);
                assert_eq!((w.autosave_generation, w.post_edit_generation), (before.5, before.6));
                assert!(w.tabs[0].edit_transaction.is_none());
                assert!(w.status_message.as_deref().is_some_and(|s| s.contains("Cannot reserve typing Undo history")));
            });
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("boundary sentinel"));
        });
    }
}

#[test]
fn deletion_undo_ime_commit_stays_separate_from_the_next_removal_run() {
    for raw in [false, true] {
        with_editor("BASEabc", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4);
            visual.update(|window, cx| {
                let mut handler = window.take_registered_input_handler().unwrap();
                assert!(handler.accepts_text_input(window, cx));
                handler.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                handler.replace_text_in_range(None, "日本", window, cx);
            });
            visual.run_until_parked(); paint(visual);
            let committed = text(workspace, visual); assert_eq!(committed, original.replacen("BASEabc", "BASE日本abc", 1));
            press(visual, "delete"); press(visual, "delete"); assert_eq!(slots(workspace, visual), 2);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), committed);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
            press(visual, "ctrl-y"); press(visual, "ctrl-y");
            assert_eq!(text(workspace, visual), original.replacen("BASEabc", "BASE日本c", 1));
        });
    }
}

#[test]
fn deletion_undo_valid_noops_record_and_saved_commands_execute_in_fresh_editor() {
    for case in 0..3 {
        let mut events = Vec::new();
        with_editor("BASE", false, |workspace, visual, _| {
            if case != 1 { caret(workspace, visual, 0); }
            visual.dispatch_action(crate::actions::MacroStartRecording); visual.run_until_parked();
            if case == 2 { visual.update_window_entity(workspace, |w, _, _| w.tabs[0].buffer.read_only = true); paint(visual); }
            press(visual, if case == 0 { "backspace" } else { "delete" });
            visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
            events = macros::recorded_events_for_test().unwrap();
            assert!(matches!(events.as_slice(), [macros::MacroEvent::Backspace]) && case == 0
                || matches!(events.as_slice(), [macros::MacroEvent::Delete]) && case != 0);
            assert_eq!(text(workspace, visual), "BASE"); assert_eq!(slots(workspace, visual), 0);
        });
        let events = saved_events(events);
        with_editor("BASE", false, |workspace, visual, _| {
            if case != 0 { caret(workspace, visual, 0); }
            let _playback = macros::override_playback_for_test(events, Default::default());
            visual.dispatch_action(crate::actions::MacroPlayback); visual.run_until_parked();
            assert_eq!(text(workspace, visual), if case == 0 { "BAS" } else { "ASE" });
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), "BASE");
        });
    }
}

#[test]
fn deletion_undo_saved_unicode_macro_replays_real_handlers_as_one_explicit_group() {
    for raw in [false, true] {
        let mut events = Vec::new();
        with_editor("BASEa中b😀c", raw, |workspace, visual, base| {
            caret(workspace, visual, base + 4); visual.dispatch_action(crate::actions::MacroStartRecording); visual.run_until_parked();
            for _ in 0..5 { press(visual, "delete"); }
            visual.dispatch_action(crate::actions::MacroStopRecording); visual.run_until_parked();
            events = macros::recorded_events_for_test().unwrap(); assert_eq!(events.len(), 5); assert_eq!(slots(workspace, visual), 3);
        });
        let events = saved_events(events);
        with_editor("BASEa中b😀c", raw, |workspace, visual, base| {
            let original = text(workspace, visual); caret(workspace, visual, base + 4);
            let _playback = macros::override_playback_for_test(events, Default::default());
            visual.dispatch_action(crate::actions::MacroPlayback); visual.run_until_parked();
            assert_eq!(text(workspace, visual), replace_body(&original, "BASE")); assert_eq!(slots(workspace, visual), 1);
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), original);
        });
    }
}

#[test]
fn deletion_undo_held_sort_rejects_same_depth_extension_aba_and_cancel() {
    for case in 0..3 {
        with_editor("z\nBASEabc", false, |workspace, visual, base| {
            caret(workspace, visual, base + 4); press(visual, "delete");
            let before = visual.update(|_, cx| super::typing_undo::HistoryStamp::capture(&workspace.read(cx).tabs[0]));
            let (send, receive) = futures::channel::oneshot::channel();
            visual.update_window_entity(workspace, |w, _, _| w.prepared_test_gate = Some(receive));
            visual.dispatch_action(crate::actions::EditSortLinesLexIgnoreCaseAsc); visual.run_until_parked();
            visual.update(|_, cx| assert!(workspace.read(cx).pending_prepared_edit.is_some()));
            match case { 0 => press(visual, "delete"), 1 => { press(visual, "ctrl-z"); press(visual, "ctrl-y"); },
                _ => { visual.dispatch_action(crate::actions::EditFindClose); visual.run_until_parked(); } }
            let _ = send.send(()); visual.run_until_parked();
            visual.update(|_, cx| { let w = workspace.read(cx); assert!(w.pending_prepared_edit.is_none()); assert_eq!(w.tabs[0].undo_stack.len(), 1);
                assert_eq!(before.matches(&w.tabs[0]), case == 2); });
            assert_eq!(text(workspace, visual), if case == 0 { "z\nBASEc" } else { "z\nBASEbc" });
            assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("boundary sentinel"));
            press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), "z\nBASEabc");
        });
    }
}

#[test]
fn deletion_undo_held_bookmark_copy_rejects_same_depth_removal_extension() {
    let body = format!("BASEabc\n{}", "row\n".repeat(70000));
    with_editor(&body, false, |workspace, visual, base| {
        caret(workspace, visual, base + 4); press(visual, "delete");
        let (send, receive) = futures::channel::oneshot::channel();
        visual.update_window_entity(workspace, |w, _, _| w.bookmark_copy_test_gate = Some(receive));
        visual.dispatch_action(crate::actions::CopyBookmarkedLines); visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).pending_bookmark_copy.is_some()));
        press(visual, "delete"); let _ = send.send(()); visual.run_until_parked();
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.pending_bookmark_copy.is_none()); });
        assert_eq!(visual.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("boundary sentinel"));
        press(visual, "ctrl-z"); assert_eq!(text(workspace, visual), body);
    });
}
