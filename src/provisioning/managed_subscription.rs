//! Caller-owned desired subscriptions survive a fresh session, never a new grant.
use super::*;

#[derive(Debug)]
enum State {
    New,
    Waiting { anchor: TrustAnchor, retry_at: u64 },
    Active { call: ManagedCall, refresh_at: u64 },
    Stopped,
}

/// One bounded desired GET subscription, owned by the application.
///
/// Recreate from application configuration after a process/device restart. Run
/// the connection reactor, collect responses, then poll the subscription. A fresh EDHOC
/// session under the same trust grant re-registers automatically. A different
/// grant requires a new explicit subscription; no old intent crosses that boundary.
/// This stores no payload and adds no table to `ManagedConnection` or `App`.
#[derive(Debug)]
pub struct ManagedSubscription {
    path: &'static str,
    state: State,
}
impl ManagedSubscription {
    /// Desired static path; syntax and current permission are checked on polling.
    #[must_use]
    pub const fn new(path: &'static str) -> Self {
        Self {
            path,
            state: State::New,
        }
    }
    /// Whether the caller has stopped this desired subscription.
    #[must_use]
    pub const fn is_stopped(&self) -> bool {
        matches!(self.state, State::Stopped)
    }
}

impl<T: DatagramIo, P, const DEFERRED: usize, const CACHES: usize, const ROUTES: usize>
    ManagedConnection<'_, T, P, DEFERRED, CACHES, ROUTES>
where
    P: MemoryProfile + MemoryLayout<false> + AppAssembled<false>,
{
    /// Attempt at most one registration/refresh for caller-owned desired interest.
    ///
    /// Returns true when a request was sent. Failed attempts are spaced by one
    /// second; absent initial responses refresh after 30 seconds. Received Max-Age
    /// selects later refresh deadlines plus RFC 7641 randomized 5 to 15 second delay.
    /// Random-source failure uses the maximum delay. This does not
    /// drive EDHOC or consume responses. Call `poll`/`poll_with` first.
    pub fn poll_subscription(
        &mut self,
        subscription: &mut ManagedSubscription,
        now_ms: u64,
    ) -> Result<bool, ManagedError<T::Error>> {
        self.time(now_ms)?;
        if subscription.is_stopped() {
            return Ok(false);
        }
        self.ensure_grant()?;
        if !self.is_connected() {
            return Ok(false);
        }
        let anchor = self.grant.as_ref().expect("connected grant").anchor();
        let mut refresh = None;
        match subscription.state {
            State::Stopped => return Ok(false),
            State::New => {}
            State::Waiting {
                anchor: original,
                retry_at,
            } => {
                if original != anchor {
                    return Err(ManagedError::StaleHandle);
                }
                if now_ms < retry_at {
                    return Ok(false);
                }
            }
            State::Active { call, refresh_at } => {
                if call.anchor != anchor {
                    return Err(ManagedError::StaleHandle);
                }
                if self.check_handle(call.anchor, call.epoch).is_ok() {
                    if now_ms < refresh_at {
                        return Ok(false);
                    }
                    refresh = Some(call);
                }
            }
        }
        subscription.state = match refresh {
            Some(call) => State::Active {
                call,
                refresh_at: now_ms.saturating_add(1000),
            },
            None => State::Waiting {
                anchor,
                retry_at: now_ms.saturating_add(1000),
            },
        };
        let call = if let Some(call) = refresh {
            if self
                .recovery
                .as_ref()
                .is_some_and(CoapRecovery::has_candidate)
            {
                return Err(ManagedError::Busy);
            }
            let endpoint = self.endpoint;
            let next = self
                .app_mut()?
                .get(subscription.path)
                .to(endpoint)
                .reregister_call(call.call)
                .send(now_ms);
            let next = match next {
                Ok(next) => next,
                Err(error) => {
                    // A failed refresh may have already retired the old Call.
                    // Cancel/collect its local ownership before starting anew.
                    self.cancel(call, now_ms)?;
                    subscription.state = State::Waiting {
                        anchor,
                        retry_at: now_ms.saturating_add(1000),
                    };
                    return Err(ManagedError::Application(error));
                }
            };
            ManagedCall { call: next, ..call }
        } else {
            self.observe(subscription.path, now_ms)?
        };
        subscription.state = State::Active {
            call,
            refresh_at: now_ms.saturating_add(30_000),
        };
        Ok(true)
    }

    /// Collect a notification and schedule freshness recovery without copying it
    /// into subscription state. Terminal/local failures retry after 30 seconds.
    /// An insufficient output buffer leaves the response and schedule unchanged.
    pub fn take_subscription_response_into<'s>(
        &'s mut self,
        subscription: &mut ManagedSubscription,
        output: &'s mut [u8],
        now_ms: u64,
    ) -> Result<Option<Result<Response<'s>, CallFailure>>, ManagedError<T::Error>> {
        let State::Active { call, .. } = subscription.state else {
            return Ok(None);
        };
        let randomness = self.randomness;
        let result = self.take_response_into(call, output, now_ms)?;
        if let Some(ref reply) = result {
            let mut random = [0; 2];
            let delay = if randomness(&mut random) {
                5_000 + (u64::from(u16::from_be_bytes(random)) * 10_001 / 65_536)
            } else {
                15_000
            };
            subscription.state = match reply {
                Ok(response)
                    if response.code().is_success() && response.observe_seq().is_some() =>
                {
                    State::Active {
                        call,
                        refresh_at: now_ms.saturating_add(
                            u64::from(response.max_age_secs().unwrap_or(60))
                                .saturating_mul(1000)
                                .saturating_add(delay),
                        ),
                    }
                }
                _ => State::Waiting {
                    anchor: call.anchor,
                    retry_at: now_ms.saturating_add(30_000),
                },
            };
        }
        Ok(result)
    }

    /// Stop desired interest and cancel its local Call. Subsequent notifications
    /// use normal reactive cancellation. No new registration occurs after this.
    pub fn stop_subscription(
        &mut self,
        subscription: &mut ManagedSubscription,
        now_ms: u64,
    ) -> Result<(), ManagedError<T::Error>> {
        let previous = core::mem::replace(&mut subscription.state, State::Stopped);
        if let State::Active { call, .. } = previous {
            match self.cancel(call, now_ms) {
                Ok(_) | Err(ManagedError::StaleHandle) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}
