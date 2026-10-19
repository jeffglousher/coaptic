use coaptic::error::{EncodeError, ParseError};
use coaptic::message::extended::{self, CapacityError, Message, Token};
use coaptic::message::{self, Code, MessageId, Opt, OptionNumber, Type};

fn request(token: &[u8]) -> Message<'_> {
    Message {
        ty: Type::Confirmable,
        code: Code::GET,
        id: MessageId::new(0x1234),
        token,
        options: &[],
        payload: &[],
    }
}

#[test]
fn rfc8974_boundaries_use_the_specified_token_prefixes() {
    for (len, prefix) in [
        (0, &[0x40, 1, 0x12, 0x34][..]),
        (8, &[0x48, 1, 0x12, 0x34][..]),
        (9, &[0x49, 1, 0x12, 0x34][..]),
        (12, &[0x4c, 1, 0x12, 0x34][..]),
        (13, &[0x4d, 1, 0x12, 0x34, 0][..]),
        (268, &[0x4d, 1, 0x12, 0x34, 255][..]),
        (269, &[0x4e, 1, 0x12, 0x34, 0, 0][..]),
        (65804, &[0x4e, 1, 0x12, 0x34, 255, 255][..]),
    ] {
        let token = vec![0x9a; len];
        let mut wire = vec![0; prefix.len() + len];
        assert_eq!(request(&token).encode(&mut wire), Ok(wire.len()));
        assert_eq!(&wire[..prefix.len()], prefix);
        assert_eq!(&wire[prefix.len()..], token);
        let parsed = extended::decode(&wire).unwrap();
        assert_eq!(parsed.token(), token);
        assert_eq!(parsed.message_id(), MessageId::new(0x1234));
        assert_eq!(parsed.token().as_ptr(), wire[prefix.len()..].as_ptr());
        let mut encoded = vec![0; wire.len()];
        assert_eq!(parsed.encode(&mut encoded), Ok(wire.len()));
        assert_eq!(encoded, wire);
    }
}

#[test]
fn malformed_length_is_distinct_from_owned_capacity() {
    for (wire, error) in [
        (&[0x4f, 1, 0, 0][..], ParseError::BadTokenLength),
        (&[0x4d, 1, 0, 0][..], ParseError::TruncatedToken),
        (&[0x4e, 1, 0, 0][..], ParseError::TruncatedToken),
        (&[0x4e, 1, 0, 0, 0][..], ParseError::TruncatedToken),
        (&[0x4d, 1, 0, 0, 0][..], ParseError::TruncatedToken),
        (&[0x4e, 1, 0, 0, 0, 0][..], ParseError::TruncatedToken),
        (&[0x49, 1, 0, 0][..], ParseError::TruncatedToken),
    ] {
        assert_eq!(extended::decode(wire), Err(error));
    }
    let token = [0x42; 32];
    let mut wire = [0; 37];
    request(&token).encode(&mut wire).unwrap();
    let parsed = extended::decode(&wire).unwrap();
    assert_eq!(
        parsed.copy_token::<8>(),
        Err(CapacityError {
            required: 32,
            available: 8
        })
    );
    assert_eq!(parsed.copy_token::<32>().unwrap().as_bytes(), token);
}

#[test]
fn over_capacity_request_can_receive_full_token_error_without_owned_storage() {
    let token = [0x6a; 300];
    let mut incoming = [0; 306];
    request(&token).encode(&mut incoming).unwrap();
    let parsed = extended::decode(&incoming).unwrap();
    assert!(parsed.copy_token::<32>().is_err());
    let response = Message {
        ty: Type::Acknowledgement,
        code: Code::BAD_REQUEST,
        id: parsed.message_id(),
        token: parsed.token(),
        options: &[],
        payload: &[],
    };
    let mut outgoing = [0; 306];
    assert_eq!(response.encode(&mut outgoing), Ok(306));
    let response = extended::decode(&outgoing).unwrap();
    assert_eq!(response.code(), Code::BAD_REQUEST);
    assert_eq!(response.token(), token);
    assert_eq!(response.ty(), Type::Acknowledgement);
}

#[test]
fn extended_options_and_payload_use_the_same_codec() {
    let token = [0xff; 13];
    let options = [
        Opt::new(OptionNumber::new(11), b"a"),
        Opt::new(OptionNumber::new(15), b"x=y"),
    ];
    let message = Message {
        options: &options,
        payload: b"payload",
        ..request(&token)
    };
    let mut wire = [0; 64];
    let len = message.encode(&mut wire).unwrap();
    // TKL extension then token, then delta/length encoded URI options.
    assert_eq!(&wire[18..len], b"\xb1a\x43x=y\xffpayload");
    let parsed = extended::decode(&wire[..len]).unwrap();
    assert_eq!(parsed.options().collect::<Vec<_>>(), options);
    assert_eq!(parsed.payload(), b"payload");

    for malformed in [&[0xf0][..], &[0x1f][..], &[0xd0][..], &[0xff][..]] {
        let mut extended_wire = wire[..18].to_vec();
        extended_wire.extend_from_slice(malformed);
        let mut ordinary_wire = vec![0x40, 1, 0, 0];
        ordinary_wire.extend_from_slice(malformed);
        assert_eq!(
            extended::decode(&extended_wire).unwrap_err(),
            message::decode(&ordinary_wire).unwrap_err()
        );
    }
}

#[test]
fn empty_rules_and_wire_maximum_are_checked_before_output() {
    let mut output = [0x55; 32];
    assert_eq!(
        Message {
            code: Code::EMPTY,
            ..request(&[1])
        }
        .encode(&mut output),
        Err(EncodeError::EmptyMessageNotEmpty)
    );
    assert_eq!(output, [0x55; 32]);
    let too_long = vec![0; extended::MAX_TOKEN_LEN + 1];
    assert_eq!(
        request(&too_long).encode(&mut output),
        Err(EncodeError::TokenTooLong)
    );
    assert_eq!(output, [0x55; 32]);
    assert_eq!(
        extended::decode(&[0x41, 0, 0, 0, 1]),
        Err(ParseError::EmptyMessageNotEmpty)
    );
    assert_eq!(
        extended::decode(&[0x40, 0, 0, 0, 0]),
        Err(ParseError::EmptyMessageNotEmpty)
    );
}

#[test]
fn short_output_refuses_without_overwriting_token_prefix() {
    let token = [7; 13];
    for cap in 0..18 {
        let mut output = vec![0x55; cap];
        assert_eq!(
            request(&token).encode(&mut output),
            Err(EncodeError::BufferTooSmall)
        );
        assert!(output.iter().all(|b| *b == 0x55));
    }
}

#[test]
fn owned_tokens_compare_all_bytes_and_compact_default_is_unchanged() {
    assert_eq!(core::mem::size_of::<message::Token>(), 9);
    assert_eq!(message::Token::MAX_LEN, 8);
    let mut second = [1; 32];
    second[31] = 2;
    assert_ne!(
        Token::<32>::new(&[1; 32]).unwrap(),
        Token::<32>::new(&second).unwrap()
    );
    assert_ne!(
        Token::<32>::new(&[1; 31]).unwrap(),
        Token::<32>::new(&[1; 32]).unwrap()
    );
    assert_eq!(Token::<32>::default().len(), 0);
    assert!(Token::<0>::EMPTY.is_empty());
    assert!(Token::<0>::new(&[1]).is_err());
    // Legacy API remains explicitly compact; opting into this standalone codec
    // does not silently opt the App into larger retained tokens.
    assert_eq!(
        message::decode(&[0x49, 1, 0, 0]),
        Err(ParseError::BadTokenLength)
    );
}

#[test]
fn selected_owned_representation_uses_normal_message_and_option_api() {
    let token = Token::<32>::new(&[0x77; 32]).unwrap();
    let options = [Opt::new(OptionNumber::new(11), b"resource")];
    let original = message::Message::con(Code::GET, MessageId::new(23), token)
        .with_options(&options)
        .with_payload(b"body");
    let mut wire = [0; 80];
    let len = original.encode(&mut wire).unwrap();
    let parsed = message::decode_with_token::<Token<32>>(&wire[..len]).unwrap();
    assert_eq!(parsed.token(), token);
    assert_eq!(parsed.uri_path().next().unwrap().unwrap(), "resource");
    assert_eq!(parsed.payload(), b"body");
    assert_eq!(parsed.check_rfc7252_formats(), Ok(()));
    assert_eq!(parsed.check_rfc7252_options(), Ok(()));
    let mut copy = [0; 80];
    assert_eq!(parsed.encode(&mut copy), Ok(len));
    assert_eq!(&copy[..len], &wire[..len]);
    let refused = message::decode_with_token::<message::Token>(&wire[..len]);
    assert_eq!(
        refused,
        Err(message::TokenDecodeError::Capacity(CapacityError {
            required: 32,
            available: 8,
        }))
    );
    assert_eq!(
        message::decode_with_token::<Token<32>>(&[0x4f, 1, 0, 0]),
        Err(message::TokenDecodeError::Malformed(
            ParseError::BadTokenLength
        ))
    );
}

#[test]
fn changing_owned_token_representation_preserves_other_message_fields() {
    let base = message::Message::new(Type::NonConfirmable, Code::CONTENT, MessageId::new(7))
        .with_payload(b"answer");
    let extended = base.with_token(Token::<64>::new(&[3; 64]).unwrap());
    assert_eq!(extended.payload(), b"answer");
    assert_eq!(extended.message_id(), MessageId::new(7));
    let compact = extended.with_token(message::Token::new(&[9]).unwrap());
    assert_eq!(compact.token().as_bytes(), &[9]);
    assert_eq!(compact.payload(), b"answer");
}
