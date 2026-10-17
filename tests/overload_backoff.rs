#![cfg(feature = "std")]

use coaptic::app::Error;
use coaptic::storage::UdpSocketIo;
use coaptic::{App, Code, Response, get, profiles};
use std::net::UdpSocket;
use std::time::{Duration, Instant};

#[test]
fn udp_overload_backoff_preserves_other_resource_progress() {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let peer = socket.local_addr().unwrap().into();
    socket.set_nonblocking(true).unwrap();
    let client_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    client_socket.set_nonblocking(true).unwrap();
    let mut server = App::profile::<profiles::Default>()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .block_wise::<false>()
        .route("busy", get(|_| Response::too_many_requests(2)))
        .route("ready", get(|_| Response::content(b"ready")))
        .allow_unprotected()
        .bind(UdpSocketIo::new(socket, [0; 1473]).unwrap())
        .unwrap();
    let mut client = App::profile::<profiles::Default>()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .block_wise::<false>()
        .allow_unprotected()
        .bind(UdpSocketIo::new(client_socket, [0; 1473]).unwrap())
        .unwrap();
    for (path, now, expected) in [
        ("busy", 0, Code::TOO_MANY_REQUESTS),
        ("ready", 1, Code::CONTENT),
        ("busy", 2000, Code::TOO_MANY_REQUESTS),
    ] {
        let call = client.get(path).to(peer).send(now).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            server.poll(now).unwrap();
            client.poll(now).unwrap();
            if let Some(response) = client.take_response(call) {
                assert_eq!(response.unwrap().code(), expected);
                break;
            }
            assert!(Instant::now() < deadline, "loopback response deadline");
            std::thread::yield_now();
        }
        assert!(matches!(
            client.get("busy").to(peer).send(now + 1),
            Err(Error::Backoff { .. })
        ));
    }
}
