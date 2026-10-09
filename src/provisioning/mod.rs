//! Authenticated, bounded EDHOC provisioning with pinned P-256 credentials.
//!
//! The optional `edhoc` feature supports method 3, cipher suite 2, credentials
//! by reference and mandatory message 4. Install each [`PinnedPeer`] through
//! an authenticated commissioning channel. An untrusted network or a UUID
//! lookup is insufficient to establish trust. Authorization is checked against
//! the authenticated [`Principal`] before a session becomes usable.
//!
//! Persist identity and trust, then run a fresh handshake after either peer
//! restarts. Supply fresh cryptographically secure entropy on every attempt.
//! Session keys are volatile: never restore them into a new App. A new
//! [`Session`] belongs to a new App, without old exchanges, replay state,
//! cached replies or Observe registrations. Reusing a session key instead
//! requires the durable sequence and replay contracts in [`crate::oscore`].
//!
//! Handshake state and keys have redacted Debug output. Lakers internally
//! copies key material; this interface does not promise erasure of all copies.
//! All storage is fixed; larger profiles, EAD and credential discovery are
//! refused. Transport integration owns retries and must retransmit the exact
//! saved message rather than creating a new ephemeral key for a retry.
//!
//! The private EDHOC implementation is Lakers v0.8.0 with a pinned parser/MAC
//! hardening patch. Its BSD notice and source provenance ship with the crate.

mod cloud;
mod coap;
mod connection_id;
mod crypto;
mod enrollment;
mod identity;
mod lakers;
mod trust;

pub use cloud::{
    AdmissionLimits, AdmissionRegistry, AdmittedSession, Association, CloudAdmission, CloudError,
};
pub use coap::{CoapProvisioner, CoapRecovery, PollError, RecoveryError, Status};
pub use connection_id::ConnectionId;
pub use enrollment::{
    AuthenticatedPolicyRequest, AuthorityConnection, AuthorityResponder, BootstrapAuthority,
    EnrollmentError, EnrollmentPolicy, OnlinePolicyAuthority, PendingPolicy, PolicyQuery,
    PolicyServeError, PolicySnapshot, ProtectedPolicyResponse, VerifiedPolicy,
};
pub use identity::{Identity, PinnedPeer, Principal};
pub use trust::{
    PeerTrust, TrustAnchor, TrustCommitError, TrustError, TrustGrant, TrustRecord, TrustRecordParts,
};

use core::fmt;

use self::lakers::{CredentialTransfer, EDHOCMethod, EDHOCSuite};

use crate::oscore::{DeriveParams, SecurityContext};

use crypto::Crypto;

/// Provisioning failure. A failed transition consumes its handshake state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A private scalar or public P-256 point is invalid.
    InvalidKey,
    /// The entropy source failed.
    Entropy,
    /// The message is malformed or exceeds the fixed capacity.
    Parsing,
    /// The selected method, suite, CID or credential profile is unsupported.
    Profile,
    /// The peer did not authenticate with the installed credential.
    Authentication,
    /// The authenticated identity is no longer authorized.
    Unauthorized,
    /// The local identity and peer use the same authentication key.
    SelfPeer,
    /// OSCORE derivation failed.
    Derivation,
}

impl From<lakers::EDHOCError> for Error {
    fn from(value: lakers::EDHOCError) -> Self {
        match value {
            lakers::EDHOCError::MacVerificationFailed
            | lakers::EDHOCError::UnexpectedCredential => Self::Authentication,
            _ => Self::Parsing,
        }
    }
}

/// A complete EDHOC message in fixed storage.
#[derive(Clone, Eq, PartialEq)]
pub struct Message {
    bytes: [u8; lakers::MAX_MESSAGE_SIZE_LEN],
    len: usize,
}

impl Message {
    /// Maximum accepted message size, independent of an App's body pools.
    pub const CAPACITY: usize = lakers::MAX_MESSAGE_SIZE_LEN;

    /// Copies exactly one wire message; empty and oversized messages fail.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > Self::CAPACITY {
            return Err(Error::Parsing);
        }
        let mut result = Self {
            bytes: [0; lakers::MAX_MESSAGE_SIZE_LEN],
            len: bytes.len(),
        };
        result.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(result)
    }

    /// Returns the complete wire bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    fn buffer(&self) -> Result<lakers::EdhocMessageBuffer, Error> {
        lakers::EdhocMessageBuffer::new_from_slice(self.as_bytes()).map_err(|_| Error::Parsing)
    }

    fn from_buffer(buffer: &lakers::EdhocMessageBuffer) -> Result<Self, Error> {
        Self::from_slice(buffer.as_slice())
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message").field("len", &self.len).finish()
    }
}

fn check_identity(identity: &Identity, peer: &PinnedPeer) -> Result<(), Error> {
    if identity.public_x() == peer.credential().public_key().ok_or(Error::InvalidKey)? {
        return Err(Error::SelfPeer);
    }
    Ok(())
}

fn check_reference(id: &lakers::IdCred, peer: &PinnedPeer) -> Result<(), Error> {
    let credential = peer.credential();
    let kid = credential.kid.ok_or(Error::Profile)?;
    let kid_bytes = kid.as_slice();
    if kid_bytes.len() != 1 || id.as_full_value() != [0xa1, 4, 0x41, kid_bytes[0]] {
        return Err(Error::Authentication);
    }
    Ok(())
}

fn message_1_peer_id(message: &Message) -> Result<ConnectionId, Error> {
    let bytes = message.as_bytes();
    if bytes.len() < 37 || bytes[..4] != [3, 2, 0x58, 0x20] {
        return Err(Error::Profile);
    }
    let (peer_id, len) = ConnectionId::decode_prefix(&bytes[36..])?;
    if bytes.len() != 36 + len {
        return Err(Error::Profile);
    }
    Ok(peer_id)
}

/// An unauthenticated credential lookup hint decoded from EDHOC message 3.
///
/// A trusted registry must resolve it to one unambiguous installed credential.
/// Possession and current policy are checked afterward against the full principal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CredentialReference(u8);

impl CredentialReference {
    /// One-byte credential reference supported by this commissioning profile.
    #[must_use]
    pub const fn kid(self) -> u8 {
        self.0
    }

    fn from_id(id: &lakers::IdCred) -> Result<Self, Error> {
        match id.as_full_value() {
            [0xa1, 4, 0x41, kid] => Ok(Self(*kid)),
            _ => Err(Error::Profile),
        }
    }
}

/// Initiator awaiting the responder's authenticated message 2.
pub struct Initiator {
    state: lakers::EdhocInitiatorWaitM2<Crypto>,
    peer: PinnedPeer,
    local_principal: Principal,
    local_id: ConnectionId,
}

impl Initiator {
    /// Starts a new handshake with fresh ephemeral entropy and an installed pin.
    /// The local connection ID is zero and must be reserved outside live sessions.
    pub fn start(
        identity: &Identity,
        peer: PinnedPeer,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        Self::start_with_id(identity, peer, ConnectionId::INITIATOR, entropy)
    }

    /// Starts a fresh handshake with a caller-reserved local connection identifier.
    /// Reserve this ID outside all live EDHOC sessions and OSCORE recipient IDs
    /// without an ID Context. The peer chooses and authenticates its own distinct ID.
    pub fn start_with_id(
        identity: &Identity,
        peer: PinnedPeer,
        local_id: ConnectionId,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        check_identity(identity, &peer)?;
        let crypto = Crypto::fresh(entropy)?;
        let mut initiator =
            lakers::EdhocInitiator::new(crypto, EDHOCMethod::StatStat, EDHOCSuite::CipherSuite2);
        initiator.set_identity(identity.scalar(), identity.credential());
        let (state, message) = initiator.prepare_message_1(Some(local_id.lakers()), &None)?;
        Ok((
            Self {
                state,
                peer,
                local_principal: identity.peer().principal(),
                local_id,
            },
            Message::from_buffer(&message)?,
        ))
    }

    /// Authenticates message 2 and prepares message 3 after current authorization.
    /// No session or exporter is available before message 4 is verified.
    pub fn receive_message_2(
        self,
        message: &Message,
        authorize: impl FnOnce(&Principal) -> bool,
    ) -> Result<(InitiatorConfirm, Message), Error> {
        let message = message.buffer()?;
        let (public_x, _) = lakers::parse_message_2(&message)?;
        crypto::validate_public_x(&public_x)?;
        let (state, cid, id, ead) = self.state.parse_message_2(&message)?;
        let peer_id = ConnectionId::from_lakers(cid)?;
        if peer_id == self.local_id || ead.is_some() {
            return Err(Error::Profile);
        }
        check_reference(&id, &self.peer)?;
        let state = state.verify_message_2(self.peer.credential())?;
        if !authorize(&self.peer.principal()) {
            return Err(Error::Unauthorized);
        }
        let (state, message, _) =
            state.prepare_message_3(CredentialTransfer::ByReference, &None)?;
        Ok((
            InitiatorConfirm {
                state,
                peer: self.peer,
                local_principal: self.local_principal,
                local_id: self.local_id,
                peer_id,
            },
            Message::from_buffer(&message)?,
        ))
    }
}

/// Initiator awaiting mandatory explicit key confirmation in message 4.
pub struct InitiatorConfirm {
    state: lakers::EdhocInitiatorWaitM4<Crypto>,
    peer: PinnedPeer,
    local_principal: Principal,
    local_id: ConnectionId,
    peer_id: ConnectionId,
}

impl InitiatorConfirm {
    /// Returns the responder's authenticated connection identifier for routing message 3.
    #[must_use]
    pub const fn peer_connection_id(&self) -> ConnectionId {
        self.peer_id
    }

    /// Verifies message 4 and checks authorization again before exposing keys.
    pub fn receive_message_4(
        self,
        message: &Message,
        authorize: impl FnOnce(&Principal) -> bool,
    ) -> Result<Session, Error> {
        let (mut state, ead) = self.state.process_message_4(&message.buffer()?)?;
        if ead.is_some() {
            return Err(Error::Profile);
        }
        if !authorize(&self.peer.principal()) {
            return Err(Error::Unauthorized);
        }
        let secret = state.edhoc_exporter(0, &[], 16);
        let salt = state.edhoc_exporter(1, &[], 8);
        Session::derive(
            &secret[..16],
            &salt[..8],
            self.local_id,
            self.peer_id,
            self.local_principal,
            self.peer,
        )
    }
}

/// Responder awaiting the initiator's authenticated message 3.
pub struct Responder {
    state: lakers::EdhocResponderWaitM3<Crypto>,
    peer: PinnedPeer,
    local_principal: Principal,
    local_id: ConnectionId,
    peer_id: ConnectionId,
}

impl Responder {
    /// Checks the supported profile and peer point, then prepares message 2.
    /// The local connection ID is one and must be reserved outside live sessions.
    pub fn receive_message_1(
        identity: &Identity,
        peer: PinnedPeer,
        message: &Message,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        Self::receive_message_1_with_id(identity, peer, ConnectionId::RESPONDER, message, entropy)
    }

    /// Prepares message 2 with a caller-reserved local connection identifier.
    /// Reserve this ID outside live EDHOC sessions and OSCORE recipient IDs
    /// without an ID Context. It must differ from the initiator's message 1 ID.
    pub fn receive_message_1_with_id(
        identity: &Identity,
        peer: PinnedPeer,
        local_id: ConnectionId,
        message: &Message,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        check_identity(identity, &peer)?;
        let (responder, message) =
            RegistryResponder::receive_message_1(identity, local_id, message, entropy)?;
        Ok((
            Self {
                state: responder.state,
                peer,
                local_principal: responder.local_principal,
                local_id,
                peer_id: responder.peer_id,
            },
            message,
        ))
    }

    /// Authenticates message 3, checks current authorization and prepares message 4.
    /// The caller must retain message 4 for exact retransmission until delivery.
    pub fn receive_message_3(
        self,
        message: &Message,
        authorize: impl FnOnce(&Principal) -> bool,
    ) -> Result<(Session, Message), Error> {
        let (state, id, ead) = self.state.parse_message_3(&message.buffer()?)?;
        if ead.is_some() {
            return Err(Error::Profile);
        }
        check_reference(&id, &self.peer)?;
        let (state, _) = state.verify_message_3(self.peer.credential())?;
        if !authorize(&self.peer.principal()) {
            return Err(Error::Unauthorized);
        }
        let (mut state, message) = state.prepare_message_4(&None)?;
        let secret = state.edhoc_exporter(0, &[], 16);
        let salt = state.edhoc_exporter(1, &[], 8);
        Ok((
            Session::derive(
                &secret[..16],
                &salt[..8],
                self.local_id,
                self.peer_id,
                self.local_principal,
                self.peer,
            )?,
            Message::from_buffer(&message)?,
        ))
    }
}

/// Responder deferring peer selection until message 3 exposes its lookup hint.
///
/// Message 1 supplies routing state and an ephemeral point, never a credential
/// identity. Bound admission and address validation before constructing this
/// state. The registry callback returns installed credentials only; application
/// permissions remain a check against the authenticated full principal.
pub struct RegistryResponder {
    state: lakers::EdhocResponderWaitM3<Crypto>,
    local_principal: Principal,
    local_public_x: [u8; 32],
    local_id: ConnectionId,
    peer_id: ConnectionId,
}

impl RegistryResponder {
    /// Checks the bounded profile and prepares message 2 with a reserved local ID.
    /// The ID must be unused by pending handshakes, retained replies and live
    /// OSCORE recipients, and must differ from the initiator's connection ID.
    pub fn receive_message_1(
        identity: &Identity,
        local_id: ConnectionId,
        message: &Message,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        message_1_peer_id(message)?;
        let message = message.buffer()?;
        let (method, suites, public_x, cid, ead) = lakers::parse_message_1(&message)?;
        let peer_id = ConnectionId::from_lakers(cid)?;
        if method != 3 || suites.as_slice() != [2] || peer_id == local_id || ead.is_some() {
            return Err(Error::Profile);
        }
        crypto::validate_public_x(&public_x)?;
        let responder = lakers::EdhocResponder::new(
            Crypto::fresh(entropy)?,
            EDHOCMethod::StatStat,
            identity.scalar(),
            identity.credential(),
        );
        let (state, _, _) = responder.process_message_1(&message)?;
        let (state, message) = state.prepare_message_2(
            CredentialTransfer::ByReference,
            Some(local_id.lakers()),
            &None,
        )?;
        Ok((
            Self {
                state,
                local_principal: identity.peer().principal(),
                local_public_x: identity.public_x(),
                local_id,
                peer_id,
            },
            Message::from_buffer(&message)?,
        ))
    }

    /// Resolves a lookup hint, verifies possession, checks policy and creates M4.
    /// Unknown or ambiguous references fail closed. The authorization callback
    /// runs only after MAC verification, receiving the full installed principal.
    /// Retain the exact message 4 and finish its send before exposing the session.
    pub fn receive_message_3(
        self,
        message: &Message,
        resolve: impl FnOnce(CredentialReference) -> Option<PinnedPeer>,
        authorize: impl FnOnce(&Principal) -> bool,
    ) -> Result<(Session, Message), Error> {
        let (state, id, ead) = self.state.parse_message_3(&message.buffer()?)?;
        if ead.is_some() {
            return Err(Error::Profile);
        }
        let reference = CredentialReference::from_id(&id)?;
        let peer = resolve(reference).ok_or(Error::Authentication)?;
        check_reference(&id, &peer)?;
        if peer.public_x() == self.local_public_x {
            return Err(Error::SelfPeer);
        }
        let (state, _) = state.verify_message_3(peer.credential())?;
        if !authorize(&peer.principal()) {
            return Err(Error::Unauthorized);
        }
        let (mut state, message) = state.prepare_message_4(&None)?;
        let secret = state.edhoc_exporter(0, &[], 16);
        let salt = state.edhoc_exporter(1, &[], 8);
        Ok((
            Session::derive(
                &secret[..16],
                &salt[..8],
                self.local_id,
                self.peer_id,
                self.local_principal,
                peer,
            )?,
            Message::from_buffer(&message)?,
        ))
    }
}

/// Local and authenticated peer identities with a fresh OSCORE context for one App.
pub struct Session {
    context: SecurityContext,
    principal: Principal,
    local_principal: Principal,
}

impl Session {
    fn derive(
        secret: &[u8],
        salt: &[u8],
        local_id: ConnectionId,
        peer_id: ConnectionId,
        local_principal: Principal,
        peer: PinnedPeer,
    ) -> Result<Self, Error> {
        let context = SecurityContext::derive(DeriveParams {
            master_secret: secret,
            master_salt: salt,
            sender_id: peer_id.as_bytes(),
            recipient_id: local_id.as_bytes(),
            id_context: &[],
        })
        .map_err(|_| Error::Derivation)?;
        Ok(Self {
            context,
            principal: peer.principal(),
            local_principal,
        })
    }

    /// Returns the peer identity authenticated by the completed exchange.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Full principal of the local credential used to authenticate this exchange.
    /// Check it against the caller's current local identity binding before handoff;
    /// [`PeerTrust::accept_session`] verifies both local and peer trust bindings.
    #[must_use]
    pub const fn local_principal(&self) -> &Principal {
        &self.local_principal
    }

    /// Moves the fresh context and peer identity to the caller.
    /// Bind a new secure App and carry this identity into its authorization policy.
    /// Use [`PeerTrust::accept_session`] before this move when enforcing durable
    /// trust; the local identity metadata is no longer available afterward.
    pub fn into_parts(self) -> (SecurityContext, Principal) {
        (self.context, self.principal)
    }
}

macro_rules! redacted_debug {
    ($($name:ident),+) => {$ (
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    )+};
}

redacted_debug!(
    Initiator,
    InitiatorConfirm,
    Responder,
    RegistryResponder,
    Session
);

#[cfg(test)]
mod conn_id_tests;

#[cfg(test)]
mod tests;
