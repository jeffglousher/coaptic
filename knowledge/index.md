---
okf_version: "0.2"
---

# Project

* [coaptic crate](crate.md) - Stand-alone `no_std` CoAP engine crate. Locked name `coaptic`.
* [External review](../REVIEW.md) - What the crate is, six areas, done vs deferred vs not planned, 0.1 API freeze, how to check.
* [Caller contract](../CALLER.md) - What the caller must own (send, clock, jitter, RST/4.xx, route/If-Match, Encode+Access).
* [Review policy](review.md) - Agents and humans are equal authors and reviewers.
* [OKF frontmatter](okf-frontmatter.md) - Three-pass concept frontmatter (extract → enrich → validate).

# Architecture

* [Architecture](architecture.md) - Six areas and bounded progress. Canonical source is `design.md`.
* [Memory areas](memory-areas.md) - The six core-managed areas, linked to `design.md` sections.
* [Memory](memory.md) - Locked datagram/body slots, Storage trait, builder, and names. Block-wise off: no body pools. Enabled default body 4096 = 4 × 1024.
* [Profiles](profiles.md) - Default vs Constrained capacity numbers.
* [Block testing](block-testing.md) - Combinatorial Block/Q-Block SZX × 1–25-block sweep. Default is not the test ceiling.
* [RFCs](rfcs/) - Local IETF CoAP/CoRE copies. One concept plus `.txt`/`.pdf` per RFC.
* [Plugtest](plugtest/) - ETSI CoAP plugtest TDs and the coaptic mapping.

# Operations

* [CI](ci.md) - PR-only `fmt`/`clippy`/`test`/`doc`/`okf`. `v*` tags publish rustdoc.
