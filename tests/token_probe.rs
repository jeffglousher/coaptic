use coaptic::message::{Type, extended};
use coaptic::storage::{DatagramIo, DatagramTransport, Endpoint};
use coaptic::{App, Code, Response};

type LongToken = extended::Token<32>;
coaptic::memory_profile! { Wide {
    token: LongToken,
    rx_datagram: (4, 256), tx_datagram: (4, 256),
    rx_body: (1, 1024), tx_body: (1, 1024), dedup: 4, observe: 2,
} }
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);

#[cfg(feature = "oscore")]
#[test]
fn association_policy_silently_discards_unprotected_oversized_token() {
    let mut server = App::profile::<coaptic::profiles::Default>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .require_associations()
        .bind(Io::default())
        .unwrap();
    server.transport_mut().receive(extended::Message {
        ty: Type::Confirmable,
        code: Code::GET,
        id: coaptic::message::MessageId::new(44),
        token: &[0x44; 32],
        options: &[],
        payload: &[],
    });
    server.poll(0).unwrap();
    assert_eq!(server.transport().len, 0);
    assert_eq!(server.metrics().token_capacity_refused, 0);
}

impl coaptic::storage::AsyncDatagramIo for Io {
    async fn recv(&mut self, output: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        DatagramIo::recv(self, output)?.ok_or("empty")
    }
    async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        DatagramIo::send(self, peer, bytes)
    }
}
struct NeverTimer;
impl coaptic::platform::AsyncTimer for NeverTimer {
    async fn wait_until(&mut self, _: u64) {
        core::future::pending::<()>().await
    }
}
#[test]
fn asynchronous_probe_preserves_canonical_send_and_drop_cleanup() {
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    let mut app = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Io::default())
        .unwrap();
    let mut timer = NeverTimer;
    drop(
        app.get("discard")
            .to(PEER)
            .probe_token_support(9)
            .send_async(0, || Ok::<_, ()>(0), &mut timer),
    );
    let probe = {
        let mut send = pin!(
            app.get("discard")
                .to(PEER)
                .probe_token_support(9)
                .send_until_async(0, || Ok::<_, ()>(0), &mut timer, 100)
        );
        match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Ok(probe)) => probe,
            _ => panic!("ready transport must accept probe"),
        }
    };
    let request = extended::decode(&app.transport().wire[..app.transport().len]).unwrap();
    assert_eq!(request.token().len(), 32);
    assert_eq!(request.ty(), Type::Confirmable);
    assert_eq!(request.code(), Code::GET);
    assert_eq!(request.options().count(), 1);
    assert!(app.cancel(probe.call()));
}

#[cfg(feature = "oscore")]
#[test]
fn protected_policy_silently_discards_unprotected_oversized_token() {
    let context = coaptic::oscore::BoundedSecurityContext::derive(coaptic::oscore::DeriveParams {
        master_secret: &[0x42; 16],
        master_salt: &[],
        sender_id: &[2],
        recipient_id: &[1],
        id_context: &[],
    })
    .unwrap();
    let mut server = App::profile::<coaptic::profiles::Default>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(context)
        .bind(Io::default())
        .unwrap();
    server.transport_mut().receive(extended::Message {
        ty: Type::Confirmable,
        code: Code::GET,
        id: coaptic::message::MessageId::new(44),
        token: &[0x44; 32],
        options: &[],
        payload: &[],
    });
    server.poll(0).unwrap();
    assert_eq!(server.transport().len, 0);
    assert_eq!(server.metrics().token_capacity_refused, 0);
    assert_eq!(server.metrics().tx_ok, 0);
}

use coaptic::app::{CallFailure, TokenSupport, TokenSupportOutcome};

#[test]
fn canonical_probe_classifies_echoed_responses_and_bounds_evidence() {
    assert!(core::mem::size_of::<TokenSupport>() <= 56);
    for (code, expected) in [
        (Code::CONTENT, TokenSupportOutcome::Supported(Code::CONTENT)),
        (Code::BAD_REQUEST, TokenSupportOutcome::LengthRefused),
        (
            Code::SERVICE_UNAVAILABLE,
            TokenSupportOutcome::TemporaryRefusal,
        ),
    ] {
        let mut app = App::profile::<Wide>()
            .block_wise::<false>()
            .token_length(32)
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(Io::default())
            .unwrap();
        let probe = app
            .post("discarded")
            .to(PEER)
            .non()
            .payload(b"discarded")
            .probe_token_support(17)
            .send(0)
            .unwrap();
        let request = extended::decode(&app.transport().wire[..app.transport().len]).unwrap();
        assert_eq!(request.ty(), Type::Confirmable);
        assert_eq!(request.code(), Code::GET);
        assert!(request.payload().is_empty());
        let options: Vec<_> = request.options().collect();
        assert_eq!(options.len(), 1);
        assert_eq!(
            options[0].number(),
            coaptic::message::OptionNumber::IF_NONE_MATCH
        );
        let mid = request.message_id();
        let call = probe.call();
        app.transport_mut().receive(extended::Message {
            ty: Type::Acknowledgement,
            code,
            id: mid,
            token: call.token().as_bytes(),
            options: &[],
            payload: &[],
        });
        app.poll(1).unwrap();
        let result = app.take_response(call).unwrap();
        let evidence = probe.record(&result, 1);
        assert_eq!(evidence.outcome(), expected);
        assert_eq!(evidence.tested_length(), 32);
        assert_eq!(evidence.expires_at(), 1_800_001);
        assert!(!evidence.usable_for(PEER, 18, 32, 2));
        assert!(!evidence.usable_for(PEER, 17, 33, 2));
        assert!(!evidence.usable_for(PEER, 17, 32, 1_800_001));
        assert_eq!(evidence.usable_for(PEER, 17, 32, 2), code == Code::CONTENT);
        assert!(probe.record_for(&result, 1, 1799).is_err());
        assert!(probe.record_for(&result, 1, 86401).is_err());
        assert!(probe.record_for(&result, 1, 86400).is_ok());
    }
}

#[test]
fn probe_reset_timeout_and_cancel_use_ordinary_call_ownership() {
    let mut app = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Io::default())
        .unwrap();
    let probe = app.get("").to(PEER).probe_token_support(0).send(0).unwrap();
    let request = extended::decode(&app.transport().wire[..app.transport().len]).unwrap();
    let mid = request.message_id();
    app.transport_mut().receive(extended::Message {
        ty: Type::Reset,
        code: Code::EMPTY,
        id: mid,
        token: &[],
        options: &[],
        payload: &[],
    });
    app.poll(1).unwrap();
    let result = app.take_response(probe.call()).unwrap();
    assert_eq!(
        probe.record(&result, 1).outcome(),
        TokenSupportOutcome::Unsupported
    );
    let next = app.get("").to(PEER).probe_token_support(1).send(2).unwrap();
    assert!(app.cancel(next.call()));
    assert!(!app.cancel(next.call()));
    assert_eq!(
        next.record(&Err(CallFailure::TimedOut), 3).outcome(),
        TokenSupportOutcome::Unknown
    );
    assert_eq!(
        next.record(&Ok(Response::content(b"unrelated")), 3)
            .outcome(),
        TokenSupportOutcome::Unknown
    );
}

struct Io {
    incoming: Option<([u8; 256], usize)>,
    wire: [u8; 256],
    len: usize,
}
impl Default for Io {
    fn default() -> Self {
        Self {
            incoming: None,
            wire: [0; 256],
            len: 0,
        }
    }
}
impl DatagramTransport for Io {
    type Error = &'static str;
}
impl DatagramIo for Io {
    fn recv(&mut self, output: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((wire, n)) = self.incoming.take() else {
            return Ok(None);
        };
        output[..n].copy_from_slice(&wire[..n]);
        Ok(Some((n, PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.wire[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        Ok(bytes.len())
    }
}
impl Io {
    fn receive(&mut self, message: extended::Message<'_>) {
        let mut wire = [0; 256];
        let n = message.encode(&mut wire).unwrap();
        self.incoming = Some((wire, n));
    }
}
