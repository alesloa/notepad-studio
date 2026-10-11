// Exercise the registered, painted tab-popup row with a temporary trash backend.
use super::*;
use crate::{settings::{override_for_test, Settings}, tab::Tab, workspace::trash::test_support};
use gpui::{AppContext, Context, Render, TestAppContext, VisualContext, Window, WindowAppearance};

struct PaintedTrashMenu { workspace: Entity<Workspace>, state: TabContextMenuState, flags: TabFlags }
impl Render for PaintedTrashMenu {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        div().size_full().child(main_menu_panel(&theme, &self.state, &self.flags, self.workspace.clone()))
    }
}

#[test]
fn tab_trash_waits_for_confirmation_before_filesystem_or_document_effects() {
    let _settings = override_for_test(Settings::default());
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let directory = crate::persist::TempDir::new("tab-trash-confirmation");
    let path = directory.join("neutral.txt");
    std::fs::write(&path, "disk café中😀\r\n").expect("fixture file");
    let control = test_support::install(directory.path());
    let mut cx = TestAppContext::single();
    let workspace = cx.new(|cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        let mut tab = Tab::empty();
        tab.buffer.file_path = Some(path.clone());
        tab.buffer.rope = ropey::Rope::from_str("unsaved café中😀\r\n");
        tab.buffer.dirty = true;
        w.tabs = vec![tab, Tab::empty()];
        w.active = 0;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.show_tab_context_menu(0, gpui::point(px(20.), px(20.)), cx);
        w
    });
    let state = workspace.read_with(&cx, |w, _| w.tab_context_menu.expect("popup"));
    let flags = workspace.read_with(&cx, |w, _| flags_for(w, 0).expect("flags"));
    let y = trash_row_y(&flags);
    let (_, visual) = cx.add_window_view(|_, _| PaintedTrashMenu { workspace: workspace.clone(), state, flags });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.simulate_click(gpui::point(px(30.), px(y)), gpui::Modifiers::none());
    visual.run_until_parked();
    assert!(control.calls().is_empty(), "trash ran before the user answered");
    let prompts = control.prompts();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].0, path);
    assert!(prompts[0].1);
    assert!(prompts[0].2.contains("Unsaved changes"));
    control.answer(false);
    visual.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).expect("original disk"), "disk café中😀\r\n");
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.tabs.len(), 2);
        assert_eq!(w.tabs[0].buffer.rope.to_string(), "unsaved café中😀\r\n");
    });
    cx.quit();
}

fn trash_row_y(flags: &TabFlags) -> f32 {
    let mut y = 5.; // panel border and top padding, at the fixture's 16px rem.
    for item in main_items(flags) {
        match item {
            MenuItem::Leaf { action: LeafAction::MoveToTrash, .. } => return y + 11.,
            MenuItem::Sep => y += 9.,
            _ => y += 22.,
        }
    }
    panic!("the tab menu must register its trash command")
}

struct Fixture {
    workspace: Entity<Workspace>, path: std::path::PathBuf, disk: Vec<u8>, id: uuid::Uuid,
    control: test_support::Control, snapshots: std::sync::mpsc::Receiver<crate::session::SessionManifest>, y: f32,
}

fn with_fixture(count: usize, exit: bool, test: impl FnOnce(&Fixture, &mut gpui::VisualTestContext)) {
    use crate::text::encoding::{encode_checked, Encoding};
    let disk = encode_checked("disk café中😀\r\n", Encoding::Utf16Le).expect("encoded fixture");
    with_document(count, exit, "neutral café中😀.txt", &disk, test);
}

fn with_document(count: usize, exit: bool, name: &str, bytes: &[u8], test: impl FnOnce(&Fixture, &mut gpui::VisualTestContext)) {
    let _settings = override_for_test(Settings { tab_exit_on_last_close: exit, ..Default::default() });
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    let directory = crate::persist::TempDir::new("tab-trash-owned");
    let path = directory.join(name);
    let disk = bytes.to_vec();
    std::fs::write(&path, &disk).expect("fixture file");
    let control = test_support::install(directory.path());
    let (sink, snapshots) = std::sync::mpsc::channel();
    let mut cx = TestAppContext::single();
    let workspace = cx.new(|cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., WindowAppearance::Dark);
        w.tabs = (0..count).map(|_| Tab::empty()).collect();
        w.session_snapshot_sink = Some(sink);
        w.session_final_save_result = Some(true);
        w.active = if count == 1 { 0 } else { 1 };
        w.tabs[w.active].buffer = crate::io::read_buffer(&path).expect("read UTF-16 fixture");
        if let Some(payload) = w.tabs[w.active].buffer.rich_doc.as_mut() {
            payload.active_block = Some(0);
            let position = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 };
            payload.selection = Some(crate::rich_doc::TextSelection { anchor: position, cursor: position });
            w.rich_doc_insert_text("unsaved ", cx);
        } else { w.replace_byte_range(0..0, "unsaved\t", cx); }
        // Retire only the fixture's initial edit debounce; later real edits keep
        // their normal live-window recovery paths and all writes reach the sink.
        w.autosave_generation = w.autosave_generation.wrapping_add(1);
        w.post_edit_generation = w.post_edit_generation.wrapping_add(1);
        let tab = &mut w.tabs[w.active];
        tab.buffer.bookmarks.insert(0);
        tab.buffer.marks.push(0..3);
        tab.selection.anchor = 0; tab.selection.head = 3;
        tab.begin_select_marker = Some(0);
        tab.observe_content_activity();
        w.closed_tabs.push(crate::tab::ClosedTab { buffer: crate::buffer::Buffer::empty(), session_id: uuid::Uuid::new_v4() });
        w.touch_mru();
        w.show_tab_context_menu(w.active, gpui::point(px(20.), px(20.)), cx);
        w
    });
    let state = workspace.read_with(&cx, |w, _| w.tab_context_menu.expect("popup"));
    let flags = workspace.read_with(&cx, |w, _| flags_for(w, w.active).expect("flags"));
    let y = trash_row_y(&flags);
    let (_, visual) = cx.add_window_view(|_, _| PaintedTrashMenu { workspace: workspace.clone(), state, flags });
    visual.update(|window, cx| { window.activate_window(); window.focus(&workspace.read(cx).focus_handle.clone(), cx); });
    visual.run_until_parked();
    let fixture = Fixture { workspace, path, disk, id: state.target_tab_id, control, snapshots, y };
    test(&fixture, visual);
    assert!(crate::recent::snapshot().is_empty());
    cx.quit();
}

impl Fixture {
    fn begin(&self, visual: &mut gpui::VisualTestContext) {
        visual.simulate_click(gpui::point(px(30.), px(self.y)), gpui::Modifiers::none());
        visual.run_until_parked();
    }
    fn answer(&self, confirmed: bool, visual: &mut gpui::VisualTestContext) {
        self.control.answer(confirmed); visual.run_until_parked();
    }
    fn state(&self, cx: &TestAppContext) -> State {
        self.workspace.read_with(cx, |w, _| State::capture(w))
    }
    fn disk_unchanged(&self) { assert_eq!(std::fs::read(&self.path).expect("original disk remains"), self.disk); }
}

#[derive(PartialEq)]
struct State {
    tabs: Vec<TabState>, closed: Vec<uuid::Uuid>, active: usize, mru: Vec<uuid::Uuid>,
    autosave: u64, recovery: Vec<u8>, eager_recovery: Vec<u8>,
}
#[derive(PartialEq)]
struct TabState {
    id: uuid::Uuid, instance: usize, text: String, path: Option<std::path::PathBuf>,
    raw: crate::buffer::ContentRevision, activity: crate::buffer::ContentRevision,
    encoding: crate::text::encoding::Encoding, eol: crate::text::line_ending::LineEnding,
    dirty: bool, read_only: bool, saved: Option<crate::buffer::SavePoint>, selection: (usize, usize),
    undo: Vec<u8>, redo: Vec<u8>, bookmarks: Vec<usize>, marks: Vec<std::ops::Range<usize>>, marker: Option<usize>,
    draft: Vec<u8>,
}
impl State {
    fn capture(w: &Workspace) -> Self {
        Self { tabs: w.tabs.iter().map(|tab| TabState {
            id: tab.session_id, instance: std::sync::Arc::as_ptr(&tab.instance) as usize,
            text: tab.buffer.rope.to_string(), path: tab.buffer.file_path.clone(), raw: tab.buffer.raw_revision.clone(),
            activity: tab.content_activity.clone(), encoding: tab.buffer.encoding, eol: tab.buffer.line_ending,
            dirty: tab.buffer.dirty, read_only: tab.buffer.read_only, saved: tab.buffer.savepoint.clone(),
            selection: (tab.selection.anchor, tab.selection.head), undo: crate::buffer::serialize_undo_stack(&tab.undo_stack),
            redo: crate::buffer::serialize_undo_stack(&tab.redo_stack), bookmarks: tab.buffer.bookmarks.iter().copied().collect(),
            marks: tab.buffer.marks.clone(), marker: tab.begin_select_marker,
            draft: serde_json::to_vec(&tab.csv_draft).expect("draft state"),
        }).collect(), closed: w.closed_tabs.iter().map(|tab| tab.session_id).collect(), active: w.active,
            mru: w.tab_mru.iter().copied().collect(), autosave: w.autosave_generation,
            recovery: recovery_bytes(w.session_snapshot(crate::session::BakPolicy::Full)),
            eager_recovery: recovery_bytes(w.session_snapshot(crate::session::BakPolicy::UntitledOnly)) }
    }

    fn matches_recovery(&self, snapshot: crate::session::SessionManifest) -> bool {
        // Live edits schedule both recovery policies. Each must describe this
        // same document state; eager path-backed metadata intentionally differs.
        let bytes = recovery_bytes(snapshot);
        bytes == self.recovery || bytes == self.eager_recovery
    }
}

fn recovery_bytes(mut snapshot: crate::session::SessionManifest) -> Vec<u8> {
    // Each snapshot allocates a fresh CSV basis ID. Compare logical recovery
    // state while TabState separately checks the stored draft without alteration.
    for tab in &mut snapshot.tabs {
        if let Some(draft) = &mut tab.csv_draft { draft.backup_id = Some(uuid::Uuid::nil()); }
    }
    serde_json::to_vec(&snapshot).expect("snapshot")
}

#[test]
fn tab_trash_cancel_dropped_reply_and_backend_failure_preserve_edit_and_recovery_state() {
    for count in [1, 3] { for reply in 0..3 {
        with_fixture(count, count == 1, |fixture, visual| {
            let before = fixture.state(&visual.cx);
            fixture.begin(visual);
            assert_eq!(fixture.control.prompts().len(), 1);
            match reply {
                0 => fixture.answer(false, visual),
                1 => { fixture.control.drop_reply(); visual.run_until_parked(); }
                _ => { fixture.control.fail(); fixture.answer(true, visual); }
            }
            assert!(fixture.state(&visual.cx) == before, "cancel/error changed document or recovery state");
            assert_eq!(fixture.control.calls().len(), usize::from(reply == 2));
            assert!(fixture.snapshots.try_iter().all(|snapshot| before.matches_recovery(snapshot)),
                "cancel/error wrote a different recovery snapshot");
            fixture.disk_unchanged();
        });
    } }
}

#[test]
fn tab_trash_confirmation_follows_the_original_tab_across_switches_and_reordering() {
    for clean in [false, true] {
        with_fixture(3, false, |fixture, visual| {
            visual.update_window_entity(&fixture.workspace, |w, window, cx| {
                if clean { w.undo(&crate::actions::EditUndo, window, cx); assert!(!w.tabs[1].buffer.dirty); }
                w.active = 0;
            });
            fixture.begin(visual);
            assert_eq!(fixture.control.prompts()[0].1, !clean);
            visual.update_window_entity(&fixture.workspace, |w, _, cx| {
                w.move_tab(1, 2, cx); w.set_active_tab(0, cx);
            });
            let before = fixture.state(&visual.cx);
            fixture.snapshots.try_iter().for_each(drop);
            fixture.answer(true, visual);
            assert_eq!(fixture.control.calls(), vec![fixture.path.clone()]);
            assert!(!fixture.path.exists());
            assert_eq!(std::fs::read(fixture.control.destination()).expect("simulated trash"), fixture.disk);
            let after = fixture.state(&visual.cx);
            assert_eq!(after.tabs.len(), 2);
            assert!(after.tabs.iter().all(|tab| tab.id != fixture.id));
            assert!(after.tabs.iter().zip(&before.tabs).all(|(a, b)| a == b));
            assert_eq!(after.closed, before.closed);
            let saved: Vec<_> = fixture.snapshots.try_iter().collect();
            assert!(!saved.is_empty());
            assert!(saved.last().expect("final snapshot").tabs.iter().all(|tab| tab.id != fixture.id));
        });
    }
}

#[test]
fn tab_trash_stale_edits_incarnation_path_metadata_and_window_replies_have_no_effects() {
    for change in 0..13 {
        with_fixture(3, false, |fixture, visual| {
            fixture.begin(visual);
            visual.update_window_entity(&fixture.workspace, |w, window, cx| {
                match change {
                    0 => w.replace_byte_range(0..0, "late 中😀", cx),
                    1 => {
                        let original = w.tabs[1].buffer.raw_revision.clone();
                        w.replace_byte_range(0..0, "temporary", cx);
                        w.undo(&crate::actions::EditUndo, window, cx);
                        assert!(w.tabs[1].buffer.raw_revision == original, "real Undo returned to the captured revision");
                    }
                    2 => w.tabs[1].buffer.file_path = Some(fixture.path.with_file_name("changed-path.txt")),
                    3 => w.tabs[1].buffer.encoding = crate::text::encoding::Encoding::Utf8Bom,
                    4 => w.tabs[1].buffer.line_ending = crate::text::line_ending::LineEnding::Lf,
                    5 => w.tabs[1].buffer.read_only = true,
                    6 => w.tabs[1].buffer.mark_saved(),
                    7 => {
                        let old = w.tabs.remove(1);
                        w.tabs.insert(1, Tab::from_closed(crate::tab::ClosedTab { buffer: old.buffer, session_id: old.session_id }));
                    }
                    8 => { w.tabs.remove(1); w.active = 0; }
                    9 => w.window_closed = true,
                    10 => w.session_window_id = uuid::Uuid::new_v4(),
                    11 => w.tabs[1].buffer.loading = Some(crate::buffer::LoadProgress {
                        bytes_read: 0, total_bytes: 10, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    }),
                    _ => w.tabs[1].buffer.rope = ropey::Rope::from_str("replaced without adopting the old revision"),
                }
            });
            visual.run_until_parked();
            let before = fixture.state(&visual.cx);
            fixture.snapshots.try_iter().for_each(drop);
            fixture.answer(true, visual);
            assert!(fixture.control.calls().is_empty(), "stale reply reached trash, transition {change}");
            assert!(fixture.state(&visual.cx) == before, "stale reply altered current state, transition {change}");
            assert!(fixture.snapshots.try_iter().all(|snapshot| before.matches_recovery(snapshot)),
                "recovery changed after stale reply, transition {change}");
            fixture.disk_unchanged();
        });
    }
}

#[test]
fn tab_trash_native_window_closure_rejects_a_reply_even_if_workspace_entity_survives() {
    with_fixture(3, false, |fixture, visual| {
        fixture.begin(visual);
        let before = fixture.state(&visual.cx);
        visual.update(|window, _| window.remove_window());
        fixture.answer(true, visual);
        assert!(fixture.control.calls().is_empty());
        assert!(fixture.state(&visual.cx) == before);
        fixture.disk_unchanged();
    });
}

#[test]
fn tab_trash_last_tab_preflight_failure_prevents_the_filesystem_effect() {
    with_fixture(1, true, |fixture, visual| {
        let before = fixture.state(&visual.cx);
        fixture.begin(visual);
        fixture.control.recovery([false]);
        fixture.answer(true, visual);
        assert!(fixture.control.calls().is_empty());
        assert!(fixture.state(&visual.cx) == before);
        fixture.disk_unchanged();
        let saved: Vec<_> = fixture.snapshots.try_iter().collect();
        assert!(!saved.is_empty());
        assert!(saved.iter().all(|snapshot| recovery_bytes(snapshot.clone()) == before.recovery));
        assert_eq!(saved[0].tabs[0].id, fixture.id);
        assert_eq!(saved[0].tabs[0].path.as_ref(), Some(&fixture.path));
    });
}

#[test]
fn tab_trash_post_delete_recovery_failure_keeps_original_tab_history_as_editable_untitled() {
    with_fixture(1, true, |fixture, visual| {
        visual.update_window_entity(&fixture.workspace, |w, _, _| w.tabs[0].buffer.read_only = true);
        let before = fixture.state(&visual.cx);
        fixture.begin(visual);
        fixture.control.recovery([true, false]);
        fixture.answer(true, visual);
        assert_eq!(fixture.control.calls(), vec![fixture.path.clone()]);
        assert!(!fixture.path.exists());
        assert_eq!(std::fs::read(fixture.control.destination()).expect("trash bytes"), fixture.disk);
        let after = fixture.state(&visual.cx);
        let old = &before.tabs[0]; let kept = &after.tabs[0];
        assert_eq!(kept.id, old.id); assert_eq!(kept.instance, old.instance);
        assert_eq!(kept.text, old.text); assert_eq!(kept.undo, old.undo); assert_eq!(kept.redo, old.redo);
        assert_eq!(kept.selection, old.selection); assert_eq!(kept.bookmarks, old.bookmarks); assert_eq!(kept.marks, old.marks);
        assert!(kept.path.is_none() && kept.dirty && !kept.read_only && kept.saved.is_none());
        assert_eq!(kept.encoding, old.encoding); assert_eq!(kept.eol, old.eol); assert_eq!(after.closed, before.closed);
        fixture.workspace.read_with(&visual.cx, |w, _| {
            assert!(!w.window_closed);
            assert!(w.status_message.as_deref().is_some_and(|status| status.contains("Save As")));
        });
        let saved: Vec<_> = fixture.snapshots.try_iter().collect();
        assert!(saved.len() >= 3);
        let recovered = saved.last().expect("kept document recovery");
        assert_eq!(recovered.tabs[0].id, fixture.id);
        assert!(recovered.tabs[0].path.is_none() && recovered.tabs[0].dirty && !recovered.tabs[0].read_only);
        visual.update_window_entity(&fixture.workspace, |w, window, cx| {
            w.undo(&crate::actions::EditUndo, window, cx);
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "disk café中😀\n");
            assert!(w.tabs[0].buffer.dirty && w.tabs[0].buffer.file_path.is_none());
            assert_eq!(w.tabs[0].redo_stack.len(), 1);
        });
    });
}

#[test]
fn tab_trash_confirmed_last_tab_honors_exit_policy_and_final_recovery_snapshot() {
    for exit in [false, true] {
        with_fixture(1, exit, |fixture, visual| {
            fixture.begin(visual);
            fixture.answer(true, visual);
            assert_eq!(fixture.control.calls(), vec![fixture.path.clone()]);
            assert!(!fixture.path.exists());
            fixture.workspace.read_with(&visual.cx, |w, _| {
                assert_eq!(w.window_closed, exit);
                assert_eq!(w.tabs.len(), 1);
                assert_ne!(w.tabs[0].session_id, fixture.id);
                assert!(w.tabs[0].buffer.rope.len_bytes() == 0 && !w.tabs[0].buffer.dirty);
            });
            let saved: Vec<_> = fixture.snapshots.try_iter().collect();
            assert!(!saved.is_empty());
            let final_saved = saved.last().expect("final snapshot");
            assert!(final_saved.tabs.iter().all(|tab| tab.id != fixture.id));
            assert!(final_saved.tabs.is_empty(), "fresh empty tabs are not recovery documents");
        });
    }
}

fn attach_draft(w: &mut Workspace, kind: usize, cx: &mut Context<Workspace>) {
    let id = w.tabs[1].session_id;
    let draft = crate::session::CsvDraft { value: "draft café中😀".into(), cursor: 5, anchor: 0,
        refusal: Some("keep this draft".into()), ..Default::default() };
    match kind {
        0 => w.tabs[1].csv_draft = Some(draft),
        1 | 2 => {
            let mut model = crate::ui::csv_doc::CsvModel::empty(b',');
            if kind == 1 {
                model.cell_input = Some(crate::ui::csv_doc::model::CsvCellInput::new(cx.focus_handle(), draft.value));
                model.editing_cell = Some((0, 0));
            } else { model.detached_draft = Some(draft); }
            w.csv_models.insert(id, model);
        }
        _ => w.begin_edit_group(),
    }
}

#[test]
fn tab_trash_uncommitted_csv_drafts_and_transactions_are_never_committed_or_discarded() {
    for late in [false, true] { for kind in 0..4 {
        with_fixture(3, false, |fixture, visual| {
            if late { fixture.begin(visual); }
            visual.update_window_entity(&fixture.workspace, |w, _, cx| attach_draft(w, kind, cx));
            let before = fixture.state(&visual.cx);
            if late { fixture.answer(true, visual); } else { fixture.begin(visual); }
            assert!(fixture.control.calls().is_empty());
            assert_eq!(fixture.control.prompts().len(), usize::from(late));
            assert!(fixture.state(&visual.cx) == before);
            fixture.workspace.read_with(&visual.cx, |w, _| {
                match kind {
                    0 => assert_eq!(w.tabs[1].csv_draft.as_ref().expect("recovered draft").value, "draft café中😀"),
                    1 => assert_eq!(w.csv_models[&fixture.id].cell_input.as_ref().expect("live input").buffer, "draft café中😀"),
                    2 => assert_eq!(w.csv_models[&fixture.id].detached_draft.as_ref().expect("detached draft").value, "draft café中😀"),
                    _ => assert!(w.tabs[1].edit_transaction.is_some()),
                }
            });
            fixture.disk_unchanged();
        });
    } }
}

#[test]
fn tab_trash_native_format_edits_and_edit_undo_aba_reject_stale_confirmations() {
    for change in 0..4 {
        with_document(3, false, "neutral café中😀.rtf", br"{\rtf1\ansi seed}", |fixture, visual| {
            visual.update_window_entity(&fixture.workspace, |w, _, _| {
                let payload = w.tabs[1].buffer.rich_doc.as_mut().expect("native payload");
                payload.active_block = Some(0);
                let position = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 };
                payload.selection = Some(crate::rich_doc::TextSelection { anchor: position, cursor: position });
            });
            fixture.begin(visual);
            let raw = fixture.workspace.read_with(&visual.cx, |w, _| w.tabs[1].buffer.raw_revision.clone());
            visual.update_window_entity(&fixture.workspace, |w, window, cx| {
                match change {
                    0 | 1 => {
                        let before = w.tabs[1].buffer.rich_doc.as_ref().expect("payload").revision.clone();
                        w.rich_doc_insert_text("late 中😀 ", cx);
                        if change == 1 {
                            let focus = cx.focus_handle(); window.focus(&focus, cx); w.rich_doc_body_focus = Some(focus);
                            w.undo(&crate::actions::EditUndo, window, cx);
                            assert!(w.tabs[1].buffer.rich_doc.as_ref().expect("payload").revision == before);
                        }
                    }
                    2 => w.tabs[1].buffer.rich_doc.as_mut().expect("payload").format = crate::rich_doc::DocFormat::Odt,
                    _ => w.tabs[1].buffer.rich_doc.as_mut().expect("payload").original_bytes = std::sync::Arc::new(Vec::new()),
                }
                assert!(w.tabs[1].buffer.raw_revision == raw, "native edits leave raw source unchanged");
            });
            let before = fixture.state(&visual.cx);
            let native = fixture.workspace.read_with(&visual.cx, |w, _| {
                let payload = w.tabs[1].buffer.rich_doc.as_ref().expect("payload");
                (payload.flat_search_text().0, payload.undo_stack.len(), payload.redo_stack.len(), payload.revision.clone(), payload.format)
            });
            fixture.snapshots.try_iter().for_each(drop);
            fixture.answer(true, visual);
            assert!(fixture.control.calls().is_empty());
            assert!(fixture.state(&visual.cx) == before);
            fixture.workspace.read_with(&visual.cx, |w, _| {
                let payload = w.tabs[1].buffer.rich_doc.as_ref().expect("retained payload");
                assert_eq!((payload.flat_search_text().0, payload.undo_stack.len(), payload.redo_stack.len(), payload.revision.clone(), payload.format), native);
            });
            fixture.disk_unchanged();
        });
    }
}

#[test]
fn tab_trash_a_new_confirmation_supersedes_an_older_reply() {
    with_fixture(3, false, |fixture, visual| {
        fixture.begin(visual);
        visual.update_window_entity(&fixture.workspace, |w, _, cx| w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx));
        visual.update(|window, cx| {
            let painted = fixture.workspace.read(cx).tab_context_menu.expect("new popup");
            run_leaf_action(LeafAction::MoveToTrash, painted, &fixture.workspace, window, cx);
        });
        visual.run_until_parked();
        assert_eq!(fixture.control.prompts().len(), 2);
        let before = fixture.state(&visual.cx);
        fixture.answer(true, visual);
        assert!(fixture.control.calls().is_empty());
        assert!(fixture.state(&visual.cx) == before);
        fixture.answer(false, visual);
        assert!(fixture.control.calls().is_empty());
        fixture.disk_unchanged();
    });
}

#[test]
fn tab_trash_missing_paths_and_unfinished_loads_are_refused_before_any_prompt() {
    for loading in [false, true] {
        with_fixture(3, false, |fixture, visual| {
            visual.update_window_entity(&fixture.workspace, |w, _, _| {
                if loading {
                    w.tabs[1].buffer.loading = Some(crate::buffer::LoadProgress { bytes_read: 0, total_bytes: 10,
                        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) });
                } else { w.tabs[1].buffer.file_path = None; }
            });
            let before = fixture.state(&visual.cx);
            fixture.begin(visual);
            assert!(fixture.control.calls().is_empty() && fixture.control.prompts().is_empty());
            assert!(fixture.state(&visual.cx) == before);
            fixture.disk_unchanged();
        });
    }
}

#[test]
fn tab_trash_retains_native_payload_and_history_after_final_recovery_failure() {
    with_document(1, true, "neutral.rtf", br"{\rtf1\ansi seed}", |fixture, visual| {
        let before = fixture.state(&visual.cx);
        let native = fixture.workspace.read_with(&visual.cx, |w, _| {
            let payload = w.tabs[0].buffer.rich_doc.as_ref().expect("native document");
            (payload.original_bytes.clone(), payload.flat_search_text().0,
                payload.undo_stack.len(), payload.redo_stack.len(), payload.revision.clone(), payload.format)
        });
        assert!(native.2 > 0, "native initial edit recorded real history");
        fixture.begin(visual);
        fixture.control.recovery([true, false]);
        fixture.answer(true, visual);
        assert_eq!(fixture.control.calls(), vec![fixture.path.clone()]);
        assert_eq!(std::fs::read(fixture.control.destination()).expect("native trash bytes"), fixture.disk);
        let after = fixture.state(&visual.cx);
        assert_eq!(before.tabs[0].id, after.tabs[0].id);
        assert_eq!(before.tabs[0].instance, after.tabs[0].instance);
        fixture.workspace.read_with(&visual.cx, |w, _| {
            let tab = &w.tabs[0];
            assert!(tab.buffer.file_path.is_none() && tab.buffer.dirty && !w.window_closed);
            let payload = tab.buffer.rich_doc.as_ref().expect("retained native document");
            assert!(std::sync::Arc::ptr_eq(&payload.original_bytes, &native.0));
            assert_eq!((payload.flat_search_text().0, payload.undo_stack.len(), payload.redo_stack.len(), payload.revision.clone(), payload.format),
                (native.1.clone(), native.2, native.3, native.4.clone(), native.5));
        });
        let saved: Vec<_> = fixture.snapshots.try_iter().collect();
        let recovered = saved.last().expect("native recovery snapshot");
        assert_eq!(recovered.tabs[0].id, fixture.id);
        assert!(recovered.tabs[0].path.is_none() && recovered.tabs[0].rich_backup);
    });
}
