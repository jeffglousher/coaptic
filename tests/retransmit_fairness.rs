//! Finite due-CON selection fairness; no successful-send guarantee.
use coaptic::Endpoint;
use coaptic::message::MessageId;
use coaptic::storage::{
    DatagramSlots, Engine, EngineBuilder, Memory, PendingCons, Retransmit, Storage, profiles,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static ALLOCS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Tracking;
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1))
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1))
            }
        });
        unsafe { System.realloc(ptr, layout, n) }
    }
}
#[global_allocator]
static ALLOCATOR: Tracking = Tracking;
fn no_alloc(f: impl FnOnce()) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCS.with(|c| c.set(None));
        }
    }
    ALLOCS.with(|c| c.set(Some(0)));
    let reset = Reset;
    f();
    let n = ALLOCS.with(|c| c.get().unwrap());
    drop(reset);
    assert_eq!(n, 0);
}
fn late_poll<S: Storage + DatagramSlots + PendingCons>(mut engine: Engine<S>) {
    no_alloc(|| {
        let a = engine.acquire_tx().unwrap();
        let b = engine.acquire_tx().unwrap();
        let owner_a = Endpoint::v4([192, 0, 2, 1], 5683);
        let owner_b = Endpoint::v4([192, 0, 2, 2], 5683);
        engine.write_tx(a, b"complete A", owner_a).unwrap();
        engine.write_tx(b, b"complete B", owner_b).unwrap();
        engine
            .record_pending_con(a, owner_a, MessageId::new(1), 0, 0)
            .unwrap();
        engine
            .record_pending_con(b, owner_b, MessageId::new(2), 0, 0)
            .unwrap();
        let Some(Retransmit::Due(first)) = engine.poll_retransmit(2_000) else {
            panic!("first due")
        };
        assert_eq!(first.tx_slot(), a);
        let Some(Retransmit::Due(second)) = engine.poll_retransmit(6_000) else {
            panic!("second due")
        };
        assert_eq!(
            second.tx_slot(),
            b,
            "late poll must not select slot zero repeatedly"
        );
        assert_eq!(second.rto().attempts(), 1);
        assert_eq!(
            engine.storage().tx_payload(b),
            Some(b"complete B".as_slice())
        );
    });
}
#[test]
fn fixed_late_poll() {
    late_poll(
        EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .unwrap(),
    );
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_late_poll() {
    late_poll(
        EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build_alloc(coaptic::storage::Capacities::from_profile::<
                profiles::Default,
            >())
            .unwrap(),
    );
}
fn cycle<S: Storage + DatagramSlots + PendingCons>(mut engine: Engine<S>, pin_first: bool) {
    no_alloc(|| {
        let count = engine.capacities().tx_datagram_slots;
        assert!(count <= 4);
        let mut slots = [None; 4];
        for (i, slot) in slots.iter_mut().enumerate().take(count) {
            let id = engine.acquire_tx().unwrap();
            assert_eq!(id.index(), i);
            *slot = Some(id);
            let owner = Endpoint::v4([192, 0, 2, i as u8 + 1], 5683);
            engine
                .write_tx(id, b"complete retained response", owner)
                .unwrap();
            engine
                .record_pending_con(id, owner, MessageId::new(i as u16), 0, 0)
                .unwrap();
        }
        assert!(engine.acquire_tx().is_none());
        let acquisition_cursor = engine.tx_cursor();
        if pin_first {
            core::mem::forget(engine.access_tx(slots[0].unwrap()).unwrap());
        }
        // NSTART refusal retains both exact pending identities and timers.
        if count > 1 {
            let second = slots[1].unwrap();
            let before = engine.pending_con(second).unwrap();
            assert!(
                engine
                    .record_pending_con(
                        second,
                        Endpoint::v4([192, 0, 2, 1], 5683),
                        MessageId::new(77),
                        0,
                        0
                    )
                    .is_none()
            );
            assert_eq!(engine.pending_con(second), Some(before));
        }
        assert!(engine.poll_retransmit(1_999).is_none());
        let mut now = 2_000;
        // Every owner is due at every call, including the previously serviced
        // one: advancing by a minute exposes starvation from index-zero scans.
        for attempt in 1..=4 {
            for slot in slots.iter().take(count) {
                let id = slot.unwrap();
                let before = engine.pending_con(id).unwrap();
                let Some(Retransmit::Due(due)) = engine.poll_retransmit(now) else {
                    panic!("due event")
                };
                assert_eq!(due.tx_slot(), id);
                assert_eq!(due.endpoint(), before.endpoint());
                assert_eq!(due.message_id(), before.message_id());
                assert_eq!(due.rto().attempts(), attempt);
                assert_eq!(due.rto().timeout_ms(), before.rto().timeout_ms() * 2);
                assert_eq!(
                    due.rto().next_timeout_ms(),
                    now + u64::from(due.rto().timeout_ms())
                );
                assert_eq!(engine.pending_con(id), Some(due));
                assert_eq!(
                    engine.storage().tx_payload(id),
                    Some(b"complete retained response".as_slice())
                );
                assert_eq!(engine.tx_cursor(), acquisition_cursor);
                if pin_first && id == slots[0].unwrap() {
                    assert_eq!(
                        engine.access_tx(id).err(),
                        Some(coaptic::storage::SlotError::Pinned)
                    );
                }
                now += 60_000;
            }
        }
        for slot in slots.iter().take(count) {
            let id = slot.unwrap();
            let before = engine.pending_con(id).unwrap();
            assert_eq!(
                engine.poll_retransmit(now),
                Some(Retransmit::GiveUp(before))
            );
            assert!(engine.pending_con(id).is_none());
            now += 60_000;
        }
        assert!(engine.poll_retransmit(now).is_none());
        assert_eq!(engine.tx_occupied(), count);
    });
}
fn churn<S: Storage + DatagramSlots + PendingCons>(mut engine: Engine<S>) {
    no_alloc(|| {
        let a = engine.acquire_tx().unwrap();
        let b = engine.acquire_tx().unwrap();
        // Keep every other slot occupied so release/reacquire exercises the
        // actual slot-zero refill without changing allocation policy.
        while engine.acquire_tx().is_some() {}
        let owner_a = Endpoint::v4([192, 0, 2, 1], 5683);
        let owner_b = Endpoint::v4([192, 0, 2, 2], 5683);
        engine.write_tx(a, b"A", owner_a).unwrap();
        engine.write_tx(b, b"B", owner_b).unwrap();
        engine
            .record_pending_con(a, owner_a, MessageId::new(1), 0, 0)
            .unwrap();
        engine
            .record_pending_con(b, owner_b, MessageId::new(2), 0, 0)
            .unwrap();
        let Some(Retransmit::Due(first)) = engine.poll_retransmit(2_000) else {
            panic!("A")
        };
        assert_eq!(first.tx_slot(), a);
        assert_eq!(engine.take_pending_con(MessageId::new(1), owner_b), None);
        assert_eq!(engine.take_pending_con(MessageId::new(1), owner_a), Some(a));
        engine.release_tx(a).unwrap();
        assert_eq!(engine.acquire_tx(), Some(a));
        engine.write_tx(a, b"fresh A", owner_a).unwrap();
        engine
            .record_pending_con(a, owner_a, MessageId::new(3), 2_000, 0)
            .unwrap();
        let Some(Retransmit::Due(next)) = engine.poll_retransmit(60_000) else {
            panic!("B")
        };
        assert_eq!(
            next.tx_slot(),
            b,
            "slot zero refill cannot overtake waiting owner"
        );
        assert_eq!(next.rto().attempts(), 1);
        let Some(Retransmit::Due(next)) = engine.poll_retransmit(60_000) else {
            panic!("fresh A")
        };
        assert_eq!(next.tx_slot(), a);
        assert_eq!(next.message_id(), MessageId::new(3));
        assert_eq!(engine.storage().tx_payload(a), Some(b"fresh A".as_slice()));
    });
}
coaptic::memory_profile! {
    One {
        rx_datagram: (1,512), tx_datagram: (1,512),
        rx_body: (1,1024), tx_body: (1,1024), dedup: 1, observe: 1,
    }
}
fn fixed() -> Engine<Memory<profiles::Default>> {
    EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::new())
        .unwrap()
}
#[test]
fn fixed_full_cycle() {
    cycle(fixed(), false);
}
#[test]
fn fixed_pinned_owner_does_not_block_other_events() {
    cycle(fixed(), true);
}
#[test]
fn fixed_refill() {
    churn(fixed());
}
#[test]
fn fixed_single_owner() {
    cycle(
        EngineBuilder::new()
            .profile::<One>()
            .block_wise(false)
            .build(Memory::<One>::new())
            .unwrap(),
        false,
    );
}
#[cfg(feature = "alloc")]
fn allocated(count: usize) -> Engine<coaptic::storage::AllocMemory> {
    if count == 1 {
        return EngineBuilder::new()
            .profile::<One>()
            .block_wise(false)
            .build_alloc(coaptic::storage::Capacities::from_profile::<One>())
            .unwrap();
    }
    let c = coaptic::storage::Capacities::from_profile::<profiles::Default>();
    EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build_alloc(c)
        .unwrap()
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_full_cycle() {
    cycle(allocated(4), false);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_pinned_owner_does_not_block_other_events() {
    cycle(allocated(4), true);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_refill() {
    churn(allocated(4));
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_single_owner() {
    cycle(allocated(1), false);
}

#[test]
fn layout_evidence() {
    println!(
        "fixed_default={} fixed_one={} usize={}",
        core::mem::size_of::<Memory<profiles::Default>>(),
        core::mem::size_of::<Memory<One>>(),
        core::mem::size_of::<usize>()
    );
    #[cfg(feature = "alloc")]
    {
        use coaptic::storage::{AllocMemory, Capacities};
        let mut c = Capacities::from_profile::<profiles::Default>();
        c.rx_body_slots = None;
        c.rx_body_bytes = None;
        c.tx_body_slots = None;
        c.tx_body_bytes = None;
        let required = AllocMemory::required_heap_bytes(c).unwrap();
        println!(
            "allocated_inline={} requested_heap={required}",
            core::mem::size_of::<AllocMemory>()
        );
        assert!(AllocMemory::try_new(c, required).is_ok());
        assert!(matches!(
            AllocMemory::try_new(c, required - 1),
            Err(coaptic::BuildError::HeapBudgetExceeded { .. })
        ));
    }
}

coaptic::memory_profile! {
    Zero {
        rx_datagram: (0,512), tx_datagram: (0,512),
        rx_body: (1,1024), tx_body: (1,1024), dedup: 0, observe: 0,
    }
}
#[test]
fn fixed_empty_storage() {
    no_alloc(|| {
        let mut storage = Memory::<Zero>::new();
        assert!(storage.poll_retransmit(0).is_none());
        assert!(storage.poll_retransmit(u64::MAX).is_none());
    });
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_empty_storage() {
    let mut c = coaptic::storage::Capacities::from_profile::<Zero>();
    c.rx_body_slots = None;
    c.rx_body_bytes = None;
    c.tx_body_slots = None;
    c.tx_body_bytes = None;
    let mut storage = coaptic::storage::AllocMemory::try_new(c, usize::MAX).unwrap();
    no_alloc(|| {
        assert!(storage.poll_retransmit(0).is_none());
        assert!(storage.poll_retransmit(u64::MAX).is_none());
    });
}

fn quiet_scan<S: Storage + DatagramSlots + PendingCons>(mut engine: Engine<S>) {
    no_alloc(|| {
        let a = engine.acquire_tx().unwrap();
        let b = engine.acquire_tx().unwrap();
        let owner_a = Endpoint::v4([192, 0, 2, 1], 5683);
        let owner_b = Endpoint::v4([192, 0, 2, 2], 5683);
        engine.write_tx(a, b"A", owner_a).unwrap();
        engine.write_tx(b, b"B", owner_b).unwrap();
        engine
            .record_pending_con(a, owner_a, MessageId::new(1), 0, 0)
            .unwrap();
        engine
            .record_pending_con(b, owner_b, MessageId::new(2), 10_000, 0)
            .unwrap();
        let Some(Retransmit::Due(first)) = engine.poll_retransmit(2_000) else {
            panic!("first")
        };
        let waiting = engine.pending_con(b).unwrap();
        assert!(engine.poll_retransmit(3_000).is_none());
        assert_eq!(engine.pending_con(a), Some(first));
        assert_eq!(engine.pending_con(b), Some(waiting));
        // Refused malformed replacement must not reset the selection cursor
        // or the already-advanced exact retry snapshot.
        assert!(engine.replace_pending_con(first, b"bad").is_err());
        assert_eq!(engine.pending_con(a), Some(first));
        let Some(Retransmit::Due(next)) = engine.poll_retransmit(60_000) else {
            panic!("waiting")
        };
        assert_eq!(next.tx_slot(), b);
    });
}
#[test]
fn fixed_quiet_scan_and_refusal_preserve_cursor() {
    quiet_scan(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_quiet_scan_and_refusal_preserve_cursor() {
    quiet_scan(allocated(4));
}
