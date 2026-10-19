# IETF RFC Implementation and Compliance Plan

## Purpose

This plan defines Coaptic's IETF RFC commitments, supported implementation
boundaries, compliance evidence and remaining protocol work. It is the
architectural authority for RFC scope. [Project 2](https://github.com/users/jeffglousher/projects/2)
and the linked owner issues schedule implementation. Public API contracts remain
in rustdoc. Original protocol text is retained in [knowledge/rfcs](../knowledge/rfcs/index.md).

The implementation is a portable Rust library with secure defaults, explicit
resource limits and caller-owned I/O, runtime, time, entropy and storage. This is
pre-release development, without compatibility or version-bump obligations.
Default operation requires neither `std` nor allocation; `std` and
`alloc` are independent. Allocated storage retains checked limits. These are
implementation constraints, not exceptions to applicable protocol requirements.

## Architecture and development focus

Coaptic's development focus is CoAP over UDP, pairwise OSCORE message protection,
and EDHOC establishment of authenticated OSCORE contexts. Provisioning, enrollment,
ownership, authorization and recovery are part of delivering a usable solution.
They are not satisfied by an encrypted demonstration with manually staged keys.
The immediate deployment targets are constrained devices, including the reserved
ESP32-S3, and Rust host services. Taldra is a consumer and contributor; its
deployment needs inform reusable contracts without coupling the libraries.

TCP/TLS/WebSockets, reliable-transport signaling/BERT and DTLS identity/version
profiles are welcome contribution opportunities. They are outside active MVP
development, with no promised delivery or later completion gate. This replaces
the earlier deferred delivery commitments for those features. A contribution
must name its supported boundary and meet the same correctness, secure-default,
resource-bound and focused validation requirements as actively developed code.
Existing codecs and companion fixtures are not delivered endpoint support.

The operator selected direct UDP endpoints for this milestone. Resource Directory,
proxy/gateway/cache, multicast and standardized ACE authorization are unsupported
contribution opportunities, alongside reliable transports and DTLS. They have no
promised delivery or later completion gate. This resolves the role-selection
questions in #517, #518 and the directory/ACE portions of #519/#365; it does not
claim those protocols were implemented. Adoption requires a concrete use case,
security/trust model and bounded implementation and acceptance contract.
Discovery does not grant trust, forwarding does not grant authorization, and
multicast does not implicitly adopt group security.

UDP is an architectural choice. Reliability, congestion handling, scalability
and comparative performance claims require measurements of named workloads,
sources and environments; no universal superiority claim follows from that choice.

### Resource and delivery acceptance

Constrained means deterministic resource ownership and bounded memory/work for
reliability, performance and ease of integration. Accepted capabilities remain
required. Each implementation must include capacity exhaustion, refusal and
recovery; a fixed array by itself does not establish a sufficient memory budget.
Reuse identity/body storage across lifetimes where possible and account for all
metadata, scratch, initialization, security and platform reservations. Keep total
ceilings independent of feature allowances and distinguish static layout from
whole-system peak use.

Use `SmallDevice` with `profiles::Constrained` pools as the starting device
configuration. Explicitly record its route, Call, deferred and security capacities
and any overrides. Security, blockwise transfers and storage backend are options
of the same App. Compare equal-capability workloads and change one setting at a
time; a profile name alone is not a memory or performance specification.

The execution priority is completed coding and integration on real loopback and
the reserved ESP32. Focused success/refusal/recovery checks accompany each batch.
Benchmark expansion, long stability runs and presentation are not prerequisites
for implementing the next accepted requirement. Current-state reports use current
source/configuration evidence or mark a metric unmeasured. Historical evidence
stays archived; it is not the default report or a substitute for current results.

## Provisioning and enrollment decisions

The published specifications contribute distinct parts of the solution:

- RFC 8613 protects application messages using an established OSCORE context.
- RFC 9528 establishes authenticated keys with EDHOC; RFC 9668 maps EDHOC to
  CoAP and OSCORE. The application profile still defines credential trust,
  authentication choices and how the peer becomes authorized.
- RFCs 9200/9203 provide standardized authorization-server, client and resource-
  server exchanges and OSCORE proof-of-possession tokens. They are candidates
  for delegated permissions, not a substitute for deciding initial device
  ownership. RFC 9203 has its own context-provisioning procedure; it is not
  automatically an authorization layer on the existing EDHOC-derived context.
- [draft-ietf-lake-authz-08](https://datatracker.ietf.org/doc/html/draft-ietf-lake-authz-08)
  describes manufacturer-trust-assisted enrollment through EDHOC EAD. This exact
  draft is not adopted; refresh official publication state before adoption or a
  current-status claim.

Commissioning requires authenticated encryption and explicit owner authorization.
The guided Rust CLI defaults to PIV approval with separately provisioned
touch-required or explicit touchless development credentials. Native WebAuthn is
an optional provider; PIV touch is not FIDO2 user verification. Production approval
and custody claims require qualification of the selected provider and platform.
Approval is required for administrative transactions, not on ordinary restart.
Alternative providers preserve the same authenticated transaction and durable
trust contract; the approval key is distinct from each device's identity key.

The default onboarding path must be secure and convenient: guided authenticated
local pairing, unique credentials and durable permissions, followed by automatic
encrypted operation and restart recovery. Support alternative commissioning
providers behind that contract. Users must not need a cloud account, recurring
operator taps, manual cryptographic parameter selection or disabled message
protection to complete ordinary operation. The implemented device workflow uses
the trusted privileged local USB commissioning channel to bind device and owner
keys. Physical presence alone or an unverified device name does not authenticate
a peer; that channel's local custody is an explicit deployment assumption.

Stricter administrative approval, preinstalled organization/manufacturer trust
and restrictive management or authorization-expiry policies are explicit hardening
choices. Convenience changes the commissioning effort and policy, not the need
for authenticated encryption and peer/owner authorization. Verification, storage
or policy failures must not trigger unprotected traffic or unverified-first-contact fallback.

- Convenient path: generate a unique device key, authenticate and pin it
  through a scanned public-key fingerprint or trusted local commissioning
  channel, then complete EDHOC and an explicit ownership/permission decision.
  Self-generated credentials do not require a public CA, but their first binding
  to the intended device and owner still needs authentication in both directions.
- Hardened candidate: independently provision manufacturer/organization trust,
  authenticate device and management authority, require the selected approval
  policy, and qualify credential custody and rollback resistance on the platform.
  Hardware-backed custody is a platform capability, not a generic library claim.

Blind trust on first use cannot exclude an active attacker at first contact
(RFC 9528 Appendix D.5). Initial trust must use the authenticated commissioning
channel; verification failure must refuse pairing.

Both paths need explicit decisions for owner claim/transfer, authority trust,
revocation, replacement, reset/re-enrollment, entropy, durable
state and application-effect receipts. Core enrollment authority, local pairing, managed reconnection and trust/store
contracts exist. Remaining platform workflows and physical qualification are tracked in [#365](https://github.com/jeffglousher/coaptic/issues/365) and
[#333](https://github.com/jeffglousher/coaptic/issues/333).

### Durable offline pairing

Paired devices must communicate directly for long periods without internet or a
continually reachable enrollment authority. Initial provisioning establishes each
device's own unique persistent identity key in qualified device custody and
installs authenticated peer/owner credentials and permissions. The operator's
approval credential authenticates that transaction; it is distinct from the
device's identity key. Ordinary messages, reconnects and
unplug/replug or power-cycle recovery require neither a hardware-key tap nor
another human pairing ceremony. Management tooling must support local authorized
commissioning; internet availability is not an enrollment prerequisite.

After restart, restore the qualified identity/trust state and establish fresh
session keys automatically with EDHOC. The current profile keeps session keys
volatile; restored identities do not justify restoring old session keys, nonce
counters or exchanges. Long-lived authorization and automatically refreshed
session keys have separate lifecycles. A short online-authorization lease must
not impose a periodic operator presence or reconnection requirement on this path.

The normal default uses durable local flash under the operator's physical custody,
with `LocalPairing` and `PolicyValidity::OfflineLocalStorage`. Restore the same
identity, owner, permissions and revocation tombstone after ordinary restart.
Physical flash rollback protection and hardware-backed custody are optional
hardening. `PeerTrust` with `PolicyValidity::OfflineProtectedAnchor` retains its
stronger, separately qualified protected monotonic authority contract; a record
and counter in ordinary flash do not meet it. Qualify actual device commits,
local trust/permission restoration and automated peer recovery on ESP32. Core
contracts and in-memory tests alone do not establish those platform guarantees.

Revocation takes effect when an authenticated update reaches the enforcing peer,
or through authorized local management. A disconnected peer cannot know about an
undelivered remote revocation. Once applied, revoke/regrant, replacement or owner
changes must invalidate affected grants, sessions, pending work and cached effects
and survive restart. Optional expiring/online policies retain their own explicit
contracts; they are not the default durable-pairing requirement.

## Commitment and document state

| Commitment | Meaning |
| --- | --- |
| Committed | Required for the current protocol milestone. Missing implementation or proof remains an open gap. |
| Contribution opportunity | Contributions welcome outside active MVP development. No delivery promise or completion gate; adoption requires a supported boundary and reviewed implementation/proof. |
| Interest | Candidate capability with no current completion gate. Adoption requires an explicit scope decision. |
| Reference | Guidance or a dependency with named applicable uses; not a promise to implement every feature in that document. |

Document state is independent of commitment. Published Standards Track,
Informational and Experimental RFCs are distinguished from exact-version drafts.
Informational category does not remove applicable normative requirements.
Published status does not establish implementation, security or interoperability.
Updates, obsoletions and considered errata belong in each requirement assessment.

A compliance boundary names the delivered library/adapter stack, client/server or
intermediary role, transport, authentication method, algorithm, options and actual
application profile. An optional role can remain unsupported; an unconditional
requirement cannot be waived merely by calling support a selected profile.
Caller responsibilities require an explicit, usable integration contract.
Companion fixtures do not establish a supported core API.

## Implementation assessment

Requirement-specific implementation and direct proof are defined by the linked
RFC ledgers, whose source references resolve in the same Git revision as the
ledger. Other profile ratings use the general source baseline below.

The [core profiles](requirements/CORE-PROFILES.md),
[Observe and transfer profiles](requirements/OBSERVE-TRANSFERS.md) and
[security profiles](requirements/SECURITY-PROFILES.md) organize the selected roles
into finite requirement groups. The detailed discovery and EDHOC ledgers cover
individual boundaries. Each maps applicability, implementation and focused
evidence; none declares whole-RFC compliance. The
[qualification plan](requirements/VALIDATION-PLAN.md) owns the remaining independent,
platform and campaign proof, with exact prerequisites and exit conditions.

This assessment is versioned with the implementation: local ledger/source links
resolve within the same Git tree. The reconciliation starts from merged core
`b371059` and companion `72a87df`; accompanying source changes and their focused
proof are recorded by their delivery PRs and owner issues. Historical firmware or
captures retain their exact earlier revisions. Ratings describe a named role,
not every role or all requirements in its RFC.

| Level | Assessment |
| --- | --- |
| L0 | No delivered implementation of the named role. |
| L1 | Encoding, decoding or constants only; no complete runtime/API for the role. |
| L2 | Partial public API/profile; material implementation gaps remain. |
| L3 | Usable named profile with direct tests and finite execution evidence; applicable requirements are not yet fully assessed. |
| L4 | All applicable requirements have reviewed implementation and proof, with justified SHOULD dispositions and resolved counterexamples. |

No RFC is rated L4 by this assessment. There is no aggregate compliance percentage:
missing mandatory requirements cannot be offset by unrelated passing tests.
Independent interoperability and local requirement tests are separate evidence
dimensions. Counts, smoke campaigns and hosted CI alone are not conformance proof.

In the following table, ST means Standards Track, Info means Informational and
Exp means Experimental. Titles are shortened for readability.

## CoAP and CoRE scope

| RFC / document state | Commitment and implementation boundary | Implementation | Remaining proof / owner |
| --- | --- | --- | --- |
| [6690](../knowledge/rfcs/rfc6690.txt) — CoRE Link Format; ST | Committed: origin discovery resource, link grammar and applicable metadata. | L3: bounded metadata, parser, serializer and URI identity; [discovery ledger](requirements/RFC6690.md). | Implementation delivered in #598. Independent discovery and URI directions remain #200; optional query filtering remains unsupported with query-ignore behavior. #519 records delivery and the unsupported RD boundary. |
| [7252](../knowledge/rfcs/rfc7252.txt) — CoAP; ST | Committed: UDP origin client/server with OSCORE security assessed under RFC 8613. Contribution opportunity: proxy/cache, HTTP mapping and multicast roles. | L3: origin exchanges; L0: delivered DTLS binding and intermediary/group roles. | Finish applicable UDP requirement ledger, duplicate/congestion/addressing proof and the named OSCORE stack boundary. Section 9's DTLS NoSec/RPK requirement is not implemented by OSCORE; no whole-RFC/section-9 compliance claim or MVP DTLS delivery promise follows. [#199](https://github.com/jeffglousher/coaptic/issues/199), [#200](https://github.com/jeffglousher/coaptic/issues/200). Contribution binding: [#515](https://github.com/jeffglousher/coaptic/issues/515). Unsupported contribution roles: [#517](https://github.com/jeffglousher/coaptic/issues/517), [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [7390](../knowledge/rfcs/rfc7390.txt) — Group Communication; Exp | Interest: experimental group profile. Base multicast is unsupported under RFC 7252, not by adopting this RFC wholesale. | L0. | No group runtime or multi-responder API. Keep experimental and revised-draft commitments distinct. [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [7641](../knowledge/rfcs/rfc7641.txt) — Observe; ST | Committed: client/server relations and named valid transfer/selection interactions. | L3: ordinary and selected GET/FETCH, direct/deferred, classic/Q and bounded CON recovery profiles. | [Observe/transfer ledger](requirements/OBSERVE-TRANSFERS.md) records whole-resource nonzero classic registration, selected Q initial ranges, bounded recovery and validator handling. #520 owns delivery; independent directions remain #200. Changed Content-Format cannot become a different-format 2.xx notification on one relation. |
| [7959](../knowledge/rfcs/rfc7959.txt) — Block-Wise Transfers; ST | Committed: origin Block1/Block2, complete representations and validators. | L3: bounded origin transfers, complete immutable snapshots and current validators. | Applicable direct profiles and explicit unsupported combinations are mapped in the Observe/transfer ledger. Remaining independent cancellation, expiry and representation-identity directions are #200. |
| [7967](../knowledge/rfcs/rfc7967.txt) — No-Response; Info | Committed: option processing, supported masks and valid interactions. | L3: supported masks, ACK behavior and explicit pre-I/O progress restrictions. | The Observe/transfer ledger classifies registration, classic Continue and CON/NON Q upload restrictions. Suppressed silence never establishes application success; independent proof remains #200. |
| [8075](../knowledge/rfcs/rfc8075.txt) — HTTP-to-CoAP Mapping; ST | Contribution opportunity: application gateway use case and ownership boundary. HTTP transport support is outside active core UDP development. | L0. | Unsupported in this milestone. A contribution needs method/status/media-type/conditional/stream/error/security mapping and independent proof. CoAP-to-HTTP is separately assessed under RFC 7252 section 10. [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [8132](../knowledge/rfcs/rfc8132.txt) — FETCH, PATCH and iPATCH; ST | Committed: protocol methods and exact selection/transfer identity. Application patch formats remain separately named payload contracts. | L3: methods, retained selections, fragmented follow-ups and validator-aware Observe. | The Observe/transfer ledger maps selection/transfer identity and direct tests. Independent directions remain #200. Application JSON patch fixture subsets do not establish universal patch-format semantics. |
| [8323](../knowledge/rfcs/rfc8323.txt) — TCP, TLS and WebSockets; ST | Contribution opportunity: reliable client/server sessions, signaling and negotiated BERT; no delivery promise or MVP gate. | L1: Engine BERT codecs; L0: reliable session runtime. | A contributed profile needs framing/partial I/O, CSM/Ping/Pong/Release/Abort, lifecycle/identity epochs, reviewed TLS/WS adapters, negotiated size/BERT ownership and independent directions. [#516](https://github.com/jeffglousher/coaptic/issues/516). |
| [8516](../knowledge/rfcs/rfc8516.txt) — Too Many Requests; ST | Committed: explicit server overload policy and client similar-request backoff contract. | L3: explicit 4.29/5.03 constructors and bounded similar-request cooldown, merged in #567. | Applications choose client-specific versus general overload. Reserve/refusal delivery is #586; independent/device overload proof remains #200/#333. |
| [8613](../knowledge/rfcs/rfc8613.txt) — OSCORE; ST | Committed: pairwise origin endpoints and inner blockwise transfers. Contribution opportunity: protected intermediary/Outer Block-wise role. | L3: supported pairwise profile; L0: protected intermediary. | Bounded associations, owner epochs, nonce/replay persistence and admission are delivered. Complete independent profile proof and deployment budgets remain #200/#333; see the security ledger. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). Intermediary: [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [8710](../knowledge/rfcs/rfc8710.txt) — Multipart Content-Format; ST | Interest: bounded multipart codec/API. | L0: no dedicated codec; bytes can be transported opaquely. | No multipart compliance claim. Adoption requires a payload API and grammar/limit tests; opaque transport is not format implementation. |
| [8768](../knowledge/rfcs/rfc8768.txt) — Hop-Limit; ST | Committed: origin/codec handling. Contribution opportunity: proxy forwarding/loop refusal. | L1: option codec and 5.08 constant; L0: intermediary behavior. | Separate exact origin behavior from decrement/forward/loop rules and prove each delivered role. [#517](https://github.com/jeffglousher/coaptic/issues/517), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8974](../knowledge/rfcs/rfc8974.txt) — Extended Tokens and Stateless Clients; ST | Committed: bounded UDP extended-token client/server support. Contribution opportunity: reliable capability negotiation. Interest: stateless-client recipes. | L3: configurable owned UDP tokens, complete matching, canonical probing and authenticated capacity refusal, merged in #598. | Compact eight-byte defaults remain. Full tokens are echoed without maximum-sized scratch; probes bind association/owner epoch and expiry. #516 is delivered; independent directions remain #200. |
| [9175](../knowledge/rfcs/rfc9175.txt) — Echo, Request-Tag and Token Processing; ST | Committed: endpoint freshness, request identity and token rules. | L3: caller-owned Echo and transfer identity mechanisms. | Full applicable requirement ledger, owner/epoch/lifetime isolation and interaction evidence. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [9176](../knowledge/rfcs/rfc9176.txt) — Resource Directory; ST | Contribution opportunity: registration/update/removal/lookup client and directory server roles. | L0. | Unsupported in this milestone. A contribution needs link/URI foundations, leases, endpoint/owner epochs, lookup filters and authoritative mutation policy. [#519](https://github.com/jeffglousher/coaptic/issues/519). |
| [9177](../knowledge/rfcs/rfc9177.txt) — Q-Block; ST | Committed: unicast origin transfers. Contribution opportunity: applicable proxy role with full-representation caching. | L3: bounded unicast transfers and selected Observe/CON recovery; L0: proxy. | The Observe/transfer ledger states the one-set CON recovery capacity/refusal contract and immutable epochs. Remaining independent directions are #200. Proxy/cache support is outside this milestone; no arbitrary transparent or multicast forwarding claim follows. |
| [9178](../knowledge/rfcs/rfc9178.txt) — Cellular Power-Efficient Devices; Info | Reference: deployment/power guidance. | Not rated. | No cellular protocol profile or modem power qualification claim follows from core portability. |
| [9203](../knowledge/rfcs/rfc9203.txt) — ACE OSCORE Profile; ST | Contribution opportunity: standardized authorization interoperability for a named delegated-access use case. | L0: ACE wire profile. | ACE framework/token roles, context provisioning, policy/audience/expiry and independent profile proof if adopted. Custom enrollment/current policy is not ACE. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9290](../knowledge/rfcs/rfc9290.txt) — Concise Problem Details; ST | Committed: bounded CBOR problem APIs. | L3: public encode/decode/response API and direct tests. | Clause-level required-field, extension, text/metadata, malformed-input and round-trip proof; no general media-codec claim. [#202](https://github.com/jeffglousher/coaptic/issues/202). |
| [9423](../knowledge/rfcs/rfc9423.txt) — Target Attributes Registry; Info | Reference with committed applicability: registered discovery attribute names/semantics. | L3: registered attribute semantics in the bounded metadata API. | Discovery parser/serializer and registry applicability are assessed in the discovery ledger. RD is unsupported; registry administration is not a deliverable. #519/#200. |
| [9668](../knowledge/rfcs/rfc9668.txt) — EDHOC with CoAP and OSCORE; ST | Committed: baseline transport/context establishment for the named provisioning profile. Interest: optional combined-request profile, outside the message_4-required MVP profile. | L3: sequential bootstrap with message_4 required and suites 2/3; [bootstrap option ledger](requirements/RFC7252-EDHOC-OPTIONS.md) and [security ledger](requirements/SECURITY-PROFILES.md). | Independent establishment/error/EAD proof remains #200/#365. Combined requests are optional and unsupported by the message_4-required profile; do not advertise combined support. |
| [9876](../knowledge/rfcs/rfc9876.txt) — Content-Format Registration Updates; ST | Reference with committed applicability: identifier use and documentation reservations. | Not rated. | Apply relevant registry rules; no new content codec or registration campaign is implied. |
| [9952](../knowledge/rfcs/rfc9952.txt) — CoAP over DTLS ALPN; Info | Reference conditional on a contributed DTLS adapter profile; no MVP gate. | L0: supported core binding absent. | State ALPN applicability/support in a contributed backend profile and qualify it when used. This RFC adds no universal new DTLS ALPN handshake requirement. [#515](https://github.com/jeffglousher/coaptic/issues/515). |

## Security and format dependencies

Each dependency is assessed for its actual use, not as a claim to implement a
general-purpose replacement for the dependency library. Original copies not yet
retained locally must be imported from the official RFC source when their
requirement ledger is created.

| RFC / document state | Commitment and implementation boundary | Implementation | Remaining proof / owner |
| --- | --- | --- | --- |
| [5246](https://www.rfc-editor.org/rfc/rfc5246.html) — TLS 1.2; ST, obsoleted by RFC 8446 | Reference: applicable DTLS 1.2 record/alert rules. | Companion profile evidence only. | Preserve exact failure-stage alert semantics and the unresolved GCM finding; no general core TLS claim. [#486](https://github.com/jeffglousher/coaptic/issues/486), [#199](https://github.com/jeffglousher/coaptic/issues/199). |
| [6347](https://www.rfc-editor.org/rfc/rfc6347.html) — DTLS 1.2; ST, obsoleted by RFC 9147 | Contribution opportunity: named Rust adapter/identity profiles; no delivery promise or MVP gate. | L2: companion PSK qualification; L0: supported core endpoint binding. | A contributed profile needs a supported endpoint API, declared PSK/RPK/X.509 trust profiles, bounds, independent proof and exact literal-test dispositions. Existing companion findings remain open. No obsolete/unsafe suite is required merely by retaining this version. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7250](https://www.rfc-editor.org/rfc/rfc7250.html) — Raw Public Keys; ST | Contribution opportunity: DTLS RPK profile, conditional on a contributed binding. | L0: DTLS RPK path unsupported. | A contributed profile needs credential negotiation/trust/identity and actual RPK loss/refusal/recovery proof; X.509 does not substitute. EDHOC pinned public-key credentials are a different protocol binding. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199). |
| [9147](https://www.rfc-editor.org/rfc/rfc9147.html) — DTLS 1.3; ST | Contribution opportunity: reviewed backend/version profile; no delivery promise or MVP gate. | L0. | A contributed profile needs version-specific state/record/identity/congestion behavior and independent proof; DTLS 1.2 evidence is not reusable as version compliance. [#515](https://github.com/jeffglousher/coaptic/issues/515). |
| [9200](https://www.rfc-editor.org/rfc/rfc9200.html) — ACE-OAuth; ST | Contribution opportunity: authorization-server/resource-server/client wire roles for delegated access. | L0: ACE; custom policy foundations exist separately. | Unsupported in this milestone. Adoption needs named token formats, audience/expiry/current-policy/owner isolation and interoperable endpoints. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9202](https://www.rfc-editor.org/rfc/rfc9202.html) — ACE DTLS Profile; ST, updated by RFC 9430 | Contribution opportunity: standardized DTLS authorization profile; no MVP gate. | L0. | A contributed profile needs applicable update inventory, authz-info/token/bootstrap and policy proof; ordinary DTLS is insufficient. [#365](https://github.com/jeffglousher/coaptic/issues/365), [#515](https://github.com/jeffglousher/coaptic/issues/515). |
| [9528](../knowledge/rfcs/rfc9528.txt) — EDHOC; ST | Committed: method 3 and declared credential/application profile, with applicable section 8 requirements. | L3: method 3, pinned CCS/kid, suites 2/3, full-width exporter, bounded EAD/errors/CIDs and secret erasure; [security ledger](requirements/SECURITY-PROFILES.md). | Scoped section-8 audit and erasure fixes delivered in #598. Complete independent profile proof remains #200/#365. Other authentication/credential profiles are unsupported; local vectors do not establish independent interoperability. |
| [9594](https://www.rfc-editor.org/rfc/rfc9594.html) — ACE Group Key Provisioning; ST | Interest: group KDC framework plus separately selected specialized provisioning profile. | L0. | Not a current gate; framework alone is not complete Group OSCORE provisioning. [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [8949](https://www.rfc-editor.org/rfc/rfc8949.html) — CBOR; ST | Reference: encoding/decoding required by problem details, security and declared native payloads. | Selected fixed-storage codecs exist. | Record exact accepted types/deterministic encoding/limits per consumer. No universal CBOR implementation claim. [#202](https://github.com/jeffglousher/coaptic/issues/202). |
| [9052](https://www.rfc-editor.org/rfc/rfc9052.html) — COSE Structures; ST | Reference: applicable OSCORE/EDHOC structures. | Selected protected-message structures exist. | Trace exact structures and reconcile superseded draft references; no general COSE feature claim. [#365](https://github.com/jeffglousher/coaptic/issues/365), [#202](https://github.com/jeffglousher/coaptic/issues/202). |
| [9053](https://www.rfc-editor.org/rfc/rfc9053.html) — COSE Algorithms; ST | Reference: algorithms used by declared profiles. | Selected algorithms/dependencies exist. | Record algorithm identifiers, parameters and source-specific proof; crypto agility does not require every registered algorithm. [#365](https://github.com/jeffglousher/coaptic/issues/365), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [5869](https://www.rfc-editor.org/rfc/rfc5869.html) — HKDF; Info | Reference: selected key derivation in OSCORE/EDHOC. | Rust dependency/profile integration exists. | Preserve algorithm/input/output bindings and relevant vectors; primitive tests alone do not prove protocol context derivation. [#202](https://github.com/jeffglousher/coaptic/issues/202). |

The dependency ledger must also include applicable updates and transitive
normative requirements discovered during clause assessment. This annex is the
current direct-use inventory, not a declaration that dependency closure is complete.

## Drafts and experimental interest

| Document | Commitment | Implementation boundary and required disposition |
| --- | --- | --- |
| [draft-ietf-core-groupcomm-bis](https://datatracker.ietf.org/doc/draft-ietf-core-groupcomm-bis/) | Interest | Revised group communication; outside current completion gate. Pin exact version and refresh publication state before adoption. |
| [draft-ietf-core-oscore-groupcomm](https://datatracker.ietf.org/doc/draft-ietf-core-oscore-groupcomm/) | Interest | Group OSCORE; no implementation claim. Security/group epochs and specialized provisioning need a separately accepted profile. |
| draft-tiloca-lake-implem-cons | Reference under reconciliation | Vendored credential-checking comments reference an unversioned draft. Identify actual revision, applicable final requirements and reached behavior. |
| draft-ietf-cose-rfc8152bis | Superseded reference under reconciliation | Vendored Enc_structure comment refers to a COSE draft. Assess actual code against applicable published COSE requirements. |
| [draft-ietf-lake-authz-08](https://datatracker.ietf.org/doc/html/draft-ietf-lake-authz-08) | Interest: exact revision -08, not adopted | ELA manufacturer-trust-assisted enrollment candidate. Draft-linked AccessDenied code/comment is not implementation evidence. Assess actual EAD/trust/service roles before adopting this exact version or claiming support. |

Draft citations and enum values are not draft-conformance evidence. Assessment
records exact version and verified publication status; assigned author-copy RFC
numbers do not establish publication. Draft interest does not block the committed
milestone. General platform targets, including the [Linux eBPF pilot #522](https://github.com/jeffglousher/coaptic/issues/522),
have separate implementation plans and do not change IETF RFC commitments.

## Compliance acceptance

Each committed role has a requirement ledger with:

1. RFC/version, section and stable requirement identity, considered update/erratum.
2. Role/profile and normative MUST/MUST NOT/SHOULD/MAY classification.
3. Applicability and an evidence-backed rationale for exclusions or profile qualifications.
4. Implementation API/source, owner issue and integration responsibilities.
5. Exact test/peer/backend/source identities, result, limitations and unresolved counterexamples.

SHOULD deviations document their reason and consequences. Optional features are
declared supported or unsupported; once supported, their applicable requirements
remain binding. Syntax refusal, operational capacity refusal and unsupported
feature refusal are distinct. Reserved limits must permit required protocol
responses without unauthorized ownership or security-state changes.
Protocol-required response and counter transitions remain explicit, with durable
reservation where required, and preserve subsequent recovery.

Every increment proves positive behavior, refusal/failure and fresh recovery for
its supported fixed/allocated configurations. Independent wire proof identifies
the actual peer/backend and direction. Shared-backend fixtures do not establish
independent cryptographic interoperability. A literal fixture/RFC conflict gets a
reviewed disposition; a substitute alert or scenario is never relabeled PASS.
Failed observations and required-feature SKIPs remain open evidence gaps.

Historical captures, measurements and firmware stay bound to their exact sources.
No current-source attribution follows from a matching feature name or newer CI
pin. A compliance claim names the completed ledger, delivered profile and source;
whole-RFC wording is withheld while applicable mandatory requirements remain open.

## Delivery and remaining proof

The implementation baseline includes #598: bounded associations and admission,
readiness/deadlines, extended UDP tokens, discovery metadata/URI views, selected
Observe/transfer work, durable owner/backup recovery and receipt integration, and
EDHOC erasure. #589 delivered shared sync/async capabilities. These are present
code, not an unstarted backlog. Later changes update their specific ledgers.

This reconciliation implements #520 nonzero/selection/validator interactions and
#521 caller-owned diagnostics without additional App storage. #486 corrects the
companion connection-level GCM failure policy and requires its own exact capture
proof. Each closes only after merged success/refusal/recovery proof. #340 no longer describes the App as
limited to one security association; its remaining deployment accounting belongs
with #333/#341 rather than a second association implementation.

The [qualification plan](requirements/VALIDATION-PLAN.md) separates the remaining
literal scenarios (#199), independent peers/addressing (#200), finite host/parser
campaigns and diagnostic adapters (#202), physical custody/persistence (#333),
and controlled measurements (#341). These are actual remaining proof, not CI
requirements and not implied by cross-compilation or layout tests. #365 owns the
installed authorization/recovery boundary and its unresolved counterexamples.
The Linux hook pilot #522 needs an actual Linux verifier/hook execution lane;
Windows tests cannot establish it.

Publication #132 is an owner decision after a concrete candidate, package review
and requested release qualification. No development issue, project status or
successful test authorizes tags, registry publication or external outreach.
Hosted CI is advisory during pre-release; focused local proof and review support
authorized squash merges without waiting for the hosted matrix.

## Evidence and implementation anchors

- [Public integration boundary](../src/lib.rs), [datagram I/O](../src/storage/io.rs),
  [token codecs](../src/message/extended.rs) and [bounded token probes](../src/app/client_probe.rs).
- [Discovery](../src/app/site.rs), [Link Format](../src/link_format.rs),
  [URI views](../src/uri.rs), [App dispatch](../src/app/mod.rs) and
  [client selection/transfer](../src/app/client.rs).
- [Admission reserve/refusal](../src/app/admission.rs),
  [caller-owned diagnostics](../src/app/diagnostics.rs) and
  [whole-object ceilings](../tests/footprint.rs).
- [EDHOC/profile](../src/provisioning/mod.rs), [CoAP mapping](../src/provisioning/coap.rs),
  [durable local pairing](../src/provisioning/local_pairing.rs),
  [application receipts](../src/provisioning/receipt.rs) and
  [guided CLI](../tools/coaptic-enroll/README.md).
- [Companion process inventory](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/interop/capabilities.json),
  [DTLS qualification adapter](https://github.com/jeffglousher/coaptic-validation/blob/main/crates/peer-coaptic/src/dtls.rs)
  and [qualification plan](requirements/VALIDATION-PLAN.md).

These locate implementations and test contracts, not executed current-peer or
firmware results. Owner issues and delivery PRs retain exact source/backend/image
identities for each observation. Source presence or aggregate test totals do not
promote an assessment to L4.
