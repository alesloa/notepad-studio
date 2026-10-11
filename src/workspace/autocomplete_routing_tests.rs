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
fn completion_actual_pane_edges_and_zoom_keep_the_caret_line_clear() {
    let names = "alpha albatross alpine";
    let right = format!("{}al\n{names}", "word ".repeat(40));
    let wrapped = format!("{}al\n{names}", "word ".repeat(200));
    let bottom = format!("{names}{}al", "\n".repeat(80));
    let mut placed = 0;
    let mut dismissed = 0;
    for rem in [8., 16., 32., 48.] {
        for (width, height) in [(800., 480.), (600., 240.)] {
            for (source, caret, wrap) in [
                (format!("al\n{names}"), 2, false),
                (right.clone(), right.find('\n').unwrap(), false),
                (wrapped.clone(), wrapped.find('\n').unwrap(), true),
                (bottom.clone(), bottom.len(), false),
            ] {
                let _settings = crate::settings::override_for_test(preferences());
                let mut cx = TestAppContext::single();
                let (workspace, visual) = fixture(&mut cx);
                visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                visual.update_window_entity(&workspace, |w, window, cx| {
                    w.close_autocomplete(cx);
                    w.window_closed = false;
                    w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
                    w.session_final_save_result = Some(true);
                    w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&source);
                    w.active_tab_mut().selection.collapse_to(caret);
                    w.word_wrap = wrap;
                    w.zoom_level = rem / 16.;
                    window.set_rem_size(gpui::px(rem));
                    cx.notify();
                });
                visual.run_until_parked();
                visual.update_window_entity(&workspace, |w, _, cx| w.ensure_cursor_visible(cx));
                visual.run_until_parked();
                if !wrap && source.starts_with("word ") {
                    visual.update_window_entity(&workspace, |w, _, cx| {
                        if let Some(caret_bounds) = w.autocomplete_caret_bounds() {
                            w.set_scroll_left(w.scroll_left() + caret_bounds.left() - w.viewport.bounds.right() + gpui::px(8.), cx);
                        }
                    });
                    visual.run_until_parked();
                }
                press(visual, &["EditAutocompleteWord"]);
                let painted_menu = visual.debug_bounds("autocomplete-menu");
                let placement = visual.update(|window, cx| {
                    let w = workspace.read(cx);
                    let placement = crate::ui::autocomplete_popup::occupied_bounds(w, window);
                    if height == 480. {
                        assert!(placement.is_some(), "roomy pane must retain completion: rem={rem} wrap={wrap}");
                    }
                    if let Some(menu) = placement {
                        let caret_bounds = w.autocomplete_caret_bounds().expect("a placed list needs a painted caret");
                        let pane = w.viewport.bounds.intersect(&gpui::Bounds::new(
                            gpui::Point::default(), window.viewport_size()));
                        assert!(!menu.intersects(&caret_bounds), "rem={rem} size={width}x{height} wrap={wrap}");
                        assert!(menu.bottom() <= caret_bounds.top() || menu.top() >= caret_bounds.bottom());
                        assert!(menu.left() >= pane.left() && menu.right() <= pane.right());
                        assert!(menu.top() >= pane.top() && menu.bottom() <= pane.bottom());
                        assert_eq!(painted_menu, Some(menu));
                    } else {
                        assert!(w.autocomplete.is_none(), "unplaceable menu must retire its input session");
                    }
                    assert_eq!(w.selection().head, caret);
                    assert_eq!(w.buffer().rope.to_string(), source);
                    placement
                });
                if placement.is_some() {
                    placed += 1;
                    press(visual, &["MoveDown", "EditPageDown", "EditPageUp"]);
                    visual.update(|window, cx| {
                        let w = workspace.read(cx);
                        assert_eq!(w.selection().head, caret, "list navigation must keep the editor caret");
                        assert_eq!(w.buffer().rope.to_string(), source);
                        let menu = crate::ui::autocomplete_popup::occupied_bounds(w, window).expect("navigable list");
                        assert!(!menu.intersects(&w.autocomplete_caret_bounds().unwrap()));
                        assert!(w.tabs[w.active].undo_stack.is_empty());
                    });
                    visual.simulate_keystrokes("escape");
                } else {
                    dismissed += 1;
                }
                cx.quit();
            }
        }
    }
    assert!(placed > 0);
    assert!(dismissed > 0, "a zoomed window with a zero-height editor must dismiss completion");
    assert_eq!(placed + dismissed, 32);
    println!("completion layout cases: {} placed, {} dismissed", placed, dismissed);
}

#[test]
fn real_workspace_keys_navigate_accept_and_cancel_completion() {
    let _settings = crate::settings::override_for_test(preferences());
    for accept in ["enter", "tab"] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx);
        visual.simulate_keystrokes("down up down");
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

// Read-only parity audit: ScintillaBase.cxx:141-183, AutoComplete.cxx:238-246.
fn autocomplete_current_list_key(action: &str, last: bool, expected: usize) {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
    if last { visual.simulate_keystrokes("down down"); }
    press(visual, &[action]);
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.selection().head, 2, "list navigation must retain the editor caret");
        let state = w.autocomplete.as_ref().expect("list navigation must retain the opening");
        assert_eq!(state.selected, expected, "list navigation must follow the oracle boundary");
        assert_eq!(w.buffer().rope.to_string(), SAMPLE);
    });
    assert!(visual.debug_bounds("autocomplete-menu").is_some()); cx.quit();
}

#[test] fn autocomplete_current_up_clamps_at_first_row() { autocomplete_current_list_key("MoveUp", false, 0); }
#[test] fn autocomplete_current_down_clamps_at_last_row() { autocomplete_current_list_key("MoveDown", true, 2); }
#[test] fn autocomplete_current_page_down_moves_the_list() { autocomplete_current_list_key("EditPageDown", false, 2); }
#[test] fn autocomplete_current_page_up_moves_the_list() { autocomplete_current_list_key("EditPageUp", true, 0); }
#[test] fn autocomplete_current_home_moves_to_first_row() { autocomplete_current_list_key("MoveLineStart", true, 0); }
#[test] fn autocomplete_current_end_moves_to_last_row() { autocomplete_current_list_key("MoveLineEnd", false, 2); }

#[test]
fn autocomplete_current_painted_backspace_and_caret_movement() {
    let _settings = crate::settings::override_for_test(preferences());
    for movement in ["MoveLeft", "MoveRight"] {
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
        visual.simulate_keystrokes("p");
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.selection().head, 3);
            assert_eq!(w.autocomplete.as_ref().unwrap().candidates, ["albatross", "alpha", "alpine"]);
        });
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        press(visual, &["EditBackspace"]);
        visual.update(|_, cx| {
            let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), SAMPLE);
            assert!(w.autocomplete.is_some());
        });
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        press(visual, &[movement]); visual.run_until_parked();
        visual.update(|_, cx| assert!(workspace.read(cx).autocomplete.is_none()));
        // GPUI Frame::clear retains debug_bounds entries; absence cannot be inferred from this cache.
        visual.update(|window, cx| assert!(crate::ui::autocomplete_popup::occupied_bounds(workspace.read(cx), window).is_none()));
        cx.quit();
    }
}

#[test]
fn autocomplete_current_no_popup_keeps_normal_home_end_page_movement() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
    visual.simulate_keystrokes("escape");
    press(visual, &["MoveLineStart"]);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, 0));
    press(visual, &["MoveLineEnd"]);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, 2));
    press(visual, &["EditPageDown"]);
    visual.update(|_, cx| { let w = workspace.read(cx); assert_ne!(w.selection().head, 2); assert!(w.autocomplete.is_none()); });
    press(visual, &["EditPageUp"]);
    visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), SAMPLE); assert!(w.autocomplete.is_none()); });
    cx.quit();
}

#[test]
fn autocomplete_current_extend_commands_keep_editor_selection_behavior() {
    let _settings = crate::settings::override_for_test(preferences());
    for action in ["SelectLineStart", "SelectLineEnd", "SelectPageUp", "SelectPageDown"] {
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
        press(visual, &[action]);
        visual.update(|_, cx| { let w = workspace.read(cx); assert!(w.autocomplete.is_none()); assert_eq!(w.selection().anchor, 2); assert_eq!(w.buffer().rope.to_string(), SAMPLE); });
        cx.quit();
    }
}

#[test]
fn autocomplete_current_one_and_empty_list_boundaries_are_safe() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
    visual.update_window_entity(&workspace, |w, window, cx| {
        w.close_autocomplete(cx); w.active_tab_mut().buffer.rope = ropey::Rope::from_str("al\nalpha");
        w.active_tab_mut().selection.collapse_to(2); w.autocomplete_word(&EditAutocompleteWord, window, cx);
    }); visual.run_until_parked();
    for action in ["MoveUp", "MoveDown", "MoveLineStart", "MoveLineEnd", "EditPageUp", "EditPageDown"] {
        press(visual, &[action]);
        visual.update(|_, cx| { let w = workspace.read(cx); assert_eq!(w.autocomplete.as_ref().unwrap().selected, 0); assert_eq!(w.selection().head, 2); });
    }
    visual.update_window_entity(&workspace, |w, _, cx| {
        w.autocomplete.as_mut().unwrap().selected = usize::MAX;
        w.autocomplete_nav(i32::MIN, cx); assert_eq!(w.autocomplete.as_ref().unwrap().selected, 0);
        w.autocomplete_nav(i32::MAX, cx); assert_eq!(w.autocomplete.as_ref().unwrap().selected, 0);
        w.autocomplete.as_mut().unwrap().candidates.clear();
        w.autocomplete_nav(i32::MIN, cx); w.autocomplete_nav(i32::MAX, cx);
    });
    visual.update(|window, cx| { let _ = window.draw(cx); }); visual.run_until_parked();
    visual.update(|_, cx| { let w = workspace.read(cx); assert!(w.autocomplete.is_none()); assert_eq!(w.buffer().rope.to_string(), "al\nalpha"); });
    cx.quit();
}

#[test]
fn autocomplete_current_page_step_uses_painted_small_pane_rows() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
    let names = (0..30).map(|i| format!("al{i:02}")).collect::<Vec<_>>().join(" ");
    visual.simulate_resize(gpui::size(gpui::px(600.), gpui::px(160.)));
    visual.update_window_entity(&workspace, |w, window, cx| {
        w.close_autocomplete(cx); w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&format!("al\n{names}"));
        w.active_tab_mut().selection.collapse_to(2); w.autocomplete_word(&EditAutocompleteWord, window, cx);
        assert_eq!(w.autocomplete.as_ref().unwrap().candidates.len(), 30);
    }); visual.run_until_parked();
    let menu = visual.debug_bounds("autocomplete-menu").unwrap();
    let rows = ((f32::from(menu.size.height) - 6.) / 22.).round() as usize;
    assert!((1..8).contains(&rows), "controlled small pane has fewer than eight painted rows: {rows}");
    press(visual, &["EditPageDown"]);
    visual.update(|_, cx| { let w=workspace.read(cx); assert_eq!(w.selection().head, 2); assert_eq!(w.autocomplete.as_ref().unwrap().selected, rows); });
    press(visual, &["MoveLineEnd", "EditPageUp"]);
    visual.update(|_, cx| { let w=workspace.read(cx); assert_eq!(w.selection().head, 2); assert_eq!(w.autocomplete.as_ref().unwrap().selected, 29-rows); });
    cx.quit();
}

#[test]
fn autocomplete_current_list_navigation_does_not_record_editor_macros() {
    let _settings = crate::settings::override_for_test(preferences());
    let _execution = crate::macros::override_execution_for_test(true);
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
    for action in ["MoveUp", "MoveDown", "EditPageDown", "EditPageUp", "MoveLineEnd", "MoveLineStart"] {
        press(visual, &[action]);
    }
    crate::macros::stop_recording();
    assert!(crate::macros::current().is_empty());
    visual.update(|_, cx| assert_eq!(workspace.read(cx).selection().head, 2));
    visual.simulate_keystrokes("escape");
    crate::macros::start_recording();
    press(visual, &["MoveLineStart"]);
    crate::macros::stop_recording();
    assert!(matches!(crate::macros::current().as_slice(), [crate::macros::MacroEvent::MoveLineStart]));
    cx.quit();
}

fn autocomplete_acceptance_manual_during_preedit(action: &str) {
    use gpui::{ElementInputHandler, InputHandler};
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx);
    visual.update_window_entity(&workspace, |w, _, cx| {
        w.close_autocomplete(cx); w.active_tab_mut().buffer.rope=ropey::Rope::from_str("ret\nreturn");
        w.active_tab_mut().buffer.language=Some(crate::syntax::LanguageId::Rust);
        w.active_tab_mut().selection.collapse_to(3); cx.notify();
    }); visual.run_until_parked();
    visual.update(|window, cx| {
        let mut handler=ElementInputHandler::new(window.bounds(), workspace.clone());
        handler.replace_and_mark_text_in_range(None, "u", Some(1..1), window, cx);
        assert_eq!(handler.marked_text_range(window, cx), Some(3..4));
        assert!(workspace.read(cx).autocomplete.is_none());
    });
    if action=="EditAutocompleteFuncAndWord" { visual.dispatch_action(EditAutocompleteFuncAndWord); visual.run_until_parked(); } else { press(visual, &[action]); }
    visual.update(|window, cx| {
        let w=workspace.read(cx); assert!(w.is_composing_text(window), "manual completion must preserve the marked session");
        assert_eq!(w.buffer().rope.to_string(), "retu\nreturn");
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1); assert!(w.tabs[w.active].redo_stack.is_empty());
        assert!(w.autocomplete.is_none(), "manual completion must not reopen over an active preedit: {action}");
    });
    visual.update(|window, cx| {
        let mut handler=ElementInputHandler::new(window.bounds(), workspace.clone());
        assert_eq!(handler.marked_text_range(window, cx), Some(3..4));
    }); cx.quit();
}

#[test]
fn autocomplete_acceptance_manual_word_refuses_active_preedit() {
    autocomplete_acceptance_manual_during_preedit("EditAutocompleteWord");
}
#[test]
fn autocomplete_acceptance_manual_function_refuses_active_preedit() {
    autocomplete_acceptance_manual_during_preedit("EditAutocompleteFunc");
}

fn autocomplete_acceptance_manual_after_preedit(committed: bool) {
    use gpui::{ElementInputHandler, InputHandler};
    let _settings = crate::settings::override_for_test(preferences());
    for action in ["EditAutocompleteWord", "EditAutocompleteFunc", "EditAutocompleteFuncAndWord"] {
        let mut cx=TestAppContext::single(); let (workspace, visual)=fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.close_autocomplete(cx); w.active_tab_mut().buffer.rope=ropey::Rope::from_str("ret\nreturn");
            w.active_tab_mut().buffer.language=Some(crate::syntax::LanguageId::Rust);
            w.active_tab_mut().selection.collapse_to(3); cx.notify();
        }); visual.run_until_parked();
        visual.update(|window, cx| {
            let mut handler=ElementInputHandler::new(window.bounds(), workspace.clone());
            handler.replace_and_mark_text_in_range(None, "u", Some(1..1), window, cx);
            if committed { handler.replace_text_in_range(None, "u", window, cx); } else { handler.unmark_text(window, cx); }
            assert_eq!(handler.marked_text_range(window, cx), None);
        });
        if action=="EditAutocompleteFuncAndWord" { visual.dispatch_action(EditAutocompleteFuncAndWord); visual.run_until_parked(); } else { press(visual, &[action]); }
        visual.update(|window, cx| { let w=workspace.read(cx); assert!(!w.is_composing_text(window)); assert!(w.autocomplete.is_some()); });
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "return\nreturn"));
        press(visual, &["EditUndo"]);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "retu\nreturn"));
        press(visual, &["EditUndo"]);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "ret\nreturn"));
        cx.quit();
    }
}

#[test]
fn autocomplete_acceptance_manual_combined_refuses_active_preedit() {
    autocomplete_acceptance_manual_during_preedit("EditAutocompleteFuncAndWord");
}

#[test]
fn autocomplete_acceptance_manual_after_unmark_keeps_normal_completion() {
    autocomplete_acceptance_manual_after_preedit(false);
}
#[test]
fn autocomplete_acceptance_manual_after_commit_keeps_normal_completion() {
    autocomplete_acceptance_manual_after_preedit(true);
}

#[test]
fn autocomplete_boundary_real_unicode_save_accept_and_undo_preserve_encoding_and_crlf() {
    use crate::text::encoding::Encoding;
    let _settings=crate::settings::override_for_test(preferences());
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        for accept in ["enter", "tab"] {
            let temp=crate::persist::TempDir::new("completion-unicode-boundary"); let path=temp.join("neutral.txt");
            let original=crate::text::encoding::encode("😀 caf\r\ncaféine", encoding);
            std::fs::write(&path, &original).unwrap(); let buffer=crate::io::read_buffer(&path).unwrap();
            let mut cx=TestAppContext::single(); let (workspace, visual)=fixture(&mut cx);
            visual.update_window_entity(&workspace, |w, _, cx| {
                w.close_autocomplete(cx); w.active_tab_mut().buffer=buffer; w.active_tab_mut().selection.collapse_to(8); cx.notify();
            }); visual.run_until_parked(); press(visual, &["EditAutocompleteWord"]);
            visual.simulate_input("é");
            visual.update(|_, cx| { let w=workspace.read(cx); let state=w.autocomplete.as_ref().unwrap(); assert_eq!(state.replace_start, 5); assert_eq!(state.caret, 10); });
            visual.simulate_keystrokes(accept); press(visual, &["FileSave"]);
            assert_eq!(std::fs::read(&path).unwrap(), crate::text::encoding::encode("😀 caféine\r\ncaféine", encoding));
            press(visual, &["EditUndo"]);
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "😀 café\ncaféine"));
            press(visual, &["EditUndo", "FileSave"]);
            assert_eq!(std::fs::read(&path).unwrap(), original); cx.quit();
        }
    }
}

#[test]
fn autocomplete_boundary_real_unicode_selection_and_tab_switch_refuse_old_row_coordinates() {
    let _settings=crate::settings::override_for_test(preferences());
    for action in ["SelectLeft", "TabGoto2"] {
        let mut cx=TestAppContext::single(); let (workspace, visual)=fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.close_autocomplete(cx); w.active_tab_mut().buffer.rope=ropey::Rope::from_str("😀 café\ncaféine");
            w.active_tab_mut().selection.collapse_to(10); w.tabs.push(crate::tab::Tab::empty()); cx.notify();
        }); visual.run_until_parked(); press(visual, &["EditAutocompleteWord"]);
        let menu=visual.debug_bounds("autocomplete-menu").unwrap();
        press(visual, &[action]);
        visual.update(|_, cx| {
            let w=workspace.read(cx); assert!(w.autocomplete.is_none());
            if action=="SelectLeft" { assert_eq!(w.selection().ordered(), 8..10); }
            else { assert_eq!(w.active, 1); }
        });
        // Actual click at the old painted position; this does not inject a captured callback.
        visual.simulate_click(menu.origin+gpui::point(gpui::px(12.),gpui::px(12.)), gpui::Modifiers::none());
        visual.update(|_, cx| {
            let w=workspace.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), "😀 café\ncaféine");
            assert_eq!(w.tabs[1].buffer.rope.to_string(), ""); assert!(w.autocomplete.is_none());
            assert!(w.tabs.iter().all(|tab| tab.undo_stack.is_empty()));
        }); cx.quit();
    }
}

fn autocomplete_boundary_word_case_scope(language: Option<crate::syntax::LanguageId>, expected: &[&str]) {
    let _settings=crate::settings::override_for_test(preferences());
    let mut cx=TestAppContext::single(); let (workspace, visual)=fixture(&mut cx);
    visual.update_window_entity(&workspace, |w, _, cx| {
        w.close_autocomplete(cx); w.active_tab_mut().buffer.rope=ropey::Rope::from_str("an\nAndroid anchor");
        w.active_tab_mut().buffer.language=language; w.active_tab_mut().selection.collapse_to(2); cx.notify();
    }); visual.run_until_parked(); press(visual, &["EditAutocompleteWord"]);
    visual.update(|_, cx| {
        let w=workspace.read(cx); assert!(w.loaded_api_snapshot(language).is_none());
        assert_eq!(w.autocomplete.as_ref().unwrap().candidates, expected);
    }); cx.quit();
}
#[test]
fn autocomplete_boundary_normal_text_word_scan_matches_exact_prefix_case_without_api() {
    // AutoCompletion.cpp:485-486: Normal Text without an active API uses MATCHCASE.
    autocomplete_boundary_word_case_scope(None, &["anchor"]);
}
#[test]
fn autocomplete_boundary_rust_word_scan_keeps_insensitive_prefix_without_api() {
    autocomplete_boundary_word_case_scope(Some(crate::syntax::LanguageId::Rust), &["Android", "anchor"]);
}

#[test]
fn autocomplete_boundary_normal_text_case_refilter_accept_and_undo() {
    for (prefix, suffix, expected) in [("an", "c", "anchor"), ("An", "d", "Android")] {
        let _settings=crate::settings::override_for_test(Settings { autocompletion_brief: true, ..preferences() });
        let mut cx=TestAppContext::single(); let (workspace, visual)=fixture(&mut cx);
        let source=format!("{prefix}\nAndroid anchor");
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.close_autocomplete(cx); w.active_tab_mut().buffer.rope=ropey::Rope::from_str(&source);
            w.active_tab_mut().buffer.language=None; w.active_tab_mut().selection.collapse_to(prefix.len()); cx.notify();
        }); visual.run_until_parked(); press(visual, &["EditAutocompleteWord"]);
        visual.simulate_input(suffix);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).autocomplete.as_ref().unwrap().candidates, [expected]));
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), format!("{expected}\nAndroid anchor")));
        press(visual, &["EditUndo"]);
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), format!("{prefix}{suffix}\nAndroid anchor")));
        cx.quit();
    }
}

#[test]
fn autocomplete_boundary_backspace_then_case_change_rechecks_same_opening() {
    for language in [None, Some(crate::syntax::LanguageId::Rust)] {
        let _settings=crate::settings::override_for_test(Settings { autocompletion_brief: true, ..preferences() });
        let mut cx=TestAppContext::single(); let (workspace, visual)=fixture(&mut cx);
        visual.update_window_entity(&workspace, |w, _, cx| {
            w.close_autocomplete(cx); w.active_tab_mut().buffer.rope=ropey::Rope::from_str("an\nAndroid anchor");
            w.active_tab_mut().buffer.language=language; w.active_tab_mut().selection.collapse_to(2); cx.notify();
        }); visual.run_until_parked(); press(visual, &["EditAutocompleteWord"]);
        visual.simulate_keystrokes("backspace"); visual.simulate_input("N");
        visual.update(|_, cx| {
            let w=workspace.read(cx); assert_eq!(w.buffer().rope.to_string(), "aN\nAndroid anchor");
            if language.is_none() { assert!(w.autocomplete.is_none()); }
            else { assert_eq!(w.autocomplete.as_ref().unwrap().candidates, ["Android", "anchor"]); }
        }); cx.quit();
    }
}
