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
pub(crate) use asynchronous::{
    complete_client_async, continue_qblock1_ack_async, pace_qblock1_async, recover_qblock2_async,
};

use crate::error::{BlockTransferError, EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, Message, MessageId, Opt, OptionsBuilder, ParsedMessage, Token,
    Transmission, Type, encode_uint,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, BodyTag, DatagramIo, DatagramSlots, Endpoint, Engine,
    ExchangeEntry, ExchangeKey, Exchanges, MemoryLayout, Missing, ObserveInterest, ObserveKey,
    ObserveSlots, OutgoingBlock, PendingCons, Present, SlotId, Storage,
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
pub struct Call {
    token: Token,
    peer: Endpoint,
}

impl Call {
    pub(crate) const fn new(token: Token, peer: Endpoint) -> Self {
        Self { token, peer }
    }

    /// Token sent on the request (and expected on the response).
    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    /// Destination of the request / source of the matched response.
    #[must_use]
    pub const fn peer(self) -> Endpoint {
        self.peer.physical()
    }
}

/// Caller-buffer collection failed without consuming the completed reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseBufferError {
    /// The complete representation does not fit; retry with a larger buffer.
    TooSmall {
        /// Exact complete representation length.
        required: usize,
        /// Supplied output capacity.
        available: usize,
    },
    /// Complete bytes are unavailable (legacy truncated snapshot or invalid storage).
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
    // Canonically encoded code, Token, MID and selected options. Capture time
    // and copied payload bookkeeping are not representation-header identity.
    pub(super) fn same_header(&self, other: &Self) -> bool {
        self.peer == other.peer
            && self.header[..usize::from(self.header_len)]
                == other.header[..usize::from(other.header_len)]
    }

    pub(super) fn from_parsed(
        parsed: &ParsedMessage<'_>,
        peer: Endpoint,
        received_at_ms: u64,
    ) -> Result<Self, CallFailure> {
        let mut options = OptionsBuilder::<RESPONSE_OPTION_COUNT>::new();
        let mut locations = [0usize; 2];
        for option in parsed.options() {
            let index = match option.number() {
                crate::message::OptionNumber::LOCATION_PATH => Some(0),
                crate::message::OptionNumber::LOCATION_QUERY => Some(1),
                _ => None,
            };
            if let Some(index) = index {
                locations[index] += 1;
                if locations[index] > super::LOCATION_MAX {
                    return Err(CallFailure::ResponseMetadataBounds);
                }
            }
            options
                .push(option)
                .map_err(|_| CallFailure::ResponseMetadataBounds)?;
        }
        let message = Message::new(parsed.ty(), parsed.code(), parsed.message_id())
            .with_token(parsed.token())
            .with_options(options.as_slice());
        let mut header = [0; RESPONSE_OPTION_BYTES];
        let header_len = crate::message::encode(&message, &mut header)
            .map_err(|_| CallFailure::ResponseMetadataBounds)?;
        let src = parsed.payload();
        let n = src.len().min(INLINE_PAYLOAD);
        let mut payload = [0u8; INLINE_PAYLOAD];
        payload[..n].copy_from_slice(&src[..n]);
        Ok(Self {
            received_at_ms,
            header,
            header_len: header_len as u16,
            peer,
            payload,
            payload_len: n as u16,
            payload_src_len: u16::try_from(src.len()).unwrap_or(u16::MAX),
        })
    }

    pub(super) fn response(&self) -> Response<'_> {
        // The stored header was produced by the encoder and is immutable.
        let parsed = crate::message::decode(&self.header[..usize::from(self.header_len)])
            .expect("encoded response header");
        let mut response = Response::from_client(
            parsed.code(),
            parsed.ty(),
            parsed.token(),
            parsed.message_id(),
            self.peer,
            &self.payload[..usize::from(self.payload_len)],
            parsed.content_format().and_then(Result::ok),
        )
        .with_payload_src_len(self.payload_src_len);
        if let Some(etag) = parsed
            .etag()
            .next()
            .filter(|tag| (1..=8).contains(&tag.len()))
        {
            response = response.etag(etag);
        }
        if let Some(seq) = parsed.observe().and_then(Result::ok) {
            response = response.observe(seq);
        }
        if let Some(age) = parsed.max_age().and_then(Result::ok) {
            response = response.max_age(age);
        }
        if let Ok(Some(echo)) = crate::message::Echo::from_message(&parsed) {
            response = response.echo(echo);
        }
        for path in parsed.location_path().flatten() {
            if path.len() <= 255 && path != "." && path != ".." {
                response = response.location_path(path);
            }
        }
        for query in parsed.location_query().flatten() {
            if query.len() <= 255 {
                response = response.location_query(query);
            }
        }
        response.with_received_header(parsed, self.received_at_ms)
    }
}

#[derive(Clone, Copy, Debug)]
struct InboxRow {
    complete: bool,
    call: Call,
    meta: Result<ReplyMeta, CallFailure>,
    body: Option<SlotId>,
    datagram: Option<SlotId>,
}

/// Bounded completed-reply table (not a seventh memory area).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientInbox<const CALLS: usize> {
    rows: [Option<InboxRow>; CALLS],
    held: Option<ReplyMeta>,
}

impl<const CALLS: usize> ClientInbox<CALLS> {
    pub(super) fn usage(&self) -> (usize, usize) {
        (
            self.rows.iter().flatten().count(),
            self.rows
                .iter()
                .flatten()
                .filter(|row| row.complete)
                .count(),
        )
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

    /// Store a completed reply. Same [`Call`] overwrites (Observe).
    ///
    /// Returns the previous body slot to release. `Err` if every row is a
    /// different Call — caller must not evict an untaken reply.
    fn insert(
        &mut self,
        call: Call,
        meta: Result<ReplyMeta, CallFailure>,
        body: Option<SlotId>,
    ) -> Result<Option<SlotId>, ()> {
        if let Some(slot) = self.rows.iter_mut().find(|row| {
            row.as_ref()
                .is_some_and(|row| row.call.token == call.token && row.call.peer == call.peer)
        }) {
            let prev = slot.take().and_then(|row| row.body);
            *slot = Some(InboxRow {
                complete: true,
                call,
                meta,
                body,
                datagram: None,
            });
            return Ok(prev);
        }
        if let Some(slot) = self.rows.iter_mut().find(|row| row.is_none()) {
            *slot = Some(InboxRow {
                complete: true,
                call,
                meta,
                body,
                datagram: None,
            });
            return Ok(None);
        }
        Err(())
    }

    pub(super) fn holds_datagram(&self, rx: SlotId) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|row| row.datagram == Some(rx))
    }

    fn take_datagram(&mut self, call: Call) -> Option<SlotId> {
        self.rows
            .iter_mut()
            .flatten()
            .find(|row| row.call == call)?
            .datagram
            .take()
    }

    fn completed(&self, call: Call) -> Option<&InboxRow> {
        self.rows
            .iter()
            .flatten()
            .find(|row| row.complete && row.call == call)
    }

    fn discard(&mut self, call: Call) {
        for row in &mut self.rows {
            if row.as_ref().is_some_and(|row| row.call == call) {
                *row = None;
            }
        }
    }

    fn contains(&self, call: Call) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|row| row.call == call && row.complete)
    }
    fn partial_meta(&self, call: Call, body: SlotId) -> Option<ReplyMeta> {
        self.rows
            .iter()
            .flatten()
            .find(|row| row.call == call && !row.complete && row.body == Some(body))?
            .meta
            .ok()
    }

    fn retain_partial(
        &mut self,
        call: Call,
        meta: ReplyMeta,
        body: SlotId,
    ) -> Result<Option<SlotId>, ()> {
        if self.partial_meta(call, body).is_some() {
            return Ok(None);
        }
        let previous = self.insert(call, Ok(meta), Some(body))?;
        if let Some(row) = self.rows.iter_mut().flatten().find(|row| row.call == call) {
            row.complete = false;
        }
        Ok(previous)
    }

    fn take(&mut self, call: Call) -> Option<(Result<Response<'_>, CallFailure>, Option<SlotId>)> {
        let i = self.rows.iter().position(|row| {
            row.as_ref().is_some_and(|row| {
                row.complete && row.call.token == call.token && row.call.peer == call.peer
            })
        })?;
        let row = self.rows[i].take()?;
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
        Some((response, row.body))
    }
}

/// Owned query values retained across Block2 requests, in original order.
#[derive(Clone, Copy, Debug)]
struct RetainedQueries {
    bytes: [u8; 256],
    lengths: [u16; MAX_PATH_SEGMENTS],
    count: usize,
}

impl RetainedQueries {
    fn new(queries: &[&str]) -> Result<Self, EncodeError> {
        let mut retained = Self {
            bytes: [0; 256],
            lengths: [0; MAX_PATH_SEGMENTS],
            count: queries.len(),
        };
        if queries.len() > MAX_PATH_SEGMENTS {
            return Err(EncodeError::OptionValueTooLong);
        }
        let mut offset = 0;
        for (i, query) in queries.iter().enumerate() {
            if query.len() > 255 || query.len() > retained.bytes.len() - offset {
                return Err(EncodeError::OptionValueTooLong);
            }
            retained.bytes[offset..offset + query.len()].copy_from_slice(query.as_bytes());
            retained.lengths[i] = query.len() as u16;
            offset += query.len();
        }
        Ok(retained)
    }

    fn values(&self) -> impl Iterator<Item = &str> {
        let mut offset = 0;
        self.lengths[..self.count].iter().map(move |len| {
            let end = offset + usize::from(*len);
            let value = core::str::from_utf8(&self.bytes[offset..end]).expect("copied UTF-8 query");
            offset = end;
            value
        })
    }
}

/// Exact canonical request identity for Observe cancellation and FETCH follow-ups.
/// Fixed header/Token; Observe and ETag are omitted. Includes the FETCH body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObserveRequest {
    bytes: [u8; OBSERVE_REQUEST_BYTES],
    len: usize,
}

impl ObserveRequest {
    fn same_target(&self, other: &Self) -> bool {
        use crate::message::OptionNumber as N;
        let left = crate::message::decode(&self.bytes[..self.len]).expect("encoded identity");
        let right = crate::message::decode(&other.bytes[..other.len]).expect("encoded identity");
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
struct LiveCall {
    call: Call,
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
pub(crate) struct ClientLives<const CALLS: usize> {
    rows: [Option<LiveCall>; CALLS],
}

impl<const CALLS: usize> ClientLives<CALLS> {
    pub(super) fn usage(&self) -> (usize, usize) {
        (
            self.rows.iter().flatten().count(),
            self.rows.iter().flatten().filter(|row| row.retired).count(),
        )
    }

    pub(super) fn next_timeout_after<Mem: Storage + Exchanges + ObserveSlots>(
        &self,
        engine: &Engine<Mem>,
        inbox: &ClientInbox<CALLS>,
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

    pub(super) fn response_credits(&self, inbox: &ClientInbox<CALLS>) -> usize {
        self.rows
            .iter()
            .flatten()
            .filter(|row| {
                !row.retired
                    && !inbox.rows.iter().flatten().any(|reply| {
                        reply.call == row.call && reply.complete && reply.meta.is_err()
                    })
            })
            .count()
            + inbox
                .rows
                .iter()
                .flatten()
                .filter(|row| {
                    row.meta.is_ok()
                        && !self
                            .rows
                            .iter()
                            .flatten()
                            .any(|live| !live.retired && live.call == row.call)
                })
                .count()
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
            if let Some(row) = slot
                && row.backoff_until_ms <= now_ms
            {
                row.backoff_until_ms = 0;
                if row.retired {
                    *slot = None;
                }
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

    fn record_backoff(&mut self, call: Call, parsed: &ParsedMessage<'_>, now_ms: u64) {
        if !matches!(
            parsed.code(),
            Code::TOO_MANY_REQUESTS | Code::SERVICE_UNAVAILABLE
        ) {
            return;
        }
        // Invalid or duplicate Max-Age cannot become a remote scheduling instruction.
        if parsed
            .get_options(crate::message::OptionNumber::MAX_AGE)
            .count()
            > 1
        {
            return;
        }
        let seconds = match parsed.max_age() {
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

    fn can_admit(&self, token: Token, peer: Endpoint) -> bool {
        self.rows.iter().any(|row| match row {
            None => true,
            Some(live) => !live.retired && live.call.token == token && live.call.peer == peer,
        })
    }

    fn insert(&mut self, live: LiveCall) {
        if let Some(existing) = self.rows.iter_mut().find(|row| {
            row.as_ref().is_some_and(|row| {
                !row.retired && row.call.token == live.call.token && row.call.peer == live.call.peer
            })
        }) {
            *existing = Some(live);
            return;
        }
        if let Some(slot) = self.rows.iter_mut().find(|row| row.is_none()) {
            *slot = Some(live);
        }
    }

    fn get(&self, call: Call) -> Option<LiveCall> {
        self.rows.iter().copied().find_map(|row| {
            row.filter(|row| {
                !row.retired && row.call.token == call.token && row.call.peer == call.peer
            })
        })
    }

    fn observe_fresh(&self, call: Call, parsed: &ParsedMessage<'_>, now_ms: u64) -> bool {
        let Some(sequence) = observation_start(parsed) else {
            return true;
        };
        let Some((previous, received)) = self.get(call).and_then(|live| live.observed) else {
            return true;
        };
        let delta = sequence.wrapping_sub(previous) & 0x00ff_ffff;
        (delta != 0 && delta < 0x0080_0000) || now_ms.saturating_sub(received) > 128_000
    }

    pub(crate) fn observe_format_admissible(
        &self,
        call: Call,
        parsed: &ParsedMessage<'_>,
        now_ms: u64,
        protected: bool,
    ) -> bool {
        (!protected && !self.observe_fresh(call, parsed, now_ms))
            || self.observe_format_matches(call, parsed)
    }

    fn observe_format_matches(&self, call: Call, parsed: &ParsedMessage<'_>) -> bool {
        if observation_start(parsed).is_none() {
            return true;
        }
        if parsed
            .content_format()
            .is_some_and(|format| format.is_err())
            || parsed
                .get_options(crate::message::OptionNumber::CONTENT_FORMAT)
                .count()
                > 1
        {
            return false;
        }
        // 2.03 may validate a cached representation without repeating its format.
        if parsed.code() == Code::VALID && parsed.content_format().is_none() {
            return true;
        }
        self.get(call)
            .and_then(|live| live.observed_format)
            .is_none_or(|initial| initial == parsed.content_format().and_then(Result::ok))
    }

    fn record_observation(&mut self, call: Call, parsed: &ParsedMessage<'_>, now_ms: u64) {
        if let Some(sequence) = observation_start(parsed)
            && let Some(live) = self
                .rows
                .iter_mut()
                .flatten()
                .find(|live| live.call == call)
        {
            live.observed = Some((sequence, now_ms));
            if live.observed_format.is_none()
                && (parsed.code() != Code::VALID || parsed.content_format().is_some())
            {
                live.observed_format = Some(parsed.content_format().and_then(Result::ok));
            }
        }
    }

    fn upload_key(&self, call: Call) -> BlockKey {
        BlockKey::new(call.token, call.peer).with_identity(
            self.get(call)
                .map(|live| live.request_tag)
                .unwrap_or(BodyTag::ABSENT),
        )
    }

    fn remove(&mut self, call: Call) {
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

    fn observe_call<E>(
        &self,
        peer: Endpoint,
        identity: ObserveRequest,
        target: Option<Call>,
        is_live: impl Fn(Call) -> bool,
    ) -> Result<Call, Error<E>> {
        let mut matches = self.rows.iter().flatten().filter(|live| {
            live.call.peer == peer
                && live.observe == OutgoingObserve::Register
                && live.due_ms != 0
                && is_live(live.call)
                && live.request_identity == Some(identity)
                && target.is_none_or(|call| live.call == call)
        });
        let call = matches
            .next()
            .ok_or(Error::ObserveCancellationMismatch)?
            .call;
        if matches.next().is_some() {
            return Err(Error::ObserveCancellationAmbiguous);
        }
        Ok(call)
    }
}

// RFC 7959 section 2.6: only block zero carries a new notification.
fn observation_start(parsed: &ParsedMessage<'_>) -> Option<u32> {
    if !parsed.code().is_success()
        || parsed
            .block2()
            .is_some_and(|block| block.is_ok_and(|block| block.num() != 0))
        || parsed
            .q_block2()
            .any(|block| block.is_ok_and(|block| block.num() != 0))
    {
        return None;
    }
    parsed.observe().and_then(Result::ok)
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
    S: super::AppStorage = super::AppStore<P, BLOCK_WISE>,
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
    observe_target: Option<Call>,
    deadline_ms: Option<u64>,
    _dest: core::marker::PhantomData<Dest>,
}

impl<
    P,
    T,
    const N: usize,
    const BLOCK_WISE: bool,
    S: super::AppStorage,
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
    S: super::AppStorage,
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
    /// [`Self::poll`] overwrites the hold. Datagram App has no hold. After
    /// [`Outgoing::observe`], the same `call` yields the initial
    /// representation and later notifications.
    /// Plaintext/DTLS notifications obey RFC 7641 serial-number ordering
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
    pub fn take_response(&mut self, call: Call) -> Option<Result<Response<'_>, CallFailure>> {
        if let Some(rx) = self.inbox.take_datagram(call) {
            let _ = self.engine.release_rx(rx);
        }
        let (response, body) = self.inbox.take(call)?;
        if !client_observe_live(&self.engine, call) {
            self.lives.remove(call);
            super::oscore::cancel(&mut self.oscore, call.peer, call.token());
        }
        if let Some(id) = body {
            if rx_body_payload(&self.engine, id)
                .is_some_and(|bytes| bytes.len() > super::RESPONSE_BODY)
            {
                release_rx_body(&mut self.engine, id);
                return Some(Err(CallFailure::ResponseBodyBounds));
            }
            if let Some(bytes) = rx_body_payload(&self.engine, id) {
                P::store(&mut self.assembled, bytes);
            }
            release_rx_body(&mut self.engine, id);
            if let Some(bytes) = P::view(&self.assembled) {
                return Some(response.map(|response| response.with_assembled(bytes)));
            }
        }
        Some(response)
    }
    /// Collect a complete representation into caller-owned storage.
    ///
    /// Enable [`super::AppBuilder::full_responses`] before bind to retain full
    /// ordinary datagrams and bodies larger than the legacy assembled hold.
    /// An insufficient buffer leaves the reply and all bytes available for a
    /// later attempt; no bytes are copied and no partial success is returned.
    /// Metadata borrows App; `payload()` (and block `body()`) borrows `output`.
    /// The nested outcome distinguishes local Call failure from remote 4.xx/5.xx.
    /// Block2 / Q-Block2 require block-wise storage. Without it, the Call
    /// fails with [`CallFailure::BlockTransfer`] containing
    /// [`BlockTransferError::NoBodyPools`], rather than returning one fragment.
    pub fn take_response_into<'a>(
        &'a mut self,
        call: Call,
        output: &'a mut [u8],
    ) -> Result<Option<Result<Response<'a>, CallFailure>>, ResponseBufferError> {
        let Some(row) = self.inbox.completed(call) else {
            return Ok(None);
        };
        let body = row.body;
        let datagram = row.datagram;
        if let Ok(meta) = row.meta {
            let bytes = if let Some(body) = body {
                self.engine
                    .rx_body_payload(body)
                    .ok_or(ResponseBufferError::Unavailable)?
            } else if let Some(rx) = datagram {
                self.engine
                    .decode_rx(rx)
                    .map_err(|_| ResponseBufferError::Unavailable)?
                    .payload()
            } else {
                if meta.payload_src_len > meta.payload_len {
                    return Err(ResponseBufferError::Unavailable);
                }
                &meta.payload[..usize::from(meta.payload_len)]
            };
            if bytes.len() > output.len() {
                return Err(ResponseBufferError::TooSmall {
                    required: bytes.len(),
                    available: output.len(),
                });
            }
        }
        // Copy before releasing the occupied storage; bounds were proven above.
        let len = if let Some(body) = body {
            let bytes = self
                .engine
                .rx_body_payload(body)
                .ok_or(ResponseBufferError::Unavailable)?;
            output[..bytes.len()].copy_from_slice(bytes);
            bytes.len()
        } else if let Some(rx) = datagram {
            let parsed = self
                .engine
                .decode_rx(rx)
                .map_err(|_| ResponseBufferError::Unavailable)?;
            let bytes = parsed.payload();
            output[..bytes.len()].copy_from_slice(bytes);
            bytes.len()
        } else if let Ok(meta) = row.meta {
            let len = usize::from(meta.payload_len);
            output[..len].copy_from_slice(&meta.payload[..len]);
            len
        } else {
            0
        };
        if let Some(rx) = self.inbox.take_datagram(call) {
            let _ = self.engine.release_rx(rx);
        }
        let (response, _) = self.inbox.take(call).expect("completed reply");
        if let Some(body) = body {
            let _ = self.engine.release_rx_body(body);
        }
        if !client_observe_live(&self.engine, call) {
            self.lives.remove(call);
            super::oscore::cancel(&mut self.oscore, call.peer, call.token());
        }
        Ok(Some(response.map(|response| {
            response.with_complete_payload(&output[..len], body.is_some())
        })))
    }

    /// Cancel an active call and reclaim its exchange, body, Observe and OSCORE
    /// state. The next `take_response` yields `Err(CallFailure::Cancelled)`.
    /// No wire message is sent; use `Outgoing::deregister` for Observe signaling.
    /// Returns false for unknown/already terminal ordinary calls.
    pub fn cancel(&mut self, call: Call) -> bool {
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
    S: super::AppStorage,
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
    pub const fn reregister_call(mut self, call: Call) -> Self {
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
    pub const fn deregister_call(mut self, call: Call) -> Self {
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
    S: super::AppStorage,
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
    pub fn send(self, now_ms: u64) -> Result<Call, Error<T::Error>>
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
    ) -> impl core::future::Future<Output = Result<Call, AsyncCallError<T::Error, C>>> + 'a
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
    ) -> impl core::future::Future<Output = Result<Call, AsyncCallError<T::Error, C>>> + 'a
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
    ) -> impl core::future::Future<Output = Result<Call, AsyncCallError<T::Error, C>>> + 'a
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
                return Ok(prepared.commit());
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
            Ok(prepared.commit())
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
            token: Token::EMPTY,
            path: path.segments(),
            payload: self.payload,
            content_format: self.content_format,
            echo: self.echo,
            no_response: self.no_response,
            accept: self.accept,
            request_tag: self.request_tag,
            etag: self.etag,
            if_match: self.if_match,
            if_none_match: self.if_none_match,
            queries: &queries[..query_n],
            q_block1: self.q_block1,
            q_block2: self.q_block2,
            block2: self.block2,
            observe,
        };
        let request_identity = if observe == OutgoingObserve::Off && self.code != Code::FETCH {
            None
        } else {
            Some(
                request_identity_identity(&spec).map_err(|error| match error {
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
                    && (!self.app.inbox.contains(live.call)
                        || client_observe_live(&self.app.engine, live.call))
                    && live
                        .request_identity
                        .as_ref()
                        .is_some_and(|original| original.same_target(identity))
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
            && self.app.lives.response_credits(&self.app.inbox)
                >= self
                    .app
                    .engine
                    .capacities()
                    .rx_datagram_slots
                    .saturating_sub(super::admission::RX_HEADROOM)
        {
            Metrics::inc(&mut self.app.engine.metrics_mut().admission_rx_refused);
            return Err(Error::Saturated);
        }
        if !replacing && self.app.lives.rows.iter().flatten().count() >= CALLS {
            return Err(Error::Saturated);
        }
        let token = if replacing {
            self.app
                .lives
                .observe_call::<T::Error>(
                    dest,
                    request_identity.expect("Observe identity"),
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
                })?
                .token()
        } else {
            self.app.next_token()?
        };
        if !self.app.lives.can_admit(token, dest) {
            return Err(Error::Saturated);
        }
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
        let prior = refreshing
            .then(|| self.app.lives.get(Call::new(token, dest)))
            .flatten();
        let deadline_ms = match (self.deadline_ms, prior.and_then(|live| live.deadline_ms)) {
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
            observed: prior.and_then(|live| live.observed),
            observed_format: prior.and_then(|live| live.observed_format),
            request_identity,
            deadline_ms,
            q_pace_ms: 0,
            backoff_until_ms: 0,
            retired: false,
            due_ms: now_ms.saturating_add(u64::from(if self.ty == Type::NonConfirmable {
                Transmission::NON_LIFETIME_MS
            } else {
                Transmission::EXCHANGE_LIFETIME_MS
            })),
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
            live: Some(live),
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
    S: super::AppStorage,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
{
    fn next_token<E>(&mut self) -> Result<Token, Error<E>> {
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

struct ClientSend<'a> {
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &'a [&'a str],
    payload: &'a [u8],
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    accept: Option<ContentFormat>,
    request_tag: BodyTag,
    etag: Option<&'a [u8]>,
    if_match: Option<&'a [u8]>,
    if_none_match: bool,
    queries: &'a [&'a str],
    q_block1: bool,
    q_block2: bool,
    block2: Option<BlockValue>,
    observe: OutgoingObserve,
}

// One option builder feeds both wire encoding and exact cancellation identity.
fn with_client_options<R>(
    spec: &ClientSend<'_>,
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
        for query in spec.queries {
            push_opt(&mut opts, Opt::uri_query(query))?;
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
        Ok(())
    })();
    filled?;
    Ok(use_options(opts.as_slice()))
}

fn request_identity_identity(spec: &ClientSend<'_>) -> Result<ObserveRequest, EncodeError> {
    with_client_options(spec, true, |options| {
        let mut identity = ObserveRequest {
            bytes: [0; OBSERVE_REQUEST_BYTES],
            len: 0,
        };
        let message = Message::new(Type::Confirmable, spec.code, MessageId::new(0))
            .with_options(options)
            .with_payload(spec.payload);
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
                    rx,
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
                rx,
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
    inbox: &'a mut ClientInbox<C>,
    lives: &'a mut ClientLives<C>,
    oscore: &'a mut super::oscore::Field<R>,
    staged: Option<StagedClient>,
    pending: Option<crate::storage::PendingCon>,
    live: Option<LiveCall>,
    call: Call,
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
        if self.live.as_ref().expect("prepared call").ty == Type::Confirmable {
            self.pending = Some(
                self.engine
                    .reserve_pending_con(staged.tx, self.call.peer, staged.mid)
                    .ok_or(Error::Saturated)?,
            );
        }
        Ok(())
    }

    fn commit(&mut self) -> Call {
        if let Some(mut live) = self.live.take() {
            if self.replacing {
                if let Some(rx) = self.inbox.take_datagram(self.call) {
                    let _ = self.engine.release_rx(rx);
                }
                self.inbox.discard(self.call);
            }
            live.due_ms =
                self.now_ms
                    .saturating_add(u64::from(if live.ty == Type::NonConfirmable {
                        Transmission::NON_LIFETIME_MS
                    } else {
                        Transmission::EXCHANGE_LIFETIME_MS
                    }));
            self.lives.insert(live);
            arm_q_pacing(self.engine, self.lives, self.ids, self.now_ms, self.call);
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
        let live = self.live.as_ref().expect("prepared Q upload");
        let mut queries = [""; MAX_PATH_SEGMENTS];
        let mut count = 0;
        for query in live.queries.values() {
            queries[count] = query;
            count += 1;
        }
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
            &queries[..count],
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

    fn send(mut self) -> Result<Call, Error<T::Error>>
    where
        T: DatagramIo,
    {
        let Some(staged) = self.staged.take() else {
            return Ok(self.commit());
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
        Ok(self.commit())
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
        let own_deadline = self.live.as_ref().and_then(|live| live.deadline_ms);
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
            .and_then(|live| live.deadline_ms)
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    staged: StagedClient,
    spec: &ClientSend<'_>,
) -> Result<Call, Error<T::Error>>
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    spec: &ClientSend<'_>,
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    queries: &[&str],
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
) -> Result<Call, Error<T::Error>>
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    _now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    queries: &[&str],
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
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    response_state: super::oscore::ResponseState,
    full_responses: bool,
    now_ms: u64,
    peer: Endpoint,
    parsed: &ParsedMessage<'_>,
    rx: SlotId,
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
    let via_observe = matching_observe(engine, parsed, peer);
    if via_exchange.is_none() && !via_observe {
        let outcome = if parsed.ty() == Type::Confirmable {
            super::send_empty_rst(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        let _ = engine.release_rx(rx);
        return outcome;
    }
    // RFC 7252 sections 5.4.1 and 4.2: reject the response, not an
    // otherwise matching ACK. No inbox/body/Observe state may accept it.
    if parsed.unknown_critical().is_some() {
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
    let fresh = super::oscore::is_active(oscore) || lives.observe_fresh(call, parsed, now_ms);
    if !lives.observe_format_admissible(call, parsed, now_ms, super::oscore::is_active(oscore)) {
        let outcome = if parsed.ty() == Type::Confirmable {
            super::send_empty_rst(engine, io, peer, parsed.message_id())
        } else {
            Ok(())
        };
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            call,
            CallFailure::ObserveFormatMismatch,
            true,
        );
        let _ = engine.release_rx(rx);
        return outcome;
    }
    if parsed.ty() == Type::Confirmable
        && let Err(e) = super::send_empty_ack(engine, io, peer, parsed.message_id())
    {
        let _ = engine.release_rx(rx);
        return Err(e);
    }
    if let Err(error) = super::oscore::commit_response(oscore, peer, response_state) {
        let _ = engine.release_rx(rx);
        return Err(error);
    }
    // A matching piggybacked ACK ends retransmission even if its Observe
    // value is stale. A delayed NON/CON notification cannot acknowledge a
    // concurrently refreshed request merely by sharing its Token.
    if parsed.ty() == Type::Acknowledgement
        && let Some(entry) = via_exchange
        && let Some(tx) = engine.take_pending_con(entry.message_id(), peer)
    {
        let _ = engine.release_tx(tx);
    }
    // ACK stale CON notifications too, but do not refresh their lifetime,
    // replace the inbox, or alter assembly. OSCORE already authenticated and
    // ordered notifications by Partial IV (RFC 8613 section 4.1.3.5.2).
    if !fresh {
        let _ = engine.release_rx(rx);
        return Ok(());
    }
    // Piggybacked ACK shares the request MID. A separate CON/NON response
    // uses a new MID; stop RTO using the exchange's request MID.
    if let Some(entry) = via_exchange
        && let Some(tx) = engine.take_pending_con(entry.message_id(), peer)
    {
        let _ = engine.release_tx(tx);
    }
    if let Some(tx) = engine.take_pending_con(parsed.message_id(), peer) {
        let _ = engine.release_tx(tx);
    }

    if let Some(body) = upload_body(engine, lives.upload_key(Call::new(parsed.token(), peer)))
        && let Some(transfer) = engine.tx_body_transfer(body)
        && matches!(
            transfer.role(),
            BlockRole::OutgoingBlock1 | BlockRole::OutgoingQBlock1
        )
    {
        if transfer.role() == BlockRole::OutgoingQBlock1
            && parsed.code() == Code::REQUEST_ENTITY_INCOMPLETE
            && parsed.content_format() == Some(Ok(ContentFormat::MISSING_BLOCKS))
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
                parsed.block1().and_then(Result::ok),
                parsed.q_block1().and_then(Result::ok),
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

    let metadata = match ReplyMeta::from_parsed(parsed, peer, now_ms) {
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

    lives.record_backoff(call, parsed, now_ms);
    let overloaded = matches!(
        parsed.code(),
        Code::TOO_MANY_REQUESTS | Code::SERVICE_UNAVAILABLE
    );

    // Classify from the already-decoded response. A plain 2.05 must not
    // stack Block IO scratch via MissingBlock probes.
    if !overloaded && parsed.block2().is_some() {
        match engine.apply_block2_rx(rx) {
            Ok(progress) if progress.complete() => {
                lives.record_observation(Call::new(parsed.token(), peer), parsed, now_ms);
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
                    via_exchange.is_some(),
                    full_responses,
                );
                return Ok(());
            }
            Ok(progress) => {
                lives.record_observation(Call::new(parsed.token(), peer), parsed, now_ms);
                if let Some(previous) = inbox.take_datagram(call) {
                    let _ = engine.release_rx(previous);
                }
                match inbox.retain_partial(Call::new(parsed.token(), peer), metadata, progress.id())
                {
                    Ok(Some(previous)) if previous != progress.id() => {
                        let _ = engine.release_rx_body(previous);
                    }
                    Ok(_) => {}
                    Err(()) => {
                        let _ = engine.release_rx_body(progress.id());
                        let _ = engine.release_rx(rx);
                        return Err(Error::Saturated);
                    }
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
    } else if !overloaded && parsed.q_block2().next().is_some() {
        match engine.apply_q_block2_rx(rx) {
            Ok(progress) if progress.complete() => {
                lives.record_observation(Call::new(parsed.token(), peer), parsed, now_ms);
                let _ = engine.note_q_receive(progress.id(), now_ms);
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
                    via_exchange.is_some(),
                    full_responses,
                );
                return Ok(());
            }
            Ok(progress) => {
                lives.record_observation(Call::new(parsed.token(), peer), parsed, now_ms);
                if let Some(previous) = inbox.take_datagram(call) {
                    let _ = engine.release_rx(previous);
                }
                match inbox.retain_partial(Call::new(parsed.token(), peer), metadata, progress.id())
                {
                    Ok(Some(previous)) if previous != progress.id() => {
                        let _ = engine.release_rx_body(previous);
                    }
                    Ok(_) => {}
                    Err(()) => {
                        let _ = engine.release_rx_body(progress.id());
                        let _ = engine.release_rx(rx);
                        return Err(Error::Saturated);
                    }
                }
                let _ = engine.note_q_receive(progress.id(), now_ms);
                let outcome = if needs_q_continue(engine, progress.id()) {
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
    let replacing_rx = inbox
        .rows
        .iter()
        .flatten()
        .any(|row| row.call == call && row.datagram.is_some());
    if full_responses
        && !replacing_rx
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

    lives.record_observation(Call::new(parsed.token(), peer), parsed, now_ms);
    accept_client_observe(engine, lives, parsed, peer, now_ms, via_exchange.is_some());
    take_exchange(engine, parsed, peer);
    store_reply(
        engine,
        inbox,
        Call::new(parsed.token(), peer),
        metadata,
        None,
    );
    if full_responses {
        if let Some(row) = inbox.rows.iter_mut().flatten().find(|row| row.call == call) {
            row.datagram = Some(rx);
        } else {
            let _ = engine.release_rx(rx);
        }
    } else {
        let _ = engine.release_rx(rx);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_assembled<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    rx: SlotId,
    body: SlotId,
    metadata: ReplyMeta,
    via_exchange: bool,
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
    let metadata = inbox.partial_meta(call, body).unwrap_or(metadata);
    let original = crate::message::decode(&metadata.header[..usize::from(metadata.header_len)])
        .expect("encoded response header");
    accept_client_observe(
        engine,
        lives,
        &original,
        peer,
        metadata.received_at_ms,
        via_exchange,
    );
    take_exchange(engine, parsed, peer);
    store_reply(
        engine,
        inbox,
        Call::new(parsed.token(), peer),
        metadata,
        Some(body),
    );
    let _ = engine.release_rx(rx);
}

fn store_reply<Mem: Storage + BodySlots + DatagramSlots, const CALLS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS>,
    call: Call,
    meta: ReplyMeta,
    body: Option<SlotId>,
) {
    if let Some(rx) = inbox.take_datagram(call) {
        let _ = engine.release_rx(rx);
    }
    match inbox.insert(call, Ok(meta), body) {
        Ok(prev) => {
            if let Some(id) = prev
                && Some(id) != body
            {
                let _ = engine.release_rx_body(id);
            }
        }
        Err(()) => {
            if let Some(id) = body {
                let _ = engine.release_rx_body(id);
            }
        }
    }
}

/// Check unencrypted routing fields before touching protected response state.
pub(super) fn has_response_target<Mem: Storage + Exchanges + ObserveSlots + PendingCons>(
    engine: &Engine<Mem>,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
) -> bool {
    matching_exchange(engine, parsed, peer).is_some() || matching_observe(engine, parsed, peer)
}

fn matching_observe<Mem: Storage + ObserveSlots>(
    engine: &Engine<Mem>,
    parsed: &ParsedMessage<'_>,
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
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
) -> Option<crate::storage::ExchangeEntry> {
    let id = engine.lookup_exchange(ExchangeKey::new(parsed.token(), peer))?;
    let entry = engine.exchange_entry(id)?;
    (entry.matches_response(parsed) && !exchange_is_unsent(engine, entry)).then_some(entry)
}

// A reserved initial CON owns admission and bytes, but grants no response
// authority until the transport accepts it. Match the complete identity:
// an old NON exchange may refer to a TX slot reused by another request.
fn exchange_is_unsent<Mem: Storage + PendingCons>(
    engine: &Engine<Mem>,
    entry: ExchangeEntry,
) -> bool {
    engine.pending_con(entry.tx_slot()).is_some_and(|pending| {
        pending.message_id() == entry.message_id()
            && pending.endpoint() == entry.endpoint()
            && !pending.rto().is_started()
    })
}

fn take_exchange<Mem: Storage + Exchanges>(
    engine: &mut Engine<Mem>,
    parsed: &ParsedMessage<'_>,
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
        && transfer.q_holes().is_none()
        && transfer.window_mask() == 0
        && transfer.window_base() > 0
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_block2_continue_async, bound(crate::app::async_server::ProtocolIo), calls(send_followup => send_followup_async))]
fn send_block2_continue<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    parsed: &ParsedMessage<'_>,
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
        lives,
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
    lives: &ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    parsed: &ParsedMessage<'_>,
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
        lives,
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
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    recover: crate::storage::QBlockRecover,
) -> Result<bool, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: DatagramIo,
{
    let call = Call::new(recover.key().token(), recover.key().endpoint());
    if lives.get(call).is_none() {
        return Ok(false);
    }
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
        lives,
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
fn send_followup<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    token: Token,
    peer: Endpoint,
    block2: Option<BlockValue>,
    q_block2: &[BlockValue],
    recovery_ty: Option<Type>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let live = lives.get(Call::new(token, peer));
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
    let fetch = live
        .as_ref()
        .filter(|live| live.code == Code::FETCH)
        .and_then(|live| live.request_identity.as_ref())
        .map(|identity| {
            crate::message::decode(&identity.bytes[..identity.len])
                .expect("encoded request identity")
        });
    let cf = live
        .and_then(|live| live.content_format)
        .map(ContentFormat::encode);
    let filled = (|| -> Result<(), EncodeError> {
        if let Some(live) = live.as_ref() {
            if let Some(tag) = live.if_match.as_slice() {
                push_opt(&mut opts, Opt::if_match(tag))?;
            }
            if live.if_none_match {
                push_opt(&mut opts, Opt::if_none_match())?;
            }
            // RFC 9177 section 4.4: Q missing requests avoid ETag validation
            // replies (2.03) without payload. Classic Block2 retains it.
            if q_block2.is_empty()
                && let Some(tag) = live.etag.as_slice()
            {
                push_opt(&mut opts, Opt::etag(tag))?;
            }
        }
        if let Some(ref encoded) = cf {
            push_opt(&mut opts, Opt::content_format(encoded))?;
        }
        if let Some(live) = live.as_ref() {
            for segment in live.path.segments() {
                push_opt(&mut opts, Opt::uri_path(segment))?;
            }
        }
        if let Some(live) = live.as_ref() {
            for query in live.queries.values() {
                push_opt(&mut opts, Opt::uri_query(query))?;
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
        if let Some(echo) = live.as_ref().and_then(|live| live.echo.as_ref()) {
            push_opt(&mut opts, Opt::echo(echo.as_slice()))?;
        }
        if let Some(tag) = live.as_ref().and_then(|live| live.request_tag.as_slice()) {
            push_opt(&mut opts, Opt::request_tag(tag))?;
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
            fetch
                .as_ref()
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
            let mut queries = [""; MAX_PATH_SEGMENTS];
            let mut count = 0;
            for query in live.queries.values() {
                queries[count] = query;
                count += 1;
            }
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
                &queries[..count],
                live.accept,
                fetch.as_ref().expect("retained FETCH identity").payload(),
                live.request_tag,
                live.content_format,
                live.echo,
                live.no_response,
                live.if_match.as_slice(),
                if q_block2.is_empty() {
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
    lives: &mut ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    parsed: &ParsedMessage<'_>,
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
    let Some(live) = lives.get(Call::new(parsed.token(), peer)) else {
        return Ok(());
    };
    let mut queries = [""; MAX_PATH_SEGMENTS];
    let mut query_n = 0;
    for query in live.queries.values() {
        queries[query_n] = query;
        query_n += 1;
    }
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
            &queries[..query_n],
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
        arm_q_pacing(engine, lives, ids, completed, live.call);
    }
    Ok(())
}

// Only success-suppressed NON uploads use timer progression. The last packet's
// bounded unbiased 0..=1000ms jitter sample also schedules its next payload set.
fn arm_q_pacing<Mem: Storage + BodySlots, const CALLS: usize>(
    engine: &Engine<Mem>,
    lives: &mut ClientLives<CALLS>,
    ids: &AppIds,
    now_ms: u64,
    call: Call,
) {
    let body = upload_body(engine, lives.upload_key(call));
    let ready = body
        .and_then(|body| engine.tx_body_transfer(body))
        .is_some_and(|t| {
            t.role() == BlockRole::OutgoingQBlock1
                && !t.is_complete()
                && t.window_mask() == (1u16 << crate::storage::BlockTransfer::MAX_PAYLOADS) - 1
        });
    if let Some(live) = lives
        .rows
        .iter_mut()
        .flatten()
        .find(|live| live.call == call)
    {
        live.q_pace_ms = if ready
            && live.ty == Type::NonConfirmable
            && live
                .no_response
                .is_some_and(|mask| mask.suppresses(Code::CONTINUE))
        {
            now_ms
                .checked_add(u64::from(
                    crate::message::QBlockTransmission::NON_TIMEOUT_MS + ids.jitter(),
                ))
                .unwrap_or(0)
        } else {
            0
        };
    }
}

/// At most one due payload set per poll, oldest deadline first. Missed time does
/// not accumulate send credit. Ingress runs first so valid Continue/recovery can
/// disarm or rearm this opportunity before selection.
#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(pace_qblock1_io_async, bound(crate::app::async_server::ProtocolIo), calls(continue_block1_tx => continue_block1_tx_async))]
pub(crate) fn pace_qblock1<Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: DatagramIo,
{
    let Some(live) = lives
        .rows
        .iter()
        .flatten()
        .filter(|live| live.q_pace_ms != 0 && live.q_pace_ms <= now_ms)
        .min_by_key(|live| live.q_pace_ms)
        .copied()
    else {
        return Ok(());
    };
    // Clear before attempting: every success rearms from now, every failure
    // retires the Call without changing the uncertainty of remote effects.
    if let Some(row) = lives
        .rows
        .iter_mut()
        .flatten()
        .find(|row| row.call == live.call)
    {
        row.q_pace_ms = 0;
    }
    let Some(body) = upload_body(engine, lives.upload_key(live.call)) else {
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
            live.call.token(),
            None,
            None,
            live.call.peer,
            body,
        ),
        Err(error) => Err(error),
    };
    if result.is_err() {
        abandon_send(engine, oscore, live.call.token(), live.call.peer);
        fail_call(
            engine,
            inbox,
            lives,
            oscore,
            live.call,
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
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
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
    let Some(body) = upload_body(engine, lives.upload_key(call)) else {
        return Ok(());
    };
    let Some(transfer) = engine.tx_body_transfer(body) else {
        return Ok(());
    };
    if transfer.role() != BlockRole::OutgoingQBlock1 || transfer.is_complete() {
        return Ok(());
    }
    let acknowledged =
        if transfer.window_mask() == (1u16 << crate::storage::BlockTransfer::MAX_PAYLOADS) - 1 {
            // Every earlier payload was individually ACKed before the next
            // was issued, so this ACK confirms the entire current set.
            engine
                .ack_q_block1(body, transfer.num())
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
    lives: &mut ClientLives<CALLS>,
    ids: &mut AppIds,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    token: Token,
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
    let live = lives.get(Call::new(token, peer));
    let ty = live.map(|live| live.ty).unwrap_or(Type::Confirmable);
    let code = live.map(|live| live.code).unwrap_or(Code::PUT);
    let path = live.map(|live| live.path);
    let content_format = live.and_then(|live| live.content_format);
    let echo = live.and_then(|live| live.echo);
    let no_response = live.and_then(|live| live.no_response);
    let if_match = live.as_ref().and_then(|live| live.if_match.as_slice());
    let etag = live.as_ref().and_then(|live| live.etag.as_slice());
    let if_none_match = live.is_some_and(|live| live.if_none_match);
    let segments: &[&str] = path.as_ref().map_or(&[], Path::segments);
    let mut queries = [""; MAX_PATH_SEGMENTS];
    let mut query_n = 0;
    if let Some(live) = live.as_ref() {
        for query in live.queries.values() {
            queries[query_n] = query;
            query_n += 1;
        }
    }
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
                &queries[..query_n],
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
                &queries[..query_n],
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
            arm_q_pacing(engine, lives, ids, completed, Call::new(token, peer));
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    queries: &[&str],
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    dest: Endpoint,
    first_ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    queries: &[&str],
    accept: Option<ContentFormat>,
    content_format: Option<ContentFormat>,
    echo: Option<crate::message::Echo>,
    no_response: Option<crate::message::NoResponse>,
    if_match: Option<&[u8]>,
    etag: Option<&[u8]>,
    if_none_match: bool,
    body: SlotId,
    mut tx: Option<SlotId>,
    take_first: Option<(Token, Endpoint)>,
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    queries: &[&str],
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    queries: &[&str],
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
    oscore: &mut super::oscore::Field<REQUESTS>,
    tx: SlotId,
    ty: Type,
    code: Code,
    mid: MessageId,
    token: Token,
    path: &[&str],
    queries: &[&str],
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
            matches!(
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
        for query in queries {
            push_opt(&mut opts, Opt::uri_query(query))?;
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
    match engine.record_request(tx, dest) {
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
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid, jitter));
    match super::finish_send(engine, io, tx, dest, pending) {
        Ok(()) => Ok(()),
        Err(e) => {
            drop_exchange_for_tx(engine, tx);
            Err(e)
        }
    }
}

// A send can fail after earlier Q-Block datagrams went out. No Call is
// returned, so retire all local state owned by this request. Never rewind
// OSCORE sequence numbers: a transport error does not prove non-delivery.
fn abandon_send<Mem, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    token: Token,
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

fn drop_exchange_for_tx<Mem: Storage + Exchanges>(engine: &mut Engine<Mem>, tx: SlotId) {
    let n = engine.capacities().tx_datagram_slots;
    for i in 0..n {
        let id = SlotId::from_index(i);
        let Some(entry) = engine.exchange_entry(id) else {
            continue;
        };
        if entry.tx_slot() != tx {
            continue;
        }
        let _ = engine.take_exchange(entry.key());
        return;
    }
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

fn upload_body<Mem: Storage + BodySlots>(engine: &Engine<Mem>, key: BlockKey) -> Option<SlotId> {
    engine
        .lookup_tx_body_role(key, BlockRole::OutgoingBlock1)
        .or_else(|| engine.lookup_tx_body_role(key, BlockRole::OutgoingQBlock1))
}

fn rx_body_payload<Mem: Storage + BodySlots>(engine: &Engine<Mem>, id: SlotId) -> Option<&[u8]> {
    engine.rx_body_payload(id)
}

fn release_rx_body<Mem: Storage + BodySlots>(engine: &mut Engine<Mem>, id: SlotId) {
    let _ = engine.release_rx_body(id);
}

fn client_observe_live<Mem: Storage + ObserveSlots>(engine: &Engine<Mem>, call: Call) -> bool {
    let key = ObserveKey::new_client(call.token(), call.peer);
    engine.lookup_observe(key).is_some()
}

fn accept_client_observe<Mem, const CALLS: usize>(
    engine: &mut Engine<Mem>,
    lives: &ClientLives<CALLS>,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    now_ms: u64,
    via_exchange: bool,
) where
    Mem: Storage + ObserveSlots,
{
    let key = ObserveKey::new_client(parsed.token(), peer);
    let live = lives.get(Call::new(parsed.token(), peer));
    if live.is_some_and(|live| live.observe == OutgoingObserve::Deregister) {
        let _ = engine.take_observe(key);
        return;
    }
    let subscribed = live.is_some_and(|live| live.observe == OutgoingObserve::Register)
        || engine.lookup_observe(key).is_some();
    if subscribed && parsed.observe().and_then(Result::ok).is_some() && parsed.code().is_success() {
        let _ = engine.insert_observe(ObserveInterest::new_client(parsed.token(), peer));
        let max_age = parsed
            .max_age()
            .and_then(Result::ok)
            .unwrap_or(super::DEFAULT_MAX_AGE_SECS);
        let _ = engine.refresh_observe_max_age(key, now_ms, max_age, None);
    } else if via_exchange || subscribed {
        let _ = engine.take_observe(key);
    }
}

/// Retransmission give-up completes the call rather than silently losing it.
pub(crate) fn give_up_client<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    tx: SlotId,
) where
    Mem: Storage + DatagramSlots + Exchanges + BodySlots + ObserveSlots + PendingCons,
{
    fail_pending_client(engine, inbox, lives, oscore, tx, CallFailure::TimedOut);
}

/// Retire a pending request with its actual completion failure classification.
pub(crate) fn fail_pending_client<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    tx: SlotId,
    failure: CallFailure,
) where
    Mem: Storage + DatagramSlots + Exchanges + BodySlots + ObserveSlots + PendingCons,
{
    if let Some(entry) = exchange_for_tx(engine, tx) {
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

fn exchange_for_tx<Mem: Storage + Exchanges>(
    engine: &Engine<Mem>,
    tx: SlotId,
) -> Option<ExchangeEntry> {
    let n = engine.capacities().tx_datagram_slots;
    (0..n).find_map(|i| {
        let entry = engine.exchange_entry(SlotId::from_index(i))?;
        (entry.tx_slot() == tx).then_some(entry)
    })
}

fn exchange_for_mid<Mem: Storage + Exchanges>(
    engine: &Engine<Mem>,
    message_id: MessageId,
    peer: Endpoint,
) -> Option<ExchangeEntry> {
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
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    key: ExchangeKey,
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
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    call: Call,
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
    if let Some(rx) = inbox.take_datagram(call) {
        let _ = engine.release_rx(rx);
    }
    if let Ok(Some(body)) = inbox.insert(call, Err(failure), None) {
        let _ = engine.release_rx_body(body);
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
    if let Some(body) = upload_body(engine, lives.upload_key(call))
        && engine.tx_body_transfer(body).is_some_and(|transfer| {
            matches!(
                transfer.role(),
                BlockRole::OutgoingBlock1 | BlockRole::OutgoingQBlock1
            )
        })
    {
        let _ = engine.release_tx_body(body);
    }
    super::oscore::cancel(oscore, call.peer, call.token());
    // Untaken failures occupy the same bounded completion budget as replies.
    for live in lives
        .rows
        .iter_mut()
        .flatten()
        .filter(|live| live.call == call)
    {
        live.due_ms = 0;
        live.deadline_ms = None;
    }
}

pub(crate) fn complete_client_rst<Mem, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    message_id: MessageId,
    peer: Endpoint,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    let entry = engine
        .lookup_pending_con(message_id, peer)
        .and_then(|tx| exchange_for_tx(engine, tx))
        .or_else(|| {
            exchange_for_mid(engine, message_id, peer)
                .filter(|entry| !exchange_is_unsent(engine, *entry))
        });
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
    inbox: &mut ClientInbox<CALLS>,
    lives: &mut ClientLives<CALLS>,
    oscore: &mut super::oscore::Field<REQUESTS>,
    now_ms: u64,
) where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    lives.expire_backoffs(now_ms);
    let mut due = [None; CALLS];
    for (i, live) in lives.rows.iter().enumerate() {
        let Some(live) = *live else {
            continue;
        };
        if live.retired || (inbox.contains(live.call) && !client_observe_live(engine, live.call)) {
            continue;
        }
        let failure = if live.deadline_ms.is_some_and(|deadline| now_ms >= deadline) {
            Some(CallFailure::DeadlineExceeded)
        } else if live.due_ms != 0
            && now_ms >= live.due_ms
            && engine
                .lookup_exchange(ExchangeKey::new(live.call.token(), live.call.peer))
                .is_some()
        {
            Some(CallFailure::TimedOut)
        } else {
            None
        };
        due[i] = failure.map(|failure| (live.call, failure));
    }
    for (call, failure) in due.into_iter().flatten() {
        fail_call(engine, inbox, lives, oscore, call, failure, true);
    }
}

#[cfg(feature = "oscore")]
pub(super) fn revoke_association<S: super::AppStorage, const C: usize, const R: usize>(
    engine: &mut Engine<S>,
    inbox: &mut ClientInbox<C>,
    lives: &mut ClientLives<C>,
    oscore: &mut super::oscore::Field<R>,
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
