// PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:325-327,451-454 —
// manual Function Completion shows API names and selects the typed prefix.
use gpui::{Entity, TestAppContext, VisualTestContext, WindowAppearance};

use super::Workspace;
use crate::{settings::Settings, syntax::LanguageId};

pub(in crate::workspace) fn press(visual: &mut VisualTestContext, action: &str) {
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

pub(in crate::workspace) fn fixture<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
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
        w.active_tab_mut().selection.collapse_to(source.len());
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    (workspace, visual)
}

pub(in crate::workspace) fn preferences() -> Settings {
    Settings {
        toolbar_hidden: true,
        tab_bar_hidden: true,
        folder_panel_visible: false,
        ..Settings::default()
    }
}

#[test]
fn manual_function_completion_shows_names_outside_the_prefix_and_selects_a_late_match() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "ret");
    press(visual, "EditAutocompleteFunc");
    assert!(visual.debug_bounds("autocomplete-menu").is_some());
    visual.update(|_, cx| {
        let state = workspace.read(cx).autocomplete.as_ref().unwrap();
        assert!(state.candidates.iter().any(|name| name == "Arc"));
        assert_eq!(
            state.candidates.get(state.selected).map(String::as_str),
            Some("return")
        );
        assert!(state.candidates.len() <= super::MAX_CANDIDATES);
    });
    visual.simulate_keystrokes("enter");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "return"));
    press(visual, "EditUndo");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "ret"));
    cx.quit();
}

#[test]
fn manual_function_completion_dismisses_when_no_name_matches_the_prefix() {
    let _settings = crate::settings::override_for_test(preferences());
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "zzz");
    press(visual, "EditAutocompleteFunc");
    // ScintillaBase.cxx:369-372 / AutoComplete.cxx:249-300 — auto-hide on a miss.
    assert!(visual.debug_bounds("autocomplete-menu").is_none());
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert!(w.autocomplete.is_none());
        assert_eq!(w.buffer().rope.to_string(), "zzz");
        assert_eq!(w.status_message.as_deref(), Some("Auto-completion: no match for \"zzz\""));
    });
    visual.simulate_keystrokes("escape");
    cx.quit();
}

#[test]
fn initial_manual_function_can_complete_an_explicit_member_from_mouse_or_keys() {
    for mouse in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "ret");
        press(visual, "EditAutocompleteFunc");
        press(visual, "MoveLineStart"); // Select the first explicit member without wrapping.
        visual.update(|_, cx| {
            let state = workspace.read(cx).autocomplete.as_ref().unwrap();
            assert_eq!(
                state.candidates.get(state.selected).map(String::as_str),
                Some("Arc")
            );
        });
        if mouse {
            let bounds = visual.debug_bounds("autocomplete-menu").unwrap();
            visual.simulate_click(
                bounds.origin + gpui::point(gpui::px(12.), gpui::px(12.)),
                gpui::Modifiers::none(),
            );
        } else {
            visual.simulate_keystrokes("enter");
        }
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "Arc");
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "ret"));
        cx.quit();
    }
}

#[test]
fn initial_manual_function_handles_case_correction_and_exact_noop_without_fake_undo() {
    for source in ["Arc", "arc"] {
        let _settings = crate::settings::override_for_test(preferences());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source);
        press(visual, "EditAutocompleteFunc");
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "Arc");
            assert_eq!(
                w.tabs[w.active].undo_stack.len(),
                usize::from(source != "Arc")
            );
            assert!(w.autocomplete.is_none());
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
        cx.quit();
    }
}

#[test]
fn editing_permanently_revokes_the_manual_member_exception_for_retained_rows() {
    for mouse in [false, true] {
        let _settings = crate::settings::override_for_test(preferences());
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "ret");
        press(visual, "EditAutocompleteFunc");
        visual.simulate_input("u");
        press(visual, "MoveLineStart");
        if mouse {
            let bounds = visual.debug_bounds("autocomplete-menu").unwrap();
            visual.simulate_click(
                bounds.origin + gpui::point(gpui::px(12.), gpui::px(12.)),
                gpui::Modifiers::none(),
            );
        } else {
            visual.simulate_keystrokes("enter");
        }
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(
                w.buffer().rope.to_string(),
                if mouse { "retu" } else { "retu\n" }
            );
            assert!(w.autocomplete.is_none());
            if mouse {
                assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
            }
        });
        if !mouse {
            press(visual, "EditUndo");
            visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "retu"));
        }
        cx.quit();
    }
}
