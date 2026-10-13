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

## Commitment and document state

| Commitment | Meaning |
| --- | --- |
| Committed | Required for the current protocol milestone. Missing implementation or proof remains an open gap. |
| Deferred commitment | Required planned capability, sequenced after foundational work. Included in full-stack completion; not represented as completed or excluded. |
| Interest | Candidate capability with no current completion gate. Adoption requires an explicit scope decision. |
| Reference | Guidance or a dependency with named applicable uses; not a promise to implement every feature in that document. |

Document state is independent of commitment. Published Standards Track,
Informational and Experimental RFCs are distinguished from exact-version drafts.
Informational category does not remove applicable normative requirements.
Published status does not establish implementation, security or interoperability.
Updates, obsoletions and considered errata belong in each requirement assessment.

A compliance boundary names the delivered library/adapter stack, client/server or
intermediary role, transport, authentication method, algorithm, options and actual
application profile. An optional role can be deferred; an unconditional
requirement cannot be waived merely by calling support a selected profile.
Caller responsibilities require an explicit, usable integration contract.
Companion fixtures do not establish a supported core API.

## Implementation assessment

Requirement-specific implementation and direct proof are defined by the linked
RFC ledgers, whose source references resolve in the same Git revision as the
ledger. Other profile ratings use the general source baseline below.

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
| [7252](../knowledge/rfcs/rfc7252.txt) — CoAP; ST | Committed: UDP origin client/server and supported Rust security binding. Deferred: proxy/cache, HTTP mapping and multicast roles. | L3: origin exchanges; L0: delivered DTLS binding and intermediary/group roles. | Section9 requires NoSec and RawPublicKey modes for that specification. Explicit NoSec compatibility preserves secure defaults; OSCORE/PSK/X.509 evidence does not replace RPK. Finish base requirement ledger, duplicate/congestion/addressing proof and supported stack boundary. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199), [#200](https://github.com/jeffglousher/coaptic/issues/200). Deferred roles: [#517](https://github.com/jeffglousher/coaptic/issues/517), [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [7390](../knowledge/rfcs/rfc7390.txt) — Group Communication; Exp | Interest: experimental group profile. Committed/deferred base multicast is assessed under RFC 7252, not by adopting this RFC wholesale. | L0. | No group runtime or multi-responder API. Keep experimental and revised-draft commitments distinct. [#518](https://github.com/jeffglousher/coaptic/issues/518). |
| [7641](../knowledge/rfcs/rfc7641.txt) — Observe; ST | Committed: client/server relations and named valid transfer/selection interactions. | L3: ordinary relations; L2: complete committed combination scope. | Server FETCH/query-GET selection-aware registration, deferred selection, nonzero initial Block2 and Q-Block2 profiles, supersession ownership. Changed Content-Format cannot become a different-format 2.xx notification on one relation. [#520](https://github.com/jeffglousher/coaptic/issues/520); independent proof [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7959](../knowledge/rfcs/rfc7959.txt) — Block-Wise Transfers; ST | Committed: origin Block1/Block2, complete representations and validators. | L3: bounded client/server transfers. | Applicable requirement ledger and named selection/Observe interactions; cancellation, expiry, conditional and representation-identity proof by direction. [#520](https://github.com/jeffglousher/coaptic/issues/520), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7967](../knowledge/rfcs/rfc7967.txt) — No-Response; Info | Committed: option processing, supported masks and valid interactions. | L2: public API with typed restrictions. | Accepted upload/registration combinations and exact suppression/control behavior. Classify required versus optional behavior before extending APIs. [#520](https://github.com/jeffglousher/coaptic/issues/520). |
| [8075](../knowledge/rfcs/rfc8075.txt) — HTTP-to-CoAP Mapping; ST | Deferred commitment: bounded Rust HTTP-to-CoAP gateway. | L0. | Method/status/media-type/conditional/stream/error/security mapping and independent gateway proof. CoAP-to-HTTP is separately assessed under RFC 7252 section 10. [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [8132](../knowledge/rfcs/rfc8132.txt) — FETCH, PATCH and iPATCH; ST | Committed: protocol methods and exact selection/transfer identity. Application patch formats remain separately named payload contracts. | L3: method API; L2: complete selection/transfer scope. | Selected FETCH follow-up fragmentation, server selected Observe and applicable validators. Peer JSON patch fixtures cover a limited application subset, not universal patch semantics. [#520](https://github.com/jeffglousher/coaptic/issues/520), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8323](../knowledge/rfcs/rfc8323.txt) — TCP, TLS and WebSockets; ST | Deferred commitment: named reliable client/server sessions, signaling and negotiated BERT. | L1: Engine BERT codecs; L0: reliable session runtime. | Framing/partial I/O, CSM/Ping/Pong/Release/Abort, lifecycle/identity epochs, reviewed TLS/WS adapters, negotiated size/BERT ownership and independent directions. [#516](https://github.com/jeffglousher/coaptic/issues/516). |
| [8516](../knowledge/rfcs/rfc8516.txt) — Too Many Requests; ST | Committed: explicit server overload policy and client similar-request backoff contract. | L0: no dedicated delivered profile/proof. | Distinguish client-specific 4.29 from general 5.03 overload; bound refusal response work and honor applicable Max-Age/default/similar-request rules. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8613](../knowledge/rfcs/rfc8613.txt) — OSCORE; ST | Committed: pairwise origin endpoints and inner blockwise transfers. Deferred: protected intermediary/Outer Block-wise role. | L3: supported pairwise profile; L0: protected intermediary. | Requirement/algorithm/context inventory, multiple bounded association ownership, nonce/replay/persistence boundaries and remaining independent modes. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). Intermediary: [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [8710](../knowledge/rfcs/rfc8710.txt) — Multipart Content-Format; ST | Interest: bounded multipart codec/API. | L0: no dedicated codec; bytes can be transported opaquely. | No multipart compliance claim. Adoption requires a payload API and grammar/limit tests; opaque transport is not format implementation. |
| [8768](../knowledge/rfcs/rfc8768.txt) — Hop-Limit; ST | Committed: origin/codec handling. Deferred: proxy forwarding/loop refusal. | L1: option codec and 5.08 constant; L0: intermediary behavior. | Separate exact origin behavior from decrement/forward/loop rules and prove each delivered role. [#517](https://github.com/jeffglousher/coaptic/issues/517), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [8974](../knowledge/rfcs/rfc8974.txt) — Extended Tokens and Stateless Clients; ST | Committed: bounded UDP extended-token client/server support. Deferred: reliable capability negotiation. Interest: stateless-client recipes. | L0: extended tokens; existing tokens limited to eight bytes. | Shared codec/storage/matching and UDP support discovery, reliable CSM integration and capacity-refusal response budgeting. Distinguish recognized syntax beyond capacity from malformed encoding. [#516](https://github.com/jeffglousher/coaptic/issues/516). |
| [9175](../knowledge/rfcs/rfc9175.txt) — Echo, Request-Tag and Token Processing; ST | Committed: endpoint freshness, request identity and token rules. | L3: caller-owned Echo and transfer identity mechanisms. | Full applicable requirement ledger, owner/epoch/lifetime isolation and interaction evidence. [#340](https://github.com/jeffglousher/coaptic/issues/340), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [9176](../knowledge/rfcs/rfc9176.txt) — Resource Directory; ST | Deferred commitment: bounded registration/update/removal/lookup client and directory server. | L0. | Link/URI foundations, leases, endpoint/owner epochs, lookup filters and authoritative mutation policy. [#519](https://github.com/jeffglousher/coaptic/issues/519). |
| [9177](../knowledge/rfcs/rfc9177.txt) — Q-Block; ST | Committed: unicast origin transfers. Deferred: applicable proxy role with full-representation caching. | L3: bounded transfers; L2: complete committed interactions; L0: proxy. | Named Observe/selection combinations and remaining requirement/direction proof. Proxy scope must follow section 3, not arbitrary transparent/random-access/multicast forwarding. [#520](https://github.com/jeffglousher/coaptic/issues/520), [#200](https://github.com/jeffglousher/coaptic/issues/200), [#517](https://github.com/jeffglousher/coaptic/issues/517). |
| [9178](../knowledge/rfcs/rfc9178.txt) — Cellular Power-Efficient Devices; Info | Reference: deployment/power guidance. | Not rated. | No cellular protocol profile or modem power qualification claim follows from core portability. |
| [9203](../knowledge/rfcs/rfc9203.txt) — ACE OSCORE Profile; ST | Deferred commitment: standardized authorization interoperability. | L0: ACE wire profile. | ACE framework/token roles, context provisioning, policy/audience/expiry and independent profile proof. Custom enrollment/current policy is not ACE. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9290](../knowledge/rfcs/rfc9290.txt) — Concise Problem Details; ST | Committed: bounded CBOR problem APIs. | L3: public encode/decode/response API and direct tests. | Clause-level required-field, extension, text/metadata, malformed-input and round-trip proof; no general media-codec claim. [#202](https://github.com/jeffglousher/coaptic/issues/202). |
| [9423](../knowledge/rfcs/rfc9423.txt) — Target Attributes Registry; Info | Reference with committed applicability: registered discovery attribute names/semantics. | L2: metadata scope incomplete. | Apply relevant registry rules to discovery/directory APIs; registry administration is not a deliverable. [#519](https://github.com/jeffglousher/coaptic/issues/519). |
| [9668](../knowledge/rfcs/rfc9668.txt) — EDHOC with CoAP and OSCORE; ST | Committed: baseline transport/context establishment for the named provisioning profile. Deferred: optional combined-request profile. | L2: sequential bootstrap with message_4 required; direct unknown-elective option proof in the [bootstrap option ledger](requirements/RFC7252-EDHOC-OPTIONS.md). | Audit identifier/context mapping and applicable discovery; complete critical-option wire handling and known-option dispositions. Combined requests are optional; a message_4-required profile must not advertise combined support. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9876](../knowledge/rfcs/rfc9876.txt) — Content-Format Registration Updates; ST | Reference with committed applicability: identifier use and documentation reservations. | Not rated. | Apply relevant registry rules; no new content codec or registration campaign is implied. |
| [9952](../knowledge/rfcs/rfc9952.txt) — CoAP over DTLS ALPN; Info | Reference with committed applicability in the selected DTLS adapter profile. | L0: supported core binding absent. | State ALPN applicability/support in backend profile and qualify it when used. This RFC adds no universal new DTLS ALPN handshake requirement. [#515](https://github.com/jeffglousher/coaptic/issues/515). |

## Security and format dependencies

Each dependency is assessed for its actual use, not as a claim to implement a
general-purpose replacement for the dependency library. Original copies not yet
retained locally must be imported from the official RFC source when their
requirement ledger is created.

| RFC / document state | Commitment and implementation boundary | Baseline | Missing implementation or proof / owner |
| --- | --- | --- | --- |
| [5246](https://www.rfc-editor.org/rfc/rfc5246.html) — TLS 1.2; ST, obsoleted by RFC 8446 | Reference: applicable DTLS 1.2 record/alert rules. | Companion profile evidence only. | Preserve exact failure-stage alert semantics and the unresolved GCM finding; no general core TLS claim. [#486](https://github.com/jeffglousher/coaptic/issues/486), [#199](https://github.com/jeffglousher/coaptic/issues/199). |
| [6347](https://www.rfc-editor.org/rfc/rfc6347.html) — DTLS 1.2; ST, obsoleted by RFC 9147 | Committed: named Rust adapter/identity profiles for the delivered stack. | L2: companion PSK qualification; L0: supported core endpoint binding. | Supported client/server endpoint API, PSK/RPK/X.509 trust profiles, bounds, independent stack proof and exact literal-test dispositions. No obsolete/unsafe suite is required merely by retaining this version. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199), [#200](https://github.com/jeffglousher/coaptic/issues/200). |
| [7250](https://www.rfc-editor.org/rfc/rfc7250.html) — Raw Public Keys; ST | Committed: accepted DTLS RPK profile and base-CoAP security boundary. | L0: accepted RPK path unsupported. | Credential negotiation/trust/identity and actual RPK loss/refusal/recovery proof; X.509 does not substitute. [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199). |
| [9147](https://www.rfc-editor.org/rfc/rfc9147.html) — DTLS 1.3; ST | Deferred commitment: explicit reviewed backend/version profile. | L0. | Version-specific state/record/identity/congestion behavior and independent proof; DTLS 1.2 evidence is not reusable as version compliance. [#515](https://github.com/jeffglousher/coaptic/issues/515). |
| [9200](https://www.rfc-editor.org/rfc/rfc9200.html) — ACE-OAuth; ST | Deferred commitment: authorization-server/resource-server/client wire roles. | L0: ACE; custom policy foundations exist separately. | Named token/profile formats, audience/expiry/current-policy/owner isolation and interoperable endpoints. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
| [9202](https://www.rfc-editor.org/rfc/rfc9202.html) — ACE DTLS Profile; ST, updated by RFC 9430 | Deferred commitment: standardized DTLS authorization profile. | L0. | Applicable update/profile inventory, authz-info/token/bootstrap and policy proof; ordinary DTLS is insufficient. [#365](https://github.com/jeffglousher/coaptic/issues/365), [#515](https://github.com/jeffglousher/coaptic/issues/515). |
| [9528](https://www.rfc-editor.org/rfc/rfc9528.html) — EDHOC; ST | Committed: method 3 and declared credential/application profile, with applicable section 8 requirements. | L2: suite 2/reference credentials and message_4 profile. | Absent a qualifying application profile, section 8 requires kid, exporter, errors1/2, EAD and suites2/3. Single method support is permitted. Qualify the actual profile and complete requirements; no after-the-fact profile invented to disguise gaps. [#365](https://github.com/jeffglousher/coaptic/issues/365). |
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
| [draft-ietf-lake-authz](https://datatracker.ietf.org/doc/draft-ietf-lake-authz/) | Interest; source reference under reconciliation | Draft-linked AccessDenied code/comment is not evidence of an implemented enrollment/authorization protocol. Pin revision and trace externally reachable behavior before claiming support. |

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
| P0 | Unresolved companion GCM structural/authentication finding. | [#486](https://github.com/jeffglousher/coaptic/issues/486): reviewed correction and strict source-bound refusal/recovery; no finding suppression or substitute CCM proof. |
| P1 | Base-RFC delivered security boundary and RPK; EDHOC applicable mandatory requirements. | [#515](https://github.com/jeffglousher/coaptic/issues/515), [#199](https://github.com/jeffglousher/coaptic/issues/199), [#365](https://github.com/jeffglousher/coaptic/issues/365): exact7252/9528/9668 ledgers and supported profiles. |
| P1 | Bounded association/context ownership and fair progress needed by protocol roles. | [#340](https://github.com/jeffglousher/coaptic/issues/340): capacity/epoch/replay/expiry/cancel isolation, checked refusal and valid-peer recovery. |
| P2 | Remaining discovery metadata/parsing, Observe/selection/transfer APIs, overload/backoff and extended tokens. | [#519](https://github.com/jeffglousher/coaptic/issues/519), [#520](https://github.com/jeffglousher/coaptic/issues/520), [#340](https://github.com/jeffglousher/coaptic/issues/340), [#516](https://github.com/jeffglousher/coaptic/issues/516): narrow clause-driven implementation and independent directions. |
| P2 | Missing requirement ledgers and independent evidence for established profiles. | [#200](https://github.com/jeffglousher/coaptic/issues/200), [#202](https://github.com/jeffglousher/coaptic/issues/202): finite requirement inventory; no full-compliance claim from fixture totals. |
| P3 | Deferred reliable sessions/BERT, DTLS 1.3, proxy/cache/HTTP, RD, ACE and base multicast. | [#516](https://github.com/jeffglousher/coaptic/issues/516), [#515](https://github.com/jeffglousher/coaptic/issues/515), [#517](https://github.com/jeffglousher/coaptic/issues/517), [#519](https://github.com/jeffglousher/coaptic/issues/519), [#365](https://github.com/jeffglousher/coaptic/issues/365), [#518](https://github.com/jeffglousher/coaptic/issues/518): consume specific ownership/transport contracts; remain unfinished full-stack commitments. |

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
