// Notepad++-style change.log auto-open. On first launch and on every
// version bump, append `change.log` (read-only, ephemeral) to the active
// workspace. Marker file at `<config>/Notepad Studio/changelog_seen.toml` tracks
// the last-seen version so subsequent launches with the same version do
// not re-open the tab.
//
// Path resolution favours the bundled / installed asset first, then walks
// up from the executable to the repo root for `cargo run` development.
// Returns None when no candidate exists — the caller silently skips the
// auto-open in that case.

use std::path::{Path, PathBuf};

use gpui::App;
use serde::{Deserialize, Serialize};

use crate::consts::APP_NAME;

const MARKER_FILENAME: &str = "changelog_seen.toml";
const CHANGELOG_FILENAME: &str = "change.log";

#[derive(Debug, Default, Serialize, Deserialize)]
struct SeenMarker {
    last_version: Option<String>,
}

fn marker_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(MARKER_FILENAME))
}

fn load_marker() -> SeenMarker {
    let Some(path) = marker_path() else {
        return SeenMarker::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return SeenMarker::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

fn save_marker(version: &str) {
    let Some(path) = marker_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let marker = SeenMarker {
        last_version: Some(version.to_string()),
    };
    if let Ok(text) = toml::to_string_pretty(&marker) {
        let _ = std::fs::write(&path, text);
    }
}

/// Resolve the on-disk location of `change.log`. Order:
///   1. `cargo run` dev: walk up from current_exe() looking for a Cargo.toml
///      sibling, return the workspace root's `change.log` if it exists.
///   2. macOS .app bundle: `<bundle>.app/Contents/Resources/change.log`.
///   3. Plain binary install (Linux/Windows or non-bundled mac): same dir
///      as the executable.
///
/// Returns None when no candidate exists. Caller should then write the
/// current version to the marker and bail — we do NOT want to retry the
/// resolve every launch.
pub(crate) fn changelog_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;

    // Dev path first — walk up until a `Cargo.toml` is found and check its
    // sibling `change.log`. Order matters: keep ahead of bundle/binary
    // resolution so `cargo run` from the repo always picks up the latest
    // unbundled file.
    if let Some(repo_root) = walk_to_cargo_root(&exe) {
        let candidate = repo_root.join(CHANGELOG_FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // macOS .app bundle: <…>/Notepad Studio.app/Contents/MacOS/notepad-studio → look
    // sideways for Resources/change.log.
    #[cfg(target_os = "macos")]
    {
        if let Some(resources) = bundle_resources_dir(&exe) {
            let candidate = resources.join(CHANGELOG_FILENAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // Plain binary install — change.log lives next to the executable.
    if let Some(parent) = exe.parent() {
        let candidate = parent.join(CHANGELOG_FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn walk_to_cargo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.parent()?;
    loop {
        if cur.join("Cargo.toml").is_file() {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
}

#[cfg(target_os = "macos")]
fn bundle_resources_dir(exe: &Path) -> Option<PathBuf> {
    // Standard layout: Foo.app/Contents/MacOS/<binary>.
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()?.to_str()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?;
    if contents.file_name()?.to_str()? != "Contents" {
        return None;
    }
    Some(contents.join("Resources"))
}

/// Open `change.log` in the first available Workspace iff the current
/// `CARGO_PKG_VERSION` differs from the last-seen marker. Returns true
/// when a tab was opened.
///
/// Call only on cold start — the dock-reopen path on macOS is a session
/// resume, not a fresh launch, so we skip it there.
pub(crate) fn maybe_open_changelog(cx: &mut App) -> bool {
    let current_version = env!("CARGO_PKG_VERSION");
    let marker = load_marker();
    let already_seen = marker
        .last_version
        .as_deref()
        .map(|v| v == current_version)
        .unwrap_or(false);
    if already_seen {
        return false;
    }

    let Some(path) = changelog_path() else {
        // No change.log on disk — record the current version so we don't
        // poll for it every launch. If a future install drops the file
        // into place we'll still skip it; that's a one-time miss in
        // exchange for avoiding repeated stat-misses on every cold start.
        save_marker(current_version);
        return false;
    };

    // Find the first Workspace window.
    let opened = cx
        .windows()
        .into_iter()
        .find_map(|h| h.downcast::<crate::workspace::Workspace>())
        .map(|workspace| {
            workspace
                .update(cx, |ws, _window, cx| {
                    ws.open_changelog_silent(path.clone(), cx);
                })
                .is_ok()
        })
        .unwrap_or(false);

    if opened {
        save_marker(current_version);
    }
    opened
}
