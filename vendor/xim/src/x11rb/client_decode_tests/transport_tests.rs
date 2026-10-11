use super::*;
use crate::client::Client;

fn xconnect(major: u32, minor: u32, max: u32) -> Event {
    Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 1,
        window: 10,
        type_: 8,
        data: [3, major, minor, max, 0].into(),
    })
}

fn key_press() -> KeyPressEvent {
    KeyPressEvent {
        response_type: x11rb::protocol::xproto::KEY_PRESS_EVENT,
        detail: 38,
        sequence: 2,
        time: 1,
        root: 1,
        event: 10,
        child: 0,
        root_x: 0,
        root_y: 0,
        event_x: 0,
        event_y: 0,
        state: 0u16.into(),
        same_screen: true,
    }
}

#[test]
fn unsupported_02_is_rejected_before_forwarding_a_key() {
    let mut client = client(Vec::new());
    let mut handler = Handler::default();
    assert!(matches!(
        client.filter_event(&xconnect(0, 2, 64), &mut handler),
        Err(ClientError::UnsupportedTransport)
    ));
    assert_eq!(client.transport_max, 20);
    assert_eq!(client.im_window, 3);
    assert!(client.has_conn.sent.borrow().is_empty());
    assert!(client.has_conn.with_reply.borrow().is_empty());
}

fn assert_rejected_without_mutation(mut event: Event, invalid_format: Option<u8>) {
    if let (Event::ClientMessage(message), Some(format)) = (&mut event, invalid_format) {
        message.format = format;
    }
    let mut client = client(Vec::new());
    let mut handler = Handler::default();
    let result = client.filter_event(&event, &mut handler);
    if invalid_format.is_some() {
        assert!(matches!(result, Err(ClientError::InvalidReply)));
    } else {
        assert!(matches!(result, Err(ClientError::UnsupportedTransport)));
    }
    assert_eq!(client.im_window, 3);
    assert_eq!(client.transport_max, 20);
    assert_eq!(client.sequence, 0);
    assert!(client.buf.is_empty());
    assert!(client.has_conn.sent.borrow().is_empty());
    assert!(client.has_conn.with_reply.borrow().is_empty());
    assert_eq!(*client.has_conn.send_attempts.borrow(), 0);
    assert!(handler.delivered.is_empty());
    assert_eq!(handler.commit_calls, 0);
}

#[test]
fn unsupported_transport_versions_preserve_prior_state() {
    for (major, minor) in [(0, 1), (0, 2), (1, 0), (2, 0), (2, 1), (9, 9)] {
        let mut event = xconnect(major, minor, 64);
        if let Event::ClientMessage(message) = &mut event {
            message.data = [30, major, minor, 64, 0].into();
        }
        assert_rejected_without_mutation(event, None);
    }
}

#[test]
fn xconnect_requires_format32_before_state_mutation() {
    for format in [0, 8, 16] {
        assert_rejected_without_mutation(xconnect(0, 0, 64), Some(format));
    }
}

fn connected(max: u32) -> X11rbClient<WireConnection> {
    let mut client = client(Vec::new());
    assert!(client
        .filter_event(&xconnect(0, 0, max), &mut Handler::default())
        .unwrap());
    client.has_conn.sent.borrow_mut().clear();
    client.has_conn.with_reply.borrow_mut().clear();
    client.has_conn.send_attempts.replace(0);
    client
}

fn assert_property_send(client: &X11rbClient<WireConnection>, expected: Request, len: usize) {
    let replies = client.has_conn.with_reply.borrow();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].first().copied(), Some(16)); // InternAtom.
    assert_eq!(replies[0].get(8..19).unwrap(), b"_XIM_DATA_0");
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 2);
    let property = &sent[0];
    assert_eq!(property.first().copied(), Some(18)); // ChangeProperty.
    assert_eq!(property.get(1).copied(), Some(2)); // Append.
    assert_eq!(property.get(4..8).unwrap(), 3u32.to_ne_bytes());
    assert_eq!(property.get(8..12).unwrap(), 42u32.to_ne_bytes());
    assert_eq!(
        property.get(12..16).unwrap(),
        u32::from(AtomEnum::STRING).to_ne_bytes()
    );
    assert_eq!(property.get(16).copied(), Some(8));
    assert_eq!(property.get(20..24).unwrap(), (len as u32).to_ne_bytes());
    assert_eq!(property.len(), 24 + len);
    assert_eq!(
        xim_parser::read::<Request>(property.get(24..).unwrap()).unwrap(),
        expected
    );
    let notify = &sent[1];
    assert_eq!(notify.first().copied(), Some(25)); // SendEvent.
    let (message, _) = ClientMessageEvent::try_parse(notify.get(12..).unwrap()).unwrap();
    assert_eq!(message.window, 3);
    assert_eq!(message.type_, 9);
    assert_eq!(message.format, 32);
    assert_eq!(message.data.as_data32(), [len as u32, 42, 0, 0, 0]);
    assert_eq!(*client.has_conn.send_attempts.borrow(), 2);
    assert!(client.buf.is_empty());
}

#[test]
fn supported_00_caps_threshold_and_forwards_real_key_via_property() {
    let mut client = connected(64);
    assert_eq!(client.transport_max, 20);
    let event = key_press();
    let xev = client.serialize_event(&event);
    client
        .forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &event)
        .unwrap();
    assert_property_send(
        &client,
        Request::ForwardEvent {
            input_method_id: 7,
            input_context_id: 11,
            flag: xim_parser::ForwardEventFlag::empty(),
            serial_number: 2,
            xev,
        },
        44,
    );
}

#[test]
fn supported_00_small_packet_uses_one_zero_padded_client_message() {
    let mut client = connected(20);
    client.close(7).unwrap();
    assert!(client.has_conn.with_reply.borrow().is_empty());
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 1);
    let (message, _) = ClientMessageEvent::try_parse(sent[0].get(12..).unwrap()).unwrap();
    assert_eq!(message.format, 8);
    assert_eq!(message.window, 3);
    assert_eq!(message.type_, 9);
    let bytes = message.data.as_data8();
    assert_eq!(
        xim_parser::read::<Request>(&bytes).unwrap(),
        Request::Close { input_method_id: 7 }
    );
    assert_eq!(bytes.get(8..).unwrap(), &[0; 12]);
}

#[test]
fn supported_00_twenty_byte_boundary_uses_property() {
    let mut client = connected(20);
    let locale = "abcdefghijklmno";
    client.open(locale).unwrap();
    assert_property_send(
        &client,
        Request::Open {
            locale: locale.into(),
        },
        20,
    );
}

#[test]
fn zero_threshold_keeps_supported_property_route() {
    let mut client = connected(0);
    assert_eq!(client.transport_max, 0);
    client.close(7).unwrap();
    // Connect already consumed property sequence0.
    let replies = client.has_conn.with_reply.borrow();
    assert_eq!(replies[0].get(8..19).unwrap(), b"_XIM_DATA_1");
    drop(replies);
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        xim_parser::read::<Request>(sent[0].get(24..).unwrap()).unwrap(),
        Request::Close { input_method_id: 7 }
    );
}

#[test]
fn maximum_advertised_threshold_is_bounded_before_sending() {
    let client = connected(u32::MAX);
    assert_eq!(client.transport_max, 20);
}

#[test]
fn shared_sender_returns_typed_error_for_unimplemented_multi_cm() {
    let mut client = client(Vec::new());
    let request = Request::Open {
        locale: "abcdefghijklmnopqrs".into(),
    }; //24 bytes.
    let result: Result<(), ClientError> = send_req_impl(
        &client.has_conn,
        &client.atoms,
        client.im_window,
        &mut client.buf,
        &mut client.sequence,
        64,
        &request,
    );
    assert!(matches!(result, Err(ClientError::Other(_))));
    assert!(client.has_conn.sent.borrow().is_empty());
    assert!(client.has_conn.with_reply.borrow().is_empty());
}

#[test]
fn shared_sender_accepts_exactly_twenty_byte_client_message() {
    let mut client = client(Vec::new());
    let request = Request::Open {
        locale: "abcdefghijklmno".into(),
    };
    let result: Result<(), ClientError> = send_req_impl(
        &client.has_conn,
        &client.atoms,
        client.im_window,
        &mut client.buf,
        &mut client.sequence,
        21,
        &request,
    );
    result.unwrap();
    assert!(client.has_conn.with_reply.borrow().is_empty());
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 1);
    let (message, _) = ClientMessageEvent::try_parse(sent[0].get(12..).unwrap()).unwrap();
    assert_eq!(message.format, 8);
    assert_eq!(
        xim_parser::read::<Request>(&message.data.as_data8()).unwrap(),
        request
    );
}

#[test]
fn property_send_failure_propagates_before_notification() {
    let mut client = connected(64);
    client.has_conn.reject_send = true;
    assert!(matches!(
        client.forward_event(7, 11, xim_parser::ForwardEventFlag::empty(), &key_press()),
        Err(ClientError::Other(_))
    ));
    assert!(client.has_conn.sent.borrow().is_empty());
    assert_eq!(*client.has_conn.send_attempts.borrow(), 1);
    assert_eq!(client.has_conn.with_reply.borrow().len(), 1);
}

#[test]
fn small_message_send_failure_propagates() {
    let mut client = connected(20);
    client.has_conn.reject_send = true;
    assert!(matches!(client.close(7), Err(ClientError::Other(_))));
    assert!(client.has_conn.sent.borrow().is_empty());
    assert_eq!(*client.has_conn.send_attempts.borrow(), 1);
    assert!(client.has_conn.with_reply.borrow().is_empty());
}

#[test]
fn connect_send_failure_propagates() {
    let mut client = client(Vec::new());
    client.has_conn.reject_send = true;
    assert!(matches!(
        client.filter_event(&xconnect(0, 0, 64), &mut Handler::default()),
        Err(ClientError::Other(_))
    ));
    assert!(client.has_conn.sent.borrow().is_empty());
    assert_eq!(*client.has_conn.send_attempts.borrow(), 1);
}
