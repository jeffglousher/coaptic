//! Type-erased [`Resource`](super::Resource) for a heterogeneous [`Site`](super::Site).
//!
//! Public constructors are safe. Each [`ResourceDyn::new`] monomorphizes a
//! vtable of `fn(*mut (), …)` wrappers. The instance lives in an aligned
//! inline slot ([`MAX_RESOURCE_BYTES`] bytes, 16-byte align). Two `unsafe`
//! blocks reconstruct `&mut R` and run `drop_in_place` — the only `unsafe`
//! in this crate.

use core::mem::{align_of, size_of};
use core::ptr;

use super::reply::Reply;
use super::request::Request;
use super::resource::{Method, Resource};

/// Inline slot for one erased resource. Larger types panic at [`ResourceDyn::new`].
pub const MAX_RESOURCE_BYTES: usize = 128;

/// Alignment of the inline slot. Types with a greater alignment panic.
pub const MAX_RESOURCE_ALIGN: usize = 16;

#[repr(align(16))]
struct Slot([u8; MAX_RESOURCE_BYTES]);

type Call = fn(*mut (), Method, &Request<'_>) -> Reply;
type DropInPlace = fn(*mut ());

/// Owned, type-erased [`Resource`].
///
/// Built only by [`ResourceDyn::new`]. The site table stores these so `Temp`
/// and `Led` can sit in the same fixed array without an allocator.
pub struct ResourceDyn {
    slot: Slot,
    call: Call,
    drop: DropInPlace,
}

impl ResourceDyn {
    /// Move `resource` into an inline slot.
    ///
    /// # Panics
    ///
    /// If `R` is larger than [`MAX_RESOURCE_BYTES`] or more aligned than
    /// [`MAX_RESOURCE_ALIGN`]. Store a handle (for example an index into
    /// caller memory) or raise those constants.
    #[must_use]
    pub fn new<R: Resource>(resource: R) -> Self {
        assert!(
            size_of::<R>() <= MAX_RESOURCE_BYTES,
            "resource is {} bytes; Site inline slot is {MAX_RESOURCE_BYTES} (MAX_RESOURCE_BYTES)",
            size_of::<R>()
        );
        assert!(
            align_of::<R>() <= MAX_RESOURCE_ALIGN,
            "resource align is {}; Site inline slot align is {MAX_RESOURCE_ALIGN} (MAX_RESOURCE_ALIGN)",
            align_of::<R>()
        );

        let mut slot = Slot([0u8; MAX_RESOURCE_BYTES]);
        // SAFETY: `slot` is 16-byte aligned and large enough for `R`.
        // `ptr::write` moves `resource` in; we drop it exactly once via
        // `drop_in::<R>` in [`Drop`].
        unsafe {
            ptr::write(slot.0.as_mut_ptr().cast::<R>(), resource);
        }
        Self {
            slot,
            call: call::<R>,
            drop: drop_in::<R>,
        }
    }

    /// Dispatch `method` to the stored instance.
    #[must_use]
    pub fn call(&mut self, method: Method, req: &Request<'_>) -> Reply {
        (self.call)(self.slot.0.as_mut_ptr().cast::<()>(), method, req)
    }
}

impl Drop for ResourceDyn {
    fn drop(&mut self) {
        (self.drop)(self.slot.0.as_mut_ptr().cast::<()>());
    }
}

fn call<R: Resource>(ptr: *mut (), method: Method, req: &Request<'_>) -> Reply {
    // SAFETY: `ptr` is the inline slot from [`ResourceDyn::new`] for this `R`.
    // The slot is uniquely owned by this `ResourceDyn` for its lifetime.
    let resource = unsafe { &mut *ptr.cast::<R>() };
    match method {
        Method::Get => resource.get(req),
        Method::Post => resource.post(req),
        Method::Put => resource.put(req),
        Method::Delete => resource.delete(req),
        Method::Fetch => resource.fetch(req),
        Method::Patch => resource.patch(req),
        Method::IPatch => resource.ipatch(req),
    }
}

fn drop_in<R>(ptr: *mut ()) {
    // SAFETY: same provenance as [`call`]; runs once from [`ResourceDyn::drop`].
    unsafe {
        ptr::drop_in_place(ptr.cast::<R>());
    }
}
