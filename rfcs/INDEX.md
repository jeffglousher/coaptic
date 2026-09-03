# CoAP / CoRE RFC local copies

Canonical IETF plain-text RFCs (and PDFs) for the constrained CoAP architecture project.
Fetched from [rfc-editor.org](https://www.rfc-editor.org/). Do not treat HTML dumps as source.

Text files: `https://www.rfc-editor.org/rfc/rfcNNNN.txt`
PDFs: `https://www.rfc-editor.org/rfc/rfcNNNN.pdf` when published that way;
pre-RFC 8650 ASCII RFCs used the official `rfcNNNN.txt.pdf` rendering from the RFC Editor rsync tree, saved here as `rfcNNNN.pdf`.

The five documents explicitly cited by `design.md` are marked **core (cited)**.

## Index

| RFC | Title | Date | Status | Role | Files | Why in this tree |
| ---: | --- | --- | --- | --- | --- | --- |
| [6690](rfc6690.txt) | Constrained RESTful Environments (CoRE) Link Format | August 2012 | Proposed Standard | related | `rfc6690.txt` (47,720 B), `rfc6690.pdf` (36,251 B) | CoRE discovery payload and .well-known/core; used by CoAP resource discovery. |
| [7252](rfc7252.txt) | The Constrained Application Protocol (CoAP) | June 2014 | Proposed Standard | core (cited) | `rfc7252.txt` (258,789 B), `rfc7252.pdf` (172,659 B) | Base CoAP protocol. Updated by RFC 7959, 8613, 8974, 9175, and 9876. |
| [7390](rfc7390.txt) | Group Communication for the Constrained Application Protocol (CoAP) | October 2014 | Experimental | related | `rfc7390.txt` (106,675 B), `rfc7390.pdf` (71,026 B) | Multicast / group CoAP; still the published group-comm spec (bis is not yet an RFC). |
| [7641](rfc7641.txt) | Observing Resources in the Constrained Application Protocol (CoAP) | September 2015 | Proposed Standard | core (cited) | `rfc7641.txt` (65,842 B), `rfc7641.pdf` (43,711 B) | Observe option and notifications. Updated by RFC 8323. |
| [7959](rfc7959.txt) | Block-Wise Transfers in the Constrained Application Protocol (CoAP) | August 2016 | Proposed Standard | core (cited) | `rfc7959.txt` (87,515 B), `rfc7959.pdf` (55,916 B) | Classic Block1/Block2 transfers. Updates RFC 7252; updated by RFC 8323. |
| [7967](rfc7967.txt) | Constrained Application Protocol (CoAP) Option for No Server Response | August 2016 | Informational | related | `rfc7967.txt` (40,314 B), `rfc7967.pdf` (28,749 B) | No-Response option; used with NON/group/OSCORE to suppress unwanted replies. |
| [8075](rfc8075.txt) | Guidelines for Mapping Implementations: HTTP to the Constrained Application Protocol (CoAP) | February 2017 | Proposed Standard | related | `rfc8075.txt` (86,096 B), `rfc8075.pdf` (59,793 B) | HTTP-to-CoAP cross-proxy mapping (methods, status, URI, media types). |
| [8132](rfc8132.txt) | PATCH and FETCH Methods for the Constrained Application Protocol (CoAP) | April 2017 | Proposed Standard | related | `rfc8132.txt` (42,359 B), `rfc8132.pdf` (31,164 B) | Adds FETCH and PATCH (and iPATCH) methods to CoAP. |
| [8323](rfc8323.txt) | CoAP (Constrained Application Protocol) over TCP, TLS, and WebSockets | February 2018 | Proposed Standard | related | `rfc8323.txt` (110,771 B), `rfc8323.pdf` (75,562 B) | Reliable-transport CoAP, including Ping/Pong signaling (7.02/7.03). Updates RFC 7641 and 7959; updated by RFC 8974. |
| [8516](rfc8516.txt) | "Too Many Requests" Response Code for the Constrained Application Protocol | January 2019 | Proposed Standard | related | `rfc8516.txt` (11,786 B), `rfc8516.pdf` (17,571 B) | CoAP 4.29 Too Many Requests; overload / rate-limit signaling. |
| [8613](rfc8613.txt) | Object Security for Constrained RESTful Environments (OSCORE) | July 2019 | Proposed Standard | related | `rfc8613.txt` (203,804 B), `rfc8613.pdf` (143,556 B) | Application-layer CoAP object security. Updates RFC 7252. |
| [8710](rfc8710.txt) | Multipart Content-Format for the Constrained Application Protocol (CoAP) | February 2020 | Proposed Standard | related | `rfc8710.txt` (19,046 B), `rfc8710.pdf` (162,305 B) | CoAP multipart content-format for combining several representations. |
| [8768](rfc8768.txt) | Constrained Application Protocol (CoAP) Hop-Limit Option | March 2020 | Proposed Standard | related | `rfc8768.txt` (16,860 B), `rfc8768.pdf` (151,199 B) | Hop-Limit option to bound proxy forwarding loops. |
| [8974](rfc8974.txt) | Extended Tokens and Stateless Clients in the Constrained Application Protocol (CoAP) | January 2021 | Proposed Standard | related | `rfc8974.txt` (46,951 B), `rfc8974.pdf` (242,796 B) | Extended Token length (TKL). Updates RFC 7252 and 8323. |
| [9175](rfc9175.txt) | Constrained Application Protocol (CoAP): Echo, Request-Tag, and Token Processing | February 2022 | Proposed Standard | core (cited) | `rfc9175.txt` (74,120 B), `rfc9175.pdf` (366,012 B) | Echo and Request-Tag options, plus Token processing. Updates RFC 7252. |
| [9176](rfc9176.txt) | Constrained RESTful Environments (CoRE) Resource Directory | April 2022 | Proposed Standard | related | `rfc9176.txt` (149,556 B), `rfc9176.pdf` (702,392 B) | Resource Directory for registering and looking up CoRE links. |
| [9177](rfc9177.txt) | Constrained Application Protocol (CoAP) Block-Wise Transfer Options Supporting Robust Transmission | March 2022 | Proposed Standard | core (cited) | `rfc9177.txt` (99,517 B), `rfc9177.pdf` (507,498 B) | Q-Block1/Q-Block2 for efficient large-body transfer (preferred by design.md). |
| [9178](rfc9178.txt) | Building Power-Efficient Constrained Application Protocol (CoAP) Devices for Cellular Networks | May 2022 | Informational | related | `rfc9178.txt` (37,776 B), `rfc9178.pdf` (179,186 B) | LWIG guidance for sleepy/cellular CoAP devices (title verified as requested). |
| [9203](rfc9203.txt) | The Object Security for Constrained RESTful Environments (OSCORE) Profile of the Authentication and Authorization for Constrained Environments (ACE) Framework | August 2022 | Proposed Standard | related | `rfc9203.txt` (72,611 B), `rfc9203.pdf` (369,187 B) | ACE coap_oscore profile: OSCORE security context from ACE access tokens. |
| [9290](rfc9290.txt) | Concise Problem Details for Constrained Application Protocol (CoAP) APIs | October 2022 | Proposed Standard | related | `rfc9290.txt` (47,301 B), `rfc9290.pdf` (271,196 B) | CBOR problem-details payload and CoAP Content-Format for API errors. |
| [9423](rfc9423.txt) | Constrained RESTful Environments (CoRE) Target Attributes Registry | April 2024 | Informational | related | `rfc9423.txt` (16,391 B), `rfc9423.pdf` (143,276 B) | IANA registry for CoRE link-format target attributes used in discovery. |
| [9668](rfc9668.txt) | Using Ephemeral Diffie-Hellman Over COSE (EDHOC) with the Constrained Application Protocol (CoAP) and Object Security for Constrained RESTful Environments (OSCORE) | November 2024 | Proposed Standard | related | `rfc9668.txt` (60,218 B), `rfc9668.pdf` (299,036 B) | EDHOC over CoAP and combined EDHOC+OSCORE to establish an OSCORE context. |
| [9876](rfc9876.txt) | Updates to the IANA Registration Procedures for Constrained Application Protocol (CoAP) Content-Formats | November 2025 | Proposed Standard | related | `rfc9876.txt` (29,129 B), `rfc9876.pdf` (130,564 B) | Updates RFC 7252 IANA CoAP Content-Formats registration procedures. |
| [9952](rfc9952.txt) | Application-Layer Protocol Negotiation (ALPN) ID for CoAP over DTLS | March 2026 | Informational | related | `rfc9952.txt` (9,065 B), `rfc9952.pdf` (71,039 B) | ALPN identifier for CoAP over DTLS (coap). |

## Core (cited by design.md)

- **RFC 7252** (June 2014) — The Constrained Application Protocol (CoAP)
  - Base CoAP protocol. Updated by RFC 7959, 8613, 8974, 9175, and 9876.
  - [`rfc7252.txt`](rfc7252.txt), [`rfc7252.pdf`](rfc7252.pdf)
- **RFC 7641** (September 2015) — Observing Resources in the Constrained Application Protocol (CoAP)
  - Observe option and notifications. Updated by RFC 8323.
  - [`rfc7641.txt`](rfc7641.txt), [`rfc7641.pdf`](rfc7641.pdf)
- **RFC 7959** (August 2016) — Block-Wise Transfers in the Constrained Application Protocol (CoAP)
  - Classic Block1/Block2 transfers. Updates RFC 7252; updated by RFC 8323.
  - [`rfc7959.txt`](rfc7959.txt), [`rfc7959.pdf`](rfc7959.pdf)
- **RFC 9175** (February 2022) — Constrained Application Protocol (CoAP): Echo, Request-Tag, and Token Processing
  - Echo and Request-Tag options, plus Token processing. Updates RFC 7252.
  - [`rfc9175.txt`](rfc9175.txt), [`rfc9175.pdf`](rfc9175.pdf)
- **RFC 9177** (March 2022) — Constrained Application Protocol (CoAP) Block-Wise Transfer Options Supporting Robust Transmission
  - Q-Block1/Q-Block2 for efficient large-body transfer (preferred by design.md).
  - [`rfc9177.txt`](rfc9177.txt), [`rfc9177.pdf`](rfc9177.pdf)

## Related CoAP / CoRE protocol machinery

- **RFC 6690** (August 2012, Proposed Standard) — Constrained RESTful Environments (CoRE) Link Format
- **RFC 7390** (October 2014, Experimental) — Group Communication for the Constrained Application Protocol (CoAP)
- **RFC 7967** (August 2016, Informational) — Constrained Application Protocol (CoAP) Option for No Server Response
- **RFC 8075** (February 2017, Proposed Standard) — Guidelines for Mapping Implementations: HTTP to the Constrained Application Protocol (CoAP)
- **RFC 8132** (April 2017, Proposed Standard) — PATCH and FETCH Methods for the Constrained Application Protocol (CoAP)
- **RFC 8323** (February 2018, Proposed Standard) — CoAP (Constrained Application Protocol) over TCP, TLS, and WebSockets
- **RFC 8516** (January 2019, Proposed Standard) — "Too Many Requests" Response Code for the Constrained Application Protocol
- **RFC 8613** (July 2019, Proposed Standard) — Object Security for Constrained RESTful Environments (OSCORE)
- **RFC 8710** (February 2020, Proposed Standard) — Multipart Content-Format for the Constrained Application Protocol (CoAP)
- **RFC 8768** (March 2020, Proposed Standard) — Constrained Application Protocol (CoAP) Hop-Limit Option
- **RFC 8974** (January 2021, Proposed Standard) — Extended Tokens and Stateless Clients in the Constrained Application Protocol (CoAP)
- **RFC 9176** (April 2022, Proposed Standard) — Constrained RESTful Environments (CoRE) Resource Directory
- **RFC 9178** (May 2022, Informational) — Building Power-Efficient Constrained Application Protocol (CoAP) Devices for Cellular Networks
- **RFC 9203** (August 2022, Proposed Standard) — The Object Security for Constrained RESTful Environments (OSCORE) Profile of the Authentication and Authorization for Constrained Environments (ACE) Framework
- **RFC 9290** (October 2022, Proposed Standard) — Concise Problem Details for Constrained Application Protocol (CoAP) APIs
- **RFC 9423** (April 2024, Informational) — Constrained RESTful Environments (CoRE) Target Attributes Registry
- **RFC 9668** (November 2024, Proposed Standard) — Using Ephemeral Diffie-Hellman Over COSE (EDHOC) with the Constrained Application Protocol (CoAP) and Object Security for Constrained RESTful Environments (OSCORE)
- **RFC 9876** (November 2025, Proposed Standard) — Updates to the IANA Registration Procedures for Constrained Application Protocol (CoAP) Content-Formats
- **RFC 9952** (March 2026, Informational) — Application-Layer Protocol Negotiation (ALPN) ID for CoAP over DTLS

## Selection notes

- **RFC 7252 updates / updated-by:** this tree includes the documents that update 7252 (RFC 7959, 8613, 8974, 9175, 9876) and the documents that update 7641 and 7959 (RFC 8323; RFC 8974 also updates 8323).
- **RFC 9178** title is correct: *Building Power-Efficient Constrained Application Protocol (CoAP) Devices for Cellular Networks* (Informational, May 2022).
- **RFC 9200 (ACE-OAuth)** was skipped: it is the OAuth 2.0 authorization framework for constrained environments, not CoAP protocol machinery. The CoAP-relevant OSCORE profile is **RFC 9203**.
- **Congestion control / ping-pong:** there is no published CoAP-specific congestion-control RFC (CoCoA and FASOR remain expired I-Ds). Basic CoAP congestion control is in RFC 7252 §4.7. CoAP Ping/Pong signaling codes 7.02/7.03 are in **RFC 8323**. UDP usage guidelines that CoAP cites (BCP 145 / RFC 8085) are not CoAP protocol documents and were not copied.
- **Skipped CoRE WG payload/application RFCs** (not CoAP protocol machinery): SenML (RFC 8428, 8790, 8798, 9100, 9193), YANG-CBOR / SID (RFC 9254, 9595, 9997), device-identifier URNs (RFC 9039), DNS over CoAP (RFC 9953).
- **Not yet RFCs** (CoRE WG I-Ds / RFC Editor queue as of 2026-09-02): Group Communication bis, Group OSCORE, CoAP pub/sub, Constrained Resource Identifiers, CoAP corrections/clarifications. Those were not downloaded.

## Verification

Every `rfcNNNN.txt` in this directory is non-empty and contains an IETF RFC header (`Internet Engineering Task Force` / `Request for Comments:`). Every `rfcNNNN.pdf` starts with `%PDF-`.

