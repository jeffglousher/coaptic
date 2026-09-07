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
//! fn get_temp(_req: Request<'_>) -> Response<'static> {
//!     Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
//! }
//!
//! fn get_led(_: Request<'_>) -> Response<'static> {
//!     // Demo payload. Real LED state is firmware-owned, not an App bag.
//!     Response::content(b"off")
//! }
//!
//! fn put_led(_req: Request<'_>) -> Response<'static> {
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
//!     .block_wise::<true>()
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
//! (RFC 9175 4.01) is [`AppBuilder::echo_freshness`]. Pairwise OSCORE
//! (feature `oscore`) is `App::set_oscore`.
//!
//! The happy path does not use [`Access`](crate::storage::Access) or
//! [`SlotId`]. Engine remains the advanced escape hatch
//! ([`App::engine_mut`]) for explicit slots, custom RST / remaining 4.xx,
//! and BERT edges (future / backlog).
mod client;
mod oscore;
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
    OptionsBuilder, ParsedMessage, Transmission, Type, decode, encode, encode_uint,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, DatagramIo, DatagramIoError, DatagramSlots, DedupEntry,
    DedupKey, DedupSlots, Endpoint, Engine, EngineBuilder, Exchanges, Memory, MemoryLayout,
    MemoryProfile, Metrics, Missing, ObserveInterest, ObserveKey, ObserveResource, ObserveSlots,
    OutgoingBlock, PendingCons, Present, QBlockRecover, Retransmit, SlotError, SlotId, Storage,
    WithBodies,
};

/// RFC 7252 default Max-Age when a registration or notify omits it.
pub(crate) const DEFAULT_MAX_AGE_SECS: u32 = 60;

pub use client::{Call, Outgoing};
pub use request::{IntoPath, MAX_PATH_SEGMENTS, PathError, Request, split_path};
pub use response::{
    AppAssembled, INLINE_PAYLOAD, IntoResponse, LOCATION_MAX, RESPONSE_BODY, Response,
};
pub use routing::{
    HandlerFn, Method, MethodRouter, ObserveSource, delete, fetch, get, ipatch, patch, post, put,
};
pub use site::{DEFAULT_ROUTES, LINK_FORMAT_PER_ROUTE, Site, link_format_capacity};

use response::AssembledField;

/// Scratch for one inbound or outbound datagram (crate Default profile).
const DATAGRAM_SCRATCH: usize = 1472;

/// Engine storage selected by [`AppBuilder::block_wise`].
pub(crate) type AppStore<P, const BLOCK_WISE: bool> = <P as MemoryLayout<BLOCK_WISE>>::Store;

/// CoAP app: profile memory + transport + a bounded [`Site`].
///
/// Handlers see a borrowed [`Request`] and return an owned [`Response`].
/// Outbound work is [`Self::get`] / [`Self::put`] → [`Outgoing::send`] →
/// [`Self::take_response`]. The reactor owns per-slot state machines
/// inside [`Self::poll`]. `N` is the maximum number of routes (default 8);
/// raise it with [`AppBuilder::routes`]. `BLOCK_WISE` is
/// [`AppBuilder::block_wise`]: `false` stores only [`Memory<P>`] (no body
/// pool arrays and no client Block2 assembled hold). You do not need
/// [`crate::storage::Access`] on this path — [`Self::engine_mut`] is the
/// advanced escape hatch.
pub struct App<
    P: MemoryProfile = crate::profiles::Default,
    T = (),
    const N: usize = DEFAULT_ROUTES,
    const BLOCK_WISE: bool = false,
> where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    engine: Engine<AppStore<P, BLOCK_WISE>>,
    io: T,
    site: Site<N>,
    ids: Ids,
    tokens: u32,
    inbox: client::ClientInbox,
    lives: client::ClientLives,
    echo_fresh_ms: Option<u64>,
    /// Caller-owned OSCORE context (`()` without the `oscore` feature).
    oscore: oscore::Field,
    /// Client Block2 / Q-Block2 snapshot for [`Self::take_response`].
    /// Present when `BLOCK_WISE`; zero-sized otherwise.
    assembled: AssembledField<P, BLOCK_WISE>,
}

/// Builder: [`App::profile`] → [`block_wise`](Self::block_wise) →
/// [`route`](Self::route) → [`bind`](Self::bind).
///
/// [`Self::block_wise`] is required before bind (typestate).
/// `.block_wise::<true>()` enables body pools for Block / Q-Block and the
/// client assembled-body hold; `.block_wise::<false>()` keeps datagram
/// slots only and does not reserve body-pool or assembled-hold RAM on
/// [`App`].
pub struct AppBuilder<
    P: MemoryProfile,
    Block = Missing,
    const N: usize = DEFAULT_ROUTES,
    const BLOCK_WISE: bool = false,
> {
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
            site: Site::new(),
            echo_fresh_ms: None,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<P: MemoryProfile, Block, const N: usize, const BLOCK_WISE: bool>
    AppBuilder<P, Block, N, BLOCK_WISE>
{
    /// Site table size (default [`DEFAULT_ROUTES`]). Call before
    /// [`Self::route`].
    ///
    /// # Panics
    ///
    /// If a route is already registered.
    #[must_use]
    pub fn routes<const M: usize>(self) -> AppBuilder<P, Block, M, BLOCK_WISE> {
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
    ///
    /// Catalog capacity is compile-time ([`link_format_capacity`] after
    /// [`Self::routes`]). Overflow is 5.00, not a truncated list.
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

impl<P: MemoryProfile, const N: usize, const PREV: bool> AppBuilder<P, Missing, N, PREV> {
    /// Enable or disable body pools, then [`AppBuilder::bind`].
    ///
    /// The flag is a const generic so [`App`] RAM matches Storage:
    /// `.block_wise::<false>()` does not reserve RX/TX body arrays or the
    /// client Block2 assembled hold.
    #[must_use]
    pub fn block_wise<const ENABLED: bool>(self) -> AppBuilder<P, Present, N, ENABLED> {
        AppBuilder {
            site: self.site,
            echo_fresh_ms: self.echo_fresh_ms,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<P, const N: usize> AppBuilder<P, Present, N, false>
where
    P: MemoryProfile + MemoryLayout<false, Store = Memory<P>>,
{
    /// Construct profile [`Memory`] (datagram pools only) and bind `io`.
    pub fn bind<T>(self, io: T) -> Result<App<P, T, N, false>, BuildError> {
        let engine = EngineBuilder::new()
            .profile::<P>()
            .block_wise(false)
            .build(Memory::<P>::new())?;
        Ok(App {
            engine,
            io,
            site: self.site,
            ids: Ids::new(1),
            tokens: 0,
            inbox: client::ClientInbox::new(),
            lives: client::ClientLives::new(),
            echo_fresh_ms: self.echo_fresh_ms,
            oscore: oscore::empty_field(),
            assembled: Default::default(),
        })
    }
}

impl<P, const N: usize> AppBuilder<P, Present, N, true>
where
    P: MemoryProfile + MemoryLayout<true, Store = Memory<P, WithBodies<P>>>,
{
    /// Construct profile [`Memory`] with body pools and bind `io`.
    pub fn bind<T>(self, io: T) -> Result<App<P, T, N, true>, BuildError> {
        let engine = EngineBuilder::new()
            .profile::<P>()
            .block_wise(true)
            .build(Memory::<P>::with_block_wise())?;
        Ok(App {
            engine,
            io,
            site: self.site,
            ids: Ids::new(1),
            tokens: 0,
            inbox: client::ClientInbox::new(),
            lives: client::ClientLives::new(),
            echo_fresh_ms: self.echo_fresh_ms,
            oscore: oscore::empty_field(),
            assembled: Default::default(),
        })
    }
}

impl<
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
> App<P, T, N, BLOCK_WISE>
{
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
    ///
    /// See [`AppBuilder::well_known_core`].
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
    pub const fn engine(&self) -> &Engine<AppStore<P, BLOCK_WISE>> {
        &self.engine
    }

    /// Mutably borrow the Engine (advanced path).
    ///
    /// Escape hatch for explicit slots, [`crate::storage::Access`] /
    /// [`crate::storage::AccessMut`], custom RST / remaining 4.xx, and
    /// BERT edges (future / backlog). [`Self::poll`] already pins and
    /// releases; App handlers do not need this.
    pub const fn engine_mut(&mut self) -> &mut Engine<AppStore<P, BLOCK_WISE>> {
        &mut self.engine
    }

    /// Attach a caller-owned pairwise OSCORE context.
    ///
    /// Requires the `oscore` crate feature. You derive
    /// [`crate::oscore::SecurityContext`] (Master Secret, Sender/Recipient
    /// IDs, replay window). Engine does not store keys. Protect happens
    /// when a datagram is encoded into a TX slot, so CON retransmit
    /// reuses the same ciphertext.
    ///
    /// This slice covers a single pairwise context on the happy-path
    /// request/response. Observe notifications, outer Block-wise, group
    /// OSCORE, and other ciphers are out.
    ///
    /// Fail-closed while a context is attached: a non-empty datagram
    /// without an OSCORE option is rejected (4.01 on a request; a plain
    /// 2.xx does not complete a [`Call`]). Empty ACK/RST stay unprotected
    /// (RFC 7252 reliability). AEAD failure is a silent drop.
    #[cfg(feature = "oscore")]
    pub fn set_oscore(&mut self, ctx: crate::oscore::SecurityContext) -> &mut Self {
        self.oscore = Some(ctx);
        self
    }

    /// Borrow the attached OSCORE context, if any.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub const fn oscore(&self) -> Option<&crate::oscore::SecurityContext> {
        self.oscore.as_ref()
    }

    /// Mutably borrow the attached OSCORE context.
    #[cfg(feature = "oscore")]
    pub const fn oscore_mut(&mut self) -> Option<&mut crate::oscore::SecurityContext> {
        self.oscore.as_mut()
    }

    /// Copy of Engine reactor counters.
    ///
    /// Always-on wrapping `u32` fields at recv/send/`progress` sites. A
    /// `std` dogfood bin can print after a run:
    ///
    /// ```
    /// # use coaptic::storage::DatagramIo;
    /// # use coaptic::{App, Endpoint, profiles};
    /// # struct NullIo;
    /// # impl DatagramIo for NullIo {
    /// #     type Error = &'static str;
    /// #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
    /// #         Ok(None)
    /// #     }
    /// #     fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> { Ok(0) }
    /// # }
    /// let mut app = App::profile::<profiles::Default>()
    ///     .block_wise::<false>()
    ///     .bind(NullIo)
    ///     .unwrap();
    /// app.poll(0).unwrap();
    /// let snap = app.metrics();
    /// assert_eq!(snap.progress, 1);
    /// ```
    ///
    /// See [`crate::storage::Metrics`] and [`Engine::metrics`].
    #[must_use]
    pub const fn metrics(&self) -> Metrics {
        self.engine.metrics()
    }

    /// Zero Engine reactor counters.
    pub fn reset_metrics(&mut self) {
        self.engine.reset_metrics();
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

impl<P, T, const N: usize, const BLOCK_WISE: bool> App<P, T, N, BLOCK_WISE>
where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
    T: DatagramIo,
{
    /// One loop step: recv, progress, route, handler, send, release.
    ///
    /// `now_ms` is the caller clock (CON RTO, Observe Max-Age, Q-Block
    /// wait). No [`SlotId`] on this path.
    ///
    /// **Inbound.** Handlers see a borrowed [`Request`] and return an owned
    /// [`Response`]. Empty CON (code 0.00) is answered with empty RST
    /// (RFC 7252 ping). Incomplete Block1 / Q-Block1 is 2.31 (handler not
    /// run); a complete body is [`Request::body`]. A large response ships
    /// as Block2 / Q-Block2 from the TX body. Site misses are 4.04 / 4.05
    /// with [`Response::problem`]. Apply-error 4.08 uses problem details;
    /// Q-Block1 holes after `NON_RECEIVE_TIMEOUT` use
    /// [`Response::missing_blocks`]. When [`Self::echo_freshness`] is set,
    /// a request that is not [`EchoFreshness::Fresh`] is 4.01 with a
    /// minted Echo. Location-Path / Location-Query on the [`Response`]
    /// are written on the wire. [`Response::separate`] is an empty ACK
    /// to a CON request, then the representation in a later CON (new
    /// Message ID; NON request → NON). A retransmitted CON request (same
    /// Message ID + peer) is answered from the Dedup Table without a
    /// second handler call while the row is live (`EXCHANGE_LIFETIME`):
    /// the cached ACK bytes, or a pinned TX slot when the ACK does not
    /// fit the compact sidecar. POST / PATCH / FETCH never re-run on a
    /// live row (empty ACK if there is nothing to replay). GET / PUT /
    /// DELETE / iPATCH replay when a cache exists; without one, re-run is
    /// allowed (RFC 7252 §4.5 MAY). After `EXCHANGE_LIFETIME` expiry,
    /// Dedup Miss is a new exchange (POST / PATCH / FETCH may re-run).
    /// An OSCORE CON retransmit is keyed on the *outer* Message ID so the
    /// cached protected ACK is replayed without a second unprotect.
    /// Observe register / deregister and
    /// [`ObserveSource`] notify run here; caller-built notifications use
    /// [`Self::notify`]. Empty RST matching a notification Message ID
    /// drops that observer (RFC 7641 §4.5; RST has no Token).
    ///
    /// **Outbound.** Token + peer match a [`Call`]; [`Self::take_response`]
    /// is the [`Response`] ([`Response::body`] when Block2 / Q-Block2
    /// assembled). Continues reuse the path from [`Outgoing::send`]. Uri-Query,
    /// Accept, ETag, If-Match / If-None-Match, and early Block2 are set on
    /// [`Outgoing`]. An Observe subscribe ([`Outgoing::observe`]) uses the
    /// same [`Call`].
    ///
    /// **Retransmit.** Engine schedules CON RTO inside [`Engine::progress`]
    /// / [`Engine::poll_retransmit`] and does not send. This poll sends on
    /// [`Retransmit::Due`] and releases on [`Retransmit::GiveUp`]. A matching
    /// response (piggybacked ACK or separate CON/NON) clears that pending CON
    /// using the request Message ID. Empty RST matching a client outstanding
    /// request forgets the exchange. Empty ACK then silence, and lost NON,
    /// expire after RFC 7252 §4.8.2 `EXCHANGE_LIFETIME` / `NON_LIFETIME`.
    /// `now_ms` is the caller clock (jitter is 0 from [`Outgoing::send`]).
    /// Advanced slots / [`Access`](crate::storage::Access) / remaining RST
    /// policy / BERT (future / backlog): [`Self::engine_mut`].
    pub fn poll(&mut self, now_ms: u64) -> Result<(), Error<T::Error>> {
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            self.echo_fresh_ms,
            now_ms,
        )
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
        response: Response<'static>,
    ) -> Result<usize, Error<T::Error>> {
        let resource = ObserveResource::from_path(path);
        notify_engine(
            &mut self.engine,
            &mut self.io,
            &mut self.ids,
            now_ms,
            resource,
            &response,
        )
    }

    /// Mark observers of `path` due. [`Self::poll`] encodes via
    /// [`ObserveSource`] when progress surfaces the row.
    ///
    /// Returns how many rows were marked. Use [`Self::notify`] when you
    /// already have the [`Response`].
    pub fn signal(&mut self, path: &[&str]) -> usize {
        let resource = ObserveResource::from_path(path);
        self.engine.signal_observe_resource(resource)
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
    oscore: &mut oscore::Field,
    echo_fresh_ms: Option<u64>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges + DedupSlots,
    T: DatagramIo,
{
    // RX pool full must not skip RTO / Observe / Q-Block recover. Surface
    // Saturated after those timers still run (RFC 7252 §4.2).
    let (received, recv_saturated) = match engine.recv_from(io) {
        Ok(id) => (id, false),
        Err(DatagramIoError::Saturated) => (None, true),
        Err(e) => return Err(e.into()),
    };
    expire_request_dedup(engine, now_ms);
    client::expire_client_exchanges(engine, inbox, lives, now_ms);
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
            oscore,
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
            let seq = interest.seq();
            match site.observe_source(interest.resource()) {
                Some(source) => {
                    let response = source();
                    if let Err(e) =
                        send_notification(engine, io, ids, now_ms, interest, &response, seq)
                    {
                        let _ = engine.restore_observe_due(id, seq);
                        return Err(e);
                    }
                }
                None => {
                    let _ = engine.restore_observe_due(id, seq);
                }
            }
        }
    }

    if let Some(recover) = progress.qblock_recover() {
        send_qblock_recover(engine, io, ids, now_ms, recover)?;
    }
    if recv_saturated {
        return Err(Error::Saturated);
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
        // Retransmit of an already-acked NUM (RFC 7959): replay 2.31, not 4.08.
        Err(BlockTransferError::Overlap | BlockTransferError::AlreadyComplete) => {
            return InboundBody::Continue;
        }
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
    now_ms: u64,
    recover: QBlockRecover,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
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
                oscore: oscore::no_request(),
            };
            send_response(
                engine,
                io,
                meta,
                &Response::missing_blocks(nums.into_iter().take(n)),
                now_ms,
                &oscore::empty_field(),
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
    oscore: &mut oscore::Field,
    echo_fresh_ms: Option<u64>,
    now_ms: u64,
    rx: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges + DedupSlots,
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
            Metrics::inc(&mut engine.metrics_mut().rx_error);
            let _ = engine.release_rx(rx);
            return Ok(());
        }
    };

    if parsed.is_empty_ack_or_rst() {
        if parsed.is_empty_rst() {
            client::complete_client_rst(engine, inbox, lives, parsed.message_id(), peer);
        }
        if let Some(tx) = engine.match_empty_ack_rst(&parsed, peer) {
            let _ = engine.release_tx(tx);
        }
        if parsed.is_empty_ack() {
            let _ = engine.ack_observe_con(parsed.message_id(), peer);
        } else {
            // RST is Empty (no Token). Match the notify Message ID.
            let _ = engine.reject_observe_notify(parsed.message_id(), peer);
        }
        let _ = engine.release_rx(rx);
        return Ok(());
    }

    if parsed.is_empty() {
        let outcome = if parsed.ty() == Type::Confirmable {
            send_empty_rst(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    }

    // Dedup keys the *outer* Message ID + peer. OSCORE CON retransmit
    // must replay the cached protected ACK before unprotect, or the
    // replay window consumes the Partial IV and the client never ACKs.
    if parsed.code().is_request() && parsed.ty() == Type::Confirmable {
        match replay_con_request(engine, io, peer, &parsed, now_ms) {
            Replay::Hit(outcome) => {
                let _ = engine.release_rx(rx);
                return outcome;
            }
            Replay::Miss => {}
        }
    }

    let mut inner_scratch = [0u8; DATAGRAM_SCRATCH];
    let opened = match oscore::inbound(oscore, &parsed, &mut inner_scratch) {
        Ok(opened) => opened,
        Err(e) => {
            let outcome = oscore_inbound_error(engine, io, &parsed, peer, now_ms, e);
            let _ = engine.release_rx(rx);
            return outcome;
        }
    };
    let (parsed, oscore_req) = match opened {
        Some((inner, req)) => {
            #[cfg(feature = "oscore")]
            {
                (inner, Some(req))
            }
            #[cfg(not(feature = "oscore"))]
            {
                (inner, req)
            }
        }
        None => (parsed, oscore::no_request()),
    };

    if !parsed.code().is_request() {
        return client::complete_client(engine, io, inbox, lives, ids, now_ms, peer, &parsed, rx);
    }

    let no_response = NoResponse::from_message(&parsed).unwrap_or(NoResponse::DEFAULT);
    if matches!(parsed.block2(), Some(Err(_)))
        || matches!(parsed.q_block2().next(), Some(Err(_)))
        || matches!(parsed.block1(), Some(Err(_)))
    {
        let meta = SendResponse {
            dest: peer,
            ty: parsed.ty(),
            mid: parsed.message_id(),
            token: parsed.token(),
            no_response,
            block2: None,
            q_block2: None,
            block1: None,
            oscore: oscore_req,
        };
        let outcome = send_response(
            engine,
            io,
            meta,
            &Response::problem(Code::BAD_OPTION).title("Bad Option"),
            now_ms,
            oscore,
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }
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
        oscore: oscore_req,
    };

    if let Some(fresh_ms) = echo_fresh_ms {
        match Engine::<Mem>::echo_freshness(&parsed, now_ms, fresh_ms) {
            EchoFreshness::Fresh => {}
            EchoFreshness::Missing | EchoFreshness::Invalid | EchoFreshness::Stale => {
                let outcome =
                    send_response(engine, io, meta, &unauthorized_echo(now_ms), now_ms, oscore);
                let _ = engine.release_rx(rx);
                return outcome;
            }
        }
    }

    let assembled = assemble_inbound_body(engine, rx, now_ms);
    match assembled {
        InboundBody::Continue => {
            let outcome =
                send_response(engine, io, meta, &Response::new(Code::CONTINUE), now_ms, oscore);
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::IncompleteEntity => {
            // Apply errors (gap, SZX mismatch, overflow, Q-Block duplicate, …).
            // Classic Overlap / AlreadyComplete replay 2.31 above.
            // Window holes use `send_qblock_recover` → `Response::missing_blocks`.
            let outcome = send_response(
                engine,
                io,
                meta,
                &Response::problem(Code::REQUEST_ENTITY_INCOMPLETE)
                    .title("Request Entity Incomplete"),
                now_ms,
                oscore,
            );
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::None | InboundBody::Complete(_) => {}
    }

    let mut catalog = [0u8; RESPONSE_BODY];
    let (response, plan) = {
        let body = match assembled {
            InboundBody::Complete(id) => engine.rx_body_payload(id),
            InboundBody::None | InboundBody::Continue | InboundBody::IncompleteEntity => None,
        };
        match Request::from_decoded(parsed, peer, body) {
            Ok(request) => {
                let plan = ObservePlan::from_request(site, &request);
                (site.dispatch(request, &mut catalog), plan)
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

    let outcome = if response.is_separate() {
        send_separate(engine, io, ids, now_ms, meta, &response, oscore)
    } else {
        send_response(engine, io, meta, &response, now_ms, oscore)
    };
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

fn apply_observe<'a, S: Storage + ObserveSlots>(
    engine: &mut Engine<S>,
    now_ms: u64,
    peer: Endpoint,
    mut response: Response<'a>,
    plan: ObservePlan,
) -> Response<'a> {
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
            } else {
                response = response.without_observe();
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
    response: &Response<'_>,
) -> Result<usize, Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: DatagramIo,
{
    if resource.is_none() {
        return Ok(0);
    }
    let n = engine.capacities().observe_entries;
    let mut held = NotifyHeldCache::from_engine(engine, now_ms);
    let mut sent = 0usize;
    for i in 0..n {
        let id = SlotId::from_index(i);
        let Some(interest) = engine.observe_interest(id) else {
            continue;
        };
        if interest.resource() != resource {
            continue;
        }
        if held.count(engine, interest.endpoint(), now_ms)
            >= usize::from(crate::message::Transmission::NSTART)
        {
            Metrics::inc(&mut engine.metrics_mut().nstart_reject);
            continue;
        }
        let Some(seq) = engine.next_observe_seq(id) else {
            continue;
        };
        let Some(interest) = engine.observe_interest(id) else {
            continue;
        };
        let dest = interest.endpoint();
        if let Err(e) = send_notification(engine, io, ids, now_ms, interest, response, seq) {
            let _ = engine.restore_observe_due(id, seq);
            return Err(e);
        }
        held.add(dest);
        sent += 1;
    }
    Ok(sent)
}

/// Per-endpoint notification NSTART counts for one [`notify_engine`] pass.
///
/// One O(n) occupancy scan instead of `observe_endpoint_held` per observer.
/// Shipped profiles have ≤4 observe entries; the table covers 8 unique
/// endpoints and falls back to a full scan if that overflows (alloc backend).
const NOTIFY_HELD_CACHE: usize = 8;

struct NotifyHeldCache {
    endpoints: [Option<Endpoint>; NOTIFY_HELD_CACHE],
    held: [u8; NOTIFY_HELD_CACHE],
    overflow: bool,
}

impl NotifyHeldCache {
    fn from_engine<S: Storage + ObserveSlots>(engine: &Engine<S>, now_ms: u64) -> Self {
        let mut cache = Self {
            endpoints: [None; NOTIFY_HELD_CACHE],
            held: [0; NOTIFY_HELD_CACHE],
            overflow: false,
        };
        let n = engine.capacities().observe_entries;
        for i in 0..n {
            let Some(row) = engine.observe_interest(SlotId::from_index(i)) else {
                continue;
            };
            if row.is_notify_held(now_ms) {
                cache.add(row.endpoint());
            }
        }
        cache
    }

    fn add(&mut self, endpoint: Endpoint) {
        if let Some(i) = self.index(endpoint) {
            self.held[i] = self.held[i].saturating_add(1);
            return;
        }
        if let Some(i) = self.endpoints.iter().position(Option::is_none) {
            self.endpoints[i] = Some(endpoint);
            self.held[i] = 1;
            return;
        }
        self.overflow = true;
    }

    fn index(&self, endpoint: Endpoint) -> Option<usize> {
        self.endpoints.iter().position(|row| *row == Some(endpoint))
    }

    fn count<S: Storage + ObserveSlots>(
        &self,
        engine: &Engine<S>,
        endpoint: Endpoint,
        now_ms: u64,
    ) -> usize {
        if let Some(i) = self.index(endpoint) {
            return usize::from(self.held[i]);
        }
        if self.overflow {
            return observe_endpoint_held(engine, endpoint, now_ms);
        }
        0
    }
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
    response: &Response<'_>,
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
    let mut notify = *response;
    notify.set_observe(seq);
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
        oscore: oscore::no_request(),
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
        &notify,
        notify.payload(),
        None,
        None,
        &oscore::empty_field(),
        oscore::no_request(),
    ) {
        Ok(()) => finish_send(engine, io, tx, dest, pending),
        Err(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => {
            let _ = engine.release_tx(tx);
            start_notify_block2(engine, io, meta, &notify, ty, pending)
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e.into())
        }
    };
    outcome?;

    let confirmable = ty == Type::Confirmable;
    let _ = engine.record_observe_notify(interest.key(), now_ms, mid, confirmable);
    let max_age = notify.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS);
    let con_mid = confirmable.then_some(mid);
    let _ = engine.refresh_observe_max_age(interest.key(), now_ms, max_age, con_mid);
    Ok(())
}

fn start_notify_block2<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response<'_>,
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
    let outcome = issue_classic(
        engine,
        io,
        meta,
        response,
        ty,
        meta.mid,
        id,
        pending,
        &oscore::empty_field(),
    );
    if outcome.is_err() {
        let _ = engine.release_tx_body(id);
    }
    outcome
}

fn send_separate<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    meta: SendResponse,
    response: &Response<'_>,
    oscore_ctx: &oscore::Field,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let key = BlockKey::new(meta.token, meta.dest);
    if let Some(id) = engine.lookup_tx_body(key) {
        if engine.tx_body_transfer(id).is_some_and(|t| {
            matches!(
                t.role(),
                BlockRole::OutgoingBlock2 | BlockRole::OutgoingQBlock2
            )
        }) {
            let ty = match meta.ty {
                Type::Confirmable => Type::Acknowledgement,
                Type::NonConfirmable => Type::NonConfirmable,
                Type::Acknowledgement | Type::Reset => return Ok(()),
            };
            return continue_outgoing(engine, io, meta, response, ty, id, oscore_ctx);
        }
    }

    let ty = match meta.ty {
        Type::Confirmable => {
            send_empty_ack(engine, io, meta.dest, meta.mid)?;
            remember_empty_ack(engine, meta.dest, meta.mid, now_ms);
            Type::Confirmable
        }
        Type::NonConfirmable => Type::NonConfirmable,
        Type::Acknowledgement | Type::Reset => return Ok(()),
    };

    if meta.no_response.suppresses(response.code()) {
        return Ok(());
    }

    let mid = ids.next();
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid));

    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };

    match encode_response(
        engine,
        tx,
        ty,
        mid,
        meta.token,
        response,
        response.payload(),
        None,
        meta.block1,
        oscore_ctx,
        meta.oscore,
    ) {
        Ok(()) => finish_send(engine, io, tx, meta.dest, pending),
        Err(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => {
            let _ = engine.release_tx(tx);
            let meta = SendResponse { mid, ..meta };
            start_outgoing(engine, io, meta, response, ty, key, oscore_ctx)
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e.into())
        }
    }
}

fn send_response<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response<'_>,
    now_ms: u64,
    oscore_ctx: &oscore::Field,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    if meta.no_response.suppresses(response.code()) {
        return match meta.ty {
            Type::Confirmable => {
                send_empty_ack(engine, io, meta.dest, meta.mid)?;
                remember_empty_ack(engine, meta.dest, meta.mid, now_ms);
                Ok(())
            }
            Type::NonConfirmable | Type::Acknowledgement | Type::Reset => Ok(()),
        };
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
            return continue_outgoing(engine, io, meta, response, ty, id, oscore_ctx);
        }
    }

    let Some(tx) = acquire_tx_or_evict(engine) else {
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
        oscore_ctx,
        meta.oscore,
    ) {
        Ok(()) => match remember_tx_reply(engine, tx, meta.dest, meta.mid, meta.ty, now_ms) {
            KeepTx::Yes => send_pinned_tx(engine, io, tx, meta.dest),
            KeepTx::No => finish_send(engine, io, tx, meta.dest, None),
        },
        Err(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => {
            let _ = engine.release_tx(tx);
            start_outgoing(engine, io, meta, response, ty, key, oscore_ctx)
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
    response: &Response<'_>,
    ty: Type,
    key: BlockKey,
    oscore_ctx: &oscore::Field,
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
        issue_q_window(engine, io, meta, response, ty, id, oscore_ctx)
    } else {
        issue_classic(
            engine, io, meta, response, ty, meta.mid, id, None, oscore_ctx,
        )
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
    response: &Response<'_>,
    ty: Type,
    id: SlotId,
    oscore_ctx: &oscore::Field,
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
                issue_q_window(engine, io, meta, response, ty, id, oscore_ctx)
            }
            Some(_) => Ok(()),
            None => issue_q_window(engine, io, meta, response, ty, id, oscore_ctx),
        },
        BlockRole::OutgoingBlock2 => issue_classic(
            engine, io, meta, response, ty, meta.mid, id, None, oscore_ctx,
        ),
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn issue_classic<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response<'_>,
    ty: Type,
    mid: MessageId,
    id: SlotId,
    pending: Option<(u64, MessageId)>,
    oscore_ctx: &oscore::Field,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots,
    T: DatagramIo,
{
    let issued = engine.next_block2(id).map_err(Error::Block)?;
    send_issued(
        engine, io, meta, response, ty, mid, issued, false, pending, oscore_ctx,
    )?;
    if issued.complete() {
        let _ = engine.release_tx_body(id);
    }
    Ok(())
}

fn issue_q_window<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse,
    response: &Response<'_>,
    first_ty: Type,
    id: SlotId,
    oscore_ctx: &oscore::Field,
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
        send_issued(
            engine, io, meta, response, ty, mid, issued, true, None, oscore_ctx,
        )?;
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
    response: &Response<'_>,
    ty: Type,
    mid: MessageId,
    issued: OutgoingBlock,
    q_block: bool,
    pending: Option<(u64, MessageId)>,
    oscore_ctx: &oscore::Field,
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
        oscore_ctx,
        meta.oscore,
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
    response: &Response<'_>,
    payload: &[u8],
    block: Option<BlockOpt>,
    block1: Option<BlockValue>,
    oscore_ctx: &oscore::Field,
    oscore_req: oscore::Request,
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
    let mut opts = OptionsBuilder::<{ 8 + 2 * LOCATION_MAX }>::new();
    let filled = (|| -> Result<(), EncodeError> {
        if let Some(etag) = response.etag_bytes() {
            push_opt(&mut opts, Opt::etag(etag))?;
        }
        if let Some(ref encoded) = observe {
            push_opt(&mut opts, Opt::observe(encoded))?;
        }
        for segment in response.location_paths() {
            push_opt(&mut opts, Opt::location_path(segment))?;
        }
        if let Some(ref encoded) = cf {
            push_opt(&mut opts, Opt::content_format(encoded))?;
        }
        if let Some(ref encoded) = max_age {
            push_opt(&mut opts, Opt::max_age(encoded))?;
        }
        for query in response.location_queries() {
            push_opt(&mut opts, Opt::location_query(query))?;
        }
        if let Some(ref encoded) = block_enc {
            if block.is_some_and(|b| b.q_block) {
                if let Some(ref size2) = size2_enc {
                    push_opt(&mut opts, Opt::size2(size2))?;
                }
                push_opt(&mut opts, Opt::q_block2(encoded))?;
            } else {
                push_opt(&mut opts, Opt::block2(encoded))?;
            }
        }
        if let Some(ref encoded) = block1_enc {
            push_opt(&mut opts, Opt::block1(encoded))?;
        }
        if let Some(ref echo) = echo {
            push_opt(&mut opts, Opt::echo(echo.as_slice()))?;
        }
        Ok(())
    })();
    match filled {
        Ok(()) => {
            let msg = Message::new(ty, response.code(), mid)
                .with_token(token)
                .with_options(opts.as_slice())
                .with_payload(payload);
            oscore::encode_message(oscore_ctx, oscore_req, engine, tx, &msg)
        }
        Err(EncodeError::OptionsFull) => encode_options_full_500(engine, tx, ty, mid, token),
        Err(e) => Err(SlotMessageError::Encode(e)),
    }
}

pub(crate) fn push_opt<'a, const N: usize>(
    opts: &mut OptionsBuilder<'a, N>,
    opt: Opt<'a>,
) -> Result<(), EncodeError> {
    opts.push(opt)
        .map(|_| ())
        .map_err(|_| EncodeError::OptionsFull)
}

fn encode_options_full_500<S: Storage + DatagramSlots>(
    engine: &mut Engine<S>,
    tx: SlotId,
    ty: Type,
    mid: MessageId,
    token: crate::message::Token,
) -> Result<(), SlotMessageError> {
    let response = Response::problem(Code::INTERNAL_SERVER_ERROR).title("Options full");
    let cf = response.format().map(crate::ContentFormat::encode);
    let mut opts = OptionsBuilder::<4>::new();
    if let Some(ref encoded) = cf {
        push_opt(&mut opts, Opt::content_format(encoded)).map_err(SlotMessageError::Encode)?;
    }
    let msg = Message::new(ty, response.code(), mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(response.payload());
    engine.encode_tx(tx, &msg).map(|_| ())
}

enum Replay<E> {
    Hit(Result<(), Error<E>>),
    Miss,
}

enum KeepTx {
    Yes,
    No,
}

/// GET / PUT / DELETE / iPATCH may re-run when a live Dedup row has no
/// replay (RFC 7252 §4.5 MAY). POST / PATCH / FETCH never re-run.
fn request_may_rerun_on_duplicate(code: Code) -> bool {
    matches!(code, Code::GET | Code::PUT | Code::DELETE | Code::IPATCH)
}

fn replay_con_request<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    peer: Endpoint,
    parsed: &ParsedMessage<'_>,
    now_ms: u64,
) -> Replay<T::Error>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    let Some(id) = engine.lookup_dedup(DedupKey::new(parsed.message_id(), peer)) else {
        return Replay::Miss;
    };
    let Some(entry) = engine.dedup_entry(id) else {
        return Replay::Miss;
    };
    if entry.due_ms() != 0 && now_ms >= entry.due_ms() {
        release_dedup_row(engine, entry);
        return Replay::Miss;
    }
    if let Some(bytes) = entry.replay() {
        return Replay::Hit(send_replay_bytes(engine, io, peer, bytes));
    }
    if let Some(tx) = entry.tx_pin() {
        return Replay::Hit(engine.send_tx(io, tx).map(|_| ()).map_err(Into::into));
    }
    if request_may_rerun_on_duplicate(parsed.code()) {
        Replay::Miss
    } else {
        // Never silent: ACK the CON so the client stops RTO. Persist the
        // empty ACK so a later Miss-after-lifetime is the only re-run path.
        remember_empty_ack(engine, peer, parsed.message_id(), now_ms);
        Replay::Hit(send_empty_ack(engine, io, peer, parsed.message_id()))
    }
}

fn send_replay_bytes<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    bytes: &[u8],
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    let Some(tx) = acquire_tx_or_evict(engine) else {
        return Err(Error::Saturated);
    };
    if let Err(e) = engine.write_tx(tx, bytes, dest) {
        let _ = engine.release_tx(tx);
        return Err(Error::Slot(e));
    }
    let send = engine.send_tx(io, tx);
    let _ = engine.release_tx(tx);
    send?;
    Ok(())
}

fn send_pinned_tx<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    tx: SlotId,
    dest: Endpoint,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots,
    T: DatagramIo,
{
    if let Err(e) = engine.set_tx_endpoint(tx, dest) {
        let _ = engine.release_tx(tx);
        return Err(Error::Slot(e));
    }
    engine.send_tx(io, tx)?;
    Ok(())
}

fn remember_tx_reply<S: Storage + DatagramSlots + DedupSlots>(
    engine: &mut Engine<S>,
    tx: SlotId,
    dest: Endpoint,
    mid: MessageId,
    request_ty: Type,
    now_ms: u64,
) -> KeepTx {
    if request_ty != Type::Confirmable {
        return KeepTx::No;
    }
    let due = dedup_due_ms(now_ms);
    let mut entry = DedupEntry::new(mid, dest).with_due_ms(due);
    let copied = {
        let Ok(access) = engine.access_tx(tx) else {
            remember_empty_ack(engine, dest, mid, now_ms);
            return KeepTx::No;
        };
        let slice = access.as_bytes();
        if slice.len() > DATAGRAM_SCRATCH {
            None
        } else {
            let mut buf = [0u8; DATAGRAM_SCRATCH];
            buf[..slice.len()].copy_from_slice(slice);
            Some((buf, slice.len()))
        }
    };
    let Some((buf, n)) = copied else {
        return if store_request_dedup(engine, entry.with_tx_pin(tx)) {
            KeepTx::Yes
        } else {
            KeepTx::No
        };
    };
    if n <= DedupEntry::REPLAY_MAX {
        entry = entry.with_replay(&buf[..n]);
        let _ = store_request_dedup(engine, entry);
        KeepTx::No
    } else if store_request_dedup(engine, entry.with_tx_pin(tx)) {
        KeepTx::Yes
    } else {
        KeepTx::No
    }
}

fn remember_empty_ack<S: Storage + DedupSlots>(
    engine: &mut Engine<S>,
    dest: Endpoint,
    mid: MessageId,
    now_ms: u64,
) {
    let mut buf = [0u8; 16];
    let Ok(n) = encode(&Message::empty_ack(mid), &mut buf) else {
        return;
    };
    let _ = store_request_dedup(
        engine,
        DedupEntry::new(mid, dest)
            .with_due_ms(dedup_due_ms(now_ms))
            .with_replay(&buf[..n]),
    );
}

fn store_request_dedup<S: Storage + DedupSlots>(engine: &mut Engine<S>, entry: DedupEntry) -> bool {
    if !entry.has_usable_replay() {
        return false;
    }
    release_existing_dedup(engine, entry.key());
    if engine.insert_dedup(entry).is_some() {
        return true;
    }
    evict_oldest_app_dedup(engine) && engine.insert_dedup(entry).is_some()
}

fn release_existing_dedup<S: Storage + DedupSlots>(engine: &mut Engine<S>, key: DedupKey) {
    let Some(id) = engine.lookup_dedup(key) else {
        return;
    };
    if let Some(old) = engine.dedup_entry(id) {
        release_dedup_row(engine, old);
    } else {
        let _ = engine.remove_dedup(key);
    }
}

fn release_dedup_row<S: Storage + DedupSlots>(engine: &mut Engine<S>, entry: DedupEntry) {
    if let Some(pin) = entry.tx_pin() {
        let _ = engine.release_tx(pin);
    }
    let _ = engine.remove_dedup(entry.key());
}

fn dedup_due_ms(now_ms: u64) -> u64 {
    now_ms.saturating_add(u64::from(Transmission::EXCHANGE_LIFETIME_MS))
}

fn expire_request_dedup<S: Storage + DedupSlots>(engine: &mut Engine<S>, now_ms: u64) {
    let n = engine.capacities().dedup_entries;
    for i in 0..n {
        let Some(entry) = engine.dedup_entry(SlotId::from_index(i)) else {
            continue;
        };
        if entry.due_ms() != 0 && now_ms >= entry.due_ms() {
            release_dedup_row(engine, entry);
        }
    }
}

fn acquire_tx_or_evict<S: Storage + DatagramSlots + DedupSlots>(
    engine: &mut Engine<S>,
) -> Option<SlotId> {
    if let Some(id) = engine.acquire_tx() {
        return Some(id);
    }
    if evict_one_dedup_pin(engine) {
        return engine.acquire_tx();
    }
    None
}

fn evict_one_dedup_pin<S: Storage + DedupSlots>(engine: &mut Engine<S>) -> bool {
    let n = engine.capacities().dedup_entries;
    let mut best: Option<DedupEntry> = None;
    for i in 0..n {
        let Some(entry) = engine.dedup_entry(SlotId::from_index(i)) else {
            continue;
        };
        if entry.tx_pin().is_none() || entry.due_ms() == 0 {
            continue;
        }
        match best {
            Some(cur) if entry.due_ms() >= cur.due_ms() => {}
            _ => best = Some(entry),
        }
    }
    let Some(entry) = best else {
        return false;
    };
    if let Some(pin) = entry.tx_pin() {
        let _ = engine.release_tx(pin);
    }
    let _ = engine.remove_dedup(entry.key());
    let mut buf = [0u8; 16];
    if let Ok(n) = encode(&Message::empty_ack(entry.message_id()), &mut buf) {
        let row = DedupEntry::new(entry.message_id(), entry.endpoint())
            .with_due_ms(entry.due_ms())
            .with_replay(&buf[..n]);
        let _ = engine.insert_dedup(row);
    }
    true
}

fn evict_oldest_app_dedup<S: Storage + DedupSlots>(engine: &mut Engine<S>) -> bool {
    let n = engine.capacities().dedup_entries;
    let mut best: Option<DedupEntry> = None;
    for i in 0..n {
        let Some(entry) = engine.dedup_entry(SlotId::from_index(i)) else {
            continue;
        };
        if entry.due_ms() == 0 {
            continue;
        }
        match best {
            Some(cur) if entry.due_ms() >= cur.due_ms() => {}
            _ => best = Some(entry),
        }
    }
    let Some(entry) = best else {
        return false;
    };
    release_dedup_row(engine, entry);
    true
}

/// Send occupied TX `tx` to `dest`.
///
/// When `pending` is `Some`, admit the CON into the pending table **before**
/// `send_tx` so a second CON to the same peer cannot occupy TX without RTO
/// ([`Transmission::NSTART`](crate::message::Transmission::NSTART)). `None`
/// from [`Engine::record_pending_con`](Engine::record_pending_con) releases
/// the slot and returns [`Error::Saturated`]. An IO failure after admit
/// takes the pending mark and releases TX.
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
    if let Some((now_ms, mid)) = pending {
        if engine
            .record_pending_con(tx, dest, mid, now_ms, 0)
            .is_none()
        {
            let _ = engine.release_tx(tx);
            return Err(Error::Saturated);
        }
        match engine.send_tx(io, tx) {
            Ok(_) => Ok(()),
            Err(e) => {
                let _ = engine.take_pending_con(mid, dest);
                let _ = engine.release_tx(tx);
                Err(e.into())
            }
        }
    } else {
        let send = engine.send_tx(io, tx);
        let _ = engine.release_tx(tx);
        send?;
        Ok(())
    }
}

fn oscore_inbound_error<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    parsed: &crate::message::ParsedMessage<'_>,
    peer: Endpoint,
    now_ms: u64,
    err: oscore::InboundError,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    #[cfg(feature = "oscore")]
    {
        // AEAD / replay / OSCORE-option processing: silent drop (no 4.00
        // decrypt oracle). A response is never answered. Only a *plain*
        // request (no OSCORE option) while a context is attached is 4.01.
        if !parsed.code().is_request() || parsed.oscore().is_some() {
            return Ok(());
        }
        let _ = err;
        let meta = SendResponse {
            dest: peer,
            ty: parsed.ty(),
            mid: parsed.message_id(),
            token: parsed.token(),
            no_response: NoResponse::DEFAULT,
            block2: None,
            q_block2: None,
            block1: None,
            oscore: oscore::no_request(),
        };
        send_response(
            engine,
            io,
            meta,
            &Response::problem(Code::UNAUTHORIZED).title("Unauthorized"),
            now_ms,
            &oscore::empty_field(),
        )
    }
    #[cfg(not(feature = "oscore"))]
    {
        let _ = (engine, io, parsed, peer, now_ms, err);
        Ok(())
    }
}

fn unauthorized_echo(now_ms: u64) -> Response<'static> {
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
    /// Request Partial IV when the inbound request was OSCORE-protected.
    oscore: oscore::Request,
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
    send_empty(engine, io, dest, Message::empty_ack(mid))
}

fn send_empty_rst<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    mid: crate::message::MessageId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots,
    T: DatagramIo,
{
    send_empty(engine, io, dest, Message::empty_rst(mid))
}

fn send_empty<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    msg: Message<'static>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots,
    T: DatagramIo,
{
    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
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
    /// TX pool, client inbox, or outstanding Call table is full.
    Saturated,
    /// Block / Q-Block body start, issue, or recover encode failed.
    Block(BlockTransferError),
    /// Uri-Path has more than [`MAX_PATH_SEGMENTS`] segments.
    Path,
    /// OSCORE protect / unprotect failed (feature `oscore`).
    #[cfg(feature = "oscore")]
    Oscore(crate::oscore::Error),
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
            Self::Saturated => f.write_str("a bounded table is saturated"),
            Self::Block(e) => write!(f, "{e}"),
            Self::Path => f.write_str("uri-path has too many segments"),
            #[cfg(feature = "oscore")]
            Self::Oscore(e) => write!(f, "{e}"),
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
            #[cfg(feature = "oscore")]
            Self::Oscore(e) => Some(e),
        }
    }
}
