//! Property tests for import preview and merge strategies

use proptest::prelude::*;
use rustconn_core::import::{
    DuplicateAction, ImportPreview, ImportResult, MergeStrategy, PreviewConnection, PreviewGroup,
};
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, SshConfig};
use uuid::Uuid;

// Strategy for generating connection names
fn name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9 ]{0,29}"
}

// Strategy for generating hostnames
fn host_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z][a-z0-9-]{0,15}\\.[a-z]{2,6}",
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}",
    ]
}

// Strategy for generating ports
fn port_strategy() -> impl Strategy<Value = u16> {
    prop_oneof![Just(22u16), Just(3389u16), Just(5900u16), 1024u16..65535]
}

fn create_ssh_connection(name: String, host: String, port: u16) -> Connection {
    Connection::new(name, host, port, ProtocolConfig::Ssh(SshConfig::default()))
}

proptest! {
    /// Property: MergeStrategy::all returns all strategies
    #[test]
    fn merge_strategy_all_returns_all(_dummy in 0..1) {
        let all = MergeStrategy::all();
        prop_assert_eq!(all.len(), 4);
        prop_assert!(all.contains(&MergeStrategy::SkipExisting));
        prop_assert!(all.contains(&MergeStrategy::UpdateExisting));
        prop_assert!(all.contains(&MergeStrategy::CreateDuplicates));
        prop_assert!(all.contains(&MergeStrategy::AskForEach));
    }

    /// Property: Each MergeStrategy has a display name
    #[test]
    fn merge_strategy_has_display_name(_dummy in 0..1) {
        for strategy in MergeStrategy::all() {
            let name = strategy.display_name();
            prop_assert!(!name.is_empty());
        }
    }

    /// Property: Each MergeStrategy has a description
    #[test]
    fn merge_strategy_has_description(_dummy in 0..1) {
        for strategy in MergeStrategy::all() {
            let desc = strategy.description();
            prop_assert!(!desc.is_empty());
            prop_assert!(desc.len() > 10); // Should be meaningful
        }
    }

    /// Property: Preview with no existing connections marks all as new
    #[test]
    fn preview_no_existing_all_new(
        names in prop::collection::vec(name_strategy(), 1..5),
        hosts in prop::collection::vec(host_strategy(), 1..5),
    ) {
        let count = names.len().min(hosts.len());
        let mut result = ImportResult::new();

        for i in 0..count {
            result.add_connection(create_ssh_connection(
                names[i].clone(),
                hosts[i].clone(),
                22,
            ));
        }

        let existing: Vec<Connection> = vec![];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        prop_assert_eq!(preview.total_connections(), count);
        prop_assert_eq!(preview.new_connections(), count);
        prop_assert_eq!(preview.duplicate_connections(), 0);
        prop_assert_eq!(preview.new_count, count);
        prop_assert_eq!(preview.skip_count, 0);
    }

    /// Property: Preview correctly identifies duplicates by host+port
    #[test]
    fn preview_identifies_duplicates(
        name1 in name_strategy(),
        name2 in name_strategy(),
        host in host_strategy(),
        port in port_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(name1, host.clone(), port));

        let existing = vec![create_ssh_connection(name2, host, port)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        prop_assert_eq!(preview.duplicate_connections(), 1);
        prop_assert!(preview.connections[0].is_duplicate);
        prop_assert!(preview.connections[0].existing_id.is_some());
    }

    /// Property: SkipExisting strategy marks duplicates as Skip
    #[test]
    fn skip_existing_marks_skip(
        name in name_strategy(),
        host in host_strategy(),
        port in port_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(name.clone(), host.clone(), port));

        let existing = vec![create_ssh_connection(name, host, port)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        prop_assert_eq!(preview.connections[0].suggested_action, DuplicateAction::Skip);
        prop_assert_eq!(preview.skip_count, 1);
    }

    /// Property: UpdateExisting strategy marks duplicates as Update
    #[test]
    fn update_existing_marks_update(
        name in name_strategy(),
        host in host_strategy(),
        port in port_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(name.clone(), host.clone(), port));

        let existing = vec![create_ssh_connection(name, host, port)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::UpdateExisting,
            "test",
            "test.yml",
        );

        prop_assert_eq!(preview.connections[0].suggested_action, DuplicateAction::Update);
        prop_assert_eq!(preview.update_count, 1);
    }

    /// Property: CreateDuplicates strategy marks duplicates as CreateDuplicate
    #[test]
    fn create_duplicates_marks_create(
        name in name_strategy(),
        host in host_strategy(),
        port in port_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(name.clone(), host.clone(), port));

        let existing = vec![create_ssh_connection(name, host, port)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::CreateDuplicates,
            "test",
            "test.yml",
        );

        prop_assert_eq!(
            preview.connections[0].suggested_action,
            DuplicateAction::CreateDuplicate
        );
        prop_assert_eq!(preview.duplicate_count, 1);
    }

    /// Property: set_connection_action updates counts correctly
    #[test]
    fn set_action_updates_counts(
        name in name_strategy(),
        host in host_strategy(),
        port in port_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(name.clone(), host.clone(), port));

        let existing = vec![create_ssh_connection(name, host, port)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let mut preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        prop_assert_eq!(preview.skip_count, 1);
        prop_assert_eq!(preview.update_count, 0);

        // Change to Update
        prop_assert!(preview.set_connection_action(0, DuplicateAction::Update));
        prop_assert_eq!(preview.skip_count, 0);
        prop_assert_eq!(preview.update_count, 1);

        // Change to CreateDuplicate
        prop_assert!(preview.set_connection_action(0, DuplicateAction::CreateDuplicate));
        prop_assert_eq!(preview.update_count, 0);
        prop_assert_eq!(preview.duplicate_count, 1);
    }

    /// Property: set_connection_action returns false for invalid index
    #[test]
    fn set_action_invalid_index_returns_false(
        name in name_strategy(),
        host in host_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(name, host, 22));

        let existing: Vec<Connection> = vec![];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let mut preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        prop_assert!(!preview.set_connection_action(999, DuplicateAction::Skip));
    }

    /// Property: apply returns correct partitions
    #[test]
    fn apply_partitions_correctly(
        new_name in name_strategy(),
        new_host in host_strategy(),
        dup_name in name_strategy(),
        dup_host in host_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(create_ssh_connection(new_name, new_host, 22));
        result.add_connection(create_ssh_connection(dup_name.clone(), dup_host.clone(), 3389));

        let existing = vec![create_ssh_connection(dup_name, dup_host, 3389)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let mut preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        // Change duplicate to Update
        preview.set_connection_action(1, DuplicateAction::Update);

        let (to_create, to_update, _groups) = preview.apply();

        prop_assert_eq!(to_create.len(), 1);
        prop_assert_eq!(to_update.len(), 1);
    }

    /// Property: Group deduplication works correctly
    #[test]
    fn group_deduplication(
        group_name in "[a-zA-Z][a-zA-Z0-9 ]{0,20}",
    ) {
        let mut result = ImportResult::new();
        result.add_group(ConnectionGroup::new(group_name.clone()));

        let existing_groups = vec![ConnectionGroup::new(group_name)];
        let existing_connections: Vec<Connection> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing_connections,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        prop_assert_eq!(preview.total_groups(), 1);
        prop_assert!(preview.groups[0].is_duplicate);
        prop_assert!(preview.groups[0].existing_id.is_some());
    }

    /// Property: apply excludes duplicate groups
    #[test]
    fn apply_excludes_duplicate_groups(
        group_name in "[a-zA-Z][a-zA-Z0-9 ]{0,20}",
        new_group_name in "[a-zA-Z][a-zA-Z0-9 ]{0,20}",
    ) {
        prop_assume!(group_name != new_group_name);

        let mut result = ImportResult::new();
        result.add_group(ConnectionGroup::new(group_name.clone()));
        result.add_group(ConnectionGroup::new(new_group_name));

        let existing_groups = vec![ConnectionGroup::new(group_name)];
        let existing_connections: Vec<Connection> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing_connections,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        let (_, _, groups_to_create) = preview.apply();

        // Only the new group should be created
        prop_assert_eq!(groups_to_create.len(), 1);
    }

    /// Property: summary contains all counts
    #[test]
    fn summary_contains_counts(
        count in 1usize..5,
    ) {
        let mut result = ImportResult::new();
        for i in 0..count {
            result.add_connection(create_ssh_connection(
                format!("Server {i}"),
                format!("host{i}.example.com"),
                22,
            ));
        }
        result.add_group(ConnectionGroup::new("Group".to_string()));

        let existing: Vec<Connection> = vec![];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        let summary = preview.summary();
        let expected = format!("New: {count}");
        prop_assert!(summary.contains(&expected));
        prop_assert!(summary.contains("Groups: 1"));
    }

    /// Property: Counts are consistent
    #[test]
    fn counts_are_consistent(
        new_count in 0usize..3,
        dup_count in 0usize..3,
    ) {
        let mut result = ImportResult::new();

        // Add new connections
        for i in 0..new_count {
            result.add_connection(create_ssh_connection(
                format!("New {i}"),
                format!("new{i}.example.com"),
                22,
            ));
        }

        // Add connections that will be duplicates
        for i in 0..dup_count {
            result.add_connection(create_ssh_connection(
                format!("Dup {i}"),
                format!("dup{i}.example.com"),
                22,
            ));
        }

        // Create existing connections for duplicates
        let existing: Vec<Connection> = (0..dup_count)
            .map(|i| create_ssh_connection(
                format!("Existing {i}"),
                format!("dup{i}.example.com"),
                22,
            ))
            .collect();
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        // Verify counts
        prop_assert_eq!(preview.total_connections(), new_count + dup_count);
        prop_assert_eq!(preview.new_connections(), new_count);
        prop_assert_eq!(preview.duplicate_connections(), dup_count);
        prop_assert_eq!(preview.new_count, new_count);
        prop_assert_eq!(preview.skip_count, dup_count);
    }
}

#[test]
fn test_preview_connection_fields() {
    let conn = create_ssh_connection("Test".to_string(), "host.example.com".to_string(), 22);
    let existing_id = Uuid::new_v4();

    let preview_conn = PreviewConnection {
        connection: conn.clone(),
        is_duplicate: true,
        existing_id: Some(existing_id),
        suggested_action: DuplicateAction::Update,
    };

    assert!(preview_conn.is_duplicate);
    assert_eq!(preview_conn.existing_id, Some(existing_id));
    assert_eq!(preview_conn.suggested_action, DuplicateAction::Update);
}

#[test]
fn test_preview_group_fields() {
    let group = ConnectionGroup::new("Test Group".to_string());
    let existing_id = Uuid::new_v4();

    let preview_group = PreviewGroup {
        group: group.clone(),
        is_duplicate: true,
        existing_id: Some(existing_id),
    };

    assert!(preview_group.is_duplicate);
    assert_eq!(preview_group.existing_id, Some(existing_id));
}

#[test]
fn test_import_preview_new() {
    let preview = ImportPreview::new("ssh_config", "/home/user/.ssh/config");

    assert_eq!(preview.source_id, "ssh_config");
    assert_eq!(preview.source_path, "/home/user/.ssh/config");
    assert_eq!(preview.total_connections(), 0);
    assert_eq!(preview.total_groups(), 0);
}

#[test]
fn test_duplicate_action_equality() {
    assert_eq!(DuplicateAction::Skip, DuplicateAction::Skip);
    assert_eq!(DuplicateAction::Update, DuplicateAction::Update);
    assert_eq!(
        DuplicateAction::CreateDuplicate,
        DuplicateAction::CreateDuplicate
    );
    assert_ne!(DuplicateAction::Skip, DuplicateAction::Update);
}

#[test]
fn test_merge_strategy_default() {
    let default = MergeStrategy::default();
    assert_eq!(default, MergeStrategy::SkipExisting);
}

#[test]
fn test_apply_with_skip_action() {
    let mut result = ImportResult::new();
    result.add_connection(create_ssh_connection(
        "Server".to_string(),
        "host.example.com".to_string(),
        22,
    ));

    let existing = vec![create_ssh_connection(
        "Existing".to_string(),
        "host.example.com".to_string(),
        22,
    )];
    let existing_groups: Vec<ConnectionGroup> = vec![];

    let preview = ImportPreview::from_result(
        &result,
        &existing,
        &existing_groups,
        MergeStrategy::SkipExisting,
        "test",
        "test.yml",
    );

    let (to_create, to_update, _) = preview.apply();

    // Skipped connections should not appear in either list
    assert!(to_create.is_empty());
    assert!(to_update.is_empty());
}

#[test]
fn test_different_ports_not_duplicates() {
    let mut result = ImportResult::new();
    result.add_connection(create_ssh_connection(
        "Server".to_string(),
        "host.example.com".to_string(),
        22,
    ));

    let existing = vec![create_ssh_connection(
        "Existing".to_string(),
        "host.example.com".to_string(),
        2222, // Different port
    )];
    let existing_groups: Vec<ConnectionGroup> = vec![];

    let preview = ImportPreview::from_result(
        &result,
        &existing,
        &existing_groups,
        MergeStrategy::SkipExisting,
        "test",
        "test.yml",
    );

    // Should not be a duplicate because port is different
    assert_eq!(preview.duplicate_connections(), 0);
    assert_eq!(preview.new_connections(), 1);
}
