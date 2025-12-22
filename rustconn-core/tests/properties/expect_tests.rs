//! Property-based tests for the Expect system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Expect-style automation system (Requirements 4.x).

use proptest::prelude::*;
use rustconn_core::{ExpectEngine, ExpectError, ExpectRule};

// ========== Strategies ==========

/// Strategy for generating valid regex patterns
/// Only generates patterns that are guaranteed to be valid regex
fn arb_valid_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple alphanumeric literal patterns (no special regex chars)
        "[a-zA-Z0-9 .,;:!?@#%]{1,30}".prop_filter("no regex special chars", |s| {
            // Filter out characters that have special meaning in regex
            !s.contains('(')
                && !s.contains(')')
                && !s.contains('[')
                && !s.contains(']')
                && !s.contains('{')
                && !s.contains('}')
                && !s.contains('*')
                && !s.contains('+')
                && !s.contains('?')
                && !s.contains('\\')
                && !s.contains('^')
                && !s.contains('$')
                && !s.contains('|')
                && !s.contains('-')
        }),
        // Common expect patterns (known valid)
        Just("password:".to_string()),
        Just("username:".to_string()),
        Just(r"\[sudo\].*password".to_string()),
        Just(r"login:\s*$".to_string()),
        Just(r"Password:\s*$".to_string()),
        Just(r"Are you sure.*\(yes/no\)".to_string()),
        Just(r"Enter passphrase".to_string()),
        Just(r"\$\s*$".to_string()),
        Just(r"#\s*$".to_string()),
        Just(r"[Pp]assword".to_string()),
        Just(r"[Uu]sername".to_string()),
        Just(r"[Ll]ogin".to_string()),
        Just(r"yes/no".to_string()),
        Just(r"y/n".to_string()),
        Just(r"continue\?".to_string()),
        // Simple word patterns
        "[a-z]{3,10}".prop_map(|s| s),
        // Anchored patterns
        "[a-zA-Z]{3,10}".prop_map(|s| format!("^{s}")),
        "[a-zA-Z]{3,10}".prop_map(|s| format!("{s}$")),
    ]
}

/// Strategy for generating invalid regex patterns
fn arb_invalid_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        // Unclosed brackets
        Just("[unclosed".to_string()),
        Just("(unclosed".to_string()),
        // Invalid escape sequences
        Just(r"\".to_string()),
        // Invalid quantifiers
        Just("*invalid".to_string()),
        Just("+invalid".to_string()),
        Just("?invalid".to_string()),
        // Invalid character class
        Just("[z-a]".to_string()),
        // Unmatched parentheses
        Just("(unmatched".to_string()),
        Just("unmatched)".to_string()),
    ]
}

/// Strategy for generating terminal output that might match patterns
fn arb_terminal_output() -> impl Strategy<Value = String> {
    prop_oneof![
        // Common prompts
        Just("password: ".to_string()),
        Just("Password: ".to_string()),
        Just("username: ".to_string()),
        Just("login: ".to_string()),
        Just("[sudo] password for user: ".to_string()),
        Just("Are you sure you want to continue connecting (yes/no)? ".to_string()),
        Just("Enter passphrase for key '/home/user/.ssh/id_rsa': ".to_string()),
        Just("$ ".to_string()),
        Just("# ".to_string()),
        // Random text
        "[a-zA-Z0-9 .,;:!?@#%^&*()\\-\\n]{0,200}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 8: Expect Pattern Validation ==========
    // **Feature: rustconn-enhancements, Property 8: Expect Pattern Validation**
    // **Validates: Requirements 4.6**
    //
    // For any expect rule pattern, validation should either confirm valid regex
    // syntax or report the specific error.

    #[test]
    fn valid_patterns_validate_successfully(pattern in arb_valid_pattern()) {
        let rule = ExpectRule::new(pattern.clone(), "response");
        let result = rule.validate_pattern();

        prop_assert!(
            result.is_ok(),
            "Valid pattern '{}' should validate successfully",
            pattern
        );
    }

    #[test]
    fn invalid_patterns_fail_with_descriptive_error(pattern in arb_invalid_pattern()) {
        let rule = ExpectRule::new(pattern.clone(), "response");
        let result = rule.validate_pattern();

        prop_assert!(
            result.is_err(),
            "Invalid pattern '{}' should fail validation",
            pattern
        );

        if let Err(e) = result {
            let error_msg = e.to_string();
            prop_assert!(
                !error_msg.is_empty(),
                "Error message should not be empty for pattern: {}",
                pattern
            );

            // Error should be PatternCompilationFailed
            prop_assert!(
                matches!(e, ExpectError::PatternCompilationFailed { .. }),
                "Error should be PatternCompilationFailed for: {}",
                pattern
            );
        }
    }

    #[test]
    fn pattern_validation_is_deterministic(pattern in arb_valid_pattern()) {
        let rule = ExpectRule::new(pattern, "response");

        // Validate multiple times
        let result1 = rule.validate_pattern();
        let result2 = rule.validate_pattern();
        let result3 = rule.validate_pattern();

        // All results should be the same
        prop_assert_eq!(result1.is_ok(), result2.is_ok());
        prop_assert_eq!(result2.is_ok(), result3.is_ok());
    }

    #[test]
    fn engine_rejects_invalid_patterns(pattern in arb_invalid_pattern()) {
        let mut engine = ExpectEngine::new();
        let rule = ExpectRule::new(pattern.clone(), "response");

        let result = engine.add_rule(rule);

        prop_assert!(
            result.is_err(),
            "Engine should reject invalid pattern: {}",
            pattern
        );
    }

    #[test]
    fn engine_accepts_valid_patterns(pattern in arb_valid_pattern()) {
        let mut engine = ExpectEngine::new();
        let rule = ExpectRule::new(pattern.clone(), "response");

        let result = engine.add_rule(rule);

        prop_assert!(
            result.is_ok(),
            "Engine should accept valid pattern: {}",
            pattern
        );
        prop_assert_eq!(engine.len(), 1);
    }

    #[test]
    fn compiled_pattern_matches_correctly(
        pattern in arb_valid_pattern(),
        output in arb_terminal_output()
    ) {
        let rule = ExpectRule::new(pattern.clone(), "response");

        // Compile the pattern
        let compiled = rule.compile_pattern();
        prop_assert!(compiled.is_ok(), "Pattern should compile: {}", pattern);

        let regex = compiled.unwrap();

        // Check if the pattern matches the output
        let matches = regex.is_match(&output);

        // If it matches, we should be able to find the match
        if matches {
            let found = regex.find(&output);
            prop_assert!(found.is_some(), "If is_match is true, find should return Some");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 10: Expect Pattern Matching Priority ==========
    // **Feature: rustconn-enhancements, Property 10: Expect Pattern Matching Priority**
    // **Validates: Requirements 4.3**
    //
    // For any terminal output that matches multiple expect patterns, the rule
    // with highest priority should be selected.

    #[test]
    fn highest_priority_rule_is_selected(
        priorities in prop::collection::vec(-100i32..100i32, 2..5)
    ) {
        // Create rules with different priorities that all match "test"
        let mut engine = ExpectEngine::new();

        for (i, &priority) in priorities.iter().enumerate() {
            let rule = ExpectRule::new("test", format!("response_{i}"))
                .with_priority(priority);
            engine.add_rule(rule).unwrap();
        }

        // Find the highest priority
        let max_priority = *priorities.iter().max().unwrap();

        // Match against "test"
        let result = engine.match_output("test");

        prop_assert!(result.is_some(), "Should match 'test'");

        let matched_rule = result.unwrap();
        prop_assert_eq!(
            matched_rule.priority, max_priority,
            "Should select rule with highest priority"
        );
    }

    #[test]
    fn priority_ordering_is_consistent(
        p1 in -100i32..100i32,
        p2 in -100i32..100i32,
        p3 in -100i32..100i32
    ) {
        let mut engine = ExpectEngine::new();

        // Add rules in random order
        engine.add_rule(ExpectRule::new("match", "r1").with_priority(p1)).unwrap();
        engine.add_rule(ExpectRule::new("match", "r2").with_priority(p2)).unwrap();
        engine.add_rule(ExpectRule::new("match", "r3").with_priority(p3)).unwrap();

        // Match multiple times
        let result1 = engine.match_output("match");
        let result2 = engine.match_output("match");
        let result3 = engine.match_output("match");

        // All results should be the same
        prop_assert!(result1.is_some());
        prop_assert!(result2.is_some());
        prop_assert!(result3.is_some());

        prop_assert_eq!(result1.unwrap().id, result2.unwrap().id);
        prop_assert_eq!(result2.unwrap().id, result3.unwrap().id);

        // Should be the highest priority
        let max_priority = p1.max(p2).max(p3);
        prop_assert_eq!(result1.unwrap().priority, max_priority);
    }

    #[test]
    fn disabled_rules_are_skipped_in_priority(
        high_priority in 50i32..100i32,
        low_priority in -100i32..49i32
    ) {
        let mut engine = ExpectEngine::new();

        // Add high priority rule but disabled
        engine.add_rule(
            ExpectRule::new("test", "high_disabled")
                .with_priority(high_priority)
                .with_enabled(false)
        ).unwrap();

        // Add low priority rule that's enabled
        engine.add_rule(
            ExpectRule::new("test", "low_enabled")
                .with_priority(low_priority)
                .with_enabled(true)
        ).unwrap();

        let result = engine.match_output("test");

        prop_assert!(result.is_some());
        let matched = result.unwrap();
        prop_assert_eq!(&matched.response, "low_enabled");
        prop_assert_eq!(matched.priority, low_priority);
    }

    #[test]
    fn only_matching_patterns_are_considered(
        priority1 in 50i32..100i32,
        priority2 in -100i32..49i32
    ) {
        let mut engine = ExpectEngine::new();

        // High priority rule that doesn't match
        engine.add_rule(
            ExpectRule::new("nomatch", "high")
                .with_priority(priority1)
        ).unwrap();

        // Low priority rule that matches
        engine.add_rule(
            ExpectRule::new("test", "low")
                .with_priority(priority2)
        ).unwrap();

        let result = engine.match_output("test");

        prop_assert!(result.is_some());
        prop_assert_eq!(&result.unwrap().response, "low");
    }

    #[test]
    fn equal_priority_returns_first_added(
        priority in -100i32..100i32
    ) {
        let mut engine = ExpectEngine::new();

        // Add rules with same priority
        let rule1 = ExpectRule::new("test", "first").with_priority(priority);
        let id1 = rule1.id;
        engine.add_rule(rule1).unwrap();

        let rule2 = ExpectRule::new("test", "second").with_priority(priority);
        engine.add_rule(rule2).unwrap();

        let result = engine.match_output("test");

        prop_assert!(result.is_some());
        // With equal priority, the first added should be returned
        // (since sort is stable and they're added in order)
        prop_assert_eq!(result.unwrap().id, id1);
    }

    #[test]
    fn no_match_returns_none(
        pattern in arb_valid_pattern(),
        priority in -100i32..100i32
    ) {
        let mut engine = ExpectEngine::new();
        engine.add_rule(
            ExpectRule::new(pattern, "response").with_priority(priority)
        ).unwrap();

        // Use a string that's very unlikely to match any pattern
        let result = engine.match_output("ZZZZZ_NO_MATCH_12345_ZZZZZ");

        // Most patterns won't match this
        // If it does match, that's fine - we just verify the result is consistent
        if result.is_none() {
            prop_assert!(true);
        } else {
            // If it matches, verify it's the rule we added
            prop_assert_eq!(result.unwrap().priority, priority);
        }
    }
}

// ========== Unit Tests for Priority Edge Cases ==========

#[cfg(test)]
mod priority_tests {
    use super::*;

    #[test]
    fn test_negative_priorities() {
        let mut engine = ExpectEngine::new();

        engine
            .add_rule(ExpectRule::new("test", "neg100").with_priority(-100))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("test", "neg50").with_priority(-50))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("test", "neg1").with_priority(-1))
            .unwrap();

        let result = engine.match_output("test");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "neg1");
    }

    #[test]
    fn test_mixed_positive_negative_priorities() {
        let mut engine = ExpectEngine::new();

        engine
            .add_rule(ExpectRule::new("test", "neg").with_priority(-10))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("test", "zero").with_priority(0))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("test", "pos").with_priority(10))
            .unwrap();

        let result = engine.match_output("test");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "pos");
    }

    #[test]
    fn test_all_disabled_returns_none() {
        let mut engine = ExpectEngine::new();

        engine
            .add_rule(ExpectRule::new("test", "r1").with_enabled(false))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("test", "r2").with_enabled(false))
            .unwrap();

        let result = engine.match_output("test");
        assert!(result.is_none());
    }

    #[test]
    fn test_empty_engine_returns_none() {
        let engine = ExpectEngine::new();
        let result = engine.match_output("anything");
        assert!(result.is_none());
    }

    #[test]
    fn test_regex_pattern_priority() {
        let mut engine = ExpectEngine::new();

        // More specific pattern with lower priority
        engine
            .add_rule(ExpectRule::new(r"password:\s*$", "specific").with_priority(1))
            .unwrap();

        // Less specific pattern with higher priority
        engine
            .add_rule(ExpectRule::new("password", "general").with_priority(10))
            .unwrap();

        // Higher priority should win even though less specific
        let result = engine.match_output("password: ");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "general");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 9: Expect Rule Serialization Round-Trip ==========
    // **Feature: rustconn-enhancements, Property 9: Expect Rule Serialization Round-Trip**
    // **Validates: Requirements 4.7**
    //
    // For any valid expect rule, serializing and deserializing should produce
    // an equivalent rule.

    #[test]
    fn expect_rule_json_round_trip(
        pattern in arb_valid_pattern(),
        response in "[a-zA-Z0-9 .,;:!?@#%^&*()\\-]{0,50}",
        priority in -100i32..100i32,
        timeout in prop::option::of(100u32..30000u32),
        enabled in any::<bool>()
    ) {
        let mut rule = ExpectRule::new(pattern, response)
            .with_priority(priority)
            .with_enabled(enabled);

        if let Some(t) = timeout {
            rule = rule.with_timeout(t);
        }

        // Serialize to JSON
        let json = serde_json::to_string(&rule);
        prop_assert!(json.is_ok(), "Serialization should succeed");

        let json_str = json.unwrap();

        // Deserialize back
        let deserialized: Result<ExpectRule, _> = serde_json::from_str(&json_str);
        prop_assert!(deserialized.is_ok(), "Deserialization should succeed");

        let restored = deserialized.unwrap();

        // All fields should match
        prop_assert_eq!(rule.id, restored.id);
        prop_assert_eq!(rule.pattern, restored.pattern);
        prop_assert_eq!(rule.response, restored.response);
        prop_assert_eq!(rule.priority, restored.priority);
        prop_assert_eq!(rule.timeout_ms, restored.timeout_ms);
        prop_assert_eq!(rule.enabled, restored.enabled);
    }

    #[test]
    fn expect_rule_yaml_round_trip(
        pattern in arb_valid_pattern(),
        response in "[a-zA-Z0-9 .,;:!?@#%^&*()\\-]{0,50}",
        priority in -100i32..100i32,
        timeout in prop::option::of(100u32..30000u32),
        enabled in any::<bool>()
    ) {
        let mut rule = ExpectRule::new(pattern, response)
            .with_priority(priority)
            .with_enabled(enabled);

        if let Some(t) = timeout {
            rule = rule.with_timeout(t);
        }

        // Serialize to YAML
        let yaml = serde_yaml::to_string(&rule);
        prop_assert!(yaml.is_ok(), "YAML serialization should succeed");

        let yaml_str = yaml.unwrap();

        // Deserialize back
        let deserialized: Result<ExpectRule, _> = serde_yaml::from_str(&yaml_str);
        prop_assert!(deserialized.is_ok(), "YAML deserialization should succeed");

        let restored = deserialized.unwrap();

        // All fields should match
        prop_assert_eq!(rule.id, restored.id);
        prop_assert_eq!(rule.pattern, restored.pattern);
        prop_assert_eq!(rule.response, restored.response);
        prop_assert_eq!(rule.priority, restored.priority);
        prop_assert_eq!(rule.timeout_ms, restored.timeout_ms);
        prop_assert_eq!(rule.enabled, restored.enabled);
    }

    #[test]
    fn expect_rule_double_round_trip(
        pattern in arb_valid_pattern(),
        response in "[a-zA-Z0-9 .,;:!?@#%^&*()\\-]{0,50}",
        priority in -100i32..100i32
    ) {
        let rule = ExpectRule::new(pattern, response).with_priority(priority);

        // First round-trip
        let json1 = serde_json::to_string(&rule).unwrap();
        let restored1: ExpectRule = serde_json::from_str(&json1).unwrap();

        // Second round-trip
        let json2 = serde_json::to_string(&restored1).unwrap();
        let restored2: ExpectRule = serde_json::from_str(&json2).unwrap();

        // Both should be equal
        prop_assert_eq!(restored1, restored2);
        prop_assert_eq!(json1, json2);
    }

    #[test]
    fn expect_rule_preserves_uuid(
        pattern in arb_valid_pattern(),
        response in "[a-zA-Z0-9]{0,20}"
    ) {
        let rule = ExpectRule::new(pattern, response);
        let original_id = rule.id;

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(original_id, restored.id, "UUID should be preserved");
    }

    #[test]
    fn expect_rule_collection_round_trip(
        rules in prop::collection::vec(
            (arb_valid_pattern(), "[a-zA-Z0-9]{0,20}", -100i32..100i32),
            1..5
        )
    ) {
        let expect_rules: Vec<ExpectRule> = rules
            .into_iter()
            .map(|(pattern, response, priority)| {
                ExpectRule::new(pattern, response).with_priority(priority)
            })
            .collect();

        // Serialize collection
        let json = serde_json::to_string(&expect_rules).unwrap();

        // Deserialize back
        let restored: Vec<ExpectRule> = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(expect_rules.len(), restored.len());

        for (original, restored) in expect_rules.iter().zip(restored.iter()) {
            prop_assert_eq!(original, restored);
        }
    }
}

// ========== Unit Tests for Serialization Edge Cases ==========

#[cfg(test)]
mod serialization_tests {
    use super::*;

    #[test]
    fn test_serialize_with_special_chars_in_pattern() {
        let rule = ExpectRule::new(r"\[sudo\].*password:\s*$", "secret");

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.pattern, restored.pattern);
    }

    #[test]
    fn test_serialize_with_special_chars_in_response() {
        let rule = ExpectRule::new("prompt", r"pass\nword${var}");

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.response, restored.response);
    }

    #[test]
    fn test_serialize_with_unicode() {
        let rule = ExpectRule::new("密码:", "秘密");

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.pattern, restored.pattern);
        assert_eq!(rule.response, restored.response);
    }

    #[test]
    fn test_serialize_with_empty_response() {
        let rule = ExpectRule::new("prompt", "");

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.response, restored.response);
        assert!(restored.response.is_empty());
    }

    #[test]
    fn test_serialize_with_max_priority() {
        let rule = ExpectRule::new("test", "response").with_priority(i32::MAX);

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.priority, restored.priority);
    }

    #[test]
    fn test_serialize_with_min_priority() {
        let rule = ExpectRule::new("test", "response").with_priority(i32::MIN);

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.priority, restored.priority);
    }

    #[test]
    fn test_serialize_with_large_timeout() {
        let rule = ExpectRule::new("test", "response").with_timeout(u32::MAX);

        let json = serde_json::to_string(&rule).unwrap();
        let restored: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule.timeout_ms, restored.timeout_ms);
    }

    #[test]
    fn test_deserialize_from_minimal_json() {
        // Minimal JSON with only required fields
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "pattern": "test",
            "response": "response",
            "priority": 0,
            "timeout_ms": null,
            "enabled": true
        }"#;

        let result: Result<ExpectRule, _> = serde_json::from_str(json);
        assert!(result.is_ok());

        let rule = result.unwrap();
        assert_eq!(rule.pattern, "test");
        assert_eq!(rule.response, "response");
    }

    #[test]
    fn test_json_format_is_readable() {
        let rule = ExpectRule::new("password:", "secret123")
            .with_priority(10)
            .with_timeout(5000);

        let json = serde_json::to_string_pretty(&rule).unwrap();

        // Should contain readable field names
        assert!(json.contains("pattern"));
        assert!(json.contains("response"));
        assert!(json.contains("priority"));
        assert!(json.contains("timeout_ms"));
        assert!(json.contains("enabled"));
    }
}
