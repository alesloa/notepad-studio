// X Consortium XIM protocol, Appendix D.6/D.7: property-with-CM output.
use super::*;

pub(super) fn send_property<C: HasConnection>(
    connection: &C, target: Window, protocol: Atom, buf: &mut Vec<u8>,
    sequence: &mut u16, req: &Request,
) -> Result<(), ClientError> {
    let size = req.size();
    if !(4..=client_transport::MAX_PACKET).contains(&size) || size % 4 != 0 {
        return Err(ClientError::InvalidReply);
    }
    buf.clear();
    buf.try_reserve_exact(size).map_err(|_| ClientError::InvalidReply)?;
    buf.resize(size, 0);
    xim_parser::write(req, buf);
    // Keep the existing finite client atom namespace, including its wrap behavior.
    let atom = connection.conn().intern_atom(false, format!("_XIM_DATA_{}", sequence).as_bytes())?.reply()?.atom;
    *sequence = sequence.wrapping_add(1);
    connection.conn().change_property(PropMode::APPEND, target, atom, AtomEnum::STRING,
        8, size as u32, buf)?;
    connection.conn().send_event(false, target, EventMask::NO_EVENT, ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 0, window: target,
        type_: protocol, data: [size as u32, atom, 0, 0, 0].into(),
    })?;
    buf.clear();
    connection.conn().flush()?;
    Ok(())
}
