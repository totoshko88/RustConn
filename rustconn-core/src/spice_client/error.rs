//! SPICE client error types
//!
//! This module re-exports the unified `EmbeddedClientError` type as `SpiceClientError`
//! for backward compatibility. All error variants are shared across RDP, VNC, and SPICE clients.

pub use crate::embedded_client_error::SpiceClientError;
