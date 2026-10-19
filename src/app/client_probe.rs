//! UDP extended-token discovery over the ordinary bounded client path.

#[cfg(all(test, feature = "oscore"))]
mod tests {
    use super::*;
    #[test]
    fn evidence_isolated_from_reused_address_and_association_epoch() {
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let association = super::super::super::Association {
            route: peer.with_association(1),
            owner: [1; 32],
            epoch: 7,
        };
        let record = TokenSupport {
            peer: association.route,
            epoch: 7,
            length: 32,
            outcome: TokenSupportOutcome::Supported(Code::CONTENT),
            expires_ms: 100,
        };
        assert!(record.usable_for_association(association, 32, 0));
        assert!(!record.usable_for(peer, 7, 32, 0));
        assert!(!record.usable_for_association(
            super::super::super::Association {
                route: peer.with_association(2),
                ..association
            },
            32,
            0
        ));
        assert!(!record.usable_for_association(
            super::super::super::Association {
                epoch: 8,
                ..association
            },
            32,
            0
        ));
    }
}

use super::*;

/// Terminal canonical discovery request. Only send operations remain available.
pub struct TokenProbe<O> {
    outgoing: O,
    epoch: u64,
}

/// A sent canonical probe, tied to its ordinary client Call and caller epoch.
#[derive(Clone, Copy, Debug)]
pub struct TokenProbeCall<K: OwnedToken = Token> {
    call: Call<K>,
    epoch: u64,
}

/// Meaning of the response to the canonical RFC 8974 UDP probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenSupportOutcome {
    /// A matching echoed response other than the defined capacity refusals.
    Supported(Code),
    /// An echoed 4.00 indicates permanent refusal at the tested length.
    LengthRefused,
    /// An echoed 5.03 indicates temporary refusal.
    TemporaryRefusal,
    /// An empty Reset rejected the canonical extended-token request.
    Unsupported,
    /// No usable evidence: timeout, cancellation, unmatched response or local failure.
    Unknown,
}

/// Invalid capability lifetime; accepted values are 1800 through 86400 seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenSupportLifetimeError;

/// Caller-owned capability evidence; no App table or background cache is created.
///
/// Discard records when the endpoint's owner/key epoch changes. The epoch supplied
/// to the probe is application policy, not authentication established by this record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenSupport {
    peer: Endpoint,
    epoch: u64,
    length: usize,
    outcome: TokenSupportOutcome,
    expires_ms: u64,
}

impl<K: OwnedToken> TokenProbeCall<K> {
    /// Ordinary Call for response collection or cancellation.
    pub const fn call(self) -> Call<K> {
        self.call
    }

    /// Classify this probe's collected result with the default 1800-second lifetime.
    pub fn record(
        self,
        result: &Result<Response<'_, K>, CallFailure>,
        now_ms: u64,
    ) -> TokenSupport {
        self.record_for(result, now_ms, 1800)
            .expect("valid default lifetime")
    }

    /// Classify exact matching evidence with a caller-selected bounded lifetime.
    pub fn record_for(
        self,
        result: &Result<Response<'_, K>, CallFailure>,
        now_ms: u64,
        lifetime_secs: u32,
    ) -> Result<TokenSupport, TokenSupportLifetimeError> {
        if !(1800..=86400).contains(&lifetime_secs) {
            return Err(TokenSupportLifetimeError);
        }
        let outcome = match result {
            Ok(response)
                if response.token() == Some(self.call.token)
                    && response.peer() == Some(self.call.peer) =>
            {
                match response.code() {
                    Code::BAD_REQUEST => TokenSupportOutcome::LengthRefused,
                    Code::SERVICE_UNAVAILABLE => TokenSupportOutcome::TemporaryRefusal,
                    code => TokenSupportOutcome::Supported(code),
                }
            }
            Err(CallFailure::Reset) => TokenSupportOutcome::Unsupported,
            _ => TokenSupportOutcome::Unknown,
        };
        Ok(TokenSupport {
            peer: self.call.peer,
            epoch: self.epoch,
            length: self.call.token.len(),
            outcome,
            expires_ms: now_ms.saturating_add(u64::from(lifetime_secs) * 1000),
        })
    }
}
impl TokenSupport {
    /// Classification, preserving permanent and temporary refusal.
    pub const fn outcome(self) -> TokenSupportOutcome {
        self.outcome
    }
    /// Full token length actually tested.
    pub const fn tested_length(self) -> usize {
        self.length
    }
    /// Absolute expiry in the same monotonic clock domain as polling.
    pub const fn expires_at(self) -> u64 {
        self.expires_ms
    }
    /// Whether unassociated endpoint/epoch evidence permits the requested length.
    pub fn usable_for(self, peer: Endpoint, epoch: u64, length: usize, now_ms: u64) -> bool {
        self.peer == self.peer.physical()
            && peer == peer.physical()
            && self.matches(peer, epoch, length, now_ms)
    }
    /// Whether evidence belongs to this exact installed association and epoch.
    pub fn usable_for_association(
        self,
        association: super::super::Association,
        length: usize,
        now_ms: u64,
    ) -> bool {
        self.matches(association.route, association.epoch, length, now_ms)
    }
    fn matches(self, peer: Endpoint, epoch: u64, length: usize, now_ms: u64) -> bool {
        matches!(self.outcome, TokenSupportOutcome::Supported(_))
            && self.peer == peer
            && self.epoch == epoch
            && length <= self.length
            && now_ms < self.expires_ms
    }
}

impl<
    'a,
    P,
    T,
    const N: usize,
    const B: bool,
    S: super::super::AppStorage<Token = P::Token>,
    const D: usize,
    const C: usize,
    const R: usize,
    const A: usize,
> Outgoing<'a, P, T, N, Present, B, S, D, C, R, A>
where
    P: crate::storage::MemoryProfile + MemoryLayout<B> + AppAssembled<B>,
{
    /// Replace the request intent with the canonical CON GET/If-None-Match probe.
    ///
    /// Call after selecting the destination and optional deadline. The terminal
    /// wrapper prevents later changes from making a Reset ambiguous. The selected
    /// App token length is tested. Use an application owner/key epoch, changing it
    /// whenever the same address may represent a different peer or security context.
    /// For an installed association, supply its authenticated policy epoch.
    pub fn probe_token_support(mut self, epoch: u64) -> TokenProbe<Self> {
        self.code = Code::GET;
        self.ty = Type::Confirmable;
        self.path = Ok(Path::empty());
        self.payload = &[];
        self.content_format = None;
        self.echo = None;
        self.no_response = None;
        self.accept = None;
        self.request_tag = BodyTag::ABSENT;
        self.etag = None;
        self.if_match = None;
        self.if_none_match = true;
        self.queries = [""; MAX_PATH_SEGMENTS];
        self.query_n = 0;
        self.q_block1 = false;
        self.q_block2 = false;
        self.block2 = None;
        self.observe = OutgoingObserve::Off;
        self.observe_target = None;
        TokenProbe {
            outgoing: self,
            epoch,
        }
    }
}

impl<
    'a,
    P,
    T,
    const N: usize,
    const B: bool,
    S: super::super::AppStorage<Token = P::Token>,
    const D: usize,
    const C: usize,
    const R: usize,
    const A: usize,
> TokenProbe<Outgoing<'a, P, T, N, Present, B, S, D, C, R, A>>
where
    P: crate::storage::MemoryProfile + MemoryLayout<B> + AppAssembled<B>,
    T: crate::storage::DatagramTransport,
{
    /// Send through the ordinary bounded synchronous client path.
    pub fn send(self, now_ms: u64) -> Result<TokenProbeCall<S::Token>, Error<T::Error>>
    where
        T: DatagramIo,
    {
        self.outgoing.send(now_ms).map(|call| TokenProbeCall {
            call,
            epoch: self.epoch,
        })
    }
    /// Send through the ordinary asynchronous ownership/cancellation path.
    pub fn send_async<E: 'a>(
        self,
        now_ms: u64,
        completed_at: impl FnMut() -> Result<u64, E> + 'a,
        timer: &'a mut impl crate::platform::AsyncTimer,
    ) -> impl core::future::Future<
        Output = Result<TokenProbeCall<S::Token>, AsyncCallError<T::Error, E>>,
    > + 'a
    where
        T: crate::storage::AsyncDatagramIo,
    {
        let epoch = self.epoch;
        let send = self.outgoing.send_async(now_ms, completed_at, timer);
        async move { send.await.map(|call| TokenProbeCall { call, epoch }) }
    }
    /// Async send with an additional absolute driving deadline.
    pub fn send_until_async<E: 'a>(
        self,
        now_ms: u64,
        completed_at: impl FnMut() -> Result<u64, E> + 'a,
        timer: &'a mut impl crate::platform::AsyncTimer,
        send_deadline_ms: u64,
    ) -> impl core::future::Future<
        Output = Result<TokenProbeCall<S::Token>, AsyncCallError<T::Error, E>>,
    > + 'a
    where
        T: crate::storage::AsyncDatagramIo,
    {
        let epoch = self.epoch;
        let send = self
            .outgoing
            .send_until_async(now_ms, completed_at, timer, send_deadline_ms);
        async move { send.await.map(|call| TokenProbeCall { call, epoch }) }
    }
}
