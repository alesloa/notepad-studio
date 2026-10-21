use super::*;
use crate::{buffer::Buffer, settings::Settings};
use gpui::{
    AppContext, Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance,
    WindowOptions,
};

thread_local! { static GATE: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<()>>> = const { std::cell::RefCell::new(None) }; }
pub(super) fn take_gate() -> Option<futures::channel::oneshot::Receiver<()>> {
    GATE.with(|slot| slot.borrow_mut().take())
}

fn workspace(buffer: Buffer, cx: &mut Context<Workspace>) -> Workspace {
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
    w.tabs = vec![Tab::with_buffer(buffer), Tab::empty()];
    w.active = 0;
    w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
    w.session_final_save_result = Some(true);
    w
}

fn with_editor(source: &Path, run: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext)) {
    let _settings = crate::settings::override_for_test(Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        tab_exit_on_last_close: false,
        ..Default::default()
    });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    // Synthetic legacy payload: the DOC converter consumes its AST, and the
    // separately written source sentinel proves that export never overwrites it.
    let mut payload =
        crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi hello world}".to_vec()).unwrap();
    payload.format = DocFormat::Doc;
    let mut buffer = Buffer::empty();
    buffer.file_path = Some(source.to_path_buf());
    buffer.rope = ropey::Rope::from_str(&payload.raw_view_text);
    buffer.rich_doc = Some(payload);
    buffer.mark_saved();
    let mut app = TestAppContext::single();
    app.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = app.add_window_view(|_, cx| workspace(buffer, cx));
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&entity.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    run(&entity, visual);
    app.quit();
}

fn begin(visual: &mut VisualTestContext) -> futures::channel::oneshot::Sender<Option<PathBuf>> {
    let (reply, selected) = futures::channel::oneshot::channel();
    let _picker = crate::io::SaveAsDialogOverrideForTest::new(selected);
    visual.dispatch_action(RichDocSaveAsDocx);
    visual.run_until_parked();
    reply
}

fn open_other_window(visual: &mut VisualTestContext, destination: &Path) {
    let mut buffer = Buffer::empty();
    buffer.file_path = Some(destination.to_path_buf());
    visual
        .update(|_, cx| {
            cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|cx| workspace(buffer, cx))
            })
        })
        .expect("second editor");
    visual.run_until_parked();
}

#[test]
fn export_untouched_legacy_ast_survives_active_tab_change_and_opens_valid_docx() {
    let directory = crate::persist::TempDir::new("rich-export");
    let source = directory.join("source.doc");
    let destination = directory.join("converted café中.docx");
    std::fs::write(&source, b"legacy sentinel").unwrap();
    with_editor(&source, |entity, visual| {
        let reply = begin(visual);
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs.swap(0, 1);
            w.active = 0;
            cx.notify();
        });
        reply.send(Some(destination.clone())).unwrap();
        visual.run_until_parked();
        let payload = crate::rich_doc::open(&destination).expect("valid OOXML export");
        assert!(payload.flat_search_text().0.contains("hello world"));
        assert_eq!(std::fs::read(&source).unwrap(), b"legacy sentinel");
        visual.update(|_, cx| {
            assert!(
                entity
                    .read(cx)
                    .tabs
                    .iter()
                    .any(|tab| tab.buffer.file_path.as_ref() == Some(&source))
            )
        });
    });
}

#[test]
fn export_refuses_an_open_target_in_another_window_and_original_file_aliases() {
    for alias in [false, true] {
        let directory = crate::persist::TempDir::new("rich-export-open");
        let source = directory.join("source.doc");
        let destination = directory.join("target.docx");
        std::fs::write(&source, b"legacy sentinel").unwrap();
        if alias {
            std::fs::hard_link(&source, &destination).unwrap();
        } else {
            std::fs::write(&destination, b"target sentinel").unwrap();
        }
        with_editor(&source, |_, visual| {
            if !alias {
                open_other_window(visual, &destination);
            }
            let reply = begin(visual);
            reply.send(Some(destination.clone())).unwrap();
            visual.run_until_parked();
            assert_eq!(
                std::fs::read(&destination).unwrap(),
                if alias {
                    b"legacy sentinel".as_slice()
                } else {
                    b"target sentinel".as_slice()
                }
            );
            assert_eq!(std::fs::read(&source).unwrap(), b"legacy sentinel");
        });
    }
}

#[test]
fn late_export_picker_reply_cannot_write_after_edit_reopen_or_window_removal() {
    for mode in 0..3 {
        let directory = crate::persist::TempDir::new("rich-export-stale");
        let source = directory.join("source.doc");
        let destination = directory.join("target.docx");
        std::fs::write(&source, b"legacy sentinel").unwrap();
        std::fs::write(&destination, b"target sentinel").unwrap();
        with_editor(&source, |entity, visual| {
            let reply = begin(visual);
            match mode {
                0 => visual.update_window_entity(entity, |w, _, cx| {
                    w.replace_byte_range(0..0, "changed", cx)
                }),
                1 => visual.update_window_entity(entity, |w, _, _| {
                    let old = w.tabs.remove(0);
                    let id = old.session_id;
                    let mut reopened = Tab::with_buffer(old.buffer);
                    reopened.session_id = id;
                    w.tabs.insert(0, reopened);
                }),
                _ => visual.update(|window, _| window.remove_window()),
            }
            reply.send(Some(destination.clone())).unwrap();
            visual.run_until_parked();
            assert_eq!(std::fs::read(&destination).unwrap(), b"target sentinel");
        });
    }
}

#[test]
fn prepared_export_rechecks_other_windows_before_destination_write() {
    let directory = crate::persist::TempDir::new("rich-export-stage");
    let source = directory.join("source.doc");
    let destination = directory.join("target.docx");
    std::fs::write(&source, b"legacy sentinel").unwrap();
    std::fs::write(&destination, b"target sentinel").unwrap();
    with_editor(&source, |_, visual| {
        let (release, gate) = futures::channel::oneshot::channel();
        let _ = GATE.with(|slot| slot.replace(Some(gate)));
        let reply = begin(visual);
        reply.send(Some(destination.clone())).unwrap();
        visual.run_until_parked();
        open_other_window(visual, &destination);
        release.send(()).unwrap();
        visual.run_until_parked();
        assert_eq!(std::fs::read(&destination).unwrap(), b"target sentinel");
    });
}

#[test]
fn cancelling_prepared_export_preserves_destination_and_clears_progress() {
    let directory = crate::persist::TempDir::new("rich-export-cancel");
    let source = directory.join("source.doc");
    let destination = directory.join("target.docx");
    std::fs::write(&source, b"legacy sentinel").unwrap();
    std::fs::write(&destination, b"target sentinel").unwrap();
    with_editor(&source, |entity, visual| {
        let (release, gate) = futures::channel::oneshot::channel();
        let _ = GATE.with(|slot| slot.replace(Some(gate)));
        let reply = begin(visual);
        reply.send(Some(destination.clone())).unwrap();
        visual.run_until_parked();
        visual.update_window_entity(entity, |w, _, _| {
            w.file_copy
                .as_ref()
                .unwrap()
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed)
        });
        release.send(()).unwrap();
        visual.run_until_parked();
        assert_eq!(std::fs::read(&destination).unwrap(), b"target sentinel");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.file_copy.is_none() && w.file_copy_request.is_none());
        });
    });
}
