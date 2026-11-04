#![no_std]
//! Caller-owned Embassy network and time adapters. No executor, heap or extra
//! packet buffer is created here. The caller must drive the network runner and
//! install the target's Embassy time driver.

use coaptic::{
    Endpoint,
    storage::{AsyncDatagramIo, DatagramTransport},
};
use embassy_net::{
    IpAddress, IpEndpoint,
    udp::{RecvError, SendError, UdpSocket},
};

/// Explicit transport failures; no truncation is reported as a complete packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Embassy cannot represent a nonzero IPv6 zone identifier.
    UnsupportedScope,
    /// Native receive failure.
    Receive(RecvError),
    /// Native send failure.
    Send(SendError),
}

fn destination(peer: Endpoint) -> Result<IpEndpoint, Error> {
    if peer.scope_id() != 0 {
        return Err(Error::UnsupportedScope);
    }
    let addr = match peer.as_ipv4() {
        Some((bytes, _)) => IpAddress::Ipv4(bytes.into()),
        None => IpAddress::Ipv6(peer.as_ipv6().expect("endpoint family").0.into()),
    };
    Ok(IpEndpoint::new(addr, peer.port()))
}

fn peer(endpoint: IpEndpoint) -> Endpoint {
    match endpoint.addr {
        IpAddress::Ipv4(ip) => Endpoint::v4(ip.octets(), endpoint.port),
        IpAddress::Ipv6(ip) => Endpoint::v6(ip.octets(), endpoint.port),
    }
}

/// An adopted socket, still backed by the caller's Embassy packet pools.
///
/// The socket belongs to one Embassy stack/interface. IPv6 addresses use scope
/// zero in that interface's namespace; nonzero outbound scopes are refused.
/// Do not share receive access while the App owns this transport.
pub struct Udp<'a>(UdpSocket<'a>);

impl<'a> Udp<'a> {
    /// Adopt a bound socket without changing its buffers or socket options.
    pub fn new(socket: UdpSocket<'a>) -> Self {
        Self(socket)
    }
    /// Inspect or configure the socket without rebinding the adapter.
    pub fn socket(&self) -> &UdpSocket<'a> {
        &self.0
    }
    /// Recover the socket; its original caller-owned buffers remain attached.
    pub fn into_inner(self) -> UdpSocket<'a> {
        self.0
    }
}

impl DatagramTransport for Udp<'_> {
    type Error = Error;
}
impl AsyncDatagramIo for Udp<'_> {
    async fn recv(&mut self, buf: &mut [u8]) -> Result<(usize, Endpoint), Error> {
        // Embassy consumes one complete queued datagram and rejects truncation.
        // A Pending receive stores a waker, not the borrowed packet buffer.
        let (len, metadata) = self.0.recv_from(buf).await.map_err(Error::Receive)?;
        Ok((len, peer(metadata.endpoint)))
    }
    async fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Error> {
        // Native queue acceptance and Ready occur in the same poll. This is
        // network-stack acceptance, not confirmation of physical delivery.
        self.0
            .send_to(bytes, destination(dest)?)
            .await
            .map_err(Error::Send)?;
        Ok(bytes.len())
    }
}

/// Milliseconds in the target's Embassy boot-time domain, shared by all copies.
#[derive(Clone, Copy, Default)]
pub struct Clock;

impl Clock {
    /// Current time, refusing unrepresentable millisecond timestamps.
    pub fn now_ms(self) -> Result<u64, core::num::TryFromIntError> {
        u64::try_from(
            u128::from(embassy_time::Instant::now().as_ticks()) * 1000
                / u128::from(embassy_time::TICK_HZ),
        )
    }
}

fn deadline_ticks(ms: u64) -> Option<u64> {
    // Round up so a low-frequency clock never completes before the deadline.
    u64::try_from((u128::from(ms) * u128::from(embassy_time::TICK_HZ)).div_ceil(1000)).ok()
}

impl coaptic::platform::AsyncTimer for Clock {
    async fn wait_until(&mut self, deadline_ms: u64) {
        match deadline_ticks(deadline_ms) {
            Some(ticks) => embassy_time::Timer::at(embassy_time::Instant::from_ticks(ticks)).await,
            None => core::future::pending().await,
        }
    }
}

#[cfg(test)]
mod tests;
