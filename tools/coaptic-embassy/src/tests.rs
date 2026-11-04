extern crate std;
use super::*;
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embassy_net_driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, vec, vec::Vec};

type Packets = Rc<RefCell<VecDeque<Vec<u8>>>>;
struct TestDriver(Packets, Packets);
struct Rx(Vec<u8>);
struct Tx(Packets);
impl RxToken for Rx {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, f: F) -> R {
        f(&mut self.0)
    }
}
impl TxToken for Tx {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0; len];
        let result = f(&mut packet);
        self.0.borrow_mut().push_back(packet);
        result
    }
}
impl Driver for TestDriver {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx;
    fn receive(&mut self, _: &mut Context) -> Option<(Rx, Tx)> {
        self.0
            .borrow_mut()
            .pop_front()
            .map(|p| (Rx(p), Tx(self.1.clone())))
    }
    fn transmit(&mut self, _: &mut Context) -> Option<Tx> {
        Some(Tx(self.1.clone()))
    }
    fn link_state(&mut self, _: &mut Context) -> LinkState {
        LinkState::Up
    }
    fn hardware_address(&self) -> HardwareAddress {
        HardwareAddress::Ip
    }
    fn capabilities(&self) -> Capabilities {
        let mut c = Capabilities::default();
        c.max_transmission_unit = 1500;
        c.checksum.ipv4 = embassy_net_driver::Checksum::None;
        c
    }
}
fn packet(body: &[u8]) -> Vec<u8> {
    let mut p = vec![0; 28 + body.len()];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&((28 + body.len()) as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&[10, 0, 0, 2]);
    p[16..20].copy_from_slice(&[10, 0, 0, 1]);
    p[20..22].copy_from_slice(&5684u16.to_be_bytes());
    p[22..24].copy_from_slice(&5683u16.to_be_bytes());
    p[24..26].copy_from_slice(&((8 + body.len()) as u16).to_be_bytes());
    p[28..].copy_from_slice(body);
    p
}
fn poll<F: Future>(f: core::pin::Pin<&mut F>) -> Poll<F::Output> {
    f.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn native_stack_receive_truncation_cancel_and_send_backpressure() {
    let incoming = Rc::new(RefCell::new(VecDeque::new()));
    let outgoing = Rc::new(RefCell::new(VecDeque::new()));
    let mut resources = embassy_net::StackResources::<1>::new();
    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new([10, 0, 0, 1].into(), 24),
        gateway: None,
        dns_servers: Default::default(),
    });
    let (stack, mut runner) = embassy_net::new(
        TestDriver(incoming.clone(), outgoing.clone()),
        config,
        &mut resources,
        17,
    );
    let mut rm = [embassy_net::udp::PacketMetadata::EMPTY; 1];
    let mut tm = [embassy_net::udp::PacketMetadata::EMPTY; 1];
    let mut rb = [0; 128];
    let mut tb = [0; 128];
    let mut socket = UdpSocket::new(stack, &mut rm, &mut rb, &mut tm, &mut tb);
    socket.bind(5683).unwrap();
    let mut io = Udp::new(socket);
    let mut run = pin!(runner.run());
    assert!(poll(run.as_mut()).is_pending());
    let mut out = [0; 8];
    {
        let mut receive = pin!(io.recv(&mut out));
        assert!(poll(receive.as_mut()).is_pending());
    }
    incoming.borrow_mut().push_back(packet(b"too large packet"));
    assert!(poll(run.as_mut()).is_pending());
    assert_eq!(
        poll(pin!(io.recv(&mut out))),
        Poll::Ready(Err(Error::Receive(RecvError::Truncated)))
    );
    incoming.borrow_mut().push_back(packet(b"recovery"));
    assert!(poll(run.as_mut()).is_pending());
    assert_eq!(
        poll(pin!(io.recv(&mut out))),
        Poll::Ready(Ok((8, Endpoint::v4([10, 0, 0, 2], 5684))))
    );
    assert_eq!(&out, b"recovery");
    let dest = Endpoint::v4([10, 0, 0, 2], 5684);
    assert_eq!(poll(pin!(io.send(dest, b"first"))), Poll::Ready(Ok(5)));
    {
        let mut send = pin!(io.send(dest, b"cancelled"));
        assert!(poll(send.as_mut()).is_pending());
    }
    assert!(poll(run.as_mut()).is_pending());
    assert_eq!(poll(pin!(io.send(dest, b"next"))), Poll::Ready(Ok(4)));
    assert!(poll(run.as_mut()).is_pending());
    let sent = outgoing.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(&sent[0][28..], b"first");
    assert_eq!(&sent[1][28..], b"next");
    assert_eq!(io.into_inner().endpoint().port, 5683);
}

#[test]
fn endpoints_and_deadlines_do_not_lose_identity_or_round_early() {
    for ep in [
        Endpoint::v4([127, 0, 0, 1], 5683),
        Endpoint::v6([1; 16], 123),
    ] {
        assert_eq!(peer(destination(ep).unwrap()), ep);
    }
    assert_eq!(
        destination(Endpoint::v6_scoped([1; 16], 5683, 3)),
        Err(Error::UnsupportedScope)
    );
    for ms in [0, 1, 999, 1000, 123456] {
        let ticks = deadline_ticks(ms).unwrap();
        assert!(u128::from(ticks) * 1000 >= u128::from(ms) * u128::from(embassy_time::TICK_HZ));
    }
}

#[test]
fn full_app_blockwise_exchange_and_fresh_recovery() {
    use coaptic::{App, Request, Response, get, resources::SmallDevice};
    use core::future::pending;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct WakeCount(AtomicUsize);
    impl std::task::Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    const BODY: [u8; 2000] = {
        let mut bytes = [0; 2000];
        let mut i = 0;
        while i < bytes.len() {
            bytes[i] = (i % 251) as u8;
            i += 1;
        }
        bytes
    };
    fn content(_: Request<'_>) -> Response<'static> {
        Response::content(&BODY).etag(b"stable")
    }
    let to_server = Rc::new(RefCell::new(VecDeque::new()));
    let to_client = Rc::new(RefCell::new(VecDeque::new()));
    let mut server_resources = embassy_net::StackResources::<1>::new();
    let mut client_resources = embassy_net::StackResources::<1>::new();
    let config = |last| {
        embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
            address: embassy_net::Ipv4Cidr::new([10, 0, 0, last].into(), 24),
            gateway: None,
            dns_servers: Default::default(),
        })
    };
    let (server_stack, mut server_runner) = embassy_net::new(
        TestDriver(to_server.clone(), to_client.clone()),
        config(1),
        &mut server_resources,
        17,
    );
    let (client_stack, mut client_runner) = embassy_net::new(
        TestDriver(to_client.clone(), to_server.clone()),
        config(2),
        &mut client_resources,
        19,
    );
    let mut srm = [embassy_net::udp::PacketMetadata::EMPTY; 2];
    let mut stm = [embassy_net::udp::PacketMetadata::EMPTY; 2];
    let mut crm = [embassy_net::udp::PacketMetadata::EMPTY; 2];
    let mut ctm = [embassy_net::udp::PacketMetadata::EMPTY; 2];
    let mut srb = [0; 2304];
    let mut stb = [0; 2304];
    let mut crb = [0; 2304];
    let mut ctb = [0; 2304];
    let mut ss = UdpSocket::new(server_stack, &mut srm, &mut srb, &mut stm, &mut stb);
    let mut cs = UdpSocket::new(client_stack, &mut crm, &mut crb, &mut ctm, &mut ctb);
    ss.bind(5683).unwrap();
    cs.bind(5684).unwrap();
    let mut server = App::with_resources(SmallDevice::new().block_wise::<true>())
        .deterministic_for_tests()
        .allow_unprotected()
        .route("body", get(content))
        .bind(Udp::new(ss))
        .unwrap();
    let mut client = App::with_resources(SmallDevice::new().block_wise::<true>())
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Udp::new(cs))
        .unwrap();
    let clock = Clock;
    let mut server_timer = clock;
    let mut client_timer = clock;
    let completed = RefCell::new(0);
    {
        let serve = async {
            loop {
                server
                    .wait_async(clock.now_ms().unwrap(), &mut server_timer, pending())
                    .await
                    .unwrap();
                let now = clock.now_ms().unwrap();
                server
                    .progress_async(now, &mut || clock.now_ms(), &mut server_timer, now + 1000)
                    .await
                    .unwrap();
            }
        };
        let request = async {
            for _ in 0..2 {
                let call = client
                    .get("body")
                    .to(Endpoint::v4([10, 0, 0, 1], 5683))
                    .send_async(
                        clock.now_ms().unwrap(),
                        || clock.now_ms(),
                        &mut client_timer,
                    )
                    .await
                    .unwrap();
                loop {
                    client
                        .wait_async(clock.now_ms().unwrap(), &mut client_timer, pending())
                        .await
                        .unwrap();
                    let now = clock.now_ms().unwrap();
                    client
                        .progress_async(now, &mut || clock.now_ms(), &mut client_timer, now + 1000)
                        .await
                        .unwrap();
                    if let Some(response) = client.take_response(call) {
                        let response = response.unwrap();
                        assert_eq!(response.body().unwrap_or(response.payload()), BODY);
                        *completed.borrow_mut() += 1;
                        break;
                    }
                }
            }
        };
        let wake = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = Waker::from(wake.clone());
        let mut cx = Context::from_waker(&waker);
        let mut serve = pin!(serve);
        let mut request = pin!(request);
        let mut sr = pin!(server_runner.run());
        let mut cr = pin!(client_runner.run());
        let started = std::time::Instant::now();
        let mut done = false;
        // Finite manual test scheduler. This deliberately polls both links; it
        // verifies real stack wake registration, not a production executor.
        for _ in 0..256 {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            assert!(sr.as_mut().poll(&mut cx).is_pending());
            assert!(cr.as_mut().poll(&mut cx).is_pending());
            assert!(serve.as_mut().poll(&mut cx).is_pending());
            if request.as_mut().poll(&mut cx).is_ready() {
                done = true;
                break;
            }
            assert!(to_server.borrow().len() <= 2);
            assert!(to_client.borrow().len() <= 2);
        }
        assert!(done, "bounded exchange did not finish");
        assert!(wake.0.load(Ordering::Relaxed) > 0);
        // Dropping the suspended server and runners cancels their waits before
        // App/socket owners are inspected or destroyed.
    }
    assert_eq!(*completed.borrow(), 2);
    assert!(to_server.borrow().is_empty());
    assert!(to_client.borrow().is_empty());
    assert_eq!(server.metrics().rx_accepted, 4);
    assert_eq!(client.metrics().rx_accepted, 4);
    assert_eq!(server.transport().socket().endpoint().port, 5683);
    assert_eq!(client.transport().socket().endpoint().port, 5684);
}
