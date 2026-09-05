//! Bounded URI-Path router. App state, not a seventh Engine area.

use super::reply::Reply;
use super::request::{MAX_PATH_SEGMENTS, Path, PathError, Request};
use super::resource::{Method, Resource};

/// Default number of method+path bindings.
pub const DEFAULT_ROUTES: usize = 8;

type Handle = for<'a> fn(&'a Request<'a>) -> Reply<'a>;

#[derive(Clone, Copy, Debug)]
struct Route {
    path: Path<'static>,
    method: Method,
    handle: Handle,
}

/// Fixed table of path + method → [`Resource`].
///
/// `N` is the maximum number of bindings (one per `.get` / `.put` / …).
/// This is application state. It is not an Engine memory area.
#[derive(Clone, Debug)]
pub struct Router<const N: usize = DEFAULT_ROUTES> {
    routes: [Option<Route>; N],
    len: usize,
}

impl<const N: usize> Router<N> {
    /// Empty router.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            routes: [None; N],
            len: 0,
        }
    }

    /// Number of bindings.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no routes are registered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a further `.get` / `.put` would panic.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= N
    }

    /// Maximum bindings (`N`).
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Start a path. Segment slices, not a string DSL:
    /// `.at(&["sensors", "temp"]).get(Temp)`.
    pub fn at<'a>(&'a mut self, segments: &[&'static str]) -> At<'a, N> {
        At {
            router: self,
            path: expect_path(segments),
        }
    }

    /// Match `request`: handler, else 4.05 if the path exists, else 4.04.
    #[must_use]
    pub fn dispatch<'a>(&self, request: &'a Request<'a>) -> Reply<'a> {
        let mut path_hit = false;
        for route in self.routes.iter().flatten() {
            if !route.path.matches(request.path()) {
                continue;
            }
            path_hit = true;
            if Some(route.method) == request.method() {
                return (route.handle)(request);
            }
        }
        if path_hit {
            Reply::method_not_allowed()
        } else {
            Reply::not_found()
        }
    }

    fn push(&mut self, path: Path<'static>, method: Method, handle: Handle) {
        assert!(
            self.len < N,
            "router is full ({N} routes); use Server<_, _, N> with a larger N"
        );
        self.routes[self.len] = Some(Route {
            path,
            method,
            handle,
        });
        self.len += 1;
    }
}

impl<const N: usize> Default for Router<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Path scope: chain `.get` / `.put` / … then `.at` for the next path.
pub struct At<'a, const N: usize = DEFAULT_ROUTES> {
    router: &'a mut Router<N>,
    path: Path<'static>,
}

impl<'a, const N: usize> At<'a, N> {
    /// Next path on this router.
    pub fn at(self, segments: &[&'static str]) -> At<'a, N> {
        At {
            router: self.router,
            path: expect_path(segments),
        }
    }

    /// GET → `resource`.
    pub fn get<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::Get, R::handle)
    }

    /// POST → `resource`.
    pub fn post<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::Post, R::handle)
    }

    /// PUT → `resource`.
    pub fn put<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::Put, R::handle)
    }

    /// DELETE → `resource`.
    pub fn delete<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::Delete, R::handle)
    }

    /// FETCH → `resource`.
    pub fn fetch<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::Fetch, R::handle)
    }

    /// PATCH → `resource`.
    pub fn patch<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::Patch, R::handle)
    }

    /// iPATCH → `resource`.
    pub fn ipatch<R: Resource>(self, _resource: R) -> Self {
        self.bind(Method::IPatch, R::handle)
    }

    fn bind(self, method: Method, handle: Handle) -> Self {
        self.router.push(self.path, method, handle);
        self
    }
}

fn expect_path(segments: &[&'static str]) -> Path<'static> {
    match Path::from_segments(segments) {
        Ok(path) => path,
        Err(PathError::TooLong) => panic!(
            "URI-Path has more than {MAX_PATH_SEGMENTS} segments; shorten the path or raise MAX_PATH_SEGMENTS"
        ),
        Err(PathError::BadUtf8) => unreachable!("static &str is UTF-8"),
    }
}
