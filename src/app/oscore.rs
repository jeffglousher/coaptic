//! App-side OSCORE hook. Zero-sized when the `oscore` feature is off.
//!
//! Each live Token retains the Observe registration and the latest Block2/Q-Block2
//! request independently. Response authentication tries at most these two bindings
//! using one fixed scratch buffer; older download requests are not retained.
//! Download follow-ups omit Observe and preserve notification replay history.

#[cfg(feature = "oscore")]
use crate::error::EncodeError;
use crate::error::SlotMessageError;
use crate::message::{Message, OwnedToken, ParsedMessage, Token};
use crate::storage::{DatagramSlots, Endpoint, Engine, SlotId, Storage};

#[cfg(feature = "oscore")]
use crate::oscore::{Error as OscoreError, RequestRef};

/// [`App`](super::App) field: a caller-owned context, or an empty marker.
pub(crate) use super::associations::Store;
pub(crate) type Field<const REQUESTS: usize, K = Token> =
    dyn super::associations::ContextStorage<REQUESTS, K>;

#[cfg(feature = "oscore")]
fn selected<const R: usize, K: OwnedToken>(
    ctx: &Field<R, K>,
    namespace: u32,
) -> Result<Option<&crate::oscore::BoundedSecurityContext<R, K>>, OscoreError> {
    let value = ctx.context(namespace);
    if ctx.is_some() && value.is_none() {
        Err(OscoreError::Context)
    } else {
        Ok(value)
    }
}

#[cfg(feature = "oscore")]
fn selected_mut<const R: usize, K: OwnedToken>(
    ctx: &mut Field<R, K>,
    namespace: u32,
) -> Result<Option<&mut crate::oscore::BoundedSecurityContext<R, K>>, OscoreError> {
    let protected = ctx.is_some();
    let value = ctx.context_mut(namespace);
    if protected && value.is_none() {
        Err(OscoreError::Context)
    } else {
        Ok(value)
    }
}

pub(super) fn resolve<E, const R: usize, K: OwnedToken>(
    ctx: &Field<R, K>,
    peer: Endpoint,
) -> Result<Endpoint, super::Error<E>> {
    #[cfg(feature = "oscore")]
    {
        ctx.resolve(peer).map_err(super::Error::Oscore)
    }
    #[cfg(not(feature = "oscore"))]
    {
        let _ = ctx;
        Ok(peer)
    }
}

pub(super) fn route_allowed<const R: usize, K: OwnedToken>(
    ctx: &Field<R, K>,
    peer: Endpoint,
) -> bool {
    #[cfg(feature = "oscore")]
    {
        !ctx.is_some() || ctx.context(peer.association()).is_some()
    }
    #[cfg(not(feature = "oscore"))]
    {
        let _ = (ctx, peer);
        true
    }
}

pub(crate) fn has_live_requests<const REQUESTS: usize, K: OwnedToken>(
    ctx: &Field<REQUESTS, K>,
) -> bool {
    #[cfg(feature = "oscore")]
    {
        ctx.has_live_requests()
    }
    #[cfg(not(feature = "oscore"))]
    {
        let _ = ctx;
        false
    }
}

#[cfg(feature = "oscore")]
pub(crate) type CheckpointCommit<'a> =
    dyn FnMut(Option<super::Association>, crate::oscore::ReplayCheckpoint) -> bool + 'a;
#[cfg(not(feature = "oscore"))]
pub(crate) type CheckpointCommit<'a> = dyn FnMut() -> bool + 'a;

#[cfg(feature = "oscore")]
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum CheckpointState {
    Volatile,
    Dirty,
    Clean,
}

#[cfg(not(feature = "oscore"))]
#[derive(Clone, Copy)]
pub(crate) struct CheckpointState;

impl CheckpointState {
    pub(crate) const fn new() -> Self {
        #[cfg(feature = "oscore")]
        {
            Self::Volatile
        }
        #[cfg(not(feature = "oscore"))]
        {
            Self
        }
    }

    pub(crate) const fn required(self) -> bool {
        #[cfg(feature = "oscore")]
        {
            !matches!(self, Self::Volatile)
        }
        #[cfg(not(feature = "oscore"))]
        {
            false
        }
    }

    #[cfg(feature = "oscore")]
    pub(crate) const fn require(&mut self) {
        if !self.required() {
            *self = Self::Dirty;
        }
    }

    #[cfg(feature = "oscore")]
    pub(crate) const fn invalidate(&mut self) {
        if self.required() {
            *self = Self::Dirty;
        }
    }

    pub(crate) fn flush<E, const REQUESTS: usize, K: OwnedToken>(
        &mut self,
        ctx: &Field<REQUESTS, K>,
        commit: &mut Option<&mut CheckpointCommit<'_>>,
    ) -> Result<(), super::Error<E>> {
        #[cfg(feature = "oscore")]
        if self.required() {
            let commit = commit
                .as_mut()
                .ok_or(super::Error::OscoreCheckpointRequired)?;
            if matches!(self, Self::Dirty) {
                if !ctx.checkpoints(*commit) {
                    return Err(super::Error::OscoreCheckpointFailed);
                }
                *self = Self::Clean;
            }
        }
        #[cfg(not(feature = "oscore"))]
        let _ = (ctx, commit);
        Ok(())
    }
}

/// Request binding carried on an inbound OSCORE exchange.
#[cfg(feature = "oscore")]
pub(crate) type Request = Option<RequestRef>;
#[cfg(not(feature = "oscore"))]
#[derive(Clone, Copy)]
pub(crate) struct Request;

/// Authenticated, checked response state, committed after critical-option
/// validation and successful CON acknowledgment. No persistent storage barrier.
#[derive(Clone, Copy)]
pub(crate) enum ResponseState<K: OwnedToken = Token> {
    #[cfg(not(feature = "oscore"))]
    _Token(core::convert::Infallible, core::marker::PhantomData<K>),
    None,
    #[cfg(feature = "oscore")]
    Notification(K, Option<crate::oscore::PartialIv>),
    #[cfg(feature = "oscore")]
    Replay(u64),
    #[cfg(feature = "oscore")]
    ResponseWithoutPiv(K),
}

pub(crate) fn commit_response<E, const REQUESTS: usize, K: OwnedToken>(
    ctx: &mut Field<REQUESTS, K>,
    peer: Endpoint,
    state: ResponseState<K>,
) -> Result<(), super::Error<E>> {
    #[cfg(feature = "oscore")]
    if let Some(ctx) = selected_mut(ctx, peer.association()).map_err(super::Error::Oscore)? {
        match state {
            ResponseState::None => {}
            ResponseState::Notification(token, piv) => ctx
                .accept_notification(token, piv)
                .map_err(super::Error::Oscore)?,
            ResponseState::ResponseWithoutPiv(token) => ctx
                .accept_response_without_piv(token)
                .map_err(super::Error::Oscore)?,
            ResponseState::Replay(sequence) => {
                if !ctx.replay_fresh(sequence) {
                    return Err(super::Error::Oscore(OscoreError::Replay));
                }
                ctx.replay_accept(sequence);
            }
        }
    }
    #[cfg(not(feature = "oscore"))]
    let _ = (ctx, state, peer);
    Ok(())
}

/// Parsed Inner and its encoded wire, borrowed from the same unprotect output.
#[cfg(feature = "oscore")]
pub(crate) type Opened<'a, K = Token> =
    (ParsedMessage<'a, K>, &'a [u8], RequestRef, ResponseState<K>);
#[cfg(not(feature = "oscore"))]
pub(crate) type Opened<'a, K = Token> = (ParsedMessage<'a, K>, &'a [u8], Request, ResponseState<K>);

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
pub(crate) const fn empty_field<const REQUESTS: usize, K: OwnedToken>() -> Store<REQUESTS, 1, K> {
    Store::new()
}

#[cfg(feature = "oscore")]
pub(crate) fn is_active<const REQUESTS: usize, K: OwnedToken>(ctx: &Field<REQUESTS, K>) -> bool {
    ctx.is_some()
}

/// Request binding stored on an Observe interest (notifications).
pub(crate) fn request_from_interest<K: OwnedToken>(
    interest: crate::storage::ObserveInterest<K>,
) -> Request {
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
pub(crate) fn is_active<const REQUESTS: usize, K: OwnedToken>(_ctx: &Field<REQUESTS, K>) -> bool {
    false
}

/// After decode: unprotect an OSCORE message, or reject unprotected messages.
///
/// When a context is attached, a message without an OSCORE option is
/// fail-closed except empty ACK/RST (and other empty code-0.00 control
/// datagrams). Those are RFC 7252 reliability / ping; they carry no
/// application body. A token-matching unprotected 2.xx/4.xx/5.xx must
/// not complete a [`Call`](super::Call).
#[cfg(feature = "oscore")]
pub(crate) fn inbound<'a, const REQUESTS: usize, K: OwnedToken>(
    ctx: &mut Field<REQUESTS, K>,
    peer: Endpoint,
    parsed: &ParsedMessage<'_, K>,
    scratch: &'a mut [u8],
) -> Result<Option<Opened<'a, K>>, InboundError> {
    let Some(ctx) = selected_mut(ctx, peer.association())? else {
        return Ok(None);
    };
    if parsed.oscore().is_some() {
        let mut response_state = ResponseState::None;
        let (wire, request) = if parsed.code().is_request() {
            ctx.unprotect_request_wire(parsed, scratch)?
        } else {
            ctx.unprotect_bound_response_wire(parsed, scratch)?
        };
        let inner: ParsedMessage<'a, K> =
            crate::message::decode_with_token(wire).map_err(|error| match error {
                crate::message::TokenDecodeError::Malformed(error) => OscoreError::Parse(error),
                crate::message::TokenDecodeError::Capacity(_) => OscoreError::MessageLength,
            })?;
        if !parsed.code().is_request() {
            let header =
                crate::oscore::OscoreHeader::parse(parsed.oscore().ok_or(OscoreError::Header)?)?;
            let observe = inner.observe().is_some();
            if observe && ctx.observe_request(parsed.token()) != Some(request) {
                // Response to a non-Observe request must not carry Inner Observe.
                return Err(OscoreError::Replay);
            }
            if observe {
                // Piggybacked register ACK (`protect_response`, no Partial
                // IV) is bound to the request. Do not spend the RFC 8613
                // §7.4.1 no-PIV notification budget on it.
                let register_ack =
                    parsed.ty() == crate::message::Type::Acknowledgement && header.piv.is_none();
                if !register_ack {
                    ctx.notification_fresh(parsed.token(), header.piv)?;
                    response_state = ResponseState::Notification(parsed.token(), header.piv);
                }
            } else if inner.q_block2().next().is_some()
                || (inner.code() == crate::message::Code::REQUEST_ENTITY_INCOMPLETE
                    && inner.content_format()
                        == Some(Ok(crate::message::ContentFormat::MISSING_BLOCKS)))
            {
                // Q response sets and missing-block reports may keep the Call
                // active. Authenticate
                // before updating replay state; keep the binding until Call
                // completion/cancellation, including an out-of-order final block.
                if let Some(piv) = header.piv {
                    if !ctx.replay_fresh(piv.seq()) {
                        return Err(OscoreError::Replay);
                    }
                    response_state = ResponseState::Replay(piv.seq());
                } else {
                    // One no-PIV response is permitted for a request, including
                    // from independent peers. Others need fresh response PIVs.
                    if ctx.observe_request(parsed.token()) == Some(request) {
                        ctx.notification_fresh(parsed.token(), None)?;
                        response_state = ResponseState::Notification(parsed.token(), None);
                    } else {
                        ctx.response_without_piv_fresh(parsed.token())?;
                        response_state = ResponseState::ResponseWithoutPiv(parsed.token());
                    }
                }
            }
            // Authentication is not application admission. Unknown critical
            // options or a failed CON ACK can leave the Call pending. Retire
            // ordinary bindings when App collects/completes/cancels the Call,
            // or replace them when a continuation encodes a fresh request.
        }
        return Ok(Some((inner, wire, request, response_state)));
    }
    if parsed.is_empty() {
        return Ok(None);
    }
    Err(OscoreError::Unprotected)
}

#[cfg(not(feature = "oscore"))]
pub(crate) fn inbound<'a, const REQUESTS: usize, K: OwnedToken>(
    _ctx: &mut Field<REQUESTS, K>,
    peer: Endpoint,
    _parsed: &ParsedMessage<'_, K>,
    _scratch: &'a mut [u8],
) -> Result<Option<Opened<'a, K>>, InboundError> {
    let _ = peer;
    Ok(None)
}

/// Encode `msg`, or OSCORE-protect a request into `tx`.
///
/// Size failures stay [`EncodeError::BufferTooSmall`] so the client may
/// start Inner Block1. Other protect failures (feature `oscore`) are the
/// App `Oscore` error and must not look like a size miss.
pub(crate) fn encode_request<S: Storage + DatagramSlots, E, const REQUESTS: usize>(
    ctx: &mut Field<REQUESTS, S::Token>,
    peer: Endpoint,
    engine: &mut Engine<S>,
    tx: SlotId,
    msg: &Message<'_, S::Token>,
) -> Result<(), super::Error<E>> {
    let _ = (&ctx, peer);
    #[cfg(feature = "oscore")]
    if let Some(ctx) = selected_mut(ctx, peer.association()).map_err(super::Error::Oscore)? {
        return protect_into_tx(engine, tx, |out| {
            ctx.protect_request(msg, out).map_err(protect_err)
        });
    }
    engine
        .encode_tx(tx, msg)
        .map(|_| ())
        .map_err(super::Error::from)
}

/// Encode `msg`, or OSCORE-protect a response bound to `request`.
///
/// Fail-closed: an attached context without a stored [`RequestRef`] does
/// not fall back to unprotected traffic. Size misses stay
/// [`EncodeError::BufferTooSmall`] so the server may start Inner Block2.
pub(crate) fn encode_message<S: Storage + DatagramSlots, E, const REQUESTS: usize>(
    ctx: &Field<REQUESTS, S::Token>,
    peer: Endpoint,
    request: Request,
    engine: &mut Engine<S>,
    tx: SlotId,
    msg: &Message<'_, S::Token>,
) -> Result<(), super::Error<E>> {
    let _ = (ctx, request, peer);
    #[cfg(feature = "oscore")]
    if let Some(ctx) = selected(ctx, peer.association()).map_err(super::Error::Oscore)? {
        let Some(request) = request else {
            return Err(protect_err(OscoreError::Context));
        };
        return protect_into_tx(engine, tx, |out| {
            ctx.protect_response(msg, request, out).map_err(protect_err)
        });
    }
    engine
        .encode_tx(tx, msg)
        .map(|_| ())
        .map_err(super::Error::from)
}

/// Protect a notification with a new Partial IV (RFC 8613 §4.1.3.5.2).
///
/// Fail-closed: an attached context without a stored [`RequestRef`] does
/// not fall back to unprotected traffic.
pub(crate) fn encode_notification<S: Storage + DatagramSlots, E, const REQUESTS: usize>(
    ctx: &mut Field<REQUESTS, S::Token>,
    peer: Endpoint,
    request: Request,
    engine: &mut Engine<S>,
    tx: SlotId,
    msg: &Message<'_, S::Token>,
) -> Result<(), super::Error<E>> {
    #[cfg(not(feature = "oscore"))]
    let _ = (ctx, request, peer);
    #[cfg(feature = "oscore")]
    if ctx.is_some() {
        return protect_into_tx(engine, tx, |out| {
            stage_notification(ctx, peer, request, msg, out)
        });
    }
    engine
        .encode_tx(tx, msg)
        .map(|_| ())
        .map_err(super::Error::from)
}

pub(crate) fn stage_notification<E, const REQUESTS: usize, K: OwnedToken>(
    ctx: &mut Field<REQUESTS, K>,
    peer: Endpoint,
    request: Request,
    msg: &Message<'_, K>,
    out: &mut [u8],
) -> Result<usize, super::Error<E>> {
    #[cfg(not(feature = "oscore"))]
    let _ = (ctx, request, peer);
    #[cfg(feature = "oscore")]
    if let Some(oscore) = selected_mut(ctx, peer.association()).map_err(super::Error::Oscore)? {
        let Some(request) = request else {
            return Err(protect_err(OscoreError::Context));
        };
        return oscore
            .protect_response_with_piv(msg, request, out)
            .map_err(protect_err);
    }
    crate::message::encode(msg, out)
        .map_err(|error| super::Error::Message(SlotMessageError::Encode(error)))
}

#[cfg(feature = "oscore")]
fn protect_into_tx<S: Storage + DatagramSlots, E>(
    engine: &mut Engine<S>,
    tx: SlotId,
    encode: impl FnOnce(&mut [u8]) -> Result<usize, super::Error<E>>,
) -> Result<(), super::Error<E>> {
    let mut access = engine.access_tx_mut(tx).map_err(super::Error::from)?;
    let limit = access.capacity().min(S::TX_DATAGRAM_BYTES);
    if access.payload().is_empty() {
        // A fresh reservation has no retained packet to preserve on failure.
        // Commit its length only after protection and binding admission succeed.
        let n = encode(&mut access.bytes_mut()[..limit])?;
        return access.set_len(n).map_err(super::Error::from);
    }
    drop(access);
    protect_retained_tx(engine, tx, limit, encode)
}

#[cfg(feature = "oscore")]
#[cold]
#[inline(never)]
fn protect_retained_tx<S: Storage + DatagramSlots, E>(
    engine: &mut Engine<S>,
    tx: SlotId,
    limit: usize,
    encode: impl FnOnce(&mut [u8]) -> Result<usize, super::Error<E>>,
) -> Result<(), super::Error<E>> {
    // Replacing an occupied packet is transactional: an encode refusal must
    // leave its previous ciphertext available for retransmission.
    let mut wire = S::TxScratch::default();
    let limit = limit.min(wire.as_ref().len());
    let n = encode(&mut wire.as_mut()[..limit])?;
    fill_tx(engine, tx, &wire.as_ref()[..n]).map_err(super::Error::from)
}

#[cfg(feature = "oscore")]
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

/// Map a protect failure onto App encode (request, response, notification).
///
/// Only size misses become [`EncodeError::BufferTooSmall`] (Inner Block1 /
/// Block2 may start). Protocol failures (`SequenceExhausted`, `Encrypt`,
/// `Context`, …) stay [`super::Error::Oscore`] so they are not mistaken
/// for a too-large body.
#[cfg(feature = "oscore")]
fn protect_err<E>(err: OscoreError) -> super::Error<E> {
    match err {
        OscoreError::BufferTooSmall | OscoreError::MessageLength => {
            super::Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall))
        }
        OscoreError::Encode(e) => super::Error::Message(SlotMessageError::Encode(e)),
        OscoreError::Options => {
            super::Error::Message(SlotMessageError::Encode(EncodeError::OptionsFull))
        }
        other => super::Error::Oscore(other),
    }
}

/// Release a terminal client's request binding without resetting crypto state.
pub(crate) fn cancel<const REQUESTS: usize, K: OwnedToken>(
    ctx: &mut Field<REQUESTS, K>,
    peer: Endpoint,
    token: K,
) {
    #[cfg(feature = "oscore")]
    if let Some(ctx) = ctx.context_mut(peer.association()) {
        let _ = ctx.take(token);
    }
    #[cfg(not(feature = "oscore"))]
    let _ = (ctx, token, peer);
}

#[cfg(feature = "oscore")]
#[coaptic_io::dual_io(send_qblock1_recover_async, bound(crate::app::async_server::ProtocolIo), calls(super::finish_send => super::finish_send_async))]
pub(crate) fn send_qblock1_recover<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut super::AppIds,
    ctx: &mut Field<REQUESTS, S::Token>,
    now_ms: u64,
    recover: crate::storage::QBlockRecover<S::Token>,
) -> Result<(), super::Error<T::Error>>
where
    S: Storage
        + DatagramSlots
        + crate::storage::BodySlots
        + crate::storage::PendingCons
        + crate::storage::DedupSlots,
    T: crate::storage::DatagramIo,
{
    use crate::message::{Code, ContentFormat, Opt, Type};
    use crate::storage::BlockRole;
    let binding = engine
        .rx_body_transfer(recover.id())
        .filter(|transfer| {
            recover.role() == BlockRole::IncomingQBlock1
                && transfer.role() == BlockRole::IncomingQBlock1
                && transfer.key() == recover.key()
        })
        .and_then(|transfer| transfer.oscore_request);
    let Some((token, request)) = binding else {
        return Err(super::Error::Oscore(OscoreError::Unsupported));
    };
    let mut nums = [0; 16];
    let n = recover.copy_missing_nums(&mut nums);
    let response: super::Response<'_> = super::Response::missing_blocks(nums.into_iter().take(n));
    let format = ContentFormat::MISSING_BLOCKS.encode();
    let options = [Opt::content_format(&format)];
    let message = Message::new(
        Type::NonConfirmable,
        Code::REQUEST_ENTITY_INCOMPLETE,
        ids.next_for(engine, io, now_ms)?,
    )
    .with_token(token)
    .with_options(&options)
    .with_payload(response.payload());
    let tx = engine.acquire_tx().ok_or(super::Error::Saturated)?;
    let encoded = encode_notification(
        ctx,
        recover.key().endpoint(),
        Some(request),
        engine,
        tx,
        &message,
    );
    if let Err(error) = encoded {
        let _ = engine.release_tx(tx);
        return Err(error);
    }
    super::finish_send(engine, io, tx, recover.key().endpoint(), None)
}

#[cfg(all(test, feature = "alloc", feature = "oscore"))]
mod request_capacity_tests {
    use super::*;
    use crate::message::{Code, MessageId, Token, Type};
    use crate::storage::{AllocMemory, Capacities};

    #[test]
    fn request_protection_respects_actual_tx_before_replacing_binding() {
        let capacities = Capacities {
            rx_datagram_slots: 1,
            rx_datagram_bytes: 128,
            tx_datagram_slots: 1,
            tx_datagram_bytes: 128,
            dedup_entries: 1,
            observe_entries: 1,
            rx_body_slots: None,
            rx_body_bytes: None,
            tx_body_slots: None,
            tx_body_bytes: None,
        };
        assert!(
            <AllocMemory as DatagramSlots>::TxScratch::default()
                .as_ref()
                .len()
                > 128
        );
        let memory = AllocMemory::try_new(capacities, usize::MAX).unwrap();
        let mut engine = Engine::from_storage(memory);
        let mut ctx = Some(
            crate::oscore::SecurityContext::derive(crate::oscore::DeriveParams {
                master_secret: &[0x24; 16],
                master_salt: &[],
                sender_id: &[1],
                recipient_id: &[2],
                id_context: &[],
            })
            .unwrap(),
        );
        let token = Token::from_checked(&[7]);
        let short =
            Message::new(Type::Confirmable, Code::POST, MessageId::new(1)).with_token(token);
        let tx = engine.acquire_tx().unwrap();
        encode_request::<_, core::convert::Infallible, 4>(
            &mut ctx,
            Endpoint::v4([127, 0, 0, 1], 5683),
            &mut engine,
            tx,
            &short,
        )
        .unwrap();
        let original = ctx.as_ref().unwrap().lookup(token).unwrap();
        let before = ctx.as_ref().unwrap().sender_seq();
        let mut bytes = [0u8; 128];
        let retained = {
            let view = engine.access_tx(tx).unwrap();
            let body = view.as_bytes();
            bytes[..body.len()].copy_from_slice(body);
            body.len()
        };
        let large = Message::new(Type::Confirmable, Code::POST, MessageId::new(2))
            .with_token(token)
            .with_payload(&[0x5a; 200]);
        assert!(matches!(
            encode_request::<_, core::convert::Infallible, 4>(
                &mut ctx,
                Endpoint::v4([127, 0, 0, 1], 5683),
                &mut engine,
                tx,
                &large
            ),
            Err(super::super::Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall
            )))
        ));
        assert_eq!(ctx.as_ref().unwrap().lookup(token), Some(original));
        assert_eq!(ctx.as_ref().unwrap().sender_seq(), before + 1);
        assert_eq!(engine.access_tx(tx).unwrap().as_bytes(), &bytes[..retained]);
        encode_request::<_, core::convert::Infallible, 4>(
            &mut ctx,
            Endpoint::v4([127, 0, 0, 1], 5683),
            &mut engine,
            tx,
            &short,
        )
        .unwrap();
        assert_eq!(
            ctx.as_ref().unwrap().lookup(token).unwrap().piv().seq(),
            before + 1
        );
        engine.release_tx(tx).unwrap();
    }

    #[test]
    fn request_binding_refusal_preserves_retained_tx_and_leaves_fresh_tx_empty() {
        use crate::storage::{Memory, profiles};

        let mut engine = Engine::from_storage(Memory::<profiles::Constrained>::new());
        let mut ctx = Some(
            crate::oscore::BoundedSecurityContext::<1>::derive(crate::oscore::DeriveParams {
                master_secret: &[0x24; 16],
                master_salt: &[],
                sender_id: &[1],
                recipient_id: &[2],
                id_context: &[],
            })
            .unwrap(),
        );
        let peer = Endpoint::v4([127, 0, 0, 1], 5683);
        let first = Token::from_checked(&[1]);
        let second = Token::from_checked(&[2]);
        let message =
            Message::new(Type::Confirmable, Code::GET, MessageId::new(1)).with_token(first);
        let retained = engine.acquire_tx().unwrap();
        encode_request::<_, core::convert::Infallible, 1>(
            &mut ctx,
            peer,
            &mut engine,
            retained,
            &message,
        )
        .unwrap();
        let original = ctx.as_ref().unwrap().lookup(first).unwrap();
        let before = ctx.as_ref().unwrap().sender_seq();
        let mut bytes = [0; 64];
        let n = {
            let access = engine.access_tx(retained).unwrap();
            let n = access.as_bytes().len();
            bytes[..n].copy_from_slice(access.as_bytes());
            n
        };
        let fresh = engine.acquire_tx().unwrap();
        let refused =
            Message::new(Type::Confirmable, Code::GET, MessageId::new(2)).with_token(second);
        for (tx, spent) in [(retained, 1), (fresh, 2)] {
            assert_eq!(
                encode_request::<_, core::convert::Infallible, 1>(
                    &mut ctx,
                    peer,
                    &mut engine,
                    tx,
                    &refused,
                ),
                Err(super::super::Error::Oscore(OscoreError::Saturated))
            );
            assert_eq!(ctx.as_ref().unwrap().lookup(first), Some(original));
            assert_eq!(ctx.as_ref().unwrap().lookup(second), None);
            assert_eq!(ctx.as_ref().unwrap().sender_seq(), before + spent);
            assert_eq!(engine.access_tx(retained).unwrap().as_bytes(), &bytes[..n]);
            assert!(engine.access_tx(fresh).unwrap().as_bytes().is_empty());
        }
        assert_eq!(ctx.as_mut().unwrap().take(first), Some(original));
        encode_request::<_, core::convert::Infallible, 1>(
            &mut ctx,
            peer,
            &mut engine,
            fresh,
            &refused,
        )
        .unwrap();
        assert_eq!(
            ctx.as_ref().unwrap().lookup(second).unwrap().piv().seq(),
            before + 2
        );
        assert_eq!(engine.access_tx(retained).unwrap().as_bytes(), &bytes[..n]);
        assert!(!engine.access_tx(fresh).unwrap().as_bytes().is_empty());
        engine.release_tx(retained).unwrap();
        engine.release_tx(fresh).unwrap();
    }
}

/// Authenticate a valid over-capacity request without retaining its token.
#[cfg(feature = "oscore")]
#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_token_capacity_refusal_async, bound(crate::app::async_server::ProtocolIo), calls(super::finish_send => super::finish_send_async, super::send_empty_ack => super::send_empty_ack_async, super::replay_con_request => super::replay_con_request_async))]
pub(crate) fn send_token_capacity_refusal<S, T, const R: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut super::AppIds,
    now_ms: u64,
    peer: Endpoint,
    wire: &[u8],
    ctx: &mut Field<R, S::Token>,
    checkpoint: &mut CheckpointState,
    commit: &mut Option<&mut CheckpointCommit<'_>>,
    dedup_closed: &mut Option<super::DedupClosed>,
) -> Result<(), super::Error<T::Error>>
where
    S: Storage + DatagramSlots + crate::storage::PendingCons + crate::storage::DedupSlots,
    T: crate::storage::DatagramIo,
{
    use crate::message::{Code, NoResponse, Type};
    use crate::storage::DedupEntry;
    let Ok(raw) = crate::message::extended::decode(wire) else {
        return Ok(());
    };
    if !raw.code().is_request() || !matches!(raw.ty(), Type::Confirmable | Type::NonConfirmable) {
        return Ok(());
    }
    let Ok(projected) = crate::message::decode_without_token::<S::Token>(wire) else {
        return Ok(());
    };
    let Ok(peer) = ctx.inbound_route(peer, &projected) else {
        return Ok(());
    };
    if projected.ty() == Type::Confirmable
        && let super::Replay::Hit(outcome) =
            super::replay_con_request(engine, io, peer, &projected, now_ms, dedup_closed)
    {
        return outcome;
    }
    let Some(selected) = ctx.context_mut(peer.association()) else {
        return Ok(());
    };
    let before = selected.replay_checkpoint();
    let mut inner = S::RxScratch::default();
    let opened = selected.unprotect_request(&projected, inner.as_mut());
    if before != selected.replay_checkpoint() {
        checkpoint.invalidate();
    }
    checkpoint.flush(ctx, commit)?;
    let Ok((plain, request)) = opened else {
        return Ok(());
    };
    crate::storage::Metrics::inc(&mut engine.metrics_mut().token_capacity_refused);
    let suppress = plain
        .no_response()
        .and_then(Result::ok)
        .unwrap_or(NoResponse::DEFAULT)
        .suppresses(Code::BAD_REQUEST);
    if suppress {
        if raw.ty() == Type::Confirmable {
            super::remember_empty_ack(
                engine,
                peer,
                raw.message_id(),
                now_ms,
                plain.code(),
                dedup_closed,
            );
        }
        return if raw.ty() == Type::Confirmable {
            super::send_empty_ack(engine, io, peer, raw.message_id())
        } else {
            Ok(())
        };
    }
    let ty = if raw.ty() == Type::Confirmable {
        Type::Acknowledgement
    } else {
        Type::NonConfirmable
    };
    let mid = if ty == Type::Acknowledgement {
        raw.message_id()
    } else {
        ids.next_for(engine, io, now_ms)?
    };
    let response = Message::new(ty, Code::BAD_REQUEST, mid).with_token(S::Token::EMPTY);
    let mut protected = S::TxScratch::default();
    let Some(selected) = ctx.context(peer.association()) else {
        return Ok(());
    };
    let len = selected
        .protect_response(&response, request, protected.as_mut())
        .map_err(super::Error::Oscore)?;
    let encoded = crate::message::extended::decode(&protected.as_ref()[..len])
        .map_err(|error| super::Error::Message(SlotMessageError::Parse(error)))?;
    let tx = super::admission::control_tx(engine).ok_or(super::Error::Saturated)?;
    let result = (|| {
        let mut access = engine.access_tx_mut(tx)?;
        let len = encoded
            .with_token(raw.token())
            .encode(access.bytes_mut())
            .map_err(SlotMessageError::Encode)?;
        access.set_len(len)?;
        Ok::<_, SlotMessageError>(())
    })();
    if let Err(error) = result {
        let _ = engine.release_tx(tx);
        return Err(error.into());
    }
    if raw.ty() == Type::Confirmable {
        let entry = engine.access_tx(tx).ok().and_then(|access| {
            let bytes = access.as_bytes();
            (bytes.len() <= DedupEntry::REPLAY_MAX).then(|| {
                DedupEntry::new(mid, peer)
                    .with_due_ms(super::dedup_due_ms(now_ms))
                    .with_replay(bytes)
            })
        });
        if let Some(entry) = entry {
            super::note_dedup_store(engine, entry, plain.code(), dedup_closed);
        } else {
            super::remember_empty_ack(engine, peer, mid, now_ms, plain.code(), dedup_closed);
        }
    }
    super::finish_send(engine, io, tx, peer, None)
}
