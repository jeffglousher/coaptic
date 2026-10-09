extern crate std;

use super::*;

fn identity(value: u8, kid: u8) -> Identity {
    let mut private = [0; 32];
    private[31] = value;
    Identity::from_private_key(private, kid).unwrap()
}

fn entropy(value: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |out| {
        out.fill(0);
        out[31] = value;
        true
    }
}

fn handshake(a: u8, b: u8) -> (Session, Session) {
    handshake_with_ids(a, b, ConnectionId::INITIATOR, ConnectionId::RESPONDER)
}

fn handshake_with_ids(
    a: u8,
    b: u8,
    initiator_id: ConnectionId,
    responder_id: ConnectionId,
) -> (Session, Session) {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let (initiator, m1) =
        Initiator::start_with_id(&client, server.peer(), initiator_id, entropy(a)).unwrap();
    assert_eq!(m1.as_bytes()[36], initiator_id.as_u8());
    let (responder, m2) =
        Responder::receive_message_1_with_id(&server, client.peer(), responder_id, &m1, entropy(b))
            .unwrap();
    let (initiator, m3) = initiator
        .receive_message_2(&m2, |p| *p == server.peer().principal())
        .unwrap();
    assert_eq!(initiator.peer_connection_id(), responder_id);
    let (server_session, m4) = responder
        .receive_message_3(&m3, |p| *p == client.peer().principal())
        .unwrap();
    let client_session = initiator
        .receive_message_4(&m4, |p| *p == server.peer().principal())
        .unwrap();
    (client_session, server_session)
}

#[test]
fn compact_connection_ids_validate_all_values() {
    for value in 0..=u8::MAX {
        match ConnectionId::new(value) {
            Ok(id) => {
                assert!(value <= 23);
                assert_eq!(id.as_u8(), value);
                assert_eq!(id.lakers().as_cbor(), &[value]);
                assert_eq!(ConnectionId::from_lakers(id.lakers()), Ok(id));
            }
            Err(error) => {
                assert!(value > 23);
                assert_eq!(error, Error::Profile);
            }
        }
    }
}

#[test]
fn connection_id_decoder_checks_storage_and_truncation() {
    for length in 0..=23 {
        let mut encoded = std::vec![0x40 + length];
        encoded.extend(core::iter::repeat_n(0xff, usize::from(length)));
        let mut decoder = lakers::CBORDecoder::new(&encoded);
        let result = lakers::ConnId::from_decoder(&mut decoder);
        if length <= 7 {
            let id = result.unwrap();
            assert_eq!(id.as_cbor(), encoded);
            assert_eq!(id.as_slice(), &encoded[1..]);
            assert!(decoder.finished());
        } else {
            assert!(result.is_err(), "connection ID length {length}");
            assert_eq!(decoder.position(), 0);
        }
        for end in 0..encoded.len() {
            assert!(
                lakers::ConnId::from_decoder(&mut lakers::CBORDecoder::new(&encoded[..end]))
                    .is_err(),
                "connection ID length {length}, truncated at {end}"
            );
        }
    }
}

#[test]
fn malformed_response_connection_id_is_refused_before_authorization() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let (initiator, m1) = Initiator::start(&client, server.peer(), entropy(3)).unwrap();
    let (_, m2) = Responder::receive_message_1(&server, client.peer(), &m1, entropy(4)).unwrap();
    let mut bytes = m2.as_bytes().to_vec();
    let combined_len = lakers::CBORDecoder::new(&bytes).bytes().unwrap().len();
    let ciphertext_start = bytes.len() - combined_len + lakers::P256_ELEM_LEN;
    // XOR encryption lets an unauthenticated response replace C_R's first byte
    // with an eight-byte bstr header, exceeding ConnId's seven-byte ID capacity.
    bytes[ciphertext_start] ^= ConnectionId::RESPONDER.as_u8() ^ 0x48;
    assert!(matches!(
        initiator.receive_message_2(&Message::from_slice(&bytes).unwrap(), |_| panic!(
            "malformed response must not authorize"
        )),
        Err(Error::Parsing)
    ));
}

#[test]
fn peer_selected_connection_ids_derive_correct_oscore_directions() {
    use crate::message::{Code, Message as CoapMessage, MessageId, Token, Type, decode};

    for (initiator_id, responder_id) in [(2, 3), (22, 23), (1, 0)] {
        let (client, server) = handshake_with_ids(
            3,
            4,
            ConnectionId::new(initiator_id).unwrap(),
            ConnectionId::new(responder_id).unwrap(),
        );
        let (mut client, _) = client.into_parts();
        let (mut server, _) = server.into_parts();
        assert_eq!(client.sender_id(), &[responder_id]);
        assert_eq!(client.recipient_id(), &[initiator_id]);
        assert_eq!(server.sender_id(), &[initiator_id]);
        assert_eq!(server.recipient_id(), &[responder_id]);
        assert_eq!(client.sender_key(), server.recipient_key());
        assert_eq!(client.recipient_key(), server.sender_key());
        assert_eq!(client.common_iv(), server.common_iv());
        let request = CoapMessage::new(Type::Confirmable, Code::POST, MessageId::new(55))
            .with_token(Token::from_checked(&[9, 8]))
            .with_payload(b"authenticated dynamic connection IDs");
        let mut wire = [0; 256];
        let length = client.protect_request(&request, &mut wire).unwrap();
        let mut inner = [0; 256];
        let (plain, _) = server
            .unprotect_request(&decode(&wire[..length]).unwrap(), &mut inner)
            .unwrap();
        assert_eq!(plain.code(), request.code());
        assert_eq!(plain.ty(), request.ty());
        assert_eq!(plain.message_id(), request.message_id());
        assert_eq!(plain.token(), request.token());
        assert_eq!(plain.payload(), request.payload());
    }
}

#[test]
fn responder_refuses_colliding_and_unsupported_connection_ids_before_entropy() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let local_id = ConnectionId::new(7).unwrap();
    let (_, m1) = Initiator::start_with_id(&client, server.peer(), local_id, entropy(3)).unwrap();
    for peer_id in [7, 24, 0x20, 0x40, 0xff] {
        let mut bytes = m1.as_bytes().to_vec();
        bytes[36] = peer_id;
        assert!(matches!(
            Responder::receive_message_1_with_id(
                &server,
                client.peer(),
                local_id,
                &Message::from_slice(&bytes).unwrap(),
                |_| panic!("unsupported or colliding IDs must not request entropy")
            ),
            Err(Error::Profile)
        ));
    }
}

#[test]
fn initiator_refuses_authenticated_peer_collisions_and_unsupported_ids() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let local_id = ConnectionId::new(7).unwrap();
    for peer_bytes in [
        &[7][..],
        &[0x41, 24],
        &[0x20],
        &[0x42, 1, 2],
        &[0x40],
        &[0x41, 1],
    ] {
        let (initiator, m1) =
            Initiator::start_with_id(&client, server.peer(), local_id, entropy(3)).unwrap();
        let responder = lakers::EdhocResponder::new(
            Crypto::fresh(entropy(4)).unwrap(),
            EDHOCMethod::StatStat,
            server.scalar(),
            server.credential(),
        );
        let (responder, _, _) = responder.process_message_1(&m1.buffer().unwrap()).unwrap();
        let (_, message) = responder
            .prepare_message_2(
                CredentialTransfer::ByReference,
                Some(
                    lakers::ConnId::from_decoder(&mut lakers::CBORDecoder::new(peer_bytes))
                        .unwrap(),
                ),
                &None,
            )
            .unwrap();
        assert!(matches!(
            initiator.receive_message_2(&Message::from_buffer(&message).unwrap(), |_| panic!(
                "invalid peer ID must not reach authorization"
            )),
            Err(Error::Profile)
        ));
    }
}

#[test]
fn fresh_handshake_authenticates_both_pins() {
    let (client, server) = handshake(3, 4);
    assert_eq!(*client.principal(), identity(2, 1).peer().principal());
    assert_eq!(*server.principal(), identity(1, 0).peer().principal());
}

#[test]
fn wrong_pin_or_revoked_authorization_is_refused() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let (initiator, m1) = Initiator::start(&client, identity(5, 1).peer(), entropy(3)).unwrap();
    let (_, m2) = Responder::receive_message_1(&server, client.peer(), &m1, entropy(4)).unwrap();
    assert!(initiator.receive_message_2(&m2, |_| true).is_err());
    let (initiator, m1) = Initiator::start(&client, server.peer(), entropy(3)).unwrap();
    let (_, m2) = Responder::receive_message_1(&server, client.peer(), &m1, entropy(4)).unwrap();
    assert!(matches!(
        initiator.receive_message_2(&m2, |_| false),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn invalid_ephemeral_point_and_profile_are_refused_before_entropy() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let (_, m1) = Initiator::start(&client, server.peer(), entropy(3)).unwrap();
    let mut bytes = m1.as_bytes().to_vec();
    bytes[4..36].fill(0xff);
    assert!(matches!(
        Responder::receive_message_1(
            &server,
            client.peer(),
            &Message::from_slice(&bytes).unwrap(),
            |_| panic!("entropy must not run")
        ),
        Err(Error::InvalidKey)
    ));
    bytes[0] = 0;
    assert!(matches!(
        Responder::receive_message_1(
            &server,
            client.peer(),
            &Message::from_slice(&bytes).unwrap(),
            |_| panic!("entropy must not run")
        ),
        Err(Error::Profile)
    ));
}

#[test]
fn self_peer_is_refused_even_with_a_different_kid() {
    let client = identity(1, 0);
    assert!(matches!(
        Initiator::start(&client, identity(1, 1).peer(), |_| panic!(
            "entropy must not run"
        )),
        Err(Error::SelfPeer)
    ));
}

#[test]
fn message_capacity_and_redacted_state() {
    assert!(Message::from_slice(&[]).is_err());
    assert!(Message::from_slice(&[0; Message::CAPACITY + 1]).is_err());
    let client = identity(1, 0);
    let (initiator, _) = Initiator::start(&client, identity(2, 1).peer(), entropy(3)).unwrap();
    assert_eq!(std::format!("{initiator:?}"), "Initiator { .. }");
}

#[test]
fn protected_payload_metadata_replay_and_restart() {
    use crate::message::{
        Code, Message as CoapMessage, MessageId, Opt, OptionsBuilder, Token, Type, decode,
    };

    let (client, server) = handshake(3, 4);
    let (mut client, _) = client.into_parts();
    let (mut server, _) = server.into_parts();
    assert_eq!(client.sender_key(), server.recipient_key());
    assert_eq!(client.recipient_key(), server.sender_key());
    assert_eq!(client.common_iv(), server.common_iv());
    let mut options = OptionsBuilder::<2>::new();
    options.push(Opt::uri_path("provisioned")).unwrap();
    options.push(Opt::uri_query("complete=true")).unwrap();
    let token = Token::from_checked(&[1, 3, 5, 7]);
    let request = CoapMessage::new(Type::Confirmable, Code::PUT, MessageId::new(54321))
        .with_token(token)
        .with_options(options.as_slice())
        .with_payload(b"complete request body");
    let mut wire = [0; 256];
    let len = client.protect_request(&request, &mut wire).unwrap();
    let protected = decode(&wire[..len]).unwrap();
    let mut inner = [0; 256];
    let (plain, reference) = server.unprotect_request(&protected, &mut inner).unwrap();
    assert_eq!(plain.code(), request.code());
    assert_eq!(plain.ty(), request.ty());
    assert_eq!(plain.token(), token);
    assert_eq!(plain.message_id(), request.message_id());
    assert_eq!(plain.payload(), request.payload());
    let actual: std::vec::Vec<_> = plain.options().collect();
    assert_eq!(actual.as_slice(), options.as_slice());
    assert!(server.unprotect_request(&protected, &mut inner).is_err());
    let response = CoapMessage::new(Type::Acknowledgement, Code::CHANGED, request.message_id())
        .with_token(token)
        .with_payload(b"complete response body");
    let mut reply = [0; 256];
    let reply_len = server
        .protect_response(&response, reference, &mut reply)
        .unwrap();
    let opened = client
        .unprotect_response(&decode(&reply[..reply_len]).unwrap(), reference, &mut inner)
        .unwrap();
    assert_eq!(opened.code(), response.code());
    assert_eq!(opened.ty(), response.ty());
    assert_eq!(opened.message_id(), response.message_id());
    assert_eq!(opened.token(), token);
    assert_eq!(opened.payload(), response.payload());
    let (new_client, new_server) = handshake(5, 6);
    let (new_client, _) = new_client.into_parts();
    let (mut new_server, _) = new_server.into_parts();
    assert_ne!(new_client.sender_key(), client.sender_key());
    assert_eq!(
        new_server
            .unprotect_request(&protected, &mut inner)
            .unwrap_err(),
        crate::oscore::Error::Decrypt
    );
}

#[test]
fn final_confirmation_corruption_and_revocation_are_refused() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    for revoked in [false, true] {
        let (initiator, m1) = Initiator::start(&client, server.peer(), entropy(3)).unwrap();
        let (responder, m2) =
            Responder::receive_message_1(&server, client.peer(), &m1, entropy(4)).unwrap();
        let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
        let (_, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
        let mut bytes = m4.as_bytes().to_vec();
        if !revoked {
            *bytes.last_mut().unwrap() ^= 1;
        }
        assert!(
            initiator
                .receive_message_4(&Message::from_slice(&bytes).unwrap(), |_| !revoked)
                .is_err()
        );
    }
}

#[test]
fn truncated_and_oversized_containers_are_refused_without_panics() {
    let client = identity(1, 0);
    let server = identity(2, 1);
    let (_, m1) = Initiator::start(&client, server.peer(), entropy(3)).unwrap();
    for length in 1..m1.as_bytes().len() {
        assert!(
            Responder::receive_message_1(
                &server,
                client.peer(),
                &Message::from_slice(&m1.as_bytes()[..length]).unwrap(),
                |_| panic!("truncated input must not request entropy")
            )
            .is_err()
        );
    }
    let mut empty_suites = std::vec![3, 0x98, 0];
    empty_suites.extend_from_slice(&m1.as_bytes()[2..]);
    assert!(matches!(
        Responder::receive_message_1(
            &server,
            client.peer(),
            &Message::from_slice(&empty_suites).unwrap(),
            |_| panic!("empty suites must be refused before entropy")
        ),
        Err(Error::Profile)
    ));
    for bytes in [
        &[0x5a, 0xff, 0xff, 0xff, 0xff][..],
        &[0xa0],
        &[0x40],
        &[0x47, 1, 2, 3],
        &[0x48, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ] {
        let (initiator, m1) = Initiator::start(&client, server.peer(), entropy(3)).unwrap();
        let (responder, m2) =
            Responder::receive_message_1(&server, client.peer(), &m1, entropy(4)).unwrap();
        let (initiator, _) = initiator.receive_message_2(&m2, |_| true).unwrap();
        let malformed = Message::from_slice(bytes).unwrap();
        assert!(
            responder
                .receive_message_3(&malformed, |_| panic!("malformed m3 must not authorize"))
                .is_err()
        );
        assert!(
            initiator
                .receive_message_4(&malformed, |_| panic!("malformed m4 must not authorize"))
                .is_err()
        );
    }
}
