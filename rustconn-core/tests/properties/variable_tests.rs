//! Property-based tests for the Variables system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Variables system (Requirements 6.x).

use proptest::prelude::*;
use rustconn_core::{Variable, VariableManager, VariableScope};

// ========== Strategies ==========

/// Strategy for generating valid variable names
fn arb_var_name() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_]{0,15}".prop_map(|s| s)
}

/// Strategy for generating variable values (no nested references)
fn arb_var_value() -> impl Strategy<Value = String> {
    // Generate values that don't contain ${...} patterns
    "[a-zA-Z0-9 .,;:!?@#%^&*()\\[\\]<>/-]{0,50}".prop_map(|s| s.replace("${", "").replace("}", ""))
}

/// Strategy for generating a list of unique variable names
fn arb_var_names() -> impl Strategy<Value = Vec<String>> {
    prop::collection::hash_set(arb_var_name(), 0..10).prop_map(|set| set.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 1: Variable Substitution Consistency ==========
    // **Feature: rustconn-enhancements, Property 1: Variable Substitution Consistency**
    // **Validates: Requirements 6.3, 6.6**
    //
    // For any string containing variable references and any variable scope,
    // substituting variables and then checking for remaining unresolved references
    // should yield either a fully resolved string or a list of undefined variables.

    #[test]
    fn variable_substitution_consistency(
        var_names in arb_var_names(),
        values in prop::collection::vec(arb_var_value(), 0..10)
    ) {
        let mut manager = VariableManager::new();

        // Set up some variables (not all var_names may have values)
        for (i, name) in var_names.iter().enumerate() {
            if i < values.len() {
                manager.set_global(Variable::new(name.clone(), values[i].clone()));
            }
        }

        // Create a string with all variable references
        let input = var_names.iter()
            .map(|n| format!("${{{}}}", n))
            .collect::<Vec<_>>()
            .join(" ");

        // Substitute should succeed (undefined vars become empty)
        let result = manager.substitute(&input, VariableScope::Global);
        prop_assert!(result.is_ok(), "Substitution should not fail: {:?}", result);

        let substituted = result.unwrap();

        // Check that defined variables are substituted
        for (i, name) in var_names.iter().enumerate() {
            if i < values.len() {
                // Defined variable should have its value in result
                prop_assert!(
                    substituted.contains(&values[i]),
                    "Defined variable {} should be substituted with value {}",
                    name, values[i]
                );
            }
        }

        // Parse remaining references - should be empty (undefined vars become empty string)
        let remaining = VariableManager::parse_references(&substituted).unwrap();
        prop_assert!(
            remaining.is_empty(),
            "After substitution, no variable references should remain: {:?}",
            remaining
        );
    }

    #[test]
    fn substitution_with_all_defined_vars_leaves_no_references(
        var_names in arb_var_names(),
        values in prop::collection::vec(arb_var_value(), 1..20)
    ) {
        // Skip if no variables
        if var_names.is_empty() {
            return Ok(());
        }

        let mut manager = VariableManager::new();

        // Define all variables
        for (i, name) in var_names.iter().enumerate() {
            let value = &values[i % values.len()];
            manager.set_global(Variable::new(name.clone(), value.clone()));
        }

        // Create input with all variables
        let input = var_names.iter()
            .map(|n| format!("${{{}}}", n))
            .collect::<Vec<_>>()
            .join(" ");

        let result = manager.substitute(&input, VariableScope::Global).unwrap();

        // No ${...} patterns should remain
        prop_assert!(
            !result.contains("${"),
            "No variable references should remain after full substitution: {}",
            result
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 2: Variable Resolution with Override ==========
    // **Feature: rustconn-enhancements, Property 2: Variable Resolution with Override**
    // **Validates: Requirements 6.2**
    //
    // For any variable name that exists in both global and local scope,
    // resolution should return the local value when using connection scope.

    #[test]
    fn variable_override_local_takes_precedence(
        var_name in arb_var_name(),
        global_value in arb_var_value(),
        local_value in arb_var_value()
    ) {
        // Ensure values are different to make the test meaningful
        prop_assume!(global_value != local_value);

        let mut manager = VariableManager::new();
        let conn_id = uuid::Uuid::new_v4();

        // Set global variable
        manager.set_global(Variable::new(var_name.clone(), global_value.clone()));

        // Set connection-local variable with same name
        manager.set_connection(conn_id, Variable::new(var_name.clone(), local_value.clone()));

        // Resolution at connection scope should return local value
        let result = manager.resolve(&var_name, VariableScope::Connection(conn_id)).unwrap();
        prop_assert_eq!(
            result, local_value,
            "Connection-scoped variable should override global"
        );

        // Resolution at global scope should still return global value
        let global_result = manager.resolve(&var_name, VariableScope::Global).unwrap();
        prop_assert_eq!(
            global_result, global_value,
            "Global scope should return global value"
        );
    }

    #[test]
    fn variable_override_document_takes_precedence_over_global(
        var_name in arb_var_name(),
        global_value in arb_var_value(),
        doc_value in arb_var_value()
    ) {
        prop_assume!(global_value != doc_value);

        let mut manager = VariableManager::new();
        let doc_id = uuid::Uuid::new_v4();

        // Set global variable
        manager.set_global(Variable::new(var_name.clone(), global_value.clone()));

        // Set document variable with same name
        manager.set_document(doc_id, Variable::new(var_name.clone(), doc_value.clone()));

        // Resolution at document scope should return document value
        let result = manager.resolve(&var_name, VariableScope::Document(doc_id)).unwrap();
        prop_assert_eq!(
            result, doc_value,
            "Document-scoped variable should override global"
        );
    }

    #[test]
    fn variable_override_connection_takes_precedence_over_document(
        var_name in arb_var_name(),
        doc_value in arb_var_value(),
        conn_value in arb_var_value()
    ) {
        prop_assume!(doc_value != conn_value);

        let mut manager = VariableManager::new();
        let doc_id = uuid::Uuid::new_v4();
        let conn_id = uuid::Uuid::new_v4();

        // Set document variable
        manager.set_document(doc_id, Variable::new(var_name.clone(), doc_value.clone()));

        // Set connection variable with same name
        manager.set_connection(conn_id, Variable::new(var_name.clone(), conn_value.clone()));

        // Associate connection with document
        manager.set_connection_document(conn_id, doc_id);

        // Resolution at connection scope should return connection value
        let result = manager.resolve(&var_name, VariableScope::Connection(conn_id)).unwrap();
        prop_assert_eq!(
            result, conn_value,
            "Connection-scoped variable should override document"
        );
    }

    #[test]
    fn variable_fallback_to_parent_scope(
        var_name in arb_var_name(),
        global_value in arb_var_value()
    ) {
        let mut manager = VariableManager::new();
        let doc_id = uuid::Uuid::new_v4();
        let conn_id = uuid::Uuid::new_v4();

        // Only set global variable
        manager.set_global(Variable::new(var_name.clone(), global_value.clone()));

        // Associate connection with document
        manager.set_connection_document(conn_id, doc_id);

        // Resolution at connection scope should fall back to global
        let result = manager.resolve(&var_name, VariableScope::Connection(conn_id)).unwrap();
        prop_assert_eq!(
            result, global_value.clone(),
            "Should fall back to global when not defined in connection or document"
        );

        // Resolution at document scope should also fall back to global
        let doc_result = manager.resolve(&var_name, VariableScope::Document(doc_id)).unwrap();
        prop_assert_eq!(
            doc_result, global_value,
            "Should fall back to global when not defined in document"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 3: Nested Variable Resolution Depth ==========
    // **Feature: rustconn-enhancements, Property 3: Nested Variable Resolution Depth**
    // **Validates: Requirements 6.4, 6.7**
    //
    // For any chain of variable references, resolution should terminate within
    // the configured maximum depth and detect cycles.

    #[test]
    fn nested_resolution_terminates_within_depth(
        depth in 1usize..8,
        final_value in arb_var_value()
    ) {
        let mut manager = VariableManager::new();

        // Create a chain of variables: var0 -> ${var1} -> ... -> ${varN} -> final_value
        for i in 0..depth {
            let name = format!("var{}", i);
            let value = format!("${{var{}}}", i + 1);
            manager.set_global(Variable::new(name, value));
        }
        manager.set_global(Variable::new(format!("var{}", depth), final_value.clone()));

        // Resolution should succeed and return the final value
        let result = manager.resolve("var0", VariableScope::Global);
        prop_assert!(result.is_ok(), "Resolution should succeed for depth {}: {:?}", depth, result);
        prop_assert_eq!(result.unwrap(), final_value);
    }

    #[test]
    fn nested_resolution_detects_direct_cycle(
        var_name in arb_var_name()
    ) {
        let mut manager = VariableManager::new();

        // Self-reference: var -> ${var}
        manager.set_global(Variable::new(var_name.clone(), format!("${{{}}}", var_name)));

        let result = manager.resolve(&var_name, VariableScope::Global);
        prop_assert!(
            matches!(result, Err(rustconn_core::VariableError::CircularReference(_))),
            "Self-reference should be detected as circular: {:?}",
            result
        );
    }

    #[test]
    fn nested_resolution_detects_indirect_cycle(
        var1 in arb_var_name(),
        var2 in arb_var_name()
    ) {
        // Ensure different names
        prop_assume!(var1 != var2);

        let mut manager = VariableManager::new();

        // Create cycle: var1 -> ${var2} -> ${var1}
        manager.set_global(Variable::new(var1.clone(), format!("${{{}}}", var2)));
        manager.set_global(Variable::new(var2.clone(), format!("${{{}}}", var1)));

        let result = manager.resolve(&var1, VariableScope::Global);
        prop_assert!(
            matches!(result, Err(rustconn_core::VariableError::CircularReference(_))),
            "Indirect cycle should be detected: {:?}",
            result
        );
    }

    #[test]
    fn detect_cycles_finds_all_cycles(
        var1 in arb_var_name(),
        var2 in arb_var_name()
    ) {
        prop_assume!(var1 != var2);

        let mut manager = VariableManager::new();

        // Create cycle
        manager.set_global(Variable::new(var1.clone(), format!("${{{}}}", var2)));
        manager.set_global(Variable::new(var2.clone(), format!("${{{}}}", var1)));

        let result = manager.detect_cycles();
        prop_assert!(
            result.is_err(),
            "detect_cycles should find the cycle"
        );
    }

    #[test]
    fn no_false_positive_cycles(
        var_names in arb_var_names(),
        values in prop::collection::vec(arb_var_value(), 1..10)
    ) {
        // Skip if no variables
        if var_names.is_empty() {
            return Ok(());
        }

        let mut manager = VariableManager::new();

        // Create a linear chain (no cycles)
        for (i, name) in var_names.iter().enumerate() {
            let value = if i + 1 < var_names.len() {
                // Reference next variable
                format!("prefix_{} ${{{}}}", values[i % values.len()], var_names[i + 1])
            } else {
                // Last variable has a plain value
                values[i % values.len()].clone()
            };
            manager.set_global(Variable::new(name.clone(), value));
        }

        // detect_cycles should not find any cycles
        let result = manager.detect_cycles();
        prop_assert!(
            result.is_ok(),
            "Linear chain should not be detected as cycle: {:?}",
            result
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 4: Variable Serialization Round-Trip ==========
    // **Feature: rustconn-enhancements, Property 4: Variable Serialization Round-Trip**
    // **Validates: Requirements 6.8**
    //
    // For any valid variable definition, serializing to JSON and deserializing
    // should produce an equivalent variable.

    #[test]
    fn variable_json_round_trip(
        name in arb_var_name(),
        value in arb_var_value(),
        is_secret in any::<bool>(),
        description in prop::option::of(arb_var_value())
    ) {
        let mut var = Variable::new(name, value).with_secret(is_secret);
        if let Some(desc) = description {
            var = var.with_description(desc);
        }

        // Serialize to JSON
        let json = serde_json::to_string(&var).expect("Serialization should succeed");

        // Deserialize back
        let parsed: Variable = serde_json::from_str(&json).expect("Deserialization should succeed");

        // Should be equal
        prop_assert_eq!(var, parsed, "Round-trip should preserve variable");
    }

    #[test]
    fn variable_yaml_round_trip(
        name in arb_var_name(),
        value in arb_var_value(),
        is_secret in any::<bool>(),
        description in prop::option::of(arb_var_value())
    ) {
        let mut var = Variable::new(name, value).with_secret(is_secret);
        if let Some(desc) = description {
            var = var.with_description(desc);
        }

        // Serialize to YAML
        let yaml = serde_yaml::to_string(&var).expect("YAML serialization should succeed");

        // Deserialize back
        let parsed: Variable = serde_yaml::from_str(&yaml).expect("YAML deserialization should succeed");

        // Should be equal
        prop_assert_eq!(var, parsed, "YAML round-trip should preserve variable");
    }

    #[test]
    fn variable_preserves_secret_flag(
        name in arb_var_name(),
        value in arb_var_value(),
        is_secret in any::<bool>()
    ) {
        let var = Variable::new(name, value).with_secret(is_secret);

        let json = serde_json::to_string(&var).unwrap();
        let parsed: Variable = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(
            var.is_secret(), parsed.is_secret(),
            "Secret flag should be preserved through serialization"
        );
    }

    #[test]
    fn variable_preserves_description(
        name in arb_var_name(),
        value in arb_var_value(),
        description in prop::option::of(arb_var_value())
    ) {
        let mut var = Variable::new(name, value);
        if let Some(desc) = description.clone() {
            var = var.with_description(desc);
        }

        let json = serde_json::to_string(&var).unwrap();
        let parsed: Variable = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(
            var.description.clone(), parsed.description.clone(),
            "Description should be preserved through serialization"
        );
    }
}

// ========== Unit Tests for Edge Cases ==========
// **Validates: Requirements 6.6**

#[cfg(test)]
mod edge_case_tests {
    use rustconn_core::{Variable, VariableError, VariableManager, VariableScope};

    #[test]
    fn test_undefined_variable_returns_error() {
        let manager = VariableManager::new();

        let result = manager.resolve("nonexistent", VariableScope::Global);
        assert!(matches!(result, Err(VariableError::Undefined(name)) if name == "nonexistent"));
    }

    #[test]
    fn test_undefined_variable_in_substitution_becomes_empty() {
        let manager = VariableManager::new();

        let result = manager.substitute("prefix_${undefined}_suffix", VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "prefix__suffix");
    }

    #[test]
    fn test_empty_input_string() {
        let manager = VariableManager::new();

        let result = manager.substitute("", VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_special_characters_in_values() {
        let mut manager = VariableManager::new();

        // Test various special characters
        let special_values = vec![
            ("var1", "value with spaces"),
            ("var2", "value\twith\ttabs"),
            ("var3", "value\nwith\nnewlines"),
            ("var4", "value!@#$%^&*()"),
            ("var5", "value<>[]{}"),
            ("var6", "value'\"quotes"),
            ("var7", "value\\backslash"),
            ("var8", "日本語"), // Unicode
            ("var9", "émojis 🎉"),
        ];

        for (name, value) in &special_values {
            manager.set_global(Variable::new(*name, *value));
        }

        for (name, expected_value) in &special_values {
            let result = manager.resolve(name, VariableScope::Global);
            assert!(result.is_ok(), "Failed to resolve {}", name);
            assert_eq!(
                result.unwrap(),
                *expected_value,
                "Value mismatch for {}",
                name
            );
        }
    }

    #[test]
    fn test_variable_with_dollar_sign_in_value() {
        let mut manager = VariableManager::new();

        // Value contains $ but not a valid variable reference
        manager.set_global(Variable::new("price", "$100"));

        let result = manager.resolve("price", VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "$100");
    }

    #[test]
    fn test_partial_variable_syntax_not_substituted() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("var", "value"));

        // These should NOT be treated as variable references
        let test_cases = vec![
            ("$var", "$var"),       // Missing braces
            ("${}", "${}"),         // Empty name (invalid syntax, left as-is)
            ("${123}", "${123}"),   // Starts with number (invalid)
            ("${ var}", "${ var}"), // Space in name (invalid)
        ];

        for (input, expected) in test_cases {
            let result = manager.substitute(input, VariableScope::Global);
            assert!(result.is_ok(), "Failed for input: {}", input);
            assert_eq!(result.unwrap(), expected, "Mismatch for input: {}", input);
        }
    }

    #[test]
    fn test_multiple_undefined_variables() {
        let manager = VariableManager::new();

        let result = manager.substitute("${a} ${b} ${c}", VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "  "); // All become empty strings
    }

    #[test]
    fn test_mixed_defined_and_undefined_variables() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("defined", "value"));

        let result = manager.substitute("${defined} ${undefined}", VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "value ");
    }

    #[test]
    fn test_variable_name_case_sensitivity() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("MyVar", "value1"));
        manager.set_global(Variable::new("myvar", "value2"));
        manager.set_global(Variable::new("MYVAR", "value3"));

        // All three should be distinct
        assert_eq!(
            manager.resolve("MyVar", VariableScope::Global).unwrap(),
            "value1"
        );
        assert_eq!(
            manager.resolve("myvar", VariableScope::Global).unwrap(),
            "value2"
        );
        assert_eq!(
            manager.resolve("MYVAR", VariableScope::Global).unwrap(),
            "value3"
        );
    }

    #[test]
    fn test_variable_with_underscore_prefix() {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new("_private", "secret"));
        manager.set_global(Variable::new("__dunder", "double"));

        assert_eq!(
            manager.resolve("_private", VariableScope::Global).unwrap(),
            "secret"
        );
        assert_eq!(
            manager.resolve("__dunder", VariableScope::Global).unwrap(),
            "double"
        );
    }

    #[test]
    fn test_very_long_variable_name() {
        let mut manager = VariableManager::new();
        let long_name = "a".repeat(1000);
        manager.set_global(Variable::new(long_name.clone(), "value"));

        let result = manager.resolve(&long_name, VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "value");
    }

    #[test]
    fn test_very_long_variable_value() {
        let mut manager = VariableManager::new();
        let long_value = "x".repeat(10000);
        manager.set_global(Variable::new("var", long_value.clone()));

        let result = manager.resolve("var", VariableScope::Global);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), long_value);
    }

    #[test]
    fn test_empty_scope_returns_undefined() {
        let manager = VariableManager::new();
        let doc_id = uuid::Uuid::new_v4();
        let conn_id = uuid::Uuid::new_v4();

        // All scopes should return undefined for non-existent variable
        assert!(manager.resolve("var", VariableScope::Global).is_err());
        assert!(
            manager
                .resolve("var", VariableScope::Document(doc_id))
                .is_err()
        );
        assert!(
            manager
                .resolve("var", VariableScope::Connection(conn_id))
                .is_err()
        );
    }
}
