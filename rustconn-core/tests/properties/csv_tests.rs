//! Property-based tests for CSV import/export.

use proptest::prelude::*;
use rustconn_core::export::csv_export::CsvExporter;
use rustconn_core::import::{CsvImporter, ImportSource};
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType, SshConfig};

/// Strategy for generating a valid protocol type string.
fn protocol_strategy() -> impl Strategy<Value = ProtocolType> {
    prop_oneof![
        Just(ProtocolType::Ssh),
        Just(ProtocolType::Rdp),
        Just(ProtocolType::Vnc),
        Just(ProtocolType::Telnet),
        Just(ProtocolType::Sftp),
    ]
}

/// Strategy for generating a safe connection name (non-empty, no control chars).
/// Must not have leading/trailing whitespace since the importer trims fields.
fn safe_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_.]{0,29}"
}

/// Strategy for generating a safe hostname.
fn safe_host_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9.-]{0,20}\\.[a-z]{2,4}"
}

/// Strategy for generating optional username.
fn optional_username_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z_][a-z0-9_]{0,15}".prop_map(Some),]
}

/// Strategy for generating optional description (may contain commas, quotes, newlines).
/// Must not have leading/trailing whitespace to survive trim in the importer.
fn optional_description_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z][a-zA-Z0-9,;.!?]{0,39}".prop_map(Some),]
}

/// Strategy for generating tags (each tag is simple alphanumeric).
fn tags_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{1,10}", 0..4)
}

/// Helper: create a default protocol config for a given type.
fn default_config(protocol: ProtocolType) -> ProtocolConfig {
    match protocol {
        ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
        ProtocolType::Rdp => ProtocolConfig::Rdp(rustconn_core::models::RdpConfig::default()),
        ProtocolType::Vnc => ProtocolConfig::Vnc(rustconn_core::models::VncConfig::default()),
        ProtocolType::Telnet => {
            ProtocolConfig::Telnet(rustconn_core::models::TelnetConfig::default())
        }
        ProtocolType::Sftp => ProtocolConfig::Sftp(SshConfig::default()),
        _ => ProtocolConfig::Ssh(SshConfig::default()),
    }
}

// ---------------------------------------------------------------------------
// Proptest 4: Round-trip — export → parse → equivalent connections
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn csv_round_trip(
        names in prop::collection::vec(safe_name_strategy(), 1..5),
        hosts in prop::collection::vec(safe_host_strategy(), 1..5),
        protocols in prop::collection::vec(protocol_strategy(), 1..5),
        ports in prop::collection::vec(1u16..=65535, 1..5),
        usernames in prop::collection::vec(optional_username_strategy(), 1..5),
        descriptions in prop::collection::vec(optional_description_strategy(), 1..5),
        tags_list in prop::collection::vec(tags_strategy(), 1..5),
    ) {
        let count = names.len()
            .min(hosts.len())
            .min(protocols.len())
            .min(ports.len())
            .min(usernames.len())
            .min(descriptions.len())
            .min(tags_list.len());

        let mut connections = Vec::with_capacity(count);
        for i in 0..count {
            let protocol = protocols[i];
            let config = default_config(protocol);
            let mut conn = Connection::new(
                names[i].clone(),
                hosts[i].clone(),
                ports[i],
                config,
            );
            conn.username = usernames[i].clone();
            conn.description = descriptions[i].clone();
            conn.tags = tags_list[i].clone();
            connections.push(conn);
        }

        let groups: Vec<ConnectionGroup> = Vec::new();

        // Export
        let exporter = CsvExporter::new();
        let csv_content = exporter.export_to_string(&connections, &groups);

        // Re-import
        let importer = CsvImporter::new();
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let csv_path = dir.path().join("roundtrip.csv");
        std::fs::write(&csv_path, &csv_content)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let result = importer
            .import_from_path(&csv_path)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        prop_assert_eq!(
            result.connections.len(),
            count,
            "Round-trip should preserve connection count"
        );

        for (original, imported) in connections.iter().zip(result.connections.iter()) {
            prop_assert_eq!(&original.name, &imported.name);
            prop_assert_eq!(&original.host, &imported.host);
            prop_assert_eq!(original.port, imported.port);
            prop_assert_eq!(original.protocol, imported.protocol);
            prop_assert_eq!(&original.username, &imported.username);
            prop_assert_eq!(&original.description, &imported.description);
            // Tags: imported connection gets "imported:csv" appended, so compare user tags
            let original_tags: Vec<&str> = original.tags.iter()
                .filter(|t| !t.starts_with("imported:"))
                .map(String::as_str)
                .collect();
            let imported_tags: Vec<&str> = imported.tags.iter()
                .filter(|t| !t.starts_with("imported:"))
                .map(String::as_str)
                .collect();
            prop_assert_eq!(original_tags, imported_tags);
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest 5: Strings with commas, quotes, newlines are correctly quoted (RFC 4180)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(60))]

    #[test]
    fn csv_special_chars_quoted_correctly(
        name in "[a-zA-Z0-9]{1,10}",
        host in "[a-z]{1,8}\\.[a-z]{2,3}",
        // Description with special CSV characters (must not be whitespace-only)
        special in prop_oneof![
            Just("value,with,commas".to_string()),
            Just("value\"with\"quotes".to_string()),
            Just("line1\nline2\nline3".to_string()),
            Just("mixed,\"special\nchars".to_string()),
            "[a-zA-Z][a-zA-Z0-9,\"]{1,29}",
        ],
    ) {
        let config = ProtocolConfig::Ssh(SshConfig::default());
        let mut conn = Connection::new(name.clone(), host.clone(), 22, config);
        conn.description = Some(special.clone());

        let exporter = CsvExporter::new();
        let csv_content = exporter.export_to_string(&[conn], &[]);

        // Write and re-import
        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let csv_path = dir.path().join("special.csv");
        std::fs::write(&csv_path, &csv_content)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        let importer = CsvImporter::new();
        let result = importer
            .import_from_path(&csv_path)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        prop_assert_eq!(result.connections.len(), 1, "Should import exactly 1 connection");
        let imported = &result.connections[0];
        prop_assert_eq!(&imported.name, &name);
        prop_assert_eq!(&imported.host, &host);
        prop_assert_eq!(&imported.description, &Some(special));
    }
}

// ---------------------------------------------------------------------------
// Proptest 6: Missing required fields (name, host) → SkippedEntry
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn csv_missing_required_fields_skipped(
        has_name in proptest::bool::ANY,
        has_host in proptest::bool::ANY,
        name_val in "[a-zA-Z]{1,10}",
        host_val in "[a-z]{1,8}\\.[a-z]{2,3}",
    ) {
        // At least one of name/host must be missing for this test
        prop_assume!(!has_name || !has_host);

        let name_field = if has_name { &name_val as &str } else { "" };
        let host_field = if has_host { &host_val as &str } else { "" };

        let csv_content = format!(
            "name,host,port,protocol\n{name_field},{host_field},22,ssh\n"
        );

        let dir = tempfile::tempdir().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let csv_path = dir.path().join("missing.csv");
        std::fs::write(&csv_path, &csv_content)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        let importer = CsvImporter::new();
        let result = importer
            .import_from_path(&csv_path)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        prop_assert_eq!(
            result.connections.len(),
            0,
            "Row with missing required field should not produce a connection"
        );
        prop_assert!(
            !result.skipped.is_empty(),
            "Row with missing required field should be recorded as skipped"
        );
    }
}
