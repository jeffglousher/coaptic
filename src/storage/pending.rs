//! Pending CON sidecar on outgoing datagram slots.
//!
//! Confirm matching is Message ID plus remote [`Endpoint`]. Retransmit
//! bookkeeping ([`PendingRto`]) lives on the same sidecar. This is not the
//! Dedup Table. See `knowledge/rfcs/rfc7252.txt` §4.2 / §4.4 / §4.8.

use super::endpoint::Endpoint;
use super::slot::SlotId;
use crate::message::{MessageId, Transmission};

/// Refusal to atomically replace one retained pending CON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingReplaceError {
    /// Slot, endpoint, MID or retry state no longer matches the expected view.
    Stale,
    /// Replacement is not a valid nonempty CON with a new MID and the same Token.
    InvalidMessage,
    /// The occupied slot is pinned or the replacement exceeds its capacity.
    Slot(super::SlotError),
    /// The replacement body cannot be admitted within the configured pool.
    Body(crate::error::BlockTransferError),
    /// A custom storage backend has not implemented atomic replacement.
    Unsupported,
}

/// Occupied TX slot waiting for an empty ACK or RST.
///
/// Stored as sidecar on the outgoing datagram slot. Capacity is the TX
/// Datagram Pool. This is not a seventh core memory area.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PendingCon {
    message_id: MessageId,
    endpoint: Endpoint,
    tx_slot: SlotId,
    rto: PendingRto,
}

impl PendingCon {
    /// Pending CON on `tx_slot` for `message_id` sent to `endpoint`.
    ///
    /// RTO is [`PendingRto::new`]`(0, 0)` (ACK_TIMEOUT at time zero). Use
    /// [`Self::with_rto`] after [`PendingCons::record_pending_con`] with a
    /// real clock.
    #[must_use]
    pub const fn new(message_id: MessageId, endpoint: Endpoint, tx_slot: SlotId) -> Self {
        Self {
            message_id,
            endpoint,
            tx_slot,
            rto: PendingRto::new(0, 0),
        }
    }

    /// Replace retransmit bookkeeping.
    #[must_use]
    pub const fn with_rto(self, rto: PendingRto) -> Self {
        Self {
            message_id: self.message_id,
            endpoint: self.endpoint,
            tx_slot: self.tx_slot,
            rto,
        }
    }

    /// Message ID of the outgoing CON.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.message_id
    }

    /// Remote endpoint the CON was sent to.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.endpoint
    }

    /// TX datagram slot that holds the outgoing bytes.
    #[must_use]
    pub const fn tx_slot(self) -> SlotId {
        self.tx_slot
    }

    /// Retransmit / RTO bookkeeping for this pending CON.
    #[must_use]
    pub const fn rto(self) -> PendingRto {
        self.rto
    }
}

/// Retransmit counter and timeout for one pending CON.
///
/// The core does not read an OS clock or draw randomness. [`Self::new`]
/// takes caller `now_ms` and `jitter_ms`; see
/// [`Transmission::initial_timeout_ms`]. See `knowledge/rfcs/rfc7252.txt`
/// §4.2.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PendingRto {
    attempts: u8,
    started: bool,
    next_timeout_ms: u64,
    timeout_ms: u32,
}

impl PendingRto {
    /// Initial RTO: retransmission counter 0, timeout from
    /// [`Transmission::initial_timeout_ms`], due at `now_ms + timeout`.
    #[must_use]
    pub const fn new(now_ms: u64, jitter_ms: u32) -> Self {
        let timeout_ms = Transmission::initial_timeout_ms(jitter_ms);
        Self {
            attempts: 0,
            started: true,
            timeout_ms,
            next_timeout_ms: now_ms.saturating_add(timeout_ms as u64),
        }
    }

    pub(crate) const fn reserved() -> Self {
        Self {
            attempts: 0,
            started: false,
            next_timeout_ms: 0,
            timeout_ms: 0,
        }
    }

    /// Whether transport acceptance started this CON's retransmission timer.
    /// A reservation counts against NSTART but cannot match ACK/RST or expire.
    #[must_use]
    pub const fn is_started(self) -> bool {
        self.started
    }

    /// Retransmission counter (0 after the first send).
    #[must_use]
    pub const fn attempts(self) -> u8 {
        self.attempts
    }

    /// Absolute timeout in the caller's clock domain, or `None` before send.
    #[must_use]
    pub const fn next_timeout_ms(self) -> Option<u64> {
        if self.started {
            Some(self.next_timeout_ms)
        } else {
            None
        }
    }

    /// Current timeout duration in milliseconds.
    #[must_use]
    pub const fn timeout_ms(self) -> u32 {
        self.timeout_ms
    }

    /// Whether `now_ms` is at or past [`Self::next_timeout_ms`].
    #[must_use]
    pub const fn is_due(self, now_ms: u64) -> bool {
        self.started && now_ms >= self.next_timeout_ms
    }

    /// After a due timeout: doubled timeout and incremented counter, or
    /// `None` when [`Transmission::MAX_RETRANSMIT`] is already reached.
    ///
    /// Next due is `now_ms` plus the doubled timeout (late polls shift the
    /// schedule; the core does not burst catch-up).
    #[must_use]
    pub const fn next_attempt(self, now_ms: u64) -> Option<Self> {
        if !self.started || self.attempts >= Transmission::MAX_RETRANSMIT {
            return None;
        }
        let timeout_ms = self.timeout_ms.saturating_mul(2);
        Some(Self {
            attempts: self.attempts.saturating_add(1),
            started: true,
            timeout_ms,
            next_timeout_ms: now_ms.saturating_add(timeout_ms as u64),
        })
    }
}

/// One due retransmit or a give-up after [`Transmission::MAX_RETRANSMIT`].
///
/// [`Self::Due`] keeps the TX slot pending (caller resends the bytes already
/// in the slot). [`Self::GiveUp`] clears pending and RTO; the slot stays
/// occupied so the caller can release it. See
/// `knowledge/rfcs/rfc7252.txt` §4.2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retransmit {
    /// Timeout expired and attempts remain.
    Due(PendingCon),
    /// Retransmission counter reached `MAX_RETRANSMIT` on this timeout.
    GiveUp(PendingCon),
}

/// Typed pending-CON access on the outgoing datagram pool.
///
/// Insert, lookup, take, and poll scan the configured TX slot count (O(n) in
/// capacity). Capacity is the TX Datagram Pool: a slot must already be
/// occupied. [`super::Memory`] and `AllocMemory` (with `alloc`) implement this.
pub trait PendingCons {
    /// Replace the encoded CON in its existing TX slot, preserving its RTO.
    ///
    /// `expected` must equal the complete current pending view, including retry
    /// state. Pass the view returned by [`Retransmit::Due`] immediately after a
    /// timeout. This is an exact snapshot, not a generation handle: caller-owned
    /// slot/MID reuse must not recreate an old matching snapshot.
    /// The staged datagram must be a nonempty CON with a different MID and the
    /// original Token. Refusal preserves all bytes, length and sidecars.
    /// No slot is acquired: full TX occupancy does not prevent replacement.
    ///
    /// This does not send, encrypt, update Observe/body ownership, or reserve
    /// MIDs. The caller must select a safe new MID and finish staging before
    /// calling. The old MID stops matching ACK/RST here; bounded superseded-ACK
    /// interest policy belongs to the caller. This alone does not implement
    /// RFC 7641 notification supersession. Custom backends default to refusal.
    fn replace_pending_con(
        &mut self,
        expected: PendingCon,
        bytes: &[u8],
    ) -> Result<PendingCon, PendingReplaceError> {
        let _ = (expected, bytes);
        Err(PendingReplaceError::Unsupported)
    }

    /// Reserve occupied TX `id` before an asynchronous first CON send.
    ///
    /// Counts against endpoint NSTART but starts no timer and cannot match
    /// ACK/RST. An identical reservation is idempotent; any other existing
    /// pending owner or exhausted endpoint capacity refuses without mutation.
    /// Bytes remain caller-owned and must already contain the intended CON.
    fn reserve_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
    ) -> Option<PendingCon>;

    /// Start RTO after the transport accepted the reserved CON in full.
    ///
    /// Refuses unless the exact reservation snapshot still owns its slot.
    /// `now_ms` is acceptance time, not reservation time. This does not send.
    /// Reservation snapshots are not generation handles: do not recreate an
    /// identical slot/MID snapshot while an earlier selection remains live.
    fn commit_pending_con(
        &mut self,
        expected: PendingCon,
        now_ms: u64,
        jitter_ms: u32,
    ) -> Option<PendingCon>;

    /// Cancel only the exact untransmitted reservation, releasing NSTART.
    /// The TX slot and its bytes stay occupied for the caller to release/reuse.
    /// A sent, replaced or removed snapshot refuses without mutation.
    fn cancel_pending_con(&mut self, expected: PendingCon) -> bool;

    /// Mark occupied TX `id` as pending and start RTO.
    ///
    /// `now_ms` is the caller clock (no OS time in the core). `jitter_ms`
    /// is applied by [`Transmission::initial_timeout_ms`]. `None` when `id`
    /// is free or out of range, or when outstanding pending CONs to
    /// `endpoint` already equal [`Transmission::NSTART`]. Idempotent when
    /// the same MID and endpoint are already recorded on `id` (RTO is not
    /// reset).
    fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
        now_ms: u64,
        jitter_ms: u32,
    ) -> Option<SlotId>;

    /// Occupied TX slot pending for `message_id` and `endpoint`, if any.
    fn lookup_pending_con(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId>;

    /// Clear pending and RTO for the matching TX slot. The slot stays occupied.
    fn take_pending_con(&mut self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId>;

    /// Reserved or transmitted CON view at TX `id`, if marked.
    /// Inspect [`PendingRto::is_started`] before using its timer deadline.
    fn pending_con(&self, id: SlotId) -> Option<PendingCon>;

    /// Select a due pending CON without changing its RTO or selection cursor.
    ///
    /// The result is the exact current snapshot, before counting a retransmit.
    /// Dropping it has no effect. Owned backends select round-robin, advancing
    /// only at commit. Every backend must expose its actual pending work.
    fn peek_retransmit(&self, now_ms: u64) -> Option<PendingCon>;

    /// Commit a previously selected timeout after the caller completes its work.
    ///
    /// `expected` must still match the entire pending snapshot and be due at
    /// `now_ms`. ACK removal, replacement or a previous commit makes it stale;
    /// refusal returns `None` without changing pending state or the cursor.
    /// This is a snapshot, not a generation handle: callers must not recreate
    /// an identical slot/MID/RTO snapshot while a selection remains outstanding.
    ///
    /// On `Due`, counts one retransmit and schedules backoff from `now_ms`;
    /// on `GiveUp`, clears pending but leaves TX bytes occupied. Neither sends
    /// nor releases bytes. Commit only after successful resend (or after the
    /// caller accepts give-up). The returned `Due` contains the updated RTO.
    fn commit_retransmit(&mut self, expected: PendingCon, now_ms: u64) -> Option<Retransmit>;

    /// Next pending CON whose timeout is due at `now_ms`, if any.
    ///
    /// Owned fixed and allocated backends scan round-robin (O(n)), advancing
    /// only after committing an event. A pinned payload does not pin its RTO.
    /// Custom backends define their selection policy. On
    /// [`Retransmit::Due`], schedules the next timeout (exponential
    /// backoff). On [`Retransmit::GiveUp`], clears pending. Does not send.
    fn poll_retransmit(&mut self, now_ms: u64) -> Option<Retransmit>;
}

pub(crate) fn validate_replacement(
    expected: PendingCon,
    current: Option<PendingCon>,
    original: Option<&[u8]>,
    bytes: &[u8],
    capacity: usize,
    pinned: bool,
) -> Result<PendingCon, PendingReplaceError> {
    if current != Some(expected) || !expected.rto().is_started() {
        return Err(PendingReplaceError::Stale);
    }
    if pinned {
        return Err(PendingReplaceError::Slot(super::SlotError::Pinned));
    }
    if bytes.len() > capacity {
        return Err(PendingReplaceError::Slot(
            super::SlotError::LengthExceedsSlot,
        ));
    }
    let old = crate::message::extended::decode(original.ok_or(PendingReplaceError::Stale)?)
        .map_err(|_| PendingReplaceError::InvalidMessage)?;
    let new =
        crate::message::extended::decode(bytes).map_err(|_| PendingReplaceError::InvalidMessage)?;
    if old.ty() != crate::message::Type::Confirmable
        || old.code().is_empty()
        || old.message_id() != expected.message_id()
        || new.ty() != crate::message::Type::Confirmable
        || new.code().is_empty()
        || new.message_id() == expected.message_id()
        || new.token() != old.token()
    {
        return Err(PendingReplaceError::InvalidMessage);
    }
    Ok(
        PendingCon::new(new.message_id(), expected.endpoint(), expected.tx_slot())
            .with_rto(expected.rto()),
    )
}

/// MID + RTO stored on one TX slot. Endpoint stays on the existing sidecar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingMark {
    pub message_id: MessageId,
    pub rto: PendingRto,
}

impl PendingMark {
    pub(crate) const fn new(message_id: MessageId, rto: PendingRto) -> Self {
        Self { message_id, rto }
    }
}

pub(crate) struct DueSlot {
    pub id: SlotId,
    pub event: Retransmit,
    pub mark: Option<PendingMark>,
}

pub(crate) fn next_due_slot(
    slot_count: usize,
    start: usize,
    now_ms: u64,
    mut pending_at: impl FnMut(SlotId) -> Option<PendingCon>,
) -> Option<DueSlot> {
    // Normalize even stale/custom-backend cursors without modulo by zero or
    // overflowing start + offset. Each slot is inspected at most once.
    let start = start.checked_rem(slot_count)?;
    for i in (start..slot_count).chain(0..start) {
        let id = SlotId::from_index(i);
        let Some(pending) = pending_at(id) else {
            continue;
        };
        if let Some(due) = due_snapshot(pending, now_ms) {
            return Some(due);
        }
    }
    None
}

pub(crate) fn due_snapshot(pending: PendingCon, now_ms: u64) -> Option<DueSlot> {
    if !pending.rto().is_due(now_ms) {
        return None;
    }
    let id = pending.tx_slot();
    Some(match pending.rto().next_attempt(now_ms) {
        Some(rto) => DueSlot {
            id,
            event: Retransmit::Due(pending.with_rto(rto)),
            mark: Some(PendingMark::new(pending.message_id(), rto)),
        },
        None => DueSlot {
            id,
            event: Retransmit::GiveUp(pending),
            mark: None,
        },
    })
}

pub(crate) fn outstanding_pending(
    slot_count: usize,
    endpoint: Endpoint,
    mut pending_at: impl FnMut(SlotId) -> Option<PendingCon>,
) -> usize {
    (0..slot_count)
        .filter(|&i| {
            pending_at(SlotId::from_index(i))
                .is_some_and(|p| p.endpoint().physical() == endpoint.physical())
        })
        .count()
}

/// Whether a new pending CON may be recorded on `id`.
///
/// `Some(false)` is an idempotent hit (keep existing RTO). `Some(true)`
/// admits a new or replacement mark. `None` is an NSTART reject.
pub(crate) fn record_admission(
    existing: Option<PendingCon>,
    message_id: MessageId,
    endpoint: Endpoint,
    outstanding: usize,
) -> Option<bool> {
    if let Some(pending) = existing {
        if pending.message_id() == message_id && pending.endpoint() == endpoint {
            return Some(false);
        }
        if pending.endpoint().physical() == endpoint.physical() {
            return Some(true);
        }
    }
    if outstanding >= usize::from(Transmission::NSTART) {
        return None;
    }
    Some(true)
}

#[cfg(test)]
mod fairness_tests {
    use super::*;

    #[test]
    fn empty_and_out_of_range_cursor_are_bounded() {
        assert!(next_due_slot(0, usize::MAX, u64::MAX, |_| panic!("empty scan")).is_none());
        for start in [0, 1, 3, usize::MAX] {
            let mut visited = [false; 3];
            assert!(
                next_due_slot(3, start, 0, |id| {
                    assert!(!visited[id.index()]);
                    visited[id.index()] = true;
                    None
                })
                .is_none()
            );
            assert_eq!(visited, [true; 3]);
        }
    }

    #[test]
    fn skip_future_owner_and_wrap_without_mutation() {
        let owner = Endpoint::v4([192, 0, 2, 1], 5683);
        let ready = PendingCon::new(MessageId::new(1), owner, SlotId::from_index(0))
            .with_rto(PendingRto::new(0, 0));
        let future = PendingCon::new(MessageId::new(2), owner, SlotId::from_index(2))
            .with_rto(PendingRto::new(10_000, 0));
        let rows = [Some(ready), None, Some(future)];
        let due = next_due_slot(3, 2, 2_000, |id| rows[id.index()]).unwrap();
        assert_eq!(due.id, ready.tx_slot());
        assert_eq!(rows, [Some(ready), None, Some(future)]);
        assert_eq!(
            next_due_slot(1, usize::MAX, 2_000, |_| Some(ready))
                .unwrap()
                .id,
            ready.tx_slot()
        );
    }
}
