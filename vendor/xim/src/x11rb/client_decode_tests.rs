// Exercise the production X11 property transport, parser and client dispatcher
// without a display server. The connection supplies only an encoded reply.
use super::*;
use core::convert::TryFrom;
use std::{cell::RefCell, io::IoSlice, vec};
use x11rb::{
    connection::{BufWithFds, DiscardMode, RawEventAndSeqNumber, ReplyOrError,
        RequestConnection, RequestKind, SequenceNumber},
    cookie::{Cookie, CookieWithFds, VoidCookie},
    protocol::xproto::{GetPropertyReply, Setup},
    utils::RawFdContainer,
    x11_utils::{ExtensionInformation, Serialize, TryParse, TryParseFd, X11Error},
};

struct SparseProperty { packet: Vec<u8>, repeats: usize, suffix: Vec<u8> }

#[derive(Default)]
struct WireConnection {
    setup: Setup,
    property: Vec<u8>,
    dynamic_property: bool,
    property_store: RefCell<std::collections::HashMap<u32, Vec<u8>>>,
    deleted_properties: RefCell<Vec<u32>>,
    property_metadata: Option<(u8, u32, u32)>,
    property_value_len: Option<u32>,
    sparse_properties: RefCell<std::collections::HashMap<u32, SparseProperty>>,
    sent: RefCell<Vec<Vec<u8>>>,
    send_attempts: RefCell<usize>,
    reject_send: bool,
    reject_send_at: Option<usize>,
    reject_flush: bool,
    flush_calls: RefCell<usize>,
    reply_opcode: RefCell<u8>,
    with_reply: RefCell<Vec<Vec<u8>>>,
}

impl HasConnection for WireConnection {
    type Connection = Self;
    fn conn(&self) -> &Self { self }
}

impl RequestConnection for WireConnection {
    type Buf = Vec<u8>;
    fn send_request_with_reply<R: TryParse>(&self, bufs: &[IoSlice<'_>], _: Vec<RawFdContainer>)
        -> Result<Cookie<'_, Self, R>, ConnectionError>
    {
        let opcode = bufs.first().and_then(|buf| buf.first()).copied()
            .ok_or(ConnectionError::UnsupportedExtension)?;
        self.reply_opcode.replace(opcode);
        self.with_reply.borrow_mut().push(bufs.iter().flat_map(|buf| buf.iter().copied()).collect());
        Ok(Cookie::new(self, 1))
    }
    fn send_request_with_reply_with_fds<R: TryParseFd>(&self, _: &[IoSlice<'_>], _: Vec<RawFdContainer>)
        -> Result<CookieWithFds<'_, Self, R>, ConnectionError>
    { Err(ConnectionError::UnsupportedExtension) }
    fn send_request_without_reply(&self, bufs: &[IoSlice<'_>], _: Vec<RawFdContainer>)
        -> Result<VoidCookie<'_, Self>, ConnectionError>
    {
        *self.send_attempts.borrow_mut() += 1;
        if self.reject_send || self.reject_send_at == Some(*self.send_attempts.borrow()) { return Err(ConnectionError::UnsupportedExtension); }
        self.sent.borrow_mut().push(bufs.iter().flat_map(|buf| buf.iter().copied()).collect());
        Ok(VoidCookie::new(self, 2))
    }
    fn discard_reply(&self, _: SequenceNumber, _: RequestKind, _: DiscardMode) {}
    fn prefetch_extension_information(&self, _: &'static str) -> Result<(), ConnectionError> { Ok(()) }
    fn extension_information(&self, _: &'static str) -> Result<Option<ExtensionInformation>, ConnectionError> { Ok(None) }
    fn wait_for_reply_or_raw_error(&self, _: SequenceNumber) -> Result<ReplyOrError<Vec<u8>>, ConnectionError> {
        if *self.reply_opcode.borrow() == 16 {
            let reply = x11rb::protocol::xproto::InternAtomReply { sequence: 1, length: 0, atom: 42 };
            let mut bytes = reply.serialize().to_vec();
            bytes.resize(32, 0);
            return Ok(ReplyOrError::Reply(bytes));
        }
        if *self.reply_opcode.borrow() != 20 { return Err(ConnectionError::UnsupportedExtension); }
        let (value, format, type_, bytes_after) = if self.dynamic_property {
            let requests = self.with_reply.borrow();
            let request = requests.last().ok_or(ConnectionError::UnsupportedExtension)?;
            let field = |start| -> Result<u32, ConnectionError> {
                let bytes = request.get(start..start + 4).ok_or(ConnectionError::UnsupportedExtension)?;
                Ok(u32::from_ne_bytes(bytes.try_into().map_err(|_| ConnectionError::UnsupportedExtension)?))
            };
            let atom = field(8)?;
            let start = usize::try_from(field(16)?).map_err(|_| ConnectionError::UnsupportedExtension)?
                .checked_mul(4).ok_or(ConnectionError::UnsupportedExtension)?;
            let count = usize::try_from(field(20)?).map_err(|_| ConnectionError::UnsupportedExtension)?
                .checked_mul(4).ok_or(ConnectionError::UnsupportedExtension)?;
            let mut properties = self.property_store.borrow_mut();
            let mut sparse = self.sparse_properties.borrow_mut();
            if let Some(SparseProperty { packet, repeats, suffix }) = sparse.get(&atom) {
                let repeated_len = packet.len().checked_mul(*repeats).ok_or(ConnectionError::UnsupportedExtension)?;
                let total = repeated_len.checked_add(suffix.len()).ok_or(ConnectionError::UnsupportedExtension)?;
                if start > total { return Err(ConnectionError::UnsupportedExtension); }
                let end = start.saturating_add(count).min(total);
                let mut value = Vec::new();
                for index in start..end {
                    let byte = if index < repeated_len {
                        *packet.get(index % packet.len()).ok_or(ConnectionError::UnsupportedExtension)?
                    } else { *suffix.get(index - repeated_len).ok_or(ConnectionError::UnsupportedExtension)? };
                    value.push(byte);
                }
                let bytes_after = total.saturating_sub(end) as u32;
                if request.get(1) == Some(&1) && bytes_after == 0 {
                    sparse.remove(&atom);
                    self.deleted_properties.borrow_mut().push(atom);
                }
                (value, 8, u32::from(AtomEnum::STRING), bytes_after)
            } else if let Some(property) = properties.get(&atom) {
                if start > property.len() { return Err(ConnectionError::UnsupportedExtension); }
                let end = start.saturating_add(count).min(property.len());
                let value = property.get(start..end).unwrap_or_default().to_vec();
                let bytes_after = property.len().saturating_sub(end) as u32;
                if request.get(1) == Some(&1) && bytes_after == 0 {
                    properties.remove(&atom);
                    self.deleted_properties.borrow_mut().push(atom);
                }
                (value, 8, u32::from(AtomEnum::STRING), bytes_after)
            } else { (Vec::new(), 0, 0, 0) }
        } else { (self.property.clone(), 8, u32::from(AtomEnum::STRING), 0) };
        let (format, type_, bytes_after) = self.property_metadata.unwrap_or((format, type_, bytes_after));
        let length = value.len().saturating_add(3) / 4;
        let value_len = if format == 16 || format == 32 { value.len() / usize::from(format / 8) } else { value.len() };
        let reply = GetPropertyReply { format, sequence: 1, length: length as u32, type_,
            bytes_after, value_len: value_len as u32, value };
        let mut bytes = reply.serialize();
        if let Some(value_len) = self.property_value_len {
            bytes[16..20].copy_from_slice(&value_len.to_ne_bytes());
        }
        bytes.resize(32 + length * 4, 0);
        Ok(ReplyOrError::Reply(bytes))
    }
    fn wait_for_reply(&self, sequence: SequenceNumber) -> Result<Option<Vec<u8>>, ConnectionError> {
        match self.wait_for_reply_or_raw_error(sequence)? {
            ReplyOrError::Reply(reply) => Ok(Some(reply)),
            ReplyOrError::Error(_) => Ok(None),
        }
    }
    fn wait_for_reply_with_fds_raw(&self, _: SequenceNumber)
        -> Result<ReplyOrError<BufWithFds<Vec<u8>>, Vec<u8>>, ConnectionError>
    { Err(ConnectionError::UnsupportedExtension) }
    fn check_for_raw_error(&self, _: SequenceNumber) -> Result<Option<Vec<u8>>, ConnectionError> { Ok(None) }
    fn prefetch_maximum_request_bytes(&self) {}
    fn maximum_request_bytes(&self) -> usize { 65536 }
    fn parse_error(&self, _: &[u8]) -> Result<X11Error, ParseError> { Err(ParseError::InsufficientData) }
    fn parse_event(&self, _: &[u8]) -> Result<Event, ParseError> { Err(ParseError::InsufficientData) }
}

impl Connection for WireConnection {
    fn wait_for_raw_event_with_sequence(&self) -> Result<RawEventAndSeqNumber<Vec<u8>>, ConnectionError> {
        Err(ConnectionError::UnsupportedExtension)
    }
    fn poll_for_raw_event_with_sequence(&self) -> Result<Option<RawEventAndSeqNumber<Vec<u8>>>, ConnectionError> { Ok(None) }
    fn flush(&self) -> Result<(), ConnectionError> {
        *self.flush_calls.borrow_mut() += 1;
        if self.reject_flush { Err(ConnectionError::UnsupportedExtension) } else { Ok(()) }
    }
    fn setup(&self) -> &Setup { &self.setup }
    fn generate_id(&self) -> Result<u32, ReplyOrIdError> { Ok(99) }
}

#[derive(Default)]
struct Handler {
    delivered: Vec<(u16, u16, String)>,
    reject_reset: bool,
    reject_commit: bool,
    commit_calls: usize,
    ack_seen_during_commit: bool,
}

impl ClientHandler<X11rbClient<WireConnection>> for Handler {
    fn handle_reset_ic(&mut self, _: &mut X11rbClient<WireConnection>, im: u16, ic: u16, text: &str)
        -> Result<(), ClientError>
    {
        if self.reject_reset { return Err(ClientError::UnsupportedTransport); }
        self.delivered.push((im, ic, text.into()));
        Ok(())
    }
    fn handle_commit(&mut self, client: &mut X11rbClient<WireConnection>, im: u16, ic: u16, text: &str)
        -> Result<(), ClientError>
    {
        self.commit_calls += 1;
        self.ack_seen_during_commit = !client.has_conn.sent.borrow().is_empty();
        if self.reject_commit { return Err(ClientError::UnsupportedTransport); }
        self.delivered.push((im, ic, text.into()));
        Ok(())
    }
    fn handle_preedit_draw(&mut self, _: &mut X11rbClient<WireConnection>, im: u16, ic: u16,
        _: i32, _: i32, _: i32, _: xim_parser::PreeditDrawStatus, text: &str,
        _: Vec<xim_parser::Feedback>) -> Result<(), ClientError>
    {
        self.delivered.push((im, ic, text.into()));
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ReplyKind { Reset, Commit, Preedit }

fn reply(kind: ReplyKind, text: &[u8]) -> Request {
    match kind {
        ReplyKind::Reset => Request::ResetIcReply {
            input_method_id: 7, input_context_id: 11, preedit_string: text.to_vec(),
        },
        ReplyKind::Commit => Request::Commit {
            input_method_id: 7, input_context_id: 11,
            data: xim_parser::CommitData::Chars { commited: text.to_vec(), syncronous: false },
        },
        ReplyKind::Preedit => Request::PreeditDraw {
            input_method_id: 7, input_context_id: 11, caret: 0, chg_first: 0,
            chg_length: 0, status: xim_parser::PreeditDrawStatus::empty(),
            preedit_string: text.to_vec(), feedbacks: Vec::new(),
        },
    }
}

fn client(property: Vec<u8>) -> X11rbClient<WireConnection> {
    X11rbClient { has_conn: WireConnection { property, ..Default::default() },
        server_owner_window: 2, im_window: 3, server_atom: 4,
        atoms: Atoms { XIM_SERVERS: 5, LOCALES: 6, TRANSPORT: 7,
            XIM_XCONNECT: 8, XIM_PROTOCOL: 9 },
        transport_max: 20, transport: ClientTransport::default(), properties: PropertyReceiver::default(), client_window: 10, im_attributes: Default::default(),
        ic_attributes: Default::default(), sequence: 0, buf: Vec::new() }
}

fn event(length: usize) -> Event {
    Event::ClientMessage(ClientMessageEvent { response_type: CLIENT_MESSAGE_EVENT,
        format: 32, sequence: 0, window: 10, type_: 9,
        data: [length as u32, 12, 0, 0, 0].into() })
}

fn invalid_reply(kind: ReplyKind, text: &[u8]) {
    let wire = xim_parser::write_to_vec(reply(kind, text));
    let mut client = client(wire.clone());
    let mut handler = Handler::default();
    assert!(matches!(client.filter_event(&event(wire.len()), &mut handler), Err(ClientError::InvalidReply)));
    assert!(handler.delivered.is_empty());
    assert!(client.has_conn.sent.borrow().is_empty());
    assert!(client.im_attributes.is_empty() && client.ic_attributes.is_empty());
    assert_eq!(client.has_conn.property, wire);
}

#[test]
fn reset_invalid_utf8_returns_error() { invalid_reply(ReplyKind::Reset, &[0x1b, 0x25, 0x47, 0xff]); }
#[test]
fn commit_invalid_utf8_returns_error() { invalid_reply(ReplyKind::Commit, &[0x1b, 0x25, 0x47, 0xff]); }
#[test]
fn preedit_invalid_utf8_returns_error() { invalid_reply(ReplyKind::Preedit, &[0x1b, 0x25, 0x47, 0xff]); }
#[test]
fn reset_invalid_charset_returns_error() { invalid_reply(ReplyKind::Reset, &[0x1b, 0x24, 0x28, 0x5a]); }
#[test]
fn commit_invalid_charset_returns_error() { invalid_reply(ReplyKind::Commit, &[0x1b, 0x24, 0x28, 0x5a]); }
#[test]
fn preedit_invalid_charset_returns_error() { invalid_reply(ReplyKind::Preedit, &[0x1b, 0x24, 0x28, 0x5a]); }
#[test]
fn chinese_high_byte_returns_error() { invalid_reply(ReplyKind::Reset, &[0x1b, 0x24, 0x28, 0x41, 0xff]); }
#[test]
fn korean_high_byte_returns_error() { invalid_reply(ReplyKind::Reset, &[0x1b, 0x24, 0x28, 0x43, 0xff]); }

fn valid_reply(kind: ReplyKind) {
    for (wire_text, expected) in [
        (Vec::new(), ""),
        (xim_ctext::utf8_to_compound_text("文😀 café"), "文😀 café"),
        (vec![0x1b, 0x2d, 0x41, 0xe9], "é"),
        (vec![0x1b, 0x24, 0x28, 0x41, 0x44, 0x63], "你"),
        (vec![0x1b, 0x24, 0x28, 0x43, 0x33, 0x4d], "넌"),
    ] {
        let wire = xim_parser::write_to_vec(reply(kind, &wire_text));
        let mut client = client(wire.clone());
        let mut handler = Handler::default();
        assert!(client.filter_event(&event(wire.len()), &mut handler).unwrap());
        assert_eq!(handler.delivered, vec![(7, 11, expected.into())]);
        assert!(client.has_conn.sent.borrow().is_empty());
    }
}

#[test]
fn valid_reset_compound_text_is_unchanged() { valid_reply(ReplyKind::Reset); }
#[test]
fn valid_commit_compound_text_is_unchanged() { valid_reply(ReplyKind::Commit); }
#[test]
fn valid_preedit_compound_text_is_unchanged() { valid_reply(ReplyKind::Preedit); }

fn japanese_ascii_reply(kind: ReplyKind) {
    for (text, expected) in [
        (&b"\x1b$(BEl5~\x1b(B, (B)!"[..], "東京, (B)!"),
        (&b"\x1b(B\x1b(Btail"[..], "tail"),
        (&b"(B literal"[..], "(B literal"),
        (&b"%G literal -A $"[..], "%G literal -A $"),
        (&b"\x1b-B\xa1\x1b(B tail!"[..], "Ą tail!"),
        (&b"\x1b-B\xa1\x1b(B\x1b-A\xa1"[..], "Ą¡"),
    ] {
        let wire = xim_parser::write_to_vec(reply(kind, text));
        let mut client = client(wire.clone());
        let mut handler = Handler::default();
        assert!(client.filter_event(&event(wire.len()), &mut handler).unwrap());
        assert_eq!(handler.delivered, vec![(7, 11, expected.into())]);
        assert!(client.has_conn.sent.borrow().is_empty());
    }
}

#[test]
fn reset_consumes_japanese_ascii_reset() { japanese_ascii_reply(ReplyKind::Reset); }
#[test]
fn commit_consumes_japanese_ascii_reset() { japanese_ascii_reply(ReplyKind::Commit); }
#[test]
fn preedit_consumes_japanese_ascii_reset() { japanese_ascii_reply(ReplyKind::Preedit); }

fn invalid_ascii_reply(kind: ReplyKind) {
    for text in [&b"\x1b"[..], &b"\x1b("[..], &b"\x1b(A"[..], &b"\x1b(0"[..],
        &b"\x1b(I"[..], &b"\x1b(J"[..], &b"\x1bZ"[..],
        &b"\x1b-B\xa1\x1b(B\xa1"[..], &b"\x1b-B\xa1\x1b(B\x1b(B\xa1"[..]] {
        invalid_reply(kind, text);
    }
}

#[test]
fn reset_rejects_invalid_ascii_designation() { invalid_ascii_reply(ReplyKind::Reset); }
#[test]
fn commit_rejects_invalid_ascii_designation() { invalid_ascii_reply(ReplyKind::Commit); }
#[test]
fn preedit_rejects_invalid_ascii_designation() { invalid_ascii_reply(ReplyKind::Preedit); }

#[test]
fn reset_refuses_unsupported_prior_gr_without_callback() {
    invalid_reply(ReplyKind::Reset, b"\x1b-B\xa1\x1b(B\xa1");
}
#[test]
fn commit_refuses_unsupported_prior_gr_without_callback() {
    invalid_reply(ReplyKind::Commit, b"\x1b-B\xa1\x1b(B\xa1");
}
#[test]
fn preedit_refuses_unsupported_prior_gr_without_callback() {
    invalid_reply(ReplyKind::Preedit, b"\x1b-B\xa1\x1b(B\xa1");
}

#[test]
fn reset_handler_error_is_propagated() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Reset, &xim_ctext::utf8_to_compound_text("文")));
    let mut client = client(wire.clone());
    let mut handler = Handler { reject_reset: true, ..Default::default() };
    assert!(matches!(client.filter_event(&event(wire.len()), &mut handler), Err(ClientError::UnsupportedTransport)));
    assert!(handler.delivered.is_empty());
}

#[test]
fn truncated_wire_returns_protocol_error() {
    let mut client = client(vec![65, 0]);
    let mut handler = Handler::default();
    assert!(matches!(client.filter_event(&event(2), &mut handler), Err(ClientError::InvalidReply)));
    assert!(handler.delivered.is_empty());
    assert!(client.has_conn.with_reply.borrow().is_empty());
}

mod commit_tests;
mod transport_tests;

mod multicm_tests;

mod property_tests;
