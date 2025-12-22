//! Property-based tests for the Connection Tasks system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Connection Tasks system (Requirements 1.x).

use proptest::prelude::*;
use rustconn_core::{
    ConnectionTask, FolderConnectionTracker, TaskCondition, TaskError, TaskTiming, Variable,
    VariableManager, VariableScope,
};

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

/// Strategy for generating task timing
fn arb_task_timing() -> impl Strategy<Value = TaskTiming> {
    prop_oneof![
        Just(TaskTiming::PreConnect),
        Just(TaskTiming::PostDisconnect),
    ]
}

/// Strategy for generating task conditions
fn arb_task_condition() -> impl Strategy<Value = TaskCondition> {
    (any::<bool>(), any::<bool>()).prop_map(|(first, last)| TaskCondition {
        only_first_in_folder: first,
        only_last_in_folder: last,
    })
}

/// Strategy for generating a list of unique variable names with values
fn arb_var_map() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec((arb_var_name(), arb_var_value()), 1..10).prop_map(|pairs| {
        // Deduplicate by name
        let mut seen = std::collections::HashSet::new();
        pairs
            .into_iter()
            .filter(|(name, _)| seen.insert(name.clone()))
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 11: Task Variable Substitution ==========
    // **Feature: rustconn-enhancements, Property 11: Task Variable Substitution**
    // **Validates: Requirements 1.6**
    //
    // For any task command containing variable references, execution should use
    // substituted values.

    #[test]
    fn task_variable_substitution_replaces_all_references(
        var_map in arb_var_map()
    ) {
        // Skip if no variables
        if var_map.is_empty() {
            return Ok(());
        }

        let mut manager = VariableManager::new();

        // Set up all variables
        for (name, value) in &var_map {
            manager.set_global(Variable::new(name.clone(), value.clone()));
        }

        // Create a command that references all variables
        let command = var_map.iter()
            .map(|(name, _)| format!("${{{}}}", name))
            .collect::<Vec<_>>()
            .join(" ");

        let task = ConnectionTask::new_pre_connect(&command);

        // Substitute variables
        let result = task.substitute_command(&manager, VariableScope::Global);
        prop_assert!(result.is_ok(), "Substitution should succeed: {:?}", result);

        let substituted = result.unwrap();

        // All variable values should appear in the result
        for (_, value) in &var_map {
            prop_assert!(
                substituted.contains(value),
                "Substituted command should contain value '{}': got '{}'",
                value, substituted
            );
        }

        // No ${...} patterns should remain
        prop_assert!(
            !substituted.contains("${"),
            "No variable references should remain after substitution: {}",
            substituted
        );
    }

    #[test]
    fn task_substitution_preserves_non_variable_text(
        prefix in "[a-zA-Z0-9]{1,10}",
        suffix in "[a-zA-Z0-9]{1,10}",
        var_name in arb_var_name(),
        var_value in arb_var_value()
    ) {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(var_name.clone(), var_value.clone()));

        let command = format!("{} ${{{}}}{}", prefix, var_name, suffix);
        let task = ConnectionTask::new_pre_connect(&command);

        let result = task.substitute_command(&manager, VariableScope::Global).unwrap();

        // Prefix and suffix should be preserved
        prop_assert!(
            result.starts_with(&prefix),
            "Prefix should be preserved: expected '{}' at start of '{}'",
            prefix, result
        );
        prop_assert!(
            result.ends_with(&suffix),
            "Suffix should be preserved: expected '{}' at end of '{}'",
            suffix, result
        );

        // Variable should be substituted
        let expected = format!("{} {}{}", prefix, var_value, suffix);
        prop_assert_eq!(result, expected);
    }

    #[test]
    fn task_substitution_uses_connection_scope_override(
        var_name in arb_var_name(),
        global_value in arb_var_value(),
        local_value in arb_var_value()
    ) {
        prop_assume!(global_value != local_value);

        let mut manager = VariableManager::new();
        let conn_id = uuid::Uuid::new_v4();

        // Set global and connection-local variables
        manager.set_global(Variable::new(var_name.clone(), global_value.clone()));
        manager.set_connection(conn_id, Variable::new(var_name.clone(), local_value.clone()));

        let command = format!("echo ${{{}}}", var_name);
        let task = ConnectionTask::new_pre_connect(&command);

        // With connection scope, should use local value
        let conn_result = task.substitute_command(&manager, VariableScope::Connection(conn_id)).unwrap();
        prop_assert!(
            conn_result.contains(&local_value),
            "Connection scope should use local value: expected '{}' in '{}'",
            local_value, conn_result
        );

        // With global scope, should use global value
        let global_result = task.substitute_command(&manager, VariableScope::Global).unwrap();
        prop_assert!(
            global_result.contains(&global_value),
            "Global scope should use global value: expected '{}' in '{}'",
            global_value, global_result
        );
    }

    #[test]
    fn task_substitution_handles_undefined_gracefully(
        defined_name in arb_var_name(),
        defined_value in arb_var_value(),
        undefined_name in arb_var_name()
    ) {
        prop_assume!(defined_name != undefined_name);

        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(defined_name.clone(), defined_value.clone()));

        // Command with both defined and undefined variables
        let command = format!("${{{}}}_${{{}}}", defined_name, undefined_name);
        let task = ConnectionTask::new_pre_connect(&command);

        let result = task.substitute_command(&manager, VariableScope::Global);
        prop_assert!(result.is_ok(), "Should handle undefined variables gracefully");

        let substituted = result.unwrap();
        // Defined variable should be substituted
        prop_assert!(
            substituted.contains(&defined_value),
            "Defined variable should be substituted"
        );
        // Undefined variable becomes empty string
        prop_assert!(
            !substituted.contains("${"),
            "No variable references should remain"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 12: Task Failure Handling ==========
    // **Feature: rustconn-enhancements, Property 12: Task Failure Handling**
    // **Validates: Requirements 1.3**
    //
    // For any pre-connect task that returns non-zero exit code, the connection
    // attempt should be aborted.

    // Note: We can't easily test actual command execution in property tests,
    // but we can test the task configuration and error handling logic.

    #[test]
    fn pre_connect_task_has_abort_on_failure_by_default(
        command in "[a-zA-Z0-9 ]{1,20}"
    ) {
        let task = ConnectionTask::new_pre_connect(&command);

        prop_assert!(
            task.abort_on_failure,
            "Pre-connect tasks should abort on failure by default"
        );
        prop_assert!(
            task.is_pre_connect(),
            "Task should be identified as pre-connect"
        );
    }

    #[test]
    fn post_disconnect_task_does_not_abort_by_default(
        command in "[a-zA-Z0-9 ]{1,20}"
    ) {
        let task = ConnectionTask::new_post_disconnect(&command);

        prop_assert!(
            !task.abort_on_failure,
            "Post-disconnect tasks should not abort on failure by default"
        );
        prop_assert!(
            task.is_post_disconnect(),
            "Task should be identified as post-disconnect"
        );
    }

    #[test]
    fn task_abort_on_failure_can_be_configured(
        command in "[a-zA-Z0-9 ]{1,20}",
        abort in any::<bool>()
    ) {
        let task = ConnectionTask::new_pre_connect(&command)
            .with_abort_on_failure(abort);

        prop_assert_eq!(
            task.abort_on_failure, abort,
            "abort_on_failure should be configurable"
        );
    }

    #[test]
    fn task_error_non_zero_exit_contains_code(
        exit_code in 1i32..255
    ) {
        let error = TaskError::NonZeroExit(exit_code);
        let error_string = error.to_string();

        prop_assert!(
            error_string.contains(&exit_code.to_string()),
            "Error message should contain exit code: {}",
            error_string
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Task Condition Tests ==========

    #[test]
    fn task_condition_should_execute_no_restrictions(
        is_first in any::<bool>(),
        is_last in any::<bool>()
    ) {
        let condition = TaskCondition::new();

        // No restrictions means always execute
        prop_assert!(
            condition.should_execute(is_first, is_last),
            "Task with no restrictions should always execute"
        );
    }

    #[test]
    fn task_condition_first_in_folder_only_executes_for_first(
        is_first in any::<bool>(),
        is_last in any::<bool>()
    ) {
        let condition = TaskCondition::first_in_folder();

        let should_execute = condition.should_execute(is_first, is_last);

        if is_first {
            prop_assert!(should_execute, "Should execute when is_first=true");
        } else {
            prop_assert!(!should_execute, "Should not execute when is_first=false");
        }
    }

    #[test]
    fn task_condition_last_in_folder_only_executes_for_last(
        is_first in any::<bool>(),
        is_last in any::<bool>()
    ) {
        let condition = TaskCondition::last_in_folder();

        let should_execute = condition.should_execute(is_first, is_last);

        if is_last {
            prop_assert!(should_execute, "Should execute when is_last=true");
        } else {
            prop_assert!(!should_execute, "Should not execute when is_last=false");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Folder Connection Tracker Tests ==========

    #[test]
    fn folder_tracker_first_connection_returns_true(
        folder_id in prop::option::of(any::<u128>().prop_map(|n| uuid::Uuid::from_u128(n)))
    ) {
        let mut tracker = FolderConnectionTracker::new();

        // First connection should return true
        let is_first = tracker.connection_opened(folder_id);
        prop_assert!(is_first, "First connection should return is_first=true");

        // Second connection should return false
        let is_first_again = tracker.connection_opened(folder_id);
        prop_assert!(!is_first_again, "Second connection should return is_first=false");
    }

    #[test]
    fn folder_tracker_last_connection_returns_true(
        folder_id in prop::option::of(any::<u128>().prop_map(|n| uuid::Uuid::from_u128(n))),
        num_connections in 1usize..10
    ) {
        let mut tracker = FolderConnectionTracker::new();

        // Open multiple connections
        for _ in 0..num_connections {
            tracker.connection_opened(folder_id);
        }

        // Close all but one
        for _ in 0..num_connections - 1 {
            let is_last = tracker.connection_closed(folder_id);
            prop_assert!(!is_last, "Should not be last while connections remain");
        }

        // Close the last one
        let is_last = tracker.connection_closed(folder_id);
        prop_assert!(is_last, "Last connection should return is_last=true");
    }

    #[test]
    fn folder_tracker_active_count_is_accurate(
        folder_id in prop::option::of(any::<u128>().prop_map(|n| uuid::Uuid::from_u128(n))),
        opens in 1usize..20,
        closes in 0usize..10
    ) {
        let mut tracker = FolderConnectionTracker::new();

        // Open connections
        for _ in 0..opens {
            tracker.connection_opened(folder_id);
        }

        // Close some connections (but not more than opened)
        let actual_closes = closes.min(opens);
        for _ in 0..actual_closes {
            tracker.connection_closed(folder_id);
        }

        let expected_count = opens - actual_closes;
        let actual_count = tracker.active_count(folder_id);

        prop_assert_eq!(
            actual_count, expected_count,
            "Active count should be {} after {} opens and {} closes",
            expected_count, opens, actual_closes
        );
    }

    #[test]
    fn folder_tracker_independent_folders(
        folder1 in any::<u128>().prop_map(|n| uuid::Uuid::from_u128(n)),
        folder2 in any::<u128>().prop_map(|n| uuid::Uuid::from_u128(n))
    ) {
        prop_assume!(folder1 != folder2);

        let mut tracker = FolderConnectionTracker::new();

        // Open connections in folder1
        tracker.connection_opened(Some(folder1));
        tracker.connection_opened(Some(folder1));

        // Open connection in folder2
        tracker.connection_opened(Some(folder2));

        // Counts should be independent
        prop_assert_eq!(tracker.active_count(Some(folder1)), 2);
        prop_assert_eq!(tracker.active_count(Some(folder2)), 1);

        // Closing in folder1 shouldn't affect folder2
        tracker.connection_closed(Some(folder1));
        prop_assert_eq!(tracker.active_count(Some(folder1)), 1);
        prop_assert_eq!(tracker.active_count(Some(folder2)), 1);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Task Serialization Tests ==========

    #[test]
    fn task_json_round_trip(
        command in "[a-zA-Z0-9 ]{1,50}",
        timing in arb_task_timing(),
        condition in arb_task_condition(),
        timeout in prop::option::of(1000u32..60000),
        abort in any::<bool>(),
        description in prop::option::of("[a-zA-Z0-9 ]{1,30}")
    ) {
        let mut task = ConnectionTask::with_id(uuid::Uuid::new_v4(), timing, &command)
            .with_condition(condition)
            .with_abort_on_failure(abort);

        if let Some(t) = timeout {
            task = task.with_timeout(t);
        }
        if let Some(d) = description {
            task = task.with_description(d);
        }

        // Serialize to JSON
        let json = serde_json::to_string(&task).expect("Serialization should succeed");

        // Deserialize back
        let parsed: ConnectionTask = serde_json::from_str(&json).expect("Deserialization should succeed");

        // Should be equal
        prop_assert_eq!(task.id, parsed.id);
        prop_assert_eq!(task.timing, parsed.timing);
        prop_assert_eq!(task.command, parsed.command);
        prop_assert_eq!(task.condition, parsed.condition);
        prop_assert_eq!(task.timeout_ms, parsed.timeout_ms);
        prop_assert_eq!(task.abort_on_failure, parsed.abort_on_failure);
        prop_assert_eq!(task.description, parsed.description);
    }

    #[test]
    fn task_timing_serialization_round_trip(
        timing in arb_task_timing()
    ) {
        let json = serde_json::to_string(&timing).expect("Serialization should succeed");
        let parsed: TaskTiming = serde_json::from_str(&json).expect("Deserialization should succeed");

        prop_assert_eq!(timing, parsed);
    }

    #[test]
    fn task_condition_serialization_round_trip(
        condition in arb_task_condition()
    ) {
        let json = serde_json::to_string(&condition).expect("Serialization should succeed");
        let parsed: TaskCondition = serde_json::from_str(&json).expect("Deserialization should succeed");

        prop_assert_eq!(condition, parsed);
    }
}
