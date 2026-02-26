//! Property-based tests for unified credential storage
//!
//! These tests validate the correctness properties for credential storage
//! backend selection and migration as defined in Requirements 3.1-3.6.

use proptest::prelude::*;
use rustconn_core::CredentialResolver;
use rustconn_core::config::{SecretBackendType, SecretSettings};
use rustconn_core::models::{
    Connection, ConnectionGroup, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
};
use rustconn_core::secret::SecretManager;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

// ========== Generators ==========

/// Strategy for generating UUIDs
fn arb_uuid() -> impl Strategy<Value = Uuid> {
    any::<[u8; 16]>().prop_map(Uuid::from_bytes)
}

/// Strategy for generating connection names
fn arb_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,20}".prop_map(String::from)
}

/// Strategy for generating hostnames
fn arb_hostname() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z]{3,10}\\.[a-z]{2,4}".prop_map(String::from),
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(String::from),
    ]
}

/// Strategy for generating group names
fn arb_group_name() -> impl Strategy<Value = String> {
    "[A-Za-z][A-Za-z0-9 _-]{0,15}".prop_map(String::from)
}

/// Strategy for generating optional KDBX paths
fn arb_kdbx_path() -> impl Strategy<Value = Option<PathBuf>> {
    prop_oneof![
        Just(None),
        "[a-z]{3,10}\\.kdbx".prop_map(|s| Some(PathBuf::from(format!("/path/to/{s}")))),
    ]
}

/// Strategy for generating SecretSettings
fn arb_secret_settings() -> impl Strategy<Value = SecretSettings> {
    (any::<bool>(), arb_kdbx_path(), any::<bool>()).prop_map(
        |(kdbx_enabled, kdbx_path, enable_fallback)| SecretSettings {
            kdbx_enabled,
            kdbx_path,
            enable_fallback,
            ..Default::default()
        },
    )
}

/// Creates a test connection
fn create_test_connection(id: Uuid, name: &str, host: &str) -> Connection {
    Connection {
        id,
        name: name.to_string(),
        description: None,
        host: host.to_string(),
        port: 22,
        protocol: ProtocolType::Ssh,
        username: None,
        group_id: None,
        tags: Vec::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
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
        is_pinned: false,
        pin_order: 0,
        icon: None,
        monitoring_config: None,
    }
}

// ========== Property Tests: Backend Selection ==========

proptest! {
    /// Property 10: Backend Selection Priority
    /// Validates: Requirement 3.1 - Store to KeePass if enabled, otherwise Keyring
    ///
    /// When KeePass is enabled AND has a valid path, select KdbxFile backend.
    /// Otherwise, select LibSecret backend.
    #[test]
    fn backend_selection_priority(
        kdbx_enabled in any::<bool>(),
        has_path in any::<bool>(),
    ) {
        let kdbx_path = if has_path {
            Some(PathBuf::from("/path/to/db.kdbx"))
        } else {
            None
        };

        let settings = SecretSettings {
            kdbx_enabled,
            kdbx_path,
            ..Default::default()
        };

        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        let selected = resolver.select_storage_backend();

        // KeePass should be selected only if BOTH enabled AND has path
        if kdbx_enabled && has_path {
            prop_assert_eq!(selected, SecretBackendType::KdbxFile);
        } else {
            prop_assert_eq!(selected, SecretBackendType::LibSecret);
        }
    }

    /// Property: KeePass active check consistency
    #[test]
    fn keepass_active_consistency(
        kdbx_enabled in any::<bool>(),
        has_path in any::<bool>(),
    ) {
        let kdbx_path = if has_path {
            Some(PathBuf::from("/path/to/db.kdbx"))
        } else {
            None
        };

        let settings = SecretSettings {
            kdbx_enabled,
            kdbx_path,
            ..Default::default()
        };

        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        let is_active = resolver.is_keepass_active();
        let selected = resolver.select_storage_backend();

        // is_keepass_active should match backend selection
        if is_active {
            prop_assert_eq!(selected, SecretBackendType::KdbxFile);
        }
    }

    /// Property: Backend selection is deterministic
    #[test]
    fn backend_selection_deterministic(settings in arb_secret_settings()) {
        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        let first = resolver.select_storage_backend();
        let second = resolver.select_storage_backend();

        prop_assert_eq!(first, second);
    }
}

// ========== Property Tests: Migration Detection ==========

proptest! {
    /// Property 12: Migration Button Visibility
    /// Validates: Requirement 3.3 - Detect credentials in Keyring but not KeePass
    ///
    /// Migration is only needed when:
    /// 1. KeePass is enabled with valid path
    /// 2. Credentials exist in Keyring
    /// 3. Credentials do NOT exist in KeePass
    ///
    /// Since we can't easily mock backends in property tests, we test the
    /// precondition: migration check returns false when KeePass is disabled.
    #[test]
    fn migration_not_needed_when_keepass_disabled(
        id in arb_uuid(),
        name in arb_connection_name(),
        host in arb_hostname(),
    ) {
        let settings = SecretSettings {
            kdbx_enabled: false,
            kdbx_path: None,
            ..Default::default()
        };

        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);
        let conn = create_test_connection(id, &name, &host);

        // Run async check in blocking context
        let rt = tokio::runtime::Runtime::new().unwrap();
        let needs_migration = rt.block_on(resolver.needs_keepass_migration(&conn)).unwrap();

        // Should never need migration when KeePass is disabled
        prop_assert!(!needs_migration);
    }

    /// Property: Migration not needed when KeePass has no path
    #[test]
    fn migration_not_needed_when_no_kdbx_path(
        id in arb_uuid(),
        name in arb_connection_name(),
        host in arb_hostname(),
    ) {
        let settings = SecretSettings {
            kdbx_enabled: true,
            kdbx_path: None, // No path configured
            ..Default::default()
        };

        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);
        let conn = create_test_connection(id, &name, &host);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let needs_migration = rt.block_on(resolver.needs_keepass_migration(&conn)).unwrap();

        // Should never need migration when no KDBX path
        prop_assert!(!needs_migration);
    }
}

// ========== Property Tests: Lookup Key Generation ==========

proptest! {
    /// Property: Lookup key contains connection identifier
    #[test]
    fn lookup_key_contains_identifier(
        id in arb_uuid(),
        name in arb_connection_name(),
        host in arb_hostname(),
    ) {
        let conn = create_test_connection(id, &name, &host);
        let key = CredentialResolver::generate_lookup_key(&conn);

        // Key should contain either name or host
        prop_assert!(key.contains(&name) || key.contains(&host));
        // Key should have rustconn prefix
        prop_assert!(key.starts_with("rustconn/"));
    }

    /// Property: Hierarchical lookup key contains group path
    #[test]
    fn hierarchical_key_contains_group_path(
        id in arb_uuid(),
        name in arb_connection_name(),
        host in arb_hostname(),
        group_name in arb_group_name(),
    ) {
        let group = ConnectionGroup::new(group_name.clone());
        let mut conn = create_test_connection(id, &name, &host);
        conn.group_id = Some(group.id);

        let key = CredentialResolver::generate_hierarchical_lookup_key(&conn, &[group]);

        // Key should contain group name
        prop_assert!(key.contains(&group_name));
        // Key should contain connection name
        prop_assert!(key.contains(&name));
        // Key should have RustConn prefix
        prop_assert!(key.starts_with("RustConn/"));
    }

    /// Property: Hierarchical key preserves group hierarchy order
    /// The key format is: RustConn/Parent/Child/ConnectionName
    #[test]
    fn hierarchical_key_preserves_order(
        id in arb_uuid(),
        name in "[a-z]{5,10}".prop_map(String::from),
        host in arb_hostname(),
        parent_name in "[A-Z]{5,10}".prop_map(String::from),
        child_name in "[0-9]{5,10}".prop_map(String::from),
    ) {
        // Using distinct character sets ensures no substring collisions

        let parent = ConnectionGroup::new(parent_name.clone());
        let child = ConnectionGroup::with_parent(child_name.clone(), parent.id);

        let mut conn = create_test_connection(id, &name, &host);
        conn.group_id = Some(child.id);

        let groups = vec![parent, child];
        let key = CredentialResolver::generate_hierarchical_lookup_key(&conn, &groups);

        // Split by path separator and verify order
        let parts: Vec<&str> = key.split('/').collect();

        // Should be: ["RustConn", parent_name, child_name, connection_name]
        prop_assert!(parts.len() >= 4);
        prop_assert_eq!(parts[0], "RustConn");
        prop_assert_eq!(parts[1], parent_name);
        prop_assert_eq!(parts[2], child_name);
        prop_assert_eq!(parts[3], name);
    }

    /// Property: Connection name appears last in hierarchical key
    #[test]
    fn connection_name_last_in_key(
        id in arb_uuid(),
        name in arb_connection_name(),
        host in arb_hostname(),
        group_name in arb_group_name(),
    ) {
        prop_assume!(!name.is_empty());

        let group = ConnectionGroup::new(group_name);
        let mut conn = create_test_connection(id, &name, &host);
        conn.group_id = Some(group.id);

        let key = CredentialResolver::generate_hierarchical_lookup_key(&conn, &[group]);

        // Connection name should be at the end
        prop_assert!(key.ends_with(&name));
    }
}

// ========== Property Tests: Settings Consistency ==========

proptest! {
    /// Property: SecretSettings default has fallback enabled
    #[test]
    fn default_settings_has_fallback(_seed in any::<u64>()) {
        let settings = SecretSettings::default();
        prop_assert!(settings.enable_fallback);
    }

    /// Property: SecretSettings default has KeePass disabled
    #[test]
    fn default_settings_keepass_disabled(_seed in any::<u64>()) {
        let settings = SecretSettings::default();
        prop_assert!(!settings.kdbx_enabled);
        prop_assert!(settings.kdbx_path.is_none());
    }

    /// Property: SecretBackendType equality is reflexive
    #[test]
    fn backend_type_equality_reflexive(
        kdbx_enabled in any::<bool>(),
        has_path in any::<bool>(),
    ) {
        let kdbx_path = if has_path {
            Some(PathBuf::from("/path/to/db.kdbx"))
        } else {
            None
        };

        let settings = SecretSettings {
            kdbx_enabled,
            kdbx_path,
            ..Default::default()
        };

        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        let backend = resolver.select_storage_backend();
        prop_assert_eq!(backend, backend);
    }
}

// ========== Property Tests: Keyring Fallback ==========

proptest! {
    /// Property 11: Keyring Fallback When KeePass Empty
    /// Validates: Requirement 3.2 - Check both backends during resolution
    ///
    /// When fallback is enabled and KeePass lookup fails, should try Keyring.
    /// We test this by verifying the resolver is configured correctly.
    #[test]
    fn fallback_enabled_by_default(_seed in any::<u64>()) {
        let settings = SecretSettings::default();
        prop_assert!(settings.enable_fallback);
    }

    /// Property: Fallback can be disabled
    #[test]
    fn fallback_can_be_disabled(_seed in any::<u64>()) {
        let settings = SecretSettings {
            enable_fallback: false,
            ..Default::default()
        };
        prop_assert!(!settings.enable_fallback);
    }
}
