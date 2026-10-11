//! Caller-owned platform adapters, available without `std` or an allocator.
//!
//! [`CheckedClock`] validates a fallible platform clock before its timestamp is
//! passed to App. Transport remains [`crate::storage::DatagramIo`]; secure
//! entropy remains [`crate::app::RandomSource`]. No platform SDK is required.

/// A clock reading failed or moved behind the last accepted reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockError<E> {
    /// The platform could not provide a timestamp (including conversion overflow).
    Source(E),
    /// The timestamp regressed. Neither value is substituted for the reading.
    Regressed {
        /// Last successfully accepted millisecond timestamp.
        previous_ms: u64,
        /// Refused millisecond timestamp.
        observed_ms: u64,
    },
}

impl<E: core::fmt::Display> core::fmt::Display for ClockError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Source(error) => write!(f, "platform clock: {error}"),
            Self::Regressed {
                previous_ms,
                observed_ms,
            } => {
                write!(
                    f,
                    "platform clock regressed from {previous_ms} to {observed_ms} ms"
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl<E: std::error::Error + 'static> std::error::Error for ClockError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Regressed { .. } => None,
        }
    }
}

/// Validate a caller-owned millisecond clock without allocating or sleeping.
///
/// The callback may borrow mutable driver state. It must return monotonic elapsed
/// milliseconds from one epoch and finish in bounded time. Extend wrapping
/// hardware counters in the platform adapter; a wrapped value is a regression.
/// Repeated timestamps are accepted. Failure does not lower the high-water mark;
/// recovery requires a reading at least as large as the last accepted one.
///
/// Use the same clock for every timed operation on an App (send, poll, completion,
/// checkpoint dispatch). Propagate errors **before** calling that operation. App's
/// raw timestamp methods remain available and do not enforce use of this guard.
/// Do not replace the guard or reset its epoch while timed App state survives.
/// A stopped clock or a forward jump cannot be detected by monotonicity alone;
/// this is not hardware clock qualification or a persistent security clock.
///
/// ```
/// use coaptic::platform::CheckedClock;
/// // Replace with a fallible platform timer read, not wall-clock/calendar time.
/// let mut ticks = 0_u64;
/// let mut clock = CheckedClock::new(|| {
///     ticks += 1;
///     Ok::<_, core::convert::Infallible>(ticks)
/// });
/// let now_ms = clock.now_ms().unwrap();
/// assert_eq!(now_ms, 1);
/// // app.poll(now_ms)?;
/// ```
pub struct CheckedClock<C> {
    source: C,
    previous_ms: Option<u64>,
}

impl<C> CheckedClock<C> {
    /// Store the platform callback without reading it or allocating.
    pub const fn new(source: C) -> Self {
        Self {
            source,
            previous_ms: None,
        }
    }

    /// Read once and accept only a successful, nondecreasing timestamp.
    ///
    /// An error preserves the last accepted timestamp. The callback's own state
    /// may change on any read; that state remains the platform's responsibility.
    pub fn now_ms<E>(&mut self) -> Result<u64, ClockError<E>>
    where
        C: FnMut() -> Result<u64, E>,
    {
        let observed_ms = (self.source)().map_err(ClockError::Source)?;
        if let Some(previous_ms) = self.previous_ms {
            if observed_ms < previous_ms {
                return Err(ClockError::Regressed {
                    previous_ms,
                    observed_ms,
                });
            }
        }
        self.previous_ms = Some(observed_ms);
        Ok(observed_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_clock_preserves_high_water_mark_across_failure_and_regression() {
        let mut samples = [
            Err("offline"),
            Ok(10),
            Ok(10),
            Err("offline"),
            Ok(9),
            Ok(8),
            Ok(10),
            Ok(11),
        ]
        .into_iter();
        let mut clock = CheckedClock::new(|| samples.next().unwrap());
        assert_eq!(clock.now_ms(), Err(ClockError::Source("offline")));
        assert_eq!(clock.now_ms(), Ok(10));
        assert_eq!(clock.now_ms(), Ok(10));
        assert_eq!(clock.now_ms(), Err(ClockError::Source("offline")));
        for observed_ms in [9, 8] {
            assert_eq!(
                clock.now_ms(),
                Err(ClockError::Regressed {
                    previous_ms: 10,
                    observed_ms
                })
            );
        }
        assert_eq!(clock.now_ms(), Ok(10));
        assert_eq!(clock.now_ms(), Ok(11));
        assert_eq!(samples.next(), None);
    }

    #[test]
    fn checked_clock_accepts_full_range_and_refuses_wrap_and_conversion_overflow() {
        let mut samples = [
            0,
            u128::from(u64::MAX),
            u128::from(u64::MAX) + 1,
            0,
            u128::from(u64::MAX),
        ]
        .into_iter();
        let mut clock = CheckedClock::new(|| u64::try_from(samples.next().unwrap()));
        assert_eq!(clock.now_ms(), Ok(0));
        assert_eq!(clock.now_ms(), Ok(u64::MAX));
        assert!(matches!(clock.now_ms(), Err(ClockError::Source(_))));
        assert_eq!(
            clock.now_ms(),
            Err(ClockError::Regressed {
                previous_ms: u64::MAX,
                observed_ms: 0
            })
        );
        assert_eq!(clock.now_ms(), Ok(u64::MAX));
    }
}
