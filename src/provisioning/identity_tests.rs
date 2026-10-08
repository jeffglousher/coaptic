extern crate std;

use super::{Identity, PinnedPeer};
use crate::provisioning::Error;

fn hex<const N: usize>(value: &str) -> [u8; N] {
    assert_eq!(value.len(), N * 2);
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    bytes
}

fn scalar_one() -> [u8; 32] {
    let mut scalar = [0; 32];
    scalar[31] = 1;
    scalar
}

fn generator() -> [u8; 65] {
    hex(concat!(
        "04",
        "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
        "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"
    ))
}

fn expected_credential() -> [u8; 82] {
    hex(concat!(
        "a108a101a5010202412a2001215820",
        "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
        "225820",
        "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"
    ))
}

#[test]
fn invalid_private_scalars_are_rejected() {
    let order = hex("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");
    for scalar in [[0; 32], [0xff; 32], order] {
        assert!(matches!(
            Identity::from_private_key(scalar, 0x2a),
            Err(Error::InvalidKey)
        ));
    }
}

#[test]
fn invalid_public_points_are_rejected() {
    let mut zero_point = [0; 65];
    zero_point[0] = 4;
    let mut oversized_coordinates = [0xff; 65];
    oversized_coordinates[0] = 4;
    for bytes in [
        &[][..],
        &[0][..],
        &[4, 1, 2][..],
        &zero_point[..],
        &oversized_coordinates[..],
    ] {
        assert!(matches!(
            PinnedPeer::from_public_key(bytes, 0x2a),
            Err(Error::InvalidKey)
        ));
    }
}

#[test]
fn private_identity_and_sec1_import_share_the_canonical_credential() {
    let identity = Identity::from_private_key(scalar_one(), 0x2a).unwrap();
    let peer = PinnedPeer::from_public_key(&generator(), 0x2a).unwrap();
    let credential = identity.credential();
    assert_eq!(credential.bytes.as_slice(), expected_credential());
    assert_eq!(peer.credential(), credential);
    assert_eq!(credential.kid.unwrap().as_slice(), &[0x2a]);
    assert_eq!(credential.public_key(), Some(identity.public_x()));
    assert_eq!(identity.scalar(), scalar_one());
    assert_eq!(identity.peer().principal(), peer.principal());
}

#[test]
fn compressed_sec1_normalizes_to_the_same_principal() {
    let point = generator();
    let mut compressed = [0; 33];
    compressed[0] = 3;
    compressed[1..].copy_from_slice(&point[1..33]);
    let compressed = PinnedPeer::from_public_key(&compressed, 0x2a).unwrap();
    let uncompressed = PinnedPeer::from_public_key(&point, 0x2a).unwrap();
    assert_eq!(compressed.credential(), uncompressed.credential());
    assert_eq!(compressed.principal(), uncompressed.principal());
    assert_eq!(compressed.principal().id(), uncompressed.principal().id());
}

#[test]
fn principal_vector_is_stable_for_local_and_pinned_credentials() {
    let expected = hex::<32>("1c92384384d12f5661686d8a142a25a6d07473938551abe1044ff042a7b81d22");
    let identity = Identity::from_private_key(scalar_one(), 0x2a).unwrap();
    let pin = PinnedPeer::from_public_key(&generator(), 0x2a).unwrap();
    for principal in [identity.peer().principal(), pin.principal()] {
        assert_eq!(principal.fingerprint(), &expected);
        let mut text = [0; 36];
        assert_eq!(
            principal.id().hyphenated().encode_lower(&mut text),
            "1c923843-84d1-8f56-a168-6d8a142a25a6"
        );
    }
}

#[test]
fn opposite_y_key_alias_keeps_the_x_coordinate_for_self_pin_rejection() {
    let identity = Identity::from_private_key(scalar_one(), 0x2a).unwrap();
    let opposite = Identity::from_private_key(
        hex("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632550"),
        0x2b,
    )
    .unwrap();
    assert_eq!(identity.public_x(), opposite.public_x());
    assert_ne!(identity.peer().principal(), opposite.peer().principal());
}

#[test]
fn credential_identifier_and_public_key_changes_change_the_full_principal() {
    let identity = Identity::from_private_key(scalar_one(), 0x2a).unwrap();
    let another_kid = PinnedPeer::from_public_key(&generator(), 0x2b).unwrap();
    let mut second_scalar = scalar_one();
    second_scalar[31] = 2;
    let another_key = Identity::from_private_key(second_scalar, 0x2a).unwrap();
    assert_ne!(identity.peer().principal(), another_kid.principal());
    assert_eq!(identity.public_x(), another_kid.public_x());
    assert_ne!(identity.peer().principal(), another_key.peer().principal());
    assert_ne!(identity.public_x(), another_key.public_x());
}

#[test]
fn exported_credential_copy_cannot_mutate_the_pin() {
    let peer = PinnedPeer::from_public_key(&generator(), 0x2a).unwrap();
    let principal = peer.principal();
    let mut exported = peer.credential();
    exported.bytes.content.fill(0);
    assert_eq!(peer.credential().bytes.as_slice(), expected_credential());
    assert_eq!(peer.principal(), principal);
}

#[test]
fn uuid_label_has_custom_version_and_rfc_variant_without_changing_other_bits() {
    let principal = PinnedPeer::from_public_key(&generator(), 0x2a)
        .unwrap()
        .principal();
    let id = principal.id();
    assert_eq!(id.get_version(), Some(uuid::Version::Custom));
    assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
    for (index, byte) in id.as_bytes().iter().enumerate() {
        let fingerprint = principal.fingerprint()[index];
        let expected = match index {
            6 => (fingerprint & 0x0f) | 0x80,
            8 => (fingerprint & 0x3f) | 0x80,
            _ => fingerprint,
        };
        assert_eq!(*byte, expected);
    }
    let mut text = [0; 36];
    assert_eq!(id.hyphenated().encode_lower(&mut text).len(), 36);
}

#[test]
fn identity_debug_is_redacted() {
    let scalar = [0x11; 32];
    let identity = Identity::from_private_key(scalar, 0x2a).unwrap();
    let output = std::format!("{identity:?}");
    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains(&std::format!("{scalar:?}")));
    assert!(!output.contains("1111111111111111111111111111111111111111111111111111111111111111"));
}
