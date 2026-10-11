// PowerEditor/src/Notepad_plus.rc:574 / Parameters.cpp:203 — manual call hint.
use super::*;
use crate::workspace::autocomplete::completion_api_tests::{fixture, preferences, press};
use gpui::VisualContext;

fn definitions() -> &'static str {
    "<NotepadPlus><AutoComplete><KeyWord name='draw' func='yes'><Overload retVal='bool' descr='Draw an original synthetic shape.'><Param name='color: Color'/></Overload><Overload retVal='Result'><Param name='color: Color'/><Param name='width: usize'/><Param name='height: usize'/></Overload></KeyWord><KeyWord name='reticulate'/></AutoComplete></NotepadPlus>"
}

fn registry_directory() -> (
    crate::persist::TempDir,
    super::super::completion_api::TestApiDirectoryGuard,
) {
    let temp = crate::persist::TempDir::new("function-hint-fixture");
    std::fs::write(temp.join("rust.xml"), definitions()).unwrap();
    let directory =
        super::super::completion_api::override_directory_for_test(temp.path().to_owned());
    (temp, directory)
}

#[test]
fn manual_hint_shortcut_loads_signature_without_opening_completion() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("function-hint-manual");
    std::fs::write(temp.join("rust.xml"), "<NotepadPlus><AutoComplete><KeyWord name='draw' func='yes'><Overload retVal='bool' descr='Draw an original synthetic shape.'><Param name='color: Color'/><Param name='width: usize'/></Overload></KeyWord></AutoComplete></NotepadPlus>").unwrap();
    let _directory =
        crate::workspace::completion_api::override_directory_for_test(temp.path().to_owned());
    let mut cx = gpui::TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(blue, ");
    press(visual, "EditAutocompleteFuncParams");
    assert!(visual.debug_bounds("function-hint").is_some());
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert!(w.autocomplete.is_none());
        assert_eq!(w.buffer().rope.to_string(), "draw(blue, ");
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    cx.quit();
}

#[test]
fn overload_keys_and_mouse_controls_preserve_source_focus_and_history() {
    let _settings = crate::settings::override_for_test(preferences());
    let (_temp, _directory) = registry_directory();
    let mut cx = gpui::TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(blue, ");
    press(visual, "EditAutocompleteFuncParams");
    visual.update(|window, cx| {
        let snapshot = workspace.read(cx).function_hint_snapshot(window).unwrap();
        assert_eq!(snapshot.call.argument, 1);
        assert_eq!(snapshot.overload, 0);
    });
    press(visual, "EditAutocompleteFuncParamsPrevious");
    visual.update(|window, cx| {
        assert_eq!(
            workspace
                .read(cx)
                .function_hint_snapshot(window)
                .unwrap()
                .overload,
            1
        )
    });
    press(visual, "EditAutocompleteFuncParamsNext");
    visual.update(|window, cx| {
        assert_eq!(
            workspace
                .read(cx)
                .function_hint_snapshot(window)
                .unwrap()
                .overload,
            0
        )
    });
    let next = visual.debug_bounds("hint-next").unwrap();
    visual.simulate_click(next.center(), gpui::Modifiers::none());
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert_eq!(w.function_hint_snapshot(window).unwrap().overload, 1);
        assert!(w.focus_handle.is_focused(window));
        assert_eq!(w.buffer().rope.to_string(), "draw(blue, ");
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    visual.simulate_keystrokes("escape");
    visual.update(|_, cx| {
        assert!(
            !workspace.read(cx).function_hint_active(),
            "Escape must retire the hint intent"
        )
    });
    // GPUI keeps debug selector bounds between frames. Prove the old control
    // no longer consumes the click through the freshly rendered dispatch tree.
    visual.simulate_click(next.center(), gpui::Modifiers::none());
    visual.update(|_, cx| assert!(!workspace.read(cx).function_hint_active()));
    cx.quit();
}

#[test]
fn committed_input_triggers_and_updates_hints_while_preference_disabled_requires_manual_action() {
    for automatic in [false, true] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings {
            autocompletion_func_params_hint: automatic,
            ..preferences()
        });
        let (_temp, _directory) = registry_directory();
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw");
        visual.simulate_input("(");
        assert_eq!(visual.debug_bounds("function-hint").is_some(), automatic);
        if !automatic {
            press(visual, "EditAutocompleteFuncParams");
        }
        visual.simulate_input("blue, 10, ");
        visual.update(|window, cx| {
            let snapshot = workspace.read(cx).function_hint_snapshot(window).unwrap();
            assert_eq!(snapshot.call.argument, 2);
            assert_eq!(
                snapshot.overload, 1,
                "outgrown overload changes to one with enough parameters"
            );
        });
        visual.simulate_input("20)");
        visual.update(|_, cx| {
            assert!(
                !workspace.read(cx).function_hint_active(),
                "closed call: {}",
                workspace.read(cx).buffer().rope
            )
        });
        cx.quit();
    }
}

#[test]
fn hint_and_completion_coexist_without_intercepting_acceptance_or_covering_the_active_line() {
    for mouse in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let (_temp, _directory) = registry_directory();
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "return reticulate\n\n\n\n\n\n\ndraw(ret");
        press(visual, "EditAutocompleteWord");
        let opening =
            visual.update(|_, cx| workspace.read(cx).autocomplete.as_ref().unwrap().session_id);
        press(visual, "EditAutocompleteFuncParams");
        let hint = visual.debug_bounds("function-hint").unwrap();
        let completion = visual.debug_bounds("autocomplete-menu").unwrap();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            let caret = w.autocomplete_caret_bounds().unwrap();
            assert_eq!(w.autocomplete.as_ref().unwrap().session_id, opening);
            assert!(
                !hint.intersects(&completion)
                    && !hint.intersects(&caret)
                    && !completion.intersects(&caret)
            );
        });
        if mouse {
            visual.simulate_click(
                completion.origin + gpui::point(gpui::px(12.), gpui::px(12.)),
                gpui::Modifiers::none(),
            );
        } else {
            visual.simulate_keystrokes("enter");
        }
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.buffer().rope.to_string().ends_with("draw(reticulate"));
            assert!(w.autocomplete.is_none());
            assert!(w.function_hint_snapshot(window).is_some());
        });
        cx.quit();
    }
}

#[test]
fn hints_refuse_strings_comments_and_unbounded_prefixes_without_editing_text() {
    let _settings = crate::settings::override_for_test(preferences());
    let (_temp, _directory) = registry_directory();
    for source in [
        "\"draw(blue, ".to_owned(),
        "/* earlier\n draw(blue, ".to_owned(),
        format!(
            "{}draw(",
            "// comment\n".repeat(call_context::MAX_CALL_SCAN_BYTES / 11 + 1)
        ),
    ] {
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, &source);
        press(visual, "EditAutocompleteFuncParams");
        assert!(visual.debug_bounds("function-hint").is_none());
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), source);
            assert!(w.tabs[w.active].undo_stack.is_empty());
            if source.len() > call_context::MAX_CALL_SCAN_BYTES {
                assert!(w.status_message.as_ref().unwrap().contains("8 MiB"));
            }
        });
        cx.quit();
    }
}

fn held_scan(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> HintRequest {
    w.arm_function_hint(true, window, cx);
    w.refresh_function_hint_for_frame(window, cx);
    w.function_hint.task = None;
    w.function_hint.request.clone().unwrap()
}

#[test]
fn older_scan_or_painted_navigation_cannot_overwrite_a_new_hint_or_newer_status() {
    let _settings = crate::settings::override_for_test(preferences());
    let (_temp, _directory) = registry_directory();
    let mut cx = gpui::TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(");
    press(visual, "EditAutocompleteFuncParams");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let old_session = w.function_hint.state.as_ref().unwrap().snapshot.session;
        let older = held_scan(w, window, cx);
        let latest = held_scan(w, window, cx);
        let context = call_context::find_call(
            "draw(",
            5,
            &latest.api.registry.environment,
            CallLanguage::Rust,
        )
        .unwrap();
        w.set_status("A newer independent status");
        w.finish_function_hint(latest, Ok(context.clone()), window, cx);
        let current = w.function_hint_snapshot(window).unwrap();
        w.finish_function_hint(older, Ok(context), window, cx);
        w.navigate_function_hint_session(old_session, 1, window, cx);
        w.dismiss_function_hint_session(old_session, cx);
        assert_eq!(
            w.function_hint_snapshot(window).unwrap().session,
            current.session
        );
        assert_eq!(w.function_hint_snapshot(window).unwrap().overload, 0);
        assert_eq!(
            w.status_message.as_deref(),
            Some("A newer independent status")
        );
    });
    cx.quit();
}

#[test]
fn pending_hint_scan_cannot_return_after_escape_focus_window_or_view_round_trip() {
    for transition in [0, 1, 2, 3] {
        let _settings = crate::settings::override_for_test(preferences());
        let (_temp, _directory) = registry_directory();
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw(");
        press(visual, "EditAutocompleteFuncParams");
        let request = visual.update_window_entity(&workspace, held_scan);
        match transition {
            0 => visual.simulate_keystrokes("escape"),
            1 => {
                press(visual, "EditGotoLine");
                visual.simulate_keystrokes("escape");
            }
            2 => {
                visual.deactivate_window();
                visual.update(|window, _| window.activate_window());
                visual.run_until_parked();
            }
            _ => visual.update_window_entity(&workspace, |w, _, cx| {
                w.active_tab_mut().buffer.file_path = Some("hint.md".into());
                w.set_markdown_view_mode(true, cx);
                w.set_markdown_view_mode(false, cx);
            }),
        }
        visual.update_window_entity(&workspace, |w, window, cx| {
            let context = call_context::find_call(
                "draw(",
                5,
                &request.api.registry.environment,
                CallLanguage::Rust,
            )
            .unwrap();
            w.finish_function_hint(request, Ok(context), window, cx);
            assert!(!w.function_hint_active());
            assert!(w.function_hint_snapshot(window).is_none());
            assert_eq!(w.buffer().rope.to_string(), "draw(");
        });
        cx.quit();
    }
}

#[test]
fn ime_preedit_hides_hints_until_final_commit_or_unmark_in_fresh_source() {
    use gpui::{ElementInputHandler, InputHandler};
    for unmark in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let (_temp, _directory) = registry_directory();
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw");
        visual.update(|window, cx| {
            ElementInputHandler::new(window.bounds(), workspace.clone())
                .replace_and_mark_text_in_range(None, "(", Some(1..1), window, cx);
        });
        visual.run_until_parked();
        press(visual, "EditAutocompleteFuncParams");
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.is_composing_text(window));
            assert!(!w.function_hint_active());
            assert!(w.autocomplete.is_none());
        });
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
            if unmark {
                handler.unmark_text(window, cx);
            } else {
                handler.replace_text_in_range(None, "(", window, cx);
            }
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("function-hint").is_some());
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(!w.is_composing_text(window));
            assert_eq!(w.function_hint_snapshot(window).unwrap().call.argument, 0);
            assert_eq!(w.buffer().rope.to_string(), "draw(");
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| {
            assert!(!workspace.read(cx).function_hint_active());
            assert_eq!(workspace.read(cx).buffer().rope.to_string(), "draw");
        });
        cx.quit();
    }
}

#[test]
fn hint_padding_and_wheel_preserve_editor_selection_and_scroll_then_outside_click_dismisses() {
    let _settings = crate::settings::override_for_test(preferences());
    let (_temp, _directory) = registry_directory();
    let mut cx = gpui::TestAppContext::single();
    let source = format!("draw(blue, \n{}", "\n".repeat(200));
    let (workspace, visual) = fixture(&mut cx, &source);
    visual.update_window_entity(&workspace, |w, _, cx| {
        w.active_tab_mut().selection.collapse_to(11);
        cx.notify();
    });
    visual.run_until_parked();
    press(visual, "EditAutocompleteFuncParams");
    let hint = visual.debug_bounds("function-hint").unwrap();
    let old_next = visual.debug_bounds("hint-next").unwrap();
    visual.update(|_, cx| assert!(workspace.read(cx).max_scroll_top() > gpui::px(0.)));
    visual.simulate_click(
        hint.origin + gpui::point(gpui::px(2.), gpui::px(1.)),
        gpui::Modifiers::none(),
    );
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: hint.center(),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -1.)),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert!(w.function_hint_snapshot(window).is_some());
        assert!(w.focus_handle.is_focused(window));
        assert_eq!(w.selection().head, 11);
        assert_eq!(w.scroll_top(), gpui::px(0.));
    });
    let caret = visual.update(|_, cx| workspace.read(cx).autocomplete_caret_bounds().unwrap());
    visual.simulate_click(caret.origin, gpui::Modifiers::none());
    visual.update(|_, cx| assert!(!workspace.read(cx).function_hint_active()));
    visual.simulate_click(old_next.center(), gpui::Modifiers::none());
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert!(
            w.selection().head > 11,
            "the retired control must let clicks reach the editor"
        );
        assert!(!w.function_hint_active());
    });
    cx.quit();
}

#[test]
fn undo_redo_language_cursor_and_macro_replay_cannot_resurrect_a_held_scan() {
    for transition in ["undo", "redo", "language", "cursor", "macro", "completion"] {
        let _settings = crate::settings::override_for_test(preferences());
        let (_temp, _directory) = registry_directory();
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw(");
        visual.simulate_input("r");
        if transition == "redo" {
            press(visual, "EditUndo");
        }
        press(visual, "EditAutocompleteFuncParams");
        let request = visual.update_window_entity(&workspace, held_scan);
        match transition {
            "undo" => press(visual, "EditUndo"),
            "redo" => press(visual, "EditRedo"),
            "language" => visual.dispatch_action(crate::actions::LanguageNormalText),
            "cursor" => visual.simulate_keystrokes("left"),
            "macro" => visual.update_window_entity(&workspace, |w, window, cx| {
                assert!(w.replay_macro(
                    &[crate::macros::MacroEvent::InsertText("ed".into())],
                    1,
                    window,
                    cx
                ));
            }),
            _ => visual.update_window_entity(&workspace, |w, window, cx| {
                assert!(w.replay_completion_word("r", "red", window, cx));
            }),
        }
        visual.update_window_entity(&workspace, |w, window, cx| {
            let context = call_context::find_call(
                "draw(r",
                6,
                &request.api.registry.environment,
                CallLanguage::Rust,
            )
            .unwrap();
            w.finish_function_hint(request, Ok(context), window, cx);
            assert!(!w.function_hint_active(), "{transition}");
        });
        cx.quit();
    }
}

#[test]
fn actual_hint_frames_follow_wrapped_tabs_bottom_edges_zoom_and_long_api_labels() {
    for (source, wrap, zoom) in [
        ("\tdraw(".to_owned(), false, 1.),
        (format!("{}draw(", "word ".repeat(200)), true, 1.5),
        (format!("{}draw(", "\n".repeat(50)), false, 1.),
    ] {
        let _settings = crate::settings::override_for_test(preferences());
        let (temp, _directory) = registry_directory();
        let xml = format!("<NotepadPlus><AutoComplete><KeyWord name='draw' func='yes'><Overload retVal='{}' descr='{}'><Param name='{}'/></Overload></KeyWord></AutoComplete></NotepadPlus>", "r".repeat(8000), "description ".repeat(650), "中".repeat(2600));
        std::fs::write(temp.join("rust.xml"), xml).unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, &source);
        visual.simulate_resize(gpui::size(gpui::px(600.), gpui::px(400.)));
        visual.update_window_entity(&workspace, |w, window, cx| {
            w.word_wrap = wrap;
            w.zoom_level = zoom;
            window.set_rem_size(gpui::px(16. * zoom));
            cx.notify();
        });
        visual.run_until_parked();
        visual.update_window_entity(&workspace, |w, _, cx| w.ensure_cursor_visible(cx));
        visual.run_until_parked();
        press(visual, "EditAutocompleteFuncParams");
        let bounds = visual.debug_bounds("function-hint").unwrap();
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.function_hint_snapshot(window).is_some());
            let caret = w.autocomplete_caret_bounds().unwrap();
            assert!(!bounds.intersects(&caret));
            assert!(
                bounds.left() >= w.viewport.bounds.left()
                    && bounds.right() <= w.viewport.bounds.right()
            );
            assert!(
                bounds.top() >= w.viewport.bounds.top()
                    && bounds.bottom() <= w.viewport.bounds.bottom()
            );
            assert_eq!(w.buffer().rope.to_string(), source);
            assert!(w.tabs[w.active].undo_stack.is_empty());
            if !wrap && source.starts_with('\n') {
                assert!(bounds.bottom() <= caret.top());
            }
        });
        cx.quit();
    }
}

#[test]
fn matched_pair_opener_starts_hint_and_skipping_closer_retires_it() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        auto_insert_parens: true,
        ..preferences()
    });
    let (_temp, _directory) = registry_directory();
    let mut cx = gpui::TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw");
    visual.simulate_input("(");
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert!(w.function_hint_snapshot(window).is_some());
        assert_eq!(w.buffer().rope.to_string(), "draw()");
        assert_eq!(w.selection().head, 5);
    });
    visual.simulate_input(")");
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert!(!w.function_hint_active());
        assert_eq!(w.buffer().rope.to_string(), "draw()");
        assert_eq!(w.selection().head, 6);
        assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
    });
    cx.quit();
}

#[test]
fn same_frame_tab_round_trip_retires_the_hint_session_and_held_scan() {
    let _settings = crate::settings::override_for_test(preferences());
    let (_temp, _directory) = registry_directory();
    let mut cx = gpui::TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(");
    press(visual, "EditAutocompleteFuncParams");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let request = held_scan(w, window, cx);
        w.tabs.push(crate::tab::Tab::empty());
        w.set_active_tab(1, cx);
        w.set_active_tab(0, cx);
        let context = call_context::find_call("draw(", 5, &request.api.registry.environment, CallLanguage::Rust).unwrap();
        w.finish_function_hint(request, Ok(context), window, cx);
        assert!(!w.function_hint_active());
        assert!(w.function_hint_snapshot(window).is_none());
        assert_eq!(w.buffer().rope.to_string(), "draw(");
    });
    cx.quit();
}
