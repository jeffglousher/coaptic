//! Host UDP and monotonic timer adapters for Coaptic's native async App APIs.
//!
//! The OS readiness reactor uses host allocation. Packet storage stays bounded:
//! the Engine owns its pools and the caller supplies receive scratch. This does
//! The same transport works with Engine I/O and App request/progress/wait APIs.

use async_io::Async;
use coaptic::{Endpoint, storage::AsyncDatagramIo};
use std::{io, net::UdpSocket};

/// Shared monotonic epoch for App timestamps and absolute async timers.
#[derive(Clone, Copy)]
pub struct Clock(std::time::Instant);

impl Default for Clock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock {
    /// Milliseconds since this epoch; refuse conversion overflow.
    pub fn now_ms(self) -> io::Result<u64> {
        u64::try_from(self.0.elapsed().as_millis()).map_err(|_| io::ErrorKind::InvalidData.into())
    }
}

impl coaptic::platform::AsyncTimer for Clock {
    async fn wait_until(&mut self, deadline_ms: u64) {
        if let Some(deadline) = self
            .0
            .checked_add(std::time::Duration::from_millis(deadline_ms))
        {
            async_io::Timer::at(deadline).await;
        } else {
            core::future::pending::<()>().await;
        }
    }
}

/// Readiness-driven UDP with reusable caller-owned truncation-detection scratch.
///
/// Scratch needs one byte more than the largest Engine RX slot. Received bytes
/// are copied once into that slot; there is no packet queue or growing buffer.
pub struct Udp<S> {
    socket: Async<UdpSocket>,
    scratch: S,
}

impl<S: AsMut<[u8]>> Udp<S> {
    /// Register a socket with the host reactor; sets nonblocking mode.
    pub fn new(socket: UdpSocket, mut scratch: S) -> io::Result<Self> {
        if scratch.as_mut().is_empty() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            socket: Async::new(socket)?,
            scratch,
        })
    }

    /// Bound local endpoint, including a port selected by the OS.
    pub fn local_endpoint(&self) -> io::Result<Endpoint> {
        self.socket.get_ref().local_addr().map(Endpoint::from)
    }
}

impl<S: AsMut<[u8]>> coaptic::storage::DatagramTransport for Udp<S> {
    type Error = io::Error;
}

impl<S: AsMut<[u8]>> AsyncDatagramIo for Udp<S> {
    async fn recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, Endpoint)> {
        let required = buf
            .len()
            .checked_add(1)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let scratch = self
            .scratch
            .as_mut()
            .get_mut(..required)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let (len, peer) = self
            .socket
            .read_with(|socket| socket.recv_from(scratch))
            .await?;
        if len > buf.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        buf[..len].copy_from_slice(&scratch[..len]);
        Ok((len, Endpoint::from(peer)))
    }

    async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.socket
            .write_with(|socket| socket.send_to(bytes, std::net::SocketAddr::from(dest)))
            .await
    }
}

#[cfg(test)]
mod app_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::storage::{Engine, EngineBuilder, Memory, MemoryProfile, profiles};
    use core::{future::Future, pin::pin, task::Poll};
    use std::time::Duration;

    type TestEngine = Engine<Memory<profiles::Constrained>>;
    const RX_BYTES: usize = profiles::Constrained::RX_DATAGRAM_BYTES;

    fn engine() -> TestEngine {
        EngineBuilder::new()
            .profile::<profiles::Constrained>()
            .block_wise(false)
            .build(Memory::<profiles::Constrained>::new())
            .unwrap()
    }

    fn socket() -> Udp<[u8; RX_BYTES + 1]> {
        Udp::new(UdpSocket::bind("127.0.0.1:0").unwrap(), [0; RX_BYTES + 1]).unwrap()
    }

    async fn deadline<T>(operation: impl Future<Output = T>) -> T {
        let mut operation = pin!(operation);
        let mut timer = pin!(async_io::Timer::after(Duration::from_secs(5)));
        core::future::poll_fn(|cx| {
            if let Poll::Ready(result) = operation.as_mut().poll(cx) {
                return Poll::Ready(result);
            }
            assert!(
                timer.as_mut().poll(cx).is_pending(),
                "UDP readiness deadline"
            );
            Poll::Pending
        })
        .await
    }

    #[test]
    fn real_udp_pending_receive_wakes_and_preserves_engine_ownership() {
        async_io::block_on(deadline(async {
            let mut sender = socket();
            let mut receiver = socket();
            let destination = receiver.local_endpoint().unwrap();
            let source = sender.local_endpoint().unwrap();
            let mut tx_engine = engine();
            let mut rx_engine = engine();
            let tx = tx_engine.acquire_tx().unwrap();
            tx_engine.write_tx(tx, b"async wire", destination).unwrap();
            let rx = {
                let mut receive = pin!(rx_engine.recv_from_async(&mut receiver));
                core::future::poll_fn(|cx| {
                    assert!(receive.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                })
                .await;
                let mut send = pin!(async {
                    async_io::Timer::after(Duration::from_millis(10)).await;
                    assert_eq!(tx_engine.send_tx_async(&mut sender, tx).await.unwrap(), 10);
                });
                let mut received = None;
                let mut sent = false;
                core::future::poll_fn(|cx| {
                    if received.is_none()
                        && let Poll::Ready(result) = receive.as_mut().poll(cx)
                    {
                        received = Some(result.unwrap());
                    }
                    if !sent && send.as_mut().poll(cx).is_ready() {
                        sent = true;
                    }
                    match (received, sent) {
                        (Some(id), true) => Poll::Ready(id),
                        _ => Poll::Pending,
                    }
                })
                .await
            };
            assert_eq!(rx_engine.rx_endpoint(rx), Some(source));
            assert_eq!(rx_engine.access_rx(rx).unwrap().as_bytes(), b"async wire");
            assert_eq!(tx_engine.tx_occupied(), 1);
            assert_eq!(tx_engine.metrics().tx_ok, 1);
            assert_eq!(rx_engine.metrics().rx_accepted, 1);
            tx_engine.release_tx(tx).unwrap();
            rx_engine.release_rx(rx).unwrap();
        }));
    }

    #[test]
    fn cancelled_receive_and_oversize_refusal_recover_on_same_socket() {
        async_io::block_on(deadline(async {
            let mut sender = socket();
            let mut receiver = socket();
            let destination = receiver.local_endpoint().unwrap();
            let mut engine = engine();
            {
                let mut receive = pin!(engine.recv_from_async(&mut receiver));
                core::future::poll_fn(|cx| {
                    assert!(receive.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                })
                .await;
            }
            assert_eq!(engine.rx_occupied(), 0);
            sender.send(destination, &[0; RX_BYTES + 1]).await.unwrap();
            assert!(engine.recv_from_async(&mut receiver).await.is_err());
            assert_eq!(engine.rx_occupied(), 0);
            sender.send(destination, &[42; RX_BYTES]).await.unwrap();
            let rx = engine.recv_from_async(&mut receiver).await.unwrap();
            assert_eq!(engine.access_rx(rx).unwrap().as_bytes(), &[42; RX_BYTES]);
            engine.release_rx(rx).unwrap();
            assert_eq!(engine.rx_occupied(), 0);
        }));
    }
}
