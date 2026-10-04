//! Scalar C ABI for running the bounded Coaptic probe inside ESPHome/ESP-IDF.
//!
//! This adapter exercises real App loopback and optional OSCORE through a Rust
//! static library. It is a qualification component, not a networked CoAP driver
//! or a Taldra gateway. ESPHome owns its task, console and stack accounting.
//!
//! Generate a pinned ESPHome configuration with `tools/qualification/esphome.py`.
//! The resulting component links this crate using Rust 1.97.1 for the selected
//! C3/C6 target. Only scalar values cross the ABI; no Rust-owned pointers escape.
//! A panic terminates through ESP-IDF's `abort`, so missing captures cannot pass.
#![cfg_attr(target_os = "none", no_std)]

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
