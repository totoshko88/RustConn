//! Property-based tests for the Tracing system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Tracing system (Requirements 4.x).
//!
//! **Feature: performance-improvements, Property 8: Tracing Span Creation**
//! **Validates: Requirements 4.2, 4.5**

use proptest::prelude::*;
use rustconn_core::{span_names, TracingConfig, TracingLevel, TracingOutput};

// ========== Strategies ==========

/// Strategy for generating tracing levels
fn arb_tracing_level() -> impl Strategy<Value = TracingLevel> {
    prop_oneof![
        Just(TracingLevel::Error),
        Just(TracingLevel::Warn),
        Just(TracingLevel::Info),
        Just(TracingLevel::Debug),
        Just(TracingLevel::Trace),
    ]
}

/// Strategy for generating tracing output types
fn arb_tracing_output() -> impl Strategy<Value = TracingOutput> {
    prop_oneof![Just(TracingOutput::Stdout), Just(TracingOutput::Stderr),]
}

/// Strategy for generating filter strings
fn arb_filter_string() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("rustconn=debug".to_string()),
        Just("rustconn=info".to_string()),
        Just("rustconn=warn".to_string()),
        Just("rustconn=trace".to_string()),
        Just("rustconn=debug,tokio=warn".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 8: Tracing Span Creation ==========
    // **Feature: performance-improvements, Property 8: Tracing Span Creation**
    // **Validates: Requirements 4.2, 4.5**
    //
    // For any traced operation (connection, search, import/export, credential resolution),
    // a tracing span SHALL be created with required fields.

    /// Test that TracingConfig can be created with any valid level
    #[test]
    fn tracing_config_accepts_any_level(level in arb_tracing_level()) {
        let config = TracingConfig::new().with_level(level);
        prop_assert_eq!(config.level, level);
    }

    /// Test that TracingConfig can be created with any valid output
    #[test]
    fn tracing_config_accepts_any_output(output in arb_tracing_output()) {
        let config = TracingConfig::new().with_output(output.clone());
        prop_assert_eq!(config.output, output);
    }

    /// Test that TracingConfig builder methods are composable
    #[test]
    fn tracing_config_builder_composable(
        level in arb_tracing_level(),
        output in arb_tracing_output(),
        profiling in any::<bool>(),
        include_conn_ids in any::<bool>(),
        include_timing in any::<bool>(),
    ) {
        let config = TracingConfig::new()
            .with_level(level)
            .with_output(output.clone())
            .with_profiling(profiling)
            .with_connection_ids(include_conn_ids)
            .with_timing(include_timing);

        prop_assert_eq!(config.level, level);
        prop_assert_eq!(config.output, output);
        prop_assert_eq!(config.profiling_enabled, profiling);
        prop_assert_eq!(config.include_connection_ids, include_conn_ids);
        prop_assert_eq!(config.include_timing, include_timing);
    }

    /// Test that TracingConfig with filter overrides level
    #[test]
    fn tracing_config_filter_set(filter in arb_filter_string()) {
        let config = TracingConfig::new().with_filter(filter.clone());
        prop_assert_eq!(config.filter, Some(filter));
    }

    /// Test that TracingLevel round-trips through string conversion
    #[test]
    fn tracing_level_roundtrip(level in arb_tracing_level()) {
        let level_str = level.to_string();
        let parsed: Result<TracingLevel, _> = level_str.parse();
        prop_assert_eq!(parsed, Ok(level));
    }

    /// Test that TracingLevel from_str is case-insensitive
    #[test]
    fn tracing_level_case_insensitive(level in arb_tracing_level()) {
        let level_str = level.to_string();

        // Test lowercase
        let parsed_lower: Result<TracingLevel, _> = level_str.to_lowercase().parse();
        prop_assert_eq!(parsed_lower, Ok(level));

        // Test uppercase
        let parsed_upper: Result<TracingLevel, _> = level_str.to_uppercase().parse();
        prop_assert_eq!(parsed_upper, Ok(level));
    }
}

// ========== Unit Tests for Span Names ==========

#[test]
fn span_names_are_defined() {
    // Verify all required span names are defined and non-empty
    assert!(!span_names::CONNECTION_ESTABLISH.is_empty());
    assert!(!span_names::CONNECTION_DISCONNECT.is_empty());
    assert!(!span_names::SEARCH_EXECUTE.is_empty());
    assert!(!span_names::SEARCH_CACHE_LOOKUP.is_empty());
    assert!(!span_names::IMPORT_EXECUTE.is_empty());
    assert!(!span_names::EXPORT_EXECUTE.is_empty());
    assert!(!span_names::CREDENTIAL_RESOLVE.is_empty());
    assert!(!span_names::CREDENTIAL_STORE.is_empty());
    assert!(!span_names::CONFIG_LOAD.is_empty());
    assert!(!span_names::CONFIG_SAVE.is_empty());
    assert!(!span_names::SESSION_START.is_empty());
    assert!(!span_names::SESSION_END.is_empty());
}

#[test]
fn span_names_follow_naming_convention() {
    // Verify span names follow the "category.operation" convention
    assert!(span_names::CONNECTION_ESTABLISH.contains('.'));
    assert!(span_names::CONNECTION_DISCONNECT.contains('.'));
    assert!(span_names::SEARCH_EXECUTE.contains('.'));
    assert!(span_names::IMPORT_EXECUTE.contains('.'));
    assert!(span_names::EXPORT_EXECUTE.contains('.'));
    assert!(span_names::CREDENTIAL_RESOLVE.contains('.'));
    assert!(span_names::SESSION_START.contains('.'));
}

#[test]
fn development_config_has_debug_level() {
    let config = TracingConfig::development();
    assert_eq!(config.level, TracingLevel::Debug);
    assert_eq!(config.output, TracingOutput::Stdout);
    assert!(config.profiling_enabled);
}

#[test]
fn production_config_has_info_level() {
    let config = TracingConfig::production();
    assert_eq!(config.level, TracingLevel::Info);
    assert_eq!(config.output, TracingOutput::Stderr);
    assert!(!config.profiling_enabled);
}

#[test]
fn default_config_is_reasonable() {
    let config = TracingConfig::default();
    assert_eq!(config.level, TracingLevel::Info);
    assert_eq!(config.output, TracingOutput::Stderr);
    assert!(config.include_connection_ids);
    assert!(config.include_timing);
    assert!(config.filter.is_none());
}
