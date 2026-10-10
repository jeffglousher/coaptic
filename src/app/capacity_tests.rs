//! Client table bounds are independent of packet pools and allocator selection.
use super::{App, AppStorage, DEFAULT_ROUTES, Error, Response, get};
use crate::message::{Code, Message, MessageId, Opt, Type, decode, encode};
use crate::storage::{self, DatagramIo, Endpoint, MemoryProfile};

struct Eight;
impl MemoryProfile for Eight {
    const RX_DATAGRAM_SLOTS: usize = 8;
    const RX_DATAGRAM_BYTES: usize = 256;
    const TX_DATAGRAM_SLOTS: usize = 8;
    const TX_DATAGRAM_BYTES: usize = 256;
    const DEDUP_ENTRIES: usize = 8;
    const OBSERVE_ENTRIES: usize = 8;
    const RX_BODY_SLOTS: usize = 1;
    const RX_BODY_BYTES: usize = 1024;
    const TX_BODY_SLOTS: usize = 1;
    const TX_BODY_BYTES: usize = 1024;
    type RxDatagram = storage::DatagramPool<8, 256>;
    type TxDatagram = storage::DatagramPool<8, 256>;
    type RxBody = storage::BodyPool<1, 1024>;
    type TxBody = storage::BodyPool<1, 1024>;
    type Dedup = storage::DedupTable<8>;
    type Observe = storage::ObserveTable<8>;
    type Exchange = storage::ExchangeTable<8>;
    type RxScratch = storage::DatagramScratch<256>;
    type TxScratch = storage::DatagramScratch<256>;
}

struct Io {
    incoming: Option<([u8; 256], usize)>,
    sent: usize,
    wire: [u8; 256],
    len: usize,
}
impl Default for Io {
    fn default() -> Self {
        Self {
            incoming: None,
            sent: 0,
            wire: [0; 256],
            len: 0,
        }
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
impl DatagramIo for Io {
    type Error = &'static str;
    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((wire, len)) = self.incoming.take() else {
            return Ok(None);
        };
        buf[..len].copy_from_slice(&wire[..len]);
        Ok(Some((len, PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.wire[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        self.sent += 1;
        Ok(bytes.len())
    }
}

type Client<S, const C: usize> = App<Eight, Io, DEFAULT_ROUTES, false, S, 0, C>;

fn expiry<const C: usize, S: AppStorage>(mut app: Client<S, C>) {
    let mut calls = [None; C];
    for call in &mut calls {
        *call = Some(
            app.get("value")
                .non()
                .deadline(10)
                .to(PEER)
                .send(0)
                .unwrap(),
        );
    }
    assert_eq!(
        app.get("extra").non().to(PEER).send(0),
        Err(Error::Saturated)
    );
    assert_eq!(app.transport().sent, C);
    app.poll(10).unwrap();
    // Failure completions are retained until collected, just like successes.
    assert_eq!(
        app.get("extra").non().to(PEER).send(10),
        Err(Error::Saturated)
    );
    for call in calls.into_iter().flatten() {
        assert_eq!(
            app.take_response(call).unwrap().unwrap_err(),
            crate::CallFailure::DeadlineExceeded
        );
    }
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    app.get("recovered").non().to(PEER).send(11).unwrap();
}

fn completions<const C: usize, S: AppStorage>(mut app: Client<S, C>) {
    let mut calls = [None; C];
    for call in &mut calls {
        let current = app.get("value").to(PEER).send(0).unwrap();
        let request = decode(&app.transport().wire[..app.transport().len]).unwrap();
        let message = Message::new(Type::Acknowledgement, Code::CONTENT, request.message_id())
            .with_token(current.token())
            .with_payload(b"complete");
        let mut wire = [0; 256];
        let len = encode(&message, &mut wire).unwrap();
        app.transport_mut().incoming = Some((wire, len));
        app.poll(1).unwrap();
        *call = Some(current);
    }
    assert_eq!(app.get("extra").to(PEER).send(2), Err(Error::Saturated));
    assert_eq!(app.transport().sent, C);
    for call in calls.into_iter().flatten() {
        let reply = app.take_response(call).unwrap().unwrap();
        assert_eq!(reply.code(), Code::CONTENT);
        assert_eq!(reply.payload(), b"complete");
    }
    app.get("recovered").to(PEER).send(3).unwrap();
}

fn fixed<const C: usize>() {
    let build = || {
        App::profile::<Eight>()
            .client_calls::<C>()
            .block_wise::<false>()
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap()
    };
    expiry(build());
    completions(build());
}

#[test]
fn client_capacity_fixed_success_full_refusal_expiry_and_recovery() {
    fixed::<1>();
    fixed::<4>();
    fixed::<8>();
}

#[cfg(feature = "alloc")]
#[test]
fn client_capacity_alloc_matches_fixed() {
    fn check<const C: usize>() {
        let mut capacities = storage::Capacities::from_profile::<Eight>();
        capacities.rx_body_slots = None;
        capacities.rx_body_bytes = None;
        capacities.tx_body_slots = None;
        capacities.tx_body_bytes = None;
        let build = || {
            App::profile::<Eight>()
                .block_wise::<false>()
                .client_calls::<C>()
                .allow_plaintext()
                .deterministic_for_tests()
                .bind_alloc(Io::default(), capacities)
                .unwrap()
        };
        expiry(build());
        completions(build());
    }
    check::<1>();
    check::<4>();
    check::<8>();
}

#[test]
fn client_capacity_zero_serves_without_client_tables() {
    let mut app = App::profile::<Eight>()
        .client_calls::<0>()
        .routes::<1>()
        .deferred::<1>()
        .block_wise::<false>()
        .route("value", get(|_| Response::content(b"server")))
        .allow_plaintext()
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    assert_eq!(app.client_capacity(), 0);
    assert_eq!(app.get("value").to(PEER).send(0), Err(Error::Saturated));
    assert_eq!(app.transport().sent, 0);
    let options = [Opt::uri_path("value")];
    let message =
        Message::new(Type::Confirmable, Code::GET, MessageId::new(20)).with_options(&options);
    let mut wire = [0; 256];
    let len = encode(&message, &mut wire).unwrap();
    app.transport_mut().incoming = Some((wire, len));
    app.poll(0).unwrap();
    let reply = decode(&app.transport().wire[..app.transport().len]).unwrap();
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.payload(), b"server");
    assert!(matches!(
        App::builder()
            .client_calls::<0>()
            .deterministic_for_tests()
            .bind(Io::default()),
        Err(crate::BuildError::SecurityRequired)
    ));
}

#[test]
fn client_capacity_resizes_inline_metadata_and_preserves_default() {
    fn size<const C: usize>() -> (usize, usize) {
        let app = App::builder()
            .client_calls::<C>()
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        (core::mem::size_of_val(&app), app.client_metadata_bytes())
    }
    let zero = size::<0>();
    let one = size::<1>();
    let four = size::<4>();
    let eight = size::<8>();
    assert!(zero.0 < one.0 && one.0 < four.0 && four.0 < eight.0);
    assert_eq!(eight.0 - four.0, eight.1 - four.1);
    assert_eq!(four.0 - zero.0, four.1 - zero.1);
    let default = App::builder()
        .allow_plaintext()
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    assert_eq!(default.client_capacity(), 4);
    assert_eq!(core::mem::size_of_val(&default), four.0);
}

#[test]
fn client_capacity_full_response_short_output_preserves_slot_and_bytes() {
    fn run<S: AppStorage>(mut app: Client<S, 1>) {
        let call = app.get("value").to(PEER).send(0).unwrap();
        let mid = decode(&app.transport().wire[..app.transport().len])
            .unwrap()
            .message_id();
        let body = [0x42; 200];
        let message = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
            .with_token(call.token())
            .with_payload(&body);
        let mut wire = [0; 256];
        let len = encode(&message, &mut wire).unwrap();
        app.transport_mut().incoming = Some((wire, len));
        app.poll(1).unwrap();
        let mut short = [0x55; 199];
        assert!(matches!(
            app.take_response_into(call, &mut short),
            Err(super::ResponseBufferError::TooSmall {
                required: 200,
                available: 199
            })
        ));
        assert_eq!(short, [0x55; 199]);
        assert_eq!(app.get("extra").to(PEER).send(2), Err(Error::Saturated));
        assert_eq!(app.transport().sent, 1);
        let mut output = [0; 200];
        assert_eq!(
            app.take_response_into(call, &mut output)
                .unwrap()
                .unwrap()
                .unwrap()
                .payload(),
            &body
        );
        assert_eq!(app.engine_mut().rx_occupied(), 0);
        app.get("recovered").to(PEER).send(3).unwrap();
    }
    let builder = || {
        App::profile::<Eight>()
            .client_calls::<1>()
            .block_wise::<false>()
            .full_responses()
            .allow_plaintext()
            .deterministic_for_tests()
    };
    run(builder().bind(Io::default()).unwrap());
    #[cfg(feature = "alloc")]
    {
        let mut c = storage::Capacities::from_profile::<Eight>();
        c.rx_body_slots = None;
        c.rx_body_bytes = None;
        c.tx_body_slots = None;
        c.tx_body_bytes = None;
        run(builder().bind_alloc(Io::default(), c).unwrap());
    }
}

#[cfg(feature = "oscore")]
#[test]
fn client_capacity_does_not_expand_oscore_bindings_or_leak_on_refusal() {
    let context = crate::oscore::SecurityContext::derive(crate::oscore::DeriveParams {
        master_secret: &[1; 16],
        master_salt: &[],
        sender_id: &[1],
        recipient_id: &[2],
        id_context: &[],
    })
    .unwrap();
    let mut app = App::profile::<Eight>()
        .client_calls::<8>()
        .block_wise::<false>()
        .oscore(context)
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    let mut calls = [None; 4];
    for call in &mut calls {
        *call = Some(
            app.get("value")
                .non()
                .deadline(10)
                .to(PEER)
                .send(0)
                .unwrap(),
        );
    }
    for _ in 0..3 {
        assert_eq!(
            app.get("refused").non().to(PEER).send(0),
            Err(Error::Oscore(crate::oscore::Error::Saturated))
        );
    }
    assert_eq!(app.transport().sent, 4);
    let sequence_after_refusal = app.oscore.as_ref().unwrap().sender_seq();
    assert!(sequence_after_refusal >= 4);
    app.poll(10).unwrap();
    for call in calls.into_iter().flatten() {
        assert_eq!(
            app.take_response(call).unwrap().unwrap_err(),
            crate::CallFailure::DeadlineExceeded
        );
    }
    for _ in 0..4 {
        app.get("recovered")
            .non()
            .deadline(20)
            .to(PEER)
            .send(11)
            .unwrap();
    }
    assert_eq!(app.transport().sent, 8);
    assert!(app.oscore.as_ref().unwrap().sender_seq() > sequence_after_refusal);
}

#[test]
fn client_capacity_zero_refuses_before_drawing_request_entropy() {
    let mut app = App::builder()
        .client_calls::<0>()
        .allow_plaintext()
        .randomness(|bytes| {
            assert_eq!(bytes.len(), 2, "only initial MID entropy is needed");
            bytes.fill(1);
            true
        })
        .bind(Io::default())
        .unwrap();
    assert_eq!(app.get("value").to(PEER).send(0), Err(Error::Saturated));
    assert_eq!(app.transport().sent, 0);
}

#[test]
fn client_capacity_observe_holds_slot_until_cancellation() {
    let mut app = App::profile::<Eight>()
        .client_calls::<1>()
        .block_wise::<false>()
        .allow_plaintext()
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    let call = app.get("value").observe().to(PEER).send(0).unwrap();
    let mid = decode(&app.transport().wire[..app.transport().len])
        .unwrap()
        .message_id();
    let options = [Opt::new(crate::message::OptionNumber::OBSERVE, &[1])];
    let message = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
        .with_token(call.token())
        .with_options(&options)
        .with_payload(b"observed");
    let mut wire = [0; 256];
    let len = encode(&message, &mut wire).unwrap();
    app.transport_mut().incoming = Some((wire, len));
    app.poll(1).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap().payload(),
        b"observed"
    );
    assert_eq!(app.get("other").to(PEER).send(2), Err(Error::Saturated));
    assert_eq!(app.get("value").observe().to(PEER).send(2).unwrap(), call);
    assert_eq!(
        app.transport().sent,
        1,
        "duplicate registration reuses its slot"
    );
    assert_eq!(
        app.get("value")
            .deregister_call(call)
            .to(PEER)
            .send(3)
            .unwrap(),
        call
    );
    let mid = decode(&app.transport().wire[..app.transport().len])
        .unwrap()
        .message_id();
    let message = Message::new(Type::Acknowledgement, Code::CONTENT, mid).with_token(call.token());
    let len = encode(&message, &mut wire).unwrap();
    app.transport_mut().incoming = Some((wire, len));
    app.poll(4).unwrap();
    app.take_response(call).unwrap().unwrap();
    app.get("other").to(PEER).send(5).unwrap();
}

#[cfg(feature = "oscore")]
#[test]
fn client_capacity_protected_completions_refuse_and_recover() {
    fn run<const C: usize>() {
        fn context(sender: &[u8], recipient: &[u8]) -> crate::oscore::SecurityContext {
            crate::oscore::SecurityContext::derive(crate::oscore::DeriveParams {
                master_secret: &[2; 16],
                master_salt: &[],
                sender_id: sender,
                recipient_id: recipient,
                id_context: &[],
            })
            .unwrap()
        }
        let mut client = App::profile::<Eight>()
            .client_calls::<C>()
            .block_wise::<false>()
            .oscore(context(&[1], &[2]))
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        let mut server = App::profile::<Eight>()
            .client_calls::<0>()
            .block_wise::<false>()
            .oscore(context(&[2], &[1]))
            .deterministic_for_tests()
            .route("value", get(|_| Response::content(b"protected")))
            .bind(Io::default())
            .unwrap();
        let mut calls = [None; C];
        for call in &mut calls {
            *call = Some(client.get("value").to(PEER).send(0).unwrap());
            server.transport_mut().incoming =
                Some((client.transport().wire, client.transport().len));
            server.poll(1).unwrap();
            client.transport_mut().incoming =
                Some((server.transport().wire, server.transport().len));
            client.poll(2).unwrap();
        }
        let sequence = client.oscore.as_ref().unwrap().sender_seq();
        assert_eq!(client.get("extra").to(PEER).send(3), Err(Error::Saturated));
        assert_eq!(client.oscore.as_ref().unwrap().sender_seq(), sequence);
        assert_eq!(client.transport().sent, C);
        for call in calls.into_iter().flatten() {
            assert_eq!(
                client.take_response(call).unwrap().unwrap().payload(),
                b"protected"
            );
        }
        client.get("value").to(PEER).send(4).unwrap();
    }
    run::<1>();
    run::<4>();
}
