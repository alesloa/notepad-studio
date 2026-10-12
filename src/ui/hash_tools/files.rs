// MISC/md5/md5Dlgs.cpp:134-261 — chosen order, full paths, digest + basename.
use std::{fmt::Write, fs::File, path::PathBuf, sync::atomic::{AtomicBool, AtomicU64, Ordering}};
use crate::hash_tools::{self, Algorithm};
pub(super) const MAX_FILES: usize = 4096;
pub(super) const MAX_METADATA: usize = 16 * 1024 * 1024;
pub(super) struct ResultText { pub(super) paths: String, pub(super) hashes: String, pub(super) status: String }
fn append(output: &mut String, text: &str, used: &mut usize) -> Result<(), String> {
    *used = used.checked_add(text.len().saturating_mul(4)).filter(|&n| n <= MAX_METADATA)
        .ok_or_else(|| "Hash file list/results exceed the 16 MiB metadata limit; choose fewer files".to_string())?;
    output.try_reserve(text.len()).map_err(|_| "Not enough memory for hash file results".to_string())?;
    output.push_str(text); Ok(())
}
pub(super) fn prepare(paths: Vec<PathBuf>, algorithm: Algorithm, cancel: &AtomicBool, progress: &AtomicU64) -> Result<ResultText, String> {
    if paths.len() > MAX_FILES { return Err("Choose at most 4,096 files to hash".into()); }
    let mut used = paths.len().checked_mul(std::mem::size_of::<PathBuf>()).ok_or("Too many file paths")?;
    for path in &paths {
        used = used.checked_add(path.as_os_str().as_encoded_bytes().len().saturating_mul(4))
            .filter(|&n| n <= MAX_METADATA).ok_or("Hash paths exceed the 16 MiB metadata limit; choose fewer files")?;
    }
    let mut output = ResultText { paths: String::new(), hashes: String::new(), status: String::new() };
    for path in paths {
        if cancel.load(Ordering::Relaxed) { return Err("Hash cancelled; previous results retained".into()); }
        let result: Result<String, String> = (|| {
            let mut file = File::open(&path).map_err(|error| error.to_string())?;
            let before = file.metadata().map_err(|error| error.to_string())?;
            if !before.is_file() { return Err("The selected path is not a regular file".into()); }
            let hash = hash_tools::digest_reader(algorithm, &mut file, cancel, progress).map_err(|error| error.to_string())?;
            let after = file.metadata().map_err(|error| error.to_string())?;
            let current = std::fs::metadata(&path).map_err(|error| error.to_string())?;
            if before.len() != after.len() || before.modified().ok() != after.modified().ok()
                || before.len() != current.len() || before.modified().ok() != current.modified().ok() {
                return Err("File changed while hashing; choose it again".into());
            }
            Ok(hash)
        })();
        match result {
            Ok(hash) => {
                let full = path.to_string_lossy(); let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                // Preflight the whole row before adding either panel's row.
                let row_cost = full.len().saturating_add(name.len()).saturating_add(hash.len()).saturating_add(6).saturating_mul(4);
                if used.checked_add(row_cost).is_none_or(|n| n > MAX_METADATA) {
                    return Err("Hash file results exceed the 16 MiB metadata limit; choose fewer files".into());
                }
                append(&mut output.paths, &full, &mut used)?; append(&mut output.paths, "\r\n", &mut used)?;
                append(&mut output.hashes, &hash, &mut used)?; append(&mut output.hashes, "  ", &mut used)?;
                append(&mut output.hashes, &name, &mut used)?; append(&mut output.hashes, "\r\n", &mut used)?;
            }
            Err(error) => {
                if cancel.load(Ordering::Relaxed) { return Err("Hash cancelled; previous results retained".into()); }
                let path: String = path.to_string_lossy().chars().take(512).collect();
                let error: String = error.chars().take(512).collect();
                let _ = write!(&mut output.status, "Could not hash {path}: {error}");
                if output.hashes.is_empty() { return Err(output.status); }
                return Ok(output);
            }
        }
    }
    output.status = "Finished hashing files".into(); Ok(output)
}
