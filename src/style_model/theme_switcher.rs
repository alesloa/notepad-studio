// SPDX-License-Identifier: Apache-2.0
//
// `ThemeSwitcher::discover()` — builds the list shown in the dialog's theme
// dropdown. Mirrors NPP's `Notepad_plus_Window.cpp:225` enumeration:
//
//   1. Synthetic "Default" entry pointing at `<userdata>/stylers.xml`.
//   2. Every `*.xml` in `<userdata>/themes/` (user overrides + imports).
//   3. Every bundled theme NOT already represented by a user-override XML.
//      Bundled themes register a save-redirect to `<userdata>/themes/<name>.xml`
//      so "Save & Close" never overwrites the in-binary copy.

use std::path::PathBuf;

use crate::style_model::bundled;
use crate::style_model::paths;

/// One row in the theme dropdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeEntry {
    /// Display label in the combo box.
    pub display_name: String,
    /// Source: a path on disk, OR a bundled in-binary string.
    pub source: ThemeSource,
    /// When `Some`, "Save & Close" writes here instead of overwriting `source`.
    /// Used to redirect bundled-theme edits into the user-overrides directory.
    pub save_redirect: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSource {
    /// File on disk. Read with `std::fs::read_to_string` at load time.
    UserFile(PathBuf),
    /// Bundled theme; the inner string IS the XML.
    Bundled(&'static str),
}

/// Build the theme list per NPP discovery rules.
///
/// `user_root` and `themes_dir` are passed in so tests can use a temp dir
/// instead of the real platform path. Production code calls `discover_default`.
pub fn discover_with(
    user_root: Option<&std::path::Path>,
    themes_dir: Option<&std::path::Path>,
) -> Vec<ThemeEntry> {
    let mut out = Vec::new();

    // 1. Synthetic "Default" → <root>/stylers.xml.
    if let Some(root) = user_root {
        out.push(ThemeEntry {
            display_name: "Default".to_string(),
            source: ThemeSource::UserFile(root.join("stylers.xml")),
            save_redirect: None,
        });
    }

    // 2. *.xml in user themes dir.
    let mut user_names: Vec<String> = Vec::new();
    if let Some(themes) = themes_dir {
        if let Ok(entries) = std::fs::read_dir(themes) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("xml") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        user_names.push(stem.to_string());
                        out.push(ThemeEntry {
                            display_name: stem.to_string(),
                            source: ThemeSource::UserFile(p.clone()),
                            save_redirect: None,
                        });
                    }
                }
            }
        }
    }

    // 3. Bundled themes — only if no user override of the same stem exists.
    for name in bundled::names() {
        if user_names.iter().any(|n| n == name) {
            continue;
        }
        let redirect = themes_dir.map(|d| d.join(format!("{name}.xml")));
        out.push(ThemeEntry {
            display_name: name.to_string(),
            source: ThemeSource::Bundled(bundled::get(name).expect("bundled lookup")),
            save_redirect: redirect,
        });
    }

    // Sort everything except the synthetic "Default" entry alphabetically
    // (case-insensitive) — NPP parity.
    if out.len() > 1 {
        let tail = &mut out[1..];
        tail.sort_by(|a, b| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        });
    }

    out
}

/// Production entry point: enumerate using the platform's real config dirs.
pub fn discover() -> Vec<ThemeEntry> {
    let root = paths::user_data_root();
    let themes = paths::user_themes_dir();
    discover_with(root.as_deref(), themes.as_deref())
}

/// Resolve the path that "Save & Close" should write to for a given theme.
/// - `save_redirect` (bundled override path) wins when set.
/// - Otherwise return the on-disk source path.
/// - Bundled themes without a redirect cannot be saved (caller decides).
pub fn save_path_for(entry: &ThemeEntry) -> Option<PathBuf> {
    if let Some(p) = &entry.save_redirect {
        return Some(p.clone());
    }
    match &entry.source {
        ThemeSource::UserFile(p) => Some(p.clone()),
        ThemeSource::Bundled(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_user_dirs_lists_default_plus_bundled() {
        let tmp = tempdir();
        let user_root = tmp.path();
        let themes_dir = tmp.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();

        let list = discover_with(Some(user_root), Some(&themes_dir));
        assert_eq!(list[0].display_name, "Default");
        let bundled_count = list
            .iter()
            .filter(|e| matches!(e.source, ThemeSource::Bundled(_)))
            .count();
        assert!(bundled_count >= 2, "expected at least 2 bundled themes");
    }

    #[test]
    fn user_override_wins_over_bundle() {
        let tmp = tempdir();
        let user_root = tmp.path();
        let themes_dir = tmp.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        // Create a user override that masks the bundled "NotepadStudioLight".
        let override_path = themes_dir.join("NotepadStudioLight.xml");
        std::fs::write(&override_path, "<NotepadPlus></NotepadPlus>").unwrap();

        let list = discover_with(Some(user_root), Some(&themes_dir));
        let light_entries: Vec<_> = list
            .iter()
            .filter(|e| e.display_name == "NotepadStudioLight")
            .collect();
        assert_eq!(light_entries.len(), 1, "no duplicate NotepadStudioLight entries");
        assert!(matches!(
            light_entries[0].source,
            ThemeSource::UserFile(_)
        ));
        assert!(light_entries[0].save_redirect.is_none());
    }

    #[test]
    fn bundled_save_redirect_set() {
        let tmp = tempdir();
        let themes_dir = tmp.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        let list = discover_with(Some(tmp.path()), Some(&themes_dir));
        let bundled_entry = list
            .iter()
            .find(|e| matches!(e.source, ThemeSource::Bundled(_)))
            .expect("at least one bundled entry");
        let redirect = bundled_entry
            .save_redirect
            .as_ref()
            .expect("bundled must have a save redirect");
        assert!(redirect.starts_with(&themes_dir));
    }

    #[test]
    fn save_path_resolution() {
        // Bundled with redirect → returns the redirect.
        let bundled_entry = ThemeEntry {
            display_name: "X".into(),
            source: ThemeSource::Bundled("<n />"),
            save_redirect: Some(PathBuf::from("/tmp/x.xml")),
        };
        assert_eq!(save_path_for(&bundled_entry), Some(PathBuf::from("/tmp/x.xml")));

        // Bundled without redirect → None.
        let bundled_no_redirect = ThemeEntry {
            display_name: "Y".into(),
            source: ThemeSource::Bundled("<n />"),
            save_redirect: None,
        };
        assert_eq!(save_path_for(&bundled_no_redirect), None);

        // User file → that file.
        let user_entry = ThemeEntry {
            display_name: "Z".into(),
            source: ThemeSource::UserFile(PathBuf::from("/u/z.xml")),
            save_redirect: None,
        };
        assert_eq!(save_path_for(&user_entry), Some(PathBuf::from("/u/z.xml")));
    }

    /// Tiny inline temp-dir helper (avoids adding `tempfile` as a dep).
    fn tempdir() -> TempDir {
        let mut path = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        path.push(format!("notepad-studio-style-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
