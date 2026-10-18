use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

trait DatagramIo {
    fn send(&mut self) -> usize;
}
trait AsyncDatagramIo {
    async fn send_async(&mut self) -> usize;
}

struct Transport(usize);
impl DatagramIo for Transport {
    fn send(&mut self) -> usize {
        self.0 += 1;
        self.0
    }
}
impl AsyncDatagramIo for Transport {
    async fn send_async(&mut self) -> usize {
        self.0 += 1;
        self.0
    }
}

#[coaptic_io::dual_io(child_async, methods(send => send_async))]
fn child<T: DatagramIo>(io: &mut T) -> usize {
    io.send()
}

#[coaptic_io::dual_io(parent_async, calls(child => child_async))]
fn parent<T: DatagramIo>(io: &mut T) -> usize {
    let first = child(io);
    first + child(io)
}

#[test]
fn preserves_sync_and_async_commit_order() {
    let mut sync = Transport(0);
    assert_eq!(parent(&mut sync), 3);
    let mut asynchronous = Transport(0);
    let mut future = pin!(parent_async(&mut asynchronous));
    assert_eq!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(3)
    );
}
