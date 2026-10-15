use super::*;
use crate::oscore::{DeriveParams, SecurityContext};

struct NoIo;
impl DatagramIo for NoIo {
    type Error = core::convert::Infallible;
    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        Ok(None)
    }
    fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> {
        panic!("setter/idle poll must not send")
    }
}
fn context() -> SecurityContext {
    SecurityContext::derive(DeriveParams {
        master_secret: &[24; 16],
        master_salt: &[],
        sender_id: &[1],
        recipient_id: &[2],
        id_context: &[],
    })
    .unwrap()
}

#[test]
fn route_removal_security_latch_refusal_preserves_keys_and_clean_checkpoint() {
    let mut app = App::profile::<crate::profiles::Constrained>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(context())
        .route("target", get(|_| Response::content(b"old")))
        .bind(NoIo)
        .unwrap();
    app.oscore_checkpoint = oscore::CheckpointState::Clean;
    let key = DedupKey::new(MessageId::new(7), Endpoint::v4([192, 0, 2, 2], 5683));
    app.dedup_closed = Some(DedupClosed { key, due_ms: 10 });
    assert_eq!(app.remove_route("target"), Err(RouteRemovalError::Busy));
    assert_eq!(app.oscore().unwrap().sender_seq(), 0);
    assert_eq!(app.dedup_closed.as_ref().unwrap().key, key);
    assert!(matches!(
        app.oscore_checkpoint,
        oscore::CheckpointState::Clean
    ));
    app.poll_with_oscore_checkpoint(10, |_| panic!("removal must not dirty checkpoint"))
        .unwrap();
    assert_eq!(app.remove_route("target"), Ok(true));
    assert_eq!(app.oscore().unwrap().sender_seq(), 0);
    assert!(matches!(
        app.oscore_checkpoint,
        oscore::CheckpointState::Clean
    ));
}

#[test]
fn route_removal_waits_for_independent_security_request_binding() {
    use crate::message::{Message, Token, Type};
    let token = Token::new(&[9]).unwrap();
    let mut ctx = context();
    let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(1)).with_token(token);
    let mut wire = [0; 256];
    ctx.protect_request(&request, &mut wire).unwrap();
    let reference = ctx.lookup(token).unwrap();
    let mut app = App::profile::<crate::profiles::Constrained>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(ctx)
        .route("target", get(|_| Response::content(b"old")))
        .bind(NoIo)
        .unwrap();
    app.oscore_checkpoint = oscore::CheckpointState::Clean;
    assert_eq!(app.remove_route("target"), Err(RouteRemovalError::Busy));
    assert_eq!(app.oscore().unwrap().lookup(token), Some(reference));
    assert_eq!(app.oscore().unwrap().sender_seq(), 1);
    assert!(matches!(
        app.oscore_checkpoint,
        oscore::CheckpointState::Clean
    ));
    // This fixture directly owns an advanced request binding, independently of Calls.
    assert_eq!(app.oscore.as_mut().unwrap().take(token), Some(reference));
    assert_eq!(app.remove_route("target"), Ok(true));
    assert_eq!(app.oscore().unwrap().sender_seq(), 1);
    assert!(matches!(
        app.oscore_checkpoint,
        oscore::CheckpointState::Clean
    ));
    app.oscore
        .as_mut()
        .unwrap()
        .protect_request(&request, &mut wire)
        .unwrap();
    assert_eq!(app.oscore().unwrap().sender_seq(), 2);
}
#[test]
fn oscore_install_fail_closed_dedup_latch_refuses_until_old_epoch_expiry() {
    let mut app = App::profile::<crate::profiles::Constrained>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(context())
        .bind(NoIo)
        .unwrap();
    app.oscore_checkpoint = oscore::CheckpointState::Clean;
    let key = DedupKey::new(MessageId::new(7), Endpoint::v4([192, 0, 2, 2], 5683));
    app.dedup_closed = Some(DedupClosed { key, due_ms: 10 });
    let mut candidate = Some(context());
    candidate.as_mut().unwrap().set_sender_seq(100).unwrap();
    assert!(matches!(
        app.set_oscore(&mut candidate),
        Err(OscoreInstallError::Busy)
    ));
    assert!(matches!(
        app.oscore_checkpoint,
        oscore::CheckpointState::Clean
    ));
    assert_eq!(app.dedup_closed.as_ref().unwrap().key, key);
    assert_eq!(app.oscore().unwrap().sender_seq(), 0);
    assert_eq!(candidate.as_ref().unwrap().sender_seq(), 100);
    app.poll_with_oscore_checkpoint(10, |_| panic!("busy replacement did not dirty checkpoint"))
        .unwrap();
    assert!(app.dedup_closed.is_none());
    app.set_oscore(&mut candidate).unwrap();
    assert!(candidate.is_none());
    assert_eq!(app.oscore().unwrap().sender_seq(), 100);
    assert!(matches!(
        app.oscore_checkpoint,
        oscore::CheckpointState::Dirty
    ));
}
