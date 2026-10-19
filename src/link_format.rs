//! Borrowed CoRE discovery links and bounded serialization (RFC 6690).
//!
//! Parsing retains the caller's UTF-8 input; it allocates no storage. Iterate to
//! the end to validate the entire document. Links are yielded only after their
//! attributes have been checked. Unknown attributes remain available unchanged.
//! URI references are checked for legal URI characters and percent escapes;
//! resolving their authority, path and anchor is the caller's responsibility.
//! Extension-specific semantics (including RFC 5987 values) are not interpreted.

use core::fmt::{self, Write};

/// Malformed discovery data or insufficient caller storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Invalid delimiters, character encoding, quoting or attribute value.
    Syntax,
    /// A non-repeatable `rt`, `if` or `sz` attribute was repeated.
    DuplicateAttribute,
    /// `href` is reserved for query filtering, not a link attribute.
    ReservedAttribute,
    /// The output slice cannot hold the complete result; it is unchanged.
    OutputTooSmall,
}

/// A possibly escaped quoted attribute value, borrowing the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Text<'a> {
    raw: &'a str,
    escaped: bool,
}

impl<'a> Text<'a> {
    /// Encoded contents, without surrounding quotes. Backslash escapes remain.
    pub const fn encoded(self) -> &'a str {
        self.raw
    }

    /// Logical characters, interpreting quoted-pair escapes without allocation.
    pub fn chars(self) -> impl Iterator<Item = char> + 'a {
        let mut chars = self.raw.chars();
        core::iter::from_fn(move || {
            let ch = chars.next()?;
            if self.escaped && ch == '\\' {
                chars.next()
            } else {
                Some(ch)
            }
        })
    }

    /// Copy the decoded UTF-8 value. Short output is left unchanged.
    pub fn copy_into(self, output: &mut [u8]) -> Result<usize, Error> {
        let len: usize = self.chars().map(char::len_utf8).sum();
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        let mut n = 0;
        for ch in self.chars() {
            let count = ch.len_utf8();
            ch.encode_utf8(&mut output[n..n + count]);
            n += count;
        }
        Ok(n)
    }
}

/// One parsed attribute. Repeated extension attributes are retained in order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Attribute<'a> {
    /// Attribute name with wire spelling preserved; name matching ignores ASCII case.
    pub name: &'a str,
    /// Absent for a flag attribute; quoted empty strings remain present.
    pub value: Option<Text<'a>>,
}

/// A size estimate may exceed every integer type supported by this device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeEstimate {
    /// Exact advertised estimate, not a guarantee of response size.
    Bytes(u64),
    /// A valid cardinal larger than `u64::MAX` (RFC 6690 section 3.3).
    Larger,
}

/// One validated link, borrowing its URI and attribute storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Link<'a> {
    target: &'a str,
    attributes: &'a str,
}

impl<'a> Link<'a> {
    /// URI reference inside angle brackets, with percent escapes preserved.
    pub const fn target(self) -> &'a str {
        self.target
    }

    /// Attributes in wire order, including unknown extensions and repeated `ct`.
    pub fn attributes(self) -> impl Iterator<Item = Attribute<'a>> {
        let mut rest = self.attributes;
        core::iter::from_fn(move || {
            if rest.is_empty() {
                return None;
            }
            let (attribute, next) = attribute(&rest[1..]).ok()?;
            rest = next;
            Some(attribute)
        })
    }

    /// First value for this name. Use [`Self::attributes`] for repeated values.
    pub fn value(self, name: &str) -> Option<Text<'a>> {
        self.attributes()
            .find(|a| a.name.eq_ignore_ascii_case(name))?
            .value
    }

    /// Resource type names/URIs, separated by spaces on the wire.
    pub fn resource_types(self) -> impl Iterator<Item = &'a str> {
        self.value("rt").into_iter().flat_map(|v| words(v.raw))
    }

    /// Interface description names/URIs.
    pub fn interfaces(self) -> impl Iterator<Item = &'a str> {
        self.value("if").into_iter().flat_map(|v| words(v.raw))
    }

    /// All advertised content formats, including repeated `ct` attributes.
    pub fn content_formats(self) -> impl Iterator<Item = u16> {
        self.attributes()
            .filter(|a| a.name.eq_ignore_ascii_case("ct"))
            .filter_map(|a| a.value)
            .flat_map(|v| words(v.raw))
            .filter_map(|word| word.parse().ok())
    }

    /// Advertised maximum size, preserving the distinction between absent and huge.
    pub fn size_estimate(self) -> Option<SizeEstimate> {
        self.value("sz").map(|v| match v.raw.parse() {
            Ok(size) => SizeEstimate::Bytes(size),
            Err(_) => SizeEstimate::Larger,
        })
    }

    /// Whether `obs` is present. Its value and later occurrences are ignored as
    /// required by RFC 7641; this hint does not guarantee registration acceptance.
    pub fn observable(self) -> bool {
        self.attributes()
            .any(|a| a.name.eq_ignore_ascii_case("obs"))
    }
}

/// Streaming parser, ending permanently after its first error.
#[derive(Clone, Debug)]
pub struct Links<'a> {
    rest: &'a str,
}

/// Parse a UTF-8 link-format representation with constant auxiliary memory.
/// Empty input is a valid empty catalog. Whitespace outside quoted values is
/// rejected: RFC 6690 removes HTTP header linear whitespace from this encoding.
pub const fn parse(input: &str) -> Links<'_> {
    Links { rest: input }
}

impl<'a> Iterator for Links<'a> {
    type Item = Result<Link<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let input = core::mem::take(&mut self.rest);
        Some(parse_link(input).map(|(link, rest)| {
            self.rest = rest;
            link
        }))
    }
}

impl core::iter::FusedIterator for Links<'_> {}

fn parse_link(input: &str) -> Result<(Link<'_>, &str), Error> {
    let input = input.strip_prefix('<').ok_or(Error::Syntax)?;
    let end = input.find('>').ok_or(Error::Syntax)?;
    let target = &input[..end];
    if !uri_chars(target) {
        return Err(Error::Syntax);
    }
    let attrs = &input[end + 1..];
    let mut rest = attrs;
    let mut seen = 0u8;
    while let Some(next) = rest.strip_prefix(';') {
        let (attr, next) = attribute(next)?;
        validate_attribute(attr, &mut seen)?;
        rest = next;
    }
    let attributes = &attrs[..attrs.len() - rest.len()];
    if !rest.is_empty() {
        rest = rest.strip_prefix(',').ok_or(Error::Syntax)?;
        if rest.is_empty() {
            return Err(Error::Syntax);
        }
    }
    Ok((Link { target, attributes }, rest))
}

fn attribute(input: &str) -> Result<(Attribute<'_>, &str), Error> {
    let end = input
        .bytes()
        .take_while(|b| name_char(*b) || *b == b'*')
        .count();
    if end == 0 {
        return Err(Error::Syntax);
    }
    let name = &input[..end];
    let extended = name.ends_with('*');
    let base = name.strip_suffix('*').unwrap_or(name);
    if base.is_empty() || !base.bytes().all(name_char) {
        return Err(Error::Syntax);
    }
    let rest = &input[end..];
    let Some(rest) = rest.strip_prefix('=') else {
        if extended {
            return Err(Error::Syntax);
        }
        return Ok((Attribute { name, value: None }, rest));
    };
    let (raw, escaped, rest) = if let Some(body) = rest.strip_prefix('"') {
        if extended {
            return Err(Error::Syntax);
        }
        let mut chars = body.char_indices();
        let end = loop {
            let (i, ch) = chars.next().ok_or(Error::Syntax)?;
            match ch {
                '"' => break i,
                '\\' => {
                    let (_, escaped) = chars.next().ok_or(Error::Syntax)?;
                    if !escaped.is_ascii() || escaped == '\r' || escaped == '\n' {
                        return Err(Error::Syntax);
                    }
                }
                ch if ch.is_ascii_control() && ch != '\t' => return Err(Error::Syntax),
                _ => {}
            }
        };
        (&body[..end], true, &body[end + 1..])
    } else {
        let end = rest.bytes().take_while(|b| token_char(*b)).count();
        if end == 0 {
            return Err(Error::Syntax);
        }
        (&rest[..end], false, &rest[end..])
    };
    Ok((
        Attribute {
            name,
            value: Some(Text { raw, escaped }),
        },
        rest,
    ))
}

fn validate_attribute(attr: Attribute<'_>, seen: &mut u8) -> Result<(), Error> {
    let name = core_name(attr.name).unwrap_or(attr.name);
    let bit = match name {
        "href" => return Err(Error::ReservedAttribute),
        "rt" => 1,
        "if" => 2,
        "sz" => 4,
        _ => 0,
    };
    if *seen & bit != 0 {
        return Err(Error::DuplicateAttribute);
    }
    *seen |= bit;
    let valid = match name {
        "rt" | "if" => attr.value.is_some_and(|v| relation_types(v.raw)),
        "sz" => attr.value.is_some_and(|v| !v.escaped && cardinal(v.raw)),
        "ct" => attr.value.is_some_and(|v| {
            list(v.raw) && words(v.raw).all(|w| cardinal(w) && w.parse::<u16>().is_ok())
        }),
        _ => true,
    };
    if valid { Ok(()) } else { Err(Error::Syntax) }
}

fn core_name(name: &str) -> Option<&'static str> {
    ["rt", "if", "ct", "sz", "obs", "href"]
        .into_iter()
        .find(|known| name.eq_ignore_ascii_case(known))
}

fn words(value: &str) -> impl Iterator<Item = &str> {
    value.split(' ').filter(|s| !s.is_empty())
}

fn list(value: &str) -> bool {
    !value.is_empty() && !value.starts_with(' ') && !value.ends_with(' ')
}

fn cardinal(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|b| b.is_ascii_digit())
}

fn relation_types(value: &str) -> bool {
    list(value)
        && words(value).all(|word| {
            let registered = word.as_bytes()[0].is_ascii_lowercase()
                && word.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-')
                });
            let absolute = word.split_once(':').is_some_and(|(scheme, _)| {
                !scheme.is_empty()
                    && scheme.as_bytes()[0].is_ascii_alphabetic()
                    && scheme
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
                    && uri_chars(word)
            });
            registered || absolute
        })
}

fn name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b)
}

fn token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'()*+-./:<=>?@[]^_`{|}~".contains(&b)
}

fn uri_chars(value: &str) -> bool {
    let mut bytes = value.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            if !bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                || !bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
            {
                return false;
            }
        } else if !b.is_ascii_alphanumeric() && !b"-._~:/?#[]@!$&'()*+,;=".contains(&b) {
            return false;
        }
    }
    true
}

/// An application-defined attribute to serialize. Values are logical text, not
/// pre-escaped wire data; quotes and backslashes are escaped by the writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Extension<'a> {
    /// Attribute name; core metadata names and `href` are reserved here.
    pub name: &'a str,
    /// `None` emits a flag; `Some("")` emits an empty quoted value.
    pub value: Option<&'a str>,
}

/// Borrowed metadata for one discovery link. No table or payload is allocated.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metadata<'a> {
    /// Space-separated resource type names or absolute URIs.
    pub resource_types: Option<&'a str>,
    /// Space-separated interface names or absolute URIs.
    pub interfaces: Option<&'a str>,
    /// Available content-format codes; empty means unspecified.
    pub content_formats: &'a [u16],
    /// Maximum estimated representation size, if advertised.
    pub size_estimate: Option<u64>,
    /// Advertise the observation hint.
    pub observable: bool,
    /// Additional application attributes in output order.
    pub extensions: &'a [Extension<'a>],
}

impl Metadata<'_> {
    /// Exact encoded attribute length (including semicolons), after validation.
    pub fn encoded_len(self) -> Result<usize, Error> {
        self.validate()?;
        let mut writer = Output {
            bytes: None,
            len: 0,
        };
        self.write(&mut writer).map_err(|_| Error::OutputTooSmall)?;
        Ok(writer.len)
    }

    /// Serialize attributes including leading semicolons. On any error, output
    /// is unchanged. This can append metadata to an existing path serializer.
    pub fn write_into(self, output: &mut [u8]) -> Result<usize, Error> {
        let len = self.encoded_len()?;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        self.write(&mut Output {
            bytes: Some(output),
            len: 0,
        })
        .map_err(|_| Error::OutputTooSmall)?;
        Ok(len)
    }

    fn validate(self) -> Result<(), Error> {
        for value in [self.resource_types, self.interfaces].into_iter().flatten() {
            if !relation_types(value) {
                return Err(Error::Syntax);
            }
        }
        for ext in self.extensions {
            if core_name(ext.name).is_some() {
                return Err(Error::ReservedAttribute);
            }
            // Extended RFC 5987 values need their own encoder, not quoted text.
            if ext.name.is_empty()
                || !ext.name.bytes().all(name_char)
                || ext.name.contains('*')
                || ext
                    .value
                    .is_some_and(|s| s.chars().any(|c| c.is_ascii_control() && c != '\t'))
            {
                return Err(Error::Syntax);
            }
        }
        Ok(())
    }

    fn write(self, output: &mut impl Write) -> fmt::Result {
        for (name, value) in [("rt", self.resource_types), ("if", self.interfaces)] {
            if let Some(value) = value {
                write!(output, ";{name}=")?;
                quoted(output, value)?;
            }
        }
        if !self.content_formats.is_empty() {
            output.write_str(";ct=\"")?;
            for (i, value) in self.content_formats.iter().enumerate() {
                if i != 0 {
                    output.write_char(' ')?;
                }
                write!(output, "{value}")?;
            }
            output.write_char('"')?;
        }
        if let Some(value) = self.size_estimate {
            write!(output, ";sz={value}")?;
        }
        if self.observable {
            output.write_str(";obs")?;
        }
        for ext in self.extensions {
            write!(output, ";{}", ext.name)?;
            if let Some(value) = ext.value {
                output.write_char('=')?;
                quoted(output, value)?;
            }
        }
        Ok(())
    }
}

/// Write a complete link into caller storage. Input is a URI reference already
/// percent-encoded by the caller. Invalid input or short output leaves the entire
/// output unchanged. Separate multiple links with a comma.
pub fn write_link(target: &str, metadata: Metadata<'_>, output: &mut [u8]) -> Result<usize, Error> {
    if !uri_chars(target) {
        return Err(Error::Syntax);
    }
    let len = metadata
        .encoded_len()?
        .checked_add(target.len())
        .and_then(|n| n.checked_add(2))
        .ok_or(Error::OutputTooSmall)?;
    if output.len() < len {
        return Err(Error::OutputTooSmall);
    }
    let mut writer = Output {
        bytes: Some(output),
        len: 0,
    };
    write!(&mut writer, "<{target}>").map_err(|_| Error::OutputTooSmall)?;
    metadata
        .write(&mut writer)
        .map_err(|_| Error::OutputTooSmall)?;
    Ok(len)
}

fn quoted(output: &mut impl Write, value: &str) -> fmt::Result {
    output.write_char('"')?;
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            output.write_char('\\')?;
        }
        output.write_char(ch)?;
    }
    output.write_char('"')
}

struct Output<'a> {
    bytes: Option<&'a mut [u8]>,
    len: usize,
}

impl Write for Output<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(fmt::Error)?;
        if let Some(bytes) = self.bytes.as_mut() {
            bytes
                .get_mut(self.len..end)
                .ok_or(fmt::Error)?
                .copy_from_slice(s.as_bytes());
        }
        self.len = end;
        Ok(())
    }
}
