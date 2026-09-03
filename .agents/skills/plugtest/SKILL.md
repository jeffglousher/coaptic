---
name: plugtest
description: Use when adding or checking CoAP interoperability against this repo's plugtest bundle. Not for generic unit tests.
license: MIT OR Apache-2.0
---

# Plugtest

Read [knowledge/plugtest/](../../../knowledge/plugtest/) first, especially [requirements](../../../knowledge/plugtest/requirements.md).

- Implement in-scope TDs from the vendored YAML (`base`, `block`, `link`). Do not invent TD identifiers.
- Map each TD to the local RFC copies in `knowledge/rfcs/`. Open the `.txt`, not a rewrite.
- Skip `dtls` and `6lowpan` until security work. Do not implement DTLS or OSCORE to pass those TDs.
- [RFC 9175](../../../knowledge/rfcs/rfc9175.md) and [RFC 9177](../../../knowledge/rfcs/rfc9177.md) have no ETSI TDs. Derive checks from those RFC files.
- Keep the six-area bounded engine. Loss and separate-response cases must not invent extra pools.
