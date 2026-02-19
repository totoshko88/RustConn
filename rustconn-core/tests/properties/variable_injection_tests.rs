//! Property-based tests for variable injection prevention (SEC-07)
//!
//! Validates that `substitute_for_command()` and `validate_command_value()`
//! correctly reject values containing control characters, null bytes,
//! and newlines while allowing safe values through.

use proptest::prelude::*;
use rustconn_core::{Variable, VariableManager, VariableScope};

/// Strategy for generating safe variable values (printable ASCII + tab, no shell metacharacters)
fn arb_safe_value() -> impl Strategy<Value = String> {
    // Exclude shell metacharacters: ; | & ` $ ( ) < > !
    prop::collection::vec(
        prop_oneof![
            prop::char::range(' ', ' '), // space
            prop::char::range('#', '#'),
            prop::char::range('%', '%'),
            prop::char::range('+', '+'),
            prop::char::range(',', '.'), // , - .
            prop::char::range('0', '9'),
            prop::char::range(':', ':'),
            prop::char::range('=', '='),
            prop::char::range('@', 'Z'), // @ A-Z
            prop::char::range('^', '_'), // ^ _
            prop::char::range('a', 'z'),
            prop::char::range('{', '{'),
            prop::char::range('}', '}'),
            prop::char::range('~', '~'),
            prop::char::range('/', '/'),
            Just('\t'),
        ],
        0..100,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>())
}

/// Strategy for generating values with at least one control character
fn arb_unsafe_control_value() -> impl Strategy<Value = String> {
    let control_char = prop::sample::select(vec![
        '\0', '\x01', '\x02', '\x03', '\x04', '\x05', '\x06', '\x07', '\x08', '\x0B', '\x0C',
        '\x0E', '\x0F', '\x10', '\x11', '\x12', '\x13', '\x14', '\x15', '\x16', '\x17', '\x18',
        '\x19', '\x1A', '\x1B', '\x1C', '\x1D', '\x1E', '\x1F', '\n', '\r',
    ]);

    (arb_safe_value(), control_char, arb_safe_value())
        .prop_map(|(prefix, bad, suffix)| format!("{prefix}{bad}{suffix}"))
}

/// Strategy for generating valid variable names
fn arb_var_name() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_]{0,15}"
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    // ========== Property: Safe values always pass validation ==========

    #[test]
    fn safe_values_pass_command_substitution(
        name in arb_var_name(),
        value in arb_safe_value()
    ) {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(name.clone(), value));

        let input = format!("cmd ${{{name}}}");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_ok(), "Safe value should pass: {:?}", result);
    }

    // ========== Property: Control characters always rejected ==========

    #[test]
    fn control_chars_rejected_in_command_substitution(
        name in arb_var_name(),
        value in arb_unsafe_control_value()
    ) {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(name.clone(), value));

        let input = format!("cmd ${{{name}}}");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_err(), "Unsafe value should be rejected");

        if let Err(e) = result {
            let msg = e.to_string();
            prop_assert!(
                msg.contains("unsafe"),
                "Error should mention 'unsafe': {msg}"
            );
        }
    }

    // ========== Property: Null bytes always rejected ==========

    #[test]
    fn null_bytes_always_rejected(
        name in arb_var_name(),
        prefix in "[a-zA-Z0-9]{0,20}",
        suffix in "[a-zA-Z0-9]{0,20}"
    ) {
        let value = format!("{prefix}\0{suffix}");
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(name.clone(), value));

        let input = format!("${{{name}}}");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_err(), "Null byte should always be rejected");
    }

    // ========== Property: Newlines always rejected ==========

    #[test]
    fn newlines_always_rejected(
        name in arb_var_name(),
        prefix in "[a-zA-Z0-9]{0,20}",
        suffix in "[a-zA-Z0-9]{0,20}",
        newline in prop::sample::select(vec!['\n', '\r'])
    ) {
        let value = format!("{prefix}{newline}{suffix}");
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(name.clone(), value));

        let input = format!("${{{name}}}");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_err(), "Newline should always be rejected");
    }

    // ========== Property: Tab character is allowed ==========

    #[test]
    fn tab_character_allowed(
        name in arb_var_name(),
        prefix in "[a-zA-Z0-9]{1,10}",
        suffix in "[a-zA-Z0-9]{1,10}"
    ) {
        let value = format!("{prefix}\t{suffix}");
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(name.clone(), value.clone()));

        let input = format!("${{{name}}}");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_ok(), "Tab should be allowed");
        prop_assert_eq!(result.unwrap(), value);
    }

    // ========== Property: Undefined variables become empty (not error) ==========

    #[test]
    fn undefined_vars_become_empty_in_command_substitution(
        name in arb_var_name()
    ) {
        let manager = VariableManager::new();
        let input = format!("prefix_${{{name}}}_suffix");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_ok(), "Undefined vars should not error");
        prop_assert_eq!(result.unwrap(), "prefix__suffix");
    }

    // ========== Property: Multiple variables all validated ==========

    #[test]
    fn multiple_vars_all_validated(
        safe_name in "[a-zA-Z][a-zA-Z0-9]{0,8}",
        evil_name in "[a-zA-Z][a-zA-Z0-9]{0,8}",
        safe_value in "[a-zA-Z0-9]{1,20}",
    ) {
        // Ensure names are different
        prop_assume!(safe_name != evil_name);

        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(safe_name.clone(), safe_value));
        manager.set_global(Variable::new(evil_name.clone(), "bad\nvalue"));

        let input = format!("${{{safe_name}}} ${{{evil_name}}}");
        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_err(), "Should reject if any variable is unsafe");
    }

    // ========== Property: Substitution result contains no variable references ==========

    #[test]
    fn command_substitution_resolves_all_references(
        names in prop::collection::hash_set(arb_var_name(), 1..5),
        values in prop::collection::vec("[a-zA-Z0-9 .,:\\-_@#%^/=+~]{0,30}", 1..10)
    ) {
        let mut manager = VariableManager::new();
        let names: Vec<_> = names.into_iter().collect();

        for (i, name) in names.iter().enumerate() {
            let value = &values[i % values.len()];
            manager.set_global(Variable::new(name.clone(), value.clone()));
        }

        let input = names.iter()
            .map(|n| format!("${{{n}}}"))
            .collect::<Vec<_>>()
            .join(" ");

        let result = manager.substitute_for_command(&input, VariableScope::Global);
        prop_assert!(result.is_ok());

        // Verify that the original variable references are resolved
        let output = result.unwrap();
        for name in &names {
            let pattern = format!("${{{name}}}");
            prop_assert!(
                !output.contains(&pattern),
                "Variable reference {pattern} should be resolved in: {output}"
            );
        }
    }
}
