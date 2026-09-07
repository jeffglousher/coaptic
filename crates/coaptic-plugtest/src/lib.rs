//! Multi-implementation CoAP#4 plugtest harness with pcap capture and grading.
//!
//! This crate is **not** part of the `coaptic` library. The library stays
//! `no_std` with zero runtime Cargo dependencies. Peers, pcap, and DTLS live
//! here as test/harness code.
//!
//! ```text
//! cargo test -p coaptic-plugtest
//! cargo test -p coaptic-plugtest --features dtls
//! cargo run -p coaptic-plugtest --bin dogfood
//! ```
//!
//! # Architecture
//!
//! - [`peer::Peer`] — start/stop a server, send a client request, poll, local
//!   UDP address. Backends: [`coaptic::CoapticPeer`] (App server + App client) and
//!   [`coap_rs::CoapRsPeer`] (`coap` / coap-rs). Add a new backend by
//!   implementing [`peer::Peer`].
//! - [`dogfood`] — timed coaptic ↔ coap-rs loops (GET/PUT/POST, Observe
//!   register, block-wise). Wall min/mean/p50/p99/max, Engine occupancy, and
//!   [`coaptic::storage::Metrics`] via [`coaptic::storage::Engine::metrics`].
//! - [`runner`] — each vendored TD × useful role pairs (coaptic server /
//!   coap-rs client, and the swap). Same-impl coaptic↔coaptic is also run
//!   for base GETs.
//! - [`pcap`] — records UDP payloads (and decrypted CoAP when DTLS unwraps).
//!   Writes PCAP (LINKTYPE_RAW IPv4/UDP). Ports, Message ID, Token, and
//!   timestamps are wild-carded by [`grade`].
//! - [`grade`] — deterministic field asserts from golden JSON
//!   (`expectations/catalog.json`).
//!
//! # DTLS
//!
//! Feature `dtls` pulls **webrtc-dtls** (the same stack coap-rs uses) as a
//! harness dependency. The `coaptic` **library** does not terminate DTLS and
//! stays zero-dep. The harness wraps UDP + webrtc-dtls as a sync
//! [`coaptic::storage::DatagramIo`] (`DtlsIo`) so `App::poll` and the App
//! client see plaintext CoAP. Mixed pairs (`coap-rs→coaptic`,
//! `coaptic→coap-rs`, `coaptic→coaptic`) run handshake + GET `/secure`.
//! There is no wire tap on the webrtc-dtls socket, so ClientHello cipher
//! lists are not graded.
//!
//! Raw-public-key TDs (`TD_COAP_DTLS_04`–`07`) run as mutually-authenticated
//! ECDSA certificates: webrtc-dtls has no RFC 7250 RPK certificate type.
//! Cipher `TLS_ECDHE_ECDSA_WITH_AES_128_CCM_8` is requested when the stack
//! offers it.
//!
//! # 6LoWPAN
//!
//! `TD_6LoWPAN_*` stay skipped (`future/backlog: 6LoWPAN (contributor opportunity)`).
//!
//! Tracking: <https://github.com/jeffglousher/coaptic/issues/56>.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod catalog;
pub mod coap_rs;
pub mod coaptic;
pub mod dogfood;
pub mod grade;
pub mod pcap;
pub mod peer;
pub mod runner;
pub mod site;

#[cfg(feature = "dtls")]
pub mod dtls;

/// Vendored golden expectations (JSON). Ports / time are wildcards; CORE asserts type and echo MID/Token.
pub const CATALOG_JSON: &str = include_str!("../expectations/catalog.json");
