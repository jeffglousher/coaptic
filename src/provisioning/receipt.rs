//! Durable application receipts across fresh security sessions.
//!
//! The store transaction checks the current authoritative grant and resource
//! permission while holding its policy fence through the effect and receipt
//! commit. Persist the effect and receipt together before returning success.
//! A rollbackable database needs reconciliation against an independent latest
//! boundary after backup restoration; ordinary crash recovery is insufficient.

use super::{PeerTrust, Principal, TrustAnchor, TrustGrant};

/// Application operation identity scoped to a full authenticated principal.
///
/// Persist this identity with pending work before its first transmission, then
/// retain it across reconnects until a matching durable receipt is received.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationId([u8; 16]);

impl OperationId {
    /// Uses a caller-issued stable identifier; UUIDv8 bytes are suitable labels.
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Exact operation identifier without truncation or text conversion.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Bounded telemetry content and its complete semantic commitment.
pub struct TelemetryOperation<'a> {
    id: OperationId,
    resource: [u8; 32],
    content_format: Option<u16>,
    payload: &'a [u8],
    digest: [u8; 32],
}

impl<'a> TelemetryOperation<'a> {
    /// Largest payload accepted by this datagram application profile.
    pub const MAX_PAYLOAD: usize = 1024;

    /// Commits the exact target, content format, payload length and payload.
    ///
    /// `resource` is the server's canonical resource identity. The protected
    /// request's target must be authorized independently of this digest.
    pub fn new(
        id: OperationId,
        resource: [u8; 32],
        content_format: Option<u16>,
        payload: &'a [u8],
    ) -> Result<Self, ReceiptError<core::convert::Infallible>> {
        if payload.len() > Self::MAX_PAYLOAD {
            return Err(ReceiptError::Capacity);
        }
        let mut hash = blake3::Hasher::new_derive_key("coaptic telemetry operation v1");
        hash.update(&resource);
        match content_format {
            Some(value) => {
                hash.update(&[1]);
                hash.update(&value.to_be_bytes());
            }
            None => {
                hash.update(&[0]);
            }
        }
        hash.update(&(payload.len() as u64).to_be_bytes());
        hash.update(payload);
        Ok(Self {
            id,
            resource,
            content_format,
            payload,
            digest: *hash.finalize().as_bytes(),
        })
    }

    /// Stable operation identifier.
    pub const fn id(&self) -> OperationId {
        self.id
    }

    /// Server-selected canonical resource identity.
    pub const fn resource(&self) -> &[u8; 32] {
        &self.resource
    }

    /// Complete content-format identity, including absence.
    pub const fn content_format(&self) -> Option<u16> {
        self.content_format
    }

    /// Complete payload to commit atomically with its receipt.
    pub const fn payload(&self) -> &[u8] {
        self.payload
    }

    /// Domain-separated commitment to the operation's exact content.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Accepts a complete receipt only for this operation's ID and exact content.
    ///
    /// Call this after authenticating a successful protected response from the
    /// intended service for the original principal. Receipt bytes contain no
    /// principal or service identity and do not authenticate themselves. Success
    /// relies on that service honoring the [`ReceiptStore`] durability contract;
    /// it does not independently prove persistence or backup freshness.
    ///
    /// Malformed or mismatched receipts return [`ReceiptError::InvalidReceipt`].
    /// Keep the pending operation's ID and content across uncertain replies and
    /// reconnects; retire it only after accepting a matching receipt and recording
    /// that completion at the caller's durable boundary. A CoAP ACK alone does
    /// not complete the operation.
    pub fn accept_receipt(
        &self,
        payload: &[u8],
    ) -> Result<Receipt, ReceiptError<core::convert::Infallible>> {
        Receipt::decode(payload)
            .filter(|receipt| self.matches_receipt(receipt))
            .ok_or(ReceiptError::InvalidReceipt)
    }

    fn matches_receipt(&self, receipt: &Receipt) -> bool {
        receipt.id == self.id && receipt.digest == self.digest
    }
}

/// A durable store's committed operation and complete content digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Receipt {
    id: OperationId,
    digest: [u8; 32],
    sequence: u64,
}

impl Receipt {
    /// Constructs committed metadata; sequence zero is reserved and refused.
    pub fn from_parts(id: OperationId, digest: [u8; 32], sequence: u64) -> Option<Self> {
        (sequence != 0).then_some(Self {
            id,
            digest,
            sequence,
        })
    }

    /// Operation whose durable effect this receipt identifies.
    pub const fn id(&self) -> OperationId {
        self.id
    }

    /// Exact content commitment recorded by the store.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Store-issued committed sequence; this is not an OSCORE nonce.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Complete fixed receipt payload for a protected application response.
    pub fn encode(&self) -> [u8; 56] {
        let mut bytes = [0; 56];
        bytes[..16].copy_from_slice(self.id.as_bytes());
        bytes[16..48].copy_from_slice(&self.digest);
        bytes[48..].copy_from_slice(&self.sequence.to_be_bytes());
        bytes
    }

    /// Validates the complete payload; trailing and truncated bytes fail.
    ///
    /// This checks structure only. Use [`TelemetryOperation::accept_receipt`]
    /// to match a response to pending work after authenticating its service.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 56 {
            return None;
        }
        Self::from_parts(
            OperationId::new(bytes[..16].try_into().ok()?),
            bytes[16..48].try_into().ok()?,
            u64::from_be_bytes(bytes[48..].try_into().ok()?),
        )
    }
}

/// Failure to authorize or durably complete an application operation.
#[derive(Debug, Eq, PartialEq)]
pub enum ReceiptError<E> {
    /// Payload exceeds the declared application bound.
    Capacity,
    /// The grant or resource permission is no longer current.
    Unauthorized,
    /// This operation ID already names different content for this principal.
    Conflict,
    /// Receipt bytes are malformed or metadata names a different operation.
    InvalidReceipt,
    /// Commit outcome is unknown; retry the same operation after reconciliation.
    Persistence(E),
}

/// Durable telemetry effect and receipt transaction with a policy fence.
///
/// Key receipts by the full `principal` and operation ID. Verify the exact
/// `expected` anchor against independently current policy and authorize the
/// operation's resource while serializing against policy updates. Hold that
/// fence until effect and receipt are durably committed. A duplicate with the
/// same digest returns its existing receipt; different content returns conflict.
/// Failure or lost acknowledgement must never justify repeating the effect
/// with a new operation ID. Bound storage/work or return a capacity failure.
pub trait ReceiptStore {
    /// Provider-owned persistence error.
    type Error;

    /// Atomically authorize, apply and commit one operation or recover its receipt.
    fn commit(
        &mut self,
        expected: &TrustAnchor,
        principal: &Principal,
        operation: &TelemetryOperation<'_>,
    ) -> Result<Receipt, ReceiptError<Self::Error>>;
}

/// Checks the captured local grant and verifies complete provider output.
///
/// The provider's authoritative transaction check is still required: a local
/// policy replica cannot establish freshness during an external policy change.
pub fn commit_telemetry<S: ReceiptStore>(
    trust: &PeerTrust,
    grant: &TrustGrant,
    operation: &TelemetryOperation<'_>,
    store: &mut S,
) -> Result<Receipt, ReceiptError<S::Error>> {
    if !trust.permits(grant, grant.principal()) {
        return Err(ReceiptError::Unauthorized);
    }
    let receipt = store.commit(&grant.anchor(), grant.principal(), operation)?;
    if !operation.matches_receipt(&receipt) {
        return Err(ReceiptError::InvalidReceipt);
    }
    Ok(receipt)
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
