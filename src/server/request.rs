//! [`Request`]: decoded inbound request with path and peer.

use crate::message::{Code, MessageId, ParsedMessage, Token, Type};
use crate::storage::{Endpoint, SlotId};

use super::resource::Method;

/// Maximum Uri-Path segments stored on a [`Request`] or a route.
pub const MAX_PATH_SEGMENTS: usize = 8;

/// Decoded request the router and [`Resource`](super::Resource) see.
///
/// [`SlotId`] is available as [`Self::slot_id`] for Engine-level work.
/// Ordinary handlers do not need it.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    message: ParsedMessage<'a>,
    path: Path<'a>,
    peer: Endpoint,
    method: Option<Method>,
    slot: SlotId,
}

impl<'a> Request<'a> {
    pub(crate) fn from_decoded(
        message: ParsedMessage<'a>,
        peer: Endpoint,
        slot: SlotId,
    ) -> Result<Self, PathError> {
        let path = Path::from_message(&message)?;
        let method = Method::from_code(message.code());
        Ok(Self {
            message,
            path,
            peer,
            method,
            slot,
        })
    }

    /// Request method, if `code` is a known class-0 method.
    #[must_use]
    pub const fn method(&self) -> Option<Method> {
        self.method
    }

    /// Wire code (GET / PUT / …).
    #[must_use]
    pub const fn code(&self) -> Code {
        self.message.code()
    }

    /// Uri-Path segments in wire order (no leading slash).
    #[must_use]
    pub fn path(&self) -> &[&str] {
        self.path.segments()
    }

    /// Remote peer that sent this datagram.
    #[must_use]
    pub const fn peer(&self) -> Endpoint {
        self.peer
    }

    /// Request payload (empty if the 0xFF marker is absent).
    #[must_use]
    pub const fn payload(&self) -> &'a [u8] {
        self.message.payload()
    }

    /// Token (echo this on the reply).
    #[must_use]
    pub const fn token(&self) -> Token {
        self.message.token()
    }

    /// Message ID (echo this on a piggybacked ACK).
    #[must_use]
    pub const fn message_id(&self) -> MessageId {
        self.message.message_id()
    }

    /// CON / NON / ACK / RST.
    #[must_use]
    pub const fn ty(&self) -> Type {
        self.message.ty()
    }

    /// Full decoded datagram. Advanced: options, Observe, Block, …
    #[must_use]
    pub const fn message(&self) -> ParsedMessage<'a> {
        self.message
    }

    /// RX slot that holds this datagram. Advanced: Engine pin / body / Observe.
    #[must_use]
    pub const fn slot_id(&self) -> SlotId {
        self.slot
    }
}

/// Uri-Path segments, capped at [`MAX_PATH_SEGMENTS`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Path<'a> {
    segs: [&'a str; MAX_PATH_SEGMENTS],
    len: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathError {
    BadUtf8,
    TooLong,
}

impl<'a> Path<'a> {
    pub(crate) const fn empty() -> Self {
        Self {
            segs: [""; MAX_PATH_SEGMENTS],
            len: 0,
        }
    }

    pub(crate) fn from_segments(segments: &[&'a str]) -> Result<Self, PathError> {
        if segments.len() > MAX_PATH_SEGMENTS {
            return Err(PathError::TooLong);
        }
        let mut path = Self::empty();
        for (i, segment) in segments.iter().enumerate() {
            path.segs[i] = segment;
        }
        path.len = segments.len() as u8;
        Ok(path)
    }

    pub(crate) fn from_message(parsed: &ParsedMessage<'a>) -> Result<Self, PathError> {
        let mut path = Self::empty();
        for segment in parsed.uri_path() {
            let segment = segment.map_err(|_| PathError::BadUtf8)?;
            let i = usize::from(path.len);
            if i >= MAX_PATH_SEGMENTS {
                return Err(PathError::TooLong);
            }
            path.segs[i] = segment;
            path.len += 1;
        }
        Ok(path)
    }

    pub(crate) fn segments(&self) -> &[&'a str] {
        &self.segs[..usize::from(self.len)]
    }

    pub(crate) fn matches(self, other: &[&str]) -> bool {
        self.segments() == other
    }
}
