//! Wire-equivalence boundaries for notification option scratch.

use super::*;
use crate::message::Token;
use crate::storage::{Memory, profiles};
use core::convert::Infallible;

// Frozen notification construction from 28d9f1a6. The original 24-entry scratch
// is independent of the candidate's four-entry scratch; security encoding is shared.
#[allow(clippy::too_many_arguments)]
fn legacy_notification<S: Storage + DatagramSlots>(
    engine: &mut Engine<S>,
    peer: Endpoint,
    tx: SlotId,
    ty: Type,
    mid: MessageId,
    token: S::Token,
    response: &Response<'_, S::Token>,
    payload: &[u8],
    context: &mut oscore::Field<4, S::Token>,
    request: oscore::Request,
) -> Result<(), Error<Infallible>> {
    response.validate().map_err(Error::Response)?;
    let cf = response.format().map(ContentFormat::encode);
    let max_age = response.max_age_secs().map(EncodedUint::new);
    let observe = response
        .observe_seq()
        .map(|seq| encode_uint(seq & 0x00ff_ffff));
    let mut opts = OptionsBuilder::<{ 8 + 2 * LOCATION_MAX }>::new();
    if let Some(etag) = response.etag_bytes() {
        push_opt(&mut opts, Opt::etag(etag)).unwrap();
    }
    if let Some(ref encoded) = observe {
        push_opt(&mut opts, Opt::observe(encoded)).unwrap();
    }
    if let Some(ref encoded) = cf {
        push_opt(&mut opts, Opt::content_format(encoded)).unwrap();
    }
    if let Some(ref encoded) = max_age {
        push_opt(&mut opts, Opt::max_age(encoded)).unwrap();
    }
    let msg = Message::new(ty, response.code(), mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    oscore::encode_notification(context, peer, request, engine, tx, &msg)
}

fn notification_cases<const BYTES: usize>(
    mut contexts: impl FnMut() -> (oscore::Store<4, 1>, oscore::Request),
) {
    type Base = profiles::Constrained;
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let token = Token::new(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    for mask in 0..16 {
        for boundary in [0, 1, 255, 256, 65535, 65536, 0x00ff_ffff, u32::MAX] {
            let mut response = Response::content(b"body");
            if mask & 1 != 0 {
                response = response.etag(if boundary == 0 { b"a" } else { b"12345678" });
            }
            if mask & 2 != 0 {
                response = response.observe(boundary);
            }
            if mask & 4 != 0 {
                response = response.content_format(ContentFormat::new(boundary as u16));
            }
            if mask & 8 != 0 {
                response = response.max_age(boundary);
            }
            let mut candidate =
                Engine::from_storage(Memory::<profiles::WithTxDatagram<Base, 1, BYTES>>::new());
            let mut legacy =
                Engine::from_storage(Memory::<profiles::WithTxDatagram<Base, 1, BYTES>>::new());
            let tx = candidate.acquire_tx().unwrap();
            let old_tx = legacy.acquire_tx().unwrap();
            let (mut context, request) = contexts();
            let (mut old_context, old_request) = contexts();
            let peer = oscore::resolve::<Infallible, 4, Token>(&context, peer).unwrap();
            let result = encode_notification::<_, Infallible, 4>(
                &mut candidate,
                peer,
                tx,
                Type::NonConfirmable,
                MessageId::new(17),
                token,
                &response,
                response.payload(),
                &mut context,
                request,
            );
            let old_result = legacy_notification(
                &mut legacy,
                peer,
                old_tx,
                Type::NonConfirmable,
                MessageId::new(17),
                token,
                &response,
                response.payload(),
                &mut old_context,
                old_request,
            );
            assert_eq!(
                result, old_result,
                "mask={mask}, boundary={boundary}, bytes={BYTES}"
            );
            assert_eq!(
                candidate.storage().tx_payload(tx),
                legacy.storage().tx_payload(old_tx)
            );
            if BYTES >= 128 {
                assert!(result.is_ok());
                assert_eq!(
                    candidate.decode_tx(tx).unwrap().oscore().is_some(),
                    oscore::is_active(&context)
                );
            }
            if BYTES == 1 {
                assert_eq!(
                    result,
                    Err(Error::Message(SlotMessageError::Encode(
                        EncodeError::BufferTooSmall
                    )))
                );
            }
            // Compare the next notification too: prior refusals must preserve
            // the same sequence ownership and subsequent recovery behavior.
            if result.is_err() {
                let response = Response::content(b"");
                let result = encode_notification::<_, Infallible, 4>(
                    &mut candidate,
                    peer,
                    tx,
                    Type::NonConfirmable,
                    MessageId::new(18),
                    token,
                    &response,
                    b"",
                    &mut context,
                    request,
                );
                let old_result = legacy_notification(
                    &mut legacy,
                    peer,
                    old_tx,
                    Type::NonConfirmable,
                    MessageId::new(18),
                    token,
                    &response,
                    b"",
                    &mut old_context,
                    old_request,
                );
                assert_eq!(result, old_result);
                assert_eq!(
                    candidate.storage().tx_payload(tx),
                    legacy.storage().tx_payload(old_tx)
                );
            }
        }
    }
}

#[test]
fn notification_four_metadata_options_match_legacy_unprotected() {
    let contexts = || (oscore::Store::<4, 1>::new(), oscore::no_request());
    notification_cases::<128>(contexts);
    notification_cases::<32>(contexts);
    notification_cases::<16>(contexts);
    notification_cases::<1>(contexts);
}

#[cfg(feature = "oscore")]
#[test]
fn notification_four_metadata_options_match_legacy_protected() {
    notification_cases::<128>(protected_response_context);
    notification_cases::<32>(protected_response_context);
    notification_cases::<16>(protected_response_context);
    notification_cases::<1>(protected_response_context);
}

#[cfg(feature = "oscore")]
fn protected_response_context() -> (oscore::Store<4, 1>, oscore::Request) {
    let mut store = oscore::Store::<4, 1>::new();
    let mut context = Some(
        crate::oscore::BoundedSecurityContext::derive(crate::oscore::DeriveParams {
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
            &mut context,
        )
        .unwrap();
    let request =
        crate::oscore::RequestRef::from_kid(&[2], crate::oscore::PartialIv::from_seq(0).unwrap())
            .unwrap();
    (store, Some(request))
}

#[test]
fn notification_response_validation_precedes_slot_or_encoding_failure() {
    let mut engine = Engine::from_storage(Memory::<
        profiles::WithTxDatagram<profiles::Constrained, 1, 1>,
    >::new());
    let mut context = oscore::Store::<4, 1>::new();
    // Even an unoccupied, out-of-range slot must not mask the first response error.
    let response = Response::content(b"body").etag(&[]).location_path(".");
    assert_eq!(
        encode_notification::<_, Infallible, 4>(
            &mut engine,
            Endpoint::v4([192, 0, 2, 1], 5683),
            SlotId::from_index(7),
            Type::NonConfirmable,
            MessageId::new(1),
            Token::EMPTY,
            &response,
            b"body",
            &mut context,
            oscore::no_request(),
        ),
        Err(Error::Response(ResponseError::EtagLength)),
    );
}

// Force the original general builder as an oracle; do not let both sides choose
// the specialization. The caller-provided payload intentionally differs from
// Response's payload, as it can after App selects a representation range.
#[allow(clippy::too_many_arguments)]
fn response_encoding_oracle<const BYTES: usize>(
    contexts: impl Fn() -> (oscore::Store<4, 1>, oscore::Request),
    response: &Response<'_, Token>,
    ty: Type,
    block: Option<BlockOpt>,
    block1: Option<BlockOpt>,
    occupied: bool,
) -> Result<(), Error<Infallible>> {
    type Base = profiles::Constrained;
    let mut candidate =
        Engine::from_storage(Memory::<profiles::WithTxDatagram<Base, 1, BYTES>>::new());
    let mut general =
        Engine::from_storage(Memory::<profiles::WithTxDatagram<Base, 1, BYTES>>::new());
    let tx = candidate.acquire_tx().unwrap();
    let old_tx = general.acquire_tx().unwrap();
    if occupied {
        for engine in [&mut candidate, &mut general] {
            let mut access = engine.access_tx_mut(tx).unwrap();
            access.bytes_mut()[0] = 0xa5;
            access.set_len(1).unwrap();
        }
    }
    let (mut context, request) = contexts();
    let (mut old_context, old_request) = contexts();
    let peer =
        oscore::resolve::<Infallible, 4, Token>(&context, Endpoint::v4([192, 0, 2, 1], 5683))
            .unwrap();
    let token = Token::new(b"12345678").unwrap();
    let result = encode_response::<_, Infallible, 4>(
        &mut candidate,
        peer,
        tx,
        ty,
        MessageId::new(0x1234),
        token,
        response,
        b"selected",
        block,
        block1,
        &mut context,
        request,
    );
    let old_result = encode_response_options::<_, Infallible, 4, 24>(
        &mut general,
        peer,
        old_tx,
        ty,
        MessageId::new(0x1234),
        token,
        response,
        b"selected",
        block,
        block1,
        &mut old_context,
        old_request,
    );
    assert_eq!(
        result, old_result,
        "capacity={BYTES}, occupied={occupied}, response={response:?}"
    );
    assert_eq!(
        candidate.storage().tx_payload(tx),
        general.storage().tx_payload(old_tx)
    );
    assert!(!candidate.tx_is_pinned(tx));
    assert!(!general.tx_is_pinned(old_tx));
    if result.is_ok() && !oscore::is_active(&context) {
        assert_eq!(candidate.decode_tx(tx).unwrap().payload(), b"selected");
    }
    // A subsequent fresh-PIV notification observes sequence/binding state left
    // by either success or refusal, including replacement of a retained packet.
    let probe = Response::content(b"").observe(0);
    let next = encode_notification::<_, Infallible, 4>(
        &mut candidate,
        peer,
        tx,
        Type::NonConfirmable,
        MessageId::new(0x1235),
        token,
        &probe,
        b"",
        &mut context,
        request,
    );
    let old_next = encode_notification::<_, Infallible, 4>(
        &mut general,
        peer,
        old_tx,
        Type::NonConfirmable,
        MessageId::new(0x1235),
        token,
        &probe,
        b"",
        &mut old_context,
        old_request,
    );
    assert_eq!(next, old_next);
    assert_eq!(
        candidate.storage().tx_payload(tx),
        general.storage().tx_payload(old_tx)
    );
    assert!(!candidate.tx_is_pinned(tx));
    assert!(!general.tx_is_pinned(old_tx));
    result
}

fn ordinary_response_cases(contexts: impl Copy + Fn() -> (oscore::Store<4, 1>, oscore::Request)) {
    for mask in 0..4 {
        // Zero has an empty encoded value but must retain the Content-Format
        // option. The other values exercise both integer-width boundaries.
        for format in [0, 255, 256, 65535] {
            let mut response = Response::content(b"original");
            if mask & 1 != 0 {
                response = response.etag(b"12345678");
            }
            if mask & 2 != 0 {
                response = response.content_format(ContentFormat::new(format));
            }
            for ty in [Type::Acknowledgement, Type::NonConfirmable] {
                assert!(
                    response_encoding_oracle::<128>(contexts, &response, ty, None, None, false)
                        .is_ok()
                );
            }
        }
    }
    let base = Response::content(b"original")
        .etag(b"fixture")
        .content_format(ContentFormat::OCTET_STREAM);
    for occupied in [false, true] {
        assert!(
            response_encoding_oracle::<16>(
                contexts,
                &base,
                Type::Acknowledgement,
                None,
                None,
                occupied
            )
            .is_err()
        );
        assert!(
            response_encoding_oracle::<128>(
                contexts,
                &base,
                Type::Acknowledgement,
                None,
                None,
                occupied
            )
            .is_ok()
        );
    }
    // Each excluded owner independently forces the general route, including
    // present-but-zero/empty values that must not be mistaken for absence.
    for response in [
        base.max_age(0),
        base.observe(0),
        base.echo(crate::message::Echo::new(b"echo").unwrap()),
        base.location_path(""),
        base.location_query(""),
        base.with_fresh_piv(),
    ] {
        assert!(
            response_encoding_oracle::<128>(
                contexts,
                &response,
                Type::NonConfirmable,
                None,
                None,
                false
            )
            .is_ok()
        );
    }
    for q_block in [false, true] {
        let block = BlockOpt {
            value: BlockValue::new(0, true, 0).unwrap(),
            q_block,
            size2: Some(1234),
        };
        assert!(
            response_encoding_oracle::<128>(
                contexts,
                &base,
                Type::Acknowledgement,
                Some(block),
                None,
                false
            )
            .is_ok()
        );
        assert!(
            response_encoding_oracle::<128>(
                contexts,
                &base,
                Type::Acknowledgement,
                None,
                Some(block),
                false
            )
            .is_ok()
        );
    }
    for response in [
        Response::content(b"original").etag(&[]),
        Response::content(b"original").location_path("."),
        Response::content_copy(&[0; INLINE_PAYLOAD + 1]),
    ] {
        let expected = response.validate().unwrap_err();
        assert_eq!(
            response_encoding_oracle::<1>(
                contexts,
                &response,
                Type::Acknowledgement,
                None,
                None,
                false
            ),
            Err(Error::Response(expected))
        );
    }
}

#[test]
fn ordinary_response_direct_options_match_general_unprotected() {
    ordinary_response_cases(|| (oscore::Store::<4, 1>::new(), oscore::no_request()));
}

#[cfg(feature = "oscore")]
#[test]
fn ordinary_response_direct_options_match_general_protected() {
    ordinary_response_cases(protected_response_context);
    let contexts = || {
        let (store, _) = protected_response_context();
        (store, oscore::no_request())
    };
    for occupied in [false, true] {
        assert_eq!(
            response_encoding_oracle::<128>(
                contexts,
                &Response::content(b"original"),
                Type::Acknowledgement,
                None,
                None,
                occupied
            ),
            Err(Error::Oscore(crate::oscore::Error::Context)),
        );
    }
}
