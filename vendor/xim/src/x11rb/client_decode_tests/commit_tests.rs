use super::*;
use xim_parser::{read, CommitData};

#[derive(Clone, Copy)]
enum Kind {
    Chars,
    Keysym,
    Both,
}

fn commit(kind: Kind, synchronous: bool, text: &[u8]) -> Request {
    let data = match kind {
        Kind::Chars => CommitData::Chars {
            commited: text.to_vec(),
            syncronous: synchronous,
        },
        Kind::Keysym => CommitData::Keysym {
            keysym: 0x41,
            syncronous: synchronous,
        },
        Kind::Both => CommitData::Both {
            keysym: 0x41,
            commited: text.to_vec(),
            syncronous: synchronous,
        },
    };
    Request::Commit {
        input_method_id: 7,
        input_context_id: 11,
        data,
    }
}

fn wire(kind: Kind, synchronous: bool, text: &[u8]) -> Vec<u8> {
    let bytes = xim_parser::write_to_vec(commit(kind, synchronous, text));
    // XIM flags are bit0 synchronous, bit1 characters, bit2 keysym.
    let lookup = match kind {
        Kind::Chars => 2u16,
        Kind::Keysym => 4,
        Kind::Both => 6,
    };
    let expected = lookup | u16::from(synchronous);
    assert_eq!(bytes.get(8..10).unwrap(), expected.to_ne_bytes());
    bytes
}

fn assert_sync_reply(client: &X11rbClient<WireConnection>) {
    let sent = client.has_conn.sent.borrow();
    assert_eq!(sent.len(), 1);
    assert_eq!(*client.has_conn.send_attempts.borrow(), 1);
    let request = sent.first().unwrap();
    assert_eq!(request.len(), 44);
    assert_eq!(request.first().copied(), Some(25)); // X11 SendEvent opcode.
    let (message, _) = ClientMessageEvent::try_parse(request.get(12..).unwrap()).unwrap();
    assert_eq!(message.format, 8);
    assert_eq!(message.window, 3); // The fixture's negotiated IM window.
    assert_eq!(message.type_, 9);
    assert_eq!(
        read::<Request>(&message.data.as_data8()).unwrap(),
        Request::SyncReply {
            input_method_id: 7,
            input_context_id: 11,
        }
    );
}

fn valid_commit(kind: Kind, synchronous: bool) {
    let text = "文😀 café";
    let bytes = wire(kind, synchronous, &xim_ctext::utf8_to_compound_text(text));
    let mut client = client(bytes.clone());
    let mut handler = Handler::default();
    assert!(client
        .filter_event(&event(bytes.len()), &mut handler)
        .unwrap());
    match kind {
        Kind::Keysym => {
            assert!(handler.delivered.is_empty());
            assert_eq!(handler.commit_calls, 0);
        }
        Kind::Chars | Kind::Both => {
            assert_eq!(handler.delivered, vec![(7, 11, text.into())]);
            assert_eq!(handler.commit_calls, 1);
            assert!(!handler.ack_seen_during_commit);
        }
    }
    if synchronous {
        assert_sync_reply(&client);
    } else {
        assert!(client.has_conn.sent.borrow().is_empty());
        assert_eq!(*client.has_conn.send_attempts.borrow(), 0);
    }
}

#[test]
fn parser_preserves_all_commit_flags() {
    for kind in [Kind::Chars, Kind::Keysym, Kind::Both] {
        for synchronous in [false, true] {
            let text = xim_ctext::utf8_to_compound_text("文");
            assert_eq!(
                read::<Request>(&wire(kind, synchronous, &text)).unwrap(),
                commit(kind, synchronous, &text)
            );
        }
    }
}

#[test]
fn generated_commit_impl_matches_generator_snippet() {
    fn commit_impl(source: &str) -> &str {
        source.split_once("impl XimRead for CommitData {").unwrap().1
            .split_once("impl XimRead for HotKeyTriggers {").unwrap().0
    }
    let parser = include_str!("../../../xim-parser/src/parser.rs");
    let snippet = include_str!("../../../xim-gen/res/snippet.rs");
    assert_eq!(commit_impl(parser), commit_impl(snippet));
}

#[test]
fn synchronous_chars_ack_after_callback() {
    valid_commit(Kind::Chars, true);
}
#[test]
fn asynchronous_chars_do_not_ack() {
    valid_commit(Kind::Chars, false);
}
#[test]
fn synchronous_keysym_acks_without_text_callback() {
    valid_commit(Kind::Keysym, true);
}
#[test]
fn asynchronous_keysym_has_no_text_or_ack() {
    valid_commit(Kind::Keysym, false);
}
#[test]
fn synchronous_both_delivers_text_then_ack() {
    valid_commit(Kind::Both, true);
}
#[test]
fn asynchronous_both_delivers_text_without_ack() {
    valid_commit(Kind::Both, false);
}

#[test]
fn malformed_both_does_not_publish_or_ack() {
    for synchronous in [false, true] {
        let bytes = wire(Kind::Both, synchronous, b"\x1b%G\xff");
        let mut client = client(bytes.clone());
        let mut handler = Handler::default();
        assert!(matches!(
            client.filter_event(&event(bytes.len()), &mut handler),
            Err(ClientError::InvalidReply)
        ));
        assert!(handler.delivered.is_empty());
        assert_eq!(handler.commit_calls, 0);
        assert_eq!(*client.has_conn.send_attempts.borrow(), 0);
    }
}

#[test]
fn refused_text_callback_does_not_ack() {
    for kind in [Kind::Chars, Kind::Both] {
        for synchronous in [false, true] {
            let bytes = wire(kind, synchronous, &xim_ctext::utf8_to_compound_text("文"));
            let mut client = client(bytes.clone());
            let mut handler = Handler {
                reject_commit: true,
                ..Default::default()
            };
            assert!(matches!(
                client.filter_event(&event(bytes.len()), &mut handler),
                Err(ClientError::UnsupportedTransport)
            ));
            assert!(handler.delivered.is_empty());
            assert_eq!(handler.commit_calls, 1);
            assert_eq!(*client.has_conn.send_attempts.borrow(), 0);
            assert!(!handler.ack_seen_during_commit);
        }
    }
}

#[test]
fn acknowledgement_send_error_propagates_after_text_callback() {
    for kind in [Kind::Chars, Kind::Both] {
        let bytes = wire(kind, true, &xim_ctext::utf8_to_compound_text("文"));
        let mut client = client(bytes.clone());
        client.has_conn.reject_send = true;
        let mut handler = Handler::default();
        assert!(matches!(
            client.filter_event(&event(bytes.len()), &mut handler),
            Err(ClientError::Other(_))
        ));
        assert_eq!(handler.delivered, vec![(7, 11, "文".into())]);
        assert_eq!(handler.commit_calls, 1);
        assert!(!handler.ack_seen_during_commit);
        assert_eq!(*client.has_conn.send_attempts.borrow(), 1);
        assert!(client.has_conn.sent.borrow().is_empty());
    }
}
