//! Property tests for log sanitization functionality

use proptest::prelude::*;
use rustconn_core::session::{contains_sensitive_prompt, sanitize_output, SanitizeConfig};

/// Strategy for generating safe text (no sensitive patterns)
fn safe_text_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 .,!?\\-_]{0,100}".prop_map(|s| s.to_string())
}

proptest! {
    /// Property: Disabled sanitization returns input unchanged
    #[test]
    fn disabled_sanitization_returns_unchanged(
        input in ".*",
    ) {
        let config = SanitizeConfig::disabled();
        let result = sanitize_output(&input, &config);
        prop_assert_eq!(result, input);
    }

    /// Property: Safe text without sensitive patterns is unchanged
    #[test]
    fn safe_text_unchanged(
        text in safe_text_strategy(),
    ) {
        let config = SanitizeConfig::new();
        let result = sanitize_output(&text, &config);
        // Safe text should not be modified (unless it accidentally matches a pattern)
        // We check that the result doesn't contain [REDACTED] for truly safe text
        if !text.to_lowercase().contains("password")
            && !text.to_lowercase().contains("token")
            && !text.to_lowercase().contains("api")
            && !text.to_lowercase().contains("secret")
            && !text.to_lowercase().contains("bearer")
            && !text.contains("AKIA")
            && !text.contains("-----BEGIN")
        {
            prop_assert_eq!(result, text);
        }
    }

    /// Property: Password patterns are always sanitized when enabled
    #[test]
    fn password_patterns_sanitized(
        prefix in "[a-zA-Z ]{0,20}",
        password_value in "[a-zA-Z0-9]{8,20}",
    ) {
        let config = SanitizeConfig::new();
        let input = format!("{prefix}password: {password_value}");
        let result = sanitize_output(&input, &config);

        // The password value should be redacted
        prop_assert!(
            !result.contains(&password_value) || result.contains("[REDACTED]"),
            "Password value should be redacted: input={input}, result={result}"
        );
    }

    /// Property: API key patterns are sanitized
    #[test]
    fn api_key_patterns_sanitized(
        key_value in "[a-zA-Z0-9]{20,40}",
    ) {
        let config = SanitizeConfig::new();
        let input = format!("api_key: {key_value}");
        let result = sanitize_output(&input, &config);

        prop_assert!(
            result.contains("[REDACTED]"),
            "API key should be redacted: input={input}, result={result}"
        );
    }

    /// Property: Bearer tokens are sanitized
    #[test]
    fn bearer_tokens_sanitized(
        token_value in "[a-zA-Z0-9._-]{20,50}",
    ) {
        let config = SanitizeConfig::new();
        let input = format!("Authorization: Bearer {token_value}");
        let result = sanitize_output(&input, &config);

        prop_assert!(
            result.contains("[REDACTED]"),
            "Bearer token should be redacted: input={input}, result={result}"
        );
    }

    /// Property: AWS access key IDs are sanitized
    #[test]
    fn aws_access_keys_sanitized(
        suffix in "[A-Z0-9]{16}",
    ) {
        let config = SanitizeConfig::new();
        let input = format!("AWS_ACCESS_KEY_ID=AKIA{suffix}");
        let result = sanitize_output(&input, &config);

        prop_assert!(
            result.contains("[REDACTED]"),
            "AWS key should be redacted: input={input}, result={result}"
        );
    }

    /// Property: Custom patterns are applied
    #[test]
    fn custom_patterns_applied(
        secret_value in "[a-z]{10,20}",
    ) {
        let config = SanitizeConfig::new()
            .with_custom_pattern(r"mysecret_[a-z]+");
        let input = format!("config: mysecret_{secret_value}");
        let result = sanitize_output(&input, &config);

        prop_assert!(
            result.contains("[REDACTED]"),
            "Custom pattern should be redacted: input={input}, result={result}"
        );
    }

    /// Property: Replacement text is configurable
    #[test]
    fn replacement_text_configurable(
        replacement in "[A-Z_]{1,10}",
        password_value in "[a-zA-Z0-9]{10,20}",
    ) {
        let replacement_text = format!("[{replacement}]");
        let config = SanitizeConfig::new()
            .with_replacement(&replacement_text);
        let input = format!("password: {password_value}");
        let result = sanitize_output(&input, &config);

        prop_assert!(
            result.contains(&replacement_text),
            "Custom replacement should be used: expected={replacement_text}, result={result}"
        );
    }

    /// Property: Full line sanitization removes entire lines with sensitive prompts
    #[test]
    fn full_line_sanitization(
        prefix in "[a-zA-Z ]{0,10}",
        suffix in "[a-zA-Z0-9 ]{0,20}",
    ) {
        let config = SanitizeConfig::new()
            .with_full_line_sanitization(true);
        let input = format!("{prefix}Password:{suffix}");
        let result = sanitize_output(&input, &config);

        prop_assert!(
            result.contains("[REDACTED]"),
            "Line with password prompt should be redacted: input={input}, result={result}"
        );
    }

    /// Property: contains_sensitive_prompt detects password prompts
    #[test]
    fn detects_password_prompts(
        prefix in "[a-zA-Z ]{0,10}",
        suffix in "[a-zA-Z ]{0,10}",
    ) {
        let line = format!("{prefix}password:{suffix}");
        prop_assert!(
            contains_sensitive_prompt(&line),
            "Should detect password prompt: {line}"
        );
    }

    /// Property: contains_sensitive_prompt is case insensitive
    #[test]
    fn sensitive_prompt_case_insensitive(
        case_variant in prop_oneof![
            Just("password:"),
            Just("PASSWORD:"),
            Just("Password:"),
            Just("PaSsWoRd:"),
        ],
    ) {
        prop_assert!(
            contains_sensitive_prompt(case_variant),
            "Should detect case variant: {case_variant}"
        );
    }

    /// Property: Safe lines don't trigger sensitive prompt detection
    #[test]
    fn safe_lines_not_detected(
        text in "[a-zA-Z0-9 .,!?]{0,50}",
    ) {
        // Filter out any text that might accidentally contain sensitive words
        let lower = text.to_lowercase();
        if !lower.contains("password")
            && !lower.contains("secret")
            && !lower.contains("token")
            && !lower.contains("pass:")
            && !lower.contains("api_key")
            && !lower.contains("private_key")
            && !lower.contains("sudo")
            && !lower.contains("passphrase")
            && !lower.contains("pin")
            && !lower.contains("otp")
            && !lower.contains("2fa")
            && !lower.contains("mfa")
        {
            prop_assert!(
                !contains_sensitive_prompt(&text),
                "Safe text should not be detected as sensitive: {text}"
            );
        }
    }
}

#[test]
fn test_sanitize_config_default() {
    let config = SanitizeConfig::default();
    assert!(config.enabled);
    assert_eq!(config.replacement, "[REDACTED]");
    assert!(config.custom_patterns.is_empty());
    assert!(!config.sanitize_full_lines);
}

#[test]
fn test_sanitize_config_disabled() {
    let config = SanitizeConfig::disabled();
    assert!(!config.enabled);
}

#[test]
fn test_private_key_sanitization() {
    let config = SanitizeConfig::new();
    let input =
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
    let result = sanitize_output(input, &config);
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn test_openssh_key_sanitization() {
    let config = SanitizeConfig::new();
    let input = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA...\n-----END OPENSSH PRIVATE KEY-----";
    let result = sanitize_output(input, &config);
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn test_multiline_sanitization() {
    let config = SanitizeConfig::new().with_full_line_sanitization(true);
    let input = "Connecting to server...\nPassword: \nAuthenticated successfully";
    let result = sanitize_output(input, &config);

    // The password line should be redacted
    assert!(result.contains("[REDACTED]"));
    // Other lines should remain
    assert!(result.contains("Connecting"));
    assert!(result.contains("Authenticated"));
}

#[test]
fn test_sensitive_prompts_list() {
    // Test various sensitive prompts
    let prompts = [
        "password:",
        "Password:",
        "Enter passphrase",
        "sudo password",
        "api_key:",
        "token:",
        "secret:",
        "Enter PIN",
        "OTP:",
        "2FA:",
    ];

    for prompt in prompts {
        assert!(contains_sensitive_prompt(prompt), "Should detect: {prompt}");
    }
}

#[test]
fn test_sha256_fingerprint_sanitization() {
    let config = SanitizeConfig::new();
    // SHA256 fingerprints are 43 base64 characters
    let input = "Host key fingerprint is SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
    let result = sanitize_output(input, &config);
    assert!(result.contains("[REDACTED]"));
}
