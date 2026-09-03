# Agent instructions

Read `design.md` before changing architecture or memory types. Do not rewrite `design.md` or the RFCs unless the task says to.

- Protocol behavior stays in `rfcs/`. Do not restate wire format, timers, or option semantics in docs or comments.
- Default is `no_std` with no allocator. `alloc` and `std` are optional (`std` implies `alloc`).
- Own the slot and table types in this crate. Do not add `heapless` as the architecture.
- Keep pull requests small. One concern per PR.
- Run `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and `cargo doc --no-deps --all-features` before you finish.
