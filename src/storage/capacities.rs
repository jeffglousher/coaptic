//! Runtime capacity description shared by the builder and both backends.

use super::memory::MemoryProfile;
use crate::error::BuildError;

/// Byte multiple required for enabled body slots (max Block/Q-Block SZX).
pub(crate) const BODY_BYTE_QUANTUM: usize = 1024;

/// Runtime sizes for [`AllocMemory`](crate::AllocMemory) / [`EngineBuilder::build_alloc`](crate::EngineBuilder::build_alloc).
///
/// Also the size report from [`Storage::capacities`](crate::Storage::capacities).
/// Body fields are `None` when Storage has no body pools.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacities {
    /// Incoming Datagram Pool slot count.
    pub rx_datagram_slots: usize,
    /// Bytes in each RX datagram slot.
    pub rx_datagram_bytes: usize,
    /// Outgoing Datagram Pool slot count.
    pub tx_datagram_slots: usize,
    /// Bytes in each TX datagram slot.
    pub tx_datagram_bytes: usize,
    /// Dedup table entries.
    pub dedup_entries: usize,
    /// Observe interest table entries.
    pub observe_entries: usize,
    /// Incoming Body Pool slot count. `None` when body pools are absent.
    pub rx_body_slots: Option<usize>,
    /// Complete-body bytes in each RX body slot. `None` when body pools are absent.
    pub rx_body_bytes: Option<usize>,
    /// Outgoing Body Pool slot count. `None` when body pools are absent.
    pub tx_body_slots: Option<usize>,
    /// Complete-body bytes in each TX body slot. `None` when body pools are absent.
    pub tx_body_bytes: Option<usize>,
}

impl Capacities {
    /// Datagram and table sizes from `P`. Body fields are absent.
    #[must_use]
    pub const fn from_profile<P: MemoryProfile>() -> Self {
        Self {
            rx_datagram_slots: P::RX_DATAGRAM_SLOTS,
            rx_datagram_bytes: P::RX_DATAGRAM_BYTES,
            tx_datagram_slots: P::TX_DATAGRAM_SLOTS,
            tx_datagram_bytes: P::TX_DATAGRAM_BYTES,
            dedup_entries: P::DEDUP_ENTRIES,
            observe_entries: P::OBSERVE_ENTRIES,
            rx_body_slots: None,
            rx_body_bytes: None,
            tx_body_slots: None,
            tx_body_bytes: None,
        }
    }

    /// Add body sizes from `P`. Used with `.block_wise(true)`.
    #[must_use]
    pub const fn with_block_wise<P: MemoryProfile>(self) -> Self {
        Self {
            rx_body_slots: Some(P::RX_BODY_SLOTS),
            rx_body_bytes: Some(P::RX_BODY_BYTES),
            tx_body_slots: Some(P::TX_BODY_SLOTS),
            tx_body_bytes: Some(P::TX_BODY_BYTES),
            ..self
        }
    }

    /// Whether all four body fields are present.
    #[must_use]
    pub const fn has_body_pools(&self) -> bool {
        self.rx_body_slots.is_some()
            && self.rx_body_bytes.is_some()
            && self.tx_body_slots.is_some()
            && self.tx_body_bytes.is_some()
    }

    pub(crate) const fn body_fields_consistent(&self) -> bool {
        let any = self.rx_body_slots.is_some()
            || self.rx_body_bytes.is_some()
            || self.tx_body_slots.is_some()
            || self.tx_body_bytes.is_some();
        let all = self.has_body_pools();
        all || !any
    }

    pub(crate) fn validate_for_build(&self, block_wise: bool) -> Result<(), BuildError> {
        if !self.body_fields_consistent() {
            return Err(BuildError::IncompleteBodyCapacities);
        }
        if block_wise {
            if !self.has_body_pools() {
                return Err(BuildError::MissingBodyPools);
            }
            let rx_slots = self.rx_body_slots.expect("checked");
            let rx_bytes = self.rx_body_bytes.expect("checked");
            let tx_slots = self.tx_body_slots.expect("checked");
            let tx_bytes = self.tx_body_bytes.expect("checked");
            if rx_slots == 0 || rx_bytes == 0 || tx_slots == 0 || tx_bytes == 0 {
                return Err(BuildError::ZeroBodyCapacity);
            }
            if !bytes_ok(rx_bytes) || !bytes_ok(tx_bytes) {
                return Err(BuildError::BodyBytesNotMultipleOf1024);
            }
        } else if self.has_body_pools() {
            return Err(BuildError::UnexpectedBodyPools);
        }
        Ok(())
    }
}

pub(crate) const fn bytes_ok(bytes: usize) -> bool {
    bytes % BODY_BYTE_QUANTUM == 0
}
