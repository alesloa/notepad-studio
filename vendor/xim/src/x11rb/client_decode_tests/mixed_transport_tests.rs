// X Consortium XIM Appendix D.3: negotiated mixed CM/property-with-CM.
use super::*;
use x11rb::protocol::xproto::{Property, PropertyNotifyEvent, PROPERTY_NOTIFY_EVENT};

fn negotiate(major: u32, minor: u32) -> Event {
    Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 32, sequence: 1,
        window: 10, type_: 8, data: [3, major, minor, 64, 0].into(),
    })
}
fn notification(window: u32, state: Property) -> Event {
    Event::PropertyNotify(PropertyNotifyEvent {
        response_type: PROPERTY_NOTIFY_EVENT, sequence: 2, window, atom: 12,
        time: 3, state,
    })
}
fn stored_packets() -> (X11rbClient<WireConnection>, Vec<u8>) {
    let wire = xim_parser::write_to_vec(reply(ReplyKind::Commit, b"hello"));
    let mut c = client(Vec::new());
    c.has_conn.dynamic_property = true;
    c.has_conn.property_store.borrow_mut().insert(12, wire.clone());
    (c, wire)
}
fn expected_negotiation(major: u32, minor: u32) {
    let mut c = client(Vec::new());
    let actual = c.filter_event(&negotiate(major, minor), &mut Handler::default());
    assert!(matches!(actual, Ok(true)), "spec transport {}.{}: {:?}", major, minor, actual);
}
#[test] fn mixed_02_negotiation() { expected_negotiation(0, 2); }

#[test] fn mixed_00_negotiation() { expected_negotiation(0, 0); }
#[test] fn mixed_01_negotiation() { expected_negotiation(0, 1); }

#[test]
fn mixed_02_complete_cm_then_property() {
    let (mut c, wire) = stored_packets();
    assert!(c.filter_event(&negotiate(0,2), &mut Handler::default()).unwrap());
    c.has_conn.sent.borrow_mut().clear(); c.has_conn.with_reply.borrow_mut().clear();
    let mut h = Handler::default();
    let mut bytes = [0; 20]; bytes[..wire.len()].copy_from_slice(&wire);
    let cm = Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 8, sequence: 1, window: 10,
        type_: 9, data: bytes.into(),
    });
    assert!(c.filter_event(&cm, &mut h).unwrap());
    assert_eq!(h.delivered.len(), 1);
    let actual = c.filter_event(&event(wire.len()), &mut h);
    assert!(matches!(actual, Ok(true)), "complete CM must allow next property packet: {:?}", actual);
    assert_eq!(h.delivered.len(), 2);
}


#[test]
fn mixed_property_only_deleted() {
    let (mut c, _) = stored_packets();
    let mut h = Handler::default();
    assert!(!c.filter_event(&notification(10, Property::DELETE), &mut h).unwrap());
    assert!(c.has_conn.with_reply.borrow().is_empty());
    assert!(h.delivered.is_empty());
}
#[test]
fn mixed_unrelated_window() {
    let (mut c, _) = stored_packets();
    let mut h = Handler::default();
    assert!(!c.filter_event(&notification(999, Property::NEW_VALUE), &mut h).unwrap());
    assert!(c.has_conn.with_reply.borrow().is_empty());
    assert!(h.delivered.is_empty());
}

#[test]
fn mixed_02_large_outbound_uses_property() {
    use crate::client::Client;
    let mut c = client(Vec::new());
    assert!(c.filter_event(&negotiate(0,2), &mut Handler::default()).unwrap());
    c.has_conn.sent.borrow_mut().clear(); c.has_conn.with_reply.borrow_mut().clear();
    c.open(&"C".repeat(80)).unwrap();
    let sent = c.has_conn.sent.borrow();
    assert_eq!(sent.len(), 2, "one ChangeProperty and one protocol notification");
    assert_eq!(sent[0][0], 18);
    assert_eq!(sent[1][0], 25);
    let (message, _) = ClientMessageEvent::try_parse(&sent[1][12..]).unwrap();
    assert_eq!(message.format, 32);
}
#[test]
fn mixed_02_pending_cm_refuses_property_without_fetch() {
    let (mut c, wire) = stored_packets();
    assert!(c.filter_event(&negotiate(0,2), &mut Handler::default()).unwrap());
    c.has_conn.sent.borrow_mut().clear(); c.has_conn.with_reply.borrow_mut().clear();
    let long = xim_parser::write_to_vec(reply(ReplyKind::Commit, &[b'a'; 80]));
    let mut bytes = [0; 20]; bytes.copy_from_slice(&long[..20]);
    let more = Event::ClientMessage(ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT, format: 8, sequence: 1, window: 10,
        type_: 42, data: bytes.into(),
    });
    let mut h = Handler::default();
    assert!(c.filter_event(&more, &mut h).unwrap());
    assert!(h.delivered.is_empty());
    assert!(matches!(c.filter_event(&event(wire.len()), &mut h), Err(ClientError::InvalidReply)));
    assert!(c.has_conn.with_reply.borrow().is_empty());
    assert_eq!(c.transport.retained(), (0, 0));
}
#[test]
fn mixed_00_appended_property_packets() {
    let (mut c, wire) = stored_packets();
    c.has_conn.property_store.borrow_mut().get_mut(&12).unwrap().extend_from_slice(&wire);
    let mut h = Handler::default();
    assert!(c.filter_event(&event(wire.len()), &mut h).unwrap());
    assert_eq!(c.properties.retained(), (1, wire.len()));
    assert!(c.filter_event(&event(wire.len()), &mut h).unwrap());
    assert_eq!(h.delivered.len(), 2);
    assert_eq!(c.properties.retained(), (0, 0));
}

fn connected(threshold: u32) -> X11rbClient<WireConnection> {
    let mut c=client(Vec::new()); let mut ev=negotiate(0,2);
    if let Event::ClientMessage(m)=&mut ev {m.data=[3,0,2,threshold,0].into();}
    assert!(c.filter_event(&ev,&mut Handler::default()).unwrap());
    assert!(matches!(c.transport.mode,ClientMode::Mixed {dividing_size} if dividing_size==threshold));
    c.has_conn.sent.borrow_mut().clear(); c.has_conn.with_reply.borrow_mut().clear();
    c.has_conn.send_attempts.replace(0); c.sequence=0; c
}
#[test]
fn mixed_actual_writer_thresholds_and_equality_preserve_packet_bytes() {
    use crate::client::Client;
    for threshold in [0,20,40,64,u32::MAX] {
        for locale_len in [0,15,35,59,80] {
            let mut c=connected(threshold);let locale="C".repeat(locale_len);
            let expected=xim_parser::write_to_vec(Request::Open{locale:locale.clone()});
            c.open(&locale).unwrap();let sent=c.has_conn.sent.borrow();
            if expected.len()>=threshold as usize {
                assert_eq!(sent.len(),2);assert_eq!(sent[0][0],18);assert_eq!(sent[0][1],2);
                assert_eq!(sent[0][16],8);assert_eq!(&sent[0][12..16],u32::from(AtomEnum::STRING).to_ne_bytes());
                assert_eq!(&sent[0][24..],expected);
                let (m,_)=ClientMessageEvent::try_parse(&sent[1][12..]).unwrap();
                assert_eq!(m.format,32);assert_eq!(m.data.as_data32(),[expected.len() as u32,42,0,0,0]);
            } else {
                assert_eq!(sent.len(),expected.chunks(20).count());let mut actual=Vec::new();
                for (i,r) in sent.iter().enumerate(){assert_eq!(r[0],25);
                    let (m,_)=ClientMessageEvent::try_parse(&r[12..]).unwrap();assert_eq!(m.format,8);
                    assert_eq!(m.type_,if i+1==sent.len(){9}else{42});actual.extend_from_slice(&m.data.as_data8());}
                assert_eq!(&actual[..expected.len()],expected);assert!(actual[expected.len()..].iter().all(|b|*b==0));
            }
            assert!(c.buf.is_empty());
        }
    }
}
#[test]
fn mixed_writer_failures_retire_fragment_and_preserve_independent_property() {
    use crate::client::Client;
    for fail in [1,2,3,4] {
        let mut c=connected(64);c.properties.finish(12,20,20).unwrap();
        let long=xim_parser::write_to_vec(reply(ReplyKind::Commit,&[b'a';80]));
        let mut data=[0;20];data.copy_from_slice(&long[..20]);
        let ev=Event::ClientMessage(ClientMessageEvent{response_type:CLIENT_MESSAGE_EVENT,format:8,
            sequence:1,window:10,type_:42,data:data.into()});
        assert!(c.filter_event(&ev,&mut Handler::default()).unwrap());
        c.has_conn.reject_intern=fail==1;
        c.has_conn.reject_send_at=if fail==2{Some(1)}else if fail==3{Some(2)}else{None};
        c.has_conn.reject_flush=fail==4;
        assert!(c.open(&"C".repeat(80)).is_err());assert!(c.buf.is_empty());
        assert_eq!(c.transport.retained(),(0,0));assert_eq!(c.properties.retained(),(1,20));
        assert_eq!(c.sequence,if fail==1{0}else{1});
        assert!(c.has_conn.sent.borrow().iter().all(|r|r[0]!=19));
    }
}
#[test]
fn mixed_writer_atom_sequence_wraps_in_existing_finite_namespace() {
    use crate::client::Client;
    let mut c=connected(64);c.sequence=u16::MAX;
    c.open(&"C".repeat(80)).unwrap();assert_eq!(c.sequence,0);
    c.open(&"C".repeat(80)).unwrap();assert_eq!(c.sequence,1);
    let r=c.has_conn.with_reply.borrow();let names:Vec<_>=r.iter().map(|r|{
        let n=u16::from_ne_bytes(r[4..6].try_into().unwrap()) as usize;r[8..8+n].to_vec()}).collect();
    assert_eq!(names,vec![b"_XIM_DATA_65535".to_vec(),b"_XIM_DATA_0".to_vec()]);
}
#[test]
fn mixed_combined_pending_property_and_cm_budget_refuses_before_reserve() {
    let mut c=connected(64);c.properties.finish(12,4,64*1024*1024-4).unwrap();
    let long=xim_parser::write_to_vec(reply(ReplyKind::Commit,&[b'a';80]));
    let mut data=[0;20];data.copy_from_slice(&long[..20]);
    let ev=Event::ClientMessage(ClientMessageEvent{response_type:CLIENT_MESSAGE_EVENT,format:8,
        sequence:1,window:10,type_:42,data:data.into()});
    let mut h=Handler::default();assert!(matches!(c.filter_event(&ev,&mut h),Err(ClientError::InvalidReply)));
    assert_eq!(c.transport.retained(),(0,0));assert_eq!(c.properties.retained(),(1,64*1024*1024-4));
    assert!(h.delivered.is_empty());
}

fn receive_cm(c:&mut X11rbClient<WireConnection>,h:&mut Handler,wire:&[u8]) {
    let n=wire.chunks(20).count();
    for (i,bytes) in wire.chunks(20).enumerate(){
        let mut data=[0;20];data[..bytes.len()].copy_from_slice(bytes);
        let e=Event::ClientMessage(ClientMessageEvent{response_type:CLIENT_MESSAGE_EVENT,format:8,
            sequence:1,window:10,type_:if i+1==n{9}else{42},data:data.into()});
        let before=h.commit_calls;assert!(c.filter_event(&e,h).unwrap());
        if i+1!=n{assert_eq!(h.commit_calls,before);}
    }
}
fn append(c:&X11rbClient<WireConnection>,atom:u32,bytes:&[u8]) {
    c.has_conn.property_store.borrow_mut().entry(atom).or_default().extend_from_slice(bytes);
}
fn tagged_property(length:usize,atom:u32)->Event {
    let mut e=event(length);if let Event::ClientMessage(m)=&mut e{m.data=[length as u32,atom,0,0,0].into();} e
}
#[test]
fn mixed_unicode_cm_property_cm_dispatches_each_complete_packet_in_order() {
    let mut c=connected(64);c.has_conn.dynamic_property=true;let mut h=Handler::default();
    let texts=["文😀", "property café 文😀", "long fragmented 文😀 café repeated repeated repeated"];
    let packets:Vec<_>=texts.iter().map(|t|xim_parser::write_to_vec(reply(ReplyKind::Commit,&xim_ctext::utf8_to_compound_text(t)))).collect();
    receive_cm(&mut c,&mut h,&packets[0]);append(&c,12,&packets[1]);
    assert!(c.filter_event(&event(packets[1].len()),&mut h).unwrap());receive_cm(&mut c,&mut h,&packets[2]);
    assert_eq!(h.delivered,texts.iter().map(|t|(7,11,(*t).into())).collect::<Vec<_>>());assert_eq!(h.commit_calls,3);
}
#[test]
fn mixed_variable_property_append_offsets_delete_reuse_between_cm_frames() {
    let mut c=connected(64);c.has_conn.dynamic_property=true;let mut h=Handler::default();
    let a=xim_parser::write_to_vec(reply(ReplyKind::Commit,b"first"));
    let b=xim_parser::write_to_vec(reply(ReplyKind::Commit,b"second longer"));append(&c,12,&a);append(&c,12,&b);
    assert!(c.filter_event(&event(a.len()),&mut h).unwrap());receive_cm(&mut c,&mut h,&a);
    assert!(c.filter_event(&event(b.len()),&mut h).unwrap());assert_eq!(c.properties.retained(),(0,0));
    append(&c,12,&a);assert!(c.filter_event(&event(a.len()),&mut h).unwrap());
    let offsets:Vec<_>=c.has_conn.with_reply.borrow().iter().map(|r|u32::from_ne_bytes(r[16..20].try_into().unwrap())).collect();
    assert_eq!(offsets,vec![0,(a.len()/4) as u32,0]);assert_eq!(h.commit_calls,4);
}
#[test]
fn mixed_sync_commit_callback_precedes_cm_or_property_ack() {
    for threshold in [0,64] { for use_cm in [false,true] {
        let mut c=connected(threshold);c.has_conn.dynamic_property=true;let mut h=Handler::default();
        let wire=xim_parser::write_to_vec(Request::Commit{input_method_id:7,input_context_id:11,
            data:xim_parser::CommitData::Chars{commited:xim_ctext::utf8_to_compound_text("文😀"),syncronous:true}});
        if use_cm{receive_cm(&mut c,&mut h,&wire);}else{append(&c,12,&wire);assert!(c.filter_event(&event(wire.len()),&mut h).unwrap());}
        assert_eq!(h.delivered,vec![(7,11,"文😀".into())]);assert!(!h.ack_seen_during_commit);
        let sent=c.has_conn.sent.borrow();let expected=Request::SyncReply{input_method_id:7,input_context_id:11};
        if threshold==0{assert_eq!(sent.len(),2);assert_eq!(xim_parser::read::<Request>(&sent[0][24..]).unwrap(),expected);}
        else{assert_eq!(sent.len(),1);let (m,_)=ClientMessageEvent::try_parse(&sent[0][12..]).unwrap();assert_eq!(xim_parser::read::<Request>(&m.data.as_data8()).unwrap(),expected);}
    }}
}
#[test]
fn mixed_callback_failure_retires_own_atom_without_ack_or_other_atom_reset() {
    let mut c=connected(64);c.has_conn.dynamic_property=true;let mut h=Handler::default();
    let wire=xim_parser::write_to_vec(Request::Commit{input_method_id:7,input_context_id:11,
        data:xim_parser::CommitData::Chars{commited:b"hello".to_vec(),syncronous:true}});
    for atom in [12,13]{append(&c,atom,&wire);append(&c,atom,&wire);assert!(c.filter_event(&tagged_property(wire.len(),atom),&mut h).unwrap());}
    c.has_conn.sent.borrow_mut().clear();h.reject_commit=true;h.delivered.clear();
    assert!(c.filter_event(&event(wire.len()),&mut h).is_err());assert!(c.has_conn.sent.borrow().is_empty());
    assert!(h.delivered.is_empty());assert_eq!(c.properties.retained(),(1,wire.len()));
    h.reject_commit=false;assert!(c.filter_event(&tagged_property(wire.len(),13),&mut h).unwrap());assert_eq!(c.properties.retained(),(0,0));
}
#[test]
fn mixed_reconnect_disconnect_and_server_error_retire_both_receivers() {
    for lifecycle in [0,1,2] {
        let mut c=connected(64);c.properties.finish(12,20,20).unwrap();let mut h=Handler::default();
        if lifecycle==0{assert!(c.filter_event(&negotiate(0,2),&mut h).unwrap());}
        else if lifecycle==1{let wire=xim_parser::write_to_vec(Request::DisconnectReply{});receive_cm(&mut c,&mut h,&wire);}
        else{let e=Event::Error(X11Error{error_kind:x11rb::protocol::ErrorKind::Window,error_code:3,
            sequence:1,bad_value:3,minor_opcode:0,major_opcode:25,extension_name:None,request_name:None});
            assert!(matches!(c.filter_event(&e,&mut h),Err(ClientError::NoXimServer)));}
        assert_eq!(c.properties.retained(),(0,0));assert_eq!(c.transport.retained(),(0,0));
    }
}
#[test]
fn mixed_malformed_property_or_cm_never_publishes_and_clean_frame_recovers() {
    for kind in [0,1,2,3] {
        let mut c=connected(64);c.has_conn.dynamic_property=true;let mut h=Handler::default();
        let wire=xim_parser::write_to_vec(reply(ReplyKind::Commit,b"hello"));append(&c,12,&wire);
        let e=if kind==0{event(262148)}else if kind==1{event(5)}else if kind==2{
            c.has_conn.property_metadata=Some((8,1,0));event(wire.len())}else{
            let bad=xim_parser::write_to_vec(reply(ReplyKind::Commit,&[0x1b,0x25,0x47,0xff]));
            let mut data=[0;20];data[..bad.len()].copy_from_slice(&bad);
            Event::ClientMessage(ClientMessageEvent{response_type:CLIENT_MESSAGE_EVENT,format:8,sequence:1,window:10,type_:9,data:data.into()})};
        assert!(c.filter_event(&e,&mut h).is_err());assert!(h.delivered.is_empty());
        assert_eq!(c.transport.retained(),(0,0));assert_eq!(c.properties.retained(),(0,0));
        c.has_conn.property_metadata=None;receive_cm(&mut c,&mut h,&wire);assert_eq!(h.delivered,vec![(7,11,"hello".into())]);
    }
}
#[test]
fn mixed_oversized_writer_refuses_before_intern_or_send() {
    use crate::client::Client;
    let mut c=connected(0);assert!(matches!(c.open(&"C".repeat(262144)),Err(ClientError::InvalidReply)));
    assert!(c.has_conn.with_reply.borrow().is_empty());assert!(c.has_conn.sent.borrow().is_empty());assert!(c.buf.is_empty());assert_eq!(c.sequence,0);
}

#[test]
fn mixed_full_pending_map_accepts_consumed_atom_and_refuses_new_tail() {
    for extra_tail in [false,true] {
        let mut c=connected(64);c.has_conn.dynamic_property=true;
        for atom in 0..256{c.properties.finish(atom,4,4).unwrap();}
        let wire=xim_parser::write_to_vec(reply(ReplyKind::Commit,b"hello"));append(&c,1000,&wire);
        if extra_tail{append(&c,1000,&wire);}
        let mut h=Handler::default();let r=c.filter_event(&tagged_property(wire.len(),1000),&mut h);
        if extra_tail{assert!(r.is_err());assert!(h.delivered.is_empty());}else{assert!(r.unwrap());assert_eq!(h.commit_calls,1);}
        assert_eq!(c.properties.retained(),(256,1024));
        receive_cm(&mut c,&mut h,&wire);assert_eq!(h.commit_calls,if extra_tail{1}else{2});
    }
}
