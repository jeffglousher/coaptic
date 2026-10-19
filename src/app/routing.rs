//! Method routers and handler fns (`Request` → `Response`).
//!
//! Taste is Axum (`get(h).put(p)`) and Ohkami (per-method bits on one path).
//! Stored handlers are fn pointers — no boxes, no allocator, no `unsafe`.
//! These are the **site** routers ([`get`], [`put`], …). Distinct from the
//! outbound builders [`App::get`](super::App::get) / [`App::put`](super::App::put).
//! Domain data that outlives a request stays outside [`App`](super::App).

use crate::message::{Code, OwnedToken, Token};

use super::request::Request;
use super::response::Response;

/// Function pointer stored in a [`MethodRouter`].
///
/// Default handlers are relatively stateless: `Request` → [`Response`].
pub type HandlerFn<T = Token> = fn(Request<'_, T>) -> Response<'static, T>;

/// Snapshot for a due Observe notification (`App::poll` / [`App::signal`](super::App::signal)).
///
/// Domain data stays outside `App`. This fn builds the current
/// representation when Engine pacing surfaces a notify.
pub type ObserveSource<T = Token> = fn() -> Response<'static, T>;

#[derive(Clone, Copy)]
pub(crate) enum SnapshotSource<T: OwnedToken = Token> {
    Plain(ObserveSource<T>),
    Selected(HandlerFn<T>),
}

impl<T: OwnedToken> SnapshotSource<T> {
    pub(crate) fn snapshot(
        self,
        interest: &crate::storage::ObserveInterest<T>,
        association: Option<super::Association>,
    ) -> Option<Response<'static, T>> {
        match self {
            Self::Plain(source) => Some(source()),
            Self::Selected(source) => {
                let selection = interest.selection()?;
                let parsed = crate::message::decode_with_token::<T>(
                    &selection.bytes[..usize::from(selection.len)],
                )
                .ok()?;
                let request = Request::from_decoded(parsed, interest.endpoint().physical(), None)
                    .ok()?
                    .with_association(association);
                Some(source(request))
            }
        }
    }
}

/// Per-path method table (Ohkami-style bits; Axum-style `.get().put()` chain).
///
/// Built with [`get`], [`put`], [`post`], [`delete`], [`fetch`], [`patch`],
/// [`ipatch`]. One route occupies one site slot regardless of how many
/// methods are set. Unknown method on a matching path is 4.05
/// ([`Response::problem`]).
///
/// ```
/// use coaptic::{Request, Response, get};
///
/// fn read(_: Request<'_>) -> Response<'static> { Response::content(b"ok") }
/// fn write(_: Request<'_>) -> Response<'static> { Response::changed() }
///
/// let methods = get(read).put(write);
/// # let _ = methods;
/// ```
#[derive(Clone, Copy)]
pub struct MethodRouter<T: OwnedToken = Token> {
    handlers: [Option<HandlerFn<T>>; METHOD_COUNT],
    observe: Option<SnapshotSource<T>>,
}

const METHOD_COUNT: usize = 7;

impl<T: OwnedToken> MethodRouter<T> {
    /// No methods. Unknown method on a matching path is 4.05 (RFC 9290 CBOR).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            handlers: [None; METHOD_COUNT],
            observe: None,
        }
    }

    /// Register a poll-time snapshot for Observe notifications.
    ///
    /// A successful queryless GET with Observe=0 that includes Observe on the
    /// [`Response`] still registers without this. This hook lets
    /// [`App::poll`](super::App::poll) encode a notification when Engine
    /// progress yields `observe_notify` (after [`App::signal`](super::App::signal)).
    /// This requestless source declines FETCH and query-bearing GET registration.
    /// Use [`Self::observe_selected`] to retain and supply each observer's selection.
    #[must_use]
    pub const fn observe(self, source: ObserveSource<T>) -> Self {
        let mut this = self;
        this.observe = Some(SnapshotSource::Plain(source));
        this
    }

    /// Register a selection-aware notification source for GET or FETCH.
    ///
    /// The callback receives retained method, path, ordered queries, Accept,
    /// Content-Format and FETCH payload, plus the relation's peer and association.
    /// Its Message ID/type are synthetic; it is a snapshot input, not new ingress.
    /// Encoded selection is bounded by `BlockTransfer::REQUEST_IDENTITY_BYTES`
    /// (512 bytes, including options and framing). Oversize selection declines
    /// Observe registration while returning the ordinary response. Retention
    /// belongs to the Observe row and ends with that relation.
    #[must_use]
    pub const fn observe_selected(mut self, source: HandlerFn<T>) -> Self {
        self.observe = Some(SnapshotSource::Selected(source));
        self
    }

    pub(crate) const fn observe_source(self) -> Option<SnapshotSource<T>> {
        self.observe
    }

    /// Bind GET (0.01).
    #[must_use]
    pub const fn get(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::Get, handler)
    }

    /// Bind POST (0.02).
    #[must_use]
    pub const fn post(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::Post, handler)
    }

    /// Bind PUT (0.03).
    #[must_use]
    pub const fn put(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::Put, handler)
    }

    /// Bind DELETE (0.04).
    #[must_use]
    pub const fn delete(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::Delete, handler)
    }

    /// Bind FETCH (0.05).
    #[must_use]
    pub const fn fetch(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::Fetch, handler)
    }

    /// Bind PATCH (0.06).
    #[must_use]
    pub const fn patch(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::Patch, handler)
    }

    /// Bind iPATCH (0.07).
    #[must_use]
    pub const fn ipatch(self, handler: HandlerFn<T>) -> Self {
        self.on(Method::IPatch, handler)
    }

    const fn on(mut self, method: Method, handler: HandlerFn<T>) -> Self {
        self.handlers[method.index()] = Some(handler);
        self
    }

    pub(crate) fn call(&self, method: Method, req: Request<'_, T>) -> Response<'static, T> {
        match self.handlers[method.index()] {
            Some(handler) => handler(req),
            None => Response::problem(Code::METHOD_NOT_ALLOWED).title("Method Not Allowed"),
        }
    }
}

impl<T: OwnedToken> Default for MethodRouter<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// GET (0.01) **site** router. Chain [`.put`](MethodRouter::put) and siblings.
///
/// Distinct from the outbound builder [`App::get`](super::App::get).
#[must_use]
pub const fn get<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().get(handler)
}

/// POST (0.02) router.
#[must_use]
pub const fn post<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().post(handler)
}

/// PUT (0.03) router.
#[must_use]
pub const fn put<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().put(handler)
}

/// DELETE (0.04) router.
#[must_use]
pub const fn delete<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().delete(handler)
}

/// FETCH (0.05) router.
#[must_use]
pub const fn fetch<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().fetch(handler)
}

/// PATCH (0.06) router.
#[must_use]
pub const fn patch<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().patch(handler)
}

/// iPATCH (0.07) router.
#[must_use]
pub const fn ipatch<T: OwnedToken>(handler: HandlerFn<T>) -> MethodRouter<T> {
    MethodRouter::new().ipatch(handler)
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

    const fn index(self) -> usize {
        match self {
            Self::Get => 0,
            Self::Post => 1,
            Self::Put => 2,
            Self::Delete => 3,
            Self::Fetch => 4,
            Self::Patch => 5,
            Self::IPatch => 6,
        }
    }
}
