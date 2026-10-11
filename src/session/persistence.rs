use super::{BakPolicy, SessionManifest, bak_dir, bak_path_in, manifest_path, undo_path_in};
use crate::buffer::{UndoEntry, serialize_undo_stack};
use crate::tab::{ClosedTab, Tab};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(crate) enum Backup {
    Write { id: Uuid, rope: ropey::Rope, undo: Vec<UndoEntry>, rich: Option<super::rich_backup::Capture> },
    Delete { id: Uuid },
    Keep { id: Uuid },
}

impl std::fmt::Debug for Backup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backup").field("id", &self.id()).finish_non_exhaustive()
    }
}

impl Backup {
    pub(super) fn id(&self) -> Uuid {
        match self { Self::Write { id, .. } | Self::Delete { id } | Self::Keep { id } => *id }
    }

    pub(super) fn capture(tab: &Tab, policy: BakPolicy) -> Self {
        let id = tab.session_id;
        match policy {
            BakPolicy::UntitledOnly if tab.buffer.file_path.is_some() => Self::Keep { id },
            _ if tab.buffer.dirty || tab.buffer.file_path.is_none() => Self::Write {
                id, rope: tab.buffer.rope.clone(), undo: tab.undo_stack.clone(),
                rich: tab.buffer.rich_doc.as_ref().map(super::rich_backup::Capture::new),
            },
            _ => Self::Delete { id },
        }
    }

    pub(super) fn raw(tab: &Tab) -> Self {
        Self::Write { id: tab.session_id, rope: tab.buffer.rope.clone(), undo: tab.undo_stack.clone(), rich: None }
    }

    pub(super) fn closed(tab: &ClosedTab) -> Self {
        Self::Write { id: tab.session_id, rope: tab.buffer.rope.clone(), undo: Vec::new(),
            rich: tab.buffer.rich_doc.as_ref().map(super::rich_backup::Capture::new) }
    }

    fn write_at(&self, dir: &Path) -> io::Result<()> {
        match self {
            Self::Write { id, rope, undo, rich } => {
                let source = rope.to_string();
                if let Some(rich) = rich {
                    rich.write(&super::rich_backup::path_in(dir, *id), &source)?;
                }
                crate::persist::atomic_write(&bak_path_in(dir, *id), source.as_bytes())?;
                let bytes = serialize_undo_stack(undo);
                if bytes.is_empty() {
                    remove_if_present(&undo_path_in(dir, *id))?;
                } else {
                    crate::persist::atomic_write(&undo_path_in(dir, *id), &bytes)?;
                }
            }
            Self::Delete { id } => {
                remove_if_present(&bak_path_in(dir, *id))?;
                remove_if_present(&undo_path_in(dir, *id))?;
                remove_if_present(&super::rich_backup::path_in(dir, *id))?;
            }
            Self::Keep { .. } => {}
        }
        Ok(())
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub(crate) struct PendingSave {
    generation: u128,
    manifest: SessionManifest,
    reap: bool,
}

pub(crate) fn prepare_save(window_id: Uuid, manifest: SessionManifest, policy: BakPolicy) -> io::Result<PendingSave> {
    let (generation, manifest) = super::windows::update(window_id, manifest)
        .ok_or_else(|| io::Error::other("session window lock is poisoned"))?;
    Ok(PendingSave { generation, manifest, reap: matches!(policy, BakPolicy::Full) })
}

#[derive(Default)]
struct Writer {
    committed: u128,
    written_backups: BTreeMap<Uuid, Arc<Backup>>,
    #[cfg(test)]
    fail_before_publish: bool,
}

impl Writer {
    fn write_at(&mut self, mut pending: PendingSave, path: &Path, dir: &Path) -> io::Result<()> {
        // Every job contains all windows' captured backups. A newer job may
        // finish first; an older one must then leave both bytes and metadata alone.
        if pending.generation < self.committed { return Ok(()); }
        std::fs::create_dir_all(dir)?;
        for backup in &pending.manifest.backups {
            if !matches!(backup.as_ref(), Backup::Write { .. }) { continue; }
            let id = backup.id();
            if self.written_backups.get(&id).is_some_and(|previous| Arc::ptr_eq(previous, backup)) {
                continue;
            }
            // A partial write must invalidate the cache before returning an
            // error, so the next job repairs both the body and undo file.
            self.written_backups.remove(&id);
            backup.write_at(dir)?;
            self.written_backups.insert(id, backup.clone());
        }
        for tab in &mut pending.manifest.tabs {
            let Some(draft) = tab.csv_draft.as_mut() else { continue; };
            let id = draft.backup_id.ok_or_else(|| io::Error::other("CSV recovery is missing its source backup identity"))?;
            let backup = pending.manifest.backups.iter().find(|backup| backup.id() == tab.id)
                .ok_or_else(|| io::Error::other("CSV recovery is missing its captured source"))?;
            let Backup::Write { rope, .. } = backup.as_ref() else {
                return Err(io::Error::other("CSV recovery source was not captured"));
            };
            let source = rope.to_string();
            let digest = super::csv_source_digest(&source);
            let basis_path = super::csv_basis_path_in(dir, id);
            match std::fs::read(&basis_path) {
                Ok(previous) if previous == source.as_bytes() => {}
                Ok(_) => return Err(io::Error::other("CSV recovery source identity already holds different bytes")),
                Err(err) if err.kind() == io::ErrorKind::NotFound => crate::persist::atomic_write(&basis_path, source.as_bytes())?,
                Err(err) => return Err(err),
            }
            draft.source_digest = Some(digest);
        }
        // Publishing the manifest first ensures a failed write cannot prune
        // files still owned by the preceding on-disk session.
        #[cfg(test)]
        if self.fail_before_publish { return Err(io::Error::other("injected manifest publication failure")); }
        super::write_manifest_at(&pending.manifest, path)?;
        self.committed = pending.generation;
        for backup in &pending.manifest.backups {
            if matches!(backup.as_ref(), Backup::Delete { .. }) {
                backup.write_at(dir)?;
                self.written_backups.remove(&backup.id());
            } else if matches!(backup.as_ref(), Backup::Write { rich: None, .. }) {
                // A recovered raw-source tab can share a UUID with an older
                // native backup. Remove that stale state only after publication.
                remove_if_present(&super::rich_backup::path_in(dir, backup.id()))?;
            }
        }
        if pending.reap {
            let bases: std::collections::BTreeSet<Uuid> = pending.manifest.tabs.iter()
                .filter_map(|tab| tab.csv_draft.as_ref().and_then(|draft| draft.backup_id)).collect();
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|ext| ext.to_str()) != Some("csv-basis") { continue; }
                    let Some(id) = path.file_stem().and_then(|stem| stem.to_str()).and_then(|stem| Uuid::parse_str(stem).ok()) else { continue; };
                    if !bases.contains(&id) { remove_if_present(&path)?; }
                }
            }
            super::reap_orphan_baks_at(&pending.manifest, dir);
            let referenced: std::collections::BTreeSet<Uuid> = pending.manifest.tabs.iter().map(|tab| tab.id)
                .chain(pending.manifest.closed_tabs.iter().map(|tab| tab.id)).collect();
            self.written_backups.retain(|id, _| referenced.contains(id));
        }
        Ok(())
    }
}

static WRITER: Mutex<Writer> = Mutex::new(Writer {
    committed: 0, written_backups: BTreeMap::new(),
    #[cfg(test)]
    fail_before_publish: false,
});

pub(crate) fn write_prepared(pending: PendingSave) -> io::Result<()> {
    let (Some(path), Some(dir)) = (manifest_path(), bak_dir()) else {
        return Err(io::Error::other("no config directory for session recovery"));
    };
    let mut writer = WRITER.lock().map_err(|_| io::Error::other("session writer lock is poisoned"))?;
    writer.write_at(pending, &path, &dir)
}

#[cfg(test)]
pub(crate) fn write_csv_fixture(manifest: SessionManifest, path: &Path, dir: &Path) -> io::Result<()> {
    Writer::default().write_at(PendingSave { generation: 1, manifest, reap: true }, path, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::TempDir;
    use crate::theme::ThemeMode;

    fn draft(text: &str) -> Tab {
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(text);
        tab.buffer.dirty = true;
        tab
    }

    fn pending(generation: u128, tabs: &[Tab]) -> PendingSave {
        PendingSave {
            generation,
            manifest: super::super::snapshot_workspace_with_policy(tabs, 0, &[], ThemeMode::Dark, false, false, &[], BakPolicy::Full),
            reap: true,
        }
    }

    #[test]
    fn csv_basis_survives_raw_replacement_then_backup_or_manifest_failure() {
        for fail_manifest in [false, true] {
            let dir = TempDir::new("csv-basis-failure");
            let path = dir.join("session.toml");
            let mut tab = draft("h\noriginal\n");
            tab.csv_draft = Some(super::super::CsvDraft {
                row: Some(0), col: Some(0), value: "old draft 中😀".into(),
                delimiter: Some(b','), has_header: true, source_matches: true, ..Default::default()
            });
            let id = tab.session_id;
            let mut writer = Writer::default();
            writer.write_at(pending(1, std::slice::from_ref(&tab)), &path, dir.path()).unwrap();
            let previous = std::fs::read(&path).unwrap();
            let manifest: SessionManifest = toml::from_str(std::str::from_utf8(&previous).unwrap()).unwrap();
            let entry = &manifest.tabs[0];
            let basis = super::super::csv_basis_path_in(dir.path(), entry.csv_draft.as_ref().unwrap().backup_id.unwrap());
            let old_basis = std::fs::read(&basis).unwrap();
            tab.buffer.rope = ropey::Rope::from_str("h\nnewer raw source\n");
            tab.csv_draft.as_mut().unwrap().value = "newer draft".into();
            let mut next = vec![tab];
            if fail_manifest { writer.fail_before_publish = true; }
            else {
                next.push(draft("blocked backup"));
                std::fs::create_dir(bak_path_in(dir.path(), next[1].session_id)).unwrap();
            }
            assert!(writer.write_at(pending(2, &next), &path, dir.path()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), previous);
            assert_eq!(std::fs::read(&basis).unwrap(), old_basis);
            assert_eq!(std::fs::read_to_string(bak_path_in(dir.path(), id)).unwrap(), "h\nnewer raw source\n");
            let super::super::RestoreOutcome::Restored(restored) = super::super::restore_tab_in(entry, dir.path()) else { panic!("lost draft"); };
            assert_eq!(restored.buffer.rope.to_string(), "h\noriginal\n");
            assert_eq!(restored.csv_draft.as_ref().unwrap().value, "old draft 中😀");
            assert!(restored.csv_draft.as_ref().unwrap().source_matches);
            assert_eq!(writer.committed, 1);
        }
    }

    #[test]
    #[ignore = "release timing on a draft above the streaming threshold"]
    fn large_session_capture_timing() {
        let dir = TempDir::new("session-capture-timing");
        let path = dir.join("session.toml");
        let tabs = vec![draft(&"x".repeat(11 * 1024 * 1024))];
        let mut writer = Writer::default();
        let started = std::time::Instant::now();
        for generation in 1..=5 {
            writer.write_at(pending(generation, &tabs), &path, dir.path()).expect("write");
        }
        let synchronous = started.elapsed();
        let started = std::time::Instant::now();
        let jobs: Vec<_> = (6..=10).map(|generation| pending(generation, &tabs)).collect();
        let capture = started.elapsed();
        for job in jobs { writer.write_at(job, &path, dir.path()).expect("worker write"); }
        println!("11 MiB draft, five full synchronous captures/writes: {synchronous:?}; five UI captures: {capture:?}");
        let mut csv = draft(&"x".repeat(11 * 1024 * 1024));
        let source_path = dir.join("large.csv");
        std::fs::write(&source_path, csv.buffer.rope.to_string()).unwrap();
        csv.buffer.file_path = Some(source_path);
        csv.csv_draft = Some(super::super::CsvDraft {
            row: Some(0), col: Some(0), value: "draft 中😀".into(), delimiter: Some(b','),
            source_matches: true, ..Default::default()
        });
        let started = std::time::Instant::now();
        let jobs: Vec<_> = (11..=15).map(|generation| pending(generation, std::slice::from_ref(&csv))).collect();
        let capture = started.elapsed();
        let started = std::time::Instant::now();
        for job in jobs { writer.write_at(job, &path, dir.path()).unwrap(); }
        println!("11 MiB CSV source with unfinished cell: five UI captures {capture:?}; five worker hashes/basis writes {elapsed:?}", elapsed = started.elapsed());
    }

    #[test]
    fn unsaved_doc_view_text_and_formatting_survive_a_restart() {
        use crate::rich_doc::edits::Edit;
        let dir = TempDir::new("session-rich-edits");
        let manifest_path = dir.join("session.toml");
        for name in ["source.rtf", "source.docx", "source.odt"] {
            let path = dir.join(name);
            if name.ends_with("rtf") {
                std::fs::write(&path, r"{\rtf1\ansi caf\'e9}").expect("rtf");
            } else {
                super::super::tests::rich_fixture(&path);
            }
            let original = std::fs::read(&path).expect("original");
            let mut tab = Tab::empty();
            tab.buffer = crate::io::read_buffer(&path).expect("open");
            let payload = tab.buffer.rich_doc.as_mut().expect("payload");
            payload.apply_edit(Edit::InsertText { block: 0, run: 0, byte: 0, text: "draft 中 ".into() });
            payload.apply_edit(Edit::SetBlockBold { block: 0, on: true });
            let expected = payload.block_text(0);
            tab.buffer.dirty = true;
            let mut writer = Writer::default();
            writer.write_at(pending(1, &[tab]), &manifest_path, dir.path()).expect("autosave");
            let manifest = super::super::load_manifest_at(&manifest_path, dir.path()).expect("manifest");
            let entry = manifest.tabs.first().expect("entry");
            let restored = match super::super::restore_tab_in(entry, dir.path()) {
                super::super::RestoreOutcome::Restored(tab) => tab,
                super::super::RestoreOutcome::Skipped(reason) => panic!("lost rich document: {reason}"),
            };
            let payload = restored.buffer.rich_doc.as_ref().expect("restored payload");
            assert_eq!(payload.block_text(0), expected, "{name}: unsaved text lost");
            assert!(payload.block_is_bold(0), "{name}: unsaved formatting lost");
            assert!(restored.buffer.dirty);
            assert_eq!(std::fs::read(&path).expect("original kept"), original);
            let saved = dir.join(&format!("saved-{name}"));
            crate::rich_doc::save(payload, &saved, None).expect("save recovered native document");
            let saved_payload = crate::rich_doc::open(&saved).expect("reopen saved native document");
            assert_eq!(saved_payload.block_text(0), expected);
            assert!(saved_payload.block_is_bold(0));

            // The structured file carries its own raw source and original
            // native bytes. Neither the legacy shadow nor the source is needed.
            std::fs::remove_file(&path).expect("remove original");
            std::fs::remove_file(bak_path_in(dir.path(), entry.id)).expect("remove raw shadow");
            let reopened = match super::super::restore_tab_in(entry, dir.path()) {
                super::super::RestoreOutcome::Restored(tab) => tab,
                super::super::RestoreOutcome::Skipped(reason) => panic!("missing native recovery: {reason}"),
            };
            assert_eq!(reopened.buffer.rich_doc.as_ref().expect("native recovery").block_text(0), expected);

            let closed = ClosedTab { buffer: reopened.buffer, session_id: reopened.session_id };
            let closed_entry = super::super::snapshot_closed_tab(&closed);
            Backup::closed(&closed).write_at(dir.path()).expect("closed recovery");
            std::fs::remove_file(bak_path_in(dir.path(), entry.id)).expect("remove closed raw shadow");
            let restored_closed = super::super::restore_closed_tab_in(&closed_entry, dir.path()).expect("closed tab");
            assert_eq!(restored_closed.buffer.rich_doc.as_ref().expect("closed payload").block_text(0), expected);

            // A lost manifest must find the .rich file on its own, once per
            // UUID, and require Save As instead of trusting an old destination.
            let orphan = super::super::load_manifest_at(&dir.join("missing.toml"), dir.path()).expect("orphan manifest");
            let orphan_entry = orphan.tabs.iter().find(|tab| tab.id == entry.id).expect("orphan entry");
            assert!(orphan_entry.rich_backup);
            assert!(orphan_entry.path.is_none());
            let recovered = match super::super::restore_tab_in(orphan_entry, dir.path()) {
                super::super::RestoreOutcome::Restored(tab) => tab,
                super::super::RestoreOutcome::Skipped(reason) => panic!("missing orphan recovery: {reason}"),
            };
            assert!(recovered.buffer.file_path.is_none());
            assert_eq!(recovered.buffer.rich_doc.as_ref().expect("orphan payload").block_text(0), expected);
        }
    }

    #[test]
    fn recovered_native_raw_edits_remain_unsaved_when_save_is_refused() {
        let dir = TempDir::new("session-native-raw-edits");
        let manifest_path = dir.join("session.toml");
        for name in ["source.docx", "source.odt"] {
            let path = dir.join(name);
            super::super::tests::rich_fixture(&path);
            let original = std::fs::read(&path).expect("original bytes");
            let mut tab = Tab::empty();
            tab.buffer = crate::io::read_buffer(&path).expect("open native document");
            tab.buffer.rope.insert(0, "unsaved source 中 ");
            tab.buffer.dirty = true;
            let expected = tab.buffer.rope.to_string();
            let mut writer = Writer::default();
            writer.write_at(pending(1, &[tab]), &manifest_path, dir.path()).expect("recovery write");
            let manifest = super::super::load_manifest_at(&manifest_path, dir.path()).expect("manifest");
            let entry = manifest.tabs.first().expect("entry");
            let restored = match super::super::restore_tab_in(entry, dir.path()) {
                super::super::RestoreOutcome::Restored(tab) => tab,
                super::super::RestoreOutcome::Skipped(reason) => panic!("raw edits lost: {reason}"),
            };
            assert!(restored.buffer.dirty);
            assert_eq!(restored.buffer.rope.to_string(), expected);
            let payload = restored.buffer.rich_doc.as_ref().expect("native payload");
            assert!(matches!(crate::rich_doc::save(payload, &path, Some(&expected)),
                Err(crate::rich_doc::RichDocError::UnsupportedRawEdits)));
            assert_eq!(std::fs::read(&path).expect("untouched original"), original);
        }
    }

    #[test]
    fn damaged_doc_view_recovery_keeps_raw_source_as_a_safe_scratch_tab() {
        let dir = TempDir::new("session-rich-fallback");
        let path = dir.join("source.docx");
        super::super::tests::rich_fixture(&path);
        let original = std::fs::read(&path).expect("original");
        let mut tab = Tab::empty();
        tab.buffer = crate::io::read_buffer(&path).expect("open");
        tab.buffer.rich_doc.as_mut().expect("payload").apply_edit(crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: true });
        tab.buffer.dirty = true;
        let entry = super::super::snapshot_tab_with_policy(&tab, BakPolicy::Full);
        Backup::capture(&tab, BakPolicy::Full).write_at(dir.path()).expect("backup");
        let closed = ClosedTab { buffer: tab.buffer, session_id: tab.session_id };
        let closed_entry = super::super::snapshot_closed_tab(&closed);
        std::fs::write(super::super::rich_backup::path_in(dir.path(), entry.id), b"damaged").expect("damage");
        let restored = match super::super::restore_tab_in(&entry, dir.path()) {
            super::super::RestoreOutcome::Restored(tab) => tab,
            super::super::RestoreOutcome::Skipped(reason) => panic!("lost readable raw source: {reason}"),
        };
        assert!(restored.buffer.file_path.is_none());
        assert!(restored.buffer.rich_doc.is_none());
        assert!(restored.buffer.dirty);
        assert_eq!(restored.buffer.rope, closed.buffer.rope);
        assert!(restored.buffer.new_label.as_ref().expect("label").starts_with("Recovered raw source "));
        let restored_closed = super::super::restore_closed_tab_in(&closed_entry, dir.path()).expect("closed fallback");
        assert!(restored_closed.buffer.file_path.is_none());
        assert!(restored_closed.buffer.rich_doc.is_none());
        assert!(restored_closed.buffer.dirty);
        assert_eq!(std::fs::read(path).expect("original untouched"), original);
        let mut writer = Writer::default();
        writer.write_at(pending(1, &[restored]), &dir.join("session.toml"), dir.path()).expect("save scratch recovery");
        assert!(!super::super::rich_backup::path_in(dir.path(), entry.id).exists());
    }

    #[test]
    fn clean_closed_native_recovery_becomes_dirty_when_the_source_is_missing_or_changed() {
        let dir = TempDir::new("session-rich-clean-source");
        let path = dir.join("source.rtf");
        std::fs::write(&path, br"{\rtf1\ansi original}").expect("original");
        let closed = ClosedTab { buffer: crate::io::read_buffer(&path).expect("open"), session_id: Uuid::new_v4() };
        let entry = super::super::snapshot_closed_tab(&closed);
        Backup::closed(&closed).write_at(dir.path()).expect("backup");
        let restored = super::super::restore_closed_tab_in(&entry, dir.path()).expect("unchanged restore");
        assert!(!restored.buffer.dirty);
        std::fs::write(&path, br"{\rtf1\ansi external change}").expect("external change");
        let restored = super::super::restore_closed_tab_in(&entry, dir.path()).expect("changed restore");
        assert!(restored.buffer.dirty);
        assert_eq!(restored.buffer.rich_doc.as_ref().expect("payload").block_text(0), "original");
        std::fs::remove_file(&path).expect("remove original");
        let restored = super::super::restore_closed_tab_in(&entry, dir.path()).expect("missing restore");
        assert!(restored.buffer.dirty);
        assert_eq!(restored.buffer.rich_doc.as_ref().expect("payload").block_text(0), "original");
    }

    #[test]
    fn capture_preserves_the_buffer_until_the_worker_writes_it() {
        let dir = TempDir::new("session-captured-write");
        let path = dir.join("session.toml");
        let mut tabs = vec![draft("café\n中")];
        let id = tabs[0].session_id;
        let captured = pending(1, &tabs);
        assert!(!bak_path_in(dir.path(), id).exists());
        tabs[0].buffer.rope.insert(0, "later ");
        let mut writer = Writer::default();
        writer.write_at(captured, &path, dir.path()).expect("write");
        assert_eq!(std::fs::read_to_string(bak_path_in(dir.path(), id)).expect("backup"), "café\n中");
        let text = std::fs::read_to_string(&path).expect("manifest");
        assert!(!text.contains("backups"));
        let entry = super::super::load_manifest_at(&path, dir.path()).expect("manifest").tabs.remove(0);
        match super::super::restore_tab_in(&entry, dir.path()) {
            super::super::RestoreOutcome::Restored(tab) => assert_eq!(tab.buffer.rope.to_string(), "café\n中"),
            super::super::RestoreOutcome::Skipped(reason) => panic!("lost draft: {reason}"),
        }
    }

    #[test]
    fn delayed_old_job_cannot_overwrite_a_newer_complete_snapshot() {
        let dir = TempDir::new("session-write-order");
        let path = dir.join("session.toml");
        let mut tabs = vec![draft("old")];
        let old = pending(1, &tabs);
        let first = tabs[0].session_id;
        tabs[0].buffer.rope = ropey::Rope::from_str("new first window");
        tabs.push(draft("new second window"));
        let second = tabs[1].session_id;
        let mut writer = Writer::default();
        writer.write_at(pending(2, &tabs), &path, dir.path()).expect("new write");
        writer.write_at(old, &path, dir.path()).expect("stale write");
        assert_eq!(std::fs::read_to_string(bak_path_in(dir.path(), first)).expect("first"), "new first window");
        assert_eq!(std::fs::read_to_string(bak_path_in(dir.path(), second)).expect("second"), "new second window");
        assert_eq!(super::super::load_manifest_at(&path, dir.path()).expect("manifest").tabs.len(), 2);
    }

    #[test]
    fn backup_failure_keeps_the_previous_manifest_and_unreferenced_drafts() {
        let dir = TempDir::new("session-write-failure");
        let path = dir.join("session.toml");
        let tabs = vec![draft("previous")];
        let mut writer = Writer::default();
        writer.write_at(pending(1, &tabs), &path, dir.path()).expect("first write");
        let previous = std::fs::read(&path).expect("previous manifest");
        let orphan = bak_path_in(dir.path(), Uuid::new_v4());
        std::fs::write(&orphan, "keep until successful recovery write").expect("orphan");
        let next = vec![draft("next")];
        let failed_backup = bak_path_in(dir.path(), next[0].session_id);
        std::fs::create_dir(&failed_backup).expect("block backup destination");
        assert!(writer.write_at(pending(2, &next), &path, dir.path()).is_err());
        assert_eq!(std::fs::read(&path).expect("manifest"), previous);
        assert!(orphan.exists());
        assert_eq!(writer.committed, 1);
        std::fs::remove_dir(&failed_backup).expect("remove empty fixture directory");
        writer.write_at(pending(2, &next), &path, dir.path()).expect("retry");
        assert!(!orphan.exists());
        assert_eq!(std::fs::read_to_string(&failed_backup).expect("new backup"), "next");
    }

    #[test]
    fn failed_job_cannot_leave_a_cached_backup_out_of_sync_on_retry() {
        let dir = TempDir::new("session-partial-retry");
        let path = dir.join("session.toml");
        let mut tabs = vec![draft("previous snapshot")];
        let previous = pending(1, &tabs).manifest;
        let id = tabs[0].session_id;
        let mut writer = Writer::default();
        writer.write_at(PendingSave { generation: 1, manifest: previous.clone(), reap: true }, &path, dir.path()).expect("previous");
        tabs[0].buffer.rope = ropey::Rope::from_str("partly written newer snapshot");
        tabs.push(draft("blocked"));
        std::fs::create_dir(bak_path_in(dir.path(), tabs[1].session_id)).expect("block second write");
        assert!(writer.write_at(pending(2, &tabs), &path, dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(bak_path_in(dir.path(), id)).expect("partial body"), "partly written newer snapshot");
        writer.write_at(PendingSave { generation: 3, manifest: previous, reap: true }, &path, dir.path()).expect("retry earlier recovery point");
        assert_eq!(std::fs::read_to_string(bak_path_in(dir.path(), id)).expect("repaired"), "previous snapshot");
    }

    #[test]
    fn clean_snapshot_removes_only_its_stale_backup_after_publication() {
        let dir = TempDir::new("session-clean-write");
        let path = dir.join("session.toml");
        let mut tabs = vec![draft("saved"), draft("other window")];
        let first = tabs[0].session_id;
        let second = tabs[1].session_id;
        let mut writer = Writer::default();
        writer.write_at(pending(1, &tabs), &path, dir.path()).expect("drafts");
        tabs[0].buffer.file_path = Some(dir.join("saved.txt"));
        tabs[0].buffer.dirty = false;
        writer.write_at(pending(2, &tabs), &path, dir.path()).expect("clean snapshot");
        assert!(!bak_path_in(dir.path(), first).exists());
        assert!(bak_path_in(dir.path(), second).exists());
        let manifest = super::super::load_manifest_at(&path, dir.path()).expect("manifest");
        assert!(!manifest.tabs[0].dirty);
        assert!(manifest.tabs[1].dirty);
    }
}
