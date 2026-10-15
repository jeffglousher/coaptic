use super::lakers::{CBORDecoder, ConnId};

const MAX_CONNID_ENCODED_LEN: usize = core::mem::size_of::<ConnId>();

#[test]
fn oversized_connection_identifiers_return_errors() {
    for length in MAX_CONNID_ENCODED_LEN..24 {
        let mut bytes = [0xab; 24];
        bytes[0] = 0x40 | length as u8;
        assert!(ConnId::from_decoder(&mut CBORDecoder::new(&bytes[..length + 1])).is_err());
    }
}

#[test]
fn supported_connection_identifier_lengths_round_trip() {
    for length in 0..MAX_CONNID_ENCODED_LEN {
        let bytes = [0xab; MAX_CONNID_ENCODED_LEN];
        let id = ConnId::from_slice(&bytes[..length]).unwrap();
        let decoded = ConnId::from_decoder(&mut CBORDecoder::new(id.as_cbor())).unwrap();
        assert_eq!(decoded.as_slice(), &bytes[..length]);
        assert_eq!(decoded.as_cbor(), id.as_cbor());
    }
}

#[test]
fn compact_connection_identifiers_round_trip() {
    for byte in (0..24).chain(0x20..0x38) {
        let bytes = [byte];
        let decoded = ConnId::from_decoder(&mut CBORDecoder::new(&bytes)).unwrap();
        assert_eq!(decoded.as_slice(), bytes);
        assert_eq!(decoded.as_cbor(), bytes);
    }
}

#[test]
fn truncated_and_unsupported_connection_identifiers_return_errors() {
    for length in 0..24 {
        let mut bytes = [0xab; 24];
        bytes[0] = 0x40 | length as u8;
        for available in 0..=length {
            assert!(ConnId::from_decoder(&mut CBORDecoder::new(&bytes[..available])).is_err());
        }
    }
    for byte in 0u8..=u8::MAX {
        if byte < 24 || (0x20..0x38).contains(&byte) || (0x40..0x58).contains(&byte) {
            continue;
        }
        assert!(ConnId::from_decoder(&mut CBORDecoder::new(&[byte])).is_err());
    }
}
