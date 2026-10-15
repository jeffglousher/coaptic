//! App checks actual readable and writable scratch, independently of declarations.
use coaptic::storage::{DatagramIo, Memory, MemoryProfile};
use coaptic::{App, BuildError, profiles};
use std::cell::Cell;
struct Scratch<const READ: usize, const WRITE: usize>([u8; 1472]);
impl<const R: usize, const W: usize> Default for Scratch<R, W> {
    fn default() -> Self {
        Self([0; 1472])
    }
}
impl<const R: usize, const W: usize> AsRef<[u8]> for Scratch<R, W> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..R]
    }
}
impl<const R: usize, const W: usize> AsMut<[u8]> for Scratch<R, W> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0[..W]
    }
}
struct Profile<const RR: usize, const RW: usize, const TR: usize, const TW: usize>;
impl<const RR: usize, const RW: usize, const TR: usize, const TW: usize> MemoryProfile
    for Profile<RR, RW, TR, TW>
{
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
    type TxScratch = Scratch<TR, TW>;
}
struct NoIo;
impl DatagramIo for NoIo {
    type Error = ();
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
            .allow_plaintext()
            .bind_storage_with_entropy(NoIo, memory, &mut random)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    assert_eq!(draws.get(), 0);
    assert_eq!(
        App::profile::<Profile<RR, RW, TR, TW>>()
            .block_wise::<false>()
            .allow_plaintext()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    assert_eq!(draws.get(), 0);
    assert!(
        App::profile::<Profile<1472, 1472, 1472, 1472>>()
            .block_wise::<false>()
            .allow_plaintext()
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
            .allow_plaintext()
            .deterministic_for_tests()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::RandomnessAlreadyConfigured)
    );
    assert_eq!(
        App::profile::<Invalid>()
            .block_wise::<false>()
            .allow_plaintext()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    assert_eq!(draws.get(), 0);
    assert_eq!(
        App::builder()
            .allow_plaintext()
            .bind_with_entropy(NoIo, &mut random)
            .err(),
        Some(BuildError::RandomnessUnavailable)
    );
    assert_eq!(draws.get(), 1);
}
