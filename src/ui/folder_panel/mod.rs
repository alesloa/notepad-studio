// `View ▸ Folder as Workspace` left dock — NPP-style file tree with multi-
// root support, lazy directory loading, and a click-to-open-tab handler.
//
// Layout split:
//   `mod.rs`           — public entry, panel state, render top-level
//   `tree.rs`          — directory cache, expand/collapse, row layout
//   `context_menu.rs`  — right-click menus on tree rows
//
// Persistence: roots, expanded set, panel visible/hidden, width, and
// show-hidden are all stored in `Settings`. The Workspace owns the
// in-memory directory cache and selected-row state since both are
// runtime-only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::settings;

pub(crate) mod context_menu;
pub(crate) mod tree;

pub(crate) use tree::folder_panel;

/// Runtime state for the folder-as-workspace panel. Persistent bits live in
/// `Settings`; this struct holds per-workspace selection / context-menu /
/// drag state. The directory listing cache itself is process-global (see
/// `cached_read_dir_sorted` below) — directory contents are OS truth and
/// don't differ between Notepad Studio windows.
pub(crate) struct FolderPanelState {
    /// Last-clicked path. Drives the selected-row highlight and is the
    /// target for keyboard navigation in v2.
    pub(crate) selected: Option<PathBuf>,
    /// Right-click context menu state. `Some(state)` while visible.
    pub(crate) context_menu: Option<self::context_menu::FolderContextMenuState>,
    /// In-flight splitter drag — captures the initial mouse x and panel
    /// width on mouse-down so subsequent moves can compute the new width
    /// without leaning on the original handle bounds.
    pub(crate) drag: Option<SplitterDrag>,
}

impl FolderPanelState {
    pub(crate) fn new() -> Self {
        Self {
            selected: None,
            context_menu: None,
            drag: None,
        }
    }

    /// Drop a path's cached children — called on collapse and on Refresh.
    pub(crate) fn invalidate(&mut self, path: &Path) {
        cache_invalidate(path);
    }

    /// Drop every cached entry under `root` (including `root` itself).
    pub(crate) fn invalidate_subtree(&mut self, root: &Path) {
        cache_invalidate_subtree(root);
    }
}

/// Process-global directory-listing cache. Each entry remembers the
/// `show_hidden` value that was active when the listing was read; on
/// lookup, we evict the entry if the current `show_hidden` differs (so
/// toggling `Show Hidden Files` always re-reads from disk).
struct DirCacheEntry {
    show_hidden: bool,
    entries: Vec<DirEntry>,
}

static DIR_CACHE: OnceLock<Mutex<HashMap<PathBuf, DirCacheEntry>>> = OnceLock::new();

fn dir_cache() -> &'static Mutex<HashMap<PathBuf, DirCacheEntry>> {
    DIR_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cache-aware front for `read_dir_sorted`. Hits the global cache on
/// matching `(path, show_hidden)`; otherwise falls back to disk and
/// populates the cache for the next paint.
pub(crate) fn cached_read_dir_sorted(dir: &Path, show_hidden: bool) -> Vec<DirEntry> {
    {
        let guard = dir_cache().lock().unwrap();
        if let Some(entry) = guard.get(dir) {
            if entry.show_hidden == show_hidden {
                return entry.entries.clone();
            }
        }
    }
    let entries = read_dir_sorted(dir, show_hidden);
    let mut guard = dir_cache().lock().unwrap();
    guard.insert(
        dir.to_path_buf(),
        DirCacheEntry {
            show_hidden,
            entries: entries.clone(),
        },
    );
    entries
}

/// Drop a single path's cached children — Refresh on a folder row.
pub(crate) fn cache_invalidate(path: &Path) {
    dir_cache().lock().unwrap().remove(path);
}

/// Drop every cached entry under `root` (including `root` itself) — used
/// by Refresh / Collapse All Below / Remove from Workspace so a later
/// expand re-reads from disk.
pub(crate) fn cache_invalidate_subtree(root: &Path) {
    let mut guard = dir_cache().lock().unwrap();
    guard.retain(|p, _| !p.starts_with(root));
}

/// Drop the entire cache — toggling Show Hidden Files flips the filter
/// for every directory at once, so the simplest correct response is a
/// full clear (`cached_read_dir_sorted` would otherwise rebuild lazily
/// anyway).
pub(crate) fn cache_clear() {
    dir_cache().lock().unwrap().clear();
}

/// Splitter drag snapshot captured on mouse-down. Consumed by the workspace's
/// global mouse-move listener.
#[derive(Clone, Copy)]
pub(crate) struct SplitterDrag {
    pub(crate) start_mouse_x: gpui::Pixels,
    pub(crate) start_width: f32,
}

/// One directory entry in the panel's cached children list. We intentionally
/// keep this as a flat struct (vs `enum DirEntry { File / Dir }`) — the
/// `is_dir` flag drives row rendering and chevron presence, and a future
/// "symlink-to-dir" case still classifies as `is_dir = true`.
#[derive(Clone, Debug)]
pub(crate) struct DirEntry {
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
}

/// Read `dir`'s entries, applying the `show_hidden` filter and sorting
/// folders-before-files / case-insensitive within each group. Returns an
/// empty vec on read error (logs to stderr) so the caller's expand-row
/// branch still has something to render.
pub(crate) fn read_dir_sorted(dir: &Path, show_hidden: bool) -> Vec<DirEntry> {
    let mut entries: Vec<DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|de| {
                let name = de.file_name();
                let name_str = name.to_string_lossy();
                if !show_hidden && name_str.starts_with('.') {
                    return None;
                }
                let metadata = de.metadata().ok()?;
                Some(DirEntry {
                    path: de.path(),
                    is_dir: metadata.is_dir(),
                })
            })
            .collect(),
        Err(err) => {
            eprintln!(
                "[folder-panel] read_dir({}): {err}",
                dir.display()
            );
            return Vec::new();
        }
    };
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default()
            .cmp(
                &b.path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default(),
            ),
    });
    entries
}

/// Whether `path` is currently expanded per the persisted set. Lookup is
/// O(n) over the vec but the set is bounded by the user's open count.
pub(crate) fn is_expanded(expanded: &[PathBuf], path: &Path) -> bool {
    expanded.iter().any(|p| p == path)
}

/// Toggle expansion for `path`, persisting via Settings. Returns the new
/// state (`true` if now expanded). Caller is responsible for calling
/// `cx.notify()` on the workspace.
pub(crate) fn toggle_expanded(path: &Path) -> bool {
    let mut snap = settings::snapshot();
    let next = if let Some(idx) = snap.folder_panel_expanded.iter().position(|p| p == path) {
        snap.folder_panel_expanded.remove(idx);
        false
    } else {
        snap.folder_panel_expanded.push(path.to_path_buf());
        true
    };
    settings::replace_and_save(snap);
    next
}

/// Collapse `path` and every descendant currently in the expanded set —
/// used by the "Collapse All Below" context-menu item.
pub(crate) fn collapse_subtree(root: &Path) {
    let mut snap = settings::snapshot();
    snap.folder_panel_expanded
        .retain(|p| !p.starts_with(root) && p != root);
    settings::replace_and_save(snap);
}

/// Add `path` as a top-level root if it isn't already present. Returns the
/// new total root count for caller status updates.
pub(crate) fn add_root(path: PathBuf) -> usize {
    let mut snap = settings::snapshot();
    if !snap.folder_panel_roots.iter().any(|p| p == &path) {
        snap.folder_panel_roots.push(path);
    }
    let n = snap.folder_panel_roots.len();
    settings::replace_and_save(snap);
    n
}

/// Remove `path` from the root list. Also drops any expanded entries that
/// fall under it so collapsing doesn't leave orphan paths in settings.
pub(crate) fn remove_root(path: &Path) {
    let mut snap = settings::snapshot();
    snap.folder_panel_roots.retain(|p| p != path);
    snap.folder_panel_expanded
        .retain(|p| !p.starts_with(path) && p != path);
    settings::replace_and_save(snap);
}

/// Set `folder_panel_visible` to `next` and persist.
pub(crate) fn set_visible(next: bool) {
    let mut snap = settings::snapshot();
    if snap.folder_panel_visible != next {
        snap.folder_panel_visible = next;
        settings::replace_and_save(snap);
    }
}

/// Set `folder_panel_show_hidden` to `next` and persist.
pub(crate) fn set_show_hidden(next: bool) {
    let mut snap = settings::snapshot();
    if snap.folder_panel_show_hidden != next {
        snap.folder_panel_show_hidden = next;
        settings::replace_and_save(snap);
    }
}

/// Clamp + persist a new panel width.
pub(crate) fn set_width(next: f32) {
    let clamped = next.clamp(
        settings::FOLDER_PANEL_MIN_WIDTH,
        settings::FOLDER_PANEL_MAX_WIDTH,
    );
    let mut snap = settings::snapshot();
    if (snap.folder_panel_width - clamped).abs() > 0.5 {
        snap.folder_panel_width = clamped;
        settings::replace_and_save(snap);
    }
}

/// Build a `HashSet` view of the persisted expanded list — cheaper for the
/// per-row "is this expanded?" check during render.
pub(crate) fn expanded_set(expanded: &[PathBuf]) -> HashSet<PathBuf> {
    expanded.iter().cloned().collect()
}
