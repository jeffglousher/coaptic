//! Shipped [`MemoryProfile`] implementations.
//!
//! Slot and table counts below are modest starting numbers.
//! These are payload capacities, not total RAM or stack budgets. Inline storage
//! also retains protocol state and sidecars; constructing an App by value can
//! require additional pool temporaries. Measure the concrete App layout and
//! optimized target frames before choosing a task's stack size.

use super::exchange::ExchangeTable;
use super::memory::MemoryProfile;
use super::pool::{BodyPool, DatagramPool, DatagramScratch};
use super::table::{DedupTable, ObserveTable};

/// A profile with only its shared client/server Observe table resized.
///
/// Usually selected through [`crate::resources::Resources::observe_entries`].
/// Other pool dimensions, scratch, Calls and security bindings are unchanged.
pub struct WithObserve<P, const ENTRIES: usize>(core::marker::PhantomData<P>);

impl<P: MemoryProfile, const ENTRIES: usize> MemoryProfile for WithObserve<P, ENTRIES> {
    type Token = P::Token;
    const RX_DATAGRAM_SLOTS: usize = P::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = P::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = P::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = P::TX_DATAGRAM_BYTES;
    const DEDUP_ENTRIES: usize = P::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = ENTRIES;
    const RX_BODY_SLOTS: usize = P::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = P::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = P::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = P::TX_BODY_BYTES;

    type RxDatagram = P::RxDatagram;
    type TxDatagram = P::TxDatagram;
    type RxBody = P::RxBody;
    type TxBody = P::TxBody;
    type Dedup = P::Dedup;
    type Observe = ObserveTable<ENTRIES, P::Token>;
    type Exchange = P::Exchange;
    type RxScratch = P::RxScratch;
    type TxScratch = P::TxScratch;
}

/// A profile with only its duplicate-reply history table resized.
///
/// Usually selected through [`crate::resources::Resources::dedup_entries`].
/// Other pool dimensions, scratch, Calls and security bindings are unchanged.
pub struct WithDedup<P, const ENTRIES: usize>(core::marker::PhantomData<P>);

impl<P: MemoryProfile, const ENTRIES: usize> MemoryProfile for WithDedup<P, ENTRIES> {
    type Token = P::Token;
    const RX_DATAGRAM_SLOTS: usize = P::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = P::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = P::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = P::TX_DATAGRAM_BYTES;
    const DEDUP_ENTRIES: usize = ENTRIES;
    const OBSERVE_ENTRIES: usize = P::OBSERVE_ENTRIES;
    const RX_BODY_SLOTS: usize = P::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = P::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = P::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = P::TX_BODY_BYTES;

    type RxDatagram = P::RxDatagram;
    type TxDatagram = P::TxDatagram;
    type RxBody = P::RxBody;
    type TxBody = P::TxBody;
    type Dedup = DedupTable<ENTRIES>;
    type Observe = P::Observe;
    type Exchange = P::Exchange;
    type RxScratch = P::RxScratch;
    type TxScratch = P::TxScratch;
}

/// A profile with only its RX datagram pool and its scratch resized.
///
/// Usually selected through [`crate::resources::Resources::rx_datagram`].
/// Other dimensions and the owned token representation are unchanged.
pub struct WithRxDatagram<P, const SLOTS: usize, const BYTES: usize>(core::marker::PhantomData<P>);

impl<P: MemoryProfile, const SLOTS: usize, const BYTES: usize> MemoryProfile
    for WithRxDatagram<P, SLOTS, BYTES>
{
    type Token = P::Token;
    const RX_DATAGRAM_SLOTS: usize = SLOTS;
    const RX_DATAGRAM_BYTES: usize = BYTES;
    const TX_DATAGRAM_SLOTS: usize = P::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = P::TX_DATAGRAM_BYTES;
    const DEDUP_ENTRIES: usize = P::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = P::OBSERVE_ENTRIES;
    const RX_BODY_SLOTS: usize = P::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = P::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = P::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = P::TX_BODY_BYTES;

    type RxDatagram = DatagramPool<SLOTS, BYTES>;
    type TxDatagram = P::TxDatagram;
    type RxBody = P::RxBody;
    type TxBody = P::TxBody;
    type Dedup = P::Dedup;
    type Observe = P::Observe;
    type Exchange = P::Exchange;
    type RxScratch = DatagramScratch<BYTES>;
    type TxScratch = P::TxScratch;
}

/// A profile with only its TX datagram pool, its scratch and exchange table resized.
///
/// Usually selected through [`crate::resources::Resources::tx_datagram`].
/// Other dimensions and the owned token representation are unchanged.
pub struct WithTxDatagram<P, const SLOTS: usize, const BYTES: usize>(core::marker::PhantomData<P>);

impl<P: MemoryProfile, const SLOTS: usize, const BYTES: usize> MemoryProfile
    for WithTxDatagram<P, SLOTS, BYTES>
{
    type Token = P::Token;
    const RX_DATAGRAM_SLOTS: usize = P::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = P::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = SLOTS;
    const TX_DATAGRAM_BYTES: usize = BYTES;
    const DEDUP_ENTRIES: usize = P::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = P::OBSERVE_ENTRIES;
    const RX_BODY_SLOTS: usize = P::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = P::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = P::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = P::TX_BODY_BYTES;

    type RxDatagram = P::RxDatagram;
    type TxDatagram = DatagramPool<SLOTS, BYTES>;
    type RxBody = P::RxBody;
    type TxBody = P::TxBody;
    type Dedup = P::Dedup;
    type Observe = P::Observe;
    type Exchange = ExchangeTable<SLOTS, P::Token>;
    type RxScratch = P::RxScratch;
    type TxScratch = DatagramScratch<BYTES>;
}

/// A profile with only its RX body pool resized.
///
/// Usually selected through [`crate::resources::Resources::rx_body`].
/// Other dimensions and the owned token representation are unchanged.
pub struct WithRxBody<P, const SLOTS: usize, const BYTES: usize>(core::marker::PhantomData<P>);

impl<P: MemoryProfile, const SLOTS: usize, const BYTES: usize> MemoryProfile
    for WithRxBody<P, SLOTS, BYTES>
{
    type Token = P::Token;
    const RX_DATAGRAM_SLOTS: usize = P::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = P::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = P::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = P::TX_DATAGRAM_BYTES;
    const DEDUP_ENTRIES: usize = P::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = P::OBSERVE_ENTRIES;
    const RX_BODY_SLOTS: usize = SLOTS;
    const RX_BODY_BYTES: usize = BYTES;
    const TX_BODY_SLOTS: usize = P::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = P::TX_BODY_BYTES;

    type RxDatagram = P::RxDatagram;
    type TxDatagram = P::TxDatagram;
    type RxBody = BodyPool<SLOTS, BYTES, P::Token>;
    type TxBody = P::TxBody;
    type Dedup = P::Dedup;
    type Observe = P::Observe;
    type Exchange = P::Exchange;
    type RxScratch = P::RxScratch;
    type TxScratch = P::TxScratch;
}

/// A profile with only its TX body pool resized.
///
/// Usually selected through [`crate::resources::Resources::tx_body`].
/// Other dimensions and the owned token representation are unchanged.
pub struct WithTxBody<P, const SLOTS: usize, const BYTES: usize>(core::marker::PhantomData<P>);

impl<P: MemoryProfile, const SLOTS: usize, const BYTES: usize> MemoryProfile
    for WithTxBody<P, SLOTS, BYTES>
{
    type Token = P::Token;
    const RX_DATAGRAM_SLOTS: usize = P::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = P::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = P::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = P::TX_DATAGRAM_BYTES;
    const DEDUP_ENTRIES: usize = P::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = P::OBSERVE_ENTRIES;
    const RX_BODY_SLOTS: usize = P::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = P::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = SLOTS;
    const TX_BODY_BYTES: usize = BYTES;

    type RxDatagram = P::RxDatagram;
    type TxDatagram = P::TxDatagram;
    type RxBody = P::RxBody;
    type TxBody = BodyPool<SLOTS, BYTES, P::Token>;
    type Dedup = P::Dedup;
    type Observe = P::Observe;
    type Exchange = P::Exchange;
    type RxScratch = P::RxScratch;
    type TxScratch = P::TxScratch;
}

macro_rules! impl_profile {
    (
        $name:ident
        rx_dgram = $rx_n:expr, $rx_b:expr;
        tx_dgram = $tx_n:expr, $tx_b:expr;
        rx_body = $rxb_n:expr, $rxb_b:expr;
        tx_body = $txb_n:expr, $txb_b:expr;
        dedup = $dedup:expr;
        observe = $obs:expr;
    ) => {
        impl MemoryProfile for $name {
            type Token = crate::message::Token;
            const RX_DATAGRAM_SLOTS: usize = $rx_n;
            const RX_DATAGRAM_BYTES: usize = $rx_b;
            const TX_DATAGRAM_SLOTS: usize = $tx_n;
            const TX_DATAGRAM_BYTES: usize = $tx_b;
            const DEDUP_ENTRIES: usize = $dedup;
            const OBSERVE_ENTRIES: usize = $obs;
            const RX_BODY_SLOTS: usize = $rxb_n;
            const RX_BODY_BYTES: usize = $rxb_b;
            const TX_BODY_SLOTS: usize = $txb_n;
            const TX_BODY_BYTES: usize = $txb_b;

            type RxDatagram = DatagramPool<$rx_n, $rx_b>;
            type TxDatagram = DatagramPool<$tx_n, $tx_b>;
            type RxBody = BodyPool<$rxb_n, $rxb_b>;
            type TxBody = BodyPool<$txb_n, $txb_b>;
            type Dedup = DedupTable<$dedup>;
            type Observe = ObserveTable<$obs>;
            type Exchange = ExchangeTable<$tx_n>;
            type RxScratch = DatagramScratch<$rx_b>;
            type TxScratch = DatagramScratch<$tx_b>;
        }

        const _: () = {
            assert!($rxb_n > 0 && $txb_n > 0);
            assert!($rxb_b > 0 && $txb_b > 0);
            assert!($rxb_b % 1024 == 0 && $txb_b % 1024 == 0);
        };
    };
}

/// Default capacity profile.
///
/// | Area | Count | Bytes |
/// | --- | ---: | ---: |
/// | RX / TX datagram | 4 | **1472** |
/// | RX / TX body (block-wise enabled only) | 2 | **4096** (= 4 × 1024) |
/// | Dedup entries | 8 | compact replay + optional TX pin |
/// | Observe entries | 4 | — |
///
/// 1472 is IPv4 UDP max on Ethernet (`1500 − 20 − 8`). 4096 is four max-size
/// Block/Q-Block blocks.
pub struct Default;

impl_profile! {
    Default
    rx_dgram = 4, 1472;
    tx_dgram = 4, 1472;
    rx_body = 2, 4096;
    tx_body = 2, 4096;
    dedup = 8;
    observe = 4;
}

/// Constrained / unknown-PMTU profile ([RFC 7252] §4.6 **1152**).
///
/// | Area | Count | Bytes |
/// | --- | ---: | ---: |
/// | RX / TX datagram | 2 | **1152** |
/// | RX / TX body (block-wise enabled only) | 1 | 4096 |
/// | Dedup entries | 4 | compact replay + optional TX pin |
/// | Observe entries | 2 | — |
///
/// Body bytes are not locked for this profile; 4096 is used when block-wise is
/// enabled so the same quantum as [`struct@Default`] applies. `.block_wise(false)`
/// still omits body pools entirely.
///
/// [RFC 7252]: https://www.rfc-editor.org/rfc/rfc7252#section-4.6
pub struct Constrained;

impl_profile! {
    Constrained
    rx_dgram = 2, 1152;
    tx_dgram = 2, 1152;
    rx_body = 1, 4096;
    tx_body = 1, 4096;
    dedup = 4;
    observe = 2;
}

/// Gateway pool starting sizes; not a peer-count or throughput guarantee.
pub struct Gateway;

impl_profile! {
    Gateway
    rx_dgram = 8, 1472;
    tx_dgram = 8, 1472;
    rx_body = 2, 8192;
    tx_body = 2, 8192;
    dedup = 16;
    observe = 8;
}

/// Host service pool starting sizes. Security associations are independently bounded.
pub struct Service;

impl_profile! {
    Service
    rx_dgram = 32, 1472;
    tx_dgram = 32, 1472;
    rx_body = 4, 16384;
    tx_body = 4, 16384;
    dedup = 64;
    observe = 32;
}

/// Define a custom fixed memory profile with named capacities and matching array types.
///
/// Use the resulting type with [`crate::resources::Resources`]. Body dimensions
/// are instantiated only when block-wise storage is enabled. All dimensions are
/// compile-time constants; these numbers exclude metadata and stack temporaries.
///
/// ```
/// coaptic::memory_profile! {
///     pub MyMemory {
///         rx_datagram: (2, 1152), tx_datagram: (3, 1152),
///         rx_body: (1, 4096), tx_body: (1, 4096),
///         dedup: 4, observe: 2,
///     }
/// }
/// let resources = coaptic::resources::Resources::<MyMemory>::new().client_calls::<2>();
/// let builder = coaptic::App::with_resources(resources);
/// ```
#[macro_export]
macro_rules! memory_profile {
    ($(#[$meta:meta])* $vis:vis $name:ident {
        token: $token:ty,
        rx_datagram: ($rx_n:expr, $rx_b:expr), tx_datagram: ($tx_n:expr, $tx_b:expr),
        rx_body: ($rxb_n:expr, $rxb_b:expr), tx_body: ($txb_n:expr, $txb_b:expr),
        dedup: $dedup:expr, observe: $obs:expr $(,)?
    }) => {
        $(#[$meta])*
        $vis struct $name;
        impl $crate::storage::MemoryProfile for $name {
            type Token = $token;
            const RX_DATAGRAM_SLOTS: usize = $rx_n;
            const RX_DATAGRAM_BYTES: usize = $rx_b;
            const TX_DATAGRAM_SLOTS: usize = $tx_n;
            const TX_DATAGRAM_BYTES: usize = $tx_b;
            const DEDUP_ENTRIES: usize = $dedup;
            const OBSERVE_ENTRIES: usize = $obs;
            const RX_BODY_SLOTS: usize = $rxb_n;
            const RX_BODY_BYTES: usize = $rxb_b;
            const TX_BODY_SLOTS: usize = $txb_n;
            const TX_BODY_BYTES: usize = $txb_b;
            type RxDatagram = $crate::storage::DatagramPool<$rx_n, $rx_b>;
            type TxDatagram = $crate::storage::DatagramPool<$tx_n, $tx_b>;
            type RxBody = $crate::storage::BodyPool<$rxb_n, $rxb_b, $token>;
            type TxBody = $crate::storage::BodyPool<$txb_n, $txb_b, $token>;
            type Dedup = $crate::storage::DedupTable<$dedup>;
            type Observe = $crate::storage::ObserveTable<$obs, $token>;
            type Exchange = $crate::storage::ExchangeTable<$tx_n, $token>;
            type RxScratch = $crate::storage::DatagramScratch<$rx_b>;
            type TxScratch = $crate::storage::DatagramScratch<$tx_b>;
        }
    };
    ($(#[$meta:meta])* $vis:vis $name:ident { $($fields:tt)* }) => {
        $crate::memory_profile! { $(#[$meta])* $vis $name {
            token: $crate::message::Token, $($fields)*
        } }
    };

}
