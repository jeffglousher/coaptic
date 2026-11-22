use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::{
    io,
    net::UdpSocket,
    time::{Duration, Instant},
};

// Deadlines are absolute milliseconds since start. Winsock accepts an i32 timeout.
fn timeout(start: Instant, due_ms: u64) -> Timespec {
    let remaining = Duration::from_millis(due_ms)
        .saturating_sub(start.elapsed())
        .min(Duration::from_millis(i32::MAX as u64));
    Timespec {
        tv_sec: remaining.as_secs() as i64,
        tv_nsec: remaining.subsec_nanos() as _,
    }
}

pub fn wait(socket: &UdpSocket, start: Instant, due_ms: Option<u64>) -> io::Result<()> {
    let timeout = due_ms.map(|due| timeout(start, due));
    if timeout == Some(Timespec::default()) {
        return Ok(());
    }

    let mut fds = [PollFd::new(socket, PollFlags::RDNORM)];
    match poll(&mut fds, timeout.as_ref()) {
        // App owns receive/error handling. An interrupt also restarts its clock.
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::storage::{DatagramIo, UdpSocketIo};

    #[test]
    fn deadlines_are_absolute_and_bounded() {
        let start = Instant::now();
        assert_eq!(timeout(start, 0), Timespec::default());
        assert_eq!(
            Duration::try_from(timeout(start, u64::MAX)).unwrap(),
            Duration::from_millis(i32::MAX as u64),
        );
    }

    #[test]
    fn idle_socket_waits_until_its_deadline() -> io::Result<()> {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        let start = Instant::now();
        wait(&socket, start, Some(20))?;
        assert!(start.elapsed() >= Duration::from_millis(20));
        Ok(())
    }

    #[test]
    fn readiness_leaves_refusal_and_reuse_to_the_transport() -> io::Result<()> {
        let receiver = UdpSocket::bind("127.0.0.1:0")?;
        receiver.set_nonblocking(true)?;
        let sender = UdpSocket::bind("127.0.0.1:0")?;
        let dest = receiver.local_addr()?;
        let mut transport = UdpSocketIo::new(receiver, [0; 5])?;
        let mut buf = [0; 4];
        for packet in [&[1; 8][..], &[][..], b"read"] {
            sender.send_to(packet, dest)?;
            wait(transport.socket(), Instant::now(), Some(1000))?;
            let result = transport.recv(&mut buf);
            if packet.len() > buf.len() {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            } else {
                let (len, _) = result?.unwrap();
                assert_eq!(&buf[..len], packet);
            }
        }
        assert!(transport.recv(&mut buf)?.is_none());
        Ok(())
    }
}
