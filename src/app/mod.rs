//! Route inbound CoAP requests and drive outbound exchanges.
//!
//! ```text
//! RX slot (+ body) --view--> Request
//! handler(Request) -> Response
//! Response --encode--> TX slot (+ body)
//!
//! Outgoing (get/put) --encode--> TX slot
//! poll matches Token + endpoint
//! RX --copy--> Response::payload  (non-Block: min(len, INLINE_PAYLOAD) = 128)
//! Block2 / Q-Block2 --apply--> RX body --copy--> Response::body()
//! ```
//!
//! [`Request`] is a borrowed view for the handler call only. [`Response`] is
//! owned intent — the same type [`App::take_response`] yields for a
//! completed [`Call`]. Non-Block client payloads copy at most
//! [`INLINE_PAYLOAD`] (128) bytes; check
//! [`Response::payload_truncated`]. Assembled Block2 is
//! [`Response::body`]. The reactor owns per-slot state machines inside
//! [`App::poll`]. Applications own domain data that outlives a request.
//!
//! Bind requires a provisioned pairwise OSCORE context. The example's hidden
//! credentials are test fixtures; supply authenticated credentials and persist
//! sender-sequence reservations in production. Select unprotected traffic
//! explicitly with [`AppBuilder::allow_unprotected`].
//!
//! ```
//! # #[cfg(feature = "oscore")]
//! # {
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
//! # impl coaptic::storage::DatagramTransport for NullIo {
//! #     type Error = &'static str;
//! # }
//! # impl DatagramIo for NullIo {
//! #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
//! #         Ok(None)
//! #     }
//! #     fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> { Ok(bytes.len()) }
//! # }
//! # let provisioned_credentials = coaptic::oscore::DeriveParams {
//! #     master_secret: &[0x42; 16], master_salt: &[], sender_id: &[1],
//! #     recipient_id: &[2], id_context: &[],
//! # };
//! let context = coaptic::oscore::SecurityContext::derive(provisioned_credentials).unwrap();
//! let mut app = App::profile::<profiles::Default>()
//!     .randomness(|bytes| getrandom::fill(bytes).is_ok())
//!     .block_wise::<true>()
//!     .route("sensors/temp", get(get_temp))
//!     .route("leds/0", get(get_led).put(put_led))
//!     .well_known_core()
//!     .oscore(context)
//!     .bind(NullIo)
//!     .unwrap();
//! app.poll(0).unwrap();
//!
//! let peer = Endpoint::v4([192, 0, 2, 2], 5683);
//! let call = app.get("sensors/temp").to(peer).send(0).unwrap();
//! app.poll(0).unwrap();
//! let _ = app.take_response(call);
//! # }
//! ```
//!
//! [`App::builder`] starts with fixed default datagram storage. Adjust the
//! same builder with [`AppBuilder::block_wise`], [`AppBuilder::bind_storage`]
//! or `AppBuilder::bind_alloc` (`alloc` independently of `std`). No separate
//! advanced reactor is needed for custom storage. [`AppBuilder::full_responses`]
//! and [`App::take_response_into`] collect exact bytes into caller storage;
//! insufficient capacity leaves the complete reply available for retry.
//!
//! [`App::poll_with`] admits caller-owned asynchronous/durable work without
//! blocking the reactor. Return [`Response::deferred`] with a supplied
//! [`DeferredReply`], then [`App::complete`] with [`Response::payload_borrowed`].
//! A deferred Block2 reply retains one bounded snapshot and serves matching
//! follow-ups without dispatching the durable operation again.
//!
//! Site capacity defaults to [`DEFAULT_ROUTES`] (8); raise it with
//! [`.routes::<16>()`](AppBuilder::routes) before bind. Paths accept
//! `"sensors/temp"` or `&["sensors", "temp"]` ([`IntoPath`]).
//!
//! Observe: return [`.observe`](Response::observe) on a successful queryless GET, or
//! attach [`MethodRouter::observe`]; later representations are
//! [`App::notify`]. Server re-registration with the same peer, Token and route
//! advances the retained 24-bit sequence while replacing scheduling and OSCORE
//! request state. After that relation is removed, or when a new App is
//! constructed, the next registration starts at sequence 0. Notifications are
//! selected by exact path segments. [`MethodRouter::observe_selected`] retains
//! each GET/FETCH selection for notification callbacks. Requestless sources
//! decline FETCH and query-bearing GET registration with an ordinary response.
//! Client subscribe/refresh uses [`Outgoing::observe`] /
//! [`Outgoing::reregister_call`]; cancellation uses
//! [`Outgoing::deregister`] on the same [`Call`]. Echo verification
//! (RFC 9175 4.01) is caller-owned through [`AppBuilder::echo_policy`]. Pairwise OSCORE
//! (default feature `oscore`) is `AppBuilder::oscore` before bind.
//!
//! The happy path does not use [`Access`](crate::storage::Access) or
//! [`SlotId`]. Engine remains the advanced escape hatch
//! ([`App::engine_mut`]) for explicit slots, custom RST / remaining 4.xx,
//! and codecs not exposed by the App transfer API, such as BERT.
mod admission;
mod associations;
mod builder;
mod client;
pub mod diagnostics;
mod echo;
mod identity;
mod oscore;
mod request;
pub mod resources;
mod response;
mod routing;
mod send;
use async_server::{complete_deferred_observe_async, dispatch_rx_async, send_notification_async};
use send::finish_protocol as finish_send_async;

pub(crate) fn completion_time<T: crate::storage::DatagramTransport>(_io: &T, now_ms: u64) -> u64 {
    now_ms
}

fn commit_q_window_send<S: Storage + BodySlots, E>(
    engine: &mut Engine<S>,
    issued: OutgoingBlock,
) -> Result<(), Error<E>> {
    engine
        .commit_q_block2(issued)
        .map_err(|_| Error::CommitAfterSend)
}

fn send_may_have_reached_peer<T: crate::storage::DatagramTransport>(
    _io: &T,
    error: &Error<T::Error>,
) -> bool {
    matches!(
        error,
        Error::Io(DatagramIoError::Io(_) | DatagramIoError::SendLength { .. })
    )
}

fn send_retransmit_packet<S: AppStorage, T: DatagramIo, const C: usize, const R: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    _inbox: &mut client::ClientInbox<C, S::Token>,
    _lives: &mut client::ClientLives<C, S::Token>,
    _oscore: &mut oscore::Field<R, S::Token>,
    tx: SlotId,
) -> Result<(), Error<T::Error>> {
    engine.send_tx(io, tx)?;
    Ok(())
}
pub(crate) mod async_server;
pub use async_server::AsyncIoError;
mod site;
mod usage;
mod wake;

pub use usage::AppUsage;
pub use wake::AppWake;

#[cfg(test)]
mod lifecycle_soak;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod capacity_tests;

#[cfg(test)]
mod reservation_tests;

#[cfg(all(test, feature = "oscore"))]
mod oscore_install_tests;

use core::marker::PhantomData;

use crate::error::{BlockTransferError, BuildError, EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, Echo, EncodedUint, Message, MessageId, NoResponse, Opt,
    OptionsBuilder, OwnedToken, ParsedMessage, Transmission, Type, encode, encode_uint,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, DatagramIo, DatagramIoError, DatagramSlots, DedupEntry,
    DedupKey, DedupSlots, Endpoint, Engine, EngineBuilder, Exchanges, MemoryLayout, MemoryProfile,
    Metrics, Missing, ObserveInterest, ObserveKey, ObserveResource, ObserveSlots, OutgoingBlock,
    PendingCons, Present, QBlockRecover, Retransmit, SlotError, SlotId, Storage,
};

/// RFC 7252 default Max-Age when a registration or notify omits it.
pub(crate) const DEFAULT_MAX_AGE_SECS: u32 = 60;

/// Default number of live client Calls and retained completions per App.
pub const DEFAULT_CLIENT_CALLS: usize = 4;

pub use associations::Association;
pub use client::{
    AsyncCallError, Call, CallFailure, OBSERVE_REQUEST_BYTES, Outgoing, RESPONSE_OPTION_BYTES,
    RESPONSE_OPTION_COUNT, ResponseBufferError, TokenProbe, TokenProbeCall, TokenSupport,
    TokenSupportLifetimeError, TokenSupportOutcome,
};
pub use echo::{EchoCheck, EchoDecision, EchoPolicy};
use identity::{AppIds, Source as IdentitySource};
pub use identity::{IdentityError, RandomSource};
pub use request::{IntoPath, MAX_PATH_SEGMENTS, PathError, Request, split_path};
pub use response::{
    AppAssembled, INLINE_PAYLOAD, IntoResponse, LOCATION_MAX, RESPONSE_BODY, Response,
    ResponseError,
};
pub use routing::{
    HandlerFn, Method, MethodRouter, ObserveSource, delete, fetch, get, ipatch, patch, post, put,
};
pub use site::{
    DEFAULT_ROUTES, LINK_FORMAT_PER_ROUTE, LinkFormatScratch, RouteError, Site,
    link_format_capacity,
};

/// Refusal to remove a route. No routes or protocol state are changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteRemovalError {
    /// The supplied path is invalid.
    Path(PathError),
    /// App-owned protocol work must drain before a route slot can be reused.
    Busy,
}

use response::AssembledField;

/// Refusal to install a staged OSCORE context. No key material is retained here.
#[cfg(feature = "oscore")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OscoreInstallError {
    /// The caller did not stage a replacement context.
    MissingCandidate,
    /// No free association slot; existing owners are never evicted.
    Capacity,
    /// Recipient routing or owner/epoch identity would be ambiguous.
    Ambiguous,
    /// Local routing generations cannot be reused. Recreate the drained App.
    GenerationExhausted,
    /// Handle is stale or was not issued by this App.
    UnknownAssociation,
    /// App-owned protocol state has not drained under the current context.
    Busy,
}

#[cfg(feature = "oscore")]
impl core::fmt::Display for OscoreInstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::UnknownAssociation => "unknown security association",
            Self::Capacity => "association capacity exhausted",
            Self::Ambiguous => "ambiguous association recipient identity",
            Self::GenerationExhausted => "association generations exhausted",
            Self::MissingCandidate => "no OSCORE context was staged",
            Self::Busy => "OSCORE context replacement requires drained protocol state",
        })
    }
}

#[cfg(all(feature = "oscore", feature = "std"))]
impl std::error::Error for OscoreInstallError {}

/// Storage capabilities used by the single [`App`] interface.
///
/// Fixed [`crate::storage::Memory`] and allocator-backed `AllocMemory` (with
/// `alloc`) use the same routing, request builders and protocol reactor.
pub trait AppStorage:
    Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges + DedupSlots
{
}
impl<S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges + DedupSlots>
    AppStorage for S
{
}

/// Opaque completion authority for work admitted by [`App::poll_with`].
///
/// Retain only with the originating App. Handles include a fresh random nonce
/// and monotonic generation; a removed/reused row never accepts an old handle.
/// This is local scheduling identity, not application durable idempotency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeferredReply {
    nonce: crate::message::Token,
    generation: u64,
}

type AppIngress<'a> = dyn FnMut(&[u8], Endpoint) -> bool + 'a;

type AppDispatch<'a, K> =
    dyn FnMut(Request<'_, K>, Option<DeferredReply>) -> Response<'static, K> + 'a;

#[derive(Clone, Copy)]
struct DeferredRow<K: OwnedToken = crate::message::Token> {
    handle: DeferredReply,
    meta: SendResponse<'static, K>,
    deadline_ms: u64,
    selection: DeferredSelection,
    q_selectors: DeferredQSelectors,
    snapshot: Option<(SlotId, client::ReplyMeta)>,
    completed: bool,
    observe: Option<DeferredObserve>,
}

#[derive(Clone, Copy)]
struct DeferredObserve {
    resource: ObserveResource,
    // Sequence and format of this handle's transport-ambiguous attempt.
    attempted: Option<(u32, Option<ContentFormat>)>,
    // Only the latest send that reached the transport grants empty-RST authority.
    failed_mid: Option<(MessageId, u64)>,
}
// Owned wire values only; the representation key retains its separate 512-byte bound.
#[derive(Clone, Copy)]
struct DeferredQSelectors {
    values: [u32; crate::storage::BlockTransfer::MAX_PAYLOADS as usize],
    len: usize,
}
impl DeferredQSelectors {
    fn from_request(parsed: &ParsedMessage<'_, impl OwnedToken>) -> Option<Self> {
        let mut result = Self {
            values: [0; crate::storage::BlockTransfer::MAX_PAYLOADS as usize],
            len: 0,
        };
        for value in parsed.q_block2() {
            let value = value.ok()?;
            if result.len == result.values.len() {
                return None;
            }
            result.values[result.len] = value.as_uint();
            result.len += 1;
        }
        Some(result)
    }
    fn iter(&self) -> impl Iterator<Item = BlockValue> + '_ {
        self.values[..self.len]
            .iter()
            .map(|&value| BlockValue::from_uint(value).expect("validated selector"))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct DeferredSelection {
    bytes: [u8; client::RESPONSE_OPTION_BYTES],
    len: usize,
}
impl DeferredSelection {
    #[cfg(test)]
    fn from_request(parsed: &ParsedMessage<'_, impl OwnedToken>) -> Result<Self, EncodeError> {
        Self::from_parts(parsed, parsed.payload())
    }
    fn stored(self) -> crate::storage::ObserveSelection {
        crate::storage::ObserveSelection {
            bytes: self.bytes,
            len: self.len as u16,
        }
    }
    fn from_parts(
        parsed: &ParsedMessage<'_, impl OwnedToken>,
        payload: &[u8],
    ) -> Result<Self, EncodeError> {
        let mut options = OptionsBuilder::<{ client::RESPONSE_OPTION_COUNT }>::new();
        for option in parsed.options().filter(|option| {
            (parsed.code() == Code::FETCH && option.number() == crate::message::OptionNumber::ETAG)
                || matches!(
                    option.number(),
                    crate::message::OptionNumber::IF_MATCH
                        | crate::message::OptionNumber::IF_NONE_MATCH
                        | crate::message::OptionNumber::URI_PATH
                        | crate::message::OptionNumber::URI_QUERY
                        | crate::message::OptionNumber::CONTENT_FORMAT
                        | crate::message::OptionNumber::ACCEPT
                )
        }) {
            push_opt(&mut options, option)?;
        }
        let message = Message::new(Type::NonConfirmable, parsed.code(), MessageId::new(0))
            .with_token(parsed.token())
            .with_options(options.as_slice())
            .with_payload(if parsed.code() == Code::FETCH {
                payload
            } else {
                &[]
            });
        let mut selection = Self {
            bytes: [0; client::RESPONSE_OPTION_BYTES],
            len: 0,
        };
        selection.len = encode(&message, &mut selection.bytes)?;
        Ok(selection)
    }
}

fn deferred_reply_metadata<K: OwnedToken>(
    response: &Response<'_, K>,
    meta: SendResponse<'_, K>,
    now_ms: u64,
) -> Result<client::ReplyMeta, ResponseError> {
    let mut options = OptionsBuilder::<{ client::RESPONSE_OPTION_COUNT }>::new();
    let cf = response.format().map(ContentFormat::encode);
    let age = response.max_age_secs().map(encode_uint);
    let echo = response.echo_option();
    let result = (|| -> Result<(), EncodeError> {
        if let Some(tag) = response.etag_bytes() {
            push_opt(&mut options, Opt::etag(tag))?;
        }
        for location in response.location_paths() {
            push_opt(&mut options, Opt::location_path(location))?;
        }
        if let Some(cf) = cf.as_ref() {
            push_opt(&mut options, Opt::content_format(cf))?;
        }
        if let Some(age) = age.as_ref() {
            push_opt(&mut options, Opt::max_age(age))?;
        }
        for query in response.location_queries() {
            push_opt(&mut options, Opt::location_query(query))?;
        }
        if let Some(echo) = echo.as_ref() {
            push_opt(&mut options, Opt::echo(echo.as_slice()))?;
        }
        Ok(())
    })();
    result.map_err(|_| ResponseError::DeferredMetadataBounds)?;
    let message = Message::new(Type::NonConfirmable, response.code(), meta.mid)
        .with_token(meta.token)
        .with_options(options.as_slice());
    let mut bytes = [0; client::RESPONSE_OPTION_BYTES];
    let n = encode(&message, &mut bytes).map_err(|_| ResponseError::DeferredMetadataBounds)?;
    let parsed = crate::message::decode_with_token::<K>(&bytes[..n])
        .map_err(|_| ResponseError::DeferredMetadataBounds)?;
    client::ReplyMeta::from_parsed(&parsed, meta.dest, now_ms)
        .map_err(|_| ResponseError::DeferredMetadataBounds)
}

struct DeferredTable<const DEFERRED: usize, K: OwnedToken = crate::message::Token> {
    rows: [Option<DeferredRow<K>>; DEFERRED],
    generation: u64,
}
impl<const DEFERRED: usize, K: OwnedToken> DeferredTable<DEFERRED, K> {
    const fn new() -> Self {
        Self {
            rows: [None; DEFERRED],
            generation: 0,
        }
    }
    fn expire<S: AppStorage<Token = K>>(&mut self, engine: &mut Engine<S>, now_ms: u64) {
        for row in &mut self.rows {
            let Some(value) = *row else {
                continue;
            };
            let body = value.snapshot.map(|(body, _)| body);
            let owns_observe = value
                .observe
                .is_none_or(|observe| observe.attempted.is_none())
                || deferred_observe_owner(engine, value).is_some();
            let body_live = owns_observe
                && body.is_some_and(|body| {
                    engine.tx_body_transfer(body).is_some_and(|transfer| {
                        transfer.key().token() == value.meta.token
                            && transfer.key().endpoint() == value.meta.dest
                    })
                });
            if now_ms >= value.deadline_ms || (value.completed && !body_live) {
                if value.completed
                    && body_live
                    && engine.tx_body_is_pinned(body.expect("live body"))
                {
                    continue;
                }
                if !value.completed && retire_deferred_observe(engine, value).is_err() {
                    continue;
                }
                if body_live {
                    let _ = engine.release_tx_body(body.expect("live body"));
                }
                *row = None;
            }
        }
    }
    fn blocks_observe(&self, key: ObserveKey<K>) -> bool {
        self.rows.iter().flatten().any(|row| {
            row.observe.is_some() && ObserveKey::new(row.meta.token, row.meta.dest) == key
        })
    }
    fn supersede_observe<S: AppStorage<Token = K>>(
        &mut self,
        engine: &mut Engine<S>,
        key: ObserveKey<K>,
    ) -> Result<(), crate::storage::SlotError> {
        for row in &mut self.rows {
            if let Some(value) = *row
                && value.observe.is_some()
                && ObserveKey::new(value.meta.token, value.meta.dest) == key
            {
                retire_deferred_observe(engine, value)?;
                *row = None;
            }
        }
        Ok(())
    }
    fn reject_observe_attempt<S: AppStorage<Token = K>>(
        &mut self,
        engine: &mut Engine<S>,
        mid: MessageId,
        endpoint: Endpoint,
        now_ms: u64,
    ) -> Result<(), SlotError> {
        for row in &mut self.rows {
            if let Some(value) = *row
                && value.meta.dest == endpoint
                && value.observe.is_some_and(|observe| {
                    observe
                        .failed_mid
                        .is_some_and(|(sent, expires)| sent == mid && now_ms < expires)
                })
                && deferred_observe_owner(engine, value).is_some()
            {
                retire_deferred_observe(engine, value)?;
                *row = None;
            }
        }
        Ok(())
    }
    fn candidate<T: crate::storage::DatagramTransport>(
        &mut self,
        ids: &mut AppIds,
        io: &mut T,
    ) -> Result<Option<DeferredReply>, IdentityError> {
        if self.rows.iter().all(Option::is_some) {
            return Ok(None);
        }
        let Some(generation) = self.generation.checked_add(1) else {
            return Ok(None);
        };
        let nonce = ids.nonce(&mut |bytes| io.fill_random(bytes))?;
        self.generation = generation;
        Ok(Some(DeferredReply { nonce, generation }))
    }
    fn insert(
        &mut self,
        handle: DeferredReply,
        meta: SendResponse<'static, K>,
        deadline_ms: u64,
        selection: DeferredSelection,
        q_selectors: DeferredQSelectors,
        observe: Option<DeferredObserve>,
    ) {
        let row = self
            .rows
            .iter_mut()
            .find(|row| row.is_none())
            .expect("reserved capacity");
        *row = Some(DeferredRow {
            handle,
            meta,
            deadline_ms,
            selection,
            q_selectors,
            snapshot: None,
            completed: false,
            observe,
        });
    }
}

// Deferred authority is matched to its attempted relation, not a reused slot.
// Raw Engine replacement remains the advanced caller's ownership boundary.
fn deferred_observe_owner<S: AppStorage>(
    engine: &Engine<S>,
    row: DeferredRow<S::Token>,
) -> Option<ObserveInterest<S::Token>> {
    let observe = row.observe?;
    let (seq, format) = observe.attempted?;
    let interest = engine
        .lookup_observe(ObserveKey::new(row.meta.token, row.meta.dest))
        .and_then(|id| engine.observe_interest(id))?;
    if interest.resource() != observe.resource
        || interest.seq() != seq
        || interest.content_format() != format
    {
        return None;
    }
    #[cfg(feature = "oscore")]
    if oscore::request_from_interest(interest) != row.meta.oscore {
        return None;
    }
    Some(interest)
}
fn retire_deferred_observe<S: AppStorage>(
    engine: &mut Engine<S>,
    row: DeferredRow<S::Token>,
) -> Result<(), crate::storage::SlotError> {
    let Some(interest) = deferred_observe_owner(engine, row) else {
        // A prepared large response can fail before a relation is installed,
        // including the ordinary-response fallback when the Observe table is full.
        // This handle still owns its staged body, independently of any live observer.
        if row
            .observe
            .is_some_and(|observe| observe.attempted.is_none())
            && let Some((body, _)) = row.snapshot
        {
            engine.release_tx_body(body)?;
        }
        return Ok(());
    };
    if let Some(body) =
        response_body_for(engine, BlockKey::new(interest.token(), interest.endpoint()))
        && engine.tx_body_is_pinned(body)
    {
        return Err(crate::storage::SlotError::Pinned);
    }
    if let Some(mid) = interest.notify_hold().and_then(|hold| hold.con_mid())
        && let Some(tx) = engine.lookup_owned_con(mid, interest.endpoint())
    {
        engine.release_tx(tx)?;
    }
    engine.take_observe(interest.key());
    release_observe_response_body(engine, interest);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(complete_deferred_observe_io_async, bound(crate::app::async_server::ProtocolIo), calls(complete_deferred_q_observe => complete_deferred_q_observe_async, completion_time => async_server::completion_time_async, send_may_have_reached_peer => async_server::send_may_have_reached_peer_async, finish_send => finish_send_async))]
fn complete_deferred_observe<S: AppStorage, T: DatagramIo, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    row: &mut DeferredRow<S::Token>,
    response: &Response<'_, S::Token>,
) -> Result<(), Error<T::Error>> {
    if row.meta.no_response.suppresses(response.code()) {
        retire_deferred_observe(engine, *row)?;
        row.completed = true;
        return Ok(());
    }
    let observe = row.observe.expect("Observe completion");
    let attempted_format = if observe.attempted.is_some() {
        let owner = deferred_observe_owner(engine, *row).ok_or(Error::DeferredUnknown)?;
        if response.code().is_success()
            && !owner.content_format_matches(response.code(), response.format())
        {
            return Err(Error::Response(ResponseError::DeferredObserveFormat));
        }
        (!owner.format_unbound()).then_some(owner.content_format())
    } else {
        None
    };
    if row.meta.q_block2.is_some() && response.code().is_success() && response.code() != Code::VALID
    {
        return complete_deferred_q_observe(engine, io, ids, oscore, now_ms, row, response);
    }
    if observe_endpoint_held(engine, row.meta.dest, now_ms) >= usize::from(Transmission::NSTART) {
        return Err(Error::Saturated);
    }
    let key = ObserveKey::new(row.meta.token, row.meta.dest);
    let previous_id = engine.lookup_observe(key);
    let previous = previous_id.and_then(|id| engine.observe_interest(id));
    let body = response_body_for(engine, BlockKey::new(row.meta.token, row.meta.dest));
    if body.is_some_and(|id| engine.tx_body_is_pinned(id)) {
        return Err(Error::Slot(crate::storage::SlotError::Pinned));
    }
    let room = previous_id.is_some()
        || (0..engine.capacities().observe_entries)
            .any(|index| engine.observe_interest(SlotId::from_index(index)).is_none());
    let register = response.code().is_success() && room;
    let sequence = previous
        .filter(|old| old.resource() == observe.resource)
        .map_or(0, |old| old.seq().wrapping_add(1) & 0x00ff_ffff);
    let response = if register {
        response.without_observe().observe(sequence)
    } else {
        response.without_observe()
    };
    let mid = ids.next_for(engine, io, now_ms)?;
    let ty = if row.meta.ty == Type::Confirmable {
        Type::Confirmable
    } else {
        Type::NonConfirmable
    };
    let tx = prepare_deferred_observe::<S, T, REQUESTS>(
        engine, oscore, row, &response, ty, mid, now_ms,
    )?;
    let mut installed = None;
    if register {
        let mut interest = ObserveInterest::new(row.meta.token, row.meta.dest)
            .with_resource(observe.resource)
            .with_seq(sequence)
            .with_block2_max_szx(row.meta.block2.or(row.meta.q_block2).map(|b| b.szx()))
            .with_q_block2(row.meta.q_block2.is_some())
            .with_no_response(row.meta.no_response)
            .with_selection(Some(row.selection.stored()));
        interest = initial_observe_format(interest, &response, attempted_format);
        #[cfg(feature = "oscore")]
        {
            interest = interest.with_oscore(row.meta.oscore);
        }
        if previous.is_some_and(|old| old.is_pending()) {
            interest.mark_due();
        }
        let result = match previous_id {
            Some(id) => engine
                .storage_mut()
                .set_observe_interest(id, interest)
                .map(|()| id),
            None => engine
                .insert_observe(interest)
                .ok_or(crate::storage::SlotError::NotOccupied),
        };
        let id = match result {
            Ok(id) => id,
            Err(error) => {
                let _ = engine.release_tx(tx);
                return Err(error.into());
            }
        };
        row.observe.as_mut().expect("Observe completion").attempted =
            Some((sequence, interest.content_format()));
        installed = Some((id, interest));
        if engine
            .record_observe_notify(key, now_ms, mid, ty == Type::Confirmable)
            .is_none()
        {
            let _ = engine.release_tx(tx);
            return Err(Error::Saturated);
        }
    }
    if let Some(body) = body.filter(|body| row.snapshot.is_none_or(|(current, _)| current != *body))
        && let Err(error) = engine.release_tx_body(body)
    {
        if let Some((id, interest)) = installed {
            let _ = engine.storage_mut().set_observe_interest(id, interest);
        }
        let _ = engine.release_tx(tx);
        return Err(error.into());
    }
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid, ids.jitter()));
    if let Err(error) = finish_send(engine, io, tx, row.meta.dest, pending) {
        // Bytes may have reached the peer. Keep this sequence/format owner and
        // handle, but do not retain a hold for a TX that finish_send released.
        if let Some((id, interest)) = installed {
            let _ = engine.storage_mut().set_observe_interest(id, interest);
            if send_may_have_reached_peer(io, &error) {
                row.observe.as_mut().expect("Observe completion").failed_mid = Some((
                    mid,
                    now_ms.saturating_add(u64::from(Transmission::EXCHANGE_LIFETIME_MS)),
                ));
            }
        }
        return Err(error);
    }
    let now_ms = completion_time(io, now_ms);
    if register {
        if let Some((id, mut interest)) = installed {
            interest.record_notify(now_ms, mid, ty == Type::Confirmable);
            engine
                .storage_mut()
                .set_observe_interest(id, interest)
                .map_err(|_| Error::CommitAfterSend)?;
        }
        row.observe.as_mut().expect("registered Observe").failed_mid = None;
        if ty == Type::NonConfirmable {
            let _ = engine.refresh_observe_max_age(
                key,
                now_ms,
                response.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS),
                None,
            );
        }
    } else if let Some(previous) = previous {
        engine.take_observe(previous.key());
    }
    if !register {
        row.observe = None;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(complete_deferred_q_observe_async, bound(crate::app::async_server::ProtocolIo), calls(send_may_have_reached_peer => async_server::send_may_have_reached_peer_async, issue_deferred_q => issue_deferred_q_async, issue_q_window => issue_q_window_async, completion_time => async_server::completion_time_async))]
fn complete_deferred_q_observe<S: AppStorage, T: DatagramIo, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    row: &mut DeferredRow<S::Token>,
    response: &Response<'_, S::Token>,
) -> Result<(), Error<T::Error>> {
    let observed = row.observe.expect("Observe completion");
    if observed.attempted.is_some() && deferred_observe_owner(engine, *row).is_none() {
        return Err(Error::DeferredUnknown);
    }
    let key = ObserveKey::new(row.meta.token, row.meta.dest);
    let previous = engine.lookup_observe(key);
    let previous_body = response_body_for(engine, BlockKey::new(row.meta.token, row.meta.dest));
    if previous_body.is_some_and(|id| engine.tx_body_is_pinned(id)) {
        return Err(Error::Slot(crate::storage::SlotError::Pinned));
    }
    if observe_endpoint_held(engine, row.meta.dest, now_ms) >= usize::from(Transmission::NSTART) {
        return Err(Error::Saturated);
    }
    let room = previous.is_some()
        || (0..engine.capacities().observe_entries)
            .any(|index| engine.observe_interest(SlotId::from_index(index)).is_none());
    if !room {
        return Err(Error::Saturated);
    }
    let sequence = observed.attempted.map_or_else(
        || {
            previous
                .and_then(|id| engine.observe_interest(id))
                .filter(|old| old.resource() == observed.resource)
                .map_or(0, |old| old.seq().wrapping_add(1) & 0x00ff_ffff)
        },
        |(sequence, _)| sequence,
    );
    let response = response.without_observe().observe(sequence);
    let mid = ids.next_for(engine, io, now_ms)?;
    // Stage and size-check before committing the relation. The actual sender
    // encodes again so each protected datagram consumes its own fresh nonce.
    let tx = prepare_deferred_observe::<S, T, REQUESTS>(
        engine,
        oscore,
        row,
        &response,
        Type::NonConfirmable,
        mid,
        now_ms,
    )?;
    engine.release_tx(tx)?;
    let body = row.snapshot.expect("Q snapshot staged").0;
    if let Some(old) = previous_body.filter(|old| *old != body) {
        engine.release_tx_body(old)?;
    }
    #[allow(unused_mut)]
    let mut interest = ObserveInterest::new(row.meta.token, row.meta.dest)
        .with_resource(observed.resource)
        .with_seq(sequence)
        .with_content_format(response.format())
        .with_block2_max_szx(row.meta.q_block2.map(|q| q.szx()))
        .with_q_block2(true)
        .with_no_response(row.meta.no_response)
        .with_selection(Some(row.selection.stored()));
    interest = initial_observe_format(interest, &response, None);
    #[cfg(feature = "oscore")]
    {
        interest = interest.with_oscore(row.meta.oscore);
    }
    match previous {
        Some(id) => engine.storage_mut().set_observe_interest(id, interest)?,
        None => {
            engine.insert_observe(interest).ok_or(Error::Saturated)?;
        }
    }
    row.observe.as_mut().expect("Observe completion").attempted =
        Some((sequence, interest.content_format()));
    let q = row.meta.q_block2.expect("Q completion");
    let szx = engine.tx_body_transfer(body).expect("Q snapshot").szx();
    let meta = SendResponse {
        ty: Type::NonConfirmable,
        mid,
        q_block2: Some(BlockValue::new(q.num(), q.more(), szx).expect("negotiated Q SZX")),
        ..row.meta
    };
    let result = if row.meta.q_block2.is_some_and(|q| !q.more() || q.num() != 0)
        || row.q_selectors.iter().nth(1).is_some()
    {
        issue_deferred_q(
            engine,
            io,
            ids,
            now_ms,
            meta,
            &response,
            Type::NonConfirmable,
            body,
            Some(
                plan_deferred_q(row.q_selectors.iter(), response.payload().len())
                    .map_err(Error::Block)?,
            ),
            oscore,
            &mut None,
        )
    } else {
        issue_q_window(
            engine,
            io,
            ids,
            now_ms,
            meta,
            &response,
            Type::NonConfirmable,
            body,
            oscore,
            &mut None,
        )
    };
    let now_ms = completion_time(io, now_ms);
    // Keep a failed completion handle and immutable body retryable. The
    // installed relation owns the snapshot after successful completion.
    if result.is_ok() {
        record_q_observe_sent(engine, BlockKey::new(key.token(), key.endpoint()), now_ms);
        let _ = engine.refresh_observe_max_age(
            key,
            now_ms,
            response.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS),
            None,
        );
        row.snapshot = None;
    } else if let Err(error) = &result
        && send_may_have_reached_peer(io, error)
    {
        let sent = engine
            .tx_body_transfer(body)
            .and_then(|transfer| transfer.q_observe_last_mid())
            .unwrap_or(mid);
        row.observe.as_mut().expect("Q completion owner").failed_mid = Some((
            sent,
            now_ms.saturating_add(u64::from(Transmission::EXCHANGE_LIFETIME_MS)),
        ));
    }
    result
}

// Stage bytes and all bounded snapshot ownership before installing a relation or sending.
#[allow(clippy::too_many_arguments)]
fn prepare_deferred_observe<
    S: AppStorage,
    T: crate::storage::DatagramTransport,
    const REQUESTS: usize,
>(
    engine: &mut Engine<S>,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    row: &mut DeferredRow<S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    mid: MessageId,
    now_ms: u64,
) -> Result<SlotId, Error<T::Error>> {
    let metadata = deferred_reply_metadata(response, row.meta, now_ms).map_err(Error::Response)?;
    if let Some(q) = row
        .meta
        .q_block2
        .filter(|_| response.observe_seq().is_some() && response.code() != Code::VALID)
    {
        let existing = row.snapshot.map(|(body, _)| body);
        let body = if let Some((body, retained)) = row.snapshot {
            if !retained.same_header(&metadata)
                || !retained_body_matches(engine, body, response.payload())
            {
                return Err(Error::Block(BlockTransferError::IdentityMismatch));
            }
            body
        } else {
            let key = response_block_key(row.meta.token, row.meta.dest, response)?;
            response
                .etag_bytes()
                .ok_or(Error::Block(BlockTransferError::MissingIdentity))?;
            engine
                .start_q_block2(key, response.payload(), q.szx())
                .map_err(Error::Block)?
        };
        let mut transfer = engine.tx_body_transfer(body).expect("Q body staged");
        transfer.remember_notification(response.code(), response.format(), response.max_age_secs());
        transfer.remember_q_observe(
            response.observe_seq().expect("Q observation"),
            row.deadline_ms,
        );
        transfer.retain_observe_selection(Some(row.selection.stored()));
        #[cfg(feature = "oscore")]
        {
            transfer.oscore_request = row.meta.oscore.map(|request| (row.meta.token, request));
        }
        engine.storage_mut().set_tx_body_transfer(body, transfer)?;
        let probe_meta = SendResponse {
            q_block2: row
                .q_selectors
                .iter()
                .find(|q| q.num() != 0)
                .or(row.meta.q_block2),
            ..row.meta
        };
        let encoded = encode_q_observe_first::<S, T, REQUESTS>(
            engine, probe_meta, response, ty, mid, body, oscore,
        );
        if encoded.is_ok() {
            row.snapshot = Some((body, metadata));
        } else if existing.is_none() {
            let _ = engine.release_tx_body(body);
        }
        return encoded;
    }
    if let Some((body, retained)) = row.snapshot {
        if !retained.same_header(&metadata)
            || !retained_body_matches(engine, body, response.payload())
        {
            return Err(Error::Block(BlockTransferError::IdentityMismatch));
        }
    } else {
        let tx = admission::ordinary_tx(engine).ok_or(Error::Saturated)?;
        let preference = row
            .meta
            .block2
            .filter(|_| response.code().is_success() && response.code() != Code::VALID);
        let too_large =
            preference.is_some_and(|b| response.payload().len() > usize::from(b.size()));
        let encoded = if too_large {
            Err(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )))
        } else {
            encode_response(
                engine,
                row.meta.dest,
                tx,
                ty,
                mid,
                row.meta.token,
                response,
                response.payload(),
                preference.map(|value| BlockOpt {
                    value,
                    q_block: false,
                    size2: Some(response.payload().len()),
                }),
                None,
                oscore,
                row.meta.oscore,
            )
        };
        match encoded {
            Ok(()) => return Ok(tx),
            Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)))
                if response.code().is_success() && response.code() != Code::VALID =>
            {
                let _ = engine.release_tx(tx);
            }
            Err(error) => {
                let _ = engine.release_tx(tx);
                return Err(error);
            }
        }
    }
    let existing = row.snapshot.map(|(body, _)| body);
    let body = match existing {
        Some(body) => body,
        None => {
            response
                .etag_bytes()
                .ok_or(Error::Block(BlockTransferError::MissingIdentity))?;
            let key = response_block_key(row.meta.token, row.meta.dest, response)?;
            engine
                .start_block2(key, response.payload(), 6)
                .map_err(Error::Block)?
        }
    };
    let outcome = (|| {
        for szx in (0..=row.meta.block2.map_or(6, |b| b.szx())).rev() {
            let block = BlockValue::new(0, false, szx).expect("bounded SZX");
            let issued = engine.requested_block2(body, block).map_err(Error::Block)?;
            match encode_issued::<S, T, REQUESTS>(
                engine, row.meta, response, ty, mid, issued, false, oscore,
            ) {
                Ok(tx) => return Ok(tx),
                Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)))
                    if szx > 0 => {}
                Err(error) => return Err(error),
            }
        }
        unreachable!("finite SZX search returns at zero")
    })();
    if outcome.is_ok() {
        row.snapshot = Some((
            body,
            row.snapshot.map_or(metadata, |(_, retained)| retained),
        ));
    } else if existing.is_none() {
        let _ = engine.release_tx_body(body);
    }
    outcome
}

/// Engine storage selected by [`AppBuilder::block_wise`].
pub type AppStore<P, const BLOCK_WISE: bool> = <P as MemoryLayout<BLOCK_WISE>>::Store;

/// CoAP app: profile memory + transport + a bounded [`Site`].
///
/// Handlers see a borrowed [`Request`] and return an owned [`Response`].
/// Outbound work is [`Self::get`] / [`Self::put`] → [`Outgoing::send`] →
/// [`Self::take_response`]. Non-Block [`Response::payload`] copies at most
/// [`INLINE_PAYLOAD`] (128) bytes; [`Response::payload_truncated`] is
/// `true` when the datagram was longer. Assembled Block2 / Q-Block2 is
/// [`Response::body`]. The reactor owns per-slot state machines
/// inside [`Self::poll`]. `N` is the maximum number of routes (default 8);
/// raise it with [`AppBuilder::routes`]. `BLOCK_WISE` is
/// [`AppBuilder::block_wise`]: `false` stores only [`crate::storage::Memory<P>`] (no body
/// pool arrays and no client Block2 assembled hold). You do not need
/// [`crate::storage::Access`] on this path — [`Self::engine_mut`] is the
/// advanced escape hatch.
pub struct App<
    P: MemoryProfile = crate::profiles::Default,
    T = (),
    const N: usize = DEFAULT_ROUTES,
    const BLOCK_WISE: bool = false,
    S: AppStorage<Token = P::Token> = AppStore<P, BLOCK_WISE>,
    const DEFERRED: usize = 0,
    const CALLS: usize = { DEFAULT_CLIENT_CALLS },
    const REQUESTS: usize = 4,
    const ASSOCIATIONS: usize = 1,
> where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    engine: Engine<S>,
    io: T,
    site: Site<N, S::Token>,
    ids: AppIds,
    inbox: client::ClientInbox<CALLS, S::Token>,
    lives: client::ClientLives<CALLS, S::Token>,
    echo_policy: Option<EchoPolicy>,
    full_responses: bool,
    deferred_lifetime_ms: u64,
    /// Last POST/PATCH/FETCH whose Dedup insert failed. A CON retransmit
    /// of that Message ID + peer is ACKed without a handler re-run until
    /// `due_ms` (fail-closed).
    dedup_closed: Option<DedupClosed>,
    /// Caller-owned OSCORE context (`()` without the `oscore` feature).
    oscore: oscore::Store<REQUESTS, ASSOCIATIONS, S::Token>,
    oscore_checkpoint: oscore::CheckpointState,
    /// Client Block2 / Q-Block2 snapshot for [`Self::take_response`].
    /// Present when `BLOCK_WISE`; zero-sized otherwise.
    assembled: AssembledField<P, BLOCK_WISE>,
    deferred: DeferredTable<DEFERRED, S::Token>,
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
    const DEFERRED: usize = 0,
    const CALLS: usize = { DEFAULT_CLIENT_CALLS },
    const REQUESTS: usize = 4,
    const ASSOCIATIONS: usize = 1,
> {
    site: Site<N, P::Token>,
    echo_policy: Option<EchoPolicy>,
    full_responses: bool,
    deferred_lifetime_ms: u64,
    identity: Option<IdentitySource>,
    token_length: Option<usize>,
    oscore: oscore::Store<REQUESTS, ASSOCIATIONS, P::Token>,
    oscore_checkpoint: oscore::CheckpointState,
    allow_unprotected: bool,
    _p: PhantomData<P>,
    _b: PhantomData<Block>,
}

impl<
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
    S: AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
{
    /// Selected OSCORE request-binding capacity, independent of Calls.
    /// The table is absent when the `oscore` feature is disabled.
    pub const fn security_request_capacity(&self) -> usize {
        REQUESTS
    }

    /// Number of context slots selected independently of shared packet pools.
    pub const fn association_capacity(&self) -> usize {
        ASSOCIATIONS
    }

    /// Installed identity at a bounded slot, including a revoked row awaiting unpinning.
    #[cfg(feature = "oscore")]
    pub fn association_at(&self, index: usize) -> Option<Association> {
        self.oscore.rows.get(index)?.as_ref()?.association
    }

    /// Whether an installed handle still permits traffic and effects.
    #[cfg(feature = "oscore")]
    pub fn association_is_active(&self, association: Association) -> bool {
        self.oscore
            .rows
            .iter()
            .flatten()
            .any(|row| !row.revoked && row.association == Some(association))
    }

    /// Borrow exact association state for durable sequence/replay management.
    #[cfg(feature = "oscore")]
    pub fn association_context(
        &self,
        association: Association,
    ) -> Option<&crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>> {
        self.oscore
            .rows
            .iter()
            .flatten()
            .find(|row| row.association == Some(association))
            .map(|row| &row.context)
    }

    /// Advance durable sender reservations or restore state before traffic begins.
    /// As with `oscore_mut`, replacing keys while protocol work is live is forbidden.
    #[cfg(feature = "oscore")]
    pub fn association_context_mut(
        &mut self,
        association: Association,
    ) -> Option<&mut crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>> {
        self.oscore_checkpoint.invalidate();
        self.oscore
            .rows
            .iter_mut()
            .flatten()
            .find(|row| !row.revoked && row.association == Some(association))
            .map(|row| &mut row.context)
    }

    /// Inline OSCORE field bytes, including keys, replay state, request bindings
    /// association identity, slot state and alignment. Zero without `oscore`. Excludes temporary stack and caller
    /// persistence/runtime storage; allocated packet pools do not move this field.
    pub const fn security_metadata_bytes(&self) -> usize {
        core::mem::size_of::<oscore::Store<REQUESTS, ASSOCIATIONS, S::Token>>()
    }

    /// Number of live client Calls/completions selected at build time.
    pub const fn client_capacity(&self) -> usize {
        CALLS
    }

    /// Inline client-table bytes, including retained response metadata.
    ///
    /// Excludes packet/body pools, security state, other App fields and
    /// temporary stack space. Allocator-backed packet storage does not move
    /// these tables to the heap.
    pub const fn client_metadata_bytes(&self) -> usize {
        core::mem::size_of::<client::ClientInbox<CALLS, S::Token>>()
            + core::mem::size_of::<client::ClientLives<CALLS, S::Token>>()
    }

    /// Bind `methods` on Uri-Path `path`.
    ///
    /// `path` is [`IntoPath`]: `&["sensors", "temp"]` or `"sensors/temp"`.
    /// Panics on invalid paths or a new path at capacity; use [`Self::try_route`]
    /// for recoverable refusal.
    pub fn route(&mut self, path: impl IntoPath, methods: MethodRouter<S::Token>) -> &mut Self {
        self.site.route(path, methods);
        self
    }

    /// Bind `methods` on a `'static` URI-Path (`"/leds/0"`).
    pub fn route_path(&mut self, path: &'static str, methods: MethodRouter<S::Token>) -> &mut Self {
        self.site.route_path(path, methods);
        self
    }

    /// Checked route registration; refusal preserves all existing routes.
    ///
    /// See [`Site::try_route`] for path and capacity rules. Replacement has the
    /// same semantics as [`Self::route`]; it does not cancel live observations.
    pub fn try_route(
        &mut self,
        path: impl IntoPath,
        methods: MethodRouter<S::Token>,
    ) -> Result<&mut Self, RouteError> {
        self.site.try_route(path, methods)?;
        Ok(self)
    }

    /// Checked registration of a static slash-separated path.
    pub fn try_route_path(
        &mut self,
        path: &'static str,
        methods: MethodRouter<S::Token>,
    ) -> Result<&mut Self, RouteError> {
        self.try_route(path, methods)
    }

    /// Remove a route only after all App-owned protocol work has drained.
    ///
    /// Returns `false` for an unknown path, even while busy. Invalid paths and
    /// busy refusal preserve routes, keys, checkpoints and protocol state and
    /// perform no I/O. Remaining route identities stay stable; freed capacity
    /// can be reused with [`Self::try_route`].
    ///
    /// Drain or cancel Calls, deferred work, Observe interests, packet/body
    /// slots, exchanges and cached/duplicate history first. This conservative
    /// check covers the whole App, including work for other paths. Collected
    /// response backing alone does not keep the App busy.
    ///
    /// This is local handler configuration, not policy revocation. External
    /// adapter queues, caller-held advanced resource identities and current
    /// policy are caller-owned; discard old identities before slot reuse.
    pub fn remove_route(&mut self, path: impl IntoPath) -> Result<bool, RouteRemovalError> {
        let path = request::path_from_into(path).map_err(RouteRemovalError::Path)?;
        if !self.site.contains_path(path) {
            return Ok(false);
        }
        if self.has_protocol_state() {
            return Err(RouteRemovalError::Busy);
        }
        Ok(self.site.remove_path(path))
    }

    fn has_protocol_state(&mut self) -> bool {
        if self.inbox.has_protocol_state()
            || self.lives.has_protocol_state()
            || self.deferred.rows.iter().any(Option::is_some)
            || self.dedup_closed.is_some()
            || oscore::has_live_requests(&self.oscore)
        {
            return true;
        }
        let storage = self.engine.storage_mut();
        !storage.rx_datagram().is_empty()
            || !storage.tx_datagram().is_empty()
            || !storage.dedup().is_empty()
            || !storage.observe().is_empty()
            || storage.rx_body().is_some_and(|pool| !pool.is_empty())
            || storage.tx_body().is_some_and(|pool| !pool.is_empty())
            || (0..storage.capacities().tx_datagram_slots)
                .any(|index| storage.exchange_entry(SlotId::from_index(index)).is_some())
    }

    /// Serve `/.well-known/core` from registered paths.
    ///
    /// See [`AppBuilder::well_known_core`].
    pub const fn well_known_core(&mut self) -> &mut Self {
        self.site.well_known_core();
        self
    }

    /// Install an explicit server Echo issuer/verifier. See [`EchoPolicy`].
    pub const fn echo_policy(&mut self, policy: EchoPolicy) -> &mut Self {
        self.echo_policy = Some(policy);
        self
    }

    /// Advanced mutable access to the site table.
    ///
    /// Direct Site mutation bypasses App lifecycle checks. Prefer
    /// [`Self::remove_route`] for removal and discard caller-held identities
    /// of removed routes before any slot reuse.
    pub fn site(&mut self) -> &mut Site<N, S::Token> {
        &mut self.site
    }

    /// Borrow the Engine (advanced path).
    ///
    /// The happy path is [`Self::poll`]. Use this when you need explicit
    /// slots or [`crate::storage::Access`].
    #[must_use]
    pub const fn engine(&self) -> &Engine<S> {
        &self.engine
    }

    /// Earliest armed protocol timeout in the caller's monotonic millisecond clock.
    ///
    /// Covers CON retransmissions, Call deadlines/lifetimes, overload cooldowns,
    /// Q-Block pacing/recovery, Observe lifetimes, deferred work and dedup expiry.
    /// Zero requests an immediate poll to arm a Q-Block receive wait. Past values
    /// remain past until polling processes them; this query never consumes work,
    /// advances a cursor, reads a clock, performs I/O or allocates memory.
    ///
    /// This is the timer input to an event loop, not a complete readiness test.
    /// Also poll for socket ingress, queued datagrams, signalled notifications,
    /// checkpoint retries and application state changes. `None` means no timed
    /// work, not that these other sources are idle. Pinned resources or failing
    /// I/O/checkpoints can prevent progress: wait for their owner or retry policy
    /// rather than busy-looping on an expired timeout. Recompute after mutations.
    ///
    /// Scans configured TX, Observe, RX-body, dedup, Call and deferred capacities.
    /// Each Call may additionally scan exchange and Observe tables. Uses constant
    /// scratch space and no additional stored timer metadata on either backend.
    #[must_use]
    pub fn next_timeout(&self) -> Option<u64> {
        self.next_timeout_after(None)
    }

    // Exclude work already being handled, while retaining every later candidate
    // rather than filtering only the final minimum (which can hide another Call).
    pub(crate) fn next_timeout_after(&self, after: Option<u64>) -> Option<u64> {
        let mut next = self
            .lives
            .next_timeout_after(&self.engine, &self.inbox, after);
        let mut consider = |due: u64| {
            if after.is_none_or(|after| due > after) {
                next = Some(next.map_or(due, |old| old.min(due)));
            }
        };
        let capacities = self.engine.capacities();
        for index in 0..capacities.tx_datagram_slots {
            if let Some(due) = self
                .engine
                .pending_con(SlotId::from_index(index))
                .and_then(|pending| pending.rto().next_timeout_ms())
            {
                consider(due);
            }
        }
        for index in 0..capacities.observe_entries {
            if after.is_some()
                && let Some(crate::storage::ObserveNotifyHold::Non { until_ms, .. }) = self
                    .engine
                    .observe_interest(SlotId::from_index(index))
                    .and_then(|row| row.notify_hold())
            {
                consider(until_ms);
            }
            if let Some(lifetime) = self
                .engine
                .observe_interest(SlotId::from_index(index))
                .and_then(|interest| interest.lifetime())
            {
                consider(lifetime.due_ms());
            }
        }
        for index in 0..capacities.rx_body_slots.unwrap_or(0) {
            if let Some(transfer) = self.engine.rx_body_transfer(SlotId::from_index(index))
                && transfer.q_holes().is_some()
            {
                consider(
                    transfer
                        .q_receive()
                        .map_or(0, |wait| wait.next_timeout_ms()),
                );
            }
        }
        for index in 0..capacities.tx_body_slots.unwrap_or(0) {
            if let Some(deadline) = self
                .engine
                .tx_body_transfer(SlotId::from_index(index))
                .and_then(|t| t.terminal_deadline())
            {
                consider(deadline);
            }
        }
        for index in 0..capacities.dedup_entries {
            if let Some(entry) = self.engine.dedup_entry(SlotId::from_index(index))
                && entry.due_ms() != 0
            {
                consider(entry.due_ms());
            }
        }
        if let Some(closed) = self.dedup_closed
            && closed.due_ms != 0
        {
            consider(closed.due_ms);
        }
        for row in self.deferred.rows.iter().flatten() {
            consider(row.deadline_ms);
        }
        next
    }

    /// Next poll opportunity from retained ingress, notifications and protocol timers.
    ///
    /// Returns `Some(now_ms)` for ready work or elapsed timers, a later absolute
    /// time for scheduled work, or `None` when these sources have no opportunity.
    /// Held full responses and pinned RX slots are excluded. Deferred Observe
    /// work and endpoint NSTART holds suppress notification readiness; NON holds
    /// supply a wake time and CON holds use their retransmission timer.
    ///
    /// Recompute after polling or changing App state. Socket readiness, released
    /// pins, new application signals, checkpoint retries and policy changes must
    /// wake the event loop independently. A returned opportunity does not promise
    /// successful I/O or admission; back off failures in the caller's event loop.
    ///
    /// No I/O, allocation, cursor advancement or timer mutation occurs. The mutable
    /// borrow is required by the storage occupancy interface. In addition to
    /// [`Self::next_timeout`], scans RX slots and pending Observe interests.
    /// Each interest scans deferred and congestion state; congestion checks may
    /// scan Observe capacity once per TX slot. All scans are capacity-bounded
    /// and use constant scratch space.
    #[must_use]
    pub fn next_poll_at(&mut self, now_ms: u64) -> Option<u64> {
        let mut next = self.next_timeout().map(|due| due.max(now_ms));
        let capacities = self.engine.capacities();
        for index in 0..capacities.rx_datagram_slots {
            let id = SlotId::from_index(index);
            let pool = self.engine.storage_mut().rx_datagram();
            if pool.is_occupied(id) && !pool.is_pinned(id) && !self.inbox.holds_datagram(id) {
                return Some(now_ms);
            }
        }
        for index in 0..capacities.tx_body_slots.unwrap_or(0) {
            let id = SlotId::from_index(index);
            if let Some(transfer) = self.engine.tx_body_transfer(id)
                && transfer.q_observe_confirmable()
                && transfer.q_observe_pending_mid().is_none()
                && !transfer.is_complete()
                && !self.engine.tx_body_is_pinned(id)
                && oscore::route_allowed(&self.oscore, transfer.endpoint())
                && observe_endpoint_held(&self.engine, transfer.endpoint(), now_ms)
                    < usize::from(Transmission::NSTART)
            {
                return Some(now_ms);
            }
        }
        for index in 0..capacities.observe_entries {
            let Some(interest) = self.engine.observe_interest(SlotId::from_index(index)) else {
                continue;
            };
            if interest.key().is_client()
                || !interest.is_pending()
                || self.deferred.blocks_observe(interest.key())
                || (!interest.is_deleted()
                    && self.site.observe_source(interest.resource()).is_none())
            {
                continue;
            }
            if observe_endpoint_held(&self.engine, interest.endpoint(), now_ms)
                < usize::from(Transmission::NSTART)
            {
                return Some(now_ms);
            }
            for held_index in 0..capacities.observe_entries {
                if let Some(held) = self.engine.observe_interest(SlotId::from_index(held_index))
                    && !held.key().is_client()
                    && held.endpoint().physical() == interest.endpoint().physical()
                    && let Some(crate::storage::ObserveNotifyHold::Non { until_ms, .. }) =
                        held.notify_hold()
                    && until_ms > now_ms
                {
                    next = Some(next.map_or(until_ms, |old| old.min(until_ms)));
                }
            }
        }
        next
    }

    /// Mutably borrow the Engine (advanced path).
    ///
    /// Escape hatch for explicit slots, [`crate::storage::Access`] /
    /// [`crate::storage::AccessMut`], custom RST / remaining 4.xx, and
    /// codecs not exposed by App, such as BERT. [`Self::poll`] already pins and
    /// releases; App handlers do not need this.
    pub const fn engine_mut(&mut self) -> &mut Engine<S> {
        &mut self.engine
    }

    /// Attach a caller-owned pairwise OSCORE context.
    ///
    /// Prefer `AppBuilder::oscore` for initial construction; ordinary bind
    /// requires a context. This setter retains explicit replacement support
    /// and never removes protection or enables unprotected fallback.
    ///
    /// This setter configures the single primary context, with `REQUESTS` live
    /// request bindings shared by Calls and Observe. It refuses once explicit
    /// associations have been installed. Replacing the primary context
    /// does not migrate live exchanges, tokens, replay state, or subscriptions.
    /// Drain, collect, cancel or expire work under the current context first,
    /// including cached replies and fail-closed dedup history. Busy refusal
    /// leaves both the App and the caller's staged candidate unchanged, performs
    /// no I/O, and preserves checkpoint status. Success takes the candidate and
    /// invalidates the durable checkpoint; protection is never removed.
    ///
    /// Stage rare replacements in a caller-owned `Option`, then call
    /// `app.set_oscore(&mut candidate)?`. An empty option returns
    /// [`OscoreInstallError::MissingCandidate`]; live App state returns
    /// [`OscoreInstallError::Busy`]. The compact error does not move or expose
    /// context secrets and requires no allocation. Already-collected reply
    /// backing is cleared on success and does not keep the App busy forever.
    ///
    /// This checks App-owned lifecycle state, not key freshness, safe same-key
    /// rollback, external adapter queues or revocation policy. `oscore_mut` and
    /// `engine_mut` remain advanced escape hatches, not context-rotation APIs.
    /// Use separate Apps for separate peers or simultaneous key epochs.
    /// Timed Q-Block2 recovery for a live client GET uses a fresh protected
    /// request and retains its path, queries and selection options. Only the
    /// latest request binding is retained; late responses to earlier requests
    /// are not qualified. Timed server Q-Block1 reports retain the Token and
    /// request reference with the highest authenticated request Partial IV in
    /// the body sidecar, so reordered older payloads cannot replace it. Each
    /// report uses a fresh response Partial IV; failed sends retain bounded
    /// retry state and never reuse that sequence. Body completion/expiry drops
    /// the reference. This adds 24 bytes per body sidecar on the 64-bit host
    /// (96 bytes across Default RX/TX body slots), only with OSCORE enabled.
    /// Advanced receive bodies without a retained authenticated request or a
    /// live client Call return [`crate::oscore::Error::Unsupported`] without
    /// sending unprotected while a context is attached.
    /// Ordinary Q-Block2 response batches use fresh response Partial IVs and
    /// retain the request binding through assembly until response collection
    /// or cancellation. Recipient replay checks use the context's bounded
    /// 32-sequence window; this does not qualify Q Observe.
    ///
    /// Protected responses must match the Call endpoint and any piggybacked
    /// ACK Message ID before request-binding or notification replay state is
    /// consumed. Established notifications match the client Observe relation;
    /// unrelated protected CON responses receive an empty matching Reset;
    /// unmatched ACK/NON responses are silently discarded. No payload is opened
    /// and no request binding or replay state is consumed by this rejection. Authentication
    /// alone does not retire an ordinary request binding: rejected responses
    /// and failed ACK sends leave pending Calls usable. Collection, cancellation
    /// and terminal local failure release the binding; continuation replaces it.
    /// Response replay bookkeeping commits after critical-option validation and
    /// successful CON acknowledgment. This is in-memory admission, not a durable
    /// replay checkpoint barrier for application effects.
    ///
    /// Requires the `oscore` crate feature. You derive
    /// [`crate::oscore::SecurityContext`] (Master Secret, Sender/Recipient
    /// IDs, replay window). Engine does not store keys. Protect happens
    /// when a datagram is encoded into a TX slot, so CON retransmit
    /// reuses the same ciphertext.
    ///
    /// This slice covers a pairwise context on the happy-path
    /// request/response, Observe register/notify, Inner Block-wise
    /// (Block1/Block2: fragment, then protect), and Dual-class
    /// Max-Age / No-Response / ETag placement. Outer Block-wise (proxy
    /// hop-by-hop), group OSCORE, and other ciphers are out.
    ///
    /// Fail-closed while a context is attached: a non-empty datagram
    /// without an OSCORE option is rejected (4.01 on a request; a plain
    /// 2.xx or plain Block completion does not complete a [`Call`]).
    /// Empty ACK/RST stay unprotected (RFC 7252 reliability). AEAD
    /// failure is a silent drop.
    #[cfg(feature = "oscore")]
    pub fn set_oscore(
        &mut self,
        candidate: &mut Option<crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>>,
    ) -> Result<&mut Self, OscoreInstallError> {
        if candidate.is_none() {
            return Err(OscoreInstallError::MissingCandidate);
        }
        if self.has_protocol_state() || self.oscore.generation != 0 {
            return Err(OscoreInstallError::Busy);
        }
        // Collected replies are only borrow backing, not outstanding work.
        self.inbox = client::ClientInbox::new();
        P::store(&mut self.assembled, &[]);
        self.lives = client::ClientLives::new();
        self.oscore
            .set_primary(candidate.take().expect("checked candidate"));
        self.oscore_checkpoint.invalidate();
        Ok(self)
    }

    #[cfg(feature = "edhoc")]
    pub(crate) fn check_association_install<U: OwnedToken>(
        &self,
        endpoint: Endpoint,
        owner: [u8; 32],
        epoch: u64,
        context: &crate::oscore::BoundedSecurityContext<REQUESTS, U>,
    ) -> Result<(), OscoreInstallError> {
        self.oscore
            .check_install(endpoint, owner, epoch, context)
            .map(|_| ())
    }

    /// Install an independently owned context without reallocating shared pools.
    /// The caller must authenticate owner, epoch and endpoint before installation.
    /// Refusal leaves the candidate and existing associations unchanged.
    #[cfg(feature = "oscore")]
    pub fn install_association(
        &mut self,
        endpoint: Endpoint,
        owner: [u8; 32],
        epoch: u64,
        candidate: &mut Option<crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>>,
    ) -> Result<Association, OscoreInstallError> {
        let association = self.oscore.install(endpoint, owner, epoch, candidate)?;
        self.oscore_checkpoint.invalidate();
        Ok(association)
    }

    /// Revoke all local work for this association and reclaim its context.
    /// Revocation is immediate even when a pinned buffer prevents reclamation.
    /// `Busy` means the association stays disabled: release its pins and retry.
    /// Other associations keep their keys, retained work and capacity ownership.
    #[cfg(feature = "oscore")]
    pub fn remove_association(
        &mut self,
        association: Association,
    ) -> Result<crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>, OscoreInstallError> {
        let index = self
            .oscore
            .rows
            .iter()
            .position(|row| {
                row.as_ref()
                    .is_some_and(|row| row.association == Some(association))
            })
            .ok_or(OscoreInstallError::UnknownAssociation)?;
        let peer = association.route;
        client::revoke_association(
            &mut self.engine,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            peer,
        );
        self.oscore.rows[index]
            .as_mut()
            .expect("matched association")
            .revoked = true;
        for row in &mut self.deferred.rows {
            if row.as_ref().is_some_and(|row| row.meta.dest == peer) {
                *row = None;
            }
        }
        if self
            .dedup_closed
            .is_some_and(|row| row.key.endpoint() == peer)
        {
            self.dedup_closed = None;
        }
        let caps = self.engine.capacities();
        for i in 0..caps.dedup_entries {
            if let Some(row) = self
                .engine
                .dedup_entry(SlotId::from_index(i))
                .filter(|row| row.key().endpoint() == peer)
            {
                self.engine.remove_dedup(row.key());
            }
        }
        for i in 0..caps.observe_entries {
            if let Some(row) = self
                .engine
                .observe_interest(SlotId::from_index(i))
                .filter(|row| row.endpoint() == peer)
            {
                self.engine.remove_observe(row.key());
            }
        }
        let mut pinned = false;
        for i in 0..caps.tx_datagram_slots {
            let id = SlotId::from_index(i);
            if let Some(row) = self
                .engine
                .exchange_entry(id)
                .filter(|row| row.endpoint() == peer)
            {
                self.engine.take_exchange(row.key());
            }
            if let Some(row) = self
                .engine
                .pending_con(id)
                .filter(|row| row.endpoint() == peer)
            {
                self.engine.take_pending_con(row.message_id(), peer);
            }
            if self.engine.tx_endpoint(id) == Some(peer) {
                pinned |= self.engine.release_tx(id) == Err(SlotError::Pinned);
            }
        }
        for i in 0..caps.rx_datagram_slots {
            let id = SlotId::from_index(i);
            if self.engine.rx_endpoint(id) == Some(peer) {
                pinned |= self.engine.release_rx(id) == Err(SlotError::Pinned);
            }
        }
        for i in 0..caps.rx_body_slots.unwrap_or(0) {
            let id = SlotId::from_index(i);
            if self
                .engine
                .rx_body_transfer(id)
                .is_some_and(|row| row.key().endpoint() == peer)
            {
                pinned |= self.engine.release_rx_body(id) == Err(SlotError::Pinned);
            }
        }
        for i in 0..caps.tx_body_slots.unwrap_or(0) {
            let id = SlotId::from_index(i);
            if self
                .engine
                .tx_body_transfer(id)
                .is_some_and(|row| row.key().endpoint() == peer)
            {
                pinned |= self.engine.release_tx_body(id) == Err(SlotError::Pinned);
            }
        }
        if pinned {
            return Err(OscoreInstallError::Busy);
        }
        self.oscore_checkpoint.invalidate();
        Ok(self.oscore.rows[index]
            .take()
            .expect("matched association")
            .context)
    }

    /// Borrow the attached OSCORE context, if any.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn oscore(&self) -> Option<&crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>> {
        self.oscore.as_ref()
    }

    /// Mutably borrow the attached OSCORE context.
    ///
    /// Advanced escape hatch: this bypasses the lifecycle checks in
    /// [`Self::set_oscore`]. Do not replace keys or reset sender/replay state
    /// while App-owned protocol work remains live.
    #[cfg(feature = "oscore")]
    pub fn oscore_mut(
        &mut self,
    ) -> Option<&mut crate::oscore::BoundedSecurityContext<REQUESTS, S::Token>> {
        self.oscore_checkpoint.invalidate();
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
    /// # impl coaptic::storage::DatagramTransport for NullIo {
    /// #     type Error = &'static str;
    /// # }
    /// # impl DatagramIo for NullIo {
    /// #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
    /// #         Ok(None)
    /// #     }
    /// #     fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> { Ok(bytes.len()) }
    /// # }
    /// let mut app = App::profile::<profiles::Default>()
    ///     .randomness(|bytes| getrandom::fill(bytes).is_ok())
    ///     .block_wise::<false>()
    ///     .allow_unprotected()
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

    /// Snapshot the six core storage areas without allocation or retained state.
    ///
    /// See [`crate::storage::StorageUsage`] for scope and sampling limits.
    #[must_use]
    pub fn storage_usage(&mut self) -> crate::storage::StorageUsage {
        self.engine.storage_usage()
    }

    /// Snapshot App admission storage, including retained replies and cooldowns.
    ///
    /// Work is bounded by the configured Call, deferred and association counts.
    /// See [`AppUsage`] for scope and lifetime semantics.
    #[must_use]
    pub fn resource_usage(&mut self) -> AppUsage {
        use crate::storage::PoolUsage;
        let (requests, cooldowns) = self.lives.usage();
        let (replies, completed) = self.inbox.usage();
        #[cfg(feature = "oscore")]
        let (security, revoked) = (
            Some(PoolUsage {
                occupied: self.oscore.rows.iter().flatten().count(),
                capacity: ASSOCIATIONS,
            }),
            self.oscore
                .rows
                .iter()
                .flatten()
                .filter(|row| row.revoked)
                .count(),
        );
        #[cfg(not(feature = "oscore"))]
        let (security, revoked) = (None, 0);
        AppUsage {
            storage: self.engine.storage_usage(),
            ordinary_tx_capacity: self
                .engine
                .capacities()
                .tx_datagram_slots
                .saturating_sub(admission::TX_HEADROOM),
            full_response_capacity: self
                .engine
                .capacities()
                .rx_datagram_slots
                .saturating_sub(admission::RX_HEADROOM),
            routes: PoolUsage {
                occupied: self.site.len(),
                capacity: N,
            },
            client_requests: PoolUsage {
                occupied: requests,
                capacity: CALLS,
            },
            client_cooldowns: cooldowns,
            client_replies: PoolUsage {
                occupied: replies,
                capacity: CALLS,
            },
            completed_replies: completed,
            deferred: PoolUsage {
                occupied: self.deferred.rows.iter().flatten().count(),
                capacity: DEFERRED,
            },
            security,
            revoked_associations: revoked,
        }
    }

    /// Copy optional request-path work counters from the Engine.
    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn work_metrics(&self) -> crate::storage::WorkMetrics {
        self.engine.work_metrics()
    }

    /// Reset work counters independently of the always-on reactor counters.
    #[cfg(feature = "diagnostics")]
    pub fn reset_work_metrics(&mut self) {
        self.engine.reset_work_metrics();
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

impl<
    P,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
    S: AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
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
    /// (RFC 7252 ping). Incomplete Q-Block1 gets an empty ACK for CON;
    /// NON payloads get 2.31 only after a complete payload set, with the
    /// acknowledged Q-Block1 NUM. Successful Q-Block2 responses use all-NON
    /// payloads; a CON request receives a separate empty ACK before the set.
    /// Q recovery and partial-body expiry run after
    /// ingress; exhausted client Q downloads complete with [`CallFailure::TimedOut`].
    /// Incomplete classic Block1 is 2.31 (handler not
    /// run) and echoes the stored Block1 SZX; a complete body is [`Request::body`].
    /// The server does not advertise a smaller SZX: an accepted block already
    /// fits the fixed datagram and body slot. Unrecognized critical
    /// options (not in the implemented set) and an OSCORE option with no
    /// attached context are 4.02 before the handler. A large response ships
    /// as Block2 / Q-Block2 from the TX body. Site misses are 4.04 / 4.05
    /// with [`Response::problem`]. Apply-error 4.08 uses problem details;
    /// Q-Block1 holes after `NON_RECEIVE_TIMEOUT` use
    /// [`Response::missing_blocks`]. An installed [`Self::echo_policy`] can
    /// challenge or reject with 4.01 before processing. Location-Path /
    /// Location-Query on the [`Response`]
    /// are written on the wire. [`Response::separate`] is an empty ACK
    /// to a CON request, then the completed representation in a new CON in
    /// the same poll (NON request â†’ NON). Use [`Self::poll_with`] and
    /// [`Self::complete`] for work completed outside the reactor. A retransmitted CON request (same
    /// Message ID + peer) is answered from the Dedup Table without a
    /// second handler call while the row is live (`EXCHANGE_LIFETIME`):
    /// the cached ACK bytes, or a pinned TX slot when the ACK does not
    /// fit the compact sidecar. POST / PATCH / FETCH never re-run on a
    /// live row (empty ACK if there is nothing to replay). If Dedup insert
    /// fails after the first POST / PATCH / FETCH (table full of
    /// never-expire rows), a retransmit is still ACKed without a handler
    /// re-run until `EXCHANGE_LIFETIME` (fail-closed, not Miss). GET / PUT /
    /// DELETE / iPATCH replay when a cache exists; without one, re-run is
    /// allowed (RFC 7252 §4.5 MAY). After `EXCHANGE_LIFETIME` expiry,
    /// Dedup Miss is a new exchange (POST / PATCH / FETCH may re-run).
    /// Proxy-Uri or Proxy-Scheme on a request is 5.05 before the site
    /// (origin; this crate is not a forward-proxy).
    /// Accepted CON responses also retain an empty ACK in this bounded table,
    /// including protected responses after reply collection. Replay is keyed
    /// by peer and Outer MID, without redelivering the response. Failed initial
    /// ACK sends do not install a replay. Rows expire at `EXCHANGE_LIFETIME`;
    /// table pressure can evict older App rows, so callers must size the profile
    /// for their concurrent request/notification history. This adds no body pool.
    /// An OSCORE CON retransmit is keyed on the *outer* Message ID so the
    /// cached protected ACK is replayed without a second unprotect.
    /// Classic Block2 ACKs use the same bounded cache, including protected
    /// fragments. Large replies can pin TX datagrams until expiry or pressure
    /// eviction; callers must budget reply history as well as active work.
    /// Observe register / deregister and
    /// [`ObserveSource`] notify run here. Pending notification signals are
    /// claimed after ingress: cancellation/re-registration cannot transfer a
    /// signal to a replacement row, and an earlier send failure preserves it.
    /// Caller-built notifications use
    /// [`Self::notify`]. Empty RST matching a notification Message ID
    /// drops that observer (RFC 7641 §4.5; RST has no Token).
    /// Max-Age expiry only makes representation data stale; it does not
    /// remove client or server Observe relations. CON notification retry
    /// exhaustion removes the server relation independently of Max-Age.
    /// Successful DELETE queues a terminal 4.04 CON for each server observer,
    /// preserving notification holds and retrying local send failures. Poll
    /// delivers these without requiring an ObserveSource. Each observer is
    /// removed after sending; pending CON delivery remains until ACK/give-up.
    ///
    /// **Outbound.** Token + peer match a [`Call`]; [`Self::take_response`]
    /// is the [`Response`]. Non-Block [`Response::payload`] copies at most
    /// [`INLINE_PAYLOAD`] (128) bytes ([`Response::payload_truncated`] when
    /// the datagram was longer). [`Response::body`] is the assembled Block2
    /// / Q-Block2 representation. Continues reuse the path from
    /// [`Outgoing::send`]. Uri-Query,
    /// Accept, ETag, If-Match / If-None-Match, and early Block2 are set on
    /// [`Outgoing`]. An Observe subscribe ([`Outgoing::observe`]) uses the
    /// same [`Call`].
    ///
    /// Retained Q-Block2 representations support block-zero and repeated
    /// Continue reselection without acknowledging the current set again.
    /// Future Continue sets must follow the issued window. Missing-block
    /// reports are Non-confirmable, at most one per progress pass, and are
    /// not gated by endpoint NSTART or PROBING_RATE.
    ///
    /// **Retransmit.** Engine schedules CON RTO inside [`Engine::progress`]
    /// / [`Engine::poll_retransmit`] and does not send. This poll sends on
    /// [`Retransmit::Due`] and releases on [`Retransmit::GiveUp`]. A matching
    /// response (piggybacked ACK or separate CON/NON) clears that pending CON
    /// using the request Message ID. Empty RST matching a client outstanding
    /// request forgets the exchange. Empty ACK then silence, and lost NON,
    /// expire after RFC 7252 §4.8.2 `EXCHANGE_LIFETIME` / `NON_LIFETIME`.
    /// `now_ms` is the caller monotonic clock. Production CON jitter comes
    /// from the configured [`RandomSource`]; retransmits retain that schedule.
    /// Receive errors, including refusal of oversized datagrams, still allow
    /// this bounded progress pass. If later dispatch or progress fails, that
    /// error takes precedence; otherwise the receive error is returned. An
    /// error therefore does not mean that no datagrams were sent or retained
    /// work was dispatched. Bound transport calls and callbacks separately
    /// when budgeting wall-clock polling latency.
    /// At most one eligible RX datagram is dispatched per poll. Queued unpinned
    /// datagrams take precedence over fresh ingress; retained full responses
    /// are excluded during selection. With successful dispatch opportunities,
    /// queued work progresses within the RX slot count. Pinned buffers, failing
    /// I/O and caller callback duration remain outside that selection bound.
    /// Advanced slots / [`Access`](crate::storage::Access) / remaining RST
    /// policy or BERT codecs: [`Self::engine_mut`].
    pub fn poll(&mut self, now_ms: u64) -> Result<(), Error<T::Error>> {
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            None,
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            None,
            now_ms,
            None,
        )
    }

    /// Poll with caller-owned stateful application dispatch and optional deferral.
    ///
    /// The callback replaces site handler dispatch, after protocol/option/body
    /// validation. It borrows the complete request only during the callback.
    /// Copy work into caller-owned bounded storage, retain the supplied handle,
    /// and return [`Response::deferred`] to finish through [`Self::complete`].
    /// Deferred rows selected by [`AppBuilder::deferred`] are bounded
    /// independently of client Calls (zero by default). A handle
    /// is `None` when full, selection metadata exceeds 512 bytes, lifetime is
    /// zero, or initial Q-Block2 has more than ten selectors. An observable
    /// GET registration or selected GET/FETCH source may defer with classic or
    /// Q-Block2 preferences. Classic registrations start at NUM0; Q registration
    /// retains its selected initial ranges. Returning deferred without a handle produces 5.03.
    /// Empty ACK/retransmission is protocol reception, never durable completion.
    pub fn poll_with(
        &mut self,
        now_ms: u64,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<T::Error>> {
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            None,
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            Some(&mut dispatch),
            now_ms,
            None,
        )
    }

    /// Poll with a caller-owned durable checkpoint barrier for inbound OSCORE requests.
    ///
    /// This permanently requires checkpoint-aware polling on this App, including
    /// after [`Self::set_oscore`]. [`Self::poll`] and [`Self::poll_with`] then return
    /// [`Error::OscoreCheckpointRequired`] before I/O. To require this from bind,
    /// select [`AppBuilder::require_oscore_checkpoint`].
    ///
    /// The synchronous callback receives the current recipient replay window.
    /// Atomically persist it with the exact authenticated context/key epoch and
    /// rollback-protected freshness. Return `true` only when the commit is durable
    /// across power loss; retain any storage error in caller-owned state and
    /// return `false` to receive [`Error::OscoreCheckpointFailed`]. The callback
    /// may run twice: first for initial/dirty state before I/O, then for a newly
    /// authenticated request before body admission, Echo policy, handlers,
    /// deferral, response caching or acknowledgment. Clean idle polls and accepted
    /// cached retransmissions do not write. An authenticated malformed inner
    /// message that consumes replay state also crosses the barrier before drop.
    ///
    /// Failure retains consumed replay state in RAM, releases the received
    /// datagram and performs no effects or acceptance acknowledgment for that
    /// request. The next durable poll retries the dirty checkpoint before I/O.
    /// The original ciphertext remains a replay even after that retry; the peer
    /// must issue a fresh protected request. A crash after commit but before
    /// effects can similarly lose an operation. This is replay safety, not
    /// exactly-once application completion or a durable work queue.
    ///
    /// Before receiving with reused keys after restart, restore the latest
    /// authenticated recipient checkpoint and sender reservation. Discard prior
    /// Calls, Observe registrations, body assemblies and deferred handles; issue
    /// fresh protected requests/registrations. Client response admission and
    /// notification state are not made durable here. Shared replay-window changes
    /// from Q responses are checkpointed before subsequent polling traffic.
    /// Borrowing [`Self::oscore_mut`] or replacing the context invalidates the
    /// last commit and forces another checkpoint before I/O. No allocation or
    /// persistent backend is selected by this method.
    #[cfg(feature = "oscore")]
    pub fn poll_with_oscore_checkpoint(
        &mut self,
        now_ms: u64,
        mut checkpoint: impl FnMut(crate::oscore::ReplayCheckpoint) -> bool,
    ) -> Result<(), Error<T::Error>> {
        self.oscore_checkpoint.require();
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            Some(&mut |association, state| association.is_none() && checkpoint(state)),
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            None,
            now_ms,
            None,
        )
    }

    /// Durable request polling with caller-owned stateful application dispatch.
    ///
    /// Checkpoint ordering and failure semantics are those of
    /// [`Self::poll_with_oscore_checkpoint`]. Application dispatch and deferred
    /// handles follow [`Self::poll_with`]; the checkpoint succeeds before either
    /// can admit work or send its empty acknowledgment.
    #[cfg(feature = "oscore")]
    pub fn poll_with_oscore_checkpoint_and_dispatch(
        &mut self,
        now_ms: u64,
        mut checkpoint: impl FnMut(crate::oscore::ReplayCheckpoint) -> bool,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<T::Error>> {
        self.oscore_checkpoint.require();
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            Some(&mut |association, state| association.is_none() && checkpoint(state)),
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            Some(&mut dispatch),
            now_ms,
            None,
        )
    }

    /// Poll with a durable barrier keyed by each installed association's owner and epoch.
    /// Returning false refuses effects and acknowledgments across this App until a
    /// later successful commit. The barrier is shared; persist all associations in
    /// one bounded transaction or remove a failed association before resuming.
    /// `None` identifies the primary context when using single-context configuration.
    #[cfg(feature = "oscore")]
    pub fn poll_with_association_checkpoint(
        &mut self,
        now_ms: u64,
        mut checkpoint: impl FnMut(Option<Association>, crate::oscore::ReplayCheckpoint) -> bool,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<T::Error>> {
        self.oscore_checkpoint.require();
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            Some(&mut checkpoint),
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            Some(&mut dispatch),
            now_ms,
            None,
        )
    }

    /// Share ingress with a bounded protocol admission handler without another packet buffer.
    /// Returning true consumes that datagram before application routing. The callback
    /// borrows the RX pool only for this call; it must not retain or dispatch its bytes.
    /// Remaining application progress and one queued RX dispatch still run in this poll.
    #[cfg(feature = "oscore")]
    pub fn poll_with_association_ingress(
        &mut self,
        now_ms: u64,
        mut ingress: impl FnMut(&[u8], Endpoint) -> bool,
        mut checkpoint: impl FnMut(Option<Association>, crate::oscore::ReplayCheckpoint) -> bool,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<T::Error>> {
        self.oscore_checkpoint.require();
        poll_engine(
            &mut self.engine,
            &mut self.io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            Some(&mut checkpoint),
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            Some(&mut dispatch),
            now_ms,
            Some(&mut ingress),
        )
    }

    /// Complete admitted work through the same separate-response/Block2 sender.
    ///
    /// The payload may borrow caller memory for this call; oversized bodies are
    /// copied into bounded TX body storage. Success retires the handle, retaining
    /// any pending CON retransmissions in App. A large response retains bounded
    /// header metadata and its TX-body snapshot until matching Block2 follow-ups
    /// finish or the original deferred deadline expires. Matching method/path/
    /// queries/selection never dispatch domain work again. FETCH selection includes
    /// the original request ETag options; changing, adding or omitting a validator
    /// cannot select the retained snapshot. An admitted Q-Block2 selector list retains
    /// its complete snapshot until the original deadline, including after the
    /// last block is sent. Follow-ups can reselect individual blocks or the
    /// remainder of a payload set, using a smaller block size if needed. Recovery
    /// may include ordered equal-size selectors; overlapping blocks are sent once.
    /// The entire union is validated before sending: at most ten unique NON
    /// payloads per call. A larger union or any invalid range refuses without sending a partial
    /// selection or changing the retained snapshot: completion returns
    /// `BlockTransferError::Gap`, while a follow-up receives Bad Request.
    /// Explicit Q responses require an ETag even when the body fits one datagram.
    /// Metadata is bounded
    /// to 24 options and 512 encoded bytes; overflow fails before sending.
    /// Failure leaves the handle available for
    /// retry until its deadline; remote effects can already have happened. Never
    /// re-run durable work on a send failure. OSCORE sequences are never rewound.
    /// Once a Q snapshot is retained, retries must preserve its body, ETag, code and retained options;
    /// an error or Valid response cannot replace that attempted representation.
    /// Initial Q-Block2 deferral retains at most ten ordered, equal-size selectors.
    /// A larger selector list offers no deferred handle (5.03 if deferral is requested).
    /// Completion preflights every range before identity allocation or sending.
    /// An observable GET or selection-aware GET/FETCH registration can defer its
    /// initial response, including classic and Q-Block2 preferences. Classic
    /// initial NUM selects no separate subresource: notifications start at NUM0.
    /// With body storage enabled, an oversized successful
    /// completion requires an ETag and retains one immutable classic Block2
    /// snapshot. The first block is sized against actual datagram capacity,
    /// including OSCORE overhead, before registration or sending. Matching
    /// follow-ups carry no Observe option and do not redispatch the handler.
    /// A failed-send retry must preserve the body, ETag, code and response options.
    /// Notifications stay coalesced while this initial snapshot is retained;
    /// the final block or original deferred deadline releases that snapshot
    /// without removing its established relation. An unfinished handle's expiry
    /// still removes only its attempted relation. Body and datagram exhaustion
    /// refuse without sending and do not rewind OSCORE sequences.
    /// If the bounded Observe table is full, success is an ordinary response
    /// without Observe. Registration is installed before sending. An ambiguous
    /// transport failure retains the handle and attempted relation; retry uses
    /// a new Observe sequence and, when protected, a fresh Partial IV. Successful
    /// retries must preserve the initial Content-Format. An exact-peer empty RST
    /// for the latest transport-ambiguous attempt revokes that handle and relation;
    /// earlier failed-attempt MIDs are not retained. This reset authority expires
    /// after EXCHANGE_LIFETIME even if the deferred deadline is longer, so MID reuse
    /// cannot revoke an old handle. Preflight failures grant no new
    /// RST authority. Pending notifications
    /// remain coalesced but blocked until completion succeeds. Cancellation,
    /// expiry, or a newer same-peer/token registration or deregistration revokes
    /// the pending handle and removes only its attempted relation. A deferred
    /// registration that has not attempted output does not own an earlier live
    /// relation. Direct mutation through `engine_mut` remains an advanced caller
    /// ownership boundary: do not replace App-owned relations while a deferred
    /// handle is live. Selected FETCH and query-bearing GET Observe retain the
    /// complete bounded selection, including reassembled uploads. Nonzero initial
    /// Block2 and Q-Block2 deferred Observe remain unsupported. A requested
    /// classic block size is retained as a maximum for later notifications until
    /// re-registration. Encoding may select a smaller block. Ordinary
    /// deferred queries without Observe remain available.
    pub fn complete(
        &mut self,
        handle: DeferredReply,
        response: Response<'_, S::Token>,
        now_ms: u64,
    ) -> Result<(), Error<T::Error>> {
        complete_engine(
            &mut self.engine,
            &mut self.io,
            &mut self.ids,
            &mut self.oscore,
            &mut self.deferred,
            &mut self.dedup_closed,
            handle,
            response,
            now_ms,
        )
    }

    /// Send the current representation to server-side observers of `path`.
    /// Exact registered path segments select the route. Client subscriptions
    /// are separate, even when the peer and Token match an incoming observer.
    ///
    /// Honors notification NSTART. CON when the row
    /// [`ObserveInterest::must_confirm`]; otherwise NON. Use
    /// [`Self::notify_confirmable`] to request CON explicitly. Returns how many
    /// notifications were sent. Domain data stays in `response` — `App`
    /// does not hold it. Successful notifications must keep the initial
    /// Content-Format, including its absence. A mismatch sends 4.06 without
    /// Observe and ends that relation. Non-success responses also omit Observe
    /// and end the relation after successful transmission (RFC 7641 §4.2).
    /// With body pools enabled, oversized notifications start classic Block2;
    /// protected notification block zero uses a fresh OSCORE Partial IV. The
    /// route handler must serve the same payload and ETag on retained-body
    /// follow-ups. Changes return `BlockTransferError::IdentityMismatch` before
    /// body output or transfer advancement. Content-Format and Max-Age from
    /// that notification, including absence, are repeated on each follow-up
    /// through the final block.
    /// The body snapshot survives incomplete same-Token follow-up blocks,
    /// including send failure. Final-reply caching/completion, cancellation, replacement registration,
    /// matching notification RST or CON give-up releases it.
    /// This broadcasts the supplied response to every matching relation. For
    /// per-observer selections, use [`Self::signal`] with
    /// [`MethodRouter::observe_selected`] instead.
    /// Terminal responses use CON delivery. Pending CONs still count toward
    /// endpoint notification NSTART after the observer row has been removed.
    pub fn notify(
        &mut self,
        now_ms: u64,
        path: &[&str],
        response: Response<'static, S::Token>,
    ) -> Result<usize, Error<T::Error>> {
        let resource = self.site.observe_resource(path);
        notify_engine(
            &mut self.engine,
            &mut self.io,
            &mut self.ids,
            &mut self.oscore,
            now_ms,
            resource,
            &response,
            false,
            &self.deferred,
        )
    }

    /// Send the current representation using Confirmable notifications.
    ///
    /// Selects CON for this call without changing [`Self::notify`]'s adaptive
    /// default or bypassing per-endpoint NSTART. Returns the number sent;
    /// observers blocked by an outstanding notification are skipped, with no
    /// representation queued. Retry those observers after their hold clears.
    ///
    /// An admitted notification that fails to send retains its CON requirement
    /// when retried through [`Self::poll`] and the route's [`ObserveSource`].
    /// As with [`Self::notify`], the response itself is not retained; a retry
    /// samples the source's current representation. A successful CON send
    /// starts the normal 24-hour confirmation interval for later adaptive
    /// notifications. Matching ACK/RST and retransmission rules are unchanged.
    /// All other payload, Content-Format and terminal-response rules are those
    /// of [`Self::notify`].
    pub fn notify_confirmable(
        &mut self,
        now_ms: u64,
        path: &[&str],
        response: Response<'static, S::Token>,
    ) -> Result<usize, Error<T::Error>> {
        let resource = self.site.observe_resource(path);
        notify_engine(
            &mut self.engine,
            &mut self.io,
            &mut self.ids,
            &mut self.oscore,
            now_ms,
            resource,
            &response,
            true,
            &self.deferred,
        )
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(complete_engine_async, bound(crate::app::async_server::ProtocolIo), calls(complete_deferred_observe => complete_deferred_observe_async, complete_deferred_q => complete_deferred_q_async, send_separate => send_separate_async))]
fn complete_engine<S: AppStorage, T: DatagramIo, const DEFERRED: usize, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    deferred: &mut DeferredTable<DEFERRED, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
    handle: DeferredReply,
    response: Response<'_, S::Token>,
    now_ms: u64,
) -> Result<(), Error<T::Error>> {
    deferred.expire(engine, now_ms);
    let index = deferred
        .rows
        .iter()
        .position(|row| {
            row.is_some_and(|row| {
                row.handle == handle && !row.completed && now_ms < row.deadline_ms
            })
        })
        .ok_or(Error::DeferredUnknown)?;
    response.validate().map_err(Error::Response)?;
    if response.is_deferred() {
        return Err(Error::DeferredUnknown);
    }
    let response = response.with_fresh_piv();
    let row = deferred.rows[index].expect("matched row");
    if row.observe.is_some() {
        complete_deferred_observe(
            engine,
            io,
            ids,
            oscore,
            now_ms,
            deferred.rows[index].as_mut().expect("matched row"),
            &response,
        )?;
        if deferred.rows[index].is_some_and(|row| row.snapshot.is_some()) {
            deferred.rows[index]
                .as_mut()
                .expect("completed Observe snapshot")
                .completed = true;
        } else {
            deferred.rows[index] = None;
        }
        return Ok(());
    }
    if row.meta.q_block2.is_some()
        && row.snapshot.is_some()
        && (!response.code().is_success() || response.code() == Code::VALID)
    {
        // A partial representation may already be visible remotely. Keep
        // that snapshot retryable; an error or Valid response cannot turn
        // its old bytes into a different completed deferred response.
        return Err(Error::Block(BlockTransferError::IdentityMismatch));
    }
    let metadata = deferred_reply_metadata(&response, row.meta, now_ms).map_err(Error::Response)?;
    if row.meta.q_block2.is_some()
        && row
            .snapshot
            .is_some_and(|(_, retained)| !retained.same_header(&metadata))
    {
        return Err(Error::Block(BlockTransferError::IdentityMismatch));
    }
    let outcome = if row.meta.q_block2.is_some()
        && response.code().is_success()
        && response.code() != Code::VALID
    {
        complete_deferred_q(
            engine,
            io,
            ids,
            now_ms,
            row.meta,
            &response,
            &row.q_selectors,
            &mut deferred.rows[index].as_mut().expect("matched row").snapshot,
            metadata,
            oscore,
            dedup_closed,
        )
    } else {
        send_separate(
            engine,
            io,
            ids,
            now_ms,
            SendResponse {
                q_block2: None,
                q_request: None,
                ..row.meta
            },
            &response,
            oscore,
            dedup_closed,
            true,
        )
    };
    let body = response_body_for(engine, BlockKey::new(row.meta.token, row.meta.dest));
    if let Some(body) = body {
        let retained = if row.meta.q_block2.is_some() {
            row.snapshot.map_or(metadata, |(_, retained)| retained)
        } else {
            metadata
        };
        deferred.rows[index].as_mut().expect("matched row").snapshot = Some((body, retained));
    }
    outcome?;
    if body.is_some() {
        deferred.rows[index]
            .as_mut()
            .expect("matched row")
            .completed = true;
    } else {
        deferred.rows[index] = None;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(poll_engine_async, bound(crate::app::async_server::ProtocolIo), calls(progress_q_observe => progress_q_observe_async, refresh_progress_deadlines => async_server::refresh_progress_deadlines_async, send_retransmit_packet => async_server::send_retransmit_packet_async, send_qblock_recover => async_server::send_qblock_recover_async, dispatch_rx => dispatch_rx_async, send_notification => send_notification_async, client::pace_qblock1 => client::pace_qblock1_async), methods(recv_from => receive_retained_async), renames(progress_before_dispatch => progress_before_dispatch_async, next_qblock_recovery => next_qblock_recovery_async))]
fn poll_engine<
    Mem,
    T,
    const N: usize,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    site: &Site<N, Mem::Token>,
    ids: &mut AppIds,
    inbox: &mut client::ClientInbox<CALLS, Mem::Token>,
    lives: &mut client::ClientLives<CALLS, Mem::Token>,
    oscore: &mut oscore::Field<REQUESTS, Mem::Token>,
    oscore_checkpoint: &mut oscore::CheckpointState,
    mut checkpoint_commit: Option<&mut oscore::CheckpointCommit<'_>>,
    echo_policy: Option<EchoPolicy>,
    dedup_closed: &mut Option<DedupClosed>,
    full_responses: bool,
    deferred: &mut DeferredTable<DEFERRED, Mem::Token>,
    deferred_lifetime_ms: u64,
    mut dispatch: Option<&mut AppDispatch<'_, Mem::Token>>,
    now_ms: u64,
    mut ingress: Option<&mut AppIngress<'_>>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges + DedupSlots,
    T: DatagramIo,
{
    oscore_checkpoint.flush(oscore, &mut checkpoint_commit)?;
    #[cfg(feature = "diagnostics")]
    crate::storage::WorkMetrics::add(&mut engine.work_metrics_mut().polls, 1);
    // RX saturation or transport refusal must not skip RTO / Observe /
    // Q-Block recovery. Return the receive error after progressing work.
    // Local deadlines must progress even if receiving the next packet fails.
    deferred.expire(engine, now_ms);
    for index in 0..engine.capacities().tx_body_slots.unwrap_or(0) {
        let id = SlotId::from_index(index);
        if engine
            .tx_body_transfer(id)
            .and_then(|t| t.terminal_deadline())
            .is_some_and(|due| now_ms >= due)
        {
            // Advanced caller pins defer reclamation until their release.
            let _ = engine.release_tx_body(id);
        }
    }
    expire_request_dedup(engine, dedup_closed, now_ms);
    client::expire_client_exchanges(engine, inbox, lives, oscore, now_ms);
    let (received, recv_error) = match engine.recv_from(io) {
        Ok(id) => (id, None),
        Err(e) => (None, Some(e)),
    };
    let progress = engine.progress_before_dispatch(now_ms, |rx| !inbox.holds_datagram(rx));
    let mut replacement_error = None;
    if let Some(retransmit) = progress.retransmit() {
        match retransmit {
            Retransmit::Due(pending) => {
                let replacement = supersede_notification(
                    engine, io, site, ids, oscore, deferred, now_ms, pending,
                );
                // Staging refusal still makes the due attempt with retained bytes;
                // no unprotected fallback and no retry-budget reset.
                if engine.pending_con(pending.tx_slot()).is_some() {
                    send_retransmit_packet(engine, io, inbox, lives, oscore, pending.tx_slot())?;
                }
                replacement_error = replacement.err();
            }
            Retransmit::GiveUp(pending) => {
                client::give_up_client(engine, inbox, lives, oscore, pending.tx_slot());
                if let Some(interest) =
                    engine.reject_observe_notify(pending.message_id(), pending.endpoint())
                {
                    release_observe_response_body(engine, interest);
                }
                control_q_observe(engine, pending.endpoint(), pending.message_id(), false)?;
                engine.release_tx(pending.tx_slot())?;
            }
        }
    }

    // Max-Age expires a representation, not an observation (RFC 7641
    // section 3.3.1). Only explicit CON-wait expiry terminates an interest.
    if let Some(crate::storage::ObserveExpiry::ClientOff(id)) = progress.observe_expired()
        && let Some(interest) = engine.observe_interest(id)
    {
        let _ = engine.take_observe(interest.key());
        release_observe_response_body(engine, interest);
    }

    // A fresh receive must not bypass queued work left by an earlier failure.
    // Held replies were excluded inside the rotating scan, not just here:
    // otherwise one held row can repeatedly hide the next eligible owner.
    if let Some(rx) = progress
        .rx_ready()
        .filter(|rx| !inbox.holds_datagram(*rx))
        .or(received)
    {
        let consumed = if let Some(intercept) = ingress.as_mut() {
            let peer = engine
                .rx_endpoint(rx)
                .expect("retained endpoint")
                .physical();
            let bytes = engine.access_rx(rx)?;
            intercept(bytes.as_bytes(), peer)
        } else {
            false
        };
        if consumed {
            engine.release_rx(rx)?;
        } else {
            dispatch_rx(
                engine,
                io,
                site,
                inbox,
                lives,
                ids,
                oscore,
                oscore_checkpoint,
                &mut checkpoint_commit,
                echo_policy,
                dedup_closed,
                full_responses,
                deferred,
                deferred_lifetime_ms,
                dispatch
                    .as_mut()
                    .map(|dispatch| &mut **dispatch as &mut AppDispatch<'_, Mem::Token>),
                now_ms,
                rx,
            )?;
        }
    }

    progress_q_observe(engine, io, ids, oscore, now_ms)?;

    // Claim after ingress. Replacement registrations start with no pending
    // signal, so a reused slot cannot inherit the old relation's opportunity.
    if let Some(id) = progress
        .observe_notify()
        .filter(|id| {
            engine
                .observe_interest(*id)
                .is_some_and(|interest| !deferred.blocks_observe(interest.key()))
        })
        .and_then(|id| engine.claim_observe_notification(id, now_ms))
        && let Some(interest) = engine.observe_interest(id)
    {
        let seq = interest.seq();
        if observe_endpoint_held(engine, interest.endpoint(), now_ms)
            >= usize::from(Transmission::NSTART)
        {
            let _ = engine.restore_observe_due(id, seq);
        } else {
            let response = if interest.is_deleted() {
                Some(Response::not_found())
            } else {
                site.observe_source(interest.resource()).and_then(|source| {
                    source.snapshot(&interest, oscore.association(interest.endpoint()))
                })
            };
            if let Some(response) = response {
                if let Err(e) =
                    send_notification(engine, io, ids, oscore, now_ms, interest, &response, seq)
                {
                    let _ = engine.restore_observe_due(id, seq);
                    return Err(e);
                }
            } else {
                let _ = engine.restore_observe_due(id, seq);
            }
        }
    }

    // Process received payloads before advancing recovery or reclaiming an
    // exhausted partial body. No stale pre-dispatch opportunity is retained.
    let (recover, expired) = engine.next_qblock_recovery(now_ms);
    if let Some(key) = expired {
        client::qblock_give_up(engine, inbox, lives, oscore, key);
    }
    if let Some(recover) =
        recover.filter(|recover| oscore::route_allowed(oscore, recover.key().endpoint()))
    {
        send_qblock_recover(
            engine,
            io,
            inbox,
            lives,
            ids,
            oscore,
            now_ms,
            recover,
            dedup_closed,
        )?;
    }
    refresh_progress_deadlines(engine, io);
    client::pace_qblock1(engine, io, inbox, lives, ids, oscore, now_ms)?;
    if let Some(error) = recv_error {
        return Err(error.into());
    }
    if let Some(error) = replacement_error {
        return Err(error);
    }
    Ok(())
}

// Only an exact retained CON packet can advance a Q Observe representation.
// The body remains the owner when terminal delivery has removed its observer.
fn control_q_observe<S: Storage + BodySlots>(
    engine: &mut Engine<S>,
    peer: Endpoint,
    mid: MessageId,
    acknowledged: bool,
) -> Result<(), SlotError> {
    for index in 0..engine.capacities().tx_body_slots.unwrap_or(0) {
        let id = SlotId::from_index(index);
        let Some(mut transfer) = engine.tx_body_transfer(id) else {
            continue;
        };
        if transfer.endpoint() != peer || transfer.q_observe_pending_mid() != Some(mid) {
            continue;
        }
        if !acknowledged {
            engine.release_tx_body(id)?;
            continue;
        }
        transfer.set_q_observe_pending_mid(None);
        if !transfer.is_complete() {
            let next = transfer.num() + 1;
            if next.is_multiple_of(u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS)) {
                // Every earlier packet in the set was separately confirmed.
                let _ = transfer.ack_q_window(next);
            }
        }
        engine.storage_mut().set_tx_body_transfer(id, transfer)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(progress_q_observe_async, bound(crate::app::async_server::ProtocolIo), calls(issue_q_window => issue_q_window_async, completion_time => async_server::completion_time_async))]
fn progress_q_observe<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots + ObserveSlots,
    T: DatagramIo,
{
    for index in 0..engine.capacities().tx_body_slots.unwrap_or(0) {
        let id = SlotId::from_index(index);
        let Some(transfer) = engine.tx_body_transfer(id) else {
            continue;
        };
        if !transfer.q_observe_confirmable()
            || transfer.q_observe_pending_mid().is_some()
            || transfer.is_complete()
            || !oscore::route_allowed(oscore, transfer.endpoint())
            || observe_endpoint_held(engine, transfer.endpoint(), now_ms)
                >= usize::from(Transmission::NSTART)
        {
            continue;
        }
        let Some((code, format, age)) = transfer.notification_metadata() else {
            continue;
        };
        let mut response = Response::new(code).retain_max_age(age).with_fresh_piv();
        if let Some(format) = format {
            response = response.content_format(format);
        }
        if let Some(etag) = transfer.identity().as_slice() {
            response = response.etag(etag);
        }
        if code.is_success() && !transfer.q_observe_recovery() {
            response = response.observe(transfer.q_observe_sequence().expect("Q Observe"));
        }
        #[cfg(feature = "oscore")]
        let request = transfer.oscore_request.map(|(_, request)| request);
        #[cfg(not(feature = "oscore"))]
        let request = oscore::no_request();
        let mid = ids.next_for(engine, io, now_ms)?;
        let meta = SendResponse {
            dest: transfer.endpoint(),
            token: transfer.token(),
            mid,
            ty: Type::Confirmable,
            no_response: NoResponse::DEFAULT,
            block2: None,
            q_block2: None,
            q_request: None,
            block1: None,
            oscore: request,
            request: Code::GET,
        };
        issue_q_window(
            engine,
            io,
            ids,
            now_ms,
            meta,
            &response,
            Type::Confirmable,
            id,
            oscore,
            &mut None,
        )?;
        let now_ms = completion_time(io, now_ms);
        let _ = engine.record_observe_notify(
            ObserveKey::new(transfer.token(), transfer.endpoint()),
            now_ms,
            mid,
            true,
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum InboundBody {
    None,
    Continue,
    QWait(Option<BlockValue>),
    Refused(Code),
    Complete(SlotId),
}

fn assemble_inbound_body<Mem>(
    engine: &mut Engine<Mem>,
    rx: SlotId,
    now_ms: u64,
    parsed: &ParsedMessage<'_, Mem::Token>,
    oscore_request: oscore::Request,
) -> InboundBody
where
    Mem: Storage + DatagramSlots + BodySlots,
{
    #[cfg(not(feature = "oscore"))]
    let _ = oscore_request;
    // Classify from the first decode. Plain GET must not enter apply_*
    // (each stacks profile-sized Block IO scratch, then MissingBlock).
    if parsed.block1().is_some() {
        return match engine.apply_block1_rx(rx) {
            Ok(progress) if progress.complete() => InboundBody::Complete(progress.id()),
            Ok(_) => InboundBody::Continue,
            // Retransmit of an already-acked NUM (RFC 7959): replay 2.31, not 4.08.
            Err(BlockTransferError::Overlap | BlockTransferError::AlreadyComplete) => {
                InboundBody::Continue
            }
            Err(BlockTransferError::MissingBlock) => InboundBody::None,
            Err(BlockTransferError::NoBodyPools) => InboundBody::Refused(Code::BAD_OPTION),
            Err(BlockTransferError::Overflow) => {
                InboundBody::Refused(Code::REQUEST_ENTITY_TOO_LARGE)
            }
            Err(_) => InboundBody::Refused(Code::REQUEST_ENTITY_INCOMPLETE),
        };
    }
    if parsed.q_block1().is_some() {
        return match engine.apply_q_block1_rx(rx) {
            Ok(progress) if progress.complete() => {
                let _ = engine.note_q_receive(progress.id(), now_ms);
                InboundBody::Complete(progress.id())
            }
            Ok(progress) => {
                #[cfg(feature = "oscore")]
                if let Some(request) = oscore_request {
                    engine.remember_qblock1_request(progress.id(), parsed.token(), request);
                }
                let _ = engine.note_q_receive(progress.id(), now_ms);
                qblock1_wait(engine, rx, parsed, false)
            }
            Err(BlockTransferError::Duplicate) => {
                #[cfg(feature = "oscore")]
                if let Some(request) = oscore_request {
                    for index in 0..engine.capacities().rx_body_slots.unwrap_or(0) {
                        let id = SlotId::from_index(index);
                        if engine.rx_body_transfer(id).is_some_and(|transfer| {
                            transfer.role() == BlockRole::IncomingQBlock1
                                && Some(transfer.endpoint()) == engine.rx_endpoint(rx)
                                && transfer.identity().as_slice() == parsed.request_tag().next()
                        }) {
                            engine.remember_qblock1_request(id, parsed.token(), request);
                            break;
                        }
                    }
                }
                qblock1_wait(engine, rx, parsed, true)
            }
            Err(BlockTransferError::OutsideWindow) if parsed.ty() == Type::NonConfirmable => {
                match engine.validated_q1_future_rx(rx) {
                    Ok(id) => {
                        #[cfg(feature = "oscore")]
                        if let Some(request) = oscore_request {
                            engine.remember_qblock1_request(id, parsed.token(), request);
                        }
                        // Future traffic is not body progress and cannot reset
                        // an armed recovery's finite retry budget.
                        if engine
                            .rx_body_transfer(id)
                            .is_some_and(|t| t.q_receive().is_none())
                        {
                            let _ = engine.note_q_receive(id, now_ms);
                        }
                        InboundBody::QWait(None)
                    }
                    Err(BlockTransferError::Overflow) => {
                        InboundBody::Refused(Code::REQUEST_ENTITY_TOO_LARGE)
                    }
                    Err(_) => InboundBody::Refused(Code::REQUEST_ENTITY_INCOMPLETE),
                }
            }
            Err(BlockTransferError::MissingBlock) => InboundBody::None,
            Err(BlockTransferError::NoBodyPools) => InboundBody::Refused(Code::BAD_OPTION),
            Err(BlockTransferError::Overflow) => {
                InboundBody::Refused(Code::REQUEST_ENTITY_TOO_LARGE)
            }
            Err(_) => InboundBody::Refused(Code::REQUEST_ENTITY_INCOMPLETE),
        };
    }
    InboundBody::None
}

// An empty ACK acknowledges a CON payload without promising a complete
// body. NON Continue is only due after a whole MAX_PAYLOADS_SET is present.
// A duplicate repeats that Continue only for the block that completed the set.
fn qblock1_wait<Mem: Storage + DatagramSlots + BodySlots>(
    engine: &Engine<Mem>,
    rx: SlotId,
    parsed: &ParsedMessage<'_, Mem::Token>,
    duplicate: bool,
) -> InboundBody {
    let next = if parsed.ty() == Type::NonConfirmable {
        let peer = engine.rx_endpoint(rx);
        let tag = parsed.request_tag().next();
        let request_num = parsed
            .q_block1()
            .and_then(Result::ok)
            .map(|block| block.num());
        (0..engine.capacities().rx_body_slots.unwrap_or(0)).find_map(|index| {
            let transfer = engine.rx_body_transfer(SlotId::from_index(index))?;
            if transfer.role() != BlockRole::IncomingQBlock1
                || Some(transfer.endpoint()) != peer
                || transfer.identity().as_slice() != tag
            {
                return None;
            }
            if duplicate {
                let ack = request_num.and_then(|num| transfer.q_continue_for(num))?;
                return BlockValue::new(ack, true, transfer.szx()).ok();
            }
            if transfer.window_base() == 0 || transfer.window_mask() != 0 {
                return None;
            }
            BlockValue::new(transfer.window_base() - 1, true, transfer.szx()).ok()
        })
    } else {
        None
    };
    InboundBody::QWait(next)
}

/// Write a successfully unprotected Inner over the RX slot.
///
/// Block-wise assembly reads that slot (RFC 8613 §4.1.3.4.1). Encode or
/// `write_rx` failure is an error — not `Ok(())` that drops the Inner.
#[cfg(feature = "oscore")]
pub(crate) fn write_unprotected_rx<Mem, E>(
    engine: &mut Engine<Mem>,
    rx: SlotId,
    inner: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
) -> Result<(), Error<E>>
where
    Mem: Storage + DatagramSlots,
{
    let mut inner_wire = Mem::RxScratch::default();
    let n = match inner.encode(inner_wire.as_mut()) {
        Ok(n) => n,
        Err(e) => {
            let _ = engine.release_rx(rx);
            return Err(Error::Message(SlotMessageError::Encode(e)));
        }
    };
    if let Err(e) = engine.write_rx(rx, &inner_wire.as_ref()[..n], peer) {
        let _ = engine.release_rx(rx);
        return Err(Error::Slot(e));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_qblock_recover_io_async, bound(crate::app::async_server::ProtocolIo), calls(send_response => send_response_async, client::recover_qblock2 => client::recover_qblock2_async, oscore::send_qblock1_recover => oscore::send_qblock1_recover_async), methods(send_tx => send_owned_packet_async))]
fn send_qblock_recover<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut client::ClientInbox<CALLS, Mem::Token>,
    lives: &mut client::ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    recover: QBlockRecover<Mem::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots + Exchanges + ObserveSlots,
    T: DatagramIo,
{
    if recover.role() == BlockRole::IncomingQBlock2
        && client::recover_qblock2(engine, io, inbox, lives, ids, oscore, now_ms, recover)?
    {
        return Ok(());
    }
    // Protected recovery requires an authenticated request binding. Advanced
    // body state without one must never fall back to unprotected traffic.
    #[cfg(feature = "oscore")]
    if oscore::is_active(oscore) {
        return oscore::send_qblock1_recover(engine, io, ids, oscore, now_ms, recover);
    }
    match recover.role() {
        BlockRole::IncomingQBlock2 => {
            let mid = ids.next_for(engine, io, now_ms)?;
            let Some(tx) = engine.acquire_tx() else {
                return Err(Error::Saturated);
            };
            if let Err(e) =
                engine.encode_q_block2_recover_tx(recover, tx, Type::NonConfirmable, Code::GET, mid)
            {
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
                mid: ids.next_for(engine, io, now_ms)?,
                token: recover.key().token(),
                no_response: NoResponse::DEFAULT,
                block2: None,
                q_block2: None,
                q_request: None,
                block1: None,
                oscore: oscore::no_request(),
                request: Code::GET,
            };
            send_response(
                engine,
                io,
                ids,
                meta,
                &Response::missing_blocks(nums.into_iter().take(n)),
                now_ms,
                oscore,
                dedup_closed,
            )
        }
        _ => Ok(()),
    }
}

/// 4.02 before [`Site::dispatch`]: unknown critical, OSCORE with no
/// attached context, or a malformed Block / Q-Block2 value.
fn bad_option_request<K: OwnedToken, const REQUESTS: usize>(
    parsed: &ParsedMessage<'_, K>,
    oscore: &oscore::Field<REQUESTS, K>,
) -> bool {
    let mut tags = parsed.request_tag();
    let first = tags.next();
    let bad_tag = first.is_some_and(|tag| tag.len() > 8) || tags.next().is_some();
    bad_tag
        || (parsed.code() == Code::FETCH
            && (matches!(parsed.content_format(), Some(Err(_)))
                || parsed
                    .get_options(crate::message::OptionNumber::CONTENT_FORMAT)
                    .nth(1)
                    .is_some()))
        || parsed.unknown_critical().is_some()
        || (parsed.oscore().is_some() && !oscore::is_active(oscore))
        || matches!(parsed.block2(), Some(Err(_)))
        || parsed
            .q_block2()
            .any(|value| value.map_or(true, |block| block.is_bert()))
        || mixes_block_and_qblock(parsed)
        || parsed
            .get_options(crate::message::OptionNumber::Q_BLOCK1)
            .nth(1)
            .is_some()
        || (parsed.q_block2().nth(1).is_some()
            && (parsed.observe().is_some()
                || parsed.q_block2().any(|q| {
                    q.is_ok_and(|q| {
                        q.more()
                            && q.num() % u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS) == 0
                    })
                })))
        || matches!(parsed.block1(), Some(Err(_)))
}

fn mixes_block_and_qblock(parsed: &ParsedMessage<'_, impl OwnedToken>) -> bool {
    // RFC 9177 §4.1. Same-side Block and Q-Block are 4.02. A mix across
    // Inner and Outer is not visible on one of these messages.
    (parsed.block1().is_some() || parsed.block2().is_some())
        && (parsed.q_block1().is_some() || parsed.q_block2().next().is_some())
}

fn valid_q_selections(parsed: &ParsedMessage<'_, impl OwnedToken>) -> bool {
    let mut previous: Option<BlockValue> = None;
    for value in parsed.q_block2() {
        let Ok(value) = value else {
            return false;
        };
        if previous.is_some_and(|last| value.num() <= last.num() || value.szx() != last.szx()) {
            return false;
        }
        previous = Some(value);
    }
    true
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(dispatch_rx_io_async, bound(crate::app::async_server::ProtocolIo), calls(send_token_capacity_refusal => send_token_capacity_refusal_async, oscore::send_token_capacity_refusal => oscore::send_token_capacity_refusal_async, admission::send_refusal => admission::send_refusal_async, issue_q_observe_followup => issue_q_observe_followup_async, send_separate => send_separate_async, send_response => send_response_async, issue_classic => issue_classic_async, issue_deferred_q => issue_deferred_q_async, replay_con_request => replay_con_request_async, oscore_inbound_error => oscore_inbound_error_async, send_empty_ack => send_empty_ack_async, send_empty_rst => send_empty_rst_async, send_empty => send_empty_async, client::complete_client => client::complete_client_async, client::continue_qblock1_ack => client::continue_qblock1_ack_async))]
fn dispatch_rx<
    Mem,
    T,
    const N: usize,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    site: &Site<N, Mem::Token>,
    inbox: &mut client::ClientInbox<CALLS, Mem::Token>,
    lives: &mut client::ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, Mem::Token>,
    oscore_checkpoint: &mut oscore::CheckpointState,
    checkpoint_commit: &mut Option<&mut oscore::CheckpointCommit<'_>>,
    echo_policy: Option<EchoPolicy>,
    dedup_closed: &mut Option<DedupClosed>,
    full_responses: bool,
    deferred: &mut DeferredTable<DEFERRED, Mem::Token>,
    deferred_lifetime_ms: u64,
    dispatch: Option<&mut AppDispatch<'_, Mem::Token>>,
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

    let mut scratch = Mem::RxScratch::default();
    let n = match copy_rx(engine, rx, scratch.as_mut()) {
        Ok(n) => n,
        Err(e) => {
            let _ = engine.release_rx(rx);
            return Err(e);
        }
    };

    let parsed = match crate::message::decode_with_token::<Mem::Token>(&scratch.as_ref()[..n]) {
        Ok(parsed) => parsed,
        Err(crate::message::TokenDecodeError::Capacity(_)) => {
            // The shared decoder validated the complete wire syntax before the
            // owned token copy failed. Refuse while its original bytes remain
            // borrowed from this RX scratch; never truncate the identity.
            let request = crate::message::extended::decode(&scratch.as_ref()[..n])
                .expect("validated extended token datagram");
            let protected = request
                .options()
                .any(|option| option.number() == crate::message::OptionNumber::OSCORE);
            let outcome = if protected {
                #[cfg(feature = "oscore")]
                {
                    oscore::send_token_capacity_refusal(
                        engine,
                        io,
                        ids,
                        now_ms,
                        peer,
                        &scratch.as_ref()[..n],
                        oscore,
                        oscore_checkpoint,
                        checkpoint_commit,
                        dedup_closed,
                    )
                }
                #[cfg(not(feature = "oscore"))]
                {
                    Ok(())
                }
            } else if oscore::is_active(oscore) {
                // Attached contexts and association-only policy reject application
                // traffic without OSCORE before any capacity response is emitted.
                Ok(())
            } else {
                send_token_capacity_refusal(engine, io, ids, request, peer, now_ms)
            };
            let _ = engine.release_rx(rx);
            return outcome;
        }
        Err(crate::message::TokenDecodeError::Malformed(_)) => {
            Metrics::inc(&mut engine.metrics_mut().rx_error);
            let _ = engine.release_rx(rx);
            return Ok(());
        }
    };

    let peer = if parsed.is_empty_ack_or_rst() {
        match control_route(engine, deferred, peer, parsed.message_id(), now_ms) {
            Some(route) => route,
            None => {
                let _ = engine.release_rx(rx);
                return Ok(());
            }
        }
    } else if !parsed.is_empty() {
        #[cfg(feature = "oscore")]
        {
            match oscore.inbound_route(peer, &parsed) {
                Ok(route) => route,
                Err(_) => {
                    let _ = engine.release_rx(rx);
                    return Ok(());
                }
            }
        }
        #[cfg(not(feature = "oscore"))]
        {
            peer
        }
    } else {
        peer
    };
    engine.set_rx_endpoint(rx, peer)?;

    if parsed.is_empty_ack_or_rst() {
        if parsed.is_empty_rst() {
            if let Err(error) =
                deferred.reject_observe_attempt(engine, parsed.message_id(), peer, now_ms)
            {
                let _ = engine.release_rx(rx);
                return Err(error.into());
            }
            client::complete_client_rst(engine, inbox, lives, oscore, parsed.message_id(), peer);
        }
        if let Some(tx) = engine.match_empty_ack_rst(&parsed, peer) {
            let _ = engine.release_tx(tx);
            if let Err(error) =
                control_q_observe(engine, peer, parsed.message_id(), parsed.is_empty_ack())
            {
                let _ = engine.release_rx(rx);
                return Err(error.into());
            }
            if parsed.is_empty_ack()
                && let Err(error) = client::continue_qblock1_ack(
                    engine,
                    io,
                    inbox,
                    lives,
                    ids,
                    oscore,
                    now_ms,
                    parsed.message_id(),
                    peer,
                )
            {
                let _ = engine.release_rx(rx);
                return Err(error);
            }
        }
        if parsed.is_empty_ack() {
            let _ = engine.ack_observe_con(parsed.message_id(), peer);
        } else {
            // Delayed RST: retire the exact current packet before detaching its
            // relation. A pinned packet refuses with both owners intact.
            if let Some(interest) = engine.superseded_observe_notify(parsed.message_id(), peer)
                && let Some(mid) = interest.notify_hold().and_then(|hold| hold.con_mid())
                && let Some(tx) = engine.lookup_owned_con(mid, peer)
                && let Err(error) = engine.release_tx(tx)
            {
                let _ = engine.release_rx(rx);
                return Err(error.into());
            }
            // RST identifies a server notification by MID and endpoint. Its
            // request reference belongs to the removed server interest;
            // outgoing client bindings may independently use the same Token.
            if let Some(interest) = engine.reject_observe_notify(parsed.message_id(), peer) {
                release_observe_response_body(engine, interest);
            }
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

    // Keep accepted pending work independent of expiring ACK-cache rows.
    // Check the authenticated request's retained outer identity before OSCORE
    // replay rejection; duplicates never invoke application work again.
    if parsed.code().is_request()
        && deferred
            .rows
            .iter()
            .flatten()
            .any(|row| row.meta.dest == peer && row.meta.mid == parsed.message_id())
    {
        let outcome = if parsed.ty() == Type::Confirmable {
            send_empty_ack(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    }

    // Dedup keys the *outer* Message ID + peer. OSCORE CON retransmit
    // must replay the cached protected ACK before unprotect, or the
    // replay window consumes the Partial IV and the client never ACKs.
    if (parsed.code().is_request() || parsed.code().is_response())
        && parsed.ty() == Type::Confirmable
    {
        match replay_con_request(engine, io, peer, &parsed, now_ms, dedup_closed) {
            Replay::Hit(outcome) => {
                let _ = engine.release_rx(rx);
                return outcome;
            }
            Replay::Miss => {}
        }
    }

    // Match the Outer routing identity before OSCORE consumes a binding or
    // advances recipient replay/notification state. Inner authentication cannot
    // replace the Call's endpoint and piggybacked-ACK matching rules.
    if oscore::is_active(oscore)
        && parsed.oscore().is_some()
        && !parsed.code().is_request()
        && !client::has_response_target(engine, &parsed, peer)
    {
        // RFC 7252 sections 4.2 and 5.3.2: a canceled/unmatched separate CON
        // response must be rejected at the message layer. Silently dropping it
        // strands the sender's NSTART budget until retransmission expiry.
        let outcome = if parsed.ty() == Type::Confirmable {
            send_empty_rst(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    }

    // RFC 9177 §4.1: Block and Q-Block on the Outer side are 4.02 before
    // the ciphertext is treated as a body. Inner options are still sealed.
    if parsed.oscore().is_some() && mixes_block_and_qblock(&parsed) {
        // Plaintext 4.02: the ciphertext is not a request this response can
        // protect, and the outer options are already not one body.
        let outcome = send_empty(
            engine,
            io,
            peer,
            Message::new(Type::Acknowledgement, Code::BAD_OPTION, parsed.message_id())
                .with_token(parsed.token()),
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }

    let mut inner_scratch = Mem::RxScratch::default();
    #[cfg(feature = "oscore")]
    let before = oscore
        .context(peer.association())
        .map(|ctx| ctx.replay_checkpoint());
    let opened = oscore::inbound(oscore, peer, &parsed, inner_scratch.as_mut());
    #[cfg(feature = "oscore")]
    if before
        != oscore
            .context(peer.association())
            .map(|ctx| ctx.replay_checkpoint())
    {
        oscore_checkpoint.invalidate();
    }
    if let Err(error) = oscore_checkpoint.flush(oscore, checkpoint_commit) {
        let _ = engine.release_rx(rx);
        return Err(error);
    }
    let opened = match opened {
        Ok(opened) => opened,
        Err(e) => {
            let outcome =
                oscore_inbound_error(engine, io, ids, &parsed, peer, now_ms, e, dedup_closed);
            let _ = engine.release_rx(rx);
            return outcome;
        }
    };
    let (parsed, oscore_req, response_state) = match opened {
        Some((inner, req, response_state)) => {
            #[cfg(feature = "oscore")]
            {
                // Inner Block-wise (RFC 8613 §4.1.3.4.1): assembly reads
                // the RX slot. Replace the outer OSCORE datagram with the
                // opened Inner so Block1/Block2 see Class E options.
                write_unprotected_rx(engine, rx, &inner, peer)?;
                (inner, Some(req), response_state)
            }
            #[cfg(not(feature = "oscore"))]
            {
                (inner, req, response_state)
            }
        }
        None => (parsed, oscore::no_request(), oscore::ResponseState::None),
    };

    if !parsed.code().is_request() {
        let cache_ack = parsed.ty() == Type::Confirmable
            && parsed.unknown_critical().is_none()
            && lives.observe_format_admissible(
                Call::new(parsed.token(), peer),
                &parsed,
                now_ms,
                oscore::is_active(oscore),
            )
            && client::has_response_target(engine, &parsed, peer);
        #[cfg(feature = "oscore")]
        let before = oscore
            .context(peer.association())
            .map(|ctx| ctx.replay_checkpoint());
        let outcome = client::complete_client(
            engine,
            io,
            inbox,
            lives,
            ids,
            oscore,
            response_state,
            full_responses,
            now_ms,
            peer,
            &parsed,
            rx,
        );
        #[cfg(feature = "oscore")]
        if before
            != oscore
                .context(peer.association())
                .map(|ctx| ctx.replay_checkpoint())
        {
            oscore_checkpoint.invalidate();
        }
        // Only a successfully admitted response has earned an ACK replay.
        // A failed initial ACK must retry admission, not skip delivery.
        if outcome.is_ok() && cache_ack {
            remember_empty_ack(
                engine,
                peer,
                parsed.message_id(),
                now_ms,
                parsed.code(),
                dedup_closed,
            );
        }
        return outcome;
    }

    let no_response = NoResponse::from_message(&parsed).unwrap_or(NoResponse::DEFAULT);
    // Origin: Proxy-Uri / Proxy-Scheme MUST 5.05 (RFC 7252 §5.10.2).
    // Before the site so a missing Uri-Path is not 4.04.
    if parsed.proxy_uri().is_some() || parsed.proxy_scheme().is_some() {
        let meta = SendResponse {
            dest: peer,
            ty: parsed.ty(),
            mid: parsed.message_id(),
            token: parsed.token(),
            no_response,
            block2: None,
            q_block2: None,
            q_request: None,
            block1: None,
            oscore: oscore_req,
            request: parsed.code(),
        };
        let outcome = send_response(
            engine,
            io,
            ids,
            meta,
            &Response::problem(Code::PROXYING_NOT_SUPPORTED).title("Proxying Not Supported"),
            now_ms,
            oscore,
            dedup_closed,
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }
    // Known-stack critical check and OSCORE-without-context run here,
    // before Site::dispatch. `knowledge/rfcs/rfc7252.txt` §5.4.1;
    // `knowledge/rfcs/rfc8613.txt` §8.2 (option 9 with no context).
    if bad_option_request(&parsed, oscore) {
        let meta = SendResponse {
            dest: peer,
            ty: parsed.ty(),
            mid: parsed.message_id(),
            token: parsed.token(),
            no_response,
            block2: None,
            q_block2: None,
            q_request: None,
            block1: None,
            oscore: oscore_req,
            request: parsed.code(),
        };
        let mut response = Response::problem(Code::BAD_OPTION).title("Bad Option");
        if let Some(number) = parsed.unknown_critical() {
            // A single u16 option fits the fixed problem payload. Preserve a
            // valid 4.02 fallback if the compact encoder's limits ever change.
            response = response
                .unprocessed_options(&[u64::from(number.get())])
                .unwrap_or(response);
        }
        let outcome = send_response(
            engine,
            io,
            ids,
            meta,
            &response,
            now_ms,
            oscore,
            dedup_closed,
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }
    let block2 = parsed.block2().and_then(Result::ok);
    let q_block2 = parsed.q_block2().next().and_then(Result::ok);
    let block1 = parsed.block1().and_then(Result::ok).map(|value| BlockOpt {
        value,
        q_block: false,
        size2: None,
    });
    let meta = SendResponse {
        dest: peer,
        ty: parsed.ty(),
        mid: parsed.message_id(),
        token: parsed.token(),
        no_response,
        block2,
        q_block2,
        q_request: q_block2.map(|_| parsed),
        block1,
        oscore: oscore_req,
        request: parsed.code(),
    };

    // RFC 9177 section 4.3 requires 4.00 for missing Request-Tag or Size1.
    // Reject before admission/dispatch, preserving any existing partial body.
    let missing_q1_metadata = parsed.q_block1().is_some()
        && (parsed.request_tag().next().is_none() || !matches!(parsed.size1(), Some(Ok(_))));
    // RFC 8132 section 2.3.1 requires FETCH Content-Format, even for an
    // empty selection. Do not allocate partial bodies or call a handler.
    let missing_fetch_format = parsed.code() == Code::FETCH && parsed.content_format().is_none();
    if missing_q1_metadata || missing_fetch_format || !valid_q_selections(&parsed) {
        let outcome = send_response(
            engine,
            io,
            ids,
            SendResponse {
                block2: None,
                q_block2: None,
                q_request: None,
                ..meta
            },
            &Response::bad_request(),
            now_ms,
            oscore,
            dedup_closed,
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }

    if admission::refuse_new_request(engine, &parsed, peer) {
        let outcome = admission::send_refusal(
            engine,
            io,
            ids,
            now_ms,
            SendResponse {
                block1: None,
                block2: None,
                q_block2: None,
                q_request: None,
                ..meta
            },
            oscore,
            dedup_closed,
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }

    if let Some(policy) = echo_policy {
        let decision = policy(EchoCheck {
            echo: Echo::from_message(&parsed),
            peer,
            now_ms,
            oscore_protected: oscore::is_active(oscore),
        });
        match decision {
            EchoDecision::Accept => {}
            EchoDecision::Challenge(_) | EchoDecision::Reject => {
                let outcome = send_response(
                    engine,
                    io,
                    ids,
                    meta,
                    &unauthorized_echo(decision),
                    now_ms,
                    oscore,
                    dedup_closed,
                );
                let _ = engine.release_rx(rx);
                return outcome;
            }
        }
    }

    let assembled = assemble_inbound_body(engine, rx, now_ms, &parsed, oscore_req);
    let body_owner = async_server::BodyOwner::incoming(
        engine,
        match assembled {
            InboundBody::Complete(id) => Some(id),
            _ => None,
        },
    );
    let engine = &mut *body_owner.engine;

    match assembled {
        InboundBody::Continue => {
            let outcome = send_response(
                engine,
                io,
                ids,
                SendResponse {
                    block2: None,
                    q_block2: None,
                    q_request: None,
                    ..meta
                },
                &Response::new(Code::CONTINUE),
                now_ms,
                oscore,
                dedup_closed,
            );
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::QWait(continue_block) => {
            let outcome = if let Some(value) = continue_block {
                send_response(
                    engine,
                    io,
                    ids,
                    SendResponse {
                        block2: None,
                        q_block2: None,
                        q_request: None,
                        block1: Some(BlockOpt {
                            value,
                            q_block: true,
                            size2: None,
                        }),
                        ..meta
                    },
                    &Response::new(Code::CONTINUE),
                    now_ms,
                    oscore,
                    dedup_closed,
                )
            } else if meta.ty == Type::Confirmable {
                // Body admission and authenticated replay state already advanced.
                // Retain the ACK before I/O so a failed send can be retried by
                // the peer without reopening the protected request.
                remember_empty_ack(
                    engine,
                    meta.dest,
                    meta.mid,
                    now_ms,
                    meta.request,
                    dedup_closed,
                );
                send_empty_ack(engine, io, meta.dest, meta.mid)
            } else {
                Ok(())
            };
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::Refused(code) => {
            // Apply errors (gap, SZX mismatch, overflow, …).
            // Classic Overlap / AlreadyComplete replay 2.31 above.
            // Window holes use `send_qblock_recover` → `Response::missing_blocks`.
            let outcome = send_response(
                engine,
                io,
                ids,
                meta,
                &Response::problem(code).title(match code {
                    Code::REQUEST_ENTITY_INCOMPLETE => "Request Entity Incomplete",
                    Code::REQUEST_ENTITY_TOO_LARGE => "Request Entity Too Large",
                    _ => "Block Transfer Rejected",
                }),
                now_ms,
                oscore,
                dedup_closed,
            );
            let _ = engine.release_rx(rx);
            return outcome;
        }
        InboundBody::None | InboundBody::Complete(_) => {}
    }

    // Follow-up blocks belong to the retained selection, not merely its Token.
    if (block2.is_some() || q_block2.is_some()) && parsed.observe().is_none() {
        let selection = engine
            .lookup_observe(ObserveKey::new(parsed.token(), peer))
            .and_then(|id| engine.observe_interest(id))
            .and_then(|row| row.selection().copied());
        if selection.is_some_and(|expected| {
            DeferredSelection::from_parts(
                &parsed,
                match assembled {
                    InboundBody::Complete(id) => {
                        engine.rx_body_payload(id).unwrap_or(parsed.payload())
                    }
                    _ => parsed.payload(),
                },
            )
            .ok()
            .map(DeferredSelection::stored)
                != Some(expected)
        }) {
            let outcome = send_response(
                engine,
                io,
                ids,
                SendResponse {
                    block2: None,
                    q_block2: None,
                    q_request: None,
                    ..meta
                },
                &Response::bad_request(),
                now_ms,
                oscore,
                dedup_closed,
            );
            if let InboundBody::Complete(id) = assembled {
                let _ = engine.release_rx_body(id);
            }
            let _ = engine.release_rx(rx);
            return outcome;
        }
    }

    // A terminal notification has no observer row, but its body still owns the
    // exact selection and response metadata until the last requested block.
    if (meta.block2.is_some() || meta.q_block2.is_some())
        && parsed.observe().is_none()
        && let Some(body) = response_body_for(engine, BlockKey::new(meta.token, peer))
    {
        let transfer = engine.tx_body_transfer(body).expect("located body");
        if let (Some(expected), Some((code, format, age))) = (
            transfer.observe_selection(),
            transfer.notification_metadata(),
        ) {
            let actual = DeferredSelection::from_parts(
                &parsed,
                match assembled {
                    InboundBody::Complete(id) => {
                        engine.rx_body_payload(id).unwrap_or(parsed.payload())
                    }
                    _ => parsed.payload(),
                },
            )
            .ok()
            .map(DeferredSelection::stored);
            let outcome = if actual != Some(expected) {
                send_response(
                    engine,
                    io,
                    ids,
                    SendResponse {
                        block2: None,
                        q_block2: None,
                        q_request: None,
                        ..meta
                    },
                    &Response::bad_request(),
                    now_ms,
                    oscore,
                    dedup_closed,
                )
            } else {
                let response = Response::new(code)
                    .retain_format(format)
                    .retain_max_age(age);
                let response = match transfer.identity().as_slice() {
                    Some(tag) => response.etag(tag),
                    None => response,
                };
                let ty = if meta.ty == Type::Confirmable {
                    Type::Acknowledgement
                } else {
                    Type::NonConfirmable
                };
                let mid = if ty == Type::Acknowledgement {
                    meta.mid
                } else {
                    ids.next_for(engine, io, now_ms)?
                };
                if meta.q_block2.is_some() && transfer.q_observe_sequence().is_some() {
                    issue_q_observe_followup(
                        engine,
                        io,
                        ids,
                        now_ms,
                        SendResponse { mid, ..meta },
                        &response,
                        ty,
                        body,
                        oscore,
                        dedup_closed,
                    )
                } else {
                    issue_classic(
                        engine,
                        io,
                        meta,
                        &response,
                        ty,
                        mid,
                        body,
                        None,
                        true,
                        Some((now_ms, dedup_closed)),
                        oscore,
                    )
                }
            };
            if outcome.is_ok()
                && engine.tx_body_transfer(body).is_some_and(|current| {
                    current.q_observe_last_mid() != transfer.q_observe_last_mid()
                })
            {
                record_q_observe_sent(engine, BlockKey::new(meta.token, peer), now_ms);
            }
            if let InboundBody::Complete(id) = assembled {
                let _ = engine.release_rx_body(id);
            }
            let _ = engine.release_rx(rx);
            return outcome;
        }
    }

    // Deferred responses own a stable bounded TX snapshot. Authenticated
    // classic or bounded Q-Block2 follow-ups select it without re-running work.
    if (meta.block2.is_some() || meta.q_block2.is_some())
        && let Some(index) = deferred.rows.iter().position(|row| {
            row.is_some_and(|row| {
                row.completed
                    && row.meta.token == meta.token
                    && row.meta.dest == peer
                    && (row.observe.is_none() || parsed.observe().is_none())
            })
        })
    {
        let row = deferred.rows[index].expect("matched snapshot");
        let (body, retained) = row.snapshot.expect("completed snapshot");
        let outcome = 'reply: {
            if DeferredSelection::from_parts(
                &parsed,
                match assembled {
                    InboundBody::Complete(id) => {
                        engine.rx_body_payload(id).unwrap_or(parsed.payload())
                    }
                    _ => parsed.payload(),
                },
            )
            .ok()
                != Some(row.selection)
                || (parsed.code() != Code::FETCH && !parsed.payload().is_empty())
                || row.meta.q_block2.is_some() != meta.q_block2.is_some()
                || (row.observe.is_some()
                    && meta
                        .block2
                        .is_some_and(|requested| engine.requested_block2(body, requested).is_err()))
            {
                send_response(
                    engine,
                    io,
                    ids,
                    meta,
                    &Response::bad_request(),
                    now_ms,
                    oscore,
                    dedup_closed,
                )
            } else if meta.q_block2.is_some() {
                let response = retained.response();
                let ty = if meta.ty == Type::Confirmable {
                    Type::Acknowledgement
                } else {
                    Type::NonConfirmable
                };
                let meta = if ty == Type::NonConfirmable {
                    SendResponse {
                        mid: match ids.next_for(engine, io, now_ms) {
                            Ok(mid) => mid,
                            Err(e) => break 'reply Err(e),
                        },
                        ..meta
                    }
                } else {
                    meta
                };
                match issue_deferred_q(
                    engine,
                    io,
                    ids,
                    now_ms,
                    meta,
                    &response,
                    ty,
                    body,
                    None,
                    oscore,
                    dedup_closed,
                ) {
                    Err(Error::Block(BlockTransferError::Gap)) => send_response(
                        engine,
                        io,
                        ids,
                        meta,
                        &Response::bad_request(),
                        now_ms,
                        oscore,
                        dedup_closed,
                    ),
                    other => other,
                }
            } else {
                let response = retained.response();
                let ty = if meta.ty == Type::Confirmable {
                    Type::Acknowledgement
                } else {
                    Type::NonConfirmable
                };
                let mid = if ty == Type::Acknowledgement {
                    meta.mid
                } else {
                    match ids.next_for(engine, io, now_ms) {
                        Ok(mid) => mid,
                        Err(e) => break 'reply Err(e),
                    }
                };
                issue_classic(
                    engine,
                    io,
                    meta,
                    &response,
                    ty,
                    mid,
                    body,
                    None,
                    true,
                    Some((now_ms, dedup_closed)),
                    oscore,
                )
            }
        };
        if let InboundBody::Complete(id) = assembled {
            let _ = engine.release_rx_body(id);
        }
        let _ = engine.release_rx(rx);
        deferred.expire(engine, now_ms);
        return outcome;
    }

    // Continuations may use control headroom; their handler still needs capacity.
    if admission::refuse_dispatch(engine) {
        // Authenticated cancellation can release its relation under pressure,
        // even when the accompanying fresh representation cannot be admitted.
        let cancel = Request::from_decoded(parsed, peer, None)
            .ok()
            .is_some_and(|request| ObservePlan::from_request(site, &request).deregister);
        if cancel {
            let key = ObserveKey::new(parsed.token(), peer);
            let cleanup = (|| -> Result<(), crate::storage::SlotError> {
                retire_terminal_response(engine, parsed.token(), peer)?;
                deferred.supersede_observe(engine, key)?;
                if let Some(interest) = engine
                    .lookup_observe(key)
                    .and_then(|id| engine.observe_interest(id))
                {
                    retire_suppressed_observe(engine, interest)?;
                }
                Ok(())
            })();
            if let Err(error) = cleanup {
                if let InboundBody::Complete(id) = assembled {
                    let _ = engine.release_rx_body(id);
                }
                let _ = engine.release_rx(rx);
                return Err(error.into());
            }
        }
        let outcome = admission::send_refusal(
            engine,
            io,
            ids,
            now_ms,
            SendResponse {
                block1: None,
                block2: None,
                q_block2: None,
                q_request: None,
                ..meta
            },
            oscore,
            dedup_closed,
        );
        if let InboundBody::Complete(id) = assembled {
            let _ = engine.release_rx_body(id);
        }
        let _ = engine.release_rx(rx);
        return outcome;
    }

    let request_plan = Request::from_decoded(
        parsed,
        peer,
        match assembled {
            InboundBody::Complete(id) => engine.rx_body_payload(id),
            _ => None,
        },
    )
    .ok()
    .map(|request| ObservePlan::from_request(site, &request));
    // RFC 7959 section 2.6 observes the entire resource and starts every
    // notification at its first block. NUM is not an observed subresource.
    let meta = if request_plan
        .is_some_and(|plan| plan.register && plan.registration_supported && plan.has_source)
    {
        SendResponse {
            block2: meta
                .block2
                .map(|b| BlockValue::new(0, false, b.szx()).expect("validated Block2")),
            ..meta
        }
    } else {
        meta
    };
    if request_plan.is_some_and(|plan| plan.register || plan.deregister) {
        if let Err(error) = retire_terminal_response(engine, parsed.token(), peer) {
            if let InboundBody::Complete(id) = assembled {
                let _ = engine.release_rx_body(id);
            }
            let _ = engine.release_rx(rx);
            return Err(error.into());
        }
        if let Err(error) =
            deferred.supersede_observe(engine, ObserveKey::new(parsed.token(), peer))
        {
            if let InboundBody::Complete(id) = assembled {
                let _ = engine.release_rx_body(id);
            }
            let _ = engine.release_rx(rx);
            return Err(error.into());
        }
    }
    let deferred_observe = request_plan
        .filter(|plan| {
            plan.register
                && plan.has_source
                && !plan.resource.is_none()
                && plan.registration_supported
                && !no_response.suppresses(Code::CONTENT)
        })
        .map(|plan| DeferredObserve {
            resource: plan.resource,
            attempted: None,
            failed_mid: None,
        });
    let selection = DeferredSelection::from_parts(
        &parsed,
        match assembled {
            InboundBody::Complete(id) => engine.rx_body_payload(id).unwrap_or(parsed.payload()),
            _ => parsed.payload(),
        },
    )
    .ok();
    let q_selectors = DeferredQSelectors::from_request(&parsed);
    let deferred_handle = if selection.is_some()
        && dispatch.is_some()
        && deferred_lifetime_ms != 0
        && (parsed.observe().is_none() || deferred_observe.is_some())
        && q_selectors.is_some()
    {
        match deferred.candidate(ids, io) {
            Ok(handle) => handle,
            Err(error) => {
                if let InboundBody::Complete(id) = assembled {
                    let _ = engine.release_rx_body(id);
                }
                let _ = engine.release_rx(rx);
                return Err(Error::Identity(error));
            }
        }
    } else {
        None
    };
    let mut catalog = site::LinkFormatScratch::<N>::new();
    let (response, plan) = {
        let body = match assembled {
            InboundBody::Complete(id) => engine.rx_body_payload(id),
            InboundBody::None
            | InboundBody::Continue
            | InboundBody::QWait(_)
            | InboundBody::Refused(_) => None,
        };
        match Request::from_decoded(parsed, peer, body)
            .map(|request| request.with_association(oscore.association(peer)))
        {
            Ok(request) => {
                let retained = match assembled {
                    InboundBody::Complete(id) => engine
                        .rx_body_transfer(id)
                        .is_some_and(|t| t.if_none_match()),
                    _ => false,
                };
                let request = request.with_retained_if_none_match(retained);
                let plan = ObservePlan::from_request(site, &request);
                let response = match dispatch {
                    Some(dispatch) => dispatch(request, deferred_handle),
                    None => site.dispatch(request, catalog.as_mut()),
                };
                (response, plan)
            }
            Err(
                request::PathError::BadUtf8
                | request::PathError::EmptySegment
                | request::PathError::DotSegment,
            ) => (
                Response::problem(Code::BAD_REQUEST).title("Bad Request"),
                ObservePlan::idle(),
            ),
            Err(request::PathError::TooLong) => (
                Response::problem(Code::NOT_FOUND).title("Not Found"),
                ObservePlan::idle(),
            ),
        }
    };
    if response.is_deferred() {
        let outcome = if let Some(handle) = deferred_handle {
            // Record before sending ACK: a failed send cannot cause durable work
            // to be dispatched twice when the request is retransmitted.
            deferred.insert(
                handle,
                SendResponse {
                    dest: meta.dest,
                    ty: meta.ty,
                    mid: meta.mid,
                    token: meta.token,
                    no_response: meta.no_response,
                    block2: meta.block2,
                    q_block2: meta.q_block2,
                    q_request: None,
                    block1: meta.block1,
                    oscore: meta.oscore,
                    request: meta.request,
                },
                now_ms.saturating_add(deferred_lifetime_ms),
                selection.expect("admitted selection"),
                q_selectors.expect("admitted selectors"),
                deferred_observe,
            );
            if meta.ty == Type::Confirmable {
                let outcome = send_empty_ack(engine, io, peer, meta.mid);
                if outcome.is_ok() {
                    remember_empty_ack(engine, peer, meta.mid, now_ms, meta.request, dedup_closed);
                }
                outcome
            } else {
                Ok(())
            }
        } else {
            send_response(
                engine,
                io,
                ids,
                meta,
                &Response::new(Code::SERVICE_UNAVAILABLE),
                now_ms,
                oscore,
                dedup_closed,
            )
        };
        if let InboundBody::Complete(id) = assembled {
            let _ = engine.release_rx_body(id);
        }
        let _ = engine.release_rx(rx);
        return outcome;
    }
    if let Err(error) = response.validate() {
        if let InboundBody::Complete(id) = assembled {
            let _ = engine.release_rx_body(id);
        }
        let _ = engine.release_rx(rx);
        return Err(Error::Response(error));
    }
    let response = match apply_observe(engine, now_ms, peer, response, plan, oscore_req) {
        Ok(response) => response,
        Err(error) => {
            if let InboundBody::Complete(id) = assembled {
                let _ = engine.release_rx_body(id);
            }
            let _ = engine.release_rx(rx);
            return Err(error.into());
        }
    };

    let outcome = if response.is_separate() {
        send_separate(
            engine,
            io,
            ids,
            now_ms,
            meta,
            &response,
            oscore,
            dedup_closed,
            false,
        )
    } else {
        send_response(
            engine,
            io,
            ids,
            meta,
            &response,
            now_ms,
            oscore,
            dedup_closed,
        )
    };
    let retained =
        retain_q_observe_selection(engine, BlockKey::new(meta.token, peer), plan.selection)
            .map_err(Error::Slot);
    if outcome.is_ok() && response.observe_seq().is_some() {
        record_q_observe_sent(engine, BlockKey::new(meta.token, peer), now_ms);
    }
    if let InboundBody::Complete(id) = assembled {
        let _ = engine.release_rx_body(id);
    }
    let _ = engine.release_rx(rx);
    outcome.and(retained)
}

#[derive(Clone, Copy)]
struct ObservePlan<K: OwnedToken = crate::message::Token> {
    selection: Option<crate::storage::ObserveSelection>,
    register: bool,
    registration_supported: bool,
    no_response: NoResponse,
    block2_max_szx: Option<u8>,
    q_block2: bool,
    deregister: bool,
    delete: bool,
    has_source: bool,
    token: K,
    resource: ObserveResource,
}

impl<K: OwnedToken> ObservePlan<K> {
    fn idle() -> Self {
        Self {
            register: false,
            selection: None,
            registration_supported: false,
            no_response: NoResponse::DEFAULT,
            block2_max_szx: None,
            q_block2: false,
            deregister: false,
            delete: false,
            has_source: false,
            token: K::EMPTY,
            resource: ObserveResource::NONE,
        }
    }

    fn from_request<const N: usize>(site: &Site<N, K>, request: &Request<'_, K>) -> Self {
        let selected = matches!(
            site.observe_source(site.observe_resource(request.path())),
            Some(routing::SnapshotSource::Selected(_))
        );
        let selection = {
            DeferredSelection::from_parts(
                &request.message(),
                request.body().unwrap_or(request.payload()),
            )
            .ok()
            .map(DeferredSelection::stored)
        };
        Self {
            register: request.is_observe_register(),
            no_response: NoResponse::from_message(&request.message())
                .unwrap_or(NoResponse::DEFAULT),
            selection,
            registration_supported: selection.is_some()
                // RFC 7641 section 4.1 permits ordinary-response fallback.
                // Classic notifications begin at NUM0. Q registration may
                // select an initial range; later epochs still represent the
                // complete resource, not a separately observed block.
                && request.message().block2().is_none_or(|block| {
                    block.is_ok_and(|b| !b.is_bert())
                })
                && request.message().q_block2().next().is_none_or(|block| {
                    block.is_ok_and(|b| !b.is_bert())
                })
                && (selected
                    || (request.method() == Some(Method::Get)
                        && request.uri_query().next().is_none())),
            block2_max_szx: request
                .get_option(crate::message::OptionNumber::BLOCK2)
                .and_then(|o| o.as_block().ok())
                .filter(|b| !b.is_bert())
                .map(|b| b.szx())
                .or_else(|| {
                    request
                        .message()
                        .q_block2()
                        .next()
                        .and_then(Result::ok)
                        .map(|b| b.szx())
                }),
            q_block2: request.message().q_block2().next().is_some(),
            deregister: request.is_observe_deregister(),
            delete: request.method() == Some(Method::Delete),
            has_source: site.has_observe_source(request.path()),
            token: request.token(),
            resource: site.observe_resource(request.path()),
        }
    }
}

fn apply_observe<'a, S: Storage + ObserveSlots + BodySlots + PendingCons>(
    engine: &mut Engine<S>,
    now_ms: u64,
    peer: Endpoint,
    mut response: Response<'a, S::Token>,
    plan: ObservePlan<S::Token>,
    oscore_req: oscore::Request,
) -> Result<Response<'a, S::Token>, crate::storage::SlotError> {
    let key = ObserveKey::new(plan.token, peer);

    let opted = response.observe_seq().is_some() || plan.has_source;
    response = response.without_observe();
    let previous = if plan.deregister || plan.register {
        // Retire only this relation's current notification before replacing its
        // row. A pinned TX refuses without detaching the old row or body.
        if let Some(interest) = engine
            .lookup_observe(key)
            .and_then(|id| engine.observe_interest(id))
            && let Some(mid) = interest.notify_hold().and_then(|hold| hold.con_mid())
            && let Some(tx) = engine.lookup_owned_con(mid, peer)
        {
            engine.release_tx(tx)?;
        }
        engine.take_observe(key)
    } else {
        None
    };
    if let Some(interest) = previous {
        release_observe_response_body(engine, interest);
    }
    if plan.register
        && plan.registration_supported
        && !plan.no_response.suppresses(Code::CONTENT)
        && response.code().is_success()
        && opted
    {
        // Re-registration replaces scheduling and protection state, but the
        // same Token/resource must retain ordering across that replacement.
        let sequence = previous
            .filter(|interest| interest.resource() == plan.resource)
            .map_or(0, |interest| interest.seq().wrapping_add(1) & 0x00ff_ffff);
        let interest = ObserveInterest::new(plan.token, peer)
            .with_seq(sequence)
            .with_resource(plan.resource)
            .with_block2_max_szx(plan.block2_max_szx)
            .with_q_block2(plan.q_block2)
            .with_no_response(plan.no_response)
            .with_selection(plan.selection);
        let interest = initial_observe_format(interest, &response, None);
        #[cfg(feature = "oscore")]
        let interest = interest.with_oscore(oscore_req);
        #[cfg(not(feature = "oscore"))]
        let _ = oscore_req;
        if engine.insert_observe(interest).is_some() {
            let max_age = response.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS);
            let _ = engine.refresh_observe_max_age(key, now_ms, max_age, None);
            response = response.observe(sequence);
        } else {
            response = response.without_observe();
        }
    }
    if plan.delete && response.code().is_success() {
        let _ = engine.mark_observe_resource_deleted(plan.resource);
    }
    Ok(response)
}

fn initial_observe_format<K: OwnedToken>(
    interest: ObserveInterest<K>,
    response: &Response<'_, K>,
    attempted_format: Option<Option<ContentFormat>>,
) -> ObserveInterest<K> {
    if response.code() != Code::VALID || response.format().is_some() {
        return interest.with_content_format(response.format());
    }
    if let Some(format) = attempted_format {
        return interest.with_content_format(format);
    }
    // A validation response need not repeat cached representation metadata.
    // Accept can bind its format; otherwise the resource must keep its cached
    // format invariant and the first Content supplies the observable binding.
    let accept = interest.selection().and_then(|selection| {
        crate::message::decode_with_token::<K>(&selection.bytes[..usize::from(selection.len)])
            .ok()?
            .accept()?
            .ok()
    });
    match accept {
        Some(format) => interest.with_content_format(Some(format)),
        None => interest.with_unbound_format(),
    }
}

fn bind_observe_format<S: Storage + ObserveSlots>(
    engine: &mut Engine<S>,
    interest: ObserveInterest<S::Token>,
    code: Code,
    format: Option<ContentFormat>,
) -> Result<(), crate::storage::SlotError> {
    if !interest.format_unbound() || !code.is_success() || (code == Code::VALID && format.is_none())
    {
        return Ok(());
    }
    let id = engine
        .lookup_observe(interest.key())
        .ok_or(crate::storage::SlotError::NotOccupied)?;
    let current = engine
        .observe_interest(id)
        .ok_or(crate::storage::SlotError::NotOccupied)?;
    let result = engine
        .storage_mut()
        .set_observe_interest(id, current.with_content_format(format));
    if result.is_err() {
        // A possibly visible representation must not leave an unbound relation
        // able to send another format after a failed metadata commit.
        engine.take_observe(interest.key());
    }
    result
}

// Cancellation can follow a committed Q packet while a later send is pending.
// Bind before BodyOwner releases the only retained representation metadata.
type OutgoingVisibility<'a, S> = (&'a mut bool, fn(&mut Engine<S>, SlotId));

fn bind_cancelled_q_format<S: Storage + ObserveSlots + BodySlots>(
    engine: &mut Engine<S>,
    body: SlotId,
) {
    let Some(transfer) = engine.tx_body_transfer(body) else {
        return;
    };
    if transfer.q_observe_last_mid().is_none() {
        return;
    }
    let Some((code, format, _)) = transfer.notification_metadata() else {
        return;
    };
    let key = ObserveKey::new(transfer.key().token(), transfer.endpoint());
    let Some(interest) = engine
        .lookup_observe(key)
        .and_then(|id| engine.observe_interest(id))
    else {
        return;
    };
    let _ = bind_observe_format(engine, interest, code, format);
}

#[allow(clippy::too_many_arguments)]
fn supersede_notification<S, T, const N: usize, const REQUESTS: usize, const DEFERRED: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    site: &Site<N, S::Token>,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    deferred: &DeferredTable<DEFERRED, S::Token>,
    now_ms: u64,
    pending: crate::storage::PendingCon,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: crate::storage::DatagramTransport,
{
    let interest = (0..engine.capacities().observe_entries).find_map(|index| {
        let row = engine.observe_interest(SlotId::from_index(index))?;
        (!row.key().is_client()
            && row.is_pending()
            && row.endpoint() == pending.endpoint()
            && row.notify_hold().and_then(|hold| hold.con_mid()) == Some(pending.message_id()))
        .then_some(row)
    });
    let Some(interest) = interest else {
        return Ok(());
    };
    // Finish the immutable Q representation before replacing its epoch/type.
    if response_body_for(engine, BlockKey::new(interest.token(), interest.endpoint()))
        .and_then(|id| engine.tx_body_transfer(id))
        .is_some_and(|body| {
            body.q_observe_confirmable()
                && (!body.is_complete() || body.q_observe_pending_mid().is_some())
        })
    {
        return Ok(());
    }
    if deferred.blocks_observe(interest.key()) {
        return Ok(());
    }
    let mut response = if interest.is_deleted() {
        Response::not_found()
    } else {
        let Some(source) = site.observe_source(interest.resource()) else {
            return Ok(());
        };
        let Some(response) = source.snapshot(&interest, oscore.association(interest.endpoint()))
        else {
            return Ok(());
        };
        response
    };
    response.validate().map_err(Error::Response)?;
    if response.code().is_success()
        && !interest.content_format_matches(response.code(), response.format())
    {
        response = Response::new(Code::NOT_ACCEPTABLE);
    }
    if interest.suppresses(response.code()) {
        retire_suppressed_observe(engine, interest)?;
        return Ok(());
    }
    let terminal = !response.code().is_success();
    let previous_body =
        response_body_for(engine, BlockKey::new(interest.token(), interest.endpoint()));
    if interest.superseded_con_mids().len() == usize::from(Transmission::MAX_RETRANSMIT) {
        return Err(Error::Saturated);
    }
    let mid = ids.next_for(engine, io, now_ms)?;
    let mut replacement = interest
        .supersede_con(now_ms, mid)
        .ok_or(Error::Saturated)?;
    if interest.format_unbound()
        && response.code().is_success()
        && (response.code() != Code::VALID || response.format().is_some())
    {
        replacement = replacement.with_content_format(response.format());
    }
    response = if terminal {
        response.without_observe()
    } else {
        response.observe(replacement.seq())
    };
    let mut wire = S::TxScratch::default();
    let preferred = interest.block2_max_szx();
    let oversized =
        !terminal && preferred.is_some_and(|szx| response.payload().len() > (1usize << (szx + 4)));
    let staged = if oversized {
        Err(Error::Message(SlotMessageError::Encode(
            EncodeError::BufferTooSmall,
        )))
    } else {
        stage_observe_replacement(
            oscore,
            interest,
            mid,
            &response,
            None,
            response.payload(),
            wire.as_mut(),
        )
    };
    let mut body = None;
    let n = match staged {
        Ok(n) => n,
        Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))) => {
            let Some(capacity) = engine.capacities().tx_body_bytes else {
                return Err(Error::Message(SlotMessageError::Encode(
                    EncodeError::BufferTooSmall,
                )));
            };
            let key = response_block_key(interest.token(), interest.endpoint(), &response)?;
            let maximum = preferred.unwrap_or(BlockValue::SZX_MAX);
            let mut encoded = None;
            for szx in (0..=maximum).rev() {
                let mut transfer = crate::storage::BlockTransfer::outgoing(
                    key,
                    BlockRole::OutgoingBlock2,
                    response.payload().len(),
                    szx,
                    capacity,
                )
                .map_err(Error::Block)?;
                transfer.remember_notification(
                    response.code(),
                    response.format(),
                    response.max_age_secs(),
                );
                transfer.retain_observe_selection(interest.selection().copied());
                transfer.expire_terminal_at(
                    now_ms.saturating_add(u64::from(Transmission::EXCHANGE_LIFETIME_MS)),
                );
                let (block, offset, len) = transfer.issue_outgoing().map_err(Error::Block)?;
                match stage_observe_replacement(
                    oscore,
                    interest,
                    mid,
                    &response,
                    Some(block),
                    &response.payload()[offset..offset + len],
                    wire.as_mut(),
                ) {
                    Ok(n) => {
                        body = Some(transfer);
                        encoded = Some(n);
                        break;
                    }
                    Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)))
                        if szx > 0 => {}
                    Err(error) => return Err(error),
                }
            }
            encoded.expect("finite SZX staging returns at zero")
        }
        Err(error) => return Err(error),
    };
    if body.is_none()
        && let Some(id) = previous_body
    {
        drop(engine.access_tx_body_mut(id)?);
    }
    let commit = if let Some(transfer) = body {
        engine.storage_mut().replace_observe_body(
            pending,
            interest,
            replacement,
            &wire.as_ref()[..n],
            previous_body,
            transfer,
            response.payload(),
        )
    } else {
        engine.supersede_observe_con(pending, interest, replacement, &wire.as_ref()[..n])
    };
    commit.map_err(|error| match error {
        crate::storage::PendingReplaceError::Slot(slot) => Error::Slot(slot),
        crate::storage::PendingReplaceError::Body(error) => Error::Block(error),
        crate::storage::PendingReplaceError::Unsupported => Error::ObserveSupersessionUnsupported,
        _ => Error::Saturated,
    })?;
    #[cfg(feature = "diagnostics")]
    if body.is_some() {
        crate::storage::WorkMetrics::add(&mut engine.work_metrics_mut().raw_tx_copied_bytes, n);
        crate::storage::WorkMetrics::add(
            &mut engine.work_metrics_mut().block_tx_staged_bytes,
            response.payload().len(),
        );
    }
    if body.is_none() {
        release_observe_response_body(engine, interest);
    }
    if terminal {
        // The pending packet retains delivery/retry ownership after the relation
        // ends. Old notification ACKs cannot acknowledge its new Message ID.
        let _ = engine.take_observe(interest.key());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_observe_replacement<K: OwnedToken, E, const REQUESTS: usize>(
    oscore: &mut oscore::Field<REQUESTS, K>,
    interest: ObserveInterest<K>,
    mid: MessageId,
    response: &Response<'_, K>,
    block: Option<BlockValue>,
    payload: &[u8],
    wire: &mut [u8],
) -> Result<usize, Error<E>> {
    let cf = response.format().map(crate::ContentFormat::encode);
    let age = response.max_age_secs().map(EncodedUint::new);
    let seq = response.observe_seq().map(encode_uint);
    let block = block.map(|block| block.encode());
    let mut options = OptionsBuilder::<5>::new();
    let filled = (|| -> Result<(), EncodeError> {
        if let Some(tag) = response.etag_bytes() {
            push_opt(&mut options, Opt::etag(tag))?;
        }
        if let Some(ref seq) = seq {
            push_opt(&mut options, Opt::observe(seq))?;
        }
        if let Some(ref cf) = cf {
            push_opt(&mut options, Opt::content_format(cf))?;
        }
        if let Some(ref age) = age {
            push_opt(&mut options, Opt::max_age(age))?;
        }
        if let Some(ref block) = block {
            push_opt(&mut options, Opt::block2(block))?;
        }
        Ok(())
    })();
    filled.map_err(|error| Error::Message(SlotMessageError::Encode(error)))?;
    let message = Message::new(Type::Confirmable, response.code(), mid)
        .with_token(interest.token())
        .with_options(options.as_slice())
        .with_payload(payload);
    oscore::stage_notification(
        oscore,
        interest.endpoint(),
        oscore::request_from_interest(interest),
        &message,
        wire,
    )
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(notify_engine_async, bound(crate::app::async_server::ProtocolIo), calls(send_notification => send_notification_async))]
fn notify_engine<S, T, const REQUESTS: usize, const DEFERRED: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    resource: ObserveResource,
    response: &Response<'_, S::Token>,
    confirmable: bool,
    deferred: &DeferredTable<DEFERRED, S::Token>,
) -> Result<usize, Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + DedupSlots,
    T: DatagramIo,
{
    response.validate().map_err(Error::Response)?;
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
        if interest.key().is_client()
            || interest.resource() != resource
            || deferred.blocks_observe(interest.key())
        {
            continue;
        }
        if held.count(engine, interest.endpoint(), now_ms)
            >= usize::from(crate::message::Transmission::NSTART)
        {
            Metrics::inc(&mut engine.metrics_mut().nstart_reject);
            continue;
        }
        let Some(seq) = engine.next_observe_seq_with_confirmation(id, confirmable) else {
            continue;
        };
        let Some(interest) = engine.observe_interest(id) else {
            continue;
        };
        let dest = interest.endpoint();
        match send_notification(engine, io, ids, oscore, now_ms, interest, response, seq) {
            Ok(true) => {
                held.add(dest);
                sent += 1;
            }
            Ok(false) => {}
            Err(e) => {
                let _ = engine.restore_observe_due(id, seq);
                return Err(e);
            }
        }
    }
    Ok(sent)
}

/// Per-endpoint notification NSTART counts for one [`notify_engine`] pass.
///
/// Bounded scans of Observe and pending-CON tables, including terminal
/// delivery whose observer row is already removed, without double-counting.
/// Shipped profiles have ≤4 observe entries; the table covers 8 unique
/// endpoints and falls back to a full scan if that overflows (alloc backend).
const NOTIFY_HELD_CACHE: usize = 8;

struct NotifyHeldCache {
    endpoints: [Option<Endpoint>; NOTIFY_HELD_CACHE],
    held: [u8; NOTIFY_HELD_CACHE],
    overflow: bool,
}

impl NotifyHeldCache {
    fn from_engine<S: Storage + ObserveSlots + PendingCons>(
        engine: &Engine<S>,
        now_ms: u64,
    ) -> Self {
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
            if !row.key().is_client() && row.is_notify_held(now_ms) {
                cache.add(row.endpoint());
            }
        }
        for i in 0..engine.capacities().tx_datagram_slots {
            if let Some(pending) = engine.pending_con(SlotId::from_index(i))
                && !pending_has_observe_hold(engine, pending, now_ms)
            {
                cache.add(pending.endpoint());
            }
        }
        cache
    }

    fn add(&mut self, endpoint: Endpoint) {
        let endpoint = endpoint.physical();
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
        let endpoint = endpoint.physical();
        self.endpoints.iter().position(|row| *row == Some(endpoint))
    }

    fn count<S: Storage + ObserveSlots + PendingCons>(
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

fn observe_endpoint_held<S: Storage + ObserveSlots + PendingCons>(
    engine: &Engine<S>,
    endpoint: Endpoint,
    now_ms: u64,
) -> usize {
    let n = engine.capacities().observe_entries;
    let interests = (0..n)
        .filter(|&i| {
            engine
                .observe_interest(SlotId::from_index(i))
                .is_some_and(|row| {
                    !row.key().is_client()
                        && row.endpoint().physical() == endpoint.physical()
                        && row.is_notify_held(now_ms)
                })
        })
        .count();
    interests
        + (0..engine.capacities().tx_datagram_slots)
            .filter(|&i| {
                engine
                    .pending_con(SlotId::from_index(i))
                    .is_some_and(|pending| {
                        pending.endpoint().physical() == endpoint.physical()
                            && !pending_has_observe_hold(engine, pending, now_ms)
                    })
            })
            .count()
}

fn pending_has_observe_hold<S: Storage + ObserveSlots>(
    engine: &Engine<S>,
    pending: crate::storage::PendingCon,
    now_ms: u64,
) -> bool {
    (0..engine.capacities().observe_entries).any(|i| {
        engine
            .observe_interest(SlotId::from_index(i))
            .is_some_and(|row| {
                !row.key().is_client()
                    && row.endpoint() == pending.endpoint()
                    && row.is_notify_held(now_ms)
                    && (row.notify_hold().and_then(|hold| hold.con_mid())
                        == Some(pending.message_id())
                        || row.lifetime().and_then(|life| life.con_mid())
                            == Some(pending.message_id()))
            })
    })
}

// Empty control messages carry no protected recipient ID or Token. Resolve
// only a unique retained outgoing MID at the physical endpoint.
fn control_route<S: Storage + PendingCons + ObserveSlots + Exchanges, const D: usize>(
    engine: &Engine<S>,
    deferred: &DeferredTable<D, S::Token>,
    peer: Endpoint,
    mid: MessageId,
    now_ms: u64,
) -> Option<Endpoint> {
    let mut found = None;
    let mut ambiguous = false;
    let mut accept = |route: Endpoint| {
        // Observe and Exchange ownership can be prepared before transport
        // acceptance. Neither may turn an unsent reservation into wire authority.
        if engine
            .lookup_owned_con(mid, route)
            .and_then(|id| engine.pending_con(id))
            .is_some_and(|pending| !pending.rto().is_started())
        {
            return;
        }
        if route.physical() == peer.physical() {
            if found.is_some_and(|previous| previous != route) {
                ambiguous = true;
            }
            found = Some(route);
        }
    };
    for i in 0..engine.capacities().tx_datagram_slots {
        if let Some(row) = engine.pending_con(SlotId::from_index(i))
            && row.message_id() == mid
        {
            accept(row.endpoint());
        }
    }
    for i in 0..engine.capacities().tx_datagram_slots {
        if let Some(row) = engine.exchange_entry(SlotId::from_index(i))
            && row.message_id() == mid
        {
            accept(row.endpoint());
        }
    }
    for i in 0..engine.capacities().observe_entries {
        if let Some(row) = engine.observe_interest(SlotId::from_index(i))
            && !row.key().is_client()
            && (row.lifetime().and_then(|life| life.con_mid()) == Some(mid)
                || row
                    .notify_hold()
                    .is_some_and(|hold| hold.message_id() == mid)
                || row.superseded_con_mids().contains(&mid))
        {
            accept(row.endpoint());
        }
    }
    for row in deferred.rows.iter().flatten() {
        if row.observe.is_some_and(|observe| {
            observe
                .failed_mid
                .is_some_and(|(sent, expires)| sent == mid && now_ms < expires)
        }) {
            accept(row.meta.dest);
        }
    }
    if ambiguous { None } else { found }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_notification_io_async, bound(crate::app::async_server::ProtocolIo), calls(completion_time => async_server::completion_time_async, send_may_have_reached_peer => async_server::send_may_have_reached_peer_async, start_outgoing => start_outgoing_async, start_notify_block2 => start_notify_block2_async, finish_send => finish_send_async))]
fn send_notification<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    interest: ObserveInterest<S::Token>,
    response: &Response<'_, S::Token>,
    seq: u32,
) -> Result<bool, Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + DedupSlots,
    T: DatagramIo,
{
    response.validate().map_err(Error::Response)?;
    let mut notify = if interest.is_deleted() {
        Response::not_found()
    } else if response.code().is_success()
        && !interest.content_format_matches(response.code(), response.format())
    {
        Response::new(Code::NOT_ACCEPTABLE)
    } else {
        *response
    };
    if interest.suppresses(notify.code()) {
        retire_suppressed_observe(engine, interest)?;
        return Ok(false);
    }
    // Terminal delivery remains tracked even after removing the observer row.
    let ty = if !notify.code().is_success() || interest.must_confirm(now_ms) {
        Type::Confirmable
    } else {
        Type::NonConfirmable
    };
    let mid = ids.next_for(engine, io, now_ms)?;
    if notify.code().is_success() {
        notify.set_observe(seq);
    } else {
        notify = notify.without_observe();
    }
    let dest = interest.endpoint();
    let token = interest.token();
    let key = BlockKey::new(token, dest);
    if let Some(body) = response_body_for(engine, key) {
        let _ = engine.release_tx_body(body);
    }

    let oscore_req = oscore::request_from_interest(interest);
    let meta = SendResponse {
        dest,
        ty,
        mid,
        token,
        no_response: NoResponse::DEFAULT,
        block2: interest
            .block2_max_szx()
            .filter(|_| {
                !interest.q_block2()
                    || (!notify.code().is_success() && notify.etag_bytes().is_none())
            })
            .map(|szx| BlockValue::new(0, false, szx).expect("admitted SZX")),
        q_block2: interest
            .block2_max_szx()
            .filter(|_| {
                interest.q_block2() && (notify.code().is_success() || notify.etag_bytes().is_some())
            })
            .map(|szx| BlockValue::new(0, true, szx).expect("admitted SZX")),
        q_request: None,
        block1: None,
        oscore: oscore_req,
        request: interest
            .selection()
            .and_then(|selection| {
                crate::message::extended::decode(&selection.bytes[..usize::from(selection.len)])
                    .ok()
            })
            .map_or(Code::GET, |parsed| parsed.code()),
    };

    if meta.q_block2.is_some() {
        let mut visible = false;
        let outcome = start_outgoing(
            engine,
            io,
            ids,
            now_ms,
            meta,
            &notify,
            ty,
            key,
            oscore,
            &mut None,
            false,
            Some((&mut visible, bind_cancelled_q_format::<S>)),
        );
        let ambiguous = match &outcome {
            Err(error) => {
                matches!(error, Error::CommitAfterSend) || send_may_have_reached_peer(io, error)
            }
            Ok(()) => false,
        };
        if outcome.is_ok() || visible || ambiguous {
            bind_observe_format(engine, interest, notify.code(), notify.format())
                .map_err(|_| Error::CommitAfterSend)?;
        }
        outcome?;
        retain_q_observe_selection(engine, key, interest.selection().copied())?;
        let now_ms = completion_time(io, now_ms);
        if !notify.code().is_success() {
            engine.take_observe(interest.key());
            return Ok(true);
        }
        record_q_observe_sent(engine, key, now_ms);
        if ty == Type::NonConfirmable {
            let _ = engine.refresh_observe_max_age(
                interest.key(),
                now_ms,
                notify.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS),
                None,
            );
        }
        return Ok(true);
    }
    let Some(tx) = acquire_tx_or_evict(engine) else {
        return Err(Error::Saturated);
    };
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid, ids.jitter()));
    let oversized = notify.code().is_success()
        && interest
            .block2_max_szx()
            .is_some_and(|szx| notify.payload().len() > (1usize << (szx + 4)));
    let encoded = if oversized {
        Err(Error::Message(SlotMessageError::Encode(
            EncodeError::BufferTooSmall,
        )))
    } else {
        encode_notification(
            engine,
            dest,
            tx,
            ty,
            mid,
            token,
            &notify,
            notify.payload(),
            oscore,
            oscore_req,
        )
    };
    let outcome = match encoded {
        Ok(()) => finish_send(engine, io, tx, dest, pending),
        Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))) => {
            let _ = engine.release_tx(tx);
            start_notify_block2(engine, io, meta, &notify, ty, pending, oscore)
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e)
        }
    };
    let ambiguous = match &outcome {
        Err(error) => {
            matches!(error, Error::CommitAfterSend) || send_may_have_reached_peer(io, error)
        }
        Ok(()) => false,
    };
    if outcome.is_ok() || ambiguous {
        bind_observe_format(engine, interest, notify.code(), notify.format())
            .map_err(|_| Error::CommitAfterSend)?;
    }
    outcome?;
    let now_ms = completion_time(io, now_ms);
    if !notify.code().is_success() {
        let _ = engine.take_observe(interest.key());
        return Ok(true);
    }

    let confirmable = ty == Type::Confirmable;
    let _ = engine.record_observe_notify(interest.key(), now_ms, mid, confirmable);
    if !confirmable {
        let max_age = notify.max_age_secs().unwrap_or(DEFAULT_MAX_AGE_SECS);
        let _ = engine.refresh_observe_max_age(interest.key(), now_ms, max_age, None);
    }
    Ok(true)
}

#[coaptic_io::dual_io(start_notify_block2_async, bound(crate::app::async_server::ProtocolIo), calls(issue_observe_preferred => issue_observe_preferred_async, issue_classic => issue_classic_async))]
fn start_notify_block2<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    pending: Option<(u64, MessageId, u32)>,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots + ObserveSlots,
    T: DatagramIo,
{
    let key = response_block_key(meta.token, meta.dest, response)?;
    let id = match engine.start_block2(
        key,
        response.payload(),
        meta.block2.map_or(BlockValue::SZX_MAX, |b| b.szx()),
    ) {
        Ok(id) => id,
        Err(BlockTransferError::NoBodyPools) => {
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )));
        }
        Err(e) => return Err(Error::Block(e)),
    };
    let mut body_owner = async_server::BodyOwner::outgoing(engine, id);
    let engine = &mut *body_owner.engine;
    if let Some(mut transfer) = engine.tx_body_transfer(id) {
        transfer.remember_notification(response.code(), response.format(), response.max_age_secs());
        transfer.retain_observe_selection(
            engine
                .lookup_observe(ObserveKey::new(meta.token, meta.dest))
                .and_then(|id| engine.observe_interest(id))
                .and_then(|row| row.selection().copied()),
        );
        if let Some((now, _, _)) = pending {
            transfer.expire_terminal_at(
                now.saturating_add(u64::from(Transmission::EXCHANGE_LIFETIME_MS)),
            );
        }
        engine
            .storage_mut()
            .set_tx_body_transfer(id, transfer)
            .map_err(Error::Slot)?;
    }
    let outcome = if let Some(preferred) = meta.block2 {
        issue_observe_preferred(
            engine,
            io,
            meta,
            response,
            ty,
            id,
            pending,
            None,
            oscore_ctx,
            preferred.szx(),
        )
    } else {
        issue_classic(
            engine, io, meta, response, ty, meta.mid, id, pending, true, None, oscore_ctx,
        )
    };
    if outcome.is_err() {
        let _ = engine.release_tx_body(id);
    }
    body_owner.disarm();
    outcome
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_separate_async, bound(crate::app::async_server::ProtocolIo), calls(send_response => send_response_async, start_outgoing => start_outgoing_async, continue_outgoing => continue_outgoing_async, send_empty_ack => send_empty_ack_async, finish_send => finish_send_async))]
fn send_separate<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
    acknowledged: bool,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    response.validate().map_err(Error::Response)?;
    if meta.q_block2.is_some() && response.code().is_success() {
        // Q bodies consistently use NON responses, including handlers that
        // request a separate response. The Q sender supplies the empty ACK.
        return send_response(
            engine,
            io,
            ids,
            meta,
            response,
            now_ms,
            oscore_ctx,
            dedup_closed,
        );
    }
    let key = BlockKey::new(meta.token, meta.dest);
    if let Some(id) = response_body_for(engine, key)
        && engine.tx_body_transfer(id).is_some_and(|t| {
            matches!(
                t.role(),
                BlockRole::OutgoingBlock2 | BlockRole::OutgoingQBlock2
            )
        })
    {
        let ty = match meta.ty {
            Type::Confirmable if acknowledged => Type::Confirmable,
            Type::Confirmable => Type::Acknowledgement,
            Type::NonConfirmable => Type::NonConfirmable,
            Type::Acknowledgement | Type::Reset => return Ok(()),
        };
        let mut meta = meta;
        if ty != Type::Acknowledgement {
            meta.mid = ids.next_for(engine, io, now_ms)?;
        }
        return continue_outgoing(
            engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            ty,
            id,
            oscore_ctx,
            dedup_closed,
        );
    }

    let ty = match meta.ty {
        Type::Confirmable => {
            remember_empty_ack(
                engine,
                meta.dest,
                meta.mid,
                now_ms,
                meta.request,
                dedup_closed,
            );
            if !acknowledged {
                send_empty_ack(engine, io, meta.dest, meta.mid)?;
            }
            Type::Confirmable
        }
        Type::NonConfirmable => Type::NonConfirmable,
        Type::Acknowledgement | Type::Reset => return Ok(()),
    };

    if meta.no_response.suppresses(response.code()) {
        return Ok(());
    }

    let mid = ids.next_for(engine, io, now_ms)?;
    if meta.block2.is_some()
        && response.code().is_success()
        && response.code() != Code::VALID
        && response.observe_seq().is_some()
    {
        return start_outgoing(
            engine,
            io,
            ids,
            now_ms,
            SendResponse { mid, ..meta },
            response,
            ty,
            key,
            oscore_ctx,
            dedup_closed,
            false,
            None,
        );
    }
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid, ids.jitter()));

    let Some(tx) = acquire_tx_or_evict(engine) else {
        return Err(Error::Saturated);
    };

    match encode_response(
        engine,
        meta.dest,
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
        Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))) => {
            // True size miss: Inner Block2 (also under OSCORE). Protocol
            // protect failures are `Error::Oscore`, not this arm.
            let _ = engine.release_tx(tx);
            let meta = SendResponse { mid, ..meta };
            start_outgoing(
                engine,
                io,
                ids,
                now_ms,
                meta,
                response,
                ty,
                key,
                oscore_ctx,
                dedup_closed,
                false,
                None,
            )
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_response_async, bound(crate::app::async_server::ProtocolIo), calls(admission::send_control_response => admission::send_control_response_async, start_outgoing => start_outgoing_async, continue_outgoing => continue_outgoing_async, send_pinned_tx => send_pinned_tx_async, send_empty_ack => send_empty_ack_async, finish_send => finish_send_async))]
fn send_response<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    now_ms: u64,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    response.validate().map_err(Error::Response)?;
    if meta.no_response.suppresses(response.code()) {
        return match meta.ty {
            Type::Confirmable => {
                remember_empty_ack(
                    engine,
                    meta.dest,
                    meta.mid,
                    now_ms,
                    meta.request,
                    dedup_closed,
                );
                send_empty_ack(engine, io, meta.dest, meta.mid)?;
                Ok(())
            }
            Type::NonConfirmable | Type::Acknowledgement | Type::Reset => Ok(()),
        };
    }

    if response.code() == Code::CONTINUE || response.code() == Code::REQUEST_ENTITY_INCOMPLETE {
        return admission::send_control_response(
            engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            oscore_ctx,
            dedup_closed,
        );
    }

    let ty = match meta.ty {
        Type::Confirmable => Type::Acknowledgement,
        Type::NonConfirmable => Type::NonConfirmable,
        Type::Acknowledgement | Type::Reset => return Ok(()),
    };

    let mut meta = meta;
    if ty == Type::NonConfirmable {
        meta.mid = ids.next_for(engine, io, now_ms)?;
    }

    let key = BlockKey::new(meta.token, meta.dest);
    if let Some(id) = response_body_for(engine, key)
        .filter(|_| response.code().is_success() && response.code() != Code::CONTINUE)
        && engine.tx_body_transfer(id).is_some_and(|t| {
            matches!(
                t.role(),
                BlockRole::OutgoingBlock2 | BlockRole::OutgoingQBlock2
            )
        })
    {
        return continue_outgoing(
            engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            ty,
            id,
            oscore_ctx,
            dedup_closed,
        );
    }

    if (meta.block2.is_some() || meta.q_block2.is_some())
        && response.code().is_success()
        && response.code() != Code::VALID
    {
        return start_outgoing(
            engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            ty,
            key,
            oscore_ctx,
            dedup_closed,
            true,
            None,
        );
    }

    let Some(tx) = acquire_tx_or_evict(engine) else {
        return Err(Error::Saturated);
    };

    match encode_response(
        engine,
        meta.dest,
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
        Ok(()) => match remember_tx_reply(
            engine,
            tx,
            meta.dest,
            meta.mid,
            meta.ty,
            now_ms,
            meta.request,
            dedup_closed,
        ) {
            KeepTx::Yes => send_pinned_tx(engine, io, tx, meta.dest),
            KeepTx::No => finish_send(engine, io, tx, meta.dest, None),
        },
        Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))) => {
            // True size miss: Inner Block2 (also under OSCORE). Protocol
            // protect failures are `Error::Oscore`, not this arm.
            let _ = engine.release_tx(tx);
            start_outgoing(
                engine,
                io,
                ids,
                now_ms,
                meta,
                response,
                ty,
                key,
                oscore_ctx,
                dedup_closed,
                true,
                None,
            )
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e)
        }
    }
}

fn response_block_key<K: OwnedToken, E>(
    token: K,
    endpoint: Endpoint,
    response: &Response<'_, K>,
) -> Result<BlockKey<K>, Error<E>> {
    let key = BlockKey::new(token, endpoint);
    match response.etag_bytes() {
        Some(tag) => Ok(key.with_identity(
            crate::storage::BodyTag::new(tag)
                .map_err(BlockTransferError::from)
                .map_err(Error::Block)?,
        )),
        None => Ok(key),
    }
}

// A new same-token registration/deregistration replaces a completed relation's
// terminal transfer too. Preflight every owner before releasing any of them.
fn retire_terminal_response<S: Storage + BodySlots + PendingCons + DatagramSlots>(
    engine: &mut Engine<S>,
    token: S::Token,
    peer: Endpoint,
) -> Result<(), crate::storage::SlotError> {
    let Some(body) = response_body_for(engine, BlockKey::new(token, peer)) else {
        return Ok(());
    };
    if engine
        .tx_body_transfer(body)
        .and_then(|t| t.terminal_deadline())
        .is_none()
    {
        return Ok(());
    }
    if engine.tx_body_is_pinned(body) {
        return Err(crate::storage::SlotError::Pinned);
    }
    for commit in [false, true] {
        for index in 0..engine.capacities().tx_datagram_slots {
            let tx = SlotId::from_index(index);
            let belongs = engine
                .pending_con(tx)
                .is_some_and(|pending| pending.endpoint() == peer)
                && engine
                    .decode_tx(tx)
                    .is_ok_and(|packet| packet.token() == token && packet.code().is_response());
            if belongs {
                if commit {
                    engine.release_tx(tx)?;
                } else if engine.tx_is_pinned(tx) {
                    return Err(crate::storage::SlotError::Pinned);
                }
            }
        }
    }
    engine.release_tx_body(body)
}

// Suppressed terminal responses still end the relation and all owned retries.
fn retire_suppressed_observe<
    S: Storage + BodySlots + ObserveSlots + PendingCons + DatagramSlots,
>(
    engine: &mut Engine<S>,
    interest: ObserveInterest<S::Token>,
) -> Result<(), crate::storage::SlotError> {
    let body = response_body_for(engine, BlockKey::new(interest.token(), interest.endpoint()));
    if body.is_some_and(|id| engine.tx_body_is_pinned(id)) {
        return Err(crate::storage::SlotError::Pinned);
    }
    if let Some(mid) = interest.notify_hold().and_then(|hold| hold.con_mid())
        && let Some(tx) = engine.lookup_owned_con(mid, interest.endpoint())
    {
        engine.release_tx(tx)?;
    }
    if let Some(body) = body {
        engine.release_tx_body(body)?;
    }
    engine.take_observe(interest.key());
    Ok(())
}

fn release_observe_response_body<S: Storage + BodySlots>(
    engine: &mut Engine<S>,
    interest: ObserveInterest<S::Token>,
) {
    if !interest.key().is_client()
        && let Some(id) =
            response_body_for(engine, BlockKey::new(interest.token(), interest.endpoint()))
    {
        let _ = engine.release_tx_body(id);
    }
}

fn response_body_for<S: Storage + BodySlots>(
    engine: &Engine<S>,
    key: BlockKey<S::Token>,
) -> Option<SlotId> {
    (0..engine.capacities().tx_body_slots.unwrap_or(0)).find_map(|index| {
        let id = SlotId::from_index(index);
        let transfer = engine.tx_body_transfer(id)?;
        (transfer.token() == key.token()
            && transfer.endpoint() == key.endpoint()
            && matches!(
                transfer.role(),
                BlockRole::OutgoingBlock2 | BlockRole::OutgoingQBlock2
            ))
        .then_some(id)
    })
}

fn retain_q_observe_selection<S: Storage + BodySlots>(
    engine: &mut Engine<S>,
    key: BlockKey<S::Token>,
    selection: Option<crate::storage::ObserveSelection>,
) -> Result<(), crate::storage::SlotError> {
    if let Some(id) = response_body_for(engine, key)
        && let Some(mut transfer) = engine.tx_body_transfer(id)
        && transfer.q_observe_sequence().is_some()
    {
        transfer.retain_observe_selection(selection);
        engine.storage_mut().set_tx_body_transfer(id, transfer)?;
    }
    Ok(())
}

fn record_q_observe_sent<S: Storage + BodySlots + ObserveSlots>(
    engine: &mut Engine<S>,
    key: BlockKey<S::Token>,
    now_ms: u64,
) {
    if let Some(body) = response_body_for(engine, key)
        && let Some(transfer) = engine.tx_body_transfer(body)
        && let Some(mid) = transfer.q_observe_last_mid()
    {
        let _ = engine.record_observe_notify(
            ObserveKey::new(key.token(), key.endpoint()),
            now_ms,
            mid,
            transfer.q_observe_confirmable(),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_q_observe_first<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    mid: MessageId,
    body: SlotId,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
) -> Result<SlotId, Error<T::Error>>
where
    S: Storage + DatagramSlots + BodySlots + DedupSlots,
    T: crate::storage::DatagramTransport,
{
    loop {
        let issued = if let Some(q) = meta.q_block2.filter(|q| q.num() != 0) {
            engine.reissue_q_block2(body, q.num())
        } else {
            engine.peek_q_block2(body)
        }
        .map_err(Error::Block)?;
        match encode_issued::<S, T, REQUESTS>(engine, meta, response, ty, mid, issued, true, oscore)
        {
            Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))) => {
                // NUM is expressed in the requested SZX. A selective offset
                // cannot be silently reinterpreted after size negotiation.
                if meta.q_block2.is_some_and(|q| q.num() != 0)
                    || meta.q_request.is_some_and(|request| {
                        request.q_block2().any(|q| q.is_ok_and(|q| q.num() != 0))
                    })
                {
                    return Err(Error::Message(SlotMessageError::Encode(
                        EncodeError::BufferTooSmall,
                    )));
                }
                let mut transfer = engine
                    .tx_body_transfer(body)
                    .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
                let Some(szx) = transfer.szx().checked_sub(1) else {
                    return Err(Error::Message(SlotMessageError::Encode(
                        EncodeError::BufferTooSmall,
                    )));
                };
                transfer
                    .negotiate_q_observe_szx(szx)
                    .map_err(Error::Block)?;
                engine.storage_mut().set_tx_body_transfer(body, transfer)?;
            }
            result => return result,
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_q_observe_followup_async, bound(crate::app::async_server::ProtocolIo), calls(admission::send_refusal => admission::send_refusal_async, q_response_mid => q_response_mid_async, issue_q_window => issue_q_window_async, issue_q_selection => issue_q_selection_async))]
fn issue_q_observe_followup<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    body: SlotId,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let transfer = engine
        .tx_body_transfer(body)
        .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
    let q = meta
        .q_block2
        .ok_or(Error::Block(BlockTransferError::IdentityMismatch))?;
    if q.szx() != transfer.szx() {
        return Err(Error::Block(BlockTransferError::SzxMismatch));
    }
    let continuation = !transfer.q_observe_confirmable()
        && q.more()
        && q.num() != 0
        && q.num() % u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS) == 0
        && meta
            .q_request
            .is_none_or(|request| request.q_block2().nth(1).is_none());
    let response = if continuation && response.code().is_success() {
        response
            .without_observe()
            .observe(transfer.q_observe_sequence().expect("Q snapshot"))
    } else {
        response.without_observe()
    };
    #[cfg(feature = "oscore")]
    let meta = if continuation {
        SendResponse {
            oscore: transfer.oscore_request.map(|(_, request)| request),
            ..meta
        }
    } else {
        meta
    };
    if transfer.q_observe_confirmable() {
        if transfer.q_observe_pending_mid().is_some() || !transfer.is_complete() {
            return admission::send_refusal(
                engine,
                io,
                ids,
                now_ms,
                SendResponse {
                    block1: None,
                    block2: None,
                    q_block2: None,
                    q_request: None,
                    ..meta
                },
                oscore,
                closed,
            );
        }
        let mut retained = transfer;
        let selection = if let Some(request) = meta.q_request {
            retained.select_q_observe_recovery(
                request.q_block2().map(|q| q.expect("validated ingress")),
            )
        } else {
            retained.select_q_observe_recovery(core::iter::once(q))
        };
        if let Err(error) = selection {
            if error == BlockTransferError::OutsideWindow {
                return admission::send_refusal(
                    engine,
                    io,
                    ids,
                    now_ms,
                    SendResponse {
                        block1: None,
                        block2: None,
                        q_block2: None,
                        q_request: None,
                        ..meta
                    },
                    oscore,
                    closed,
                );
            }
            return Err(Error::Block(error));
        }
        #[cfg(feature = "oscore")]
        {
            retained.oscore_request = meta.oscore.map(|request| (meta.token, request));
        }
        engine.storage_mut().set_tx_body_transfer(body, retained)?;
        let mid = q_response_mid(engine, io, ids, now_ms, meta, ty, closed)?;
        return issue_q_window(
            engine,
            io,
            ids,
            now_ms,
            SendResponse { mid, ..meta },
            &response,
            Type::Confirmable,
            body,
            oscore,
            closed,
        );
    }
    // The complete immutable snapshot already owns every range. A Continue
    // selects one bounded window, including after an initial M=0 selection;
    // it need not infer delivery from the server's local issue bitmap.
    issue_q_selection(
        engine, io, ids, now_ms, meta, &response, ty, body, oscore, closed,
    )
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(start_outgoing_async, bound(crate::app::async_server::ProtocolIo), calls(issue_borrowed_classic => issue_borrowed_classic_async, issue_observe_preferred => issue_observe_preferred_async, issue_classic => issue_classic_async, issue_q_selection => issue_q_selection_async, issue_q_window => issue_q_window_async))]
fn start_outgoing<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    key: BlockKey<S::Token>,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
    allow_borrowed: bool,
    visible: Option<OutgoingVisibility<'_, S>>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    if allow_borrowed
        && meta.q_block2.is_none()
        && !response.fresh_piv()
        && response.observe_seq().is_none()
    {
        return issue_borrowed_classic(
            engine,
            io,
            meta,
            response,
            ty,
            key,
            now_ms,
            oscore_ctx,
            dedup_closed,
        );
    }
    let szx = szx_for(meta.block2, meta.q_block2);
    let started = if meta.q_block2.is_some() {
        let etag = response
            .etag_bytes()
            .ok_or(Error::Block(BlockTransferError::MissingIdentity))?;
        let key = key.with_identity(
            crate::storage::BodyTag::new(etag)
                .map_err(BlockTransferError::from)
                .map_err(Error::Block)?,
        );
        engine.start_q_block2(key, response.payload(), szx)
    } else {
        let key = response_block_key(key.token(), key.endpoint(), response)?;
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
    let mut body_owner = async_server::BodyOwner::outgoing(engine, id)
        .with_before_release(visible.as_ref().map(|(_, hook)| *hook));
    let engine = &mut *body_owner.engine;
    if meta.q_block2.is_some()
        && (response.observe_seq().is_some() || !response.code().is_success())
    {
        let sequence = response.observe_seq().unwrap_or(0);
        let mut transfer = engine.tx_body_transfer(id).expect("started body");
        transfer.remember_notification(response.code(), response.format(), response.max_age_secs());
        transfer.remember_q_observe(
            sequence,
            now_ms.saturating_add(u64::from(Transmission::EXCHANGE_LIFETIME_MS)),
        );
        #[cfg(feature = "oscore")]
        {
            transfer.oscore_request = meta.oscore.map(|request| (meta.token, request));
        }
        engine
            .storage_mut()
            .set_tx_body_transfer(id, transfer)
            .map_err(Error::Slot)?;
        let tx = encode_q_observe_first::<S, T, REQUESTS>(
            engine, meta, response, ty, meta.mid, id, oscore_ctx,
        )?;
        engine.release_tx(tx)?;
    }
    let selective = meta.q_block2.is_some_and(|q| !q.more() || q.num() != 0)
        || meta
            .q_request
            .is_some_and(|p| p.q_block2().nth(1).is_some());
    let outcome = if selective {
        issue_q_selection(
            engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            ty,
            id,
            oscore_ctx,
            dedup_closed,
        )
    } else if meta.q_block2.is_some() {
        issue_q_window(
            engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            ty,
            id,
            oscore_ctx,
            dedup_closed,
        )
    } else if response.observe_seq().is_some() && meta.block2.is_some_and(|b| b.num() == 0) {
        issue_observe_preferred(
            engine,
            io,
            meta,
            response,
            ty,
            id,
            (ty == Type::Confirmable).then_some((now_ms, meta.mid, ids.jitter())),
            Some((now_ms, dedup_closed)),
            oscore_ctx,
            szx,
        )
    } else {
        issue_classic(
            engine,
            io,
            meta,
            response,
            ty,
            meta.mid,
            id,
            (ty == Type::Confirmable).then_some((now_ms, meta.mid, ids.jitter())),
            response.fresh_piv() || response.observe_seq().is_some(),
            Some((now_ms, dedup_closed)),
            oscore_ctx,
        )
    };
    // Preserve packet commitment before error cleanup releases its Q body.
    // This caller-owned witness is protocol state, independent of diagnostics.
    if let Some((visible, _)) = visible {
        *visible |= engine
            .tx_body_transfer(id)
            .is_some_and(|transfer| transfer.q_observe_last_mid().is_some());
    }
    // Selective requests are independent exchanges. A fresh handler body is
    // only a temporary snapshot; retaining it would leave an unadvanced window
    // occupying a body slot forever. Existing full-transfer state is handled
    // separately by continue_outgoing and survives recovery selections.
    if outcome.is_err()
        || (selective
            && engine
                .tx_body_transfer(id)
                .is_none_or(|body| body.q_observe_sequence().is_none()))
    {
        let _ = engine.release_tx_body(id);
    }
    body_owner.disarm();
    outcome
}

/// Encodes an ordinary independently selected block from the handler's borrow.
///
/// Body capacity and pool admission retain the existing bounded policy. The
/// borrow ends during this call; only encoded TX bytes survive for replay.
/// Observe, deferred replies and Q-Block continue to own complete snapshots.
#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_borrowed_classic_async, bound(crate::app::async_server::ProtocolIo), calls(send_pinned_tx => send_pinned_tx_async, finish_send => finish_send_async))]
fn issue_borrowed_classic<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    key: BlockKey<S::Token>,
    now_ms: u64,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let key = response_block_key(key.token(), key.endpoint(), response)?;
    let capacity =
        engine
            .capacities()
            .tx_body_bytes
            .ok_or(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )))?;
    crate::storage::BlockTransfer::outgoing(
        key,
        BlockRole::OutgoingBlock2,
        response.payload().len(),
        szx_for(meta.block2, None),
        capacity,
    )
    .map_err(Error::Block)?;
    let body = engine
        .acquire_tx_body()
        .ok_or(Error::Block(BlockTransferError::Saturated))?;
    let _ = engine.release_tx_body(body);
    let requested = meta.block2.unwrap_or(
        BlockValue::new(0, false, BlockValue::SZX_MAX)
            .map_err(BlockTransferError::from)
            .map_err(Error::Block)?,
    );
    let (selected, payload, block) =
        match crate::storage::classic_block2_range(response.payload().len(), requested) {
            Ok((value, range)) => (
                *response,
                &response.payload()[range],
                Some(BlockOpt {
                    value,
                    q_block: false,
                    size2: None,
                }),
            ),
            Err(BlockTransferError::Gap) => (Response::bad_request(), &[][..], None),
            Err(error) => return Err(Error::Block(error)),
        };
    let tx = if block.is_some() {
        acquire_tx_or_evict(engine)
    } else {
        admission::ordinary_tx(engine)
    }
    .ok_or(Error::Saturated)?;
    if let Err(error) = encode_response(
        engine,
        meta.dest,
        tx,
        ty,
        meta.mid,
        meta.token,
        &selected,
        payload,
        block,
        meta.block1,
        oscore_ctx,
        meta.oscore,
    ) {
        let _ = engine.release_tx(tx);
        return Err(error);
    }
    let keep = if block.is_some() && ty == Type::Acknowledgement {
        remember_tx_reply(
            engine,
            tx,
            meta.dest,
            meta.mid,
            meta.ty,
            now_ms,
            meta.request,
            dedup_closed,
        )
    } else {
        KeepTx::No
    };
    match keep {
        KeepTx::Yes => send_pinned_tx(engine, io, tx, meta.dest),
        KeepTx::No => finish_send(engine, io, tx, meta.dest, None),
    }
}

fn retained_body_matches<S: Storage + BodySlots>(
    engine: &mut Engine<S>,
    id: SlotId,
    payload: &[u8],
) -> bool {
    #[cfg(feature = "diagnostics")]
    {
        let candidates = engine
            .tx_body_payload(id)
            .filter(|body| body.len() == payload.len())
            .map_or(0, <[u8]>::len);
        let work = engine.work_metrics_mut();
        crate::storage::WorkMetrics::add(&mut work.body_compare_calls, 1);
        crate::storage::WorkMetrics::add(&mut work.body_compare_candidate_bytes, candidates);
    }
    engine.tx_body_payload(id) == Some(payload)
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(continue_outgoing_async, bound(crate::app::async_server::ProtocolIo), calls(issue_classic => issue_classic_async, issue_q_selection => issue_q_selection_async, issue_q_window => issue_q_window_async))]
fn continue_outgoing<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    id: SlotId,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let role = engine
        .tx_body_transfer(id)
        .map(|t| t.role())
        .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
    match role {
        BlockRole::OutgoingQBlock2 => {
            let transfer = engine
                .tx_body_transfer(id)
                .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
            if transfer.identity().as_slice() != response.etag_bytes()
                || !retained_body_matches(engine, id, response.payload())
            {
                return Err(Error::Block(BlockTransferError::IdentityMismatch));
            }
            if meta.q_block2.is_some_and(|q| q.szx() != transfer.szx()) {
                return Err(Error::Block(BlockTransferError::SzxMismatch));
            }
            match meta.q_block2 {
                Some(q)
                    if !q.more()
                        // NUM=0 requests the body, never acknowledges a set.
                        // Repeated Continue requests reselect an already issued
                        // set without acknowledging or advancing it again.
                        || q.num() <= transfer.window_base()
                        || meta
                            .q_request
                            .is_some_and(|p| p.q_block2().nth(1).is_some())
                        || q.num() % u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS)
                            != 0 =>
                {
                    issue_q_selection(
                        engine,
                        io,
                        ids,
                        now_ms,
                        meta,
                        response,
                        ty,
                        id,
                        oscore_ctx,
                        dedup_closed,
                    )
                }
                Some(q) => {
                    engine.ack_q_block2(id, q.num()).map_err(Error::Block)?;
                    issue_q_window(
                        engine,
                        io,
                        ids,
                        now_ms,
                        meta,
                        response,
                        ty,
                        id,
                        oscore_ctx,
                        dedup_closed,
                    )
                }
                None => issue_q_window(
                    engine,
                    io,
                    ids,
                    now_ms,
                    meta,
                    response,
                    ty,
                    id,
                    oscore_ctx,
                    dedup_closed,
                ),
            }
        }
        BlockRole::OutgoingBlock2 => {
            let transfer = engine
                .tx_body_transfer(id)
                .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
            if transfer.identity().as_slice() != response.etag_bytes()
                || !retained_body_matches(engine, id, response.payload())
            {
                return Err(Error::Block(BlockTransferError::IdentityMismatch));
            }
            let retained = match transfer.notification_metadata() {
                Some((_, format, max_age)) => {
                    response.retain_format(format).retain_max_age(max_age)
                }
                None => *response,
            };
            issue_classic(
                engine,
                io,
                meta,
                &retained,
                ty,
                meta.mid,
                id,
                None,
                true,
                Some((now_ms, dedup_closed)),
                oscore_ctx,
            )
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_observe_preferred_async, bound(crate::app::async_server::ProtocolIo), calls(issue_classic => issue_classic_async))]
fn issue_observe_preferred<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    id: SlotId,
    pending: Option<(u64, MessageId, u32)>,
    mut replay: Option<(u64, &mut Option<DedupClosed>)>,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    maximum: u8,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    for szx in (0..=maximum).rev() {
        let selected = SendResponse {
            block2: Some(BlockValue::new(0, false, szx).expect("bounded SZX")),
            ..meta
        };
        let result = issue_classic(
            engine,
            io,
            selected,
            response,
            ty,
            meta.mid,
            id,
            pending,
            true,
            replay.as_mut().map(|(now, closed)| (*now, &mut **closed)),
            oscore,
        );
        match result {
            Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)))
                if szx > 0 => {}
            other => return other,
        }
    }
    unreachable!("finite SZX search returns at zero")
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_classic_async, bound(crate::app::async_server::ProtocolIo), calls(send_pinned_tx => send_pinned_tx_async, finish_send => finish_send_async))]
fn issue_classic<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    mid: MessageId,
    id: SlotId,
    pending: Option<(u64, MessageId, u32)>,
    retain_incomplete: bool,
    replay: Option<(u64, &mut Option<DedupClosed>)>,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let range = match meta.block2 {
        Some(requested) => engine.requested_block2(id, requested),
        None => engine.next_block2(id),
    };
    let issued = match range {
        Ok(issued) => issued,
        Err(BlockTransferError::Gap) => {
            let _ = engine.release_tx_body(id);
            let tx = admission::ordinary_tx(engine).ok_or(Error::Saturated)?;
            let error = Response::bad_request();
            if let Err(e) = encode_response(
                engine,
                meta.dest,
                tx,
                ty,
                mid,
                meta.token,
                &error,
                error.payload(),
                None,
                meta.block1,
                oscore_ctx,
                meta.oscore,
            ) {
                let _ = engine.release_tx(tx);
                return Err(e);
            }
            return finish_send(engine, io, tx, meta.dest, pending);
        }
        Err(e) => return Err(Error::Block(e)),
    };
    let tx = encode_issued::<S, T, REQUESTS>(
        engine, meta, response, ty, mid, issued, false, oscore_ctx,
    )?;
    let keep = if let Some((now_ms, closed)) = replay.filter(|_| ty == Type::Acknowledgement) {
        remember_tx_reply(
            engine,
            tx,
            meta.dest,
            mid,
            meta.ty,
            now_ms,
            meta.request,
            closed,
        )
    } else {
        KeepTx::No
    };
    let cached = engine
        .lookup_dedup(DedupKey::new(mid, meta.dest))
        .and_then(|row| engine.dedup_entry(row))
        .is_some_and(|row| row.tx_pin() == Some(tx) || row.replay().is_some());
    // Once the final reply is retained, retry uses those exact wire bytes and
    // bypasses this body path. Reclaim now even if the first send fails.
    if issued.complete() && cached {
        let _ = engine.release_tx_body(id);
    }
    match keep {
        KeepTx::Yes => send_pinned_tx(engine, io, tx, meta.dest),
        KeepTx::No => finish_send(engine, io, tx, meta.dest, pending),
    }?;
    // Ordinary classic requests reconstruct independent snapshots. Observe
    // snapshots survive every incomplete follow-up, whose response correctly
    // omits Observe, until the last block is sent or lifecycle cleanup runs.
    if issued.complete() || !retain_incomplete {
        let _ = engine.release_tx_body(id);
    }
    Ok(())
}

// RFC 9177 section 4.4: M=0 asks for exactly NUM; a non-aligned
// M=1 asks for NUM through the end of its MAX_PAYLOADS_SET. Reissuing
// selections never advances or acknowledges a retained transfer's window.
#[coaptic_io::dual_io(q_response_mid_async, bound(crate::app::async_server::ProtocolIo), calls(send_empty_ack => send_empty_ack_async))]
fn q_response_mid<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    first_ty: Type,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<MessageId, Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + DedupSlots,
    T: DatagramIo,
{
    if first_ty == Type::Acknowledgement {
        // Preflight has succeeded. Cache before I/O so duplicate protected
        // requests can retry the ACK without re-entering the handler.
        remember_empty_ack(
            engine,
            meta.dest,
            meta.mid,
            now_ms,
            meta.request,
            dedup_closed,
        );
        send_empty_ack(engine, io, meta.dest, meta.mid)?;
        ids.next_for(engine, io, now_ms)
    } else {
        Ok(meta.mid)
    }
}

// Deferred Q responses retain an immutable full snapshot until their deadline.
// A requested SZX selects byte ranges; it never mutates a transfer window or
// releases the snapshot after a last block, so later recovery can reselect it.
#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(complete_deferred_q_async, bound(crate::app::async_server::ProtocolIo), calls(issue_deferred_q => issue_deferred_q_async))]
fn complete_deferred_q<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    selectors: &DeferredQSelectors,
    snapshot: &mut Option<(SlotId, client::ReplyMeta)>,
    metadata: client::ReplyMeta,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    if meta.no_response.suppresses(response.code()) {
        return Ok(());
    }
    let plan = plan_deferred_q(selectors.iter(), response.payload().len()).map_err(Error::Block)?;
    let key = response_block_key(meta.token, meta.dest, response)?;
    if response.etag_bytes().is_none() {
        return Err(Error::Block(BlockTransferError::MissingIdentity));
    }
    let existing = response_body_for(engine, key);
    let body = if let Some(body) = existing {
        let transfer = engine
            .tx_body_transfer(body)
            .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
        if transfer.role() != BlockRole::OutgoingQBlock2
            || transfer.identity().as_slice() != response.etag_bytes()
            || !retained_body_matches(engine, body, response.payload())
        {
            return Err(Error::Block(BlockTransferError::IdentityMismatch));
        }
        body
    } else {
        engine
            .start_q_block2(
                key,
                response.payload(),
                meta.q_block2.expect("Q deferred request").szx(),
            )
            .map_err(Error::Block)?
    };
    // The deferred row owns the immutable representation before any send can
    // suspend. Cancellation after a partial window must retain those same bytes
    // and headers until retry, explicit cancellation, or deadline cleanup.
    *snapshot = Some((body, snapshot.map_or(metadata, |(_, retained)| retained)));
    let meta = SendResponse {
        mid: ids.next_for(engine, io, now_ms)?,
        ..meta
    };
    issue_deferred_q(
        engine,
        io,
        ids,
        now_ms,
        meta,
        response,
        Type::NonConfirmable,
        body,
        Some(plan),
        oscore,
        closed,
    )
}
struct DeferredQPlan {
    selected: [u32; crate::storage::BlockTransfer::MAX_PAYLOADS as usize],
    used: usize,
}
fn plan_deferred_q(
    values: impl Iterator<Item = BlockValue>,
    length: usize,
) -> Result<DeferredQPlan, BlockTransferError> {
    let mut plan = DeferredQPlan {
        selected: [0; crate::storage::BlockTransfer::MAX_PAYLOADS as usize],
        used: 0,
    };
    for value in values {
        let size = usize::from(value.size());
        let start = value.num() as usize * size;
        if start >= length && !(start == 0 && length == 0) {
            return Err(BlockTransferError::Gap);
        }
        let count = if value.more() {
            u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS)
                - value.num() % u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS)
        } else {
            1
        };
        for step in 0..count {
            let num = value.num() + step;
            if num as usize * size >= length && step != 0 {
                break;
            }
            if plan.used != 0 && num <= plan.selected[plan.used - 1] {
                continue;
            }
            if plan.used == plan.selected.len() {
                return Err(BlockTransferError::Gap);
            }
            plan.selected[plan.used] = num;
            plan.used += 1;
        }
    }
    Ok(plan)
}
#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_deferred_q_async, bound(crate::app::async_server::ProtocolIo), calls(q_response_mid => q_response_mid_async, finish_send => finish_send_async))]
fn issue_deferred_q<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    first_ty: Type,
    body: SlotId,
    plan: Option<DeferredQPlan>,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let q = meta.q_block2.expect("Q selection");
    let length = engine
        .tx_body_payload(body)
        .ok_or(Error::Block(BlockTransferError::NoTransfer))?
        .len();
    let size = usize::from(q.size());
    let plan = match plan {
        Some(plan) => plan,
        None => if let Some(request) = meta.q_request {
            plan_deferred_q(
                request
                    .q_block2()
                    .map(|value| value.expect("validated ingress")),
                length,
            )
        } else {
            plan_deferred_q(core::iter::once(q), length)
        }
        .map_err(Error::Block)?,
    };
    for (step, &num) in plan.selected[..plan.used].iter().enumerate() {
        let offset = num as usize * size;
        if offset >= length && step != 0 {
            break;
        }
        let end = (offset + size).min(length);
        let mut scratch = [0u8; 1024];
        scratch[..end - offset].copy_from_slice(
            &engine
                .tx_body_payload(body)
                .ok_or(Error::Block(BlockTransferError::NoTransfer))?[offset..end],
        );
        let value = crate::message::BlockValue::new(num, end < length, q.szx())
            .map_err(|e| Error::Block(e.into()))?;
        let mid = if step == 0 {
            q_response_mid(engine, io, ids, now_ms, meta, first_ty, closed)?
        } else {
            ids.next_for(engine, io, now_ms)?
        };
        let tx = acquire_tx_or_evict(engine).ok_or(Error::Saturated)?;
        if let Err(error) = encode_response(
            engine,
            meta.dest,
            tx,
            Type::NonConfirmable,
            mid,
            meta.token,
            response,
            &scratch[..end - offset],
            Some(BlockOpt {
                value,
                q_block: true,
                size2: Some(length),
            }),
            meta.block1,
            oscore,
            meta.oscore,
        ) {
            let _ = engine.release_tx(tx);
            return Err(error);
        }
        finish_send(engine, io, tx, meta.dest, None)?;
        if let Some(mut transfer) = engine.tx_body_transfer(body)
            && transfer.q_observe_sequence().is_some()
        {
            transfer.set_q_observe_last_mid(Some(mid));
            engine
                .storage_mut()
                .set_tx_body_transfer(body, transfer)
                .map_err(|_| Error::CommitAfterSend)?;
        }
        if end == length {
            break;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_q_selection_async, bound(crate::app::async_server::ProtocolIo), calls(q_response_mid => q_response_mid_async, send_issued => send_issued_async))]
fn issue_q_selection<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    first_ty: Type,
    id: SlotId,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let selections = || meta.q_request.into_iter().flat_map(ParsedMessage::q_block2);
    // Validate every requested starting range before the first send, so a
    // bad later option cannot turn a rejected request into partial output.
    for q in selections() {
        let q = q.map_err(|e| Error::Block(e.into()))?;
        engine.reissue_q_block2(id, q.num()).map_err(Error::Block)?;
    }
    let mut last_sent = None;
    for q in selections() {
        let q = q.map_err(|e| Error::Block(e.into()))?;
        let count = if q.more() {
            u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS)
                - q.num() % u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS)
        } else {
            1
        };
        for offset in 0..count {
            let num = q.num() + offset;
            if last_sent.is_some_and(|last| num <= last) {
                continue;
            }
            let issued = engine.reissue_q_block2(id, num).map_err(Error::Block)?;
            let mid = if last_sent.is_none() {
                q_response_mid(engine, io, ids, now_ms, meta, first_ty, dedup_closed)?
            } else {
                ids.next_for(engine, io, now_ms)?
            };
            let ty = Type::NonConfirmable;
            send_issued(
                engine, io, meta, response, ty, mid, issued, true, None, oscore_ctx,
            )?;
            if let Some(mut transfer) = engine.tx_body_transfer(id)
                && transfer.q_observe_sequence().is_some()
            {
                transfer.set_q_observe_last_mid(Some(mid));
                engine
                    .storage_mut()
                    .set_tx_body_transfer(id, transfer)
                    .map_err(|_| Error::CommitAfterSend)?;
            }
            last_sent = Some(num);
            if !issued.block().more() {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_q_window_async, bound(crate::app::async_server::ProtocolIo), calls(q_response_mid => q_response_mid_async, send_issued => send_issued_async, commit_q_window_send => async_server::commit_q_window_send_async))]
fn issue_q_window<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    first_ty: Type,
    id: SlotId,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let retained_observe = engine
        .tx_body_transfer(id)
        .is_some_and(|transfer| transfer.q_observe_sequence().is_some());
    let confirmable = retained_observe && first_ty == Type::Confirmable;
    if confirmable {
        let mut transfer = engine
            .tx_body_transfer(id)
            .ok_or(Error::Block(BlockTransferError::NoTransfer))?;
        transfer.set_q_observe_confirmable();
        if transfer.q_observe_pending_mid().is_some() {
            return Ok(());
        }
        engine
            .storage_mut()
            .set_tx_body_transfer(id, transfer)
            .map_err(Error::Slot)?;
    }
    let mut extra = 0u16;
    loop {
        let issued = match engine.peek_q_block2(id) {
            Ok(issued) => issued,
            Err(BlockTransferError::OutsideWindow) => return Ok(()),
            Err(e) => return Err(Error::Block(e)),
        };
        let mid = if extra == 0 {
            q_response_mid(engine, io, ids, now_ms, meta, first_ty, dedup_closed)?
        } else {
            ids.next_for(engine, io, now_ms)?
        };
        let ty = if confirmable {
            Type::Confirmable
        } else {
            Type::NonConfirmable
        };
        send_issued(
            engine,
            io,
            meta,
            response,
            ty,
            mid,
            issued,
            true,
            confirmable.then_some((now_ms, mid, ids.jitter())),
            oscore_ctx,
        )?;
        commit_q_window_send(engine, issued)?;
        if retained_observe && !confirmable {
            let mut transfer = engine.tx_body_transfer(id).ok_or(Error::CommitAfterSend)?;
            transfer.set_q_observe_last_mid(Some(mid));
            engine
                .storage_mut()
                .set_tx_body_transfer(id, transfer)
                .map_err(|_| Error::CommitAfterSend)?;
        }
        if confirmable {
            let mut transfer = engine.tx_body_transfer(id).ok_or(Error::CommitAfterSend)?;
            transfer.set_q_observe_pending_mid(Some(mid));
            engine
                .storage_mut()
                .set_tx_body_transfer(id, transfer)
                .map_err(|_| Error::CommitAfterSend)?;
            return Ok(());
        }
        extra = extra.saturating_add(1);
        if issued.complete() {
            if !retained_observe {
                let _ = engine.release_tx_body(id);
            }
            return Ok(());
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_issued_async, bound(crate::app::async_server::ProtocolIo), calls(finish_send => finish_send_async))]
fn send_issued<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    mid: MessageId,
    issued: OutgoingBlock,
    q_block: bool,
    pending: Option<(u64, MessageId, u32)>,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots,
    T: DatagramIo,
{
    let tx = encode_issued::<S, T, REQUESTS>(
        engine, meta, response, ty, mid, issued, q_block, oscore_ctx,
    )?;
    finish_send(engine, io, tx, meta.dest, pending)
}

#[allow(clippy::too_many_arguments)]
fn encode_issued<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    ty: Type,
    mid: MessageId,
    issued: OutgoingBlock,
    q_block: bool,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
) -> Result<SlotId, Error<T::Error>>
where
    S: Storage + DatagramSlots + BodySlots + DedupSlots,
    T: crate::storage::DatagramTransport,
{
    let mut chunk = [0u8; 1024];
    let n = copy_issued(engine, issued, &mut chunk).map_err(Error::Block)?;
    let size2 = if q_block {
        engine.tx_body_transfer(issued.id()).map(|t| t.filled())
    } else {
        None
    };
    let Some(tx) = acquire_tx_or_evict(engine) else {
        return Err(Error::Saturated);
    };
    let block = Some(BlockOpt {
        value: issued.block(),
        q_block,
        size2,
    });
    if let Err(e) = encode_response(
        engine,
        meta.dest,
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
        return Err(e);
    }
    Ok(tx)
}

fn copy_issued<S: Storage + BodySlots>(
    engine: &mut Engine<S>,
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
    #[cfg(feature = "diagnostics")]
    crate::storage::WorkMetrics::add(
        &mut engine.work_metrics_mut().block_tx_staged_bytes,
        issued.len(),
    );
    Ok(issued.len())
}

#[allow(clippy::too_many_arguments)]
fn encode_response<S: Storage + DatagramSlots, E, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    peer: Endpoint,
    tx: SlotId,
    ty: Type,
    mid: MessageId,
    token: S::Token,
    response: &Response<'_, S::Token>,
    payload: &[u8],
    block: Option<BlockOpt>,
    block1: Option<BlockOpt>,
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    oscore_req: oscore::Request,
) -> Result<(), Error<E>> {
    response.validate().map_err(Error::Response)?;
    let cf = response.format().map(crate::ContentFormat::encode);
    let max_age = response.max_age_secs().map(EncodedUint::new);
    let observe = response
        .observe_seq()
        .map(|seq| encode_uint(seq & 0x00ff_ffff));
    let block_enc = block.map(|b| b.value.encode());
    let size2_enc = block
        .and_then(|b| b.size2)
        .map(|n| encode_uint(u32::try_from(n).unwrap_or(u32::MAX)));
    let block1_enc = block1.map(|b| b.value.encode());
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
            push_opt(
                &mut opts,
                if block1.is_some_and(|b| b.q_block) {
                    Opt::q_block1(encoded)
                } else {
                    Opt::block1(encoded)
                },
            )?;
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
            if response.fresh_piv()
                || block.is_some_and(|b| b.q_block)
                || (response.observe_seq().is_some() && ty != Type::Acknowledgement)
            {
                oscore::encode_notification(oscore_ctx, peer, oscore_req, engine, tx, &msg)
            } else {
                oscore::encode_message(oscore_ctx, peer, oscore_req, engine, tx, &msg)
            }
        }
        Err(EncodeError::OptionsFull) => {
            encode_options_full_500(engine, peer, tx, ty, mid, token, oscore_ctx, oscore_req)
        }
        Err(e) => Err(Error::Message(SlotMessageError::Encode(e))),
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_notification<S: Storage + DatagramSlots, E, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    peer: Endpoint,
    tx: SlotId,
    ty: Type,
    mid: MessageId,
    token: S::Token,
    response: &Response<'_, S::Token>,
    payload: &[u8],
    oscore_ctx: &mut oscore::Field<REQUESTS, S::Token>,
    oscore_req: oscore::Request,
) -> Result<(), Error<E>> {
    response.validate().map_err(Error::Response)?;
    let cf = response.format().map(crate::ContentFormat::encode);
    let max_age = response.max_age_secs().map(EncodedUint::new);
    let observe = response
        .observe_seq()
        .map(|seq| encode_uint(seq & 0x00ff_ffff));
    let mut opts = OptionsBuilder::<{ 8 + 2 * LOCATION_MAX }>::new();
    let filled = (|| -> Result<(), EncodeError> {
        if let Some(etag) = response.etag_bytes() {
            push_opt(&mut opts, Opt::etag(etag))?;
        }
        if let Some(ref encoded) = observe {
            push_opt(&mut opts, Opt::observe(encoded))?;
        }
        if let Some(ref encoded) = cf {
            push_opt(&mut opts, Opt::content_format(encoded))?;
        }
        if let Some(ref encoded) = max_age {
            push_opt(&mut opts, Opt::max_age(encoded))?;
        }
        Ok(())
    })();
    match filled {
        Ok(()) => {
            let msg = Message::new(ty, response.code(), mid)
                .with_token(token)
                .with_options(opts.as_slice())
                .with_payload(payload);
            oscore::encode_notification(oscore_ctx, peer, oscore_req, engine, tx, &msg)
        }
        Err(EncodeError::OptionsFull) => {
            encode_options_full_500(engine, peer, tx, ty, mid, token, oscore_ctx, oscore_req)
        }
        Err(e) => Err(Error::Message(SlotMessageError::Encode(e))),
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

/// 5.00 when the response option list cannot be built.
///
/// Under attached OSCORE this is OSCORE-protected. It must not fall back
/// to unprotected traffic.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_options_full_500<S: Storage + DatagramSlots, E, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    peer: Endpoint,
    tx: SlotId,
    ty: Type,
    mid: MessageId,
    token: S::Token,
    oscore_ctx: &oscore::Field<REQUESTS, S::Token>,
    oscore_req: oscore::Request,
) -> Result<(), Error<E>> {
    let response: Response<'_, S::Token> =
        Response::problem(Code::INTERNAL_SERVER_ERROR).title("Options full");
    response.validate().map_err(Error::Response)?;
    let cf = response.format().map(crate::ContentFormat::encode);
    let mut opts = OptionsBuilder::<4>::new();
    if let Some(ref encoded) = cf {
        push_opt(&mut opts, Opt::content_format(encoded))
            .map_err(|e| Error::Message(SlotMessageError::Encode(e)))?;
    }
    let msg = Message::new(ty, response.code(), mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(response.payload());
    oscore::encode_message(oscore_ctx, peer, oscore_req, engine, tx, &msg)
}

enum Replay<E> {
    Hit(Result<(), Error<E>>),
    Miss,
}

enum KeepTx {
    Yes,
    No,
}

/// Last POST/PATCH/FETCH whose Dedup insert failed. Compact: one key.
#[derive(Clone, Copy)]
struct DedupClosed {
    key: DedupKey,
    due_ms: u64,
}

/// GET / PUT / DELETE / iPATCH may re-run when a live Dedup row has no
/// replay (RFC 7252 §4.5 MAY). POST / PATCH / FETCH never re-run.
fn request_may_rerun_on_duplicate(code: Code) -> bool {
    matches!(code, Code::GET | Code::PUT | Code::DELETE | Code::IPATCH)
}

#[coaptic_io::dual_io(replay_con_request_async, bound(crate::app::async_server::ProtocolIo), calls(replay_closed => replay_closed_async, send_replay_bytes => send_replay_bytes_async, send_empty_ack => send_empty_ack_async), methods(send_tx => send_tx_async))]
fn replay_con_request<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    peer: Endpoint,
    parsed: &ParsedMessage<'_, S::Token>,
    now_ms: u64,
    dedup_closed: &mut Option<DedupClosed>,
) -> Replay<T::Error>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    let key = DedupKey::new(parsed.message_id(), peer);
    let Some(id) = engine.lookup_dedup(key) else {
        return replay_closed(engine, io, peer, parsed, now_ms, dedup_closed);
    };
    let Some(entry) = engine.dedup_entry(id) else {
        return replay_closed(engine, io, peer, parsed, now_ms, dedup_closed);
    };
    if entry.due_ms() != 0 && now_ms >= entry.due_ms() {
        release_dedup_row(engine, entry);
        return replay_closed(engine, io, peer, parsed, now_ms, dedup_closed);
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
        remember_empty_ack(
            engine,
            peer,
            parsed.message_id(),
            now_ms,
            parsed.code(),
            dedup_closed,
        );
        Replay::Hit(send_empty_ack(engine, io, peer, parsed.message_id()))
    }
}

/// POST / PATCH / FETCH: insert failure after the first response is
/// fail-closed (empty ACK), not Miss.
#[coaptic_io::dual_io(replay_closed_async, bound(crate::app::async_server::ProtocolIo), calls(send_empty_ack => send_empty_ack_async))]
fn replay_closed<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    peer: Endpoint,
    parsed: &ParsedMessage<'_, S::Token>,
    now_ms: u64,
    dedup_closed: &mut Option<DedupClosed>,
) -> Replay<T::Error>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    if request_may_rerun_on_duplicate(parsed.code()) {
        return Replay::Miss;
    }
    let Some(closed) = *dedup_closed else {
        return Replay::Miss;
    };
    if closed.key != DedupKey::new(parsed.message_id(), peer) {
        return Replay::Miss;
    }
    if closed.due_ms != 0 && now_ms >= closed.due_ms {
        *dedup_closed = None;
        return Replay::Miss;
    }
    remember_empty_ack(
        engine,
        peer,
        parsed.message_id(),
        now_ms,
        parsed.code(),
        dedup_closed,
    );
    Replay::Hit(send_empty_ack(engine, io, peer, parsed.message_id()))
}

#[coaptic_io::dual_io(send_replay_bytes_async, bound(crate::app::async_server::ProtocolIo), methods(send_tx => send_owned_packet_async))]
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
    let Some(tx) = admission::control_tx(engine) else {
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

#[coaptic_io::dual_io(send_pinned_tx_async, bound(crate::app::async_server::ProtocolIo), methods(send_tx => send_tx_async))]
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

#[allow(clippy::too_many_arguments)]
fn remember_tx_reply<S: Storage + DatagramSlots + DedupSlots>(
    engine: &mut Engine<S>,
    tx: SlotId,
    dest: Endpoint,
    mid: MessageId,
    request_ty: Type,
    now_ms: u64,
    request: Code,
    dedup_closed: &mut Option<DedupClosed>,
) -> KeepTx {
    if request_ty != Type::Confirmable {
        return KeepTx::No;
    }
    let due = dedup_due_ms(now_ms);
    let mut entry = DedupEntry::new(mid, dest).with_due_ms(due);
    let inline = {
        let Ok(access) = engine.access_tx(tx) else {
            remember_empty_ack(engine, dest, mid, now_ms, request, dedup_closed);
            return KeepTx::No;
        };
        let slice = access.as_bytes();
        if slice.len() <= DedupEntry::REPLAY_MAX {
            entry = entry.with_replay(slice);
            true
        } else {
            entry = entry.with_tx_pin(tx);
            false
        }
    };
    if note_dedup_store(engine, entry, request, dedup_closed) && !inline {
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
    request: Code,
    dedup_closed: &mut Option<DedupClosed>,
) {
    let mut buf = [0u8; 16];
    let Ok(n) = encode(&Message::empty_ack(mid), &mut buf) else {
        if !request_may_rerun_on_duplicate(request)
            && dedup_closed
                .as_ref()
                .is_none_or(|c| c.key != DedupKey::new(mid, dest))
        {
            *dedup_closed = Some(DedupClosed {
                key: DedupKey::new(mid, dest),
                due_ms: dedup_due_ms(now_ms),
            });
        }
        return;
    };
    let _ = note_dedup_store(
        engine,
        DedupEntry::new(mid, dest)
            .with_due_ms(dedup_due_ms(now_ms))
            .with_replay(&buf[..n]),
        request,
        dedup_closed,
    );
}

fn note_dedup_store<S: Storage + DedupSlots>(
    engine: &mut Engine<S>,
    entry: DedupEntry,
    request: Code,
    dedup_closed: &mut Option<DedupClosed>,
) -> bool {
    let key = entry.key();
    let due_ms = entry.due_ms();
    if store_request_dedup(engine, entry) {
        if dedup_closed.as_ref().is_some_and(|c| c.key == key) {
            *dedup_closed = None;
        }
        true
    } else {
        if !request_may_rerun_on_duplicate(request) {
            let already = dedup_closed.as_ref().is_some_and(|c| c.key == key);
            if !already {
                *dedup_closed = Some(DedupClosed { key, due_ms });
            }
        }
        false
    }
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

fn expire_request_dedup<S: Storage + DedupSlots>(
    engine: &mut Engine<S>,
    dedup_closed: &mut Option<DedupClosed>,
    now_ms: u64,
) {
    if let Some(closed) = *dedup_closed
        && closed.due_ms != 0
        && now_ms >= closed.due_ms
    {
        *dedup_closed = None;
    }
    if engine.storage_mut().dedup().is_empty() {
        return;
    }
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
    for _ in 0..=admission::TX_HEADROOM {
        if admission::free_tx(engine) > admission::TX_HEADROOM || !evict_one_dedup_pin(engine) {
            break;
        }
    }
    admission::ordinary_tx(engine)
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
    pending: Option<(u64, MessageId, u32)>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons,
    T: DatagramIo,
{
    send::finish(engine, io, tx, dest, pending)
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(oscore_inbound_error_async, bound(crate::app::async_server::ProtocolIo), calls(send_response => send_response_async))]
fn oscore_inbound_error<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    parsed: &crate::message::ParsedMessage<'_, S::Token>,
    peer: Endpoint,
    now_ms: u64,
    err: oscore::InboundError,
    dedup_closed: &mut Option<DedupClosed>,
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
            q_request: None,
            block1: None,
            oscore: oscore::no_request(),
            request: parsed.code(),
        };
        send_response(
            engine,
            io,
            ids,
            meta,
            // Unprotected OSCORE processing error: Max-Age 0 so a
            // cacheable 4.01 does not stick in intermediaries
            // (`knowledge/rfcs/rfc8613.txt` §8.2 / §4.1.3.1).
            &Response::problem(Code::UNAUTHORIZED)
                .title("Unauthorized")
                .max_age(0),
            now_ms,
            &mut oscore::empty_field::<4, S::Token>(),
            dedup_closed,
        )
    }
    #[cfg(not(feature = "oscore"))]
    {
        let _ = (engine, io, ids, parsed, peer, now_ms, err, dedup_closed);
        Ok(())
    }
}

fn unauthorized_echo<K: OwnedToken>(decision: EchoDecision) -> Response<'static, K> {
    let response = Response::problem(Code::UNAUTHORIZED)
        .title("Unauthorized")
        .max_age(0);
    match decision {
        EchoDecision::Challenge(challenge) => response.echo(challenge),
        EchoDecision::Reject | EchoDecision::Accept => response,
    }
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
struct SendResponse<'a, K: OwnedToken = crate::message::Token> {
    dest: Endpoint,
    ty: Type,
    mid: MessageId,
    token: K,
    no_response: NoResponse,
    block2: Option<BlockValue>,
    q_block2: Option<BlockValue>,
    q_request: Option<ParsedMessage<'a, K>>,
    /// Echo of the request Block1 (RFC 7959 §2.5 Continue / final).
    block1: Option<BlockOpt>,
    /// Request Partial IV when the inbound request was OSCORE-protected.
    oscore: oscore::Request,
    /// Inbound request code (Dedup fail-closed for POST / PATCH / FETCH).
    request: Code,
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
    let n = src.len();
    drop(access);
    #[cfg(feature = "diagnostics")]
    crate::storage::WorkMetrics::add(&mut engine.work_metrics_mut().app_rx_staged_bytes, n);
    Ok(n)
}

#[coaptic_io::dual_io(send_empty_ack_async, bound(crate::app::async_server::ProtocolIo), calls(send_empty => send_empty_async))]
pub(crate) fn send_empty_ack<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    mid: crate::message::MessageId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    send_empty(engine, io, dest, Message::empty_ack(mid))
}

#[coaptic_io::dual_io(send_empty_rst_async, bound(crate::app::async_server::ProtocolIo), calls(send_empty => send_empty_async))]
fn send_empty_rst<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    mid: crate::message::MessageId,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    send_empty(engine, io, dest, Message::empty_rst(mid))
}

#[coaptic_io::dual_io(send_empty_async, bound(crate::app::async_server::ProtocolIo), methods(send_tx => send_owned_packet_async))]
fn send_empty<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    msg: Message<'static, impl OwnedToken>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + DedupSlots,
    T: DatagramIo,
{
    let Some(tx) = admission::control_tx(engine) else {
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

// RFC 8974 capacity refusal: called only after a syntactically valid request exceeds S::Token capacity.
#[coaptic_io::dual_io(send_token_capacity_refusal_async, bound(crate::app::async_server::ProtocolIo), methods(send_tx => send_owned_packet_async), calls(send_empty_ack => send_empty_ack_async))]
fn send_token_capacity_refusal<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    request: crate::message::extended::Datagram<'_>,
    peer: Endpoint,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + DedupSlots,
    T: DatagramIo,
{
    if !request.code().is_request()
        || !matches!(request.ty(), Type::Confirmable | Type::NonConfirmable)
        || request
            .options()
            .any(|option| option.number() == crate::message::OptionNumber::OSCORE)
    {
        return Ok(());
    }
    // RFC 7967 error suppression remains effective even before token ownership.
    Metrics::inc(&mut engine.metrics_mut().token_capacity_refused);
    let suppressed = request.options().any(|option| {
        option.number() == crate::message::OptionNumber::NO_RESPONSE
            && option.value().len() == 1
            && option.value()[0] & 8 != 0
    });
    if suppressed {
        return if request.ty() == Type::Confirmable {
            send_empty_ack(engine, io, peer, request.message_id())
        } else {
            Ok(())
        };
    }
    let (ty, id) = if request.ty() == Type::Confirmable {
        (Type::Acknowledgement, request.message_id())
    } else {
        (Type::NonConfirmable, ids.next_for(engine, io, now_ms)?)
    };
    let tx = admission::control_tx(engine).ok_or(Error::Saturated)?;
    let encoded = engine
        .access_tx_mut(tx)
        .map_err(Error::Slot)
        .and_then(|mut access| {
            crate::message::extended::Message {
                ty,
                code: Code::BAD_REQUEST,
                id,
                token: request.token(),
                options: &[],
                payload: &[],
            }
            .encode(access.bytes_mut())
            .map_err(|error| Error::Message(error.into()))
        });
    let n = match encoded {
        Ok(n) => n,
        Err(error) => {
            let _ = engine.release_tx(tx);
            return Err(error);
        }
    };
    if let Err(error) = engine
        .storage_mut()
        .set_tx_len(tx, n)
        .and_then(|()| engine.set_tx_endpoint(tx, peer))
    {
        let _ = engine.release_tx(tx);
        return Err(Error::Slot(error));
    }
    let outcome = engine.send_tx(io, tx);
    let _ = engine.release_tx(tx);
    outcome?;
    Ok(())
}

/// Failure of [`App::poll`] or [`Outgoing::send`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error<E> {
    /// A complete datagram was accepted but its protocol state commit failed.
    /// Transmission occurred; do not automatically repeat application effects.
    CommitAfterSend,
    /// Fresh identity or randomized transmission schedule could not be obtained.
    Identity(IdentityError),
    /// Deferred handle is unknown, expired, already completed or invalid.
    DeferredUnknown,
    /// Handler/notification response contains an invalid bounded option.
    Response(ResponseError),
    /// Absolute caller deadline has already elapsed; no request was sent.
    DeadlineElapsed,
    /// A matched overload response defers this method and target. No I/O occurred.
    Backoff {
        /// Absolute monotonic millisecond time when a new request may be sent.
        until_ms: u64,
    },
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
    /// The payload requires fragmentation with ETag outside classic/Q-Block1 FETCH.
    /// Nothing has been sent. Use a payload
    /// fitting one datagram or construct a conditional transfer with [`Engine`].
    ConditionalUploadUnsupported,
    /// The upload's No-Response mask is outside the supported App combinations.
    /// Classic Block1 permits zero and error-only masks 8, 16 and 24; Q-Block1
    /// permits zero and 16 (5.xx only), plus 2 and 18 for CON uploads. Nothing was
    /// sent. Other masks need a single datagram
    /// or a caller-managed Engine transfer.
    NoResponseUploadUnsupported,
    /// Observe registration requires interest in 2.xx. Error-only masks and
    /// deregistration suppression are supported; reserved bits do not refuse registration.
    NoResponseObserveUnsupported,
    /// No live subscription matches the cancellation's request identity/Call.
    ObserveCancellationMismatch,
    /// No live subscription matches the refresh request identity and Call.
    ObserveRefreshMismatch,
    /// Observe registration/cancellation is defined only for GET and FETCH.
    ObserveMethodUnsupported,
    /// The storage backend has not implemented the required atomic replacement.
    /// Original bytes and delivery budget remain.
    ObserveSupersessionUnsupported,
    /// Multiple subscriptions match; select one with `deregister_call`.
    ObserveCancellationAmbiguous,
    /// Observe request identity exceeds the App's 512 encoded-byte bound.
    ObserveRequestTooLarge,
    /// FETCH selection identity exceeds 512 encoded bytes. Nothing was sent;
    /// use a smaller selection or retain the request with the advanced Engine.
    FetchRequestTooLarge,
    /// RFC 8132 requires Content-Format on FETCH, even with an empty payload.
    /// Nothing was sent; select the format that describes the selection body.
    FetchContentFormatRequired,
    /// RFC 9177 requires a present Request-Tag for Q-Block1.
    RequestTagRequired,
    /// Another live operation at this peer already owns this Request-Tag.
    RequestTagInUse,
    /// OSCORE protect / unprotect failed (feature `oscore`).
    #[cfg(feature = "oscore")]
    Oscore(crate::oscore::Error),
    /// Durable request policy requires a checkpoint callback; no I/O was attempted.
    #[cfg(feature = "oscore")]
    OscoreCheckpointRequired,
    /// Recipient request checkpoint did not confirm a durable commit.
    #[cfg(feature = "oscore")]
    OscoreCheckpointFailed,
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
            Self::CommitAfterSend => f.write_str("datagram accepted but protocol commit failed"),
            Self::Identity(error) => write!(f, "{error}"),
            Self::DeferredUnknown => f.write_str("deferred response handle is unavailable"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Slot(e) => write!(f, "{e}"),
            Self::Message(e) => write!(f, "{e}"),
            Self::Response(error) => write!(f, "{error}"),
            Self::DeadlineElapsed => f.write_str("call deadline already elapsed"),
            Self::Backoff { until_ms } => write!(f, "request backed off until {until_ms} ms"),
            Self::Saturated => f.write_str("a bounded table is saturated"),
            Self::Block(e) => write!(f, "{e}"),
            Self::Path => f.write_str("uri-path has too many segments"),
            Self::ObserveMethodUnsupported => f.write_str("Observe requires GET or FETCH"),
            Self::ObserveSupersessionUnsupported => {
                f.write_str("storage backend does not support this atomic Observe replacement")
            }
            Self::ObserveRefreshMismatch => {
                f.write_str("Observe refresh does not match a live subscription")
            }
            Self::ObserveCancellationMismatch => {
                f.write_str("Observe cancellation does not match a live request")
            }
            Self::ObserveCancellationAmbiguous => {
                f.write_str("Observe cancellation requires an explicit Call")
            }
            Self::ObserveRequestTooLarge => {
                f.write_str("Observe request identity exceeds 512 bytes")
            }
            Self::FetchContentFormatRequired => f.write_str("FETCH requires Content-Format"),
            Self::FetchRequestTooLarge => f.write_str("FETCH request identity exceeds 512 bytes"),
            Self::RequestTagRequired => f.write_str("Q-Block1 requires a Request-Tag"),
            Self::RequestTagInUse => f.write_str("Request-Tag already in use at this peer"),
            Self::NoResponseUploadUnsupported => {
                f.write_str("No-Response mask is unsupported for this upload combination")
            }
            Self::NoResponseObserveUnsupported => {
                f.write_str("Observe registration requires interest in 2.xx responses")
            }
            Self::ConditionalUploadUnsupported => {
                f.write_str("conditional fragmented uploads are unsupported")
            }
            #[cfg(feature = "oscore")]
            Self::Oscore(e) => write!(f, "{e}"),
            #[cfg(feature = "oscore")]
            Self::OscoreCheckpointRequired => f.write_str("OSCORE checkpoint callback is required"),
            #[cfg(feature = "oscore")]
            Self::OscoreCheckpointFailed => f.write_str("OSCORE checkpoint commit failed"),
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
            Self::Identity(error) => Some(error),
            Self::Io(e) => Some(e),
            Self::Slot(e) => Some(e),
            Self::Message(e) => Some(e),
            Self::Saturated
            | Self::CommitAfterSend
            | Self::DeadlineElapsed
            | Self::DeferredUnknown
            | Self::Backoff { .. } => None,
            Self::Block(e) => Some(e),
            Self::Response(e) => Some(e),
            Self::Path
            | Self::ConditionalUploadUnsupported
            | Self::NoResponseUploadUnsupported
            | Self::NoResponseObserveUnsupported
            | Self::ObserveMethodUnsupported
            | Self::ObserveRefreshMismatch
            | Self::ObserveCancellationMismatch
            | Self::ObserveCancellationAmbiguous
            | Self::FetchContentFormatRequired
            | Self::FetchRequestTooLarge
            | Self::ObserveRequestTooLarge
            | Self::ObserveSupersessionUnsupported
            | Self::RequestTagRequired
            | Self::RequestTagInUse => None,
            #[cfg(feature = "oscore")]
            Self::Oscore(e) => Some(e),
            #[cfg(feature = "oscore")]
            Self::OscoreCheckpointRequired | Self::OscoreCheckpointFailed => None,
        }
    }
}

impl<
    P,
    T: crate::storage::DatagramTransport,
    const N: usize,
    const BLOCK_WISE: bool,
    S: AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    /// Retire a deferred local handle without claiming remote cancellation.
    ///
    /// Caller owns accepted application work and must decide whether it can be
    /// cancelled. A previously sent empty ACK remains only proof of reception.
    pub fn cancel_deferred(&mut self, handle: DeferredReply) -> bool {
        let Some(row) = self
            .deferred
            .rows
            .iter_mut()
            .find(|row| row.is_some_and(|row| row.handle == handle && !row.completed))
        else {
            return false;
        };
        if retire_deferred_observe(&mut self.engine, row.expect("matched row")).is_err() {
            return false;
        }
        if let Some(body) = row.and_then(|row| row.snapshot.map(|(body, _)| body)) {
            let _ = self.engine.release_tx_body(body);
        }
        *row = None;
        true
    }

    #[cfg(feature = "edhoc")]
    pub(crate) fn deferred_pending(&self, handle: DeferredReply, now_ms: u64) -> bool {
        self.deferred
            .rows
            .iter()
            .flatten()
            .any(|row| row.handle == handle && !row.completed && now_ms < row.deadline_ms)
    }

    /// Mark observers of `path` due. [`Self::poll`] encodes via
    /// [`ObserveSource`] when progress surfaces the row.
    ///
    /// Returns how many rows were marked. Use [`Self::notify`] when you
    /// already have the [`Response`].
    ///
    /// For an outstanding CON notification, signals coalesce until its next
    /// retransmission deadline. A new source response replaces that packet with a fresh MID (and OSCORE Partial IV), retaining
    /// the same TX slot, retry count and backoff. Without a signal, retries use
    /// identical bytes. Terminal responses end the relation while retaining CON
    /// delivery; a Content-Format change terminates it with 4.06.
    /// Block2 replacements reuse the retained body slot and stage a new first block.
    /// Terminal bodies retain their error and selection after the row ends, until
    /// completion, replacement registration or EXCHANGE_LIFETIME expiry.
    /// Staging refusal does not consume the signal or extend the retry budget.
    ///
    /// Superseded ACKs do not complete or prolong the current chain. Up to four
    /// superseded MIDs remain on the Observe row so an exact-peer RST can cancel
    /// that chain. Current ACK, cancellation, re-registration or final timeout
    /// clears this history; it is not a history across registrations. The row's
    /// storage is included in fixed profiles and allocated heap-budget checks.
    pub fn signal(&mut self, path: &[&str]) -> usize {
        let resource = self.site.observe_resource(path);
        self.engine.signal_observe_resource(resource)
    }
}

fn refresh_progress_deadlines<S: Storage, T>(_: &Engine<S>, _: &mut T) {}
