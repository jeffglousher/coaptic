//! Stand-alone [`no_std`] CoAP engine.
//!
//! Architecture and the bounded-memory contract live in [`design.md`][design].
//! Protocol behavior is defined by the IETF documents in [`knowledge/rfcs/`][rfcs].
//! This crate does not restate wire format.
//!
//! This release implements the **storage layer** only: [`Engine`] is generic over
//! [`Storage`] (acquire / release / rotate). There is no CoAP parser and no
//! Ethernet handling. A datagram slot holds CoAP message bytes (UDP payload);
//! [`Peer`] is sidecar metadata, currently a placeholder until `Endpoint` exists.
//!
//! # Backends
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

#[cfg(feature = "alloc")]
mod alloc_memory;
mod builder;
mod capacities;
mod engine;
mod error;
mod memory;
mod occupancy;
mod pool;
pub mod profiles;
mod slot;
mod storage;
mod table;

#[cfg(feature = "alloc")]
pub use alloc_memory::AllocMemory;
pub use builder::{EngineBuilder, Missing, Present};
pub use capacities::Capacities;
pub use engine::Engine;
pub use error::BuildError;
pub use memory::{Memory, MemoryProfile, NoBodies, WithBodies};
pub use pool::{BodyPool, DatagramPool};
pub use slot::{Peer, SlotError, SlotId};
pub use storage::{SlotPool, Storage};
pub use table::{DedupTable, ObserveTable};

#[cfg(test)]
mod tests;
