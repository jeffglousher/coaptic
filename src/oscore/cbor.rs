//! Minimal CBOR for OSCORE HKDF `info` and COSE AAD. Not a general decoder.

use super::Error;
use super::{AEAD_AES_CCM_16_64_128, OSCORE_VERSION};

/// `Enc_structure` prefix: `["Encrypt0", h'']` (11 bytes) before `external_aad`.
const ENC0_PREFIX: &[u8] = &[
    0x83, 0x68, b'E', b'n', b'c', b'r', b'y', b'p', b't', b'0', 0x40,
];

pub(crate) fn encode_info(
    out: &mut [u8],
    id: &[u8],
    id_context: Option<&[u8]>,
    label: &str,
    l: u8,
) -> Result<usize, Error> {
    let mut i = 0;
    push(out, &mut i, 0x85)?;
    put_bstr(out, &mut i, id)?;
    match id_context {
        None => push(out, &mut i, 0xf6)?,
        Some(ctx) => put_bstr(out, &mut i, ctx)?,
    }
    put_uint(out, &mut i, AEAD_AES_CCM_16_64_128 as u8)?;
    put_tstr(out, &mut i, label)?;
    put_uint(out, &mut i, l)?;
    Ok(i)
}

pub(crate) fn encode_aad(
    out: &mut [u8],
    request_kid: &[u8],
    request_piv: &[u8],
    class_i: &[u8],
) -> Result<usize, Error> {
    // Size fields in their original order before touching out. An oversized
    // earlier field must still refuse before a later field's invalid length.
    let array_len =
        4 + usize::from(OSCORE_VERSION > 23) + usize::from(AEAD_AES_CCM_16_64_128 as u8 > 23);
    let array_len = aad_bstr_end(array_len, request_kid.len())?;
    let array_len = aad_bstr_end(array_len, request_piv.len())?;
    let array_len = aad_bstr_end(array_len, class_i.len())?;

    if out.len() < ENC0_PREFIX.len() {
        return Err(Error::BufferTooSmall);
    }
    out[..ENC0_PREFIX.len()].copy_from_slice(ENC0_PREFIX);
    let mut n = ENC0_PREFIX.len();
    put_len(out, &mut n, 0x40, array_len)?;
    let end = n.checked_add(array_len).ok_or(Error::BufferTooSmall)?;
    if end > out.len() {
        return Err(Error::BufferTooSmall);
    }

    // The old push_slice wrote no external payload on a short output. Only
    // encode directly after its complete destination has passed the same check.
    let payload = &mut out[n..end];
    let mut i = 0;
    push(payload, &mut i, 0x85)?;
    put_uint(payload, &mut i, OSCORE_VERSION)?;
    push(payload, &mut i, 0x81)?;
    put_uint(payload, &mut i, AEAD_AES_CCM_16_64_128 as u8)?;
    put_bstr(payload, &mut i, request_kid)?;
    put_bstr(payload, &mut i, request_piv)?;
    put_bstr(payload, &mut i, class_i)?;
    Ok(end)
}

// Virtual counterpart of one put_bstr into the original 48-byte array. Check
// the length encoding first: put_len returns Id even if its array is full.
fn aad_bstr_end(i: usize, len: usize) -> Result<usize, Error> {
    let header_len = if len <= 23 {
        1
    } else if len <= 255 {
        2
    } else {
        return Err(Error::Id);
    };
    let end = i
        .checked_add(header_len)
        .and_then(|n| n.checked_add(len))
        .ok_or(Error::BufferTooSmall)?;
    if end > 48 {
        return Err(Error::BufferTooSmall);
    }
    Ok(end)
}

fn put_bstr(out: &mut [u8], i: &mut usize, value: &[u8]) -> Result<(), Error> {
    put_len(out, i, 0x40, value.len())?;
    push_slice(out, i, value)
}

fn put_tstr(out: &mut [u8], i: &mut usize, value: &str) -> Result<(), Error> {
    put_len(out, i, 0x60, value.len())?;
    push_slice(out, i, value.as_bytes())
}

fn put_len(out: &mut [u8], i: &mut usize, major: u8, len: usize) -> Result<(), Error> {
    if len <= 23 {
        push(out, i, major | (len as u8))
    } else if len <= 255 {
        push(out, i, major | 24)?;
        push(out, i, len as u8)
    } else {
        Err(Error::Id)
    }
}

fn put_uint(out: &mut [u8], i: &mut usize, n: u8) -> Result<(), Error> {
    if n <= 23 {
        push(out, i, n)
    } else {
        push(out, i, 0x18)?;
        push(out, i, n)
    }
}

fn push(out: &mut [u8], i: &mut usize, b: u8) -> Result<(), Error> {
    if *i >= out.len() {
        return Err(Error::BufferTooSmall);
    }
    out[*i] = b;
    *i += 1;
    Ok(())
}

fn push_slice(out: &mut [u8], i: &mut usize, bytes: &[u8]) -> Result<(), Error> {
    let end = i.checked_add(bytes.len()).ok_or(Error::BufferTooSmall)?;
    if end > out.len() {
        return Err(Error::BufferTooSmall);
    }
    out[*i..end].copy_from_slice(bytes);
    *i = end;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aad_rfc8613_request_and_bound_response_bytes() {
        // Appendix C.4/C.7 use the request kid and Partial IV for both AADs.
        let mut out = [0xa5; 32];
        let expected = [
            0x83, 0x68, 0x45, 0x6e, 0x63, 0x72, 0x79, 0x70, 0x74, 0x30, 0x40, 0x48, 0x85, 0x01,
            0x81, 0x0a, 0x40, 0x41, 0x14, 0x40,
        ];
        assert_eq!(encode_aad(&mut out, &[], &[0x14], &[]), Ok(expected.len()));
        assert_eq!(&out[..expected.len()], expected);
        assert!(out[expected.len()..].iter().all(|byte| *byte == 0xa5));

        // Appendix C.5 uses a one-byte request kid.
        let expected = [
            0x83, 0x68, 0x45, 0x6e, 0x63, 0x72, 0x79, 0x70, 0x74, 0x30, 0x40, 0x49, 0x85, 0x01,
            0x81, 0x0a, 0x41, 0x00, 0x41, 0x14, 0x40,
        ];
        out.fill(0xa5);
        assert_eq!(encode_aad(&mut out, &[0], &[0x14], &[]), Ok(expected.len()));
        assert_eq!(&out[..expected.len()], expected);
        assert!(out[expected.len()..].iter().all(|byte| *byte == 0xa5));
    }

    #[test]
    fn aad_matches_frozen_encoder_at_length_and_output_boundaries() {
        let kid = [0x31; 256];
        let piv = [0x72; 256];
        let class_i = [0xb4; 256];
        // Distinct bytes expose field-order mistakes. Include supported maxima
        // and out-of-profile inputs accepted by this private function.
        let lengths = [
            (0, 0, 0),
            (0, 1, 0),
            (1, 1, 3),
            (7, 5, 0),
            (7, 5, 23),
            (7, 5, 24),
            (7, 5, 28),
            (7, 5, 29),
            (0, 0, 16),
            (0, 0, 17),
            (23, 0, 0),
            (24, 0, 0),
            (0, 23, 0),
            (0, 24, 0),
            (0, 0, 23),
            (0, 0, 24),
            (40, 0, 0),
            (41, 0, 0),
            (42, 0, 0),
            (43, 0, 0),
            (0, 40, 0),
            (0, 41, 0),
            (0, 42, 0),
            (0, 0, 40),
            (0, 0, 41),
            (255, 0, 0),
            (256, 0, 0),
            (0, 255, 0),
            (0, 256, 0),
            (0, 0, 255),
            (0, 0, 256),
            (42, 256, 0),
            (43, 256, 0),
            (41, 0, 256),
            (41, 1, 256),
            (0, 41, 256),
            (0, 42, 256),
            (255, 256, 256),
        ];
        for (kid_len, piv_len, class_len) in lengths {
            // Includes prefix/header truncation, exact/short payload and tail.
            for capacity in 0..=80 {
                let mut expected = [0xa5; 80];
                let mut actual = expected;
                let before = frozen_original::encode_aad(
                    &mut expected[..capacity],
                    &kid[..kid_len],
                    &piv[..piv_len],
                    &class_i[..class_len],
                );
                let after = encode_aad(
                    &mut actual[..capacity],
                    &kid[..kid_len],
                    &piv[..piv_len],
                    &class_i[..class_len],
                );
                assert_eq!(
                    after, before,
                    "result: kid={kid_len} piv={piv_len} class={class_len} capacity={capacity}"
                );
                assert_eq!(
                    actual, expected,
                    "writes: kid={kid_len} piv={piv_len} class={class_len} capacity={capacity}"
                );
            }
        }
    }

    #[test]
    fn hkdf_info_keeps_rfc8613_known_bytes() {
        let cases: [(&[u8], &str, u8, &[u8]); 3] = [
            (
                &[],
                "Key",
                16,
                &[0x85, 0x40, 0xf6, 0x0a, 0x63, b'K', b'e', b'y', 0x10],
            ),
            (
                &[1],
                "Key",
                16,
                &[0x85, 0x41, 0x01, 0xf6, 0x0a, 0x63, b'K', b'e', b'y', 0x10],
            ),
            (
                &[],
                "IV",
                13,
                &[0x85, 0x40, 0xf6, 0x0a, 0x62, b'I', b'V', 0x0d],
            ),
        ];
        for (id, label, len, expected) in cases {
            let mut out = [0xa5; 32];
            assert_eq!(
                encode_info(&mut out, id, None, label, len),
                Ok(expected.len())
            );
            assert_eq!(&out[..expected.len()], expected);
            assert!(out[expected.len()..].iter().all(|byte| *byte == 0xa5));
        }
    }

    // Frozen AAD encoder and its CBOR helpers copied unchanged from 3971a4d.
    // This oracle retains the old staging array and its partial-write behavior.
    mod frozen_original {
        //! Minimal CBOR for OSCORE HKDF `info` and COSE AAD. Not a general decoder.

        use super::Error;
        use super::{AEAD_AES_CCM_16_64_128, OSCORE_VERSION};

        /// `Enc_structure` prefix: `["Encrypt0", h'']` (11 bytes) before `external_aad`.
        const ENC0_PREFIX: &[u8] = &[
            0x83, 0x68, b'E', b'n', b'c', b'r', b'y', b'p', b't', b'0', 0x40,
        ];

        pub(crate) fn encode_aad(
            out: &mut [u8],
            request_kid: &[u8],
            request_piv: &[u8],
            class_i: &[u8],
        ) -> Result<usize, Error> {
            let mut aad_array = [0u8; 48];
            let mut i = 0;
            push(&mut aad_array, &mut i, 0x85)?;
            put_uint(&mut aad_array, &mut i, OSCORE_VERSION)?;
            push(&mut aad_array, &mut i, 0x81)?;
            put_uint(&mut aad_array, &mut i, AEAD_AES_CCM_16_64_128 as u8)?;
            put_bstr(&mut aad_array, &mut i, request_kid)?;
            put_bstr(&mut aad_array, &mut i, request_piv)?;
            put_bstr(&mut aad_array, &mut i, class_i)?;

            if out.len() < ENC0_PREFIX.len() {
                return Err(Error::BufferTooSmall);
            }
            out[..ENC0_PREFIX.len()].copy_from_slice(ENC0_PREFIX);
            let mut n = ENC0_PREFIX.len();
            put_bstr(out, &mut n, &aad_array[..i])?;
            Ok(n)
        }

        fn put_bstr(out: &mut [u8], i: &mut usize, value: &[u8]) -> Result<(), Error> {
            put_len(out, i, 0x40, value.len())?;
            push_slice(out, i, value)
        }

        fn put_len(out: &mut [u8], i: &mut usize, major: u8, len: usize) -> Result<(), Error> {
            if len <= 23 {
                push(out, i, major | (len as u8))
            } else if len <= 255 {
                push(out, i, major | 24)?;
                push(out, i, len as u8)
            } else {
                Err(Error::Id)
            }
        }

        fn put_uint(out: &mut [u8], i: &mut usize, n: u8) -> Result<(), Error> {
            if n <= 23 {
                push(out, i, n)
            } else {
                push(out, i, 0x18)?;
                push(out, i, n)
            }
        }

        fn push(out: &mut [u8], i: &mut usize, b: u8) -> Result<(), Error> {
            if *i >= out.len() {
                return Err(Error::BufferTooSmall);
            }
            out[*i] = b;
            *i += 1;
            Ok(())
        }

        fn push_slice(out: &mut [u8], i: &mut usize, bytes: &[u8]) -> Result<(), Error> {
            let end = i.checked_add(bytes.len()).ok_or(Error::BufferTooSmall)?;
            if end > out.len() {
                return Err(Error::BufferTooSmall);
            }
            out[*i..end].copy_from_slice(bytes);
            *i = end;
            Ok(())
        }
    }
}
