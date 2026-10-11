use std::{collections::VecDeque, mem};

use xim::ClientError;

use super::xim_handler::XimCallbackEvent;

const MAX_CALLBACKS: usize = 128;
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

// Keep the usual single-callback path inline. The byte budget charges allocated
// String and deque capacity, including both deques while growth is prepared.
pub(super) struct XimCallbackQueue {
    first: Option<XimCallbackEvent>,
    rest: VecDeque<XimCallbackEvent>,
    text_capacity: usize,
    max_count: usize,
    max_bytes: usize,
    max_text: usize,
}

trait Reserve {
    fn text(&mut self, text: &mut String, capacity: usize) -> Result<(), ClientError>;
    fn events(&mut self, events: &mut VecDeque<XimCallbackEvent>, capacity: usize)
        -> Result<(), ClientError>;
}

struct FallibleReserve;

impl Reserve for FallibleReserve {
    fn text(&mut self, text: &mut String, capacity: usize) -> Result<(), ClientError> {
        text.try_reserve_exact(capacity).map_err(|_| ClientError::InvalidReply)
    }

    fn events(&mut self, events: &mut VecDeque<XimCallbackEvent>, capacity: usize)
        -> Result<(), ClientError>
    {
        events.try_reserve_exact(capacity).map_err(|_| ClientError::InvalidReply)
    }
}

impl XimCallbackQueue {
    pub(super) fn new() -> Self {
        Self {
            first: None,
            rest: VecDeque::new(),
            text_capacity: 0,
            max_count: MAX_CALLBACKS,
            max_bytes: MAX_RETAINED_BYTES,
            max_text: MAX_TEXT_BYTES,
        }
    }

    fn retained_bytes(&self) -> Result<usize, ClientError> {
        self.rest.capacity().checked_mul(mem::size_of::<XimCallbackEvent>())
            .and_then(|bytes| bytes.checked_add(self.text_capacity))
            .ok_or(ClientError::InvalidReply)
    }

    fn check_bytes(&self, extra: usize) -> Result<(), ClientError> {
        let bytes = self.retained_bytes()?.checked_add(extra)
            .ok_or(ClientError::InvalidReply)?;
        if bytes > self.max_bytes { return Err(ClientError::InvalidReply); }
        Ok(())
    }

    fn prepare_slot(&self, text_bytes: usize, reserve: &mut impl Reserve)
        -> Result<Option<VecDeque<XimCallbackEvent>>, ClientError>
    {
        let count = self.rest.len().checked_add(usize::from(self.first.is_some()))
            .ok_or(ClientError::InvalidReply)?;
        if count >= self.max_count || text_bytes > self.max_text {
            return Err(ClientError::InvalidReply);
        }
        self.check_bytes(text_bytes)?;
        if self.first.is_none() || self.rest.len() < self.rest.capacity() {
            return Ok(None);
        }
        let capacity = self.rest.len().checked_add(1).ok_or(ClientError::InvalidReply)?;
        let bytes = capacity.checked_mul(mem::size_of::<XimCallbackEvent>())
            .and_then(|bytes| bytes.checked_add(text_bytes))
            .ok_or(ClientError::InvalidReply)?;
        self.check_bytes(bytes)?;
        let mut grown = VecDeque::new();
        reserve.events(&mut grown, capacity)?;
        if grown.capacity() < capacity { return Err(ClientError::InvalidReply); }
        let bytes = grown.capacity().checked_mul(mem::size_of::<XimCallbackEvent>())
            .and_then(|bytes| bytes.checked_add(text_bytes))
            .ok_or(ClientError::InvalidReply)?;
        self.check_bytes(bytes)?;
        Ok(Some(grown))
    }

    fn insert(&mut self, event: XimCallbackEvent, grown: Option<VecDeque<XimCallbackEvent>>) {
        if self.first.is_none() {
            self.first = Some(event);
        } else {
            if let Some(mut grown) = grown {
                grown.extend(self.rest.drain(..));
                self.rest = grown;
            }
            // prepare_slot reserved and validated a free slot before insertion.
            self.rest.push_back(event);
        }
    }

    pub(super) fn push_xevent(&mut self, event: x11rb::protocol::Event) -> Result<(), ClientError> {
        let grown = self.prepare_slot(0, &mut FallibleReserve)?;
        self.insert(XimCallbackEvent::XimXEvent(event), grown);
        Ok(())
    }

    pub(super) fn push_text(&mut self, text: &str, event: impl FnOnce(String) -> XimCallbackEvent)
        -> Result<(), ClientError>
    {
        self.push_text_with(text, event, &mut FallibleReserve)
    }

    fn push_text_with(&mut self, text: &str, event: impl FnOnce(String) -> XimCallbackEvent,
        reserve: &mut impl Reserve) -> Result<(), ClientError>
    {
        let grown = self.prepare_slot(text.len(), reserve)?;
        let mut copy = String::new();
        reserve.text(&mut copy, text.len())?;
        if copy.capacity() < text.len() || copy.capacity() > self.max_text {
            return Err(ClientError::InvalidReply);
        }
        let deque_bytes = grown.as_ref().map_or(Some(0), |events| {
            events.capacity().checked_mul(mem::size_of::<XimCallbackEvent>())
        }).ok_or(ClientError::InvalidReply)?;
        let extra = deque_bytes.checked_add(copy.capacity()).ok_or(ClientError::InvalidReply)?;
        self.check_bytes(extra)?;
        let text_capacity = self.text_capacity.checked_add(copy.capacity())
            .ok_or(ClientError::InvalidReply)?;
        copy.push_str(text);
        self.insert(event(copy), grown);
        self.text_capacity = text_capacity;
        Ok(())
    }

    pub(super) fn take(&mut self) -> XimCallbackBatch {
        self.text_capacity = 0;
        XimCallbackBatch { first: self.first.take(), rest: mem::take(&mut self.rest) }
    }
}

pub(super) struct XimCallbackBatch {
    first: Option<XimCallbackEvent>,
    rest: VecDeque<XimCallbackEvent>,
}

// The caller restores XIM and releases its state borrow before publication.
// An ACK failure may follow acceptance, so publish the prefix before the error.
pub(super) fn finish_xim_filter(result: Result<bool, ClientError>, mut callbacks: XimCallbackBatch,
    mut publish: impl FnMut(XimCallbackEvent)) -> Result<bool, ClientError>
{
    if let Some(event) = callbacks.first.take() { publish(event); }
    for event in callbacks.rest { publish(event); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(text: String) -> XimCallbackEvent { XimCallbackEvent::XimCommitEvent(123, text) }
    fn texts(batch: XimCallbackBatch) -> Vec<String> {
        let mut texts = Vec::new();
        assert!(finish_xim_filter(Ok(true), batch, |event| {
            if let XimCallbackEvent::XimCommitEvent(_, text) = event { texts.push(text); }
        }).is_ok());
        texts
    }

    struct TestReserve { reject_text: bool, reject_events: bool, extra_text: usize, extra_events: usize }
    impl Reserve for TestReserve {
        fn text(&mut self, text: &mut String, capacity: usize) -> Result<(), ClientError> {
            if self.reject_text { return Err(ClientError::InvalidReply); }
            FallibleReserve.text(text, capacity + self.extra_text)
        }
        fn events(&mut self, events: &mut VecDeque<XimCallbackEvent>, capacity: usize)
            -> Result<(), ClientError>
        {
            if self.reject_events { return Err(ClientError::InvalidReply); }
            FallibleReserve.events(events, capacity + self.extra_events)
        }
    }
    fn reserve() -> TestReserve {
        TestReserve { reject_text: false, reject_events: false, extra_text: 0, extra_events: 0 }
    }

    #[test]
    fn single_callback_uses_no_deque_allocation_and_take_releases_capacity() {
        let mut queue = XimCallbackQueue::new();
        queue.push_text("first", commit).unwrap();
        assert_eq!(queue.rest.capacity(), 0);
        assert_eq!(queue.retained_bytes().unwrap(), 5);
        assert_eq!(texts(queue.take()), ["first"]);
        assert_eq!(queue.retained_bytes().unwrap(), 0);
        assert!(texts(queue.take()).is_empty());
    }

    #[test]
    fn count_boundary_keeps_the_accepted_prefix() {
        let mut queue = XimCallbackQueue::new();
        for _ in 0..MAX_CALLBACKS { queue.push_text("a", commit).unwrap(); }
        assert!(queue.push_text("rejected", commit).is_err());
        assert_eq!(texts(queue.take()).len(), MAX_CALLBACKS);
    }

    #[test]
    fn exact_text_and_retained_byte_limits() {
        let mut queue = XimCallbackQueue::new();
        queue.max_text = 5;
        queue.max_bytes = 5;
        queue.push_text("first", commit).unwrap();
        assert!(queue.push_text("", commit).is_err()); // deque allocation is charged
        assert_eq!(texts(queue.take()), ["first"]);
        assert!(queue.push_text("longer", commit).is_err());
        assert_eq!(queue.retained_bytes().unwrap(), 0);
    }

    #[test]
    fn actual_string_capacity_is_charged_before_copy() {
        let mut queue = XimCallbackQueue::new();
        queue.max_bytes = 5;
        let mut allocation = reserve();
        allocation.extra_text = 5;
        assert!(queue.push_text_with("a", commit, &mut allocation).is_err());
        assert!(texts(queue.take()).is_empty());
        queue.max_bytes = 6;
        queue.push_text_with("a", commit, &mut allocation).unwrap();
        assert_eq!(queue.retained_bytes().unwrap(), 6);
        assert_eq!(texts(queue.take()), ["a"]);
    }

    #[test]
    fn actual_deque_capacity_refusal_preserves_prefix_and_live_budget() {
        let mut queue = XimCallbackQueue::new();
        queue.push_text("a", commit).unwrap();
        queue.max_bytes = 2 + mem::size_of::<XimCallbackEvent>();
        let mut allocation = reserve();
        allocation.extra_events = 1;
        assert!(queue.push_text_with("b", commit, &mut allocation).is_err());
        assert_eq!(queue.rest.capacity(), 0);
        assert_eq!(queue.retained_bytes().unwrap(), 1);
        queue.push_text("b", commit).unwrap();
        assert_eq!(queue.retained_bytes().unwrap(), queue.max_bytes);
        assert_eq!(texts(queue.take()), ["a", "b"]);
    }

    #[test]
    fn allocation_refusal_does_not_mutate_prefix() {
        for reject_text in [true, false] {
            let mut queue = XimCallbackQueue::new();
            queue.push_text("first", commit).unwrap();
            let mut allocation = reserve();
            allocation.reject_text = reject_text;
            allocation.reject_events = !reject_text;
            assert!(queue.push_text_with("second", commit, &mut allocation).is_err());
            assert_eq!(queue.rest.capacity(), 0);
            assert_eq!(texts(queue.take()), ["first"]);
        }
    }

    #[test]
    fn per_text_limit_charges_capacity_and_failed_growth_keeps_existing_deque() {
        let mut queue = XimCallbackQueue::new();
        queue.max_text = 5;
        let mut allocation = reserve();
        allocation.extra_text = 5;
        assert!(queue.push_text_with("a", commit, &mut allocation).is_err());
        assert_eq!(queue.retained_bytes().unwrap(), 0);
        queue.push_text("a", commit).unwrap();
        queue.push_text("b", commit).unwrap();
        let old_capacity = queue.rest.capacity();
        let old_bytes = queue.retained_bytes().unwrap();
        allocation = reserve();
        allocation.reject_text = true;
        assert!(queue.push_text_with("c", commit, &mut allocation).is_err());
        assert_eq!(queue.rest.capacity(), old_capacity);
        assert_eq!(queue.retained_bytes().unwrap(), old_bytes);
        assert_eq!(texts(queue.take()), ["a", "b"]);
        assert_eq!(queue.rest.capacity(), 0);
        assert_eq!(queue.retained_bytes().unwrap(), 0);
    }
}
