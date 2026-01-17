//! Property-based tests for configuration round-trip through ConfigManager
//!
//! **Feature: rustconn, Property 6: Connection Serialization Round-Trip** (extended to full config)
//! **Validates: Requirements 10.5, 10.6**
//!
//! **Feature: rustconn-enhancements, Property 4: Settings Persistence Round-Trip**
//! **Validates: Requirements 6.1, 6.2**

use proptest::prelude::*;
use rustconn_core::{
    config::AppSettings, config::ColorScheme, config::ConfigManager, config::LoggingSettings,
    config::SecretBackendType, config::SecretSettings, config::SessionRestoreSettings,
    config::TerminalSettings, config::UiSettings, Connection, ConnectionGroup, HistorySettings,
    ProtocolConfig, RdpConfig, RdpGateway, Resolution, Snippet, SnippetVariable, SshAuthMethod,
    SshConfig, SshKeySource, VncConfig,
};
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

// ========== Generators ==========

// Strategy for generating valid connection names
fn arb_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,31}".prop_map(|s| s)
}

// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*".prop_map(|s| s)
}

// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9_]{0,15}".prop_map(Some),]
}

// Strategy for generating tags
fn arb_tags() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{1,10}", 0..5)
}

// Strategy for SSH auth method
fn arb_ssh_auth_method() -> impl Strategy<Value = SshAuthMethod> {
    prop_oneof![
        Just(SshAuthMethod::Password),
        Just(SshAuthMethod::PublicKey),
        Just(SshAuthMethod::KeyboardInteractive),
        Just(SshAuthMethod::Agent),
    ]
}

// Strategy for optional PathBuf
fn arb_optional_path() -> impl Strategy<Value = Option<PathBuf>> {
    prop_oneof![
        Just(None),
        "[a-z]{1,10}(/[a-z]{1,10}){0,3}".prop_map(|s| Some(PathBuf::from(s))),
    ]
}

// Strategy for optional string
fn arb_optional_string() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9_-]{1,20}".prop_map(Some),]
}

// Strategy for custom SSH options
fn arb_custom_options() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map("[A-Za-z]{1,20}", "[a-zA-Z0-9]{1,10}", 0..3)
}

// Strategy for SSH config
fn arb_ssh_config() -> impl Strategy<Value = SshConfig> {
    (
        arb_ssh_auth_method(),
        arb_optional_path(),
        arb_optional_string(),
        any::<bool>(),
        arb_custom_options(),
        arb_optional_string(),
    )
        .prop_map(
            |(
                auth_method,
                key_path,
                proxy_jump,
                use_control_master,
                custom_options,
                startup_command,
            )| {
                SshConfig {
                    auth_method,
                    key_path,
                    key_source: SshKeySource::Default,
                    agent_key_fingerprint: None,
                    identities_only: false,
                    proxy_jump,
                    use_control_master,
                    agent_forwarding: false,
                    custom_options,
                    startup_command,
                    jump_host_id: None,
                }
            },
        )
}

// Strategy for optional resolution
fn arb_optional_resolution() -> impl Strategy<Value = Option<Resolution>> {
    prop_oneof![
        Just(None),
        (640u32..4096u32, 480u32..2160u32).prop_map(|(w, h)| Some(Resolution::new(w, h))),
    ]
}

// Strategy for optional color depth
fn arb_optional_color_depth() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![
        Just(None),
        prop_oneof![Just(8u8), Just(15u8), Just(16u8), Just(24u8), Just(32u8)].prop_map(Some),
    ]
}

// Strategy for optional RDP gateway
fn arb_optional_gateway() -> impl Strategy<Value = Option<RdpGateway>> {
    prop_oneof![
        Just(None),
        (arb_host(), 1u16..65535u16, arb_optional_string()).prop_map(
            |(hostname, port, username)| {
                Some(RdpGateway {
                    hostname,
                    port,
                    username,
                })
            }
        ),
    ]
}

// Strategy for custom args
fn arb_custom_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-zA-Z0-9_=-]{1,20}", 0..3)
}

// Strategy for RDP config
fn arb_rdp_config() -> impl Strategy<Value = RdpConfig> {
    (
        arb_optional_resolution(),
        arb_optional_color_depth(),
        any::<bool>(),
        arb_optional_gateway(),
        arb_custom_args(),
    )
        .prop_map(
            |(resolution, color_depth, audio_redirect, gateway, custom_args)| RdpConfig {
                resolution,
                color_depth,
                audio_redirect,
                gateway,
                shared_folders: Vec::new(),
                custom_args,
                client_mode: Default::default(),
                performance_mode: Default::default(),
            },
        )
}

// Strategy for optional encoding
fn arb_optional_encoding() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        prop_oneof![
            Just("tight".to_string()),
            Just("zrle".to_string()),
            Just("hextile".to_string()),
        ]
        .prop_map(Some),
    ]
}

// Strategy for optional compression/quality (0-9)
fn arb_optional_level() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![Just(None), (0u8..=9u8).prop_map(Some),]
}

// Strategy for VNC config
fn arb_vnc_config() -> impl Strategy<Value = VncConfig> {
    (
        arb_optional_encoding(),
        arb_optional_level(),
        arb_optional_level(),
        arb_custom_args(),
    )
        .prop_map(|(encoding, compression, quality, custom_args)| VncConfig {
            client_mode: Default::default(),
            performance_mode: Default::default(),
            encoding,
            compression,
            quality,
            view_only: false,
            scaling: true,
            clipboard_enabled: true,
            custom_args,
        })
}

// Strategy for protocol config
fn arb_protocol_config() -> impl Strategy<Value = ProtocolConfig> {
    prop_oneof![
        arb_ssh_config().prop_map(ProtocolConfig::Ssh),
        arb_rdp_config().prop_map(ProtocolConfig::Rdp),
        arb_vnc_config().prop_map(ProtocolConfig::Vnc),
    ]
}

// Strategy for generating a complete Connection
fn arb_connection() -> impl Strategy<Value = Connection> {
    (
        arb_name(),
        arb_host(),
        arb_port(),
        arb_protocol_config(),
        arb_username(),
        arb_tags(),
    )
        .prop_map(|(name, host, port, protocol_config, username, tags)| {
            let mut conn = Connection::new(name, host, port, protocol_config);
            if let Some(u) = username {
                conn = conn.with_username(u);
            }
            if !tags.is_empty() {
                conn = conn.with_tags(tags);
            }
            conn
        })
}

// Strategy for generating a ConnectionGroup
fn arb_group() -> impl Strategy<Value = ConnectionGroup> {
    arb_name().prop_map(ConnectionGroup::new)
}

// Strategy for generating a SnippetVariable
fn arb_snippet_variable() -> impl Strategy<Value = SnippetVariable> {
    (
        "[a-z][a-z0-9_]{0,15}".prop_map(|s| s),
        arb_optional_string(),
        arb_optional_string(),
    )
        .prop_map(|(name, description, default_value)| {
            let mut var = SnippetVariable::new(name);
            if let Some(d) = description {
                var = var.with_description(d);
            }
            if let Some(v) = default_value {
                var = var.with_default(v);
            }
            var
        })
}

// Strategy for generating a Snippet
fn arb_snippet() -> impl Strategy<Value = Snippet> {
    (
        arb_name(),
        "[a-zA-Z0-9 _-]{1,50}".prop_map(|s| s), // command
        arb_optional_string(),                  // description
        arb_optional_string(),                  // category
        arb_tags(),
        prop::collection::vec(arb_snippet_variable(), 0..3),
    )
        .prop_map(|(name, command, description, category, tags, variables)| {
            let mut snippet = Snippet::new(name, command);
            if let Some(d) = description {
                snippet = snippet.with_description(d);
            }
            if let Some(c) = category {
                snippet = snippet.with_category(c);
            }
            if !tags.is_empty() {
                snippet = snippet.with_tags(tags);
            }
            if !variables.is_empty() {
                snippet = snippet.with_variables(variables);
            }
            snippet
        })
}

// Strategy for generating AppSettings
fn arb_settings() -> impl Strategy<Value = AppSettings> {
    (
        "[A-Za-z ]{1,20}".prop_map(|s| s), // font_family
        8u32..32u32,                       // font_size
        1000u32..100000u32,                // scrollback_lines
        any::<bool>(),                     // logging enabled
        1u32..365u32,                      // retention_days
        any::<bool>(),                     // enable_fallback
        any::<bool>(),                     // remember_window_geometry
    )
        .prop_map(
            |(
                font_family,
                font_size,
                scrollback_lines,
                logging_enabled,
                retention_days,
                enable_fallback,
                remember_window_geometry,
            )| {
                let mut settings = AppSettings::default();
                settings.terminal.font_family = font_family;
                settings.terminal.font_size = font_size;
                settings.terminal.scrollback_lines = scrollback_lines;
                settings.logging.enabled = logging_enabled;
                settings.logging.retention_days = retention_days;
                settings.secrets.enable_fallback = enable_fallback;
                settings.ui.remember_window_geometry = remember_window_geometry;
                settings
            },
        )
}

// Strategy for generating SecretBackendType
fn arb_secret_backend_type() -> impl Strategy<Value = SecretBackendType> {
    prop_oneof![
        Just(SecretBackendType::KeePassXc),
        Just(SecretBackendType::KdbxFile),
        Just(SecretBackendType::LibSecret),
    ]
}

// Strategy for generating optional KDBX path (must end with .kdbx)
fn arb_optional_kdbx_path() -> impl Strategy<Value = Option<PathBuf>> {
    prop_oneof![
        Just(None),
        "[a-z]{1,10}(/[a-z]{1,10}){0,2}/[a-z]{1,10}\\.kdbx".prop_map(|s| Some(PathBuf::from(s))),
    ]
}

// Strategy for generating SecretSettings with all fields
fn arb_secret_settings() -> impl Strategy<Value = SecretSettings> {
    (
        arb_secret_backend_type(),
        any::<bool>(), // enable_fallback
        arb_optional_kdbx_path(),
        any::<bool>(), // kdbx_enabled
    )
        .prop_map(
            |(preferred_backend, enable_fallback, kdbx_path, kdbx_enabled)| SecretSettings {
                preferred_backend,
                enable_fallback,
                kdbx_path,
                kdbx_enabled,
                kdbx_password: None,           // Password is never serialized
                kdbx_password_encrypted: None, // Encrypted password for persistence
                kdbx_key_file: None,           // Key file path
                kdbx_use_key_file: false,      // Use key file instead of password
                kdbx_use_password: true,       // Use password for authentication
                bitwarden_password: None,      // Bitwarden password is never serialized
                bitwarden_password_encrypted: None, // Encrypted Bitwarden password
            },
        )
}

// Strategy for generating optional window dimensions
fn arb_optional_dimension() -> impl Strategy<Value = Option<i32>> {
    prop_oneof![Just(None), (100i32..4000i32).prop_map(Some),]
}

// Strategy for generating complete AppSettings with all fields
// Used for Property 4: Settings Persistence Round-Trip
fn arb_full_settings() -> impl Strategy<Value = AppSettings> {
    (
        "[A-Za-z ]{1,20}".prop_map(|s| s), // font_family
        8u32..32u32,                       // font_size
        1000u32..100000u32,                // scrollback_lines
        any::<bool>(),                     // logging enabled
        "[a-z]{1,10}(/[a-z]{1,10}){0,2}".prop_map(PathBuf::from), // log_directory
        1u32..365u32,                      // retention_days
        arb_secret_backend_type(),         // preferred_backend
        any::<bool>(),                     // enable_fallback
        any::<bool>(),                     // remember_window_geometry
        arb_optional_dimension(),          // window_width
        arb_optional_dimension(),          // window_height
        arb_optional_dimension(),          // sidebar_width
    )
        .prop_map(
            |(
                font_family,
                font_size,
                scrollback_lines,
                logging_enabled,
                log_directory,
                retention_days,
                preferred_backend,
                enable_fallback,
                remember_window_geometry,
                window_width,
                window_height,
                sidebar_width,
            )| {
                AppSettings {
                    terminal: TerminalSettings {
                        font_family,
                        font_size,
                        scrollback_lines,
                        ..TerminalSettings::default()
                    },
                    logging: LoggingSettings {
                        enabled: logging_enabled,
                        log_directory,
                        retention_days,
                        log_activity: true,
                        log_input: false,
                        log_output: false,
                    },
                    secrets: SecretSettings {
                        preferred_backend,
                        enable_fallback,
                        kdbx_path: None,
                        kdbx_enabled: false,
                        kdbx_password: None,
                        kdbx_password_encrypted: None,
                        kdbx_key_file: None,
                        kdbx_use_key_file: false,
                        kdbx_use_password: true,
                        bitwarden_password: None,
                        bitwarden_password_encrypted: None,
                    },
                    ui: UiSettings {
                        color_scheme: ColorScheme::default(),
                        remember_window_geometry,
                        window_width,
                        window_height,
                        sidebar_width,
                        enable_tray_icon: true,
                        minimize_to_tray: false,
                        expanded_groups: std::collections::HashSet::new(),
                        session_restore: SessionRestoreSettings::default(),
                        search_history: Vec::new(),
                    },
                    connection: rustconn_core::ConnectionSettings::default(),
                    global_variables: Vec::new(),
                    history: HistorySettings::default(),
                }
            },
        )
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 6: Connection Serialization Round-Trip** (extended to full config)
    /// **Validates: Requirements 10.5, 10.6**
    ///
    /// For any list of valid Connection objects, saving through ConfigManager and then loading
    /// should produce equivalent Connection objects with all fields preserved.
    #[test]
    fn connections_config_round_trip(connections in prop::collection::vec(arb_connection(), 0..10)) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Save connections
        manager.save_connections(&connections).expect("Failed to save connections");

        // Load connections back
        let loaded = manager.load_connections().expect("Failed to load connections");

        // Verify count matches
        prop_assert_eq!(connections.len(), loaded.len(), "Connection count should match");

        // Verify each connection (order should be preserved)
        for (original, loaded_conn) in connections.iter().zip(loaded.iter()) {
            prop_assert_eq!(original.id, loaded_conn.id, "ID should be preserved");
            prop_assert_eq!(&original.name, &loaded_conn.name, "Name should be preserved");
            prop_assert_eq!(original.protocol, loaded_conn.protocol, "Protocol should be preserved");
            prop_assert_eq!(&original.host, &loaded_conn.host, "Host should be preserved");
            prop_assert_eq!(original.port, loaded_conn.port, "Port should be preserved");
            prop_assert_eq!(&original.username, &loaded_conn.username, "Username should be preserved");
            prop_assert_eq!(original.group_id, loaded_conn.group_id, "Group ID should be preserved");
            prop_assert_eq!(&original.tags, &loaded_conn.tags, "Tags should be preserved");
            prop_assert_eq!(&original.protocol_config, &loaded_conn.protocol_config, "Protocol config should be preserved");

            // Timestamps may have nanosecond precision loss in TOML
            prop_assert_eq!(
                original.created_at.timestamp(),
                loaded_conn.created_at.timestamp(),
                "Created timestamp should be preserved (second precision)"
            );
            prop_assert_eq!(
                original.updated_at.timestamp(),
                loaded_conn.updated_at.timestamp(),
                "Updated timestamp should be preserved (second precision)"
            );
        }
    }

    /// **Feature: rustconn, Property 6: Connection Serialization Round-Trip** (groups)
    /// **Validates: Requirements 10.5, 10.6**
    ///
    /// For any list of valid ConnectionGroup objects, saving through ConfigManager and then loading
    /// should produce equivalent ConnectionGroup objects with all fields preserved.
    #[test]
    fn groups_config_round_trip(groups in prop::collection::vec(arb_group(), 0..10)) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Save groups
        manager.save_groups(&groups).expect("Failed to save groups");

        // Load groups back
        let loaded = manager.load_groups().expect("Failed to load groups");

        // Verify count matches
        prop_assert_eq!(groups.len(), loaded.len(), "Group count should match");

        // Verify each group
        for (original, loaded_group) in groups.iter().zip(loaded.iter()) {
            prop_assert_eq!(original.id, loaded_group.id, "ID should be preserved");
            prop_assert_eq!(&original.name, &loaded_group.name, "Name should be preserved");
            prop_assert_eq!(original.parent_id, loaded_group.parent_id, "Parent ID should be preserved");
            prop_assert_eq!(original.expanded, loaded_group.expanded, "Expanded state should be preserved");

            // Timestamps may have nanosecond precision loss in TOML
            prop_assert_eq!(
                original.created_at.timestamp(),
                loaded_group.created_at.timestamp(),
                "Created timestamp should be preserved (second precision)"
            );
        }
    }

    /// **Feature: rustconn, Property 6: Connection Serialization Round-Trip** (snippets)
    /// **Validates: Requirements 10.5, 10.6**
    ///
    /// For any list of valid Snippet objects, saving through ConfigManager and then loading
    /// should produce equivalent Snippet objects with all fields preserved.
    #[test]
    fn snippets_config_round_trip(snippets in prop::collection::vec(arb_snippet(), 0..10)) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Save snippets
        manager.save_snippets(&snippets).expect("Failed to save snippets");

        // Load snippets back
        let loaded = manager.load_snippets().expect("Failed to load snippets");

        // Verify count matches
        prop_assert_eq!(snippets.len(), loaded.len(), "Snippet count should match");

        // Verify each snippet
        for (original, loaded_snippet) in snippets.iter().zip(loaded.iter()) {
            prop_assert_eq!(original.id, loaded_snippet.id, "ID should be preserved");
            prop_assert_eq!(&original.name, &loaded_snippet.name, "Name should be preserved");
            prop_assert_eq!(&original.description, &loaded_snippet.description, "Description should be preserved");
            prop_assert_eq!(&original.command, &loaded_snippet.command, "Command should be preserved");
            prop_assert_eq!(&original.category, &loaded_snippet.category, "Category should be preserved");
            prop_assert_eq!(&original.tags, &loaded_snippet.tags, "Tags should be preserved");
            prop_assert_eq!(&original.variables, &loaded_snippet.variables, "Variables should be preserved");
        }
    }

    /// **Feature: rustconn, Property 6: Connection Serialization Round-Trip** (settings)
    /// **Validates: Requirements 10.5, 10.6**
    ///
    /// For any valid AppSettings object, saving through ConfigManager and then loading
    /// should produce an equivalent AppSettings object with all fields preserved.
    #[test]
    fn settings_config_round_trip(settings in arb_settings()) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Save settings
        manager.save_settings(&settings).expect("Failed to save settings");

        // Load settings back
        let loaded = manager.load_settings().expect("Failed to load settings");

        // Verify all fields are preserved
        prop_assert_eq!(settings.terminal.font_family, loaded.terminal.font_family, "Font family should be preserved");
        prop_assert_eq!(settings.terminal.font_size, loaded.terminal.font_size, "Font size should be preserved");
        prop_assert_eq!(settings.terminal.scrollback_lines, loaded.terminal.scrollback_lines, "Scrollback lines should be preserved");
        prop_assert_eq!(settings.logging.enabled, loaded.logging.enabled, "Logging enabled should be preserved");
        prop_assert_eq!(settings.logging.retention_days, loaded.logging.retention_days, "Retention days should be preserved");
        prop_assert_eq!(settings.secrets.enable_fallback, loaded.secrets.enable_fallback, "Enable fallback should be preserved");
        prop_assert_eq!(settings.ui.remember_window_geometry, loaded.ui.remember_window_geometry, "Remember window geometry should be preserved");
    }

    /// **Feature: rustconn-enhancements, Property 4: Settings Persistence Round-Trip**
    /// **Validates: Requirements 6.1, 6.2**
    ///
    /// For any settings configuration, saving via build_settings() and then loading via
    /// set_settings() should produce identical settings values. This tests the complete
    /// settings dialog round-trip through persistence.
    #[test]
    fn settings_dialog_round_trip(settings in arb_full_settings()) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Simulate dialog's build_settings() -> save -> load -> set_settings() cycle
        // Save settings (simulates what happens after build_settings())
        manager.save_settings(&settings).expect("Failed to save settings");

        // Load settings back (simulates what set_settings() would receive)
        let loaded = manager.load_settings().expect("Failed to load settings");

        // Verify complete equality - all fields must match exactly
        // Terminal settings
        prop_assert_eq!(
            &settings.terminal.font_family,
            &loaded.terminal.font_family,
            "Terminal font family should be preserved"
        );
        prop_assert_eq!(
            settings.terminal.font_size,
            loaded.terminal.font_size,
            "Terminal font size should be preserved"
        );
        prop_assert_eq!(
            settings.terminal.scrollback_lines,
            loaded.terminal.scrollback_lines,
            "Terminal scrollback lines should be preserved"
        );

        // Logging settings
        prop_assert_eq!(
            settings.logging.enabled,
            loaded.logging.enabled,
            "Logging enabled should be preserved"
        );
        prop_assert_eq!(
            &settings.logging.log_directory,
            &loaded.logging.log_directory,
            "Log directory should be preserved"
        );
        prop_assert_eq!(
            settings.logging.retention_days,
            loaded.logging.retention_days,
            "Retention days should be preserved"
        );

        // Secret settings
        prop_assert_eq!(
            settings.secrets.preferred_backend,
            loaded.secrets.preferred_backend,
            "Preferred secret backend should be preserved"
        );
        prop_assert_eq!(
            settings.secrets.enable_fallback,
            loaded.secrets.enable_fallback,
            "Enable fallback should be preserved"
        );

        // UI settings
        prop_assert_eq!(
            settings.ui.remember_window_geometry,
            loaded.ui.remember_window_geometry,
            "Remember window geometry should be preserved"
        );
        prop_assert_eq!(
            settings.ui.window_width,
            loaded.ui.window_width,
            "Window width should be preserved"
        );
        prop_assert_eq!(
            settings.ui.window_height,
            loaded.ui.window_height,
            "Window height should be preserved"
        );
        prop_assert_eq!(
            settings.ui.sidebar_width,
            loaded.ui.sidebar_width,
            "Sidebar width should be preserved"
        );
    }

    /// **Feature: keepass-integration, Property 3: Settings Serialization Round-Trip**
    /// **Validates: Requirements 5.1, 5.2, 5.3**
    ///
    /// For any valid SecretSettings, serializing to TOML and deserializing back SHALL produce
    /// equivalent settings, except the kdbx_password field which SHALL always be None after
    /// deserialization (security requirement - passwords are never persisted to disk).
    #[test]
    fn secret_settings_serialization_round_trip(settings in arb_secret_settings()) {
        // Serialize to TOML
        let toml_str = toml::to_string(&settings)
            .expect("SecretSettings should serialize to TOML");

        // Deserialize back from TOML
        let deserialized: SecretSettings = toml::from_str(&toml_str)
            .expect("TOML should deserialize back to SecretSettings");

        // Verify all serializable fields are preserved
        prop_assert_eq!(
            settings.preferred_backend,
            deserialized.preferred_backend,
            "Preferred backend should be preserved"
        );
        prop_assert_eq!(
            settings.enable_fallback,
            deserialized.enable_fallback,
            "Enable fallback should be preserved"
        );
        prop_assert_eq!(
            &settings.kdbx_path,
            &deserialized.kdbx_path,
            "KDBX path should be preserved"
        );
        prop_assert_eq!(
            settings.kdbx_enabled,
            deserialized.kdbx_enabled,
            "KDBX enabled should be preserved"
        );

        // CRITICAL: Verify password is NOT serialized (security requirement)
        // The kdbx_password field should always be None after deserialization
        prop_assert!(
            deserialized.kdbx_password.is_none(),
            "KDBX password must NOT be serialized - should always be None after deserialization"
        );

        // Verify the TOML string does not contain actual password values
        // Note: kdbx_use_password is a boolean flag, not a password value
        prop_assert!(
            !toml_str.contains("kdbx_password ="),
            "Serialized TOML must not contain kdbx_password field (actual password)"
        );
        prop_assert!(
            !toml_str.contains("kdbx_password_encrypted ="),
            "Serialized TOML must not contain kdbx_password_encrypted field"
        );
    }
}
