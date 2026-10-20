//! Durable local pairing for devices under the operator's physical custody.
//!
//! The commissioning caller authenticates the operator and the local transport.
//! Storage must preserve complete records with serialized compare-and-swap and
//! confirm its durable boundary before returning success. This profile survives
//! ordinary restart; it does not detect rollback of an entire flash/disk image.
//! Use the separate protected-anchor APIs when that threat is in scope.
//!
//! Keep identity custody separate. Persist no EDHOC/OSCORE session or nonce state;
//! recover identity and this record, then perform a fresh authenticated handshake.
//! Stop and discard all live session/application owners before changing pairing.

use core::fmt;

use super::{EnrollmentPolicy, Identity, PinnedPeer, Principal};

/// Maximum encoded public pairing record or commissioning challenge.
const CAPACITY: usize = PairingRecord::MAX_ENCODED_LEN;

/// Refusal before exposing or changing local pairing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingError {
    /// Invalid, noncanonical, oversized or internally inconsistent CBOR v1.
    Record,
    /// Identity, peer or request binding differs.
    Binding,
    /// No fresh challenge, failed entropy, expired deadline or clock regression.
    Challenge,
    /// The current owner differs, is absent, or a transfer does not change owner.
    Owner,
    /// Access is disabled, unpaired or blocked by an uncertain write.
    Unauthorized,
    /// No greater revision is representable.
    Exhausted,
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "local pairing refused: {self:?}")
    }
}
impl core::error::Error for PairingError {}

/// A failed or ambiguous write blocks the live pairing until verified recovery.
#[derive(Debug)]
pub enum PairingCommitError<E> {
    /// Validation failed before storage.
    State(PairingError),
    /// Storage could not confirm the complete durable transition.
    Store(E),
}

impl<E: fmt::Display> fmt::Display for PairingCommitError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => error.fmt(f),
            Self::Store(error) => write!(f, "pairing storage failed: {error}"),
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for PairingCommitError<E> {}

/// Public durable policy, bound to one independently stored device identity.
/// CBOR contains no private key, bearer credential or session material.
#[derive(Clone, Debug)]
pub struct PairingRecord {
    store: [u8; 32],
    revision: u64,
    local: [u8; 32],
    policy: Option<EnrollmentPolicy>,
    enabled: bool,
    recovery_owner: Option<[u8; 32]>,
}

impl PairingRecord {
    /// Maximum canonical record size, including one recovery credential.
    pub const MAX_ENCODED_LEN: usize = 251;
    /// Explicit first setup only. Missing/corrupt previously initialized storage
    /// never authorizes calling this automatically.
    pub fn unpaired(identity: &Identity, store: [u8; 32]) -> Result<Self, PairingError> {
        if store == [0; 32] {
            return Err(PairingError::Record);
        }
        Ok(Self {
            store,
            revision: 1,
            local: *identity.peer().principal().fingerprint(),
            policy: None,
            enabled: false,
            recovery_owner: None,
        })
    }

    /// Stable local store identifier.
    #[must_use]
    pub const fn store(&self) -> &[u8; 32] {
        &self.store
    }

    /// Full device credential fingerprint bound into this public record.
    #[must_use]
    pub const fn local_principal(&self) -> &[u8; 32] {
        &self.local
    }

    /// Advances on grant, peer/policy change, revoke and regrant.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Owner and permissions remain present in a revocation tombstone.
    #[must_use]
    pub const fn policy(&self) -> Option<&EnrollmentPolicy> {
        self.policy.as_ref()
    }

    /// Whether the installed policy currently grants access.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Optional preauthorized recovery credential; grants no service access.
    #[must_use]
    pub const fn recovery_owner(&self) -> Option<&[u8; 32]> {
        self.recovery_owner.as_ref()
    }

    /// Check a candidate originating grant for receipt-only reconciliation.
    /// The current record must be a disabled tombstone for the same identity,
    /// store, owner, service and permissions. This does not authenticate history:
    /// the retained operation must authenticate the prior record's commitment.
    pub fn validate_reconciliation_prior(&self, prior: &Self) -> Result<(), PairingError> {
        let current = self.policy.as_ref().ok_or(PairingError::Binding)?;
        let previous = prior.policy.as_ref().ok_or(PairingError::Binding)?;
        if self.enabled
            || !prior.enabled
            || prior.revision >= self.revision
            || prior.store != self.store
            || prior.local != self.local
            || current.peer().principal() != previous.peer().principal()
            || current.owner() != previous.owner()
            || current.resource_policy() != previous.resource_policy()
        {
            return Err(PairingError::Binding);
        }
        Ok(())
    }

    /// Complete record commitment for serialized compare-and-swap, not rollback proof.
    #[must_use]
    pub fn commitment(&self) -> [u8; 32] {
        let mut bytes = [0; CAPACITY];
        let len = self
            .encode(&mut bytes)
            .expect("validated pairing record fits");
        blake3::derive_key("coaptic local pairing record v1", &bytes[..len])
    }

    /// Encode the fixed public CBOR v1 record without allocating.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, PairingError> {
        let mut out = Writer {
            bytes: output,
            pos: 0,
        };
        // Preserve commitments of deployed records with no recovery credential.
        out.raw(&[
            if self.recovery_owner.is_some() {
                0x8a
            } else {
                0x89
            },
            1,
        ])?;
        out.bytes(&self.store)?;
        out.uint(self.revision)?;
        out.bytes(&self.local)?;
        if let Some(policy) = &self.policy {
            out.bytes(&policy.peer().public_key())?;
            out.uint(u64::from(policy.peer().kid()))?;
            out.bytes(policy.owner())?;
            out.bytes(policy.resource_policy())?;
        } else {
            out.raw(&[0xf6, 0])?;
            out.bytes(&[0; 32])?;
            out.bytes(&[0; 32])?;
        }
        out.raw(&[if self.enabled { 0xf5 } else { 0xf4 }])?;
        if let Some(owner) = &self.recovery_owner {
            out.bytes(owner)?;
        }
        Ok(out.pos)
    }

    /// Validate an entire bounded canonical CBOR record. This validates its
    /// representation; the caller still authenticates storage and physical custody.
    pub fn decode(input: &[u8]) -> Result<Self, PairingError> {
        let mut read = Reader::new(input)?;
        let recovery = match read.take(1)?[0] {
            0x89 => false,
            0x8a => true,
            _ => return Err(PairingError::Record),
        };
        read.exact(&[1])?;
        let store = read.bytes()?;
        let revision = read.uint()?;
        let local = read.bytes()?;
        let public = if read.peek() == Some(0xf6) {
            read.exact(&[0xf6])?;
            None
        } else {
            Some(read.bytes::<65>()?)
        };
        let kid: u8 = read.uint()?.try_into().map_err(|_| PairingError::Record)?;
        let owner = read.bytes::<32>()?;
        let permissions = read.bytes::<32>()?;
        let enabled = read.boolean()?;
        let recovery_owner = if recovery {
            Some(read.bytes::<32>()?)
        } else {
            None
        };
        read.finish()?;
        let policy = match public {
            Some(public) => {
                if revision < 2 {
                    return Err(PairingError::Record);
                }
                Some(
                    EnrollmentPolicy::new(
                        PinnedPeer::from_public_key(&public, kid)
                            .map_err(|_| PairingError::Record)?,
                        owner,
                        permissions,
                    )
                    .map_err(|_| PairingError::Record)?,
                )
            }
            None => {
                if revision != 1
                    || enabled
                    || kid != 0
                    || owner != [0; 32]
                    || permissions != [0; 32]
                    || recovery_owner.is_some()
                {
                    return Err(PairingError::Record);
                }
                None
            }
        };
        if store == [0; 32] || local == [0; 32] || revision == 0 {
            return Err(PairingError::Record);
        }
        if recovery_owner.as_ref().is_some_and(|backup| {
            *backup == [0; 32]
                || policy
                    .as_ref()
                    .is_none_or(|policy| policy.owner() == backup)
        }) {
            return Err(PairingError::Record);
        }
        Ok(Self {
            store,
            revision,
            local,
            policy,
            enabled,
            recovery_owner,
        })
    }
}

/// One exact, short-lived public commissioning request from the device.
#[derive(Clone, Debug)]
pub struct PairingChallenge {
    device: PinnedPeer,
    store: [u8; 32],
    revision: u64,
    digest: [u8; 32],
    nonce: [u8; 32],
}

impl PairingChallenge {
    /// Device credential read through the authenticated local commissioning path.
    #[must_use]
    pub const fn device(&self) -> &PinnedPeer {
        &self.device
    }
    /// Exact expected store/revision/record commitment.
    #[must_use]
    pub const fn expected(&self) -> ([u8; 32], u64, [u8; 32]) {
        (self.store, self.revision, self.digest)
    }
    /// Fresh device nonce, never reused after failure or restart.
    #[must_use]
    pub const fn nonce(&self) -> &[u8; 32] {
        &self.nonce
    }

    /// Encode the public challenge as bounded CBOR v1.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, PairingError> {
        let mut out = Writer {
            bytes: output,
            pos: 0,
        };
        out.raw(&[0x87, 1])?;
        out.bytes(&self.device.public_key())?;
        out.uint(u64::from(self.device.kid()))?;
        out.bytes(&self.store)?;
        out.uint(self.revision)?;
        out.bytes(&self.digest)?;
        out.bytes(&self.nonce)?;
        Ok(out.pos)
    }

    /// Decode representation only; the transport must authenticate the device.
    pub fn decode(input: &[u8]) -> Result<Self, PairingError> {
        let mut read = Reader::new(input)?;
        read.exact(&[0x87, 1])?;
        let key = read.bytes::<65>()?;
        let kid = read.uint()?.try_into().map_err(|_| PairingError::Record)?;
        let store = read.bytes()?;
        let revision = read.uint()?;
        let digest = read.bytes()?;
        let nonce = read.bytes()?;
        read.finish()?;
        if store == [0; 32] || revision == 0 || nonce == [0; 32] {
            return Err(PairingError::Record);
        }
        Ok(Self {
            device: PinnedPeer::from_public_key(&key, kid).map_err(|_| PairingError::Record)?,
            store,
            revision,
            digest,
            nonce,
        })
    }
}

/// Live local policy. No session survives its replacement or process restart.
pub struct LocalPairing {
    record: PairingRecord,
    pending: Option<([u8; 32], u64, u64)>,
    blocked: bool,
}

/// Exact policy/action delivered by an authenticated local commissioning host.
/// This is public data, not a signature or bearer authorization. Accept only on
/// the privileged local commissioning channel after the host verifies approval.
#[derive(Clone, Debug)]
pub struct PairingUpdate {
    challenge: PairingChallenge,
    policy: EnrollmentPolicy,
    revoke: bool,
    current_owner: Option<[u8; 32]>,
    recovery_action: u8,
}

impl PairingUpdate {
    /// Bind a grant or revocation to one live device challenge and exact policy.
    #[must_use]
    pub const fn new(challenge: PairingChallenge, policy: EnrollmentPolicy, revoke: bool) -> Self {
        Self {
            challenge,
            policy,
            revoke,
            current_owner: None,
            recovery_action: 0,
        }
    }

    /// Bind an explicit transfer to the authenticated current owner's approval.
    /// Both owners, the new peer/permissions and the complete live challenge
    /// must be covered by that approval. Owner bytes alone authenticate nothing.
    pub fn transfer(
        challenge: PairingChallenge,
        current_owner: [u8; 32],
        policy: EnrollmentPolicy,
    ) -> Result<Self, PairingError> {
        if current_owner == [0; 32] || &current_owner == policy.owner() {
            return Err(PairingError::Owner);
        }
        Ok(Self {
            challenge,
            policy,
            revoke: false,
            current_owner: Some(current_owner),
            recovery_action: 0,
        })
    }

    /// Set or remove the recovery credential after current-owner approval and,
    /// when installing one, proof of possession by that distinct credential.
    /// Both proofs must cover this exact challenge, policy and recovery owner.
    pub fn set_recovery(
        challenge: PairingChallenge,
        policy: EnrollmentPolicy,
        recovery_owner: Option<[u8; 32]>,
    ) -> Result<Self, PairingError> {
        if recovery_owner
            .as_ref()
            .is_some_and(|owner| *owner == [0; 32] || owner == policy.owner())
        {
            return Err(PairingError::Owner);
        }
        Ok(Self {
            challenge,
            policy,
            revoke: false,
            current_owner: recovery_owner,
            recovery_action: 1,
        })
    }

    /// Recover through the previously authorized credential and a destination
    /// owner proof. Both approvals must cover the same complete live challenge
    /// and destination policy. This metadata itself authenticates neither proof.
    pub fn recover(
        challenge: PairingChallenge,
        recovery_owner: [u8; 32],
        policy: EnrollmentPolicy,
    ) -> Result<Self, PairingError> {
        if recovery_owner == [0; 32] || &recovery_owner == policy.owner() {
            return Err(PairingError::Owner);
        }
        Ok(Self {
            challenge,
            policy,
            revoke: false,
            current_owner: Some(recovery_owner),
            recovery_action: 2,
        })
    }

    /// Whether this changes recovery authorization without changing service policy.
    #[must_use]
    pub const fn is_recovery_configuration(&self) -> bool {
        self.recovery_action == 1
    }

    /// Whether this consumes the installed recovery credential.
    #[must_use]
    pub const fn is_recovery(&self) -> bool {
        self.recovery_action == 2
    }

    /// Public device challenge bound by the operator approval.
    #[must_use]
    pub const fn challenge(&self) -> &PairingChallenge {
        &self.challenge
    }
    /// Exact peer, owner and permissions selected by the operator.
    #[must_use]
    pub const fn policy(&self) -> &EnrollmentPolicy {
        &self.policy
    }
    /// Whether this update disables the existing policy.
    #[must_use]
    pub const fn is_revocation(&self) -> bool {
        self.revoke
    }

    /// Current owner releasing the device, only for an explicit transfer.
    /// The policy names the destination owner. This metadata is not a proof.
    #[must_use]
    pub const fn current_owner(&self) -> Option<&[u8; 32]> {
        if self.recovery_action == 0 {
            self.current_owner.as_ref()
        } else {
            None
        }
    }

    /// Encode at most 384 bytes of canonical application CBOR v1.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, PairingError> {
        let mut challenge = [0; 224];
        let len = self.challenge.encode(&mut challenge)?;
        let mut out = Writer {
            bytes: output,
            pos: 0,
        };
        if self.recovery_action != 0 {
            out.raw(&[0x88, 1, self.recovery_action + 2])?;
        } else if self.current_owner.is_some() {
            out.raw(&[0x88, 1, 2])?;
        } else {
            out.raw(&[0x87, 1, u8::from(self.revoke)])?;
        }
        out.bytes(&challenge[..len])?;
        out.bytes(&self.policy.peer().public_key())?;
        out.uint(u64::from(self.policy.peer().kid()))?;
        out.bytes(self.policy.owner())?;
        out.bytes(self.policy.resource_policy())?;
        if let Some(owner) = &self.current_owner {
            out.bytes(owner)?;
        } else if self.recovery_action == 1 {
            out.raw(&[0xf6])?;
        }
        Ok(out.pos)
    }

    /// Validate the complete bounded update. Transport authentication is external.
    pub fn decode(input: &[u8]) -> Result<Self, PairingError> {
        if input.len() > 384 {
            return Err(PairingError::Record);
        }
        let mut read = Reader {
            bytes: input,
            pos: 0,
        };
        let array = read.take(1)?[0];
        read.exact(&[1])?;
        let action = match (array, read.uint()?) {
            (0x87, action @ 0..=1) | (0x88, action @ 2..=4) => action,
            _ => return Err(PairingError::Record),
        };
        read.exact(&[0x58])?;
        let len = usize::from(read.take(1)?[0]);
        if len < 24 {
            return Err(PairingError::Record);
        }
        let challenge = PairingChallenge::decode(read.take(len)?)?;
        let peer = read.bytes::<65>()?;
        let kid = read.uint()?.try_into().map_err(|_| PairingError::Record)?;
        let owner = read.bytes()?;
        let permissions = read.bytes()?;
        let current_owner = if action == 3 && read.peek() == Some(0xf6) {
            read.exact(&[0xf6])?;
            None
        } else if action >= 2 {
            Some(read.bytes()?)
        } else {
            None
        };
        read.finish()?;
        let policy = EnrollmentPolicy::new(
            PinnedPeer::from_public_key(&peer, kid).map_err(|_| PairingError::Record)?,
            owner,
            permissions,
        )
        .map_err(|_| PairingError::Record)?;
        match action {
            2 => Self::transfer(challenge, current_owner.ok_or(PairingError::Owner)?, policy),
            3 => Self::set_recovery(challenge, policy, current_owner),
            4 => Self::recover(challenge, current_owner.ok_or(PairingError::Owner)?, policy),
            _ => Ok(Self::new(challenge, policy, action == 1)),
        }
    }

    /// Consume through the exact authenticated local management path.
    pub fn apply<E>(
        &self,
        state: &mut LocalPairing,
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        if self.recovery_action == 1 {
            return state.set_recovery(
                &self.challenge,
                &self.policy,
                self.current_owner,
                now_ms,
                commit,
            );
        }
        if self.recovery_action == 2 {
            return state.recover(
                &self.challenge,
                self.current_owner
                    .as_ref()
                    .ok_or(PairingCommitError::State(PairingError::Owner))?,
                self.policy.clone(),
                now_ms,
                commit,
            );
        }
        if let Some(owner) = &self.current_owner {
            return state.transfer(&self.challenge, owner, self.policy.clone(), now_ms, commit);
        }
        if self.revoke {
            if state.record.policy.as_ref().is_none_or(|old| {
                old.peer().principal() != self.policy.peer().principal()
                    || old.owner() != self.policy.owner()
                    || old.resource_policy() != self.policy.resource_policy()
            }) {
                state.pending = None;
                return Err(PairingCommitError::State(PairingError::Binding));
            }
            state.revoke(&self.challenge, self.policy.owner(), now_ms, commit)
        } else {
            state.pair(&self.challenge, self.policy.clone(), now_ms, commit)
        }
    }
}

impl LocalPairing {
    /// Restore verified local storage and confirm the independently recovered key.
    pub fn restore(identity: &Identity, record: PairingRecord) -> Result<Self, PairingError> {
        if record.local != *identity.peer().principal().fingerprint()
            || record
                .policy
                .as_ref()
                .is_some_and(|policy| policy.peer().public_x() == identity.public_x())
        {
            return Err(PairingError::Binding);
        }
        Ok(Self {
            record,
            pending: None,
            blocked: false,
        })
    }

    /// Complete public durable state, including a disabled tombstone.
    #[must_use]
    pub const fn checkpoint(&self) -> &PairingRecord {
        &self.record
    }

    /// Read the currently enabled peer. Refuses before pair, after revoke and
    /// after a failed/ambiguous commit.
    pub fn peer(&self) -> Result<&PinnedPeer, PairingError> {
        if self.blocked || !self.record.enabled {
            return Err(PairingError::Unauthorized);
        }
        self.record
            .policy
            .as_ref()
            .map(EnrollmentPolicy::peer)
            .ok_or(PairingError::Unauthorized)
    }

    /// Check before every bootstrap/application poll, including cached responses.
    #[must_use]
    pub fn permits(&self, principal: &Principal) -> bool {
        self.peer().is_ok_and(|peer| peer.principal() == *principal)
    }

    pub(crate) fn trust_record(
        &self,
        identity: &Identity,
    ) -> Result<super::TrustRecord, super::TrustError> {
        if self.record.local != *identity.peer().principal().fingerprint() {
            return Err(super::TrustError::IdentityMismatch);
        }
        let peer = self.peer().map_err(|_| super::TrustError::Unauthorized)?;
        super::TrustRecord::from_parts(super::TrustRecordParts {
            version: 1,
            store_id: self.record.store,
            generation: self.record.revision,
            local_principal: self.record.local,
            peer_public_key: peer.public_key(),
            peer_kid: peer.kid(),
            enabled: true,
        })
    }

    /// Start a new commissioning attempt. Replaces any earlier pending attempt.
    /// The caller authenticates the operator before applying its exact result.
    pub fn begin(
        &mut self,
        identity: &Identity,
        now_ms: u64,
        mut entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<PairingChallenge, PairingError> {
        self.pending = None;
        if self.blocked || self.record.local != *identity.peer().principal().fingerprint() {
            return Err(PairingError::Binding);
        }
        let deadline = now_ms.checked_add(60_000).ok_or(PairingError::Challenge)?;
        let mut nonce = [0; 32];
        if !entropy(&mut nonce) || nonce == [0; 32] {
            return Err(PairingError::Challenge);
        }
        self.pending = Some((nonce, now_ms, deadline));
        Ok(PairingChallenge {
            device: identity.peer(),
            store: self.record.store,
            revision: self.record.revision,
            digest: self.record.commitment(),
            nonce,
        })
    }

    /// Consume an authenticated operator grant for this exact live challenge.
    /// Stop/discard all old sessions before calling. `commit` must compare the
    /// complete prior record and confirm durable storage before returning success.
    pub fn pair<E>(
        &mut self,
        challenge: &PairingChallenge,
        policy: EnrollmentPolicy,
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        self.check(challenge, now_ms)
            .map_err(PairingCommitError::State)?;
        if challenge.device.public_x() == policy.peer().public_x() {
            return Err(PairingCommitError::State(PairingError::Binding));
        }
        if self
            .record
            .policy
            .as_ref()
            .is_some_and(|old| old.owner() != policy.owner())
        {
            return Err(PairingCommitError::State(PairingError::Owner));
        }
        let mut next = self.record.clone();
        next.policy = Some(policy);
        next.enabled = true;
        self.commit(next, commit)
    }

    /// Consume an authenticated owner revocation. The tombstone retains ownership.
    pub fn revoke<E>(
        &mut self,
        challenge: &PairingChallenge,
        owner: &[u8; 32],
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        self.check(challenge, now_ms)
            .map_err(PairingCommitError::State)?;
        if self
            .record
            .policy
            .as_ref()
            .is_none_or(|old| old.owner() != owner)
        {
            return Err(PairingCommitError::State(PairingError::Owner));
        }
        let mut next = self.record.clone();
        next.enabled = false;
        self.commit(next, commit)
    }

    /// Transfer to an explicitly approved new owner, peer and permissions.
    /// Authenticate `current_owner` on the privileged local channel and bind its
    /// approval to this complete challenge and destination policy. The current
    /// claim remains authoritative after revocation; ordinary `pair` cannot
    /// transfer it. Stop/discard every old session and App before calling, then
    /// establish fresh sessions from the committed policy. This never resets
    /// the device identity, store or revision, and grants no rollback protection.
    pub fn transfer<E>(
        &mut self,
        challenge: &PairingChallenge,
        current_owner: &[u8; 32],
        policy: EnrollmentPolicy,
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        self.check(challenge, now_ms)
            .map_err(PairingCommitError::State)?;
        if self
            .record
            .policy
            .as_ref()
            .is_none_or(|old| old.owner() != current_owner)
            || policy.owner() == current_owner
        {
            return Err(PairingCommitError::State(PairingError::Owner));
        }
        if challenge.device.public_x() == policy.peer().public_x() {
            return Err(PairingCommitError::State(PairingError::Binding));
        }
        let mut next = self.record.clone();
        next.policy = Some(policy);
        next.enabled = true;
        next.recovery_owner = None;
        self.commit(next, commit)
    }

    /// Configure one recovery credential with authenticated owner approval and
    /// backup proof of possession. The exact existing service policy must match;
    /// this operation cannot activate revoked access or change its permissions.
    pub fn set_recovery<E>(
        &mut self,
        challenge: &PairingChallenge,
        policy: &EnrollmentPolicy,
        recovery_owner: Option<[u8; 32]>,
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        self.check(challenge, now_ms)
            .map_err(PairingCommitError::State)?;
        if self.record.policy.as_ref().is_none_or(|old| {
            old.owner() != policy.owner()
                || old.peer().principal() != policy.peer().principal()
                || old.resource_policy() != policy.resource_policy()
        }) || recovery_owner
            .as_ref()
            .is_some_and(|owner| *owner == [0; 32] || owner == policy.owner())
        {
            return Err(PairingCommitError::State(PairingError::Owner));
        }
        let mut next = self.record.clone();
        next.recovery_owner = recovery_owner;
        self.commit(next, commit)
    }

    /// Consume a preauthorized recovery credential and destination owner proof.
    /// The authenticated host must verify both exact approvals. The storage
    /// callback must refuse unresolved durable effects before committing a new
    /// grant. Recovery preserves device identity and clears backup authorization.
    pub fn recover<E>(
        &mut self,
        challenge: &PairingChallenge,
        recovery_owner: &[u8; 32],
        policy: EnrollmentPolicy,
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        self.check(challenge, now_ms)
            .map_err(PairingCommitError::State)?;
        if self.record.recovery_owner.as_ref() != Some(recovery_owner)
            || recovery_owner == policy.owner()
            || self
                .record
                .policy
                .as_ref()
                .is_none_or(|old| old.owner() == policy.owner())
        {
            return Err(PairingCommitError::State(PairingError::Owner));
        }
        if challenge.device.public_x() == policy.peer().public_x() {
            return Err(PairingCommitError::State(PairingError::Binding));
        }
        let mut next = self.record.clone();
        next.policy = Some(policy);
        next.enabled = true;
        next.recovery_owner = None;
        self.commit(next, commit)
    }

    /// Consume current-owner approval for an existing receipt, without changing
    /// this disabled policy or its revision. The privileged caller must bind
    /// approval to the full challenge, prior commitment and exact receipt.
    /// `reconcile` must authenticate originating work against that prior commitment,
    /// or an exact already-completed operation and receipt under this tombstone.
    /// The latter proves completion, not original-grant history. Accept only an
    /// existing service receipt and confirm durable readback.
    /// A failed or ambiguous callback poisons this live owner until recovery.
    pub fn reconcile<E>(
        &mut self,
        challenge: &PairingChallenge,
        prior: &PairingRecord,
        now_ms: u64,
        reconcile: impl FnOnce(&PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        self.check(challenge, now_ms)
            .map_err(PairingCommitError::State)?;
        self.record
            .validate_reconciliation_prior(prior)
            .map_err(PairingCommitError::State)?;
        self.blocked = true;
        reconcile(&self.record).map_err(PairingCommitError::Store)?;
        self.blocked = false;
        Ok(())
    }

    fn check(&mut self, challenge: &PairingChallenge, now_ms: u64) -> Result<(), PairingError> {
        let pending = self.pending.take().ok_or(PairingError::Challenge)?;
        if self.blocked || now_ms < pending.1 || now_ms >= pending.2 || pending.0 != challenge.nonce
        {
            return Err(PairingError::Challenge);
        }
        if challenge.store != self.record.store
            || challenge.revision != self.record.revision
            || challenge.digest != self.record.commitment()
            || challenge.device.principal().fingerprint() != &self.record.local
        {
            return Err(PairingError::Binding);
        }
        Ok(())
    }

    fn commit<E>(
        &mut self,
        mut next: PairingRecord,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), E>,
    ) -> Result<(), PairingCommitError<E>> {
        next.revision = self
            .record
            .revision
            .checked_add(1)
            .ok_or(PairingCommitError::State(PairingError::Exhausted))?;
        self.blocked = true;
        commit(&self.record, &next).map_err(PairingCommitError::Store)?;
        self.record = next;
        self.blocked = false;
        Ok(())
    }
}

pub(crate) struct Writer<'a> {
    pub(crate) bytes: &'a mut [u8],
    pub(crate) pos: usize,
}
impl Writer<'_> {
    pub(crate) fn raw(&mut self, bytes: &[u8]) -> Result<(), PairingError> {
        let end = self
            .pos
            .checked_add(bytes.len())
            .ok_or(PairingError::Record)?;
        self.bytes
            .get_mut(self.pos..end)
            .ok_or(PairingError::Record)?
            .copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }
    pub(crate) fn bytes(&mut self, value: &[u8]) -> Result<(), PairingError> {
        let len: u8 = value.len().try_into().map_err(|_| PairingError::Record)?;
        self.raw(&[0x58, len])?;
        self.raw(value)
    }
    pub(crate) fn uint(&mut self, value: u64) -> Result<(), PairingError> {
        match value {
            0..=23 => self.raw(&[value as u8]),
            24..=255 => self.raw(&[0x18, value as u8]),
            256..=65535 => {
                self.raw(&[0x19])?;
                self.raw(&(value as u16).to_be_bytes())
            }
            65536..=4294967295 => {
                self.raw(&[0x1a])?;
                self.raw(&(value as u32).to_be_bytes())
            }
            _ => {
                self.raw(&[0x1b])?;
                self.raw(&value.to_be_bytes())
            }
        }
    }
}

pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, PairingError> {
        if bytes.len() > CAPACITY {
            return Err(PairingError::Record);
        }
        Ok(Self { bytes, pos: 0 })
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], PairingError> {
        let end = self.pos.checked_add(len).ok_or(PairingError::Record)?;
        let result = self.bytes.get(self.pos..end).ok_or(PairingError::Record)?;
        self.pos = end;
        Ok(result)
    }
    pub(crate) fn exact(&mut self, value: &[u8]) -> Result<(), PairingError> {
        if self.take(value.len())? != value {
            return Err(PairingError::Record);
        }
        Ok(())
    }
    pub(crate) fn bytes<const N: usize>(&mut self) -> Result<[u8; N], PairingError> {
        self.exact(&[0x58, N as u8])?;
        self.take(N)?.try_into().map_err(|_| PairingError::Record)
    }
    fn boolean(&mut self) -> Result<bool, PairingError> {
        match self.take(1)?[0] {
            0xf4 => Ok(false),
            0xf5 => Ok(true),
            _ => Err(PairingError::Record),
        }
    }
    pub(crate) fn uint(&mut self) -> Result<u64, PairingError> {
        let first = self.take(1)?[0];
        if first <= 23 {
            return Ok(u64::from(first));
        }
        let (len, minimum) = match first {
            0x18 => (1, 24),
            0x19 => (2, 256),
            0x1a => (4, 65536),
            0x1b => (8, 4294967296),
            _ => return Err(PairingError::Record),
        };
        let value = self
            .take(len)?
            .iter()
            .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
        if value < minimum {
            return Err(PairingError::Record);
        }
        Ok(value)
    }
    pub(crate) fn finish(self) -> Result<(), PairingError> {
        if self.pos != self.bytes.len() {
            return Err(PairingError::Record);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_transfer_tests.rs"]
mod transfer_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(value: u8) -> Identity {
        Identity::from_private_key([value; 32], value).unwrap()
    }
    fn policy(peer: &Identity, owner: u8) -> EnrollmentPolicy {
        EnrollmentPolicy::new(peer.peer(), [owner; 32], [9; 32]).unwrap()
    }
    fn commit(_: &PairingRecord, _: &PairingRecord) -> Result<(), ()> {
        Ok(())
    }
    fn entropy(out: &mut [u8]) -> bool {
        out.fill(7);
        true
    }

    #[test]
    fn reconciliation_consumes_fresh_challenge_preserves_tombstone_and_poison_refuses() {
        let device = identity(1);
        let peer = identity(2);
        let mut state =
            LocalPairing::restore(&device, PairingRecord::unpaired(&device, [3; 32]).unwrap())
                .unwrap();
        let challenge = state.begin(&device, 0, entropy).unwrap();
        state.pair(&challenge, policy(&peer, 4), 1, commit).unwrap();
        let prior = state.checkpoint().clone();
        assert!(
            state
                .checkpoint()
                .validate_reconciliation_prior(&prior)
                .is_err()
        );
        let challenge = state.begin(&device, 2, entropy).unwrap();
        state.revoke(&challenge, &[4; 32], 3, commit).unwrap();
        let mut backup_changed = state.checkpoint().clone();
        backup_changed.recovery_owner = Some([8; 32]);
        assert!(backup_changed.validate_reconciliation_prior(&prior).is_ok());
        let tombstone = state.checkpoint().commitment();
        for variant in 0..6 {
            let mut wrong = prior.clone();
            match variant {
                0 => wrong.enabled = false,
                1 => wrong.revision = state.checkpoint().revision(),
                2 => wrong.store[0] ^= 1,
                3 => wrong.local[0] ^= 1,
                4 => wrong.policy = Some(policy(&identity(3), 4)),
                _ => wrong.policy = Some(policy(&peer, 5)),
            }
            assert!(
                state
                    .checkpoint()
                    .validate_reconciliation_prior(&wrong)
                    .is_err()
            );
        }
        let challenge = state.begin(&device, 4, entropy).unwrap();
        let mut calls = 0;
        state
            .reconcile(&challenge, &prior, 5, |_| {
                calls += 1;
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(state.checkpoint().commitment(), tombstone);
        assert!(!state.permits(&peer.peer().principal()));
        assert!(
            state
                .reconcile(&challenge, &prior, 6, |_| Ok::<_, ()>(()))
                .is_err()
        );
        let challenge = state.begin(&device, 7, entropy).unwrap();
        assert!(matches!(
            state.reconcile(&challenge, &prior, 8, |_| Err(())),
            Err(PairingCommitError::Store(()))
        ));
        assert!(state.begin(&device, 9, entropy).is_err());
        assert_eq!(state.checkpoint().commitment(), tombstone);
        let mut state = LocalPairing::restore(&device, state.checkpoint().clone()).unwrap();
        let challenge = state.begin(&device, 10, entropy).unwrap();
        assert!(
            state
                .reconcile::<()>(&challenge, &prior, 60_010, |_| panic!("expired callback"))
                .is_err()
        );
    }

    #[test]
    fn pair_restart_revoke_and_regrant_keep_identity_owner_and_permissions() {
        let device = identity(1);
        let peer = identity(2);
        let mut state =
            LocalPairing::restore(&device, PairingRecord::unpaired(&device, [3; 32]).unwrap())
                .unwrap();
        assert!(!state.permits(&peer.peer().principal()));
        let challenge = state.begin(&device, 0, entropy).unwrap();
        state.pair(&challenge, policy(&peer, 4), 1, commit).unwrap();
        assert!(state.permits(&peer.peer().principal()));
        let mut bytes = [0; CAPACITY];
        let len = state.checkpoint().encode(&mut bytes).unwrap();
        let mut state =
            LocalPairing::restore(&device, PairingRecord::decode(&bytes[..len]).unwrap()).unwrap();
        assert!(state.permits(&peer.peer().principal()));
        assert!(state.pair(&challenge, policy(&peer, 4), 1, commit).is_err());
        let challenge = state.begin(&device, 0, entropy).unwrap();
        state.revoke(&challenge, &[4; 32], 1, commit).unwrap();
        assert!(!state.permits(&peer.peer().principal()));
        let challenge = state.begin(&device, 2, entropy).unwrap();
        assert!(matches!(
            state.pair(&challenge, policy(&peer, 5), 3, commit),
            Err(PairingCommitError::State(PairingError::Owner))
        ));
        let challenge = state.begin(&device, 4, entropy).unwrap();
        state.pair(&challenge, policy(&peer, 4), 5, commit).unwrap();
        assert_eq!(state.checkpoint().revision(), 4);
    }

    #[test]
    fn uncertain_write_blocks_every_grant_and_requires_storage_recovery() {
        let device = identity(1);
        let peer = identity(2);
        let mut state =
            LocalPairing::restore(&device, PairingRecord::unpaired(&device, [3; 32]).unwrap())
                .unwrap();
        let challenge = state.begin(&device, 10, entropy).unwrap();
        assert!(matches!(
            state.pair(&challenge, policy(&peer, 4), 11, |_, _| Err(7)),
            Err(PairingCommitError::Store(7))
        ));
        assert!(!state.permits(&peer.peer().principal()));
        assert!(state.begin(&device, 12, entropy).is_err());
    }

    #[test]
    fn challenge_checks_deadline_clock_binding_and_exact_cbor() {
        let device = identity(1);
        let peer = identity(2);
        let record = PairingRecord::unpaired(&device, [3; 32]).unwrap();
        let mut bytes = [0; CAPACITY];
        let len = record.encode(&mut bytes).unwrap();
        assert_eq!(
            PairingRecord::decode(&bytes[..len]).unwrap().commitment(),
            record.commitment()
        );
        for end in 0..len {
            assert!(PairingRecord::decode(&bytes[..end]).is_err());
        }
        bytes[len] = 0;
        assert!(PairingRecord::decode(&bytes[..len + 1]).is_err());
        assert!(LocalPairing::restore(&peer, record.clone()).is_err());
        let mut state = LocalPairing::restore(&device, record).unwrap();
        for now in [9, 60_010] {
            let challenge = state.begin(&device, 10, entropy).unwrap();
            let len = challenge.encode(&mut bytes).unwrap();
            let decoded = PairingChallenge::decode(&bytes[..len]).unwrap();
            assert_eq!(decoded.nonce(), challenge.nonce());
            assert!(state.pair(&decoded, policy(&peer, 4), now, commit).is_err());
        }
        assert!(state.begin(&device, 0, |_| false).is_err());
        assert!(
            state
                .begin(&device, 0, |out| {
                    out.fill(0);
                    true
                })
                .is_err()
        );
    }
}
