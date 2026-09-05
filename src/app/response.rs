//! [`Response`]: owned intent that [`App`](super::App) encodes into TX.

use crate::message::{Code, ContentFormat};

/// Bytes copied into a [`Response`] when the payload is not `'static`.
pub const INLINE_PAYLOAD: usize = 128;

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

/// Owned response intent. No lifetime — `'static` payload or a small inline copy.
///
/// Handlers return this. [`App::poll`](super::App::poll) writes it into a TX
/// datagram (`encode_tx`) and sends. Block-wise TX body start
/// (`start_block2` / Q-Block2) is Phase 2: a payload that does not fit one
/// datagram is still encoded as a single message today. The reactor owns
/// per-slot state machines inside that loop.
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

    /// Set Max-Age in seconds.
    #[must_use]
    pub const fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// Set Observe sequence. Notification send is Phase 2 (not in
    /// [`App::poll`](super::App::poll) yet).
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

    /// Observe sequence, if set.
    #[must_use]
    pub const fn observe_seq(&self) -> Option<u32> {
        self.observe
    }
}
