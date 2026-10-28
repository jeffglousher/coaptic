use coaptic::Endpoint;
use coaptic::storage::{Capacities, DatagramIo};
use coaptic::{App, BuildError, resources};

struct Idle;

#[derive(Default)]
struct CountingIo {
    sent: usize,
}

impl coaptic::storage::DatagramTransport for CountingIo {
    type Error = core::convert::Infallible;
}

impl DatagramIo for CountingIo {
    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        Ok(None)
    }

    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.sent += 1;
        Ok(bytes.len())
    }
}

macro_rules! check_discovery_uri_path_refusal {
    ($config:expr) => {{
        use coaptic::app::{PathError, RouteError, get};
        let mut builder = App::with_resources($config)
            .allow_unprotected()
            .deterministic_for_tests()
            .route("ok", get(route_status));
        assert_eq!(
            builder.try_route(".", get(route_status)).err(),
            Some(RouteError::Path(PathError::DotSegment))
        );
        let mut app = builder.bind(CountingIo::default()).unwrap();
        assert_eq!(app.site().len(), 1);
        assert_eq!(
            app.try_route(&[".."], get(route_status)).err(),
            Some(RouteError::Path(PathError::DotSegment))
        );
        let peer = Endpoint::v4([192, 0, 2, 2], 5683);
        assert!(matches!(
            app.get("a/../b").to(peer).send(0),
            Err(coaptic::app::Error::Path)
        ));
        assert_eq!(app.transport().sent, 0);
        let call = app.get("%2E").to(peer).send(0).unwrap();
        assert_eq!(app.transport().sent, 1);
        assert!(app.cancel(call));
        assert_eq!(app.site().len(), 1);
    }};
}

#[test]
fn discovery_uri_fixed_path_refusal_preserves_builder_app_and_io() {
    check_discovery_uri_path_refusal!(resources::SmallDevice::new().routes::<1>());
}

#[cfg(feature = "alloc")]
#[test]
fn discovery_uri_allocated_path_refusal_preserves_builder_app_and_io() {
    check_discovery_uri_path_refusal!(
        resources::SmallDevice::new()
            .routes::<1>()
            .allocated(64 * 1024)
    );
}

fn route_status(_: coaptic::Request<'_>) -> coaptic::Response<'static> {
    coaptic::Response::content(b"ready")
}

#[test]
fn route_removal_reclaims_fixed_builder_and_app_capacity() {
    use coaptic::app::{PathError, RouteError, get};
    let mut legacy = App::profile::<coaptic::profiles::Constrained>()
        .block_wise::<false>()
        .routes::<1>()
        .allow_unprotected()
        .deterministic_for_tests()
        .route("old", get(route_status));
    assert!(legacy.remove_route("old").unwrap());
    legacy.try_route("new", get(route_status)).unwrap();
    let mut legacy = legacy.bind(Idle).unwrap();
    assert_eq!(legacy.remove_route("new"), Ok(true));

    let mut configured = App::with_resources(resources::SmallDevice::new().routes::<1>())
        .allow_unprotected()
        .deterministic_for_tests()
        .route("old", get(route_status));
    assert_eq!(
        configured.remove_route("bad//path"),
        Err(PathError::EmptySegment)
    );
    assert_eq!(
        configured.try_route("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    assert!(configured.remove_route("old").unwrap());
    configured.try_route("new", get(route_status)).unwrap();
    let mut app = configured.bind(Idle).unwrap();
    assert_eq!(app.remove_route("missing"), Ok(false));
    assert_eq!(app.remove_route("new"), Ok(true));
    app.try_route("replacement", get(route_status)).unwrap();
    assert_eq!(app.site().len(), 1);
    app.poll(0).unwrap();
}

#[cfg(feature = "alloc")]
#[test]
fn route_removal_reclaims_allocated_capacity_after_call_recovery() {
    use coaptic::app::{RouteError, RouteRemovalError, get};
    let mut builder = App::with_resources(
        resources::SmallDevice::new()
            .routes::<1>()
            .allocated(64 * 1024),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("old", get(route_status));
    assert_eq!(builder.remove_route("old"), Ok(true));
    builder.try_route("target", get(route_status)).unwrap();
    let mut app = builder.bind(Idle).unwrap();
    let call = app
        .get("remote")
        .to(Endpoint::v4([127, 0, 0, 1], 5683))
        .send(0)
        .unwrap();
    assert_eq!(app.remove_route("target"), Err(RouteRemovalError::Busy));
    assert_eq!(
        app.try_route("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    assert!(app.cancel(call));
    assert_eq!(app.remove_route("target"), Err(RouteRemovalError::Busy));
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        coaptic::CallFailure::Cancelled
    );
    assert_eq!(app.remove_route("target"), Ok(true));
    app.try_route("fresh", get(route_status)).unwrap();
    app.poll(1).unwrap();
}

#[test]
fn checked_routes_preserve_fixed_builders_and_apps_after_refusal() {
    use coaptic::app::{RouteError, get};
    let mut legacy = App::profile::<coaptic::profiles::Constrained>()
        .block_wise::<false>()
        .routes::<1>()
        .deterministic_for_tests()
        .allow_unprotected();
    legacy.try_route("status", get(route_status)).unwrap();
    assert_eq!(
        legacy.try_route_path("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    let mut legacy = legacy.bind(Idle).unwrap();
    assert_eq!(legacy.site().len(), 1);
    legacy.try_route_path("status", get(route_status)).unwrap();

    let mut configured = App::with_resources(resources::SmallDevice::new().routes::<1>())
        .deterministic_for_tests()
        .allow_unprotected();
    configured
        .try_route_path("status", get(route_status))
        .unwrap();
    assert_eq!(
        configured.try_route("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    let mut app = configured.bind(Idle).unwrap();
    assert_eq!(
        app.try_route_path("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    app.try_route("status", get(route_status)).unwrap();
    assert_eq!(app.site().len(), 1);
    app.poll(0).unwrap();
}

#[cfg(feature = "alloc")]
#[test]
fn checked_routes_preserve_allocated_builder_and_app_after_refusal() {
    use coaptic::app::{RouteError, get};
    let config = resources::SmallDevice::new().routes::<1>();
    let mut builder = App::with_resources(config.allocated(64 * 1024))
        .deterministic_for_tests()
        .allow_unprotected();
    builder.try_route("status", get(route_status)).unwrap();
    assert_eq!(
        builder.try_route("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    let mut app = builder.bind(Idle).unwrap();
    assert_eq!(
        app.try_route("extra", get(route_status)).err(),
        Some(RouteError::Full)
    );
    app.try_route_path("status", get(route_status)).unwrap();
    assert_eq!(app.site().len(), 1);
    app.poll(0).unwrap();
}
impl coaptic::storage::DatagramTransport for Idle {
    type Error = core::convert::Infallible;
}

impl DatagramIo for Idle {
    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        Ok(None)
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        Ok(bytes.len())
    }
}

coaptic::memory_profile! {
    Custom {
        rx_datagram: (2, 512), tx_datagram: (3, 512),
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
                        .allow_unprotected()
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
        .allow_unprotected()
        .randomness(|_| false)
        .bind(Idle)
        .err();
    assert_eq!(error, Some(BuildError::RandomnessUnavailable));
    let mut app = App::with_resources(resources::Resources::<Custom>::new().client_calls::<0>())
        .allow_unprotected()
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
        .allow_unprotected()
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
    assert_eq!((c.rx_datagram_slots, c.rx_datagram_bytes), (2, 512));
    assert_eq!((c.tx_datagram_slots, c.tx_datagram_bytes), (3, 512));
    assert_eq!((c.dedup_entries, c.observe_entries), (2, 1));
}

#[test]
fn observe_override_preserves_other_dimensions_and_body_selection() {
    let base = resources::Device::new().pool_capacities();
    let configured = resources::Device::new()
        .observe_entries::<1>()
        .observe_entries::<6>()
        .client_calls::<2>()
        .security_requests::<3>();
    let mut expected = base;
    expected.observe_entries = 6;
    assert_eq!(configured.pool_capacities(), expected);
    assert_eq!(configured.client_capacity(), 2);
    assert_eq!(configured.security_request_capacity(), 3);
    assert_eq!(configured.route_capacity(), 8);
    assert_eq!(configured.deferred_capacity(), 2);
    let mut app = App::with_resources(configured)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert_eq!(app.engine().capacities(), expected);
    app.poll(0).unwrap();
    let disabled = resources::Device::new()
        .observe_entries::<0>()
        .block_wise::<false>();
    assert_eq!(disabled.pool_capacities().observe_entries, 0);
    assert!(!disabled.pool_capacities().has_body_pools());
}

#[test]
fn fixed_pool_overrides_preserve_unrelated_dimensions() {
    let base = resources::Resources::<Custom>::new().block_wise::<true>();
    let mut expected = base.pool_capacities();
    expected.rx_datagram_slots = 1;
    expected.rx_datagram_bytes = 256;
    assert_eq!(base.rx_datagram::<1, 256>().pool_capacities(), expected);

    let base = resources::Resources::<Custom>::new().block_wise::<true>();
    let mut expected = base.pool_capacities();
    expected.tx_datagram_slots = 5;
    expected.tx_datagram_bytes = 768;
    assert_eq!(base.tx_datagram::<5, 768>().pool_capacities(), expected);

    let base = resources::Resources::<Custom>::new().block_wise::<true>();
    let mut expected = base.pool_capacities();
    expected.rx_body_slots = Some(3);
    expected.rx_body_bytes = Some(1024);
    assert_eq!(base.rx_body::<3, 1024>().pool_capacities(), expected);

    let base = resources::Resources::<Custom>::new().block_wise::<true>();
    let mut expected = base.pool_capacities();
    expected.tx_body_slots = Some(2);
    expected.tx_body_bytes = Some(3072);
    assert_eq!(base.tx_body::<2, 3072>().pool_capacities(), expected);
}

#[test]
fn fixed_pool_overrides_compose_and_preserve_app_bounds() {
    let config = resources::Device::new()
        .routes::<3>()
        .client_calls::<2>()
        .deferred::<1>()
        .security_requests::<3>()
        .associations::<2>()
        .rx_datagram::<7, 1024>()
        .observe_entries::<1>()
        .tx_body::<2, 2048>()
        .tx_datagram::<3, 768>()
        .dedup_entries::<5>()
        .rx_body::<1, 1024>()
        .rx_datagram::<2, 256>();
    let expected = Capacities {
        rx_datagram_slots: 2,
        rx_datagram_bytes: 256,
        tx_datagram_slots: 3,
        tx_datagram_bytes: 768,
        dedup_entries: 5,
        observe_entries: 1,
        rx_body_slots: Some(1),
        rx_body_bytes: Some(1024),
        tx_body_slots: Some(2),
        tx_body_bytes: Some(2048),
    };
    assert_eq!(core::mem::size_of_val(&config), 0);
    assert_eq!(config.pool_capacities(), expected);
    assert_eq!(config.route_capacity(), 3);
    assert_eq!(config.client_capacity(), 2);
    assert_eq!(config.deferred_capacity(), 1);
    assert_eq!(config.security_request_capacity(), 3);
    assert_eq!(config.association_capacity(), 2);
    let mut app = App::with_resources(config)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert_eq!(app.engine().capacities(), expected);
    app.poll(0).unwrap();
}

#[test]
fn fixed_datagram_overrides_resize_storage_scratch_and_exchanges() {
    use coaptic::profiles::{WithRxDatagram, WithTxDatagram};
    use coaptic::storage::{Memory, MemoryProfile, SlotPool};

    type P = WithTxDatagram<WithRxDatagram<Custom, 1, 193>, 5, 769>;
    let _: resources::Resources<P> = resources::Resources::<Custom>::new()
        .rx_datagram::<1, 193>()
        .tx_datagram::<5, 769>();
    let mut memory = Memory::<P>::new();
    assert_eq!(memory.rx_datagram().slot_count(), 1);
    assert_eq!(memory.rx_datagram().slot_bytes(), 193);
    assert_eq!(memory.tx_datagram().slot_count(), 5);
    assert_eq!(memory.tx_datagram().slot_bytes(), 769);
    let rx = <P as MemoryProfile>::RxScratch::default();
    let mut tx = <P as MemoryProfile>::TxScratch::default();
    assert_eq!(rx.as_ref().len(), 193);
    assert_eq!(tx.as_mut().len(), 769);
    // The last byte must be usable, independently of RX and the base profile.
    tx.as_mut()[768] = 42;
    assert_eq!(tx.as_ref()[768], 42);
    let exchanges = memory.exchange_mut();
    assert_eq!(exchanges.slot_count(), 5);
    let first = exchanges.acquire().unwrap();
    for _ in 1..5 {
        assert!(exchanges.acquire().is_some());
    }
    assert!(exchanges.acquire().is_none());
    exchanges.release(first).unwrap();
    assert!(exchanges.acquire().is_some());

    // RX resizing alone must preserve the original TX scratch and table.
    type RxOnly = WithRxDatagram<Custom, 1, 193>;
    assert_eq!(
        <RxOnly as MemoryProfile>::TxScratch::default()
            .as_ref()
            .len(),
        512
    );
    assert_eq!(Memory::<RxOnly>::new().exchange().slot_count(), 3);
    type TxOnly = WithTxDatagram<Custom, 5, 769>;
    assert_eq!(
        <TxOnly as MemoryProfile>::RxScratch::default()
            .as_ref()
            .len(),
        512
    );
    assert_eq!(
        App::with_resources(resources::Resources::<Custom>::new().rx_datagram::<1, 193>())
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Idle)
            .err(),
        Some(BuildError::PacketHeadroom)
    );
    assert_eq!(
        App::with_resources(resources::Resources::<Custom>::new().tx_datagram::<1, 769>())
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Idle)
            .err(),
        Some(BuildError::PacketHeadroom)
    );
    let mut app = App::with_resources(
        resources::Resources::<Custom>::new()
            .rx_datagram::<2, 193>()
            .tx_datagram::<5, 769>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .bind(Idle)
    .unwrap();
    app.poll(0).unwrap();
}

#[test]
fn fixed_body_overrides_preserve_typestate_omission_and_reenable() {
    use coaptic::profiles::{WithRxBody, WithTxBody};
    use coaptic::storage::{Memory, SlotPool};

    type P = WithTxBody<WithRxBody<Custom, 2, 1024>, 3, 3072>;
    assert_eq!(
        core::mem::size_of::<Memory<P>>(),
        core::mem::size_of::<Memory<Custom>>()
    );
    let disabled = resources::Resources::<Custom>::new()
        .rx_body::<2, 1024>()
        .block_wise::<true>()
        .tx_body::<3, 3072>()
        .block_wise::<false>();
    assert!(!disabled.pool_capacities().has_body_pools());
    let disabled_app = App::with_resources(disabled)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert!(!disabled_app.engine().capacities().has_body_pools());
    let enabled = resources::Resources::<Custom>::new()
        .rx_body::<2, 1024>()
        .tx_body::<3, 3072>()
        .block_wise::<true>();
    let mut memory = Memory::<P>::with_block_wise();
    assert_eq!(memory.rx_body().slot_count(), 2);
    assert_eq!(memory.rx_body().slot_bytes(), 1024);
    assert_eq!(memory.tx_body().slot_count(), 3);
    assert_eq!(memory.tx_body().slot_bytes(), 3072);
    let slot = memory.tx_body_mut().acquire().unwrap();
    memory.tx_body_mut().set_len(slot, 3072).unwrap();
    assert!(memory.tx_body_mut().set_len(slot, 3073).is_err());
    memory.tx_body_mut().release(slot).unwrap();
    assert!(memory.tx_body_mut().acquire().is_some());
    let expected = enabled.pool_capacities();
    let app = App::with_resources(enabled)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert_eq!(app.engine().capacities(), expected);
}

type ResourceToken = coaptic::message::extended::Token<32>;
coaptic::memory_profile! {
    WideResources {
        token: ResourceToken,
        rx_datagram: (2, 256), tx_datagram: (3, 512),
        rx_body: (1, 1024), tx_body: (1, 1024),
        dedup: 2, observe: 1,
    }
}

#[test]
fn fixed_pool_overrides_preserve_custom_token_in_every_pool() {
    use coaptic::profiles::{
        WithDedup, WithObserve, WithRxBody, WithRxDatagram, WithTxBody, WithTxDatagram,
    };
    use coaptic::storage::{
        BodyPool, ExchangeEntry, ExchangeTable, Memory, ObserveTable, SlotPool,
    };

    type P = WithObserve<
        WithDedup<
            WithTxBody<
                WithRxBody<WithTxDatagram<WithRxDatagram<WideResources, 2, 192>, 4, 640>, 2, 2048>,
                1,
                3072,
            >,
            3,
        >,
        2,
    >;
    let config: resources::Resources<P, 8, true> = resources::Resources::<WideResources>::new()
        .rx_datagram::<2, 192>()
        .tx_datagram::<4, 640>()
        .rx_body::<2, 2048>()
        .tx_body::<1, 3072>()
        .dedup_entries::<3>()
        .observe_entries::<2>()
        .block_wise::<true>();
    let mut memory = Memory::<P>::with_block_wise();
    let _: &BodyPool<2, 2048, ResourceToken> = memory.rx_body();
    let _: &BodyPool<1, 3072, ResourceToken> = memory.tx_body();
    let _: &ObserveTable<2, ResourceToken> = memory.observe();
    let exchanges: &mut ExchangeTable<4, ResourceToken> = memory.exchange_mut();
    let mut bytes = [7; 32];
    bytes[31] = 91;
    let token = ResourceToken::new(&bytes).unwrap();
    let mut tx = coaptic::storage::DatagramPool::<1, 64>::new();
    let tx_slot = tx.acquire().unwrap();
    let entry = ExchangeEntry::new(
        token,
        Endpoint::v4([192, 0, 2, 1], 5683),
        coaptic::message::MessageId::new(1),
        tx_slot,
    );
    let exchange = exchanges.insert(entry).unwrap();
    assert_eq!(exchanges.entry(exchange).unwrap().token().as_bytes(), bytes);
    let mut app = App::with_resources(config)
        .token_length(32)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    let call = app
        .get("wide")
        .to(Endpoint::v4([192, 0, 2, 1], 5683))
        .send(0)
        .unwrap();
    assert_eq!(call.token().len(), 32);
    assert!(app.cancel(call));
    assert!(app.take_response(call).is_some());
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_observe_override_preserves_custom_pool_dimensions() {
    let mut expected = Capacities::from_profile::<Custom>();
    expected.tx_datagram_slots = 7;
    let config = resources::Resources::<Custom>::new()
        .allocated(128 * 1024)
        .capacities(expected)
        .observe_entries(9);
    expected.observe_entries = 9;
    assert_eq!(config.pool_capacities(), expected);
    let app = App::with_resources(config)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert_eq!(app.engine().capacities(), expected);
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_presets_require_budget_and_preserve_call_bounds() {
    macro_rules! check {
        ($preset:ty, $calls:expr) => {{
            let error = App::with_resources(<$preset>::new().allocated(0))
                .allow_unprotected()
                .deterministic_for_tests()
                .bind(Idle)
                .err()
                .unwrap();
            assert!(matches!(
                error,
                BuildError::HeapBudgetExceeded { limit: 0, .. }
            ));
            let mut app = App::with_resources(<$preset>::new().allocated(2 * 1024 * 1024))
                .allow_unprotected()
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
            .allow_unprotected()
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
            .allow_unprotected()
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
            .allow_unprotected()
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
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Idle)
            .is_ok()
    );
}

#[cfg(not(feature = "oscore"))]
#[test]
fn security_resource_bound_is_zero_sized_without_oscore() {
    let resources = resources::SmallDevice::new().security_requests::<64>();
    assert_eq!(resources.security_request_capacity(), 64);
    let mut app = App::with_resources(resources)
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Idle)
        .unwrap();
    assert_eq!(app.security_request_capacity(), 64);
    assert_eq!(app.security_metadata_bytes(), 0);
    app.poll(0).unwrap();
}
