//! Method routers, handler fns, and [`State`].
//!
//! Taste is Axum (`get(h).put(p)`) and Ohkami (per-method bits on one path).
//! Stored handlers are fn pointers — no boxes, no allocator, no `unsafe`.

use crate::message::Code;

use super::reply::Reply;
use super::request::Request;

/// Function pointer stored in a [`MethodRouter`].
///
/// [`App`](super::App) owns one `S` and passes [`State`]`<&mut S>` on each call.
pub type HandlerFn<S> = fn(State<&mut S>, Request<'_>) -> Reply;

/// Shared application state passed into a handler.
///
/// [`App`](super::App) owns one `T`. Dispatch borrows it as [`State`]`<&mut T>` —
/// no `Arc`, no allocator. Destructure (`State(s)`) to get `&mut T`.
///
/// ```
/// use coaptic::{Reply, Request, State};
///
/// struct Sensors {
///     temp_c: i16,
/// }
///
/// fn get_temp(State(s): State<&mut Sensors>, _req: Request<'_>) -> Reply {
///     let _ = s.temp_c;
///     Reply::content(b"21.5")
/// }
/// # let _ = get_temp;
/// ```
pub struct State<T>(pub T);

impl<T> State<T> {
    /// Wrap `inner`.
    #[must_use]
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }

    /// Unwrap.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// Per-path method table (Ohkami-style bits; Axum-style `.get().put()` chain).
///
/// Built with [`get`], [`put`], [`post`], [`delete`], [`fetch`], [`patch`],
/// [`ipatch`]. One route occupies one site slot regardless of how many
/// methods are set.
pub struct MethodRouter<S> {
    handlers: [Option<HandlerFn<S>>; METHOD_COUNT],
}

const METHOD_COUNT: usize = 7;

impl<S> MethodRouter<S> {
    /// No methods. Unknown method on a matching path is 4.05.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            handlers: [None; METHOD_COUNT],
        }
    }

    /// Bind GET (0.01).
    #[must_use]
    pub const fn get(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::Get, handler)
    }

    /// Bind POST (0.02).
    #[must_use]
    pub const fn post(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::Post, handler)
    }

    /// Bind PUT (0.03).
    #[must_use]
    pub const fn put(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::Put, handler)
    }

    /// Bind DELETE (0.04).
    #[must_use]
    pub const fn delete(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::Delete, handler)
    }

    /// Bind FETCH (0.05).
    #[must_use]
    pub const fn fetch(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::Fetch, handler)
    }

    /// Bind PATCH (0.06).
    #[must_use]
    pub const fn patch(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::Patch, handler)
    }

    /// Bind iPATCH (0.07).
    #[must_use]
    pub const fn ipatch(self, handler: HandlerFn<S>) -> Self {
        self.on(Method::IPatch, handler)
    }

    const fn on(mut self, method: Method, handler: HandlerFn<S>) -> Self {
        self.handlers[method.index()] = Some(handler);
        self
    }

    pub(crate) fn call(&self, method: Method, state: &mut S, req: Request<'_>) -> Reply {
        match self.handlers[method.index()] {
            Some(handler) => handler(State(state), req),
            None => Reply::method_not_allowed(),
        }
    }
}

impl<S> Default for MethodRouter<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Clone for MethodRouter<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for MethodRouter<S> {}

/// GET (0.01) router. Chain [`.put`](MethodRouter::put) and siblings.
#[must_use]
pub const fn get<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
    MethodRouter::new().get(handler)
}

/// POST (0.02) router.
#[must_use]
pub const fn post<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
    MethodRouter::new().post(handler)
}

/// PUT (0.03) router.
#[must_use]
pub const fn put<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
    MethodRouter::new().put(handler)
}

/// DELETE (0.04) router.
#[must_use]
pub const fn delete<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
    MethodRouter::new().delete(handler)
}

/// FETCH (0.05) router.
#[must_use]
pub const fn fetch<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
    MethodRouter::new().fetch(handler)
}

/// PATCH (0.06) router.
#[must_use]
pub const fn patch<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
    MethodRouter::new().patch(handler)
}

/// iPATCH (0.07) router.
#[must_use]
pub const fn ipatch<S>(handler: HandlerFn<S>) -> MethodRouter<S> {
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

/// Split a `'static` URI-Path (`"/sensors/temp"`) into Uri-Path segments.
///
/// Leading `/` is ignored. Empty segments are dropped. Writes into `out`
/// and returns the count.
///
/// # Panics
///
/// If the path has more segments than `out.len()`.
pub fn split_path(path: &'static str, out: &mut [&'static str]) -> usize {
    let mut n = 0;
    for segment in path.trim_start_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        assert!(
            n < out.len(),
            "URI-Path has more than {} segments; shorten the path or raise MAX_PATH_SEGMENTS",
            out.len()
        );
        out[n] = segment;
        n += 1;
    }
    n
}
