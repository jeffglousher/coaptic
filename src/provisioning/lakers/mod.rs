//! Private production subset of [Lakers v0.8.0](https://github.com/lake-rs/lakers).
//!
//! Source is pinned to [462b30f](https://github.com/jeffglousher/lakers/commit/462b30f5b3e9cfab96c05fe65f36d19eaa7002ad).
//! The BSD-3-Clause notice is retained in `LICENSE-BSD`; source transformations,
//! including local connection-ID bounds hardening, and hashes are recorded in
//! `provenance.json`.
//! The bounded suite/profile extension is retained in `profile.patch`, applied
//! after the original vendoring and formatting transformations.
//! `exporter.patch` adds consuming access to the completed exporter secret;
//! the public checked exporter and its storage are owned by Coaptic.
//! `ead.patch` retains complete, validated EAD sequences for authenticated
//! processing; the bounded representation and policy hooks are Coaptic-owned.
#![allow(dead_code, deprecated, unexpected_cfgs, unused_imports, missing_docs)]
#![allow(clippy::all, clippy::pedantic)]

mod core;
mod edhoc;
mod shared;

pub use self::core::*;
