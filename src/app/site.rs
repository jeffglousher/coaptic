//! Bounded table of [`MethodRouter`](super::MethodRouter) routes.

use crate::message::ContentFormat;
use crate::storage::ObserveResource;

use super::request::{MAX_PATH_SEGMENTS, Path, PathError, Request};
use super::response::Response;
use super::routing::{Method, MethodRouter, ObserveSource, split_path};

/// Default number of routes in a [`Site`] / [`App`](super::App).
pub const DEFAULT_ROUTES: usize = 8;

const WELL_KNOWN: &[&str] = &[".well-known", "core"];

struct Entry {
    path: Path<'static>,
    methods: MethodRouter,
}

/// Fixed table of path → method router.
///
/// `N` is the maximum number of paths (default 8). GET+PUT on one path
/// occupies one slot. This is a routing table, not an Engine memory area
/// and not a shared application bag.
pub struct Site<const N: usize = DEFAULT_ROUTES> {
    entries: [Option<Entry>; N],
    len: usize,
    well_known: bool,
}

impl<const N: usize> Site<N> {
    /// Empty site.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; N],
            len: 0,
            well_known: false,
        }
    }

    /// Number of registered routes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no routes are registered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a further [`Self::route`] would panic (unless it replaces a path).
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= N
    }

    /// Maximum routes (`N`).
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Whether [`Self::well_known_core`] is enabled.
    #[must_use]
    pub const fn has_well_known_core(&self) -> bool {
        self.well_known
    }

    /// Serve `/.well-known/core` from registered paths (RFC 6690 link-format).
    pub const fn well_known_core(&mut self) -> &mut Self {
        self.well_known = true;
        self
    }

    /// Bind `methods` on `segments`. Replaces an existing path.
    ///
    /// # Panics
    ///
    /// If the table is full and `segments` is a new path, or if the path has
    /// more than [`MAX_PATH_SEGMENTS`] segments.
    pub fn route(&mut self, segments: &[&'static str], methods: MethodRouter) -> &mut Self {
        let path = expect_path(segments);
        for entry in self.entries.iter_mut().flatten() {
            if entry.path.matches(path.segments()) {
                entry.methods = methods;
                return self;
            }
        }
        assert!(
            self.len < N,
            "site is full ({N} routes); bind with .routes::<M>() for a larger M"
        );
        self.entries[self.len] = Some(Entry { path, methods });
        self.len += 1;
        self
    }

    /// Bind `methods` on a `'static` URI-Path (`"/leds/0"`).
    ///
    /// # Panics
    ///
    /// Same as [`Self::route`], plus if `path` has more than
    /// [`MAX_PATH_SEGMENTS`] segments.
    pub fn route_path(&mut self, path: &'static str, methods: MethodRouter) -> &mut Self {
        let mut segments = [""; MAX_PATH_SEGMENTS];
        let n = split_path(path, &mut segments);
        self.route(&segments[..n], methods)
    }

    /// Match `request`: handler, else 4.05 if the path exists, else 4.04.
    #[must_use]
    pub fn dispatch(&self, request: Request<'_>) -> Response {
        for entry in self.entries.iter().flatten() {
            if !entry.path.matches(request.path()) {
                continue;
            }
            return match request.method() {
                Some(method) => entry.methods.call(method, request),
                None => Response::method_not_allowed(),
            };
        }
        if self.well_known && path_is_well_known(request.path()) {
            return match request.method() {
                Some(Method::Get) => link_format(self),
                _ => Response::method_not_allowed(),
            };
        }
        Response::not_found()
    }

    /// Snapshot fn for `resource`, if the matching route registered one.
    #[must_use]
    pub(crate) fn observe_source(&self, resource: ObserveResource) -> Option<ObserveSource> {
        if resource.is_none() {
            return None;
        }
        for entry in self.entries.iter().flatten() {
            if ObserveResource::from_path(entry.path.segments()) == resource {
                return entry.methods.observe_source();
            }
        }
        None
    }

    /// Whether this path has an Observe snapshot or is otherwise observable
    /// only via a handler that returns Observe on the response.
    #[must_use]
    pub(crate) fn has_observe_source(&self, segments: &[&str]) -> bool {
        self.observe_source(ObserveResource::from_path(segments))
            .is_some()
    }
}

impl<const N: usize> Default for Site<N> {
    fn default() -> Self {
        Self::new()
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

fn path_is_well_known(path: &[&str]) -> bool {
    path == WELL_KNOWN
}

fn link_format<const N: usize>(site: &Site<N>) -> Response {
    let mut buf = [0u8; super::response::INLINE_PAYLOAD];
    let mut n = 0usize;
    let mut first = true;
    for entry in site.entries.iter().flatten() {
        if !first {
            if n >= buf.len() {
                break;
            }
            buf[n] = b',';
            n += 1;
        }
        first = false;
        if !write_link(&mut buf, &mut n, entry.path.segments()) {
            break;
        }
    }
    Response::content_copy(&buf[..n]).content_format(ContentFormat::LINK_FORMAT)
}

fn write_link(buf: &mut [u8], n: &mut usize, segments: &[&str]) -> bool {
    if *n >= buf.len() {
        return false;
    }
    buf[*n] = b'<';
    *n += 1;
    if segments.is_empty() {
        if *n >= buf.len() {
            return false;
        }
        buf[*n] = b'/';
        *n += 1;
    } else {
        for segment in segments {
            if *n + 1 + segment.len() >= buf.len() {
                return false;
            }
            buf[*n] = b'/';
            *n += 1;
            buf[*n..*n + segment.len()].copy_from_slice(segment.as_bytes());
            *n += segment.len();
        }
    }
    if *n >= buf.len() {
        return false;
    }
    buf[*n] = b'>';
    *n += 1;
    true
}
