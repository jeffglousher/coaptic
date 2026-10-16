extern crate std;

use super::*;
use std::vec::Vec;

fn identity(scalar: u8, kid: u8) -> Identity {
    let mut private = [0; 32];
    private[31] = scalar;
    Identity::from_private_key(private, kid).unwrap()
}

fn entropy(scalar: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |bytes| {
        bytes.fill(0);
        bytes[31] = scalar;
        true
    }
}

fn field(label: i32) -> Ead {
    let mut field = Ead::new();
    field.push(0, None).unwrap();
    field.push(label, Some(b"opaque")).unwrap();
    field.push(0, Some(b"padding")).unwrap();
    field
}

fn exchange(
    suite: CipherSuite,
    fields: &[Ead; 4],
    registry: bool,
    handled: bool,
) -> (Session, Session, Vec<i32>) {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut seen = Vec::new();
    let (initiator, m1) = Initiator::start_with_suite_and_ead(
        &a,
        b.peer(),
        ConnectionId::INITIATOR,
        suite,
        &fields[0],
        entropy(3),
    )
    .unwrap();
    let mut first = |item: EadItem<'_>| {
        assert_ne!(item.label, 0, "padding must be discarded");
        assert_eq!(item.value, Some(&b"opaque"[..]));
        seen.push(item.label);
        handled
    };
    let (pinned, lookup, m2) = if registry {
        let (state, m2) = RegistryResponder::receive_message_1_with_ead(
            &b,
            ConnectionId::RESPONDER,
            &m1,
            &fields[1],
            &mut first,
            entropy(4),
        )
        .unwrap();
        (None, Some(state), m2)
    } else {
        let (state, m2) = Responder::receive_message_1_with_ead(
            &b,
            a.peer(),
            ConnectionId::RESPONDER,
            &m1,
            &fields[1],
            &mut first,
            entropy(4),
        )
        .unwrap();
        (Some(state), None, m2)
    };
    let (initiator, m3) = initiator
        .receive_message_2_with_ead(
            &m2,
            &fields[2],
            |principal, item| {
                assert_eq!(*principal, b.peer().principal());
                assert_ne!(item.label, 0);
                assert_eq!(item.value, Some(&b"opaque"[..]));
                seen.push(item.label);
                handled
            },
            |_| true,
        )
        .unwrap();
    let mut third = |principal: &Principal, item: EadItem<'_>| {
        assert_eq!(*principal, a.peer().principal());
        assert_ne!(item.label, 0);
        assert_eq!(item.value, Some(&b"opaque"[..]));
        seen.push(item.label);
        handled
    };
    let (server, m4) = match pinned {
        Some(state) => state.receive_message_3_with_ead(&m3, &fields[3], &mut third, |_| true),
        None => lookup.unwrap().receive_message_3_with_ead(
            &m3,
            |_| Some(a.peer()),
            &fields[3],
            &mut third,
            |_| true,
        ),
    }
    .unwrap();
    let client = initiator
        .receive_message_4_with_ead(
            &m4,
            |principal, item| {
                assert_eq!(*principal, b.peer().principal());
                assert_ne!(item.label, 0);
                assert_eq!(item.value, Some(&b"opaque"[..]));
                seen.push(item.label);
                handled
            },
            |_| true,
        )
        .unwrap();
    (client, server, seen)
}

#[test]
fn ead_registered_label_widths_and_optional_values_round_trip_exactly() {
    for (label, encoded) in [
        (0, &[0][..]),
        (23, &[23]),
        (24, &[0x18, 24]),
        (255, &[0x18, 255]),
        (256, &[0x19, 1, 0]),
        (65535, &[0x19, 255, 255]),
        (-1, &[0x20]),
        (-24, &[0x37]),
        (-25, &[0x38, 24]),
        (-256, &[0x38, 255]),
        (-257, &[0x39, 1, 0]),
        (-65535, &[0x39, 255, 254]),
    ] {
        for value in [None, Some(&[][..]), Some(&b"opaque"[..])] {
            let mut field = Ead::new();
            field.push(label, value).unwrap();
            assert!(field.as_encoded().starts_with(encoded));
            assert_eq!(Ead::from_encoded(field.as_encoded()).unwrap(), field);
            if label != 0 {
                let mut count = 0;
                Ead::process(&Some(field), |item| {
                    assert_eq!(item, EadItem { label, value });
                    count += 1;
                    true
                })
                .unwrap();
                assert_eq!(count, 1);
            }
        }
    }
}

#[test]
fn ead_malformed_nonpreferred_and_oversized_input_refuses_without_panics() {
    for encoded in [
        &[0x18][..],
        &[0x19, 1],
        &[0x18, 1],
        &[0x19, 0, 24],
        &[0x38, 0],
        &[0, 0x58, 0],
        &[0, 0x59, 0, 24],
        &[0, 0x41],
        &[0, 0x42, 1],
        &[0, 0x5f, 0xff],
        &[0, 0x60],
        &[0, 0xf6],
        &[0, 0xa0],
        &[0, 0x80],
        &[0, 0x40, 0x40],
        &[0x39, 255, 255],
        &[0x1a, 0, 1, 0, 0],
    ] {
        assert!(Ead::from_encoded(encoded).is_err(), "{encoded:x?}");
    }
    let mut full = Ead::from_encoded(&[0; Ead::CAPACITY]).unwrap();
    let saved = full.clone();
    for (label, value) in [
        (0, None),
        (65535, Some(&b"x"[..])),
        (65536, None),
        (i32::MIN, None),
    ] {
        assert!(full.push(label, value).is_err());
        assert_eq!(full, saved);
    }
    assert_eq!(
        Ead::from_encoded(&[0; Ead::CAPACITY + 1]),
        Err(EadError::Capacity)
    );
    let mut value = Ead::new();
    value.push(0, Some(&[0xa5; 61])).unwrap();
    assert_eq!(value.as_encoded().len(), Ead::CAPACITY);
    assert_eq!(Ead::from_encoded(value.as_encoded()).unwrap(), value);
    assert_eq!(
        Ead::new().push(0, Some(&[0xa5; 62])),
        Err(EadError::Capacity)
    );
    for outside in [
        &[0x1a, 0, 1, 0, 0][..],
        &[0x39, 255, 255],
        &[0x1b, 255, 255, 255, 255, 255, 255, 255, 255],
        &[0x3b, 255, 255, 255, 255, 255, 255, 255, 255],
    ] {
        assert_eq!(Ead::from_encoded(outside), Err(EadError::Label));
    }
    assert!(!std::format!("{:?}", field(-24)).contains("opaque"));
    assert!(
        !std::format!(
            "{:?}",
            EadItem {
                label: 24,
                value: Some(b"secret")
            }
        )
        .contains("secret")
    );
}

#[test]
fn ead_multiple_items_padding_and_critical_handlers_authenticate_both_suites_and_paths() {
    for suite in [CipherSuite::Suite2, CipherSuite::Suite3] {
        for registry in [false, true] {
            for (label, handled) in [(65535, false), (-65535, true)] {
                let fields = core::array::from_fn(|_| field(label));
                let (client, server, seen) = exchange(suite, &fields, registry, handled);
                assert_eq!(seen, [label; 4]);
                let (client, _) = client.into_parts();
                let (server, _) = server.into_parts();
                assert_eq!(client.sender_key(), server.recipient_key());
                assert_eq!(client.common_iv(), server.common_iv());
            }
            let fields = core::array::from_fn(|_| Ead::from_encoded(&[0; Ead::CAPACITY]).unwrap());
            let (client, server, seen) = exchange(suite, &fields, registry, false);
            assert!(seen.is_empty());
            let (plain, _, _) = exchange(
                suite,
                &core::array::from_fn(|_| Ead::new()),
                registry,
                false,
            );
            let (client, _) = client.into_parts();
            let (server, _) = server.into_parts();
            let (plain, _) = plain.into_parts();
            assert_eq!(client.sender_key(), server.recipient_key());
            assert_ne!(client.sender_key(), plain.sender_key());
        }
    }
}

#[test]
fn ead_unknown_critical_items_abort_each_stage_and_fresh_exchange_recovers() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let critical = field(-24);
    let empty = Ead::new();
    for suite in [CipherSuite::Suite2, CipherSuite::Suite3] {
        for stage in 1..=4 {
            let (initiator, m1) = Initiator::start_with_suite_and_ead(
                &a,
                b.peer(),
                ConnectionId::INITIATOR,
                suite,
                if stage == 1 { &critical } else { &empty },
                entropy(3),
            )
            .unwrap();
            if stage == 1 {
                assert!(matches!(
                    Responder::receive_message_1(&b, a.peer(), &m1, |_| panic!(
                        "critical EAD before entropy"
                    )),
                    Err(Error::Profile)
                ));
                assert!(matches!(
                    RegistryResponder::receive_message_1(
                        &b,
                        ConnectionId::RESPONDER,
                        &m1,
                        |_| panic!("critical EAD before entropy")
                    ),
                    Err(Error::Profile)
                ));
                continue;
            }
            let (responder, m2) = Responder::receive_message_1_with_ead(
                &b,
                a.peer(),
                ConnectionId::RESPONDER,
                &m1,
                if stage == 2 { &critical } else { &empty },
                |_| false,
                entropy(4),
            )
            .unwrap();
            if stage == 2 {
                assert!(matches!(
                    initiator
                        .receive_message_2(&m2, |_| panic!("critical EAD before authorization")),
                    Err(Error::Profile)
                ));
                continue;
            }
            let (initiator, m3) = initiator
                .receive_message_2_with_ead(
                    &m2,
                    if stage == 3 { &critical } else { &empty },
                    |_, _| false,
                    |_| true,
                )
                .unwrap();
            if stage == 3 {
                assert!(matches!(
                    responder
                        .receive_message_3(&m3, |_| panic!("critical EAD before authorization")),
                    Err(Error::Profile)
                ));
                continue;
            }
            let (_, m4) = responder
                .receive_message_3_with_ead(&m3, &critical, |_, _| false, |_| true)
                .unwrap();
            assert!(matches!(
                initiator.receive_message_4(&m4, |_| panic!("critical EAD before authorization")),
                Err(Error::Profile)
            ));
        }
        let (client, server, _) =
            exchange(suite, &core::array::from_fn(|_| Ead::new()), false, false);
        let (client, _) = client.into_parts();
        let (server, _) = server.into_parts();
        assert_eq!(client.sender_key(), server.recipient_key());
    }
}

#[test]
fn ead_tampering_cannot_invoke_authenticated_handlers() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let data = field(24);
    let corrupt = |message: &Message| {
        let mut bytes = message.as_bytes().to_vec();
        *bytes.last_mut().unwrap() ^= 1;
        Message::from_slice(&bytes).unwrap()
    };
    for suite in [CipherSuite::Suite2, CipherSuite::Suite3] {
        for stage in 1..=4 {
            let (initiator, m1) = Initiator::start_with_suite_and_ead(
                &a,
                b.peer(),
                ConnectionId::INITIATOR,
                suite,
                &data,
                entropy(3),
            )
            .unwrap();
            let m1 = if stage == 1 { corrupt(&m1) } else { m1 };
            let (responder, m2) = Responder::receive_message_1_with_ead(
                &b,
                a.peer(),
                ConnectionId::RESPONDER,
                &m1,
                &data,
                |_| false,
                entropy(4),
            )
            .unwrap();
            if stage <= 2 {
                let m2 = if stage == 2 { corrupt(&m2) } else { m2 };
                assert!(
                    initiator
                        .receive_message_2_with_ead(
                            &m2,
                            &data,
                            |_, _| panic!("unverified EAD"),
                            |_| panic!("unverified peer")
                        )
                        .is_err()
                );
                continue;
            }
            let (initiator, m3) = initiator
                .receive_message_2_with_ead(&m2, &data, |_, _| false, |_| true)
                .unwrap();
            if stage == 3 {
                assert!(
                    responder
                        .receive_message_3_with_ead(
                            &corrupt(&m3),
                            &data,
                            |_, _| panic!("unverified EAD"),
                            |_| panic!("unverified peer")
                        )
                        .is_err()
                );
                continue;
            }
            let (_, m4) = responder
                .receive_message_3_with_ead(&m3, &data, |_, _| false, |_| true)
                .unwrap();
            assert!(
                initiator
                    .receive_message_4_with_ead(
                        &corrupt(&m4),
                        |_, _| panic!("unverified EAD"),
                        |_| panic!("unverified peer")
                    )
                    .is_err()
            );
        }
    }
}
