use super::*;
use crate::client::Client;

const MORE_DATA: u32 = 42;

fn connected01() -> X11rbClient<WireConnection> {
    let mut client = client(Vec::new());
    let result = client.filter_event(&xconnect(1, 64), &mut Handler::default());
    assert!(matches!(result, Ok(true)), "0.1 negotiation: {:?}", result);
    client.has_conn.sent.borrow_mut().clear();
    client.has_conn.with_reply.borrow_mut().clear();
    client.has_conn.send_attempts.replace(0);
    client
}

fn xconnect(minor: u32, dividing_size: u32) -> Event {
    Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 1,
        window: 10, type_: 8, data: [3, 0, minor, dividing_size, 0].into(),
    })
}

fn chunk(atom: u32, bytes: &[u8]) -> Event {
    assert!(bytes.len() <= 20);
    let mut data = [0; 20];
    data[..bytes.len()].copy_from_slice(bytes);
    Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 8, sequence: 1,
        window: 10, type_: atom, data: data.into(),
    })
}

fn chunks(wire: &[u8]) -> Vec<Event> {
    let count = wire.chunks(20).count();
    wire.chunks(20).enumerate().map(|(index, bytes)| {
        chunk(if index + 1 == count { 9 } else { MORE_DATA }, bytes)
    }).collect()
}

fn key_press() -> KeyPressEvent {
    KeyPressEvent {
        response_type: x11rb::protocol::xproto::KEY_PRESS_EVENT,
        detail: 38, sequence: 2, time: 1, root: 1, event: 10, child: 0,
        root_x: 0, root_y: 0, event_x: 0, event_y: 0,
        state: 0u16.into(), same_screen: true,
    }
}

fn outgoing_multi_cm(minor: u32) {
    let mut client = client(Vec::new());
    let negotiated = client.filter_event(&xconnect(minor, 64), &mut Handler::default());
    assert!(matches!(negotiated, Ok(true)), "0.{} negotiation: {:?}", minor, negotiated);
    client.has_conn.sent.borrow_mut().clear();
    client.has_conn.with_reply.borrow_mut().clear();
    let key = key_press();
    let expected = xim_parser::write_to_vec(Request::ForwardEvent {
        input_method_id: 7, input_context_id: 11,
        flag: xim_parser::ForwardEventFlag::empty(), serial_number: 2,
        xev: client.serialize_event(&key),
    });
    assert_eq!(expected.len(), 44);
    client.forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &key).unwrap();
    assert!(client.has_conn.with_reply.borrow().is_empty());
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 3);
    for (index, (raw, part)) in sent.iter().zip(expected.chunks(20)).enumerate() {
        assert_eq!(raw.first(), Some(&25)); // SendEvent, never ChangeProperty.
        let (message, _) = ClientMessageEvent::try_parse(&raw[12..]).unwrap();
        assert_eq!(message.window, 3);
        assert_eq!(message.format, 8);
        assert_eq!(message.type_, if index == 2 { 9 } else { MORE_DATA });
        let data = message.data.as_data8();
        assert_eq!(&data[..part.len()], part);
        assert!(data[part.len()..].iter().all(|byte| *byte == 0));
    }
}

#[test]
fn multicm_design_negotiated_01_forwards_real_key_as_three_ordered_chunks() {
    outgoing_multi_cm(1);
}

#[test]
fn multicm_design_more_data_is_consumed_without_delivering_partial_text() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"abcdefghijklmnopqrstuvwx"));
    assert_eq!(wire.len(), 36);
    let mut client = connected01();
    let mut handler = Handler::default();
    let result = client.filter_event(&chunk(MORE_DATA, &wire[..20]), &mut handler);
    assert!(matches!(result, Ok(true)), "first chunk: {:?}", result);
    assert!(handler.delivered.is_empty());
    assert!(client.has_conn.sent.borrow().is_empty());
}

fn receive_full_packet(kind: ReplyKind, text: &str) {
    let mut encoded = b"\x1b%G".to_vec();
    encoded.extend_from_slice(text.as_bytes());
    let request = reply(kind, &encoded);
    let wire = xim_parser::write_to_vec(&request);
    assert_eq!(xim_parser::read::<Request>(&wire).unwrap(), request);
    assert!(wire.len() > 40);
    let messages = chunks(&wire);
    let mut client = connected01();
    let mut handler = Handler::default();
    for message in &messages[..messages.len() - 1] {
        let result = client.filter_event(message, &mut handler);
        assert!(matches!(result, Ok(true)), "intermediate chunk: {:?}", result);
        assert!(handler.delivered.is_empty());
    }
    let final_message = messages.last().unwrap();
    let result = client.filter_event(final_message, &mut handler);
    assert!(matches!(result, Ok(true)), "final chunk: {:?}", result);
    assert_eq!(handler.delivered, vec![(7, 11, text.into())]);
    assert!(client.has_conn.sent.borrow().is_empty());
}

#[test]
fn multicm_design_commit_is_delivered_once_after_final_chunk() {
    receive_full_packet(ReplyKind::Commit, "abcdefghijklmnop 😀 中 rstuvwxyz123456789");
}

#[test]
fn multicm_design_reset_is_delivered_once_after_final_chunk() {
    receive_full_packet(ReplyKind::Reset, "abcdefghijklmnop 😀 中 rstuvwxyz123456789");
}

#[test]
fn multicm_design_preedit_is_delivered_once_after_final_chunk() {
    receive_full_packet(ReplyKind::Preedit, "abcdefghijklmnop 😀 中 rstuvwxyz123456789");
}

#[test]
fn multicm_design_more_data_requires_format8() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"abcdefghijklmnopqrstuvwx"));
    let mut event = chunk(MORE_DATA, &wire[..20]);
    if let Event::ClientMessage(message) = &mut event { message.format = 32; }
    let mut client = connected01();
    let mut handler = Handler::default();
    assert!(matches!(client.filter_event(&event, &mut handler), Err(ClientError::InvalidReply)));
    assert!(handler.delivered.is_empty());
}

#[test]
fn multicm_design_nonfinal_chunk_cannot_complete_declared_packet() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"12345678"));
    assert_eq!(wire.len(), 20);
    let mut client = connected01();
    let mut handler = Handler::default();
    assert!(matches!(client.filter_event(&chunk(MORE_DATA, &wire), &mut handler), Err(ClientError::InvalidReply)));
    assert!(handler.delivered.is_empty());
}

#[test]
fn multicm_design_early_final_does_not_publish_an_embedded_fresh_request() {
    let original = xim_parser::write_to_vec(reply(ReplyKind::Commit,
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890"));
    assert!(original.len() > 40);
    let unrelated = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"new"));
    assert!(unrelated.len() <= 20);
    let mut client = connected01();
    let mut handler = Handler::default();
    assert!(client.filter_event(&chunk(MORE_DATA, &original[..20]), &mut handler).is_ok());
    assert!(handler.delivered.is_empty());
    let result = client.filter_event(&chunk(9, &unrelated), &mut handler);
    assert!(matches!(result, Err(ClientError::InvalidReply)),
        "early final result {:?}; published {:?}", result, handler.delivered);
    assert!(handler.delivered.is_empty());
}

#[test]
fn multicm_design_single_cm_control_preserves_current_text_delivery() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    assert_eq!(wire.len(), 20);
    let mut client = connected01();
    let mut handler = Handler::default();
    assert!(client.filter_event(&chunk(9, &wire), &mut handler).unwrap());
    assert_eq!(handler.delivered, vec![(7, 11, "hello".into())]);
}

#[test]
fn multicm_design_00_control_keeps_real_property_forwarding() {
    let mut client = client(Vec::new());
    assert!(client.filter_event(&xconnect(0, 64), &mut Handler::default()).unwrap());
    client.has_conn.sent.borrow_mut().clear();
    client.has_conn.with_reply.borrow_mut().clear();
    client.forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &key_press()).unwrap();
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].first(), Some(&18)); // ChangeProperty.
    assert_eq!(sent[1].first(), Some(&25)); // SendEvent.
    let (message, _) = ClientMessageEvent::try_parse(&sent[1][12..]).unwrap();
    assert_eq!(message.format, 32);
    assert_eq!(message.data.as_data32(), [44, 42, 0, 0, 0]);
}

#[test]
fn multicm_design_unrelated_window_control_remains_ignored() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"abcdefghijklmnopqrstuvwx"));
    let mut event = chunk(MORE_DATA, &wire[..20]);
    if let Event::ClientMessage(message) = &mut event { message.window = 99; }
    let mut client = client(Vec::new());
    let mut handler = Handler::default();
    assert!(!client.filter_event(&event, &mut handler).unwrap());
    assert!(handler.delivered.is_empty());
}

#[test]
fn exact_20_and_40_byte_outgoing_packets_have_no_empty_final() {
    for text_len in [8, 28] {
        let mut client = connected01();
        let request = reply(ReplyKind::Commit, &vec![b'a'; text_len]);
        let size = request.size();
        assert!(size == 20 || size == 40);
        client.send_req(request).unwrap();
        let sent = client.has_conn.sent.borrow();
        assert_eq!(sent.len(), size / 20);
        let (last, _) = ClientMessageEvent::try_parse(&sent.last().unwrap()[12..]).unwrap();
        assert_eq!(last.type_, 9);
    }
}

#[test]
fn middle_final_and_flush_errors_stop_outgoing_and_propagate() {
    for failure in [Some(2), Some(3), None] {
        let mut client = connected01();
        client.has_conn.reject_send_at = failure;
        client.has_conn.reject_flush = failure.is_none();
        let start_flush = *client.has_conn.flush_calls.borrow();
        assert!(client.forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &key_press()).is_err());
        assert_eq!(*client.has_conn.send_attempts.borrow(), failure.unwrap_or(3));
        assert_eq!(client.has_conn.sent.borrow().len(), failure.map_or(3, |n| n - 1));
        assert_eq!(*client.has_conn.flush_calls.borrow() - start_flush, usize::from(failure.is_none()));
    }
}

#[test]
fn synchronous_fragmented_commit_calls_handler_before_one_ack() {
    let mut client = connected01();
    let wire = xim_parser::write_to_vec(Request::Commit {
        input_method_id: 7, input_context_id: 11,
        data: xim_parser::CommitData::Chars { commited: vec![b'a'; 45], syncronous: true },
    });
    let mut handler = Handler::default();
    for message in chunks(&wire) { assert!(client.filter_event(&message, &mut handler).unwrap()); }
    assert_eq!(handler.commit_calls, 1);
    assert!(!handler.ack_seen_during_commit);
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 1);
    let (ack, _) = ClientMessageEvent::try_parse(&sent[0][12..]).unwrap();
    assert_eq!(xim_parser::read::<Request>(&ack.data.as_data8()).unwrap(), Request::SyncReply {
        input_method_id: 7, input_context_id: 11,
    });
}

fn start_pending(client: &mut X11rbClient<WireConnection>, handler: &mut Handler) {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, &[b'a'; 60]));
    assert!(client.filter_event(&chunk(MORE_DATA, &wire[..20]), handler).unwrap());
    assert_eq!(client.transport.retained().0, 20);
}

#[test]
fn wrong_format_and_padding_clear_pending_and_next_valid_packet_works() {
    for format in [0, 16, 32] {
        let mut client = connected01();
        let mut handler = Handler::default();
        start_pending(&mut client, &mut handler);
        let mut event = chunk(9, &[0; 20]);
        if let Event::ClientMessage(msg) = &mut event { msg.format = format; }
        assert!(matches!(client.filter_event(&event, &mut handler), Err(ClientError::InvalidReply)));
        assert_eq!(client.transport.retained(), (0, 0));
        let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"ok"));
        assert!(client.filter_event(&chunk(9, &wire), &mut handler).unwrap());
        assert_eq!(handler.delivered, vec![(7, 11, "ok".into())]);
    }
    let mut client = connected01();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"ok"));
    let mut bad = wire.clone();
    bad.resize(20, 1);
    assert!(client.filter_event(&chunk(9, &bad), &mut handler).is_err());
    assert!(handler.delivered.is_empty());
    assert_eq!(client.transport.retained(), (0, 0));
}

#[test]
fn unrelated_events_and_independent_outgoing_preserve_incoming() {
    let mut client = connected01();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, &[b'a'; 60]));
    assert!(client.filter_event(&chunk(MORE_DATA, &wire[..20]), &mut handler).unwrap());
    let retained = client.transport.retained();
    for (window, atom) in [(99, MORE_DATA), (10, 123)] {
        let mut event = chunk(atom, &[0; 20]);
        if let Event::ClientMessage(msg) = &mut event { msg.window = window; }
        assert!(!client.filter_event(&event, &mut handler).unwrap());
        assert_eq!(client.transport.retained(), retained);
    }
    client.forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &key_press()).unwrap();
    assert_eq!(client.transport.retained(), retained);
    for event in chunks(&wire).into_iter().skip(1) { assert!(client.filter_event(&event, &mut handler).unwrap()); }
    assert_eq!(handler.delivered, vec![(7, 11, "a".repeat(60))]);
}

#[test]
fn valid_reconnect_clears_partial_for_both_supported_modes() {
    for minor in [0, 1] {
        let mut client = connected01();
        let mut handler = Handler::default();
        start_pending(&mut client, &mut handler);
        assert!(client.filter_event(&xconnect(minor, 20), &mut handler).unwrap());
        assert_eq!(client.transport.retained(), (0, 0));
        assert_eq!(client.transport.more_data.is_some(), minor == 1);
    }
}

#[test]
fn parser_and_callback_errors_do_not_revive_consumed_bytes() {
    for reject_handler in [false, true] {
        let mut client = connected01();
        let mut handler = Handler { reject_commit: reject_handler, ..Default::default() };
        let mut wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, &[b'a'; 60]));
        if !reject_handler { wire[0] = 255; }
        let events = chunks(&wire);
        for event in &events[..events.len() - 1] { assert!(client.filter_event(event, &mut handler).unwrap()); }
        assert!(client.filter_event(events.last().unwrap(), &mut handler).is_err());
        assert_eq!(client.transport.retained(), (0, 0));
        assert!(handler.delivered.is_empty());
        handler.reject_commit = false;
        let valid = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"fresh"));
        assert!(client.filter_event(&chunk(9, &valid), &mut handler).unwrap());
        assert_eq!(handler.delivered, vec![(7, 11, "fresh".into())]);
    }
}

#[test]
fn maximum_header_bounds_retained_bytes_and_refuses_extra_nonfinal() {
    let mut client = connected01();
    let mut handler = Handler::default();
    let mut data = [0; 20];
    data[0] = 255;
    data[2..4].copy_from_slice(&u16::MAX.to_ne_bytes());
    assert!(client.filter_event(&chunk(MORE_DATA, &data), &mut handler).unwrap());
    let max = client_transport::MAX_PACKET;
    assert!(client.transport.retained().1 <= max);
    while client.transport.retained().0 + 20 < max {
        assert!(client.filter_event(&chunk(MORE_DATA, &[0; 20]), &mut handler).unwrap());
        assert!(client.transport.retained().1 <= max);
    }
    assert_eq!(client.transport.retained().0, 262140);
    assert!(client.filter_event(&chunk(MORE_DATA, &[0; 20]), &mut handler).is_err());
    assert_eq!(client.transport.retained(), (0, 0));
    assert!(handler.delivered.is_empty());
}

#[test]
fn oversized_outgoing_is_refused_before_wire_send() {
    let mut client = connected01();
    let request = Request::PreeditDraw {
        input_method_id: 7, input_context_id: 11, caret: 0, chg_first: 0, chg_length: 0,
        status: xim_parser::PreeditDrawStatus::empty(), preedit_string: Vec::new(),
        feedbacks: vec![xim_parser::Feedback::REVERSE; 65535],
    };
    assert!(request.size() > client_transport::MAX_PACKET);
    assert!(matches!(client.send_req(request), Err(ClientError::InvalidReply)));
    assert!(client.has_conn.sent.borrow().is_empty());
}

#[test]
fn disconnected_and_failed_reconnect_retire_pending_input() {
    let mut client = connected01();
    let mut handler = Handler::default();
    start_pending(&mut client, &mut handler);
    assert!(client.filter_event(&xconnect(2, 64), &mut handler).is_err());
    assert_eq!(client.transport.retained(), (0, 0));
    let wire = xim_parser::write_to_vec(Request::DisconnectReply {});
    assert!(client.filter_event(&chunk(9, &wire), &mut handler).unwrap());
    assert_eq!(client.transport.retained(), (0, 0));
}

#[test]
fn failed_outgoing_releases_partial_receive_without_publication() {
    let mut client = connected01();
    let mut handler = Handler::default();
    start_pending(&mut client, &mut handler);
    client.has_conn.reject_send_at = Some(2);
    assert!(client.forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &key_press()).is_err());
    assert_eq!(client.transport.retained(), (0, 0));
    assert!(client.buf.is_empty());
    assert!(handler.delivered.is_empty());
}

fn maximum_packet() -> Vec<u8> {
    let mut wire = vec![0; client_transport::MAX_PACKET];
    wire[0] = 255; // Valid maximum transport frame; deliberately unknown protocol opcode.
    wire[2..4].copy_from_slice(&u16::MAX.to_ne_bytes());
    wire
}

#[test]
fn maximum_completed_frame_reaches_parser_once_then_releases_storage() {
    let mut client = connected01();
    let mut handler = Handler::default();
    let wire = maximum_packet();
    let events = chunks(&wire);
    for event in &events[..events.len() - 1] { assert!(client.filter_event(event, &mut handler).unwrap()); }
    assert!(matches!(client.filter_event(events.last().unwrap(), &mut handler), Err(ClientError::ReadProtocol(_))));
    assert_eq!(client.transport.retained(), (0, 0));
    assert!(handler.delivered.is_empty());
}

#[test]
#[ignore = "release transport timing; set NOTEPAD_STUDIO_XIM_CM_BENCH"]
fn bounded_maximum_multicm_release_timing() {
    if std::env::var_os("NOTEPAD_STUDIO_XIM_CM_BENCH").is_none() { return; }
    let wire = maximum_packet();
    let events = chunks(&wire);
    let mut client = connected01();
    let mut handler = Handler::default();
    for sample in 0..12 {
        let start = std::time::Instant::now();
        for event in &events[..events.len() - 1] { assert!(client.filter_event(event, &mut handler).unwrap()); }
        assert!(matches!(client.filter_event(events.last().unwrap(), &mut handler), Err(ClientError::ReadProtocol(_))));
        std::println!("sample={} bytes={} events={} elapsed_us={:.3}", sample, wire.len(), events.len(), start.elapsed().as_secs_f64() * 1_000_000.0);
        assert_eq!(client.transport.retained(), (0, 0));
    }
    assert!(handler.delivered.is_empty());
}
