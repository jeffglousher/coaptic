//! Bounded replacement handshakes without ownership of the live application.

use core::{convert::Infallible, fmt};

use super::super::{ConnectionId, Identity, PeerError, PinnedPeer, Principal, Session};
use super::{
    ClientState, CoapProvisioner, CompletedCacheMatch, Control, ControlKind, DATAGRAM_CAPACITY,
    EXCHANGE_LIFETIME_MS, ErrorReceipt, ErrorRequest, Exchange, Operation, OptionRole, PollError,
    ResponseAck, Role, Status, flush_control, has_unsupported_critical, queue_control,
    request_metadata, request_path, request_peer_error, same_mid_namespace, send,
};
use crate::message::{Code, Message as CoapMessage, MessageId, Type, decode};
use crate::storage::{DatagramIo, Endpoint};

/// Failure of a bounded replacement handshake; the caller's live App is untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryError<E = Infallible> {
    /// No completion-cache slot or unreserved compact connection ID is available.
    Capacity,
    /// A candidate or an unconfirmed application handoff already exists.
    Busy,
    /// The candidate, cached send, or monotonic clock failed.
    Bootstrap(PollError<E>),
}

#[allow(clippy::large_enum_variant)]
enum CachedRole {
    Client {
        operations: [Operation; 4],
        acks: [Option<ResponseAck>; 3],
        pending: u8,
        peer_error_slot: Option<usize>,
        abort: Option<ErrorRequest>,
    },
    Server {
        exchanges: [Option<Exchange>; 2],
        pending: u8,
        peer_error_slot: Option<usize>,
        final_ready: bool,
        final_error: Option<ErrorReceipt>,
    },
}

struct Cache {
    local_id: ConnectionId,
    expires: u64,
    replies: bool,
    role: CachedRole,
    control: Option<Control>,
}

impl Cache {
    fn from_controller(controller: CoapProvisioner<'_>, expires: u64, replies: bool) -> Self {
        let mut control = controller.control;
        if !replies && let Some(control) = &mut control {
            control.pending = false;
        }
        let expires = control
            .as_ref()
            .map_or(expires, |control| expires.max(control.expires));
        let role = match controller.role {
            Role::Client(client) => {
                let (peer_error_slot, abort) = match client.state {
                    ClientState::Aborted { slot, .. } => (Some(slot), None),
                    ClientState::LocalAbort { request, .. } if replies => (None, Some(request)),
                    _ => (None, None),
                };
                CachedRole::Client {
                    operations: [
                        client.ids.first,
                        client.ids.second,
                        client.ids.echo,
                        client.ids.error,
                    ],
                    acks: client.acks,
                    pending: if replies { client.pending_acks } else { 0 },
                    peer_error_slot,
                    abort,
                }
            }
            Role::Server(server) => {
                let (peer_error_slot, final_ready, final_error) = match server.state {
                    super::ServerState::PeerAborted { slot, .. } => (Some(slot), false, None),
                    super::ServerState::Complete => (None, true, None),
                    super::ServerState::FinalAbort(receipt) => (None, false, Some(receipt)),
                    _ => (None, false, None),
                };
                CachedRole::Server {
                    exchanges: server.exchanges,
                    pending: if replies { server.pending } else { 0 },
                    peer_error_slot,
                    final_ready,
                    final_error,
                }
            }
        };
        Self {
            local_id: controller.local_id,
            expires,
            replies,
            role,
            control,
        }
    }

    fn classify(&self, bytes: &[u8], now: u64) -> CompletedCacheMatch {
        if bytes.len() < 4 || now >= self.expires {
            return CompletedCacheMatch::NoMatch;
        }
        let mid = MessageId::new(u16::from_be_bytes([bytes[2], bytes[3]]));
        if bytes.len() <= DATAGRAM_CAPACITY
            && self.control.as_ref().is_some_and(|control| {
                decode(bytes).is_ok_and(|message| control.owns(message, now))
            })
        {
            return CompletedCacheMatch::RefusedMid;
        }
        match &self.role {
            CachedRole::Client {
                operations,
                acks,
                abort,
                ..
            } => {
                for ack in acks.iter().flatten() {
                    if now < ack.expires
                        && mid == ack.mid
                        && same_mid_namespace(bytes[0], ack.response.bytes[0])
                    {
                        return if bytes == ack.response.as_slice() {
                            CompletedCacheMatch::ExactDuplicate
                        } else {
                            CompletedCacheMatch::MidCollision
                        };
                    }
                }
                if let Some(abort) = abort
                    && let Ok(parsed) = decode(bytes)
                {
                    let classified = abort.classify(bytes, parsed, now);
                    if classified != CompletedCacheMatch::NoMatch {
                        return classified;
                    }
                }
                if bytes[0] & 0x20 != 0 && operations.iter().any(|operation| operation.mid == mid) {
                    return CompletedCacheMatch::MidCollision;
                }
            }
            CachedRole::Server {
                exchanges,
                final_error,
                final_ready,
                ..
            } => {
                for exchange in exchanges.iter().flatten() {
                    if now < exchange.expires
                        && mid == exchange.operation.mid
                        && same_mid_namespace(bytes[0], exchange.request.bytes[0])
                    {
                        return if bytes == exchange.request.as_slice() {
                            CompletedCacheMatch::ExactDuplicate
                        } else {
                            CompletedCacheMatch::MidCollision
                        };
                    }
                }
                if let Some(receipt) = final_error {
                    return receipt.classify(bytes, now);
                }
                if self.replies && *final_ready && ErrorReceipt::matches_new(bytes, self.local_id) {
                    return CompletedCacheMatch::FreshErrorRequest;
                }
            }
        }
        CompletedCacheMatch::NoMatch
    }

    fn queue(&mut self, bytes: &[u8], now: u64) -> bool {
        if !self.replies {
            return false;
        }
        if let Some(control) = &mut self.control
            && decode(bytes)
                .is_ok_and(|message| control.owns(message, now) && control.can_replay(message))
        {
            control.pending = true;
            return true;
        }
        match &mut self.role {
            CachedRole::Client {
                acks,
                pending,
                abort,
                ..
            } => {
                for (index, ack) in acks.iter().enumerate() {
                    if ack.as_ref().is_some_and(|ack| {
                        now < ack.expires
                            && bytes == ack.response.as_slice()
                            && decode(bytes).is_ok_and(|message| {
                                message.ty() == crate::message::Type::Confirmable
                            })
                    }) {
                        *pending |= 1 << index;
                        return true;
                    }
                }
                if let Some(abort) = abort {
                    return decode(bytes).is_ok_and(|parsed| abort.ingest(bytes, parsed, now));
                }
            }
            CachedRole::Server {
                exchanges,
                pending,
                final_error,
                final_ready,
                ..
            } => {
                for (index, exchange) in exchanges.iter().enumerate() {
                    if exchange.as_ref().is_some_and(|exchange| {
                        now < exchange.expires && bytes == exchange.request.as_slice()
                    }) {
                        *pending |= 1 << index;
                        return true;
                    }
                }
                if let Some(receipt) = final_error {
                    return receipt.queue(bytes, now);
                }
                if *final_ready && ErrorReceipt::matches_new(bytes, self.local_id) {
                    let Some(expires) = now.checked_add(EXCHANGE_LIFETIME_MS) else {
                        return false;
                    };
                    *final_error = Some(ErrorReceipt::new(bytes, expires));
                    *final_ready = false;
                    self.expires = self.expires.max(expires);
                    return true;
                }
            }
        }
        false
    }

    fn peer_error(&self) -> Option<PeerError<'_>> {
        match &self.role {
            CachedRole::Client {
                acks,
                peer_error_slot: Some(slot),
                ..
            } => PeerError::parse(
                decode(acks[*slot].as_ref()?.response.as_slice())
                    .ok()?
                    .payload(),
            ),
            CachedRole::Server {
                final_error: Some(receipt),
                ..
            } => Some(receipt.peer_error()),
            CachedRole::Server {
                exchanges,
                peer_error_slot: Some(slot),
                ..
            } => request_peer_error(exchanges[*slot].as_ref()?.request.as_slice()),
            _ => None,
        }
    }

    fn pending(&self) -> bool {
        self.control.as_ref().is_some_and(|c| c.pending)
            || match &self.role {
                CachedRole::Client { pending, .. } => *pending != 0,
                CachedRole::Server {
                    pending,
                    final_error,
                    ..
                } => *pending != 0 || final_error.as_ref().is_some_and(ErrorReceipt::pending),
            }
            || matches!(&self.role, CachedRole::Client { abort: Some(abort), .. } if abort.pending())
    }

    const fn error_in_flight(&self, now: u64) -> bool {
        matches!(&self.role, CachedRole::Client { abort: Some(abort), .. } if abort.in_flight(now))
    }

    fn next_deadline(&self, now: u64) -> u64 {
        let deadline = if self.pending() { now } else { self.expires };
        match &self.role {
            CachedRole::Client {
                abort: Some(abort), ..
            } => deadline.min(abort.next_deadline(now)),
            CachedRole::Server {
                final_error: Some(receipt),
                ..
            } => deadline.min(receipt.next_deadline(now)),
            _ => deadline,
        }
    }

    fn flush<T: DatagramIo>(
        &mut self,
        io: &mut T,
        endpoint: Endpoint,
        now: u64,
    ) -> Result<bool, PollError<T::Error>> {
        let mut progress = false;
        progress |= flush_control(&mut self.control, io, endpoint, now)?;
        match &mut self.role {
            CachedRole::Client {
                acks,
                pending,
                abort,
                ..
            } => {
                for (index, ack) in acks.iter().enumerate() {
                    let bit = 1 << index;
                    if *pending & bit != 0 {
                        let ack = ack.as_ref().expect("queued completion acknowledgement");
                        if now < ack.expires {
                            let mut bytes = [0; 4];
                            let len = CoapMessage::empty_ack(ack.mid)
                                .encode(&mut bytes)
                                .expect("four-byte empty acknowledgement");
                            send(io, endpoint, &bytes[..len])?;
                            progress = true;
                        }
                        *pending &= !bit;
                    }
                }
                if let Some(abort) = abort {
                    progress |= abort.flush(io, endpoint, now)?;
                }
            }
            CachedRole::Server {
                exchanges,
                pending,
                final_error,
                ..
            } => {
                for (index, exchange) in exchanges.iter().enumerate() {
                    let bit = 1 << index;
                    if *pending & bit != 0 {
                        let exchange = exchange.as_ref().expect("queued completion response");
                        if now < exchange.expires {
                            send(io, endpoint, exchange.response.as_slice())?;
                            progress = true;
                        }
                        *pending &= !bit;
                    }
                }
                if let Some(receipt) = final_error {
                    progress |= receipt.flush(io, endpoint, now)?;
                }
            }
        }
        Ok(progress)
    }
}

/// One candidate handshake and a fixed number of completion or retired caches.
///
/// The caller retains its current secure App throughout authentication. Untrusted
/// datagrams cannot replace it, erase its replay state, or evict unexpired caches.
/// Each cache reserves its connection ID until its original lifetime ends; the
/// active application's ID remains reserved even after its cache expires.
/// Saturation rejects new work instead of replacing another exchange.
///
/// Receive once and route bootstrap datagrams to [`Self::ingest`], then call
/// [`Self::flush`] on the same caller-owned transport. This controller never
/// receives application traffic. Before [`Self::start`], quiesce all outgoing
/// App Confirmable interactions with this endpoint and suspend new ones until
/// the candidate terminates, preserving NSTART=1. Also coordinate application
/// Message ID allocation with [`Self::reserves_message_id`].
///
/// Install a returned [`Session`] into a new App, replacing old exchange and
/// replay state, and call [`Self::confirm_handoff`] only after that swap succeeds.
/// Until confirmation, both application IDs stay reserved and new handshakes are
/// refused. If installation fails while the old App remains usable, call
/// [`Self::abandon_handoff`]. Session installation and persistent trust policy
/// remain caller responsibilities.
pub struct CoapRecovery<'a, const CACHES: usize = 2> {
    identity: &'a Identity,
    peer: PinnedPeer,
    endpoint: Endpoint,
    last_now: u64,
    active: Option<ConnectionId>,
    handoff: Option<ConnectionId>,
    candidate: Option<CoapProvisioner<'a>>,
    candidate_exposed: bool,
    candidate_expires: Option<u64>,
    control: Option<Control>,
    caches: [Option<Cache>; CACHES],
}

impl<'a, const CACHES: usize> CoapRecovery<'a, CACHES> {
    /// Initial admission retains one identifier-only refusal control, separate
    /// from candidate and completion-cache storage. Flush before more admission;
    /// sent controls retain their MID until the original exchange lifetime ends.
    pub const CONTROL_CAPACITY: usize = 1;

    /// Creates an idle controller that accepts the first supported message 1.
    /// Zero cache capacity is refused before any entropy or network operation.
    pub fn new(
        identity: &'a Identity,
        peer: PinnedPeer,
        endpoint: Endpoint,
        now_ms: u64,
    ) -> Result<Self, RecoveryError> {
        if CACHES == 0 {
            return Err(RecoveryError::Capacity);
        }
        Ok(Self {
            identity,
            peer,
            endpoint,
            last_now: now_ms,
            active: None,
            handoff: None,
            candidate: None,
            candidate_exposed: false,
            candidate_expires: None,
            control: None,
            caches: core::array::from_fn(|_| None),
        })
    }

    /// Starts a fresh initiator after caller-controlled liveness or policy checks.
    /// A plaintext error, Reset, or bootstrap packet is not a recovery command.
    /// The caller must first quiesce outgoing App Confirmable interactions.
    pub fn start(
        &mut self,
        now_ms: u64,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(), RecoveryError> {
        self.start_with_suite(now_ms, super::super::CipherSuite::Suite2, entropy)
    }

    /// Start a fresh candidate with a fixed local suite policy and no fallback.
    pub fn start_with_suite(
        &mut self,
        now_ms: u64,
        suite: super::super::CipherSuite,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(), RecoveryError> {
        self.check_time(now_ms)?;
        if self.has_candidate() || self.handoff.is_some() {
            return Err(RecoveryError::Busy);
        }
        let expires = now_ms
            .checked_add(EXCHANGE_LIFETIME_MS)
            .ok_or(RecoveryError::Bootstrap(PollError::Clock))?;
        let local_id = self.available_id(None).ok_or(RecoveryError::Capacity)?;
        let candidate = CoapProvisioner::start_with_suite(
            self.identity,
            self.peer.clone(),
            self.endpoint,
            local_id,
            suite,
            now_ms,
            |bytes| {
                for _ in 0..8 {
                    if !entropy(bytes) {
                        return false;
                    }
                    if bytes.len() != 22 {
                        return true;
                    }
                    let mid = MessageId::new(u16::from_be_bytes([bytes[0], bytes[1]]));
                    if !self.reserves_message_id(mid, self.endpoint, now_ms)
                        && !self.reserves_message_id(mid.wrapping_add(1), self.endpoint, now_ms)
                        && !self.reserves_message_id(mid.wrapping_add(2), self.endpoint, now_ms)
                        && !self.reserves_message_id(mid.wrapping_add(3), self.endpoint, now_ms)
                    {
                        return true;
                    }
                }
                false
            },
        )
        .map_err(RecoveryError::Bootstrap)?;
        self.candidate = Some(candidate);
        self.candidate_exposed = false;
        self.candidate_expires = Some(expires);
        Ok(())
    }

    /// Routes exact completion duplicates before the current candidate.
    /// A changed datagram with a still-owned Message ID is silently dropped.
    /// A fresh supported message 1 may create one responder candidate if space
    /// is available. It does not change the active App or authenticated identity.
    /// A terminal candidate failure is quarantined without evicting old caches.
    /// Unsupported critical Confirmable POSTs at the EDHOC resource queue 4.02
    /// before payload parsing or entropy. One refusal owner remains reserved
    /// across candidate termination and application handoff; it cannot renew its
    /// lifetime. A pending control must be flushed before admitting more input.
    pub fn ingest(
        &mut self,
        bytes: &[u8],
        endpoint: Endpoint,
        now_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
        mut authorize: impl FnMut(&Principal) -> bool,
    ) -> Result<Status, RecoveryError> {
        self.check_time(now_ms)?;
        if endpoint != self.endpoint {
            return Ok(Status::Ignored);
        }
        for cache in self.caches.iter_mut().flatten() {
            match cache.classify(bytes, now_ms) {
                CompletedCacheMatch::ExactDuplicate
                | CompletedCacheMatch::FreshErrorRequest
                | CompletedCacheMatch::RefusedMid => {
                    return Ok(if cache.queue(bytes, now_ms) {
                        Status::Progress
                    } else {
                        Status::Ignored
                    });
                }
                CompletedCacheMatch::MidCollision => return Ok(Status::Ignored),
                CompletedCacheMatch::NoMatch => {}
            }
        }
        if bytes.len() > DATAGRAM_CAPACITY {
            return Ok(Status::Ignored);
        }
        let Ok(message) = decode(bytes) else {
            return Ok(Status::Ignored);
        };
        if let Some(control) = &mut self.control {
            if control.pending {
                return Ok(Status::Ignored);
            }
            if control.owns(message, now_ms) {
                if !control.can_replay(message) {
                    return Ok(Status::Ignored);
                }
                control.pending = true;
                return Ok(Status::Progress);
            }
        }
        if let Some(candidate) = &mut self.candidate {
            if candidate.classify_completed_cache(bytes, endpoint, now_ms)
                == CompletedCacheMatch::MidCollision
            {
                return Ok(Status::Ignored);
            }
            let result = candidate.ingest(bytes, endpoint, now_ms, entropy, authorize);
            return self.finish_candidate_result(result);
        }
        if message.ty() == Type::Confirmable
            && message.code() == Code::POST
            && has_unsupported_critical(message, OptionRole::Request)
            && request_path(message)
        {
            return queue_control(
                &mut self.control,
                now_ms,
                ControlKind::BadOption(Operation {
                    mid: message.message_id(),
                    token: message.token(),
                }),
            )
            .map(|queued| {
                if queued {
                    Status::Progress
                } else {
                    Status::Ignored
                }
            })
            .map_err(|failure| RecoveryError::Bootstrap(failure.error()));
        }
        if self.handoff.is_some() {
            return Ok(Status::Ignored);
        }
        let payload = message.payload();
        if !request_metadata(message) || payload.first() != Some(&0xf5) {
            return Ok(Status::Ignored);
        }
        let Ok(message_1) = super::super::Message::from_slice(&payload[1..]) else {
            return Ok(Status::Ignored);
        };
        let Some(peer_id) = super::super::message_1_peer_id(&message_1)
            .ok()
            .or_else(|| super::super::wrong_selected_suite_peer_id(&message_1))
        else {
            return Ok(Status::Ignored);
        };
        let Some(local_id) = self.available_id(Some(peer_id)) else {
            return Ok(Status::Ignored);
        };
        let expires = now_ms
            .checked_add(EXCHANGE_LIFETIME_MS)
            .ok_or(RecoveryError::Bootstrap(PollError::Clock))?;
        let mut candidate = CoapProvisioner::listen_with_id(
            self.identity,
            self.peer.clone(),
            self.endpoint,
            local_id,
            now_ms,
        );
        let result = candidate.ingest(bytes, endpoint, now_ms, &mut entropy, &mut authorize);
        self.candidate = Some(candidate);
        self.candidate_exposed = false;
        self.candidate_expires = Some(expires);
        self.finish_candidate_result(result)
    }

    /// Sends pending completion bytes and the candidate's due work, without recv.
    /// Failed or short sends retain the exact pending datagram for retry.
    /// An expired completed candidate is removed and returns [`Status::Expired`].
    pub fn flush<T: DatagramIo>(
        &mut self,
        io: &mut T,
        now_ms: u64,
    ) -> Result<Status, RecoveryError<T::Error>> {
        self.check_time(now_ms).map_err(widen)?;
        let mut progress = false;
        progress |= flush_control(&mut self.control, io, self.endpoint, now_ms)
            .map_err(RecoveryError::Bootstrap)?;
        for cache in self.caches.iter_mut().flatten() {
            progress |= cache
                .flush(io, self.endpoint, now_ms)
                .map_err(RecoveryError::Bootstrap)?;
        }
        let Some(candidate) = &mut self.candidate else {
            return Ok(if progress {
                Status::Progress
            } else {
                Status::Idle
            });
        };
        if candidate
            .next_deadline()
            .is_some_and(|deadline| deadline <= now_ms)
            && (candidate.pending()
                || matches!(&candidate.role, Role::Client(client) if matches!(client.state, ClientState::Message2(_) | ClientState::Message4(_)) && client.retry.due(now_ms)))
        {
            self.candidate_exposed = true;
        }
        match candidate.flush(io, now_ms) {
            Ok(Status::Expired) => {
                self.retire_candidate(now_ms);
                Ok(Status::Expired)
            }
            Ok(Status::Complete) => Ok(Status::Complete),
            Ok(Status::Progress) => Ok(Status::Progress),
            Ok(_) => Ok(if progress {
                Status::Progress
            } else {
                Status::Idle
            }),
            Err(error) => {
                if terminal(&error) {
                    self.retire_candidate(now_ms);
                }
                Err(RecoveryError::Bootstrap(error))
            }
        }
    }

    /// Moves a fully authenticated session out after every pending send succeeds.
    /// Both old and new application IDs remain reserved until handoff confirmation.
    /// The compact completion cache retains no EDHOC handshake crypto state.
    pub fn take_session(&mut self) -> Option<Session> {
        if self.handoff.is_some()
            || self.control.as_ref().is_some_and(|control| control.pending)
            || self
                .caches
                .iter()
                .flatten()
                .any(|cache| cache.pending() || cache.error_in_flight(self.last_now))
        {
            return None;
        }
        let slot = self.caches.iter().position(Option::is_none)?;
        let candidate = self.candidate.as_mut()?;
        let session = candidate.take_session()?;
        let candidate = self.candidate.take().expect("completed candidate");
        let local_id = candidate.local_id;
        let expires = candidate.expires.expect("completed exchange lifetime");
        self.caches[slot] = Some(Cache::from_controller(candidate, expires, true));
        self.handoff = Some(local_id);
        self.candidate_exposed = false;
        self.candidate_expires = None;
        Some(session)
    }

    /// Confirms that the caller atomically replaced its old App with the new one.
    /// Old completion caches keep their original deadlines after confirmation.
    pub fn confirm_handoff(&mut self) -> bool {
        let Some(local_id) = self.handoff.take() else {
            return false;
        };
        self.active = Some(local_id);
        true
    }

    /// Abandons an uninstalled session while retaining the caller's old active ID.
    /// Use only if the returned session was never installed into a live App.
    /// Its already transmitted completion replies remain cached until expiry.
    pub fn abandon_handoff(&mut self, now_ms: u64) -> Result<bool, RecoveryError> {
        self.check_time(now_ms)?;
        Ok(self.handoff.take().is_some())
    }

    /// Cancels a candidate without changing the live App or evicting any cache.
    /// An attempt exposed to the transport retains its ID and duplicate metadata
    /// until its original lifetime; a never-transmitted attempt needs no quarantine.
    /// A recognized peer abort retains ACK/diagnostic ownership even if cancelled
    /// before its first successful ACK send; flush continues that bounded work.
    pub fn discard_candidate(&mut self, now_ms: u64) -> Result<bool, RecoveryError> {
        self.check_time(now_ms)?;
        let present = self.candidate.is_some();
        self.retire_candidate(now_ms);
        Ok(present)
    }

    /// The local connection ID of the caller's last confirmed application session.
    #[must_use]
    pub const fn active_connection_id(&self) -> Option<ConnectionId> {
        self.active
    }

    /// Whether bootstrap work is in flight, including retained error requests
    /// awaiting acknowledgement. Outgoing App CONs must remain quiescent.
    #[must_use]
    pub const fn has_candidate(&self) -> bool {
        if self.candidate.is_some() {
            return true;
        }
        let mut index = 0;
        while index < CACHES {
            if let Some(cache) = &self.caches[index]
                && cache.error_in_flight(self.last_now)
            {
                return true;
            }
            index += 1;
        }
        false
    }

    /// Whether a returned session still awaits an application handoff decision.
    #[must_use]
    pub const fn awaiting_handoff(&self) -> bool {
        self.handoff.is_some()
    }

    /// The current candidate's error or most recently retained peer abort.
    /// This unauthenticated diagnostic survives candidate retirement until its
    /// cache expires. It does not replace or revoke the caller's active App.
    #[must_use]
    pub fn peer_error(&self) -> Option<PeerError<'_>> {
        self.candidate
            .as_ref()
            .and_then(CoapProvisioner::peer_error)
            .or_else(|| {
                self.caches
                    .iter()
                    .flatten()
                    .filter(|cache| cache.peer_error().is_some())
                    .max_by_key(|cache| cache.expires)
                    .and_then(Cache::peer_error)
            })
    }

    /// The earliest exact send, candidate wait, or completion-cache deadline.
    /// Expiration never releases the caller's confirmed active connection ID.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        let mut next = self
            .candidate
            .as_ref()
            .and_then(CoapProvisioner::next_deadline);
        if let Some(control) = &self.control {
            let deadline = if control.pending {
                self.last_now
            } else {
                control.expires
            };
            next = Some(next.map_or(deadline, |previous| previous.min(deadline)));
        }
        for cache in self.caches.iter().flatten() {
            let deadline = cache.next_deadline(self.last_now);
            next = Some(next.map_or(deadline, |previous| previous.min(deadline)));
        }
        next
    }

    /// Whether bootstrap owns a locally generated Message ID at this endpoint.
    /// Application ACKs may echo peer-owned request IDs; new App requests must
    /// avoid these locally generated IDs while their reservation remains live.
    #[must_use]
    pub fn reserves_message_id(&self, mid: MessageId, endpoint: Endpoint, now_ms: u64) -> bool {
        if endpoint != self.endpoint {
            return false;
        }
        if let Some(candidate) = &self.candidate
            && let Role::Client(client) = &candidate.role
            && [
                client.ids.first.mid,
                client.ids.second.mid,
                client.ids.echo.mid,
                client.ids.error.mid,
            ]
            .contains(&mid)
        {
            return true;
        }
        self.caches.iter().flatten().any(|cache| {
            now_ms < cache.expires
                && matches!(&cache.role, CachedRole::Client { operations, .. } if operations.iter().any(|operation| operation.mid == mid))
        })
    }

    pub(in crate::provisioning) fn owns_error_response(
        &self,
        bytes: &[u8],
        endpoint: Endpoint,
        now: u64,
    ) -> bool {
        if endpoint != self.endpoint || bytes.len() > DATAGRAM_CAPACITY {
            return false;
        }
        let Ok(parsed) = decode(bytes) else {
            return false;
        };
        if let Some(candidate) = &self.candidate
            && let Role::Client(client) = &candidate.role
            && let ClientState::LocalAbort { request, .. } = &client.state
            && request.classify(bytes, parsed, now) != CompletedCacheMatch::NoMatch
        {
            return true;
        }
        self.caches.iter().flatten().any(|cache| {
            now < cache.expires
                && matches!(&cache.role, CachedRole::Client { abort: Some(request), .. }
                if request.classify(bytes, parsed, now) != CompletedCacheMatch::NoMatch)
        })
    }

    fn available_id(&self, peer_id: Option<ConnectionId>) -> Option<ConnectionId> {
        if !self.caches.iter().any(Option::is_none) {
            return None;
        }
        (0..24).find_map(|raw| {
            let id = ConnectionId::new(raw).expect("compact connection identifier");
            (Some(id) != self.active
                && Some(id) != self.handoff
                && Some(id) != peer_id
                && !self
                    .caches
                    .iter()
                    .flatten()
                    .any(|cache| cache.local_id == id))
            .then_some(id)
        })
    }

    fn check_time(&mut self, now: u64) -> Result<(), RecoveryError> {
        if now < self.last_now {
            self.retire_candidate(self.last_now);
            return Err(RecoveryError::Bootstrap(PollError::Clock));
        }
        self.last_now = now;
        if self
            .control
            .as_ref()
            .is_some_and(|control| now >= control.expires)
        {
            self.control = None;
        }
        for cache in &mut self.caches {
            if cache.as_ref().is_some_and(|cache| now >= cache.expires) {
                *cache = None;
            }
        }
        Ok(())
    }

    fn finish_candidate_result(
        &mut self,
        result: Result<Status, PollError>,
    ) -> Result<Status, RecoveryError> {
        if matches!(&result, Ok(Status::Expired)) || result.as_ref().is_err_and(terminal) {
            self.retire_candidate(self.last_now);
        }
        result.map_err(RecoveryError::Bootstrap)
    }

    fn retire_candidate(&mut self, now: u64) {
        let Some(candidate) = self.candidate.take() else {
            return;
        };
        let mut expires = self
            .candidate_expires
            .take()
            .expect("candidate reservation");
        if let Some(deadline) = candidate.expires {
            expires = expires.max(deadline);
        }
        if let Some(control) = &candidate.control {
            expires = expires.max(control.expires);
        }
        match &candidate.role {
            Role::Client(client) => {
                for ack in client.acks.iter().flatten() {
                    expires = expires.max(ack.expires);
                }
            }
            Role::Server(server) => {
                for exchange in server.exchanges.iter().flatten() {
                    expires = expires.max(exchange.expires);
                }
            }
        }
        let aborted = candidate.peer_error().is_some() || candidate.local_abort();
        if (self.candidate_exposed || aborted) && now < expires {
            let slot = self
                .caches
                .iter()
                .position(Option::is_none)
                .expect("reserved cache slot");
            self.caches[slot] = Some(Cache::from_controller(candidate, expires, aborted));
        }
        self.candidate_exposed = false;
    }
}

fn terminal<E>(error: &PollError<E>) -> bool {
    !matches!(error, PollError::Io(_) | PollError::ShortSend)
}

fn widen<E>(error: RecoveryError) -> RecoveryError<E> {
    match error {
        RecoveryError::Capacity => RecoveryError::Capacity,
        RecoveryError::Busy => RecoveryError::Busy,
        RecoveryError::Bootstrap(error) => RecoveryError::Bootstrap(super::widen(error)),
    }
}

impl<const CACHES: usize> fmt::Debug for CoapRecovery<'_, CACHES> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CoapRecovery")
            .field("endpoint", &self.endpoint)
            .field("active_connection_id", &self.active)
            .field("candidate", &self.has_candidate())
            .field("awaiting_handoff", &self.awaiting_handoff())
            .field("deadline", &self.next_deadline())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
