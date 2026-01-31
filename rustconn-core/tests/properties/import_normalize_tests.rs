//! Property tests for import normalization functionality

use proptest::prelude::*;
use rustconn_core::import::{
    is_valid_hostname, looks_like_hostname, parse_host_port, ImportNormalizer, ImportResult,
    NormalizeOptions,
};
use rustconn_core::models::{
    Connection, ConnectionGroup, ProtocolConfig, SshAuthMethod, SshConfig,
};

// Strategy for generating valid hostnames
fn hostname_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z][a-z0-9-]{0,20}\\.[a-z]{2,6}",
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}",
        "10\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}",
    ]
}

// Strategy for generating valid ports
fn port_strategy() -> impl Strategy<Value = u16> {
    1u16..65535
}

proptest! {
    /// Property: parse_host_port extracts port correctly when present
    #[test]
    fn parse_host_port_extracts_port(
        host in "[a-z][a-z0-9-]{0,20}",
        port in port_strategy(),
    ) {
        let input = format!("{host}:{port}");
        let (parsed_host, parsed_port) = parse_host_port(&input);

        prop_assert_eq!(parsed_host, host);
        prop_assert_eq!(parsed_port, Some(port));
    }

    /// Property: parse_host_port returns None for port when not present
    #[test]
    fn parse_host_port_no_port(host in "[a-z][a-z0-9.-]{1,50}") {
        // Ensure no colon followed by digits at the end
        if !host.contains(':') || host.chars().last().is_none_or(|c| !c.is_ascii_digit()) {
            let (parsed_host, parsed_port) = parse_host_port(&host);
            prop_assert_eq!(parsed_host, host);
            prop_assert!(parsed_port.is_none());
        }
    }

    /// Property: parse_host_port handles IPv6 addresses
    #[test]
    fn parse_host_port_ipv6(
        a in 0u16..65535,
        b in 0u16..65535,
        port in port_strategy(),
    ) {
        let ipv6 = format!("{a:x}:{b:x}::1");
        let input = format!("[{ipv6}]:{port}");
        let (parsed_host, parsed_port) = parse_host_port(&input);

        prop_assert_eq!(parsed_host, ipv6);
        prop_assert_eq!(parsed_port, Some(port));
    }

    /// Property: is_valid_hostname rejects empty strings
    #[test]
    fn is_valid_hostname_rejects_empty(_dummy in 0..1) {
        prop_assert!(!is_valid_hostname(""));
        prop_assert!(!is_valid_hostname("   "));
    }

    /// Property: is_valid_hostname rejects placeholder values
    #[test]
    fn is_valid_hostname_rejects_placeholders(_dummy in 0..1) {
        prop_assert!(!is_valid_hostname("tmp"));
        prop_assert!(!is_valid_hostname("TMP"));
        prop_assert!(!is_valid_hostname("placeholder"));
        prop_assert!(!is_valid_hostname("PLACEHOLDER"));
        prop_assert!(!is_valid_hostname("none"));
        prop_assert!(!is_valid_hostname("NONE"));
    }

    /// Property: is_valid_hostname accepts valid hostnames
    #[test]
    fn is_valid_hostname_accepts_valid(host in hostname_strategy()) {
        prop_assert!(is_valid_hostname(&host));
    }

    /// Property: looks_like_hostname accepts FQDNs
    #[test]
    fn looks_like_hostname_accepts_fqdn(
        subdomain in "[a-z][a-z0-9-]{0,10}",
        domain in "[a-z]{2,10}",
        tld in "[a-z]{2,6}",
    ) {
        let fqdn = format!("{subdomain}.{domain}.{tld}");
        prop_assert!(looks_like_hostname(&fqdn));
    }

    /// Property: looks_like_hostname accepts IP addresses
    #[test]
    fn looks_like_hostname_accepts_ip(
        a in 1u8..255,
        b in 0u8..255,
        c in 0u8..255,
        d in 1u8..255,
    ) {
        let ip = format!("{a}.{b}.{c}.{d}");
        prop_assert!(looks_like_hostname(&ip));
    }

    /// Property: looks_like_hostname accepts variable syntax
    #[test]
    fn looks_like_hostname_accepts_variables(var in "[A-Z][A-Z0-9_]{0,20}") {
        let var_syntax = format!("${{{var}}}");
        prop_assert!(looks_like_hostname(&var_syntax));
    }

    /// Property: looks_like_hostname rejects plain words without dots
    #[test]
    fn looks_like_hostname_rejects_plain_words(word in "[a-z]{3,20}") {
        // Plain words without dots, IPs, or variables should be rejected
        if !word.contains('.') && word.parse::<std::net::IpAddr>().is_err() {
            prop_assert!(!looks_like_hostname(&word));
        }
    }

    /// Property: NormalizeOptions::all enables all options
    #[test]
    fn normalize_options_all_enables_all(_dummy in 0..1) {
        let opts = NormalizeOptions::all();
        prop_assert!(opts.add_source_tag);
        prop_assert!(opts.add_timestamp_tag);
        prop_assert!(opts.validate_key_paths);
        prop_assert!(opts.deduplicate_groups);
        prop_assert!(opts.normalize_auth_method);
        prop_assert!(opts.normalize_ports);
    }

    /// Property: NormalizeOptions::minimal enables essential options only
    #[test]
    fn normalize_options_minimal_essential_only(_dummy in 0..1) {
        let opts = NormalizeOptions::minimal();
        prop_assert!(!opts.add_source_tag);
        prop_assert!(!opts.add_timestamp_tag);
        prop_assert!(!opts.validate_key_paths);
        prop_assert!(opts.deduplicate_groups);
        prop_assert!(opts.normalize_auth_method);
        prop_assert!(opts.normalize_ports);
    }

    /// Property: Normalizer adds source tag when enabled
    #[test]
    fn normalizer_adds_source_tag(
        source_id in "[a-z_]{3,20}",
        conn_name in "[a-zA-Z0-9 ]{1,30}",
        host in hostname_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(Connection::new_ssh(conn_name, host, 22));

        let options = NormalizeOptions {
            add_source_tag: true,
            ..Default::default()
        };
        let normalizer = ImportNormalizer::new(&source_id, options);
        normalizer.normalize(&mut result);

        let expected_tag = format!("imported:{source_id}");
        prop_assert!(result.connections[0].tags.contains(&expected_tag));
    }

    /// Property: Normalizer normalizes port 0 to default
    #[test]
    fn normalizer_fixes_zero_port(
        conn_name in "[a-zA-Z0-9 ]{1,30}",
        host in hostname_strategy(),
    ) {
        let mut result = ImportResult::new();
        result.add_connection(Connection::new_ssh(conn_name, host, 0));

        let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
        normalizer.normalize(&mut result);

        prop_assert_eq!(result.connections[0].port, 22); // SSH default
    }

    /// Property: Normalizer sets auth_method to PublicKey when key_path present
    #[test]
    fn normalizer_sets_auth_method_for_key(
        conn_name in "[a-zA-Z0-9 ]{1,30}",
        host in hostname_strategy(),
        key_path in "/[a-z]+/[a-z_]+",
    ) {
        let mut result = ImportResult::new();
        let ssh_config = SshConfig {
            key_path: Some(std::path::PathBuf::from(&key_path)),
            auth_method: SshAuthMethod::Password,
            ..Default::default()
        };
        let conn = Connection::new(
            conn_name,
            host,
            22,
            ProtocolConfig::Ssh(ssh_config),
        );
        result.add_connection(conn);

        let mut options = NormalizeOptions::minimal();
        options.validate_key_paths = false;
        let normalizer = ImportNormalizer::new("test", options);
        normalizer.normalize(&mut result);

        if let ProtocolConfig::Ssh(ref ssh) = result.connections[0].protocol_config {
            prop_assert_eq!(ssh.auth_method.clone(), SshAuthMethod::PublicKey);
        }
    }

    /// Property: Normalizer deduplicates groups with same name
    #[test]
    fn normalizer_deduplicates_groups(
        group_name in "[a-zA-Z0-9 ]{1,30}",
        count in 2usize..5,
    ) {
        let mut result = ImportResult::new();
        for _ in 0..count {
            result.add_group(ConnectionGroup::new(group_name.clone()));
        }

        let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
        normalizer.normalize(&mut result);

        prop_assert_eq!(result.groups.len(), 1);
        prop_assert_eq!(result.groups[0].name.clone(), group_name);
    }

    /// Property: Normalizer remaps connection group_ids after deduplication
    #[test]
    fn normalizer_remaps_group_ids(
        group_name in "[a-zA-Z0-9 ]{1,30}",
        conn_name in "[a-zA-Z0-9 ]{1,30}",
        host in hostname_strategy(),
    ) {
        let mut result = ImportResult::new();

        let group1 = ConnectionGroup::new(group_name.clone());
        let group1_id = group1.id;
        let group2 = ConnectionGroup::new(group_name.clone());
        let group2_id = group2.id;

        result.add_group(group1);
        result.add_group(group2);

        let mut conn = Connection::new_ssh(conn_name, host, 22);
        conn.group_id = Some(group2_id);
        result.add_connection(conn);

        let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
        normalizer.normalize(&mut result);

        // Connection should now point to the first group
        prop_assert_eq!(result.connections[0].group_id, Some(group1_id));
    }
}

#[test]
fn test_parse_host_port_edge_cases() {
    // Empty string
    let (host, port) = parse_host_port("");
    assert_eq!(host, "");
    assert_eq!(port, None);

    // Just a colon - actual behavior returns ":" as host
    let (host, port) = parse_host_port(":");
    assert_eq!(host, ":");
    assert_eq!(port, None);

    // Port without host
    let (host, port) = parse_host_port(":22");
    assert_eq!(host, "");
    assert_eq!(port, Some(22));

    // Invalid port (not numeric)
    let (host, port) = parse_host_port("host:abc");
    assert_eq!(host, "host:abc");
    assert_eq!(port, None);

    // Port out of range
    let (host, port) = parse_host_port("host:99999");
    assert_eq!(host, "host:99999");
    assert_eq!(port, None);
}

#[test]
fn test_normalize_preserves_non_ssh_connections() {
    let mut result = ImportResult::new();
    let conn = Connection::new_rdp("RDP Server".to_string(), "rdp.example.com".to_string(), 0);
    result.add_connection(conn);

    let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
    normalizer.normalize(&mut result);

    // Port should be normalized to RDP default
    assert_eq!(result.connections[0].port, 3389);
}

#[test]
fn test_normalize_timestamp_tag_format() {
    let mut result = ImportResult::new();
    result.add_connection(Connection::new_ssh(
        "Test".to_string(),
        "host.example.com".to_string(),
        22,
    ));

    let options = NormalizeOptions {
        add_timestamp_tag: true,
        ..Default::default()
    };
    let normalizer = ImportNormalizer::new("test", options);
    normalizer.normalize(&mut result);

    let has_timestamp_tag = result.connections[0]
        .tags
        .iter()
        .any(|t| t.starts_with("imported_at:"));
    assert!(has_timestamp_tag);
}

#[test]
fn test_import_result_merge() {
    let mut result1 = ImportResult::new();
    result1.add_connection(Connection::new_ssh(
        "Server 1".to_string(),
        "host1.example.com".to_string(),
        22,
    ));
    result1.add_group(ConnectionGroup::new("Group 1".to_string()));

    let mut result2 = ImportResult::new();
    result2.add_connection(Connection::new_ssh(
        "Server 2".to_string(),
        "host2.example.com".to_string(),
        22,
    ));
    result2.add_group(ConnectionGroup::new("Group 2".to_string()));

    result1.merge(result2);

    assert_eq!(result1.connections.len(), 2);
    assert_eq!(result1.groups.len(), 2);
}

#[test]
fn test_import_result_summary() {
    let mut result = ImportResult::new();
    result.add_connection(Connection::new_ssh(
        "Server".to_string(),
        "host.example.com".to_string(),
        22,
    ));
    result.add_group(ConnectionGroup::new("Group".to_string()));
    result.add_skipped(rustconn_core::import::SkippedEntry::new(
        "skipped", "reason",
    ));

    let summary = result.summary();
    assert!(summary.contains("Imported: 1"));
    assert!(summary.contains("Groups: 1"));
    assert!(summary.contains("Skipped: 1"));
}

#[test]
fn test_import_result_has_errors() {
    let mut result = ImportResult::new();
    assert!(!result.has_errors());

    result.add_error(rustconn_core::error::ImportError::UnsupportedFormat(
        "test".to_string(),
    ));
    assert!(result.has_errors());
}

#[test]
fn test_import_result_has_skipped() {
    let mut result = ImportResult::new();
    assert!(!result.has_skipped());

    result.add_skipped(rustconn_core::import::SkippedEntry::new("id", "reason"));
    assert!(result.has_skipped());
}

#[test]
fn test_skipped_entry_with_location() {
    let entry =
        rustconn_core::import::SkippedEntry::with_location("identifier", "reason", "file.txt:10");

    assert_eq!(entry.identifier, "identifier");
    assert_eq!(entry.reason, "reason");
    assert_eq!(entry.location, Some("file.txt:10".to_string()));
}
