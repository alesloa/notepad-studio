// Shared helpers for the TOML state files under `<config>/Notepad Studio/`
// (settings, session, windows, recent files, shortcuts, macros).
//
// Writes go to a temp file that is flushed to disk and then renamed over the
// target, so a crash or a full disk mid-write leaves the previous file in
// place. Reads are lenient: a file that no longer matches the schema keeps
// every field that still parses instead of falling back to defaults
// wholesale, and a file that isn't TOML at all is moved aside so the next
// save doesn't overwrite what the user might still want to recover.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// Replace `path` with `contents` without ever leaving it half-written.
///
/// The bytes go to a uniquely named sibling reserved with `create_new`, are
/// synced to disk, and then renamed over the target on the same volume.
/// Failed writes remove only this call's owned temporary, leaving the old
/// target and any pre-existing temporary files untouched.
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = path.with_file_name(format!(".notepad-studio-{}.tmp", uuid::Uuid::new_v4()));
    // A failed reservation owns nothing: return before any cleanup can touch it.
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = file.write_all(contents).and_then(|()| file.sync_all());
    // Close the write handle before replacement, including on Windows.
    drop(file);
    let result = result.and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    sync_parent_dir(path);
    Ok(())
}

/// Make the rename itself durable. Only Unix exposes directory fsync;
/// NTFS journals the rename on its own.
#[cfg(unix)]
fn sync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) {}

/// `path` with `suffix` appended to its file name (`session.toml` →
/// `session.toml.tmp`).
fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Move an unreadable state file to `<name>.corrupt` (replacing an older
/// one) so the next save doesn't destroy it. Returns where it went.
pub(crate) fn quarantine(path: &Path) -> Option<PathBuf> {
    let target = sibling_with_suffix(path, ".corrupt");
    match fs::rename(path, &target) {
        Ok(()) => {
            eprintln!(
                "[persist] {} could not be read; kept it as {}",
                path.display(),
                target.display()
            );
            Some(target)
        }
        Err(err) => {
            eprintln!("[persist] couldn't move aside {}: {err}", path.display());
            None
        }
    }
}

/// Copy a state file that only partly loaded to `<name>.corrupt` before the
/// next save rewrites it with just the parts that survived.
fn keep_copy(path: &Path, raw: &[u8]) {
    let target = sibling_with_suffix(path, ".corrupt");
    match atomic_write(&target, raw) {
        Ok(()) => eprintln!(
            "[persist] {} only partly loaded; the original is kept as {}",
            path.display(),
            target.display()
        ),
        Err(err) => eprintln!("[persist] couldn't copy {}: {err}", path.display()),
    }
}

/// Outcome of reading a TOML state file with `load_toml`.
pub(crate) enum Loaded<T> {
    /// No file yet (first launch) or it couldn't be opened.
    Missing,
    /// The file matched the schema.
    Clean(T),
    /// Some of the file didn't match; this is what survived. The original
    /// is kept as `<name>.corrupt`: copied when part of it loaded, moved
    /// when it wasn't TOML at all.
    Salvaged(T),
}

impl<T> Loaded<T> {
    pub(crate) fn into_option(self) -> Option<T> {
        match self {
            Loaded::Missing => None,
            Loaded::Clean(v) | Loaded::Salvaged(v) => Some(v),
        }
    }
}

/// Read a TOML state file. A file that parses but no longer fits `T` goes
/// through `salvage`; a file that isn't TOML (or isn't UTF-8 at all after a
/// lossy decode) is quarantined and `fallback` stands in for it.
pub(crate) fn load_toml<T: DeserializeOwned>(
    path: &Path,
    salvage: impl FnOnce(&toml::Table) -> T,
    fallback: impl FnOnce() -> T,
) -> Loaded<T> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                eprintln!("[persist] couldn't read {}: {err}", path.display());
            }
            return Loaded::Missing;
        }
    };
    let text = String::from_utf8_lossy(&raw);
    match toml::from_str::<T>(&text) {
        Ok(value) => Loaded::Clean(value),
        Err(strict) => match text.parse::<toml::Table>() {
            Ok(table) => {
                eprintln!(
                    "[persist] {} doesn't match the current format, keeping what fits: {strict}",
                    path.display()
                );
                keep_copy(path, &raw);
                Loaded::Salvaged(salvage(&table))
            }
            Err(_) => {
                quarantine(path);
                Loaded::Salvaged(fallback())
            }
        },
    }
}

/// Deserialize `T` from `table`, keeping every key whose value still fits
/// and dropping the ones that don't, so one bad field costs that field
/// instead of the whole file. A bad element inside an array drops only
/// that element. `required` names keys `T` can't do without: if any of
/// them is missing or bad, the result is `None`.
///
/// Keys absent from the result fall back to whatever `T`'s serde defaults
/// say, so this is meant for structs whose optional fields carry
/// `#[serde(default)]` (AGENTS.md §8).
pub(crate) fn salvage_table<T: DeserializeOwned>(
    table: &toml::Table,
    required: &[&str],
) -> Option<T> {
    let fits = |t: &toml::Table| t.clone().try_into::<T>().ok();
    if !required.iter().all(|key| table.contains_key(*key)) {
        return None;
    }
    // Most elements of a damaged array are fine; don't rebuild those key by key.
    if let Some(whole) = fits(table) {
        return Some(whole);
    }
    let mut kept = toml::Table::new();
    for key in required {
        let value = table.get(*key)?;
        kept.insert((*key).to_string(), value.clone());
    }
    let mut best = fits(&kept)?;
    for (key, value) in table {
        if required.contains(&key.as_str()) {
            continue;
        }
        kept.insert(key.clone(), value.clone());
        if let Some(t) = fits(&kept) {
            best = t;
            continue;
        }
        // The whole value doesn't fit. An array may still have good
        // elements: rebuild it one element at a time.
        let toml::Value::Array(items) = value else {
            kept.remove(key);
            continue;
        };
        let mut good = Vec::new();
        for item in items {
            good.push(item.clone());
            kept.insert(key.clone(), toml::Value::Array(good.clone()));
            match fits(&kept) {
                Some(t) => best = t,
                None => {
                    good.pop();
                }
            }
        }
        kept.insert(key.clone(), toml::Value::Array(good));
        if let Some(t) = fits(&kept) {
            best = t;
        } else {
            kept.remove(key);
        }
    }
    Some(best)
}

/// `salvage_table` for each element of an array of tables (the shape of
/// `[[tabs]]`, `[[macros]]` and friends). Elements that can't be salvaged
/// are dropped; the count of dropped elements comes back alongside.
pub(crate) fn salvage_array<T: DeserializeOwned>(
    value: Option<&toml::Value>,
    required: &[&str],
) -> (Vec<T>, usize) {
    let Some(toml::Value::Array(items)) = value else {
        return (Vec::new(), usize::from(value.is_some()));
    };
    let mut out = Vec::with_capacity(items.len());
    let mut dropped = 0usize;
    for item in items {
        let parsed = match item {
            toml::Value::Table(t) => salvage_table::<T>(t, required),
            other => other.clone().try_into::<T>().ok(),
        };
        match parsed {
            Some(v) => out.push(v),
            None => dropped += 1,
        }
    }
    (out, dropped)
}

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

#[cfg(test)]
#[path = "persist_corrupt_copy_tests.rs"]
mod corrupt_copy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq, Default)]
    struct Sample {
        #[serde(default)]
        width: u32,
        #[serde(default)]
        name: String,
        #[serde(default)]
        sizes: Vec<u8>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Keyed {
        id: u32,
        #[serde(default)]
        label: String,
    }

    fn table(src: &str) -> toml::Table {
        match src.parse::<toml::Table>() {
            Ok(t) => t,
            Err(e) => panic!("fixture isn't TOML: {e}"),
        }
    }

    #[test]
    fn salvage_keeps_good_fields_and_drops_bad_ones() {
        let t = table("width = -3\nname = \"kept\"\nsizes = [1, 300, 2, \"x\", 3]\nextra = 1\n");
        assert!(t.clone().try_into::<Sample>().is_err());
        let s: Option<Sample> = salvage_table(&t, &[]);
        assert_eq!(
            s,
            Some(Sample { width: 0, name: "kept".into(), sizes: vec![1, 2, 3] })
        );
    }

    #[test]
    fn salvage_needs_required_keys() {
        assert_eq!(salvage_table::<Keyed>(&table("label = \"x\"\n"), &["id"]), None);
        assert_eq!(salvage_table::<Keyed>(&table("id = \"x\"\n"), &["id"]), None);
        assert_eq!(
            salvage_table::<Keyed>(&table("id = 7\nlabel = 3\n"), &["id"]),
            Some(Keyed { id: 7, label: String::new() })
        );
    }

    #[test]
    fn salvage_array_drops_only_the_bad_elements() {
        let t = table("[[rows]]\nid = 1\n[[rows]]\nlabel = \"no id\"\n[[rows]]\nid = 3\nlabel = 9\n");
        let (rows, dropped) = salvage_array::<Keyed>(t.get("rows"), &["id"]);
        assert_eq!(dropped, 1);
        assert_eq!(
            rows,
            vec![
                Keyed { id: 1, label: String::new() },
                Keyed { id: 3, label: String::new() },
            ]
        );
        let (none, dropped) = salvage_array::<Keyed>(Some(&toml::Value::Integer(4)), &["id"]);
        assert!(none.is_empty());
        assert_eq!(dropped, 1);
    }

    #[test]
    fn load_toml_quarantines_a_file_that_is_not_toml() {
        let dir = TempDir::new("persist-load");
        let p = dir.join("state.toml");
        assert!(matches!(
            load_toml::<Sample>(&p, |_| Sample::default(), Sample::default),
            Loaded::Missing
        ));

        let _ = fs::write(&p, "width = 5\n");
        assert!(matches!(
            load_toml::<Sample>(&p, |_| Sample::default(), Sample::default),
            Loaded::Clean(Sample { width: 5, .. })
        ));

        let _ = fs::write(&p, "width = \"wide\"\nname = \"n\"\n");
        let got = load_toml::<Sample>(
            &p,
            |t| salvage_table(t, &[]).unwrap_or_default(),
            Sample::default,
        );
        assert!(matches!(got, Loaded::Salvaged(Sample { width: 0, ref name, .. }) if name == "n"));
        assert!(p.exists(), "a salvageable file stays where it is");
        assert_eq!(
            fs::read_to_string(dir.join("state.toml.corrupt")).ok().as_deref(),
            Some("width = \"wide\"\nname = \"n\"\n"),
            "and a copy keeps the field the next save will drop"
        );

        let _ = fs::write(&p, b"\xff\xfe[[[ not toml");
        let got = load_toml::<Sample>(&p, |_| Sample::default(), Sample::default);
        assert!(matches!(got, Loaded::Salvaged(_)));
        assert!(!p.exists());
        assert_eq!(
            fs::read(dir.join("state.toml.corrupt")).ok().as_deref(),
            Some(&b"\xff\xfe[[[ not toml"[..])
        );
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = TempDir::new("persist-atomic");
        let p = dir.join("a.toml");
        assert!(atomic_write(&p, b"one").is_ok());
        assert!(atomic_write(&p, b"two, longer").is_ok());
        assert_eq!(fs::read(&p).ok().as_deref(), Some(&b"two, longer"[..]));
        assert!(!dir.join("a.toml.tmp").exists());
    }

    #[test]
    fn atomic_write_keeps_an_existing_temp_sibling() {
        let dir = TempDir::new("persist-owned-temp");
        let path = dir.join("state.toml");
        let tmp = dir.join("state.toml.tmp");
        fs::write(&path, b"old state").expect("write state fixture");
        fs::write(&tmp, b"recovery bytes").expect("write existing sibling");

        let result = atomic_write(&path, b"new state");

        assert_eq!(fs::read(&tmp).expect("existing sibling survives"), b"recovery bytes");
        let expected: &[u8] = if result.is_ok() { b"new state" } else { b"old state" };
        assert_eq!(fs::read(&path).expect("state survives"), expected);
    }

    #[test]
    fn atomic_write_keeps_a_document_hard_linked_to_the_temp_sibling() {
        let dir = TempDir::new("persist-linked-temp");
        let path = dir.join("state.toml");
        let tmp = dir.join("state.toml.tmp");
        let document = dir.join("document.txt");
        let original = b"unrelated document\r\n";
        fs::write(&path, b"old state").expect("write state fixture");
        fs::write(&document, original).expect("write document fixture");
        fs::hard_link(&document, &tmp).expect("link existing sibling");

        let result = atomic_write(&path, b"new state");

        assert_eq!(fs::read(&document).expect("document survives"), original);
        assert_eq!(fs::read(&tmp).expect("existing link survives"), original);
        let expected: &[u8] = if result.is_ok() { b"new state" } else { b"old state" };
        assert_eq!(fs::read(&path).expect("state survives"), expected);
    }

    /// Cost of the fsync `atomic_write` adds over a plain write + rename,
    /// for the session autosave sizes. Timing only:
    /// `cargo test --release -p notepad-studio --bin notepad-studio atomic_write_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn atomic_write_cost() {
        let dir = TempDir::new("persist-cost");
        let p = dir.join("s.toml");
        let tmp = dir.join("s.toml.tmp");
        for size in [1024usize, 100 * 1024, 4 * 1024 * 1024] {
            let data = vec![b'x'; size];
            let rounds = 40u32;
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                assert!(fs::write(&tmp, &data).is_ok());
                assert!(fs::rename(&tmp, &p).is_ok());
            }
            let plain = t.elapsed() / rounds;
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                assert!(atomic_write(&p, &data).is_ok());
            }
            let synced = t.elapsed() / rounds;
            println!("{size:>8} bytes: write+rename {plain:?}, atomic_write {synced:?}");
        }
    }

    #[test]
    fn failed_atomic_write_keeps_the_old_file() {
        let dir = TempDir::new("persist-atomic-fail");
        let target = dir.join("a.toml");
        fs::create_dir(&target).expect("create target directory");
        let original = target.join("original.txt");
        fs::write(&original, b"original").expect("write directory contents");
        // A nonempty target directory refuses replacement after the temp write.
        assert!(atomic_write(&target, b"replacement").is_err());
        assert_eq!(fs::read(&original).expect("original survives"), b"original");
        assert_eq!(fs::read_dir(dir.path()).expect("list scratch directory").count(), 1);
    }

    #[test]
    fn atomic_write_keeps_a_directory_at_the_old_temp_name() {
        let dir = TempDir::new("persist-temp-directory");
        let target = dir.join("a.toml");
        let legacy = dir.join("a.toml.tmp");
        fs::create_dir(&legacy).expect("create existing temporary directory");
        let original = legacy.join("original.txt");
        fs::write(&original, b"recovery bytes").expect("write directory contents");
        atomic_write(&target, b"new state").expect("save beside existing directory");
        assert_eq!(fs::read(&target).expect("read target"), b"new state");
        assert_eq!(fs::read(&original).expect("directory contents survive"), b"recovery bytes");
        assert_eq!(fs::read_dir(dir.path()).expect("list scratch directory").count(), 2);
    }

    #[test]
    fn parallel_atomic_writes_publish_one_complete_payload() {
        let dir = TempDir::new("persist-parallel");
        let target = dir.join("state.toml");
        fs::write(&target, b"old state").expect("write old target");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let writers: Vec<_> = (0u8..4).map(|index| {
            let path = target.clone();
            let ready = barrier.clone();
            std::thread::spawn(move || {
                let payload = vec![b'a' + index; 16 * 1024];
                ready.wait();
                atomic_write(&path, &payload)
            })
        }).collect();
        for writer in writers {
            writer.join().expect("writer joins").expect("atomic save succeeds");
        }
        let final_bytes = fs::read(&target).expect("read published target");
        assert!((0u8..4).any(|index| final_bytes == vec![b'a' + index; 16 * 1024]));
        assert_eq!(fs::read_dir(dir.path()).expect("list scratch directory").count(), 1);
    }
}
