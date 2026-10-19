//! Bounded App identity allocation and retransmission jitter.
use crate::error::BuildError;
use crate::message::{MessageId, OwnedToken, Token, Transmission};

const _: () = assert!(Transmission::ACK_RANDOM_SPAN_MS <= u16::MAX as u32);

/// Fill every requested byte from a cryptographically secure source.
/// Return false on failure; App never substitutes predictable bytes.
/// The caller owns entropy quality, initialization and platform integration.
/// Calls must finish in bounded time and must not re-enter App.
pub type RandomSource = fn(&mut [u8]) -> bool;

/// Failure to obtain a fresh identity or retransmission schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    /// Entropy source failed, or bounded unbiased sampling could not finish.
    RandomnessUnavailable,
    /// The source repeatedly produced an active Token, or test Tokens exhausted.
    TokenExhausted,
    /// The MID space cannot yet be reused within EXCHANGE_LIFETIME.
    MessageIdExhausted,
}
impl core::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::RandomnessUnavailable => "randomness unavailable",
            Self::TokenExhausted => "fresh Token unavailable",
            Self::MessageIdExhausted => "Message ID reuse interval has not elapsed",
        })
    }
}
#[cfg(feature = "std")]
impl std::error::Error for IdentityError {}

#[derive(Clone, Copy)]
pub(super) enum Source {
    Random(RandomSource),
    Io { initial_mid: u16 },
    Test,
}

pub(super) struct AppIds {
    source: Source,
    next_mid: u16,
    issued: u32,
    last_mid_ms: u64,
    jitter: u16,
    test_token: u32,
    token_length: u32,
}
impl AppIds {
    pub(super) fn new(
        source: Option<Source>,
        token_length: Option<usize>,
    ) -> Result<Self, BuildError> {
        let source = source.ok_or(BuildError::RandomnessRequired)?;
        let next_mid = match source {
            Source::Test => 1,
            Source::Io { initial_mid } => initial_mid,
            Source::Random(fill) => {
                let mut bytes = [0; 2];
                if !fill(&mut bytes) {
                    return Err(BuildError::RandomnessUnavailable);
                }
                u16::from_be_bytes(bytes)
            }
        };
        Ok(Self {
            source,
            next_mid,
            issued: 0,
            last_mid_ms: 0,
            jitter: 0,
            test_token: 0,
            token_length: token_length
                .unwrap_or(if matches!(source, Source::Test) { 4 } else { 8 })
                .try_into()
                .map_err(|_| BuildError::TokenLength)?,
        })
    }

    pub(super) fn token<K: OwnedToken>(
        &mut self,
        io_random: &mut impl FnMut(&mut [u8]) -> bool,
    ) -> Result<K, IdentityError> {
        self.token_with_length(self.token_length as usize, io_random)
    }

    pub(super) fn nonce(
        &mut self,
        io_random: &mut impl FnMut(&mut [u8]) -> bool,
    ) -> Result<Token, IdentityError> {
        self.token_with_length(
            if matches!(self.source, Source::Test) {
                4
            } else {
                8
            },
            io_random,
        )
    }

    fn token_with_length<K: OwnedToken>(
        &mut self,
        len: usize,
        io_random: &mut impl FnMut(&mut [u8]) -> bool,
    ) -> Result<K, IdentityError> {
        let mut token = K::try_zeroed(len).map_err(|_| IdentityError::TokenExhausted)?;
        match self.source {
            Source::Test => {
                if len < 4 {
                    return Err(IdentityError::TokenExhausted);
                }
                self.test_token = self
                    .test_token
                    .checked_add(1)
                    .ok_or(IdentityError::TokenExhausted)?;
                token.as_bytes_mut()[len - 4..].copy_from_slice(&self.test_token.to_be_bytes());
            }
            Source::Random(_) | Source::Io { .. } => {
                if !self.fill(token.as_bytes_mut(), io_random) {
                    return Err(IdentityError::RandomnessUnavailable);
                }
            }
        }
        Ok(token)
    }

    pub(super) fn next_for<E, S, T: crate::storage::DatagramTransport>(
        &mut self,
        engine: &crate::storage::Engine<S>,
        io: &mut T,
        now_ms: u64,
    ) -> Result<MessageId, super::Error<E>>
    where
        S: crate::storage::Storage + crate::storage::PendingCons,
    {
        if self.issued == 65_536
            && (0..engine.capacities().tx_datagram_slots).any(|i| {
                engine
                    .pending_con(crate::storage::SlotId::from_index(i))
                    .is_some()
            })
        {
            return Err(super::Error::Identity(IdentityError::MessageIdExhausted));
        }
        self.next(now_ms, &mut |bytes| io.fill_random(bytes))
    }

    pub(super) fn next<E>(
        &mut self,
        now_ms: u64,
        io_random: &mut impl FnMut(&mut [u8]) -> bool,
    ) -> Result<MessageId, super::Error<E>> {
        if self.issued == 65_536
            && now_ms.saturating_sub(self.last_mid_ms)
                < u64::from(Transmission::EXCHANGE_LIFETIME_MS)
        {
            return Err(super::Error::Identity(IdentityError::MessageIdExhausted));
        }
        let jitter = self
            .sample_jitter(io_random)
            .map_err(super::Error::Identity)?;
        if self.issued == 65_536 {
            self.issued = 0;
        }
        let mid = MessageId::new(self.next_mid);
        self.next_mid = self.next_mid.wrapping_add(1);
        self.issued += 1;
        self.last_mid_ms = self.last_mid_ms.max(now_ms);
        // The protocol's jitter interval is at most 1000 milliseconds.
        self.jitter = jitter as u16;
        Ok(mid)
    }

    pub(super) const fn jitter(&self) -> u32 {
        self.jitter as u32
    }

    fn fill(&self, bytes: &mut [u8], io_random: &mut impl FnMut(&mut [u8]) -> bool) -> bool {
        match self.source {
            Source::Random(fill) => fill(bytes),
            Source::Io { .. } => io_random(bytes),
            Source::Test => false,
        }
    }

    fn sample_jitter(
        &self,
        io_random: &mut impl FnMut(&mut [u8]) -> bool,
    ) -> Result<u32, IdentityError> {
        if matches!(self.source, Source::Test) {
            return Ok(0);
        }
        let width = Transmission::ACK_RANDOM_SPAN_MS + 1;
        let limit = u32::MAX - (u32::MAX % width);
        for _ in 0..8 {
            let mut bytes = [0; 4];
            if !self.fill(&mut bytes, io_random) {
                return Err(IdentityError::RandomnessUnavailable);
            }
            let value = u32::from_be_bytes(bytes);
            if value < limit {
                return Ok(value % width);
            }
        }
        Err(IdentityError::RandomnessUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    #[test]
    fn jitter_covers_closed_rfc_interval_and_bounds_rejection_work() {
        static VALUE: AtomicU32 = AtomicU32::new(0);
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn fill(bytes: &mut [u8]) -> bool {
            if bytes.len() == 4 {
                CALLS.fetch_add(1, Ordering::SeqCst);
                bytes.copy_from_slice(&VALUE.load(Ordering::SeqCst).to_be_bytes());
            } else {
                bytes.fill(0);
            }
            true
        }
        let mut ids = AppIds::new(Some(Source::Random(fill)), None).unwrap();
        for value in 0..=1000 {
            VALUE.store(value, Ordering::SeqCst);
            ids.next::<()>(0, &mut |_| false).unwrap();
            assert_eq!(ids.jitter(), value);
            assert_eq!(Transmission::initial_timeout_ms(ids.jitter()), 2000 + value);
        }
        VALUE.store(u32::MAX, Ordering::SeqCst);
        CALLS.store(0, Ordering::SeqCst);
        let issued = ids.issued;
        assert_eq!(
            ids.next::<()>(0, &mut |_| false),
            Err(super::super::Error::Identity(
                IdentityError::RandomnessUnavailable
            ))
        );
        assert_eq!(CALLS.load(Ordering::SeqCst), 8);
        assert_eq!(ids.issued, issued);
    }

    #[test]
    fn mid_wrap_waits_from_last_issue_and_clock_rollback_cannot_shorten_wait() {
        let mut ids = AppIds::new(Some(Source::Test), None).unwrap();
        let mut seen = [false; 65536];
        for i in 0..65536 {
            let now = if i == 65534 { 5000 } else { 1000 };
            let mid = ids.next::<()>(now, &mut |_| false).unwrap().get() as usize;
            assert!(!seen[mid]);
            seen[mid] = true;
        }
        assert_eq!(
            ids.next::<()>(251_999, &mut |_| false),
            Err(super::super::Error::Identity(
                IdentityError::MessageIdExhausted
            ))
        );
        assert_eq!(ids.next::<()>(252_000, &mut |_| false).unwrap().get(), 1);
    }
}
