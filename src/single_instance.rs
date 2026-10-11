// Single-instance lock — at most one Notepad Studio process at a time. Implemented
// with a PID file at `<config>/Notepad Studio/.lock` and a poll-based "raise
// request" sentinel at `<config>/Notepad Studio/.activate-request`. No background
// threads — the activation poller runs on gpui's foreground task pool via
// `cx.spawn`.
//
// Cross-platform PID liveness uses `kill -0 <pid>` on Unix (POSIX) and the
// `tasklist` shell tool on Windows. Both are stdlib `Command` calls so we
// don't pull in any new crates.
//
// Lifecycle:
// 1. `acquire_or_raise()` runs at the top of `main`, before any window is
//    spawned. If a live PID is in `.lock`, drop a sentinel file the running
//    instance picks up via the poller, then `std::process::exit(0)`. If the
//    `.lock` is stale (PID no longer alive), delete and continue.
// 2. The launching process writes its own PID to `.lock`.
// 3. `start_activation_poller(cx)` spawns a 250ms poll loop that watches
//    for the sentinel; on hit, raises every app window so modeless dialogs
//    come forward with their editor window on second-launch attempts.
// 4. `release()` runs at quit time and deletes `.lock`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use gpui::App;

use crate::consts::APP_NAME;

/// Paths that need to be opened in this process once windows are up. Pushed
/// to from three sources:
///   - cold-start argv parsing in `acquire_or_raise` (Linux/Windows: file
///     paths come in via the command line when the OS / shell launches us)
///   - the gpui `on_open_urls` callback wired in `main` (macOS: Finder's
///     Open With / drop-on-icon delivers `file://…` URLs to the running app
///     via the NSApplication delegate, not via argv)
///   - the activation poller's sentinel-handoff path (a second-launch
///     attempt writes paths to the sentinel and exits; the live instance
///     drains them here)
///
/// The activation poller drains the queue on every tick and calls
/// `open_path_in_active_or_new_window` for each path.
static PENDING_OPEN_PATHS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Push a single path onto the pending-open queue. Safe to call from any
/// thread or from inside the gpui `on_open_urls` callback (no `cx` needed).
pub(crate) fn pending_open_paths_push(path: PathBuf) {
    if let Ok(mut q) = PENDING_OPEN_PATHS.lock() {
        q.push(path);
    }
}

fn pending_open_paths_drain() -> Vec<PathBuf> {
    PENDING_OPEN_PATHS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

fn lock_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(".lock"))
}

fn activation_sentinel_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(".activate-request"))
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // `kill -0 <pid>` returns 0 if the process exists and the caller has
    // permission to signal it, non-zero otherwise. Permission errors are
    // also treated as "alive" (it's still running, we just can't signal).
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    // `tasklist /FI "PID eq <pid>" /NH` prints a row when the process is
    // alive and the literal "INFO: No tasks are running..." otherwise.
    // Match on the row presence by checking output contains the PID string.
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains(&pid.to_string())
        }
        Err(_) => false,
    }
}

#[cfg(not(any(unix, windows)))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// Read a PID from `.lock` if present and well-formed.
fn read_lock_pid() -> Option<u32> {
    let path = lock_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    raw.trim().parse::<u32>().ok()
}

/// Write the current process's PID to `.lock`. Creates the parent dir if
/// missing. Best-effort — failure is logged and ignored so the launch
/// proceeds (the lock is a hint, not a hard guarantee).
fn write_lock() {
    let Some(path) = lock_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, std::process::id().to_string()) {
        eprintln!("[single-instance] failed to write lock file: {err}");
    }
}

/// Drop a sentinel file the existing instance polls for. The poller calls
/// `raise_all_app_windows` (and opens any forwarded paths) when it sees the
/// file. Sentinel content is newline-separated absolute paths — empty body
/// means "just raise". Best-effort.
fn write_activation_sentinel(paths: &[PathBuf]) {
    let Some(path) = activation_sentinel_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content: String = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&path, content);
}

/// Pull positional file path arguments off `std::env::args()`, drop any
/// `-`-prefixed flag, and absolutize relative paths against the CWD so the
/// running instance can open them regardless of where the launcher's cwd
/// pointed. Returns the resolved `PathBuf`s in argv order.
fn parse_file_path_args() -> Vec<PathBuf> {
    let cwd = std::env::current_dir().ok();
    std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .map(|p| {
            if p.is_absolute() {
                p
            } else if let Some(base) = cwd.as_ref() {
                base.join(p)
            } else {
                p
            }
        })
        .collect()
}

/// Parse a `file://` URL (RFC 3986 percent-encoded) into a `PathBuf`. Used
/// to translate the strings macOS's `application:openURLs:` delegate hands
/// to gpui's `on_open_urls` callback. Returns `None` for non-`file:` URLs
/// or malformed input. Hosts other than the local machine are not
/// supported (no SMB / network mount handling).
pub(crate) fn parse_file_url(url: &str) -> Option<PathBuf> {
    let after_scheme = url.strip_prefix("file://")?;
    // `file:///abs/path` → leading '/' is the start of the absolute path.
    // `file://host/abs/path` → drop the `host` segment, keep `/abs/path`.
    let raw_path = if after_scheme.starts_with('/') {
        after_scheme
    } else {
        let slash = after_scheme.find('/')?;
        &after_scheme[slash..]
    };
    Some(PathBuf::from(percent_decode(raw_path)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Queue every positional file path from this process's argv onto the
/// pending-open list. Called from `main` after `acquire_or_raise` returns
/// — i.e. only in the process that owns the single-instance lock (or in
/// every process when multi-instance mode is enabled). Cold-start
/// `Exec=notepad-studio %F` on Linux and Explorer "Open with…" on Windows
/// both deliver paths via argv; this hook turns them into the same
/// pending-open path everything else uses.
pub(crate) fn queue_argv_files() {
    for p in parse_file_path_args() {
        pending_open_paths_push(p);
    }
}

/// Top-of-main entry point. Returns to the caller if this process owns the
/// lock; calls `std::process::exit(0)` if another live instance is running
/// (after dropping the activation sentinel so the running instance comes
/// to the foreground).
///
/// `Settings.multi_instance_mode` (NPP `Multi-Instance & Date ▸ Multi-instance
/// settings *`) gates the lock entirely:
/// - `MonoInstance` (default) — current behaviour: raise existing instance.
/// - `AlwaysMulti` / `NewInstanceForSession` — skip the lock so each launch
///   is a fresh process. The third variant's session-handoff side is partially
///   deferred (see UNTESTED.md); both are treated as "skip the lock" today.
pub(crate) fn acquire_or_raise() {
    if matches!(
        crate::settings::snapshot().multi_instance_mode,
        crate::settings::MultiInstanceMode::AlwaysMulti
            | crate::settings::MultiInstanceMode::NewInstanceForSession
    ) {
        // Multi-instance: every launch is its own process. Don't touch the
        // lock or sentinel — leaving the existing mono-instance lock alone
        // means a switch back to MonoInstance still works on the very next
        // launch without a manual cleanup step.
        return;
    }
    if let Some(pid) = read_lock_pid() {
        if pid != std::process::id() && pid_alive(pid) {
            // Another instance is alive. Forward any argv file paths to it
            // via the sentinel, then exit silently with code 0 — matches
            // Notepad++ behaviour and avoids a confusing "second window
            // appeared briefly then vanished" flicker. An empty sentinel
            // (no argv paths) tells the running instance to just raise.
            let paths = parse_file_path_args();
            write_activation_sentinel(&paths);
            std::process::exit(0);
        }
        // Stale lock from a crashed prior process — drop it.
        if let Some(path) = lock_path() {
            let _ = std::fs::remove_file(&path);
        }
    }
    write_lock();
}

/// Drop the lock file. Called from the app-quit hook so a clean exit leaves
/// the next launch unencumbered. A crash leaves the lock behind; the next
/// launch will read it, see the dead PID, and clear it via the stale-lock
/// branch above.
///
/// Multi-instance launches don't write the lock, so they also don't touch it
/// on quit — clobbering the file would strand a peer mono-instance process.
pub(crate) fn release() {
    if matches!(
        crate::settings::snapshot().multi_instance_mode,
        crate::settings::MultiInstanceMode::AlwaysMulti
            | crate::settings::MultiInstanceMode::NewInstanceForSession
    ) {
        return;
    }
    let Some(path) = lock_path() else {
        return;
    };
    let _ = std::fs::remove_file(&path);
}

pub(crate) fn take_ready_open_paths(cx: &mut App) -> Vec<PathBuf> {
    if crate::app::startup::blocks_editor(cx) { Vec::new() } else { pending_open_paths_drain() }
}

/// Spawn a foreground gpui task that polls every 250ms for:
///   - any pending file paths queued via `pending_open_paths_push` (the
///     cold-start argv path, the macOS `on_open_urls` callback path)
///   - the activation sentinel file (a second-launch attempt asking the
///     live instance to either raise or open paths it forwarded)
///
/// Both sources are drained on every tick. Detached — the task runs for
/// the lifetime of the app.
pub(crate) fn start_activation_poller(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            gpui::Timer::after(Duration::from_millis(250)).await;

            // 1. Drain the in-process pending-open queue. Covers cold-start
            //    argv (Linux/Windows) and macOS `on_open_urls` events.
            let pending = cx.update(take_ready_open_paths);
            if !pending.is_empty() {
                let _ = cx.update(|cx| {
                    for p in pending {
                        crate::app::window::open_path_in_active_or_new_window(p, cx);
                    }
                });
            }

            // 2. Drain the activation sentinel. A second-launch process
            //    writes either an empty body (just raise) or a newline-
            //    separated list of absolute paths to open.
            let Some(path) = activation_sentinel_path() else {
                continue;
            };
            if !path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let _ = std::fs::remove_file(&path);

            let forwarded: Vec<PathBuf> = content
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect();

            let _ = cx.update(|cx| {
                if forwarded.is_empty() {
                    // No paths — second launch just wants attention.
                    crate::app::window::raise_all_app_windows(cx);
                } else {
                    // Paths present — open each. open_path_in_active_or_new_window
                    // already brings the target window forward, so the raise
                    // happens for free.
                    for p in forwarded {
                        crate::app::window::open_path_in_active_or_new_window(p, cx);
                    }
                }
            });
        }
    })
    .detach();
}
