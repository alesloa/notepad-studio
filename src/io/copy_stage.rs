// Save a Copy As prepares bytes off the UI thread, then uses the normal rollback writer.
use std::{fs, io::{self, Write}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};
use crate::{buffer::Buffer, settings::BackupOnSaveMode};

#[derive(Default)]
pub(crate) struct Progress {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) written: AtomicU64,
    pub(crate) total: AtomicU64,
}

impl Progress {
    fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) { Err(io::Error::new(io::ErrorKind::Interrupted, "copy cancelled")) }
        else { Ok(()) }
    }
}

#[derive(PartialEq)]
struct Stamp { length: u64, modified: Option<std::time::SystemTime>, readonly: bool }
fn stamp(path: &Path) -> Result<Option<Stamp>, String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(Stamp { length: metadata.len(), modified: metadata.modified().ok(), readonly: metadata.permissions().readonly() })),
        Ok(_) => Err("destination is not a file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) struct Backup {
    mode: BackupOnSaveMode, custom: Option<PathBuf>,
    #[cfg(test)]
    fault: Option<tests::Fault>,
}
impl Backup {
    pub(super) fn capture() -> Self {
        let settings = crate::settings::snapshot();
        Self { mode: settings.backup_on_save_mode, custom: (settings.backup_use_custom_dir
            && !settings.backup_custom_dir.trim().is_empty()).then(|| PathBuf::from(settings.backup_custom_dir.trim())),
            #[cfg(test)]
            fault: tests::take_fault(),
        }
    }
    fn run(&self, path: &Path) {
        if matches!(self.mode, BackupOnSaveMode::None) { return; }
        if let Err(error) = crate::session::write_save_backup(path, self.mode, self.custom.as_deref()) {
            eprintln!("[backup-on-save] {error}");
        }
    }
}

pub(super) struct Stage { directory: PathBuf, file: PathBuf, destination: PathBuf, selected: Option<Stamp> }
#[derive(Debug)]
pub(super) enum CommitError { Cancelled, Preparation(String), Write(super::SaveError) }
impl CommitError {
    pub(super) fn critical(&self) -> bool { matches!(self, Self::Write(super::SaveError::NotRestored { .. })) }
}
impl std::fmt::Display for CommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Cancelled => formatter.write_str("copy cancelled"), Self::Preparation(error) => formatter.write_str(error), Self::Write(error) => std::fmt::Display::fmt(error, formatter) }
    }
}
impl Stage {
    pub(super) fn prepare(buffer: Buffer, destination: PathBuf, progress: &Progress) -> Result<Self, String> {
        progress.check().map_err(|error| error.to_string())?;
        let selected = stamp(&destination)?;
        let name = destination.file_name().ok_or("destination has no file name")?;
        let parent = destination.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let directory = parent.join(format!(".notepad-studio-copy-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).map_err(|error| error.to_string())?;
        let file = directory.join(name);
        let stage = Self { directory, file, destination, selected };
        // The reserved directory contains no prior file, so the shared writer cannot back up user data here.
        super::copy_writer::write_copy(&buffer, &stage.file)?;
        progress.check().map_err(|error| error.to_string())?;
        progress.total.store(fs::metadata(&stage.file).map_err(|error| error.to_string())?.len(), Ordering::Relaxed);
        Ok(stage)
    }

    pub(super) fn commit(&self, backup: &Backup, progress: &Progress) -> Result<(), CommitError> {
        #[cfg(test)]
        if let Some(fault) = &backup.fault {
            progress.check().map_err(|_| CommitError::Cancelled)?;
            if stamp(&self.destination).map_err(CommitError::Preparation)? != self.selected { return Err(CommitError::Preparation("destination changed while preparing the copy".into())); }
            let bytes = fs::read(&self.file).map_err(|error| CommitError::Preparation(error.to_string()))?;
            return super::write_file_safely_with_restore(&self.destination, &bytes, |file, bytes| {
                file.write_all(bytes.get(..bytes.len().min(3)).unwrap_or_default())?;
                progress.cancel.store(true, Ordering::Relaxed);
                Err(io::Error::new(io::ErrorKind::Interrupted, "synthetic cancellation after write"))
            }, |file, bytes| {
                if fault.fail_restore { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "synthetic restore failure")); }
                use io::{Seek, SeekFrom};
                file.seek(SeekFrom::Start(0))?; file.write_all(bytes)?; file.set_len(bytes.len() as u64)?; file.sync_all()
            }).map_err(CommitError::Write);
        }
        self.commit_with(backup, progress, |file, bytes| {
            for chunk in bytes.chunks(crate::io_stream::STREAM_CHUNK_SIZE) {
                progress.check()?;
                file.write_all(chunk)?;
                progress.written.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
            progress.check()
        })
    }

    fn commit_with(&self, backup: &Backup, progress: &Progress,
        write: impl Fn(&mut fs::File, &[u8]) -> io::Result<()>) -> Result<(), CommitError> {
        progress.check().map_err(|_| CommitError::Cancelled)?;
        if stamp(&self.destination).map_err(CommitError::Preparation)? != self.selected { return Err(CommitError::Preparation("destination changed while preparing the copy".into())); }
        let bytes = fs::read(&self.file).map_err(|error| CommitError::Preparation(error.to_string()))?;
        progress.check().map_err(|_| CommitError::Cancelled)?;
        if self.selected.is_some() { backup.run(&self.destination); }
        progress.check().map_err(|_| CommitError::Cancelled)?;
        super::write_file_safely_with(&self.destination, &bytes, write).map_err(CommitError::Write)
    }
}
impl Drop for Stage {
    fn drop(&mut self) { let _ = fs::remove_file(&self.file); let _ = fs::remove_dir(&self.directory); }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(super) struct Fault { pub(super) fail_restore: bool }
    thread_local! { static FAULT: std::cell::RefCell<Option<Fault>> = const { std::cell::RefCell::new(None) }; }
    pub(super) fn take_fault() -> Option<Fault> { FAULT.with(|slot| slot.borrow_mut().take()) }
    pub(crate) struct Override(Option<Fault>);
    impl Override { pub(crate) fn new(fail_restore: bool) -> Self { Self(FAULT.with(|slot| slot.replace(Some(Fault { fail_restore })))) } }
    impl Drop for Override { fn drop(&mut self) { FAULT.with(|slot| slot.replace(self.0.take())); } }
    #[test]
    fn interrupted_or_failed_copy_write_restores_existing_and_removes_new_destination() {
        let _settings = crate::settings::override_for_test(Default::default());
        for existed in [false, true] {
            for cancelled in [false, true] {
                let directory = crate::persist::TempDir::new("copy-rollback"); let destination = directory.join("copy.txt");
                if existed { fs::write(&destination, b"original destination").expect("fixture"); }
                let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str("next café中😀\n");
                let progress = Progress::default();
                let stage = Stage::prepare(buffer, destination.clone(), &progress).expect("stage");
                let result = stage.commit_with(&Backup::capture(), &progress, |file, bytes| {
                    file.write_all(bytes.get(..3).expect("fixture prefix"))?;
                    if cancelled { progress.cancel.store(true, Ordering::Relaxed); progress.check() }
                    else { Err(io::Error::other("synthetic write failure")) }
                });
                assert!(result.is_err());
                if existed { assert_eq!(fs::read(&destination).expect("restored"), b"original destination"); }
                else { assert!(!destination.exists()); }
                drop(stage); assert_eq!(fs::read_dir(directory.path()).expect("directory").count(), usize::from(existed));
            }
        }
    }
}
