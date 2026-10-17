#![cfg(all(feature = "std", feature = "oscore"))]

use coaptic::oscore::{DeriveParams, SecurityContext};
use coaptic::storage::UdpSocketIo;
use coaptic::{App, Endpoint, Response, get, resources};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

fn socket() -> (UdpSocketIo<[u8; 1473]>, Endpoint) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let endpoint = socket.local_addr().unwrap().into();
    socket.set_nonblocking(true).unwrap();
    (UdpSocketIo::new(socket, [0; 1473]).unwrap(), endpoint)
}
fn context(secret: u8, sender: &[u8], recipient: &[u8]) -> SecurityContext {
    SecurityContext::derive(DeriveParams {
        master_secret: &[secret; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}
// Two simultaneous response snapshots require two TX bodies; every other
// dimension is the Constrained profile's existing bound.
coaptic::memory_profile! { ConstrainedTwoBodies {
    rx_datagram: (2, 1152), tx_datagram: (2, 1152),
    rx_body: (1, 4096), tx_body: (2, 4096), dedup: 4, observe: 2,
} }
const BODY: [u8; 2048] = [0x5a; 2048];

#[test]
fn udp_two_owners_share_constrained_pools_and_finish_blockwise_and_observe() {
    let (io, server_endpoint) = socket();
    let mut server = App::with_resources(
        resources::Resources::<ConstrainedTwoBodies>::new()
            .routes::<4>()
            .client_calls::<1>()
            .associations::<2>()
            .block_wise::<true>(),
    )
    .randomness(|bytes| getrandom::fill(bytes).is_ok())
    .require_associations()
    .route(
        "large",
        get(|request| {
            assert!(request.association().is_some());
            Response::new(coaptic::Code::CONTENT)
                .payload_borrowed(&BODY)
                .etag(b"body")
        }),
    )
    .route("observe", get(|_| Response::content(b"initial").observe(0)))
    .bind(io)
    .unwrap();
    let (a_io, a_endpoint) = socket();
    let (b_io, b_endpoint) = socket();
    let mut a = App::with_resources(resources::SmallDevice::new().block_wise::<true>())
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .oscore(context(1, &[1], &[0]))
        .bind(a_io)
        .unwrap();
    let mut b = App::with_resources(resources::SmallDevice::new().block_wise::<true>())
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .oscore(context(2, &[2], &[0]))
        .bind(b_io)
        .unwrap();
    let owner_a = server
        .install_association(a_endpoint, [1; 32], 1, &mut Some(context(1, &[0], &[1])))
        .unwrap();
    let owner_b = server
        .install_association(b_endpoint, [2; 32], 2, &mut Some(context(2, &[0], &[2])))
        .unwrap();
    let ca = a.get("large").to(server_endpoint).send(0).unwrap();
    let cb = b.get("large").to(server_endpoint).send(0).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut done = [false; 2];
    while !done.iter().all(|value| *value) {
        server.poll(1).unwrap();
        a.poll(1).unwrap();
        b.poll(1).unwrap();
        for (index, (client, call)) in [(&mut a, ca), (&mut b, cb)].into_iter().enumerate() {
            if !done[index] {
                let mut output = [0; BODY.len()];
                if let Some(reply) = client.take_response_into(call, &mut output).unwrap() {
                    let reply = reply.unwrap();
                    assert_eq!(reply.payload().len(), BODY.len());
                    assert_eq!(reply.payload(), BODY);
                    done[index] = true;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "both peers must complete under contention"
        );
        std::thread::yield_now();
    }
    let ca = a
        .get("observe")
        .to(server_endpoint)
        .observe()
        .send(2)
        .unwrap();
    let cb = b
        .get("observe")
        .to(server_endpoint)
        .observe()
        .send(2)
        .unwrap();
    let mut done = [false; 2];
    while !done.iter().all(|value| *value) {
        server.poll(3).unwrap();
        a.poll(3).unwrap();
        b.poll(3).unwrap();
        for (index, (client, call)) in [(&mut a, ca), (&mut b, cb)].into_iter().enumerate() {
            if !done[index] {
                if let Some(reply) = client.take_response(call) {
                    assert_eq!(reply.unwrap().payload(), b"initial");
                    done[index] = true;
                }
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        server
            .notify(4, &["observe"], Response::content(b"updated"))
            .unwrap(),
        2
    );
    let mut notified = [false; 2];
    while !notified.iter().all(|value| *value) {
        a.poll(5).unwrap();
        b.poll(5).unwrap();
        for (index, (client, call)) in [(&mut a, ca), (&mut b, cb)].into_iter().enumerate() {
            if !notified[index] {
                if let Some(reply) = client.take_response(call) {
                    assert_eq!(reply.unwrap().payload(), b"updated");
                    notified[index] = true;
                }
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    server.remove_association(owner_a).unwrap();
    assert!(server.association_is_active(owner_b));
    assert!(!server.association_is_active(owner_a));
    server.poll(6).unwrap();
    assert!(a.cancel(ca));
    assert!(b.cancel(cb));
}
