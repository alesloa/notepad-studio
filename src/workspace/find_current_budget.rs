//! Owned-capacity accounting for Rich FindAll. Regex/framework allocations and
//! allocator bookkeeping are outside this ledger; it is not a process RSS cap.
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use super::find_current_projection::ProjectionError;

pub(super) const OWNED_BYTES: usize = 192 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Ledger(Arc<Counters>);
struct Counters { used: AtomicUsize, peak: AtomicUsize, limit: usize }
impl Default for Ledger {
    fn default() -> Self { Self::with_limit(OWNED_BYTES) }
}
impl Ledger {
    fn with_limit(limit: usize) -> Self {
        Self(Arc::new(Counters { used: AtomicUsize::new(0), peak: AtomicUsize::new(0), limit }))
    }
    pub(super) fn pool(&self, limit: usize) -> Pool {
        Pool { ledger: self.clone(), local: Arc::new(Counters {
            used: AtomicUsize::new(0), peak: AtomicUsize::new(0), limit,
        }) }
    }
    pub(super) fn used(&self) -> usize { self.0.used.load(Ordering::Acquire) }
    pub(super) fn peak(&self) -> usize { self.0.peak.load(Ordering::Acquire) }
}
fn admit(counters: &Counters, bytes: usize) -> Result<(), ProjectionError> {
    let used = counters.used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
        used.checked_add(bytes).filter(|&next| next <= counters.limit)
    }).map_err(|_| ProjectionError::Limit)?;
    counters.peak.fetch_max(used.saturating_add(bytes), Ordering::AcqRel);
    Ok(())
}
#[derive(Clone)]
pub(super) struct Pool { ledger: Ledger, local: Arc<Counters> }
impl Pool {
    pub(super) fn charge(&self, bytes: usize) -> Result<Credit, ProjectionError> {
        admit(&self.ledger.0, bytes)?;
        if let Err(error) = admit(&self.local, bytes) {
            self.ledger.0.used.fetch_sub(bytes, Ordering::AcqRel);
            return Err(error);
        }
        Ok(Credit { pool: self.clone(), bytes, local_bytes: bytes })
    }
    fn charge_growth(&self, bytes: usize, old: usize) -> Result<Credit, ProjectionError> {
        let local_bytes = bytes.checked_sub(old).ok_or(ProjectionError::Limit)?;
        admit(&self.ledger.0, bytes)?;
        if let Err(error) = admit(&self.local, local_bytes) {
            self.ledger.0.used.fetch_sub(bytes, Ordering::AcqRel);
            return Err(error);
        }
        Ok(Credit { pool: self.clone(), bytes, local_bytes })
    }
}
pub(super) struct Credit { pool: Pool, bytes: usize, local_bytes: usize }
impl Drop for Credit {
    fn drop(&mut self) {
        self.pool.local.used.fetch_sub(self.local_bytes, Ordering::AcqRel);
        self.pool.ledger.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub(super) trait Capacity: Default {
    fn capacity_bytes(&self) -> Option<usize>;
    fn reserve_bytes(&mut self, bytes: usize) -> Result<(), ProjectionError>;
}
impl Capacity for String {
    fn capacity_bytes(&self) -> Option<usize> { Some(self.capacity()) }
    fn reserve_bytes(&mut self, bytes: usize) -> Result<(), ProjectionError> {
        self.try_reserve_exact(bytes.saturating_sub(self.len())).map_err(|_| ProjectionError::Allocation)
    }
}
impl<T> Capacity for Vec<T> {
    fn capacity_bytes(&self) -> Option<usize> { self.capacity().checked_mul(std::mem::size_of::<T>()) }
    fn reserve_bytes(&mut self, bytes: usize) -> Result<(), ProjectionError> {
        let size = std::mem::size_of::<T>();
        if size == 0 || !bytes.is_multiple_of(size) { return Err(ProjectionError::Limit); }
        self.try_reserve_exact((bytes / size).saturating_sub(self.len())).map_err(|_| ProjectionError::Allocation)
    }
}

// Field order is intentional: free data before releasing its admitted capacity.
pub(super) struct Owned<T: Capacity> { pub(super) data: T, credit: Credit }
impl<T: Capacity> Owned<T> {
    pub(super) fn new(pool: &Pool) -> Result<Self, ProjectionError> {
        Ok(Self { data: T::default(), credit: pool.charge(0)? })
    }
    pub(super) fn reserve(&mut self, bytes: usize) -> Result<(), ProjectionError> {
        if bytes <= self.data.capacity_bytes().ok_or(ProjectionError::Limit)? { return Ok(()); }
        // Charge the entire candidate allocation while the old allocation stays
        // charged. This conservatively covers realloc's old/new overlap.
        let mut replacement = self.credit.pool.charge_growth(bytes, self.credit.bytes)?;
        self.data.reserve_bytes(bytes)?;
        if self.data.capacity_bytes() != Some(bytes) {
            // try_reserve_exact can overallocate. Refuse and free the complete
            // allocation before either credit; no excess buffer stays retained.
            drop(std::mem::take(&mut self.data));
            return Err(ProjectionError::Limit);
        }
        replacement.local_bytes = bytes;
        self.credit.local_bytes = 0;
        self.credit = replacement;
        Ok(())
    }
    pub(super) fn into_parts(self) -> (T, Credit) { (self.data, self.credit) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_admission_includes_full_old_and_new_allocations() {
        let ledger = Ledger::with_limit(24);
        let pool = ledger.pool(16);
        let mut text = Owned::<String>::new(&pool).unwrap();
        text.reserve(8).unwrap(); text.data.push_str("old");
        assert_eq!(ledger.used(), 8);
        text.reserve(16).unwrap();
        assert_eq!(ledger.used(), 16); assert_eq!(ledger.peak(), 24);
        assert_eq!(text.data, "old");
        assert_eq!(text.reserve(32), Err(ProjectionError::Limit));
        assert_eq!(ledger.used(), 16);
        drop(text); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn pool_refusal_rolls_back_global_admission() {
        let ledger = Ledger::with_limit(128);
        let pool = ledger.pool(8);
        let held = pool.charge(8).unwrap();
        assert!(matches!(pool.charge(1), Err(ProjectionError::Limit)));
        assert_eq!(ledger.used(), 8);
        drop(held); assert_eq!(ledger.used(), 0);
        assert!(ledger.pool(128).charge(129).is_err());
        assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn credit_survives_shared_map_until_the_last_reader() {
        let ledger = Ledger::with_limit(128);
        let pool = ledger.pool(128);
        let mut map = Owned::<Vec<(usize, usize, usize)>>::new(&pool).unwrap();
        map.reserve(24).unwrap(); map.data.push((0, 0, 1));
        let (offsets, credit) = map.into_parts();
        let proof = crate::find_block_map::ValidatedFindBlockMap::from_projection(
            "a", offsets, Box::new(credit),
        ).unwrap();
        let cache = proof.shared(); let result = Arc::clone(&cache);
        drop(proof); drop(cache); assert_eq!(ledger.used(), 24);
        drop(result); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn excess_reported_capacity_is_freed_before_candidate_credit() {
        #[derive(Default)] struct Excess { capacity: usize }
        impl Capacity for Excess {
            fn capacity_bytes(&self) -> Option<usize> { Some(self.capacity) }
            fn reserve_bytes(&mut self, bytes: usize) -> Result<(), ProjectionError> {
                self.capacity = bytes.saturating_add(1); Ok(())
            }
        }
        let ledger = Ledger::with_limit(64);
        let mut excess = Owned::<Excess>::new(&ledger.pool(64)).unwrap();
        assert_eq!(excess.reserve(32), Err(ProjectionError::Limit));
        assert_eq!(excess.data.capacity, 0);
        assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn fallible_reservation_failure_releases_candidate_credit() {
        #[derive(Default)] struct Refused;
        impl Capacity for Refused {
            fn capacity_bytes(&self) -> Option<usize> { Some(0) }
            fn reserve_bytes(&mut self, _: usize) -> Result<(), ProjectionError> { Err(ProjectionError::Allocation) }
        }
        let ledger = Ledger::with_limit(64);
        let mut refused = Owned::<Refused>::new(&ledger.pool(64)).unwrap();
        assert_eq!(refused.reserve(32), Err(ProjectionError::Allocation));
        assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn independent_pools_share_the_workspace_ceiling() {
        let ledger = Ledger::with_limit(10);
        let a = ledger.pool(10).charge(6).unwrap();
        assert!(ledger.pool(10).charge(5).is_err());
        let b = ledger.pool(10).charge(4).unwrap();
        assert_eq!(ledger.used(), 10); drop(a); drop(b);
        assert_eq!(ledger.used(), 0);
    }
}
