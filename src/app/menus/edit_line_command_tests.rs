use crate::{settings::Settings, tab::Tab, workspace::Workspace};
use gpui::{
    Action, Entity, MenuItem, TestAppContext, VisualContext, VisualTestContext, WindowAppearance,
};

fn menu_action(label: &str) -> Box<dyn Action> {
    fn find(items: &[MenuItem], label: &str) -> Option<Box<dyn Action>> {
        for item in items {
            match item {
                MenuItem::Action { name, action, .. } if name.as_ref() == label => {
                    return Some(action.boxed_clone())
                }
                MenuItem::Submenu(menu) => {
                    if let Some(action) = find(&menu.items, label) {
                        return Some(action);
                    }
                }
                _ => {}
            }
        }
        None
    }
    find(&super::build(&super::MenuState::default()).items, label).expect("actual Edit menu action")
}

fn with_source(
    text: &str,
    anchor: usize,
    head: usize,
    test: impl FnOnce(&Entity<Workspace>, &mut VisualTestContext),
) {
    let _settings = crate::settings::override_for_test(Settings {
        indent_uses_tabs: false,
        tab_width: 4,
        ..Default::default()
    });
    let mut cx = TestAppContext::single();
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
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.line_ending = crate::text::line_ending::LineEnding::Lf;
        tab.selection.anchor = anchor;
        tab.selection.head = head;
        w.tabs = vec![tab];
        w.active = 0;
        w.window_closed = true;
        w
    });
    visual.update(|window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
    });
    visual.run_until_parked();
    test(&workspace, visual);
    cx.quit();
}

fn invoke(label: &str, visual: &mut VisualTestContext) {
    let action = menu_action(label);
    visual.update(|window, cx| window.dispatch_action(action, cx));
    visual.run_until_parked();
}

#[test]
fn edit_menu_increase_indent_changes_line_prefix_without_inserting_at_caret() {
    with_source("a😀b\nq", 5, 5, |workspace, visual| {
        invoke("Increase Line Indent", visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "    a😀b\nq");
            assert_eq!((w.selection().anchor, w.selection().head), (9, 9));
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a😀b\nq"));
    });
}

#[test]
fn edit_menu_indent_preserves_selected_unicode_text_and_canonicalizes_mixed_indent() {
    with_source("  café😀z\nq", 7, 2, |workspace, visual| {
        invoke("Increase Line Indent", visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "      café😀z\nq");
            assert_eq!((w.selection().anchor, w.selection().head), (11, 6));
        });
    });
    with_source(" \té😀\nq", 2, 4, |workspace, visual| {
        invoke("Decrease Line Indent", visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "é😀\nq");
            assert_eq!((w.selection().anchor, w.selection().head), (0, 2));
        });
    });
}

#[test]
fn edit_menu_duplicate_current_line_uses_caret_line_with_selected_text() {
    with_source("ab😀cd\nq", 1, 6, |workspace, visual| {
        invoke("Duplicate Current Line", visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "ab😀cd\nab😀cd\nq");
            assert_eq!((w.selection().anchor, w.selection().head), (1, 6));
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual
            .update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "ab😀cd\nq"));
    });
}

#[test]
fn edit_menu_line_actions_have_distinct_catalog_ids_from_keyboard_commands() {
    for (label, id, oracle) in [
        (
            "Increase Line Indent",
            "EditIncreaseLineIndent",
            "IDM_EDIT_INS_TAB",
        ),
        (
            "Decrease Line Indent",
            "EditDecreaseLineIndent",
            "IDM_EDIT_RMV_TAB",
        ),
        (
            "Duplicate Current Line",
            "EditDuplicateCurrentLine",
            "IDM_EDIT_DUP_LINE",
        ),
    ] {
        let action = menu_action(label);
        assert!(
            action.name().ends_with(id),
            "{label} should have its own action, got {}",
            action.name()
        );
        let row = crate::shortcuts::catalog()
            .iter()
            .find(|entry| entry.id == id)
            .expect("rebindable menu command");
        assert_eq!(row.npp, Some(oracle));
        assert!(row.keys.is_empty());
        assert!(row.mac_keys.is_empty());
    }
}

#[test]
fn edit_menu_line_indent_groups_multiline_unicode_edits_and_restores_selection() {
    with_source("a\r\n\r\n \t😀\rq", 12, 1, |workspace, visual| {
        let before = visual.update(|_, cx| workspace.read(cx).selection());
        invoke("Increase Line Indent", visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "    a\r\n\r\n        😀\rq");
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.tabs[0].edit_transaction.is_none());
            assert!(w.selection().anchor > w.selection().head);
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "a\r\n\r\n \t😀\rq");
            assert_eq!(
                (w.selection().anchor, w.selection().head),
                (before.anchor, before.head)
            );
        });
        visual.dispatch_action(crate::actions::EditRedo);
        visual.run_until_parked();
        visual.update(|_, cx| {
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                "    a\r\n\r\n        😀\rq"
            )
        });
    });
}

#[test]
fn edit_menu_line_commands_preserve_refused_or_noop_history_and_source() {
    for refusal in [
        "noop",
        "read-only",
        "preview",
        "inactive",
        "loading",
        "block",
    ] {
        with_source("é😀\nq", 2, 2, |workspace, visual| {
            invoke("Increase Line Indent", visual);
            visual.dispatch_action(crate::actions::EditUndo);
            visual.run_until_parked();
            if refusal == "inactive" {
                visual.deactivate_window();
            }
            visual.update_window_entity(workspace, |w, _, _| match refusal {
                "read-only" => w.tabs[0].buffer.read_only = true,
                "preview" => w.tabs[0].view_mode = crate::tab::TabViewMode::MarkdownPreview,
                "loading" => {
                    w.tabs[0].buffer.loading = Some(crate::buffer::LoadProgress {
                        bytes_read: 0,
                        total_bytes: 12_000_000,
                        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    })
                }
                "block" => {
                    w.tabs[0].block_selection = Some(crate::buffer::BlockSelection::default())
                }
                _ => {}
            });
            invoke("Decrease Line Indent", visual);
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), "é😀\nq", "{refusal}");
                assert_eq!(w.tabs[0].redo_stack.len(), 1, "{refusal}");
                assert!(w.tabs[0].undo_stack.is_empty());
                assert!(w.tabs[0].edit_transaction.is_none());
                if refusal != "noop" {
                    assert!(w.status_message.is_some(), "{refusal}");
                }
            });
        });
    }
}

#[test]
fn edit_menu_line_commands_leave_a_focused_find_field_untouched() {
    with_source("é😀\nq", 2, 2, |workspace, visual| {
        visual.update_window_entity(workspace, |w, window, cx| {
            w.find_state.open = true;
            w.find_state.query = "find😀".to_string();
            window.focus(&w.find_focus.clone(), cx);
            cx.notify();
        });
        visual.run_until_parked();
        for label in [
            "Increase Line Indent",
            "Decrease Line Indent",
            "Duplicate Current Line",
        ] {
            invoke(label, visual);
            visual.update(|window, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), "é😀\nq");
                assert_eq!(w.find_state.query, "find😀");
                assert!(w.find_focus.is_focused(window));
                assert!(w.tabs[0].undo_stack.is_empty());
            });
        }
    });
}

#[test]
fn edit_menu_line_actions_rebind_and_replay_as_one_nested_macro_undo() {
    with_source("a\nq", 1, 1, |workspace, visual| {
        let mut overrides = crate::shortcuts::Overrides::new();
        overrides.insert(
            "EditIncreaseLineIndent".into(),
            vec!["ctrl-alt-shift-i".into()],
        );
        visual.update(|_, cx| crate::shortcuts::reload_keymap(cx, &overrides));
        visual.simulate_keystrokes("ctrl-alt-shift-i");
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "    a\nq"));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update_window_entity(workspace, |w, window, cx| {
            assert!(w.replay_macro(
                &[
                    crate::macros::MacroEvent::IncreaseLineIndent,
                    crate::macros::MacroEvent::DuplicateCurrentLine
                ],
                2,
                window,
                cx
            ));
            assert_eq!(
                w.buffer().rope.to_string(),
                "        a\n        a\n    a\nq"
            );
            assert_eq!(w.tabs[0].undo_stack.len(), 1);
            assert!(w.tabs[0].edit_transaction.is_none());
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| assert_eq!(workspace.read(cx).buffer().rope.to_string(), "a\nq"));
    });
}

#[test]
fn edit_menu_line_indent_uses_the_current_language_override() {
    with_source("a😀", 1, 1, |workspace, visual| {
        let mut settings = Settings {
            indent_uses_tabs: false,
            tab_width: 4,
            ..Default::default()
        };
        settings.language_indent_overrides.insert(
            "rust".into(),
            crate::settings::IndentOverride {
                uses_tabs: true,
                width: 2,
            },
        );
        let _settings = crate::settings::override_for_test(settings);
        visual.update_window_entity(workspace, |w, _, _| {
            w.tabs[0].buffer.language = Some(crate::syntax::LanguageId::Rust)
        });
        invoke("Increase Line Indent", visual);
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "\ta😀");
            assert_eq!(w.selection().head, 2);
        });
    });
}

#[test]
fn edit_menu_line_commands_save_bom_utf16_crlf_and_restore_saved_identity_with_undo_redo() {
    use crate::text::encoding::{encode, Encoding};
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let path =
            std::env::temp_dir().join(format!("notepad-studio-line-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, encode("é😀\r\nq", encoding)).unwrap();
        with_source("", 0, 0, |workspace, visual| {
            visual.update_window_entity(workspace, |w, _, cx| {
                w.tabs[0].buffer = crate::io::read_buffer(&path).unwrap();
                w.tabs[0].selection.collapse_to(2);
                cx.notify();
            });
            visual.run_until_parked();
            invoke("Increase Line Indent", visual);
            invoke("Duplicate Current Line", visual);
            visual.dispatch_action(crate::actions::FileSave);
            visual.run_until_parked();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                encode("    é😀\r\n    é😀\r\nq", encoding)
            );
            for _ in 0..2 {
                visual.dispatch_action(crate::actions::EditUndo);
                visual.run_until_parked();
            }
            visual.update(|_, cx| {
                let w = workspace.read(cx);
                assert_eq!(w.buffer().rope.to_string(), "é😀\nq");
                assert_eq!(w.selection().head, 2);
                assert!(w.buffer().dirty);
                assert_eq!(w.buffer().encoding, encoding);
                assert_eq!(
                    w.buffer().line_ending,
                    crate::text::line_ending::LineEnding::Crlf
                );
            });
            for _ in 0..2 {
                visual.dispatch_action(crate::actions::EditRedo);
                visual.run_until_parked();
            }
            visual.update(|_, cx| assert!(!workspace.read(cx).buffer().dirty));
        });
        std::fs::remove_file(path).unwrap();
    }
}
