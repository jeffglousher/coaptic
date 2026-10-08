//! Online commissioning and policy refresh over a separately pinned authority.
//!
//! The bootstrap pin must come from an authenticated commissioning path whose
//! trust does not depend on the record being refreshed. A public identifier,
//! ordinary-storage snapshot or unauthenticated discovery cannot supply it.
//! Each exchange consumes a fresh EDHOC session and a fresh entropy challenge;
//! no session, challenge or pending exchange may be restored after restart.
//!
//! This is a versioned application exchange over standard CoAP and OSCORE,
//! not a standardized ownership-transfer protocol. The authority provider owns
//! authenticated administrator approval, exclusive device ownership, resource
//! authorization, durable transactions and independently current policy. A
//! rollbackable database backup is not a monotonic authority. The host example
//! supplies an in-memory fixture, not a production storage or ownership service.
//!
//! A verified snapshot proves the authority's response to this fresh challenge.
//! It does not promise instantaneous visibility of later external revocation.
//! Serialize refresh and application admission with policy changes; unavailable
//! refresh blocks admission. Offline rollback resistance needs a separately
//! qualified protected monotonic provider.

use core::cell::Cell;
use core::fmt;

use crate::Endpoint;
use crate::message::{
    Code, ContentFormat, Message, MessageId, Opt, OptionNumber, ParsedMessage, Token, Type, decode,
};
use crate::oscore::{RequestRef, SecurityContext};

use super::{
    Identity, PeerTrust, PinnedPeer, Principal, Session, TrustAnchor, TrustCommitError, TrustError,
    TrustRecord, TrustRecordParts,
};

const REQUEST_DOMAIN: &str = "https://github.com/jeffglousher/coaptic online policy request v1";
const POLICY_DOMAIN: &str = "https://github.com/jeffglousher/coaptic online policy snapshot v1";
const REQUEST_LEN: usize = 271;
const RESPONSE_LEN: usize = 272;
const DATAGRAM_LEN: usize = 384;
const POLICY_PATH: &str = "coaptic-policy";

/// Refusal of an authority exchange; a failed exchange is consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnrollmentError {
    /// The supplied session did not authenticate the independent authority pin.
    Authority,
    /// The supplied local identity differs from the authenticated local principal.
    LocalIdentity,
    /// Fresh cryptographic entropy failed or returned an all-zero challenge.
    Entropy,
    /// The monotonic deadline expired, exceeded the profile bound or moved backwards.
    Deadline,
    /// The datagram came from a different transport endpoint.
    Endpoint,
    /// Framing, version, options, token, message ID or exact payload length failed.
    Message,
    /// The response differs from the exact challenge, operation or policy binding.
    Binding,
    /// The authority returned an older generation or changed the same generation.
    Rollback,
    /// Owner and resource-policy commitments must both be nonzero.
    Policy,
    /// The public trust record was invalid.
    Trust(TrustError),
    /// OSCORE protection or authentication failed.
    Oscore(crate::oscore::Error),
}

impl fmt::Display for EnrollmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority => f.write_str("session did not authenticate bootstrap authority"),
            Self::LocalIdentity => f.write_str("session local identity differs"),
            Self::Entropy => f.write_str("fresh challenge entropy failed"),
            Self::Deadline => f.write_str("authority exchange monotonic deadline failed"),
            Self::Endpoint => f.write_str("authority endpoint differs"),
            Self::Message => f.write_str("invalid authority application message"),
            Self::Binding => f.write_str("authority response binding differs"),
            Self::Rollback => f.write_str("authority policy generation moved backwards"),
            Self::Policy => f.write_str("owner and resource policy must be nonzero"),
            Self::Trust(error) => error.fmt(f),
            Self::Oscore(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for EnrollmentError {}

impl From<crate::oscore::Error> for EnrollmentError {
    fn from(value: crate::oscore::Error) -> Self {
        Self::Oscore(value)
    }
}

/// Independently provisioned management authority credential.
///
/// Construct only from a pin installed through an authenticated commissioning
/// path independent of mutable device policy. This constructor validates no
/// manufacturing provenance or ownership claim. Public QR labels are lookup
/// metadata; administrator approval and credential possession remain required.
#[derive(Clone, Debug)]
pub struct BootstrapAuthority(PinnedPeer);

impl BootstrapAuthority {
    /// Wrap the caller's independently authenticated authority pin.
    #[must_use]
    pub const fn new(pin: PinnedPeer) -> Self {
        Self(pin)
    }

    /// Pin to use for a fresh EDHOC exchange with the authority.
    #[must_use]
    pub const fn pin(&self) -> &PinnedPeer {
        &self.0
    }
}

/// Exact owner and server policy selected by authenticated commissioning.
///
/// The owner is the full identifier used by the authority's account registry.
/// `resource_policy` is a full commitment to that registry's canonical resource
/// permissions. The authority must enforce those permissions independently;
/// this metadata does not grant access and is not a secret or bearer token.
#[derive(Clone, Debug)]
pub struct EnrollmentPolicy {
    peer: PinnedPeer,
    owner: [u8; 32],
    resource_policy: [u8; 32],
}

impl EnrollmentPolicy {
    /// Bind the installed application peer to an approved owner and policy.
    pub fn new(
        peer: PinnedPeer,
        owner: [u8; 32],
        resource_policy: [u8; 32],
    ) -> Result<Self, EnrollmentError> {
        validate_policy(&owner, &resource_policy)?;
        Ok(Self {
            peer,
            owner,
            resource_policy,
        })
    }

    /// Exact application credential to install after approval.
    #[must_use]
    pub const fn peer(&self) -> &PinnedPeer {
        &self.peer
    }

    /// Full approved owner identifier.
    #[must_use]
    pub const fn owner(&self) -> &[u8; 32] {
        &self.owner
    }

    /// Full canonical resource-policy commitment.
    #[must_use]
    pub const fn resource_policy(&self) -> &[u8; 32] {
        &self.resource_policy
    }
}

/// Requested authority operation with fixed inline storage and no allocator.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum PolicyQuery {
    /// First installation against a confirmed never-initialized store.
    Enroll {
        /// Confirmed never-initialized authority store.
        expected: TrustAnchor,
        /// Exact administrator-approved owner, application pin and permissions.
        policy: EnrollmentPolicy,
    },
    /// Freshly resolve current policy, including after an uncertain first commit.
    Refresh {
        /// Exact last-known public record anchor, not assumed current.
        expected: TrustAnchor,
        /// Full last-known snapshot commitment; zero only for genesis recovery.
        policy_commitment: [u8; 32],
    },
}

impl PolicyQuery {
    /// The caller's exact last-known anchor; it is not assumed fresh.
    #[must_use]
    pub const fn expected(&self) -> TrustAnchor {
        match self {
            Self::Enroll { expected, .. } | Self::Refresh { expected, .. } => *expected,
        }
    }

    /// Refresh an exact last-known snapshot without assuming its freshness.
    #[must_use]
    pub fn refresh(snapshot: &PolicySnapshot) -> Self {
        Self::Refresh {
            expected: snapshot.record.anchor(),
            policy_commitment: snapshot.commitment(),
        }
    }

    /// Recover an uncertain first commit using the original confirmed genesis.
    /// Missing local state or failed restore does not establish this genesis.
    pub fn recover_initial_commit(expected: TrustAnchor) -> Result<Self, EnrollmentError> {
        if expected.parts().1 != 0 {
            return Err(EnrollmentError::Trust(TrustError::Initialized));
        }
        Ok(Self::Refresh {
            expected,
            policy_commitment: [0; 32],
        })
    }
}

/// Authoritative public policy returned by the trusted service provider.
///
/// Construction checks representation only. On the server, obtain it through
/// [`OnlinePolicyAuthority`]'s serialized authenticated management transaction.
/// The anchor is the exact full-record digest; it supplies no monotonicity by
/// itself. Preserve owner and permission commitments in the authoritative store.
#[derive(Clone, Debug)]
pub struct PolicySnapshot {
    record: TrustRecord,
    owner: [u8; 32],
    resource_policy: [u8; 32],
}

impl PolicySnapshot {
    /// Validate a service provider's public record and policy commitments.
    pub fn new(
        record: TrustRecord,
        owner: [u8; 32],
        resource_policy: [u8; 32],
    ) -> Result<Self, EnrollmentError> {
        validate_policy(&owner, &resource_policy)?;
        Ok(Self {
            record,
            owner,
            resource_policy,
        })
    }

    /// Exact application trust record, including disabled tombstones.
    #[must_use]
    pub const fn record(&self) -> &TrustRecord {
        &self.record
    }

    /// Full current owner identifier.
    #[must_use]
    pub const fn owner(&self) -> &[u8; 32] {
        &self.owner
    }

    /// Full current canonical permissions commitment.
    #[must_use]
    pub const fn resource_policy(&self) -> &[u8; 32] {
        &self.resource_policy
    }

    /// Full digest binding the record anchor, owner and resource permissions.
    /// This is exact snapshot metadata, not authentication or anti-rollback.
    #[must_use]
    pub fn commitment(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key(POLICY_DOMAIN);
        let mut anchor = [0; 72];
        encode_anchor(self.record.anchor(), &mut anchor);
        hash.update(&anchor);
        hash.update(&self.owner);
        hash.update(&self.resource_policy);
        *hash.finalize().as_bytes()
    }
}

/// A request authenticated with the actual device credential and OSCORE context.
///
/// Only [`AuthorityResponder::serve`] creates this value after authenticating
/// the request and comparing its complete device fingerprint with the session.
/// Its fields supply no administrator approval; the authority provider must
/// check the separately authenticated approval before enrollment.
#[derive(Debug)]
pub struct AuthenticatedPolicyRequest {
    principal: Principal,
    query: PolicyQuery,
}

impl AuthenticatedPolicyRequest {
    /// Complete device principal authenticated by EDHOC and OSCORE.
    #[must_use]
    pub const fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Exact requested operation and expected store state.
    #[must_use]
    pub const fn query(&self) -> &PolicyQuery {
        &self.query
    }
}

/// Independently current, serialized ownership and trust authority provider.
///
/// For `Enroll`, authenticate administrator approval for this exact device,
/// owner, application pin and permissions; enforce exclusive ownership; compare
/// and swap the exact confirmed genesis; and return success only after a
/// power-loss-durable transaction. Device possession alone is insufficient.
/// For `Refresh`, resolve the latest committed record or tombstone for the
/// authenticated device and requested store. Missing state or failed validation
/// is an error; never synthesize genesis after a failed restore.
///
/// The service must obtain current policy from an authority that survives
/// rollback of ordinary storage, or quarantine/reconcile restored databases
/// against a qualified independent authority. Transaction durability alone does
/// not establish freshness. An ambiguous commit must produce an error; the
/// device starts a fresh session and refreshes to learn what actually committed.
/// Advance the record generation for every owner or resource-policy change,
/// including otherwise unchanged peer credentials, so equal-generation
/// commitments always name identical complete snapshots.
/// Bound lookups and storage work before allocating request state.
pub trait OnlinePolicyAuthority {
    /// Store refusal, unavailability, conflict or ambiguous commit result.
    type Error;

    /// Resolve one authenticated device operation under management serialization.
    fn resolve(
        &mut self,
        request: &AuthenticatedPolicyRequest,
    ) -> Result<PolicySnapshot, Self::Error>;
}

/// Failure before or during the authority's management transaction.
#[derive(Debug)]
pub enum PolicyServeError<E> {
    /// Authentication, framing or public policy validation failed.
    Exchange(EnrollmentError),
    /// Authority refused or could not establish its current durable state.
    Provider(E),
}

/// Fresh, single-operation authority channel with no mutable context escape.
///
/// Complete EDHOC using the independent bootstrap pin first. A client channel
/// checks both authenticated principals; a server channel checks its actual
/// authority identity. The endpoint is routing metadata, not authentication.
/// Consuming one channel per operation prevents response nonce reuse and makes
/// failure invalidate the entire exchange. Transport admission, deadlines and
/// exact retransmission scheduling remain caller-owned.
pub struct AuthorityConnection {
    context: SecurityContext,
    local: Principal,
    peer: Principal,
    endpoint: Endpoint,
}

/// Fresh server channel that can only answer an authenticated device operation.
/// It cannot issue a client request or supply a bootstrap-pin bypass.
///
/// ```compile_fail
/// # use coaptic::provisioning::{AuthorityResponder, PolicyQuery};
/// # use coaptic::message::MessageId;
/// fn bypass_pin(server: AuthorityResponder, query: PolicyQuery) {
///     let _ = server.begin(query, MessageId::new(1), |_| true);
/// }
/// ```
pub struct AuthorityResponder {
    channel: AuthorityConnection,
}

impl AuthorityResponder {
    /// Accept a server session using the service's actual authority identity.
    /// EDHOC must already have authenticated the device credential; the provider
    /// subsequently verifies administrator approval, ownership and permissions.
    pub fn from_session(
        identity: &Identity,
        endpoint: Endpoint,
        session: Session,
    ) -> Result<Self, EnrollmentError> {
        if *session.local_principal() != identity.peer().principal() {
            return Err(EnrollmentError::LocalIdentity);
        }
        Ok(Self {
            channel: AuthorityConnection::take(endpoint, session),
        })
    }

    /// Authenticate one device request and return one immutable protected reply.
    /// Consume the channel before calling the provider. Retain the exact reply
    /// for retries; never resolve or encrypt a second answer for this exchange.
    pub fn serve<A: OnlinePolicyAuthority>(
        self,
        endpoint: Endpoint,
        datagram: &[u8],
        authority: &mut A,
    ) -> Result<ProtectedPolicyResponse, PolicyServeError<A::Error>> {
        self.channel.serve(endpoint, datagram, authority)
    }
}

impl AuthorityConnection {
    /// Maximum authority response lifetime in caller-supplied monotonic milliseconds.
    pub const MAX_EXCHANGE_LIFETIME_MS: u64 = 30_000;

    /// Accept a fresh client session authenticated by the independent pin.
    pub fn from_session(
        identity: &Identity,
        authority: &BootstrapAuthority,
        endpoint: Endpoint,
        session: Session,
    ) -> Result<Self, EnrollmentError> {
        if *session.local_principal() != identity.peer().principal() {
            return Err(EnrollmentError::LocalIdentity);
        }
        if *session.principal() != authority.pin().principal() {
            return Err(EnrollmentError::Authority);
        }
        Ok(Self::take(endpoint, session))
    }

    fn take(endpoint: Endpoint, session: Session) -> Self {
        let local = *session.local_principal();
        let (context, peer) = session.into_parts();
        Self {
            context,
            local,
            peer,
            endpoint,
        }
    }

    /// Issue one protected request with fresh cryptographically secure entropy.
    ///
    /// The entropy provider must be qualified across restart, including reseeding
    /// and clone/snapshot behavior. Retransmit only [`PendingPolicy::datagram`]
    /// within the caller's CoAP deadline and congestion limits. After failure,
    /// start fresh EDHOC and refresh the authority; never restore this channel.
    /// `now_ms`, later request checks and response acceptance must use the same
    /// qualified monotonic clock. Wall-clock corrections and persisted clock
    /// values cannot establish this exchange's lifetime.
    pub fn begin(
        mut self,
        query: PolicyQuery,
        message_id: MessageId,
        now_ms: u64,
        deadline_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<PendingPolicy, EnrollmentError> {
        if deadline_ms <= now_ms || deadline_ms - now_ms > Self::MAX_EXCHANGE_LIFETIME_MS {
            return Err(EnrollmentError::Deadline);
        }
        if matches!(&query, PolicyQuery::Enroll { expected, .. } if expected.parts().1 != 0) {
            return Err(EnrollmentError::Trust(TrustError::Initialized));
        }
        if matches!(&query, PolicyQuery::Refresh { expected, policy_commitment }
            if (expected.parts().1 == 0) != (*policy_commitment == [0; 32]))
        {
            return Err(EnrollmentError::Binding);
        }
        let mut nonce = [0; 32];
        if !entropy(&mut nonce) || nonce == [0; 32] {
            return Err(EnrollmentError::Entropy);
        }
        let payload = encode_request(self.local, &query, nonce);
        let digest = blake3::derive_key(REQUEST_DOMAIN, &payload);
        let token = Token::from_checked(&nonce[..8]);
        let format = ContentFormat::OCTET_STREAM.encode();
        let options = [Opt::uri_path(POLICY_PATH), Opt::content_format(&format)];
        let message = Message::con(Code::POST, message_id, token)
            .with_options(&options)
            .with_payload(&payload);
        let mut datagram = [0; DATAGRAM_LEN];
        let len = self.context.protect_request(&message, &mut datagram)?;
        let request = self.context.lookup(token).ok_or(EnrollmentError::Binding)?;
        Ok(PendingPolicy {
            channel: self,
            query,
            nonce,
            digest,
            token,
            message_id,
            deadline_ms,
            last_seen_ms: Cell::new(now_ms),
            clock_blocked: Cell::new(false),
            request,
            datagram,
            len,
        })
    }

    /// Authenticate one device request and return one immutable protected reply.
    ///
    /// Consume the channel before calling the provider. Retain and retransmit
    /// the returned exact datagram; do not resolve the transaction again for a
    /// retry. If an exchange or commit outcome is uncertain, the device must
    /// refresh using a new EDHOC session. This exchange accepts a single small
    /// piggybacked ACK response; block-wise and Observe are deliberately refused.
    fn serve<A: OnlinePolicyAuthority>(
        mut self,
        endpoint: Endpoint,
        datagram: &[u8],
        authority: &mut A,
    ) -> Result<ProtectedPolicyResponse, PolicyServeError<A::Error>> {
        let exchange = |error| PolicyServeError::Exchange(error);
        if endpoint != self.endpoint {
            return Err(exchange(EnrollmentError::Endpoint));
        }
        let outer = decode(datagram).map_err(|_| exchange(EnrollmentError::Message))?;
        let mut scratch = [0; DATAGRAM_LEN];
        let (message, reference) = self
            .context
            .unprotect_request(&outer, &mut scratch)
            .map_err(|error| exchange(error.into()))?;
        if message.ty() != Type::Confirmable
            || message.code() != Code::POST
            || !valid_options(message, true)
        {
            return Err(exchange(EnrollmentError::Message));
        }
        let (local, query, nonce) = decode_request(message.payload()).map_err(exchange)?;
        if local != *self.peer.fingerprint() {
            return Err(exchange(EnrollmentError::LocalIdentity));
        }
        let digest = blake3::derive_key(REQUEST_DOMAIN, message.payload());
        let request = AuthenticatedPolicyRequest {
            principal: self.peer,
            query,
        };
        let snapshot = authority
            .resolve(&request)
            .map_err(PolicyServeError::Provider)?;
        validate_snapshot(&snapshot, &request.query, &self.peer).map_err(exchange)?;
        let payload = encode_response(nonce, digest, &snapshot);
        let format = ContentFormat::OCTET_STREAM.encode();
        let options = [Opt::content_format(&format)];
        let reply = Message::new(Type::Acknowledgement, Code::CONTENT, message.message_id())
            .with_token(message.token())
            .with_options(&options)
            .with_payload(&payload);
        let mut bytes = [0; DATAGRAM_LEN];
        let len = self
            .context
            .protect_response(&reply, reference, &mut bytes)
            .map_err(|error| exchange(error.into()))?;
        Ok(ProtectedPolicyResponse {
            endpoint,
            bytes,
            len,
        })
    }
}

/// One protected request awaiting its exact authority response.
pub struct PendingPolicy {
    channel: AuthorityConnection,
    query: PolicyQuery,
    nonce: [u8; 32],
    digest: [u8; 32],
    token: Token,
    message_id: MessageId,
    deadline_ms: u64,
    last_seen_ms: Cell<u64>,
    clock_blocked: Cell<bool>,
    request: RequestRef,
    datagram: [u8; DATAGRAM_LEN],
    len: usize,
}

impl PendingPolicy {
    /// Destination and exact protected bytes; retransmission never re-encrypts.
    pub fn datagram(&self, now_ms: u64) -> Result<(Endpoint, &[u8]), EnrollmentError> {
        self.check_deadline(now_ms)?;
        Ok((self.channel.endpoint, &self.datagram[..self.len]))
    }

    /// Exact monotonic response deadline for the caller's transport scheduler.
    #[must_use]
    pub const fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }

    fn check_deadline(&self, now_ms: u64) -> Result<(), EnrollmentError> {
        if self.clock_blocked.get()
            || now_ms < self.last_seen_ms.get()
            || now_ms >= self.deadline_ms
        {
            self.clock_blocked.set(true);
            Err(EnrollmentError::Deadline)
        } else {
            self.last_seen_ms.set(now_ms);
            Ok(())
        }
    }

    /// Consume this exchange and authenticate its complete policy response.
    ///
    /// Every error consumes the pending request and its keys. The same reply
    /// cannot be accepted twice. This deliberately provides no offline lease or
    /// recovery of a previous security context.
    pub fn accept(
        self,
        identity: &Identity,
        endpoint: Endpoint,
        now_ms: u64,
        datagram: &[u8],
    ) -> Result<VerifiedPolicy, EnrollmentError> {
        self.check_deadline(now_ms)?;
        if self.channel.local != identity.peer().principal() {
            return Err(EnrollmentError::LocalIdentity);
        }
        if endpoint != self.channel.endpoint {
            return Err(EnrollmentError::Endpoint);
        }
        let outer = decode(datagram).map_err(|_| EnrollmentError::Message)?;
        if outer.token() != self.token
            || outer.message_id() != self.message_id
            || outer.ty() != Type::Acknowledgement
        {
            return Err(EnrollmentError::Message);
        }
        let mut scratch = [0; DATAGRAM_LEN];
        let message =
            self.channel
                .context
                .unprotect_response(&outer, self.request, &mut scratch)?;
        if message.code() != Code::CONTENT || !valid_options(message, false) {
            return Err(EnrollmentError::Message);
        }
        let (nonce, digest, snapshot) = decode_response(message.payload())?;
        if nonce != self.nonce || digest != self.digest {
            return Err(EnrollmentError::Binding);
        }
        validate_snapshot(&snapshot, &self.query, &self.channel.local)?;
        PeerTrust::restore(identity, snapshot.record.clone(), snapshot.record.anchor())
            .map_err(EnrollmentError::Trust)?;
        Ok(VerifiedPolicy {
            snapshot,
            accepted_ms: now_ms,
            deadline_ms: self.deadline_ms,
        })
    }
}

/// Exact protected response for one authenticated device request.
pub struct ProtectedPolicyResponse {
    endpoint: Endpoint,
    bytes: [u8; DATAGRAM_LEN],
    len: usize,
}

impl ProtectedPolicyResponse {
    /// Destination and immutable bytes for bounded exact retransmission.
    #[must_use]
    pub fn datagram(&self) -> (Endpoint, &[u8]) {
        (self.endpoint, &self.bytes[..self.len])
    }
}

/// Current public policy authenticated against one fresh authority challenge.
///
/// This value cannot be cloned or constructed from public metadata. The local
/// persistence callback mirrors the already committed authority record; it does
/// not advance or establish the authority's monotonic state. A failed mirror
/// write returns no usable trust. Refresh again after ambiguous failure.
///
/// ```compile_fail
/// # use coaptic::provisioning::VerifiedPolicy;
/// fn reuse(policy: VerifiedPolicy) {
///     let _ = policy.clone();
/// }
/// ```
pub struct VerifiedPolicy {
    snapshot: PolicySnapshot,
    accepted_ms: u64,
    deadline_ms: u64,
}

impl VerifiedPolicy {
    /// Authenticated current snapshot, including owner and policy commitments.
    #[must_use]
    pub const fn snapshot(&self) -> &PolicySnapshot {
        &self.snapshot
    }

    /// Monotonic time at which this exact response was authenticated.
    #[must_use]
    pub const fn accepted_ms(&self) -> u64 {
        self.accepted_ms
    }

    /// Original challenge deadline; acceptance never extends the policy lifetime.
    /// Enforce this deadline before every managed application I/O and effect.
    #[must_use]
    pub const fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }

    /// Persist the exact public mirror before restoring application trust.
    ///
    /// The caller's private identity and independent bootstrap pin must already
    /// be durably available. Success means crash-consistent local persistence;
    /// anti-rollback still comes from a fresh authority exchange on restart.
    /// This method returns trust metadata, not an unrestricted online lease;
    /// the managed owner must preserve [`Self::accepted_ms`] and the original
    /// [`Self::deadline_ms`] and enforce them before every I/O and effect.
    /// The callback must report ambiguous writes as errors. It cannot create a
    /// usable trust policy on failure and must never replace authority with
    /// genesis. The returned policy may be disabled; grant acquisition then
    /// refuses application admission.
    pub fn persist_and_restore<E>(
        self,
        identity: &Identity,
        persist: impl FnOnce(&PolicySnapshot) -> Result<(), E>,
    ) -> Result<PeerTrust, TrustCommitError<E>> {
        let anchor = self.snapshot.record.anchor();
        let trust = PeerTrust::restore(identity, self.snapshot.record.clone(), anchor)
            .map_err(TrustCommitError::State)?;
        persist(&self.snapshot).map_err(TrustCommitError::Persistence)?;
        Ok(trust)
    }
}

fn validate_policy(owner: &[u8; 32], resource_policy: &[u8; 32]) -> Result<(), EnrollmentError> {
    if owner == &[0; 32] || resource_policy == &[0; 32] {
        Err(EnrollmentError::Policy)
    } else {
        Ok(())
    }
}

fn validate_snapshot(
    snapshot: &PolicySnapshot,
    query: &PolicyQuery,
    principal: &Principal,
) -> Result<(), EnrollmentError> {
    let actual = snapshot.record.anchor();
    let expected = query.expected();
    if snapshot.record.local_principal() != principal.fingerprint()
        || actual.parts().0 != expected.parts().0
    {
        return Err(EnrollmentError::Binding);
    }
    if actual.parts().1 < expected.parts().1
        || (actual.parts().1 == expected.parts().1 && actual != expected)
    {
        return Err(EnrollmentError::Rollback);
    }
    if let PolicyQuery::Refresh {
        policy_commitment, ..
    } = query
    {
        if actual.parts().1 == expected.parts().1 && snapshot.commitment() != *policy_commitment {
            return Err(EnrollmentError::Rollback);
        }
    }
    if let PolicyQuery::Enroll { expected, policy } = query {
        if expected.parts().1 != 0
            || actual.parts().1 != 1
            || !snapshot.record.enabled()
            || snapshot.record.peer().principal() != policy.peer.principal()
            || snapshot.owner != policy.owner
            || snapshot.resource_policy != policy.resource_policy
        {
            return Err(EnrollmentError::Binding);
        }
    }
    Ok(())
}

fn valid_options(message: ParsedMessage<'_>, request: bool) -> bool {
    let mut paths = 0;
    let mut formats = 0;
    for option in message.options() {
        match option.number() {
            OptionNumber::URI_PATH if request && option.value() == POLICY_PATH.as_bytes() => {
                paths += 1
            }
            OptionNumber::CONTENT_FORMAT if option.value() == [42] => formats += 1,
            _ => return false,
        }
    }
    paths == usize::from(request) && formats == 1
}

fn encode_request(principal: Principal, query: &PolicyQuery, nonce: [u8; 32]) -> [u8; REQUEST_LEN] {
    let mut out = [0; REQUEST_LEN];
    out[..4].copy_from_slice(b"CPQ1");
    out[4] = u8::from(matches!(query, PolicyQuery::Enroll { .. }));
    out[5..37].copy_from_slice(&nonce);
    out[37..69].copy_from_slice(principal.fingerprint());
    encode_anchor(query.expected(), &mut out[69..141]);
    if let PolicyQuery::Enroll { policy, .. } = query {
        out[141..206].copy_from_slice(&policy.peer.public_key());
        out[206] = policy.peer.kid();
        out[207..239].copy_from_slice(&policy.owner);
        out[239..271].copy_from_slice(&policy.resource_policy);
    } else if let PolicyQuery::Refresh {
        policy_commitment, ..
    } = query
    {
        out[141..173].copy_from_slice(policy_commitment);
    }
    out
}

fn decode_request(bytes: &[u8]) -> Result<([u8; 32], PolicyQuery, [u8; 32]), EnrollmentError> {
    if bytes.len() != REQUEST_LEN || &bytes[..4] != b"CPQ1" {
        return Err(EnrollmentError::Message);
    }
    let nonce = array(&bytes[5..37]);
    if nonce == [0; 32] {
        return Err(EnrollmentError::Entropy);
    }
    let local = array(&bytes[37..69]);
    let expected = decode_anchor(&bytes[69..141])?;
    let query = match bytes[4] {
        0 if bytes[173..].iter().all(|byte| *byte == 0)
            && (expected.parts().1 == 0) == bytes[141..173].iter().all(|byte| *byte == 0) =>
        {
            PolicyQuery::Refresh {
                expected,
                policy_commitment: array(&bytes[141..173]),
            }
        }
        1 if expected.parts().1 == 0 => PolicyQuery::Enroll {
            expected,
            policy: EnrollmentPolicy::new(
                PinnedPeer::from_public_key(&bytes[141..206], bytes[206])
                    .map_err(|_| EnrollmentError::Message)?,
                array(&bytes[207..239]),
                array(&bytes[239..271]),
            )?,
        },
        _ => return Err(EnrollmentError::Message),
    };
    Ok((local, query, nonce))
}

fn encode_response(
    nonce: [u8; 32],
    digest: [u8; 32],
    snapshot: &PolicySnapshot,
) -> [u8; RESPONSE_LEN] {
    let mut out = [0; RESPONSE_LEN];
    out[..4].copy_from_slice(b"CPS1");
    out[4..36].copy_from_slice(&nonce);
    out[36..68].copy_from_slice(&digest);
    let parts = snapshot.record.parts();
    out[68] = parts.version;
    out[69..101].copy_from_slice(&parts.store_id);
    out[101..109].copy_from_slice(&parts.generation.to_be_bytes());
    out[109..141].copy_from_slice(&parts.local_principal);
    out[141..206].copy_from_slice(&parts.peer_public_key);
    out[206] = parts.peer_kid;
    out[207] = u8::from(parts.enabled);
    out[208..240].copy_from_slice(&snapshot.owner);
    out[240..272].copy_from_slice(&snapshot.resource_policy);
    out
}

fn decode_response(bytes: &[u8]) -> Result<([u8; 32], [u8; 32], PolicySnapshot), EnrollmentError> {
    if bytes.len() != RESPONSE_LEN || &bytes[..4] != b"CPS1" || bytes[207] > 1 {
        return Err(EnrollmentError::Message);
    }
    let record = TrustRecord::from_parts(TrustRecordParts {
        version: bytes[68],
        store_id: array(&bytes[69..101]),
        generation: u64::from_be_bytes(array(&bytes[101..109])),
        local_principal: array(&bytes[109..141]),
        peer_public_key: array(&bytes[141..206]),
        peer_kid: bytes[206],
        enabled: bytes[207] == 1,
    })
    .map_err(EnrollmentError::Trust)?;
    let snapshot = PolicySnapshot::new(record, array(&bytes[208..240]), array(&bytes[240..272]))?;
    Ok((array(&bytes[4..36]), array(&bytes[36..68]), snapshot))
}

fn encode_anchor(anchor: TrustAnchor, out: &mut [u8]) {
    let (store, generation, digest) = anchor.parts();
    out[..32].copy_from_slice(&store);
    out[32..40].copy_from_slice(&generation.to_be_bytes());
    out[40..72].copy_from_slice(&digest);
}

fn decode_anchor(bytes: &[u8]) -> Result<TrustAnchor, EnrollmentError> {
    TrustAnchor::from_parts(
        array(&bytes[..32]),
        u64::from_be_bytes(array(&bytes[32..40])),
        array(&bytes[40..72]),
    )
    .map_err(EnrollmentError::Trust)
}

fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(bytes);
    out
}

macro_rules! redacted_debug {
    ($($ty:ident),+) => {$ (
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}

redacted_debug!(
    AuthorityConnection,
    AuthorityResponder,
    PendingPolicy,
    ProtectedPolicyResponse,
    VerifiedPolicy
);

#[cfg(test)]
#[path = "enrollment_tests.rs"]
mod tests;
