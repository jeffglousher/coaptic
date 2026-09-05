//! Stand-alone [`no_std`] CoAP library: messages plus bounded storage.
//!
//! # Modules
//!
//! - [`message`] — decode and encode a CoAP datagram (`&[u8]` / `&mut [u8]`).
//!   Usable with no [`Engine`].
//! - [`storage`] — [`Engine`] generic over [`Storage`] (acquire / release /
//!   rotate), [`Memory`], pools, and tables. Happy-path types are
//!   re-exported at the crate root; typestate markers, raw `*Table` /
//!   `*Pool` types, and backend traits live in [`storage`].
//! - [`profiles`] — [`profiles::Default`] (1472-byte datagrams) and
//!   [`profiles::Constrained`] (1152).
//!
//! Architecture and the bounded-memory contract live in [`design.md`][design].
//! Protocol behavior is defined by the IETF documents in [`knowledge/rfcs/`][rfcs].
//! This crate does not restate wire format. There is no Ethernet handling: a
//! datagram slot holds CoAP message bytes (UDP payload). [`Endpoint`] is
//! sidecar metadata (address and port) next to the slot. The Dedup Table
//! stores [`DedupEntry`] rows keyed by Message ID and remote [`Endpoint`].
//! Pending CON matching ([`PendingCon`]) is sidecar on TX datagram slots,
//! including retransmit / RTO bookkeeping ([`PendingRto`]; caller clock and
//! jitter). A different identity from Dedup. Token matching ([`ExchangeEntry`]) is a
//! compact table keyed by [`Token`] and remote [`Endpoint`], sized from the
//! TX pool count (not a seventh area). Observe interest ([`ObserveInterest`])
//! fills the existing [`storage::ObserveTable`] (Token + remote [`Endpoint`];
//! pending notify and 24-bit sequence live on the row).
//! Classic Block1 / Block2 body assembly uses a [`BlockTransfer`] sidecar on
//! Incoming / Outgoing Body Pool slots when `.block_wise(true)` (Token +
//! remote [`Endpoint`]; see `design.md`). Incoming Q-Block1 / Q-Block2 use
//! the same slots with a [`BlockTransfer::MAX_PAYLOADS`] window (RFC 9177
//! §7.2 default 10). Outgoing Q-Block1 / Q-Block2 issue up to that many
//! outstanding blocks per window on the Outgoing Body Pool. Incoming
//! Q-Block window holes surface as [`QBlockRecover`] from
//! [`Engine::progress`]. BERT and plugtest are out of scope here. OSCORE
//! and DTLS are also out of scope.
//!
//! # Message
//!
//! [`decode`] parses a UDP payload into [`ParsedMessage`], a borrowed view
//! (header, [`Token`], option iterator, payload). [`encode`] writes a
//! [`Message`] into a caller buffer. Round-trip is byte-stable for canonical
//! option encoding. [`OptionsBuilder`] inserts options in any order and
//! yields a non-decreasing slice for [`Message::with_options`].
//!
//! [`Engine`] can decode an occupied RX/TX datagram slot and encode a
//! [`Message`] into an acquired slot (`set_len` included). Temporary
//! [`Access`] / [`AccessMut`] pins an occupied RX/TX datagram or body
//! slot against release (`design.md` §Application memory access). Empty ACK/RST
//! constructors and [`ParsedMessage`] detectors live in [`message`].
//! [`Ids`] is a wrapping Message ID counter; [`Token::mint`] takes
//! caller entropy ([`message::TokenSource`]; the core does not call an OS
//! RNG). [`Message::con`] / [`Message::non`] build a CON/NON skeleton
//! without [`Engine`]. Pending
//! CON matching is sidecar on TX datagram slots ([`PendingCon`]), not a
//! seventh area and not the Dedup Table. [`Engine::poll_retransmit`] walks
//! due pending TX slots using a caller-supplied clock. [`Engine::progress`]
//! is one bounded pass of that poll, one rotating unpinned RX step, one
//! rotating Observe notify, and at most one incoming Q-Block recover
//! (`design.md` §Reference progress contract).
//! Outstanding request matching
//! ([`ExchangeEntry`]) uses Token plus remote [`Endpoint`]; empty ACK is
//! not a response for that table. Observe register/deregister fills
//! [`ObserveInterest`] rows; [`Engine::signal_observe`] marks a row pending
//! and progress surfaces at most one (sequence on the row, no body queued).
//! Classic incoming Block1 / Block2, incoming
//! Q-Block1 / Q-Block2, and outgoing Block1 / Block2 / Q-Block1 / Q-Block2
//! assemble or slice complete bodies in body-pool slots ([`BlockTransfer`])
//! when [`storage::BodySlots`] is implemented. Optional
//! format/critical checks
//! remain separate calls. The library does not invent 4.02 / RST policy.
//!
//! Option *values* stay opaque at the wire layer. [`message::value`] encodes
//! and decodes RFC 7252 empty / opaque / uint / string values (uint uses a
//! stack buffer; string decode is a `&str` view). Named [`Opt`] constructors
//! and [`ParsedMessage`] accessors cover Table 4 options plus Observe
//! (RFC 7641 option 6; not in Table 4, elective) and Block / Q-Block / Size2
//! ([`BlockValue`] NUM/M/SZX; RFC 7959 / RFC 9177; not in Table 4). Two
//! optional checks, neither used by [`decode`]:
//!
//! - [`ParsedMessage::check_rfc7252_options`] — unrecognized critical
//!   ([`ParseError::UnrecognizedCritical`])
//! - [`ParsedMessage::check_rfc7252_formats`] — known option, wrong format
//!   ([`ParseError::BadOptionFormat`])
//!
//! The library does not invent 4.02 / RST policy.
//!
//! # Storage backends
//!
//! - `no_std` default: [`Memory<P>`](Memory) owns typed arrays sized by
//!   [`storage::MemoryProfile`] named associated constants.
//!   [`profiles::Default`] uses 1472-byte datagrams;
//!   [`profiles::Constrained`] uses 1152. Body pools exist only as
//!   `Memory<P, storage::WithBodies<P>>` when block-wise is enabled
//!   (default body 4096 = 4 × 1024).
//! - `alloc`: [`AllocMemory`] plus runtime [`Capacities`] (heap at init, then no
//!   growth). Same [`Storage`] trait. Not a carved byte slab.
//!
//! # Builder
//!
//! [`EngineBuilder`] is consuming and typestate-gated. [`EngineBuilder::build`]
//! exists only after the required areas are specified (profile or
//! method-by-method) and [`.block_wise`](EngineBuilder::block_wise) is set.
//! `.block_wise(false)` requires Storage with no body pools.
//! `.block_wise(true)` requires body capacity in bytes, a multiple of 1024.
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
