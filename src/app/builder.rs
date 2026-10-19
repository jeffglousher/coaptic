//! App construction, security selection and platform binding.

use super::*;
use crate::storage::DatagramTransport;

impl App {
    /// Start with the default fixed profile and datagram-only storage.
    ///
    /// Supply [`AppBuilder::randomness`] and [`AppBuilder::bind`] for simple
    /// requests, or adjust [`AppBuilder::block_wise`], routes, storage and
    /// full-response retention through the same builder.
    #[must_use]
    pub fn builder() -> AppBuilder<crate::profiles::Default, Present> {
        Self::profile::<crate::profiles::Default>().block_wise::<false>()
    }

    /// Start from a memory profile. Next: [`AppBuilder::block_wise`].
    #[must_use]
    pub const fn profile<P: MemoryProfile>() -> AppBuilder<P> {
        Self::profile_requests::<P, 4, 1>()
    }

    pub(super) const fn profile_requests<
        P: MemoryProfile,
        const REQUESTS: usize,
        const ASSOCIATIONS: usize,
    >() -> AppBuilder<
        P,
        Missing,
        DEFAULT_ROUTES,
        false,
        0,
        DEFAULT_CLIENT_CALLS,
        REQUESTS,
        ASSOCIATIONS,
    > {
        AppBuilder {
            site: Site::new(),
            echo_policy: None,
            full_responses: false,
            deferred_lifetime_ms: Transmission::EXCHANGE_LIFETIME_MS as u64,
            identity: None,
            token_length: None,
            oscore: oscore::Store::new(),
            oscore_checkpoint: oscore::CheckpointState::new(),
            allow_unprotected: false,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<
    P: MemoryProfile,
    Block,
    const N: usize,
    const BLOCK_WISE: bool,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> AppBuilder<P, Block, N, BLOCK_WISE, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
{
    /// Size the live client Call and completion tables independently of pools.
    ///
    /// Defaults to [`DEFAULT_CLIENT_CALLS`]. Zero is useful for a server-only
    /// App: all new client requests refuse with [`Error::Saturated`]. Untaken
    /// replies and Observe subscriptions retain their Call slot. Collection
    /// frees ordinary Calls; subscriptions require cancellation or expiry.
    ///
    /// Overload backoffs survive response collection and consume admission capacity
    /// until their Max-Age expires. The existing Call row retains its request
    /// identity during cooldown; no separate backoff table or query copy is reserved.
    ///
    /// These tables are inline fixed arrays, even with allocator-backed packet
    /// storage. Each Call reserves request/Observe and response-header metadata;
    /// expiry also uses bounded scratch proportional to this count. This does
    /// not resize packet/body pools, server observers, routes or security state.
    /// Pairwise OSCORE defaults to four live request bindings per context. A
    /// larger Call table does not increase that independent protected limit.
    /// Counts are fixed-array lengths checked by Rust's type layout; there is
    /// no runtime multiplication or allocator growth for these tables.
    ///
    /// ```
    /// let _small_client = coaptic::App::builder().client_calls::<1>();
    /// let _server_only = coaptic::App::builder().client_calls::<0>();
    /// ```
    #[must_use]
    pub fn client_calls<const SLOTS: usize>(
        self,
    ) -> AppBuilder<P, Block, N, BLOCK_WISE, DEFERRED, SLOTS, REQUESTS, ASSOCIATIONS> {
        AppBuilder {
            site: self.site,
            echo_policy: self.echo_policy,
            full_responses: self.full_responses,
            deferred_lifetime_ms: self.deferred_lifetime_ms,
            identity: self.identity,
            token_length: self.token_length,
            oscore: self.oscore,
            oscore_checkpoint: self.oscore_checkpoint,
            allow_unprotected: self.allow_unprotected,
            _p: PhantomData,
            _b: PhantomData,
        }
    }

    /// Site table size (default [`DEFAULT_ROUTES`]). Call before
    /// [`Self::route`].
    ///
    /// # Panics
    ///
    /// If a route is already registered.
    #[must_use]
    pub fn routes<const M: usize>(
        self,
    ) -> AppBuilder<P, Block, M, BLOCK_WISE, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS> {
        assert!(
            self.site.is_empty(),
            "call .routes::<M>() before .route(...)"
        );
        let well_known = self.site.has_well_known_core();
        let mut site = Site::new();
        if well_known {
            site.well_known_core();
        }
        AppBuilder {
            site,
            echo_policy: self.echo_policy,
            full_responses: self.full_responses,
            deferred_lifetime_ms: self.deferred_lifetime_ms,
            identity: self.identity,
            token_length: self.token_length,
            oscore: self.oscore,
            oscore_checkpoint: self.oscore_checkpoint,
            allow_unprotected: self.allow_unprotected,
            _p: PhantomData,
            _b: PhantomData,
        }
    }

    /// Configure bounded deferred server work on the same App builder.
    ///
    /// Defaults to zero rows: ordinary clients reserve no request/response
    /// header arrays for deferred work. Four rows is a useful server starting
    /// point. Each enabled row reserves bounded 512-byte request selection and
    /// 512-byte response-header metadata; body bytes stay in existing TX pools.
    /// The slot count is independent of storage allocation and `std`.
    ///
    /// TX slots are shared by pending CONs, encoded dedup replies and response
    /// workspace. Reserve at least pending-CON slots plus one workspace slot;
    /// adding dedup-entry capacity preserves every cached wire reply. With a
    /// smaller pool, App reclaims a dedup wire pin into an empty-ACK tombstone:
    /// the original request is acknowledged without repeating its handler.
    #[must_use]
    pub fn deferred<const SLOTS: usize>(
        self,
    ) -> AppBuilder<P, Block, N, BLOCK_WISE, SLOTS, CALLS, REQUESTS, ASSOCIATIONS> {
        AppBuilder {
            site: self.site,
            echo_policy: self.echo_policy,
            full_responses: self.full_responses,
            deferred_lifetime_ms: self.deferred_lifetime_ms,
            identity: self.identity,
            token_length: self.token_length,
            oscore: self.oscore,
            oscore_checkpoint: self.oscore_checkpoint,
            allow_unprotected: self.allow_unprotected,
            _p: PhantomData,
            _b: PhantomData,
        }
    }

    /// Retain complete client representations for [`App::take_response_into`].
    ///
    /// Ordinary responses keep one bounded RX datagram occupied until collection;
    /// Block2 retains its existing RX body. Untaken replies apply backpressure.
    /// Each active or uncollected Call reserves one logical RX credit, including
    /// blockwise Calls. At most `rx_datagram_slots - 1` such Calls are admitted;
    /// the remaining packet receives control traffic and overload refusals.
    /// Collection or cancellation releases the credit. Observe refreshes reuse it.
    #[must_use]
    pub const fn full_responses(mut self) -> Self {
        self.full_responses = true;
        self
    }

    /// Bound how long a deferred response handle remains usable.
    ///
    /// Enable slots with [`Self::deferred`] before accepting deferred work.
    /// Defaults to EXCHANGE_LIFETIME. Application identity and durable
    /// idempotency remain caller-owned; increasing this bound does not extend
    /// protocol duplicate history after completion or change retransmission.
    /// Zero refuses deferred admission. Uses the caller monotonic clock.
    #[must_use]
    pub const fn deferred_lifetime(mut self, milliseconds: u64) -> Self {
        self.deferred_lifetime_ms = milliseconds;
        self
    }

    /// Bind `methods` on Uri-Path `path`.
    ///
    /// `path` is [`IntoPath`]: `"sensors/temp"` or `&["sensors", "temp"]`.
    /// `methods` is a site router ([`get`], [`put`], …), not an outbound
    /// [`App::get`] builder.
    /// Panics on invalid paths or a new path at capacity; use [`Self::try_route`]
    /// for recoverable refusal.
    #[must_use]
    pub fn route(mut self, path: impl IntoPath, methods: MethodRouter<P::Token>) -> Self {
        self.site.route(path, methods);
        self
    }

    /// Bind `methods` on a `'static` URI-Path (`"/leds/0"`).
    #[must_use]
    pub fn route_path(mut self, path: &'static str, methods: MethodRouter<P::Token>) -> Self {
        self.site.route_path(path, methods);
        self
    }

    /// Checked route registration; refusal preserves this builder and its routes.
    ///
    /// Borrow the builder to recover from refusal, then consume it with `bind`.
    /// See [`Site::try_route`] for path and capacity rules.
    pub fn try_route(
        &mut self,
        path: impl IntoPath,
        methods: MethodRouter<P::Token>,
    ) -> Result<&mut Self, RouteError> {
        self.site.try_route(path, methods)?;
        Ok(self)
    }

    /// Checked registration of a static slash-separated path.
    pub fn try_route_path(
        &mut self,
        path: &'static str,
        methods: MethodRouter<P::Token>,
    ) -> Result<&mut Self, RouteError> {
        self.try_route(path, methods)
    }

    /// Remove a route before binding, preserving this builder on path refusal.
    /// Returns whether the path was registered. See [`Site::remove_route`].
    pub fn remove_route(&mut self, path: impl IntoPath) -> Result<bool, PathError> {
        self.site.remove_route(path)
    }

    /// Serve `/.well-known/core` from registered paths.
    ///
    /// Catalog scratch is [`LinkFormatScratch`] (compile-time
    /// [`link_format_capacity`] after [`Self::routes`]). Overflow is 5.00,
    /// not a truncated list.
    #[must_use]
    pub fn well_known_core(mut self) -> Self {
        self.site.well_known_core();
        self
    }

    /// Supply cryptographically secure bytes for Tokens, initial MID and RTO jitter.
    /// Required before bind; see [`RandomSource`]. The library does not call an OS RNG.
    /// Eight-byte Tokens are checked against all active App calls. MID allocation
    /// uses one sequence across local CON/NON requests, responses and Q windows.
    /// After 65536 allocations, reuse waits at least EXCHANGE_LIFETIME from the
    /// last allocation and requires pending transmissions to finish.
    ///
    /// Keep the App alive for the endpoint's lifetime. Recreating it at the same
    /// local endpoint requires a quiet EXCHANGE_LIFETIME interval or a caller-
    /// managed persistence strategy through the advanced Engine API. A random
    /// initial MID alone is not a restart collision guarantee.
    #[must_use]
    pub const fn randomness(mut self, source: RandomSource) -> Self {
        self.identity = Some(IdentitySource::Random(source));
        self
    }

    /// Select the length of generated request tokens (at least eight bytes).
    /// Binding refuses lengths above this profile's token storage capacity.
    /// The default is eight random bytes; deterministic test identities keep
    /// their four-byte default unless this setting is supplied explicitly.
    #[must_use]
    pub const fn token_length(mut self, len: usize) -> Self {
        self.token_length = Some(len);
        self
    }

    /// Predictable counters and minimum RTO, for reproducible tests only.
    /// Do not use this configuration on a production network.
    #[must_use]
    pub const fn deterministic_for_tests(mut self) -> Self {
        self.identity = Some(IdentitySource::Test);
        self
    }

    /// Configure pairwise OSCORE before binding any storage or transport.
    ///
    /// Ordinary App construction requires this provisioned context. Context
    /// derivation, peer authentication and durable sender-sequence reservation
    /// remain caller-owned. Never reuse a key and sender ID with a reset sequence
    /// after restart. See [`App::set_oscore`] for supported protocol combinations
    /// and the in-memory replay window's durability limits.
    ///
    /// The `oscore` feature is enabled by default and works without an allocator.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn oscore(
        mut self,
        context: crate::oscore::BoundedSecurityContext<REQUESTS, P::Token>,
    ) -> Self {
        self.oscore.set_primary(context);
        self.oscore_checkpoint.invalidate();
        self
    }

    /// Bind with no active keys while refusing all nonempty traffic until installation.
    /// Select capacity through `Resources::associations` before constructing this builder.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn require_associations(mut self) -> Self {
        self.oscore.generation = self.oscore.generation.max(1);
        self
    }

    /// Require durable recipient request checkpoints before any traffic is polled.
    ///
    /// Binding requires a provisioned OSCORE context even if unprotected was
    /// explicitly allowed. Use [`App::poll_with_oscore_checkpoint`] or
    /// [`App::poll_with_oscore_checkpoint_and_dispatch`] for every poll. Ordinary
    /// polling fails closed before I/O. The first durable poll commits the current
    /// replay window before receiving; subsequent authenticated requests commit
    /// their updated window before acknowledgment, body admission or handlers.
    ///
    /// This guards inbound request effects. Recipient checkpoint freshness,
    /// sender reservations and durable application completion remain caller-owned;
    /// client response/Observe state is not made persistent by this option.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub const fn require_oscore_checkpoint(mut self) -> Self {
        self.oscore_checkpoint.require();
        self
    }

    /// Permit unprotected CoAP for compatibility or tests.
    ///
    /// This supplies no confidentiality, integrity or peer authentication.
    /// Deterministic test mode does not select this mode. A configured
    /// OSCORE context still protects traffic and rejects unprotected messages.
    /// Disabling the Cargo `oscore` feature also requires this explicit opt-out;
    /// removing cryptography never silently disables message protection.
    #[must_use]
    pub const fn allow_unprotected(mut self) -> Self {
        self.allow_unprotected = true;
        self
    }

    pub(super) fn validate_security(&self) -> Result<(), BuildError> {
        if self.oscore_checkpoint.required() && !oscore::is_active(&self.oscore) {
            return Err(BuildError::SecurityRequired);
        }
        #[cfg(feature = "oscore")]
        if self.oscore.is_some() {
            return Ok(());
        }
        if self.allow_unprotected {
            Ok(())
        } else {
            Err(BuildError::SecurityRequired)
        }
    }

    /// Install an explicit server Echo issuer/verifier. See [`EchoPolicy`].
    ///
    /// Off by default. Challenge/reject decisions produce 4.01 before body
    /// assembly or handler side effects. Only `EchoDecision::Accept` proceeds.
    #[must_use]
    pub const fn echo_policy(mut self, policy: EchoPolicy) -> Self {
        self.echo_policy = Some(policy);
        self
    }
}

impl<
    P: MemoryProfile,
    Block,
    const N: usize,
    const PREV: bool,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> AppBuilder<P, Block, N, PREV, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
{
    /// Enable or disable body pools, then [`AppBuilder::bind`].
    ///
    /// Shipped profiles use 4096 bytes per body slot, even
    /// [`crate::profiles::Constrained`]. Enabling this adds RX and TX body pools
    /// (8 KiB total for Constrained; 16 KiB for Default), a 4 KiB client
    /// assembled-body hold, and bookkeeping. Datagram sizes do not shrink
    /// that body quantum; size the complete `App` for your target.
    ///
    /// The flag is a const generic so [`App`] RAM matches Storage:
    /// `.block_wise::<false>()` does not reserve RX/TX body arrays or the
    /// client Block2 assembled hold. A piggybacked payload larger than
    /// [`INLINE_PAYLOAD`] (128) is still truncated on
    /// [`App::take_response`] in either mode; enable Block2 and read
    /// [`Response::body`] for the full representation.
    /// Without body pools, a received Block2 / Q-Block2 reply completes the
    /// Call with [`CallFailure::BlockTransfer`] containing
    /// [`BlockTransferError::NoBodyPools`]. No fragment is returned as success.
    #[must_use]
    pub fn block_wise<const ENABLED: bool>(
        self,
    ) -> AppBuilder<P, Present, N, ENABLED, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS> {
        AppBuilder {
            site: self.site,
            echo_policy: self.echo_policy,
            full_responses: self.full_responses,
            deferred_lifetime_ms: self.deferred_lifetime_ms,
            identity: self.identity,
            token_length: self.token_length,
            oscore: self.oscore,
            oscore_checkpoint: self.oscore_checkpoint,
            allow_unprotected: self.allow_unprotected,
            _p: PhantomData,
            _b: PhantomData,
        }
    }
}

impl<
    P,
    const N: usize,
    const BLOCK_WISE: bool,
    const DEFERRED: usize,
    const CALLS: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> AppBuilder<P, Present, N, BLOCK_WISE, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>
where
    P: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    pub(super) fn validate_scratch<S: DatagramSlots>(
        c: crate::storage::Capacities,
    ) -> Result<(), BuildError> {
        c.validate_for_build(BLOCK_WISE)?;
        if c.rx_datagram_slots < 2 || c.tx_datagram_slots < 2 {
            return Err(BuildError::PacketHeadroom);
        }
        // Check the actual borrowed slices: custom storage need not use the
        // default associated constants. This is App admission, not Engine policy.
        {
            let mut rx = S::RxScratch::default();
            if c.rx_datagram_bytes > rx.as_ref().len() || c.rx_datagram_bytes > rx.as_mut().len() {
                return Err(BuildError::SizeMismatch);
            }
        }
        {
            let mut tx = S::TxScratch::default();
            if c.tx_datagram_bytes > tx.as_ref().len() || c.tx_datagram_bytes > tx.as_mut().len() {
                return Err(BuildError::SizeMismatch);
            }
        }
        Ok(())
    }

    fn fixed_capacities() -> crate::storage::Capacities {
        let c = crate::storage::Capacities::from_profile::<P>();
        if BLOCK_WISE {
            c.with_block_wise::<P>()
        } else {
            c
        }
    }

    /// Bind the profile's fixed memory, with or without body pools.
    /// At least two RX and two TX packets are required: one of each remains
    /// available for ingress and transient control/refusal traffic.
    ///
    /// Deep configuration uses [`Self::bind_storage`] / `Self::bind_alloc`
    /// without changing request builders, response types or the reactor.
    /// Missing security configuration and scratch dimensions that cannot hold
    /// the declared datagrams are rejected before storage construction.
    pub fn bind<T>(
        self,
        io: T,
    ) -> Result<
        App<P, T, N, BLOCK_WISE, AppStore<P, BLOCK_WISE>, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>,
        BuildError,
    > {
        self.validate_security()?;
        Self::validate_scratch::<AppStore<P, BLOCK_WISE>>(Self::fixed_capacities())?;
        let engine = EngineBuilder::new()
            .profile::<P>()
            .block_wise(BLOCK_WISE)
            .build(AppStore::<P, BLOCK_WISE>::default())?;
        self.bind_built(io, engine)
    }

    /// Bind caller-selected storage without changing the App interface.
    ///
    /// App validates datagram sizes against both readable and writable scratch
    /// slices before construction; Engine validates pool/body dimensions.
    /// `P` remains the default-memory marker;
    /// the selected `S` supplies actual datagram/body dimensions and scratch.
    /// With `alloc`, use `Self::bind_alloc` for one-time bounded allocation.
    pub fn bind_storage<T, S: AppStorage<Token = P::Token>>(
        self,
        io: T,
        storage: S,
    ) -> Result<App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>, BuildError>
    {
        self.validate_security()?;
        let c = storage.capacities();
        Self::validate_scratch::<S>(c)?;
        let engine = EngineBuilder::new()
            .rx_datagram(c.rx_datagram_slots, c.rx_datagram_bytes)
            .tx_datagram(c.tx_datagram_slots, c.tx_datagram_bytes)
            .dedup(c.dedup_entries)
            .observe(c.observe_entries)
            .block_wise(BLOCK_WISE)
            .build(storage)?;
        self.bind_built(io, engine)
    }

    /// Allocate bounded pools once, then use the ordinary App API.
    ///
    /// Requires `alloc`, independently of `std`. There is no growth during poll.
    /// Allocation/layout failure returns [`BuildError`]. For an aggregate heap
    /// request limit, construct [`crate::storage::AllocMemory<P::Token>::try_new`] and pass
    /// it to [`Self::bind_storage`].
    #[cfg(feature = "alloc")]
    pub fn bind_alloc<T>(
        self,
        io: T,
        c: crate::storage::Capacities,
    ) -> Result<
        App<
            P,
            T,
            N,
            BLOCK_WISE,
            crate::storage::AllocMemory<P::Token>,
            DEFERRED,
            CALLS,
            REQUESTS,
            ASSOCIATIONS,
        >,
        BuildError,
    > {
        self.validate_security()?;
        Self::validate_scratch::<crate::storage::AllocMemory<P::Token>>(c)?;
        let engine = EngineBuilder::new()
            .rx_datagram(c.rx_datagram_slots, c.rx_datagram_bytes)
            .tx_datagram(c.tx_datagram_slots, c.tx_datagram_bytes)
            .dedup(c.dedup_entries)
            .observe(c.observe_entries)
            .block_wise(BLOCK_WISE)
            .build_alloc_with_token::<P::Token>(c)?;
        self.bind_built(io, engine)
    }

    /// Bind fixed storage with a caller-owned stateful cryptographic RNG.
    ///
    /// The callback may borrow a hardware RNG and must fill every byte securely
    /// in bounded time or return false. No allocation or global callback state is
    /// introduced. A prior `randomness` or `deterministic_for_tests` selection is
    /// an error, even if it would select the same source. Security and source
    /// ambiguity are checked first. Invalid datagram/scratch dimensions then
    /// refuse before invoking entropy or touching the socket. Scratch defaults
    /// are constructed for validation; caller-defined scratch owns its costs.
    /// See [`crate::platform::WithEntropy`] for adapter composition and limits.
    // Preserve the existing App type parameters; only the transport is wrapped.
    #[allow(clippy::type_complexity)]
    pub fn bind_with_entropy<T: DatagramTransport, R: FnMut(&mut [u8]) -> bool>(
        self,
        io: T,
        random: R,
    ) -> Result<
        App<
            P,
            crate::platform::WithEntropy<T, R>,
            N,
            BLOCK_WISE,
            AppStore<P, BLOCK_WISE>,
            DEFERRED,
            CALLS,
            REQUESTS,
            ASSOCIATIONS,
        >,
        BuildError,
    > {
        self.validate_entropy_source()?;
        Self::validate_scratch::<AppStore<P, BLOCK_WISE>>(Self::fixed_capacities())?;
        let (builder, io) = self.prepare_entropy(io, random)?;
        builder.bind(io)
    }

    /// Bind caller-selected storage and a stateful entropy callback.
    /// See [`Self::bind_with_entropy`] for the callback and refusal contract.
    pub fn bind_storage_with_entropy<
        T: DatagramTransport,
        R: FnMut(&mut [u8]) -> bool,
        S: AppStorage<Token = P::Token>,
    >(
        self,
        io: T,
        storage: S,
        random: R,
    ) -> Result<
        App<
            P,
            crate::platform::WithEntropy<T, R>,
            N,
            BLOCK_WISE,
            S,
            DEFERRED,
            CALLS,
            REQUESTS,
            ASSOCIATIONS,
        >,
        BuildError,
    > {
        self.validate_entropy_source()?;
        Self::validate_scratch::<S>(storage.capacities())?;
        let (builder, io) = self.prepare_entropy(io, random)?;
        builder.bind_storage(io, storage)
    }

    /// Allocate bounded pools once and bind a stateful entropy callback.
    /// Requires `alloc` independently of `std`; see [`Self::bind_with_entropy`].
    #[cfg(feature = "alloc")]
    #[allow(clippy::type_complexity)]
    pub fn bind_alloc_with_entropy<T: DatagramTransport, R: FnMut(&mut [u8]) -> bool>(
        self,
        io: T,
        c: crate::storage::Capacities,
        random: R,
    ) -> Result<
        App<
            P,
            crate::platform::WithEntropy<T, R>,
            N,
            BLOCK_WISE,
            crate::storage::AllocMemory<P::Token>,
            DEFERRED,
            CALLS,
            REQUESTS,
            ASSOCIATIONS,
        >,
        BuildError,
    > {
        self.validate_entropy_source()?;
        Self::validate_scratch::<crate::storage::AllocMemory<P::Token>>(c)?;
        let (builder, io) = self.prepare_entropy(io, random)?;
        builder.bind_alloc(io, c)
    }

    pub(super) fn validate_entropy_source(&self) -> Result<(), BuildError> {
        self.validate_security()?;
        if self.identity.is_some() {
            return Err(BuildError::RandomnessAlreadyConfigured);
        }
        Ok(())
    }

    pub(super) fn prepare_entropy<T: DatagramTransport, R: FnMut(&mut [u8]) -> bool>(
        mut self,
        io: T,
        random: R,
    ) -> Result<(Self, crate::platform::WithEntropy<T, R>), BuildError> {
        self.validate_entropy_source()?;
        let mut io = crate::platform::WithEntropy::new(io, random);
        let mut seed = [0; 2];
        if !io.fill_random(&mut seed) {
            return Err(BuildError::RandomnessUnavailable);
        }
        self.identity = Some(IdentitySource::Io {
            initial_mid: u16::from_be_bytes(seed),
        });
        Ok((self, io))
    }

    fn bind_built<T, S: AppStorage<Token = P::Token>>(
        self,
        io: T,
        engine: Engine<S>,
    ) -> Result<App<P, T, N, BLOCK_WISE, S, DEFERRED, CALLS, REQUESTS, ASSOCIATIONS>, BuildError>
    {
        let length =
            self.token_length
                .unwrap_or(if matches!(self.identity, Some(IdentitySource::Test)) {
                    4
                } else {
                    8
                });
        if self.token_length.is_some_and(|len| len < 8)
            || length > P::Token::MAX_LEN
            || length > crate::message::extended::MAX_TOKEN_LEN
        {
            return Err(BuildError::TokenLength);
        }
        Ok(App {
            engine,
            io,
            site: self.site,
            ids: AppIds::new(self.identity, self.token_length)?,
            inbox: client::ClientInbox::new(),
            lives: client::ClientLives::new(),
            echo_policy: self.echo_policy,
            full_responses: self.full_responses,
            deferred_lifetime_ms: self.deferred_lifetime_ms,
            dedup_closed: None,
            oscore: self.oscore,
            oscore_checkpoint: self.oscore_checkpoint,
            assembled: Default::default(),
            deferred: DeferredTable::new(),
        })
    }
}
