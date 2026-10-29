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
    let contexts = || {
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
        let request = crate::oscore::RequestRef::from_kid(
            &[2],
            crate::oscore::PartialIv::from_seq(0).unwrap(),
        )
        .unwrap();
        (store, Some(request))
    };
    notification_cases::<128>(contexts);
    notification_cases::<32>(contexts);
    notification_cases::<16>(contexts);
    notification_cases::<1>(contexts);
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
