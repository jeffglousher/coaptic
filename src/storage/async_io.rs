//! Awaitable transport operations over the engine's existing packet slots.

use super::{
    DatagramIoError, DatagramSlots, Endpoint, Engine, Metrics, PendingCon, PendingCons, SlotError,
    SlotId, Storage,
};

/// Executor-neutral transport for complete datagrams, without `std` or allocation.
///
/// Implementations borrow the caller's packet storage throughout each operation.
/// They must register the current task for useful readiness changes and must not
/// block the executor. No boxed future or staging buffer is required by this trait.
/// Platform task, socket and timer storage remain part of the integration budget.
///
/// Dropping a pending receive must leave no outstanding access to its buffer.
/// A pending send must not have accepted the packet for transmission: commit
/// acceptance by the transport/network stack and return `Ready` in the same poll.
/// This need not wait for physical radio delivery. After an error, transmission
/// may be ambiguous;
/// retry policy belongs to the protocol owner. Implementations must remain
/// memory-safe even if a future is forgotten: Drop alone cannot make DMA into
/// borrowed memory safe. A platform that cannot satisfy this needs its own
/// storage whose lifetime covers hardware access and explicit completion.
///
/// This is the Engine I/O boundary. It does not itself drive App protocol work,
/// start retransmission timers or provide an executor.
#[allow(async_fn_in_trait)]
pub trait AsyncDatagramIo {
    /// Transport-specific failure, independent of a CoAP response code.
    type Error;

    /// Wait for one complete datagram. Reject truncation and oversize packets.
    async fn recv(&mut self, buf: &mut [u8]) -> Result<(usize, Endpoint), Self::Error>;

    /// Wait until one complete packet is sent; success reports `bytes.len()`.
    async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error>;
}

/// Outcome of an initial CON send that could not install an active RTO.
#[derive(Debug, PartialEq, Eq)]
pub enum InitialConSendError<E, C> {
    /// Missing packet/endpoint, invalid encoding, empty message or non-CON type.
    /// The transport was not polled and storage was not changed.
    InvalidPacket,
    /// Existing pending owner, exhausted NSTART or backend reservation refusal.
    /// The transport was not polled and existing ownership was not changed.
    AdmissionRefused,
    /// Transport or packet-length failure. Transmission may be ambiguous;
    /// the reservation is cancelled but exact encoded bytes remain owned.
    Transport(DatagramIoError<E>),
    /// The transport accepted the complete packet, but the completion clock
    /// failed. No active retransmission timer was installed. Do not assume that
    /// the peer did not receive the packet or automatically re-encode it.
    ClockAfterSend(C),
    /// The transport accepted the complete packet, but the backend refused
    /// the reservation commit. No successful timer installation is promised.
    CommitAfterSend,
}

struct InitialReservation<'a, S: Storage + PendingCons> {
    engine: &'a mut Engine<S>,
    expected: Option<PendingCon>,
}

impl<S: Storage + PendingCons> Drop for InitialReservation<'_, S> {
    fn drop(&mut self) {
        if let Some(expected) = self.expected.take() {
            let _ = self.engine.cancel_pending_con(expected);
        }
    }
}

impl<S: Storage + DatagramSlots + PendingCons> Engine<S> {
    /// Reserve and send an initial CON, starting RTO at transport completion.
    ///
    /// Derives the MID from the occupied nonempty CON and uses its existing TX
    /// endpoint, retaining any logical association identity. Only the physical
    /// endpoint crosses the transport boundary. Existing pending ownership
    /// refuses, including an earlier unstarted reservation.
    ///
    /// Reservation occurs on first poll. Dropping an unpolled future has no
    /// effect; cancellation while waiting cancels its reservation. Encoded TX
    /// bytes remain occupied on every outcome. `completed_at` is called once,
    /// only after full transport acceptance, and must use the monotonic clock
    /// domain used for later protocol polling. It must not block.
    ///
    /// Clock/commit errors after sending are explicit: the peer may already
    /// act on the packet, but no active RTO was installed by this operation.
    /// The owner must resolve that state without assuming non-delivery. This
    /// operation does not install App Calls or drive other protocol work.
    pub async fn send_con_async<T: AsyncDatagramIo, C>(
        &mut self,
        io: &mut T,
        id: SlotId,
        jitter_ms: u32,
        completed_at: impl FnOnce() -> Result<u64, C>,
    ) -> Result<PendingCon, InitialConSendError<T::Error, C>> {
        let endpoint = self
            .tx_endpoint(id)
            .ok_or(InitialConSendError::InvalidPacket)?;
        let mid = {
            let packet = self
                .decode_tx(id)
                .map_err(|_| InitialConSendError::InvalidPacket)?;
            if packet.ty() != crate::message::Type::Confirmable || packet.is_empty() {
                return Err(InitialConSendError::InvalidPacket);
            }
            packet.message_id()
        };
        if self.pending_con(id).is_some() {
            return Err(InitialConSendError::AdmissionRefused);
        }
        let expected = self
            .reserve_pending_con(id, endpoint, mid)
            .ok_or(InitialConSendError::AdmissionRefused)?;
        let mut reservation = InitialReservation {
            engine: self,
            expected: Some(expected),
        };
        reservation
            .engine
            .send_tx_async(io, id)
            .await
            .map_err(InitialConSendError::Transport)?;
        let now_ms = completed_at().map_err(InitialConSendError::ClockAfterSend)?;
        let started = reservation
            .engine
            .commit_pending_con(expected, now_ms, jitter_ms)
            .ok_or(InitialConSendError::CommitAfterSend)?;
        reservation.expected = None;
        Ok(started)
    }
}

// Own the reservation across suspension; cancellation and errors release it.
// Exclusive engine borrowing prevents concurrent reuse without another pool.
struct ReceiveReservation<'a, S: Storage> {
    engine: &'a mut Engine<S>,
    id: Option<SlotId>,
}

impl<S: Storage> Drop for ReceiveReservation<'_, S> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self.engine.release_rx(id);
        }
    }
}

impl<S: Storage + DatagramSlots> Engine<S> {
    /// Receive directly into an existing RX slot, awaiting transport readiness.
    ///
    /// Saturation returns before polling the transport. Cancellation releases
    /// the reservation; success transfers the occupied slot to the caller.
    /// The future holds an exclusive engine borrow, not a packet-sized buffer.
    /// It never drives timers or protocol processing.
    pub async fn recv_from_async<T: AsyncDatagramIo>(
        &mut self,
        io: &mut T,
    ) -> Result<SlotId, DatagramIoError<T::Error>> {
        #[cfg(feature = "diagnostics")]
        super::WorkMetrics::add(&mut self.work_metrics_mut().recv_attempts, 1);
        let id = self.acquire_rx().ok_or(DatagramIoError::Saturated)?;
        let mut reservation = ReceiveReservation {
            engine: self,
            id: Some(id),
        };
        let result = match reservation.engine.storage_mut().rx_payload_mut(id) {
            Some(bytes) => io.recv(bytes).await.map_err(DatagramIoError::Io),
            None => Err(DatagramIoError::Slot(SlotError::NotOccupied)),
        };
        let result = result.and_then(|(len, endpoint)| {
            reservation.engine.storage_mut().set_rx_len(id, len)?;
            reservation
                .engine
                .storage_mut()
                .set_rx_endpoint(id, endpoint.physical())?;
            Ok(len)
        });
        match result {
            Ok(_len) => {
                Metrics::inc(&mut reservation.engine.metrics_mut().rx_accepted);
                #[cfg(feature = "diagnostics")]
                super::WorkMetrics::add(
                    &mut reservation.engine.work_metrics_mut().rx_wire_bytes,
                    _len,
                );
                reservation.id = None;
                Ok(id)
            }
            Err(error) => {
                Metrics::inc(&mut reservation.engine.metrics_mut().rx_error);
                Err(error)
            }
        }
    }

    /// Send directly from an occupied TX slot, retaining its pin across await.
    ///
    /// Cancellation unpins but does not release or alter the slot. The owner
    /// retains the exact encoded packet, including ciphertext, for retry or
    /// explicit release. No retry counter or protocol timer is advanced here.
    /// Success, failure and cancellation all preserve slot ownership.
    pub async fn send_tx_async<T: AsyncDatagramIo>(
        &mut self,
        io: &mut T,
        id: SlotId,
    ) -> Result<usize, DatagramIoError<T::Error>> {
        let result = match self.tx_endpoint(id) {
            None => Err(DatagramIoError::Slot(SlotError::NotOccupied)),
            Some(dest) => match self.access_tx(id) {
                Err(error) => Err(DatagramIoError::Slot(error)),
                Ok(access) => {
                    let bytes = access.as_bytes();
                    io.send(dest.physical(), bytes)
                        .await
                        .map_err(DatagramIoError::Io)
                        .and_then(|actual| {
                            if actual == bytes.len() {
                                Ok(actual)
                            } else {
                                Err(DatagramIoError::SendLength {
                                    expected: bytes.len(),
                                    actual,
                                })
                            }
                        })
                }
            },
        };
        match result {
            Ok(len) => {
                Metrics::inc(&mut self.metrics_mut().tx_ok);
                #[cfg(feature = "diagnostics")]
                super::WorkMetrics::add(&mut self.work_metrics_mut().tx_wire_bytes, len);
                Ok(len)
            }
            Err(error) => {
                Metrics::inc(&mut self.metrics_mut().tx_fail);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{EngineBuilder, Memory, profiles};
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);

    struct Io {
        ready: bool,
        fail: bool,
        short: bool,
        polls: usize,
        sent: usize,
        address: usize,
    }
    impl Io {
        fn new() -> Self {
            Self {
                ready: false,
                fail: false,
                short: false,
                polls: 0,
                sent: 0,
                address: 0,
            }
        }
        async fn ready(&mut self) {
            core::future::poll_fn(|cx| {
                self.polls += 1;
                if self.ready {
                    Poll::Ready(())
                } else {
                    self.ready = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await
        }
    }
    impl AsyncDatagramIo for Io {
        type Error = ();
        async fn recv(&mut self, buf: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            self.address = buf.as_ptr() as usize;
            self.ready().await;
            if self.fail {
                return Err(());
            }
            if self.short {
                return Ok((buf.len() + 1, PEER));
            }
            buf[..4].copy_from_slice(b"wire");
            Ok((4, PEER))
        }
        async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            assert_eq!(dest, PEER);
            assert_eq!(bytes, b"wire");
            self.address = bytes.as_ptr() as usize;
            self.ready().await;
            if self.fail {
                return Err(());
            }
            self.sent += 1;
            Ok(bytes.len() - usize::from(self.short))
        }
    }
    fn engine() -> Engine<Memory<profiles::Constrained>> {
        EngineBuilder::new()
            .profile::<profiles::Constrained>()
            .block_wise(false)
            .build(Memory::<profiles::Constrained>::new())
            .unwrap()
    }
    fn poll<F: Future>(future: core::pin::Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    const CON: &[u8] = &[0x40, 0x01, 0x12, 0x34];

    struct ConIo(Io);

    impl AsyncDatagramIo for ConIo {
        type Error = ();

        async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            unreachable!("send-only fixture")
        }

        async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            assert_eq!(dest, PEER);
            assert_eq!(bytes, CON);
            self.0.ready().await;
            if self.0.fail {
                return Err(());
            }
            self.0.sent += 1;
            Ok(bytes.len() - usize::from(self.0.short))
        }
    }

    #[test]
    fn initial_con_unpolled_and_pending_cancellation_preserve_packet() {
        let mut engine = engine();
        let tx = engine.acquire_tx().unwrap();
        engine.write_tx(tx, CON, PEER).unwrap();
        let mut io = ConIo(Io::new());
        drop(engine.send_con_async(&mut io, tx, 0, || Ok::<_, ()>(100)));
        assert_eq!(io.0.polls, 0);
        assert_eq!(engine.pending_con(tx), None);
        {
            let mut operation = pin!(engine.send_con_async(
                &mut io,
                tx,
                0,
                || -> Result<u64, ()> { panic!("clock called before acceptance") }
            ));
            assert!(poll(operation.as_mut()).is_pending());
        }
        assert_eq!(io.0.sent, 0);
        assert_eq!(engine.pending_con(tx), None);
        assert_eq!(engine.access_tx(tx).unwrap().as_bytes(), CON);
        assert_eq!(engine.tx_occupied(), 1);
    }

    #[test]
    fn initial_con_transport_failure_cancels_without_clock_or_reencoding() {
        let mut engine = engine();
        let tx = engine.acquire_tx().unwrap();
        engine.write_tx(tx, CON, PEER).unwrap();
        let mut io = ConIo(Io::new());
        io.0.ready = true;
        for (fail, short) in [(true, false), (false, true)] {
            io.0.fail = fail;
            io.0.short = short;
            {
                let mut operation = pin!(engine.send_con_async(
                    &mut io,
                    tx,
                    0,
                    || -> Result<u64, ()> { panic!("clock called after failed transport") }
                ));
                assert!(matches!(
                    poll(operation.as_mut()),
                    Poll::Ready(Err(InitialConSendError::Transport(_)))
                ));
            }
            assert_eq!(engine.pending_con(tx), None);
            assert_eq!(engine.access_tx(tx).unwrap().as_bytes(), CON);
        }
    }

    #[test]
    fn initial_con_timer_uses_completion_clock_and_preserves_existing_owner() {
        let mut engine = engine();
        let tx = engine.acquire_tx().unwrap();
        engine.write_tx(tx, CON, PEER).unwrap();
        let mut io = ConIo(Io::new());
        let now = core::cell::Cell::new(10);
        let calls = core::cell::Cell::new(0);
        let started = {
            let mut operation = pin!(engine.send_con_async(&mut io, tx, 7, || {
                calls.set(calls.get() + 1);
                Ok::<_, ()>(now.get())
            }));
            assert!(poll(operation.as_mut()).is_pending());
            assert_eq!(calls.get(), 0);
            now.set(9000);
            match poll(operation.as_mut()) {
                Poll::Ready(Ok(started)) => started,
                _ => panic!("accepted CON"),
            }
        };
        assert_eq!(calls.get(), 1);
        assert!(started.rto().is_started());
        assert_eq!(started.rto().next_timeout_ms(), Some(11007));
        assert_eq!(engine.pending_con(tx), Some(started));
        let polls = io.0.polls;
        {
            let mut operation = pin!(engine.send_con_async(&mut io, tx, 0, || Ok::<_, ()>(0)));
            assert_eq!(
                poll(operation.as_mut()),
                Poll::Ready(Err(InitialConSendError::AdmissionRefused))
            );
        }
        assert_eq!(io.0.polls, polls);
        assert_eq!(engine.pending_con(tx), Some(started));
    }

    #[test]
    fn initial_con_clock_failure_reports_accepted_packet_without_active_timer() {
        let mut engine = engine();
        let tx = engine.acquire_tx().unwrap();
        engine.write_tx(tx, CON, PEER).unwrap();
        let mut io = ConIo(Io::new());
        io.0.ready = true;
        {
            let mut operation = pin!(engine.send_con_async(&mut io, tx, 0, || Err(42)));
            assert_eq!(
                poll(operation.as_mut()),
                Poll::Ready(Err(InitialConSendError::ClockAfterSend(42)))
            );
        }
        assert_eq!(io.0.sent, 1);
        assert_eq!(engine.metrics().tx_ok, 1);
        assert_eq!(engine.pending_con(tx), None);
        assert_eq!(engine.access_tx(tx).unwrap().as_bytes(), CON);
    }

    #[test]
    fn initial_con_invalid_packet_refuses_before_transport() {
        let mut engine = engine();
        let tx = engine.acquire_tx().unwrap();
        let mut io = ConIo(Io::new());
        for bytes in [
            &[0x40, 0x00, 0x12, 0x34][..],
            &[0x50, 0x01, 0x12, 0x34],
            &[],
        ] {
            engine.write_tx(tx, bytes, PEER).unwrap();
            {
                let mut operation = pin!(engine.send_con_async(&mut io, tx, 0, || Ok::<_, ()>(0)));
                assert_eq!(
                    poll(operation.as_mut()),
                    Poll::Ready(Err(InitialConSendError::InvalidPacket))
                );
            }
            assert_eq!(engine.pending_con(tx), None);
        }
        assert_eq!(io.0.polls, 0);
    }

    #[test]
    fn async_receive_cancel_failure_and_saturation_release_without_staging() {
        let mut engine = engine();
        let mut io = Io::new();
        {
            let mut pending = pin!(engine.recv_from_async(&mut io));
            assert!(poll(pending.as_mut()).is_pending());
        }
        assert_eq!(engine.rx_occupied(), 0);
        assert_eq!(engine.metrics().rx_accepted, 0);
        for malformed in [false, true] {
            io.fail = !malformed;
            io.short = malformed;
            let mut operation = pin!(engine.recv_from_async(&mut io));
            assert!(matches!(poll(operation.as_mut()), Poll::Ready(Err(_))));
        }
        assert_eq!(engine.rx_occupied(), 0);
        io.fail = false;
        io.short = false;
        let id = {
            let mut operation = pin!(engine.recv_from_async(&mut io));
            match poll(operation.as_mut()) {
                Poll::Ready(Ok(id)) => id,
                _ => panic!("ready receive"),
            }
        };
        {
            let bytes = engine.access_rx(id).unwrap();
            assert_eq!(bytes.as_bytes(), b"wire");
            assert_eq!(bytes.as_bytes().as_ptr() as usize, io.address);
        }
        assert_eq!(engine.metrics().rx_accepted, 1);
        while engine.acquire_rx().is_some() {}
        let polls = io.polls;
        {
            let mut operation = pin!(engine.recv_from_async(&mut io));
            assert_eq!(
                poll(operation.as_mut()),
                Poll::Ready(Err(DatagramIoError::Saturated))
            );
        }
        assert_eq!(io.polls, polls);
    }

    #[test]
    fn async_send_cancel_preserves_exact_packet_and_unpins_for_retry() {
        let mut engine = engine();
        let id = engine.acquire_tx().unwrap();
        engine.write_tx(id, b"wire", PEER).unwrap();
        let address = engine.access_tx(id).unwrap().as_bytes().as_ptr() as usize;
        let mut io = Io::new();
        {
            let mut pending = pin!(engine.send_tx_async(&mut io, id));
            assert!(poll(pending.as_mut()).is_pending());
        }
        assert_eq!(io.sent, 0);
        assert_eq!(io.address, address);
        assert_eq!(engine.metrics().tx_ok, 0);
        assert_eq!(engine.metrics().tx_fail, 0);
        assert_eq!(engine.access_tx(id).unwrap().as_bytes(), b"wire");
        for (fail, short) in [(true, false), (false, true), (false, false)] {
            io.fail = fail;
            io.short = short;
            {
                let mut operation = pin!(engine.send_tx_async(&mut io, id));
                let result = poll(operation.as_mut());
                if fail || short {
                    assert!(matches!(result, Poll::Ready(Err(_))));
                } else {
                    assert_eq!(result, Poll::Ready(Ok(4)));
                }
            }
            assert_eq!(engine.tx_occupied(), 1);
            assert_eq!(engine.access_tx(id).unwrap().as_bytes(), b"wire");
        }
        assert_eq!(engine.metrics().tx_ok, 1);
        assert_eq!(engine.metrics().tx_fail, 2);
        engine.release_tx(id).unwrap();
        assert_eq!(engine.tx_occupied(), 0);
    }
}
