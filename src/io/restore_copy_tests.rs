use super::*;
use super::super::{write_file_safely_with, write_file_safely_with_restore, SaveError};
use std::{cell::{Cell, RefCell}, fs, io::Cursor, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase { Open, Metadata, Read, Seek, Write, SetLen, Sync }
#[derive(Clone, Copy)]
enum Failure { Code(i32), Kind(io::ErrorKind) }
struct State { copy: PathBuf, failure: Option<(Phase, Failure)>, scratch: usize, phases: Vec<Phase> }
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

pub(super) fn check(copy: &Path, phase: Phase) -> io::Result<()> {
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut().filter(|state| state.copy == copy) else { return Ok(()); };
        state.phases.push(phase);
        match state.failure.filter(|(failed, _)| *failed == phase).map(|(_, error)| error) {
            Some(Failure::Code(code)) => Err(io::Error::from_raw_os_error(code)),
            Some(Failure::Kind(kind)) => Err(kind.into()),
            None => Ok(()),
        }
    })
}
pub(super) fn observe(copy: &Path, scratch: usize) {
    STATE.with(|slot| { if let Some(state) = slot.borrow_mut().as_mut().filter(|state| state.copy == copy) {
        state.scratch = state.scratch.max(scratch);
    } });
}
struct Override(Option<State>);
impl Override {
    fn new(copy: &Path, failure: Option<(Phase, Failure)>) -> Self {
        Self(STATE.with(|slot| slot.replace(Some(State { copy: copy.to_owned(), failure, scratch: 0, phases: Vec::new() }))))
    }
    fn scratch(&self) -> usize { STATE.with(|slot| slot.borrow().as_ref().map(|state| state.scratch).unwrap_or(0)) }
    fn phases(&self) -> Vec<Phase> { STATE.with(|slot| slot.borrow().as_ref().map(|state| state.phases.clone()).unwrap_or_default()) }
}
impl Drop for Override { fn drop(&mut self) { STATE.with(|slot| slot.replace(self.0.take())); } }

// `file-id` is a Windows-only dependency.
#[cfg(target_os = "windows")]
#[test]
fn actual_writer_restoration_uses_fixed_scratch_budget() {
    let dir = crate::persist::TempDir::new("restore-fixed-budget"); let target = dir.join("prior.txt");
    let rescue = dir.join("prior.txt.notepad-studio-save"); let alias = dir.join("alias.txt");
    let original = vec![b'x'; 18 * 1024 * 1024]; fs::write(&target, &original).expect("prior version");
    fs::hard_link(&target, &alias).expect("hard link"); let identity = file_id::get_file_id(&target).expect("identity");
    let scope = Override::new(&rescue, None);
    let result = write_file_safely_with(&target, b"new version", |file, _| {
        file.write_all(b"partial")?; Err(io::Error::from_raw_os_error(112))
    });
    assert!(matches!(result, Err(SaveError::Restored { ref source, .. }) if source.raw_os_error() == Some(112)));
    assert_eq!(fs::read(&target).expect("restored"), original); assert_eq!(fs::read(&alias).expect("alias bytes"), original);
    assert_eq!(file_id::get_file_id(&target).expect("target identity"), identity);
    assert_eq!(file_id::get_file_id(&alias).expect("alias identity"), identity);
    assert_eq!(scope.scratch(), 65_536); assert_eq!(fs::read_dir(dir.path()).expect("owned directory").count(), 2);
    assert_eq!(scope.phases(), vec![Phase::Open, Phase::Metadata, Phase::Read, Phase::Seek, Phase::Write, Phase::SetLen, Phase::Sync]);
    eprintln!("G2 prior_bytes={} fixed_restore_scratch={} identity_and_bytes_preserved=true", original.len(), scope.scratch());
}

#[test]
fn actual_writer_rescue_read_oom_retains_prior_bytes_before_restore_mutation() {
    let dir = crate::persist::TempDir::new("restore-read-error"); let target = dir.join("prior.txt");
    let rescue = dir.join("prior.txt.notepad-studio-save"); let unrelated = dir.join("unrelated.txt");
    let other_copy = dir.join("unrelated.copy"); let original = "prior café中😀\r\n".as_bytes();
    fs::write(&target, original).expect("prior"); fs::write(&unrelated, b"unrelated").expect("unrelated");
    fs::write(&other_copy, b"unrelated").expect("other rescue");
    let scope = Override::new(&rescue, Some((Phase::Read, Failure::Kind(io::ErrorKind::OutOfMemory))));
    let mut other = fs::OpenOptions::new().write(true).open(&unrelated).expect("other target");
    restore(&mut other, &other_copy).expect("other path unaffected");
    assert_eq!(fs::read(&unrelated).expect("unrelated bytes"), b"unrelated");
    let invoked = Cell::new(false);
    let error = write_file_safely_with_restore(&target, b"new version", |file, _| {
        file.write_all(b"partial")?; Err(io::Error::from_raw_os_error(112))
    }, |file, copy| {
        invoked.set(true); let position = file.stream_position()?; let before = fs::read(&target)?;
        let result = restore(file, copy);
        assert_eq!(file.stream_position().expect("unchanged position"), position);
        assert_eq!(fs::read(&target).expect("before restoration mutation"), before); result
    }).expect_err("produced rescue-read error");
    assert!(invoked.get(), "reading is now owned by the restore callback");
    assert!(matches!(&error, SaveError::NotRestored { source, restore, copy, .. }
        if source.raw_os_error() == Some(112) && restore.kind() == io::ErrorKind::OutOfMemory
            && restore.raw_os_error().is_none() && copy.as_ref() == Some(&rescue)));
    assert_eq!(scope.phases(), vec![Phase::Open, Phase::Metadata, Phase::Read]);
    assert!(error.to_string().contains(&rescue.display().to_string()));
    assert_eq!(fs::read(&rescue).expect("retained prior bytes"), original);
    eprintln!("G2 produced_read_error=OutOfMemory callback_invoked=true target_seek_and_bytes_unchanged=true rescue={}", rescue.display());
}

#[test]
fn actual_writer_restores_empty_shorter_longer_and_multichunk_originals() {
    for length in [0, 3, 65_536, 65_567, 196_615] {
        for grow_target in [false, true] {
            let dir = crate::persist::TempDir::new("restore-size"); let target = dir.join("prior.txt");
            let original: Vec<u8> = (0..length).map(|offset| (offset % 251) as u8).collect();
            fs::write(&target, &original).expect("prior"); let readonly = fs::metadata(&target).expect("metadata").permissions().readonly();
            let changed = vec![b'z'; if grow_target { length + 33 } else { 1 }];
            let result = write_file_safely_with(&target, &changed, |file, bytes| {
                file.write_all(bytes)?; Err(io::Error::from_raw_os_error(112))
            });
            assert!(matches!(result, Err(SaveError::Restored { ref source, .. }) if source.raw_os_error() == Some(112)));
            assert_eq!(fs::read(&target).expect("restored"), original);
            assert_eq!(fs::metadata(&target).expect("metadata").permissions().readonly(), readonly);
            assert_eq!(fs::read_dir(dir.path()).expect("cleanup").count(), 1);
        }
    }
}

#[test]
fn actual_writer_retains_rescue_and_exact_error_for_each_restore_phase() {
    for phase in [Phase::Open, Phase::Metadata, Phase::Read, Phase::Seek, Phase::Write, Phase::SetLen, Phase::Sync] {
        let dir = crate::persist::TempDir::new("restore-phase-error"); let target = dir.join("prior.txt");
        let rescue = dir.join("prior.txt.notepad-studio-save"); let original = b"previous complete version\r\n";
        fs::write(&target, original).expect("prior"); let scope = Override::new(&rescue, Some((phase, Failure::Code(5))));
        let error = write_file_safely_with(&target, b"new version", |file, _| {
            file.write_all(b"partial")?; Err(io::Error::from_raw_os_error(112))
        }).expect_err("injected restore failure");
        assert!(matches!(&error, SaveError::NotRestored { source, restore, copy, .. }
            if source.raw_os_error() == Some(112) && restore.raw_os_error() == Some(5) && copy.as_ref() == Some(&rescue)));
        assert_eq!(scope.phases().last(), Some(&phase));
        assert_eq!(fs::read(&rescue).expect("retained"), original);
        assert!(error.to_string().contains(&rescue.display().to_string()));
        if matches!(phase, Phase::Open | Phase::Metadata | Phase::Read | Phase::Seek | Phase::Write) {
            assert!(fs::read(&target).expect("partial target").starts_with(b"partial"));
        } else { assert_eq!(fs::read(&target).expect("copied before final error"), original); }
    }
}

#[test]
fn real_missing_rescue_open_error_precedes_target_seek() {
    let dir = crate::persist::TempDir::new("restore-missing-copy"); let target = dir.join("target.txt");
    fs::write(&target, b"partial target").expect("target");
    let mut file = fs::OpenOptions::new().write(true).open(&target).expect("handle"); file.seek(SeekFrom::Start(4)).expect("position");
    let error = restore(&mut file, &dir.join("missing.copy")).expect_err("missing rescue");
    assert_eq!(error.kind(), io::ErrorKind::NotFound); assert_eq!(file.stream_position().expect("position"), 4);
    assert_eq!(fs::read(&target).expect("unchanged"), b"partial target");
}

struct Reader<'a> { input: Cursor<&'a [u8]>, short: usize, interrupted: usize, fail_after: Option<u64>, max_request: usize }
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { input: Cursor::new(bytes), short: usize::MAX, interrupted: 0, fail_after: None, max_request: 0 } }
}
impl Read for Reader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.max_request = self.max_request.max(buffer.len());
        if self.interrupted > 0 { self.interrupted -= 1; return Err(io::ErrorKind::Interrupted.into()); }
        if self.fail_after.is_some_and(|at| self.input.position() >= at) { return Err(io::Error::from_raw_os_error(5)); }
        let count = buffer.len().min(self.short); self.input.read(buffer.get_mut(..count).expect("bounded fixture"))
    }
}
struct Writer { bytes: Vec<u8>, short: usize, interrupted: usize, fail_after: Option<usize> }
impl Writer { fn new() -> Self { Self { bytes: Vec::new(), short: usize::MAX, interrupted: 0, fail_after: None } } }
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.interrupted > 0 { self.interrupted -= 1; return Err(io::ErrorKind::Interrupted.into()); }
        if self.fail_after.is_some_and(|at| self.bytes.len() >= at) { return Err(io::Error::from_raw_os_error(112)); }
        let count = bytes.len().min(self.short); self.bytes.extend_from_slice(bytes.get(..count).expect("bounded fixture")); Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

#[test]
fn pump_retries_interrupted_and_short_reads_and_writes() {
    let bytes = "café中😀\r\n".repeat(15_000).into_bytes();
    let mut reader = Reader::new(&bytes); reader.short = 7; reader.interrupted = 2;
    let mut writer = Writer::new(); writer.short = 3; writer.interrupted = 2;
    pump(&mut reader, &mut writer, bytes.len() as u64, &mut [0u8; SCRATCH_BYTES]).expect("complete short I/O");
    assert_eq!(writer.bytes, bytes); assert!(reader.max_request <= SCRATCH_BYTES);
}

#[test]
fn pump_rejects_early_eof_or_growth_and_preserves_bytes_read_so_far() {
    for expected in [2, 4] {
        let mut reader = Reader::new(b"abc"); let mut writer = Writer::new();
        let error = pump(&mut reader, &mut writer, expected, &mut [0u8; SCRATCH_BYTES]).expect_err("length changed");
        assert_eq!(error.kind(), if expected == 2 { io::ErrorKind::InvalidData } else { io::ErrorKind::UnexpectedEof });
        assert_eq!(writer.bytes, if expected == 2 { b"ab".as_slice() } else { b"abc".as_slice() });
    }
}

#[test]
fn pump_retains_exact_read_or_partial_write_errors() {
    for read_fails in [false, true] {
        let mut reader = Reader::new(b"abcdefghijklmnop"); reader.short = 4;
        let mut writer = Writer::new(); writer.short = 2;
        if read_fails { reader.fail_after = Some(8); } else { writer.fail_after = Some(6); }
        let error = pump(&mut reader, &mut writer, 16, &mut [0u8; SCRATCH_BYTES]).expect_err("I/O error");
        assert_eq!(error.raw_os_error(), Some(if read_fails { 5 } else { 112 }));
        assert_eq!(writer.bytes, if read_fails { b"abcdefgh".as_slice() } else { b"abcdef".as_slice() });
    }
}

#[test]
fn pump_rejects_zero_progress_or_oversized_writer_counts() {
    struct Broken(usize);
    impl Write for Broken { fn write(&mut self, _: &[u8]) -> io::Result<usize> { Ok(self.0) } fn flush(&mut self) -> io::Result<()> { Ok(()) } }
    for count in [0, usize::MAX] {
        let error = pump(&mut Cursor::new(b"abc"), &mut Broken(count), 3, &mut [0u8; SCRATCH_BYTES]).expect_err("invalid count");
        assert_eq!(error.kind(), if count == 0 { io::ErrorKind::WriteZero } else { io::ErrorKind::InvalidData });
    }
}

#[test]
fn pump_rejects_malicious_reader_counts_in_data_and_eof_probe() {
    struct Broken;
    impl Read for Broken { fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Ok(usize::MAX) } }
    for expected in [0, 1, u64::MAX] {
        let mut writer = Writer::new();
        let error = pump(&mut Broken, &mut writer, expected, &mut [0u8; SCRATCH_BYTES]).expect_err("malicious count");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData); assert!(writer.bytes.is_empty());
    }
}

#[test]
fn pump_caps_each_request_and_refuses_empty_scratch_without_reading() {
    let bytes = vec![b'x'; 2 * SCRATCH_BYTES + 17]; let mut reader = Reader::new(&bytes); let mut writer = Writer::new();
    pump(&mut reader, &mut writer, bytes.len() as u64, &mut vec![0u8; 2 * SCRATCH_BYTES]).expect("bounded requests");
    assert_eq!(reader.max_request, SCRATCH_BYTES); assert_eq!(writer.bytes, bytes);
    let mut reader = Reader::new(b"a"); let mut writer = Writer::new();
    let error = pump(&mut reader, &mut writer, u64::MAX, &mut []).expect_err("no scratch");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput); assert_eq!(reader.max_request, 0); assert!(writer.bytes.is_empty());
}

#[test]
fn pump_checks_large_expected_length_without_file_sized_allocation() {
    let mut reader = Reader::new(b"short"); let mut writer = Writer::new();
    let error = pump(&mut reader, &mut writer, u64::MAX, &mut [0u8; SCRATCH_BYTES]).expect_err("EOF before huge length");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof); assert_eq!(writer.bytes, b"short"); assert_eq!(reader.max_request, SCRATCH_BYTES);
}
