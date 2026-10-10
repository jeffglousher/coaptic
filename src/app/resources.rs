//! Select resource bounds once, then bind platform services with [`App::with_resources`].
//!
//! Presets are starting budgets, not power, throughput or whole-device RAM promises.
//! All support fixed storage without `std` or `alloc`. With `alloc`, call
//! [`Resources::allocated`] with an explicit heap request limit before construction.
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
/// construction remains available for compatibility with earlier callers.
pub struct Resources<
    P = crate::profiles::Default,
    const ROUTES: usize = 8,
    const BLOCK_WISE: bool = false,
    const DEFERRED: usize = 0,
    const CALLS: usize = 4,
    S = Fixed,
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
/// This retains the current single OSCORE context/four live request-binding limit.
/// More packet storage does not add security associations or establish throughput.
pub type Service = Resources<crate::profiles::Service, 32, true, 8, 4>;

impl<P: MemoryProfile, const N: usize, const B: bool, const D: usize, const C: usize>
    Resources<P, N, B, D, C>
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

    /// Select whether complete-body pools are present. Configure before allocation.
    #[must_use]
    pub const fn block_wise<const ENABLED: bool>(self) -> Resources<P, N, ENABLED, D, C> {
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
    pub fn allocated(self, heap_limit: usize) -> Resources<P, N, B, D, C, Allocated> {
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

impl<P: MemoryProfile, const N: usize, const B: bool, const D: usize, const C: usize> Default
    for Resources<P, N, B, D, C>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<P, const N: usize, const B: bool, const D: usize, const C: usize, S>
    Resources<P, N, B, D, C, S>
{
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
    pub fn client_calls<const M: usize>(self) -> Resources<P, N, B, D, M, S> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }
    /// Set route capacity before routes are registered.
    #[must_use]
    pub fn routes<const M: usize>(self) -> Resources<P, M, B, D, C, S> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }
    /// Set inline deferred server-work capacity.
    #[must_use]
    pub fn deferred<const M: usize>(self) -> Resources<P, N, B, M, C, S> {
        Resources {
            storage: self.storage,
            _profile: PhantomData,
        }
    }
}

#[cfg(feature = "alloc")]
impl<P, const N: usize, const B: bool, const D: usize, const C: usize>
    Resources<P, N, B, D, C, Allocated>
{
    /// Effective packet/body/table capacities, including runtime customization.
    #[must_use]
    pub const fn pool_capacities(&self) -> crate::storage::Capacities {
        self.storage.capacities
    }

    /// Checked requested heap bytes; this does not allocate or include inline App state.
    pub fn required_heap_bytes(&self) -> Result<usize, BuildError> {
        self.storage.capacities.validate_for_build(B)?;
        crate::storage::AllocMemory::required_heap_bytes(self.storage.capacities)
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
> {
    inner: AppBuilder<P, Present, N, B, D, C>,
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
        P: MemoryProfile,
        const N: usize,
        const B: bool,
        const D: usize,
        const C: usize,
        S,
    >(
        resources: Resources<P, N, B, D, C, S>,
    ) -> ConfiguredBuilder<P, N, B, D, C, S> {
        ConfiguredBuilder {
            inner: Self::profile::<P>()
                .block_wise::<B>()
                .routes::<N>()
                .deferred::<D>()
                .client_calls::<C>()
                .full_responses(),
            storage: resources.storage,
        }
    }
}

impl<P: MemoryProfile, const N: usize, const B: bool, const D: usize, const C: usize, S>
    ConfiguredBuilder<P, N, B, D, C, S>
{
    /// Register a handler within the selected route capacity.
    #[must_use]
    pub fn route(mut self, path: impl IntoPath, methods: MethodRouter) -> Self {
        self.inner = self.inner.route(path, methods);
        self
    }
    /// Register a static path within the selected route capacity.
    #[must_use]
    pub fn route_path(mut self, path: &'static str, methods: MethodRouter) -> Self {
        self.inner = self.inner.route_path(path, methods);
        self
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
    /// Select deterministic identity generation for tests only.
    #[must_use]
    pub fn deterministic_for_tests(mut self) -> Self {
        self.inner = self.inner.deterministic_for_tests();
        self
    }
    /// Explicit unprotected compatibility/test mode; does not override OSCORE.
    #[must_use]
    pub fn allow_plaintext(mut self) -> Self {
        self.inner = self.inner.allow_plaintext();
        self
    }
    /// Supply a provisioned OSCORE context; protection is required by default.
    #[cfg(feature = "oscore")]
    #[must_use]
    pub fn oscore(mut self, context: crate::oscore::SecurityContext) -> Self {
        self.inner = self.inner.oscore(context);
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
> ConfiguredBuilder<P, N, B, D, C, Fixed>
{
    /// Bind fixed storage without allocating. The App owns its arrays and I/O.
    pub fn bind<T>(self, io: T) -> Result<App<P, T, N, B, AppStore<P, B>, D, C>, BuildError> {
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
> ConfiguredBuilder<P, N, B, D, C, Allocated>
{
    /// Validate security, dimensions and budget, then allocate once and bind I/O.
    pub fn bind<T>(
        self,
        io: T,
    ) -> Result<App<P, T, N, B, crate::storage::AllocMemory, D, C>, BuildError> {
        self.inner.validate_security()?;
        self.storage.capacities.validate_for_build(B)?;
        let c = self.storage.capacities;
        if c.rx_datagram_bytes > <crate::storage::AllocMemory as DatagramSlots>::RX_DATAGRAM_BYTES
            || c.tx_datagram_bytes
                > <crate::storage::AllocMemory as DatagramSlots>::TX_DATAGRAM_BYTES
        {
            return Err(BuildError::SizeMismatch);
        }
        let storage =
            crate::storage::AllocMemory::try_new(self.storage.capacities, self.storage.heap_limit)?;
        self.inner.bind_storage(io, storage)
    }
}
