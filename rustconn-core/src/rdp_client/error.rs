//! RDP client error types
//!
//! This module re-exports the unified `EmbeddedClientError` type as `RdpClientError`
//! for backward compatibility. All error variants are shared across RDP, VNC, and SPICE clients.

pub use crate::embedded_client_error::RdpClientError;
