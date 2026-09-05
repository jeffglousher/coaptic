//! [`Resource`]: owned instance with per-method hooks.

use crate::message::Code;

use super::reply::Reply;
use super::request::Request;

/// A CoAP resource moved into the site.
///
/// Implement the methods you serve. Unimplemented methods stay
/// [`Reply::method_not_allowed`]. [`App::at`](super::App::at) stores the
/// instance; [`App::poll`](super::App::poll) dispatches by request method.
///
/// ```
/// use coaptic::{Reply, Request, Resource};
///
/// struct Led { on: bool }
///
/// impl Resource for Led {
///     fn get(&mut self, _: &Request<'_>) -> Reply {
///         Reply::content(if self.on { b"on" } else { b"off" })
///     }
///     fn put(&mut self, req: &Request<'_>) -> Reply {
///         self.on = req.payload() == b"1";
///         Reply::changed()
///     }
/// }
/// ```
pub trait Resource: 'static {
    /// GET (0.01).
    fn get(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }

    /// POST (0.02).
    fn post(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }

    /// PUT (0.03).
    fn put(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }

    /// DELETE (0.04).
    fn delete(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }

    /// FETCH (0.05).
    fn fetch(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }

    /// PATCH (0.06).
    fn patch(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }

    /// iPATCH (0.07).
    fn ipatch(&mut self, _req: &Request<'_>) -> Reply {
        Reply::method_not_allowed()
    }
}

/// Request method (class 0 codes).
///
/// Unknown class-0 codes are not a [`Method`]. The site still answers 4.05
/// when the path exists.
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
