//! Bounded table of [`MethodRouter`](super::MethodRouter) routes.

use crate::message::{Code, ContentFormat};
use crate::storage::ObserveResource;

use super::request::{IntoPath, MAX_PATH_SEGMENTS, Path, PathError, Request, path_from_into};
use super::response::{INLINE_PAYLOAD, RESPONSE_BODY, Response};
use super::routing::{Method, MethodRouter, SnapshotSource};

/// Default number of routes in a [`Site`] / [`App`](super::App).
pub const DEFAULT_ROUTES: usize = 8;

/// Refusal to register a route. Existing routes are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteError {
    /// The supplied path is invalid.
    Path(PathError),
    /// No slot remains for a new path. Existing paths may still be replaced.
    Full,
}

/// Bytes budgeted for one RFC 6690 link plus a comma (`</short>,`).
///
/// The `/.well-known/core` scratch buffer is [`LinkFormatScratch`] /
/// [`link_format_capacity`]: `N ×` this, floored at [`super::INLINE_PAYLOAD`]
/// and capped at [`super::RESPONSE_BODY`]. A catalog that still does not
/// fit is 5.00, never a silently truncated list.
/// URI percent-encoding can expand each segment byte to three bytes; this
/// estimate is not a guarantee that every registered path fits discovery.
pub const LINK_FORMAT_PER_ROUTE: usize = 24;

/// Compile-time `/.well-known/core` payload bytes for a [`Site`] of `N` routes.
///
/// `[u8; link_format_capacity::<N>()]` is not a stable array length (the
/// bound depends on `N`). [`LinkFormatScratch`] is the stack buffer
/// [`App::poll`](super::App::poll) uses instead of `[u8; RESPONSE_BODY]`.
#[must_use]
pub const fn link_format_capacity<const N: usize>() -> usize {
    let n = N.saturating_mul(LINK_FORMAT_PER_ROUTE);
    if n < INLINE_PAYLOAD {
        INLINE_PAYLOAD
    } else if n > RESPONSE_BODY {
        RESPONSE_BODY
    } else {
        n
    }
}

const _: () =
    assert!(link_format_capacity::<DEFAULT_ROUTES>() == DEFAULT_ROUTES * LINK_FORMAT_PER_ROUTE);
const _: () = assert!(link_format_capacity::<1>() == INLINE_PAYLOAD);

/// Stack scratch for `/.well-known/core`, exposing [`link_format_capacity`] bytes.
///
/// `[u8; link_format_capacity::<N>()]` needs generic const exprs.
/// `[[u8; LINK_FORMAT_PER_ROUTE]; N]` is a legal const-generic array; when
/// that is below [`INLINE_PAYLOAD`], the 128-byte floor is used instead so
/// a few longer paths still fit. Overflow stays 5.00.
///
/// For default `N = 8` this is ~192 bytes, not [`RESPONSE_BODY`] (4096).
/// The backing enum reserves the larger of `N * LINK_FORMAT_PER_ROUTE` and
/// [`INLINE_PAYLOAD`], plus its discriminant. The [`RESPONSE_BODY`] ceiling
/// limits the exposed slice, not this backing storage: `N = 256` reserves over
/// 6144 bytes while exposing 4096. Include the full layout in stack budgets.
pub struct LinkFormatScratch<const N: usize> {
    inner: LinkFormatInner<N>,
}

enum LinkFormatInner<const N: usize> {
    /// `N × LINK_FORMAT_PER_ROUTE` when that is at least [`INLINE_PAYLOAD`].
    PerRoute([[u8; LINK_FORMAT_PER_ROUTE]; N]),
    /// [`INLINE_PAYLOAD`] floor when `N × 24` is smaller.
    Inline([u8; INLINE_PAYLOAD]),
}

impl<const N: usize> LinkFormatScratch<N> {
    /// Zeroed scratch for [`Site::dispatch`].
    #[must_use]
    pub const fn new() -> Self {
        let inner = if N.saturating_mul(LINK_FORMAT_PER_ROUTE) >= INLINE_PAYLOAD {
            LinkFormatInner::PerRoute([[0u8; LINK_FORMAT_PER_ROUTE]; N])
        } else {
            LinkFormatInner::Inline([0u8; INLINE_PAYLOAD])
        };
        Self { inner }
    }
}

impl<const N: usize> AsMut<[u8]> for LinkFormatScratch<N> {
    fn as_mut(&mut self) -> &mut [u8] {
        let cap = link_format_capacity::<N>();
        match &mut self.inner {
            LinkFormatInner::PerRoute(chunks) => {
                let buf = chunks.as_flattened_mut();
                let n = cap.min(buf.len());
                &mut buf[..n]
            }
            LinkFormatInner::Inline(buf) => {
                let n = cap.min(buf.len());
                &mut buf[..n]
            }
        }
    }
}

impl<const N: usize> Default for LinkFormatScratch<N> {
    fn default() -> Self {
        Self::new()
    }
}

const WELL_KNOWN: &[&str] = &[".well-known", "core"];

struct Entry {
    path: Path<'static>,
    methods: MethodRouter,
}

/// Fixed table of path → method router.
///
/// `N` is the maximum number of paths (default 8). GET+PUT on one path
/// occupies one slot. Prefer [`crate::App::route`] / [`crate::app::AppBuilder::route`]
/// on the happy path. This is a routing table, not an Engine memory area
/// and not a shared application bag. `/.well-known/core` uses
/// [`LinkFormatScratch`] ([`link_format_capacity`]).
/// Use [`Self::try_route`] for recoverable registration at a resource boundary.
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
    ///
    /// The catalog is written into [`LinkFormatScratch`] (sized by
    /// [`link_format_capacity`] for this `N`). Paths that do not fit
    /// yield 5.00; registered links are never dropped silently.
    pub const fn well_known_core(&mut self) -> &mut Self {
        self.well_known = true;
        self
    }

    /// Bind `methods` on Uri-Path `path`. Replaces an existing path.
    ///
    /// `path` is [`IntoPath`]: `&["sensors", "temp"]` or `"sensors/temp"`
    /// (leading `/` ignored). Empty segments are rejected.
    /// Use [`Self::try_route`] to handle refusal without panicking.
    ///
    /// # Panics
    ///
    /// If the table is full and `path` is new, if the path has more than
    /// [`MAX_PATH_SEGMENTS`] segments, or if a segment is empty.
    pub fn route(&mut self, path: impl IntoPath, methods: MethodRouter) -> &mut Self {
        let path = expect_path(path);
        self.insert_route(path, methods).unwrap_or_else(|_| {
            panic!("site is full ({N} routes); bind with .routes::<M>() for a larger M")
        })
    }

    /// Register or replace a route without panicking on invalid paths or capacity.
    ///
    /// Refusal leaves all routes unchanged. Replacing an existing path succeeds
    /// even when full and retains its local route identity. No allocation occurs.
    pub fn try_route(
        &mut self,
        path: impl IntoPath,
        methods: MethodRouter,
    ) -> Result<&mut Self, RouteError> {
        let path = path_from_into(path).map_err(RouteError::Path)?;
        self.insert_route(path, methods)
    }

    /// Checked registration of a static slash-separated path.
    pub fn try_route_path(
        &mut self,
        path: &'static str,
        methods: MethodRouter,
    ) -> Result<&mut Self, RouteError> {
        self.try_route(path, methods)
    }

    fn insert_route(
        &mut self,
        path: Path<'static>,
        methods: MethodRouter,
    ) -> Result<&mut Self, RouteError> {
        for entry in self.entries.iter_mut().flatten() {
            if entry.path.matches(path.segments()) {
                entry.methods = methods;
                return Ok(self);
            }
        }
        if self.len >= N {
            return Err(RouteError::Full);
        }
        let index = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(RouteError::Full)?;
        self.entries[index] = Some(Entry { path, methods });
        self.len += 1;
        Ok(self)
    }

    /// Remove a path and reclaim its slot, returning whether it was registered.
    ///
    /// Invalid paths leave the table unchanged. Remaining route identities are
    /// stable; entries are never compacted. Copies of a removed route's advanced
    /// resource identity become invalid and must not be reused.
    ///
    /// A standalone Site owns no protocol state. When borrowing a Site from an
    /// App, use [`App::remove_route`](super::App::remove_route) instead: it checks
    /// that App-owned work has drained before an identity can be reused.
    pub fn remove_route(&mut self, path: impl IntoPath) -> Result<bool, PathError> {
        let path = path_from_into(path)?;
        Ok(self.remove_path(path))
    }

    pub(crate) fn contains_path(&self, path: Path<'static>) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.path.matches(path.segments()))
    }

    pub(crate) fn remove_path(&mut self, path: Path<'static>) -> bool {
        let Some(index) = self.entries.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.path.matches(path.segments()))
        }) else {
            return false;
        };
        self.entries[index] = None;
        self.len -= 1;
        true
    }

    /// Bind `methods` on a `'static` URI-Path (`"/leds/0"`).
    ///
    /// Same as [`Self::route`] with a slash-separated string.
    ///
    /// # Panics
    ///
    /// Same as [`Self::route`].
    pub fn route_path(&mut self, path: &'static str, methods: MethodRouter) -> &mut Self {
        self.route(path, methods)
    }

    /// Match `request`: handler, else 4.05 if the path exists, else 4.04.
    /// Unbound method and unknown path use RFC 9290 problem details (CBOR).
    ///
    /// `catalog` is scratch for `/.well-known/core` when the list exceeds
    /// [`INLINE_PAYLOAD`]. Unused for ordinary routes. Pass
    /// [`LinkFormatScratch`] (or any slice of
    /// [`link_format_capacity`], capped at [`RESPONSE_BODY`]).
    /// Links encode decoded path segments as URI references without changing
    /// their boundaries (RFC 6690 section 2 and RFC 7252 section 6.5).
    /// Unsupported unicast discovery query filters are ignored.
    #[must_use]
    pub fn dispatch<'a>(&self, request: Request<'_>, catalog: &'a mut [u8]) -> Response<'a> {
        for entry in self.entries.iter().flatten() {
            if !entry.path.matches(request.path()) {
                continue;
            }
            return match request.method() {
                Some(method) => entry.methods.call(method, request),
                None => Response::problem(Code::METHOD_NOT_ALLOWED).title("Method Not Allowed"),
            };
        }
        if self.well_known && path_is_well_known(request.path()) {
            return match request.method() {
                Some(Method::Get) => link_format(self, catalog),
                _ => Response::problem(Code::METHOD_NOT_ALLOWED).title("Method Not Allowed"),
            };
        }
        Response::problem(Code::NOT_FOUND).title("Not Found")
    }

    /// Stable local identity, selected by exact segment comparison.
    pub(crate) fn observe_resource(&self, segments: &[&str]) -> ObserveResource {
        self.entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| {
                entry
                    .as_ref()
                    .filter(|entry| entry.path.matches(segments))
                    .map(|_| ObserveResource::app_route(index))
            })
            .unwrap_or(ObserveResource::NONE)
    }

    /// Snapshot fn for an opaque App route identity.
    pub(crate) fn observe_source(&self, resource: ObserveResource) -> Option<SnapshotSource> {
        self.entries.iter().enumerate().find_map(|(index, entry)| {
            entry
                .as_ref()
                .filter(|_| ObserveResource::app_route(index) == resource)
                .and_then(|entry| entry.methods.observe_source())
        })
    }

    pub(crate) fn has_observe_source(&self, segments: &[&str]) -> bool {
        self.observe_source(self.observe_resource(segments))
            .is_some()
    }
}

impl<const N: usize> Default for Site<N> {
    fn default() -> Self {
        Self::new()
    }
}

fn expect_path(path: impl IntoPath) -> Path<'static> {
    match path_from_into(path) {
        Ok(path) => path,
        Err(PathError::TooLong) => panic!(
            "URI-Path has more than {MAX_PATH_SEGMENTS} segments; shorten the path or raise MAX_PATH_SEGMENTS"
        ),
        Err(PathError::EmptySegment) => {
            panic!("URI-Path has an empty segment; omit extra slashes")
        }
        Err(PathError::DotSegment) => panic!("URI-Path has a forbidden dot segment"),
        Err(PathError::BadUtf8) => unreachable!("static &str is UTF-8"),
    }
}

fn path_is_well_known(path: &[&str]) -> bool {
    path == WELL_KNOWN
}

fn link_format<'a, const N: usize>(site: &Site<N>, storage: &'a mut [u8]) -> Response<'a> {
    let cap = link_format_capacity::<N>().min(storage.len());
    let buf = &mut storage[..cap];
    let mut n = 0usize;
    let mut first = true;
    for entry in site.entries.iter().flatten() {
        if !first {
            if n >= buf.len() {
                return Response::internal_error();
            }
            buf[n] = b',';
            n += 1;
        }
        first = false;
        if !write_link(buf, &mut n, entry.path.segments()) {
            return Response::internal_error();
        }
    }
    Response::new(Code::CONTENT)
        .payload_copy_full(&buf[..n])
        .content_format(ContentFormat::LINK_FORMAT)
        .etag(&catalog_etag(&buf[..n]))
}

/// Identity of this catalog. RFC 9177 section 4.4 requires an ETag on every Q-Block2 payload.
fn catalog_etag(body: &[u8]) -> [u8; 4] {
    let mut hash = 0x811c_9dc5u32;
    for byte in body {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash.to_be_bytes()
}

fn link_bytes(segments: &[&str], limit: usize) -> Option<usize> {
    if segments.is_empty() {
        (limit >= 3).then_some(3)
    } else {
        segments.iter().try_fold(2usize, |total, segment| {
            let total = total.checked_add(1)?;
            if total > limit {
                return None;
            }
            segment.as_bytes().iter().try_fold(total, |n, &byte| {
                let n = n.checked_add(if uri_path_byte_is_literal(byte) { 1 } else { 3 })?;
                (n <= limit).then_some(n)
            })
        })
    }
}

fn uri_path_byte_is_literal(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
        )
}

fn write_link(buf: &mut [u8], n: &mut usize, segments: &[&str]) -> bool {
    if link_bytes(segments, buf.len().saturating_sub(*n)).is_none() {
        return false;
    }
    buf[*n] = b'<';
    *n += 1;
    if segments.is_empty() {
        buf[*n] = b'/';
        *n += 1;
    } else {
        for segment in segments {
            buf[*n] = b'/';
            *n += 1;
            for &byte in segment.as_bytes() {
                if uri_path_byte_is_literal(byte) {
                    buf[*n] = byte;
                    *n += 1;
                } else {
                    const HEX: &[u8; 16] = b"0123456789ABCDEF";
                    buf[*n] = b'%';
                    buf[*n + 1] = HEX[usize::from(byte >> 4)];
                    buf[*n + 2] = HEX[usize::from(byte & 15)];
                    *n += 3;
                }
            }
        }
    }
    buf[*n] = b'>';
    *n += 1;
    true
}
