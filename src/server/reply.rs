//! [`Reply`]: response builders that [`Server`](super::Server) encodes.

use crate::message::{Code, ContentFormat};

/// Outgoing response. [`Server::poll`](super::Server::poll) encodes this
/// into a TX slot and sends it.
///
/// Builders such as [`Reply::content`] and [`Reply::not_found`] cover the
/// usual codes. Chain [`Self::with_content_format`], [`Self::with_etag`],
/// [`Self::with_max_age`], or [`Self::with_observe`] when needed.
#[derive(Clone, Copy, Debug)]
pub struct Reply<'a> {
    code: Code,
    payload: &'a [u8],
    content_format: Option<ContentFormat>,
    etag: Option<&'a [u8]>,
    max_age: Option<u32>,
    observe: Option<u32>,
}

impl<'a> Reply<'a> {
    /// Response with `code` and no payload.
    #[must_use]
    pub const fn new(code: Code) -> Self {
        Self {
            code,
            payload: &[],
            content_format: None,
            etag: None,
            max_age: None,
            observe: None,
        }
    }

    /// 2.05 Content with `payload`.
    #[must_use]
    pub const fn content(payload: &'a [u8]) -> Self {
        Self::new(Code::CONTENT).with_payload(payload)
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

    /// Replace the payload.
    #[must_use]
    pub const fn with_payload(mut self, payload: &'a [u8]) -> Self {
        self.payload = payload;
        self
    }

    /// Set Content-Format.
    #[must_use]
    pub const fn with_content_format(mut self, format: ContentFormat) -> Self {
        self.content_format = Some(format);
        self
    }

    /// Set ETag (1–8 bytes on the wire).
    #[must_use]
    pub const fn with_etag(mut self, etag: &'a [u8]) -> Self {
        self.etag = Some(etag);
        self
    }

    /// Set Max-Age in seconds.
    #[must_use]
    pub const fn with_max_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// Set Observe sequence. Notification send is Phase 2 (not in
    /// [`Server::poll`](super::Server::poll) yet).
    #[must_use]
    pub const fn with_observe(mut self, sequence: u32) -> Self {
        self.observe = Some(sequence);
        self
    }

    /// Response code.
    #[must_use]
    pub const fn code(&self) -> Code {
        self.code
    }

    /// Payload bytes.
    #[must_use]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// Content-Format, if set.
    #[must_use]
    pub const fn content_format(&self) -> Option<ContentFormat> {
        self.content_format
    }

    /// ETag, if set.
    #[must_use]
    pub const fn etag(&self) -> Option<&'a [u8]> {
        self.etag
    }

    /// Max-Age seconds, if set.
    #[must_use]
    pub const fn max_age(&self) -> Option<u32> {
        self.max_age
    }

    /// Observe sequence, if set.
    #[must_use]
    pub const fn observe(&self) -> Option<u32> {
        self.observe
    }
}
