use super::*;
use crate::actions::{EditAutocompleteFunc, LanguageNormalText};
use crate::workspace::autocomplete::completion_api_tests::{fixture, preferences, press};
use gpui::{TestAppContext, VisualContext};

fn document(body: &str) -> String {
    format!("<NotepadPlus><AutoComplete>{body}</AutoComplete></NotepadPlus>")
}

fn held_request(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> ApiRequest {
    w.autocomplete_function(&EditAutocompleteFunc, window, cx);
    w.completion_api.task = None;
    w.completion_api.pending.clone().unwrap()
}

#[test]
fn stale_manual_api_reply_retires_only_its_loading_status() {
    for outcome in ["loaded", "missing", "invalid"] {
        for newer_status in [false, true] {
            let _settings = crate::settings::override_for_test(preferences());
            let temp = crate::persist::TempDir::new("api-stale-loading-status");
            let _directory = override_directory_for_test(temp.path().to_owned());
            let mut cx = TestAppContext::single();
            let (workspace, visual) = fixture(&mut cx, "head\r\ncaféZe");
            visual.update_window_entity(&workspace, |w, window, cx| {
                let request = held_request(w, window, cx);
                assert!(w.autocomplete.is_none());
                assert!(w.status_message.as_ref().unwrap().starts_with("Loading API definitions from "));
                let rope = w.buffer().rope.clone();
                let revision = w.buffer().raw_revision.clone();
                let selection = w.selection();
                window.focus(&w.find_focus, cx);
                window.focus(&w.focus_handle, cx);
                if newer_status { w.set_status("A newer command finished"); }
                let result = match outcome {
                    "loaded" => ApiRegistry::parse(&document("<KeyWord name='caféZeta'/>")).map(Some),
                    "missing" => Ok(None),
                    _ => ApiRegistry::parse("broken").map(Some),
                };
                w.finish_completion_api(request, temp.join("rust.xml"), result, window, cx);
                assert_eq!(w.status_message.as_deref(), newer_status.then_some("A newer command finished"));
                assert!(w.completion_api.pending.is_none());
                assert!(w.autocomplete.is_none());
                assert_eq!(w.loaded_api_snapshot(Some(LanguageId::Rust)).is_some(), outcome == "loaded");
                assert!(w.buffer().rope.is_instance(&rope));
                assert_eq!(w.buffer().raw_revision, revision);
                assert_eq!((w.selection().anchor, w.selection().head), (selection.anchor, selection.head));
                assert!(w.active_tab_mut().undo_stack.is_empty());
                assert!(w.active_tab_mut().redo_stack.is_empty());
            });
            cx.quit();
        }
    }
}

#[test]
fn stale_manual_api_reload_keeps_a_status_it_did_not_create() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("api-reload-status-owner");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "ret");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let first = held_request(w, window, cx);
        assert!(w.autocomplete.is_some());
        w.set_status("Loading API definitions from a user note");
        w.request_completion_api(CandidateKind::Function(Some(LanguageId::Rust)), true, window, cx);
        w.completion_api.task = None;
        let request = w.completion_api.pending.clone().unwrap();
        assert_ne!(request.id, first.id);
        assert_eq!(request.status.as_deref(), Some("Loading API definitions from a user note"));
        window.focus(&w.find_focus, cx);
        window.focus(&w.focus_handle, cx);
        w.finish_completion_api(request, temp.join("rust.xml"), Ok(None), window, cx);
        assert_eq!(w.status_message.as_deref(), Some("Loading API definitions from a user note"));
        assert!(w.completion_api.pending.is_none());
        assert_eq!(w.buffer().rope.to_string(), "ret");
    });
    cx.quit();
}

#[test]
fn an_older_manual_api_reply_cannot_clear_a_newer_loading_request() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("api-newer-loading-owner");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "caféZe");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let older = held_request(w, window, cx);
        let current = held_request(w, window, cx);
        assert_ne!(older.id, current.id);
        assert_eq!(older.status, current.status);
        w.finish_completion_api(older, temp.join("rust.xml"), Ok(None), window, cx);
        assert_eq!(w.status_message, current.status);
        assert_eq!(w.completion_api.pending.as_ref().map(|request| request.id), Some(current.id));
        w.finish_completion_api(current, temp.join("rust.xml"),
            ApiRegistry::parse(&document("<KeyWord name='caféZeta'/>")).map(Some), window, cx);
        assert!(w.completion_api.pending.is_none());
        assert!(w.status_message.as_ref().is_none_or(|status| !status.starts_with("Loading API definitions from ")));
        assert_eq!(w.autocomplete.as_ref().unwrap().candidates, ["caféZeta"]);
        assert_eq!(w.buffer().rope.to_string(), "caféZe");
        assert!(w.active_tab_mut().undo_stack.is_empty());
    });
    cx.quit();
}

#[test]
fn pending_load_preserves_a_newer_status_even_when_the_editor_context_stays_valid() {
    for invalid in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("api-status-ownership");
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "ret");
        visual.update_window_entity(&workspace, |w, window, cx| {
            let request = held_request(w, window, cx);
            w.set_status("An unrelated command finished");
            let result = if invalid {
                ApiRegistry::parse("broken").map(Some)
            } else {
                ApiRegistry::parse(&document("<KeyWord name='retFresh'/>")).map(Some)
            };
            w.finish_completion_api(request, temp.join("rust.xml"), result, window, cx);
            assert_eq!(
                w.status_message.as_deref(),
                Some("An unrelated command finished")
            );
            if !invalid {
                assert_eq!(w.autocomplete.as_ref().unwrap().candidates, ["retFresh"]);
            }
        });
        cx.quit();
    }
}

#[test]
fn cold_registry_replaces_only_api_names_and_keeps_overload_metadata_out_of_insertion() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("api-cold-load");
    let xml = document("<KeyWord name='reticulate' func='yes'><Overload retVal='Result&lt;bool&gt;' descr='Original test description.'><Param name='count: usize'/></Overload></KeyWord><KeyWord name='alpha'/>");
    std::fs::write(temp.join("rust.xml"), &xml).unwrap();
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "ret");
    let opening = visual.update_window_entity(&workspace, |w, window, cx| {
        w.autocomplete_function(&EditAutocompleteFunc, window, cx);
        let state = w.autocomplete.as_ref().unwrap();
        assert!(
            state.candidates.iter().any(|name| name == "return"),
            "original completion remains interactive during loading"
        );
        state.session_id
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        let state = w.autocomplete.as_ref().unwrap();
        assert_ne!(state.session_id, opening);
        assert_eq!(state.candidates, ["alpha", "reticulate"]);
        assert_eq!(
            state.candidates.get(state.selected).map(String::as_str),
            Some("reticulate")
        );
        let registry = w
            .loaded_api_snapshot(Some(LanguageId::Rust))
            .unwrap()
            .registry;
        assert_eq!(
            registry.find("reticulate").unwrap().overloads[0].parameters,
            ["count: usize"]
        );
        assert_eq!(w.tabs[w.active].undo_stack.len(), 0);
    });
    visual.simulate_keystrokes("enter");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "reticulate"));
    press(visual, "EditUndo");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "ret"));
    assert_eq!(std::fs::read_to_string(temp.join("rust.xml")).unwrap(), xml);
    cx.quit();
}

#[test]
fn utf16_api_completion_preserves_unicode_crlf_and_document_encoding_through_undo() {
    use crate::text::{
        encoding::{self, Encoding},
        line_ending::LineEnding,
    };
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("api-unicode-fidelity");
        let xml = document("<KeyWord name='caféine'/>");
        let definitions = encoding::encode(&xml, Encoding::Utf16Le);
        std::fs::write(temp.join("rust.xml"), &definitions).unwrap();
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "head\r\ncafé");
        visual.update_window_entity(&workspace, |w, _, _| {
            w.active_tab_mut().buffer.encoding = encoding;
            w.active_tab_mut().buffer.line_ending = LineEnding::Crlf;
        });
        press(visual, "EditAutocompleteFunc");
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| {
            let buffer = workspace.read(cx).buffer();
            assert_eq!(buffer.rope.to_string(), "head\r\ncaféine");
            assert_eq!(buffer.encoding, encoding);
            assert_eq!(buffer.line_ending, LineEnding::Crlf);
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| {
            let buffer = workspace.read(cx).buffer();
            assert_eq!(buffer.rope.to_string(), "head\r\ncafé");
            assert_eq!(
                encoding::encode(&buffer.rope.to_string(), buffer.encoding),
                encoding::encode("head\r\ncafé", encoding)
            );
        });
        assert_eq!(std::fs::read(temp.join("rust.xml")).unwrap(), definitions);
        cx.quit();
    }
}

#[test]
fn missing_definitions_keep_original_completion_and_invalid_reload_keeps_last_good_snapshot() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("api-fallback");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "ret");
    press(visual, "EditAutocompleteFunc");
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        let state = w.autocomplete.as_ref().unwrap();
        assert_eq!(
            state.candidates.get(state.selected).map(String::as_str),
            Some("return")
        );
        assert!(w.loaded_api_snapshot(Some(LanguageId::Rust)).is_none());
    });
    std::fs::write(
        temp.join("rust.xml"),
        document("<KeyWord name='reticulate'/>"),
    )
    .unwrap();
    press(visual, "EditAutocompleteFunc");
    let good = visual.update(|_, cx| {
        workspace
            .read(cx)
            .loaded_api_snapshot(Some(LanguageId::Rust))
            .unwrap()
    });
    std::fs::write(temp.join("rust.xml"), b"<broken>").unwrap();
    press(visual, "EditAutocompleteFunc");
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        let still_good = w.loaded_api_snapshot(Some(LanguageId::Rust)).unwrap();
        assert!(Arc::ptr_eq(&still_good.registry, &good.registry));
        assert_eq!(still_good.generation, good.generation);
        assert_eq!(w.autocomplete.as_ref().unwrap().candidates, ["reticulate"]);
        assert!(w
            .status_message
            .as_ref()
            .unwrap()
            .contains("Cannot load API definitions"));
        assert!(w.status_message.as_ref().unwrap().contains("rust.xml"));
        assert_eq!(w.buffer().rope.to_string(), "ret");
        assert!(w.tabs[w.active].undo_stack.is_empty());
    });
    assert_eq!(std::fs::read(temp.join("rust.xml")).unwrap(), b"<broken>");
    cx.quit();
}

#[test]
fn normal_registry_environment_controls_qualified_prefixes_case_filtering_and_acceptance() {
    for (source, body, suffix, expected) in [
        ("api.$wr", "<Environment ignoreCase='no' additionalWordChar='.$'/><KeyWord name='api.$write' func='yes'><Overload retVal='bool'><Param name='value'/></Overload></KeyWord><KeyWord name='api.$wrong'/>", "i", "api.$write"),
        ("Be", "<Environment ignoreCase='no'/><KeyWord name='Beta'/><KeyWord name='beta'/>", "t", "Beta"),
    ] {
        let _settings = crate::settings::override_for_test(crate::settings::Settings { autocompletion_brief: true, ..preferences() });
        let temp = crate::persist::TempDir::new("api-normal-environment");
        std::fs::write(temp.join("normal.xml"), document(body)).unwrap();
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source);
        visual.update_window_entity(&workspace, |w, _, _| w.active_tab_mut().buffer.language = None);
        press(visual, "EditAutocompleteFunc");
        assert!(visual.debug_bounds("autocomplete-menu").is_some());
        visual.simulate_input(suffix);
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(state.replace_start, 0);
            assert_eq!(state.candidates, [expected]);
        });
        visual.simulate_keystrokes("tab");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), expected));
        press(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), format!("{source}{suffix}")));
        cx.quit();
    }
}

#[test]
fn combined_completion_merges_registry_names_with_words_and_word_matching_uses_environment() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("api-combined");
    std::fs::write(
        temp.join("rust.xml"),
        document("<Environment ignoreCase='no'/><KeyWord name='apricot'/>"),
    )
    .unwrap();
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "Apple apple applied\nap");
    visual.dispatch_action(crate::actions::EditAutocompleteFuncAndWord);
    visual.update(|_, cx| {
        assert_eq!(
            workspace.read(cx).autocomplete.as_ref().unwrap().candidates,
            ["apple", "applied", "apricot"]
        )
    });
    visual.simulate_keystrokes("down enter");
    visual.update(|_, cx| {
        assert_eq!(
            workspace.read(cx).buffer().rope.to_string(),
            "Apple apple applied\napplied"
        )
    });
    press(visual, "EditUndo");
    press(visual, "EditAutocompleteWord");
    visual.update(|_, cx| {
        assert_eq!(
            workspace.read(cx).autocomplete.as_ref().unwrap().candidates,
            ["apple", "applied"]
        )
    });
    cx.quit();
}

#[test]
fn automatic_completion_uses_negative_cache_until_a_manual_request_discovers_a_new_file() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        autocompletion_mode: crate::settings::AutocompletionMode::Function,
        autocompletion_from_chars: 1,
        ..preferences()
    });
    let temp = crate::persist::TempDir::new("api-negative-cache");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "r");
    visual.simulate_input("e");
    visual.update(|_, cx| {
        assert!(workspace
            .read(cx)
            .completion_api
            .contains(Some(LanguageId::Rust)))
    });
    std::fs::write(temp.join("rust.xml"), document("<KeyWord name='redraw'/>")).unwrap();
    visual.simulate_keystrokes("escape");
    visual.simulate_input("d");
    visual.update(|_, cx| {
        assert!(workspace
            .read(cx)
            .loaded_api_snapshot(Some(LanguageId::Rust))
            .is_none())
    });
    press(visual, "EditAutocompleteFunc");
    visual.update(|_, cx| {
        assert_eq!(
            workspace.read(cx).autocomplete.as_ref().unwrap().candidates,
            ["redraw"]
        )
    });
    cx.quit();
}

#[test]
fn an_older_reply_cannot_replace_newer_api_popup_or_status_and_old_rows_stay_inert() {
    let _settings = crate::settings::override_for_test(preferences());
    let temp = crate::persist::TempDir::new("api-reordered-replies");
    let _directory = override_directory_for_test(temp.path().to_owned());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "ret");
    visual.update_window_entity(&workspace, |w, window, cx| {
        let older = held_request(w, window, cx);
        let current = held_request(w, window, cx);
        let current_popup = w.autocomplete.as_ref().unwrap().session_id;
        w.set_status("Current interaction status");
        w.finish_completion_api(
            older.clone(),
            temp.join("rust.xml"),
            Err(ApiRegistry::parse("broken").unwrap_err()),
            window,
            cx,
        );
        assert_eq!(
            w.status_message.as_deref(),
            Some("Current interaction status")
        );
        assert_eq!(w.autocomplete.as_ref().unwrap().session_id, current_popup);
        assert!(w.loaded_api_snapshot(Some(LanguageId::Rust)).is_none());
        w.finish_completion_api(
            current,
            temp.join("rust.xml"),
            Ok(Some(
                ApiRegistry::parse(&document("<KeyWord name='retLatest'/>")).unwrap(),
            )),
            window,
            cx,
        );
        let latest = w.loaded_api_snapshot(Some(LanguageId::Rust)).unwrap();
        let latest_popup = w.autocomplete.as_ref().unwrap().session_id;
        w.set_status("Latest status");
        w.finish_completion_api(
            older.clone(),
            temp.join("rust.xml"),
            Ok(Some(
                ApiRegistry::parse(&document("<KeyWord name='retOld'/>")).unwrap(),
            )),
            window,
            cx,
        );
        assert_eq!(
            w.loaded_api_snapshot(Some(LanguageId::Rust))
                .unwrap()
                .generation,
            latest.generation
        );
        assert_eq!(w.autocomplete.as_ref().unwrap().session_id, latest_popup);
        assert_eq!(w.status_message.as_deref(), Some("Latest status"));
        assert!(!w.accept_autocomplete_session(older.popup.unwrap(), "return", window, cx));
        assert_eq!(w.autocomplete.as_ref().unwrap().session_id, latest_popup);
        assert_eq!(w.buffer().rope.to_string(), "ret");
    });
    visual.simulate_keystrokes("enter");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "retLatest"));
    cx.quit();
}

#[test]
fn changed_editor_context_keeps_its_text_popup_and_status_when_a_reply_arrives() {
    for change in ["edit", "language", "preview", "read-only", "tab"] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("api-stale-context");
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "ret");
        let request = visual.update_window_entity(&workspace, held_request);
        if change == "edit" {
            visual.simulate_input("u");
        }
        visual.update_window_entity(&workspace, |w, window, cx| {
            match change {
                "language" => w.language_normal_text(&LanguageNormalText, window, cx),
                "preview" => w.active_tab_mut().view_mode = crate::tab::TabViewMode::CsvPreview,
                "read-only" => w.active_tab_mut().buffer.read_only = true,
                "tab" => {
                    w.tabs.push(crate::tab::Tab::empty());
                    w.active = 1;
                }
                _ => {}
            }
            let before = w.buffer().rope.to_string();
            let popup = w.autocomplete.as_ref().map(|state| state.session_id);
            w.set_status("New editor context status");
            w.finish_completion_api(
                request,
                temp.join("rust.xml"),
                Ok(Some(
                    ApiRegistry::parse(&document("<KeyWord name='retLatest'/>")).unwrap(),
                )),
                window,
                cx,
            );
            assert_eq!(w.buffer().rope.to_string(), before);
            assert_eq!(w.autocomplete.as_ref().map(|state| state.session_id), popup);
            assert_eq!(
                w.status_message.as_deref(),
                Some("New editor context status")
            );
            assert!(w.loaded_api_snapshot(Some(LanguageId::Rust)).is_some());
        });
        cx.quit();
    }
}

#[test]
fn cache_bounds_include_missing_languages_and_directory_overrides_restore() {
    let mut cache = CompletionApiCache::default();
    for language in [
        LanguageId::Rust,
        LanguageId::Python,
        LanguageId::C,
        LanguageId::CSharp,
    ] {
        cache.store(Some(language), None);
    }
    cache.touch(Some(LanguageId::Rust));
    cache.store(None, None);
    assert_eq!(cache.slots.len(), MAX_CACHED_LANGUAGES);
    assert!(cache.contains(Some(LanguageId::Rust)));
    assert!(!cache.contains(Some(LanguageId::Python)));
    assert!(cache.contains(None));
    assert!(api_directory().is_none());
    let outer = override_directory_for_test(PathBuf::from("synthetic-outer"));
    {
        let _inner = override_directory_for_test(PathBuf::from("synthetic-inner"));
        assert_eq!(api_directory(), Some(PathBuf::from("synthetic-inner")));
    }
    assert_eq!(api_directory(), Some(PathBuf::from("synthetic-outer")));
    drop(outer);
    assert!(api_directory().is_none());
}

#[test]
fn pending_normal_api_cannot_reopen_after_focus_or_window_left_and_returned() {
    for deactivate in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let temp = crate::persist::TempDir::new("api-pending-lifecycle");
        let _directory = override_directory_for_test(temp.path().to_owned());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "cu");
        let request = visual.update_window_entity(&workspace, |w, window, cx| {
            w.active_tab_mut().buffer.language = None;
            w.autocomplete_function(&EditAutocompleteFunc, window, cx);
            assert!(w.autocomplete.is_none());
            // Hold the synthetic reply; no file read or user configuration access.
            w.completion_api.task = None;
            w.completion_api.pending.clone().unwrap()
        });
        if deactivate {
            visual.deactivate_window();
            visual.update(|window, _| window.activate_window());
            visual.run_until_parked();
        } else {
            press(visual, "EditGotoLine");
            visual.simulate_keystrokes("escape");
        }
        visual.update_window_entity(&workspace, |w, window, cx| {
            assert!(window.is_window_active());
            assert!(w.focus_handle.is_focused(window));
            w.set_status("A later dialog owns this status");
            let registry = ApiRegistry::parse(&document("<KeyWord name='customName'/>")).unwrap();
            w.finish_completion_api(
                request,
                temp.join("normal.xml"),
                Ok(Some(registry)),
                window,
                cx,
            );
            assert!(
                w.autocomplete.is_none(),
                "a late reply must not revive the dismissed interaction"
            );
            assert_eq!(
                w.status_message.as_deref(),
                Some("A later dialog owns this status")
            );
            assert!(
                w.loaded_api_snapshot(None).is_some(),
                "the valid reply can still prime the cache"
            );
        });
        cx.quit();
    }
}

#[test]
fn known_languages_use_safe_explicit_api_filenames() {
    assert_eq!(api_filename(None), "normal.xml");
    assert_eq!(api_filename(Some(LanguageId::CSharp)), "cs.xml");
    assert_eq!(api_filename(Some(LanguageId::JavaScript)), "javascript.xml");
    assert_eq!(api_filename(Some(LanguageId::TypeScript)), "typescript.xml");
    assert_eq!(api_filename(Some(LanguageId::PowerShell)), "powershell.xml");
    assert_eq!(api_filename(Some(LanguageId::Tsx)), "tsx.xml");
    for language in LanguageId::all() {
        let path = std::path::Path::new(api_filename(Some(*language)));
        assert_eq!(path.components().count(), 1);
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("xml")
        );
    }
}
