//! [`DatagramIo`]: first-class bind from any datagram transport into Engine slots.
//!
//! Transport progress in `design.md` moves bytes between the platform boundary
//! and Datagram Slots. This trait is that bind. The core still does not own a
//! socket, clock, or send policy (`CALLER.md`).

use super::DatagramSlots;
use super::Endpoint;
use super::Engine;
use super::SlotError;
use super::SlotId;
use super::Storage;

/// Caller-owned datagram transport (UDP, test loopback, or a `no_std` radio).
///
/// Implement this on the socket or driver. [`Engine::recv_from`] and
/// [`Engine::send_tx`] are the Engine-side bind. Do not treat raw
/// `recv`/`sendto` plus `write_rx` as the integration surface.
///
/// `recv` writes into the caller buffer (an RX slot when used through
/// [`Engine::recv_from`]). `None` means no datagram this call (idle poll,
/// timeout, or would-block). The core does not send.
pub trait DatagramIo {
    /// Transport-specific failure. Not a CoAP code.
    type Error;

    /// Receive one datagram into `buf`.
    ///
    /// `Ok(None)` is idle. `Ok(Some((n, endpoint)))` fills `buf[..n]` and
    /// names the remote peer. `n` must not exceed `buf.len()`.
    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error>;

    /// Send one datagram to `dest`.
    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error>;
}

/// Failure of [`Engine::recv_from`] / [`Engine::send_tx`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatagramIoError<E> {
    /// Incoming Datagram Pool is full. The transport datagram is not consumed.
    Saturated,
    /// Slot addressing or fill-length failure.
    Slot(SlotError),
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
            Self::Saturated => None,
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
        let id = self.acquire_rx().ok_or(DatagramIoError::Saturated)?;
        let outcome = match self.storage_mut().rx_payload_mut(id) {
            Some(buf) => io.recv(buf),
            None => {
                let _ = self.release_rx(id);
                return Err(DatagramIoError::Slot(SlotError::NotOccupied));
            }
        };
        match outcome {
            Ok(Some((n, endpoint))) => {
                if let Err(e) = self.storage_mut().set_rx_len(id, n) {
                    let _ = self.release_rx(id);
                    return Err(DatagramIoError::Slot(e));
                }
                if let Err(e) = self.storage_mut().set_rx_endpoint(id, endpoint) {
                    let _ = self.release_rx(id);
                    return Err(DatagramIoError::Slot(e));
                }
                Ok(Some(id))
            }
            Ok(None) => {
                let _ = self.release_rx(id);
                Ok(None)
            }
            Err(e) => {
                let _ = self.release_rx(id);
                Err(DatagramIoError::Io(e))
            }
        }
    }

    /// Send occupied TX `id` through `io`. Pins [`crate::Access`] for the call.
    ///
    /// Does not release the slot (pending CON / give-up still apply). Does
    /// not invent RST / 4.xx / No-Response policy.
    pub fn send_tx<T: DatagramIo>(
        &mut self,
        io: &mut T,
        id: SlotId,
    ) -> Result<usize, DatagramIoError<T::Error>> {
        let dest = self
            .tx_endpoint(id)
            .ok_or(DatagramIoError::Slot(SlotError::NotOccupied))?;
        let n = {
            let access = self.access_tx(id).map_err(DatagramIoError::Slot)?;
            io.send(dest, access.as_bytes())
                .map_err(DatagramIoError::Io)?
        };
        Ok(n)
    }
}

#[cfg(feature = "std")]
impl DatagramIo for std::net::UdpSocket {
    type Error = std::io::Error;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        match self.recv_from(buf) {
            Ok((n, from)) => Ok(Some((n, Endpoint::from(from)))),
            Err(e) if is_idle_io(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.send_to(bytes, std::net::SocketAddr::from(dest))
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
        assert_eq!(engine.rx_endpoint(rx), Some(peer));
        let parsed = engine.decode_rx(rx).expect("decode");
        assert_eq!(parsed.code(), Code::EMPTY);
        assert_eq!(parsed.ty(), Type::Acknowledgement);
        engine.release_rx(rx).expect("release");

        assert_eq!(engine.recv_from(&mut io).expect("idle"), None);
        assert_eq!(engine.rx_occupied(), 0);
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
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let (wire, n) = encode_empty_ack();
        let mut io = Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        };
        assert_eq!(engine.recv_from(&mut io), Err(DatagramIoError::Saturated));
        assert!(io.inbox.is_some());
    }
}
