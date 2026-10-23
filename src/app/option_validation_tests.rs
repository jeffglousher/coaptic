//! Bounded equivalence checks for the request option admission predicate.

use super::{oscore, validate_request_options};
use crate::message::{
    Code, Message, MessageId, Opt, OptionNumber, OptionsBuilder, ParsedMessage, Token, Type,
    decode, encode,
};

// Frozen predicate from ac39f907: keep this independent of the one-pass state.
fn legacy_bad_option_request(parsed: &ParsedMessage<'_>, oscore: &oscore::Field<4>) -> bool {
    let mut tags = parsed.request_tag();
    let first = tags.next();
    let bad_tag = first.is_some_and(|tag| tag.len() > 8) || tags.next().is_some();
    bad_tag
        || (parsed.code() == Code::FETCH
            && (matches!(parsed.content_format(), Some(Err(_)))
                || parsed
                    .get_options(crate::message::OptionNumber::CONTENT_FORMAT)
                    .nth(1)
                    .is_some()))
        || parsed.unknown_critical().is_some()
        || (parsed.oscore().is_some() && !oscore::is_active(oscore))
        || matches!(parsed.block2(), Some(Err(_)))
        || parsed
            .q_block2()
            .any(|value| value.map_or(true, |block| block.is_bert()))
        || ((parsed.block1().is_some() || parsed.block2().is_some())
            && (parsed.q_block1().is_some() || parsed.q_block2().next().is_some()))
        || parsed
            .get_options(crate::message::OptionNumber::Q_BLOCK1)
            .nth(1)
            .is_some()
        || (parsed.q_block2().nth(1).is_some()
            && (parsed.observe().is_some()
                || parsed.q_block2().any(|q| {
                    q.is_ok_and(|q| {
                        q.more()
                            && q.num() % u32::from(crate::storage::BlockTransfer::MAX_PAYLOADS) == 0
                    })
                })))
        || matches!(parsed.block1(), Some(Err(_)))
}

fn compare_case(code: Code, extra: &[Opt<'_>], oscore: &oscore::Field<4>) {
    let mut options = OptionsBuilder::<5>::new();
    options.push(Opt::uri_path("bench")).unwrap();
    for option in extra {
        options.push(*option).unwrap();
    }
    let message = Message::new(Type::Confirmable, code, MessageId::new(1))
        .with_token(Token::new(&[1]).unwrap())
        .with_options(options.as_slice());
    let mut wire = [0; 128];
    let n = encode(&message, &mut wire).unwrap();
    let parsed = decode(&wire[..n]).unwrap();
    let validated = validate_request_options(&parsed, oscore);
    assert_eq!(
        validated.is_none(),
        legacy_bad_option_request(&parsed, oscore),
        "{code:?}, {extra:?}, security active={}",
        oscore::is_active(oscore)
    );
    if let Some(validated) = validated {
        assert_eq!(validated.block1, parsed.block1().and_then(Result::ok));
        assert_eq!(validated.block2, parsed.block2().and_then(Result::ok));
        assert_eq!(
            validated.q_block2,
            parsed.q_block2().next().and_then(Result::ok)
        );
        // Frozen accessor-based 4.00 checks, independent of collected state.
        let missing_q1_metadata = parsed.q_block1().is_some()
            && (parsed.request_tag().next().is_none() || !matches!(parsed.size1(), Some(Ok(_))));
        let missing_fetch_format = code == Code::FETCH && parsed.content_format().is_none();
        let mut previous: Option<crate::message::BlockValue> = None;
        let valid_q_selections = parsed.q_block2().all(|value| {
            let Ok(value) = value else {
                return false;
            };
            if previous.is_some_and(|last| value.num() <= last.num() || value.szx() != last.szx()) {
                return false;
            }
            previous = Some(value);
            true
        });
        assert_eq!(
            validated.bad_request,
            missing_q1_metadata || missing_fetch_format || !valid_q_selections,
            "{code:?}, {extra:?}"
        );
    }
}

fn compare_cases(oscore: &oscore::Field<4>) {
    // Each value is syntactically decodable wire data. Include formats that
    // this predicate intentionally ignores, as well as values it must reject.
    let options = [
        Opt::new(OptionNumber::IF_MATCH, &[]),
        Opt::new(OptionNumber::new(65001), &[]),
        Opt::new(OptionNumber::new(65000), &[]),
        Opt::uri_path("second"),
        Opt::new(OptionNumber::OBSERVE, &[]),
        Opt::new(OptionNumber::OBSERVE, &[0; 4]),
        Opt::new(OptionNumber::OSCORE, &[]),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[]),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[0xff; 2]),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[1; 3]),
        Opt::new(OptionNumber::BLOCK1, &[]),
        Opt::new(OptionNumber::BLOCK1, &[1; 4]),
        Opt::new(OptionNumber::BLOCK1, &[7]),
        Opt::new(OptionNumber::BLOCK2, &[]),
        Opt::new(OptionNumber::BLOCK2, &[1; 4]),
        Opt::new(OptionNumber::BLOCK2, &[7]),
        Opt::new(OptionNumber::Q_BLOCK1, &[]),
        Opt::new(OptionNumber::Q_BLOCK1, &[1; 4]),
        Opt::new(OptionNumber::Q_BLOCK2, &[]),
        Opt::new(OptionNumber::Q_BLOCK2, &[0x10]),
        Opt::new(OptionNumber::Q_BLOCK2, &[1; 4]),
        Opt::new(OptionNumber::Q_BLOCK2, &[7]),
        Opt::new(OptionNumber::Q_BLOCK2, &[0x08]),
        Opt::new(OptionNumber::Q_BLOCK2, &[0x18]),
        Opt::new(OptionNumber::SIZE1, &[]),
        Opt::new(OptionNumber::SIZE1, &[0xff; 4]),
        Opt::new(OptionNumber::SIZE1, &[1; 5]),
        Opt::new(OptionNumber::REQUEST_TAG, &[]),
        Opt::new(OptionNumber::REQUEST_TAG, b"12345678"),
        Opt::new(OptionNumber::REQUEST_TAG, b"123456789"),
        Opt::uri_query("a=1"),
    ];
    for code in [Code::GET, Code::FETCH] {
        // Includes the exact fixture's sole Uri-Path and the missing FETCH
        // format, whose later 4.00 must not become this predicate's 4.02.
        compare_case(code, &[], oscore);
        for first in options {
            compare_case(code, &[first], oscore);
            for second in options {
                // Both orders preserve first-value semantics for Block1/2.
                compare_case(code, &[first, second], oscore);
            }
        }
        for first in [
            Opt::new(OptionNumber::Q_BLOCK2, &[]),
            Opt::new(OptionNumber::Q_BLOCK2, &[0x08]),
            Opt::new(OptionNumber::Q_BLOCK2, &[0x18]),
        ] {
            for third in [
                Opt::new(OptionNumber::OBSERVE, &[]),
                Opt::new(OptionNumber::OBSERVE, &[1; 4]),
                Opt::uri_query("a=1"),
            ] {
                compare_case(
                    code,
                    &[first, Opt::new(OptionNumber::Q_BLOCK2, &[0x10]), third],
                    oscore,
                );
            }
        }
        for q1 in [&[][..], &[1; 4], &[7]] {
            for first in [&[][..], &[0xff; 4], &[1; 5]] {
                for second in [&[][..], &[1; 5]] {
                    // Empty tags count as present. Size1 must use its first
                    // value even when a later duplicate has a different shape.
                    compare_case(
                        code,
                        &[
                            Opt::new(OptionNumber::Q_BLOCK1, q1),
                            Opt::new(OptionNumber::SIZE1, first),
                            Opt::new(OptionNumber::SIZE1, second),
                            Opt::request_tag(&[]),
                        ],
                        oscore,
                    );
                }
            }
        }
    }
}

#[test]
fn one_pass_matches_legacy_without_security_context() {
    compare_cases(&oscore::Store::<4, 1>::new());
}

#[cfg(feature = "oscore")]
#[test]
fn one_pass_matches_legacy_with_context_and_retained_security_policy() {
    use crate::oscore::{BoundedSecurityContext, DeriveParams};
    use crate::storage::Endpoint;

    let mut store = oscore::Store::<4, 1>::new();
    let mut candidate = Some(
        BoundedSecurityContext::derive(DeriveParams {
            master_secret: &[0x11; 16],
            master_salt: &[],
            sender_id: &[1],
            recipient_id: &[2],
            id_context: &[],
        })
        .unwrap(),
    );
    store
        .install(
            Endpoint::v4([192, 0, 2, 1], 5683),
            [0x22; 32],
            1,
            &mut candidate,
        )
        .unwrap();
    assert!(oscore::is_active(&store));
    compare_cases(&store);

    // Removing the context must not turn retained security policy into first use.
    store.rows[0] = None;
    assert!(oscore::is_active(&store));
    compare_cases(&store);
}
