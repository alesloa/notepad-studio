// Per-window placement persistence.
//
// Each editor window gets its own slot in `<config>/Notepad Studio/window.toml`,
// keyed by the order it was first observed in this session. Bounds + zoom
// flow through `record_window_bounds` / `record_zoom_level`, both of which
// take a `&Window` so the right slot is updated. On exit, every observed
// window is flushed in slot order, so multi-window users get the same
// layout back on next launch.
//
// File format (new):
//
//   [[window]]
//   x = 0.0
//   y = 0.0
//   width = 1100.0
//   height = 720.0
//   zoom_level = 1.0
//
//   [[window]]
//   x = 1200.0
//   ...
//
// File format (legacy v0.0.3): a single flat record with the same field
// names but no `[[window]]` wrapping. `load_windows` falls back to that
// shape so users upgrading from v0.0.3 don't lose their last layout. New
// saves always write the multi-window format.
//
// Zoom is `#[serde(default)]` so legacy files written before zoom existed
// still deserialize cleanly.
//
// Implementation note: the per-window cache is a `BTreeMap<WindowId, …>`
// rather than `HashMap` because we want a tie-broken stable iteration when
// two windows share a slot index (shouldn't happen, but cheap insurance).
// The slot index is assigned the first time a `WindowId` is observed and
// is monotonic for the session.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use gpui::{Bounds, Pixels, Window, WindowBounds, WindowId, point, px, size};

use crate::consts::APP_NAME;
use crate::persist;

/// One window's persisted placement. The file root is a list of these.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub(crate) struct WindowEntry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    pub(crate) zoom_level: Option<f32>,
}

impl WindowEntry {
    fn from_bounds(b: Bounds<Pixels>, zoom_level: Option<f32>) -> Self {
        Self {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            width: f32::from(b.size.width),
            height: f32::from(b.size.height),
            zoom_level,
        }
    }

    /// The saved placement, or `None` when the file holds one no window can
    /// have (NaN, infinity, a zero or negative size); the caller then uses
    /// the default placement instead of opening an invisible window.
    pub(crate) fn into_bounds(self) -> Option<Bounds<Pixels>> {
        let finite = [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite());
        if !finite || self.width <= 0.0 || self.height <= 0.0 {
            return None;
        }
        Some(Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        })
    }
}

/// Fields a window entry can't be placed without.
const REQUIRED_FIELDS: [&str; 4] = ["x", "y", "width", "height"];

/// New file root. `windows` is ordered; entry 0 is the first window
/// restored on launch, entry 1 the second, etc.
#[derive(Serialize, Deserialize, Debug, Default)]
struct WindowsFile {
    #[serde(default, rename = "window")]
    windows: Vec<WindowEntry>,
}

/// Per-`WindowId` slot bookkeeping. The first time we observe a `WindowId`,
/// we assign it the next available slot index. Subsequent observations of
/// the same id update that slot's entry.
#[derive(Default)]
struct WindowCache {
    next_slot: usize,
    /// Map of `WindowId` → (slot_index, entry).
    by_id: BTreeMap<WindowId, (usize, WindowEntry)>,
    /// Last zoom level reported for each `WindowId`. Used to fill in
    /// `zoom_level` when bounds are recorded before any zoom event has
    /// fired for that window.
    last_zoom: BTreeMap<WindowId, f32>,
}

static WINDOW_CACHE: Mutex<Option<WindowCache>> = Mutex::new(None);

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("window.toml"))
}

/// Load every persisted window entry, oldest-slot first. Returns an empty
/// vec when the file is missing, unreadable, or empty.
///
/// Migration: if the file parses as the legacy single-record format, the
/// returned vec contains exactly that one entry.
pub(crate) fn load_windows() -> Vec<WindowEntry> {
    let Some(path) = settings_path() else {
        return Vec::new();
    };
    load_windows_at(&path)
}

/// `load_windows` for an explicit path. A damaged entry costs that entry
/// (or just its zoom), not the rest of the layout; a file that isn't TOML
/// is moved aside as `window.toml.corrupt`.
fn load_windows_at(path: &Path) -> Vec<WindowEntry> {
    // Every TOML document is a table, so this only quarantines non-TOML.
    let Some(table) =
        persist::load_toml::<toml::Table>(path, Clone::clone, toml::Table::new).into_option()
    else {
        return Vec::new();
    };
    let mut windows: Vec<WindowEntry> = if table.contains_key("window") {
        let (windows, dropped) = persist::salvage_array(table.get("window"), &REQUIRED_FIELDS);
        if dropped > 0 {
            eprintln!(
                "[config] skipped {dropped} unreadable window entr{} in {}",
                if dropped == 1 { "y" } else { "ies" },
                path.display()
            );
        }
        windows
    } else {
        // Legacy v0.0.3 single-record format.
        persist::salvage_table(&table, &REQUIRED_FIELDS).into_iter().collect()
    };
    for w in &mut windows {
        // `clamp` passes NaN through, and a NaN rem size breaks layout.
        w.zoom_level = w.zoom_level.filter(|z| z.is_finite());
    }
    windows
}

fn save_windows(entries: &[WindowEntry]) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Err(err) = save_windows_at(&path, entries) {
        eprintln!("[config] couldn't save {}: {err}", path.display());
    }
}

fn save_windows_at(path: &Path, entries: &[WindowEntry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = WindowsFile {
        windows: entries.to_vec(),
    };
    let serialized = toml::to_string_pretty(&file).map_err(std::io::Error::other)?;
    persist::atomic_write(path, serialized.as_bytes())
}

/// Record `window`'s current bounds into its slot. Allocates a new slot
/// the first time `window` is seen this session.
pub(crate) fn record_window_bounds(window: &Window) {
    let WindowBounds::Windowed(b) = window.window_bounds() else {
        return;
    };
    let id = window.window_handle().window_id();

    let Ok(mut guard) = WINDOW_CACHE.lock() else {
        return;
    };
    let cache = guard.get_or_insert_with(WindowCache::default);
    // Carry over any zoom level previously reported for this window — bounds
    // events fire often, zoom events rarely, so we want the bounds path to
    // preserve zoom rather than overwrite it with `None`.
    let zoom = cache
        .by_id
        .get(&id)
        .and_then(|(_, e)| e.zoom_level)
        .or_else(|| cache.last_zoom.get(&id).copied());
    let entry = WindowEntry::from_bounds(b, zoom);
    cache
        .by_id
        .entry(id)
        .and_modify(|(_, e)| *e = entry)
        .or_insert_with(|| {
            let slot = cache_next_slot(&mut cache.next_slot);
            (slot, entry)
        });
}

/// Record `window`'s zoom level into its slot. Allocates a new slot the
/// first time `window` is seen this session — same path as bounds.
pub(crate) fn record_zoom_level(window: &Window, zoom: f32) {
    let id = window.window_handle().window_id();

    let Ok(mut guard) = WINDOW_CACHE.lock() else {
        return;
    };
    let cache = guard.get_or_insert_with(WindowCache::default);
    cache.last_zoom.insert(id, zoom);

    if let Some((_, entry)) = cache.by_id.get_mut(&id) {
        entry.zoom_level = Some(zoom);
        return;
    }

    // Zoom event arrived before any bounds event — seed a slot from the
    // current window bounds so the zoom isn't dropped on the floor.
    let WindowBounds::Windowed(b) = window.window_bounds() else {
        return;
    };
    let entry = WindowEntry::from_bounds(b, Some(zoom));
    let slot = cache_next_slot(&mut cache.next_slot);
    cache.by_id.insert(id, (slot, entry));
}

/// Pull a slot index and bump the counter. Helper so the inline closure in
/// `record_window_bounds` stays readable.
fn cache_next_slot(next_slot: &mut usize) -> usize {
    let slot = *next_slot;
    *next_slot += 1;
    slot
}

/// Drop any cache entries whose `WindowId` isn't in `alive`, then write the
/// remaining entries to disk in slot order. Called from `on_window_closed`
/// with the still-open window set so the file mirrors what the user
/// actually has on screen at quit time.
///
/// When `alive` is empty (last window closed), the just-closed window's
/// bounds are written THEN the cache is fully cleared. Without the clear,
/// a close→reopen→close cycle inside one process would leave the prior
/// closed window's bounds in the cache; the next alive-empty flush would
/// write BOTH entries to `window.toml`, and the following reopen would
/// spawn two windows.
pub(crate) fn flush_window_settings_for_alive(alive: &[WindowId]) {
    if alive.is_empty() {
        let snapshot: Vec<WindowEntry> = match WINDOW_CACHE.lock() {
            Ok(mut guard) => {
                let cache = guard.get_or_insert_with(WindowCache::default);
                let mut entries: Vec<(usize, WindowEntry)> =
                    cache.by_id.values().copied().collect();
                entries.sort_by_key(|(slot, _)| *slot);
                let snapshot: Vec<WindowEntry> =
                    entries.into_iter().map(|(_, e)| e).collect();
                // Reset the cache so a reopen inside the same process starts
                // with a clean slot allocator and an empty id map. Without
                // this, the next reopen → close cycle accumulates stale
                // entries.
                cache.by_id.clear();
                cache.last_zoom.clear();
                cache.next_slot = 0;
                snapshot
            }
            Err(_) => return,
        };
        if !snapshot.is_empty() {
            save_windows(&snapshot);
        }
        return;
    }

    let snapshot: Vec<WindowEntry> = match WINDOW_CACHE.lock() {
        Ok(mut guard) => {
            let cache = guard.get_or_insert_with(WindowCache::default);
            cache.by_id.retain(|id, _| alive.contains(id));
            cache.last_zoom.retain(|id, _| alive.contains(id));
            let mut entries: Vec<(usize, WindowEntry)> =
                cache.by_id.values().copied().collect();
            entries.sort_by_key(|(slot, _)| *slot);
            entries.into_iter().map(|(_, e)| e).collect()
        }
        Err(_) => return,
    };

    save_windows(&snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::TempDir;

    fn write(path: &Path, text: &str) {
        if let Err(e) = std::fs::write(path, text) {
            panic!("write {}: {e}", path.display());
        }
    }

    #[test]
    fn one_bad_window_entry_does_not_lose_the_layout() {
        let dir = TempDir::new("config-windows");
        let p = dir.join("window.toml");
        write(
            &p,
            "[[window]]\nx = 10.0\ny = 20.0\nwidth = 800.0\nheight = 600.0\nzoom_level = 1.25\n\
             [[window]]\nx = 0.0\ny = 0.0\nwidth = \"wide\"\nheight = 600.0\n\
             [[window]]\nx = 5.0\ny = 6.0\nwidth = 700.0\nheight = 500.0\nzoom_level = \"big\"\n",
        );
        let got = load_windows_at(&p);
        assert_eq!(got.len(), 2, "only the entry without a usable width is skipped");
        assert_eq!(got[0].zoom_level, Some(1.25));
        assert!(got[0].into_bounds().is_some());
        assert_eq!(got[1].zoom_level, None, "a bad zoom costs only the zoom");
        assert!(got[1].into_bounds().is_some());
    }

    #[test]
    fn impossible_placement_and_zoom_fall_back_to_defaults() {
        let dir = TempDir::new("config-windows-nan");
        let p = dir.join("window.toml");
        write(
            &p,
            "[[window]]\nx = nan\ny = 0.0\nwidth = 800.0\nheight = 600.0\nzoom_level = nan\n\
             [[window]]\nx = 0.0\ny = 0.0\nwidth = 0.0\nheight = -5.0\nzoom_level = inf\n\
             [[window]]\nx = 0.0\ny = inf\nwidth = 800.0\nheight = 600.0\n",
        );
        let got = load_windows_at(&p);
        assert_eq!(got.len(), 3, "the windows still open, at the default placement");
        for w in &got {
            assert!(w.into_bounds().is_none());
            assert_eq!(w.zoom_level, None);
        }
    }

    #[test]
    fn legacy_single_record_still_loads() {
        let dir = TempDir::new("config-windows-legacy");
        let p = dir.join("window.toml");
        write(&p, "x = 1.0\ny = 2.0\nwidth = 900.0\nheight = 700.0\n");
        let got = load_windows_at(&p);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].zoom_level, None);
        assert!(got[0].into_bounds().is_some());
    }

    #[test]
    fn unreadable_window_file_is_kept_aside() {
        let dir = TempDir::new("config-windows-junk");
        let p = dir.join("window.toml");
        assert!(load_windows_at(&p).is_empty());
        write(&p, "[[window]\nx = ");
        assert!(load_windows_at(&p).is_empty());
        assert!(dir.join("window.toml.corrupt").exists());
    }

    #[test]
    fn saved_layout_round_trips() {
        let dir = TempDir::new("config-windows-save");
        let p = dir.join("sub").join("window.toml");
        let entries = [
            WindowEntry { x: 1.0, y: 2.0, width: 300.0, height: 400.0, zoom_level: Some(1.5) },
            WindowEntry { x: -50.0, y: 0.0, width: 640.0, height: 480.0, zoom_level: None },
        ];
        assert!(save_windows_at(&p, &entries).is_ok());
        let got = load_windows_at(&p);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].zoom_level, Some(1.5));
        assert_eq!(got[1].zoom_level, None);
        assert!(!dir.path().join("sub").join("window.toml.tmp").exists());
    }
}