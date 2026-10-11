use gpui::{Context, EntityInputHandler, Window};
use crate::session::{BakPolicy, SessionManifest, RestoreOutcome};
use crate::text::encoding::Encoding;
use crate::workspace::Workspace;
use crate::ui::csv_doc::actions::{ensure_model, open_cell_editor, open_header_editor, commit_cell_edit, cancel_cell_edit};
use crate::ui::csv_doc::actions::savepoint_tests::with_csv;

fn press_keys(visual: &mut gpui::VisualTestContext, keys: &str) {
    let keys = keys.split_whitespace().map(|key| {
        let action = match key {
            "ctrl-a" => "EditSelectAll", "ctrl-s" => "FileSave",
            "ctrl-c" => "EditCopy", "ctrl-x" => "EditCut", "ctrl-v" => "EditPaste",
            "end" => "MoveLineEnd", _ => return key,
        };
        let entry = crate::shortcuts::catalog().iter().find(|entry| entry.id == action).unwrap();
        crate::shortcuts::platform_default_keys(entry)[0]
    }).collect::<Vec<_>>().join(" ");
    visual.simulate_keystrokes(&keys);
}

fn type_draft(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>, value: &str) {
    let model = w.csv_models.get_mut(&w.tabs[w.active].session_id).unwrap();
    let input = model.cell_input.as_mut().unwrap();
    input.anchor = 0;
    input.cursor = input.buffer.len();
    w.replace_text_in_range(None, value, window, cx);
}

fn save_capture(w: &Workspace, path: &std::path::Path) -> (SessionManifest, std::path::PathBuf) {
    let dir = path.parent().unwrap().join("recovery");
    std::fs::create_dir_all(&dir).unwrap();
    let snapshot = w.session_snapshot(BakPolicy::Full);
    crate::session::write_csv_fixture(snapshot, &dir.join("session.toml"), &dir).unwrap();
    let manifest = toml::from_str(&std::fs::read_to_string(dir.join("session.toml")).unwrap()).unwrap();
    (manifest, dir)
}

fn reopen(w: &mut Workspace, entry: &crate::session::SessionTab, dir: &std::path::Path, window: &mut Window, cx: &mut Context<Workspace>) {
    let RestoreOutcome::Restored(tab) = crate::session::restore_csv_fixture(entry, dir) else { panic!("draft lost"); };
    w.csv_models.clear();
    w.tabs = vec![tab];
    w.active = 0;
    ensure_model(w, window, cx);
    // Explicitly select the recovered input in this empty-window fixture.
    let handle = w.csv_models.get(&w.tabs[w.active].session_id).unwrap().cell_input.as_ref().unwrap().focus_handle.clone();
    window.focus(&handle, cx);
}

fn editor(w: &Workspace) -> &str {
    &w.csv_models.get(&w.tabs[w.active].session_id).unwrap().cell_input.as_ref().unwrap().buffer
}

#[test]
fn csv_recovery_clean_raw_and_uncommitted_unicode_survive_actual_capture_restore() {
    for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        with_csv(encoding, |w, window, cx, path| {
            let disk = std::fs::read(path).unwrap();
            let raw = w.buffer().rope.to_string();
            open_cell_editor(w, 0, 0, window, cx);
            let before_generation = w.autosave_generation;
            type_draft(w, window, cx, "draft 中😀");
            assert!(w.autosave_generation > before_generation);
            assert!(!w.buffer().dirty);
            let (manifest, dir) = save_capture(w, path);
            assert!(manifest.tabs[0].dirty);
            let draft = manifest.tabs[0].csv_draft.as_ref().unwrap();
            assert_eq!(draft.value, "draft 中😀");
            assert_eq!(std::fs::read_to_string(crate::session::csv_basis_path_in(&dir, draft.backup_id.unwrap())).unwrap(), raw);
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert_eq!(w.buffer().rope.to_string(), raw);
            assert_eq!(editor(w), "draft 中😀");
            assert!(w.buffer().dirty, "recovered draft has no proven saved marker");
            assert_eq!(std::fs::read(path).unwrap(), disk);
            w.save_active(&crate::actions::FileSave, window, cx);
            assert!(crate::io::read_buffer(path).unwrap().rope.to_string().contains("draft 中😀"));
            assert!(!w.buffer().dirty);
        });
    }
}

#[test]
fn csv_recovery_read_only_and_stale_drafts_remain_editable_without_source_changes() {
    for stale in [false, true] {
        with_csv(Encoding::Utf16Le, |w, window, cx, path| {
            open_cell_editor(w, 0, 0, window, cx);
            type_draft(w, window, cx, "draft 中😀");
            if stale { w.replace_byte_range(0..0, "changed,source\n", cx); }
            else { w.active_tab_mut().buffer.read_only = true; }
            let raw = w.buffer().rope.to_string();
            let (manifest, dir) = save_capture(w, path);
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert_eq!(editor(w), "draft 中😀");
            assert!(!commit_cell_edit(w, cx));
            assert_eq!(w.buffer().rope.to_string(), raw);
            type_draft(w, window, cx, "still editable 😀");
            assert_eq!(editor(w), "still editable 😀");
            assert!(w.status_message.is_some());
        });
    }
}

#[test]
fn csv_recovery_uses_physical_coordinates_after_display_reordering() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
        let model = w.csv_models.get_mut(&w.tabs[w.active].session_id).unwrap();
        model.row_order.swap(0, 1);
        model.col_order.swap(0, 1);
        open_cell_editor(w, 0, 0, window, cx);
        type_draft(w, window, cx, "physical target");
        let (manifest, dir) = save_capture(w, path);
        let draft = manifest.tabs[0].csv_draft.as_ref().unwrap();
        assert_eq!((draft.row, draft.col), (Some(1), Some(1)));
        reopen(w, &manifest.tabs[0], &dir, window, cx);
        assert!(commit_cell_edit(w, cx));
        let model = w.csv_models.get(&w.tabs[w.active].session_id).unwrap();
        assert_eq!(model.cell(1, 1), "physical target");
        assert_eq!(model.cell(0, 0), "caf\u{00E9}");
    });
}

#[test]
fn csv_recovery_preserves_header_mode_and_custom_delimiter() {
    for header in [false, true] {
        with_csv(Encoding::Utf16Be, |w, window, cx, path| {
            let len = w.buffer().rope.len_bytes();
            w.replace_byte_range(0..len, "label;value\ncaf\u{00E9};1\n中😀;2\n", cx);
            let model = w.csv_models.get_mut(&w.tabs[w.active].session_id).unwrap();
            model.set_delimiter(b';');
            model.has_header = header;
            ensure_model(w, window, cx);
            w.write_active_to_disk().unwrap();
            if header { open_header_editor(w, 0, window, cx); }
            else { open_cell_editor(w, 0, 0, window, cx); }
            type_draft(w, window, cx, "header 😀");
            let (manifest, dir) = save_capture(w, path);
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert!(commit_cell_edit(w, cx));
            let model = w.csv_models.get(&w.tabs[w.active].session_id).unwrap();
            assert_eq!(model.delimiter, b';');
            assert_eq!(model.has_header, header);
            assert_eq!(if header { model.header(0) } else { model.cell(0, 0) }, "header 😀");
        });
    }
}

#[test]
fn csv_recovery_invalid_or_missing_coordinates_never_drop_text_or_edit_source() {
    for metadata in ["row = -1\ncol = 0", "row = 'bad'\ncol = 0", "col = 0", "row = 0\ncol = 999999"] {
        with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
            open_cell_editor(w, 0, 0, window, cx);
            type_draft(w, window, cx, "draft 中😀");
            let raw = w.buffer().rope.to_string();
            let (mut manifest, dir) = save_capture(w, path);
            let original = manifest.tabs[0].csv_draft.take().unwrap();
            let mut draft: crate::session::CsvDraft = toml::from_str(metadata).unwrap();
            draft.value = original.value;
            draft.delimiter = original.delimiter;
            draft.has_header = original.has_header;
            draft.backup_id = original.backup_id;
            draft.source_digest = original.source_digest;
            draft.source_matches = true;
            manifest.tabs[0].csv_draft = Some(draft);
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert_eq!(editor(w), "draft 中😀");
            assert!(w.csv_models.get(&w.tabs[w.active].session_id).unwrap().draft_needs_strip());
            assert!(!commit_cell_edit(w, cx));
            assert_eq!(w.buffer().rope.to_string(), raw);
            assert!(w.status_message.as_ref().is_some_and(|message| message.contains("copy") || message.contains("cancel")));
        });
    }
}

#[test]
fn csv_recovery_bad_digest_and_missing_basis_preserve_the_draft() {
    for missing in [false, true] {
        with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
            open_cell_editor(w, 0, 0, window, cx);
            type_draft(w, window, cx, "draft 中😀");
            let (mut manifest, dir) = save_capture(w, path);
            let draft = manifest.tabs[0].csv_draft.as_mut().unwrap();
            if missing { std::fs::remove_file(crate::session::csv_basis_path_in(&dir, draft.backup_id.unwrap())).unwrap(); }
            else { draft.source_digest = Some("invalid".into()); }
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert_eq!(editor(w), "draft 中😀");
            assert!(!commit_cell_edit(w, cx));
        });
    }
}

#[test]
fn csv_recovery_cancel_removes_draft_and_only_reaps_basis_after_publication() {
    with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
        open_cell_editor(w, 0, 0, window, cx);
        type_draft(w, window, cx, "draft 中😀");
        let (manifest, dir) = save_capture(w, path);
        let basis = crate::session::csv_basis_path_in(&dir, manifest.tabs[0].csv_draft.as_ref().unwrap().backup_id.unwrap());
        assert!(basis.is_file());
        cancel_cell_edit(w, cx);
        let snapshot = w.session_snapshot(BakPolicy::Full);
        assert!(snapshot.tabs[0].csv_draft.is_none());
        assert!(basis.is_file());
        crate::session::write_csv_fixture(snapshot, &dir.join("session.toml"), &dir).unwrap();
        let manifest: SessionManifest = toml::from_str(&std::fs::read_to_string(dir.join("session.toml")).unwrap()).unwrap();
        assert!(manifest.tabs[0].csv_draft.is_none());
        assert!(!basis.exists());
    });
}

#[test]
fn csv_recovery_legacy_session_entry_has_no_draft() {
    let entry: crate::session::SessionTab = toml::from_str(&format!("id = '{}'", uuid::Uuid::new_v4())).unwrap();
    assert!(entry.csv_draft.is_none());
}

fn with_rendered_csv(test: impl FnOnce(&gpui::Entity<Workspace>, &mut gpui::VisualTestContext, &std::path::Path)) {
    use gpui::VisualContext;
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        toolbar_hidden: true, folder_panel_visible: false, ..Default::default()
    });
    let dir = crate::persist::TempDir::new("csv-recovery-frame");
    let path = dir.join("source.csv");
    std::fs::write(&path, "label,value\ncaf\u{00E9},1\n中😀,2\n").unwrap();
    let buffer = crate::io::read_buffer(&path).unwrap();
    let mut cx = gpui::TestAppContext::single();
    cx.update(|cx| crate::shortcuts::register_catalog(cx, &Default::default()));
    let (entity, visual) = cx.add_window_view(|_, cx| {
        let mut w = Workspace::new(cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), 1., gpui::WindowAppearance::Dark);
        w.active_tab_mut().buffer = buffer;
        w.active_tab_mut().view_mode = crate::tab::TabViewMode::CsvPreview;
        w.window_closed = true;
        w
    });
    visual.update_window_entity(&entity, |w, window, cx| {
        window.activate_window();
        ensure_model(w, window, cx);
        open_cell_editor(w, 0, 0, window, cx);
    });
    visual.run_until_parked();
    press_keys(visual, "ctrl-a");
    assert!(visual.debug_bounds("csv-cell-input").is_some());
    visual.simulate_input("draft 中😀");
    visual.update(|_, cx| assert_eq!(editor(entity.read(cx)), "draft 中😀"));
    test(&entity, visual, &path);
    cx.quit();
}

#[test]
fn csv_recovery_real_frame_save_keeps_grid_actions_usable() {
    with_rendered_csv(|entity, visual, path| {
        press_keys(visual, "ctrl-s ctrl-a");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(crate::io::read_buffer(path).unwrap().rope.to_string().contains("draft 中😀"));
            let model = w.csv_models.get(&w.tabs[w.active].session_id).unwrap();
            assert!(model.cell_input.is_none());
            assert!(matches!(w.input_focus(window), crate::tab::InputFocus::CsvGrid));
            assert_eq!(model.selected_rows.len(), model.row_count());
        });
    });
}

#[test]
fn csv_recovery_real_frame_enter_and_tab_keep_grid_actions_usable() {
    for key in ["enter", "tab"] {
        with_rendered_csv(|entity, visual, _| {
            press_keys(visual, key);
            press_keys(visual, "ctrl-a");
            visual.update(|window, cx| {
                let w = entity.read(cx);
                let model = w.csv_models.get(&w.tabs[w.active].session_id).unwrap();
                assert_eq!(model.cell(0, 0), "draft 中😀");
                assert!(matches!(w.input_focus(window), crate::tab::InputFocus::CsvGrid));
                assert_eq!(model.selected_rows.len(), model.row_count());
            });
        });
    }
}

#[test]
fn csv_recovery_real_frame_tab_switch_routes_text_to_the_visible_editor() {
    use gpui::VisualContext;
    with_rendered_csv(|entity, visual, _| {
        visual.update_window_entity(entity, |w, _, cx| {
            w.tabs.push(crate::tab::Tab::empty());
            w.set_active_tab(1, cx);
        });
        visual.run_until_parked();
        visual.simulate_input("visible source");
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert_eq!(w.buffer().rope.to_string(), "visible source");
            assert!(w.tabs[0].buffer.rope.to_string().contains("draft 中😀"));
        });
    });
}

#[test]
fn csv_recovery_real_frame_escape_discards_only_the_draft() {
    with_rendered_csv(|entity, visual, _| {
        let original = visual.update(|_, cx| entity.read(cx).buffer().rope.to_string());
        press_keys(visual, "escape");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            assert!(w.csv_models.get(&w.tabs[w.active].session_id).unwrap().cell_input.is_none());
            assert_eq!(w.buffer().rope.to_string(), original);
            assert!(matches!(w.input_focus(window), crate::tab::InputFocus::CsvGrid));
            assert!(w.session_snapshot(BakPolicy::Full).tabs[0].csv_draft.is_none());
        });
    });
}

#[test]
fn csv_recovery_damaged_metadata_preserves_the_draft_text() {
    for metadata in ["cursor = 'bad'", "anchor = -1", "delimiter = 999", "has_header = 'bad'", "is_header = 99", "source_matches = []", "source_digest = 123"] {
        let entry: crate::session::SessionTab = toml::from_str(&format!("id = '{}'\n[csv_draft]\nvalue = 'draft 中😀'\nrow = 0\ncol = 0\n{metadata}", uuid::Uuid::new_v4())).unwrap();
        assert_eq!(entry.csv_draft.as_ref().unwrap().value, "draft 中😀");
        assert!(!entry.csv_draft.as_ref().unwrap().source_matches);
    }
}

#[test]
fn csv_recovery_actual_edit_actions_schedule_autosave_and_cancel_capture() {
    use gpui::VisualContext;
    with_rendered_csv(|entity, visual, _| {
        for action in 0..4 {
            let before = visual.update_window_entity(entity, |w, window, cx| {
                type_draft(w, window, cx, "draft 中😀");
                let input = w.csv_models.get_mut(&w.tabs[w.active].session_id).unwrap().cell_input.as_mut().unwrap();
                input.anchor = 0; input.cursor = input.buffer.len();
                if action == 1 { cx.write_to_clipboard(gpui::ClipboardItem::new_string("paste 😀".into())); }
                w.autosave_generation
            });
            press_keys(visual, ["ctrl-x", "ctrl-v", "backspace", "delete"][action]);
            visual.update(|_, cx| assert!(entity.read(cx).autosave_generation > before));
            visual.run_until_parked();
            visual.update(|_, cx| {
                let w = entity.read(cx);
                let snapshot = w.session_snapshot(BakPolicy::Full);
                assert_eq!(snapshot.tabs[0].csv_draft.as_ref().unwrap().value, editor(w));
            });
        }
        press_keys(visual, "escape");
        visual.run_until_parked();
        visual.update(|_, cx| assert!(entity.read(cx).session_snapshot(BakPolicy::Full).tabs[0].csv_draft.is_none()));
    });
}

#[test]
fn csv_recovery_real_frame_large_detached_draft_is_visible_editable_and_lossless() {
    use gpui::VisualContext;
    with_rendered_csv(|entity, visual, path| {
        let value = "中😀\r\nquoted \"value\" ".repeat(60_000);
        let raw = visual.update_window_entity(entity, |w, _, cx| {
            let model = w.csv_models.get_mut(&w.tabs[w.active].session_id).unwrap();
            let input = model.cell_input.as_mut().unwrap();
            input.buffer = value.clone(); input.cursor = input.buffer.len(); input.anchor = input.cursor;
            model.editing_cell = None;
            model.detached_draft = Some(crate::session::CsvDraft { refusal: Some("Copy or cancel this recovered draft".into()), ..Default::default() });
            w.active_tab_mut().buffer.file_path = None;
            cx.notify();
            w.buffer().rope.to_string()
        });
        press_keys(visual, "end");
        let bounds = visual.debug_bounds("csv-cell-input").unwrap();
        visual.simulate_click(gpui::point(bounds.right() - gpui::px(20.), bounds.center().y), gpui::Modifiers::default());
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let input = w.csv_models.get(&w.tabs[w.active].session_id).unwrap().cell_input.as_ref().unwrap();
            assert!(input.cursor > value.len() / 2, "mouse offsets stay in the displayed suffix");
            assert!(input.buffer.is_char_boundary(input.cursor));
        });
        press_keys(visual, "ctrl-a ctrl-c");
        assert!(visual.debug_bounds("csv-recovered-draft").is_some());
        assert!(visual.debug_bounds("csv-cell-input").is_some());
        visual.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), value));
        press_keys(visual, "end");
        visual.simulate_input("tail 😀");
        visual.update_window_entity(entity, |w, window, cx| {
            assert_eq!(editor(w), format!("{value}tail 😀"));
            assert!(!commit_cell_edit(w, cx));
            assert_eq!(w.buffer().rope.to_string(), raw);
            let snapshot = w.session_snapshot(BakPolicy::Full);
            assert_eq!(snapshot.tabs[0].csv_draft.as_ref().unwrap().value, format!("{value}tail 😀"));
            let (manifest, dir) = save_capture(w, path);
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert_eq!(editor(w), format!("{value}tail 😀"));
            assert_eq!(w.buffer().rope.to_string(), raw);
            assert!(w.csv_models.get(&w.tabs[w.active].session_id).unwrap().draft_needs_strip());
        });
    });
}

#[test]
fn csv_recovery_real_frame_unchanged_commit_keeps_grid_focus() {
    with_rendered_csv(|entity, visual, _| {
        press_keys(visual, "ctrl-a");
        visual.simulate_input("caf\u{00E9}");
        press_keys(visual, "enter ctrl-a");
        visual.update(|window, cx| {
            let w = entity.read(cx);
            let model = w.csv_models.get(&w.tabs[w.active].session_id).unwrap();
            assert!(model.cell_input.is_none());
            assert!(matches!(w.input_focus(window), crate::tab::InputFocus::CsvGrid));
            assert_eq!(model.selected_rows.len(), model.row_count());
        });
    });
}

#[test]
fn csv_recovery_real_frame_commit_preserves_newer_field_focus_and_inactive_window() {
    use gpui::VisualContext;
    for inactive in [false, true] {
        with_rendered_csv(|entity, visual, _| {
            if inactive { visual.deactivate_window(); }
            visual.update_window_entity(entity, |w, window, cx| {
                assert!(commit_cell_edit(w, cx));
                if !inactive { w.goto_state.open = true; window.focus(&w.goto_focus, cx); }
                cx.notify();
            });
            visual.run_until_parked();
            visual.update(|window, cx| {
                let w = entity.read(cx);
                if inactive {
                    assert!(!window.is_window_active());
                    assert!(w.csv_models.get(&w.tabs[w.active].session_id).unwrap().retired_cell_focus.is_some());
                } else { assert!(w.goto_focus.is_focused(window)); }
            });
            if inactive {
                visual.update(|window, _| window.activate_window());
                press_keys(visual, "ctrl-a");
                visual.update(|window, cx| assert!(matches!(entity.read(cx).input_focus(window), crate::tab::InputFocus::CsvGrid)));
            }
        });
    }
}

#[test]
fn csv_recovery_unsupported_or_native_destinations_are_detached_untitled_drafts() {
    for (extension, rich_backup) in [("docx", false), ("txt", false), ("bin", false), ("csv", true)] {
        with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
            open_cell_editor(w, 0, 0, window, cx);
            type_draft(w, window, cx, "draft 中😀");
            let raw = w.buffer().rope.to_string();
            let (mut manifest, dir) = save_capture(w, path);
            let original = path.parent().unwrap().join(format!("destination.{extension}"));
            let native_bytes = b"PK native document bytes must stay untouched";
            std::fs::write(&original, native_bytes).unwrap();
            manifest.tabs[0].path = Some(original.clone());
            manifest.tabs[0].rich_backup = rich_backup;
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert!(w.buffer().file_path.is_none(), "{extension}, rich={rich_backup}");
            assert_eq!(w.buffer().rope.to_string(), raw);
            assert_eq!(editor(w), "draft 中😀");
            assert!(w.csv_models.get(&w.tabs[w.active].session_id).unwrap().draft_needs_strip());
            assert!(!commit_cell_edit(w, cx));
            w.save_active(&crate::actions::FileSave, window, cx);
            assert_eq!(std::fs::read(&original).unwrap(), native_bytes);
            assert!(w.status_message.as_ref().is_some_and(|message| message.contains("copy") && message.contains("Save As")));
            cancel_cell_edit(w, cx);
            assert!(w.buffer().file_path.is_none());
        });
    }
}

#[test]
fn csv_recovery_csv_tsv_and_valid_untitled_drafts_keep_exact_source_binding() {
    for extension in [Some("csv"), Some("tsv"), None] {
        with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
            open_cell_editor(w, 0, 0, window, cx);
            type_draft(w, window, cx, "draft 中😀");
            let (mut manifest, dir) = save_capture(w, path);
            manifest.tabs[0].path = extension.map(|extension| path.with_extension(extension));
            reopen(w, &manifest.tabs[0], &dir, window, cx);
            assert_eq!(w.buffer().file_path, manifest.tabs[0].path);
            assert!(commit_cell_edit(w, cx));
            assert!(w.buffer().rope.to_string().contains("draft 中😀"));
        });
    }
}

#[test]
fn csv_recovery_real_debounce_captures_only_the_latest_draft_then_cancellation() {
    use gpui::VisualContext;
    with_rendered_csv(|entity, visual, _| {
        let (sink, captures) = std::sync::mpsc::channel();
        visual.update_window_entity(entity, |w, window, cx| {
            w.session_snapshot_sink = Some(sink);
            type_draft(w, window, cx, "first draft");
            type_draft(w, window, cx, "latest draft 中😀");
        });
        visual.run_until_parked();
        assert!(captures.try_recv().is_err());
        // Production autosave uses smol's wall-clock Timer, not GPUI's fake clock.
        std::thread::sleep(std::time::Duration::from_millis(350));
        visual.run_until_parked();
        let snapshot = captures.try_recv().unwrap();
        assert_eq!(snapshot.tabs[0].csv_draft.as_ref().unwrap().value, "latest draft 中😀");
        assert!(captures.try_recv().is_err(), "older generations did not capture");
        press_keys(visual, "escape");
        std::thread::sleep(std::time::Duration::from_millis(350));
        visual.run_until_parked();
        assert!(captures.try_recv().unwrap().tabs[0].csv_draft.is_none());
        assert!(captures.try_recv().is_err());
    });
}

#[test]
fn csv_recovery_missing_all_backups_preserves_draft_and_fallback_encoding() {
    for missing_file in [false, true] {
        with_csv(Encoding::Utf8Bom, |w, window, cx, path| {
            open_cell_editor(w, 0, 0, window, cx);
            type_draft(w, window, cx, "draft 中😀");
            let (manifest, dir) = save_capture(w, path);
            let entry = &manifest.tabs[0];
            std::fs::remove_file(crate::session::csv_basis_path_in(&dir, entry.csv_draft.as_ref().unwrap().backup_id.unwrap())).unwrap();
            std::fs::remove_file(dir.join(format!("{}.bak", entry.id))).unwrap();
            if missing_file { std::fs::remove_file(path).unwrap(); }
            else { std::fs::write(path, crate::text::encoding::encode("label,value\r\ncaf\u{00E9},8\r\n", Encoding::Utf16Be)).unwrap(); }
            reopen(w, entry, &dir, window, cx);
            assert_eq!(editor(w), "draft 中😀");
            assert!(!commit_cell_edit(w, cx));
            if missing_file {
                assert!(w.buffer().rope.len_bytes() == 0);
                assert!(w.csv_models.get(&w.tabs[w.active].session_id).unwrap().draft_needs_strip());
            } else {
                assert_eq!(w.buffer().encoding, Encoding::Utf16Be);
                assert_eq!(w.buffer().line_ending, crate::text::line_ending::LineEnding::Crlf);
                assert_eq!(w.buffer().rope.to_string(), "label,value\ncaf\u{00E9},8\n");
            }
        });
    }
}

#[test]
fn csv_recovery_rendering_safety_clamps_malformed_runtime_caret() {
    use gpui::{Element, IntoElement, VisualContext};
    with_rendered_csv(|entity, visual, _| {
        visual.update_window_entity(entity, |w, _, cx| {
            let input = w.csv_models.get_mut(&w.tabs[w.active].session_id).unwrap().cell_input.as_mut().unwrap();
            input.buffer = "中😀".into(); input.cursor = 1; input.anchor = 5;
            cx.notify();
        });
        visual.update(|window, cx| {
            let w = entity.read(cx);
            let input = w.csv_models.get(&w.tabs[w.active].session_id).unwrap().cell_input.as_ref().unwrap();
            let mut element = crate::ui::csv_doc::cell_input::cell_input_element(&w.theme(), entity.downgrade(), input.focus_handle.clone()).into_element();
            let prepaint = element.prepaint(None, None, gpui::Bounds::default(), &mut (), window, cx).unwrap();
            assert_eq!(prepaint.cursor, 0);
            assert_eq!(prepaint.anchor, 3);
        });
    });
}

#[test]
fn csv_recovery_rendering_safety_lazy_restore_preserves_newer_focus_and_inactive_window() {
    use gpui::VisualContext;
    for case in 0..3 {
        let inactive = case == 1;
        with_rendered_csv(|entity, visual, path| {
            let (manifest, dir) = visual.update_window_entity(entity, |w, _, _| save_capture(w, path));
            if inactive { visual.deactivate_window(); }
            visual.update_window_entity(entity, |w, window, cx| {
                let RestoreOutcome::Restored(tab) = crate::session::restore_csv_fixture(&manifest.tabs[0], &dir) else { panic!("missing recovery"); };
                w.csv_models.clear(); w.tabs = vec![tab]; w.active = 0;
                if case == 0 { w.goto_state.open = true; window.focus(&w.goto_focus, cx); }
                if case == 2 { window.focus(&w.focus_handle, cx); }
                ensure_model(w, window, cx);
                assert_eq!(editor(w), "draft 中😀");
                let handle = &w.csv_models.get(&w.tabs[w.active].session_id).unwrap().cell_input.as_ref().unwrap().focus_handle;
                assert_eq!(handle.is_focused(window), case == 2);
                if inactive { assert!(!window.is_window_active()); }
                else if case == 0 { assert!(w.goto_focus.is_focused(window)); }
            });
            visual.run_until_parked();
            assert!(visual.debug_bounds("csv-cell-input").is_some());
        });
    }
}
