# Current-source ESP32-S3 Wi-Fi reliability

This finite qualification uses production core `b875088`, with documentation
checkpoint `48e513e71bdb81020b6d75231325ceacfce3d0a8`, and companion
`1de100ccca190e2f660810b24b0aac91ed7a0dfe` (tree equals accepted `4190a89`).
It is a functional device campaign, not a speed comparison or certification.
The prior software recovery campaign remains separate evidence.

## Source and platform

The existing companion core junction was preserved. Process-local Cargo paths
selected this checkout; Cargo output and resolved metadata identify that actual
core and AES `0.9.4`. The locked optimized Xtensa static archive enables
`async-network,oscore,standalone`. A sync image needs a separately compiled
`network,oscore,standalone` archive. The Rust feature selects the device path;
the matching C++ flag selects platform initialization and worker reservation.
The async archive SHA-256 is
`ad3a7a5f27d5ee20f764727510f67cbdf43fd9c4d123036634eec3410f6a45f9`.
The separately compiled sync archive SHA-256 is
`440f1ceafb1bc44c5f7f7cf8901a2eb3d2142f5d862aa00e435c19366f88d56a`.
The device compiler is Rust 1.97.0-nightly (`8ea53bcd7`, 2026-07-08), target
`xtensa-esp32s3-none-elf`; ESPHome is 2026.9.1 with ESP-IDF 5.5.5.
The existing fixture explicitly selects a 16 KiB instruction cache.

The current-source Windows host client enables `network,oscore,host,async-network`,
compiled with Rust 1.99.1 (`58cf8f9c3`). Its SHA-256 is
`5586fc9414eb65d8afb9f92e5511a86e4b7be27b1f6ebd6e4933961f3d515b3d`.
Host client mode does not establish device mode. The independent unprotected
client uses aiocoap 0.4.16.

The fixed Constrained service retains two 1,152-byte RX/TX datagrams, one
4,096-byte body in each direction, four duplicate entries, two Observe slots,
four Calls, four security request bindings, one association, no deferred jobs
and an eight-entry route table (seven used,
including discovery). The existing FreeRTOS worker priority is one; its stack
reservation is 98,304 bytes sync or 196,608 bytes async. These reservations are
not a proof of minimum stack requirements. No automatic pool growth is enabled.

## Finite matrix and observations

An initial fixture assembly incorrectly paired the async archive with sync C++
configuration. Two attempts refused source-bound readiness; normal firmware was
restored after each. These are retained fixture failures, not passing sync
qualification or evidence of a defect in a correctly assembled core/runtime pair.
Subsequent assembly binds the archive feature selection to the C++ mode.

The sync and async unprotected devices each passed 24 complete current-source Coaptic GETs
(eight each identity, small and 2,000-byte Block2), followed by all nine existing
independent-client cases: firmware identity, NON GET, binary echo/size refusal,
1,792-byte Block1 upload, 2,000-byte Block2 download, discovery/errors,
three Observe cancellation/reuse rounds, malformed/oversized recovery and ten
repeated GETs. Factory app bytes matched the installed OTA at offset 65,536.
Each expected run ID was checked on a live CoAP response. Encrypted API captures
retain RSSI and resource observations. The corrected sync path also used an exact
CoAP identity probe for readiness.

Normal authorized firmware was restored and its expected identity verified after
the async cell. OTA changes only the application image; no erase, provisioning,
credential reset or security-history rollback is part of this procedure.

The sync and async protected devices each passed the existing nine-case driver: protected
firmware identity, complete 2,000-byte Block2, 128-byte echo, 129-byte refusal,
plaintext/wrong-key/tamper/replay refusal and fresh valid recovery. It then passed
24 protected timed reads. The negative/refusal driver uses the sync host App;
the timing client uses async host I/O. These client modes are distinct from the
two compiled device modes.
The original exclusive host security record and matching device credentials were
recovered without provisioning. Legitimate sender/replay-state advancement was
preserved through normal-firmware restoration, which was verified again.

All four valid device cells passed: 96 complete timed reads plus two nine-case
independent unprotected campaigns and two nine-case protected campaigns. Normal
firmware was restored and verified after every cell. No reproduced protocol
defect required a core change. The [numeric receipt](ESP-WIFI-RELIABILITY-20261117.json)
contains per-image identities, archive features, cases, resource samples and
restoration results; its exporter requires a successful matching trial and
complete source-bound client/capture evidence for every cell.

## Limits and next measurement

The installed campaign has no Q-Block2 suffix-loss selector, duplicate-effect
counter or explicit pool-exhaustion/reuse control. The passing size refusal and
Observe reuse cases do not establish those different behaviors. Existing sync/
async unprotected/OSCORE software blackout tests and Q-Block2 regression tests
remain software evidence. Physical Wi-Fi interruption was not injected; the
shared AP was not changed. Cold power recovery (#333), deferred credential-touch
recovery (#365) and reverse LAN initiation remain separate prerequisites.

API connections/disconnections describe the observer, not radio retry counts.
The filtered observer does not retain all Wi-Fi retry/disconnect messages, so
zero radio retries or disconnects cannot be inferred. Finite stack watermarks
and resource snapshots cover executed paths only.
RSSI samples across the four cells were -45 to -41 dBm, with no unexpected API
disconnects or observed service-failure messages. Protected resource snapshots
precede the short traffic campaigns (their counters are zero); they do not
establish protected traffic-period stack or heap peaks. The unprotected cells
include traffic-period samples. An async-unprotected SDK snapshot was not retained;
its SDK build log, configuration, image and source identities were retained.

The async unprotected host trace totals 328.9204 ms wall time for 24 operations,
including 327.1083 ms waiting and 0.6008 ms in active host App polling. These
categories describe host work; the receive await includes radio, device,
network and scheduling latency. It is not isolated device CPU. There were 32
send events for the expected 32 request/block exchanges, without extra sends in
this read cohort. The independent malformed campaign has intentional extra input.

The next bounded performance assignment is to measure current-device task CPU
and socket-ready-to-send time for the same small/large reads, separating
preemption/waiting from active processing. Pair that evidence with the existing
host await trace before selecting an optimization. The present data cannot
attribute the receive wait to the protocol core, scheduler or radio. Historical
timer/cache candidates remain rejected; no new performance gain is claimed.

Private firmware, credential-bearing configs and wire captures remain outside
the repository. The private batch receipt binds source, archive/image hashes,
live identities, command outputs, resource samples and restoration results.
