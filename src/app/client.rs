//! Outbound App request: encode TX, match Token + peer, take a [`Response`].
//!
//! ```text
//! Outgoing --encode--> TX slot
//! poll matches Token + endpoint
//! take_response returns Some(Ok(remote)) or Some(Err(local failure))
//! RX --copy--> Response::payload  (non-Block: min(len, INLINE_PAYLOAD) = 128)
//! Block2 / Q-Block2 --apply--> RX body --copy--> Response::body()  (RESPONSE_BODY)
//! ```
//!
//! A 200-byte piggybacked 2.05 with [`.block_wise::<false>()`](super::AppBuilder::block_wise)
//! copies 128 bytes into [`Response::payload`]. Check
//! [`Response::payload_truncated`]. The full assembled representation is
//! [`Response::body`], not a 4KiB field on [`Response`].
//!
//! App defaults to four live client requests, including Observe subscriptions
//! and replies waiting for `take_response`. [`super::AppBuilder::client_calls`]
//! sizes those inline tables independently of packet storage. Other pool and
//! security limits can be lower. Call-table exhaustion is `Error::Saturated`.
//! Matched 4.29 and 5.03 replies retain method/peer/path/query backoff until
//! Max-Age expires (60 seconds when absent). `Outgoing::send` returns
//! `Error::Backoff` before I/O for that target. Other request bodies and options
//! do not change similarity. Application-defined series can impose wider limits.
//! Backoffs consume client admission capacity and retain one bounded query/path
//! copy per row; they survive response collection and cancellation. No active
//! backoff is evicted. Error-body block follow-ups are not sent during backoff.
//! Upload continuation and cleanup select request-body roles separately from
//! server response bodies, even at identical peer/Token/tag keys.
//! Each live call retains up to 256 URI-query bytes plus lengths for Block2
//! continuation identity. With the default four Calls this bounded state adds
//! about 1.1 KiB on 64-bit hosts, separate from assembled-body storage.
//! Unknown critical response options reject the response (RST for CON);
//! unknown elective options are retained in `Response::received_options`.
//! New Observe representations retain their initial Content-Format, including
//! absence. A fresh changed/malformed format terminates the Call with
//! `CallFailure::ObserveFormatMismatch` (RST for CON); stale representations do
//! not change this state. A 2.03 validation may omit Content-Format.
//! A matching ACK still stops retries. Replies retain at most 24 options and
//! 512 encoded header/Token/option bytes, with at most eight Location-Path and
//! eight Location-Query values. Overflow completes the call with
//! `CallFailure::ResponseMetadataBounds`, never a partial metadata snapshot.
//! Four reply rows plus one borrowed-response hold each contain a fixed
//! 512-byte header buffer. No allocation or additional assembled-body buffer
//! is used for metadata.
//!
//! [`Call`] is Token plus peer — not a [`SlotId`](crate::storage::SlotId).
//! [`App::poll`](super::App::poll) advances this alongside site routing on
//! the same socket. Continues reuse the path recorded at
//! [`Outgoing::send`]. Observe subscribe ([`Outgoing::observe`]) keeps the
//! same [`Call`] after the Exchange is cleared.
//! FETCH retains its encoded selection request (at most 512 bytes excluding
//! Token, Observe and ETag) in the existing per-call identity buffer. Block2
//! and Q-Block2 follow-ups preserve its body, Content-Format and conditions.
//! Download follow-ups retain If-Match, If-None-Match and Content-Format.
//! Classic Block2 also retains request ETag; Q-Block2 omits it to request
//! payloads instead of cache-validation responses. Two retained 8-byte tags
//! and their presence/condition flags cost at most 96 bytes across the default four Calls
//! including host alignment.
//! Larger FETCH requests return [`Error::FetchRequestTooLarge`] before I/O;
//! use caller-managed Engine transfers for larger selection bodies.

#[path = "client_async.rs"]
mod asynchronous;
#[path = "client_probe.rs"]
mod probe;
pub(crate) use asynchronous::{
    complete_client_async, continue_qblock1_ack_async, pace_qblock1_async, recover_qblock2_async,
};
pub use probe::{
    TokenProbe, TokenProbeCall, TokenSupport, TokenSupportLifetimeError, TokenSupportOutcome,
};

use crate::error::{BlockTransferError, EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, Message, MessageId, Opt, OptionsBuilder, OwnedToken,
    ParsedMessage, Token, Transmission, Type, encode_uint,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, BodyTag, DatagramIo, DatagramSlots, Endpoint, Engine,
    ExchangeEntry, ExchangeKey, Exchanges, MemoryLayout, Missing, ObserveInterest, ObserveKey,
    ObserveSlots, OutgoingBlock, PendingCon, PendingCons, Present, SlotId, Storage,
};

use super::identity::AppIds;
use super::request::{IntoPath, MAX_PATH_SEGMENTS, Path, PathError, path_from_into};
use super::response::{AppAssembled, INLINE_PAYLOAD, Response};
use super::{App, Error, Method, Metrics, push_opt};

/// Option slots for an outbound request (path + query + Table 4 extras).
const CLIENT_OPTION_SLOTS: usize = 10 + 2 * MAX_PATH_SEGMENTS;

/// Outstanding client exchange (Token + destination).
///
/// Identity for [`App::take_response`](super::App::take_response) and
/// [`App::cancel`](super::App::cancel). Not a
/// slot. After [`Outgoing::observe`], the same `Call` yields the initial
/// representation and later notifications.
///
/// Non-Block [`Response::payload`] is at most
/// [`INLINE_PAYLOAD`](super::INLINE_PAYLOAD) (128) bytes — see
/// [`Response::payload_truncated`]. Assembled Block2 is
/// [`Response::body`].
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
/// let peer = Endpoint::v4([192, 0, 2, 2], 5683);
/// let call = app.get("sensors/temp").to(peer).send(0).unwrap();
/// app.poll(0).unwrap();
/// let response = app.take_response(call);
/// # let _ = response;
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Call<K: OwnedToken = Token> {
    token: K,
    peer: Endpoint,
}

impl<K: OwnedToken> Call<K> {
    pub(crate) const fn new(token: K, peer: Endpoint) -> Self {
        Self { token, peer }
    }

    /// Token sent on the request (and expected on the response).
    #[must_use]
    pub const fn token(self) -> K {
        self.token
    }

    /// Destination of the request / source of the matched response.
    #[must_use]
    pub const fn peer(self) -> Endpoint {
        self.peer.physical()
    }
}

/// Complete-response collection failed without consuming the reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseBufferError {
    /// The complete representation does not fit; retry with a larger buffer.
    TooSmall {
        /// Exact complete representation length.
        required: usize,
        /// Supplied output capacity.
        available: usize,
    },
    /// Complete bytes are unavailable because the snapshot is truncated or storage is invalid.
    Unavailable,
}
impl core::fmt::Display for ResponseBufferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooSmall {
                required,
                available,
            } => write!(f, "response needs {required} bytes; buffer has {available}"),
            Self::Unavailable => f.write_str("complete response bytes unavailable"),
        }
    }
}
#[cfg(feature = "std")]
impl std::error::Error for ResponseBufferError {}

/// Local terminal outcome of a client call, distinct from a remote CoAP response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallFailure {
    /// Retaining the reply would consume mandatory packet headroom.
    PacketHeadroom,
    /// Caller cancelled this call locally (no deregistration is sent).
    Cancelled,
    /// The owning security association was revoked locally.
    AssociationRevoked,
    /// Explicit cancellation could not be sent; local subscription is retired.
    CancellationFailed,
    /// Re-registration could not be sent; local subscription is retired.
    ReregistrationFailed,
    /// A new Observe representation changes or malforms its established format.
    ObserveFormatMismatch,
    /// A block continuation could not be sent. Poll returns the detailed error;
    /// the call is retired locally and remote effects remain uncertain.
    ContinuationFailed,
    /// Caller-supplied absolute deadline was reached.
    DeadlineExceeded,
    /// Retransmissions or the response lifetime were exhausted.
    TimedOut,
    /// Peer rejected the exchange with an empty Reset.
    Reset,
    /// Received blocks could not be assembled into the requested representation.
    BlockTransfer(BlockTransferError),
    /// Response metadata exceeds App's option count, byte, or Location count bound.
    ResponseMetadataBounds,
    /// Complete block representation exceeds App's fixed assembled-response hold.
    ResponseBodyBounds,
}
impl core::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PacketHeadroom => f.write_str("response cannot retain the last RX packet"),
            Self::AssociationRevoked => f.write_str("security association revoked"),
            Self::Cancelled => f.write_str("call cancelled"),
            Self::CancellationFailed => f.write_str("Observe cancellation send failed"),
            Self::ReregistrationFailed => f.write_str("Observe re-registration send failed"),
            Self::ObserveFormatMismatch => f.write_str("Observe notification format mismatch"),
            Self::ContinuationFailed => f.write_str("block continuation send failed"),
            Self::DeadlineExceeded => f.write_str("call deadline exceeded"),
            Self::TimedOut => f.write_str("request timed out"),
            Self::Reset => f.write_str("peer reset the request"),
            Self::ResponseMetadataBounds => f.write_str("response metadata exceeds App bounds"),
            Self::ResponseBodyBounds => f.write_str("response body exceeds App bounds"),
            Self::BlockTransfer(error) => write!(f, "response block transfer failed: {error}"),
        }
    }
}
#[cfg(feature = "std")]
impl std::error::Error for CallFailure {}

/// Maximum options retained per client response, including unknown elective options.
pub const RESPONSE_OPTION_COUNT: usize = 24;
/// Maximum encoded header, Token and options retained per client response (no payload).
pub const RESPONSE_OPTION_BYTES: usize = 512;

/// Maximum canonical Observe request bytes retained per live Call for exact
/// cancellation matching, including a fixed four-byte header and payload.
/// Each configured App Call slot reserves this space plus bounded length metadata.
/// Observe and ETag options are excluded; no hash replaces exact comparison.
pub const OBSERVE_REQUEST_BYTES: usize = 512;

#[derive(Clone, Copy, Debug)]
pub(super) struct ReplyMeta {
    received_at_ms: u64,
    header: [u8; RESPONSE_OPTION_BYTES],
    header_len: u16,
    peer: Endpoint,
    payload: [u8; INLINE_PAYLOAD],
    payload_len: u16,
    payload_src_len: u16,
}

impl ReplyMeta {
    pub(super) const fn peer(&self) -> Endpoint {
        self.peer
    }

    pub(super) const fn received_at_ms(&self) -> u64 {
        self.received_at_ms
    }

    // Canonically encoded code, Token, MID and selected options. Capture time
    // and copied payload bookkeeping are not representation-header identity.
    pub(super) fn same_header(&self, other: &Self) -> bool {
        self.peer == other.peer
            && self.header[..usize::from(self.header_len)]
                == other.header[..usize::from(other.header_len)]
    }

    // The caller builds this header from bounded Response fields. Encode into
    // the final hold so deferred registration never decodes its own output.
    pub(super) fn deferred(
        header: &Message<'_, impl OwnedToken>,
        peer: Endpoint,
        now_ms: u64,
    ) -> Result<Self, EncodeError> {
        let mut reply = Self {
            received_at_ms: now_ms,
            header: [0; RESPONSE_OPTION_BYTES],
            header_len: 0,
            peer,
            payload: [0; INLINE_PAYLOAD],
            payload_len: 0,
            payload_src_len: 0,
        };
        reply.header_len = crate::message::encode(header, &mut reply.header)? as u16;
        Ok(reply)
    }

    #[cfg(test)]
    pub(super) fn from_parsed(
        parsed: &ParsedMessage<'_, impl OwnedToken>,
        peer: Endpoint,
        received_at_ms: u64,
    ) -> Result<Self, CallFailure> {
        Ok(
            ReplyCapture::checked(parsed, peer, received_at_ms, &ResponseOptions::read(parsed))?
                .retain(),
        )
    }

    pub(super) fn response<K: OwnedToken>(&self) -> Response<'_, K> {
        // The stored header was produced by the encoder and is immutable.
        let parsed =
            ParsedMessage::<K>::from_retained_header(&self.header[..usize::from(self.header_len)])
                .expect("encoded response header");
        let mut response =
            Response::from_client(parsed, self, &self.payload[..usize::from(self.payload_len)])
                .with_payload_src_len(self.payload_src_len);
        use crate::message::OptionNumber as N;
        // Wire options are ordered. Singleton accessors use the first value,
        // including an invalid first value; Location options preserve repeats.
        let mut previous = None;
        for option in parsed.options() {
            let number = option.number();
            let first = previous.replace(number) != Some(number);
            match number {
                N::CONTENT_FORMAT if first => {
                    response =
                        response.retain_format(option.as_uint16().ok().map(ContentFormat::new))
                }
                N::ETAG if first && (1..=8).contains(&option.value().len()) => {
                    response = response.etag(option.value())
                }
                N::OBSERVE if first => {
                    if let Ok(seq) = crate::message::decode_observe(option.value()) {
                        response = response.observe(seq);
                    }
                }
                N::MAX_AGE if first => response = response.retain_max_age(option.as_uint().ok()),
                N::ECHO if first => {
                    if let Ok(echo) = crate::message::Echo::new(option.value()) {
                        response = response.echo(echo);
                    }
                }
                N::LOCATION_PATH => {
                    if let Ok(path) = option.as_str()
                        && path.len() <= 255
                        && path != "."
                        && path != ".."
                    {
                        response = response.location_path(path);
                    }
                }
                N::LOCATION_QUERY => {
                    if let Ok(query) = option.as_str()
                        && query.len() <= 255
                    {
                        response = response.location_query(query);
                    }
                }
                _ => {}
            }
        }
        response
    }
}

// Collect response facts before applying any state transition. Error policy
// still belongs to the consumer (e.g. a stale notification ignores its format).
#[derive(Default)]
struct ResponseOptions<'a> {
    unknown_critical: bool,
    metadata_overflow: bool,
    observe: Option<u32>,
    format: Option<Result<ContentFormat, crate::error::ValueError>>,
    repeated_format: bool,
    max_age: Option<Result<u32, crate::error::ValueError>>,
    repeated_age: bool,
    block1: Option<Result<BlockValue, crate::error::ValueError>>,
    q_block1: Option<Result<BlockValue, crate::error::ValueError>>,
    block2: Option<Result<BlockValue, crate::error::ValueError>>,
    q_block2: Option<Result<BlockValue, crate::error::ValueError>>,
    later_block2: bool,
    size2: Option<Result<u32, crate::error::ValueError>>,
    etag: Option<&'a [u8]>,
}

impl<'a> ResponseOptions<'a> {
    fn read(parsed: &ParsedMessage<'a, impl OwnedToken>) -> Self {
        use crate::message::OptionNumber as N;
        let mut facts = Self::default();
        let mut previous = None;
        let mut locations = [0; 2];
        for (i, option) in parsed.options().enumerate() {
            let number = option.number();
            let first = previous.replace(number) != Some(number);
            facts.metadata_overflow |= i >= RESPONSE_OPTION_COUNT;
            match number {
                N::LOCATION_PATH | N::LOCATION_QUERY => {
                    let count = &mut locations[usize::from(number == N::LOCATION_QUERY)];
                    *count += 1;
                    facts.metadata_overflow |= *count > super::LOCATION_MAX;
                }
                N::OBSERVE if first => {
                    facts.observe = crate::message::decode_observe(option.value()).ok()
                }
                N::CONTENT_FORMAT if first => {
                    facts.format = Some(option.as_uint16().map(ContentFormat::new))
                }
                N::CONTENT_FORMAT => facts.repeated_format = true,
                N::MAX_AGE if first => facts.max_age = Some(option.as_uint()),
                N::MAX_AGE => facts.repeated_age = true,
                N::ETAG if first => facts.etag = Some(option.value()),
                N::SIZE2 if first => facts.size2 = Some(option.as_uint()),
                N::BLOCK1 if first => facts.block1 = Some(option.as_block()),
                N::Q_BLOCK1 if first => facts.q_block1 = Some(option.as_block()),
                N::BLOCK2 if first => {
                    let block = option.as_block();
                    facts.later_block2 |= block.is_ok_and(|value| value.num() != 0);
                    facts.block2 = Some(block);
                }
                N::Q_BLOCK2 => {
                    let block = option.as_block();
                    facts.later_block2 |= block.is_ok_and(|value| value.num() != 0);
                    if first {
                        facts.q_block2 = Some(block);
                    }
                }
                _ => facts.unknown_critical |= number.is_critical() && !number.is_known(),
            }
        }
        facts
    }
}

// Check retention limits on every fragment, including duplicates. Materialize
// bytes only when the inbox actually acquires a new representation.
struct ReplyCapture<'a, 'wire, K: OwnedToken> {
    parsed: &'a ParsedMessage<'wire, K>,
    peer: Endpoint,
    received_at_ms: u64,
    header_len: u16,
    observe_age: Option<u32>,
}

impl<'a, 'wire, K: OwnedToken> ReplyCapture<'a, 'wire, K> {
    fn checked(
        parsed: &'a ParsedMessage<'wire, K>,
        peer: Endpoint,
        received_at_ms: u64,
        options: &ResponseOptions<'_>,
    ) -> Result<Self, CallFailure> {
        let header_len = parsed
            .encoded_header_len()
            .filter(|n| *n <= RESPONSE_OPTION_BYTES)
            .ok_or(CallFailure::ResponseMetadataBounds)?;
        if options.metadata_overflow {
            return Err(CallFailure::ResponseMetadataBounds);
        }
        Ok(Self {
            parsed,
            peer,
            received_at_ms,
            header_len: header_len as u16,
            observe_age: (parsed.code().is_success() && options.observe.is_some()).then(|| {
                options
                    .max_age
                    .and_then(Result::ok)
                    .unwrap_or(super::DEFAULT_MAX_AGE_SECS)
            }),
        })
    }

    fn retain(self) -> ReplyMeta {
        let parsed = self.parsed;
        let prefix = Message::new(parsed.ty(), parsed.code(), parsed.message_id())
            .with_token(parsed.token());
        let mut header = [0; RESPONSE_OPTION_BYTES];
        let start = prefix
            .encode(&mut header)
            .expect("checked token and header capacity");
        header[start..usize::from(self.header_len)].copy_from_slice(parsed.options_bytes());
        let src = parsed.payload();
        let n = src.len().min(INLINE_PAYLOAD);
        let mut payload = [0u8; INLINE_PAYLOAD];
        payload[..n].copy_from_slice(&src[..n]);
        ReplyMeta {
            received_at_ms: self.received_at_ms,
            header,
            header_len: self.header_len,
            peer: self.peer,
            payload,
            payload_len: n as u16,
            payload_src_len: u16::try_from(src.len()).unwrap_or(u16::MAX),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct InboxRow<K: OwnedToken = Token> {
    call: Call<K>,
    meta: Result<ReplyMeta, CallFailure>,
    storage: ReplyStorage,
}

#[derive(Clone, Copy, Debug)]
enum ReplyStorage {
    Inline,
    Datagram(SlotId),
    Assembled(SlotId),
    Partial {
        body: SlotId,
        observe_age: Option<u32>,
    },
}

impl ReplyStorage {
    fn complete_payload<'a, S: Storage + DatagramSlots + BodySlots>(
        self,
        meta: &'a ReplyMeta,
        engine: &'a Engine<S>,
    ) -> Result<&'a [u8], ResponseBufferError> {
        match self {
            Self::Assembled(id) => engine.rx_body_payload(id),
            Self::Datagram(id) => {
                // The inbox owns the exact admitted datagram until collection.
                let start = usize::from(meta.header_len) + usize::from(meta.payload_src_len != 0);
                engine
                    .storage()
                    .rx_payload(id)
                    .and_then(|wire| wire.get(start..))
            }
            Self::Inline if meta.payload_src_len <= meta.payload_len => {
                Some(&meta.payload[..usize::from(meta.payload_len)])
            }
            Self::Inline | Self::Partial { .. } => None,
        }
        .ok_or(ResponseBufferError::Unavailable)
    }

    fn complete(self) -> bool {
        !matches!(self, Self::Partial { .. })
    }
    fn datagram(self) -> Option<SlotId> {
        match self {
            Self::Datagram(rx) => Some(rx),
            _ => None,
        }
    }
    fn body(self) -> Option<SlotId> {
        match self {
            Self::Partial { body, .. } | Self::Assembled(body) => Some(body),
            _ => None,
        }
    }
}

// An existing Call owns its row even when an earlier row is vacant.
fn existing_or_vacant<T>(
    rows: &mut [Option<T>],
    matches: impl Fn(&T) -> bool,
) -> Option<&mut Option<T>> {
    let mut free = None;
    for slot in rows {
        match slot {
            Some(row) if matches(row) => return Some(slot),
            None if free.is_none() => free = Some(slot),
            _ => {}
        }
    }
    free
}

/// Bounded completed-reply table (not a seventh memory area).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientInbox<const CALLS: usize, K: OwnedToken = Token> {
    rows: [Option<InboxRow<K>>; CALLS],
    held: Option<ReplyMeta>,
}

impl<const CALLS: usize, K: OwnedToken> ClientInbox<CALLS, K> {
    pub(super) fn usage(&self) -> (usize, usize) {
        self.rows
            .iter()
            .flatten()
            .fold((0, 0), |(used, complete), row| {
                (used + 1, complete + usize::from(row.storage.complete()))
            })
    }

    pub(super) fn has_protocol_state(&self) -> bool {
        self.rows.iter().any(Option::is_some)
    }

    pub(crate) const fn new() -> Self {
        Self {
            rows: [None; CALLS],
            held: None,
        }
    }

    fn slot_for(&mut self, call: Call<K>) -> Option<&mut Option<InboxRow<K>>> {
        existing_or_vacant(&mut self.rows, |row| row.call == call)
    }

    pub(super) fn holds_datagram(&self, rx: SlotId) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|row| row.storage.datagram() == Some(rx))
    }

    fn completed_index(&self, call: Call<K>) -> Option<usize> {
        self.rows.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|row| row.storage.complete() && row.call == call)
        })
    }

    fn contains(&self, call: Call<K>) -> bool {
        self.completed_index(call).is_some()
    }

    #[allow(clippy::type_complexity)]
    fn take_at(
        &mut self,
        index: usize,
    ) -> (
        Result<Response<'_, K>, CallFailure>,
        Option<SlotId>,
        Option<SlotId>,
    ) {
        let row = self.rows[index].take().expect("completed reply");
        let response = match row.meta {
            Ok(meta) => {
                self.held = Some(meta);
                Ok(self.held.as_ref().expect("stored reply").response())
            }
            Err(error) => {
                self.held = None;
                Err(error)
            }
        };
        (response, row.storage.body(), row.storage.datagram())
    }
}

/// Owned query values retained across Block2 requests, in original order.
#[derive(Clone, Copy, Debug)]
struct RetainedQueries {
    bytes: [u8; 256],
    lengths: [u8; MAX_PATH_SEGMENTS],
    count: u8,
}

impl RetainedQueries {
    const EMPTY: Self = Self {
        bytes: [0; 256],
        lengths: [0; MAX_PATH_SEGMENTS],
        count: 0,
    };

    fn new(queries: &[&str]) -> Result<Self, EncodeError> {
        if queries.len() > MAX_PATH_SEGMENTS {
            return Err(EncodeError::OptionValueTooLong);
        }
        let mut retained = Self {
            count: queries.len() as u8,
            ..Self::EMPTY
        };
        let mut offset = 0;
        for (i, query) in queries.iter().enumerate() {
            if query.len() > 255 || query.len() > retained.bytes.len() - offset {
                return Err(EncodeError::OptionValueTooLong);
            }
            retained.bytes[offset..offset + query.len()].copy_from_slice(query.as_bytes());
            retained.lengths[i] = query.len() as u8;
            offset += query.len();
        }
        Ok(retained)
    }

    fn values(&self) -> impl Iterator<Item = &[u8]> {
        let mut offset = 0;
        self.lengths[..usize::from(self.count)]
            .iter()
            .map(move |len| {
                let end = offset + usize::from(*len);
                let value = &self.bytes[offset..end];
                offset = end;
                value
            })
    }
}

/// Canonical request identity for Observe, FETCH and custom-option follow-ups.
/// Fixed header/Token; Observe and ETag are omitted. FETCH/Observe retain payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObserveRequest {
    bytes: [u8; OBSERVE_REQUEST_BYTES],
    len: usize,
}

impl ObserveRequest {
    fn same_target(&self, other: &Self) -> bool {
        use crate::message::OptionNumber as N;
        let left =
            crate::message::extended::decode(&self.bytes[..self.len]).expect("encoded identity");
        let right =
            crate::message::extended::decode(&other.bytes[..other.len]).expect("encoded identity");
        let cache_key = |number: N| {
            !number.is_no_cache_key()
                && !matches!(number, N::BLOCK1 | N::BLOCK2 | N::Q_BLOCK1 | N::Q_BLOCK2)
        };
        left.code() == right.code()
            && (left.code() != Code::FETCH || left.payload() == right.payload())
            && left
                .options()
                .filter(|option| cache_key(option.number()))
                .map(|option| (option.number(), option.value()))
                .eq(right
                    .options()
                    .filter(|option| cache_key(option.number()))
                    .map(|option| (option.number(), option.value())))
    }
}

#[derive(Clone, Copy, Debug)]
struct LiveCall<K: OwnedToken = Token> {
    call: Call<K>,
    queries: RetainedQueries,
    accept: Option<ContentFormat>,
    request_tag: BodyTag,
    if_match: BodyTag,
    if_none_match: bool,
    etag: BodyTag,
    path: Path<'static>,
    code: Code,
    ty: Type,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    observe: OutgoingObserve,
    observed: Option<(u32, u64)>,
    observed_format: Option<Option<ContentFormat>>,
    request_identity: Option<ObserveRequest>,
    /// When the outstanding request may be forgotten (`0` = never).
    due_ms: u64,
    deadline_ms: Option<u64>,
    /// Next NON Q1 set may start at this instant; zero means disarmed.
    q_pace_ms: u64,
    /// Cooldown retains this row's request identity, without a second buffer.
    backoff_until_ms: u64,
    /// Response collected or Call cancelled; only cooldown identity remains live.
    retired: bool,
}

/// Path / type for an outstanding client request (Block1 / Block2 Continue).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientLives<const CALLS: usize, K: OwnedToken = Token> {
    rows: [Option<LiveCall<K>>; CALLS],
}

impl<const CALLS: usize, K: OwnedToken> ClientLives<CALLS, K> {
    pub(super) fn usage(&self) -> (usize, usize) {
        self.rows
            .iter()
            .flatten()
            .fold((0, 0), |(used, retired), row| {
                (used + 1, retired + usize::from(row.retired))
            })
    }

    pub(super) fn next_timeout_after<Mem: Storage<Token = K> + Exchanges + ObserveSlots>(
        &self,
        engine: &Engine<Mem>,
        inbox: &ClientInbox<CALLS, K>,
        after: Option<u64>,
    ) -> Option<u64> {
        self.rows
            .iter()
            .flatten()
            .flat_map(|live| {
                let active = !live.retired;
                let waiting = active
                    && (!inbox.contains(live.call) || client_observe_live(engine, live.call));
                (live.backoff_until_ms != 0)
                    .then_some(live.backoff_until_ms)
                    .into_iter()
                    .chain((active && live.q_pace_ms != 0).then_some(live.q_pace_ms))
                    .chain(live.deadline_ms.filter(|_| waiting))
                    .chain(
                        (waiting
                            && live.due_ms != 0
                            && engine
                                .lookup_exchange(ExchangeKey::new(
                                    live.call.token(),
                                    live.call.peer,
                                ))
                                .is_some())
                        .then_some(live.due_ms),
                    )
            })
            .filter(|deadline| after.is_none_or(|after| *deadline > after))
            .min()
    }

    fn response_credits_exhausted(&self, inbox: &ClientInbox<CALLS, K>, limit: usize) -> bool {
        if limit > CALLS.saturating_mul(2) {
            return false;
        }
        let active = self
            .rows
            .iter()
            .flatten()
            .filter(|row| !row.retired)
            .count();
        let mut credits = active;
        // Each live Call owns one credit until its terminal failure. A successful
        // reply that outlives its Call still owns a credit until collection.
        for reply in inbox.rows.iter().flatten() {
            let has_live = active != 0 && self.get(reply.call).is_some();
            if reply.meta.is_ok() && !has_live {
                credits += 1;
            } else if reply.meta.is_err() && reply.storage.complete() && has_live {
                credits -= 1;
            }
        }
        credits >= limit
    }

    pub(super) fn has_protocol_state(&self) -> bool {
        self.rows.iter().flatten().any(|row| !row.retired)
    }

    pub(crate) const fn new() -> Self {
        Self {
            rows: [None; CALLS],
        }
    }

    fn expire_backoffs(&mut self, now_ms: u64) {
        for slot in &mut self.rows {
            Self::expire_backoff(slot, now_ms);
        }
    }

    fn expire_backoff(slot: &mut Option<LiveCall<K>>, now_ms: u64) {
        if let Some(row) = slot
            && row.backoff_until_ms != 0
            && row.backoff_until_ms <= now_ms
        {
            if row.retired {
                *slot = None;
            } else {
                row.backoff_until_ms = 0;
            }
        }
    }

    fn backoff_until(
        &self,
        peer: Endpoint,
        code: Code,
        path: &Path<'_>,
        queries: &RetainedQueries,
    ) -> Option<u64> {
        self.rows
            .iter()
            .flatten()
            .filter(|row| {
                row.backoff_until_ms != 0
                    && row.call.peer == peer
                    && row.code == code
                    && row.path.segments() == path.segments()
                    && row.queries.values().eq(queries.values())
            })
            .map(|row| row.backoff_until_ms)
            .max()
    }

    fn record_backoff(&mut self, call: Call<K>, options: &ResponseOptions<'_>, now_ms: u64) {
        if options.repeated_age {
            return;
        }
        let seconds = match options.max_age {
            Some(Ok(value)) => value,
            None => super::DEFAULT_MAX_AGE_SECS,
            Some(Err(_)) => return,
        };
        self.expire_backoffs(now_ms);
        if seconds == 0 {
            return;
        }
        if let Some(row) = self
            .rows
            .iter_mut()
            .flatten()
            .find(|row| !row.retired && row.call == call)
        {
            row.backoff_until_ms = row
                .backoff_until_ms
                .max(now_ms.saturating_add(u64::from(seconds) * 1000));
        }
    }

    fn get(&self, call: Call<K>) -> Option<&LiveCall<K>> {
        self.rows
            .iter()
            .flatten()
            .find(|row| !row.retired && row.call == call)
    }

    fn get_mut(&mut self, call: Call<K>) -> Option<&mut LiveCall<K>> {
        self.rows
            .iter_mut()
            .flatten()
            .find(|row| !row.retired && row.call == call)
    }

    fn classify_observation(
        &self,
        call: Call<K>,
        code: Code,
        options: &ResponseOptions<'_>,
        now_ms: u64,
        protected: bool,
    ) -> Result<ResponseObservation, CallFailure> {
        let Some(sequence) = options
            .observe
            .filter(|_| code.is_success() && !options.later_block2)
        else {
            return Ok(ResponseObservation::Ordinary);
        };
        let live = self.get(call);
        if !protected && let Some((previous, received)) = live.and_then(|row| row.observed) {
            let delta = sequence.wrapping_sub(previous) & 0x00ff_ffff;
            if (delta == 0 || delta >= 0x0080_0000) && now_ms.saturating_sub(received) <= 128_000 {
                return Ok(ResponseObservation::Stale);
            }
        }
        let format = options
            .format
            .transpose()
            .map_err(|_| CallFailure::ObserveFormatMismatch)?;
        if options.repeated_format {
            return Err(CallFailure::ObserveFormatMismatch);
        }
        // A 2.03 without Content-Format validates the cached format.
        let update = (format.is_some() || code != Code::VALID).then_some(format);
        if let (Some(initial), Some(new)) = (live.and_then(|row| row.observed_format), update)
            && initial != new
        {
            return Err(CallFailure::ObserveFormatMismatch);
        }
        Ok(ResponseObservation::New {
            sequence,
            format: update,
        })
    }

    fn record_observation(&mut self, call: Call<K>, observation: ResponseObservation, now_ms: u64) {
        if let ResponseObservation::New { sequence, format } = observation
            && let Some(live) = self
                .rows
                .iter_mut()
                .flatten()
                .find(|live| live.call == call)
        {
            live.observed = Some((sequence, now_ms));
            if live.observed_format.is_none() {
                live.observed_format = format;
            }
        }
    }

    fn upload_key(&self, call: Call<K>) -> BlockKey<K> {
        BlockKey::new(call.token, call.peer).with_identity(
            self.get(call)
                .map(|live| live.request_tag)
                .unwrap_or(BodyTag::ABSENT),
        )
    }

    fn remove(&mut self, call: Call<K>) {
        let Some(index) = self.rows.iter().position(|row| {
            row.as_ref()
                .is_some_and(|row| !row.retired && row.call == call)
        }) else {
            return;
        };
        let row = self.rows[index].as_ref().expect("matched row");
        if row.backoff_until_ms == 0 {
            self.rows[index] = None;
            return;
        }
        // Concurrent requests may receive overload before either response is
        // collected. Merge their retired identities, preserving the longest delay.
        let other = self.rows.iter().position(|entry| {
            entry.as_ref().is_some_and(|entry| {
                entry.retired
                    && entry.call.peer == row.call.peer
                    && entry.code == row.code
                    && entry.path.segments() == row.path.segments()
                    && entry.queries.values().eq(row.queries.values())
            })
        });
        if let Some(other) = other {
            let until = row.backoff_until_ms;
            let entry = self.rows[other].as_mut().expect("matched cooldown");
            entry.backoff_until_ms = entry.backoff_until_ms.max(until);
            self.rows[index] = None;
        } else {
            let row = self.rows[index].as_mut().expect("matched row");
            row.retired = true;
            row.due_ms = 0;
            row.deadline_ms = None;
            row.q_pace_ms = 0;
            row.observe = OutgoingObserve::Off;
        }
    }

    fn observe_slot<E>(
        &self,
        peer: Endpoint,
        identity: &ObserveRequest,
        target: Option<Call<K>>,
        is_live: impl Fn(Call<K>) -> bool,
    ) -> Result<(usize, &LiveCall<K>), Error<E>> {
        let mut matches = self.rows.iter().enumerate().filter_map(|(index, slot)| {
            let live = slot.as_ref()?;
            (live.call.peer == peer
                && live.observe == OutgoingObserve::Register
                && live.due_ms != 0
                && target.is_none_or(|call| live.call == call)
                && live.request_identity.as_ref() == Some(identity)
                && is_live(live.call))
            .then_some((index, live))
        });
        let found = matches.next().ok_or(Error::ObserveCancellationMismatch)?;
        if matches.next().is_some() {
            return Err(Error::ObserveCancellationAmbiguous);
        }
        Ok(found)
    }
}

#[derive(Clone, Copy)]
enum ResponseObservation {
    Ordinary,
    Stale,
    New {
        sequence: u32,
        // None keeps the representation validated by a 2.03 response.
        format: Option<Option<ContentFormat>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutgoingObserve {
    Off,
    Register,
    Deregister,
}

/// Outbound request builder: [`App::get`](super::App::get) / [`App::put`](super::App::put).
///
/// Distinct from the site routers [`get`](super::get) / [`put`](super::put).
/// Chain [`.to`](Self::to), optional [`.payload`](Self::payload) /
/// [`.non`](Self::non) / [`.query`](Self::query) / [`.accept`](Self::accept) /
/// [`.etag`](Self::etag) / [`.if_match`](Self::if_match) /
/// [`.if_none_match`](Self::if_none_match) / [`.observe`](Self::observe) /
/// [`.deregister`](Self::deregister) / [`.block2`](Self::block2) /
/// [`.q_block1`](Self::q_block1) / [`.q_block2`](Self::q_block2), then
/// [`.send`](Self::send). Default type is CON. Path sugar is
/// [`super::IntoPath`] (`"sensors/temp"` or `&["sensors", "temp"]`).
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
/// let peer = Endpoint::v4([192, 0, 2, 2], 5683);
/// let call = app.get("sensors/temp").to(peer).send(0).unwrap();
/// app.poll(0).unwrap();
/// let _ = app.take_response(call);
/// ```
pub struct Outgoing<
    'a,
    P,
    T,
    const N: usize,
    Dest = Missing,
    const BLOCK_WISE: bool = false,
    S: super::AppStorage<Token = P::Token> = super::AppStore<P, BLOCK_WISE>,
    const DEFERRED: usize = 0,
    const CALLS: usize = { super::DEFAULT_CLIENT_CALLS },
    const REQUESTS: usize = 4,
    const ASSOCIATIONS: usize = 1,
> where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    app: &'a mut App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>,
    code: Code,
    ty: Type,
    dest: Option<Endpoint>,
    association_valid: bool,
    path: Result<Path<'static>, PathError>,
    payload: &'a [u8],
    custom_options: &'a [Opt<'a>],
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    accept: Option<ContentFormat>,
    request_tag: BodyTag,
    etag: Option<&'a [u8]>,
    if_match: Option<&'a [u8]>,
    if_none_match: bool,
    queries: [&'a str; MAX_PATH_SEGMENTS],
    query_n: u8,
    q_block1: bool,
    q_block2: bool,
    block2: Option<BlockValue>,
    observe: OutgoingObserve,
    observe_target: Option<Call<S::Token>>,
    deadline_ms: Option<u64>,
    _dest: core::marker::PhantomData<Dest>,
}

impl<
    P,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
    S: super::AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    /// CON GET builder. Next: [`Outgoing::to`].
    ///
    /// Distinct from the site router [`get`](super::get).
    /// `path` is [`IntoPath`]: `"sensors/temp"` or `&["sensors", "temp"]`.
    #[must_use]
    pub fn get(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::Get, path)
    }

    /// CON PUT builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn put(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::Put, path)
    }

    /// CON POST builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn post(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::Post, path)
    }

    /// CON DELETE builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn delete(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::Delete, path)
    }

    /// CON FETCH builder. Set [`Outgoing::content_format`] for the selection
    /// body (also required for an empty selection), then [`Outgoing::to`].
    #[must_use]
    pub fn fetch(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::Fetch, path)
    }

    /// CON PATCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn patch(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::Patch, path)
    }

    /// CON iPATCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn ipatch(
        &mut self,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        self.request(Method::IPatch, path)
    }

    /// CON request builder for `method`. Next: [`Outgoing::to`].
    #[must_use]
    pub fn request(
        &mut self,
        method: Method,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        Outgoing {
            app: self,
            code: method.code(),
            ty: Type::Confirmable,
            dest: None,
            association_valid: true,
            path: path_from_into(path),
            payload: &[],
            custom_options: &[],
            content_format: None,
            echo: None,
            no_response: None,
            accept: None,
            request_tag: BodyTag::ABSENT,
            etag: None,
            if_match: None,
            if_none_match: false,
            queries: [""; MAX_PATH_SEGMENTS],
            query_n: 0,
            q_block1: false,
            q_block2: false,
            block2: None,
            observe: OutgoingObserve::Off,
            observe_target: None,
            deadline_ms: None,
            _dest: core::marker::PhantomData,
        }
    }
}

impl<
    P,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
    S: super::AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    /// Take the matched [`Response`] for `call`, if [`App::poll`](Self::poll)
    /// has completed it.
    ///
    /// **Non-Block payload is at most [`super::INLINE_PAYLOAD`] (128) bytes.** A
    /// 200-byte piggybacked 2.05 with
    /// [`.block_wise::<false>()`](super::AppBuilder::block_wise) copies 128
    /// bytes into [`Response::payload`]. Check
    /// [`Response::payload_truncated`] / [`Response::payload_src_len`].
    /// [`Response`] does not own a 4KiB copy.
    ///
    /// When `BLOCK_WISE` and Block2 / Q-Block2 assembled, copies the RX
    /// body into an App hold and returns [`Response::body`] borrowed from
    /// that hold (released from the Engine). Representations above
    /// [`super::RESPONSE_BODY`] fail with [`CallFailure::ResponseBodyBounds`],
    /// even when a custom Engine profile can assemble more bytes. A later [`Self::take_response`] or
    /// [`Self::poll`] overwrites the hold. Without block-wise storage, there is no
    /// assembled hold. After [`Outgoing::observe`], the same `call` yields the initial
    /// representation and later notifications.
    /// Notifications without OSCORE obey RFC 7641 serial-number ordering
    /// and its strict 128-second fallback. Stale CON notifications are ACKed
    /// without replacing data or refreshing Max-Age. OSCORE notifications use
    /// authenticated Partial-IV ordering instead. For block-wise notifications,
    /// ordering is established by the first accepted block, before completion.
    /// Max-Age expiry does not cancel the subscription. Use the retained
    /// `received_at_ms` and `max_age_secs` (default 60 seconds when absent)
    /// to assess representation freshness against the same monotonic clock.
    /// A response without Observe ends the subscription; explicit cancellation
    /// and deadlines also release it. Unrecognized unprotected CON responses
    /// receive Reset so the peer can stop notifying.
    /// `None` means pending/unknown/already taken. `Some(Ok(_))` is an actual
    /// remote response, including remote 4.xx/5.xx. `Some(Err(_))` is a local
    /// terminal outcome; no synthetic CoAP response code is invented.
    /// Location, Max-Age, Echo and raw options are retained within
    /// [`RESPONSE_OPTION_COUNT`] / [`RESPONSE_OPTION_BYTES`]. Block-wise replies
    /// retain the first accepted fragment's header metadata; no response is
    /// exposed until assembly completes. Metadata and assembled bytes borrow App.
    pub fn take_response(
        &mut self,
        call: Call<S::Token>,
    ) -> Option<Result<Response<'_, S::Token>, CallFailure>> {
        let index = self.inbox.completed_index(call)?;
        let (response, body, datagram) = self.inbox.take_at(index);
        if let Some(rx) = datagram {
            let _ = self.engine.release_rx(rx);
        }
        if !client_observe_live(&self.engine, call) {
            self.lives.remove(call);
            super::oscore::cancel(&mut self.oscore, call.peer, call.token());
        }
        if let Some(id) = body {
            let result = match self.engine.rx_body_payload(id) {
                Some(bytes) if bytes.len() <= super::RESPONSE_BODY => {
                    P::store::<S>(&mut self.assembled, bytes);
                    Ok(())
                }
                Some(_) => Err(CallFailure::ResponseBodyBounds),
                None => Err(CallFailure::BlockTransfer(BlockTransferError::Slot(
                    crate::storage::SlotError::NotOccupied,
                ))),
            };
            let _ = self.engine.release_rx_body(id);
            if let Err(failure) = result {
                return Some(Err(failure));
            }
            if let Some(bytes) = P::view::<S>(&self.assembled) {
                return Some(response.map(|response| response.with_assembled(bytes)));
            }
        }
        Some(response)
    }
    /// Collect a complete representation into caller-owned storage.
    ///
    /// Enable [`super::AppBuilder::full_responses`] before bind to retain full
    /// ordinary datagrams and bodies larger than the [`super::RESPONSE_BODY`] hold.
    /// An insufficient buffer leaves the reply and all bytes available for a
    /// later attempt; no bytes are copied and no partial success is returned.
    /// Metadata borrows App; `payload()` (and block `body()`) borrows `output`.
    /// The nested outcome distinguishes local Call failure from remote 4.xx/5.xx.
    /// Block2 / Q-Block2 require block-wise storage. Without it, the Call
    /// fails with [`CallFailure::BlockTransfer`] containing
    /// [`BlockTransferError::NoBodyPools`], rather than returning one fragment.
    #[allow(clippy::type_complexity)]
    pub fn take_response_into<'a>(
        &'a mut self,
        call: Call<S::Token>,
        output: &'a mut [u8],
    ) -> Result<Option<Result<Response<'a, S::Token>, CallFailure>>, ResponseBufferError> {
        let Some(index) = self.inbox.completed_index(call) else {
            return Ok(None);
        };
        let row = self.inbox.rows[index].as_ref().expect("completed reply");
        let len = if let Ok(meta) = &row.meta {
            let bytes = row.storage.complete_payload(meta, &self.engine)?;
            if bytes.len() > output.len() {
                return Err(ResponseBufferError::TooSmall {
                    required: bytes.len(),
                    available: output.len(),
                });
            }
            output[..bytes.len()].copy_from_slice(bytes);
            bytes.len()
        } else {
            0
        };
        let storage = row.storage;
        self.release_complete_reply(call, storage);
        let (response, _, _) = self.inbox.take_at(index);
        Ok(Some(response.map(|response| {
            response.with_complete_payload(&output[..len], storage.body().is_some())
        })))
    }

    /// Consume a complete reply by borrowing its payload and metadata for `read`.
    ///
    /// The callback can inspect or decode the bytes directly in the occupied
    /// datagram, assembled body, or inbox. Its result can outlive the callback.
    /// After it returns, the reply's slots are available for reuse. An active
    /// Observe Call continues to receive subsequent notifications.
    ///
    /// As with [`Self::take_response_into`], enable
    /// [`super::AppBuilder::full_responses`] to collect a complete ordinary
    /// reply larger than the inline limit. Unavailable bytes leave the reply
    /// available for snapshot collection. A pending Call or a local failure
    /// never invokes the callback. Remote error responses do invoke it.
    pub fn take_response_with<R>(
        &mut self,
        call: Call<S::Token>,
        read: impl FnOnce(Response<'_, S::Token>) -> R,
    ) -> Result<Option<Result<R, CallFailure>>, ResponseBufferError> {
        let Some(index) = self.inbox.completed_index(call) else {
            return Ok(None);
        };
        let row = self.inbox.rows[index].as_ref().expect("completed reply");
        let storage = row.storage;
        let result = match &row.meta {
            Ok(meta) => {
                let bytes = storage.complete_payload(meta, &self.engine)?;
                Ok(read(
                    meta.response()
                        .with_complete_payload(bytes, storage.body().is_some()),
                ))
            }
            Err(failure) => Err(*failure),
        };
        self.inbox.rows[index] = None;
        self.release_complete_reply(call, storage);
        Ok(Some(result))
    }

    fn release_complete_reply(&mut self, call: Call<S::Token>, storage: ReplyStorage) {
        if let Some(rx) = storage.datagram() {
            let _ = self.engine.release_rx(rx);
        }
        if let Some(body) = storage.body() {
            let _ = self.engine.release_rx_body(body);
        }
        if !client_observe_live(&self.engine, call) {
            self.lives.remove(call);
            super::oscore::cancel(&mut self.oscore, call.peer, call.token());
        }
    }

    /// Cancel an active call and reclaim its exchange, body, Observe and OSCORE
    /// state. The next `take_response` yields `Err(CallFailure::Cancelled)`.
    /// No wire message is sent; use `Outgoing::deregister` for Observe signaling.
    /// Returns false for unknown/already terminal ordinary calls.
    pub fn cancel(&mut self, call: Call<S::Token>) -> bool {
        if self.lives.get(call).is_none()
            || (self.inbox.contains(call) && !client_observe_live(&self.engine, call))
        {
            return false;
        }
        fail_call(
            &mut self.engine,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            call,
            CallFailure::Cancelled,
            true,
        );
        true
    }
}

impl<
    'a,
    P,
    T,
    const N: usize,
    Dest,
    const BLOCK_WISE: bool,
    S: super::AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> Outgoing<'a, P, T, N, Dest, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    /// Destination endpoint (Token matching uses this peer).
    #[must_use]
    pub fn to(
        self,
        peer: Endpoint,
    ) -> Outgoing<'a, P, T, N, Present, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        Outgoing {
            app: self.app,
            code: self.code,
            ty: self.ty,
            dest: Some(peer),
            association_valid: self.association_valid,
            path: self.path,
            payload: self.payload,
            custom_options: self.custom_options,
            content_format: self.content_format,
            echo: self.echo,
            no_response: self.no_response,
            accept: self.accept,
            request_tag: self.request_tag,
            etag: self.etag,
            if_match: self.if_match,
            if_none_match: self.if_none_match,
            queries: self.queries,
            query_n: self.query_n,
            q_block1: self.q_block1,
            q_block2: self.q_block2,
            block2: self.block2,
            observe: self.observe,
            observe_target: self.observe_target,
            deadline_ms: self.deadline_ms,
            _dest: core::marker::PhantomData,
        }
    }

    /// Select an association issued by this App, including when peers share an endpoint.
    #[must_use]
    pub fn to_association(
        mut self,
        association: super::Association,
    ) -> Outgoing<'a, P, T, N, Present, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
    {
        use super::associations::ContextStorage;
        self.association_valid =
            self.app.oscore.association(association.route) == Some(association);
        self.to(association.route)
    }

    /// Absolute deadline in the same monotonic millisecond clock as `send`/`poll`.
    /// A deadline at/before `send` is refused before I/O. At `poll`, the deadline
    /// wins over a response processed at that same time. Covers all fragments
    /// and the entire Observe subscription until cancelled/deregistered.
    #[must_use]
    pub const fn deadline(mut self, deadline_ms: u64) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self
    }

    /// Request body. A payload that does not fit one datagram is sent as
    /// Block1 (or Q-Block1 after [`.q_block1`](Self::q_block1)) when
    /// block-wise is enabled.
    /// Initial classic Block1 sizing tries at most seven sizes against the
    /// configured TX capacity, including security overhead. Later block-number
    /// or security-sequence growth can still refuse a continuation.
    #[must_use]
    pub const fn payload(mut self, payload: &'a [u8]) -> Self {
        self.payload = payload;
        self
    }

    /// Set application-defined elective options, replacing any previous list.
    ///
    /// Values are borrowed while preparing the request. Accepted Calls retain
    /// copies in existing bounded storage for retries, Block1/Block2 and Observe.
    /// Options are sorted by number; repeated numbers retain their supplied order.
    /// Number zero, critical options and numbers in [`crate::message::OptionNumber::is_known`]
    /// are refused with [`Error::CustomOptionNotAllowed`] before I/O. Use the typed
    /// builders for protocol options. Under OSCORE these unknown options are encrypted
    /// as Class E; extensions requiring another protection class are unsupported.
    ///
    /// The complete initial option list shares the existing 26-option bound.
    /// Canonical request identity (including the body for FETCH/Observe) must fit
    /// 512 encoded bytes, otherwise [`Error::CustomRequestTooLarge`] is returned.
    /// Fragmented uploads share a 512-byte, 12-option retained-control area with
    /// Observe and initial Block2/Q-Block2 controls. Overflow is an explicit
    /// [`Error::Block`] refusal, never silent omission.
    #[must_use]
    pub const fn custom_options(mut self, options: &'a [Opt<'a>]) -> Self {
        self.custom_options = options;
        self
    }

    /// Content-Format option.
    #[must_use]
    pub const fn content_format(mut self, format: ContentFormat) -> Self {
        self.content_format = Some(format);
        self
    }

    /// Express disinterest in selected response classes (RFC 7967).
    ///
    /// An explicit zero bitmap is retained on the wire. Server suppression is
    /// optional: returned responses remain observable. Silence is never reported
    /// as success; use [`Self::deadline`] for a partial mask, or [`App::cancel`]
    /// to cease listening when suppressing all classes. Empty ACKs only stop RTO.
    /// Zero is also retained on every Block1 or Q-Block1 upload fragment.
    /// Classic Block1 also supports error-only masks 8, 16 and 24: successful
    /// Continue responses still advance the upload. Suppressed errors leave the
    /// outcome unknown; deadline or cancellation bounds the local wait without
    /// proving success. A peer may still return a selected error response.
    /// Q-Block1 permits masks 2, 16 and 18 for CON and NON. CON uses transport
    /// ACKs; NON masks 2/18 pace each new ten-payload set after a sampled 2..=3s
    /// delay unless Continue arrives first. Poll sends at most one timed set,
    /// without catch-up bursts or inferred acknowledgement. All modes keep 4.08
    /// missing-block reports. Q-Block1 rejects suppression of 4.xx; other bitmap
    /// bits are preserved without changing upload admission or pacing.
    /// Observe permits error-only masks; 2.xx suppression refuses registration.
    /// Suppressed terminal errors end server interest without a notification;
    /// use freshness recovery or a deadline to bound the client wait.
    #[must_use]
    pub const fn no_response(mut self, value: crate::message::NoResponse) -> Self {
        self.no_response = Some(value);
        self
    }

    /// Echo a server-provided freshness challenge (RFC 9175).
    ///
    /// The caller must use this only for the endpoint that supplied the Echo,
    /// and preserve its security context and Inner/Outer protection class.
    /// App retains the value on every upload fragment and download continuation.
    /// Use [`Response::echo_option`] after a 4.01 response or preemptive challenge;
    /// retries are explicit so the caller can decide whether an unsafe operation
    /// is still fresh. This setter does not authenticate a challenge or retry a call.
    #[must_use]
    pub const fn echo(mut self, echo: crate::message::Echo) -> Self {
        self.echo = Some(echo);
        self
    }

    /// Accept option, retained on every Block1, Q-Block1 and Block2 request.
    #[must_use]
    pub const fn accept(mut self, format: ContentFormat) -> Self {
        self.accept = Some(format);
        self
    }

    /// Request-Tag for this operation (RFC 9175 sections 3.3/3.4).
    ///
    /// The caller supplies a unique value for each distinct Q-Block1 body and
    /// owns uniqueness across App instances/restarts. Active reuse at the same
    /// peer is refused. A present empty tag is distinct from absence. App emits
    /// one tag, retaining it for upload fragments and Block2 follow-ups.
    /// The App receive path also supports one tag and explicitly rejects longer
    /// values or multiple tags rather than silently ignoring identity components.
    #[must_use]
    pub const fn request_tag(mut self, tag: BodyTag) -> Self {
        self.request_tag = tag;
        self
    }

    /// ETag cache validator (1..=8 bytes), distinct from a write precondition.
    /// Classic/Q-Block1 FETCH retains this validator on every fragment, with the
    /// existing 512-byte canonical selection bound. Other fragmented methods
    /// return [`Error::ConditionalUploadUnsupported`]. First-fragment sizing tries
    /// at most seven block sizes against the actual unprotected/protected TX limit.
    /// Later option or Partial-IV growth can still refuse; no security state rewinds.
    /// The handler validates the complete selection and may return 2.03 Valid.
    /// FETCH follow-ups repeat the full selection and re-fragment it when needed.
    /// Q-Block2 follow-up fragmentation requires a retained Request-Tag. Body
    /// saturation or minimum-fragment encoding failure terminates the Call.
    #[must_use]
    pub const fn etag(mut self, tag: &'a [u8]) -> Self {
        self.etag = Some(tag);
        self
    }

    /// If-Match option, retained on every Block1/Q-Block1 fragment and reissue.
    /// An empty tag requires any current representation. The receiving handler
    /// evaluates the precondition against its current representation when it
    /// commits the complete body; intermediate ACKs do not prove it has passed.
    #[must_use]
    pub const fn if_match(mut self, tag: &'a [u8]) -> Self {
        self.if_match = Some(tag);
        self
    }

    /// If-None-Match option. Block1/Q-Block1 send it only on NUM0 and retain
    /// it for complete-body handler evaluation. The handler checks current
    /// existence atomically with its application effect; partial ACKs are not success.
    /// Q-Block1 retains the condition even when NUM0 arrives after later blocks.
    /// With OSCORE and one Token, the client accepts only its latest request
    /// binding: a response to an older reordered fragment may remain unknown.
    /// Missing-block reports and fresh selective reissues preserve this security
    /// boundary. Use a deadline and check current state when retrying an uncertain effect.
    #[must_use]
    pub const fn if_none_match(mut self) -> Self {
        self.if_none_match = true;
        self
    }

    /// Append a Uri-Query value. Total query bytes are bounded to 256 at send;
    /// each value is bounded to 255 bytes. Empty values are retained. More than
    /// [`super::MAX_PATH_SEGMENTS`] values are rejected at send. Values and
    /// their order are retained across Block1, Q-Block1 and Block2 requests.
    #[must_use]
    pub fn query(mut self, value: &'a str) -> Self {
        let n = usize::from(self.query_n);
        if n < MAX_PATH_SEGMENTS {
            self.queries[n] = value;
            self.query_n += 1;
        } else {
            self.query_n = (MAX_PATH_SEGMENTS + 1) as u8;
        }
        self
    }

    /// Ask for Block2 on the first GET (early size negotiation).
    #[must_use]
    pub const fn block2(mut self, block: BlockValue) -> Self {
        self.block2 = Some(block);
        self
    }

    /// Send as NON (no pending CON). Default is CON.
    #[must_use]
    pub const fn non(mut self) -> Self {
        self.ty = Type::NonConfirmable;
        self
    }

    /// Send a large body with Q-Block1 (windowed) instead of classic Block1.
    /// Requires [`Self::request_tag`]; absent tags are refused before I/O.
    /// CON uploads keep one payload outstanding and advance on its matching
    /// empty ACK. [`Self::non`] keeps each payload NON and sends bounded sets.
    /// Valid 4.08 missing-block reports for NON uploads reissue at most one
    /// payload set, preserving the original ranges and request options.
    /// Malformed, oversized or unsent-range reports are silently discarded.
    /// This per-upload limit does not establish endpoint-wide congestion control.
    #[must_use]
    pub const fn q_block1(mut self) -> Self {
        self.q_block1 = true;
        self
    }

    /// Ask for the entire Q-Block2 body (NUM 0, M=1, SZX 1024).
    /// The peer may still use classic Block2.
    #[must_use]
    pub const fn q_block2(mut self) -> Self {
        self.q_block2 = true;
        self
    }

    /// GET/FETCH Observe=0 (register). Later notifications use the same
    /// [`Call`] / [`App::take_response`](App::take_response).
    /// App retains up to 512 encoded bytes of canonical request identity
    /// (excluding ETag and Observe) for exact cancellation matching. Larger
    /// requests are refused before I/O; use Engine for caller-owned retention.
    /// Repeated subscriptions to the same peer and cache-key target return the
    /// existing Call without I/O, new state, or deadline changes (RFC 7641 §3.1).
    /// FETCH payloads distinguish selections; ETag, NoCacheKey options and block
    /// controls do not create a second subscription. The caller distributes the
    /// single response stream to its consumers. Use [`Self::reregister_call`]
    /// to refresh the request explicitly; cancellation ends the shared Call.
    #[must_use]
    pub const fn observe(mut self) -> Self {
        self.observe = OutgoingObserve::Register;
        self.observe_target = None;
        self
    }

    /// Refresh a specific live subscription with Observe=0 and the same Token.
    /// Repeat the original method, path, options and payload; ETags may differ.
    /// Mismatch is refused before I/O. Notification ordering and the existing
    /// absolute deadline survive the refresh (a new deadline may shorten it).
    /// Pending transmissions and queued representations are retired before send.
    /// On send failure, the local subscription terminates with
    /// [`CallFailure::ReregistrationFailed`]; remote state remains uncertain.
    /// The caller schedules refresh, including RFC 7641 section 3.3's randomized
    /// delay after Max-Age. App does not automatically refresh subscriptions.
    #[must_use]
    pub const fn reregister_call(mut self, call: Call<S::Token>) -> Self {
        self.observe = OutgoingObserve::Register;
        self.observe_target = Some(call);
        self
    }

    /// GET/FETCH Observe=1 (deregister). Reuses the uniquely matching live Call.
    /// Repeat the registration request's method, path, options and payload;
    /// ETags may differ. A mismatch or ambiguous match is refused before I/O.
    /// [`Self::deregister_call`] also validates the explicit Call handle.
    /// After validation, cancellation retires the old local subscription,
    /// queued notifications and pending transmissions. If sending fails,
    /// `take_response` reports [`CallFailure::CancellationFailed`]; remote
    /// cancellation is uncertain and must not be assumed successful.
    #[must_use]
    pub const fn deregister(mut self) -> Self {
        self.observe = OutgoingObserve::Deregister;
        self.observe_target = None;
        self
    }

    /// Cancel a specific subscription, with the same request checks as
    /// [`Self::deregister`]. The Call must still be live at the selected peer.
    #[must_use]
    pub const fn deregister_call(mut self, call: Call<S::Token>) -> Self {
        self.observe = OutgoingObserve::Deregister;
        self.observe_target = Some(call);
        self
    }
}

impl<
    'a,
    P,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
    S: super::AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> Outgoing<'a, P, T, N, Present, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
    T: crate::storage::DatagramTransport,
{
    /// Encode the request, record the Exchange, and send.
    ///
    /// `now_ms` starts CON RTO on Engine with injected random jitter. Later
    /// [`App::poll`](App::poll) sends Due retransmits when that clock
    /// advances. Returns a [`Call`] for [`App::take_response`](App::take_response).
    /// Production Tokens use eight injected random bytes; Message IDs use a
    /// randomly initialized counter and bounded reuse guard. See
    /// [`super::AppBuilder::randomness`]. This crate does not call an OS RNG. A payload that does
    /// not fit one datagram starts Block1 / Q-Block1 when block-wise is on.
    ///
    /// On failure, local request state is retired, including any partial
    /// upload and OSCORE request binding. An error does not prove that the
    /// peer received no datagrams or performed no action; retry policy must
    /// account for that uncertainty. Security sequence numbers are not reset.
    ///
    /// [`Error::Saturated`] if the configured client Call table is full and
    /// this Token is not one of its retained Calls. No reply is evicted.
    pub fn send(self, now_ms: u64) -> Result<Call<S::Token>, Error<T::Error>>
    where
        T: DatagramIo,
    {
        self.prepare(now_ms, None)?.send()
    }

    /// Encode and admit this request, then await complete datagram acceptance.
    ///
    /// Uses the same options, body slots and Call semantics as `send`, including
    /// initial Block1 and Q-Block1 payload sets. The future owns cleanup from its
    /// construction, so dropping it before polling also cancels admission.
    /// `completed_at` supplies the monotonic completion clock after each accepted
    /// datagram. Cancellation and errors do not rewind OSCORE sequence numbers;
    /// the peer may already have acted on earlier datagrams in a Q payload set.
    /// Driving subsequent responses, retransmissions and deadlines remains the
    /// caller's responsibility. The clock must finish in bounded time and use
    /// the same nondecreasing monotonic domain as `now_ms` and later App polling.
    /// The configured request deadline is checked before transport acceptance,
    /// races every pending send, and is checked again against completion time.
    /// The timer uses the same monotonic domain and unregisters on cancellation.
    pub fn send_async<C: 'a>(
        self,
        now_ms: u64,
        completed_at: impl FnMut() -> Result<u64, C> + 'a,
        timer: &'a mut impl crate::platform::AsyncTimer,
    ) -> impl core::future::Future<Output = Result<Call<S::Token>, AsyncCallError<T::Error, C>>> + 'a
    where
        T: crate::storage::AsyncDatagramIo,
    {
        self.send_with_limit(now_ms, completed_at, timer, None)
    }

    /// Send with an additional absolute driving deadline, without changing the
    /// installed Call's response deadline or lifetime.
    ///
    /// Uses the same eager ownership and cancellation contract as [`Self::send_async`].
    /// Expiry of `send_deadline_ms` returns [`AsyncCallError::ProgressRequired`];
    /// expiry of the request's own `.deadline(...)` remains [`Error::DeadlineElapsed`].
    /// Either result can follow acceptance of earlier Q-Block payloads.
    pub fn send_until_async<C: 'a>(
        self,
        now_ms: u64,
        completed_at: impl FnMut() -> Result<u64, C> + 'a,
        timer: &'a mut impl crate::platform::AsyncTimer,
        send_deadline_ms: u64,
    ) -> impl core::future::Future<Output = Result<Call<S::Token>, AsyncCallError<T::Error, C>>> + 'a
    where
        T: crate::storage::AsyncDatagramIo,
    {
        self.send_with_limit(now_ms, completed_at, timer, Some(send_deadline_ms))
    }

    fn send_with_limit<C: 'a>(
        self,
        now_ms: u64,
        completed_at: impl FnMut() -> Result<u64, C> + 'a,
        timer: &'a mut impl crate::platform::AsyncTimer,
        send_deadline: Option<u64>,
    ) -> impl core::future::Future<Output = Result<Call<S::Token>, AsyncCallError<T::Error, C>>> + 'a
    where
        T: crate::storage::AsyncDatagramIo,
    {
        let progress_deadline = self
            .app
            .next_timeout()
            .into_iter()
            .chain(send_deadline)
            .min();
        let prepared = self.prepare(now_ms, progress_deadline);
        async move {
            let mut prepared = prepared.map_err(AsyncCallError::Request)?;
            let mut completed_at = completed_at;
            if prepared.staged.is_none() {
                return Ok(prepared.commit(None));
            }
            let staged = prepared.accept(&mut completed_at, timer).await?;
            if staged.continue_q {
                let body = staged.body.expect("Q body");
                while let Some(complete) = prepared
                    .stage_next_q(body)
                    .map_err(AsyncCallError::Request)?
                {
                    prepared.accept(&mut completed_at, timer).await?;
                    if complete {
                        break;
                    }
                }
            }
            Ok(prepared.commit(staged.body))
        }
    }

    fn prepare(
        self,
        now_ms: u64,
        progress_deadline: Option<u64>,
    ) -> Result<PreparedCall<'a, S, T, CALLS, REQUESTS>, Error<T::Error>> {
        #[cfg(feature = "oscore")]
        if !self.association_valid {
            return Err(Error::Oscore(crate::oscore::Error::Context));
        }

        if self.deadline_ms.is_some_and(|deadline| deadline <= now_ms) {
            return Err(Error::DeadlineElapsed);
        }
        if self.custom_options.len() > CLIENT_OPTION_SLOTS {
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::OptionsFull,
            )));
        }
        for option in self.custom_options {
            let number = option.number();
            if number.get() == 0 || number.is_known() || number.is_critical() {
                return Err(Error::CustomOptionNotAllowed(number));
            }
        }
        if self.code == Code::FETCH && self.content_format.is_none() {
            return Err(Error::FetchContentFormatRequired);
        }
        let dest = super::oscore::resolve(
            &self.app.oscore,
            self.dest.expect("typestate: to() was called"),
        )?;
        let path = self.path.map_err(|_| Error::Path)?;
        let observe = self.observe;
        let refreshing = observe == OutgoingObserve::Register && self.observe_target.is_some();
        let replacing = observe == OutgoingObserve::Deregister || refreshing;
        if observe != OutgoingObserve::Off && !matches!(self.code, Code::GET | Code::FETCH) {
            return Err(Error::ObserveMethodUnsupported);
        }
        if self
            .no_response
            .is_some_and(|value| value.suppresses(Code::REQUEST_ENTITY_INCOMPLETE))
            && self.q_block1
        {
            return Err(Error::NoResponseUploadUnsupported);
        }
        if self
            .no_response
            .is_some_and(|value| value.suppresses(Code::CONTENT))
            && observe == OutgoingObserve::Register
        {
            return Err(Error::NoResponseObserveUnsupported);
        }
        let queries = self.queries;
        let query_n = usize::from(self.query_n);
        if query_n > MAX_PATH_SEGMENTS {
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::OptionValueTooLong,
            )));
        }
        let if_match = BodyTag::from_first(self.if_match).map_err(|_| {
            Error::Message(SlotMessageError::Encode(EncodeError::OptionValueTooLong))
        })?;
        if self.etag.is_some_and(<[u8]>::is_empty) {
            return Err(Error::Message(SlotMessageError::Parse(
                crate::error::ParseError::BadOptionFormat(crate::message::OptionNumber::ETAG),
            )));
        }
        let etag = BodyTag::from_first(self.etag).map_err(|_| {
            Error::Message(SlotMessageError::Encode(EncodeError::OptionValueTooLong))
        })?;
        let retained_queries = RetainedQueries::new(&queries[..query_n])
            .map_err(|e| Error::Message(SlotMessageError::Encode(e)))?;
        self.app.lives.expire_backoffs(now_ms);
        if let Some(until_ms) =
            self.app
                .lives
                .backoff_until(dest, self.code, &path, &retained_queries)
        {
            return Err(Error::Backoff { until_ms });
        }
        let mut spec = ClientSend {
            dest,
            ty: self.ty,
            code: self.code,
            token: S::Token::EMPTY,
            path: path.segments(),
            payload: self.payload,
            custom_options: self.custom_options,
            content_format: self.content_format,
            echo: self.echo,
            no_response: self.no_response,
            accept: self.accept,
            request_tag: self.request_tag,
            etag: self.etag,
            if_match: self.if_match,
            if_none_match: self.if_none_match,
            queries: &retained_queries,
            q_block1: self.q_block1,
            q_block2: self.q_block2,
            block2: self.block2,
            observe,
        };
        let request_identity = if observe == OutgoingObserve::Off
            && self.code != Code::FETCH
            && self.custom_options.is_empty()
        {
            None
        } else {
            Some(
                request_identity_identity(&spec).map_err(|error| match error {
                    EncodeError::BufferTooSmall if !self.custom_options.is_empty() => {
                        Error::CustomRequestTooLarge
                    }
                    EncodeError::BufferTooSmall if observe == OutgoingObserve::Off => {
                        Error::FetchRequestTooLarge
                    }
                    EncodeError::BufferTooSmall => Error::ObserveRequestTooLarge,
                    other => Error::Message(SlotMessageError::Encode(other)),
                })?,
            )
        };
        if observe == OutgoingObserve::Register && !refreshing {
            let identity = request_identity.as_ref().expect("Observe identity");
            if let Some(existing) = self.app.lives.rows.iter().flatten().find(|live| {
                live.call.peer == dest
                    && live.observe == OutgoingObserve::Register
                    && live.due_ms != 0
                    && live
                        .request_identity
                        .as_ref()
                        .is_some_and(|original| original.same_target(identity))
                    && (!self.app.inbox.contains(live.call)
                        || client_observe_live(&self.app.engine, live.call))
            }) {
                let call = existing.call;
                let App {
                    engine,
                    io,
                    ids,
                    inbox,
                    lives,
                    oscore,
                    ..
                } = self.app;
                return Ok(PreparedCall {
                    engine,
                    io,
                    ids,
                    inbox,
                    lives,
                    oscore,
                    progress_deadline,
                    staged: None,
                    pending: None,
                    live: None,
                    call,
                    now_ms,
                    replacing: false,
                    owned: false,
                });
            }
        }
        if self.app.full_responses
            && !replacing
            && self.app.lives.response_credits_exhausted(
                &self.app.inbox,
                self.app
                    .engine
                    .capacities()
                    .rx_datagram_slots
                    .saturating_sub(super::admission::RX_HEADROOM),
            )
        {
            Metrics::inc(&mut self.app.engine.metrics_mut().admission_rx_refused);
            return Err(Error::Saturated);
        }
        let (live_index, token, prior) = if replacing {
            let (index, live) = self
                .app
                .lives
                .observe_slot::<T::Error>(
                    dest,
                    request_identity.as_ref().expect("Observe identity"),
                    self.observe_target,
                    |call| {
                        !self.app.inbox.contains(call)
                            || client_observe_live(&self.app.engine, call)
                    },
                )
                .map_err(|error| {
                    if refreshing {
                        Error::ObserveRefreshMismatch
                    } else {
                        error
                    }
                })?;
            (
                index,
                live.call.token(),
                refreshing.then_some((live.deadline_ms, live.observed, live.observed_format)),
            )
        } else {
            let index = self
                .app
                .lives
                .rows
                .iter()
                .position(Option::is_none)
                .ok_or(Error::Saturated)?;
            (index, self.app.next_token()?, None)
        };
        if self.q_block1 && self.request_tag.is_absent() {
            return Err(Error::RequestTagRequired);
        }
        if !self.request_tag.is_absent()
            && self.app.lives.rows.iter().flatten().any(|live| {
                !live.retired
                    && live.call.peer == dest
                    && live.request_tag == self.request_tag
                    && live.call.token != token
            })
        {
            return Err(Error::RequestTagInUse);
        }
        let (prior_deadline, observed, observed_format) = prior.unwrap_or_default();
        let deadline_ms = match (self.deadline_ms, prior_deadline) {
            (Some(new), Some(old)) => Some(new.min(old)),
            (new, old) => new.or(old),
        };
        if deadline_ms.is_some_and(|deadline| deadline <= now_ms) {
            return Err(Error::DeadlineElapsed);
        }
        spec.token = token;
        if replacing {
            // A new request reuses this Token. Do not retain old CONs, body
            // fragments or authenticated request bindings under that identity.
            abandon_send(&mut self.app.engine, &mut self.app.oscore, token, dest);
            fail_call(
                &mut self.app.engine,
                &mut self.app.inbox,
                &mut self.app.lives,
                &mut self.app.oscore,
                Call::new(token, dest),
                if refreshing {
                    CallFailure::ReregistrationFailed
                } else {
                    CallFailure::CancellationFailed
                },
                true,
            );
        }
        // Cached server replies share the TX pool with outgoing Calls. Reclaim
        // evictable pins before deciding ordinary admission, preserving reserve.
        for _ in 0..=super::admission::TX_HEADROOM {
            if super::admission::free_tx(&mut self.app.engine) > super::admission::TX_HEADROOM
                || !super::evict_one_dedup_pin(&mut self.app.engine)
            {
                break;
            }
        }
        let staged = stage_client(
            &mut self.app.engine,
            &mut self.app.io,
            &mut self.app.ids,
            &mut self.app.oscore,
            now_ms,
            &spec,
        );
        let staged = match staged {
            Ok(staged) => staged,
            Err(error) => {
                abandon_send(&mut self.app.engine, &mut self.app.oscore, token, dest);
                return Err(error);
            }
        };
        let call = Call::new(token, dest);
        let live = LiveCall {
            call,
            queries: retained_queries,
            accept: self.accept,
            request_tag: self.request_tag,
            if_match,
            if_none_match: self.if_none_match,
            etag,
            path,
            code: self.code,
            ty: self.ty,
            content_format: self.content_format,
            echo: self.echo,
            no_response: self.no_response,
            observe,
            observed,
            observed_format,
            request_identity,
            deadline_ms,
            q_pace_ms: 0,
            backoff_until_ms: 0,
            retired: false,
            due_ms: 0, // Armed from transport acceptance in PreparedCall::commit.
        };
        let App {
            engine,
            io,
            ids,
            inbox,
            lives,
            oscore,
            ..
        } = self.app;
        let mut prepared = PreparedCall {
            progress_deadline,
            engine,
            io,
            ids,
            inbox,
            lives,
            oscore,
            staged: Some(staged),
            pending: None,
            live: Some((live_index, live)),
            call,
            now_ms,
            replacing,
            owned: true,
        };
        prepared.admit()?;
        Ok(prepared)
    }
}

impl<
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
    T: crate::storage::DatagramTransport,
    const N: usize,
    const BLOCK_WISE: bool,
    S: super::AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
{
    fn next_token<E>(&mut self) -> Result<S::Token, Error<E>> {
        for _ in 0..8 {
            let token = self
                .ids
                .token(&mut |bytes| self.io.fill_random(bytes))
                .map_err(Error::Identity)?;
            if !self
                .lives
                .rows
                .iter()
                .flatten()
                .any(|live| live.call.token() == token)
            {
                return Ok(token);
            }
        }
        Err(Error::Identity(super::IdentityError::TokenExhausted))
    }
}

struct ClientSend<'a, K: OwnedToken = Token> {
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: K,
    path: &'a [&'a str],
    payload: &'a [u8],
    custom_options: &'a [Opt<'a>],
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    accept: Option<ContentFormat>,
    request_tag: BodyTag,
    etag: Option<&'a [u8]>,
    if_match: Option<&'a [u8]>,
    if_none_match: bool,
    queries: &'a RetainedQueries,
    q_block1: bool,
    q_block2: bool,
    block2: Option<BlockValue>,
    observe: OutgoingObserve,
}

// One option builder feeds both wire encoding and exact cancellation identity.
fn with_client_options<R>(
    spec: &ClientSend<'_, impl OwnedToken>,
    identity_only: bool,
    use_options: impl FnOnce(&[Opt<'_>]) -> R,
) -> Result<R, EncodeError> {
    let cf = spec.content_format.map(ContentFormat::encode);
    let no_response = spec.no_response.map(crate::message::NoResponse::encode);
    let acc = spec.accept.map(ContentFormat::encode);
    let q2 = spec
        .q_block2
        .then(|| BlockValue::new(0, true, BlockValue::SZX_MAX))
        .and_then(Result::ok)
        .map(BlockValue::encode);
    let b2 = spec.block2.map(BlockValue::encode);
    let mut opts = OptionsBuilder::<CLIENT_OPTION_SLOTS>::new();
    let filled = (|| -> Result<(), EncodeError> {
        if let Some(tag) = spec.if_match {
            push_opt(&mut opts, Opt::if_match(tag))?;
        }
        if !identity_only && let Some(tag) = spec.etag {
            push_opt(&mut opts, Opt::etag(tag))?;
        }
        if spec.if_none_match {
            push_opt(&mut opts, Opt::if_none_match())?;
        }
        if !identity_only {
            match spec.observe {
                OutgoingObserve::Register => {
                    push_opt(&mut opts, Opt::observe_register())?;
                }
                OutgoingObserve::Deregister => {
                    push_opt(&mut opts, Opt::observe_deregister())?;
                }
                OutgoingObserve::Off => {}
            }
        }
        for segment in spec.path {
            push_opt(&mut opts, Opt::uri_path(segment))?;
        }
        if let Some(ref encoded) = cf {
            push_opt(&mut opts, Opt::content_format(encoded))?;
        }
        for query in spec.queries.values() {
            push_opt(
                &mut opts,
                Opt::new(crate::message::OptionNumber::URI_QUERY, query),
            )?;
        }
        if let Some(ref encoded) = acc {
            push_opt(&mut opts, Opt::accept(encoded))?;
        }
        if let Some(ref encoded) = b2 {
            push_opt(&mut opts, Opt::block2(encoded))?;
        }
        if let Some(ref encoded) = q2 {
            push_opt(&mut opts, Opt::q_block2(encoded))?;
        }
        if let Some(ref value) = no_response {
            push_opt(&mut opts, Opt::no_response(value))?;
        }
        if let Some(echo) = spec.echo.as_ref() {
            push_opt(&mut opts, Opt::echo(echo.as_slice()))?;
        }
        if let Some(tag) = spec.request_tag.as_slice() {
            push_opt(&mut opts, Opt::request_tag(tag))?;
        }
        for option in spec.custom_options {
            push_opt(&mut opts, *option)?;
        }
        Ok(())
    })();
    filled?;
    Ok(use_options(opts.as_slice()))
}

fn request_identity_identity(
    spec: &ClientSend<'_, impl OwnedToken>,
) -> Result<ObserveRequest, EncodeError> {
    with_client_options(spec, true, |options| {
        let mut identity = ObserveRequest {
            bytes: [0; OBSERVE_REQUEST_BYTES],
            len: 0,
        };
        let message = Message::new(Type::Confirmable, spec.code, MessageId::new(0))
            .with_options(options)
            .with_payload(
                if spec.code == Code::FETCH || spec.observe != OutgoingObserve::Off {
                    spec.payload
                } else {
                    &[]
                },
            );
        identity.len = crate::message::encode(&message, &mut identity.bytes)?;
        Ok(identity)
    })?
}

struct StagedClient {
    tx: SlotId,
    mid: MessageId,
    jitter: u32,
    body: Option<SlotId>,
    continue_q: bool,
}

#[cfg(test)]
mod async_request_tests {
    struct NeverTimer;
    impl crate::platform::AsyncTimer for NeverTimer {
        async fn wait_until(&mut self, _: u64) {
            core::future::pending::<()>().await;
        }
    }

    use super::*;
    use core::{
        cell::Cell,
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);

    #[derive(Default)]
    struct State {
        ready: Cell<bool>,
        sent: Cell<usize>,
        entered: Cell<usize>,
        fail_at: Cell<Option<usize>>,
        blocks: Cell<u64>,
    }

    struct AsyncOnly<'a>(&'a State);
    impl crate::storage::DatagramTransport for AsyncOnly<'_> {
        type Error = ();
    }
    impl crate::app::async_server::ProtocolIo for AsyncOnly<'_> {
        fn completed_at(&self) -> u64 {
            90
        }
    }
    impl crate::storage::AsyncDatagramIo for AsyncOnly<'_> {
        async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            panic!("initial send does not receive")
        }
        async fn send(&mut self, peer: Endpoint, wire: &[u8]) -> Result<usize, ()> {
            assert_eq!(peer, PEER);
            self.0.entered.set(self.0.entered.get() + 1);
            core::future::poll_fn(|cx| {
                if !self.0.ready.get() {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                self.0.ready.set(false);
                if self.0.fail_at.get() == Some(self.0.sent.get()) {
                    return Poll::Ready(Err(()));
                }
                let message = crate::message::decode(wire).unwrap();
                if let Some(Ok(block)) = message.q_block1() {
                    self.0.blocks.set(self.0.blocks.get() | (1 << block.num()));
                }
                self.0.sent.set(self.0.sent.get() + 1);
                Poll::Ready(Ok(wire.len()))
            })
            .await
        }
    }

    #[test]
    fn native_async_initial_call_drop_and_acceptance_share_existing_slots() {
        let mut timer = NeverTimer;
        let state = State::default();
        let mut app = App::with_resources(crate::resources::SmallDevice::new())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(AsyncOnly(&state))
            .unwrap();
        let initial = app.engine.tx_occupied();
        drop(
            app.get("sensor")
                .to(PEER)
                .send_async(0, || Ok::<_, ()>(90), &mut timer),
        );
        assert_eq!(state.entered.get(), 0);
        assert_eq!(app.engine.tx_occupied(), initial);
        assert_eq!(app.lives.rows.iter().flatten().count(), 0);
        {
            let mut send = pin!(app.get("sensor").to(PEER).send_async(
                0,
                || Ok::<_, ()>(90),
                &mut timer
            ));
            assert!(
                send.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        assert_eq!(app.engine.tx_occupied(), initial);
        assert_eq!(app.lives.rows.iter().flatten().count(), 0);
        let call = {
            let mut send = pin!(app.get("sensor").to(PEER).send_async(
                0,
                || Ok::<_, ()>(90),
                &mut timer
            ));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(send.as_mut().poll(&mut cx).is_pending());
            state.ready.set(true);
            match send.as_mut().poll(&mut cx) {
                Poll::Ready(Ok(call)) => call,
                _ => panic!("accepted call"),
            }
        };
        assert_eq!(state.sent.get(), 1);
        assert!(app.lives.get(call).is_some());
        let exchange = app
            .engine
            .lookup_exchange(ExchangeKey::new(call.token(), PEER))
            .unwrap();
        let entry = app.engine.exchange_entry(exchange).unwrap();
        let pending = app.engine.pending_con(entry.tx_slot()).unwrap();
        assert_eq!(
            pending.rto().next_timeout_ms(),
            Some(90 + u64::from(Transmission::initial_timeout_ms(0)))
        );
        assert!(app.cancel(call));
        assert_eq!(app.engine.tx_occupied(), initial);
    }

    #[test]
    fn native_async_block1_and_q_window_acceptance_and_partial_failure() {
        let mut timer = NeverTimer;
        let payload = [7; 1500];
        for q in [false, true] {
            for fail in [false, true] {
                let state = State::default();
                state.fail_at.set(fail.then_some(if q { 1 } else { 0 }));
                let mut app =
                    App::with_resources(crate::resources::SmallDevice::new().block_wise::<true>())
                        .deterministic_for_tests()
                        .allow_unprotected()
                        .bind(AsyncOnly(&state))
                        .unwrap();
                let result = {
                    let mut request = app.post("upload").payload(&payload).to(PEER);
                    if q {
                        request = request
                            .non()
                            .q_block1()
                            .request_tag(BodyTag::new(b"tag").unwrap());
                    }
                    let mut send = pin!(request.send_async(0, || Ok::<_, ()>(90), &mut timer));
                    let mut cx = Context::from_waker(Waker::noop());
                    let mut result = None;
                    for _ in 0..32 {
                        state.ready.set(true);
                        if let Poll::Ready(outcome) = send.as_mut().poll(&mut cx) {
                            result = Some(outcome);
                            break;
                        }
                    }
                    result.expect("bounded initial payload set")
                };
                if fail {
                    assert!(result.is_err());
                    assert_eq!(app.engine.tx_occupied(), 0);
                    assert_eq!(app.lives.rows.iter().flatten().count(), 0);
                    for i in 0..app.engine.capacities().tx_body_slots.unwrap_or(0) {
                        assert!(app.engine.tx_body_transfer(SlotId::from_index(i)).is_none());
                    }
                } else {
                    let call = result.unwrap();
                    assert!(app.lives.get(call).is_some());
                    if q {
                        assert!(state.sent.get() > 1);
                        assert_eq!(state.blocks.get() & 3, 3);
                    } else {
                        assert_eq!(state.sent.get(), 1);
                    }
                    assert!(app.cancel(call));
                    assert_eq!(app.engine.tx_occupied(), 0);
                }
            }
        }
    }

    #[test]
    fn native_async_clock_failure_after_send_leaves_no_call() {
        let mut timer = NeverTimer;
        for non in [false, true] {
            let state = State::default();
            state.ready.set(true);
            let mut app = App::with_resources(crate::resources::SmallDevice::new())
                .deterministic_for_tests()
                .allow_unprotected()
                .bind(AsyncOnly(&state))
                .unwrap();
            {
                let request = app.get("sensor").to(PEER);
                let request = if non { request.non() } else { request };
                let mut send = pin!(request.send_async(0, || Err(42), &mut timer));
                assert!(matches!(
                    send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Ready(Err(AsyncCallError::ClockAfterSend(42)))
                ));
            }
            assert_eq!(state.sent.get(), 1);
            assert_eq!(app.engine.tx_occupied(), 0);
            assert_eq!(app.lives.rows.iter().flatten().count(), 0);
        }
    }

    #[test]
    fn native_async_clock_regression_after_send_leaves_no_call() {
        let mut timer = NeverTimer;
        for non in [false, true] {
            let state = State::default();
            state.ready.set(true);
            let mut app = App::with_resources(crate::resources::SmallDevice::new())
                .deterministic_for_tests()
                .allow_unprotected()
                .bind(AsyncOnly(&state))
                .unwrap();
            {
                let request = app.get("sensor").to(PEER);
                let request = if non { request.non() } else { request };
                let mut send = pin!(request.send_async(50, || Ok::<_, ()>(49), &mut timer));
                assert!(matches!(
                    send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Ready(Err(AsyncCallError::ClockRegressedAfterSend {
                        previous_ms: 50,
                        observed_ms: 49,
                    }))
                ));
            }
            assert_eq!(state.sent.get(), 1);
            assert_eq!(app.engine.tx_occupied(), 0);
            assert_eq!(app.lives.rows.iter().flatten().count(), 0);
        }
    }

    #[test]
    fn native_async_deadline_wakes_pending_transport_and_cleans_upload() {
        struct ManualTimer<'a> {
            ready: &'a Cell<bool>,
            due: &'a Cell<Option<u64>>,
            waker: &'a core::cell::RefCell<Option<Waker>>,
        }
        impl crate::platform::AsyncTimer for ManualTimer<'_> {
            async fn wait_until(&mut self, deadline: u64) {
                struct Registration<'a>(&'a core::cell::RefCell<Option<Waker>>);
                impl Drop for Registration<'_> {
                    fn drop(&mut self) {
                        self.0.borrow_mut().take();
                    }
                }
                let _registration = Registration(self.waker);
                self.due.set(Some(deadline));
                core::future::poll_fn(|cx| {
                    if self.ready.get() {
                        Poll::Ready(())
                    } else {
                        *self.waker.borrow_mut() = Some(cx.waker().clone());
                        Poll::Pending
                    }
                })
                .await;
            }
        }
        for partial_q in [false, true] {
            let state = State::default();
            state.ready.set(partial_q);
            let ready = Cell::new(false);
            let due = Cell::new(None);
            let waker = core::cell::RefCell::new(None);
            let mut timer = ManualTimer {
                ready: &ready,
                due: &due,
                waker: &waker,
            };
            let mut app =
                App::with_resources(crate::resources::SmallDevice::new().block_wise::<true>())
                    .deterministic_for_tests()
                    .allow_unprotected()
                    .bind(AsyncOnly(&state))
                    .unwrap();
            let body = [42; 1500];
            {
                let request = app.post("upload").to(PEER).deadline(50);
                let request = if partial_q {
                    request
                        .non()
                        .payload(&body)
                        .request_tag(BodyTag::new(b"deadline").unwrap())
                        .q_block1()
                } else {
                    request
                };
                let mut send = pin!(request.send_async(0, || Ok::<_, ()>(10), &mut timer));
                assert!(
                    send.as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                assert_eq!(due.get(), Some(50));
                assert!(waker.borrow().is_some());
                ready.set(true);
                waker.borrow_mut().take().unwrap().wake();
                // Deadline wins even when transport becomes ready in the same poll.
                state.ready.set(true);
                assert!(matches!(
                    send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Ready(Err(AsyncCallError::Request(Error::DeadlineElapsed)))
                ));
            }
            assert_eq!(state.sent.get(), usize::from(partial_q));
            assert_eq!(app.engine.tx_occupied(), 0);
            assert_eq!(app.storage_usage().tx_body.unwrap().occupied, 0);
            assert!(app.lives.rows.iter().all(Option::is_none));
            assert!(waker.borrow().is_none());
        }
    }

    #[test]
    fn native_async_pending_send_yields_for_other_live_call_deadline() {
        struct DueTimer;
        impl crate::platform::AsyncTimer for DueTimer {
            async fn wait_until(&mut self, deadline: u64) {
                assert_eq!(deadline, 50);
            }
        }
        let state = State::default();
        state.ready.set(true);
        let mut app = App::with_resources(
            crate::resources::Resources::<super::super::tests::AdmissionTwoRxCalls, 4>::new()
                .client_calls::<2>(),
        )
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(AsyncOnly(&state))
        .unwrap();
        let mut timer = NeverTimer;
        let first = {
            let mut send = pin!(app.get("first").to(PEER).non().deadline(50).send_async(
                0,
                || Ok::<_, ()>(10),
                &mut timer
            ));
            match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Ok(call)) => call,
                _ => panic!("first accepted"),
            }
        };
        let mut due = DueTimer;
        {
            let mut send = pin!(app.get("second").to(PEER).non().send_async(
                10,
                || Ok::<_, ()>(50),
                &mut due
            ));
            assert!(matches!(
                send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Err(AsyncCallError::ProgressRequired { at_ms: 50 }))
            ));
        }
        assert_eq!(state.sent.get(), 1);
        assert_eq!(app.engine.tx_occupied(), 0);
        assert!(app.lives.get(first).is_some());
        assert_eq!(app.lives.rows.iter().flatten().count(), 1);
    }

    #[test]
    fn native_async_driving_limit_preserves_call_response_lifetime() {
        let state = State::default();
        state.ready.set(true);
        let mut timer = NeverTimer;
        let mut app = App::with_resources(
            crate::resources::Resources::<super::super::tests::AdmissionTwoRxCalls, 4>::new()
                .client_calls::<2>(),
        )
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(AsyncOnly(&state))
        .unwrap();
        let call = {
            let mut send = pin!(app.get("first").to(PEER).non().send_until_async(
                0,
                || Ok::<_, ()>(10),
                &mut timer,
                50
            ));
            match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Ok(call)) => call,
                _ => panic!("accepted before driving limit"),
            }
        };
        let live = app.lives.get(call).unwrap();
        assert_eq!(live.deadline_ms, None);
        assert!(live.due_ms > 50);
        {
            let mut send = pin!(app.get("second").to(PEER).non().send_until_async(
                10,
                || Ok::<_, ()>(10),
                &mut timer,
                10
            ));
            assert!(matches!(
                send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Err(AsyncCallError::ProgressRequired { at_ms: 10 }))
            ));
        }
        assert_eq!(state.sent.get(), 1);
        assert_eq!(app.engine.tx_occupied(), 0);
        assert!(app.lives.get(call).is_some());
    }

    #[test]
    fn native_async_deadline_at_acceptance_stops_call_installation() {
        let mut timer = NeverTimer;
        let state = State::default();
        let mut app = App::with_resources(crate::resources::SmallDevice::new())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(AsyncOnly(&state))
            .unwrap();
        {
            let mut send = pin!(app.get("sensor").to(PEER).deadline(50).send_async(
                0,
                || Ok::<_, ()>(50),
                &mut timer
            ));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(send.as_mut().poll(&mut cx).is_pending());
            state.ready.set(true);
            assert!(matches!(
                send.as_mut().poll(&mut cx),
                Poll::Ready(Err(AsyncCallError::Request(Error::DeadlineElapsed)))
            ));
        }
        assert_eq!(state.sent.get(), 1);
        assert_eq!(app.engine.tx_occupied(), 0);
        assert_eq!(app.lives.rows.iter().flatten().count(), 0);
    }

    #[test]
    fn native_async_response_continuation_cancellation_reclaims_call_and_ack() {
        let mut timer = NeverTimer;
        for poll_before_drop in [false, true] {
            let state = State::default();
            state.ready.set(true);
            let mut app = App::with_resources(crate::resources::SmallDevice::new())
                .deterministic_for_tests()
                .allow_unprotected()
                .bind(AsyncOnly(&state))
                .unwrap();
            let call = {
                let mut send = pin!(app.get("sensor").to(PEER).send_async(
                    0,
                    || Ok::<_, ()>(90),
                    &mut timer
                ));
                match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(Ok(call)) => call,
                    _ => panic!("initial acceptance"),
                }
            };
            let message = Message::new(Type::Confirmable, Code::CONTENT, MessageId::new(70))
                .with_token(call.token())
                .with_payload(b"reply");
            let mut wire = [0; 128];
            let len = crate::message::encode(&message, &mut wire).unwrap();
            let parsed = crate::message::decode(&wire[..len]).unwrap();
            let rx = app.engine.acquire_rx().unwrap();
            app.engine.encode_rx(rx, &message).unwrap();
            app.engine.set_rx_endpoint(rx, PEER).unwrap();
            {
                let mut acknowledged = false;
                let future = complete_client_async(
                    &mut app.engine,
                    &mut app.io,
                    &mut app.inbox,
                    &mut app.lives,
                    &mut app.ids,
                    &mut app.oscore,
                    super::super::oscore::ResponseState::None,
                    false,
                    90,
                    PEER,
                    &parsed,
                    None,
                    rx,
                    &mut acknowledged,
                );
                let mut future = pin!(future);
                if poll_before_drop {
                    assert!(
                        future
                            .as_mut()
                            .poll(&mut Context::from_waker(Waker::noop()))
                            .is_pending()
                    );
                }
            }
            assert_eq!(app.engine.tx_occupied(), 0);
            assert_eq!(app.engine.rx_occupied(), 0);
            assert!(matches!(
                app.take_response(call),
                Some(Err(CallFailure::ContinuationFailed))
            ));
        }
    }

    #[test]
    fn native_async_ack_clock_failure_retires_only_ambiguous_call() {
        for before_acceptance in [false, true] {
            let mut timer = NeverTimer;
            let state = State::default();
            state.ready.set(true);
            let mut app = App::with_resources(crate::resources::SmallDevice::new())
                .deterministic_for_tests()
                .allow_unprotected()
                .bind(AsyncOnly(&state))
                .unwrap();
            let call = {
                let mut send = pin!(app.get("sensor").to(PEER).send_async(
                    0,
                    || Ok::<_, ()>(90),
                    &mut timer
                ));
                match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(Ok(call)) => call,
                    _ => panic!("initial acceptance"),
                }
            };
            let response = Message::new(Type::Confirmable, Code::CONTENT, MessageId::new(70))
                .with_token(call.token())
                .with_payload(b"reply");
            let rx = app.engine.acquire_rx().unwrap();
            app.engine.encode_rx(rx, &response).unwrap();
            app.engine.set_rx_endpoint(rx, PEER).unwrap();
            state.ready.set(true);
            let mut samples =
                [if before_acceptance { Err(42) } else { Ok(90) }, Err(42)].into_iter();
            let mut clock = || samples.next().unwrap();
            {
                let mut progress = pin!(app.progress_async(90, &mut clock, &mut timer, 1000));
                let Poll::Ready(Err(Error::Io(crate::storage::DatagramIoError::Io(error)))) =
                    progress
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                else {
                    panic!("clock failure must be reported");
                };
                assert!(matches!(
                    (before_acceptance, error),
                    (true, crate::app::AsyncIoError::ClockBeforeSend(42))
                        | (false, crate::app::AsyncIoError::ClockAfterSend(42))
                ));
            }
            assert_eq!(state.sent.get(), if before_acceptance { 1 } else { 2 });
            assert_eq!(app.engine.rx_occupied(), 0);
            if before_acceptance {
                assert!(app.take_response(call).is_none());
                assert!(app.lives.get(call).is_some());
                assert_eq!(app.engine.tx_occupied(), 1);
            } else {
                assert_eq!(app.engine.tx_occupied(), 0);
                assert!(matches!(
                    app.take_response(call),
                    Some(Err(CallFailure::ContinuationFailed))
                ));
            }
        }
    }

    #[test]
    fn native_async_response_continuation_acceptance_keeps_completion() {
        let mut timer = NeverTimer;
        let state = State::default();
        state.ready.set(true);
        let mut app = App::with_resources(crate::resources::SmallDevice::new())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(AsyncOnly(&state))
            .unwrap();
        let call = {
            let mut send = pin!(app.get("sensor").to(PEER).send_async(
                0,
                || Ok::<_, ()>(90),
                &mut timer
            ));
            match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Ok(call)) => call,
                _ => panic!("initial acceptance"),
            }
        };
        let message = Message::new(Type::Confirmable, Code::CONTENT, MessageId::new(70))
            .with_token(call.token())
            .with_payload(b"reply");
        let mut wire = [0; 128];
        let len = crate::message::encode(&message, &mut wire).unwrap();
        let parsed = crate::message::decode(&wire[..len]).unwrap();
        let rx = app.engine.acquire_rx().unwrap();
        app.engine.encode_rx(rx, &message).unwrap();
        app.engine.set_rx_endpoint(rx, PEER).unwrap();
        {
            let mut acknowledged = false;
            let future = complete_client_async(
                &mut app.engine,
                &mut app.io,
                &mut app.inbox,
                &mut app.lives,
                &mut app.ids,
                &mut app.oscore,
                super::super::oscore::ResponseState::None,
                false,
                90,
                PEER,
                &parsed,
                None,
                rx,
                &mut acknowledged,
            );
            let continuation_bytes = core::mem::size_of_val(&future);
            assert!(
                continuation_bytes <= 16 * 1024,
                "client response future: {continuation_bytes}"
            );
            #[cfg(feature = "std")]
            std::println!(
                "SmallDevice Constrained unprotected fixed blockwise=false ClientResponseFuture={continuation_bytes}"
            );
            let mut future = pin!(future);
            let mut cx = Context::from_waker(Waker::noop());
            assert!(future.as_mut().poll(&mut cx).is_pending());
            state.ready.set(true);
            assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
        }
        assert_eq!(app.engine.tx_occupied(), 0);
        let response = app.take_response(call).unwrap().unwrap();
        assert_eq!(response.payload(), b"reply");
    }

    #[test]
    fn native_async_q_ack_cancellation_retires_advanced_upload() {
        let mut timer = NeverTimer;
        let payload = [7; 1500];
        for poll_before_drop in [false, true] {
            let state = State::default();
            state.ready.set(true);
            let mut app =
                App::with_resources(crate::resources::SmallDevice::new().block_wise::<true>())
                    .deterministic_for_tests()
                    .allow_unprotected()
                    .bind(AsyncOnly(&state))
                    .unwrap();
            let call = {
                let mut send = pin!(
                    app.post("upload")
                        .to(PEER)
                        .payload(&payload)
                        .q_block1()
                        .request_tag(BodyTag::new(b"tag").unwrap())
                        .send_async(0, || Ok::<_, ()>(90), &mut timer)
                );
                match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(Ok(call)) => call,
                    _ => panic!("first Q payload"),
                }
            };
            let exchange = app
                .engine
                .lookup_exchange(ExchangeKey::new(call.token(), PEER))
                .unwrap();
            let entry = app.engine.exchange_entry(exchange).unwrap();
            let tx = app
                .engine
                .take_pending_con(entry.message_id(), PEER)
                .unwrap();
            app.engine.release_tx(tx).unwrap();
            {
                let mut future = pin!(continue_qblock1_ack_async(
                    &mut app.engine,
                    &mut app.io,
                    &mut app.inbox,
                    &mut app.lives,
                    &mut app.ids,
                    &mut app.oscore,
                    90,
                    entry.message_id(),
                    PEER,
                ));
                if poll_before_drop {
                    assert!(
                        future
                            .as_mut()
                            .poll(&mut Context::from_waker(Waker::noop()))
                            .is_pending()
                    );
                }
            }
            assert_eq!(state.sent.get(), 1);
            assert_eq!(app.engine.tx_occupied(), 0);
            assert!(
                app.engine
                    .lookup_exchange(ExchangeKey::new(call.token(), PEER))
                    .is_none()
            );
            for i in 0..app.engine.capacities().tx_body_slots.unwrap_or(0) {
                assert!(app.engine.tx_body_transfer(SlotId::from_index(i)).is_none());
            }
            assert!(matches!(
                app.take_response(call),
                Some(Err(CallFailure::ContinuationFailed))
            ));
        }
    }

    #[cfg(feature = "oscore")]
    #[test]
    fn native_async_protected_observe_preserves_identity_and_consumed_sequences() {
        let mut timer = NeverTimer;
        use crate::oscore::{DeriveParams, SecurityContext};
        let context = |sender, recipient| {
            SecurityContext::derive(DeriveParams {
                master_secret: &[7; 16],
                master_salt: &[],
                sender_id: sender,
                recipient_id: recipient,
                id_context: &[],
            })
            .unwrap()
        };
        let state = State::default();
        let mut app = App::with_resources(crate::resources::SmallDevice::new())
            .deterministic_for_tests()
            .oscore(context(&[1], &[2]))
            .bind(AsyncOnly(&state))
            .unwrap();
        drop(
            app.get("sensor")
                .to(PEER)
                .observe()
                .send_async(0, || Ok::<_, ()>(90), &mut timer),
        );
        assert_eq!(app.oscore().unwrap().sender_seq(), 1);
        assert_eq!(state.sent.get(), 0);
        assert_eq!(app.engine.tx_occupied(), 0);
        let call = {
            let mut send = pin!(app.get("sensor").to(PEER).observe().send_async(
                0,
                || Ok::<_, ()>(90),
                &mut timer
            ));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(send.as_mut().poll(&mut cx).is_pending());
            state.ready.set(true);
            match send.as_mut().poll(&mut cx) {
                Poll::Ready(Ok(call)) => call,
                _ => panic!("accepted Observe"),
            }
        };
        assert_eq!(app.oscore().unwrap().sender_seq(), 2);
        assert!(app.oscore().unwrap().lookup(call.token()).is_some());
        assert_eq!(
            app.lives.get(call).unwrap().observe,
            OutgoingObserve::Register
        );
        let id = app
            .engine
            .lookup_exchange(ExchangeKey::new(call.token(), PEER))
            .unwrap();
        let entry = app.engine.exchange_entry(id).unwrap();
        {
            let packet = app.engine.access_tx(entry.tx_slot()).unwrap();
            let outer = crate::message::decode(packet.as_bytes()).unwrap();
            assert!(outer.oscore().is_some());
            let mut peer = context(&[2], &[1]);
            let mut plain = [0; 256];
            let (request, _) = peer.unprotect_request(&outer, &mut plain).unwrap();
            assert_eq!(request.code(), Code::GET);
            assert_eq!(request.observe(), Some(Ok(0)));
        }
        assert!(app.cancel(call));
        assert_eq!(app.oscore().unwrap().sender_seq(), 2);
        assert!(app.oscore().unwrap().lookup(call.token()).is_none());
    }

    #[test]
    fn native_async_small_device_independent_storage_ceilings() {
        let mut timer = NeverTimer;
        let state = State::default();
        let mut app = App::with_resources(crate::resources::SmallDevice::new())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(AsyncOnly(&state))
            .unwrap();
        let app_bytes = core::mem::size_of_val(&app);
        let prepared = app.get("sensor").to(PEER).prepare(0, None).unwrap();
        let prepared_bytes = core::mem::size_of_val(&prepared);
        drop(prepared);
        let future = app
            .get("sensor")
            .to(PEER)
            .send_async(0, || Ok::<_, ()>(90), &mut timer);
        let future_bytes = core::mem::size_of_val(&future);
        assert!(app_bytes <= 32 * 1024, "whole SmallDevice App: {app_bytes}");
        assert!(prepared_bytes <= 4096, "prepared Call: {prepared_bytes}");
        assert!(
            future_bytes <= 8192,
            "initial request future: {future_bytes}"
        );
        #[cfg(feature = "std")]
        std::println!(
            "SmallDevice Constrained routes=4 calls=1 deferred=0 security_requests=4 associations=1 unprotected fixed blockwise=false App={app_bytes} PreparedCall={prepared_bytes} SendFuture={future_bytes}"
        );
    }
}

/// Failure to prepare or transmit an asynchronous App request.
///
/// An error never proves that earlier datagrams did not reach the peer. In
/// particular, a Q-Block1 initial set can fail after accepting earlier payloads.
#[derive(Debug)]
pub enum AsyncCallError<E, C> {
    /// Validation, capacity, encoding, admission or transport failure.
    Request(Error<E>),
    /// A complete datagram was accepted, then its completion clock failed.
    ClockAfterSend(C),
    /// A datagram was accepted, then its completion clock moved backwards.
    ClockRegressedAfterSend {
        /// Preparation or previous accepted datagram time.
        previous_ms: u64,
        /// Invalid clock sample after transport acceptance.
        observed_ms: u64,
    },
    /// Existing App work needs a protocol pass before this send can wait longer.
    /// Earlier packets in this request may already have reached the peer.
    ProgressRequired {
        /// Existing App deadline that interrupted this initial send.
        at_ms: u64,
    },
    /// A complete CON was accepted, then its reservation commit failed.
    CommitAfterSend,
}

impl<E, C> From<super::send::AsyncSendError<E, C>> for AsyncCallError<E, C> {
    fn from(error: super::send::AsyncSendError<E, C>) -> Self {
        match error {
            super::send::AsyncSendError::Prepare(error) => Self::Request(error),
            super::send::AsyncSendError::Transport(error) => Self::Request(error.into()),
            super::send::AsyncSendError::ClockAfterSend(error) => Self::ClockAfterSend(error),
            super::send::AsyncSendError::CommitAfterSend => Self::CommitAfterSend,
        }
    }
}

// Ownership uses existing packet/body pools. Only the same bounded LiveCall
// options that sync send constructs are retained while the transport waits.
struct PreparedCall<'a, S: super::AppStorage, T, const C: usize, const R: usize> {
    progress_deadline: Option<u64>,
    engine: &'a mut Engine<S>,
    io: &'a mut T,
    ids: &'a mut AppIds,
    inbox: &'a mut ClientInbox<C, S::Token>,
    lives: &'a mut ClientLives<C, S::Token>,
    oscore: &'a mut super::oscore::Field<R, S::Token>,
    staged: Option<StagedClient>,
    pending: Option<crate::storage::PendingCon>,
    live: Option<(usize, LiveCall<S::Token>)>,
    call: Call<S::Token>,
    now_ms: u64,
    replacing: bool,
    owned: bool,
}

impl<S: super::AppStorage, T, const C: usize, const R: usize> Drop
    for PreparedCall<'_, S, T, C, R>
{
    fn drop(&mut self) {
        if !self.owned {
            return;
        }
        if let Some(pending) = self.pending.take() {
            let _ = self.engine.cancel_pending_con(pending);
        }
        if let Some(staged) = self.staged.take() {
            let _ = self.engine.release_tx(staged.tx);
        }
        abandon_send(self.engine, self.oscore, self.call.token(), self.call.peer);
    }
}

impl<S: super::AppStorage, T: crate::storage::DatagramTransport, const C: usize, const R: usize>
    PreparedCall<'_, S, T, C, R>
{
    fn admit(&mut self) -> Result<(), Error<T::Error>> {
        let staged = self.staged.as_ref().expect("prepared datagram");
        self.engine
            .record_request(staged.tx, self.call.peer)?
            .ok_or(Error::Saturated)?;
        let (_, live) = self.live.as_ref().expect("prepared call");
        if live.ty == Type::Confirmable {
            self.pending = Some(
                self.engine
                    .reserve_pending_con(staged.tx, self.call.peer, staged.mid)
                    .ok_or(Error::Saturated)?,
            );
        }
        Ok(())
    }

    fn commit(&mut self, body: Option<SlotId>) -> Call<S::Token> {
        if let Some((index, mut live)) = self.live.take() {
            if self.replacing
                && let Some(slot) = self.inbox.slot_for(self.call)
            {
                replace_reply(self.engine, slot, None);
            }
            live.due_ms =
                self.now_ms
                    .saturating_add(u64::from(if live.ty == Type::NonConfirmable {
                        Transmission::NON_LIFETIME_MS
                    } else {
                        Transmission::EXCHANGE_LIFETIME_MS
                    }));
            arm_q_pacing(self.engine, &mut live, body, self.ids, self.now_ms);
            // Preparation exclusively borrows the App through transport acceptance.
            self.lives.rows[index] = Some(live);
        }
        self.owned = false;
        self.call
    }

    // Encode one continuation synchronously; borrowed option and block scratch
    // lifetimes finish before the transport await begins.
    fn stage_next_q(&mut self, body: SlotId) -> Result<Option<bool>, Error<T::Error>> {
        let issued = match self.engine.next_q_block1(body) {
            Ok(issued) => issued,
            Err(BlockTransferError::OutsideWindow) => return Ok(None),
            Err(error) => return Err(Error::Block(error)),
        };
        let (_, live) = self.live.as_ref().expect("prepared Q upload");
        let staged = stage_block1_issued(
            self.engine,
            self.io,
            self.ids,
            self.oscore,
            self.now_ms,
            self.call.peer,
            live.ty,
            live.code,
            self.call.token(),
            live.path.segments(),
            &live.queries,
            live.accept,
            live.content_format,
            live.echo,
            live.no_response,
            live.if_match.as_slice(),
            live.etag.as_slice(),
            live.if_none_match,
            issued,
            true,
            None,
        )?;
        self.staged = Some(staged);
        self.admit()?;
        Ok(Some(issued.complete()))
    }

    fn send(mut self) -> Result<Call<S::Token>, Error<T::Error>>
    where
        T: DatagramIo,
    {
        let Some(staged) = self.staged.take() else {
            return Ok(self.commit(None));
        };
        let pending = self.pending.take().map(|p| (p, staged.jitter));
        super::send::finish_prepared(
            self.engine,
            self.io,
            staged.tx,
            self.call.peer,
            pending,
            self.now_ms,
        )?;
        if staged.continue_q {
            let body = staged.body.expect("Q body");
            while let Some(complete) = self.stage_next_q(body)? {
                let next = self.staged.take().expect("staged Q payload");
                let pending = self.pending.take().map(|p| (p, next.jitter));
                super::send::finish_prepared(
                    self.engine,
                    self.io,
                    next.tx,
                    self.call.peer,
                    pending,
                    self.now_ms,
                )?;
                if complete {
                    break;
                }
            }
        }
        Ok(self.commit(staged.body))
    }

    async fn accept<CError>(
        &mut self,
        completed_at: &mut impl FnMut() -> Result<u64, CError>,
        timer: &mut impl crate::platform::AsyncTimer,
    ) -> Result<StagedClient, AsyncCallError<T::Error, CError>>
    where
        T: crate::storage::AsyncDatagramIo,
    {
        let completed = core::cell::Cell::new(None);
        let own_deadline = self.live.as_ref().and_then(|(_, live)| live.deadline_ms);
        let other_deadline = self
            .lives
            .next_timeout_after(self.engine, self.inbox, None)
            .into_iter()
            .chain(self.progress_deadline)
            .chain(
                (0..self.engine.capacities().tx_datagram_slots).filter_map(|index| {
                    self.engine
                        .pending_con(SlotId::from_index(index))
                        .and_then(|pending| pending.rto().next_timeout_ms())
                }),
            )
            .min();
        let deadline = own_deadline.into_iter().chain(other_deadline).min();
        if let Some(deadline) = deadline.filter(|due| *due <= self.now_ms) {
            return Err(if own_deadline == Some(deadline) {
                AsyncCallError::Request(Error::DeadlineElapsed)
            } else {
                AsyncCallError::ProgressRequired { at_ms: deadline }
            });
        }
        let staged = self.staged.take().expect("staged datagram");
        let pending = self.pending.take();
        {
            use core::future::Future;
            let send = super::send::finish_async(
                self.engine,
                self.io,
                staged.tx,
                self.call.peer,
                pending.map(|p| (p.message_id(), staged.jitter)),
                pending,
                || {
                    let now = completed_at()?;
                    completed.set(Some(now));
                    Ok(now)
                },
            );
            let mut send = core::pin::pin!(send);
            match deadline {
                Some(deadline) => {
                    let mut timeout = core::pin::pin!(timer.wait_until(deadline));
                    core::future::poll_fn(|cx| {
                        if timeout.as_mut().poll(cx).is_ready() {
                            return core::task::Poll::Ready(Err(
                                if own_deadline == Some(deadline) {
                                    AsyncCallError::Request(Error::DeadlineElapsed)
                                } else {
                                    AsyncCallError::ProgressRequired { at_ms: deadline }
                                },
                            ));
                        }
                        send.as_mut()
                            .poll(cx)
                            .map(|result| result.map_err(AsyncCallError::from))
                    })
                    .await
                }
                None => send.await.map_err(AsyncCallError::from),
            }
        }?;
        let observed_ms = match completed.get() {
            Some(now) => now,
            None => completed_at().map_err(AsyncCallError::ClockAfterSend)?,
        };
        if observed_ms < self.now_ms {
            return Err(AsyncCallError::ClockRegressedAfterSend {
                previous_ms: self.now_ms,
                observed_ms,
            });
        }
        self.now_ms = observed_ms;
        if self
            .live
            .as_ref()
            .and_then(|(_, live)| live.deadline_ms)
            .is_some_and(|deadline| self.now_ms >= deadline)
        {
            return Err(AsyncCallError::Request(Error::DeadlineElapsed));
        }
        Ok(staged)
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_staged_client_async, bound(crate::app::async_server::ProtocolIo), calls(issue_q_block1_window => issue_q_block1_window_async, finish_client_send => finish_client_send_async))]
fn send_staged_client<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    staged: StagedClient,
    spec: &ClientSend<'_, Mem::Token>,
) -> Result<Call<Mem::Token>, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let mut sent = finish_client_send(
        engine,
        io,
        staged.tx,
        spec.dest,
        spec.ty,
        now_ms,
        staged.mid,
        staged.jitter,
    );
    if sent.is_ok() && staged.continue_q {
        sent = issue_q_block1_window(
            engine,
            io,
            ids,
            oscore,
            now_ms,
            spec.dest,
            spec.ty,
            spec.code,
            spec.token,
            spec.path,
            spec.queries,
            spec.accept,
            spec.content_format,
            spec.echo,
            spec.no_response,
            spec.if_match,
            spec.etag,
            spec.if_none_match,
            staged.body.expect("Q upload body"),
            None,
            None,
        )
        .map(|_| ());
    }
    if sent.is_err()
        && let Some(body) = staged.body
    {
        let _ = engine.release_tx_body(body);
    }
    sent.map(|()| Call::new(spec.token, spec.dest))
}

fn stage_client<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    spec: &ClientSend<'_, Mem::Token>,
) -> Result<StagedClient, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: crate::storage::DatagramTransport,
{
    if spec.path.len() > MAX_PATH_SEGMENTS {
        return Err(Error::Path);
    }
    let mid = ids.next_for(engine, io, now_ms)?;
    let Some(tx) = super::admission::ordinary_tx(engine) else {
        return Err(Error::Saturated);
    };
    let encoded = with_client_options(spec, false, |options| {
        let msg = Message::new(spec.ty, spec.code, mid)
            .with_token(spec.token)
            .with_options(options)
            .with_payload(spec.payload);
        super::oscore::encode_request(oscore, spec.dest, engine, tx, &msg)
    });
    let encoded = match encoded {
        Ok(result) => result,
        Err(error) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Message(SlotMessageError::Encode(error)));
        }
    };
    match encoded {
        Ok(_) => Ok(StagedClient {
            tx,
            mid,
            jitter: ids.jitter(),
            body: None,
            continue_q: false,
        }),
        Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))) => {
            // Classic Block1 requires 2.xx Continue. Test the suppressed class,
            // preserving unrelated bitmap bits on every fragment. CON Q uploads
            // use transport ACKs; their success-suppression bit does not hide ACKs.
            if !spec.q_block1
                && spec
                    .no_response
                    .is_some_and(|value| value.suppresses(Code::CONTINUE))
            {
                let _ = engine.release_tx(tx);
                return Err(Error::NoResponseUploadUnsupported);
            }
            // FETCH retains fragmented cache-validator semantics.
            // An ETag is not an If-Match write precondition.
            if spec.etag.is_some() && spec.code != Code::FETCH {
                let _ = engine.release_tx(tx);
                return Err(Error::ConditionalUploadUnsupported);
            }
            // Inner Block-wise: fragment first, then protect each
            // datagram (RFC 8613 §4.1.3.4.1). Same path with OSCORE on.
            // Protocol protect failures are `Error::Oscore`, not this arm.
            with_client_options(spec, false, |controls| {
                stage_client_block1(
                    engine,
                    io,
                    ids,
                    oscore,
                    now_ms,
                    spec.dest,
                    spec.ty,
                    spec.code,
                    spec.token,
                    spec.path,
                    spec.queries,
                    spec.accept,
                    spec.payload,
                    spec.request_tag,
                    spec.content_format,
                    spec.echo,
                    spec.no_response,
                    spec.if_match,
                    spec.etag,
                    spec.if_none_match,
                    spec.q_block1,
                    tx,
                    mid,
                    controls,
                )
            })
            .map_err(|error| Error::Message(SlotMessageError::Encode(error)))?
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_client_block1_async, bound(crate::app::async_server::ProtocolIo), calls(send_staged_client => send_staged_client_async))]
fn send_client_block1<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    payload: &[u8],
    request_tag: BodyTag,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    q_block1: bool,
    tx: SlotId,
    mid: MessageId,
    controls: &[Opt<'_>],
) -> Result<Call<Mem::Token>, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let staged = stage_client_block1(
        engine,
        io,
        ids,
        oscore,
        now_ms,
        dest,
        ty,
        code,
        token,
        path,
        queries,
        accept,
        payload,
        request_tag,
        content_format,
        echo,
        no_response,
        if_match,
        etag,
        if_none_match,
        q_block1,
        tx,
        mid,
        controls,
    )?;
    let spec = ClientSend {
        dest,
        ty,
        code,
        token,
        path,
        payload,
        custom_options: &[],
        content_format,
        echo,
        no_response,
        accept,
        request_tag,
        etag,
        if_match,
        if_none_match,
        queries,
        q_block1,
        q_block2: false,
        block2: None,
        observe: OutgoingObserve::Off,
    };
    send_staged_client(engine, io, ids, oscore, now_ms, staged, &spec)
}

#[allow(clippy::too_many_arguments)]
fn stage_client_block1<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    _io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    _now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    payload: &[u8],
    request_tag: BodyTag,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    q_block1: bool,
    tx: SlotId,
    mid: MessageId,
    controls: &[Opt<'_>],
) -> Result<StagedClient, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: crate::storage::DatagramTransport,
{
    // At most seven candidate SZX values. Only size misses try a smaller
    // prefix. Protection may consume sequence numbers; never rewind them.
    let mut initial_szx = BlockValue::SZX_MAX;
    loop {
        let size = 1usize << (initial_szx + 4);
        let count = payload.len().min(size);
        let block = BlockValue::new(0, count < payload.len(), initial_szx)
            .map_err(BlockTransferError::from)
            .map_err(Error::Block)?;
        match encode_block1_request(
            dest,
            engine,
            oscore,
            tx,
            ty,
            code,
            mid,
            token,
            path,
            queries,
            accept,
            content_format,
            echo,
            no_response,
            if_match,
            etag,
            if_none_match,
            block,
            q_block1,
            request_tag,
            u32::try_from(payload.len()).ok(),
            &payload[..count],
            controls,
        ) {
            Ok(()) => break,
            Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)))
                if initial_szx > 0 =>
            {
                initial_szx -= 1
            }
            Err(error) => {
                let _ = engine.release_tx(tx);
                return Err(error);
            }
        }
    }
    let key = BlockKey::new(token, dest).with_identity(request_tag);
    let started = if q_block1 {
        engine.start_q_block1(key, payload, initial_szx)
    } else {
        engine.start_block1(key, payload, initial_szx)
    };
    let body = match started {
        Ok(id) => id,
        Err(BlockTransferError::NoBodyPools) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )));
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Block(e));
        }
    };
    let mut transfer = engine.tx_body_transfer(body).expect("admitted upload");
    if let Err(error) = transfer.retain_upload_controls(controls).and_then(|()| {
        engine
            .storage_mut()
            .set_tx_body_transfer(body, transfer)
            .map_err(Into::into)
    }) {
        let _ = engine.release_tx(tx);
        let _ = engine.release_tx_body(body);
        return Err(Error::Block(error));
    }
    // The first datagram already owns its MID, jitter and protected RequestRef.
    // Advance the admitted body once and send those exact bytes. A NON Q set
    // continues only after that successful send; CON waits for its matching ACK.
    let first = if q_block1 {
        engine.next_q_block1(body)
    } else {
        engine.next_block1(body)
    };
    match first {
        Ok(issued) => Ok(StagedClient {
            tx,
            mid,
            jitter: ids.jitter(),
            body: Some(body),
            continue_q: q_block1 && ty == Type::NonConfirmable && !issued.complete(),
        }),
        Err(error) => {
            let _ = engine.release_tx(tx);
            let _ = engine.release_tx_body(body);
            Err(Error::Block(error))
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(complete_client_io_async, bound(crate::app::async_server::ProtocolIo), calls(send_block2_continue => send_block2_continue_async, send_q_block2_continue => send_q_block2_continue_async, reissue_qblock1_missing => reissue_qblock1_missing_async, continue_block1_tx => continue_block1_tx_async, super::send_empty_rst => super::send_empty_rst_async, super::send_empty_ack => super::send_empty_ack_async))]
pub(crate) fn complete_client<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    response_state: super::oscore::ResponseState<Mem::Token>,
    full_responses: bool,
    now_ms: u64,
    peer: Endpoint,
    parsed: &ParsedMessage<'_, Mem::Token>,
    plaintext: Option<(&[u8], usize)>,
    rx: SlotId,
    acknowledged: &mut bool,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage
        + DatagramSlots
        + PendingCons
        + Exchanges
        + BodySlots
        + ObserveSlots
        + crate::storage::DedupSlots,
    T: DatagramIo,
{
    let via_exchange = matching_exchange(engine, parsed, peer);
    let matched = via_exchange.is_some() || matching_observe(engine, parsed, peer);
    let options = matched.then(|| ResponseOptions::read(parsed));
    let overloaded = matches!(
        parsed.code(),
        Code::TOO_MANY_REQUESTS | Code::SERVICE_UNAVAILABLE
    );
    if let Some((wire, received_len)) = plaintext {
        // Block fragments are owned by the assembly. Overload replies carry
        // their complete ordinary payload even if the peer included Block2.
        let retain = full_responses
            && options.as_ref().is_some_and(|facts| {
                !facts.unknown_critical
                    && (overloaded || (facts.block2.is_none() && facts.q_block2.is_none()))
            });
        super::admit_plaintext_rx(engine, rx, wire, peer, received_len, retain)?;
    }
    let Some(options) = options else {
        let outcome = if parsed.ty() == Type::Confirmable {
            super::send_empty_rst(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    };
    // RFC 7252 sections 5.4.1 and 4.2: reject the response, not an
    // otherwise matching ACK. No inbox/body/Observe state may accept it.
    if options.unknown_critical {
        if parsed.ty() == Type::Acknowledgement
            && let Some(entry) = via_exchange
            && let Some(tx) = engine.take_pending_con(entry.message_id(), peer)
        {
            let _ = engine.release_tx(tx);
        }
        let outcome = if parsed.ty() == Type::Confirmable {
            super::send_empty_rst(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    }
    let call = Call::new(parsed.token(), peer);
    let observation = match lives.classify_observation(
        call,
        parsed.code(),
        &options,
        now_ms,
        super::oscore::is_active(oscore),
    ) {
        Ok(observation) => observation,
        Err(failure) => {
            let outcome = if parsed.ty() == Type::Confirmable {
                super::send_empty_rst(engine, io, peer, parsed.message_id())
            } else {
                Ok(())
            };
            fail_call(engine, inbox, lives, oscore, call, failure, true);
            let _ = engine.release_rx(rx);
            return outcome;
        }
    };
    if parsed.ty() == Type::Confirmable {
        if let Err(e) = super::send_empty_ack(engine, io, peer, parsed.message_id()) {
            let _ = engine.release_rx(rx);
            return Err(e);
        }
        *acknowledged = true;
    }
    if let Err(error) = super::oscore::commit_response(oscore, peer, response_state) {
        let _ = engine.release_rx(rx);
        return Err(error);
    }
    // A stale piggybacked ACK still ends request retransmission. A stale
    // notification with the same token cannot acknowledge a refreshed exchange.
    let stale = matches!(observation, ResponseObservation::Stale);
    if (!stale || parsed.ty() == Type::Acknowledgement)
        && let Some(entry) = via_exchange
        && let Some(tx) = engine.take_pending_con(entry.message_id(), peer)
    {
        let _ = engine.release_tx(tx);
    }
    if stale {
        let _ = engine.release_rx(rx);
        return Ok(());
    }
    if via_exchange.is_none_or(|entry| entry.message_id() != parsed.message_id())
        && let Some(tx) = engine.take_pending_con(parsed.message_id(), peer)
    {
        let _ = engine.release_tx(tx);
    }

    if let Some((body, transfer)) = upload_body(engine, || lives.upload_key(call)) {
        if transfer.role == BlockRole::OutgoingQBlock1
            && parsed.code() == Code::REQUEST_ENTITY_INCOMPLETE
            && options.format == Some(Ok(ContentFormat::MISSING_BLOCKS))
            && lives
                .get(Call::new(parsed.token(), peer))
                .is_some_and(|live| live.ty == Type::NonConfirmable)
        {
            let outcome =
                reissue_qblock1_missing(engine, io, lives, ids, oscore, now_ms, parsed, peer, body);
            if outcome.is_err() {
                abandon_send(engine, oscore, parsed.token(), peer);
                fail_call(
                    engine,
                    inbox,
                    lives,
                    oscore,
                    Call::new(parsed.token(), peer),
                    CallFailure::ContinuationFailed,
                    true,
                );
            }
            let _ = engine.release_rx(rx);
            return outcome;
        }
        if parsed.code() == Code::CONTINUE {
            let outcome = continue_block1_tx(
                engine,
                io,
                lives,
                ids,
                oscore,
                now_ms,
                parsed.token(),
                options.block1.and_then(Result::ok),
                options.q_block1.and_then(Result::ok),
                peer,
                body,
            );
            if outcome.is_err() {
                abandon_send(engine, oscore, parsed.token(), peer);
                fail_call(
                    engine,
                    inbox,
                    lives,
                    oscore,
                    Call::new(parsed.token(), peer),
                    CallFailure::ContinuationFailed,
                    true,
                );
            }
            let _ = engine.release_rx(rx);
            return outcome;
        }
        let _ = engine.release_tx_body(body);
    }

    let metadata = match ReplyCapture::checked(parsed, peer, now_ms, &options) {
        Ok(metadata) => metadata,
        Err(error) => {
            fail_call(
                engine,
                inbox,
                lives,
                oscore,
                Call::new(parsed.token(), peer),
                error,
                true,
            );
            let _ = engine.release_rx(rx);
            return Ok(());
        }
    };

    if overloaded {
        lives.record_backoff(call, &options, now_ms);
    }

    // Classify from the already-decoded response. A plain 2.05 must not
    // stack Block IO scratch via MissingBlock probes.
    if !overloaded && let Some(block) = options.block2 {
        let progress = block.map_err(BlockTransferError::from).and_then(|block| {
            let which = crate::storage::RxBlockOpt::Block2;
            let (key, expected) =
                which.body_key(call.token(), peer, options.size2, options.etag)?;
            engine.apply_call_block2(key, block, parsed.payload(), expected, false)
        });
        match progress {
            Ok(progress) if progress.complete() => {
                lives.record_observation(call, observation, now_ms);
                finish_assembled(
                    engine,
                    inbox,
                    lives,
                    oscore,
                    parsed,
                    peer,
                    rx,
                    progress.id(),
                    metadata,
                    full_responses,
                );
                return Ok(());
            }
            Ok(progress) => {
                lives.record_observation(call, observation, now_ms);
                if store_block_reply(
                    engine,
                    inbox.slot_for(call),
                    call,
                    progress.id(),
                    metadata,
                    false,
                )
                .is_none()
                {
                    let _ = engine.release_rx(rx);
                    return Err(Error::Saturated);
                }
                take_exchange(engine, parsed, peer);
                let outcome = send_block2_continue(
                    engine,
                    io,
                    lives,
                    ids,
                    oscore,
                    now_ms,
                    parsed,
                    peer,
                    progress.id(),
                );
                if outcome.is_err() {
                    abandon_send(engine, oscore, parsed.token(), peer);
                    fail_call(
                        engine,
                        inbox,
                        lives,
                        oscore,
                        Call::new(parsed.token(), peer),
                        CallFailure::ContinuationFailed,
                        true,
                    );
                }
                let _ = engine.release_rx(rx);
                return outcome;
            }
            Err(BlockTransferError::Overlap | BlockTransferError::AlreadyComplete) => {
                let _ = engine.release_rx(rx);
                return Ok(());
            }
            Err(e) => {
                fail_call(
                    engine,
                    inbox,
                    lives,
                    oscore,
                    Call::new(parsed.token(), peer),
                    CallFailure::BlockTransfer(e),
                    true,
                );
                let _ = engine.release_rx(rx);
                return Ok(());
            }
        }
    } else if !overloaded && let Some(block) = options.q_block2 {
        let progress = block.map_err(BlockTransferError::from).and_then(|block| {
            let which = crate::storage::RxBlockOpt::QBlock2;
            let (key, expected) =
                which.body_key(call.token(), peer, options.size2, options.etag)?;
            engine.apply_call_block2(key, block, parsed.payload(), expected, true)
        });
        match progress {
            Ok(progress) if progress.complete() => {
                lives.record_observation(call, observation, now_ms);
                let _ = engine.note_q2_receive(progress.id(), now_ms, parsed.ty());
                finish_assembled(
                    engine,
                    inbox,
                    lives,
                    oscore,
                    parsed,
                    peer,
                    rx,
                    progress.id(),
                    metadata,
                    full_responses,
                );
                return Ok(());
            }
            Ok(progress) => {
                lives.record_observation(call, observation, now_ms);
                if store_block_reply(
                    engine,
                    inbox.slot_for(call),
                    call,
                    progress.id(),
                    metadata,
                    false,
                )
                .is_none()
                {
                    let _ = engine.release_rx(rx);
                    return Err(Error::Saturated);
                }
                let _ = engine.note_q2_receive(progress.id(), now_ms, parsed.ty());
                // CON payloads advance through their individual ACKs (RFC 9177
                // appendix A.2). A window pull races the sender's next CON train.
                let outcome = if parsed.ty() != Type::Confirmable
                    && needs_q_continue(engine, progress.id())
                {
                    take_exchange(engine, parsed, peer);
                    send_q_block2_continue(
                        engine,
                        io,
                        lives,
                        ids,
                        oscore,
                        now_ms,
                        parsed,
                        peer,
                        progress.id(),
                    )
                } else {
                    Ok(())
                };
                if outcome.is_err() {
                    abandon_send(engine, oscore, parsed.token(), peer);
                    fail_call(
                        engine,
                        inbox,
                        lives,
                        oscore,
                        Call::new(parsed.token(), peer),
                        CallFailure::ContinuationFailed,
                        true,
                    );
                }
                let _ = engine.release_rx(rx);
                return outcome;
            }
            Err(BlockTransferError::Duplicate | BlockTransferError::AlreadyComplete) => {
                let _ = engine.release_rx(rx);
                return Ok(());
            }
            Err(e) => {
                fail_call(
                    engine,
                    inbox,
                    lives,
                    oscore,
                    Call::new(parsed.token(), peer),
                    CallFailure::BlockTransfer(e),
                    true,
                );
                let _ = engine.release_rx(rx);
                return Ok(());
            }
        }
    }

    // An Observe replacement releases its previous RX ownership before keeping
    // the new packet. Unexpected/custom states still cannot consume headroom.
    let slot = inbox.slot_for(call);
    if full_responses
        && !slot
            .as_deref()
            .and_then(Option::as_ref)
            .is_some_and(|row| row.storage.datagram().is_some())
        && super::admission::free_rx(engine) < super::admission::RX_HEADROOM
    {
        Metrics::inc(&mut engine.metrics_mut().admission_rx_refused);
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::PacketHeadroom,
            true,
        );
        let _ = engine.release_rx(rx);
        return Ok(());
    }

    lives.record_observation(call, observation, now_ms);
    accept_client_observe(engine, lives, call, metadata.observe_age, now_ms);
    take_exchange(engine, parsed, peer);
    if let Some(slot) = slot {
        replace_reply(
            engine,
            slot,
            Some(InboxRow {
                call,
                meta: Ok(metadata.retain()),
                storage: if full_responses {
                    ReplyStorage::Datagram(rx)
                } else {
                    ReplyStorage::Inline
                },
            }),
        );
        if full_responses {
            return Ok(());
        }
    }
    let _ = engine.release_rx(rx);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_assembled<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
    rx: SlotId,
    body: SlotId,
    metadata: ReplyCapture<'_, '_, Mem::Token>,
    full_responses: bool,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    let call = Call::new(parsed.token(), peer);
    if !full_responses
        && engine
            .rx_body_payload(body)
            .is_some_and(|bytes| bytes.len() > super::RESPONSE_BODY)
    {
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::ResponseBodyBounds,
            true,
        );
        let _ = engine.release_rx(rx);
        return;
    }
    let incoming = (metadata.observe_age, metadata.received_at_ms);
    let (age, received_at_ms) =
        store_block_reply(engine, inbox.slot_for(call), call, body, metadata, true)
            .unwrap_or(incoming);
    accept_client_observe(engine, lives, call, age, received_at_ms);
    take_exchange(engine, parsed, peer);
    let _ = engine.release_rx(rx);
}

// A replacement transfers ownership before releasing displaced slots. A retained
// assembly can change state in place without moving its first fragment's header.
fn replace_reply<Mem: Storage + BodySlots + DatagramSlots>(
    engine: &mut Engine<Mem>,
    slot: &mut Option<InboxRow<Mem::Token>>,
    next: Option<InboxRow<Mem::Token>>,
) {
    if let Some(previous) = slot.as_ref() {
        if let Some(rx) = previous.storage.datagram() {
            let _ = engine.release_rx(rx);
        }
        if let Some(body) = previous.storage.body()
            && next.as_ref().and_then(|row| row.storage.body()) != Some(body)
        {
            let _ = engine.release_rx_body(body);
        }
    }
    *slot = next;
}

fn store_block_reply<Mem: Storage + BodySlots + DatagramSlots>(
    engine: &mut Engine<Mem>,
    slot: Option<&mut Option<InboxRow<Mem::Token>>>,
    call: Call<Mem::Token>,
    body: SlotId,
    meta: ReplyCapture<'_, '_, Mem::Token>,
    complete: bool,
) -> Option<(Option<u32>, u64)> {
    let Some(slot) = slot else {
        let _ = engine.release_rx_body(body);
        return None;
    };
    if let Some(row) = slot.as_mut()
        && let ReplyStorage::Partial {
            body: current,
            observe_age,
        } = row.storage
        && current == body
    {
        if complete {
            row.storage = ReplyStorage::Assembled(body);
        }
        return Some((
            observe_age,
            row.meta
                .as_ref()
                .expect("partial reply metadata")
                .received_at_ms,
        ));
    }
    let observe_age = meta.observe_age;
    let received_at_ms = meta.received_at_ms;
    let storage = if complete {
        ReplyStorage::Assembled(body)
    } else {
        ReplyStorage::Partial { body, observe_age }
    };
    replace_reply(
        engine,
        slot,
        Some(InboxRow {
            call,
            meta: Ok(meta.retain()),
            storage,
        }),
    );
    Some((observe_age, received_at_ms))
}

/// Check unencrypted routing fields before touching protected response state.
pub(super) fn has_response_target<Mem: Storage + Exchanges + ObserveSlots + PendingCons>(
    engine: &Engine<Mem>,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
) -> bool {
    matching_exchange(engine, parsed, peer).is_some() || matching_observe(engine, parsed, peer)
}

fn matching_observe<Mem: Storage + ObserveSlots>(
    engine: &Engine<Mem>,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
) -> bool {
    parsed.code().is_response()
        && matches!(parsed.ty(), Type::Confirmable | Type::NonConfirmable)
        && engine
            .lookup_observe(ObserveKey::new_client(parsed.token(), peer))
            .is_some()
}

fn matching_exchange<Mem: Storage + Exchanges + PendingCons>(
    engine: &Engine<Mem>,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
) -> Option<crate::storage::ExchangeEntry<Mem::Token>> {
    let id = engine.lookup_exchange(ExchangeKey::new(parsed.token(), peer))?;
    let entry = engine.exchange_entry(id)?;
    (entry.matches_response(parsed) && !exchange_is_unsent(engine, entry)).then_some(entry)
}

// A reserved initial CON owns admission and bytes, but grants no response
// authority until the transport accepts it. Match the complete identity:
// an old NON exchange may refer to a TX slot reused by another request.
fn exchange_is_unsent<Mem: Storage + PendingCons>(
    engine: &Engine<Mem>,
    entry: ExchangeEntry<Mem::Token>,
) -> bool {
    engine.pending_con(entry.tx_slot()).is_some_and(|pending| {
        pending.message_id() == entry.message_id()
            && pending.endpoint() == entry.endpoint()
            && !pending.rto().is_started()
    })
}

fn take_exchange<Mem: Storage + Exchanges>(
    engine: &mut Engine<Mem>,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
) {
    let _ = engine.take_exchange(ExchangeKey::new(parsed.token(), peer));
}

fn needs_q_continue<Mem: Storage + BodySlots>(engine: &Engine<Mem>, id: SlotId) -> bool {
    let Some(transfer) = engine.rx_body_transfer(id) else {
        return false;
    };
    transfer.role() == BlockRole::IncomingQBlock2
        && !transfer.is_complete()
        && transfer.window_mask() == 0
        && transfer.window_base() > 0
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_block2_continue_async, bound(crate::app::async_server::ProtocolIo), calls(send_followup => send_followup_async))]
fn send_block2_continue<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
    body: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let Some(transfer) = engine.rx_body_transfer(body) else {
        return Ok(());
    };
    let Ok(block) = BlockValue::new(transfer.next_num(), false, transfer.szx()) else {
        return Ok(());
    };
    send_followup(
        engine,
        io,
        lives.get(Call::new(parsed.token(), peer)),
        ids,
        oscore,
        now_ms,
        parsed.token(),
        peer,
        Some(block),
        &[],
        None,
    )
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_q_block2_continue_async, bound(crate::app::async_server::ProtocolIo), calls(send_followup => send_followup_async))]
fn send_q_block2_continue<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
    body: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let Some(transfer) = engine.rx_body_transfer(body) else {
        return Ok(());
    };
    let Ok(block) = BlockValue::new(transfer.window_base(), true, transfer.szx()) else {
        return Ok(());
    };
    send_followup(
        engine,
        io,
        lives.get(Call::new(parsed.token(), peer)),
        ids,
        oscore,
        now_ms,
        parsed.token(),
        peer,
        None,
        &[block],
        None,
    )
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(recover_qblock2_io_async, bound(crate::app::async_server::ProtocolIo), calls(send_followup => send_followup_async))]
pub(crate) fn recover_qblock2<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    recover: crate::storage::QBlockRecover<Mem::Token>,
) -> Result<bool, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: DatagramIo,
{
    let call = Call::new(recover.key().token(), recover.key().endpoint());
    let Some(live) = lives.get(call) else {
        return Ok(false);
    };
    let mut nums = [0; crate::storage::BlockTransfer::MAX_PAYLOADS as usize];
    let count = recover.copy_missing_nums(&mut nums);
    let mut blocks = [BlockValue::new(0, false, 0).expect("valid block");
        crate::storage::BlockTransfer::MAX_PAYLOADS as usize];
    for (block, num) in blocks.iter_mut().zip(&nums[..count]) {
        *block = BlockValue::new(*num, false, recover.szx())
            .map_err(BlockTransferError::from)
            .map_err(Error::Block)?;
    }
    let outcome = send_followup(
        engine,
        io,
        Some(live),
        ids,
        oscore,
        now_ms,
        call.token(),
        call.peer,
        None,
        &blocks[..count],
        Some(Type::NonConfirmable),
    );
    if outcome.is_err() {
        abandon_send(engine, oscore, call.token(), call.peer);
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::ContinuationFailed,
            true,
        );
    }
    outcome.map(|()| true)
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_followup_async, bound(crate::app::async_server::ProtocolIo), calls(send_client_block1 => send_client_block1_async, super::finish_send => super::finish_send_async))]
fn send_followup<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    live: Option<&LiveCall<Mem::Token>>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    token: Mem::Token,
    peer: Endpoint,
    block2: Option<BlockValue>,
    q_block2: &[BlockValue],
    recovery_ty: Option<Type>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let ty = recovery_ty.unwrap_or_else(|| live.map(|live| live.ty).unwrap_or(Type::Confirmable));
    let code = live.map(|live| live.code).unwrap_or(Code::GET);
    let mut opts = OptionsBuilder::<
        { CLIENT_OPTION_SLOTS + crate::storage::BlockTransfer::MAX_PAYLOADS as usize - 1 },
    >::new();
    let accept = live.and_then(|live| live.accept).map(ContentFormat::encode);
    let no_response = live
        .and_then(|live| live.no_response)
        .map(crate::message::NoResponse::encode);
    let b2 = block2.map(BlockValue::encode);
    let mut q2 = [encode_uint(0); crate::storage::BlockTransfer::MAX_PAYLOADS as usize];
    if q_block2.len() > q2.len() {
        return Err(Error::Message(SlotMessageError::Encode(
            EncodeError::OptionsFull,
        )));
    }
    for (encoded, block) in q2.iter_mut().zip(q_block2) {
        *encoded = block.encode();
    }
    let retained_request = live
        .and_then(|live| live.request_identity.as_ref())
        .map(|identity| {
            crate::message::decode_with_token::<Mem::Token>(&identity.bytes[..identity.len])
                .expect("encoded request identity")
        });
    let cf = live
        .and_then(|live| live.content_format)
        .map(ContentFormat::encode);
    let filled = (|| -> Result<(), EncodeError> {
        if let Some(live) = live {
            if let Some(tag) = live.if_match.as_slice() {
                push_opt(&mut opts, Opt::if_match(tag))?;
            }
            if live.if_none_match {
                push_opt(&mut opts, Opt::if_none_match())?;
            }
            // Missing GET blocks retrieve bytes, rather than revalidate the
            // original cached response. Other methods retain their selection.
            if q_block2.is_empty()
                && code != Code::GET
                && let Some(tag) = live.etag.as_slice()
            {
                push_opt(&mut opts, Opt::etag(tag))?;
            }
        }
        if let Some(live) = live {
            for segment in live.path.segments() {
                push_opt(&mut opts, Opt::uri_path(segment))?;
            }
        }
        if let Some(ref encoded) = cf {
            push_opt(&mut opts, Opt::content_format(encoded))?;
        }
        if let Some(live) = live {
            for query in live.queries.values() {
                push_opt(
                    &mut opts,
                    Opt::new(crate::message::OptionNumber::URI_QUERY, query),
                )?;
            }
        }
        if let Some(ref encoded) = accept {
            push_opt(&mut opts, Opt::accept(encoded))?;
        }
        if let Some(ref encoded) = b2 {
            push_opt(&mut opts, Opt::block2(encoded))?;
        }
        for encoded in &q2[..q_block2.len()] {
            push_opt(&mut opts, Opt::q_block2(encoded))?;
        }
        if let Some(ref value) = no_response {
            push_opt(&mut opts, Opt::no_response(value))?;
        }
        if let Some(echo) = live.and_then(|live| live.echo.as_ref()) {
            push_opt(&mut opts, Opt::echo(echo.as_slice()))?;
        }
        if let Some(tag) = live.and_then(|live| live.request_tag.as_slice()) {
            push_opt(&mut opts, Opt::request_tag(tag))?;
        }
        if let Some(request) = retained_request.as_ref() {
            for option in request
                .options()
                .filter(|option| !option.number().is_known())
            {
                push_opt(&mut opts, option)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = filled {
        return Err(Error::Message(SlotMessageError::Encode(e)));
    }
    let mid = ids.next_for(engine, io, now_ms)?;
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(
            retained_request
                .as_ref()
                .filter(|_| code == Code::FETCH)
                .map(|request| request.payload())
                .unwrap_or(&[]),
        );
    let Some(tx) = super::admission::ordinary_tx(engine) else {
        return Err(Error::Saturated);
    };
    match super::oscore::encode_request(oscore, peer, engine, tx, &msg) {
        Ok(_) => {}
        Err(Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)))
            if code == Code::FETCH =>
        {
            let Some(live) = live else {
                let _ = engine.release_tx(tx);
                return Err(Error::Saturated);
            };
            // Q response selection uses Q-Block1; same-side classic/Q mixing is
            // forbidden. The existing request tag remains the operation identity.
            if !q_block2.is_empty() && live.request_tag.is_absent() {
                let _ = engine.release_tx(tx);
                return Err(Error::Block(BlockTransferError::MissingIdentity));
            }
            return send_client_block1(
                engine,
                io,
                ids,
                oscore,
                now_ms,
                peer,
                ty,
                code,
                token,
                live.path.segments(),
                &live.queries,
                live.accept,
                retained_request
                    .as_ref()
                    .expect("retained FETCH identity")
                    .payload(),
                live.request_tag,
                live.content_format,
                live.echo,
                live.no_response,
                live.if_match.as_slice(),
                if q_block2.is_empty() && code != Code::GET {
                    live.etag.as_slice()
                } else {
                    None
                },
                live.if_none_match,
                !q_block2.is_empty(),
                tx,
                mid,
                opts.as_slice(),
            )
            .map(|_| ());
        }
        Err(error) => {
            let _ = engine.release_tx(tx);
            return Err(error);
        }
    }
    match engine.record_request(tx, peer) {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Saturated);
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            return Err(e.into());
        }
    }
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid, ids.jitter()));
    super::finish_send(engine, io, tx, peer, pending)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(reissue_qblock1_missing_async, bound(crate::app::async_server::ProtocolIo), calls(send_block1_issued => send_block1_issued_async, super::completion_time => super::async_server::completion_time_async))]
fn reissue_qblock1_missing<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    parsed: &ParsedMessage<'_, Mem::Token>,
    peer: Endpoint,
    body: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let mut nums = [0; crate::storage::BlockTransfer::MAX_PAYLOADS as usize];
    let Ok(count) = crate::message::MissingBlocks::decode(parsed.payload(), &mut nums) else {
        return Ok(()); // RFC 9177 section 5: silently drop malformed ordering.
    };
    let Some(transfer) = engine.tx_body_transfer(body) else {
        return Ok(());
    };
    // Preflight the entire report, including only previously issued payloads.
    // A valid first NUM cannot cause output before an invalid later NUM.
    for &num in &nums[..count] {
        if num >= transfer.next_num() || engine.reissue_q_block1(body, num).is_err() {
            return Ok(());
        }
    }
    let Some(live) = lives.get_mut(Call::new(parsed.token(), peer)) else {
        return Ok(());
    };
    for &num in &nums[..count] {
        let issued = engine.reissue_q_block1(body, num).map_err(Error::Block)?;
        send_block1_issued(
            engine,
            io,
            ids,
            oscore,
            now_ms,
            peer,
            Type::NonConfirmable,
            live.code,
            parsed.token(),
            live.path.segments(),
            &live.queries,
            live.accept,
            live.content_format,
            live.echo,
            live.no_response,
            live.if_match.as_slice(),
            live.etag.as_slice(),
            live.if_none_match,
            issued,
            true,
            None,
        )?;
    }
    if count != 0 {
        let completed = super::completion_time(io, now_ms);
        arm_q_pacing(engine, live, Some(body), ids, completed);
    }
    Ok(())
}

// Only success-suppressed NON uploads use timer progression. The last packet's
// bounded unbiased 0..=1000ms jitter sample also schedules its next payload set.
fn arm_q_pacing<Mem: Storage + BodySlots>(
    engine: &Engine<Mem>,
    live: &mut LiveCall<Mem::Token>,
    body: Option<SlotId>,
    ids: &AppIds,
    now_ms: u64,
) {
    live.q_pace_ms = if live.ty == Type::NonConfirmable
        && live
            .no_response
            .is_some_and(|mask| mask.suppresses(Code::CONTINUE))
        && body
            .and_then(|id| engine.tx_body_transfer(id))
            .is_some_and(|transfer| {
                transfer.role() == BlockRole::OutgoingQBlock1
                    && !transfer.is_complete()
                    && transfer.window_mask()
                        == (1u16 << crate::storage::BlockTransfer::MAX_PAYLOADS) - 1
            }) {
        now_ms
            .checked_add(u64::from(
                crate::message::QBlockTransmission::NON_TIMEOUT_MS + ids.jitter(),
            ))
            .unwrap_or(0)
    } else {
        0
    };
}

/// At most one due payload set per poll, oldest deadline first. Missed time does
/// not accumulate send credit. Ingress runs first so valid Continue/recovery can
/// disarm or rearm this opportunity before selection.
#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(pace_qblock1_io_async, bound(crate::app::async_server::ProtocolIo), calls(continue_block1_tx => continue_block1_tx_async))]
pub(crate) fn pace_qblock1<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: DatagramIo,
{
    let Some(live) = lives
        .rows
        .iter_mut()
        .flatten()
        .filter(|live| live.q_pace_ms != 0 && live.q_pace_ms <= now_ms)
        .min_by_key(|live| live.q_pace_ms)
    else {
        return Ok(());
    };
    // Clear before attempting: every success rearms from now, every failure
    // retires the Call without changing the uncertainty of remote effects.
    live.q_pace_ms = 0;
    let call = live.call;
    let key = BlockKey::new(call.token, call.peer).with_identity(live.request_tag);
    let Some((body, _)) = upload_body(engine, || key) else {
        return Ok(());
    };
    let result = match engine.pace_q_block1(body).map_err(Error::Block) {
        Ok(()) => continue_block1_tx(
            engine,
            io,
            lives,
            ids,
            oscore,
            now_ms,
            call.token(),
            None,
            None,
            call.peer,
            body,
        ),
        Err(error) => Err(error),
    };
    if result.is_err() {
        abandon_send(engine, oscore, call.token(), call.peer);
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::ContinuationFailed,
            true,
        );
    }
    result
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(continue_qblock1_ack_io_async, bound(crate::app::async_server::ProtocolIo), calls(continue_block1_tx => continue_block1_tx_async))]
pub(crate) fn continue_qblock1_ack<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    mid: MessageId,
    peer: Endpoint,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: DatagramIo,
{
    // Called only after a matching pending CON was taken. Duplicate/wrong
    // endpoint/MID ACKs cannot drive the next payload.
    let Some(entry) = exchange_for_mid(engine, mid, peer) else {
        return Ok(());
    };
    let call = Call::new(entry.token(), peer);
    let Some(live) = lives.get(call) else {
        return Ok(());
    };
    if live.ty != Type::Confirmable {
        return Ok(());
    }
    let Some((body, transfer)) = upload_body(engine, || {
        BlockKey::new(call.token, call.peer).with_identity(live.request_tag)
    }) else {
        return Ok(());
    };
    if transfer.role != BlockRole::OutgoingQBlock1 || transfer.complete {
        return Ok(());
    }
    let acknowledged = if transfer.full_window {
        // Every earlier payload was individually ACKed before the next
        // was issued, so this ACK confirms the entire current set.
        engine
            .ack_q_block1(body, transfer.num)
            .map_err(Error::Block)
            .map(|_| ())
    } else {
        Ok(())
    };
    let outcome = match acknowledged {
        Ok(()) => continue_block1_tx(
            engine,
            io,
            lives,
            ids,
            oscore,
            now_ms,
            call.token(),
            None,
            None,
            peer,
            body,
        ),
        Err(error) => Err(error),
    };
    if outcome.is_err() {
        abandon_send(engine, oscore, call.token(), peer);
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::ContinuationFailed,
            true,
        );
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(continue_block1_tx_async, bound(crate::app::async_server::ProtocolIo), calls(issue_block1 => issue_block1_async, issue_q_block1_window => issue_q_block1_window_async, super::completion_time => super::async_server::completion_time_async))]
fn continue_block1_tx<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    token: Mem::Token,
    block1: Option<BlockValue>,
    q_ack: Option<BlockValue>,
    peer: Endpoint,
    body: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let Some(transfer) = engine.tx_body_transfer(body) else {
        return Ok(());
    };
    if let Some(preferred) = block1
        && transfer.role() == BlockRole::OutgoingBlock1
        && preferred.szx() < transfer.szx()
    {
        engine
            .adopt_smaller_block1(body, preferred.szx())
            .map_err(Error::Block)?;
    }
    if transfer.is_complete() {
        return Ok(());
    }
    let row = lives.get_mut(Call::new(token, peer));
    let live = row.as_deref();
    let ty = live.map(|live| live.ty).unwrap_or(Type::Confirmable);
    let code = live.map(|live| live.code).unwrap_or(Code::PUT);
    let content_format = live.and_then(|live| live.content_format);
    let echo = live.and_then(|live| live.echo);
    let no_response = live.and_then(|live| live.no_response);
    let if_match = live.and_then(|live| live.if_match.as_slice());
    let etag = live.and_then(|live| live.etag.as_slice());
    let if_none_match = live.is_some_and(|live| live.if_none_match);
    let segments: &[&str] = live.map_or(&[], |live| live.path.segments());
    let queries = live.map_or(&RetainedQueries::EMPTY, |live| &live.queries);
    let accept = live.and_then(|live| live.accept);
    match transfer.role() {
        BlockRole::OutgoingBlock1 => {
            let _ = engine.take_exchange(ExchangeKey::new(token, peer));
            issue_block1(
                engine,
                io,
                ids,
                oscore,
                now_ms,
                peer,
                ty,
                code,
                token,
                segments,
                queries,
                accept,
                content_format,
                echo,
                no_response,
                if_match,
                etag,
                if_none_match,
                body,
                None,
            )
        }
        BlockRole::OutgoingQBlock1 => {
            let paced = ty == Type::NonConfirmable
                && no_response.is_some_and(|mask| mask.suppresses(Code::CONTINUE));
            if paced {
                if let Some(q) = q_ack {
                    if transfer
                        .window_base()
                        .checked_add(u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS) - 1)
                        != Some(q.num())
                        || q.szx() != transfer.szx()
                        || !q.more()
                    {
                        return Ok(());
                    }
                } else if transfer.window_mask() != 0 {
                    // A response without the current set's valid Continue must
                    // neither grant send credit nor postpone its pacing timer.
                    return Ok(());
                }
            }
            if let Some(q) = q_ack {
                engine.ack_q_block1(body, q.num()).map_err(Error::Block)?;
            }
            issue_q_block1_window(
                engine,
                io,
                ids,
                oscore,
                now_ms,
                peer,
                ty,
                code,
                token,
                segments,
                queries,
                accept,
                content_format,
                echo,
                no_response,
                if_match,
                etag,
                if_none_match,
                body,
                None,
                Some((token, peer)),
            )
            .map(|_| ())?;
            let completed = super::completion_time(io, now_ms);
            if let Some(live) = row {
                arm_q_pacing(engine, live, Some(body), ids, completed);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_block1_async, bound(crate::app::async_server::ProtocolIo), calls(send_block1_issued => send_block1_issued_async))]
fn issue_block1<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    body: SlotId,
    tx: Option<SlotId>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let issued = engine.next_block1(body).map_err(Error::Block)?;
    send_block1_issued(
        engine,
        io,
        ids,
        oscore,
        now_ms,
        dest,
        ty,
        code,
        token,
        path,
        queries,
        accept,
        content_format,
        echo,
        no_response,
        if_match,
        etag,
        if_none_match,
        issued,
        false,
        tx,
    )
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(issue_q_block1_window_async, bound(crate::app::async_server::ProtocolIo), calls(send_block1_issued => send_block1_issued_async))]
fn issue_q_block1_window<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    dest: Endpoint,
    first_ty: Type,
    code: Code,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    body: SlotId,
    mut tx: Option<SlotId>,
    take_first: Option<(Mem::Token, Endpoint)>,
) -> Result<bool, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let mut sent = false;
    loop {
        let issued = match engine.next_q_block1(body) {
            Ok(issued) => issued,
            Err(BlockTransferError::OutsideWindow) => return Ok(sent),
            Err(e) => return Err(Error::Block(e)),
        };
        if !sent && let Some((tok, ep)) = take_first {
            let _ = engine.take_exchange(ExchangeKey::new(tok, ep));
        }
        let ty = first_ty;
        send_block1_issued(
            engine,
            io,
            ids,
            oscore,
            now_ms,
            dest,
            ty,
            code,
            token,
            path,
            queries,
            accept,
            content_format,
            echo,
            no_response,
            if_match,
            etag,
            if_none_match,
            issued,
            true,
            tx.take(),
        )?;
        sent = true;
        // CON uploads have one outstanding payload; the matching empty ACK
        // opens the next send. NON retains its bounded payload-set burst.
        if first_ty == Type::Confirmable || issued.complete() {
            return Ok(true);
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_block1_issued_async, bound(crate::app::async_server::ProtocolIo), calls(finish_client_send => finish_client_send_async))]
fn send_block1_issued<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    issued: OutgoingBlock,
    q_block1: bool,
    tx: Option<SlotId>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let staged = stage_block1_issued(
        engine,
        io,
        ids,
        oscore,
        now_ms,
        dest,
        ty,
        code,
        token,
        path,
        queries,
        accept,
        content_format,
        echo,
        no_response,
        if_match,
        etag,
        if_none_match,
        issued,
        q_block1,
        tx,
    )?;
    finish_client_send(
        engine,
        io,
        staged.tx,
        dest,
        ty,
        now_ms,
        staged.mid,
        staged.jitter,
    )
}

#[allow(clippy::too_many_arguments)]
fn stage_block1_issued<Mem, T, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    issued: OutgoingBlock,
    q_block1: bool,
    tx: Option<SlotId>,
) -> Result<StagedClient, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: crate::storage::DatagramTransport,
{
    let mut chunk = [0u8; 1024];
    let n = copy_tx_range(engine, issued, &mut chunk).map_err(Error::Block)?;
    let tag = engine
        .tx_body_transfer(issued.id())
        .map(|t| t.identity())
        .unwrap_or(BodyTag::ABSENT);
    let size1 = engine
        .tx_body_transfer(issued.id())
        .map(|t| t.filled())
        .and_then(|len| u32::try_from(len).ok());
    let tx = match tx {
        Some(tx) => tx,
        None => super::admission::ordinary_tx(engine).ok_or(Error::Saturated)?,
    };
    let mid = match ids.next_for(engine, io, now_ms) {
        Ok(mid) => mid,
        Err(error) => {
            let _ = engine.release_tx(tx);
            return Err(error);
        }
    };
    let transfer = engine.tx_body_transfer(issued.id());
    let stored = transfer
        .as_ref()
        .and_then(|transfer| transfer.upload_controls());
    let mut controls =
        OptionsBuilder::<{ crate::storage::BlockTransfer::MAX_PAYLOADS as usize + 2 }>::new();
    if let Some(stored) = stored {
        for option in stored.options() {
            if let Err(error) = push_opt(&mut controls, Opt::new(option.number(), option.value())) {
                let _ = engine.release_tx(tx);
                return Err(Error::Message(SlotMessageError::Encode(error)));
            }
        }
    }
    if let Err(error) = encode_block1_request(
        dest,
        engine,
        oscore,
        tx,
        ty,
        code,
        mid,
        token,
        path,
        queries,
        accept,
        content_format,
        echo,
        no_response,
        if_match,
        etag,
        if_none_match,
        issued.block(),
        q_block1,
        tag,
        size1,
        &chunk[..n],
        controls.as_slice(),
    ) {
        let _ = engine.release_tx(tx);
        return Err(error);
    }

    Ok(StagedClient {
        tx,
        mid,
        jitter: ids.jitter(),
        body: None,
        continue_q: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn encode_block1_request<Mem, E, const REQUESTS: usize>(
    dest: Endpoint,
    engine: &mut Engine<Mem>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    tx: SlotId,
    ty: Type,
    code: Code,
    mid: MessageId,
    token: Mem::Token,
    path: &[&str],
    queries: &RetainedQueries,
    accept: Option<ContentFormat>,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    block: BlockValue,
    q_block1: bool,
    tag: BodyTag,
    size1: Option<u32>,
    payload: &[u8],
    controls: &[Opt<'_>],
) -> Result<(), Error<E>>
where
    Mem: Storage + DatagramSlots,
{
    let cf = content_format.map(ContentFormat::encode);
    let acc = accept.map(ContentFormat::encode);
    let blk = block.encode();
    let size = size1.map(encode_uint);
    let no_response = no_response.map(crate::message::NoResponse::encode);
    let mut opts = OptionsBuilder::<
        { CLIENT_OPTION_SLOTS + crate::storage::BlockTransfer::MAX_PAYLOADS as usize + 2 },
    >::new();
    let filled = (|| -> Result<(), EncodeError> {
        for option in controls.iter().filter(|o| {
            !o.number().is_known()
                || matches!(
                    o.number(),
                    crate::message::OptionNumber::OBSERVE
                        | crate::message::OptionNumber::BLOCK2
                        | crate::message::OptionNumber::Q_BLOCK2
                )
        }) {
            push_opt(&mut opts, *option)?;
        }
        if let Some(value) = if_match {
            push_opt(&mut opts, Opt::if_match(value))?;
        }
        if let Some(value) = etag {
            push_opt(&mut opts, Opt::etag(value))?;
        }
        if if_none_match && block.num() == 0 {
            push_opt(&mut opts, Opt::if_none_match())?;
        }
        for segment in path {
            push_opt(&mut opts, Opt::uri_path(segment))?;
        }
        if let Some(ref encoded) = cf {
            push_opt(&mut opts, Opt::content_format(encoded))?;
        }
        for query in queries.values() {
            push_opt(
                &mut opts,
                Opt::new(crate::message::OptionNumber::URI_QUERY, query),
            )?;
        }
        if let Some(ref encoded) = acc {
            push_opt(&mut opts, Opt::accept(encoded))?;
        }
        if q_block1 {
            push_opt(&mut opts, Opt::q_block1(&blk))?;
        } else {
            push_opt(&mut opts, Opt::block1(&blk))?;
        }
        if let Some(ref encoded) = size {
            push_opt(&mut opts, Opt::size1(encoded))?;
        }
        if let Some(ref value) = no_response {
            push_opt(&mut opts, Opt::no_response(value))?;
        }
        if let Some(echo) = echo.as_ref() {
            push_opt(&mut opts, Opt::echo(echo.as_slice()))?;
        }
        if let Some(tag) = tag.as_slice() {
            push_opt(&mut opts, Opt::request_tag(tag))?;
        }
        Ok(())
    })();
    if let Err(e) = filled {
        return Err(Error::Message(SlotMessageError::Encode(e)));
    }
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    super::oscore::encode_request(oscore, dest, engine, tx, &msg)
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(finish_client_send_async, bound(crate::app::async_server::ProtocolIo), calls(super::finish_send => super::finish_send_async))]
fn finish_client_send<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    tx: SlotId,
    dest: Endpoint,
    ty: Type,
    now_ms: u64,
    mid: MessageId,
    jitter: u32,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges,
    T: DatagramIo,
{
    let exchange = match engine.record_request(tx, dest) {
        Ok(Some(entry)) => entry.key(),
        Ok(None) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Saturated);
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            return Err(e.into());
        }
    };
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid, jitter));
    match super::finish_send(engine, io, tx, dest, pending) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = engine.take_exchange(exchange);
            Err(e)
        }
    }
}

// A send can fail after earlier Q-Block datagrams went out. No Call is
// returned, so retire all local state owned by this request. Never rewind
// OSCORE sequence numbers: a transport error does not prove non-delivery.
fn abandon_send<Mem, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    token: Mem::Token,
    peer: Endpoint,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
{
    let _ = engine.take_exchange(ExchangeKey::new(token, peer));
    for i in 0..engine.capacities().tx_datagram_slots {
        let id = SlotId::from_index(i);
        let Some(pending) = engine.pending_con(id) else {
            continue;
        };
        if engine.tx_endpoint(id) == Some(peer)
            && engine
                .decode_tx(id)
                .is_ok_and(|message| message.token() == token && message.code().is_request())
        {
            let _ = engine.take_pending_con(pending.message_id(), peer);
            let _ = engine.release_tx(id);
        }
    }
    for i in 0..engine.capacities().tx_body_slots.unwrap_or(0) {
        let id = SlotId::from_index(i);
        if engine.tx_body_transfer(id).is_some_and(|transfer| {
            transfer.key().token() == token
                && transfer.key().endpoint() == peer
                && matches!(
                    transfer.role(),
                    BlockRole::OutgoingBlock1 | BlockRole::OutgoingQBlock1
                )
        }) {
            let _ = engine.release_tx_body(id);
        }
    }
    super::oscore::cancel(oscore, peer, token);
}

fn copy_tx_range<Mem: Storage + BodySlots>(
    engine: &Engine<Mem>,
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

// Keep only the facts callers use across a mutation. A full transfer also
// contains request bindings and notification metadata.
struct UploadProgress {
    role: BlockRole,
    num: u32,
    complete: bool,
    full_window: bool,
}

fn upload_body<Mem: Storage + BodySlots>(
    engine: &mut Engine<Mem>,
    key: impl FnOnce() -> BlockKey<Mem::Token>,
) -> Option<(SlotId, UploadProgress)> {
    let occupied = super::tx_body_slots_to_scan(engine);
    if occupied.is_empty() {
        return None;
    }
    let key = key();
    let mut q = None;
    for i in occupied {
        let id = SlotId::from_index(i);
        let Some(transfer) = engine.tx_body_transfer(id) else {
            continue;
        };
        if transfer.key() != key {
            continue;
        }
        let progress = || UploadProgress {
            role: transfer.role(),
            num: transfer.num(),
            complete: transfer.is_complete(),
            full_window: transfer.window_mask()
                == (1u16 << crate::storage::BlockTransfer::MAX_PAYLOADS) - 1,
        };
        match transfer.role() {
            BlockRole::OutgoingBlock1 => return Some((id, progress())),
            BlockRole::OutgoingQBlock1 if q.is_none() => q = Some((id, progress())),
            _ => {}
        }
    }
    q
}

fn client_observe_live<Mem: Storage + ObserveSlots>(
    engine: &Engine<Mem>,
    call: Call<Mem::Token>,
) -> bool {
    let key = ObserveKey::new_client(call.token(), call.peer);
    engine.lookup_observe(key).is_some()
}

fn accept_client_observe<Mem, const CALLS: usize>(
    engine: &mut Engine<Mem>,
    lives: &ClientLives<CALLS, Mem::Token>,
    call: Call<Mem::Token>,
    observe_age: Option<u32>,
    now_ms: u64,
) where
    Mem: Storage + ObserveSlots,
{
    let key = ObserveKey::new_client(call.token(), call.peer);
    if let Some(max_age) = observe_age {
        let mode = lives.get(call).map(|live| live.observe);
        if mode != Some(OutgoingObserve::Deregister)
            && (mode == Some(OutgoingObserve::Register) || engine.lookup_observe(key).is_some())
        {
            if let Some(id) =
                engine.insert_observe(ObserveInterest::new_client(call.token(), call.peer))
            {
                let _ = engine.refresh_observe_lifetime_at(
                    id,
                    crate::storage::ObserveLifetime::max_age(now_ms, max_age),
                );
            }
            return;
        }
    }
    let _ = engine.take_observe(key);
}

/// Retire a pending request with its actual completion failure classification.
pub(crate) fn fail_pending_client<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    pending: PendingCon,
    failure: CallFailure,
) where
    Mem: Storage + DatagramSlots + Exchanges + BodySlots + ObserveSlots + PendingCons,
{
    if let Some(entry) = exchange_for_mid(engine, pending.message_id(), pending.endpoint()) {
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            Call::new(entry.token(), entry.endpoint()),
            failure,
            false,
        );
    }
}

fn exchange_for_mid<Mem: Storage + Exchanges>(
    engine: &Engine<Mem>,
    message_id: MessageId,
    peer: Endpoint,
) -> Option<ExchangeEntry<Mem::Token>> {
    let n = engine.capacities().tx_datagram_slots;
    (0..n).find_map(|i| {
        let entry = engine.exchange_entry(SlotId::from_index(i))?;
        (entry.message_id() == message_id && entry.endpoint() == peer).then_some(entry)
    })
}

// Retire a timed-out Q download in the same poll that reclaims its body.
// The inbox must not keep a partial SlotId that another transfer can reuse.
pub(crate) fn qblock_give_up<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    key: ExchangeKey<Mem::Token>,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    let call = Call::new(key.token(), key.endpoint());
    if lives.get(call).is_some() {
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::TimedOut,
            true,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn fail_call<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    call: Call<Mem::Token>,
    failure: CallFailure,
    release_pending: bool,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    fail_call_row(
        engine,
        inbox,
        oscore,
        call,
        lives.get_mut(call),
        failure,
        release_pending,
    );
}

fn fail_call_row<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    call: Call<Mem::Token>,
    live: Option<&mut LiveCall<Mem::Token>>,
    failure: CallFailure,
    release_pending: bool,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    let key = ExchangeKey::new(call.token(), call.peer);
    if let Some(entry) = engine.take_exchange(key) {
        // NON/empty ACK may have released and reused the old TX slot. Only
        // a matching pending CON still establishes ownership of that slot.
        if release_pending {
            let reserved = engine.pending_con(entry.tx_slot()).filter(|pending| {
                pending.message_id() == entry.message_id()
                    && pending.endpoint() == entry.endpoint()
                    && !pending.rto().is_started()
            });
            let tx = if let Some(reserved) = reserved {
                engine
                    .cancel_pending_con(reserved)
                    .then_some(entry.tx_slot())
            } else {
                engine.take_pending_con(entry.message_id(), entry.endpoint())
            };
            if let Some(tx) = tx {
                let _ = engine.release_tx(tx);
            }
        }
    }
    let _ = engine.take_observe(ObserveKey::new_client(call.token(), call.peer));
    if let Some(slot) = inbox.slot_for(call) {
        replace_reply(
            engine,
            slot,
            Some(InboxRow {
                call,
                meta: Err(failure),
                storage: ReplyStorage::Inline,
            }),
        );
    }
    for i in 0..engine.capacities().rx_body_slots.unwrap_or(0) {
        let id = SlotId::from_index(i);
        if engine.rx_body_transfer(id).is_some_and(|transfer| {
            transfer.key().token() == call.token()
                && transfer.key().endpoint() == call.peer
                && matches!(
                    transfer.role(),
                    BlockRole::IncomingBlock2 | BlockRole::IncomingQBlock2
                )
        }) {
            let _ = engine.release_rx_body(id);
        }
    }
    if let Some((body, _)) = upload_body(engine, || {
        BlockKey::new(call.token, call.peer)
            .with_identity(live.as_ref().map_or(BodyTag::ABSENT, |row| row.request_tag))
    }) {
        let _ = engine.release_tx_body(body);
    }
    super::oscore::cancel(oscore, call.peer, call.token());
    // Untaken failures occupy the same bounded completion budget as replies.
    if let Some(live) = live {
        live.due_ms = 0;
        live.deadline_ms = None;
    }
}

pub(crate) fn complete_client_rst<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    message_id: MessageId,
    peer: Endpoint,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    // The original TX slot can already hold an unrelated separate response.
    let entry = exchange_for_mid(engine, message_id, peer)
        .filter(|entry| !exchange_is_unsent(engine, *entry));
    if let Some(entry) = entry {
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            Call::new(entry.token(), entry.endpoint()),
            CallFailure::Reset,
            false,
        );
    }
}

#[cfg(test)]
mod reservation_tests {
    use super::*;
    use crate::storage::{EngineBuilder, Memory, PendingCon, profiles};

    fn reserved_request() -> (Engine<Memory<profiles::Default>>, PendingCon, ExchangeEntry) {
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .unwrap();
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let tx = engine.acquire_tx().unwrap();
        let mid = MessageId::new(801);
        engine
            .encode_tx(tx, &Message::new(Type::Confirmable, Code::GET, mid))
            .unwrap();
        let entry = engine.record_request(tx, peer).unwrap().unwrap();
        let reserved = engine.reserve_pending_con(tx, peer, mid).unwrap();
        (engine, reserved, entry)
    }

    #[test]
    fn refused_block_send_on_reused_tx_removes_only_its_inserted_exchange() {
        struct Refuse;
        impl crate::storage::DatagramTransport for Refuse {
            type Error = ();
        }
        impl DatagramIo for Refuse {
            fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
                Ok(None)
            }
            fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, ()> {
                Err(())
            }
        }

        let (mut engine, reserved, first) = reserved_request();
        engine.commit_pending_con(reserved, 0, 0).unwrap();
        let peer = first.endpoint();
        let tx = engine.take_pending_con(first.message_id(), peer).unwrap();
        engine.release_tx(tx).unwrap();
        for _ in 0..engine.capacities().tx_datagram_slots {
            if engine.tx_cursor() == tx.index() {
                break;
            }
            engine.rotate_tx();
        }
        assert_eq!(engine.acquire_tx(), Some(tx));
        let mid = MessageId::new(802);
        let token = Token::new(b"blk").unwrap();
        engine
            .encode_tx(
                tx,
                &Message::new(Type::Confirmable, Code::PUT, mid).with_token(token),
            )
            .unwrap();
        let sent = finish_client_send(
            &mut engine,
            &mut Refuse,
            tx,
            peer,
            Type::Confirmable,
            1,
            mid,
            0,
        );
        assert!(matches!(sent, Err(Error::Io(_))));
        assert!(engine.lookup_exchange(first.key()).is_some());
        assert!(
            engine
                .lookup_exchange(ExchangeKey::new(token, peer))
                .is_none()
        );
        assert_eq!(engine.tx_occupied(), 0);
    }

    #[test]
    fn initial_reservation_rejects_responses_until_acceptance() {
        let (mut engine, reserved, entry) = reserved_request();
        let mut wire = [0; 32];
        for ty in [
            Type::Acknowledgement,
            Type::Confirmable,
            Type::NonConfirmable,
        ] {
            let len = crate::message::encode(
                &Message::new(ty, Code::CONTENT, entry.message_id()),
                &mut wire,
            )
            .unwrap();
            let parsed = crate::message::decode(&wire[..len]).unwrap();
            assert!(!has_response_target(&engine, &parsed, entry.endpoint()));
        }
        engine.commit_pending_con(reserved, 40, 0).unwrap();
        let len = crate::message::encode(
            &Message::new(Type::Acknowledgement, Code::CONTENT, entry.message_id()),
            &mut wire,
        )
        .unwrap();
        let parsed = crate::message::decode(&wire[..len]).unwrap();
        assert!(has_response_target(&engine, &parsed, entry.endpoint()));
    }

    #[test]
    fn initial_reservation_rst_does_not_retire_exchange() {
        let (mut engine, reserved, entry) = reserved_request();
        let mut inbox = ClientInbox::<1>::new();
        let mut lives = ClientLives::<1>::new();
        let mut oscore = super::super::oscore::Store::<1, 1>::new();
        complete_client_rst(
            &mut engine,
            &mut inbox,
            &mut lives,
            &mut oscore,
            entry.message_id(),
            entry.endpoint(),
        );
        let call = Call::new(entry.token(), entry.endpoint());
        assert!(!inbox.contains(call));
        assert!(engine.lookup_exchange(entry.key()).is_some());
        assert_eq!(engine.pending_con(entry.tx_slot()), Some(reserved));
        engine.commit_pending_con(reserved, 40, 0).unwrap();
        complete_client_rst(
            &mut engine,
            &mut inbox,
            &mut lives,
            &mut oscore,
            entry.message_id(),
            entry.endpoint(),
        );
        assert!(inbox.contains(call));
        assert!(engine.lookup_exchange(entry.key()).is_none());
    }

    #[test]
    fn initial_reservation_local_failure_reclaims_tx_and_admission() {
        let (mut engine, _, entry) = reserved_request();
        let mut inbox = ClientInbox::<1>::new();
        let mut lives = ClientLives::<1>::new();
        let mut oscore = super::super::oscore::Store::<1, 1>::new();
        let call = Call::new(entry.token(), entry.endpoint());
        fail_call(
            &mut engine,
            &mut inbox,
            &mut lives,
            &mut oscore,
            call,
            CallFailure::TimedOut,
            true,
        );
        assert!(inbox.contains(call));
        assert!(engine.lookup_exchange(entry.key()).is_none());
        assert!(engine.pending_con(entry.tx_slot()).is_none());
        assert_eq!(engine.tx_occupied(), 0);
        assert!(engine.acquire_tx().is_some());
    }

    #[test]
    fn unrelated_reservation_on_reused_tx_preserves_old_exchange_authority() {
        let (mut engine, reserved, entry) = reserved_request();
        assert!(engine.cancel_pending_con(reserved));
        let other_mid = MessageId::new(802);
        let other = engine
            .reserve_pending_con(entry.tx_slot(), entry.endpoint(), other_mid)
            .unwrap();
        assert!(!exchange_is_unsent(&engine, entry));
        let mut inbox = ClientInbox::<1>::new();
        let mut lives = ClientLives::<1>::new();
        let mut oscore = super::super::oscore::Store::<1, 1>::new();
        fail_call(
            &mut engine,
            &mut inbox,
            &mut lives,
            &mut oscore,
            Call::new(entry.token(), entry.endpoint()),
            CallFailure::TimedOut,
            true,
        );
        assert_eq!(engine.pending_con(entry.tx_slot()), Some(other));
        assert_eq!(engine.tx_occupied(), 1);
    }
}

pub(crate) fn expire_client_exchanges<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS, Mem::Token>,
    lives: &mut ClientLives<CALLS, Mem::Token>,
    oscore: &mut super::oscore::Field<REQUESTS, Mem::Token>,
    now_ms: u64,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    // A failure only clears this Call's owners. The live table keeps stable
    // slots, so unrelated Calls can be examined and expired in the same pass.
    for slot in &mut lives.rows {
        ClientLives::<CALLS, _>::expire_backoff(slot, now_ms);
        let Some(live) = slot.as_mut() else {
            continue;
        };
        // Most polls precede both timers. Inspect only this row's metadata until
        // expiry is possible; completed replies and Observe still decide whether
        // a due timer may terminate the Call.
        let deadline_reached = live.deadline_ms.is_some_and(|deadline| now_ms >= deadline);
        let lifetime_reached = live.due_ms != 0 && now_ms >= live.due_ms;
        if live.retired || (!deadline_reached && !lifetime_reached) {
            continue;
        }
        if inbox.contains(live.call) && !client_observe_live(engine, live.call) {
            continue;
        }
        let failure = if deadline_reached {
            Some(CallFailure::DeadlineExceeded)
        } else if lifetime_reached
            && engine
                .lookup_exchange(ExchangeKey::new(live.call.token(), live.call.peer))
                .is_some()
        {
            Some(CallFailure::TimedOut)
        } else {
            None
        };
        if let Some(failure) = failure {
            fail_call_row(engine, inbox, oscore, live.call, Some(live), failure, true);
        }
    }
}

#[cfg(feature = "oscore")]
pub(super) fn revoke_association<S: super::AppStorage, const C: usize, const R: usize>(
    engine: &mut Engine<S>,
    inbox: &mut ClientInbox<C, S::Token>,
    lives: &mut ClientLives<C, S::Token>,
    oscore: &mut super::oscore::Field<R, S::Token>,
    peer: Endpoint,
) {
    for i in 0..C {
        if let Some(call) = lives.rows[i]
            .as_ref()
            .map(|live| live.call)
            .filter(|call| call.peer == peer)
        {
            fail_call(
                engine,
                inbox,
                lives,
                oscore,
                call,
                CallFailure::AssociationRevoked,
                true,
            );
            lives.rows[i] = None;
        }
    }
}
