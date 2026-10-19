//! A bounded owner for one enrolled peer and one fixed endpoint.

use core::{convert::Infallible, fmt, marker::PhantomData};

use crate::app::{
    AppAssembled, AppStore, Call, CallFailure, DeferredReply, IntoPath, Method, MethodRouter,
    ObserveSource, RandomSource, Request, Response, ResponseBufferError,
};
use crate::error::BuildError;
use crate::message::{ContentFormat, MessageId, OptionNumber, Transmission, Type, decode};
use crate::storage::{
    AsyncDatagramIo, DatagramIo, DatagramTransport, DedupKey, Endpoint, MemoryLayout,
    MemoryProfile, SlotId,
};
use crate::{App, profiles};

use super::{
    CoapRecovery, Identity, PeerTrust, PinnedPeer, Principal, RecoveryError, TrustAnchor,
    TrustCommitError, TrustError, TrustGrant, TrustRecord,
};

#[path = "managed_async.rs"]
mod asynchronous;
#[path = "managed_subscription.rs"]
mod subscription;
pub use asynchronous::ManagedAsyncError;
pub use subscription::ManagedSubscription;

const DATAGRAM_BYTES: usize = 1472;
const MID_RANGES: usize = 8;
const BOOTSTRAP_MIDS: usize = 16;
const LIFETIME: u64 = Transmission::EXCHANGE_LIFETIME_MS as u64;

/// Transport or bounded routing failure inside a managed connection.
#[derive(Debug)]
pub enum ManagedIoError<E> {
    /// The caller's transport failed; delivery may be uncertain.
    Transport(E),
    /// A different live session or bootstrap exchange owns this Message ID.
    MessageIdReserved,
    /// The fixed Message-ID reservation or datagram capacity is exhausted.
    Capacity,
    /// The transport returned an invalid datagram length.
    DatagramLength,
}

impl<E: fmt::Display> fmt::Display for ManagedIoError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::MessageIdReserved => f.write_str("managed Message ID is reserved"),
            Self::Capacity => f.write_str("managed transport capacity exhausted"),
            Self::DatagramLength => f.write_str("invalid managed datagram length"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ManagedIoError<E> {}

/// Failure of a managed action, with trust, bootstrap and application errors distinct.
#[derive(Debug)]
pub enum ManagedError<E = Infallible> {
    /// Current trust does not authorize the captured connection grant.
    Trust(TrustError),
    /// EDHOC recovery or its transport failed.
    Recovery(RecoveryError<ManagedIoError<E>>),
    /// Protected application processing or its transport failed.
    Application(crate::app::Error<ManagedIoError<E>>),
    /// A fresh bounded App could not be constructed.
    Build(BuildError),
    /// A fresh authenticated session is required.
    NotConnected,
    /// Bootstrap must wait for application Confirmable work to quiesce.
    Busy,
    /// The supplied monotonic time moved backward or overflowed a reservation.
    Clock,
    /// A profile, route, or bounded reservation exceeds this owner's capacity.
    Capacity,
    /// An observable path exceeds App's path bounds.
    Path,
    /// Complete response collection needs different caller-owned storage.
    ResponseBuffer(ResponseBufferError),
    /// The operation belongs to a discarded connection or trust generation.
    StaleHandle,
    /// The online authority lease expired; fresh verified policy is required.
    PolicyExpired,
    /// The shared listener owns bootstrap for this externally admitted session.
    ExternalBootstrap,
}

impl<E: fmt::Display + fmt::Debug> fmt::Display for ManagedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trust(error) => error.fmt(f),
            Self::Recovery(error) => write!(f, "managed bootstrap failed: {error:?}"),
            Self::Application(error) => error.fmt(f),
            Self::Build(error) => error.fmt(f),
            Self::NotConnected => f.write_str("fresh authenticated connection required"),
            Self::Busy => f.write_str("managed Confirmable work is busy"),
            Self::Clock => f.write_str("invalid managed monotonic clock"),
            Self::Capacity => f.write_str("managed capacity exhausted"),
            Self::Path => f.write_str("invalid managed observable path"),
            Self::ResponseBuffer(error) => error.fmt(f),
            Self::StaleHandle => f.write_str("operation belongs to a discarded managed connection"),
            Self::PolicyExpired => f.write_str("managed online policy expired"),
            Self::ExternalBootstrap => f.write_str("shared listener owns managed bootstrap"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ManagedError<E> {}

/// Explicit freshness contract for this owner's installed trust policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyValidity {
    /// Durable local pairing under physical custody. Survives ordinary restart;
    /// it does not detect rollback of an entire flash/disk image.
    OfflineLocalStorage,
    /// The caller supplies a qualified protected monotonic trust anchor.
    OfflineProtectedAnchor,
    /// The freshly authenticated online policy expires at this monotonic time.
    /// Delayed acceptance must retain its authority exchange's original deadline.
    OnlineUntil {
        /// Monotonic time at which this exact fresh authority proof was accepted.
        accepted_ms: u64,
        /// Original deadline established before the authority challenge was sent.
        deadline_ms: u64,
    },
}

/// Failure to install freshly authenticated policy after closing prior traffic.
#[derive(Debug)]
pub enum ManagedRefreshError<E> {
    /// The authority or policy persistence callback failed.
    Authority(E),
    /// The restored policy does not bind this owner's local identity.
    Trust(TrustError),
    /// Monotonic time moved backward or overflowed a transport reservation.
    Clock,
    /// The restored online policy is already expired.
    PolicyExpired,
}

impl<E: fmt::Display> fmt::Display for ManagedRefreshError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority(error) => write!(f, "managed policy refresh failed: {error}"),
            Self::Trust(error) => error.fmt(f),
            Self::Clock => f.write_str("invalid managed policy refresh clock"),
            Self::PolicyExpired => f.write_str("refreshed managed policy is already expired"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ManagedRefreshError<E> {}

/// An outgoing operation bound to its authenticated connection and trust epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedCall<K: crate::message::OwnedToken = crate::message::Token> {
    call: Call<K>,
    anchor: TrustAnchor,
    epoch: u64,
}

impl<K: crate::message::OwnedToken> ManagedCall<K> {
    /// Public CoAP token for logging or correlation, never an authorization key.
    #[must_use]
    pub const fn token(self) -> K {
        self.call.token()
    }

    /// Fixed destination selected by the managed owner.
    #[must_use]
    pub const fn peer(self) -> Endpoint {
        self.call.peer()
    }
}

/// Deferred application work bound to the connection that accepted it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedDeferredReply {
    handle: DeferredReply,
    anchor: TrustAnchor,
    epoch: u64,
}

#[derive(Clone, Copy)]
struct MidRange {
    first: u16,
    count: u32,
    epoch: u64,
    expires: u64,
}

impl MidRange {
    fn contains(self, mid: MessageId, now: u64) -> bool {
        now < self.expires && u32::from(mid.get().wrapping_sub(self.first)) < self.count
    }
}

struct ConnectionIo<T> {
    raw: Option<T>,
    bytes: [u8; DATAGRAM_BYTES],
    queued: Option<(usize, Endpoint)>,
    now: u64,
    epoch: u64,
    ranges: [Option<MidRange>; MID_RANGES],
    bootstrap: [Option<(MessageId, u64)>; BOOTSTRAP_MIDS],
    bootstrap_mode: bool,
}

impl<T> ConnectionIo<T> {
    fn new(raw: Option<T>, now: u64) -> Self {
        Self {
            raw,
            bytes: [0; DATAGRAM_BYTES],
            queued: None,
            now,
            epoch: 0,
            ranges: [None; MID_RANGES],
            bootstrap: [None; BOOTSTRAP_MIDS],
            bootstrap_mode: false,
        }
    }

    fn prune(&mut self, now: u64) {
        self.now = now;
        for range in &mut self.ranges {
            if range.is_some_and(|range| now >= range.expires) {
                *range = None;
            }
        }
        for reservation in &mut self.bootstrap {
            if reservation.is_some_and(|(_, expires)| now >= expires) {
                *reservation = None;
            }
        }
    }

    fn reserved_by_app(&self, mid: MessageId) -> bool {
        self.ranges
            .iter()
            .flatten()
            .any(|range| range.contains(mid, self.now))
    }

    fn reserved(&self, mid: MessageId) -> bool {
        self.reserved_by_app(mid) || self.bootstrap.iter().flatten().any(|(old, _)| *old == mid)
    }

    fn next_epoch_available(&self) -> bool {
        self.epoch < u64::MAX && self.ranges.iter().any(Option::is_none)
    }
}

impl<T: DatagramIo> ConnectionIo<T> {
    fn receive(&mut self) -> Result<(), ManagedIoError<T::Error>> {
        if self.queued.is_some() {
            return Ok(());
        }
        if let Some((len, endpoint)) = self
            .raw
            .as_mut()
            .expect("managed transport owned")
            .recv(&mut self.bytes)
            .map_err(ManagedIoError::Transport)?
        {
            if len > self.bytes.len() {
                return Err(ManagedIoError::DatagramLength);
            }
            self.queued = Some((len, endpoint));
        }
        Ok(())
    }
}

impl<T: DatagramTransport> ConnectionIo<T> {
    fn reserve(&mut self, mid: MessageId) -> Result<(), ManagedIoError<T::Error>> {
        let expires = self
            .now
            .checked_add(LIFETIME)
            .ok_or(ManagedIoError::Capacity)?;
        if self.bootstrap_mode {
            if self.reserved_by_app(mid) {
                return Err(ManagedIoError::MessageIdReserved);
            }
            if self.bootstrap.iter().flatten().any(|(old, _)| *old == mid) {
                return Ok(());
            }
            let slot = self
                .bootstrap
                .iter_mut()
                .find(|row| row.is_none())
                .ok_or(ManagedIoError::Capacity)?;
            *slot = Some((mid, expires));
            return Ok(());
        }
        if self.bootstrap.iter().flatten().any(|(old, _)| *old == mid)
            || self
                .ranges
                .iter()
                .flatten()
                .any(|range| range.epoch != self.epoch && range.contains(mid, self.now))
        {
            return Err(ManagedIoError::MessageIdReserved);
        }
        if let Some(range) = self
            .ranges
            .iter_mut()
            .flatten()
            .find(|range| range.epoch == self.epoch)
        {
            range.count = range
                .count
                .max(u32::from(mid.get().wrapping_sub(range.first)) + 1);
            range.expires = expires;
        } else {
            let slot = self
                .ranges
                .iter_mut()
                .find(|row| row.is_none())
                .ok_or(ManagedIoError::Capacity)?;
            *slot = Some(MidRange {
                first: mid.get(),
                count: 1,
                epoch: self.epoch,
                expires,
            });
        }
        Ok(())
    }
}

impl<T: DatagramTransport> crate::storage::DatagramTransport for ConnectionIo<T> {
    type Error = ManagedIoError<T::Error>;
    fn fill_random(&mut self, bytes: &mut [u8]) -> bool {
        self.raw.as_mut().is_some_and(|raw| raw.fill_random(bytes))
    }
}

impl<T: DatagramIo> DatagramIo for ConnectionIo<T> {
    // Preserve optional platform entropy through this ownership decorator.

    fn recv(&mut self, output: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((len, endpoint)) = self.queued else {
            return Ok(None);
        };
        if len > output.len() {
            self.queued = None;
            return Err(ManagedIoError::DatagramLength);
        }
        output[..len].copy_from_slice(&self.bytes[..len]);
        self.queued = None;
        Ok(Some((len, endpoint)))
    }

    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if let Ok(message) = decode(bytes)
            && matches!(message.ty(), Type::Confirmable | Type::NonConfirmable)
        {
            self.reserve(message.message_id())?;
        }
        self.raw
            .as_mut()
            .expect("managed transport owned")
            .send(endpoint, bytes)
            .map_err(ManagedIoError::Transport)
    }
}

impl<T: AsyncDatagramIo> ConnectionIo<T> {
    async fn receive_async(&mut self) -> Result<(), ManagedIoError<T::Error>> {
        if self.queued.is_some() {
            return Ok(());
        }
        let (len, endpoint) = self
            .raw
            .as_mut()
            .expect("managed transport owned")
            .recv(&mut self.bytes)
            .await
            .map_err(ManagedIoError::Transport)?;
        if len > self.bytes.len() {
            return Err(ManagedIoError::DatagramLength);
        }
        self.queued = Some((len, endpoint));
        Ok(())
    }
}

impl<T: AsyncDatagramIo> AsyncDatagramIo for ConnectionIo<T> {
    async fn recv(&mut self, output: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        self.receive_async().await?;
        let (len, endpoint) = self.queued.take().expect("received managed packet");
        if len > output.len() {
            return Err(ManagedIoError::DatagramLength);
        }
        output[..len].copy_from_slice(&self.bytes[..len]);
        Ok((len, endpoint))
    }
    async fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if let Ok(message) = decode(bytes)
            && matches!(message.ty(), Type::Confirmable | Type::NonConfirmable)
        {
            self.reserve(message.message_id())?;
        }
        self.raw
            .as_mut()
            .expect("managed transport owned")
            .send(endpoint, bytes)
            .await
            .map_err(ManagedIoError::Transport)
    }
}

type SecureApp<P, T, const ROUTES: usize, const DEFERRED: usize> =
    App<P, ConnectionIo<T>, ROUTES, false, AppStore<P, false>, DEFERRED>;

#[allow(clippy::large_enum_variant)]
enum State<P, T, const ROUTES: usize, const DEFERRED: usize>
where
    P: MemoryProfile + MemoryLayout<false> + AppAssembled<false>,
{
    Waiting(ConnectionIo<T>),
    Established(SecureApp<P, T, ROUTES, DEFERRED>),
}

/// Ordinary one-peer EDHOC/OSCORE ownership with mandatory trust checks.
///
/// This owner borrows an immutable enrolled identity and exclusively owns trust,
/// bootstrap state, application state and the transport. Every action that can
/// produce traffic or effects revalidates its captured grant. Revocation,
/// replacement and authority refresh discard prior sessions, pending calls,
/// Observe registrations, deferred handles and queued datagrams. A new grant
/// authorizes only a fresh handshake. Independent authorities must call
/// [`Self::refresh`] after an external policy change; no storage is polled here.
///
/// Keys remain volatile. Never restore a prior session after restart. The random
/// source must provide fresh cryptographically secure bytes in bounded time.
/// Trust commits and refresh callbacks must authenticate management and meet
/// [`PeerTrust`]'s durability and anti-rollback contracts. Resource authorization
/// belongs in the principal-bearing dispatch/effect callbacks.
///
/// This first profile has one fixed endpoint and no block-wise transfer. A
/// request or response must fit its selected profile's datagram after CoAP and
/// OSCORE overhead. Complete response collection uses caller-owned storage and
/// preserves metadata. The receive queue has a fixed 1472-byte ceiling. Eight
/// local session MID ranges and sixteen bootstrap MIDs retain original reuse
/// reservations; bounded exhaustion or collisions fail before a socket send.
/// Owned recovery pauses App polling while a candidate is in flight and drops
/// unrelated application datagrams during that pause. This preserves NSTART;
/// application CON peers can retry after completion or candidate failure.
/// Externally admitted owners leave bootstrap exclusively to their listener.
///
/// Online policy expiration and clock rollback close all owned state before I/O.
/// A successful refresh always needs fresh authentication. Offline operation
/// selects either durable local custody through [`Self::from_local_pairing`] or
/// a qualified protected monotonic anchor through [`PolicyValidity::OfflineProtectedAnchor`].
/// Ordinary flash does not satisfy the protected anchor profile's physical
/// rollback contract.
///
/// ```compile_fail
/// use coaptic::provisioning::ManagedConnection;
/// use coaptic::storage::DatagramIo;
/// fn escape<T: DatagramIo>(connection: &mut ManagedConnection<'_, T>) {
///     connection.transport_mut();
/// }
/// ```
/// ```compile_fail
/// use coaptic::provisioning::ManagedConnection;
/// use coaptic::storage::DatagramIo;
/// fn replace_keys<T: DatagramIo>(connection: &mut ManagedConnection<'_, T>) {
///     connection.oscore_mut();
/// }
/// ```
pub struct ManagedConnection<
    'a,
    T,
    P = profiles::Constrained,
    const DEFERRED: usize = 0,
    const CACHES: usize = 2,
    const ROUTES: usize = 8,
> where
    P: MemoryProfile + MemoryLayout<false> + AppAssembled<false>,
{
    identity: &'a Identity,
    local_pairing: Option<&'a super::LocalPairing>,
    trust: PeerTrust,
    grant: Option<TrustGrant>,
    recovery: Option<CoapRecovery<'a, CACHES>>,
    state: Option<State<P, T, ROUTES, DEFERRED>>,
    endpoint: Endpoint,
    randomness: RandomSource,
    observed: [Option<ObservedRoute<P::Token>>; ROUTES],
    last_now: u64,
    refresh_blocked: bool,
    validity: PolicyValidity,
    effects: [Option<ManagedDeferredReply>; DEFERRED],
    external_bootstrap: bool,
    _profile: PhantomData<P>,
}

impl<'a, T: DatagramTransport, P, const DEFERRED: usize, const CACHES: usize, const ROUTES: usize>
    ManagedConnection<'a, T, P, DEFERRED, CACHES, ROUTES>
where
    P: MemoryProfile + MemoryLayout<false> + AppAssembled<false>,
{
    /// Own a normally paired device without requiring an online authority or
    /// protected monotonic hardware. The immutable borrow prevents management
    /// changes until this whole session/application owner has been discarded.
    /// Durable custody and resource permissions remain caller responsibilities.
    pub fn from_local_pairing(
        identity: &'a Identity,
        pairing: &'a super::LocalPairing,
        endpoint: Endpoint,
        io: T,
        randomness: RandomSource,
        now_ms: u64,
    ) -> Result<Self, ManagedError<T::Error>> {
        let trust =
            PeerTrust::from_local_pairing(identity, pairing).map_err(ManagedError::Trust)?;
        let mut connection = Self::new(
            identity,
            trust,
            endpoint,
            io,
            randomness,
            PolicyValidity::OfflineLocalStorage,
            now_ms,
        )?;
        connection.local_pairing = Some(pairing);
        Ok(connection)
    }

    /// Own an already enrolled trust policy and transport without starting I/O.
    pub fn new(
        identity: &'a Identity,
        trust: PeerTrust,
        endpoint: Endpoint,
        io: T,
        randomness: RandomSource,
        validity: PolicyValidity,
        now_ms: u64,
    ) -> Result<Self, ManagedError<T::Error>> {
        if CACHES == 0
            || CACHES > (BOOTSTRAP_MIDS - 3) / 3
            || P::RX_DATAGRAM_BYTES > DATAGRAM_BYTES
            || P::TX_DATAGRAM_BYTES > DATAGRAM_BYTES
        {
            return Err(ManagedError::Capacity);
        }
        trust.grant(identity).map_err(ManagedError::Trust)?;
        if matches!(validity, PolicyValidity::OnlineUntil { accepted_ms, .. } if now_ms < accepted_ms)
        {
            return Err(ManagedError::Clock);
        }
        if matches!(validity, PolicyValidity::OnlineUntil { deadline_ms, .. } if now_ms >= deadline_ms)
        {
            return Err(ManagedError::PolicyExpired);
        }
        if now_ms.checked_add(LIFETIME).is_none() {
            return Err(ManagedError::Clock);
        }
        Ok(Self {
            identity,
            local_pairing: None,
            trust,
            grant: None,
            recovery: None,
            state: Some(State::Waiting(ConnectionIo::new(Some(io), now_ms))),
            endpoint,
            randomness,
            observed: [None; ROUTES],
            last_now: now_ms,
            refresh_blocked: false,
            validity,
            effects: [None; DEFERRED],
            external_bootstrap: false,
            _profile: PhantomData,
        })
    }

    /// Own a session authenticated by a shared listener under its captured grant.
    /// The listener retains global connection-ID, completion-cache and receive
    /// routing ownership. This owner never starts or ingests its own bootstrap;
    /// policy refresh still requires the listener to admit a whole fresh owner.
    /// Its routed transport must isolate this association and enforce any shared
    /// endpoint-wide Message-ID/congestion reservations before socket sends.
    #[allow(clippy::too_many_arguments)]
    pub fn from_admitted_session(
        identity: &'a Identity,
        trust: PeerTrust,
        grant: TrustGrant,
        session: super::Session,
        endpoint: Endpoint,
        io: T,
        randomness: RandomSource,
        validity: PolicyValidity,
        now_ms: u64,
    ) -> Result<Self, ManagedError<T::Error>> {
        let session = trust
            .accept_session(&grant, session)
            .map_err(ManagedError::Trust)?;
        let mut owner = Self::new(identity, trust, endpoint, io, randomness, validity, now_ms)?;
        owner.grant = Some(grant);
        owner.external_bootstrap = true;
        owner.install_authenticated(session, now_ms)?;
        Ok(owner)
    }

    /// Read current committed policy metadata without exposing mutable trust.
    #[must_use]
    pub const fn trust(&self) -> &PeerTrust {
        &self.trust
    }

    /// Retained unauthenticated bootstrap diagnostics, without changing live
    /// trust or connection state. The caller chooses its logging policy.
    #[must_use]
    pub fn peer_error(&self) -> Option<super::PeerError<'_>> {
        self.recovery.as_ref().and_then(CoapRecovery::peer_error)
    }

    /// Current explicit authority freshness contract, evaluated on each action.
    #[must_use]
    pub const fn policy_validity(&self) -> PolicyValidity {
        self.validity
    }

    /// Whether authority refresh or ambiguous persistence blocks authorization.
    #[must_use]
    pub const fn is_blocked(&self) -> bool {
        self.refresh_blocked || self.trust.is_blocked()
    }

    /// Whether a live authenticated App is owned under the current grant.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        matches!(&self.state, Some(State::Established(_)))
            && self
                .grant
                .as_ref()
                .is_some_and(|grant| self.trust.permits(grant, grant.principal()))
            && !self.refresh_blocked
            && !matches!(self.validity, PolicyValidity::OnlineUntil { deadline_ms, .. } if self.last_now >= deadline_ms)
    }

    /// Opaque generation of the currently authorized authenticated session.
    ///
    /// Compare values only within this connection owner. A changed value marks
    /// an authenticated session installation; starting or rejecting a candidate
    /// leaves the retained session's value unchanged. This lets a caller resume
    /// local work after quiescing it for peer recovery. It is not a peer identity,
    /// a durable sequence or proof that the peer remains reachable.
    ///
    /// Returns `None` whenever [`Self::is_connected`] is false. Sampling does not
    /// advance time, refresh authority, perform I/O or allocate storage.
    #[must_use]
    pub fn session_generation(&self) -> Option<u64> {
        if !self.is_connected() {
            return None;
        }
        match &self.state {
            Some(State::Established(app)) => Some(app.transport().epoch),
            _ => None,
        }
    }

    /// Copy diagnostics from the retained authenticated App, if one exists.
    ///
    /// Sampling performs no I/O, allocation, clock advancement or policy change.
    /// A retained App can remain observable while this owner is blocked; `Some`
    /// does not grant permission to send or dispatch. Bootstrap has no App and
    /// returns `None`. The snapshot's readiness fields describe only the App;
    /// use [`Self::next_poll_at`] and [`Self::next_timeout`] to schedule this owner.
    pub fn diagnostic_snapshot(
        &mut self,
        now_ms: u64,
    ) -> Option<crate::app::diagnostics::Snapshot> {
        match self.state.as_mut() {
            Some(State::Established(app)) => Some(app.diagnostic_snapshot(now_ms)),
            _ => None,
        }
    }

    /// Earliest recovery, active application or online-policy deadline.
    ///
    /// Recovery can report an immediately pending send. Application timers are
    /// excluded while a recovery candidate pauses App polling. Blocked owners
    /// return `None`: only an explicit policy refresh can resume their work.
    /// Passive MID reservations are pruned on the next action, not by a separate
    /// wakeup. Caller-owned subscriptions and admission retries have their own
    /// scheduling obligations.
    ///
    /// No I/O, clock advancement, allocation or state mutation occurs. This
    /// scans existing bounded recovery and App storage without storing timers.
    /// Socket readiness, local commands and policy changes need independent wakes.
    #[must_use]
    pub fn next_timeout(&self) -> Option<u64> {
        if self.is_blocked() {
            return None;
        }
        let mut next = self.recovery.as_ref().and_then(CoapRecovery::next_deadline);
        if let PolicyValidity::OnlineUntil { deadline_ms, .. } = self.validity {
            next = Some(next.map_or(deadline_ms, |due| due.min(deadline_ms)));
        }
        if !self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
            && let Some(State::Established(app)) = &self.state
            && let Some(due) = app.next_timeout()
        {
            next = Some(next.map_or(due, |old| old.min(due)));
        }
        next
    }

    /// Next opportunity to drive this owner, using the same clock as [`Self::poll`].
    ///
    /// Combines [`Self::next_timeout`] with retained ingress and active App
    /// readiness. Elapsed deadlines and invalid clock values return `now_ms` so
    /// the next poll can advance or fail closed; this query does neither itself.
    /// Recompute after any state change. It does not promise I/O or admission
    /// success: wait for transport readiness or caller retry policy after failure,
    /// and wake independently for released pins, local work and policy changes.
    ///
    /// No I/O, allocation or timer mutation occurs. The mutable borrow permits
    /// App's capacity-bounded storage scan; no extra buffers or tables are used.
    #[must_use]
    pub fn next_poll_at(&mut self, now_ms: u64) -> Option<u64> {
        if self.is_blocked() {
            return None;
        }
        if now_ms < self.last_now || now_ms.checked_add(LIFETIME).is_none() {
            return Some(now_ms);
        }
        let mut next = self.next_timeout().map(|due| due.max(now_ms));
        let paused = self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate);
        match self.state.as_mut() {
            Some(State::Waiting(io)) if io.queued.is_some() => return Some(now_ms),
            Some(State::Established(app)) => {
                if app.transport().queued.is_some() {
                    return Some(now_ms);
                }
                if !paused && let Some(due) = app.next_poll_at(now_ms) {
                    next = Some(next.map_or(due, |old| old.min(due)));
                }
            }
            _ => {}
        }
        next
    }

    /// Configure an observable path before starting a connection.
    /// Its snapshot callback runs only inside authorized polling.
    pub fn observable(
        &mut self,
        path: &'static str,
        source: ObserveSource<P::Token>,
    ) -> Result<(), ManagedError<T::Error>> {
        self.configure_observable(path, Some(source))
    }

    /// Configure an observable path whose snapshots are supplied by `notify`.
    /// The dispatch callback must include Observe on a successful registration.
    pub fn observable_manual(&mut self, path: &'static str) -> Result<(), ManagedError<T::Error>> {
        self.configure_observable(path, None)
    }

    fn configure_observable(
        &mut self,
        path: &'static str,
        source: Option<ObserveSource<P::Token>>,
    ) -> Result<(), ManagedError<T::Error>> {
        if self.grant.is_some() {
            return Err(ManagedError::Busy);
        }
        let mut parts = [""; crate::app::MAX_PATH_SEGMENTS];
        crate::app::split_path(path, &mut parts).map_err(|_| ManagedError::Path)?;
        if let Some(row) = self
            .observed
            .iter_mut()
            .flatten()
            .find(|(old, _)| *old == path)
        {
            *row = (path, source);
            return Ok(());
        }
        *self
            .observed
            .iter_mut()
            .find(|row| row.is_none())
            .ok_or(ManagedError::Capacity)? = Some((path, source));
        Ok(())
    }

    /// Start a fresh initiator once outgoing application CON traffic is quiescent.
    /// Candidate failures retain an already authenticated App and its grant.
    pub fn connect(&mut self, now_ms: u64) -> Result<(), ManagedError<T::Error>> {
        self.connect_with_suite(now_ms, super::CipherSuite::Suite2)
    }

    /// Connect using a fixed local EDHOC suite; errors cannot select a fallback.
    pub fn connect_with_suite(
        &mut self,
        now_ms: u64,
        suite: super::CipherSuite,
    ) -> Result<(), ManagedError<T::Error>> {
        self.prepare(now_ms)?;
        if self.external_bootstrap {
            return Err(ManagedError::ExternalBootstrap);
        }
        if self.has_pending_con() {
            return Err(ManagedError::Busy);
        }
        let randomness = self.randomness;
        let Self {
            recovery, state, ..
        } = self;
        let io = state_io(state.as_mut().expect("managed state"));
        recovery
            .as_mut()
            .expect("prepared recovery")
            .start_with_suite(now_ms, suite, |bytes| {
                if !randomness(bytes) {
                    return false;
                }
                if bytes.len() == 22 {
                    let mid = MessageId::new(u16::from_be_bytes([bytes[0], bytes[1]]));
                    return !io.reserved(mid)
                        && !io.reserved(mid.wrapping_add(1))
                        && !io.reserved(mid.wrapping_add(2))
                        && !io.reserved(mid.wrapping_add(3));
                }
                true
            })
            .map_err(|error| ManagedError::Recovery(widen(error)))
    }

    /// Receive exactly one socket datagram and advance authorized bounded work.
    /// Dispatch receives the full authenticated peer principal, including for
    /// deferred work. Cached replies and retries cross the same policy gate.
    pub fn poll_with(
        &mut self,
        now_ms: u64,
        dispatch: impl FnMut(
            &Principal,
            Request<'_, P::Token>,
            Option<ManagedDeferredReply>,
        ) -> Response<'static, P::Token>,
    ) -> Result<(), ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.prepare(now_ms)?;
        if self.external_bootstrap {
            let io = state_io(self.state.as_mut().expect("managed state"));
            io.receive().map_err(|error| {
                ManagedError::Application(crate::app::Error::Io(
                    crate::storage::DatagramIoError::Io(error),
                ))
            })?;
            if io.queued.is_some_and(|(_, from)| from != self.endpoint) {
                io.queued = None;
            }
            return self.poll_app(now_ms, dispatch);
        }
        state_io(self.state.as_mut().expect("managed state"))
            .receive()
            .map_err(|error| {
                ManagedError::Recovery(RecoveryError::Bootstrap(super::PollError::Io(error)))
            })?;
        self.route_ingress(now_ms)?;
        self.flush_recovery(now_ms)?;
        self.install_session(now_ms)?;
        self.ensure_grant()?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Ok(());
        }
        self.poll_app(now_ms, dispatch)
    }

    /// Advance a client connection without accepting application resource work.
    /// Authenticated incoming requests receive 4.04 under the current grant.
    pub fn poll(&mut self, now_ms: u64) -> Result<(), ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.poll_with(now_ms, |_, _, _| {
            Response::new(crate::message::Code::NOT_FOUND)
        })
    }

    /// Send a protected Confirmable request to the enrolled endpoint.
    pub fn request(
        &mut self,
        method: Method,
        path: impl IntoPath,
        payload: &[u8],
        format: Option<ContentFormat>,
        now_ms: u64,
    ) -> Result<ManagedCall<P::Token>, ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.request_inner(method, path, payload, format, now_ms, false)
    }

    /// Register a protected GET Observe relation to the enrolled endpoint.
    pub fn observe(
        &mut self,
        path: impl IntoPath,
        now_ms: u64,
    ) -> Result<ManagedCall<P::Token>, ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.request_inner(Method::Get, path, &[], None, now_ms, true)
    }

    fn request_inner(
        &mut self,
        method: Method,
        path: impl IntoPath,
        payload: &[u8],
        format: Option<ContentFormat>,
        now_ms: u64,
        observe: bool,
    ) -> Result<ManagedCall<P::Token>, ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.time(now_ms)?;
        self.ensure_grant()?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Err(ManagedError::Busy);
        }
        let endpoint = self.endpoint;
        let anchor = self.grant.as_ref().expect("authorized grant").anchor();
        let epoch = state_io(self.state.as_mut().expect("managed state")).epoch;
        let mut request = self
            .app_mut()?
            .request(method, path)
            .to(endpoint)
            .payload(payload);
        if let Some(format) = format {
            request = request.content_format(format);
        }
        if observe {
            request = request.observe();
        }
        request
            .send(now_ms)
            .map(|call| ManagedCall {
                call,
                anchor,
                epoch,
            })
            .map_err(ManagedError::Application)
    }

    /// Collect a complete protected response without exposing App or key state.
    #[allow(clippy::type_complexity)]
    pub fn take_response_into<'s>(
        &'s mut self,
        call: ManagedCall<P::Token>,
        output: &'s mut [u8],
        now_ms: u64,
    ) -> Result<Option<Result<Response<'s, P::Token>, CallFailure>>, ManagedError<T::Error>> {
        self.time(now_ms)?;
        self.ensure_grant()?;
        self.check_handle(call.anchor, call.epoch)?;
        self.app_mut()?
            .take_response_into(call.call, output)
            .map_err(ManagedError::ResponseBuffer)
    }

    /// Retire a local request or subscription without sending deregistration.
    pub fn cancel(
        &mut self,
        call: ManagedCall<P::Token>,
        now_ms: u64,
    ) -> Result<bool, ManagedError<T::Error>> {
        self.time(now_ms)?;
        self.ensure_grant()?;
        self.check_handle(call.anchor, call.epoch)?;
        Ok(self.app_mut()?.cancel(call.call))
    }

    /// Finish an accepted deferred request under the current connection grant.
    pub fn complete(
        &mut self,
        handle: ManagedDeferredReply,
        response: Response<'_, P::Token>,
        now_ms: u64,
    ) -> Result<(), ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.time(now_ms)?;
        self.ensure_grant()?;
        self.check_handle(handle.anchor, handle.epoch)?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Err(ManagedError::Busy);
        }
        self.app_mut()?
            .complete(handle.handle, response, now_ms)
            .map_err(ManagedError::Application)
    }

    /// Cancel local deferred delivery and its remaining effect permission.
    /// External durable work retains the caller's own cancellation semantics.
    pub fn cancel_deferred(
        &mut self,
        handle: ManagedDeferredReply,
        now_ms: u64,
    ) -> Result<bool, ManagedError<T::Error>> {
        self.time(now_ms)?;
        self.ensure_grant()?;
        self.check_handle(handle.anchor, handle.epoch)?;
        Ok(self.app_mut()?.cancel_deferred(handle.handle))
    }

    /// Send the representation to an explicitly configured observable path.
    pub fn notify(
        &mut self,
        path: &[&str],
        response: Response<'static, P::Token>,
        now_ms: u64,
    ) -> Result<usize, ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.time(now_ms)?;
        self.ensure_grant()?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Err(ManagedError::Busy);
        }
        if !observable_path(&self.observed, path) {
            return Err(ManagedError::Path);
        }
        self.app_mut()?
            .notify(now_ms, path, response)
            .map_err(ManagedError::Application)
    }

    /// Invoke a synchronous domain authorization/commit callback while holding
    /// exclusive current connection ownership. The callback owns resource policy
    /// and durable effects; this guard does not make external effects atomic.
    /// The handle must remain pending and unexpired. Its invocation permission
    /// is consumed before the callback, including when the callback returns an
    /// error. Use a durable receipt to recover uncertain completion rather than
    /// invoking the effect again. Sending its deferred response remains separate.
    pub fn with_authorized_effect<R>(
        &mut self,
        handle: ManagedDeferredReply,
        now_ms: u64,
        effect: impl FnOnce(&Principal) -> R,
    ) -> Result<R, ManagedError<T::Error>> {
        self.time(now_ms)?;
        self.ensure_grant()?;
        self.check_handle(handle.anchor, handle.epoch)?;
        if !self.is_connected() {
            return Err(ManagedError::NotConnected);
        }
        if !self.app_mut()?.deferred_pending(handle.handle, now_ms)
            || self.effects.contains(&Some(handle))
        {
            return Err(ManagedError::StaleHandle);
        }
        let app = match self.state.as_ref().expect("managed state") {
            State::Established(app) => app,
            State::Waiting(_) => unreachable!(),
        };
        for row in &mut self.effects {
            if row.is_some_and(|old| !app.deferred_pending(old.handle, now_ms)) {
                *row = None;
            }
        }
        *self
            .effects
            .iter_mut()
            .find(|row| row.is_none())
            .ok_or(ManagedError::Capacity)? = Some(handle);
        Ok(effect(
            self.grant.as_ref().expect("authorized grant").principal(),
        ))
    }

    /// Commit revocation and discard all live traffic after success or ambiguity.
    pub fn revoke<E>(
        &mut self,
        commit: impl FnOnce(&TrustAnchor, &TrustRecord, &TrustAnchor) -> Result<(), E>,
    ) -> Result<TrustAnchor, TrustCommitError<E>> {
        if self.local_pairing.is_some() {
            return Err(TrustCommitError::State(TrustError::LocalPolicy));
        }
        let result = self.trust.revoke(commit);
        if result.is_ok() || self.trust.is_blocked() {
            self.discard();
        }
        result
    }

    /// Commit a replacement pin for this immutable local identity. Even the same
    /// pin requires a fresh handshake after a successful generation change.
    pub fn replace<E>(
        &mut self,
        peer: PinnedPeer,
        enabled: bool,
        commit: impl FnOnce(&TrustAnchor, &TrustRecord, &TrustAnchor) -> Result<(), E>,
    ) -> Result<TrustAnchor, TrustCommitError<E>> {
        if self.local_pairing.is_some() {
            return Err(TrustCommitError::State(TrustError::LocalPolicy));
        }
        let result = self.trust.replace(self.identity, peer, enabled, commit);
        if result.is_ok() || self.trust.is_blocked() {
            self.discard();
        }
        result
    }

    /// Discard all prior traffic before a freshly authenticated authority read.
    /// Failure leaves this owner blocked until another successful refresh.
    /// Success installs policy metadata, requiring a fresh handshake even when
    /// its generation and credential are unchanged.
    pub fn refresh<E>(
        &mut self,
        now_ms: u64,
        refresh: impl FnOnce(&Identity) -> Result<(PeerTrust, PolicyValidity), E>,
    ) -> Result<(), ManagedRefreshError<E>> {
        if self.local_pairing.is_some() {
            return Err(ManagedRefreshError::Trust(TrustError::LocalPolicy));
        }
        self.discard();
        self.refresh_blocked = true;
        if now_ms < self.last_now || now_ms.checked_add(LIFETIME).is_none() {
            return Err(ManagedRefreshError::Clock);
        }
        self.last_now = now_ms;
        let (trust, validity) = refresh(self.identity).map_err(ManagedRefreshError::Authority)?;
        if trust.is_blocked() {
            return Err(ManagedRefreshError::Trust(TrustError::Blocked));
        }
        if trust.checkpoint().local_principal() != self.identity.peer().principal().fingerprint() {
            return Err(ManagedRefreshError::Trust(TrustError::IdentityMismatch));
        }
        if matches!(validity, PolicyValidity::OnlineUntil { accepted_ms, .. } if now_ms < accepted_ms)
        {
            return Err(ManagedRefreshError::Clock);
        }
        if matches!(validity, PolicyValidity::OnlineUntil { deadline_ms, .. } if now_ms >= deadline_ms)
        {
            return Err(ManagedRefreshError::PolicyExpired);
        }
        self.trust = trust;
        self.validity = validity;
        self.refresh_blocked = false;
        Ok(())
    }

    fn prepare(&mut self, now: u64) -> Result<(), ManagedError<T::Error>> {
        self.time(now)?;
        if self.refresh_blocked {
            return Err(ManagedError::Trust(TrustError::Blocked));
        }
        if self.external_bootstrap {
            return self.ensure_grant();
        }
        if self.grant.is_none() {
            let grant = self
                .trust
                .grant(self.identity)
                .map_err(ManagedError::Trust)?;
            let recovery = CoapRecovery::new(
                self.identity,
                self.trust.checkpoint().peer().clone(),
                self.endpoint,
                now,
            )
            .map_err(|error| ManagedError::Recovery(widen(error)))?;
            self.grant = Some(grant);
            self.recovery = Some(recovery);
        }
        self.ensure_grant()
    }

    fn time(&mut self, now: u64) -> Result<(), ManagedError<T::Error>> {
        if now < self.last_now || now.checked_add(LIFETIME).is_none() {
            self.discard();
            self.refresh_blocked = true;
            return Err(ManagedError::Clock);
        }
        self.last_now = now;
        state_io(self.state.as_mut().expect("managed state")).prune(now);
        if matches!(self.validity, PolicyValidity::OnlineUntil { deadline_ms, .. } if now >= deadline_ms)
        {
            self.discard();
            self.refresh_blocked = true;
            return Err(ManagedError::PolicyExpired);
        }
        Ok(())
    }

    fn ensure_grant(&mut self) -> Result<(), ManagedError<T::Error>> {
        if self.refresh_blocked {
            return Err(ManagedError::Trust(TrustError::Blocked));
        }
        let result: Result<(), TrustError> = match &self.grant {
            Some(grant) if self.trust.permits(grant, grant.principal()) => return Ok(()),
            Some(_) => self
                .trust
                .grant(self.identity)
                .map(|_| ())
                .and(Err(TrustError::StaleGrant)),
            None => self
                .trust
                .grant(self.identity)
                .map(|_| ())
                .and(Err(TrustError::StaleGrant)),
        };
        self.discard();
        Err(ManagedError::Trust(result.expect_err("invalid grant")))
    }

    fn has_pending_con(&self) -> bool {
        match &self.state {
            Some(State::Established(app)) => {
                (0..app.engine().capacities().tx_datagram_slots).any(|index| {
                    app.engine()
                        .pending_con(SlotId::from_index(index))
                        .is_some()
                })
            }
            _ => false,
        }
    }

    fn check_handle(
        &mut self,
        anchor: TrustAnchor,
        epoch: u64,
    ) -> Result<(), ManagedError<T::Error>> {
        if self
            .grant
            .as_ref()
            .is_none_or(|grant| grant.anchor() != anchor)
            || state_io(self.state.as_mut().expect("managed state")).epoch != epoch
        {
            return Err(ManagedError::StaleHandle);
        }
        Ok(())
    }

    fn route_ingress(&mut self, now_ms: u64) -> Result<(), ManagedError<T::Error>> {
        if self.external_bootstrap {
            let io = state_io(self.state.as_mut().expect("managed state"));
            if io.queued.is_some_and(|(_, from)| from != self.endpoint) {
                io.queued = None;
            }
            return Ok(());
        }
        let pending_con = self.has_pending_con();
        let app_mid_collision = match self.state.as_ref().expect("managed state") {
            State::Established(app) => app.transport().queued.is_some_and(|(len, from)| {
                decode(&app.transport().bytes[..len]).is_ok_and(|message| {
                    matches!(message.ty(), Type::Confirmable | Type::NonConfirmable)
                        && app
                            .engine()
                            .lookup_dedup(DedupKey::new(message.message_id(), from))
                            .is_some()
                })
            }),
            State::Waiting(_) => false,
        };
        let Self {
            trust,
            grant,
            recovery,
            state,
            endpoint,
            randomness,
            ..
        } = self;
        let io = state_io(state.as_mut().expect("managed state"));
        if let Some((len, from)) = io.queued {
            let bootstrap = decode(&io.bytes[..len]).is_ok_and(|message| {
                if message
                    .options()
                    .any(|option| option.number() == OptionNumber::OSCORE)
                {
                    return false;
                }
                let format = message
                    .content_format()
                    .and_then(Result::ok)
                    .map(|format| format.get());
                let mut echoes = message
                    .options()
                    .filter(|option| option.number() == OptionNumber::ECHO);
                let echo_challenge = message.code() == crate::message::Code::UNAUTHORIZED
                    && message.payload().is_empty()
                    && echoes
                        .next()
                        .is_some_and(|option| (1..=40).contains(&option.value().len()))
                    && echoes.next().is_none()
                    && recovery.as_ref().is_some_and(CoapRecovery::has_candidate);
                (message.code() == crate::message::Code::POST
                    && format == Some(65)
                    && message
                        .options()
                        .filter(|option| option.number() == OptionNumber::URI_PATH)
                        .map(|option| option.value())
                        .eq([b".well-known".as_slice(), b"edhoc".as_slice()]))
                    || ((message.code().is_success()
                        || message.code().is_client_error()
                        || message.code().is_server_error())
                        && format == Some(64))
                    || echo_challenge
                    || recovery.as_ref().is_some_and(|recovery| {
                        recovery.owns_error_response(&io.bytes[..len], from, now_ms)
                    })
                    || (message.code().is_empty()
                        && recovery.as_ref().is_some_and(CoapRecovery::has_candidate))
            });
            if from != *endpoint {
                io.queued = None;
            } else if bootstrap {
                let captured = grant.as_ref().expect("prepared grant");
                let result = if app_mid_collision
                    || (pending_con
                        && !recovery
                            .as_ref()
                            .expect("prepared recovery")
                            .has_candidate()
                        && decode(&io.bytes[..len])
                            .is_ok_and(|message| message.code().is_request()))
                {
                    Ok(super::Status::Ignored)
                } else {
                    recovery.as_mut().expect("prepared recovery").ingest(
                        &io.bytes[..len],
                        from,
                        now_ms,
                        *randomness,
                        |principal| trust.permits(captured, principal),
                    )
                };
                io.queued = None;
                result.map_err(|error| ManagedError::Recovery(widen(error)))?;
            } else if recovery
                .as_ref()
                .expect("prepared recovery")
                .has_candidate()
                || !matches!(state, Some(State::Established(_)))
            {
                state_io(state.as_mut().expect("managed state")).queued = None;
            }
        }
        Ok(())
    }

    fn flush_recovery(&mut self, now: u64) -> Result<(), ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.ensure_grant()?;
        let io = state_io(self.state.as_mut().expect("managed state"));
        io.bootstrap_mode = true;
        let result = self
            .recovery
            .as_mut()
            .expect("prepared recovery")
            .flush(io, now);
        io.bootstrap_mode = false;
        result.map(|_| ()).map_err(ManagedError::Recovery)
    }

    fn install_session(&mut self, now: u64) -> Result<(), ManagedError<T::Error>> {
        let Some(session) = self
            .recovery
            .as_mut()
            .expect("prepared recovery")
            .take_session()
        else {
            return Ok(());
        };
        let session = self
            .trust
            .accept_session(self.grant.as_ref().expect("prepared grant"), session)
            .map_err(ManagedError::Trust)?;
        if let Err(error) = self.install_authenticated(session, now) {
            self.recovery
                .as_mut()
                .expect("prepared recovery")
                .abandon_handoff(now)
                .map_err(|error| ManagedError::Recovery(widen(error)))?;
            return Err(error);
        }
        self.recovery
            .as_mut()
            .expect("prepared recovery")
            .confirm_handoff();
        Ok(())
    }

    fn install_authenticated(
        &mut self,
        session: super::Session,
        now: u64,
    ) -> Result<(), ManagedError<T::Error>> {
        if !state_io(self.state.as_mut().expect("managed state")).next_epoch_available() {
            return Err(ManagedError::Capacity);
        }
        let (context, _) = session.into_parts();
        let context = context
            .into_token_storage::<P::Token>()
            .map_err(|error| ManagedError::Application(crate::app::Error::Oscore(error)))?;
        let mut builder = App::profile::<P>()
            .routes::<ROUTES>()
            .block_wise::<false>()
            .deferred::<DEFERRED>()
            .full_responses()
            .randomness(self.randomness)
            .oscore(context);
        for &(path, source) in self.observed.iter().flatten() {
            let route = source.map_or(MethodRouter::new(), |source| {
                MethodRouter::new().observe(source)
            });
            builder = builder.route_path(path, route);
        }
        let mut app = builder
            .bind(ConnectionIo::new(None, now))
            .map_err(ManagedError::Build)?;
        let mut io = match self.state.take().expect("managed state") {
            State::Waiting(io) => io,
            State::Established(old) => old.into_io(),
        };
        io.queued = None;
        io.epoch += 1;
        *app.transport_mut() = io;
        self.state = Some(State::Established(app));
        self.effects.fill(None);
        Ok(())
    }

    fn poll_app(
        &mut self,
        now_ms: u64,
        mut dispatch: impl FnMut(
            &Principal,
            Request<'_, P::Token>,
            Option<ManagedDeferredReply>,
        ) -> Response<'static, P::Token>,
    ) -> Result<(), ManagedError<T::Error>>
    where
        T: DatagramIo,
    {
        self.ensure_grant()?;
        let grant = self.grant.as_ref().expect("prepared grant");
        let captured = grant.principal();
        let anchor = grant.anchor();
        let observed = &self.observed;
        if let Some(State::Established(app)) = &mut self.state {
            let epoch = app.transport().epoch;
            app.poll_with(now_ms, |request, deferred| {
                let response = dispatch(
                    captured,
                    request,
                    deferred.map(|handle| ManagedDeferredReply {
                        handle,
                        anchor,
                        epoch,
                    }),
                );
                if request.is_observe_register() && !observable_path(observed, request.path()) {
                    response.without_observe()
                } else {
                    response
                }
            })
            .map_err(ManagedError::Application)?;
        }
        Ok(())
    }

    fn app_mut(
        &mut self,
    ) -> Result<&mut SecureApp<P, T, ROUTES, DEFERRED>, ManagedError<T::Error>> {
        match self.state.as_mut().expect("managed state") {
            State::Established(app) => Ok(app),
            State::Waiting(_) => Err(ManagedError::NotConnected),
        }
    }

    fn discard(&mut self) {
        let mut io = match self.state.take().expect("managed state") {
            State::Waiting(io) => io,
            State::Established(app) => app.into_io(),
        };
        io.queued = None;
        self.state = Some(State::Waiting(io));
        self.grant = None;
        self.recovery = None;
        self.effects.fill(None);
    }
}

type ObservedRoute<K> = (&'static str, Option<ObserveSource<K>>);

fn observable_path<const ROUTES: usize, K: crate::message::OwnedToken>(
    observed: &[Option<ObservedRoute<K>>; ROUTES],
    path: &[&str],
) -> bool {
    observed.iter().flatten().any(|(registered, _)| {
        let mut parts = [""; crate::app::MAX_PATH_SEGMENTS];
        crate::app::split_path(registered, &mut parts).is_ok_and(|len| parts[..len] == *path)
    })
}

fn state_io<P, T, const ROUTES: usize, const DEFERRED: usize>(
    state: &mut State<P, T, ROUTES, DEFERRED>,
) -> &mut ConnectionIo<T>
where
    P: MemoryProfile + MemoryLayout<false> + AppAssembled<false>,
{
    match state {
        State::Waiting(io) => io,
        State::Established(app) => app.transport_mut(),
    }
}

fn widen<E>(error: RecoveryError) -> RecoveryError<ManagedIoError<E>> {
    match error {
        RecoveryError::Capacity => RecoveryError::Capacity,
        RecoveryError::Busy => RecoveryError::Busy,
        RecoveryError::Bootstrap(error) => RecoveryError::Bootstrap(match error {
            super::PollError::Io(never) => match never {},
            super::PollError::Provisioning(error) => super::PollError::Provisioning(error),
            super::PollError::Timeout => super::PollError::Timeout,
            super::PollError::ShortSend => super::PollError::ShortSend,
            super::PollError::Clock => super::PollError::Clock,
            super::PollError::Rejected => super::PollError::Rejected,
            super::PollError::PeerAbort(code) => super::PollError::PeerAbort(code),
        }),
    }
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
