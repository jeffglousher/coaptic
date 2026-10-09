//! Sequential EDHOC bootstrap over caller-owned CoAP datagrams.
//!
//! This fixed profile uses one installed pin, one endpoint, Confirmable POSTs
//! and the default EDHOC resource. It supports piggybacked replies and separate
//! Confirmable or Non-confirmable replies. Block-wise transfer, discovery,
//! proxies, EAD and combined EDHOC/OSCORE requests are outside this profile.
//! No application plaintext policy is changed by bootstrap.
//! The initiator accepts at most one initial piggybacked Echo challenge, saving
//! its exact response and retrying the same message 1 under a separately reserved
//! Message ID. This neither renews entropy nor extends the original wait limit.
//!
//! [`CoapProvisioner::poll`] requires exclusive access to the bootstrap
//! transport. After handing a session to a new secure App, demultiplex the
//! EDHOC resource into [`CoapProvisioner::ingest`] and call
//! [`CoapProvisioner::flush`] for cached replies. Blindly polling both owners
//! on the same socket can consume the other's traffic. Keep the completion
//! cache resident until [`CoapProvisioner::next_deadline`] expires; message 4
//! can be lost even after the responder has handed its session to the App.

#[path = "recovery.rs"]
mod recovery;

pub use recovery::{CoapRecovery, RecoveryError};

use core::{convert::Infallible, fmt};

use crate::message::{
    Code, Message as CoapMessage, MessageId, Opt, OptionNumber, ParsedMessage, Token, Type, decode,
    decode_uint16,
};
use crate::storage::{DatagramIo, Endpoint};

use super::{
    ConnectionId, Error, Identity, Initiator, InitiatorConfirm, Message, PinnedPeer, Principal,
    Responder, Session,
};

const DATAGRAM_CAPACITY: usize = 384;
const EXCHANGE_LIFETIME_MS: u64 = 247_000;
const MAX_RETRANSMIT: u8 = 4;
const REQUEST_FORMAT: u16 = 65;
const RESPONSE_FORMAT: u16 = 64;

/// Bootstrap progress after one bounded controller call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// No datagram or deadline required work.
    Idle,
    /// A valid transition, retransmission or cached reply made progress.
    Progress,
    /// Authentication completed and all immediately required sends succeeded.
    Complete,
    /// An unrelated or unsupported datagram was ignored without consuming state.
    Ignored,
    /// Completion caches expired; this controller must no longer serve traffic.
    Expired,
}

/// Bootstrap failure, retaining cached bytes across transport failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollError<E = Infallible> {
    /// Caller-owned transport failed; the pending send remains cached.
    Io(E),
    /// A wire-valid EDHOC operation failed and consumed its handshake state.
    Provisioning(Error),
    /// The bounded handshake or Confirmable exchange exhausted its wait.
    Timeout,
    /// A complete datagram send reported a different byte count.
    ShortSend,
    /// The supplied monotonic clock moved backward or a deadline overflowed.
    Clock,
    /// The matching Confirmable operation received a Reset.
    Rejected,
}

#[derive(Clone, Copy)]
enum Failure {
    Provisioning(Error),
    Timeout,
    Clock,
    Rejected,
}

impl Failure {
    fn error<E>(self) -> PollError<E> {
        match self {
            Self::Provisioning(error) => PollError::Provisioning(error),
            Self::Timeout => PollError::Timeout,
            Self::Clock => PollError::Clock,
            Self::Rejected => PollError::Rejected,
        }
    }
}

struct Wire {
    bytes: [u8; DATAGRAM_CAPACITY],
    len: usize,
}

impl Wire {
    fn empty() -> Self {
        Self {
            bytes: [0; DATAGRAM_CAPACITY],
            len: 0,
        }
    }

    fn copy(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > DATAGRAM_CAPACITY {
            return Err(Error::Parsing);
        }
        let mut wire = Self::empty();
        wire.bytes[..bytes.len()].copy_from_slice(bytes);
        wire.len = bytes.len();
        Ok(wire)
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[derive(Clone, Copy)]
struct Operation {
    mid: MessageId,
    token: Token,
}

struct Ids {
    first: Operation,
    second: Operation,
    echo: Operation,
    first_timeout: u64,
    second_timeout: u64,
}

impl Ids {
    fn fresh(entropy: &mut impl FnMut(&mut [u8]) -> bool) -> Result<Self, Error> {
        let mut seed = [0; 22];
        if !entropy(&mut seed) {
            return Err(Error::Entropy);
        }
        if seed[2..10] == seed[10..18] {
            seed[17] ^= 1;
        }
        let mid = MessageId::new(u16::from_be_bytes([seed[0], seed[1]]));
        let jitter = |a, b| u64::from(u16::from_be_bytes([a, b])) * 1001 / 65_536;
        Ok(Self {
            first: Operation {
                mid,
                token: Token::from_checked(&seed[2..10]),
            },
            second: Operation {
                mid: mid.wrapping_add(1),
                token: Token::from_checked(&seed[10..18]),
            },
            echo: Operation {
                mid: mid.wrapping_add(2),
                token: Token::from_checked(&seed[2..10]),
            },
            first_timeout: 2000 + jitter(seed[18], seed[19]),
            second_timeout: 2000 + jitter(seed[20], seed[21]),
        })
    }
}

struct Retry {
    initial: u64,
    interval: u64,
    first_sent: Option<u64>,
    next_retry: Option<u64>,
    wait_until: Option<u64>,
    retransmits: u8,
}

impl Retry {
    fn new(initial: u64) -> Self {
        Self {
            initial,
            interval: initial,
            first_sent: None,
            next_retry: None,
            wait_until: None,
            retransmits: 0,
        }
    }

    fn deadline(&self, now: u64) -> u64 {
        if self.first_sent.is_none() {
            now
        } else {
            self.next_retry.or(self.wait_until).unwrap_or(now)
        }
    }

    fn due(&self, now: u64) -> bool {
        self.first_sent.is_none() || self.next_retry.is_some_and(|deadline| now >= deadline)
    }

    fn sent(&mut self, now: u64) -> Result<(), Failure> {
        if let Some(first) = self.first_sent {
            self.retransmits += 1;
            self.interval *= 2;
            let deadline = now.checked_add(self.interval).ok_or(Failure::Clock)?;
            let span = first.checked_add(self.initial * 15).ok_or(Failure::Clock)?;
            self.next_retry =
                (self.retransmits < MAX_RETRANSMIT && deadline <= span).then_some(deadline);
        } else {
            self.first_sent = Some(now);
            self.next_retry = Some(now.checked_add(self.initial).ok_or(Failure::Clock)?);
            let deadline = now.checked_add(self.initial * 31).ok_or(Failure::Clock)?;
            self.wait_until = Some(self.wait_until.map_or(deadline, |old| old.min(deadline)));
        }
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)]
enum ClientState {
    Message2(Initiator),
    Message4(InitiatorConfirm),
    Complete,
    Failed,
}

struct ResponseAck {
    response: Wire,
    mid: MessageId,
    expires: u64,
}

struct Client {
    state: ClientState,
    ids: Ids,
    operation: Operation,
    request: Wire,
    retry: Retry,
    acks: [Option<ResponseAck>; 3],
    pending_acks: u8,
    echo_retried: bool,
}

#[allow(clippy::large_enum_variant)]
enum ServerState {
    Message1,
    Message3(Responder),
    Complete,
    Failed,
}

struct Exchange {
    operation: Operation,
    request: Wire,
    response: Wire,
    expires: u64,
}

struct Server {
    state: ServerState,
    exchanges: [Option<Exchange>; 2],
    pending: u8,
}

#[allow(clippy::large_enum_variant)]
enum Role {
    Client(Client),
    Server(Server),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompletedCacheMatch {
    ExactDuplicate,
    MidCollision,
    NoMatch,
}

fn same_mid_namespace(left: u8, right: u8) -> bool {
    (left ^ right) & 0x20 == 0
}

struct Input<'a> {
    now: u64,
    identity: &'a Identity,
    peer: &'a PinnedPeer,
    local_id: ConnectionId,
    expires: &'a mut Option<u64>,
    session: &'a mut Option<Session>,
    entropy: &'a mut dyn FnMut(&mut [u8]) -> bool,
    authorize: &'a mut dyn FnMut(&Principal) -> bool,
}

/// Bounded, allocator-free sequential bootstrap for one authenticated peer.
///
/// Each datagram buffer holds at most 384 bytes. One Confirmable request is
/// outstanding at a time, using a randomized 2–3 second initial timeout and at
/// most four retransmissions. Caller polling never extends the transmit span
/// or the 247 second exchange lifetime. Endpoint, Message ID, token and complete
/// request bytes isolate cached responses. Endpoint metadata supplies routing,
/// while the installed credential and resulting [`Principal`] supply identity.
///
/// Coordinate this controller with other traffic to preserve one outstanding
/// Confirmable operation per endpoint and prevent Message ID reuse during the
/// exchange lifetime. A new controller requires fresh entropy and a new
/// handshake; transport retries reuse their cached bytes without entropy.
pub struct CoapProvisioner<'a> {
    identity: &'a Identity,
    peer: PinnedPeer,
    endpoint: Endpoint,
    local_id: ConnectionId,
    role: Role,
    last_now: u64,
    expires: Option<u64>,
    session: Option<Session>,
    failure: Option<Failure>,
    expired: bool,
}

impl<'a> CoapProvisioner<'a> {
    /// Maximum complete CoAP datagram accepted by this bootstrap profile.
    pub const DATAGRAM_CAPACITY: usize = DATAGRAM_CAPACITY;

    /// Milliseconds that an accepted exchange's duplicate cache remains valid.
    pub const EXCHANGE_LIFETIME_MS: u64 = EXCHANGE_LIFETIME_MS;

    /// Starts an initiator with fresh operation identifiers, jitter and keys.
    /// The first [`Self::poll`] or [`Self::flush`] sends the cached message 1.
    /// The default local connection ID zero must be reserved outside live sessions.
    pub fn start(
        identity: &'a Identity,
        peer: PinnedPeer,
        endpoint: Endpoint,
        now_ms: u64,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<Self, PollError> {
        Self::start_with_id(
            identity,
            peer,
            endpoint,
            ConnectionId::INITIATOR,
            now_ms,
            entropy,
        )
    }

    /// Starts with a local ID reserved outside live handshakes and OSCORE contexts.
    /// The authenticated responder selects a distinct ID for routing message 3.
    pub fn start_with_id(
        identity: &'a Identity,
        peer: PinnedPeer,
        endpoint: Endpoint,
        local_id: ConnectionId,
        now_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<Self, PollError> {
        let expires = now_ms
            .checked_add(EXCHANGE_LIFETIME_MS)
            .ok_or(PollError::Clock)?;
        let ids = Ids::fresh(&mut entropy).map_err(PollError::Provisioning)?;
        let (state, message) =
            Initiator::start_with_id(identity, peer.clone(), local_id, &mut entropy)
                .map_err(PollError::Provisioning)?;
        let request = request(ids.first, &[0xf5], &message).map_err(PollError::Provisioning)?;
        let operation = ids.first;
        let retry = Retry::new(ids.first_timeout);
        Ok(Self {
            identity,
            peer,
            endpoint,
            local_id,
            role: Role::Client(Client {
                state: ClientState::Message2(state),
                ids,
                operation,
                request,
                retry,
                acks: [None, None, None],
                pending_acks: 0,
                echo_retried: false,
            }),
            last_now: now_ms,
            expires: Some(expires),
            session: None,
            failure: None,
            expired: false,
        })
    }

    /// Listens for one pinned initiator at the supplied endpoint.
    /// Waiting for message 1 has no deadline; its first valid CoAP envelope
    /// begins the bounded handshake and requests fresh ephemeral entropy.
    /// The default local connection ID one must be reserved outside live sessions.
    #[must_use]
    pub fn listen(
        identity: &'a Identity,
        peer: PinnedPeer,
        endpoint: Endpoint,
        now_ms: u64,
    ) -> Self {
        Self::listen_with_id(identity, peer, endpoint, ConnectionId::RESPONDER, now_ms)
    }

    /// Listens with a local ID reserved outside live handshakes and OSCORE contexts.
    /// A colliding or unsupported initiator ID is refused before requesting entropy.
    #[must_use]
    pub fn listen_with_id(
        identity: &'a Identity,
        peer: PinnedPeer,
        endpoint: Endpoint,
        local_id: ConnectionId,
        now_ms: u64,
    ) -> Self {
        Self {
            identity,
            peer,
            endpoint,
            local_id,
            role: Role::Server(Server {
                state: ServerState::Message1,
                exchanges: [None, None],
                pending: 0,
            }),
            last_now: now_ms,
            expires: None,
            session: None,
            failure: None,
            expired: false,
        }
    }

    /// Returns the caller-reserved local EDHOC connection and OSCORE recipient ID.
    #[must_use]
    pub const fn local_connection_id(&self) -> ConnectionId {
        self.local_id
    }

    pub(super) fn classify_completed_cache(
        &self,
        bytes: &[u8],
        endpoint: Endpoint,
        now_ms: u64,
    ) -> CompletedCacheMatch {
        if endpoint != self.endpoint
            || self.failure.is_some()
            || self.expired
            || !self.complete()
            || self.expires.is_none_or(|expires| now_ms >= expires)
            || bytes.len() < 4
        {
            return CompletedCacheMatch::NoMatch;
        }
        let mid = MessageId::new(u16::from_be_bytes([bytes[2], bytes[3]]));
        let cached = match &self.role {
            Role::Client(client) => client.acks.iter().flatten().find_map(|response| {
                (now_ms < response.expires
                    && mid == response.mid
                    && same_mid_namespace(bytes[0], response.response.bytes[0]))
                .then_some(response.response.as_slice())
            }),
            Role::Server(server) => server.exchanges.iter().flatten().find_map(|exchange| {
                (now_ms < exchange.expires
                    && mid == exchange.operation.mid
                    && same_mid_namespace(bytes[0], exchange.request.bytes[0]))
                .then_some(exchange.request.as_slice())
            }),
        };
        match cached {
            Some(cached) if cached == bytes => CompletedCacheMatch::ExactDuplicate,
            Some(_) => CompletedCacheMatch::MidCollision,
            None => CompletedCacheMatch::NoMatch,
        }
    }

    /// Returns the exact monotonic millisecond deadline for the next action.
    /// Immediate cached sends use the last supplied time. `None` means idle
    /// listening, a terminal failure or expired completion caches.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        if self.failure.is_some() || self.expired {
            return None;
        }
        let action = match &self.role {
            Role::Client(client) => {
                if client.pending_acks != 0 {
                    Some(self.last_now)
                } else if matches!(
                    client.state,
                    ClientState::Message2(_) | ClientState::Message4(_)
                ) {
                    Some(client.retry.deadline(self.last_now))
                } else {
                    None
                }
            }
            Role::Server(server) => (server.pending != 0).then_some(self.last_now),
        };
        match (action, self.expires) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Moves the authenticated session out after its pending confirmation sends.
    /// The controller remains resident to answer exact completion duplicates.
    /// Bind a new secure App and demultiplex bootstrap traffic with [`Self::ingest`].
    pub fn take_session(&mut self) -> Option<Session> {
        if self.failure.is_some() || self.expired || !self.complete() || self.pending() {
            return None;
        }
        self.session.take()
    }

    /// Sends pending cached bytes, receives at most one datagram, then sends any
    /// generated reply or due retry. Use an exclusively owned bootstrap transport.
    /// Entropy is used only by a responder's first accepted handshake attempt;
    /// cached duplicates and retransmissions never call it.
    pub fn poll<T: DatagramIo>(
        &mut self,
        io: &mut T,
        now_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
        mut authorize: impl FnMut(&Principal) -> bool,
    ) -> Result<Status, PollError<T::Error>> {
        self.check_time(now_ms)?;
        if self.expired {
            return Ok(Status::Expired);
        }
        let mut progress = false;
        if self.pending() {
            progress = matches!(self.flush(io, now_ms)?, Status::Progress | Status::Complete);
        }
        let mut bytes = [0; DATAGRAM_CAPACITY];
        let received = io.recv(&mut bytes).map_err(PollError::Io)?;
        let mut ignored = false;
        if let Some((len, endpoint)) = received {
            if len > bytes.len() {
                return Err(PollError::Provisioning(Error::Parsing));
            }
            match self.ingest(
                &bytes[..len],
                endpoint,
                now_ms,
                &mut entropy,
                &mut authorize,
            ) {
                Ok(Status::Progress | Status::Complete) => progress = true,
                Ok(Status::Ignored) => ignored = true,
                Ok(_) => {}
                Err(error) => return Err(widen(error)),
            }
        }
        progress |= matches!(self.flush(io, now_ms)?, Status::Progress | Status::Complete);
        Ok(self.status(progress, ignored))
    }

    /// Processes one complete datagram routed to bootstrap by the caller.
    /// Unrelated endpoints or invalid CoAP metadata leave the handshake intact.
    /// A matching wire-valid malformed EDHOC message consumes its state.
    /// Responses remain queued for [`Self::flush`], without receiving App traffic.
    pub fn ingest(
        &mut self,
        bytes: &[u8],
        endpoint: Endpoint,
        now_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
        mut authorize: impl FnMut(&Principal) -> bool,
    ) -> Result<Status, PollError> {
        self.check_time(now_ms)?;
        if self.expired {
            return Ok(Status::Expired);
        }
        if endpoint != self.endpoint || bytes.len() > DATAGRAM_CAPACITY {
            return Ok(Status::Ignored);
        }
        let Ok(parsed) = decode(bytes) else {
            return Ok(Status::Ignored);
        };
        let mut input = Input {
            now: now_ms,
            identity: self.identity,
            peer: &self.peer,
            local_id: self.local_id,
            expires: &mut self.expires,
            session: &mut self.session,
            entropy: &mut entropy,
            authorize: &mut authorize,
        };
        let result = match &mut self.role {
            Role::Client(client) => client.ingest(bytes, parsed, &mut input),
            Role::Server(server) => server.ingest(bytes, parsed, &mut input),
        };
        match result {
            Ok(true) => Ok(self.status(true, false)),
            Ok(false) => Ok(Status::Ignored),
            Err(failure) => {
                self.failure = Some(failure);
                self.session = None;
                Err(failure.error())
            }
        }
    }

    /// Sends cached replies or a due Confirmable retry without receiving traffic.
    /// A failed or short send retains exactly the same bytes for the next call.
    pub fn flush<T: DatagramIo>(
        &mut self,
        io: &mut T,
        now_ms: u64,
    ) -> Result<Status, PollError<T::Error>> {
        self.check_time(now_ms)?;
        if self.expired {
            return Ok(Status::Expired);
        }
        let mut progress = false;
        match &mut self.role {
            Role::Client(client) => {
                for index in 0..client.acks.len() {
                    let bit = 1 << index;
                    if client.pending_acks & bit != 0 {
                        let ack = client.acks[index]
                            .as_ref()
                            .expect("queued response acknowledgement");
                        if now_ms < ack.expires {
                            let mut bytes = [0; 4];
                            let len = CoapMessage::empty_ack(ack.mid)
                                .encode(&mut bytes)
                                .expect("four-byte empty acknowledgement");
                            send(io, self.endpoint, &bytes[..len])?;
                            progress = true;
                        }
                        client.pending_acks &= !bit;
                    }
                }
                if matches!(
                    client.state,
                    ClientState::Message2(_) | ClientState::Message4(_)
                ) && client.retry.due(now_ms)
                {
                    if let Some(first) = client.retry.first_sent {
                        if now_ms > first + client.retry.initial * 15 {
                            client.retry.next_retry = None;
                            return Ok(self.status(progress, false));
                        }
                    }
                    send(io, self.endpoint, client.request.as_slice())?;
                    if let Err(failure) = client.retry.sent(now_ms) {
                        self.failure = Some(failure);
                        return Err(failure.error());
                    }
                    progress = true;
                }
            }
            Role::Server(server) => {
                for index in 0..server.exchanges.len() {
                    let bit = 1 << index;
                    if server.pending & bit != 0 {
                        let exchange = server.exchanges[index]
                            .as_ref()
                            .expect("queued cached response");
                        if now_ms < exchange.expires {
                            send(io, self.endpoint, exchange.response.as_slice())?;
                            progress = true;
                        }
                        server.pending &= !bit;
                    }
                }
            }
        }
        Ok(self.status(progress, false))
    }

    fn complete(&self) -> bool {
        match &self.role {
            Role::Client(client) => matches!(client.state, ClientState::Complete),
            Role::Server(server) => matches!(server.state, ServerState::Complete),
        }
    }

    fn pending(&self) -> bool {
        match &self.role {
            Role::Client(client) => {
                client.pending_acks != 0
                    || (matches!(
                        client.state,
                        ClientState::Message2(_) | ClientState::Message4(_)
                    ) && client.retry.first_sent.is_none())
            }
            Role::Server(server) => server.pending != 0,
        }
    }

    fn status(&self, progress: bool, ignored: bool) -> Status {
        if self.expired {
            Status::Expired
        } else if self.complete() && !self.pending() {
            Status::Complete
        } else if progress {
            Status::Progress
        } else if ignored {
            Status::Ignored
        } else {
            Status::Idle
        }
    }

    fn check_time<E>(&mut self, now: u64) -> Result<(), PollError<E>> {
        if let Some(failure) = self.failure {
            return Err(failure.error());
        }
        if now < self.last_now {
            self.failure = Some(Failure::Clock);
            self.session = None;
            return Err(PollError::Clock);
        }
        self.last_now = now;
        if self.expires.is_some_and(|expires| now >= expires) {
            if self.complete() {
                self.expired = true;
                self.session = None;
                match &mut self.role {
                    Role::Client(client) => {
                        client.acks = [None, None, None];
                        client.pending_acks = 0;
                    }
                    Role::Server(server) => {
                        server.exchanges = [None, None];
                        server.pending = 0;
                    }
                }
                return Ok(());
            }
            self.failure = Some(Failure::Timeout);
            return Err(PollError::Timeout);
        }
        if let Role::Client(client) = &self.role {
            if matches!(
                client.state,
                ClientState::Message2(_) | ClientState::Message4(_)
            ) && client
                .retry
                .wait_until
                .is_some_and(|expires| now >= expires)
            {
                self.failure = Some(Failure::Timeout);
                return Err(PollError::Timeout);
            }
        }
        Ok(())
    }
}

impl fmt::Debug for CoapProvisioner<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoapProvisioner")
            .field("endpoint", &self.endpoint)
            .field("deadline", &self.next_deadline())
            .finish_non_exhaustive()
    }
}

impl Client {
    fn ingest(
        &mut self,
        bytes: &[u8],
        parsed: ParsedMessage<'_>,
        input: &mut Input<'_>,
    ) -> Result<bool, Failure> {
        let now = input.now;
        for (index, ack) in self.acks.iter().enumerate() {
            if let Some(ack) = ack {
                if now < ack.expires
                    && parsed.message_id() == ack.mid
                    && same_mid_namespace(bytes[0], ack.response.bytes[0])
                {
                    if ack.response.as_slice() == bytes {
                        if parsed.ty() == Type::Confirmable {
                            self.pending_acks |= 1 << index;
                        }
                        return Ok(true);
                    }
                    return Ok(false);
                }
            }
        }
        if !matches!(
            self.state,
            ClientState::Message2(_) | ClientState::Message4(_)
        ) {
            return Ok(false);
        }
        if parsed.is_empty_ack_or_rst() && parsed.message_id() == self.operation.mid {
            if parsed.is_empty_rst() {
                return Err(Failure::Rejected);
            }
            self.retry.next_retry = None;
            self.retry.wait_until = *input.expires;
            return Ok(true);
        }
        if matches!(self.state, ClientState::Message2(_))
            && !self.echo_retried
            && parsed.ty() == Type::Acknowledgement
            && parsed.code() == Code::UNAUTHORIZED
            && parsed.message_id() == self.operation.mid
            && parsed.token() == self.operation.token
            && parsed.payload().is_empty()
        {
            let mut options = parsed.options();
            let Some(echo) = options.next() else {
                return Ok(false);
            };
            if echo.number() != OptionNumber::ECHO
                || !(1..=40).contains(&echo.value().len())
                || options.next().is_some()
            {
                return Ok(false);
            }
            let old = decode(self.request.as_slice())
                .map_err(|_| Failure::Provisioning(Error::Parsing))?;
            let options = [
                Opt::new(OptionNumber::URI_PATH, b".well-known"),
                Opt::new(OptionNumber::URI_PATH, b"edhoc"),
                Opt::new(OptionNumber::CONTENT_FORMAT, &[REQUEST_FORMAT as u8]),
                Opt::new(OptionNumber::ECHO, echo.value()),
            ];
            let mut request = Wire::empty();
            request.len = CoapMessage::con(Code::POST, self.ids.echo.mid, self.ids.echo.token)
                .with_options(&options)
                .with_payload(old.payload())
                .encode(&mut request.bytes)
                .map_err(|_| Failure::Provisioning(Error::Parsing))?;
            self.acks[2] = Some(ResponseAck {
                response: Wire::copy(bytes).map_err(Failure::Provisioning)?,
                mid: parsed.message_id(),
                expires: input.expires.ok_or(Failure::Clock)?,
            });
            let deadline = self.retry.wait_until;
            self.retry = Retry::new(self.ids.first_timeout);
            self.retry.wait_until = deadline;
            self.operation = self.ids.echo;
            self.request = request;
            self.echo_retried = true;
            return Ok(true);
        }
        if parsed.token() != self.operation.token
            || (parsed.ty() == Type::Acknowledgement && parsed.message_id() != self.operation.mid)
            || !response_metadata(parsed)
        {
            return Ok(false);
        }
        let until = now
            .checked_add(EXCHANGE_LIFETIME_MS)
            .ok_or(Failure::Clock)?;
        let message = Message::from_slice(parsed.payload()).map_err(Failure::Provisioning)?;
        let index = match self.state {
            ClientState::Message2(_) => 0,
            _ => 1,
        };
        let state = core::mem::replace(&mut self.state, ClientState::Failed);
        match state {
            ClientState::Message2(state) => {
                let (state, message) = state
                    .receive_message_2(&message, &mut input.authorize)
                    .map_err(Failure::Provisioning)?;
                self.request = request(
                    self.ids.second,
                    state.peer_connection_id().lakers().as_cbor(),
                    &message,
                )
                .map_err(Failure::Provisioning)?;
                self.operation = self.ids.second;
                self.retry = Retry::new(self.ids.second_timeout);
                self.state = ClientState::Message4(state);
            }
            ClientState::Message4(state) => {
                *input.session = Some(
                    state
                        .receive_message_4(&message, &mut input.authorize)
                        .map_err(Failure::Provisioning)?,
                );
                self.state = ClientState::Complete;
                *input.expires = Some(until);
            }
            _ => return Ok(false),
        }
        self.acks[index] = Some(ResponseAck {
            response: Wire::copy(bytes).map_err(Failure::Provisioning)?,
            mid: parsed.message_id(),
            expires: until,
        });
        if parsed.ty() == Type::Confirmable {
            self.pending_acks |= 1 << index;
        }
        Ok(true)
    }
}

impl Server {
    fn ingest(
        &mut self,
        bytes: &[u8],
        parsed: ParsedMessage<'_>,
        input: &mut Input<'_>,
    ) -> Result<bool, Failure> {
        let now = input.now;
        for (index, exchange) in self.exchanges.iter().enumerate() {
            if let Some(exchange) = exchange {
                if now < exchange.expires
                    && parsed.message_id() == exchange.operation.mid
                    && same_mid_namespace(bytes[0], exchange.request.bytes[0])
                {
                    if exchange.request.as_slice() == bytes {
                        self.pending |= 1 << index;
                        return Ok(true);
                    }
                    return Ok(false);
                }
            }
        }
        if !request_metadata(parsed) || parsed.payload().len() < 2 {
            return Ok(false);
        }
        let prefix_len = match &self.state {
            ServerState::Message1 if parsed.payload()[0] == 0xf5 => 1,
            ServerState::Message3(_) => {
                let Ok((id, len)) = ConnectionId::decode_prefix(parsed.payload()) else {
                    return Ok(false);
                };
                if id != input.local_id {
                    return Ok(false);
                }
                len
            }
            _ => return Ok(false),
        };
        if parsed.payload().len() <= prefix_len {
            return Ok(false);
        }
        let until = now
            .checked_add(EXCHANGE_LIFETIME_MS)
            .ok_or(Failure::Clock)?;
        let message =
            Message::from_slice(&parsed.payload()[prefix_len..]).map_err(Failure::Provisioning)?;
        let operation = Operation {
            mid: parsed.message_id(),
            token: parsed.token(),
        };
        let state = core::mem::replace(&mut self.state, ServerState::Failed);
        let (index, message) = match state {
            ServerState::Message1 => {
                let (state, message) = Responder::receive_message_1_with_id(
                    input.identity,
                    input.peer.clone(),
                    input.local_id,
                    &message,
                    &mut input.entropy,
                )
                .map_err(Failure::Provisioning)?;
                self.state = ServerState::Message3(state);
                *input.expires = Some(until);
                (0, message)
            }
            ServerState::Message3(state) => {
                let (ready, message) = state
                    .receive_message_3(&message, &mut input.authorize)
                    .map_err(Failure::Provisioning)?;
                *input.session = Some(ready);
                self.state = ServerState::Complete;
                *input.expires = Some(until);
                (1, message)
            }
            _ => return Ok(false),
        };
        self.exchanges[index] = Some(Exchange {
            operation,
            request: Wire::copy(bytes).map_err(Failure::Provisioning)?,
            response: response(operation, &message).map_err(Failure::Provisioning)?,
            expires: until,
        });
        self.pending |= 1 << index;
        Ok(true)
    }
}

fn request(operation: Operation, prefix: &[u8], message: &Message) -> Result<Wire, Error> {
    let mut payload = [0; DATAGRAM_CAPACITY];
    let len = message.as_bytes().len() + prefix.len();
    if len > payload.len() {
        return Err(Error::Parsing);
    }
    payload[..prefix.len()].copy_from_slice(prefix);
    payload[prefix.len()..len].copy_from_slice(message.as_bytes());
    let options = [
        Opt::new(OptionNumber::URI_PATH, b".well-known"),
        Opt::new(OptionNumber::URI_PATH, b"edhoc"),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[REQUEST_FORMAT as u8]),
    ];
    let mut wire = Wire::empty();
    wire.len = CoapMessage::con(Code::POST, operation.mid, operation.token)
        .with_options(&options)
        .with_payload(&payload[..len])
        .encode(&mut wire.bytes)
        .map_err(|_| Error::Parsing)?;
    Ok(wire)
}

fn response(operation: Operation, message: &Message) -> Result<Wire, Error> {
    let options = [Opt::new(
        OptionNumber::CONTENT_FORMAT,
        &[RESPONSE_FORMAT as u8],
    )];
    let mut wire = Wire::empty();
    wire.len = CoapMessage::new(Type::Acknowledgement, Code::CHANGED, operation.mid)
        .with_token(operation.token)
        .with_options(&options)
        .with_payload(message.as_bytes())
        .encode(&mut wire.bytes)
        .map_err(|_| Error::Parsing)?;
    Ok(wire)
}

fn request_metadata(message: ParsedMessage<'_>) -> bool {
    if message.ty() != Type::Confirmable || message.code() != Code::POST {
        return false;
    }
    let mut options = message.options();
    matches!(options.next(), Some(option) if option.number() == OptionNumber::URI_PATH && option.value() == b".well-known")
        && matches!(options.next(), Some(option) if option.number() == OptionNumber::URI_PATH && option.value() == b"edhoc")
        && matches!(options.next(), Some(option) if option.number() == OptionNumber::CONTENT_FORMAT && decode_uint16(option.value()) == Ok(REQUEST_FORMAT))
        && options.next().is_none()
}

fn response_metadata(message: ParsedMessage<'_>) -> bool {
    if !matches!(
        message.ty(),
        Type::Acknowledgement | Type::Confirmable | Type::NonConfirmable
    ) || message.code() != Code::CHANGED
        || message.payload().is_empty()
    {
        return false;
    }
    let mut options = message.options();
    matches!(options.next(), Some(option) if option.number() == OptionNumber::CONTENT_FORMAT && decode_uint16(option.value()) == Ok(RESPONSE_FORMAT))
        && options.next().is_none()
}

fn send<T: DatagramIo>(
    io: &mut T,
    endpoint: Endpoint,
    bytes: &[u8],
) -> Result<(), PollError<T::Error>> {
    let sent = io.send(endpoint, bytes).map_err(PollError::Io)?;
    if sent != bytes.len() {
        return Err(PollError::ShortSend);
    }
    Ok(())
}

fn widen<E>(error: PollError) -> PollError<E> {
    match error {
        PollError::Io(error) => match error {},
        PollError::Provisioning(error) => PollError::Provisioning(error),
        PollError::Timeout => PollError::Timeout,
        PollError::ShortSend => PollError::ShortSend,
        PollError::Clock => PollError::Clock,
        PollError::Rejected => PollError::Rejected,
    }
}

#[cfg(test)]
#[path = "coap_tests.rs"]
mod tests;
