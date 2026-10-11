use super::*;
use crate::{actions::*, buffer::Buffer, settings::Settings};
use gpui::{Entity, TestAppContext, VisualTestContext};
type WorkerGatePair = (Option<futures::channel::oneshot::Receiver<()>>, Option<futures::channel::oneshot::Receiver<()>>);

#[test]
fn actual_copy_picker_suggests_current_name_and_cancel_preserves_original() {
    for label in ["new 1", "named café中😀.txt"] {
        let mut buffer = Buffer::untitled(); buffer.new_label = Some(label.to_owned().into());
        with_editor(buffer, |entity, visual| {
            visual.simulate_input("unsaved");
            let requests = crate::io::SaveDialogRequestCaptureForTest::new();
            let reply = begin(visual, false);
            assert_eq!(requests.requests(), vec![("Save a Copy As", label.to_owned())]);
            assert!(reply.send(None).is_ok()); visual.run_until_parked();
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; assert!(tab.buffer.file_path.is_none()); assert!(tab.buffer.dirty);
                assert_eq!(tab.buffer.new_label.as_ref().map(|label| label.as_ref()), Some(label)); });
        });
    }
}

thread_local! {
    static OPEN_COPY: std::cell::RefCell<Option<Option<bool>>> = const { std::cell::RefCell::new(None) };
    static WORKER_GATES: std::cell::RefCell<WorkerGatePair> = const { std::cell::RefCell::new((None, None)) };
}
pub(super) fn take_worker_gates() -> WorkerGatePair {
    WORKER_GATES.with(|slot| std::mem::take(&mut *slot.borrow_mut()))
}
struct WorkerGates(WorkerGatePair);
impl WorkerGates {
    fn new(prepared: Option<futures::channel::oneshot::Receiver<()>>, commit: Option<futures::channel::oneshot::Receiver<()>>) -> Self {
        Self(WORKER_GATES.with(|slot| slot.replace((prepared, commit))))
    }
}
impl Drop for WorkerGates { fn drop(&mut self) { WORKER_GATES.with(|slot| slot.replace(std::mem::take(&mut self.0))); } }
pub(super) fn open_copy_answer() -> Option<Option<bool>> { OPEN_COPY.with(|answer| *answer.borrow()) }
pub(crate) struct Options(Option<Option<bool>>);
impl Options { pub(crate) fn new(answer: Option<bool>) -> Self { Self(OPEN_COPY.with(|slot| slot.replace(Some(answer)))) } }
impl Drop for Options { fn drop(&mut self) { OPEN_COPY.with(|slot| slot.replace(self.0)); } }

fn with_editor(buffer: Buffer, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    with_editor_view(buffer, None, test);
}
fn with_editor_view(buffer: Buffer, view: Option<crate::tab::TabViewMode>, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true, folder_panel_visible: false, tab_exit_on_last_close: false,
        default_directory_mode: crate::settings::DirectoryMode::FollowDocument, ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::with_buffer(buffer);
        if let Some(view) = view { tab.view_mode = view; }
        tab.buffer.language = None;
        w.tabs = vec![tab, Tab::empty()];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w.touch_mru();
        w
    });
    visual.update(|window, cx| {
        let closing = entity.clone();
        window.on_window_should_close(cx, move |_, cx| closing.update(cx, crate::app::quit::save_before_window_close));
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&entity, visual);
    app.quit();
}

fn begin(visual: &mut VisualTestContext, open: bool) -> futures::channel::oneshot::Sender<Option<PathBuf>> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let _options = Options::new(Some(open));
    let _dialog = crate::io::SaveAsDialogOverrideForTest::new(receiver);
    visual.dispatch_action(FileSaveCopyAs);
    visual.run_until_parked();
    sender
}

#[test]
fn actual_copy_keeps_original_history_dirty_path_recovery_and_reordered_active_tab() {
    let dir = crate::persist::TempDir::new("copy-owned");
    let source = dir.join("source.txt");
    let target = dir.join("copy café中😀.txt");
    let bytes = crate::text::encoding::encode_checked("disk café中😀\r\n", Encoding::Utf16Le).expect("fixture");
    std::fs::write(&source, &bytes).expect("source");
    with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
        visual.simulate_input("unsaved ");
        let before = visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0];
            (tab.session_id, tab.instance.clone(), tab.undo_stack.len(), tab.buffer.savepoint.clone(), tab.buffer.rope.to_string())
        });
        let generation = visual.update(|_, cx| entity.read(cx).autosave_generation);
        let sender = begin(visual, false);
        let sibling = visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs.swap(0, 1); w.active = 0; w.touch_mru(); cx.notify(); w.tabs[0].session_id
        }));
        assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); let tab = &w.tabs[1];
            assert_eq!(tab.session_id, before.0); assert!(Arc::ptr_eq(&tab.instance, &before.1));
            assert_eq!(tab.undo_stack.len(), before.2); assert!(tab.buffer.savepoint == before.3);
            assert_eq!(tab.buffer.rope.to_string(), before.4); assert!(tab.buffer.dirty);
            assert_eq!(tab.buffer.file_path.as_ref(), Some(&source));
            assert_eq!(w.tabs[w.active].session_id, sibling); assert_eq!(w.autosave_generation, generation);
        });
        assert_eq!(std::fs::read(&source).expect("source bytes"), bytes);
        assert_eq!(std::fs::read(&target).expect("copy bytes"), crate::text::encoding::encode_checked("unsaved disk café中😀\r\n", Encoding::Utf16Le).expect("expected"));
        assert!(crate::recent::snapshot().is_empty());
        visual.update(|_, cx| entity.update(cx, |w, cx| w.set_active_tab(1, cx)));
        for _ in 0..before.2 { visual.dispatch_action(EditUndo); }
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[1].buffer.rope.to_string(), "disk café中😀\n"));
    });
}

#[test]
fn actual_copy_cancel_and_write_errors_leave_original_state() {
    for mode in ["options", "picker", "write", "encoding"] {
        let dir = crate::persist::TempDir::new("copy-errors");
        let source = dir.join("source.txt"); std::fs::write(&source, b"disk\r\n").expect("fixture");
        let target = dir.join("existing.txt"); std::fs::write(&target, b"must survive").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            visual.simulate_input("中😀 ");
            if mode == "encoding" { visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.encoding = Encoding::Latin1)); }
            let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                (tab.session_id, tab.buffer.rope.to_string(), tab.undo_stack.len(), tab.buffer.file_path.clone(), tab.buffer.dirty) });
            if mode == "options" {
                let _options = Options::new(None);
                visual.dispatch_action(FileSaveCopyAs); visual.run_until_parked();
            } else {
                let sender = begin(visual, false);
                let chosen = match mode { "picker" => None, "write" => Some(dir.path().to_path_buf()), _ => Some(target.clone()) };
                assert!(sender.send(chosen).is_ok()); visual.run_until_parked();
            }
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                assert_eq!(before, (tab.session_id, tab.buffer.rope.to_string(), tab.undo_stack.len(), tab.buffer.file_path.clone(), tab.buffer.dirty)); });
            assert_eq!(std::fs::read(&source).expect("original"), b"disk\r\n");
            assert_eq!(std::fs::read(&target).expect("destination"), b"must survive");
            assert!(crate::recent::snapshot().is_empty());
        });
    }
}

#[test]
fn actual_copy_rejects_edit_undo_path_encoding_eol_readonly_save_and_window_changes() {
    for change in ["edit", "undo", "path", "encoding", "eol", "readonly", "save", "window", "close"] {
        let dir = crate::persist::TempDir::new("copy-stale");
        let source = dir.join("source.txt"); std::fs::write(&source, b"disk\r\n").expect("fixture");
        let target = dir.join("existing.txt"); std::fs::write(&target, b"must survive").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            if change == "save" { visual.simulate_input("pending "); }
            let sender = begin(visual, false);
            match change {
                "edit" | "undo" => { visual.simulate_input("late "); if change == "undo" { visual.dispatch_action(EditUndo); } },
                "close" => { assert!(visual.simulate_close()); },
                "save" => visual.dispatch_action(FileSave),
                _ => visual.update(|_, cx| entity.update(cx, |w, _| match change {
                    "path" => w.tabs[0].buffer.file_path = Some(dir.join("different.txt")),
                    "encoding" => w.tabs[0].buffer.encoding = Encoding::Utf8Bom,
                    "eol" => w.tabs[0].buffer.line_ending = LineEnding::Cr,
                    "readonly" => w.tabs[0].buffer.read_only = true,
                    "window" => w.session_window_id = uuid::Uuid::new_v4(),
                    _ => panic!("case"),
                })),
            }
            visual.run_until_parked();
            assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            assert_eq!(std::fs::read(&target).expect("destination"), b"must survive", "{change}");
            assert!(crate::recent::snapshot().is_empty());
        });
    }
}

#[test]
fn actual_copy_rejects_restored_uuid_and_superseded_requests() {
    for restore in [true, false] {
        let dir = crate::persist::TempDir::new("copy-incarnation");
        let source = dir.join("source.txt"); std::fs::write(&source, b"disk\r\n").expect("fixture");
        let old_target = dir.join("old.txt"); let new_target = dir.join("new.txt");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let old_id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
            let first = begin(visual, false);
            if restore {
                visual.dispatch_action(FileCloseTab); visual.run_until_parked();
                visual.dispatch_action(FileRestoreClosedTab); visual.run_until_parked();
                visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[entity.read(cx).active].session_id, old_id));
                assert!(first.send(Some(old_target.clone())).is_ok()); visual.run_until_parked();
                assert!(!old_target.exists());
            } else {
                let second = begin(visual, false);
                assert!(first.send(Some(old_target.clone())).is_ok()); visual.run_until_parked(); assert!(!old_target.exists());
                assert!(second.send(Some(new_target.clone())).is_ok()); visual.run_until_parked(); assert!(new_target.is_file());
            }
        });
    }
}

#[test]
fn actual_copy_untitled_readonly_bom_and_open_copy_choice() {
    let dir = crate::persist::TempDir::new("copy-untitled"); let target = dir.join("copy.txt");
    let mut buffer = Buffer::untitled(); buffer.rope = ropey::Rope::from_str("café中😀\n");
    buffer.dirty = true; buffer.read_only = true; buffer.encoding = Encoding::Utf8Bom; buffer.line_ending = LineEnding::Crlf;
    with_editor(buffer, |entity, visual| {
        let id = visual.update(|_, cx| entity.read(cx).tabs[0].session_id);
        let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx); let original = w.tabs.iter().find(|tab| tab.session_id == id).expect("original");
            assert!(original.buffer.file_path.is_none()); assert!(original.buffer.dirty); assert!(original.buffer.read_only);
            assert_eq!(w.tabs[w.active].buffer.file_path.as_ref(), Some(&target));
        });
        assert_eq!(std::fs::read(&target).expect("copy"), crate::text::encoding::encode_checked("café中😀\r\n", Encoding::Utf8Bom).expect("expected"));
        assert_eq!(crate::recent::snapshot(), vec![target.clone()]);
    });
}

#[test]
fn actual_native_copy_preserves_rtf_bytes() {
    let dir = crate::persist::TempDir::new("copy-native");
    let source = dir.join("source.rtf"); let target = dir.join("copy.rtf");
    let native = br"{\rtf1\ansi literal caf\'e9\par next}";
    std::fs::write(&source, native).expect("fixture");
    with_editor(crate::io::read_buffer(&source).expect("read native"), |entity, visual| {
        let sender = begin(visual, false); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("native copy"), native);
        visual.update(|_, cx| { let w = entity.read(cx); assert!(w.tabs[0].buffer.rich_doc.is_some()); assert!(!w.tabs[0].buffer.dirty); });
    });
}

#[test]
fn actual_copy_to_same_path_keeps_dirty_and_already_open_destination_is_refused() {
    for other_open in [false, true] {
        let dir = crate::persist::TempDir::new("copy-open-target"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        std::fs::write(&source, b"disk\r\n").expect("source"); std::fs::write(&target, b"other original").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            visual.simulate_input("unsaved ");
            if other_open { visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[1].buffer.file_path = Some(target.clone()))); }
            let sender = begin(visual, false);
            assert!(sender.send(Some(if other_open { target.clone() } else { source.clone() })).is_ok()); visual.run_until_parked();
            visual.update(|_, cx| { let w = entity.read(cx); assert!(w.tabs[0].buffer.dirty); assert_eq!(w.tabs[0].buffer.file_path.as_ref(), Some(&source)); });
            if other_open { assert_eq!(std::fs::read(&target).expect("target"), b"other original"); }
            else { assert_eq!(std::fs::read(&source).expect("source"), b"unsaved disk\r\n"); }
        });
    }
}

#[test]
fn actual_copy_does_not_record_a_macro_event() {
    let dir = crate::persist::TempDir::new("copy-macro"); let target = dir.join("copy.txt");
    let _recording = crate::macros::override_execution_for_test(true);
    with_editor(Buffer::untitled(), |_, visual| {
        let sender = begin(visual, false); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        crate::macros::stop_recording(); assert!(crate::macros::current().is_empty());
    });
}

fn odt_fixture() -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in [
        ("mimetype", "application/vnd.oasis.opendocument.text"),
        ("content.xml", "<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>café中😀</text:p></office:text></office:body></office:document-content>"),
        ("styles.xml", "<office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:styles/></office:document-styles>"),
        ("META-INF/manifest.xml", "<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"/>"),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).expect("zip part");
        zip.write_all(text.as_bytes()).expect("zip data");
    }
    zip.finish().expect("zip").into_inner()
}

#[test]
fn actual_native_docx_odt_copies_are_byte_identical_and_bad_raw_edits_are_refused() {
    let docx = include_bytes!("../rich_doc/fixtures/hello.docx").to_vec();
    for (extension, bytes) in [("docx", docx), ("odt", odt_fixture())] {
        for dirty in [false, true] {
            let dir = crate::persist::TempDir::new("copy-container");
            let source = dir.join(&format!("source.{extension}")); let target = dir.join(&format!("copy.{extension}"));
            std::fs::write(&source, &bytes).expect("native fixture");
            with_editor(crate::io::read_buffer(&source).expect("read native"), |entity, visual| {
                if dirty { visual.simulate_input("invalid raw source "); }
                let sender = begin(visual, false); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
                if dirty {
                    assert!(!target.exists());
                    visual.update(|_, cx| assert!(entity.read(cx).status_message.as_deref().is_some_and(|status| status.contains("failed"))));
                } else { assert_eq!(std::fs::read(&target).expect("native copy"), bytes); }
                assert_eq!(std::fs::read(&source).expect("original container"), bytes);
            });
        }
    }
}

#[test]
fn actual_native_copy_rejects_live_doc_view_edit_and_edit_undo_aba() {
    for undo in [false, true] {
        let dir = crate::persist::TempDir::new("copy-native-stale"); let source = dir.join("source.rtf"); let target = dir.join("copy.rtf");
        std::fs::write(&source, br"{\rtf1\ansi seed}").expect("native fixture");
        with_editor(crate::io::read_buffer(&source).expect("read native"), |entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, _| {
                let payload = w.tabs[0].buffer.rich_doc.as_mut().expect("payload");
                payload.active_block = Some(0);
                let point = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 };
                payload.selection = Some(crate::rich_doc::TextSelection { anchor: point, cursor: point });
            }));
            let sender = begin(visual, false);
            visual.update(|window, cx| entity.update(cx, |w, cx| {
                w.rich_doc_insert_text("late 中😀 ", cx);
                if undo { let focus = cx.focus_handle(); window.focus(&focus, cx); w.rich_doc_body_focus = Some(focus); w.undo(&EditUndo, window, cx); }
            }));
            assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked(); assert!(!target.exists());
        });
    }
}

#[test]
fn copy_legacy_doc_preserves_unedited_native_bytes_and_refuses_edits() {
    let _settings = crate::settings::override_for_test(Default::default());
    let dir = crate::persist::TempDir::new("copy-legacy"); let source = dir.join("source.rtf"); let target = dir.join("copy.doc");
    std::fs::write(&source, br"{\rtf1\ansi seed}").expect("native fixture");
    let mut buffer = crate::io::read_buffer(&source).expect("native buffer");
    let payload = buffer.rich_doc.as_mut().expect("payload"); payload.format = DocFormat::Doc;
    payload.original_bytes = Arc::new(b"synthetic legacy container bytes".to_vec());
    crate::io::copy_writer::write_copy(&buffer, &target).expect("unedited native copy");
    assert_eq!(std::fs::read(&target).expect("native bytes"), b"synthetic legacy container bytes");
    buffer.dirty = true;
    assert!(crate::io::copy_writer::write_copy(&buffer, &target).is_err());
    assert_eq!(std::fs::read(&target).expect("native unchanged"), b"synthetic legacy container bytes");
}

#[test]
fn actual_copy_worker_cancels_or_rejects_stale_ownership_before_destination_effects() {
    for change in ["edit", "undo", "path", "restore", "open-target", "external-target", "cancel", "escape", "close", "reorder"] {
        let dir = crate::persist::TempDir::new("copy-worker-owner"); let source = dir.join("source.txt"); let target = dir.join("copy.txt");
        std::fs::write(&source, "source café中😀\r\n").expect("source"); std::fs::write(&target, b"original destination").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let (release, held) = futures::channel::oneshot::channel(); let _gate = WorkerGates::new(Some(held), None);
            let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            visual.update(|_, cx| assert!(entity.read(cx).file_copy.is_some(), "{change}"));
            assert_eq!(std::fs::read(&target).expect("uncommitted target"), b"original destination");
            match change {
                "edit" | "undo" => { visual.simulate_input("x"); if change == "undo" { visual.dispatch_action(EditUndo); } },
                "restore" => { visual.dispatch_action(FileCloseTab); visual.run_until_parked(); visual.dispatch_action(FileRestoreClosedTab); },
                "path" => visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.file_path = Some(dir.join("other.txt")))),
                "open-target" => visual.update(|_, cx| entity.update(cx, |w, cx| w.open_path(target.clone(), cx))),
                "external-target" => std::fs::write(&target, b"external replacement must survive").expect("external write"),
                "cancel" => { let bounds = visual.debug_bounds("loading-cancel").expect("cancel chip"); visual.simulate_click(bounds.center(), gpui::Modifiers::none()); },
                "escape" => visual.simulate_keystrokes("escape"),
                "close" => { assert!(visual.simulate_close()); },
                "reorder" => visual.update(|_, cx| entity.update(cx, |w, cx| { w.tabs.swap(0, 1); w.active = 0; w.touch_mru(); cx.notify(); })),
                _ => panic!("case"),
            }
            visual.run_until_parked(); assert!(release.send(()).is_ok()); visual.run_until_parked();
            let expected: &[u8] = match change { "reorder" => "source café中😀\r\n".as_bytes(), "external-target" => b"external replacement must survive", _ => b"original destination" };
            assert_eq!(std::fs::read(&target).expect("destination"), expected, "{change}");
            if change != "reorder" && change != "open-target" { assert!(crate::recent::snapshot().is_empty(), "{change}"); }
            assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 2, "stage cleanup: {change}");
        });
    }
}

#[test]
fn actual_copy_completion_rechecks_original_before_opening_or_recording_directory() {
    for change in ["edit", "path", "restore", "cancel", "close"] {
        let dir = crate::persist::TempDir::new("copy-completion-owner"); let source = dir.join("source.txt"); let target = dir.join("copy.txt");
        std::fs::write(&source, b"source\r\n").expect("source"); std::fs::write(&target, b"previous target").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let (release, held) = futures::channel::oneshot::channel(); let _gate = WorkerGates::new(None, Some(held));
            let before = crate::settings::snapshot().last_used_directory;
            let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            match change {
                "edit" => visual.simulate_input("late"),
                "path" => visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[0].buffer.file_path = Some(dir.join("other.txt")))),
                "restore" => { visual.dispatch_action(FileCloseTab); visual.run_until_parked(); visual.dispatch_action(FileRestoreClosedTab); },
                "cancel" => { let bounds = visual.debug_bounds("loading-cancel").expect("cancel chip"); visual.simulate_click(bounds.center(), gpui::Modifiers::none()); },
                "close" => { assert!(visual.simulate_close()); },
                _ => panic!("case"),
            }
            visual.run_until_parked(); assert!(release.send(()).is_ok()); visual.run_until_parked();
            assert!(crate::recent::snapshot().is_empty(), "{change}");
            assert_eq!(crate::settings::snapshot().last_used_directory, before, "{change}");
            assert_eq!(std::fs::read(&target).expect("target"), if matches!(change, "cancel" | "close") { b"previous target".as_slice() } else { b"source\r\n".as_slice() });
            assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 2);
        });
    }
}

#[test]
fn large_edited_native_capture_yields_and_cancel_preserves_original_history() {
    let dir = crate::persist::TempDir::new("copy-native-capture"); let target = dir.join("copy.rtf");
    let sample = "café中😀 ".repeat(1024 * 1024);
    let mut payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi seed}".to_vec()).expect("native");
    if let Some(crate::rich_doc::ast::Block::Paragraph { runs, .. }) = payload.ast.blocks.first_mut() {
        runs.first_mut().expect("run").text = sample;
    }
    payload.edit_log.push(crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: true });
    let _settings = crate::settings::override_for_test(Default::default());
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(&payload.raw_view_text); buffer.rich_doc = Some(payload); buffer.refresh_dirty();
    with_editor_view(buffer, Some(crate::tab::TabViewMode::Edit), |entity, visual| {
        let sender = begin(visual, true); let started = std::time::Instant::now(); assert!(sender.send(Some(target.clone())).is_ok());
        // One executor turn captures at most 64 KiB; the timer yields before the next chunk.
        visual.run_until_parked();
        let first_turn = started.elapsed();
        visual.update(|_, cx| assert!(entity.read(cx).file_copy.is_some()));
        visual.simulate_keystrokes("escape"); visual.run_until_parked();
        println!("FILE_NATIVE_COPY_CANCEL first_capture_turn_ms={:.3} cancel_complete_ms={:.3}", first_turn.as_secs_f64() * 1e3, started.elapsed().as_secs_f64() * 1e3);
        assert!(!target.exists());
        visual.update(|_, cx| { let payload = entity.read(cx).tabs[0].buffer.rich_doc.as_ref().expect("original payload");
            assert_eq!(payload.edit_log.len(), 1); assert!(payload.undo_stack.is_empty()); assert!(entity.read(cx).tabs[0].buffer.dirty); });
        assert_eq!(std::fs::read_dir(dir.path()).expect("no stage").count(), 0);
    });
}

#[test]
fn actual_copy_refuses_destination_alias_open_in_another_workspace_window() {
    let dir = crate::persist::TempDir::new("copy-other-window"); let source = dir.join("source.txt"); let target = dir.join("target.txt"); let alias = dir.join("alias.txt");
    std::fs::write(&source, b"source").expect("source"); std::fs::write(&target, b"other window data").expect("target"); std::fs::hard_link(&target, &alias).expect("alias");
    with_editor(crate::io::read_buffer(&source).expect("read"), |_, visual| {
        let sender = begin(visual, false);
        let other = visual.update(|_, cx| cx.open_window(Default::default(), |_, cx| cx.new(|cx| {
            let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
            w.tabs = vec![Tab::with_buffer(crate::io::read_buffer(&alias).expect("other buffer"))];
            w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true); w
        }))).expect("other workspace window");
        assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        assert_eq!(std::fs::read(&target).expect("target"), b"other window data");
        assert_eq!(std::fs::read(&source).expect("source"), b"source"); assert!(crate::recent::snapshot().is_empty());
        visual.update(|_, cx| { let _ = other.update(cx, |_, window, _| window.remove_window()); });
    });
}

#[test]
fn actual_copy_cancel_with_failed_restore_reports_retained_rescue_bytes_even_after_source_edit() {
    for source_changed in [false, true] {
        let dir = crate::persist::TempDir::new("copy-rescue-error"); let source = dir.join("source.txt"); let target = dir.join("copy.txt");
        std::fs::write(&source, b"source\r\n").expect("source"); std::fs::write(&target, b"original destination to rescue").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let _fault = super::super::copy_stage::tests::Override::new(true);
            let (release, held) = futures::channel::oneshot::channel(); let _gate = WorkerGates::new(None, Some(held));
            let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            if source_changed { visual.simulate_input("late"); }
            assert!(release.send(()).is_ok()); visual.run_until_parked();
            let rescue = dir.join("copy.txt.notepad-studio-save");
            assert_eq!(std::fs::read(&rescue).expect("retained rescue"), b"original destination to rescue");
            visual.update(|_, cx| { let status = entity.read(cx).status_message.as_deref().expect("error");
                assert!(status.contains("putting the original back also failed")); assert!(status.contains(&rescue.display().to_string())); });
            assert!(crate::recent::snapshot().is_empty()); assert_eq!(std::fs::read(&source).expect("original"), b"source\r\n");
        });
    }
}

#[test]
#[ignore = "optimized >10 MiB background copy caller timing"]
fn large_plain_copy_caller_timing() {
    let _settings = crate::settings::override_for_test(Default::default());
    let dir = crate::persist::TempDir::new("copy-timing"); let target = dir.join("copy.txt"); let baseline = dir.join("baseline.txt");
    let sample = "café中😀\n".repeat(1024 * 1024); let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(&sample); buffer.line_ending = LineEnding::Crlf;
    let before = std::time::Instant::now(); crate::io::copy_writer::write_copy(&buffer, &baseline).expect("baseline"); let blocking = before.elapsed();
    with_editor(buffer, |entity, visual| {
        let sender = begin(visual, false); let started = std::time::Instant::now(); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        let complete = started.elapsed();
        assert_eq!(std::fs::read(&target).expect("copy"), std::fs::read(&baseline).expect("baseline bytes"));
        visual.update(|_, cx| assert!(entity.read(cx).file_copy.is_none()));
        println!("FILE_COPY_TIMING input_bytes={} output_bytes={} blocking_baseline_ms={:.3} background_caller_ms={:.3}", sample.len(), std::fs::metadata(&target).expect("metadata").len(), blocking.as_secs_f64() * 1e3, complete.as_secs_f64() * 1e3);
    });
}

#[test]
fn actual_native_copy_allocation_refusal_keeps_both_files_and_original_history() {
    let dir = crate::persist::TempDir::new("copy-native-budget"); let source = dir.join("source.rtf"); let target = dir.join("copy.rtf");
    std::fs::write(&source, br"{\rtf1\ansi seed}").expect("source"); std::fs::write(&target, b"existing target").expect("target");
    with_editor(crate::io::read_buffer(&source).expect("native"), |entity, visual| {
        visual.simulate_input("raw edit ");
        let before = visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; (tab.undo_stack.len(), tab.buffer.raw_revision.clone(), tab.buffer.file_path.clone(), tab.buffer.rope.to_string()) });
        let _limit = super::super::copy_snapshot::tests::Limit::new(8);
        let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
        visual.update(|_, cx| { let w = entity.read(cx); let tab = &w.tabs[0];
            assert_eq!(before.0, tab.undo_stack.len()); assert!(before.1 == tab.buffer.raw_revision);
            assert_eq!(before.2, tab.buffer.file_path); assert_eq!(before.3, tab.buffer.rope.to_string());
            assert!(w.status_message.as_deref().is_some_and(|status| status.contains("snapshot limit"))); assert!(w.file_copy.is_none()); });
        assert_eq!(std::fs::read(&source).expect("source"), br"{\rtf1\ansi seed}");
        assert_eq!(std::fs::read(&target).expect("target"), b"existing target"); assert!(crate::recent::snapshot().is_empty());
        assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 2);
    });
}

#[test]
fn actual_edited_native_copy_uses_the_native_writer_and_keeps_source_undo_and_savepoint() {
    let docx = include_bytes!("../rich_doc/fixtures/hello.docx").to_vec();
    for (extension, bytes) in [("docx", docx), ("odt", odt_fixture()), ("rtf", br"{\rtf1\ansi seed}".to_vec())] {
        let dir = crate::persist::TempDir::new("copy-native-edited"); let source = dir.join(&format!("source.{extension}")); let target = dir.join(&format!("copy.{extension}"));
        std::fs::write(&source, &bytes).expect("source");
        with_editor(crate::io::read_buffer(&source).expect("native"), |entity, visual| {
            let before = visual.update(|_, cx| entity.update(cx, |w, cx| {
                let payload = w.tabs[0].buffer.rich_doc.as_mut().expect("payload"); payload.active_block = Some(0);
                let point = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 }; payload.selection = Some(crate::rich_doc::TextSelection { anchor: point, cursor: point });
                w.rich_doc_insert_text("café中😀 ", cx);
                let tab = &w.tabs[0]; let payload = tab.buffer.rich_doc.as_ref().expect("edited");
                (payload.ast.clone(), payload.undo_stack.len(), payload.edit_log.clone(), tab.buffer.savepoint.clone(), tab.buffer.dirty)
            }));
            let sender = begin(visual, false); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            let reference = dir.join(&format!("reference.{extension}"));
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0];
                crate::rich_doc::save(tab.buffer.rich_doc.as_ref().expect("native payload"), &reference, Some(&tab.buffer.rope.to_string())).expect("reference native writer"); });
            assert_eq!(std::fs::read(&target).expect("copied native bytes"), std::fs::read(&reference).expect("native writer bytes"), "{extension}");
            assert_eq!(std::fs::read(&source).expect("source preserved"), bytes);
            visual.update(|_, cx| { let tab = &entity.read(cx).tabs[0]; let payload = tab.buffer.rich_doc.as_ref().expect("original native");
                assert_eq!(payload.ast, before.0); assert_eq!(payload.undo_stack.len(), before.1); assert_eq!(payload.edit_log, before.2);
                assert!(tab.buffer.savepoint == before.3); assert_eq!(tab.buffer.dirty, before.4); assert_eq!(tab.buffer.file_path.as_ref(), Some(&source)); });
            assert!(crate::recent::snapshot().is_empty());
        });
    }
}

#[test]
fn actual_copy_refuses_open_destination_hardlinks_before_picker_write_and_after_staging() {
    for late in [false, true] {
        let dir = crate::persist::TempDir::new("copy-open-alias"); let source = dir.join("source.txt"); let target = dir.join("destination.txt"); let alias = dir.join("other-name.txt");
        std::fs::write(&source, b"source").expect("source"); std::fs::write(&target, b"open destination").expect("target"); std::fs::hard_link(&target, &alias).expect("alias");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let (release, held) = futures::channel::oneshot::channel(); let _gate = WorkerGates::new(late.then_some(held), None);
            if !late { visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[1] = Tab::with_buffer(crate::io::read_buffer(&alias).expect("alias")))); }
            let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            if late { visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[1] = Tab::with_buffer(crate::io::read_buffer(&alias).expect("alias")))); assert!(release.send(()).is_ok()); visual.run_until_parked(); }
            assert_eq!(std::fs::read(&target).expect("target"), b"open destination"); assert_eq!(std::fs::read(&source).expect("source"), b"source");
            assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 3);
        });
    }
}

#[test]
fn large_plain_copy_actual_staged_cancel_and_stale_controls_keep_both_files() {
    let sample = "café中😀\r\n".repeat(1024 * 1024);
    for change in ["cancel", "escape", "edit", "restore"] {
        let dir = crate::persist::TempDir::new("copy-large-controls"); let source = dir.join("source.txt"); let target = dir.join("copy.txt");
        std::fs::write(&source, sample.as_bytes()).expect("large source"); std::fs::write(&target, b"existing destination").expect("target");
        with_editor(crate::io::read_buffer(&source).expect("read"), |entity, visual| {
            let (release, held) = futures::channel::oneshot::channel(); let _gate = WorkerGates::new(Some(held), None);
            let sender = begin(visual, true); assert!(sender.send(Some(target.clone())).is_ok()); visual.run_until_parked();
            visual.update(|_, cx| assert!(entity.read(cx).file_copy.as_ref().expect("progress").total.load(Ordering::Relaxed) > crate::io_stream::STREAM_THRESHOLD_BYTES));
            match change {
                "cancel" => { let bounds = visual.debug_bounds("loading-cancel").expect("cancel chip"); visual.simulate_click(bounds.center(), gpui::Modifiers::none()); },
                "escape" => visual.simulate_keystrokes("escape"),
                "edit" => visual.simulate_input("late"),
                "restore" => { visual.dispatch_action(FileCloseTab); visual.run_until_parked(); visual.dispatch_action(FileRestoreClosedTab); },
                _ => panic!("case"),
            }
            visual.run_until_parked(); assert!(release.send(()).is_ok()); visual.run_until_parked();
            assert_eq!(std::fs::read(&target).expect("destination"), b"existing destination", "{change}");
            assert_eq!(std::fs::read(&source).expect("source"), sample.as_bytes(), "{change}");
            assert_eq!(std::fs::read_dir(dir.path()).expect("cleanup").count(), 2); assert!(crate::recent::snapshot().is_empty());
        });
    }
}
