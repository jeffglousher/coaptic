//! Named storage-layer tests from `knowledge/memory.md` plus body-pool checks.

#[cfg(feature = "alloc")]
use super::Capacities;
use super::DatagramPool;
use super::DedupEntry;
use super::DedupKey;
use super::DedupTable;
use super::Endpoint;
use super::Engine;
use super::EngineBuilder;
use super::Memory;
use super::MemoryProfile;
use super::PendingCon;
use super::SlotError;
use super::SlotId;
use super::SlotPool;
use super::Storage;
use super::WithBodies;
use super::profiles;
use crate::error::BuildError;
use crate::message::{Code, Message, MessageId, Type, empty_ack, empty_rst, encode};

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

fn sample_datagram(id: u16) -> ([u8; 16], usize) {
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(id));
    let mut buf = [0u8; 16];
    let n = encode(&msg, &mut buf).expect("encode sample");
    (buf, n)
}

#[test]
fn engine_write_rx_associates_endpoint() {
    let mut engine = build_default();
    let id = engine.acquire_rx().expect("rx");
    let ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let (buf, n) = sample_datagram(0x1111);
    assert_eq!(engine.rx_endpoint(id), None);
    assert_eq!(engine.write_rx(id, &buf[..n], ep).expect("write"), n);
    assert_eq!(engine.rx_endpoint(id), Some(ep));
    let parsed = engine.decode_rx(id).expect("decode");
    assert_eq!(parsed.message_id(), MessageId::new(0x1111));

    engine.release_rx(id).expect("release");
    assert_eq!(engine.rx_endpoint(id), None);
    while engine.acquire_rx().is_some() {}
    engine.release_rx(id).expect("free the written slot");
    let reused = engine.acquire_rx().expect("reuse same slot");
    assert_eq!(reused, id);
    assert_eq!(engine.rx_endpoint(reused), None);
}

#[test]
fn engine_set_tx_endpoint() {
    let mut engine = build_default();
    let id = engine.acquire_tx().expect("tx");
    let ep = Endpoint::v6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 5683);
    engine.set_tx_endpoint(id, ep).expect("set");
    assert_eq!(engine.tx_endpoint(id), Some(ep));
    assert_eq!(engine.set_rx_endpoint(id, ep), Err(SlotError::NotOccupied));
}

#[test]
fn dedup_hit_miss_and_capacity() {
    let mut table = DedupTable::<2>::new();
    let a = DedupEntry::new(MessageId::new(1), Endpoint::v4([192, 0, 2, 1], 5683));
    let b = DedupEntry::new(MessageId::new(2), Endpoint::v4([192, 0, 2, 1], 5683));
    let miss_id = DedupEntry::new(MessageId::new(1), Endpoint::v4([192, 0, 2, 2], 5683));
    let miss_port = DedupEntry::new(MessageId::new(1), Endpoint::v4([192, 0, 2, 1], 5684));
    let extra = DedupEntry::new(MessageId::new(3), Endpoint::v4([192, 0, 2, 1], 5683));

    assert_eq!(table.lookup(a.key()), None);
    let id_a = table.insert(a).expect("insert a");
    assert_eq!(table.lookup(a.key()), Some(id_a));
    assert_eq!(table.insert(a).expect("idempotent"), id_a);
    assert_eq!(table.occupied_count(), 1);
    assert_eq!(table.lookup(miss_id.key()), None);
    assert_eq!(table.lookup(miss_port.key()), None);

    let id_b = table.insert(b).expect("insert b");
    assert_ne!(id_a, id_b);
    assert!(table.insert(extra).is_none());
    assert_eq!(table.occupied_count(), 2);
    assert_eq!(table.entry(id_a), Some(a));

    assert!(table.remove(a.key()));
    assert_eq!(table.lookup(a.key()), None);
    assert!(!table.remove(a.key()));
    let id_extra = table.insert(extra).expect("room after remove");
    assert_eq!(id_extra, id_a);
}

#[test]
fn engine_dedup_insert_lookup_remove() {
    let mut engine = build_default();
    let ep = Endpoint::v4([198, 51, 100, 7], 5683);
    let key = DedupKey::new(MessageId::new(9), ep);
    let entry = DedupEntry::from(key);
    assert_eq!(engine.lookup_dedup(key), None);
    let id = engine.insert_dedup(entry).expect("insert");
    assert_eq!(engine.lookup_dedup(key), Some(id));
    assert_eq!(engine.dedup_entry(id), Some(entry));
    assert!(engine.remove_dedup(key));
    assert_eq!(engine.lookup_dedup(key), None);
}

#[test]
fn engine_dedup_fills_to_capacity() {
    let mut engine = build_default();
    let n = profiles::Default::DEDUP_ENTRIES;
    let ep = Endpoint::v4([203, 0, 113, 1], 5683);
    for i in 0..n {
        let entry = DedupEntry::new(MessageId::new(i as u16), ep);
        assert!(engine.insert_dedup(entry).is_some());
    }
    let overflow = DedupEntry::new(MessageId::new(n as u16), ep);
    assert!(engine.insert_dedup(overflow).is_none());
    let first = DedupKey::new(MessageId::new(0), ep);
    assert!(engine.lookup_dedup(first).is_some());
}

#[test]
fn pending_con_hit_miss_wrong_endpoint() {
    let mut pool = DatagramPool::<2, 16>::new();
    let ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let other = Endpoint::v4([192, 0, 2, 2], 5683);
    let mid = MessageId::new(0x1111);

    let id = pool.acquire().expect("tx");
    pool.set_endpoint(id, ep).expect("endpoint");
    pool.set_pending_mid(id, mid).expect("pending");
    assert_eq!(pool.lookup_pending(mid, ep), Some(id));
    assert_eq!(pool.lookup_pending(MessageId::new(0x2222), ep), None);
    assert_eq!(pool.lookup_pending(mid, other), None);
    assert_eq!(pool.pending_mid(id), Some(mid));

    pool.clear_pending_mid(id).expect("clear");
    assert_eq!(pool.lookup_pending(mid, ep), None);
    assert_eq!(pool.pending_mid(id), None);
}

#[test]
fn pending_con_capacity_is_tx_pool() {
    let mut pool = DatagramPool::<2, 16>::new();
    let ep = Endpoint::v4([198, 51, 100, 1], 5683);
    let a = pool.acquire().expect("a");
    let b = pool.acquire().expect("b");
    assert!(pool.acquire().is_none());
    pool.set_endpoint(a, ep).expect("ep a");
    pool.set_endpoint(b, ep).expect("ep b");
    pool.set_pending_mid(a, MessageId::new(1)).expect("a");
    pool.set_pending_mid(b, MessageId::new(2)).expect("b");
    assert_eq!(pool.lookup_pending(MessageId::new(1), ep), Some(a));
    assert_eq!(
        pool.set_pending_mid(SlotId::from_index(2), MessageId::new(3)),
        Err(SlotError::InvalidSlot)
    );
    pool.release(a).expect("release");
    assert_eq!(pool.pending_mid(a), None);
    let reused = pool.acquire().expect("reuse");
    assert_eq!(reused, a);
    assert_eq!(pool.pending_mid(reused), None);
}

#[test]
fn engine_encode_con_then_empty_ack_clears_pending() {
    let mut engine = build_default();
    let ep = Endpoint::v4([203, 0, 113, 9], 5683);
    let mid = MessageId::new(0x4242);
    let tx = engine.acquire_tx().expect("tx");
    let con = Message::new(Type::Confirmable, Code::GET, mid);
    engine.encode_tx(tx, &con).expect("encode con");
    assert_eq!(engine.record_pending_con(tx, ep, mid), Some(tx));
    assert_eq!(engine.pending_con(tx), Some(PendingCon::new(mid, ep, tx)));
    assert_eq!(engine.lookup_pending_con(mid, ep), Some(tx));

    let rx = engine.acquire_rx().expect("rx");
    let ack = empty_ack(mid);
    let mut buf = [0u8; 8];
    let n = encode(&ack, &mut buf).expect("ack bytes");
    engine.write_rx(rx, &buf[..n], ep).expect("write ack");
    let matched = engine.match_empty_ack_rst_rx(rx).expect("match");
    assert_eq!(matched, Some(tx));
    assert_eq!(engine.pending_con(tx), None);
    assert_eq!(engine.lookup_pending_con(mid, ep), None);
    engine.release_tx(tx).expect("caller releases");
}

#[test]
fn engine_empty_rst_matches_and_wrong_endpoint_misses() {
    let mut engine = build_default();
    let ep = Endpoint::v4([192, 0, 2, 10], 5683);
    let other = Endpoint::v4([192, 0, 2, 11], 5683);
    let mid = MessageId::new(9);
    let tx = engine.acquire_tx().expect("tx");
    engine
        .encode_tx(tx, &Message::new(Type::Confirmable, Code::GET, mid))
        .expect("encode");
    engine.record_pending_con(tx, ep, mid).expect("pending");

    let rst = empty_rst(mid);
    let mut buf = [0u8; 8];
    let n = encode(&rst, &mut buf).expect("rst bytes");
    let parsed = crate::message::decode(&buf[..n]).expect("rst");
    assert!(parsed.is_empty_rst());
    assert_eq!(engine.match_empty_ack_rst(&parsed, other), None);
    assert_eq!(engine.lookup_pending_con(mid, ep), Some(tx));
    assert_eq!(engine.match_empty_ack_rst(&parsed, ep), Some(tx));
    assert_eq!(engine.pending_con(tx), None);
}

#[test]
fn engine_pending_con_distinct_from_dedup() {
    let mut engine = build_default();
    let ep = Endpoint::v4([192, 0, 2, 20], 5683);
    let mid = MessageId::new(5);
    let tx = engine.acquire_tx().expect("tx");
    engine
        .encode_tx(tx, &Message::new(Type::Confirmable, Code::GET, mid))
        .expect("encode");
    engine.record_pending_con(tx, ep, mid).expect("pending");
    let dedup = engine
        .insert_dedup(DedupEntry::new(mid, ep))
        .expect("dedup");
    assert_eq!(engine.lookup_dedup(DedupKey::new(mid, ep)), Some(dedup));

    let parsed = crate::message::decode(&[0x60, 0x00, 0x00, 0x05]).expect("ack");
    assert_eq!(engine.match_empty_ack_rst(&parsed, ep), Some(tx));
    assert_eq!(engine.lookup_dedup(DedupKey::new(mid, ep)), Some(dedup));
    assert_eq!(engine.pending_con(tx), None);
}

#[test]
fn engine_record_pending_con_saturation() {
    let mut engine = build_default();
    let ep = Endpoint::v4([192, 0, 2, 30], 5683);
    let n = profiles::Default::TX_DATAGRAM_SLOTS;
    for i in 0..n {
        let id = engine.acquire_tx().expect("tx");
        let mid = MessageId::new(i as u16);
        engine
            .encode_tx(id, &Message::new(Type::Confirmable, Code::GET, mid))
            .expect("encode");
        assert_eq!(engine.record_pending_con(id, ep, mid), Some(id));
    }
    assert!(engine.acquire_tx().is_none());
    assert!(
        engine
            .record_pending_con(SlotId::from_index(n), ep, MessageId::new(99))
            .is_none()
    );
}

#[test]
fn engine_non_empty_or_wrong_type_does_not_take_pending() {
    let mut engine = build_default();
    let ep = Endpoint::v4([192, 0, 2, 40], 5683);
    let mid = MessageId::new(3);
    let tx = engine.acquire_tx().expect("tx");
    engine
        .encode_tx(tx, &Message::new(Type::Confirmable, Code::GET, mid))
        .expect("encode");
    engine.record_pending_con(tx, ep, mid).expect("pending");

    let get = Message::new(Type::Confirmable, Code::GET, mid);
    let mut buf = [0u8; 16];
    let n = encode(&get, &mut buf).expect("get");
    let parsed = crate::message::decode(&buf[..n]).expect("decode get");
    assert_eq!(engine.match_empty_ack_rst(&parsed, ep), None);
    assert_eq!(engine.lookup_pending_con(mid, ep), Some(tx));
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
    fn alloc_write_rx_associates_endpoint() {
        let mut engine = build_alloc(false);
        let id = engine.acquire_rx().expect("rx");
        let ep = Endpoint::v4([192, 0, 2, 9], 5683);
        let (buf, n) = sample_datagram(0x2222);
        engine.write_rx(id, &buf[..n], ep).expect("write");
        assert_eq!(engine.rx_endpoint(id), Some(ep));
        assert_eq!(
            engine.decode_rx(id).expect("decode").message_id(),
            MessageId::new(0x2222)
        );
    }

    #[test]
    fn alloc_dedup_hit_miss_and_capacity() {
        let mut engine = build_alloc(false);
        let ep = Endpoint::v4([192, 0, 2, 8], 5683);
        let a = DedupEntry::new(MessageId::new(1), ep);
        let id = engine.insert_dedup(a).expect("insert");
        assert_eq!(engine.lookup_dedup(a.key()), Some(id));
        assert_eq!(engine.insert_dedup(a).expect("idempotent"), id);
        for i in 1..profiles::Default::DEDUP_ENTRIES {
            let entry = DedupEntry::new(MessageId::new(i as u16 + 1), ep);
            assert!(engine.insert_dedup(entry).is_some());
        }
        assert!(
            engine
                .insert_dedup(DedupEntry::new(MessageId::new(99), ep))
                .is_none()
        );
    }

    #[test]
    fn alloc_encode_con_then_empty_ack_clears_pending() {
        let mut engine = build_alloc(false);
        let ep = Endpoint::v4([192, 0, 2, 12], 5683);
        let mid = MessageId::new(0x55);
        let tx = engine.acquire_tx().expect("tx");
        engine
            .encode_tx(tx, &Message::new(Type::Confirmable, Code::GET, mid))
            .expect("encode");
        engine.record_pending_con(tx, ep, mid).expect("pending");
        let rx = engine.acquire_rx().expect("rx");
        let ack = empty_ack(mid);
        let mut buf = [0u8; 8];
        let n = encode(&ack, &mut buf).expect("ack");
        engine.write_rx(rx, &buf[..n], ep).expect("write");
        assert_eq!(engine.match_empty_ack_rst_rx(rx).expect("match"), Some(tx));
        assert_eq!(engine.pending_con(tx), None);
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
