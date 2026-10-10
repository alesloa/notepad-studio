// Opening a file that is already open switches to its tab instead of loading
// a second copy, as Notepad++ does.
// PowerEditor/src/NppIO.cpp:305 — doOpen looks the name up with
// getBufferFromName and returns the found buffer rather than reopening it.

use std::path::{Path, PathBuf};

use gpui::Context;

use super::Workspace;

/// Notepad++'s refusal when Save As targets a file open in another tab
/// (NppIO.cpp:2081), naming this product.
pub(crate) fn already_open_message() -> String {
    format!("The file is already opened in {}.", crate::consts::APP_NAME)
}

impl Workspace {
    /// Index of the tab showing `path`, if any.
    pub(crate) fn open_tab_index_for(&self, path: &Path) -> Option<usize> {
        let paths: Vec<Option<&Path>> = self
            .tabs
            .iter()
            .map(|t| t.buffer.file_path.as_deref())
            .collect();
        open_tab_index(&paths, path, |p| std::fs::canonicalize(p).ok())
    }

    /// Activate the tab already showing `path`, if there is one. Returns
    /// whether it found one.
    pub(crate) fn activate_open_tab_for(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let Some(idx) = self.open_tab_index_for(path) else {
            return false;
        };
        self.set_active_tab(idx, cx);
        cx.notify();
        true
    }

    /// Save As guard: when `target` is open in a tab other than `saving`,
    /// switch to that tab and return true, so the caller refuses the save
    /// with `already_open_message` (NppIO.cpp:2079-2087).
    pub(crate) fn save_target_open_elsewhere(
        &mut self,
        target: &Path,
        saving: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.open_tab_index_for(target) {
            Some(other) if other != saving => {
                self.set_active_tab(other, cx);
                cx.notify();
                true
            }
            _ => false,
        }
    }
}

/// Show `already_open_message` as an error box.
pub(crate) async fn show_already_open_error() {
    rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(crate::consts::APP_NAME)
        .set_description(already_open_message())
        .set_buttons(rfd::MessageButtons::Ok)
        .show()
        .await;
}

/// Index of the first tab whose path names the same file as `target`. Paths
/// are compared as written first, then through `canonical` so `.\a.txt`,
/// a symlink, or (on Windows) a different letter case still match. A path
/// `canonical` can't resolve (deleted file, unsaved tab) only matches as
/// written.
fn open_tab_index(
    tab_paths: &[Option<&Path>],
    target: &Path,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
) -> Option<usize> {
    if let Some(i) = tab_paths.iter().position(|p| *p == Some(target)) {
        return Some(i);
    }
    let target = canonical(target)?;
    tab_paths
        .iter()
        .position(|p| p.and_then(&canonical).is_some_and(|c| c == target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_same_path_as_written() {
        let a = Path::new("/docs/a.txt");
        let b = Path::new("/docs/b.txt");
        let tabs = [None, Some(a), Some(b)];
        assert_eq!(open_tab_index(&tabs, b, |_| None), Some(2));
        assert_eq!(open_tab_index(&tabs, Path::new("/docs/c.txt"), |_| None), None);
        assert_eq!(open_tab_index(&[], a, |_| None), None);
    }

    #[test]
    fn matches_through_canonical_form() {
        // Stand-in resolver: lower-cases, as a case-insensitive file system
        // resolves names.
        let canon = |p: &Path| Some(PathBuf::from(p.to_string_lossy().to_lowercase()));
        let tabs = [Some(Path::new("/Docs/Notes.TXT"))];
        assert_eq!(open_tab_index(&tabs, Path::new("/docs/notes.txt"), canon), Some(0));
    }

    #[test]
    fn real_files_match_through_a_relative_spelling() {
        let dir = std::env::temp_dir().join(format!("ns-open-existing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let other = dir.join("b.txt");
        std::fs::write(&other, b"y").unwrap();
        let roundabout = dir.join(".").join("a.txt");
        let tabs = [Some(other.as_path()), Some(file.as_path())];
        let canon = |p: &Path| std::fs::canonicalize(p).ok();
        assert_eq!(open_tab_index(&tabs, &roundabout, canon), Some(1));
        // A missing file only matches as written.
        let gone = dir.join("gone.txt");
        assert_eq!(open_tab_index(&tabs, &gone, canon), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
