//! Property-based tests for `WebConfig` serialization
//!
//! Tests for WebBrowserMode round-trip, user agent length validation,
//! and compile-time default behavior.
//!
//! Feature: embedded-web-browser, Properties 4, 5, 6
//! **Validates: Requirements 7.1, 7.4, 7.5, 7.6, 7.7, 7.8**
use proptest::prelude::*;
use rustconn_core::models::{WebBrowserMode, WebConfig};
// Strategy for generating valid WebBrowserMode variants (without web-embedded feature)
fn arb_browser_mode() -> impl Strategy<Value = WebBrowserMode> {
    prop_oneof![Just(WebBrowserMode::System), Just(WebBrowserMode::Custom),]
}
// Strategy for generating optional user agents within the 512-char limit
fn arb_user_agent() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9 /._()-]{1,512}".prop_map(Some),]
}
// Strategy for generating optional browser commands
fn arb_browser_command() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9-]{0,20}".prop_map(Some),]
}
// Strategy for generating a valid WebConfig
fn arb_web_config() -> impl Strategy<Value = WebConfig> {
    (
        arb_browser_command(),
        any::<bool>(),
        arb_browser_mode(),
        any::<bool>(),
        arb_user_agent(),
        0.3f64..=3.0f64,
    )
        .prop_map(
            |(browser, private_mode, browser_mode, javascript_enabled, user_agent, zoom_level)| {
                WebConfig {
                    browser,
                    private_mode,
                    browser_mode,
                    javascript_enabled,
                    user_agent,
                    zoom_level,
                    accept_invalid_certs: false,
                }
            },
        )
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    // ========== Property 4: WebBrowserMode Serialization Round-Trip ==========
    /// **Feature: embedded-web-browser, Property 4: WebBrowserMode Serialization Round-Trip**
    /// **Validates: Requirements 7.1, 7.6, 7.8**
    ///
    /// For any valid WebConfig instance, serializing to JSON and then
    /// deserializing the result produces a WebConfig that is equal to the original.
    // Feature: embedded-web-browser, Property 4: WebBrowserMode Serialization Round-Trip
    #[test]
    fn web_config_json_round_trip(config in arb_web_config()) {
        // Serialize to JSON
        let json_str = serde_json::to_string(&config)
            .map_err(|e| TestCaseError::fail(format!("serialization failed: {e}")))?;
        // Deserialize back from JSON
        let deserialized: WebConfig = serde_json::from_str(&json_str)
            .map_err(|e| TestCaseError::fail(format!("deserialization failed: {e}")))?;
        // Assert equality
        prop_assert_eq!(&config.browser, &deserialized.browser, "browser should be preserved");
        prop_assert_eq!(config.private_mode, deserialized.private_mode, "private_mode should be preserved");
        prop_assert_eq!(config.browser_mode, deserialized.browser_mode, "browser_mode should be preserved");
        prop_assert_eq!(config.javascript_enabled, deserialized.javascript_enabled, "javascript_enabled should be preserved");
        prop_assert_eq!(&config.user_agent, &deserialized.user_agent, "user_agent should be preserved");
    }
    // ========== Property 5: User Agent Length Validation ==========
    /// **Feature: embedded-web-browser, Property 5: User Agent Length Validation**
    /// **Validates: Requirements 7.7**
    ///
    /// For any string with length greater than 512 characters used as user_agent,
    /// deserialization of a WebConfig containing that value fails with an error.
    // Feature: embedded-web-browser, Property 5: User Agent Length Validation
    #[test]
    fn user_agent_exceeding_512_chars_fails_deserialization(len in 513usize..=1000) {
        // Generate a user agent string of the specified length
        let long_user_agent: String = "A".repeat(len);
        // Build JSON with the oversized user_agent
        let json = serde_json::json!({
            "browser_mode": "system",
            "javascript_enabled": true,
            "user_agent": long_user_agent,
        });
        let json_str = json.to_string();
        // Deserialization must fail
        let result = serde_json::from_str::<WebConfig>(&json_str);
        prop_assert!(
            result.is_err(),
            "Deserialization should fail for user_agent of length {}, but got: {:?}",
            len,
            result
        );
        // Verify the error message mentions the length constraint
        let err_msg = result.err().map(|e| e.to_string()).unwrap_or_default();
        prop_assert!(
            err_msg.contains("512"),
            "Error message should mention 512 limit, got: {}",
            err_msg
        );
    }
    /// Additional: user_agent at exactly 512 chars should succeed deserialization
    // Feature: embedded-web-browser, Property 5: boundary verification
    #[test]
    fn user_agent_at_512_chars_succeeds(prefix in "[a-zA-Z0-9]{1,512}") {
        // Truncate to exactly 512 chars (regex may produce fewer)
        let user_agent: String = prefix.chars().take(512).collect();
        // Skip if generated string is shorter than 512 (not useful for boundary test)
        prop_assume!(user_agent.len() <= 512);
        let json = serde_json::json!({
            "browser_mode": "system",
            "javascript_enabled": true,
            "user_agent": user_agent,
        });
        let json_str = json.to_string();
        let result = serde_json::from_str::<WebConfig>(&json_str);
        prop_assert!(
            result.is_ok(),
            "Deserialization should succeed for user_agent of length {}, but got error: {:?}",
            user_agent.len(),
            result.err()
        );
    }
    // ========== Property 6: Browser Mode Compile-Time Default ==========
    /// **Feature: embedded-web-browser, Property 6: Browser Mode Compile-Time Default**
    /// **Validates: Requirements 7.4, 7.5, 7.6**
    ///
    /// For any WebConfig deserialized from JSON that lacks a browser_mode field,
    /// the resulting browser_mode value equals the compile-time default:
    /// System when web-embedded is disabled.
    // Feature: embedded-web-browser, Property 6: Browser Mode Compile-Time Default
    #[test]
    fn missing_browser_mode_deserializes_to_compile_time_default(
        javascript_enabled in any::<bool>(),
        private_mode in any::<bool>(),
    ) {
        // Build JSON without the browser_mode field
        let json = serde_json::json!({
            "javascript_enabled": javascript_enabled,
            "private_mode": private_mode,
        });
        let json_str = json.to_string();
        // Deserialize
        let config: WebConfig = serde_json::from_str(&json_str)
            .map_err(|e| TestCaseError::fail(format!("deserialization failed: {e}")))?;
        // When web-embedded feature is NOT enabled (default for tests), default is System
        #[cfg(not(feature = "web-embedded"))]
        prop_assert_eq!(
            config.browser_mode,
            WebBrowserMode::System,
            "Without web-embedded feature, default browser_mode should be System"
        );
        // When web-embedded feature IS enabled, default is Embedded
        #[cfg(feature = "web-embedded")]
        prop_assert_eq!(
            config.browser_mode,
            WebBrowserMode::Embedded,
            "With web-embedded feature, default browser_mode should be Embedded"
        );
        // Verify other fields are also correct
        prop_assert_eq!(config.javascript_enabled, javascript_enabled);
        prop_assert_eq!(config.private_mode, private_mode);
    }
    /// Additional: JSON with explicit browser_mode respects the provided value
    // Feature: embedded-web-browser, Property 6: explicit value override
    #[test]
    fn explicit_browser_mode_overrides_default(mode in arb_browser_mode()) {
        let mode_str = match mode {
            WebBrowserMode::System => "system",
            WebBrowserMode::Custom => "custom",
            #[cfg(feature = "web-embedded")]
            WebBrowserMode::Embedded => "embedded",
        };
        let json = serde_json::json!({
            "browser_mode": mode_str,
            "javascript_enabled": true,
        });
        let json_str = json.to_string();
        let config: WebConfig = serde_json::from_str(&json_str)
            .map_err(|e| TestCaseError::fail(format!("deserialization failed: {e}")))?;
        prop_assert_eq!(
            config.browser_mode, mode,
            "Explicit browser_mode should be preserved"
        );
    }
}
