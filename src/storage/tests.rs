//! Named storage-layer tests from `knowledge/memory.md` plus body-pool checks.

#[cfg(feature = "alloc")]
use super::Capacities;
use super::Engine;
use super::EngineBuilder;
use super::Memory;
use super::MemoryProfile;
use super::SlotError;
use super::Storage;
use super::WithBodies;
use super::profiles;
use crate::error::BuildError;

fn build_default() -> Engine<Memory<profiles::Default>> {
    EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::new())
        .expect("default no-body build")
}

fn build_default_bodies() -> Engine<Memory<profiles::Default, WithBodies<profiles::Default>>> {
    EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(true)
        .build(Memory::<profiles::Default>::with_block_wise())
        .expect("default block-wise build")
}

fn fill_until_saturated<S: Storage>(engine: &mut Engine<S>, slots: usize) {
    for _ in 0..slots {
        assert!(engine.acquire_rx().is_some());
    }
    assert!(engine.acquire_rx().is_none());
    assert_eq!(engine.rx_occupied(), slots);
}

fn reuse_after_release<S: Storage>(engine: &mut Engine<S>, slots: usize) {
    let mut first = None;
    for i in 0..slots {
        let id = engine.acquire_rx().expect("slot before saturation");
        if i == 1 {
            first = Some(id);
        }
    }
    assert!(engine.acquire_rx().is_none());
    let released = first.expect("need at least two slots");
    engine.release_rx(released).expect("release occupied");
    let reused = engine.acquire_rx().expect("reuse after release");
    assert_eq!(reused, released);
    assert!(engine.acquire_rx().is_none());
}

fn cursor_skips_released_zero<S: Storage>(engine: &mut Engine<S>) {
    let first = engine.acquire_rx().expect("first acquire");
    assert_eq!(first.index(), 0);
    assert_eq!(engine.rx_cursor(), 1);
    engine.release_rx(first).expect("release first");
    let second = engine.acquire_rx().expect("second acquire");
    assert_ne!(second.index(), 0);
    assert_ne!(second, first);
}

fn pool_suite<S: Storage>(engine: &mut Engine<S>, rx_slots: usize) {
    fill_until_saturated(engine, rx_slots);
    while engine.rx_occupied() > 0 {
        // Drain by walking possible ids.
        let n = engine.storage_mut().rx_datagram().slot_count();
        for i in 0..n {
            let id = crate::SlotId::from_index(i);
            let _ = engine.release_rx(id);
        }
    }
    cursor_skips_released_zero(engine);
    while engine.rx_occupied() > 0 {
        let n = engine.storage_mut().rx_datagram().slot_count();
        for i in 0..n {
            let id = crate::SlotId::from_index(i);
            let _ = engine.release_rx(id);
        }
    }
    reuse_after_release(engine, rx_slots);
}

#[test]
fn build_mismatch() {
    let storage = Memory::<profiles::Default>::new();
    let err = EngineBuilder::new()
        .profile::<profiles::Constrained>()
        .block_wise(false)
        .build(storage)
        .expect_err("1472/4 Default does not match 1152/2 Constrained");
    assert_eq!(err, BuildError::SizeMismatch);
}

#[test]
fn build_body_bytes_not_multiple_of_1024() {
    let storage = Memory::<profiles::Default>::with_block_wise();
    let err = EngineBuilder::new()
        .profile::<profiles::Default>()
        .rx_body(2, 1000)
        .block_wise(true)
        .build(storage)
        .expect_err("1000 is not a multiple of 1024");
    assert_eq!(err, BuildError::BodyBytesNotMultipleOf1024);
}

#[test]
fn block_wise_false_has_no_body_pools() {
    let mut engine = build_default();
    assert!(!engine.has_body_pools());
    assert!(engine.capacities().rx_body_slots.is_none());
    assert!(engine.capacities().rx_body_bytes.is_none());
    assert!(engine.capacities().tx_body_slots.is_none());
    assert!(engine.capacities().tx_body_bytes.is_none());
    assert!(engine.storage_mut().rx_body().is_none());
    assert!(engine.storage_mut().tx_body().is_none());
    assert!(engine.acquire_rx_body().is_none());
    assert!(engine.acquire_tx_body().is_none());
    assert_eq!(
        engine.release_rx_body(crate::SlotId::from_index(0)),
        Err(SlotError::InvalidSlot)
    );

    let err = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(true)
        .build(Memory::<profiles::Default>::new())
        .expect_err("no-body Memory cannot satisfy block_wise(true)");
    assert_eq!(err, BuildError::MissingBodyPools);

    let err = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::with_block_wise())
        .expect_err("WithBodies cannot satisfy block_wise(false)");
    assert_eq!(err, BuildError::UnexpectedBodyPools);
}

#[test]
fn acquire_until_full_then_saturation() {
    let mut engine = build_default();
    fill_until_saturated(&mut engine, profiles::Default::RX_DATAGRAM_SLOTS);
}

#[test]
fn release_and_reuse() {
    let mut engine = build_default();
    reuse_after_release(&mut engine, profiles::Default::RX_DATAGRAM_SLOTS);
}

#[test]
fn rotating_cursor_does_not_restart_at_zero() {
    let mut engine = build_default();
    cursor_skips_released_zero(&mut engine);
}

#[test]
fn rotate_advances_search_start() {
    let mut engine = build_default();
    assert_eq!(engine.rx_cursor(), 0);
    engine.rotate_rx();
    assert_eq!(engine.rx_cursor(), 1);
    let id = engine.acquire_rx().expect("acquire after rotate");
    assert_eq!(id.index(), 1);
}

#[test]
fn no_alloc_backends_pass_pool_suite() {
    let mut engine = build_default();
    pool_suite(&mut engine, profiles::Default::RX_DATAGRAM_SLOTS);

    let mut constrained = EngineBuilder::new()
        .profile::<profiles::Constrained>()
        .block_wise(false)
        .build(Memory::<profiles::Constrained>::new())
        .expect("constrained");
    pool_suite(&mut constrained, profiles::Constrained::RX_DATAGRAM_SLOTS);
}

#[test]
fn block_wise_body_pools_acquire_and_saturate() {
    let mut engine = build_default_bodies();
    assert!(engine.has_body_pools());
    assert_eq!(
        engine.capacities().rx_body_bytes,
        Some(profiles::Default::RX_BODY_BYTES)
    );

    for _ in 0..profiles::Default::RX_BODY_SLOTS {
        assert!(engine.acquire_rx_body().is_some());
    }
    assert!(engine.acquire_rx_body().is_none());

    let first = {
        let mut engine = build_default_bodies();
        let a = engine.acquire_rx_body().expect("body 0");
        engine.release_rx_body(a).expect("release body");
        let b = engine.acquire_rx_body().expect("should skip 0");
        assert_ne!(b, a);
        assert_eq!(
            engine.rx_body_cursor(),
            Some(2 % profiles::Default::RX_BODY_SLOTS)
        );
        a
    };
    let _ = first;
}

#[test]
fn method_by_method_matches_storage() {
    let storage = Memory::<profiles::Default>::new();
    let engine = EngineBuilder::new()
        .rx_datagram(
            profiles::Default::RX_DATAGRAM_SLOTS,
            profiles::Default::RX_DATAGRAM_BYTES,
        )
        .tx_datagram(
            profiles::Default::TX_DATAGRAM_SLOTS,
            profiles::Default::TX_DATAGRAM_BYTES,
        )
        .dedup(profiles::Default::DEDUP_ENTRIES)
        .observe(profiles::Default::OBSERVE_ENTRIES)
        .block_wise(false)
        .build(storage)
        .expect("method-by-method");
    assert!(!engine.has_body_pools());
}

#[test]
fn tables_acquire_release_rotate() {
    let mut engine = build_default();
    let d = engine.acquire_dedup().expect("dedup");
    let o = engine.acquire_observe().expect("observe");
    engine.rotate_dedup();
    engine.rotate_observe();
    engine.release_dedup(d).expect("release dedup");
    engine.release_observe(o).expect("release observe");
}

#[cfg(feature = "alloc")]
mod alloc_backend {
    use super::*;

    fn build_alloc(block_wise: bool) -> Engine<crate::AllocMemory> {
        let caps = if block_wise {
            Capacities::from_profile::<profiles::Default>().with_block_wise::<profiles::Default>()
        } else {
            Capacities::from_profile::<profiles::Default>()
        };
        EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(block_wise)
            .build_alloc(caps)
            .expect("build_alloc")
    }

    #[test]
    fn alloc_engine_encodes_and_decodes_tx_slot() {
        let mut engine = build_alloc(false);
        let id = engine.acquire_tx().expect("tx");
        let cf = crate::ContentFormat::JSON.encode();
        let mut opts = crate::OptionsBuilder::<2>::new();
        opts.push(crate::Opt::uri_path("heap")).expect("path");
        opts.push(crate::Opt::content_format(&cf)).expect("cf");
        let msg = crate::Message::new(
            crate::Type::Confirmable,
            crate::Code::GET,
            crate::MessageId::new(7),
        )
        .with_options(opts.as_slice());
        engine.encode_tx(id, &msg).expect("encode");
        let parsed = engine.decode_tx(id).expect("decode");
        assert_eq!(parsed.uri_path().next(), Some(Ok("heap")));
        assert_eq!(
            parsed.content_format(),
            Some(Ok(crate::ContentFormat::JSON))
        );
    }

    #[test]
    fn alloc_and_no_alloc_backends_pass_the_same_pool_tests() {
        let mut stack = build_default();
        let mut heap = build_alloc(false);
        pool_suite(&mut stack, profiles::Default::RX_DATAGRAM_SLOTS);
        pool_suite(&mut heap, profiles::Default::RX_DATAGRAM_SLOTS);
    }

    #[test]
    fn alloc_build_mismatch() {
        let caps = Capacities::from_profile::<profiles::Default>();
        let err = EngineBuilder::new()
            .profile::<profiles::Constrained>()
            .block_wise(false)
            .build_alloc(caps)
            .expect_err("alloc size mismatch");
        assert_eq!(err, BuildError::SizeMismatch);
    }

    #[test]
    fn alloc_body_bytes_not_multiple_of_1024() {
        let mut caps =
            Capacities::from_profile::<profiles::Default>().with_block_wise::<profiles::Default>();
        caps.rx_body_bytes = Some(1000);
        let err = EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(true)
            .build_alloc(caps)
            .expect_err("alloc body quantum");
        assert_eq!(err, BuildError::BodyBytesNotMultipleOf1024);
    }

    #[test]
    fn alloc_block_wise_false_has_no_body_pools() {
        let mut engine = build_alloc(false);
        assert!(!engine.has_body_pools());
        assert!(engine.acquire_rx_body().is_none());
        assert!(engine.storage_mut().rx_body().is_none());
    }

    #[test]
    fn alloc_acquire_until_full_then_saturation() {
        let mut engine = build_alloc(false);
        fill_until_saturated(&mut engine, profiles::Default::RX_DATAGRAM_SLOTS);
    }

    #[test]
    fn alloc_release_and_reuse() {
        let mut engine = build_alloc(false);
        reuse_after_release(&mut engine, profiles::Default::RX_DATAGRAM_SLOTS);
    }

    #[test]
    fn alloc_rotating_cursor_does_not_restart_at_zero() {
        let mut engine = build_alloc(false);
        cursor_skips_released_zero(&mut engine);
    }

    #[test]
    fn alloc_block_wise_bodies() {
        let mut engine = build_alloc(true);
        assert!(engine.has_body_pools());
        for _ in 0..profiles::Default::RX_BODY_SLOTS {
            assert!(engine.acquire_rx_body().is_some());
        }
        assert!(engine.acquire_rx_body().is_none());
    }

    #[test]
    fn alloc_zero_body_capacity_is_error() {
        let mut caps =
            Capacities::from_profile::<profiles::Default>().with_block_wise::<profiles::Default>();
        caps.rx_body_slots = Some(0);
        let err = EngineBuilder::new()
            .profile::<profiles::Default>()
            .rx_body(0, 4096)
            .block_wise(true)
            .build_alloc(caps)
            .expect_err("zero is not disabled");
        assert_eq!(err, BuildError::ZeroBodyCapacity);
    }
}
