// Shared rich-document search coordinates. Legacy search keeps its existing map;
// background projections must validate their complete flat-text coverage first.
use std::sync::Arc;

type BlockOffset = (usize, usize, usize);

pub(crate) struct FindBlockMap {
    offsets: Vec<BlockOffset>,
    // The credit object belongs to the allocation, not to an individual reader.
    // Its Drop releases the future runtime ledger charge after the final Arc.
    _allocation_credit: Option<Box<dyn Send + Sync>>,
}

impl FindBlockMap {
    /// Move the existing synchronous map without copying or rescanning it.
    pub(crate) fn legacy(offsets: Vec<BlockOffset>) -> Arc<Self> {
        Arc::new(Self { offsets, _allocation_credit: None })
    }

    pub(crate) fn as_slice(&self) -> &[BlockOffset] {
        &self.offsets
    }

    /// Only the checked constructor can create the validated proof wrapper.
    fn validated(offsets: Vec<BlockOffset>, allocation_credit: Box<dyn Send + Sync>) -> ValidatedFindBlockMap {
        ValidatedFindBlockMap(Arc::new(Self {
            offsets,
            _allocation_credit: Some(allocation_credit),
        }))
    }
}

// This proof is reserved for the next background-projection phase. A legacy Arc
// cannot become a proof; neither the wrapper nor map fields are exposed.
#[allow(dead_code)]
pub(crate) struct ValidatedFindBlockMap(Arc<FindBlockMap>);

#[allow(dead_code)]
impl ValidatedFindBlockMap {
    /// The caller supplies an owned credit for the Vec's actual retained capacity.
    /// This validates coordinates only, and does not establish a memory budget.
    pub(crate) fn from_projection(
        text: &str,
        offsets: Vec<BlockOffset>,
        allocation_credit: Box<dyn Send + Sync>,
    ) -> Option<Self> {
        let mut previous: Option<(usize, usize)> = None;
        for &(block, start, end) in &offsets {
            if start > end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return None;
            }
            match previous {
                None if start != 0 => return None,
                Some((previous_block, previous_end))
                    if block <= previous_block || previous_end.checked_add(1) != Some(start)
                        || text.get(previous_end..start) != Some("\n") => return None,
                _ => {}
            }
            previous = Some((block, end));
        }
        if previous.map_or(!text.is_empty(), |(_, end)| end != text.len()) {
            return None;
        }
        Some(FindBlockMap::validated(offsets, allocation_credit))
    }

    pub(crate) fn shared(&self) -> Arc<FindBlockMap> {
        Arc::clone(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Credit(Arc<AtomicUsize>);
    impl Drop for Credit {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn legacy_moves_the_original_allocation_and_clones_only_the_arc() {
        let offsets = vec![(2, 0, 3), (7, 4, 8)];
        let pointer = offsets.as_ptr();
        let map = FindBlockMap::legacy(offsets);
        let reader = Arc::clone(&map);
        assert_eq!(map.as_slice().as_ptr(), pointer);
        assert!(Arc::ptr_eq(&map, &reader));
        assert_eq!(reader.as_slice(), &[(2, 0, 3), (7, 4, 8)]);
    }

    #[test]
    fn projection_accepts_empty_blocks_skipped_ast_indices_and_utf8() {
        let proof = ValidatedFindBlockMap::from_projection(
            "中\n\n😀", vec![(1, 0, 3), (5, 4, 4), (8, 5, 9)], Box::new(()),
        ).unwrap();
        assert_eq!(proof.shared().as_slice(), &[(1, 0, 3), (5, 4, 4), (8, 5, 9)]);
        assert!(ValidatedFindBlockMap::from_projection("", vec![], Box::new(())).is_some());
        assert!(ValidatedFindBlockMap::from_projection("", vec![(9, 0, 0)], Box::new(())).is_some());
    }

    #[test]
    fn invalid_projection_cannot_mint_a_proof_and_releases_its_credit() {
        let cases = [
            ("a", vec![]),
            ("a", vec![(0, 1, 1)]),
            ("a", vec![(0, 0, 2)]),
            ("a", vec![(0, 1, 0)]),
            ("中", vec![(0, 0, 1)]),
            ("a\nb", vec![(0, 0, 1), (0, 2, 3)]),
            ("a\nb", vec![(1, 0, 1), (0, 2, 3)]),
            ("a b", vec![(0, 0, 1), (1, 2, 3)]),
            ("a\nb", vec![(0, 0, 1), (1, 1, 3)]),
            ("ab", vec![(0, 0, 1)]),
            ("a", vec![(0, 0, usize::MAX)]),
        ];
        for (text, offsets) in cases {
            let released = Arc::new(AtomicUsize::new(0));
            assert!(ValidatedFindBlockMap::from_projection(
                text, offsets, Box::new(Credit(Arc::clone(&released))),
            ).is_none());
            assert_eq!(released.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn credit_lives_until_the_last_shared_reader_drops() {
        let released = Arc::new(AtomicUsize::new(0));
        let proof = ValidatedFindBlockMap::from_projection(
            "a", vec![(0, 0, 1)], Box::new(Credit(Arc::clone(&released))),
        ).unwrap();
        let find_cache = proof.shared();
        let result_reader = Arc::clone(&find_cache);
        drop(proof);
        drop(find_cache);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        assert_eq!(result_reader.as_slice(), &[(0, 0, 1)]);
        drop(result_reader);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
