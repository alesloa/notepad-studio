//! Bounded producer/channel/worker handoff for incremental Rich text capture.
use futures::{channel::mpsc, StreamExt};
use super::{find_current_budget::{Credit, Pool}, find_current_projection::{
    CaptureCursor, CaptureStep, ChunkSlots, ProjectionError, ProjectionSource,
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
}
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
    /// One guarded call per UI frame. Full sends retain their existing chunk,
    /// with no additional copy/allocation. Source-ownership checks are external.
    pub(super) fn capture(&mut self, source: &impl ProjectionSource) -> Result<Progress, ProjectionError> {
        let (step, nodes, bytes) = if let Some(step) = self.pending.take() { (step, 0, 0) } else {
            let Some(permit) = self.slots.acquire() else {
                return Ok(Progress { nodes: 0, bytes: 0, done: false, blocked: true });
            };
            let step = self.cursor.step(source, permit, &self.pool)?;
            let nodes = step.nodes; let bytes = step.chunk.text_bytes();
            (step, nodes, bytes)
        };
        let done = step.done;
        match self.sender.try_send(step) {
            Ok(()) => Ok(Progress { nodes, bytes, done, blocked: false }),
            Err(error) if error.is_full() => {
                self.pending = Some(error.into_inner());
                Ok(Progress { nodes, bytes, done: false, blocked: true })
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
}
