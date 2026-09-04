//! Stand-alone [`no_std`] CoAP library: messages plus bounded storage.
//!
//! # Modules
//!
//! - [`message`] — decode and encode a CoAP datagram (`&[u8]` / `&mut [u8]`).
//!   Usable with no [`Engine`].
//! - [`storage`] — [`Engine`] generic over [`Storage`] (acquire / release /
//!   rotate), [`Memory`], pools, and tables. Main types are also re-exported
//!   at the crate root.
//! - [`profiles`] — [`profiles::Default`] (1472-byte datagrams) and
//!   [`profiles::Constrained`] (1152).
//!
//! Architecture and the bounded-memory contract live in [`design.md`][design].
//! Protocol behavior is defined by the IETF documents in [`knowledge/rfcs/`][rfcs].
//! This crate does not restate wire format. There is no Ethernet handling: a
//! datagram slot holds CoAP message bytes (UDP payload). [`Peer`] is sidecar
//! metadata, currently a placeholder until `Endpoint` exists. OSCORE, DTLS,
//! and plugtest harnesses are out of scope here.
//!
//! # Message
//!
//! [`decode`] parses a UDP payload into [`ParsedMessage`], a borrowed view
//! (header, [`Token`], option iterator, payload). [`encode`] writes a
//! [`Message`] into a caller buffer. Round-trip is byte-stable for canonical
//! option encoding. Critical unrecognized options are a structured
//! [`ParseError::UnrecognizedCritical`] via
//! [`ParsedMessage::check_rfc7252_options`]; the library does not invent
//! 4.02 / RST policy.
//!
//! # Storage backends
//!
//! - `no_std` default: [`Memory<P>`](Memory) owns typed arrays sized by
//!   [`MemoryProfile`] named associated constants. [`profiles::Default`] uses
//!   1472-byte datagrams; [`profiles::Constrained`] uses 1152. Body pools exist
//!   only as [`Memory<P, WithBodies<P>>`] when block-wise is enabled (default
//!   body 4096 = 4 × 1024).
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

pub use error::{BuildError, EncodeError, ParseError};
pub use message::{
    Code, Header, Message, MessageId, Opt, OptionNumber, Options, ParsedMessage, Token, Type,
    decode, encode,
};
#[cfg(feature = "alloc")]
pub use storage::AllocMemory;
pub use storage::{
    BodyPool, Capacities, DatagramPool, DedupTable, Engine, EngineBuilder, Memory, MemoryProfile,
    Missing, NoBodies, ObserveTable, Peer, Present, SlotError, SlotId, SlotPool, Storage,
    WithBodies,
};
