//! Property-based tests for Connection Clipboard operations
//!
//! **Feature: rustconn-bugfixes, Property 10: Connection Copy Creates Valid Duplicate**
//! **Feature: rustconn-bugfixes, Property 11: Connection Paste Preserves Group**
//! **Validates: Requirements 12.1, 12.2, 12.3**

use chrono::Utc;
use proptest::prelude::*;
use rustconn_core::{Connection, ProtocolConfig, RdpConfig, SshConfig, VncConfig};
use uuid::Uuid;

// ========== ConnectionClipboard Implementation for Testing ==========
// This mirrors the implementation in rustconn/src/state.rs for testing purposes

/// Internal clipboard for connection copy/paste operations
#[derive(Debug, Clone, Default)]
pub struct ConnectionClipboard {
    /// Copied connection data
    connection: Option<Connection>,
    /// Source group ID where the connection was copied from
    source_group: Option<Uuid>,
}

impl ConnectionClipboard {
    /// Creates a new empty clipboard
    #[must_use]
    pub const fn new() -> Self {
        Self {
            connection: None,
            source_group: None,
        }
    }

    /// Copies a connection to the clipboard
    pub fn copy(&mut self, connection: &Connection, group_id: Option<Uuid>) {
        self.connection = Some(connection.clone());
        self.source_group = group_id;
    }

    /// Pastes the connection from the clipboard, creating a duplicate
    #[must_use]
    pub fn paste(&self) -> Option<Connection> {
        self.connection.as_ref().map(|conn| {
            let mut new_conn = conn.clone();
            new_conn.id = Uuid::new_v4();
            new_conn.name = format!("{} (Copy)", conn.name);
            let now = Utc::now();
            new_conn.created_at = now;
            new_conn.updated_at = now;
            new_conn.last_connected = None;
            new_conn
        })
    }

    /// Checks if the clipboard has content
    #[must_use]
    pub const fn has_content(&self) -> bool {
        self.connection.is_some()
    }

    /// Gets the source group ID
    #[must_use]
    pub const fn source_group(&self) -> Option<Uuid> {
        self.source_group
    }

    /// Clears the clipboard
    pub fn clear(&mut self) {
        self.connection = None;
        self.source_group = None;
    }
}

// ========== Generators ==========

/// Strategy for generating valid connection names (non-empty)
fn arb_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,31}".prop_map(|s| s)
}

/// Strategy for generating valid hostnames (non-empty)
fn arb_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*".prop_map(|s| s)
}

/// Strategy for generating valid ports (non-zero)
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

/// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9_]{0,15}".prop_map(Some),]
}

/// Strategy for generating tags
fn arb_tags() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{1,10}", 0..5)
}

/// Strategy for SSH config
fn arb_ssh_config() -> impl Strategy<Value = SshConfig> {
    Just(SshConfig::default())
}

/// Strategy for RDP config
fn arb_rdp_config() -> impl Strategy<Value = RdpConfig> {
    Just(RdpConfig::default())
}

/// Strategy for VNC config
fn arb_vnc_config() -> impl Strategy<Value = VncConfig> {
    Just(VncConfig::default())
}

/// Strategy for protocol config
fn arb_protocol_config() -> impl Strategy<Value = ProtocolConfig> {
    prop_oneof![
        arb_ssh_config().prop_map(ProtocolConfig::Ssh),
        arb_rdp_config().prop_map(ProtocolConfig::Rdp),
        arb_vnc_config().prop_map(ProtocolConfig::Vnc),
    ]
}

/// Strategy for generating a complete Connection
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

/// Strategy for generating optional group IDs
fn arb_optional_group_id() -> impl Strategy<Value = Option<Uuid>> {
    prop_oneof![
        Just(None),
        any::<[u8; 16]>().prop_map(|bytes| Some(Uuid::from_bytes(bytes))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 10: Connection Copy Creates Valid Duplicate**
    /// **Validates: Requirements 12.1, 12.2**
    ///
    /// For any connection, copying and pasting should create a new connection with:
    /// - A different (new) ID
    /// - "(Copy)" suffix appended to the name
    /// - All other fields preserved
    #[test]
    fn copy_paste_creates_valid_duplicate(conn in arb_connection()) {
        let mut clipboard = ConnectionClipboard::new();
        let original_id = conn.id;
        let original_name = conn.name.clone();

        // Copy the connection
        clipboard.copy(&conn, conn.group_id);

        // Verify clipboard has content
        prop_assert!(clipboard.has_content(), "Clipboard should have content after copy");

        // Paste the connection
        let pasted = clipboard.paste().expect("Paste should return a connection");

        // Verify ID is different
        prop_assert_ne!(
            pasted.id, original_id,
            "Pasted connection should have a different ID"
        );

        // Verify name has "(Copy)" suffix
        let expected_name = format!("{} (Copy)", original_name);
        prop_assert_eq!(
            &pasted.name, &expected_name,
            "Pasted connection should have '(Copy)' suffix"
        );

        // Verify other fields are preserved
        prop_assert_eq!(&pasted.host, &conn.host, "Host should be preserved");
        prop_assert_eq!(pasted.port, conn.port, "Port should be preserved");
        prop_assert_eq!(&pasted.username, &conn.username, "Username should be preserved");
        prop_assert_eq!(&pasted.tags, &conn.tags, "Tags should be preserved");
        prop_assert_eq!(
            &pasted.protocol_config, &conn.protocol_config,
            "Protocol config should be preserved"
        );
        prop_assert_eq!(pasted.protocol, conn.protocol, "Protocol type should be preserved");

        // Verify timestamps are updated
        prop_assert!(
            pasted.created_at >= conn.created_at,
            "Created timestamp should be updated"
        );
        prop_assert!(
            pasted.updated_at >= conn.updated_at,
            "Updated timestamp should be updated"
        );

        // Verify last_connected is cleared
        prop_assert!(
            pasted.last_connected.is_none(),
            "Last connected should be cleared for pasted connection"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 11: Connection Paste Preserves Group**
    /// **Validates: Requirements 12.3**
    ///
    /// For any copied connection with a source group, pasting should return
    /// the same source group ID.
    #[test]
    fn paste_preserves_source_group(
        conn in arb_connection(),
        group_id in arb_optional_group_id(),
    ) {
        let mut clipboard = ConnectionClipboard::new();

        // Copy the connection with a specific group
        clipboard.copy(&conn, group_id);

        // Verify source group is preserved
        prop_assert_eq!(
            clipboard.source_group(), group_id,
            "Source group should be preserved in clipboard"
        );

        // Paste should still work
        let pasted = clipboard.paste().expect("Paste should return a connection");
        prop_assert_ne!(pasted.id, conn.id, "Pasted connection should have different ID");
    }

    /// **Feature: rustconn-bugfixes, Property 10: Connection Copy Creates Valid Duplicate**
    /// **Validates: Requirements 12.1**
    ///
    /// Multiple pastes from the same copy should create unique connections each time.
    #[test]
    fn multiple_pastes_create_unique_connections(conn in arb_connection()) {
        let mut clipboard = ConnectionClipboard::new();

        // Copy the connection
        clipboard.copy(&conn, conn.group_id);

        // Paste multiple times
        let paste1 = clipboard.paste().expect("First paste should succeed");
        let paste2 = clipboard.paste().expect("Second paste should succeed");
        let paste3 = clipboard.paste().expect("Third paste should succeed");

        // All pasted connections should have unique IDs
        prop_assert_ne!(paste1.id, paste2.id, "First and second paste should have different IDs");
        prop_assert_ne!(paste2.id, paste3.id, "Second and third paste should have different IDs");
        prop_assert_ne!(paste1.id, paste3.id, "First and third paste should have different IDs");

        // All should be different from original
        prop_assert_ne!(paste1.id, conn.id, "First paste should differ from original");
        prop_assert_ne!(paste2.id, conn.id, "Second paste should differ from original");
        prop_assert_ne!(paste3.id, conn.id, "Third paste should differ from original");

        // All should have the same name (with Copy suffix)
        let expected_name = format!("{} (Copy)", conn.name);
        prop_assert_eq!(&paste1.name, &expected_name, "First paste name should match");
        prop_assert_eq!(&paste2.name, &expected_name, "Second paste name should match");
        prop_assert_eq!(&paste3.name, &expected_name, "Third paste name should match");
    }

    /// **Feature: rustconn-bugfixes, Property 10: Connection Copy Creates Valid Duplicate**
    /// **Validates: Requirements 12.1, 12.2**
    ///
    /// Copying a new connection should replace the previous clipboard content.
    #[test]
    fn copy_replaces_previous_content(
        conn1 in arb_connection(),
        conn2 in arb_connection(),
    ) {
        let mut clipboard = ConnectionClipboard::new();

        // Copy first connection
        clipboard.copy(&conn1, conn1.group_id);
        prop_assert!(clipboard.has_content(), "Clipboard should have content");

        // Copy second connection (should replace)
        clipboard.copy(&conn2, conn2.group_id);
        prop_assert!(clipboard.has_content(), "Clipboard should still have content");

        // Paste should return second connection's data
        let pasted = clipboard.paste().expect("Paste should succeed");
        let expected_name = format!("{} (Copy)", conn2.name);
        prop_assert_eq!(
            &pasted.name, &expected_name,
            "Pasted connection should be based on second copied connection"
        );
        prop_assert_eq!(&pasted.host, &conn2.host, "Host should match second connection");
    }

    /// **Feature: rustconn-bugfixes, Property 10: Connection Copy Creates Valid Duplicate**
    /// **Validates: Requirements 12.1**
    ///
    /// Empty clipboard should return None on paste.
    #[test]
    fn empty_clipboard_returns_none_on_paste(_seed in any::<u64>()) {
        let clipboard = ConnectionClipboard::new();

        prop_assert!(!clipboard.has_content(), "New clipboard should be empty");
        prop_assert!(clipboard.paste().is_none(), "Paste on empty clipboard should return None");
        prop_assert!(clipboard.source_group().is_none(), "Source group should be None for empty clipboard");
    }

    /// **Feature: rustconn-bugfixes, Property 10: Connection Copy Creates Valid Duplicate**
    /// **Validates: Requirements 12.1**
    ///
    /// Clearing the clipboard should remove all content.
    #[test]
    fn clear_removes_clipboard_content(conn in arb_connection(), group_id in arb_optional_group_id()) {
        let mut clipboard = ConnectionClipboard::new();

        // Copy a connection
        clipboard.copy(&conn, group_id);
        prop_assert!(clipboard.has_content(), "Clipboard should have content after copy");

        // Clear the clipboard
        clipboard.clear();

        // Verify clipboard is empty
        prop_assert!(!clipboard.has_content(), "Clipboard should be empty after clear");
        prop_assert!(clipboard.paste().is_none(), "Paste should return None after clear");
        prop_assert!(clipboard.source_group().is_none(), "Source group should be None after clear");
    }
}
