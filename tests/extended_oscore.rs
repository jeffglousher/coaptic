#![cfg(feature = "oscore")]

use coaptic::message::{Code, Message, MessageId, Opt, Type, decode_with_token, extended};
use coaptic::oscore::{BoundedSecurityContext, DeriveParams, Error};

type Token = extended::Token<300>;
type Context = BoundedSecurityContext<2, Token>;

fn context(server: bool) -> Context {
    Context::derive(DeriveParams {
        master_secret: &[0x17; 16],
        master_salt: &[0x31; 8],
        sender_id: if server { &[2] } else { &[1] },
        recipient_id: if server { &[1] } else { &[2] },
        id_context: b"extended",
    })
    .unwrap()
}

#[test]
fn protected_extended_tokens_round_trip_across_length_encodings() {
    for length in [9, 13, 269] {
        let mut client = context(false);
        let mut server = context(true);
        let token = Token::new(&[0xa7; 300][..length]).unwrap();
        let options = [Opt::uri_host("sensor.example"), Opt::uri_path("reading")];
        let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(8))
            .with_token(token)
            .with_options(&options);
        let mut wire = [0; 1024];
        let mut plain = [0; 1024];
        let size = client.protect_request(&request, &mut wire).unwrap();
        let outer = decode_with_token::<Token>(&wire[..size]).unwrap();
        assert_eq!(outer.token(), token);
        let (opened, binding) = server.unprotect_request(&outer, &mut plain).unwrap();
        assert_eq!(opened.token(), token);
        assert_eq!(opened.uri_path().next(), Some(Ok("reading")));
        assert_eq!(opened.uri_host(), Some(Ok("sensor.example")));
        let response = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(8))
            .with_token(token)
            .with_payload(b"value");
        let size = server
            .protect_response(&response, binding, &mut wire)
            .unwrap();
        let outer = decode_with_token::<Token>(&wire[..size]).unwrap();
        let opened = client
            .unprotect_response(&outer, client.lookup(token).unwrap(), &mut plain)
            .unwrap();
        assert_eq!(opened.token(), token);
        assert_eq!(opened.payload(), b"value");
    }
}

#[test]
fn security_binding_compares_full_token_and_recovers_after_capacity_release() {
    let mut client = context(false);
    let mut bytes = [1; 32];
    let first = Token::new(&bytes).unwrap();
    bytes[31] = 2;
    let second = Token::new(&bytes).unwrap();
    bytes[31] = 3;
    let third = Token::new(&bytes).unwrap();
    let mut wire = [0; 256];
    for token in [first, second] {
        let request =
            Message::new(Type::Confirmable, Code::GET, MessageId::new(9)).with_token(token);
        client.protect_request(&request, &mut wire).unwrap();
    }
    assert_ne!(client.lookup(first), client.lookup(second));
    assert!(client.lookup(third).is_none());
    let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(10)).with_token(third);
    assert_eq!(
        client.protect_request(&request, &mut wire),
        Err(Error::Saturated)
    );
    assert!(client.take(first).is_some());
    client.protect_request(&request, &mut wire).unwrap();
    assert!(client.lookup(first).is_none());
    assert!(client.lookup(second).is_some());
    assert!(client.lookup(third).is_some());
}
