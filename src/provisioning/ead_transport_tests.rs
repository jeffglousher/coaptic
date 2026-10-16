use super::*;
use crate::provisioning::{CipherSuite, Ead};

fn field(critical: bool) -> Ead {
    let mut result = Ead::new();
    result.push(0, None).unwrap();
    result
        .push(if critical { -65535 } else { 65535 }, Some(b"opaque"))
        .unwrap();
    result.push(0, Some(b"pad")).unwrap();
    result
}

fn reply(request: &[u8], message: &Message) -> Vec<u8> {
    let p = decode(request).unwrap();
    let options = [Opt::new(
        OptionNumber::CONTENT_FORMAT,
        &[RESPONSE_FORMAT as u8],
    )];
    let mut output = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::new(Type::Acknowledgement, Code::CHANGED, p.message_id())
        .with_token(p.token())
        .with_options(&options)
        .with_payload(message.as_bytes())
        .encode(&mut output)
        .unwrap();
    output[..len].to_vec()
}

#[test]
fn ead_coap_server_emits_cached_critical_errors_and_accepts_elective_first_and_third_fields() {
    for critical in [false, true] {
        for stage in [1, 3] {
            let a = identity(1, 0);
            let b = identity(2, 1);
            let data = field(critical);
            let empty = Ead::new();
            let mut template = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
            let mut ci = Io::default();
            template.flush(&mut ci, 0).unwrap();
            let first = ci.last();
            let parsed = decode(&first).unwrap();
            let options: Vec<_> = parsed.options().collect();
            let (initiator, m1) = Initiator::start_with_suite_and_ead(
                &a,
                b.peer(),
                ConnectionId::INITIATOR,
                CipherSuite::Suite2,
                if stage == 1 { &data } else { &empty },
                entropy(3),
            )
            .unwrap();
            let mut payload = std::vec![0xf5];
            payload.extend_from_slice(m1.as_bytes());
            let request = recode(
                &first,
                parsed.ty(),
                parsed.message_id(),
                parsed.token(),
                &options,
                &payload,
            );
            let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
            server
                .ingest(
                    &request,
                    CLIENT,
                    0,
                    |out| {
                        assert!(
                            !critical || stage != 1,
                            "critical field must refuse before entropy"
                        );
                        entropy(4)(out)
                    },
                    no_authorization,
                )
                .unwrap();
            let mut si = Io::default();
            let request = if critical && stage == 1 {
                request
            } else {
                server.flush(&mut si, 0).unwrap();
                let m2 = Message::from_slice(decode(&si.last()).unwrap().payload()).unwrap();
                let (confirm, m3) = initiator
                    .receive_message_2_with_ead(
                        &m2,
                        if stage == 3 { &data } else { &empty },
                        |_, _| false,
                        |_| true,
                    )
                    .unwrap();
                let mut payload = ConnectionId::RESPONDER.lakers().as_cbor().to_vec();
                payload.extend_from_slice(m3.as_bytes());
                let third = recode(
                    &first,
                    Type::Confirmable,
                    MessageId::new(1),
                    Token::from_checked(&[1]),
                    &options,
                    &payload,
                );
                server
                    .ingest(&third, CLIENT, 1, no_entropy, |_| {
                        assert!(!critical, "critical EAD must not reach final policy");
                        true
                    })
                    .unwrap();
                if !critical {
                    server.flush(&mut si, 1).unwrap();
                    let m4 = Message::from_slice(decode(&si.last()).unwrap().payload()).unwrap();
                    let client = confirm.receive_message_4(&m4, |_| true).unwrap();
                    let server = server.take_session().unwrap();
                    let (client, _) = client.into_parts();
                    let (server, _) = server.into_parts();
                    assert_eq!(client.sender_key(), server.recipient_key());
                    continue;
                }
                third
            };
            si.fail = true;
            assert_eq!(server.flush(&mut si, 2), Err(PollError::Io(9)));
            let saved = si.last();
            let response = decode(&saved).unwrap();
            assert_eq!(response.code(), Code::BAD_REQUEST);
            assert_eq!(
                PeerError::parse(response.payload()).unwrap().code(),
                PeerErrorCode::Unspecified
            );
            assert_eq!(response.token(), decode(&request).unwrap().token());
            si.fail = false;
            assert_eq!(
                server.flush(&mut si, 3),
                Err(PollError::Provisioning(Error::Profile))
            );
            assert_eq!(si.last(), saved);
            assert!(server.take_session().is_none());
            server
                .ingest(&request, CLIENT, 4, no_entropy, no_authorization)
                .unwrap();
            assert_eq!(
                server.flush(&mut si, 4),
                Err(PollError::Provisioning(Error::Profile))
            );
            assert_eq!(si.last(), saved);
        }
    }
}

#[test]
fn ead_coap_initiator_emits_critical_error_requests_and_accepts_elective_second_and_fourth_fields()
{
    for critical in [false, true] {
        for stage in [2, 4] {
            let a = identity(1, 0);
            let b = identity(2, 1);
            let data = field(critical);
            let empty = Ead::new();
            let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
            let mut ci = Io::default();
            client.flush(&mut ci, 0).unwrap();
            let first = ci.last();
            let m1 = Message::from_slice(&decode(&first).unwrap().payload()[1..]).unwrap();
            let (responder, m2) = Responder::receive_message_1_with_ead(
                &b,
                a.peer(),
                ConnectionId::RESPONDER,
                &m1,
                if stage == 2 { &data } else { &empty },
                |_| false,
                entropy(4),
            )
            .unwrap();
            let response = reply(&first, &m2);
            client
                .ingest(&response, SERVER, 1, no_entropy, |_| {
                    assert!(!critical || stage != 2);
                    true
                })
                .unwrap();
            if !critical || stage == 4 {
                client.flush(&mut ci, 1).unwrap();
                let third = ci.last();
                let m3 = Message::from_slice(&decode(&third).unwrap().payload()[1..]).unwrap();
                let (server, m4) = responder
                    .receive_message_3_with_ead(
                        &m3,
                        if stage == 4 { &data } else { &empty },
                        |_, _| false,
                        |_| true,
                    )
                    .unwrap();
                client
                    .ingest(&reply(&third, &m4), SERVER, 2, no_entropy, |_| {
                        assert!(!critical);
                        true
                    })
                    .unwrap();
                if !critical {
                    let client = client.take_session().unwrap();
                    let (client, _) = client.into_parts();
                    let (server, _) = server.into_parts();
                    assert_eq!(client.sender_key(), server.recipient_key());
                    continue;
                }
            }
            assert!(client.take_session().is_none());
            ci.short = true;
            assert_eq!(client.flush(&mut ci, 3), Err(PollError::ShortSend));
            let saved = ci.last();
            let error = decode(&saved).unwrap();
            assert_eq!(error.code(), Code::POST);
            assert_eq!(error.payload()[0], ConnectionId::RESPONDER.as_u8().unwrap());
            assert_eq!(
                PeerError::parse(&error.payload()[1..]).unwrap().code(),
                PeerErrorCode::Unspecified
            );
            ci.short = false;
            assert_eq!(
                client.flush(&mut ci, 4),
                Err(PollError::Provisioning(Error::Profile))
            );
            assert_eq!(ci.last(), saved);
        }
    }
}
