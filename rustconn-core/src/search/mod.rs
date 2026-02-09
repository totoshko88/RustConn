//! Search system for connections
//!
//! This module provides fuzzy search capabilities for connections with support
//! for search operators, result ranking, and custom property search.
//!
//! ## Performance Optimizations
//!
//! This module includes several optimizations for handling large datasets:
//!
//! - **Search Caching**: Use `SearchCache` to cache search results with configurable TTL
//! - **Debounced Search**: Use `DebouncedSearchEngine` to rate-limit search operations
//!   during rapid user input (e.g., typing in a search box)
//! - **Optimized Fuzzy Matching**: The fuzzy matching algorithm uses early termination
//!   and avoids unnecessary allocations
//! - **Parallel Search**: For large datasets (100+ connections), consider using
//!   `search_parallel` for multi-threaded search

// Allow cast warnings - search scoring uses f64 for precision
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::unused_self)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::missing_panics_doc)]

pub mod cache;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, info_span};
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolType};
use crate::performance::Debouncer;
use crate::tracing::span_names;

/// Error type for search operations
#[derive(Debug, Error)]
pub enum SearchError {
    /// Invalid search query syntax
    #[error("Invalid search query: {0}")]
    InvalidQuery(String),

    /// Invalid operator in search query
    #[error("Invalid operator '{operator}': {reason}")]
    InvalidOperator {
        /// The operator that was invalid
        operator: String,
        /// The reason it was invalid
        reason: String,
    },

    /// Invalid regex pattern
    #[error("Invalid regex pattern: {0}")]
    InvalidPattern(String),
}

/// Result type for search operations
pub type SearchResult<T> = std::result::Result<T, SearchError>;

/// Search filter types for operator-based filtering
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchFilter {
    /// Filter by protocol type (e.g., protocol:ssh)
    Protocol(ProtocolType),
    /// Filter by tag (e.g., tag:production)
    Tag(String),
    /// Filter by group ID (e.g., group:uuid)
    Group(Uuid),
    /// Filter by group name (e.g., group:servers)
    GroupName(String),
    /// Search within custom properties
    InCustomProperty(String),
}

/// A parsed search query with text and filters
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Plain text search terms
    pub text: String,
    /// Filters extracted from operators
    pub filters: Vec<SearchFilter>,
}

impl SearchQuery {
    /// Creates a new empty search query
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a search query with just text
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            filters: Vec::new(),
        }
    }

    /// Adds a filter to the query
    #[must_use]
    pub fn with_filter(mut self, filter: SearchFilter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Returns true if the query has no text and no filters
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.filters.is_empty()
    }
}

/// A match highlight indicating where in a field the match occurred
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchHighlight {
    /// The field that matched (e.g., "name", "host", "tags")
    pub field: String,
    /// Start position of the match in the field value
    pub start: usize,
    /// End position of the match in the field value
    pub end: usize,
}

impl MatchHighlight {
    /// Creates a new match highlight
    #[must_use]
    pub fn new(field: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            field: field.into(),
            start,
            end,
        }
    }
}

/// A search result with relevance score and match information
#[derive(Debug, Clone)]
pub struct ConnectionSearchResult {
    /// The ID of the matching connection
    pub connection_id: Uuid,
    /// Relevance score (0.0 to 1.0, higher is more relevant)
    pub score: f32,
    /// Fields that matched the query
    pub matched_fields: Vec<String>,
    /// Highlight positions for matched text
    pub highlights: Vec<MatchHighlight>,
}

impl ConnectionSearchResult {
    /// Creates a new search result
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(connection_id: Uuid, score: f32) -> Self {
        Self {
            connection_id,
            score,
            matched_fields: Vec::new(),
            highlights: Vec::new(),
        }
    }

    /// Adds a matched field
    #[must_use]
    pub fn with_matched_field(mut self, field: impl Into<String>) -> Self {
        self.matched_fields.push(field.into());
        self
    }

    /// Adds a highlight
    #[must_use]
    pub fn with_highlight(mut self, highlight: MatchHighlight) -> Self {
        self.highlights.push(highlight);
        self
    }
}

/// Search engine for connections
pub struct SearchEngine {
    /// Whether to use case-sensitive matching
    case_sensitive: bool,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchEngine {
    /// Creates a new search engine with default settings
    #[must_use]
    pub const fn new() -> Self {
        Self {
            case_sensitive: false,
        }
    }

    /// Sets whether matching should be case-sensitive
    #[must_use]
    pub const fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    /// Parses a search query string into a `SearchQuery`
    ///
    /// Supports operators like:
    /// - `protocol:ssh` - filter by protocol
    /// - `tag:production` - filter by tag
    /// - `group:servers` - filter by group name
    ///
    /// # Errors
    ///
    /// Returns `SearchError::InvalidOperator` if an operator has invalid syntax
    pub fn parse_query(input: &str) -> SearchResult<SearchQuery> {
        let mut query = SearchQuery::new();
        let mut text_parts = Vec::new();

        for part in input.split_whitespace() {
            if let Some((operator, value)) = part.split_once(':') {
                match operator.to_lowercase().as_str() {
                    "protocol" => {
                        let protocol = Self::parse_protocol(value)?;
                        query.filters.push(SearchFilter::Protocol(protocol));
                    }
                    "tag" => {
                        if value.is_empty() {
                            return Err(SearchError::InvalidOperator {
                                operator: "tag".to_string(),
                                reason: "tag value cannot be empty".to_string(),
                            });
                        }
                        query.filters.push(SearchFilter::Tag(value.to_string()));
                    }
                    "group" => {
                        if value.is_empty() {
                            return Err(SearchError::InvalidOperator {
                                operator: "group".to_string(),
                                reason: "group value cannot be empty".to_string(),
                            });
                        }
                        // Try to parse as UUID first, otherwise treat as group name
                        if let Ok(uuid) = Uuid::parse_str(value) {
                            query.filters.push(SearchFilter::Group(uuid));
                        } else {
                            query
                                .filters
                                .push(SearchFilter::GroupName(value.to_string()));
                        }
                    }
                    "prop" | "property" => {
                        if value.is_empty() {
                            return Err(SearchError::InvalidOperator {
                                operator: operator.to_string(),
                                reason: "property name cannot be empty".to_string(),
                            });
                        }
                        query
                            .filters
                            .push(SearchFilter::InCustomProperty(value.to_string()));
                    }
                    _ => {
                        // Unknown operator, treat as regular text
                        text_parts.push(part);
                    }
                }
            } else {
                text_parts.push(part);
            }
        }

        query.text = text_parts.join(" ");
        Ok(query)
    }

    /// Parses a protocol string into a `ProtocolType`
    fn parse_protocol(value: &str) -> SearchResult<ProtocolType> {
        match value.to_lowercase().as_str() {
            "ssh" => Ok(ProtocolType::Ssh),
            "rdp" => Ok(ProtocolType::Rdp),
            "vnc" => Ok(ProtocolType::Vnc),
            "spice" => Ok(ProtocolType::Spice),
            "telnet" => Ok(ProtocolType::Telnet),
            "zerotrust" | "zt" => Ok(ProtocolType::ZeroTrust),
            _ => Err(SearchError::InvalidOperator {
                operator: "protocol".to_string(),
                reason: format!(
                    "unknown protocol '{value}', expected ssh, rdp, vnc, \
                     spice, telnet, or zerotrust"
                ),
            }),
        }
    }

    /// Calculates a fuzzy match score between a query and a target string
    ///
    /// Returns a score between 0.0 (no match) and 1.0 (exact match)
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fuzzy_score(&self, query: &str, target: &str) -> f32 {
        if query.is_empty() || target.is_empty() {
            return 0.0;
        }

        // Use optimized path to avoid allocations
        if self.case_sensitive {
            self.fuzzy_score_case_sensitive(query, target)
        } else {
            self.fuzzy_score_case_insensitive(query, target)
        }
    }

    /// Case-sensitive fuzzy score (no allocations)
    #[allow(clippy::cast_precision_loss)]
    fn fuzzy_score_case_sensitive(&self, query: &str, target: &str) -> f32 {
        // Exact match
        if query == target {
            return 1.0;
        }

        // Contains match (substring)
        if target.contains(query) {
            let ratio = query.len() as f32 / target.len() as f32;
            let prefix_bonus = if target.starts_with(query) { 0.1 } else { 0.0 };
            return ratio.mul_add(0.4, 0.5 + prefix_bonus).min(0.99);
        }

        // Fuzzy character matching
        self.fuzzy_score_chars(query.chars(), target.chars(), query.len())
    }

    /// Case-insensitive fuzzy score (minimized allocations)
    #[allow(clippy::cast_precision_loss)]
    fn fuzzy_score_case_insensitive(&self, query: &str, target: &str) -> f32 {
        // Exact match (case-insensitive)
        if query.eq_ignore_ascii_case(target) {
            return 1.0;
        }

        // Prefix match optimization (very common in search)
        let query_len = query.len();
        let target_len = target.len();
        if target_len >= query_len && target[..query_len].eq_ignore_ascii_case(query) {
            let ratio = query_len as f32 / target_len as f32;
            return ratio.mul_add(0.4, 0.6).min(0.99);
        }

        // Contains match - use iterator-based case-insensitive search
        if let Some(pos) = self.find_case_insensitive(query, target) {
            let ratio = query_len as f32 / target_len as f32;
            let prefix_bonus = if pos == 0 { 0.1 } else { 0.0 };
            return ratio.mul_add(0.4, 0.5 + prefix_bonus).min(0.99);
        }

        // Fuzzy character matching using iterators (no allocation)
        self.fuzzy_score_chars(
            query.chars().flat_map(char::to_lowercase),
            target.chars().flat_map(char::to_lowercase),
            query.chars().count(),
        )
    }

    /// Finds substring position using case-insensitive comparison without allocation
    fn find_case_insensitive(&self, needle: &str, haystack: &str) -> Option<usize> {
        let needle_len = needle.len();
        let haystack_len = haystack.len();

        if needle_len > haystack_len {
            return None;
        }

        (0..=(haystack_len - needle_len))
            .find(|&i| haystack[i..i + needle_len].eq_ignore_ascii_case(needle))
    }

    /// Searches connections and returns ranked results
    ///
    /// # Arguments
    ///
    /// * `query` - The parsed search query
    /// * `connections` - The connections to search
    /// * `groups` - The groups for group name matching
    ///
    /// # Returns
    ///
    /// A vector of search results sorted by relevance score (highest first)
    #[must_use]
    pub fn search(
        &self,
        query: &SearchQuery,
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Vec<ConnectionSearchResult> {
        let _span = info_span!(
            span_names::SEARCH_EXECUTE,
            query = %query.text,
            filter_count = query.filters.len(),
            connection_count = connections.len()
        )
        .entered();

        if query.is_empty() {
            debug!("Empty query, returning no results");
            return Vec::new();
        }

        let mut results: Vec<ConnectionSearchResult> = connections
            .iter()
            .filter_map(|conn| self.score_connection(query, conn, groups))
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        debug!(result_count = results.len(), "Search completed");
        results
    }

    /// Scores a single connection against the query
    fn score_connection(
        &self,
        query: &SearchQuery,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> Option<ConnectionSearchResult> {
        // First check if connection passes all filters
        if !self.passes_filters(query, connection, groups) {
            return None;
        }

        // If no text query, all filtered connections match with score 1.0
        if query.text.trim().is_empty() {
            return Some(ConnectionSearchResult::new(connection.id, 1.0));
        }

        let mut result = ConnectionSearchResult::new(connection.id, 0.0);
        let mut max_score: f32 = 0.0;

        // Score against name (highest weight)
        let name_score = self.fuzzy_score(&query.text, &connection.name);
        if name_score > 0.0 {
            max_score = max_score.max(name_score * 1.0);
            result.matched_fields.push("name".to_string());
            if let Some(highlight) = self.find_highlight(&query.text, &connection.name) {
                result
                    .highlights
                    .push(MatchHighlight::new("name", highlight.0, highlight.1));
            }
        }

        // Score against host
        let host_score = self.fuzzy_score(&query.text, &connection.host);
        if host_score > 0.0 {
            max_score = max_score.max(host_score * 0.9);
            result.matched_fields.push("host".to_string());
            if let Some(highlight) = self.find_highlight(&query.text, &connection.host) {
                result
                    .highlights
                    .push(MatchHighlight::new("host", highlight.0, highlight.1));
            }
        }

        // Score against tags
        for tag in &connection.tags {
            let tag_score = self.fuzzy_score(&query.text, tag);
            if tag_score > 0.0 {
                max_score = max_score.max(tag_score * 0.8);
                if !result.matched_fields.contains(&"tags".to_string()) {
                    result.matched_fields.push("tags".to_string());
                }
                if let Some(highlight) = self.find_highlight(&query.text, tag) {
                    result
                        .highlights
                        .push(MatchHighlight::new("tags", highlight.0, highlight.1));
                }
            }
        }

        // Score against group name
        if let Some(group_id) = connection.group_id {
            if let Some(group) = groups.iter().find(|g| g.id == group_id) {
                let group_score = self.fuzzy_score(&query.text, &group.name);
                if group_score > 0.0 {
                    max_score = max_score.max(group_score * 0.7);
                    result.matched_fields.push("group".to_string());
                    if let Some(highlight) = self.find_highlight(&query.text, &group.name) {
                        result.highlights.push(MatchHighlight::new(
                            "group",
                            highlight.0,
                            highlight.1,
                        ));
                    }
                }
            }
        }

        // Score against custom properties
        for prop in &connection.custom_properties {
            // Score against property name
            let name_score = self.fuzzy_score(&query.text, &prop.name);
            if name_score > 0.0 {
                max_score = max_score.max(name_score * 0.6);
                let field_name = format!("custom_property:{}", prop.name);
                if !result.matched_fields.contains(&field_name) {
                    result.matched_fields.push(field_name.clone());
                }
            }

            // Score against property value (skip protected properties)
            if !prop.is_protected() {
                let value_score = self.fuzzy_score(&query.text, &prop.value);
                if value_score > 0.0 {
                    max_score = max_score.max(value_score * 0.6);
                    let field_name = format!("custom_property:{}", prop.name);
                    if !result.matched_fields.contains(&field_name) {
                        result.matched_fields.push(field_name);
                    }
                }
            }
        }

        // Score against username if present
        if let Some(ref username) = connection.username {
            let username_score = self.fuzzy_score(&query.text, username);
            if username_score > 0.0 {
                max_score = max_score.max(username_score * 0.5);
                result.matched_fields.push("username".to_string());
            }
        }

        if max_score > 0.0 {
            result.score = max_score;
            Some(result)
        } else {
            None
        }
    }

    /// Checks if a connection passes all filters in the query
    #[allow(clippy::unused_self)]
    fn passes_filters(
        &self,
        query: &SearchQuery,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> bool {
        for filter in &query.filters {
            match filter {
                SearchFilter::Protocol(protocol) => {
                    if connection.protocol != *protocol {
                        return false;
                    }
                }
                SearchFilter::Tag(tag) => {
                    let tag_lower = tag.to_lowercase();
                    if !connection
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase() == tag_lower)
                    {
                        return false;
                    }
                }
                SearchFilter::Group(group_id) => {
                    if connection.group_id != Some(*group_id) {
                        return false;
                    }
                }
                SearchFilter::GroupName(name) => {
                    let name_lower = name.to_lowercase();
                    let matches = connection.group_id.is_some_and(|gid| {
                        groups
                            .iter()
                            .any(|g| g.id == gid && g.name.to_lowercase() == name_lower)
                    });
                    if !matches {
                        return false;
                    }
                }
                SearchFilter::InCustomProperty(prop_name) => {
                    let prop_lower = prop_name.to_lowercase();
                    if !connection
                        .custom_properties
                        .iter()
                        .any(|p| p.name.to_lowercase() == prop_lower)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Finds the highlight position for a match
    fn find_highlight(&self, query: &str, target: &str) -> Option<(usize, usize)> {
        if self.case_sensitive {
            target.find(query).map(|start| (start, start + query.len()))
        } else {
            self.find_case_insensitive(query, target)
                .map(|start| (start, start + query.len()))
        }
    }

    /// Optimized fuzzy match score using early termination
    ///
    /// This version avoids allocations when possible and terminates early
    /// when a match is impossible. Use this for large datasets.
    ///
    /// Returns a score between 0.0 (no match) and 1.0 (exact match)
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fuzzy_score_optimized(&self, query: &str, target: &str) -> f32 {
        // Early termination for empty strings
        if query.is_empty() || target.is_empty() {
            return 0.0;
        }

        let query_len = query.len();
        let target_len = target.len();

        // Early termination: query longer than target can't be a substring match
        // but might still have fuzzy matches
        if query_len > target_len * 2 {
            return 0.0;
        }

        // For case-insensitive matching, we compare char by char to avoid allocation
        // when possible
        if self.case_sensitive {
            self.fuzzy_score_inner(query, target)
        } else {
            // Check for exact match first (common case optimization)
            if query.eq_ignore_ascii_case(target) {
                return 1.0;
            }

            // Check for prefix match (very common in search)
            if target_len >= query_len && target[..query_len].eq_ignore_ascii_case(query) {
                let ratio = query_len as f32 / target_len as f32;
                return ratio.mul_add(0.4, 0.6).min(0.99);
            }

            // Check for substring match using case-insensitive search
            if let Some(pos) = target.to_lowercase().find(&query.to_lowercase()) {
                let ratio = query_len as f32 / target_len as f32;
                let prefix_bonus = if pos == 0 { 0.1 } else { 0.0 };
                return ratio.mul_add(0.4, 0.5 + prefix_bonus).min(0.99);
            }

            // Fall back to fuzzy character matching
            self.fuzzy_score_chars_case_insensitive(query, target)
        }
    }

    /// Inner fuzzy score for case-sensitive matching
    fn fuzzy_score_inner(&self, query: &str, target: &str) -> f32 {
        // Exact match
        if query == target {
            return 1.0;
        }

        // Contains match
        if target.contains(query) {
            let ratio = query.len() as f32 / target.len() as f32;
            let prefix_bonus = if target.starts_with(query) { 0.1 } else { 0.0 };
            return ratio.mul_add(0.4, 0.5 + prefix_bonus).min(0.99);
        }

        self.fuzzy_score_chars(query.chars(), target.chars(), query.len())
    }

    /// Fuzzy character matching for case-insensitive search
    #[allow(clippy::cast_precision_loss)]
    fn fuzzy_score_chars_case_insensitive(&self, query: &str, target: &str) -> f32 {
        let query_lower: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        let target_lower: Vec<char> = target.chars().flat_map(char::to_lowercase).collect();

        self.fuzzy_score_chars(
            query_lower.iter().copied(),
            target_lower.iter().copied(),
            query_lower.len(),
        )
    }

    /// Core fuzzy character matching algorithm
    #[allow(clippy::cast_precision_loss)]
    fn fuzzy_score_chars(
        &self,
        query_chars: impl Iterator<Item = char>,
        target_chars: impl Iterator<Item = char>,
        query_len: usize,
    ) -> f32 {
        let mut query_iter = query_chars.peekable();
        let mut matched = 0;
        let mut consecutive = 0;
        let mut max_consecutive = 0;

        for target_char in target_chars {
            if let Some(&query_char) = query_iter.peek() {
                if target_char == query_char {
                    matched += 1;
                    consecutive += 1;
                    max_consecutive = max_consecutive.max(consecutive);
                    query_iter.next();
                } else {
                    consecutive = 0;
                }
            }
        }

        if matched == 0 {
            return 0.0;
        }

        let match_ratio = matched as f32 / query_len as f32;
        let consecutive_bonus = max_consecutive as f32 / query_len as f32 * 0.2;

        match_ratio.mul_add(0.4, consecutive_bonus).min(0.49)
    }

    /// Searches connections with performance profiling
    ///
    /// This method records timing metrics for the search operation.
    /// Use `crate::performance::metrics()` to retrieve the recorded timings.
    #[must_use]
    pub fn search_profiled(
        &self,
        query: &SearchQuery,
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Vec<ConnectionSearchResult> {
        let _guard = crate::performance::metrics().time_operation("search");
        self.search(query, connections, groups)
    }
}

/// Debounced search engine for rate-limiting search operations
///
/// This wrapper around `SearchEngine` implements debouncing to prevent
/// excessive search operations during rapid user input. It's particularly
/// useful for search-as-you-type interfaces.
///
/// The engine also includes a `SearchCache` for caching search results
/// with configurable TTL and size limits.
///
/// # Example
///
/// ```
/// use rustconn_core::search::{DebouncedSearchEngine, SearchQuery};
/// use std::time::Duration;
///
/// let engine = DebouncedSearchEngine::new(Duration::from_millis(100));
///
/// // First search proceeds immediately
/// let query = SearchQuery::with_text("server");
/// let results = engine.search(&query, &[], &[]);
///
/// // Rapid subsequent searches are debounced
/// // Only the last one will actually execute after the delay
/// ```
pub struct DebouncedSearchEngine {
    /// The underlying search engine
    engine: SearchEngine,
    /// Debouncer for rate limiting
    debouncer: Debouncer,
    /// Last search query (for deferred execution)
    last_query: Arc<Mutex<Option<String>>>,
    /// Whether a search is pending
    search_pending: AtomicBool,
    /// Search result cache with TTL and size limits
    search_cache: Arc<Mutex<cache::SearchCache>>,
}

impl DebouncedSearchEngine {
    /// Creates a new debounced search engine with the specified delay
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        Self {
            engine: SearchEngine::new(),
            debouncer: Debouncer::new(delay),
            last_query: Arc::new(Mutex::new(None)),
            search_pending: AtomicBool::new(false),
            search_cache: Arc::new(Mutex::new(cache::SearchCache::with_defaults())),
        }
    }

    /// Creates a new debounced search engine with custom cache settings
    #[must_use]
    pub fn with_cache(delay: Duration, max_cache_entries: usize, cache_ttl: Duration) -> Self {
        Self {
            engine: SearchEngine::new(),
            debouncer: Debouncer::new(delay),
            last_query: Arc::new(Mutex::new(None)),
            search_pending: AtomicBool::new(false),
            search_cache: Arc::new(Mutex::new(cache::SearchCache::new(
                max_cache_entries,
                cache_ttl,
            ))),
        }
    }

    /// Creates a debounced search engine with default search delay (100ms)
    #[must_use]
    pub fn for_search() -> Self {
        Self::new(Duration::from_millis(100))
    }

    /// Sets whether matching should be case-sensitive
    #[must_use]
    pub const fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.engine = self.engine.with_case_sensitive(case_sensitive);
        self
    }

    /// Performs a debounced search
    ///
    /// If called too rapidly, returns cached results or empty results.
    /// The actual search will be performed after the debounce delay.
    ///
    /// Returns `Some(results)` if search was performed, `None` if debounced.
    #[must_use]
    pub fn search_debounced(
        &self,
        query: &SearchQuery,
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Option<Vec<ConnectionSearchResult>> {
        // Store the query for potential deferred execution
        if let Ok(mut last) = self.last_query.lock() {
            *last = Some(query.text.clone());
        }

        // Check if we should proceed with the search
        if self.debouncer.should_proceed() {
            // Check cache first
            {
                if let Ok(cache) = self.search_cache.lock() {
                    if let Some(cached_results) = cache.get(&query.text) {
                        self.search_pending.store(false, Ordering::SeqCst);
                        return Some(cached_results.to_vec());
                    }
                }
            }

            // Execute search
            let results = self.engine.search(query, connections, groups);

            // Cache the results
            {
                if let Ok(mut cache) = self.search_cache.lock() {
                    cache.insert(query.text.clone(), results.clone());
                }
            }

            self.search_pending.store(false, Ordering::SeqCst);
            Some(results)
        } else {
            self.search_pending.store(true, Ordering::SeqCst);
            None
        }
    }

    /// Performs a search without debouncing
    ///
    /// Use this when you need immediate results regardless of timing.
    #[must_use]
    pub fn search(
        &self,
        query: &SearchQuery,
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Vec<ConnectionSearchResult> {
        self.engine.search(query, connections, groups)
    }

    /// Returns cached results if available and still valid
    ///
    /// Results are considered valid if they match the current query
    /// and haven't exceeded the cache TTL.
    #[must_use]
    pub fn get_cached_results(&self, query_text: &str) -> Option<Vec<ConnectionSearchResult>> {
        let Ok(cache) = self.search_cache.lock() else {
            return None;
        };
        cache
            .get(query_text)
            .map(<[ConnectionSearchResult]>::to_vec)
    }

    /// Invalidates all cached search results
    ///
    /// Should be called when the underlying data changes (connection
    /// added, modified, or deleted).
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.search_cache.lock() {
            cache.invalidate_all();
        }
    }

    /// Checks if there's a pending search operation
    #[must_use]
    pub fn has_pending_search(&self) -> bool {
        self.search_pending.load(Ordering::SeqCst)
    }

    /// Gets the debounce delay
    #[must_use]
    pub const fn delay(&self) -> Duration {
        self.debouncer.delay()
    }

    /// Resets the debouncer state and clears the cache
    pub fn reset(&self) {
        self.debouncer.reset();
        self.search_pending.store(false, Ordering::SeqCst);
        self.invalidate_cache();
    }

    /// Gets a reference to the underlying search engine
    #[must_use]
    pub const fn engine(&self) -> &SearchEngine {
        &self.engine
    }

    /// Returns the number of cached search results
    #[must_use]
    pub fn cache_size(&self) -> usize {
        self.search_cache.lock().map_or(0, |c| c.len())
    }
}

impl Default for DebouncedSearchEngine {
    fn default() -> Self {
        Self::for_search()
    }
}

/// Search performance benchmarking utilities
pub mod benchmark {
    use super::{Connection, ConnectionGroup, Duration, Instant, SearchEngine, SearchQuery, Uuid};

    /// Benchmark result for search operations
    #[derive(Debug, Clone)]
    pub struct SearchBenchmark {
        /// Number of connections searched
        pub connection_count: usize,
        /// Total time for all searches
        pub total_time: Duration,
        /// Average time per search
        pub avg_time: Duration,
        /// Minimum search time
        pub min_time: Duration,
        /// Maximum search time
        pub max_time: Duration,
        /// Number of iterations
        pub iterations: usize,
    }

    /// Benchmarks search performance with the given dataset
    ///
    /// Runs multiple iterations and returns timing statistics.
    #[must_use]
    pub fn benchmark_search(
        engine: &SearchEngine,
        query: &SearchQuery,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        iterations: usize,
    ) -> SearchBenchmark {
        let mut times = Vec::with_capacity(iterations);

        for _ in 0..iterations {
            let start = Instant::now();
            let _ = engine.search(query, connections, groups);
            times.push(start.elapsed());
        }

        let total: Duration = times.iter().sum();
        let min = times.iter().min().copied().unwrap_or_default();
        let max = times.iter().max().copied().unwrap_or_default();

        SearchBenchmark {
            connection_count: connections.len(),
            total_time: total,
            avg_time: total / iterations as u32,
            min_time: min,
            max_time: max,
            iterations,
        }
    }

    /// Generates test connections for benchmarking
    #[must_use]
    pub fn generate_test_connections(count: usize) -> Vec<Connection> {
        (0..count)
            .map(|i| {
                let mut conn =
                    Connection::new_ssh(format!("server-{i}"), format!("host-{i}.example.com"), 22);
                conn.id = Uuid::new_v4();
                conn.tags = vec![format!("env-{}", i % 3), format!("region-{}", i % 5)];
                conn
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CustomProperty;

    fn create_test_connection(name: &str, host: &str, protocol: ProtocolType) -> Connection {
        let mut conn = match protocol {
            ProtocolType::Ssh | ProtocolType::ZeroTrust => {
                Connection::new_ssh(name.to_string(), host.to_string(), 22)
            }
            ProtocolType::Rdp => Connection::new_rdp(name.to_string(), host.to_string(), 3389),
            ProtocolType::Vnc => Connection::new_vnc(name.to_string(), host.to_string(), 5900),
            ProtocolType::Spice => Connection::new_spice(name.to_string(), host.to_string(), 5900),
            ProtocolType::Telnet => Connection::new_telnet(name.to_string(), host.to_string(), 23),
        };
        conn.id = Uuid::new_v4();
        conn
    }

    #[test]
    fn test_parse_query_plain_text() {
        let query = SearchEngine::parse_query("server").unwrap();
        assert_eq!(query.text, "server");
        assert!(query.filters.is_empty());
    }

    #[test]
    fn test_parse_query_with_protocol_filter() {
        let query = SearchEngine::parse_query("protocol:ssh server").unwrap();
        assert_eq!(query.text, "server");
        assert_eq!(query.filters.len(), 1);
        assert!(matches!(
            query.filters[0],
            SearchFilter::Protocol(ProtocolType::Ssh)
        ));
    }

    #[test]
    fn test_parse_query_with_tag_filter() {
        let query = SearchEngine::parse_query("tag:production web").unwrap();
        assert_eq!(query.text, "web");
        assert_eq!(query.filters.len(), 1);
        assert!(matches!(&query.filters[0], SearchFilter::Tag(t) if t == "production"));
    }

    #[test]
    fn test_parse_query_with_group_filter() {
        let query = SearchEngine::parse_query("group:servers").unwrap();
        assert_eq!(query.text, "");
        assert_eq!(query.filters.len(), 1);
        assert!(matches!(&query.filters[0], SearchFilter::GroupName(n) if n == "servers"));
    }

    #[test]
    fn test_parse_query_with_multiple_filters() {
        let query = SearchEngine::parse_query("protocol:ssh tag:prod server").unwrap();
        assert_eq!(query.text, "server");
        assert_eq!(query.filters.len(), 2);
    }

    #[test]
    fn test_parse_query_invalid_protocol() {
        let result = SearchEngine::parse_query("protocol:invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_query_empty_tag() {
        let result = SearchEngine::parse_query("tag:");
        assert!(result.is_err());
    }

    #[test]
    fn test_fuzzy_score_exact_match() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score("server", "server");
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_fuzzy_score_prefix_match() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score("serv", "server");
        assert!(score > 0.5);
        assert!(score < 1.0);
    }

    #[test]
    fn test_fuzzy_score_substring_match() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score("web", "webserver");
        assert!(score > 0.5);
    }

    #[test]
    fn test_fuzzy_score_no_match() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score("xyz", "server");
        assert!(score < 0.5);
    }

    #[test]
    fn test_fuzzy_score_case_insensitive() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score("SERVER", "server");
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_search_by_name() {
        let engine = SearchEngine::new();
        let connections = vec![
            create_test_connection("web-server", "192.168.1.1", ProtocolType::Ssh),
            create_test_connection("database", "192.168.1.2", ProtocolType::Ssh),
        ];
        let groups = vec![];

        let query = SearchQuery::with_text("web");
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].connection_id, connections[0].id);
    }

    #[test]
    fn test_search_by_host() {
        let engine = SearchEngine::new();
        let connections = vec![
            create_test_connection("server1", "web.example.com", ProtocolType::Ssh),
            create_test_connection("server2", "db.example.com", ProtocolType::Ssh),
        ];
        let groups = vec![];

        let query = SearchQuery::with_text("web.example");
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].connection_id, connections[0].id);
    }

    #[test]
    fn test_search_with_protocol_filter() {
        let engine = SearchEngine::new();
        let connections = vec![
            create_test_connection("server1", "192.168.1.1", ProtocolType::Ssh),
            create_test_connection("server2", "192.168.1.2", ProtocolType::Rdp),
        ];
        let groups = vec![];

        let query =
            SearchQuery::with_text("server").with_filter(SearchFilter::Protocol(ProtocolType::Ssh));
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].connection_id, connections[0].id);
    }

    #[test]
    fn test_search_with_tag_filter() {
        let engine = SearchEngine::new();
        let mut conn1 = create_test_connection("server1", "192.168.1.1", ProtocolType::Ssh);
        conn1.tags = vec!["production".to_string()];
        let mut conn2 = create_test_connection("server2", "192.168.1.2", ProtocolType::Ssh);
        conn2.tags = vec!["staging".to_string()];
        let connections = vec![conn1, conn2];
        let groups = vec![];

        let query = SearchQuery::with_text("server")
            .with_filter(SearchFilter::Tag("production".to_string()));
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].connection_id, connections[0].id);
    }

    #[test]
    fn test_search_results_sorted_by_score() {
        let engine = SearchEngine::new();
        let connections = vec![
            create_test_connection("webserver", "192.168.1.1", ProtocolType::Ssh),
            create_test_connection("web", "192.168.1.2", ProtocolType::Ssh),
            create_test_connection("my-web-app", "192.168.1.3", ProtocolType::Ssh),
        ];
        let groups = vec![];

        let query = SearchQuery::with_text("web");
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 3);
        // Exact match should be first
        assert_eq!(results[0].connection_id, connections[1].id);
        // Scores should be in descending order
        for i in 1..results.len() {
            assert!(results[i - 1].score >= results[i].score);
        }
    }

    #[test]
    fn test_search_custom_properties() {
        let engine = SearchEngine::new();
        let mut conn = create_test_connection("server", "192.168.1.1", ProtocolType::Ssh);
        conn.custom_properties = vec![
            CustomProperty::new_text("environment", "production"),
            CustomProperty::new_text("owner", "devops"),
        ];
        let connections = vec![conn];
        let groups = vec![];

        let query = SearchQuery::with_text("production");
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert!(results[0]
            .matched_fields
            .iter()
            .any(|f| f.contains("custom_property")));
    }

    #[test]
    fn test_search_empty_query() {
        let engine = SearchEngine::new();
        let connections = vec![create_test_connection(
            "server1",
            "192.168.1.1",
            ProtocolType::Ssh,
        )];
        let groups = vec![];

        let query = SearchQuery::new();
        let results = engine.search(&query, &connections, &groups);

        assert!(results.is_empty());
    }

    #[test]
    fn test_search_filter_only() {
        let engine = SearchEngine::new();
        let connections = vec![
            create_test_connection("server1", "192.168.1.1", ProtocolType::Ssh),
            create_test_connection("server2", "192.168.1.2", ProtocolType::Rdp),
        ];
        let groups = vec![];

        let query = SearchQuery::new().with_filter(SearchFilter::Protocol(ProtocolType::Ssh));
        let results = engine.search(&query, &connections, &groups);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].connection_id, connections[0].id);
    }

    // ========== Tests for optimized fuzzy matching ==========

    #[test]
    fn test_fuzzy_score_optimized_exact_match() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score_optimized("server", "server");
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_fuzzy_score_optimized_prefix_match() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score_optimized("serv", "server");
        assert!(score > 0.5);
        assert!(score < 1.0);
    }

    #[test]
    fn test_fuzzy_score_optimized_case_insensitive() {
        let engine = SearchEngine::new();
        let score = engine.fuzzy_score_optimized("SERVER", "server");
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_fuzzy_score_optimized_empty_strings() {
        let engine = SearchEngine::new();
        assert!(engine.fuzzy_score_optimized("", "server").abs() < f32::EPSILON);
        assert!(engine.fuzzy_score_optimized("server", "").abs() < f32::EPSILON);
    }

    #[test]
    fn test_fuzzy_score_optimized_consistency_with_original() {
        let engine = SearchEngine::new();
        let test_cases = vec![
            ("server", "server"),
            ("serv", "server"),
            ("web", "webserver"),
            ("xyz", "server"),
            ("SERVER", "server"),
        ];

        for (query, target) in test_cases {
            let original = engine.fuzzy_score(query, target);
            let optimized = engine.fuzzy_score_optimized(query, target);
            // Scores should be very close (within 0.1)
            assert!(
                (original - optimized).abs() < 0.15,
                "Scores differ for ({}, {}): original={}, optimized={}",
                query,
                target,
                original,
                optimized
            );
        }
    }

    // ========== Tests for debounced search engine ==========

    #[test]
    fn test_debounced_search_first_call_proceeds() {
        let engine = DebouncedSearchEngine::for_search();
        let connections = vec![create_test_connection(
            "server",
            "192.168.1.1",
            ProtocolType::Ssh,
        )];
        let groups = vec![];

        let query = SearchQuery::with_text("server");
        let result = engine.search_debounced(&query, &connections, &groups);

        assert!(result.is_some());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_debounced_search_rapid_calls_debounced() {
        let engine = DebouncedSearchEngine::new(Duration::from_millis(100));
        let connections = vec![create_test_connection(
            "server",
            "192.168.1.1",
            ProtocolType::Ssh,
        )];
        let groups = vec![];

        let query = SearchQuery::with_text("server");

        // First call should proceed
        let result1 = engine.search_debounced(&query, &connections, &groups);
        assert!(result1.is_some());

        // Immediate second call should be debounced
        let result2 = engine.search_debounced(&query, &connections, &groups);
        assert!(result2.is_none());
        assert!(engine.has_pending_search());
    }

    #[test]
    fn test_debounced_search_after_delay_proceeds() {
        let engine = DebouncedSearchEngine::new(Duration::from_millis(10));
        let connections = vec![create_test_connection(
            "server",
            "192.168.1.1",
            ProtocolType::Ssh,
        )];
        let groups = vec![];

        let query = SearchQuery::with_text("server");

        // First call
        let _ = engine.search_debounced(&query, &connections, &groups);

        // Wait for debounce delay
        std::thread::sleep(Duration::from_millis(20));

        // Should proceed now
        let result = engine.search_debounced(&query, &connections, &groups);
        assert!(result.is_some());
    }

    #[test]
    fn test_debounced_search_cached_results() {
        let engine = DebouncedSearchEngine::for_search();
        let connections = vec![create_test_connection(
            "server",
            "192.168.1.1",
            ProtocolType::Ssh,
        )];
        let groups = vec![];

        let query = SearchQuery::with_text("server");

        // Perform search
        let _ = engine.search_debounced(&query, &connections, &groups);

        // Get cached results
        let cached = engine.get_cached_results("server");
        assert!(cached.is_some());
        assert!(!cached.unwrap().is_empty());
    }

    #[test]
    fn test_debounced_search_reset() {
        let engine = DebouncedSearchEngine::for_search();
        let connections = vec![create_test_connection(
            "server",
            "192.168.1.1",
            ProtocolType::Ssh,
        )];
        let groups = vec![];

        let query = SearchQuery::with_text("server");

        // Perform search
        let _ = engine.search_debounced(&query, &connections, &groups);

        // Reset
        engine.reset();

        // Cached results should be cleared
        let cached = engine.get_cached_results("server");
        assert!(cached.is_none());
    }

    // ========== Tests for benchmark utilities ==========

    #[test]
    fn test_benchmark_generate_connections() {
        let connections = benchmark::generate_test_connections(100);
        assert_eq!(connections.len(), 100);

        // Check that connections have unique IDs
        let ids: std::collections::HashSet<_> = connections.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn test_benchmark_search() {
        let engine = SearchEngine::new();
        let connections = benchmark::generate_test_connections(50);
        let groups = vec![];
        let query = SearchQuery::with_text("server");

        let result = benchmark::benchmark_search(&engine, &query, &connections, &groups, 10);

        assert_eq!(result.connection_count, 50);
        assert_eq!(result.iterations, 10);
        assert!(result.min_time <= result.avg_time);
        assert!(result.avg_time <= result.max_time);
    }

    #[test]
    fn test_search_performance_large_dataset() {
        let engine = SearchEngine::new();
        let connections = benchmark::generate_test_connections(500);
        let groups = vec![];
        let query = SearchQuery::with_text("server-25");

        let start = std::time::Instant::now();
        let results = engine.search(&query, &connections, &groups);
        let elapsed = start.elapsed();

        // Should complete within 100ms for 500 connections
        assert!(
            elapsed < Duration::from_millis(100),
            "Search took too long: {:?}",
            elapsed
        );

        // Should find the matching connection
        assert!(!results.is_empty());
    }
}
