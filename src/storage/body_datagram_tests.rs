//! Retained bodies and outgoing packets are independent storage areas.

use super::*;
use crate::message::{MessageId, Token};

crate::memory_profile! {
    BorrowProfile {
        rx_datagram: (2, 1152), tx_datagram: (2, 1152),
        rx_body: (2, 2048), tx_body: (2, 2048), dedup: 2, observe: 2,
    }
}

fn peer() -> Endpoint {
    Endpoint::v4([192, 0, 2, 9], 5683)
}

fn prepare_tx<S: BodyDatagramSlots<Token = Token>>(store: &mut S) -> SlotId {
    let tx = store.tx_datagram().acquire().unwrap();
    store.tx_payload_mut(tx).unwrap().fill(0xa5);
    store.set_tx_len(tx, 7).unwrap();
    store.set_tx_endpoint(tx, peer()).unwrap();
    store.set_tx_pending(tx, MessageId::new(123)).unwrap();
    tx
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    packet: [u8; 1152],
    packet_len: usize,
    endpoint: Option<Endpoint>,
    pending: Option<MessageId>,
    transfer: Option<BlockTransfer>,
    body_state: (usize, usize, bool),
    tx_state: (usize, usize, bool),
}

fn snapshot<S: BodyDatagramSlots<Token = Token>>(
    store: &mut S,
    body: SlotId,
    tx: SlotId,
) -> Snapshot {
    let packet = <[u8; 1152]>::try_from(&*store.tx_payload_mut(tx).unwrap()).unwrap();
    let body_pool = store.tx_body().unwrap();
    let body_state = (
        body_pool.occupied_count(),
        body_pool.cursor(),
        body_pool.is_pinned(body),
    );
    let tx_pool = store.tx_datagram();
    let tx_state = (
        tx_pool.occupied_count(),
        tx_pool.cursor(),
        tx_pool.is_pinned(tx),
    );
    Snapshot {
        packet,
        packet_len: store.tx_payload(tx).unwrap().len(),
        endpoint: store.tx_endpoint(tx),
        pending: store.tx_pending_mid(tx),
        transfer: store.tx_body_transfer(body),
        body_state,
        tx_state,
    }
}

fn borrowed_ranges<S: BodyDatagramSlots<Token = Token>>(mut store: S) {
    let mut payload = [0; 1061];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    for len in [37, 1024, 1061] {
        let body = store
            .start_block2(BlockKey::new(Token::EMPTY, peer()), &payload[..len], 6)
            .unwrap();
        let tx = prepare_tx(&mut store);
        // Equal numeric IDs address separate pools, not overlapping bytes.
        assert_eq!(body, tx);
        let body_ptr = store.tx_body_payload(body).unwrap().as_ptr();
        let packet_ptr = store.tx_payload_mut(tx).unwrap().as_ptr();
        let mut expected = snapshot(&mut store, body, tx);
        {
            let (retained, mut packet) = store.tx_body_and_tx_mut(body, tx).unwrap();
            assert_eq!(retained.as_ptr(), body_ptr);
            assert_eq!(packet.bytes_mut().as_ptr(), packet_ptr);
            assert_eq!(retained, &payload[..len]);
            assert_eq!(packet.id(), tx);
            assert!(packet.is_pinned());
            assert_eq!(packet.capacity(), 1152);
            assert_eq!(packet.payload(), &[0xa5; 7]);
            let full = len.min(1024);
            packet.bytes_mut()[8..8 + full].copy_from_slice(&retained[..full]);
            packet.bytes_mut()[8 + full..8 + len].copy_from_slice(&retained[full..]);
            assert_eq!(packet.payload(), &[0xa5; 7]);
            assert_eq!(packet.set_len(1153), Err(SlotError::LengthExceedsSlot));
            assert_eq!(packet.payload().len(), 7);
            packet.set_len(8 + len).unwrap();
            assert_eq!(&packet.payload()[8..], retained);
            assert!(packet.is_pinned());
        }
        expected.packet[8..8 + len].copy_from_slice(&payload[..len]);
        expected.packet_len = 8 + len;
        assert!(!store.tx_datagram().is_pinned(tx));
        assert_eq!(snapshot(&mut store, body, tx), expected);
        assert_eq!(store.tx_body_payload(body), Some(&payload[..len]));
        store.tx_body().unwrap().release(body).unwrap();
        store.tx_datagram().release(tx).unwrap();
    }
}

fn refused_access<S: BodyDatagramSlots<Token = Token>>(mut store: S) {
    let payload = [0x39; 37];
    let body = store
        .start_block2(BlockKey::new(Token::EMPTY, peer()), &payload, 6)
        .unwrap();
    let tx = prepare_tx(&mut store);
    let free_body = store.tx_body().unwrap().acquire().unwrap();
    store.tx_body().unwrap().release(free_body).unwrap();
    let free_tx = store.tx_datagram().acquire().unwrap();
    store.tx_datagram().release(free_tx).unwrap();
    let invalid_body = SlotId::from_index(store.tx_body().unwrap().slot_count());
    let invalid_tx = SlotId::from_index(store.tx_datagram().slot_count());
    let before = snapshot(&mut store, body, tx);
    for (body_arg, tx_arg, error) in [
        (invalid_body, tx, SlotError::InvalidSlot),
        (body, invalid_tx, SlotError::InvalidSlot),
        (free_body, tx, SlotError::NotOccupied),
        (body, free_tx, SlotError::NotOccupied),
    ] {
        assert_eq!(
            store.tx_body_and_tx_mut(body_arg, tx_arg).unwrap_err(),
            error
        );
        assert_eq!(snapshot(&mut store, body, tx), before);
        assert_eq!(store.tx_body_payload(body), Some(payload.as_slice()));
        assert!(!store.tx_body().unwrap().is_occupied(free_body));
        assert!(!store.tx_datagram().is_occupied(free_tx));
    }

    // Forgetting guards safely retains their pin bits after the Rust borrow ends.
    // This exercises refusal even when no live guard prevents the call statically.
    core::mem::forget(store.access_tx_body_mut(body).unwrap());
    let pinned_body = snapshot(&mut store, body, tx);
    assert!(pinned_body.body_state.2);
    assert_eq!(store.tx_body_and_tx_mut(body, tx).unwrap().0, &payload);
    assert_eq!(snapshot(&mut store, body, tx), pinned_body);
    core::mem::forget(store.access_tx(tx).unwrap());
    let pinned_tx = snapshot(&mut store, body, tx);
    assert!(pinned_tx.tx_state.2);
    assert_eq!(
        store.tx_body_and_tx_mut(body, tx).unwrap_err(),
        SlotError::Pinned
    );
    assert_eq!(snapshot(&mut store, body, tx), pinned_tx);
    assert_eq!(store.tx_body_payload(body), Some(payload.as_slice()));
    assert_eq!(store.tx_datagram().release(tx), Err(SlotError::Pinned));
}

fn absent_body<S: BodyDatagramSlots<Token = Token>>(mut store: S) {
    let tx = prepare_tx(&mut store);
    let cursor = store.tx_datagram().cursor();
    assert_eq!(
        store
            .tx_body_and_tx_mut(SlotId::from_index(0), tx)
            .unwrap_err(),
        SlotError::InvalidSlot
    );
    assert_eq!(store.tx_payload(tx), Some([0xa5; 7].as_slice()));
    assert_eq!(store.tx_payload_mut(tx).unwrap(), &[0xa5; 1152]);
    assert_eq!(store.tx_endpoint(tx), Some(peer()));
    assert_eq!(store.tx_pending_mid(tx), Some(MessageId::new(123)));
    assert_eq!(store.tx_datagram().cursor(), cursor);
    assert_eq!(store.tx_datagram().occupied_count(), 1);
    assert!(!store.tx_datagram().is_pinned(tx));
}

#[test]
fn fixed_body_datagram_borrows_full_and_partial_ranges() {
    borrowed_ranges(Memory::<BorrowProfile>::with_block_wise());
}

#[test]
fn fixed_body_datagram_refusals_preserve_state() {
    refused_access(Memory::<BorrowProfile>::with_block_wise());
    absent_body(Memory::<BorrowProfile>::new());
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_body_datagram_borrows_full_and_partial_ranges() {
    let caps = Capacities::from_profile::<BorrowProfile>().with_block_wise::<BorrowProfile>();
    borrowed_ranges(AllocMemory::<Token>::try_new(caps, usize::MAX).unwrap());
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_body_datagram_refusals_preserve_state() {
    let caps = Capacities::from_profile::<BorrowProfile>();
    refused_access(
        AllocMemory::<Token>::try_new(caps.with_block_wise::<BorrowProfile>(), usize::MAX).unwrap(),
    );
    absent_body(AllocMemory::<Token>::try_new(caps, usize::MAX).unwrap());
}
