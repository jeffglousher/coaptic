//! Library unit tests for RFC 7252 decode/encode. Not plugtest.

use super::{Code, Message, MessageId, Opt, OptionNumber, Token, Type, decode, encode};
use crate::MemoryProfile;
use crate::error::{EncodeError, ParseError};
use crate::profiles;
use crate::storage::{EngineBuilder, Memory, SlotPool};

fn encode_to<'a>(msg: &Message<'_>, buf: &'a mut [u8]) -> &'a [u8] {
    let n = encode(msg, buf).expect("encode");
    &buf[..n]
}

fn assert_roundtrip(msg: &Message<'_>) {
    let mut buf = [0u8; 1024];
    let n = encode(msg, &mut buf).expect("encode");
    let parsed = decode(&buf[..n]).expect("decode");
    assert_eq!(parsed.ty(), msg.ty());
    assert_eq!(parsed.code(), msg.code());
    assert_eq!(parsed.message_id(), msg.message_id());
    assert_eq!(parsed.token(), msg.token());
    assert_eq!(parsed.payload(), msg.payload());

    let mut expected = msg.options().iter();
    for opt in parsed.options() {
        let want = expected.next().expect("decoded extra option");
        assert_eq!(opt.number(), want.number());
        assert_eq!(opt.value(), want.value());
    }
    assert!(expected.next().is_none());

    let mut again = [0u8; 1024];
    let n2 = parsed.encode(&mut again).expect("re-encode");
    assert_eq!(&buf[..n], &again[..n2]);
}

#[test]
fn roundtrip_get_uri_path() {
    let opts = [Opt::new(OptionNumber::URI_PATH, b"temp")];
    let token = Token::new(&[0x21]).expect("token");
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(0x1234))
        .with_token(token)
        .with_options(&opts);
    assert_roundtrip(&msg);

    let mut buf = [0u8; 32];
    let bytes = encode_to(&msg, &mut buf);
    // ver=1 T=CON TKL=1 code=GET mid=0x1234 token=0x21 Uri-Path "temp"
    assert_eq!(
        bytes,
        &[0x41, 0x01, 0x12, 0x34, 0x21, 0xb4, b't', b'e', b'm', b'p']
    );
}

#[test]
fn roundtrip_post_with_payload() {
    let opts = [Opt::new(OptionNumber::CONTENT_FORMAT, &[])];
    let msg = Message::new(Type::NonConfirmable, Code::POST, MessageId::new(0x0001))
        .with_options(&opts)
        .with_payload(b"hi");
    assert_roundtrip(&msg);
    assert!(msg.code().is_request());
}

#[test]
fn roundtrip_empty_ack() {
    let msg = Message::new(Type::Acknowledgement, Code::EMPTY, MessageId::new(0x00ab));
    assert_roundtrip(&msg);
    let mut buf = [0u8; 8];
    assert_eq!(encode_to(&msg, &mut buf), &[0x60, 0x00, 0x00, 0xab]);
}

#[test]
fn roundtrip_empty_rst() {
    let msg = Message::new(Type::Reset, Code::EMPTY, MessageId::new(7));
    assert_roundtrip(&msg);
}

#[test]
fn roundtrip_extended_delta_and_length() {
    let thirteen = [b'x'; 13];
    let opts = [
        Opt::new(OptionNumber::URI_PATH, &thirteen),
        Opt::new(OptionNumber::SIZE1, &[0x10]),
        Opt::new(OptionNumber::new(269), b"z"),
    ];
    let msg = Message::new(Type::Confirmable, Code::PUT, MessageId::new(9)).with_options(&opts);
    assert_roundtrip(&msg);
}

#[test]
fn roundtrip_repeatable_uri_path() {
    let opts = [
        Opt::new(OptionNumber::URI_PATH, b"a"),
        Opt::new(OptionNumber::URI_PATH, b"b"),
    ];
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(1)).with_options(&opts);
    assert_roundtrip(&msg);
}

#[test]
fn payload_ff_inside_option_value_is_not_marker() {
    let opts = [Opt::new(OptionNumber::URI_PATH, &[0xff])];
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(2)).with_options(&opts);
    assert_roundtrip(&msg);
}

#[test]
fn reject_truncated_header() {
    assert_eq!(
        decode(&[0x40, 0x01, 0x00]),
        Err(ParseError::TruncatedHeader)
    );
    assert_eq!(decode(&[]), Err(ParseError::TruncatedHeader));
}

#[test]
fn reject_unsupported_version() {
    assert_eq!(
        decode(&[0x00, 0x01, 0x00, 0x01]),
        Err(ParseError::UnsupportedVersion)
    );
    assert_eq!(
        decode(&[0x80, 0x01, 0x00, 0x01]),
        Err(ParseError::UnsupportedVersion)
    );
}

#[test]
fn reject_bad_tkl() {
    // TKL 9, enough following bytes that only TKL is wrong.
    let mut buf = [0u8; 16];
    buf[0] = 0x49;
    buf[1] = 0x01;
    assert_eq!(decode(&buf), Err(ParseError::BadTokenLength));
}

#[test]
fn reject_truncated_token() {
    // TKL 2 but only one byte after the header.
    assert_eq!(
        decode(&[0x42, 0x01, 0x00, 0x01, 0xaa]),
        Err(ParseError::TruncatedToken)
    );
}

#[test]
fn accept_tkl_8() {
    let token = Token::new(&[1, 2, 3, 4, 5, 6, 7, 8]).expect("8-byte token");
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(1)).with_token(token);
    assert_roundtrip(&msg);
}

#[test]
fn reject_token_new_too_long() {
    assert!(Token::new(&[0; 9]).is_none());
}

#[test]
fn reject_option_value_overflow() {
    // delta=11 length=4 but only two value bytes.
    assert_eq!(
        decode(&[0x40, 0x01, 0x00, 0x01, 0xb4, b'a', b'b']),
        Err(ParseError::TruncatedOption)
    );
}

#[test]
fn reject_option_number_overflow() {
    // Delta 14 + 0xFFFF => 269 + 65535, which does not fit in u16.
    assert_eq!(
        decode(&[0x40, 0x01, 0x00, 0x01, 0xe0, 0xff, 0xff]),
        Err(ParseError::OptionNumberOverflow)
    );
}

#[test]
fn reject_reserved_option_delta() {
    assert_eq!(
        decode(&[0x40, 0x01, 0x00, 0x01, 0xf0]),
        Err(ParseError::ReservedOptionDelta)
    );
}

#[test]
fn reject_reserved_option_length() {
    assert_eq!(
        decode(&[0x40, 0x01, 0x00, 0x01, 0x0f]),
        Err(ParseError::ReservedOptionLength)
    );
}

#[test]
fn reject_truncated_extended_delta() {
    assert_eq!(
        decode(&[0x40, 0x01, 0x00, 0x01, 0xd0]),
        Err(ParseError::TruncatedOption)
    );
}

#[test]
fn reject_payload_marker_without_payload() {
    assert_eq!(
        decode(&[0x40, 0x01, 0x00, 0x01, 0xff]),
        Err(ParseError::PayloadMarkerWithoutPayload)
    );
}

#[test]
fn reject_empty_message_with_extra_bytes() {
    assert_eq!(
        decode(&[0x60, 0x00, 0x00, 0x01, 0x00]),
        Err(ParseError::EmptyMessageNotEmpty)
    );
}

#[test]
fn reject_empty_message_with_token() {
    assert_eq!(
        decode(&[0x61, 0x00, 0x00, 0x01, 0xaa]),
        Err(ParseError::EmptyMessageNotEmpty)
    );
}

#[test]
fn encode_empty_with_payload_fails() {
    let msg =
        Message::new(Type::Acknowledgement, Code::EMPTY, MessageId::new(1)).with_payload(b"x");
    let mut buf = [0u8; 16];
    assert_eq!(
        encode(&msg, &mut buf),
        Err(EncodeError::EmptyMessageNotEmpty)
    );
}

#[test]
fn encode_options_must_ascend() {
    let opts = [
        Opt::new(OptionNumber::URI_PATH, b"a"),
        Opt::new(OptionNumber::IF_MATCH, b""),
    ];
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(1)).with_options(&opts);
    let mut buf = [0u8; 32];
    assert_eq!(
        encode(&msg, &mut buf),
        Err(EncodeError::OptionsNotAscending)
    );
}

#[test]
fn encode_buffer_too_small() {
    let msg = Message::new(Type::Acknowledgement, Code::EMPTY, MessageId::new(1));
    let mut buf = [0u8; 3];
    assert_eq!(encode(&msg, &mut buf), Err(EncodeError::BufferTooSmall));
}

#[test]
fn unrecognized_critical_is_structured_not_policy() {
    // Option 23 (Block2) is critical and not in RFC 7252 Table 4.
    // Decode accepts it as opaque; check_rfc7252_options reports it.
    let opts = [Opt::new(OptionNumber::new(23), &[0x02])];
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(3)).with_options(&opts);
    let mut buf = [0u8; 32];
    let bytes = encode_to(&msg, &mut buf);
    let parsed = decode(bytes).expect("opaque parse");
    assert_eq!(parsed.unrecognized_critical(), Some(OptionNumber::new(23)));
    assert_eq!(
        parsed.check_rfc7252_options(),
        Err(ParseError::UnrecognizedCritical(OptionNumber::new(23)))
    );

    // Observe (6) is elective; not reported.
    let opts = [Opt::new(OptionNumber::new(6), &[0x00])];
    let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(4)).with_options(&opts);
    let bytes = encode_to(&msg, &mut buf);
    let parsed = decode(bytes).expect("opaque parse");
    assert_eq!(parsed.unrecognized_critical(), None);
    parsed.check_rfc7252_options().expect("elective unknown");
}

#[test]
fn code_helpers() {
    assert!(Code::GET.is_request());
    assert!(!Code::GET.is_response());
    assert!(Code::CONTENT.is_success());
    assert!(Code::NOT_FOUND.is_client_error());
    assert!(Code::INTERNAL_SERVER_ERROR.is_server_error());
    assert!(Code::CONTENT.is_response());
    assert_eq!(Code::from_class_detail(2, 5), Some(Code::CONTENT));
    assert_eq!((Code::GET.class(), Code::GET.detail()), (0, 1));
    assert_eq!(Type::Confirmable.to_bits(), 0);
    assert_eq!(Type::from_bits(2), Some(Type::Acknowledgement));
}

#[test]
fn decode_from_datagram_pool_slot() {
    let mut engine = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::new())
        .expect("storage build");

    let id = engine.acquire_rx().expect("rx slot");
    let opts = [Opt::new(OptionNumber::URI_PATH, b"slot")];
    let msg =
        Message::new(Type::Confirmable, Code::GET, MessageId::new(0x2222)).with_options(&opts);

    let n = {
        let slot = engine
            .storage_mut()
            .rx_datagram_mut()
            .payload_mut(id)
            .expect("occupied");
        encode(&msg, slot).expect("encode into slot")
    };
    engine
        .storage_mut()
        .rx_datagram_mut()
        .set_len(id, n)
        .expect("set_len");

    let bytes = engine.storage().rx_datagram().payload(id).expect("filled");
    let parsed = decode(bytes).expect("decode slot bytes");
    assert_eq!(parsed.code(), Code::GET);
    assert_eq!(parsed.message_id(), MessageId::new(0x2222));
    let opt = parsed.options().next().expect("Uri-Path");
    assert_eq!(opt.number(), OptionNumber::URI_PATH);
    assert_eq!(opt.value(), b"slot");
    assert!(engine.storage().rx_datagram().slot_count() >= 1);
}

#[test]
fn default_datagram_slot_is_1472() {
    assert_eq!(profiles::Default::RX_DATAGRAM_BYTES, 1472);
}
