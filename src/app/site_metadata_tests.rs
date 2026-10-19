extern crate std;

use super::*;
use crate::link_format::{self, Extension, SizeEstimate};
use crate::message::decode;
use crate::storage::Endpoint;

const EMPTY: Metadata<'static> = Metadata {
    resource_types: None,
    interfaces: None,
    content_formats: &[],
    size_estimate: None,
    observable: false,
    extensions: &[],
};
static SENSOR: Metadata<'static> = Metadata {
    resource_types: Some("temperature-c"),
    interfaces: Some("sensor"),
    content_formats: &[0, 50],
    size_estimate: Some(1024),
    observable: true,
    extensions: &[Extension {
        name: "title",
        value: Some("Kitchen \"sensor\""),
    }],
};
static INVALID: Metadata<'static> = Metadata {
    resource_types: Some("bad value!"),
    ..EMPTY
};
static LARGE: Metadata<'static> = Metadata {
    extensions: &[Extension {
        name: "title",
        value: Some(
            "An intentionally long static descriptor for a discovery catalog which does not fit the one-route default scratch budget and must produce an explicit capacity refusal.",
        ),
    }],
    ..EMPTY
};

fn handler(_: Request<'_>) -> Response<'static> {
    Response::content(b"ready")
}

fn dispatch<'a, const N: usize>(site: &Site<N>, scratch: &'a mut [u8]) -> Response<'a> {
    let wire = b"\x40\x01\x00\x01\xbb.well-known\x04core";
    let request = Request::from_decoded(
        decode(wire).unwrap(),
        Endpoint::v4([192, 0, 2, 1], 5683),
        None,
    )
    .unwrap();
    site.dispatch(request, scratch)
}

#[test]
fn registered_metadata_reaches_discovery_and_changes_catalog_etag() {
    let mut site = Site::<1>::new();
    site.route("t", super::super::get(handler))
        .well_known_core();
    let mut scratch = LinkFormatScratch::<1>::new();
    let response = dispatch(&site, scratch.as_mut());
    let old_etag: [u8; 4] = response.etag_bytes().unwrap().try_into().unwrap();
    assert_eq!(response.payload(), b"</t>");

    site.set_discovery_metadata("t", Some(&SENSOR)).unwrap();
    let response = dispatch(&site, scratch.as_mut());
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.format(), Some(ContentFormat::LINK_FORMAT));
    assert_ne!(response.etag_bytes(), Some(old_etag.as_slice()));
    let text = core::str::from_utf8(response.payload()).unwrap();
    let link = link_format::parse(text).next().unwrap().unwrap();
    assert_eq!(link.target(), "/t");
    assert_eq!(link.resource_types().next(), Some("temperature-c"));
    assert_eq!(link.interfaces().next(), Some("sensor"));
    assert_eq!(
        link.content_formats().collect::<std::vec::Vec<_>>(),
        [0, 50]
    );
    assert_eq!(link.size_estimate(), Some(SizeEstimate::Bytes(1024)));
    assert!(link.observable());
    assert_eq!(
        link.value("title")
            .unwrap()
            .chars()
            .collect::<std::string::String>(),
        "Kitchen \"sensor\""
    );
}

#[test]
fn refused_metadata_preserves_existing_value_and_missing_route_is_explicit() {
    let mut site = Site::<1>::new();
    site.route("t", super::super::get(handler))
        .well_known_core();
    site.set_discovery_metadata("t", Some(&SENSOR)).unwrap();
    assert_eq!(
        site.set_discovery_metadata("t", Some(&INVALID)).err(),
        Some(RouteError::Metadata(link_format::Error::Syntax))
    );
    assert_eq!(
        site.set_discovery_metadata("missing", Some(&SENSOR)).err(),
        Some(RouteError::NotFound)
    );
    assert_eq!(
        site.set_discovery_metadata("..", None).err(),
        Some(RouteError::Path(PathError::DotSegment))
    );
    let mut scratch = LinkFormatScratch::<1>::new();
    assert!(
        core::str::from_utf8(dispatch(&site, scratch.as_mut()).payload())
            .unwrap()
            .contains("temperature-c")
    );
}

#[test]
fn replacement_preserves_metadata_removal_and_clear_release_it() {
    let mut site = Site::<1>::new();
    site.route("t", super::super::get(handler))
        .well_known_core();
    site.set_discovery_metadata("t", Some(&SENSOR)).unwrap();
    site.try_route("t", super::super::get(handler)).unwrap();
    let mut scratch = LinkFormatScratch::<1>::new();
    assert!(
        dispatch(&site, scratch.as_mut())
            .payload()
            .ends_with(b";title=\"Kitchen \\\"sensor\\\"\"")
    );
    site.set_discovery_metadata("t", None).unwrap();
    assert_eq!(dispatch(&site, scratch.as_mut()).payload(), b"</t>");
    site.set_discovery_metadata("t", Some(&SENSOR)).unwrap();
    assert_eq!(site.remove_route("t"), Ok(true));
    site.route("new", super::super::get(handler));
    assert_eq!(dispatch(&site, scratch.as_mut()).payload(), b"</new>");
}

#[test]
fn metadata_overflow_refuses_then_caller_budget_or_removal_recovers() {
    let mut site = Site::<1>::new();
    site.route("t", super::super::get(handler))
        .well_known_core();
    site.set_discovery_metadata("t", Some(&LARGE)).unwrap();
    let mut scratch = LinkFormatScratch::<1>::new();
    let refused = dispatch(&site, scratch.as_mut());
    assert_eq!(refused.code(), Code::INTERNAL_SERVER_ERROR);
    assert!(refused.payload().is_empty());
    let mut larger = [0; 256];
    let accepted = dispatch(&site, &mut larger);
    assert_eq!(accepted.code(), Code::CONTENT);
    assert!(accepted.payload().len() > link_format_capacity::<1>());
    assert_eq!(
        link_format::parse(core::str::from_utf8(accepted.payload()).unwrap()).count(),
        1
    );
    site.set_discovery_metadata("t", None).unwrap();
    assert_eq!(dispatch(&site, scratch.as_mut()).payload(), b"</t>");
}

#[test]
fn metadata_adds_only_one_pointer_to_each_route_and_no_catalog_buffer() {
    use core::mem::size_of;
    let word = size_of::<usize>();
    assert_eq!(size_of::<Option<&'static Metadata<'static>>>(), word);
    assert!(size_of::<Entry>() <= size_of::<(Path<'static>, MethodRouter)>() + word);
    assert_eq!(link_format_capacity::<1>(), INLINE_PAYLOAD);
    std::println!(
        "entry={} descriptor={} site1={} site8={} scratch1={}",
        size_of::<Entry>(),
        size_of::<Metadata<'static>>(),
        size_of::<Site<1>>(),
        size_of::<Site<8>>(),
        size_of::<LinkFormatScratch<1>>()
    );
}
