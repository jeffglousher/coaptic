//! App checks actual readable and writable scratch, independently of declarations.
use coaptic::storage::{DatagramIo, Memory, MemoryProfile};
use coaptic::{App, BuildError, profiles};
use std::cell::Cell;
struct Scratch<const READ: usize, const WRITE: usize, const CAP: usize = WRITE>([u8; 1472]);
impl<const R: usize, const W: usize, const C: usize> Default for Scratch<R, W, C> {
    fn default() -> Self {
        Self([0; 1472])
    }
}
impl<const R: usize, const W: usize, const C: usize> AsRef<[u8]> for Scratch<R, W, C> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..R]
    }
}
impl<const R: usize, const W: usize, const C: usize> AsMut<[u8]> for Scratch<R, W, C> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0[..W]
    }
}
impl<const R: usize, const W: usize, const C: usize> coaptic::storage::ResidentScratch
    for Scratch<R, W, C>
{
    const CAPACITY: usize = C;
}
struct Profile<
    const RR: usize,
    const RW: usize,
    const TR: usize,
    const TW: usize,
    const RES_R: usize = 1472,
    const RES_W: usize = 1472,
    const RES_C: usize = RES_W,
>;
impl<
    const RR: usize,
    const RW: usize,
    const TR: usize,
    const TW: usize,
    const RES_R: usize,
    const RES_W: usize,
    const RES_C: usize,
> MemoryProfile for Profile<RR, RW, TR, TW, RES_R, RES_W, RES_C>
{
    type Token = <profiles::Default as MemoryProfile>::Token;
    const RX_DATAGRAM_SLOTS: usize = <profiles::Default as MemoryProfile>::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = <profiles::Default as MemoryProfile>::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = <profiles::Default as MemoryProfile>::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = <profiles::Default as MemoryProfile>::TX_DATAGRAM_BYTES;
    const RX_BODY_SLOTS: usize = <profiles::Default as MemoryProfile>::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = <profiles::Default as MemoryProfile>::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = <profiles::Default as MemoryProfile>::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = <profiles::Default as MemoryProfile>::TX_BODY_BYTES;
    const DEDUP_ENTRIES: usize = <profiles::Default as MemoryProfile>::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = <profiles::Default as MemoryProfile>::OBSERVE_ENTRIES;
    type RxDatagram = <profiles::Default as MemoryProfile>::RxDatagram;
    type TxDatagram = <profiles::Default as MemoryProfile>::TxDatagram;
    type RxBody = <profiles::Default as MemoryProfile>::RxBody;
    type TxBody = <profiles::Default as MemoryProfile>::TxBody;
    type Dedup = <profiles::Default as MemoryProfile>::Dedup;
    type Observe = <profiles::Default as MemoryProfile>::Observe;
    type Exchange = <profiles::Default as MemoryProfile>::Exchange;
    type RxScratch = Scratch<RR, RW>;
    type ResidentRxScratch = Scratch<RES_R, RES_W, RES_C>;
    type TxScratch = Scratch<TR, TW>;
}
struct NoIo;
impl coaptic::storage::DatagramTransport for NoIo {
    type Error = ();
}

impl DatagramIo for NoIo {
    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, coaptic::Endpoint)>, ()> {
        panic!("unexpected receive")
    }
    fn send(&mut self, _: coaptic::Endpoint, _: &[u8]) -> Result<usize, ()> {
        panic!("unexpected send")
    }
}
fn refuse<const RR: usize, const RW: usize, const TR: usize, const TW: usize>() {
    let draws = Cell::new(0);
    let mut random = |bytes: &mut [u8]| {
        draws.set(draws.get() + 1);
        bytes.fill(9);
        true
    };
    let memory = Memory::<Profile<RR, RW, TR, TW>>::default();
    assert_eq!(
        App::builder()
            .allow_unprotected()
            .bind_storage_with_entropy(NoIo, memory, &mut random)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    assert_eq!(draws.get(), 0);
    assert_eq!(
        App::profile::<Profile<RR, RW, TR, TW>>()
            .block_wise::<false>()
            .allow_unprotected()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    assert_eq!(draws.get(), 0);
    assert!(
        App::profile::<Profile<1472, 1472, 1472, 1472>>()
            .block_wise::<false>()
            .allow_unprotected()
            .bind_with_entropy(NoIo, &mut random)
            .is_ok()
    );
    assert_eq!(draws.get(), 1);
}
#[test]
fn actual_scratch_slices_refuse_and_exact_capacity_recovers() {
    refuse::<1471, 1472, 1472, 1472>();
    refuse::<1472, 1471, 1472, 1472>();
    refuse::<1472, 1472, 1471, 1472>();
    refuse::<1472, 1472, 1472, 1471>();
}

#[test]
fn admission_preserves_security_and_entropy_ambiguity_precedence() {
    let draws = Cell::new(0);
    let mut random = |_: &mut [u8]| {
        draws.set(draws.get() + 1);
        false
    };
    type Invalid = Profile<1471, 1472, 1472, 1472>;
    assert_eq!(
        App::profile::<Invalid>()
            .block_wise::<false>()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::SecurityRequired)
    );
    assert_eq!(
        App::profile::<Invalid>()
            .block_wise::<false>()
            .allow_unprotected()
            .deterministic_for_tests()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::RandomnessAlreadyConfigured)
    );
    assert_eq!(
        App::profile::<Invalid>()
            .block_wise::<false>()
            .allow_unprotected()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    assert_eq!(draws.get(), 0);
    assert_eq!(
        App::builder()
            .allow_unprotected()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::RandomnessUnavailable)
    );
    assert_eq!(draws.get(), 1);
}

// Temporary scratch may be thread-local even when App's owned pools are not.
struct LocalScratch {
    bytes: [u8; 1472],
    _local: std::rc::Rc<()>,
}
impl Default for LocalScratch {
    fn default() -> Self {
        Self {
            bytes: [0; 1472],
            _local: std::rc::Rc::new(()),
        }
    }
}
impl AsRef<[u8]> for LocalScratch {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl AsMut<[u8]> for LocalScratch {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
struct LocalScratchProfile;
impl MemoryProfile for LocalScratchProfile {
    type Token = <profiles::Default as MemoryProfile>::Token;
    const RX_DATAGRAM_SLOTS: usize = <profiles::Default as MemoryProfile>::RX_DATAGRAM_SLOTS;
    const RX_DATAGRAM_BYTES: usize = <profiles::Default as MemoryProfile>::RX_DATAGRAM_BYTES;
    const TX_DATAGRAM_SLOTS: usize = <profiles::Default as MemoryProfile>::TX_DATAGRAM_SLOTS;
    const TX_DATAGRAM_BYTES: usize = <profiles::Default as MemoryProfile>::TX_DATAGRAM_BYTES;
    const RX_BODY_SLOTS: usize = <profiles::Default as MemoryProfile>::RX_BODY_SLOTS;
    const RX_BODY_BYTES: usize = <profiles::Default as MemoryProfile>::RX_BODY_BYTES;
    const TX_BODY_SLOTS: usize = <profiles::Default as MemoryProfile>::TX_BODY_SLOTS;
    const TX_BODY_BYTES: usize = <profiles::Default as MemoryProfile>::TX_BODY_BYTES;
    const DEDUP_ENTRIES: usize = <profiles::Default as MemoryProfile>::DEDUP_ENTRIES;
    const OBSERVE_ENTRIES: usize = <profiles::Default as MemoryProfile>::OBSERVE_ENTRIES;
    type RxDatagram = <profiles::Default as MemoryProfile>::RxDatagram;
    type TxDatagram = <profiles::Default as MemoryProfile>::TxDatagram;
    type RxBody = <profiles::Default as MemoryProfile>::RxBody;
    type TxBody = <profiles::Default as MemoryProfile>::TxBody;
    type Dedup = <profiles::Default as MemoryProfile>::Dedup;
    type Observe = <profiles::Default as MemoryProfile>::Observe;
    type Exchange = <profiles::Default as MemoryProfile>::Exchange;
    type RxScratch = LocalScratch;
    type ResidentRxScratch = coaptic::storage::DatagramScratch<1472>;
    type TxScratch = coaptic::storage::DatagramScratch<1472>;
}
#[test]
fn temporary_scratch_auto_traits_do_not_restrict_app() {
    fn thread_safe<T: Send + Sync>() {}
    thread_safe::<App<LocalScratchProfile, (), 8, false>>();
    thread_safe::<App<LocalScratchProfile, (), 8, true>>();
}

#[test]
fn resident_scratch_capacity_is_checked_before_entropy_only_when_used() {
    fn check<const R: usize, const W: usize>() {
        let draws = Cell::new(0);
        let mut random = |bytes: &mut [u8]| {
            draws.set(draws.get() + 1);
            bytes.fill(9);
            true
        };
        assert_eq!(
            App::profile::<Profile<1472, 1472, 1472, 1472, R, W>>()
                .block_wise::<true>()
                .allow_unprotected()
                .bind_with_entropy(NoIo, &mut random)
                .err(),
            Some(BuildError::SizeMismatch)
        );
        assert_eq!(draws.get(), 0);
        assert!(
            App::profile::<Profile<1472, 1472, 1472, 1472, R, W>>()
                .block_wise::<false>()
                .allow_unprotected()
                .bind_with_entropy(NoIo, &mut random)
                .is_ok()
        );
        assert_eq!(draws.get(), 1);
    }
    check::<1471, 1472>();
    check::<1472, 1471>();
}

#[test]
fn resident_capacity_declaration_must_match_actual_writable_slice() {
    fn refuse<const CAP: usize>() {
        let draws = Cell::new(0);
        let mut random = |bytes: &mut [u8]| {
            draws.set(draws.get() + 1);
            bytes.fill(9);
            true
        };
        assert_eq!(
            App::profile::<Profile<1472, 1472, 1472, 1472, 1472, 1472, CAP>>()
                .block_wise::<true>()
                .allow_unprotected()
                .bind_with_entropy(NoIo, &mut random)
                .err(),
            Some(BuildError::SizeMismatch)
        );
        assert_eq!(draws.get(), 0);
        assert!(
            App::profile::<Profile<1472, 1472, 1472, 1472, 1472, 1472, CAP>>()
                .block_wise::<false>()
                .allow_unprotected()
                .bind_with_entropy(NoIo, &mut random)
                .is_ok()
        );
        assert!(draws.get() > 0);
    }
    refuse::<1471>();
    refuse::<1473>();
}
