// Shared helpers for files Notepad Studio writes and reads back.

/// Test-only scratch directory under the system temp dir, removed on drop.
#[cfg(test)]
pub(crate) struct TempDir(std::path::PathBuf);

#[cfg(test)]
impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "notepad-studio-test-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        if let Err(e) = std::fs::create_dir_all(&path) {
            panic!("create {}: {e}", path.display());
        }
        Self(path)
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.0
    }

    pub(crate) fn join(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
