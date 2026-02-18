//! Property-based tests for native export/import functionality
//!
//! Tests correctness properties for the RustConn native export format (.rcn).

use proptest::prelude::*;
use rustconn_core::cluster::Cluster;
use rustconn_core::export::{NativeExport, NativeImportError, NATIVE_FORMAT_VERSION};
use rustconn_core::models::{Connection, ConnectionGroup, ConnectionTemplate};
use rustconn_core::variables::Variable;

/// Generates a valid hostname
fn arb_hostname() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9-]{0,20}(\\.[a-z][a-z0-9-]{0,10})*")
        .unwrap()
        .prop_filter("hostname must not be empty", |s| !s.is_empty())
}

/// Generates a valid connection name
fn arb_connection_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z][a-zA-Z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("name must not be empty", |s| !s.is_empty())
}

/// Generates a valid port number
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..65535
}

/// Generates a valid variable name
fn arb_variable_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z_][a-zA-Z0-9_]{0,20}")
        .unwrap()
        .prop_filter("variable name must not be empty", |s| !s.is_empty())
}

/// Generates a valid variable value
fn arb_variable_value() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9_./:-]{1,50}")
        .unwrap()
        .prop_filter("variable value must not be empty", |s| !s.is_empty())
}

/// Strategy for generating test SSH connections
fn arb_ssh_connection() -> impl Strategy<Value = Connection> {
    (arb_connection_name(), arb_hostname(), arb_port())
        .prop_map(|(name, host, port)| Connection::new_ssh(name, host, port))
}

/// Strategy for generating test RDP connections
fn arb_rdp_connection() -> impl Strategy<Value = Connection> {
    (arb_connection_name(), arb_hostname(), arb_port())
        .prop_map(|(name, host, port)| Connection::new_rdp(name, host, port))
}

/// Strategy for generating test VNC connections
fn arb_vnc_connection() -> impl Strategy<Value = Connection> {
    (arb_connection_name(), arb_hostname(), arb_port())
        .prop_map(|(name, host, port)| Connection::new_vnc(name, host, port))
}

/// Strategy for generating any type of connection
fn arb_connection() -> impl Strategy<Value = Connection> {
    prop_oneof![
        arb_ssh_connection(),
        arb_rdp_connection(),
        arb_vnc_connection(),
    ]
}

/// Strategy for generating connection groups
fn arb_group() -> impl Strategy<Value = ConnectionGroup> {
    arb_connection_name().prop_map(ConnectionGroup::new)
}

/// Strategy for generating connection templates
fn arb_template() -> impl Strategy<Value = ConnectionTemplate> {
    arb_connection_name().prop_map(ConnectionTemplate::new_ssh)
}

/// Strategy for generating clusters
fn arb_cluster() -> impl Strategy<Value = Cluster> {
    arb_connection_name().prop_map(Cluster::new)
}

/// Strategy for generating variables
fn arb_variable() -> impl Strategy<Value = Variable> {
    (arb_variable_name(), arb_variable_value()).prop_map(|(name, value)| Variable::new(name, value))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 12: Native Export Contains All Data Types**
    /// **Validates: Requirements 13.2**
    ///
    /// For any NativeExport with connections, groups, templates, clusters, and variables,
    /// the JSON output should contain all data.
    #[test]
    fn prop_native_export_contains_all_data_types(
        connections in prop::collection::vec(arb_connection(), 0..5),
        groups in prop::collection::vec(arb_group(), 0..3),
        templates in prop::collection::vec(arb_template(), 0..3),
        clusters in prop::collection::vec(arb_cluster(), 0..3),
        variables in prop::collection::vec(arb_variable(), 0..5)
    ) {
        let export = NativeExport::with_data(
            connections.clone(),
            groups.clone(),
            templates.clone(),
            clusters.clone(),
            variables.clone(),
            Vec::new(),
        );

        let json = export.to_json().expect("Serialization should succeed");

        // Property: JSON should contain all data types
        prop_assert!(json.contains("\"connections\""), "JSON should contain connections field");
        prop_assert!(json.contains("\"groups\""), "JSON should contain groups field");
        prop_assert!(json.contains("\"templates\""), "JSON should contain templates field");
        prop_assert!(json.contains("\"clusters\""), "JSON should contain clusters field");
        prop_assert!(json.contains("\"variables\""), "JSON should contain variables field");

        // Property: Counts should match
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON should be valid");

        prop_assert_eq!(
            parsed["connections"].as_array().map(|a| a.len()).unwrap_or(0),
            connections.len(),
            "Connections count mismatch"
        );
        prop_assert_eq!(
            parsed["groups"].as_array().map(|a| a.len()).unwrap_or(0),
            groups.len(),
            "Groups count mismatch"
        );
        prop_assert_eq!(
            parsed["templates"].as_array().map(|a| a.len()).unwrap_or(0),
            templates.len(),
            "Templates count mismatch"
        );
        prop_assert_eq!(
            parsed["clusters"].as_array().map(|a| a.len()).unwrap_or(0),
            clusters.len(),
            "Clusters count mismatch"
        );
        prop_assert_eq!(
            parsed["variables"].as_array().map(|a| a.len()).unwrap_or(0),
            variables.len(),
            "Variables count mismatch"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 13: Native Import Restores All Data**
    /// **Validates: Requirements 13.3**
    ///
    /// For any valid NativeExport JSON, importing should restore all connections,
    /// groups, templates, clusters, and variables.
    #[test]
    fn prop_native_import_restores_all_data(
        connections in prop::collection::vec(arb_connection(), 0..5),
        groups in prop::collection::vec(arb_group(), 0..3),
        templates in prop::collection::vec(arb_template(), 0..3),
        clusters in prop::collection::vec(arb_cluster(), 0..3),
        variables in prop::collection::vec(arb_variable(), 0..5)
    ) {
        let export = NativeExport::with_data(
            connections.clone(),
            groups.clone(),
            templates.clone(),
            clusters.clone(),
            variables.clone(),
            Vec::new(),
        );

        let json = export.to_json().expect("Serialization should succeed");
        let imported = NativeExport::from_json(&json).expect("Import should succeed");

        // Property: All data should be restored
        prop_assert_eq!(
            imported.connections.len(),
            connections.len(),
            "Connections count mismatch after import"
        );
        prop_assert_eq!(
            imported.groups.len(),
            groups.len(),
            "Groups count mismatch after import"
        );
        prop_assert_eq!(
            imported.templates.len(),
            templates.len(),
            "Templates count mismatch after import"
        );
        prop_assert_eq!(
            imported.clusters.len(),
            clusters.len(),
            "Clusters count mismatch after import"
        );
        prop_assert_eq!(
            imported.variables.len(),
            variables.len(),
            "Variables count mismatch after import"
        );

        // Property: Connection IDs should be preserved
        for original in &connections {
            let found = imported.connections.iter().any(|c| c.id == original.id);
            prop_assert!(found, "Connection with ID {} not found after import", original.id);
        }

        // Property: Group IDs should be preserved
        for original in &groups {
            let found = imported.groups.iter().any(|g| g.id == original.id);
            prop_assert!(found, "Group with ID {} not found after import", original.id);
        }

        // Property: Template IDs should be preserved
        for original in &templates {
            let found = imported.templates.iter().any(|t| t.id == original.id);
            prop_assert!(found, "Template with ID {} not found after import", original.id);
        }

        // Property: Cluster IDs should be preserved
        for original in &clusters {
            let found = imported.clusters.iter().any(|c| c.id == original.id);
            prop_assert!(found, "Cluster with ID {} not found after import", original.id);
        }

        // Property: Variable names should be preserved
        for original in &variables {
            let found = imported.variables.iter().any(|v| v.name == original.name);
            prop_assert!(found, "Variable with name '{}' not found after import", original.name);
        }
    }

    /// **Feature: rustconn-bugfixes, Property 14: Native Format Round-Trip**
    /// **Validates: Requirements 13.6**
    ///
    /// For any valid NativeExport, serializing to JSON and deserializing should
    /// produce an equivalent export.
    #[test]
    fn prop_native_format_round_trip(
        connections in prop::collection::vec(arb_connection(), 0..5),
        groups in prop::collection::vec(arb_group(), 0..3),
        templates in prop::collection::vec(arb_template(), 0..3),
        clusters in prop::collection::vec(arb_cluster(), 0..3),
        variables in prop::collection::vec(arb_variable(), 0..5)
    ) {
        let export = NativeExport::with_data(
            connections.clone(),
            groups.clone(),
            templates.clone(),
            clusters.clone(),
            variables.clone(),
            Vec::new(),
        );

        let json = export.to_json().expect("Serialization should succeed");
        let imported = NativeExport::from_json(&json).expect("Import should succeed");

        // Property: Version should be preserved
        prop_assert_eq!(
            imported.version,
            export.version,
            "Version mismatch after round-trip"
        );

        // Property: App version should be preserved
        prop_assert_eq!(
            imported.app_version,
            export.app_version,
            "App version mismatch after round-trip"
        );

        // Property: Connection data should be equivalent
        for (original, reimported) in connections.iter().zip(imported.connections.iter()) {
            prop_assert_eq!(original.id, reimported.id, "Connection ID mismatch");
            prop_assert_eq!(&original.name, &reimported.name, "Connection name mismatch");
            prop_assert_eq!(&original.host, &reimported.host, "Connection host mismatch");
            prop_assert_eq!(original.port, reimported.port, "Connection port mismatch");
            prop_assert_eq!(original.protocol, reimported.protocol, "Connection protocol mismatch");
        }

        // Property: Group data should be equivalent
        for (original, reimported) in groups.iter().zip(imported.groups.iter()) {
            prop_assert_eq!(original.id, reimported.id, "Group ID mismatch");
            prop_assert_eq!(&original.name, &reimported.name, "Group name mismatch");
        }

        // Property: Template data should be equivalent
        for (original, reimported) in templates.iter().zip(imported.templates.iter()) {
            prop_assert_eq!(original.id, reimported.id, "Template ID mismatch");
            prop_assert_eq!(&original.name, &reimported.name, "Template name mismatch");
        }

        // Property: Cluster data should be equivalent
        for (original, reimported) in clusters.iter().zip(imported.clusters.iter()) {
            prop_assert_eq!(original.id, reimported.id, "Cluster ID mismatch");
            prop_assert_eq!(&original.name, &reimported.name, "Cluster name mismatch");
        }

        // Property: Variable data should be equivalent
        for (original, reimported) in variables.iter().zip(imported.variables.iter()) {
            prop_assert_eq!(&original.name, &reimported.name, "Variable name mismatch");
            prop_assert_eq!(&original.value, &reimported.value, "Variable value mismatch");
        }
    }

    /// **Feature: rustconn-bugfixes, Property 15: Native Format Schema Version**
    /// **Validates: Requirements 13.4**
    ///
    /// For any NativeExport, the JSON output should contain a version field with value >= 1.
    #[test]
    fn prop_native_format_schema_version(
        connections in prop::collection::vec(arb_connection(), 0..3)
    ) {
        let export = NativeExport::with_data(
            connections,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );

        let json = export.to_json().expect("Serialization should succeed");

        // Property: JSON should contain version field
        prop_assert!(json.contains("\"version\""), "JSON should contain version field");

        // Property: Version should be >= 1
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON should be valid");
        let version = parsed["version"].as_u64().expect("Version should be a number");

        prop_assert!(version >= 1, "Version should be >= 1, got {}", version);
        prop_assert_eq!(
            version as u32,
            NATIVE_FORMAT_VERSION,
            "Version should match NATIVE_FORMAT_VERSION"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 16: Native Import Version Validation**
    /// **Validates: Requirements 13.5**
    ///
    /// For any NativeExport JSON with version > NATIVE_FORMAT_VERSION,
    /// importing should return an UnsupportedVersion error.
    #[test]
    fn prop_native_import_version_validation(
        future_version in (NATIVE_FORMAT_VERSION + 1)..1000u32
    ) {
        let json = format!(r#"{{
            "version": {},
            "exported_at": "2024-01-01T00:00:00Z",
            "app_version": "0.1.0",
            "connections": [],
            "groups": [],
            "templates": [],
            "clusters": [],
            "variables": []
        }}"#, future_version);

        let result = NativeExport::from_json(&json);

        // Property: Import should fail with UnsupportedVersion error
        prop_assert!(
            matches!(result, Err(NativeImportError::UnsupportedVersion(v)) if v == future_version),
            "Expected UnsupportedVersion({}) error, got {:?}",
            future_version,
            result
        );
    }
}
