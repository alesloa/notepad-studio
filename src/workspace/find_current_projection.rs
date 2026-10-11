//! Compact Rich search projection. Text is copied in bounded UI steps; only
//! owned chunks and plain block offsets cross into the background worker.

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use super::find_current_budget::{Owned, Pool};

pub(super) const FRAME_TEXT_BYTES: usize = 64 * 1024;
pub(super) const FRAME_NODES: usize = 1024;
const FRAME_BOUNDARIES: usize = FRAME_NODES * 2;
pub(super) const SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub(super) const RETAINED_BYTES: usize = 80 * 1024 * 1024;
const CHUNK_SLOTS: usize = 4;

pub(super) trait ProjectionSource {
    fn block_count(&self) -> usize;
    /// None denotes a nontextual block, which is skipped without traversal.
    fn run_count(&self, block: usize) -> Option<usize>;
    fn run_text(&self, block: usize, run: usize) -> Option<&str>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectionError { Allocation, SourceChanged, Limit, InvalidChunk }

/// Permits count chunks held by the producer, channel, and worker together.
/// Channel capacity alone does not bound all retained chunks.
#[derive(Clone, Default)]
pub(super) struct ChunkSlots(Arc<AtomicUsize>);
impl ChunkSlots {
    pub(super) fn acquire(&self) -> Option<ChunkPermit> {
        let mut count = self.0.load(Ordering::Acquire);
        loop {
            if count >= CHUNK_SLOTS { return None; }
            match self.0.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(ChunkPermit(self.0.clone())),
                Err(actual) => count = actual,
            }
        }
    }
}
pub(super) struct ChunkPermit(Arc<AtomicUsize>);
impl Drop for ChunkPermit {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); }
}

#[derive(Clone, Copy, Debug)]
struct Boundary { block: usize, at: usize, end: bool }
pub(super) struct ProjectionChunk {
    text: Owned<String>,
    boundaries: Owned<Vec<Boundary>>,
    _permit: ChunkPermit,
    complete: bool,
}
impl ProjectionChunk {
    fn new(permit: ChunkPermit, pool: &Pool) -> Result<Self, ProjectionError> {
        let mut text = Owned::<String>::new(pool)?;
        text.reserve(FRAME_TEXT_BYTES)?;
        let mut boundaries = Owned::<Vec<Boundary>>::new(pool)?;
        boundaries.reserve(FRAME_BOUNDARIES * std::mem::size_of::<Boundary>())?;
        if text.data.capacity() > FRAME_TEXT_BYTES || boundaries.data.capacity() > FRAME_BOUNDARIES {
            return Err(ProjectionError::Limit);
        }
        Ok(Self { text, boundaries, _permit: permit, complete: false })
    }
    pub(super) fn text_bytes(&self) -> usize { self.text.data.len() }
    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.text.data.capacity().saturating_add(self.boundaries.data.capacity()
            .saturating_mul(std::mem::size_of::<Boundary>()))
    }
}

#[derive(Default, Debug)]
pub(super) struct CaptureCursor {
    block: usize,
    run: usize,
    byte: usize,
    open: bool,
    textual_before: bool,
    logical_bytes: usize,
}
pub(super) struct CaptureStep {
    pub(super) chunk: ProjectionChunk,
    pub(super) nodes: usize,
    pub(super) done: bool,
}
impl CaptureCursor {
    /// Call at most once per scheduled frame after validating source ownership.
    /// A caller must acquire a permit before entering this method.
    pub(super) fn step(&mut self, source: &impl ProjectionSource, permit: ChunkPermit, pool: &Pool)
        -> Result<CaptureStep, ProjectionError>
    {
        let mut chunk = ProjectionChunk::new(permit, pool)?;
        let mut nodes = 0;
        while self.block < source.block_count() && nodes < FRAME_NODES
            && chunk.text.data.len() < FRAME_TEXT_BYTES && chunk.boundaries.data.len() < FRAME_BOUNDARIES
        {
            if !self.open {
                nodes += 1;
                let Some(_) = source.run_count(self.block) else {
                    self.block += 1;
                    continue;
                };
                if self.textual_before { chunk.text.data.push('\n'); }
                chunk.boundaries.data.push(Boundary { block: self.block, at: chunk.text.data.len(), end: false });
                self.open = true;
                self.run = 0;
                self.byte = 0;
                self.textual_before = true;
            }
            let runs = source.run_count(self.block).ok_or(ProjectionError::SourceChanged)?;
            if self.run >= runs {
                if chunk.boundaries.data.len() == FRAME_BOUNDARIES { break; }
                chunk.boundaries.data.push(Boundary { block: self.block, at: chunk.text.data.len(), end: true });
                self.block += 1;
                self.open = false;
                continue;
            }
            if nodes >= FRAME_NODES { break; }
            nodes += 1;
            let text = source.run_text(self.block, self.run).ok_or(ProjectionError::SourceChanged)?;
            let tail = text.get(self.byte..).ok_or(ProjectionError::SourceChanged)?;
            let room = FRAME_TEXT_BYTES.saturating_sub(chunk.text.data.len());
            let take = tail.floor_char_boundary(room.min(tail.len()));
            if take == 0 && !tail.is_empty() { break; }
            chunk.text.data.push_str(tail.get(..take).ok_or(ProjectionError::SourceChanged)?);
            self.byte = self.byte.checked_add(take).ok_or(ProjectionError::Limit)?;
            if self.byte == text.len() { self.run += 1; self.byte = 0; }
        }
        self.logical_bytes = self.logical_bytes.checked_add(chunk.text.data.len()).ok_or(ProjectionError::Limit)?;
        if self.logical_bytes > SOURCE_BYTES { return Err(ProjectionError::Limit); }
        chunk.complete = self.block == source.block_count();
        let done = chunk.complete;
        Ok(CaptureStep { chunk, nodes, done })
    }
}

pub(super) struct Projection {
    pub(super) text: Owned<String>,
    pub(super) offsets: Owned<Vec<(usize, usize, usize)>>,
}
pub(super) struct ProjectionBuilder {
    text: Owned<String>,
    offsets: Owned<Vec<(usize, usize, usize)>>,
    open: Option<(usize, usize)>,
    last_block: Option<usize>,
    complete: bool,
}
impl ProjectionBuilder {
    pub(super) fn new(pool: &Pool) -> Result<Self, ProjectionError> {
        Ok(Self { text: Owned::new(pool)?, offsets: Owned::new(pool)?, open: None, last_block: None, complete: false })
    }
    pub(super) fn push(&mut self, chunk: ProjectionChunk) -> Result<(), ProjectionError> {
        if self.complete { return Err(ProjectionError::InvalidChunk); }
        let base = self.text.data.len();
        let needed = base.checked_add(chunk.text.data.len()).ok_or(ProjectionError::Limit)?;
        if needed > SOURCE_BYTES { return Err(ProjectionError::Limit); }
        let text_capacity = grown_capacity(self.text.data.capacity(), needed, SOURCE_BYTES, 1024 * 1024)?;
        let ends = chunk.boundaries.data.iter().filter(|b| b.end).count();
        let offsets_needed = self.offsets.data.len().checked_add(ends).ok_or(ProjectionError::Limit)?;
        let offset_capacity = grown_capacity(self.offsets.data.capacity(), offsets_needed,
            RETAINED_BYTES / std::mem::size_of::<(usize, usize, usize)>(), 1024)?;
        let retained = text_capacity.checked_add(offset_capacity.checked_mul(
            std::mem::size_of::<(usize, usize, usize)>()).ok_or(ProjectionError::Limit)?)
            .and_then(|n| n.checked_add(CHUNK_SLOTS * (FRAME_TEXT_BYTES
                + FRAME_BOUNDARIES * std::mem::size_of::<Boundary>())))
            .ok_or(ProjectionError::Limit)?;
        if retained > RETAINED_BYTES { return Err(ProjectionError::Limit); }
        if text_capacity > self.text.data.capacity() {
            self.text.reserve(text_capacity)?;
        }
        if offset_capacity > self.offsets.data.capacity() {
            self.offsets.reserve(offset_capacity.checked_mul(std::mem::size_of::<(usize, usize, usize)>())
                .ok_or(ProjectionError::Limit)?)?;
        }
        let actual = self.text.data.capacity().checked_add(self.offsets.data.capacity().checked_mul(
            std::mem::size_of::<(usize, usize, usize)>()).ok_or(ProjectionError::Limit)?)
            .and_then(|n| n.checked_add(CHUNK_SLOTS * (FRAME_TEXT_BYTES
                + FRAME_BOUNDARIES * std::mem::size_of::<Boundary>())))
            .ok_or(ProjectionError::Limit)?;
        if actual > RETAINED_BYTES || self.text.data.capacity() > SOURCE_BYTES {
            return Err(ProjectionError::Limit);
        }
        let mut previous = 0;
        for boundary in &chunk.boundaries.data {
            if boundary.at < previous || boundary.at > chunk.text.data.len()
                || !chunk.text.data.is_char_boundary(boundary.at)
            { return Err(ProjectionError::InvalidChunk); }
            previous = boundary.at;
            let at = base.checked_add(boundary.at).ok_or(ProjectionError::Limit)?;
            if boundary.end {
                let Some((block, start)) = self.open.take() else { return Err(ProjectionError::InvalidChunk); };
                if block != boundary.block || start > at { return Err(ProjectionError::InvalidChunk); }
                self.offsets.data.push((block, start, at));
                self.last_block = Some(block);
            } else {
                if self.open.is_some() || self.last_block.is_some_and(|last| last >= boundary.block) {
                    return Err(ProjectionError::InvalidChunk);
                }
                self.open = Some((boundary.block, at));
            }
        }
        self.text.data.push_str(&chunk.text.data);
        self.complete = chunk.complete;
        Ok(())
    }
    pub(super) fn finish(self) -> Result<Projection, ProjectionError> {
        if !self.complete || self.open.is_some() { return Err(ProjectionError::InvalidChunk); }
        Ok(Projection { text: self.text, offsets: self.offsets })
    }
}
fn grown_capacity(current: usize, needed: usize, maximum: usize, initial: usize)
    -> Result<usize, ProjectionError>
{
    if needed > maximum { return Err(ProjectionError::Limit); }
    if needed <= current { return Ok(current); }
    let mut next = current.max(initial).min(maximum);
    while next < needed { next = next.saturating_mul(2).min(maximum); }
    Ok(next)
}

impl ProjectionSource for crate::rich_doc::RichDocPayload {
    fn block_count(&self) -> usize { self.ast.blocks.len() }
    fn run_count(&self, block: usize) -> Option<usize> { self.block_runs(block).map(<[_]>::len) }
    fn run_text(&self, block: usize, run: usize) -> Option<&str> {
        self.block_runs(block)?.get(run).map(|run| run.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::find_current_budget::Ledger;
    struct Source(Vec<Option<Vec<String>>>);
    impl ProjectionSource for Source {
        fn block_count(&self) -> usize { self.0.len() }
        fn run_count(&self, block: usize) -> Option<usize> {
            self.0.get(block).and_then(|b| b.as_ref()).map(Vec::len)
        }
        fn run_text(&self, block: usize, run: usize) -> Option<&str> {
            self.0.get(block)?.as_ref()?.get(run).map(String::as_str)
        }
    }
    fn project(source: &impl ProjectionSource) -> Projection {
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        let mut cursor = CaptureCursor::default();
        let mut builder = ProjectionBuilder::new(&pool).unwrap();
        loop {
            let step = cursor.step(source, slots.acquire().unwrap(), &pool).unwrap();
            assert!(step.nodes <= FRAME_NODES);
            assert!(step.chunk.text.data.len() <= FRAME_TEXT_BYTES);
            assert!(step.chunk.retained_bytes() <= FRAME_TEXT_BYTES
                + FRAME_BOUNDARIES * std::mem::size_of::<Boundary>());
            let done = step.done;
            builder.push(step.chunk).unwrap();
            if done { return builder.finish().unwrap(); }
        }
    }
    #[test]
    fn production_rich_ast_adapter_matches_visible_projection() {
        let payload = crate::rich_doc::rtf::open_bytes(
            br"{\rtf1\ansi first\par second\par third}".to_vec(),
        ).unwrap();
        let projection = project(&payload);
        let (expected, offsets) = payload.flat_search_text();
        assert_eq!(projection.text.data, expected);
        assert_eq!(projection.offsets.data, offsets);
    }
    #[test]
    fn visible_projection_preserves_runs_blocks_and_empty_paragraphs() {
        let p = project(&Source(vec![None, Some(vec!["中".into(), " café".into()]),
            Some(Vec::new()), None, Some(vec!["last".into()])]));
        assert_eq!(p.text.data, "中 café\n\nlast");
        assert_eq!(p.offsets.data, vec![(1, 0, 9), (2, 10, 10), (4, 11, 15)]);
    }
    #[test]
    fn long_utf8_run_crosses_frames_without_truncation() {
        let text = "中😀é".repeat(40_000);
        let p = project(&Source(vec![Some(vec![text.clone()])]));
        assert_eq!(p.text.data, text);
        assert_eq!(p.offsets.data, vec![(0, 0, text.len())]);
    }
    #[test]
    fn empty_runs_and_skipped_blocks_yield_by_node_budget() {
        let p = project(&Source(vec![None; 10_000]));
        assert!(p.text.data.is_empty());
        let p = project(&Source(vec![Some(vec![String::new(); 10_000])]));
        assert_eq!(p.offsets.data, vec![(0, 0, 0)]);
    }
    #[test]
    fn slots_bound_queued_worker_and_producer_chunks_together() {
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        let mut held = Vec::new();
        for _ in 0..4 { held.push(ProjectionChunk::new(slots.acquire().unwrap(), &pool).unwrap()); }
        assert!(slots.acquire().is_none());
        let worker_chunk = held.remove(0);
        assert!(slots.acquire().is_none());
        drop(worker_chunk);
        let producer = slots.acquire().unwrap();
        assert!(slots.acquire().is_none());
        drop(producer);
        assert!(slots.acquire().is_some());
    }
    #[test]
    fn worker_capacity_growth_is_geometric_across_actual_frames() {
        let source = Source(vec![Some(vec!["x".repeat(6 * 1024 * 1024)])]);
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        let mut cursor = CaptureCursor::default();
        let mut builder = ProjectionBuilder::new(&pool).unwrap();
        let mut capacity = 0;
        let mut changes = 0;
        loop {
            let step = cursor.step(&source, slots.acquire().unwrap(), &pool).unwrap();
            let done = step.done;
            builder.push(step.chunk).unwrap();
            if builder.text.data.capacity() != capacity {
                changes += 1;
                capacity = builder.text.data.capacity();
            }
            if done { break; }
        }
        assert!(changes <= 4, "capacity grew {changes} times across 97 frames");
        assert_eq!(builder.finish().unwrap().text.data.len(), 6 * 1024 * 1024);
    }
    #[test]
    fn over_budget_capture_refuses_whole_projection_and_releases_chunk_slot() {
        let source = Source(vec![Some(vec!["x".repeat(SOURCE_BYTES + 1)])]);
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        let mut cursor = CaptureCursor::default();
        loop {
            match cursor.step(&source, slots.acquire().unwrap(), &pool) {
                Ok(step) => { assert!(!step.done); drop(step); }
                Err(error) => { assert_eq!(error, ProjectionError::Limit); break; }
            }
        }
        assert_eq!(slots.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn changed_source_run_is_refused_without_retaining_slot() {
        struct Changed;
        impl ProjectionSource for Changed {
            fn block_count(&self) -> usize { 1 }
            fn run_count(&self, _: usize) -> Option<usize> { Some(1) }
            fn run_text(&self, _: usize, _: usize) -> Option<&str> { None }
        }
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        assert!(matches!(CaptureCursor::default().step(&Changed, slots.acquire().unwrap(), &pool),
            Err(ProjectionError::SourceChanged)));
        assert_eq!(slots.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn balanced_but_incomplete_block_prefix_cannot_publish() {
        let slots = ChunkSlots::default();
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let source = Source(vec![Some(vec![]); FRAME_NODES + 1]);
        let step = CaptureCursor::default().step(&source, slots.acquire().unwrap(), &pool).unwrap();
        assert!(!step.done);
        let mut builder = ProjectionBuilder::new(&pool).unwrap();
        builder.push(step.chunk).unwrap(); assert!(builder.open.is_none());
        assert!(matches!(builder.finish(), Err(ProjectionError::InvalidChunk)));
        assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn unfinished_projection_cannot_publish_partial_text() {
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        let source = Source(vec![Some(vec!["x".repeat(FRAME_TEXT_BYTES + 1)])]);
        let mut cursor = CaptureCursor::default();
        let step = cursor.step(&source, slots.acquire().unwrap(), &pool).unwrap();
        assert!(!step.done);
        let mut builder = ProjectionBuilder::new(&pool).unwrap();
        builder.push(step.chunk).unwrap();
        assert!(matches!(builder.finish(), Err(ProjectionError::InvalidChunk)));
    }
    #[test]
    fn continued_block_and_empty_paragraphs_keep_boundary_capacity_bounded() {
        let mut blocks = vec![Some(vec!["x".repeat(FRAME_TEXT_BYTES)])];
        blocks.extend((0..4096).map(|_| Some(Vec::new())));
        let p = project(&Source(blocks));
        assert_eq!(p.offsets.data.len(), 4097);
        assert_eq!(p.text.data.len(), FRAME_TEXT_BYTES + 4096);
    }
    #[test]
    fn invalid_map_boundary_is_refused() {
        let slots = ChunkSlots::default();
        let ledger = Ledger::default();
        let pool = ledger.pool(RETAINED_BYTES);
        let mut chunk = ProjectionChunk::new(slots.acquire().unwrap(), &pool).unwrap();
        chunk.text.data.push('中');
        chunk.boundaries.data.push(Boundary { block: 0, at: 1, end: false });
        assert_eq!(ProjectionBuilder::new(&pool).unwrap().push(chunk), Err(ProjectionError::InvalidChunk));
    }
}
