use coaptic::error::BuildError;
use coaptic::storage::{DatagramIo, Endpoint, Memory, WithBodies};
use coaptic::{App, profiles};

struct Idle;

impl DatagramIo for Idle {
    type Error = core::convert::Infallible;

    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        Ok(None)
    }

    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        Ok(bytes.len())
    }
}

#[test]
fn caller_owned_storage_preserves_body_presence_validation() {
    let result = App::builder()
        .deterministic_for_tests()
        .allow_plaintext()
        .bind_storage(
            Idle,
            Memory::<profiles::Default, WithBodies<profiles::Default>>::new(),
        );
    assert!(matches!(result, Err(BuildError::UnexpectedBodyPools)));
    let result = App::builder()
        .block_wise::<true>()
        .deterministic_for_tests()
        .allow_plaintext()
        .bind_storage(Idle, Memory::<profiles::Default>::new());
    assert!(matches!(result, Err(BuildError::MissingBodyPools)));
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_storage_preserves_body_dimension_validation() {
    use coaptic::storage::Capacities;

    let plain = Capacities::from_profile::<profiles::Default>();
    let bodies = plain.with_block_wise::<profiles::Default>();
    let result = App::builder()
        .deterministic_for_tests()
        .allow_plaintext()
        .bind_alloc(Idle, bodies);
    assert!(matches!(result, Err(BuildError::UnexpectedBodyPools)));
    let mut incomplete = bodies;
    incomplete.tx_body_bytes = None;
    let mut zero = bodies;
    zero.rx_body_slots = Some(0);
    let mut unaligned = bodies;
    unaligned.rx_body_bytes = Some(1025);
    for (capacity, expected) in [
        (plain, BuildError::MissingBodyPools),
        (incomplete, BuildError::IncompleteBodyCapacities),
        (zero, BuildError::ZeroBodyCapacity),
        (unaligned, BuildError::BodyBytesNotMultipleOf1024),
    ] {
        let result = App::builder()
            .block_wise::<true>()
            .deterministic_for_tests()
            .allow_plaintext()
            .bind_alloc(Idle, capacity);
        match result {
            Err(error) => assert_eq!(error, expected),
            Ok(_) => panic!("invalid body dimensions were accepted"),
        }
    }
}
