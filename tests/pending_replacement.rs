//! Atomic pending-CON storage replacement; no App/Observe supersession claim.
use coaptic::Endpoint;
use coaptic::message::{MessageId, decode};
use coaptic::storage::{
    DatagramSlots, Engine, EngineBuilder, Memory, PendingCon, PendingCons, PendingReplaceError,
    Retransmit, SlotError, Storage, profiles,
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
const OWNER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const OTHER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const OLD: &[u8] = b"\x41\x45\x12\x34\x01\xffold complete response";
const NEW: &[u8] = b"\x41\x45\x12\x36\x01\xffnew complete response with more bytes";

fn unchanged<S: Storage + DatagramSlots + PendingCons>(engine: &Engine<S>, pending: PendingCon) {
    assert_eq!(engine.pending_con(pending.tx_slot()), Some(pending));
    assert_eq!(engine.storage().tx_payload(pending.tx_slot()), Some(OLD));
    assert_eq!(engine.storage().tx_endpoint(pending.tx_slot()), Some(OWNER));
}
fn exercise<S: Storage + DatagramSlots + PendingCons>(mut engine: Engine<S>) {
    no_alloc(|| {
        let capacities = engine.capacities();
        let tx = engine.acquire_tx().unwrap();
        engine.write_tx(tx, OLD, OWNER).unwrap();
        engine
            .record_pending_con(tx, OWNER, MessageId::new(0x1234), 100, 123)
            .unwrap();
        let initial = engine.pending_con(tx).unwrap();
        let other = engine.acquire_tx().unwrap();
        engine.write_tx(other, b"unrelated bytes", OTHER).unwrap();
        while engine.acquire_tx().is_some() {}
        assert_eq!(engine.tx_occupied(), capacities.tx_datagram_slots);
        // Staging failure changes neither bytes nor the retained pending owner.
        let mut short = [0u8; 4];
        let message = coaptic::message::Message::new(
            coaptic::message::Type::Confirmable,
            coaptic::Code::CONTENT,
            MessageId::new(0x1236),
        )
        .with_payload(b"complete staging body");
        assert!(coaptic::message::encode(&message, &mut short).is_err());
        unchanged(&engine, initial);
        let due = initial.rto().next_timeout_ms();
        assert!(engine.poll_retransmit(due - 1).is_none());
        let Some(Retransmit::Due(current)) = engine.poll_retransmit(due) else {
            panic!("due retry")
        };
        assert_eq!(current.rto().attempts(), 1);
        let mut larger = EngineBuilder::new()
            .profile::<profiles::Gateway>()
            .block_wise(false)
            .build(Memory::<profiles::Gateway>::new())
            .unwrap();
        let mut outside = larger.acquire_tx().unwrap();
        for _ in 0..capacities.tx_datagram_slots {
            outside = larger.acquire_tx().unwrap();
        }
        assert_eq!(
            engine.replace_pending_con(
                PendingCon::new(current.message_id(), OWNER, outside).with_rto(current.rto()),
                NEW
            ),
            Err(PendingReplaceError::Stale)
        );
        unchanged(&engine, current);
        for stale in [
            initial,
            PendingCon::new(current.message_id(), OTHER, tx).with_rto(current.rto()),
            PendingCon::new(MessageId::new(9), OWNER, tx).with_rto(current.rto()),
            PendingCon::new(current.message_id(), OWNER, other).with_rto(current.rto()),
        ] {
            assert_eq!(
                engine.replace_pending_con(stale, NEW),
                Err(PendingReplaceError::Stale)
            );
            unchanged(&engine, current);
        }
        for invalid in [
            b"".as_slice(),
            b"\x40\x00\x12\x35",
            b"\x61\x45\x12\x35\x01",
            OLD,
            b"\x41\x45\x12\x35\x02",
            b"\x41\x45\x12\x35\x01\xf0",
        ] {
            assert_eq!(
                engine.replace_pending_con(current, invalid),
                Err(PendingReplaceError::InvalidMessage)
            );
            unchanged(&engine, current);
        }
        assert_eq!(
            engine.replace_pending_con(current, &[0; 1473]),
            Err(PendingReplaceError::Slot(SlotError::LengthExceedsSlot))
        );
        unchanged(&engine, current);
        let short_bytes = b"\x41\x45\x12\x35\x01\xffx";
        let shorter = engine.replace_pending_con(current, short_bytes).unwrap();
        assert_eq!(
            engine.storage().tx_payload(tx),
            Some(short_bytes.as_slice())
        );
        assert_eq!(shorter.rto(), current.rto());
        let replacement = engine.replace_pending_con(shorter, NEW).unwrap();
        assert_eq!(replacement.rto(), current.rto());
        assert_eq!(replacement.tx_slot(), tx);
        assert_eq!(replacement.endpoint(), OWNER);
        assert_eq!(engine.capacities(), capacities);
        assert_eq!(engine.tx_occupied(), capacities.tx_datagram_slots);
        assert_eq!(engine.storage().tx_payload(tx), Some(NEW));
        assert_eq!(
            engine.storage().tx_payload(other),
            Some(b"unrelated bytes".as_slice())
        );
        assert_eq!(engine.take_pending_con(current.message_id(), OWNER), None);
        assert_eq!(
            engine.take_pending_con(replacement.message_id(), OTHER),
            None
        );
        assert_eq!(
            engine.lookup_pending_con(replacement.message_id(), OWNER),
            Some(tx)
        );
        // NSTART remains occupied; another slot cannot admit a second owner CON.
        assert_eq!(
            engine.record_pending_con(other, OWNER, MessageId::new(99), due, 0),
            None
        );
        assert_eq!(engine.storage().tx_endpoint(other), Some(OTHER));
        let mut pending = replacement;
        for attempt in 2..=4 {
            let deadline = pending.rto().next_timeout_ms();
            assert!(engine.poll_retransmit(deadline - 1).is_none());
            let Some(Retransmit::Due(next)) = engine.poll_retransmit(deadline) else {
                panic!("retry")
            };
            assert_eq!(next.rto().attempts(), attempt);
            assert_eq!(next.rto().timeout_ms(), pending.rto().timeout_ms() * 2);
            assert_eq!(engine.storage().tx_payload(tx), Some(NEW));
            pending = next;
        }
        assert_eq!(
            engine.poll_retransmit(pending.rto().next_timeout_ms()),
            Some(Retransmit::GiveUp(pending))
        );
        assert_eq!(
            engine.replace_pending_con(pending, NEW),
            Err(PendingReplaceError::Stale)
        );
        engine.release_tx(tx).unwrap();
        assert_eq!(
            engine.replace_pending_con(pending, NEW),
            Err(PendingReplaceError::Stale)
        );
        assert_eq!(engine.acquire_tx(), Some(tx));
        engine.write_tx(tx, OLD, OWNER).unwrap();
        engine
            .record_pending_con(tx, OWNER, MessageId::new(0x1234), 100_000, 0)
            .unwrap();
        let fresh = engine.pending_con(tx).unwrap();
        let replaced = engine.replace_pending_con(fresh, NEW).unwrap();
        let ack = [0x60, 0, 0x12, 0x36];
        assert_eq!(
            engine.match_empty_ack_rst(&decode(&ack).unwrap(), OTHER),
            None
        );
        assert_eq!(
            engine.match_empty_ack_rst(&decode(&ack).unwrap(), OWNER),
            Some(tx)
        );
        assert_eq!(engine.pending_con(tx), None);
        engine.write_tx(tx, OLD, OWNER).unwrap();
        engine
            .record_pending_con(tx, OWNER, MessageId::new(0x1234), 200_000, 0)
            .unwrap();
        let pinned = engine.pending_con(tx).unwrap();
        // A deliberately forgotten safe guard keeps the pin alive for this refusal test.
        core::mem::forget(engine.access_tx(tx).unwrap());
        assert_eq!(
            engine.replace_pending_con(pinned, NEW),
            Err(PendingReplaceError::Slot(SlotError::Pinned))
        );
        unchanged(&engine, pinned);
        assert_ne!(replaced.message_id(), pinned.message_id());
    });
}
#[test]
fn fixed_atomic_pending_replacement() {
    exercise(
        EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build(Memory::<profiles::Default>::new())
            .unwrap(),
    );
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_atomic_pending_replacement() {
    exercise(
        EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(false)
            .build_alloc(coaptic::storage::Capacities::from_profile::<
                profiles::Default,
            >())
            .unwrap(),
    );
}
