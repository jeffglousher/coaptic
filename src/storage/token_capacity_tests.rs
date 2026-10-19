//! Selected token capacity changes identity storage, never truncates wire tokens.
use super::*;
use crate::Code;
use crate::error::SlotMessageError;
use crate::message::{Message, MessageId, TokenDecodeError, Type, extended};
type LongToken = extended::Token<32>;
crate::memory_profile! {
    LongProfile {
        token: LongToken,
        rx_datagram: (2, 256), tx_datagram: (2, 256),
        rx_body: (1, 1024), tx_body: (1, 1024), dedup: 2, observe: 2,
    }
}
fn identities<S>(mut engine: Engine<S>)
where
    S: Storage<Token = LongToken> + DatagramSlots + Exchanges + ObserveSlots + BodySlots,
{
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut a = [7; 16];
    a[15] = 1;
    let mut b = a;
    b[15] = 2;
    let first = LongToken::new(&a).unwrap();
    let second = LongToken::new(&b).unwrap();
    for (mid, token) in [(1, first), (2, second)] {
        let slot = engine.acquire_tx().unwrap();
        engine
            .encode_tx(
                slot,
                &Message::new(Type::Confirmable, Code::GET, MessageId::new(mid)).with_token(token),
            )
            .unwrap();
        let recorded = engine.record_request(slot, peer).unwrap().unwrap();
        assert_eq!(recorded.token().as_bytes(), token.as_bytes());
        assert_eq!(engine.decode_tx(slot).unwrap().token(), token);
        engine
            .insert_observe(ObserveInterest::new(token, peer))
            .unwrap();
    }
    let first_key = ExchangeKey::new(first, peer);
    let second_key = ExchangeKey::new(second, peer);
    assert_ne!(
        engine.lookup_exchange(first_key),
        engine.lookup_exchange(second_key)
    );
    engine.take_exchange(first_key).unwrap();
    assert!(engine.lookup_exchange(second_key).is_some());
    engine.take_observe(ObserveKey::new(first, peer)).unwrap();
    assert!(
        engine
            .lookup_observe(ObserveKey::new(second, peer))
            .is_some()
    );
    let body = engine
        .start_block2(BlockKey::new(second, peer), &[4; 32], 0)
        .unwrap();
    assert_eq!(engine.tx_body_transfer(body).unwrap().token(), second);
}
#[test]
fn selected_token_fixed_storage_retains_full_suffix_identity() {
    let engine = EngineBuilder::new()
        .profile::<LongProfile>()
        .block_wise(true)
        .build(Memory::<LongProfile>::with_block_wise())
        .unwrap();
    identities(engine);
}
#[cfg(feature = "alloc")]
#[test]
fn selected_token_allocated_storage_retains_full_suffix_and_accounts_layout() {
    let caps = Capacities::from_profile::<LongProfile>().with_block_wise::<LongProfile>();
    assert!(
        AllocMemory::<LongToken>::required_heap_bytes(caps).unwrap()
            > AllocMemory::<crate::message::Token>::required_heap_bytes(caps).unwrap()
    );
    let engine = EngineBuilder::new()
        .profile::<LongProfile>()
        .block_wise(true)
        .build_alloc_with_token::<LongToken>(caps)
        .unwrap();
    identities(engine);
}
#[test]
fn selected_token_engine_capacity_and_malformed_errors_remain_distinct() {
    let mut engine = EngineBuilder::new()
        .profile::<profiles::Constrained>()
        .block_wise(false)
        .build(Memory::<profiles::Constrained>::new())
        .unwrap();
    let rx = engine.acquire_rx().unwrap();
    let token = LongToken::new(&[9; 16]).unwrap();
    engine
        .encode_rx(
            rx,
            &Message::new(Type::Confirmable, Code::GET, MessageId::new(3)).with_token(token),
        )
        .unwrap();
    assert!(
        matches!(engine.decode_rx(rx), Err(SlotMessageError::TokenCapacity(error))
        if error.required == 16 && error.available == 8)
    );
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    engine.write_rx(rx, &[0x4d, 1, 0], peer).unwrap();
    assert!(matches!(
        engine.decode_rx(rx),
        Err(SlotMessageError::Parse(_))
    ));
    assert!(matches!(
        crate::message::decode_with_token::<LongToken>(&[0x4d, 1, 0]),
        Err(TokenDecodeError::Malformed(_))
    ));
}
