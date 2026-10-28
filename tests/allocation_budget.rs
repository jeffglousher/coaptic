//! Observe real allocation requests and inject refusal on this test thread only.
#![cfg(feature = "alloc")]

use coaptic::message::{OwnedToken, Token};
use coaptic::storage::{AllocMemory, Capacities, Storage};
use coaptic::{App, BuildError, profiles};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
struct Record {
    attempts: usize,
    requested: usize,
    live: usize,
    fail_at: Option<usize>,
}

thread_local! {
    static RECORD: Cell<Option<Record>> = const { Cell::new(None) };
}

struct FaultAllocator;

// Test-only allocator: fail a selected request, otherwise delegate to System.
unsafe impl GlobalAlloc for FaultAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let fail = RECORD
            .try_with(|cell| {
                let Some(mut record) = cell.get() else {
                    return false;
                };
                record.attempts += 1;
                let fail = record.fail_at == Some(record.attempts);
                cell.set(Some(record));
                fail
            })
            .unwrap_or(false);
        if fail {
            return std::ptr::null_mut();
        }
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let _ = RECORD.try_with(|cell| {
                if let Some(mut record) = cell.get() {
                    record.requested += layout.size();
                    record.live += layout.size();
                    cell.set(Some(record));
                }
            });
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let _ = RECORD.try_with(|cell| {
            if let Some(mut record) = cell.get() {
                record.live -= layout.size();
                cell.set(Some(record));
            }
        });
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: FaultAllocator = FaultAllocator;

fn track<R>(fail_at: Option<usize>, run: impl FnOnce() -> R) -> (R, Record) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RECORD.with(|cell| cell.set(None));
        }
    }
    RECORD.with(|cell| {
        cell.set(Some(Record {
            fail_at,
            ..Record::default()
        }))
    });
    let reset = Reset;
    let result = run();
    let record = RECORD.with(|cell| cell.get().unwrap());
    drop(reset);
    (result, record)
}

fn exercise<T: OwnedToken>(c: Capacities, attempts: usize) {
    let required = AllocMemory::<T>::required_heap_bytes(c).unwrap();
    let (result, baseline) = track(None, || {
        let mut memory = AllocMemory::<T>::try_new(c, required)?;
        // Exhaustion and recovery operate without growing any pool.
        let id = memory.rx_datagram().acquire().unwrap();
        for _ in 1..c.rx_datagram_slots {
            assert!(memory.rx_datagram().acquire().is_some());
        }
        assert!(memory.rx_datagram().acquire().is_none());
        memory.rx_datagram().release(id).unwrap();
        assert!(memory.rx_datagram().acquire().is_some());
        assert_eq!(memory.capacities(), c);
        Ok::<_, BuildError>(())
    });
    assert_eq!(result, Ok(()));
    assert_eq!(
        baseline.requested, required,
        "metadata accounting must match actual layouts"
    );
    assert_eq!(baseline.live, 0);
    assert_eq!(baseline.attempts, attempts, "startup allocation requests");

    let (error, record) = track(None, || {
        AllocMemory::<T>::try_new(c, required - 1).unwrap_err()
    });
    assert_eq!(
        error,
        BuildError::HeapBudgetExceeded {
            required,
            limit: required - 1
        }
    );
    assert_eq!(
        record.attempts, 0,
        "budget refusal precedes every allocation"
    );

    for index in 1..=baseline.attempts {
        let (error, record) = track(Some(index), || {
            AllocMemory::<T>::try_new(c, required).unwrap_err()
        });
        assert_eq!(error, BuildError::AllocationFailed, "allocation {index}");
        assert_eq!(record.attempts, index);
        assert_eq!(
            record.live, 0,
            "partial construction leaked at allocation {index}"
        );
        let (recovered, record) = track(None, || AllocMemory::<T>::try_new(c, required).map(drop));
        assert_eq!(recovered, Ok(()));
        assert_eq!(record.live, 0);
        assert_eq!(record.requested, required);
    }
}

#[test]
fn exact_budget_and_every_allocation_failure_without_bodies() {
    exercise::<Token>(Capacities::from_profile::<profiles::Constrained>(), 16);
    exercise::<Token>(Capacities::from_profile::<profiles::Default>(), 20);
}

#[test]
fn exact_budget_and_every_allocation_failure_with_bodies() {
    exercise::<Token>(
        Capacities::from_profile::<profiles::Constrained>()
            .with_block_wise::<profiles::Constrained>(),
        24,
    );
    exercise::<Token>(
        Capacities::from_profile::<profiles::Default>().with_block_wise::<profiles::Default>(),
        30,
    );
}

#[test]
fn impossible_layout_and_aggregate_overflow_refuse_without_allocating() {
    let base = Capacities::from_profile::<profiles::Default>();
    let mut cases = [base; 10];
    cases[0].rx_datagram_slots = usize::MAX;
    cases[1].rx_datagram_bytes = usize::MAX;
    // Each payload layout fits, but four such payloads exceed usize.
    cases[2].rx_datagram_bytes = isize::MAX as usize;
    cases[3].dedup_entries = usize::MAX;
    cases[4].observe_entries = usize::MAX;
    cases[5].tx_datagram_slots = usize::MAX;
    cases[6] = base.with_block_wise::<profiles::Default>();
    cases[6].rx_body_slots = Some(usize::MAX);
    cases[7] = base.with_block_wise::<profiles::Default>();
    cases[7].tx_body_slots = Some(usize::MAX);
    cases[8] = base.with_block_wise::<profiles::Default>();
    cases[8].rx_body_bytes = Some(isize::MAX as usize & !1023);
    cases[8].rx_body_slots = Some(4);
    cases[9] = base.with_block_wise::<profiles::Default>();
    cases[9].tx_body_bytes = Some(isize::MAX as usize & !1023);
    cases[9].tx_body_slots = Some(4);
    for c in cases {
        let (error, record) = track(None, || {
            AllocMemory::<coaptic::message::Token>::try_new(c, usize::MAX).unwrap_err()
        });
        assert_eq!(error, BuildError::CapacityOverflow);
        assert_eq!(record.attempts, 0);
    }
}

#[test]
fn incomplete_body_configuration_refuses_before_allocating() {
    let mut c = Capacities::from_profile::<profiles::Default>();
    c.rx_body_slots = Some(1);
    let (error, record) = track(None, || {
        AllocMemory::<coaptic::message::Token>::try_new(c, usize::MAX).unwrap_err()
    });
    assert_eq!(error, BuildError::IncompleteBodyCapacities);
    assert_eq!(record.attempts, 0);
}

#[test]
fn budgeted_storage_binds_to_app_and_legacy_allocation_is_fallible() {
    let c = Capacities::from_profile::<profiles::Default>();
    let required = AllocMemory::<coaptic::message::Token>::required_heap_bytes(c).unwrap();
    let memory = AllocMemory::<coaptic::message::Token>::try_new(c, required).unwrap();
    let _app = App::builder()
        .allow_unprotected()
        .deterministic_for_tests()
        .bind_storage((), memory)
        .unwrap();
    let (error, record) = track(Some(1), || {
        App::builder()
            .allow_unprotected()
            .deterministic_for_tests()
            .bind_alloc((), c)
            .err()
    });
    assert_eq!(error, Some(BuildError::AllocationFailed));
    assert_eq!(record.live, 0);
    assert!(
        App::builder()
            .allow_unprotected()
            .deterministic_for_tests()
            .bind_alloc((), c)
            .is_ok()
    );
}

#[test]
fn missing_security_refuses_before_any_allocation() {
    let c = Capacities::from_profile::<profiles::Default>();
    let (error, record) = track(Some(1), || {
        App::builder()
            .deterministic_for_tests()
            .bind_alloc((), c)
            .err()
    });
    assert_eq!(error, Some(BuildError::SecurityRequired));
    assert_eq!(record.attempts, 0);
}

#[test]
fn configured_heap_refusals_precede_allocation_and_allow_recovery() {
    use coaptic::resources::SmallDevice;
    let (error, record) = track(Some(1), || {
        App::with_resources(SmallDevice::new().allocated(64 * 1024))
            .deterministic_for_tests()
            .bind(())
            .err()
    });
    assert_eq!(error, Some(BuildError::SecurityRequired));
    assert_eq!(record.attempts, 0);

    let required = SmallDevice::new()
        .allocated(64 * 1024)
        .required_heap_bytes()
        .unwrap();
    let (error, record) = track(Some(1), || {
        App::with_resources(SmallDevice::new().allocated(required - 1))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(())
            .err()
    });
    assert_eq!(
        error,
        Some(BuildError::HeapBudgetExceeded {
            required,
            limit: required - 1
        })
    );
    assert_eq!(record.attempts, 0);

    let mut c = SmallDevice::new().pool_capacities();
    c.rx_datagram_bytes = 1473;
    let (error, record) = track(Some(1), || {
        App::with_resources(SmallDevice::new().allocated(64 * 1024).capacities(c))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(())
            .err()
    });
    assert_eq!(error, Some(BuildError::SizeMismatch));
    assert_eq!(record.attempts, 0);

    let (result, record) = track(None, || {
        App::with_resources(SmallDevice::new().allocated(required))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(())
            .map(drop)
    });
    assert_eq!(result, Ok(()));
    assert_eq!(record.requested, required);
    assert_eq!(record.live, 0);
}

#[test]
fn legacy_app_scratch_capacity_refuses_before_allocation() {
    for rx in [true, false] {
        let mut c = Capacities::from_profile::<profiles::Default>();
        if rx {
            c.rx_datagram_bytes += 1;
        } else {
            c.tx_datagram_bytes += 1;
        }
        let (result, record) = track(None, || {
            App::builder()
                .allow_unprotected()
                .deterministic_for_tests()
                .bind_alloc((), c)
                .map(drop)
        });
        assert_eq!(result, Err(BuildError::SizeMismatch), "RX={rx}");
        assert_eq!(record.attempts, 0);
    }
}

#[test]
fn scratch_limit_refuses_before_entropy_and_recovers_at_exact_limit() {
    use coaptic::storage::DatagramIo;
    struct NoIo;
    impl coaptic::storage::DatagramTransport for NoIo {
        type Error = ();
    }

    impl DatagramIo for NoIo {
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, coaptic::Endpoint)>, ()> {
            panic!("no receive during bind")
        }
        fn send(&mut self, _: coaptic::Endpoint, _: &[u8]) -> Result<usize, ()> {
            panic!("no send during bind")
        }
    }
    for rx in [true, false] {
        let mut c = Capacities::from_profile::<profiles::Default>();
        if rx {
            c.rx_datagram_bytes += 1;
        } else {
            c.tx_datagram_bytes += 1;
        }
        let entropy = Cell::new(0);
        let mut random = |out: &mut [u8]| {
            entropy.set(entropy.get() + 1);
            out.fill(7);
            true
        };
        let (result, record) = track(None, || {
            App::builder()
                .allow_unprotected()
                .bind_alloc_with_entropy(NoIo, c, &mut random)
                .map(drop)
        });
        assert_eq!(result, Err(BuildError::SizeMismatch));
        assert_eq!(record.attempts, 0);
        assert_eq!(entropy.get(), 0);
        let (result, record) = track(None, || {
            App::with_resources(
                coaptic::resources::Resources::<profiles::Default>::new()
                    .allocated(usize::MAX)
                    .capacities(c),
            )
            .allow_unprotected()
            .bind_with_entropy(NoIo, &mut random)
            .map(drop)
        });
        assert_eq!(result, Err(BuildError::SizeMismatch));
        assert_eq!(record.attempts, 0);
        assert_eq!(entropy.get(), 0);
        // Engine-only storage remains legal; App adds the scratch admission rule.
        let memory = AllocMemory::<coaptic::message::Token>::try_new(c, usize::MAX).unwrap();
        assert_eq!(
            App::builder()
                .allow_unprotected()
                .bind_storage_with_entropy(NoIo, memory, &mut random)
                .err(),
            Some(BuildError::SizeMismatch)
        );
        assert_eq!(entropy.get(), 0);
        let c = Capacities::from_profile::<profiles::Default>();
        assert!(
            App::builder()
                .allow_unprotected()
                .bind_alloc_with_entropy(NoIo, c, &mut random)
                .is_ok()
        );
        assert_eq!(entropy.get(), 1);
        let memory = AllocMemory::<coaptic::message::Token>::try_new(c, usize::MAX).unwrap();
        assert!(
            App::builder()
                .allow_unprotected()
                .deterministic_for_tests()
                .bind_storage(NoIo, memory)
                .is_ok()
        );
    }
}

#[test]
fn zero_and_asymmetric_tables_have_exact_requests_and_recover() {
    let mut c = Capacities::from_profile::<profiles::Default>();
    c.rx_datagram_slots = 1;
    // One RX owner needs metadata, occupied, pin and one payload request (4).
    // Each nonempty table adds exactly metadata + occupancy (2).
    for (dedup, observe, tx, attempts) in [
        (0, 0, 0, 4),
        (1, 0, 0, 6),
        (0, 2, 0, 6),
        (0, 0, 3, 12),
        (1, 2, 3, 16),
        (3, 1, 1, 14),
    ] {
        c.dedup_entries = dedup;
        c.observe_entries = observe;
        c.tx_datagram_slots = tx;
        exercise::<Token>(c, attempts);
        // Asymmetric real body pools retain both pin arrays: 4 + 6 requests.
        let mut bodies = c.with_block_wise::<profiles::Default>();
        bodies.rx_body_slots = Some(1);
        bodies.tx_body_slots = Some(3);
        exercise::<Token>(bodies, attempts + 10);
    }
    c.rx_datagram_slots = 0;
    c.tx_datagram_slots = 0;
    c.dedup_entries = 0;
    c.observe_entries = 0;
    assert_eq!(AllocMemory::<Token>::required_heap_bytes(c), Ok(0));
    let (result, record) = track(None, || AllocMemory::<Token>::try_new(c, 0).map(drop));
    assert_eq!(result, Ok(()));
    assert_eq!(record.attempts, 0);
    assert_eq!(record.requested, 0);
    assert_eq!(record.live, 0);
}

#[test]
fn selected_token_layout_has_exact_requests_and_failure_recovery() {
    exercise::<coaptic::message::extended::Token<32>>(
        Capacities::from_profile::<profiles::Default>().with_block_wise::<profiles::Default>(),
        30,
    );
}
