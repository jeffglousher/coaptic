use core::fmt;

use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};

use super::{Error, lakers};

const CREDENTIAL_DOMAIN: &str = "https://github.com/jeffglousher/coaptic credential fingerprint v1";
const CREDENTIAL_LEN: usize = 82;

/// Caller-provisioned P-256 private identity for the fixed EDHOC profile.
///
/// Import a private scalar supplied by the caller's trusted provisioning system.
/// Debug output is redacted and this value cannot be cloned. Import and upstream
/// EDHOC processing can copy key material; erasure of every copy is not guaranteed.
///
/// ```compile_fail
/// # use coaptic::provisioning::Identity;
/// fn duplicate(identity: Identity) {
///     let _duplicate = identity.clone();
/// }
/// ```
pub struct Identity {
    private_key: SecretKey,
    peer: PinnedPeer,
}

impl Identity {
    /// Import a nonzero private scalar below the P-256 group order.
    ///
    /// `kid` identifies the generated credential within this provisioning profile;
    /// it is independent of EDHOC connection identifiers and OSCORE Sender IDs.
    /// Invalid scalars return [`Error::InvalidKey`].
    pub fn from_private_key(private_key: [u8; 32], kid: u8) -> Result<Self, Error> {
        let private_key = SecretKey::from_slice(&private_key).map_err(|_| Error::InvalidKey)?;
        let peer = PinnedPeer::from_key(private_key.public_key(), kid)?;
        Ok(Self { private_key, peer })
    }

    /// Export this identity's immutable public credential and principal.
    /// Install it at the other endpoint through a trusted provisioning path.
    #[must_use]
    pub fn peer(&self) -> PinnedPeer {
        self.peer.clone()
    }

    pub(crate) fn credential(&self) -> lakers::Credential {
        self.peer.credential()
    }

    pub(crate) fn scalar(&self) -> [u8; 32] {
        self.private_key.to_bytes().into()
    }

    pub(crate) fn public_x(&self) -> [u8; 32] {
        self.peer.public_x()
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("private_key", &"[REDACTED]")
            .field("principal", &self.peer.principal)
            .finish()
    }
}

/// Immutable canonical public credential installed as a trusted peer pin.
///
/// Construction validates the P-256 point and normalizes its public encoding.
/// It does not authenticate a network peer or authorize resource access. Obtain
/// pins through a trusted provisioning path and verify possession during EDHOC.
#[derive(Clone, Debug)]
pub struct PinnedPeer {
    credential: lakers::Credential,
    principal: Principal,
}

impl PinnedPeer {
    /// Import a valid compressed or uncompressed SEC1 P-256 public point.
    ///
    /// Equivalent SEC1 encodings produce the same canonical credential and
    /// [`Principal`] for a given `kid`. The credential identifier is one byte;
    /// changing it changes the credential fingerprint even if the key is reused.
    /// Infinity, malformed encodings and points outside the curve return
    /// [`Error::InvalidKey`]. Construction supplies no trust bootstrap.
    pub fn from_public_key(public_key: &[u8], kid: u8) -> Result<Self, Error> {
        let public_key = PublicKey::from_sec1_bytes(public_key).map_err(|_| Error::InvalidKey)?;
        Self::from_key(public_key, kid)
    }

    fn from_key(public_key: PublicKey, kid: u8) -> Result<Self, Error> {
        let point = public_key.to_encoded_point(false);
        let x = point.x().ok_or(Error::InvalidKey)?;
        let y = point.y().ok_or(Error::InvalidKey)?;
        let mut encoded = [0; CREDENTIAL_LEN];
        encoded[..9].copy_from_slice(&[0xa1, 0x08, 0xa1, 0x01, 0xa5, 0x01, 0x02, 0x02, 0x41]);
        encoded[9] = kid;
        encoded[10..15].copy_from_slice(&[0x20, 0x01, 0x21, 0x58, 0x20]);
        encoded[15..47].copy_from_slice(x);
        encoded[47..50].copy_from_slice(&[0x22, 0x58, 0x20]);
        encoded[50..].copy_from_slice(y);
        let credential = lakers::Credential::parse_ccs(&encoded).map_err(|_| Error::Parsing)?;
        let principal = Principal(blake3::derive_key(CREDENTIAL_DOMAIN, &encoded));
        Ok(Self {
            credential,
            principal,
        })
    }

    /// Full credential identity metadata for this pin.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    pub(crate) const fn credential(&self) -> lakers::Credential {
        self.credential
    }

    pub(crate) fn public_x(&self) -> [u8; 32] {
        let mut x = [0; 32];
        x.copy_from_slice(&self.credential.bytes.as_slice()[15..47]);
        x
    }
}

/// Domain-separated 32-byte fingerprint of a canonical public credential.
///
/// This is credential lookup and trust metadata. Neither the fingerprint nor its
/// UUID label authenticates a peer, grants access, proves freshness, identifies a
/// cryptographic epoch or prevents rollback. They are not protocol key identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Principal([u8; 32]);

impl Principal {
    /// Full BLAKE3 fingerprint; preserve all 32 bytes for credential comparisons.
    #[must_use]
    pub const fn fingerprint(&self) -> &[u8; 32] {
        &self.0
    }

    /// Optional UUIDv8 display/database label derived from this fingerprint.
    ///
    /// The version and variant replace six bits, leaving 122 custom bits from
    /// the first 16 fingerprint bytes. This lossy label cannot replace the full
    /// fingerprint or the authenticated credential in trust decisions.
    #[must_use]
    pub fn id(&self) -> uuid::Uuid {
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&self.0[..16]);
        uuid::Uuid::new_v8(bytes)
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
