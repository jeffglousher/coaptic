//! Stand-alone [`no_std`] CoAP library: an approachable [`App`] face over
//! messages and bounded storage.
//!
//! Crate-root types are the happy path: [`App`], [`Request`], [`Response`],
//! [`Call`], [`Outgoing`], [`get`] / [`put`] / [`post`] / [`delete`] /
//! [`fetch`] / [`patch`] / [`ipatch`], [`Endpoint`], [`profiles`],
//! [`Code`], [`ContentFormat`], [`Method`], [`ProblemDetails`], and the
//! errors from [`bind`](app::AppBuilder::bind) / [`App::poll`] /
//! [`Outgoing::send`]. Engine slots, tables, Block/Q-Block types,
//! [`Ids`](message::Ids), and [`DatagramIo`](storage::DatagramIo) live in
//! [`storage`] / [`message`].
//!
//! # Happy path
//!
//! ```text
//! RX slot (+ body) --view--> Request
//! handler(Request) -> Response
//! Response --encode--> TX slot (+ body)
//!
//! Outgoing (get/put) --encode--> TX slot
//! poll matches Token + endpoint
//! RX --copy--> Response::payload  (non-Block: min(len, INLINE_PAYLOAD) = 128)
//! ```
//!
//! ```
//! use coaptic::{App, Request, Response, get, profiles};
//! # use coaptic::storage::DatagramIo;
//! # use coaptic::Endpoint;
//! # struct NullIo;
//! # impl DatagramIo for NullIo {
//! #     type Error = &'static str;
//! #     fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
//! #         Ok(None)
//! #     }
//! #     fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> { Ok(bytes.len()) }
//! # }
//!
//! fn get_temp(_req: Request<'_>) -> Response<'static> {
//!     Response::content(b"21.5")
//! }
//!
//! let mut app = App::profile::<profiles::Default>()
//!     .randomness(|bytes| getrandom::fill(bytes).is_ok())
//!     .block_wise::<true>()
//!     .route("sensors/temp", get(get_temp))
//!     .bind(NullIo)
//!     .unwrap();
//! app.poll(0).unwrap();
//!
//! let peer = Endpoint::v4([192, 0, 2, 2], 5683);
//! let call = app.get("sensors/temp").to(peer).send(0).unwrap();
//! app.poll(0).unwrap();
//! let _response = app.take_response(call);
//! ```
//!
//! Path arguments accept both `"sensors/temp"` and `&["sensors", "temp"]`
//! ([`app::IntoPath`]; empty segments are rejected). Handlers are
//! `fn(Request<'_>) -> Response`. One [`Response`] type covers handler
//! intent and a completed client exchange ([`App::take_response`]).
//! Non-Block [`Response::payload`] copies at most
//! [`app::INLINE_PAYLOAD`] (128) bytes; a longer piggybacked 2.05 with
//! `.block_wise::<false>()` is truncated —
//! [`Response::payload_truncated`] is `true`. Assembled Block2 / Q-Block2
//! is [`Response::body`]. Observe subscribe is [`Outgoing::observe`]; the
//! initial representation and later notifications use the same [`Call`].
//! [`Outgoing::deregister`] sends Observe=1. Structured 4.xx bodies use
//! [`Response::problem`]
//! (RFC 9290). Q-Block1 holes from `poll` use [`Response::missing_blocks`]
//! (RFC 9177, Content-Format 272). Caller-owned Echo issuance and verification use
//! [`app::AppBuilder::echo_policy`].
//!
//! # What you own
//!
//! The socket ([`storage::DatagramIo`]), the clock (`now_ms` into
//! [`App::poll`]), the destination of an outbound request, any domain
//! state that outlives a request, and — when the `oscore` feature is on —
//! the OSCORE `SecurityContext` (Master Secret, Sender/Recipient IDs,
//! replay window; feature `oscore`). There is no global App State. Supply
//! cryptographically secure entropy through [`app::AppBuilder::randomness`]
//! for Tokens, the initial Message ID and retransmission jitter. The library
//! itself does not call an OS RNG.
//!
//! # Modules
//!
//! - [`app`] — [`App`]: [`route`](app::AppBuilder::route), method routers,
//!   `fn(Request<'_>) -> Response`, [`App::poll`], [`App::notify`],
//!   outbound [`App::get`] / [`Outgoing::send`] / [`App::take_response`].
//! - [`message`] — [`decode`](message::decode) / [`encode`](message::encode)
//!   a datagram. No [`Engine`](storage::Engine) required.
//! - [`storage`] — [`Engine`](storage::Engine) over
//!   [`Storage`](storage::Storage); pools, tables,
//!   [`DatagramIo`](storage::DatagramIo), wrapping
//!   [`Metrics`] (`Engine::metrics` /
//!   [`App::metrics`](App::metrics)). Advanced:
//!   [`Access`](storage::Access) / [`AccessMut`](storage::AccessMut).
//! - `oscore` — pairwise OSCORE (feature `oscore`): caller-owned
//!   `SecurityContext`, `App::set_oscore`.
//! - [`profiles`] — [`profiles::Default`] (1472-byte datagrams) and
//!   [`profiles::Constrained`] (1152).
//!
//! Protocol copies: [`knowledge/rfcs/`][rfcs]. Architecture planning:
//! [GitHub project][plan]. This rustdoc does not restate wire format.
//! Integration tests live under `tests/` and `crates/coaptic-plugtest`.
//! Timed mixed-stack dogfood + Observe notify collect:
//! `cargo run -p coaptic-plugtest --bin dogfood`. CI compares `--iterations 2`
//! against `crates/coaptic-plugtest/baselines/` (`--compare`; Metrics floors
//! fail, including mixed-pair Block assemble sums; wall timings print as
//! delta; `progress` is informational).
//!
//! # Future / backlog
//!
//! Protocol support and qualification are tracked separately in the
//! repository issues. Passing individual scenarios does not establish full RFC
//! conformance. Engine BERT (SZX 7) codecs exist; App does not expose them.
//! Pairwise OSCORE (RFC 8613) is the `oscore` feature: a caller-owned
//! `SecurityContext` (Master Secret, Sender/Recipient IDs, replay
//! window). `App::set_oscore` attaches it; Engine does not store keys.
//! Group OSCORE, other ciphers, Outer Block-wise over OSCORE (proxy
//! hop-by-hop), and first-party DTLS remain backlog — the
//! `coaptic-plugtest` harness (feature `dtls`) wraps webrtc-dtls as a
//! [`DatagramIo`](storage::DatagramIo). Inner Block-wise (Block1/Block2:
//! fragment then protect) and Dual-class Max-Age / No-Response / ETag
//! placement are on the App path. Alternative networks (6LoWPAN,
//! LoRaWAN, …) are the same future / backlog.
//!
//! # Qualification
//!
//! The repository retains compiler, command and result artifacts for finite
//! campaigns. `tools/qualification/seeded.py` includes compound lifecycle soak
//! and caller-owned replay-storage barriers across process termination.
//! `tools/qualification/coverage.py --branches` records LLVM branch counters
//! and execution of the named contracts in `contracts.json`. That contract
//! list covers selected App and OSCORE behavior; it is not a complete RFC audit.
//! `tools/qualification/fuzz.py` runs coverage-guided datagram, CBOR and OSCORE
//! campaigns with address sanitization. Test counts and percentages establish
//! neither exhaustive inputs nor protocol completeness.
//!
//! `tools/qualification/esp32.py --chip esp32c3 --output REPORT.json` builds core
//! and OSCORE firmware images with distinct run identities. Use `--chip esp32c6`
//! for C6; the HAL features and Rust targets differ. An ISA description does not
//! identify the chip. `uv run --python 3.13 --with esptool==5.4.0 python
//! tools/qualification/esp32_detect.py --output PORTS.json` inventories ports.
//! Add `--port COM7` to query that device's ROM and reset it afterward without
//! writing flash. Add `--firmware-report REPORT.json` to verify chip selection
//! and image hashes. Unprepared chip families refuse image selection.
//!
//! Flash each selected ELF, retain console output, then run the build-report
//! command with `--record --capture-core
//! CORE.log --capture-oscore OSCORE.log`. A build report has no device runtime
//! pass until both captures match their firmware identity and contain valid
//! stack measurements. These probes use loopback traffic; radio, platform
//! entropy, allocators and flash power-loss recovery need separate evidence.
//! Firmware paths in new reports are relative, so a downloaded artifact can be
//! moved as a directory before selecting images or recording captures.
//!
//! `uv run --python 3.13 --with esphome==2026.9.1 python
//! tools/qualification/esphome_probe.py --chip esp32c3 --output REPORT.json`
//! validates and generates an ESPHome external-component configuration. Add
//! `--compile` to link complete ESPHome/ESP-IDF firmware against the Rust probe;
//! code generation alone leaves `build_passed` false. C6 uses `--chip esp32c6`.
//! The component owns one 96 KiB FreeRTOS task with a 30-second work deadline,
//! deletes it on the ESPHome loop after completion or timeout, and records used
//! stack bytes from ESP-IDF's minimum free-stack measurement. Recording uses the
//! same
//! `--record --capture-core ... --capture-oscore ...` options and additionally
//! binds the result to the ESPHome runtime. Hardware execution remains pending
//! until both configurations produce matching captures.
//!
//! This component qualifies Coaptic inside ESPHome; the networked CoAP driver
//! and Taldra gateway are tracked in [#333]. No Taldra streaming or durability
//! guarantee is inferred from a CoAP acknowledgement or a loopback result.
//!
//! [#333]: https://github.com/jeffglousher/coaptic/issues/333
//!
//! # Features
//!
//! - `alloc` — enable the allocator. Off by default.
//! - `std` — enable the standard library. Implies `alloc`.
//! - `oscore` — pairwise OSCORE (RFC 8613). Pulls RustCrypto `aes` /
//!   `ccm` / `hkdf` / `sha2`. Off by default so the crate stays zero-dep.
//!
//! [`no_std`]: https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-prelude
//! [rfcs]: https://github.com/jeffglousher/coaptic/tree/main/knowledge/rfcs
//! [plan]: https://github.com/users/jeffglousher/projects/2

#![no_std]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod app;
pub mod error;
pub mod message;
#[cfg(feature = "oscore")]
pub mod oscore;
pub mod storage;

pub use storage::profiles;

pub use app::{
    App, Call, CallFailure, EchoCheck, EchoDecision, EchoPolicy, Error, Method, Outgoing, Request,
    Response, ResponseError, delete, fetch, get, ipatch, patch, post, put,
};
pub use error::BuildError;
pub use message::{Code, ContentFormat, ProblemDetails};
pub use storage::{Endpoint, Metrics};
