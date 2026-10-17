# Development

Follow [AGENTS.md](AGENTS.md) for delivery. Use the pinned Rust toolchain;
the library is edition 2024 with MSRV 1.85. Test peers may require newer tools.
Merges to `main` are squash-only and require the repository's CI checks.

## Validate the change

Choose checks from the affected boundary and record what ran. For Rust changes:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Also check `--no-default-features` when code moves across modules or feature gates.
For allocation/security changes, add the affected `alloc`, `oscore`, `alloc,oscore`
and `std` configurations from the release matrix below. For rustdoc changes, run
`cargo doc --no-deps --all-features`; executable examples are checked by doctests.
Documentation-only edits need link/source review, not an unrelated protocol campaign.

Use focused loopback or ESP32 proof when transport, persistence or hardware behavior
changes. Reuse existing tests and source-matched evidence. After a meaningful check
passes, continue delivery; repeat testing for a concrete unresolved concern.

## Companion validation

[coaptic-validation](https://github.com/jeffglousher/coaptic-validation) owns
independent peers, hardware tooling and benchmarks. This repository owns the Rust
library, examples and unit/integration tests.

CI calls a pinned companion workflow against the exact library revision. Follow
its [run instructions](https://github.com/jeffglousher/coaptic-validation#run) for
transport, security or harness changes. Retain both revisions, lockfiles, setup
limits and reports. Review suite-pin changes. Short process timings are preliminary
samples; a library-to-itself exchange does not establish independent interoperability.

## Release validation

Run the complete feature and packaging matrix before release, and when a change
alters the corresponding cross-feature or compiler compatibility contract:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
cargo test --all-features
cargo test --no-default-features --features alloc
cargo test --no-default-features --features oscore
cargo test --no-default-features --features alloc,oscore
cargo test --no-default-features --features std
cargo test --features std --test std_fixed_storage
cargo doc --no-deps --all-features
cargo package -p coaptic --locked
cargo +1.85.0 check --locked -p coaptic --no-default-features
cargo +1.85.0 check --locked -p coaptic --all-features
```

Only `coaptic` is published. Harnesses and peers stay out of the crate. Inspect
packaged files and license notices, complete maintainer/external review, and agree
on the final release snapshot before publishing. Package preparation does not
authorize publication.

Release tags trigger rustdoc and crates.io publication. The tag must match the
manifest version exactly (`v0.0.10` for `0.0.10`). Configure the repository's
`CARGO_REGISTRY_TOKEN` secret with an authorized crates.io token; never commit it.
After publication, update the README dependency and manifest documentation URL
to the released crate and `https://docs.rs/coaptic`.
