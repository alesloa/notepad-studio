// SPDX-License-Identifier: Apache-2.0
//
// Cross-platform paths for theme storage. Notepad Studio's user-data root is a
// single directory under the platform's standard config / app-support location.
//
//   macOS:   $HOME/Library/Application Support/Notepad Studio/
//   Linux:   ${XDG_CONFIG_HOME:-$HOME/.config}/notepad-studio/
//   Windows: %APPDATA%\Notepad Studio\
//
// Inside that root:
//   - stylers.xml             — the active "Default" theme (per-user copy).
//   - themes/                 — user theme overrides + user-imported themes.
//
// Bundled themes are NOT on disk; they live in the binary as `&'static str`
// loaded via `include_str!` (see `bundled.rs`).

use std::path::PathBuf;

/// User-data root for Notepad Studio. Returns `None` when the platform's
/// directory resolver fails (extremely unusual on supported OSes).
pub fn user_data_root() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        // macOS: dirs::config_dir() returns ~/Library/Application Support/.
        dirs::config_dir().map(|p| p.join("Notepad Studio"))
    } else if cfg!(target_os = "linux") {
        // Linux: dirs::config_dir() honours $XDG_CONFIG_HOME or falls back to
        // ~/.config. Kebab-case "notepad-studio" matches XDG conventions.
        dirs::config_dir().map(|p| p.join("notepad-studio"))
    } else if cfg!(target_os = "windows") {
        // Windows: dirs::config_dir() returns %APPDATA% (Roaming).
        dirs::config_dir().map(|p| p.join("Notepad Studio"))
    } else {
        // Fallback: same as Linux pattern under home.
        dirs::config_dir().map(|p| p.join("notepad-studio"))
    }
}

/// Path to the active "Default" theme XML — `<root>/stylers.xml`.
pub fn stylers_xml_path() -> Option<PathBuf> {
    user_data_root().map(|root| root.join("stylers.xml"))
}

/// User-themes override directory — `<root>/themes/`.
pub fn user_themes_dir() -> Option<PathBuf> {
    user_data_root().map(|root| root.join("themes"))
}

/// Ensure the user-data root and themes directory both exist. Idempotent.
/// Returns `Ok(())` if they exist or were freshly created. Errors propagate
/// the underlying `io::Error`.
pub fn ensure_user_dirs() -> std::io::Result<()> {
    if let Some(root) = user_data_root() {
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(root.join("themes"))?;
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve user-data root for this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_root_resolves() {
        // We don't assert the exact path because the test runner's HOME may
        // vary, but every supported platform should resolve SOMETHING.
        assert!(user_data_root().is_some(), "user_data_root must resolve");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_path_format() {
        let p = user_data_root().expect("macOS path");
        let s = p.to_string_lossy();
        assert!(
            s.contains("Library/Application Support/Notepad Studio"),
            "expected Application Support/Notepad Studio, got {s}"
        );
    }

    #[test]
    fn stylers_path_underneath_root() {
        let root = user_data_root().expect("root");
        let stylers = stylers_xml_path().expect("stylers");
        assert!(stylers.starts_with(&root));
        assert_eq!(stylers.file_name().and_then(|s| s.to_str()), Some("stylers.xml"));
    }

    #[test]
    fn themes_dir_underneath_root() {
        let root = user_data_root().expect("root");
        let themes = user_themes_dir().expect("themes");
        assert!(themes.starts_with(&root));
        assert_eq!(themes.file_name().and_then(|s| s.to_str()), Some("themes"));
    }
}
