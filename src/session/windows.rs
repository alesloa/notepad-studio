use super::SessionManifest;
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

#[derive(Default)]
struct WindowSnapshots {
    snapshots: Vec<(Uuid, SessionManifest)>,
    live: BTreeSet<Uuid>,
}

impl WindowSnapshots {
    fn register(&mut self, id: Uuid, snapshot: SessionManifest) {
        // After the last window closes, a dock reopen restores the on-disk
        // session into the first new workspace. Retire the old owners first.
        if self.live.is_empty() {
            self.snapshots.clear();
        }
        self.live.insert(id);
        self.update(id, snapshot);
    }

    fn release(&mut self, id: Uuid) {
        // Keep its last snapshot recoverable until the next restore. Closing
        // a window is different from closing each document in that window.
        self.live.remove(&id);
    }

    fn update(&mut self, id: Uuid, snapshot: SessionManifest) -> SessionManifest {
        let incoming: BTreeSet<Uuid> = snapshot.tabs.iter().map(|tab| tab.id)
            .chain(snapshot.closed_tabs.iter().map(|tab| tab.id)).collect();
        for (key, previous) in &mut self.snapshots {
            if *key == id {
                continue;
            }
            // A moved document keeps its UUID. Its new owner must supersede
            // the old owner's cached entry, including its restore history.
            let active_id = previous.tabs.get(previous.active).map(|tab| tab.id);
            previous.tabs.retain(|tab| !incoming.contains(&tab.id));
            previous.closed_tabs.retain(|tab| !incoming.contains(&tab.id));
            previous.mru.retain(|tab| !incoming.contains(tab));
            previous.active = active_id.and_then(|active| previous.tabs.iter().position(|tab| tab.id == active))
                .unwrap_or_else(|| previous.active.min(previous.tabs.len().saturating_sub(1)));
        }
        if let Some((_, previous)) = self.snapshots.iter_mut().find(|(key, _)| *key == id) {
            *previous = snapshot.clone();
        } else {
            self.snapshots.push((id, snapshot.clone()));
        }
        let mut combined = SessionManifest {
            theme: snapshot.theme,
            show_whitespace: snapshot.show_whitespace,
            word_wrap: snapshot.word_wrap,
            ..Default::default()
        };
        for (key, window) in &self.snapshots {
            if *key == id {
                combined.active = combined.tabs.len().saturating_add(window.active);
            }
            combined.tabs.extend(window.tabs.iter().cloned());
            combined.closed_tabs.extend(window.closed_tabs.iter().cloned());
            combined.mru.extend(window.mru.iter().copied());
        }
        combined.active = combined.active.min(combined.tabs.len().saturating_sub(1));
        combined
    }
}

static WINDOWS: OnceLock<Mutex<WindowSnapshots>> = OnceLock::new();

fn store() -> &'static Mutex<WindowSnapshots> {
    WINDOWS.get_or_init(|| Mutex::new(WindowSnapshots::default()))
}

pub(crate) fn register_workspace(id: Uuid, snapshot: SessionManifest) {
    let Ok(mut windows) = store().lock() else {
        eprintln!("[session] cannot register window: session lock is poisoned");
        return;
    };
    windows.register(id, snapshot);
}

pub(crate) fn release_workspace(id: Uuid) {
    if let Ok(mut windows) = store().lock() {
        windows.release(id);
    }
}

pub(super) fn update(id: Uuid, snapshot: SessionManifest) -> Option<SessionManifest> {
    let Ok(mut windows) = store().lock() else {
        eprintln!("[session] cannot save windows: session lock is poisoned");
        return None;
    };
    Some(windows.update(id, snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(id: Uuid, label: &str) -> super::super::SessionTab {
        toml::from_str(&format!("id = '{id}'\ndirty = true\nnew_label = '{label}'")).expect("draft")
    }

    #[test]
    fn saving_one_window_keeps_another_windows_recovery_files() {
        let dir = crate::persist::TempDir::new("multi-window-session");
        let manifest_path = dir.join("session.toml");
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let backup_dir = dir.join("");
        let first_backup = super::super::bak_path_in(&backup_dir, first);
        let first_undo = super::super::undo_path_in(&backup_dir, first);
        std::fs::write(&first_backup, "unsaved first window").expect("backup");
        std::fs::write(&first_undo, b"undo").expect("undo");
        let second_backup = super::super::bak_path_in(&backup_dir, second);
        std::fs::write(&second_backup, "unsaved second window").expect("backup");
        let mut snapshots = WindowSnapshots::default();
        snapshots.update(Uuid::new_v4(), SessionManifest {
            tabs: vec![draft(first, "first")], ..Default::default()
        });
        let second_window = Uuid::new_v4();
        let manifest = snapshots.update(second_window, SessionManifest {
            tabs: vec![draft(second, "second")], ..Default::default()
        });
        super::super::write_manifest_at(&manifest, &manifest_path).expect("manifest");
        super::super::reap_orphan_baks_at(&manifest, &backup_dir);
        assert_eq!(std::fs::read_to_string(&first_backup).expect("first window backup survives"), "unsaved first window");
        assert!(first_undo.exists());
        let restored = super::super::load_manifest_at(&manifest_path, &backup_dir).expect("restore");
        assert_eq!(restored.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(), vec![first, second]);
        for (entry, expected) in restored.tabs.iter().zip(["unsaved first window", "unsaved second window"]) {
            match super::super::restore_tab_in(entry, &backup_dir) {
                super::super::RestoreOutcome::Restored(tab) => assert_eq!(tab.buffer.rope.to_string(), expected),
                super::super::RestoreOutcome::Skipped(reason) => panic!("draft was skipped: {reason}"),
            }
        }
        let manifest = snapshots.update(second_window, SessionManifest::default());
        super::super::reap_orphan_baks_at(&manifest, &backup_dir);
        assert!(first_backup.exists());
        assert!(first_undo.exists());
        assert!(!second_backup.exists());
    }

    #[test]
    fn empty_new_window_cannot_replace_the_restored_session() {
        let mut snapshots = WindowSnapshots::default();
        let first_window = Uuid::new_v4();
        let second_window = Uuid::new_v4();
        let tab = Uuid::new_v4();
        snapshots.register(first_window, SessionManifest { tabs: vec![draft(tab, "draft")], ..Default::default() });
        snapshots.register(second_window, SessionManifest::default());
        let manifest = snapshots.update(second_window, SessionManifest::default());
        assert_eq!(manifest.tabs.len(), 1);
        assert_eq!(manifest.tabs[0].id, tab);
    }

    #[test]
    fn moving_a_tab_updates_ownership_and_remaps_the_other_windows_active_tab() {
        let mut snapshots = WindowSnapshots::default();
        let first_window = Uuid::new_v4();
        let second_window = Uuid::new_v4();
        let moved = Uuid::new_v4();
        let active = Uuid::new_v4();
        snapshots.register(first_window, SessionManifest {
            active: 1, tabs: vec![draft(moved, "move"), draft(active, "stay")],
            mru: vec![active, moved], ..Default::default()
        });
        snapshots.register(second_window, SessionManifest { tabs: vec![draft(moved, "moved")], ..Default::default() });
        let first = &snapshots.snapshots[0].1;
        assert_eq!(first.tabs.len(), 1);
        assert_eq!(first.tabs[first.active].id, active);
        assert_eq!(first.mru, vec![active]);
        let manifest = snapshots.update(second_window, SessionManifest {
            tabs: vec![draft(moved, "moved")], ..Default::default()
        });
        assert_eq!(manifest.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(), vec![active, moved]);
        assert_eq!(manifest.tabs[manifest.active].id, moved);
    }

    #[test]
    fn dock_reopen_replaces_old_window_owners_without_duplicating_tabs() {
        let mut snapshots = WindowSnapshots::default();
        let first_window = Uuid::new_v4();
        let second_window = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        snapshots.register(first_window, SessionManifest { tabs: vec![draft(first, "first")], ..Default::default() });
        snapshots.register(second_window, SessionManifest { tabs: vec![draft(second, "second")], ..Default::default() });
        snapshots.release(first_window);
        let manifest = snapshots.update(second_window, SessionManifest { tabs: vec![draft(second, "second")], ..Default::default() });
        assert_eq!(manifest.tabs.len(), 2);
        snapshots.release(second_window);
        let reopened = Uuid::new_v4();
        snapshots.register(reopened, manifest.clone());
        let manifest = snapshots.update(reopened, manifest);
        assert_eq!(manifest.tabs.len(), 2);
        assert_eq!(snapshots.snapshots.len(), 1);
    }

    #[test]
    fn closing_and_reopening_a_document_keeps_one_owner_of_its_restore_history() {
        let mut snapshots = WindowSnapshots::default();
        let first_window = Uuid::new_v4();
        let second_window = Uuid::new_v4();
        let tab = Uuid::new_v4();
        let closed = toml::from_str(&format!("id = '{tab}'\nnew_label = 'closed'")).expect("closed tab");
        snapshots.register(first_window, SessionManifest { closed_tabs: vec![closed], ..Default::default() });
        snapshots.register(second_window, SessionManifest::default());
        let manifest = snapshots.update(second_window, SessionManifest::default());
        assert_eq!(manifest.closed_tabs.len(), 1);
        let manifest = snapshots.update(second_window, SessionManifest { tabs: vec![draft(tab, "reopened")], ..Default::default() });
        assert_eq!(manifest.tabs.len(), 1);
        assert!(manifest.closed_tabs.is_empty());
    }
}
