//! Optional request-path work accounting, enabled by `diagnostics`.
//!
//! These counters record logical bytes at named copy and encoding sites, not
//! CPU instructions, elapsed time, allocator activity, or transport-internal
//! copies. Counters wrap; snapshots contain no payloads or peer identifiers.
//! The default build contains neither this structure nor its increments.

/// Clock-free work counters on [`super::Engine`], occupying 112 bytes.
///
/// Use deltas between snapshots after warm-up. Instrumented timings must be
/// reported separately from feature-disabled black-box benchmarks. Custom
/// storage implementations may perform additional copies internally.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkMetrics {
    /// App polling calls, including idle polls and failures.
    pub polls: u64,
    /// Engine receive calls, including saturation before transport access.
    pub recv_attempts: u64,
    /// Receive calls for which the transport returned no datagram.
    pub recv_idle: u64,
    /// Complete accepted datagram bytes, excluding transport staging.
    pub rx_wire_bytes: u64,
    /// Complete successfully sent datagram bytes, including retransmissions.
    pub tx_wire_bytes: u64,
    /// Bytes copied from RX slots into App dispatch scratch.
    pub app_rx_staged_bytes: u64,
    /// Payload bytes copied into Engine incoming block scratch.
    pub block_rx_staged_bytes: u64,
    /// Body bytes copied into App or Engine outgoing block scratch.
    pub block_tx_staged_bytes: u64,
    /// Payload bytes in successful Engine TX encodings, including protection.
    pub tx_payload_encoded_bytes: u64,
    /// Complete bodies passed to successful outgoing transfer starts.
    ///
    /// Built-in storage copies these bytes into a fresh body slot. A custom
    /// storage implementation may retain them differently.
    pub body_snapshot_bytes: u64,
    /// Bytes passed to successful Engine raw RX slot writes.
    pub raw_rx_copied_bytes: u64,
    /// Bytes passed to successful Engine raw TX slot writes.
    pub raw_tx_copied_bytes: u64,
    /// Retained body equality checks after representation identity matches.
    pub body_compare_calls: u64,
    /// Lengths of equally sized bodies presented to equality checks.
    ///
    /// This is a candidate-byte bound: comparisons may stop at a differing
    /// byte or use implementation-specific pointer or vectorized shortcuts.
    pub body_compare_candidate_bytes: u64,
}

impl WorkMetrics {
    pub(crate) const ZERO: Self = Self {
        polls: 0,
        recv_attempts: 0,
        recv_idle: 0,
        rx_wire_bytes: 0,
        tx_wire_bytes: 0,
        app_rx_staged_bytes: 0,
        block_rx_staged_bytes: 0,
        block_tx_staged_bytes: 0,
        tx_payload_encoded_bytes: 0,
        body_snapshot_bytes: 0,
        raw_rx_copied_bytes: 0,
        raw_tx_copied_bytes: 0,
        body_compare_calls: 0,
        body_compare_candidate_bytes: 0,
    };

    pub(crate) fn add(counter: &mut u64, bytes: usize) {
        *counter = counter.wrapping_add(bytes as u64);
    }
}
