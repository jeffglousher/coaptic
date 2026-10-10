//! [`DatagramIo`]: first-class bind from any datagram transport into Engine slots.
//!
//! This trait moves bytes between the platform boundary and Datagram Slots.
//! The core still does not own a socket, clock, or send policy.

use super::DatagramSlots;
use super::Endpoint;
use super::Engine;
use super::Metrics;
use super::SlotError;
use super::SlotId;
use super::Storage;

/// Caller-owned datagram transport (UDP, test loopback, or a `no_std` radio).
///
/// Available without `std` or `alloc`. Implement it on the platform socket or
/// driver and pass that value to [`crate::app::AppBuilder::bind`], which uses
/// fixed profile storage. App owns the transport value without requiring a
/// `Box`; platform network-stack and task allocations remain outside Coaptic's
/// storage budget. [`Engine::recv_from`] and [`Engine::send_tx`] borrow the
/// transport directly. `Ok(None)` means timeout or would-block.
///
/// Engine work is bounded by storage capacity, not by elapsed time. For a
/// bounded polling latency, implementations must bound the time spent in both
/// receive and send (for example, use nonblocking sockets). The caller must
/// also bound application callbacks and the interval between polls.
pub trait DatagramIo {
    /// Transport-specific failure. Not a CoAP code.
    type Error;

    /// Receive one datagram into `buf`.
    ///
    /// `Ok(None)` is idle. `Ok(Some((n, endpoint)))` fills `buf[..n]` and
    /// names the remote peer. `n` must not exceed `buf.len()`. Success must
    /// represent the complete datagram: refuse oversized packets rather than
    /// returning a truncated prefix.
    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error>;

    /// Send one complete datagram to `dest`; success must report `bytes.len()`.
    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error>;
}

/// Failure of [`Engine::recv_from`] / [`Engine::send_tx`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatagramIoError<E> {
    /// Incoming Datagram Pool is full. The transport datagram is not consumed.
    Saturated,
    /// Slot addressing or fill-length failure.
    Slot(SlotError),
    /// Transport reported a byte count different from the complete datagram.
    SendLength {
        /// Datagram length requested.
        expected: usize,
        /// Byte count reported by the transport.
        actual: usize,
    },
    /// Error from [`DatagramIo::Error`].
    Io(E),
}

impl<E> From<SlotError> for DatagramIoError<E> {
    fn from(e: SlotError) -> Self {
        Self::Slot(e)
    }
}

impl<E> core::fmt::Display for DatagramIoError<E>
where
    E: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Saturated => f.write_str("incoming datagram pool is saturated"),
            Self::Slot(e) => write!(f, "{e}"),
            Self::SendLength { expected, actual } => write!(
                f,
                "datagram send reported {actual} bytes; expected {expected}"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(feature = "std")]
impl<E> std::error::Error for DatagramIoError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Saturated | Self::SendLength { .. } => None,
            Self::Slot(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

impl<S: Storage + DatagramSlots> Engine<S> {
    /// Recv one datagram from `io` into a newly acquired RX slot.
    ///
    /// `Ok(None)` is an idle poll ([`DatagramIo::recv`] returned `None`); the
    /// slot is released. `Err(Saturated)` leaves the transport unread.
    /// Does not call [`Self::progress`] and does not send.
    pub fn recv_from<T: DatagramIo>(
        &mut self,
        io: &mut T,
    ) -> Result<Option<SlotId>, DatagramIoError<T::Error>> {
        #[cfg(feature = "diagnostics")]
        super::WorkMetrics::add(&mut self.work_metrics_mut().recv_attempts, 1);
        let id = self.acquire_rx().ok_or(DatagramIoError::Saturated)?;
        let outcome = match self.storage_mut().rx_payload_mut(id) {
            Some(buf) => io.recv(buf),
            None => {
                let _ = self.release_rx(id);
                Metrics::inc(&mut self.metrics_mut().rx_error);
                return Err(DatagramIoError::Slot(SlotError::NotOccupied));
            }
        };
        match outcome {
            Ok(Some((n, endpoint))) => {
                if let Err(e) = self.storage_mut().set_rx_len(id, n) {
                    let _ = self.release_rx(id);
                    Metrics::inc(&mut self.metrics_mut().rx_error);
                    return Err(DatagramIoError::Slot(e));
                }
                if let Err(e) = self.storage_mut().set_rx_endpoint(id, endpoint) {
                    let _ = self.release_rx(id);
                    Metrics::inc(&mut self.metrics_mut().rx_error);
                    return Err(DatagramIoError::Slot(e));
                }
                Metrics::inc(&mut self.metrics_mut().rx_accepted);
                #[cfg(feature = "diagnostics")]
                super::WorkMetrics::add(&mut self.work_metrics_mut().rx_wire_bytes, n);
                Ok(Some(id))
            }
            Ok(None) => {
                #[cfg(feature = "diagnostics")]
                super::WorkMetrics::add(&mut self.work_metrics_mut().recv_idle, 1);
                let _ = self.release_rx(id);
                Ok(None)
            }
            Err(e) => {
                let _ = self.release_rx(id);
                Metrics::inc(&mut self.metrics_mut().rx_error);
                Err(DatagramIoError::Io(e))
            }
        }
    }

    /// Send occupied TX `id` through `io`. Pins [`Access`](crate::storage::Access) for the call.
    ///
    /// Does not release the slot (pending CON / give-up still apply). Does
    /// not invent RST / 4.xx / No-Response policy.
    pub fn send_tx<T: DatagramIo>(
        &mut self,
        io: &mut T,
        id: SlotId,
    ) -> Result<usize, DatagramIoError<T::Error>> {
        let dest = match self.tx_endpoint(id) {
            Some(dest) => dest,
            None => {
                Metrics::inc(&mut self.metrics_mut().tx_fail);
                return Err(DatagramIoError::Slot(SlotError::NotOccupied));
            }
        };
        let outcome = match self.access_tx(id) {
            Ok(access) => {
                let bytes = access.as_bytes();
                io.send(dest, bytes)
                    .map_err(DatagramIoError::Io)
                    .and_then(|actual| {
                        if actual == bytes.len() {
                            Ok(actual)
                        } else {
                            Err(DatagramIoError::SendLength {
                                expected: bytes.len(),
                                actual,
                            })
                        }
                    })
            }
            Err(e) => Err(DatagramIoError::Slot(e)),
        };
        match outcome {
            Ok(n) => {
                Metrics::inc(&mut self.metrics_mut().tx_ok);
                #[cfg(feature = "diagnostics")]
                super::WorkMetrics::add(&mut self.work_metrics_mut().tx_wire_bytes, n);
                Ok(n)
            }
            Err(e) => {
                Metrics::inc(&mut self.metrics_mut().tx_fail);
                Err(e)
            }
        }
    }
}

/// Receives into scratch with one extra byte so exact-capacity datagrams are
/// accepted and oversized datagrams cannot become valid-looking prefixes.
/// Uses default-profile-sized stack scratch; larger caller buffers use a
/// fallible temporary allocation of `buf.len() + 1` bytes only with `alloc`.
/// Without `alloc`, larger buffers return `InvalidInput` before socket I/O.
/// Native receive errors
/// (including platform-specific truncation errors) are preserved.
/// Use [`UdpSocketIo`] with caller-supplied scratch to avoid that temporary
/// allocation when receiving larger datagrams.
#[cfg(feature = "std")]
impl DatagramIo for std::net::UdpSocket {
    type Error = std::io::Error;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        const STACK_BYTES: usize =
            <super::profiles::Default as super::MemoryProfile>::RX_DATAGRAM_BYTES + 1;
        let required = buf
            .len()
            .checked_add(1)
            .ok_or(std::io::ErrorKind::InvalidInput)?;
        let mut stack = [0; STACK_BYTES];
        #[cfg(feature = "alloc")]
        let mut heap = std::vec::Vec::new();
        let scratch = if required <= stack.len() {
            &mut stack[..required]
        } else {
            #[cfg(feature = "alloc")]
            {
                heap.try_reserve_exact(required)
                    .map_err(std::io::Error::other)?;
                heap.resize(required, 0);
                heap.as_mut_slice()
            }
            #[cfg(not(feature = "alloc"))]
            return Err(std::io::ErrorKind::InvalidInput.into());
        };
        recv_udp(self, buf, scratch)
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.send_to(bytes, std::net::SocketAddr::from(dest))
    }
}

/// Standard UDP transport with reusable caller-supplied receive scratch.
///
/// Scratch must contain at least one byte more than each destination buffer.
/// It can be an array, a borrowed slice or storage allocated once by the caller.
/// This adapter neither grows nor allocates scratch during receive. Custom
/// `AsMut` implementations and the OS network stack control their own memory.
/// Accepted packets are still copied into the destination; this is not zero-copy.
/// Socket blocking, timeout and nonblocking settings are retained.
///
/// ```no_run
/// use coaptic::storage::UdpSocketIo;
/// use std::net::UdpSocket;
/// let socket = UdpSocket::bind("127.0.0.1:5683")?;
/// socket.set_nonblocking(true)?;
/// let mut scratch = [0; 2049];
/// let transport = UdpSocketIo::new(socket, &mut scratch[..])?;
/// # Ok::<(), std::io::Error>(())
/// ```
#[cfg(feature = "std")]
pub struct UdpSocketIo<S> {
    socket: std::net::UdpSocket,
    scratch: S,
}

#[cfg(feature = "std")]
impl<S: AsMut<[u8]>> UdpSocketIo<S> {
    /// Own the socket and scratch. Empty scratch is refused.
    pub fn new(socket: std::net::UdpSocket, mut scratch: S) -> std::io::Result<Self> {
        if scratch.as_mut().is_empty() {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        Ok(Self { socket, scratch })
    }

    /// Access the socket for endpoint inspection or configuration.
    #[must_use]
    pub const fn socket(&self) -> &std::net::UdpSocket {
        &self.socket
    }

    /// Recover the socket and caller storage.
    #[must_use]
    pub fn into_parts(self) -> (std::net::UdpSocket, S) {
        (self.socket, self.scratch)
    }
}

#[cfg(feature = "std")]
impl<S: AsMut<[u8]>> DatagramIo for UdpSocketIo<S> {
    type Error = std::io::Error;

    fn recv(&mut self, buf: &mut [u8]) -> std::io::Result<Option<(usize, Endpoint)>> {
        recv_udp(&self.socket, buf, self.scratch.as_mut())
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> std::io::Result<usize> {
        self.socket.send_to(bytes, std::net::SocketAddr::from(dest))
    }
}

#[cfg(feature = "std")]
fn recv_udp(
    socket: &std::net::UdpSocket,
    buf: &mut [u8],
    scratch: &mut [u8],
) -> std::io::Result<Option<(usize, Endpoint)>> {
    let required = buf
        .len()
        .checked_add(1)
        .ok_or(std::io::ErrorKind::InvalidInput)?;
    let scratch = scratch
        .get_mut(..required)
        .ok_or(std::io::ErrorKind::InvalidInput)?;
    match socket.recv_from(scratch) {
        Ok((n, _)) if n > buf.len() => Err(std::io::ErrorKind::InvalidData.into()),
        Ok((n, from)) => {
            buf[..n].copy_from_slice(&scratch[..n]);
            Ok(Some((n, Endpoint::from(from))))
        }
        Err(e) if is_idle_io(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(feature = "std")]
fn is_idle_io(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::DatagramIo;
    use super::DatagramIoError;
    use super::Endpoint;
    use crate::message::{Code, Message, MessageId, Type, encode};
    use crate::storage::EngineBuilder;
    use crate::storage::Memory;
    use crate::storage::profiles;

    #[derive(Default)]
    struct Loopback {
        inbox: Option<(Endpoint, [u8; 64], usize)>,
        last_send: Option<(Endpoint, [u8; 64], usize)>,
    }

    impl DatagramIo for Loopback {
        type Error = &'static str;

        fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            let Some((ep, bytes, n)) = self.inbox.take() else {
                return Ok(None);
            };
            if n > buf.len() {
                return Err("short buf");
            }
            buf[..n].copy_from_slice(&bytes[..n]);
            Ok(Some((n, ep)))
        }

        fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            if bytes.len() > 64 {
                return Err("too long");
            }
            let mut slot = [0u8; 64];
            slot[..bytes.len()].copy_from_slice(bytes);
            self.last_send = Some((dest, slot, bytes.len()));
            Ok(bytes.len())
        }
    }

    fn encode_empty_ack() -> ([u8; 64], usize) {
        let msg = Message::empty_ack(MessageId::new(7));
        let mut buf = [0u8; 64];
        let n = encode(&msg, &mut buf).expect("ack");
        (buf, n)
    }

    #[test]
    fn recv_from_stores_endpoint_and_idle_releases() {
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .expect("build");
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let (wire, n) = encode_empty_ack();
        let mut io = Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        };
        let rx = engine.recv_from(&mut io).expect("recv").expect("stored");
        assert_eq!(engine.metrics().rx_accepted, 1);
        assert_eq!(engine.metrics().rx_error, 0);
        assert_eq!(engine.rx_endpoint(rx), Some(peer));
        let parsed = engine.decode_rx(rx).expect("decode");
        assert_eq!(parsed.code(), Code::EMPTY);
        assert_eq!(parsed.ty(), Type::Acknowledgement);
        engine.release_rx(rx).expect("release");

        assert_eq!(engine.recv_from(&mut io).expect("idle"), None);
        assert_eq!(engine.rx_occupied(), 0);
        assert_eq!(engine.metrics().rx_accepted, 1);
    }

    #[test]
    fn send_tx_uses_sidecar_endpoint() {
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .expect("build");
        let dest = Endpoint::v4([192, 0, 2, 9], 5683);
        let (wire, n) = encode_empty_ack();
        let tx = engine.acquire_tx().expect("tx");
        engine.write_tx(tx, &wire[..n], dest).expect("write_tx");
        let mut io = Loopback::default();
        let sent = engine.send_tx(&mut io, tx).expect("send_tx");
        assert_eq!(sent, n);
        assert_eq!(engine.metrics().tx_ok, 1);
        assert_eq!(engine.metrics().tx_fail, 0);
        let (got_ep, got, got_n) = io.last_send.expect("sent");
        assert_eq!(got_ep, dest);
        assert_eq!(&got[..got_n], &wire[..n]);
        engine.release_tx(tx).expect("release");
    }

    #[test]
    fn recv_from_saturated_does_not_consume() {
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .expect("build");
        while engine.acquire_rx().is_some() {}
        engine.reset_metrics();
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let (wire, n) = encode_empty_ack();
        let mut io = Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        };
        assert_eq!(engine.recv_from(&mut io), Err(DatagramIoError::Saturated));
        assert_eq!(engine.metrics().saturated, 1);
        assert!(io.inbox.is_some());
    }

    #[test]
    fn send_length_failure_retains_datagram_for_complete_retry() {
        struct Count(usize);
        impl DatagramIo for Count {
            type Error = &'static str;
            fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
                Ok(None)
            }
            fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> {
                Ok(self.0)
            }
        }
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .unwrap();
        let (wire, n) = encode_empty_ack();
        let tx = engine.acquire_tx().unwrap();
        let dest = Endpoint::v4([192, 0, 2, 9], 5683);
        engine.write_tx(tx, &wire[..n], dest).unwrap();
        for actual in [0, n - 1, n + 1, usize::MAX] {
            assert_eq!(
                engine.send_tx(&mut Count(actual), tx),
                Err(DatagramIoError::SendLength {
                    expected: n,
                    actual
                })
            );
            assert_eq!(engine.access_tx(tx).unwrap().as_bytes(), &wire[..n]);
            assert_eq!(engine.tx_endpoint(tx), Some(dest));
        }
        assert_eq!(engine.metrics().tx_ok, 0);
        assert_eq!(engine.metrics().tx_fail, 4);
        assert_eq!(engine.send_tx(&mut Count(n), tx), Ok(n));
        assert_eq!(engine.metrics().tx_ok, 1);
        engine.release_tx(tx).unwrap();
    }

    #[test]
    fn send_tx_fail_increments_tx_fail() {
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .expect("build");
        let dest = Endpoint::v4([192, 0, 2, 9], 5683);
        let (wire, n) = encode_empty_ack();
        let tx = engine.acquire_tx().expect("tx");
        engine.write_tx(tx, &wire[..n], dest).expect("write_tx");
        struct FailSend;
        impl DatagramIo for FailSend {
            type Error = &'static str;
            fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
                Ok(None)
            }
            fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> {
                Err("nope")
            }
        }
        assert!(engine.send_tx(&mut FailSend, tx).is_err());
        assert_eq!(engine.metrics().tx_ok, 0);
        assert_eq!(engine.metrics().tx_fail, 1);
        engine.release_tx(tx).expect("release");
    }
}

#[cfg(all(test, feature = "std"))]
mod udp_tests {
    use super::*;
    use std::net::UdpSocket;
    use std::time::Duration;

    #[test]
    fn reusable_udp_scratch_rejects_oversize_without_exposing_prefix() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let sender = UdpSocket::bind(address).unwrap();
            let socket = UdpSocket::bind(address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let dest = socket.local_addr().unwrap();
            let mut scratch = [0; 2049];
            let mut receiver = UdpSocketIo::new(socket, &mut scratch[..]).unwrap();
            for capacity in [0, 64, 1472, 2048] {
                let mut output = std::vec![0xA5; capacity];
                for extra in [0, 1, 100] {
                    let payload = std::vec![0x39; capacity + extra];
                    sender.send_to(&payload, dest).unwrap();
                    output.fill(0xA5);
                    let got = receiver.recv(&mut output);
                    if extra == 0 {
                        assert_eq!(
                            got.unwrap(),
                            Some((capacity, sender.local_addr().unwrap().into()))
                        );
                        assert_eq!(output, payload);
                    } else {
                        assert!(got.is_err());
                        assert!(output.iter().all(|b| *b == 0xA5));
                    }
                    sender.send_to(b"", dest).unwrap();
                    assert_eq!(receiver.recv(&mut output).unwrap().unwrap().0, 0);
                }
            }
            receiver.socket().set_nonblocking(true).unwrap();
            assert_eq!(receiver.recv(&mut [0; 2048]).unwrap(), None);
            let (socket, _) = receiver.into_parts();
            assert_eq!(socket.local_addr().unwrap(), dest);
        }
    }

    #[test]
    fn insufficient_scratch_does_not_consume_the_datagram() {
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let dest = socket.local_addr().unwrap();
        let mut receiver = UdpSocketIo::new(socket, [0; 65]).unwrap();
        sender.send_to(&[0x39; 64], dest).unwrap();
        let mut too_large = [0xA5; 65];
        assert_eq!(
            receiver.recv(&mut too_large).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(too_large, [0xA5; 65]);
        let mut fits = [0; 64];
        assert_eq!(receiver.recv(&mut fits).unwrap().unwrap().0, 64);
        assert_eq!(fits, [0x39; 64]);
        assert!(
            matches!(UdpSocketIo::new(sender, [0; 0]), Err(e) if e.kind() == std::io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn reusable_udp_scratch_sends_to_the_requested_peer() {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender = socket.local_addr().unwrap();
        let mut io = UdpSocketIo::new(socket, [0; 65]).unwrap();
        assert_eq!(
            io.send(peer.local_addr().unwrap().into(), b"packet")
                .unwrap(),
            6
        );
        let mut bytes = [0; 64];
        let (n, from) = peer.recv_from(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"packet");
        assert_eq!(from, sender);
    }

    #[test]
    fn reusable_udp_delivers_a_complete_app_block2_body() {
        use crate::message::{BlockValue, Message, MessageId, Opt, Token, Type, decode};
        use crate::{App, Code, Response, get};
        static BODY: [u8; 2000] = [0x5A; 2000];
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let destination = socket.local_addr().unwrap();
        let mut app = App::builder()
            .routes::<1>()
            .block_wise::<true>()
            .randomness(|bytes| {
                bytes.fill(0x39);
                true
            })
            .route("large", get(|_| Response::content(&BODY).etag(b"body")))
            .allow_plaintext()
            .bind(UdpSocketIo::new(socket, [0; 1473]).unwrap())
            .unwrap();
        let token = Token::new(b"body").unwrap();
        let mut complete = std::vec::Vec::new();
        for number in 0..2 {
            let selected = BlockValue::new(number, false, 6).unwrap().encode();
            let options = [Opt::uri_path("large"), Opt::block2(&selected)];
            let mid = MessageId::new(number as u16 + 1);
            let request = Message::con(Code::GET, mid, token).with_options(&options);
            let mut bytes = [0; 1472];
            let n = request.encode(&mut bytes).unwrap();
            peer.send_to(&bytes[..n], destination).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while app.metrics().rx_accepted <= number {
                assert!(std::time::Instant::now() < deadline);
                app.poll(u64::from(number)).unwrap();
            }
            let (n, from) = peer.recv_from(&mut bytes).unwrap();
            assert_eq!(from, destination);
            let response = decode(&bytes[..n]).unwrap();
            assert_eq!(response.ty(), Type::Acknowledgement);
            assert_eq!(response.code(), Code::CONTENT);
            assert_eq!(response.message_id(), mid);
            assert_eq!(response.token(), token);
            assert_eq!(response.etag().next(), Some(&b"body"[..]));
            let block = response.block2().unwrap().unwrap();
            assert_eq!(block.num(), number);
            assert_eq!(block.more(), number == 0);
            complete.extend_from_slice(response.payload());
        }
        assert_eq!(complete, BODY);
    }

    #[test]
    fn udp_exact_capacity_oversize_and_recovery_ipv4_ipv6() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let sender = UdpSocket::bind(address).unwrap();
            let mut receiver = UdpSocket::bind(address).unwrap();
            receiver
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let dest = receiver.local_addr().unwrap();
            // The large raw-socket fallback is an explicit allocator capability.
            let capacities = if cfg!(feature = "alloc") {
                &[0, 64, 1472, 2048][..]
            } else {
                &[0, 64, 1472][..]
            };
            for &capacity in capacities {
                let mut output = std::vec![0xA5; capacity];
                for extra in [0, 1, 100] {
                    let payload = std::vec![0x39; capacity + extra];
                    sender.send_to(&payload, dest).unwrap();
                    output.fill(0xA5);
                    let got = DatagramIo::recv(&mut receiver, &mut output);
                    if extra == 0 {
                        assert_eq!(
                            got.unwrap(),
                            Some((capacity, sender.local_addr().unwrap().into()))
                        );
                        assert_eq!(output, payload);
                    } else {
                        assert!(
                            got.is_err(),
                            "accepted truncated {address} capacity={capacity} extra={extra}"
                        );
                        // No prefix is exposed, even on Windows native failure.
                        assert!(output.iter().all(|b| *b == 0xA5));
                    }
                    sender.send_to(b"", dest).unwrap();
                    assert_eq!(
                        DatagramIo::recv(&mut receiver, &mut output)
                            .unwrap()
                            .unwrap()
                            .0,
                        0
                    );
                }
            }
            receiver.set_nonblocking(true).unwrap();
            assert_eq!(DatagramIo::recv(&mut receiver, &mut [0; 8]).unwrap(), None);
        }
    }
    #[test]
    fn oversized_udp_cannot_occupy_an_engine_slot_and_next_packet_survives() {
        use crate::storage::{EngineBuilder, Memory, profiles};
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let dest = receiver.local_addr().unwrap();
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .unwrap();
        sender.send_to(&[0x41; 3000], dest).unwrap();
        assert!(matches!(
            engine.recv_from(&mut receiver),
            Err(DatagramIoError::Io(_))
        ));
        assert_eq!(engine.rx_occupied(), 0);
        assert_eq!(engine.metrics().rx_accepted, 0);
        assert_eq!(engine.metrics().rx_error, 1);
        let packet = [0x60, 0, 0x12, 0x34];
        sender.send_to(&packet, dest).unwrap();
        let rx = engine.recv_from(&mut receiver).unwrap().unwrap();
        assert_eq!(engine.access_rx(rx).unwrap().as_bytes(), packet);
        engine.release_rx(rx).unwrap();
        assert_eq!(engine.rx_occupied(), 0);
    }
}
