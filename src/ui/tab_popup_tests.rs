// Real painted GPUI row callbacks; frozen views keep the captured old state.
use super::*;
use crate::{
    settings::{override_for_test, Settings},
    tab::{InputFocus, Tab},
};
use gpui::{AppContext, Context, Render, TestAppContext, VisualContext, Window, WindowAppearance};

#[derive(Clone, Copy)]
enum Panel {
    Main,
    Submenu(TabSubmenu),
    Backdrop,
}

struct PaintedMenu {
    workspace: Entity<Workspace>,
    state: TabContextMenuState,
    flags: TabFlags,
    panel: Panel,
}

impl Render for PaintedMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let panel = match self.panel {
            Panel::Main => {
                main_menu_panel(&theme, &self.state, &self.flags, self.workspace.clone())
                    .into_any_element()
            }
            Panel::Submenu(which) => submenu_panel(
                &theme,
                which,
                self.state,
                &self.flags,
                self.workspace.clone(),
            )
            .into_any_element(),
            Panel::Backdrop => tab_context_menu(
                &theme,
                self.state,
                flags_for(self.workspace.read(cx), 0).expect("flags"),
                self.workspace.clone(),
            )
            .into_any_element(),
        };
        div().size_full().child(panel)
    }
}

fn workspace(cx: &mut TestAppContext, directory: &std::path::Path) -> Entity<Workspace> {
    cx.new(|cx| {
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
        w.tabs = (0..3)
            .map(|i| {
                let mut tab = Tab::empty();
                tab.buffer.file_path = Some(directory.join(format!("neutral-{i}.txt")));
                tab.buffer.rope = ropey::Rope::from_str(&format!("source-{i}"));
                tab
            })
            .collect();
        w.active = 1;
        w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
        w.show_tab_context_menu(1, gpui::point(px(20.), px(20.)), cx);
        w
    })
}

fn clipboard(cx: &mut gpui::App) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

#[test]
fn tab_popup_submenu_rows_resolve_reordered_targets_and_reject_stale_popups() {
    for which in [
        TabSubmenu::CopyToClipboard,
        TabSubmenu::MoveDocument,
        TabSubmenu::ApplyColor,
        TabSubmenu::CloseMultiple,
        TabSubmenu::OpenInto,
    ] {
        for transition in 0..6 {
            // Exercise stale shell rows without ever invoking an OS launcher.
            if which == TabSubmenu::OpenInto && transition == 0 {
                continue;
            }
            let _settings = override_for_test(Settings::default());
            let directory = crate::persist::TempDir::new("painted-tab-submenu");
            let mut cx = TestAppContext::single();
            let w = workspace(&mut cx, directory.path());
            w.update(&mut cx, |w, cx| {
                w.open_tab_submenu(which, gpui::point(px(230.), px(20.)), cx)
            });
            let state = w.read_with(&cx, |w, _| w.tab_context_menu.expect("painted state"));
            let target = state.target_tab_id;
            let flags = w.read_with(&cx, |w, _| flags_for(w, 1).expect("flags"));
            let (_, visual) = cx.add_window_view(|_, _| PaintedMenu {
                workspace: w.clone(),
                state,
                flags,
                panel: Panel::Submenu(which),
            });
            visual.update(|window, _| window.activate_window());
            visual.run_until_parked();
            visual.update_window_entity(&w, |w, window, cx| {
                if which == TabSubmenu::OpenInto {
                    for tab in &mut w.tabs {
                        tab.buffer.file_path = None;
                    }
                }
                match transition {
                    0 => w.move_tab(1, 2, cx),
                    1 => {
                        w.tabs.remove(1);
                        w.active = 0;
                    }
                    2 => w.hide_tab_context_menu(cx),
                    3 => w.show_tab_context_menu(0, gpui::point(px(25.), px(25.)), cx),
                    4 => w.show_tab_context_menu(1, gpui::point(px(25.), px(25.)), cx),
                    _ => {
                        w.close_tab_submenu(cx);
                        w.open_tab_submenu(which, gpui::point(px(230.), px(20.)), cx);
                    }
                }
                w.set_status("newer status");
                w.focus_input(InputFocus::FindQuery, window, cx);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("clipboard sentinel".into()));
            });
            let before = visual.update(|_, cx| {
                w.read(cx)
                    .tabs
                    .iter()
                    .map(|tab| tab.session_id)
                    .collect::<Vec<_>>()
            });
            let current_popup =
                visual.update(|_, cx| w.read(cx).tab_context_menu.map(|state| state.popup_id));
            visual.simulate_click(gpui::point(px(30.), px(16.)), gpui::Modifiers::none());
            visual.run_until_parked();
            visual.update_window_entity(&w, |w, window, cx| {
                if transition == 0 {
                    let target_tab = w
                        .tabs
                        .iter()
                        .find(|tab| tab.session_id == target)
                        .expect("original target");
                    match which {
                        TabSubmenu::CopyToClipboard => assert_eq!(
                            clipboard(cx),
                            directory.join("neutral-1.txt").display().to_string()
                        ),
                        TabSubmenu::MoveDocument => assert_eq!(w.tabs[0].session_id, target),
                        TabSubmenu::ApplyColor => assert_eq!(target_tab.buffer.tab_color, Some(1)),
                        TabSubmenu::CloseMultiple => assert_eq!(
                            w.tabs.iter().map(|tab| tab.session_id).collect::<Vec<_>>(),
                            [target]
                        ),
                        _ => unreachable!(),
                    }
                    assert!(w.tab_context_menu.is_none());
                } else {
                    assert_eq!(
                        w.tabs.iter().map(|tab| tab.session_id).collect::<Vec<_>>(),
                        before
                    );
                    assert!(w.tabs.iter().all(|tab| tab.buffer.tab_color.is_none()));
                    assert_eq!(clipboard(cx), "clipboard sentinel");
                    assert_eq!(w.status_message.as_deref(), Some("newer status"));
                    assert_eq!(
                        w.tab_context_menu.map(|state| state.popup_id),
                        if transition == 1 { None } else { current_popup }
                    );
                }
                assert!(w.find_focus.is_focused(window));
            });
            cx.quit();
        }
    }
}

#[test]
fn tab_popup_old_hover_and_backdrop_callbacks_preserve_a_newer_popup() {
    for callback in 0..3 {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("painted-tab-hover");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        let state = w.read_with(&cx, |w, _| w.tab_context_menu.expect("old popup"));
        let flags = w.read_with(&cx, |w, _| flags_for(w, 1).expect("flags"));
        let (_, visual) = cx.add_window_view(|_, _| PaintedMenu {
            workspace: w.clone(),
            state,
            flags,
            panel: if callback == 2 {
                Panel::Backdrop
            } else {
                Panel::Main
            },
        });
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        visual.update_window_entity(&w, |w, window, cx| {
            w.show_tab_context_menu(0, gpui::point(px(35.), px(35.)), cx);
            w.open_tab_submenu(TabSubmenu::ApplyColor, gpui::point(px(260.), px(35.)), cx);
            w.set_status("newer status");
            w.focus_input(InputFocus::FindQuery, window, cx);
        });
        let new_popup = visual.update(|_, cx| w.read(cx).tab_context_menu.expect("new popup"));
        if callback == 2 {
            visual.simulate_click(gpui::point(px(600.), px(400.)), gpui::Modifiers::none());
        } else {
            visual.simulate_mouse_move(
                gpui::point(px(30.), px(if callback == 0 { 16. } else { 38. })),
                None,
                gpui::Modifiers::none(),
            );
        }
        visual.update_window_entity(&w, |w, window, _| {
            let current = w.tab_context_menu.expect("newer popup survives");
            assert_eq!(current.popup_id, new_popup.popup_id);
            assert!(current.open_submenu == new_popup.open_submenu);
            assert_eq!(current.submenu_anchor, new_popup.submenu_anchor);
            assert_eq!(w.status_message.as_deref(), Some("newer status"));
            assert!(w.find_focus.is_focused(window));
        });
        cx.quit();
    }
}

#[test]
fn tab_popup_rename_bar_tracks_original_document_after_movement() {
    let _settings = override_for_test(Settings::default());
    let directory = crate::persist::TempDir::new("rename-moved-target");
    let original = directory.join("neutral-1.txt");
    let neighbor = directory.join("neutral-2.txt");
    let renamed = directory.join("renamed café中😀.txt");
    std::fs::write(&original, "original disk\r\n").expect("original disk");
    std::fs::write(&neighbor, "neighbor disk\r\n").expect("neighbor disk");
    let mut cx = TestAppContext::single();
    let w = workspace(&mut cx, directory.path());
    let visual = cx.add_empty_window();
    visual.update_window_entity(&w, |w, window, cx| {
        w.open_tab_rename(1, cx);
        w.tab_rename.as_mut().expect("rename prompt").text = "renamed café中😀.txt".into();
        w.move_tab(1, 2, cx);
        w.focus_input(InputFocus::TabRename, window, cx);
        w.insert_newline(&crate::actions::EditInsertNewline, window, cx);
        assert!(w.tab_rename.is_none());
        assert_eq!(w.tabs[2].buffer.file_path.as_ref(), Some(&renamed));
        assert_eq!(w.tabs[1].buffer.file_path.as_ref(), Some(&neighbor));
        assert_eq!(w.tabs[2].buffer.rope.to_string(), "source-1");
        assert!(w.tabs[2].undo_stack.is_empty());
        assert!(w.focus_handle.is_focused(window));
    });
    assert!(!original.exists());
    assert_eq!(
        std::fs::read_to_string(&renamed).expect("renamed original"),
        "original disk\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(&neighbor).expect("neighbor survives"),
        "neighbor disk\r\n"
    );
    cx.quit();
}

#[test]
fn tab_popup_rename_refuses_closed_or_path_changed_targets_and_existing_destinations() {
    for change in 0..2 {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("rename-refusal");
        let original = directory.join("neutral-1.txt");
        let other = directory.join("neutral-2.txt");
        let renamed = directory.join("new-name.txt");
        std::fs::write(&original, "original disk").expect("original disk");
        std::fs::write(&other, "other disk").expect("other disk");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        let visual = cx.add_empty_window();
        visual.update_window_entity(&w, |w, window, cx| {
            w.open_tab_rename(1, cx);
            w.tab_rename.as_mut().expect("rename prompt").text = "new-name.txt".into();
            match change {
                0 => {
                    w.tabs.remove(1);
                    w.active = 0;
                }
                1 => w.tabs[1].buffer.file_path = Some(other.clone()),
                _ => {}
            }
            w.focus_input(InputFocus::FindQuery, window, cx);
            w.commit_tab_rename(cx);
            assert!(w.tab_rename.is_none());
            assert!(w.find_focus.is_focused(window));
            assert!(w.tabs.iter().all(|tab| tab.undo_stack.is_empty()));
        });
        assert_eq!(
            std::fs::read_to_string(&original).expect("original survives"),
            "original disk"
        );
        assert_eq!(
            std::fs::read_to_string(&other).expect("other survives"),
            "other disk"
        );
        assert!(!renamed.exists());
        cx.quit();
    }
}

#[test]
fn tab_popup_rename_collision_preserves_both_files() {
    let _settings = override_for_test(Settings::default());
    let directory = crate::persist::TempDir::new("rename-collision");
    let original = directory.join("neutral-1.txt");
    let other = directory.join("neutral-2.txt");
    std::fs::write(&original, "original disk").expect("original disk");
    std::fs::write(&other, "other disk").expect("other disk");
    let mut cx = TestAppContext::single();
    let w = workspace(&mut cx, directory.path());
    let visual = cx.add_empty_window();
    visual.update_window_entity(&w, |w, _, cx| {
        w.open_tab_rename(1, cx);
        w.tab_rename.as_mut().expect("prompt").text = "neutral-2.txt".into();
        w.commit_tab_rename(cx);
        assert_eq!(w.tabs[1].buffer.file_path.as_ref(), Some(&original));
        assert_eq!(w.tabs[1].buffer.rope.to_string(), "source-1");
        assert!(w.tabs[1].undo_stack.is_empty());
    });
    assert_eq!(
        std::fs::read_to_string(&original).expect("original survives"),
        "original disk"
    );
    assert_eq!(
        std::fs::read_to_string(&other).expect("other survives"),
        "other disk"
    );
    cx.quit();
}

struct PaintedRename {
    workspace: Entity<Workspace>,
    state: crate::find_state::TabRenameState,
}
impl Render for PaintedRename {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::tab_rename_bar::tab_rename_bar(
            &Theme::for_mode(crate::theme::ThemeMode::Dark),
            &self.state,
            self.workspace.clone(),
        )
    }
}

#[test]
fn tab_popup_old_rename_close_and_field_callbacks_preserve_newer_prompts_and_find() {
    for close in [false, true] {
        for newer_prompt in [false, true] {
            let _settings = override_for_test(Settings::default());
            let directory = crate::persist::TempDir::new("painted-rename-stale");
            let mut cx = TestAppContext::single();
            let w = workspace(&mut cx, directory.path());
            w.update(&mut cx, |w, cx| w.open_tab_rename(1, cx));
            let state = w.read_with(&cx, |w, _| {
                w.tab_rename.as_ref().expect("old prompt").clone()
            });
            let (_, visual) = cx.add_window_view(|_, _| PaintedRename {
                workspace: w.clone(),
                state,
            });
            visual.update(|window, _| window.activate_window());
            visual.run_until_parked();
            let input = visual
                .debug_bounds("tab-rename-input")
                .expect("painted field");
            visual.update_window_entity(&w, |w, window, cx| {
                w.close_tab_rename(cx);
                if newer_prompt {
                    w.open_tab_rename(0, cx);
                    let state = w.tab_rename.as_mut().expect("new prompt");
                    state.text = "new draft".into();
                    state.text_cursor = 5;
                    state.text_anchor = 2;
                }
                w.find_state.open = true;
                w.focus_input(InputFocus::FindQuery, window, cx);
                w.set_status("newer status");
            });
            let click = if close {
                gpui::point(input.right() + px(12.), input.center().y)
            } else {
                gpui::point(input.left() + px(9.), input.center().y)
            };
            visual.simulate_click(click, gpui::Modifiers::none());
            visual.update_window_entity(&w, |w, window, _| {
                assert_eq!(w.tab_rename.is_some(), newer_prompt);
                if let Some(state) = &w.tab_rename {
                    assert_eq!(state.text, "new draft");
                    assert_eq!((state.text_cursor, state.text_anchor), (5, 2));
                }
                assert!(w.find_state.open);
                assert!(w.find_focus.is_focused(window));
                assert_eq!(w.status_message.as_deref(), Some("newer status"));
            });
            cx.quit();
        }
    }
}

#[test]
fn tab_popup_submenu_generation_changes_only_on_open_switch_or_reopen() {
    let directory = crate::persist::TempDir::new("submenu-generation");
    let mut cx = TestAppContext::single();
    let w = workspace(&mut cx, directory.path());
    w.update(&mut cx, |w, cx| {
        let anchor = gpui::point(px(230.), px(20.));
        w.open_tab_submenu(TabSubmenu::CopyToClipboard, anchor, cx);
        let first = w
            .tab_context_menu
            .expect("menu")
            .submenu_id
            .expect("submenu id");
        w.open_tab_submenu(TabSubmenu::CopyToClipboard, anchor, cx);
        assert_eq!(w.tab_context_menu.expect("menu").submenu_id, Some(first));
        w.open_tab_submenu(
            TabSubmenu::CopyToClipboard,
            gpui::point(px(240.), px(20.)),
            cx,
        );
        assert_eq!(w.tab_context_menu.expect("menu").submenu_id, Some(first));
        w.open_tab_submenu(TabSubmenu::ApplyColor, anchor, cx);
        let second = w
            .tab_context_menu
            .expect("menu")
            .submenu_id
            .expect("submenu id");
        assert_ne!(first, second);
        w.close_tab_submenu(cx);
        assert!(w.tab_context_menu.expect("menu").submenu_id.is_none());
        w.open_tab_submenu(TabSubmenu::ApplyColor, anchor, cx);
        assert_ne!(w.tab_context_menu.expect("menu").submenu_id, Some(second));
    });
    cx.quit();
}

#[test]
fn tab_popup_current_rename_buttons_work_without_dismissing_find_or_newer_focus() {
    for own_focus in [false, true] {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("painted-rename-current");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        w.update(&mut cx, |w, cx| w.open_tab_rename(1, cx));
        let state = w.read_with(&cx, |w, _| w.tab_rename.as_ref().expect("prompt").clone());
        let (_, visual) = cx.add_window_view(|_, _| PaintedRename {
            workspace: w.clone(),
            state,
        });
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        let input = visual
            .debug_bounds("tab-rename-input")
            .expect("painted field");
        visual.simulate_click(
            gpui::point(input.left() + px(9.), input.center().y),
            gpui::Modifiers::none(),
        );
        visual.update_window_entity(&w, |w, window, cx| {
            assert!(w
                .tab_rename_focus
                .as_ref()
                .expect("rename focus")
                .is_focused(window));
            let state = w.tab_rename.as_ref().expect("prompt");
            assert_eq!((state.text_cursor, state.text_anchor), (0, 0));
            w.find_state.open = true;
            if !own_focus {
                w.focus_input(InputFocus::FindQuery, window, cx);
            }
        });
        visual.simulate_click(
            gpui::point(input.right() + px(12.), input.center().y),
            gpui::Modifiers::none(),
        );
        visual.update_window_entity(&w, |w, window, _| {
            assert!(w.tab_rename.is_none(), "painted close button must fire");
            assert!(w.find_state.open);
            assert!(if own_focus {
                w.focus_handle.is_focused(window)
            } else {
                w.find_focus.is_focused(window)
            });
        });
        cx.quit();
    }
}

#[test]
fn tab_popup_rename_rejects_null_paths_without_truncating_native_arguments() {
    for source_null in [false, true] {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("rename-null-path");
        let original = directory.join("neutral-1.txt");
        std::fs::write(&original, "source café中😀\r\n").expect("original disk");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        w.update(&mut cx, |w, cx| {
            if source_null {
                w.tabs[1].buffer.file_path = Some(std::path::PathBuf::from(format!(
                    "{}\0suffix",
                    original.display()
                )));
            }
            w.open_tab_rename(1, cx);
            w.tab_rename.as_mut().expect("prompt").text = if source_null {
                "new-name.txt"
            } else {
                "new-name.txt\0suffix"
            }
            .into();
            w.commit_tab_rename(cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("Rename failed — file path contains a null character")
            );
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "source-1");
            assert!(w.tabs[1].undo_stack.is_empty());
        });
        assert_eq!(
            std::fs::read_to_string(&original).expect("original survives"),
            "source café中😀\r\n"
        );
        assert!(!directory.join("new-name.txt").exists());
        cx.quit();
    }
}

#[test]
fn tab_popup_retained_rename_protocol_cannot_edit_a_replacement_prompt() {
    use gpui::InputHandler;
    let _settings = override_for_test(Settings::default());
    let directory = crate::persist::TempDir::new("rename-retained-protocol");
    let mut cx = TestAppContext::single();
    let w = workspace(&mut cx, directory.path());
    w.update(&mut cx, |w, cx| w.open_tab_rename(1, cx));
    let state = w.read_with(&cx, |w, _| w.tab_rename.as_ref().expect("prompt").clone());
    let (_, visual) = cx.add_window_view(|_, _| PaintedRename {
        workspace: w.clone(),
        state,
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    let bounds = visual.debug_bounds("tab-rename-input").expect("field");
    let (focus, prompt_id) = visual.update(|_, cx| {
        let w = w.read(cx);
        (
            w.tab_rename_focus.clone().expect("focus"),
            w.tab_rename.as_ref().expect("prompt").prompt_id,
        )
    });
    let mut retained = crate::input_field::rename_input::RenameInputHandler::new(
        bounds,
        w.clone(),
        prompt_id,
        focus,
    );
    visual.update_window_entity(&w, |w, window, cx| {
        w.open_tab_rename(0, cx);
        let state = w.tab_rename.as_mut().expect("replacement prompt");
        state.text = "new draft".into();
        state.text_cursor = 5;
        state.text_anchor = 2;
        w.focus_input(InputFocus::TabRename, window, cx);
    });
    visual.update(|window, cx| {
        retained.replace_and_mark_text_in_range(Some(0..3), "delayed中", Some(0..1), window, cx);
        retained.replace_text_in_range(Some(0..2), "late😀", window, cx);
        retained.unmark_text(window, cx);
        let state = w.read(cx).tab_rename.as_ref().expect("replacement remains");
        assert_eq!(state.text, "new draft");
        assert_eq!((state.text_cursor, state.text_anchor), (5, 2));
    });
    cx.quit();
}

#[test]
fn tab_popup_retired_rename_target_field_does_not_take_newer_find_focus() {
    for removed in [false, true] {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("rename-retired-field");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        w.update(&mut cx, |w, cx| w.open_tab_rename(1, cx));
        let state = w.read_with(&cx, |w, _| w.tab_rename.as_ref().expect("prompt").clone());
        let (_, visual) = cx.add_window_view(|_, _| PaintedRename {
            workspace: w.clone(),
            state,
        });
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        let input = visual.debug_bounds("tab-rename-input").expect("field");
        visual.update_window_entity(&w, |w, window, cx| {
            if removed {
                w.tabs.remove(1);
                w.active = 0;
            } else {
                w.tabs[1].buffer.file_path = Some(directory.join("changed-path.txt"));
            }
            w.find_state.open = true;
            w.focus_input(InputFocus::FindQuery, window, cx);
            w.set_status("newer status");
        });
        visual.simulate_click(
            gpui::point(input.left() + px(9.), input.center().y),
            gpui::Modifiers::none(),
        );
        visual.update_window_entity(&w, |w, window, _| {
            assert!(w.find_focus.is_focused(window));
            assert_eq!(w.status_message.as_deref(), Some("newer status"));
            assert_eq!(w.tab_rename.as_ref().expect("prompt").text, "neutral-1.txt");
        });
        cx.quit();
    }
}

#[test]
fn tab_popup_rename_protocol_rejects_retired_focus_closed_targets_and_inactive_windows() {
    use gpui::InputHandler;
    for transition in 0..6 {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("rename-protocol-context");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        w.update(&mut cx, |w, cx| w.open_tab_rename(1, cx));
        let state = w.read_with(&cx, |w, _| w.tab_rename.as_ref().expect("prompt").clone());
        let prompt_id = state.prompt_id;
        let (_, visual) = cx.add_window_view(|_, _| PaintedRename {
            workspace: w.clone(),
            state,
        });
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        let bounds = visual.debug_bounds("tab-rename-input").expect("field");
        let focus = visual.update_window_entity(&w, |w, window, cx| {
            w.focus_input(InputFocus::TabRename, window, cx);
            w.tab_rename_focus.clone().expect("focus")
        });
        let mut retained = crate::input_field::rename_input::RenameInputHandler::new(
            bounds,
            w.clone(),
            prompt_id,
            focus,
        );
        visual.update_window_entity(&w, |w, window, cx| {
            w.find_state.query = "newer find".into();
            w.find_state.query_cursor = 5;
            w.find_state.query_anchor = 2;
            match transition {
                0 => {
                    w.find_state.open = true;
                    w.focus_input(InputFocus::FindQuery, window, cx);
                }
                1 => {
                    w.tab_rename = None;
                }
                2 => {
                    w.tabs.remove(1);
                    w.active = 0;
                }
                3 => {
                    w.tabs[1].buffer.file_path = Some(directory.join("changed-path.txt"));
                }
                4 => {
                    w.tab_rename_focus = Some(cx.focus_handle());
                }
                _ => {}
            }
            w.set_status("newer status");
        });
        if transition == 5 {
            visual.deactivate_window();
        }
        visual.update(|window, cx| {
            let before = {
                let w = w.read(cx);
                (
                    w.tab_rename
                        .as_ref()
                        .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor)),
                    w.tabs
                        .iter()
                        .map(|t| t.buffer.rope.to_string())
                        .collect::<Vec<_>>(),
                )
            };
            assert!(
                !retained.accepts_text_input(window, cx),
                "context {transition}"
            );
            assert!(retained.selected_text_range(true, window, cx).is_none());
            assert!(retained.marked_text_range(window, cx).is_none());
            let mut adjusted = None;
            assert!(retained
                .text_for_range(0..3, &mut adjusted, window, cx)
                .is_none());
            assert!(adjusted.is_none());
            assert!(retained.bounds_for_range(0..3, window, cx).is_none());
            assert!(retained
                .character_index_for_point(bounds.center(), window, cx)
                .is_none());
            retained.replace_and_mark_text_in_range(Some(0..3), "delayed中", None, window, cx);
            retained.replace_text_in_range(Some(0..2), "late😀", window, cx);
            retained.unmark_text(window, cx);
            let w = w.read(cx);
            assert_eq!(
                w.tab_rename
                    .as_ref()
                    .map(|s| (s.text.clone(), s.text_cursor, s.text_anchor)),
                before.0
            );
            assert_eq!(
                w.tabs
                    .iter()
                    .map(|t| t.buffer.rope.to_string())
                    .collect::<Vec<_>>(),
                before.1
            );
            assert_eq!(w.find_state.query, "newer find");
            assert_eq!(
                (w.find_state.query_cursor, w.find_state.query_anchor),
                (5, 2)
            );
            assert_eq!(w.status_message.as_deref(), Some("newer status"));
            if transition == 0 {
                assert!(w.find_focus.is_focused(window));
            }
        });
        cx.quit();
    }
}

#[test]
fn tab_popup_current_rename_protocol_forwards_unicode_utf16_after_target_moves() {
    use gpui::InputHandler;
    let _settings = override_for_test(Settings::default());
    let directory = crate::persist::TempDir::new("rename-protocol-unicode");
    let mut cx = TestAppContext::single();
    let w = workspace(&mut cx, directory.path());
    w.update(&mut cx, |w, cx| w.open_tab_rename(1, cx));
    let state = w.read_with(&cx, |w, _| w.tab_rename.as_ref().expect("prompt").clone());
    let prompt_id = state.prompt_id;
    let (_, visual) = cx.add_window_view(|_, _| PaintedRename {
        workspace: w.clone(),
        state,
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    let bounds = visual.debug_bounds("tab-rename-input").expect("field");
    let focus = visual.update_window_entity(&w, |w, window, cx| {
        w.move_tab(1, 2, cx);
        let state = w.tab_rename.as_mut().expect("prompt");
        state.text = "a😀中c".into();
        state.text_cursor = 5;
        state.text_anchor = 1;
        w.focus_input(InputFocus::TabRename, window, cx);
        w.tab_rename_focus.clone().expect("focus")
    });
    let mut retained = crate::input_field::rename_input::RenameInputHandler::new(
        bounds,
        w.clone(),
        prompt_id,
        focus,
    );
    visual.update(|window, cx| {
        assert!(retained.accepts_text_input(window, cx));
        assert_eq!(
            retained
                .selected_text_range(false, window, cx)
                .expect("selection")
                .range,
            1..3
        );
        let mut adjusted = None;
        assert_eq!(
            retained
                .text_for_range(1..4, &mut adjusted, window, cx)
                .as_deref(),
            Some("😀中")
        );
        assert_eq!(adjusted, Some(1..4));
        assert_eq!(retained.bounds_for_range(1..3, window, cx), Some(bounds));
        retained.replace_and_mark_text_in_range(Some(1..3), "éĀ", Some(0..1), window, cx);
        retained.unmark_text(window, cx);
        retained.replace_text_in_range(Some(3..4), "😀", window, cx);
        let w = w.read(cx);
        let state = w.tab_rename.as_ref().expect("prompt");
        assert_eq!(state.text, "aéĀ😀c");
        assert_eq!((state.text_cursor, state.text_anchor), (9, 9));
        assert!(w.tabs.iter().all(|tab| tab.undo_stack.is_empty()));
        assert_eq!(w.tabs[2].buffer.rope.to_string(), "source-1");
    });
    cx.quit();
}

#[test]
fn tab_popup_rename_keeps_encoded_disk_bytes_unsaved_text_and_history() {
    use crate::text::{
        encoding::{self, Encoding},
        line_ending::LineEnding,
    };
    use gpui::EntityInputHandler;
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le] {
        let _settings = override_for_test(Settings::default());
        let directory = crate::persist::TempDir::new("rename-text-fidelity");
        let original = directory.join("neutral-1.txt");
        let renamed = directory.join("renamed-é中😀.txt");
        let disk = encoding::encode("café中😀\r\nsecond\r\n", encoding);
        std::fs::write(&original, &disk).expect("neutral disk");
        let mut cx = TestAppContext::single();
        let w = workspace(&mut cx, directory.path());
        w.update(&mut cx, |w, _| {
            w.tabs[1].buffer = crate::io::read_buffer(&original).expect("loaded")
        });
        let (_, visual) = cx.add_window_view(|_, _| gpui::Empty);
        visual.update(|window, _| window.activate_window());
        visual.update_window_entity(&w, |w, window, cx| {
            w.focus_input(InputFocus::Editor, window, cx);
            w.replace_text_in_range(Some(0..0), "unsavedĀ", window, cx);
            let source = w.tabs[1].buffer.rope.to_string();
            let history = crate::buffer::serialize_undo_stack(&w.tabs[1].undo_stack);
            let revision = w.tabs[1].buffer.raw_revision.clone();
            let selection = w.tabs[1].selection;
            assert_eq!(w.tabs[1].undo_stack.len(), 1);
            assert!(w.tabs[1].buffer.dirty);
            w.open_tab_rename(1, cx);
            w.tab_rename.as_mut().expect("prompt").text = "renamed-é中😀.txt".into();
            w.focus_input(InputFocus::TabRename, window, cx);
            w.insert_newline(&crate::actions::EditInsertNewline, window, cx);
            let tab = &w.tabs[1];
            assert_eq!(tab.buffer.file_path.as_ref(), Some(&renamed));
            assert_eq!(tab.buffer.rope.to_string(), source);
            assert_eq!(
                crate::buffer::serialize_undo_stack(&tab.undo_stack),
                history
            );
            assert_eq!(tab.buffer.raw_revision, revision);
            assert_eq!(
                (
                    tab.selection.anchor,
                    tab.selection.head,
                    tab.selection.is_dragging
                ),
                (selection.anchor, selection.head, selection.is_dragging)
            );
            assert!(tab.buffer.dirty);
            assert_eq!(tab.buffer.encoding, encoding);
            assert_eq!(tab.buffer.line_ending, LineEnding::Crlf);
        });
        assert!(!original.exists());
        assert_eq!(std::fs::read(&renamed).expect("renamed disk"), disk);
        cx.quit();
    }
}
