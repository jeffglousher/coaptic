//! Caller-owned local diagnostics, without retained App state or event buffers.
//!
//! Snapshots read existing counters, admission usage and readiness. Record only
//! outcomes actually surfaced to the caller; sampling does not recover silent
//! authentication/replay drops, overwritten counter wraps, or unseen occupancy
//! peaks. No peer labels, payloads, keys or error strings are stored; no clock is read.
//! These values are local observations and must not select network error replies.

use super::{App, AppAssembled, AppStorage, AppUsage, AsyncCallError, CallFailure, Error};
use crate::error::{BlockTransferError, EncodeError, SlotMessageError};
use crate::storage::{DatagramIoError, MemoryLayout, MemoryProfile, Metrics, PoolUsage};

/// Fixed-cardinality category for a surfaced local outcome.
/// Detailed public errors remain available to the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Reason {
    /// Packet/table/byte capacity, including mandatory admission headroom.
    Capacity,
    /// An authenticated or unprotected matched overload response imposed cooldown.
    Backoff,
    /// Malformed or inconsistent protocol/options/response intent.
    Protocol,
    /// Security policy, authentication, replay or checkpoint failure, deliberately combined.
    Security,
    /// Local deadline or exchange/response lifetime elapsed; remote effects remain possible.
    Deadline,
    /// Local cancellation; this alone does not prove remote deregistration.
    Cancelled,
    /// Transport failure; peer delivery may be unknown.
    Transport,
    /// Acceptance occurred but completion/commit failed, or a continuation failed.
    DeliveryUncertain,
    /// Missing/stale handle, conflicting identity or invalid slot state.
    LocalState,
    /// Existing protocol work must run before a send can wait longer.
    ProgressRequired,
    /// The selected protocol/storage combination is unsupported.
    Unsupported,
    /// Fresh identity or entropy could not be obtained.
    Identity,
}

impl Reason {
    /// All categories in stable index order; no attacker-derived labels.
    pub const ALL: [Self; 12] = [
        Self::Capacity,
        Self::Backoff,
        Self::Protocol,
        Self::Security,
        Self::Deadline,
        Self::Cancelled,
        Self::Transport,
        Self::DeliveryUncertain,
        Self::LocalState,
        Self::ProgressRequired,
        Self::Unsupported,
        Self::Identity,
    ];

    /// Classify a surfaced synchronous or generic App error without inspecting transport data.
    pub fn from_error<E>(error: &Error<E>) -> Self {
        match error {
            Error::CommitAfterSend => Self::DeliveryUncertain,
            Error::Identity(_) => Self::Identity,
            Error::DeadlineElapsed => Self::Deadline,
            Error::Backoff { .. } => Self::Backoff,
            Error::Saturated | Error::ObserveRequestTooLarge | Error::FetchRequestTooLarge => {
                Self::Capacity
            }
            Error::Io(DatagramIoError::Saturated) => Self::Capacity,
            Error::Io(DatagramIoError::Slot(_)) | Error::Slot(_) => Self::LocalState,
            Error::Io(_) => Self::Transport,
            Error::Message(SlotMessageError::TokenCapacity(_))
            | Error::Message(SlotMessageError::Encode(EncodeError::BufferTooSmall)) => {
                Self::Capacity
            }
            Error::Message(SlotMessageError::Slot(_)) => Self::LocalState,
            Error::Message(_) => Self::Protocol,
            Error::Block(error) => Self::from_block(error),
            Error::Response(
                super::ResponseError::PayloadTruncated
                | super::ResponseError::DeferredMetadataBounds
                | super::ResponseError::LocationPathBounds
                | super::ResponseError::LocationQueryBounds,
            ) => Self::Capacity,
            Error::Response(_) => Self::Protocol,
            Error::DeferredUnknown
            | Error::ObserveCancellationMismatch
            | Error::ObserveRefreshMismatch
            | Error::ObserveCancellationAmbiguous
            | Error::RequestTagInUse => Self::LocalState,
            Error::ConditionalUploadUnsupported
            | Error::NoResponseUploadUnsupported
            | Error::NoResponseObserveUnsupported
            | Error::ObserveMethodUnsupported
            | Error::ObserveSupersessionUnsupported => Self::Unsupported,
            Error::Path | Error::FetchContentFormatRequired | Error::RequestTagRequired => {
                Self::Protocol
            }
            #[cfg(feature = "oscore")]
            Error::Oscore(_) | Error::OscoreCheckpointRequired | Error::OscoreCheckpointFailed => {
                Self::Security
            }
        }
    }

    /// Classify a collected terminal Call outcome.
    pub fn from_call_failure(failure: &CallFailure) -> Self {
        match failure {
            CallFailure::PacketHeadroom
            | CallFailure::ResponseMetadataBounds
            | CallFailure::ResponseBodyBounds => Self::Capacity,
            CallFailure::Cancelled => Self::Cancelled,
            CallFailure::AssociationRevoked => Self::Security,
            CallFailure::CancellationFailed
            | CallFailure::ReregistrationFailed
            | CallFailure::ContinuationFailed => Self::DeliveryUncertain,
            CallFailure::DeadlineExceeded | CallFailure::TimedOut => Self::Deadline,
            CallFailure::ObserveFormatMismatch | CallFailure::Reset => Self::Protocol,
            CallFailure::BlockTransfer(error) => Self::from_block(error),
        }
    }

    /// Preserve acceptance ambiguity from asynchronous initial send outcomes.
    pub fn from_async_call<E, C>(error: &AsyncCallError<E, C>) -> Self {
        match error {
            AsyncCallError::Request(error) => Self::from_error(error),
            AsyncCallError::ClockAfterSend(_)
            | AsyncCallError::ClockRegressedAfterSend { .. }
            | AsyncCallError::CommitAfterSend => Self::DeliveryUncertain,
            AsyncCallError::ProgressRequired { .. } => Self::ProgressRequired,
        }
    }

    /// Classify asynchronous progress/transport outcomes without copying clock errors.
    pub fn from_async_io<E, C>(error: &super::AsyncIoError<E, C>) -> Self {
        match error {
            super::AsyncIoError::Deadline => Self::Deadline,
            super::AsyncIoError::Transport(_) => Self::Transport,
            super::AsyncIoError::ClockBeforeSend(_)
            | super::AsyncIoError::ClockRegressedBeforeSend { .. } => Self::LocalState,
            super::AsyncIoError::ClockAfterSend(_)
            | super::AsyncIoError::ClockRegressedAfterSend { .. } => Self::DeliveryUncertain,
        }
    }

    fn from_block(error: &BlockTransferError) -> Self {
        match error {
            BlockTransferError::Saturated
            | BlockTransferError::Overflow
            | BlockTransferError::TokenCapacity(_)
            | BlockTransferError::Encode(EncodeError::BufferTooSmall) => Self::Capacity,
            BlockTransferError::NoBodyPools => Self::Unsupported,
            BlockTransferError::Slot(_)
            | BlockTransferError::NoTransfer
            | BlockTransferError::AlreadyComplete => Self::LocalState,
            _ => Self::Protocol,
        }
    }
}

/// A consistent caller-owned observation taken without advancing protocol state.
/// Counts are slot identities, not additive byte/RAM measurements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    /// Caller-supplied monotonic sampling time; no clock is read internally.
    pub at_ms: u64,
    /// Existing wrapping reactor/admission counters.
    pub metrics: Metrics,
    /// Current storage and App table occupancy/capacity.
    pub usage: AppUsage,
    /// Earliest protocol deadline, excluding completions waiting for collection.
    pub next_timeout_ms: Option<u64>,
    /// Earliest useful bounded poll, including already retained eligible work.
    pub next_poll_ms: Option<u64>,
    /// Existing optional work counters, not CPU time or total memory traffic.
    #[cfg(feature = "diagnostics")]
    pub work: crate::storage::WorkMetrics,
}

impl<
    P,
    T,
    const N: usize,
    const B: bool,
    S,
    const D: usize,
    const C: usize,
    const R: usize,
    const A: usize,
> App<P, T, N, B, S, D, C, R, A>
where
    P: MemoryProfile + MemoryLayout<B> + AppAssembled<B>,
    S: AppStorage<Token = P::Token>,
{
    /// Read bounded local diagnostics without I/O, allocation, expiry or mutation
    /// of protocol ownership. Work scans only configured tables.
    ///
    /// The returned value belongs to the caller. Dropping it changes nothing.
    /// Resetting App metrics does not reset any caller-owned recorder.
    pub fn diagnostic_snapshot(&mut self, now_ms: u64) -> Snapshot {
        Snapshot {
            at_ms: now_ms,
            metrics: self.metrics(),
            usage: self.resource_usage(),
            next_timeout_ms: self.next_timeout(),
            next_poll_ms: self.next_poll_at(now_ms),
            #[cfg(feature = "diagnostics")]
            work: self.work_metrics(),
        }
    }
}

/// Optional caller-owned sink for bounded categories and sampled occupancy maxima.
///
/// No App stores a reference to this recorder. Call `record` exactly once for
/// each outcome of interest; observing both a poll error and its later Call
/// failure counts twice if the caller records both. Counters wrap at their integer
/// maximum. `reset` clears only this object, not Engine counters or live work.
/// No events are queued, so there is no hidden overflow/drop policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Recorder {
    counts: [u32; Reason::ALL.len()],
    samples: u64,
    maxima: Option<AppUsage>,
}

impl Recorder {
    /// Record one local category, wrapping independently of other categories.
    pub fn record(&mut self, reason: Reason) {
        let count = &mut self.counts[reason as usize];
        *count = count.wrapping_add(1);
    }
    /// Recorded count for one category.
    pub const fn count(&self, reason: Reason) -> u32 {
        self.counts[reason as usize]
    }
    /// Number of samples, wrapping at `u64::MAX`.
    pub const fn samples(&self) -> u64 {
        self.samples
    }
    /// Maxima observed at explicit sampling instants, never lifetime high-water.
    ///
    /// Counts are componentwise maxima and need not have occurred simultaneously.
    /// Capacity fields describe the latest sample. Use one recorder per unchanged
    /// App configuration and reset before changing the App/storage it observes.
    pub const fn sampled_maxima(&self) -> Option<AppUsage> {
        self.maxima
    }
    /// Read a snapshot into the sampled maxima without changing its App.
    pub fn sample(&mut self, snapshot: &Snapshot) {
        self.samples = self.samples.wrapping_add(1);
        let Some(previous) = self.maxima else {
            self.maxima = Some(snapshot.usage);
            return;
        };
        let mut maxima = snapshot.usage;
        merge_pool(
            &mut maxima.storage.rx_datagram,
            previous.storage.rx_datagram,
        );
        merge_pool(
            &mut maxima.storage.tx_datagram,
            previous.storage.tx_datagram,
        );
        merge_pool(&mut maxima.storage.dedup, previous.storage.dedup);
        merge_pool(&mut maxima.storage.observe, previous.storage.observe);
        merge_optional(&mut maxima.storage.rx_body, previous.storage.rx_body);
        merge_optional(&mut maxima.storage.tx_body, previous.storage.tx_body);
        merge_pool(&mut maxima.routes, previous.routes);
        merge_pool(&mut maxima.client_requests, previous.client_requests);
        merge_pool(&mut maxima.client_replies, previous.client_replies);
        merge_pool(&mut maxima.deferred, previous.deferred);
        merge_optional(&mut maxima.security, previous.security);
        maxima.client_cooldowns = maxima.client_cooldowns.max(previous.client_cooldowns);
        maxima.completed_replies = maxima.completed_replies.max(previous.completed_replies);
        maxima.revoked_associations = maxima
            .revoked_associations
            .max(previous.revoked_associations);
        self.maxima = Some(maxima);
    }
    /// Clear samples and recorded reasons; live protocol state is unaffected.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn merge_pool(current: &mut PoolUsage, previous: PoolUsage) {
    current.occupied = current.occupied.max(previous.occupied);
}
fn merge_optional(current: &mut Option<PoolUsage>, previous: Option<PoolUsage>) {
    if let (Some(current), Some(previous)) = (current, previous) {
        merge_pool(current, previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorder_wraps_and_reset_is_local() {
        let mut recorder = Recorder::default();
        recorder.counts[Reason::Capacity as usize] = u32::MAX;
        recorder.record(Reason::Capacity);
        assert_eq!(recorder.count(Reason::Capacity), 0);
        recorder.record(Reason::Security);
        assert_eq!(recorder.count(Reason::Security), 1);
        recorder.reset();
        assert_eq!(recorder, Recorder::default());
    }
}
