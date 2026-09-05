//! Approachable CoAP app: owned resources, not slot plumbing.
//!
//! [`App::poll`] is one loop step: recv, progress, route, method hook, send,
//! release. The Engine still owns CoAP mechanics ([`design.md`][design]
//! §Application memory access). This module is a façade — not a seventh
//! memory area.
//!
//! ```
//! use coaptic::{
//!     App, ContentFormat, DatagramIo, Endpoint, Reply, Request, Resource, profiles,
//! };
//!
//! struct Temp { celsius: i16 }
//! impl Resource for Temp {
//!     fn get(&mut self, _req: &Request<'_>) -> Reply {
//!         Reply::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
//!     }
//! }
//!
//! struct Led { on: bool }
//! impl Resource for Led {
//!     fn get(&mut self, _: &Request<'_>) -> Reply {
//!         Reply::content(if self.on { b"on" } else { b"off" })
//!     }
//!     fn put(&mut self, req: &Request<'_>) -> Reply {
//!         self.on = req.payload() == b"1";
//!         Reply::changed()
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
//! let mut app = App::profile::<profiles::Default>()
//!     .block_wise(true)
//!     .bind(NullIo)
//!     .unwrap();
//! app.at(&["sensors", "temp"], Temp { celsius: 215 })
//!    .at(&["leds", "0"], Led { on: false })
//!    .well_known_core();
//! app.poll(0).unwrap();
//! ```
//!
//! Site capacity defaults to [`DEFAULT_RESOURCES`] (8). Raise it with
//! [`.resources::<16>()`](AppBuilder::resources) before [`AppBuilder::bind`].
//! Engine / [`DatagramIo`] remain the advanced path (`CALLER.md`,
//! `ERGONOMICS.md`). Escape: [`App::engine_mut`].
//!
//! [design]: https://github.com/jeffglousher/coaptic/blob/main/design.md

mod reply;
mod request;
mod resource;
#[allow(unsafe_code)]
mod resource_dyn;
mod site;

#[cfg(test)]
mod tests;

use core::marker::PhantomData;

use crate::error::{BuildError, SlotMessageError};
use crate::message::{
    EncodedUint, Message, NoResponse, Opt, OptionsBuilder, Type, decode, encode_uint,
};
use crate::storage::{
    BodySlots, DatagramIo, DatagramIoError, DatagramSlots, Endpoint, Engine, EngineBuilder, Memory,
    MemoryProfile, Missing, ObserveSlots, PendingCons, Present, Retransmit, SlotError, SlotId,
    Storage, WithBodies,
};

pub use reply::{INLINE_PAYLOAD, Reply};
pub use request::{MAX_PATH_SEGMENTS, Request};
pub use resource::{Method, Resource};
pub use resource_dyn::{MAX_RESOURCE_ALIGN, MAX_RESOURCE_BYTES, ResourceDyn};
pub use site::{DEFAULT_RESOURCES, Site};

/// Scratch for one inbound or outbound datagram (crate Default profile).
const DATAGRAM_SCRATCH: usize = 1472;

enum EngineSlot<P: MemoryProfile> {
    Datagram(Engine<Memory<P>>),
    BlockWise(Engine<Memory<P, WithBodies<P>>>),
}

/// CoAP app: profile memory + transport + a bounded [`Site`].
///
/// `N` is the maximum number of owned resources (default 8). Increase with
/// [`AppBuilder::resources`] (`App::<_, _, 16>` after bind).
pub struct App<
    P: MemoryProfile = crate::profiles::Default,
    T = (),
    const N: usize = DEFAULT_RESOURCES,
> {
    engine: EngineSlot<P>,
    io: T,
    site: Site<N>,
}

/// Builder: [`App::profile`] → [`AppBuilder::block_wise`] → [`AppBuilder::bind`].
pub struct AppBuilder<P: MemoryProfile, Block = Missing, const N: usize = DEFAULT_RESOURCES> {
    block_wise: Option<bool>,
    _p: PhantomData<P>,
    _b: PhantomData<Block>,
}

impl App {
    /// Start from a memory profile. Next: [`AppBuilder::block_wise`].
    #[must_use]
    pub const fn profile<P: MemoryProfile>() -> AppBuilder<P> {
        AppBuilder {
            block_wise: None,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<P: MemoryProfile, Block, const N: usize> AppBuilder<P, Block, N> {
    /// Site table size (default [`DEFAULT_RESOURCES`]).
    #[must_use]
    pub const fn resources<const M: usize>(self) -> AppBuilder<P, Block, M> {
        AppBuilder {
            block_wise: self.block_wise,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<P: MemoryProfile, const N: usize> AppBuilder<P, Missing, N> {
    /// Enable or disable body pools, then [`AppBuilder::bind`].
    #[must_use]
    pub const fn block_wise(self, enabled: bool) -> AppBuilder<P, Present, N> {
        AppBuilder {
            block_wise: Some(enabled),
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<P: MemoryProfile, const N: usize> AppBuilder<P, Present, N> {
    /// Construct profile [`Memory`] and bind `io`.
    pub fn bind<T>(self, io: T) -> Result<App<P, T, N>, BuildError> {
        let enabled = self.block_wise.expect("typestate: block_wise was set");
        let engine = if enabled {
            let built = EngineBuilder::new()
                .profile::<P>()
                .block_wise(true)
                .build(Memory::<P>::with_block_wise())?;
            EngineSlot::BlockWise(built)
        } else {
            let built = EngineBuilder::new()
                .profile::<P>()
                .block_wise(false)
                .build(Memory::<P>::new())?;
            EngineSlot::Datagram(built)
        };
        Ok(App {
            engine,
            io,
            site: Site::new(),
        })
    }
}

impl<P: MemoryProfile, T, const N: usize> App<P, T, N> {
    /// Move `resource` onto `segments`.
    pub fn at<R: Resource>(&mut self, segments: &[&'static str], resource: R) -> &mut Self {
        self.site.at(segments, resource);
        self
    }

    /// Serve `/.well-known/core` from registered paths.
    pub fn well_known_core(&mut self) -> &mut Self {
        self.site.well_known_core();
        self
    }

    /// The site table.
    pub fn site(&mut self) -> &mut Site<N> {
        &mut self.site
    }

    /// Engine (advanced: slots, Observe, Block).
    #[must_use]
    pub fn engine(&self) -> EngineRef<'_, P> {
        match &self.engine {
            EngineSlot::Datagram(engine) => EngineRef::Datagram(engine),
            EngineSlot::BlockWise(engine) => EngineRef::BlockWise(engine),
        }
    }

    /// Mutable Engine (advanced).
    pub fn engine_mut(&mut self) -> EngineMut<'_, P> {
        match &mut self.engine {
            EngineSlot::Datagram(engine) => EngineMut::Datagram(engine),
            EngineSlot::BlockWise(engine) => EngineMut::BlockWise(engine),
        }
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

    /// Drop the app and keep the transport.
    #[must_use]
    pub fn into_io(self) -> T {
        self.io
    }
}

impl<P, T, const N: usize> App<P, T, N>
where
    P: MemoryProfile,
    T: DatagramIo,
    Memory<P>: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    Memory<P, WithBodies<P>>: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
{
    /// One loop step: recv, progress, route, method hook, send, release.
    ///
    /// Retransmit: `send_tx` on [`Retransmit::Due`], release on
    /// [`Retransmit::GiveUp`].
    ///
    /// Incoming requests are dispatched through the site (4.04 / 4.05
    /// when no match). CON is answered with a piggybacked ACK.
    ///
    /// Observe notify, Observe expiry, and Q-Block recover are **not**
    /// sent here yet (Phase 2). [`Engine::progress`] still surfaces them;
    /// handle those through [`Self::engine_mut`] if you need them now.
    pub fn poll(&mut self, now_ms: u64) -> Result<(), Error<T::Error>> {
        match &mut self.engine {
            EngineSlot::Datagram(engine) => {
                poll_engine(engine, &mut self.io, &mut self.site, now_ms)
            }
            EngineSlot::BlockWise(engine) => {
                poll_engine(engine, &mut self.io, &mut self.site, now_ms)
            }
        }
    }
}

/// Borrowed Engine after `.block_wise(false)` or `.block_wise(true)`.
pub enum EngineRef<'a, P: MemoryProfile> {
    /// Datagram pools only.
    Datagram(&'a Engine<Memory<P>>),
    /// Body pools included.
    BlockWise(&'a Engine<Memory<P, WithBodies<P>>>),
}

/// Mutable Engine after `.block_wise(false)` or `.block_wise(true)`.
pub enum EngineMut<'a, P: MemoryProfile> {
    /// Datagram pools only.
    Datagram(&'a mut Engine<Memory<P>>),
    /// Body pools included.
    BlockWise(&'a mut Engine<Memory<P, WithBodies<P>>>),
}

impl<P: MemoryProfile> EngineMut<'_, P> {
    /// Occupied RX slots.
    #[must_use]
    pub fn rx_occupied(&mut self) -> usize {
        match self {
            Self::Datagram(engine) => engine.rx_occupied(),
            Self::BlockWise(engine) => engine.rx_occupied(),
        }
    }

    /// Occupied TX slots.
    #[must_use]
    pub fn tx_occupied(&mut self) -> usize {
        match self {
            Self::Datagram(engine) => engine.tx_occupied(),
            Self::BlockWise(engine) => engine.tx_occupied(),
        }
    }
}

fn poll_engine<S, T, const N: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    site: &mut Site<N>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: DatagramIo,
{
    let received = engine.recv_from(io)?;
    let progress = engine.progress(now_ms);

    if let Some(retransmit) = progress.retransmit() {
        match retransmit {
            Retransmit::Due(pending) => {
                engine.send_tx(io, pending.tx_slot())?;
            }
            Retransmit::GiveUp(pending) => {
                engine.release_tx(pending.tx_slot())?;
            }
        }
    }

    if let Some(rx) = progress.rx_ready().or(received) {
        dispatch_rx(engine, io, site, rx)?;
    }

    // Phase 2: Observe notify / lifetime / Q-Block recover.
    let _ = progress.observe_notify();
    let _ = progress.observe_expired();
    let _ = progress.qblock_recover();
    Ok(())
}

fn dispatch_rx<S, T, const N: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    site: &mut Site<N>,
    rx: SlotId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons,
    T: DatagramIo,
{
    let Some(peer) = engine.rx_endpoint(rx) else {
        let _ = engine.release_rx(rx);
        return Ok(());
    };

    let mut scratch = [0u8; DATAGRAM_SCRATCH];
    let n = match copy_rx(engine, rx, &mut scratch) {
        Ok(n) => n,
        Err(e) => {
            let _ = engine.release_rx(rx);
            return Err(e);
        }
    };

    let parsed = match decode(&scratch[..n]) {
        Ok(parsed) => parsed,
        Err(_) => {
            let _ = engine.release_rx(rx);
            return Ok(());
        }
    };

    if parsed.is_empty_ack_or_rst() {
        if let Some(tx) = engine.match_empty_ack_rst(&parsed, peer) {
            let _ = engine.release_tx(tx);
        }
        let _ = engine.release_rx(rx);
        return Ok(());
    }

    if parsed.is_empty() {
        let outcome = if parsed.ty() == Type::Confirmable {
            send_empty_ack(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    }

    if !parsed.code().is_request() {
        let _ = engine.release_rx(rx);
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
            let reply = site.dispatch(&request);
            send_reply(engine, io, meta, &reply)
        }
        Err(request::PathError::BadUtf8) => send_reply(engine, io, meta, &Reply::bad_request()),
        Err(request::PathError::TooLong) => send_reply(engine, io, meta, &Reply::not_found()),
    };
    let _ = engine.release_rx(rx);
    outcome
}

fn send_reply<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendReply,
    reply: &Reply,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots,
    T: DatagramIo,
{
    if meta.no_response.suppresses(reply.code()) {
        return Ok(());
    }

    let ty = match meta.ty {
        Type::Confirmable => Type::Acknowledgement,
        Type::NonConfirmable => Type::NonConfirmable,
        Type::Acknowledgement | Type::Reset => return Ok(()),
    };

    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };

    let cf = reply.format().map(crate::ContentFormat::encode);
    let max_age = reply.max_age_secs().map(EncodedUint::new);
    let observe = reply
        .observe_seq()
        .map(|seq| encode_uint(seq & 0x00ff_ffff));
    let mut opts = OptionsBuilder::<4>::new();
    if let Some(ref encoded) = observe {
        let _ = opts.push(Opt::observe(encoded));
    }
    if let Some(etag) = reply.etag_bytes() {
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

    if let Err(e) = engine.encode_tx(tx, &msg) {
        let _ = engine.release_tx(tx);
        return Err(Error::Message(e));
    }
    if let Err(e) = engine.set_tx_endpoint(tx, meta.dest) {
        let _ = engine.release_tx(tx);
        return Err(Error::Slot(e));
    }
    let send = engine.send_tx(io, tx);
    let _ = engine.release_tx(tx);
    send?;
    Ok(())
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

/// Failure of [`App::poll`].
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
