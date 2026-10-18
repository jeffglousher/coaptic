//! Cancellation ownership around awaitable client protocol continuations.

use super::*;
use crate::app::{async_server::ProtocolIo, oscore};
use crate::storage::DatagramIoError;
use core::future::Future;

struct Continuation<'a, S, const C: usize, const R: usize>
where
    S: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    engine: &'a mut Engine<S>,
    inbox: &'a mut ClientInbox<C>,
    lives: &'a mut ClientLives<C>,
    oscore: &'a mut oscore::Field<R>,
    call: Option<Call>,
    rx: Option<SlotId>,
    cancel: bool,
}

impl<S, const C: usize, const R: usize> Continuation<'_, S, C, R>
where
    S: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    // A method receiver also forces the async closure to capture the entire
    // Drop owner, rather than independently capturing its reference fields.
    fn finish<T: ProtocolIo>(&mut self, result: &Result<impl Sized, Error<T::Error>>) {
        // A failed ACK may already have stopped the peer's response retries.
        // Keep cancellation cleanup armed for ambiguous sends so the Call
        // cannot remain live after discarding its only accepted response.
        self.cancel = result.as_ref().err().is_some_and(|error| match error {
            Error::Io(DatagramIoError::Io(error)) => !T::refused_before_send(error),
            Error::Io(DatagramIoError::SendLength { .. }) | Error::CommitAfterSend => true,
            _ => false,
        });
    }
}

impl<S, const C: usize, const R: usize> Drop for Continuation<'_, S, C, R>
where
    S: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
{
    fn drop(&mut self) {
        if !self.cancel {
            return;
        }
        // A continuation may already have advanced a body cursor or accepted
        // earlier Q payloads. Retire the affected Call rather than replaying
        // those mutations or silently leaving it without a future wakeup.
        if let Some(call) = self.call.filter(|call| self.lives.get(*call).is_some()) {
            abandon_send(self.engine, self.oscore, call.token(), call.peer);
            fail_call(
                self.engine,
                self.inbox,
                self.lives,
                self.oscore,
                call,
                CallFailure::ContinuationFailed,
                true,
            );
        }
        if let Some(rx) = self.rx.filter(|rx| !self.inbox.holds_datagram(*rx)) {
            let _ = self.engine.release_rx(rx);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn complete_client_async<'a, Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &'a mut Engine<Mem>,
    io: &'a mut T,
    inbox: &'a mut ClientInbox<CALLS>,
    lives: &'a mut ClientLives<CALLS>,
    ids: &'a mut AppIds,
    oscore: &'a mut oscore::Field<REQUESTS>,
    response_state: oscore::ResponseState,
    full_responses: bool,
    now_ms: u64,
    peer: Endpoint,
    parsed: &'a ParsedMessage<'_>,
    rx: SlotId,
) -> impl Future<Output = Result<(), Error<T::Error>>> + 'a
where
    Mem: Storage
        + DatagramSlots
        + PendingCons
        + Exchanges
        + BodySlots
        + ObserveSlots
        + crate::storage::DedupSlots,
    T: ProtocolIo,
{
    let call = lives
        .get(Call::new(parsed.token(), peer))
        .map(|live| live.call);
    let mut owner = Continuation {
        engine,
        inbox,
        lives,
        oscore,
        call,
        rx: Some(rx),
        cancel: true,
    };
    async move {
        let result = complete_client_io_async(
            owner.engine,
            io,
            owner.inbox,
            owner.lives,
            ids,
            owner.oscore,
            response_state,
            full_responses,
            now_ms,
            peer,
            parsed,
            rx,
        )
        .await;
        owner.finish::<T>(&result);
        result
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn recover_qblock2_async<'a, Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &'a mut Engine<Mem>,
    io: &'a mut T,
    inbox: &'a mut ClientInbox<CALLS>,
    lives: &'a mut ClientLives<CALLS>,
    ids: &'a mut AppIds,
    oscore: &'a mut oscore::Field<REQUESTS>,
    now_ms: u64,
    recover: crate::storage::QBlockRecover,
) -> impl Future<Output = Result<bool, Error<T::Error>>> + 'a
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: ProtocolIo,
{
    let call = Call::new(recover.key().token(), recover.key().endpoint());
    let mut owner = Continuation {
        engine,
        inbox,
        lives,
        oscore,
        call: Some(call),
        rx: None,
        cancel: true,
    };
    async move {
        let result = recover_qblock2_io_async(
            owner.engine,
            io,
            owner.inbox,
            owner.lives,
            ids,
            owner.oscore,
            now_ms,
            recover,
        )
        .await;
        owner.finish::<T>(&result);
        result
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pace_qblock1_async<'a, Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &'a mut Engine<Mem>,
    io: &'a mut T,
    inbox: &'a mut ClientInbox<CALLS>,
    lives: &'a mut ClientLives<CALLS>,
    ids: &'a mut AppIds,
    oscore: &'a mut oscore::Field<REQUESTS>,
    now_ms: u64,
) -> impl Future<Output = Result<(), Error<T::Error>>> + 'a
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: ProtocolIo,
{
    let call = lives
        .rows
        .iter()
        .flatten()
        .filter(|live| live.q_pace_ms != 0 && live.q_pace_ms <= now_ms)
        .min_by_key(|live| live.q_pace_ms)
        .map(|live| live.call);
    let mut owner = Continuation {
        engine,
        inbox,
        lives,
        oscore,
        call,
        rx: None,
        cancel: true,
    };
    async move {
        let result = pace_qblock1_io_async(
            owner.engine,
            io,
            owner.inbox,
            owner.lives,
            ids,
            owner.oscore,
            now_ms,
        )
        .await;
        owner.finish::<T>(&result);
        result
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn continue_qblock1_ack_async<'a, Mem, T, const CALLS: usize, const REQUESTS: usize>(
    engine: &'a mut Engine<Mem>,
    io: &'a mut T,
    inbox: &'a mut ClientInbox<CALLS>,
    lives: &'a mut ClientLives<CALLS>,
    ids: &'a mut AppIds,
    oscore: &'a mut oscore::Field<REQUESTS>,
    now_ms: u64,
    mid: MessageId,
    peer: Endpoint,
) -> impl Future<Output = Result<(), Error<T::Error>>> + 'a
where
    Mem: Storage + DatagramSlots + PendingCons + Exchanges + BodySlots + ObserveSlots,
    T: ProtocolIo,
{
    let call = exchange_for_mid(engine, mid, peer).map(|entry| Call::new(entry.token(), peer));
    let mut owner = Continuation {
        engine,
        inbox,
        lives,
        oscore,
        call,
        rx: None,
        cancel: true,
    };
    async move {
        let result = continue_qblock1_ack_io_async(
            owner.engine,
            io,
            owner.inbox,
            owner.lives,
            ids,
            owner.oscore,
            now_ms,
            mid,
            peer,
        )
        .await;
        owner.finish::<T>(&result);
        result
    }
}
