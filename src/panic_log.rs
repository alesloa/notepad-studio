// Panic hook writes a timestamped record to a platform-appropriate log file:
//   * macOS:   ~/Library/Logs/Notepad Studio/panic.log
//   * Linux:   $XDG_STATE_HOME/Notepad Studio/panic.log (fallback ~/.local/state/...)
//   * Windows: %LOCALAPPDATA%\Notepad Studio\Logs\panic.log
// The default unwind/abort behavior is preserved — we just append before
// chaining to the previously installed hook.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

use crate::consts::APP_NAME;

fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return dirs::home_dir().map(|h| h.join("Library/Logs").join(APP_NAME));
    }
    #[cfg(target_os = "linux")]
    {
        return dirs::state_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))
            .map(|d| d.join(APP_NAME));
    }
    #[cfg(target_os = "windows")]
    {
        return dirs::data_local_dir().map(|d| d.join(APP_NAME).join("Logs"));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        dirs::data_local_dir().map(|d| d.join(APP_NAME).join("Logs"))
    }
}

pub(crate) fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = log_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("panic.log");
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let location = info
                    .location()
                    .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                    .unwrap_or_else(|| "<unknown location>".to_string());
                let payload = info.payload();
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "<non-string panic payload>".to_string()
                };
                let backtrace = std::backtrace::Backtrace::force_capture();
                let _ = writeln!(
                    file,
                    "[t={timestamp}] panic at {location}: {msg}\n{backtrace}\n----"
                );
            }
        }
        previous(info);
    }));
}
