//! Tokio adapters for Coaptic's standard-future async App APIs.
//!
//! The caller owns and drives the Tokio runtime, enabling its I/O and time
//! drivers. This crate creates no runtime, thread, task, or packet queue.
//! Tokio's reactor and executor allocate independently of Coaptic's bounded
//! pools. Receive scratch is supplied once by the caller and reused.

use coaptic::{Endpoint, storage::AsyncDatagramIo};
use std::{io, time::Duration};
use tokio::net::UdpSocket;

/// A monotonic epoch shared by App timestamps and absolute Tokio timers.
///
/// Copy this clock for timers belonging to the same App. Poll timers on the
/// caller's runtime with its time driver enabled. Tokio's clock also respects
/// the runtime's paused-time support when that optional feature is used.
#[derive(Clone, Copy)]
pub struct Clock(tokio::time::Instant);

impl Default for Clock {
    fn default() -> Self {
        Self(tokio::time::Instant::now())
    }
}

impl Clock {
    /// Milliseconds since this epoch, refusing integer conversion overflow.
    pub fn now_ms(self) -> io::Result<u64> {
        u64::try_from(self.0.elapsed().as_millis()).map_err(|_| io::ErrorKind::InvalidData.into())
    }
}

impl coaptic::platform::AsyncTimer for Clock {
    async fn wait_until(&mut self, deadline_ms: u64) {
        if let Some(deadline) = self.0.checked_add(Duration::from_millis(deadline_ms)) {
            tokio::time::sleep_until(deadline).await;
        } else {
            // An unrepresentable absolute deadline cannot expire early.
            core::future::pending::<()>().await;
        }
    }
}

/// Caller-owned Tokio UDP socket and reusable truncation-detection scratch.
///
/// Scratch must hold one byte more than the largest supplied receive buffer.
/// Accepted bytes are copied once into that buffer. Oversized datagrams are
/// consumed and rejected; no truncated prefix is delivered as a valid packet.
pub struct Udp<S> {
    socket: UdpSocket,
    scratch: S,
}

impl<S: AsMut<[u8]>> Udp<S> {
    /// Adopt an existing Tokio socket without rebinding or changing options.
    ///
    /// Keep its originating runtime alive and driven. Empty scratch is refused.
    /// On error both arguments are dropped. Do not receive through another
    /// handle to the same socket while this adapter owns App I/O.
    pub fn new(socket: UdpSocket, mut scratch: S) -> io::Result<Self> {
        if scratch.as_mut().is_empty() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self { socket, scratch })
    }

    /// Register an existing standard socket with the currently entered runtime.
    ///
    /// This sets nonblocking mode, retaining its address and other options.
    /// The caller must enter a Tokio runtime with I/O enabled; no current runtime
    /// returns an error. As with Tokio's `UdpSocket::from_std`, a runtime without
    /// an I/O driver can panic. Prefer [`Self::new`] when registration is already
    /// owned by the caller. On any error the socket and scratch are dropped.
    pub fn from_std(socket: std::net::UdpSocket, scratch: S) -> io::Result<Self> {
        tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
        socket.set_nonblocking(true)?;
        Self::new(UdpSocket::from_std(socket)?, scratch)
    }

    /// Bound local endpoint, including a port selected by the OS.
    pub fn local_endpoint(&self) -> io::Result<Endpoint> {
        self.socket.local_addr().map(Endpoint::from)
    }

    /// Inspect or configure the adopted socket, leaving receives to the adapter.
    #[must_use]
    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    /// Return both owners without rebinding or deregistering the Tokio socket.
    ///
    /// The caller can use Tokio's `into_std` to deregister afterward. That socket
    /// remains nonblocking until the caller explicitly changes its mode.
    #[must_use]
    pub fn into_parts(self) -> (UdpSocket, S) {
        (self.socket, self.scratch)
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
        // Tokio's receive future is cancellation-safe: Pending consumes no
        // datagram. The scratch borrow cannot outlive this future.
        let (len, peer) = self.socket.recv_from(scratch).await?;
        if len > buf.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        buf[..len].copy_from_slice(&scratch[..len]);
        Ok((len, Endpoint::from(peer)))
    }

    async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.socket
            .send_to(bytes, std::net::SocketAddr::from(dest))
            .await
    }
}

#[cfg(test)]
mod tests;
