//! Property-based tests for the Search system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Search system (Requirements 14.x, 10.5).

use proptest::prelude::*;
use rustconn_core::{
    Connection, ConnectionGroup, CustomProperty, ProtocolType, SearchEngine, SearchFilter,
    SearchQuery,
};
use uuid::Uuid;

// ========== Strategies ==========

/// Strategy for generating valid search text
fn arb_search_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_-]{1,20}".prop_map(|s| s)
}

/// Strategy for generating protocol types
fn arb_protocol() -> impl Strategy<Value = ProtocolType> {
    prop_oneof![
        Just(ProtocolType::Ssh),
        Just(ProtocolType::Rdp),
        Just(ProtocolType::Vnc),
        Just(ProtocolType::Spice),
    ]
}

/// Strategy for generating protocol names as strings
fn arb_protocol_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ssh".to_string()),
        Just("rdp".to_string()),
        Just("vnc".to_string()),
        Just("spice".to_string()),
    ]
}

/// Strategy for generating tag names
fn arb_tag() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,15}".prop_map(|s| s)
}

/// Strategy for generating group names
fn arb_group_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,15}".prop_map(|s| s.trim().to_string())
}

/// Strategy for generating a test connection
fn arb_connection() -> impl Strategy<Value = Connection> {
    (
        arb_search_text(),
        arb_search_text(),
        arb_protocol(),
        prop::collection::vec(arb_tag(), 0..3),
    )
        .prop_map(|(name, host, protocol, tags)| {
            let mut conn = match protocol {
                ProtocolType::Ssh | ProtocolType::ZeroTrust => Connection::new_ssh(name, host, 22),
                ProtocolType::Rdp => Connection::new_rdp(name, host, 3389),
                ProtocolType::Vnc => Connection::new_vnc(name, host, 5900),
                ProtocolType::Spice => Connection::new_spice(name, host, 5900),
            };
            conn.tags = tags;
            conn
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 29: Search Operator Parsing ==========
    // **Feature: rustconn-enhancements, Property 29: Search Operator Parsing**
    // **Validates: Requirements 14.4**
    //
    // For any search string with operators like "protocol:ssh", parsing should
    // extract the correct filter type and value.

    #[test]
    fn search_operator_parsing_protocol(
        protocol_name in arb_protocol_name(),
        search_text in arb_search_text()
    ) {
        let query_str = format!("protocol:{} {}", protocol_name, search_text);
        let result = SearchEngine::parse_query(&query_str);

        prop_assert!(result.is_ok(), "Parsing should succeed for valid protocol");
        let query = result.unwrap();

        // Should have exactly one protocol filter
        let protocol_filters: Vec<_> = query.filters.iter()
            .filter(|f| matches!(f, SearchFilter::Protocol(_)))
            .collect();
        prop_assert_eq!(protocol_filters.len(), 1, "Should have exactly one protocol filter");

        // The filter should match the input protocol
        let expected_protocol = match protocol_name.as_str() {
            "ssh" => ProtocolType::Ssh,
            "rdp" => ProtocolType::Rdp,
            "vnc" => ProtocolType::Vnc,
            "spice" => ProtocolType::Spice,
            _ => unreachable!(),
        };

        prop_assert!(
            matches!(&protocol_filters[0], SearchFilter::Protocol(p) if *p == expected_protocol),
            "Protocol filter should match input"
        );

        // Text should be preserved
        prop_assert_eq!(query.text, search_text, "Search text should be preserved");
    }


    #[test]
    fn search_operator_parsing_tag(
        tag in arb_tag(),
        search_text in arb_search_text()
    ) {
        let query_str = format!("tag:{} {}", tag, search_text);
        let result = SearchEngine::parse_query(&query_str);

        prop_assert!(result.is_ok(), "Parsing should succeed for valid tag");
        let query = result.unwrap();

        // Should have exactly one tag filter
        let tag_filters: Vec<_> = query.filters.iter()
            .filter(|f| matches!(f, SearchFilter::Tag(_)))
            .collect();
        prop_assert_eq!(tag_filters.len(), 1, "Should have exactly one tag filter");

        // The filter should match the input tag
        prop_assert!(
            matches!(&tag_filters[0], SearchFilter::Tag(t) if t == &tag),
            "Tag filter should match input"
        );

        // Text should be preserved
        prop_assert_eq!(query.text, search_text, "Search text should be preserved");
    }

    #[test]
    fn search_operator_parsing_group(
        group_name in arb_group_name(),
        search_text in arb_search_text()
    ) {
        // Skip empty group names
        prop_assume!(!group_name.is_empty());

        let query_str = format!("group:{} {}", group_name.replace(' ', "_"), search_text);
        let result = SearchEngine::parse_query(&query_str);

        prop_assert!(result.is_ok(), "Parsing should succeed for valid group");
        let query = result.unwrap();

        // Should have exactly one group filter (either Group or GroupName)
        let group_filters: Vec<_> = query.filters.iter()
            .filter(|f| matches!(f, SearchFilter::Group(_) | SearchFilter::GroupName(_)))
            .collect();
        prop_assert_eq!(group_filters.len(), 1, "Should have exactly one group filter");

        // Text should be preserved
        prop_assert_eq!(query.text, search_text, "Search text should be preserved");
    }

    #[test]
    fn search_operator_parsing_multiple_filters(
        protocol_name in arb_protocol_name(),
        tag in arb_tag(),
        search_text in arb_search_text()
    ) {
        let query_str = format!("protocol:{} tag:{} {}", protocol_name, tag, search_text);
        let result = SearchEngine::parse_query(&query_str);

        prop_assert!(result.is_ok(), "Parsing should succeed for multiple filters");
        let query = result.unwrap();

        // Should have both protocol and tag filters
        prop_assert_eq!(query.filters.len(), 2, "Should have two filters");

        let has_protocol = query.filters.iter().any(|f| matches!(f, SearchFilter::Protocol(_)));
        let has_tag = query.filters.iter().any(|f| matches!(f, SearchFilter::Tag(_)));

        prop_assert!(has_protocol, "Should have protocol filter");
        prop_assert!(has_tag, "Should have tag filter");

        // Text should be preserved
        prop_assert_eq!(query.text, search_text, "Search text should be preserved");
    }

    #[test]
    fn search_operator_parsing_unknown_operator_becomes_text(
        unknown_op in "[a-z]{3,8}",
        value in arb_search_text(),
        search_text in arb_search_text()
    ) {
        // Skip known operators
        prop_assume!(!["protocol", "tag", "group", "prop", "property"].contains(&unknown_op.as_str()));

        let query_str = format!("{}:{} {}", unknown_op, value, search_text);
        let result = SearchEngine::parse_query(&query_str);

        prop_assert!(result.is_ok(), "Parsing should succeed for unknown operator");
        let query = result.unwrap();

        // Unknown operator should be treated as text
        let expected_text = format!("{}:{} {}", unknown_op, value, search_text);
        prop_assert_eq!(query.text, expected_text, "Unknown operator should become text");
        prop_assert!(query.filters.is_empty(), "Should have no filters for unknown operator");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 27: Search Fuzzy Matching ==========
    // **Feature: rustconn-enhancements, Property 27: Search Fuzzy Matching**
    // **Validates: Requirements 14.1**
    //
    // For any search query and connection name, fuzzy matching should return
    // a score between 0.0 and 1.0.

    #[test]
    fn search_fuzzy_matching_score_range(
        query in arb_search_text(),
        target in arb_search_text()
    ) {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score(&query, &target);

        prop_assert!(score >= 0.0, "Score should be >= 0.0, got {}", score);
        prop_assert!(score <= 1.0, "Score should be <= 1.0, got {}", score);
    }

    #[test]
    fn search_fuzzy_matching_exact_match_is_one(
        text in arb_search_text()
    ) {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score(&text, &text);

        prop_assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "Exact match should have score 1.0, got {}",
            score
        );
    }

    #[test]
    fn search_fuzzy_matching_case_insensitive(
        text in arb_search_text()
    ) {
        let engine = SearchEngine::new();
        let upper = text.to_uppercase();
        let lower = text.to_lowercase();

        let score = engine.fuzzy_score(&upper, &lower);

        prop_assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "Case-insensitive match should have score 1.0, got {}",
            score
        );
    }

    #[test]
    fn search_fuzzy_matching_prefix_scores_higher_than_suffix(
        prefix in "[a-z]{2,5}",
        suffix in "[a-z]{2,5}"
    ) {
        prop_assume!(prefix != suffix);

        let engine = SearchEngine::new();
        let target = format!("{}{}", prefix, suffix);

        let prefix_score = engine.fuzzy_score(&prefix, &target);
        let suffix_score = engine.fuzzy_score(&suffix, &target);

        // Prefix match should score at least as high as suffix match
        // (due to prefix bonus in the algorithm)
        prop_assert!(
            prefix_score >= suffix_score - 0.1,
            "Prefix score {} should be >= suffix score {} - 0.1",
            prefix_score, suffix_score
        );
    }

    #[test]
    fn search_fuzzy_matching_empty_query_returns_zero(
        target in arb_search_text()
    ) {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score("", &target);

        prop_assert!(
            score.abs() < f32::EPSILON,
            "Empty query should have score 0.0, got {}",
            score
        );
    }

    #[test]
    fn search_fuzzy_matching_empty_target_returns_zero(
        query in arb_search_text()
    ) {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score(&query, "");

        prop_assert!(
            score.abs() < f32::EPSILON,
            "Empty target should have score 0.0, got {}",
            score
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 28: Search Scope Coverage ==========
    // **Feature: rustconn-enhancements, Property 28: Search Scope Coverage**
    // **Validates: Requirements 14.2**
    //
    // For any search query, matching should check name, host, tags, group name,
    // and custom properties.

    #[test]
    fn search_scope_matches_name(
        name in arb_search_text(),
        host in arb_search_text()
    ) {
        prop_assume!(name != host);

        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh(name.clone(), host, 22);
        conn.id = Uuid::new_v4();
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text(&name);
        let results = engine.search(&query, &connections, &groups);

        prop_assert!(!results.is_empty(), "Should find connection by name");
        prop_assert!(
            results[0].matched_fields.contains(&"name".to_string()),
            "Should indicate name field matched"
        );
    }

    #[test]
    fn search_scope_matches_host(
        name in arb_search_text(),
        host in arb_search_text()
    ) {
        prop_assume!(name != host);

        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh(name, host.clone(), 22);
        conn.id = Uuid::new_v4();
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text(&host);
        let results = engine.search(&query, &connections, &groups);

        prop_assert!(!results.is_empty(), "Should find connection by host");
        prop_assert!(
            results[0].matched_fields.contains(&"host".to_string()),
            "Should indicate host field matched"
        );
    }

    #[test]
    fn search_scope_matches_tags(
        name in arb_search_text(),
        tag in arb_tag()
    ) {
        prop_assume!(name != tag);

        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh(name, "example.com".to_string(), 22);
        conn.id = Uuid::new_v4();
        conn.tags = vec![tag.clone()];
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text(&tag);
        let results = engine.search(&query, &connections, &groups);

        prop_assert!(!results.is_empty(), "Should find connection by tag");
        prop_assert!(
            results[0].matched_fields.contains(&"tags".to_string()),
            "Should indicate tags field matched"
        );
    }

    #[test]
    fn search_scope_matches_group_name(
        conn_name in arb_search_text(),
        group_name in arb_group_name()
    ) {
        prop_assume!(!group_name.is_empty());
        prop_assume!(conn_name != group_name);

        let engine = SearchEngine::new();
        let group = ConnectionGroup::new(group_name.clone());
        let mut conn = Connection::new_ssh(conn_name, "example.com".to_string(), 22);
        conn.id = Uuid::new_v4();
        conn.group_id = Some(group.id);
        let connections = vec![conn];
        let groups = vec![group];

        let query = SearchQuery::with_text(&group_name);
        let results = engine.search(&query, &connections, &groups);

        prop_assert!(!results.is_empty(), "Should find connection by group name");
        prop_assert!(
            results[0].matched_fields.contains(&"group".to_string()),
            "Should indicate group field matched"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 30: Search Result Ranking ==========
    // **Feature: rustconn-enhancements, Property 30: Search Result Ranking**
    // **Validates: Requirements 14.3**
    //
    // For any search with multiple results, results should be ordered by
    // descending relevance score.

    #[test]
    fn search_result_ranking_descending_order(
        search_text in arb_search_text(),
        connections in prop::collection::vec(arb_connection(), 2..10)
    ) {
        let engine = SearchEngine::new();
        let groups = vec![];

        let query = SearchQuery::with_text(&search_text);
        let results = engine.search(&query, &connections, &groups);

        // Results should be in descending order by score
        for i in 1..results.len() {
            prop_assert!(
                results[i - 1].score >= results[i].score,
                "Results should be in descending order: {} >= {}",
                results[i - 1].score, results[i].score
            );
        }
    }

    #[test]
    fn search_result_ranking_exact_match_first(
        exact_name in arb_search_text(),
        other_name in arb_search_text()
    ) {
        prop_assume!(exact_name != other_name);
        prop_assume!(!other_name.contains(&exact_name));
        // Case-insensitive check: ensure names are different even ignoring case
        prop_assume!(exact_name.to_lowercase() != other_name.to_lowercase());
        prop_assume!(!other_name.to_lowercase().contains(&exact_name.to_lowercase()));

        let engine = SearchEngine::new();

        // Create two connections: one with exact name match, one without
        let mut conn_exact = Connection::new_ssh(exact_name.clone(), "host1.example.com".to_string(), 22);
        conn_exact.id = Uuid::new_v4();

        let mut conn_other = Connection::new_ssh(other_name, "host2.example.com".to_string(), 22);
        conn_other.id = Uuid::new_v4();

        let connections = vec![conn_other.clone(), conn_exact.clone()]; // Put exact match second
        let groups = vec![];

        let query = SearchQuery::with_text(&exact_name);
        let results = engine.search(&query, &connections, &groups);

        if !results.is_empty() {
            // Exact match should be first
            prop_assert_eq!(
                results[0].connection_id, conn_exact.id,
                "Exact match should be ranked first"
            );
        }
    }

    #[test]
    fn search_result_ranking_all_scores_valid(
        search_text in arb_search_text(),
        connections in prop::collection::vec(arb_connection(), 1..5)
    ) {
        let engine = SearchEngine::new();
        let groups = vec![];

        let query = SearchQuery::with_text(&search_text);
        let results = engine.search(&query, &connections, &groups);

        for result in &results {
            prop_assert!(
                result.score >= 0.0 && result.score <= 1.0,
                "All scores should be in [0.0, 1.0], got {}",
                result.score
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 23: Custom Property Search Inclusion ==========
    // **Feature: rustconn-enhancements, Property 23: Custom Property Search Inclusion**
    // **Validates: Requirements 10.5**
    //
    // For any search query, connections with matching custom property values
    // should be included in results.

    #[test]
    fn search_custom_property_value_included(
        conn_name in arb_search_text(),
        prop_name in arb_search_text(),
        prop_value in arb_search_text()
    ) {
        // Ensure the property value is distinct from connection name
        prop_assume!(conn_name != prop_value);
        prop_assume!(!conn_name.to_lowercase().contains(&prop_value.to_lowercase()));

        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh(conn_name, "example.com".to_string(), 22);
        conn.id = Uuid::new_v4();
        conn.custom_properties = vec![CustomProperty::new_text(&prop_name, &prop_value)];
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text(&prop_value);
        let results = engine.search(&query, &connections, &groups);

        prop_assert!(!results.is_empty(), "Should find connection by custom property value");

        // Should indicate custom property matched
        let has_custom_prop_match = results[0].matched_fields.iter()
            .any(|f| f.starts_with("custom_property:"));
        prop_assert!(
            has_custom_prop_match,
            "Should indicate custom property field matched, got: {:?}",
            results[0].matched_fields
        );
    }

    #[test]
    fn search_custom_property_name_included(
        conn_name in arb_search_text(),
        prop_name in arb_search_text(),
        prop_value in arb_search_text()
    ) {
        // Ensure the property name is distinct from connection name
        prop_assume!(conn_name != prop_name);
        prop_assume!(!conn_name.to_lowercase().contains(&prop_name.to_lowercase()));

        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh(conn_name, "example.com".to_string(), 22);
        conn.id = Uuid::new_v4();
        conn.custom_properties = vec![CustomProperty::new_text(&prop_name, &prop_value)];
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text(&prop_name);
        let results = engine.search(&query, &connections, &groups);

        prop_assert!(!results.is_empty(), "Should find connection by custom property name");
    }

    #[test]
    fn search_protected_property_value_not_searched(
        secret_value in "[x-z]{5,10}"
    ) {
        // Use fixed values that have NO characters in common with x, y, z
        // to ensure we're only testing that the protected VALUE is not searched
        let conn_name = "1234567890";
        let prop_name = "ABCDEFGHIJ";
        let host = "192.168.1.1";

        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh(conn_name.to_string(), host.to_string(), 22);
        conn.id = Uuid::new_v4();
        conn.custom_properties = vec![CustomProperty::new_protected(prop_name, &secret_value)];
        let connections = vec![conn];
        let groups = vec![];

        // Search for the secret value - should NOT find it
        let query = SearchQuery::with_text(&secret_value);
        let results = engine.search(&query, &connections, &groups);

        // Protected property values should not be searchable
        prop_assert!(
            results.is_empty(),
            "Protected property values should not be searchable, but found results for '{}'",
            secret_value
        );
    }

    #[test]
    fn search_with_custom_property_filter(
        conn_name in arb_search_text(),
        prop_name in arb_search_text()
    ) {
        let engine = SearchEngine::new();

        // Connection with the custom property
        let mut conn_with_prop = Connection::new_ssh(conn_name.clone(), "host1.example.com".to_string(), 22);
        conn_with_prop.id = Uuid::new_v4();
        conn_with_prop.custom_properties = vec![CustomProperty::new_text(&prop_name, "some value")];

        // Connection without the custom property
        let mut conn_without_prop = Connection::new_ssh(format!("{}_other", conn_name), "host2.example.com".to_string(), 22);
        conn_without_prop.id = Uuid::new_v4();

        let connections = vec![conn_with_prop.clone(), conn_without_prop];
        let groups = vec![];

        // Search with property filter
        let query = SearchQuery::new()
            .with_filter(SearchFilter::InCustomProperty(prop_name));
        let results = engine.search(&query, &connections, &groups);

        prop_assert_eq!(results.len(), 1, "Should find only connection with custom property");
        prop_assert_eq!(
            results[0].connection_id, conn_with_prop.id,
            "Should find the connection with the custom property"
        );
    }
}

// ========== Unit Tests for Edge Cases ==========

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_search_returns_empty() {
        let engine = SearchEngine::new();
        let connections = vec![Connection::new_ssh(
            "server".to_string(),
            "example.com".to_string(),
            22,
        )];
        let groups = vec![];

        let query = SearchQuery::new();
        let results = engine.search(&query, &connections, &groups);

        assert!(results.is_empty());
    }

    #[test]
    fn test_whitespace_only_search_returns_empty() {
        let engine = SearchEngine::new();
        let connections = vec![Connection::new_ssh(
            "server".to_string(),
            "example.com".to_string(),
            22,
        )];
        let groups = vec![];

        let query = SearchQuery::with_text("   ");
        let results = engine.search(&query, &connections, &groups);

        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_only_search_returns_all_matching() {
        let engine = SearchEngine::new();
        let mut conn1 =
            Connection::new_ssh("server1".to_string(), "host1.example.com".to_string(), 22);
        conn1.id = Uuid::new_v4();
        let mut conn2 =
            Connection::new_rdp("server2".to_string(), "host2.example.com".to_string(), 3389);
        conn2.id = Uuid::new_v4();
        let connections = vec![conn1.clone(), conn2];
        let groups = vec![];

        let query = SearchQuery::new().with_filter(SearchFilter::Protocol(ProtocolType::Ssh));
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].connection_id, conn1.id);
    }

    #[test]
    fn test_invalid_protocol_returns_error() {
        let result = SearchEngine::parse_query("protocol:invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_tag_returns_error() {
        let result = SearchEngine::parse_query("tag:");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_group_returns_error() {
        let result = SearchEngine::parse_query("group:");
        assert!(result.is_err());
    }

    #[test]
    fn test_group_uuid_filter() {
        let group_id = Uuid::new_v4();
        let query_str = format!("group:{}", group_id);
        let result = SearchEngine::parse_query(&query_str);

        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(matches!(&query.filters[0], SearchFilter::Group(id) if *id == group_id));
    }

    #[test]
    fn test_search_highlights_substring() {
        let engine = SearchEngine::new();
        let mut conn = Connection::new_ssh("webserver".to_string(), "example.com".to_string(), 22);
        conn.id = Uuid::new_v4();
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text("web");
        let results = engine.search(&query, &connections, &groups);

        assert!(!results.is_empty());
        assert!(!results[0].highlights.is_empty());

        let name_highlight = results[0].highlights.iter().find(|h| h.field == "name");
        assert!(name_highlight.is_some());
        let highlight = name_highlight.unwrap();
        assert_eq!(highlight.start, 0);
        assert_eq!(highlight.end, 3);
    }
}

// ========== Search Cache Property Tests ==========
// These tests validate the correctness properties for the SearchCache
// as defined in the performance-improvements design document.

use rustconn_core::{ConnectionSearchResult, SearchCache};
use std::time::Duration;

/// Strategy for generating cache query strings
fn arb_cache_query() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_-]{1,30}".prop_map(|s| s)
}

/// Strategy for generating search results
fn arb_search_results() -> impl Strategy<Value = Vec<ConnectionSearchResult>> {
    prop::collection::vec(
        (0.0f32..=1.0f32).prop_map(|score| ConnectionSearchResult::new(Uuid::new_v4(), score)),
        0..10,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 1: Search Cache Round-Trip ==========
    // **Feature: performance-improvements, Property 1: Search Cache Round-Trip**
    // **Validates: Requirements 2.1, 2.2**
    //
    // For any search query and result set, caching the results and retrieving
    // them within TTL SHALL return identical results.

    #[test]
    fn cache_round_trip_preserves_results(
        query in arb_cache_query(),
        results in arb_search_results()
    ) {
        let mut cache = SearchCache::new(100, Duration::from_secs(60));

        // Insert results
        cache.insert(query.clone(), results.clone());

        // Retrieve results
        let cached = cache.get(&query);

        prop_assert!(cached.is_some(), "Cached results should be retrievable");
        let cached_results = cached.unwrap();

        // Verify same length
        prop_assert_eq!(
            cached_results.len(),
            results.len(),
            "Cached results should have same length"
        );

        // Verify each result matches
        for (i, (cached, original)) in cached_results.iter().zip(results.iter()).enumerate() {
            prop_assert_eq!(
                cached.connection_id,
                original.connection_id,
                "Connection ID at index {} should match",
                i
            );
            prop_assert!(
                (cached.score - original.score).abs() < f32::EPSILON,
                "Score at index {} should match: {} vs {}",
                i,
                cached.score,
                original.score
            );
        }
    }

    #[test]
    fn cache_round_trip_same_query_returns_cached(
        query in arb_cache_query(),
        results in arb_search_results()
    ) {
        let mut cache = SearchCache::new(100, Duration::from_secs(60));

        // Insert results
        cache.insert(query.clone(), results.clone());

        // Multiple retrievals should return the same results
        for _ in 0..3 {
            let cached = cache.get(&query);
            prop_assert!(cached.is_some(), "Cached results should be retrievable on repeated access");
            prop_assert_eq!(
                cached.unwrap().len(),
                results.len(),
                "Cached results length should be consistent"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 2: Search Cache Invalidation ==========
    // **Feature: performance-improvements, Property 2: Search Cache Invalidation**
    // **Validates: Requirements 2.3**
    //
    // For any cached search results, calling invalidate_all() SHALL clear
    // all cached entries.

    #[test]
    fn cache_invalidation_clears_all_entries(
        queries in prop::collection::vec(arb_cache_query(), 1..10),
        results in arb_search_results()
    ) {
        let mut cache = SearchCache::new(100, Duration::from_secs(60));

        // Insert multiple entries
        for query in &queries {
            cache.insert(query.clone(), results.clone());
        }

        // Verify entries exist
        prop_assert!(cache.len() > 0, "Cache should have entries before invalidation");

        // Invalidate all
        cache.invalidate_all();

        // Verify cache is empty
        prop_assert!(cache.is_empty(), "Cache should be empty after invalidation");

        // Verify no queries return results
        for query in &queries {
            prop_assert!(
                cache.get(query).is_none(),
                "Query '{}' should not return results after invalidation",
                query
            );
        }
    }

    #[test]
    fn cache_invalidation_allows_new_inserts(
        query in arb_cache_query(),
        results1 in arb_search_results(),
        results2 in arb_search_results()
    ) {
        let mut cache = SearchCache::new(100, Duration::from_secs(60));

        // Insert first results
        cache.insert(query.clone(), results1);

        // Invalidate
        cache.invalidate_all();

        // Insert new results
        cache.insert(query.clone(), results2.clone());

        // Verify new results are cached
        let cached = cache.get(&query);
        prop_assert!(cached.is_some(), "New results should be cached after invalidation");
        prop_assert_eq!(
            cached.unwrap().len(),
            results2.len(),
            "Cached results should match new results"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 3: Search Cache TTL Expiration ==========
    // **Feature: performance-improvements, Property 3: Search Cache TTL Expiration**
    // **Validates: Requirements 2.4**
    //
    // For any cached search result, after TTL expiration, the cache SHALL
    // return None for that query.

    #[test]
    fn cache_ttl_expiration_returns_none(
        query in arb_cache_query(),
        results in arb_search_results()
    ) {
        // Use a very short TTL for testing
        let mut cache = SearchCache::new(100, Duration::from_millis(5));

        // Insert results
        cache.insert(query.clone(), results);

        // Verify results are available immediately
        prop_assert!(
            cache.get(&query).is_some(),
            "Results should be available immediately after insert"
        );

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));

        // Verify results are no longer available
        prop_assert!(
            cache.get(&query).is_none(),
            "Results should not be available after TTL expiration"
        );
    }

    #[test]
    fn cache_ttl_evict_stale_removes_expired(
        queries in prop::collection::vec(arb_cache_query(), 1..5),
        results in arb_search_results()
    ) {
        // Use a very short TTL for testing
        let mut cache = SearchCache::new(100, Duration::from_millis(5));

        // Insert multiple entries
        for query in &queries {
            cache.insert(query.clone(), results.clone());
        }

        let initial_count = cache.len();
        prop_assert!(initial_count > 0, "Cache should have entries");

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));

        // Evict stale entries
        let evicted = cache.evict_stale();

        // All entries should be evicted
        prop_assert_eq!(
            evicted,
            initial_count,
            "All entries should be evicted after TTL expiration"
        );
        prop_assert!(cache.is_empty(), "Cache should be empty after evicting stale entries");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 4: Search Cache Size Limit ==========
    // **Feature: performance-improvements, Property 4: Search Cache Size Limit**
    // **Validates: Requirements 2.5**
    //
    // For any sequence of cache insertions, the cache size SHALL never
    // exceed the configured maximum entries.

    #[test]
    fn cache_size_limit_never_exceeded(
        max_entries in 1usize..20,
        queries in prop::collection::vec(arb_cache_query(), 1..50)
    ) {
        let mut cache = SearchCache::new(max_entries, Duration::from_secs(60));

        // Insert many entries
        for query in &queries {
            cache.insert(query.clone(), vec![]);

            // Verify size limit is never exceeded
            prop_assert!(
                cache.len() <= max_entries,
                "Cache size {} should not exceed max_entries {}",
                cache.len(),
                max_entries
            );
        }
    }

    #[test]
    fn cache_size_limit_evicts_oldest(
        queries in prop::collection::vec(arb_cache_query(), 5..15)
    ) {
        // Ensure unique queries
        let unique_queries: Vec<_> = queries.into_iter()
            .enumerate()
            .map(|(i, q)| format!("{}_{}", q, i))
            .collect();

        let max_entries = 3;
        let mut cache = SearchCache::new(max_entries, Duration::from_secs(60));

        // Insert entries with small delays to ensure ordering
        for query in &unique_queries {
            cache.insert(query.clone(), vec![]);
            std::thread::sleep(Duration::from_millis(1));
        }

        // Cache should be at max capacity
        prop_assert!(
            cache.len() <= max_entries,
            "Cache size {} should not exceed max_entries {}",
            cache.len(),
            max_entries
        );

        // The most recent entries should still be in the cache
        // (older entries should have been evicted)
        let recent_queries: Vec<_> = unique_queries.iter().rev().take(max_entries).collect();
        for query in &recent_queries {
            prop_assert!(
                cache.get(query).is_some(),
                "Recent query '{}' should still be in cache",
                query
            );
        }
    }
}

// ========== Debounce Property Tests ==========
// These tests validate the correctness properties for the Debouncer
// as defined in the performance-improvements design document.

use rustconn_core::Debouncer;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 13: Debounce Behavior ==========
    // **Feature: performance-improvements, Property 13: Debounce Behavior**
    // **Validates: Requirements 7.1, 7.2, 7.3**
    //
    // For any sequence of rapid inputs, only the final input after debounce
    // delay SHALL trigger execution.

    #[test]
    fn debounce_first_call_always_proceeds(
        delay_ms in 10u64..500u64
    ) {
        let debouncer = Debouncer::new(Duration::from_millis(delay_ms));

        // First call should always proceed
        prop_assert!(
            debouncer.should_proceed(),
            "First call to should_proceed() should return true"
        );
    }

    #[test]
    fn debounce_rapid_calls_blocked(
        delay_ms in 50u64..200u64,
        rapid_call_count in 2usize..10usize
    ) {
        let debouncer = Debouncer::new(Duration::from_millis(delay_ms));

        // First call proceeds
        prop_assert!(debouncer.should_proceed(), "First call should proceed");

        // Rapid subsequent calls should be blocked
        let mut blocked_count = 0;
        for _ in 0..rapid_call_count {
            if !debouncer.should_proceed() {
                blocked_count += 1;
            }
        }

        // At least some calls should be blocked (all if truly rapid)
        prop_assert!(
            blocked_count > 0,
            "Rapid calls should be blocked, but {} of {} proceeded",
            rapid_call_count - blocked_count,
            rapid_call_count
        );
    }

    #[test]
    fn debounce_after_delay_proceeds(
        delay_ms in 10u64..100u64
    ) {
        let debouncer = Debouncer::new(Duration::from_millis(delay_ms));

        // First call proceeds
        prop_assert!(debouncer.should_proceed(), "First call should proceed");

        // Wait for delay to expire
        std::thread::sleep(Duration::from_millis(delay_ms + 10));

        // Call after delay should proceed
        prop_assert!(
            debouncer.should_proceed(),
            "Call after delay should proceed"
        );
    }

    #[test]
    fn debounce_pending_flag_set_when_blocked(
        delay_ms in 50u64..200u64
    ) {
        let debouncer = Debouncer::new(Duration::from_millis(delay_ms));

        // First call proceeds, no pending
        prop_assert!(debouncer.should_proceed(), "First call should proceed");
        prop_assert!(!debouncer.has_pending(), "No pending after first call");

        // Second rapid call is blocked and sets pending
        let blocked = !debouncer.should_proceed();
        if blocked {
            prop_assert!(
                debouncer.has_pending(),
                "Pending flag should be set when call is blocked"
            );
        }
    }

    #[test]
    fn debounce_reset_clears_state(
        delay_ms in 50u64..200u64
    ) {
        let debouncer = Debouncer::new(Duration::from_millis(delay_ms));

        // First call proceeds
        prop_assert!(debouncer.should_proceed(), "First call should proceed");

        // Second rapid call is blocked
        let _ = debouncer.should_proceed();

        // Reset the debouncer
        debouncer.reset();

        // After reset, next call should proceed immediately
        prop_assert!(
            debouncer.should_proceed(),
            "Call after reset should proceed immediately"
        );

        // Pending flag should be cleared
        prop_assert!(
            !debouncer.has_pending(),
            "Pending flag should be cleared after reset"
        );
    }

    #[test]
    fn debounce_for_search_has_100ms_delay(_dummy in 0..1i32) {
        let debouncer = Debouncer::for_search();

        // Verify the delay is 100ms
        prop_assert_eq!(
            debouncer.delay(),
            Duration::from_millis(100),
            "for_search() should create debouncer with 100ms delay"
        );
    }

    #[test]
    fn debounce_mark_pending_sets_flag(_dummy in 0..1i32) {
        let debouncer = Debouncer::for_search();

        // Initially no pending
        prop_assert!(!debouncer.has_pending(), "Initially no pending");

        // Mark pending
        debouncer.mark_pending();

        // Now has pending
        prop_assert!(
            debouncer.has_pending(),
            "has_pending() should return true after mark_pending()"
        );
    }
}

#[cfg(test)]
mod debounce_edge_case_tests {
    use super::*;

    #[test]
    fn test_debounce_sequence_timing() {
        let debouncer = Debouncer::new(Duration::from_millis(50));

        // First call proceeds
        assert!(debouncer.should_proceed());

        // Rapid calls are blocked
        assert!(!debouncer.should_proceed());
        assert!(!debouncer.should_proceed());

        // Wait for delay
        std::thread::sleep(Duration::from_millis(60));

        // Now should proceed
        assert!(debouncer.should_proceed());
    }

    #[test]
    fn test_debounce_for_render_has_16ms_delay() {
        let debouncer = Debouncer::for_render();
        assert_eq!(debouncer.delay(), Duration::from_millis(16));
    }

    #[test]
    fn test_debounce_custom_delay() {
        let debouncer = Debouncer::new(Duration::from_millis(250));
        assert_eq!(debouncer.delay(), Duration::from_millis(250));
    }
}
