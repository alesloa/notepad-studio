use gpui::{ElementInputHandler, InputHandler, TestAppContext, VisualContext};

use crate::settings::{AutocompletionMode, Settings};
use crate::workspace::autocomplete::completion_api_tests::{fixture, preferences, press};

fn settings(mode: AutocompletionMode) -> Settings {
    Settings {
        autocompletion_mode: mode,
        autocompletion_from_chars: 3,
        autocompletion_func_params_hint: false,
        ..preferences()
    }
}

#[test]
fn ime_final_or_unmark_opens_word_completion_and_preserves_saved_encoding_history() {
    use crate::text::encoding::{encode, Encoding};
    for unmark in [false, true] {
        for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
            let _settings = crate::settings::override_for_test(settings(AutocompletionMode::Word));
            let directory = crate::persist::TempDir::new("ime-completion-save");
            let path = directory.join("neutral.rs");
            std::fs::write(&path, encode("android andromeda\r\n", encoding)).unwrap();
            let buffer = crate::io::read_buffer(&path).unwrap();
            let original = buffer.rope.to_string();
            let mut cx = TestAppContext::single();
            let (workspace, visual) = fixture(&mut cx, &original);
            visual.update_window_entity(&workspace, |w, _, _| {
                w.active_tab_mut().buffer = buffer;
                w.active_tab_mut().selection.collapse_to(original.len());
            });
            visual.update(|window, cx| {
                let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
                handler.replace_and_mark_text_in_range(None, "and", Some(3..3), window, cx);
                assert!(workspace.read(cx).autocomplete.is_none());
                assert!(workspace.read(cx).is_composing_text(window));
                if unmark {
                    handler.unmark_text(window, cx);
                } else {
                    handler.replace_text_in_range(None, "and", window, cx);
                }
                handler.unmark_text(window, cx);
            });
            visual.run_until_parked();
            let bounds = visual.debug_bounds("autocomplete-menu").unwrap();
            visual.update(|window, cx| {
                let w = workspace.read(cx);
                assert!(!w.is_composing_text(window));
                assert_eq!(
                    w.autocomplete.as_ref().unwrap().candidates,
                    ["android", "andromeda"]
                );
                assert!(!bounds.intersects(&w.autocomplete_caret_bounds().unwrap()));
                assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
            });
            visual.simulate_keystrokes("enter");
            press(visual, "FileSave");
            let saved = encode("android andromeda\r\nandroid", encoding);
            assert_eq!(std::fs::read(&path).unwrap(), saved);
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert!(w.autocomplete.is_none());
                assert_eq!(w.tabs[w.active].undo_stack.len(), 2);
                assert!(!w.buffer().dirty);
            });
            press(visual, "EditUndo");
            visual.update(|_, cx| {
                assert_eq!(
                    workspace.read(cx).buffer().rope.to_string(),
                    format!("{original}and")
                )
            });
            press(visual, "EditUndo");
            visual
                .update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), original));
            press(visual, "EditRedo");
            press(visual, "EditRedo");
            visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
            cx.quit();
        }
    }
}

#[test]
fn ime_completion_respects_disabled_threshold_nonword_and_selected_replacement_input() {
    for (mode, text, selected) in [
        (AutocompletionMode::None, "and", false),
        (AutocompletionMode::Word, "an", false),
        (AutocompletionMode::Word, "and ", false),
        (AutocompletionMode::Word, "and😀", false),
        (AutocompletionMode::Word, "and", true),
    ] {
        let _settings = crate::settings::override_for_test(settings(mode));
        let mut cx = TestAppContext::single();
        let source = if selected {
            "android andromeda\nx"
        } else {
            "android andromeda\n"
        };
        let (workspace, visual) = fixture(&mut cx, source);
        if selected {
            visual.update_window_entity(&workspace, |w, _, _| {
                w.active_tab_mut().selection.anchor = source.len() - 1
            });
        }
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
            let end = text.encode_utf16().count();
            handler.replace_and_mark_text_in_range(None, text, Some(end..end), window, cx);
            handler.unmark_text(window, cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(!w.is_composing_text(window));
            assert!(w.autocomplete.is_none());
            assert_eq!(
                w.buffer().rope.to_string(),
                format!("android andromeda\n{text}")
            );
            assert_eq!(w.tabs[w.active].undo_stack.len(), 1);
        });
        cx.quit();
    }
}

#[test]
fn cancelled_ime_keeps_prior_redo_and_never_opens_completion() {
    let _settings = crate::settings::override_for_test(settings(AutocompletionMode::Word));
    let mut cx = TestAppContext::single();
    let source = "android andromeda\n";
    let (workspace, visual) = fixture(&mut cx, source);
    visual.simulate_input("x");
    press(visual, "EditUndo");
    visual.update(|window, cx| {
        let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
        handler.replace_and_mark_text_in_range(None, "and", Some(3..3), window, cx);
        handler.replace_text_in_range(None, "", window, cx);
        handler.unmark_text(window, cx);
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        let w = workspace.read(cx);
        assert!(w.autocomplete.is_none());
        assert_eq!(w.buffer().rope.to_string(), source);
        assert!(w.tabs[w.active].undo_stack.is_empty());
        assert_eq!(w.tabs[w.active].redo_stack.len(), 1);
    });
    press(visual, "EditRedo");
    visual.update(|_, cx| {
        assert_eq!(
            workspace.read(cx).buffer().rope.to_string(),
            format!("{source}x")
        )
    });
    cx.quit();
}

#[test]
fn moving_a_marked_range_does_not_open_completion_between_preedit_updates() {
    let _settings = crate::settings::override_for_test(settings(AutocompletionMode::Word));
    let mut cx = TestAppContext::single();
    let (workspace, visual) = fixture(&mut cx, "android andromeda\n");
    visual.update(|window, cx| {
        let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
        handler.replace_and_mark_text_in_range(None, "and", Some(3..3), window, cx);
        handler.replace_and_mark_text_in_range(Some(0..0), "and", Some(3..3), window, cx);
        let w = workspace.read(cx);
        assert!(w.is_composing_text(window));
        assert!(w.autocomplete.is_none());
        assert_eq!(w.buffer().rope.to_string(), "andandroid andromeda\nand");
        assert_eq!(w.tabs[w.active].undo_stack.len(), 2);
        handler.unmark_text(window, cx);
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        let w = workspace.read(cx);
        assert!(!w.is_composing_text(window));
        assert!(w.autocomplete.is_some());
        assert_eq!(w.tabs[w.active].undo_stack.len(), 2);
    });
    cx.quit();
}

#[test]
fn ime_final_completion_counts_unicode_characters_and_accepts_without_losing_the_prefix() {
    for (source, text, threshold, accepted) in [
        ("café caféine\n", "café", 3, "caféine"),
        ("中文稿 中文字\n", "中文", 2, "中文字"),
    ] {
        let _settings = crate::settings::override_for_test(Settings {
            autocompletion_from_chars: threshold,
            ..settings(AutocompletionMode::Word)
        });
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, source);
        visual.update(|window, cx| {
            let mut handler = ElementInputHandler::new(window.bounds(), workspace.clone());
            let end = text.encode_utf16().count();
            handler.replace_and_mark_text_in_range(None, text, Some(end..end), window, cx);
            assert!(workspace.read(cx).autocomplete.is_none());
            handler.replace_text_in_range(None, text, window, cx);
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            assert!(workspace
                .read(cx)
                .autocomplete
                .as_ref()
                .unwrap()
                .candidates
                .iter()
                .any(|name| name == accepted))
        });
        visual.simulate_keystrokes("enter");
        visual.update(|_, cx| {
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                format!("{source}{accepted}")
            )
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| {
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                format!("{source}{text}")
            )
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), source));
        cx.quit();
    }
}
