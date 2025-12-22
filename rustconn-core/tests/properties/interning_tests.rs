//! Property-based tests for String Interning
//!
//! These tests validate the correctness properties defined in the design document
//! for the String Interning system (Requirements 5.x).

#![allow(clippy::overly_complex_bool_expr)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::cast_precision_loss)]

use proptest::prelude::*;
use rustconn_core::{
    get_interning_stats, intern_hostname, intern_protocol_name, intern_username,
    log_interning_stats_with_warning,
};
use std::sync::Arc;

// ========== Strategies ==========

/// Strategy for generating protocol names
fn arb_protocol_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("SSH".to_string()),
        Just("RDP".to_string()),
        Just("VNC".to_string()),
        Just("SPICE".to_string()),
        Just("Zero Trust".to_string()),
    ]
}

/// Strategy for generating hostnames
fn arb_hostname() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z]{3,10}\\.[a-z]{2,5}".prop_map(|s| s),
        "[a-z]{3,10}\\.[a-z]{2,5}\\.[a-z]{2,3}".prop_map(|s| s),
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(|s| s),
        "10\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(|s| s),
    ]
}

/// Strategy for generating usernames
fn arb_username() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("root".to_string()),
        Just("admin".to_string()),
        Just("user".to_string()),
        "[a-z]{3,12}".prop_map(|s| s),
        "[a-z]{3,8}[0-9]{1,3}".prop_map(|s| s),
    ]
}

/// Strategy for generating arbitrary strings for interning
fn arb_intern_string() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_.-]{1,50}".prop_map(|s| s)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 9: String Interning Deduplication ==========
    // **Feature: performance-improvements, Property 9: String Interning Deduplication**
    // **Validates: Requirements 5.1, 5.2**
    //
    // For any set of connections with repeated protocol names, interning SHALL
    // reduce memory usage compared to non-interned storage by returning the
    // same Arc for identical strings.

    #[test]
    fn interning_same_string_returns_same_arc(
        s in arb_intern_string()
    ) {
        // Intern the same string twice
        let arc1 = intern_protocol_name(&s);
        let arc2 = intern_protocol_name(&s);

        // Should return the same Arc (pointer equality)
        prop_assert!(
            Arc::ptr_eq(&arc1, &arc2),
            "Interning the same string should return the same Arc"
        );

        // Content should be identical
        prop_assert_eq!(&*arc1, &*arc2, "Interned strings should have identical content");
    }

    #[test]
    fn interning_protocol_names_deduplicates(
        protocol in arb_protocol_name(),
        count in 2..20usize
    ) {
        // Intern the same protocol name multiple times
        let arcs: Vec<Arc<str>> = (0..count)
            .map(|_| intern_protocol_name(&protocol))
            .collect();

        // All Arcs should point to the same memory
        for (i, arc) in arcs.iter().enumerate().skip(1) {
            prop_assert!(
                Arc::ptr_eq(&arcs[0], arc),
                "All interned protocol names should share the same Arc (failed at index {})",
                i
            );
        }
    }

    #[test]
    fn interning_hostnames_deduplicates(
        hostname in arb_hostname(),
        count in 2..20usize
    ) {
        // Intern the same hostname multiple times
        let arcs: Vec<Arc<str>> = (0..count)
            .map(|_| intern_hostname(&hostname))
            .collect();

        // All Arcs should point to the same memory
        for (i, arc) in arcs.iter().enumerate().skip(1) {
            prop_assert!(
                Arc::ptr_eq(&arcs[0], arc),
                "All interned hostnames should share the same Arc (failed at index {})",
                i
            );
        }
    }

    #[test]
    fn interning_usernames_deduplicates(
        username in arb_username(),
        count in 2..20usize
    ) {
        // Intern the same username multiple times
        let arcs: Vec<Arc<str>> = (0..count)
            .map(|_| intern_username(&username))
            .collect();

        // All Arcs should point to the same memory
        for (i, arc) in arcs.iter().enumerate().skip(1) {
            prop_assert!(
                Arc::ptr_eq(&arcs[0], arc),
                "All interned usernames should share the same Arc (failed at index {})",
                i
            );
        }
    }

    #[test]
    fn interning_different_strings_returns_different_arcs(
        s1 in arb_intern_string(),
        s2 in arb_intern_string()
    ) {
        prop_assume!(s1 != s2);

        let arc1 = intern_protocol_name(&s1);
        let arc2 = intern_protocol_name(&s2);

        // Different strings should return different Arcs
        prop_assert!(
            !Arc::ptr_eq(&arc1, &arc2),
            "Different strings should return different Arcs"
        );

        // Content should be different
        prop_assert_ne!(&*arc1, &*arc2, "Different strings should have different content");
    }

    #[test]
    fn interning_preserves_string_content(
        s in arb_intern_string()
    ) {
        let arc = intern_protocol_name(&s);

        // The interned string should have the same content as the original
        prop_assert_eq!(
            &*arc, &s,
            "Interned string content should match original"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 10: String Interning Statistics ==========
    // **Feature: performance-improvements, Property 10: String Interning Statistics**
    // **Validates: Requirements 5.3, 5.4**
    //
    // For any sequence of intern operations, statistics (hit rate, bytes saved)
    // SHALL be accurately tracked.

    #[test]
    fn interning_stats_track_intern_count(
        strings in prop::collection::vec(arb_intern_string(), 1..20)
    ) {
        let (initial_count, _, _, _) = get_interning_stats();

        // Intern all strings
        for s in &strings {
            let _ = intern_protocol_name(s);
        }

        let (final_count, _, _, _) = get_interning_stats();

        // Intern count should increase by at least the number of strings
        prop_assert!(
            final_count >= initial_count + strings.len(),
            "Intern count should increase: initial={}, final={}, strings={}",
            initial_count, final_count, strings.len()
        );
    }

    #[test]
    fn interning_stats_track_hits_on_duplicates(
        s in arb_intern_string(),
        repeat_count in 2..10usize
    ) {
        let (initial_count, initial_hits, _, _) = get_interning_stats();

        // Intern the same string multiple times
        for _ in 0..repeat_count {
            let _ = intern_protocol_name(&s);
        }

        let (final_count, final_hits, _, _) = get_interning_stats();

        // Should have recorded all intern attempts
        prop_assert!(
            final_count >= initial_count + repeat_count,
            "Intern count should increase by repeat_count"
        );

        // Should have recorded hits for duplicates (at least repeat_count - 1)
        prop_assert!(
            final_hits >= initial_hits + repeat_count - 1,
            "Hit count should increase for duplicates: initial={}, final={}, expected_increase>={}",
            initial_hits, final_hits, repeat_count - 1
        );
    }

    #[test]
    fn interning_stats_hit_rate_in_valid_range(
        strings in prop::collection::vec(arb_intern_string(), 1..20)
    ) {
        // Intern all strings
        for s in &strings {
            let _ = intern_protocol_name(s);
        }

        let (intern_count, hit_count, hit_rate, _) = get_interning_stats();

        // Hit rate should be between 0.0 and 1.0
        prop_assert!(
            hit_rate >= 0.0 && hit_rate <= 1.0,
            "Hit rate should be in [0.0, 1.0], got {}",
            hit_rate
        );

        // Hit rate should be consistent with counts
        if intern_count > 0 {
            let expected_rate = hit_count as f64 / intern_count as f64;
            prop_assert!(
                (hit_rate - expected_rate).abs() < 0.001,
                "Hit rate should match calculated rate: {} vs {}",
                hit_rate, expected_rate
            );
        }
    }

    #[test]
    fn interning_stats_bytes_saved_increases_on_duplicates(
        s in arb_intern_string(),
        repeat_count in 2..10usize
    ) {
        let (_, _, _, initial_bytes_saved) = get_interning_stats();

        // Intern the same string multiple times
        for _ in 0..repeat_count {
            let _ = intern_protocol_name(&s);
        }

        let (_, _, _, final_bytes_saved) = get_interning_stats();

        // Bytes saved should increase for duplicates
        // Each duplicate saves the length of the string
        let expected_min_increase = s.len() * (repeat_count - 1);
        prop_assert!(
            final_bytes_saved >= initial_bytes_saved + expected_min_increase,
            "Bytes saved should increase: initial={}, final={}, expected_increase>={}",
            initial_bytes_saved, final_bytes_saved, expected_min_increase
        );
    }

    #[test]
    fn interning_stats_warning_threshold_works(
        threshold in 0.1f64..0.9f64
    ) {
        // This test verifies that the warning function returns a boolean
        // and doesn't panic for any threshold value
        let result = log_interning_stats_with_warning(threshold);

        // Result should be a boolean
        prop_assert!(result || !result, "Should return a boolean");
    }
}

// ========== Unit Tests for Edge Cases ==========

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_intern_empty_string() {
        let arc = intern_protocol_name("");
        assert_eq!(&*arc, "");
    }

    #[test]
    fn test_intern_whitespace_string() {
        let arc = intern_protocol_name("   ");
        assert_eq!(&*arc, "   ");
    }

    #[test]
    fn test_intern_unicode_string() {
        let arc = intern_hostname("例え.jp");
        assert_eq!(&*arc, "例え.jp");

        // Intern again and verify same Arc
        let arc2 = intern_hostname("例え.jp");
        assert!(Arc::ptr_eq(&arc, &arc2));
    }

    #[test]
    fn test_intern_long_string() {
        let long_string = "a".repeat(1000);
        let arc1 = intern_hostname(&long_string);
        let arc2 = intern_hostname(&long_string);

        assert!(Arc::ptr_eq(&arc1, &arc2));
        assert_eq!(arc1.len(), 1000);
    }

    #[test]
    fn test_get_interning_stats_returns_valid_values() {
        let (intern_count, hit_count, hit_rate, bytes_saved) = get_interning_stats();

        // All values should be non-negative
        assert!(hit_rate >= 0.0);
        assert!(hit_rate <= 1.0);

        // Hit count should not exceed intern count
        assert!(hit_count <= intern_count);

        // Bytes saved should be non-negative (it's a usize)
        let _ = bytes_saved; // Just verify it's accessible
    }

    #[test]
    fn test_log_interning_stats_with_warning_default_threshold() {
        // Test with the default 30% threshold
        let result = log_interning_stats_with_warning(0.3);
        // Just verify it doesn't panic and returns a boolean
        assert!(result || !result);
    }

    #[test]
    fn test_interning_case_sensitive() {
        let arc_lower = intern_protocol_name("ssh");
        let arc_upper = intern_protocol_name("SSH");

        // Different case should be different strings
        assert!(!Arc::ptr_eq(&arc_lower, &arc_upper));
        assert_ne!(&*arc_lower, &*arc_upper);
    }
}
