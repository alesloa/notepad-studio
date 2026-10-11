use gpui::{Context, EntityInputHandler, Window};
use crate::ui::csv_doc::actions::{open_cell_editor, cancel_cell_edit};
use crate::ui::csv_doc::actions::savepoint_tests::with_csv;
use crate::text::encoding::Encoding;
use crate::workspace::Workspace;
use crate::tab::Tab;

fn type_pending(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    open_cell_editor(w, 0, 0, window, cx);
    w.replace_text_in_range(Some(0..4), "draft 中😀", window, cx);
}

fn pending(w: &Workspace) -> &str {
    &w.csv_models.get(&w.tabs[w.active].session_id).expect("model").cell_input.as_ref().expect("pending editor retained").buffer
}

#[test]
fn csv_pending_ctrl_s_saves_the_visible_cell_value() {
    with_csv(Encoding::Utf16Le, |w, window, cx, path| {
        type_pending(w, window, cx);
        w.save_active(&crate::actions::FileSave, window, cx);
        let saved = crate::io::read_buffer(path).expect("saved CSV");
        assert!(saved.rope.to_string().contains("draft 中😀"));
        assert!(!w.buffer().dirty);
    });
}

#[test]
fn csv_pending_save_all_saves_even_a_previously_clean_buffer() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
        type_pending(w, window, cx);
        w.run_save_all(cx);
        assert!(crate::io::read_buffer(path).expect("saved CSV").rope.to_string().contains("draft 中😀"));
    });
}

#[test]
fn csv_pending_tab_context_save_targets_the_visible_value() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
        type_pending(w, window, cx);
        w.save_tab_at(0, cx);
        assert!(crate::io::read_buffer(path).expect("saved CSV").rope.to_string().contains("draft 中😀"));
    });
}

#[test]
fn csv_pending_tab_switch_commits_before_hiding_the_editor() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        w.tabs.push(Tab::empty());
        w.set_active_tab(1, cx);
        assert_eq!(w.active, 1);
        assert!(w.tabs[0].buffer.rope.to_string().contains("draft 中😀"));
        assert!(w.tabs[0].buffer.dirty);
    });
}

#[test]
fn csv_pending_failed_read_only_commit_keeps_editor_and_blocks_close() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        w.tabs.push(Tab::empty());
        w.active_tab_mut().buffer.read_only = true;
        let id = w.tabs[0].session_id;
        w.close_tab_at(0, window, cx); // Previously clean: no native dialog in the red fixture.
        assert!(w.tabs.iter().any(|tab| tab.session_id == id));
        assert_eq!(pending(w), "draft 中😀");
    });
}

#[test]
fn csv_pending_failed_read_only_commit_blocks_switch_and_retains_text() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        w.tabs.push(Tab::empty());
        w.active_tab_mut().buffer.read_only = true;
        w.set_active_tab(1, cx);
        assert_eq!(w.active, 0);
        assert_eq!(pending(w), "draft 中😀");
    });
}

#[test]
fn csv_pending_failed_save_keeps_the_cell_draft_visible() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        w.active_tab_mut().buffer.read_only = true;
        w.save_active(&crate::actions::FileSave, window, cx);
        assert_eq!(pending(w), "draft 中😀");
        assert!(w.status_message.as_ref().is_some_and(|status| status.contains("read-only")));
    });
}

#[test]
fn csv_pending_cancel_is_an_explicit_discard() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        let original = w.buffer().rope.to_string();
        type_pending(w, window, cx);
        cancel_cell_edit(w, cx);
        assert_eq!(w.buffer().rope.to_string(), original);
        assert!(!w.buffer().dirty);
        assert!(w.csv_models.get(&w.tabs[0].session_id).expect("model").cell_input.is_none());
    });
}

#[test]
fn csv_pending_source_mode_refusal_preserves_input_focus() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        w.active_tab_mut().buffer.read_only = true;
        w.view_csv_source(&crate::actions::ViewCsvSource, window, cx);
        assert_eq!(w.tabs[w.active].view_mode, crate::tab::TabViewMode::CsvPreview);
        assert!(matches!(w.input_focus(window), crate::tab::InputFocus::CsvCell));
        assert_eq!(pending(w), "draft 中😀");
    });
}

#[test]
fn csv_pending_stale_source_refuses_without_mutating_model_or_history() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        w.replace_byte_range(0..0, "external,change\n", cx);
        let source = w.buffer().rope.to_string();
        let text_history = w.tabs[w.active].undo_stack.len();
        assert!(!crate::ui::csv_doc::actions::commit_cell_edit(w, cx));
        crate::ui::csv_doc::actions::ensure_model(w, window, cx);
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!(w.tabs[w.active].undo_stack.len(), text_history);
        let model = w.csv_models.get(&w.tabs[w.active].session_id).expect("model");
        assert!(model.undo_stack.is_empty());
        assert_eq!(model.cell(0, 0), "caf\u{00E9}");
        assert_eq!(pending(w), "draft 中😀");
    });
}

#[test]
fn csv_pending_failed_serialization_is_atomic() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        let source = w.buffer().rope.to_string();
        w.csv_models.get_mut(&w.tabs[w.active].session_id).expect("model").delimiter = 0xFF;
        assert!(!crate::ui::csv_doc::actions::commit_cell_edit(w, cx));
        assert_eq!(w.buffer().rope.to_string(), source);
        assert!(!w.buffer().dirty);
        assert!(w.tabs[w.active].undo_stack.is_empty());
        assert!(w.csv_models.get(&w.tabs[w.active].session_id).expect("model").undo_stack.is_empty());
        assert_eq!(pending(w), "draft 中😀");
    });
}

#[test]
fn csv_pending_loading_refusal_keeps_draft() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        type_pending(w, window, cx);
        let source = w.buffer().rope.to_string();
        w.active_tab_mut().buffer.loading = Some(crate::buffer::LoadProgress {
            bytes_read: 1, total_bytes: 2,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        assert!(!crate::ui::csv_doc::actions::commit_cell_edit(w, cx));
        assert_eq!(w.buffer().rope.to_string(), source);
        assert_eq!(pending(w), "draft 中😀");
    });
}

#[test]
fn csv_pending_async_close_save_targets_uuid_and_keeps_other_tab_active() {
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = completed.clone();
    with_csv(Encoding::Utf16Be, |w, window, cx, path| {
        w.window_closed = false;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        type_pending(w, window, cx);
        let id = w.tabs[w.active].session_id;
        let path = path.to_path_buf();
        w.tabs.push(Tab::empty());
        w.active = 1;
        let active_id = w.tabs[1].session_id;
        cx.spawn_in(window, async move |this, cx| {
            super::save_tab_for_close(&this, cx, 0).await.expect("save for close");
            this.update(cx, |w, _| {
                assert_eq!(w.tabs[w.active].session_id, active_id);
                assert!(!w.tabs.iter().find(|tab| tab.session_id == id).expect("target exists").buffer.dirty);
            }).expect("workspace alive");
            assert!(crate::io::read_buffer(&path).expect("saved CSV").rope.to_string().contains("draft 中😀"));
            done.store(true, std::sync::atomic::Ordering::Relaxed);
        }).detach();
    });
    assert!(completed.load(std::sync::atomic::Ordering::Relaxed));
}
