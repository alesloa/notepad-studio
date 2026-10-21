// Cross-platform shell helpers — reveal a path in the OS file manager, open
// a terminal at a directory, and open a file with the OS default viewer.
//
// All operations spawn a child process and return immediately; success means
// "the launcher accepted the request", not "the user finished interacting".
// Errors bubble up as `Result<(), String>` so callers can flash a status
// line. None of these touch the rope / buffer state.

use std::path::Path;
use std::process::Command;

/// URLs stay data for the existing `open` dependency, including its native
/// ShellExecute backend on Windows. Never interpolate a link into a shell.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    open_url_with(url, |url| open::that_detached(url))
}

fn open_url_with(url: &str, launch: impl FnOnce(&str) -> std::io::Result<()>) -> Result<(), String> {
    if url.is_empty() || url.contains('\0') || url.starts_with('-') {
        return Err("Invalid link".into());
    }
    launch(url).map_err(|error| error.to_string())
}

/// Open the system file manager focused on `path` — Finder reveals the file
/// on macOS, Explorer with `/select` on Windows, and `xdg-open` on the
/// containing directory on Linux (no select-equivalent in the freedesktop
/// stack).
pub(crate) fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = path.parent().unwrap_or(Path::new("."));
        Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("Unsupported platform".to_string())
}

/// Open a terminal window with `dir` as its working directory. Linux walks
/// a fallback chain because the freedesktop spec has no canonical terminal
/// — try gnome-terminal first, then konsole, then xterm.
pub(crate) fn open_terminal_at(dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-a")
            .arg("Terminal")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        // `start` is a cmd builtin — invoke through cmd /C. The empty quoted
        // string is the window title slot; without it, cmd treats the next
        // quoted arg as the title and silently drops it.
        Command::new("cmd")
            .args(["/C", "start", "\"\"", "cmd", "/K"])
            .arg(format!("cd /D \"{}\"", dir.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let attempts: &[(&str, &[&str])] = &[
            ("gnome-terminal", &["--working-directory"]),
            ("konsole", &["--workdir"]),
            ("xfce4-terminal", &["--working-directory"]),
            ("xterm", &["-e"]),
        ];
        for (cmd, flags) in attempts {
            let mut command = Command::new(cmd);
            if *cmd == "xterm" {
                command.arg("-e").arg(format!(
                    "cd {} && ${{SHELL:-/bin/bash}}",
                    dir.display()
                ));
            } else {
                for f in *flags {
                    command.arg(f);
                }
                command.arg(dir);
            }
            if command.spawn().is_ok() {
                return Ok(());
            }
        }
        return Err("No supported terminal emulator found".to_string());
    }
    #[allow(unreachable_code)]
    Err("Unsupported platform".to_string())
}

/// Open `path` with the OS default viewer/handler — `open` on macOS,
/// `xdg-open` on Linux, native ShellExecute on Windows.
pub(crate) fn open_with_default_viewer(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        // open 5.3.4, windows.rs:40 — the enabled shellexecute-on-windows
        // feature sends lpFile as UTF-16 data, without CMD or a process wait.
        return open::that_detached(path).map_err(|error| error.to_string());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("Unsupported platform".to_string())
}

#[cfg(test)]
mod url_tests {
    #[test]
    fn links_reach_the_native_launcher_unchanged_and_errors_are_returned() {
        let url = "https://example.com/?first=1&second=é%20中#fragment";
        assert!(super::open_url_with(url, |actual| { assert_eq!(actual, url); Ok(()) }).is_ok());
        let error = super::open_url_with(url, |_| Err(std::io::Error::other("controlled launch failure")));
        assert_eq!(error.err().as_deref(), Some("controlled launch failure"));
        for invalid in ["", "-bad", "https://example.com/\0"] {
            assert!(super::open_url_with(invalid, |_| panic!("invalid link reached launcher")).is_err());
        }
    }
}
