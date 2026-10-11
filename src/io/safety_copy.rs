use std::{fs, io, path::{Path, PathBuf}};

// A synced sibling rescue must exist before the in-place target write starts.
pub(super) fn make_safety_copy(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    for n in 0u32..100 {
        let mut candidate = name.to_os_string(); candidate.push(".notepad-studio-save");
        if n > 0 { candidate.push(format!(".{n}")); }
        let target = dir.join(candidate);
        match fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
        let prepared = prepare(path, &target);
        if let Err(error) = prepared { discard(&target); return Err(error); }
        return Ok(target);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many leftover .notepad-studio-save copies"))
}

fn prepare(source: &Path, copy: &Path) -> io::Result<()> {
    #[cfg(all(test, target_os = "windows"))]
    if let Some(tests::Fault::Copy(code)) = tests::fault(source) { return Err(io::Error::from_raw_os_error(code)); }
    fs::copy(source, copy)?;
    // Windows FlushFileBuffers requires write access; reopening must not truncate.
    let file = fs::OpenOptions::new().write(true).open(copy)?;
    #[cfg(all(test, target_os = "windows"))]
    if let Some(tests::Fault::Sync(code)) = tests::fault(source) { return Err(io::Error::from_raw_os_error(code)); }
    file.sync_all()
}

pub(super) fn discard(copy: &Path) {
    if let Err(error) = fs::remove_file(copy) {
        if error.kind() != io::ErrorKind::NotFound { eprintln!("[save rescue] couldn't remove {}: {error}", copy.display()); }
    }
}

#[cfg(all(test, target_os = "windows"))]
pub(super) mod tests {
    use super::*;
    #[derive(Clone, Copy)]
    pub(crate) enum Fault { Copy(i32), Sync(i32) }
    thread_local! { static FAULT: std::cell::RefCell<Option<(PathBuf, Fault)>> = const { std::cell::RefCell::new(None) }; }
    pub(super) fn fault(path: &Path) -> Option<Fault> {
        FAULT.with(|slot| slot.borrow().as_ref().filter(|(target, _)| target == path).map(|(_, fault)| *fault))
    }
    pub(crate) struct Override(Option<(PathBuf, Fault)>);
    impl Override {
        pub(crate) fn new(path: &Path, fault: Fault) -> Self {
            Self(FAULT.with(|slot| slot.replace(Some((path.to_owned(), fault)))))
        }
    }
    impl Drop for Override { fn drop(&mut self) { FAULT.with(|slot| slot.replace(self.0.take())); } }
}
