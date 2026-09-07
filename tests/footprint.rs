//! RAM locks for Default / Constrained App vs Memory (#93 / PERF-00).
//!
//! `.block_wise::<false>()` must not pay `WithBodies` padding or the
//! client Block2 assembled hold (`RESPONSE_BODY`). Host `usize=8`
//! magnitudes at `06a39cd`: Memory Default 13416, WithBodies 30312
//! (Δ16896), App-with-enum 33600.

use core::mem::size_of;

use coaptic::Response;
use coaptic::app::{App, DEFAULT_ROUTES, RESPONSE_BODY};
use coaptic::profiles;
use coaptic::storage::{Memory, MemoryLayout, WithBodies};

/// Block-wise App may pay the assembled hold on top of Memory body pools.
/// Padding to `App`'s alignment stays well under 64 bytes.
const ASSEMBLED_HOLD_PAD: usize = 64;

fn assert_datagram_omits_assembled(
    datagram_app: usize,
    block_wise_app: usize,
    datagram_mem: usize,
    block_wise_mem: usize,
) {
    assert!(
        datagram_mem < block_wise_mem,
        "Memory body pools must add RAM ({datagram_mem} !< {block_wise_mem})"
    );
    assert!(
        datagram_app < block_wise_mem,
        "datagram App ({datagram_app}) must not include WithBodies Memory ({block_wise_mem})"
    );
    let overhead = datagram_app.saturating_sub(datagram_mem);
    assert!(
        overhead < RESPONSE_BODY,
        "datagram App must not carry the assembled RESPONSE_BODY hold (App {datagram_app}, Memory {datagram_mem}, overhead {overhead})"
    );
    let mem_delta = block_wise_mem - datagram_mem;
    let app_delta = block_wise_app - datagram_app;
    assert!(
        app_delta >= mem_delta + RESPONSE_BODY,
        "block-wise App must include assembled hold beyond Memory body pools (App Δ {app_delta}, Memory Δ {mem_delta})"
    );
    let extra = app_delta - mem_delta;
    assert!(
        extra <= RESPONSE_BODY + ASSEMBLED_HOLD_PAD,
        "assembled hold extra {extra} exceeds RESPONSE_BODY + pad"
    );
}

#[test]
fn datagram_app_does_not_include_body_pools() {
    type Datagram = App<profiles::Default, ()>;
    type BlockWise = App<profiles::Default, (), DEFAULT_ROUTES, true>;

    let datagram_app = size_of::<Datagram>();
    let block_wise_app = size_of::<BlockWise>();
    let datagram_mem = size_of::<Memory<profiles::Default>>();
    let block_wise_mem = size_of::<Memory<profiles::Default, WithBodies<profiles::Default>>>();

    assert_datagram_omits_assembled(datagram_app, block_wise_app, datagram_mem, block_wise_mem);
}

#[test]
fn constrained_datagram_app_omits_body_pools() {
    type Datagram = App<profiles::Constrained, ()>;
    type BlockWise = App<profiles::Constrained, (), DEFAULT_ROUTES, true>;

    let datagram_app = size_of::<Datagram>();
    let block_wise_app = size_of::<BlockWise>();
    let datagram_mem = size_of::<Memory<profiles::Constrained>>();
    let block_wise_mem =
        size_of::<Memory<profiles::Constrained, WithBodies<profiles::Constrained>>>();

    assert_datagram_omits_assembled(datagram_app, block_wise_app, datagram_mem, block_wise_mem);
}

#[test]
fn memory_layout_selects_store() {
    fn assert_store<P: MemoryLayout<B>, const B: bool>() {}
    assert_store::<profiles::Default, false>();
    assert_store::<profiles::Default, true>();
    assert_eq!(
        size_of::<<profiles::Default as MemoryLayout<false>>::Store>(),
        size_of::<Memory<profiles::Default>>()
    );
    assert_eq!(
        size_of::<<profiles::Default as MemoryLayout<true>>::Store>(),
        size_of::<Memory<profiles::Default, WithBodies<profiles::Default>>>()
    );
}

#[test]
fn response_is_not_a_4kib_copy() {
    let n = size_of::<Response<'static>>();
    assert!(
        n < 768,
        "Response must not own [u8;4096] (got {n}; ~520 with inline 128 + location slices)"
    );
}
