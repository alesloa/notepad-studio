use super::super::syntax_refresh_tests::with_sources;
use crate::{session::{SessionManifest, SessionTab}, syntax::LanguageId};

fn entry(path: &std::path::Path, id: uuid::Uuid, language: LanguageId) -> SessionTab {
    let mut tab: SessionTab = toml::from_str(&format!("id = \"{id}\"" )).expect("entry");
    tab.path = Some(path.to_path_buf()); tab.language = Some(language); tab
}

#[test]
fn session_identity_actual_restore_duplicate_disk_ids_are_independent() {
    let dir = crate::persist::TempDir::new("session-identity");
    let first = dir.join("first.py"); let second = dir.join("second.rs");
    std::fs::write(&first, "def first():\n    return 1\n").expect("first");
    std::fs::write(&second, "fn second() {}\n").expect("second");
    let id = uuid::Uuid::new_v4();
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.apply_session(SessionManifest { tabs: vec![entry(&first, id, LanguageId::Python), entry(&second, id, LanguageId::Rust)], active: 1, ..Default::default() });
            assert_eq!(w.active, 1);
            assert_eq!(w.tabs[0].buffer.rope.to_string(), "def first():\n    return 1\n");
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "fn second() {}\n");
            w.hold_syntax_refresh(true); w.start_initial_syntax(cx);
        }));
        visual.run_until_parked();
        visual.update(|_, cx| {
            let w = entity.read(cx);
            let request = &w.syntax_refresh.held.as_ref().expect("held active worker").0;
            assert!(request.rope.is_instance(&w.tabs[1].buffer.rope), "later active duplicate must capture its own source");
            assert_eq!(request.language, LanguageId::Rust);
            assert_ne!(w.tabs[0].session_id, w.tabs[1].session_id, "backup identities must be independent");
        });
        visual.update(|_, cx| entity.update(cx, |w, cx| w.release_syntax_refresh(cx)));
        for _ in 0..50 { visual.cx.executor().advance_clock(std::time::Duration::from_millis(2)); visual.run_until_parked(); }
        visual.update(|_, cx| {
            let w = entity.read(cx);
            assert!(w.tabs.iter().all(|tab| !tab.initial_syntax_pending && !tab.buffer.highlights.is_empty()));
            assert_eq!(w.active, 1);
            assert_eq!(w.tabs[0].buffer.language, Some(LanguageId::Python));
            assert_eq!(w.tabs[1].buffer.language, Some(LanguageId::Rust));
        });
    });
}

#[test]
fn session_identity_actual_restore_duplicate_backup_ids_preserves_history() {
    use crate::buffer::{ContentRevision, EditOp, UndoEntry, serialize_undo_stack};
    let dir = crate::persist::TempDir::new("session-identity-backup");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let id = uuid::Uuid::new_v4(); let source = "fn recovered() {}\n";
    std::fs::write(dir.join(&format!("{id}.bak")), source).expect("backup");
    let history = vec![UndoEntry::Single(EditOp { byte_start: 0, removed: "fn prior() {}\n".into(), inserted: source.into(), sel_before_anchor: 0, sel_before_head: 0, before_revision: ContentRevision::default(), after_revision: ContentRevision::default() })];
    let bytes = serialize_undo_stack(&history);
    std::fs::write(dir.join(&format!("{id}.undo")), &bytes).expect("history");
    let mut saved = entry(&dir.join("missing.rs"), id, LanguageId::Rust); saved.dirty = true;
    saved.cursor_anchor = 3; saved.cursor_head = 8; saved.read_only = true;
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.apply_session(SessionManifest { tabs: vec![saved.clone(), saved.clone()], active: 1, ..Default::default() });
            for tab in &w.tabs {
                assert_eq!(tab.buffer.rope.to_string(), source);
                assert!(tab.buffer.dirty && tab.buffer.read_only);
                assert_eq!(tab.selection.anchor, 3); assert_eq!(tab.selection.head, 8);
                assert_eq!(serialize_undo_stack(&tab.undo_stack), bytes);
            }
            assert_ne!(w.tabs[0].session_id, w.tabs[1].session_id, "resolve original backup before assigning fresh independent identity");
        }));
    });
}

#[test]
fn session_identity_actual_restore_closed_only_is_recoverable() {
    let dir = crate::persist::TempDir::new("session-closed-only");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let id = uuid::Uuid::new_v4();
    std::fs::write(dir.join(&format!("{id}.bak")), "closed recovered source").expect("backup");
    let closed = toml::from_str(&format!("id = \"{id}\"\ndirty = true\nnew_label = \"recovered\"" )).expect("closed entry");
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.apply_session(SessionManifest { tabs: vec![entry(&dir.join("missing.rs"), uuid::Uuid::new_v4(), LanguageId::Rust)], closed_tabs: vec![closed], ..Default::default() });
            assert_eq!(w.closed_tabs.len(), 1, "missing open file must not discard recovered closed history");
            assert_eq!(w.closed_tabs[0].buffer.rope.to_string(), "closed recovered source");
            assert!(w.closed_tabs[0].buffer.dirty);
        }));
    });
}


#[test]
fn session_identity_actual_restore_undo_and_full_writer_keep_independent_backups() {
    use crate::buffer::{ContentRevision, EditOp, UndoEntry, serialize_undo_stack};
    let dir = crate::persist::TempDir::new("session-independent-save");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let id = uuid::Uuid::new_v4(); let source = "fn recovered() {}\n";
    std::fs::write(dir.join(&format!("{id}.bak")), source).expect("backup");
    let history = vec![UndoEntry::Single(EditOp { byte_start: 0, removed: "fn prior() {}\n".into(), inserted: source.into(), sel_before_anchor: 0, sel_before_head: 0, before_revision: ContentRevision::default(), after_revision: ContentRevision::default() })];
    std::fs::write(dir.join(&format!("{id}.undo")), serialize_undo_stack(&history)).expect("history");
    let mut saved = entry(&dir.join("missing.rs"), id, LanguageId::Rust); saved.dirty = true;
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.apply_session(SessionManifest { tabs: vec![saved.clone(), saved], active: 1, ..Default::default() });
        }));
        visual.dispatch_action(crate::actions::EditUndo);
        visual.run_until_parked();
        visual.update(|_, cx| entity.update(cx, |w, _| {
            assert_eq!(w.tabs[0].buffer.rope.to_string(), source);
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "fn prior() {}\n");
            let ids = [w.tabs[0].session_id, w.tabs[1].session_id];
            let manifest_path = dir.join("session.toml");
            crate::session::write_csv_fixture(w.session_snapshot(crate::session::BakPolicy::Full), &manifest_path, dir.path()).expect("actual background writer");
            assert_eq!(std::fs::read_to_string(dir.join(&format!("{}.bak", ids[0]))).expect("first backup"), source);
            assert_eq!(std::fs::read_to_string(dir.join(&format!("{}.bak", ids[1]))).expect("second backup"), "fn prior() {}\n");
            let manifest = toml::from_str(&std::fs::read_to_string(manifest_path).expect("manifest")).expect("decode");
            w.apply_session(manifest);
            assert_eq!([w.tabs[0].session_id, w.tabs[1].session_id], ids);
            assert_eq!(w.tabs[0].buffer.rope.to_string(), source);
            assert_eq!(w.tabs[1].buffer.rope.to_string(), "fn prior() {}\n");
            assert_eq!(w.active, 1);
        }));
    });
}

#[test]
fn session_identity_actual_restore_clean_metadata_and_savepoints_survive() {
    use crate::text::encoding::Encoding;
    let dir = crate::persist::TempDir::new("session-identity-metadata");
    let first = dir.join("first.rs"); let second = dir.join("second.rs");
    let text = "fn caf\u{e9}() { let value = \"\u{4e2d}\u{1f600}\"; }\r\n";
    std::fs::write(&first, crate::text::encoding::encode(text, Encoding::Utf8Bom)).expect("BOM");
    std::fs::write(&second, crate::text::encoding::encode(text, Encoding::Utf16Be)).expect("UTF16");
    let id = uuid::Uuid::new_v4();
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            let mut first_entry = entry(&first, id, LanguageId::Rust);
            first_entry.cursor_anchor = 7; first_entry.cursor_head = 8;
            first_entry.read_only = true; first_entry.scroll_top = 27.; first_entry.scroll_left = 13.; first_entry.tab_color = Some(3);
            let mut second_entry = first_entry.clone(); second_entry.path = Some(second.clone());
            w.apply_session(SessionManifest { tabs: vec![first_entry, second_entry], active: 1, ..Default::default() });
            assert_ne!(w.tabs[0].session_id, w.tabs[1].session_id);
            for (tab, encoding) in w.tabs.iter().zip([Encoding::Utf8Bom, Encoding::Utf16Be]) {
                assert_eq!(tab.buffer.encoding, encoding); assert!(!tab.buffer.dirty);
                assert!(tab.buffer.savepoint.is_some()); assert!(tab.buffer.read_only);
                assert_eq!(tab.buffer.rope.to_string(), text.replace("\r\n", "\n"));
                assert_eq!(tab.selection.anchor, 6); assert_eq!(tab.selection.head, 8); // mid-byte anchor clamps before e-acute
                assert_eq!(f32::from(tab.scroll_top), 27.); assert_eq!(f32::from(tab.scroll_left), 13.);
                assert_eq!(tab.buffer.tab_color, Some(3));
            }
        }));
    });
}

#[test]
fn session_identity_actual_restore_raw_and_csv_recovery_use_original_paths() {
    let dir = crate::persist::TempDir::new("session-identity-recovery");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let raw_id = uuid::Uuid::new_v4(); let csv_id = uuid::Uuid::new_v4(); let basis = uuid::Uuid::new_v4();
    let source = "a,b\n1,2\n";
    std::fs::write(dir.join(&format!("{raw_id}.bak")), "raw recovered text").expect("raw backup");
    std::fs::write(crate::session::csv_basis_path_in(dir.path(), basis), source).expect("CSV basis");
    std::fs::write(dir.join(&format!("{csv_id}.bak")), "wrong stale source").expect("stale fallback");
    let mut raw = entry(&dir.join("missing.docx"), raw_id, LanguageId::Rust); raw.rich_backup = true; raw.dirty = true;
    let mut csv = entry(&dir.join("missing.csv"), csv_id, LanguageId::Rust); csv.dirty = true;
    csv.csv_draft = Some(crate::session::CsvDraft { value: "unsaved cell".into(), source_matches: true, backup_id: Some(basis), source_digest: Some(crate::session::csv_source_digest(source)), ..Default::default() });
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.apply_session(SessionManifest { tabs: vec![raw.clone(), raw, csv.clone(), csv], ..Default::default() });
            let ids: std::collections::BTreeSet<_> = w.tabs.iter().map(|tab| tab.session_id).collect(); assert_eq!(ids.len(), 4);
            for tab in &w.tabs[..2] {
                assert_eq!(tab.buffer.rope.to_string(), "raw recovered text"); assert!(tab.buffer.file_path.is_none());
                assert!(tab.buffer.dirty && tab.buffer.rich_doc.is_none());
            }
            for tab in &w.tabs[2..] {
                assert_eq!(tab.buffer.rope.to_string(), source);
                let draft = tab.csv_draft.as_ref().expect("draft");
                assert_eq!(draft.value, "unsaved cell"); assert_eq!(draft.backup_id, Some(basis)); assert!(draft.source_matches);
            }
            let status = w.status_message.as_deref().expect("status");
            assert!(status.contains("recovered raw source for 2 document(s)")); assert!(status.contains("repaired 2 duplicate document ID(s)"));
        }));
    });
}

#[test]
fn session_identity_actual_restore_closed_only_startup_blank_keeps_one_welcome_and_view() {
    let dir = crate::persist::TempDir::new("session-welcome-view");
    let _root = crate::session::override_restore_backup_root(dir.path().to_path_buf());
    let id = uuid::Uuid::new_v4();
    std::fs::write(dir.join(&format!("{id}.bak")), "closed source").expect("backup");
    let closed: crate::session::SessionClosedTab = toml::from_str(&format!("id = \"{id}\"\ndirty = true" )).expect("closed");
    with_sources(|entity, visual| {
        let _settings = crate::settings::override_for_test(crate::settings::Settings { startup_open_blank: true, ..Default::default() });
        visual.update(|_, cx| entity.update(cx, |w, cx| {
            w.tabs = vec![crate::tab::Tab::empty()]; w.active = 0;
            let welcome = w.tabs[0].session_id;
            w.apply_session(SessionManifest { tabs: vec![entry(&dir.join("missing.rs"), uuid::Uuid::new_v4(), LanguageId::Rust)], closed_tabs: vec![closed], theme: Some(crate::theme::ThemeMode::Light), show_whitespace: true, word_wrap: true, ..Default::default() });
            assert_eq!(w.tabs.len(), 1); assert_eq!(w.tabs[0].session_id, welcome); assert_eq!(w.active, 0);
            assert_eq!(w.closed_tabs.len(), 1); assert_eq!(w.theme_mode, crate::theme::ThemeMode::Light);
            assert!(w.show_whitespace && w.word_wrap); assert!(!w.tabs[0].initial_syntax_pending);
            assert!(w.status_message.as_deref().expect("status").contains("1 couldn't be reopened"));
            w.start_initial_syntax(cx); assert_eq!(w.syntax_refresh.started, 0);
        }));
    });
}

#[test]
fn session_identity_actual_restore_active_survivor_keeps_original_document() {
    let dir = crate::persist::TempDir::new("session-active-survivor");
    let active = dir.join("active.py"); let last = dir.join("last.rs");
    std::fs::write(&active, "def active():\n    return 1\n").unwrap();
    std::fs::write(&last, "fn last() {}\n").unwrap();
    with_sources(|entity, visual| {
        visual.update(|_, cx| entity.update(cx, |w, _| {
            w.apply_session(SessionManifest { tabs: vec![
                entry(&dir.join("missing.rs"), uuid::Uuid::new_v4(), LanguageId::Rust),
                entry(&active, uuid::Uuid::new_v4(), LanguageId::Python),
                entry(&last, uuid::Uuid::new_v4(), LanguageId::Rust)], active: 1, ..Default::default() });
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[w.active].buffer.file_path.as_ref(), Some(&active), "skipping an earlier file must retain the selected surviving document");
        }));
    });
}

#[test]
fn session_identity_actual_restore_active_first_last_and_skipped_fallback() {
    let dir = crate::persist::TempDir::new("session-active-controls");
    let first = dir.join("first.rs"); let last = dir.join("last.rs");
    std::fs::write(&first, "fn first() {}\n").unwrap();
    std::fs::write(&last, "fn last() {}\n").unwrap();
    // Selected first and last survive a skipped middle; a missing selected
    // entry keeps the old clamped-index fallback onto the last survivor.
    for (active, expected) in [(0, &first), (2, &last), (1, &last)] {
        with_sources(|entity, visual| {
            visual.update(|_, cx| entity.update(cx, |w, _| {
                w.apply_session(SessionManifest { tabs: vec![
                    entry(&first, uuid::Uuid::new_v4(), LanguageId::Rust),
                    entry(&dir.join("missing.rs"), uuid::Uuid::new_v4(), LanguageId::Rust),
                    entry(&last, uuid::Uuid::new_v4(), LanguageId::Rust)], active, ..Default::default() });
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(w.tabs[w.active].buffer.file_path.as_ref(), Some(expected));
            }));
        });
    }
}
