use super::*;
use crate::{actions::*, buffer::{ContentRevision, SavePoint}, settings::Settings, text::line_ending::LineEnding};
use crate::text::encoding;
use gpui::{Entity, TestAppContext, VisualTestContext};
use std::{fs, sync::Arc};

fn with_editor(tabs: Vec<Tab>, test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings { toolbar_hidden: true, folder_panel_visible: false,
        tab_exit_on_last_close: false, save_all_confirm: false, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = app.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.tabs = tabs; w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0); w.session_final_save_result = Some(true);
        w.touch_mru(); w
    });
    visual.update(|window, cx| { window.activate_window(); window.focus(&entity.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked(); test(&entity, visual); app.quit();
}

#[derive(PartialEq)]
struct State {
    id: Uuid, instance: Arc<()>, path: Option<PathBuf>, label: Option<String>, text: String,
    dirty: bool, readonly: bool, encoding: Encoding, eol: LineEnding, revision: ContentRevision,
    savepoint: Option<SavePoint>, undo: Vec<u8>, redo: Vec<u8>, selection: (usize, usize),
    native: Option<String>, native_revision: Option<ContentRevision>, native_bytes: Option<Arc<Vec<u8>>>,
    recovery: String,
}
impl State {
    fn capture(tab: &Tab) -> Self {
        Self { id: tab.session_id, instance: tab.instance.clone(), path: tab.buffer.file_path.clone(),
            label: tab.buffer.new_label.as_ref().map(ToString::to_string), text: tab.buffer.rope.to_string(),
            dirty: tab.buffer.dirty, readonly: tab.buffer.read_only, encoding: tab.buffer.encoding, eol: tab.buffer.line_ending,
            revision: tab.buffer.raw_revision.clone(), savepoint: tab.buffer.savepoint.clone(),
            undo: crate::buffer::serialize_undo_stack(&tab.undo_stack), redo: crate::buffer::serialize_undo_stack(&tab.redo_stack),
            selection: (tab.selection.anchor, tab.selection.head),
            native: tab.buffer.rich_doc.as_ref().map(|payload| format!("{:?}", (&payload.ast, &payload.edit_log, &payload.undo_stack, &payload.redo_stack))),
            native_revision: tab.buffer.rich_doc.as_ref().map(|payload| payload.revision.clone()),
            native_bytes: tab.buffer.rich_doc.as_ref().map(|payload| payload.original_bytes.clone()),
            recovery: toml::to_string(&crate::session::snapshot_tab_with_policy(tab, crate::session::BakPolicy::Full)).expect("recovery metadata"),
        }
    }
    fn assert_preserved(&self, tab: &Tab) {
        let after = Self::capture(tab); assert!(self == &after, "document, history, or recovery state changed on refusal");
        assert!(Arc::ptr_eq(&self.instance, &after.instance));
        if let (Some(before), Some(after)) = (&self.native_bytes, &after.native_bytes) { assert!(Arc::ptr_eq(before, after)); }
    }
}

fn source_tab(path: &Path, bytes: &[u8]) -> Tab {
    fs::write(path, bytes).expect("source fixture"); let mut tab = Tab::with_buffer(read_buffer(path).expect("load fixture"));
    tab.buffer.language = None; tab.view_mode = TabViewMode::Edit; tab
}

fn edit_source(entity: &Entity<Workspace>, visual: &mut VisualTestContext) {
    let rich = visual.update(|_, cx| entity.read(cx).tabs[0].buffer.rich_doc.is_some());
    if rich {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            let payload = w.tabs[0].buffer.rich_doc.as_mut().expect("native fixture"); payload.active_block = Some(0);
            let point = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 };
            payload.selection = Some(crate::rich_doc::TextSelection { anchor: point, cursor: point });
            w.rich_doc_insert_text("draft café中😀 ", cx);
        }));
    } else { visual.simulate_input("draft café中😀 "); }
    visual.run_until_parked();
}

fn select_target(visual: &mut VisualTestContext, path: &Path, copy: bool) {
    let (sender, reply) = futures::channel::oneshot::channel(); let _dialog = SaveAsDialogOverrideForTest::new(reply);
    let _options = save_copy::tests::Options::new(Some(false));
    if copy { visual.dispatch_action(FileSaveCopyAs); } else { visual.dispatch_action(FileSaveAs); }
    visual.run_until_parked(); assert!(sender.send(Some(path.to_owned())).is_ok()); visual.run_until_parked();
}

#[test]
fn actual_save_save_as_save_all_and_copy_rescue_errors_keep_source_history_and_recovery() {
    let formats = [("txt", encoding::encode_checked("disk café中😀\r\n", Encoding::Utf16Be).expect("UTF16")),
        ("rtf", br"{\rtf1\ansi seed}".to_vec()), ("docx", include_bytes!("../rich_doc/fixtures/hello.docx").to_vec())];
    for (extension, bytes) in formats {
        for action in ["save", "save_as", "save_all", "copy"] {
            for fault in [safety_copy::tests::Fault::Copy(112), safety_copy::tests::Fault::Sync(5)] {
                let dir = crate::persist::TempDir::new("save-caller-rescue"); let source = dir.join(&format!("source.{extension}"));
                let destination = dir.join(&format!("destination.{extension}"));
                let writing_source = matches!(action, "save" | "save_all");
                let target = if writing_source { &source } else { &destination };
                let tab = source_tab(&source, &bytes); if !writing_source { fs::write(&destination, b"unrelated prior target").expect("destination"); }
                let prior_target = fs::read(target).expect("prior destination");
                with_editor(vec![tab, Tab::empty()], |entity, visual| {
                    edit_source(entity, visual);
                    let before = visual.update(|_, cx| State::capture(&entity.read(cx).tabs[0]));
                    let _fault = safety_copy::tests::Override::new(target, fault);
                    match action {
                        "save" => visual.dispatch_action(FileSave), "save_all" => visual.dispatch_action(FileSaveAll),
                        "save_as" => select_target(visual, target, false), "copy" => select_target(visual, target, true),
                        _ => panic!("case"),
                    }
                    visual.run_until_parked();
                    visual.update(|_, cx| { let w = entity.read(cx); before.assert_preserved(&w.tabs[0]); assert_eq!(w.active, 0);
                        assert!(w.status_message.as_deref().is_some_and(|status| status.contains("couldn't keep a copy") && status.contains("nothing was changed")), "{action} {extension}: {:?}", w.status_message);
                        assert!(w.file_copy.is_none()); });
                    assert_eq!(fs::read(&source).expect("source"), bytes); assert_eq!(fs::read(target).expect("target"), prior_target);
                    assert!(crate::recent::snapshot().is_empty());
                    assert_eq!(fs::read_dir(dir.path()).expect("no owned rescue/stage").count(), if writing_source { 1 } else { 2 });
                });
            }
        }
    }
}

#[test]
fn actual_save_as_rescue_refusal_restores_untitled_label_and_bom_metadata() {
    let dir = crate::persist::TempDir::new("save-caller-untitled"); let target = dir.join("destination.txt");
    fs::write(&target, b"prior target\r\n").expect("target");
    let mut tab = Tab::empty(); tab.buffer.encoding = Encoding::Utf8Bom; tab.buffer.line_ending = LineEnding::Crlf;
    tab.buffer.new_label = Some("named café中😀.txt".into());
    with_editor(vec![tab, Tab::empty()], |entity, visual| {
        visual.simulate_input("unsaved café中😀\n");
        let before = visual.update(|_, cx| State::capture(&entity.read(cx).tabs[0]));
        let _fault = safety_copy::tests::Override::new(&target, safety_copy::tests::Fault::Sync(5));
        select_target(visual, &target, false);
        visual.update(|_, cx| before.assert_preserved(&entity.read(cx).tabs[0]));
        assert_eq!(fs::read(&target).expect("prior target"), b"prior target\r\n"); assert!(crate::recent::snapshot().is_empty());
    });
}

#[test]
fn actual_save_all_rescue_refusal_keeps_failed_tab_dirty_and_continues_other_targets() {
    let dir = crate::persist::TempDir::new("save-caller-batch"); let refused = dir.join("refused.txt"); let later = dir.join("later.txt");
    let first = source_tab(&refused, b"first old\r\n"); let second = source_tab(&later, b"second old\r\n");
    with_editor(vec![first, second, Tab::empty()], |entity, visual| {
        edit_source(entity, visual);
        let before = visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.set_active_tab(1, cx); w.replace_byte_range(0..0, "latest ", cx); w.set_active_tab(0, cx); State::capture(&w.tabs[0])
        }));
        let _fault = safety_copy::tests::Override::new(&refused, safety_copy::tests::Fault::Sync(5));
        visual.dispatch_action(FileSaveAll); visual.run_until_parked();
        visual.update(|_, cx| { let w = entity.read(cx); before.assert_preserved(&w.tabs[0]); assert!(!w.tabs[1].buffer.dirty); assert_eq!(w.active, 0);
            assert!(w.status_message.as_deref().is_some_and(|status| status.starts_with("Saved 1, 1 failed:") && status.contains("couldn't keep a copy"))); });
        assert_eq!(fs::read(&refused).expect("refused original"), b"first old\r\n");
        assert_eq!(fs::read(&later).expect("later saved"), b"latest second old\r\n");
    });
}

#[test]
fn synced_rescue_writer_preserves_encoding_bom_eol_and_unicode_bytes() {
    for selected in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for eol in [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr] {
            let text = if selected == Encoding::Latin1 { "café\nsecond\n" } else { "café中😀\nsecond\n" };
            let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(text); buffer.encoding = selected; buffer.line_ending = eol;
            let dir = crate::persist::TempDir::new("save-durability-fidelity"); let target = dir.join("target.txt");
            fs::write(&target, b"prior complete version\r\n").expect("old");
            copy_writer::write_buffer(&buffer, &target).expect("faithful overwrite");
            let raw = rope_to_bytes(&buffer.rope, eol); let expected = encoding::encode_checked(std::str::from_utf8(&raw).expect("UTF8 rope"), selected).expect("encoding");
            assert_eq!(fs::read(&target).expect("written"), expected);
            assert_eq!(fs::read_dir(dir.path()).expect("rescue cleanup").count(), 1);
        }
    }
}
