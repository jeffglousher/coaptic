---
okf_version: "0.2"
---

# Project

* [coaptic crate](crate.md) - Stand-alone `no_std` CoAP engine crate. Locked name `coaptic`.
* [Review policy](review.md) - Agents and humans are equal authors and reviewers.
* [OKF frontmatter](okf-frontmatter.md) - Three-pass concept frontmatter (extract → enrich → validate).

# Architecture

* [Architecture](architecture.md) - Six areas and bounded progress. Canonical source is `design.md`.
* [Memory areas](memory-areas.md) - The six core-managed areas, linked to `design.md` sections.
* [RFCs](rfcs/) - Local IETF CoAP/CoRE copies. One concept plus `.txt`/`.pdf` per RFC.
* [Plugtest](plugtest/) - ETSI CoAP plugtest TDs and the coaptic mapping.

# Operations

* [CI](ci.md) - PR-only `fmt`/`clippy`/`test`/`doc`/`okf`. `v*` tags publish rustdoc.
