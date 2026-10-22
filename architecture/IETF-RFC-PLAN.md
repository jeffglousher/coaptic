# Protocol scope

Coaptic's current milestone is a direct UDP client/server library with pairwise
OSCORE, EDHOC and durable local commissioning. It supports fixed storage and
startup-allocated pools, synchronous operation and native async operation.
See [PLAN.md](../PLAN.md) for work order and
[qualification](requirements/VALIDATION-PLAN.md) for the remaining execution.

## Supported profiles

These are selected endpoint roles. They are not claims to implement every role
or requirement in each RFC. The linked ledgers identify the actual APIs, limits
and direct tests.

| Feature | Standards | Requirement ledger |
| --- | --- | --- |
| UDP requests, replies, retransmission, duplicate handling and bounded overload | [7252](../knowledge/rfcs/rfc7252.txt), [8516](../knowledge/rfcs/rfc8516.txt) | [Core profiles](requirements/CORE-PROFILES.md) |
| Discovery, link metadata and URI handling | [6690](../knowledge/rfcs/rfc6690.txt), [9423](../knowledge/rfcs/rfc9423.txt) | [Discovery](requirements/RFC6690.md) |
| Observe, including selected GET/FETCH resources | [7641](../knowledge/rfcs/rfc7641.txt), [8132](../knowledge/rfcs/rfc8132.txt) | [Observe and transfers](requirements/OBSERVE-TRANSFERS.md) |
| Classic Block1/Block2 and unicast Q-Block | [7959](../knowledge/rfcs/rfc7959.txt), [9177](../knowledge/rfcs/rfc9177.txt) | [Observe and transfers](requirements/OBSERVE-TRANSFERS.md) |
| No-Response masks and their supported interactions | [7967](../knowledge/rfcs/rfc7967.txt) | [Observe and transfers](requirements/OBSERVE-TRANSFERS.md) |
| Bounded extended UDP tokens, probing and capacity refusal | [8974](../knowledge/rfcs/rfc8974.txt) | [Core profiles](requirements/CORE-PROFILES.md) |
| Echo, Request-Tag and token processing | [9175](../knowledge/rfcs/rfc9175.txt) | [Core profiles](requirements/CORE-PROFILES.md) |
| Pairwise OSCORE and inner blockwise transfers | [8613](../knowledge/rfcs/rfc8613.txt) | [Security](requirements/SECURITY-PROFILES.md) |
| EDHOC method 3, pinned CCS/kid credentials, suites 2/3 and required message 4 | [9528](../knowledge/rfcs/rfc9528.txt), [9668](../knowledge/rfcs/rfc9668.txt) | [Security](requirements/SECURITY-PROFILES.md), [CoAP mapping](requirements/RFC7252-EDHOC-OPTIONS.md) |
| Bounded CBOR problem details | [9290](../knowledge/rfcs/rfc9290.txt) | [Core profiles](requirements/CORE-PROFILES.md) |

Hop-Limit ([RFC 8768](../knowledge/rfcs/rfc8768.txt)) has an option codec and
5.08 code, not proxy forwarding behavior. Application PATCH/iPATCH payload
formats remain application contracts.

Detailed EDHOC boundaries are in the [EAD](requirements/RFC9528-EAD.md),
[exporter](requirements/RFC9528-EXPORTER.md) and
[error reception](requirements/RFC9528-ERROR-RECEPTION.md) ledgers.

## Provisioning and recovery

First provisioning installs a unique device identity and an authenticated
owner/peer binding with permissions. The default commissioning channel is trusted,
privileged local USB. A device name or first network contact does not establish
trust. Verification, storage or policy failure refuses the transaction.

The guided Rust CLI supports separately provisioned touch-required PIV and
explicit touchless development credentials. PIV touch proves presence, not FIDO2
user verification. The approval credential is distinct from the device identity.

Ordinary durable flash is the default. Restore identity, trust and permissions
after restart, then establish fresh EDHOC/OSCORE sessions without another operator
approval. Reusing OSCORE keys instead requires durable sender reservations and
replay checkpoints. Missing or corrupt history never silently becomes first use.

Hardware rollback protection is optional. The stronger
`OfflineProtectedAnchor` policy requires a separately qualified protected
monotonic authority; ordinary flash counters do not satisfy it.

Revocation takes effect when an authenticated update reaches the enforcing peer.
A disconnected peer cannot know an undelivered update. Applied ownership and
permission changes invalidate affected sessions, grants and queued work.
Preserve unresolved durable effects for authenticated receipt-only recovery.
A CoAP ACK does not commit an application effect.

## Outside this milestone

These are contribution opportunities, not unfinished promises:

- DTLS endpoints: [RFC 6347](https://www.rfc-editor.org/rfc/rfc6347.html),
  [9147](https://www.rfc-editor.org/rfc/rfc9147.html), raw public keys
  [7250](https://www.rfc-editor.org/rfc/rfc7250.html) and ALPN
  [9952](../knowledge/rfcs/rfc9952.txt). Companion DTLS fixtures do not supply a
  supported core binding. OSCORE does not implement RFC 7252 section 9's DTLS
  NoSec/RPK requirements.
- TCP/TLS/WebSockets and negotiated BERT endpoints:
  [RFC 8323](../knowledge/rfcs/rfc8323.txt). Existing BERT codecs are not a
  reliable-session runtime.
- Proxy, gateway/cache and HTTP mapping:
  [RFC 7252](../knowledge/rfcs/rfc7252.txt),
  [8075](../knowledge/rfcs/rfc8075.txt) and proxy Hop-Limit behavior.
- Multicast and group security:
  [RFC 7390](../knowledge/rfcs/rfc7390.txt),
  [9594](https://www.rfc-editor.org/rfc/rfc9594.html) and the group-communication
  drafts below.
- Resource Directory: [RFC 9176](../knowledge/rfcs/rfc9176.txt).
- ACE wire roles and profiles:
  [RFC 9200](https://www.rfc-editor.org/rfc/rfc9200.html),
  [9202](https://www.rfc-editor.org/rfc/rfc9202.html) and
  [9203](../knowledge/rfcs/rfc9203.txt). Local commissioning is not ACE.
- Multipart payload codecs: [RFC 8710](../knowledge/rfcs/rfc8710.txt).
  Opaque byte transport does not implement a payload format.
- RFC 9668's optional combined request profile; the selected EDHOC flow is
  sequential and requires message 4.

Adoption needs an agreed use case, trust model, bounded API and implementation
with independent proof. Discovery and forwarding do not confer authorization;
multicast does not confer group security.

## Standards references

The selected CBOR, COSE and derivation uses depend on
[RFC 8949](https://www.rfc-editor.org/rfc/rfc8949.html),
[9052](https://www.rfc-editor.org/rfc/rfc9052.html),
[9053](https://www.rfc-editor.org/rfc/rfc9053.html) and
[5869](https://www.rfc-editor.org/rfc/rfc5869.html).
[RFC 9876](../knowledge/rfcs/rfc9876.txt) applies to Content-Format identifiers.
[RFC 9178](../knowledge/rfcs/rfc9178.txt) is cellular deployment guidance, not a
delivered cellular profile. Companion DTLS 1.2 evidence also references
[TLS 1.2](https://www.rfc-editor.org/rfc/rfc5246.html), which is obsoleted by
[TLS 1.3](https://www.rfc-editor.org/rfc/rfc8446.html); DTLS 1.2 is obsoleted by
DTLS 1.3. The ACE DTLS profile has the applicable
[RFC 9430](https://www.rfc-editor.org/rfc/rfc9430.html) update.

Unadopted candidates include
[draft-ietf-core-groupcomm-bis](https://datatracker.ietf.org/doc/draft-ietf-core-groupcomm-bis/),
[draft-ietf-core-oscore-groupcomm](https://datatracker.ietf.org/doc/draft-ietf-core-oscore-groupcomm/)
and the exact
[draft-ietf-lake-authz-08](https://datatracker.ietf.org/doc/html/draft-ietf-lake-authz-08)
enrollment proposal. Vendored comments also cite `draft-tiloca-lake-implem-cons`
and the superseded `draft-ietf-cose-rfc8152bis`. A citation or enum value is not
support. Verify exact versions, publication state and applicable final
requirements before adopting a draft or making a new standards claim.

## Implementation assessment

The implemented endpoint profiles are **L3**: usable selected profiles with
direct tests and finite execution evidence, not completed whole-RFC assessment.
Hop-Limit and BERT codecs are **L1**; unsupported core endpoint roles are **L0**.
The companion DTLS PSK fixture is **L2** (partial public API) for that fixture
alone. **L4** would
require reviewed implementation and proof for every applicable requirement;
nothing here claims L4.

Each ledger entry identifies the RFC section, role, normative requirement,
applicability, source/API, tests and remaining counterexamples. Explain SHOULD
deviations and applicable updates/errata. Informational publication status does
not waive applicable requirements. Unsupported features remain explicit.

Evidence names its actual peer, backend, direction and source. Shared-backend
tests do not establish independent cryptographic interoperability. Fixture/RFC
conflicts retain their disposition; failures and required-feature skips are
not passes. Test totals, CI results and unrelated passing requirements cannot
compensate for missing mandatory proof. No certification or aggregate compliance
percentage is claimed.

Original protocol text is in [knowledge/rfcs](../knowledge/rfcs/index.md).
