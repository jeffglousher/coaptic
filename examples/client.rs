//! README client; explicitly unprotected on localhost.

#[cfg(any(unix, windows))]
#[path = "support/udp_wait.rs"]
mod udp_wait;

#[path = "support/host_error.rs"]
pub mod host_error;

use coaptic::storage::UdpSocketIo;
use coaptic::{App, Endpoint};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

fn main() -> Result<(), host_error::Error> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .full_responses()
        .allow_unprotected()
        .bind(UdpSocketIo::new(socket, [0; 1473])?)?;

    let peer = Endpoint::v4([127, 0, 0, 1], 5683);
    let clock = Instant::now();
    let call = client.get("sensors/temp").to(peer).send(0)?;
    loop {
        let now =
            u64::try_from(clock.elapsed().as_millis()).map_err(|_| host_error::Error::Clock)?;
        client.poll(now)?;
        if let Some(outcome) = client.take_response_with(call, |reply| {
            if !reply.code().is_success() {
                return Err(host_error::Error::UnexpectedReply);
            }
            println!("{}", core::str::from_utf8(reply.payload())?);
            Ok(())
        })? {
            return outcome?;
        }
        if clock.elapsed() >= Duration::from_secs(5) {
            return Err(host_error::Error::Timeout);
        }
        #[cfg(any(unix, windows))]
        {
            let due = client.next_poll_at(now).unwrap_or(5000).min(5000);
            udp_wait::wait([client.transport().socket()], clock, Some(due))?;
        }
        #[cfg(not(any(unix, windows)))]
        std::thread::sleep(Duration::from_millis(1));
    }
}
