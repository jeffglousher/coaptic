//! Stand-alone [`no_std`] CoAP library: messages plus bounded storage.
//!
//! Crate-root types are the happy path ([`Engine`], [`Memory`], [`Endpoint`],
//! [`Progress`], [`Access`], [`Ids`], keyed rows, Block/Q-Block types, main
//! errors). Typestate markers, raw tables/pools, and backend traits live in
//! [`storage`] / [`message`]. Repo map: `README.md`. Reviewer brief:
//! `REVIEW.md`. Architecture: [`design.md`][design]. Protocol: [`knowledge/rfcs/`][rfcs].
//! This rustdoc does not restate wire format.
//!
//! # Modules
//!
//! - [`message`] — decode/encode a CoAP datagram. No [`Engine`] required.
//! - [`storage`] — [`Engine`] generic over [`Storage`]; [`Memory`], pools, tables.
//! - [`profiles`] — [`profiles::Default`] (1472-byte datagrams) and
//!   [`profiles::Constrained`] (1152).
//!
//! # Message
//!
//! [`decode`] / [`encode`] a UDP payload. [`OptionsBuilder`] inserts options
//! in any order. [`Ids`] and [`Token::mint`] ([`message::TokenSource`]) are
//! caller-owned; the core has no OS RNG. [`Message::con`] / [`Message::non`]
//! build a CON/NON skeleton. Empty ACK/RST live in [`message`].
//! [`message::value`] covers RFC 7252 empty/opaque/uint/string. Named
//! [`Opt`] helpers cover Table 4 plus Observe and Block / Q-Block / Size2
//! ([`BlockValue`]). Optional, not used by [`decode`]:
//!
//! - [`ParsedMessage::check_rfc7252_options`] — unrecognized critical
//! - [`ParsedMessage::check_rfc7252_formats`] — known option, wrong format
//!
//! The library does not invent 4.02 / RST policy.
//!
//! # Storage and progress
//!
//! A datagram slot holds CoAP bytes (UDP payload), not Ethernet. [`Endpoint`]
//! is sidecar metadata. Not seventh areas: Dedup ([`DedupEntry`]), pending CON
//! + RTO ([`PendingCon`] / [`PendingRto`]), token matching ([`ExchangeEntry`]),
//! Observe interest ([`ObserveInterest`]), Block/Q-Block ([`BlockTransfer`]).
//! Incoming Q-Block holes surface as [`QBlockRecover`].
//!
//! [`Access`] / [`AccessMut`] pin occupied bytes against release.
//! [`Engine::progress`] is one bounded pass: CON retransmit poll, one rotating
//! unpinned RX step, one rotating Observe notify, at most one incoming
//! Q-Block recover. The caller owns clock, jitter, and send.
//!
//! - `no_std` default: [`Memory<P>`](Memory) sized by
//!   [`storage::MemoryProfile`]. Body pools exist only as
//!   `Memory<P, storage::WithBodies<P>>` when `.block_wise(true)` (default
//!   body 4096 = 4 × 1024).
//! - `alloc`: [`AllocMemory`] + runtime [`Capacities`] (heap at init, then no
//!   growth).
//!
//! [`EngineBuilder`] is consuming and typestate-gated.
//! [`.block_wise`](EngineBuilder::block_wise)`(false)` omits body pools.
//!
//! OSCORE, DTLS, BERT, and an in-crate plugtest harness are out of scope.
//!
//! # Features
//!
//! - `alloc` — enable the allocator. Off by default.
//! - `std` — enable the standard library. Implies `alloc`.
//!
//! [`no_std`]: https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-prelude
//! [design]: https://github.com/jeffglousher/coaptic/blob/main/design.md
//! [rfcs]: https://github.com/jeffglousher/coaptic/tree/main/knowledge/rfcs

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod error;
pub mod message;
pub mod storage;

pub use storage::profiles;

pub use error::{
    BlockTransferError, BuildError, EncodeError, OptionsFull, ParseError, SlotMessageError,
    ValueError,
};
pub use message::{
    BlockValue, Code, ContentFormat, EncodedUint, Header, Ids, Message, MessageId,
    OBSERVE_DEREGISTER, OBSERVE_REGISTER, OBSERVE_SEQUENCE_MASK, Opt, OptionNumber,
    OptionValueFormat, Options, OptionsBuilder, ParsedMessage, Token, Transmission, Type, decode,
    decode_block, decode_observe, decode_uint, decode_uint16, empty_ack, empty_rst, encode,
    encode_block, encode_observe, encode_uint,
};
#[cfg(feature = "alloc")]
pub use storage::AllocMemory;
pub use storage::{
    Access, AccessMut, BlockKey, BlockProgress, BlockRole, BlockTransfer, Capacities, DedupEntry,
    DedupKey, Endpoint, Engine, EngineBuilder, ExchangeEntry, ExchangeKey, Memory, ObserveInterest,
    ObserveKey, OutgoingBlock, PendingCon, PendingRto, Progress, QBlockRecover, Retransmit,
    SlotError, SlotId, Storage,
};
