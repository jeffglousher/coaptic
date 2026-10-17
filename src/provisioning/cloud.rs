//! Bounded admission on one caller-owned shared UDP listener.
//!
//! Receive once, route the EDHOC resource to [`CloudAdmission::ingest`], then
//! spend one send opportunity on [`CloudAdmission::flush`]. Application traffic
//! belongs to the selected managed owner. Address validation, routing and
//! credential lookup confer no identity; message 3 must prove possession before
//! a current [`TrustGrant`] can admit a session. Resource permissions remain a
//! separate check against that authenticated principal.
//!
//! This profile accepts Confirmable sequential EDHOC requests and piggybacked
//! replies, requires message 4, and has no combined EDHOC/OSCORE flow. Each exact
//! endpoint has at most one provisional handshake. Fixed global and per-address
//! windows limit admission work; a keyed, expiring Echo proves reachability
//! before ephemeral-key work. These limits do not guarantee service against a
//! distributed flood. The caller must refresh authoritative policy replicas.
//! Routable failures retain bounded EDHOC error responses. Peer error requests
//! retain payload-free acknowledgements without changing an installed association.

use core::{cell::Cell, convert::Infallible, fmt};

use subtle::ConstantTimeEq;

use crate::message::{
    Code, Message as CoapMessage, MessageId, Opt, OptionNumber, ParsedMessage, Type, decode,
    decode_uint16,
};
use crate::oscore::OscoreHeader;
use crate::storage::{DatagramIo, Endpoint};

use super::coap::{CompletedCacheMatch, ErrorReceipt};
use super::{
    ConnectionId, CredentialReference, Error, Identity, Message, MessageFailure, PeerError,
    PeerErrorCode, PeerTrust, PollError, RegistryResponder, Session, Status, TrustGrant,
    message_1_peer_id,
};

const DATAGRAM_CAPACITY: usize = 384;
const EXCHANGE_LIFETIME_MS: u64 = 247_000;
const ECHO_LEN: usize = 24;
const ECHO_DOMAIN: &[u8] = b"coaptic cloud EDHOC reachability v1";
const MAX_COUNTER: u64 = 0x00ff_ffff_ffff_ffff;

/// Trusted lookup of one unambiguous installed credential policy.
///
/// References are attacker-controlled hints. Return `None` for missing or
/// ambiguous references, and bound database work independently. A matching
/// reference alone never grants access. Mutations and admission must share an
/// exclusive event loop or management lock; refresh external policy changes.
pub trait AdmissionRegistry {
    /// Resolves public lookup metadata to current, authenticated commissioning state.
    fn resolve(&self, reference: CredentialReference) -> Option<&PeerTrust>;
}

/// Fixed time-window work limits, separate from storage capacities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionLimits {
    /// Duration of each global and per-source work window.
    pub window_ms: u64,
    /// Maximum EDHOC datagrams considered per global window.
    pub global_datagrams: u16,
    /// Maximum EDHOC datagrams considered per source-address window.
    pub source_datagrams: u16,
    /// Maximum expensive M1/M3 processing steps per global window.
    pub global_crypto: u16,
    /// Maximum expensive M1/M3 processing steps per source-address window.
    pub source_crypto: u16,
    /// Validity of an address-bound Echo value; duplicates retain original expiry.
    pub echo_lifetime_ms: u64,
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            window_ms: 1_000,
            global_datagrams: 64,
            source_datagrams: 8,
            global_crypto: 8,
            source_crypto: 2,
            echo_lifetime_ms: 10_000,
        }
    }
}

/// Admission or one complete cached datagram send failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudError<E = Infallible> {
    /// A storage capacity or configured work limit is invalid.
    Capacity,
    /// Startup entropy or a cryptographic transition failed.
    Provisioning(Error),
    /// A routed unauthenticated peer error was acknowledged; installed Apps remain live.
    PeerAbort {
        /// Exact endpoint of the acknowledged error request.
        endpoint: Endpoint,
        /// Responder identifier of the aborted bootstrap attempt.
        local_id: ConnectionId,
        /// Diagnostic classification; it grants no authority or identity.
        code: PeerErrorCode,
    },
    /// A monotonic clock moved backward or a required deadline overflowed.
    Clock,
    /// The listener exhausted its seven-byte identifier namespace.
    Identifiers,
    /// A caller-owned datagram transport failed; exact bytes remain pending.
    Io(E),
    /// The transport reported a partial datagram send; exact bytes remain pending.
    ShortSend,
}

/// Reservation of one authenticated association on this listener.
///
/// Keep it live until its application owner is quiesced, then release it through
/// [`CloudAdmission::release`]. Routing this metadata does not authenticate an
/// application packet; the selected owner must verify OSCORE and permissions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Association {
    local_id: ConnectionId,
    peer_id: ConnectionId,
    endpoint: Endpoint,
    reference: CredentialReference,
    grant: TrustGrant,
}

impl Association {
    /// Local OSCORE recipient identifier reserved by the shared listener.
    #[must_use]
    pub const fn local_connection_id(&self) -> ConnectionId {
        self.local_id
    }

    /// Authenticated peer-selected identifier used by the local OSCORE sender.
    #[must_use]
    pub const fn peer_connection_id(&self) -> ConnectionId {
        self.peer_id
    }

    /// Exact route admitted by this handshake; address migration needs fresh recovery.
    #[must_use]
    pub const fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    /// Current-generation authorization captured after cryptographic verification.
    #[must_use]
    pub const fn grant(&self) -> &TrustGrant {
        &self.grant
    }
}

/// Fresh keys, current authorization and a live cloud routing reservation.
pub struct AdmittedSession {
    session: Session,
    association: Association,
}

impl AdmittedSession {
    /// Inspect authenticated metadata before moving keys into a new managed owner.
    #[must_use]
    pub const fn association(&self) -> &Association {
        &self.association
    }

    /// Moves the fresh session and reservation to its application owner.
    /// Revalidate the grant during installation and before every later I/O action.
    pub fn into_parts(self) -> (Session, Association) {
        (self.session, self.association)
    }
}

/// Session handoff refusal leaves the staged session and App unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppInstallError {
    /// No completed session was staged.
    Missing,
    /// Captured authorization or local identity is no longer current.
    Unauthorized,
    /// App refused capacity or recipient identity before moving any keys.
    App(crate::app::OscoreInstallError),
}

/// Failure while sharing one transport between admission and the application.
#[derive(Debug)]
pub enum SharedAppError<E> {
    /// Admission, trust maintenance or its bounded reply failed.
    Admission(CloudError<E>),
    /// Application protocol progress or checkpointing failed.
    Application(crate::app::Error<E>),
}

impl AdmittedSession {
    /// Move a fresh session into shared App storage after current trust revalidation.
    /// `epoch` is the caller's durable key-epoch identity, not a trust generation.
    /// Refusal preserves the candidate; successful installation consumes it once.
    pub fn install<
        P,
        T,
        const N: usize,
        const B: bool,
        S,
        const D: usize,
        const C: usize,
        const A: usize,
    >(
        candidate: &mut Option<Self>,
        app: &mut crate::App<P, T, N, B, S, D, C, 4, A>,
        registry: &impl AdmissionRegistry,
        epoch: u64,
    ) -> Result<crate::app::Association, AppInstallError>
    where
        P: crate::storage::MemoryLayout<B> + crate::app::AppAssembled<B>,
        S: crate::app::AppStorage,
    {
        let staged = candidate.as_ref().ok_or(AppInstallError::Missing)?;
        let metadata = staged.association;
        let trust = registry
            .resolve(metadata.reference)
            .ok_or(AppInstallError::Unauthorized)?;
        if !trust.permits(&metadata.grant, staged.session.principal())
            || staged.session.local_principal().fingerprint()
                != trust.checkpoint().local_principal()
        {
            return Err(AppInstallError::Unauthorized);
        }
        app.check_association_install(
            metadata.endpoint,
            *staged.session.principal().fingerprint(),
            epoch,
            &staged.session.context,
        )
        .map_err(AppInstallError::App)?;
        let staged = candidate.take().expect("checked staged session");
        let (context, principal) = staged.session.into_parts();
        let installed = app
            .install_association(
                metadata.endpoint,
                *principal.fingerprint(),
                epoch,
                &mut Some(context),
            )
            .expect("exclusive preflight established capacity and identity");
        Ok(installed)
    }
}

struct Wire {
    bytes: [u8; DATAGRAM_CAPACITY],
    len: usize,
}

impl Wire {
    fn copy(bytes: &[u8]) -> Result<Self, CloudError> {
        if bytes.len() > DATAGRAM_CAPACITY {
            return Err(CloudError::Provisioning(Error::Parsing));
        }
        let mut wire = Self {
            bytes: [0; DATAGRAM_CAPACITY],
            len: bytes.len(),
        };
        wire.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(wire)
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

struct Exchange {
    request: Wire,
    response: Option<Wire>,
    mid: MessageId,
    expires: u64,
    queued: bool,
    notice: Option<Notice>,
}

#[derive(Clone, Copy)]
enum Notice {
    Local(Error),
    Peer(PeerErrorCode),
}

impl Notice {
    fn error<E>(self, endpoint: Endpoint, local_id: ConnectionId) -> CloudError<E> {
        match self {
            Self::Local(error) => CloudError::Provisioning(error),
            Self::Peer(code) => CloudError::PeerAbort {
                endpoint,
                local_id,
                code,
            },
        }
    }
}

impl Exchange {
    fn peer_error(&self) -> Option<PeerError<'_>> {
        if !matches!(self.notice, Some(Notice::Peer(_))) {
            return None;
        }
        let parsed = decode(self.request.as_bytes()).ok()?;
        let (_, prefix) = ConnectionId::decode_prefix(parsed.payload()).ok()?;
        PeerError::parse(&parsed.payload()[prefix..])
    }

    fn classify(&mut self, bytes: &[u8], mid: MessageId, now: u64) -> Option<bool> {
        if now >= self.expires || mid != self.mid {
            return None;
        }
        let exact = self.request.as_bytes() == bytes;
        if exact && self.response.is_some() {
            self.queued = true;
        }
        Some(exact && self.response.is_some())
    }
}

struct Ready {
    session: Session,
    association: Association,
}

struct Pending {
    local_id: ConnectionId,
    peer_id: ConnectionId,
    endpoint: Endpoint,
    responder: Option<RegistryResponder>,
    ready: Option<Ready>,
    exchanges: [Option<Exchange>; 2],
    expires: u64,
    final_error: Option<ErrorReceipt>,
}

struct Retained {
    association: Association,
    exchanges: [Option<Exchange>; 2],
    expires: u64,
    final_error: Option<ErrorReceipt>,
    enabled: bool,
}

struct Challenge {
    endpoint: Endpoint,
    response: Wire,
    expires: u64,
}

#[derive(Clone, Copy)]
struct Source {
    address: Endpoint,
    datagrams: u16,
    crypto: u16,
}

/// Allocator-free cloud admission with independently bounded state and work.
///
/// The default profile reserves 64 active associations, four provisional
/// handshakes, eight retained completions and 16 source-address budget entries.
/// Provisional states also reserve active capacity. Completion-cache saturation
/// delays handoff and refuses replacement rather than evicting live entries.
/// Application Apps, queues, credentials, database and cryptographic stack are
/// outside these arrays and require their own complete resource accounting.
/// Never run another receive owner on the same listener.
pub struct CloudAdmission<
    'a,
    const ACTIVE: usize = 64,
    const PENDING: usize = 4,
    const RETAINED: usize = 8,
    const SOURCES: usize = 16,
> {
    identity: &'a Identity,
    active: [Option<Association>; ACTIVE],
    pending: [Option<Pending>; PENDING],
    retained: [Option<Retained>; RETAINED],
    sources: [Option<Source>; SOURCES],
    challenge: Option<Challenge>,
    echo_key: [u8; 32],
    limits: AdmissionLimits,
    last_now: u64,
    window_start: u64,
    datagrams: u16,
    crypto: u16,
    next_id: u64,
    send_cursor: usize,
    blocked: bool,
}

impl<'a, const A: usize, const P: usize, const C: usize, const S: usize>
    CloudAdmission<'a, A, P, C, S>
{
    /// Creates a listener with a fresh secret Echo key from cryptographic entropy.
    /// Every successful provider call must fill all bytes with fresh entropy.
    /// Invalid capacities, limits and failed entropy refuse all admission.
    pub fn new(
        identity: &'a Identity,
        now_ms: u64,
        limits: AdmissionLimits,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<Self, CloudError> {
        if A == 0
            || P == 0
            || C == 0
            || S == 0
            || limits.window_ms == 0
            || limits.global_datagrams == 0
            || limits.source_datagrams == 0
            || limits.global_crypto == 0
            || limits.source_crypto == 0
            || limits.source_datagrams > limits.global_datagrams
            || limits.source_crypto > limits.global_crypto
            || limits.echo_lifetime_ms == 0
            || limits.echo_lifetime_ms > EXCHANGE_LIFETIME_MS
        {
            return Err(CloudError::Capacity);
        }
        if now_ms.checked_add(EXCHANGE_LIFETIME_MS).is_none() {
            return Err(CloudError::Clock);
        }
        let mut echo_key = [0; 32];
        if !entropy(&mut echo_key) || echo_key == [0; 32] {
            echo_key.fill(0);
            return Err(CloudError::Provisioning(Error::Entropy));
        }
        Ok(Self {
            identity,
            active: core::array::from_fn(|_| None),
            pending: core::array::from_fn(|_| None),
            retained: core::array::from_fn(|_| None),
            sources: [None; S],
            challenge: None,
            echo_key,
            limits,
            last_now: now_ms,
            window_start: now_ms,
            datagrams: 0,
            crypto: 0,
            next_id: 1,
            send_cursor: 0,
            blocked: false,
        })
    }

    /// Processes one complete EDHOC datagram without receiving or sending.
    /// Exact endpoint/MID/token/payload duplicates queue exact saved replies.
    /// Changed MID-owned requests, unsupported metadata and exhausted admission
    /// are ignored without replacing live state. A failed M3 and its exact
    /// refusal are quarantined without repeating credential or cryptographic work.
    /// Local failures and peer aborts return Progress here; flush sends their
    /// cached response before reporting the original error or peer diagnostic.
    pub fn ingest(
        &mut self,
        bytes: &[u8],
        endpoint: Endpoint,
        now_ms: u64,
        registry: &impl AdmissionRegistry,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<Status, CloudError> {
        self.maintain(now_ms, registry)?;
        if bytes.len() > DATAGRAM_CAPACITY {
            return Ok(Status::Ignored);
        }
        let Ok(parsed) = decode(bytes) else {
            return Ok(Status::Ignored);
        };
        let Some(echo) = metadata(parsed) else {
            return Ok(Status::Ignored);
        };
        let Some(source) = self.admit_datagram(endpoint) else {
            return Ok(Status::Ignored);
        };
        let reserved_mid = self.reserves_message_id(parsed.message_id(), endpoint, now_ms);
        for pending in self.pending.iter_mut().flatten() {
            if pending.endpoint != endpoint {
                continue;
            }
            for exchange in pending.exchanges.iter_mut().flatten() {
                if let Some(queued) = exchange.classify(bytes, parsed.message_id(), now_ms) {
                    return Ok(if queued {
                        Status::Progress
                    } else {
                        Status::Ignored
                    });
                }
            }
            if let Some(receipt) = &mut pending.final_error {
                if receipt.classify(bytes, now_ms) != CompletedCacheMatch::NoMatch {
                    return Ok(if receipt.queue(bytes, now_ms) {
                        Status::Progress
                    } else {
                        Status::Ignored
                    });
                }
            } else if !reserved_mid
                && pending.ready.is_some()
                && !pending
                    .exchanges
                    .iter()
                    .flatten()
                    .any(|exchange| exchange.queued)
                && ErrorReceipt::matches_new(bytes, pending.local_id)
            {
                let expires = now_ms
                    .checked_add(EXCHANGE_LIFETIME_MS)
                    .ok_or(CloudError::Clock)?;
                pending.ready = None;
                pending.final_error = Some(ErrorReceipt::new(bytes, expires));
                pending.expires = pending.expires.max(expires);
                return Ok(Status::Progress);
            }
        }
        for retained in self.retained.iter_mut().flatten() {
            if retained.association.endpoint != endpoint {
                continue;
            }
            for exchange in retained.exchanges.iter_mut().flatten() {
                if let Some(queued) = exchange.classify(bytes, parsed.message_id(), now_ms) {
                    return Ok(if queued {
                        Status::Progress
                    } else {
                        Status::Ignored
                    });
                }
            }
            if let Some(receipt) = &mut retained.final_error {
                if receipt.classify(bytes, now_ms) != CompletedCacheMatch::NoMatch {
                    return Ok(if retained.enabled && receipt.queue(bytes, now_ms) {
                        Status::Progress
                    } else {
                        Status::Ignored
                    });
                }
            } else if !reserved_mid
                && retained.enabled
                && ErrorReceipt::matches_new(bytes, retained.association.local_id)
            {
                let expires = now_ms
                    .checked_add(EXCHANGE_LIFETIME_MS)
                    .ok_or(CloudError::Clock)?;
                retained.final_error = Some(ErrorReceipt::new(bytes, expires));
                retained.expires = retained.expires.max(expires);
                return Ok(Status::Progress);
            }
        }
        let payload = parsed.payload();
        if payload.first() == Some(&0xf5) {
            let Ok(message) = Message::from_slice(&payload[1..]) else {
                return Ok(Status::Ignored);
            };
            let Some(peer_id) = routing_id(&message) else {
                return Ok(Status::Ignored);
            };
            if self
                .pending
                .iter()
                .flatten()
                .any(|item| item.endpoint == endpoint)
                || self.active.iter().flatten().count() + self.pending.iter().flatten().count() >= A
            {
                return Ok(Status::Ignored);
            }
            let Some(slot) = self.pending.iter().position(Option::is_none) else {
                return Ok(Status::Ignored);
            };
            if !self.valid_echo(echo, endpoint, now_ms) {
                if self.challenge.is_some() {
                    return Ok(Status::Ignored);
                }
                let expires = now_ms
                    .checked_add(self.limits.echo_lifetime_ms)
                    .ok_or(CloudError::Clock)?;
                let token = self.echo(endpoint, now_ms);
                self.challenge = Some(Challenge {
                    endpoint,
                    response: response(parsed, Code::UNAUTHORIZED, &[], Some(&token))?,
                    expires,
                });
                return Ok(Status::Progress);
            }
            let expires = now_ms
                .checked_add(EXCHANGE_LIFETIME_MS)
                .ok_or(CloudError::Clock)?;
            let wrong_suite = super::wrong_selected_suite_peer_id(&message).is_some();
            let invalid_profile = message_1_peer_id(&message).is_err();
            if !wrong_suite && !invalid_profile && !self.admit_crypto(source) {
                return Ok(Status::Ignored);
            }
            let local_id = self.allocate_id(peer_id)?;
            let result = if wrong_suite {
                Err(MessageFailure {
                    error: Error::Profile,
                    code: PeerErrorCode::WrongCipherSuite,
                })
            } else if invalid_profile {
                Err(Error::Profile.into())
            } else {
                RegistryResponder::receive_message_1(self.identity, local_id, &message, entropy)
                    .map_err(MessageFailure::from)
            };
            let (responder, reply) = match result {
                Ok((responder, message)) => {
                    (Some(responder), exchange(bytes, parsed, &message, expires)?)
                }
                Err(failure) => (None, error_exchange(bytes, parsed, expires, failure)?),
            };
            self.pending[slot] = Some(Pending {
                local_id,
                peer_id,
                endpoint,
                responder,
                ready: None,
                exchanges: [Some(reply), None],
                expires,
                final_error: None,
            });
            return Ok(Status::Progress);
        }
        let Ok((local_id, prefix_len)) = ConnectionId::decode_prefix(payload) else {
            return Ok(Status::Ignored);
        };
        let Some(slot) = self.pending.iter().position(|entry| {
            entry.as_ref().is_some_and(|item| {
                item.local_id == local_id && item.endpoint == endpoint && item.responder.is_some()
            })
        }) else {
            return Ok(Status::Ignored);
        };
        let body = &payload[prefix_len..];
        let peer_error = PeerError::parse(body);
        if peer_error.is_some_and(|error| error.code() == PeerErrorCode::WrongCipherSuite)
            || (peer_error.is_none() && body.first().is_none_or(|byte| byte >> 5 != 2))
        {
            return Ok(Status::Ignored);
        }
        let message = Message::from_slice(body);
        if peer_error.is_none() && message.is_ok() && !self.admit_crypto(source) {
            return Ok(Status::Ignored);
        }
        let expires = now_ms
            .checked_add(EXCHANGE_LIFETIME_MS)
            .ok_or(CloudError::Clock)?;
        let pending = self.pending[slot]
            .as_mut()
            .expect("selected provisional slot");
        let responder = pending.responder.take().expect("selected message 3 state");
        if let Some(error) = peer_error {
            pending.exchanges[1] = Some(Exchange {
                request: Wire::copy(bytes)?,
                response: Some(response(parsed, Code::CHANGED, &[], None)?),
                mid: parsed.message_id(),
                expires,
                queued: true,
                notice: Some(Notice::Peer(error.code())),
            });
            pending.expires = pending.expires.max(expires);
            return Ok(Status::Progress);
        }
        pending.exchanges[1] = Some(Exchange {
            request: Wire::copy(bytes)?,
            response: None,
            mid: parsed.message_id(),
            expires: pending.expires,
            queued: false,
            notice: None,
        });
        let reference = Cell::new(None);
        let granted = Cell::new(None);
        let result = message.map_err(MessageFailure::from).and_then(|message| {
            responder.receive_message_3_with_error(
                &message,
                |hint| {
                    reference.set(Some(hint));
                    registry
                        .resolve(hint)
                        .map(|trust| trust.checkpoint().peer().clone())
                },
                |principal| {
                    let Some(trust) = reference.get().and_then(|hint| registry.resolve(hint))
                    else {
                        return false;
                    };
                    let Ok(grant) = trust.grant(self.identity) else {
                        return false;
                    };
                    if !trust.permits(&grant, principal) {
                        return false;
                    }
                    granted.set(Some(grant));
                    true
                },
            )
        });
        let (session, message) = match result {
            Ok(result) => result,
            Err(failure) => {
                pending.exchanges[1] = Some(error_exchange(bytes, parsed, expires, failure)?);
                pending.expires = pending.expires.max(expires);
                return Ok(Status::Progress);
            }
        };
        let association = Association {
            local_id: pending.local_id,
            peer_id: pending.peer_id,
            endpoint,
            reference: reference.get().expect("authenticated credential reference"),
            grant: granted.get().expect("authenticated current grant"),
        };
        pending.ready = Some(Ready {
            session,
            association,
        });
        pending.exchanges[1] = Some(exchange(bytes, parsed, &message, expires)?);
        pending.expires = expires;
        Ok(Status::Complete)
    }

    /// Sends at most one saved datagram, without receiving, with round-robin fairness.
    /// A failed or short send preserves exact bytes and grants are rechecked first.
    /// Every attempted send advances the cursor, including transport failures.
    /// A continuously queued owner therefore receives an attempt within
    /// `PENDING + RETAINED + 1` calls; successful delivery still depends on I/O.
    pub fn flush<T: DatagramIo>(
        &mut self,
        io: &mut T,
        now_ms: u64,
        registry: &impl AdmissionRegistry,
    ) -> Result<Status, CloudError<T::Error>> {
        self.maintain(now_ms, registry).map_err(widen)?;
        let count = P + C + 1;
        for offset in 0..count {
            let index = (self.send_cursor + offset) % count;
            if index < P {
                if let Some(pending) = &mut self.pending[index] {
                    for exchange in pending.exchanges.iter_mut().flatten() {
                        let sent = send_exchange(io, pending.endpoint, exchange, now_ms);
                        if !matches!(sent, Ok(false)) {
                            self.send_cursor = (index + 1) % count;
                        }
                        if sent? {
                            return exchange.notice.map_or(Ok(Status::Progress), |notice| {
                                Err(notice.error(pending.endpoint, pending.local_id))
                            });
                        }
                    }
                    if let Some(receipt) = &mut pending.final_error {
                        let sent =
                            flush_receipt(io, pending.endpoint, pending.local_id, receipt, now_ms);
                        if !matches!(sent, Ok(false)) {
                            self.send_cursor = (index + 1) % count;
                            return sent.map(|_| Status::Progress);
                        }
                        sent?;
                    }
                }
            } else if index < P + C {
                if let Some(retained) = &mut self.retained[index - P] {
                    for exchange in retained.exchanges.iter_mut().flatten() {
                        let sent =
                            send_exchange(io, retained.association.endpoint, exchange, now_ms);
                        if !matches!(sent, Ok(false)) {
                            self.send_cursor = (index + 1) % count;
                        }
                        if sent? {
                            return exchange.notice.map_or(Ok(Status::Progress), |notice| {
                                Err(notice.error(
                                    retained.association.endpoint,
                                    retained.association.local_id,
                                ))
                            });
                        }
                    }
                    if let Some(receipt) = &mut retained.final_error {
                        let sent = flush_receipt(
                            io,
                            retained.association.endpoint,
                            retained.association.local_id,
                            receipt,
                            now_ms,
                        );
                        if !matches!(sent, Ok(false)) {
                            self.send_cursor = (index + 1) % count;
                            return sent.map(|_| Status::Progress);
                        }
                        sent?;
                    }
                }
            } else if let Some(challenge) = &self.challenge {
                self.send_cursor = (index + 1) % count;
                send(io, challenge.endpoint, challenge.response.as_bytes())?;
                self.challenge = None;
                return Ok(Status::Progress);
            }
        }
        Ok(Status::Idle)
    }

    /// Moves one session after complete M4 transmission and current trust checks.
    /// Saturated completion storage retains keys internally until space or expiry.
    /// Among ready sessions, the oldest admitted handshake wins. Monotonically
    /// allocated connection IDs provide this ordering without another queue;
    /// reuse of an earlier array slot cannot starve an older ready owner.
    pub fn take_session(
        &mut self,
        now_ms: u64,
        registry: &impl AdmissionRegistry,
    ) -> Result<Option<AdmittedSession>, CloudError> {
        self.maintain(now_ms, registry)?;
        let Some(cache_slot) = self.retained.iter().position(Option::is_none) else {
            return Ok(None);
        };
        let Some(active_slot) = self.active.iter().position(Option::is_none) else {
            return Ok(None);
        };
        let Some((pending_slot, _)) = self
            .pending
            .iter()
            .enumerate()
            .filter_map(|(slot, entry)| entry.as_ref().map(|item| (slot, item)))
            .filter(|(_, item)| {
                item.ready.is_some() && !item.exchanges.iter().flatten().any(|item| item.queued)
            })
            .min_by(|(_, a), (_, b)| a.local_id.as_bytes().cmp(b.local_id.as_bytes()))
        else {
            return Ok(None);
        };
        let mut pending = self.pending[pending_slot].take().expect("ready slot");
        let ready = pending.ready.take().expect("ready session");
        let Some(trust) = registry.resolve(ready.association.reference) else {
            disable(&mut pending.exchanges);
            self.pending[pending_slot] = Some(pending);
            return Err(CloudError::Provisioning(Error::Unauthorized));
        };
        let session = match trust.accept_session(&ready.association.grant, ready.session) {
            Ok(session) => session,
            Err(_) => {
                disable(&mut pending.exchanges);
                self.pending[pending_slot] = Some(pending);
                return Err(CloudError::Provisioning(Error::Unauthorized));
            }
        };
        self.active[active_slot] = Some(ready.association);
        self.retained[cache_slot] = Some(Retained {
            association: ready.association,
            exchanges: pending.exchanges,
            expires: pending.expires,
            final_error: pending.final_error,
            enabled: true,
        });
        Ok(Some(AdmittedSession {
            session,
            association: ready.association,
        }))
    }

    /// Selects an active owner from untrusted OSCORE routing bytes and exact endpoint.
    /// The returned candidate must still authenticate and authorize the packet.
    /// Duplicate OSCORE options, ID Contexts and missing request identifiers fail.
    /// The shared-listener profile also refuses retained bootstrap MIDs at the
    /// same endpoint until their original exchange lifetime expires.
    pub fn route_oscore(
        &mut self,
        bytes: &[u8],
        endpoint: Endpoint,
        now_ms: u64,
        registry: &impl AdmissionRegistry,
    ) -> Result<Option<Association>, CloudError> {
        self.maintain(now_ms, registry)?;
        let Ok(parsed) = decode(bytes) else {
            return Ok(None);
        };
        if !parsed.code().is_request()
            || self.reserves_message_id(parsed.message_id(), endpoint, now_ms)
        {
            return Ok(None);
        }
        let mut options = parsed.get_options(OptionNumber::OSCORE);
        let Some(option) = options.next() else {
            return Ok(None);
        };
        if options.next().is_some() {
            return Ok(None);
        }
        let Ok(header) = OscoreHeader::parse(option.value()) else {
            return Ok(None);
        };
        if header.kid_context.is_some() || header.piv.is_none() {
            return Ok(None);
        }
        let Some(kid) = header.kid else {
            return Ok(None);
        };
        Ok(self
            .active
            .iter()
            .flatten()
            .find(|association| {
                association.endpoint == endpoint
                    && association.local_id.as_bytes() == kid
                    && current(registry, association)
            })
            .copied())
    }

    /// Poll one shared UDP transport for bootstrap and authenticated application work.
    /// Admission borrows App's received slot; it does not allocate another ingress queue.
    /// Each call retains App's bounded progress and flushes at most one bootstrap reply.
    /// Use `take_session` and `AdmittedSession::install` to complete a ready handoff.
    /// The checkpoint callback must honor the same durable contract as App polling.
    pub fn poll_app<
        MP,
        T,
        const N: usize,
        const B: bool,
        ST,
        const D: usize,
        const CC: usize,
        const R: usize,
        const AA: usize,
    >(
        &mut self,
        app: &mut crate::App<MP, T, N, B, ST, D, CC, R, AA>,
        registry: &impl AdmissionRegistry,
        now_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
        checkpoint: impl FnMut(Option<crate::app::Association>, crate::oscore::ReplayCheckpoint) -> bool,
        dispatch: impl FnMut(
            crate::Request<'_>,
            Option<crate::app::DeferredReply>,
        ) -> crate::Response<'static>,
    ) -> Result<Status, SharedAppError<T::Error>>
    where
        MP: crate::storage::MemoryLayout<B> + crate::app::AppAssembled<B>,
        ST: crate::app::AppStorage,
        T: DatagramIo,
    {
        let maintenance = self.maintain(now_ms, registry);
        self.reconcile_app(app, registry);
        maintenance.map_err(|error| SharedAppError::Admission(widen(error)))?;
        let mut ingress_error = None;
        let progress = app.poll_with_association_ingress(
            now_ms,
            |bytes, endpoint| {
                let Ok(parsed) = decode(bytes) else {
                    return false;
                };
                if parsed.uri_path().eq([Ok(".well-known"), Ok("edhoc")]) {
                    if let Err(error) = self.ingest(bytes, endpoint, now_ms, registry, &mut entropy)
                    {
                        ingress_error = Some(error);
                    }
                    true
                } else {
                    parsed.code().is_request()
                        && self.reserves_message_id(parsed.message_id(), endpoint, now_ms)
                }
            },
            checkpoint,
            dispatch,
        );
        // A transient application I/O failure must not starve queued admission.
        let flushed = self.flush(app.transport_mut(), now_ms, registry);
        progress.map_err(SharedAppError::Application)?;
        if let Some(error) = ingress_error {
            return Err(SharedAppError::Admission(widen(error)));
        }
        flushed.map_err(SharedAppError::Admission)
    }

    /// Revalidate every installed association before an App operation.
    /// This App must contain only sessions admitted by this listener. Revoked
    /// owners lose queued work immediately; pinned storage retains a disabled row
    /// and is reclaimed on a later call. No packet or body buffer is duplicated.
    pub fn reconcile_app<
        MP,
        T,
        const N: usize,
        const B: bool,
        ST,
        const D: usize,
        const CC: usize,
        const R: usize,
        const AA: usize,
    >(
        &mut self,
        app: &mut crate::App<MP, T, N, B, ST, D, CC, R, AA>,
        registry: &impl AdmissionRegistry,
    ) -> usize
    where
        MP: crate::storage::MemoryLayout<B> + crate::app::AppAssembled<B>,
        ST: crate::app::AppStorage,
    {
        let mut removed = 0;
        for index in 0..app.association_capacity() {
            let Some(owner) = app.association_at(index) else {
                continue;
            };
            let reservation = app.association_context(owner).and_then(|context| {
                self.active
                    .iter()
                    .flatten()
                    .find(|entry| {
                        entry.endpoint == owner.endpoint()
                            && entry.local_id.as_bytes() == context.recipient_id()
                            && entry.grant.principal().fingerprint() == owner.owner()
                    })
                    .copied()
            });
            if !self.blocked
                && app.association_is_active(owner)
                && reservation
                    .as_ref()
                    .is_some_and(|entry| current(registry, entry))
            {
                continue;
            }
            if app.remove_association(owner).is_ok() {
                if let Some(reservation) = reservation {
                    self.release(reservation);
                }
                removed += 1;
            }
        }
        removed
    }

    /// Run one application action after synchronously revoking stale owners.
    /// Refresh the registry under the same event-loop/management lock first.
    pub fn with_app<
        MP,
        T,
        const N: usize,
        const B: bool,
        ST,
        const D: usize,
        const CC: usize,
        const R: usize,
        const AA: usize,
        O,
    >(
        &mut self,
        app: &mut crate::App<MP, T, N, B, ST, D, CC, R, AA>,
        registry: &impl AdmissionRegistry,
        action: impl FnOnce(&mut crate::App<MP, T, N, B, ST, D, CC, R, AA>) -> O,
    ) -> O
    where
        MP: crate::storage::MemoryLayout<B> + crate::app::AppAssembled<B>,
        ST: crate::app::AppStorage,
    {
        self.reconcile_app(app, registry);
        action(app)
    }

    /// Releases active capacity only after the corresponding application is quiesced.
    /// Retained completion replies and their CID/MID reservations remain until expiry.
    pub fn release(&mut self, association: Association) -> bool {
        let Some(slot) = self
            .active
            .iter()
            .position(|entry| *entry == Some(association))
        else {
            return false;
        };
        self.active[slot] = None;
        true
    }

    /// Whether an identifier is owned by any pending, active or retained state.
    #[must_use]
    pub fn reserves_connection_id(&self, id: ConnectionId) -> bool {
        self.active.iter().flatten().any(|item| item.local_id == id)
            || self
                .pending
                .iter()
                .flatten()
                .any(|item| item.local_id == id)
            || self
                .retained
                .iter()
                .flatten()
                .any(|item| item.association.local_id == id)
    }

    /// Live inbound CoAP MID reservation for an exact peer endpoint.
    #[must_use]
    pub fn reserves_message_id(&self, mid: MessageId, endpoint: Endpoint, now_ms: u64) -> bool {
        self.pending.iter().flatten().any(|item| {
            item.endpoint == endpoint
                && (item
                    .exchanges
                    .iter()
                    .flatten()
                    .any(|exchange| exchange.mid == mid && now_ms < exchange.expires)
                    || item
                        .final_error
                        .as_ref()
                        .is_some_and(|receipt| receipt.reserves_message_id(mid, now_ms)))
        }) || self.retained.iter().flatten().any(|item| {
            item.association.endpoint == endpoint
                && (item
                    .exchanges
                    .iter()
                    .flatten()
                    .any(|exchange| exchange.mid == mid && now_ms < exchange.expires)
                    || item
                        .final_error
                        .as_ref()
                        .is_some_and(|receipt| receipt.reserves_message_id(mid, now_ms)))
        })
    }

    /// Borrow a retained unauthenticated diagnostic for an exact bootstrap route.
    /// It cannot authorize enrollment, revoke an App or replace its credentials.
    #[must_use]
    pub fn peer_error(&self, local_id: ConnectionId, endpoint: Endpoint) -> Option<PeerError<'_>> {
        if self.blocked {
            return None;
        }
        self.pending
            .iter()
            .flatten()
            .find_map(|item| {
                if item.local_id != local_id || item.endpoint != endpoint {
                    return None;
                }
                item.final_error
                    .as_ref()
                    .map(ErrorReceipt::peer_error)
                    .or_else(|| {
                        item.exchanges
                            .iter()
                            .flatten()
                            .find_map(Exchange::peer_error)
                    })
            })
            .or_else(|| {
                self.retained.iter().flatten().find_map(|item| {
                    if item.association.local_id != local_id
                        || item.association.endpoint != endpoint
                    {
                        return None;
                    }
                    item.final_error.as_ref().map(ErrorReceipt::peer_error)
                })
            })
    }

    /// Earliest queued send, available session handoff or original state expiry.
    ///
    /// Uses the last accepted caller time; a value at/before the current time
    /// calls for `flush` and `take_session`. Handoff readiness is suppressed when
    /// active or completion storage is full, avoiding a capacity-induced busy loop.
    /// Trust is rechecked by those operations, not by this scheduling hint.
    /// Socket readiness, policy updates and caller-released capacity must wake the
    /// event loop separately. No clock, I/O, allocation or state mutation occurs;
    /// the scan is bounded by `ACTIVE + PENDING + RETAINED` rows.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        if self.blocked {
            return None;
        }
        let can_handoff =
            self.active.iter().any(Option::is_none) && self.retained.iter().any(Option::is_none);
        let mut deadline = self.challenge.as_ref().map(|_| self.last_now);
        for item in self.pending.iter().flatten() {
            let due = if item
                .exchanges
                .iter()
                .flatten()
                .any(|exchange| exchange.queued)
                || item.final_error.as_ref().is_some_and(ErrorReceipt::pending)
                || (can_handoff && item.ready.is_some())
            {
                self.last_now
            } else {
                item.expires
            };
            deadline = Some(deadline.map_or(due, |old| old.min(due)));
        }
        for item in self.retained.iter().flatten() {
            let due = if item
                .exchanges
                .iter()
                .flatten()
                .any(|exchange| exchange.queued)
                || item.final_error.as_ref().is_some_and(ErrorReceipt::pending)
            {
                self.last_now
            } else {
                item.expires
            };
            deadline = Some(deadline.map_or(due, |old| old.min(due)));
        }
        deadline
    }

    fn maintain(&mut self, now: u64, registry: &impl AdmissionRegistry) -> Result<(), CloudError> {
        if self.blocked || now < self.last_now || now.checked_add(EXCHANGE_LIFETIME_MS).is_none() {
            self.blocked = true;
            for item in self.pending.iter_mut().flatten() {
                item.responder = None;
                item.ready = None;
                disable(&mut item.exchanges);
                if let Some(receipt) = &mut item.final_error {
                    receipt.disable();
                }
            }
            for item in self.retained.iter_mut().flatten() {
                disable(&mut item.exchanges);
                item.enabled = false;
                if let Some(receipt) = &mut item.final_error {
                    receipt.disable();
                }
            }
            self.challenge = None;
            return Err(CloudError::Clock);
        }
        self.last_now = now;
        if now - self.window_start >= self.limits.window_ms {
            self.window_start = now;
            self.datagrams = 0;
            self.crypto = 0;
            self.sources.fill(None);
        }
        if self
            .challenge
            .as_ref()
            .is_some_and(|item| now >= item.expires)
        {
            self.challenge = None;
        }
        for slot in &mut self.pending {
            if slot.as_ref().is_some_and(|item| now >= item.expires) {
                *slot = None;
            } else if let Some(item) = slot {
                if item
                    .ready
                    .as_ref()
                    .is_some_and(|ready| !current(registry, &ready.association))
                {
                    item.ready = None;
                    disable(&mut item.exchanges);
                }
            }
        }
        for slot in &mut self.retained {
            if slot.as_ref().is_some_and(|item| now >= item.expires) {
                *slot = None;
            } else if let Some(item) = slot {
                if !current(registry, &item.association) {
                    disable(&mut item.exchanges);
                    item.enabled = false;
                    if let Some(receipt) = &mut item.final_error {
                        receipt.disable();
                    }
                }
            }
        }
        Ok(())
    }

    fn admit_datagram(&mut self, endpoint: Endpoint) -> Option<usize> {
        if self.datagrams >= self.limits.global_datagrams {
            return None;
        }
        let address = if let Some((bytes, _)) = endpoint.as_ipv4() {
            Endpoint::v4(bytes, 0)
        } else {
            Endpoint::v6_scoped(endpoint.as_ipv6().expect("IPv6").0, 0, endpoint.scope_id())
        };
        let slot = self
            .sources
            .iter()
            .position(|item| item.is_some_and(|item| item.address == address))
            .or_else(|| self.sources.iter().position(Option::is_none))?;
        let source = self.sources[slot].get_or_insert(Source {
            address,
            datagrams: 0,
            crypto: 0,
        });
        if source.datagrams >= self.limits.source_datagrams {
            return None;
        }
        source.datagrams += 1;
        self.datagrams += 1;
        Some(slot)
    }

    fn admit_crypto(&mut self, source: usize) -> bool {
        let entry = self.sources[source].as_mut().expect("admitted source");
        if self.crypto >= self.limits.global_crypto || entry.crypto >= self.limits.source_crypto {
            return false;
        }
        self.crypto += 1;
        entry.crypto += 1;
        true
    }

    fn allocate_id(&mut self, peer_id: ConnectionId) -> Result<ConnectionId, CloudError> {
        for _ in 0..2 {
            if self.next_id > MAX_COUNTER {
                return Err(CloudError::Identifiers);
            }
            let bytes = self.next_id.to_be_bytes();
            self.next_id += 1;
            let id = ConnectionId::from_slice(&bytes[1..]).expect("seven-byte cloud identifier");
            if id != peer_id && !self.reserves_connection_id(id) {
                return Ok(id);
            }
        }
        Err(CloudError::Identifiers)
    }

    fn echo(&self, endpoint: Endpoint, issued: u64) -> [u8; ECHO_LEN] {
        let mut hash = blake3::Hasher::new_keyed(&self.echo_key);
        hash.update(ECHO_DOMAIN);
        if let Some((address, port)) = endpoint.as_ipv4() {
            hash.update(&[4]);
            hash.update(&address);
            hash.update(&port.to_be_bytes());
        } else {
            let (address, port) = endpoint.as_ipv6().expect("IPv6");
            hash.update(&[6]);
            hash.update(&address);
            hash.update(&port.to_be_bytes());
            hash.update(&endpoint.scope_id().to_be_bytes());
        }
        hash.update(&issued.to_be_bytes());
        let mut token = [0; ECHO_LEN];
        token[..8].copy_from_slice(&issued.to_be_bytes());
        token[8..].copy_from_slice(&hash.finalize().as_bytes()[..16]);
        token
    }

    fn valid_echo(&self, echo: Option<&[u8]>, endpoint: Endpoint, now: u64) -> bool {
        let Some(value) = echo.filter(|value| value.len() == ECHO_LEN) else {
            return false;
        };
        let issued = u64::from_be_bytes(value[..8].try_into().expect("fixed Echo timestamp"));
        issued <= now
            && now - issued < self.limits.echo_lifetime_ms
            && bool::from(value.ct_eq(&self.echo(endpoint, issued)))
    }
}

fn metadata(message: ParsedMessage<'_>) -> Option<Option<&[u8]>> {
    if message.ty() != Type::Confirmable || message.code() != Code::POST {
        return None;
    }
    let mut options = message.options();
    if !matches!(options.next(), Some(option) if option.number() == OptionNumber::URI_PATH && option.value() == b".well-known")
        || !matches!(options.next(), Some(option) if option.number() == OptionNumber::URI_PATH && option.value() == b"edhoc")
        || !matches!(options.next(), Some(option) if option.number() == OptionNumber::CONTENT_FORMAT && decode_uint16(option.value()) == Ok(65))
    {
        return None;
    }
    match options.next() {
        None => Some(None),
        Some(option)
            if option.number() == OptionNumber::ECHO
                && (1..=40).contains(&option.value().len()) =>
        {
            if options.next().is_none() {
                Some(Some(option.value()))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn routing_id(message: &Message) -> Option<ConnectionId> {
    message_1_peer_id(message).ok().or_else(|| {
        let cid = super::lakers::parse_message_1_routing_id(&message.buffer().ok()?).ok()?;
        ConnectionId::from_lakers(cid).ok()
    })
}

fn current(registry: &impl AdmissionRegistry, association: &Association) -> bool {
    registry
        .resolve(association.reference)
        .is_some_and(|trust| trust.permits(&association.grant, association.grant.principal()))
}

fn disable(exchanges: &mut [Option<Exchange>; 2]) {
    for item in exchanges.iter_mut().flatten() {
        item.response = None;
        item.queued = false;
    }
}

fn exchange(
    bytes: &[u8],
    parsed: ParsedMessage<'_>,
    message: &Message,
    expires: u64,
) -> Result<Exchange, CloudError> {
    Ok(Exchange {
        request: Wire::copy(bytes)?,
        response: Some(response(parsed, Code::CHANGED, message.as_bytes(), None)?),
        mid: parsed.message_id(),
        expires,
        queued: true,
        notice: None,
    })
}

fn error_exchange(
    bytes: &[u8],
    parsed: ParsedMessage<'_>,
    expires: u64,
    failure: MessageFailure,
) -> Result<Exchange, CloudError> {
    Ok(Exchange {
        request: Wire::copy(bytes)?,
        response: Some(response(
            parsed,
            failure.response_code(),
            failure.payload(),
            None,
        )?),
        mid: parsed.message_id(),
        expires,
        queued: true,
        notice: Some(Notice::Local(failure.error)),
    })
}

fn flush_receipt<T: DatagramIo>(
    io: &mut T,
    endpoint: Endpoint,
    local_id: ConnectionId,
    receipt: &mut ErrorReceipt,
    now: u64,
) -> Result<bool, CloudError<T::Error>> {
    receipt
        .flush(io, endpoint, now)
        .map_err(|error| match error {
            PollError::Io(error) => CloudError::Io(error),
            PollError::ShortSend => CloudError::ShortSend,
            PollError::PeerAbort(code) => CloudError::PeerAbort {
                endpoint,
                local_id,
                code,
            },
            _ => unreachable!("error receipts only send an ACK and report its diagnostic"),
        })
}

fn response(
    parsed: ParsedMessage<'_>,
    code: Code,
    payload: &[u8],
    echo: Option<&[u8]>,
) -> Result<Wire, CloudError> {
    let options = if let Some(echo) = echo {
        [Opt::new(OptionNumber::ECHO, echo)]
    } else {
        [Opt::new(OptionNumber::CONTENT_FORMAT, &[64])]
    };
    let mut wire = Wire {
        bytes: [0; DATAGRAM_CAPACITY],
        len: 0,
    };
    wire.len = CoapMessage::new(Type::Acknowledgement, code, parsed.message_id())
        .with_token(parsed.token())
        .with_options(if payload.is_empty() && echo.is_none() {
            &[]
        } else {
            &options
        })
        .with_payload(payload)
        .encode(&mut wire.bytes)
        .map_err(|_| CloudError::Provisioning(Error::Parsing))?;
    Ok(wire)
}

fn send_exchange<T: DatagramIo>(
    io: &mut T,
    endpoint: Endpoint,
    exchange: &mut Exchange,
    now: u64,
) -> Result<bool, CloudError<T::Error>> {
    if !exchange.queued || now >= exchange.expires {
        exchange.queued = false;
        return Ok(false);
    }
    let Some(response) = &exchange.response else {
        exchange.queued = false;
        return Ok(false);
    };
    send(io, endpoint, response.as_bytes())?;
    exchange.queued = false;
    Ok(true)
}

fn send<T: DatagramIo>(
    io: &mut T,
    endpoint: Endpoint,
    bytes: &[u8],
) -> Result<(), CloudError<T::Error>> {
    let sent = io.send(endpoint, bytes).map_err(CloudError::Io)?;
    if sent != bytes.len() {
        return Err(CloudError::ShortSend);
    }
    Ok(())
}

fn widen<E>(error: CloudError) -> CloudError<E> {
    match error {
        CloudError::Capacity => CloudError::Capacity,
        CloudError::Provisioning(error) => CloudError::Provisioning(error),
        CloudError::PeerAbort {
            endpoint,
            local_id,
            code,
        } => CloudError::PeerAbort {
            endpoint,
            local_id,
            code,
        },
        CloudError::Clock => CloudError::Clock,
        CloudError::Identifiers => CloudError::Identifiers,
        CloudError::Io(error) => match error {},
        CloudError::ShortSend => CloudError::ShortSend,
    }
}

impl fmt::Debug for AdmittedSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedSession")
            .field("association", &self.association)
            .finish_non_exhaustive()
    }
}

impl<const A: usize, const P: usize, const C: usize, const S: usize> fmt::Debug
    for CloudAdmission<'_, A, P, C, S>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudAdmission")
            .field("active", &self.active.iter().flatten().count())
            .field("pending", &self.pending.iter().flatten().count())
            .field("retained", &self.retained.iter().flatten().count())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "cloud_tests.rs"]
mod tests;
