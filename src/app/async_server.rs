//! Deadline-aware native async protocol transport.

use super::*;
use crate::platform::AsyncTimer;
use crate::storage::{AsyncDatagramIo, DatagramIo, DatagramTransport, Endpoint};
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

#[cfg(test)]
#[path = "async_server_tests.rs"]
mod tests;

/// Failure at an asynchronous protocol transport boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncIoError<E, C> {
    /// The bounded progress pass reached its absolute deadline before acceptance.
    Deadline,
    /// Native transport failure; transmission may be ambiguous.
    Transport(E),
    /// No transport call was made because the clock failed.
    ClockBeforeSend(C),
    /// Complete transport acceptance occurred, then the clock failed.
    /// Do not regenerate keys or repeat application effects on this outcome.
    ClockAfterSend(C),
    /// The clock regressed before a transport call.
    ClockRegressedBeforeSend {
        /// Last accepted timestamp.
        previous_ms: u64,
        /// Regressing sample.
        observed_ms: u64,
    },
    /// Complete acceptance occurred, but its timestamp regressed.
    ClockRegressedAfterSend {
        /// Timestamp sampled before acceptance.
        previous_ms: u64,
        /// Regressing completion sample.
        observed_ms: u64,
    },
}

impl<E: core::fmt::Display, C: core::fmt::Display> core::fmt::Display for AsyncIoError<E, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Deadline => f.write_str("async send deadline reached before acceptance"),
            Self::Transport(error) => write!(f, "{error}"),
            Self::ClockBeforeSend(error) => write!(f, "clock failed before send: {error}"),
            Self::ClockAfterSend(error) => write!(f, "clock failed after send acceptance: {error}"),
            Self::ClockRegressedBeforeSend {
                previous_ms,
                observed_ms,
            } => write!(
                f,
                "clock regressed before send from {previous_ms} to {observed_ms}"
            ),
            Self::ClockRegressedAfterSend {
                previous_ms,
                observed_ms,
            } => write!(
                f,
                "clock regressed after send acceptance from {previous_ms} to {observed_ms}"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl<E: std::error::Error + 'static, C: std::error::Error + 'static> std::error::Error
    for AsyncIoError<E, C>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::ClockBeforeSend(error) | Self::ClockAfterSend(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) trait ProtocolIo: AsyncDatagramIo {
    /// Latest successful complete transport acceptance timestamp.
    fn completed_at(&self) -> u64;
    fn refused_before_send(_error: &Self::Error) -> bool {
        false
    }
    fn failed_after_acceptance(_error: &Self::Error) -> bool {
        false
    }
    fn limit_deadline(&mut self, _due_ms: u64) {}
}

pub(crate) struct ProtocolTransport<'a, T, Clock, Timer> {
    pub(crate) io: &'a mut T,
    pub(crate) clock: &'a mut Clock,
    pub(crate) timer: &'a mut Timer,
    pub(crate) deadline: u64,
    pub(crate) completed: u64,
}

// A blocked send borrows the platform deadline. Restore it on cancellation as
// well as completion, before another operation reuses the timer.
struct BorrowedDeadline<'a, T: AsyncTimer> {
    timer: &'a mut T,
    previous: Option<u64>,
    changed: bool,
}

impl<T: AsyncTimer> Drop for BorrowedDeadline<'_, T> {
    fn drop(&mut self) {
        if self.changed {
            self.timer.schedule(self.previous);
        }
    }
}

impl<T: DatagramTransport, Clock: FnMut() -> Result<u64, C>, Timer, C> DatagramTransport
    for ProtocolTransport<'_, T, Clock, Timer>
{
    type Error = AsyncIoError<T::Error, C>;
    fn fill_random(&mut self, bytes: &mut [u8]) -> bool {
        self.io.fill_random(bytes)
    }
}

impl<T: AsyncDatagramIo, Clock: FnMut() -> Result<u64, C>, Timer: AsyncTimer, C> AsyncDatagramIo
    for ProtocolTransport<'_, T, Clock, Timer>
{
    async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        self.io.recv(bytes).await.map_err(AsyncIoError::Transport)
    }

    async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        let now = (self.clock)().map_err(AsyncIoError::ClockBeforeSend)?;
        if now < self.completed {
            return Err(AsyncIoError::ClockRegressedBeforeSend {
                previous_ms: self.completed,
                observed_ms: now,
            });
        }
        if now >= self.deadline {
            return Err(AsyncIoError::Deadline);
        }
        // A ready socket must not move the platform timer. A blocked send
        // borrows it only when its deadline is earlier, then puts the previous
        // deadline back. The registration is updated, not dropped.
        let deadline = self.deadline;
        let previous = self.timer.armed();
        let accepted = {
            let mut registration = BorrowedDeadline {
                timer: self.timer,
                previous,
                changed: false,
            };
            let mut send = pin!(self.io.send(dest, bytes));
            let mut waiting = false;
            let result = core::future::poll_fn(|cx| {
                if !waiting {
                    match send.as_mut().poll(cx) {
                        Poll::Ready(result) => {
                            return Poll::Ready(result.map_err(AsyncIoError::Transport));
                        }
                        Poll::Pending => {
                            if previous.map(|existing| deadline < existing).unwrap_or(true) {
                                registration.timer.schedule(Some(deadline));
                                registration.changed = true;
                            }
                            waiting = true;
                        }
                    }
                }
                if registration.timer.poll_wait(cx).is_ready() {
                    return Poll::Ready(Err(AsyncIoError::Deadline));
                }
                send.as_mut()
                    .poll(cx)
                    .map(|result| result.map_err(AsyncIoError::Transport))
            })
            .await;
            result?
        };
        let completed = (self.clock)().map_err(AsyncIoError::ClockAfterSend)?;
        if completed < now {
            return Err(AsyncIoError::ClockRegressedAfterSend {
                previous_ms: now,
                observed_ms: completed,
            });
        }
        self.completed = completed;
        Ok(accepted)
    }
}

impl<T: AsyncDatagramIo, Clock: FnMut() -> Result<u64, C>, Timer: AsyncTimer, C> ProtocolIo
    for ProtocolTransport<'_, T, Clock, Timer>
{
    fn completed_at(&self) -> u64 {
        self.completed
    }
    fn refused_before_send(error: &Self::Error) -> bool {
        matches!(
            error,
            AsyncIoError::Deadline
                | AsyncIoError::ClockBeforeSend(_)
                | AsyncIoError::ClockRegressedBeforeSend { .. }
        )
    }
    fn failed_after_acceptance(error: &Self::Error) -> bool {
        matches!(
            error,
            AsyncIoError::ClockAfterSend(_) | AsyncIoError::ClockRegressedAfterSend { .. }
        )
    }
    fn limit_deadline(&mut self, due_ms: u64) {
        self.deadline = self.deadline.min(due_ms);
    }
}

struct PacketOwner<'a, S: Storage> {
    engine: &'a mut Engine<S>,
    tx: SlotId,
}

struct RxOwner<'a, S: Storage> {
    engine: &'a mut Engine<S>,
    rx: SlotId,
    armed: bool,
}

impl<S: Storage> RxOwner<'_, S> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<S: Storage> Drop for RxOwner<'_, S> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.engine.release_rx(self.rx);
        }
    }
}

pub(super) struct BodyOwner<'a, S: Storage + BodySlots> {
    pub(super) engine: &'a mut Engine<S>,
    body: Option<(SlotId, bool)>,
    before_release: Option<fn(&mut Engine<S>, SlotId)>,
}

impl<'a, S: Storage + BodySlots> BodyOwner<'a, S> {
    pub(super) fn incoming(engine: &'a mut Engine<S>, body: Option<SlotId>) -> Self {
        Self {
            engine,
            body: body.map(|id| (id, true)),
            before_release: None,
        }
    }
    pub(super) fn outgoing(engine: &'a mut Engine<S>, body: SlotId) -> Self {
        Self {
            engine,
            body: Some((body, false)),
            before_release: None,
        }
    }
    pub(super) fn with_before_release(mut self, hook: Option<fn(&mut Engine<S>, SlotId)>) -> Self {
        self.before_release = hook;
        self
    }
    pub(super) fn disarm(&mut self) {
        self.body = None;
    }
}

struct NotifyOwner<'a, S: Storage + ObserveSlots> {
    engine: &'a mut Engine<S>,
    key: ObserveKey<S::Token>,
    seq: u32,
    armed: bool,
}

impl<S: Storage + ObserveSlots> NotifyOwner<'_, S> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<S: Storage + ObserveSlots> Drop for NotifyOwner<'_, S> {
    fn drop(&mut self) {
        if self.armed
            && let Some(id) = self.engine.lookup_observe(self.key)
        {
            let _ = self.engine.restore_observe_due(id, self.seq);
        }
    }
}

struct DeferredObserveOwner<'a, S: AppStorage> {
    engine: &'a mut Engine<S>,
    row: &'a mut DeferredRow<S::Token>,
    armed: bool,
}

impl<S: AppStorage> DeferredObserveOwner<'_, S> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<S: AppStorage> Drop for DeferredObserveOwner<'_, S> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(mut interest) = deferred_observe_owner(self.engine, *self.row)
            && let Some(id) = self.engine.lookup_observe(interest.key())
        {
            interest.clear_notify_hold();
            let _ = self.engine.storage_mut().set_observe_interest(id, interest);
        }
    }
}

impl<S: Storage + BodySlots> Drop for BodyOwner<'_, S> {
    fn drop(&mut self) {
        if let Some((body, incoming)) = self.body {
            if let Some(hook) = self.before_release {
                hook(self.engine, body);
            }
            if incoming {
                let _ = self.engine.release_rx_body(body);
            } else {
                let _ = self.engine.release_tx_body(body);
            }
        }
    }
}

impl<S: Storage> Drop for PacketOwner<'_, S> {
    fn drop(&mut self) {
        let _ = self.engine.release_tx(self.tx);
    }
}

impl<S: Storage> PacketOwner<'_, S> {
    fn engine(&mut self) -> &mut Engine<S> {
        self.engine
    }
}

impl<S: Storage + DatagramSlots> Engine<S> {
    pub(crate) fn send_owned_packet_async<'a, T: AsyncDatagramIo>(
        &'a mut self,
        io: &'a mut T,
        tx: SlotId,
    ) -> impl Future<Output = Result<(), DatagramIoError<T::Error>>> + 'a {
        let mut owner = PacketOwner { engine: self, tx };
        async move { owner.engine().send_tx_async(io, tx).await.map(|_| ()) }
    }

    // Waiting/receiving is a separate scheduling boundary. Progress consumes
    // existing RX slots and must never wait for fresh ingress before timers.
    pub(crate) async fn receive_retained_async<T: AsyncDatagramIo>(
        &mut self,
        _io: &mut T,
    ) -> Result<Option<SlotId>, DatagramIoError<T::Error>> {
        Ok(None)
    }
}

pub(crate) async fn completion_time_async<T: ProtocolIo>(io: &T, _now_ms: u64) -> u64 {
    io.completed_at()
}

pub(super) async fn send_may_have_reached_peer_async<T: ProtocolIo>(
    _io: &T,
    error: &Error<T::Error>,
) -> bool {
    match error {
        Error::Io(DatagramIoError::Io(error)) => !T::refused_before_send(error),
        Error::Io(DatagramIoError::SendLength { .. }) | Error::CommitAfterSend => true,
        _ => false,
    }
}

pub(super) async fn commit_q_window_send_async<S: Storage + BodySlots, E>(
    engine: &mut Engine<S>,
    issued: OutgoingBlock,
) -> Result<(), Error<E>> {
    engine
        .commit_q_block2(issued)
        .map_err(|_| Error::CommitAfterSend)
}

pub(super) async fn send_retransmit_packet_async<
    S: AppStorage,
    T: ProtocolIo,
    const C: usize,
    const R: usize,
>(
    engine: &mut Engine<S>,
    io: &mut T,
    inbox: &mut client::ClientInbox<C, S::Token>,
    lives: &mut client::ClientLives<C, S::Token>,
    oscore: &mut oscore::Field<R, S::Token>,
    tx: SlotId,
) -> Result<(), Error<T::Error>> {
    let expected = engine.pending_con(tx).ok_or(Error::Saturated)?;
    let result = engine.send_tx_async(io, tx).await;
    let failed = match result {
        Ok(_) => {
            if engine
                .commit_retransmit(expected, io.completed_at())
                .is_some()
            {
                if let Some(due) = engine
                    .pending_con(tx)
                    .and_then(|pending| pending.rto().next_timeout_ms())
                {
                    io.limit_deadline(due);
                }
                return Ok(());
            }
            Error::CommitAfterSend
        }
        Err(DatagramIoError::Io(error)) if T::refused_before_send(&error) => {
            return Err(Error::Io(DatagramIoError::Io(error)));
        }
        Err(error) => error.into(),
    };
    // Ambiguous transport acceptance must not silently repeat the same attempt
    // on the next pass or leave its Call/Observe owner waiting without a timer.
    client::fail_pending_client(
        engine,
        inbox,
        lives,
        oscore,
        tx,
        CallFailure::ContinuationFailed,
    );
    if let Some(interest) = engine.reject_observe_notify(expected.message_id(), expected.endpoint())
    {
        release_observe_response_body(engine, interest);
    }
    let _ = engine.release_tx(tx);
    Err(failed)
}

impl<
    P,
    T: AsyncDatagramIo,
    const N: usize,
    const BLOCK_WISE: bool,
    S: AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    fn async_send_deadline(&self, now_ms: u64, requested: u64) -> u64 {
        self.next_timeout_after(Some(now_ms))
            .map_or(requested, |due| requested.min(due))
    }
    /// Dispatch retained ingress and advance one bounded native async protocol pass.
    ///
    /// Use [`Self::wait_async`] for ingress/readiness. The absolute deadline bounds
    /// every send; expiry cancels an unaccepted transport operation and returns
    /// ownership through the protocol error path. Clock samples must use the same
    /// monotonic domain as `now_ms`, `deadline_ms`, and the timer.
    pub async fn progress_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        self.progress_protocol_async(now_ms, clock, timer, deadline_ms, None, None, None)
            .await
    }

    /// Advance a native async pass with the same stateful dispatch and deferred handles as [`Self::poll_with`].
    pub async fn progress_with_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        self.progress_protocol_async(
            now_ms,
            clock,
            timer,
            deadline_ms,
            None,
            Some(&mut dispatch),
            None,
        )
        .await
    }

    /// Progress with the same durable inbound replay barrier as [`Self::poll_with_oscore_checkpoint`].
    #[cfg(feature = "oscore")]
    pub async fn progress_with_oscore_checkpoint_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
        mut checkpoint: impl FnMut(crate::oscore::ReplayCheckpoint) -> bool,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        self.oscore_checkpoint.require();
        self.progress_protocol_async(
            now_ms,
            clock,
            timer,
            deadline_ms,
            Some(&mut |association, state| association.is_none() && checkpoint(state)),
            None,
            None,
        )
        .await
    }

    /// Advance shared associations with a durable replay barrier keyed by owner and epoch.
    #[cfg(feature = "oscore")]
    #[allow(clippy::too_many_arguments)]
    pub async fn progress_with_association_checkpoint_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
        mut checkpoint: impl FnMut(Option<Association>, crate::oscore::ReplayCheckpoint) -> bool,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        self.oscore_checkpoint.require();
        self.progress_protocol_async(
            now_ms,
            clock,
            timer,
            deadline_ms,
            Some(&mut checkpoint),
            Some(&mut dispatch),
            None,
        )
        .await
    }

    /// Share retained native async ingress with admission while preserving durable request barriers.
    #[cfg(feature = "oscore")]
    #[allow(clippy::too_many_arguments)]
    pub async fn progress_with_association_ingress_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
        mut ingress: impl FnMut(&[u8], Endpoint) -> bool,
        mut checkpoint: impl FnMut(Option<Association>, crate::oscore::ReplayCheckpoint) -> bool,
        mut dispatch: impl FnMut(
            Request<'_, S::Token>,
            Option<DeferredReply>,
        ) -> Response<'static, S::Token>,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        self.oscore_checkpoint.require();
        self.progress_protocol_async(
            now_ms,
            clock,
            timer,
            deadline_ms,
            Some(&mut checkpoint),
            Some(&mut dispatch),
            Some(&mut ingress),
        )
        .await
    }

    /// Complete deferred work using native async transport and existing packet/body pools.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_async<C>(
        &mut self,
        handle: DeferredReply,
        response: Response<'_, S::Token>,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        let deadline_ms = self.async_send_deadline(now_ms, deadline_ms);
        let mut io = ProtocolTransport {
            io: &mut self.io,
            clock,
            timer,
            deadline: deadline_ms,
            completed: now_ms,
        };
        complete_engine_async(
            &mut self.engine,
            &mut io,
            &mut self.ids,
            &mut self.oscore,
            &mut self.deferred,
            &mut self.dedup_closed,
            handle,
            response,
            now_ms,
        )
        .await
    }

    /// Send adaptive notifications with the same interest and NSTART rules as [`Self::notify`].
    #[allow(clippy::too_many_arguments)]
    pub async fn notify_async<C>(
        &mut self,
        now_ms: u64,
        path: &[&str],
        response: Response<'static, S::Token>,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<usize, Error<AsyncIoError<T::Error, C>>> {
        let resource = self.site.observe_resource(path);
        let deadline_ms = self.async_send_deadline(now_ms, deadline_ms);
        let mut io = ProtocolTransport {
            io: &mut self.io,
            clock,
            timer,
            deadline: deadline_ms,
            completed: now_ms,
        };
        notify_engine_async(
            &mut self.engine,
            &mut io,
            &mut self.ids,
            &mut self.oscore,
            now_ms,
            resource,
            &response,
            false,
            &self.deferred,
        )
        .await
    }

    /// Send explicit CON notifications through native async transport.
    #[allow(clippy::too_many_arguments)]
    pub async fn notify_confirmable_async<C>(
        &mut self,
        now_ms: u64,
        path: &[&str],
        response: Response<'static, S::Token>,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<usize, Error<AsyncIoError<T::Error, C>>> {
        let resource = self.site.observe_resource(path);
        let deadline_ms = self.async_send_deadline(now_ms, deadline_ms);
        let mut io = ProtocolTransport {
            io: &mut self.io,
            clock,
            timer,
            deadline: deadline_ms,
            completed: now_ms,
        };
        notify_engine_async(
            &mut self.engine,
            &mut io,
            &mut self.ids,
            &mut self.oscore,
            now_ms,
            resource,
            &response,
            true,
            &self.deferred,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn progress_protocol_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
        checkpoint: Option<&mut oscore::CheckpointCommit<'_>>,
        dispatch: Option<&mut AppDispatch<'_, S::Token>>,
        ingress: Option<&mut AppIngress<'_>>,
    ) -> Result<(), Error<AsyncIoError<T::Error, C>>> {
        // A quiet unprotected server reply must not resume the full protocol
        // state machine or scan its timers. Those timers are empty on this
        // path, and a ready socket never waits on the shortened deadline.
        let plain_server = checkpoint.is_none()
            && dispatch.is_none()
            && ingress.is_none()
            && self.echo_policy.is_none()
            && !self.oscore_checkpoint.required()
            && !oscore::is_active(&self.oscore);
        if plain_server {
            let mut io = ProtocolTransport {
                io: &mut self.io,
                clock,
                timer,
                deadline: deadline_ms,
                completed: now_ms,
            };
            // Read the datagram before any await. The commit below is the only
            // write: one handler call, one dedup record, then the send.
            let mut catalog = [];
            match try_plain_server(
                &mut self.engine,
                &self.site,
                &mut self.ids,
                &mut self.inbox,
                &mut self.lives,
                &mut self.oscore,
                &mut self.dedup_closed,
                &mut self.deferred,
                &mut io,
                &mut catalog,
                now_ms,
                probe_plain_bytes,
            ) {
                PlainAttempt::Miss => {}
                PlainAttempt::Done(result) => return result,
                PlainAttempt::Parked(rx, job) => {
                    return finish_plain_job(&mut self.engine, &mut io, rx, job).await;
                }
                PlainAttempt::Cold {
                    rx,
                    meta,
                    response,
                    separate,
                } => {
                    return finish_plain_cold(
                        &mut self.engine,
                        &mut self.ids,
                        &mut self.oscore,
                        &mut self.dedup_closed,
                        &mut io,
                        rx,
                        meta,
                        &response,
                        separate,
                        now_ms,
                    )
                    .await;
                }
            }
        }
        let deadline_ms = self.async_send_deadline(now_ms, deadline_ms);
        let mut io = ProtocolTransport {
            io: &mut self.io,
            clock,
            timer,
            deadline: deadline_ms,
            completed: now_ms,
        };
        poll_engine_async(
            &mut self.engine,
            P::provider::<S>(&mut self.assembled),
            &mut io,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.oscore_checkpoint,
            checkpoint,
            self.echo_policy,
            &mut self.dedup_closed,
            self.full_responses,
            &mut self.deferred,
            self.deferred_lifetime_ms,
            dispatch,
            now_ms,
            ingress,
            None,
        )
        .await
    }
}

enum PlainOnce {
    Accepted,
    Pending,
}

#[allow(
    clippy::large_enum_variant,
    reason = "Cold fallback owns a bounded response inline; boxing would allocate on the constrained path"
)]
enum PlainAttempt<'a, K: crate::message::OwnedToken, E> {
    Miss,
    Done(Result<(), Error<E>>),
    Parked(SlotId, PlainJob),
    Cold {
        rx: SlotId,
        meta: super::SendResponse<'a, K>,
        response: Response<'a, K>,
        separate: bool,
    },
}

/// Quiet unprotected request with only a path and query.
///
/// Any other option, a discovery path, or a path this walker will not own
/// returns `None` before dedup or the handler. The full progress machine then
/// makes that decision, including its 4.00 / 4.02 / 4.04 replies.
fn plain_path<'a, K: crate::message::OwnedToken, const REQUESTS: usize>(
    parsed: &crate::message::ParsedMessage<'a, K>,
    oscore: &oscore::Field<REQUESTS, K>,
) -> Option<super::request::Path<'a>> {
    if oscore::is_active(oscore) || parsed.is_empty() || !parsed.code().is_request() {
        return None;
    }
    if !matches!(parsed.ty(), Type::Confirmable | Type::NonConfirmable) {
        return None;
    }
    let mut segs = [""; super::MAX_PATH_SEGMENTS];
    let mut len = 0usize;
    for option in parsed.options() {
        match option.number() {
            crate::message::OptionNumber::URI_PATH => {
                let segment = core::str::from_utf8(option.value()).ok()?;
                if segment.is_empty()
                    || segment == "."
                    || segment == ".."
                    || segment == ".well-known"
                    || len >= super::MAX_PATH_SEGMENTS
                {
                    return None;
                }
                segs[len] = segment;
                len += 1;
            }
            crate::message::OptionNumber::URI_QUERY => {}
            crate::message::OptionNumber::NO_RESPONSE => {
                if crate::message::NoResponse::from_option(Some(option.value()))
                    != Some(crate::message::NoResponse::DEFAULT)
                {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(super::request::Path::from_validated(&segs[..len]))
}

/// Engine state that is not this one datagram. Independent of the packet bytes.
fn plain_scope<S, const DEFERRED: usize, const CALLS: usize>(
    engine: &mut Engine<S>,
    inbox: &client::ClientInbox<CALLS, S::Token>,
    lives: &client::ClientLives<CALLS, S::Token>,
    deferred: &DeferredTable<DEFERRED, S::Token>,
) -> Option<SlotId>
where
    S: AppStorage,
{
    if lives.usage().0 != 0 || deferred.rows.iter().any(|row| row.is_some()) {
        return None;
    }
    let rx = sole_plain_rx(engine)?;
    if inbox.holds_datagram(rx) {
        return None;
    }
    // One occupied RX and an empty TX already meet headroom when both pools
    // are larger than the reserve. A one-slot pool still belongs to the full
    // machine, which can refuse instead of running the handler.
    let caps = engine.capacities();
    if caps.tx_datagram_slots <= super::admission::TX_HEADROOM
        || caps.rx_datagram_slots <= super::admission::RX_HEADROOM
    {
        return None;
    }
    Some(rx)
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn try_plain_server<
    'cat,
    IO,
    E,
    S,
    Send,
    const N: usize,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
>(
    engine: &mut Engine<S>,
    site: &Site<N, S::Token>,
    ids: &mut AppIds,
    inbox: &mut client::ClientInbox<CALLS, S::Token>,
    lives: &mut client::ClientLives<CALLS, S::Token>,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
    deferred: &mut DeferredTable<DEFERRED, S::Token>,
    io: &mut IO,
    catalog: &'cat mut [u8],
    now_ms: u64,
    mut send: Send,
) -> PlainAttempt<'cat, S::Token, E>
where
    IO: DatagramTransport,
    S: AppStorage,
    Send: FnMut(&mut Engine<S>, &mut IO, Endpoint, &[u8]) -> Result<PlainOnce, Error<E>>,
{
    // --- Read. Nothing here expires, stores, or sends. ---
    // Scope does not depend on the bytes. The message id depends on the
    // bytes. The dedup read depends on that id. That is the only order.
    let Some(rx) = plain_scope(engine, inbox, lives, deferred) else {
        return PlainAttempt::Miss;
    };
    let peer = match engine.rx_endpoint(rx) {
        Some(endpoint) => endpoint.physical(),
        None => return PlainAttempt::Miss,
    };
    let mut prefix = [0u8; 128];
    let prefix_len = {
        let Some(bytes) = engine.storage().rx_payload(rx) else {
            return PlainAttempt::Miss;
        };
        if bytes.len() > prefix.len() {
            return PlainAttempt::Miss;
        }
        prefix[..bytes.len()].copy_from_slice(bytes);
        bytes.len()
    };
    let Ok(parsed) = crate::message::decode_with_token::<S::Token>(&prefix[..prefix_len]) else {
        return PlainAttempt::Miss;
    };
    let Some(path) = plain_path(&parsed, oscore) else {
        return PlainAttempt::Miss;
    };
    let request_ty = parsed.ty();
    let mut mid = parsed.message_id();
    let token = parsed.token();
    let request_code = parsed.code();
    let dedup_key = crate::storage::DedupKey::new(mid, peer);
    // A live row, or a fail-closed record from an insert that could not store
    // the reply, is the full machine's replay. It ACKs without the handler.
    if request_ty == Type::Confirmable
        && let Some(closed) = *dedup_closed
        && closed.key == dedup_key
        && (closed.due_ms == 0 || now_ms < closed.due_ms)
    {
        return PlainAttempt::Miss;
    }
    if engine.lookup_dedup(dedup_key).is_some_and(|id| {
        engine
            .dedup_entry_ref(id)
            .is_some_and(|entry| entry.due_ms() == 0 || now_ms < entry.due_ms())
    }) {
        return PlainAttempt::Miss;
    }

    // --- Commit. Handler once, then the reply bytes, then the socket. ---
    let mut response = site.dispatch(
        super::request::Request::from_prepared(parsed, path, peer, None),
        catalog,
    );
    if response.observe_seq().is_some()
        && let Err(error) = super::apply_observe(
            engine,
            now_ms,
            peer,
            &mut response,
            super::ObservePlan::idle(),
            None,
            oscore::no_request(),
        )
    {
        let _ = engine.release_rx(rx);
        return PlainAttempt::Done(Err(error.into()));
    }
    if request_ty == Type::NonConfirmable {
        match ids.next_for::<E, _, _>(engine, io, now_ms) {
            Ok(issued) => mid = issued,
            Err(error) => {
                let _ = engine.release_rx(rx);
                return PlainAttempt::Done(Err(error));
            }
        }
    }
    let handoff = response.is_deferred()
        || response.is_separate()
        || response.payload().len().saturating_add(64) > S::TX_DATAGRAM_BYTES
        || !response.has_only_etag_and_format()
        || response.etag_bytes().is_some()
        || response.format().is_some()
        || response.fresh_piv();
    if handoff {
        let separate = response.is_separate() && !response.is_deferred();
        let response = if response.is_deferred() {
            Response::new(Code::SERVICE_UNAVAILABLE)
        } else {
            response
        };
        return PlainAttempt::Cold {
            rx,
            meta: plain_meta(peer, request_ty, mid, token, request_code),
            response,
            separate,
        };
    }
    if let Err(error) = response.validate() {
        let _ = engine.release_rx(rx);
        return PlainAttempt::Done(Err(Error::Response(error)));
    }
    let wire_ty = match request_ty {
        Type::Confirmable => Type::Acknowledgement,
        Type::NonConfirmable => Type::NonConfirmable,
        Type::Acknowledgement | Type::Reset => {
            let _ = engine.release_rx(rx);
            return PlainAttempt::Done(Err(Error::Saturated));
        }
    };
    // One buffer is the record and the datagram. Dedup stores it before the
    // socket accepts, so a retransmit cannot run the handler again.
    let mut wire = [0u8; 64];
    let encoded = crate::message::encode_piggyback(
        &mut wire,
        wire_ty,
        response.code(),
        mid,
        token.as_bytes(),
        response.payload(),
    );
    let n = match encoded {
        Ok(n) => n,
        Err(EncodeError::BufferTooSmall) => {
            return PlainAttempt::Cold {
                rx,
                meta: plain_meta(peer, request_ty, mid, token, request_code),
                response,
                separate: false,
            };
        }
        Err(error) => {
            let _ = engine.release_rx(rx);
            return PlainAttempt::Done(Err(Error::Message(SlotMessageError::Encode(error))));
        }
    };
    remember_inline_reply(
        engine,
        peer,
        mid,
        request_ty,
        now_ms,
        request_code,
        dedup_closed,
        &wire[..n],
    );
    match send(engine, io, peer, &wire[..n]) {
        Ok(PlainOnce::Accepted) => {
            let _ = engine.release_rx(rx);
            PlainAttempt::Done(Ok(()))
        }
        Ok(PlainOnce::Pending) => match super::acquire_tx_or_evict(engine) {
            Some(tx) => match engine.write_tx(tx, &wire[..n], peer) {
                Ok(_) => PlainAttempt::Parked(
                    rx,
                    PlainJob {
                        tx,
                        dest: peer,
                        pin_tx: false,
                        ready_checked: true,
                    },
                ),
                Err(error) => {
                    let _ = engine.release_tx(tx);
                    let _ = engine.release_rx(rx);
                    PlainAttempt::Done(Err(error.into()))
                }
            },
            None => {
                let _ = engine.release_rx(rx);
                PlainAttempt::Done(Err(Error::Saturated))
            }
        },
        Err(error) => {
            let _ = engine.release_rx(rx);
            PlainAttempt::Done(Err(error))
        }
    }
}

/// One synchronous receive, then the quiet commit. `Miss` keeps that receive
/// so the full machine does not read the socket again.
pub(super) enum QuietPoll<E> {
    Miss(Result<Option<SlotId>, crate::storage::DatagramIoError<E>>),
    Done(Result<(), Error<E>>),
}

impl<
    P,
    T: DatagramIo,
    const N: usize,
    const BLOCK_WISE: bool,
    S: AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    pub(super) fn take_quiet_poll(&mut self, now_ms: u64) -> QuietPoll<T::Error> {
        let recv = self.engine.recv_from(&mut self.io);
        let mut catalog = [];
        let attempt = try_plain_server(
            &mut self.engine,
            &self.site,
            &mut self.ids,
            &mut self.inbox,
            &mut self.lives,
            &mut self.oscore,
            &mut self.dedup_closed,
            &mut self.deferred,
            &mut self.io,
            &mut catalog,
            now_ms,
            send_plain_sync,
        );
        match attempt {
            PlainAttempt::Miss => QuietPoll::Miss(recv),
            PlainAttempt::Done(result) => {
                if result.is_ok() {
                    crate::storage::Metrics::inc(&mut self.engine.metrics_mut().progress);
                    #[cfg(feature = "diagnostics")]
                    crate::storage::WorkMetrics::add(&mut self.engine.work_metrics_mut().polls, 1);
                }
                QuietPoll::Done(quiet_result(result, recv))
            }
            PlainAttempt::Parked(rx, job) => {
                let _ = self.engine.release_tx(job.tx);
                let _ = self.engine.release_rx(rx);
                QuietPoll::Done(Err(Error::Saturated))
            }
            PlainAttempt::Cold {
                rx,
                meta,
                response,
                separate,
            } => {
                let sent = if separate {
                    super::send_separate(
                        &mut self.engine,
                        &mut self.io,
                        &mut self.ids,
                        now_ms,
                        meta,
                        &response,
                        &mut self.oscore,
                        &mut self.dedup_closed,
                        false,
                    )
                } else {
                    super::send_response(
                        &mut self.engine,
                        &mut self.io,
                        &mut self.ids,
                        meta,
                        &response,
                        now_ms,
                        &mut self.oscore,
                        &mut self.dedup_closed,
                    )
                };
                let _ = self.engine.release_rx(rx);
                if sent.is_ok() {
                    crate::storage::Metrics::inc(&mut self.engine.metrics_mut().progress);
                    #[cfg(feature = "diagnostics")]
                    crate::storage::WorkMetrics::add(&mut self.engine.work_metrics_mut().polls, 1);
                }
                QuietPoll::Done(quiet_result(sent, recv))
            }
        }
    }
}

fn quiet_result<E>(
    result: Result<(), Error<E>>,
    recv: Result<Option<SlotId>, crate::storage::DatagramIoError<E>>,
) -> Result<(), Error<E>> {
    match result {
        Err(error) => Err(error),
        Ok(()) => match recv {
            Ok(_) => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

fn send_plain_sync<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    dest: Endpoint,
    bytes: &[u8],
) -> Result<PlainOnce, Error<T::Error>>
where
    S: Storage,
    T: DatagramIo,
{
    match io.send(dest, bytes) {
        Ok(actual) if actual == bytes.len() => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_ok);
            #[cfg(feature = "diagnostics")]
            crate::storage::WorkMetrics::add(&mut engine.work_metrics_mut().tx_wire_bytes, actual);
            Ok(PlainOnce::Accepted)
        }
        Ok(actual) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            Err(Error::from(crate::storage::DatagramIoError::SendLength {
                expected: bytes.len(),
                actual,
            }))
        }
        Err(error) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            Err(Error::from(crate::storage::DatagramIoError::Io(error)))
        }
    }
}

fn plain_meta<K: crate::message::OwnedToken>(
    peer: Endpoint,
    request_ty: Type,
    mid: MessageId,
    token: K,
    request_code: Code,
) -> super::SendResponse<'static, K> {
    super::SendResponse {
        dest: peer,
        ty: request_ty,
        mid,
        token,
        no_response: crate::message::NoResponse::DEFAULT,
        block2: None,
        q_block2: None,
        q_request: None,
        block1: None,
        oscore: oscore::no_request(),
        request: request_code,
    }
}

/// Record a confirmable reply before its send.
///
/// Reuses one due inline row when that is the whole table. Otherwise expires
/// due rows and inserts. Either way this is the only dedup write for the reply.
#[allow(
    clippy::too_many_arguments,
    reason = "Borrows the existing protocol owners without additional retained storage"
)]
fn remember_inline_reply<S>(
    engine: &mut Engine<S>,
    dest: Endpoint,
    mid: MessageId,
    request_ty: Type,
    now_ms: u64,
    request: Code,
    dedup_closed: &mut Option<DedupClosed>,
    bytes: &[u8],
) where
    S: Storage + DedupSlots,
{
    if request_ty != Type::Confirmable || bytes.len() > crate::storage::DedupEntry::REPLAY_MAX {
        return;
    }
    if let Some(closed) = *dedup_closed
        && closed.due_ms != 0
        && now_ms >= closed.due_ms
    {
        *dedup_closed = None;
    }
    let due = super::dedup_due_ms(now_ms);
    if engine
        .storage_mut()
        .overwrite_sole_due_inline(now_ms, mid, dest, due, bytes)
    {
        return;
    }
    super::expire_request_dedup(engine, dedup_closed, now_ms);
    let entry = crate::storage::DedupEntry::new(mid, dest)
        .with_due_ms(due)
        .with_replay(bytes);
    let _ = super::note_dedup_store(engine, entry, request, dedup_closed);
}

/// One readiness probe of an already-encoded reply. `Pending` sent nothing.
fn probe_plain_bytes<S, T, Clock, Timer, C>(
    engine: &mut Engine<S>,
    io: &mut ProtocolTransport<'_, T, Clock, Timer>,
    dest: Endpoint,
    bytes: &[u8],
) -> Result<PlainOnce, Error<AsyncIoError<T::Error, C>>>
where
    S: Storage,
    T: AsyncDatagramIo,
    Clock: FnMut() -> Result<u64, C>,
    Timer: AsyncTimer,
{
    let now = match (io.clock)() {
        Ok(now) => now,
        Err(error) => return Err(plain_io(AsyncIoError::ClockBeforeSend(error))),
    };
    if now < io.completed {
        return Err(plain_io(AsyncIoError::ClockRegressedBeforeSend {
            previous_ms: io.completed,
            observed_ms: now,
        }));
    }
    if now >= io.deadline {
        return Err(plain_io(AsyncIoError::Deadline));
    }
    let mut send = pin!(io.io.send(dest, bytes));
    let accepted = match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Pending => return Ok(PlainOnce::Pending),
        Poll::Ready(Ok(actual)) if actual == bytes.len() => actual,
        Poll::Ready(Ok(actual)) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            return Err(Error::from(crate::storage::DatagramIoError::SendLength {
                expected: bytes.len(),
                actual,
            }));
        }
        Poll::Ready(Err(error)) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            return Err(Error::from(crate::storage::DatagramIoError::Io(
                AsyncIoError::Transport(error),
            )));
        }
    };
    let completed = match (io.clock)() {
        Ok(completed) => completed,
        Err(error) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            return Err(plain_io(AsyncIoError::ClockAfterSend(error)));
        }
    };
    if completed < now {
        crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
        return Err(plain_io(AsyncIoError::ClockRegressedAfterSend {
            previous_ms: now,
            observed_ms: completed,
        }));
    }
    io.completed = completed;
    crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_ok);
    #[cfg(feature = "diagnostics")]
    crate::storage::WorkMetrics::add(&mut engine.work_metrics_mut().tx_wire_bytes, accepted);
    #[cfg(not(feature = "diagnostics"))]
    let _ = accepted;
    Ok(PlainOnce::Accepted)
}

async fn finish_plain_job<S, T, Clock, Timer, C>(
    engine: &mut Engine<S>,
    io: &mut ProtocolTransport<'_, T, Clock, Timer>,
    rx: SlotId,
    job: PlainJob,
) -> Result<(), Error<AsyncIoError<T::Error, C>>>
where
    S: AppStorage,
    T: AsyncDatagramIo,
    Clock: FnMut() -> Result<u64, C>,
    Timer: AsyncTimer,
{
    let mut owner = RxOwner {
        engine,
        rx,
        armed: true,
    };
    let sent = if job.pin_tx {
        super::send_pinned_tx_async(owner.engine, io, job.tx, job.dest).await
    } else if !job.ready_checked
        && let Some(result) = ready_plain_ack(owner.engine, io, job.tx, job.dest)
    {
        result
    } else {
        super::send::finish_protocol(owner.engine, io, job.tx, job.dest, None).await
    };
    let _ = owner.engine.release_rx(rx);
    owner.disarm();
    sent
}

/// Send an already-encoded plain reply when the socket accepts it immediately.
/// `None` means the socket would block; the caller keeps the TX slot and uses
/// the normal send future. A `Some` result has already released that slot.
type PlainSendResult<E, C> = Option<Result<(), Error<AsyncIoError<E, C>>>>;

fn ready_plain_ack<S, T, Clock, Timer, C>(
    engine: &mut Engine<S>,
    io: &mut ProtocolTransport<'_, T, Clock, Timer>,
    tx: SlotId,
    dest: Endpoint,
) -> PlainSendResult<T::Error, C>
where
    S: AppStorage,
    T: AsyncDatagramIo,
    Clock: FnMut() -> Result<u64, C>,
    Timer: AsyncTimer,
{
    if engine.pending_con(tx).is_some() {
        return None;
    }
    let now = match (io.clock)() {
        Ok(now) => now,
        Err(error) => {
            let _ = engine.release_tx(tx);
            return Some(Err(plain_io(AsyncIoError::ClockBeforeSend(error))));
        }
    };
    if now < io.completed {
        let _ = engine.release_tx(tx);
        return Some(Err(plain_io(AsyncIoError::ClockRegressedBeforeSend {
            previous_ms: io.completed,
            observed_ms: now,
        })));
    }
    if now >= io.deadline {
        let _ = engine.release_tx(tx);
        return Some(Err(plain_io(AsyncIoError::Deadline)));
    }
    if let Err(error) = engine.set_tx_endpoint(tx, dest) {
        let _ = engine.release_tx(tx);
        return Some(Err(Error::Slot(error)));
    }
    let mut context = Context::from_waker(Waker::noop());
    let polled = engine.poll_send_tx_once(io.io, tx, &mut context);
    let result = match polled {
        Poll::Pending => return None,
        Poll::Ready(result) => result,
    };
    let _ = engine.release_tx(tx);
    let accepted = match result {
        Ok(len) => len,
        Err(error) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            let error = match error {
                crate::storage::DatagramIoError::Io(error) => {
                    crate::storage::DatagramIoError::Io(AsyncIoError::Transport(error))
                }
                crate::storage::DatagramIoError::Slot(error) => {
                    crate::storage::DatagramIoError::Slot(error)
                }
                crate::storage::DatagramIoError::Saturated => {
                    crate::storage::DatagramIoError::Saturated
                }
                crate::storage::DatagramIoError::SendLength { expected, actual } => {
                    crate::storage::DatagramIoError::SendLength { expected, actual }
                }
            };
            return Some(Err(Error::from(error)));
        }
    };
    let completed = match (io.clock)() {
        Ok(completed) => completed,
        Err(error) => {
            crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
            return Some(Err(plain_io(AsyncIoError::ClockAfterSend(error))));
        }
    };
    if completed < now {
        crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_fail);
        return Some(Err(plain_io(AsyncIoError::ClockRegressedAfterSend {
            previous_ms: now,
            observed_ms: completed,
        })));
    }
    io.completed = completed;
    crate::storage::Metrics::inc(&mut engine.metrics_mut().tx_ok);
    #[cfg(feature = "diagnostics")]
    crate::storage::WorkMetrics::add(&mut engine.work_metrics_mut().tx_wire_bytes, accepted);
    #[cfg(not(feature = "diagnostics"))]
    let _ = accepted;
    Some(Ok(()))
}

fn plain_io<E, C>(error: AsyncIoError<E, C>) -> Error<AsyncIoError<E, C>> {
    Error::Io(crate::storage::DatagramIoError::Io(error))
}

#[allow(
    clippy::too_many_arguments,
    reason = "Cold dispatch borrows the existing protocol owners and transport"
)]
async fn finish_plain_cold<S, T, Clock, Timer, C, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
    io: &mut ProtocolTransport<'_, T, Clock, Timer>,
    rx: SlotId,
    meta: super::SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    separate: bool,
    now_ms: u64,
) -> Result<(), Error<AsyncIoError<T::Error, C>>>
where
    S: AppStorage,
    T: AsyncDatagramIo,
    Clock: FnMut() -> Result<u64, C>,
    Timer: AsyncTimer,
{
    let mut owner = RxOwner {
        engine,
        rx,
        armed: true,
    };
    let sent = if separate {
        super::send_separate_async(
            owner.engine,
            io,
            ids,
            now_ms,
            meta,
            response,
            oscore,
            dedup_closed,
            false,
        )
        .await
    } else {
        super::send_response_async(
            owner.engine,
            io,
            ids,
            meta,
            response,
            now_ms,
            oscore,
            dedup_closed,
        )
        .await
    };
    let _ = owner.engine.release_rx(rx);
    owner.disarm();
    sent
}

struct PlainJob {
    tx: SlotId,
    dest: Endpoint,
    pin_tx: bool,
    /// `ready_plain_ack` already returned `None` for this slot.
    ready_checked: bool,
}

fn sole_plain_rx<S>(engine: &mut Engine<S>) -> Option<SlotId>
where
    S: Storage + DatagramSlots + ObserveSlots + BodySlots,
{
    if engine.rx_occupied() != 1 || engine.tx_occupied() != 0 {
        return None;
    }
    if !engine.storage_mut().observe().is_empty()
        || engine
            .storage_mut()
            .tx_body()
            .is_some_and(|pool| !pool.is_empty())
        || engine
            .storage_mut()
            .rx_body()
            .is_some_and(|pool| !pool.is_empty())
    {
        return None;
    }
    let slots = engine.capacities().rx_datagram_slots;
    let mut found = None;
    for index in 0..slots {
        let id = SlotId::from_index(index);
        let pool = engine.storage_mut().rx_datagram();
        if !pool.is_occupied(id) {
            continue;
        }
        if pool.is_pinned(id) {
            return None;
        }
        found = Some(id);
    }
    found
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_rx_async<
    Mem,
    T,
    const N: usize,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
>(
    engine: &mut Engine<Mem>,
    scratch: impl super::response::DispatchScratch,
    io: &mut T,
    site: &Site<N, Mem::Token>,
    inbox: &mut client::ClientInbox<CALLS, Mem::Token>,
    lives: &mut client::ClientLives<CALLS, Mem::Token>,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, Mem::Token>,
    oscore_checkpoint: &mut oscore::CheckpointState,
    checkpoint_commit: &mut Option<&mut oscore::CheckpointCommit<'_>>,
    echo_policy: Option<EchoPolicy>,
    dedup_closed: &mut Option<DedupClosed>,
    full_responses: bool,
    deferred: &mut DeferredTable<DEFERRED, Mem::Token>,
    deferred_lifetime_ms: u64,
    dispatch: Option<&mut AppDispatch<'_, Mem::Token>>,
    now_ms: u64,
    rx: SlotId,
) -> Result<(), Error<T::Error>>
where
    Mem: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + Exchanges + DedupSlots,
    T: ProtocolIo,
{
    let mut owner = RxOwner {
        engine,
        rx,
        armed: true,
    };
    let result = super::dispatch_rx_io_async(
        owner.engine,
        scratch,
        io,
        site,
        inbox,
        lives,
        ids,
        oscore,
        oscore_checkpoint,
        checkpoint_commit,
        echo_policy,
        dedup_closed,
        full_responses,
        deferred,
        deferred_lifetime_ms,
        dispatch,
        now_ms,
        rx,
    )
    .await;
    limit_engine_deadlines(owner.engine, io);
    owner.disarm();
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn send_notification_async<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    interest: ObserveInterest<S::Token>,
    response: &Response<'_, S::Token>,
    seq: u32,
) -> Result<bool, Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots + DedupSlots,
    T: ProtocolIo,
{
    let mut owner = NotifyOwner {
        engine,
        key: interest.key(),
        seq,
        armed: true,
    };
    let result = super::send_notification_io_async(
        owner.engine,
        io,
        ids,
        oscore,
        now_ms,
        interest,
        response,
        seq,
    )
    .await;
    if result.as_ref().err().is_some_and(|error| match error {
        Error::Io(DatagramIoError::Io(error)) => T::failed_after_acceptance(error),
        Error::CommitAfterSend => true,
        _ => false,
    }) && let Some(interest) = owner.engine.take_observe(interest.key())
    {
        release_observe_response_body(owner.engine, interest);
    }
    limit_engine_deadlines(owner.engine, io);
    owner.disarm();
    result
}

pub(super) async fn complete_deferred_observe_async<
    S: AppStorage,
    T: ProtocolIo,
    const REQUESTS: usize,
>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    row: &mut DeferredRow<S::Token>,
    response: &Response<'_, S::Token>,
) -> Result<(), Error<T::Error>> {
    let mut owner = DeferredObserveOwner {
        engine,
        row,
        armed: true,
    };
    let result = super::complete_deferred_observe_io_async(
        owner.engine,
        io,
        ids,
        oscore,
        now_ms,
        owner.row,
        response,
    )
    .await;
    limit_engine_deadlines(owner.engine, io);
    owner.disarm();
    result
}

// Bound later sends by timers created during this same progress pass.
pub(super) fn limit_engine_deadlines<S, T>(engine: &Engine<S>, io: &mut T)
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: ProtocolIo,
{
    let now = io.completed_at();
    let mut limit = |due| {
        if due > now {
            io.limit_deadline(due);
        }
    };
    let caps = engine.capacities();
    for index in 0..caps.observe_entries {
        if let Some(row) = engine.observe_interest(SlotId::from_index(index)) {
            if let Some(lifetime) = row.lifetime() {
                limit(lifetime.due_ms());
            }
            if let Some(crate::storage::ObserveNotifyHold::Non { until_ms, .. }) = row.notify_hold()
            {
                limit(until_ms);
            }
        }
    }
    for index in 0..caps.rx_body_slots.unwrap_or(0) {
        if let Some(wait) = engine
            .rx_body_transfer(SlotId::from_index(index))
            .and_then(|row| row.q_receive())
        {
            limit(wait.next_timeout_ms());
        }
    }
    for index in 0..caps.tx_body_slots.unwrap_or(0) {
        if let Some(due) = engine
            .tx_body_transfer(SlotId::from_index(index))
            .and_then(|row| row.terminal_deadline())
        {
            limit(due);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn send_qblock_recover_async<S, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    inbox: &mut client::ClientInbox<CALLS, S::Token>,
    lives: &mut client::ClientLives<CALLS, S::Token>,
    ids: &mut AppIds,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    now_ms: u64,
    recover: QBlockRecover<S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + BodySlots + DedupSlots + Exchanges + ObserveSlots,
    T: ProtocolIo,
{
    let expected = engine
        .rx_body_transfer(recover.id())
        .and_then(|row| row.q_receive());
    let result = super::send_qblock_recover_io_async(
        engine,
        io,
        inbox,
        lives,
        ids,
        oscore,
        now_ms,
        recover,
        dedup_closed,
    )
    .await;
    let accepted_failure = result.as_ref().err().is_some_and(|error| match error {
        Error::Io(DatagramIoError::Io(error)) => !T::refused_before_send(error),
        Error::Io(DatagramIoError::SendLength { .. }) | Error::CommitAfterSend => true,
        _ => false,
    });
    let commit_failed = result.is_ok()
        && !expected
            .is_some_and(|wait| engine.commit_qblock_recovery(recover, wait, io.completed_at()));
    if accepted_failure || commit_failed {
        client::qblock_give_up(
            engine,
            inbox,
            lives,
            oscore,
            crate::storage::ExchangeKey::new(recover.key().token(), recover.key().endpoint()),
        );
        let _ = engine.release_rx_body(recover.id());
    }
    limit_engine_deadlines(engine, io);
    if commit_failed {
        Err(Error::CommitAfterSend)
    } else {
        result
    }
}

pub(super) async fn refresh_progress_deadlines_async<S, T>(engine: &Engine<S>, io: &mut T)
where
    S: Storage + DatagramSlots + PendingCons + ObserveSlots + BodySlots,
    T: ProtocolIo,
{
    limit_engine_deadlines(engine, io);
}
