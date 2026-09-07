//! Outbound App request: encode TX, match Token + peer, take a [`Response`].
//!
//! ```text
//! Outgoing --encode--> TX slot
//! poll matches Token + endpoint
//! RX --copy--> Response
//! Block2 / Q-Block2 --apply--> RX body --copy--> Response::body()
//! ```
//!
//! [`Call`] is Token plus peer — not a [`SlotId`](crate::storage::SlotId).
//! [`App::poll`](super::App::poll) advances this alongside site routing on
//! the same socket. Continues reuse the path recorded at
//! [`Outgoing::send`]. Observe subscribe ([`Outgoing::observe`]) keeps the
//! same [`Call`] after the Exchange is cleared.

use crate::error::{BlockTransferError, EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, Ids, Message, MessageId, Opt, OptionsBuilder, ParsedMessage,
    Token, Type, encode_uint,
};
use crate::storage::{
    BlockKey, BlockRole, BodySlots, DatagramIo, DatagramSlots, Endpoint, Engine, ExchangeKey,
    Exchanges, MemoryLayout, Missing, ObserveInterest, ObserveKey, ObserveResource, ObserveSlots,
    OutgoingBlock, PendingCons, Present, SlotId, Storage,
};

use super::request::{IntoPath, MAX_PATH_SEGMENTS, Path, PathError, path_from_into};
use super::response::{INLINE_PAYLOAD, Response};
use super::{App, Error, Method};

/// Outstanding client exchange (Token + destination).
///
/// Identity for [`App::take_response`](super::App::take_response). Not a
/// slot. After [`Outgoing::observe`], the same `Call` yields the initial
/// representation and later notifications.
///
/// ```
/// # use coaptic::storage::DatagramIo;
/// # use coaptic::{App, Endpoint, profiles};
/// # struct NullIo;
/// # impl DatagramIo for NullIo {
/// #     type Error = &'static str;
/// #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
/// #         Ok(None)
/// #     }
/// #     fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> { Ok(0) }
/// # }
/// let mut app = App::profile::<profiles::Default>()
///     .block_wise::<false>()
///     .bind(NullIo)
///     .unwrap();
/// let peer = Endpoint::v4([192, 0, 2, 2], 5683);
/// let call = app.get("sensors/temp").to(peer).send(0).unwrap();
/// app.poll(0).unwrap();
/// let response = app.take_response(call);
/// # let _ = response;
/// ```
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

/// How many completed client [`Response`] values [`App`](super::App) holds.
pub(crate) const RESPONSE_INBOX: usize = 4;

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
    observe: Option<u32>,
    etag: [u8; 8],
    etag_len: u8,
}

impl ReplyMeta {
    fn from_parsed(parsed: &ParsedMessage<'_>, peer: Endpoint) -> Self {
        let src = parsed.payload();
        let n = src.len().min(INLINE_PAYLOAD);
        let mut payload = [0u8; INLINE_PAYLOAD];
        payload[..n].copy_from_slice(&src[..n]);
        let mut etag = [0u8; 8];
        let etag_len = parsed
            .etag()
            .next()
            .map(|tag| {
                let n = tag.len().min(8);
                etag[..n].copy_from_slice(&tag[..n]);
                n as u8
            })
            .unwrap_or(0);
        Self {
            code: parsed.code(),
            ty: parsed.ty(),
            token: parsed.token(),
            mid: parsed.message_id(),
            peer,
            payload,
            payload_len: n as u16,
            content_format: parsed.content_format().and_then(Result::ok),
            observe: parsed.observe().and_then(Result::ok),
            etag,
            etag_len,
        }
    }

    fn into_response(self) -> Response {
        let mut response = Response::from_client(
            self.code,
            self.ty,
            self.token,
            self.mid,
            self.peer,
            &self.payload[..usize::from(self.payload_len)],
            self.content_format,
        );
        if self.etag_len > 0 {
            response = response.etag(&self.etag[..usize::from(self.etag_len)]);
        }
        match self.observe {
            Some(seq) => response.observe(seq),
            None => response,
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
    rows: [Option<InboxRow>; RESPONSE_INBOX],
    evict: u8,
}

impl ClientInbox {
    pub(crate) const fn new() -> Self {
        Self {
            rows: [None; RESPONSE_INBOX],
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
        self.evict = u8::try_from((i + 1) % RESPONSE_INBOX).unwrap_or(0);
        evicted
    }

    fn take(&mut self, call: Call) -> Option<(Response, Option<SlotId>)> {
        let i = self.rows.iter().position(|row| {
            row.as_ref()
                .is_some_and(|row| row.call.token == call.token && row.call.peer == call.peer)
        })?;
        self.rows[i]
            .take()
            .map(|row| (row.meta.into_response(), row.body))
    }
}

#[derive(Clone, Copy, Debug)]
struct LiveCall {
    call: Call,
    path: Path<'static>,
    code: Code,
    ty: Type,
    content_format: Option<ContentFormat>,
    observe: OutgoingObserve,
}

/// Path / type for an outstanding client request (Block1 / Block2 Continue).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientLives {
    rows: [Option<LiveCall>; RESPONSE_INBOX],
    evict: u8,
}

impl ClientLives {
    pub(crate) const fn new() -> Self {
        Self {
            rows: [None; RESPONSE_INBOX],
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
        self.evict = u8::try_from((i + 1) % RESPONSE_INBOX).unwrap_or(0);
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

    fn token_for(&self, path: Path<'static>, peer: Endpoint) -> Option<Token> {
        self.rows.iter().copied().find_map(|row| {
            row.filter(|row| row.call.peer == peer && row.path == path)
                .map(|row| row.call.token)
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutgoingObserve {
    Off,
    Register,
    Deregister,
}

/// Outbound request builder: [`App::get`](super::App::get) / [`App::put`](super::App::put).
///
/// Distinct from the site routers [`get`](super::get) / [`put`](super::put).
/// Chain [`.to`](Self::to), optional [`.payload`](Self::payload) /
/// [`.non`](Self::non) / [`.query`](Self::query) / [`.accept`](Self::accept) /
/// [`.etag`](Self::etag) / [`.if_match`](Self::if_match) /
/// [`.if_none_match`](Self::if_none_match) / [`.observe`](Self::observe) /
/// [`.deregister`](Self::deregister) / [`.block2`](Self::block2) /
/// [`.q_block1`](Self::q_block1) / [`.q_block2`](Self::q_block2), then
/// [`.send`](Self::send). Default type is CON. Path sugar is
/// [`super::IntoPath`] (`"sensors/temp"` or `&["sensors", "temp"]`).
///
/// ```
/// # use coaptic::storage::DatagramIo;
/// # use coaptic::{App, Endpoint, profiles};
/// # struct NullIo;
/// # impl DatagramIo for NullIo {
/// #     type Error = &'static str;
/// #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
/// #         Ok(None)
/// #     }
/// #     fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, Self::Error> { Ok(0) }
/// # }
/// let mut app = App::profile::<profiles::Default>()
///     .block_wise::<false>()
///     .bind(NullIo)
///     .unwrap();
/// let peer = Endpoint::v4([192, 0, 2, 2], 5683);
/// let call = app.get("sensors/temp").to(peer).send(0).unwrap();
/// app.poll(0).unwrap();
/// let _ = app.take_response(call);
/// ```
pub struct Outgoing<'a, P, T, const N: usize, Dest = Missing, const BLOCK_WISE: bool = false>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE>,
{
    app: &'a mut App<P, T, N, BLOCK_WISE>,
    code: Code,
    ty: Type,
    dest: Option<Endpoint>,
    path: Result<Path<'static>, PathError>,
    payload: &'a [u8],
    content_format: Option<ContentFormat>,
    accept: Option<ContentFormat>,
    etag: Option<&'a [u8]>,
    if_match: Option<&'a [u8]>,
    if_none_match: bool,
    queries: [&'a str; MAX_PATH_SEGMENTS],
    query_n: u8,
    q_block1: bool,
    q_block2: bool,
    block2: Option<BlockValue>,
    observe: OutgoingObserve,
    _dest: core::marker::PhantomData<Dest>,
}

impl<P, T, const N: usize, const BLOCK_WISE: bool> App<P, T, N, BLOCK_WISE>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE>,
{
    /// CON GET builder. Next: [`Outgoing::to`].
    ///
    /// Distinct from the site router [`get`](super::get).
    /// `path` is [`IntoPath`]: `"sensors/temp"` or `&["sensors", "temp"]`.
    #[must_use]
    pub fn get(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::Get, path)
    }

    /// CON PUT builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn put(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::Put, path)
    }

    /// CON POST builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn post(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::Post, path)
    }

    /// CON DELETE builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn delete(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::Delete, path)
    }

    /// CON FETCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn fetch(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::Fetch, path)
    }

    /// CON PATCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn patch(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::Patch, path)
    }

    /// CON iPATCH builder. Next: [`Outgoing::to`].
    #[must_use]
    pub fn ipatch(&mut self, path: impl IntoPath) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        self.request(Method::IPatch, path)
    }

    /// CON request builder for `method`. Next: [`Outgoing::to`].
    #[must_use]
    pub fn request(
        &mut self,
        method: Method,
        path: impl IntoPath,
    ) -> Outgoing<'_, P, T, N, Missing, BLOCK_WISE> {
        Outgoing {
            app: self,
            code: method.code(),
            ty: Type::Confirmable,
            dest: None,
            path: path_from_into(path),
            payload: &[],
            content_format: None,
            accept: None,
            etag: None,
            if_match: None,
            if_none_match: false,
            queries: [""; MAX_PATH_SEGMENTS],
            query_n: 0,
            q_block1: false,
            q_block2: false,
            block2: None,
            observe: OutgoingObserve::Off,
            _dest: core::marker::PhantomData,
        }
    }
}

impl<P, T, const N: usize, const BLOCK_WISE: bool> App<P, T, N, BLOCK_WISE>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE>,
{
    /// Take the matched [`Response`] for `call`, if [`App::poll`](Self::poll)
    /// has completed it.
    ///
    /// When Block2 / Q-Block2 assembled, copies the RX body into
    /// [`Response::body`] and releases that body slot. After
    /// [`Outgoing::observe`], the same `call` yields the initial
    /// representation and later notifications.
    pub fn take_response(&mut self, call: Call) -> Option<Response> {
        let (mut response, body) = self.inbox.take(call)?;
        if !client_observe_live(&self.engine, call) {
            self.lives.remove(call);
        }
        if let Some(id) = body {
            if let Some(bytes) = rx_body_payload(&self.engine, id) {
                response.copy_body(bytes);
            }
            release_rx_body(&mut self.engine, id);
        }
        Some(response)
    }
}

impl<'a, P, T, const N: usize, Dest, const BLOCK_WISE: bool> Outgoing<'a, P, T, N, Dest, BLOCK_WISE>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE>,
{
    /// Destination endpoint (Token matching uses this peer).
    #[must_use]
    pub fn to(self, peer: Endpoint) -> Outgoing<'a, P, T, N, Present, BLOCK_WISE> {
        Outgoing {
            app: self.app,
            code: self.code,
            ty: self.ty,
            dest: Some(peer),
            path: self.path,
            payload: self.payload,
            content_format: self.content_format,
            accept: self.accept,
            etag: self.etag,
            if_match: self.if_match,
            if_none_match: self.if_none_match,
            queries: self.queries,
            query_n: self.query_n,
            q_block1: self.q_block1,
            q_block2: self.q_block2,
            block2: self.block2,
            observe: self.observe,
            _dest: core::marker::PhantomData,
        }
    }

    /// Request body. A payload that does not fit one datagram is sent as
    /// Block1 (or Q-Block1 after [`.q_block1`](Self::q_block1)) when
    /// block-wise is enabled.
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

    /// Accept option.
    #[must_use]
    pub const fn accept(mut self, format: ContentFormat) -> Self {
        self.accept = Some(format);
        self
    }

    /// ETag option (conditional GET / validation).
    #[must_use]
    pub const fn etag(mut self, tag: &'a [u8]) -> Self {
        self.etag = Some(tag);
        self
    }

    /// If-Match option.
    #[must_use]
    pub const fn if_match(mut self, tag: &'a [u8]) -> Self {
        self.if_match = Some(tag);
        self
    }

    /// If-None-Match option.
    #[must_use]
    pub const fn if_none_match(mut self) -> Self {
        self.if_none_match = true;
        self
    }

    /// Append a Uri-Query value. Empty values and values past
    /// [`super::MAX_PATH_SEGMENTS`] are ignored.
    #[must_use]
    pub fn query(mut self, value: &'a str) -> Self {
        if value.is_empty() {
            return self;
        }
        let n = usize::from(self.query_n);
        if n < MAX_PATH_SEGMENTS {
            self.queries[n] = value;
            self.query_n += 1;
        }
        self
    }

    /// Ask for Block2 on the first GET (early size negotiation).
    #[must_use]
    pub const fn block2(mut self, block: BlockValue) -> Self {
        self.block2 = Some(block);
        self
    }

    /// Send as NON (no pending CON). Default is CON.
    #[must_use]
    pub const fn non(mut self) -> Self {
        self.ty = Type::NonConfirmable;
        self
    }

    /// Send a large body with Q-Block1 (windowed) instead of classic Block1.
    #[must_use]
    pub const fn q_block1(mut self) -> Self {
        self.q_block1 = true;
        self
    }

    /// Ask for Q-Block2 (NUM 0, SZX 1024). The peer may still use classic Block2.
    #[must_use]
    pub const fn q_block2(mut self) -> Self {
        self.q_block2 = true;
        self
    }

    /// GET/FETCH Observe=0 (register). Later notifications use the same
    /// [`Call`] / [`App::take_response`](App::take_response).
    #[must_use]
    pub const fn observe(mut self) -> Self {
        self.observe = OutgoingObserve::Register;
        self
    }

    /// GET/FETCH Observe=1 (deregister). Reuses the Token of an existing
    /// subscribe to the same path and peer when one is live.
    #[must_use]
    pub const fn deregister(mut self) -> Self {
        self.observe = OutgoingObserve::Deregister;
        self
    }
}

impl<P, T, const N: usize, const BLOCK_WISE: bool> Outgoing<'_, P, T, N, Present, BLOCK_WISE>
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE>,
    T: DatagramIo,
{
    /// Encode the request, record the Exchange, and send.
    ///
    /// `now_ms` starts CON RTO on Engine (caller clock; jitter is 0). Later
    /// [`App::poll`](App::poll) sends Due retransmits when that clock
    /// advances. Returns a [`Call`] for
    /// [`App::take_response`](App::take_response). Tokens and Message IDs are
    /// App counters — this crate does not call an OS RNG. A payload that does
    /// not fit one datagram starts Block1 / Q-Block1 when block-wise is on.
    pub fn send(self, now_ms: u64) -> Result<Call, Error<T::Error>> {
        let dest = self.dest.expect("typestate: to() was called");
        let path = self.path.map_err(|_| Error::Path)?;
        let observe = self.observe;
        let token = match observe {
            OutgoingObserve::Deregister => reuse_observe_token(self.app, path, dest),
            OutgoingObserve::Off | OutgoingObserve::Register => self.app.next_token(),
        };
        if observe == OutgoingObserve::Deregister {
            take_client_observe(&mut self.app.engine, ObserveKey::new(token, dest));
        }
        let queries = self.queries;
        let query_n = usize::from(self.query_n);
        let spec = ClientSend {
            dest,
            ty: self.ty,
            code: self.code,
            token,
            path: path.segments(),
            payload: self.payload,
            content_format: self.content_format,
            accept: self.accept,
            etag: self.etag,
            if_match: self.if_match,
            if_none_match: self.if_none_match,
            queries: &queries[..query_n],
            q_block1: self.q_block1,
            q_block2: self.q_block2,
            block2: self.block2,
            observe,
        };
        let call = send_client(
            &mut self.app.engine,
            &mut self.app.io,
            &mut self.app.ids,
            now_ms,
            spec,
        )?;
        self.app.lives.insert(LiveCall {
            call,
            path,
            code: self.code,
            ty: self.ty,
            content_format: self.content_format,
            observe,
        });
        Ok(call)
    }
}

impl<P: MemoryLayout<BLOCK_WISE>, T, const N: usize, const BLOCK_WISE: bool>
    App<P, T, N, BLOCK_WISE>
{
    fn next_token(&mut self) -> Token {
        self.tokens = self.tokens.wrapping_add(1);
        if self.tokens == 0 {
            self.tokens = 1;
        }
        Token::from_checked(&self.tokens.to_be_bytes())
    }
}

struct ClientSend<'a> {
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &'a [&'a str],
    payload: &'a [u8],
    content_format: Option<ContentFormat>,
    accept: Option<ContentFormat>,
    etag: Option<&'a [u8]>,
    if_match: Option<&'a [u8]>,
    if_none_match: bool,
    queries: &'a [&'a str],
    q_block1: bool,
    q_block2: bool,
    block2: Option<BlockValue>,
    observe: OutgoingObserve,
}

fn send_client<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    spec: ClientSend<'_>,
) -> Result<Call, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    if spec.path.len() > MAX_PATH_SEGMENTS {
        return Err(Error::Path);
    }
    let Some(tx) = engine.acquire_tx() else {
        return Err(Error::Saturated);
    };
    let mid = ids.next();
    let cf = spec.content_format.map(ContentFormat::encode);
    let acc = spec.accept.map(ContentFormat::encode);
    let q2 = spec
        .q_block2
        .then(|| BlockValue::new(0, false, BlockValue::SZX_MAX))
        .and_then(Result::ok)
        .map(BlockValue::encode);
    let b2 = spec.block2.map(BlockValue::encode);
    let mut opts = OptionsBuilder::<16>::new();
    if let Some(tag) = spec.if_match {
        let _ = opts.push(Opt::if_match(tag));
    }
    if let Some(tag) = spec.etag {
        let _ = opts.push(Opt::etag(tag));
    }
    if spec.if_none_match {
        let _ = opts.push(Opt::if_none_match());
    }
    match spec.observe {
        OutgoingObserve::Register => {
            let _ = opts.push(Opt::observe_register());
        }
        OutgoingObserve::Deregister => {
            let _ = opts.push(Opt::observe_deregister());
        }
        OutgoingObserve::Off => {}
    }
    for segment in spec.path {
        let _ = opts.push(Opt::uri_path(segment));
    }
    if let Some(ref encoded) = cf {
        let _ = opts.push(Opt::content_format(encoded));
    }
    for query in spec.queries {
        let _ = opts.push(Opt::uri_query(query));
    }
    if let Some(ref encoded) = acc {
        let _ = opts.push(Opt::accept(encoded));
    }
    if let Some(ref encoded) = b2 {
        let _ = opts.push(Opt::block2(encoded));
    }
    if let Some(ref encoded) = q2 {
        let _ = opts.push(Opt::q_block2(encoded));
    }
    let msg = Message::new(spec.ty, spec.code, mid)
        .with_token(spec.token)
        .with_options(opts.as_slice())
        .with_payload(spec.payload);
    match engine.encode_tx(tx, &msg) {
        Ok(_) => finish_client_send(engine, io, tx, spec.dest, spec.ty, now_ms, mid)
            .map(|()| Call::new(spec.token, spec.dest)),
        Err(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => send_client_block1(
            engine,
            io,
            ids,
            now_ms,
            spec.dest,
            spec.ty,
            spec.code,
            spec.token,
            spec.path,
            spec.payload,
            spec.content_format,
            spec.q_block1,
            tx,
        ),
        Err(e) => {
            let _ = engine.release_tx(tx);
            Err(Error::Message(e))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_client_block1<Mem, T>(
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
    q_block1: bool,
    tx: SlotId,
) -> Result<Call, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let key = BlockKey::new(token, dest);
    let started = if q_block1 {
        engine.start_q_block1(key, payload, BlockValue::SZX_MAX)
    } else {
        engine.start_block1(key, payload, BlockValue::SZX_MAX)
    };
    let body = match started {
        Ok(id) => id,
        Err(BlockTransferError::NoBodyPools) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Message(SlotMessageError::Encode(
                EncodeError::BufferTooSmall,
            )));
        }
        Err(e) => {
            let _ = engine.release_tx(tx);
            return Err(Error::Block(e));
        }
    };
    let outcome = if q_block1 {
        issue_q_block1_window(
            engine,
            io,
            ids,
            now_ms,
            dest,
            ty,
            code,
            token,
            path,
            content_format,
            body,
            Some(tx),
            None,
        )
        .map(|_| ())
    } else {
        issue_block1(
            engine,
            io,
            ids,
            now_ms,
            dest,
            ty,
            code,
            token,
            path,
            content_format,
            body,
            Some(tx),
        )
    };
    if outcome.is_err() {
        let _ = engine.release_tx_body(body);
    }
    outcome?;
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
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: DatagramIo,
{
    let via_exchange = matching_exchange(engine, parsed, peer);
    let via_observe = engine
        .lookup_observe(ObserveKey::new(parsed.token(), peer))
        .is_some();
    if via_exchange.is_none() && !via_observe {
        let _ = engine.release_rx(rx);
        return Ok(());
    }
    if parsed.ty() == Type::Confirmable {
        if let Err(e) = super::send_empty_ack(engine, io, peer, parsed.message_id()) {
            let _ = engine.release_rx(rx);
            return Err(e);
        }
    }
    // Piggybacked ACK shares the request MID. A separate CON/NON response
    // uses a new MID; stop RTO using the exchange's request MID.
    if let Some(entry) = via_exchange {
        if let Some(tx) = engine.take_pending_con(entry.message_id(), peer) {
            let _ = engine.release_tx(tx);
        }
    }
    if let Some(tx) = engine.take_pending_con(parsed.message_id(), peer) {
        let _ = engine.release_tx(tx);
    }

    if let Some(body) = engine.lookup_tx_body(BlockKey::new(parsed.token(), peer)) {
        if let Some(transfer) = engine.tx_body_transfer(body) {
            if matches!(
                transfer.role(),
                BlockRole::OutgoingBlock1 | BlockRole::OutgoingQBlock1
            ) {
                if parsed.code() == Code::CONTINUE {
                    let outcome =
                        continue_block1_tx(engine, io, lives, ids, now_ms, parsed, peer, body);
                    let _ = engine.release_rx(rx);
                    return outcome;
                }
                let _ = engine.release_tx_body(body);
            }
        }
    }

    match engine.apply_block2_rx(rx) {
        Ok(progress) if progress.complete() => {
            accept_client_observe(engine, lives, parsed, peer, now_ms, via_exchange.is_some());
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
            let _ = engine.note_q_receive(progress.id(), now_ms);
            accept_client_observe(engine, lives, parsed, peer, now_ms, via_exchange.is_some());
            finish_assembled(engine, inbox, parsed, peer, rx, progress.id());
            return Ok(());
        }
        Ok(progress) => {
            let _ = engine.note_q_receive(progress.id(), now_ms);
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

    accept_client_observe(engine, lives, parsed, peer, now_ms, via_exchange.is_some());
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
    Mem: Storage + Exchanges + BodySlots + ObserveSlots,
{
    take_exchange(engine, parsed, peer);
    let _ = engine.take_observe(ObserveKey::new(parsed.token(), peer));
    let key = BlockKey::new(parsed.token(), peer);
    if let Some(id) = engine.lookup_rx_body(key) {
        let _ = engine.release_rx_body(id);
    }
    if let Some(id) = engine.lookup_tx_body(key) {
        let _ = engine.release_tx_body(id);
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
        if live.observe == OutgoingObserve::Register {
            let _ = opts.push(Opt::observe_register());
        }
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

#[allow(clippy::too_many_arguments)]
fn continue_block1_tx<Mem, T>(
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
    let Some(transfer) = engine.tx_body_transfer(body) else {
        return Ok(());
    };
    if transfer.is_complete() {
        return Ok(());
    }
    let live = lives.get(Call::new(parsed.token(), peer));
    let ty = live.map(|live| live.ty).unwrap_or(Type::Confirmable);
    let code = live.map(|live| live.code).unwrap_or(Code::PUT);
    let path = live.map(|live| live.path);
    let content_format = live.and_then(|live| live.content_format);
    let segments: &[&str] = path.as_ref().map_or(&[], Path::segments);
    match transfer.role() {
        BlockRole::OutgoingBlock1 => {
            take_exchange(engine, parsed, peer);
            issue_block1(
                engine,
                io,
                ids,
                now_ms,
                peer,
                ty,
                code,
                parsed.token(),
                segments,
                content_format,
                body,
                None,
            )
        }
        BlockRole::OutgoingQBlock1 => {
            if let Some(Ok(q)) = parsed.q_block1() {
                engine.ack_q_block1(body, q.num()).map_err(Error::Block)?;
            }
            issue_q_block1_window(
                engine,
                io,
                ids,
                now_ms,
                peer,
                ty,
                code,
                parsed.token(),
                segments,
                content_format,
                body,
                None,
                Some((parsed.token(), peer)),
            )
            .map(|_| ())
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn issue_block1<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    content_format: Option<ContentFormat>,
    body: SlotId,
    tx: Option<SlotId>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let issued = engine.next_block1(body).map_err(Error::Block)?;
    send_block1_issued(
        engine,
        io,
        ids,
        now_ms,
        dest,
        ty,
        code,
        token,
        path,
        content_format,
        issued,
        false,
        tx,
    )
}

#[allow(clippy::too_many_arguments)]
fn issue_q_block1_window<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    dest: Endpoint,
    first_ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    content_format: Option<ContentFormat>,
    body: SlotId,
    mut tx: Option<SlotId>,
    take_first: Option<(Token, Endpoint)>,
) -> Result<bool, Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let mut sent = false;
    let mut extra = 0u16;
    loop {
        let issued = match engine.next_q_block1(body) {
            Ok(issued) => issued,
            Err(BlockTransferError::OutsideWindow) => return Ok(sent),
            Err(e) => return Err(Error::Block(e)),
        };
        if !sent {
            if let Some((tok, ep)) = take_first {
                let _ = engine.take_exchange(ExchangeKey::new(tok, ep));
            }
        }
        let ty = if extra == 0 {
            first_ty
        } else {
            Type::NonConfirmable
        };
        send_block1_issued(
            engine,
            io,
            ids,
            now_ms,
            dest,
            ty,
            code,
            token,
            path,
            content_format,
            issued,
            true,
            tx.take(),
        )?;
        sent = true;
        extra = extra.saturating_add(1);
        if issued.complete() {
            return Ok(true);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_block1_issued<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    ids: &mut Ids,
    now_ms: u64,
    dest: Endpoint,
    ty: Type,
    code: Code,
    token: Token,
    path: &[&str],
    content_format: Option<ContentFormat>,
    issued: OutgoingBlock,
    q_block1: bool,
    tx: Option<SlotId>,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots,
    T: DatagramIo,
{
    let mut chunk = [0u8; 1024];
    let n = copy_tx_range(engine, issued, &mut chunk).map_err(Error::Block)?;
    let size1 = engine
        .tx_body_transfer(issued.id())
        .map(|t| t.filled())
        .and_then(|len| u32::try_from(len).ok());
    let tx = match tx {
        Some(tx) => tx,
        None => engine.acquire_tx().ok_or(Error::Saturated)?,
    };
    let mid = ids.next();
    let cf = content_format.map(ContentFormat::encode);
    let blk = issued.block().encode();
    let size = size1.map(encode_uint);
    let mut opts = OptionsBuilder::<16>::new();
    for segment in path {
        let _ = opts.push(Opt::uri_path(segment));
    }
    if let Some(ref encoded) = cf {
        let _ = opts.push(Opt::content_format(encoded));
    }
    if q_block1 {
        let _ = opts.push(Opt::q_block1(&blk));
    } else {
        let _ = opts.push(Opt::block1(&blk));
    }
    if let Some(ref encoded) = size {
        let _ = opts.push(Opt::size1(encoded));
    }
    let msg = Message::new(ty, code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(&chunk[..n]);
    if let Err(e) = engine.encode_tx(tx, &msg) {
        let _ = engine.release_tx(tx);
        return Err(Error::Message(e));
    }
    finish_client_send(engine, io, tx, dest, ty, now_ms, mid)
}

fn finish_client_send<Mem, T>(
    engine: &mut Engine<Mem>,
    io: &mut T,
    tx: SlotId,
    dest: Endpoint,
    ty: Type,
    now_ms: u64,
    mid: MessageId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges,
    T: DatagramIo,
{
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
    match super::finish_send(engine, io, tx, dest, pending) {
        Ok(()) => Ok(()),
        Err(e) => {
            drop_exchange_for_tx(engine, tx);
            Err(e)
        }
    }
}

fn drop_exchange_for_tx<Mem: Storage + Exchanges>(engine: &mut Engine<Mem>, tx: SlotId) {
    let n = engine.capacities().tx_datagram_slots;
    for i in 0..n {
        let id = SlotId::from_index(i);
        let Some(entry) = engine.exchange_entry(id) else {
            continue;
        };
        if entry.tx_slot() != tx {
            continue;
        }
        let _ = engine.take_exchange(entry.key());
        return;
    }
}

fn copy_tx_range<Mem: Storage + BodySlots>(
    engine: &Engine<Mem>,
    issued: OutgoingBlock,
    dest: &mut [u8],
) -> Result<usize, BlockTransferError> {
    let payload = engine
        .tx_body_payload(issued.id())
        .ok_or(BlockTransferError::NoTransfer)?;
    let end = issued
        .offset()
        .checked_add(issued.len())
        .ok_or(BlockTransferError::Overflow)?;
    if end > payload.len() || issued.len() > dest.len() {
        return Err(BlockTransferError::Overflow);
    }
    dest[..issued.len()].copy_from_slice(&payload[issued.offset()..end]);
    Ok(issued.len())
}

fn rx_body_payload<Mem: Storage + BodySlots>(engine: &Engine<Mem>, id: SlotId) -> Option<&[u8]> {
    engine.rx_body_payload(id)
}

fn release_rx_body<Mem: Storage + BodySlots>(engine: &mut Engine<Mem>, id: SlotId) {
    let _ = engine.release_rx_body(id);
}

fn client_observe_live<Mem: Storage + ObserveSlots>(engine: &Engine<Mem>, call: Call) -> bool {
    let key = ObserveKey::new(call.token(), call.peer());
    engine.lookup_observe(key).is_some()
}

fn take_client_observe<Mem: Storage + ObserveSlots>(engine: &mut Engine<Mem>, key: ObserveKey) {
    let _ = engine.take_observe(key);
}

fn reuse_observe_token<P, T, const N: usize, const BLOCK_WISE: bool>(
    app: &mut App<P, T, N, BLOCK_WISE>,
    path: Path<'static>,
    dest: Endpoint,
) -> Token
where
    P: crate::storage::MemoryProfile + MemoryLayout<BLOCK_WISE>,
{
    if let Some(token) = app.lives.token_for(path, dest) {
        return token;
    }
    let resource = ObserveResource::from_path(path.segments());
    observe_token_on(&app.engine, resource, dest).unwrap_or_else(|| app.next_token())
}

fn observe_token_on<Mem>(
    engine: &Engine<Mem>,
    resource: ObserveResource,
    dest: Endpoint,
) -> Option<Token>
where
    Mem: Storage + ObserveSlots,
{
    let n = engine.capacities().observe_entries;
    (0..n).find_map(|i| {
        engine
            .observe_interest(SlotId::from_index(i))
            .and_then(|row| {
                (row.endpoint() == dest && row.resource() == resource).then_some(row.token())
            })
    })
}

fn accept_client_observe<Mem>(
    engine: &mut Engine<Mem>,
    lives: &ClientLives,
    parsed: &ParsedMessage<'_>,
    peer: Endpoint,
    now_ms: u64,
    via_exchange: bool,
) where
    Mem: Storage + ObserveSlots,
{
    let key = ObserveKey::new(parsed.token(), peer);
    let live = lives.get(Call::new(parsed.token(), peer));
    if live.is_some_and(|live| live.observe == OutgoingObserve::Deregister) {
        let _ = engine.take_observe(key);
        return;
    }
    if parsed.observe().is_some() && parsed.code().is_success() {
        let resource = live
            .map(|live| ObserveResource::from_path(live.path.segments()))
            .unwrap_or(ObserveResource::NONE);
        let _ = engine
            .insert_observe(ObserveInterest::new(parsed.token(), peer).with_resource(resource));
        let max_age = parsed
            .max_age()
            .and_then(Result::ok)
            .unwrap_or(super::DEFAULT_MAX_AGE_SECS);
        let _ = engine.refresh_observe_max_age(key, now_ms, max_age, None);
    } else if via_exchange {
        let _ = engine.take_observe(key);
    }
}

pub(crate) fn forget_exchange_tx<Mem>(engine: &mut Engine<Mem>, lives: &mut ClientLives, tx: SlotId)
where
    Mem: Storage + Exchanges + BodySlots + ObserveSlots,
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
        let _ = engine.take_observe(ObserveKey::new(key.token(), key.endpoint()));
        let block = BlockKey::new(key.token(), key.endpoint());
        if let Some(body) = engine.lookup_rx_body(block) {
            let _ = engine.release_rx_body(body);
        }
        if let Some(body) = engine.lookup_tx_body(block) {
            let _ = engine.release_tx_body(body);
        }
        lives.remove(Call::new(key.token(), key.endpoint()));
        return;
    }
}
