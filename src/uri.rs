//! Borrowed absolute CoAP URI views and caller-buffer option decoding.
//!
//! DNS, interface-zone resolution and transport creation belong to the caller.
//! Parsing `coaps` requires DTLS; it never selects unprotected UDP or substitutes
//! OSCORE for that transport. Relative references and dot segments must be
//! resolved by the caller before parsing. No allocation or normalization buffer
//! is retained. Iteration and decoding take linear work in the supplied bytes.

use core::net::Ipv6Addr;

/// Transport required by a URI scheme, independent of message-level OSCORE.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    /// `coap`, default port 5683. May carry OSCORE-protected messages.
    Udp,
    /// `coaps`, default port 5684. Requires a caller-provided DTLS adapter.
    Dtls,
}

/// A URI cannot be interpreted within this absolute, resolved URI profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Scheme is not `coap` or `coaps`.
    Scheme,
    /// Missing/invalid authority, user information, or IP literal.
    Authority,
    /// Port is not an unsigned 16-bit decimal value.
    Port,
    /// Invalid character, percent escape, or decoded UTF-8.
    Encoding,
    /// Fragments are not allowed in CoAP request URIs.
    Fragment,
    /// Resolve dot segments before constructing CoAP Uri-Path options.
    DotSegment,
    /// Caller output is short; its bytes remain unchanged.
    Capacity,
}

/// Percent-encoded URI text validated by [`Uri::parse`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Component<'a>(&'a str);

impl<'a> Component<'a> {
    /// Original encoded spelling; delimiters have already been split.
    pub const fn encoded(self) -> &'a str {
        self.0
    }

    /// Exact number of bytes needed for the decoded option value.
    pub fn decoded_len(self) -> usize {
        decoded(self.0).count()
    }

    /// Decode once into caller storage, preserving encoded slash/ampersand and
    /// literal plus signs. Short output is not modified.
    pub fn decode<'b>(self, output: &'b mut [u8]) -> Result<&'b str, Error> {
        let n = self.decoded_len();
        if output.len() < n {
            return Err(Error::Capacity);
        }
        for (out, byte) in output[..n].iter_mut().zip(decoded(self.0)) {
            *out = byte;
        }
        // Parsing validates decoded UTF-8 before a Component can be constructed.
        core::str::from_utf8(&output[..n]).map_err(|_| Error::Encoding)
    }
}

/// Host identity without DNS or interface lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Host<'a> {
    /// Registered name or IPv4 spelling, preserved for caller resolution.
    Name(Component<'a>),
    /// Valid IPv6 literal, with optional percent-encoded interface zone.
    Ipv6 {
        /// Parsed 128-bit address, without a platform interface identifier.
        address: Ipv6Addr,
        /// Encoded zone text for caller-owned interface resolution.
        zone: Option<Component<'a>>,
    },
}

/// Checked views into an absolute CoAP URI. Does not own any input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Uri<'a> {
    transport: Transport,
    authority: &'a str,
    host: Host<'a>,
    port: u16,
    path: &'a str,
    query: Option<&'a str>,
}

impl<'a> Uri<'a> {
    /// Parse an ASCII absolute URI. Accepts case-insensitive schemes, explicit
    /// or empty/default ports, IPv6 zones encoded with `%25`, empty path/query
    /// segments and UTF-8 percent escapes. Refuses userinfo, fragments and dot
    /// segments. The input length itself bounds parsing work; no DNS is invoked.
    pub fn parse(input: &'a str) -> Result<Self, Error> {
        if input.contains('#') {
            return Err(Error::Fragment);
        }
        let (scheme, rest) = input.split_once("://").ok_or(Error::Scheme)?;
        let transport = if scheme.eq_ignore_ascii_case("coap") {
            Transport::Udp
        } else if scheme.eq_ignore_ascii_case("coaps") {
            Transport::Dtls
        } else {
            return Err(Error::Scheme);
        };
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let authority = &rest[..end];
        if authority.is_empty() || authority.contains('@') {
            return Err(Error::Authority);
        }
        let (host, port) = if let Some(literal) = authority.strip_prefix('[') {
            let close = literal.find(']').ok_or(Error::Authority)?;
            let host = &literal[..close];
            let suffix = &literal[close + 1..];
            let port = if suffix.is_empty() {
                None
            } else {
                Some(suffix.strip_prefix(':').ok_or(Error::Authority)?)
            };
            let (address, zone) = match host.split_once("%25") {
                Some((address, zone)) if !zone.is_empty() => {
                    validate(zone, unreserved)?;
                    (address, Some(Component(zone)))
                }
                Some(_) => return Err(Error::Authority),
                None => (host, None),
            };
            (
                Host::Ipv6 {
                    address: address.parse().map_err(|_| Error::Authority)?,
                    zone,
                },
                port,
            )
        } else {
            let (name, port) = authority
                .split_once(':')
                .map_or((authority, None), |(n, p)| (n, Some(p)));
            if name.is_empty() {
                return Err(Error::Authority);
            }
            validate(name, |b| unreserved(b) || subdelim(b))?;
            (Host::Name(Component(name)), port)
        };
        let default = match transport {
            Transport::Udp => 5683,
            Transport::Dtls => 5684,
        };
        let port = match port {
            None | Some("") => default,
            Some(value) => {
                if !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::Port);
                }
                value.parse().map_err(|_| Error::Port)?
            }
        };
        let tail = &rest[end..];
        let (path, query) = tail
            .split_once('?')
            .map_or((tail, None), |(p, q)| (p, Some(q)));
        validate(path, |b| pchar(b) || b == b'/')?;
        if let Some(query) = query {
            validate(query, |b| pchar(b) || b == b'/' || b == b'?')?;
        }
        let uri = Self {
            transport,
            authority,
            host,
            port,
            path,
            query,
        };
        for segment in uri.path_segments() {
            let mut bytes = decoded(segment.0);
            if bytes.next() == Some(b'.') {
                match (bytes.next(), bytes.next()) {
                    (None, _) | (Some(b'.'), None) => return Err(Error::DotSegment),
                    _ => {}
                }
            }
        }
        Ok(uri)
    }

    /// Required transport. A caller must refuse if its adapter cannot supply it.
    pub const fn transport(self) -> Transport {
        self.transport
    }
    /// Original authority spelling, including explicit port and brackets.
    pub const fn authority(self) -> &'a str {
        self.authority
    }
    /// Host identity, retaining an IPv6 interface zone if supplied.
    pub const fn host(self) -> Host<'a> {
        self.host
    }
    /// Effective UDP port, including the scheme default.
    pub const fn port(self) -> u16 {
        self.port
    }
    /// Encoded path as supplied, including the leading slash when present.
    pub const fn path(self) -> &'a str {
        self.path
    }
    /// Encoded query; absent and present-but-empty remain distinct.
    pub const fn query(self) -> Option<&'a str> {
        self.query
    }
    /// Uri-Path option components, preserving empty and escaped segments.
    /// Empty path and `/` produce no options, as RFC 7252 section 6.4 requires.
    pub fn path_segments(self) -> impl Iterator<Item = Component<'a>> {
        let path = if self.path.is_empty() || self.path == "/" {
            None
        } else {
            self.path.strip_prefix('/')
        };
        path.into_iter().flat_map(|p| p.split('/')).map(Component)
    }
    /// Uri-Query option components. Split before decoding, never treating `+`
    /// as a space or a percent-encoded ampersand as a separator.
    pub fn query_arguments(self) -> impl Iterator<Item = Component<'a>> {
        self.query
            .into_iter()
            .flat_map(|q| q.split('&'))
            .map(Component)
    }
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~".contains(&b)
}
fn subdelim(b: u8) -> bool {
    b"!$&'()*+,;=".contains(&b)
}
fn pchar(b: u8) -> bool {
    unreserved(b) || subdelim(b) || b == b':' || b == b'@'
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
fn decoded(input: &str) -> impl Iterator<Item = u8> + '_ {
    let mut bytes = input.bytes();
    core::iter::from_fn(move || {
        let b = bytes.next()?;
        Some(if b == b'%' {
            (hex(bytes.next()?)? << 4) | hex(bytes.next()?)?
        } else {
            b
        })
    })
}
fn validate(input: &str, allowed: impl Fn(u8) -> bool) -> Result<(), Error> {
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            if bytes.next().and_then(hex).is_none() || bytes.next().and_then(hex).is_none() {
                return Err(Error::Encoding);
            }
        } else if !allowed(b) {
            return Err(Error::Encoding);
        }
    }
    // Validate one decoded scalar at a time, without a component-sized buffer.
    let mut bytes = decoded(input);
    while let Some(first) = bytes.next() {
        let n = match first {
            0..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(Error::Encoding),
        };
        let mut scalar = [first, 0, 0, 0];
        for b in &mut scalar[1..n] {
            *b = bytes.next().ok_or(Error::Encoding)?;
        }
        core::str::from_utf8(&scalar[..n]).map_err(|_| Error::Encoding)?;
    }
    Ok(())
}
