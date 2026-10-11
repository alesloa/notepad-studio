// X Consortium XIM protocol, Appendix D: transport 0.1, Tables D.4/D.5.
use super::*;

pub(super) const MAX_PACKET: usize = 4 + (u16::MAX as usize) * 4;

#[derive(Default)]
pub(super) struct ClientTransport {
    pub(super) more_data: Option<Atom>,
    incoming: Vec<u8>,
    expected: usize,
}

impl ClientTransport {
    pub(super) fn clear(&mut self) {
        self.incoming = Vec::new();
        self.expected = 0;
    }

    #[cfg(test)]
    pub(super) fn retained(&self) -> (usize, usize) {
        (self.incoming.len(), self.incoming.capacity())
    }

    pub(super) fn reset(&mut self, more_data: Option<Atom>) {
        self.clear();
        self.more_data = more_data;
    }

    pub(super) fn receive(&mut self, msg: &ClientMessageEvent, final_chunk: bool)
        -> Result<Option<Vec<u8>>, ClientError>
    {
        let result = self.receive_chunk(msg, final_chunk);
        if result.is_err() { self.clear(); }
        result
    }

    fn receive_chunk(&mut self, msg: &ClientMessageEvent, final_chunk: bool)
        -> Result<Option<Vec<u8>>, ClientError>
    {
        if msg.format != 8 { return Err(ClientError::InvalidReply); }
        let data = msg.data.as_data8();
        if self.expected == 0 {
            self.expected = 4 + usize::from(u16::from_ne_bytes([data[2], data[3]])) * 4;
            self.incoming.try_reserve_exact(self.expected)
                .map_err(|_| ClientError::InvalidReply)?;
        }
        let remaining = self.expected.checked_sub(self.incoming.len())
            .ok_or(ClientError::InvalidReply)?;
        if final_chunk {
            if remaining == 0 || remaining > 20 || data.get(remaining..).ok_or(ClientError::InvalidReply)?.iter().any(|b| *b != 0) {
                return Err(ClientError::InvalidReply);
            }
            self.incoming.extend_from_slice(data.get(..remaining).ok_or(ClientError::InvalidReply)?);
            self.expected = 0;
            Ok(Some(std::mem::take(&mut self.incoming)))
        } else {
            if remaining <= 20 { return Err(ClientError::InvalidReply); }
            self.incoming.extend_from_slice(&data);
            Ok(None)
        }
    }
}

pub(super) fn send_multi<C: HasConnection>(
    connection: &C, target: Window, more_data: Atom, protocol: Atom,
    buf: &mut Vec<u8>, req: &Request,
) -> Result<(), ClientError> {
    let size = req.size();
    if !(4..=MAX_PACKET).contains(&size) || size % 4 != 0 {
        return Err(ClientError::InvalidReply);
    }
    buf.clear();
    buf.try_reserve_exact(size).map_err(|_| ClientError::InvalidReply)?;
    buf.resize(size, 0);
    xim_parser::write(req, buf);
    for (index, bytes) in buf.chunks(20).enumerate() {
        let mut data = [0; 20];
        data[..bytes.len()].copy_from_slice(bytes);
        let final_chunk = (index + 1) * 20 >= size;
        connection.conn().send_event(false, target, EventMask::NO_EVENT, ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT, format: 8, sequence: 0,
            window: target, type_: if final_chunk { protocol } else { more_data },
            data: data.into(),
        })?;
    }
    buf.clear();
    connection.conn().flush()?;
    Ok(())
}
