use coaptic::message::{Code, Message, MessageId, Token, decode_with_token, extended};
use coaptic::storage::{
    BlockKey, BlockRole, BodyPool, DatagramPool, Endpoint, ExchangeEntry, ExchangeKey,
    ExchangeTable, ObserveInterest, ObserveKey, ObserveTable, SlotPool,
};

fn token(suffix: u8) -> extended::Token<32> {
    let mut bytes = [0x41; 32];
    bytes[31] = suffix;
    extended::Token::new(&bytes).unwrap()
}

#[test]
fn extended_exchange_keys_match_full_token_peer_and_message_id() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let other = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut packets = DatagramPool::<1, 64>::new();
    let packet = packets.acquire().unwrap();
    let mut table = ExchangeTable::<1, extended::Token<32>>::new();
    let entry = ExchangeEntry::new(token(1), peer, MessageId::new(10), packet);
    let slot = table.insert(entry).unwrap();
    assert_eq!(table.lookup(ExchangeKey::new(token(1), peer)), Some(slot));
    assert_eq!(table.lookup(ExchangeKey::new(token(2), peer)), None);
    assert_eq!(table.lookup(ExchangeKey::new(token(1), other)), None);
    let mut wire = [0; 64];
    let response = Message::new(
        coaptic::message::Type::Acknowledgement,
        Code::CONTENT,
        MessageId::new(10),
    )
    .with_token(token(1));
    let n = response.encode(&mut wire).unwrap();
    assert!(entry.matches_response(&decode_with_token::<extended::Token<32>>(&wire[..n]).unwrap()));
    let wrong_mid = Message::new(
        coaptic::message::Type::Acknowledgement,
        Code::CONTENT,
        MessageId::new(11),
    )
    .with_token(token(1));
    let n = wrong_mid.encode(&mut wire).unwrap();
    assert!(
        !entry.matches_response(&decode_with_token::<extended::Token<32>>(&wire[..n]).unwrap())
    );
    assert!(
        table
            .insert(ExchangeEntry::new(
                token(2),
                peer,
                MessageId::new(11),
                packet
            ))
            .is_none()
    );
    assert_eq!(table.take(entry.key()), Some(entry));
    assert!(table.lookup(entry.key()).is_none());
    let replacement = ExchangeEntry::new(token(2), peer, MessageId::new(11), packet);
    table.insert(replacement).unwrap();
    assert!(table.lookup(entry.key()).is_none());
    assert!(table.lookup(replacement.key()).is_some());
}

#[test]
fn extended_observe_rows_preserve_direction_and_release_without_aliasing() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut table = ObserveTable::<2, extended::Token<32>>::new();
    let server = ObserveInterest::new(token(1), peer);
    let client = ObserveInterest::new_client(token(1), peer);
    let a = table.insert(server).unwrap();
    let b = table.insert(client).unwrap();
    assert_ne!(a, b);
    assert_eq!(table.lookup(ObserveKey::new(token(2), peer)), None);
    assert_eq!(table.take(server.key()), Some(server));
    assert_eq!(table.entry(b), Some(client));
    assert!(table.insert(ObserveInterest::new(token(2), peer)).is_some());
    assert!(table.lookup(server.key()).is_none());
}

#[test]
fn extended_body_sidecars_preserve_full_tokens_and_reclaim_on_release() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let first = BlockKey::new(token(1), peer);
    let second = BlockKey::new(token(2), peer);
    let mut bodies = BodyPool::<1, 64, extended::Token<32>>::new();
    let id = bodies
        .start_outgoing(first, BlockRole::OutgoingQBlock2, &[0x33; 32], 0)
        .unwrap();
    assert_eq!(bodies.lookup(first), Some(id));
    assert_eq!(bodies.lookup(second), None);
    assert_eq!(bodies.transfer(id).unwrap().token(), token(1));
    assert!(
        bodies
            .start_outgoing(second, BlockRole::OutgoingBlock2, b"new", 0)
            .is_err()
    );
    assert_eq!(bodies.payload(id), Some(&[0x33; 32][..]));
    bodies.release(id).unwrap();
    assert!(bodies.lookup(first).is_none());
    let reused = bodies
        .start_outgoing(second, BlockRole::OutgoingBlock2, b"new", 0)
        .unwrap();
    assert_eq!(bodies.transfer(reused).unwrap().token(), token(2));
    assert!(bodies.lookup(first).is_none());
}

#[test]
fn default_rows_still_use_compact_tokens_and_explicit_extended_rows_grow_only_when_selected() {
    use core::mem::size_of;
    assert_eq!(size_of::<Token>(), 9);
    assert_eq!(size_of::<ExchangeKey>(), size_of::<ExchangeKey<Token>>());
    assert!(size_of::<ExchangeTable<1, extended::Token<32>>>() > size_of::<ExchangeTable<1>>());
    assert!(size_of::<ObserveTable<1, extended::Token<32>>>() > size_of::<ObserveTable<1>>());
    assert!(size_of::<BodyPool<1, 64, extended::Token<32>>>() > size_of::<BodyPool<1, 64>>());
}
