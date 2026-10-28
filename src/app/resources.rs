//! Select resource bounds once, then bind platform services with [`App::with_resources`].
//!
//! Presets are starting budgets, not power, throughput or whole-device RAM promises.
//! All support fixed storage without `std` or `alloc`. With `alloc`, call
//! `Resources::allocated` with an explicit heap request limit before construction.
//! The configured builder deliberately has no resource setters:
//!
//! ```compile_fail
//! use coaptic::{App, resources::SmallDevice};
//! let builder = App::with_resources(SmallDevice::new());
//! let builder = builder.client_calls::<8>(); // customize Resources first
//! ```

use super::*;

/// Fixed storage selected by the resource configuration's memory profile.
pub struct Fixed;

/// Runtime pool dimensions with an explicit maximum heap request size.
#[cfg(feature = "alloc")]
pub struct Allocated {
    capacities: crate::storage::Capacities,
    heap_limit: usize,
}

/// All configurable App resource dimensions, selected before platform binding.
///
/// `P` determines fixed pool/table sizes. Other dimensions remain const generics
/// so disabled areas occupy no memory and no allocator is required. Customize
/// this value, then move it to [`App::with_resources`]. Existing `AppBuilder`
/// construction also supports staged configuration.
pub struct Resources<
    P = crate::profiles::Default,
    const ROUTES: usize = 8,
    const BLOCK_WISE: bool = false,
    const DEFERRED: usize = 0,
    const CALLS: usize = 4,
    S = Fixed,
    const REQUESTS: usize = 4,
    const ASSOCIATIONS: usize = 1,
> {
    storage: S,
    _profile: PhantomData<P>,
}

/// Small datagram device: constrained pools, four routes, one client Call.
pub type SmallDevice = Resources<crate::profiles::Constrained, 4, false, 0, 1>;
/// Capable device: default pools with bodies, eight routes, two deferred jobs, four Calls.
pub type Device = Resources<crate::profiles::Default, 8, true, 2, 4>;
/// Gateway starting budget: larger pools, sixteen routes, four deferred jobs and Calls.
pub type Gateway = Resources<crate::profiles::Gateway, 16, true, 4, 4>;
/// Host service starting budget: larger pools, thirty-two routes, eight deferred jobs, four Calls.
///
/// This defaults to one OSCORE context and four live request bindings.
/// Customize the latter with [`Resources::security_requests`].
/// More packet storage does not add security associations or establish throughput.
pub type Service = Resources<crate::profiles::Service, 32, true, 8, 4>;

impl<
    P: MemoryProfile,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> Resources<P, N, B, D, C, Fixed, REQUESTS, ASSOCIATIONS>
{
    /// Create fixed resource bounds from a memory profile and the type's dimensions.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Effective packet/body/table capacities for this fixed configuration.
    #[must_use]
    pub const fn pool_capacities(&self) -> crate::storage::Capacities {
        let capacities = crate::storage::Capacities::from_profile::<P>();
        if B {
            capacities.with_block_wise::<P>()
        } else {
            capacities
        }
    }

    /// Set fixed RX datagram slot count and bytes per slot.
    ///
    /// The RX scratch buffer uses the same byte capacity. Other dimensions are unchanged.
    /// Binding an App requires at least two RX slots for packet headroom.
    #[must_use]
    pub const fn rx_datagram<const SLOTS: usize, const BYTES: usize>(
        self,
    ) -> Resources<
        crate::profiles::WithRxDatagram<P, SLOTS, BYTES>,
        N,
        B,
        D,
        C,
        Fixed,
        REQUESTS,
        ASSOCIATIONS,
    > {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Set fixed TX datagram slot count and bytes per slot.
    ///
    /// The TX scratch buffer uses the same byte capacity; the outstanding-exchange
    /// table uses the same slot count. Calls remain independently bounded.
    /// Binding an App requires at least two TX slots for packet headroom.
    #[must_use]
    pub const fn tx_datagram<const SLOTS: usize, const BYTES: usize>(
        self,
    ) -> Resources<
        crate::profiles::WithTxDatagram<P, SLOTS, BYTES>,
        N,
        B,
        D,
        C,
        Fixed,
        REQUESTS,
        ASSOCIATIONS,
    > {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Set fixed RX complete-body slot count and bytes per slot.
    ///
    /// This preserves the selected block-wise mode. Bodies occupy no storage when
    /// disabled; when enabled, bytes must be a positive multiple of 1024 and
    /// the slot count must be positive. Other dimensions are unchanged.
    #[must_use]
    pub const fn rx_body<const SLOTS: usize, const BYTES: usize>(
        self,
    ) -> Resources<
        crate::profiles::WithRxBody<P, SLOTS, BYTES>,
        N,
        B,
        D,
        C,
        Fixed,
        REQUESTS,
        ASSOCIATIONS,
    > {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Set fixed TX complete-body slot count and bytes per slot.
    ///
    /// This preserves the selected block-wise mode. Bodies occupy no storage when
    /// disabled; when enabled, bytes must be a positive multiple of 1024 and
    /// the slot count must be positive. Other dimensions are unchanged.
    #[must_use]
    pub const fn tx_body<const SLOTS: usize, const BYTES: usize>(
        self,
    ) -> Resources<
        crate::profiles::WithTxBody<P, SLOTS, BYTES>,
        N,
        B,
        D,
        C,
        Fixed,
        REQUESTS,
        ASSOCIATIONS,
    > {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Set the shared client/server Observe-interest capacity before binding.
    ///
    /// Both roles consume this table; this does not reserve slots for either role.
    /// Zero disables retained interests. Calls and OSCORE request bindings remain
    /// independently bounded. Other profile dimensions are unchanged.
    ///
    /// ```
    /// use coaptic::resources::Device;
    /// let resources = Device::new().observe_entries::<6>().client_calls::<2>();
    /// assert_eq!(resources.pool_capacities().observe_entries, 6);
    /// ```
    #[must_use]
    pub const fn observe_entries<const M: usize>(
        self,
    ) -> Resources<crate::profiles::WithObserve<P, M>, N, B, D, C, Fixed, REQUESTS, ASSOCIATIONS>
    {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Set the duplicate-reply history capacity before binding.
    ///
    /// Other profile dimensions are unchanged. This does not reserve TX slots:
    /// cached wire replies share TX storage and may become empty-ACK tombstones.
    /// App retains its existing oldest-entry eviction policy when the table fills.
    /// Zero disables table history; only one non-idempotent request fallback is
    /// retained. Eviction or expiration can permit redispatch: this is not a
    /// durable exactly-once application-effect guarantee.
    ///
    /// ```
    /// use coaptic::resources::Device;
    /// let resources = Device::new().dedup_entries::<12>().observe_entries::<6>();
    /// assert_eq!(resources.pool_capacities().dedup_entries, 12);
    /// assert_eq!(resources.pool_capacities().observe_entries, 6);
    /// ```
    #[must_use]
    pub const fn dedup_entries<const M: usize>(
        self,
    ) -> Resources<crate::profiles::WithDedup<P, M>, N, B, D, C, Fixed, REQUESTS, ASSOCIATIONS>
    {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Select whether complete-body pools are present. Configure before allocation.
    #[must_use]
    pub const fn block_wise<const ENABLED: bool>(
        self,
    ) -> Resources<P, N, ENABLED, D, C, Fixed, REQUESTS, ASSOCIATIONS> {
        Resources {
            storage: Fixed,
            _profile: PhantomData,
        }
    }

    /// Use the same pool dimensions with bounded, fallible heap storage.
    ///
    /// The limit covers requested heap layouts, including metadata. It excludes
    /// inline App state, stack, allocator overhead and platform/application memory.
    /// No growth occurs after binding. Requires `alloc`, independently of `std`.
    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn allocated(
        self,
        heap_limit: usize,
    ) -> Resources<P, N, B, D, C, Allocated, REQUESTS, ASSOCIATIONS> {
        let capacities = self.pool_capacities();
        Resources {
            storage: Allocated {
                capacities,
                heap_limit,
            },
            _profile: PhantomData,
        }
    }
}

impl<
    P: MemoryProfile,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> Default for Resources<P, N, B, D, C, Fixed, REQUESTS, ASSOCIATIONS>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<
    P,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    S,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> Resources<P, N, B, D, C, S, REQUESTS, ASSOCIATIONS>
{
    /// Number of independently owned OSCORE contexts stored inline.
    pub const fn association_capacity(&self) -> usize {
        ASSOCIATIONS
    }

    /// Select association capacity without changing shared packet/body pools.
    pub fn associations<const M: usize>(self) -> Resources<P, N, B, D, C, S, REQUESTS, M> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }

    /// Inline OSCORE request-binding capacity, independent of Calls and packet pools.
    /// Without the `oscore` feature this table occupies no memory.
    #[must_use]
    pub const fn security_request_capacity(&self) -> usize {
        REQUESTS
    }

    /// Select the inline Token-to-request binding count for one OSCORE context.
    ///
    /// Supply a matching `oscore::BoundedSecurityContext<N>` when OSCORE is
    /// enabled. Zero refuses outbound protected requests while retaining authenticated
    /// server responses. This does not add security associations or alter replay state.
    /// Table memory remains inline for fixed and allocated Apps; it is outside the
    /// allocated pool heap limit. No resizing occurs while polling.
    #[must_use]
    pub fn security_requests<const M: usize>(self) -> Resources<P, N, B, D, C, S, M, ASSOCIATIONS> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }

    /// Configured inline client Call/completion capacity.
    #[must_use]
    pub const fn client_capacity(&self) -> usize {
        C
    }

    /// Configured route capacity.
    #[must_use]
    pub const fn route_capacity(&self) -> usize {
        N
    }

    /// Configured deferred-work capacity.
    #[must_use]
    pub const fn deferred_capacity(&self) -> usize {
        D
    }
    /// Set inline client Call/completion capacity, including retained Observe Calls.
    #[must_use]
    pub fn client_calls<const M: usize>(
        self,
    ) -> Resources<P, N, B, D, M, S, REQUESTS, ASSOCIATIONS> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }
    /// Set route capacity before routes are registered.
    #[must_use]
    pub fn routes<const M: usize>(self) -> Resources<P, M, B, D, C, S, REQUESTS, ASSOCIATIONS> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }
    /// Set inline deferred server-work capacity.
    #[must_use]
    pub fn deferred<const M: usize>(self) -> Resources<P, N, B, M, C, S, REQUESTS, ASSOCIATIONS> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }
}

#[cfg(feature = "alloc")]
impl<
    P: MemoryProfile,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> Resources<P, N, B, D, C, Allocated, REQUESTS, ASSOCIATIONS>
{
    /// Set the shared client/server Observe-interest count in allocated storage.
    ///
    /// Both roles consume this table; no slots are reserved for either role.
    /// Binding checks the existing heap limit, including table metadata, before
    /// allocation. No growth occurs while polling; other capacities are unchanged.
    #[must_use]
    pub fn observe_entries(mut self, entries: usize) -> Self {
        self.storage.capacities.observe_entries = entries;
        self
    }

    /// Set duplicate-reply history capacity in allocated storage.
    ///
    /// Binding checks the existing heap limit including table metadata before
    /// allocation or transport activity. No growth occurs while polling. Other
    /// capacities and App's existing eviction/fallback policy are unchanged;
    /// see the fixed configuration's `dedup_entries` documentation for limits.
    #[must_use]
    pub fn dedup_entries(mut self, entries: usize) -> Self {
        self.storage.capacities.dedup_entries = entries;
        self
    }

    /// Effective packet/body/table capacities, including runtime customization.
    #[must_use]
    pub const fn pool_capacities(&self) -> crate::storage::Capacities {
        self.storage.capacities
    }

    /// Checked requested heap bytes; this does not allocate or include inline App state.
    pub fn required_heap_bytes(&self) -> Result<usize, BuildError> {
        self.storage.capacities.validate_for_build(B)?;
        crate::storage::AllocMemory::<P::Token>::required_heap_bytes(self.storage.capacities)
    }

    /// Maximum requested heap bytes selected for this configuration.
    #[must_use]
    pub const fn heap_limit(&self) -> usize {
        self.storage.heap_limit
    }
    /// Customize runtime pool dimensions on the resource configuration.
    ///
    /// Body presence must match the selected block-wise mode. Bind validates
    /// dimensions and the heap limit before allocation or transport activity.
    /// Allocated datagrams cannot exceed the 1472-byte scratch ceiling; larger
    /// dimensions return [`BuildError::SizeMismatch`].
    #[must_use]
    pub fn capacities(mut self, capacities: crate::storage::Capacities) -> Self {
        self.storage.capacities = capacities;
        self
    }
}

/// Application setup after resource selection: routes, security and platform services.
///
/// Resource dimensions cannot be overridden here. Complete client responses are
/// retained by default; use [`App::take_response_into`] with caller-owned output.
pub struct ConfiguredBuilder<
    P: MemoryProfile,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    S,
    const REQUESTS: usize = 4,
    const ASSOCIATIONS: usize = 1,
> {
    inner: AppBuilder<P, Present, N, B, D, C, REQUESTS, ASSOCIATIONS>,
    storage: S,
}

impl App {
    /// Select resource bounds once, then configure security/routes and bind I/O.
    ///
    /// ```
    /// use coaptic::{App, resources::SmallDevice};
    /// let resources = SmallDevice::new().client_calls::<2>();
    /// let builder = App::with_resources(resources);
    /// // Supply randomness, a provisioned OSCORE context, routes and transport.
    /// ```
    #[must_use]
    pub fn with_resources<
        const ASSOCIATIONS: usize,
        P: MemoryProfile,
        const N: usize,
        const B: bool,
        const D: usize,
        const C: usize,
        S,
        const REQUESTS: usize,
    >(
        resources: Resources<P, N, B, D, C, S, REQUESTS, ASSOCIATIONS>,
    ) -> ConfiguredBuilder<P, N, B, D, C, S, REQUESTS, ASSOCIATIONS> {
        ConfiguredBuilder {
            inner: Self::profile_requests::<P, REQUESTS, ASSOCIATIONS>()
                .block_wise::<B>()
                .routes::<N>()
                .deferred::<D>()
                .client_calls::<C>()
                .full_responses(),
            storage: resources.storage,
        }
    }
}

impl<
    P: MemoryProfile,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    S,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> ConfiguredBuilder<P, N, B, D, C, S, REQUESTS, ASSOCIATIONS>
{
    /// Register a handler within the selected route capacity.
    /// Panics on invalid paths or a new path at capacity; use [`Self::try_route`]
    /// for recoverable refusal.
    #[must_use]
    pub fn route(mut self, path: impl IntoPath, methods: MethodRouter<P::Token>) -> Self {
        self.inner = self.inner.route(path, methods);
        self
    }
    /// Register a static path within the selected route capacity.
    #[must_use]
    pub fn route_path(mut self, path: &'static str, methods: MethodRouter<P::Token>) -> Self {
        self.inner = self.inner.route_path(path, methods);
        self
    }
    /// Checked registration within the selected capacity, preserving the builder
    /// on refusal. After registration, consume the builder with `bind`.
    ///
    /// ```
    /// use coaptic::{App, Request, Response, resources::SmallDevice};
    /// use coaptic::app::{get, RouteError};
    /// fn status(_: Request<'_>) -> Response<'static> { Response::content(b"ready") }
    /// let mut builder = App::with_resources(SmallDevice::new().routes::<1>());
    /// assert!(builder.try_route("status", get(status)).is_ok());
    /// assert_eq!(builder.try_route("extra", get(status)).err(), Some(RouteError::Full));
    /// assert!(builder.try_route("status", get(status)).is_ok());
    /// ```
    pub fn try_route(
        &mut self,
        path: impl IntoPath,
        methods: MethodRouter<P::Token>,
    ) -> Result<&mut Self, RouteError> {
        self.inner.try_route(path, methods)?;
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
    /// Remove a route before binding and reclaim the selected capacity.
    /// Invalid paths preserve the builder. Returns whether the path existed.
    ///
    /// ```
    /// use coaptic::{App, Request, Response, get, resources::SmallDevice};
    /// fn status(_: Request<'_>) -> Response<'static> { Response::content(b"ready") }
    /// let mut builder = App::with_resources(SmallDevice::new().routes::<1>())
    ///     .route("old", get(status));
    /// assert_eq!(builder.remove_route("old"), Ok(true));
    /// assert_eq!(builder.remove_route("missing"), Ok(false));
    /// assert!(builder.try_route("new", get(status)).is_ok());
    /// ```
    pub fn remove_route(&mut self, path: impl IntoPath) -> Result<bool, PathError> {
        self.inner.remove_route(path)
    }

    /// Serve discovery from the registered routes.
    #[must_use]
    pub fn well_known_core(mut self) -> Self {
        self.inner = self.inner.well_known_core();
        self
    }
    /// Supply platform secure randomness. Failure never substitutes predictable bytes.
    #[must_use]
    pub fn randomness(mut self, source: RandomSource) -> Self {
        self.inner = self.inner.randomness(source);
        self
    }
    /// Select request token length; see [`AppBuilder::token_length`].
    #[must_use]
    pub fn token_length(mut self, len: usize) -> Self {
        self.inner = self.inner.token_length(len);
        self
    }

    /// Select deterministic identity generation for tests only.
    #[must_use]
    pub fn deterministic_for_tests(mut self) -> Self {
        self.inner = self.inner.deterministic_for_tests();
        self
    }
    /// Permit unprotected CoAP; see [`AppBuilder::allow_unprotected`].
    #[must_use]
    pub fn allow_unprotected(mut self) -> Self {
        self.inner = self.inner.allow_unprotected();
        self
    }
    /// Supply a provisioned OSCORE context; protection is required by default.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn oscore(
        mut self,
        context: crate::oscore::BoundedSecurityContext<REQUESTS, P::Token>,
    ) -> Self {
        self.inner = self.inner.oscore(context);
        self
    }
    /// Refuse application traffic until an explicitly owned association is installed.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn require_associations(mut self) -> Self {
        self.inner = self.inner.require_associations();
        self
    }

    /// Require durable replay checkpointing before inbound effects.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn require_oscore_checkpoint(mut self) -> Self {
        self.inner = self.inner.require_oscore_checkpoint();
        self
    }
    /// Set the lifetime of admitted deferred work.
    #[must_use]
    pub fn deferred_lifetime(mut self, milliseconds: u64) -> Self {
        self.inner = self.inner.deferred_lifetime(milliseconds);
        self
    }
    /// Install the caller's Echo issuer/verifier.
    #[must_use]
    pub fn echo_policy(mut self, policy: EchoPolicy) -> Self {
        self.inner = self.inner.echo_policy(policy);
        self
    }
}

impl<
    P: MemoryLayout<B> + AppAssembled<B>,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> ConfiguredBuilder<P, N, B, D, C, Fixed, REQUESTS, ASSOCIATIONS>
{
    /// Bind fixed storage and a caller-owned stateful cryptographic RNG.
    /// See [`AppBuilder::bind_with_entropy`] for source ambiguity and failures.
    // Preserve the existing App type parameters; only the transport is wrapped.
    #[allow(clippy::type_complexity)]
    pub fn bind_with_entropy<T: DatagramIo, R: FnMut(&mut [u8]) -> bool>(
        self,
        io: T,
        random: R,
    ) -> Result<
        App<
            P,
            crate::platform::WithEntropy<T, R>,
            N,
            B,
            AppStore<P, B>,
            D,
            C,
            REQUESTS,
            ASSOCIATIONS,
        >,
        BuildError,
    > {
        self.inner.bind_with_entropy(io, random)
    }

    /// Bind fixed storage without allocating. The App owns its arrays and I/O.
    pub fn bind<T>(
        self,
        io: T,
    ) -> Result<App<P, T, N, B, AppStore<P, B>, D, C, REQUESTS, ASSOCIATIONS>, BuildError> {
        let Fixed = self.storage;
        self.inner.bind(io)
    }
}

#[cfg(feature = "alloc")]
impl<
    P: MemoryLayout<B> + AppAssembled<B>,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
    const REQUESTS: usize,
    const ASSOCIATIONS: usize,
> ConfiguredBuilder<P, N, B, D, C, Allocated, REQUESTS, ASSOCIATIONS>
{
    /// Bind bounded heap storage and a stateful cryptographic RNG, without `std`.
    /// See [`AppBuilder::bind_with_entropy`] for source ambiguity and failures.
    #[allow(clippy::type_complexity)]
    pub fn bind_with_entropy<T: DatagramIo, R: FnMut(&mut [u8]) -> bool>(
        self,
        io: T,
        random: R,
    ) -> Result<
        App<
            P,
            crate::platform::WithEntropy<T, R>,
            N,
            B,
            crate::storage::AllocMemory<P::Token>,
            D,
            C,
            REQUESTS,
            ASSOCIATIONS,
        >,
        BuildError,
    > {
        self.inner.validate_entropy_source()?;
        AppBuilder::<P, Present, N, B, D, C, REQUESTS, ASSOCIATIONS>::validate_scratch::<
            crate::storage::AllocMemory<P::Token>,
        >(self.storage.capacities)?;
        let (inner, io) = self.inner.prepare_entropy(io, random)?;
        ConfiguredBuilder {
            inner,
            storage: self.storage,
        }
        .bind(io)
    }

    /// Validate security, dimensions and budget, then allocate once and bind I/O.
    pub fn bind<T>(
        self,
        io: T,
    ) -> Result<
        App<P, T, N, B, crate::storage::AllocMemory<P::Token>, D, C, REQUESTS, ASSOCIATIONS>,
        BuildError,
    > {
        self.inner.validate_security()?;
        AppBuilder::<P, Present, N, B, D, C, REQUESTS, ASSOCIATIONS>::validate_scratch::<
            crate::storage::AllocMemory<P::Token>,
        >(self.storage.capacities)?;
        let storage = crate::storage::AllocMemory::<P::Token>::try_new(
            self.storage.capacities,
            self.storage.heap_limit,
        )?;
        self.inner.bind_storage(io, storage)
    }
}
