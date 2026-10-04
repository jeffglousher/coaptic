//! Scalar C ABI for running the bounded Coaptic probe inside ESPHome/ESP-IDF.
//!
//! This adapter exercises real App loopback and optional OSCORE through a Rust
//! static library. Feature `network` also supplies a live IPv4 UDP qualification
//! service. ESPHome owns sockets, tasks, console and stack accounting. This is
//! not yet a device driver, Home Assistant entity integration or Taldra gateway.
//!
//! Generate a pinned ESPHome configuration with `tools/qualification/esphome_probe.py`.
//! The component links this crate using Rust 1.97.1 for C3/C6 or Espressif Rust
//! 1.97.0.0 for S3. Network callbacks borrow buffers synchronously; no Rust-owned
//! pointers escape. The loopback ABI uses only scalar values.
//! A panic terminates through ESP-IDF's `abort`, so missing captures cannot pass.
#![cfg_attr(target_os = "none", no_std)]

#[cfg(feature = "network")]
pub mod network;

#[cfg(all(feature = "network", target_os = "none"))]
mod network_ffi;

/// ABI revision required by the ESPHome qualification component.
#[unsafe(no_mangle)]
pub extern "C" fn coaptic_probe_version() -> u32 {
    1
}

/// Returns one when the linked Rust probe includes OSCORE, otherwise zero.
#[unsafe(no_mangle)]
pub extern "C" fn coaptic_probe_oscore() -> u32 {
    u32::from(cfg!(feature = "oscore"))
}

/// Executes the finite loopback campaign; zero means pass and minus one refusal.
///
/// Call once from the owned qualification task. The caller supplies sufficient
/// stack and records its high-water mark. Success does not establish networking,
/// radio, platform entropy, allocator stress or durable storage integration.
#[unsafe(no_mangle)]
pub extern "C" fn coaptic_probe_run() -> i32 {
    if qualification_no_std::protocol_runtime_probe().is_ok() {
        0
    } else {
        -1
    }
}

#[cfg(target_os = "none")]
unsafe extern "C" {
    fn abort() -> !;
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { abort() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scalar_abi_executes_the_protocol_probe() {
        assert_eq!(super::coaptic_probe_version(), 1);
        assert_eq!(
            super::coaptic_probe_oscore(),
            u32::from(cfg!(feature = "oscore"))
        );
        assert_eq!(super::coaptic_probe_run(), 0);
    }
}
