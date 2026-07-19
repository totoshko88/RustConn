//! Property-based tests for snippet variable extraction
//!
//! **Feature: rustconn, Property 12: Snippet Variable Extraction**
//! **Validates: Requirements 8.3**

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;
use rustconn_core::SnippetManager;

// Strategy for generating valid variable names (alphanumeric with underscores, starting with letter or underscore)
fn arb_var_name() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_]{0,15}".prop_map(|s| s)
}

// Strategy for generating a list of unique variable names
fn arb_var_names() -> impl Strategy<Value = Vec<String>> {
    prop::collection::hash_set(arb_var_name(), 0..10).prop_map(|set| set.into_iter().collect())
}

// Strategy for generating non-variable text (text that doesn't contain ${...} patterns)
fn arb_plain_text() -> impl Strategy<Value = String> {
    // Generate text that won't accidentally form variable patterns
    "[a-zA-Z0-9 .,;:!?@#%^&*()\\[\\]<>/-]{0,50}".prop_map(|s| {
        // Ensure no accidental ${...} patterns
        s.replace("${", "").replace("}", "")
    })
}

// Strategy for generating a command template with known variables
fn arb_command_with_vars(vars: Vec<String>) -> impl Strategy<Value = (String, Vec<String>)> {
    arb_plain_text().prop_map(move |prefix| {
        let mut command = prefix;
        for var in &vars {
            command.push_str(&format!(" ${{{}}}", var));
        }
        (command, vars.clone())
    })
}

// Strategy for generating a command template with random variables
fn arb_command_template() -> impl Strategy<Value = (String, Vec<String>)> {
    arb_var_names().prop_flat_map(|vars| arb_command_with_vars(vars))
}

// Strategy for generating variable values
fn arb_var_value() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_.-]{1,30}".prop_map(|s| s)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 12: Snippet Variable Extraction**
    /// **Validates: Requirements 8.3**
    ///
    /// For any snippet command template containing variable placeholders (e.g., `${var_name}`),
    /// extracting variables should return all unique variable names present in the template.
    #[test]
    fn extract_variables_finds_all_variables((command, expected_vars) in arb_command_template()) {
        let extracted = SnippetManager::extract_variables(&command);

        // Convert to sets for comparison (order doesn't matter)
        let expected_set: HashSet<String> = expected_vars.into_iter().collect();
        let extracted_set: HashSet<String> = extracted.into_iter().collect();

        prop_assert_eq!(
            expected_set,
            extracted_set,
            "Extracted variables should match expected variables"
        );
    }

    /// Property: Extracting variables from a command with no variables returns empty
    #[test]
    fn extract_variables_empty_for_plain_text(text in arb_plain_text()) {
        let extracted = SnippetManager::extract_variables(&text);
        prop_assert!(
            extracted.is_empty(),
            "Plain text without variables should extract to empty: {:?}",
            extracted
        );
    }

    /// Property: Variable extraction is idempotent
    #[test]
    fn extract_variables_idempotent((command, _) in arb_command_template()) {
        let first_extraction = SnippetManager::extract_variables(&command);
        let second_extraction = SnippetManager::extract_variables(&command);

        prop_assert_eq!(
            first_extraction,
            second_extraction,
            "Variable extraction should be idempotent"
        );
    }

    /// Property: Duplicate variables in command are deduplicated
    #[test]
    fn extract_variables_deduplicates(var_name in arb_var_name()) {
        // Create a command with the same variable multiple times
        let command = format!("echo ${{{0}}} && echo ${{{0}}} && echo ${{{0}}}", var_name);
        let extracted = SnippetManager::extract_variables(&command);

        prop_assert_eq!(
            extracted.len(),
            1,
            "Duplicate variables should be deduplicated"
        );
        prop_assert_eq!(
            &extracted[0],
            &var_name,
            "The deduplicated variable should match"
        );
    }

    /// Property: Substitution replaces all occurrences of variables
    #[test]
    fn substitute_replaces_all_occurrences(
        var_name in arb_var_name(),
        value in arb_var_value()
    ) {
        // Create a command with the variable multiple times, using unique markers
        // to avoid false positives from value appearing in command text
        let marker1 = "MARKER_ONE_";
        let marker2 = "MARKER_TWO_";
        let command = format!("{marker1}${{{0}}} {marker2}${{{0}}}", var_name);
        let mut values = HashMap::new();
        values.insert(var_name.clone(), value.clone());

        let result = SnippetManager::substitute_variables(&command, &values);

        // The result should not contain the variable placeholder
        prop_assert!(
            !result.contains(&format!("${{{}}}", var_name)),
            "All variable occurrences should be replaced"
        );

        // Both markers should be followed by the value
        let expected = format!("{marker1}{value} {marker2}{value}");
        prop_assert_eq!(
            result,
            expected,
            "Both variable occurrences should be replaced with the value"
        );
    }

    /// Property: Substitution with all values produces no remaining variables
    #[test]
    fn substitute_all_leaves_no_variables((command, vars) in arb_command_template()) {
        // Skip if no variables
        if vars.is_empty() {
            return Ok(());
        }

        // Generate values for all variables
        let values: HashMap<String, String> = vars
            .iter()
            .enumerate()
            .map(|(i, v)| (v.clone(), format!("value{}", i)))
            .collect();

        let result = SnippetManager::substitute_variables(&command, &values);

        // Check that no ${...} patterns remain
        prop_assert!(
            !result.contains("${"),
            "No variable placeholders should remain after full substitution: {}",
            result
        );
    }

    /// Property: Substitution preserves unmatched variables
    #[test]
    fn substitute_preserves_unmatched(
        var1 in arb_var_name(),
        var2 in arb_var_name(),
        value in arb_var_value()
    ) {
        // Ensure var1 and var2 are different
        prop_assume!(var1 != var2);

        let command = format!("${{{0}}} ${{{1}}}", var1, var2);
        let mut values = HashMap::new();
        values.insert(var1.clone(), value.clone());
        // var2 is not provided

        let result = SnippetManager::substitute_variables(&command, &values);

        // var1 should be replaced
        prop_assert!(
            !result.contains(&format!("${{{}}}", var1)),
            "Provided variable should be replaced"
        );

        // var2 should remain
        prop_assert!(
            result.contains(&format!("${{{}}}", var2)),
            "Unprovided variable should remain: {}",
            result
        );
    }

    /// Property: Round-trip - extract then substitute returns original structure
    #[test]
    fn extract_substitute_round_trip((command, vars) in arb_command_template()) {
        // Skip if no variables
        if vars.is_empty() {
            return Ok(());
        }

        // Extract variables
        let extracted = SnippetManager::extract_variables(&command);

        // Create values that are the variable names wrapped in markers
        let values: HashMap<String, String> = extracted
            .iter()
            .map(|v| (v.clone(), format!("[{}]", v)))
            .collect();

        let result = SnippetManager::substitute_variables(&command, &values);

        // Each variable should now appear as [var_name] instead of ${var_name}
        for var in &extracted {
            prop_assert!(
                result.contains(&format!("[{}]", var)),
                "Substituted value should appear in result for variable: {}",
                var
            );
            prop_assert!(
                !result.contains(&format!("${{{}}}", var)),
                "Original placeholder should not appear for variable: {}",
                var
            );
        }
    }

    // ========== Property 5: Snippet Variable Extraction (rustconn-enhancements) ==========

    /// **Feature: rustconn-enhancements, Property 5: Snippet Variable Extraction**
    /// **Validates: Requirements 7.2, 7.3**
    ///
    /// For any snippet command containing ${variable} placeholders, the extracted
    /// variable list should contain exactly the unique variable names present in
    /// the command, with no duplicates.
    #[test]
    fn snippet_variable_extraction_uniqueness((command, expected_vars) in arb_command_template()) {
        let extracted = SnippetManager::extract_variables(&command);

        // Property: No duplicates in extracted variables
        let extracted_set: HashSet<String> = extracted.iter().cloned().collect();
        prop_assert_eq!(
            extracted.len(),
            extracted_set.len(),
            "Extracted variables should have no duplicates"
        );

        // Property: Extracted variables match expected (unique) variables
        let expected_set: HashSet<String> = expected_vars.into_iter().collect();
        prop_assert_eq!(
            extracted_set,
            expected_set,
            "Extracted variables should exactly match the unique variables in the command"
        );
    }

    /// **Feature: rustconn-enhancements, Property 5: Snippet Variable Extraction**
    /// **Validates: Requirements 7.2, 7.3**
    ///
    /// For any snippet with variables, extracting variable objects should produce
    /// SnippetVariable instances with the correct names.
    #[test]
    fn snippet_variable_objects_extraction((command, expected_vars) in arb_command_template()) {
        let var_objects = SnippetManager::extract_variable_objects(&command);

        // Property: Number of variable objects matches unique variable count
        let expected_set: HashSet<String> = expected_vars.into_iter().collect();
        prop_assert_eq!(
            var_objects.len(),
            expected_set.len(),
            "Number of extracted variable objects should match unique variable count"
        );

        // Property: All variable names are present
        let extracted_names: HashSet<String> = var_objects.iter().map(|v| v.name.clone()).collect();
        prop_assert_eq!(
            extracted_names,
            expected_set,
            "Variable object names should match expected variables"
        );

        // Property: All variable objects have None for description and default_value initially
        for var in &var_objects {
            prop_assert!(
                var.description.is_none(),
                "Newly extracted variable should have no description"
            );
            prop_assert!(
                var.default_value.is_none(),
                "Newly extracted variable should have no default value"
            );
        }
    }
}
