//! App-side OSCORE hook. Zero-sized when the `oscore` feature is off.

use crate::error::{EncodeError, SlotMessageError};
use crate::message::{Message, ParsedMessage};
use crate::storage::{DatagramSlots, Engine, SlotId, Storage};

#[cfg(feature = "oscore")]
use crate::oscore::{Error as OscoreError, RequestRef, SecurityContext};

/// [`App`](super::App) field: a caller-owned context, or an empty marker.
#[cfg(feature = "oscore")]
pub(crate) type Field = Option<SecurityContext>;
#[cfg(not(feature = "oscore"))]
#[derive(Clone, Copy)]
pub(crate) struct Field;

/// Request binding carried on an inbound OSCORE exchange.
#[cfg(feature = "oscore")]
pub(crate) type Request = Option<RequestRef>;
#[cfg(not(feature = "oscore"))]
#[derive(Clone, Copy)]
pub(crate) struct Request;

/// Opened inner message after unprotect.
#[cfg(feature = "oscore")]
pub(crate) type Opened<'a> = (ParsedMessage<'a>, RequestRef);
#[cfg(not(feature = "oscore"))]
pub(crate) type Opened<'a> = (ParsedMessage<'a>, Request);

#[cfg(feature = "oscore")]
pub(crate) type InboundError = OscoreError;
#[cfg(not(feature = "oscore"))]
pub(crate) type InboundError = core::convert::Infallible;

#[cfg(feature = "oscore")]
pub(crate) const fn no_request() -> Request {
    None
}

#[cfg(not(feature = "oscore"))]
pub(crate) const fn no_request() -> Request {
    Request
}

#[cfg(feature = "oscore")]
pub(crate) const fn empty_field() -> Field {
    None
}

#[cfg(not(feature = "oscore"))]
pub(crate) const fn empty_field() -> Field {
    Field
}

#[cfg(feature = "oscore")]
pub(crate) fn is_active(ctx: &Field) -> bool {
    ctx.is_some()
}

/// Request binding stored on an Observe interest (notifications).
pub(crate) fn request_from_interest(interest: crate::storage::ObserveInterest) -> Request {
    #[cfg(feature = "oscore")]
    {
        interest.oscore()
    }
    #[cfg(not(feature = "oscore"))]
    {
        let _ = interest;
        no_request()
    }
}

#[cfg(not(feature = "oscore"))]
pub(crate) fn is_active(_ctx: &Field) -> bool {
    false
}

/// After decode: unprotect an OSCORE message, or reject plaintext.
///
/// When a context is attached, a message without an OSCORE option is
/// fail-closed except empty ACK/RST (and other empty code-0.00 control
/// datagrams). Those are RFC 7252 reliability / ping; they carry no
/// application body. A token-matching unprotected 2.xx/4.xx/5.xx must
/// not complete a [`Call`](super::Call).
#[cfg(feature = "oscore")]
pub(crate) fn inbound<'a>(
    ctx: &mut Field,
    parsed: &ParsedMessage<'_>,
    scratch: &'a mut [u8],
) -> Result<Option<Opened<'a>>, InboundError> {
    let Some(ctx) = ctx.as_mut() else {
        return Ok(None);
    };
    if parsed.oscore().is_some() {
        let (inner, request) = if parsed.code().is_request() {
            ctx.unprotect_request(parsed, scratch)?
        } else {
            let request = ctx.lookup(parsed.token()).ok_or(OscoreError::Context)?;
            let inner = ctx.unprotect_response(parsed, request, scratch)?;
            let header =
                crate::oscore::OscoreHeader::parse(parsed.oscore().ok_or(OscoreError::Header)?)?;
            let observe = inner.observe().is_some();
            if observe && !ctx.is_observe(parsed.token()) {
                // Response to a non-Observe request must not carry Inner Observe.
                return Err(OscoreError::Replay);
            }
            if observe {
                ctx.accept_notification(parsed.token(), header.piv)?;
            } else {
                let _ = ctx.take(parsed.token());
            }
            (inner, request)
        };
        return Ok(Some((inner, request)));
    }
    if parsed.is_empty() {
        return Ok(None);
    }
    Err(OscoreError::Unprotected)
}

#[cfg(not(feature = "oscore"))]
pub(crate) fn inbound<'a>(
    _ctx: &mut Field,
    _parsed: &ParsedMessage<'_>,
    _scratch: &'a mut [u8],
) -> Result<Option<Opened<'a>>, InboundError> {
    Ok(None)
}

/// Encode `msg`, or OSCORE-protect a request into `tx`.
pub(crate) fn encode_request<S: Storage + DatagramSlots>(
    ctx: &mut Field,
    engine: &mut Engine<S>,
    tx: SlotId,
    msg: &Message<'_>,
) -> Result<(), SlotMessageError> {
    let _ = ctx;
    #[cfg(feature = "oscore")]
    if let Some(ctx) = ctx.as_mut() {
        let mut wire = [0u8; super::DATAGRAM_SCRATCH];
        let n = ctx.protect_request(msg, &mut wire).map_err(protect_err)?;
        return fill_tx(engine, tx, &wire[..n]);
    }
    engine.encode_tx(tx, msg).map(|_| ())
}

/// Encode `msg`, or OSCORE-protect a response bound to `request`.
pub(crate) fn encode_message<S: Storage + DatagramSlots>(
    ctx: &Field,
    request: Request,
    engine: &mut Engine<S>,
    tx: SlotId,
    msg: &Message<'_>,
) -> Result<(), SlotMessageError> {
    let _ = (ctx, request);
    #[cfg(feature = "oscore")]
    if let (Some(ctx), Some(request)) = (ctx.as_ref(), request) {
        let mut wire = [0u8; super::DATAGRAM_SCRATCH];
        let n = ctx
            .protect_response(msg, request, &mut wire)
            .map_err(protect_err)?;
        return fill_tx(engine, tx, &wire[..n]);
    }
    engine.encode_tx(tx, msg).map(|_| ())
}

/// Protect a notification with a new Partial IV (RFC 8613 §4.1.3.5.2).
///
/// Fail-closed: an attached context without a stored [`RequestRef`] does
/// not fall back to plaintext.
pub(crate) fn encode_notification<S: Storage + DatagramSlots>(
    ctx: &mut Field,
    request: Request,
    engine: &mut Engine<S>,
    tx: SlotId,
    msg: &Message<'_>,
) -> Result<(), SlotMessageError> {
    #[cfg(not(feature = "oscore"))]
    let _ = (ctx, request);
    #[cfg(feature = "oscore")]
    if let Some(oscore) = ctx.as_mut() {
        let Some(request) = request else {
            return Err(protect_err(OscoreError::Context));
        };
        let mut wire = [0u8; super::DATAGRAM_SCRATCH];
        let n = oscore
            .protect_response_with_piv(msg, request, &mut wire)
            .map_err(protect_err)?;
        return fill_tx(engine, tx, &wire[..n]);
    }
    engine.encode_tx(tx, msg).map(|_| ())
}

#[cfg_attr(not(feature = "oscore"), allow(dead_code))]
pub(crate) fn fill_tx<S: Storage + DatagramSlots>(
    engine: &mut Engine<S>,
    tx: SlotId,
    bytes: &[u8],
) -> Result<(), SlotMessageError> {
    let mut access = engine.access_tx_mut(tx)?;
    if bytes.len() > access.capacity() {
        return Err(SlotMessageError::Encode(EncodeError::BufferTooSmall));
    }
    access.bytes_mut()[..bytes.len()].copy_from_slice(bytes);
    access
        .set_len(bytes.len())
        .map_err(SlotMessageError::Slot)?;
    Ok(())
}

/// Map a protect failure onto encode. Never falls back to plaintext.
#[cfg(feature = "oscore")]
fn protect_err(err: OscoreError) -> SlotMessageError {
    match err {
        OscoreError::BufferTooSmall | OscoreError::MessageLength => {
            SlotMessageError::Encode(EncodeError::BufferTooSmall)
        }
        OscoreError::Encode(e) => SlotMessageError::Encode(e),
        OscoreError::Options | OscoreError::Saturated => {
            SlotMessageError::Encode(EncodeError::OptionsFull)
        }
        _ => SlotMessageError::Encode(EncodeError::BufferTooSmall),
    }
}
