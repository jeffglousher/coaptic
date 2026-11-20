# Allocated Service Linux process-clock audit

Linux recorded an additional process CPU clock within the same
measurement gates as the user/system tick counters. The extra clock
shows lower server and client CPU time in all eight rotated pairs in
the 30-second campaign. The 72-second unprotected campaign shows mixed
pair results and wider latency variation within each window. The
original guard decisions stand for both campaigns.

## Recorded clock and verification

The helper binds `clock_getcpuclockid` to the verified native server and
client processes, then uses `clock_gettime_ns` for before/after total
CPU time. It checks the PID, start ticks, process group and clock ID.
Each clock reports 1 ns resolution. It measures all threads in one
process and excludes descendants. The separate 100 Hz `/proc` counters
retain user and system CPU components. Snapshot fences and resolution
account for possible differences between readings. The reported clock
resolution gives no bound on measurement error.

The audit confirms identity and positive deltas in all 64 clock pairs
across 32 windows, divides by raw verified completions, and checks each
input against the existing per-window hash. The largest absolute delta
between clock backends at one process window is 14.1 ms. The longest
process clock snapshot bracket is 210 µs wall time. The alternate CPU
totals are diagnostic. The 20-tick and latency guards continue to
govern #650 comparisons.

## 30-second diagnostic

- Unprotected server clock averages 135.12 to 107.42 µs per verified
  read, a fall of 20.5%. Individual pair decreases are 12.0–25.3%.
  Client clock averages 153.57 to 131.72 µs, a fall of 14.2%.
- Established OSCORE server clock averages 165.82 to 144.66 µs per
  verified read, a fall of 12.8%. Individual pair decreases are
  3.9–23.3%. Client clock averages 185.02 to 166.06 µs, a fall of
  10.2%.
- All latency guards pass. Seven primary counter windows miss the
  accounting floor and both primary CPU ratios stay withheld.

[Short campaign](LINUX-SERVICE-ALLOCATED-C4-CPU-20261120.md) ·
[Short diagnostic receipt](LINUX-ALLOCATED-PROCESS-CLOCK-SHORT-20261120.json)

## 72-second unprotected diagnostic

Server clock averages 260.45 to 237.77 µs per verified read, with
pair changes from −24.2% to +3.4%. Across eight windows, compare the
medians of ten consecutive sets of 360 verified completions per
window. The within-window span is 307–510 µs. In the separate short
campaign, consecutive sets of 300 verified completions span 4–111 µs
within a window. The order records completions, with full four-Call
groups. It carries no per-sample endpoint identity or scheduler data.
Two pair medians and the pooled median exceed the original guard.

The hosted runners differ between campaigns. Use a same-binary rotated
control to test the repeatability of the long comparison at the same
load and capacities. Both #650 modes stay open.

[Long campaign](LINUX-SERVICE-ALLOCATED-C4-LONG-CPU-20261120.md) ·
[Long diagnostic receipt](LINUX-ALLOCATED-PROCESS-CLOCK-LONG-UNPROTECTED-20261120.json)

## 72-second established OSCORE diagnostic

On the separately recorded Intel host, server clock averages 183.08 to
167.69 µs per verified read. All four pair readings fall by 1.5–18.2%.
Client clock averages 214.43 to 214.41 µs, with one pair rising 14.6%.
The within-window spread of 360-completion medians is 18–159 µs.
The independent pair and pooled latency failures withhold the primary
comparison despite the alternate server clock direction.

[Protected diagnostic receipt](LINUX-ALLOCATED-PROCESS-CLOCK-LONG-OSCORE-20261120.json)
