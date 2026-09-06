//! Plugtest resource catalog shared by both peer backends.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use coaptic::app::MethodRouter;
use coaptic::message::Precondition;
use coaptic::{Code, ContentFormat, Request, Response, get, post, put};

/// Text body for `/test` GET.
pub const TEST_BODY: &[u8] = b"core-test-payload";
/// Body for `/separate`.
pub const SEP_BODY: &[u8] = b"separate-payload";
/// Body for `/secure` (DTLS TDs).
pub const SECURE_BODY: &[u8] = b"secure-payload";
/// First Observe representation.
pub const OBS_BODY: &[u8] = b"obs-0";
/// Later Observe representation.
pub const OBS_BODY_2: &[u8] = b"obs-1";
/// `/path/sub1` payload.
pub const PATH_SUB1: &[u8] = b"path-sub1";
/// `/path` link-format children.
pub const PATH_LINKS: &str = "</path/sub1>,</path/sub2>";
/// Large Block2 / Block1 body (bigger than a Default datagram).
pub const LARGE_LEN: usize = 2000;
/// Patterned large body (same rule as the in-crate harness).
#[must_use]
pub fn large_body() -> Vec<u8> {
    (0..LARGE_LEN).map(|i| (i % 251) as u8).collect()
}

/// RFC 6690 catalog used by LINK TDs (same strings as `tests/plugtest/link.rs`).
pub const LINK_CATALOG: &[&str] = &[
    "</test>;rt=\"Type1 Type2\";if=\"If1\";sz=123",
    "</link1>;rt=\"Type2 Type3\";if=\"If2\"",
    "</link2>;rt=\"Type1 Type3\";if=\"foo\"",
    "</link3>",
    "</path>;ct=40",
    "</large>;sz=1024",
];

static VALIDATE_BODY: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
static VALIDATE_ETAG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
static TEST_EXISTS: AtomicBool = AtomicBool::new(true);
static OBS_SEQ: AtomicU32 = AtomicU32::new(0);

fn validate_body() -> &'static Mutex<Vec<u8>> {
    VALIDATE_BODY.get_or_init(|| Mutex::new(TEST_BODY.to_vec()))
}

fn validate_etag() -> &'static Mutex<Vec<u8>> {
    VALIDATE_ETAG.get_or_init(|| Mutex::new(b"etag1".to_vec()))
}

/// Reset per-TD resource state.
pub fn reset() {
    *validate_body().lock().expect("body") = TEST_BODY.to_vec();
    *validate_etag().lock().expect("etag") = b"etag1".to_vec();
    TEST_EXISTS.store(true, Ordering::SeqCst);
    OBS_SEQ.store(0, Ordering::SeqCst);
}

fn get_test(_req: Request<'_>) -> Response {
    if !TEST_EXISTS.load(Ordering::SeqCst) {
        return Response::not_found();
    }
    Response::content(TEST_BODY).content_format(ContentFormat::TEXT_PLAIN)
}

fn put_test(req: Request<'_>) -> Response {
    TEST_EXISTS.store(true, Ordering::SeqCst);
    let _ = req.payload();
    Response::changed()
}

fn post_test(_req: Request<'_>) -> Response {
    TEST_EXISTS.store(true, Ordering::SeqCst);
    Response::created()
}

fn delete_test(_req: Request<'_>) -> Response {
    TEST_EXISTS.store(false, Ordering::SeqCst);
    Response::deleted()
}

fn get_separate(_req: Request<'_>) -> Response {
    Response::content(SEP_BODY).content_format(ContentFormat::TEXT_PLAIN)
}

fn get_query(_req: Request<'_>) -> Response {
    Response::content(TEST_BODY).content_format(ContentFormat::TEXT_PLAIN)
}

fn get_seg(_req: Request<'_>) -> Response {
    Response::content(TEST_BODY).content_format(ContentFormat::TEXT_PLAIN)
}

fn get_validate(req: Request<'_>) -> Response {
    let etag = validate_etag().lock().expect("etag").clone();
    let body = validate_body().lock().expect("body").clone();
    let tags: Vec<Vec<u8>> = req.etag().map(|t| t.to_vec()).collect();
    if tags.iter().any(|t| t == &etag) {
        return Response::valid().etag(&etag);
    }
    let mut r = Response::content_copy(&body)
        .content_format(ContentFormat::TEXT_PLAIN)
        .etag(&etag);
    if req.accept().is_some() {
        r = r.content_format(ContentFormat::TEXT_PLAIN);
    }
    r
}

fn put_validate(req: Request<'_>) -> Response {
    let etag = validate_etag().lock().expect("etag").clone();
    let exists = true;
    match req.precondition(exists, Some(etag.as_slice())) {
        Precondition::Unconditional | Precondition::Satisfied => {
            *validate_body().lock().expect("body") = req.payload().to_vec();
            Response::changed()
        }
        Precondition::FailedIfMatch | Precondition::FailedIfNoneMatch => {
            Response::precondition_failed()
        }
    }
}

fn get_large(_req: Request<'_>) -> Response {
    // 'static leak once — Default TX body is 4096; 2000 must Block2.
    static LARGE: OnceLock<Vec<u8>> = OnceLock::new();
    let body = LARGE.get_or_init(large_body);
    // Response::content wants 'static; copy into a leaked box for the handler.
    // The harness is a test crate; this is once per process.
    static LEAK: OnceLock<&'static [u8]> = OnceLock::new();
    let slice = *LEAK.get_or_init(|| body.clone().leak());
    Response::content(slice).content_format(ContentFormat::TEXT_PLAIN)
}

fn put_large(req: Request<'_>) -> Response {
    let _ = req.body().or(Some(req.payload()));
    Response::changed()
}

fn post_large_create(req: Request<'_>) -> Response {
    let _ = req.body().or(Some(req.payload()));
    Response::created()
}

fn post_large_post(req: Request<'_>) -> Response {
    let _ = req.body().or(Some(req.payload()));
    static LEAK: OnceLock<&'static [u8]> = OnceLock::new();
    let slice = *LEAK.get_or_init(|| large_body().leak());
    Response::new(Code::CHANGED)
        .with_static(slice)
        .content_format(ContentFormat::TEXT_PLAIN)
}

fn get_obs(_req: Request<'_>) -> Response {
    Response::content(OBS_BODY)
        .content_format(ContentFormat::TEXT_PLAIN)
        .observe(0)
        .max_age(5)
}

fn obs_snapshot() -> Response {
    Response::content(OBS_BODY_2)
        .content_format(ContentFormat::TEXT_PLAIN)
        .observe(OBS_SEQ.fetch_add(1, Ordering::SeqCst).saturating_add(1))
}

fn delete_obs(_req: Request<'_>) -> Response {
    Response::deleted()
}

fn get_link1(_req: Request<'_>) -> Response {
    Response::content(b"link1")
}

fn get_path(_req: Request<'_>) -> Response {
    Response::content(PATH_LINKS.as_bytes()).content_format(ContentFormat::LINK_FORMAT)
}

fn get_path_sub1(_req: Request<'_>) -> Response {
    Response::content(PATH_SUB1)
}

fn get_secure(_req: Request<'_>) -> Response {
    Response::content(SECURE_BODY).content_format(ContentFormat::TEXT_PLAIN)
}

fn well_known(req: Request<'_>) -> Response {
    let queries: Vec<String> = req
        .uri_query()
        .filter_map(|s| s.ok().map(str::to_owned))
        .collect();
    let payload = filter_catalog(&queries);
    Response::content_copy(payload.as_bytes()).content_format(ContentFormat::LINK_FORMAT)
}

/// Filter [`LINK_CATALOG`] by RFC 6690 query keys (`rt`, `if`, `sz`, `href`).
#[must_use]
pub fn filter_catalog(queries: &[String]) -> String {
    if queries.is_empty() {
        return LINK_CATALOG.join(",");
    }
    LINK_CATALOG
        .iter()
        .filter(|link| queries.iter().all(|q| link_matches(link, q)))
        .copied()
        .collect::<Vec<_>>()
        .join(",")
}

fn link_matches(link: &str, query: &str) -> bool {
    let Some((key, value)) = query.split_once('=') else {
        return false;
    };
    match key {
        "href" => href_match(link, value),
        "rt" => attr_match(link, "rt", value),
        "if" => attr_match(link, "if", value),
        "sz" => {
            if value == "*" {
                link.contains("sz=")
            } else {
                link.contains(&format!("sz={value}"))
            }
        }
        _ => false,
    }
}

fn href_match(link: &str, value: &str) -> bool {
    let href = link
        .split('>')
        .next()
        .and_then(|s| s.strip_prefix('<'))
        .unwrap_or("");
    if let Some(prefix) = value.strip_suffix('*') {
        href.starts_with(prefix)
    } else {
        href == value
    }
}

fn attr_match(link: &str, attr: &str, value: &str) -> bool {
    let needle = format!("{attr}=");
    let Some(start) = link.find(&needle) else {
        return false;
    };
    if value == "*" {
        return true;
    }
    let rest = &link[start + needle.len()..];
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    let end = rest.find('"').unwrap_or(rest.len());
    let listed = &rest[..end];
    if let Some(prefix) = value.strip_suffix('*') {
        listed.split_whitespace().any(|t| t.starts_with(prefix))
    } else {
        listed.split_whitespace().any(|t| t == value)
    }
}

/// Method router table for the coaptic App server (24 slots).
pub fn routers() -> [(&'static str, MethodRouter); 16] {
    [
        (
            "test",
            get(get_test)
                .put(put_test)
                .post(post_test)
                .delete(delete_test),
        ),
        ("separate", get(get_separate)),
        ("query", get(get_query)),
        ("validate", get(get_validate).put(put_validate)),
        ("seg1/seg2/seg3", get(get_seg)),
        ("large", get(get_large)),
        ("large-update", put(put_large)),
        ("large-create", post(post_large_create)),
        ("large-post", post(post_large_post)),
        ("obs", get(get_obs).delete(delete_obs).observe(obs_snapshot)),
        ("obs-non", get(get_obs).observe(obs_snapshot)),
        ("link1", get(get_link1)),
        ("link2", get(get_link1)),
        ("link3", get(get_link1)),
        ("path", get(get_path)),
        ("path/sub1", get(get_path_sub1)),
    ]
}

/// Extra routes that do not fit the array above.
pub fn extra_routers() -> [(&'static str, MethodRouter); 2] {
    [
        (".well-known/core", get(well_known)),
        ("secure", get(get_secure)),
    ]
}
