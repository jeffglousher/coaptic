//! Approachable CoAP server: routes and resources, not slot plumbing.
//!
//! [`Server::poll`] is one loop step: recv, progress, route, reply, send,
//! release. The Engine still owns CoAP mechanics ([`design.md`][design]
//! §Application memory access). This module is a façade — not a seventh
//! memory area, not a routing-model lock-in.
//!
//! ```
//! use coaptic::{
//!     DatagramIo, Endpoint, EngineBuilder, Memory, Reply, Request, Resource, Server,
//!     profiles,
//! };
//!
//! struct Temp;
//! impl Resource for Temp {
//!     fn handle<'a>(_request: &'a Request<'a>) -> Reply<'a> {
//!         Reply::content(b"21.5")
//!     }
//! }
//!
//! # struct NullIo;
//! # impl DatagramIo for NullIo {
//! #     type Error = &'static str;
//! #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
//! #         Ok(None)
//! #     }
//! #     fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> { Ok(0) }
//! # }
//! let engine = EngineBuilder::new()
//!     .profile::<profiles::Default>()
//!     .block_wise(false)
//!     .build(Memory::<profiles::Default>::new())
//!     .unwrap();
//! let mut server = Server::new(engine, NullIo);
//! server.router().at(&["sensors", "temp"]).get(Temp);
//! server.poll(0).unwrap();
//! ```
//!
//! Engine / [`DatagramIo`] remain the advanced path (`CALLER.md`,
//! `ERGONOMICS.md`).
//!
//! [design]: https://github.com/jeffglousher/coaptic/blob/main/design.md

mod reply;
mod request;
mod resource;
mod router;

#[cfg(test)]
mod tests;

use crate::error::SlotMessageError;
use crate::message::{
    EncodedUint, Message, NoResponse, Opt, OptionsBuilder, Type, decode, encode_uint,
};
use crate::storage::{
    BodySlots, DatagramIo, DatagramIoError, DatagramSlots, Endpoint, Engine, ObserveSlots,
    PendingCons, Retransmit, SlotError, SlotId, Storage,
};

pub use reply::Reply;
pub use request::{MAX_PATH_SEGMENTS, Request};
pub use resource::{Method, Resource};
pub use router::{At, DEFAULT_ROUTES, Router};

/// Scratch for one inbound or outbound datagram (crate Default profile).
const DATAGRAM_SCRATCH: usize = 1472;

/// CoAP server: Engine + transport + a bounded [`Router`].
///
/// `ROUTES` is the maximum number of `.get` / `.put` / … bindings (default 8).
/// Increase with `Server<_, _, 16>` if you need more.
pub struct Server<S: Storage, T, const ROUTES: usize = DEFAULT_ROUTES> {
    engine: Engine<S>,
    io: T,
    router: Router<ROUTES>,
}

impl<S: Storage, T, const ROUTES: usize> Server<S, T, ROUTES> {
    /// Bind `engine` to `io`. Register routes with [`Self::router`].
    #[must_use]
    pub const fn new(engine: Engine<S>, io: T) -> Self {
        Self {
            engine,
            io,
            router: Router::new(),
        }
    }

    /// Add routes: `.at(&["sensors", "temp"]).get(Temp)`.
    pub fn router(&mut self) -> &mut Router<ROUTES> {
        &mut self.router
    }

    /// Engine (advanced: slots, Observe, Block).
    #[must_use]
    pub const fn engine(&self) -> &Engine<S> {
        &self.engine
    }

    /// Mutable Engine (advanced).
    pub const fn engine_mut(&mut self) -> &mut Engine<S> {
        &mut self.engine
    }

    /// Transport.
    #[must_use]
    pub const fn transport(&self) -> &T {
        &self.io
    }

    /// Mutable transport.
    pub const fn transport_mut(&mut self) -> &mut T {
        &mut self.io
    }

    /// Split into Engine and transport. Routes are dropped.
    #[must_use]
    pub fn into_parts(self) -> (Engine<S>, T) {
        (self.engine, self.io)
    }
}

impl<S, T, const ROUTES: usize> Server<S, T, ROUTES>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: DatagramIo,
{
    /// One loop step: recv, progress, route, reply, send, release.
    ///
    /// Retransmit: `send_tx` on [`Retransmit::Due`], release on
    /// [`Retransmit::GiveUp`].
    ///
    /// Incoming requests are dispatched through the router (4.04 / 4.05
    /// when no match). CON is answered with a piggybacked ACK.
    ///
    /// Observe notify, Observe expiry, and Q-Block recover are **not**
    /// sent here yet (Phase 2). [`Engine::progress`] still surfaces them;
    /// handle those through [`Self::engine_mut`] if you need them now.
    pub fn poll(&mut self, now_ms: u64) -> Result<(), Error<T::Error>> {
        let received = self.engine.recv_from(&mut self.io)?;
        let progress = self.engine.progress(now_ms);

        if let Some(retransmit) = progress.retransmit() {
            match retransmit {
                Retransmit::Due(pending) => {
                    self.engine.send_tx(&mut self.io, pending.tx_slot())?;
                }
                Retransmit::GiveUp(pending) => {
                    self.engine.release_tx(pending.tx_slot())?;
                }
            }
        }

        if let Some(rx) = progress.rx_ready().or(received) {
            self.dispatch_rx(rx)?;
        }

        // Phase 2: Observe notify / lifetime / Q-Block recover.
        let _ = progress.observe_notify();
        let _ = progress.observe_expired();
        let _ = progress.qblock_recover();
        Ok(())
    }

    fn dispatch_rx(&mut self, rx: SlotId) -> Result<(), Error<T::Error>> {
        let Some(peer) = self.engine.rx_endpoint(rx) else {
            let _ = self.engine.release_rx(rx);
            return Ok(());
        };

        let mut scratch = [0u8; DATAGRAM_SCRATCH];
        let n = match copy_rx(&mut self.engine, rx, &mut scratch) {
            Ok(n) => n,
            Err(e) => {
                let _ = self.engine.release_rx(rx);
                return Err(e);
            }
        };

        let parsed = match decode(&scratch[..n]) {
            Ok(parsed) => parsed,
            Err(_) => {
                let _ = self.engine.release_rx(rx);
                return Ok(());
            }
        };

        if parsed.is_empty_ack_or_rst() {
            if let Some(tx) = self.engine.match_empty_ack_rst(&parsed, peer) {
                let _ = self.engine.release_tx(tx);
            }
            let _ = self.engine.release_rx(rx);
            return Ok(());
        }

        if parsed.is_empty() {
            let outcome = if parsed.ty() == Type::Confirmable {
                send_empty_ack(&mut self.engine, &mut self.io, peer, parsed.message_id())
            } else {
                Ok(())
            };
            let _ = self.engine.release_rx(rx);
            return outcome;
        }

        if !parsed.code().is_request() {
            let _ = self.engine.release_rx(rx);
            return Ok(());
        }

        let no_response = NoResponse::from_message(&parsed).unwrap_or(NoResponse::DEFAULT);
        let meta = SendReply {
            dest: peer,
            ty: parsed.ty(),
            mid: parsed.message_id(),
            token: parsed.token(),
            no_response,
        };
        let outcome = match Request::from_decoded(parsed, peer, rx) {
            Ok(request) => {
                let reply = self.router.dispatch(&request);
                self.send_reply(meta, &reply)
            }
            Err(request::PathError::BadUtf8) => self.send_reply(meta, &Reply::bad_request()),
            Err(request::PathError::TooLong) => self.send_reply(meta, &Reply::not_found()),
        };
        let _ = self.engine.release_rx(rx);
        outcome
    }

    fn send_reply(&mut self, meta: SendReply, reply: &Reply<'_>) -> Result<(), Error<T::Error>> {
        if meta.no_response.suppresses(reply.code()) {
            return Ok(());
        }

        let ty = match meta.ty {
            Type::Confirmable => Type::Acknowledgement,
            Type::NonConfirmable => Type::NonConfirmable,
            Type::Acknowledgement | Type::Reset => return Ok(()),
        };

        let Some(tx) = self.engine.acquire_tx() else {
            return Err(Error::Saturated);
        };

        let cf = reply.content_format().map(crate::ContentFormat::encode);
        let max_age = reply.max_age().map(EncodedUint::new);
        let observe = reply.observe().map(|seq| encode_uint(seq & 0x00ff_ffff));
        let mut opts = OptionsBuilder::<4>::new();
        if let Some(ref encoded) = observe {
            let _ = opts.push(Opt::observe(encoded));
        }
        if let Some(etag) = reply.etag() {
            let _ = opts.push(Opt::etag(etag));
        }
        if let Some(ref encoded) = cf {
            let _ = opts.push(Opt::content_format(encoded));
        }
        if let Some(ref encoded) = max_age {
            let _ = opts.push(Opt::max_age(encoded));
        }

        let msg = Message::new(ty, reply.code(), meta.mid)
            .with_token(meta.token)
            .with_options(opts.as_slice())
            .with_payload(reply.payload());

        if let Err(e) = self.engine.encode_tx(tx, &msg) {
            let _ = self.engine.release_tx(tx);
            return Err(Error::Message(e));
        }
        if let Err(e) = self.engine.set_tx_endpoint(tx, meta.dest) {
            let _ = self.engine.release_tx(tx);
            return Err(Error::Slot(e));
        }
        let send = self.engine.send_tx(&mut self.io, tx);
        let _ = self.engine.release_tx(tx);
        send?;
        Ok(())
    }
}

struct SendReply {
    dest: Endpoint,
    ty: Type,
    mid: crate::message::MessageId,
    token: crate::message::Token,
    no_response: NoResponse,
}

fn copy_rx<S: Storage + DatagramSlots, E>(
    engine: &mut Engine<S>,
    rx: SlotId,
    dest: &mut [u8],
) -> Result<usize, Error<E>> {
    let access = engine.access_rx(rx)?;
    let src = access.as_bytes();
    if src.len() > dest.len() {
        return Err(Error::Message(SlotMessageError::Slot(
            SlotError::LengthExceedsSlot,
        )));
    }
    dest[..src.len()].copy_from_slice(src);
    Ok(src.len())
}

fn send_empty_ack<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    mid: crate::message::MessageId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots,
    T: DatagramIo,
{
    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
    let msg = Message::empty_ack(mid);
    if let Err(e) = engine.encode_tx(tx, &msg) {
        let _ = engine.release_tx(tx);
        return Err(Error::Message(e));
    }
    if let Err(e) = engine.set_tx_endpoint(tx, dest) {
        let _ = engine.release_tx(tx);
        return Err(Error::Slot(e));
    }
    let send = engine.send_tx(io, tx);
    let _ = engine.release_tx(tx);
    send?;
    Ok(())
}

/// Failure of [`Server::poll`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error<E> {
    /// [`Engine::recv_from`] / [`Engine::send_tx`].
    Io(DatagramIoError<E>),
    /// Slot addressing or fill.
    Slot(SlotError),
    /// Decode / encode in a slot.
    Message(SlotMessageError),
    /// TX pool is full. The RX datagram was released.
    Saturated,
}

impl<E> From<DatagramIoError<E>> for Error<E> {
    fn from(e: DatagramIoError<E>) -> Self {
        match e {
            DatagramIoError::Saturated => Self::Saturated,
            other => Self::Io(other),
        }
    }
}

impl<E> From<SlotError> for Error<E> {
    fn from(e: SlotError) -> Self {
        Self::Slot(e)
    }
}

impl<E> From<SlotMessageError> for Error<E> {
    fn from(e: SlotMessageError) -> Self {
        match e {
            SlotMessageError::Slot(slot) => Self::Slot(slot),
            other => Self::Message(other),
        }
    }
}

impl<E> core::fmt::Display for Error<E>
where
    E: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Slot(e) => write!(f, "{e}"),
            Self::Message(e) => write!(f, "{e}"),
            Self::Saturated => f.write_str("outgoing datagram pool is saturated"),
        }
    }
}

#[cfg(feature = "std")]
impl<E> std::error::Error for Error<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Slot(e) => Some(e),
            Self::Message(e) => Some(e),
            Self::Saturated => None,
        }
    }
}
