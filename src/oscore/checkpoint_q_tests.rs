use super::{Loopback, client_c1, server_c1};
use crate::message::{BlockValue, Code, Message, MessageId, Opt, Type, decode, encode_uint};
use crate::oscore::ReplayCheckpoint;
use crate::storage::Endpoint;
use crate::{App, profiles};

#[test]
fn q_response_replay_changes_are_persisted_before_subsequent_poll_traffic() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut client = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .require_oscore_checkpoint()
        .oscore(client_c1())
        .bind(Loopback::default())
        .unwrap();
    let mut durable = ReplayCheckpoint::from_parts(0, 0).unwrap();
    client
        .poll_with_oscore_checkpoint(0, |checkpoint| {
            durable = checkpoint;
            true
        })
        .unwrap();
    let call = client.get("large").q_block2().to(peer).send(0).unwrap();
    let (_, bytes, n) = client.transport_mut().last_send.take().unwrap();
    let request = decode(&bytes[..n]).unwrap();
    let mut server = server_c1();
    let mut scratch = [0; 256];
    let (_, request_ref) = server.unprotect_request(&request, &mut scratch).unwrap();
    let body = b"0123456789abcdefghijklmnopqrstuv";
    for num in 0..2 {
        let block = BlockValue::from_size(num, num == 0, 16).unwrap().encode();
        let size = encode_uint(32);
        let opts = [Opt::etag(b"v1"), Opt::size2(&size), Opt::q_block2(&block)];
        let response = Message::new(
            if num == 0 {
                Type::Acknowledgement
            } else {
                Type::NonConfirmable
            },
            Code::CONTENT,
            if num == 0 {
                request.message_id()
            } else {
                MessageId::new(900)
            },
        )
        .with_token(call.token())
        .with_options(&opts)
        .with_payload(&body[num as usize * 16..num as usize * 16 + 16]);
        let mut wire = [0; 256];
        let n = if num == 0 {
            server.protect_response(&response, request_ref, &mut wire)
        } else {
            server.protect_response_with_piv(&response, request_ref, &mut wire)
        }
        .unwrap();
        client.transport_mut().inbox = Some((peer, wire, n));
        client
            .poll_with_oscore_checkpoint(num as u64 + 1, |_| {
                panic!("response admission is not a durable request barrier")
            })
            .unwrap();
    }
    let response = client.take_response(call).unwrap().unwrap();
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.body(), Some(body.as_slice()));
    assert_eq!(response.etag_bytes(), Some(b"v1".as_slice()));
    let accepted = client.oscore().unwrap().replay_checkpoint();
    assert_ne!(accepted, durable);
    assert_eq!(
        client.poll_with_oscore_checkpoint(3, |checkpoint| {
            assert_eq!(checkpoint, accepted);
            false
        }),
        Err(crate::Error::OscoreCheckpointFailed)
    );
    client
        .poll_with_oscore_checkpoint(4, |checkpoint| {
            durable = checkpoint;
            true
        })
        .unwrap();
    assert_eq!(durable, accepted);
    client
        .poll_with_oscore_checkpoint(5, |_| panic!("clean idle poll does not persist"))
        .unwrap();
}
