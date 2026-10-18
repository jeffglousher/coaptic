//! Slot ownership across initial App sends and their transport completion.

use super::Error;
use crate::message::MessageId;
use crate::storage::{
    AsyncDatagramIo, DatagramIo, DatagramIoError, DatagramSlots, Endpoint, Engine, PendingCon,
    PendingCons, SlotId, Storage,
};

/// An initial send failed. Transport errors may already have transmitted bytes;
/// clock and commit errors explicitly follow successful complete acceptance.
#[derive(Debug)]
pub(crate) enum AsyncSendError<E, C> {
    Prepare(Error<E>),
    Transport(DatagramIoError<E>),
    ClockAfterSend(C),
    CommitAfterSend,
}

// Construct before the returned future: even an unpolled send owns cleanup.
// The transport's borrowed packet future is dropped before this owner, so its
// Access pin is gone before release_tx. No packet bytes live in this guard.
struct SendOwner<'a, S: Storage + PendingCons> {
    engine: &'a mut Engine<S>,
    tx: SlotId,
    reserved: Option<PendingCon>,
    release: bool,
}

impl<S: Storage + PendingCons> Drop for SendOwner<'_, S> {
    fn drop(&mut self) {
        if let Some(reserved) = self.reserved.take() {
            let _ = self.engine.cancel_pending_con(reserved);
        }
        if self.release {
            let _ = self.engine.release_tx(self.tx);
        }
    }
}

impl<'a, S: Storage + DatagramSlots + PendingCons> SendOwner<'a, S> {
    fn prepare<E>(
        engine: &'a mut Engine<S>,
        tx: SlotId,
        dest: Endpoint,
        mid: Option<MessageId>,
        expected: Option<PendingCon>,
    ) -> Result<Self, Error<E>> {
        // Existing protocol ownership is not transferred by this initial-send
        // API. Refuse it without changing its endpoint, bytes or timer.
        if let Some(expected) = expected {
            if engine.pending_con(tx) != Some(expected)
                || expected.rto().is_started()
                || expected.endpoint() != dest
                || Some(expected.message_id()) != mid
            {
                return Err(Error::Saturated);
            }
        } else if engine.pending_con(tx).is_some() {
            return Err(Error::Saturated);
        }
        let mut owner = Self {
            engine,
            tx,
            reserved: expected,
            release: true,
        };
        owner.engine.set_tx_endpoint(tx, dest)?;
        if let Some(mid) = mid
            && expected.is_none()
        {
            owner.reserved = Some(
                owner
                    .engine
                    .reserve_pending_con(tx, dest, mid)
                    .ok_or(Error::Saturated)?,
            );
        }
        Ok(owner)
    }

    fn commit(&mut self, now_ms: u64, jitter_ms: u32) -> bool {
        if let Some(reserved) = self.reserved {
            if self
                .engine
                .commit_pending_con(reserved, now_ms, jitter_ms)
                .is_none()
            {
                return false;
            }
            self.reserved = None;
            self.release = false;
        }
        true
    }
}

pub(super) fn finish<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    tx: SlotId,
    dest: Endpoint,
    pending: Option<(u64, MessageId, u32)>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons,
    T: DatagramIo,
{
    let mut owner = SendOwner::prepare(engine, tx, dest, pending.map(|(_, mid, _)| mid), None)?;
    owner.engine.send_tx(io, tx)?;
    let (now_ms, jitter_ms) = pending.map_or((0, 0), |(now, _, jitter)| (now, jitter));
    if !owner.commit(now_ms, jitter_ms) {
        return Err(Error::Saturated);
    }
    Ok(())
}

pub(super) fn finish_prepared<S, T>(
    engine: &mut Engine<S>,
    io: &mut T,
    tx: SlotId,
    dest: Endpoint,
    pending: Option<(crate::storage::PendingCon, u32)>,
    now_ms: u64,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons,
    T: DatagramIo,
{
    let mut owner = SendOwner::prepare(
        engine,
        tx,
        dest,
        pending.map(|(p, _)| p.message_id()),
        pending.map(|(p, _)| p),
    )?;
    owner.engine.send_tx(io, tx)?;
    if !owner.commit(now_ms, pending.map_or(0, |(_, jitter)| jitter)) {
        return Err(Error::Saturated);
    }
    Ok(())
}

/// Transfer an occupied TX slot into an awaitable initial send.
///
/// The returned future eagerly owns the slot and CON admission, including when
/// dropped before its first poll. Cancellation, transport failure and post-send
/// clock/commit failure release this ownership; they never rewind security
/// sequence numbers. A successful NON releases TX; a successful CON retains its
/// exact bytes with an RTO starting at `completed_at`. The clock runs only after
/// complete transport acceptance and only for CON. It must not block.
///
/// This helper does not install a Call, retire a deferred response, or change
/// replay/Observe/body authority. Its caller owns those transaction effects and
/// must treat errors after transmission as uncertain application execution.
pub(crate) fn finish_async<'a, S, T, C>(
    engine: &'a mut Engine<S>,
    io: &'a mut T,
    tx: SlotId,
    dest: Endpoint,
    pending: Option<(MessageId, u32)>,
    expected: Option<PendingCon>,
    completed_at: impl FnOnce() -> Result<u64, C> + 'a,
) -> impl core::future::Future<Output = Result<(), AsyncSendError<T::Error, C>>> + 'a
where
    S: Storage + DatagramSlots + PendingCons,
    T: AsyncDatagramIo,
{
    let owner = SendOwner::prepare(engine, tx, dest, pending.map(|(mid, _)| mid), expected);
    async move {
        let mut owner = owner.map_err(AsyncSendError::Prepare)?;
        owner
            .engine
            .send_tx_async(io, tx)
            .await
            .map_err(AsyncSendError::Transport)?;
        if let Some((_, jitter)) = pending {
            let now_ms = completed_at().map_err(AsyncSendError::ClockAfterSend)?;
            if !owner.commit(now_ms, jitter) {
                return Err(AsyncSendError::CommitAfterSend);
            }
        }
        Ok(())
    }
}

// Protocol traversal supplies a transport wrapper that samples its completion
// clock on each accepted send. Ownership begins at construction, just as for
// public request sending; dropping before the first poll also releases TX.
pub(crate) fn finish_protocol<'a, S, T>(
    engine: &'a mut Engine<S>,
    io: &'a mut T,
    tx: SlotId,
    dest: Endpoint,
    pending: Option<(u64, MessageId, u32)>,
) -> impl core::future::Future<Output = Result<(), Error<T::Error>>> + 'a
where
    S: Storage + DatagramSlots + PendingCons,
    T: super::async_server::ProtocolIo,
{
    let owner = SendOwner::prepare(engine, tx, dest, pending.map(|(_, mid, _)| mid), None);
    async move {
        let mut owner = owner?;
        owner.engine.send_tx_async(io, tx).await?;
        if let Some((_, _, jitter)) = pending
            && !owner.commit(io.completed_at(), jitter)
        {
            return Err(Error::CommitAfterSend);
        }
        if let Some(deadline) = owner
            .engine
            .pending_con(tx)
            .and_then(|pending| pending.rto().next_timeout_ms())
        {
            io.limit_deadline(deadline);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Code, Message, Type};
    use crate::storage::{EngineBuilder, Memory, profiles};
    use core::{
        cell::Cell,
        future::{Future, poll_fn},
        pin::pin,
        task::{Context, Poll, Waker},
    };

    const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
    const MID: MessageId = MessageId::new(801);

    struct Io<'a> {
        ready: &'a Cell<bool>,
        entered: &'a Cell<usize>,
        dropped: &'a Cell<bool>,
        address: usize,
        result: Result<Option<usize>, ()>,
    }

    struct BorrowedIo<'a>(&'a Cell<bool>);
    impl Drop for BorrowedIo<'_> {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    impl crate::storage::DatagramTransport for Io<'_> {
        type Error = ();
    }

    impl AsyncDatagramIo for Io<'_> {
        async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            panic!("sender must not receive")
        }

        async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            assert_eq!(peer, PEER);
            assert_eq!(bytes.as_ptr() as usize, self.address);
            self.entered.set(self.entered.get() + 1);
            let _borrow = BorrowedIo(self.dropped);
            poll_fn(|cx| {
                if self.ready.get() {
                    Poll::Ready(self.result.map(|len| len.unwrap_or(bytes.len())))
                } else {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await
        }
    }

    fn packet(ty: Type) -> (Engine<Memory<profiles::Constrained>>, SlotId, usize) {
        let mut engine = EngineBuilder::new()
            .profile::<profiles::Constrained>()
            .block_wise(false)
            .build(Memory::<profiles::Constrained>::new())
            .unwrap();
        let tx = engine.acquire_tx().unwrap();
        engine
            .encode_tx(tx, &Message::new(ty, Code::GET, MID))
            .unwrap();
        let address = engine.access_tx(tx).unwrap().as_bytes().as_ptr() as usize;
        (engine, tx, address)
    }

    #[test]
    fn dropping_unpolled_send_releases_existing_slot_without_io() {
        let (mut engine, tx, _) = packet(Type::Confirmable);
        let address = engine.access_tx(tx).unwrap().as_bytes().as_ptr() as usize;
        let ready = Cell::new(false);
        let entered = Cell::new(0);
        let dropped = Cell::new(false);
        let mut io = Io {
            ready: &ready,
            entered: &entered,
            dropped: &dropped,
            address,
            result: Ok(None),
        };
        drop(finish_async(
            &mut engine,
            &mut io,
            tx,
            PEER,
            Some((MID, 0)),
            None,
            || Ok::<_, ()>(1),
        ));
        assert_eq!(entered.get(), 0);
        assert!(!dropped.get());
        assert_eq!(engine.tx_occupied(), 0);
        assert!(engine.pending_con(tx).is_none());
    }

    #[test]
    fn dropping_pending_send_drops_transport_before_releasing_tx() {
        for ty in [Type::Confirmable, Type::NonConfirmable] {
            let (mut engine, tx, _) = packet(ty);
            let address = engine.access_tx(tx).unwrap().as_bytes().as_ptr() as usize;
            let ready = Cell::new(false);
            let entered = Cell::new(0);
            let dropped = Cell::new(false);
            let mut io = Io {
                ready: &ready,
                entered: &entered,
                dropped: &dropped,
                address,
                result: Ok(None),
            };
            {
                let pending = (ty == Type::Confirmable).then_some((MID, 0));
                let mut send = pin!(finish_async(
                    &mut engine,
                    &mut io,
                    tx,
                    PEER,
                    pending,
                    None,
                    || Ok::<_, ()>(1)
                ));
                assert!(
                    send.as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
            }
            assert_eq!(entered.get(), 1);
            assert!(dropped.get());
            assert_eq!(engine.tx_occupied(), 0);
            assert!(engine.pending_con(tx).is_none());
        }
    }

    #[test]
    fn accepted_con_starts_at_completion_and_non_releases_without_clock() {
        for ty in [Type::Confirmable, Type::NonConfirmable] {
            let (mut engine, tx, _) = packet(ty);
            let address = engine.access_tx(tx).unwrap().as_bytes().as_ptr() as usize;
            let ready = Cell::new(false);
            let entered = Cell::new(0);
            let dropped = Cell::new(false);
            let clocks = Cell::new(0);
            let mut io = Io {
                ready: &ready,
                entered: &entered,
                dropped: &dropped,
                address,
                result: Ok(None),
            };
            {
                let pending = (ty == Type::Confirmable).then_some((MID, 7));
                let mut send = pin!(finish_async(
                    &mut engine,
                    &mut io,
                    tx,
                    PEER,
                    pending,
                    None,
                    || {
                        clocks.set(clocks.get() + 1);
                        Ok::<_, ()>(500)
                    }
                ));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(send.as_mut().poll(&mut cx).is_pending());
                assert_eq!(clocks.get(), 0);
                ready.set(true);
                assert!(matches!(send.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
            }
            assert!(dropped.get());
            if ty == Type::Confirmable {
                assert_eq!(clocks.get(), 1);
                assert_eq!(engine.tx_occupied(), 1);
                let pending = engine.pending_con(tx).unwrap();
                assert!(pending.rto().is_started());
                assert!(pending.rto().next_timeout_ms().unwrap() > 500);
                assert_eq!(engine.decode_tx(tx).unwrap().message_id(), MID);
            } else {
                assert_eq!(clocks.get(), 0);
                assert_eq!(engine.tx_occupied(), 0);
            }
        }
    }

    #[test]
    fn length_and_io_failures_release_without_sampling_clock() {
        for result in [Err(()), Ok(Some(0)), Ok(Some(99))] {
            let (mut engine, tx, _) = packet(Type::Confirmable);
            let address = engine.access_tx(tx).unwrap().as_bytes().as_ptr() as usize;
            let ready = Cell::new(true);
            let entered = Cell::new(0);
            let dropped = Cell::new(false);
            let mut io = Io {
                ready: &ready,
                entered: &entered,
                dropped: &dropped,
                address,
                result,
            };
            {
                let mut send = pin!(finish_async(
                    &mut engine,
                    &mut io,
                    tx,
                    PEER,
                    Some((MID, 0)),
                    None,
                    || -> Result<u64, ()> { panic!("clock must not run after failed send") }
                ));
                assert!(matches!(
                    send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Ready(Err(AsyncSendError::Transport(_)))
                ));
            }
            assert_eq!(engine.tx_occupied(), 0);
            assert!(engine.pending_con(tx).is_none());
        }
    }

    #[test]
    fn clock_after_acceptance_error_is_explicit_and_does_not_retry() {
        let (mut engine, tx, _) = packet(Type::Confirmable);
        let address = engine.access_tx(tx).unwrap().as_bytes().as_ptr() as usize;
        let ready = Cell::new(true);
        let entered = Cell::new(0);
        let dropped = Cell::new(false);
        let mut io = Io {
            ready: &ready,
            entered: &entered,
            dropped: &dropped,
            address,
            result: Ok(None),
        };
        {
            let mut send = pin!(finish_async(
                &mut engine,
                &mut io,
                tx,
                PEER,
                Some((MID, 0)),
                None,
                || Err(42)
            ));
            assert!(matches!(
                send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Err(AsyncSendError::ClockAfterSend(42)))
            ));
        }
        assert_eq!(entered.get(), 1);
        assert_eq!(engine.tx_occupied(), 0);
        assert!(engine.pending_con(tx).is_none());
    }
}
