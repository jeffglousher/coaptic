//! Approachable CoAP app: route inbound work, send outbound requests.
//!
//! ```text
//! RX slot (+ body) --view--> Request
//! handler(Request) -> Response
//! Response --encode--> TX slot (+ body)
//!
//! Outgoing (get/put) --encode--> TX slot
//! poll matches Token + endpoint
//! RX --copy--> Response
//! ```
//!
//! [`Request`] is a borrowed view for the handler call only. [`Response`] is
//! owned intent — the same type [`App::take_response`] yields for a
//! completed [`Call`]. The reactor owns per-slot state machines inside
//! [`App::poll`]. This module is not a seventh memory area and does **not**
//! own a global mutable shared bag. Domain data that outlives a request
//! stays outside `App`.
//!
//! ```
//! use coaptic::storage::DatagramIo;
//! use coaptic::{
//!     App, ContentFormat, Endpoint, Request, Response, get, profiles,
//! };
//!
//! fn get_temp(_req: Request<'_>) -> Response {
//!     Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
//! }
//!
//! fn get_led(_: Request<'_>) -> Response {
//!     // Demo payload. Real LED state is firmware-owned, not an App bag.
//!     Response::content(b"off")
//! }
//!
//! fn put_led(_req: Request<'_>) -> Response {
//!     Response::changed()
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
//!     .route("sensors/temp", get(get_temp))
//!     .route("leds/0", get(get_led).put(put_led))
//!     .well_known_core()
//!     .bind(NullIo)
//!     .unwrap();
//! app.poll(0).unwrap();
//!
//! let peer = Endpoint::v4([192, 0, 2, 2], 5683);
//! let call = app.get("sensors/temp").to(peer).send(0).unwrap();
//! app.poll(0).unwrap();
//! let _ = app.take_response(call);
//! ```
//!
//! Site capacity defaults to [`DEFAULT_ROUTES`] (8); raise it with
//! [`.routes::<16>()`](AppBuilder::routes) before bind. Paths accept
//! `"sensors/temp"` or `&["sensors", "temp"]` ([`IntoPath`]).
//!
//! Observe: return [`.observe`](Response::observe) on a successful GET, or
//! attach [`MethodRouter::observe`]; later representations are
//! [`App::notify`]. Client subscribe is [`Outgoing::observe`] /
//! [`Outgoing::deregister`] on the same [`Call`]. Echo freshness
//! (RFC 9175 4.01) is [`AppBuilder::echo_freshness`].
//!
//! The happy path does not use [`Access`](crate::storage::Access) or
//! [`SlotId`]. Engine remains the advanced escape hatch
//! ([`App::engine_mut`]) for explicit slots, custom RST / remaining 4.xx,
//! and BERT edges (future / backlog).
mod client;
mod request;
mod response;
mod routing;
mod site;

#[cfg(test)]
mod tests;

use core::marker::PhantomData;

use crate::error::{BlockTransferError, BuildError, EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, Echo, EchoFreshness, EncodedUint, Ids, Message, MessageId, NoResponse, Opt,
    OptionsBuilder, Type, decode, encode_uint,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, DatagramIo, DatagramIoError, DatagramSlots, Endpoint, Engine,
    EngineBuilder, Exchanges, Memory, MemoryProfile, Missing, ObserveInterest, ObserveKey,
    ObserveResource, ObserveSlots, OutgoingBlock, PendingCons, Present, QBlockRecover, Retransmit,
    SlotError, SlotId, Storage, WithBodies,
};

/// RFC 7252 default Max-Age when a registration or notify omits it.
pub(crate) const DEFAULT_MAX_AGE_SECS: u32 = 60;

pub use client::{Call, Outgoing};
pub use request::{IntoPath, MAX_PATH_SEGMENTS, PathError, Request, split_path};
pub use response::{INLINE_PAYLOAD, IntoResponse, RESPONSE_BODY, Response};
pub use routing::{
    HandlerFn, Method, MethodRouter, ObserveSource, delete, fetch, get, ipatch, patch, post, put,
};
pub use site::{DEFAULT_ROUTES, Site};

/// Scratch for one inbound or outbound datagram (crate Default profile).
const DATAGRAM_SCRATCH: usize = 1472;

enum EngineSlot<P: MemoryProfile> {
    Datagram(Engine<Memory<P>>),
    BlockWise(Engine<Memory<P, WithBodies<P>>>),
}

/// CoAP app: profile memory + transport + a bounded [`Site`].
///
/// Handlers see a borrowed [`Request`] and return an owned [`Response`].
/// Outbound work is [`Self::get`] / [`Self::put`] → [`Outgoing::send`] →
/// [`Self::take_response`]. The reactor owns per-slot state machines
/// inside [`Self::poll`]. `N` is the maximum number of routes (default 8);
/// raise it with [`AppBuilder::routes`]. `App` does not own a global
/// mutable shared bag. You do not need [`crate::storage::Access`] on this
/// path — [`Self::engine_mut`] is the advanced escape hatch.
pub struct App<P: MemoryProfile = crate::profiles::Default, T = (), const N: usize = DEFAULT_ROUTES>
{
    engine: EngineSlot<P>,
    io: T,
    site: Site<N>,
    ids: Ids,
    tokens: u32,
    inbox: client::ClientInbox,
    lives: client::ClientLives,
    echo_fresh_ms: Option<u64>,
}

/// Builder: [`App::profile`] → [`block_wise`](Self::block_wise) →
/// [`route`](Self::route) → [`bind`](Self::bind).
///
/// [`Self::block_wise`] is required before bind (typestate). `true` enables
/// body pools for Block / Q-Block; `false` keeps datagram slots only.
pub struct AppBuilder<P: MemoryProfile, Block = Missing, const N: usize = DEFAULT_ROUTES> {
    block_wise: Option<bool>,
    site: Site<N>,
    echo_fresh_ms: Option<u64>,
    _p: PhantomData<P>,
    _b: PhantomData<Block>,
}

impl App {
    /// Start from a memory profile. Next: [`AppBuilder::block_wise`].
    #[must_use]
    pub const fn profile<P: MemoryProfile>() -> AppBuilder<P> {
        AppBuilder {
            block_wise: None,
            site: Site::new(),
            echo_fresh_ms: None,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<P: MemoryProfile, Block, const N: usize> AppBuilder<P, Block, N> {
    /// Site table size (default [`DEFAULT_ROUTES`]). Call before
    /// [`Self::route`].
    ///
    /// # Panics
    ///
    /// If a route is already registered.
    #[must_use]
    pub fn routes<const M: usize>(self) -> AppBuilder<P, Block, M> {
        assert!(
            self.site.is_empty(),
            "call .routes::<M>() before .route(...)"
        );
        let well_known = self.site.has_well_known_core();
        let mut site = Site::new();
        if well_known {
            site.well_known_core();
        }
        AppBuilder {
            block_wise: self.block_wise,
            site,
            echo_fresh_ms: self.echo_fresh_ms,
            _p: PhantomData,
            _b: PhantomData,
        }
    }

    /// Bind `methods` on Uri-Path `path`.
    ///
    /// `path` is [`IntoPath`]: `"sensors/temp"` or `&["sensors", "temp"]`.
    /// `methods` is a site router ([`get`], [`put`], …), not an outbound
    /// [`App::get`] builder.
    #[must_use]
    pub fn route(mut self, path: impl IntoPath, methods: MethodRouter) -> Self {
        self.site.route(path, methods);
        self
    }

    /// Bind `methods` on a `'static` URI-Path (`"/leds/0"`).
    #[must_use]
    pub fn route_path(mut self, path: &'static str, methods: MethodRouter) -> Self {
        self.site.route_path(path, methods);
        self
    }

    /// Serve `/.well-known/core` from registered paths.
    #[must_use]
    pub fn well_known_core(mut self) -> Self {
        self.site.well_known_core();
        self
    }

    /// Require a time-fresh Echo (RFC 9175) on inbound requests.
    ///
    /// Off by default. When set, [`App::poll`] classifies via
    /// [`Engine::echo_freshness`]. Missing, invalid, or stale Echo is 4.01
    /// with [`Response::problem`] and a minted Echo challenge. Fresh
    /// requests continue to the site. Handlers do not implement this.
    #[must_use]
    pub const fn echo_freshness(mut self, fresh_ms: u64) -> Self {
        self.echo_fresh_ms = Some(fresh_ms);
        self
    }
}

impl<P: MemoryProfile, const N: usize> AppBuilder<P, Missing, N> {
    /// Enable or disable body pools, then [`AppBuilder::bind`].
    #[must_use]
    pub fn block_wise(self, enabled: bool) -> AppBuilder<P, Present, N> {
        AppBuilder {
            block_wise: Some(enabled),
            site: self.site,
            echo_fresh_ms: self.echo_fresh_ms,
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
            site: self.site,
            ids: Ids::new(1),
            tokens: 0,
            inbox: client::ClientInbox::new(),
            lives: client::ClientLives::new(),
            echo_fresh_ms: self.echo_fresh_ms,
        })
    }
}

impl<P: MemoryProfile, T, const N: usize> App<P, T, N> {
    /// Bind `methods` on Uri-Path `path`.
    ///
    /// `path` is [`IntoPath`]: `&["sensors", "temp"]` or `"sensors/temp"`.
    pub fn route(&mut self, path: impl IntoPath, methods: MethodRouter) -> &mut Self {
        self.site.route(path, methods);
        self
    }

    /// Bind `methods` on a `'static` URI-Path (`"/leds/0"`).
    pub fn route_path(&mut self, path: &'static str, methods: MethodRouter) -> &mut Self {
        self.site.route_path(path, methods);
        self
    }

    /// Serve `/.well-known/core` from registered paths.
    pub const fn well_known_core(&mut self) -> &mut Self {
        self.site.well_known_core();
        self
    }

    /// Require a time-fresh Echo on inbound requests. See
    /// [`AppBuilder::echo_freshness`].
    pub const fn echo_freshness(&mut self, fresh_ms: u64) -> &mut Self {
        self.echo_fresh_ms = Some(fresh_ms);
        self
    }

    /// The site table.
    pub fn site(&mut self) -> &mut Site<N> {
        &mut self.site
    }

    /// Borrow the Engine (advanced path).
    ///
    /// The happy path is [`Self::poll`]. Use this when you need explicit
    /// slots or [`crate::storage::Access`].
    #[must_use]
    pub fn engine(&self) -> EngineRef<'_, P> {
        match &self.engine {
            EngineSlot::Datagram(engine) => EngineRef::Datagram(engine),
            EngineSlot::BlockWise(engine) => EngineRef::BlockWise(engine),
        }
    }

    /// Mutably borrow the Engine (advanced path).
    ///
    /// Escape hatch for explicit slots, [`crate::storage::Access`] /
    /// [`crate::storage::AccessMut`], custom RST / remaining 4.xx, and
    /// BERT edges (future / backlog). [`Self::poll`] already pins and
    /// releases; App handlers do not need this.
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
    Memory<P>: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges,
    Memory<P, WithBodies<P>>:
        Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges,
{
    /// One loop step: recv, progress, route, handler, send, release.
    ///
    /// `now_ms` is the caller clock (CON RTO, Observe Max-Age, Q-Block
    /// wait). No [`SlotId`] on this path.
    ///
    /// **Inbound.** Handlers see a borrowed [`Request`] and return an owned
    /// [`Response`]. Incomplete Block1 / Q-Block1 is 2.31 (handler not
    /// run); a complete body is [`Request::body`]. A large response ships
    /// as Block2 / Q-Block2 from the TX body. Site misses are 4.04 / 4.05
    /// with [`Response::problem`]. Apply-error 4.08 uses problem details;
    /// Q-Block1 holes after `NON_RECEIVE_TIMEOUT` use
    /// [`Response::missing_blocks`]. When [`Self::echo_freshness`] is set,
    /// a request that is not [`EchoFreshness::Fresh`] is 4.01 with a
    /// minted Echo. Observe register / deregister and [`ObserveSource`]
    /// notify run here; caller-built notifications use [`Self::notify`].
    ///
    /// **Outbound.** Token + peer match a [`Call`]; [`Self::take_response`]
    /// is the [`Response`] ([`Response::body`] when Block2 / Q-Block2
    /// assembled). Continues reuse the path from [`Outgoing::send`]. An
    /// Observe subscribe ([`Outgoing::observe`]) uses the same [`Call`].
    ///
    /// Retransmit: send on [`Retransmit::Due`], release on
    /// [`Retransmit::GiveUp`]. Advanced slots / [`Access`](crate::storage::Access)
    /// / remaining RST policy / BERT (future / backlog): [`Self::engine_mut`].
    pub fn poll(&mut self, now_ms: u64) -> Result<(), Error<T::Error>> {
        match &mut self.engine {
            EngineSlot::Datagram(engine) => poll_engine(
                engine,
                &mut self.io,
                &self.site,
                &mut self.ids,
                &mut self.inbox,
                &mut self.lives,
                self.echo_fresh_ms,
                now_ms,
            ),
            EngineSlot::BlockWise(engine) => poll_engine(
                engine,
                &mut self.io,
                &self.site,
                &mut self.ids,
                &mut self.inbox,
                &mut self.lives,
                self.echo_fresh_ms,
                now_ms,
            ),
        }
    }

    /// Send the current representation to every observer of `path`.
    ///
    /// Honors notification NSTART. CON when the row
    /// [`ObserveInterest::must_confirm`]; otherwise NON. Returns how many
    /// notifications were sent. Domain data stays in `response` — `App`
    /// does not hold it.
    pub fn notify(
        &mut self,
        now_ms: u64,
        path: &[&str],
        response: Response,
    ) -> Result<usize, Error<T::Error>> {
        let resource = ObserveResource::from_path(path);
        match &mut self.engine {
            EngineSlot::Datagram(engine) => notify_engine(
                engine,
                &mut self.io,
                &mut self.ids,
                now_ms,
                resource,
                &response,
            ),
            EngineSlot::BlockWise(engine) => notify_engine(
                engine,
                &mut self.io,
                &mut self.ids,
                now_ms,
                resource,
                &response,
            ),
        }
    }

    /// Mark observers of `path` due. [`Self::poll`] encodes via
    /// [`ObserveSource`] when progress surfaces the row.
    ///
    /// Returns how many rows were marked. Use [`Self::notify`] when you
    /// already have the [`Response`].
    pub fn signal(&mut self, path: &[&str]) -> usize {
        let resource = ObserveResource::from_path(path);
        match &mut self.engine {
            EngineSlot::Datagram(engine) => engine.signal_observe_resource(resource),
            EngineSlot::BlockWise(engine) => engine.signal_observe_resource(resource),
        }
    }
}

/// Borrowed Engine after `.block_wise(false)` or `.block_wise(true)`.
///
/// Advanced path. Prefer [`App::poll`]; see [`App::engine`].
pub enum EngineRef<'a, P: MemoryProfile> {
    /// Datagram pools only.
    Datagram(&'a Engine<Memory<P>>),
    /// Body pools included.
    BlockWise(&'a Engine<Memory<P, WithBodies<P>>>),
}

/// Mutable Engine after `.block_wise(false)` or `.block_wise(true)`.
///
/// Advanced path. Prefer [`App::poll`]; see [`App::engine_mut`].
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

#[allow(clippy::too_many_arguments)]
fn poll_engine<Mem, T, const N: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    site: &Site<N>,
    ids: &mut Ids,
    inbox: &mut client::ClientInbox,
    lives: &mut client::ClientLives,
    echo_fresh_ms: Option<u64>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges,
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
                client::forget_exchange_tx(engine, lives, pending.tx_slot());
                engine.release_tx(pending.tx_slot())?;
            }
        }
    }

    if let Some(rx) = progress.rx_ready().or(received) {
        dispatch_rx(
            engine,
            io,
            site,
            inbox,
            lives,
            ids,
            echo_fresh_ms,
            now_ms,
            rx,
        )?;
    }

    if let Some(expiry) = progress.observe_expired() {
        if let Some(interest) = engine.observe_interest(expiry.slot()) {
            let _ = engine.take_observe(interest.key());
        }
    }

    if let Some(id) = progress.observe_notify() {
        if let Some(interest) = engine.observe_interest(id) {
            if let Some(source) = site.observe_source(interest.resource()) {
                let response = source();
                send_notification(engine, io, ids, now_ms, interest, &response, interest.seq())?;
            }
        }
    }

    if let Some(recover) = progress.qblock_recover() {
        send_qblock_recover(engine, io, ids, recover)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum InboundBody {
    None,
    Continue,
    IncompleteEntity,
    Complete(SlotId),
}

fn assemble_inbound_body<Mem>(engine: &mut Engine<Mem>, rx: SlotId, now_ms: u64) -> InboundBody
where
    Mem: Storage + DatagramSlots + BodySlots,
{
    match engine.apply_block1_rx(rx) {
        Ok(progress) if progress.complete() => return InboundBody::Complete(progress.id()),
        Ok(_) => return InboundBody::Continue,
        Err(BlockTransferError::MissingBlock | BlockTransferError::NoBodyPools) => {}
        Err(_) => return InboundBody::IncompleteEntity,
    }
    match engine.apply_q_block1_rx(rx) {
        Ok(progress) if progress.complete() => {
            let _ = engine.note_q_receive(progress.id(), now_ms);
            InboundBody::Complete(progress.id())
        }
        Ok(progress) => {
            let _ = engine.note_q_receive(progress.id(), now_ms);
            InboundBody::Continue
        }
        Err(BlockTransferError::MissingBlock | BlockTransferError::NoBodyPools) => {
            InboundBody::None
        }
        Err(_) => InboundBody::IncompleteEntity,
    }
}

fn send_qblock_recover<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut Ids,
    recover: QBlockRecover,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    match recover.role() {
        BlockRole::IncomingQBlock2 => {
            let Some(tx) = engine.acquire_tx() else {
                return Err(Error::Saturated);
            };
            if let Err(e) = engine.encode_q_block2_recover_tx(
                recover,
                tx,
                Type::NonConfirmable,
                Code::GET,
                ids.next(),
            ) {
                let _ = engine.release_tx(tx);
                return Err(Error::Block(e));
            }
            let send = engine.send_tx(io, tx);
            let _ = engine.release_tx(tx);
            send?;
            Ok(())
        }
        BlockRole::IncomingQBlock1 => {
            let mut nums = [0u32; 16];
            let n = recover.copy_missing_nums(&mut nums);
            let meta = SendResponse {
                dest: recover.key().endpoint(),
                ty: Type::NonConfirmable,
                mid: ids.next(),
                token: recover.key().token(),
                no_response: NoResponse::DEFAULT,
                block2: None,
                q_block2: None,
                block1: None,
            };
            send_response(
                engine,
                io,
                meta,
                &Response::missing_blocks(nums.into_iter().take(n)),
            )
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_rx<Mem, T, const N: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    site: &Site<N>,
    inbox: &mut client::ClientInbox,
    lives: &mut client::ClientLives,
    ids: &mut Ids,
    echo_fresh_ms: Option<u64>,
    now_ms: u64,
    rx: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges,
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
        if parsed.is_empty_ack() {
            let _ = engine.ack_observe_con(parsed.message_id(), peer);
        } else {
            let _ = engine.take_observe(ObserveKey::new(parsed.token(), peer));
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
        return client::complete_client(engine, io, inbox, lives, ids, now_ms, peer, &parsed, rx);
    }

    let no_response = NoResponse::from_message(&parsed).unwrap_or(NoResponse::DEFAULT);
    let block2 = parsed.block2().and_then(Result::ok);
    let q_block2 = parsed.q_block2().next().and_then(Result::ok);
    let block1 = parsed.block1().and_then(Result::ok);
    let meta = SendResponse {
        dest: peer,
        ty: parsed.ty(),
        mid: parsed.message_id(),
        token: parsed.token(),
        no_response,
        block2,
        q_block2,
        block1,
    };

    if let Some(fresh_ms) = echo_fresh_ms {
        match Engine::<Mem>::echo_freshness(&parsed, now_ms, fresh_ms) {
            EchoFreshness::Fresh => {}
            EchoFreshness::Missing | EchoFreshness::Invalid | EchoFreshness::Stale => {
                let outcome = send_response(engine, io, meta, &unauthorized_echo(now_ms));
                let _ = engine.release_rx(rx);
                return outcome;
            }
        }
    }

    let assembled = assemble_inbound_body(engine, rx, now_ms);
    match assembled {
        InboundBody::Continue => {
            let outcome = send_response(engine, io, meta, &Response::new(Code::CONTINUE));
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::IncompleteEntity => {
            // Apply errors (duplicate NUM, SZX mismatch, overflow, …).
            // Window holes use `send_qblock_recover` → `Response::missing_blocks`.
            let outcome = send_response(
                engine,
                io,
                meta,
                &Response::problem(Code::REQUEST_ENTITY_INCOMPLETE)
                    .title("Request Entity Incomplete"),
            );
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::None | InboundBody::Complete(_) => {}
    }

    let (response, plan) = {
        let parsed = match engine.decode_rx(rx) {
            Ok(parsed) => parsed,
            Err(e) => {
                let _ = engine.release_rx(rx);
                return Err(Error::Message(e));
            }
        };
        let body = match assembled {
            InboundBody::Complete(id) => engine.rx_body_payload(id),
            InboundBody::None | InboundBody::Continue | InboundBody::IncompleteEntity => None,
        };
        match Request::from_decoded(parsed, peer, body) {
            Ok(request) => {
                let plan = ObservePlan::from_request(site, &request);
                (site.dispatch(request), plan)
            }
            Err(request::PathError::BadUtf8 | request::PathError::EmptySegment) => (
                Response::problem(Code::BAD_REQUEST).title("Bad Request"),
                ObservePlan::idle(),
            ),
            Err(request::PathError::TooLong) => (
                Response::problem(Code::NOT_FOUND).title("Not Found"),
                ObservePlan::idle(),
            ),
        }
    };
    let response = apply_observe(engine, now_ms, peer, response, plan);

    let outcome = send_response(engine, io, meta, &response);
    if let InboundBody::Complete(id) = assembled {
        let _ = engine.release_rx_body(id);
    }
    let _ = engine.release_rx(rx);
    outcome
}

#[derive(Clone, Copy)]
struct ObservePlan {
    register: bool,
    deregister: bool,
    delete: bool,
    has_source: bool,
    token: crate::message::Token,
    resource: ObserveResource,
}

impl ObservePlan {
    fn idle() -> Self {
        Self {
            register: false,
            deregister: false,
            delete: false,
            has_source: false,
            token: crate::message::Token::EMPTY,
            resource: ObserveResource::NONE,
        }
    }

    fn from_request<const N: usize>(site: &Site<N>, request: &Request<'_>) -> Self {
        Self {
            register: request.is_observe_register(),
            deregister: request.is_observe_deregister(),
            delete: request.method() == Some(Method::Delete),
            has_source: site.has_observe_source(request.path()),
            token: request.token(),
            resource: ObserveResource::from_path(request.path()),
        }
    }
}

fn apply_observe<S: Storage + ObserveSlots>(
    engine: &mut Engine<S>,
    now_ms: u64,
    peer: Endpoint,
    mut response: Response,
    plan: ObservePlan,
) -> Response {
    let key = ObserveKey::new(plan.token, peer);

    if plan.deregister {
        let _ = engine.take_observe(key);
    } else if plan.register && response.code().is_success() {
        let opted = response.observe_seq().is_some() || plan.has_source;
        if opted {
            let _ = engine.take_observe(key);
            if engine
                .insert_observe(ObserveInterest::new(plan.token, peer).with_resource(plan.resource))
                .is_some()
            {
                let max_age = response.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS);
                let _ = engine.refresh_observe_max_age(key, now_ms, max_age, None);
                response = response.observe(0);
            }
        }
    }
    if plan.delete && response.code().is_success() {
        let _ = engine.take_observe_resource(plan.resource);
    }
    response
}

fn notify_engine<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    resource: ObserveResource,
    response: &Response,
) -> Result<usize, Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: DatagramIo,
{
    if resource.is_none() {
        return Ok(0);
    }
    let n = engine.capacities().observe_entries;
    let mut sent = 0usize;
    for i in 0..n {
        let id = SlotId::from_index(i);
        let Some(interest) = engine.observe_interest(id) else {
            continue;
        };
        if interest.resource() != resource {
            continue;
        }
        if observe_endpoint_held(engine, interest.endpoint(), now_ms)
            >= usize::from(crate::message::Transmission::NSTART)
        {
            continue;
        }
        let Some(seq) = engine.next_observe_seq(id) else {
            continue;
        };
        let Some(interest) = engine.observe_interest(id) else {
            continue;
        };
        send_notification(engine, io, ids, now_ms, interest, response, seq)?;
        sent += 1;
    }
    Ok(sent)
}

fn observe_endpoint_held<S: Storage + ObserveSlots>(
    engine: &Engine<S>,
    endpoint: Endpoint,
    now_ms: u64,
) -> usize {
    let n = engine.capacities().observe_entries;
    (0..n)
        .filter(|&i| {
            engine
                .observe_interest(SlotId::from_index(i))
                .is_some_and(|row| row.endpoint() == endpoint && row.is_notify_held(now_ms))
        })
        .count()
}

fn send_notification<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    interest: ObserveInterest,
    response: &Response,
    seq: u32,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: DatagramIo,
{
    let ty = if interest.must_confirm(now_ms) {
        Type::Confirmable
    } else {
        Type::NonConfirmable
    };
    let mid = ids.next();
    let response = response.observe(seq);
    let dest = interest.endpoint();
    let token = interest.token();
    let key = BlockKey::new(token, dest);
    if let Some(body) = engine.lookup_tx_body(key) {
        let _ = engine.release_tx_body(body);
    }

    let meta = SendResponse {
        dest,
        ty,
        mid,
        token,
        no_response: NoResponse::DEFAULT,
        block2: None,
        q_block2: None,
        block1: None,
    };

    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid));
    let outcome = match encode_response(
        engine,
        tx,
        ty,
        mid,
        token,
        &response,
        response.payload(),
        None,
        None,
    ) {
        Ok(()) => finish_send(engine, io, tx, dest, pending),
        Err(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => {
            let _ = engine.release_tx(tx);
            start_notify_block2(engine, io, meta, &response, ty, pending)
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e.into())
        }
    };
    outcome?;

    let con_mid = (ty == Type::Confirmable).then_some(mid);
    let _ = engine.record_observe_notify(interest.key(), now_ms, con_mid);
    let max_age = response.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS);
    let _ = engine.refresh_observe_max_age(interest.key(), now_ms, max_age, con_mid);
    Ok(())
}

fn start_notify_block2<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
    ty: Type,
    pending: Option<(u64, MessageId)>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let key = BlockKey::new(meta.token, meta.dest);
    let id = match engine.start_block2(key, response.payload(), BlockValue::SZX_MAX) {
        Ok(id) => id,
        Err(BlockTransferError::NoBodyPools) => {
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )));
        }
        Err(e) => return Err(Error::Block(e)),
    };
    let outcome = issue_classic(engine, io, meta, response, ty, meta.mid, id, pending);
    if outcome.is_err() {
        let _ = engine.release_tx_body(id);
    }
    outcome
}

fn send_response<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    if meta.no_response.suppresses(response.code()) {
        return Ok(());
    }

    let ty = match meta.ty {
        Type::Confirmable => Type::Acknowledgement,
        Type::NonConfirmable => Type::NonConfirmable,
        Type::Acknowledgement | Type::Reset => return Ok(()),
    };

    let key = BlockKey::new(meta.token, meta.dest);
    if let Some(id) = engine.lookup_tx_body(key) {
        if engine.tx_body_transfer(id).is_some_and(|t| {
            matches!(
                t.role(),
                BlockRole::OutgoingBlock2 | BlockRole::OutgoingQBlock2
            )
        }) {
            return continue_outgoing(engine, io, meta, response, ty, id);
        }
    }

    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };

    match encode_response(
        engine,
        tx,
        ty,
        meta.mid,
        meta.token,
        response,
        response.payload(),
        None,
        meta.block1,
    ) {
        Ok(()) => finish_send(engine, io, tx, meta.dest, None),
        Err(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => {
            let _ = engine.release_tx(tx);
            start_outgoing(engine, io, meta, response, ty, key)
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e.into())
        }
    }
}

fn start_outgoing<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
    ty: Type,
    key: BlockKey,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let szx = szx_for(meta.block2, meta.q_block2);
    let started = if meta.q_block2.is_some() {
        engine.start_q_block2(key, response.payload(), szx)
    } else {
        engine.start_block2(key, response.payload(), szx)
    };
    let id = match started {
        Ok(id) => id,
        Err(BlockTransferError::NoBodyPools) => {
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )));
        }
        Err(e) => return Err(Error::Block(e)),
    };
    let outcome = if meta.q_block2.is_some() {
        issue_q_window(engine, io, meta, response, ty, id)
    } else {
        issue_classic(engine, io, meta, response, ty, meta.mid, id, None)
    };
    if outcome.is_err() {
        let _ = engine.release_tx_body(id);
    }
    outcome
}

fn continue_outgoing<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
    ty: Type,
    id: SlotId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let role = engine
        .tx_body_transfer(id)
        .map(|t| t.role())
        .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
    match role {
        BlockRole::OutgoingQBlock2 => match meta.q_block2 {
            Some(q) if q.more() => {
                engine.ack_q_block2(id, q.num()).map_err(Error::Block)?;
                issue_q_window(engine, io, meta, response, ty, id)
            }
            Some(_) => Ok(()),
            None => issue_q_window(engine, io, meta, response, ty, id),
        },
        BlockRole::OutgoingBlock2 => {
            issue_classic(engine, io, meta, response, ty, meta.mid, id, None)
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn issue_classic<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
    ty: Type,
    mid: MessageId,
    id: SlotId,
    pending: Option<(u64, MessageId)>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let issued = engine.next_block2(id).map_err(Error::Block)?;
    send_issued(engine, io, meta, response, ty, mid, issued, false, pending)?;
    if issued.complete() {
        let _ = engine.release_tx_body(id);
    }
    Ok(())
}

fn issue_q_window<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
    first_ty: Type,
    id: SlotId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let mut extra = 0u16;
    loop {
        let issued = match engine.next_q_block2(id) {
            Ok(issued) => issued,
            Err(BlockTransferError::OutsideWindow) => return Ok(()),
            Err(e) => return Err(Error::Block(e)),
        };
        let (ty, mid) = if extra == 0 {
            (first_ty, meta.mid)
        } else {
            (Type::NonConfirmable, meta.mid.wrapping_add(extra))
        };
        send_issued(engine, io, meta, response, ty, mid, issued, true, None)?;
        extra = extra.saturating_add(1);
        if issued.complete() {
            let _ = engine.release_tx_body(id);
            return Ok(());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_issued<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response,
    ty: Type,
    mid: MessageId,
    issued: OutgoingBlock,
    q_block: bool,
    pending: Option<(u64, MessageId)>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let mut chunk = [0u8; 1024];
    let n = copy_issued(engine, issued, &mut chunk).map_err(Error::Block)?;
    let size2 = if q_block {
        engine.tx_body_transfer(issued.id()).map(|t| t.filled())
    } else {
        None
    };
    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
    let block = Some(BlockOpt {
        value: issued.block(),
        q_block,
        size2,
    });
    if let Err(e) = encode_response(
        engine,
        tx,
        ty,
        mid,
        meta.token,
        response,
        &chunk[..n],
        block,
        meta.block1,
    ) {
        let _ = engine.release_tx(tx);
        return Err(e.into());
    }
    finish_send(engine, io, tx, meta.dest, pending)
}

fn copy_issued<S: Storage + BodySlots>(
    engine: &Engine<S>,
    issued: OutgoingBlock,
    dest: &mut [u8],
) -> Result<usize, BlockTransferError> {
    let payload = engine
        .tx_body_payload(issued.id())
        .ok_or(BlockTransferError::NoTransfer)?;
    let end = issued
        .offset()
        .checked_add(issued.len())
        .ok_or(BlockTransferError::Overflow)?;
    if end > payload.len() || issued.len() > dest.len() {
        return Err(BlockTransferError::Overflow);
    }
    dest[..issued.len()].copy_from_slice(&payload[issued.offset()..end]);
    Ok(issued.len())
}

#[allow(clippy::too_many_arguments)]
fn encode_response<S: Storage + DatagramSlots>(
    engine: &mut Engine<S>,
    tx: SlotId,
    ty: Type,
    mid: MessageId,
    token: crate::message::Token,
    response: &Response,
    payload: &[u8],
    block: Option<BlockOpt>,
    block1: Option<BlockValue>,
) -> Result<(), SlotMessageError> {
    let cf = response.format().map(crate::ContentFormat::encode);
    let max_age = response.max_age_secs().map(EncodedUint::new);
    let observe = response
        .observe_seq()
        .map(|seq| encode_uint(seq & 0x00ff_ffff));
    let block_enc = block.map(|b| b.value.encode());
    let size2_enc = block
        .and_then(|b| b.size2)
        .map(|n| encode_uint(u32::try_from(n).unwrap_or(u32::MAX)));
    let block1_enc = block1.map(|b| b.encode());
    let echo = response.echo_option();
    let mut opts = OptionsBuilder::<8>::new();
    if let Some(etag) = response.etag_bytes() {
        let _ = opts.push(Opt::etag(etag));
    }
    if let Some(ref encoded) = observe {
        let _ = opts.push(Opt::observe(encoded));
    }
    if let Some(ref encoded) = cf {
        let _ = opts.push(Opt::content_format(encoded));
    }
    if let Some(ref encoded) = max_age {
        let _ = opts.push(Opt::max_age(encoded));
    }
    if let Some(ref encoded) = block_enc {
        if block.is_some_and(|b| b.q_block) {
            if let Some(ref size2) = size2_enc {
                let _ = opts.push(Opt::size2(size2));
            }
            let _ = opts.push(Opt::q_block2(encoded));
        } else {
            let _ = opts.push(Opt::block2(encoded));
        }
    }
    if let Some(ref encoded) = block1_enc {
        let _ = opts.push(Opt::block1(encoded));
    }
    if let Some(ref echo) = echo {
        let _ = opts.push(Opt::echo(echo.as_slice()));
    }
    let msg = Message::new(ty, response.code(), mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    engine.encode_tx(tx, &msg).map(|_| ())
}

pub(crate) fn finish_send<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    tx: SlotId,
    dest: Endpoint,
    pending: Option<(u64, MessageId)>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons,
    T: DatagramIo,
{
    if let Err(e) = engine.set_tx_endpoint(tx, dest) {
        let _ = engine.release_tx(tx);
        return Err(Error::Slot(e));
    }
    let send = engine.send_tx(io, tx);
    if let Some((now_ms, mid)) = pending {
        let _ = engine.record_pending_con(tx, dest, mid, now_ms, 0);
        send?;
        return Ok(());
    }
    let _ = engine.release_tx(tx);
    send?;
    Ok(())
}

fn unauthorized_echo(now_ms: u64) -> Response {
    let challenge = Echo::mint(now_ms, &[]).expect("timestamp Echo");
    Response::problem(Code::UNAUTHORIZED)
        .title("Unauthorized")
        .echo(challenge)
}

fn szx_for(block2: Option<BlockValue>, q_block2: Option<BlockValue>) -> u8 {
    match q_block2.or(block2) {
        Some(block) if !block.is_bert() => block.szx(),
        _ => BlockValue::SZX_MAX,
    }
}

#[derive(Clone, Copy)]
struct BlockOpt {
    value: BlockValue,
    q_block: bool,
    size2: Option<usize>,
}

#[derive(Clone, Copy)]
struct SendResponse {
    dest: Endpoint,
    ty: Type,
    mid: MessageId,
    token: crate::message::Token,
    no_response: NoResponse,
    block2: Option<BlockValue>,
    q_block2: Option<BlockValue>,
    /// Echo of the request Block1 (RFC 7959 §2.5 Continue / final).
    block1: Option<BlockValue>,
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

pub(crate) fn send_empty_ack<S, T>(
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

/// Failure of [`App::poll`] or [`Outgoing::send`].
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
    /// Block / Q-Block body start, issue, or recover encode failed.
    Block(BlockTransferError),
    /// Uri-Path has more than [`MAX_PATH_SEGMENTS`] segments.
    Path,
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
            Self::Block(e) => write!(f, "{e}"),
            Self::Path => f.write_str("uri-path has too many segments"),
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
            Self::Block(e) => Some(e),
            Self::Path => None,
        }
    }
}
