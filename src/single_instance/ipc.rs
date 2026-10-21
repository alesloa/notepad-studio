// The lock file has a stable inode and stays open for the primary's lifetime.
// Each activation is a separate, atomically published request, never a shared
// overwritten sentinel. All helpers accept a root for isolated regression tests.
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_REQUESTS_PER_TURN: usize = 16;
pub(super) struct Primary {
    _file: File,
}

pub(super) fn claim(root: &Path) -> io::Result<Option<Primary>> {
    fs::create_dir_all(root)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".instance-lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Primary { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

pub(super) fn enqueue(root: &Path, paths: &[PathBuf]) -> io::Result<PathBuf> {
    let directory = root.join(".activation-requests");
    fs::create_dir_all(&directory)?;
    // OsString's serde representation preserves Unix bytes / Windows UTF-16,
    // including whitespace and newlines that the legacy line protocol lost.
    let paths: Vec<OsString> = paths
        .iter()
        .map(|path| path.as_os_str().to_owned())
        .collect();
    let bytes = serde_json::to_vec(&paths).map_err(io::Error::other)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "activation request is too large",
        ));
    }
    let id = uuid::Uuid::new_v4();
    let pending = directory.join(format!("{id}.pending"));
    let ready = directory.join(format!("{id}.json"));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&pending, &ready)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result.map(|_| ready)
}

pub(super) fn drain(root: &Path) -> (Vec<PathBuf>, bool) {
    let Ok(entries) = fs::read_dir(root.join(".activation-requests")) else {
        return (Vec::new(), false);
    };
    let mut paths = Vec::new();
    let mut raise = false;
    for entry in entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .take(MAX_REQUESTS_PER_TURN)
    {
        let path = entry.path();
        let result = (|| -> io::Result<Vec<OsString>> {
            if !entry.file_type()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "activation request is not a file",
                ));
            }
            let mut bytes = Vec::new();
            File::open(&path)?
                .take(MAX_REQUEST_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_REQUEST_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "activation request is too large",
                ));
            }
            serde_json::from_slice(&bytes).map_err(io::Error::other)
        })();
        match result {
            Ok(request) => {
                // Remove only a fully read request. A partial publisher still has
                // its .pending name and cannot be seen by this reader.
                if fs::remove_file(&path).is_ok() {
                    paths.extend(request.into_iter().map(PathBuf::from));
                    raise = true;
                }
            }
            Err(error) => {
                eprintln!(
                    "[single-instance] invalid activation request {}: {error}",
                    path.display()
                );
                let _ = fs::rename(&path, path.with_extension("invalid"));
            }
        }
    }
    (paths, raise)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_launchers_have_exactly_one_owner_and_keep_lock_inode() {
        let root = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(17));
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut threads = Vec::new();
        for _ in 0..16 {
            let path = root.path().to_path_buf();
            let barrier = barrier.clone();
            let sender = sender.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                let owner = claim(&path).unwrap();
                sender.send(owner.is_some()).unwrap();
                barrier.wait();
                drop(owner);
            }));
        }
        barrier.wait();
        let winners = (0..16).filter(|_| receiver.recv().unwrap()).count();
        assert_eq!(winners, 1);
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(root.path().join(".instance-lock").is_file());
        let owner = claim(root.path()).unwrap().expect("released owner");
        assert!(claim(root.path()).unwrap().is_none());
        drop(owner);
        assert!(claim(root.path()).unwrap().is_some());
    }

    #[test]
    fn concurrent_requests_are_not_overwritten_and_paths_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let mut threads = Vec::new();
        for index in 0..40 {
            let root = root.path().to_path_buf();
            threads.push(std::thread::spawn(move || {
                let path = PathBuf::from(format!("/neutral/ name {index} café中\ntrailing "));
                enqueue(&root, std::slice::from_ref(&path)).unwrap();
                path
            }));
        }
        let mut expected: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let mut actual = Vec::new();
        loop {
            let (paths, ready) = drain(root.path());
            if !ready {
                break;
            }
            actual.extend(paths);
        }
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
        enqueue(root.path(), &[]).unwrap();
        assert_eq!(drain(root.path()), (Vec::new(), true));
    }

    #[test]
    fn incomplete_and_corrupt_requests_do_not_poison_valid_requests() {
        let root = tempfile::tempdir().unwrap();
        enqueue(root.path(), &["/neutral/file".into()]).unwrap();
        let directory = root.path().join(".activation-requests");
        fs::write(directory.join("incomplete.pending"), b"[").unwrap();
        fs::write(directory.join("corrupt.json"), b"[").unwrap();
        assert_eq!(
            drain(root.path()),
            (vec![PathBuf::from("/neutral/file")], true)
        );
        assert!(directory.join("incomplete.pending").exists());
        assert!(directory.join("corrupt.invalid").exists());
        assert_eq!(drain(root.path()), (Vec::new(), false));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_file_names_survive_forwarding() {
        use std::os::unix::ffi::OsStringExt;
        let root = tempfile::tempdir().unwrap();
        let path = PathBuf::from(OsString::from_vec(b"/neutral/\xff.txt".to_vec()));
        enqueue(root.path(), std::slice::from_ref(&path)).unwrap();
        assert_eq!(drain(root.path()).0, vec![path]);
    }
}
