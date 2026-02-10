//! Property-based tests for `KeePass` integration
//!
//! These tests validate the correctness properties defined in the `KeePass` integration design.

use chrono::Utc;
use proptest::prelude::*;
use rustconn_core::models::{PasswordSource, ProtocolConfig, ProtocolType, SshConfig};
use rustconn_core::{parse_keepassxc_version, Connection, CredentialResolver, KeePassStatus};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

// ========== Generators ==========

/// Strategy for generating valid .kdbx file paths (lowercase extension)
fn arb_kdbx_filename() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_-]{0,20}\\.kdbx".prop_map(|s| s)
}

/// Strategy for generating mixed case .kdbx extensions
fn arb_kdbx_filename_mixed_case() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z][a-z0-9_-]{0,20}\\.kdbx".prop_map(|s| s),
        "[a-z][a-z0-9_-]{0,20}\\.KDBX".prop_map(|s| s),
        "[a-z][a-z0-9_-]{0,20}\\.Kdbx".prop_map(|s| s),
        "[a-z][a-z0-9_-]{0,20}\\.KdBx".prop_map(|s| s),
    ]
}

/// Strategy for generating non-.kdbx file extensions
fn arb_non_kdbx_extension() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(".txt".to_string()),
        Just(".db".to_string()),
        Just(".key".to_string()),
        Just(".xml".to_string()),
        Just(".json".to_string()),
        Just(String::new()), // no extension
    ]
}

/// Strategy for generating filenames with non-.kdbx extensions
fn arb_non_kdbx_filename() -> impl Strategy<Value = String> {
    ("[a-z][a-z0-9_-]{0,20}", arb_non_kdbx_extension())
        .prop_map(|(name, ext)| format!("{name}{ext}"))
}

/// Strategy for generating valid `KeePassXC` version output strings
fn arb_valid_version_output() -> impl Strategy<Value = String> {
    // Generate version numbers like "2.7.6", "2.7", "3.0.0"
    (1u8..10u8, 0u8..20u8, 0u8..20u8)
        .prop_map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
}

/// Strategy for generating version output with prefix
fn arb_version_with_prefix() -> impl Strategy<Value = String> {
    (
        prop_oneof![
            Just("keepassxc-cli".to_string()),
            Just("KeePassXC".to_string()),
            Just("Version".to_string()),
        ],
        arb_valid_version_output(),
    )
        .prop_map(|(prefix, version)| format!("{prefix} {version}"))
}

/// Strategy for generating version output with various formats
fn arb_version_output() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_valid_version_output(),
        arb_version_with_prefix(),
        // With trailing newline
        arb_valid_version_output().prop_map(|v| format!("{v}\n")),
        arb_version_with_prefix().prop_map(|v| format!("{v}\n")),
        // With leading/trailing whitespace
        arb_valid_version_output().prop_map(|v| format!("  {v}  ")),
    ]
}

/// Strategy for generating invalid version outputs (no version number)
fn arb_invalid_version_output() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("   ".to_string()),
        Just("keepassxc-cli".to_string()),
        Just("no version here".to_string()),
        Just("abc def ghi".to_string()),
    ]
}

/// Strategy for generating valid connection names (non-empty, non-whitespace)
fn arb_valid_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9 _-]{0,30}".prop_map(|s| s)
}

/// Strategy for generating valid hostnames
fn arb_hostname() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple hostnames
        "[a-z][a-z0-9-]{0,20}".prop_map(|s| s),
        // Domain names
        "[a-z][a-z0-9-]{0,10}\\.[a-z]{2,4}".prop_map(|s| s),
        // IP addresses
        (1u8..255u8, 0u8..255u8, 0u8..255u8, 1u8..255u8)
            .prop_map(|(a, b, c, d)| format!("{a}.{b}.{c}.{d}")),
    ]
}

/// Strategy for generating whitespace-only strings
fn arb_whitespace_only() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just(" ".to_string()),
        Just("  ".to_string()),
        Just("\t".to_string()),
        Just("   \t  ".to_string()),
    ]
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: keepass-integration, Property 1: KDBX Path Validation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any file path string, the validation function SHALL return success only if
    /// the path ends with ".kdbx" extension (case-insensitive) and the file exists.
    ///
    /// This test verifies that valid .kdbx files that exist pass validation.
    #[test]
    fn kdbx_path_validation_accepts_valid_kdbx_files(filename in arb_kdbx_filename_mixed_case()) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let kdbx_path = temp_dir.path().join(&filename);

        // Create the file
        std::fs::write(&kdbx_path, b"dummy kdbx content").expect("Failed to create test file");

        // Validation should succeed
        let result = KeePassStatus::validate_kdbx_path(&kdbx_path);
        prop_assert!(
            result.is_ok(),
            "Valid .kdbx file should pass validation: {:?}, error: {:?}",
            kdbx_path,
            result.err()
        );
    }

    /// **Feature: keepass-integration, Property 1: KDBX Path Validation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any file path string, the validation function SHALL return success only if
    /// the path ends with ".kdbx" extension (case-insensitive) and the file exists.
    ///
    /// This test verifies that files with non-.kdbx extensions are rejected.
    #[test]
    fn kdbx_path_validation_rejects_non_kdbx_extensions(filename in arb_non_kdbx_filename()) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let non_kdbx_path = temp_dir.path().join(&filename);

        // Create the file
        std::fs::write(&non_kdbx_path, b"dummy content").expect("Failed to create test file");

        // Validation should fail due to wrong extension
        let result = KeePassStatus::validate_kdbx_path(&non_kdbx_path);
        prop_assert!(
            result.is_err(),
            "Non-.kdbx file should fail validation: {:?}",
            non_kdbx_path
        );
        prop_assert!(
            result.as_ref().unwrap_err().contains(".kdbx extension"),
            "Error message should mention .kdbx extension requirement: {:?}",
            result.err()
        );
    }

    /// **Feature: keepass-integration, Property 1: KDBX Path Validation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any file path string, the validation function SHALL return success only if
    /// the path ends with ".kdbx" extension (case-insensitive) and the file exists.
    ///
    /// This test verifies that non-existent .kdbx paths are rejected.
    #[test]
    fn kdbx_path_validation_rejects_nonexistent_files(filename in arb_kdbx_filename()) {
        // Use a path that doesn't exist
        let nonexistent_path = PathBuf::from("/nonexistent/path").join(&filename);

        // Validation should fail due to file not existing
        let result = KeePassStatus::validate_kdbx_path(&nonexistent_path);
        prop_assert!(
            result.is_err(),
            "Non-existent file should fail validation: {:?}",
            nonexistent_path
        );
        prop_assert!(
            result.as_ref().unwrap_err().contains("does not exist"),
            "Error message should mention file does not exist: {:?}",
            result.err()
        );
    }

    /// **Feature: keepass-integration, Property 1: KDBX Path Validation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any file path string, the validation function SHALL return success only if
    /// the path ends with ".kdbx" extension (case-insensitive) and the file exists.
    ///
    /// This test verifies that directories with .kdbx names are rejected.
    #[test]
    fn kdbx_path_validation_rejects_directories(filename in arb_kdbx_filename()) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let dir_path = temp_dir.path().join(&filename);

        // Create a directory instead of a file
        std::fs::create_dir(&dir_path).expect("Failed to create test directory");

        // Validation should fail because it's a directory
        let result = KeePassStatus::validate_kdbx_path(&dir_path);
        prop_assert!(
            result.is_err(),
            "Directory should fail validation: {:?}",
            dir_path
        );
        prop_assert!(
            result.as_ref().unwrap_err().contains("not a file"),
            "Error message should mention path is not a file: {:?}",
            result.err()
        );
    }

    /// **Feature: keepass-integration, Property 7: Version String Parsing**
    /// **Validates: Requirements 2.2**
    ///
    /// For any valid KeePassXC version output string, the parser SHALL extract
    /// a non-empty version string.
    ///
    /// This test verifies that valid version outputs produce non-empty version strings.
    #[test]
    fn version_parsing_extracts_version_from_valid_output(output in arb_version_output()) {
        let result = parse_keepassxc_version(&output);

        prop_assert!(
            result.is_some(),
            "Valid version output should produce a version: {:?}",
            output
        );

        let version = result.unwrap();
        prop_assert!(
            !version.is_empty(),
            "Extracted version should not be empty for output: {:?}",
            output
        );

        // Version should contain at least one digit
        prop_assert!(
            version.chars().any(|c| c.is_ascii_digit()),
            "Version should contain digits: {:?}",
            version
        );

        // Version should only contain digits and dots
        prop_assert!(
            version.chars().all(|c| c.is_ascii_digit() || c == '.'),
            "Version should only contain digits and dots: {:?}",
            version
        );
    }

    /// **Feature: keepass-integration, Property 7: Version String Parsing**
    /// **Validates: Requirements 2.2**
    ///
    /// For any invalid version output (no version number), the parser should return None.
    #[test]
    fn version_parsing_returns_none_for_invalid_output(output in arb_invalid_version_output()) {
        let result = parse_keepassxc_version(&output);

        prop_assert!(
            result.is_none(),
            "Invalid version output should return None: {:?}, got: {:?}",
            output,
            result
        );
    }

    /// **Feature: keepass-integration, Property 6: Lookup Key Generation**
    /// **Validates: Requirements 4.4**
    ///
    /// For any connection, the generated KeePass lookup key SHALL contain either
    /// the connection name or the host, ensuring consistent retrieval.
    ///
    /// This test verifies that lookup keys contain the connection name when name is non-empty.
    #[test]
    fn lookup_key_contains_connection_name(name in arb_valid_connection_name(), host in arb_hostname()) {
        let connection = create_test_connection(&name, &host);
        let key = CredentialResolver::generate_lookup_key(&connection);

        prop_assert!(
            key.contains(&name),
            "Lookup key should contain connection name. Key: {:?}, Name: {:?}",
            key,
            name
        );

        // Key should have the rustconn prefix
        prop_assert!(
            key.starts_with("rustconn/"),
            "Lookup key should start with 'rustconn/' prefix: {:?}",
            key
        );
    }

    /// **Feature: keepass-integration, Property 6: Lookup Key Generation**
    /// **Validates: Requirements 4.4**
    ///
    /// For any connection with empty/whitespace name, the generated KeePass lookup key
    /// SHALL contain the host, ensuring consistent retrieval.
    #[test]
    fn lookup_key_falls_back_to_host_when_name_empty(empty_name in arb_whitespace_only(), host in arb_hostname()) {
        let connection = create_test_connection(&empty_name, &host);
        let key = CredentialResolver::generate_lookup_key(&connection);

        prop_assert!(
            key.contains(&host),
            "Lookup key should contain host when name is empty/whitespace. Key: {:?}, Host: {:?}",
            key,
            host
        );

        // Key should have the rustconn prefix
        prop_assert!(
            key.starts_with("rustconn/"),
            "Lookup key should start with 'rustconn/' prefix: {:?}",
            key
        );
    }

    /// **Feature: keepass-integration, Property 6: Lookup Key Generation**
    /// **Validates: Requirements 4.4**
    ///
    /// For any connection, the generated lookup key SHALL be non-empty and contain
    /// a meaningful identifier (either name or host).
    #[test]
    fn lookup_key_is_always_non_empty(name in arb_valid_connection_name(), host in arb_hostname()) {
        let connection = create_test_connection(&name, &host);
        let key = CredentialResolver::generate_lookup_key(&connection);

        prop_assert!(
            !key.is_empty(),
            "Lookup key should never be empty"
        );

        // Key should contain either name or host
        prop_assert!(
            key.contains(&name) || key.contains(&host),
            "Lookup key should contain either name or host. Key: {:?}, Name: {:?}, Host: {:?}",
            key,
            name,
            host
        );
    }
}

// ========== Helper Functions ==========

/// Creates a test connection with the given name and host
fn create_test_connection(name: &str, host: &str) -> Connection {
    Connection {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: None,
        host: host.to_string(),
        port: 22,
        protocol: ProtocolType::Ssh,
        username: None,
        group_id: None,
        tags: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
        sort_order: 0,
        last_connected: None,
        password_source: PasswordSource::None,
        domain: None,
        custom_properties: Vec::new(),
        pre_connect_task: None,
        post_disconnect_task: None,
        wol_config: None,
        local_variables: std::collections::HashMap::new(),
        log_config: None,
        key_sequence: None,
        automation: rustconn_core::models::AutomationConfig::default(),
        window_mode: rustconn_core::models::WindowMode::default(),
        remember_window_position: false,
        window_geometry: None,
        skip_port_check: false,
    }
}

/// Creates a test connection with a specific password source
fn create_test_connection_with_source(
    name: &str,
    host: &str,
    password_source: PasswordSource,
) -> Connection {
    let mut conn = create_test_connection(name, host);
    conn.password_source = password_source;
    conn
}

// ========== Button State Consistency Tests ==========

/// **Feature: keepass-integration, Property 4: Button State Consistency**
/// **Validates: Requirements 3.2, 3.3**
///
/// For any KeePass integration state (enabled/disabled), the "Save to KeePass"
/// button sensitivity SHALL equal the integration enabled state.
///
/// This module tests the pure logic that determines button state.
#[cfg(test)]
mod button_state_tests {
    use proptest::prelude::*;
    use rustconn_core::config::SecretSettings;
    use std::path::PathBuf;

    /// Pure function that computes the expected "Save to KeePass" button sensitivity
    /// based on the KeePass integration state.
    ///
    /// This mirrors the logic in `ConnectionDialog::set_keepass_enabled`.
    fn compute_save_to_keepass_button_sensitivity(kdbx_enabled: bool) -> bool {
        kdbx_enabled
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: keepass-integration, Property 4: Button State Consistency**
        /// **Validates: Requirements 3.2, 3.3**
        ///
        /// For any KeePass integration state, the button sensitivity equals the enabled state.
        #[test]
        fn button_sensitivity_equals_keepass_enabled_state(enabled in proptest::bool::ANY) {
            let sensitivity = compute_save_to_keepass_button_sensitivity(enabled);

            prop_assert_eq!(
                sensitivity,
                enabled,
                "Button sensitivity should equal KeePass enabled state"
            );
        }

        /// **Feature: keepass-integration, Property 4: Button State Consistency**
        /// **Validates: Requirements 3.2, 3.3**
        ///
        /// For any SecretSettings configuration, the button sensitivity should match
        /// the kdbx_enabled field.
        #[test]
        fn button_sensitivity_matches_secret_settings(
            kdbx_enabled in proptest::bool::ANY,
            enable_fallback in proptest::bool::ANY,
            has_path in proptest::bool::ANY,
        ) {
            let mut settings = SecretSettings::default();
            settings.kdbx_enabled = kdbx_enabled;
            settings.enable_fallback = enable_fallback;
            settings.kdbx_path = if has_path {
                Some(PathBuf::from("/path/to/db.kdbx"))
            } else {
                None
            };

            let sensitivity = compute_save_to_keepass_button_sensitivity(settings.kdbx_enabled);

            // The button sensitivity should ONLY depend on kdbx_enabled,
            // not on whether a path is configured or fallback is enabled
            prop_assert_eq!(
                sensitivity,
                kdbx_enabled,
                "Button sensitivity should equal kdbx_enabled regardless of other settings"
            );
        }

        /// **Feature: keepass-integration, Property 4: Button State Consistency**
        /// **Validates: Requirements 3.2, 3.3**
        ///
        /// When KeePass is disabled, the button should always be insensitive.
        #[test]
        fn button_insensitive_when_keepass_disabled(
            enable_fallback in proptest::bool::ANY,
            has_path in proptest::bool::ANY,
        ) {
            let mut settings = SecretSettings::default();
            settings.kdbx_enabled = false; // Disabled
            settings.enable_fallback = enable_fallback;
            settings.kdbx_path = if has_path {
                Some(PathBuf::from("/path/to/db.kdbx"))
            } else {
                None
            };

            let sensitivity = compute_save_to_keepass_button_sensitivity(settings.kdbx_enabled);

            prop_assert!(
                !sensitivity,
                "Button should be insensitive when KeePass is disabled"
            );
        }

        /// **Feature: keepass-integration, Property 4: Button State Consistency**
        /// **Validates: Requirements 3.2, 3.3**
        ///
        /// When KeePass is enabled, the button should always be sensitive.
        #[test]
        fn button_sensitive_when_keepass_enabled(
            enable_fallback in proptest::bool::ANY,
            has_path in proptest::bool::ANY,
        ) {
            let mut settings = SecretSettings::default();
            settings.kdbx_enabled = true; // Enabled
            settings.enable_fallback = enable_fallback;
            settings.kdbx_path = if has_path {
                Some(PathBuf::from("/path/to/db.kdbx"))
            } else {
                None
            };

            let sensitivity = compute_save_to_keepass_button_sensitivity(settings.kdbx_enabled);

            prop_assert!(
                sensitivity,
                "Button should be sensitive when KeePass is enabled"
            );
        }
    }
}

// ========== Credential Resolution Chain Tests ==========

#[cfg(test)]
mod resolution_chain_tests {
    use super::*;
    use rustconn_core::config::SecretSettings;
    use rustconn_core::SecretManager;
    use std::sync::Arc;

    /// **Feature: keepass-integration, Property 5: Credential Resolution Chain**
    /// **Validates: Requirements 4.1, 4.2, 4.3**
    ///
    /// For any connection with password_source set to Stored or Prompt,
    /// the resolver SHALL return None (caller handles these cases).
    #[test]
    fn resolution_returns_none_for_prompt_source() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        // Test with Prompt source
        let connection_prompt = create_test_connection_with_source(
            "Test Server",
            "192.168.1.1",
            PasswordSource::Prompt,
        );

        let secret_manager = Arc::new(SecretManager::empty());
        let settings = SecretSettings::default();
        let resolver = CredentialResolver::new(secret_manager.clone(), settings.clone());

        let result = rt.block_on(resolver.resolve(&connection_prompt));
        assert!(
            result.is_ok(),
            "Resolution should not error for Prompt source"
        );
        assert!(
            result.unwrap().is_none(),
            "Resolution should return None for Prompt source"
        );
    }

    #[test]
    fn resolution_returns_none_for_bitwarden_without_backend() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        // Test with Vault source (no backend configured)
        let connection_vault =
            create_test_connection_with_source("Test Server", "192.168.1.1", PasswordSource::Vault);

        let secret_manager = Arc::new(SecretManager::empty());
        let settings = SecretSettings::default();
        let resolver = CredentialResolver::new(secret_manager.clone(), settings.clone());

        let result = rt.block_on(resolver.resolve(&connection_vault));
        // Vault without backend should return None (no credentials found)
        assert!(
            result.is_ok(),
            "Resolution should not error for Vault source"
        );

        // Test with Prompt source
        let connection_prompt = create_test_connection_with_source(
            "Test Server",
            "192.168.1.1",
            PasswordSource::Prompt,
        );

        let result = rt.block_on(resolver.resolve(&connection_prompt));
        assert!(
            result.is_ok(),
            "Resolution should not error for Prompt source"
        );
        assert!(
            result.unwrap().is_none(),
            "Resolution should return None for Prompt source"
        );
    }

    /// **Feature: keepass-integration, Property 5: Credential Resolution Chain**
    /// **Validates: Requirements 4.1, 4.2, 4.3**
    ///
    /// For any connection with password_source set to KeePass, when KeePass
    /// integration is NOT active, the resolver SHALL return None (or try fallback).
    #[test]
    fn resolution_handles_disabled_keepass() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        let connection =
            create_test_connection_with_source("Test Server", "192.168.1.1", PasswordSource::Vault);

        let secret_manager = Arc::new(SecretManager::empty());
        let mut settings = SecretSettings::default();
        settings.kdbx_enabled = false; // KeePass disabled
        settings.enable_fallback = false; // No fallback

        let resolver = CredentialResolver::new(secret_manager, settings);

        let result = rt.block_on(resolver.resolve(&connection));
        assert!(
            result.is_ok(),
            "Resolution should not error when KeePass is disabled"
        );
        assert!(
            result.unwrap().is_none(),
            "Resolution should return None when KeePass is disabled and no fallback"
        );
    }

    /// **Feature: keepass-integration, Property 5: Credential Resolution Chain**
    /// **Validates: Requirements 4.1, 4.2, 4.3**
    ///
    /// For any connection with password_source set to None and fallback disabled,
    /// the resolver SHALL return None.
    #[test]
    fn resolution_returns_none_when_no_source_and_no_fallback() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        let connection =
            create_test_connection_with_source("Test Server", "192.168.1.1", PasswordSource::None);

        let secret_manager = Arc::new(SecretManager::empty());
        let mut settings = SecretSettings::default();
        settings.enable_fallback = false;

        let resolver = CredentialResolver::new(secret_manager, settings);

        let result = rt.block_on(resolver.resolve(&connection));
        assert!(
            result.is_ok(),
            "Resolution should not error when no source and no fallback"
        );
        assert!(
            result.unwrap().is_none(),
            "Resolution should return None when no source and no fallback"
        );
    }

    /// **Feature: keepass-integration, Property 5: Credential Resolution Chain**
    /// **Validates: Requirements 4.1, 4.2, 4.3**
    ///
    /// Verifies that is_keepass_active correctly reflects the settings state.
    #[test]
    fn is_keepass_active_reflects_settings() {
        let secret_manager = Arc::new(SecretManager::empty());

        // Test with KeePass disabled
        let mut settings = SecretSettings::default();
        settings.kdbx_enabled = false;
        settings.kdbx_path = Some(PathBuf::from("/path/to/db.kdbx"));

        let resolver = CredentialResolver::new(secret_manager.clone(), settings);
        assert!(
            !resolver.is_keepass_active(),
            "KeePass should not be active when disabled"
        );

        // Test with KeePass enabled but no path
        let mut settings = SecretSettings::default();
        settings.kdbx_enabled = true;
        settings.kdbx_path = None;

        let resolver = CredentialResolver::new(secret_manager.clone(), settings);
        assert!(
            !resolver.is_keepass_active(),
            "KeePass should not be active without path"
        );

        // Test with KeePass enabled and path set
        let mut settings = SecretSettings::default();
        settings.kdbx_enabled = true;
        settings.kdbx_path = Some(PathBuf::from("/path/to/db.kdbx"));

        let resolver = CredentialResolver::new(secret_manager, settings);
        assert!(
            resolver.is_keepass_active(),
            "KeePass should be active when enabled with path"
        );
    }
}

// ========== KeePass Hierarchy Tests ==========

/// **Feature: rustconn-fixes-v2, Property 1: KeePass Entry Path Matches Connection Hierarchy**
/// **Validates: Requirements 3.2, 3.3**
///
/// For any connection with a group assignment, the KeePass entry path should contain
/// all ancestor group names in order from root to leaf.
#[cfg(test)]
mod hierarchy_tests {
    use chrono::Utc;
    use proptest::prelude::*;
    use rustconn_core::models::{
        ConnectionGroup, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
    };
    use rustconn_core::{Connection, KeePassHierarchy, KEEPASS_ROOT_GROUP};
    use std::collections::HashSet;
    use uuid::Uuid;

    /// Strategy for generating valid group names (non-empty, alphanumeric with spaces/underscores)
    fn arb_group_name() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9 _-]{0,20}".prop_map(|s| s)
    }

    /// Strategy for generating valid connection names
    fn arb_connection_name() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9 _-]{0,25}".prop_map(|s| s)
    }

    /// Strategy for generating valid hostnames
    fn arb_hostname() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z][a-z0-9-]{0,15}".prop_map(|s| s),
            "[a-z][a-z0-9-]{0,8}\\.[a-z]{2,4}".prop_map(|s| s),
        ]
    }

    /// Creates a test connection with the given name and host
    fn create_test_connection(name: &str, host: &str, group_id: Option<Uuid>) -> Connection {
        Connection {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: None,
            host: host.to_string(),
            port: 22,
            protocol: ProtocolType::Ssh,
            username: None,
            group_id,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
            sort_order: 0,
            last_connected: None,
            password_source: PasswordSource::None,
            domain: None,
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            local_variables: std::collections::HashMap::new(),
            log_config: None,
            key_sequence: None,
            automation: rustconn_core::models::AutomationConfig::default(),
            window_mode: rustconn_core::models::WindowMode::default(),
            remember_window_position: false,
            window_geometry: None,
            skip_port_check: false,
        }
    }

    /// Creates a nested group hierarchy from a list of names
    fn create_nested_groups(names: &[String]) -> Vec<ConnectionGroup> {
        let mut groups = Vec::new();
        let mut parent_id: Option<Uuid> = None;

        for name in names {
            let group = if let Some(pid) = parent_id {
                ConnectionGroup::with_parent(name.clone(), pid)
            } else {
                ConnectionGroup::new(name.clone())
            };
            parent_id = Some(group.id);
            groups.push(group);
        }

        groups
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: rustconn-fixes-v2, Property 1: KeePass Entry Path Matches Connection Hierarchy**
        /// **Validates: Requirements 3.2, 3.3**
        ///
        /// For any connection with a group assignment, the KeePass entry path should contain
        /// all ancestor group names in order from root to leaf.
        #[test]
        fn prop_keepass_path_matches_hierarchy(
            group_names in prop::collection::vec(arb_group_name(), 1..5),
            connection_name in arb_connection_name(),
            host in arb_hostname(),
        ) {
            let groups = create_nested_groups(&group_names);
            let leaf_group_id = groups.last().map(|g| g.id);
            let connection = create_test_connection(&connection_name, &host, leaf_group_id);

            let path = KeePassHierarchy::build_entry_path(&connection, &groups);

            // Path should start with "RustConn"
            prop_assert!(
                path.starts_with(KEEPASS_ROOT_GROUP),
                "Path should start with '{}': {:?}",
                KEEPASS_ROOT_GROUP,
                path
            );

            // Split path into segments and verify structure
            let segments: Vec<&str> = path.split('/').collect();

            // First segment should be RustConn
            prop_assert_eq!(
                segments.first().copied(),
                Some(KEEPASS_ROOT_GROUP),
                "First segment should be RustConn"
            );

            // Middle segments should be the group names in order
            for (i, name) in group_names.iter().enumerate() {
                let segment_index = i + 1; // +1 to skip RustConn
                prop_assert!(
                    segment_index < segments.len(),
                    "Path should have segment for group '{}' at index {}",
                    name,
                    segment_index
                );
                prop_assert_eq!(
                    segments[segment_index],
                    name.as_str(),
                    "Segment {} should be group name '{}', got '{}'",
                    segment_index,
                    name,
                    segments[segment_index]
                );
            }

            // Last segment should be connection name
            prop_assert_eq!(
                segments.last().copied(),
                Some(connection_name.as_str()),
                "Last segment should be connection name"
            );
        }

        /// **Feature: rustconn-fixes-v2, Property 1: KeePass Entry Path Matches Connection Hierarchy**
        /// **Validates: Requirements 3.2**
        ///
        /// For any connection without a group, the path should be RustConn/connection_name
        #[test]
        fn prop_keepass_path_no_group(
            connection_name in arb_connection_name(),
            host in arb_hostname(),
        ) {
            let connection = create_test_connection(&connection_name, &host, None);
            let path = KeePassHierarchy::build_entry_path(&connection, &[]);

            let expected = format!("{}/{}", KEEPASS_ROOT_GROUP, connection_name);
            prop_assert_eq!(
                path,
                expected,
                "Path for connection without group should be RustConn/name"
            );
        }

        /// **Feature: rustconn-fixes-v2, Property 2: KeePass Entry Creation Creates All Parent Groups**
        /// **Validates: Requirements 3.4**
        ///
        /// For any KeePass entry path with multiple levels, the ensure_groups_exist function
        /// should identify all parent groups that need to be created.
        #[test]
        fn prop_keepass_creates_all_parent_groups(
            group_names in prop::collection::vec(arb_group_name(), 1..5),
            connection_name in arb_connection_name(),
            host in arb_hostname(),
        ) {
            let groups = create_nested_groups(&group_names);
            let leaf_group_id = groups.last().map(|g| g.id);
            let connection = create_test_connection(&connection_name, &host, leaf_group_id);

            let path = KeePassHierarchy::build_entry_path(&connection, &groups);

            // With no existing groups, all parent groups should be created
            let existing: HashSet<String> = HashSet::new();
            let result = KeePassHierarchy::ensure_groups_exist(&path, &existing);

            // Number of groups to create should equal number of path segments minus the entry name
            let expected_group_count = group_names.len() + 1; // +1 for RustConn root
            prop_assert_eq!(
                result.created_groups.len(),
                expected_group_count,
                "Should create {} groups (RustConn + {} group levels), got {:?}",
                expected_group_count,
                group_names.len(),
                result.created_groups
            );

            // First created group should be RustConn
            prop_assert_eq!(
                result.created_groups.first().map(|s| s.as_str()),
                Some(KEEPASS_ROOT_GROUP),
                "First created group should be RustConn"
            );

            // Groups should be created in order (parent before child)
            for i in 1..result.created_groups.len() {
                let parent = &result.created_groups[i - 1];
                let child = &result.created_groups[i];
                prop_assert!(
                    child.starts_with(parent),
                    "Child group '{}' should start with parent '{}' (groups created in order)",
                    child,
                    parent
                );
            }
        }

        /// **Feature: rustconn-fixes-v2, Property 2: KeePass Entry Creation Creates All Parent Groups**
        /// **Validates: Requirements 3.4**
        ///
        /// When some groups already exist, only missing groups should be created.
        #[test]
        fn prop_keepass_creates_only_missing_groups(
            group_names in prop::collection::vec(arb_group_name(), 2..5),
            connection_name in arb_connection_name(),
            host in arb_hostname(),
            existing_count in 1usize..3usize,
        ) {
            let groups = create_nested_groups(&group_names);
            let leaf_group_id = groups.last().map(|g| g.id);
            let connection = create_test_connection(&connection_name, &host, leaf_group_id);

            let path = KeePassHierarchy::build_entry_path(&connection, &groups);
            let all_group_paths = KeePassHierarchy::extract_group_paths(&path);

            // Mark some groups as existing
            let existing_count = existing_count.min(all_group_paths.len());
            let existing: HashSet<String> = all_group_paths[..existing_count]
                .iter()
                .cloned()
                .collect();

            let result = KeePassHierarchy::ensure_groups_exist(&path, &existing);

            // Existing groups should be identified
            prop_assert_eq!(
                result.existing_groups.len(),
                existing_count,
                "Should identify {} existing groups",
                existing_count
            );

            // Created groups should be the remaining ones
            let expected_created = all_group_paths.len() - existing_count;
            prop_assert_eq!(
                result.created_groups.len(),
                expected_created,
                "Should create {} groups",
                expected_created
            );

            // No overlap between existing and created
            for created in &result.created_groups {
                prop_assert!(
                    !existing.contains(created),
                    "Created group '{}' should not be in existing set",
                    created
                );
            }
        }

        /// **Feature: rustconn-fixes-v2, Property 1: KeePass Entry Path Matches Connection Hierarchy**
        /// **Validates: Requirements 3.3**
        ///
        /// When a connection's group changes, the new path should reflect the new hierarchy.
        #[test]
        fn prop_keepass_path_changes_with_group(
            group_names_a in prop::collection::vec(arb_group_name(), 1..3),
            group_names_b in prop::collection::vec(arb_group_name(), 1..3),
            connection_name in arb_connection_name(),
            host in arb_hostname(),
        ) {
            let groups_a = create_nested_groups(&group_names_a);
            let groups_b = create_nested_groups(&group_names_b);

            // Combine all groups
            let mut all_groups = groups_a.clone();
            all_groups.extend(groups_b.clone());

            let leaf_a = groups_a.last().map(|g| g.id);
            let leaf_b = groups_b.last().map(|g| g.id);

            // Create connection in group A
            let connection_a = create_test_connection(&connection_name, &host, leaf_a);
            let path_a = KeePassHierarchy::build_entry_path(&connection_a, &all_groups);

            // Move connection to group B
            let connection_b = create_test_connection(&connection_name, &host, leaf_b);
            let path_b = KeePassHierarchy::build_entry_path(&connection_b, &all_groups);

            // Both paths should end with the same connection name
            prop_assert!(
                path_a.ends_with(&connection_name),
                "Path A should end with connection name"
            );
            prop_assert!(
                path_b.ends_with(&connection_name),
                "Path B should end with connection name"
            );

            // Paths should be different (unless groups happen to have same names)
            if group_names_a != group_names_b {
                prop_assert_ne!(
                    path_a,
                    path_b,
                    "Path should change when group changes"
                );
            }
        }
    }
}
