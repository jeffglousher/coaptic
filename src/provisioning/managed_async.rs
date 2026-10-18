//! Native async driving of the same enrolled connection owner.
use super::*;
use crate::app::async_server::ProtocolTransport;
use crate::app::{AppWake, AsyncCallError, AsyncIoError};
use crate::platform::AsyncTimer;
use core::{future::Future, pin::pin, task::Poll};

/// Async connection failure; post-acceptance clock errors never imply no transmission.
#[derive(Debug)]
pub enum ManagedAsyncError<E, C> {
    /// Authorization, lifecycle, or bounded admission failure.
    State(ManagedError<E>),
    /// Application protocol I/O or completion failure.
    Application(crate::app::Error<AsyncIoError<ManagedIoError<E>, C>>),
    /// Bootstrap protocol I/O or completion failure.
    Recovery(RecoveryError<AsyncIoError<ManagedIoError<E>, C>>),
    /// Initial request failure, including explicit post-acceptance ambiguity.
    Call(AsyncCallError<ManagedIoError<E>, C>),
    /// The caller's absolute operation deadline elapsed before completion.
    Deadline,
    /// The clock failed; the connection is blocked until explicit policy refresh.
    Clock(C),
}

impl<E: fmt::Debug + fmt::Display, C: fmt::Debug + fmt::Display> fmt::Display
    for ManagedAsyncError<E, C>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => write!(f, "managed connection: {error}"),
            Self::Application(error) => write!(f, "managed async application: {error}"),
            Self::Recovery(error) => write!(f, "managed async recovery: {error:?}"),
            Self::Call(error) => write!(f, "managed async request: {error:?}"),
            Self::Deadline => f.write_str("managed async operation deadline elapsed"),
            Self::Clock(error) => write!(f, "managed async clock failed: {error}"),
        }
    }
}

impl<E: core::error::Error + 'static, C: core::error::Error + 'static> core::error::Error
    for ManagedAsyncError<E, C>
{
}

struct BootstrapOwner<'a, T>(&'a mut ConnectionIo<T>);
impl<T> BootstrapOwner<'_, T> {
    fn io(&mut self) -> &mut ConnectionIo<T> {
        self.0
    }
}
impl<T> Drop for BootstrapOwner<'_, T> {
    fn drop(&mut self) {
        self.0.bootstrap_mode = false;
    }
}

// Update the existing owner fields at every sample, not only after `.await`.
// A cancelled pass therefore cannot forget a clock advancement or fault.
fn tracked_clock<'a, C>(
    last_now: &'a mut u64,
    blocked: &'a mut bool,
    clock: &'a mut impl FnMut() -> Result<u64, C>,
) -> impl FnMut() -> Result<u64, C> + 'a {
    move || match clock() {
        Ok(now) => {
            if now < *last_now || now.checked_add(LIFETIME).is_none() {
                *blocked = true;
            }
            *last_now = (*last_now).max(now);
            Ok(now)
        }
        Err(error) => {
            *blocked = true;
            Err(error)
        }
    }
}

impl<E, C> ManagedAsyncError<E, C> {
    fn uncertain_send(&self) -> bool {
        use crate::storage::DatagramIoError;
        fn io_uncertain<E, C>(error: &AsyncIoError<E, C>) -> bool {
            matches!(
                error,
                AsyncIoError::Transport(_)
                    | AsyncIoError::ClockAfterSend(_)
                    | AsyncIoError::ClockRegressedAfterSend { .. }
            )
        }
        match self {
            Self::Call(
                AsyncCallError::ClockAfterSend(_)
                | AsyncCallError::ClockRegressedAfterSend { .. }
                | AsyncCallError::CommitAfterSend,
            ) => true,
            Self::Call(AsyncCallError::Request(crate::app::Error::Io(_))) => true,
            Self::Application(
                crate::app::Error::CommitAfterSend
                | crate::app::Error::Io(DatagramIoError::SendLength { .. }),
            ) => true,
            Self::Application(crate::app::Error::Io(DatagramIoError::Io(error))) => {
                io_uncertain(error)
            }
            Self::Recovery(RecoveryError::Bootstrap(super::super::PollError::Io(error))) => {
                io_uncertain(error)
            }
            Self::Recovery(RecoveryError::Bootstrap(super::super::PollError::ShortSend)) => true,
            _ => false,
        }
    }
}

impl<T: AsyncDatagramIo, P, const DEFERRED: usize, const CACHES: usize, const ROUTES: usize>
    ManagedConnection<'_, T, P, DEFERRED, CACHES, ROUTES>
where
    P: MemoryProfile + MemoryLayout<false> + AppAssembled<false>,
{
    /// Await retained ingress, the earliest connection deadline, or caller-owned work.
    /// Losing registrations are dropped before return. The existing connection
    /// ingress buffer is reused; cancellation never publishes a partial packet.
    pub async fn wait_async(
        &mut self,
        now_ms: u64,
        timer: &mut impl AsyncTimer,
        local: impl Future<Output = ()>,
    ) -> Result<AppWake, ManagedError<T::Error>> {
        self.time(now_ms)?;
        let deadline = self.next_poll_at(now_ms);
        if deadline.is_some_and(|due| due <= now_ms) {
            return Ok(AppWake::Ready);
        }
        let mut local = pin!(local);
        let mut timeout = pin!(async {
            if let Some(due) = deadline {
                timer.wait_until(due).await;
            } else {
                core::future::pending::<()>().await;
            }
        });
        let io = state_io(self.state.as_mut().expect("managed state"));
        let mut receive = pin!(io.receive_async());
        core::future::poll_fn(|cx| {
            if local.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Ok(AppWake::Local));
            }
            if timeout.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Ok(AppWake::Deadline));
            }
            receive.as_mut().poll(cx).map(|result| {
                result.map(|()| AppWake::Ingress).map_err(|error| {
                    ManagedError::Recovery(RecoveryError::Bootstrap(super::super::PollError::Io(
                        error,
                    )))
                })
            })
        })
        .await
    }

    pub(super) fn async_deadline(&self, requested: u64) -> u64 {
        match self.validity {
            PolicyValidity::OnlineUntil { deadline_ms, .. } => requested.min(deadline_ms),
            _ => requested,
        }
    }

    fn completion_clock<C>(
        &mut self,
        clock: &mut impl FnMut() -> Result<u64, C>,
    ) -> Result<u64, ManagedAsyncError<T::Error, C>> {
        match clock() {
            Ok(now) => {
                self.time(now).map_err(ManagedAsyncError::State)?;
                if self.refresh_blocked {
                    self.discard();
                    return Err(ManagedAsyncError::State(ManagedError::Clock));
                }
                Ok(now)
            }
            Err(error) => {
                self.discard();
                self.refresh_blocked = true;
                Err(ManagedAsyncError::Clock(error))
            }
        }
    }

    fn settle<R, C>(
        &mut self,
        result: Result<R, ManagedAsyncError<T::Error, C>>,
        clock: &mut impl FnMut() -> Result<u64, C>,
    ) -> Result<(R, u64), ManagedAsyncError<T::Error, C>> {
        let completed = self.completion_clock(clock);
        match (result, completed) {
            (Err(error), _) if error.uncertain_send() => Err(error),
            (_, Err(error)) => Err(error),
            (Ok(value), Ok(now)) => Ok((value, now)),
            (Err(error), Ok(_)) => Err(error),
        }
    }

    /// Advance one bounded retained-ingress pass without waiting for a new packet.
    pub async fn progress_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<(), ManagedAsyncError<T::Error, C>> {
        self.progress_with_async(now_ms, clock, timer, deadline_ms, |_, _, _| {
            Response::new(crate::message::Code::NOT_FOUND)
        })
        .await
    }

    /// Run the same authenticated dispatcher, recovery handoff and grant gates as
    /// `poll_with`. Every send is bounded by the earlier caller/policy deadline.
    /// Use `wait_async` between passes. Dropping a pass cancels unaccepted sends;
    /// cached recovery bytes and consumed security sequences remain owned.
    pub async fn progress_with_async<C>(
        &mut self,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
        mut dispatch: impl FnMut(
            &Principal,
            Request<'_>,
            Option<ManagedDeferredReply>,
        ) -> Response<'static>,
    ) -> Result<(), ManagedAsyncError<T::Error, C>> {
        let mut now_ms = now_ms;
        self.prepare(now_ms).map_err(ManagedAsyncError::State)?;
        self.route_ingress(now_ms)
            .map_err(ManagedAsyncError::State)?;
        let mut deadline = self.async_deadline(deadline_ms);
        if let Some(due) = self
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.next_timeout_after(now_ms))
        {
            deadline = deadline.min(due);
        }
        if !self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
            && let Some(State::Established(app)) = &self.state
            && let Some(due) = app.next_timeout_after(Some(now_ms))
        {
            deadline = deadline.min(due);
        }
        if !self.external_bootstrap {
            self.ensure_grant().map_err(ManagedAsyncError::State)?;
            let result = {
                let io = state_io(self.state.as_mut().expect("managed state"));
                io.bootstrap_mode = true;
                let mut owner = BootstrapOwner(io);
                let mut clock = tracked_clock(&mut self.last_now, &mut self.refresh_blocked, clock);
                let mut transport = ProtocolTransport {
                    io: owner.io(),
                    clock: &mut clock,
                    timer,
                    deadline,
                    completed: now_ms,
                };
                self.recovery
                    .as_mut()
                    .expect("prepared recovery")
                    .flush_protocol_async(&mut transport, now_ms)
                    .await
            };
            let (_, completed) = self.settle(result.map_err(ManagedAsyncError::Recovery), clock)?;
            now_ms = completed;
            self.install_session(completed)
                .map_err(ManagedAsyncError::State)?;
            self.ensure_grant().map_err(ManagedAsyncError::State)?;
            if self
                .recovery
                .as_ref()
                .is_some_and(CoapRecovery::has_candidate)
            {
                return Ok(());
            }
        }
        let captured = self.grant.as_ref().expect("prepared grant").principal();
        let anchor = self.grant.as_ref().expect("prepared grant").anchor();
        let observed = &self.observed;
        if let Some(State::Established(app)) = &mut self.state {
            if app.transport().queued.is_some() {
                app.receive_async()
                    .await
                    .map_err(|error| ManagedAsyncError::State(ManagedError::Application(error)))?;
            }
            let epoch = app.transport().epoch;
            let result = {
                let mut clock = tracked_clock(&mut self.last_now, &mut self.refresh_blocked, clock);
                app.progress_with_async(now_ms, &mut clock, timer, deadline, |request, handle| {
                    let response = dispatch(
                        captured,
                        request,
                        handle.map(|handle| ManagedDeferredReply {
                            handle,
                            anchor,
                            epoch,
                        }),
                    );
                    if request.is_observe_register() && !observable_path(observed, request.path()) {
                        response.without_observe()
                    } else {
                        response
                    }
                })
                .await
            };
            self.settle(result.map_err(ManagedAsyncError::Application), clock)?;
        }
        Ok(())
    }

    /// Send a protected request with the same Call identity and options as `request`.
    #[allow(clippy::too_many_arguments)]
    pub async fn request_async<C>(
        &mut self,
        method: Method,
        path: impl IntoPath,
        payload: &[u8],
        format: Option<ContentFormat>,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<ManagedCall, ManagedAsyncError<T::Error, C>> {
        self.request_inner_async(
            method,
            path,
            payload,
            format,
            now_ms,
            false,
            None,
            clock,
            timer,
            deadline_ms,
        )
        .await
    }

    /// Register a protected GET Observe relation using native async I/O.
    pub async fn observe_async<C>(
        &mut self,
        path: impl IntoPath,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<ManagedCall, ManagedAsyncError<T::Error, C>> {
        self.request_inner_async(
            Method::Get,
            path,
            &[],
            None,
            now_ms,
            true,
            None,
            clock,
            timer,
            deadline_ms,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn request_inner_async<C>(
        &mut self,
        method: Method,
        path: impl IntoPath,
        payload: &[u8],
        format: Option<ContentFormat>,
        now_ms: u64,
        observe: bool,
        reregister: Option<Call>,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<ManagedCall, ManagedAsyncError<T::Error, C>> {
        self.time(now_ms).map_err(ManagedAsyncError::State)?;
        self.ensure_grant().map_err(ManagedAsyncError::State)?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Err(ManagedAsyncError::State(ManagedError::Busy));
        }
        let endpoint = self.endpoint;
        let anchor = self.grant.as_ref().expect("authorized grant").anchor();
        let epoch = state_io(self.state.as_mut().expect("managed state")).epoch;
        let deadline = self.async_deadline(deadline_ms);
        if now_ms >= deadline {
            return Err(ManagedAsyncError::Deadline);
        }
        let result = {
            let app = match self.state.as_mut().expect("managed state") {
                State::Established(app) => app,
                State::Waiting(_) => {
                    return Err(ManagedAsyncError::State(ManagedError::NotConnected));
                }
            };
            let mut clock = tracked_clock(&mut self.last_now, &mut self.refresh_blocked, clock);
            let mut request = app.request(method, path).to(endpoint).payload(payload);
            if let Some(format) = format {
                request = request.content_format(format);
            }
            if let Some(call) = reregister {
                request = request.reregister_call(call);
            } else if observe {
                request = request.observe();
            }
            request
                .send_until_async(now_ms, &mut clock, timer, deadline)
                .await
                .map_err(ManagedAsyncError::Call)
        };
        self.settle(result, clock).map(|(call, _)| ManagedCall {
            call,
            anchor,
            epoch,
        })
    }

    /// Complete authorized deferred work through the native async transport.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_async<C>(
        &mut self,
        handle: ManagedDeferredReply,
        response: Response<'_>,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<(), ManagedAsyncError<T::Error, C>> {
        self.time(now_ms).map_err(ManagedAsyncError::State)?;
        self.ensure_grant().map_err(ManagedAsyncError::State)?;
        self.check_handle(handle.anchor, handle.epoch)
            .map_err(ManagedAsyncError::State)?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Err(ManagedAsyncError::State(ManagedError::Busy));
        }
        let deadline = self.async_deadline(deadline_ms);
        let result = {
            let app = match self.state.as_mut().expect("managed state") {
                State::Established(app) => app,
                State::Waiting(_) => {
                    return Err(ManagedAsyncError::State(ManagedError::NotConnected));
                }
            };
            let mut clock = tracked_clock(&mut self.last_now, &mut self.refresh_blocked, clock);
            app.complete_async(handle.handle, response, now_ms, &mut clock, timer, deadline)
                .await
        };
        self.settle(result.map_err(ManagedAsyncError::Application), clock)
            .map(|(value, _)| value)
    }

    /// Notify an explicitly authorized observable path through native async I/O.
    #[allow(clippy::too_many_arguments)]
    pub async fn notify_async<C>(
        &mut self,
        path: &[&str],
        response: Response<'static>,
        now_ms: u64,
        clock: &mut impl FnMut() -> Result<u64, C>,
        timer: &mut impl AsyncTimer,
        deadline_ms: u64,
    ) -> Result<usize, ManagedAsyncError<T::Error, C>> {
        self.time(now_ms).map_err(ManagedAsyncError::State)?;
        self.ensure_grant().map_err(ManagedAsyncError::State)?;
        if self
            .recovery
            .as_ref()
            .is_some_and(CoapRecovery::has_candidate)
        {
            return Err(ManagedAsyncError::State(ManagedError::Busy));
        }
        if !observable_path(&self.observed, path) {
            return Err(ManagedAsyncError::State(ManagedError::Path));
        }
        let deadline = self.async_deadline(deadline_ms);
        let result = {
            let app = match self.state.as_mut().expect("managed state") {
                State::Established(app) => app,
                State::Waiting(_) => {
                    return Err(ManagedAsyncError::State(ManagedError::NotConnected));
                }
            };
            let mut clock = tracked_clock(&mut self.last_now, &mut self.refresh_blocked, clock);
            app.notify_async(now_ms, path, response, &mut clock, timer, deadline)
                .await
        };
        self.settle(result.map_err(ManagedAsyncError::Application), clock)
            .map(|(value, _)| value)
    }
}
