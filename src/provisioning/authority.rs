//! Serialized enrollment and owner approval for one registered device.
//!
//! The operator authenticates registration and management calls independently
//! of device possession. A public label is only a lookup hint. Ordinary storage
//! is not a freshness authority: restore requires a separately current anchor,
//! and missing or uncertain state must never be reinitialized automatically.

use core::fmt;

use super::{
    AuthenticatedPolicyRequest, EnrollmentPolicy, OnlinePolicyAuthority, PinnedPeer, PolicyQuery,
    PolicySnapshot, Principal, TrustAnchor, TrustRecord, TrustRecordParts,
};

const DOMAIN: &str = "https://github.com/jeffglousher/coaptic enrollment authority record v1";

/// Exact authority record identity from an independently current provider.
/// This is public metadata, not an authentication token or rollback protection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnrollmentAnchor(TrustAnchor);

impl EnrollmentAnchor {
    /// Validate lossless metadata supplied by the trusted current-state provider.
    pub fn from_parts(
        store: [u8; 32],
        revision: u64,
        digest: [u8; 32],
    ) -> Result<Self, ApprovalError> {
        if revision == 0 {
            return Err(ApprovalError::Record);
        }
        TrustAnchor::from_parts(store, revision, digest)
            .map(Self)
            .map_err(|_| ApprovalError::Record)
    }

    /// Stable device store, management revision and complete record digest.
    #[must_use]
    pub const fn parts(self) -> ([u8; 32], u64, [u8; 32]) {
        self.0.parts()
    }
}

/// Public lossless storage fields; persist the entire record in one transaction.
#[derive(Clone, Debug)]
pub struct EnrollmentRecordParts {
    /// Storage version; currently one.
    pub version: u8,
    /// Independently registered full device credential.
    pub device: PinnedPeer,
    /// Confirmed never-initialized application trust store at registration.
    pub genesis: TrustAnchor,
    /// Management revision, advanced for every approval, revoke and enrollment.
    pub revision: u64,
    /// Exact claimed owner, application peer and permission commitment.
    pub policy: Option<EnrollmentPolicy>,
    /// Whether this owner currently approves application access.
    pub approved: bool,
    /// Committed application policy, including a disabled revocation tombstone.
    pub snapshot: Option<PolicySnapshot>,
}

/// Validated public management record; constructing it does not activate access.
#[derive(Clone, Debug)]
pub struct EnrollmentRecord(EnrollmentRecordParts);

impl EnrollmentRecord {
    /// Validate a persisted record before exact restoration against current state.
    pub fn from_parts(parts: EnrollmentRecordParts) -> Result<Self, ApprovalError> {
        if parts.version != 1 || parts.revision == 0 || parts.genesis.parts().1 != 0 {
            return Err(ApprovalError::Record);
        }
        if parts.approved && parts.policy.is_none() {
            return Err(ApprovalError::Record);
        }
        if let Some(policy) = &parts.policy {
            if parts.revision < 2 || policy.peer().public_x() == parts.device.public_x() {
                return Err(ApprovalError::Record);
            }
        }
        if let Some(snapshot) = &parts.snapshot {
            let policy = parts.policy.as_ref().ok_or(ApprovalError::Record)?;
            if snapshot.record().local_principal() != parts.device.principal().fingerprint()
                || snapshot.record().anchor().parts().0 != parts.genesis.parts().0
                || snapshot.record().peer().principal() != policy.peer().principal()
                || snapshot.record().enabled() != parts.approved
                || snapshot.owner() != policy.owner()
                || snapshot.resource_policy() != policy.resource_policy()
                || snapshot
                    .record()
                    .anchor()
                    .parts()
                    .1
                    .checked_add(2)
                    .is_none_or(|minimum| parts.revision < minimum)
            {
                return Err(ApprovalError::Record);
            }
        }
        Ok(Self(parts))
    }

    /// Lossless fields for caller-owned durable serialization.
    #[must_use]
    pub fn parts(&self) -> EnrollmentRecordParts {
        self.0.clone()
    }

    /// Complete registered device principal; endpoint metadata supplies no identity.
    #[must_use]
    pub fn device(&self) -> Principal {
        self.0.device.principal()
    }

    /// Exact management anchor, distinct from the application policy anchor.
    #[must_use]
    pub fn anchor(&self) -> EnrollmentAnchor {
        let mut hash = blake3::Hasher::new_derive_key(DOMAIN);
        hash.update(&[self.0.version]);
        hash.update(self.device().fingerprint());
        hash.update(&self.0.genesis.parts().0);
        hash.update(&self.0.revision.to_be_bytes());
        hash.update(&[u8::from(self.0.approved), u8::from(self.0.policy.is_some())]);
        if let Some(policy) = &self.0.policy {
            hash.update(policy.peer().principal().fingerprint());
            hash.update(policy.owner());
            hash.update(policy.resource_policy());
        }
        hash.update(&[u8::from(self.0.snapshot.is_some())]);
        if let Some(snapshot) = &self.0.snapshot {
            hash.update(&snapshot.commitment());
        }
        EnrollmentAnchor(
            TrustAnchor::from_parts(
                self.0.genesis.parts().0,
                self.0.revision,
                *hash.finalize().as_bytes(),
            )
            .expect("validated nonzero enrollment store"),
        )
    }
}

/// Enrollment or management refused before a storage transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalError {
    /// Invalid or internally inconsistent stored representation.
    Record,
    /// Restore or management used an anchor other than the exact current record.
    Anchor,
    /// The authenticated full device principal differs from registration.
    Device,
    /// A different owner already holds this registration; approval cannot transfer it.
    Owner,
    /// No current operator approval matches the exact enrollment request.
    Approval,
    /// No completed enrollment exists to refresh.
    NotEnrolled,
    /// The request names a different store, future generation or changed equal generation.
    Policy,
    /// Storage could not establish current state; verified restoration is required.
    Blocked,
    /// The next management revision or policy generation is not representable.
    Exhausted,
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "enrollment authority refused: {self:?}")
    }
}

impl core::error::Error for ApprovalError {}

/// Durable authority transaction failure.
#[derive(Debug)]
pub enum ApprovalCommitError<E> {
    /// Refused before writing storage.
    State(ApprovalError),
    /// Failed or ambiguous persistence; the live authority remains blocked.
    Store(E),
}

impl<E: fmt::Display> fmt::Display for ApprovalCommitError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => error.fmt(f),
            Self::Store(error) => write!(f, "enrollment authority storage failed: {error}"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ApprovalCommitError<E> {}

/// Serialized durable storage plus independently current record publication.
///
/// Atomically compare `expected`, preserve the full next record, and publish its
/// new anchor only after the declared durable boundary. `None` means a confirmed
/// never-created registration, never an absent or corrupt file. Return an error
/// for an ambiguous commit. A successful file write alone does not establish
/// freshness: protect the current anchor independently or quarantine/reconcile a
/// restored database against an authenticated current authority before restore.
/// Keep operator changes, policy resolution and application effects serialized.
pub trait EnrollmentStore {
    /// Storage unavailability, conflict or ambiguous outcome.
    type Error;

    /// Read the independently current anchor under authority serialization.
    /// A cached file digest is insufficient. Unavailability must return an error.
    fn current(&mut self) -> Result<EnrollmentAnchor, Self::Error>;

    /// Commit exactly one management transition and its current-state anchor.
    fn commit(
        &mut self,
        expected: Option<&EnrollmentAnchor>,
        next: &EnrollmentRecord,
        anchor: &EnrollmentAnchor,
    ) -> Result<(), Self::Error>;
}

/// One registered device's durable claim, approval and policy authority.
///
/// Registration and `approve`/`revoke` are trusted operator entry points; never
/// expose them directly to a device or unauthenticated UI. Only `resolve` accepts
/// a device operation, after EDHOC and OSCORE authenticated its full principal.
/// The first owner remains exclusive across revocation; ownership transfer needs
/// a separately authenticated protocol. This type never restores session keys.
pub struct EnrollmentAuthority<S> {
    record: EnrollmentRecord,
    store: S,
    blocked: bool,
}

impl<S: EnrollmentStore> EnrollmentAuthority<S> {
    /// Register an independently authenticated device against confirmed genesis.
    /// Persist registration before accepting any approval or device operation.
    /// The platform registry must enforce one authority per full device credential.
    pub fn register(
        device: PinnedPeer,
        genesis: TrustAnchor,
        mut store: S,
    ) -> Result<Self, ApprovalCommitError<S::Error>> {
        let record = EnrollmentRecord::from_parts(EnrollmentRecordParts {
            version: 1,
            device,
            genesis,
            revision: 1,
            policy: None,
            approved: false,
            snapshot: None,
        })
        .map_err(ApprovalCommitError::State)?;
        store
            .commit(None, &record, &record.anchor())
            .map_err(ApprovalCommitError::Store)?;
        Ok(Self {
            record,
            store,
            blocked: false,
        })
    }

    /// Restore only against a separately authenticated, independently current anchor.
    pub fn restore(
        record: EnrollmentRecord,
        current: EnrollmentAnchor,
        store: S,
    ) -> Result<Self, ApprovalError> {
        if record.anchor() != current {
            return Err(ApprovalError::Anchor);
        }
        Ok(Self {
            record,
            store,
            blocked: false,
        })
    }

    /// Last definitely committed public record, including while access is blocked.
    /// Its presence never establishes current approval or successful restoration.
    #[must_use]
    pub const fn checkpoint(&self) -> &EnrollmentRecord {
        &self.record
    }

    /// Approve the exact device's owner, service credential and permissions.
    /// Authenticate the operator independently before calling this method.
    pub fn approve(
        &mut self,
        expected: EnrollmentAnchor,
        policy: EnrollmentPolicy,
    ) -> Result<EnrollmentAnchor, ApprovalCommitError<S::Error>> {
        self.check(expected).map_err(ApprovalCommitError::State)?;
        if self
            .record
            .0
            .policy
            .as_ref()
            .is_some_and(|old| old.owner() != policy.owner())
        {
            return Err(ApprovalCommitError::State(ApprovalError::Owner));
        }
        let mut next = self.record.0.clone();
        next.approved = true;
        next.policy = Some(policy);
        if let Some(snapshot) = &next.snapshot {
            next.snapshot = Some(
                snapshot_for(
                    &next,
                    snapshot
                        .record()
                        .anchor()
                        .parts()
                        .1
                        .checked_add(1)
                        .ok_or(ApprovalCommitError::State(ApprovalError::Exhausted))?,
                )
                .map_err(ApprovalCommitError::State)?,
            );
        }
        self.commit(next)
    }

    /// Durably revoke access and pending approval while retaining the owner's claim.
    /// Regrant advances the policy generation, invalidating old grants and retries.
    pub fn revoke(
        &mut self,
        expected: EnrollmentAnchor,
    ) -> Result<EnrollmentAnchor, ApprovalCommitError<S::Error>> {
        self.check(expected).map_err(ApprovalCommitError::State)?;
        let mut next = self.record.0.clone();
        next.approved = false;
        if let Some(snapshot) = &next.snapshot {
            next.snapshot = Some(
                snapshot_for(
                    &next,
                    snapshot
                        .record()
                        .anchor()
                        .parts()
                        .1
                        .checked_add(1)
                        .ok_or(ApprovalCommitError::State(ApprovalError::Exhausted))?,
                )
                .map_err(ApprovalCommitError::State)?,
            );
        }
        self.commit(next)
    }

    fn check(&self, expected: EnrollmentAnchor) -> Result<(), ApprovalError> {
        if self.blocked {
            return Err(ApprovalError::Blocked);
        }
        if expected != self.record.anchor() {
            return Err(ApprovalError::Anchor);
        }
        Ok(())
    }

    fn commit(
        &mut self,
        mut next: EnrollmentRecordParts,
    ) -> Result<EnrollmentAnchor, ApprovalCommitError<S::Error>> {
        next.revision = self
            .record
            .0
            .revision
            .checked_add(1)
            .ok_or(ApprovalCommitError::State(ApprovalError::Exhausted))?;
        let record = EnrollmentRecord::from_parts(next).map_err(ApprovalCommitError::State)?;
        let anchor = record.anchor();
        self.blocked = true;
        self.store
            .commit(Some(&self.record.anchor()), &record, &anchor)
            .map_err(ApprovalCommitError::Store)?;
        self.record = record;
        self.blocked = false;
        Ok(anchor)
    }
}

impl<S: EnrollmentStore> OnlinePolicyAuthority for EnrollmentAuthority<S> {
    type Error = ApprovalCommitError<S::Error>;

    fn resolve(
        &mut self,
        request: &AuthenticatedPolicyRequest,
    ) -> Result<PolicySnapshot, Self::Error> {
        self.check(self.record.anchor())
            .map_err(ApprovalCommitError::State)?;
        if *request.principal() != self.record.device() {
            return Err(ApprovalCommitError::State(ApprovalError::Device));
        }
        // A stale replica cannot renew an online lease from its cached snapshot.
        self.blocked = true;
        let current = self.store.current().map_err(ApprovalCommitError::Store)?;
        if current != self.record.anchor() {
            return Err(ApprovalCommitError::State(ApprovalError::Anchor));
        }
        self.blocked = false;
        match request.query() {
            PolicyQuery::Enroll { expected, policy } => {
                let approved = self
                    .record
                    .0
                    .policy
                    .as_ref()
                    .filter(|_| self.record.0.approved && self.record.0.snapshot.is_none())
                    .ok_or(ApprovalCommitError::State(ApprovalError::Approval))?;
                if *expected != self.record.0.genesis
                    || policy.peer().principal() != approved.peer().principal()
                    || policy.owner() != approved.owner()
                    || policy.resource_policy() != approved.resource_policy()
                {
                    return Err(ApprovalCommitError::State(ApprovalError::Approval));
                }
                let mut next = self.record.0.clone();
                let snapshot = snapshot_for(&next, 1).map_err(ApprovalCommitError::State)?;
                next.snapshot = Some(snapshot.clone());
                self.commit(next)?;
                Ok(snapshot)
            }
            PolicyQuery::Refresh {
                expected,
                policy_commitment,
            } => {
                let snapshot = self
                    .record
                    .0
                    .snapshot
                    .as_ref()
                    .ok_or(ApprovalCommitError::State(ApprovalError::NotEnrolled))?;
                let actual = snapshot.record().anchor();
                if expected.parts().0 != actual.parts().0
                    || expected.parts().1 > actual.parts().1
                    || (expected.parts().1 == 0 && *policy_commitment != [0; 32])
                    || (expected.parts().1 == actual.parts().1
                        && (*expected != actual || *policy_commitment != snapshot.commitment()))
                {
                    return Err(ApprovalCommitError::State(ApprovalError::Policy));
                }
                Ok(snapshot.clone())
            }
        }
    }
}

fn snapshot_for(
    parts: &EnrollmentRecordParts,
    generation: u64,
) -> Result<PolicySnapshot, ApprovalError> {
    let policy = parts.policy.as_ref().ok_or(ApprovalError::Approval)?;
    let record = TrustRecord::from_parts(TrustRecordParts {
        version: 1,
        store_id: parts.genesis.parts().0,
        generation,
        local_principal: *parts.device.principal().fingerprint(),
        peer_public_key: policy.peer().public_key(),
        peer_kid: policy.peer().kid(),
        enabled: parts.approved,
    })
    .map_err(|_| ApprovalError::Record)?;
    PolicySnapshot::new(record, *policy.owner(), *policy.resource_policy())
        .map_err(|_| ApprovalError::Record)
}

#[cfg(test)]
#[path = "authority_tests.rs"]
mod tests;
