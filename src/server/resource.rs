//! [`Resource`]: one type per resource, bound to a path and method.

use crate::message::Code;

use super::reply::Reply;
use super::request::Request;

/// A CoAP resource.
///
/// Implement this on a unit struct. [`Router::at`](super::Router::at)
/// `.get(Temp)` / `.put(Led)` uses the **type** only — the value is not
/// stored (no allocator, no `Box`).
///
/// The router calls [`Self::handle`] only for the method you bound. Use
/// [`Request::method`] if one type is bound to more than one method.
pub trait Resource {
    /// Build a [`Reply`] for this request.
    fn handle<'a>(request: &'a Request<'a>) -> Reply<'a>;
}

/// Request method (class 0 codes).
///
/// Unknown class-0 codes are not a [`Method`]. The router still answers
/// 4.05 when the path exists.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Method {
    /// GET (0.01).
    Get,
    /// POST (0.02).
    Post,
    /// PUT (0.03).
    Put,
    /// DELETE (0.04).
    Delete,
    /// FETCH (0.05).
    Fetch,
    /// PATCH (0.06).
    Patch,
    /// iPATCH (0.07).
    IPatch,
}

impl Method {
    /// Known request method for `code`, if any.
    #[must_use]
    pub const fn from_code(code: Code) -> Option<Self> {
        match code {
            Code::GET => Some(Self::Get),
            Code::POST => Some(Self::Post),
            Code::PUT => Some(Self::Put),
            Code::DELETE => Some(Self::Delete),
            Code::FETCH => Some(Self::Fetch),
            Code::PATCH => Some(Self::Patch),
            Code::IPATCH => Some(Self::IPatch),
            _ => None,
        }
    }

    /// Wire code for this method.
    #[must_use]
    pub const fn code(self) -> Code {
        match self {
            Self::Get => Code::GET,
            Self::Post => Code::POST,
            Self::Put => Code::PUT,
            Self::Delete => Code::DELETE,
            Self::Fetch => Code::FETCH,
            Self::Patch => Code::PATCH,
            Self::IPatch => Code::IPATCH,
        }
    }
}
