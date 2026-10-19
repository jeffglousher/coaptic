//! Bounded CBOR v1 on a privileged local commissioning channel.
//!
//! The frame marker only separates packets from diagnostic console output. It
//! authenticates nothing. Accept requests only from the trusted local host;
//! verify operator approval there before sending a state-changing update.

use super::local_pairing::{Reader, Writer};
use super::{PairingChallenge, PairingError, PairingRecord, PairingUpdate, PinnedPeer};
use crate::Endpoint;

const MAX: usize = 512;
const MAGIC: [u8; 4] = [0, b'C', b'P', 1];

/// Privileged local request; never serve this protocol on an untrusted network.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CommissioningRequest {
    /// Read public device state without creating anything.
    Inspect,
    /// Explicit first setup. Existing or corrupt identity storage must refuse.
    Initialize {
        /// Credential identifier assigned to the new device identity.
        kid: u8,
    },
    /// Replace the pending short-lived challenge after stopping live sessions.
    Challenge,
    /// Apply the exact operator-approved update and a public IPv4 routing hint.
    Update {
        /// Exact challenge, peer, owner, permissions and action approved on the host.
        update: PairingUpdate,
        /// Public IPv4 routing hint; it supplies no peer identity.
        endpoint: Endpoint,
    },
    /// Restart the platform, preserving identity/policy and discarding sessions.
    Restart,
    /// Drain existing durable work and hold production for local administration.
    /// Implementations bound the drain and hold deadlines; no grant is changed.
    Quiesce,
}

impl CommissioningRequest {
    /// Encode one bounded CBOR request, without its serial frame.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, PairingError> {
        let mut out = Writer {
            bytes: output,
            pos: 0,
        };
        match self {
            Self::Inspect => out.raw(&[0x82, 1, 0])?,
            Self::Initialize { kid } => {
                out.raw(&[0x83, 1, 1])?;
                out.uint(u64::from(*kid))?;
            }
            Self::Challenge => out.raw(&[0x82, 1, 2])?,
            Self::Update { update, endpoint } => {
                out.raw(&[0x84, 1, 3])?;
                let mut bytes = [0; 384];
                let len = update.encode(&mut bytes)?;
                blob(&mut out, &bytes[..len])?;
                let (address, port) = endpoint.as_ipv4().ok_or(PairingError::Record)?;
                validate_endpoint(address, port)?;
                let mut bytes = [0; 6];
                bytes[..4].copy_from_slice(&address);
                bytes[4..].copy_from_slice(&port.to_be_bytes());
                blob(&mut out, &bytes)?;
            }
            Self::Restart => out.raw(&[0x82, 1, 4])?,
            Self::Quiesce => out.raw(&[0x82, 1, 5])?,
        }
        Ok(out.pos)
    }

    /// Decode exact bounded canonical CBOR; validates no host/operator authority.
    pub fn decode(input: &[u8]) -> Result<Self, PairingError> {
        let mut read = reader(input)?;
        let array = read.take(1)?[0];
        read.exact(&[1])?;
        let result = match (array, read.uint()?) {
            (0x82, 0) => Self::Inspect,
            (0x83, 1) => Self::Initialize {
                kid: read.uint()?.try_into().map_err(|_| PairingError::Record)?,
            },
            (0x82, 2) => Self::Challenge,
            (0x84, 3) => {
                let update = PairingUpdate::decode(read_blob(&mut read, 384)?)?;
                let endpoint = read_blob(&mut read, 6)?;
                if endpoint.len() != 6 {
                    return Err(PairingError::Record);
                }
                let address = endpoint[..4].try_into().map_err(|_| PairingError::Record)?;
                let port = u16::from_be_bytes([endpoint[4], endpoint[5]]);
                validate_endpoint(address, port)?;
                Self::Update {
                    update,
                    endpoint: Endpoint::v4(address, port),
                }
            }
            (0x82, 4) => Self::Restart,
            (0x82, 5) => Self::Quiesce,
            _ => return Err(PairingError::Record),
        };
        read.finish()?;
        Ok(result)
    }
}

/// Read-only status of pending-work custody under the current pairing record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommissioningPending {
    /// No pending record exists; this does not prove unused history.
    Missing = 0,
    /// Authenticated empty or completed record; no unresolved effect.
    Complete = 1,
    /// Authenticated work is awaiting a matching durable receipt.
    Unresolved = 2,
    /// Read failed or this record cannot authenticate the retained pending bytes.
    Unverifiable = 3,
}

/// Public response. No response ever exports a device's private identity.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CommissioningReply {
    /// Confirmed never-initialized state, not a corrupt/missing recovery fallback.
    Empty,
    /// Recovered device public credential and complete durable policy.
    State {
        /// Device's canonical public credential.
        device: PinnedPeer,
        /// Complete public durable policy for that same device credential.
        record: PairingRecord,
        /// Pending-work status; never exports payloads, operation IDs or keys.
        pending: CommissioningPending,
    },
    /// Fresh device-generated challenge bound to its exact current record.
    Challenge(PairingChallenge),
    /// Complete committed public record after confirmed durable storage.
    Committed(PairingRecord),
    /// Restart acknowledgment; no claim about elapsed power-off time.
    Restarting,
    /// Existing durable work is complete; new production is held for60 seconds.
    /// An administrative challenge may extend the hold to its own deadline.
    Quiesced,
    /// Bounded refusal code, with no private data or detailed logs.
    Refused(u8),
}

impl CommissioningReply {
    /// Encode at most 512 bytes of canonical application CBOR v1.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, PairingError> {
        let mut out = Writer {
            bytes: output,
            pos: 0,
        };
        match self {
            Self::Empty => {
                out.raw(&[0x82, 1])?;
                out.uint(129)?;
            }
            Self::State {
                device,
                record,
                pending,
            } => {
                out.raw(&[0x86, 1])?;
                out.uint(128)?;
                out.bytes(&device.public_key())?;
                out.uint(u64::from(device.kid()))?;
                let mut bytes = [0; PairingRecord::MAX_ENCODED_LEN];
                let len = record.encode(&mut bytes)?;
                blob(&mut out, &bytes[..len])?;
                out.uint(*pending as u64)?;
            }
            Self::Challenge(challenge) => {
                out.raw(&[0x83, 1])?;
                out.uint(130)?;
                let mut bytes = [0; 224];
                let len = challenge.encode(&mut bytes)?;
                blob(&mut out, &bytes[..len])?;
            }
            Self::Committed(record) => {
                out.raw(&[0x83, 1])?;
                out.uint(131)?;
                let mut bytes = [0; PairingRecord::MAX_ENCODED_LEN];
                let len = record.encode(&mut bytes)?;
                blob(&mut out, &bytes[..len])?;
            }
            Self::Restarting => {
                out.raw(&[0x82, 1])?;
                out.uint(132)?;
            }
            Self::Quiesced => {
                out.raw(&[0x82, 1])?;
                out.uint(133)?;
            }
            Self::Refused(code) => {
                out.raw(&[0x83, 1])?;
                out.uint(255)?;
                out.uint(u64::from(*code))?;
            }
        }
        Ok(out.pos)
    }

    /// Decode the complete bounded response. Authenticate its local transport.
    pub fn decode(input: &[u8]) -> Result<Self, PairingError> {
        let mut read = reader(input)?;
        let array = read.take(1)?[0];
        read.exact(&[1])?;
        let result = match (array, read.uint()?) {
            (0x82, 129) => Self::Empty,
            (0x86, 128) => {
                let key = read.bytes::<65>()?;
                let kid = read.uint()?.try_into().map_err(|_| PairingError::Record)?;
                let device =
                    PinnedPeer::from_public_key(&key, kid).map_err(|_| PairingError::Record)?;
                let record =
                    PairingRecord::decode(read_blob(&mut read, PairingRecord::MAX_ENCODED_LEN)?)?;
                if record.local_principal() != device.principal().fingerprint()
                    || record
                        .policy()
                        .is_some_and(|policy| policy.peer().public_x() == device.public_x())
                {
                    return Err(PairingError::Binding);
                }
                let pending = match read.uint()? {
                    0 => CommissioningPending::Missing,
                    1 => CommissioningPending::Complete,
                    2 => CommissioningPending::Unresolved,
                    3 => CommissioningPending::Unverifiable,
                    _ => return Err(PairingError::Record),
                };
                Self::State {
                    device,
                    record,
                    pending,
                }
            }
            (0x83, 130) => Self::Challenge(PairingChallenge::decode(read_blob(&mut read, 224)?)?),
            (0x83, 131) => Self::Committed(PairingRecord::decode(read_blob(
                &mut read,
                PairingRecord::MAX_ENCODED_LEN,
            )?)?),
            (0x82, 132) => Self::Restarting,
            (0x82, 133) => Self::Quiesced,
            (0x83, 255) => {
                Self::Refused(read.uint()?.try_into().map_err(|_| PairingError::Record)?)
            }
            _ => return Err(PairingError::Record),
        };
        read.finish()?;
        Ok(result)
    }
}

fn validate_endpoint(address: [u8; 4], port: u16) -> Result<(), PairingError> {
    if address == [0; 4] || address[0] >= 224 || port == 0 {
        return Err(PairingError::Record);
    }
    Ok(())
}

fn reader(input: &[u8]) -> Result<Reader<'_>, PairingError> {
    if input.len() > MAX {
        return Err(PairingError::Record);
    }
    Ok(Reader {
        bytes: input,
        pos: 0,
    })
}

fn blob(out: &mut Writer<'_>, bytes: &[u8]) -> Result<(), PairingError> {
    match bytes.len() {
        0..=23 => out.raw(&[0x40 | bytes.len() as u8])?,
        24..=255 => out.raw(&[0x58, bytes.len() as u8])?,
        256..=512 => {
            out.raw(&[0x59])?;
            out.raw(&(bytes.len() as u16).to_be_bytes())?;
        }
        _ => return Err(PairingError::Record),
    }
    out.raw(bytes)
}

fn read_blob<'a>(read: &mut Reader<'a>, maximum: usize) -> Result<&'a [u8], PairingError> {
    let head = read.take(1)?[0];
    let length = match head {
        0x40..=0x57 => usize::from(head & 31),
        0x58 => {
            let len = usize::from(read.take(1)?[0]);
            if len < 24 {
                return Err(PairingError::Record);
            }
            len
        }
        0x59 => {
            let bytes = read.take(2)?;
            let len = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
            if len <= 255 {
                return Err(PairingError::Record);
            }
            len
        }
        _ => return Err(PairingError::Record),
    };
    if length > maximum {
        return Err(PairingError::Record);
    }
    read.take(length)
}

/// Fixed 512-byte frame scanner that can share a console with ordinary logs.
/// Framing supplies no authentication; malformed/oversized frames are discarded.
pub struct CommissioningDecoder {
    matched: usize,
    length: usize,
    used: usize,
    bytes: [u8; MAX],
}

impl Default for CommissioningDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl CommissioningDecoder {
    /// Empty bounded frame decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            matched: 0,
            length: 0,
            used: 0,
            bytes: [0; MAX],
        }
    }

    /// Consume one byte and borrow a complete payload until the next call.
    pub fn feed(&mut self, byte: u8) -> Option<&[u8]> {
        if self.matched < MAGIC.len() {
            self.matched = if byte == MAGIC[self.matched] {
                self.matched + 1
            } else {
                usize::from(byte == MAGIC[0])
            };
        } else if self.matched == 4 {
            self.length = usize::from(byte) << 8;
            self.matched = 5;
        } else if self.matched == 5 {
            self.length |= usize::from(byte);
            self.matched = if (1..=MAX).contains(&self.length) {
                6
            } else {
                0
            };
            self.used = 0;
        } else {
            self.bytes[self.used] = byte;
            self.used += 1;
            if self.used == self.length {
                let len = self.length;
                self.matched = 0;
                self.used = 0;
                return Some(&self.bytes[..len]);
            }
        }
        None
    }

    /// Prefix a payload with the marker and a bounded two-byte length.
    pub fn frame(payload: &[u8], output: &mut [u8]) -> Result<usize, PairingError> {
        if payload.is_empty() || payload.len() > MAX || output.len() < payload.len() + 6 {
            return Err(PairingError::Record);
        }
        output[..4].copy_from_slice(&MAGIC);
        output[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        output[6..payload.len() + 6].copy_from_slice(payload);
        Ok(payload.len() + 6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provisioning::{EnrollmentPolicy, Identity, LocalPairing};

    #[test]
    fn quiesce_is_bounded_and_rejects_trailing_bytes() {
        let mut out = [0; 8];
        let len = CommissioningRequest::Quiesce.encode(&mut out).unwrap();
        assert_eq!(&out[..len], &[0x82, 1, 5]);
        assert!(matches!(
            CommissioningRequest::decode(&out[..len]),
            Ok(CommissioningRequest::Quiesce)
        ));
        assert!(CommissioningRequest::decode(&out[..len + 1]).is_err());
        let len = CommissioningReply::Quiesced.encode(&mut out).unwrap();
        assert!(matches!(
            CommissioningReply::decode(&out[..len]),
            Ok(CommissioningReply::Quiesced)
        ));
        assert!(CommissioningReply::decode(&out[..len + 1]).is_err());
    }

    #[test]
    fn framed_update_round_trip_is_exact_bounded_and_single_use() {
        let identity = Identity::from_private_key([1; 32], 1).unwrap();
        let peer = Identity::from_private_key([2; 32], 2).unwrap();
        let record = PairingRecord::unpaired(&identity, [3; 32]).unwrap();
        let mut state = LocalPairing::restore(&identity, record).unwrap();
        let challenge = state
            .begin(&identity, 0, |out| {
                out.fill(4);
                true
            })
            .unwrap();
        let update = PairingUpdate::new(
            challenge,
            EnrollmentPolicy::new(peer.peer(), [5; 32], [6; 32]).unwrap(),
            false,
        );
        let request = CommissioningRequest::Update {
            update,
            endpoint: Endpoint::v4([127, 0, 0, 1], 5683),
        };
        let mut payload = [0; 512];
        let len = request.encode(&mut payload).unwrap();
        for end in 0..len {
            assert!(CommissioningRequest::decode(&payload[..end]).is_err());
        }
        let mut wire = [0; 518];
        let len = CommissioningDecoder::frame(&payload[..len], &mut wire).unwrap();
        let mut decoder = CommissioningDecoder::new();
        for byte in b"console output\n" {
            assert!(decoder.feed(*byte).is_none());
        }
        let mut count = 0;
        for byte in &wire[..len] {
            if let Some(payload) = decoder.feed(*byte) {
                let CommissioningRequest::Update { update, .. } =
                    CommissioningRequest::decode(payload).unwrap()
                else {
                    panic!()
                };
                update.apply(&mut state, 1, |_, _| Ok::<_, ()>(())).unwrap();
                assert!(update.apply(&mut state, 2, |_, _| Ok::<_, ()>(())).is_err());
                count += 1;
            }
        }
        assert_eq!(count, 1);
        assert!(state.permits(&peer.peer().principal()));
        let reply = CommissioningReply::Committed(state.checkpoint().clone());
        let len = reply.encode(&mut payload).unwrap();
        assert!(matches!(
            CommissioningReply::decode(&payload[..len]),
            Ok(CommissioningReply::Committed(_))
        ));
        payload[len] = 0;
        assert!(CommissioningReply::decode(&payload[..len + 1]).is_err());
    }
}
