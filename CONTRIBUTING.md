# Contributing

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
cargo clippy -p coaptic-plugtest --all-targets --features dtls,oscore -- -D warnings
cargo test --no-default-features
cargo test --all-features
cargo test --no-default-features --features alloc
cargo test --no-default-features --features oscore
cargo test --no-default-features --features alloc,oscore
cargo test --no-default-features --features std
cargo test -p coaptic-plugtest --features dtls,oscore
cargo doc --no-deps --all-features
cargo package -p coaptic --locked
cargo +1.85.0 check --locked -p coaptic --no-default-features
cargo +1.85.0 check --locked -p coaptic --all-features
```

Transport and harness changes also run the [dogfood checks](crates/coaptic-plugtest/README.md)
and the [process suite](tools/interop/README.md). Job flags are in
[CI](.github/workflows/ci.yml). Process timing is the `process-interop` artifact.
Those timings are a smoke sample.

## Qualification commands

`tools/qualification/cross_build.py --target TARGET --output PATH` builds a
bounded App probe with Rust 1.97.1 for `thumbv6m-none-eabi`,
`thumbv7em-none-eabi`, `riscv32imac-unknown-none-elf`, and
`wasm32-unknown-unknown`, in core, alloc, OSCORE, and alloc+OSCORE. That is
release code generation. Device execution, stack high-water, allocators,
entropy, and networking are #202.

`python tools/qualification/seeded.py --output PATH` runs five campaigns on
Rust 1.97.1: 6,000 body operations, 50,000 datagram mutations, 50,000 CBOR
mutations, 100,000 replay-window steps, and 512 corruption/original/replay
sequences. Seeds sit next to the oracles. Fuzzing, branch coverage, and
compound soak are #202.

`python tools/qualification/host.py --output PATH` runs the six feature modes
on the host. CI also passes `--target i686-pc-windows-msvc`,
`--target i686-unknown-linux-gnu`, and, on Linux,
`--target s390x-unknown-linux-gnu`. The s390x run uses QEMU user-mode, one
test thread, and no rustdoc. Empty runs, timeouts, a 64-bit image on a 32-bit
job, or the wrong ELF endian fail the report. These jobs execute library tests.
They are not the process/DTLS matrix or an MCU run. MCU stack and runtime stay #202.

`python tools/qualification/coverage.py --work-root BUILD_PARENT --output REPORT.json`
archives LLVM counters for compiled `src/` in the all-feature build, including
unit-test modules. It is not branch coverage or an RFC checklist. No percentage
is an acceptance bar. Requirement and branch evidence stay #202.

## Release

Only `coaptic` is published. Harnesses and peers stay out of the crate.
`cargo package` excludes `knowledge/`, `tools/`, and the workspace test crates.
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
