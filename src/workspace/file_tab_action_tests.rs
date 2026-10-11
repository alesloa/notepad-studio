// Actual handlers/UUID batch tasks on TestWindow; external launches stay in memory.
use super::*;
use crate::{
    buffer::LoadProgress,
    settings::{override_for_test, Settings},
    tab::{InputFocus, Tab, TabViewMode},
};
use gpui::{AppContext, EntityInputHandler, TestAppContext, VisualContext, WindowAppearance};
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Default)]
struct LaunchData {
    requests: Vec<(FileLaunch, PathBuf)>,
    error: Option<String>,
}

thread_local! { static LAUNCH: RefCell<Option<LaunchData>> = const { RefCell::new(None) }; }
thread_local! { static RELOAD_RESPONSE: RefCell<Option<bool>> = const { RefCell::new(None) }; }

struct ReloadResponseGuard {
    previous: Option<bool>,
    same_thread: PhantomData<Rc<()>>,
}
impl Drop for ReloadResponseGuard {
    fn drop(&mut self) {
        RELOAD_RESPONSE.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}
fn override_reload_response(response: bool) -> ReloadResponseGuard {
    ReloadResponseGuard {
        previous: RELOAD_RESPONSE.with(|slot| slot.replace(Some(response))),
        same_thread: PhantomData,
    }
}
pub(super) fn reload_response() -> Option<bool> {
    RELOAD_RESPONSE.with(|slot| *slot.borrow())
}

struct LaunchGuard {
    previous: Option<LaunchData>,
    same_thread: PhantomData<Rc<()>>,
}
impl Drop for LaunchGuard {
    fn drop(&mut self) {
        LAUNCH.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}
fn override_launch(error: Option<&str>) -> LaunchGuard {
    LaunchGuard {
        previous: LAUNCH.with(|slot| {
            slot.replace(Some(LaunchData {
                requests: Vec::new(),
                error: error.map(str::to_owned),
            }))
        }),
        same_thread: PhantomData,
    }
}
pub(super) fn capture_launch(kind: FileLaunch, path: &Path) -> Option<Result<(), String>> {
    LAUNCH.with(|slot| {
        let mut data = slot.borrow_mut();
        let data = data.as_mut()?;
        data.requests.push((kind, path.to_path_buf()));
        Some(data.error.clone().map_or(Ok(()), Err))
    })
}
fn requests() -> Vec<(FileLaunch, PathBuf)> {
    LAUNCH.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("launch override")
            .requests
            .clone()
    })
}

fn with_workspace(
    setup: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
    assert: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    with_workspace_activation(false, setup, assert);
}

fn with_workspace_activation(
    deactivate: bool,
    setup: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
    assert: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    let _settings = override_for_test(Settings::default());
    let mut cx = TestAppContext::single();
    let entity = cx.new(|cx| {
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
        w.tabs = (0..4)
            .map(|index| {
                let mut tab = Tab::empty();
                tab.buffer.new_label = Some(format!("tab-{index}").into());
                tab
            })
            .collect();
        w.active = 1;
        w.window_closed = false;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        w
    });
    {
        let cx = cx.add_empty_window();
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        if deactivate {
            cx.deactivate_window();
        }
        cx.update_window_entity(&entity, |w, window, cx| {
            window.focus(&w.focus_handle, cx);
            setup(w, window, cx);
        });
        cx.run_until_parked();
        cx.update_window_entity(&entity, assert);
    }
    cx.quit();
}

fn attach_csv_draft(w: &mut Workspace, index: usize, read_only: bool, cx: &mut Context<Workspace>) {
    use crate::ui::csv_doc::{model::CsvCellInput, CsvModel};
    let tab = &mut w.tabs[index];
    tab.buffer.rope = ropey::Rope::from_str("cell\n");
    tab.buffer.read_only = read_only;
    tab.view_mode = TabViewMode::CsvPreview;
    let mut model = CsvModel::empty(b',');
    model.has_header = false;
    model.rows = vec![vec!["cell".into()]];
    model.col_order = vec![0];
    model.row_order = vec![0];
    model.bind_source(&tab.buffer.rope, &tab.buffer.raw_revision);
    model.grid_focus = Some(cx.focus_handle());
    model.editing_cell = Some((0, 0));
    model.cell_input = Some(CsvCellInput::new(cx.focus_handle(), "draft café".into()));
    w.csv_models.insert(tab.session_id, model);
}

fn focus_csv_grid(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> FocusHandle {
    attach_csv_draft(w, w.active, false, cx);
    let model = w
        .csv_models
        .get_mut(&w.tabs[w.active].session_id)
        .expect("CSV fixture");
    model.editing_cell = None;
    model.cell_input = None;
    let focus = model.grid_focus.clone().expect("grid focus");
    window.focus(&focus, cx);
    focus
}

fn current_csv_grid(
    w: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> FocusHandle {
    crate::ui::csv_doc::actions::ensure_model(w, window, cx);
    w.csv_models
        .get(&w.tabs[w.active].session_id)
        .and_then(|model| model.grid_focus.clone())
        .expect("rebuilt grid")
}

#[test]
fn reload_csv_restores_grid_focus_after_sync_and_confirmed_deferred_success() {
    for dirty in [false, true] {
        let fixture = crate::persist::TempDir::new("file-menu-reload-focus");
        let path = fixture.join("neutral.csv");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        let old_focus = RefCell::new(None);
        with_workspace(
            |w, window, cx| {
                old_focus.replace(Some(focus_csv_grid(w, window, cx)));
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.dirty = dirty;
                // Dropping before the async confirmation is polled proves caller-thread capture.
                let _response = override_reload_response(true);
                w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
            },
            |w, window, cx| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "disk\n");
                assert!(!w.tabs[1].buffer.dirty);
                assert!(
                    current_csv_grid(w, window, cx).is_focused(window),
                    "replacement grid is focused, dirty={dirty}"
                );
                assert!(!old_focus
                    .borrow()
                    .as_ref()
                    .expect("old grid")
                    .is_focused(window));
            },
        );
    }
}

#[test]
fn reload_commits_a_focused_cell_draft_then_hands_focus_to_the_replacement_grid() {
    let fixture = crate::persist::TempDir::new("file-menu-reload-cell-focus");
    let path = fixture.join("neutral.csv");
    std::fs::write(&path, "disk\n").expect("fixture disk");
    let old_focus = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            attach_csv_draft(w, 1, false, cx);
            let focus = w
                .csv_models
                .get(&w.tabs[1].session_id)
                .expect("CSV model")
                .cell_input
                .as_ref()
                .expect("cell input")
                .focus_handle
                .clone();
            window.focus(&focus, cx);
            old_focus.replace(Some(focus));
            w.tabs[1].buffer.file_path = Some(path.clone());
            let _response = override_reload_response(true);
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
            assert!(w.tabs[1].buffer.rope.to_string().contains("draft café"));
            assert!(w.tabs[1].buffer.dirty);
        },
        |w, window, cx| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "disk\n");
            assert!(w.tabs[1].undo_stack.is_empty());
            assert!(current_csv_grid(w, window, cx).is_focused(window));
            assert!(!old_focus
                .borrow()
                .as_ref()
                .expect("old cell")
                .is_focused(window));
        },
    );
}

#[test]
fn declined_csv_reload_keeps_source_model_and_current_focus() {
    let fixture = crate::persist::TempDir::new("file-menu-reload-cancel");
    let path = fixture.join("neutral.csv");
    std::fs::write(&path, "disk\n").expect("fixture disk");
    let old_focus = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            old_focus.replace(Some(focus_csv_grid(w, window, cx)));
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.tabs[1].buffer.dirty = true;
            let _response = override_reload_response(false);
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
        },
        |w, window, cx| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
            assert!(w.tabs[1].buffer.dirty);
            assert!(old_focus
                .borrow()
                .as_ref()
                .expect("old grid")
                .is_focused(window));
            assert!(current_csv_grid(w, window, cx).is_focused(window));
        },
    );
}

#[test]
fn confirmed_csv_reload_tracks_the_same_active_uuid_after_reorder() {
    let fixture = crate::persist::TempDir::new("file-menu-reload-reorder");
    let path = fixture.join("neutral.csv");
    std::fs::write(&path, "disk\n").expect("fixture disk");
    let id = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            focus_csv_grid(w, window, cx);
            id.replace(Some(w.tabs[1].session_id));
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.tabs[1].buffer.dirty = true;
            let _response = override_reload_response(true);
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
            w.tabs.swap(1, 3);
            w.active = 3;
        },
        |w, window, cx| {
            assert_eq!(Some(w.tabs[3].session_id), *id.borrow());
            assert_eq!(w.tabs[3].buffer.rope.to_string(), "disk\n");
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "");
            assert!(current_csv_grid(w, window, cx).is_focused(window));
        },
    );
}

#[test]
fn csv_reload_handoff_does_not_override_newer_focus_tab_view_or_inactive_window() {
    for change in 0..4 {
        let fixture = crate::persist::TempDir::new("file-menu-reload-new-focus");
        let path = fixture.join("neutral.csv");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        let old_focus = RefCell::new(None);
        with_workspace_activation(
            change == 3,
            |w, window, cx| {
                old_focus.replace(Some(focus_csv_grid(w, window, cx)));
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.dirty = true;
                let _response = override_reload_response(true);
                w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
                match change {
                    0 => w.focus_input(InputFocus::FindQuery, window, cx),
                    1 => w.active = 0,
                    2 => w.tabs[1].view_mode = TabViewMode::Edit,
                    _ => {}
                }
            },
            |w, window, _| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "disk\n");
                assert!(
                    !w.csv_models.contains_key(&w.tabs[1].session_id),
                    "no handoff model for change={change}"
                );
                if change == 0 {
                    assert!(w.find_focus.is_focused(window));
                } else {
                    assert!(old_focus
                        .borrow()
                        .as_ref()
                        .expect("old grid")
                        .is_focused(window));
                }
            },
        );
    }
}

#[test]
fn failed_csv_reload_keeps_the_original_model_and_focus() {
    let fixture = crate::persist::TempDir::new("file-menu-reload-failed");
    let path = fixture.join("missing.csv");
    let old_focus = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            old_focus.replace(Some(focus_csv_grid(w, window, cx)));
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
        },
        |w, window, cx| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
            assert!(w
                .status_message
                .as_deref()
                .expect("failure status")
                .starts_with("Reload failed:"));
            assert!(old_focus
                .borrow()
                .as_ref()
                .expect("old grid")
                .is_focused(window));
            assert!(current_csv_grid(w, window, cx).is_focused(window));
        },
    );
}

#[test]
fn confirmed_reload_ignores_a_closed_target_uuid() {
    let fixture = crate::persist::TempDir::new("file-menu-reload-closed");
    let path = fixture.join("neutral.csv");
    std::fs::write(&path, "disk\n").expect("fixture disk");
    with_workspace(
        |w, window, cx| {
            focus_csv_grid(w, window, cx);
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.tabs[1].buffer.dirty = true;
            let _response = override_reload_response(true);
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
            let removed = w.tabs.remove(1);
            w.csv_models.remove(&removed.session_id);
            w.active = 0;
            window.focus(&w.focus_handle, cx);
        },
        |w, window, _| {
            assert_eq!(w.tabs.len(), 3);
            assert!(w.tabs.iter().all(|tab| tab.buffer.rope.len_bytes() == 0));
            assert!(w.focus_handle.is_focused(window));
        },
    );
}

#[test]
fn confirmed_reload_preserves_source_edits_made_after_the_prompt_opened() {
    for file in [false, true] {
        let fixture = crate::persist::TempDir::new("reload-late-source");
        let path = fixture.join("neutral.txt");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        let revision = RefCell::new(None);
        with_workspace(
            |w, window, cx| {
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.rope = ropey::Rope::from_str("old\n");
                w.tabs[1].buffer.dirty = true;
                let _response = override_reload_response(true);
                if file {
                    w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
                } else {
                    w.reload_tab(1, window, cx);
                }
                w.replace_text_in_range(None, "new café😀 ", window, cx);
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "new café😀 old\n");
                revision.replace(Some(w.tabs[1].buffer.raw_revision.clone()));
                w.focus_input(InputFocus::FindQuery, window, cx);
            },
            |w, window, _| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "new café😀 old\n");
                assert_eq!(Some(&w.tabs[1].buffer.raw_revision), revision.borrow().as_ref());
                assert_eq!(w.tabs[1].undo_stack.len(), 1);
                assert!(w.tabs[1].buffer.dirty);
                assert!(w.find_focus.is_focused(window));
                assert_eq!(w.status_message.as_deref(), Some("Reload cancelled: document changed while confirmation was open; reload again to confirm the current contents"));
            },
        );
        assert_eq!(std::fs::read_to_string(&path).expect("disk unchanged"), "disk\n");
    }
}

#[test]
fn confirmed_reload_preserves_a_new_csv_cell_editor_and_its_text() {
    for file in [false, true] {
        let fixture = crate::persist::TempDir::new("reload-late-cell");
        let path = fixture.join("neutral.csv");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        let focus = RefCell::new(None);
        let revision = RefCell::new(None);
        with_workspace(
            |w, window, cx| {
                focus_csv_grid(w, window, cx);
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.dirty = true;
                revision.replace(Some(w.tabs[1].buffer.raw_revision.clone()));
                let _response = override_reload_response(true);
                if file {
                    w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
                } else {
                    w.reload_tab(1, window, cx);
                }
                crate::ui::csv_doc::actions::open_cell_editor(w, 0, 0, window, cx);
                w.replace_text_in_range(None, " late 中😀", window, cx);
                let model = w.csv_models.get(&w.tabs[1].session_id).expect("model");
                let input = model.cell_input.as_ref().expect("new cell editor");
                assert_eq!(input.buffer, "cell late 中😀");
                focus.replace(Some(input.focus_handle.clone()));
            },
            |w, window, _| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
                assert_eq!(Some(&w.tabs[1].buffer.raw_revision), revision.borrow().as_ref());
                let model = w.csv_models.get(&w.tabs[1].session_id).expect("model survives");
                let input = model.cell_input.as_ref().expect("draft survives");
                assert_eq!(input.buffer, "cell late 中😀");
                assert_eq!((input.cursor, input.anchor), (input.buffer.len(), input.buffer.len()));
                assert_eq!(model.editing_cell, Some((0, 0)));
                assert!(focus.borrow().as_ref().expect("new editor focus").is_focused(window));
                assert!(w.tabs[1].buffer.dirty);
                assert_eq!(w.status_message.as_deref(), Some("Reload cancelled: a CSV cell edit started while confirmation was open; commit or cancel it before reloading"));
            },
        );
    }
}

#[test]
fn confirmed_reload_preserves_late_lazy_and_detached_csv_drafts() {
    for detached in [false, true] {
        let fixture = crate::persist::TempDir::new("reload-late-recovery");
        let path = fixture.join("neutral.csv");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        with_workspace(
            |w, window, cx| {
                focus_csv_grid(w, window, cx);
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.dirty = true;
                let _response = override_reload_response(true);
                w.reload_tab(1, window, cx);
                let draft = crate::session::CsvDraft {
                    value: "late recovered café😀".into(),
                    cursor: 5,
                    anchor: 0,
                    refusal: Some("Recover this value".into()),
                    ..Default::default()
                };
                if detached {
                    w.csv_models.get_mut(&w.tabs[1].session_id).expect("model").detached_draft = Some(draft);
                } else {
                    w.tabs[1].csv_draft = Some(draft);
                }
                w.focus_input(InputFocus::FindQuery, window, cx);
            },
            |w, window, _| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
                let draft = if detached {
                    w.csv_models.get(&w.tabs[1].session_id).expect("model survives").detached_draft.as_ref()
                } else {
                    w.tabs[1].csv_draft.as_ref()
                }.expect("draft survives");
                assert_eq!(draft.value, "late recovered café😀");
                assert_eq!((draft.cursor, draft.anchor), (5, 0));
                assert_eq!(draft.refusal.as_deref(), Some("Recover this value"));
                assert!(w.find_focus.is_focused(window));
                assert_eq!(w.status_message.as_deref(), Some("Reload cancelled: a CSV cell edit started while confirmation was open; commit or cancel it before reloading"));
            },
        );
    }
}

#[test]
fn confirmed_reload_preserves_rich_edits_when_raw_source_is_unchanged() {
    let fixture = crate::persist::TempDir::new("reload-late-rich");
    let path = fixture.join("neutral.rtf");
    std::fs::write(&path, br"{\rtf1\ansi seed}").expect("fixture disk");
    let raw = RefCell::new(String::new());
    let revision = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            w.tabs[1].buffer = crate::io::read_buffer(&path).expect("rich fixture");
            w.tabs[1].view_mode = TabViewMode::RichDocView;
            let payload = w.tabs[1].buffer.rich_doc.as_mut().expect("rich payload");
            let pos = crate::rich_doc::TextPos { block: 0, run: 0, byte: 0 };
            payload.selection = Some(crate::rich_doc::TextSelection { anchor: pos, cursor: pos });
            payload.active_block = Some(0);
            raw.replace(w.tabs[1].buffer.rope.to_string());
            w.tabs[1].buffer.dirty = true;
            w.rich_doc_body_focus = Some(cx.focus_handle());
            window.focus(w.rich_doc_body_focus.as_ref().expect("body focus"), cx);
            let _response = override_reload_response(true);
            w.reload_tab(1, window, cx);
            w.replace_text_in_range(None, "new 中😀 ", window, cx);
            let payload = w.tabs[1].buffer.rich_doc.as_ref().expect("rich edit");
            assert_eq!(payload.flat_search_text().0, "new 中😀 seed");
            revision.replace(Some(payload.revision.clone()));
        },
        |w, window, _| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), *raw.borrow());
            let payload = w.tabs[1].buffer.rich_doc.as_ref().expect("rich edit survives");
            assert_eq!(payload.flat_search_text().0, "new 中😀 seed");
            assert_eq!(Some(&payload.revision), revision.borrow().as_ref());
            assert_eq!(payload.undo_stack.len(), 1);
            assert_eq!(payload.edit_log.len(), 1);
            assert!(w.tabs[1].buffer.dirty);
            assert!(w.rich_doc_body_focus.as_ref().expect("body focus").is_focused(window));
            assert_eq!(w.status_message.as_deref(), Some("Reload cancelled: document changed while confirmation was open; reload again to confirm the current contents"));
        },
    );
}

#[test]
fn confirmed_reload_rejects_new_path_encoding_eol_loading_and_restored_identity() {
    for change in 0..5 {
        let fixture = crate::persist::TempDir::new("reload-late-metadata");
        let path = fixture.join("neutral.txt");
        let other = fixture.join("other.txt");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        std::fs::write(&other, "other disk\n").expect("fixture other disk");
        with_workspace(
            |w, window, cx| {
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.rope = ropey::Rope::from_str("old\n");
                w.tabs[1].buffer.dirty = true;
                let _response = override_reload_response(true);
                w.reload_tab(1, window, cx);
                match change {
                    0 => w.tabs[1].buffer.file_path = Some(other.clone()),
                    1 => w.tabs[1].buffer.encoding = crate::text::encoding::Encoding::Utf16Le,
                    2 => w.tabs[1].buffer.line_ending = crate::text::line_ending::LineEnding::Cr,
                    3 => w.tabs[1].buffer.loading = Some(LoadProgress {
                        bytes_read: 1, total_bytes: 2, cancel: Arc::new(AtomicBool::new(false)),
                    }),
                    _ => {
                        let id = w.tabs[1].session_id;
                        w.tabs[1] = Tab::empty();
                        w.tabs[1].session_id = id;
                        w.tabs[1].buffer.file_path = Some(path.clone());
                        w.tabs[1].buffer.rope = ropey::Rope::from_str("restored\n");
                    }
                }
                w.focus_input(InputFocus::FindQuery, window, cx);
            },
            |w, window, _| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), if change == 4 { "restored\n" } else { "old\n" });
                assert_eq!(w.tabs[1].buffer.file_path.as_ref(), Some(if change == 0 { &other } else { &path }));
                if change == 1 { assert_eq!(w.tabs[1].buffer.encoding, crate::text::encoding::Encoding::Utf16Le); }
                if change == 2 { assert_eq!(w.tabs[1].buffer.line_ending, crate::text::line_ending::LineEnding::Cr); }
                assert_eq!(w.tabs[1].buffer.loading.is_some(), change == 3);
                assert!(w.find_focus.is_focused(window));
                assert_eq!(w.status_message.as_deref(), Some("Reload cancelled: document changed while confirmation was open; reload again to confirm the current contents"));
            },
        );
    }
}

#[test]
fn reload_response_fixture_captures_before_poll_and_restores_nested_overrides() {
    assert_eq!(reload_response(), None);
    {
        let _outer = override_reload_response(false);
        let confirmation = {
            let _inner = override_reload_response(true);
            confirm_reload("neutral.csv".into())
        };
        assert_eq!(reload_response(), Some(false));
        let mut confirmation = std::pin::pin!(confirmation);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert_eq!(
            std::future::Future::poll(confirmation.as_mut(), &mut cx),
            std::task::Poll::Ready(true)
        );
        assert_eq!(
            std::thread::spawn(reload_response)
                .join()
                .expect("fixture thread"),
            None
        );
    }
    assert_eq!(reload_response(), None);
}

#[test]
fn external_file_actions_use_current_tab_and_literal_paths_without_editing() {
    let _launch = override_launch(None);
    let path = std::env::temp_dir().join("neutral & café (中) %PATH%.txt");
    with_workspace(
        |w, window, cx| {
            w.tabs[0].buffer.file_path = Some(PathBuf::from("wrong-tab.txt"));
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.tabs[1].buffer.rope = ropey::Rope::from_str("unsaved source");
            w.tabs[1].buffer.dirty = true;
            w.focus_input(InputFocus::FindQuery, window, cx);
            w.file_open_default_viewer(&FileOpenDefaultViewer, window, cx);
            w.file_open_containing_folder(&FileOpenContainingFolder, window, cx);
        },
        |w, _, _| {
            assert_eq!(
                requests(),
                [
                    (FileLaunch::Viewer, path.clone()),
                    (FileLaunch::Reveal, path.clone())
                ]
            );
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "unsaved source");
            assert!(w.tabs[1].buffer.dirty);
            assert!(w.tabs[1].undo_stack.is_empty());
        },
    );
}

#[test]
fn external_actions_report_untitled_invalid_paths_and_launcher_errors() {
    let _launch = override_launch(Some("synthetic launcher failure"));
    with_workspace(
        |w, window, cx| {
            w.file_open_default_viewer(&FileOpenDefaultViewer, window, cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Current document has no file path")
            );
            w.tabs[1].buffer.file_path = Some(PathBuf::from("invalid\0path"));
            w.file_open_containing_folder(&FileOpenContainingFolder, window, cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Current document has an invalid file path")
            );
            assert!(requests().is_empty());
            w.tabs[1].buffer.file_path = Some(std::env::temp_dir().join("neutral.txt"));
            w.file_open_default_viewer(&FileOpenDefaultViewer, window, cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Open in default viewer failed: synthetic launcher failure")
            );
            w.file_open_containing_folder(&FileOpenContainingFolder, window, cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Open containing folder failed: synthetic launcher failure")
            );
        },
        |_, _, _| {
            assert_eq!(requests().len(), 2);
        },
    );
}

#[test]
fn every_file_wrapper_refuses_a_missing_active_tab() {
    for empty in [false, true] {
        with_workspace(
            |w, window, cx| {
                if empty {
                    w.tabs.clear();
                } else {
                    w.active = usize::MAX;
                }
                w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
                w.file_open_default_viewer(&FileOpenDefaultViewer, window, cx);
                w.file_open_containing_folder(&FileOpenContainingFolder, window, cx);
                w.file_close_all_but_active(&FileCloseAllButActive, window, cx);
                w.file_close_all_to_left(&FileCloseAllToLeft, window, cx);
                w.file_close_all_to_right(&FileCloseAllToRight, window, cx);
                w.file_close_all_unchanged(&FileCloseAllUnchanged, window, cx);
            },
            |w, _, _| {
                assert_eq!(
                    w.status_message.as_deref(),
                    Some("No active document for this File command")
                );
                assert_eq!(w.tabs.len(), if empty { 0 } else { 4 });
            },
        );
    }
}

#[test]
fn close_multiple_batches_keep_the_original_uuid_targets_after_reorder() {
    for mode in 0..3 {
        let expected = RefCell::new(Vec::new());
        with_workspace(
            |w, window, cx| {
                let ids: Vec<_> = w.tabs.iter().map(|tab| tab.session_id).collect();
                match mode {
                    0 => {
                        expected.replace(vec![ids[1]]);
                        w.file_close_all_but_active(&FileCloseAllButActive, window, cx);
                    }
                    1 => {
                        expected.replace(vec![ids[1], ids[2], ids[3]]);
                        w.file_close_all_to_left(&FileCloseAllToLeft, window, cx);
                    }
                    _ => {
                        expected.replace(vec![ids[0], ids[1]]);
                        w.file_close_all_to_right(&FileCloseAllToRight, window, cx);
                    }
                }
                w.tabs.swap(0, 3);
                w.active = 1;
            },
            |w, _, _| {
                let mut remaining: Vec<_> = w.tabs.iter().map(|tab| tab.session_id).collect();
                let mut expected = expected.borrow_mut();
                remaining.sort();
                expected.sort();
                assert_eq!(remaining, *expected);
                assert_eq!(
                    w.tabs[w.active]
                        .buffer
                        .new_label
                        .as_ref()
                        .map(|label| label.to_string()),
                    Some("tab-1".into())
                );
            },
        );
    }
}

#[test]
fn reload_reports_untitled_and_loading_refusals_without_changes() {
    with_workspace(
        |w, window, cx| {
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Cannot reload — tab has no file path")
            );
            w.tabs[1].buffer.rope = ropey::Rope::from_str("partial\n");
            w.tabs[1].buffer.file_path = Some(std::env::temp_dir().join("neutral.txt"));
            w.tabs[1].buffer.loading = Some(LoadProgress {
                bytes_read: 1,
                total_bytes: 2,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
        },
        |w, _, _| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "partial\n");
            assert_eq!(
                w.status_message.as_deref(),
                Some("Cannot reload while the file is still loading")
            );
        },
    );
}

#[test]
fn reload_refuses_a_pending_read_only_csv_draft_before_discarding_it() {
    let fixture = crate::persist::TempDir::new("file-menu-reload-draft");
    let path = fixture.join("neutral.csv");
    std::fs::write(&path, "disk\n").expect("neutral CSV");
    with_workspace(
        |w, window, cx| {
            w.tabs.truncate(2);
            attach_csv_draft(w, 1, true, cx);
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
        },
        |w, _, _| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
            let model = w
                .csv_models
                .get(&w.tabs[1].session_id)
                .expect("draft survives");
            assert_eq!(
                model.cell_input.as_ref().expect("draft input").buffer,
                "draft café"
            );
            assert_eq!(
                w.status_message.as_deref(),
                Some("Cell edit not committed: Buffer is read-only")
            );
            assert_eq!(
                std::fs::read_to_string(&path).expect("disk unchanged"),
                "disk\n"
            );
        },
    );
}

#[test]
fn close_unchanged_aborts_before_removing_tabs_when_a_draft_cannot_commit() {
    with_workspace(
        |w, window, cx| {
            w.tabs.truncate(2);
            w.active = 0;
            attach_csv_draft(w, 0, true, cx);
            w.file_close_all_unchanged(&FileCloseAllUnchanged, window, cx);
        },
        |w, _, _| {
            assert_eq!(w.tabs.len(), 2, "draft refusal precedes the close batch");
            let model = w
                .csv_models
                .get(&w.tabs[0].session_id)
                .expect("draft survives");
            assert_eq!(
                model.cell_input.as_ref().expect("draft input").buffer,
                "draft café"
            );
            assert_eq!(
                w.status_message.as_deref(),
                Some("Cell edit not committed: Buffer is read-only")
            );
        },
    );
}

#[test]
fn close_unchanged_retains_a_committed_draft_and_closes_only_clean_tabs() {
    with_workspace(
        |w, window, cx| {
            w.active = 0;
            attach_csv_draft(w, 0, false, cx);
            w.file_close_all_unchanged(&FileCloseAllUnchanged, window, cx);
        },
        |w, _, _| {
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(
                w.tabs[0]
                    .buffer
                    .new_label
                    .as_ref()
                    .map(|label| label.to_string()),
                Some("tab-0".into())
            );
            assert!(w.tabs[0].buffer.dirty);
            assert!(w.tabs[0].buffer.rope.to_string().contains("draft café"));
            let model = w
                .csv_models
                .get(&w.tabs[0].session_id)
                .expect("CSV model remains");
            assert!(model.cell_input.is_none());
            assert_eq!(model.rows, vec![vec!["draft café"]]);
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        },
    );
}

#[test]
fn close_all_unchanged_keeps_a_fresh_blank_tab() {
    with_workspace(
        |w, window, cx| {
            w.file_close_all_unchanged(&FileCloseAllUnchanged, window, cx);
        },
        |w, _, _| {
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.active, 0);
            assert!(w.tabs[0].buffer.rope.len_bytes() == 0);
            assert!(!w.tabs[0].buffer.dirty);
        },
    );
}

#[test]
fn reload_current_saved_file_preserves_encoding_and_read_only_flag() {
    use crate::text::encoding::{encode_checked, Encoding};
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let fixture = crate::persist::TempDir::new("file-menu-reload-encoding");
        let path = fixture.join("neutral.txt");
        let bytes = encode_checked("disk café\r\n中😀", encoding).expect("fixture bytes");
        std::fs::write(&path, &bytes).expect("fixture disk");
        with_workspace(
            |w, window, cx| {
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.rope = ropey::Rope::from_str("old\n");
                w.tabs[1].buffer.read_only = true;
                w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
            },
            |w, _, _| {
                assert_eq!(w.tabs[1].buffer.encoding, encoding);
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "disk café\n中😀");
                assert!(w.tabs[1].buffer.read_only);
                assert!(!w.tabs[1].buffer.dirty);
                assert!(w.tabs[1].undo_stack.is_empty());
                assert_eq!(std::fs::read(&path).expect("disk unchanged"), bytes);
            },
        );
    }
}

#[test]
fn external_launch_fixture_restores_nested_overrides_without_sharing_threads() {
    assert!(LAUNCH.with(|slot| slot.borrow().is_none()));
    {
        let _outer = override_launch(None);
        let path = Path::new("outer.txt");
        assert!(capture_launch(FileLaunch::Viewer, path)
            .expect("outer fixture")
            .is_ok());
        assert!(
            std::thread::spawn(|| LAUNCH.with(|slot| slot.borrow().is_none()))
                .join()
                .expect("fixture thread")
        );
        {
            let _inner = override_launch(Some("inner error"));
            assert_eq!(
                capture_launch(FileLaunch::Reveal, Path::new("inner.txt")),
                Some(Err("inner error".into()))
            );
            assert_eq!(requests().len(), 1);
        }
        assert_eq!(requests(), [(FileLaunch::Viewer, path.to_path_buf())]);
    }
    assert!(LAUNCH.with(|slot| slot.borrow().is_none()));
}

#[test]
fn reload_audit_file_route_keeps_large_original_until_background_publish() {
    use std::io::Write;
    let fixture = crate::persist::TempDir::new("reload-audit-large-file");
    let path = fixture.join("neutral.txt");
    let mut file = std::fs::File::create(&path).expect("neutral fixture");
    let chunk = vec![b'x'; 1024 * 1024];
    for _ in 0..12 { file.write_all(&chunk).expect("fixture chunk"); }
    drop(file);
    with_workspace(|w, window, cx| {
        w.window_closed = false;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.session_final_save_result = Some(true);
        let tab = &mut w.tabs[w.active];
        tab.buffer.file_path = Some(path.clone());
        tab.buffer.rope = ropey::Rope::from_str("original editable source");
        tab.buffer.mark_saved();
        let before = tab.buffer.rope.clone();
        let started = std::time::Instant::now();
        w.file_reload_from_disk(&FileReloadFromDisk, window, cx);
        eprintln!("reload_audit_file_foreground_ms={:.3}", started.elapsed().as_secs_f64() * 1000.);
        assert!(w.tabs[w.active].buffer.rope.is_instance(&before), "reload must return with the old document installed before background work is polled");
    }, |_, _, _| {});
}
