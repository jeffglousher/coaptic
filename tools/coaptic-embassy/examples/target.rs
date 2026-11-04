#![no_std]
//! Board-independent entry point: call from the board's Embassy task. The board
//! supplies a bound socket, running network runner, time driver and randomness.
use coaptic::{App, Request, Response, get, resources::SmallDevice};
use coaptic_embassy::{Clock, Udp};

/// Why the example task stopped. The board decides recovery or shutdown.
#[derive(Debug)]
pub enum Failure {
    Bind,
    Clock,
    Wait,
    Progress,
    DeadlineOverflow,
}

/// A compact unprotected server task. Dropping the future closes its socket.
/// The board owns task spawning/shutdown, network resources and security policy.
pub async fn serve(
    socket: embassy_net::udp::UdpSocket<'_>,
    rng: coaptic::app::RandomSource,
) -> Result<(), Failure> {
    fn hello(_: Request<'_>) -> Response<'static> {
        Response::content(b"hello")
    }
    let mut app = App::with_resources(SmallDevice::new())
        .randomness(rng)
        .allow_unprotected()
        .route("hello", get(hello))
        .bind(Udp::new(socket))
        .map_err(|_| Failure::Bind)?;
    let clock = Clock;
    let mut timer = clock;
    loop {
        let now = clock.now_ms().map_err(|_| Failure::Clock)?;
        if app
            .wait_async(now, &mut timer, core::future::pending())
            .await
            .is_err()
        {
            return Err(Failure::Wait);
        }
        let now = clock.now_ms().map_err(|_| Failure::Clock)?;
        let deadline = now.checked_add(1000).ok_or(Failure::DeadlineOverflow)?;
        if app
            .progress_async(now, &mut || clock.now_ms(), &mut timer, deadline)
            .await
            .is_err()
        {
            return Err(Failure::Progress);
        }
    }
}
