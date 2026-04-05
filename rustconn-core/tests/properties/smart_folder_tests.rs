//! Property-based tests for Smart Folders: filter evaluation, serde round-trip,
//! and empty-filter behaviour.
//!
//! **Validates: Requirements 9.2, 9.3, 9.8, 11.5**

use std::collections::HashMap;

use chrono::Utc;
use proptest::prelude::*;
use rustconn_core::models::{Connection, ProtocolConfig, ProtocolType, SmartFolder, SshConfig};
use rustconn_core::smart_folder::SmartFolderManager;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Selects a random `ProtocolType` variant.
fn arb_protocol_type() -> impl Strategy<Value = ProtocolType> {
    prop_oneof![
        Just(ProtocolType::Ssh),
        Just(ProtocolType::Rdp),
        Just(ProtocolType::Vnc),
        Just(ProtocolType::Spice),
        Just(ProtocolType::Telnet),
        Just(ProtocolType::ZeroTrust),
        Just(ProtocolType::Serial),
        Just(ProtocolType::Sftp),
        Just(ProtocolType::Kubernetes),
        Just(ProtocolType::Mosh),
    ]
}

/// Generates a simple hostname-safe string (lowercase ascii + digits + dots).
fn arb_hostname() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,8}(\\.[a-z][a-z0-9]{0,8}){1,3}"
}

/// Generates a simple tag (lowercase ascii, 1-10 chars).
fn arb_tag() -> impl Strategy<Value = String> {
    "[a-z]{1,10}"
}

/// Builds a `Connection` with the given protocol, host, tags, and group_id.
fn make_connection(
    protocol: ProtocolType,
    host: String,
    tags: Vec<String>,
    group_id: Option<Uuid>,
) -> Connection {
    let now = Utc::now();
    Connection {
        id: Uuid::new_v4(),
        name: "test-conn".to_string(),
        description: None,
        protocol,
        host,
        port: 22,
        username: None,
        group_id,
        tags,
        created_at: now,
        updated_at: now,
        protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
        automation: Default::default(),
        sort_order: 0,
        last_connected: None,
        password_source: Default::default(),
        domain: None,
        custom_properties: Vec::new(),
        pre_connect_task: None,
        post_disconnect_task: None,
        wol_config: None,
        local_variables: HashMap::new(),
        log_config: None,
        key_sequence: None,
        window_mode: Default::default(),
        remember_window_position: false,
        window_geometry: None,
        skip_port_check: false,
        is_pinned: false,
        pin_order: 0,
        icon: None,
        monitoring_config: None,
        activity_monitor_config: None,
        theme_override: None,
        session_recording_enabled: false,
        highlight_rules: Vec::new(),
    }
}

/// Generates a `SmartFolder` with a protocol filter and tag filters that the
/// given connection is guaranteed to satisfy.
fn matching_smart_folder(
    protocol: ProtocolType,
    tags: Vec<String>,
    group_id: Option<Uuid>,
    host: &str,
) -> SmartFolder {
    // Build a glob that matches the exact host: "*.suffix" where suffix is
    // everything after the first char, so the host always matches.
    let host_pattern = if host.len() > 1 {
        format!("*{}", &host[1..])
    } else {
        format!("{}*", host)
    };

    SmartFolder {
        id: Uuid::new_v4(),
        name: "matching-folder".to_string(),
        filter_protocol: Some(protocol),
        filter_tags: tags,
        filter_host_pattern: Some(host_pattern),
        filter_group_id: group_id,
        sort_order: 0,
    }
}

// ---------------------------------------------------------------------------
// Proptest 17: Connection matching ALL criteria → present in result
// **Validates: Requirements 9.2, 11.5**
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn matching_connection_is_present_in_evaluate(
        proto in arb_protocol_type(),
        host in arb_hostname(),
        tags in prop::collection::vec(arb_tag(), 0..3),
        use_group in any::<bool>(),
    ) {
        let group_id = if use_group { Some(Uuid::new_v4()) } else { None };
        let conn = make_connection(proto, host.clone(), tags.clone(), group_id);
        let folder = matching_smart_folder(proto, tags, group_id, &host);

        let mgr = SmartFolderManager::new();
        let connections = [conn.clone()];
        let result = mgr.evaluate(&folder, &connections);

        prop_assert!(
            result.iter().any(|c| c.id == conn.id),
            "connection matching all criteria must appear in evaluate() result"
        );
    }
}

// ---------------------------------------------------------------------------
// Proptest 18: Connection NOT matching at least one criterion → absent
// **Validates: Requirements 9.2, 11.5**
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn non_matching_connection_is_absent(
        folder_proto in arb_protocol_type(),
        conn_host in arb_hostname(),
        folder_tags in prop::collection::vec(arb_tag(), 1..3),
    ) {
        // The connection deliberately has a DIFFERENT protocol and NO matching tags.
        let conn_proto = match folder_proto {
            ProtocolType::Ssh => ProtocolType::Rdp,
            _ => ProtocolType::Ssh,
        };
        let conn = make_connection(conn_proto, conn_host, Vec::new(), None);

        let folder = SmartFolder {
            id: Uuid::new_v4(),
            name: "strict-folder".to_string(),
            filter_protocol: Some(folder_proto),
            filter_tags: folder_tags,
            filter_host_pattern: None,
            filter_group_id: None,
            sort_order: 0,
        };

        let mgr = SmartFolderManager::new();
        let connections = [conn.clone()];
        let result = mgr.evaluate(&folder, &connections);

        prop_assert!(
            !result.iter().any(|c| c.id == conn.id),
            "connection not matching at least one criterion must NOT appear in result"
        );
    }
}

// ---------------------------------------------------------------------------
// Proptest 19: Serde round-trip for SmartFolder
// **Validates: Requirements 9.8, 11.5**
// ---------------------------------------------------------------------------

fn arb_smart_folder() -> impl Strategy<Value = SmartFolder> {
    (
        "[a-zA-Z0-9 _-]{1,20}",                            // name
        prop::option::of(arb_protocol_type()),             // filter_protocol
        prop::collection::vec(arb_tag(), 0..4),            // filter_tags
        prop::option::of("\\*\\.[a-z]{2,6}\\.[a-z]{2,4}"), // filter_host_pattern
        any::<bool>(),                                     // has group_id
        any::<i32>(),                                      // sort_order
    )
        .prop_map(
            |(name, proto, tags, host_pat, has_group, sort)| SmartFolder {
                id: Uuid::new_v4(),
                name,
                filter_protocol: proto,
                filter_tags: tags,
                filter_host_pattern: host_pat,
                filter_group_id: if has_group {
                    Some(Uuid::new_v4())
                } else {
                    None
                },
                sort_order: sort,
            },
        )
}

proptest! {
    #[test]
    fn serde_roundtrip_preserves_smart_folder(folder in arb_smart_folder()) {
        let json = serde_json::to_string(&folder).map_err(|e| {
            TestCaseError::fail(format!("serialization failed: {e}"))
        })?;
        let restored: SmartFolder = serde_json::from_str(&json).map_err(|e| {
            TestCaseError::fail(format!("deserialization failed: {e}"))
        })?;

        prop_assert_eq!(&folder.name, &restored.name);
        prop_assert_eq!(&folder.filter_protocol, &restored.filter_protocol);
        prop_assert_eq!(&folder.filter_tags, &restored.filter_tags);
        prop_assert_eq!(&folder.filter_host_pattern, &restored.filter_host_pattern);
        prop_assert_eq!(&folder.filter_group_id, &restored.filter_group_id);
        prop_assert_eq!(folder.sort_order, restored.sort_order);
    }
}

// ---------------------------------------------------------------------------
// Proptest 20: Empty filter → empty result
// **Validates: Requirements 9.3, 11.5**
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn empty_filter_yields_empty_result(
        proto in arb_protocol_type(),
        host in arb_hostname(),
    ) {
        let conn = make_connection(proto, host, vec!["web".to_string()], None);

        let empty_folder = SmartFolder {
            id: Uuid::new_v4(),
            name: "empty".to_string(),
            filter_protocol: None,
            filter_tags: Vec::new(),
            filter_host_pattern: None,
            filter_group_id: None,
            sort_order: 0,
        };

        let mgr = SmartFolderManager::new();
        let connections = [conn];
        let result = mgr.evaluate(&empty_folder, &connections);

        prop_assert!(
            result.is_empty(),
            "smart folder with no filter criteria must return empty result"
        );
    }
}
