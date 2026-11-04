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
