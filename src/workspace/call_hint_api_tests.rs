// Hint-only API reads share the immutable cache, never a completion invocation.
use super::*;
use crate::workspace::autocomplete::completion_api_tests::{fixture, preferences, press};
use gpui::{TestAppContext, VisualContext};

fn definitions(parameter: &str) -> String {
    format!("<NotepadPlus><AutoComplete><KeyWord name='draw' func='yes'><Overload retVal='bool'><Param name='{parameter}'/></Overload></KeyWord></AutoComplete></NotepadPlus>")
}

fn held_request(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> ApiRequest {
    w.show_function_hint(window, cx);
    w.completion_api.task = None;
    w.completion_api.pending.clone().unwrap()
}

#[test]
fn same_frame_focus_round_trip_retires_hint_and_completion_api_invocations() {
    for hint in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("api-focus-aba");
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw(");
        visual.update_window_entity(&workspace, |w, window, cx| {
            let request = if hint {
                held_request(w, window, cx)
            } else {
                w.request_completion_api(CandidateKind::Function(Some(LanguageId::Rust)),
                    true, window, cx);
                w.completion_api.task = None;
                w.completion_api.pending.clone().unwrap()
            };
            let other = cx.focus_handle();
            window.focus(&other, cx);
            window.focus(&w.focus_handle, cx);
            assert!(!w.api_request_context_valid(&request, window));
            w.set_status("A newer independent status");
            w.finish_completion_api(request, temp.join("rust.xml"),
                Ok(Some(ApiRegistry::parse(&definitions("cached only")).unwrap())),
                window, cx);
            assert!(w.loaded_api_snapshot(Some(LanguageId::Rust)).is_some());
            assert!(w.autocomplete.is_none());
            assert!(!w.function_hint_visible());
            assert_eq!(w.status_message.as_deref(), Some("A newer independent status"));
            assert_eq!(w.buffer().rope.to_string(), "draw(");
            assert!(w.tabs[w.active].undo_stack.is_empty());
        });
        cx.quit();
    }
}

#[test]
fn cold_hint_intent_cannot_restart_after_a_same_frame_focus_round_trip() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("cold-hint-focus-aba");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let request = held_request(w, window, cx);
        let other = cx.focus_handle();
        window.focus(&other, cx);
        window.focus(&w.focus_handle, cx);
        w.set_status("A newer independent status");
        // A cache-only reply must not start a new scan for the old manual
        // intent while its deferred blur notification is still pending.
        w.finish_completion_api(request, temp.join("rust.xml"),
            Ok(Some(ApiRegistry::parse(&definitions("cached only")).unwrap())),
            window, cx);
        assert!(w.loaded_api_snapshot(Some(LanguageId::Rust)).is_some());
        assert!(!w.function_hint_active());
        assert!(w.function_hint_snapshot(window).is_none());
        assert!(w.autocomplete.is_none());
        assert_eq!(w.status_message.as_deref(), Some("A newer independent status"));
        assert_eq!(w.buffer().rope.to_string(), "draw(");
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    cx.quit();
}

#[test]
fn current_hint_and_completion_api_requests_survive_noop_focus_and_repaint() {
    for hint in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("api-noop-focus");
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw(");
        let request = visual.update_window_entity(&workspace, |w, window, cx| {
            let request = if hint { held_request(w, window, cx) } else {
                w.request_completion_api(CandidateKind::Function(Some(LanguageId::Rust)),
                    true, window, cx);
                w.completion_api.task = None;
                w.completion_api.pending.clone().unwrap()
            };
            let focus = window.input_context_token();
            window.focus(&w.focus_handle, cx);
            window.activate_window();
            assert_eq!(window.input_context_token(), focus);
            request
        });
        visual.run_until_parked();
        visual.update(|window, cx| { let _ = window.draw(cx); });
        visual.update_window_entity(&workspace, |w, window, cx| {
            assert!(w.api_request_context_valid(&request, window));
            w.finish_completion_api(request, temp.join("rust.xml"),
                Ok(Some(ApiRegistry::parse(&definitions("current parameter")).unwrap())),
                window, cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            if hint {
                assert!(w.function_hint_snapshot(window).is_some());
                assert!(w.autocomplete.is_none());
            } else {
                assert!(w.autocomplete.as_ref().unwrap().candidates.iter().any(|name| name == "draw"));
            }
            assert_eq!(w.buffer().rope.to_string(), "draw(");
            assert!(w.tabs[w.active].undo_stack.is_empty());
            assert!(w.tabs[w.active].redo_stack.is_empty());
        });
        cx.quit();
    }
}

#[test]
fn hint_only_api_replies_never_open_completion_and_older_reads_cannot_replace_newer_definitions() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("hint-api-held");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let older = held_request(w, window, cx);
        let latest = held_request(w, window, cx);
        w.set_status("A newer independent status");
        w.finish_completion_api(
            latest,
            temp.join("rust.xml"),
            Ok(Some(
                ApiRegistry::parse(&definitions("new parameter")).unwrap(),
            )),
            window,
            cx,
        );
        let generation = w
            .loaded_api_snapshot(Some(LanguageId::Rust))
            .unwrap()
            .generation;
        w.finish_completion_api(
            older,
            temp.join("rust.xml"),
            Ok(Some(
                ApiRegistry::parse(&definitions("old parameter")).unwrap(),
            )),
            window,
            cx,
        );
        assert_eq!(
            w.loaded_api_snapshot(Some(LanguageId::Rust))
                .unwrap()
                .generation,
            generation
        );
        assert!(w.autocomplete.is_none());
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        let hint = w.function_hint_snapshot(window).unwrap();
        assert_eq!(
            hint.api.registry.find("draw").unwrap().overloads[0].parameters,
            ["new parameter"]
        );
        assert_eq!(
            w.status_message.as_deref(),
            Some("A newer independent status")
        );
        assert!(w.autocomplete.is_none());
    });
    cx.quit();
}

#[test]
fn pending_hint_api_cannot_reopen_after_escape_focus_window_or_source_view_round_trip() {
    for transition in 0..4 {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("hint-api-lifecycle");
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw(");
        let request = visual.update_window_entity(&workspace, held_request);
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
                w.active_tab_mut().buffer.file_path = Some("neutral.md".into());
                w.set_markdown_view_mode(true, cx);
                w.set_markdown_view_mode(false, cx);
            }),
        }
        visual.update_window_entity(&workspace, |w, window, cx| {
            w.set_status("A later dialog status");
            w.finish_completion_api(
                request,
                temp.join("rust.xml"),
                Ok(Some(
                    ApiRegistry::parse(&definitions("ignored UI reply")).unwrap(),
                )),
                window,
                cx,
            );
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(
                w.loaded_api_snapshot(Some(LanguageId::Rust)).is_some(),
                "a stale read can prime the cache"
            );
            assert!(!w.function_hint_active());
            assert!(w.function_hint_snapshot(window).is_none() && w.autocomplete.is_none());
            assert_eq!(w.status_message.as_deref(), Some("A later dialog status"));
        });
        cx.quit();
    }
}

#[test]
fn hint_loading_status_is_owned_and_missing_or_malformed_definitions_remain_actionable() {
    for malformed in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("hint-api-errors");
        if malformed {
            std::fs::write(temp.join("rust.xml"), b"<broken>").unwrap();
        }
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "draw(");
        press(visual, "EditAutocompleteFuncParams");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(!w.function_hint_active() && w.autocomplete.is_none());
            assert!(w.status_message.as_ref().unwrap().contains("rust.xml"));
            assert!(!w.status_message.as_ref().unwrap().starts_with("Loading"));
        });
        if malformed {
            assert_eq!(std::fs::read(temp.join("rust.xml")).unwrap(), b"<broken>");
        }
        cx.quit();
    }
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("hint-api-cancel-status");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(");
    visual.update_window_entity(&workspace, |w, window, cx| {
        held_request(w, window, cx);
    });
    visual.simulate_keystrokes("escape");
    visual.update(|_, cx| assert!(workspace.read(cx).status_message.is_none()));
    cx.quit();
}

#[test]
fn manual_hint_reload_replaces_valid_signatures_and_preserves_last_good_on_error() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("hint-api-reload");
    std::fs::write(temp.join("rust.xml"), definitions("first")).unwrap();
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "draw(");
    press(visual, "EditAutocompleteFuncParams");
    let first = visual.update(|window, cx| {
        workspace
            .read(cx)
            .function_hint_snapshot(window)
            .unwrap()
            .api
            .generation
    });
    std::fs::write(temp.join("rust.xml"), definitions("updated 中")).unwrap();
    press(visual, "EditAutocompleteFuncParams");
    let latest = visual.update(|window, cx| {
        let hint = workspace.read(cx).function_hint_snapshot(window).unwrap();
        assert_ne!(first, hint.api.generation);
        assert_eq!(
            hint.api.registry.find("draw").unwrap().overloads[0].parameters,
            ["updated 中"]
        );
        hint.api.generation
    });
    std::fs::write(temp.join("rust.xml"), b"<broken>").unwrap();
    press(visual, "EditAutocompleteFuncParams");
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert_eq!(
            w.function_hint_snapshot(window).unwrap().api.generation,
            latest
        );
        assert!(w.status_message.as_ref().unwrap().contains("last valid"));
        assert!(w.autocomplete.is_none());
    });
    assert_eq!(std::fs::read(temp.join("rust.xml")).unwrap(), b"<broken>");
    cx.quit();
}
