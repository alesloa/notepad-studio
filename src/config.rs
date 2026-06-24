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
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use gpui::{Bounds, Pixels, Window, WindowBounds, WindowId, point, px, size};

use crate::consts::APP_NAME;

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

    pub(crate) fn into_bounds(self) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        }
    }
}

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
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };

    // Try the new multi-window format first.
    if let Ok(file) = toml::from_str::<WindowsFile>(&raw) {
        if !file.windows.is_empty() {
            return file.windows;
        }
    }

    // Fall back to the legacy v0.0.3 single-record format.
    if let Ok(legacy) = toml::from_str::<WindowEntry>(&raw) {
        return vec![legacy];
    }

    Vec::new()
}

fn save_windows(entries: &[WindowEntry]) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = WindowsFile {
        windows: entries.to_vec(),
    };
    if let Ok(serialized) = toml::to_string_pretty(&file) {
        let _ = std::fs::write(&path, serialized);
    }
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
