//! Protect / unprotect a CoAP [`Message`] / [`ParsedMessage`].

use crate::error::EncodeError;
use crate::message::{
    Code, Message, MessageId, Opt, OptionNumber, OptionsBuilder, OwnedToken, ParsedMessage, Token,
    Type, write_option,
};

use super::aead::{self, Aad};
use super::context::{BoundedSecurityContext, RequestRef};
use super::header::{self, OptionClass, OscoreHeader, PartialIv};
use super::{Error, MAX_ID_CONTEXT_LEN, MAX_ID_LEN, MAX_PIV_LEN, TAG_LEN};

/// Maximum Class E / U options this slice copies (path + query + extras).
const OPT_SLOTS: usize = 16;
/// Inner plaintext / ciphertext scratch.
const INNER: usize = 1280;

/// Plug-in a caller can implement instead of [`super::SecurityContext`].
///
/// [`crate::App::set_oscore`] takes a matching [`BoundedSecurityContext`]. Use this trait when
/// you drive protect/unprotect yourself (tests, a later group-OSCORE
/// context, or a side table of pairwise contexts).
pub trait OscoreContext<T: OwnedToken = Token> {
    /// Protect a request. Writes a complete OSCORE datagram into `out`.
    fn protect_request(&mut self, plain: &Message<'_, T>, out: &mut [u8]) -> Result<usize, Error>;

    /// Open a request. Writes the inner CoAP datagram into `out`.
    fn unprotect_request<'a>(
        &mut self,
        protected: &ParsedMessage<'_, T>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a, T>, RequestRef), Error>;

    /// Protect a response bound to `request`.
    fn protect_response(
        &self,
        plain: &Message<'_, T>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error>;

    /// Open a response bound to `request`.
    fn unprotect_response<'a>(
        &self,
        protected: &ParsedMessage<'_, T>,
        request: RequestRef,
        out: &'a mut [u8],
    ) -> Result<ParsedMessage<'a, T>, Error>;
}

impl<const REQUESTS: usize, T: OwnedToken> OscoreContext<T>
    for BoundedSecurityContext<REQUESTS, T>
{
    fn protect_request(&mut self, plain: &Message<'_, T>, out: &mut [u8]) -> Result<usize, Error> {
        protect_request(self, plain, out)
    }

    fn unprotect_request<'a>(
        &mut self,
        protected: &ParsedMessage<'_, T>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a, T>, RequestRef), Error> {
        unprotect_request(self, protected, out)
    }

    fn protect_response(
        &self,
        plain: &Message<'_, T>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_response(self, plain, request, out)
    }

    fn unprotect_response<'a>(
        &self,
        protected: &ParsedMessage<'_, T>,
        request: RequestRef,
        out: &'a mut [u8],
    ) -> Result<ParsedMessage<'a, T>, Error> {
        unprotect_response(self, protected, request, out)
    }
}

impl<const REQUESTS: usize, T: OwnedToken> BoundedSecurityContext<REQUESTS, T> {
    /// Protect `plain` as an OSCORE request. See [`OscoreContext::protect_request`].
    pub fn protect_request(
        &mut self,
        plain: &Message<'_, T>,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_request(self, plain, out)
    }

    /// Unprotect an OSCORE request. See [`OscoreContext::unprotect_request`].
    pub fn unprotect_request<'a>(
        &mut self,
        protected: &ParsedMessage<'_, T>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a, T>, RequestRef), Error> {
        unprotect_request(self, protected, out)
    }

    /// Protect the single response without a new Partial IV for this request.
    /// The caller must use this for at most one distinct response plaintext per
    /// [`RequestRef`]. Retransmit retained bytes; use
    /// [`Self::protect_response_with_piv`] for additional responses such as Q
    /// payloads or notifications. This low-level API does not retain that budget.
    pub fn protect_response(
        &self,
        plain: &Message<'_, T>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_response(self, plain, request, out)
    }

    /// Protect a response that carries a new Partial IV (Appendix C.8).
    pub fn protect_response_with_piv(
        &mut self,
        plain: &Message<'_, T>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_response_piv(self, plain, request, out)
    }

    /// Unprotect an OSCORE response.
    pub fn unprotect_response<'a>(
        &self,
        protected: &ParsedMessage<'_, T>,
        request: RequestRef,
        out: &'a mut [u8],
    ) -> Result<ParsedMessage<'a, T>, Error> {
        unprotect_response(self, protected, request, out)
    }
}

/// See [`super::SecurityContext::protect_request`].
pub fn protect_request<const REQUESTS: usize, T: OwnedToken>(
    ctx: &mut BoundedSecurityContext<REQUESTS, T>,
    plain: &Message<'_, T>,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !plain.code().is_request() {
        return Err(Error::MessageLength);
    }
    let piv = ctx.take_sender_piv()?;
    let request = ctx.request_ref(piv);
    let nonce = ctx.request_nonce(piv);
    let aad = Aad::new(request.kid(), piv.as_bytes())?;
    let mut ciphertext = [0u8; INNER];
    let pt_len = encode_plaintext(plain, &mut ciphertext)?;
    let ct_len = aead::seal_in_place(
        ctx.sender_key(),
        &nonce,
        aad.as_bytes(),
        pt_len,
        &mut ciphertext,
    )?;
    let kid_ctx = if ctx.id_context().is_empty() {
        None
    } else {
        Some(ctx.id_context())
    };
    let n = encode_outer(
        plain.ty(),
        outer_code(plain.code(), plain.options()),
        plain.message_id(),
        plain.token(),
        plain.options(),
        Some(piv),
        kid_ctx,
        Some(ctx.sender_id()),
        &ciphertext[..ct_len],
        out,
    )?;
    let observe = plain.options().iter().any(is_observe);
    if !observe
        && plain.options().iter().any(|option| {
            matches!(
                option.number(),
                OptionNumber::BLOCK2 | OptionNumber::Q_BLOCK2
            )
        })
    {
        ctx.remember_download(plain.token(), request)?;
    } else {
        ctx.remember_live(plain.token(), request, observe)?;
    }
    Ok(n)
}

/// See [`super::SecurityContext::unprotect_request`].
pub fn unprotect_request<'a, const REQUESTS: usize, T: OwnedToken>(
    ctx: &mut BoundedSecurityContext<REQUESTS, T>,
    protected: &ParsedMessage<'_, T>,
    out: &'a mut [u8],
) -> Result<(ParsedMessage<'a, T>, RequestRef), Error> {
    let (wire, request) = unprotect_request_wire(ctx, protected, out)?;
    Ok((decode_owned::<T>(wire)?, request))
}

fn unprotect_request_wire<'a, const REQUESTS: usize, T: OwnedToken>(
    ctx: &mut BoundedSecurityContext<REQUESTS, T>,
    protected: &ParsedMessage<'_, T>,
    out: &'a mut [u8],
) -> Result<(&'a [u8], RequestRef), Error> {
    let header = parse_protected(protected)?;
    if !ctx.matches_recipient(header.kid, header.kid_context) {
        return Err(Error::Context);
    }
    let piv = header.piv.ok_or(Error::PartialIv)?;
    if !ctx.replay_fresh(piv.seq()) {
        return Err(Error::Replay);
    }
    if protected.payload().len() < TAG_LEN {
        return Err(Error::MessageLength);
    }
    let kid = header.kid.unwrap_or(&[]);
    let aad = Aad::new(kid, piv.as_bytes())?;
    let nonce = ctx.recipient_nonce(piv);
    let mut plaintext = [0u8; INNER];
    let pt_len = aead::open(
        ctx.recipient_key(),
        &nonce,
        aad.as_bytes(),
        protected.payload(),
        &mut plaintext,
    )?;
    ctx.replay_accept(piv.seq());
    let code = Code::from_raw(*plaintext[..pt_len].first().ok_or(Error::MessageLength)?);
    if !code.is_request() {
        return Err(Error::MessageCode);
    }
    let n = stitch_inner(
        protected.ty(),
        protected.message_id(),
        protected.token(),
        protected.options(),
        &plaintext[..pt_len],
        out,
    )?;
    let request = RequestRef::from_kid(kid, piv)?;
    // Caller (App) holds `request` on the inbound exchange for
    // `protect_response`. The live Token table is the client in-flight
    // set (`protect_request` → `unprotect_response`); do not remember
    // here or sequential server Tokens saturate the outbound binding table.
    Ok((&out[..n], request))
}

/// See [`super::SecurityContext::protect_response`].
pub fn protect_response<const REQUESTS: usize, T: OwnedToken>(
    ctx: &BoundedSecurityContext<REQUESTS, T>,
    plain: &Message<'_, T>,
    request: RequestRef,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !plain.code().is_response() {
        return Err(Error::MessageLength);
    }
    let aad = Aad::new(request.kid(), request.piv().as_bytes())?;
    let nonce = aead::nonce(ctx.common_iv(), request.kid(), request.piv());
    let mut ciphertext = [0u8; INNER];
    let pt_len = encode_plaintext(plain, &mut ciphertext)?;
    let ct_len = aead::seal_in_place(
        ctx.sender_key(),
        &nonce,
        aad.as_bytes(),
        pt_len,
        &mut ciphertext,
    )?;
    encode_outer(
        plain.ty(),
        outer_code(plain.code(), plain.options()),
        plain.message_id(),
        plain.token(),
        plain.options(),
        None,
        None,
        None,
        &ciphertext[..ct_len],
        out,
    )
}

/// Protect a response and optionally include a new Partial IV.
pub fn protect_response_piv<const REQUESTS: usize, T: OwnedToken>(
    ctx: &mut BoundedSecurityContext<REQUESTS, T>,
    plain: &Message<'_, T>,
    request: RequestRef,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !plain.code().is_response() {
        return Err(Error::MessageLength);
    }
    let piv = ctx.take_sender_piv()?;
    let aad = Aad::new(request.kid(), request.piv().as_bytes())?;
    let nonce = ctx.request_nonce(piv);
    let mut ciphertext = [0u8; INNER];
    let pt_len = encode_plaintext(plain, &mut ciphertext)?;
    let ct_len = aead::seal_in_place(
        ctx.sender_key(),
        &nonce,
        aad.as_bytes(),
        pt_len,
        &mut ciphertext,
    )?;
    encode_outer(
        plain.ty(),
        outer_code(plain.code(), plain.options()),
        plain.message_id(),
        plain.token(),
        plain.options(),
        Some(piv),
        None,
        None,
        &ciphertext[..ct_len],
        out,
    )
}

/// See [`super::SecurityContext::unprotect_response`].
pub fn unprotect_response<'a, const REQUESTS: usize, T: OwnedToken>(
    ctx: &BoundedSecurityContext<REQUESTS, T>,
    protected: &ParsedMessage<'_, T>,
    request: RequestRef,
    out: &'a mut [u8],
) -> Result<ParsedMessage<'a, T>, Error> {
    let (wire, _) = unprotect_response_candidates(ctx, protected, request, None, out)?;
    decode_owned::<T>(wire)
}

impl<const REQUESTS: usize, T: OwnedToken> BoundedSecurityContext<REQUESTS, T> {
    pub(crate) fn unprotect_request_wire<'a>(
        &mut self,
        protected: &ParsedMessage<'_, T>,
        out: &'a mut [u8],
    ) -> Result<(&'a [u8], RequestRef), Error> {
        unprotect_request_wire(self, protected, out)
    }

    #[cfg(test)]
    pub(crate) fn unprotect_bound_response<'a>(
        &self,
        protected: &ParsedMessage<'_, T>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a, T>, RequestRef), Error> {
        let (wire, request) = self.unprotect_bound_response_wire(protected, out)?;
        Ok((decode_owned::<T>(wire)?, request))
    }

    pub(crate) fn unprotect_bound_response_wire<'a>(
        &self,
        protected: &ParsedMessage<'_, T>,
        out: &'a mut [u8],
    ) -> Result<(&'a [u8], RequestRef), Error> {
        let request = self.lookup(protected.token()).ok_or(Error::Context)?;
        let observation = self
            .observe_request(protected.token())
            .filter(|old| *old != request);
        unprotect_response_candidates(self, protected, request, observation, out)
    }
}

fn unprotect_response_candidates<'a, const REQUESTS: usize, T: OwnedToken>(
    ctx: &BoundedSecurityContext<REQUESTS, T>,
    protected: &ParsedMessage<'_, T>,
    request: RequestRef,
    fallback: Option<RequestRef>,
    out: &'a mut [u8],
) -> Result<(&'a [u8], RequestRef), Error> {
    let header = parse_protected(protected)?;
    if protected.payload().len() < TAG_LEN {
        return Err(Error::MessageLength);
    }
    let mut plaintext = [0u8; INNER];
    let mut accepted = None;
    // At most two AEAD attempts, using the same fixed scratch. Decode and
    // admission happen only after authentication identifies the request.
    for candidate in [Some(request), fallback].into_iter().flatten() {
        let aad = Aad::new(candidate.kid(), candidate.piv().as_bytes())?;
        let nonce = match header.piv {
            Some(piv) => aead::nonce(ctx.common_iv(), ctx.recipient_id(), piv),
            None => aead::nonce(ctx.common_iv(), candidate.kid(), candidate.piv()),
        };
        match aead::open(
            ctx.recipient_key(),
            &nonce,
            aad.as_bytes(),
            protected.payload(),
            &mut plaintext,
        ) {
            Ok(len) => {
                accepted = Some((len, candidate));
                break;
            }
            Err(Error::Decrypt) => {}
            Err(error) => return Err(error),
        }
    }
    let (len, request) = accepted.ok_or(Error::Decrypt)?;
    let code = Code::from_raw(*plaintext[..len].first().ok_or(Error::MessageLength)?);
    if !code.is_response() {
        return Err(Error::MessageCode);
    }
    let n = stitch_inner(
        protected.ty(),
        protected.message_id(),
        protected.token(),
        protected.options(),
        &plaintext[..len],
        out,
    )?;
    Ok((&out[..n], request))
}

fn parse_protected<'a, T: OwnedToken>(
    protected: &ParsedMessage<'a, T>,
) -> Result<OscoreHeader<'a>, Error> {
    let value = protected.oscore().ok_or(Error::Header)?;
    if protected.payload().is_empty() {
        return Err(Error::MessageLength);
    }
    OscoreHeader::parse(value)
}

fn is_observe(opt: &Opt<'_>) -> bool {
    opt.number().get() == 6
}

/// Outer Code: FETCH / Content when Observe is present (RFC 8613 §4.1.3.5).
fn outer_code(inner: Code, opts: &[Opt<'_>]) -> Code {
    if !opts.iter().any(is_observe) {
        return if inner.is_request() {
            Code::POST
        } else {
            Code::CHANGED
        };
    }
    if inner.is_request() {
        Code::FETCH
    } else {
        Code::CONTENT
    }
}

fn encode_plaintext<T: OwnedToken>(plain: &Message<'_, T>, out: &mut [u8]) -> Result<usize, Error> {
    if out.is_empty() {
        return Err(Error::BufferTooSmall);
    }
    out[0] = plain.code().as_raw();
    let mut i = 1;
    let mut prev = 0u16;
    let notify = plain.code().is_response();
    for opt in plain.options() {
        let n = opt.number().get();
        if header::is_oscore(n) || !header::classify(n).in_plaintext() {
            continue;
        }
        if n < prev {
            return Err(Error::Encode(EncodeError::OptionsNotAscending));
        }
        // Notifications: Inner Observe MUST be empty (RFC 8613 §4.1.3.5.2).
        let value = if n == 6 && notify {
            &[][..]
        } else {
            opt.value()
        };
        i = write_option(out, i, u32::from(n - prev), value)?;
        prev = n;
    }
    if !plain.payload().is_empty() {
        if i >= out.len() {
            return Err(Error::BufferTooSmall);
        }
        out[i] = 0xff;
        i += 1;
        let end = i
            .checked_add(plain.payload().len())
            .ok_or(Error::BufferTooSmall)?;
        if end > out.len() {
            return Err(Error::BufferTooSmall);
        }
        out[i..end].copy_from_slice(plain.payload());
        i = end;
    }
    Ok(i)
}

#[allow(clippy::too_many_arguments)]
fn encode_outer<T: OwnedToken>(
    ty: Type,
    code: Code,
    mid: MessageId,
    token: T,
    inner_opts: &[Opt<'_>],
    piv: Option<PartialIv>,
    kid_context: Option<&[u8]>,
    kid: Option<&[u8]>,
    ciphertext: &[u8],
    out: &mut [u8],
) -> Result<usize, Error> {
    let mut oscore = [0u8; 1 + MAX_PIV_LEN + 1 + MAX_ID_CONTEXT_LEN + MAX_ID_LEN];
    let oscore_n = header::encode_option(&mut oscore, piv, kid_context, kid)?;
    let mut opts = OptionsBuilder::<OPT_SLOTS>::new();
    for opt in inner_opts {
        let n = opt.number().get();
        if header::is_oscore(n) || !header::encode_as_outer(n) {
            continue;
        }
        opts.push(*opt).map_err(|_| Error::Options)?;
    }
    opts.push(Opt::oscore(&oscore[..oscore_n]))
        .map_err(|_| Error::Options)?;
    // Observe responses appear as 2.05 Content (cacheable) to
    // OSCORE-unaware proxies. Outer Max-Age 0 is the Dual Class U
    // field (`knowledge/rfcs/rfc8613.txt` §4.1.3.1). Application
    // Max-Age stays Inner (`encode_as_outer(14)` is false). A full
    // outer builder is OptionsFull — do not skip the inject.
    if code.is_response() && inner_opts.iter().any(is_observe) {
        opts.push(Opt::new(OptionNumber::MAX_AGE, &[]))
            .map_err(|_| Error::Encode(EncodeError::OptionsFull))?;
    }
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(ciphertext);
    Ok(msg.encode(out)?)
}

fn stitch_inner<T: OwnedToken>(
    ty: Type,
    mid: MessageId,
    token: T,
    outer_opts: crate::message::Options<'_>,
    plaintext: &[u8],
    out: &mut [u8],
) -> Result<usize, Error> {
    if plaintext.is_empty() {
        return Err(Error::MessageLength);
    }
    let (inner_opts, payload) = crate::message::decode_options_payload(&plaintext[1..])?;
    // Merge Class U-only outer options with authenticated Inner options before
    // encoding once. Dual options on the outer datagram are not
    // merged: Observe stays Inner (§4.1.3.5.2); Block/Size stay Inner
    // (§4.1.3.4.2); Max-Age Outer is discarded (§8.4); No-Response
    // Outer is ignored (§4.1.3.6). Class E on the outer (ETag, …) is
    // discarded (§8.2 / §8.4).
    let mut opts = OptionsBuilder::<OPT_SLOTS>::new();
    for opt in outer_opts {
        let n = opt.number().get();
        if header::is_oscore(n) || header::classify(n) != OptionClass::Outer {
            continue;
        }
        opts.push(opt).map_err(|_| Error::Options)?;
    }
    for opt in inner_opts {
        opts.push(opt).map_err(|_| Error::Options)?;
    }
    let msg = Message::new(ty, Code::from_raw(plaintext[0]), mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    // Retain the former merge buffer's ceiling with larger caller storage too.
    let limit = out.len().min(INNER);
    Ok(msg.encode(&mut out[..limit])?)
}

fn decode_owned<T: OwnedToken>(wire: &[u8]) -> Result<ParsedMessage<'_, T>, Error> {
    crate::message::decode_with_token(wire).map_err(|error| match error {
        crate::message::TokenDecodeError::Malformed(error) => Error::Parse(error),
        crate::message::TokenDecodeError::Capacity(_) => Error::MessageLength,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ParseError;
    use crate::message::{decode, decode_with_token, extended};

    #[test]
    fn stitch_merges_outer_options_and_preserves_extended_token() {
        let token = extended::Token::<13>::new(b"extendedtoken").unwrap();
        let outer_options = [
            Opt::uri_host("node"),
            Opt::etag(b"fake"),
            Opt::oscore(&[]),
            Opt::new(OptionNumber::MAX_AGE, &[99]),
            Opt::new(OptionNumber::HOP_LIMIT, &[5]),
        ];
        let outer = Message::new(Type::Confirmable, Code::POST, MessageId::new(73))
            .with_token(token)
            .with_options(&outer_options);
        let mut outer_wire = [0; 128];
        let n = outer.encode(&mut outer_wire).unwrap();
        let outer = decode_with_token::<extended::Token<13>>(&outer_wire[..n]).unwrap();
        let plaintext = [Code::GET.as_raw(), 0xb1, b'x', 0x10, 0xff, b'v'];
        let mut out = [0; 128];
        let n = stitch_inner(
            outer.ty(),
            outer.message_id(),
            token,
            outer.options(),
            &plaintext,
            &mut out,
        )
        .unwrap();
        let inner = decode_with_token::<extended::Token<13>>(&out[..n]).unwrap();
        assert_eq!(inner.ty(), Type::Confirmable);
        assert_eq!(inner.code(), Code::GET);
        assert_eq!(inner.message_id(), MessageId::new(73));
        assert_eq!(inner.token(), token);
        assert!(inner.options().map(|option| option.number()).eq([
            OptionNumber::URI_HOST,
            OptionNumber::URI_PATH,
            OptionNumber::CONTENT_FORMAT,
            OptionNumber::HOP_LIMIT,
        ]));
        assert_eq!(inner.uri_host(), Some(Ok("node")));
        assert_eq!(inner.uri_path().next(), Some(Ok("x")));
        assert_eq!(
            inner.content_format(),
            Some(Ok(crate::ContentFormat::TEXT_PLAIN))
        );
        assert_eq!(
            inner.get_option(OptionNumber::HOP_LIMIT).unwrap().value(),
            &[5]
        );
        assert_eq!(inner.payload(), b"v");
    }

    #[test]
    fn stitch_refuses_malformed_inner_options_and_payload_marker() {
        let outer = decode(&[0x40, 1, 0, 1]).unwrap();
        let malformed: &[(&[u8], ParseError)] = &[
            (&[1, 0xf0], ParseError::ReservedOptionDelta),
            (&[1, 0x0f], ParseError::ReservedOptionLength),
            (&[1, 0xd0], ParseError::TruncatedOption),
            (&[1, 0xb2, b'x'], ParseError::TruncatedOption),
            (&[1, 0xff], ParseError::PayloadMarkerWithoutPayload),
        ];
        for &(plaintext, error) in malformed {
            assert_eq!(
                stitch_inner(
                    outer.ty(),
                    outer.message_id(),
                    outer.token(),
                    outer.options(),
                    plaintext,
                    &mut [0; 64],
                ),
                Err(Error::Parse(error))
            );
        }
    }

    #[test]
    fn stitch_bounds_combined_outer_and_inner_option_count() {
        let mut plaintext = [0; 35];
        plaintext[0] = Code::GET.as_raw();
        for (index, option) in plaintext[1..].as_chunks_mut::<2>().0.iter_mut().enumerate() {
            option.copy_from_slice(&[if index == 0 { 0xb1 } else { 0x01 }, b'x']);
        }
        let mut outer_wire = [0; 16];
        let outer_options = [Opt::uri_host("node")];
        let n = Message::new(Type::Confirmable, Code::POST, MessageId::new(1))
            .with_options(&outer_options)
            .encode(&mut outer_wire)
            .unwrap();
        let with_outer = decode(&outer_wire[..n]).unwrap();
        let empty_outer = decode(&[0x40, 1, 0, 1]).unwrap();
        for (outer, inner_count, accepted) in [
            (empty_outer, 16, true),
            (empty_outer, 17, false),
            (with_outer, 15, true),
            (with_outer, 16, false),
        ] {
            let mut out = [0; 128];
            let result = stitch_inner(
                outer.ty(),
                outer.message_id(),
                outer.token(),
                outer.options(),
                &plaintext[..1 + 2 * inner_count],
                &mut out,
            );
            if accepted {
                let inner = decode(&out[..result.unwrap()]).unwrap();
                assert_eq!(inner.options().count(), 16);
            } else {
                assert_eq!(result, Err(Error::Options));
            }
        }
    }

    #[test]
    fn stitch_keeps_inner_ceiling_and_refuses_short_output() {
        let outer = decode(&[0x40, 1, 0, 1]).unwrap();
        let mut plaintext = [0x5a; 1278];
        plaintext[0] = Code::GET.as_raw();
        plaintext[1] = 0xff;
        let mut out = [0; 1281];
        let n = stitch_inner(
            outer.ty(),
            outer.message_id(),
            outer.token(),
            outer.options(),
            &plaintext[..1277],
            &mut out[..1280],
        )
        .unwrap();
        assert_eq!(n, 1280);
        assert_eq!(decode(&out[..n]).unwrap().payload(), &[0x5a; 1275]);
        for (plaintext, limit) in [
            (&plaintext[..1277], 0),
            (&plaintext[..1277], 1279),
            (&plaintext[..], 1281),
        ] {
            assert_eq!(
                stitch_inner(
                    outer.ty(),
                    outer.message_id(),
                    outer.token(),
                    outer.options(),
                    plaintext,
                    &mut out[..limit],
                ),
                Err(Error::Encode(EncodeError::BufferTooSmall))
            );
        }
    }
}
