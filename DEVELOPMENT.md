# Development

Use the pinned Rust toolchain. The library uses Rust 2024 with MSRV 1.89.
Some test peers need a newer compiler.

[AGENTS.md](AGENTS.md) contains project policy.
[PLAN.md](PLAN.md) identifies the current work.
[coaptic-validation](https://github.com/jeffglousher/coaptic-validation) owns
independent peers, benchmarks and device tooling; this repository owns the
library, examples and core tests.

## Check a change

Choose checks for the code you changed. Run them after a coherent batch:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Use the relevant test filter while developing. Check without defaults when
feature gates or shared modules change:

```sh
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
```

Allocation and security changes also need their affected `alloc`, `oscore`,
`alloc,oscore` and `std` combinations. Transport, persistence and hardware
changes need relevant loopback or device proof. Keep both repository revisions,
features, capacities and runner identities with the result.

For rustdoc changes, build the docs and run relevant doctests. For Markdown-only
changes, review claims, examples and links; do not rerun protocol campaigns.

Pull requests run the advisory **Quick development checks** job. Use the
workflow's `full_validation` input for a requested full run. Enrollment-provider
checks are manual. GitHub's default CodeQL setup runs separately.

## Record useful evidence

Name the workload, security mode, storage and capacities. Preserve raw failures
and unsupported cases. Identify the source that actually ran; a newer CI pin
does not update an old measurement.

Separate object layout, heap requests, reserved stacks and process memory.
Nested objects are already included in their owners. Do not turn finite samples
into minimum-RAM, peak-memory or maximum-throughput claims.
See the [qualification checklist](architecture/requirements/VALIDATION-PLAN.md).

## Prepare an authorized release

Publication is separate work under
[#132](https://github.com/jeffglousher/coaptic/issues/132).
Select the exact candidate before running the complete matrix:

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

Inspect packaged files, licenses and dependency provenance, and complete the
agreed review. Package verification does not authorize publishing.

The `coaptic-io` macro crate generates async counterparts at compile time and
must be published before `coaptic`. Release tags trigger documentation and
crates.io publication; the tag must match the selected manifest version.
Use the authorized `CARGO_REGISTRY_TOKEN` secret. Never commit it.
After publication, verify installation and update dependency/documentation links.
