//! Resolve validated flat coordinates without flattening document runs on the UI.
use std::{ops::Range, sync::Arc};
use crate::{find_block_map::ValidatedFindBlockMap, find_in_files::FindResult, rich_doc::TextPos};
use super::{find_current_budget::{Owned, Pool}, find_current_projection::{FRAME_NODES, ProjectionError, ProjectionSource}};

pub(super) const MAX_ENDPOINTS: usize = 2000;
pub(super) const ENDPOINT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug)]
pub(super) struct Endpoint {
    pub(super) row: usize,
    pub(super) head: bool,
    block: usize,
    global: usize,
    byte: usize,
    block_len: usize,
    position: Option<TextPos>,
}

impl Endpoint {
    #[cfg(test)]
    pub(super) fn position(&self) -> Option<TextPos> { self.position }
    pub(super) fn from_global(
        proof: &ValidatedFindBlockMap, global: usize, row: usize, head: bool,
    ) -> Result<Self, ProjectionError> {
        let map = proof.shared();
        let offsets = map.as_slice();
        // The paragraph separator's first coordinate belongs to the preceding
        // paragraph end; its next coordinate belongs to the following block.
        let index = offsets.partition_point(|&(_, start, _)| start <= global);
        let &(block, start, end) = offsets.get(index.saturating_sub(1))
            .filter(|&&(_, start, end)| start <= global && global <= end)
            .ok_or(ProjectionError::InvalidChunk)?;
        Ok(Self { row, head, block, global, byte: global - start, block_len: end - start, position: None })
    }
}

struct Target {
    identity: Arc<()>,
    range: Range<usize>,
    anchor: Option<TextPos>,
    head: Option<TextPos>,
}
/// Its private positions can only be filled by the bounded checked AST pass.
pub(super) struct ResolvedTargets {
    targets: Owned<Vec<Target>>,
    proof: Arc<ValidatedFindBlockMap>,
    proven: bool,
}
impl ResolvedTargets {
    pub(super) fn new(pool: &Pool, proof: Arc<ValidatedFindBlockMap>) -> Result<Self, ProjectionError> {
        Ok(Self { targets: Owned::new(pool)?, proof, proven: false })
    }
    /// Worker-only row metadata preparation; no document text or positions here.
    pub(super) fn initialize(&mut self, rows: &[FindResult], ranges: &[Range<usize>]) -> Result<(), ProjectionError> {
        if rows.len() > MAX_ENDPOINTS / 2 || ranges.len() < rows.len() || !self.targets.data.is_empty() {
            return Err(ProjectionError::InvalidChunk);
        }
        self.targets.reserve(rows.len().checked_mul(std::mem::size_of::<Target>()).ok_or(ProjectionError::Limit)?)?;
        for (row, range) in rows.iter().zip(ranges) {
            self.targets.data.push(Target { identity: Arc::clone(&row.identity), range: range.clone(), anchor: None, head: None });
        }
        Ok(())
    }
    pub(super) fn step(&mut self, resolver: &mut EndpointResolver, source: &impl ProjectionSource) -> Result<usize, ProjectionError> {
        let previous = resolver.next;
        let nodes = resolver.step(source)?;
        for point in resolver.points.data.get(previous..resolver.next).ok_or(ProjectionError::InvalidChunk)? {
            let target = self.targets.data.get_mut(point.row).ok_or(ProjectionError::InvalidChunk)?;
            let expected = if point.head { target.range.end } else { target.range.start };
            if point.global != expected { return Err(ProjectionError::InvalidChunk); }
            let position = point.position.ok_or(ProjectionError::InvalidChunk)?;
            if position.block != point.block { return Err(ProjectionError::InvalidChunk); }
            let slot = if point.head { &mut target.head } else { &mut target.anchor };
            if slot.is_some() { return Err(ProjectionError::InvalidChunk); }
            *slot = Some(position);
        }
        self.proven = resolver.done();
        Ok(nodes)
    }
    pub(super) fn get(&self, row: usize, identity: &Arc<()>) -> Option<(TextPos, TextPos, Range<usize>)> {
        if !self.proven { return None; }
        let target = self.targets.data.get(row)?;
        if !Arc::ptr_eq(&target.identity, identity) { return None; }
        Some((target.anchor?, target.head?, target.range.clone()))
    }
    pub(super) fn map(&self) -> Arc<crate::find_block_map::FindBlockMap> { self.proof.shared() }
    pub(super) fn done(&self) -> bool { self.proven }
}

pub(super) struct EndpointResolver {
    pub(super) points: Owned<Vec<Endpoint>>,
    next: usize,
    block: Option<usize>,
    run: usize,
    bytes: usize,
    last_run_len: usize,
}

impl EndpointResolver {
    /// The worker allocates and sorts this bank before publication is attempted.
    pub(super) fn new(pool: &Pool, count: usize) -> Result<Self, ProjectionError> {
        if count > MAX_ENDPOINTS { return Err(ProjectionError::Limit); }
        let mut points = Owned::<Vec<Endpoint>>::new(pool)?;
        points.reserve(count.checked_mul(std::mem::size_of::<Endpoint>()).ok_or(ProjectionError::Limit)?)?;
        Ok(Self { points, next: 0, block: None, run: 0, bytes: 0, last_run_len: 0 })
    }
    pub(super) fn sort(&mut self) {
        self.points.data.sort_unstable_by_key(|point| (point.block, point.byte));
    }
    pub(super) fn done(&self) -> bool { self.next == self.points.data.len() }

    /// Empty runs and block visits consume the same node allowance as text runs.
    /// Exact run boundaries defer until the next nonempty run, or paragraph end.
    pub(super) fn step(&mut self, source: &impl ProjectionSource) -> Result<usize, ProjectionError> {
        let mut nodes = 0;
        while !self.done() && nodes < FRAME_NODES {
            let point = *self.points.data.get(self.next).ok_or(ProjectionError::InvalidChunk)?;
            if self.block != Some(point.block) {
                self.block = Some(point.block);
                self.run = 0;
                self.bytes = 0;
                self.last_run_len = 0;
                nodes += 1;
                if nodes == FRAME_NODES { break; }
            }
            let runs = source.run_count(point.block).ok_or(ProjectionError::SourceChanged)?;
            if self.run == runs {
                if self.bytes != point.block_len { return Err(ProjectionError::SourceChanged); }
                let final_run = runs.saturating_sub(1);
                while let Some(point) = self.points.data.get_mut(self.next).filter(|p| p.block == self.block.unwrap_or(usize::MAX)) {
                    if point.byte != self.bytes { return Err(ProjectionError::InvalidChunk); }
                    point.position = Some(TextPos { block: point.block, run: final_run, byte: self.last_run_len });
                    self.next += 1;
                }
                continue;
            }
            let text = source.run_text(point.block, self.run).ok_or(ProjectionError::SourceChanged)?;
            let end = self.bytes.checked_add(text.len()).ok_or(ProjectionError::Limit)?;
            if end > point.block_len { return Err(ProjectionError::SourceChanged); }
            nodes += 1;
            while let Some(point) = self.points.data.get_mut(self.next)
                .filter(|p| p.block == self.block.unwrap_or(usize::MAX) && p.byte < end) {
                let byte = point.byte.checked_sub(self.bytes).ok_or(ProjectionError::InvalidChunk)?;
                if !text.is_char_boundary(byte) { return Err(ProjectionError::InvalidChunk); }
                point.position = Some(TextPos { block: point.block, run: self.run, byte });
                self.next += 1;
            }
            self.bytes = end;
            self.last_run_len = text.len();
            self.run += 1;
        }
        Ok(nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::find_current_budget::Ledger;
    struct Source(Vec<Vec<String>>);
    impl ProjectionSource for Source {
        fn block_count(&self) -> usize { self.0.len() }
        fn run_count(&self, block: usize) -> Option<usize> { self.0.get(block).map(Vec::len) }
        fn run_text(&self, block: usize, run: usize) -> Option<&str> { self.0.get(block)?.get(run).map(String::as_str) }
    }
    #[test]
    fn utf8_run_boundaries_and_paragraph_separators_have_exact_positions() {
        let source = Source(vec![vec!["中".into(), "".into(), "😀".into(), "".into()], vec![]]);
        let proof = ValidatedFindBlockMap::from_projection("中😀\n", vec![(0, 0, 7), (1, 8, 8)], Box::new(())).unwrap();
        let ledger = Ledger::default();
        let mut resolver = EndpointResolver::new(&ledger.pool(ENDPOINT_BYTES), 4).unwrap();
        for (row, global) in [0, 3, 7, 8].into_iter().enumerate() {
            resolver.points.data.push(Endpoint::from_global(&proof, global, row, false).unwrap());
        }
        resolver.sort(); assert_eq!(resolver.step(&source).unwrap(), 6); assert!(resolver.done());
        let positions: Vec<_> = resolver.points.data.iter().map(|p| p.position.unwrap()).collect();
        assert_eq!(positions, vec![
            TextPos { block: 0, run: 0, byte: 0 }, TextPos { block: 0, run: 2, byte: 0 },
            TextPos { block: 0, run: 3, byte: 0 }, TextPos { block: 1, run: 0, byte: 0 },
        ]);
        drop(resolver); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn thousands_of_empty_runs_yield_and_do_not_hide_an_unbounded_scan() {
        let mut runs = vec![String::new(); FRAME_NODES * 3]; runs.push("x".into());
        let source = Source(vec![runs]);
        let proof = ValidatedFindBlockMap::from_projection("x", vec![(0, 0, 1)], Box::new(())).unwrap();
        let mut resolver = EndpointResolver::new(&Ledger::default().pool(ENDPOINT_BYTES), 1).unwrap();
        resolver.points.data.push(Endpoint::from_global(&proof, 0, 0, false).unwrap());
        for _ in 0..3 { assert_eq!(resolver.step(&source).unwrap(), FRAME_NODES); assert!(!resolver.done()); }
        assert!(resolver.step(&source).unwrap() <= FRAME_NODES); assert!(resolver.done());
        assert_eq!(resolver.points.data[0].position.unwrap().run, FRAME_NODES * 3);
    }
    #[test]
    fn malformed_utf8_coordinate_or_changed_run_length_is_refused() {
        let proof = ValidatedFindBlockMap::from_projection("中", vec![(0, 0, 3)], Box::new(())).unwrap();
        let pool = Ledger::default().pool(ENDPOINT_BYTES);
        let mut resolver = EndpointResolver::new(&pool, 1).unwrap();
        resolver.points.data.push(Endpoint::from_global(&proof, 1, 0, false).unwrap());
        assert_eq!(resolver.step(&Source(vec![vec!["中".into()]])), Err(ProjectionError::InvalidChunk));
        let mut resolver = EndpointResolver::new(&pool, 1).unwrap();
        resolver.points.data.push(Endpoint::from_global(&proof, 3, 0, false).unwrap());
        assert_eq!(resolver.step(&Source(vec![vec!["x".into()]])), Err(ProjectionError::SourceChanged));
        assert!(Endpoint::from_global(&proof, 4, 0, false).is_err());
    }
}
