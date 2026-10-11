// Actual tab-menu Reload dispatch on TestWindow, with neutral files and no prompts.
use super::*;
use crate::{
    settings::{override_for_test, Settings},
    tab::{Tab, TabViewMode},
};
use gpui::{AppContext, Context, TestAppContext, VisualContext, Window, WindowAppearance};
use std::cell::RefCell;

fn with_workspace(
    setup: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
    dispatch: impl FnOnce(&Entity<Workspace>, &mut Window, &mut gpui::App),
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
        w.tabs = (0..3).map(|_| Tab::empty()).collect();
        w.active = 1;
        w.window_closed = true;
        w
    });
    {
        let visual = cx.add_empty_window();
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        visual.update_window_entity(&entity, |w, window, cx| {
            window.focus(&w.focus_handle, cx);
            setup(w, window, cx);
        });
        visual.update(|window, cx| dispatch(&entity, window, cx));
        visual.run_until_parked();
        visual.update_window_entity(&entity, assert);
    }
    cx.quit();
}

fn attach_csv(
    w: &mut Workspace,
    target: usize,
    draft: bool,
    cx: &mut Context<Workspace>,
) -> gpui::FocusHandle {
    use crate::ui::csv_doc::{model::CsvCellInput, CsvModel};
    let tab = &mut w.tabs[target];
    tab.buffer.rope = ropey::Rope::from_str("cell\n");
    tab.view_mode = TabViewMode::CsvPreview;
    let mut model = CsvModel::empty(b',');
    model.has_header = false;
    model.rows = vec![vec!["cell".into()]];
    model.row_order = vec![0];
    model.col_order = vec![0];
    model.bind_source(&tab.buffer.rope, &tab.buffer.raw_revision);
    let focus = cx.focus_handle();
    model.grid_focus = Some(focus.clone());
    if draft {
        model.editing_cell = Some((0, 0));
        model.cell_input = Some(CsvCellInput::new(cx.focus_handle(), "draft café".into()));
    }
    w.csv_models.insert(tab.session_id, model);
    focus
}

fn dispatch_current_reload(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut gpui::App) {
    let painted = workspace.read(cx).tab_context_menu.expect("painted popup");
    run_leaf_action(LeafAction::Reload, painted, workspace, window, cx);
}

#[test]
fn tab_context_reload_refuses_read_only_drafts_in_active_and_background_targets() {
    for active in [0, 1] {
        let fixture = crate::persist::TempDir::new("context-reload-draft");
        let path = fixture.join("neutral.csv");
        std::fs::write(&path, "disk\n").expect("fixture disk");
        with_workspace(
            |w, _, cx| {
                w.active = active;
                attach_csv(w, 1, true, cx);
                w.tabs[1].buffer.read_only = true;
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
            },
            dispatch_current_reload,
            |w, _, _| {
                assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
                let model = w
                    .csv_models
                    .get(&w.tabs[1].session_id)
                    .expect("draft survives");
                assert_eq!(
                    model.cell_input.as_ref().expect("cell input").buffer,
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
}

#[test]
fn tab_context_reload_restores_a_focused_csv_grid() {
    let fixture = crate::persist::TempDir::new("context-reload-focus");
    let path = fixture.join("neutral.csv");
    std::fs::write(&path, "disk\n").expect("fixture disk");
    let old_focus = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            let focus = attach_csv(w, 1, false, cx);
            window.focus(&focus, cx);
            old_focus.replace(Some(focus));
            w.tabs[1].buffer.file_path = Some(path.clone());
            w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
        },
        dispatch_current_reload,
        |w, window, cx| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "disk\n");
            crate::ui::csv_doc::actions::ensure_model(w, window, cx);
            let focus = w
                .csv_models
                .get(&w.tabs[1].session_id)
                .and_then(|model| model.grid_focus.as_ref())
                .expect("new grid");
            assert!(focus.is_focused(window));
            assert!(!old_focus
                .borrow()
                .as_ref()
                .expect("old grid")
                .is_focused(window));
        },
    );
}

#[test]
fn tab_context_reload_does_not_reload_the_neighbor_after_reorder() {
    let fixture = crate::persist::TempDir::new("context-reload-reorder");
    let original = fixture.join("original.csv");
    let neighbor = fixture.join("neighbor.csv");
    std::fs::write(&original, "original disk\n").expect("original disk");
    std::fs::write(&neighbor, "neighbor disk\n").expect("neighbor disk");
    let id = RefCell::new(None);
    with_workspace(
        |w, _, cx| {
            id.replace(Some(w.tabs[1].session_id));
            w.tabs[1].buffer.file_path = Some(original.clone());
            w.tabs[1].buffer.rope = ropey::Rope::from_str("original old\n");
            w.tabs[2].buffer.file_path = Some(neighbor.clone());
            w.tabs[2].buffer.rope = ropey::Rope::from_str("neighbor old\n");
            w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
            w.move_tab(1, 2, cx);
        },
        // The painted row captured index1 before the strip was reordered.
        dispatch_current_reload,
        |w, _, _| {
            let original = w
                .tabs
                .iter()
                .find(|tab| Some(tab.session_id) == *id.borrow())
                .expect("original UUID");
            assert_eq!(original.buffer.rope.to_string(), "original disk\n");
            let neighbor = w
                .tabs
                .iter()
                .find(|tab| tab.buffer.file_path.as_ref() == Some(&neighbor))
                .expect("neighbor UUID");
            assert_eq!(neighbor.buffer.rope.to_string(), "neighbor old\n");
        },
    );
}

#[test]
fn invalid_or_failed_context_reload_keeps_source_and_csv_focus() {
    let fixture = crate::persist::TempDir::new("context-reload-missing");
    let old_focus = RefCell::new(None);
    with_workspace(
        |w, window, cx| {
            let focus = attach_csv(w, 1, false, cx);
            window.focus(&focus, cx);
            old_focus.replace(Some(focus));
            w.tabs[1].buffer.file_path = Some(fixture.join("missing.csv"));
            w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
        },
        |w, window, cx| {
            w.update(cx, |w, cx| w.reload_tab(usize::MAX, window, cx));
            dispatch_current_reload(w, window, cx);
        },
        |w, window, _| {
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
            assert!(w.csv_models.contains_key(&w.tabs[1].session_id));
            assert!(old_focus
                .borrow()
                .as_ref()
                .expect("old grid")
                .is_focused(window));
            assert!(w
                .status_message
                .as_deref()
                .expect("status")
                .starts_with("Reload failed:"));
        },
    );
}

fn stale_leaf_is_ignored(action: LeafAction) {
    for transition in 0..4 {
        let fixture = crate::persist::TempDir::new("context-stale-leaf");
        let path = fixture.join("neutral.txt");
        std::fs::write(&path, "disk untouched").expect("fixture disk");
        let painted = RefCell::new(None);
        let after = RefCell::new(Vec::new());
        let current_popup = RefCell::new(None);
        with_workspace(
            |w, _, cx| {
                w.tabs[1].buffer.file_path = Some(path.clone());
                w.tabs[1].buffer.rope = ropey::Rope::from_str("source untouched");
                // Close stays clean and never opens a native dirty prompt.
                w.tabs[1].buffer.read_only = matches!(action, LeafAction::ClearReadOnlyFlag);
                w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
                painted.replace(w.tab_context_menu);
                match transition {
                    0 => w.hide_tab_context_menu(cx),
                    1 => w.show_tab_context_menu(1, gpui::point(px(30.), px(30.)), cx),
                    2 => w.show_tab_context_menu(0, gpui::point(px(30.), px(30.)), cx),
                    _ => {
                        // A restored tab keeps its UUID, but its new popup has a new session.
                        let restored = w.tabs.remove(1);
                        w.tabs.push(restored);
                        w.active = 2;
                        w.show_tab_context_menu(2, gpui::point(px(30.), px(30.)), cx);
                    }
                }
                current_popup.replace(
                    w.tab_context_menu
                        .map(|state| (state.popup_id, state.target_tab_id)),
                );
                after.replace(
                    w.tabs
                        .iter()
                        .map(|tab| {
                            (
                                tab.session_id,
                                tab.buffer.rope.to_string(),
                                tab.buffer.read_only,
                            )
                        })
                        .collect(),
                );
                w.set_status("newer state");
            },
            |w, window, cx| {
                run_leaf_action(
                    action,
                    painted.borrow().expect("painted callback"),
                    w,
                    window,
                    cx,
                )
            },
            |w, _, _| {
                assert_eq!(
                    w.tabs
                        .iter()
                        .map(|tab| (
                            tab.session_id,
                            tab.buffer.rope.to_string(),
                            tab.buffer.read_only
                        ))
                        .collect::<Vec<_>>(),
                    *after.borrow()
                );
                assert_eq!(
                    w.tab_context_menu
                        .map(|state| (state.popup_id, state.target_tab_id)),
                    *current_popup.borrow()
                );
                assert!(w.tab_rename.is_none());
                assert_eq!(w.status_message.as_deref(), Some("newer state"));
                assert_eq!(
                    std::fs::read_to_string(&path).expect("disk unchanged"),
                    "disk untouched"
                );
            },
        );
    }
}

#[test]
fn stale_painted_close_preserves_closed_and_newer_popups() {
    stale_leaf_is_ignored(LeafAction::Close);
}
#[test]
fn stale_painted_save_preserves_files_and_newer_popups() {
    stale_leaf_is_ignored(LeafAction::Save);
}
#[test]
fn stale_painted_rename_preserves_dialog_and_newer_popups() {
    stale_leaf_is_ignored(LeafAction::Rename);
}
#[test]
fn stale_painted_read_only_preserves_flags_and_newer_popups() {
    stale_leaf_is_ignored(LeafAction::ToggleReadOnly);
    stale_leaf_is_ignored(LeafAction::ClearReadOnlyFlag);
}
#[test]
fn stale_painted_print_preserves_status_and_newer_popups() {
    stale_leaf_is_ignored(LeafAction::Print);
}

#[test]
fn painted_close_save_rename_and_read_only_follow_the_original_uuid_after_reorder() {
    for action in [
        LeafAction::Close,
        LeafAction::Save,
        LeafAction::Rename,
        LeafAction::ToggleReadOnly,
    ] {
        let fixture = crate::persist::TempDir::new("context-leaf-reorder");
        let original_path = fixture.join("original.txt");
        let neighbor_path = fixture.join("neighbor.txt");
        std::fs::write(&original_path, "original disk").expect("original disk");
        std::fs::write(&neighbor_path, "neighbor disk").expect("neighbor disk");
        let painted = RefCell::new(None);
        let original_id = RefCell::new(None);
        with_workspace(
            |w, _, cx| {
                original_id.replace(Some(w.tabs[1].session_id));
                w.tabs[1].buffer.file_path = Some(original_path.clone());
                w.tabs[1].buffer.rope = ropey::Rope::from_str("original source");
                w.tabs[2].buffer.file_path = Some(neighbor_path.clone());
                w.tabs[2].buffer.rope = ropey::Rope::from_str("neighbor source");
                w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
                painted.replace(w.tab_context_menu);
                w.move_tab(1, 2, cx);
            },
            |w, window, cx| {
                run_leaf_action(
                    action,
                    painted.borrow().expect("painted callback"),
                    w,
                    window,
                    cx,
                )
            },
            |w, _, _| {
                assert!(w.tab_context_menu.is_none());
                assert_eq!(
                    w.tabs
                        .iter()
                        .find(|tab| tab.buffer.file_path.as_ref() == Some(&neighbor_path))
                        .expect("neighbor survives")
                        .buffer
                        .rope
                        .to_string(),
                    "neighbor source"
                );
                assert_eq!(
                    std::fs::read_to_string(&neighbor_path).expect("neighbor disk"),
                    "neighbor disk"
                );
                match action {
                    LeafAction::Close => assert!(w
                        .tabs
                        .iter()
                        .all(|tab| Some(tab.session_id) != *original_id.borrow())),
                    LeafAction::Save => assert_eq!(
                        std::fs::read_to_string(&original_path).expect("original disk"),
                        "original source"
                    ),
                    LeafAction::Rename => assert_eq!(
                        w.tab_rename.as_ref().expect("rename opened").target_tab_idx,
                        2
                    ),
                    LeafAction::ToggleReadOnly => assert!(w.tabs[2].buffer.read_only),
                    _ => unreachable!(),
                }
            },
        );
    }
}

#[test]
fn matching_popup_with_a_closed_target_cannot_reload_its_neighbor() {
    let fixture = crate::persist::TempDir::new("context-target-closed");
    let path = fixture.join("neighbor.txt");
    std::fs::write(&path, "neighbor disk").expect("fixture disk");
    let painted = RefCell::new(None);
    with_workspace(
        |w, _, cx| {
            w.tabs[2].buffer.file_path = Some(path.clone());
            w.tabs[2].buffer.rope = ropey::Rope::from_str("neighbor source");
            w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
            painted.replace(w.tab_context_menu);
            w.tabs.remove(1);
            w.active = 1;
        },
        |w, window, cx| {
            run_leaf_action(
                LeafAction::Reload,
                painted.borrow().expect("painted callback"),
                w,
                window,
                cx,
            )
        },
        |w, _, _| {
            assert!(w.tab_context_menu.is_none());
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "neighbor source");
        },
    );
}

#[test]
fn lazy_recovered_drafts_refuse_file_and_context_reload_without_losing_data_or_focus() {
    use crate::actions::FileReloadFromDisk;
    for file in [false, true] {
        for read_only in [false, true] {
            for active in [0, 1] {
                if file && active == 0 {
                    continue;
                }
                for source_matches in [false, true] {
                    let fixture = crate::persist::TempDir::new("context-lazy-draft");
                    let path = fixture.join("neutral.csv");
                    std::fs::write(&path, "disk\n").expect("fixture disk");
                    with_workspace(
                        |w, window, cx| {
                            w.active = active;
                            w.tabs[1].buffer.file_path = Some(path.clone());
                            w.tabs[1].buffer.rope = ropey::Rope::from_str("cell\n");
                            w.tabs[1].buffer.read_only = read_only;
                            w.tabs[1].view_mode = TabViewMode::CsvPreview;
                            w.tabs[1].csv_draft = Some(crate::session::CsvDraft {
                                row: Some(0),
                                col: Some(0),
                                delimiter: Some(b','),
                                value: "recovered 中😀".into(),
                                cursor: 5,
                                anchor: 0,
                                source_matches,
                                refusal: (!source_matches)
                                    .then(|| "Recovered source changed".into()),
                                ..Default::default()
                            });
                            w.focus_input(crate::tab::InputFocus::FindQuery, window, cx);
                            w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
                        },
                        |w, window, cx| {
                            if file {
                                w.update(cx, |w, cx| {
                                    w.file_reload_from_disk(&FileReloadFromDisk, window, cx)
                                });
                            } else {
                                dispatch_current_reload(w, window, cx);
                            }
                        },
                        |w, window, _| {
                            assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
                            let draft = w.tabs[1].csv_draft.as_ref().expect("lazy draft survives");
                            assert_eq!(draft.value, "recovered 中😀");
                            assert_eq!(
                                (draft.cursor, draft.anchor, draft.row, draft.col),
                                (5, 0, Some(0), Some(0))
                            );
                            assert_eq!(draft.source_matches, source_matches);
                            assert_eq!(
                                draft.refusal.as_deref(),
                                (!source_matches).then_some("Recovered source changed")
                            );
                            assert_eq!(w.tabs[1].buffer.file_path.as_ref(), Some(&path));
                            assert_eq!(w.tabs[1].buffer.read_only, read_only);
                            assert!(!w.tabs[1].buffer.dirty);
                            assert_eq!(w.active, active);
                            assert!(!w.csv_models.contains_key(&w.tabs[1].session_id));
                            assert!(w.find_focus.is_focused(window));
                            assert_eq!(
                                w.status_message.as_deref(),
                                Some(concat!(
                                    "Open the recovered CSV grid, copy any needed cell text, ",
                                    "then commit or cancel the edit before reloading"
                                ))
                            );
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn lazy_draft_reload_keeps_the_existing_loading_and_untitled_refusal_order() {
    use crate::{actions::FileReloadFromDisk, buffer::LoadProgress};
    for file in [false, true] {
        for loading in [false, true] {
            with_workspace(
                |w, window, cx| {
                    w.tabs[1].buffer.rope = ropey::Rope::from_str("cell\n");
                    w.tabs[1].csv_draft = Some(crate::session::CsvDraft {
                        value: "recovered 中😀".into(),
                        ..Default::default()
                    });
                    if loading {
                        w.tabs[1].buffer.file_path = Some(std::env::temp_dir().join("neutral.csv"));
                        w.tabs[1].buffer.loading = Some(LoadProgress {
                            bytes_read: 1,
                            total_bytes: 2,
                            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        });
                    }
                    w.focus_input(crate::tab::InputFocus::FindQuery, window, cx);
                    w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
                },
                |w, window, cx| {
                    if file {
                        w.update(cx, |w, cx| {
                            w.file_reload_from_disk(&FileReloadFromDisk, window, cx)
                        });
                    } else {
                        dispatch_current_reload(w, window, cx);
                    }
                },
                |w, window, _| {
                    assert_eq!(
                        w.status_message.as_deref(),
                        Some(if loading {
                            "Cannot reload while the file is still loading"
                        } else {
                            "Cannot reload — tab has no file path"
                        })
                    );
                    assert_eq!(w.tabs[1].buffer.rope.to_string(), "cell\n");
                    assert_eq!(
                        w.tabs[1].csv_draft.as_ref().expect("draft kept").value,
                        "recovered 中😀"
                    );
                    assert_eq!(w.tabs[1].buffer.loading.is_some(), loading);
                    assert!(w.find_focus.is_focused(window));
                },
            );
        }
    }
}
