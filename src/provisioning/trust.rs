//! Fixed one-peer trust lifecycle with caller-owned durable storage.
//!
//! Install and change pins only through an authenticated commissioning or
//! management authority. This module does not authenticate that authority,
//! store private keys, implement an ownership-transfer protocol or discover
//! credentials. The record binds the caller-stored local identity's full
//! credential fingerprint to one installed peer pin.
//!
//! The caller supplies a trusted monotonic [`TrustAnchor`] and a durable
//! compare-and-swap callback. Its authority must survive rollback of ordinary
//! storage. A BLAKE3 record digest identifies the exact committed contents;
//! it does not authenticate storage or provide monotonicity. A file containing
//! both a record and its anchor can be rolled back together and is insufficient.
//!
//! Capture a [`TrustGrant`] before starting EDHOC, check it in every handshake
//! authorization callback and check it again when taking the completed session.
//! Trust changes invalidate all prior grants, including revoke/regrant of the
//! same credential. Before any App or bootstrap I/O, check the live grant and
//! discard invalidated owners, their exchanges and completion caches. A check
//! only in the application handler cannot stop cached replies, retransmissions,
//! Observe or deferred sends. Serialize management changes with authorization
//! and effects; independent policy replicas cannot see an external change until
//! the caller refreshes their trusted anchor.

use core::fmt;

use super::{Identity, PinnedPeer, Principal, Session, check_identity};

const RECORD_VERSION: u8 = 1;
const RECORD_DOMAIN: &str = "https://github.com/jeffglousher/coaptic trust record fingerprint v1";

/// Invalid or unusable durable trust state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustError {
    /// The caller-supplied store identity is zero.
    StoreIdentity,
    /// The record version, generation or peer credential is invalid.
    InvalidRecord,
    /// Installation requires a trusted uninitialized anchor.
    Initialized,
    /// The record does not exactly match the trusted durable anchor.
    AnchorMismatch,
    /// The supplied local identity differs from the record's full fingerprint.
    IdentityMismatch,
    /// The local identity and peer use the same authentication key.
    SelfPeer,
    /// The peer is disabled or differs from the authenticated session.
    Unauthorized,
    /// A trust change invalidated the captured authorization grant.
    StaleGrant,
    /// Persistence was attempted without a definitive success; restore first.
    Blocked,
    /// No greater generation is representable.
    GenerationExhausted,
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StoreIdentity => "trust store identity must be nonzero",
            Self::InvalidRecord => "invalid trust record",
            Self::Initialized => "trust store is already initialized",
            Self::AnchorMismatch => "trust record differs from trusted anchor",
            Self::IdentityMismatch => "local identity differs from trust record",
            Self::SelfPeer => "local identity and peer use the same authentication key",
            Self::Unauthorized => "peer is not authorized",
            Self::StaleGrant => "trust grant is no longer current",
            Self::Blocked => "trust persistence outcome requires verified restoration",
            Self::GenerationExhausted => "trust generation is exhausted",
        })
    }
}

impl core::error::Error for TrustError {}

/// Failure to commit a durable trust transition.
#[derive(Debug)]
pub enum TrustCommitError<E> {
    /// Validation refused the transition before persistence was attempted.
    State(TrustError),
    /// Persistence failed or its result was ambiguous; all grants are blocked.
    Persistence(E),
}

impl<E: fmt::Display> fmt::Display for TrustCommitError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => error.fmt(f),
            Self::Persistence(error) => write!(f, "trust persistence failed: {error}"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for TrustCommitError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Persistence(error) => Some(error),
        }
    }
}

/// Exact record identity supplied by a caller-owned monotonic authority.
///
/// This value has no secret material. Its constructors validate representation,
/// not authority or freshness. Obtain its parts through an authenticated path
/// whose monotonic state cannot be restored from an old ordinary-storage image.
/// The store identity distinguishes independent stores and remains stable across
/// local identity or peer rotation. It is not an OSCORE key epoch or UUID label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustAnchor {
    store_id: [u8; 32],
    generation: u64,
    digest: [u8; 32],
}

impl TrustAnchor {
    /// Describe a trusted, never-initialized store before its first commit.
    ///
    /// The caller must establish that this authority has never committed a
    /// record. Missing files, an absent record or a failed restore do not justify
    /// resetting the authority to genesis. `store_id` must be nonzero and unique
    /// within the caller's authority; generating it is a caller responsibility.
    pub fn genesis(store_id: [u8; 32]) -> Result<Self, TrustError> {
        Self::from_parts(store_id, 0, [0; 32])
    }

    /// Validate lossless parts read from the trusted monotonic authority.
    /// Generation zero is reserved for genesis and requires a zero digest.
    pub fn from_parts(
        store_id: [u8; 32],
        generation: u64,
        digest: [u8; 32],
    ) -> Result<Self, TrustError> {
        if store_id == [0; 32] {
            return Err(TrustError::StoreIdentity);
        }
        if generation == 0 && digest != [0; 32] {
            return Err(TrustError::InvalidRecord);
        }
        Ok(Self {
            store_id,
            generation,
            digest,
        })
    }

    /// Lossless store identity, generation and full record digest.
    #[must_use]
    pub const fn parts(self) -> ([u8; 32], u64, [u8; 32]) {
        (self.store_id, self.generation, self.digest)
    }
}

/// Fixed public persistence fields for one trust record; not a wire format.
///
/// Preserve every field without loss. The corresponding trusted anchor commits
/// their complete canonical contents. No private key, session key, sender nonce,
/// replay checkpoint or application exchange is contained in these parts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustRecordParts {
    /// Storage schema version; the current version is one.
    pub version: u8,
    /// Nonzero stable identity of the caller's trust store.
    pub store_id: [u8; 32],
    /// Committed generation; record generations start at one.
    pub generation: u64,
    /// Full BLAKE3 fingerprint of the caller-stored local credential.
    pub local_principal: [u8; 32],
    /// Canonical uncompressed SEC1 P-256 public point of the pinned peer.
    pub peer_public_key: [u8; 65],
    /// One-byte peer credential identifier, included in its full principal.
    pub peer_kid: u8,
    /// Whether this credential may receive a new authorization grant.
    pub enabled: bool,
}

/// Validated fixed record binding one local identity to one public peer pin.
///
/// Construction validates public credential representation. It does not prove
/// commissioning authority or freshness. Only exact restoration against a
/// caller-trusted anchor, or a successful durable commit, activates a policy.
#[derive(Clone, Debug)]
pub struct TrustRecord {
    store_id: [u8; 32],
    generation: u64,
    local_principal: [u8; 32],
    peer: PinnedPeer,
    enabled: bool,
}

impl TrustRecord {
    /// Validate public persistence fields without granting authorization.
    pub fn from_parts(parts: TrustRecordParts) -> Result<Self, TrustError> {
        if parts.store_id == [0; 32] {
            return Err(TrustError::StoreIdentity);
        }
        if parts.version != RECORD_VERSION || parts.generation == 0 {
            return Err(TrustError::InvalidRecord);
        }
        let peer = PinnedPeer::from_public_key(&parts.peer_public_key, parts.peer_kid)
            .map_err(|_| TrustError::InvalidRecord)?;
        Ok(Self {
            store_id: parts.store_id,
            generation: parts.generation,
            local_principal: parts.local_principal,
            peer,
            enabled: parts.enabled,
        })
    }

    /// Lossless canonical public fields for caller-owned durable storage.
    #[must_use]
    pub fn parts(&self) -> TrustRecordParts {
        TrustRecordParts {
            version: RECORD_VERSION,
            store_id: self.store_id,
            generation: self.generation,
            local_principal: self.local_principal,
            peer_public_key: self.peer.public_key(),
            peer_kid: self.peer.kid(),
            enabled: self.enabled,
        }
    }

    /// Full domain-separated digest of this exact record and its generation.
    ///
    /// The digest is record identity metadata. It is not a MAC, monotonic counter
    /// or proof that this record was commissioned or durably committed.
    #[must_use]
    pub fn anchor(&self) -> TrustAnchor {
        let mut hash = blake3::Hasher::new_derive_key(RECORD_DOMAIN);
        hash.update(&[RECORD_VERSION]);
        hash.update(&self.store_id);
        hash.update(&self.generation.to_be_bytes());
        hash.update(&self.local_principal);
        hash.update(&self.peer.public_key());
        hash.update(&[self.peer.kid(), u8::from(self.enabled)]);
        TrustAnchor {
            store_id: self.store_id,
            generation: self.generation,
            digest: *hash.finalize().as_bytes(),
        }
    }

    /// Installed immutable public pin, including its full credential principal.
    #[must_use]
    pub const fn peer(&self) -> &PinnedPeer {
        &self.peer
    }

    /// Full fingerprint that must match the caller-stored local identity.
    #[must_use]
    pub const fn local_principal(&self) -> &[u8; 32] {
        &self.local_principal
    }

    /// Whether this record enables its installed peer.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
}

/// Captured authorization for one principal at one committed trust generation.
///
/// This public metadata is issued only by an active [`PeerTrust`]. It has no
/// persistence constructor. A trust mutation invalidates it even if the same
/// credential is reinstalled. It is not an OSCORE key epoch or replay checkpoint.
///
/// ```compile_fail
/// # use coaptic::provisioning::{Principal, TrustAnchor, TrustGrant};
/// fn forge(anchor: TrustAnchor, principal: Principal) -> TrustGrant {
///     TrustGrant { anchor, principal }
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustGrant {
    anchor: TrustAnchor,
    principal: Principal,
}

impl TrustGrant {
    /// Exact committed trust anchor against which this grant was issued.
    #[must_use]
    pub const fn anchor(&self) -> TrustAnchor {
        self.anchor
    }

    /// Full principal authorized at the captured trust generation.
    #[must_use]
    pub const fn principal(&self) -> &Principal {
        &self.principal
    }
}

/// One committed peer policy with fixed storage and no retained callbacks.
///
/// Private identity storage remains caller-owned. Successful local rotation
/// binds the replacement identity's full fingerprint, but does not persist its
/// private scalar. Ensure that identity is durably available when committing
/// the record, and discard every owner using a prior grant before further I/O.
pub struct PeerTrust {
    record: TrustRecord,
    anchor: TrustAnchor,
    blocked: bool,
}

impl PeerTrust {
    /// Install the first authenticated peer only after durable persistence.
    ///
    /// `trusted_genesis` must come from a confirmed never-initialized authority.
    /// The callback durably commits the candidate record and new anchor using
    /// compare-and-swap against that exact genesis anchor. It must reject any
    /// concurrent change and return success only after power-loss durability.
    /// Record storage and authority advancement need a backend-owned recoverable
    /// transaction; writing both into one rollbackable file is insufficient.
    ///
    /// No policy or usable grant is returned on failure. If a commit succeeds
    /// but its acknowledgment is lost, read the actual trusted anchor and record
    /// and use [`Self::restore`]; do not reinstall against a fabricated genesis.
    pub fn install<E>(
        identity: &Identity,
        peer: PinnedPeer,
        trusted_genesis: TrustAnchor,
        commit: impl FnOnce(&TrustAnchor, &TrustRecord, &TrustAnchor) -> Result<(), E>,
    ) -> Result<Self, TrustCommitError<E>> {
        if trusted_genesis.generation != 0 {
            return Err(TrustCommitError::State(TrustError::Initialized));
        }
        validate_peer(identity, &peer).map_err(TrustCommitError::State)?;
        let record = TrustRecord {
            store_id: trusted_genesis.store_id,
            generation: 1,
            local_principal: *identity.peer().principal().fingerprint(),
            peer,
            enabled: true,
        };
        let anchor = record.anchor();
        commit(&trusted_genesis, &record, &anchor).map_err(TrustCommitError::Persistence)?;
        Ok(Self {
            record,
            anchor,
            blocked: false,
        })
    }

    /// Restore exactly the record named by the latest trusted monotonic anchor.
    ///
    /// Both stale and uncommitted newer records fail, as do another store's
    /// records or a different local identity. Never fall back to genesis after
    /// refusal. The caller must authenticate and freshly read the anchor; this
    /// method cannot detect rollback of the authority itself.
    pub fn restore(
        identity: &Identity,
        record: TrustRecord,
        trusted_anchor: TrustAnchor,
    ) -> Result<Self, TrustError> {
        if record.anchor() != trusted_anchor {
            return Err(TrustError::AnchorMismatch);
        }
        validate_identity(identity, &record)?;
        Ok(Self {
            record,
            anchor: trusted_anchor,
            blocked: false,
        })
    }

    /// Last definitively committed record, including a revocation tombstone.
    /// After a blocked commit this is not necessarily the durable latest record;
    /// refresh storage and authority before restoring or using it again.
    #[must_use]
    pub const fn checkpoint(&self) -> &TrustRecord {
        &self.record
    }

    /// Whether an ambiguous persistence outcome blocks every authorization.
    #[must_use]
    pub const fn is_blocked(&self) -> bool {
        self.blocked
    }

    /// Capture the active peer authorization before starting a new handshake.
    /// The supplied local identity must match this committed record exactly.
    pub fn grant(&self, identity: &Identity) -> Result<TrustGrant, TrustError> {
        self.active()?;
        validate_identity(identity, &self.record)?;
        Ok(TrustGrant {
            anchor: self.anchor,
            principal: self.record.peer.principal(),
        })
    }

    /// Revalidate a captured grant against current committed trust and principal.
    ///
    /// Call in each EDHOC authorization callback and before every owner action
    /// that can produce I/O or effects, under the caller's management lock or
    /// exclusive event-loop ownership. A policy replica must be refreshed after
    /// an external authority change; this method performs no storage reads.
    #[must_use]
    pub fn permits(&self, grant: &TrustGrant, principal: &Principal) -> bool {
        self.validate_grant(grant, principal).is_ok()
    }

    /// Consume a completed session only while its captured grant remains current.
    ///
    /// This checks the full authenticated peer principal and the local credential
    /// used by the exchange before App handoff. The caller must check the grant
    /// before every later App/bootstrap action and discard owners when
    /// authorization changes.
    pub fn accept_session(
        &self,
        grant: &TrustGrant,
        session: Session,
    ) -> Result<Session, TrustError> {
        self.validate_grant(grant, session.principal())?;
        if session.local_principal().fingerprint() != &self.record.local_principal {
            return Err(TrustError::IdentityMismatch);
        }
        Ok(session)
    }

    /// Replace the local identity binding or peer pin through trusted management.
    ///
    /// Every successful call advances the generation and invalidates old grants,
    /// even if its public fields are otherwise unchanged. `enabled == false`
    /// persists a disabled record. The callback durably compares the exact old
    /// anchor and commits the new record/anchor atomically or with backend-owned
    /// crash recovery. Return success only after actual power-loss durability.
    ///
    /// Validation failures leave this policy usable and do not invoke storage.
    /// Once storage is attempted, any error blocks all grants until fresh verified
    /// restoration, including when a write committed but its acknowledgment was
    /// lost. The old checkpoint remains available only as last-known metadata.
    pub fn replace<E>(
        &mut self,
        identity: &Identity,
        peer: PinnedPeer,
        enabled: bool,
        commit: impl FnOnce(&TrustAnchor, &TrustRecord, &TrustAnchor) -> Result<(), E>,
    ) -> Result<TrustAnchor, TrustCommitError<E>> {
        if self.blocked {
            return Err(TrustCommitError::State(TrustError::Blocked));
        }
        validate_peer(identity, &peer).map_err(TrustCommitError::State)?;
        let generation = self.next_generation().map_err(TrustCommitError::State)?;
        let record = TrustRecord {
            store_id: self.record.store_id,
            generation,
            local_principal: *identity.peer().principal().fingerprint(),
            peer,
            enabled,
        };
        self.commit(record, commit)
    }

    /// Durably disable the installed peer, retaining a revocation tombstone.
    ///
    /// The commit and ambiguous-error rules are identical to [`Self::replace`].
    /// A repeated revocation still advances the generation. This does not cancel
    /// already accepted application work; the caller owns its cancellation rules.
    pub fn revoke<E>(
        &mut self,
        commit: impl FnOnce(&TrustAnchor, &TrustRecord, &TrustAnchor) -> Result<(), E>,
    ) -> Result<TrustAnchor, TrustCommitError<E>> {
        if self.blocked {
            return Err(TrustCommitError::State(TrustError::Blocked));
        }
        let mut record = self.record.clone();
        record.generation = self.next_generation().map_err(TrustCommitError::State)?;
        record.enabled = false;
        self.commit(record, commit)
    }

    fn active(&self) -> Result<(), TrustError> {
        if self.blocked {
            Err(TrustError::Blocked)
        } else if !self.record.enabled {
            Err(TrustError::Unauthorized)
        } else {
            Ok(())
        }
    }

    fn validate_grant(&self, grant: &TrustGrant, principal: &Principal) -> Result<(), TrustError> {
        self.active()?;
        if grant.anchor != self.anchor {
            return Err(TrustError::StaleGrant);
        }
        if grant.principal != self.record.peer.principal() || *principal != grant.principal {
            return Err(TrustError::Unauthorized);
        }
        Ok(())
    }

    fn next_generation(&self) -> Result<u64, TrustError> {
        self.record
            .generation
            .checked_add(1)
            .ok_or(TrustError::GenerationExhausted)
    }

    fn commit<E>(
        &mut self,
        record: TrustRecord,
        persist: impl FnOnce(&TrustAnchor, &TrustRecord, &TrustAnchor) -> Result<(), E>,
    ) -> Result<TrustAnchor, TrustCommitError<E>> {
        let anchor = record.anchor();
        self.blocked = true;
        persist(&self.anchor, &record, &anchor).map_err(TrustCommitError::Persistence)?;
        self.record = record;
        self.anchor = anchor;
        self.blocked = false;
        Ok(anchor)
    }
}

impl fmt::Debug for PeerTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerTrust")
            .field("record", &self.record)
            .field("anchor", &self.anchor)
            .field("blocked", &self.blocked)
            .finish()
    }
}

fn validate_peer(identity: &Identity, peer: &PinnedPeer) -> Result<(), TrustError> {
    check_identity(identity, peer).map_err(|_| TrustError::SelfPeer)
}

fn validate_identity(identity: &Identity, record: &TrustRecord) -> Result<(), TrustError> {
    if identity.peer().principal().fingerprint() != &record.local_principal {
        return Err(TrustError::IdentityMismatch);
    }
    validate_peer(identity, &record.peer)
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
