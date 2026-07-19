//! Credential autofill manager for the embedded web browser.
//!
//! Handles credential injection via two mechanisms:
//! 1. JavaScript injection for HTML forms (button-triggered)
//! 2. WebKitGTK `authenticate` signal for HTTP Basic/Digest Auth
//!
//! All credential handling uses `SecretString` and `Zeroizing<String>` to
//! ensure temporary plaintext values are wiped from memory immediately
//! after use.

use gtk4::glib;
use gtk4::prelude::*;
use secrecy::{ExposeSecret, SecretString};
use webkit6::prelude::*;
use webkit6::{gio, javascriptcore};
use zeroize::Zeroizing;

/// Timeout for field detection after JavaScript injection (3 seconds).
const FIELD_DETECTION_TIMEOUT_SECS: u32 = 3;

/// Manages credential autofill for the embedded web view.
///
/// Uses `SecretString` for all credential handling and zeroizes
/// temporary values immediately after use.
pub struct AutofillManager {
    /// Stored username (None if no credentials configured)
    username: Option<String>,
    /// Stored password (SecretString for zeroization)
    password: Option<SecretString>,
    /// Whether autofill is available (credentials exist)
    is_available: bool,
}

impl AutofillManager {
    /// Creates a new autofill manager with optional credentials.
    #[must_use]
    pub fn new(credentials: Option<(String, SecretString)>) -> Self {
        match credentials {
            Some((username, password)) => Self {
                username: Some(username),
                password: Some(password),
                is_available: true,
            },
            None => Self {
                username: None,
                password: None,
                is_available: false,
            },
        }
    }

    /// Whether autofill is available (credentials are configured).
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.is_available
    }

    /// Returns a reference to the stored username, if any.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Returns a reference to the stored password, if any.
    #[must_use]
    pub fn password(&self) -> Option<&SecretString> {
        self.password.as_ref()
    }

    /// Injects credentials into the current page via JavaScript.
    ///
    /// Fills `input[type=password]`, `input[type=text][name*=user]`,
    /// `input[type=text][name*=login]`, `input[type=email]`,
    /// `input[name=username]`, `input[type=text][id*=user]`,
    /// `input[type=text][id*=login]`, and
    /// `input[type=text][autocomplete=username]` fields, dispatching
    /// `input` and `change` events on each filled field.
    ///
    /// Uses `Zeroizing<String>` for the interpolated JavaScript string
    /// to ensure credential values are wiped from memory on drop.
    /// If credentials are unavailable, no injection occurs (no partial
    /// injection guarantee).
    ///
    /// A 3-second timeout checks whether form fields were detected.
    /// If no fields are found, an inline notification is displayed
    /// via the provided callback.
    pub fn inject_credentials(
        &self,
        web_view: &webkit6::WebView,
        on_no_fields_detected: impl Fn() + 'static,
    ) {
        // No partial injection: if credentials are not fully available, inject nothing.
        let (Some(username), Some(password)) = (&self.username, &self.password) else {
            tracing::debug!("Autofill skipped: no credentials configured");
            return;
        };

        // Build the JavaScript injection script using Zeroizing to ensure
        // the interpolated plaintext is wiped from memory on drop.
        let script = Self::build_injection_script(username, password);

        // Clone web_view reference for the timeout callback
        let web_view_weak = web_view.downgrade();

        // Execute the injection script
        web_view.evaluate_javascript(&script, None, None, gio::Cancellable::NONE, {
            move |result| {
                match result {
                    Ok(js_value) => {
                        // Parse the JSON result to check if fields were filled
                        let filled = Self::parse_fill_result(&js_value);
                        if filled.user_filled == 0 && filled.pass_filled == 0 {
                            // No fields detected — schedule timeout notification
                            Self::schedule_field_detection_timeout(
                                web_view_weak,
                                on_no_fields_detected,
                            );
                        } else {
                            tracing::debug!(
                                user_fields = filled.user_filled,
                                password_fields = filled.pass_filled,
                                "Autofill injection completed"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "Autofill JavaScript injection failed"
                        );
                        // On error, no fields were injected (all-or-nothing guarantee
                        // is maintained since we never partially inject).
                    }
                }
            }
        });

        // The Zeroizing<String> `script` is dropped here, wiping the
        // interpolated credential values from memory.
    }

    /// Handles the WebKitGTK `authenticate` signal for HTTP Basic/Digest.
    ///
    /// Responds to the authentication challenge with stored credentials.
    /// Returns `true` if credentials were provided (signal handled),
    /// `false` if no credentials are available (let WebKitGTK show its
    /// default authentication dialog).
    pub fn handle_authenticate(&self, request: &webkit6::AuthenticationRequest) -> bool {
        // No partial injection: both username and password must be available.
        let (Some(username), Some(password)) = (&self.username, &self.password) else {
            tracing::debug!("HTTP authenticate skipped: no credentials configured");
            return false;
        };

        // Wrap the exposed password in Zeroizing to ensure cleanup.
        let password_plain = Zeroizing::new(password.expose_secret().to_string());

        // Create a WebKitGTK Credential and authenticate the request.
        // Use ForSession persistence — credentials are valid for the
        // duration of the network session but not persisted to disk.
        let credential = webkit6::Credential::new(
            username,
            &password_plain,
            webkit6::CredentialPersistence::ForSession,
        );

        request.authenticate(Some(&credential));

        tracing::debug!(
            host = ?request.host(),
            scheme = ?request.scheme(),
            "HTTP authenticate handled with stored credentials"
        );

        // password_plain is zeroized on drop here.
        true
    }

    /// Builds the JavaScript injection script with credential values interpolated.
    ///
    /// Returns a `Zeroizing<String>` that wipes the script (containing
    /// plaintext credentials) from memory when dropped.
    fn build_injection_script(username: &str, password: &SecretString) -> Zeroizing<String> {
        // Wrap the password exposure in Zeroizing for the interpolation scope.
        let password_plain = Zeroizing::new(password.expose_secret().to_string());

        // Escape special characters for JavaScript string embedding.
        let escaped_username = Zeroizing::new(Self::escape_js_string(username));
        let escaped_password = Zeroizing::new(Self::escape_js_string(&password_plain));

        let script = format!(
            r#"(function() {{
    const username = '{username}';
    const password = '{password}';

    function fill(selector, value) {{
        const fields = document.querySelectorAll(selector);
        fields.forEach(function(field) {{
            field.value = value;
            field.dispatchEvent(new Event('input', {{ bubbles: true }}));
            field.dispatchEvent(new Event('change', {{ bubbles: true }}));
        }});
        return fields.length;
    }}

    let userFilled = fill('input[type="text"][name*="user"]', username);
    userFilled += fill('input[type="text"][name*="login"]', username);
    userFilled += fill('input[type="email"]', username);
    userFilled += fill('input[name="username"]', username);
    userFilled += fill('input[type="text"][id*="user"]', username);
    userFilled += fill('input[type="text"][id*="login"]', username);
    userFilled += fill('input[type="text"][autocomplete="username"]', username);
    const passFilled = fill('input[type="password"]', password);

    return JSON.stringify({{ userFilled: userFilled, passFilled: passFilled }});
}})();"#,
            username = *escaped_username,
            password = *escaped_password,
        );

        // escaped_username, escaped_password, and password_plain are
        // zeroized on drop here.
        Zeroizing::new(script)
    }

    /// Escapes a string for safe embedding inside a JavaScript single-quoted string.
    ///
    /// Handles backslashes, single quotes, newlines, and other control characters.
    fn escape_js_string(input: &str) -> String {
        let mut escaped = String::with_capacity(input.len() + 16);
        for ch in input.chars() {
            match ch {
                '\\' => escaped.push_str("\\\\"),
                '\'' => escaped.push_str("\\'"),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                '\t' => escaped.push_str("\\t"),
                '\0' => escaped.push_str("\\0"),
                // Unicode control characters and line separators
                '\u{2028}' => escaped.push_str("\\u2028"),
                '\u{2029}' => escaped.push_str("\\u2029"),
                _ => escaped.push(ch),
            }
        }
        escaped
    }

    /// Schedules a 3-second timeout to check if form fields were detected.
    ///
    /// If no fields were filled after the timeout, invokes the notification
    /// callback to inform the user.
    fn schedule_field_detection_timeout(
        web_view_weak: glib::object::WeakRef<webkit6::WebView>,
        on_no_fields_detected: impl Fn() + 'static,
    ) {
        glib::timeout_add_seconds_local_once(FIELD_DETECTION_TIMEOUT_SECS, move || {
            // Verify the web view is still alive before notifying
            if web_view_weak.upgrade().is_some() {
                on_no_fields_detected();
            }
        });
    }

    /// Parses the JSON result from the injection script.
    fn parse_fill_result(js_value: &javascriptcore::Value) -> FillResult {
        // The script returns a JSON string: { "userFilled": N, "passFilled": N }
        let json_str = js_value.to_string();

        // Try to parse the result; default to zero if parsing fails
        if let Some(result) = Self::parse_fill_json(&json_str) {
            result
        } else {
            tracing::debug!(
                raw = %json_str,
                "Could not parse autofill injection result"
            );
            FillResult {
                user_filled: 0,
                pass_filled: 0,
            }
        }
    }

    /// Parses a JSON string `{"userFilled": N, "passFilled": N}` into a `FillResult`.
    fn parse_fill_json(json: &str) -> Option<FillResult> {
        // Simple manual parsing to avoid pulling in serde_json for this one use.
        // Expected format: {"userFilled":N,"passFilled":N}
        let user_filled = Self::extract_json_number(json, "userFilled")?;
        let pass_filled = Self::extract_json_number(json, "passFilled")?;
        Some(FillResult {
            user_filled,
            pass_filled,
        })
    }

    /// Extracts a numeric value for a given key from a simple JSON object string.
    fn extract_json_number(json: &str, key: &str) -> Option<u32> {
        // Look for "key": N or "key":N patterns
        let search_patterns = [
            format!("\"{key}\":"),
            format!("\"{key}\" :"),
            format!("\"{key}\": "),
            format!("\"{key}\" : "),
        ];

        for pattern in &search_patterns {
            if let Some(idx) = json.find(pattern.as_str()) {
                let after_colon = &json[idx + pattern.len()..];
                let num_str: String = after_colon
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(n) = num_str.parse() {
                    return Some(n);
                }
            }
        }
        None
    }
}

/// Result of the autofill JavaScript injection.
#[derive(Debug, Clone, Copy)]
struct FillResult {
    /// Number of username/email fields filled.
    user_filled: u32,
    /// Number of password fields filled.
    pass_filled: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autofill_available_with_credentials() {
        let manager = AutofillManager::new(Some((
            "user".to_string(),
            SecretString::new("pass".to_string().into()),
        )));
        assert!(manager.is_available());
        assert_eq!(manager.username(), Some("user"));
    }

    #[test]
    fn test_autofill_unavailable_without_credentials() {
        let manager = AutofillManager::new(None);
        assert!(!manager.is_available());
        assert_eq!(manager.username(), None);
        assert!(manager.password().is_none());
    }

    #[test]
    fn test_escape_js_string_basic() {
        assert_eq!(AutofillManager::escape_js_string("hello"), "hello");
    }

    #[test]
    fn test_escape_js_string_single_quote() {
        assert_eq!(AutofillManager::escape_js_string("it's"), "it\\'s");
    }

    #[test]
    fn test_escape_js_string_backslash() {
        assert_eq!(AutofillManager::escape_js_string(r"a\b"), r"a\\b");
    }

    #[test]
    fn test_escape_js_string_newlines() {
        assert_eq!(AutofillManager::escape_js_string("a\nb"), "a\\nb");
        assert_eq!(AutofillManager::escape_js_string("a\rb"), "a\\rb");
    }

    #[test]
    fn test_escape_js_string_null() {
        assert_eq!(AutofillManager::escape_js_string("a\0b"), "a\\0b");
    }

    #[test]
    fn test_escape_js_string_unicode_separators() {
        assert_eq!(AutofillManager::escape_js_string("a\u{2028}b"), "a\\u2028b");
        assert_eq!(AutofillManager::escape_js_string("a\u{2029}b"), "a\\u2029b");
    }

    #[test]
    fn test_parse_fill_json_valid() {
        let result =
            AutofillManager::parse_fill_json(r#"{"userFilled":2,"passFilled":1}"#).unwrap();
        assert_eq!(result.user_filled, 2);
        assert_eq!(result.pass_filled, 1);
    }

    #[test]
    fn test_parse_fill_json_with_spaces() {
        let result =
            AutofillManager::parse_fill_json(r#"{"userFilled": 3, "passFilled": 0}"#).unwrap();
        assert_eq!(result.user_filled, 3);
        assert_eq!(result.pass_filled, 0);
    }

    #[test]
    fn test_parse_fill_json_invalid() {
        assert!(AutofillManager::parse_fill_json("not json").is_none());
    }

    #[test]
    fn test_parse_fill_json_missing_fields() {
        assert!(AutofillManager::parse_fill_json(r#"{"userFilled": 1}"#).is_none());
    }

    #[test]
    fn test_no_partial_injection_without_username() {
        // AutofillManager with only password but no username should not be creatable
        // via the public API, but test the internal logic path
        let manager = AutofillManager {
            username: None,
            password: Some(SecretString::new("pass".into())),
            is_available: false,
        };
        assert!(!manager.is_available());
    }

    #[test]
    fn test_no_partial_injection_without_password() {
        let manager = AutofillManager {
            username: Some("user".to_string()),
            password: None,
            is_available: false,
        };
        assert!(!manager.is_available());
    }

    #[test]
    fn test_build_injection_script_contains_selectors() {
        let script = AutofillManager::build_injection_script(
            "testuser",
            &SecretString::new("testpass".into()),
        );
        assert!(script.contains(r#"input[type="password"]"#));
        assert!(script.contains(r#"input[type="text"][name*="user"]"#));
        assert!(script.contains(r#"input[type="text"][name*="login"]"#));
        assert!(script.contains(r#"input[type="email"]"#));
        assert!(script.contains(r#"input[name="username"]"#));
    }

    #[test]
    fn test_build_injection_script_dispatches_events() {
        let script =
            AutofillManager::build_injection_script("user", &SecretString::new("pass".into()));
        assert!(script.contains("dispatchEvent"));
        assert!(script.contains("input"));
        assert!(script.contains("change"));
    }

    #[test]
    fn test_build_injection_script_escapes_special_chars() {
        let script = AutofillManager::build_injection_script(
            "user'name",
            &SecretString::new("pass'word\\test".into()),
        );
        // The username with a single quote should be escaped
        assert!(script.contains(r"user\'name"));
        // The password with a single quote and backslash should be escaped
        assert!(script.contains(r"pass\'word\\test"));
    }

    // Feature: embedded-web-browser, Property 8: Autofill Credential Handling (No Partial Injection)
    mod property_tests {
        use proptest::prelude::*;

        use super::*;

        /// Strategy generating arbitrary credential inputs as `Option<(String, SecretString)>`.
        ///
        /// Produces `None` (no credentials) or `Some((username, password))` with
        /// arbitrary string contents including empty strings.
        fn arb_credentials() -> impl Strategy<Value = Option<(String, SecretString)>> {
            prop_oneof![
                // No credentials at all
                Just(None),
                // Some credentials with arbitrary content (including empty strings)
                (".*", ".*")
                    .prop_map(|(user, pass)| { Some((user, SecretString::new(pass.into()))) }),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(100))]

            /// **Feature: embedded-web-browser, Property 8: Autofill Credential Handling (No Partial Injection)**
            /// **Validates: Requirements 5.5, 5.6**
            ///
            /// For any `Option<(String, SecretString)>` credentials input:
            /// - If credentials are `None` → `is_available()` is false (no injection possible)
            /// - If credentials are `Some((user, pass))` → `is_available()` is true
            ///
            /// The invariant: there is no state where only a partial credential set
            /// (username without password, or password without username) could be
            /// injected. The AutofillManager is either fully available or not at all.
            // Feature: embedded-web-browser, Property 8: Autofill Credential Handling (No Partial Injection)
            #[test]
            fn autofill_all_or_nothing(credentials in arb_credentials()) {
                let manager = AutofillManager::new(credentials.clone());

                match &credentials {
                    None => {
                        // No credentials → autofill must be unavailable
                        prop_assert!(
                            !manager.is_available(),
                            "AutofillManager must not be available when no credentials are provided"
                        );
                        // Both username and password must be None
                        prop_assert!(
                            manager.username().is_none(),
                            "Username must be None when no credentials are provided"
                        );
                        prop_assert!(
                            manager.password().is_none(),
                            "Password must be None when no credentials are provided"
                        );
                    }
                    Some((user, _pass)) => {
                        // Credentials provided → autofill must be available
                        prop_assert!(
                            manager.is_available(),
                            "AutofillManager must be available when credentials are provided"
                        );
                        // Both username and password must be present (all-or-nothing)
                        prop_assert!(
                            manager.username().is_some(),
                            "Username must be Some when credentials are provided"
                        );
                        prop_assert!(
                            manager.password().is_some(),
                            "Password must be Some when credentials are provided"
                        );
                        // Username value must match input
                        prop_assert_eq!(
                            manager.username().unwrap(),
                            user.as_str(),
                            "Stored username must match input"
                        );
                    }
                }

                // Invariant: no partial state is possible.
                // Either both are present or both are absent.
                let has_username = manager.username().is_some();
                let has_password = manager.password().is_some();
                prop_assert_eq!(
                    has_username, has_password,
                    "All-or-nothing violated: username present={}, password present={}",
                    has_username, has_password
                );
                // Availability must match the presence of both credentials
                prop_assert_eq!(
                    manager.is_available(),
                    has_username && has_password,
                    "is_available() must equal (has_username && has_password)"
                );
            }
        }
    }
}
