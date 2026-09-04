//! `no_std` [`Memory`] backend: typed arrays sized by [`MemoryProfile`].

use crate::capacities::{Capacities, bytes_ok};
use crate::storage::{SlotPool, Storage};

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
