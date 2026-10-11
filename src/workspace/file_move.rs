// PowerEditor/src/ScintillaComponent/Buffer.cpp:1201-1226: cross-volume Rename.
use std::{fs::{self, File, OpenOptions}, io::{self, Read, Write}, path::{Path, PathBuf},
    sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};
use gpui::{AppContext, Context, Window};
use super::Workspace;

pub(super) struct Progress { cancel: Arc<AtomicBool>, copied: AtomicU64, total: AtomicU64 }
impl Default for Progress {
    fn default() -> Self { Self { cancel: Arc::new(AtomicBool::new(false)), copied: AtomicU64::new(0), total: AtomicU64::new(0) } }
}

fn remove_source(path: &Path) -> io::Result<()> { fs::remove_file(path) }

fn check_cancel(progress: &Progress) -> io::Result<()> {
    if progress.cancel.load(Ordering::Relaxed) { Err(io::Error::new(io::ErrorKind::Interrupted, "file move cancelled")) } else { Ok(()) }
}

#[derive(PartialEq, Eq)]
struct Stamp { length: u64, modified: Option<std::time::SystemTime> }
impl Stamp {
    fn at(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self { length: metadata.len(), modified: metadata.modified().ok() })
    }
    fn optional(path: &Path) -> io::Result<Option<Self>> {
        match Self::at(path) { Ok(stamp) => Ok(Some(stamp)), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None), Err(error) => Err(error) }
    }
}

struct StagedMove { source: PathBuf, destination: PathBuf, temporary: PathBuf, stamp: Stamp, destination_stamp: Option<Stamp> }
impl Drop for StagedMove {
    // Only the Windows block clears the flag on this owned scratch file.
    #[allow(clippy::permissions_set_readonly_false)]
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        if let Ok(metadata) = fs::metadata(&self.temporary) {
            let mut permissions = metadata.permissions();
            if permissions.readonly() { permissions.set_readonly(false); let _ = fs::set_permissions(&self.temporary, permissions); }
        }
        let _ = fs::remove_file(&self.temporary);
    }
}

impl StagedMove {
    fn prepare(source: PathBuf, destination: PathBuf, progress: &Progress) -> io::Result<Self> {
        check_cancel(progress)?;
        let stamp = Stamp::at(&source)?;
        progress.total.store(stamp.length, Ordering::Relaxed);
        let destination_stamp = Stamp::optional(&destination)?;
        let parent = destination.parent().ok_or_else(|| io::Error::other("destination has no containing folder"))?;
        let temporary = parent.join(format!(".notepad-studio-move-{}", uuid::Uuid::new_v4()));
        let mut from = File::open(&source)?;
        let mut to = OpenOptions::new().create_new(true).write(true).open(&temporary)?;
        let stage = Self { source, destination, temporary, stamp, destination_stamp };
        let mut chunk = vec![0_u8; 1024 * 1024];
        loop {
            check_cancel(progress)?;
            let count = from.read(&mut chunk)?;
            if count == 0 { break; }
            to.write_all(chunk.get(..count).ok_or_else(|| io::Error::other("invalid file read length"))?)?;
            progress.copied.fetch_add(count as u64, Ordering::Relaxed);
        }
        check_cancel(progress)?;
        to.sync_all()?;
        fs::set_permissions(&stage.temporary, from.metadata()?.permissions())?;
        if Stamp::at(&stage.source)? != stage.stamp { return Err(io::Error::other("source file changed while preparing its move")); }
        Ok(stage)
    }

    fn complete(&self, remove_source: impl FnOnce(&Path) -> io::Result<()>) -> Result<Option<String>, String> {
        self.complete_with(remove_source, |previous, destination| {
            if let Some(previous) = previous { fs::rename(previous, destination) }
            else { match fs::remove_file(destination) { Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()), result => result } }
        })
    }

    fn complete_with(&self, remove_source: impl FnOnce(&Path) -> io::Result<()>, rollback: impl FnOnce(Option<&Path>, &Path) -> io::Result<()>) -> Result<Option<String>, String> {
        if Stamp::at(&self.source).map_err(|error| error.to_string())? != self.stamp {
            return Err("source file changed while preparing its move".into());
        }
        if Stamp::optional(&self.destination).map_err(|error| error.to_string())? != self.destination_stamp {
            return Err("destination changed while preparing the file move; no file was moved".into());
        }
        if self.destination.is_dir() { return Err("destination is a folder".into()); }
        let previous = self.destination.parent().map(|parent| parent.join(format!(".notepad-studio-move-previous-{}", uuid::Uuid::new_v4())))
            .ok_or_else(|| "destination has no containing folder".to_string())?;
        let had_previous = self.destination.try_exists().map_err(|error| error.to_string())?;
        if had_previous { fs::rename(&self.destination, &previous).map_err(|error| error.to_string())?; }
        let operation = fs::rename(&self.temporary, &self.destination).and_then(|()| remove_source(&self.source));
        if let Err(error) = operation {
            let restore = rollback(had_previous.then_some(previous.as_path()), &self.destination);
            return match restore {
                Ok(()) => Err(format!("{error}; the original file and destination were left unchanged")),
                Err(restore) => Err(format!("{error}; destination rollback failed ({restore}); its previous version is at {}", previous.display())),
            };
        }
        if had_previous {
            if let Err(error) = fs::remove_file(&previous) {
                return Ok(Some(format!("File moved, but the previous destination copy at {} could not be removed: {error}", previous.display())));
            }
        }
        Ok(None)
    }
}

impl Workspace {
    pub(super) fn stop_file_move(&mut self) {
        if let Some(progress) = self.file_move.take() { progress.cancel.store(true, Ordering::Relaxed); }
    }

    pub(super) fn cancel_file_move(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file_move.is_none() { return false; }
        self.stop_file_move(); self.set_status("Rename cancelled; original file and destination unchanged"); cx.notify(); true
    }

    pub(super) fn file_move_chip(&self) -> Option<crate::ui::chrome::LoadingChip> {
        let progress = self.file_move.as_ref()?;
        let copied = progress.copied.load(Ordering::Relaxed); let total = progress.total.load(Ordering::Relaxed);
        Some(crate::ui::chrome::LoadingChip {
            progress_text: format!("Moving file {} / {}", crate::io_stream::format_bytes(copied), crate::io_stream::format_bytes(total)),
            cancel: progress.cancel.clone(),
        })
    }

    pub(super) fn rename_across_filesystems(&mut self, index: usize, source: PathBuf, destination: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return; };
        let id = tab.session_id; let instance = tab.instance.clone(); let saved = tab.buffer.savepoint.clone();
        let window_id = self.session_window_id; let native_window = window.window_handle();
        self.stop_file_move();
        let progress = Arc::new(Progress::default()); self.file_move = Some(progress.clone());
        let worker_progress = progress.clone();
        let cleanup_executor = cx.background_executor().clone();
        #[cfg(test)]
        let gate = test_support::take_gate();
        let work = cx.background_spawn(async move {
            let result = StagedMove::prepare(source, destination, &worker_progress);
            #[cfg(test)]
            if let Some(gate) = gate { let _ = gate.await; }
            crate::io::BackgroundDrop::new(result, cleanup_executor)
        });
        self.set_status("Preparing file move…"); cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |w, window, cx| {
                if w.file_move.as_ref().is_none_or(|current| !Arc::ptr_eq(current, &progress)) { return; }
                w.file_move = None;
                if progress.cancel.load(Ordering::Relaxed) { w.set_status("Rename cancelled; original file and destination unchanged"); cx.notify(); return; }
                let Some(result) = result.as_ref() else { return; };
                let stage = match result { Ok(stage) => stage, Err(error) => { w.set_status(format!("Rename failed: {error}")); cx.notify(); return; } };
                if w.window_closed || w.session_window_id != window_id || window.window_handle() != native_window { return; }
                let Some(index) = w.tabs.iter().position(|tab| tab.session_id == id && Arc::ptr_eq(&instance, &tab.instance)
                    && tab.buffer.file_path.as_ref() == Some(&stage.source) && tab.buffer.savepoint == saved) else {
                    w.set_status("Rename cancelled: the original document or saved file changed"); cx.notify(); return;
                };
                if w.file_destination_open_elsewhere(&stage.destination, index, window.window_handle(), cx) {
                    w.set_status(crate::workspace::already_open_message()); cx.notify(); return;
                }
                match stage.complete(remove_source) {
                    Ok(warning) => {
                        w.finish_file_rename(index, stage.source.clone(), stage.destination.clone(), cx);
                        if let Some(warning) = warning { w.set_status(warning); }
                    }
                    Err(error) => w.set_status(format!("Rename failed: {error}")),
                }
                cx.notify();
            });
        }).detach();
        let progress = self.file_move.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                let alive = this.update(cx, |w, cx| {
                    let alive = match (&w.file_move, &progress) { (Some(current), Some(progress)) => Arc::ptr_eq(current, progress), _ => false };
                    if alive { cx.notify(); }
                    alive
                }).unwrap_or(false);
                if !alive { break; }
            }
        }).detach();
    }
}

#[cfg(test)]
pub(super) mod test_support {
    thread_local! { static GATE: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<()>>> = const { std::cell::RefCell::new(None) }; }
    pub(super) fn take_gate() -> Option<futures::channel::oneshot::Receiver<()>> { GATE.with(|slot| slot.borrow_mut().take()) }
    pub(crate) struct Override(Option<futures::channel::oneshot::Receiver<()>>);
    impl Override {
        pub(crate) fn new(gate: futures::channel::oneshot::Receiver<()>) -> Self { Self(GATE.with(|slot| slot.replace(Some(gate)))) }
    }
    impl Drop for Override { fn drop(&mut self) { GATE.with(|slot| slot.replace(self.0.take())); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_device_staging_and_source_removal_error_restore_destination() {
        for existed in [false, true] {
            let dir = crate::persist::TempDir::new("move-rollback"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
            fs::write(&source, "source café中😀\r\n").expect("source"); if existed { fs::write(&target, b"original destination").expect("target"); }
            let stage = StagedMove::prepare(source.clone(), target.clone(), &Progress::default()).expect("stage");
            assert!(stage.complete(|_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "synthetic removal failure"))).is_err());
            assert_eq!(fs::read_to_string(&source).expect("source"), "source café中😀\r\n");
            if existed { assert_eq!(fs::read(&target).expect("target"), b"original destination"); } else { assert!(!target.exists()); }
        }
    }
    #[test]
    fn cross_device_stage_cancels_changed_source_and_preserves_selected_destination() {
        let dir = crate::persist::TempDir::new("move-stale-disk"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        fs::write(&source, b"original").expect("source"); fs::write(&target, b"selected destination").expect("target");
        let stage = StagedMove::prepare(source.clone(), target.clone(), &Progress::default()).expect("stage");
        fs::write(&source, b"changed source content").expect("source edit");
        assert!(stage.complete(|path| fs::remove_file(path)).is_err());
        assert_eq!(fs::read(&target).expect("target"), b"selected destination");
    }
    #[test]
    fn cross_device_stage_cancels_changed_destination_before_any_original_file_effect() {
        let dir = crate::persist::TempDir::new("move-stale-target"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        fs::write(&source, b"original").expect("source");
        let stage = StagedMove::prepare(source.clone(), target.clone(), &Progress::default()).expect("stage");
        fs::write(&target, b"new external destination").expect("target edit");
        assert!(stage.complete(|path| fs::remove_file(path)).is_err());
        assert_eq!(fs::read(&source).expect("source"), b"original");
        assert_eq!(fs::read(&target).expect("target"), b"new external destination");
    }
    #[test]
    fn cross_device_rollback_failure_names_the_retained_previous_destination() {
        let dir = crate::persist::TempDir::new("move-failed-rollback"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        fs::write(&source, b"original source").expect("source"); fs::write(&target, b"original target").expect("target");
        let stage = StagedMove::prepare(source.clone(), target.clone(), &Progress::default()).expect("stage");
        let error = stage.complete_with(|_| Err(io::Error::other("synthetic source removal failure")), |_, _| Err(io::Error::other("synthetic rollback failure"))).expect_err("injected failure");
        assert!(error.contains("destination rollback failed"));
        let previous = fs::read_dir(dir.path()).expect("directory").filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with(".notepad-studio-move-previous-")).expect("previous destination retained");
        assert!(error.contains(&previous.path().display().to_string()));
        assert_eq!(fs::read(previous.path()).expect("previous bytes"), b"original target");
        assert_eq!(fs::read(&source).expect("source bytes"), b"original source");
    }

    #[test]
    fn cancelled_cross_device_stage_creates_no_destination_or_temporary() {
        let dir = crate::persist::TempDir::new("move-cancelled"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        fs::write(&source, b"original source").expect("source"); let progress = Progress::default(); progress.cancel.store(true, Ordering::Relaxed);
        assert!(StagedMove::prepare(source.clone(), target.clone(), &progress).is_err());
        assert!(!target.exists()); assert_eq!(fs::read_dir(dir.path()).expect("directory").count(), 1);
    }

    #[test]
    #[ignore = "optimized >10 MiB streamed cross-device preparation timing"]
    fn cross_device_rename_large_stage_timing() {
        let dir = crate::persist::TempDir::new("move-timing"); let source = dir.join("source.txt"); let target = dir.join("target.txt");
        let sample = "café中😀\r\n".repeat(1024 * 1024); fs::write(&source, sample.as_bytes()).expect("source");
        let progress = Progress::default(); let started = std::time::Instant::now();
        let stage = StagedMove::prepare(source.clone(), target.clone(), &progress).expect("stage"); let prepared = started.elapsed();
        stage.complete(remove_source).expect("move");
        println!("FILE_MOVE_TIMING bytes={} copied={} preparation_ms={:.3} total_ms={:.3}", sample.len(), progress.copied.load(Ordering::Relaxed), prepared.as_secs_f64() * 1e3, started.elapsed().as_secs_f64() * 1e3);
        assert_eq!(fs::read(&target).expect("target"), sample.as_bytes());
    }
}
