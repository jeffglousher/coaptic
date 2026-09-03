//! Stand-alone [`no_std`] CoAP engine.
//!
//! Architecture and the bounded-memory contract live in [`design.md`][design].
//! Protocol behavior is defined by the IETF documents in [`rfcs/`][rfcs]. This
//! crate does not restate wire format.
//!
//! # Features
//!
//! - `alloc` — enable the allocator. Off by default.
//! - `std` — enable the standard library. Implies `alloc`.
//!
//! [`no_std`]: https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-prelude
//! [design]: https://github.com/jeffglousher/coaptic/blob/main/design.md
//! [rfcs]: https://github.com/jeffglousher/coaptic/tree/main/rfcs

#![no_std]

mod placeholder {
    /// Scaffold marker so rustdoc has a page. The engine is not implemented yet.
    pub struct Engine;
}

pub use placeholder::Engine;

#[cfg(test)]
mod tests {
    use super::Engine;

    #[test]
    fn placeholder_engine_exists() {
        let _engine = Engine;
    }
}
