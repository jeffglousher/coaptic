//! Stand-alone [`no_std`] CoAP library: an approachable [`App`] face over
//! messages and bounded storage.
//!
//! Crate-root types are the happy path: [`App`], [`Request`], [`Response`],
//! [`Call`], [`Outgoing`], [`get`] / [`put`] / [`post`] / [`delete`] /
//! [`fetch`] / [`patch`] / [`ipatch`], [`Endpoint`], [`profiles`],
//! [`Code`], [`ContentFormat`], [`Method`], [`ProblemDetails`], and the
//! errors from [`bind`](app::AppBuilder::bind) / [`App::poll`] / [`Outgoing::send`].
//! Engine slots, tables, Block/Q-Block types, [`Ids`](message::Ids), and
//! [`DatagramIo`](storage::DatagramIo) live in [`storage`] / [`message`].
//! [`app`] is the routing façade (`Request` to [`Response`]) and the
//! outbound client face ([`Outgoing`] → [`Response`]). Engine slots are
//! the advanced path: per-slot state machines and
//! [`Progress`](storage::Progress). The caller owns the socket
//! ([`storage::DatagramIo`]), the clock (`now_ms`), the destination of an
//! outbound request, and domain state that outlives a request — there is
//! no global App State. Protocol copies: [`knowledge/rfcs/`][rfcs].
//! Architecture planning: [GitHub project][plan]. This rustdoc does not
//! restate wire format.
//!
//! # Modules
//!
//! - [`app`] — [`App`] + [`Site`](app::Site): [`route`](app::AppBuilder::route),
//!   [`get`] / [`put`] method routers, `fn(Request<'_>) -> Response` handlers,
//!   [`Response`] builders, [`App::poll`], [`App::notify`](app::App::notify),
//!   outbound [`App::get`] / [`Outgoing::send`] / [`App::take_response`].
//!   No global mutable shared bag.
//! - [`message`] — decode/encode a CoAP datagram. No
//!   [`Engine`](storage::Engine) required.
//! - [`storage`] — [`Engine`](storage::Engine) generic over
//!   [`Storage`](storage::Storage); [`Memory`](storage::Memory), pools,
//!   tables, [`DatagramIo`](storage::DatagramIo).
//! - [`profiles`] — [`profiles::Default`] (1472-byte datagrams) and
//!   [`profiles::Constrained`] (1152).
//!
//! # Message
//!
//! [`decode`](message::decode) / [`encode`](message::encode) a UDP payload.
//! [`OptionsBuilder`](message::OptionsBuilder) inserts options in any
//! order. [`Ids`](message::Ids) and [`Token::mint`](message::Token::mint)
//! ([`message::TokenSource`]) are caller-owned; the core has no OS RNG.
//! [`Message::con`](message::Message::con) / [`Message::non`](message::Message::non)
//! build a CON/NON skeleton. Empty ACK/RST live in [`message`].
//! [`message::value`] covers RFC 7252 empty/opaque/uint/string. Named
//! [`Opt`](message::Opt) helpers cover Table 4 plus Observe, Block /
//! Q-Block / Size2 ([`BlockValue`](message::BlockValue)), Request-Tag,
//! Echo ([`Echo`](message::Echo)), Hop-Limit ([`HopLimit`](message::HopLimit)),
//! and No-Response ([`NoResponse`](message::NoResponse)).
//! [`ParsedMessage::precondition`](message::ParsedMessage::precondition)
//! classifies If-Match / If-None-Match.
//! [`ObserveTransmission`](message::ObserveTransmission) names RFC 7641
//! §4.5 constants. [`Code::FETCH`] / [`Code::PATCH`] / [`Code::IPATCH`]
//! and named 2.31 / 4.08 / 4.09 / 4.22 / 5.08 are codes only — the
//! library does not invent when to send them. Optional, not used by
//! [`decode`](message::decode):
//!
//! - [`ParsedMessage::check_rfc7252_options`](message::ParsedMessage::check_rfc7252_options)
//!   — unrecognized critical
//! - [`ParsedMessage::check_rfc7252_formats`](message::ParsedMessage::check_rfc7252_formats)
//!   — known option, wrong format
//!
//! [`ProblemDetails`] encodes RFC 9290 concise problem details (CBOR;
//! Content-Format 257). [`Response::problem`] is the App-facing builder.
//! App-generated 4.04 / 4.05 / 4.08 use it. Echo 4.01 and other
//! Engine-path 4.xx stay caller-opt-in.
//!
//! The library does not invent 4.02 / RST policy.
//!
//! # Storage and progress
//!
//! A datagram slot holds CoAP bytes (UDP payload), not Ethernet.
//! [`Endpoint`] is sidecar metadata. Not seventh areas: Dedup
//! ([`DedupEntry`](storage::DedupEntry)), pending CON and RTO
//! ([`PendingCon`](storage::PendingCon) / [`PendingRto`](storage::PendingRto)),
//! token matching ([`ExchangeEntry`](storage::ExchangeEntry)), Observe
//! interest ([`ObserveInterest`](storage::ObserveInterest) /
//! [`ObserveLifetime`](storage::ObserveLifetime) /
//! [`ObserveNotifyHold`](storage::ObserveNotifyHold)), Block/Q-Block
//! ([`BlockTransfer`](storage::BlockTransfer)), Echo freshness
//! ([`Echo`](message::Echo) on [`ExchangeEntry`](storage::ExchangeEntry)).
//! Request-Tag / ETag body identity is [`BodyTag`](storage::BodyTag) on
//! [`BlockKey`](storage::BlockKey). BERT is SZX 7 on
//! [`BlockValue`](message::BlockValue). Incoming Q-Block holes surface as
//! [`QBlockRecover`](storage::QBlockRecover).
//!
//! [`Access`](storage::Access) / [`AccessMut`](storage::AccessMut) pin
//! occupied bytes against release. [`DatagramIo`](storage::DatagramIo)
//! binds any caller transport into RX/TX slots
//! ([`Engine::recv_from`](storage::Engine::recv_from) /
//! [`Engine::send_tx`](storage::Engine::send_tx)). The core still does
//! not own a socket. [`bind`](app::AppBuilder::bind) hides construction;
//! implement [`storage::DatagramIo`] on the socket type.
//! [`Engine::progress`](storage::Engine::progress) is one bounded pass:
//! CON retransmit poll, one rotating unpinned RX step, one rotating
//! Observe notify (skips an endpoint at notification NSTART), at most
//! one Observe lifetime expiry, at most one incoming Q-Block recover.
//! The caller owns clock, jitter, and send. RFC 7641 §4.5 24-hour
//! NON-confirm is [`ObserveInterest::must_confirm`](storage::ObserveInterest::must_confirm).
//!
//! - `no_std` default: [`Memory<P>`](storage::Memory) sized by
//!   [`storage::MemoryProfile`]. Body pools exist only as
//!   `Memory<P, storage::WithBodies<P>>` when `.block_wise(true)` (default
//!   body 4096 = 4 × 1024).
//! - `alloc`: [`AllocMemory`](storage::AllocMemory) + runtime
//!   [`Capacities`](storage::Capacities) (heap at init, then no growth).
//!
//! [`EngineBuilder`](storage::EngineBuilder) is consuming and
//! typestate-gated.
//! [`.block_wise`](storage::EngineBuilder::block_wise)`(false)` omits
//! body pools.
//!
//! # App
//!
//! [`App`] is the approachable loop: [`App::profile`],
//! [`block_wise`](app::AppBuilder::block_wise),
//! [`route`](app::AppBuilder::route), [`bind`](app::AppBuilder::bind),
//! then [`App::poll`]. Handlers are `fn(Request<'_>) -> Response`:
//! borrowed request fields (`payload()`, path, token, options, `body()`
//! when Block1 / Q-Block1 assembled) and an owned [`Response`]
//! (`content` / `content_format` / [`Response::problem`] for RFC 9290
//! CBOR). Large responses use the TX body area (Block2, or Q-Block2
//! when the request asked for it) without exposing
//! [`SlotId`](storage::SlotId). Observe register / deregister,
//! [`App::notify`](app::App::notify), and poll-time notify via
//! [`ObserveSource`](app::ObserveSource) use the Engine Observe table
//! (no `SlotId` on the happy path). Progress-driven Q-Block2 recover
//! and incoming Q-Block1 assembly run inside `poll`. The reactor owns
//! per-slot state machines inside `poll`. Domain data that outlives a
//! request stays outside `App`. Outbound: [`App::get`] / [`App::put`] →
//! [`Outgoing::to`] → [`Outgoing::send`]; `poll` matches Token + peer
//! on the Exchange table; [`App::take_response`] is a [`Response`]
//! (code / payload, and [`Response::body`] when Block2 / Q-Block2
//! assembled). No `SlotId`. The caller owns the destination and must
//! take responses. Tokens and Message IDs are App counters (no OS RNG).
//! Observe client stays on the Engine path. Engine remains the
//! advanced escape hatch for explicit slots, [`Access`](storage::Access),
//! custom RST / 4.xx, and BERT edges (`app.engine_mut()`). See
//! `examples/coap_server.rs` (`std`).
//!
//! Path arguments accept both `&["sensors", "temp"]` and `"sensors/temp"`
//! ([`app::IntoPath`]; empty segments are rejected).
//!
//! OSCORE and DTLS are out of scope. 6LoWPAN is not planned.
//!
//! # Validation harness
//!
//! Integration tests (not compiled into this `no_std` crate) live under
//! `tests/`. They may use `std`. No extra Cargo dependencies.
//!
//! | Command | What it runs |
//! | --- | --- |
//! | `cargo test --test block_sweep` | Combinatorial SZX `{16…1024}` × body length in blocks `1…25` for classic Block1/Block2 and Q-Block windowed paths. Uses a large test profile (32 × 1024 body bytes). Default 4096 is not the ceiling. Tracking: [issue #49][plugtest]. |
//! | `cargo test --test block_sweep --all-features` | Same sweep plus `AllocMemory` 25 × 1024. |
//! | `cargo test --test plugtest` | In-scope CoAP#4 TDs from `tests/plugtest/td-coap4/{base,block,link}.yml` on a two-Engine loopback (datagram bytes only). `dtls` is skipped (deferred). `6lowpan` is skipped (not planned). |
//! | `cargo test --test plugtest catalog` | Hand-maintained TD lists match vendored YAML keys. |
//! | `cargo test --test plugtest td_coap_core` | All 24 `TD_COAP_CORE_*` from `base.yml`. |
//! | `cargo test --test plugtest td_coap_block` | All 6 `TD_COAP_BLOCK_*` from `block.yml`. |
//! | `cargo test --test plugtest td_coap_obs` | All in-scope `TD_COAP_OBS_*` from `block.yml` (no `TD_COAP_OBS_03`). |
//! | `cargo test --test plugtest td_coap_link` | All 9 `TD_COAP_LINK_*` from `link.yml`. |
//! | `cargo test --test plugtest inventory -- --nocapture` | Print RUN vs SKIP for every vendored TD id. |
//!
//! TD identifiers are extracted from those YAML files. This crate does not
//! invent TD numbers. Tracking: [issue #49][plugtest].
//!
//! # Features
//!
//! - `alloc` — enable the allocator. Off by default.
//! - `std` — enable the standard library. Implies `alloc`.
//!
//! [`no_std`]: https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-prelude
//! [rfcs]: https://github.com/jeffglousher/coaptic/tree/main/knowledge/rfcs
//! [plan]: https://github.com/users/jeffglousher/projects/2
//! [plugtest]: https://github.com/jeffglousher/coaptic/issues/49

#![no_std]
#![deny(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod app;
pub mod error;
pub mod message;
pub mod storage;

pub use storage::profiles;

pub use app::{
    App, Call, Error, Method, Outgoing, Request, Response, delete, fetch, get, ipatch, patch, post,
    put,
};
pub use error::BuildError;
pub use message::{Code, ContentFormat, ProblemDetails};
pub use storage::Endpoint;
