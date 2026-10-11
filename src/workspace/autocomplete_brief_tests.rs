// PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:1065–1083 —
// brief lists shrink; ordinary lists retain rows and move their selection.
use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext, WindowAppearance};

use super::{CandidateKind, Workspace};
use crate::{actions::*, settings::Settings, syntax::LanguageId};

fn preferences(brief: bool) -> Settings {
    Settings {
        toolbar_hidden: true,
        tab_bar_hidden: true,
        folder_panel_visible: false,
        autocompletion_brief: brief,
        ..Settings::default()
    }
}

fn press(visual: &mut VisualTestContext, action: &str) {
    let entry = crate::shortcuts::catalog()
        .iter()
        .find(|entry| entry.id == action)
        .unwrap();
    visual.simulate_keystrokes(
        crate::shortcuts::platform_default_keys(entry)
            .first()
            .unwrap(),
    );
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
    caret: usize,
    kind: CandidateKind,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
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
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(source);
        w.active_tab_mut().buffer.language = Some(LanguageId::Rust);
        w.active_tab_mut().selection.collapse_to(caret);
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    match kind {
        CandidateKind::Word => press(visual, "EditAutocompleteWord"),
        CandidateKind::Function(_) => press(visual, "EditAutocompleteFunc"),
        CandidateKind::Combined(_) => visual.dispatch_action(EditAutocompleteFuncAndWord),
    }
    assert!(visual.debug_bounds("autocomplete-menu").is_some());
    (workspace, visual)
}

#[test]
fn brief_preference_controls_rows_for_each_source_and_accepts_the_live_prefix() {
    for brief in [false, true] {
        let _settings = crate::settings::override_for_test(preferences(brief));
        for kind in [
            CandidateKind::Word,
            CandidateKind::Function(LanguageId::Rust),
            CandidateKind::Combined(LanguageId::Rust),
        ] {
            let mut cx = TestAppContext::single();
            let (workspace, visual) = fixture(&mut cx, "re\nref return relic", 2, kind);
            let opened = visual.update(|_, cx| {
                workspace
                    .read(cx)
                    .autocomplete
                    .as_ref()
                    .unwrap()
                    .candidates
                    .clone()
            });
            assert!(opened.len() >= 2);
            visual.simulate_input("t");
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                let state = w.autocomplete.as_ref().unwrap();
                assert_eq!(
                    state.candidates,
                    if brief {
                        vec!["return".to_string()]
                    } else {
                        opened.clone()
                    }
                );
                assert_eq!(
                    state.candidates.get(state.selected).map(String::as_str),
                    Some("return")
                );
                assert_eq!(state.caret, 3);
                assert_eq!(state.replace_start, 0);
            });
            visual.simulate_keystrokes("enter");
            visual.update(|_, cx| {
                assert_eq!(
                    workspace.read(cx).buffer().rope.to_string(),
                    "return\nref return relic"
                )
            });
            press(visual, "EditUndo");
            visual.update(|_, cx| {
                assert_eq!(
                    workspace.read(cx).buffer().rope.to_string(),
                    "ret\nref return relic"
                )
            });
            cx.quit();
        }
    }
}

#[test]
fn narrowing_preserves_a_selected_item_that_still_matches() {
    for brief in [false, true] {
        let _settings = crate::settings::override_for_test(preferences(brief));
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(
            &mut cx,
            "an\nanchor android andromeda",
            2,
            CandidateKind::Word,
        );
        visual.simulate_keystrokes("down down");
        visual.simulate_input("d");
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(
                state.candidates.get(state.selected).map(String::as_str),
                Some("andromeda")
            );
            assert_eq!(state.candidates.len(), if brief { 2 } else { 3 });
            assert_eq!(state.candidate_pool.len(), 3);
        });
        visual.simulate_keystrokes("backspace");
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(
                state.candidates.get(state.selected).map(String::as_str),
                Some("andromeda")
            );
            assert_eq!(state.candidates.len(), 3);
        });
        cx.quit();
    }
}

#[test]
fn capped_lists_discover_hidden_matches_without_losing_the_row_bound() {
    for brief in [false, true] {
        let _settings = crate::settings::override_for_test(preferences(brief));
        let words: Vec<String> = (0..70).map(|i| format!("an{i:03}word")).collect();
        let source = format!("an\n{}", words.join(" "));
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, &source, 2, CandidateKind::Word);
        visual.update(|_, cx| {
            assert_eq!(
                workspace
                    .read(cx)
                    .autocomplete
                    .as_ref()
                    .unwrap()
                    .candidates
                    .len(),
                super::MAX_CANDIDATES
            )
        });
        visual.simulate_input("06");
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(
                state.candidates.len(),
                if brief { 10 } else { super::MAX_CANDIDATES }
            );
            assert!(state.candidate_pool.len() <= super::MAX_CANDIDATES);
            for word in words.iter().skip(60) {
                assert!(state.candidates.contains(word));
            }
            assert!(
                state
                    .candidates
                    .get(state.selected)
                    .unwrap()
                    .starts_with("an06")
            );
        });
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| {
            assert!(
                workspace
                    .read(cx)
                    .buffer()
                    .rope
                    .to_string()
                    .starts_with("an060word\n")
            )
        });
        cx.quit();
    }
}

#[test]
fn retained_nonmatching_and_already_typed_rows_cannot_change_text_or_history() {
    let _settings = crate::settings::override_for_test(preferences(false));
    for suffix in ["t", "f"] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "re\nref refine return", 2, CandidateKind::Word);
        visual.simulate_input(suffix);
        let before = visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(
                w.autocomplete
                    .as_ref()
                    .unwrap()
                    .candidates
                    .first()
                    .map(String::as_str),
                Some("ref")
            );
            (
                w.buffer().rope.to_string(),
                w.tabs[w.active].undo_stack.len(),
            )
        });
        let bounds = visual.debug_bounds("autocomplete-menu").unwrap();
        visual.simulate_click(
            bounds.origin + gpui::point(gpui::px(12.), gpui::px(12.)),
            gpui::Modifiers::none(),
        );
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), before.0);
            assert_eq!(w.tabs[w.active].undo_stack.len(), before.1);
            assert!(w.autocomplete.is_none());
        });
        cx.quit();
    }
}

#[test]
fn settings_keep_old_absent_and_saved_brief_values_loadable() {
    let old: Settings = toml::from_str("").unwrap();
    assert!(!old.autocompletion_brief);
    for brief in [false, true] {
        let settings: Settings =
            toml::from_str(&format!("autocompletion_brief = {brief}")).unwrap();
        assert_eq!(settings.autocompletion_brief, brief);
        let restored: Settings = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.autocompletion_brief, brief);
    }
}

#[test]
fn broadening_a_capped_query_keeps_a_selected_matching_row() {
    for brief in [false, true] {
        let _settings = crate::settings::override_for_test(preferences(brief));
        let words: Vec<String> = (0..70).map(|i| format!("an{i:03}word")).collect();
        let source = format!("an06\n{}", words.join(" "));
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, &source, 4, CandidateKind::Word);
        for _ in 0..9 {
            visual.simulate_keystrokes("down");
        }
        visual.simulate_keystrokes("backspace");
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(state.candidates.len(), super::MAX_CANDIDATES);
            assert_eq!(
                state.candidates.get(state.selected).map(String::as_str),
                Some("an069word")
            );
            assert!(state.candidate_pool.len() <= super::MAX_CANDIDATES);
        });
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| {
            assert!(
                workspace
                    .read(cx)
                    .buffer()
                    .rope
                    .to_string()
                    .starts_with("an069word\n")
            )
        });
        cx.quit();
    }
}

#[test]
fn choosing_a_retained_nonmatch_runs_the_normal_enter_or_tab_action() {
    let _settings = crate::settings::override_for_test(Settings {
        indent_uses_tabs: true,
        ..preferences(false)
    });
    for (key, inserted) in [("enter", "\n"), ("tab", "\t")] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "re\nref return relic", 2, CandidateKind::Word);
        visual.simulate_input("t");
        visual.simulate_keystrokes("up up");
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(
                state.candidates.get(state.selected).map(String::as_str),
                Some("ref")
            );
        });
        visual.simulate_keystrokes(key);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(
                w.buffer().rope.to_string(),
                format!("ret{inserted}\nref return relic")
            );
            assert!(w.autocomplete.is_none());
        });
        cx.quit();
    }
}

#[test]
fn unicode_completion_retains_bom_crlf_and_undo_prefix_fidelity() {
    use crate::text::{encoding::Encoding, line_ending::LineEnding};

    for brief in [false, true] {
        let _settings = crate::settings::override_for_test(preferences(brief));
        for bom in ["", "\u{feff}"] {
            let mut cx = TestAppContext::single();
            let source = format!("{bom}caf\r\ncafé caféine cap");
            let (workspace, visual) = fixture(&mut cx, &source, bom.len() + 3, CandidateKind::Word);
            let encoding = if bom.is_empty() {
                Encoding::Utf16Le
            } else {
                Encoding::Utf8Bom
            };
            visual.update_window_entity(&workspace, |w, _, _| {
                w.active_tab_mut().buffer.encoding = encoding;
                w.active_tab_mut().buffer.line_ending = LineEnding::Crlf;
            });
            visual.simulate_input("é");
            visual.update(|_, cx| {
                let state = workspace.read(cx).autocomplete.as_ref().unwrap();
                assert_eq!(
                    state.candidates.get(state.selected).map(String::as_str),
                    Some("caféine")
                );
                assert_eq!(state.replace_start, bom.len());
                assert_eq!(state.caret, bom.len() + 5);
            });
            visual.simulate_keystrokes("enter");
            visual.update(|_, cx| {
                assert_eq!(workspace.read(cx).buffer().encoding, encoding);
                assert_eq!(workspace.read(cx).buffer().line_ending, LineEnding::Crlf);
                assert_eq!(
                    workspace.read(cx).buffer().rope.to_string(),
                    format!("{bom}caféine\r\ncafé caféine cap")
                )
            });
            press(visual, "EditUndo");
            visual.update(|_, cx| {
                assert_eq!(workspace.read(cx).buffer().encoding, encoding);
                assert_eq!(workspace.read(cx).buffer().line_ending, LineEnding::Crlf);
                assert_eq!(
                    workspace.read(cx).buffer().rope.to_string(),
                    format!("{bom}café\r\ncafé caféine cap")
                )
            });
            cx.quit();
        }
    }
}
