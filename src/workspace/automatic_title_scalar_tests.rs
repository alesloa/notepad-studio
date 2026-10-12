use super::*;

#[test]
fn automatic_title_scalar_replace_retains_delete_effect_before_empty_insert() {
    with_automatic_title(|entity, visual| {
        automatic_title_paste(visual, "original\nintermediate\n");
        assert_eq!(automatic_title_name(entity, visual), "original");
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.tabs[0].selection = crate::buffer::Selection { anchor: 0, head: 9, ..Default::default() };
        }));
        automatic_title_paste(visual, "\n");
        assert_eq!(automatic_title_name(entity, visual), "intermediate");
        visual.update(|_, cx| {
            let tab = &entity.read(cx).tabs[0];
            assert_eq!(tab.buffer.rope.to_string(), "\nintermediate\n");
            assert_eq!(tab.undo_stack.len(), 2);
            assert!(!tab.buffer.new_label_is_custom);
        });
        visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "original");
        visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "intermediate");
    });
}

#[test]
fn automatic_title_scalar_crlf_pair_uses_post_delete_row() {
    for (range, expected, bytes) in [(6..7, "retained", "first\rlater"), (5..6, "first", "first\nlater")] {
        let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str("first\r\nlater");
        buffer.new_label = Some("retained".into());
        with_editor(buffer, |entity, visual| {
            let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
            let _preference = crate::settings::override_for_test(settings);
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.tabs[0].selection = crate::buffer::Selection { anchor: range.start, head: range.end, ..Default::default() };
            }));
            visual.dispatch_action(crate::actions::EditDelete); visual.run_until_parked();
            assert_eq!(automatic_title_name(entity, visual), expected);
            visual.update(|_, cx| {
                let tab = &entity.read(cx).tabs[0];
                assert_eq!(tab.buffer.rope.to_string(), bytes);
                assert_eq!(tab.undo_stack.len(), 1);
            });
            visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
            visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), "first\r\nlater"));
        });
    }
}

#[test]
fn automatic_title_scalar_preference_enable_and_second_row_edit_keep_name() {
    let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str("first\nsecond");
    buffer.new_label = Some("retained".into());
    with_editor(buffer, |entity, visual| {
        let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
        let _preference = crate::settings::override_for_test(settings);
        visual.run_until_parked(); assert_eq!(automatic_title_name(entity, visual), "retained");
        visual.dispatch_action(crate::actions::MoveDocEnd); visual.run_until_parked();
        visual.simulate_input("!"); visual.run_until_parked();
        assert_eq!(automatic_title_name(entity, visual), "retained");
        visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), "first\nsecond!"));
    });
}

#[test]
fn automatic_title_scalar_saved_full_name_and_nul_collision_semantics() {
    for (path, label, expected) in [
        (Some(std::path::PathBuf::from("shared")), None, "new 1"),
        (Some(std::path::PathBuf::from("C:/neutral/shared")), None, "shared"),
        (None, Some(format!("SHARED\0{}", "hidden".repeat(20000))), "new 1"),
    ] {
        with_automatic_title(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.tabs[1].buffer.file_path = path.clone();
                w.tabs[1].buffer.new_label = label.clone().map(Into::into);
            }));
            automatic_title_paste(visual, "shared");
            assert_eq!(automatic_title_name(entity, visual), expected);
        });
    }
}

#[test]
fn automatic_title_scalar_surrogate_boundary_has_explicit_collision_and_persistence_policy() {
    let text = format!("{}😀tail", "a".repeat(62));
    let shown = format!("{}\u{fffd}", "a".repeat(62));
    for conflict in [false, true] {
        with_automatic_title(|entity, visual| {
            if conflict { visual.update(|_, cx| entity.update(cx, |w, _| w.tabs[1].buffer.new_label = Some(shown.clone().into()))); }
            automatic_title_paste(visual, &text);
            assert_eq!(automatic_title_name(entity, visual), if conflict { "new 1" } else { &shown });
            visual.update(|_, cx| {
                let w = entity.read(cx); let tab = &w.tabs[0];
                assert_eq!(tab.buffer.rope.to_string(), text);
                assert_eq!(tab.undo_stack.len(), 1);
                assert!(!tab.buffer.new_label_is_custom);
                let snapshot = w.session_snapshot(crate::session::BakPolicy::Full);
                let encoded = toml::to_string(&snapshot).expect("persist accepted label");
                let restored: crate::session::SessionManifest = toml::from_str(&encoded).expect("read accepted label");
                assert_eq!(restored.tabs[0].new_label.as_deref(), Some(if conflict { "new 1" } else { &shown }));
            });
        });
    }
}

#[test]
fn automatic_title_coalesced_source_and_raw_deletions_keep_each_notification() {
    for raw in [false, true] {
        for backspace in [false, true] {
            let original = if raw { "{\\rtf1\\ansi last}" } else { "last" };
            let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(original);
            if raw { buffer.rich_doc = Some(crate::rich_doc::rtf::open_bytes(original.as_bytes().to_vec()).expect("neutral RTF")); }
            buffer.new_label = Some("retained".into()); buffer.mark_saved();
            with_editor(buffer, |entity, visual| {
                let mut settings = crate::settings::snapshot(); settings.untitled_uses_first_line = true;
                let _preference = crate::settings::override_for_test(settings);
                let before = visual.update(|_, cx| entity.update(cx, |w, _| {
                    let at = original.find("last").expect("fixture target") + if backspace { 4 } else { 0 };
                    w.tabs[0].selection.collapse_to(at); w.tabs[0].buffer.raw_revision.clone()
                }));
                let remaining = if backspace { ["las", "la", "l", ""] } else { ["ast", "st", "t", ""] };
                for (index, text) in remaining.iter().enumerate() {
                    visual.simulate_keystrokes(if backspace { "backspace" } else { "delete" }); visual.run_until_parked();
                    let expected = if raw { format!("{{rtf1ansi {text}}}") }
                        else { if text.is_empty() { remaining[index - 1] } else { text }.to_string() };
                    assert_eq!(automatic_title_name(entity, visual), expected);
                    visual.update(|window, cx| {
                        let w = entity.read(cx); assert_eq!(w.tabs[0].buffer.rope.to_string(), original.replace("last", text));
                        assert_eq!(w.tabs[0].undo_stack.len(), 1); assert!(w.input_focus(window) == crate::tab::InputFocus::Editor);
                        assert!(w.tabs[0].buffer.dirty); assert!(!w.tabs[0].buffer.new_label_is_custom);
                    });
                }
                visual.update(|_, cx| {
                    let tab = &entity.read(cx).tabs[0];
                    let crate::buffer::UndoEntry::Group { ops, .. } = &tab.undo_stack[0] else { panic!("coalesced deletions"); };
                    assert_eq!(ops.len(), 4); assert!(ops.iter().all(|op| op.inserted.is_empty() && op.removed.len() == 1));
                });
                visual.dispatch_action(crate::actions::EditUndo); visual.run_until_parked();
                assert_eq!(automatic_title_name(entity, visual), if raw { "{rtf1ansi last}" } else { "last" });
                visual.update(|_, cx| {
                    let tab = &entity.read(cx).tabs[0]; assert_eq!(tab.buffer.rope.to_string(), original);
                    assert_eq!(tab.buffer.raw_revision, before); assert!(!tab.buffer.dirty);
                });
                visual.dispatch_action(crate::actions::EditRedo); visual.run_until_parked();
                assert_eq!(automatic_title_name(entity, visual), if raw { "{rtf1ansi }" } else if backspace { "l" } else { "t" });
                visual.update(|_, cx| assert_eq!(entity.read(cx).tabs[0].buffer.rope.to_string(), original.replace("last", "")));
            });
        }
    }
}
