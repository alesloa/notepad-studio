//! Bounded producer/channel/worker handoff for incremental Rich text capture.
use std::time::{Duration, Instant};
use futures::{channel::mpsc, StreamExt};
use super::{find_current_budget::{Credit, Pool}, find_current_projection::{
    CaptureCursor, CaptureStep, ChunkSlots, ProjectionError, ProjectionSource, FRAME_NODES, FRAME_TEXT_BYTES,
}};

pub(super) struct Producer {
    sender: mpsc::Sender<CaptureStep>,
    cursor: CaptureCursor,
    slots: ChunkSlots,
    pool: Pool,
    pending: Option<CaptureStep>,
    _control_credit: Credit,
}
pub(super) struct Consumer { receiver: mpsc::Receiver<CaptureStep>, _control_credit: Credit }
pub(super) struct Progress {
    pub(super) nodes: usize,
    pub(super) bytes: usize,
    pub(super) done: bool,
    pub(super) blocked: bool,
    pub(super) attempts: usize,
}
const FRAME_ATTEMPTS: usize = 16;
pub(super) const FRAME_COPY_BYTES: usize = FRAME_ATTEMPTS * FRAME_TEXT_BYTES;
const FRAME_SOFT_TIME: Duration = Duration::from_millis(2);
pub(super) fn pipe(projection_pool: &Pool, metadata_pool: &Pool) -> Result<(Producer, Consumer), ProjectionError> {
    let producer_credit = metadata_pool.charge(std::mem::size_of::<Producer>())?;
    let consumer_credit = metadata_pool.charge(std::mem::size_of::<Consumer>())?;
    // Buffer3 plus this single sender's reserved slot permits four queue slots.
    // Four aggregate permits also count producer pending and worker-held chunks.
    let (sender, receiver) = mpsc::channel(3);
    Ok((Producer {
        sender, cursor: CaptureCursor::default(), slots: ChunkSlots::default(),
        pool: projection_pool.clone(), pending: None, _control_credit: producer_credit,
    }, Consumer { receiver, _control_credit: consumer_credit }))
}
impl Producer {
    #[cfg(test)]
    pub(super) fn capture(&mut self, source: &impl ProjectionSource) -> Result<Progress, ProjectionError> {
        self.capture_with_nodes(source, FRAME_NODES)
    }
    /// Admit bounded quanta against time measured before the entity update.
    /// The first quantum always runs; the time target cannot starve progress.
    pub(super) fn capture_frame(&mut self, source: &impl ProjectionSource, started: Instant)
        -> Result<Progress, ProjectionError>
    {
        self.capture_frame_with_elapsed(source, || started.elapsed())
    }
    pub(super) fn capture_frame_with_elapsed(&mut self, source: &impl ProjectionSource,
        mut elapsed: impl FnMut() -> Duration) -> Result<Progress, ProjectionError>
    {
        let mut total = Progress { nodes: 0, bytes: 0, done: false, blocked: false, attempts: 0 };
        while total.attempts < FRAME_ATTEMPTS && total.nodes < FRAME_NODES && total.bytes < FRAME_COPY_BYTES {
            if total.attempts != 0 && elapsed() >= FRAME_SOFT_TIME { break; }
            let remaining = FRAME_NODES.saturating_sub(total.nodes);
            total.attempts += 1;
            let progress = self.capture_with_nodes(source, remaining)?;
            if progress.nodes > remaining || progress.bytes > FRAME_TEXT_BYTES { return Err(ProjectionError::InvalidChunk); }
            total.nodes += progress.nodes;
            total.bytes += progress.bytes;
            total.done = progress.done;
            total.blocked = progress.blocked;
            if total.blocked || total.done { break; }
        }
        Ok(total)
    }
    // Full sends retain their existing chunk, with no additional copy or
    // allocation. Node admission is shared across all quanta in one frame.
    fn capture_with_nodes(&mut self, source: &impl ProjectionSource, nodes: usize) -> Result<Progress, ProjectionError> {
        if nodes == 0 { return Err(ProjectionError::InvalidChunk); }
        let (step, nodes, bytes) = if let Some(step) = self.pending.take() { (step, 0, 0) } else {
            let Some(permit) = self.slots.acquire() else {
                return Ok(Progress { nodes: 0, bytes: 0, done: false, blocked: true, attempts: 1 });
            };
            let step = self.cursor.step_with_node_budget(source, permit, &self.pool, nodes)?;
            let nodes = step.nodes; let bytes = step.chunk.text_bytes();
            (step, nodes, bytes)
        };
        let done = step.done;
        match self.sender.try_send(step) {
            Ok(()) => Ok(Progress { nodes, bytes, done, blocked: false, attempts: 1 }),
            Err(error) if error.is_full() => {
                self.pending = Some(error.into_inner());
                Ok(Progress { nodes, bytes, done: false, blocked: true, attempts: 1 })
            }
            Err(_) => Err(ProjectionError::SourceChanged),
        }
    }
}
impl Consumer {
    pub(super) async fn next(&mut self) -> Option<CaptureStep> { self.receiver.next().await }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{find_current_budget::Ledger, find_current_projection::{
        FRAME_NODES, FRAME_TEXT_BYTES, RETAINED_BYTES, ProjectionBuilder,
    }};
    struct Source(String);
    impl ProjectionSource for Source {
        fn block_count(&self) -> usize { 1 }
        fn run_count(&self, _: usize) -> Option<usize> { Some(1) }
        fn run_text(&self, _: usize, _: usize) -> Option<&str> { Some(&self.0) }
    }
    #[test]
    fn actual_channel_bounds_worker_queued_and_producer_chunks_together() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let metadata = ledger.pool(1024 * 1024);
        let (mut producer, mut consumer) = pipe(&pool, &metadata).unwrap();
        let fixed = ledger.used();
        let source = Source("x".repeat(FRAME_TEXT_BYTES * 10));
        assert!(!producer.capture(&source).unwrap().blocked);
        let worker_held = futures::executor::block_on(consumer.next()).unwrap();
        let chunk_capacity = worker_held.chunk.retained_bytes();
        for _ in 0..3 {
            let progress = producer.capture(&source).unwrap();
            assert!(!progress.blocked); assert!(progress.nodes <= FRAME_NODES);
            assert!(progress.bytes <= FRAME_TEXT_BYTES);
        }
        assert_eq!(ledger.used(), fixed + 4 * chunk_capacity);
        for _ in 0..100 { assert!(producer.capture(&source).unwrap().blocked); }
        assert_eq!(ledger.used(), fixed + 4 * chunk_capacity);
        drop(worker_held);
        let progress = producer.capture(&source).unwrap();
        assert!(progress.nodes <= FRAME_NODES);
        assert_eq!(ledger.used(), fixed + 4 * chunk_capacity);
        drop(consumer); drop(producer); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn actual_channel_finishes_complete_text_and_detects_disconnect() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let metadata = ledger.pool(1024 * 1024);
        let (mut producer, mut consumer) = pipe(&pool, &metadata).unwrap();
        let source = Source("a\n中".into());
        assert!(producer.capture(&source).unwrap().done);
        let step = futures::executor::block_on(consumer.next()).unwrap(); assert!(step.done);
        let mut builder = ProjectionBuilder::new(&pool).unwrap(); builder.push(step.chunk).unwrap();
        assert_eq!(builder.finish().unwrap().text.data, source.0);
        drop(consumer);
        assert!(matches!(producer.capture(&source), Err(ProjectionError::SourceChanged)));
        drop(producer); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn first_quantum_survives_an_expired_soft_target() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let (mut producer, consumer) = pipe(&pool, &ledger.pool(1024 * 1024)).unwrap();
        let progress = producer.capture_frame_with_elapsed(&Source("x".repeat(FRAME_TEXT_BYTES * 10)),
            || Duration::from_millis(3)).unwrap();
        assert_eq!((progress.bytes, progress.attempts, progress.blocked), (FRAME_TEXT_BYTES, 1, false));
        drop(consumer); drop(producer); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn held_worker_stops_at_first_blocked_attempt_without_extra_copy() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let (mut producer, consumer) = pipe(&pool, &ledger.pool(1024 * 1024)).unwrap();
        let source = Source("x".repeat(FRAME_TEXT_BYTES * 10));
        let progress = producer.capture_frame_with_elapsed(&source, || Duration::ZERO).unwrap();
        assert_eq!((progress.bytes, progress.attempts, progress.blocked), (4 * FRAME_TEXT_BYTES, 5, true));
        let retained = ledger.used();
        for _ in 0..100 {
            let progress = producer.capture_frame_with_elapsed(&source, || Duration::ZERO).unwrap();
            assert_eq!((progress.bytes, progress.nodes, progress.attempts, progress.blocked), (0, 0, 1, true));
            assert_eq!(ledger.used(), retained);
        }
        drop(consumer); drop(producer); assert_eq!(ledger.used(), 0);
    }
    struct DenseSource;
    impl ProjectionSource for DenseSource {
        fn block_count(&self) -> usize { 1 }
        fn run_count(&self, _: usize) -> Option<usize> { Some(3000) }
        fn run_text(&self, _: usize, _: usize) -> Option<&str> { Some("") }
    }
    #[test]
    fn empty_runs_consume_the_aggregate_node_budget_instead_of_spinning() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let (mut producer, mut consumer) = pipe(&pool, &ledger.pool(1024 * 1024)).unwrap();
        for expected in [FRAME_NODES, FRAME_NODES, 3001 - 2 * FRAME_NODES] {
            let progress = producer.capture_frame_with_elapsed(&DenseSource, || Duration::ZERO).unwrap();
            assert_eq!((progress.nodes, progress.bytes, progress.attempts), (expected, 0, 1));
            let step = futures::executor::block_on(consumer.next()).unwrap();
            assert_eq!(step.done, expected != FRAME_NODES);
        }
        drop(consumer); drop(producer); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn concurrent_drain_cannot_exceed_sixteen_quanta_or_one_mib() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let (mut producer, mut consumer) = pipe(&pool, &ledger.pool(1024 * 1024)).unwrap();
        let source = Source("x".repeat(FRAME_COPY_BYTES * 2));
        let progress = producer.capture_frame_with_elapsed(&source, || {
            let step = futures::executor::block_on(consumer.next()).unwrap();
            assert_eq!(step.chunk.text_bytes(), FRAME_TEXT_BYTES);
            drop(step); Duration::ZERO
        }).unwrap();
        assert_eq!((progress.bytes, progress.attempts, progress.done, progress.blocked),
            (FRAME_COPY_BYTES, FRAME_ATTEMPTS, false, false));
        assert!(progress.nodes <= FRAME_NODES);
        drop(consumer); drop(producer); assert_eq!(ledger.used(), 0);
    }
    #[test]
    fn zero_remaining_nodes_refuses_before_allocating_a_chunk() {
        let ledger = Ledger::default(); let pool = ledger.pool(RETAINED_BYTES);
        let (mut producer, consumer) = pipe(&pool, &ledger.pool(1024 * 1024)).unwrap();
        let before = ledger.used();
        assert!(matches!(producer.capture_with_nodes(&DenseSource, 0), Err(ProjectionError::InvalidChunk)));
        assert_eq!(ledger.used(), before);
        drop(consumer); drop(producer); assert_eq!(ledger.used(), 0);
    }

}
