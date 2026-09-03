# Plugtest

ETSI CoAP plugtest sources and the coaptic mapping. Do not invent TD identifiers. Sequences stay in the vendored YAML and PDFs.

# Requirements

* [coaptic plugtest requirements](requirements.md) - In-scope vs deferred suites; six-area engine constraint.

# In-scope (CoAP#4)

* [Base](base.md) - `TD_COAP_CORE_*` from [td-coap4/base.yml](td-coap4/base.yml). [RFC 7252](/rfcs/rfc7252.md).
* [Block and Observe](block.md) - `TD_COAP_BLOCK_*` and `TD_COAP_OBS_*` from [td-coap4/block.yml](td-coap4/block.yml). [RFC 7959](/rfcs/rfc7959.md), [RFC 7641](/rfcs/rfc7641.md).
* [Link format](link.md) - `TD_COAP_LINK_*` from [td-coap4/link.yml](td-coap4/link.yml). [RFC 6690](/rfcs/rfc6690.md).

# Deferred

* [DTLS](dtls.md) - `TD_COAP_DTLS_*`. Not required until security work.
* [6LoWPAN](6lowpan.md) - `TD_6LoWPAN_*`. Not required until security / 6LoWPAN work.

# Artifacts

* [td-coap4/](td-coap4/) - YAML sources from [cabo/td-coap4](https://github.com/cabo/td-coap4), retrieved 2026-09-03. HTML not copied. Attribution: [td-coap4 README](td-coap4/README.md).
* [etsi/CoAP_2_Plugtests_TestDescriptions_v013.pdf](etsi/CoAP_2_Plugtests_TestDescriptions_v013.pdf) - ETSI CoAP#2 TDs, retrieved 2026-09-03 from [portal.etsi.org](https://portal.etsi.org/cti/downloads/TestSpecifications/CoAP_2_Plugtests_TestDescriptions_v013.pdf) (HTTP 200).
* [etsi/IoT_CoAP_3_Plugtests_TestDescriptions_v005.pdf](etsi/IoT_CoAP_3_Plugtests_TestDescriptions_v005.pdf) - ETSI CoAP#3 TDs, retrieved 2026-09-03 from [portal.etsi.org](https://portal.etsi.org/cti/downloads/TestSpecifications/IoT_CoAP_3_Plugtests_TestDescriptions_v005.pdf) (HTTP 200).

Canonical machine-readable TDs for this repo are CoAP#4 YAML. The PDFs are earlier-event reference.

# Out of scope

* ETSI TS 103 104 (M2M binding) — not this engine.
* Invented TD numbers for [RFC 9175](/rfcs/rfc9175.md) or [RFC 9177](/rfcs/rfc9177.md). Those RFCs have no ETSI CoAP#4 TDs.
