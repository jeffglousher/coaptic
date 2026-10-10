# Development

Humans and agents follow [AGENTS.md](AGENTS.md). Planning is
[GitHub project 2](https://github.com/users/jeffglousher/projects/2). Protocol
copies are `knowledge/rfcs/`. Public API documentation is rustdoc.

Use the toolchain in `rust-toolchain.toml`. The library is edition 2024 and
MSRV 1.85. Test peers have their own dependency requirements. Merges to `main`
are squash-only and must pass CI.

## Local validation

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

## Companion validation

[coaptic-validation](https://github.com/jeffglousher/coaptic-validation) holds
interoperability peers, hardware tooling, benchmarks, and qualification runners.
The library, examples, and unit/integration tests here are Rust.

CI calls the companion workflow at a pinned commit against the exact library
revision. Follow its [run instructions](https://github.com/jeffglousher/coaptic-validation#run)
for transport, security, or harness changes. Preserve both source revisions,
lockfiles, setup limitations, and reports. The process timings are smoke samples;
measurements remain preliminary. Updating the suite pin is a reviewed change.

## Release

Only `coaptic` is published. Harnesses and peers stay out of the crate.
`cargo package` includes only the library, Rust examples/tests, and supporting notices.
Release tags start the rustdoc and crates.io workflows. Ordinary development
does not create those tags.

For a release, first merge the reviewed change to `main` with required CI green
and complete the validation above. Inspect the packaged files and license
notices. Configure the repository's `CARGO_REGISTRY_TOKEN` secret with a
crates.io token authorized to publish `coaptic`; do not put credentials in git.
Preparation is followed by maintainer/external review and agreement on the
final release snapshot before publication. The publication tag must match the
manifest version exactly (`v0.0.10` for `0.0.10`). Pushing that tag publishes
the crate. Package preparation alone does not authorize that step.
After publication, replace the
README's Git dependency with the released version and point the manifest's
documentation URL to `https://docs.rs/coaptic`.
