# IETF RFC Implementation and Compliance Plan

## Purpose

This plan defines Coaptic's IETF RFC commitments, supported implementation
boundaries, compliance evidence and remaining protocol work. It is the
architectural authority for RFC scope. [Project 2](https://github.com/users/jeffglousher/projects/2)
and the linked owner issues schedule implementation. Public API contracts remain
in rustdoc. Original protocol text is retained in [knowledge/rfcs](../knowledge/rfcs/index.md).

The implementation is a portable Rust library with secure defaults, explicit
resource limits and caller-owned I/O, runtime, time, entropy and storage. Version
remains 0.0.10. Default operation requires neither `std` nor allocation; `std` and
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

Resource Directory, proxy/gateway/cache, multicast and standardized ACE
authorization are being selected individually with the operator. Their previous
planning entries remain visible below as selection in progress; this interview
has neither adopted all of them into the initial MVP nor declared them complete.
Selection requires a concrete Coaptic or consumer use case, a security/trust
model and a bounded implementation and acceptance contract. Discovery does not
grant trust, forwarding does not grant authorization, and multicast does not
implicitly adopt group security.

UDP is an architectural choice. Reliability, congestion handling, scalability
and comparative performance claims require measurements of named workloads,
sources and environments; no universal superiority claim follows from that choice.

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
  describes manufacturer-trust-assisted enrollment through EDHOC EAD. It remains
  an Internet-Draft, checked 2026-10-16, not a published RFC or an adopted profile.

Convenient and hardened commissioning both retain authenticated encryption and
fail-closed authorization. The operator has accepted a physical setup/verification
step and selected standard FIDO2 operator approval for the default initial
provisioning workflow. The connected development key is a verified YubiKey 4,
firmware 4.3.3, with PIV and U2F but no FIDO2. The operator explicitly selected a
separate no-touch PIV development profile for testing; production touch approval
remains a separate profile. The default workflow uses the operator's key at
initial setup; alternative commissioning providers may serve other deployment needs.

The default onboarding path must be secure and convenient: guided authenticated
local pairing, unique credentials and durable permissions, followed by automatic
encrypted operation and restart recovery. Support alternative commissioning
providers behind that contract. Users must not need a cloud account, recurring
operator taps, manual cryptographic parameter selection or disabled message
protection to complete ordinary operation. The exact default verification channel remains
an implementation decision; physical presence alone or an unverified device name
does not authenticate a peer.

Stricter administrative approval, preinstalled organization/manufacturer trust
and restrictive management or authorization-expiry policies are explicit hardening
choices. Convenience changes the commissioning effort and policy, not the need
for authenticated encryption and peer/owner authorization. Verification, storage
or policy failures must not trigger plaintext or unverified-first-contact fallback.

- Convenient path: generate a unique device key, authenticate and pin it
  through a scanned public-key fingerprint or trusted local commissioning
  channel, then complete EDHOC and an explicit ownership/permission decision.
  Self-generated credentials do not require a public CA, but their first binding
  to the intended device and owner still needs authentication in both directions.
- Hardened candidate: independently provision manufacturer/organization trust,
  authenticate device and management authority, require the selected approval
  policy, and qualify credential custody and rollback resistance on the platform.
  Hardware-backed custody is a platform capability, not a generic library claim.

These are selected requirements and design candidates, not delivered setup
workflows. Blind trust on first
use can pin continuity after first contact, but cannot exclude an active attacker
at that first contact (RFC 9528 Appendix D.5). It is not equivalent to verified
commissioning and must not become the default or an automatic fallback.

Both paths need explicit decisions for owner claim/transfer, authority trust,
revocation, replacement, reset/re-enrollment, entropy, durable
state and application-effect receipts. Core enrollment authority and trust/store
contracts exist; authenticated platform providers and the deployed ESP32 workflow
remain open in [#365](https://github.com/jeffglousher/coaptic/issues/365) and
[#333](https://github.com/jeffglousher/coaptic/issues/333).

### Durable offline pairing

Paired devices must communicate directly for long periods without internet or a
continually reachable enrollment authority. Initial provisioning establishes each
device's own unique persistent identity key in qualified device custody and
installs authenticated peer/owner credentials and permissions. The operator's
FIDO2 credential authenticates approval of that transaction; it is distinct from
the device's identity key. Ordinary messages, reconnects and
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
| Selection in progress | Previously planned role being considered individually in the architecture interview. The initial MVP boundary for it is unresolved, not silently removed or completed. |
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

The three existing ledgers cover selected discovery requirements, EDHOC CoAP
options and EDHOC error reception. They map a requirement to applicability,
implementation and focused evidence. They are not a count of implemented
features or tests, and they do not assess their entire RFCs. Complete the missing
mapping beside each implementation change, reuse applicable source-bound proof,
and distinguish a stale assessment from missing implementation.

Assessment baseline: core
[`cf94aeeadb07663319a85d417e5108625fc85639`](https://github.com/jeffglousher/coaptic/tree/cf94aeeadb07663319a85d417e5108625fc85639),
companion
[`430c5940a005463b350762eafc19f544c9590a7a`](https://github.com/jeffglousher/coaptic-validation/tree/430c5940a005463b350762eafc19f544c9590a7a).
Ratings apply to the named implementation boundary, not every role in the RFC.

| Level | Assessment |
| --- | --- |
| L0 | No delivered implementation of the named role. |
| L1 | Encoding, decoding or constants only; no complete runtime/API for the role. |
| L2 | Partial public API/profile; material implementation gaps remain. |
| L3 | Usable named profile with direct tests and finite execution evidence; applicable requirements are not yet fully assessed. |
| L4 | All applicable requirements have reviewed implementation and proof, with justified SHOULD dispositions and resolved counterexamples. |

No RFC is rated L4 at this baseline. There is no aggregate compliance percentage:
missing mandatory requirements cannot be offset by unrelated passing tests.
Independent interoperability and local requirement tests are separate evidence
dimensions. Counts, smoke campaigns and hosted CI alone are not conformance proof.

In the following table, ST means Standards Track, Info means Informational and
Exp means Experimental. Titles are shortened for readability.

## CoAP and CoRE scope

| RFC / document state | Commitment and implementation boundary | Baseline | Missing implementation or proof / owner |
| --- | --- | --- | --- |
| [6690](../knowledge/rfcs/rfc6690.txt) — CoRE Link Format; ST | Committed: origin discovery resource, link grammar and applicable metadata. | L2: bounded catalog with directly proved URI identity, dot refusal, empty output and unicast query-ignore behavior; [requirement ledger](requirements/RFC6690.md). | Remaining committed metadata/parsing scope, complete applicability inventory and independent discovery interoperability. Optional filtering remains unsupported with required query-ignore behavior. [#519](https://github.com/jeffglousher/coaptic/issues/519). |
| [7252](../knowledge/rfcs/rfc7252.txt) — CoAP; ST | Committed: UDP origin client/server with OSCORE security assessed under RFC 8613. Selection in progress: proxy/cache, HTTP mapping and multicast roles. | L3: origin exchanges; L0: delivered DTLS binding and intermediary/group roles. | Finish applicable UDP requirement ledger, duplicate/congestion/addressing proof and the named OSCORE stack boundary. Section 9's DTLS NoSec/RPK requirement is not implemented by OSCORE; no whole-RFC/section-9 compliance claim or MVP DTLS delivery promise follows. [#199](https://github.com/jeffglousher/coaptic/issues/199), [#200](https://github.com/jeffglousher/coaptic/issues/200). Contribution binding: [#515](https://github.com/jeffglousher/coaptic/issues/515). Roles under selection: [#517](https://github.com/jeffglousher/coaptic/issues/517), [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [7390](../knowledge/rfcs/rfc7390.txt) — Group Communication; Exp | Interest: experimental group profile. Base multicast selection is assessed under RFC 7252, not by adopting this RFC wholesale. | L0. | No group runtime or multi-responder API. Keep experimental and revised-draft commitments distinct. [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [7641](../knowledge/rfcs/rfc7641.txt) — Observe; ST | Committed: client/server relations and named valid transfer/selection interactions. | L3: ordinary relations; L2: complete committed combination scope. | Server FETCH/query-GET selection-aware registration, deferred selection, nonzero initial Block2 and Q-Block2 profiles, supersession ownership. Changed Content-Format cannot become a different-format 2.xx notification on one relation. [#520](https://github.com/jeffglousher/coaptic/issues/520); independent proof [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7959](../knowledge/rfcs/rfc7959.txt) — Block-Wise Transfers; ST | Committed: origin Block1/Block2, complete representations and validators. | L3: bounded client/server transfers. | Applicable requirement ledger and named selection/Observe interactions; cancellation, expiry, conditional and representation-identity proof by direction. [#520](https://github.com/jeffglousher/coaptic/issues/520), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7967](../knowledge/rfcs/rfc7967.txt) — No-Response; Info | Committed: option processing, supported masks and valid interactions. | L2: public API with typed restrictions. | Accepted upload/registration combinations and exact suppression/control behavior. Classify required versus optional behavior before extending APIs. [#520](https://github.com/jeffglousher/coaptic/issues/520). |
| [8075](../knowledge/rfcs/rfc8075.txt) — HTTP-to-CoAP Mapping; ST | Selection in progress: application gateway use case and ownership boundary. HTTP transport support is outside active core UDP development. | L0. | Select the actual gateway need before committing method/status/media-type/conditional/stream/error/security mapping and independent proof. CoAP-to-HTTP is separately assessed under RFC 7252 section 10. [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [8132](../knowledge/rfcs/rfc8132.txt) — FETCH, PATCH and iPATCH; ST | Committed: protocol methods and exact selection/transfer identity. Application patch formats remain separately named payload contracts. | L3: method API; L2: complete selection/transfer scope. | Selected FETCH follow-up fragmentation, server selected Observe and applicable validators. Peer JSON patch fixtures cover a limited application subset, not universal patch semantics. [#520](https://github.com/jeffglousher/coaptic/issues/520), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8323](../knowledge/rfcs/rfc8323.txt) — TCP, TLS and WebSockets; ST | Contribution opportunity: reliable client/server sessions, signaling and negotiated BERT; no delivery promise or MVP gate. | L1: Engine BERT codecs; L0: reliable session runtime. | A contributed profile needs framing/partial I/O, CSM/Ping/Pong/Release/Abort, lifecycle/identity epochs, reviewed TLS/WS adapters, negotiated size/BERT ownership and independent directions. [#516](https://github.com/jeffglousher/coaptic/issues/516). |
| [8516](../knowledge/rfcs/rfc8516.txt) — Too Many Requests; ST | Committed: explicit server overload policy and client similar-request backoff contract. | L0: no dedicated delivered profile/proof. | Distinguish client-specific 4.29 from general 5.03 overload; bound refusal response work and honor applicable Max-Age/default/similar-request rules. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8613](../knowledge/rfcs/rfc8613.txt) — OSCORE; ST | Committed: pairwise origin endpoints and inner blockwise transfers. Selection in progress: protected intermediary/Outer Block-wise role. | L3: supported pairwise profile; L0: protected intermediary. | Requirement/algorithm/context inventory, multiple bounded association ownership, nonce/replay/persistence boundaries and remaining independent modes. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). Intermediary: [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [8710](../knowledge/rfcs/rfc8710.txt) — Multipart Content-Format; ST | Interest: bounded multipart codec/API. | L0: no dedicated codec; bytes can be transported opaquely. | No multipart compliance claim. Adoption requires a payload API and grammar/limit tests; opaque transport is not format implementation. |
| [8768](../knowledge/rfcs/rfc8768.txt) — Hop-Limit; ST | Committed: origin/codec handling. Selection in progress: proxy forwarding/loop refusal. | L1: option codec and 5.08 constant; L0: intermediary behavior. | Separate exact origin behavior from decrement/forward/loop rules and prove each delivered role. [#517](https://github.com/jeffglousher/coaptic/issues/517), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8974](../knowledge/rfcs/rfc8974.txt) — Extended Tokens and Stateless Clients; ST | Committed: bounded UDP extended-token client/server support. Contribution opportunity: reliable capability negotiation. Interest: stateless-client recipes. | L0: extended tokens; existing tokens limited to eight bytes. | Shared codec/storage/matching, UDP support discovery and capacity-refusal response budgeting. Reliable CSM integration is conditional on a contributed reliable transport. Distinguish recognized syntax beyond capacity from malformed encoding. [#516](https://github.com/jeffglousher/coaptic/issues/516). |
| [9175](../knowledge/rfcs/rfc9175.txt) — Echo, Request-Tag and Token Processing; ST | Committed: endpoint freshness, request identity and token rules. | L3: caller-owned Echo and transfer identity mechanisms. | Full applicable requirement ledger, owner/epoch/lifetime isolation and interaction evidence. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [9176](../knowledge/rfcs/rfc9176.txt) — Resource Directory; ST | Selection in progress: registration/update/removal/lookup client and directory server roles. | L0. | Select the needed roles; acceptance needs link/URI foundations, leases, endpoint/owner epochs, lookup filters and authoritative mutation policy. [#519](https://github.com/jeffglousher/coaptic/issues/519). |
| [9177](../knowledge/rfcs/rfc9177.txt) — Q-Block; ST | Committed: unicast origin transfers. Selection in progress: applicable proxy role with full-representation caching. | L3: bounded transfers; L2: complete committed interactions; L0: proxy. | Named Observe/selection combinations and remaining requirement/direction proof. Proxy scope must follow section 3, not arbitrary transparent/random-access/multicast forwarding. [#520](https://github.com/jeffglousher/coaptic/issues/520), [#200](https://github.com/jeffglousher/coaptic/issues/200), [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [9178](../knowledge/rfcs/rfc9178.txt) — Cellular Power-Efficient Devices; Info | Reference: deployment/power guidance. | Not rated. | No cellular protocol profile or modem power qualification claim follows from core portability. |
| [9203](../knowledge/rfcs/rfc9203.txt) — ACE OSCORE Profile; ST | Selection in progress: standardized authorization interoperability for a named delegated-access use case. | L0: ACE wire profile. | ACE framework/token roles, context provisioning, policy/audience/expiry and independent profile proof if adopted. Custom enrollment/current policy is not ACE. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9290](../knowledge/rfcs/rfc9290.txt) — Concise Problem Details; ST | Committed: bounded CBOR problem APIs. | L3: public encode/decode/response API and direct tests. | Clause-level required-field, extension, text/metadata, malformed-input and round-trip proof; no general media-codec claim. [#202](https://github.com/jeffglousher/coaptic/issues/202). |
| [9423](../knowledge/rfcs/rfc9423.txt) — Target Attributes Registry; Info | Reference with committed applicability: registered discovery attribute names/semantics. | L2: metadata scope incomplete. | Apply relevant registry rules to discovery/directory APIs; registry administration is not a deliverable. [#519](https://github.com/jeffglousher/coaptic/issues/519). |
| [9668](../knowledge/rfcs/rfc9668.txt) — EDHOC with CoAP and OSCORE; ST | Committed: baseline transport/context establishment for the named provisioning profile. Interest: optional combined-request profile, outside the message_4-required MVP profile. | L2: sequential bootstrap with message_4 required; direct elective/critical role, length, repetition and bounded refusal proof, including initial recovery admission, in the [bootstrap option ledger](requirements/RFC7252-EDHOC-OPTIONS.md). | Audit identifier/context mapping, EDHOC error/profile requirements and applicable discovery; qualify independent option-extension peers and sustained admission/fairness. Combined requests are optional; a message_4-required profile must not advertise combined support. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9876](../knowledge/rfcs/rfc9876.txt) — Content-Format Registration Updates; ST | Reference with committed applicability: identifier use and documentation reservations. | Not rated. | Apply relevant registry rules; no new content codec or registration campaign is implied. |
| [9952](../knowledge/rfcs/rfc9952.txt) — CoAP over DTLS ALPN; Info | Reference conditional on a contributed DTLS adapter profile; no MVP gate. | L0: supported core binding absent. | State ALPN applicability/support in a contributed backend profile and qualify it when used. This RFC adds no universal new DTLS ALPN handshake requirement. [#515](https://github.com/jeffglousher/coaptic/issues/515). |

## Security and format dependencies

Each dependency is assessed for its actual use, not as a claim to implement a
general-purpose replacement for the dependency library. Original copies not yet
retained locally must be imported from the official RFC source when their
requirement ledger is created.

| RFC / document state | Commitment and implementation boundary | Baseline | Missing implementation or proof / owner |
| --- | --- | --- | --- |
| [5246](https://www.rfc-editor.org/rfc/rfc5246.html) — TLS 1.2; ST, obsoleted by RFC 8446 | Reference: applicable DTLS 1.2 record/alert rules. | Companion profile evidence only. | Preserve exact failure-stage alert semantics and the unresolved GCM finding; no general core TLS claim. [#486](https://github.com/jeffglousher/coaptic/issues/486), [#199](https://github.com/jeffglousher/coaptic/issues/199). |
| [6347](https://www.rfc-editor.org/rfc/rfc6347.html) — DTLS 1.2; ST, obsoleted by RFC 9147 | Contribution opportunity: named Rust adapter/identity profiles; no delivery promise or MVP gate. | L2: companion PSK qualification; L0: supported core endpoint binding. | A contributed profile needs a supported endpoint API, declared PSK/RPK/X.509 trust profiles, bounds, independent proof and exact literal-test dispositions. Existing companion findings remain open. No obsolete/unsafe suite is required merely by retaining this version. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7250](https://www.rfc-editor.org/rfc/rfc7250.html) — Raw Public Keys; ST | Contribution opportunity: DTLS RPK profile, conditional on a contributed binding. | L0: DTLS RPK path unsupported. | A contributed profile needs credential negotiation/trust/identity and actual RPK loss/refusal/recovery proof; X.509 does not substitute. EDHOC pinned public-key credentials are a different protocol binding. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199). |
| [9147](https://www.rfc-editor.org/rfc/rfc9147.html) — DTLS 1.3; ST | Contribution opportunity: reviewed backend/version profile; no delivery promise or MVP gate. | L0. | A contributed profile needs version-specific state/record/identity/congestion behavior and independent proof; DTLS 1.2 evidence is not reusable as version compliance. [#515](https://github.com/jeffglousher/coaptic/issues/515). |
| [9200](https://www.rfc-editor.org/rfc/rfc9200.html) — ACE-OAuth; ST | Selection in progress: authorization-server/resource-server/client wire roles for delegated access. | L0: ACE; custom policy foundations exist separately. | Select the needed roles and OSCORE profile; adoption needs named token formats, audience/expiry/current-policy/owner isolation and interoperable endpoints. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9202](https://www.rfc-editor.org/rfc/rfc9202.html) — ACE DTLS Profile; ST, updated by RFC 9430 | Contribution opportunity: standardized DTLS authorization profile; no MVP gate. | L0. | A contributed profile needs applicable update inventory, authz-info/token/bootstrap and policy proof; ordinary DTLS is insufficient. [#365](https://github.com/jeffglousher/coaptic/issues/365), [#515](https://github.com/jeffglousher/coaptic/issues/515). |
| [9528](../knowledge/rfcs/rfc9528.txt) — EDHOC; ST | Committed: method 3 and declared credential/application profile, with applicable section 8 requirements. | L2: reference credentials and message_4 profile; selected [inbound error processing](requirements/RFC9528-ERROR-RECEPTION.md). Since the assessment baseline, selected sequential errors/equal-ID delivery and suites 2/3 were delivered in PRs #539–#553; no L4 promotion. | Complete clause/error/CBOR/credential/CID/context inventory and independent peers. Absent a qualifying application profile, section 8 requires kid, exporter, errors 1/2, EAD and suites 2/3. Single method support is permitted. Qualify the actual profile and complete requirements; no after-the-fact profile invented to disguise gaps. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
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
| [draft-ietf-lake-authz-08](https://datatracker.ietf.org/doc/html/draft-ietf-lake-authz-08) | Selection in progress; Internet-Draft, checked 2026-10-16 | ELA manufacturer-trust-assisted enrollment candidate. Draft-linked AccessDenied code/comment is not implementation evidence. Assess actual EAD/trust/service roles before adopting this exact version or claiming support. |

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

## Gap priority

| Priority | Gap | Implementation owner / completion evidence |
| --- | --- | --- |
| P0, companion maintenance | Unresolved companion GCM structural/authentication finding. | [#486](https://github.com/jeffglousher/coaptic/issues/486): reviewed correction and strict source-bound refusal/recovery; no finding suppression or substitute CCM proof. Moving production DTLS outside the MVP does not resolve this existing finding or permit affected traffic. |
| P1 | Usable UDP/OSCORE security boundary, applicable EDHOC requirements and deployed enrollment. | [#365](https://github.com/jeffglousher/coaptic/issues/365), [#333](https://github.com/jeffglousher/coaptic/issues/333): selected commissioning/trust decisions, authenticated providers, exact 7252/8613/9528/9668 requirements and physical approval/revocation/recovery. |
| P1 | Bounded association/context ownership and fair progress needed by protocol roles. | [#340](https://github.com/jeffglousher/coaptic/issues/340): capacity/epoch/replay/expiry/cancel isolation, checked refusal and valid-peer recovery. |
| P2 | Remaining discovery metadata/parsing, Observe/selection/transfer APIs, overload/backoff and extended tokens. | [#519](https://github.com/jeffglousher/coaptic/issues/519), [#520](https://github.com/jeffglousher/coaptic/issues/520), [#340](https://github.com/jeffglousher/coaptic/issues/340), [#516](https://github.com/jeffglousher/coaptic/issues/516): narrow clause-driven implementation and independent directions. |
| P2 | Missing requirement ledgers and independent evidence for established profiles. | [#200](https://github.com/jeffglousher/coaptic/issues/200), [#202](https://github.com/jeffglousher/coaptic/issues/202): finite requirement inventory; no full-compliance claim from fixture totals. |
| Selection before scheduling | Proxy/cache/gateway, RD, ACE and base multicast roles. | [#517](https://github.com/jeffglousher/coaptic/issues/517), [#519](https://github.com/jeffglousher/coaptic/issues/519), [#365](https://github.com/jeffglousher/coaptic/issues/365), [#518](https://github.com/jeffglousher/coaptic/issues/518): decide each use case, trust model, supported role and completion contract in the architecture interview. Reliable transports/BERT and DTLS are contribution opportunities outside this queue. |

## Evidence and implementation anchors

- [Core public boundary](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/lib.rs),
  [datagram I/O](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/storage/io.rs),
  [token/codec](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/message/mod.rs).
- [Discovery](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/app/site.rs),
  [App boundaries](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/app/mod.rs),
  [client selection/transfer](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/app/client.rs).
- [EDHOC profile](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/provisioning/mod.rs),
  [CoAP mapping](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/provisioning/coap.rs),
  [problem-details API](https://github.com/jeffglousher/coaptic/blob/cf94aeeadb07663319a85d417e5108625fc85639/src/message/problem.rs).
- [Companion DTLS adapter](https://github.com/jeffglousher/coaptic-validation/blob/430c5940a005463b350762eafc19f544c9590a7a/crates/peer-coaptic/src/dtls.rs),
  [process case inventory](https://github.com/jeffglousher/coaptic-validation/blob/430c5940a005463b350762eafc19f544c9590a7a/tools/interop/capabilities.json),
  [qualified protected Observe evidence](https://github.com/jeffglousher/coaptic/pull/510#issuecomment-6123500065).

These anchors support the baseline assessment. Each completed clause requires its
own proof record; source presence and these aggregate references are insufficient
to promote a row to L4.
