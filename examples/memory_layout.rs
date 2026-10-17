#![no_std]
//! Target ABI layout probe. Compile with `cargo rustc --example memory_layout
//! --release -- --emit=llvm-ir` and inspect `COAPTIC_LAYOUT_BYTES` in the IR.
//! Values describe inline storage, excluding transport, stack and allocator backing.

use coaptic::app::{DEFAULT_ROUTES, LinkFormatScratch};
use coaptic::storage::{Memory, MemoryProfile};
use coaptic::{App, profiles};
use core::mem::size_of;

type Server = App<profiles::Default, (), DEFAULT_ROUTES, false, Memory<profiles::Default>, 0, 0>;
type OneCall = App<profiles::Default, (), DEFAULT_ROUTES, false, Memory<profiles::Default>, 0, 1>;

/// Ordered byte counts: pointer, Default Memory/App/blockwise App, Constrained
/// Memory/App/blockwise App, Gateway App/blockwise App, Service App/blockwise App,
/// zero-Call Default App, one-Call Default App, RX scratch, TX scratch, discovery scratch.
#[unsafe(no_mangle)]
pub static COAPTIC_LAYOUT_BYTES: [u32; 16] = [
    size_of::<usize>() as u32,
    size_of::<Memory<profiles::Default>>() as u32,
    size_of::<App<profiles::Default, ()>>() as u32,
    size_of::<App<profiles::Default, (), DEFAULT_ROUTES, true>>() as u32,
    size_of::<Memory<profiles::Constrained>>() as u32,
    size_of::<App<profiles::Constrained, ()>>() as u32,
    size_of::<App<profiles::Constrained, (), DEFAULT_ROUTES, true>>() as u32,
    size_of::<App<profiles::Gateway, ()>>() as u32,
    size_of::<App<profiles::Gateway, (), DEFAULT_ROUTES, true>>() as u32,
    size_of::<App<profiles::Service, ()>>() as u32,
    size_of::<App<profiles::Service, (), DEFAULT_ROUTES, true>>() as u32,
    size_of::<Server>() as u32,
    size_of::<OneCall>() as u32,
    size_of::<<profiles::Default as MemoryProfile>::RxScratch>() as u32,
    size_of::<<profiles::Default as MemoryProfile>::TxScratch>() as u32,
    size_of::<LinkFormatScratch<DEFAULT_ROUTES>>() as u32,
];

// Whole-object ceilings do not grow automatically with component sizes.
const _: () = {
    assert!(size_of::<App<profiles::Default, ()>>() <= 27 * 1024);
    assert!(size_of::<App<profiles::Default, (), DEFAULT_ROUTES, true>>() <= 49 * 1024);
    assert!(size_of::<App<profiles::Constrained, ()>>() <= 18 * 1024);
    assert!(size_of::<App<profiles::Constrained, (), DEFAULT_ROUTES, true>>() <= 31 * 1024);
};
