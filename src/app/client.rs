//! Outbound App request: encode TX, match Exchange, take a [`Reply`].
//!
//! ```text
//! Outgoing --encode--> TX slot
//! poll matches Token + endpoint (Exchange table)
//! RX datagram --copy--> Reply
//! ```
//!
//! [`Call`] is Token plus peer — not a [`SlotId`](crate::SlotId). [`App::poll`](super::App::poll)
//! advances this alongside site routing on the same Engine and socket.

use crate::message::{
    Code, ContentFormat, Ids, Message, MessageId, Opt, OptionsBuilder, ParsedMessage,
    ProblemDetails, Token, Type,
};
use crate::storage::{
    DatagramIo, DatagramSlots, Endpoint, Engine, Exchanges, Missing, PendingCons, Present, SlotId,
    Storage,
};

use super::request::{MAX_PATH_SEGMENTS, Path, PathError};
use super::response::INLINE_PAYLOAD;
use super::{App, Error, Method};

/// Outstanding client exchange (Token + destination).
///
/// Identity for [`App::take_reply`](super::App::take_reply). Not a slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Call {
    token: Token,
    peer: Endpoint,
}

impl Call {
    pub(crate) const fn new(token: Token, peer: Endpoint) -> Self {
        Self { token, peer }
    }

    /// Token sent on the request (and expected on the response).
    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    /// Destination of the request / source of the matched response.
    #[must_use]
    pub const fn peer(self) -> Endpoint {
        self.peer
    }
}

/// Matched client response. Code and payload; no [`SlotId`](crate::SlotId).
///
/// [`App::poll`](super::App::poll) copies the RX datagram into this value and
/// releases the slot. Payload is truncated at [`INLINE_PAYLOAD`] (same inline
/// cap as [`Response`](super::Response)).
#[derive(Clone, Copy, Debug)]
pub struct Reply {
    code: Code,
    ty: Type,
    token: Token,
    mid: MessageId,
    peer: Endpoint,
    payload: [u8; INLINE_PAYLOAD],
    payload_len: u16,
    content_format: Option<ContentFormat>,
}

impl Reply {
    fn from_parsed(parsed: &ParsedMessage<'_>, peer: Endpoint) -> Self {
        let src = parsed.payload();
        let n = src.len().min(INLINE_PAYLOAD);
        let mut payload = [0u8; INLINE_PAYLOAD];
        payload[..n].copy_from_slice(&src[..n]);
        Self {
            code: parsed.code(),
            ty: parsed.ty(),
            token: parsed.token(),
            mid: parsed.message_id(),
            peer,
            payload,
            payload_len: n as u16,
            content_format: parsed.content_format().and_then(Result::ok),
        }
    }

    /// Response code.
    #[must_use]
    pub const fn code(self) -> Code {
        self.code
    }

    /// CON / NON / ACK / RST on the matched datagram.
    #[must_use]
    pub const fn ty(self) -> Type {
        self.ty
    }

    /// Token (same as [`Call::token`]).
    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    /// Message ID of the matched datagram.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.mid
    }

    /// Remote endpoint that sent the response.
    #[must_use]
    pub const fn peer(self) -> Endpoint {
        self.peer
    }

    /// Payload bytes (truncated at [`INLINE_PAYLOAD`]).
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload[..usize::from(self.payload_len)]
    }

    /// Content-Format, if the response carried a well-formed option.
    #[must_use]
    pub const fn content_format(self) -> Option<ContentFormat> {
        self.content_format
    }

    /// RFC 9290 problem details when Content-Format is 257.
    #[must_use]
    pub fn problem_details(&self) -> Option<ProblemDetails<'_>> {
        if self.content_format != Some(ContentFormat::PROBLEM_DETAILS) {
            return None;
        }
        ProblemDetails::decode(self.payload()).ok()
    }
}

/// How many completed [`Reply`] values [`App`](super::App) holds.
pub(crate) const REPLY_INBOX: usize = 4;

#[derive(Clone, Copy, Debug)]
struct InboxRow {
    call: Call,
    reply: Reply,
}

/// Bounded completed-reply table (not a seventh memory area).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientInbox {
    rows: [Option<InboxRow>; REPLY_INBOX],
    evict: u8,
}

impl ClientInbox {
    pub(crate) const fn new() -> Self {
        Self {
            rows: [None; REPLY_INBOX],
            evict: 0,
        }
    }

    fn insert(&mut self, call: Call, reply: Reply) {
        if let Some(slot) = self.rows.iter_mut().find(|row| row.is_none()) {
            *slot = Some(InboxRow { call, reply });
            return;
        }
        let i = usize::from(self.evict);
        self.rows[i] = Some(InboxRow { call, reply });
        self.evict = u8::try_from((i + 1) % REPLY_INBOX).unwrap_or(0);
    }

    fn take(&mut self, call: Call) -> Option<Reply> {
        let i = self.rows.iter().position(|row| {
            row.as_ref()
                .is_some_and(|row| row.call.token == call.token && row.call.peer == call.peer)
        })?;
        self.rows[i].take().map(|row| row.reply)
    }
}

/// Outbound request builder: [`App::get`](super::App::get) / [`App::put`](super::App::put).
///
/// Chain [`.to`](Self::to), optional [`.payload`](Self::payload) /
/// [`.non`](Self::non), then [`.send`](Self::send). Default type is CON.
///
/// ```
/// # use coaptic::{App, DatagramIo, Endpoint, profiles};
/// # struct NullIo;
/// # impl DatagramIo for NullIo {
/// #     type Error = &'static str;
/// #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
/// #         Ok(None)
/// #     }
/// #     fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> { Ok(0) }
/// # }
/// let mut app = App::profile::<profiles::Default>()
///     .block_wise(false)
///     .bind(NullIo)
///     .unwrap();
/// let peer = Endpoint::v4([192, 0, 2, 2], 5683);
/// let call = app.get(&["sensors", "temp"]).to(peer).send(0).unwrap();
/// app.poll(0).unwrap();
/// let _ = app.take_reply(call);
/// ```
pub struct Outgoing<'a, P, T, const N: usize, Dest = Missing>
where
    P: crate::storage::MemoryProfile,
{
    app: &'a mut App<P, T, N>,
    code: Code,
    ty: Type,
    dest: Option<Endpoint>,
    path: Result<Path<'static>, PathError>,
    payload: &'a [u8],
    content_format: Option<ContentFormat>,
    _dest: core::marker::PhantomData<Dest>,
}

impl<P, T, const N: usize> App<P, T, N>
where
    P: crate::storage::MemoryProfile,
{
    /// CON GET builder. Next: [`Outgoing::to`].
    ///
    /// Distinct from the site router [`get`](super::get).
    #[must_use]
    pub fn get(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::Get, path)
    }

    /// CON PUT builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn put(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::Put, path)
    }

    /// CON POST builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn post(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::Post, path)
    }

    /// CON DELETE builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn delete(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::Delete, path)
    }

    /// CON FETCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn fetch(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::Fetch, path)
    }

    /// CON PATCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn patch(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::Patch, path)
    }

    /// CON iPATCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn ipatch(&mut self, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        self.request(Method::IPatch, path)
    }

    /// CON request builder for `method`. Next: [`Outgoing::to`].
    #[must_use]
    pub fn request(&mut self, method: Method, path: &[&'static str]) -> Outgoing<'_, P, T, N> {
        Outgoing {
            app: self,
            code: method.code(),
            ty: Type::Confirmable,
            dest: None,
            path: Path::from_segments(path),
            payload: &[],
            content_format: None,
            _dest: core::marker::PhantomData,
        }
    }

    /// Take the matched [`Reply`] for `call`, if [`App::poll`](Self::poll) has completed it.
    pub fn take_reply(&mut self, call: Call) -> Option<Reply> {
        self.inbox.take(call)
    }
}

impl<'a, P, T, const N: usize, Dest> Outgoing<'a, P, T, N, Dest>
where
    P: crate::storage::MemoryProfile,
{
    /// Destination endpoint (Token matching uses this peer).
    #[must_use]
    pub fn to(self, peer: Endpoint) -> Outgoing<'a, P, T, N, Present> {
        Outgoing {
            app: self.app,
            code: self.code,
            ty: self.ty,
            dest: Some(peer),
            path: self.path,
            payload: self.payload,
            content_format: self.content_format,
            _dest: core::marker::PhantomData,
        }
    }

    /// Datagram payload (this request; not Block1).
    #[must_use]
    pub const fn payload(mut self, payload: &'a [u8]) -> Self {
        self.payload = payload;
        self
    }

    /// Content-Format option.
    #[must_use]
    pub const fn content_format(mut self, format: ContentFormat) -> Self {
        self.content_format = Some(format);
        self
    }

    /// Send as NON (no pending CON). Default is CON.
    #[must_use]
    pub const fn non(mut self) -> Self {
        self.ty = Type::NonConfirmable;
        self
    }
}

impl<P, T, const N: usize> Outgoing<'_, P, T, N, Present>
where
    P: crate::storage::MemoryProfile,
    T: DatagramIo,
    crate::storage::Memory<P>: Storage + DatagramSlots + PendingCons + Exchanges,
    crate::storage::Memory<P, crate::storage::WithBodies<P>>:
        Storage + DatagramSlots + PendingCons + Exchanges,
{
    /// Encode the request, record the Exchange, and send.
    ///
    /// `now_ms` starts CON RTO (caller clock; jitter is 0). Returns a [`Call`]
    /// for [`App::take_reply`](App::take_reply). Tokens and Message IDs are
    /// App counters — this crate does not call an OS RNG.
    pub fn send(self, now_ms: u64) -> Result<Call, Error<T::Error>> {
        let dest = self.dest.expect("typestate: to() was called");
        let path = self.path.map_err(|_| Error::Path)?;
        let token = self.app.next_token();
        match &mut self.app.engine {
            super::EngineSlot::Datagram(engine) => send_client(
                engine,
                &mut self.app.io,
                &mut self.app.ids,
                now_ms,
                dest,
                self.ty,
                self.code,
                token,
                path.segments(),
                self.payload,
                self.content_format,
            ),
            super::EngineSlot::BlockWise(engine) => send_client(
                engine,
                &mut self.app.io,
                &mut self.app.ids,
                now_ms,
                dest,
                self.ty,
                self.code,
                token,
                path.segments(),
                self.payload,
                self.content_format,
            ),
        }
    }
}

impl<P: crate::storage::MemoryProfile, T, const N: usize> App<P, T, N> {
    fn next_token(&mut self) -> Token {
        self.tokens = self.tokens.wrapping_add(1);
        if self.tokens == 0 {
            self.tokens = 1;
        }
        Token::from_checked(&self.tokens.to_be_bytes())
    }
}

#[allow(clippy::too_many_arguments)]
fn send_client<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    payload: &[u8],
    content_format: Option<ContentFormat>,
) -> Result<Call, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges,
    T: DatagramIo,
{
    if path.len() > MAX_PATH_SEGMENTS {
        return Err(Error::Path);
    }
    let mut opts = OptionsBuilder::<16>::new();
    for segment in path {
        let _ = opts.push(Opt::uri_path(segment));
    }
    let cf = content_format.map(ContentFormat::encode);
    if let Some(ref encoded) = cf {
        let _ = opts.push(Opt::content_format(encoded));
    }
    let mid = ids.next();
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
    if let Err(e) = engine.encode_tx(tx, &msg) {
        let _ = engine.release_tx(tx);
        return Err(Error::Message(e));
    }
    match engine.record_request(tx, dest) {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Saturated);
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            return Err(e.into());
        }
    }
    let pending = (ty == Type::Confirmable).then_some((now_ms, mid));
    super::finish_send(engine, io, tx, dest, pending)?;
    Ok(Call::new(token, dest))
}

pub(crate) fn complete_client<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox,
    peer: Endpoint,
    parsed: &ParsedMessage<'_>,
    rx: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges,
    T: DatagramIo,
{
    let Some(_entry) = engine.match_response(parsed, peer) else {
        let _ = engine.release_rx(rx);
        return Ok(());
    };
    if let Some(tx) = engine.take_pending_con(parsed.message_id(), peer) {
        let _ = engine.release_tx(tx);
    }
    if parsed.ty() == Type::Confirmable {
        if let Err(e) = super::send_empty_ack(engine, io, peer, parsed.message_id()) {
            let _ = engine.release_rx(rx);
            return Err(e);
        }
    }
    inbox.insert(
        Call::new(parsed.token(), peer),
        Reply::from_parsed(parsed, peer),
    );
    let _ = engine.release_rx(rx);
    Ok(())
}

pub(crate) fn forget_exchange_tx<Mem: Storage + Exchanges>(engine: &mut Engine<Mem>, tx: SlotId) {
    let n = engine.capacities().tx_datagram_slots;
    for i in 0..n {
        let id = SlotId::from_index(i);
        let Some(entry) = engine.exchange_entry(id) else {
            continue;
        };
        if entry.tx_slot() == tx {
            let _ = engine.take_exchange(entry.key());
            return;
        }
    }
}
