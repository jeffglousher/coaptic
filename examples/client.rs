//! README client; explicitly plaintext on localhost.

#[path = "support/host_error.rs"]
pub mod host_error;

use coaptic::storage::UdpSocketIo;
use coaptic::{App, Endpoint};
use std::{
    net::UdpSocket,
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), host_error::Error> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .full_responses()
        .allow_plaintext()
        .bind(UdpSocketIo::new(socket, [0; 1473])?)?;

    let peer = Endpoint::v4([127, 0, 0, 1], 5683);
    let clock = Instant::now();
    let call = client.get("sensors/temp").to(peer).send(0)?;
    let mut body = [0; 1472];
    loop {
        client.poll(
            u64::try_from(clock.elapsed().as_millis()).map_err(|_| host_error::Error::Clock)?,
        )?;
        if let Some(reply) = client.take_response_into(call, &mut body)? {
            let reply = reply?;
            if !reply.code().is_success() {
                return Err(host_error::Error::UnexpectedReply);
            }
            println!("{}", core::str::from_utf8(reply.payload())?);
            return Ok(());
        }
        if clock.elapsed() >= Duration::from_secs(5) {
            return Err(host_error::Error::Timeout);
        }
        thread::sleep(Duration::from_millis(1));
    }
}
