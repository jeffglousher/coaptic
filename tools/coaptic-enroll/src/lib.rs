//! Host-side FIDO2 approval of exact public commissioning transactions.
//!
//! This is separate from device identity generation and normal EDHOC/OSCORE.
//! Registration is a trusted local administrative operation. An authenticated
//! operator approves a specific immutable request; the verifier keeps challenge
//! state in memory and consumes it even on refusal. Only the public operator
//! credential is persisted. Storage must serialize updates and persist the
//! updated authenticator counter before consuming an approval.
//!
//! The local client requests a cross-platform authenticator. This default profile
//! requires user verification and refuses backed-up credentials at approval; it
//! does not attest a manufacturer/model or supply a trusted transaction display.
//! Register known hardware through the trusted local client and review the exact
//! device, peer and application policy before approving. The operator credential
//! never receives a device's private key. The Windows host custody adapter seals
//! device keys with DPAPI; it does not provide flash/disk rollback resistance.

use std::fmt;
use std::time::{Duration, Instant};

use coaptic::provisioning::{
    ApprovalCommitError, EnrollmentAnchor, EnrollmentAuthority, EnrollmentPolicy, EnrollmentStore,
    PinnedPeer,
};
use serde::{Deserialize, Serialize};
use webauthn_rs::prelude::*;

pub mod store;

#[cfg(feature = "development-piv")]
pub mod development_piv;

#[cfg(windows)]
pub mod device;

#[cfg(windows)]
pub mod pairing;

#[cfg(test)]
mod tests;

const RP_ID: &str = "coaptic.local";
const ORIGIN: &str = "https://coaptic.local";
const TIMEOUT: Duration = Duration::from_secs(60);

type Result<T> = std::result::Result<T, Error>;

/// Refusal of operator registration, authentication or transaction binding.
#[derive(Debug)]
pub enum Error {
    /// Invalid or unsupported public input.
    Input,
    /// No successful user-verified, device-bound authenticator proof.
    Authentication,
    /// The challenge's original monotonic deadline elapsed.
    Expired,
    /// The nonzero authenticator counter failed to advance.
    Counter,
    /// Durable operator state was not confirmed; approval is unusable.
    Persistence,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "operator approval refused: {self:?}")
    }
}
impl std::error::Error for Error {}

/// Stored public operator credential; it contains no authenticator private key.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    version: u8,
    #[serde(with = "serde_bytes")]
    owner: [u8; 32],
    key: Passkey,
    counter: u32,
}

impl Operator {
    /// Exact owner commitment, independent of a display label.
    pub fn owner(&self) -> &[u8; 32] {
        &self.owner
    }

    /// Public credential identifier used only for operator key lookup.
    pub fn credential_id(&self) -> &[u8] {
        self.key.cred_id().as_ref()
    }

    /// Begin one fresh, user-verified approval of this exact request.
    pub fn begin(&self, request: Request) -> Result<(RequestChallengeResponse, PendingApproval)> {
        let started = Instant::now();
        if self.version != 1
            || request.owner != self.owner
            || credential_counter(&self.key)? != self.counter
        {
            return Err(Error::Input);
        }
        request.validate()?;
        let (challenge, state) = verifier()?
            .start_passkey_authentication(std::slice::from_ref(&self.key))
            .map_err(|_| Error::Authentication)?;
        Ok((
            challenge,
            PendingApproval {
                state,
                request,
                operator: self.clone(),
                started,
            },
        ))
    }
}

/// Public identity encoding; importing it validates representation, not trust.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicIdentity {
    #[serde(with = "serde_bytes")]
    key: Vec<u8>,
    kid: u8,
}

impl PublicIdentity {
    pub fn from_peer(peer: &PinnedPeer) -> Self {
        Self {
            key: peer.public_key().to_vec(),
            kid: peer.kid(),
        }
    }

    pub fn peer(&self) -> Result<PinnedPeer> {
        if self.key.len() != 65 {
            return Err(Error::Input);
        }
        PinnedPeer::from_public_key(&self.key, self.kid).map_err(|_| Error::Input)
    }
}

/// Version-one immutable public transaction from an authenticated commissioning channel.
/// Physical peer verification must precede this request; FIDO authenticates the operator.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u8,
    pub action: Action,
    pub device: PublicIdentity,
    pub peer: PublicIdentity,
    #[serde(with = "serde_bytes")]
    pub owner: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub permissions: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub store: [u8; 32],
    pub revision: u64,
    #[serde(with = "serde_bytes")]
    pub record_digest: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub nonce: [u8; 32],
}

/// The operation approved by the operator, included in every proof commitment.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Pair,
    Revoke,
}

impl Request {
    pub fn new(
        device: &PinnedPeer,
        peer: &PinnedPeer,
        owner: [u8; 32],
        permissions: [u8; 32],
        expected: EnrollmentAnchor,
        nonce: [u8; 32],
    ) -> Result<Self> {
        let value = Self {
            version: 1,
            action: Action::Pair,
            device: PublicIdentity::from_peer(device),
            peer: PublicIdentity::from_peer(peer),
            owner,
            permissions,
            store: expected.parts().0,
            revision: expected.parts().1,
            record_digest: expected.parts().2,
            nonce,
        };
        value.validate()?;
        Ok(value)
    }

    /// Bind one physical device's live challenge to the exact policy and action.
    pub fn for_pairing(
        challenge: &coaptic::provisioning::PairingChallenge,
        policy: &EnrollmentPolicy,
        action: Action,
    ) -> Result<Self> {
        let (store, revision, digest) = challenge.expected();
        let expected =
            EnrollmentAnchor::from_parts(store, revision, digest).map_err(|_| Error::Input)?;
        let mut request = Self::new(
            challenge.device(),
            policy.peer(),
            *policy.owner(),
            *policy.resource_policy(),
            expected,
            *challenge.nonce(),
        )?;
        request.action = action;
        Ok(request)
    }

    /// Refuse a changed live device challenge before consuming a local approval.
    pub fn matches_pairing(
        &self,
        challenge: &coaptic::provisioning::PairingChallenge,
    ) -> Result<()> {
        self.validate()?;
        let (store, revision, digest) = challenge.expected();
        if self.device.peer()?.principal() != challenge.device().principal()
            || self.store != store
            || self.revision != revision
            || self.record_digest != digest
            || &self.nonce != challenge.nonce()
        {
            return Err(Error::Input);
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        let device = self.device.peer()?;
        let peer = self.peer.peer()?;
        if self.version != 1
            || self.owner == [0; 32]
            || self.store == [0; 32]
            || self.revision == 0
            || self.nonce == [0; 32]
            || device.public_key()[1..33] == peer.public_key()[1..33]
        {
            return Err(Error::Input);
        }
        EnrollmentPolicy::new(peer, self.owner, self.permissions).map_err(|_| Error::Input)?;
        Ok(())
    }

    /// Complete domain-separated transaction commitment for review/audit.
    pub fn commitment(&self) -> Result<[u8; 32]> {
        self.validate()?;
        let mut hash = blake3::Hasher::new_derive_key("coaptic hardware commissioning request v1");
        hash.update(&[match self.action {
            Action::Pair => 0,
            Action::Revoke => 1,
        }]);
        hash.update(self.device.peer()?.principal().fingerprint());
        hash.update(self.peer.peer()?.principal().fingerprint());
        for field in [&self.owner, &self.permissions, &self.store, &self.nonce] {
            hash.update(field);
        }
        hash.update(&self.revision.to_be_bytes());
        hash.update(&self.record_digest);
        Ok(*hash.finalize().as_bytes())
    }
}

/// Consumed challenge state. It is deliberately neither cloneable nor serializable.
pub struct PendingApproval {
    state: PasskeyAuthentication,
    request: Request,
    operator: Operator,
    started: Instant,
}

impl PendingApproval {
    /// Verify the fresh proof, persist updated operator state, then expose one approval.
    /// The callback must serialize and durably compare-and-swap the prior operator state.
    pub fn finish(
        self,
        proof: &PublicKeyCredential,
        persist: impl FnOnce(&Operator, &Operator) -> std::result::Result<(), ()>,
    ) -> Result<ApprovedRequest> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        let result = verifier()?
            .finish_passkey_authentication(proof, &self.state)
            .map_err(|_| Error::Authentication)?;
        if !result.user_verified() || result.backup_eligible() || result.backup_state() {
            return Err(Error::Authentication);
        }
        if (self.operator.counter != 0 || result.counter() != 0)
            && result.counter() <= self.operator.counter
        {
            return Err(Error::Counter);
        }
        let mut updated = self.operator.clone();
        if updated.key.update_credential(&result).is_none() {
            return Err(Error::Authentication);
        }
        updated.counter = result.counter();
        persist(&self.operator, &updated).map_err(|_| Error::Persistence)?;
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        Ok(ApprovedRequest {
            request: self.request,
            started: self.started,
        })
    }
}

/// One successful operator approval, not a serializable authorization token.
pub struct ApprovedRequest {
    request: Request,
    started: Instant,
}

#[derive(Debug)]
pub enum CommitError<E> {
    Approval(Error),
    Authority(ApprovalCommitError<E>),
}

impl ApprovedRequest {
    /// Consume a production FIDO2 approval through an authenticated local
    /// commissioning sink. The sink must compare the complete live device
    /// challenge and durably commit its exact owner/peer/permissions/action.
    pub fn commit_local<T>(self, commit: impl FnOnce(&Request) -> Result<T>) -> Result<T> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        commit(&self.request)
    }
    /// Public transaction commitment for review/audit, not an authorization token.
    pub fn commitment(&self) -> Result<[u8; 32]> {
        self.request.commitment()
    }

    /// Consume approval only for the exact device and current management record.
    /// A competing change invalidates this approval; it must not be rebased/reused.
    pub fn approve<S: EnrollmentStore>(
        self,
        authority: &mut EnrollmentAuthority<S>,
    ) -> std::result::Result<EnrollmentAnchor, CommitError<S::Error>> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(CommitError::Approval(Error::Expired));
        }
        if self.request.action != Action::Pair {
            return Err(CommitError::Approval(Error::Input));
        }
        let device = self.request.device.peer().map_err(CommitError::Approval)?;
        if authority.checkpoint().device() != device.principal() {
            return Err(CommitError::Approval(Error::Input));
        }
        let expected = EnrollmentAnchor::from_parts(
            self.request.store,
            self.request.revision,
            self.request.record_digest,
        )
        .map_err(|_| CommitError::Approval(Error::Input))?;
        let policy = EnrollmentPolicy::new(
            self.request.peer.peer().map_err(CommitError::Approval)?,
            self.request.owner,
            self.request.permissions,
        )
        .map_err(|_| CommitError::Approval(Error::Input))?;
        authority
            .approve(expected, policy)
            .map_err(CommitError::Authority)
    }
}

/// In-memory initial registration state for a trusted local administrator.
pub struct PendingRegistration {
    state: PasskeyRegistration,
    owner: [u8; 32],
    started: Instant,
}

pub fn begin_registration(
    owner: [u8; 32],
) -> Result<(CreationChallengeResponse, PendingRegistration)> {
    let started = Instant::now();
    if owner == [0; 32] {
        return Err(Error::Input);
    }
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&owner[..16]);
    let id = Uuid::from_bytes(bytes);
    let (mut challenge, state) = verifier()?
        .start_passkey_registration(id, "Coaptic operator", "Coaptic operator", None)
        .map_err(|_| Error::Authentication)?;
    if let Some(selection) = challenge.public_key.authenticator_selection.as_mut() {
        selection.authenticator_attachment = Some(AuthenticatorAttachment::CrossPlatform);
    }
    Ok((
        challenge,
        PendingRegistration {
            state,
            owner,
            started,
        },
    ))
}

impl PendingRegistration {
    pub fn finish(self, proof: &RegisterPublicKeyCredential) -> Result<Operator> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        let key = verifier()?
            .finish_passkey_registration(proof, &self.state)
            .map_err(|_| Error::Authentication)?;
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        let counter = credential_counter(&key)?;
        Ok(Operator {
            version: 1,
            owner: self.owner,
            key,
            counter,
        })
    }
}

pub fn origin() -> Url {
    Url::parse(ORIGIN).expect("fixed native client origin")
}

fn verifier() -> Result<Webauthn> {
    WebauthnBuilder::new(RP_ID, &origin())
        .map_err(|_| Error::Input)?
        .rp_name("Coaptic Enrollment")
        .timeout(TIMEOUT)
        .build()
        .map_err(|_| Error::Input)
}

// Inspect only the reviewed verifier's public credential metadata. This is not
// an alternate verifier or authenticator/model attestation. Reject unsuitable
// credentials before persisting registration, rather than trapping an operator
// with a credential which can never satisfy the selected approval profile.
fn credential_counter(key: &Passkey) -> Result<u32> {
    let credential: Credential = key.clone().into();
    if !credential.user_verified || credential.backup_eligible || credential.backup_state {
        return Err(Error::Authentication);
    }
    Ok(credential.counter)
}
