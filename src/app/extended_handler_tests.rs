use super::*;
use crate::message::{Message, MessageId, Opt, OptionsBuilder, Type, decode_with_token, extended};
use crate::storage::{Endpoint, ObserveInterest, ObserveSelection};

type LongToken = extended::Token<32>;

fn token() -> LongToken {
    let mut bytes = [0x41; 32];
    bytes[31] = 0x7f;
    LongToken::new(&bytes).unwrap()
}

fn handler(request: Request<'_, LongToken>) -> Response<'static, LongToken> {
    assert_eq!(request.token(), token());
    assert_eq!(request.path(), &["sensor"]);
    Response::content(b"value").observe(7)
}

#[test]
fn extended_token_request_dispatch_and_discovery() {
    let mut site = Site::<1, LongToken>::new();
    site.route("sensor", super::super::routing::get(handler))
        .well_known_core();
    let mut options = OptionsBuilder::<1>::new();
    options.push(Opt::uri_path("sensor")).unwrap();
    let message =
        Message::con(Code::GET, MessageId::new(9), token()).with_options(options.as_slice());
    let mut wire = [0; 64];
    let n = message.encode(&mut wire).unwrap();
    let request = Request::from_decoded(
        decode_with_token::<LongToken>(&wire[..n]).unwrap(),
        Endpoint::v4([192, 0, 2, 1], 5683),
        None,
    )
    .unwrap();
    let mut scratch = LinkFormatScratch::<1>::new();
    let response = site.dispatch(request, scratch.as_mut());
    assert_eq!(response.payload(), b"value");
    let bad_method =
        Message::con(Code::POST, MessageId::new(10), token()).with_options(options.as_slice());
    let n = bad_method.encode(&mut wire).unwrap();
    let request = Request::from_decoded(
        decode_with_token::<LongToken>(&wire[..n]).unwrap(),
        Endpoint::v4([192, 0, 2, 1], 5683),
        None,
    )
    .unwrap();
    assert_eq!(
        site.dispatch(request, scratch.as_mut()).code(),
        Code::METHOD_NOT_ALLOWED
    );
    let mut discovery_options = OptionsBuilder::<2>::new();
    discovery_options
        .push(Opt::uri_path(".well-known"))
        .unwrap();
    discovery_options.push(Opt::uri_path("core")).unwrap();
    let discovery = Message::con(Code::GET, MessageId::new(11), token())
        .with_options(discovery_options.as_slice());
    let n = discovery.encode(&mut wire).unwrap();
    let request = Request::from_decoded(
        decode_with_token::<LongToken>(&wire[..n]).unwrap(),
        Endpoint::v4([192, 0, 2, 1], 5683),
        None,
    )
    .unwrap();
    assert_eq!(
        site.dispatch(request, scratch.as_mut()).payload(),
        b"</sensor>"
    );
}

#[test]
fn extended_token_snapshot_retains_selection_and_client_metadata() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut options = OptionsBuilder::<1>::new();
    options.push(Opt::uri_path("sensor")).unwrap();
    let message =
        Message::con(Code::GET, MessageId::new(9), token()).with_options(options.as_slice());
    let mut selection = ObserveSelection {
        bytes: [0; crate::storage::BlockTransfer::REQUEST_IDENTITY_BYTES],
        len: 0,
    };
    selection.len = message.encode(&mut selection.bytes).unwrap() as u16;
    let interest = ObserveInterest::new(token(), peer).with_selection(Some(selection));
    let source = SnapshotSource::Selected(handler);
    assert_eq!(
        source.snapshot(&interest, None).unwrap().payload(),
        b"value"
    );

    let response_message = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(9))
        .with_token(token())
        .with_payload(b"value");
    let mut wire = [0; 64];
    let n = response_message.encode(&mut wire).unwrap();
    let parsed = decode_with_token::<LongToken>(&wire[..n]).unwrap();
    let origin = crate::app::client::ReplyMeta::from_parsed(&parsed, peer, 123).unwrap();
    let response: Response<'_, LongToken> = origin.response();
    assert_eq!(response.token(), Some(token()));
    assert_eq!(response.received_at_ms(), Some(123));
    assert_eq!(response.payload(), b"value");
}

#[test]
fn generic_site_and_router_do_not_reserve_token_buffers() {
    assert_eq!(
        core::mem::size_of::<MethodRouter>(),
        core::mem::size_of::<MethodRouter<LongToken>>()
    );
    assert_eq!(
        core::mem::size_of::<Site<1>>(),
        core::mem::size_of::<Site<1, LongToken>>()
    );
}

#[test]
fn app_with_deferred_rows_is_send_for_a_non_sync_token() {
    use crate::message::{OwnedToken, Token};
    use crate::storage;
    use core::{cell::Cell, marker::PhantomData};

    // OwnedToken permits Send without Sync. This marker must not prevent moving
    // an idle App between threads merely because deferred metadata is retained.
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
    struct SendOnlyToken(Token, PhantomData<Cell<()>>);

    impl OwnedToken for SendOnlyToken {
        const EMPTY: Self = Self(Token::EMPTY, PhantomData);
        const MAX_LEN: usize = Token::MAX_LEN;

        fn try_from_bytes(bytes: &[u8]) -> Result<Self, extended::CapacityError> {
            <Token as OwnedToken>::try_from_bytes(bytes).map(|token| Self(token, PhantomData))
        }

        fn try_zeroed(len: usize) -> Result<Self, extended::CapacityError> {
            <Token as OwnedToken>::try_zeroed(len).map(|token| Self(token, PhantomData))
        }

        fn as_bytes_mut(&mut self) -> &mut [u8] {
            <Token as OwnedToken>::as_bytes_mut(&mut self.0)
        }

        fn as_bytes(&self) -> &[u8] {
            self.0.as_bytes()
        }
    }

    struct Tiny;
    impl storage::MemoryProfile for Tiny {
        type Token = SendOnlyToken;
        const RX_DATAGRAM_SLOTS: usize = 2;
        const RX_DATAGRAM_BYTES: usize = 256;
        const TX_DATAGRAM_SLOTS: usize = 2;
        const TX_DATAGRAM_BYTES: usize = 256;
        const DEDUP_ENTRIES: usize = 1;
        const OBSERVE_ENTRIES: usize = 1;
        const RX_BODY_SLOTS: usize = 1;
        const RX_BODY_BYTES: usize = 1024;
        const TX_BODY_SLOTS: usize = 1;
        const TX_BODY_BYTES: usize = 1024;
        type RxDatagram = storage::DatagramPool<2, 256>;
        type TxDatagram = storage::DatagramPool<2, 256>;
        type RxBody = storage::BodyPool<1, 1024, SendOnlyToken>;
        type TxBody = storage::BodyPool<1, 1024, SendOnlyToken>;
        type Dedup = storage::DedupTable<1>;
        type Observe = storage::ObserveTable<1, SendOnlyToken>;
        type Exchange = storage::ExchangeTable<2, SendOnlyToken>;
        type RxScratch = storage::DatagramScratch<256>;
        type ResidentRxScratch = storage::DatagramScratch<256>;
        type TxScratch = storage::DatagramScratch<256>;
    }

    type Store = storage::Memory<Tiny, storage::WithBodies<Tiny>>;
    type MovableApp = crate::App<Tiny, (), 1, true, Store, 1, 1, 1, 1>;
    fn assert_send<T: Send>() {}

    assert_send::<SendOnlyToken>();
    assert_send::<MovableApp>();
}
