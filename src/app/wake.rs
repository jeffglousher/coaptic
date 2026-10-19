//! Sleep registration over existing ingress slots and protocol deadlines.

use super::*;
use crate::{platform::AsyncTimer, storage::AsyncDatagramIo};
use core::{future::Future, pin::pin, task::Poll};

/// The reason an App can make another bounded progress pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppWake {
    /// Previously retained ingress or protocol work is already eligible.
    Ready,
    /// One complete datagram now occupies the existing RX pool.
    Ingress,
    /// A protocol deadline became due.
    Deadline,
    /// Caller-owned work, such as a command or deferred completion, became ready.
    Local,
}

impl<
    P,
    T: AsyncDatagramIo,
    const N: usize,
    const BLOCK_WISE: bool,
    S: AppStorage<Token = P::Token>,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    #[cfg(feature = "edhoc")]
    pub(crate) async fn receive_async(&mut self) -> Result<SlotId, Error<T::Error>> {
        self.engine
            .recv_from_async(&mut self.io)
            .await
            .map_err(Error::Io)
    }

    /// Await ingress, the earliest protocol deadline, or caller-owned local work.
    ///
    /// Existing eligible work returns immediately without receiving another
    /// packet. Otherwise this races one receive directly into an existing RX
    /// slot against one timer registration and `local`. Local work and deadlines
    /// take precedence over new ingress, so a continuously readable socket cannot
    /// starve them. No payload buffer, packet queue or boxed future is added.
    ///
    /// Dropping the wait cancels receive and releases its reserved RX slot before
    /// the App can be used again. A winning receive retains its slot for the next
    /// protocol pass. Losing futures are dropped before returning; integrations
    /// must unregister their stale wakers as documented by their traits.
    ///
    /// This is a sleep boundary, not protocol dispatch or send completion. Run
    /// a bounded protocol pass after waking and sample the clock again. Callers
    /// must persist local work independently of its wake future; repeated local
    /// readiness must be consumed before waiting again. Use
    /// `core::future::pending()` when there is no external local-work source.
    pub async fn wait_async(
        &mut self,
        now_ms: u64,
        timer: &mut impl AsyncTimer,
        local: impl Future<Output = ()>,
    ) -> Result<AppWake, Error<T::Error>> {
        let deadline = self.next_poll_at(now_ms);
        if deadline.is_some_and(|deadline| deadline <= now_ms) {
            return Ok(AppWake::Ready);
        }
        let mut local = pin!(local);
        let mut timeout = pin!(async {
            match deadline {
                Some(deadline) => timer.wait_until(deadline).await,
                None => core::future::pending().await,
            }
        });
        let mut receive = pin!(self.engine.recv_from_async(&mut self.io));
        core::future::poll_fn(|cx| {
            if local.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Ok(AppWake::Local));
            }
            if timeout.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Ok(AppWake::Deadline));
            }
            match receive.as_mut().poll(cx) {
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(AppWake::Ingress)),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error.into())),
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DatagramTransport;
    use core::{
        cell::Cell,
        future::pending,
        task::{Context, Waker},
    };

    struct Io<'a> {
        ready: &'a Cell<bool>,
        polls: &'a Cell<usize>,
        cancelled: &'a Cell<usize>,
    }

    struct Cancel<'a>(&'a Cell<usize>);
    impl Drop for Cancel<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    impl DatagramTransport for Io<'_> {
        type Error = ();
    }
    impl AsyncDatagramIo for Io<'_> {
        async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            let _registration = Cancel(self.cancelled);
            core::future::poll_fn(|_| {
                self.polls.set(self.polls.get() + 1);
                if !self.ready.get() {
                    return Poll::Pending;
                }
                bytes[..4].copy_from_slice(&[0x40, 1, 0, 1]);
                Poll::Ready(Ok((4, Endpoint::v4([192, 0, 2, 1], 5683))))
            })
            .await
        }
        async fn send(&mut self, _: Endpoint, _: &[u8]) -> Result<usize, ()> {
            unreachable!()
        }
    }

    struct Timer<'a> {
        ready: &'a Cell<bool>,
        due: &'a Cell<Option<u64>>,
        cancelled: &'a Cell<usize>,
    }
    impl AsyncTimer for Timer<'_> {
        async fn wait_until(&mut self, due: u64) {
            self.due.set(Some(due));
            let _registration = Cancel(self.cancelled);
            core::future::poll_fn(|_| {
                if self.ready.get() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
        }
    }

    #[test]
    fn idle_wait_cancels_receive_and_reclaims_reservation() {
        let ready = Cell::new(false);
        let polls = Cell::new(0);
        let cancelled = Cell::new(0);
        let due = Cell::new(None);
        let timer_cancelled = Cell::new(0);
        let mut timer = Timer {
            ready: &ready,
            due: &due,
            cancelled: &timer_cancelled,
        };
        let mut app = App::profile::<crate::profiles::Constrained>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .allow_unprotected()
            .bind(Io {
                ready: &ready,
                polls: &polls,
                cancelled: &cancelled,
            })
            .unwrap();
        {
            let mut wait = pin!(app.wait_async(0, &mut timer, pending()));
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            assert_eq!(polls.get(), 1);
            assert_eq!(due.get(), None);
        }
        assert_eq!(cancelled.get(), 1);
        assert_eq!(app.engine.rx_occupied(), 0);
        assert_eq!(timer_cancelled.get(), 0);
    }

    #[test]
    fn deadline_wins_over_continuous_ingress_and_unregisters_receive() {
        let ready = Cell::new(false);
        let timer_ready = Cell::new(false);
        let polls = Cell::new(0);
        let cancelled = Cell::new(0);
        let due = Cell::new(None);
        let timer_cancelled = Cell::new(0);
        let mut timer = Timer {
            ready: &timer_ready,
            due: &due,
            cancelled: &timer_cancelled,
        };
        let mut app = App::profile::<crate::profiles::Constrained>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .allow_unprotected()
            .bind(Io {
                ready: &ready,
                polls: &polls,
                cancelled: &cancelled,
            })
            .unwrap();
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let tx = app.engine.acquire_tx().unwrap();
        app.engine
            .encode_tx(
                tx,
                &Message::new(Type::Confirmable, Code::GET, MessageId::new(4)),
            )
            .unwrap();
        app.engine
            .record_pending_con(tx, peer, MessageId::new(4), 10, 0)
            .unwrap();
        let deadline = app.next_timeout().unwrap();
        {
            let mut wait = pin!(app.wait_async(10, &mut timer, pending()));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(wait.as_mut().poll(&mut cx).is_pending());
            assert_eq!(due.get(), Some(deadline));
            ready.set(true);
            timer_ready.set(true);
            assert!(matches!(
                wait.as_mut().poll(&mut cx),
                Poll::Ready(Ok(AppWake::Deadline))
            ));
        }
        assert_eq!(polls.get(), 1);
        assert_eq!(cancelled.get(), 1);
        assert_eq!(timer_cancelled.get(), 1);
        assert_eq!(app.engine.rx_occupied(), 0);
    }

    #[test]
    fn ingress_retained_and_ready_work_does_not_receive_again() {
        let ready = Cell::new(true);
        let polls = Cell::new(0);
        let cancelled = Cell::new(0);
        let due = Cell::new(None);
        let timer_cancelled = Cell::new(0);
        let mut timer = Timer {
            ready: &ready,
            due: &due,
            cancelled: &timer_cancelled,
        };
        let mut app = App::profile::<crate::profiles::Constrained>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .allow_unprotected()
            .bind(Io {
                ready: &ready,
                polls: &polls,
                cancelled: &cancelled,
            })
            .unwrap();
        for expected in [AppWake::Ingress, AppWake::Ready] {
            let mut wait = pin!(app.wait_async(0, &mut timer, pending()));
            assert!(
                matches!(wait.as_mut().poll(&mut Context::from_waker(Waker::noop())), Poll::Ready(Ok(wake)) if wake == expected)
            );
        }
        assert_eq!(polls.get(), 1);
        assert_eq!(app.engine.rx_occupied(), 1);
    }
}
