# Development

Follow [AGENTS.md](AGENTS.md) for delivery. Use the pinned Rust toolchain;
the library is edition 2024 with MSRV 1.89. Test peers may require newer tools.
Merges to `main` are squash-only. During pre-release, focused local validation
supports authorized merges; hosted CI is advisory and does not block delivery.

Pre-release development does not require version bumps, backwards compatibility,
deprecation cycles or compatibility layers. Update affected callers together when
changing interfaces. Keep manifest versions and dependency pins required by tooling;
release version selection belongs to an explicitly requested publication.

## Current-state assessment

Finish accepted coding/integration before expanding benchmarks or stability
campaigns. Requested performance runs identify source, workload, security,
backend and capacities. Start device comparisons from `SmallDevice` /
`profiles::Constrained`, naming route, Call, deferred and security settings.
Hold capabilities constant when comparing blockwise or protected/unprotected
operation; a preset is a starting budget, not a feature restriction.

Report whole-object storage separately from stack, heap and transport views.
An App inside a reserved stack is not additional RAM. Name missing metrics and
retain compact source-bound evidence; older results do not become current
measurements. Historical comparisons need a request or a concrete regression.

## Validate each batch

Complete a coherent batch, then choose checks that can detect faults in its
affected boundaries. Check earlier when it resolves an implementation uncertainty.
Format/lint affected Rust targets and run focused behavior tests; use the
[release matrix](#release-validation) for broader protocol/resource/integration changes.

Check `--no-default-features` when modules or feature gates move. Allocation/security
changes need the affected `alloc`, `oscore`, `alloc,oscore` and `std` configurations.
Rustdoc changes use `cargo doc --no-deps --all-features` and relevant doctests.
Documentation-only edits need link/source review, not a protocol campaign.

Use loopback or ESP32 proof when transport, persistence or hardware behavior
changes. Reuse source-matched tests/evidence; once a meaningful check passes,
continue delivery unless a concrete concern warrants another run.

## Hosted CI

Pull requests run one advisory development job: formatting, Clippy and library
tests with all features and without defaults. A queued hosted run is not a merge
prerequisite when focused local checks support the change.

Run the CI workflow manually with `full_validation` enabled for the full feature,
MSRV, documentation, packaging and companion matrix. Enrollment provider checks
are manual. The additional CodeQL candidate workflow runs weekly or manually;
GitHub's separate default CodeQL setup remains independent.

## Companion validation

[coaptic-validation](https://github.com/jeffglousher/coaptic-validation) owns
independent peers, hardware tooling and benchmarks. This repository owns the Rust
library, examples and unit/integration tests.

Full CI calls a pinned companion workflow against the exact library revision. Follow
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
cargo package --workspace --locked
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
