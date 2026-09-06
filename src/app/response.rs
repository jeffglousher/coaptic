//! [`Response`]: owned intent that [`App`](super::App) encodes into TX,
//! and the owned snapshot [`App::take_response`](super::App::take_response)
//! yields for a completed client [`Call`](super::Call).

use crate::message::{
    Code, ContentFormat, Echo, MessageId, MissingBlocks, ProblemDetails, Token, Type,
};
use crate::storage::Endpoint;

/// Bytes copied into a [`Response`] when the payload is not `'static`.
pub const INLINE_PAYLOAD: usize = 128;

/// Complete assembled client body copied into a [`Response`] (shipped profile RX body).
pub const RESPONSE_BODY: usize = 4096;

#[derive(Clone, Copy, Debug)]
enum Payload {
    Empty,
    Static(&'static [u8]),
    Inline {
        bytes: [u8; INLINE_PAYLOAD],
        len: u16,
    },
}

/// Conversion into a [`Response`].
///
/// Stored handlers are [`fn(Request<'_>) -> Response`](super::HandlerFn).
/// Implement this if a wrapper type should become a response.
pub trait IntoResponse {
    /// Build the response.
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

/// Owned response: handler intent, or a client snapshot from
/// [`App::take_response`](super::App::take_response).
///
/// Handlers return this. [`App::poll`](super::App::poll) writes a payload
/// that fits one datagram with `encode_tx`. A larger payload (within the
/// configured TX body capacity) is copied into a TX body area and shipped
/// as outgoing Block2, or Q-Block2 when that is the request's transfer.
/// Handlers do not opt in and do not see slot identifiers. The reactor
/// owns per-slot state machines inside that loop.
///
/// A completed client exchange is the same type: [`Self::code`] /
/// [`Self::payload`] / [`Self::body`] / [`Self::problem_details`] /
/// [`Self::missing_block_nums`] (and
/// [`Self::ty`] / [`Self::token`] / [`Self::peer`] when the snapshot
/// carried them). [`Self::payload`] is this datagram (truncated at
/// [`INLINE_PAYLOAD`]). [`Self::body`] is the assembled Block2 /
/// Q-Block2 body when present (truncated at [`RESPONSE_BODY`]).
///
/// ```
/// use coaptic::{ContentFormat, Response};
///
/// let response = Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN);
/// assert_eq!(response.code(), coaptic::Code::CONTENT);
/// assert_eq!(response.payload(), b"21.5");
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Response {
    code: Code,
    payload: Payload,
    content_format: Option<ContentFormat>,
    etag: [u8; 8],
    etag_len: u8,
    max_age: Option<u32>,
    observe: Option<u32>,
    ty: Option<Type>,
    token: Option<Token>,
    mid: Option<MessageId>,
    peer: Option<Endpoint>,
    body: [u8; RESPONSE_BODY],
    body_len: u16,
    has_body: bool,
    echo: Option<Echo>,
}

impl Response {
    /// Response with `code` and no payload.
    #[must_use]
    pub const fn new(code: Code) -> Self {
        Self {
            code,
            payload: Payload::Empty,
            content_format: None,
            etag: [0; 8],
            etag_len: 0,
            max_age: None,
            observe: None,
            ty: None,
            token: None,
            mid: None,
            peer: None,
            body: [0u8; RESPONSE_BODY],
            body_len: 0,
            has_body: false,
            echo: None,
        }
    }

    /// 2.05 Content with a `'static` payload.
    #[must_use]
    pub const fn content(payload: &'static [u8]) -> Self {
        Self::new(Code::CONTENT).with_static(payload)
    }

    /// 2.05 Content, copying `payload` into the inline buffer (truncated at
    /// [`INLINE_PAYLOAD`]).
    #[must_use]
    pub fn content_copy(payload: &[u8]) -> Self {
        Self::new(Code::CONTENT).payload_copy(payload)
    }

    /// 2.01 Created.
    #[must_use]
    pub const fn created() -> Self {
        Self::new(Code::CREATED)
    }

    /// 2.02 Deleted.
    #[must_use]
    pub const fn deleted() -> Self {
        Self::new(Code::DELETED)
    }

    /// 2.03 Valid.
    #[must_use]
    pub const fn valid() -> Self {
        Self::new(Code::VALID)
    }

    /// 2.04 Changed.
    #[must_use]
    pub const fn changed() -> Self {
        Self::new(Code::CHANGED)
    }

    /// 4.00 Bad Request.
    #[must_use]
    pub const fn bad_request() -> Self {
        Self::new(Code::BAD_REQUEST)
    }

    /// 4.01 Unauthorized.
    #[must_use]
    pub const fn unauthorized() -> Self {
        Self::new(Code::UNAUTHORIZED)
    }

    /// 4.03 Forbidden.
    #[must_use]
    pub const fn forbidden() -> Self {
        Self::new(Code::FORBIDDEN)
    }

    /// 4.04 Not Found.
    #[must_use]
    pub const fn not_found() -> Self {
        Self::new(Code::NOT_FOUND)
    }

    /// 4.05 Method Not Allowed.
    #[must_use]
    pub const fn method_not_allowed() -> Self {
        Self::new(Code::METHOD_NOT_ALLOWED)
    }

    /// 4.12 Precondition Failed.
    #[must_use]
    pub const fn precondition_failed() -> Self {
        Self::new(Code::PRECONDITION_FAILED)
    }

    /// 4.15 Unsupported Content-Format.
    #[must_use]
    pub const fn unsupported_content_format() -> Self {
        Self::new(Code::UNSUPPORTED_CONTENT_FORMAT)
    }

    /// 5.00 Internal Server Error.
    #[must_use]
    pub const fn internal_error() -> Self {
        Self::new(Code::INTERNAL_SERVER_ERROR)
    }

    /// 5.01 Not Implemented.
    #[must_use]
    pub const fn not_implemented() -> Self {
        Self::new(Code::NOT_IMPLEMENTED)
    }

    /// 5.03 Service Unavailable.
    #[must_use]
    pub const fn service_unavailable() -> Self {
        Self::new(Code::SERVICE_UNAVAILABLE)
    }

    /// Client or server error with RFC 9290 concise problem details (CBOR).
    ///
    /// Sets Content-Format to [`ContentFormat::PROBLEM_DETAILS`] (257) and a
    /// CBOR map that includes `response-code` (−4) equal to `code`. Title
    /// and detail are optional ([`Self::title`], [`Self::detail`]). Named
    /// builders such as [`Self::not_found`] stay empty; call this when a
    /// structured body is wanted. [`App`](super::App) uses this for
    /// generated 4.04 / 4.05, apply-error 4.08, and 4.01 when Echo
    /// freshness is on. Progress-driven Q-Block1 holes use
    /// [`Self::missing_blocks`] instead.
    ///
    /// Override the Content-Format with [`Self::content_format`] only when
    /// the peer asked for something else.
    ///
    /// ```
    /// use coaptic::{Code, ContentFormat, ProblemDetails, Response};
    ///
    /// let response = Response::problem(Code::NOT_FOUND).title("Not Found");
    /// assert_eq!(response.code(), Code::NOT_FOUND);
    /// assert_eq!(response.format(), Some(ContentFormat::PROBLEM_DETAILS));
    /// let details = ProblemDetails::decode(response.payload()).expect("cbor");
    /// assert_eq!(details.response_code(), Some(Code::NOT_FOUND));
    /// assert_eq!(details.title_text(), Some("Not Found"));
    /// ```
    #[must_use]
    pub fn problem(code: Code) -> Self {
        Self::new(code).with_problem(None, None)
    }

    /// 4.08 with RFC 9177 missing-blocks CBOR-seq (Content-Format 272).
    ///
    /// Encodes `nums` as a CBOR Sequence of unsigned integers (no array
    /// wrapper). [`App`](super::App) uses this for progress-driven Q-Block1
    /// holes. Apply errors and other 4.08 stay [`Self::problem`].
    ///
    /// ```
    /// use coaptic::{Code, ContentFormat, Response};
    ///
    /// let response = Response::missing_blocks([1, 9]);
    /// assert_eq!(response.code(), Code::REQUEST_ENTITY_INCOMPLETE);
    /// assert_eq!(response.format(), Some(ContentFormat::MISSING_BLOCKS));
    /// let mut nums = [0u32; 4];
    /// assert_eq!(response.missing_block_nums(&mut nums), Some(&[1, 9][..]));
    /// ```
    #[must_use]
    pub fn missing_blocks(nums: impl IntoIterator<Item = u32>) -> Self {
        let mut buf = [0u8; INLINE_PAYLOAD];
        match MissingBlocks::encode(nums, &mut buf) {
            Ok(n) => Self::new(Code::REQUEST_ENTITY_INCOMPLETE)
                .content_format(ContentFormat::MISSING_BLOCKS)
                .payload_copy(&buf[..n]),
            Err(_) => Self::new(Code::REQUEST_ENTITY_INCOMPLETE)
                .content_format(ContentFormat::MISSING_BLOCKS),
        }
    }

    /// Decode the payload as RFC 9177 missing-blocks NUMs when the
    /// Content-Format is [`ContentFormat::MISSING_BLOCKS`].
    #[must_use]
    pub fn missing_block_nums<'a>(&self, out: &'a mut [u32]) -> Option<&'a [u32]> {
        if self.content_format != Some(ContentFormat::MISSING_BLOCKS) {
            return None;
        }
        let n = MissingBlocks::decode(self.payload(), out).ok()?;
        Some(&out[..n])
    }

    /// Set the RFC 9290 title (−1) and keep (or set) problem-details CBOR.
    #[must_use]
    pub fn title(self, title: &str) -> Self {
        let mut detail_buf = [0u8; INLINE_PAYLOAD];
        let detail = copy_problem_field(self.problem_field(Field::Detail), &mut detail_buf);
        self.with_problem(Some(title), detail)
    }

    /// Set the RFC 9290 detail (−2) and keep (or set) problem-details CBOR.
    #[must_use]
    pub fn detail(self, detail: &str) -> Self {
        let mut title_buf = [0u8; INLINE_PAYLOAD];
        let title = copy_problem_field(self.problem_field(Field::Title), &mut title_buf);
        self.with_problem(title, Some(detail))
    }

    /// Decode the payload as RFC 9290 problem details when the Content-Format
    /// is [`ContentFormat::PROBLEM_DETAILS`].
    #[must_use]
    pub fn problem_details(&self) -> Option<ProblemDetails<'_>> {
        if self.content_format != Some(ContentFormat::PROBLEM_DETAILS) {
            return None;
        }
        ProblemDetails::decode(self.payload()).ok()
    }

    fn problem_field(&self, field: Field) -> Option<&str> {
        let parsed = ProblemDetails::decode(self.payload()).ok()?;
        match field {
            Field::Title => parsed.title_text(),
            Field::Detail => parsed.detail_text(),
        }
    }

    fn with_problem(mut self, title: Option<&str>, detail: Option<&str>) -> Self {
        self.content_format = Some(
            self.content_format
                .unwrap_or(ContentFormat::PROBLEM_DETAILS),
        );
        self.payload = encode_problem_payload(self.code, title, detail);
        self
    }

    /// Set Content-Format.
    #[must_use]
    pub const fn content_format(mut self, format: ContentFormat) -> Self {
        self.content_format = Some(format);
        self
    }

    /// Set ETag (1–8 bytes; longer values are truncated).
    #[must_use]
    pub fn etag(mut self, etag: &[u8]) -> Self {
        let n = etag.len().min(8);
        self.etag[..n].copy_from_slice(&etag[..n]);
        self.etag_len = n as u8;
        self
    }

    /// Set the Echo option (RFC 9175).
    ///
    /// [`App::poll`](super::App::poll) writes this on a freshness 4.01.
    #[must_use]
    pub const fn echo(mut self, echo: Echo) -> Self {
        self.echo = Some(echo);
        self
    }

    /// Echo option, if set.
    #[must_use]
    pub const fn echo_option(&self) -> Option<Echo> {
        self.echo
    }

    /// Set Max-Age in seconds.
    #[must_use]
    pub const fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// Set Observe sequence.
    ///
    /// On a successful GET/FETCH with Observe=0 this opts the handler into
    /// registration ([`App`](super::App) writes sequence 0 on the wire).
    /// [`App::notify`](super::App::notify) overwrites this with the table
    /// sequence.
    #[must_use]
    pub const fn observe(mut self, sequence: u32) -> Self {
        self.observe = Some(sequence);
        self
    }

    /// Replace the payload with a `'static` slice.
    #[must_use]
    pub const fn with_static(mut self, payload: &'static [u8]) -> Self {
        self.payload = if payload.is_empty() {
            Payload::Empty
        } else {
            Payload::Static(payload)
        };
        self
    }

    /// Replace the payload by copying into the inline buffer.
    #[must_use]
    pub fn payload_copy(mut self, payload: &[u8]) -> Self {
        let n = payload.len().min(INLINE_PAYLOAD);
        let mut bytes = [0u8; INLINE_PAYLOAD];
        bytes[..n].copy_from_slice(&payload[..n]);
        self.payload = if n == 0 {
            Payload::Empty
        } else {
            Payload::Inline {
                bytes,
                len: n as u16,
            }
        };
        self
    }

    /// Response code.
    #[must_use]
    pub const fn code(&self) -> Code {
        self.code
    }

    /// Payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        match self.payload {
            Payload::Empty => &[],
            Payload::Static(bytes) => bytes,
            Payload::Inline { ref bytes, len } => &bytes[..usize::from(len)],
        }
    }

    /// Content-Format, if set.
    #[must_use]
    pub const fn format(&self) -> Option<ContentFormat> {
        self.content_format
    }

    /// ETag, if set.
    #[must_use]
    pub fn etag_bytes(&self) -> Option<&[u8]> {
        if self.etag_len == 0 {
            None
        } else {
            Some(&self.etag[..usize::from(self.etag_len)])
        }
    }

    /// Max-Age seconds, if set.
    #[must_use]
    pub const fn max_age_secs(&self) -> Option<u32> {
        self.max_age
    }

    /// Observe sequence, if set (handler intent or a client snapshot).
    #[must_use]
    pub const fn observe_seq(&self) -> Option<u32> {
        self.observe
    }

    /// CON / NON / ACK / RST on a client snapshot, if [`App::take_response`](super::App::take_response)
    /// populated it.
    #[must_use]
    pub const fn ty(&self) -> Option<Type> {
        self.ty
    }

    /// Token on a client snapshot, if populated (same as [`Call::token`](super::Call::token)).
    #[must_use]
    pub const fn token(&self) -> Option<Token> {
        self.token
    }

    /// Message ID of the matched datagram on a client snapshot, if populated.
    #[must_use]
    pub const fn message_id(&self) -> Option<MessageId> {
        self.mid
    }

    /// Remote endpoint that sent a client-snapshot response, if populated.
    #[must_use]
    pub const fn peer(&self) -> Option<Endpoint> {
        self.peer
    }

    /// Complete assembled response body, if Block2 / Q-Block2 filled an RX body area.
    ///
    /// `None` when this is handler intent or a single-datagram client
    /// snapshot. Truncated at [`RESPONSE_BODY`] (Default / Constrained RX body bytes).
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

    pub(crate) fn from_client(
        code: Code,
        ty: Type,
        token: Token,
        mid: MessageId,
        peer: Endpoint,
        payload: &[u8],
        content_format: Option<ContentFormat>,
    ) -> Self {
        let mut response = Self::new(code).payload_copy(payload);
        response.content_format = content_format;
        response.ty = Some(ty);
        response.token = Some(token);
        response.mid = Some(mid);
        response.peer = Some(peer);
        response
    }

    pub(crate) fn copy_body(&mut self, src: &[u8]) {
        let n = src.len().min(RESPONSE_BODY);
        self.body[..n].copy_from_slice(&src[..n]);
        self.body_len = n as u16;
        self.has_body = true;
    }
}

#[derive(Clone, Copy)]
enum Field {
    Title,
    Detail,
}

fn copy_problem_field<'a>(src: Option<&str>, buf: &'a mut [u8]) -> Option<&'a str> {
    let text = src?;
    if text.len() > buf.len() {
        return None;
    }
    buf[..text.len()].copy_from_slice(text.as_bytes());
    core::str::from_utf8(&buf[..text.len()]).ok()
}

fn encode_problem_payload(code: Code, title: Option<&str>, detail: Option<&str>) -> Payload {
    let attempts = [(title, detail), (title, None), (None, None)];
    let mut buf = [0u8; INLINE_PAYLOAD];
    for (title, detail) in attempts {
        let mut details = ProblemDetails::new(code);
        if let Some(title) = title {
            details = details.title(title);
        }
        if let Some(detail) = detail {
            details = details.detail(detail);
        }
        if let Ok(n) = details.encode(&mut buf) {
            let mut bytes = [0u8; INLINE_PAYLOAD];
            bytes[..n].copy_from_slice(&buf[..n]);
            return Payload::Inline {
                bytes,
                len: n as u16,
            };
        }
    }
    Payload::Empty
}
