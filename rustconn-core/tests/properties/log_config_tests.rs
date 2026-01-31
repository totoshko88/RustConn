//! Property tests for log configuration

use proptest::prelude::*;
use rustconn_core::session::LogConfig;

/// Strategy for generating valid path templates
fn path_template_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("/tmp/${connection_name}.log".to_string()),
        Just("${HOME}/logs/${date}_${protocol}.log".to_string()),
        Just("/var/log/rustconn/${connection_name}_${datetime}.log".to_string()),
    ]
}

proptest! {
    /// Property: LogConfig builder preserves all values
    #[test]
    fn log_config_builder_preserves_values(
        path in path_template_strategy(),
        enabled in any::<bool>(),
        max_size_mb in 0u32..1000,
        retention_days in 0u32..365,
    ) {
        let config = LogConfig::new(&path)
            .with_enabled(enabled)
            .with_max_size_mb(max_size_mb)
            .with_retention_days(retention_days);

        prop_assert_eq!(config.path_template, path);
        prop_assert_eq!(config.enabled, enabled);
        prop_assert_eq!(config.max_size_mb, max_size_mb);
        prop_assert_eq!(config.retention_days, retention_days);
    }

    /// Property: LogConfig timestamp format is preserved
    #[test]
    fn timestamp_format_preserved(
        format in prop_oneof![
            Just("%Y-%m-%d %H:%M:%S"),
            Just("%H:%M:%S"),
            Just("%Y%m%d_%H%M%S"),
            Just("[%Y-%m-%d]"),
        ],
    ) {
        let config = LogConfig::new("/tmp/test.log")
            .with_timestamp_format(format);

        prop_assert_eq!(config.timestamp_format, format);
    }

    /// Property: Log mode flags are independent
    #[test]
    fn log_mode_flags_independent(
        log_activity in any::<bool>(),
        log_input in any::<bool>(),
        log_output in any::<bool>(),
    ) {
        let config = LogConfig::new("/tmp/test.log")
            .with_log_activity(log_activity)
            .with_log_input(log_input)
            .with_log_output(log_output);

        prop_assert_eq!(config.log_activity, log_activity);
        prop_assert_eq!(config.log_input, log_input);
        prop_assert_eq!(config.log_output, log_output);
    }

    /// Property: Validation fails for empty path when enabled
    #[test]
    fn validation_fails_empty_path_when_enabled(_dummy in 0..1) {
        let config = LogConfig::new("")
            .with_enabled(true);

        prop_assert!(config.validate().is_err());
    }

    /// Property: Validation succeeds for empty path when disabled
    #[test]
    fn validation_succeeds_empty_path_when_disabled(_dummy in 0..1) {
        let config = LogConfig::new("")
            .with_enabled(false);

        prop_assert!(config.validate().is_ok());
    }

    /// Property: Validation succeeds for non-empty path when enabled
    #[test]
    fn validation_succeeds_nonempty_path(
        path in path_template_strategy(),
    ) {
        let config = LogConfig::new(&path)
            .with_enabled(true);

        prop_assert!(config.validate().is_ok());
    }
}

#[test]
fn test_log_config_default() {
    let config = LogConfig::default();
    assert!(!config.enabled);
    assert!(!config.path_template.is_empty());
    assert_eq!(config.max_size_mb, 10);
    assert_eq!(config.retention_days, 30);
    assert!(config.log_activity);
    assert!(!config.log_input);
    assert!(!config.log_output);
}

#[test]
fn test_log_config_new_enables_logging() {
    let config = LogConfig::new("/tmp/test.log");
    assert!(config.enabled);
    assert_eq!(config.path_template, "/tmp/test.log");
}

#[test]
fn test_log_config_chaining() {
    let config = LogConfig::new("/tmp/test.log")
        .with_enabled(true)
        .with_timestamp_format("%H:%M:%S")
        .with_max_size_mb(50)
        .with_retention_days(7)
        .with_log_activity(true)
        .with_log_input(true)
        .with_log_output(false);

    assert!(config.enabled);
    assert_eq!(config.timestamp_format, "%H:%M:%S");
    assert_eq!(config.max_size_mb, 50);
    assert_eq!(config.retention_days, 7);
    assert!(config.log_activity);
    assert!(config.log_input);
    assert!(!config.log_output);
}

#[test]
fn test_log_config_equality() {
    let config1 = LogConfig::new("/tmp/test.log")
        .with_max_size_mb(10)
        .with_retention_days(30);

    let config2 = LogConfig::new("/tmp/test.log")
        .with_max_size_mb(10)
        .with_retention_days(30);

    assert_eq!(config1, config2);
}

#[test]
fn test_log_config_inequality() {
    let config1 = LogConfig::new("/tmp/test1.log");
    let config2 = LogConfig::new("/tmp/test2.log");

    assert_ne!(config1, config2);
}

#[test]
fn test_log_config_serialization() {
    let config = LogConfig::new("/tmp/test.log")
        .with_enabled(true)
        .with_max_size_mb(20);

    // Test that serialization works
    let json = serde_json::to_string(&config).expect("serialization should work");
    assert!(json.contains("/tmp/test.log"));
    assert!(json.contains("20"));

    // Test deserialization
    let restored: LogConfig = serde_json::from_str(&json).expect("deserialization should work");
    assert_eq!(restored.path_template, config.path_template);
    assert_eq!(restored.max_size_mb, config.max_size_mb);
}
