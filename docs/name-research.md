# Constrained CoAP — crate name research

Survey date: 2026-09-02 (America/Chicago). Sources: crates.io API (`/api/v1/crates?q=coap&per_page=50`, plus direct `/api/v1/crates/<name>`), lib.rs search for `coap`, and GitHub/C-ecosystem lookup for well-known CoAP names. crates.io 404 on a name is **likely available** (not a reservation). 200 is taken.

Working folder today: `constrained-coap`. Nickname: Constrained CoAP. Zip: `constrained_coap_tip`.

---

## 1. Ecosystem snapshot (notable crates only)

crates.io search for `coap` returns ~106 hits; many are false positives (`rumqttc` tags `coap` as a keyword). The real Rust CoAP map:

### Full stacks / servers / clients

| Crate | Version | Downloads / last update | What it is | Repo |
|---|---|---|---|---|
| **coap** | 0.28.0 | 571k total, 53k recent; 2026-08-28 CT | The established std CoAP library (UDP/DTLS/Observe/Block). Not a constrained/no-alloc stack. | https://github.com/covertness/coap-rs |
| **coap-lite** | 0.13.3 | 728k total, 96k recent; 2025-05-10 CT | Lightweight **message** crate; `no_std` **but requires a global allocator**. Building block for `coap` and apps. Closest name-collision risk for anything “lite/tiny/embedded”. | https://github.com/martindisch/coap-lite |
| **embedded-nal-coap** | 0.1.0-alpha.6 | 76k; 2026-01-13 CT | no_std client+server on `embedded-nal-async` (chrysn). Allocator-free *stack*, not a slot/pool architecture. | https://codeberg.org/chrysn/embedded-nal-coap |
| **coapcore** | 0.1.1 | 1.8k; 2026-01-29 CT | Ariel OS **security** CoAP stack: OSCORE/EDHOC, server-side. Uses the chrysn `coap-message` traits. Name occupies `coapcore`; `coap-core` is still free but would be confused with this. | https://github.com/ariel-os/ariel-os |
| **coap-zero** | 0.3.0 | 1.7k; 2024-02-16 CT | **Closest conceptual neighbor:** CoAP for `no_std` without alloc. Quiet, one version, Silicon Economy / Open Logistics. | https://git.openlogisticsfoundation.org/silicon-economy/libraries/serum/coap-zero |
| **minicoap** | 0.2.0 | 4.7k; 2025-10-20 CT | Tiny **zero-copy message parser/builder**, not a protocol engine. | https://github.com/jack-weilage/minicoap |
| **toad** / **kwap** | toad 0.19.1 (1.0.0-beta.9); kwap 0.10.0 | 34k / 45k; last **2023-07 / 2022-06** | Heapless / “universal” CoAP (clov-coffee). kwap renamed to toad; appears stalled. | https://github.com/clov-coffee/toad |
| **libcoap-rs** + **libcoap-sys** | 0.2.2 | 6k / 10k; 2023-05 | Idiomatic + raw bindings to C **libcoap**. | https://github.com/namib-project/libcoap-rs |
| **coapum** | 0.2.0 | 446; 2025-08-19 CT | Modern async CoAP with DTLS/observers. Host-side, not constrained. | https://github.com/jaredwolff/coapum |
| **coap-server** | 0.1.1 | 3.9k; 2022-05 | Async server. Stale. | https://github.com/jasta/coap-server-rs |
| **coap-client** | 0.3.1 | 8.3k; 2022-02 | CLI/client. Stale. | https://github.com/ryankurte/rust-coap-client |
| **async-coap** | 0.1.0 | 4.4k; **2019-08** | Google experimental async CoAP. Dead. | https://github.com/google/rust-async-coap |
| **zerodds-coap-bridge** | 1.0.0-rc.6 | 204; 2026-07 | no_std+**alloc** codec/Observe/Block + DDS bridge. | https://github.com/zero-objects/zero-dds |

### chrysn trait ecosystem (interop layer, not a competing engine)

These are the de-facto CoAP *interfaces* in Rust. A new engine can ignore them or implement them later.

| Crate | Version | Downloads / last update | What it is | Repo |
|---|---|---|---|---|
| **coap-message** | 0.3.7 | 783k, 156k recent; 2026-06-11 CT | Traits for readable/writable CoAP messages (no tokens/MIDs; transport-agnostic). | https://codeberg.org/chrysn/coap-message |
| **coap-numbers** | 0.2.10 | 253k; 2026-07-11 CT | Protocol constants. | https://codeberg.org/chrysn/coap-numbers |
| **coap-handler** | 0.2.1 (0.3.0-alpha.1) | 182k; 2025-10-16 CT | Server handler traits. | https://codeberg.org/chrysn/coap-handler |
| **coap-message-utils** / **-implementations** | 0.3.9 / 0.2.0 | 207k / 177k; 2025-10 / 2026-06 | Utilities and concrete message types. | https://codeberg.org/chrysn/coap-tools |
| **coap-request** | 0.1.0 (0.2.0-alpha.2) | 88k; 2024-01 | Request traits. | https://codeberg.org/chrysn/coap-request |
| **windowed-infinity** | 0.2.0 | 151k; 2025-10-09 CT | Ring-window buffer used for Block-wise on constrained devices. | https://codeberg.org/chrysn/windowed-infinity |
| **embedded-nal-minimal-coapserver** | 0.5.0 | 34k; 2025-07 | Minimal CoAP server on embedded-nal. | https://gitlab.com/chrysn/embedded-nal-minimal-coapserver |
| **liboscore** | 0.2.7 | 73k; 2026-03-11 CT | OSCORE (RFC 8613) wrapper. | https://gitlab.com/oscore/liboscore |
| **lakers** | 0.8.0 | 88k; 2025-02 | EDHOC (RFC 9528). | https://github.com/openwsn-berkeley/lakers |

### Names that exist **outside** crates.io (do not reuse)

| Name | Where | Why it matters |
|---|---|---|
| **libcoap** | C; libcoap.net. crates.io `libcoap` is **free**, `libcoap-rs`/`libcoap-sys` taken | Do not publish as `libcoap`. |
| **microcoap** | C; https://github.com/1248/microcoap (MCU server, last push 2018) | crates.io `microcoap` / `micro-coap` are free; still a well-known C name. |
| **nanocoap** / **gcoap** | RIOT-OS C libraries | crates.io `nanocoap`, `nano-coap`, `gcoap` are free; colliding with RIOT would be unwise. |
| **tiny-coap** | C; https://github.com/BlackBrickOrg/tiny-coap | crates.io free; known C client for tiny devices. |

**Gap this project fills:** no current crate is a **bounded, six-area (datagram/body pools + dedup + observe), allocation-independent, client+server, Q-Block-first** engine. `coap-lite` is a message crate that still allocates. `coap-zero` is no-alloc but quiet and not this architecture. `coapcore` is OSCORE/EDHOC (deferred here). `toad`/`kwap` tried heapless CoAP and stalled.

---

## 2. Candidate names — taken vs available

Checked with `GET https://crates.io/api/v1/crates/<name>` (404 = likely available, 200 = taken).

### Requested list

| Name | crates.io | Notes |
|---|---|---|
| constrained-coap | **available** | Matches folder + nickname. Slight tautology (CoAP already = Constrained Application Protocol). |
| coap-constrained | **available** | Same meaning, coap-* prefix. |
| coap-bounded | **available** | Strong architecture signal. |
| bounded-coap | **available** | Same, adjective-first. |
| coap-slots | **available** | Names datagram/body slots. |
| coap-slot | **available** | Weaker singular. |
| coap-pool | **available** | Incomplete (4 pools + 2 tables). |
| coap-static | **available** | Mild “static linking” ambiguity. |
| coap-noalloc | **available** | Sharp contrast vs coap-lite. Token is a bit ugly. |
| noalloc-coap | **available** | Same. |
| coap-bare | **available** | Vague. |
| embedded-coap | **available** | Too close to `embedded-nal-coap`. Too generic. |
| coap-embed | **available** | Awkward. |
| tiny-coap | **available** | Collides with known C project. |
| micro-coap | **available** | Collides with C `microcoap`. |
| nano-coap | **available** | Collides with RIOT nanoCoAP. |
| coap-core | **available** | **Do not use** — `coapcore` is taken (Ariel OS). |
| coapcore | **TAKEN** 0.1.1 | Ariel OS OSCORE/EDHOC stack. |
| coap-progress | **available** | Internal “progress pass” jargon; opaque to outsiders. |
| coap-machine | **available** | Generic FSM. |
| coap-fsm | **available** | Jargon. |
| coap-tip | **available** | Internal zip name; meaningless publicly. |
| ccoap | **available** | Cryptic. |
| bcoap | **available** | Cryptic. |
| slotcoap | **available** | Concatenated; less conventional than `coap-slots`. |
| coap-budget | **available** | Capacity-as-budget; secondary. |
| coap-fixed | **available** | Reads as “bugfix” or “fixed version”. |
| coap-capacity | **available** | Vague. |
| coap-six | **available** | Cute reference to six areas; too cute / opaque. |
| cc-coap | **available** | Cryptic. |
| coap-rt | **available** | Reads as “runtime” or “real-time”; muddy. |
| coap-nostd | **available** | `no_std` is not the differentiator (`coap-lite` is also no_std). |
| static-coap | **available** | Same as `coap-static`. |
| allocless-coap | **available** | Ugly. |
| coap-allocless | **available** | Ugly. |
| coap-arena | **available** | Implies an arena allocator; architecture is *no* runtime allocator. Misleading. |
| datagram-coap | **available** | CoAP is already datagram-oriented; incomplete. |

Also taken (existing ecosystem, not on the candidate list): `coap`, `coap-lite`, `coap-message`, `coap-handler`, `coap-numbers`, `libcoap-rs`, `libcoap-sys`, `coap-zero`, `minicoap`, `kwap`, `toad`, `coapum`, `coap-server`, `coap-client`.

`libcoap`, `microcoap`, `tinycoap`, `nanocoap`, `gcoap` are **free on crates.io** but occupied as C/RIOT names — treat as taken in spirit.

### Extra names invented during the survey (all available)

`heapless-coap`, `coap-heapless`, `coap-pools`, `coap-qblock`, `qblock`, `coap-limits`, `coap-engine`, `coap-stack`, `coap-baremetal`, `coap-minimal`, `coap-tiny`, `coap-nano`. None beat the top recommendations: `heapless-coap` implies the `heapless` crate; `qblock` is a transfer mechanism, not the architecture; `coap-stack`/`coap-engine` are generic.

---

## 3. Ranked recommendations (available on crates.io)

Criteria: unique vs existing CoAP crates; kebab-case, short, not cutesy; signals bounded / no-alloc / constrained; does not collide with `coap-lite` / `coap-message` / `coap` / `libcoap-rs` / `coapcore`.

| Rank | Crate | Repo name | Why |
|---|---|---|---|
| **1** | **coap-bounded** | `coap-bounded` | Best fit. `coap-*` prefix is how Rust CoAP crates are discovered; **bounded** is the architecture’s actual contract (provisioned capacity, no runtime growth) and is not a synonym of “lite”. Does not look like a chrysn trait crate or like `coap-lite`. |
| **2** | **coap-slots** | `coap-slots` | Unique technical signal: RX/TX datagram slots and body slots are the core model. Short, kebab, no existing CoAP crate uses “slot”. Slightly more implementation-shaped than product-shaped. |
| **3** | **constrained-coap** | `constrained-coap` | Already the working folder and nickname. Searchable. Mild tautology with RFC 7252’s name, but it tells a human “this is CoAP *for constrained devices*” rather than another host-side `coap`. Fine as repo even if the crate is `coap-bounded`. |
| **4** | **bounded-coap** | `bounded-coap` | Same meaning as #1, adjective-first (like `heapless`). Stands apart from the `coap-*` cluster. Slightly worse crates.io discoverability when people search `coap`. Use this if you want a product name that is not “another coap-* crate”. |
| **5** | **coap-noalloc** | `coap-noalloc` | Sharpest contrast with `coap-lite` (`no_std` **with** alloc). Honest, but the token is inelegant and “no alloc” is a property, not the architecture (six areas, slots, progress). Better as a crate *keyword* than as the name. |
| **6** | **coap-static** | `coap-static` | Signals pre-provisioned / static storage. Risk: “static” also means linking, lifetimes, or `static` items. Weaker than bounded/slots. |

**Suggested default:** crate `coap-bounded`, GitHub repo `coap-bounded` (or keep repo `constrained-coap` if the current folder identity should stick). Keywords: `coap`, `no-std`, `no-alloc`, `embedded`, `constrained`, `qblock`.

**Do not pick:** `coap-core` (shadows `coapcore`), `embedded-coap` (shadows `embedded-nal-coap`), `tiny-coap` / `micro-coap` / `nano-coap` (C/RIOT), `coap-tip` (internal), `coap-arena` (wrong memory story), `ccoap`/`bcoap`/`cc-coap` (opaque).

---

## 4. GitHub / org names

You do not need a special GitHub *user* or *org* name for the crate to work. crates.io identity is the crate name + crate owners, independent of GitHub login.

- Every recommended crate name is already a good **repo** name (kebab-case, same string). GitHub allows `any-user/coap-bounded`; global uniqueness is only within one user/org.
- An org named `coap`, `libcoap`, `coap-lite`, etc. is likely contested and is unnecessary.
- Keep crate name and repo name the same if possible (`coap-bounded` / `coap-bounded`). If the public repo stays `constrained-coap`, set `repository` in Cargo.toml to that URL and still publish the crate as `coap-bounded` — a mismatch is fine but slightly noisier.
- crates.io 404 is not a hold. Publish (even 0.0.0 / 0.1.0-alpha) when the name is decided if squatting risk matters.

---

## 5. Names to treat as taken in spirit (even if crates.io 404)

`libcoap`, `microcoap`, `micro-coap`, `nanocoap`, `nano-coap`, `tiny-coap`, `tinycoap`, `gcoap`, `coap-core`.
