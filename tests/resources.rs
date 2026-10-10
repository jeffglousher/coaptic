use coaptic::Endpoint;
use coaptic::storage::{Capacities, DatagramIo};
use coaptic::{App, BuildError, resources};

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

coaptic::memory_profile! {
    Custom {
        rx_datagram: (1, 512), tx_datagram: (3, 512),
        rx_body: (1, 2048), tx_body: (1, 2048),
        dedup: 2, observe: 1,
    }
}

macro_rules! fixed_preset_test {
    ($test:ident, $preset:ty, $calls:expr) => {
        #[test]
        fn $test() {
            // Large fixed presets need explicit construction-stack headroom.
            // This is a host API test, not a target stack qualification.
            std::thread::Builder::new()
                .stack_size(8 * 1024 * 1024)
                .spawn(|| {
                    assert_eq!(
                        App::with_resources(<$preset>::new())
                            .deterministic_for_tests()
                            .bind(Idle)
                            .err(),
                        Some(BuildError::SecurityRequired)
                    );
                    let mut app = App::with_resources(<$preset>::new())
                        .allow_plaintext()
                        .deterministic_for_tests()
                        .bind(Idle)
                        .unwrap();
                    assert_eq!(app.client_capacity(), $calls);
                    assert_eq!(
                        app.engine().capacities(),
                        <$preset>::new().pool_capacities()
                    );
                    app.poll(0).unwrap();
                })
                .unwrap()
                .join()
                .unwrap();
        }
    };
}
fixed_preset_test!(small_device_fixed, resources::SmallDevice, 1);
fixed_preset_test!(device_fixed, resources::Device, 4);
fixed_preset_test!(gateway_fixed, resources::Gateway, 4);
fixed_preset_test!(service_fixed, resources::Service, 4);

#[test]
fn custom_resources_refuse_entropy_failure_and_recover() {
    let config = resources::Resources::<Custom>::new()
        .routes::<3>()
        .client_calls::<2>()
        .deferred::<1>();
    let error = App::with_resources(config)
        .allow_plaintext()
        .randomness(|_| false)
        .bind(Idle)
        .err();
    assert_eq!(error, Some(BuildError::RandomnessUnavailable));
    let mut app = App::with_resources(resources::Resources::<Custom>::new().client_calls::<0>())
        .allow_plaintext()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert_eq!(app.client_capacity(), 0);
    assert!(matches!(
        app.get("x").to(Endpoint::v4([127, 0, 0, 1], 5683)).send(0),
        Err(coaptic::app::Error::Saturated)
    ));
    app.poll(1).unwrap();
}

#[test]
fn calls_are_customized_before_binding_and_recover_after_expiry() {
    let mut app = App::with_resources(resources::SmallDevice::new().client_calls::<1>())
        .allow_plaintext()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    let peer = Endpoint::v4([127, 0, 0, 1], 5683);
    let call = app.get("x").to(peer).send(0).unwrap();
    assert!(matches!(
        app.get("x").to(peer).send(0),
        Err(coaptic::app::Error::Saturated)
    ));
    app.poll(1_000_000).unwrap();
    // Collection releases retained completion metadata as well as the live slot.
    assert!(app.take_response(call).is_some());
    assert!(app.get("x").to(peer).send(1_000_001).is_ok());
}

#[test]
fn custom_macro_carries_named_dimensions() {
    let c = Capacities::from_profile::<Custom>();
    assert_eq!((c.rx_datagram_slots, c.rx_datagram_bytes), (1, 512));
    assert_eq!((c.tx_datagram_slots, c.tx_datagram_bytes), (3, 512));
    assert_eq!((c.dedup_entries, c.observe_entries), (2, 1));
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_presets_require_budget_and_preserve_call_bounds() {
    macro_rules! check {
        ($preset:ty, $calls:expr) => {{
            let error = App::with_resources(<$preset>::new().allocated(0))
                .allow_plaintext()
                .deterministic_for_tests()
                .bind(Idle)
                .err()
                .unwrap();
            assert!(matches!(
                error,
                BuildError::HeapBudgetExceeded { limit: 0, .. }
            ));
            let mut app = App::with_resources(<$preset>::new().allocated(2 * 1024 * 1024))
                .allow_plaintext()
                .deterministic_for_tests()
                .bind(Idle)
                .unwrap();
            assert_eq!(app.client_capacity(), $calls);
            assert_eq!(
                app.engine().capacities(),
                <$preset>::new().pool_capacities()
            );
            app.poll(0).unwrap();
        }};
    }
    check!(resources::SmallDevice, 1);
    check!(resources::Device, 4);
    check!(resources::Gateway, 4);
    check!(resources::Service, 4);
}

#[cfg(feature = "alloc")]
#[test]
fn incompatible_heap_dimensions_refuse_then_valid_configuration_recovers() {
    let base = Capacities::from_profile::<Custom>();
    let mut bodies = base.with_block_wise::<Custom>();
    let config = resources::Resources::<Custom>::new()
        .allocated(64 * 1024)
        .capacities(bodies);
    assert_eq!(
        App::with_resources(config)
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Idle)
            .err(),
        Some(BuildError::UnexpectedBodyPools)
    );
    bodies.tx_body_bytes = Some(1025);
    let config = resources::Resources::<Custom>::new()
        .block_wise::<true>()
        .allocated(64 * 1024)
        .capacities(bodies);
    assert_eq!(
        App::with_resources(config)
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Idle)
            .err(),
        Some(BuildError::BodyBytesNotMultipleOf1024)
    );
    let mut large = base;
    large.rx_datagram_bytes = 1473;
    let config = resources::Resources::<Custom>::new()
        .allocated(64 * 1024)
        .capacities(large);
    assert_eq!(
        App::with_resources(config)
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Idle)
            .err(),
        Some(BuildError::SizeMismatch)
    );
    let config = resources::Resources::<Custom>::new()
        .allocated(64 * 1024)
        .capacities(base);
    assert!(
        App::with_resources(config)
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Idle)
            .is_ok()
    );
}
