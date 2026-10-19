# Development

Follow [AGENTS.md](AGENTS.md) for delivery. Use the pinned Rust toolchain;
the library is edition 2024 with MSRV 1.89. Test peers may require newer tools.
Merges to `main` are squash-only and require the repository's CI checks.

Pre-release development does not require version bumps, backwards compatibility,
deprecation cycles or compatibility layers. Update affected callers together when
changing interfaces. Keep manifest versions and dependency pins required by tooling;
release version selection belongs to an explicitly requested publication.

## Current-state assessment

Finish accepted coding and integration before expanding comparative benchmarks
or stability campaigns. For a performance question, use a short current-source
sample with named workload, security, storage and resource settings. Start device
comparisons from `SmallDevice` / `profiles::Constrained`, keeping route, Call,
deferred and security capacities explicit. Hold these settings constant when
comparing blockwise enabled/disabled or protected/unprotected operation. The
small preset is a starting budget, not a restriction on accepted capabilities.

Report complete-object storage separately from peak stack, heap and transport
usage, without double counting an App placed inside a reserved stack. Identify
unmeasured quantities. Retain source identities and compact evidence privately;
older results do not fill gaps in a current-state report. Use historical comparison
only when requested or necessary to diagnose a particular regression.

## Validate each batch

Complete a coherent batch of capability, optimization or cleanup work before
testing. Run an earlier focused check when it resolves an implementation uncertainty.
Choose checks from the affected boundaries and record what ran. For a Rust batch:

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

Run the complete feature and packaging matrix before an explicitly requested
release. During development, select the affected checks from this matrix; expand
coverage when a concrete cross-feature or compiler concern warrants it:

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
cargo +1.89.0 check --locked -p coaptic --no-default-features
cargo +1.89.0 check --locked -p coaptic --all-features
```

`coaptic` uses the compile-time `coaptic-io` procedural macro crate to generate
native async counterparts from shared protocol bodies. A registry release must
publish that dependency before `coaptic`; it adds no device runtime dependency.
Harnesses and peers stay out of the published crates. Inspect
packaged files and license notices, complete maintainer/external review, and agree
on the final release snapshot before publishing. Package preparation does not
authorize publication.

Release tags trigger rustdoc and crates.io publication. The tag must match the
manifest version exactly (`v0.0.10` for `0.0.10`). Configure the repository's
`CARGO_REGISTRY_TOKEN` secret with an authorized crates.io token; never commit it.
After publication, update the README dependency and manifest documentation URL
to the released crate and `https://docs.rs/coaptic`.
