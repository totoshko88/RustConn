//! Expect-style automation for interactive prompts
//!
//! This module provides expect-style pattern matching for automating interactive
//! terminal prompts. It supports:
//! - Regex pattern matching against terminal output
//! - Automatic response sending when patterns match
//! - Priority-based rule ordering
//! - Timeout handling for patterns

use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl PartialEq for ExpectRule {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.pattern == other.pattern
            && self.response == other.response
            && self.priority == other.priority
            && self.timeout_ms == other.timeout_ms
            && self.enabled == other.enabled
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
    pub fn new(rule: ExpectRule) -> ExpectResult<Self> {
        let regex = rule.compile_pattern()?;
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
        for rule in rules {
            engine.add_rule(rule)?;
        }
        Ok(engine)
    }

    /// Adds a rule to the engine
    ///
    /// # Errors
    ///
    /// Returns an error if the rule's pattern fails to compile or if
    /// a rule with the same ID already exists.
    pub fn add_rule(&mut self, rule: ExpectRule) -> ExpectResult<()> {
        // Check for duplicate ID
        if self.rules.iter().any(|r| r.rule.id == rule.id) {
            return Err(ExpectError::DuplicateRuleId(rule.id));
        }

        let compiled = CompiledRule::new(rule)?;
        self.rules.push(compiled);
        self.sort_by_priority();
        Ok(())
    }

    /// Removes a rule by ID
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
        Ok(self.rules.remove(pos).rule)
    }

    /// Updates a rule
    ///
    /// # Errors
    ///
    /// Returns an error if the rule is not found or if the new pattern
    /// fails to compile.
    pub fn update_rule(&mut self, rule: ExpectRule) -> ExpectResult<()> {
        let pos = self
            .rules
            .iter()
            .position(|r| r.rule.id == rule.id)
            .ok_or(ExpectError::RuleNotFound(rule.id))?;

        let compiled = CompiledRule::new(rule)?;
        self.rules[pos] = compiled;
        self.sort_by_priority();
        Ok(())
    }

    /// Gets a rule by ID
    #[must_use]
    pub fn get_rule(&self, id: Uuid) -> Option<&ExpectRule> {
        self.rules.iter().find(|r| r.rule.id == id).map(|r| &r.rule)
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
            .sort_by(|a, b| b.rule.priority.cmp(&a.rule.priority));
    }

    /// Validates all patterns in the engine
    ///
    /// # Errors
    ///
    /// Returns an error if any pattern is invalid.
    pub fn validate_patterns(&self) -> ExpectResult<()> {
        for compiled in &self.rules {
            compiled.rule.validate_pattern()?;
        }
        Ok(())
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
                .substitute(&rule.response, scope)
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

    /// Clears all rules from the engine
    pub fn clear(&mut self) {
        self.rules.clear();
    }
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
}
