// Execute the actual GPUI handler and caller completion helper with production
// XIM wire filtering. The fake connection supplies bytes and transport failures.
use super::*;
use ::x11rb::protocol::xproto;

mod actual_gpui {
    pub mod xim_callbacks {
        use std::{string::String, vec::Vec};
        include!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../gpui/crates/gpui/src/platform/linux/x11/xim_callbacks.rs"));
    }
    // Preserve the existing GPUI enum names in this test-only inclusion.
    #[allow(clippy::enum_variant_names)]
    pub mod xim_handler {
        use std::{string::String, vec::Vec};
        include!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../gpui/crates/gpui/src/platform/linux/x11/xim_handler.rs"));
    }

    pub fn finish(result: Result<bool, xim::ClientError>, handler: &mut xim_handler::XimHandler)
        -> (Result<bool, xim::ClientError>, std::vec::Vec<(u32, std::string::String)>)
    {
        let mut delivered = std::vec::Vec::new();
        let result = xim_callbacks::finish_xim_filter(result, handler.take_callbacks(), |event| {
            match event {
                xim_handler::XimCallbackEvent::XimCommitEvent(window, text)
                | xim_handler::XimCallbackEvent::XimPreeditEvent(window, text) => {
                    delivered.push((window, text));
                }
                xim_handler::XimCallbackEvent::XimXEvent(_) => {}
            }
        });
        (result, delivered)
    }

    pub fn finish_keys(handler: &mut xim_handler::XimHandler) -> std::vec::Vec<(u32, u8)> {
        let mut keys = std::vec::Vec::new();
        assert!(xim_callbacks::finish_xim_filter(Ok(false), handler.take_callbacks(), |event| {
            if let xim_handler::XimCallbackEvent::XimXEvent(event) = event {
                match event {
                    x11rb::protocol::Event::KeyPress(key)
                    | x11rb::protocol::Event::KeyRelease(key) => keys.push((key.event, key.response_type)),
                    _ => panic!("unexpected forwarded callback"),
                }
            }
        }).is_ok());
        keys
    }
}
use actual_gpui::xim_handler::XimHandler;

fn filter(c: &mut X11rbClient<WireConnection>, h: &mut XimHandler, request: Request)
    -> Result<bool, ClientError>
{
    let packet = xim_parser::write_to_vec(request);
    c.has_conn.property = packet.clone();
    c.filter_event(&event(packet.len()), h)
}

#[test]
fn gpui_fifo_single_actual_wire_event_retains_one_event_behavior() {
    let mut c = client(Vec::new()); let mut h = XimHandler::new(); h.window = 123;
    let result = filter(&mut c, &mut h, reply(ReplyKind::Commit, b"first"));
    let (result, delivered) = actual_gpui::finish(result, &mut h);
    assert!(matches!(result, Ok(true)));
    assert_eq!(delivered, [(123, "first".into())]);
    assert!(actual_gpui::finish(Ok(false), &mut h).1.is_empty());
}

#[test]
fn gpui_fifo_two_actual_wire_commits_keep_order_and_captured_values() {
    let mut c = client(Vec::new()); let mut h = XimHandler::new(); h.window = 123;
    let first = "東京\r\n😀";
    let second = "café";
    assert!(filter(&mut c, &mut h,
        reply(ReplyKind::Commit, &xim_ctext::utf8_to_compound_text(first))).unwrap());
    h.window = 456;
    let result = filter(&mut c, &mut h,
        reply(ReplyKind::Commit, &xim_ctext::utf8_to_compound_text(second)));
    assert_eq!(actual_gpui::finish(result, &mut h).1,
        [(123, first.into()), (456, second.into())]);
}

#[test]
fn gpui_fifo_actual_wire_preedit_precedes_commit() {
    let mut c = client(Vec::new()); let mut h = XimHandler::new(); h.window = 123;
    assert!(filter(&mut c, &mut h, reply(ReplyKind::Preedit, b"preedit")).unwrap());
    let result = filter(&mut c, &mut h, reply(ReplyKind::Commit, b"commit"));
    assert_eq!(actual_gpui::finish(result, &mut h).1,
        [(123, "preedit".into()), (123, "commit".into())]);
}

#[test]
fn gpui_fifo_forwarded_keys_keep_event_origins_and_order() {
    let mut c = client(Vec::new()); let mut h = XimHandler::new();
    let mut key = xproto::KeyPressEvent {
        event: 123, response_type: xproto::KEY_PRESS_EVENT, ..Default::default()
    };
    h.handle_forward_event(&mut c, 7, 11, xim::ForwardEventFlag::empty(), key).unwrap();
    key.event = 456; key.response_type = xproto::KEY_RELEASE_EVENT;
    h.handle_forward_event(&mut c, 7, 11, xim::ForwardEventFlag::empty(), key).unwrap();
    assert_eq!(actual_gpui::finish_keys(&mut h),
        [(123, xproto::KEY_PRESS_EVENT), (456, xproto::KEY_RELEASE_EVENT)]);
    assert!(actual_gpui::finish_keys(&mut h).is_empty());
}

fn ack_failure(flush: bool) {
    let mut c = client(Vec::new()); let mut h = XimHandler::new(); h.window = 123;
    assert!(filter(&mut c, &mut h, reply(ReplyKind::Commit, b"prefix")).unwrap());
    if flush { c.has_conn.reject_flush = true; } else { c.has_conn.reject_send = true; }
    let result = filter(&mut c, &mut h, Request::Commit {
        input_method_id: 7, input_context_id: 11,
        data: xim_parser::CommitData::Chars { commited: b"accepted".to_vec(), syncronous: true },
    });
    let (result, delivered) = actual_gpui::finish(result, &mut h);
    assert!(result.is_err());
    assert_eq!(delivered, [(123, "prefix".into()), (123, "accepted".into())]);
    assert!(actual_gpui::finish(Ok(false), &mut h).1.is_empty());
}

#[test]
fn gpui_fifo_actual_ack_send_failure_publishes_accepted_prefix() { ack_failure(false); }
#[test]
fn gpui_fifo_actual_ack_flush_failure_publishes_accepted_prefix() { ack_failure(true); }

#[test]
fn gpui_fifo_actual_malformed_next_frame_publishes_only_accepted_prefix() {
    let mut c = client(Vec::new()); let mut h = XimHandler::new(); h.window = 123;
    assert!(filter(&mut c, &mut h, reply(ReplyKind::Commit, b"prefix")).unwrap());
    c.has_conn.property = vec![0, 0, 0, 0];
    let result = c.filter_event(&event(4), &mut h);
    let (result, delivered) = actual_gpui::finish(result, &mut h);
    assert!(result.is_err());
    assert_eq!(delivered, [(123, "prefix".into())]);
}

#[test]
fn gpui_fifo_actual_count_refusal_has_no_success_ack_and_keeps_prefix() {
    let mut c = client(Vec::new()); let mut h = XimHandler::new(); h.window = 123;
    for _ in 0..128 { h.handle_commit(&mut c, 7, 11, "prefix").unwrap(); }
    let result = filter(&mut c, &mut h, Request::Commit {
        input_method_id: 7, input_context_id: 11,
        data: xim_parser::CommitData::Chars { commited: b"refused".to_vec(), syncronous: true },
    });
    assert!(c.has_conn.sent.borrow().is_empty());
    let (result, delivered) = actual_gpui::finish(result, &mut h);
    assert!(result.is_err());
    assert_eq!(delivered.len(), 128);
    assert!(delivered.iter().all(|(_, text)| text == "prefix"));
    assert!(h.handle_commit(&mut c, 7, 11, "after take").is_ok());
    assert_eq!(actual_gpui::finish(Ok(false), &mut h).1, [(123, "after take".into())]);
}
