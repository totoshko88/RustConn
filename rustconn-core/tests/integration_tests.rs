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
// New lints from IronRDP integration — suppress for existing test code
#![allow(
    unreachable_pub,
    reason = "test helpers use pub for cross-module visibility"
)]
#![allow(clippy::unwrap_used, reason = "tests use unwrap/expect for brevity")]
#![allow(
    clippy::wildcard_imports,
    reason = "test modules use prelude-style imports"
)]

mod integration;
