---
type: Reference
title: Vendored cabo/td-coap4 README
description: "Attribution for the CoAP#4 YAML sources. Not a test-case rewrite."
tags: [plugtest, attribution]
status: stable
sources:
  - id: td-coap4
    resource: "https://github.com/cabo/td-coap4"
    title: cabo/td-coap4
    retrieved: "2026-09-03T00:00:00Z"
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# Vendored: ETSI CoAP#4 Test Descriptions (YAML sources)

Upstream: [cabo/td-coap4](https://github.com/cabo/td-coap4) (Carsten Bormann).  
Retrieved: 2026-09-03 from `https://raw.githubusercontent.com/cabo/td-coap4/master/`.

This directory copies the **YAML sources only**. Generated HTML (`*.html`) is not vendored; open the upstream repo or regenerate if you need HTML.

These files are reference artifacts, not OKF concepts. The OKF wrappers live beside them in [`../`](../).

Upstream published no license file. Treat this as a local copy of public ETSI plugtest TDs for interop work on coaptic.

---

td-coap4
========

This repo contains the Test Descriptions for ETSI plugtest CoAP#4.

For each area of testing, there is a pair of files:

| Area | source file | html version |
| --- | --- | --- |
| Base CoAP | [base.yml](base.yml) (upstream [base.yml](https://github.com/cabo/td-coap4/blob/master/base.yml)) | [base.html](https://github.com/cabo/td-coap4/blob/master/base.html) (not copied) |
| Block and Observe | [block.yml](block.yml) | [block.html](https://github.com/cabo/td-coap4/blob/master/block.html) (not copied) |
| Link format | [link.yml](link.yml) | [link.html](https://github.com/cabo/td-coap4/blob/master/link.html) (not copied) |
| DTLS | [dtls.yml](dtls.yml) | [dtls.html](https://github.com/cabo/td-coap4/blob/master/dtls.html) (not copied) |
| 6LoWPAN | [6lowpan.yml](6lowpan.yml) | [6lowpan.html](https://github.com/cabo/td-coap4/blob/master/6lowpan.html) (not copied) |

In the test descriptions, the following shorthands are used for the references:

| shorthand | document |
| --- | --- |
| \[COAP] | [draft-ietf-core-coap-18](https://tools.ietf.org/html/draft-ietf-core-coap-18.txt) (now [RFC 7252](../../rfcs/rfc7252.md)) |
| \[OBSERVE] | [draft-ietf-core-observe-12](https://tools.ietf.org/html/draft-ietf-core-observe-12.txt) (now [RFC 7641](../../rfcs/rfc7641.md)) |
| \[BLOCK] | [draft-ietf-core-block-14](https://tools.ietf.org/html/draft-ietf-core-block-14.txt) (now [RFC 7959](../../rfcs/rfc7959.md)) |
| \[LINK] | [RFC 6690](https://tools.ietf.org/html/rfc6690.txt) (CoRE link format; local [RFC 6690](../../rfcs/rfc6690.md)) |
| \[ND] | [RFC 6775](https://tools.ietf.org/html/rfc6775.txt) (6LoWPAN ND) |
| \[IPHC] | [RFC 6282](https://tools.ietf.org/html/rfc6282.txt) (6LoWPAN HC) |
| \[FORMAT] | [RFC 4944](https://tools.ietf.org/html/rfc4944.txt) (6LoWPAN base format) |

The following parameters have not yet been assigned by IANA but are
needed for the DTLS tests:

* 0xC0 0xAC as the cipher suite identifier for TLS_ECDHE_ECDSA_WITH_AES_128_CCM_8

Note that, for RPK, we now have the [final assignments](https://www.iana.org/assignments/tls-extensiontype-values/tls-extensiontype-values.xhtml):

* client_certificate_type: 19
* server_certificate_type: 20
