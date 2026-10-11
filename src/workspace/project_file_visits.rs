use std::io;
use std::path::Path;

const KEY_LIMIT: usize = 64 * 1024;
const BYTE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FileKey { volume: u64, id: u128 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VisitStop { Capacity, Allocation, FileResources }

impl VisitStop {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Capacity => "file identity limit reached",
            Self::Allocation => "file identity memory could not be reserved",
            Self::FileResources => "file identity resources are unavailable",
        }
    }
}

pub(super) enum VisitError { File(io::Error), Stop(VisitStop) }

pub(super) struct FileVisits {
    keys: Vec<FileKey>,
    limit: usize,
    byte_limit: usize,
    #[cfg(test)]
    reserve_failure: bool,
    #[cfg(test)]
    reserve_extra: usize,
    #[cfg(test)]
    identity_failure: Option<io::Error>,
}

impl Default for FileVisits {
    fn default() -> Self { Self::with_limits(KEY_LIMIT, BYTE_LIMIT) }
}

impl FileVisits {
    fn with_limits(limit: usize, byte_limit: usize) -> Self {
        Self { keys: Vec::new(), limit, byte_limit,
            #[cfg(test)] reserve_failure: false,
            #[cfg(test)] reserve_extra: 0,
            #[cfg(test)] identity_failure: None }
    }

    pub(super) fn admit(&mut self, path: &Path) -> Result<bool, VisitError> {
        #[cfg(test)]
        if let Some(error) = self.identity_failure.take() {
            return Err(identity_error(error));
        }
        let key = identify(path).map_err(identity_error)?;
        self.admit_key(key).map_err(VisitError::Stop)
    }

    fn admit_key(&mut self, key: FileKey) -> Result<bool, VisitStop> {
        let index = match self.keys.binary_search(&key) { Ok(_) => return Ok(false), Err(index) => index };
        if self.keys.len() >= self.limit { return Err(VisitStop::Capacity); }
        if self.keys.capacity() == 0 {
            // One lazy allocation, then no growth while any keys are retained.
            let old_bytes = self.keys.capacity().checked_mul(std::mem::size_of::<FileKey>())
                .ok_or(VisitStop::Allocation)?;
            let requested = self.limit.checked_mul(std::mem::size_of::<FileKey>())
                .and_then(|bytes| old_bytes.checked_add(bytes)).ok_or(VisitStop::Allocation)?;
            if requested > self.byte_limit { return Err(VisitStop::Allocation); }
            #[cfg(test)]
            if self.reserve_failure { return Err(VisitStop::Allocation); }
            let requested_keys = self.limit;
            #[cfg(test)]
            let requested_keys = requested_keys.checked_add(self.reserve_extra).ok_or(VisitStop::Allocation)?;
            let mut keys = Vec::new();
            keys.try_reserve_exact(requested_keys).map_err(|_| VisitStop::Allocation)?;
            let allocated = keys.capacity().checked_mul(std::mem::size_of::<FileKey>())
                .and_then(|bytes| old_bytes.checked_add(bytes)).ok_or(VisitStop::Allocation)?;
            if allocated > self.byte_limit { return Err(VisitStop::Allocation); }
            self.keys = keys;
        }
        if index > self.keys.len() || self.keys.len() >= self.keys.capacity() { return Err(VisitStop::Capacity); }
        self.keys.insert(index, key);
        Ok(true)
    }

    #[cfg(test)]
    pub(super) fn limited(limit: usize) -> Self {
        Self::with_limits(limit, limit.saturating_mul(std::mem::size_of::<FileKey>()))
    }

    #[cfg(test)]
    pub(super) fn fail_reservation(&mut self) { self.reserve_failure = true; }

    #[cfg(test)]
    pub(super) fn fail_next_identity(&mut self, kind: io::ErrorKind) { self.identity_failure = Some(io::Error::new(kind, "controlled identity refusal")); }

    #[cfg(all(test, windows))]
    pub(super) fn fail_next_identity_error(&mut self, error: io::Error) { self.identity_failure = Some(error); }
}

#[cfg(windows)]
fn identify(path: &Path) -> io::Result<FileKey> {
    match file_id::get_high_res_file_id(path)? {
        file_id::FileId::HighRes { volume_serial_number, file_id } =>
            Ok(FileKey { volume: volume_serial_number, id: file_id }),
        _ => Err(io::Error::new(io::ErrorKind::Unsupported, "full Windows file identity unavailable")),
    }
}

#[cfg(unix)]
fn identify(path: &Path) -> io::Result<FileKey> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::File::open(path)?.metadata()?;
    Ok(FileKey { volume: metadata.dev(), id: u128::from(metadata.ino()) })
}

fn identity_error(error: io::Error) -> VisitError {
    #[cfg(windows)]
    if error.raw_os_error() == Some(4) { return VisitError::Stop(VisitStop::FileResources); }
    #[cfg(unix)]
    if matches!(error.raw_os_error(), Some(23 | 24)) { return VisitError::Stop(VisitStop::FileResources); }
    VisitError::File(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u128) -> FileKey { FileKey { volume: 1, id } }

    #[test]
    fn project_overlap_bank_exact_cap_keeps_duplicate_lookup_and_refuses_new_key() {
        let mut visits = FileVisits::limited(2);
        assert!(visits.admit_key(key(2)).unwrap()); assert!(visits.admit_key(key(1)).unwrap());
        assert!(!visits.admit_key(key(2)).unwrap());
        assert_eq!(visits.admit_key(key(3)), Err(VisitStop::Capacity));
        assert_eq!(visits.keys.len(), 2); assert!(visits.keys.capacity() * std::mem::size_of::<FileKey>() <= visits.byte_limit);
    }

    #[test]
    fn project_overlap_bank_checks_reservation_overcapacity_and_arithmetic() {
        let mut visits = FileVisits::limited(2); visits.reserve_extra = 1;
        assert_eq!(visits.admit_key(key(1)), Err(VisitStop::Allocation)); assert!(visits.keys.is_empty());
        let mut visits = FileVisits::with_limits(usize::MAX, usize::MAX);
        assert_eq!(visits.admit_key(key(1)), Err(VisitStop::Allocation));
        let mut visits = FileVisits::limited(2); visits.fail_reservation();
        assert_eq!(visits.admit_key(key(1)), Err(VisitStop::Allocation)); assert!(visits.keys.is_empty());
    }

    #[test]
    fn project_overlap_full_ids_and_volumes_are_not_truncated() {
        let mut visits = FileVisits::limited(3);
        assert!(visits.admit_key(key(1)).unwrap()); assert!(visits.admit_key(key(1 + (1u128 << 100))).unwrap());
        assert!(visits.admit_key(FileKey { volume: 2, id: 1 }).unwrap());
    }

    #[test]
    #[cfg(windows)]
    fn project_overlap_windows_resource_error_stops_but_path_errors_continue() {
        assert!(matches!(identity_error(io::Error::from_raw_os_error(4)), VisitError::Stop(VisitStop::FileResources)));
        for code in [1, 50, 87] {
            assert!(matches!(identity_error(io::Error::from_raw_os_error(code)), VisitError::File(_)));
        }
    }

    #[test]
    fn project_overlap_unsupported_identity_is_an_encountered_file_failure() {
        assert!(matches!(identity_error(io::Error::new(io::ErrorKind::PermissionDenied, "controlled refusal")), VisitError::File(_)));
        assert!(matches!(identity_error(io::Error::new(io::ErrorKind::Unsupported, "controlled unsupported provider")), VisitError::File(_)));
    }
}

#[test]
#[ignore = "Release worst-case key bank benchmark; no filesystem/throughput claim"]
fn project_overlap_key_bank_release_benchmark() {
    let mut visits = FileVisits::default(); let start = std::time::Instant::now();
    for id in (0..KEY_LIMIT).rev() {
        assert!(visits.admit_key(FileKey { volume: 1, id: id as u128 }).unwrap());
    }
    assert_eq!(visits.keys.len(), KEY_LIMIT);
    eprintln!("BENCH keybank reverse_keys={} key_size={} capacity_bytes={} elapsed_ms={}",
        KEY_LIMIT, std::mem::size_of::<FileKey>(), visits.keys.capacity() * std::mem::size_of::<FileKey>(), start.elapsed().as_millis());
}
