//! Integration tests for `RustConn` core library
//!
//! This module contains integration tests that verify export/import round-trip
//! functionality for all supported formats.

// Allow common test patterns that Clippy warns about
#![allow(clippy::redundant_clone)]
#![allow(clippy::similar_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::expect_fun_call)]

mod integration;
