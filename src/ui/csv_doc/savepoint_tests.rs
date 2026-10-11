// Headless actual-handler tests; native rendering is outside this fixture.

use gpui::{AppContext, Context, TestAppContext, VisualContext, Window};

use super::{commit_cell_edit, ensure_model, open_cell_editor};
use crate::tab::{Tab, TabViewMode};
use crate::text::encoding::{self, Encoding};
use crate::workspace::Workspace;

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn with_csv(encoding: Encoding, test: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>, &std::path::Path)) {
    let fixture = Fixture(std::env::temp_dir().join(format!("notepad-studio-csv-savepoint-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&fixture.0).expect("neutral fixture directory");
    let path = fixture.0.join("source.csv");
    let source = concat!(r#""label","value""#, "\r\n", r#""café",1"#, "\r\n", r#""中😀",2"#);
    std::fs::write(&path, encoding::encode_checked(source, encoding).expect("fixture encoding")).expect("CSV fixture");
    let buffer = crate::io::read_buffer(&path).expect("open fixture");
    let mut cx = TestAppContext::single();
    let entity = cx.new(move |cx| {
        let mut workspace = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        let mut tab = Tab::empty(); tab.buffer = buffer;
        workspace.tabs = vec![tab]; workspace.active = 0;
        workspace.window_closed = true; // Block writes to the user's session/settings.
        workspace
    });
    {
        let cx = cx.add_empty_window();
        cx.update_window_entity(&entity, |w, window, cx| {
            w.set_csv_view_mode(true, cx);
            ensure_model(w, window, cx);
            focus_grid(w, window, cx);
            w.write_active_to_disk().expect("initial successful save");
            test(w, window, cx, &path);
        });
    }
    cx.quit();
}

fn focus_grid(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let model = w.csv_models.get(&w.tabs[w.active].session_id).expect("grid model");
    window.focus(model.grid_focus.as_ref().expect("grid focus"), cx);
}

fn edit_cell(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>, text: &str) {
    open_cell_editor(w, 0, 0, window, cx);
    let tab_id = w.tabs[w.active].session_id;
    w.csv_models.get_mut(&tab_id).expect("model").cell_input.as_mut().expect("cell input").buffer = text.into();
    commit_cell_edit(w, cx);
    focus_grid(w, window, cx);
}

#[test]
fn csv_undo_returns_to_the_saved_state() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        with_csv(encoding, |w, window, cx, path| {
            let saved = std::fs::read(path).expect("saved file");
            edit_cell(w, window, cx, "draft \u{4E2D}");
            assert!(w.buffer().dirty);
            w.undo(&crate::actions::EditUndo, window, cx);
            assert!(!w.buffer().dirty, "grid Undo reached the actual saved source");
            assert_eq!(std::fs::read(path).expect("disk stays saved"), saved);
        });
    }
}

#[test]
fn csv_undo_restores_original_quoted_source_spelling() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        let original = w.buffer().rope.to_string();
        edit_cell(w, window, cx, "draft");
        w.undo(&crate::actions::EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), original, "saved token requires exact source, including optional quotes");
    });
}

#[test]
fn csv_redo_returns_to_the_saved_state() {
    with_csv(Encoding::Utf16Le, |w, window, cx, path| {
        edit_cell(w, window, cx, "saved \u{1F600}");
        w.write_active_to_disk().expect("save edited grid");
        let saved = std::fs::read(path).expect("saved file");
        w.undo(&crate::actions::EditUndo, window, cx);
        assert!(w.buffer().dirty);
        w.redo(&crate::actions::EditRedo, window, cx);
        assert!(!w.buffer().dirty, "grid Redo reached the actual save");
        w.write_active_to_disk().expect("save unchanged redo");
        assert_eq!(std::fs::read(path).expect("same bytes"), saved);
    });
}

#[test]
fn csv_branches_at_the_same_history_depth_stay_dirty() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        edit_cell(w, window, cx, "saved");
        w.write_active_to_disk().expect("save branch");
        w.undo(&crate::actions::EditUndo, window, cx);
        edit_cell(w, window, cx, "different");
        assert!(w.buffer().dirty);
        w.redo(&crate::actions::EditRedo, window, cx);
        assert!(w.buffer().dirty, "discarded saved branch cannot return by stack depth");
        assert_eq!(w.csv_models.get(&w.tabs[w.active].session_id).expect("model").cell(0, 0), "different");
    });
}

#[test]
fn csv_metadata_changes_preserve_history_and_remain_unsaved() {
    with_csv(Encoding::Utf16Le, |w, window, cx, _| {
        edit_cell(w, window, cx, "draft");
        w.eol_unix(&crate::actions::EditEolUnix, window, cx);
        ensure_model(w, window, cx);
        assert_eq!(w.csv_models.get(&w.tabs[w.active].session_id).expect("model").undo_stack.len(), 1);
        w.undo(&crate::actions::EditUndo, window, cx);
        assert!(w.buffer().dirty, "LF metadata still differs from saved CRLF");
        w.eol_windows(&crate::actions::EditEolWindows, window, cx);
        assert!(!w.buffer().dirty);
        w.encoding_utf8(&crate::actions::EncodingUtf8, window, cx);
        ensure_model(w, window, cx);
        assert_eq!(w.csv_models.get(&w.tabs[w.active].session_id).expect("model").redo_stack.len(), 1);
        w.redo(&crate::actions::EditRedo, window, cx);
        w.undo(&crate::actions::EditUndo, window, cx);
        assert!(w.buffer().dirty, "UTF-8 metadata remains unsaved");
        w.encoding_utf16_le(&crate::actions::EncodingUtf16Le, window, cx);
        assert!(!w.buffer().dirty);
    });
}

#[test]
fn csv_other_tab_edits_do_not_discard_grid_history() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        edit_cell(w, window, cx, "draft");
        let id = w.tabs[w.active].session_id;
        w.tabs.push(Tab::empty()); w.active = 1;
        w.replace_byte_range(0..0, "unrelated tab", cx);
        w.active = 0;
        ensure_model(w, window, cx);
        focus_grid(w, window, cx);
        assert_eq!(w.csv_models.get(&id).expect("model").undo_stack.len(), 1);
        w.undo(&crate::actions::EditUndo, window, cx);
        assert!(!w.buffer().dirty);
    });
}

#[test]
fn csv_raw_source_edits_rebuild_and_invalidate_grid_history() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        edit_cell(w, window, cx, "grid draft");
        w.set_csv_view_mode(false, cx);
        window.focus(&w.focus_handle, cx);
        let total = w.buffer().rope.len_bytes();
        w.replace_byte_range(0..total, "label,value\nraw,3", cx);
        w.set_csv_view_mode(true, cx);
        ensure_model(w, window, cx);
        let model = w.csv_models.get(&w.tabs[w.active].session_id).expect("model");
        assert_eq!(model.cell(0, 0), "raw");
        assert!(model.undo_stack.is_empty());
        assert!(w.buffer().dirty);
    });
}

#[test]
fn csv_history_does_not_overwrite_newer_source_before_rebuild() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        edit_cell(w, window, cx, "grid draft");
        w.set_csv_view_mode(false, cx);
        window.focus(&w.focus_handle, cx);
        w.replace_byte_range(0..0, "new raw ", cx);
        let original = w.buffer().rope.to_string();
        w.set_csv_view_mode(true, cx);
        focus_grid(w, window, cx); // Exercise the action before the next preview paint.
        w.undo(&crate::actions::EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), original);
        assert!(w.buffer().dirty);
        ensure_model(w, window, cx);
        assert!(w.csv_models.get(&w.tabs[w.active].session_id).expect("model").undo_stack.is_empty());
    });
}

#[test]
fn csv_history_keeps_raw_view_replayable_after_restoring_saved_identity() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        let original = w.buffer().rope.to_string();
        edit_cell(w, window, cx, "draft");
        let edited = w.buffer().rope.to_string();
        w.undo(&crate::actions::EditUndo, window, cx);
        w.set_csv_view_mode(false, cx);
        window.focus(&w.focus_handle, cx);
        w.undo(&crate::actions::EditUndo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), edited);
        assert!(w.buffer().dirty);
        w.redo(&crate::actions::EditRedo, window, cx);
        assert_eq!(w.buffer().rope.to_string(), original);
        assert!(!w.buffer().dirty);
        assert_eq!(w.tabs[w.active].view_mode, TabViewMode::Edit);
    });
}

#[test]
fn csv_layout_history_does_not_rewrite_or_dirty_saved_source() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        let original = w.buffer().rope.to_string();
        let id = w.tabs[w.active].session_id;
        let model = w.csv_models.get_mut(&id).expect("model");
        model.push_undo();
        model.set_col_width(0, gpui::px(300.));
        let text_history = w.tabs[w.active].undo_stack.len();
        w.undo(&crate::actions::EditUndo, window, cx);
        assert!(!w.buffer().dirty);
        assert_eq!(w.buffer().rope.to_string(), original);
        assert_eq!(w.tabs[w.active].undo_stack.len(), text_history);
        w.redo(&crate::actions::EditRedo, window, cx);
        assert!(!w.buffer().dirty);
        assert_eq!(w.buffer().rope.to_string(), original);
        assert_eq!(f32::from(w.csv_models.get(&id).expect("model").col_width(0)), 300.);
    });
}

#[test]
fn csv_delimiter_choice_still_reinterprets_unchanged_source() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        let original = w.buffer().rope.to_string();
        let id = w.tabs[w.active].session_id;
        w.csv_models.get_mut(&id).expect("model").set_delimiter(b';');
        ensure_model(w, window, cx);
        let model = w.csv_models.get(&id).expect("model");
        assert_eq!(model.col_count(), 1);
        assert!(model.undo_stack.is_empty());
        assert_eq!(w.buffer().rope.to_string(), original);
        assert!(!w.buffer().dirty);
    });
}

#[test]
fn csv_refused_cell_commit_keeps_the_source_and_rebuilds_the_grid() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, _| {
        let original = w.buffer().rope.to_string();
        w.active_tab_mut().buffer.read_only = true;
        edit_cell(w, window, cx, "blocked");
        assert_eq!(w.buffer().rope.to_string(), original);
        assert!(!w.buffer().dirty);
        ensure_model(w, window, cx);
        assert_eq!(w.csv_models.get(&w.tabs[w.active].session_id).expect("model").cell(0, 0), "caf\u{00E9}");
    });
}
