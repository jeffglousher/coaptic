extern crate std;

use super::*;
use crate::provisioning::{
    CipherSuite, ConnectionId, Identity, Initiator, RegistryResponder, Responder,
};

fn hex(value: &str) -> std::vec::Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn rfc9529_published_exporter_vectors() {
    // RFC 9529 sections 2.5/2.6 and 3.7/3.8. These are published derivation
    // vectors, not a claim to support the first trace's authentication method.
    for (prk, secret, salt) in [
        (
            "2aaec8fc4ab3bc3295def6b551051a2fa561424db301fa84f642f5578a6df51a",
            "1e1c6beac3a8a1cac435de7e2f9ae7ff",
            "ce7ab844c0106d73",
        ),
        (
            "e14d06699cee248c5a04bf9227bbcd4ce394de7dcb56db43555474171e6446db",
            "f9868f6a3aca78a05d1485b35030b162",
            "ada24c7dbfc85eeb",
        ),
    ] {
        let exporter = Exporter::new(hex(prk).try_into().unwrap());
        let mut actual_secret = [0; 16];
        let mut actual_salt = [0; 8];
        exporter.export(0, &[], &mut actual_secret).unwrap();
        exporter.export(1, &[], &mut actual_salt).unwrap();
        assert_eq!(actual_secret, hex(secret).as_slice());
        assert_eq!(actual_salt, hex(salt).as_slice());
    }
}

#[test]
fn full_width_labels_and_borrowed_contexts_match_independent_hmac_vectors() {
    // Independent .NET HMAC-SHA256 expansion with literal preferred-CBOR
    // headers; PRK is bytes 00..1f and each context byte is 5a. Contexts cross
    // the old handshake buffer limit without extra exporter storage.
    let exporter = Exporter::new(core::array::from_fn(|i| i as u8));
    for (label, context_len, expected) in [
        (
            24,
            24,
            "dd82f9ddb292b2f2e81bbf256b3f69d009c7544b4d0f454d8470f32f4031abefa9",
        ),
        (
            256,
            256,
            "b52442de1aadaca98b65e4dedd12b71c3c1783782a1bc18f9034fb50f4e650fd8cff977abf6a5ebdef1720712f99015947fd02c42afe74ae06d65565479a37c9",
        ),
        (
            65536,
            65536,
            "854880e860e30f9e6147d4ae9efa9e5f1a247b289431a7ff8878b73754f0f078cad9fc7d8c202cd3edfc790d382380987ffc57522b7910965cf61f9fd5bcc13d",
        ),
        (
            1 << 32,
            0,
            "039779ced0847e79e89c8694ed75113759278b29bfa6c93d2e4d6bca07a55274",
        ),
        (
            u64::MAX,
            1,
            "dd956e4599c9df3ecfa99b5c36d47741f230ead32e3667727b599fe6723772ab",
        ),
    ] {
        let context = std::vec![0x5a; context_len];
        let expected = hex(expected);
        let mut output = std::vec![0; expected.len()];
        exporter.export(label, &context, &mut output).unwrap();
        assert_eq!(output, expected);
    }
}

#[test]
fn output_limit_refuses_without_mutation_and_valid_exports_recover() {
    let exporter = Exporter::new([0x35; 32]);
    let mut initial = [0; 32];
    exporter.export(2, b"purpose", &mut initial).unwrap();
    let mut too_long = [0xa5; Exporter::MAX_OUTPUT_LEN + 1];
    assert_eq!(
        exporter.export(2, b"purpose", &mut too_long),
        Err(ExporterError::OutputTooLong)
    );
    assert!(too_long.iter().all(|byte| *byte == 0xa5));
    let mut maximum = [0; Exporter::MAX_OUTPUT_LEN];
    exporter.export(2, b"purpose", &mut maximum).unwrap();
    // Independent .NET HMAC expansion, including all 255 output blocks.
    use sha2::Digest;
    assert_eq!(
        Sha256::digest(maximum).as_slice(),
        hex("7a1a2b4c41a06979418a5c62020ab13030866af7125aab2dfa766b4d975914de")
    );
    assert_ne!(&maximum[..32], &initial);
    exporter.export(u64::MAX, b"", &mut []).unwrap();
    let mut recovered = [0; 32];
    exporter.export(2, b"purpose", &mut recovered).unwrap();
    assert_eq!(initial, recovered);
    for (label, context) in [(3, &b"purpose"[..]), (2, &b"other"[..])] {
        exporter.export(label, context, &mut recovered).unwrap();
        assert_ne!(initial, recovered);
    }
    assert_eq!(std::format!("{exporter:?}"), "Exporter { .. }");
}

fn identity(scalar: u8, kid: u8) -> Identity {
    let mut private = [0; 32];
    private[31] = scalar;
    Identity::from_private_key(private, kid).unwrap()
}

fn entropy(scalar: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |bytes| {
        bytes.fill(0);
        bytes[31] = scalar;
        true
    }
}

#[test]
fn both_suites_and_responder_paths_keep_exporter_through_oscore_handoff() {
    use crate::oscore::{DeriveParams, SecurityContext};

    let client = identity(1, 0);
    let server = identity(2, 1);
    for suite in [CipherSuite::Suite2, CipherSuite::Suite3] {
        let mut previous_key = None;
        for registry in [false, true] {
            let mut previous_session = None;
            for fresh in [3, 5] {
                let (initiator, m1) = Initiator::start_with_suite(
                    &client,
                    server.peer(),
                    ConnectionId::INITIATOR,
                    suite,
                    entropy(fresh),
                )
                .unwrap();
                let (pinned, lookup, m2) = if registry {
                    let (responder, m2) = RegistryResponder::receive_message_1(
                        &server,
                        ConnectionId::RESPONDER,
                        &m1,
                        entropy(fresh + 1),
                    )
                    .unwrap();
                    (None, Some(responder), m2)
                } else {
                    let (responder, m2) = Responder::receive_message_1(
                        &server,
                        client.peer(),
                        &m1,
                        entropy(fresh + 1),
                    )
                    .unwrap();
                    (Some(responder), None, m2)
                };
                let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
                let (server_session, m4) = match pinned {
                    Some(responder) => responder.receive_message_3(&m3, |_| true).unwrap(),
                    None => lookup
                        .unwrap()
                        .receive_message_3(&m3, |_| Some(client.peer()), |_| true)
                        .unwrap(),
                };
                let client_session = initiator.receive_message_4(&m4, |_| true).unwrap();
                let mut expected_key = [0; 32];
                client_session
                    .exporter()
                    .export(2, b"application purpose", &mut expected_key)
                    .unwrap();
                let mut secret = [0; 16];
                let mut salt = [0; 8];
                client_session
                    .exporter()
                    .export(0, &[], &mut secret)
                    .unwrap();
                client_session.exporter().export(1, &[], &mut salt).unwrap();
                let expected_context = SecurityContext::derive(DeriveParams {
                    master_secret: &secret,
                    master_salt: &salt,
                    sender_id: ConnectionId::RESPONDER.as_bytes(),
                    recipient_id: ConnectionId::INITIATOR.as_bytes(),
                    id_context: &[],
                })
                .unwrap();
                let (client_context, principal, exporter) =
                    client_session.into_parts_with_exporter();
                assert_eq!(principal, server.peer().principal());
                assert_eq!(client_context.sender_key(), expected_context.sender_key());
                assert_eq!(client_context.common_iv(), expected_context.common_iv());
                let (server_context, principal, peer_exporter) =
                    server_session.into_parts_with_exporter();
                assert_eq!(principal, client.peer().principal());
                assert_eq!(client_context.sender_key(), server_context.recipient_key());
                let mut actual_key = [0; 32];
                exporter
                    .export(2, b"application purpose", &mut actual_key)
                    .unwrap();
                assert_eq!(actual_key, expected_key);
                peer_exporter
                    .export(2, b"application purpose", &mut actual_key)
                    .unwrap();
                assert_eq!(actual_key, expected_key);
                if let Some(old) = previous_session {
                    assert_ne!(actual_key, old);
                }
                previous_session = Some(actual_key);
                if fresh == 3 {
                    if let Some(old) = previous_key {
                        // Deferred lookup changes no transcript or key material.
                        assert_eq!(actual_key, old);
                    }
                    previous_key = Some(actual_key);
                }
            }
        }
    }
}
