//! Expect-style automation for interactive prompts
//!
//! This module provides expect-style pattern matching for automating interactive
//! terminal prompts. It supports:
//! - Regex pattern matching against terminal output
//! - Automatic response sending when patterns match
//! - Priority-based rule ordering
//! - Timeout handling for patterns

use std::sync::Arc;
use std::time::Instant;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::variables::{VariableManager, VariableScope};

/// Errors that can occur during expect operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExpectError {
    /// Invalid regex pattern
    #[error("Invalid regex pattern: {0}")]
    InvalidPattern(String),

    /// Pattern compilation failed
    #[error("Failed to compile pattern '{pattern}': {reason}")]
    PatternCompilationFailed {
        /// The pattern that failed to compile
        pattern: String,
        /// The reason for the failure
        reason: String,
    },

    /// No matching rule found
    #[error("No matching rule found for output")]
    NoMatch,

    /// Variable substitution error
    #[error("Variable error: {0}")]
    VariableError(String),

    /// Rule not found
    #[error("Rule not found: {0}")]
    RuleNotFound(Uuid),

    /// Duplicate rule ID
    #[error("Duplicate rule ID: {0}")]
    DuplicateRuleId(Uuid),
}

/// Result type for expect operations
pub type ExpectResult<T> = std::result::Result<T, ExpectError>;

/// An expect rule with pattern and response
///
/// Expect rules define patterns to match against terminal output and
/// responses to send when a match is found.
#[derive(Clone, Serialize, Deserialize)]
pub struct ExpectRule {
    /// Unique identifier for this rule
    pub id: Uuid,
    /// Regex pattern to match against terminal output
    pub pattern: String,
    /// Response to send when pattern matches (supports variables)
    pub response: String,
    /// Priority for rule ordering (higher = checked first)
    pub priority: i32,
    /// Optional timeout in milliseconds
    pub timeout_ms: Option<u32>,
    /// Whether this rule is enabled
    pub enabled: bool,
    /// Whether this rule should only fire once (default: true)
    #[serde(default = "default_one_shot")]
    pub one_shot: bool,
    /// How long to wait after the match before sending the response, in
    /// milliseconds.
    ///
    /// `None` and `Some(0)` both send immediately. A delay exists for peers that
    /// print their prompt before they are ready to read: network equipment
    /// commonly discards a reply that arrives in the same breath as the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u32>,
}

/// Default value for `one_shot` — true for backward compatibility
const fn default_one_shot() -> bool {
    true
}

impl ExpectRule {
    /// Creates a new expect rule with the given pattern and response
    ///
    /// # Arguments
    ///
    /// * `pattern` - Regex pattern to match
    /// * `response` - Response to send when matched
    #[must_use]
    pub fn new(pattern: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            pattern: pattern.into(),
            response: response.into(),
            priority: 0,
            timeout_ms: None,
            enabled: true,
            one_shot: true,
            delay_ms: None,
        }
    }

    /// Creates a new expect rule with a specific ID
    #[must_use]
    pub fn with_id(id: Uuid, pattern: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            id,
            pattern: pattern.into(),
            response: response.into(),
            priority: 0,
            timeout_ms: None,
            enabled: true,
            one_shot: true,
            delay_ms: None,
        }
    }

    /// Sets the priority for this rule
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the timeout for this rule
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Sets whether this rule is enabled
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets whether this rule fires only once
    #[must_use]
    pub const fn with_one_shot(mut self, one_shot: bool) -> Self {
        self.one_shot = one_shot;
        self
    }

    /// Sets how long to wait after the match before sending the response.
    ///
    /// `0` is stored as "no delay" rather than as a zero-length timer, so a
    /// caller that passes a config default of `0` gets an immediate send instead
    /// of a main-loop round trip.
    #[must_use]
    pub const fn with_delay(mut self, delay_ms: u32) -> Self {
        self.delay_ms = if delay_ms == 0 { None } else { Some(delay_ms) };
        self
    }

    /// Validates the regex pattern
    ///
    /// # Errors
    ///
    /// Returns `ExpectError::PatternCompilationFailed` if the pattern is invalid.
    pub fn validate_pattern(&self) -> ExpectResult<()> {
        Regex::new(&self.pattern).map_err(|e| ExpectError::PatternCompilationFailed {
            pattern: self.pattern.clone(),
            reason: e.to_string(),
        })?;
        Ok(())
    }

    /// Compiles the pattern into a Regex
    ///
    /// # Errors
    ///
    /// Returns `ExpectError::PatternCompilationFailed` if the pattern is invalid.
    pub fn compile_pattern(&self) -> ExpectResult<Regex> {
        Regex::new(&self.pattern).map_err(|e| ExpectError::PatternCompilationFailed {
            pattern: self.pattern.clone(),
            reason: e.to_string(),
        })
    }
}

impl std::fmt::Debug for ExpectRule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExpectRule")
            .field("id", &self.id)
            .field("pattern", &self.pattern)
            .field("response", &"[REDACTED]")
            .field("priority", &self.priority)
            .field("timeout_ms", &self.timeout_ms)
            .field("enabled", &self.enabled)
            .field("one_shot", &self.one_shot)
            .field("delay_ms", &self.delay_ms)
            .finish()
    }
}

impl PartialEq for ExpectRule {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.pattern == other.pattern
            && self.response == other.response
            && self.priority == other.priority
            && self.timeout_ms == other.timeout_ms
            && self.enabled == other.enabled
            && self.one_shot == other.one_shot
            && self.delay_ms == other.delay_ms
    }
}

impl Eq for ExpectRule {}

/// A compiled expect rule with pre-compiled regex pattern
#[derive(Debug, Clone)]
pub struct CompiledRule {
    /// The original rule
    pub rule: ExpectRule,
    /// The compiled regex pattern
    pub regex: Regex,
}

impl CompiledRule {
    /// Creates a new compiled rule from an expect rule
    ///
    /// # Errors
    ///
    /// Returns an error if the pattern fails to compile.
    pub fn new(mut rule: ExpectRule) -> ExpectResult<Self> {
        let regex = match rule.compile_pattern() {
            Ok(regex) => regex,
            Err(error) => {
                rule.response.zeroize();
                return Err(error);
            }
        };
        Ok(Self { rule, regex })
    }

    /// Checks if the output matches this rule's pattern
    #[must_use]
    pub fn matches(&self, output: &str) -> bool {
        self.regex.is_match(output)
    }

    /// Finds the first match in the output
    #[must_use]
    pub fn find<'a>(&self, output: &'a str) -> Option<regex::Match<'a>> {
        self.regex.find(output)
    }
}

/// Expect engine for pattern matching
///
/// The expect engine manages a collection of expect rules and matches
/// terminal output against them, returning the highest priority match.
#[derive(Debug, Clone, Default)]
pub struct ExpectEngine {
    /// Compiled rules sorted by priority (highest first)
    rules: Vec<CompiledRule>,
}

impl ExpectEngine {
    /// Creates a new empty expect engine
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an expect engine from a list of rules
    ///
    /// # Errors
    ///
    /// Returns an error if any rule's pattern fails to compile.
    pub fn from_rules(rules: Vec<ExpectRule>) -> ExpectResult<Self> {
        let mut engine = Self::new();
        let mut rules = rules.into_iter();
        while let Some(rule) = rules.next() {
            if let Err(error) = engine.add_rule(rule) {
                for mut remaining in rules {
                    remaining.response.zeroize();
                }
                return Err(error);
            }
        }
        Ok(engine)
    }

    /// Adds a rule to the engine
    ///
    /// # Errors
    ///
    /// Returns an error if the rule's pattern fails to compile or if
    /// a rule with the same ID already exists.
    pub fn add_rule(&mut self, mut rule: ExpectRule) -> ExpectResult<()> {
        // Check for duplicate ID
        if self.rules.iter().any(|r| r.rule.id == rule.id) {
            let id = rule.id;
            rule.response.zeroize();
            return Err(ExpectError::DuplicateRuleId(id));
        }

        let compiled = CompiledRule::new(rule)?;
        self.rules.push(compiled);
        self.sort_by_priority();
        Ok(())
    }

    /// Removes a rule by ID.
    ///
    /// The returned rule keeps its metadata but its response is scrubbed before
    /// ownership leaves the engine, because it may contain a resolved credential.
    ///
    /// # Errors
    ///
    /// Returns an error if the rule is not found.
    pub fn remove_rule(&mut self, id: Uuid) -> ExpectResult<ExpectRule> {
        let pos = self
            .rules
            .iter()
            .position(|r| r.rule.id == id)
            .ok_or(ExpectError::RuleNotFound(id))?;
        let mut rule = self.rules.remove(pos).rule;
        rule.response.zeroize();
        Ok(rule)
    }

    /// Updates a rule
    ///
    /// # Errors
    ///
    /// Returns an error if the rule is not found or if the new pattern
    /// fails to compile.
    pub fn update_rule(&mut self, mut rule: ExpectRule) -> ExpectResult<()> {
        let Some(pos) = self.rules.iter().position(|r| r.rule.id == rule.id) else {
            let id = rule.id;
            rule.response.zeroize();
            return Err(ExpectError::RuleNotFound(id));
        };

        let compiled = CompiledRule::new(rule)?;
        self.rules[pos].rule.response.zeroize();
        self.rules[pos] = compiled;
        self.sort_by_priority();
        Ok(())
    }

    /// Gets a rule by ID
    #[must_use]
    pub fn get_rule(&self, id: Uuid) -> Option<&ExpectRule> {
        self.rules.iter().find(|r| r.rule.id == id).map(|r| &r.rule)
    }

    /// Gets a mutable reference to a rule by ID.
    ///
    /// Used to zeroize a rule's response before removal so credentials do not
    /// linger in freed memory.
    pub fn get_rule_mut(&mut self, id: Uuid) -> Option<&mut ExpectRule> {
        self.rules
            .iter_mut()
            .find(|r| r.rule.id == id)
            .map(|r| &mut r.rule)
    }

    /// Returns all rules
    #[must_use]
    pub fn rules(&self) -> Vec<&ExpectRule> {
        self.rules.iter().map(|r| &r.rule).collect()
    }

    /// Returns the number of rules
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Returns true if there are no rules
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Sorts rules by priority (highest first)
    fn sort_by_priority(&mut self) {
        self.rules
            .sort_by_key(|a| std::cmp::Reverse(a.rule.priority));
    }

    /// Matches output against all enabled rules, returning the highest priority match
    ///
    /// Rules are checked in priority order (highest first). The first matching
    /// enabled rule is returned.
    #[must_use]
    pub fn match_output(&self, output: &str) -> Option<&ExpectRule> {
        self.rules
            .iter()
            .filter(|r| r.rule.enabled)
            .find(|r| r.matches(output))
            .map(|r| &r.rule)
    }

    /// Matches output and returns the response with variables substituted
    ///
    /// # Errors
    ///
    /// Returns an error if variable substitution fails.
    pub fn match_and_substitute(
        &self,
        output: &str,
        variable_manager: &VariableManager,
        scope: VariableScope,
    ) -> ExpectResult<Option<String>> {
        if let Some(rule) = self.match_output(output) {
            let response = variable_manager
                .substitute_for_command(&rule.response, scope)
                .map_err(|e| ExpectError::VariableError(e.to_string()))?;
            Ok(Some(response))
        } else {
            Ok(None)
        }
    }

    /// Matches output and returns the response with variables substituted (Arc version)
    ///
    /// # Errors
    ///
    /// Returns an error if variable substitution fails.
    pub fn match_and_substitute_arc(
        &self,
        output: &str,
        variable_manager: &Arc<VariableManager>,
        scope: VariableScope,
    ) -> ExpectResult<Option<String>> {
        self.match_and_substitute(output, variable_manager.as_ref(), scope)
    }

    /// Matches a single line against all enabled rules, returning the highest priority match
    ///
    /// Unlike `match_output`, this method also tries matching against the trimmed version
    /// of the line, which is useful for terminal output that may have leading/trailing whitespace.
    #[must_use]
    pub fn match_line(&self, line: &str) -> Option<&CompiledRule> {
        let trimmed = line.trim();
        self.rules
            .iter()
            .filter(|r| r.rule.enabled)
            .find(|r| r.matches(line) || r.matches(trimmed))
    }

    /// Removes a rule by ID without returning an error if not found
    ///
    /// Returns `true` if the rule was removed, `false` if not found.
    pub fn remove_by_id(&mut self, id: Uuid) -> bool {
        if let Some(pos) = self.rules.iter().position(|r| r.rule.id == id) {
            self.rules[pos].rule.response.zeroize();
            self.rules.remove(pos);
            true
        } else {
            false
        }
    }

    /// Removes all rules that have exceeded their timeout relative to `created_at`
    ///
    /// Returns the number of rules removed.
    pub fn remove_expired(&mut self, now: Instant, created_at: Instant) -> usize {
        let before = self.rules.len();
        self.rules.retain_mut(|compiled| {
            let keep = compiled.rule.timeout_ms.is_none_or(|timeout_ms| {
                now.duration_since(created_at)
                    <= std::time::Duration::from_millis(u64::from(timeout_ms))
            });
            if !keep {
                compiled.rule.response.zeroize();
            }
            keep
        });
        before - self.rules.len()
    }

    /// Removes all rules that have exceeded their timeout using per-rule `created_at` timestamps
    ///
    /// Each rule's timeout is checked against its individual creation time from the provided map.
    /// Rules without an entry in the map are kept.
    ///
    /// Returns the number of rules removed.
    pub fn remove_expired_individual(
        &mut self,
        now: Instant,
        created_at_map: &std::collections::HashMap<Uuid, Instant>,
    ) -> usize {
        let before = self.rules.len();
        self.rules.retain_mut(|compiled| {
            let keep = compiled.rule.timeout_ms.is_none_or(|timeout_ms| {
                created_at_map.get(&compiled.rule.id).is_none_or(|created| {
                    now.duration_since(*created)
                        <= std::time::Duration::from_millis(u64::from(timeout_ms))
                })
            });
            if !keep {
                compiled.rule.response.zeroize();
            }
            keep
        });
        before - self.rules.len()
    }

    /// Clears all rules from the engine after scrubbing their responses.
    pub fn clear(&mut self) {
        self.zeroize_responses();
        self.rules.clear();
    }

    /// Overwrites every rule's response with zeroes.
    ///
    /// Called when an automation session is torn down so credentials that were
    /// substituted into responses (issue #257) do not linger in freed memory.
    pub fn zeroize_responses(&mut self) {
        for compiled in &mut self.rules {
            compiled.rule.response.zeroize();
        }
    }
}

impl Drop for ExpectEngine {
    fn drop(&mut self) {
        self.zeroize_responses();
    }
}

/// The response every generated elevated rule carries.
///
/// A placeholder rather than the password itself: the caller merges these rules
/// into the connection's [`crate::models::AutomationConfig`], and the GUI's
/// `prepare_rules_from_config` then resolves `${password}` from the credential
/// cache into a `Zeroizing` buffer. Handing the plaintext in here instead would
/// put a second, unscrubbed copy in every rule for the life of the session.
pub const ELEVATED_RESPONSE_TEMPLATE: &str = "${password}\\n";

/// The priority generated elevated rules are given.
///
/// Above the default `0` so a prompt that also matches a user's own rule is
/// answered with the credential rather than with whatever that rule sends, and
/// well below `i32::MAX` so a deliberate override is still possible.
const ELEVATED_PRIORITY: i32 = 100;

/// Builds expect rules that answer privilege-escalation prompts (sudo, su, doas).
///
/// One rule per configured prompt pattern, each responding with the
/// [`ELEVATED_RESPONSE_TEMPLATE`] placeholder and carrying the configured delay.
/// The rules are **not** one-shot, because a session runs `sudo` more than once;
/// the consumer is expected to match them against the active prompt line only, so
/// a prompt still visible further up the screen cannot re-trigger them.
///
/// Returns an empty vector when elevated credentials are disabled, and skips any
/// pattern that is not a valid regex — an unusable pattern must not take the
/// whole engine down with it, since [`ExpectEngine::from_rules`] fails as a unit.
#[must_use]
pub fn elevated_credentials_rules(
    elevated: &crate::models::ElevatedCredentials,
) -> Vec<ExpectRule> {
    if !elevated.enabled {
        return Vec::new();
    }

    elevated
        .effective_prompts()
        .into_iter()
        .filter_map(|prompt| {
            let rule = ExpectRule::new(&prompt, ELEVATED_RESPONSE_TEMPLATE)
                .with_priority(ELEVATED_PRIORITY)
                .with_one_shot(false)
                .with_delay(elevated.effective_delay_ms());
            match rule.validate_pattern() {
                Ok(()) => Some(rule),
                Err(error) => {
                    tracing::warn!(
                        pattern = %prompt,
                        %error,
                        "Skipping elevated-credentials prompt pattern: not a valid regex"
                    );
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expect_rule_creation() {
        let rule = ExpectRule::new("password:", "secret123");
        assert_eq!(rule.pattern, "password:");
        assert_eq!(rule.response, "secret123");
        assert_eq!(rule.priority, 0);
        assert!(rule.enabled);
        assert!(rule.timeout_ms.is_none());
    }

    #[test]
    fn debug_redacts_response() {
        let rule = ExpectRule::new("password:", "hunter2");
        let rendered = format!("{rule:?}");
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn test_expect_rule_with_priority() {
        let rule = ExpectRule::new("prompt", "response").with_priority(10);
        assert_eq!(rule.priority, 10);
    }

    #[test]
    fn test_expect_rule_with_timeout() {
        let rule = ExpectRule::new("prompt", "response").with_timeout(5000);
        assert_eq!(rule.timeout_ms, Some(5000));
    }

    #[test]
    fn test_expect_rule_validate_valid_pattern() {
        let rule = ExpectRule::new(r"password:\s*$", "secret");
        assert!(rule.validate_pattern().is_ok());
    }

    #[test]
    fn test_expect_rule_validate_invalid_pattern() {
        let rule = ExpectRule::new(r"[invalid", "response");
        let result = rule.validate_pattern();
        assert!(matches!(
            result,
            Err(ExpectError::PatternCompilationFailed { .. })
        ));
    }

    #[test]
    fn test_expect_engine_add_rule() {
        let mut engine = ExpectEngine::new();
        let rule = ExpectRule::new("test", "response");
        assert!(engine.add_rule(rule).is_ok());
        assert_eq!(engine.len(), 1);
    }

    #[test]
    fn test_expect_engine_duplicate_id() {
        let mut engine = ExpectEngine::new();
        let id = Uuid::new_v4();
        let rule1 = ExpectRule::with_id(id, "test1", "response1");
        let rule2 = ExpectRule::with_id(id, "test2", "response2");

        assert!(engine.add_rule(rule1).is_ok());
        assert!(matches!(
            engine.add_rule(rule2),
            Err(ExpectError::DuplicateRuleId(_))
        ));
    }

    #[test]
    fn test_expect_engine_match_output() {
        let mut engine = ExpectEngine::new();
        engine
            .add_rule(ExpectRule::new("password:", "secret"))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("username:", "admin"))
            .unwrap();

        let result = engine.match_output("Enter password:");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "secret");

        let result = engine.match_output("Enter username:");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "admin");

        let result = engine.match_output("No match here");
        assert!(result.is_none());
    }

    #[test]
    fn test_expect_engine_priority_ordering() {
        let mut engine = ExpectEngine::new();

        // Add rules with different priorities
        engine
            .add_rule(ExpectRule::new("prompt", "low").with_priority(1))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("prompt", "high").with_priority(10))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("prompt", "medium").with_priority(5))
            .unwrap();

        // Should match highest priority
        let result = engine.match_output("prompt");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "high");
    }

    #[test]
    fn test_expect_engine_disabled_rules() {
        let mut engine = ExpectEngine::new();
        engine.add_rule(ExpectRule::new("test", "enabled")).unwrap();
        engine
            .add_rule(ExpectRule::new("test", "disabled").with_enabled(false))
            .unwrap();

        let result = engine.match_output("test");
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "enabled");
    }

    #[test]
    fn test_expect_engine_remove_rule() {
        let mut engine = ExpectEngine::new();
        let rule = ExpectRule::new("test", "response");
        let id = rule.id;

        engine.add_rule(rule).unwrap();
        assert_eq!(engine.len(), 1);

        let removed = engine.remove_rule(id).unwrap();
        assert_eq!(removed.pattern, "test");
        assert_eq!(engine.len(), 0);
    }

    #[test]
    fn test_expect_engine_update_rule() {
        let mut engine = ExpectEngine::new();
        let rule = ExpectRule::new("old", "response");
        let id = rule.id;

        engine.add_rule(rule).unwrap();

        let updated = ExpectRule::with_id(id, "new", "updated");
        engine.update_rule(updated).unwrap();

        let rule = engine.get_rule(id).unwrap();
        assert_eq!(rule.pattern, "new");
        assert_eq!(rule.response, "updated");
    }

    #[test]
    fn test_expect_engine_regex_patterns() {
        let mut engine = ExpectEngine::new();
        engine
            .add_rule(ExpectRule::new(r"password:\s*$", "secret"))
            .unwrap();
        engine
            .add_rule(ExpectRule::new(r"\[sudo\].*password", "sudopass"))
            .unwrap();

        assert!(engine.match_output("password: ").is_some());
        assert!(engine.match_output("[sudo] password for user:").is_some());
        assert!(engine.match_output("no match").is_none());
    }

    #[test]
    fn test_expect_engine_from_rules() {
        let rules = vec![
            ExpectRule::new("pattern1", "response1"),
            ExpectRule::new("pattern2", "response2"),
        ];

        let engine = ExpectEngine::from_rules(rules).unwrap();
        assert_eq!(engine.len(), 2);
    }

    #[test]
    fn test_expect_engine_from_rules_invalid_pattern() {
        let rules = vec![
            ExpectRule::new("valid", "response"),
            ExpectRule::new("[invalid", "response"),
        ];

        let result = ExpectEngine::from_rules(rules);
        assert!(matches!(
            result,
            Err(ExpectError::PatternCompilationFailed { .. })
        ));
    }

    #[test]
    fn test_expect_rule_serialization() {
        let rule = ExpectRule::new("pattern", "response")
            .with_priority(5)
            .with_timeout(1000);

        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: ExpectRule = serde_json::from_str(&json).unwrap();

        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_match_line_with_whitespace() {
        let mut engine = ExpectEngine::new();
        engine
            .add_rule(ExpectRule::new("password:", "secret"))
            .unwrap();

        // Should match even with leading/trailing whitespace
        let result = engine.match_line("   password:   ");
        assert!(result.is_some());
        assert_eq!(result.unwrap().rule.response, "secret");

        // Should match exact line too
        let result = engine.match_line("password:");
        assert!(result.is_some());
    }

    #[test]
    fn test_match_line_priority() {
        let mut engine = ExpectEngine::new();
        engine
            .add_rule(ExpectRule::new("prompt", "low").with_priority(1))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("prompt", "high").with_priority(10))
            .unwrap();

        let result = engine.match_line("prompt");
        assert!(result.is_some());
        assert_eq!(result.unwrap().rule.response, "high");
    }

    #[test]
    fn test_remove_by_id() {
        let mut engine = ExpectEngine::new();
        let rule = ExpectRule::new("test", "response");
        let id = rule.id;

        engine.add_rule(rule).unwrap();
        assert_eq!(engine.len(), 1);

        assert!(engine.remove_by_id(id));
        assert_eq!(engine.len(), 0);

        // Removing again returns false
        assert!(!engine.remove_by_id(id));
    }

    #[test]
    fn test_remove_expired() {
        let mut engine = ExpectEngine::new();
        engine
            .add_rule(ExpectRule::new("fast", "r1").with_timeout(100))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("slow", "r2").with_timeout(10_000))
            .unwrap();
        engine
            .add_rule(ExpectRule::new("forever", "r3")) // no timeout
            .unwrap();

        let created = Instant::now();
        // Simulate 200ms elapsed
        let now = created + std::time::Duration::from_millis(200);

        let removed = engine.remove_expired(now, created);
        assert_eq!(removed, 1); // "fast" expired
        assert_eq!(engine.len(), 2); // "slow" and "forever" remain
    }

    #[test]
    fn test_remove_expired_individual() {
        use std::collections::HashMap;

        let mut engine = ExpectEngine::new();
        let rule1 = ExpectRule::new("old", "r1").with_timeout(100);
        let rule2 = ExpectRule::new("new", "r2").with_timeout(100);
        let id1 = rule1.id;
        let id2 = rule2.id;

        engine.add_rule(rule1).unwrap();
        engine.add_rule(rule2).unwrap();

        let base = Instant::now();
        let mut created_map = HashMap::new();
        // rule1 was created 200ms ago
        created_map.insert(
            id1,
            base.checked_sub(std::time::Duration::from_millis(200))
                .unwrap(),
        );
        // rule2 was created 50ms ago
        created_map.insert(
            id2,
            base.checked_sub(std::time::Duration::from_millis(50))
                .unwrap(),
        );

        let removed = engine.remove_expired_individual(base, &created_map);
        assert_eq!(removed, 1); // rule1 expired
        assert_eq!(engine.len(), 1);
        assert!(engine.get_rule(id2).is_some());
    }

    #[test]
    fn with_delay_treats_zero_as_no_delay() {
        assert_eq!(ExpectRule::new("p", "r").with_delay(0).delay_ms, None);
        assert_eq!(
            ExpectRule::new("p", "r").with_delay(250).delay_ms,
            Some(250)
        );
    }

    #[test]
    fn delay_survives_a_serde_round_trip_and_is_absent_when_unset() {
        let with_delay = ExpectRule::new("p", "r").with_delay(400);
        let json = serde_json::to_string(&with_delay).unwrap();
        assert!(json.contains("delay_ms"));
        let back: ExpectRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.delay_ms, Some(400));

        // Omitted from the wire form when unset, and a rule stored before the
        // field existed still deserializes.
        let plain = serde_json::to_string(&ExpectRule::new("p", "r")).unwrap();
        assert!(!plain.contains("delay_ms"));
        let legacy: ExpectRule =
            serde_json::from_str(r#"{"id":"00000000-0000-0000-0000-000000000001","pattern":"p","response":"r","priority":0,"timeout_ms":null,"enabled":true}"#)
                .unwrap();
        assert_eq!(legacy.delay_ms, None);
    }

    mod elevated {
        use super::*;
        use crate::models::ElevatedCredentials;

        fn enabled() -> ElevatedCredentials {
            ElevatedCredentials {
                enabled: true,
                ..ElevatedCredentials::default()
            }
        }

        #[test]
        fn disabled_config_produces_no_rules() {
            assert!(elevated_credentials_rules(&ElevatedCredentials::default()).is_empty());
        }

        #[test]
        fn rules_carry_the_placeholder_never_a_password() {
            let rules = elevated_credentials_rules(&enabled());
            assert_eq!(rules.len(), ElevatedCredentials::default_prompts().len());
            for rule in &rules {
                assert_eq!(rule.response, ELEVATED_RESPONSE_TEMPLATE);
                assert!(!rule.one_shot, "a session runs sudo more than once");
                assert_eq!(rule.priority, ELEVATED_PRIORITY);
                assert_eq!(rule.delay_ms, Some(100));
            }
        }

        #[test]
        fn custom_prompts_replace_the_defaults() {
            let config = ElevatedCredentials {
                enabled: true,
                custom_prompts: vec![r"^Secret code:\s*$".to_string()],
                delay_ms: 0,
            };
            let rules = elevated_credentials_rules(&config);
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].pattern, r"^Secret code:\s*$");
            assert_eq!(rules[0].delay_ms, None, "0 ms means send immediately");
        }

        /// The UI clamps to 5000 ms, but the dialog is not the only way a value
        /// arrives: a shared or hand-edited config deserializes straight into the
        /// struct. A longer delay is a longer window in which the prompt can leave
        /// before the response lands, so the ceiling is enforced on the model.
        #[test]
        fn an_out_of_range_delay_is_clamped_not_honoured() {
            use crate::models::MAX_ELEVATED_DELAY_MS;

            let config = ElevatedCredentials {
                enabled: true,
                custom_prompts: Vec::new(),
                delay_ms: 600_000,
            };
            assert_eq!(config.effective_delay_ms(), MAX_ELEVATED_DELAY_MS);

            let rules = elevated_credentials_rules(&config);
            assert!(!rules.is_empty());
            for rule in &rules {
                assert_eq!(rule.delay_ms, Some(MAX_ELEVATED_DELAY_MS));
            }

            // A value inside the range is passed through untouched.
            let ok = ElevatedCredentials {
                enabled: true,
                custom_prompts: Vec::new(),
                delay_ms: 250,
            };
            assert_eq!(ok.effective_delay_ms(), 250);
            assert_eq!(
                ElevatedCredentials {
                    enabled: true,
                    custom_prompts: Vec::new(),
                    delay_ms: MAX_ELEVATED_DELAY_MS,
                }
                .effective_delay_ms(),
                MAX_ELEVATED_DELAY_MS,
                "the ceiling itself is allowed"
            );
        }

        #[test]
        fn an_invalid_custom_pattern_is_dropped_not_fatal() {
            let config = ElevatedCredentials {
                enabled: true,
                custom_prompts: vec!["[unclosed".to_string(), r"^ok:\s*$".to_string()],
                delay_ms: 100,
            };
            let rules = elevated_credentials_rules(&config);
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].pattern, r"^ok:\s*$");
            // The surviving rule must still build an engine — `from_rules` fails
            // as a unit, so a bad pattern reaching it would disable sudo
            // injection entirely.
            assert!(ExpectEngine::from_rules(rules).is_ok());
        }

        /// The property the anchoring exists for: none of the defaults may match
        /// OpenSSH's own login prompt. An unanchored `Password:` or
        /// `\w+'s password:` does, and behind a jump host that prompt can belong
        /// to the bastion (issue #191).
        #[test]
        fn defaults_never_match_an_ssh_login_prompt() {
            let engine = ExpectEngine::from_rules(elevated_credentials_rules(&enabled())).unwrap();
            for line in [
                "totoshko88@db.example.com's password: ",
                "admin@10.0.0.5's password:",
                "root's password: ",
                "Enter passphrase for key '/home/u/.ssh/id_ed25519': ",
                "admin@bastion password:",
            ] {
                assert!(
                    engine.match_line(line).is_none(),
                    "elevated injection must not answer {line:?}"
                );
            }
        }

        #[test]
        fn defaults_match_the_escalation_prompts_they_are_for() {
            let engine = ExpectEngine::from_rules(elevated_credentials_rules(&enabled())).unwrap();
            for line in [
                "[sudo] password for totoshko88: ",
                "[sudo] password for domain\\user:",
                "Password:",
                "Password: ",
                "password:",
                "doas (totoshko88@laptop) password: ",
                "Enable Password:",
                "enable password: ",
            ] {
                assert!(
                    engine.match_line(line).is_some(),
                    "elevated injection should answer {line:?}"
                );
            }
        }
    }
}
