// Single-instance ownership uses a stable OS-locked file. Activation requests
// are individually queued, atomically published and polled every 250 ms.
// The legacy PID/sentinel path remains readable for an already-running older app.

use std::{io::Read, path::PathBuf};
mod ipc;
static PRIMARY: Mutex<Option<ipc::Primary>> = Mutex::new(None);
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
    // PID files are only the upgrade bridge. Verify the executable as well
    // as existence, so a reused PID cannot swallow a future launch.
    Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="])
        .output().ok().filter(|output| output.status.success()).is_some_and(|output| {
            let command = String::from_utf8_lossy(&output.stdout);
            std::path::Path::new(command.trim()).file_name().is_some_and(|name| name == "notepad-studio")
        })
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    // Require both executable name and PID in tasklist's CSV output.
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output();
    match out {
        Ok(output) => String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split(',').map(|field| field.trim().trim_matches('"'));
            fields.next().is_some_and(|name| name.eq_ignore_ascii_case("notepad-studio.exe"))
                && fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(pid)
        }),
        Err(_) => false,
    }
}

#[cfg(not(any(unix, windows)))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// Read a PID from `.lock` if present and well-formed.
fn read_lock_pid() -> Option<u32> {
    let mut raw = String::new();
    std::fs::File::open(lock_path()?).ok()?.take(32).read_to_string(&mut raw).ok()?;
    raw.trim().parse::<u32>().ok()
}

fn write_lock() -> std::io::Result<()> {
    let path = lock_path().ok_or_else(|| std::io::Error::other("configuration directory is unavailable"))?;
    std::fs::write(path, std::process::id().to_string())
}

/// Drop a sentinel file the existing instance polls for. The poller calls
/// `raise_all_app_windows` (and opens any forwarded paths) when it sees the
/// file. Sentinel content is newline-separated absolute paths — empty body
/// means "just raise". Best-effort.
fn write_activation_sentinel(paths: &[PathBuf]) -> std::io::Result<()> {
    let path = activation_sentinel_path().ok_or_else(|| std::io::Error::other("configuration directory is unavailable"))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content: String = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, content)
}

/// Pull positional file path arguments off `std::env::args_os()`, drop any
/// `-`-prefixed flag, and absolutize relative paths against the CWD so the
/// running instance can open them regardless of where the launcher's cwd
/// pointed. Returns the resolved `PathBuf`s in argv order.
fn parse_file_path_args() -> Vec<PathBuf> {
    let cwd = std::env::current_dir().ok(); let mut positional = false;
    std::env::args_os().skip(1).filter(|arg| {
        if !positional && arg == "--" { positional = true; return false; }
        positional || !arg.as_encoded_bytes().starts_with(b"-")
    }).map(PathBuf::from).map(|path| {
        if path.is_absolute() { path } else { cwd.as_ref().map_or_else(|| path.clone(), |cwd| cwd.join(&path)) }
    }).collect()
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
pub(crate) fn acquire_or_raise() -> Result<(), String> {
    if matches!(crate::settings::snapshot().multi_instance_mode,
        crate::settings::MultiInstanceMode::AlwaysMulti | crate::settings::MultiInstanceMode::NewInstanceForSession) { return Ok(()); }
    let root = dirs::config_dir().map(|directory| directory.join(APP_NAME)).ok_or("configuration directory is unavailable")?;
    let owner = ipc::claim(&root).map_err(|error| format!("Cannot acquire the editor instance lock: {error}"))?;
    let owner = match owner {
        Some(owner) => owner,
        None => {
            let queued = ipc::enqueue(&root, &parse_file_path_args()).map_err(|error| format!("Cannot forward files to the running editor: {error}"))?;
            // If the primary closed during publication, take ownership ourselves
            // and let normal argv opening consume this launch exactly once.
            if let Some(owner) = ipc::claim(&root).map_err(|error| error.to_string())? {
                if let Err(error) = std::fs::remove_file(queued) {
                    if error.kind() != std::io::ErrorKind::NotFound { return Err(error.to_string()); }
                }
                owner
            } else { std::process::exit(0); }
        }
    };
    // An old version does not hold the new OS lock. Hand off through its old
    // protocol rather than starting a second writer against its live session.
    if read_lock_pid().is_some_and(|pid| pid != std::process::id() && pid_alive(pid)) {
        write_activation_sentinel(&parse_file_path_args()).map_err(|error| format!("Cannot activate the running editor: {error}"))?;
        drop(owner); std::process::exit(0);
    }
    write_lock().map_err(|error| format!("Cannot record editor ownership: {error}"))?;
    let mut held = PRIMARY.lock().map_err(|_| "editor ownership state is unavailable")?;
    *held = Some(owner); Ok(())
}

/// Release only this process's ownership, irrespective of a preference change.
/// Never unlink the OS-lock file: replacing its inode would allow two owners.
pub(crate) fn release() {
    let Ok(mut held) = PRIMARY.lock() else { return; };
    if held.is_some() {
        if read_lock_pid() == Some(std::process::id()) {
            if let Some(path) = lock_path() { let _ = std::fs::remove_file(path); }
        }
        held.take();
    }
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

            if cx.update(|cx| crate::app::startup::blocks_editor(cx)) { continue; }
            if let Some(root) = dirs::config_dir().map(|directory| directory.join(APP_NAME)) {
                let (paths, raise) = cx.background_executor().spawn(async move { ipc::drain(&root) }).await;
                if raise {
                    cx.update(|cx| {
                        if paths.is_empty() { crate::app::window::raise_all_app_windows(cx); }
                        for path in paths { crate::app::window::open_path_in_active_or_new_window(path, cx); }
                    });
                }
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
            let mut content = String::new();
            if let Ok(file) = std::fs::File::open(&path) {
                if let Err(error) = file.take(1024 * 1024).read_to_string(&mut content) {
                    eprintln!("[single-instance] cannot read legacy activation request: {error}");
                    content.clear();
                }
            }
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
