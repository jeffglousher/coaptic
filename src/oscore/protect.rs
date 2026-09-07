//! Protect / unprotect a CoAP [`Message`] / [`ParsedMessage`].

use crate::error::EncodeError;
use crate::message::{
    Code, Message, MessageId, Opt, OptionsBuilder, ParsedMessage, Token, Type, decode, write_option,
};

use super::aead::{self, Aad};
use super::context::{RequestRef, SecurityContext};
use super::header::{self, OptionClass, OscoreHeader, PartialIv};
use super::{Error, MAX_ID_CONTEXT_LEN, MAX_ID_LEN, MAX_PIV_LEN, TAG_LEN};

/// Maximum Class E / U options this slice copies (path + query + extras).
const OPT_SLOTS: usize = 16;
/// Inner plaintext / ciphertext scratch.
const INNER: usize = 1280;

/// Plug-in a caller can implement instead of [`SecurityContext`].
///
/// [`crate::App::set_oscore`] takes [`SecurityContext`]. Use this trait when
/// you drive protect/unprotect yourself (tests, a later group-OSCORE
/// context, or a side table of pairwise contexts).
pub trait OscoreContext {
    /// Protect a request. Writes a complete OSCORE datagram into `out`.
    fn protect_request(&mut self, plain: &Message<'_>, out: &mut [u8]) -> Result<usize, Error>;

    /// Open a request. Writes the inner CoAP datagram into `out`.
    fn unprotect_request<'a>(
        &mut self,
        protected: &ParsedMessage<'_>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a>, RequestRef), Error>;

    /// Protect a response bound to `request`.
    fn protect_response(
        &self,
        plain: &Message<'_>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error>;

    /// Open a response bound to `request`.
    fn unprotect_response<'a>(
        &self,
        protected: &ParsedMessage<'_>,
        request: RequestRef,
        out: &'a mut [u8],
    ) -> Result<ParsedMessage<'a>, Error>;
}

impl OscoreContext for SecurityContext {
    fn protect_request(&mut self, plain: &Message<'_>, out: &mut [u8]) -> Result<usize, Error> {
        protect_request(self, plain, out)
    }

    fn unprotect_request<'a>(
        &mut self,
        protected: &ParsedMessage<'_>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a>, RequestRef), Error> {
        unprotect_request(self, protected, out)
    }

    fn protect_response(
        &self,
        plain: &Message<'_>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_response(self, plain, request, out)
    }

    fn unprotect_response<'a>(
        &self,
        protected: &ParsedMessage<'_>,
        request: RequestRef,
        out: &'a mut [u8],
    ) -> Result<ParsedMessage<'a>, Error> {
        unprotect_response(self, protected, request, out)
    }
}

impl SecurityContext {
    /// Protect `plain` as an OSCORE request. See [`OscoreContext::protect_request`].
    pub fn protect_request(&mut self, plain: &Message<'_>, out: &mut [u8]) -> Result<usize, Error> {
        protect_request(self, plain, out)
    }

    /// Unprotect an OSCORE request. See [`OscoreContext::unprotect_request`].
    pub fn unprotect_request<'a>(
        &mut self,
        protected: &ParsedMessage<'_>,
        out: &'a mut [u8],
    ) -> Result<(ParsedMessage<'a>, RequestRef), Error> {
        unprotect_request(self, protected, out)
    }

    /// Protect `plain` as an OSCORE response (no new Partial IV).
    pub fn protect_response(
        &self,
        plain: &Message<'_>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_response(self, plain, request, out)
    }

    /// Protect a response that carries a new Partial IV (Appendix C.8).
    pub fn protect_response_with_piv(
        &mut self,
        plain: &Message<'_>,
        request: RequestRef,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        protect_response_piv(self, plain, request, out)
    }

    /// Unprotect an OSCORE response.
    pub fn unprotect_response<'a>(
        &self,
        protected: &ParsedMessage<'_>,
        request: RequestRef,
        out: &'a mut [u8],
    ) -> Result<ParsedMessage<'a>, Error> {
        unprotect_response(self, protected, request, out)
    }
}

/// See [`SecurityContext::protect_request`].
pub fn protect_request(
    ctx: &mut SecurityContext,
    plain: &Message<'_>,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !plain.code().is_request() {
        return Err(Error::MessageLength);
    }
    let piv = ctx.take_sender_piv()?;
    let request = ctx.request_ref(piv);
    let nonce = ctx.request_nonce(piv);
    let aad = Aad::new(request.kid(), piv.as_bytes())?;
    let mut plaintext = [0u8; INNER];
    let pt_len = encode_plaintext(plain, &mut plaintext)?;
    let mut ciphertext = [0u8; INNER];
    let ct_len = aead::seal(
        ctx.sender_key(),
        &nonce,
        aad.as_bytes(),
        &plaintext[..pt_len],
        &mut ciphertext,
    )?;
    let kid_ctx = if ctx.id_context().is_empty() {
        None
    } else {
        Some(ctx.id_context())
    };
    let n = encode_outer(
        plain.ty(),
        Code::POST,
        plain.message_id(),
        plain.token(),
        plain.options(),
        Some(piv),
        kid_ctx,
        Some(ctx.sender_id()),
        &ciphertext[..ct_len],
        out,
    )?;
    ctx.remember(plain.token(), request)?;
    Ok(n)
}

/// See [`SecurityContext::unprotect_request`].
pub fn unprotect_request<'a>(
    ctx: &mut SecurityContext,
    protected: &ParsedMessage<'_>,
    out: &'a mut [u8],
) -> Result<(ParsedMessage<'a>, RequestRef), Error> {
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
    let n = stitch_inner(
        protected.ty(),
        protected.message_id(),
        protected.token(),
        protected.options(),
        &plaintext[..pt_len],
        out,
    )?;
    let inner = decode(&out[..n])?;
    let request = RequestRef::from_kid(kid, piv)?;
    ctx.remember(protected.token(), request)?;
    Ok((inner, request))
}

/// See [`SecurityContext::protect_response`].
pub fn protect_response(
    ctx: &SecurityContext,
    plain: &Message<'_>,
    request: RequestRef,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !plain.code().is_response() {
        return Err(Error::MessageLength);
    }
    let aad = Aad::new(request.kid(), request.piv().as_bytes())?;
    let nonce = aead::nonce(ctx.common_iv(), request.kid(), request.piv());
    let mut plaintext = [0u8; INNER];
    let pt_len = encode_plaintext(plain, &mut plaintext)?;
    let mut ciphertext = [0u8; INNER];
    let ct_len = aead::seal(
        ctx.sender_key(),
        &nonce,
        aad.as_bytes(),
        &plaintext[..pt_len],
        &mut ciphertext,
    )?;
    encode_outer(
        plain.ty(),
        Code::CHANGED,
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
pub fn protect_response_piv(
    ctx: &mut SecurityContext,
    plain: &Message<'_>,
    request: RequestRef,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !plain.code().is_response() {
        return Err(Error::MessageLength);
    }
    let piv = ctx.take_sender_piv()?;
    let aad = Aad::new(request.kid(), request.piv().as_bytes())?;
    let nonce = ctx.request_nonce(piv);
    let mut plaintext = [0u8; INNER];
    let pt_len = encode_plaintext(plain, &mut plaintext)?;
    let mut ciphertext = [0u8; INNER];
    let ct_len = aead::seal(
        ctx.sender_key(),
        &nonce,
        aad.as_bytes(),
        &plaintext[..pt_len],
        &mut ciphertext,
    )?;
    encode_outer(
        plain.ty(),
        Code::CHANGED,
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

/// See [`SecurityContext::unprotect_response`].
pub fn unprotect_response<'a>(
    ctx: &SecurityContext,
    protected: &ParsedMessage<'_>,
    request: RequestRef,
    out: &'a mut [u8],
) -> Result<ParsedMessage<'a>, Error> {
    let header = parse_protected(protected)?;
    if protected.payload().len() < TAG_LEN {
        return Err(Error::MessageLength);
    }
    let aad = Aad::new(request.kid(), request.piv().as_bytes())?;
    let nonce = match header.piv {
        Some(piv) => aead::nonce(ctx.common_iv(), ctx.recipient_id(), piv),
        None => aead::nonce(ctx.common_iv(), request.kid(), request.piv()),
    };
    let mut plaintext = [0u8; INNER];
    let pt_len = aead::open(
        ctx.recipient_key(),
        &nonce,
        aad.as_bytes(),
        protected.payload(),
        &mut plaintext,
    )?;
    let n = stitch_inner(
        protected.ty(),
        protected.message_id(),
        protected.token(),
        protected.options(),
        &plaintext[..pt_len],
        out,
    )?;
    Ok(decode(&out[..n])?)
}

fn parse_protected<'a>(protected: &ParsedMessage<'a>) -> Result<OscoreHeader<'a>, Error> {
    let value = protected.oscore().ok_or(Error::Header)?;
    if protected.payload().is_empty() {
        return Err(Error::MessageLength);
    }
    OscoreHeader::parse(value)
}

fn encode_plaintext(plain: &Message<'_>, out: &mut [u8]) -> Result<usize, Error> {
    if out.is_empty() {
        return Err(Error::BufferTooSmall);
    }
    out[0] = plain.code().as_raw();
    let mut i = 1;
    let mut prev = 0u16;
    for opt in plain.options() {
        let n = opt.number().get();
        if header::is_oscore(n) || header::classify(n) != OptionClass::Inner {
            continue;
        }
        if n < prev {
            return Err(Error::Encode(EncodeError::OptionsNotAscending));
        }
        i = write_option(out, i, u32::from(n - prev), opt.value())?;
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
fn encode_outer(
    ty: Type,
    code: Code,
    mid: MessageId,
    token: Token,
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
        if header::is_oscore(n) || header::classify(n) != OptionClass::Outer {
            continue;
        }
        opts.push(*opt).map_err(|_| Error::Options)?;
    }
    opts.push(Opt::oscore(&oscore[..oscore_n]))
        .map_err(|_| Error::Options)?;
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(ciphertext);
    Ok(msg.encode(out)?)
}

fn stitch_inner(
    ty: Type,
    mid: MessageId,
    token: Token,
    outer_opts: crate::message::Options<'_>,
    plaintext: &[u8],
    out: &mut [u8],
) -> Result<usize, Error> {
    if plaintext.is_empty() {
        return Err(Error::MessageLength);
    }
    let tkl = token.len();
    let header_end = 4 + tkl;
    let rest = &plaintext[1..];
    if out.len() < header_end + rest.len() {
        return Err(Error::BufferTooSmall);
    }
    out[0] = (1 << 6) | (ty.to_bits() << 4) | (tkl as u8);
    out[1] = plaintext[0];
    let mid_be = mid.get().to_be_bytes();
    out[2] = mid_be[0];
    out[3] = mid_be[1];
    out[4..header_end].copy_from_slice(token.as_bytes());
    out[header_end..header_end + rest.len()].copy_from_slice(rest);

    // Re-encode so Class U outer options (Uri-Host, …) sit beside Class E.
    let mut merged = [0u8; INNER];
    let n = {
        let fake = decode(&out[..header_end + rest.len()])?;
        let mut opts = OptionsBuilder::<OPT_SLOTS>::new();
        for opt in outer_opts {
            let n = opt.number().get();
            if header::is_oscore(n) || header::classify(n) != OptionClass::Outer {
                continue;
            }
            opts.push(opt).map_err(|_| Error::Options)?;
        }
        for opt in fake.options() {
            opts.push(opt).map_err(|_| Error::Options)?;
        }
        let msg = Message::new(ty, fake.code(), mid)
            .with_token(token)
            .with_options(opts.as_slice())
            .with_payload(fake.payload());
        msg.encode(&mut merged)?
    };
    if n > out.len() {
        return Err(Error::BufferTooSmall);
    }
    out[..n].copy_from_slice(&merged[..n]);
    Ok(n)
}
