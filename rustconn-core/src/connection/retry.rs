//! Connection retry logic with exponential backoff
//!
//! This module provides retry configuration and utilities for handling
//! transient connection failures with automatic retry and exponential backoff.

use std::time::Duration;

/// Default maximum number of retry attempts
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

/// Default initial delay between retries in milliseconds
pub const DEFAULT_INITIAL_DELAY_MS: u64 = 1000;

/// Default maximum delay between retries in milliseconds
pub const DEFAULT_MAX_DELAY_MS: u64 = 30_000;

/// Default backoff multiplier (delay doubles each retry)
pub const DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;

/// Configuration for connection retry behavior
///
/// Implements exponential backoff with configurable parameters.
/// The delay between retries is calculated as:
/// `min(initial_delay * multiplier^attempt, max_delay)`
#[derive(Debug, Clone, PartialEq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_attempts: u32,
    /// Initial delay between retries in milliseconds
    pub initial_delay_ms: u64,
    /// Maximum delay between retries in milliseconds
    pub max_delay_ms: u64,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Whether retry is enabled
    pub enabled: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            initial_delay_ms: DEFAULT_INITIAL_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            enabled: true,
        }
    }
}

impl RetryConfig {
    /// Creates a new retry configuration with default values
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a retry configuration with no retries (single attempt)
    #[must_use]
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 0,
            enabled: false,
            ..Self::default()
        }
    }

    /// Creates a retry configuration for aggressive retry (more attempts, shorter delays)
    #[must_use]
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 5,
            initial_delay_ms: 500,
            max_delay_ms: 10_000,
            backoff_multiplier: 1.5,
            enabled: true,
        }
    }

    /// Creates a retry configuration for conservative retry (fewer attempts, longer delays)
    #[must_use]
    pub fn conservative() -> Self {
        Self {
            max_attempts: 2,
            initial_delay_ms: 2000,
            max_delay_ms: 60_000,
            backoff_multiplier: 3.0,
            enabled: true,
        }
    }

    /// Sets the maximum number of retry attempts
    #[must_use]
    pub const fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
    }

    /// Sets the initial delay between retries
    #[must_use]
    pub const fn with_initial_delay_ms(mut self, delay_ms: u64) -> Self {
        self.initial_delay_ms = delay_ms;
        self
    }

    /// Sets the maximum delay between retries
    #[must_use]
    pub const fn with_max_delay_ms(mut self, delay_ms: u64) -> Self {
        self.max_delay_ms = delay_ms;
        self
    }

    /// Sets the backoff multiplier
    #[must_use]
    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Enables or disables retry
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Calculates the delay for a given attempt number (0-indexed)
    ///
    /// Returns `None` if retry is disabled or attempt exceeds max_attempts.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Option<Duration> {
        if !self.enabled || attempt >= self.max_attempts {
            return None;
        }

        let delay_ms = self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32);
        let capped_delay_ms = (delay_ms as u64).min(self.max_delay_ms);

        Some(Duration::from_millis(capped_delay_ms))
    }

    /// Returns whether another retry should be attempted
    #[must_use]
    pub const fn should_retry(&self, attempt: u32) -> bool {
        self.enabled && attempt < self.max_attempts
    }

    /// Returns the total number of attempts (initial + retries)
    #[must_use]
    pub const fn total_attempts(&self) -> u32 {
        if self.enabled {
            self.max_attempts + 1
        } else {
            1
        }
    }
}

/// State tracker for retry operations
///
/// Tracks the current attempt number and provides methods for
/// managing retry state during connection attempts.
#[derive(Debug, Clone)]
pub struct RetryState {
    /// Current attempt number (0-indexed)
    current_attempt: u32,
    /// Configuration for retry behavior
    config: RetryConfig,
    /// Last error message (if any)
    last_error: Option<String>,
}

impl RetryState {
    /// Creates a new retry state with the given configuration
    #[must_use]
    pub fn new(config: RetryConfig) -> Self {
        Self {
            current_attempt: 0,
            config,
            last_error: None,
        }
    }

    /// Creates a new retry state with default configuration
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(RetryConfig::default())
    }

    /// Returns the current attempt number (0-indexed)
    #[must_use]
    pub const fn current_attempt(&self) -> u32 {
        self.current_attempt
    }

    /// Returns the current attempt number (1-indexed, for display)
    #[must_use]
    pub const fn attempt_number(&self) -> u32 {
        self.current_attempt + 1
    }

    /// Returns the total number of attempts that will be made
    #[must_use]
    pub const fn total_attempts(&self) -> u32 {
        self.config.total_attempts()
    }

    /// Returns whether another retry should be attempted
    #[must_use]
    pub const fn should_retry(&self) -> bool {
        self.config.should_retry(self.current_attempt)
    }

    /// Returns the delay before the next retry attempt
    #[must_use]
    pub fn next_delay(&self) -> Option<Duration> {
        self.config.delay_for_attempt(self.current_attempt)
    }

    /// Records a failed attempt and advances to the next retry
    ///
    /// Returns `true` if another retry will be attempted, `false` if exhausted.
    pub fn record_failure(&mut self, error: impl Into<String>) -> bool {
        self.last_error = Some(error.into());
        self.current_attempt += 1;
        self.should_retry()
    }

    /// Records a successful attempt
    pub fn record_success(&mut self) {
        self.last_error = None;
    }

    /// Resets the retry state for a new connection attempt
    pub fn reset(&mut self) {
        self.current_attempt = 0;
        self.last_error = None;
    }

    /// Returns the last error message
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Returns the retry configuration
    #[must_use]
    pub const fn config(&self) -> &RetryConfig {
        &self.config
    }

    /// Returns progress as a fraction (0.0 to 1.0)
    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.config.total_attempts() == 0 {
            return 1.0;
        }
        f64::from(self.current_attempt) / f64::from(self.config.total_attempts())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, DEFAULT_MAX_ATTEMPTS);
        assert_eq!(config.initial_delay_ms, DEFAULT_INITIAL_DELAY_MS);
        assert!(config.enabled);
    }

    #[test]
    fn test_no_retry_config() {
        let config = RetryConfig::no_retry();
        assert_eq!(config.max_attempts, 0);
        assert!(!config.enabled);
        assert!(!config.should_retry(0));
    }

    #[test]
    fn test_delay_calculation() {
        let config = RetryConfig::new()
            .with_initial_delay_ms(1000)
            .with_backoff_multiplier(2.0)
            .with_max_delay_ms(10_000);

        // First retry: 1000ms
        assert_eq!(
            config.delay_for_attempt(0),
            Some(Duration::from_millis(1000))
        );
        // Second retry: 2000ms
        assert_eq!(
            config.delay_for_attempt(1),
            Some(Duration::from_millis(2000))
        );
        // Third retry: 4000ms
        assert_eq!(
            config.delay_for_attempt(2),
            Some(Duration::from_millis(4000))
        );
    }

    #[test]
    fn test_delay_capping() {
        let config = RetryConfig::new()
            .with_initial_delay_ms(5000)
            .with_backoff_multiplier(3.0)
            .with_max_delay_ms(10_000)
            .with_max_attempts(5);

        // First retry: 5000ms
        assert_eq!(
            config.delay_for_attempt(0),
            Some(Duration::from_millis(5000))
        );
        // Second retry: would be 15000ms, capped to 10000ms
        assert_eq!(
            config.delay_for_attempt(1),
            Some(Duration::from_millis(10_000))
        );
    }

    #[test]
    fn test_retry_state() {
        let config = RetryConfig::new().with_max_attempts(2);
        let mut state = RetryState::new(config);

        assert_eq!(state.current_attempt(), 0);
        assert_eq!(state.attempt_number(), 1);
        assert!(state.should_retry());

        // First failure
        assert!(state.record_failure("Connection refused"));
        assert_eq!(state.current_attempt(), 1);
        assert!(state.should_retry());

        // Second failure
        assert!(!state.record_failure("Timeout"));
        assert_eq!(state.current_attempt(), 2);
        assert!(!state.should_retry());
    }

    #[test]
    fn test_retry_state_reset() {
        let mut state = RetryState::with_defaults();
        state.record_failure("Error 1");
        state.record_failure("Error 2");

        state.reset();
        assert_eq!(state.current_attempt(), 0);
        assert!(state.last_error().is_none());
    }
}
