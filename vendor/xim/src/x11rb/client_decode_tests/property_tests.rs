use super::*;

#[test]
fn property_design_announced_byte_count_becomes_four_byte_units() {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"abcdefghijklmnopqrstuvwx"));
    assert_eq!(wire.len(), 36);
    let mut client = client(wire.clone());
    assert!(client.filter_event(&event(wire.len()), &mut Handler::default()).unwrap());
    let requests = client.has_conn.with_reply.borrow();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0][0], 20); // Actual GetProperty request opcode.
    let units = u32::from_ne_bytes(requests[0][20..24].try_into().unwrap());
    assert_eq!(units, 9, "GetProperty length uses units of four bytes");
}

#[test]
fn property_design_oversized_notification_is_refused_before_fetch() {
    let mut client = client(Vec::new());
    let mut handler = Handler::default();
    assert!(matches!(client.filter_event(&event(262148), &mut handler), Err(ClientError::InvalidReply)));
    assert!(client.has_conn.with_reply.borrow().is_empty(), "hostile length must be bounded before GetProperty");
    assert!(handler.delivered.is_empty());
}

fn dynamic_client() -> X11rbClient<WireConnection> {
    let mut client = client(Vec::new());
    client.has_conn.dynamic_property = true;
    client
}

#[test]
fn property_design_fixture_honors_four_byte_offsets_and_delete_control() {
    let client = dynamic_client();
    append(&client, 12, &[1; 20]);
    append(&client, 12, &[2; 20]);
    let first = client.conn().get_property(true, 10, 12u32, AtomEnum::ANY, 0, 5).unwrap().reply().unwrap();
    assert_eq!(first.value, vec![1; 20]);
    assert_eq!(first.bytes_after, 20);
    assert!(client.has_conn.deleted_properties.borrow().is_empty());
    let last = client.conn().get_property(true, 10, 12u32, AtomEnum::ANY, 5, 5).unwrap().reply().unwrap();
    assert_eq!(last.value, vec![2; 20]);
    assert_eq!(last.bytes_after, 0);
    assert_eq!(*client.has_conn.deleted_properties.borrow(), vec![12]);
    assert!(client.has_conn.property_store.borrow().is_empty());
}

fn append(client: &X11rbClient<WireConnection>, atom: u32, bytes: &[u8]) {
    client.has_conn.property_store.borrow_mut().entry(atom).or_default().extend_from_slice(bytes);
}

fn property_event(length: usize, atom: u32) -> Event {
    let mut message = event(length);
    if let Event::ClientMessage(msg) = &mut message { msg.data = [length as u32, atom, 0, 0, 0].into(); }
    message
}

#[test]
fn property_design_two_appended_packets_deliver_in_order_then_delete() {
    let mut client = dynamic_client();
    let first = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"first123"));
    let second = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"second12"));
    assert_eq!(first.len(), 20);
    assert_eq!(second.len(), 20);
    append(&client, 12, &first);
    append(&client, 12, &second);
    let mut handler = Handler::default();
    assert!(client.filter_event(&property_event(20, 12), &mut handler).unwrap());
    assert!(client.has_conn.deleted_properties.borrow().is_empty(), "the first bounded read must leave the appended second packet");
    assert!(client.filter_event(&property_event(20, 12), &mut handler).unwrap());
    assert_eq!(handler.delivered, vec![(7, 11, "first123".into()), (7, 11, "second12".into())]);
    assert_eq!(*client.has_conn.deleted_properties.borrow(), vec![12]);
    let requests = client.has_conn.with_reply.borrow();
    let offsets: Vec<_> = requests.iter().map(|req| u32::from_ne_bytes(req[16..20].try_into().unwrap())).collect();
    assert_eq!(offsets, vec![0, 5]);
}

#[test]
fn property_design_independent_atoms_and_reused_deleted_atom_control() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    append(&client, 12, &wire);
    append(&client, 13, &wire);
    for atom in [12, 13] { assert!(client.filter_event(&property_event(wire.len(), atom), &mut handler).unwrap()); }
    append(&client, 12, &wire);
    assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).unwrap());
    assert_eq!(handler.delivered.len(), 3);
    assert_eq!(*client.has_conn.deleted_properties.borrow(), vec![12, 13, 12]);
    assert!(client.has_conn.property_store.borrow().is_empty());
}

#[test]
fn property_design_normal_sustained_atom_stream_resets_after_deletion() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let text = vec![b'a'; 64000];
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, &text));
    let mut historical_bytes = 0usize;
    for _ in 0..1100 {
        append(&client, 12, &wire);
        assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).unwrap());
        assert_eq!(handler.delivered.len(), 1);
        handler.delivered.clear();
        assert!(client.has_conn.property_store.borrow().is_empty());
        historical_bytes += wire.len();
    }
    assert!(historical_bytes > 64 * 1024 * 1024);
    assert_eq!(handler.commit_calls, 1100);
    assert_eq!(client.has_conn.deleted_properties.borrow().len(), 1100);
}

fn invalid_framing(invalid_header: bool) {
    {
        let mut client = dynamic_client();
        let mut handler = Handler::default();
        let mut wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
        if invalid_header { wire[2..4].copy_from_slice(&0u16.to_ne_bytes()); }
        let announced = if invalid_header { wire.len() } else { wire.len() - 1 };
        append(&client, 12, &wire);
        assert!(matches!(client.filter_event(&property_event(announced, 12), &mut handler), Err(ClientError::InvalidReply)));
        assert!(handler.delivered.is_empty());
    }
}

fn invalid_metadata(metadata: (u8, u32, u32)) {
    {
        let mut client = dynamic_client();
        client.has_conn.property_metadata = Some(metadata);
        let mut handler = Handler::default();
        let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
        append(&client, 12, &wire);
        assert!(matches!(client.filter_event(&property_event(wire.len(), 12), &mut handler), Err(ClientError::InvalidReply)));
        assert!(handler.delivered.is_empty());
    }
}

#[test]
fn property_design_empty_and_truncated_reply_controls_refuse_publication() {
    let mut truncated = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    truncated.truncate(10);
    for bytes in [Vec::new(), truncated] {
        let mut client = dynamic_client();
        let mut handler = Handler::default();
        append(&client, 12, &bytes);
        assert!(client.filter_event(&property_event(20, 12), &mut handler).is_err());
        assert!(handler.delivered.is_empty());
    }
}

#[test]
fn property_design_unaligned_notification_is_refused() { invalid_framing(false); }
#[test]
fn property_design_header_length_mismatch_is_refused() { invalid_framing(true); }
#[test]
fn property_design_wrong_type_is_refused() { invalid_metadata((8, 1, 0)); }
#[test]
fn property_design_wrong_format_is_refused() { invalid_metadata((16, u32::from(AtomEnum::STRING), 0)); }
#[test]
fn property_design_unaligned_bytes_after_is_refused() { invalid_metadata((8, u32::from(AtomEnum::STRING), 1)); }
#[test]
fn property_design_oversized_pending_property_is_refused() { invalid_metadata((8, u32::from(AtomEnum::STRING), 67108868)); }

#[test]
fn property_design_many_deleted_atoms_do_not_consume_pending_map_capacity() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    for atom in 1000..2024 {
        append(&client, atom, &wire);
        assert!(client.filter_event(&property_event(wire.len(), atom), &mut handler).unwrap());
        handler.delivered.clear();
        assert!(client.has_conn.property_store.borrow().is_empty());
    }
    assert_eq!(handler.commit_calls, 1024);
}

fn sparse(client: &X11rbClient<WireConnection>, atom: u32, packet: &[u8], count: usize, suffix: &[u8]) {
    client.has_conn.sparse_properties.borrow_mut().insert(atom, SparseProperty { packet: packet.to_vec(), repeats: count, suffix: suffix.to_vec() });
}

fn fill_pending_atoms(client: &mut X11rbClient<WireConnection>, handler: &mut Handler) {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    for atom in 1000..1256 {
        sparse(client, atom, &wire, 5, &[]);
        assert!(client.filter_event(&property_event(wire.len(), atom), handler).unwrap());
        handler.delivered.clear();
    }
    assert_eq!(client.has_conn.sparse_properties.borrow().len(), 256);
}

#[test]
fn property_design_full_map_refuses_only_new_pending_tail_after_bounded_fetch() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    fill_pending_atoms(&mut client, &mut handler);
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    sparse(&client, 1256, &wire, 5, &[]);
    let before = client.has_conn.with_reply.borrow().len();
    let result = client.filter_event(&property_event(wire.len(), 1256), &mut handler);
    assert!(matches!(result, Err(ClientError::InvalidReply)));
    assert_eq!(client.has_conn.with_reply.borrow().len(), before + 1);
    assert!(handler.delivered.is_empty());
}

#[test]
fn property_design_full_map_still_accepts_new_fully_consumed_atom_control() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    fill_pending_atoms(&mut client, &mut handler);
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"fresh"));
    append(&client, 1256, &wire);
    let before = client.has_conn.with_reply.borrow().len();
    assert!(client.filter_event(&property_event(wire.len(), 1256), &mut handler).unwrap());
    assert_eq!(client.has_conn.with_reply.borrow().len(), before + 1);
    assert_eq!(handler.delivered, vec![(7, 11, "fresh".into())]);
}

#[test]
fn property_design_global_pending_plus_known_frame_is_bounded_before_fetch() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    let suffix = xim_parser::write_to_vec(Request::DisconnectReply {});
    assert_eq!(suffix.len(), 4);
    sparse(&client, 12, &wire, 3355443, &suffix); // 64MiB virtual property, tiny backing store.
    assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).unwrap());
    handler.delivered.clear();
    let next = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"123456789012"));
    assert_eq!(next.len(), 24);
    append(&client, 13, &next);
    let before = client.has_conn.with_reply.borrow().len();
    assert!(matches!(client.filter_event(&property_event(next.len(), 13), &mut handler), Err(ClientError::InvalidReply)));
    assert_eq!(client.has_conn.with_reply.borrow().len(), before);
    assert!(handler.delivered.is_empty());
}

#[test]
fn property_design_variable_packets_use_cumulative_four_byte_offsets() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let first = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    let second = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"123456789012"));
    let third = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"third123"));
    for packet in [&first, &second, &third] { append(&client, 12, packet); }
    for packet in [&first, &second, &third] {
        assert!(client.filter_event(&property_event(packet.len(), 12), &mut handler).unwrap());
    }
    assert_eq!(handler.delivered, vec![(7, 11, "hello".into()), (7, 11, "123456789012".into()), (7, 11, "third123".into())]);
    let requests = client.has_conn.with_reply.borrow();
    let offsets: Vec<_> = requests.iter().map(|req| u32::from_ne_bytes(req[16..20].try_into().unwrap())).collect();
    assert_eq!(offsets, vec![0, 5, 11]);
}

#[test]
fn property_design_known_opcode_with_header_announced_trailing_data_is_refused() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let mut wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    wire.extend_from_slice(&[0; 20]);
    wire[2..4].copy_from_slice(&9u16.to_ne_bytes());
    append(&client, 12, &wire);
    assert!(matches!(client.filter_event(&property_event(wire.len(), 12), &mut handler), Err(ClientError::InvalidReply)));
    assert!(handler.delivered.is_empty());
}

#[test]
fn property_design_malformed_reply_value_len_is_refused_before_dispatch() {
    let mut client = dynamic_client();
    client.has_conn.property_value_len = Some(16);
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    append(&client, 12, &wire);
    assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).is_err());
    assert!(handler.delivered.is_empty());
}

#[test]
fn property_design_reconnect_and_disconnect_reset_same_atom_offset_control() {
    for reconnect in [false, true] {
        let mut client = dynamic_client();
        let mut handler = Handler::default();
        let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
        sparse(&client, 12, &wire, 5, &[]);
        assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).unwrap());
        if reconnect {
            let event = Event::ClientMessage(ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 1,
                window: 10, type_: 8, data: [3, 0, 0, 20, 0].into(),
            });
            assert!(client.filter_event(&event, &mut handler).unwrap());
        } else {
            let wire = xim_parser::write_to_vec(Request::DisconnectReply {});
            let mut bytes = [0; 20];
            bytes[..wire.len()].copy_from_slice(&wire);
            let event = Event::ClientMessage(ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT, format: 8, sequence: 1,
                window: 10, type_: 9, data: bytes.into(),
            });
            assert!(client.filter_event(&event, &mut handler).unwrap());
        }
        // The new peer replaces its property; its first notification starts at zero.
        client.has_conn.sparse_properties.borrow_mut().remove(&12);
        let fresh = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"fresh"));
        append(&client, 12, &fresh);
        handler.delivered.clear();
        assert!(client.filter_event(&property_event(fresh.len(), 12), &mut handler).unwrap());
        assert_eq!(handler.delivered, vec![(7, 11, "fresh".into())]);
        let requests = client.has_conn.with_reply.borrow();
        let last = requests.last().unwrap();
        assert_eq!(last[0], 20);
        assert_eq!(u32::from_ne_bytes(last[16..20].try_into().unwrap()), 0);
    }
}

#[test]
fn property_design_sparse_consumed_offsets_can_cross_64mib_without_pending_growth() {
    let client = dynamic_client();
    let packet = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    // Validate sparse fixture offset semantics past64MiB with only40 unread bytes.
    sparse(&client, 12, &packet, 3355446, &[]);
    let offset = 3355444u32 * 5;
    let response = client.conn().get_property(true, 10, 12u32, AtomEnum::ANY, offset, 5).unwrap().reply().unwrap();
    assert_eq!(response.value, packet);
    assert_eq!(response.bytes_after, 20);
    assert!(client.has_conn.deleted_properties.borrow().is_empty());
    let last = client.conn().get_property(true, 10, 12u32, AtomEnum::ANY, offset + 5, 5).unwrap().reply().unwrap();
    assert_eq!(last.bytes_after, 0);
    assert_eq!(*client.has_conn.deleted_properties.borrow(), vec![12]);
}

#[test]
fn property_design_continuously_appended_sparse_stream_crosses_64mib_with_small_tail() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let text = vec![b'a'; 64000];
    let packet = xim_parser::write_to_vec(reply(ReplyKind::Commit, &text));
    let units = (packet.len() / 4) as u32;
    for index in 0..1100u32 {
        // One new packet is appended before each notification, leaving one unread packet.
        sparse(&client, 12, &packet, index as usize + 2, &[]);
        assert!(client.filter_event(&property_event(packet.len(), 12), &mut handler).unwrap());
        assert_eq!(handler.delivered.len(), 1);
        handler.delivered.clear();
        assert!(client.has_conn.deleted_properties.borrow().is_empty(), "a continuously appended unread tail must retain the property");
        let requests = client.has_conn.with_reply.borrow();
        let raw = requests.last().unwrap();
        assert_eq!(u32::from_ne_bytes(raw[16..20].try_into().unwrap()), index * units);
        drop(requests);
        client.has_conn.with_reply.borrow_mut().clear();
    }
    assert!(1099usize * packet.len() > 64 * 1024 * 1024);
    assert_eq!(handler.commit_calls, 1100);
}

#[test]
fn property_design_related_error_resets_only_its_atom_offset() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let packet = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    for atom in [12, 13] {
        sparse(&client, atom, &packet, 5, &[]);
        assert!(client.filter_event(&property_event(packet.len(), atom), &mut handler).unwrap());
    }
    assert!(client.filter_event(&property_event(0, 12), &mut handler).is_err());
    let before = client.has_conn.with_reply.borrow().len();
    assert!(client.filter_event(&property_event(packet.len(), 13), &mut handler).unwrap());
    let requests = client.has_conn.with_reply.borrow();
    let last = requests.last().unwrap();
    assert_eq!(requests.len(), before + 1);
    assert_eq!(u32::from_ne_bytes(last[16..20].try_into().unwrap()), 5);
}

#[test]
fn negotiated_01_reconnect_releases_property_offsets_and_refuses_mixed_input() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    sparse(&client, 12, &wire, 5, &[]);
    assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).unwrap());
    assert_eq!(client.properties.retained(), (1, 80));
    let xconnect = Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 1,
        window: 10, type_: 8, data: [3, 0, 1, 64, 0].into(),
    });
    assert!(client.filter_event(&xconnect, &mut handler).unwrap());
    assert_eq!(client.properties.retained(), (0, 0));
    client.has_conn.with_reply.borrow_mut().clear();
    handler.delivered.clear();
    let fragmented = xim_parser::write_to_vec(reply(ReplyKind::Commit, &[b'a'; 60]));
    let mut bytes = [0; 20];
    bytes.copy_from_slice(&fragmented[..20]);
    let more = Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 8, sequence: 1,
        window: 10, type_: 42, data: bytes.into(),
    });
    assert!(client.filter_event(&more, &mut handler).unwrap());
    assert_eq!(client.transport.retained().0, 20);
    assert!(matches!(client.filter_event(&property_event(20, 12), &mut handler), Err(ClientError::InvalidReply)));
    assert_eq!(client.transport.retained(), (0, 0));
    assert_eq!(client.properties.retained(), (0, 0));
    assert!(client.has_conn.with_reply.borrow().is_empty());
    assert!(handler.delivered.is_empty());
}

#[test]
fn callback_error_retires_its_property_without_resetting_other_atom() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    for atom in [12, 13] {
        sparse(&client, atom, &wire, 5, &[]);
        assert!(client.filter_event(&property_event(wire.len(), atom), &mut handler).unwrap());
    }
    assert_eq!(client.properties.retained(), (2, 160));
    handler.delivered.clear();
    handler.reject_commit = true;
    assert!(matches!(client.filter_event(&property_event(wire.len(), 12), &mut handler), Err(ClientError::UnsupportedTransport)));
    assert_eq!(client.properties.retained(), (1, 80));
    assert!(handler.delivered.is_empty());
    handler.reject_commit = false;
    assert!(client.filter_event(&property_event(wire.len(), 13), &mut handler).unwrap());
    assert_eq!(client.properties.retained(), (1, 60));
    let requests = client.has_conn.with_reply.borrow();
    let last = requests.last().unwrap();
    assert_eq!(u32::from_ne_bytes(last[16..20].try_into().unwrap()), 5);
    assert_eq!(handler.delivered, vec![(7, 11, "hello".into())]);
}

#[test]
fn server_window_error_clears_offsets_while_unrelated_error_preserves_them() {
    let mut client = dynamic_client();
    let mut handler = Handler::default();
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    sparse(&client, 12, &wire, 5, &[]);
    assert!(client.filter_event(&property_event(wire.len(), 12), &mut handler).unwrap());
    let error = |bad_value| Event::Error(X11Error {
        error_kind: x11rb::protocol::ErrorKind::Window, error_code: 3, sequence: 0,
        bad_value, minor_opcode: 0, major_opcode: 25, extension_name: None, request_name: None,
    });
    assert!(!client.filter_event(&error(99), &mut handler).unwrap());
    assert_eq!(client.properties.retained(), (1, 80));
    assert!(matches!(client.filter_event(&error(3), &mut handler), Err(ClientError::NoXimServer)));
    assert_eq!(client.properties.retained(), (0, 0));
    assert_eq!(client.transport.retained(), (0, 0));
}
