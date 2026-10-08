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

mod coap;
mod crypto;
mod identity;
mod lakers;

pub use coap::{CoapProvisioner, PollError, Status};
pub use identity::{Identity, PinnedPeer, Principal};

use core::fmt;

use self::lakers::{ConnId, CredentialTransfer, EDHOCMethod, EDHOCSuite};

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
    /// The peer used a different method, suite, CID or credential profile.
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

const INITIATOR_ID: [u8; 1] = [0];
const RESPONDER_ID: [u8; 1] = [1];

fn connection_id(bytes: &[u8]) -> ConnId {
    ConnId::from_slice(bytes).expect("fixed one-byte connection identifier")
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

/// Initiator awaiting the responder's authenticated message 2.
pub struct Initiator {
    state: lakers::EdhocInitiatorWaitM2<Crypto>,
    peer: PinnedPeer,
}

impl Initiator {
    /// Starts a new handshake with fresh ephemeral entropy and an installed pin.
    pub fn start(
        identity: &Identity,
        peer: PinnedPeer,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        check_identity(identity, &peer)?;
        let crypto = Crypto::fresh(entropy)?;
        let mut initiator =
            lakers::EdhocInitiator::new(crypto, EDHOCMethod::StatStat, EDHOCSuite::CipherSuite2);
        initiator.set_identity(identity.scalar(), identity.credential());
        let (state, message) =
            initiator.prepare_message_1(Some(connection_id(&INITIATOR_ID)), &None)?;
        Ok((Self { state, peer }, Message::from_buffer(&message)?))
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
        if cid.as_slice() != RESPONDER_ID || ead.is_some() {
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
            },
            Message::from_buffer(&message)?,
        ))
    }
}

/// Initiator awaiting mandatory explicit key confirmation in message 4.
pub struct InitiatorConfirm {
    state: lakers::EdhocInitiatorWaitM4<Crypto>,
    peer: PinnedPeer,
}

impl InitiatorConfirm {
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
        Session::derive(&secret[..16], &salt[..8], true, self.peer)
    }
}

/// Responder awaiting the initiator's authenticated message 3.
pub struct Responder {
    state: lakers::EdhocResponderWaitM3<Crypto>,
    peer: PinnedPeer,
}

impl Responder {
    /// Checks the supported profile and peer point, then prepares message 2.
    pub fn receive_message_1(
        identity: &Identity,
        peer: PinnedPeer,
        message: &Message,
        entropy: impl FnMut(&mut [u8]) -> bool,
    ) -> Result<(Self, Message), Error> {
        check_identity(identity, &peer)?;
        let bytes = message.as_bytes();
        if bytes.len() != 37 || bytes[..4] != [3, 2, 0x58, 0x20] || bytes[36] != 0 {
            return Err(Error::Profile);
        }
        let message = message.buffer()?;
        let (method, suites, public_x, cid, ead) = lakers::parse_message_1(&message)?;
        if method != 3
            || suites.as_slice() != [2]
            || cid.as_slice() != INITIATOR_ID
            || ead.is_some()
        {
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
            Some(connection_id(&RESPONDER_ID)),
            &None,
        )?;
        Ok((Self { state, peer }, Message::from_buffer(&message)?))
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
            Session::derive(&secret[..16], &salt[..8], false, self.peer)?,
            Message::from_buffer(&message)?,
        ))
    }
}

/// Authenticated identity and fresh volatile OSCORE context for one new App.
pub struct Session {
    context: SecurityContext,
    principal: Principal,
}

impl Session {
    fn derive(
        secret: &[u8],
        salt: &[u8],
        initiator: bool,
        peer: PinnedPeer,
    ) -> Result<Self, Error> {
        let (sender_id, recipient_id) = if initiator {
            (&RESPONDER_ID[..], &INITIATOR_ID[..])
        } else {
            (&INITIATOR_ID[..], &RESPONDER_ID[..])
        };
        let context = SecurityContext::derive(DeriveParams {
            master_secret: secret,
            master_salt: salt,
            sender_id,
            recipient_id,
            id_context: &[],
        })
        .map_err(|_| Error::Derivation)?;
        Ok(Self {
            context,
            principal: peer.principal(),
        })
    }

    /// Returns the peer identity authenticated by the completed exchange.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Moves the fresh context and peer identity to the caller.
    /// Bind a new secure App and carry this identity into its authorization policy.
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

redacted_debug!(Initiator, InitiatorConfirm, Responder, Session);

#[cfg(test)]
mod tests;
