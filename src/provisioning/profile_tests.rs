use super::*;

fn start(suite: CipherSuite) -> (Initiator, Message) {
    Initiator::start_with_suite(
        &identity(1, 0),
        identity(2, 1).peer(),
        ConnectionId::INITIATOR,
        suite,
        entropy(3),
    )
    .unwrap()
}

#[test]
fn both_profiles_authenticate_registry_peers_and_derive_matching_oscore_keys() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let mut previous = None;
    for suite in [CipherSuite::Suite2, CipherSuite::Suite3] {
        let (initiator, m1) = start(suite);
        assert_eq!(m1.as_bytes()[1], suite.label());
        let (responder, m2) =
            RegistryResponder::receive_message_1(&server, ConnectionId::RESPONDER, &m1, entropy(4))
                .unwrap();
        let (initiator, m3) = initiator
            .receive_message_2(&m2, |p| *p == server.peer().principal())
            .unwrap();
        let (server_session, m4) = responder
            .receive_message_3(
                &m3,
                |reference| {
                    assert_eq!(reference.kid(), 0);
                    Some(client.peer())
                },
                |p| *p == client.peer().principal(),
            )
            .unwrap();
        assert_eq!(m4.as_bytes()[0] as usize, 0x40 + suite.tag_len());
        let client_session = initiator
            .receive_message_4(&m4, |p| *p == server.peer().principal())
            .unwrap();
        assert_eq!(client_session.cipher_suite(), suite);
        assert_eq!(server_session.cipher_suite(), suite);
        let (mut client_context, _) = client_session.into_parts();
        let (mut server_context, _) = server_session.into_parts();
        assert_eq!(client_context.sender_key(), server_context.recipient_key());
        assert_eq!(client_context.recipient_key(), server_context.sender_key());
        let key = *client_context.sender_key();
        if let Some(old) = previous {
            assert_ne!(key, old);
        }
        previous = Some(key);
        use crate::message::{Code, Message as CoapMessage, MessageId, Type, decode};
        let request = CoapMessage::new(Type::Confirmable, Code::POST, MessageId::new(90))
            .with_payload(b"enrolled telemetry");
        let mut wire = [0; 256];
        let len = client_context.protect_request(&request, &mut wire).unwrap();
        let mut inner = [0; 256];
        let (decoded, _) = server_context
            .unprotect_request(&decode(&wire[..len]).unwrap(), &mut inner)
            .unwrap();
        assert_eq!(decoded.payload(), b"enrolled telemetry");
    }
}

#[test]
fn suite_selection_cannot_skip_a_supported_preference_or_allocate_on_invalid_cbor() {
    let (_, original) = start(CipherSuite::Suite3);
    for suites in [
        &[0x82, 4, 3][..],
        &[0x82, 0x20, 3],
        &[0x83, 4, 5, 2],
        &[0x89, 4, 5, 6, 7, 8, 9, 10, 11, 3],
    ] {
        let mut wire = std::vec![3];
        wire.extend_from_slice(suites);
        wire.extend_from_slice(&original.as_bytes()[2..]);
        assert!(
            RegistryResponder::receive_message_1(
                &identity(2, 1),
                ConnectionId::RESPONDER,
                &Message::from_slice(&wire).unwrap(),
                entropy(4),
            )
            .is_ok()
        );
    }
    for suites in [
        &[0x82, 2, 3][..],
        &[0x82, 3, 2],
        &[0x82, 3, 3],
        &[0x82, 2, 2],
        &[0x80],
        &[0x81, 3],
        &[0x18, 3],
        &[0x98, 2, 4, 3],
        &[0x8a, 4, 5, 6, 7, 8, 9, 10, 11, 12, 3],
        &[0x9f, 4, 3, 0xff],
    ] {
        let mut wire = std::vec![3];
        wire.extend_from_slice(suites);
        wire.extend_from_slice(&original.as_bytes()[2..]);
        assert!(
            RegistryResponder::receive_message_1(
                &identity(2, 1),
                ConnectionId::RESPONDER,
                &Message::from_slice(&wire).unwrap(),
                |_| panic!("invalid profile must fail before entropy"),
            )
            .is_err(),
            "{suites:?}"
        );
    }
    for bytes in [
        &[0x18, 3, 3][..],    // nonpreferred METHOD
        &[3, 3, 0x59, 0, 32], // nonpreferred G_X length
    ] {
        let mut wire = bytes.to_vec();
        wire.extend_from_slice(&original.as_bytes()[if bytes.len() == 3 { 2 } else { 4 }..]);
        assert!(
            RegistryResponder::receive_message_1(
                &identity(2, 1),
                ConnectionId::RESPONDER,
                &Message::from_slice(&wire).unwrap(),
                |_| panic!("nonpreferred encoding must fail before entropy"),
            )
            .is_err()
        );
    }
}

#[test]
fn suite_rewriting_and_prior_preferences_remain_bound_to_the_original_transcript() {
    for selected in [2, 3] {
        let (initiator, m1) = start(CipherSuite::Suite3);
        let mut wire = std::vec![3, 0x82, 4, selected];
        wire.extend_from_slice(&m1.as_bytes()[2..]);
        let (_, m2) = RegistryResponder::receive_message_1(
            &identity(2, 1),
            ConnectionId::RESPONDER,
            &Message::from_slice(&wire).unwrap(),
            entropy(4),
        )
        .unwrap();
        assert!(
            initiator
                .receive_message_2(&m2, |_| panic!("changed transcript cannot authorize"))
                .is_err()
        );
    }
}

#[test]
fn suite3_rejects_truncated_or_corrupt_macs_and_confirmation_before_authorization() {
    for stage in 2..=4 {
        for mutation in 0..2 {
            let (initiator, m1) = start(CipherSuite::Suite3);
            let (responder, m2) = Responder::receive_message_1(
                &identity(2, 1),
                identity(1, 0).peer(),
                &m1,
                entropy(4),
            )
            .unwrap();
            let corrupt = |message: &Message| {
                let mut bytes = message.as_bytes().to_vec();
                if mutation == 0 {
                    bytes.pop();
                } else {
                    *bytes.last_mut().unwrap() ^= 1;
                }
                Message::from_slice(&bytes).unwrap()
            };
            if stage == 2 {
                assert!(
                    initiator
                        .receive_message_2(&corrupt(&m2), |_| panic!("bad MAC cannot authorize"))
                        .is_err()
                );
                continue;
            }
            let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
            if stage == 3 {
                assert!(
                    responder
                        .receive_message_3(&corrupt(&m3), |_| panic!("bad MAC cannot authorize"))
                        .is_err()
                );
                continue;
            }
            let (_, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
            assert!(
                initiator
                    .receive_message_4(&corrupt(&m4), |_| panic!("bad M4 cannot hand off"))
                    .is_err()
            );
        }
    }
}
