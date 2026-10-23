use super::*;
use crate::storage::{EngineBuilder, Memory, PendingCon, profiles};

fn notification() -> (
    Engine<Memory<profiles::Constrained>>,
    PendingCon,
    ObserveInterest,
) {
    let mut engine = EngineBuilder::new()
        .profile::<profiles::Constrained>()
        .block_wise(false)
        .build(Memory::<profiles::Constrained>::new())
        .unwrap();
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mid = MessageId::new(99);
    let token = crate::message::Token::EMPTY;
    let tx = engine.acquire_tx().unwrap();
    engine
        .encode_tx(tx, &Message::new(Type::Confirmable, Code::CONTENT, mid))
        .unwrap();
    engine.set_tx_endpoint(tx, peer).unwrap();
    let reserved = engine.reserve_pending_con(tx, peer, mid).unwrap();
    let key = ObserveKey::new(token, peer);
    let id = engine
        .insert_observe(ObserveInterest::new(token, peer))
        .unwrap();
    engine.record_observe_notify(key, 0, mid, true).unwrap();
    let interest = engine.observe_interest(id).unwrap();
    (engine, reserved, interest)
}

#[test]
fn unsent_observe_hold_does_not_authorize_control() {
    let (mut engine, reserved, _) = notification();
    let deferred = DeferredTable::<0>::new();
    assert_eq!(
        engine.lookup_pending_con(reserved.message_id(), reserved.endpoint()),
        None
    );
    assert_eq!(
        engine.lookup_owned_con(reserved.message_id(), reserved.endpoint()),
        Some(reserved.tx_slot())
    );
    assert_eq!(
        control_route(
            &engine,
            &deferred,
            reserved.endpoint(),
            reserved.message_id(),
            0
        ),
        None
    );
    engine.commit_pending_con(reserved, 40, 0).unwrap();
    assert_eq!(
        control_route(
            &engine,
            &deferred,
            reserved.endpoint(),
            reserved.message_id(),
            40
        ),
        Some(reserved.endpoint())
    );
}

#[test]
fn suppressed_observe_releases_unsent_notification() {
    let (mut engine, reserved, interest) = notification();
    retire_suppressed_observe(&mut engine, interest).unwrap();
    assert!(engine.pending_con(reserved.tx_slot()).is_none());
    assert_eq!(engine.tx_occupied(), 0);
    assert!(engine.lookup_observe(interest.key()).is_none());
}

#[test]
fn reregistration_releases_unsent_notification() {
    let (mut engine, reserved, interest) = notification();
    let mut plan = ObservePlan::idle();
    plan.deregister = true;
    plan.token = interest.token();
    let mut response = Response::content_copy(b"done").observe(17);
    apply_observe(
        &mut engine,
        0,
        interest.endpoint(),
        &mut response,
        plan,
        None,
        oscore::no_request(),
    )
    .unwrap();
    assert!(engine.pending_con(reserved.tx_slot()).is_none());
    assert_eq!(engine.tx_occupied(), 0);
    assert!(engine.lookup_observe(interest.key()).is_none());
    assert_eq!(response.observe_seq(), None);
    assert_eq!(response.payload(), b"done");
    assert_eq!(response.validate(), Ok(()));
}
