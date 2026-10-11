// Thread-local recent files and optional neutral persistence backends for tests.
use std::{cell::RefCell, marker::PhantomData, path::PathBuf, rc::Rc};
use super::RecentFiles;
use std::sync::Arc;
use super::writer::Writer;

thread_local! {
    static WRITER: RefCell<Option<Arc<Writer>>> = const { RefCell::new(None) };
    static SNAPSHOT: RefCell<Option<RecentFiles>> = const { RefCell::new(None) };
}

pub(crate) struct SnapshotGuard {
    previous: Option<RecentFiles>,
    same_thread: PhantomData<Rc<()>>,
}

impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        SNAPSHOT.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

pub(crate) fn override_for_test(paths: Vec<PathBuf>) -> SnapshotGuard {
    SnapshotGuard {
        previous: SNAPSHOT.with(|slot| slot.replace(Some(RecentFiles { paths }))),
        same_thread: PhantomData,
    }
}

pub(super) fn snapshot() -> Option<Vec<PathBuf>> {
    SNAPSHOT.with(|slot| slot.borrow().as_ref().map(|recent| recent.paths.clone()))
}

pub(super) fn mutate(update: impl FnOnce(&mut RecentFiles) -> bool) -> bool {
    let start = SNAPSHOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let recent = slot.as_mut()?;
        let changed = update(recent);
        let writer = WRITER.with(|writer| writer.borrow().clone());
        Some(if changed { writer.and_then(|writer| super::writer::queue(&writer, recent)) } else { None })
    });
    let Some(start) = start else { return false; };
    if let Some(writer) = start { writer.start(); }
    true
}

pub(super) fn observe_save(path: &std::path::Path) {
    if super::writer::controlled_write() { return; }
    WRITER.with(|slot| {
        if let Some(writer) = slot.borrow().as_ref().filter(|writer| writer.destination() == path) {
            writer.probe.inline.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    });
}

#[test]
fn snapshot_guard_restores_nested_values_and_does_not_cross_threads() {
    assert!(snapshot().is_none());
    {
        let _outer = override_for_test(vec!["neutral-café.txt".into()]);
        assert_eq!(super::snapshot(), vec![PathBuf::from("neutral-café.txt")]);
        assert!(std::thread::spawn(|| snapshot().is_none()).join().unwrap());
        {
            let _inner = override_for_test(Vec::new());
            assert!(super::snapshot().is_empty());
        }
        assert_eq!(super::snapshot(), vec![PathBuf::from("neutral-café.txt")]);
    }
    // Inspect the fixture rather than loading the real MRU after restoration.
    assert!(snapshot().is_none());
}

#[test]
fn isolated_fixture_routes_open_remove_and_clear_through_real_mru_rules() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings {
        recent_files_max: 2, ..Default::default()
    });
    let first = PathBuf::from("neutral-café.txt");
    let second = PathBuf::from("neutral-中😀.rs");
    let third = PathBuf::from("neutral-third.md");
    let _fixture = override_for_test(vec![first.clone(), second]);
    super::record_opened(third.clone());
    assert_eq!(super::snapshot(), vec![third.clone(), first.clone()]);
    super::record_opened(first.clone());
    assert_eq!(super::snapshot(), vec![first.clone(), third.clone()]);
    super::remove_path(&PathBuf::from("absent"));
    assert_eq!(super::snapshot(), vec![first.clone(), third.clone()]);
    super::remove_path(&first);
    assert_eq!(super::snapshot(), vec![third]);
    super::clear_global();
    assert!(super::snapshot().is_empty());
}

#[test]
fn fixture_mutations_restore_nested_state_and_stay_on_their_thread() {
    let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
    let outer = PathBuf::from("neutral-outer.txt");
    let _fixture = override_for_test(vec![outer.clone()]);
    {
        let _inner = override_for_test(Vec::new());
        super::record_opened("neutral-inner.txt".into());
        assert_eq!(super::snapshot(), vec![PathBuf::from("neutral-inner.txt")]);
    }
    assert_eq!(super::snapshot(), vec![outer.clone()]);
    std::thread::spawn(|| {
        let _settings = crate::settings::override_for_test(crate::settings::Settings::default());
        let _fixture = override_for_test(Vec::new());
        super::record_opened("neutral-thread.txt".into());
        assert_eq!(super::snapshot(), vec![PathBuf::from("neutral-thread.txt")]);
        super::clear_global();
        assert!(super::snapshot().is_empty());
    }).join().unwrap();
    assert_eq!(super::snapshot(), vec![outer]);
}

pub(super) struct WriterGuard(Option<Arc<Writer>>, PhantomData<Rc<()>>);
impl Drop for WriterGuard {
    fn drop(&mut self) { WRITER.with(|slot| *slot.borrow_mut() = self.0.take()); }
}
pub(super) fn override_writer(writer: Arc<Writer>) -> WriterGuard {
    WriterGuard(WRITER.with(|slot| slot.replace(Some(writer))), PhantomData)
}
