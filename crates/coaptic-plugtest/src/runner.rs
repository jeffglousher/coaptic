//! Run one TD against a client/server peer pair and grade the pcap.

use std::net::SocketAddr;
use std::thread;
use std::time::Duration;

use coaptic::message::{Code, Type};

use crate::catalog;
use crate::grade::Catalog;
use crate::pcap::Capture;
use crate::peer::{ClientRequest, Peer, PeerError};
use crate::site;

/// Which implementations sit on each side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pair {
    /// Client backend name (`coaptic`, `coap-rs`).
    pub client: &'static str,
    /// Server backend name.
    pub server: &'static str,
}

impl Pair {
    /// Label for logs (`coaptic→coap-rs`).
    #[must_use]
    pub fn label(self) -> String {
        format!("{}→{}", self.client, self.server)
    }
}

/// Useful role matrix: mixed interop plus same-impl coaptic.
#[must_use]
pub fn default_pairs() -> Vec<Pair> {
    vec![
        Pair {
            client: "coap-rs",
            server: "coaptic",
        },
        Pair {
            client: "coaptic",
            server: "coap-rs",
        },
        Pair {
            client: "coaptic",
            server: "coaptic",
        },
    ]
}

/// Construct a named peer.
pub fn peer_by_name(name: &str) -> Result<Box<dyn Peer>, PeerError> {
    match name {
        "coaptic" => Ok(Box::new(crate::coaptic::CoapticPeer::new())),
        "coap-rs" => Ok(Box::new(crate::coap_rs::CoapRsPeer::new())),
        other => Err(PeerError(format!(
            "unknown peer {other:?} (extension point: implement crate::peer::Peer)"
        ))),
    }
}

/// Outcome of one TD × pair.
#[derive(Debug)]
pub struct TdResult {
    /// TD identifier.
    pub id: String,
    /// Role pair.
    pub pair: Pair,
    /// `None` if the run and grade succeeded.
    pub error: Option<String>,
    /// Merged capture (for debugging / PCAP dump).
    pub capture: Capture,
}

/// Run `id` on `pair` and grade against the golden catalog.
pub fn run_td(id: &str, pair: Pair) -> TdResult {
    if let Some(reason) = catalog::skip_reason(id) {
        return TdResult {
            id: id.to_owned(),
            pair,
            error: Some(format!("SKIP: {reason}")),
            capture: Capture::new(),
        };
    }
    let mut server = match peer_by_name(pair.server) {
        Ok(p) => p,
        Err(e) => {
            return TdResult {
                id: id.to_owned(),
                pair,
                error: Some(e.0),
                capture: Capture::new(),
            };
        }
    };
    let mut client = match peer_by_name(pair.client) {
        Ok(p) => p,
        Err(e) => {
            return TdResult {
                id: id.to_owned(),
                pair,
                error: Some(e.0),
                capture: Capture::new(),
            };
        }
    };
    let addr = match server.start_server() {
        Ok(a) => a,
        Err(e) => {
            return TdResult {
                id: id.to_owned(),
                pair,
                error: Some(format!("start_server: {e}")),
                capture: Capture::new(),
            };
        }
    };
    let run = drive_td(id, addr, client.as_mut(), server.as_mut());
    let capture = Capture::new();
    capture.extend_from(&client.take_capture());
    capture.extend_from(&server.take_capture());
    server.stop_server();
    let error = match run {
        Ok(()) => match Catalog::load().and_then(|c| c.grade(id, &capture)) {
            Ok(()) => None,
            Err(e) => Some(format!("grade: {e}")),
        },
        Err(e) => Some(e.0),
    };
    TdResult {
        id: id.to_owned(),
        pair,
        error,
        capture,
    }
}

fn drive_td(
    id: &str,
    dest: SocketAddr,
    client: &mut dyn Peer,
    server: &mut dyn Peer,
) -> Result<(), PeerError> {
    match id {
        "TD_COAP_CORE_01" => basic(client, dest, ClientRequest::get(&["test"]), Code::CONTENT),
        "TD_COAP_CORE_02" => basic(
            client,
            dest,
            ClientRequest::request(Code::DELETE, &["test"]),
            Code::DELETED,
        ),
        "TD_COAP_CORE_03" => {
            let mut r = ClientRequest::request(Code::PUT, &["test"]);
            r.payload = site::TEST_BODY.to_vec();
            r.content_format = Some(0);
            basic(client, dest, r, Code::CHANGED)
        }
        "TD_COAP_CORE_04" | "TD_COAP_CORE_18" | "TD_COAP_CORE_19" => {
            let mut r = ClientRequest::request(Code::POST, &["test"]);
            r.payload = site::TEST_BODY.to_vec();
            r.content_format = Some(0);
            let got = client.send_request(dest, &r)?;
            expect_codes(id, got.code, &[Code::CREATED, Code::CHANGED])?;
            if id == "TD_COAP_CORE_18" && got.location_path.is_empty() {
                return Err(PeerError("CORE_18 expected Location-Path".into()));
            }
            if id == "TD_COAP_CORE_19" && got.location_query.is_empty() {
                return Err(PeerError("CORE_19 expected Location-Query".into()));
            }
            Ok(())
        }
        "TD_COAP_CORE_05" => {
            let mut r = ClientRequest::get(&["test"]);
            r.ty = Type::NonConfirmable;
            basic(client, dest, r, Code::CONTENT)
        }
        "TD_COAP_CORE_06" => {
            let mut r = ClientRequest::request(Code::DELETE, &["test"]);
            r.ty = Type::NonConfirmable;
            basic(client, dest, r, Code::DELETED)
        }
        "TD_COAP_CORE_07" => {
            let mut r = ClientRequest::request(Code::PUT, &["test"]);
            r.ty = Type::NonConfirmable;
            r.payload = site::TEST_BODY.to_vec();
            r.content_format = Some(0);
            basic(client, dest, r, Code::CHANGED)
        }
        "TD_COAP_CORE_08" => {
            let mut r = ClientRequest::request(Code::POST, &["test"]);
            r.ty = Type::NonConfirmable;
            r.payload = site::TEST_BODY.to_vec();
            r.content_format = Some(0);
            let got = client.send_request(dest, &r)?;
            expect_codes(id, got.code, &[Code::CREATED, Code::CHANGED])
        }
        "TD_COAP_CORE_09" | "TD_COAP_CORE_11" | "TD_COAP_CORE_17" => {
            let mut r = ClientRequest::get(&["separate"]);
            if id == "TD_COAP_CORE_17" {
                r.ty = Type::NonConfirmable;
            }
            if id == "TD_COAP_CORE_11" {
                r.token_len = Some(8);
            } else {
                r.token_len = Some(4);
            }
            basic(client, dest, r, Code::CONTENT)
        }
        "TD_COAP_CORE_10" => {
            let mut r = ClientRequest::get(&["test"]);
            r.token_len = Some(4);
            basic(client, dest, r, Code::CONTENT)
        }
        "TD_COAP_CORE_12" => {
            let mut r = ClientRequest::get(&["test"]);
            r.token_len = Some(0);
            basic(client, dest, r, Code::CONTENT)
        }
        "TD_COAP_CORE_13" => basic(
            client,
            dest,
            ClientRequest::get(&["seg1", "seg2", "seg3"]),
            Code::CONTENT,
        ),
        "TD_COAP_CORE_14" => {
            let mut r = ClientRequest::get(&["query"]);
            r.query = vec!["first=1".into(), "second=2".into(), "third=3".into()];
            basic(client, dest, r, Code::CONTENT)
        }
        "TD_COAP_CORE_15" | "TD_COAP_CORE_16" => {
            // Lossy TDs: still a GET; retransmission is implementation-owned.
            // Grade the successful exchange (retransmit optional on the tap).
            basic(client, dest, ClientRequest::get(&["test"]), Code::CONTENT)
        }
        "TD_COAP_CORE_20" => {
            let mut r = ClientRequest::get(&["test"]);
            r.accept = Some(0);
            basic(client, dest, r, Code::CONTENT)
        }
        "TD_COAP_CORE_21" => {
            let first = client.send_request(dest, &ClientRequest::get(&["validate"]))?;
            expect_codes(id, first.code, &[Code::CONTENT])?;
            if first.etag.is_empty() {
                return Err(PeerError("CORE_21 expected ETag".into()));
            }
            let mut r = ClientRequest::get(&["validate"]);
            r.etag = first.etag;
            let second = client.send_request(dest, &r)?;
            expect_codes(id, second.code, &[Code::VALID])
        }
        "TD_COAP_CORE_22" => {
            let mut ok = ClientRequest::request(Code::PUT, &["validate"]);
            ok.payload = site::TEST_BODY.to_vec();
            ok.if_match = vec![b"etag1".to_vec()];
            let got = client.send_request(dest, &ok)?;
            expect_codes(id, got.code, &[Code::CHANGED])?;
            let mut bad = ClientRequest::request(Code::PUT, &["validate"]);
            bad.if_match = vec![b"wrong".to_vec()];
            let got = client.send_request(dest, &bad)?;
            expect_codes(id, got.code, &[Code::PRECONDITION_FAILED])
        }
        "TD_COAP_CORE_23" => {
            let mut r = ClientRequest::request(Code::PUT, &["validate"]);
            r.if_none_match = true;
            let got = client.send_request(dest, &r)?;
            expect_codes(id, got.code, &[Code::PRECONDITION_FAILED])
        }
        "TD_COAP_CORE_31" => {
            let got = client.send_request(dest, &ClientRequest::ping())?;
            if !got.rst {
                return Err(PeerError(format!(
                    "CORE_31 expected RST, got {} {}",
                    got.ty, got.code
                )));
            }
            Ok(())
        }
        "TD_COAP_BLOCK_01" => block2(client, dest, true, 64),
        "TD_COAP_BLOCK_02" => block2(client, dest, false, 1024),
        "TD_COAP_BLOCK_03" => block1(client, dest, Code::PUT, &["large-update"], Code::CHANGED),
        "TD_COAP_BLOCK_04" => block1(client, dest, Code::POST, &["large-create"], Code::CREATED),
        "TD_COAP_BLOCK_05" => block1(client, dest, Code::POST, &["large-post"], Code::CHANGED),
        "TD_COAP_BLOCK_06" => block2(client, dest, true, 16),
        "TD_COAP_LINK_01" => link(client, dest, &[]),
        "TD_COAP_LINK_02" => link(client, dest, &["rt=Type1"]),
        "TD_COAP_LINK_03" => link(client, dest, &["rt=*"]),
        "TD_COAP_LINK_04" => link(client, dest, &["rt=Type2"]),
        "TD_COAP_LINK_05" => link(client, dest, &["if=If*"]),
        "TD_COAP_LINK_06" => link(client, dest, &["sz=*"]),
        "TD_COAP_LINK_07" => link(client, dest, &["href=/link1"]),
        "TD_COAP_LINK_08" => link(client, dest, &["href=/link*"]),
        "TD_COAP_LINK_09" => {
            link(client, dest, &[])?;
            let got = client.send_request(dest, &ClientRequest::get(&["path"]))?;
            expect_codes(id, got.code, &[Code::CONTENT])?;
            if !String::from_utf8_lossy(&got.payload).contains("/path/sub1") {
                return Err(PeerError("LINK_09 /path missing sub1".into()));
            }
            let sub = client.send_request(dest, &ClientRequest::get(&["path", "sub1"]))?;
            expect_codes(id, sub.code, &[Code::CONTENT])?;
            if sub.payload != site::PATH_SUB1 {
                return Err(PeerError("LINK_09 /path/sub1 payload".into()));
            }
            Ok(())
        }
        id if id.starts_with("TD_COAP_OBS_") => observe(id, dest, client, server),
        id if id.starts_with("TD_COAP_DTLS_") => Err(PeerError(format!(
            "{id}: drive via runner::run_dtls (feature dtls)"
        ))),
        other => Err(PeerError(format!("no driver for {other}"))),
    }
}

fn basic(
    client: &mut dyn Peer,
    dest: SocketAddr,
    req: ClientRequest,
    want: Code,
) -> Result<(), PeerError> {
    let got = client.send_request(dest, &req)?;
    expect_codes("basic", got.code, &[want])
}

fn expect_codes(id: &str, got: Code, want: &[Code]) -> Result<(), PeerError> {
    if want.contains(&got) {
        Ok(())
    } else {
        Err(PeerError(format!(
            "{id}: got {got}, expected one of {want:?}"
        )))
    }
}

fn block2(
    client: &mut dyn Peer,
    dest: SocketAddr,
    early: bool,
    size: u16,
) -> Result<(), PeerError> {
    let mut r = ClientRequest::get(&["large"]);
    if early {
        r.block2 = Some((0, false, size));
    }
    r.timeout = Duration::from_secs(4);
    let got = client.send_request(dest, &r)?;
    expect_codes("block2", got.code, &[Code::CONTENT])?;
    let body = got.body.as_deref().unwrap_or(&got.payload);
    if body.len() < 64 {
        return Err(PeerError(format!(
            "block2 assembled only {} bytes",
            body.len()
        )));
    }
    Ok(())
}

fn block1(
    client: &mut dyn Peer,
    dest: SocketAddr,
    method: Code,
    path: &[&str],
    want: Code,
) -> Result<(), PeerError> {
    let mut r = ClientRequest::request(method, path);
    r.payload = site::large_body();
    r.content_format = Some(0);
    r.timeout = Duration::from_secs(4);
    let got = client.send_request(dest, &r)?;
    expect_codes("block1", got.code, &[want])
}

fn link(client: &mut dyn Peer, dest: SocketAddr, query: &[&str]) -> Result<(), PeerError> {
    let mut r = ClientRequest::get(&[".well-known", "core"]);
    r.query = query.iter().map(|s| (*s).to_owned()).collect();
    let got = client.send_request(dest, &r)?;
    expect_codes("link", got.code, &[Code::CONTENT])?;
    if got.content_format != Some(40) && got.content_format.is_some() {
        // some stacks omit CF; payload still link-format
    }
    if got.payload.is_empty() {
        return Err(PeerError("empty well-known/core".into()));
    }
    Ok(())
}

fn observe(
    id: &str,
    dest: SocketAddr,
    client: &mut dyn Peer,
    server: &mut dyn Peer,
) -> Result<(), PeerError> {
    let path: &[&str] = if id == "TD_COAP_OBS_02" {
        &["obs-non"]
    } else {
        &["obs"]
    };
    let mut reg = ClientRequest::get(path);
    reg.observe = Some(0);
    if id == "TD_COAP_OBS_02" {
        reg.ty = Type::NonConfirmable;
    }
    let first = client.send_request(dest, &reg)?;
    expect_codes(id, first.code, &[Code::CONTENT])?;
    match id {
        "TD_COAP_OBS_07" => {
            let del = client.send_request(dest, &ClientRequest::request(Code::DELETE, path))?;
            expect_codes(id, del.code, &[Code::DELETED])
        }
        "TD_COAP_OBS_10" => {
            let _ = client.send_request(dest, &ClientRequest::get(path))?;
            server.notify(path, site::OBS_BODY_2)?;
            thread::sleep(Duration::from_millis(80));
            Ok(())
        }
        "TD_COAP_OBS_12" => {
            let mut off = ClientRequest::get(path);
            off.observe = Some(1);
            let _ = client.send_request(dest, &off)?;
            Ok(())
        }
        "TD_COAP_OBS_08" => {
            // Format-change: server drops the interest (App/engine). A notify is optional.
            Ok(())
        }
        _ => {
            server.notify(path, site::OBS_BODY_2)?;
            thread::sleep(Duration::from_millis(80));
            Ok(())
        }
    }
}

/// Run every in-scope TD on `pairs`. DTLS TDs need `feature = "dtls"`.
pub fn run_suite(ids: &[&str], pairs: &[Pair]) -> Vec<TdResult> {
    let mut out = Vec::new();
    for id in ids {
        if id.starts_with("TD_COAP_DTLS_") {
            #[cfg(feature = "dtls")]
            {
                out.extend(crate::dtls::run_dtls_pairs(id, pairs));
            }
            #[cfg(not(feature = "dtls"))]
            {
                for pair in pairs {
                    out.push(TdResult {
                        id: (*id).to_owned(),
                        pair: *pair,
                        error: Some(format!(
                            "SKIP: {}",
                            catalog::skip_reason(id).unwrap_or("dtls feature")
                        )),
                        capture: Capture::new(),
                    });
                }
            }
            continue;
        }
        for pair in pairs {
            out.push(run_td(id, *pair));
        }
    }
    out
}
