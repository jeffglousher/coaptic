//! Outbound App request: encode TX, match Exchange, take a [`Reply`].
//!
//! ```text
//! Outgoing --encode--> TX slot
//! poll matches Token + endpoint (Exchange table)
//! RX datagram --copy--> Reply
//! Block2 / Q-Block2 --apply--> RX body --copy--> Reply::body()
//! ```
//!
//! [`Call`] is Token plus peer — not a [`SlotId`](crate::SlotId). [`App::poll`](super::App::poll)
//! advances this alongside site routing on the same Engine and socket.
//! Classic Block2 Continue and Q-Block2 window Continue reuse the path
//! recorded at [`Outgoing::send`]. Q-Block2 recover stays on `poll`.

use crate::error::BlockTransferError;
use crate::message::{
    BlockValue, Code, ContentFormat, Ids, Message, MessageId, Opt, OptionsBuilder, ParsedMessage,
    ProblemDetails, Token, Type,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, DatagramIo, DatagramSlots, Endpoint, Engine, ExchangeKey,
    Exchanges, Missing, PendingCons, Present, SlotId, Storage,
};

use super::request::{MAX_PATH_SEGMENTS, Path, PathError};
use super::response::INLINE_PAYLOAD;
use super::{App, Error, Method};

/// Complete assembled client body copied into [`Reply`] (shipped profile RX body).
pub const REPLY_BODY: usize = 4096;

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
/// [`App::poll`](super::App::poll) copies the last RX datagram into
/// [`Self::payload`] (truncated at [`INLINE_PAYLOAD`], same cap as
/// [`Response`](super::Response)) and releases that datagram slot.
/// When Block2 / Q-Block2 assembly completes, the RX body stays in the
/// Engine until [`App::take_reply`](super::App::take_reply), which copies
/// it into [`Self::body`] (truncated at [`REPLY_BODY`]).
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
    body: [u8; REPLY_BODY],
    body_len: u16,
    has_body: bool,
}

impl Reply {
    fn copy_body(&mut self, src: &[u8]) {
        let n = src.len().min(REPLY_BODY);
        self.body[..n].copy_from_slice(&src[..n]);
        self.body_len = n as u16;
        self.has_body = true;
    }

    /// Response code.
    #[must_use]
    pub const fn code(&self) -> Code {
        self.code
    }

    /// CON / NON / ACK / RST on the matched datagram.
    #[must_use]
    pub const fn ty(&self) -> Type {
        self.ty
    }

    /// Token (same as [`Call::token`]).
    #[must_use]
    pub const fn token(&self) -> Token {
        self.token
    }

    /// Message ID of the matched datagram.
    #[must_use]
    pub const fn message_id(&self) -> MessageId {
        self.mid
    }

    /// Remote endpoint that sent the response.
    #[must_use]
    pub const fn peer(&self) -> Endpoint {
        self.peer
    }

    /// This datagram's payload (truncated at [`INLINE_PAYLOAD`]).
    ///
    /// For a Block2 / Q-Block2 fragment this is the last block, not the
    /// assembled body. See [`Self::body`].
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload[..usize::from(self.payload_len)]
    }

    /// Complete assembled response body, if Block2 / Q-Block2 filled an RX body area.
    ///
    /// `None` when the matched datagram was not a completed block-wise body.
    /// Truncated at [`REPLY_BODY`] (Default / Constrained RX body bytes).
    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        if self.has_body {
            Some(&self.body[..usize::from(self.body_len)])
        } else {
            None
        }
    }

    /// Whether [`Self::body`] is present (including an empty complete body).
    #[must_use]
    pub const fn has_body(&self) -> bool {
        self.has_body
    }

    /// Content-Format, if the response carried a well-formed option.
    #[must_use]
    pub const fn content_format(&self) -> Option<ContentFormat> {
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
struct ReplyMeta {
    code: Code,
    ty: Type,
    token: Token,
    mid: MessageId,
    peer: Endpoint,
    payload: [u8; INLINE_PAYLOAD],
    payload_len: u16,
    content_format: Option<ContentFormat>,
}

impl ReplyMeta {
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

    fn into_reply(self) -> Reply {
        Reply {
            code: self.code,
            ty: self.ty,
            token: self.token,
            mid: self.mid,
            peer: self.peer,
            payload: self.payload,
            payload_len: self.payload_len,
            content_format: self.content_format,
            body: [0u8; REPLY_BODY],
            body_len: 0,
            has_body: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct InboxRow {
    call: Call,
    meta: ReplyMeta,
    body: Option<SlotId>,
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

    fn insert(&mut self, call: Call, meta: ReplyMeta, body: Option<SlotId>) -> Option<SlotId> {
        if let Some(slot) = self.rows.iter_mut().find(|row| row.is_none()) {
            *slot = Some(InboxRow { call, meta, body });
            return None;
        }
        let i = usize::from(self.evict);
        let evicted = self.rows[i].take().and_then(|row| row.body);
        self.rows[i] = Some(InboxRow { call, meta, body });
        self.evict = u8::try_from((i + 1) % REPLY_INBOX).unwrap_or(0);
        evicted
    }

    fn take(&mut self, call: Call) -> Option<(Reply, Option<SlotId>)> {
        let i = self.rows.iter().position(|row| {
            row.as_ref()
                .is_some_and(|row| row.call.token == call.token && row.call.peer == call.peer)
        })?;
        self.rows[i]
            .take()
            .map(|row| (row.meta.into_reply(), row.body))
    }
}

#[derive(Clone, Copy, Debug)]
struct LiveCall {
    call: Call,
    path: Path<'static>,
    code: Code,
    ty: Type,
}

/// Path / type for an outstanding client request (Block2 Continue).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientLives {
    rows: [Option<LiveCall>; REPLY_INBOX],
    evict: u8,
}

impl ClientLives {
    pub(crate) const fn new() -> Self {
        Self {
            rows: [None; REPLY_INBOX],
            evict: 0,
        }
    }

    fn insert(&mut self, live: LiveCall) {
        if let Some(existing) = self.rows.iter_mut().find(|row| {
            row.as_ref().is_some_and(|row| {
                row.call.token == live.call.token && row.call.peer == live.call.peer
            })
        }) {
            *existing = Some(live);
            return;
        }
        if let Some(slot) = self.rows.iter_mut().find(|row| row.is_none()) {
            *slot = Some(live);
            return;
        }
        let i = usize::from(self.evict);
        self.rows[i] = Some(live);
        self.evict = u8::try_from((i + 1) % REPLY_INBOX).unwrap_or(0);
    }

    fn get(&self, call: Call) -> Option<LiveCall> {
        self.rows.iter().copied().find_map(|row| {
            row.filter(|row| row.call.token == call.token && row.call.peer == call.peer)
        })
    }

    fn remove(&mut self, call: Call) {
        if let Some(slot) = self.rows.iter_mut().find(|row| {
            row.as_ref()
                .is_some_and(|row| row.call.token == call.token && row.call.peer == call.peer)
        }) {
            *slot = None;
        }
    }
}

/// Outbound request builder: [`App::get`](super::App::get) / [`App::put`](super::App::put).
///
/// Chain [`.to`](Self::to), optional [`.payload`](Self::payload) /
/// [`.non`](Self::non) / [`.q_block2`](Self::q_block2), then
/// [`.send`](Self::send). Default type is CON.
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
    q_block2: bool,
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
            q_block2: false,
            _dest: core::marker::PhantomData,
        }
    }
}

impl<P, T, const N: usize> App<P, T, N>
where
    P: crate::storage::MemoryProfile,
    crate::storage::Memory<P>: crate::storage::BodySlots,
    crate::storage::Memory<P, crate::storage::WithBodies<P>>: crate::storage::BodySlots,
{
    /// Take the matched [`Reply`] for `call`, if [`App::poll`](Self::poll) has completed it.
    ///
    /// When Block2 / Q-Block2 assembled, copies the RX body into
    /// [`Reply::body`] and releases that body slot.
    pub fn take_reply(&mut self, call: Call) -> Option<Reply> {
        let (mut reply, body) = self.inbox.take(call)?;
        self.lives.remove(call);
        if let Some(id) = body {
            if let Some(bytes) = rx_body_payload(&self.engine, id) {
                reply.copy_body(bytes);
            }
            release_rx_body(&mut self.engine, id);
        }
        Some(reply)
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
            q_block2: self.q_block2,
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

    /// Ask for Q-Block2 (NUM 0, SZX 1024). The peer may still use classic Block2.
    #[must_use]
    pub const fn q_block2(mut self) -> Self {
        self.q_block2 = true;
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
        let q_block2 = self.q_block2;
        let call = match &mut self.app.engine {
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
                q_block2,
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
                q_block2,
            ),
        }?;
        self.app.lives.insert(LiveCall {
            call,
            path,
            code: self.code,
            ty: self.ty,
        });
        Ok(call)
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
    q_block2: bool,
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
    let q2 = q_block2
        .then(|| BlockValue::new(0, false, BlockValue::SZX_MAX))
        .and_then(Result::ok)
        .map(BlockValue::encode);
    if let Some(ref encoded) = q2 {
        let _ = opts.push(Opt::q_block2(encoded));
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn complete_client<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    inbox: &mut ClientInbox,
    lives: &mut ClientLives,
    ids: &mut Ids,
    now_ms: u64,
    peer: Endpoint,
    parsed: &ParsedMessage<'_>,
    rx: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    if matching_exchange(engine, parsed, peer).is_none() {
        let _ = engine.release_rx(rx);
        return Ok(());
    }
    if parsed.ty() == Type::Confirmable {
        if let Err(e) = super::send_empty_ack(engine, io, peer, parsed.message_id()) {
            let _ = engine.release_rx(rx);
            return Err(e);
        }
    }
    if let Some(tx) = engine.take_pending_con(parsed.message_id(), peer) {
        let _ = engine.release_tx(tx);
    }

    match engine.apply_block2_rx(rx) {
        Ok(progress) if progress.complete() => {
            finish_assembled(engine, inbox, parsed, peer, rx, progress.id());
            return Ok(());
        }
        Ok(progress) => {
            take_exchange(engine, parsed, peer);
            let outcome =
                send_block2_continue(engine, io, lives, ids, now_ms, parsed, peer, progress.id());
            let _ = engine.release_rx(rx);
            return outcome;
        }
        Err(BlockTransferError::MissingBlock | BlockTransferError::NoBodyPools) => {}
        Err(e) => {
            drop_client(engine, lives, parsed, peer, rx);
            let _ = e;
            return Ok(());
        }
    }

    match engine.apply_q_block2_rx(rx) {
        Ok(progress) if progress.complete() => {
            finish_assembled(engine, inbox, parsed, peer, rx, progress.id());
            return Ok(());
        }
        Ok(progress) => {
            let outcome = if needs_q_continue(engine, progress.id()) {
                take_exchange(engine, parsed, peer);
                send_q_block2_continue(engine, io, lives, ids, now_ms, parsed, peer, progress.id())
            } else {
                Ok(())
            };
            let _ = engine.release_rx(rx);
            return outcome;
        }
        Err(BlockTransferError::MissingBlock | BlockTransferError::NoBodyPools) => {}
        Err(_) => {
            drop_client(engine, lives, parsed, peer, rx);
            return Ok(());
        }
    }

    take_exchange(engine, parsed, peer);
    let evicted = inbox.insert(
        Call::new(parsed.token(), peer),
        ReplyMeta::from_parsed(parsed, peer),
        None,
    );
    if let Some(id) = evicted {
        let _ = engine.release_rx_body(id);
    }
    let _ = engine.release_rx(rx);
    Ok(())
}

fn finish_assembled<Mem>(
    engine: &mut Engine<Mem>,
    inbox: &mut ClientInbox,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    rx: SlotId,
    body: SlotId,
) where
    Mem: Storage + DatagramSlots + Exchanges + BodySlots,
{
    take_exchange(engine, parsed, peer);
    let evicted = inbox.insert(
        Call::new(parsed.token(), peer),
        ReplyMeta::from_parsed(parsed, peer),
        Some(body),
    );
    if let Some(id) = evicted {
        let _ = engine.release_rx_body(id);
    }
    let _ = engine.release_rx(rx);
}

fn matching_exchange<Mem: Storage + Exchanges>(
    engine: &Engine<Mem>,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
) -> Option<crate::storage::ExchangeEntry> {
    let id = engine.lookup_exchange(ExchangeKey::new(parsed.token(), peer))?;
    let entry = engine.exchange_entry(id)?;
    entry.matches_response(parsed).then_some(entry)
}

fn take_exchange<Mem: Storage + Exchanges>(
    engine: &mut Engine<Mem>,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
) {
    let _ = engine.take_exchange(ExchangeKey::new(parsed.token(), peer));
}

fn drop_client<Mem>(
    engine: &mut Engine<Mem>,
    lives: &mut ClientLives,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    rx: SlotId,
) where
    Mem: Storage + Exchanges + BodySlots,
{
    take_exchange(engine, parsed, peer);
    if let Some(id) = engine.lookup_rx_body(BlockKey::new(parsed.token(), peer)) {
        let _ = engine.release_rx_body(id);
    }
    lives.remove(Call::new(parsed.token(), peer));
    let _ = engine.release_rx(rx);
}

fn needs_q_continue<Mem: Storage + BodySlots>(engine: &Engine<Mem>, id: SlotId) -> bool {
    let Some(transfer) = engine.rx_body_transfer(id) else {
        return false;
    };
    transfer.role() == BlockRole::IncomingQBlock2
        && !transfer.is_complete()
        && transfer.q_holes().is_none()
        && transfer.window_mask() == 0
        && transfer.window_base() > 0
}

#[allow(clippy::too_many_arguments)]
fn send_block2_continue<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives,
    ids: &mut Ids,
    now_ms: u64,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    body: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let Some(transfer) = engine.rx_body_transfer(body) else {
        return Ok(());
    };
    let Ok(block) = BlockValue::new(transfer.next_num(), false, transfer.szx()) else {
        return Ok(());
    };
    send_followup(
        engine,
        io,
        lives,
        ids,
        now_ms,
        parsed.token(),
        peer,
        Some(block),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn send_q_block2_continue<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives,
    ids: &mut Ids,
    now_ms: u64,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    body: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let Some(transfer) = engine.rx_body_transfer(body) else {
        return Ok(());
    };
    let Ok(block) = BlockValue::new(transfer.window_base(), true, transfer.szx()) else {
        return Ok(());
    };
    send_followup(
        engine,
        io,
        lives,
        ids,
        now_ms,
        parsed.token(),
        peer,
        None,
        Some(block),
    )
}

#[allow(clippy::too_many_arguments)]
fn send_followup<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    lives: &ClientLives,
    ids: &mut Ids,
    now_ms: u64,
    token: Token,
    peer: Endpoint,
    block2: Option<BlockValue>,
    q_block2: Option<BlockValue>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges,
    T: DatagramIo,
{
    let live = lives.get(Call::new(token, peer));
    let ty = live.map(|live| live.ty).unwrap_or(Type::Confirmable);
    let code = live.map(|live| live.code).unwrap_or(Code::GET);
    let mut opts = OptionsBuilder::<16>::new();
    if let Some(live) = live {
        for segment in live.path.segments() {
            let _ = opts.push(Opt::uri_path(segment));
        }
    }
    let b2 = block2.map(BlockValue::encode);
    if let Some(ref encoded) = b2 {
        let _ = opts.push(Opt::block2(encoded));
    }
    let q2 = q_block2.map(BlockValue::encode);
    if let Some(ref encoded) = q2 {
        let _ = opts.push(Opt::q_block2(encoded));
    }
    let mid = ids.next();
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice());
    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
    if let Err(e) = engine.encode_tx(tx, &msg) {
        let _ = engine.release_tx(tx);
        return Err(Error::Message(e));
    }
    match engine.record_request(tx, peer) {
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
    super::finish_send(engine, io, tx, peer, pending)?;
    Ok(())
}

fn rx_body_payload<P>(engine: &super::EngineSlot<P>, id: SlotId) -> Option<&[u8]>
where
    P: crate::storage::MemoryProfile,
    crate::storage::Memory<P>: BodySlots,
    crate::storage::Memory<P, crate::storage::WithBodies<P>>: BodySlots,
{
    match engine {
        super::EngineSlot::Datagram(engine) => engine.rx_body_payload(id),
        super::EngineSlot::BlockWise(engine) => engine.rx_body_payload(id),
    }
}

fn release_rx_body<P>(engine: &mut super::EngineSlot<P>, id: SlotId)
where
    P: crate::storage::MemoryProfile,
    crate::storage::Memory<P>: BodySlots,
    crate::storage::Memory<P, crate::storage::WithBodies<P>>: BodySlots,
{
    match engine {
        super::EngineSlot::Datagram(engine) => {
            let _ = engine.release_rx_body(id);
        }
        super::EngineSlot::BlockWise(engine) => {
            let _ = engine.release_rx_body(id);
        }
    }
}

pub(crate) fn forget_exchange_tx<Mem>(engine: &mut Engine<Mem>, lives: &mut ClientLives, tx: SlotId)
where
    Mem: Storage + Exchanges + BodySlots,
{
    let n = engine.capacities().tx_datagram_slots;
    for i in 0..n {
        let id = SlotId::from_index(i);
        let Some(entry) = engine.exchange_entry(id) else {
            continue;
        };
        if entry.tx_slot() != tx {
            continue;
        }
        let key = entry.key();
        let _ = engine.take_exchange(key);
        if let Some(body) = engine.lookup_rx_body(BlockKey::new(key.token(), key.endpoint())) {
            let _ = engine.release_rx_body(body);
        }
        lives.remove(Call::new(key.token(), key.endpoint()));
        return;
    }
}
