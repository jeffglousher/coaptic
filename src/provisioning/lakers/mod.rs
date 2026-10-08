//! Private production subset of [Lakers v0.8.0](https://github.com/lake-rs/lakers).
//!
//! Source is pinned to [462b30f](https://github.com/jeffglousher/lakers/commit/462b30f5b3e9cfab96c05fe65f36d19eaa7002ad).
//! The BSD-3-Clause notice is retained in `LICENSE-BSD`; mechanical source
//! transformations and hashes are recorded in `provenance.json`.
#![allow(dead_code, deprecated, unexpected_cfgs, unused_imports, missing_docs)]
#![allow(clippy::all, clippy::pedantic)]

mod core;
mod edhoc;
mod shared;

pub use self::core::*;
