// X Consortium XIM protocol, Appendix D: Property-with-CM and APPEND transport.
use super::*;
use core::convert::TryFrom;

const MAX_PENDING_ATOMS: usize = 256;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

struct PendingProperty {
    offset: u32,
    remaining: u32,
}

#[derive(Default)]
pub(super) struct PropertyReceiver {
    pending: AHashMap<Atom, PendingProperty>,
    pending_bytes: usize,
}

impl PropertyReceiver {
    #[cfg(test)]
    pub(super) fn retained(&self) -> (usize, usize) { (self.pending.len(), self.pending_bytes) }

    pub(super) fn frame_budget(&self) -> usize { MAX_PENDING_BYTES.saturating_sub(self.pending_bytes) }

    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.pending_bytes = 0;
    }

    pub(super) fn retire(&mut self, atom: Atom) {
        if let Some(previous) = self.pending.remove(&atom) {
            self.pending_bytes = self.pending_bytes.saturating_sub(previous.remaining as usize);
        }
    }

    pub(super) fn prepare(&self, atom: Atom, length: u32) -> Result<u32, ClientError> {
        let length = usize::try_from(length).map_err(|_| ClientError::InvalidReply)?;
        if !(4..=client_transport::MAX_PACKET).contains(&length) || length % 4 != 0 {
            return Err(ClientError::InvalidReply);
        }
        let previous = self.pending.get(&atom);
        let known = previous.map_or(0, |pending| pending.remaining as usize);
        let extra = length.saturating_sub(known);
        if self.pending_bytes.checked_add(extra).ok_or(ClientError::InvalidReply)? > MAX_PENDING_BYTES {
            return Err(ClientError::InvalidReply);
        }
        let offset = previous.map_or(0, |pending| pending.offset);
        offset.checked_add((length / 4) as u32).ok_or(ClientError::InvalidReply)?;
        Ok(offset)
    }

    pub(super) fn finish(&mut self, atom: Atom, length: u32, remaining: u32)
        -> Result<(), ClientError>
    {
        if remaining % 4 != 0 { return Err(ClientError::InvalidReply); }
        let previous = self.pending.get(&atom);
        let old_remaining = previous.map_or(0, |pending| pending.remaining as usize);
        let offset = previous.map_or(0, |pending| pending.offset);
        let next_offset = offset.checked_add(length / 4).ok_or(ClientError::InvalidReply)?;
        let total = self.pending_bytes.checked_sub(old_remaining)
            .and_then(|total| total.checked_add(remaining as usize))
            .ok_or(ClientError::InvalidReply)?;
        if total.checked_add(length as usize).ok_or(ClientError::InvalidReply)? > MAX_PENDING_BYTES {
            return Err(ClientError::InvalidReply);
        }
        if remaining == 0 {
            self.retire(atom);
        } else {
            if previous.is_none() {
                if self.pending.len() >= MAX_PENDING_ATOMS { return Err(ClientError::InvalidReply); }
                self.pending.try_reserve(1).map_err(|_| ClientError::InvalidReply)?;
            }
            self.pending.insert(atom, PendingProperty { offset: next_offset, remaining });
            self.pending_bytes = total;
        }
        Ok(())
    }
}

pub(super) fn validate_reply(reply: &x11rb::protocol::xproto::GetPropertyReply, length: u32)
    -> Result<Request, ClientError>
{
    let expected = usize::try_from(length).map_err(|_| ClientError::InvalidReply)?;
    if reply.format != 8 || reply.type_ != u32::from(AtomEnum::STRING)
        || reply.value_len != length || reply.value.len() != expected {
        return Err(ClientError::InvalidReply);
    }
    let header = reply.value.get(2..4).ok_or(ClientError::InvalidReply)?;
    let header = header.try_into().map_err(|_| ClientError::InvalidReply)?;
    let declared = 4 + usize::from(u16::from_ne_bytes(header)) * 4;
    if declared != expected { return Err(ClientError::InvalidReply); }
    let request: Request = xim_parser::read(&reply.value)?;
    if request.size() != expected { return Err(ClientError::InvalidReply); }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcements_are_bounded_and_aligned_before_fetch() {
        let state = PropertyReceiver::default();
        for length in [0, 1, 2, 3, 5, 262148, u32::MAX] {
            assert!(matches!(state.prepare(12, length), Err(ClientError::InvalidReply)));
        }
        assert_eq!(state.prepare(12, 4).unwrap(), 0);
        assert_eq!(state.prepare(12, 262144).unwrap(), 0);
    }

    #[test]
    fn deleted_atoms_reset_offset_and_backlog() {
        let mut state = PropertyReceiver::default();
        state.finish(12, 20, 24).unwrap();
        assert_eq!(state.prepare(12, 24).unwrap(), 5);
        state.finish(12, 24, 0).unwrap();
        assert_eq!(state.retained(), (0, 0));
        assert_eq!(state.prepare(12, 20).unwrap(), 0);
    }

    #[test]
    fn full_atom_map_accepts_consumed_packet_but_not_new_tail() {
        let mut state = PropertyReceiver::default();
        for atom in 0..256 { state.finish(atom, 4, 4).unwrap(); }
        assert_eq!(state.retained(), (256, 1024));
        assert_eq!(state.prepare(256, 4).unwrap(), 0);
        state.finish(256, 4, 0).unwrap();
        assert_eq!(state.retained(), (256, 1024));
        assert!(matches!(state.finish(256, 4, 4), Err(ClientError::InvalidReply)));
        assert_eq!(state.retained(), (256, 1024));
    }

    #[test]
    fn full_byte_backlog_can_drain_its_known_frame() {
        let mut state = PropertyReceiver::default();
        state.finish(12, 4, (MAX_PENDING_BYTES - 4) as u32).unwrap();
        assert_eq!(state.prepare(12, 4).unwrap(), 1);
        assert!(matches!(state.prepare(13, 8), Err(ClientError::InvalidReply)));
        state.finish(12, 4, (MAX_PENDING_BYTES - 8) as u32).unwrap();
        assert_eq!(state.retained(), (1, MAX_PENDING_BYTES - 8));
    }

    #[test]
    fn checked_offset_overflow_does_not_change_state() {
        let mut state = PropertyReceiver::default();
        state.pending.insert(12, PendingProperty { offset: u32::MAX - 1, remaining: 8 });
        state.pending_bytes = 8;
        assert!(matches!(state.prepare(12, 8), Err(ClientError::InvalidReply)));
        assert!(matches!(state.finish(12, 8, 4), Err(ClientError::InvalidReply)));
        assert_eq!(state.retained(), (1, 8));
    }

    #[test]
    fn retiring_one_atom_preserves_another_offset() {
        let mut state = PropertyReceiver::default();
        state.finish(12, 20, 20).unwrap();
        state.finish(13, 24, 20).unwrap();
        state.retire(12);
        assert_eq!(state.retained(), (1, 20));
        assert_eq!(state.prepare(13, 20).unwrap(), 6);
        state.clear();
        assert_eq!(state.retained(), (0, 0));
    }
}
