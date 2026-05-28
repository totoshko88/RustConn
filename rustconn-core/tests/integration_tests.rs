//! Integration tests for `RustConn` core library
//!
//! This module contains integration tests that verify export/import round-trip
//! functionality for all supported formats.

// Allow common test patterns that Clippy warns about
#![allow(
    clippy::redundant_clone,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::too_many_lines,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::expect_fun_call,
    reason = "module-wide override for legacy code; refactored case by case"
)]

mod integration;
