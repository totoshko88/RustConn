//! Property-based tests for Document system
//!
//! Tests for document dirty tracking, serialization round-trip, and encryption.

use proptest::prelude::*;
use rustconn_core::document::{Document, DocumentManager, EncryptionStrength};
use rustconn_core::models::{Connection, ConnectionGroup, ConnectionTemplate};
use rustconn_core::variables::Variable;

// Strategy for generating valid document names
fn arb_document_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,31}".prop_map(|s| s)
}

// Strategy for generating optional descriptions
fn arb_optional_description() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9 .,!?-]{1,100}".prop_map(Some),]
}

// Strategy for generating valid connection names
fn arb_connection_name() -> impl Strategy<Value = String> {
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

// Strategy for generating a simple SSH connection
fn arb_connection() -> impl Strategy<Value = Connection> {
    (arb_connection_name(), arb_host(), arb_port())
        .prop_map(|(name, host, port)| Connection::new_ssh(name, host, port))
}

// Strategy for generating a connection group
fn arb_group() -> impl Strategy<Value = ConnectionGroup> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,31}".prop_map(|name| ConnectionGroup::new(name))
}

// Strategy for generating a variable
fn arb_variable() -> impl Strategy<Value = Variable> {
    (
        "[a-z_][a-z0-9_]{0,15}",
        "[a-zA-Z0-9_.-]{1,50}",
        any::<bool>(),
    )
        .prop_map(|(name, value, is_secret)| {
            if is_secret {
                Variable::new_secret(name, value)
            } else {
                Variable::new(name, value)
            }
        })
}

// Strategy for generating a template
fn arb_template() -> impl Strategy<Value = ConnectionTemplate> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,31}".prop_map(|name| ConnectionTemplate::new_ssh(name))
}

// Strategy for generating a document with content
fn arb_document() -> impl Strategy<Value = Document> {
    (
        arb_document_name(),
        arb_optional_description(),
        prop::collection::vec(arb_connection(), 0..5),
        prop::collection::vec(arb_group(), 0..3),
        prop::collection::vec(arb_variable(), 0..5),
        prop::collection::vec(arb_template(), 0..3),
    )
        .prop_map(
            |(name, description, connections, groups, variables, templates)| {
                let mut doc = Document::new(name);
                if let Some(desc) = description {
                    doc = doc.with_description(desc);
                }
                for conn in connections {
                    doc.connections.push(conn);
                }
                for group in groups {
                    doc.groups.push(group);
                }
                for var in variables {
                    doc.variables.insert(var.name.clone(), var);
                }
                for template in templates {
                    doc.templates.push(template);
                }
                doc
            },
        )
}

// Strategy for generating valid passwords
fn arb_password() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9!@#$%^&*]{8,32}".prop_map(|s| s)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 26: Document Dirty Tracking**
    /// **Validates: Requirements 11.5**
    ///
    /// For any document modification, the dirty flag should be set to true.
    #[test]
    fn document_dirty_tracking_on_modification(
        name in arb_document_name(),
        conn in arb_connection(),
        group in arb_group(),
        var in arb_variable(),
        template in arb_template(),
    ) {
        let mut manager = DocumentManager::new();
        let id = manager.create(name);

        // New documents start dirty
        prop_assert!(manager.is_dirty(id), "New document should be dirty");

        // Mark clean
        manager.mark_clean(id);
        prop_assert!(!manager.is_dirty(id), "Document should be clean after mark_clean");

        // Adding a connection should mark dirty
        if let Some(doc) = manager.get_mut(id) {
            doc.add_connection(conn);
        }
        prop_assert!(manager.is_dirty(id), "Document should be dirty after adding connection");

        // Mark clean again
        manager.mark_clean(id);

        // Adding a group should mark dirty
        if let Some(doc) = manager.get_mut(id) {
            doc.add_group(group);
        }
        prop_assert!(manager.is_dirty(id), "Document should be dirty after adding group");

        // Mark clean again
        manager.mark_clean(id);

        // Adding a variable should mark dirty
        if let Some(doc) = manager.get_mut(id) {
            doc.set_variable(var);
        }
        prop_assert!(manager.is_dirty(id), "Document should be dirty after adding variable");

        // Mark clean again
        manager.mark_clean(id);

        // Adding a template should mark dirty
        if let Some(doc) = manager.get_mut(id) {
            doc.add_template(template);
        }
        prop_assert!(manager.is_dirty(id), "Document should be dirty after adding template");
    }

    /// **Feature: rustconn-enhancements, Property 26: Document Dirty Tracking (removal)**
    /// **Validates: Requirements 11.5**
    ///
    /// For any document modification (including removal), the dirty flag should be set to true.
    #[test]
    fn document_dirty_tracking_on_removal(
        name in arb_document_name(),
        conn in arb_connection(),
    ) {
        let mut manager = DocumentManager::new();
        let id = manager.create(name);

        // Add a connection first
        let conn_id = conn.id;
        if let Some(doc) = manager.get_mut(id) {
            doc.add_connection(conn);
        }

        // Mark clean
        manager.mark_clean(id);
        prop_assert!(!manager.is_dirty(id), "Document should be clean");

        // Removing a connection should mark dirty
        if let Some(doc) = manager.get_mut(id) {
            doc.remove_connection(conn_id);
        }
        prop_assert!(manager.is_dirty(id), "Document should be dirty after removing connection");
    }

    /// **Feature: rustconn-enhancements, Property 24: Document Serialization Round-Trip**
    /// **Validates: Requirements 11.6**
    ///
    /// For any valid document, serializing to JSON and deserializing should produce
    /// an equivalent document.
    #[test]
    fn document_json_round_trip(doc in arb_document()) {
        // Serialize to JSON
        let json = doc.to_json()
            .expect("Document should serialize to JSON");

        // Deserialize back
        let parsed = Document::from_json(&json)
            .expect("JSON should deserialize back to Document");

        // Verify key fields are preserved
        prop_assert_eq!(doc.id, parsed.id, "ID should be preserved");
        prop_assert_eq!(doc.name, parsed.name, "Name should be preserved");
        prop_assert_eq!(doc.description, parsed.description, "Description should be preserved");
        prop_assert_eq!(doc.connections.len(), parsed.connections.len(), "Connection count should be preserved");
        prop_assert_eq!(doc.groups.len(), parsed.groups.len(), "Group count should be preserved");
        prop_assert_eq!(doc.variables.len(), parsed.variables.len(), "Variable count should be preserved");
        prop_assert_eq!(doc.templates.len(), parsed.templates.len(), "Template count should be preserved");
        prop_assert_eq!(doc.format_version, parsed.format_version, "Format version should be preserved");

        // Verify connections are preserved
        for (orig, parsed_conn) in doc.connections.iter().zip(parsed.connections.iter()) {
            prop_assert_eq!(orig.id, parsed_conn.id, "Connection ID should be preserved");
            prop_assert_eq!(&orig.name, &parsed_conn.name, "Connection name should be preserved");
            prop_assert_eq!(&orig.host, &parsed_conn.host, "Connection host should be preserved");
            prop_assert_eq!(orig.port, parsed_conn.port, "Connection port should be preserved");
        }

        // Verify groups are preserved
        for (orig, parsed_group) in doc.groups.iter().zip(parsed.groups.iter()) {
            prop_assert_eq!(orig.id, parsed_group.id, "Group ID should be preserved");
            prop_assert_eq!(&orig.name, &parsed_group.name, "Group name should be preserved");
        }

        // Verify variables are preserved
        for (name, orig_var) in &doc.variables {
            let parsed_var = parsed.variables.get(name)
                .expect("Variable should exist in parsed document");
            prop_assert_eq!(&orig_var.name, &parsed_var.name, "Variable name should be preserved");
            prop_assert_eq!(&orig_var.value, &parsed_var.value, "Variable value should be preserved");
            prop_assert_eq!(orig_var.is_secret, parsed_var.is_secret, "Variable is_secret should be preserved");
        }

        // Verify templates are preserved
        for (orig, parsed_template) in doc.templates.iter().zip(parsed.templates.iter()) {
            prop_assert_eq!(orig.id, parsed_template.id, "Template ID should be preserved");
            prop_assert_eq!(&orig.name, &parsed_template.name, "Template name should be preserved");
        }
    }

    /// **Feature: rustconn-enhancements, Property 24: Document Serialization Round-Trip (YAML)**
    /// **Validates: Requirements 11.6**
    ///
    /// For any valid document, serializing to YAML and deserializing should produce
    /// an equivalent document.
    #[test]
    fn document_yaml_round_trip(doc in arb_document()) {
        // Serialize to YAML
        let yaml = doc.to_yaml()
            .expect("Document should serialize to YAML");

        // Deserialize back
        let parsed = Document::from_yaml(&yaml)
            .expect("YAML should deserialize back to Document");

        // Verify key fields are preserved
        prop_assert_eq!(doc.id, parsed.id, "ID should be preserved");
        prop_assert_eq!(doc.name, parsed.name, "Name should be preserved");
        prop_assert_eq!(doc.description, parsed.description, "Description should be preserved");
        prop_assert_eq!(doc.connections.len(), parsed.connections.len(), "Connection count should be preserved");
        prop_assert_eq!(doc.groups.len(), parsed.groups.len(), "Group count should be preserved");
        prop_assert_eq!(doc.variables.len(), parsed.variables.len(), "Variable count should be preserved");
        prop_assert_eq!(doc.templates.len(), parsed.templates.len(), "Template count should be preserved");
    }

}

// Separate proptest block with fewer cases for encryption tests (they're slow due to Argon2)
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]

    /// **Feature: rustconn-enhancements, Property 25: Document Encryption Round-Trip**
    /// **Validates: Requirements 11.3**
    ///
    /// For any document and password, encrypting and decrypting should produce
    /// the original document.
    #[test]
    fn document_encryption_round_trip(
        doc in arb_document(),
        password in arb_password(),
    ) {
        use tempfile::tempdir;

        let temp_dir = tempdir().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("test_encrypted.rcdb");

        let mut manager = DocumentManager::new();

        // Insert the document into the manager using the insert method
        let id = manager.insert(doc.clone());

        // Save with encryption
        manager.save(id, &file_path, Some(&password), EncryptionStrength::Standard)
            .expect("Should save encrypted document");

        // Load with correct password
        let mut manager2 = DocumentManager::new();
        let loaded_id = manager2.load(&file_path, Some(&password))
            .expect("Should load encrypted document");

        let loaded_doc = manager2.get(loaded_id)
            .expect("Loaded document should exist");

        // Verify key fields are preserved
        prop_assert_eq!(doc.id, loaded_doc.id, "ID should be preserved after encryption round-trip");
        prop_assert_eq!(&doc.name, &loaded_doc.name, "Name should be preserved after encryption round-trip");
        prop_assert_eq!(&doc.description, &loaded_doc.description, "Description should be preserved");
        prop_assert_eq!(doc.connections.len(), loaded_doc.connections.len(), "Connection count should be preserved");
        prop_assert_eq!(doc.groups.len(), loaded_doc.groups.len(), "Group count should be preserved");
        prop_assert_eq!(doc.variables.len(), loaded_doc.variables.len(), "Variable count should be preserved");
        prop_assert_eq!(doc.templates.len(), loaded_doc.templates.len(), "Template count should be preserved");
    }

    /// **Feature: rustconn-enhancements, Property 25: Document Encryption - Wrong Password**
    /// **Validates: Requirements 11.3**
    ///
    /// For any encrypted document, attempting to decrypt with wrong password should fail.
    #[test]
    fn document_encryption_wrong_password_fails(
        doc in arb_document(),
        password in arb_password(),
        wrong_password in arb_password(),
    ) {
        use tempfile::tempdir;

        // Skip if passwords happen to be the same
        prop_assume!(password != wrong_password);

        let temp_dir = tempdir().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("test_encrypted.rcdb");

        let mut manager = DocumentManager::new();

        // Insert the document into the manager using the insert method
        manager.insert(doc);

        // Get the document ID (it's the first one)
        let id = manager.document_ids()[0];

        // Save with encryption
        manager.save(id, &file_path, Some(&password), EncryptionStrength::Standard)
            .expect("Should save encrypted document");

        // Try to load with wrong password
        let mut manager2 = DocumentManager::new();
        let result = manager2.load(&file_path, Some(&wrong_password));

        prop_assert!(result.is_err(), "Loading with wrong password should fail");
    }
}
