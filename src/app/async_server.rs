//! Deadline-aware native async protocol transport.

use super::*;
use crate::platform::AsyncTimer;
use crate::storage::{AsyncDatagramIo, DatagramTransport, Endpoint};
use core::{future::Future, pin::pin, task::Poll};

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
        let accepted = {
            let mut timeout = pin!(self.timer.wait_until(self.deadline));
            let mut send = pin!(self.io.send(dest, bytes));
            core::future::poll_fn(|cx| {
                if timeout.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Err(AsyncIoError::Deadline));
                }
                send.as_mut()
                    .poll(cx)
                    .map(|result| result.map_err(AsyncIoError::Transport))
            })
            .await?
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
        )
        .await
    }
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
