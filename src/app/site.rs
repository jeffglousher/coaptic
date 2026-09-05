//! Bounded table of owned [`Resource`](super::Resource) instances.

use crate::message::ContentFormat;

use super::reply::Reply;
use super::request::{MAX_PATH_SEGMENTS, Path, PathError, Request};
use super::resource::Method;
use super::resource_dyn::ResourceDyn;

/// Default number of owned resources in a [`Site`] / [`App`](super::App).
pub const DEFAULT_RESOURCES: usize = 8;

const WELL_KNOWN: &[&str] = &[".well-known", "core"];

struct Entry {
    path: Path<'static>,
    resource: ResourceDyn,
}

/// Fixed table of path → owned resource.
///
/// `N` is the maximum number of instances (default 8). One `Led` that
/// implements GET and PUT occupies one slot. This is application state,
/// not an Engine memory area.
pub struct Site<const N: usize = DEFAULT_RESOURCES> {
    entries: [Option<Entry>; N],
    len: usize,
    well_known: bool,
}

impl<const N: usize> Site<N> {
    /// Empty site.
    #[must_use]
    pub const fn new() -> Self {
        const NONE: Option<Entry> = None;
        Self {
            entries: [NONE; N],
            len: 0,
            well_known: false,
        }
    }

    /// Number of registered resources.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no resources are registered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a further [`Self::at`] would panic (unless it replaces a path).
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= N
    }

    /// Maximum instances (`N`).
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
    pub fn well_known_core(&mut self) -> &mut Self {
        self.well_known = true;
        self
    }

    /// Move `resource` onto `segments`. Replaces an existing path.
    ///
    /// # Panics
    ///
    /// If the table is full and `segments` is a new path, or if the path has
    /// more than [`MAX_PATH_SEGMENTS`] segments.
    pub fn at<R: super::Resource>(&mut self, segments: &[&'static str], resource: R) -> &mut Self {
        let path = expect_path(segments);
        for entry in self.entries.iter_mut().flatten() {
            if entry.path.matches(path.segments()) {
                entry.resource = ResourceDyn::new(resource);
                return self;
            }
        }
        assert!(
            self.len < N,
            "site is full ({N} resources); bind with .resources::<M>() for a larger M"
        );
        self.entries[self.len] = Some(Entry {
            path,
            resource: ResourceDyn::new(resource),
        });
        self.len += 1;
        self
    }

    /// Match `request`: method hook, else 4.05 if the path exists, else 4.04.
    #[must_use]
    pub fn dispatch(&mut self, request: &Request<'_>) -> Reply {
        for entry in self.entries.iter_mut().flatten() {
            if !entry.path.matches(request.path()) {
                continue;
            }
            return match request.method() {
                Some(method) => entry.resource.call(method, request),
                None => Reply::method_not_allowed(),
            };
        }
        if self.well_known && path_is_well_known(request.path()) {
            return match request.method() {
                Some(Method::Get) => link_format(self),
                _ => Reply::method_not_allowed(),
            };
        }
        Reply::not_found()
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

fn link_format<const N: usize>(site: &Site<N>) -> Reply {
    let mut buf = [0u8; super::reply::INLINE_PAYLOAD];
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
    Reply::content_copy(&buf[..n]).content_format(ContentFormat::LINK_FORMAT)
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
