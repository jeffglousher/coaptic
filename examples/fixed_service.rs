#![no_std]
//! Shared fixed-storage device/host service; no board runtime is included.

#[path = "support/fixed_service.rs"]
pub mod fixed_service;
pub use fixed_service::{TELEMETRY, echo, server, telemetry};
