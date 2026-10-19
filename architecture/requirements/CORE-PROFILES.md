# Core UDP profiles and resource contracts

This ledger follows the implementation and retained direct tests in the same
source revision. It records the finite profiles below, not every clause
of each RFC, an independent peer result, or current ESP32 peak memory use.
The [IETF plan](../IETF-RFC-PLAN.md) owns role selection and compliance levels.

## Profiles

- **UDP origin/client:** caller-owned unicast datagram I/O, time and entropy;
  sync and async App operations share protocol state. Security is explicit
  unprotected operation or pairwise OSCORE. `coaps` URI parsing identifies a
  required DTLS transport; parsing does not supply that transport.
- **Discovery:** enabled Site origin `/.well-known/core`, root-relative links,
  static borrowed route metadata, borrowed Link Format parsing and absolute
  `coap`/`coaps` URI views. Unsupported unicast filters are ignored.
- **Extended tokens:** RFC 8974 UDP with profile-selected owned capacity and an
  eight-byte compact default. Caller packet/scratch/header capacities remain
  independent limits. Reliable transport signaling and stateless-client state
  serialization are outside this profile.
- **Bounded App:** fixed or allocated storage with fixed post-construction
  capacities; explicit route, Call, deferred, association and security request
  limits. Start device configurations with SmallDevice/Constrained and name
  security/blockwise/backend choices.

## RFC 7252: selected applicable contracts

- **Sections 3, 4.1–4.8, 5.2–5.3:** shared datagram framing/options, full peer/token
  response matching, Message-ID ACK/RST handling, retries, duplicate handling and
  endpoint congestion state. Implemented in [message](../../src/message/mod.rs),
  [Engine](../../src/storage/engine.rs) and [App client](../../src/app/client.rs).
  [Storage tests](../../src/storage/tests.rs) exercise rotating progress and
  retry commit; [App tests](../../src/app/tests.rs) exercise deadlines, duplicate
  admission, response matching and recovery. This is a selected contract inventory,
  not a complete base-protocol clause/errata disposition.
- **Sections 5.10.1, 6.1–6.5:** split URI components before decoding; preserve
  path/query identity and IPv6 zone information; reject unresolved dot segments,
  userinfo, fragments and malformed escapes within the absolute-URI API.
  [URI implementation](../../src/uri.rs) and [six URI tests](../../tests/uri.rs)
  prove decoded identity, explicit transport, malformed-input recovery and atomic
  short-output refusal. DNS/interface resolution, relative-reference resolution,
  socket binding and DTLS remain caller responsibilities.
- **Sections 5.9.3.4, 5.10.5:** bounded general overload returns 5.03 with retry
  guidance where safe. [Admission](../../src/app/admission.rs) uses existing
  packet storage; [App admission tests](../../src/app/tests.rs) cover refusal
  before dispatch, send failure, held responses, cancellation and recovery.
  Authentication/replay failure may remain silent. No network-delivery guarantee
  follows from transport acceptance.

Proxy/cache roles, multicast discovery and reliable transports require separate
selected profiles; their clauses are not satisfied by these unicast APIs.

## RFC 6690 and applicable discovery attributes

The [origin discovery ledger](RFC6690.md) gives exact original clause mappings.

- **Sections 2–3:** [link_format](../../src/link_format.rs) parses borrowed links
  and attributes and serializes into caller storage. Named `rt`, `if`, `ct`,
  `sz` and `obs` handling preserves unknown extensions. The
  [11 direct tests](../../tests/link_format.rs) cover quoting, multiplicity,
  case-insensitive names, cardinal overflow, framing, exact capacity and atomic
  refusal. Parsing is incremental: consume the iterator to validate the whole
  document. Anchor/URI resolution and extension-specific semantics stay caller-owned.
- **Sections 4–4.1:** [Site](../../src/app/site.rs) supplies enabled GET discovery,
  Content-Format, empty catalog and query-ignore behavior; route metadata changes
  the catalog ETag. [Site metadata tests](../../src/app/site_metadata_tests.rs)
  prove replacement/removal, refusal/recovery and one borrowed pointer per route.
  Existing [App discovery tests](../../src/app/tests.rs) prove wire URI identity.
- **Proof boundary:** direct tests do not qualify an independent discovery client,
  protected remote interoperability, default-port deployment or multicast.
  RFC 9423 registry relevance does not imply every registered extension's semantics.
  RFC 9176 directory roles are unsupported contribution opportunities; they are not an unfinished adopted
  implementation hidden inside origin discovery.

## RFC 8974: UDP extended tokens

- **Section 2.1:** [shared codec](../../src/message/extended.rs) supports the
  extended token-length encodings and rejects reserved/malformed lengths.
  [Codec tests](../../tests/extended_tokens.rs) distinguish syntax from owned
  capacity, check wire boundaries/maximum and prove shared options/payload behavior.
- **Section 2.2.2:** [terminal probe](../../src/app/client_probe.rs) sends canonical
  CON GET/If-None-Match through ordinary bounded client ownership. Results preserve
  echoed success, permanent 4.00 length refusal, temporary 5.03 refusal, Reset and
  unknown outcomes. Caller-owned evidence expires after a selected 1800–86400
  seconds and binds exact route/epoch. [Probe tests](../../tests/token_probe.rs)
  cover sync/async, cancellation, classification and lifetime; the module test
  covers reused address/association epochs. Reset remains unauthenticated UDP
  control evidence, not cryptographic proof.
- **Section 2.2.2 capacity handling:** [App](../../src/app/mod.rs) echoes the complete
  borrowed token in 4.00 without manufacturing a maximum-token buffer or dispatching
  the handler. [OSCORE refusal](../../src/app/oscore.rs) authenticates and persists
  replay state first, then uses existing scratch. [Eight App tests](../../tests/extended_token_app.rs)
  cover fixed/allocated refusal/recovery, full-token matching, tamper, duplicate
  ciphertext, failed checkpoint and fresh recovery. Unprotected over-capacity
  requests cannot bypass attached-context/association-required policy.
- **Ownership:** [storage tests](../../tests/extended_token_storage.rs) prove full
  exchange/Observe/body identity and compact defaults. Reliable CSM negotiation and
  stateless state encodings remain unsupported. Companion #91 records independent
  unprotected extended-token peers. The protected native-libcoap32-byte success,
  authenticated64-byte capacity refusal, tag-tamper/no-effect and fresh protected
  recovery boundary passes at core 7bb32c9 / companion f6151265 in
  [run39254030000](https://github.com/jeffglousher/coaptic-validation/actions/runs/39254030000).
  The separate Observe fixture failures in that run remain retained; they do not
  invalidate this completed row or establish broader token/backend support.

## RFC 8516: overload and retry

- **Sections 3–4:** [Response](../../src/app/response.rs) exposes 4.29 and 5.03.
  The application selects client-specific throttling; admission uses general
  overload. [Client](../../src/app/client.rs) retains method/peer/path/query
  similarity and cooldown in existing Call identity storage. Absent Max-Age means
  60 seconds; concurrent instructions retain the longest deadline.
- **Section 5:** only a matching authenticated response changes protected-call
  backoff. The `overload_*` [App tests](../../src/app/tests.rs) cover default/zero
  age, exact expiry, malformed/unmatched instructions, shared admission, loopback
  recovery and OSCORE authentication. Independent-peer qualification and workload
  fairness/performance measurements remain separate evidence.

## Bounded-resource contracts and issue disposition

- **Admission (#586): implemented.** One RX and one TX headroom within configured
  pools; checked construction, ordinary/control separation, bounded refusal,
  fixed-cardinality counters and recovery. Raw Engine APIs expose physical
  capacity and require caller policy. [Scratch tests](../../tests/scratch_admission.rs),
  [resource tests](../../tests/resources.rs) and App tests cover these boundaries.
- **Associations/scheduling (#340): implemented mechanisms.** Bounded association
  rows bind owner/epoch, revocation and replay/request lifetime;
  [association tests](../../src/app/tests.rs) exercise equal endpoint/token/MID
  isolation, stale handles, checkpoint failure and shared physical NSTART.
  [Wakeup](../../src/app/wake.rs), async driving and rotating Engine progress
  expose bounded readiness; arbitrary caller handlers/I/O are outside core work
  bounds. Remaining deployment accounting must measure stack/heap initialization
  peaks and transport/security reservations on the named firmware/runtime.
- **Accounting:** [footprint checks](../../tests/footprint.rs) enforce independent
  whole-App ceilings and metadata ownership; [resources](../../tests/resources.rs)
  checks bounded allocated configuration. Static layout is not deployment peak RAM.
- **Diagnostics (#521): caller-owned implementation.** Existing
  [Metrics](../../src/storage/metrics.rs), optional [WorkMetrics](../../src/storage/work_metrics.rs),
  [StorageUsage](../../src/storage/usage.rs) and [AppUsage](../../src/app/usage.rs)
  expose counters/current occupancy. [Diagnostics](../../src/app/diagnostics.rs)
  combines these with readiness into a caller-owned snapshot. Its optional recorder
  counts twelve fixed categories of surfaced errors and retains sampled occupancy
  maxima. [Fixed/allocated tests](../../tests/app_diagnostics.rs) cover capacity
  refusal, expiry/cancellation, reset independence and transport recovery; the module
  test covers wrapping/reset. No App field, peer label or event buffer is added.
  Authentication/replay errors share one category; silent drops are not counted
  by this recorder. Sampled maxima are not exact lifetime high-water marks.
  Counters wrap; callers choose whether to record both a poll error and its later
  Call failure. The host export adapter is delivered in companion #88; overhead remains #341.
- **Disposition:** #516, #519, #521 and #586 are closed for their delivered
  implementations and relevant proof. Directory roles are outside the selected
  endpoint scope. #200 records completed finite independent integrations and explicit peer limits; #340 owns
  named deployment peaks and valid-peer progress. Completed association/readiness
  mechanisms are not a second implementation backlog.

Retained local results in the reviewed implementation PRs support the named
checks; exact executed source/toolchain identities belong with that evidence.
These direct checks do not promote independent interoperability, hardware
qualification or whole-RFC compliance levels.

