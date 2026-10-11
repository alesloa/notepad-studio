use crate::settings::AutocompletionMode;
use crate::workspace::autocomplete::completion_api_tests::{fixture, preferences, press};
use gpui::TestAppContext;

#[test]
fn native_typing_with_hostile_annotations_preserves_text_and_history() {
    for target in [0, 1, 2] {
        let mut settings = preferences();
        settings.autocompletion_mode = AutocompletionMode::None;
        settings.autocompletion_func_params_hint = false;
        let _settings = crate::settings::override_for_test(settings);
        let _recent = crate::recent::override_snapshot_for_test(Vec::new());
        let mut cx = TestAppContext::single();
        let source = "aé\r\n中\n";
        let (workspace, visual) = fixture(&mut cx, source);
        visual.update(|_, cx| {
            workspace.update(cx, |w, _| {
                let tab = w.active_tab_mut();
                tab.buffer.mark_saved();
                match target {
                    0 => {
                        tab.buffer
                            .bookmarks
                            .extend([0, isize::MAX as usize, usize::MAX]);
                    }
                    1 => {
                        tab.buffer.marks = vec![0..3, isize::MAX as usize..isize::MAX as usize + 1];
                    }
                    2 => {
                        tab.begin_select_marker = Some(isize::MAX as usize);
                    }
                    _ => unreachable!(),
                }
            })
        });
        visual.simulate_input("z");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            let tab = &w.tabs[w.active];
            assert_eq!(tab.buffer.rope.to_string(), format!("{source}z"));
            assert_eq!(tab.undo_stack.len(), 1);
            assert!(tab
                .buffer
                .bookmarks
                .iter()
                .all(|b| *b <= tab.buffer.rope.len_bytes()));
            assert!(tab.buffer.marks.iter().all(|r| tab
                .buffer
                .rope
                .get_byte_slice(r.clone())
                .is_some()));
            assert!(tab.begin_select_marker.is_none());
        });
        press(visual, "EditUndo");
        visual.update(|_, cx| {
            assert_eq!(workspace.read(cx).buffer().rope.to_string(), source);
            assert!(!workspace.read(cx).buffer().dirty);
        });
        press(visual, "EditRedo");
        visual.update(|_, cx| {
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                format!("{source}z")
            )
        });
        cx.quit();
    }
}

#[test]
fn malformed_begin_end_marker_is_retired_without_moving_the_caret() {
    let _settings = crate::settings::override_for_test(preferences());
    let _recent = crate::recent::override_snapshot_for_test(Vec::new());
    for marker in [isize::MAX as usize, usize::MAX, 1] {
        let mut cx = TestAppContext::single();
        let (workspace, visual) = fixture(&mut cx, "é\n");
        visual.update(|_, cx| {
            workspace.update(cx, |w, _| {
                w.active_tab_mut().begin_select_marker = Some(marker)
            })
        });
        visual.simulate_input("x");
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "é\nx");
            assert!(w.tabs[w.active].begin_select_marker.is_none());
            assert_eq!(w.selection().head, "é\nx".len());
        });
        cx.quit();
    }
}
