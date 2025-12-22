//! Property-based tests for the Key Sequence system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Key Sequence system (Requirements 2.x).

use proptest::prelude::*;
use rustconn_core::{KeyElement, KeySequence, KeySequenceError, SpecialKey};

// ========== Strategies ==========

/// Strategy for generating valid special keys
fn arb_special_key() -> impl Strategy<Value = SpecialKey> {
    prop_oneof![
        Just(SpecialKey::Enter),
        Just(SpecialKey::Tab),
        Just(SpecialKey::Escape),
        Just(SpecialKey::Backspace),
        Just(SpecialKey::Delete),
        Just(SpecialKey::Up),
        Just(SpecialKey::Down),
        Just(SpecialKey::Left),
        Just(SpecialKey::Right),
        Just(SpecialKey::Home),
        Just(SpecialKey::End),
        Just(SpecialKey::PageUp),
        Just(SpecialKey::PageDown),
        Just(SpecialKey::Insert),
        Just(SpecialKey::F1),
        Just(SpecialKey::F2),
        Just(SpecialKey::F3),
        Just(SpecialKey::F4),
        Just(SpecialKey::F5),
        Just(SpecialKey::F6),
        Just(SpecialKey::F7),
        Just(SpecialKey::F8),
        Just(SpecialKey::F9),
        Just(SpecialKey::F10),
        Just(SpecialKey::F11),
        Just(SpecialKey::F12),
        Just(SpecialKey::CtrlC),
        Just(SpecialKey::CtrlD),
        Just(SpecialKey::CtrlZ),
        Just(SpecialKey::CtrlA),
        Just(SpecialKey::CtrlE),
        Just(SpecialKey::CtrlL),
        Just(SpecialKey::Space),
    ]
}

/// Strategy for generating valid variable names
fn arb_var_name() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_]{0,15}"
}

/// Strategy for generating safe text (no special characters that need escaping)
fn arb_safe_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 .,;:!?@#%^&*()\\[\\]<>/-]{0,30}".prop_filter("no special chars", |s| {
        !s.contains("${") && !s.contains('{') && !s.contains('}') && !s.contains('$')
    })
}

/// Strategy for generating wait durations
fn arb_wait_duration() -> impl Strategy<Value = u32> {
    0u32..10000u32
}

/// Strategy for generating a single key element
fn arb_key_element() -> impl Strategy<Value = KeyElement> {
    prop_oneof![
        arb_safe_text().prop_map(KeyElement::Text),
        arb_special_key().prop_map(KeyElement::SpecialKey),
        arb_wait_duration().prop_map(KeyElement::Wait),
        arb_var_name().prop_map(KeyElement::Variable),
    ]
}

/// Strategy for generating a key sequence
///
/// Reserved for future property tests that need complete key sequence generation.
/// Currently, tests use more targeted element-based strategies.
#[allow(dead_code)]
fn arb_key_sequence() -> impl Strategy<Value = KeySequence> {
    prop::collection::vec(arb_key_element(), 0..10).prop_map(|elements| {
        // Filter out empty text elements
        let filtered: Vec<KeyElement> = elements
            .into_iter()
            .filter(|e| {
                if let KeyElement::Text(t) = e {
                    !t.is_empty()
                } else {
                    true
                }
            })
            .collect();
        KeySequence::from_elements(filtered)
    })
}

/// Strategy for generating invalid key sequence strings
fn arb_invalid_key_sequence() -> impl Strategy<Value = String> {
    prop_oneof![
        // Unclosed brace
        Just("{ENTER".to_string()),
        Just("text{TAB".to_string()),
        Just("${var".to_string()),
        // Empty key name
        Just("{}".to_string()),
        Just("${}".to_string()),
        // Unknown key
        "[A-Z]{5,10}".prop_map(|s| format!("{{{s}}}")),
        // Invalid wait duration
        Just("{WAIT:abc}".to_string()),
        Just("{WAIT:-100}".to_string()),
        // Invalid variable name (starts with number)
        "[0-9][a-z]{3,5}".prop_map(|s| format!("${{{s}}}")),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 6: Key Sequence Validation ==========
    // **Feature: rustconn-enhancements, Property 6: Key Sequence Validation**
    // **Validates: Requirements 2.6**
    //
    // For any key sequence string, parsing should either succeed with a valid
    // KeySequence or fail with a descriptive error for malformed input.

    #[test]
    fn key_sequence_parsing_succeeds_or_fails_with_error(
        input in "[a-zA-Z0-9 .,;:!?@#%^&*()\\[\\]<>/-]{0,50}"
    ) {
        // Any input should either parse successfully or return a descriptive error
        let result = KeySequence::parse(&input);

        match result {
            Ok(seq) => {
                // If parsing succeeds, the sequence should be valid
                prop_assert!(seq.validate().is_ok(), "Parsed sequence should be valid");
            }
            Err(e) => {
                // If parsing fails, error should be descriptive
                let error_msg = e.to_string();
                prop_assert!(
                    !error_msg.is_empty(),
                    "Error message should not be empty"
                );
            }
        }
    }

    #[test]
    fn valid_special_keys_parse_successfully(key in arb_special_key()) {
        let input = format!("{{{}}}", key.as_str());
        let result = KeySequence::parse(&input);

        prop_assert!(result.is_ok(), "Valid special key should parse: {}", input);

        let seq = result.unwrap();
        prop_assert_eq!(seq.elements.len(), 1);
        prop_assert_eq!(&seq.elements[0], &KeyElement::SpecialKey(key));
    }

    #[test]
    fn valid_wait_commands_parse_successfully(duration in arb_wait_duration()) {
        let input = format!("{{WAIT:{}}}", duration);
        let result = KeySequence::parse(&input);

        prop_assert!(result.is_ok(), "Valid wait command should parse: {}", input);

        let seq = result.unwrap();
        prop_assert_eq!(seq.elements.len(), 1);
        prop_assert_eq!(&seq.elements[0], &KeyElement::Wait(duration));
    }

    #[test]
    fn valid_variables_parse_successfully(name in arb_var_name()) {
        let input = format!("${{{}}}", name);
        let result = KeySequence::parse(&input);

        prop_assert!(result.is_ok(), "Valid variable should parse: {}", input);

        let seq = result.unwrap();
        prop_assert_eq!(seq.elements.len(), 1);
        prop_assert_eq!(&seq.elements[0], &KeyElement::Variable(name));
    }

    #[test]
    fn plain_text_parses_successfully(text in arb_safe_text()) {
        let result = KeySequence::parse(&text);

        prop_assert!(result.is_ok(), "Plain text should parse: {}", text);

        let seq = result.unwrap();
        if text.is_empty() {
            prop_assert!(seq.is_empty());
        } else {
            prop_assert_eq!(seq.elements.len(), 1);
            prop_assert_eq!(&seq.elements[0], &KeyElement::Text(text));
        }
    }

    #[test]
    fn invalid_sequences_fail_with_descriptive_error(input in arb_invalid_key_sequence()) {
        let result = KeySequence::parse(&input);

        // Should fail
        prop_assert!(result.is_err(), "Invalid sequence should fail: {}", input);

        // Error should be descriptive
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        prop_assert!(
            !error_msg.is_empty(),
            "Error message should not be empty for: {}",
            input
        );

        // Error should be one of the expected types
        prop_assert!(
            matches!(
                error,
                KeySequenceError::UnclosedBrace(_)
                    | KeySequenceError::EmptyKeyName(_)
                    | KeySequenceError::UnknownKey(_)
                    | KeySequenceError::InvalidSyntax(_)
                    | KeySequenceError::InvalidWaitDuration(_)
            ),
            "Error should be a known type: {:?}",
            error
        );
    }

    #[test]
    fn special_key_parsing_is_case_insensitive(key in arb_special_key()) {
        let key_str = key.as_str();

        // Test uppercase
        let upper = format!("{{{}}}", key_str.to_uppercase());
        let upper_result = KeySequence::parse(&upper);
        prop_assert!(upper_result.is_ok(), "Uppercase should parse: {}", upper);

        // Test lowercase
        let lower = format!("{{{}}}", key_str.to_lowercase());
        let lower_result = KeySequence::parse(&lower);
        prop_assert!(lower_result.is_ok(), "Lowercase should parse: {}", lower);

        // Both should produce the same key
        let upper_seq = upper_result.unwrap();
        let lower_seq = lower_result.unwrap();
        prop_assert_eq!(upper_seq, lower_seq, "Case should not matter");
    }

    #[test]
    fn escaped_braces_are_preserved(text in "[a-zA-Z0-9]{1,10}") {
        // Test escaped opening brace
        let input = format!("{{{{{}}}}}", text);  // {{text}}
        let result = KeySequence::parse(&input);

        prop_assert!(result.is_ok(), "Escaped braces should parse: {}", input);

        let seq = result.unwrap();
        prop_assert_eq!(seq.elements.len(), 1);

        // Should contain literal braces
        if let KeyElement::Text(t) = &seq.elements[0] {
            prop_assert!(t.contains('{') && t.contains('}'), "Should contain literal braces");
        } else {
            prop_assert!(false, "Should be text element");
        }
    }

    #[test]
    fn escaped_dollar_is_preserved(text in "[a-zA-Z0-9]{1,10}") {
        let input = format!("$${}", text);  // $$text
        let result = KeySequence::parse(&input);

        prop_assert!(result.is_ok(), "Escaped dollar should parse: {}", input);

        let seq = result.unwrap();
        prop_assert_eq!(seq.elements.len(), 1);

        // Should contain literal dollar
        if let KeyElement::Text(t) = &seq.elements[0] {
            prop_assert!(t.starts_with('$'), "Should start with literal dollar");
        } else {
            prop_assert!(false, "Should be text element");
        }
    }
}

// ========== Unit Tests for Edge Cases ==========

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_unclosed_brace_error() {
        let result = KeySequence::parse("{ENTER");
        assert!(matches!(result, Err(KeySequenceError::UnclosedBrace(_))));
    }

    #[test]
    fn test_unclosed_variable_error() {
        let result = KeySequence::parse("${var");
        assert!(matches!(result, Err(KeySequenceError::UnclosedBrace(_))));
    }

    #[test]
    fn test_empty_key_name_error() {
        let result = KeySequence::parse("{}");
        assert!(matches!(result, Err(KeySequenceError::EmptyKeyName(_))));
    }

    #[test]
    fn test_empty_variable_name_error() {
        let result = KeySequence::parse("${}");
        assert!(matches!(result, Err(KeySequenceError::EmptyKeyName(_))));
    }

    #[test]
    fn test_unknown_key_error() {
        let result = KeySequence::parse("{UNKNOWNKEY}");
        assert!(matches!(result, Err(KeySequenceError::InvalidSyntax(_))));
    }

    #[test]
    fn test_invalid_wait_duration_error() {
        let result = KeySequence::parse("{WAIT:abc}");
        assert!(matches!(
            result,
            Err(KeySequenceError::InvalidWaitDuration(_))
        ));
    }

    #[test]
    fn test_invalid_variable_name_error() {
        let result = KeySequence::parse("${123invalid}");
        assert!(matches!(result, Err(KeySequenceError::InvalidSyntax(_))));
    }

    #[test]
    fn test_complex_valid_sequence() {
        let input = "user{TAB}${password}{ENTER}{WAIT:500}sudo su{ENTER}";
        let result = KeySequence::parse(input);
        assert!(result.is_ok());

        let seq = result.unwrap();
        assert_eq!(seq.elements.len(), 7);
    }

    #[test]
    fn test_multiple_variables() {
        let input = "${user}@${host}:${port}";
        let result = KeySequence::parse(input);
        assert!(result.is_ok());

        let seq = result.unwrap();
        let vars = seq.variable_references();
        assert_eq!(vars.len(), 3);
        assert!(vars.contains(&"user"));
        assert!(vars.contains(&"host"));
        assert!(vars.contains(&"port"));
    }

    #[test]
    fn test_all_function_keys() {
        for i in 1..=12 {
            let input = format!("{{F{}}}", i);
            let result = KeySequence::parse(&input);
            assert!(result.is_ok(), "F{} should parse", i);
        }
    }

    #[test]
    fn test_all_ctrl_combinations() {
        let ctrl_keys = ["CTRL+C", "CTRL+D", "CTRL+Z", "CTRL+A", "CTRL+E", "CTRL+L"];
        for key in ctrl_keys {
            let input = format!("{{{}}}", key);
            let result = KeySequence::parse(&input);
            assert!(result.is_ok(), "{} should parse", key);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 5: Key Sequence Parse Round-Trip ==========
    // **Feature: rustconn-enhancements, Property 5: Key Sequence Parse Round-Trip**
    // **Validates: Requirements 2.7**
    //
    // For any valid key sequence string, parsing and then serializing should
    // produce an equivalent string.

    #[test]
    fn key_sequence_round_trip_from_elements(
        elements in prop::collection::vec(arb_key_element(), 0..10)
    ) {
        // Filter out empty text elements
        let filtered: Vec<KeyElement> = elements
            .into_iter()
            .filter(|e| {
                if let KeyElement::Text(t) = e {
                    !t.is_empty()
                } else {
                    true
                }
            })
            .collect();

        let seq = KeySequence::from_elements(filtered);

        // Serialize to string
        let serialized = seq.to_string();

        // Parse back
        let reparsed = KeySequence::parse(&serialized);
        prop_assert!(reparsed.is_ok(), "Serialized sequence should parse: {}", serialized);

        let reparsed_seq = reparsed.unwrap();

        // The reparsed sequence should serialize to the same string
        // (Note: consecutive text elements may be merged during parsing)
        let reserialized = reparsed_seq.to_string();
        prop_assert_eq!(
            &serialized, &reserialized,
            "Serialized strings should match after round-trip"
        );

        // Double round-trip should be stable
        let reparsed2 = KeySequence::parse(&reserialized).unwrap();
        prop_assert_eq!(
            reparsed_seq, reparsed2,
            "Double round-trip should be stable"
        );
    }

    #[test]
    fn key_sequence_round_trip_special_keys(key in arb_special_key()) {
        let seq = KeySequence::from_elements(vec![KeyElement::SpecialKey(key)]);

        let serialized = seq.to_string();
        let reparsed = KeySequence::parse(&serialized).unwrap();

        prop_assert_eq!(seq, reparsed, "Special key should round-trip");
    }

    #[test]
    fn key_sequence_round_trip_wait(duration in arb_wait_duration()) {
        let seq = KeySequence::from_elements(vec![KeyElement::Wait(duration)]);

        let serialized = seq.to_string();
        let reparsed = KeySequence::parse(&serialized).unwrap();

        prop_assert_eq!(seq, reparsed, "Wait should round-trip");
    }

    #[test]
    fn key_sequence_round_trip_variable(name in arb_var_name()) {
        let seq = KeySequence::from_elements(vec![KeyElement::Variable(name)]);

        let serialized = seq.to_string();
        let reparsed = KeySequence::parse(&serialized).unwrap();

        prop_assert_eq!(seq, reparsed, "Variable should round-trip");
    }

    #[test]
    fn key_sequence_round_trip_text(text in arb_safe_text()) {
        prop_assume!(!text.is_empty());

        let seq = KeySequence::from_elements(vec![KeyElement::Text(text)]);

        let serialized = seq.to_string();
        let reparsed = KeySequence::parse(&serialized).unwrap();

        prop_assert_eq!(seq, reparsed, "Text should round-trip");
    }

    #[test]
    fn key_sequence_round_trip_mixed(
        key in arb_special_key(),
        duration in arb_wait_duration(),
        var_name in arb_var_name(),
        text in arb_safe_text()
    ) {
        let mut elements = vec![
            KeyElement::SpecialKey(key),
            KeyElement::Wait(duration),
            KeyElement::Variable(var_name),
        ];

        if !text.is_empty() {
            elements.push(KeyElement::Text(text));
        }

        let seq = KeySequence::from_elements(elements);

        let serialized = seq.to_string();
        let reparsed = KeySequence::parse(&serialized).unwrap();

        prop_assert_eq!(seq, reparsed, "Mixed sequence should round-trip");
    }

    #[test]
    fn key_sequence_double_round_trip(
        elements in prop::collection::vec(arb_key_element(), 1..5)
    ) {
        // Filter out empty text elements
        let filtered: Vec<KeyElement> = elements
            .into_iter()
            .filter(|e| {
                if let KeyElement::Text(t) = e {
                    !t.is_empty()
                } else {
                    true
                }
            })
            .collect();

        if filtered.is_empty() {
            return Ok(());
        }

        let seq = KeySequence::from_elements(filtered);

        // First round-trip
        let s1 = seq.to_string();
        let p1 = KeySequence::parse(&s1).unwrap();

        // Second round-trip
        let s2 = p1.to_string();
        let p2 = KeySequence::parse(&s2).unwrap();

        // Both should be equal
        prop_assert_eq!(p1, p2, "Double round-trip should be stable");
        prop_assert_eq!(s1, s2, "Serialized strings should be identical");
    }
}

// Import Variable and VariableManager for substitution tests
use rustconn_core::{Variable, VariableManager, VariableScope};

/// Strategy for generating variable values (no nested references)
fn arb_var_value() -> impl Strategy<Value = String> {
    // Generate values that don't contain ${...} patterns
    "[a-zA-Z0-9 .,;:!?@#%^&*()\\[\\]<>/-]{0,50}".prop_map(|s| {
        s.replace("${", "")
            .replace("}", "")
            .replace('{', "")
            .replace('$', "")
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 7: Key Sequence Variable Substitution ==========
    // **Feature: rustconn-enhancements, Property 7: Key Sequence Variable Substitution**
    // **Validates: Requirements 2.4**
    //
    // For any key sequence containing variable references, substitution should
    // replace all valid references with their values.

    #[test]
    fn key_sequence_variable_substitution_replaces_all_refs(
        var_names in prop::collection::vec(arb_var_name(), 1..5),
        values in prop::collection::vec(arb_var_value(), 1..10)
    ) {
        let mut manager = VariableManager::new();

        // Deduplicate variable names to avoid overwriting - use a map to track final values
        let mut var_map = std::collections::HashMap::new();
        for (i, name) in var_names.iter().enumerate() {
            let value = &values[i % values.len()];
            var_map.insert(name.clone(), value.clone());
        }

        // Set up variables with deduplicated names
        for (name, value) in &var_map {
            manager.set_global(Variable::new(name.clone(), value.clone()));
        }

        // Create a key sequence with unique variables only
        let unique_names: Vec<_> = var_map.keys().cloned().collect();
        let elements: Vec<KeyElement> = unique_names
            .iter()
            .map(|name| KeyElement::Variable(name.clone()))
            .collect();

        let seq = KeySequence::from_elements(elements);

        // Substitute variables
        let result = seq.substitute_variables(&manager, VariableScope::Global);
        prop_assert!(result.is_ok(), "Substitution should succeed");

        let substituted = result.unwrap();

        // No Variable elements should remain
        for element in &substituted.elements {
            prop_assert!(
                !matches!(element, KeyElement::Variable(_)),
                "No Variable elements should remain after substitution"
            );
        }

        // All final values should be present as Text elements (for non-empty values)
        for (name, expected_value) in &var_map {
            if !expected_value.is_empty() {
                // Find the corresponding text element
                let found = substituted.elements.iter().any(|e| {
                    if let KeyElement::Text(t) = e {
                        t.contains(expected_value)
                    } else {
                        false
                    }
                });
                prop_assert!(
                    found,
                    "Value '{}' for variable '{}' should be in substituted sequence",
                    expected_value, name
                );
            }
        }
    }

    #[test]
    fn key_sequence_substitution_preserves_non_variable_elements(
        key in arb_special_key(),
        duration in arb_wait_duration(),
        var_name in arb_var_name(),
        var_value in arb_var_value()
    ) {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(var_name.clone(), var_value.clone()));

        let seq = KeySequence::from_elements(vec![
            KeyElement::SpecialKey(key),
            KeyElement::Variable(var_name),
            KeyElement::Wait(duration),
        ]);

        let result = seq.substitute_variables(&manager, VariableScope::Global);
        prop_assert!(result.is_ok());

        let substituted = result.unwrap();

        // Special key should be preserved
        let has_special_key = substituted.elements.iter().any(|e| {
            matches!(e, KeyElement::SpecialKey(k) if *k == key)
        });
        prop_assert!(has_special_key, "Special key should be preserved");

        // Wait should be preserved
        let has_wait = substituted.elements.iter().any(|e| {
            matches!(e, KeyElement::Wait(d) if *d == duration)
        });
        prop_assert!(has_wait, "Wait should be preserved");
    }

    #[test]
    fn key_sequence_substitution_with_scope_override(
        var_name in arb_var_name(),
        global_value in arb_var_value(),
        local_value in arb_var_value()
    ) {
        prop_assume!(global_value != local_value);
        prop_assume!(!local_value.is_empty());

        let mut manager = VariableManager::new();
        let conn_id = uuid::Uuid::new_v4();

        // Set global and connection-local values
        manager.set_global(Variable::new(var_name.clone(), global_value.clone()));
        manager.set_connection(conn_id, Variable::new(var_name.clone(), local_value.clone()));

        let seq = KeySequence::from_elements(vec![KeyElement::Variable(var_name)]);

        // Substitute with connection scope - should use local value
        let result = seq.substitute_variables(&manager, VariableScope::Connection(conn_id));
        prop_assert!(result.is_ok());

        let substituted = result.unwrap();

        // Should contain local value, not global
        let has_local = substituted.elements.iter().any(|e| {
            if let KeyElement::Text(t) = e {
                t.contains(&local_value)
            } else {
                false
            }
        });
        prop_assert!(has_local, "Should use connection-scoped value");
    }

    #[test]
    fn key_sequence_substitution_undefined_variable_fails(
        var_name in arb_var_name()
    ) {
        let manager = VariableManager::new();

        let seq = KeySequence::from_elements(vec![KeyElement::Variable(var_name)]);

        let result = seq.substitute_variables(&manager, VariableScope::Global);
        prop_assert!(result.is_err(), "Undefined variable should cause error");

        if let Err(e) = result {
            prop_assert!(
                matches!(e, KeySequenceError::VariableError(_)),
                "Error should be VariableError"
            );
        }
    }

    #[test]
    fn key_sequence_substitution_empty_value_removes_element(
        var_name in arb_var_name()
    ) {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(var_name.clone(), ""));

        let seq = KeySequence::from_elements(vec![
            KeyElement::SpecialKey(SpecialKey::Enter),
            KeyElement::Variable(var_name),
            KeyElement::SpecialKey(SpecialKey::Tab),
        ]);

        let result = seq.substitute_variables(&manager, VariableScope::Global);
        prop_assert!(result.is_ok());

        let substituted = result.unwrap();

        // Should have 2 elements (Enter and Tab), empty text is removed
        prop_assert_eq!(
            substituted.elements.len(), 2,
            "Empty variable value should not create element"
        );
    }

    #[test]
    fn key_sequence_substitution_multiple_same_variable(
        var_name in arb_var_name(),
        var_value in arb_var_value()
    ) {
        prop_assume!(!var_value.is_empty());

        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(var_name.clone(), var_value.clone()));

        // Use same variable multiple times
        let seq = KeySequence::from_elements(vec![
            KeyElement::Variable(var_name.clone()),
            KeyElement::SpecialKey(SpecialKey::Tab),
            KeyElement::Variable(var_name),
        ]);

        let result = seq.substitute_variables(&manager, VariableScope::Global);
        prop_assert!(result.is_ok());

        let substituted = result.unwrap();

        // Count occurrences of the value
        let count = substituted.elements.iter().filter(|e| {
            if let KeyElement::Text(t) = e {
                t == &var_value
            } else {
                false
            }
        }).count();

        prop_assert_eq!(count, 2, "Variable should be substituted twice");
    }
}

// ========== Unit Tests for Variable Substitution Edge Cases ==========

#[cfg(test)]
mod variable_substitution_tests {
    use super::*;

    #[test]
    fn test_substitution_with_nested_variables_in_text() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("user", "admin"));
        manager.set_global(Variable::new("host", "server.com"));

        // Text element containing ${var} patterns
        let seq = KeySequence::parse("ssh ${user}@${host}").unwrap();
        let result = seq.substitute_variables(&manager, VariableScope::Global);

        assert!(result.is_ok());
        let substituted = result.unwrap();

        // Should have substituted the variables in the text
        let text = substituted.to_string();
        assert!(text.contains("admin"));
        assert!(text.contains("server.com"));
    }

    #[test]
    fn test_substitution_preserves_order() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("a", "first"));
        manager.set_global(Variable::new("b", "second"));

        let seq = KeySequence::from_elements(vec![
            KeyElement::Variable("a".to_string()),
            KeyElement::SpecialKey(SpecialKey::Enter),
            KeyElement::Variable("b".to_string()),
        ]);

        let result = seq
            .substitute_variables(&manager, VariableScope::Global)
            .unwrap();

        assert_eq!(result.elements.len(), 3);
        assert_eq!(result.elements[0], KeyElement::Text("first".to_string()));
        assert_eq!(
            result.elements[1],
            KeyElement::SpecialKey(SpecialKey::Enter)
        );
        assert_eq!(result.elements[2], KeyElement::Text("second".to_string()));
    }

    #[test]
    fn test_substitution_with_special_chars_in_value() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("pass", "p@ss{w0rd}!"));

        let seq = KeySequence::from_elements(vec![KeyElement::Variable("pass".to_string())]);
        let result = seq
            .substitute_variables(&manager, VariableScope::Global)
            .unwrap();

        assert_eq!(result.elements.len(), 1);
        assert_eq!(
            result.elements[0],
            KeyElement::Text("p@ss{w0rd}!".to_string())
        );
    }
}
