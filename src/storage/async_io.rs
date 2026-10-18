//! Awaitable transport operations over the engine's existing packet slots.

use super::{
    DatagramIoError, DatagramSlots, Endpoint, Engine, Metrics, SlotError, SlotId, Storage,
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
