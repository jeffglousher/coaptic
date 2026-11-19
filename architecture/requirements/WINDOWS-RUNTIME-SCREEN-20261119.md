# Windows runtime and peer screening

The November 19 screen found a repeatable difference between the direct Mio and
Tokio fixtures on Windows. Moving from async App progress to sync App polling
inside Tokio left most of that difference in place. Staging a receive before
polling did not yield a repeatable improvement across both runtimes and security
modes. The measurements support investigating the host I/O and scheduling path
before changing shared App dispatch. The Windows screen also re-establishes a
small current-source libcoap comparison. Broader speed and CPU goals remain open.

## Source and scope

The release fixture uses core `73093d875e089f10d43530e5ebef9895a75bf4e3`
(production Rust `b875088`), companion `76be9a0`, Rust 1.99.1 MSVC, thin LTO and
one codegen unit. The benchmark dependency was explicitly bound to that core in
a private manifest overlay. Cargo metadata resolved both `coaptic` and
`coaptic-io` to the intended source. The locked input matches the companion's
committed Syn 3 lock byte for byte. The ordinary companion junction points to an
older checkout. The runner's checkout observation does not establish binary
provenance, which the build and binary hashes supply separately. The numeric
[receipt](WINDOWS-RUNTIME-SCREEN-20261119.json) binds the binaries, driver sources,
operation counts, pacing, distributions and resource ledgers to individual
windows. The initial cycle pilot used a private sampler. The final live control
and runtime comparisons use the opt-in sampler in companion
[#266](https://github.com/jeffglousher/coaptic-validation/pull/266).

All cells run serially on Windows loopback. A frozen native client drives the
CPU observations and verifies the complete representation, status, format and
ETag. OSCORE uses fresh fixture credentials and established contexts. Tests
use one outstanding CON GET except for the explicitly concurrent control below.
Device-durable security and EDHOC establishment need separate measurements.

## Pacing and counter repeatability

Two reversed passes use the fixed SmallDevice blockwise profile, 64 and
4,096-byte bodies, sync Mio and async Tokio, and both security modes. The profile
has four route slots (one populated), one Call, zero deferred jobs, four security
bindings, one association, two RX and two TX datagrams of 1,152 bytes, four dedup
and two Observe entries, and one 4,096-byte body per direction.

- 16 windows at 3,000/s for 64 bytes and 2,000/s for 4,096 bytes complete all
  160,000 operations. 38,321 starts miss at least one period despite achieving
  the average rates. Catch-up changes spacing and weakens a matched-load claim.
- 16 control windows at 800/s complete all 76,800 operations with 32 missed
  periods. Process CPU time still varies between repeats. These windows use a
  different offered load and remain separate from the first cohort.
- Eight 64-byte, 800/s pilot windows complete all 38,400 operations. The paired
  server cycle readings differ by at most 4.64% relative to the lower reading.
  One time pair differs by 2.8×. The opt-in diagnostic retains both samples
  at the existing gates and separates server from client. Windows reports
  elapsed process CPU and process cycles through different counters. Cycle
  readings depend on runtime, hardware, frequency and scheduling. They cannot
  be converted to CPU time or used in place of the original #650 gates.

## Runtime boundary and staging control

The 64-byte host-control profile allows all three runtime arms at the same
capacity and load. It uses one configured route, four Calls, zero deferred jobs,
four security bindings, one association, four RX and four TX datagrams of 1,472
bytes, eight dedup and four Observe entries, and one RX and two TX body slots of
1,024 bytes. Both Tokio arms share the allocated App, current-thread runtime,
socket and scratch type. The sync arm waits for readability around App polling.
The async arm awaits `App::wait_async` and uses `progress_async`. Mio runs its
own polling path. The observations include all these differences.

Across 12 windows, all 28,800 operations complete and eight starts miss a period.
The paired server cycle observations per verified operation are:

- Unprotected: 80,204–96,295 with Mio, 131,025–137,675 with Tokio and sync App
  polling, and 144,494–144,556 with Tokio and async App progress.
- OSCORE: 105,286–105,435 with Mio, 139,035–146,954 with Tokio and sync App
  polling, and 149,247–149,834 with Tokio and async App progress.

Two reversed control passes compare Mio with staged Mio and Tokio sync polling
with staged Tokio sync polling. All 38,400 operations complete across 16
windows and 18 starts miss a period. Neither staged arm improves every matched
pair. The unprotected staged arms both use more cycles in both repeats.
The protected staged Mio pair changes direction across repeats. Staging uses the
existing scratch but adds a retained ingress owner and changes when App polling
occurs. Preserve these as adapter controls and leave production selection alone.
Neither screen establishes a component-level CPU attribution or a qualifying
cycle or latency gain.

## Concurrent completion control

Two reversed passes use two independent local endpoints for CON, keeping one
outstanding Call per endpoint. Unpaced, closed-loop 64 and 4,096-byte GETs use
the fixed Device profile with eight route slots, four Calls, two deferred jobs,
four security bindings, one association, four datagrams of 1,472 bytes per
direction, eight dedup and four Observe entries, and two 4,096-byte bodies per
direction. The profile differs from the paced host-control. Compare only within
the concurrent cohort.

All 51,200 reads in 16 windows complete. Every window records peak pending two
and verified overlapping completions on both endpoints. Sync Mio uses fewer
server cycles in seven of eight paired comparisons. The unprotected 4,096-byte
pair reverses between repeats. The 25–192 ms windows are short, induced by a
closed-loop client with differing achieved rates. Keep both repeats. These
windows establish bounded concurrent completion and a lead for a longer
matched-rate experiment. Saturation capacity and overload recovery require
separate checks.

## Independent libcoap screen

Two rotated windows use the companion's independent Python driver, the frozen
Coaptic server, and the retained libcoap binary identified by the
[October build receipt](https://github.com/jeffglousher/coaptic-validation/blob/76be9a0/tools/benchmark/evidence/current-windows-20261023/build.json).
All 4,608 complete reads in 24 cells verify with zero request and warmup failures.
For 64 and 4,096 bytes in OSCORE, both Coaptic arms have higher closed-loop
completion rates in both repeats. The Coaptic/libcoap rate ratios range from
1.36 to 1.55 across those cells. Unprotected ratios range from 0.85 to 1.21 and
change direction within some cells. These finite results include the common
client and both endpoints. Preserve both repeats and keep their scope separate
from CPU attribution, loaded or concurrent service, and device Wi-Fi.

## Verification and next decision

The opt-in sampler passes 67 focused Python tests and 6,400 live reads across
both runtime and security modes. A live 32-read control retains the ordinary
report shape with the option disabled. Both blackout tests and 51 existing
capacity and blockwise integration tests pass. Blackout tests exercise four
sync/async and unprotected/OSCORE software cells. The physical restoration and
pool evidence remain at their own recorded source and scope.

Next, attribute runtime costs with matched `ready` and `async` App execution
before choosing an adapter change. Use repeated well-paced windows and a scoped
CPU or syscall trace, with both modes, both protections, small and blockwise
bodies, and longer runs before an adoption decision. Preserve duplicate replay,
Call cancellation and expiry, body and security-binding refusal and reuse, and
whole-object limits. Device CPU, device peer speed, and #650's original source
and 50% targets require their own suitable evidence. Keep the remaining
installed-device pools, reverse LAN and physical prerequisites open.

For a finite repeat, use the recorded frozen binaries and invoke companion
`native_diagnostic.py` with `--cpu-gated --windows-process-cycles --rate 800
--policy spin`. Select `--resource-profile small-blockwise` for the fixed-profile
run or `host-control` for the runtime and staging controls. Select the server kind,
security, payload size, operations and warmup from the corresponding receipt
row. Reuse one client across serial cells and reverse the order for the second
pass. For the concurrent control, omit `--rate` and select `--resource-profile
device-fixed --concurrency 2 --independent-endpoints`. Full raw windows remain
in the retained local evidence.
