//! Bounded [`Progress`] from one [`Engine::progress`] invocation.
//!
//! See `design.md` §Reference progress contract, Bounded progress, and
//! Ownership by progress domain.

use super::DatagramSlots;
use super::Engine;
use super::PendingCons;
use super::Retransmit;
use super::SlotId;
use super::Storage;

/// Outcome of one bounded [`Engine::progress`] pass.
///
/// Idle when [`Self::is_idle`]. Observe notify and Q-Block missing-block
/// recovery are later PRs; [`Self::observe_notify`] and
/// [`Self::qblock_recover`] stay `None` here.
///
/// See `design.md` §Reference progress contract.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Progress {
    retransmit: Option<Retransmit>,
    rx_ready: Option<SlotId>,
    observe_notify: Option<SlotId>,
    qblock_recover: Option<SlotId>,
}

impl Progress {
    /// No retransmit, no RX slot, and no later-PR domain work.
    #[must_use]
    pub const fn idle() -> Self {
        Self {
            retransmit: None,
            rx_ready: None,
            observe_notify: None,
            qblock_recover: None,
        }
    }

    /// Whether this pass reported no work.
    #[must_use]
    pub const fn is_idle(self) -> bool {
        self.retransmit.is_none()
            && self.rx_ready.is_none()
            && self.observe_notify.is_none()
            && self.qblock_recover.is_none()
    }

    /// Due CON retransmit or give-up from [`Engine::poll_retransmit`].
    ///
    /// The core does not send. [`Retransmit::GiveUp`] already cleared pending;
    /// the TX slot stays occupied so the caller can release it.
    #[must_use]
    pub const fn retransmit(self) -> Option<Retransmit> {
        self.retransmit
    }

    /// One occupied RX datagram that is not pinned, if any.
    ///
    /// Decode readiness only: the caller may [`Engine::decode_rx`]. This pass
    /// does not release the slot and does not dispatch to the application.
    #[must_use]
    pub const fn rx_ready(self) -> Option<SlotId> {
        self.rx_ready
    }

    /// Observe notify domain. Always `None` until that PR.
    ///
    /// See `design.md` §Ownership by progress domain.
    #[must_use]
    pub const fn observe_notify(self) -> Option<SlotId> {
        self.observe_notify
    }

    /// Q-Block missing-block recovery. Always `None` until that PR.
    ///
    /// See `design.md` §Ownership by progress domain.
    #[must_use]
    pub const fn qblock_recover(self) -> Option<SlotId> {
        self.qblock_recover
    }
}

impl<S: Storage + DatagramSlots + PendingCons> Engine<S> {
    /// One bounded progress invocation.
    ///
    /// Caller supplies `now_ms` (no OS clock). Each call:
    ///
    /// 1. [`Self::poll_retransmit`] — at most one due CON (`Due` / `GiveUp`).
    /// 2. One rotating RX step from the pool cursor: first occupied slot
    ///    that is not pinned. The cursor resumes after that slot.
    /// 3. Observe notify — stub ([`Progress::observe_notify`] is `None`).
    /// 4. Q-Block missing-block recovery — stub
    ///    ([`Progress::qblock_recover`] is `None`).
    ///
    /// Does not allocate, grow storage, or send on the wire. Does not release
    /// pinned slots. Does not invent 4.02 / RST / 2.31 policy.
    ///
    /// See `design.md` §Reference progress contract / Bounded progress /
    /// Ownership by progress domain.
    pub fn progress(&mut self, now_ms: u64) -> Progress {
        let retransmit = self.poll_retransmit(now_ms);
        let rx_ready = next_unpinned_rx(self);
        Progress {
            retransmit,
            rx_ready,
            observe_notify: progress_observe(),
            qblock_recover: progress_qblock(),
        }
    }
}

/// Observe notify fan-out. Later PR; see `design.md` Observe progress.
const fn progress_observe() -> Option<SlotId> {
    None
}

/// Q-Block missing-block recovery. Later PR; see `design.md` Block/Q-Block progress.
const fn progress_qblock() -> Option<SlotId> {
    None
}

/// First occupied, unpinned RX slot starting at the rotating cursor.
///
/// Advances the cursor past the visited slot (or by one when every occupied
/// slot is pinned) so the next call does not restart at slot zero.
fn next_unpinned_rx<S: Storage>(engine: &mut Engine<S>) -> Option<SlotId> {
    let n = engine.storage_mut().rx_datagram().slot_count();
    if n == 0 {
        return None;
    }
    let start = engine.storage_mut().rx_datagram().cursor();
    let mut found = None;
    for offset in 0..n {
        let id = SlotId::from_index((start + offset) % n);
        let pool = engine.storage_mut().rx_datagram();
        if pool.is_occupied(id) && !pool.is_pinned(id) {
            found = Some((id, offset));
            break;
        }
    }
    match found {
        Some((id, offset)) => {
            for _ in 0..=offset {
                engine.rotate_rx();
            }
            Some(id)
        }
        None => {
            if engine.storage_mut().rx_datagram().occupied_count() > 0 {
                engine.rotate_rx();
            }
            None
        }
    }
}
