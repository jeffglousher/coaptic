//! CoAP from microcontrollers to cloud services.
//!
//! Rust, [`no_std`], and no heap allocation by default; optional `alloc` and `std`.
//! [`App`] serves resources and exchanges messages through one client/server API.
//! OSCORE protection is required by default; EDHOC provisioning is optional.
//!
//! # Platform integration
//!
//! The core is independent of a microcontroller, board SDK or operating system.
//! Constrained devices use fixed storage without allocation or the standard
//! library. Applications supply [`storage::DatagramIo`], secure randomness,
//! provisioned credentials and a monotonic clock, and drive [`App::poll`].
//! The same service code can use a device adapter or a host transport.
//! Our physical constrained-device qualification uses an ESP32-S3; other targets
//! need their own integration and qualification evidence.
//!
//! # Client and server
//!
//! Apps require a provisioned pairwise OSCORE context before binding. Supply
//! authenticated credentials from your provisioning system as
//! `provisioned_credentials` below; the hidden doctest credentials are fixtures.
//! Persist sender-sequence reservations before reuse of a key after restart.
//! Explicit `AppBuilder::allow_unprotected` is available for unprotected tests
//! and compatibility; disabling Cargo defaults never silently permits unprotected traffic.
//!
//! ```
//! # #[cfg(feature = "oscore")]
//! # {
//! use coaptic::{App, Request, Response, get, profiles};
//! # use coaptic::storage::DatagramIo;
//! # use coaptic::Endpoint;
//! # struct NullIo;
//! # impl coaptic::storage::DatagramTransport for NullIo {
//! #     type Error = &'static str;
//! # }
//! # impl DatagramIo for NullIo {
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
//! # let provisioned_credentials = coaptic::oscore::DeriveParams {
//! #     master_secret: &[0x42; 16], master_salt: &[], sender_id: &[1],
//! #     recipient_id: &[2], id_context: &[],
//! # };
//! let context = coaptic::oscore::SecurityContext::derive(provisioned_credentials).unwrap();
//! let mut app = App::profile::<profiles::Default>()
//!     .randomness(|bytes| getrandom::fill(bytes).is_ok())
//!     .block_wise::<true>()
//!     .route("sensors/temp", get(get_temp))
//!     .oscore(context)
//!     .bind(NullIo)
//!     .unwrap();
//! app.poll(0).unwrap();
//!
//! let peer = Endpoint::v4([192, 0, 2, 2], 5683);
//! let call = app.get("sensors/temp").to(peer).send(0).unwrap();
//! app.poll(0).unwrap();
//! let _response = app.take_response(call);
//! # }
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
//! # One interface, adjustable resources
//!
//! [`App::with_resources`] selects one configuration from [`resources`], then
//! binds routes, security and platform I/O. Presets support fixed storage or,
//! with `alloc`, bounded heap storage with an explicit byte limit. Customize
//! resource bounds before constructing the App; the configured builder has no
//! resource overrides. [`memory_profile!`] defines custom fixed pool sizes.
//! `std` and `alloc` remain independent. Existing [`App::builder`] and
//! [`App::profile`] construction is retained for compatibility.
//!
//! The configured path retains complete responses by default. Collect them
//! with [`App::take_response_into`] and caller-owned output. Short buffers refuse
//! without consuming or partially copying the reply. Observe shares the client
//! Call budget. The single OSCORE context defaults to four live request bindings;
//! [`resources::Resources::security_requests`] configures that independent bound.
//! Larger storage is not a security-association or throughput guarantee.
//!
//! Stateful or durable handlers use [`App::poll_with`], [`Response::deferred`],
//! [`DeferredReply`] and [`App::complete`]. The empty ACK is only reception.
//! Domain work and durable state stay outside the App; bounded request metadata,
//! separate-response retransmission and Block2 snapshots stay inside it.
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
//! [`platform::CheckedClock`] wraps a fallible, stateful platform timer and rejects
//! backward readings. Use one clock epoch for all timed App operations and
//! propagate clock errors before sending or polling.
//!
//! # Scope
//!
//! Protocol support and qualification are tracked separately in the
//! repository issues. Passing individual scenarios does not establish full RFC
//! conformance. Engine BERT (SZX 7) codecs exist; App does not expose them.
//! Group OSCORE, other ciphers, Outer Block-wise over OSCORE (proxy
//! hop-by-hop), and first-party DTLS are not supported.
//! Inner Block-wise (Block1/Block2:
//! fragment then protect) and Dual-class Max-Age / No-Response / ETag
//! placement are on the App path. Network adapters such as 6LoWPAN and LoRaWAN
//! are not supplied by this library.
//!
//! # Features
//!
//! - `alloc` — enable the allocator. Off by default.
//! - `std` — enable OS networking and standard error integration; independent of `alloc`.
//! - `oscore` — pairwise OSCORE (RFC 8613). Pulls RustCrypto `aes` /
//!   `ccm` / `hkdf` / `sha2`. Enabled by default without `std` or an allocator.
//!   `--no-default-features` retains an Engine without runtime dependencies and requires
//!   explicit unprotected opt-out for App construction without cryptography.
//! - `edhoc` — optional pinned P-256 EDHOC method 3 / suites 2 and 3 provisioning.
//!   Implies `oscore`, stays `no_std` and uses fixed storage without an allocator.
//!   Install trusted credentials and supply fresh entropy through the caller.
//!
//! [`no_std`]: https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-prelude

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
pub mod link_format;
pub mod message;
#[cfg(feature = "oscore")]
pub mod oscore;
pub mod platform;
#[cfg(feature = "edhoc")]
pub mod provisioning;
pub mod storage;
pub mod uri;

pub use app::resources;
pub use storage::profiles;

pub use app::{
    App, AppUsage, Call, CallFailure, DeferredReply, EchoCheck, EchoDecision, EchoPolicy, Error,
    Method, Outgoing, Request, Response, ResponseBufferError, ResponseError, delete, fetch, get,
    ipatch, patch, post, put,
};
pub use error::BuildError;
pub use message::{Code, ContentFormat, ProblemDetails};
pub use storage::{Endpoint, Metrics};
