use super::*;
use core::{
    future::Future,
    task::{Context, Poll, Waker},
};

fn register(protected: bool) -> (Server, Peer) {
    let mut s = fixed();
    let mut p = Peer::new(&mut s, protected);
    let q = BlockValue::new(0, true, 1).unwrap().encode();
    let opts = [
        Opt::observe_register(),
        Opt::uri_path("obs"),
        Opt::q_block2(&q),
    ];
    s.transport_mut().incoming = Some(
        p.encode(
            &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(700))
                .with_token(key().token())
                .with_options(&opts),
        ),
    );
    s.poll_with(0, |_, _| response(b"initial").etag(b"v0"))
        .unwrap();
    assert!(s.transport_mut().take().is_some());
    assert!(s.transport_mut().take().is_none());
    (s, p)
}

fn chunk(s: &mut Server, p: &Peer, num: u32, terminal: bool) -> (MessageId, Option<u32>) {
    let wire = s.transport_mut().take().expect("next CON chunk");
    let opened = p.open(wire);
    let packet = decode(&opened.bytes[..opened.len]).unwrap();
    assert_eq!(packet.ty(), Type::Confirmable);
    assert_eq!(
        packet.code(),
        if terminal {
            Code::INTERNAL_SERVER_ERROR
        } else {
            Code::CONTENT
        }
    );
    let q = packet.q_block2().next().unwrap().unwrap();
    assert_eq!((q.num(), q.szx(), q.more()), (num, 1, num < 21));
    let start = num as usize * 32;
    assert_eq!(
        packet.payload(),
        &LARGE[start..(start + 32).min(LARGE.len())]
    );
    assert_eq!(packet.etag().next(), Some(b"v1".as_slice()));
    assert_eq!(packet.observe().is_none(), terminal);
    assert!(
        s.transport_mut().take().is_none(),
        "NSTART allows one chunk only"
    );
    (packet.message_id(), packet.observe().map(Result::unwrap))
}

#[test]
fn con_q_observe_crosses_windows_exact_acks_terminal_and_next_epoch() {
    for protected in [false, cfg!(feature = "oscore")] {
        for terminal in [false, true] {
            let (mut s, p) = register(protected);
            let value = if terminal {
                Response::new(Code::INTERNAL_SERVER_ERROR)
                    .payload_borrowed(&LARGE)
                    .etag(b"v1")
            } else {
                large()
            };
            assert_eq!(s.notify_confirmable(5000, &["obs"], value).unwrap(), 1);
            assert_eq!(s.engine_mut().lookup_observe(key()).is_none(), terminal);
            let (mut mid, epoch) = chunk(&mut s, &p, 0, terminal);
            let first = mid;
            s.transport_mut().incoming =
                Some(control(mid.wrapping_add(1000), Type::Acknowledgement));
            s.poll(5001).unwrap();
            assert!(s.transport_mut().take().is_none());
            let mut foreign = control(mid, Type::Acknowledgement);
            foreign.peer = Endpoint::v4([192, 0, 2, 99], 5683);
            s.transport_mut().incoming = Some(foreign);
            s.poll(5002).unwrap();
            assert!(s.transport_mut().take().is_none());
            for num in 1..22 {
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(5010 + u64::from(num)).unwrap();
                let (next, sequence) = chunk(&mut s, &p, num, terminal);
                assert_ne!(next, mid);
                assert_eq!(sequence, epoch);
                mid = next;
                if num == 1 {
                    s.transport_mut().incoming = Some(control(first, Type::Acknowledgement));
                    s.poll(5012).unwrap();
                    assert!(
                        s.transport_mut().take().is_none(),
                        "stale ACK cannot skip a chunk"
                    );
                }
            }
            s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
            s.poll(5100).unwrap();
            assert!(s.transport_mut().take().is_none());
            assert!(
                body(&mut s).is_some(),
                "completed snapshot remains recoverable"
            );
            if !terminal {
                assert_eq!(s.notify_confirmable(5200, &["obs"], large()).unwrap(), 1);
                let (_, next_epoch) = chunk(&mut s, &p, 0, false);
                if !protected {
                    assert_ne!(next_epoch, epoch);
                }
            }
        }
    }
}

thread_local! { static BLOCK_SEND: Cell<bool> = const { Cell::new(false) }; }
impl coaptic::storage::AsyncDatagramIo for Io {
    async fn recv(&mut self, out: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        match DatagramIo::recv(self, out)? {
            Some(frame) => Ok(frame),
            None => core::future::pending().await,
        }
    }
    async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if BLOCK_SEND.with(Cell::get) {
            core::future::pending::<()>().await;
        }
        DatagramIo::send(self, peer, bytes)
    }
}
struct Timer;
impl coaptic::platform::AsyncTimer for Timer {
    async fn wait_until(&mut self, _: u64) {
        core::future::pending::<()>().await;
    }
}

#[test]
fn con_q_observe_cancelled_continuation_retries_same_block_without_skipping() {
    for protected in [false, cfg!(feature = "oscore")] {
        let (mut s, p) = register(protected);
        s.notify_confirmable(5000, &["obs"], large()).unwrap();
        let (mid, epoch) = chunk(&mut s, &p, 0, false);
        let ack = control(mid, Type::Acknowledgement);
        let rx = s.engine_mut().acquire_rx().unwrap();
        s.engine_mut()
            .write_rx(rx, &ack.bytes[..ack.len], ack.peer)
            .unwrap();
        let mut clock = || Ok::<u64, ()>(5100);
        let mut timer = Timer;
        BLOCK_SEND.with(|b| b.set(true));
        {
            let mut progress =
                core::pin::pin!(s.progress_async(5100, &mut clock, &mut timer, 6000));
            assert!(
                progress
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        BLOCK_SEND.with(|b| b.set(false));
        assert!(s.transport_mut().take().is_none());
        assert_eq!(s.next_poll_at(5100), Some(5100));
        {
            let mut progress =
                core::pin::pin!(s.progress_async(5100, &mut clock, &mut timer, 6000));
            assert!(matches!(
                progress
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Ok(()))
            ));
        }
        assert_eq!(chunk(&mut s, &p, 1, false).1, epoch);
    }
}
