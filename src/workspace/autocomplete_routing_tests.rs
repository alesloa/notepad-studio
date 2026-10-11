// Actual Workspace rendering and CATALOG key routing on GPUI's TestWindow.
// Fixtures never open user files, save settings, or schedule recovery writes.
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

use super::Workspace;
use crate::{actions::*, settings::Settings, tab::TabViewMode};

const SAMPLE: &str = "al\nalpha albatross alpine";

fn press(visual: &mut VisualTestContext, actions: &[&str]) {
    for action in actions {
        let entry = crate::shortcuts::catalog()
            .iter()
            .find(|entry| entry.id == *action)
            .unwrap();
        let chord = crate::shortcuts::platform_default_keys(entry)
            .first()
            .unwrap();
        visual.simulate_keystrokes(chord);
    }
}

fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (workspace, visual) = cx.add_window_view(|_, cx| {
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
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(SAMPLE);
        w.active_tab_mut().selection.collapse_to(2);
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        let focus = workspace.read(cx).focus_handle.clone();
        window.focus(&focus, cx);
    });
    visual.run_until_parked();
    press(visual, &["EditAutocompleteWord"]); // CATALOG Word Completion.
    assert!(visual.debug_bounds("autocomplete-menu").is_some());
    (workspace, visual)
}

fn preferences() -> Settings {
    Settings {
        toolbar_hidden: true,
        folder_panel_visible: false,
        ..Settings::default()
    }
}

#[test]
fn real_workspace_keys_navigate_accept_and_cancel_completion() {
    let _settings = crate::settings::override_for_test(preferences());
    for accept in ["enter", "tab"] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.simulate_keystrokes("down up up down down");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.selection().head, 2);
            assert_eq!(w.autocomplete.as_ref().unwrap().selected, 1);
        });
        visual.simulate_keystrokes(accept);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "alpha\nalpha albatross alpine");
            assert!(w.autocomplete.is_none());
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        press(visual, &["EditUndo", "EditAutocompleteWord"]);
        visual.simulate_keystrokes("escape");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), SAMPLE);
            assert!(w.autocomplete.is_none());
        });
        cx.quit();
    }
}

#[test]
fn disabled_acceptance_keys_run_their_normal_editor_actions() {
    for (key, expected) in [
        ("enter", "al\n\nalpha albatross alpine"),
        ("tab", "al\t\nalpha albatross alpine"),
    ] {
        let _settings = crate::settings::override_for_test(Settings {
            autocompletion_insert_with_enter: false,
            autocompletion_insert_with_tab: false,
            indent_uses_tabs: true,
            ..preferences()
        });
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.simulate_keystrokes(key);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), expected);
            assert!(w.autocomplete.is_none());
        });
        cx.quit();
    }
}

#[test]
fn completion_cannot_open_or_accept_in_another_view() {
    let _settings = crate::settings::override_for_test(preferences());
    for mode in [
        TabViewMode::MarkdownPreview,
        TabViewMode::CsvPreview,
        TabViewMode::RichDocView,
    ] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, window, cx| {
            w.active_tab_mut().view_mode = mode;
            assert!(!w.accept_autocomplete_with("alpha", window, cx));
            assert_eq!(w.buffer().rope.to_string(), SAMPLE);
            w.autocomplete_word(&EditAutocompleteWord, window, cx);
            assert!(w.autocomplete.is_none());
        });
        cx.quit();
    }
}

#[test]
fn preview_setters_cancel_immediately_before_the_next_frame() {
    let _settings = crate::settings::override_for_test(preferences());
    for extension in ["md", "csv", "rtf"] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.active_tab_mut().buffer.file_path = Some(format!("sample.{extension}").into());
            match extension {
                "md" => w.set_markdown_view_mode(true, cx),
                "csv" => w.set_csv_view_mode(true, cx),
                _ => {
                    w.active_tab_mut().buffer.rich_doc = Some(
                        crate::rich_doc::rtf::open_bytes(b"{\\rtf1\\ansi alpha}".to_vec()).unwrap(),
                    );
                    w.set_rich_doc_view_mode(true, cx);
                }
            }
            assert!(
                w.autocomplete.is_none(),
                "{extension} transition cancels synchronously"
            );
            assert_eq!(w.buffer().rope.to_string(), SAMPLE);
        });
        cx.quit();
    }
}

#[test]
fn a_painted_row_cannot_accept_a_reopened_completion() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    let old_bounds = visual.debug_bounds("autocomplete-menu").unwrap();
    visual.update_window_entity(&workspace, |w, _, _| {
        // Model an identical new opening before its first paint. Omit notify:
        // GPUI's test app otherwise redraws at the end of every update.
        w.autocomplete.as_mut().unwrap().session_id = uuid::Uuid::new_v4();
    });
    visual.simulate_click(
        old_bounds.origin + gpui::point(gpui::px(12.), gpui::px(12.)),
        gpui::Modifiers::none(),
    );
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.buffer().rope.to_string(), SAMPLE);
        assert!(w.autocomplete.is_some());
    });
    cx.quit();
}

#[test]
fn a_painted_wheel_callback_cannot_select_a_reopened_completion() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    let old_bounds = visual.debug_bounds("autocomplete-menu").unwrap();
    visual.update_window_entity(&workspace, |w, _, _| {
        w.autocomplete.as_mut().unwrap().session_id = uuid::Uuid::new_v4();
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: old_bounds.origin + gpui::point(gpui::px(12.), gpui::px(12.)),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -1.)),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.autocomplete.as_ref().unwrap().selected, 0);
        assert_eq!(w.buffer().rope.to_string(), SAMPLE);
    });
    cx.quit();
}

#[test]
fn window_deactivation_cancels_before_a_rendered_blur() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let callback_observed = observed.clone();
    let _subscription = visual.update_window_entity(&workspace, |_, window, cx| {
        cx.observe_window_activation(window, move |w, window, _| {
            if !window.is_window_active() {
                callback_observed.set(true);
                assert!(
                    w.autocomplete.is_none(),
                    "dismiss at deactivation, before the next frame"
                );
                assert!(!w.autocomplete_context_valid(window));
                assert_eq!(w.buffer().rope.to_string(), SAMPLE);
            }
        })
    });
    visual.deactivate_window();
    assert!(observed.get());
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.update(|_, cx| assert!(workspace.read(cx).autocomplete.is_none()));
    press(visual, &["EditAutocompleteWord"]);
    assert!(visual.debug_bounds("autocomplete-menu").is_some());
    cx.quit();
}

#[test]
fn nested_test_preferences_restore_the_previous_override() {
    let _outer = crate::settings::override_for_test(Settings {
        autocompletion_insert_with_enter: false,
        ..preferences()
    });
    {
        let _inner = crate::settings::override_for_test(preferences());
        assert!(crate::settings::snapshot().autocompletion_insert_with_enter);
    }
    assert!(!crate::settings::snapshot().autocompletion_insert_with_enter);
}

#[test]
fn stale_session_dismiss_and_accept_leave_a_new_opening_untouched() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx);
    visual.update_window_entity(&workspace, |w, window, cx| {
        let old = w.autocomplete.as_ref().unwrap().session_id;
        w.autocomplete_word(&EditAutocompleteWord, window, cx);
        let current = w.autocomplete.as_ref().unwrap().session_id;
        assert_ne!(old, current);
        w.dismiss_autocomplete_session(old, cx);
        assert!(!w.accept_autocomplete_session(old, "unlisted", window, cx));
        assert!(!w.accept_autocomplete_session(old, "alpha", window, cx));
        w.scroll_autocomplete_session(old, 1, window, cx);
        assert_eq!(w.autocomplete.as_ref().unwrap().session_id, current);
        assert_eq!(w.autocomplete.as_ref().unwrap().selected, 0);
        assert_eq!(w.buffer().rope.to_string(), SAMPLE);
    });
    cx.quit();
}

#[test]
fn source_view_actions_return_body_and_grid_focus_to_the_editor() {
    let _settings = crate::settings::override_for_test(preferences());
    for route in 0..4 {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, _, _| {
            let tab = w.active_tab_mut();
            tab.buffer.file_path = Some(
                if route == 0 {
                    "sample.csv"
                } else {
                    "sample.rtf"
                }
                .into(),
            );
            if route != 0 {
                tab.buffer.rich_doc = Some(
                    crate::rich_doc::rtf::open_bytes(b"{\\rtf1\\ansi alpha}".to_vec()).unwrap(),
                );
            }
        });
        if route == 0 {
            visual.dispatch_action(ViewCsvPreview);
        } else {
            visual.dispatch_action(ViewPreview);
        }
        visual.run_until_parked();
        visual.update_window_entity(&workspace, |w, window, cx| {
            let focus = if route == 0 {
                w.csv_models
                    .get(&w.tabs[w.active].session_id)
                    .unwrap()
                    .grid_focus
                    .clone()
                    .unwrap()
            } else {
                w.rich_doc_body_focus.clone().unwrap()
            };
            window.focus(&focus, cx);
        });
        visual.run_until_parked();
        match route {
            0 => visual.dispatch_action(ViewCsvSource),
            1 => visual.dispatch_action(ViewSource),
            2 => visual.dispatch_action(crate::actions::rich_doc::RichDocRaw),
            _ => {
                visual.update_window_entity(&workspace, |w, window, cx| {
                    w.goto_state.open = true;
                    window.focus(&w.goto_focus, cx);
                    cx.notify();
                });
                visual.run_until_parked();
                // InputFieldElement's generic action routing is a separate
                // concern. Exercise the Source handler while that visible
                // field owns focus, without transferring it to the editor.
                visual.update_window_entity(&workspace, |w, window, cx| {
                    w.view_source(&ViewSource, window, cx);
                });
            }
        }
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert_eq!(
                w.tabs[w.active].view_mode,
                TabViewMode::Edit,
                "Source action route {route}"
            );
            if route == 3 {
                assert!(
                    w.goto_focus.is_focused(window),
                    "keep an unrelated visible input focused"
                );
            } else {
                assert!(
                    w.focus_handle.is_focused(window),
                    "Source owns keyboard editing, route {route}"
                );
            }
            assert!(w.autocomplete.is_none());
        });
        if route != 3 {
            press(visual, &["EditAutocompleteWord"]);
            assert!(visual.debug_bounds("autocomplete-menu").is_some());
        }
        cx.quit();
    }
}
