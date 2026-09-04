//! `no_std` [`Memory`] backend: typed arrays sized by [`MemoryProfile`].

use super::BodySlots;
use super::DatagramSlots;
use super::DedupEntry;
use super::DedupKey;
use super::DedupSlots;
use super::Endpoint;
use super::ExchangeEntry;
use super::ExchangeKey;
use super::Exchanges;
use super::ObserveInterest;
use super::ObserveKey;
use super::ObserveSlots;
use super::PendingCon;
use super::PendingCons;
use super::SlotError;
use super::SlotId;
use super::SlotPool;
use super::Storage;
use super::block::{BlockKey, BlockProgress, BlockRole, BlockTransfer, BodyOps, OutgoingBlock};
use super::capacities::{Capacities, bytes_ok};
use super::exchange::ExchangeStore;
use super::pool::DatagramBytes;
use super::table::{DedupStore, ObserveStore};
use crate::error::BlockTransferError;
use crate::message::{BlockValue, MessageId};

/// Named associated constants for the areas present in [`Memory`].
///
/// Not positional const generics on [`Memory`]. Body constants and body array
/// types are used only when block-wise is enabled
/// ([`Memory<P, WithBodies<P>>`]). `.block_wise(false)` does not allocate body
/// arrays; do not use `0` as “disabled.”
///
/// Concrete array types (`RxDatagram`, …) carry the same numbers as the
/// constants. That is how `Memory<P>` owns `[T; N]` on stable Rust without
/// positional const generics on the engine type.
///
/// Numbers for the shipped profiles: [`crate::profiles`].
pub trait MemoryProfile {
    /// Incoming Datagram Pool occupancy.
    const RX_DATAGRAM_SLOTS: usize;
    /// Bytes in each RX datagram slot.
    const RX_DATAGRAM_BYTES: usize;
    /// Outgoing Datagram Pool occupancy.
    const TX_DATAGRAM_SLOTS: usize;
    /// Bytes in each TX datagram slot.
    const TX_DATAGRAM_BYTES: usize;
    /// Dedup table occupancy.
    const DEDUP_ENTRIES: usize;
    /// Observe interest table occupancy.
    const OBSERVE_ENTRIES: usize;
    /// Incoming Body Pool occupancy. Used only when block-wise is enabled.
    const RX_BODY_SLOTS: usize;
    /// Complete-body bytes in each RX body slot (multiple of 1024 when enabled).
    const RX_BODY_BYTES: usize;
    /// Outgoing Body Pool occupancy. Used only when block-wise is enabled.
    const TX_BODY_SLOTS: usize;
    /// Complete-body bytes in each TX body slot (multiple of 1024 when enabled).
    const TX_BODY_BYTES: usize;

    /// Incoming Datagram Pool array type (`DatagramPool<SLOTS, BYTES>`).
    type RxDatagram: SlotPool + Default;
    /// Outgoing Datagram Pool array type.
    type TxDatagram: SlotPool + Default;
    /// Incoming Body Pool array type. Instantiated only with [`WithBodies`].
    type RxBody: SlotPool + Default;
    /// Outgoing Body Pool array type. Instantiated only with [`WithBodies`].
    type TxBody: SlotPool + Default;
    /// Dedup table array type.
    type Dedup: SlotPool + Default;
    /// Observe interest table array type.
    type Observe: SlotPool + Default;
    /// Outstanding-request table. Sized from [`Self::TX_DATAGRAM_SLOTS`].
    type Exchange: SlotPool + Default;
}

/// Marker: [`Memory`] has no body pools.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoBodies;

/// RX/TX body arrays owned by [`Memory<P, WithBodies<P>>`].
pub struct WithBodies<P: MemoryProfile> {
    rx: P::RxBody,
    tx: P::TxBody,
}

impl<P: MemoryProfile> WithBodies<P> {
    fn new() -> Self {
        debug_assert!(P::RX_BODY_SLOTS > 0 && P::TX_BODY_SLOTS > 0);
        debug_assert!(P::RX_BODY_BYTES > 0 && P::TX_BODY_BYTES > 0);
        debug_assert!(bytes_ok(P::RX_BODY_BYTES) && bytes_ok(P::TX_BODY_BYTES));
        Self {
            rx: P::RxBody::default(),
            tx: P::TxBody::default(),
        }
    }
}

/// `no_std` backend: typed arrays for the areas present in Storage.
///
/// `Memory<P>` has datagram pools and tables only. `Memory<P, WithBodies<P>>`
/// also owns RX/TX body pools. This is not a carved byte slab.
///
/// Body accessors exist only on the [`WithBodies`] variant:
///
/// ```compile_fail
/// fn no_body_method(m: &coaptic::Memory<coaptic::profiles::Default>) {
///     let _ = m.rx_body();
/// }
/// ```
///
/// See `design.md` and `knowledge/memory.md`.
pub struct Memory<P: MemoryProfile, B = NoBodies> {
    rx: P::RxDatagram,
    tx: P::TxDatagram,
    dedup: P::Dedup,
    observe: P::Observe,
    exchange: P::Exchange,
    #[allow(dead_code)]
    bodies: B,
}

impl<P: MemoryProfile> Memory<P> {
    /// Datagram pools and tables only. No body RAM.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rx: P::RxDatagram::default(),
            tx: P::TxDatagram::default(),
            dedup: P::Dedup::default(),
            observe: P::Observe::default(),
            exchange: P::Exchange::default(),
            bodies: NoBodies,
        }
    }

    /// Construct [`Memory<P, WithBodies<P>>`] using the profile’s body types.
    #[must_use]
    pub fn with_block_wise() -> Memory<P, WithBodies<P>> {
        Memory::<P, WithBodies<P>>::new()
    }
}

impl<P: MemoryProfile> Memory<P, WithBodies<P>> {
    /// Datagram pools, tables, and RX/TX body pools from the profile.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rx: P::RxDatagram::default(),
            tx: P::TxDatagram::default(),
            dedup: P::Dedup::default(),
            observe: P::Observe::default(),
            exchange: P::Exchange::default(),
            bodies: WithBodies::new(),
        }
    }
}

impl<P: MemoryProfile> Default for Memory<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: MemoryProfile> Default for Memory<P, WithBodies<P>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: MemoryProfile, B> Memory<P, B> {
    /// Incoming Datagram Pool.
    #[must_use]
    pub fn rx_datagram(&self) -> &P::RxDatagram {
        &self.rx
    }

    /// Incoming Datagram Pool (mutable).
    pub fn rx_datagram_mut(&mut self) -> &mut P::RxDatagram {
        &mut self.rx
    }

    /// Outgoing Datagram Pool.
    #[must_use]
    pub fn tx_datagram(&self) -> &P::TxDatagram {
        &self.tx
    }

    /// Outgoing Datagram Pool (mutable).
    pub fn tx_datagram_mut(&mut self) -> &mut P::TxDatagram {
        &mut self.tx
    }

    /// Dedup table.
    #[must_use]
    pub fn dedup(&self) -> &P::Dedup {
        &self.dedup
    }

    /// Dedup table (mutable).
    pub fn dedup_mut(&mut self) -> &mut P::Dedup {
        &mut self.dedup
    }

    /// Observe interest table.
    #[must_use]
    pub fn observe(&self) -> &P::Observe {
        &self.observe
    }

    /// Observe interest table (mutable).
    pub fn observe_mut(&mut self) -> &mut P::Observe {
        &mut self.observe
    }

    /// Outstanding-request table (Token + remote [`Endpoint`]).
    #[must_use]
    pub fn exchange(&self) -> &P::Exchange {
        &self.exchange
    }

    /// Outstanding-request table (mutable).
    pub fn exchange_mut(&mut self) -> &mut P::Exchange {
        &mut self.exchange
    }
}

impl<P: MemoryProfile> Memory<P, WithBodies<P>> {
    /// Incoming Body Pool. Present only on [`WithBodies`] storage.
    #[must_use]
    pub fn rx_body(&self) -> &P::RxBody {
        &self.bodies.rx
    }

    /// Incoming Body Pool (mutable).
    pub fn rx_body_mut(&mut self) -> &mut P::RxBody {
        &mut self.bodies.rx
    }

    /// Outgoing Body Pool. Present only on [`WithBodies`] storage.
    #[must_use]
    pub fn tx_body(&self) -> &P::TxBody {
        &self.bodies.tx
    }

    /// Outgoing Body Pool (mutable).
    pub fn tx_body_mut(&mut self) -> &mut P::TxBody {
        &mut self.bodies.tx
    }
}

macro_rules! impl_common_storage {
    () => {
        fn rx_datagram(&mut self) -> &mut dyn SlotPool {
            &mut self.rx
        }

        fn tx_datagram(&mut self) -> &mut dyn SlotPool {
            &mut self.tx
        }

        fn dedup(&mut self) -> &mut dyn SlotPool {
            &mut self.dedup
        }

        fn observe(&mut self) -> &mut dyn SlotPool {
            &mut self.observe
        }
    };
}

impl<P: MemoryProfile> Storage for Memory<P> {
    fn capacities(&self) -> Capacities {
        Capacities::from_profile::<P>()
    }

    impl_common_storage!();

    fn rx_body(&mut self) -> Option<&mut dyn SlotPool> {
        None
    }

    fn tx_body(&mut self) -> Option<&mut dyn SlotPool> {
        None
    }
}

impl<P: MemoryProfile> Storage for Memory<P, WithBodies<P>> {
    fn capacities(&self) -> Capacities {
        Capacities::from_profile::<P>().with_block_wise::<P>()
    }

    impl_common_storage!();

    fn rx_body(&mut self) -> Option<&mut dyn SlotPool> {
        Some(&mut self.bodies.rx)
    }

    fn tx_body(&mut self) -> Option<&mut dyn SlotPool> {
        Some(&mut self.bodies.tx)
    }
}

impl<P: MemoryProfile, B> DatagramSlots for Memory<P, B>
where
    P::RxDatagram: DatagramBytes,
    P::TxDatagram: DatagramBytes,
{
    fn rx_payload(&self, id: SlotId) -> Option<&[u8]> {
        DatagramBytes::payload(&self.rx, id)
    }

    fn tx_payload(&self, id: SlotId) -> Option<&[u8]> {
        DatagramBytes::payload(&self.tx, id)
    }

    fn rx_payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        DatagramBytes::payload_mut(&mut self.rx, id)
    }

    fn tx_payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        DatagramBytes::payload_mut(&mut self.tx, id)
    }

    fn set_rx_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        DatagramBytes::set_len(&mut self.rx, id, len)
    }

    fn set_tx_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        DatagramBytes::set_len(&mut self.tx, id, len)
    }

    fn rx_endpoint(&self, id: SlotId) -> Option<Endpoint> {
        DatagramBytes::endpoint(&self.rx, id)
    }

    fn tx_endpoint(&self, id: SlotId) -> Option<Endpoint> {
        DatagramBytes::endpoint(&self.tx, id)
    }

    fn set_rx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        DatagramBytes::set_endpoint(&mut self.rx, id, endpoint)
    }

    fn set_tx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        DatagramBytes::set_endpoint(&mut self.tx, id, endpoint)
    }

    fn tx_pending_mid(&self, id: SlotId) -> Option<MessageId> {
        DatagramBytes::pending_mid(&self.tx, id)
    }

    fn set_tx_pending(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError> {
        DatagramBytes::set_pending_mid(&mut self.tx, id, message_id)
    }

    fn clear_tx_pending(&mut self, id: SlotId) -> Result<(), SlotError> {
        DatagramBytes::clear_pending_mid(&mut self.tx, id)
    }

    fn lookup_tx_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        DatagramBytes::lookup_pending(&self.tx, message_id, endpoint)
    }
}

impl<P: MemoryProfile, B> PendingCons for Memory<P, B>
where
    P::TxDatagram: DatagramBytes,
{
    fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
    ) -> Option<SlotId> {
        DatagramBytes::set_endpoint(&mut self.tx, id, endpoint).ok()?;
        DatagramBytes::set_pending_mid(&mut self.tx, id, message_id).ok()?;
        Some(id)
    }

    fn lookup_pending_con(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        DatagramBytes::lookup_pending(&self.tx, message_id, endpoint)
    }

    fn take_pending_con(&mut self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        let id = DatagramBytes::lookup_pending(&self.tx, message_id, endpoint)?;
        DatagramBytes::clear_pending_mid(&mut self.tx, id).ok()?;
        Some(id)
    }

    fn pending_con(&self, id: SlotId) -> Option<PendingCon> {
        let message_id = DatagramBytes::pending_mid(&self.tx, id)?;
        let endpoint = DatagramBytes::endpoint(&self.tx, id)?;
        Some(PendingCon::new(message_id, endpoint, id))
    }
}

impl<P: MemoryProfile, B> DedupSlots for Memory<P, B>
where
    P::Dedup: DedupStore,
{
    fn insert_dedup(&mut self, entry: DedupEntry) -> Option<SlotId> {
        self.dedup.insert(entry)
    }

    fn lookup_dedup(&self, key: DedupKey) -> Option<SlotId> {
        self.dedup.lookup(key)
    }

    fn remove_dedup(&mut self, key: DedupKey) -> bool {
        self.dedup.remove(key)
    }

    fn dedup_entry(&self, id: SlotId) -> Option<DedupEntry> {
        self.dedup.entry(id)
    }
}

impl<P: MemoryProfile, B> Exchanges for Memory<P, B>
where
    P::Exchange: ExchangeStore,
{
    fn insert_exchange(&mut self, entry: ExchangeEntry) -> Option<SlotId> {
        self.exchange.insert(entry)
    }

    fn lookup_exchange(&self, key: ExchangeKey) -> Option<SlotId> {
        self.exchange.lookup(key)
    }

    fn take_exchange(&mut self, key: ExchangeKey) -> Option<ExchangeEntry> {
        self.exchange.take(key)
    }

    fn exchange_entry(&self, id: SlotId) -> Option<ExchangeEntry> {
        self.exchange.entry(id)
    }
}

impl<P: MemoryProfile, B> ObserveSlots for Memory<P, B>
where
    P::Observe: ObserveStore,
{
    fn insert_observe(&mut self, interest: ObserveInterest) -> Option<SlotId> {
        self.observe.insert(interest)
    }

    fn lookup_observe(&self, key: ObserveKey) -> Option<SlotId> {
        self.observe.lookup(key)
    }

    fn remove_observe(&mut self, key: ObserveKey) -> bool {
        self.observe.remove(key)
    }

    fn take_observe(&mut self, key: ObserveKey) -> Option<ObserveInterest> {
        self.observe.take(key)
    }

    fn observe_interest(&self, id: SlotId) -> Option<ObserveInterest> {
        self.observe.entry(id)
    }
}

impl<P: MemoryProfile> BodySlots for Memory<P> {
    fn rx_body_payload(&self, _id: SlotId) -> Option<&[u8]> {
        None
    }

    fn tx_body_payload(&self, _id: SlotId) -> Option<&[u8]> {
        None
    }

    fn rx_body_transfer(&self, _id: SlotId) -> Option<BlockTransfer> {
        None
    }

    fn tx_body_transfer(&self, _id: SlotId) -> Option<BlockTransfer> {
        None
    }

    fn lookup_rx_body(&self, _key: BlockKey) -> Option<SlotId> {
        None
    }

    fn lookup_tx_body(&self, _key: BlockKey) -> Option<SlotId> {
        None
    }

    fn admit_block1(
        &mut self,
        _key: BlockKey,
        _block: BlockValue,
        _payload: &[u8],
        _size1: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn write_block1(
        &mut self,
        _id: SlotId,
        _block: BlockValue,
        _payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn apply_block1(
        &mut self,
        _key: BlockKey,
        _block: BlockValue,
        _payload: &[u8],
        _size1: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn admit_block2(
        &mut self,
        _key: BlockKey,
        _block: BlockValue,
        _payload: &[u8],
        _size2: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn write_block2(
        &mut self,
        _id: SlotId,
        _block: BlockValue,
        _payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn apply_block2(
        &mut self,
        _key: BlockKey,
        _block: BlockValue,
        _payload: &[u8],
        _size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn start_block1(
        &mut self,
        _key: BlockKey,
        _body: &[u8],
        _szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn next_block1(&mut self, _id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn start_block2(
        &mut self,
        _key: BlockKey,
        _body: &[u8],
        _szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }

    fn next_block2(&mut self, _id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        Err(BlockTransferError::NoBodyPools)
    }
}

impl<P: MemoryProfile> BodySlots for Memory<P, WithBodies<P>>
where
    P::RxBody: BodyOps,
    P::TxBody: BodyOps,
{
    fn rx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.bodies.rx.payload(id)
    }

    fn tx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.bodies.tx.payload(id)
    }

    fn rx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        self.bodies.rx.transfer(id)
    }

    fn tx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        self.bodies.tx.transfer(id)
    }

    fn lookup_rx_body(&self, key: BlockKey) -> Option<SlotId> {
        self.bodies.rx.lookup(key)
    }

    fn lookup_tx_body(&self, key: BlockKey) -> Option<SlotId> {
        self.bodies.tx.lookup(key)
    }

    fn admit_block1(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.bodies
            .rx
            .admit_incoming(key, BlockRole::IncomingBlock1, block, payload, size1)
    }

    fn write_block1(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.bodies
            .rx
            .write_incoming(id, BlockRole::IncomingBlock1, block, payload)
    }

    fn apply_block1(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.bodies
            .rx
            .apply_incoming(key, BlockRole::IncomingBlock1, block, payload, size1)
    }

    fn admit_block2(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.bodies
            .rx
            .admit_incoming(key, BlockRole::IncomingBlock2, block, payload, size2)
    }

    fn write_block2(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.bodies
            .rx
            .write_incoming(id, BlockRole::IncomingBlock2, block, payload)
    }

    fn apply_block2(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.bodies
            .rx
            .apply_incoming(key, BlockRole::IncomingBlock2, block, payload, size2)
    }

    fn start_block1(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.bodies
            .tx
            .start_outgoing(key, BlockRole::OutgoingBlock1, body, szx)
    }

    fn next_block1(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.bodies.tx.next_outgoing(id, BlockRole::OutgoingBlock1)
    }

    fn start_block2(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.bodies
            .tx
            .start_outgoing(key, BlockRole::OutgoingBlock2, body, szx)
    }

    fn next_block2(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.bodies.tx.next_outgoing(id, BlockRole::OutgoingBlock2)
    }
}

impl<P: MemoryProfile> core::fmt::Debug for Memory<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Memory")
            .field("capacities", &self.capacities())
            .finish_non_exhaustive()
    }
}

impl<P: MemoryProfile> core::fmt::Debug for Memory<P, WithBodies<P>> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Memory")
            .field("capacities", &self.capacities())
            .finish_non_exhaustive()
    }
}
