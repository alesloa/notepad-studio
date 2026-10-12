use super::*;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, process::{Child, Command, Stdio}, time::{Duration, Instant}};

const ROOT_ENV: &str = "NOTEPAD_STUDIO_SAVE_DURABILITY_ROOT";
const TOKEN_ENV: &str = "NOTEPAD_STUDIO_SAVE_DURABILITY_TOKEN";
const PREFIX: &[u8] = b"new partial prefix";

struct OwnedChild(Child);
impl OwnedChild {
    fn interrupt(&mut self) {
        self.0.kill().expect("terminate the owned fixture child");
        let status = self.0.wait().expect("wait for the owned fixture child");
        assert!(!status.success(), "interrupted child must not finish successfully");
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => {},
            Ok(None) => {
                if let Err(error) = self.0.kill() { eprintln!("fixture child cleanup failed: {error}"); }
                if let Err(error) = self.0.wait() { eprintln!("fixture child wait failed: {error}"); }
            },
            Err(error) => {
                eprintln!("fixture child status failed: {error}");
                if let Err(error) = self.0.kill() { eprintln!("fixture child cleanup failed: {error}"); }
                if let Err(error) = self.0.wait() { eprintln!("fixture child wait failed: {error}"); }
            },
        }
    }
}

fn rescue_path(root: &Path, n: u32) -> PathBuf {
    root.join(if n == 0 { "target.txt.notepad-studio-save".into() }
        else { format!("target.txt.notepad-studio-save.{n}") })
}

fn saturate_rescues(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    (0..100).map(|n| {
        let path = rescue_path(root, n); let bytes = format!("old unrelated rescue {n}\r\n").into_bytes();
        fs::write(&path, &bytes).expect("seed a stale rescue"); (path, bytes)
    }).collect()
}

fn interrupted_fixture(root: &Path) -> bool {
    let token = Uuid::new_v4().to_string(); fs::write(root.join("owner"), &token).expect("owner marker");
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command.args(["--exact", "io::save_durability_tests::writer_child", "--nocapture", "--test-threads=1"])
        .env(ROOT_ENV, root).env(TOKEN_ENV, &token).stdout(Stdio::null()).stderr(Stdio::null());
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW for the controlled background child.
    let mut child = OwnedChild(command.spawn().expect("spawn the owned fixture child"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if fs::read(root.join("ready")).ok().as_deref() == Some(token.as_bytes()) { break; }
        if let Some(status) = child.0.try_wait().expect("child status") {
            let result = fs::read_to_string(root.join("result")).expect("child refusal result");
            assert!(status.success() && result.starts_with("refused\n"), "unexpected child result {status}: {result}");
            child.0.wait().expect("wait for the completed owned child");
            return false;
        }
        assert!(Instant::now() < deadline, "fixture did not reach its bounded write barrier");
        std::thread::sleep(Duration::from_millis(5));
    }
    child.interrupt();
    true
}

// Only the explicitly spawned child receives these markers; a normal suite run is inert.
#[test]
fn writer_child() {
    let Some(root) = std::env::var_os(ROOT_ENV) else { return; };
    let token = std::env::var(TOKEN_ENV).expect("child token"); Uuid::parse_str(&token).expect("UUID token");
    let root = fs::canonicalize(root).expect("fixture root");
    assert_eq!(root.parent(), Some(fs::canonicalize(std::env::temp_dir()).expect("temp root").as_path()));
    assert!(root.file_name().is_some_and(|name| name.to_string_lossy().starts_with("notepad-studio-test-save-durability-")));
    assert_eq!(fs::read(root.join("owner")).expect("owner marker"), token.as_bytes());
    let result = write_file_safely_with(&root.join("target.txt"), b"new complete document", |file, _| {
        file.write_all(PREFIX)?; file.sync_all()?;
        let mut ready = fs::OpenOptions::new().write(true).create_new(true).open(root.join("ready"))?;
        ready.write_all(token.as_bytes())?; ready.sync_all()?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) { std::thread::sleep(Duration::from_millis(5)); }
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "fixture parent did not interrupt its child"))
    });
    let description = match result {
        Err(error @ SaveError::NoSafetyCopy { .. }) => format!("refused\n{error}"),
        result => format!("unexpected\n{result:?}"),
    };
    fs::write(root.join("result"), description).expect("child result marker");
}

#[test]
fn process_interruption_with_available_rescue_keeps_previous_disk_version() {
    let dir = crate::persist::TempDir::new("save-durability-positive"); let target = dir.join("target.txt");
    let original = "previous café中😀\r\n".repeat(80).into_bytes(); fs::write(&target, &original).expect("original");
    assert!(interrupted_fixture(dir.path()), "available rescue permits the controlled data write");
    let changed = fs::read(&target).expect("interrupted target");
    assert!(changed.starts_with(PREFIX)); assert_ne!(changed, original);
    let rescue = rescue_path(dir.path(), 0);
    assert_eq!(fs::read(&rescue).expect("retained rescue"), original);
    println!("SAVE_INTERRUPT_POSITIVE original_sha256={:x} target_sha256={:x} recoverable_path={}", Sha256::digest(&original), Sha256::digest(&changed), rescue.display());
}

#[test]
fn process_interruption_with_saturated_rescue_names_keeps_previous_disk_version() {
    let dir = crate::persist::TempDir::new("save-durability-saturated"); let target = dir.join("target.txt");
    let original = "previous café中😀\r\n".repeat(80).into_bytes(); fs::write(&target, &original).expect("original");
    let stale = saturate_rescues(dir.path()); let interrupted = interrupted_fixture(dir.path());
    let changed = fs::read(&target).expect("interrupted target");
    for (path, bytes) in &stale { assert_eq!(fs::read(path).expect("stale rescue"), *bytes); }
    let recoverable = fs::read_dir(dir.path()).expect("fixture directory").filter_map(Result::ok)
        .filter(|entry| fs::read(entry.path()).ok().as_deref() == Some(original.as_slice())).count();
    println!("SAVE_INTERRUPT_SATURATED original_sha256={:x} target_sha256={:x} prior_versions={} stale_rescues={} target_has_partial_prefix={} refused_before_write={}",
        Sha256::digest(&original), Sha256::digest(&changed), recoverable, stale.len(), changed.starts_with(PREFIX), !interrupted);
    assert!(recoverable > 0, "interruption left no recoverable prior disk version after all 100 rescue names were occupied");
    assert!(!interrupted); assert_eq!(changed, original);
}

#[test]
fn saturated_rescue_names_refuse_before_the_data_writer_runs() {
    let dir = crate::persist::TempDir::new("save-durability-error"); let target = dir.join("target.txt");
    let original = b"previous complete document\r\n"; fs::write(&target, original).expect("original");
    let stale = saturate_rescues(dir.path());
    let invoked = std::cell::Cell::new(false);
    let result = write_file_safely_with(&target, b"replacement", |_, _| { invoked.set(true); Ok(()) });
    assert!(matches!(result, Err(SaveError::NoSafetyCopy { ref source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists), "{result:?}");
    assert!(!invoked.get());
    assert_eq!(fs::read(&target).expect("restored target"), original);
    for (path, bytes) in stale { assert_eq!(fs::read(path).expect("stale rescue"), bytes); }
    println!("SAVE_REFUSAL_POSITIVE prior_version_unchanged=true stale_rescues=100 data_writer_invoked=false");
}

#[test]
fn rescue_copy_and_sync_failures_preserve_the_exact_error_and_refuse_before_writing() {
    for fault in [safety_copy::tests::Fault::Copy(112), safety_copy::tests::Fault::Sync(5)] {
        let dir = crate::persist::TempDir::new("save-durability-sync"); let target = dir.join("target.txt");
        fs::write(&target, b"previous complete document\r\n").expect("original");
        let _fault = safety_copy::tests::Override::new(&target, fault);
        let invoked = std::cell::Cell::new(false);
        let result = write_file_safely_with(&target, b"replacement", |_, _| { invoked.set(true); Ok(()) });
        let code = match fault { safety_copy::tests::Fault::Copy(code) | safety_copy::tests::Fault::Sync(code) => code };
        assert!(matches!(result, Err(SaveError::NoSafetyCopy { ref source, .. }) if source.raw_os_error() == Some(code)), "{result:?}");
        assert!(!invoked.get()); assert_eq!(fs::read(&target).expect("unchanged"), b"previous complete document\r\n");
        assert_eq!(fs::read_dir(dir.path()).expect("owned directory").count(), 1);
    }
}

#[test]
fn write_capable_nontruncating_rescue_sync_succeeds_on_windows() {
    let dir = crate::persist::TempDir::new("save-durability-handle"); let target = dir.join("target.txt");
    let original = b"previous complete document\r\n"; fs::write(&target, original).expect("original");
    let rescue = make_safety_copy(&target).expect("write-capable sync succeeds");
    assert_eq!(fs::read(&rescue).expect("synced rescue"), original);
    assert_eq!(fs::read(&target).expect("untouched target"), original);
}

#[test]
fn partial_write_error_restores_original_or_reports_its_retained_rescue_path() {
    for restore_fails in [false, true] {
        let dir = crate::persist::TempDir::new("save-durability-rollback"); let target = dir.join("target.txt");
        let original = b"previous complete document\r\n"; fs::write(&target, original).expect("original");
        let result = write_file_safely_with_restore(&target, b"replacement", |file, _| {
            file.write_all(PREFIX)?; Err(std::io::Error::from_raw_os_error(112))
        }, |file, copy| {
            if restore_fails { return Err(std::io::Error::from_raw_os_error(5)); }
            restore_copy::restore(file, copy)
        });
        if restore_fails {
            let rescue = rescue_path(dir.path(), 0);
            let error = result.expect_err("failed restoration");
            assert!(matches!(error, SaveError::NotRestored { ref source, ref restore, ref copy, .. }
                if source.raw_os_error() == Some(112) && restore.raw_os_error() == Some(5) && copy.as_ref() == Some(&rescue)));
            assert!(error.to_string().contains(&rescue.display().to_string()));
            assert_eq!(fs::read(rescue).expect("retained prior bytes"), original);
        } else {
            assert!(matches!(result, Err(SaveError::Restored { ref source, .. }) if source.raw_os_error() == Some(112)));
            assert_eq!(fs::read(&target).expect("restored"), original);
            assert!(!rescue_path(dir.path(), 0).exists());
        }
    }
}

#[test]
fn synced_rescue_writer_keeps_hardlink_identity_and_permissions() {
    let dir = crate::persist::TempDir::new("save-durability-links"); let target = dir.join("target.txt"); let alias = dir.join("alias.txt");
    fs::write(&target, b"previous complete document\r\n").expect("original"); fs::hard_link(&target, &alias).expect("alias");
    let identity = file_id::get_file_id(&target).expect("file identity"); let readonly = fs::metadata(&target).expect("metadata").permissions().readonly();
    write_file_safely(&target, "new café中😀\r\n".as_bytes()).expect("save");
    assert_eq!(file_id::get_file_id(&target).expect("saved identity"), identity);
    assert_eq!(file_id::get_file_id(&alias).expect("alias identity"), identity);
    assert_eq!(fs::read(&alias).expect("hardlink bytes"), "new café中😀\r\n".as_bytes());
    assert_eq!(fs::metadata(&target).expect("saved metadata").permissions().readonly(), readonly);
    assert_eq!(fs::read_dir(dir.path()).expect("rescue cleanup").count(), 2);
}
