// Cross-platform shell helpers — reveal a path in the OS file manager, open
// a terminal at a directory, and open a file with the OS default viewer.
//
// All operations spawn a child process and return immediately; success means
// "the launcher accepted the request", not "the user finished interacting".
// Errors bubble up as `Result<(), String>` so callers can flash a status
// line. None of these touch the rope / buffer state.

use std::path::Path;
use std::process::Command;

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
/// `xdg-open` on Linux, the `start` shell builtin on Windows.
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
        Command::new("cmd")
            .args(["/C", "start", "\"\""])
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
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
