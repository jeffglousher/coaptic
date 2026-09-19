//! Deterministic pcap grader: CoAP fields, wildcard MID / Token / ports / time.

use std::collections::BTreeMap;

use coaptic::message::{Code, ParsedMessage, Type, decode};
use serde::{Deserialize, Serialize};

use crate::pcap::{Capture, Packet};

/// One expected CoAP datagram (golden JSON).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExpectPacket {
    /// `CON` / `NON` / `ACK` / `RST`. Omit to accept any type.
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    /// `0.01`, `2.05`, or a list of acceptable codes.
    #[serde(default)]
    pub code: Option<ExpectCode>,
    /// `*` (any) or `echo` (same as the first request in this TD).
    #[serde(default)]
    pub mid: Option<String>,
    /// `*` or `echo`.
    #[serde(default)]
    pub token: Option<String>,
    /// Uri-Path segments that must be present (order matters).
    #[serde(default)]
    pub uri_path: Option<Vec<String>>,
    /// Uri-Query values that must be present (order matters).
    #[serde(default)]
    pub uri_query: Option<Vec<String>>,
    /// Content-Format numeric value, if required.
    #[serde(default)]
    pub content_format: Option<u16>,
    /// Observe: `register` / `deregister` / `present` / `absent`.
    #[serde(default)]
    pub observe: Option<String>,
    /// Block2: `{ "num": 0, "more": true }` (size is not graded).
    #[serde(default)]
    pub block2: Option<ExpectBlock>,
    /// Block1: same shape as [`block2`](Self::block2).
    #[serde(default)]
    pub block1: Option<ExpectBlock>,
    /// Location-Path segments that must appear.
    #[serde(default)]
    pub location_path: Option<Vec<String>>,
    /// Location-Query values that must appear.
    #[serde(default)]
    pub location_query: Option<Vec<String>>,
    /// `empty` / `nonempty` / exact UTF-8 / `{ "contains": "…" }`.
    #[serde(default)]
    pub payload: Option<ExpectPayload>,
    /// Accept option numeric value.
    #[serde(default)]
    pub accept: Option<u16>,
    /// If-None-Match present.
    #[serde(default)]
    pub if_none_match: Option<bool>,
}

/// One or many acceptable codes (`"2.05"` or `["2.01","2.04"]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpectCode {
    /// Single `class.detail` string.
    One(String),
    /// Any of these codes.
    Any(Vec<String>),
}

/// Block NUM / M. SZX is not graded (implementations pick legal sizes).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpectBlock {
    /// Block number, if required.
    #[serde(default)]
    pub num: Option<u32>,
    /// More flag, if required.
    #[serde(default)]
    pub more: Option<bool>,
}

/// Payload expectation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpectPayload {
    /// `empty`, `nonempty`, or exact UTF-8.
    Word(String),
    /// Substring / hex.
    Obj {
        /// UTF-8 substring.
        #[serde(default)]
        contains: Option<String>,
        /// Exact bytes as hex (`636f7265`).
        #[serde(default)]
        hex: Option<String>,
    },
}

/// Optional DTLS handshake checks.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExpectDtls {
    /// `success` or `alert`.
    #[serde(default)]
    pub handshake: Option<String>,
    /// Cipher suite name that must appear in ClientHello (`TLS_PSK_WITH_AES_128_CCM_8`).
    #[serde(default)]
    pub client_hello_contains: Option<String>,
    /// Cipher suite selected in ServerHello.
    #[serde(default)]
    pub server_hello: Option<String>,
}

/// Golden record for one TD.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExpectTd {
    /// CoAP datagrams in order (retransmits / extra ACKs may sit between).
    #[serde(default)]
    pub coap: Vec<ExpectPacket>,
    /// DTLS handshake (ignored when the TD is plaintext).
    #[serde(default)]
    pub dtls: Option<ExpectDtls>,
    /// Extra CoAP messages after the last expected one are allowed.
    ///
    /// CORE goldens omit this (default `false`) and assert type / token echo /
    /// CON↔ACK MID. Chatty suites (OBS notifications, Block trains, LINK
    /// follow-ups, DTLS) set it: client+server taps duplicate datagrams, and
    /// leftover trains exceed the grader's slack.
    #[serde(default)]
    pub allow_extra: bool,
}

/// Whole golden file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    /// TD id → expectation.
    #[serde(flatten)]
    pub tds: BTreeMap<String, ExpectTd>,
}

impl Catalog {
    /// Parse [`crate::CATALOG_JSON`].
    pub fn load() -> Result<Self, String> {
        serde_json::from_str(crate::CATALOG_JSON).map_err(|e| e.to_string())
    }

    /// Grade `capture` against `td`.
    pub fn grade(&self, td: &str, capture: &Capture) -> Result<(), String> {
        let expect = self
            .tds
            .get(td)
            .ok_or_else(|| format!("{td}: missing golden expectation"))?;
        grade_td(td, expect, capture)
    }
}

/// Grade one TD.
pub fn grade_td(td: &str, expect: &ExpectTd, capture: &Capture) -> Result<(), String> {
    let packets = capture.snapshot();
    if let Some(ref dtls) = expect.dtls {
        grade_dtls(td, dtls, &packets)?;
    }
    let coap: Vec<(usize, ParsedView)> = packets
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            if expect.dtls.is_some() && !p.decrypted {
                // Prefer decrypted CoAP when DTLS is in play; skip handshake records.
                return None;
            }
            ParsedView::decode(&p.bytes).map(|v| (i, v))
        })
        .collect();

    let mut echo_mid: Option<u16> = None;
    let mut echo_tok: Option<Vec<u8>> = None;
    // `cursor` is an index into the filtered `coap` list, not a raw packet
    // index. Non-CoAP records (DTLS, garbage) must not skip real replies.
    let mut cursor = 0usize;
    for (step, exp) in expect.coap.iter().enumerate() {
        let mut found = None;
        for (list_i, view) in coap.iter().map(|(_, v)| v).enumerate().skip(cursor) {
            if match_packet(view, exp, echo_mid, echo_tok.as_deref()) {
                found = Some((list_i, view.clone()));
                cursor = list_i + 1;
                break;
            }
        }
        let Some((_idx, view)) = found else {
            return Err(format!(
                "{td} step {step}: no CoAP datagram matched {exp:?} (saw {} after cursor {cursor})",
                describe_remaining(&coap, cursor)
            ));
        };
        if echo_mid.is_none()
            && (view.code.is_request() || (view.code.is_empty() && view.ty == Type::Confirmable))
        {
            echo_mid = Some(view.mid);
            echo_tok = Some(view.token.clone());
        }
    }
    if !expect.allow_extra {
        let leftover: Vec<_> = coap
            .iter()
            .skip(cursor)
            .filter(|(_, v)| !v.code.is_empty() || v.ty == Type::Reset)
            .collect();
        if leftover.len() > 4 {
            return Err(format!(
                "{td}: {n} leftover CoAP datagrams (set allow_extra if this TD is chatty)",
                n = leftover.len()
            ));
        }
    }
    Ok(())
}

fn describe_remaining(coap: &[(usize, ParsedView)], cursor: usize) -> String {
    coap.iter()
        .skip(cursor)
        .take(6)
        .map(|(_, v)| format!("{} {}", v.ty, v.code))
        .collect::<Vec<_>>()
        .join(", ")
}

fn match_packet(
    view: &ParsedView,
    exp: &ExpectPacket,
    echo_mid: Option<u16>,
    echo_tok: Option<&[u8]>,
) -> bool {
    if let Some(ref ty) = exp.ty {
        if !type_matches(view.ty, ty) {
            return false;
        }
    }
    if let Some(ref codes) = exp.code {
        if !code_matches(view.code, codes) {
            return false;
        }
    }
    if let Some(ref mid) = exp.mid {
        match mid.as_str() {
            "*" => {}
            "echo" => {
                if echo_mid != Some(view.mid) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    if let Some(ref tok) = exp.token {
        match tok.as_str() {
            "*" => {}
            "echo" => {
                if echo_tok != Some(view.token.as_slice()) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    if let Some(ref path) = exp.uri_path {
        if &view.uri_path != path {
            return false;
        }
    }
    if let Some(ref query) = exp.uri_query {
        if &view.uri_query != query {
            return false;
        }
    }
    if let Some(cf) = exp.content_format {
        if view.content_format != Some(cf) {
            return false;
        }
    }
    if let Some(acc) = exp.accept {
        if view.accept != Some(acc) {
            return false;
        }
    }
    if let Some(flag) = exp.if_none_match {
        if view.if_none_match != flag {
            return false;
        }
    }
    if let Some(ref obs) = exp.observe {
        match obs.as_str() {
            "register" if view.observe != Some(0) => return false,
            "deregister" if view.observe != Some(1) => return false,
            "present" if view.observe.is_none() => return false,
            "absent" if view.observe.is_some() => return false,
            "register" | "deregister" | "present" | "absent" => {}
            _ => return false,
        }
    }
    if let Some(ref b) = exp.block2 {
        if !block_matches(view.block2, b) {
            return false;
        }
    }
    if let Some(ref b) = exp.block1 {
        if !block_matches(view.block1, b) {
            return false;
        }
    }
    if let Some(ref loc) = exp.location_path {
        if &view.location_path != loc {
            return false;
        }
    }
    if let Some(ref loc) = exp.location_query {
        if &view.location_query != loc {
            return false;
        }
    }
    if let Some(ref pay) = exp.payload {
        if !payload_matches(&view.payload, pay) {
            return false;
        }
    }
    true
}

fn type_matches(ty: Type, want: &str) -> bool {
    match want {
        "CON" => ty == Type::Confirmable,
        "NON" => ty == Type::NonConfirmable,
        "ACK" => ty == Type::Acknowledgement,
        "RST" => ty == Type::Reset,
        _ => false,
    }
}

fn code_matches(code: Code, want: &ExpectCode) -> bool {
    let got = format!("{code}");
    match want {
        ExpectCode::One(s) => got == *s,
        ExpectCode::Any(v) => v.iter().any(|s| s == &got),
    }
}

fn block_matches(got: Option<(u32, bool)>, want: &ExpectBlock) -> bool {
    let Some((num, more)) = got else {
        return false;
    };
    if want.num.is_some_and(|n| n != num) {
        return false;
    }
    if want.more.is_some_and(|m| m != more) {
        return false;
    }
    true
}

fn payload_matches(got: &[u8], want: &ExpectPayload) -> bool {
    match want {
        ExpectPayload::Word(w) if w == "empty" => got.is_empty(),
        ExpectPayload::Word(w) if w == "nonempty" => !got.is_empty(),
        ExpectPayload::Word(w) => got == w.as_bytes(),
        ExpectPayload::Obj { contains, hex } => {
            (contains.is_some() || hex.is_some())
                && contains
                    .as_ref()
                    .is_none_or(|s| std::str::from_utf8(got).is_ok_and(|t| t.contains(s)))
                && hex
                    .as_ref()
                    .is_none_or(|h| hex_decode(h).is_some_and(|b| b == got))
        }
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.is_ascii() || s.len() % 2 != 0 {
        return None;
    }
    s.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn grade_dtls(td: &str, exp: &ExpectDtls, packets: &[Packet]) -> Result<(), String> {
    let wire: Vec<&[u8]> = packets
        .iter()
        .filter(|p| !p.decrypted)
        .map(|p| p.bytes.as_slice())
        .collect();
    if let Some(ref name) = exp.client_hello_contains {
        let suite = cipher_id(name).ok_or_else(|| format!("{td}: unknown cipher {name}"))?;
        let ok = wire.iter().any(|b| client_hello_has_suite(b, suite));
        if !ok {
            return Err(format!(
                "{td}: ClientHello did not advertise {name} (or handshake not on the tap)"
            ));
        }
    }
    if let Some(ref name) = exp.server_hello {
        let suite = cipher_id(name).ok_or_else(|| format!("{td}: unknown cipher {name}"))?;
        let ok = wire.iter().any(|b| server_hello_suite(b) == Some(suite));
        if !ok {
            return Err(format!(
                "{td}: ServerHello did not select {name} (or handshake not on the tap)"
            ));
        }
    }
    match exp.handshake.as_deref() {
        Some("success") => {
            // A decrypted request alone does not prove a completed exchange.
            if !packets.iter().any(|p| {
                p.decrypted && ParsedView::decode(&p.bytes).is_some_and(|v| v.code.is_response())
            }) {
                return Err(format!("{td}: no captured decrypted CoAP response"));
            }
        }
        Some("alert") => {
            // Only plaintext fatal alerts can be interpreted from this tap.
            // Encrypted alert contents require authenticated backend evidence.
            if !wire.iter().any(|b| plaintext_fatal_alert(b)) {
                return Err(format!("{td}: no captured plaintext fatal DTLS alert"));
            }
        }
        None => {}
        Some(value) => return Err(format!("{td}: unknown handshake expectation {value}")),
    }
    Ok(())
}

fn cipher_id(name: &str) -> Option<u16> {
    match name {
        "TLS_PSK_WITH_AES_128_CCM_8" => Some(0xC0A8),
        "TLS_ECDHE_ECDSA_WITH_AES_128_CCM_8" => Some(0xC0AE),
        _ => None,
    }
}

/// Minimal DTLS ClientHello cipher-suite scan (best-effort).
fn client_hello_has_suite(record: &[u8], suite: u16) -> bool {
    let Some(body) = dtls_handshake_body(record, 1) else {
        return false;
    };
    // version(2) random(32) session_id cookie cipher_suites
    if body.len() < 35 {
        return false;
    }
    let mut i = 34; // 2+32
    let sid = usize::from(body[i]);
    i += 1 + sid;
    if i >= body.len() {
        return false;
    }
    let cookie = usize::from(body[i]);
    i += 1 + cookie;
    if i + 2 > body.len() {
        return false;
    }
    let cs_len = usize::from(u16::from_be_bytes([body[i], body[i + 1]]));
    i += 2;
    if cs_len == 0 || cs_len % 2 != 0 {
        return false;
    }
    let Some(ciphers) = body.get(i..i + cs_len) else {
        return false;
    };
    ciphers
        .chunks_exact(2)
        .any(|c| u16::from_be_bytes([c[0], c[1]]) == suite)
}

fn server_hello_suite(record: &[u8]) -> Option<u16> {
    let body = dtls_handshake_body(record, 2)?;
    if body.len() < 35 {
        return None;
    }
    let mut i = 34;
    let sid = usize::from(*body.get(i)?);
    i += 1 + sid;
    if i + 2 > body.len() {
        return None;
    }
    Some(u16::from_be_bytes([body[i], body[i + 1]]))
}

// RFC 6347 sections 4.1 and 4.2.2. This grader deliberately accepts only
// complete, unfragmented epoch-zero handshake messages. It does not claim
// reassembly or encrypted-alert authentication.
fn plaintext_record(record: &[u8], content_type: u8) -> Option<&[u8]> {
    if record.len() < 13
        || record[0] != content_type
        || record[1..3] != [0xfe, 0xfd]
        || record[3..5] != [0, 0]
    {
        return None;
    }
    let n = usize::from(u16::from_be_bytes([record[11], record[12]]));
    record.get(13..13 + n)
}

fn plaintext_fatal_alert(record: &[u8]) -> bool {
    plaintext_record(record, 21).is_some_and(|body| body.len() == 2 && body[0] == 2 && body[1] != 0)
}

fn dtls_handshake_body(record: &[u8], msg_type: u8) -> Option<&[u8]> {
    let frag = plaintext_record(record, 22)?;
    if frag.len() < 12 || frag[0] != msg_type {
        return None;
    }
    let u24 = |b: &[u8]| usize::from(b[0]) << 16 | usize::from(b[1]) << 8 | usize::from(b[2]);
    let n = u24(&frag[1..4]);
    if frag[6..9] != [0, 0, 0] || u24(&frag[9..12]) != n || frag.len() != 12 + n {
        return None;
    }
    Some(&frag[12..])
}

#[derive(Clone, Debug)]
struct ParsedView {
    ty: Type,
    code: Code,
    mid: u16,
    token: Vec<u8>,
    uri_path: Vec<String>,
    uri_query: Vec<String>,
    content_format: Option<u16>,
    accept: Option<u16>,
    observe: Option<u32>,
    block2: Option<(u32, bool)>,
    block1: Option<(u32, bool)>,
    location_path: Vec<String>,
    location_query: Vec<String>,
    if_none_match: bool,
    payload: Vec<u8>,
}

impl ParsedView {
    fn decode(bytes: &[u8]) -> Option<Self> {
        let parsed: ParsedMessage<'_> = decode(bytes).ok()?;
        Some(Self::from_parsed(parsed))
    }

    fn from_parsed(parsed: ParsedMessage<'_>) -> Self {
        let uri_path = parsed
            .uri_path()
            .filter_map(|s| s.ok().map(str::to_owned))
            .collect();
        let uri_query = parsed
            .uri_query()
            .filter_map(|s| s.ok().map(str::to_owned))
            .collect();
        let location_path = parsed
            .location_path()
            .filter_map(|s| s.ok().map(str::to_owned))
            .collect();
        let location_query = parsed
            .location_query()
            .filter_map(|s| s.ok().map(str::to_owned))
            .collect();
        let content_format = parsed
            .content_format()
            .and_then(Result::ok)
            .map(|cf| cf.get());
        let accept = parsed.accept().and_then(Result::ok).map(|cf| cf.get());
        let observe = parsed.observe().and_then(Result::ok);
        let block2 = parsed
            .block2()
            .and_then(Result::ok)
            .map(|b| (b.num(), b.more()));
        let block1 = parsed
            .block1()
            .and_then(Result::ok)
            .map(|b| (b.num(), b.more()));
        Self {
            ty: parsed.ty(),
            code: parsed.code(),
            mid: parsed.message_id().get(),
            token: parsed.token().as_bytes().to_vec(),
            uri_path,
            uri_query,
            content_format,
            accept,
            observe,
            block2,
            block1,
            location_path,
            location_query,
            if_none_match: parsed.if_none_match(),
            payload: parsed.payload().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::message::{Ids, Message, Opt, OptionsBuilder, Token, Type, encode};
    use coaptic::{Code, ContentFormat};

    #[test]
    fn payload_constraints_are_conjunctive_and_malformed_hex_is_refused() {
        let mut want = ExpectPayload::Obj {
            contains: Some("hi".into()),
            hex: Some("6869".into()),
        };
        assert!(payload_matches(b"hi", &want));
        assert!(!payload_matches(b"hi there", &want));
        want = ExpectPayload::Obj {
            contains: None,
            hex: None,
        };
        assert!(!payload_matches(b"anything", &want));
        for bad in ["h0", "123", "a\u{20ac}", "\u{e9}"] {
            assert_eq!(hex_decode(bad), None);
        }
    }

    fn record(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut r = vec![kind, 0xfe, 0xfd, 0, 0, 0, 0, 0, 0, 0, 0];
        r.extend_from_slice(&(body.len() as u16).to_be_bytes());
        r.extend_from_slice(body);
        r
    }

    fn capture(bytes: &[u8], decrypted: bool) -> Capture {
        let cap = Capture::new();
        cap.push(
            "127.0.0.1:1".parse().unwrap(),
            "127.0.0.1:2".parse().unwrap(),
            bytes,
            decrypted,
        );
        cap
    }

    #[test]
    fn dtls_success_requires_decrypted_response() {
        let exp = ExpectTd {
            dtls: Some(ExpectDtls {
                handshake: Some("success".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(grade_td("success", &exp, &Capture::new()).is_err());
        let response = [0x60, 0x45, 0, 1];
        assert!(grade_td("success", &exp, &capture(&response, false)).is_err());
        assert!(grade_td("success", &exp, &capture(&[0x40, 1, 0, 1], true)).is_err());
        assert!(grade_td("success", &exp, &capture(&[0xff], true)).is_err());
        grade_td("success", &exp, &capture(&response, true)).unwrap();
    }

    #[test]
    fn dtls_alert_requires_visible_fatal_alert() {
        let exp = ExpectTd {
            dtls: Some(ExpectDtls {
                handshake: Some("alert".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(grade_td("alert", &exp, &Capture::new()).is_err());
        for body in [&[1, 40][..], &[2, 0], &[2], &[2, 40, 0]] {
            assert!(grade_td("alert", &exp, &capture(&record(21, body), false)).is_err());
        }
        let valid = record(21, &[2, 40]);
        grade_td("alert", &exp, &capture(&valid, false)).unwrap();
        for len in 0..valid.len() {
            assert!(grade_td("alert", &exp, &capture(&valid[..len], false)).is_err());
        }
        let mut encrypted = valid.clone();
        encrypted[4] = 1;
        assert!(grade_td("alert", &exp, &capture(&encrypted, false)).is_err());
        assert!(grade_td("alert", &exp, &capture(&valid, true)).is_err());
    }

    #[test]
    fn dtls_coap_expectations_reject_plaintext_injection() {
        let exp: ExpectTd =
            serde_json::from_str(r#"{"dtls":{},"coap":[{"code":"2.05"}]}"#).unwrap();
        let bytes = [0x60, 0x45, 0, 1];
        assert!(grade_td("protected", &exp, &capture(&bytes, false)).is_err());
        grade_td("protected", &exp, &capture(&bytes, true)).unwrap();
    }

    #[test]
    fn echo_requires_an_observed_request() {
        let exp: ExpectTd =
            serde_json::from_str(r#"{"coap":[{"code":"2.05","mid":"echo","token":"echo"}]}"#)
                .unwrap();
        assert!(
            grade_td(
                "missing request",
                &exp,
                &capture(&[0x60, 0x45, 0, 1], false)
            )
            .is_err()
        );
    }

    #[test]
    fn hello_scan_refuses_truncation_fragmentation_and_encrypted_records() {
        let mut body = vec![0; 34];
        body.extend_from_slice(&[0, 0, 0, 2, 0xc0, 0xa8, 1, 0]);
        let mut handshake = vec![
            1,
            0,
            0,
            body.len() as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            body.len() as u8,
        ];
        handshake.extend_from_slice(&body);
        let valid = record(22, &handshake);
        assert!(client_hello_has_suite(&valid, 0xc0a8));
        for len in 0..valid.len() {
            assert!(!client_hello_has_suite(&valid[..len], 0xc0a8));
        }
        for (index, value) in [(4, 1), (19, 1), (24, 1), (16, 1), (2, 0), (62, 3)] {
            let mut invalid = valid.clone();
            invalid[index] = value;
            assert!(
                !client_hello_has_suite(&invalid, 0xc0a8),
                "mutation {index}"
            );
        }
    }

    #[test]
    fn grades_core_01_shape() {
        let mut ids = Ids::new(1);
        let token = Token::from_checked(&[0xAB, 0xCD]);
        let mut opts = OptionsBuilder::<4>::new();
        opts.push(Opt::uri_path("test")).ok();
        let req = ids.con(Code::GET, token).with_options(opts.as_slice());
        let mut buf = [0u8; 64];
        let n = encode(&req, &mut buf).unwrap();
        let cap = Capture::new();
        let a: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: std::net::SocketAddr = "127.0.0.1:2".parse().unwrap();
        cap.push(a, b, &buf[..n], false);

        let cf = ContentFormat::TEXT_PLAIN.encode();
        let mut opts = OptionsBuilder::<4>::new();
        opts.push(Opt::content_format(&cf)).ok();
        let ack = Message::new(Type::Acknowledgement, Code::CONTENT, req.message_id())
            .with_token(token)
            .with_options(opts.as_slice())
            .with_payload(b"hi");
        let n = encode(&ack, &mut buf).unwrap();
        cap.push(b, a, &buf[..n], false);

        let json = r#"{
            "coap": [
                {"type":"CON","code":"0.01","mid":"*","token":"*","uri_path":["test"]},
                {"type":"ACK","code":"2.05","mid":"echo","token":"echo","content_format":0,"payload":"nonempty"}
            ]
        }"#;
        let exp: ExpectTd = serde_json::from_str(json).unwrap();
        grade_td("TD_COAP_CORE_01", &exp, &cap).expect("grade");
    }

    #[test]
    fn grades_core_31_empty_con_rst_mid_echo() {
        let mid = coaptic::message::MessageId::new(0x5049);
        let ping = Message::new(Type::Confirmable, Code::EMPTY, mid);
        let mut buf = [0u8; 16];
        let n = encode(&ping, &mut buf).unwrap();
        let cap = Capture::new();
        let a: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: std::net::SocketAddr = "127.0.0.1:2".parse().unwrap();
        cap.push(a, b, &buf[..n], false);
        let rst = Message::new(Type::Reset, Code::EMPTY, mid);
        let n = encode(&rst, &mut buf).unwrap();
        cap.push(b, a, &buf[..n], false);
        let json = r#"{
            "coap": [
                {"type":"CON","code":"0.00","payload":"empty"},
                {"type":"RST","code":"0.00","mid":"echo"}
            ]
        }"#;
        let exp: ExpectTd = serde_json::from_str(json).unwrap();
        grade_td("TD_COAP_CORE_31", &exp, &cap).expect("grade");
    }

    #[test]
    fn grades_past_leading_non_coap() {
        let mut ids = Ids::new(1);
        let token = Token::from_checked(&[0xAB, 0xCD]);
        let mut opts = OptionsBuilder::<4>::new();
        opts.push(Opt::uri_path("test")).ok();
        let req = ids.con(Code::GET, token).with_options(opts.as_slice());
        let mut buf = [0u8; 64];
        let n = encode(&req, &mut buf).unwrap();
        let cap = Capture::new();
        let a: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: std::net::SocketAddr = "127.0.0.1:2".parse().unwrap();
        cap.push(a, b, &[0x16, 0xfe, 0xfd, 0, 0, 0, 0], false);
        cap.push(a, b, &buf[..n], false);
        let ack = Message::new(Type::Acknowledgement, Code::CONTENT, req.message_id())
            .with_token(token)
            .with_payload(b"hi");
        let n = encode(&ack, &mut buf).unwrap();
        cap.push(b, a, &buf[..n], false);
        let json = r#"{
            "coap": [
                {"code":"0.01","uri_path":["test"]},
                {"code":"2.05"}
            ]
        }"#;
        let exp: ExpectTd = serde_json::from_str(json).unwrap();
        grade_td("lead-noise", &exp, &cap).expect("grade");
    }
}
