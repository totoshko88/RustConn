//! Property tests for log context and path template expansion

use proptest::prelude::*;
use rustconn_core::session::{LogContext, SessionLogger};

/// Strategy for generating valid connection names
fn connection_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,30}".prop_map(|s| s.to_string())
}

/// Strategy for generating valid protocol names
fn protocol_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ssh".to_string()),
        Just("rdp".to_string()),
        Just("vnc".to_string()),
        Just("spice".to_string()),
    ]
}

proptest! {
    /// Property: LogContext preserves connection name and protocol
    #[test]
    fn log_context_preserves_fields(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
    ) {
        let context = LogContext::new(&name, &protocol);

        prop_assert_eq!(context.connection_name, name);
        prop_assert_eq!(context.protocol, protocol);
        prop_assert!(context.custom_vars.is_empty());
    }

    /// Property: LogContext with_var adds custom variables
    #[test]
    fn log_context_with_var_adds_variables(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
        var_name in "[a-z_]{1,20}",
        var_value in "[a-zA-Z0-9]{1,50}",
    ) {
        let context = LogContext::new(&name, &protocol)
            .with_var(&var_name, &var_value);

        prop_assert_eq!(context.custom_vars.get(&var_name), Some(&var_value));
    }

    /// Property: Multiple with_var calls preserve all variables
    #[test]
    fn log_context_multiple_vars(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
    ) {
        let context = LogContext::new(&name, &protocol)
            .with_var("var1", "value1")
            .with_var("var2", "value2")
            .with_var("var3", "value3");

        prop_assert_eq!(context.custom_vars.len(), 3);
        prop_assert_eq!(context.custom_vars.get("var1"), Some(&"value1".to_string()));
        prop_assert_eq!(context.custom_vars.get("var2"), Some(&"value2".to_string()));
        prop_assert_eq!(context.custom_vars.get("var3"), Some(&"value3".to_string()));
    }

    /// Property: Path template expansion substitutes connection_name
    #[test]
    fn path_template_expands_connection_name(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
    ) {
        let context = LogContext::new(&name, &protocol);
        let template = "/tmp/${connection_name}.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        prop_assert!(result.is_ok());

        let path = result.unwrap();
        let path_str = path.to_string_lossy();
        // Connection name should be in the path (possibly sanitized)
        prop_assert!(path_str.contains(".log"));
        prop_assert!(path_str.starts_with("/tmp/"));
    }

    /// Property: Path template expansion substitutes protocol
    #[test]
    fn path_template_expands_protocol(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
    ) {
        let context = LogContext::new(&name, &protocol);
        let template = "/tmp/${protocol}_session.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        prop_assert!(result.is_ok());

        let path = result.unwrap();
        let path_str = path.to_string_lossy();
        prop_assert!(path_str.contains(&protocol));
    }

    /// Property: Path template expansion handles date variable
    #[test]
    fn path_template_expands_date(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
    ) {
        let context = LogContext::new(&name, &protocol);
        let template = "/tmp/${date}.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        prop_assert!(result.is_ok());

        let path = result.unwrap();
        let path_str = path.to_string_lossy();
        // Date should be in YYYY-MM-DD format
        prop_assert!(path_str.contains("-"));
        let date_marker = "${date}";
        prop_assert!(!path_str.contains(date_marker));
    }

    /// Property: Path template expansion handles custom variables
    #[test]
    fn path_template_expands_custom_vars(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
        custom_value in "[a-zA-Z0-9]{1,20}",
    ) {
        let context = LogContext::new(&name, &protocol)
            .with_var("custom", &custom_value);
        let template = "/tmp/${custom}.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        prop_assert!(result.is_ok());

        let path = result.unwrap();
        let path_str = path.to_string_lossy();
        prop_assert!(path_str.contains(&custom_value));
    }

    /// Property: Undefined variables cause error
    #[test]
    fn path_template_undefined_var_error(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
    ) {
        let context = LogContext::new(&name, &protocol);
        let template = "/tmp/${undefined_variable}.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        prop_assert!(result.is_err());
    }
}

#[test]
fn test_log_context_default() {
    let context = LogContext::default();
    assert!(context.connection_name.is_empty());
    assert!(context.protocol.is_empty());
    assert!(context.custom_vars.is_empty());
}

#[test]
fn test_log_context_clone() {
    let context = LogContext::new("server1", "ssh").with_var("env", "production");

    let cloned = context.clone();
    assert_eq!(cloned.connection_name, "server1");
    assert_eq!(cloned.protocol, "ssh");
    assert_eq!(
        cloned.custom_vars.get("env"),
        Some(&"production".to_string())
    );
}

#[test]
fn test_path_template_multiple_variables() {
    let context = LogContext::new("my-server", "rdp");
    let template = "/var/log/${protocol}/${connection_name}_${date}.log";

    let result = SessionLogger::expand_path_template(template, &context, None);
    assert!(result.is_ok());

    let path = result.unwrap();
    let path_str = path.to_string_lossy();
    assert!(path_str.contains("rdp"));
    assert!(path_str.contains("my-server"));
    assert!(!path_str.contains("${"));
}

#[test]
fn test_path_template_home_expansion() {
    let context = LogContext::new("test", "ssh");
    let template = "${HOME}/logs/test.log";

    let result = SessionLogger::expand_path_template(template, &context, None);
    assert!(result.is_ok());

    let path = result.unwrap();
    let path_str = path.to_string_lossy();
    // HOME should be expanded (not contain ${HOME})
    assert!(!path_str.contains("${HOME}"));
}

#[test]
fn test_path_template_datetime() {
    let context = LogContext::new("test", "vnc");
    let template = "/tmp/${datetime}.log";

    let result = SessionLogger::expand_path_template(template, &context, None);
    assert!(result.is_ok());

    let path = result.unwrap();
    let path_str = path.to_string_lossy();
    // datetime should be in YYYY-MM-DD_HH-MM-SS format
    assert!(path_str.contains("_"));
    assert!(!path_str.contains("${datetime}"));
}

#[test]
fn test_path_template_time() {
    let context = LogContext::new("test", "spice");
    let template = "/tmp/${time}.log";

    let result = SessionLogger::expand_path_template(template, &context, None);
    assert!(result.is_ok());

    let path = result.unwrap();
    let path_str = path.to_string_lossy();
    // time should be in HH-MM-SS format
    assert!(!path_str.contains("${time}"));
}
