//! Finite public-App checks for borrowed custom option retention.
extern crate std;
use super::*;
use crate::message::{Opt, OptionNumber, Token, decode};
use crate::storage::{AsyncDatagramIo, BodyTag, DatagramIo, DatagramTransport};
use core::{
    future::Future,
    task::{Context, Poll, Waker},
};
use std::{collections::VecDeque, vec::Vec};

const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const BODY: [u8; 2000] = [0x5a; 2000];
const A: OptionNumber = OptionNumber::new(65000);
const B: OptionNumber = OptionNumber::new(65002);
const CUSTOM: [Opt<'static>; 3] = [
    Opt::new(B, b"last"),
    Opt::new(A, b"first"),
    Opt::new(A, b"second"),
];
crate::memory_profile! {
    Buffers {
        rx_datagram: (8, 1280), tx_datagram: (8, 1280),
        rx_body: (2, 2048), tx_body: (2, 2048),
        dedup: 16, observe: 2,
    }
}

#[derive(Default)]
struct Io {
    input: VecDeque<Vec<u8>>,
    output: VecDeque<Vec<u8>>,
    sent: usize,
}
impl DatagramTransport for Io {
    type Error = ();
}
impl DatagramIo for Io {
    fn recv(&mut self, bytes: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        let Some(wire) = self.input.pop_front() else {
            return Ok(None);
        };
        assert!(wire.len() <= bytes.len());
        bytes[..wire.len()].copy_from_slice(&wire);
        Ok(Some((wire.len(), PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        self.sent += 1;
        self.output.push_back(bytes.to_vec());
        Ok(bytes.len())
    }
}
impl AsyncDatagramIo for Io {
    async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), ()> {
        match DatagramIo::recv(self, bytes)? {
            Some(frame) => Ok(frame),
            None => core::future::pending().await,
        }
    }
    async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        DatagramIo::send(self, peer, bytes)
    }
}
struct Timer;
impl crate::platform::AsyncTimer for Timer {
    async fn wait_until(&mut self, _: u64) {
        core::future::pending().await
    }
}
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("finite memory transport blocked"),
    }
}
macro_rules! start {
    ($asynchronous:expr, $request:expr, $now:expr) => {{
        if $asynchronous {
            ready($request.send_async($now, || Ok::<_, ()>($now), &mut Timer)).map_err(|error| {
                match error {
                    AsyncCallError::Request(error) => error,
                    other => panic!("unexpected async send failure: {other:?}"),
                }
            })
        } else {
            $request.send($now)
        }
    }};
}
type TestApp = App<Buffers, Io, 8, true>;
fn progress(app: &mut TestApp, asynchronous: bool, now: u64) {
    if asynchronous {
        let mut timer = Timer;
        {
            let mut wait =
                core::pin::pin!(app.wait_async(now, &mut timer, core::future::pending()));
            if let Poll::Ready(Err(error)) =
                wait.as_mut().poll(&mut Context::from_waker(Waker::noop()))
            {
                panic!("receive error: {error:?}");
            }
        }
        ready(app.progress_async(now, &mut || Ok::<_, ()>(now), &mut timer, now + 100))
            .unwrap_or_else(|error| panic!("async progress at {now}: {error:?}"));
    } else {
        app.poll(now)
            .unwrap_or_else(|error| panic!("sync progress at {now}: {error:?}"));
    }
}
fn assert_custom(message: &ParsedMessage<'_>) {
    if message.code() == Code::FETCH {
        assert_eq!(
            message.payload(),
            b"selection",
            "FETCH follow-ups retain the selection body"
        );
    }
    let values: Vec<_> = message
        .options()
        .filter(|option| !option.number().is_known())
        .map(|option| (option.number(), option.value()))
        .collect();
    assert_eq!(
        values,
        [
            (A, b"first".as_slice()),
            (A, b"second".as_slice()),
            (B, b"last".as_slice())
        ]
    );
}
#[cfg(feature = "oscore")]
fn context(sender: &[u8], recipient: &[u8]) -> crate::oscore::SecurityContext {
    crate::oscore::SecurityContext::derive(crate::oscore::DeriveParams {
        master_secret: &[0x31; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: b"custom-options",
    })
    .unwrap()
}
fn check_request(wire: &[u8], protected: bool) -> bool {
    let parsed = decode(wire).unwrap();
    if !parsed.code().is_request() {
        return false;
    }
    #[cfg(feature = "oscore")]
    if protected {
        assert!(parsed.oscore().is_some());
        assert!(
            parsed
                .options()
                .all(|option| option.number() != A && option.number() != B)
        );
        // Independent inspection context: never consumes the real peer's replay state.
        let mut inspection = context(&[2], &[1]);
        let mut bytes = [0; 1280];
        let (inner, _) = inspection.unprotect_request(&parsed, &mut bytes).unwrap();
        assert_custom(&inner);
        return true;
    }
    assert!(!protected);
    assert_custom(&parsed);
    true
}
fn echo(request: Request<'_>) -> Response<'static> {
    let values: Vec<_> = request
        .options()
        .filter(|option| option.number() == A)
        .map(|option| option.value())
        .collect();
    assert_eq!(values, [b"first".as_slice(), b"second".as_slice()]);
    let body = request.body().unwrap_or(request.payload());
    if body.is_empty() {
        // This pure handler returns the same representation on independent
        // response selections; a selector does not repeat the upload body.
        let selection = |number| {
            request
                .get_option(number)
                .and_then(|option| BlockValue::decode(option.value()).ok())
        };
        assert!(
            selection(OptionNumber::BLOCK2).is_some_and(|block| block.num() > 0)
                || selection(OptionNumber::Q_BLOCK2)
                    .is_some_and(|block| block.num() > 0 || !block.more())
        );
    } else {
        assert_eq!(body, BODY);
    }
    Response::content(&BODY)
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"v1")
}
fn plain(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.get_option(A).unwrap().value(), b"first");
    Response::content(b"small")
}
fn snapshot() -> Response<'static> {
    Response::content(b"small")
}
fn selected(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.payload(), b"selection");
    assert_eq!(request.get_option(A).unwrap().value(), b"first");
    Response::content(&BODY)
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"v1")
}
fn download(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.get_option(A).unwrap().value(), b"first");
    Response::content(&BODY)
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"v1")
}
fn pair(protected: bool) -> (TestApp, TestApp) {
    let client = App::profile::<Buffers>()
        .block_wise::<true>()
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Io::default())
        .unwrap();
    let server = App::profile::<Buffers>()
        .block_wise::<true>()
        .deterministic_for_tests()
        .allow_unprotected()
        .route("echo", post(echo))
        .route("small", get(plain).observe(snapshot))
        .route("selected", crate::fetch(selected))
        .route("download", get(download))
        .bind(Io::default())
        .unwrap();
    #[cfg(feature = "oscore")]
    let (mut client, mut server) = (client, server);
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    let _ = protected;
    (client, server)
}
struct Reply {
    payload: Vec<u8>,
    body: Option<Vec<u8>>,
    observe: Option<u32>,
}
impl Reply {
    fn payload(&self) -> &[u8] {
        &self.payload
    }
    fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }
    fn observe_seq(&self) -> Option<u32> {
        self.observe
    }
}
fn exchange(
    client: &mut TestApp,
    server: &mut TestApp,
    call: Call,
    asynchronous: bool,
    protected: bool,
    now: u64,
) -> (Reply, usize) {
    exchange_with_loss(client, server, call, asynchronous, protected, now, false)
}
fn exchange_with_loss(
    client: &mut TestApp,
    server: &mut TestApp,
    call: Call,
    asynchronous: bool,
    protected: bool,
    now: u64,
    drop_response: bool,
) -> (Reply, usize) {
    let mut requests = 0;
    let mut dropped = false;
    for step in 0..256 {
        let tick = now + step * if drop_response { 100 } else { 1 };
        while let Some(wire) = client.transport_mut().output.pop_front() {
            requests += usize::from(check_request(&wire, protected));
            server.transport_mut().input.push_back(wire);
        }
        progress(server, asynchronous, tick);
        if drop_response && !dropped && server.transport().output.len() > 1 {
            // This case exercises an observed hole. The no-custom tests below
            // separately cover trailing-loss recovery.
            assert!(server.transport_mut().output.pop_front().is_some());
            dropped = true;
        }
        while let Some(wire) = server.transport_mut().output.pop_front() {
            client.transport_mut().input.push_back(wire);
        }
        progress(client, asynchronous, tick);
        if let Some(response) = client.take_response(call) {
            let response = response.unwrap();
            assert!(
                !drop_response || dropped,
                "Q response loss was actually injected"
            );
            return (
                Reply {
                    payload: response.payload().to_vec(),
                    body: response.body().map(<[u8]>::to_vec),
                    observe: response.observe_seq(),
                },
                requests,
            );
        }
    }
    panic!("bounded custom-option exchange did not complete");
}

#[test]
fn custom_options_sync_async_wire_retry_and_fragment_continuations() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            let (mut client, mut server) = pair(protected);
            std::println!("custom ordinary async={asynchronous} protected={protected}");
            let call = {
                let first = b"first".to_vec();
                let second = b"second".to_vec();
                let last = b"last".to_vec();
                let borrowed = [
                    Opt::new(B, &last),
                    Opt::new(A, &first),
                    Opt::new(A, &second),
                ];
                start!(
                    asynchronous,
                    client.get("small").custom_options(&borrowed).to(PEER),
                    0
                )
                .unwrap()
            }; // Values and their option descriptors are gone before retransmission.
            let original = client.transport_mut().output.pop_front().unwrap();
            assert!(check_request(&original, protected));
            let due = client.next_timeout().unwrap();
            progress(&mut client, asynchronous, due);
            assert_eq!(client.transport().output.front().unwrap(), &original);
            let (response, count) = exchange(
                &mut client,
                &mut server,
                call,
                asynchronous,
                protected,
                due + 1,
            );
            assert_eq!(response.payload(), b"small");
            assert_eq!(count, 1);
            assert_eq!(client.engine_mut().tx_occupied(), 0);
            for q in [false, true] {
                std::println!("custom transfer async={asynchronous} protected={protected} q={q}");
                let (mut client, mut server) = pair(protected);
                let mut request = client
                    .post("echo")
                    .payload(&BODY)
                    .content_format(ContentFormat::OCTET_STREAM)
                    .accept(ContentFormat::OCTET_STREAM)
                    .custom_options(&CUSTOM)
                    .to(PEER);
                if q {
                    request = request
                        .non()
                        .q_block1()
                        .q_block2()
                        .request_tag(BodyTag::new(b"custom").unwrap());
                } else {
                    request = request.block2(BlockValue::new(0, false, 3).unwrap());
                }
                let call = start!(asynchronous, request, 0).unwrap();
                let (response, count) =
                    exchange(&mut client, &mut server, call, asynchronous, protected, 1);
                assert_eq!(response.body(), Some(BODY.as_slice()));
                assert!(
                    count >= if q { 2 } else { 3 },
                    "upload fragments and response selectors were checked"
                );
                assert_eq!(client.engine_mut().tx_occupied(), 0);
                assert_eq!(client.engine_mut().rx_occupied(), 0);
            }
        }
    }
}

#[test]
fn custom_fetch_small_selection_survives_block2_followups() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            for q in [false, true] {
                std::println!("custom FETCH async={asynchronous} protected={protected} q={q}");
                let (mut client, mut server) = pair(protected);
                let mut request = client
                    .fetch("selected")
                    .payload(b"selection")
                    .content_format(ContentFormat::OCTET_STREAM)
                    .accept(ContentFormat::OCTET_STREAM)
                    .custom_options(&CUSTOM)
                    .to(PEER);
                if q {
                    request = request
                        .non()
                        .q_block2()
                        .request_tag(BodyTag::new(b"fetch").unwrap());
                } else {
                    request = request.block2(BlockValue::new(0, false, 3).unwrap());
                }
                let call = start!(asynchronous, request, 0).unwrap();
                let (response, count) = exchange_with_loss(
                    &mut client,
                    &mut server,
                    call,
                    asynchronous,
                    protected,
                    1,
                    q,
                );
                assert_eq!(response.body(), Some(BODY.as_slice()));
                assert!(
                    count >= 2,
                    "selection sent on initial request and response continuation"
                );
            }
        }
    }
}

#[test]
fn custom_get_q_block2_loss_recovery_preserves_options() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            std::println!("custom Q GET recovery async={asynchronous} protected={protected}");
            let (mut client, mut server) = pair(protected);
            let call = start!(
                asynchronous,
                client
                    .get("download")
                    .non()
                    .q_block2()
                    .custom_options(&CUSTOM)
                    .to(PEER),
                0
            )
            .unwrap();
            let (response, count) = exchange_with_loss(
                &mut client,
                &mut server,
                call,
                asynchronous,
                protected,
                1,
                true,
            );
            assert_eq!(response.body(), Some(BODY.as_slice()));
            assert!(count >= 2, "Q recovery emitted a new selector request");
        }
    }
}

#[test]
fn custom_option_refusals_precede_io_and_leave_call_capacity_reusable() {
    for asynchronous in [false, true] {
        let (mut client, _) = pair(false);
        for number in [
            OptionNumber::new(0),
            OptionNumber::CONTENT_FORMAT,
            OptionNumber::new(65001),
        ] {
            let options = [Opt::new(number, b"invalid")];
            assert!(
                matches!(start!(asynchronous, client.get("small").custom_options(&options).to(PEER), 0),
                Err(Error::CustomOptionNotAllowed(found)) if found == number)
            );
        }
        let oversized = [Opt::new(A, &[0x55; 512])];
        assert!(matches!(
            start!(
                asynchronous,
                client.get("small").custom_options(&oversized).to(PEER),
                0
            ),
            Err(Error::CustomRequestTooLarge)
        ));
        let too_many = [Opt::new(A, b"x"); 27];
        assert!(
            start!(
                asynchronous,
                client.get("small").custom_options(&too_many).to(PEER),
                0
            )
            .is_err()
        );
        assert_eq!(client.transport().sent, 0);
        assert_eq!(client.engine_mut().tx_occupied(), 0);
        let too_many_controls = [Opt::new(A, b"x"); 13];
        assert!(matches!(
            start!(
                asynchronous,
                client
                    .post("echo")
                    .payload(&BODY)
                    .custom_options(&too_many_controls)
                    .to(PEER),
                0
            ),
            Err(Error::Block(BlockTransferError::Overflow))
        ));
        assert_eq!(client.transport().sent, 0);
        assert_eq!(client.engine_mut().tx_occupied(), 0);
        for index in 0..client.engine().capacities().tx_body_slots.unwrap() {
            assert!(
                client
                    .engine()
                    .tx_body_transfer(SlotId::from_index(index))
                    .is_none()
            );
        }
        let call = start!(
            asynchronous,
            client.get("small").custom_options(&CUSTOM).to(PEER),
            0
        )
        .unwrap();
        assert!(client.cancel(call));
        assert!(matches!(
            client.take_response(call),
            Some(Err(CallFailure::Cancelled))
        ));
        assert_eq!(client.engine_mut().tx_occupied(), 0);
    }
}

#[test]
fn custom_observe_deregistration_requires_exact_option_values() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            let (mut client, mut server) = pair(protected);
            let call = start!(
                asynchronous,
                client
                    .get("small")
                    .observe()
                    .custom_options(&CUSTOM)
                    .to(PEER),
                0
            )
            .unwrap();
            let (response, _) =
                exchange(&mut client, &mut server, call, asynchronous, protected, 1);
            assert!(response.observe_seq().is_some());
            let mismatch = [Opt::new(A, b"different")];
            let sent = client.transport().sent;
            assert!(matches!(
                start!(
                    asynchronous,
                    client
                        .get("small")
                        .deregister_call(call)
                        .custom_options(&mismatch)
                        .to(PEER),
                    2
                ),
                Err(Error::ObserveCancellationMismatch)
            ));
            assert_eq!(client.transport().sent, sent);
            let cancelled = start!(
                asynchronous,
                client
                    .get("small")
                    .deregister_call(call)
                    .custom_options(&CUSTOM)
                    .to(PEER),
                3
            )
            .unwrap();
            assert_eq!(cancelled, call);
            let (response, _) = exchange(
                &mut client,
                &mut server,
                cancelled,
                asynchronous,
                protected,
                4,
            );
            assert!(response.observe_seq().is_none());
            assert_eq!(response.payload(), b"small");
        }
    }
}

// No custom options: this reproduces the independent trailing-only Q2 loss bug.
fn bare_download(_: Request<'_>) -> Response<'static> {
    Response::content(&BODY).etag(b"tail")
}
fn bare_small(_: Request<'_>) -> Response<'static> {
    Response::content(b"fresh")
}
fn bare_pair(protected: bool) -> (TestApp, TestApp) {
    let client = App::profile::<Buffers>()
        .block_wise::<true>()
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Io::default())
        .unwrap();
    let server = App::profile::<Buffers>()
        .block_wise::<true>()
        .deterministic_for_tests()
        .allow_unprotected()
        .route("tail", get(bare_download))
        .route("fresh", get(bare_small))
        .bind(Io::default())
        .unwrap();
    #[cfg(feature = "oscore")]
    let (mut client, mut server) = (client, server);
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    let _ = protected;
    (client, server)
}
fn forward(source: &mut TestApp, destination: &mut TestApp) {
    while let Some(wire) = source.transport_mut().output.pop_front() {
        destination.transport_mut().input.push_back(wire);
    }
}
fn prefix_only(client: &mut TestApp, server: &mut TestApp, asynchronous: bool) {
    forward(client, server);
    progress(server, asynchronous, 0);
    let prefix = server
        .transport_mut()
        .output
        .pop_front()
        .expect("NUM0 response");
    assert!(
        server.transport_mut().output.pop_front().is_some(),
        "trailing response lost"
    );
    assert!(server.transport().output.is_empty());
    client.transport_mut().input.push_back(prefix);
    progress(client, asynchronous, 1);
    let transfer = (0..client.engine().capacities().rx_body_slots.unwrap())
        .find_map(|index| client.engine().rx_body_transfer(SlotId::from_index(index)))
        .expect("partial response remains owned");
    assert_eq!(transfer.filled(), 1024);
    assert!(!transfer.is_complete());
    assert_eq!(transfer.expected_len(), Some(2000));
}
fn assert_tail_selector(wire: &[u8], protected: bool, token: Token) {
    fn inspect(parsed: &ParsedMessage<'_>, token: Token) {
        assert_eq!(parsed.ty(), Type::NonConfirmable);
        assert_eq!(parsed.code(), Code::GET);
        assert_eq!(parsed.token(), token);
        assert_eq!(
            parsed.uri_path().map(Result::unwrap).collect::<Vec<_>>(),
            ["tail"]
        );
        let blocks = parsed.q_block2().map(Result::unwrap).collect::<Vec<_>>();
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            (blocks[0].num(), blocks[0].more(), blocks[0].szx()),
            (1, false, 6)
        );
        assert!(parsed.etag().next().is_none());
        assert!(parsed.observe().is_none());
    }
    let parsed = decode(wire).unwrap();
    #[cfg(feature = "oscore")]
    if protected {
        let mut inspection = context(&[2], &[1]);
        let mut bytes = [0; 1280];
        let (inner, _) = inspection.unprotect_request(&parsed, &mut bytes).unwrap();
        inspect(&inner, token);
        return;
    }
    assert!(!protected);
    inspect(&parsed, token);
}
fn assert_client_reclaimed(client: &mut TestApp) {
    assert_eq!(client.engine_mut().tx_occupied(), 0);
    assert_eq!(client.engine_mut().rx_occupied(), 0);
    for index in 0..client.engine().capacities().rx_body_slots.unwrap() {
        assert!(
            client
                .engine()
                .rx_body_transfer(SlotId::from_index(index))
                .is_none()
        );
    }
}
fn fresh_completion(client: &mut TestApp, server: &mut TestApp, asynchronous: bool, now: u64) {
    let call = start!(asynchronous, client.get("fresh").non().to(PEER), now).unwrap();
    forward(client, server);
    progress(server, asynchronous, now);
    forward(server, client);
    progress(client, asynchronous, now + 1);
    assert_eq!(
        client.take_response(call).unwrap().unwrap().payload(),
        b"fresh"
    );
    assert_client_reclaimed(client);
}

#[test]
fn no_custom_q2_trailing_loss_recovers_on_receive_timeout() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            std::println!("no-custom Q2 tail async={asynchronous} protected={protected}");
            let (mut client, mut server) = bare_pair(protected);
            let call = start!(
                asynchronous,
                client.get("tail").non().q_block2().to(PEER),
                0
            )
            .unwrap();
            prefix_only(&mut client, &mut server, asynchronous);
            assert_eq!(client.next_timeout(), Some(4001));
            progress(&mut client, asynchronous, 4000);
            assert!(
                client.transport().output.is_empty(),
                "no premature recovery"
            );
            progress(&mut client, asynchronous, 4001);
            assert_eq!(client.transport().output.len(), 1);
            assert_tail_selector(
                client.transport().output.front().unwrap(),
                protected,
                call.token(),
            );
            forward(&mut client, &mut server);
            progress(&mut server, asynchronous, 4002);
            forward(&mut server, &mut client);
            progress(&mut client, asynchronous, 4003);
            assert_eq!(
                client.take_response(call).unwrap().unwrap().body(),
                Some(BODY.as_slice())
            );
            assert_client_reclaimed(&mut client);
            fresh_completion(&mut client, &mut server, asynchronous, 4004);
        }
    }
}

#[test]
fn no_custom_q2_prefix_deadline_cancel_and_retry_exhaustion_reclaim() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            for termination in 0..3 {
                let (mut client, mut server) = bare_pair(protected);
                let request = client.get("tail").non().q_block2().to(PEER);
                let request = if termination == 0 {
                    request.deadline(3000)
                } else {
                    request
                };
                let call = start!(asynchronous, request, 0).unwrap();
                prefix_only(&mut client, &mut server, asynchronous);
                let (expected, now) = if termination == 0 {
                    assert_eq!(client.next_timeout(), Some(3000));
                    progress(&mut client, asynchronous, 3000);
                    (CallFailure::DeadlineExceeded, 3000)
                } else if termination == 1 {
                    assert!(client.cancel(call));
                    (CallFailure::Cancelled, 1)
                } else {
                    let mut now = 1;
                    for _ in 0..=crate::message::QBlockTransmission::NON_MAX_RETRANSMIT {
                        now = client.next_timeout().expect("bounded retry wake");
                        assert!(now < 145000, "Q retry exhaustion precedes NON lifetime");
                        progress(&mut client, asynchronous, now);
                        client.transport_mut().output.clear(); // All recovery requests lost.
                    }
                    (CallFailure::TimedOut, now)
                };
                assert_eq!(client.take_response(call).unwrap().unwrap_err(), expected);
                assert_client_reclaimed(&mut client);
                fresh_completion(&mut client, &mut server, asynchronous, now + 1);
            }
        }
    }
}

#[test]
fn no_custom_con_q2_prefix_does_not_gain_non_receive_timeout() {
    for asynchronous in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            let (mut client, _) = bare_pair(protected);
            // A separate CON response keeps the existing CON receive policy.
            let call = start!(asynchronous, client.get("tail").q_block2().to(PEER), 0).unwrap();
            let request = client.transport_mut().output.pop_front().unwrap();
            client.transport_mut().input.push_back(con_prefix_wire(
                &request,
                protected,
                call.token(),
            ));
            progress(&mut client, asynchronous, 1);
            let ack = client
                .transport_mut()
                .output
                .pop_front()
                .expect("CON response acknowledged");
            let ack = decode(&ack).unwrap();
            assert!(ack.is_empty_ack());
            assert_eq!(ack.message_id(), MessageId::new(700));
            assert!(client.transport().output.is_empty());
            assert!(client.next_timeout().is_none_or(|due| due > 4001));
            progress(&mut client, asynchronous, 4001);
            assert!(client.transport().output.is_empty());
            assert!(client.cancel(call));
            assert_eq!(
                client.take_response(call).unwrap().unwrap_err(),
                CallFailure::Cancelled
            );
            assert_client_reclaimed(&mut client);
        }
    }
}

fn con_prefix_wire(request_wire: &[u8], protected: bool, token: Token) -> Vec<u8> {
    let block = BlockValue::new(0, true, 6).unwrap().encode();
    let size = crate::message::encode_uint(2000);
    let options = [Opt::etag(b"tail"), Opt::size2(&size), Opt::q_block2(&block)];
    let message = Message::new(Type::Confirmable, Code::CONTENT, MessageId::new(700))
        .with_token(token)
        .with_options(&options)
        .with_payload(&BODY[..1024]);
    let mut wire = [0; 1280];
    #[cfg(feature = "oscore")]
    if protected {
        let mut peer = context(&[2], &[1]);
        let request = decode(request_wire).unwrap();
        let mut opened = [0; 1280];
        let (_, reference) = peer.unprotect_request(&request, &mut opened).unwrap();
        let n = peer
            .protect_response_with_piv(&message, reference, &mut wire)
            .unwrap();
        return wire[..n].to_vec();
    }
    let _ = request_wire;
    assert!(!protected);
    let n = crate::message::encode(&message, &mut wire).unwrap();
    wire[..n].to_vec()
}
