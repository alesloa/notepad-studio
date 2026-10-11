// Recent Files persistence.
//
// Notepad++'s File ▸ Open Recent submenu — most-recently-used queue of file
// paths that survives restarts. Lives in its own `recent.toml` next to
// `window.toml` so the recents read on every menu rebuild stays cheap and
// independent of the (larger) session manifest.
//
// In-memory shape: `RecentFiles` wraps a `Vec<PathBuf>` ordered front-to-back
// from most-recent to least-recent, capped at `MAX_RECENT`. Push dedupes on
// equality (same path bumped to the front) and trims the tail past the cap.
// On-disk shape: a single TOML doc with one `paths` array of strings, since
// PathBuf round-trips fine through display strings on every platform we
// target (rfd's file dialogs already canonicalise to absolute paths).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::consts::APP_NAME;

/// Effective MRU cap. Reads `Settings.recent_files_max` (NPP `Recent Files
/// History ▸ Max. number of entries:`) and clamps to `[0, 30]` to match
/// NPP's spinner range — zero disables the list, anything beyond 30 is
/// silently capped. Default settings value is 10, matching NPP's own
/// default and the prior hardcoded `MAX_RECENT` const.
fn max_recent() -> usize {
    let n = crate::settings::snapshot()
        .recent_files_max
        .min(crate::settings::RECENT_FILES_MAX_HARD_CAP);
    n as usize
}

#[derive(Default, Debug, Clone)]
pub(crate) struct RecentFiles {
    /// Most-recent first. Bounded to `MAX_RECENT` entries by `push`.
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// Bump `path` to the front of the MRU queue. Equal entries already in
    /// the list are removed first so a re-open doesn't leave stale duplicates
    /// — paths are compared with `PathBuf::==`, which already canonicalises
    /// for the rfd-supplied absolute paths we're given. Trims to the
    /// user-configurable `Settings.recent_files_max` cap.
    pub(crate) fn push(&mut self, path: PathBuf) {
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        let cap = max_recent();
        if self.paths.len() > cap {
            self.paths.truncate(cap);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.paths.clear();
    }
}

#[derive(Serialize, Deserialize, Default)]
struct RecentFilesDisk {
    /// Stored as strings so the file stays round-trippable across platforms
    /// (Windows backslashes vs. POSIX forward slashes both serialise as TOML
    /// strings without serde adapters).
    #[serde(default)]
    paths: Vec<String>,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("recent.toml"))
}

/// Read `recent.toml` from disk into a `RecentFiles`. Missing file or any
/// parse failure returns an empty list — recents are purely a convenience
/// feature, never a hard error path. When `Settings.recent_files_check_at_launch`
/// is on (NPP `Don't check at launch time`-inverse), entries whose path no
/// longer exists are dropped during this load — keeps the File ▸ Open Recent
/// menu free of dead links across moves and deletions.
pub(crate) fn load() -> RecentFiles {
    let Some(path) = config_path() else {
        return RecentFiles::default();
    };
    let mut paths = load_paths_at(&path);
    if crate::settings::snapshot().recent_files_check_at_launch {
        paths = filter_existing(&paths);
    }
    let cap = max_recent();
    if paths.len() > cap {
        paths.truncate(cap);
    }
    RecentFiles { paths }
}

/// The paths stored in `path`. An entry that isn't a string is skipped
/// rather than costing the whole list; a file that isn't TOML is moved
/// aside as `recent.toml.corrupt`.
fn load_paths_at(path: &Path) -> Vec<PathBuf> {
    let parsed = crate::persist::load_toml(
        path,
        |t| crate::persist::salvage_table(t, &[]).unwrap_or_default(),
        RecentFilesDisk::default,
    )
    .into_option()
    .unwrap_or_default();
    parsed
        .paths
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Drop entries from `list` whose path no longer exists on disk. Preserves
/// order. Used at launch when the user opted into stale-entry pruning.
pub(crate) fn filter_existing(list: &[PathBuf]) -> Vec<PathBuf> {
    list.iter().filter(|p| p.exists()).cloned().collect()
}

/// Persist `recent` to `recent.toml`. Best-effort: any I/O error is dropped
/// silently so a non-writable config dir can't surface as a broken-looking
/// File menu interaction.
pub(crate) fn save(recent: &RecentFiles) {
    let Some(path) = config_path() else {
        return;
    };
    if let Err(err) = save_at(&path, recent) {
        eprintln!("[recent] couldn't save {}: {err}", path.display());
    }
}

fn save_at(path: &Path, recent: &RecentFiles) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let disk = RecentFilesDisk {
        paths: recent
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    };
    let serialized = toml::to_string_pretty(&disk).map_err(std::io::Error::other)?;
    crate::persist::atomic_write(path, serialized.as_bytes())
}

/// Process-global MRU list. Initialised lazily on first access from the
/// persisted file. The OS app menu is a single shared surface (macOS most
/// strictly, but every platform we ship to here) so a single shared MRU
/// across windows is the right model — a fresh recent in window A should
/// surface immediately in window B's File menu.
static GLOBAL: OnceLock<Mutex<RecentFiles>> = OnceLock::new();

#[cfg(test)]
#[path = "recent_test_snapshot.rs"]
mod test_snapshot;
#[cfg(test)]
pub(crate) use test_snapshot::override_for_test as override_snapshot_for_test;

fn global() -> &'static Mutex<RecentFiles> {
    GLOBAL.get_or_init(|| Mutex::new(load()))
}

/// Snapshot the current MRU paths for menu builders. Returns an owned `Vec`
/// to keep callers off the lock for any longer than necessary.
pub(crate) fn snapshot() -> Vec<PathBuf> {
    #[cfg(test)]
    if let Some(paths) = test_snapshot::snapshot() {
        return paths;
    }
    global()
        .lock()
        .map(|g| g.paths.clone())
        .unwrap_or_default()
}

/// Push `path` to the front of the global MRU list and persist to disk.
pub(crate) fn record_opened(path: PathBuf) {
    if let Ok(mut g) = global().lock() {
        g.push(path);
        save(&g);
    }
}

/// Empty the global MRU list and persist the cleared state.
pub(crate) fn clear_global() {
    if let Ok(mut g) = global().lock() {
        g.clear();
        save(&g);
    }
}

/// Drop a single entry from the MRU list. Used when the user confirms
/// removal of a stale recent (file no longer exists on disk).
pub(crate) fn remove_path(path: &Path) {
    if let Ok(mut g) = global().lock() {
        let before = g.paths.len();
        g.paths.retain(|p| p != path);
        if g.paths.len() != before {
            save(&g);
        }
    }
}

/// Pretty-print a path for the File ▸ Open Recent menu. When `full_path` is
/// true (NPP `Display the full path`), shows the home-collapsed full path;
/// otherwise shows only the file name (NPP default — filename-only).
pub(crate) fn display_label(path: &Path, full_path: bool) -> String {
    if !full_path {
        return path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            // `Path::join("~", rest)` would yield `~/...` only on Unix-style
            // separators; format directly so Windows still gets `~\...` if
            // it ever lands there.
            let rest_str = rest.to_string_lossy();
            if rest_str.is_empty() {
                return "~".to_string();
            }
            return format!("~{}{}", std::path::MAIN_SEPARATOR, rest_str);
        }
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::TempDir;

    #[test]
    fn a_bad_entry_does_not_empty_the_recent_list() {
        let dir = TempDir::new("recent-load");
        let p = dir.join("recent.toml");
        assert!(load_paths_at(&p).is_empty());
        assert!(std::fs::write(&p, "paths = [\"/a.txt\", 3, \"\", \"/b.txt\"]\n").is_ok());
        assert_eq!(
            load_paths_at(&p),
            vec![PathBuf::from("/a.txt"), PathBuf::from("/b.txt")]
        );
        assert!(dir.join("recent.toml.corrupt").exists());
    }

    #[test]
    fn recent_list_round_trips_and_ignores_non_utf8_bytes() {
        let dir = TempDir::new("recent-save");
        let p = dir.join("recent.toml");
        let recent = RecentFiles {
            paths: vec![PathBuf::from("/x/ñ.txt"), PathBuf::from("C:\\y\\z.md")],
        };
        assert!(save_at(&p, &recent).is_ok());
        assert_eq!(load_paths_at(&p), recent.paths);

        // A stray byte in a comment (an editor saved it as Latin-1).
        assert!(std::fs::write(&p, b"paths = [\"/ok.txt\"]\n# caf\xe9\n").is_ok());
        assert_eq!(load_paths_at(&p), vec![PathBuf::from("/ok.txt")]);
    }
}