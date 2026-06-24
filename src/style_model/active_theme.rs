// SPDX-License-Identifier: Apache-2.0
//
// Process-global cache for the active theme XML. Mirrors the pattern used
// by `crate::settings` — load lazily on first read, parse once, cache the
// result. `Workspace::theme()` calls `snapshot()` on every render, so the
// path equality check + clone-on-hit must stay cheap.
//
// Cache invalidation:
//   * automatic — when the cached `path` no longer matches
//     `Settings.theme_path` (theme switched via the dialog or another
//     window).
//   * explicit  — `invalidate()` after Style Configurator's Save & Close
//     so on-disk edits the user just wrote land on the very next render.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::xml_load::{self, ThemeDocument};

static CACHE: OnceLock<Mutex<Option<(PathBuf, ThemeDocument)>>> = OnceLock::new();
static PREVIEW: OnceLock<Mutex<Option<ThemeDocument>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<(PathBuf, ThemeDocument)>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

fn preview_cell() -> &'static Mutex<Option<ThemeDocument>> {
    PREVIEW.get_or_init(|| Mutex::new(None))
}

/// Install the Style Configurator's in-memory document as a live preview
/// override. While `Some`, every `snapshot()` call returns this document
/// instead of consulting `Settings.theme_path` — letting the editor repaint
/// with the user's pending theme/edit *without* persisting anything to
/// disk. Pass `None` (typically from the configurator's release observer
/// or on Cancel) to drop the override; the editor then falls back to the
/// on-disk file pointed at by Settings, which is unchanged unless the
/// user clicked Save.
pub fn set_preview(doc: Option<ThemeDocument>) {
    if let Ok(mut g) = preview_cell().lock() {
        *g = doc;
    }
}

/// Snapshot the parsed theme XML for `Settings.theme_path`. Returns `None`
/// when the user has no theme picked, the file can't be read, or parsing
/// fails — every callsite must tolerate `None` (vanilla theme rendering).
///
/// When the Style Configurator is open and has installed a preview via
/// `set_preview`, that document is returned directly — the on-disk path
/// is bypassed entirely so the user sees their pending pick without a
/// commit-to-disk round trip.
///
/// Cheap on cache hit (one path equality + one clone of `ThemeDocument`).
/// Cache miss reads + parses XML once, then every subsequent call hits.
pub fn snapshot() -> Option<ThemeDocument> {
    if let Ok(g) = preview_cell().lock() {
        if let Some(d) = g.as_ref() {
            return Some(d.clone());
        }
    }
    let path = crate::settings::snapshot().theme_path?;
    let mut guard = cell().lock().ok()?;
    if let Some((cached_path, doc)) = guard.as_ref() {
        if cached_path == &path {
            return Some(doc.clone());
        }
    }
    let xml = match std::fs::read_to_string(&path) {
        Ok(x) => x,
        Err(err) => {
            eprintln!("active_theme: failed to read {path:?}: {err}");
            return None;
        }
    };
    let doc = match xml_load::parse(&xml) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("active_theme: failed to parse {path:?}: {err:?}");
            return None;
        }
    };
    eprintln!(
        "active_theme: loaded {path:?} ({} lexers, {} globals)",
        doc.lexers.len(),
        doc.global_styles.len()
    );
    *guard = Some((path, doc.clone()));
    Some(doc)
}

/// Force a reload from disk on the next `snapshot()` call. The Style
/// Configurator's Save & Close path calls this so edits the user just
/// wrote to the theme XML are picked up by the editor without a relaunch.
pub fn invalidate() {
    if let Ok(mut g) = cell().lock() {
        *g = None;
    }
}
