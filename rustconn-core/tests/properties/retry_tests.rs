//! Property tests for connection retry logic

use proptest::prelude::*;
use rustconn_core::connection::{RetryConfig, RetryState};
use std::time::Duration;

/// Strategy for generating valid retry configurations
fn retry_config_strategy() -> impl Strategy<Value = RetryConfig> {
    (
        0u32..10,          // max_attempts
        100u64..10_000,    // initial_delay_ms
        1_000u64..120_000, // max_delay_ms
        1.0f64..5.0,       // backoff_multiplier
        any::<bool>(),     // enabled
    )
        .prop_map(
            |(max_attempts, initial_delay_ms, max_delay_ms, backoff_multiplier, enabled)| {
                RetryConfig::new()
                    .with_max_attempts(max_attempts)
                    .with_initial_delay_ms(initial_delay_ms)
                    .with_max_delay_ms(max_delay_ms.max(initial_delay_ms))
                    .with_backoff_multiplier(backoff_multiplier)
                    .with_enabled(enabled)
            },
        )
}

proptest! {
    /// Property: Builder pattern preserves all configuration values
    #[test]
    fn config_builder_preserves_values(
        max_attempts in 0u32..100,
        initial_delay_ms in 1u64..100_000,
        max_delay_ms in 1u64..1_000_000,
        backoff_multiplier in 1.0f64..10.0,
        enabled in any::<bool>(),
    ) {
        let config = RetryConfig::new()
            .with_max_attempts(max_attempts)
            .with_initial_delay_ms(initial_delay_ms)
            .with_max_delay_ms(max_delay_ms)
            .with_backoff_multiplier(backoff_multiplier)
            .with_enabled(enabled);

        prop_assert_eq!(config.max_attempts, max_attempts);
        prop_assert_eq!(config.initial_delay_ms, initial_delay_ms);
        prop_assert_eq!(config.max_delay_ms, max_delay_ms);
        prop_assert!((config.backoff_multiplier - backoff_multiplier).abs() < f64::EPSILON);
        prop_assert_eq!(config.enabled, enabled);
    }

    /// Property: Delay is always capped at max_delay_ms
    #[test]
    fn delay_never_exceeds_max(
        config in retry_config_strategy(),
        attempt in 0u32..20,
    ) {
        if let Some(delay) = config.delay_for_attempt(attempt) {
            prop_assert!(delay.as_millis() <= u128::from(config.max_delay_ms));
        }
    }

    /// Property: Delay increases monotonically (until capped)
    #[test]
    fn delay_increases_monotonically(
        config in retry_config_strategy(),
    ) {
        if !config.enabled || config.max_attempts == 0 {
            return Ok(());
        }

        let mut prev_delay = Duration::ZERO;
        for attempt in 0..config.max_attempts.min(10) {
            if let Some(delay) = config.delay_for_attempt(attempt) {
                prop_assert!(delay >= prev_delay, "Delay should not decrease");
                prev_delay = delay;
            }
        }
    }

    /// Property: should_retry returns false when disabled
    #[test]
    fn disabled_config_never_retries(
        max_attempts in 0u32..100,
        attempt in 0u32..100,
    ) {
        let config = RetryConfig::new()
            .with_max_attempts(max_attempts)
            .with_enabled(false);

        prop_assert!(!config.should_retry(attempt));
    }

    /// Property: should_retry returns false when attempt >= max_attempts
    #[test]
    fn exhausted_attempts_no_retry(
        max_attempts in 0u32..20,
    ) {
        let config = RetryConfig::new()
            .with_max_attempts(max_attempts)
            .with_enabled(true);

        // At max_attempts, should not retry
        prop_assert!(!config.should_retry(max_attempts));
        // Beyond max_attempts, should not retry
        prop_assert!(!config.should_retry(max_attempts + 1));
    }

    /// Property: delay_for_attempt returns None when should_retry is false
    #[test]
    fn no_delay_when_no_retry(
        config in retry_config_strategy(),
        attempt in 0u32..100,
    ) {
        if !config.should_retry(attempt) {
            prop_assert!(config.delay_for_attempt(attempt).is_none());
        }
    }

    /// Property: total_attempts is max_attempts + 1 when enabled
    #[test]
    fn total_attempts_calculation(
        max_attempts in 0u32..100,
        enabled in any::<bool>(),
    ) {
        let config = RetryConfig::new()
            .with_max_attempts(max_attempts)
            .with_enabled(enabled);

        let expected = if enabled { max_attempts + 1 } else { 1 };
        prop_assert_eq!(config.total_attempts(), expected);
    }

    /// Property: RetryState tracks attempts correctly
    #[test]
    fn retry_state_tracks_attempts(
        config in retry_config_strategy(),
        failures in 0usize..10,
    ) {
        let mut state = RetryState::new(config.clone());

        prop_assert_eq!(state.current_attempt(), 0);
        prop_assert_eq!(state.attempt_number(), 1);

        for i in 0..failures {
            let error_msg = format!("Error {i}");
            state.record_failure(&error_msg);
            prop_assert_eq!(state.current_attempt(), (i + 1) as u32);
            prop_assert_eq!(state.last_error(), Some(error_msg.as_str()));
        }
    }

    /// Property: RetryState reset clears all state
    #[test]
    fn retry_state_reset_clears_state(
        config in retry_config_strategy(),
        failures in 1usize..10,
    ) {
        let mut state = RetryState::new(config);

        for i in 0..failures {
            state.record_failure(format!("Error {i}"));
        }

        state.reset();

        prop_assert_eq!(state.current_attempt(), 0);
        prop_assert!(state.last_error().is_none());
    }

    /// Property: RetryState progress is between 0.0 and 1.0 (within valid attempts)
    #[test]
    fn retry_state_progress_bounded(
        config in retry_config_strategy(),
    ) {
        let mut state = RetryState::new(config.clone());
        let max_failures = config.total_attempts() as usize;

        // Only test within valid attempt range
        for i in 0..max_failures {
            let progress = state.progress();
            prop_assert!(progress >= 0.0, "Progress should be >= 0.0, got {progress}");
            prop_assert!(progress <= 1.0, "Progress should be <= 1.0, got {progress}");
            state.record_failure(format!("Error {i}"));
        }
    }

    /// Property: record_failure returns correct retry status
    #[test]
    fn record_failure_returns_correct_status(
        max_attempts in 1u32..10,
    ) {
        let config = RetryConfig::new()
            .with_max_attempts(max_attempts)
            .with_enabled(true);
        let mut state = RetryState::new(config);

        // First (max_attempts - 1) failures should return true
        for _ in 0..(max_attempts - 1) {
            prop_assert!(state.record_failure("error"));
        }

        // Last failure should return false (exhausted)
        prop_assert!(!state.record_failure("final error"));
    }
}

#[test]
fn test_preset_configs() {
    let default = RetryConfig::default();
    assert!(default.enabled);
    assert_eq!(default.max_attempts, 3);

    let no_retry = RetryConfig::no_retry();
    assert!(!no_retry.enabled);
    assert_eq!(no_retry.max_attempts, 0);

    let aggressive = RetryConfig::aggressive();
    assert!(aggressive.enabled);
    assert_eq!(aggressive.max_attempts, 5);
    assert!(aggressive.initial_delay_ms < default.initial_delay_ms);

    let conservative = RetryConfig::conservative();
    assert!(conservative.enabled);
    assert_eq!(conservative.max_attempts, 2);
    assert!(conservative.initial_delay_ms > default.initial_delay_ms);
}

#[test]
fn test_exponential_backoff_sequence() {
    let config = RetryConfig::new()
        .with_initial_delay_ms(1000)
        .with_backoff_multiplier(2.0)
        .with_max_delay_ms(100_000)
        .with_max_attempts(5);

    // Expected sequence: 1000, 2000, 4000, 8000, 16000
    assert_eq!(
        config.delay_for_attempt(0),
        Some(Duration::from_millis(1000))
    );
    assert_eq!(
        config.delay_for_attempt(1),
        Some(Duration::from_millis(2000))
    );
    assert_eq!(
        config.delay_for_attempt(2),
        Some(Duration::from_millis(4000))
    );
    assert_eq!(
        config.delay_for_attempt(3),
        Some(Duration::from_millis(8000))
    );
    assert_eq!(
        config.delay_for_attempt(4),
        Some(Duration::from_millis(16000))
    );
    assert_eq!(config.delay_for_attempt(5), None); // Beyond max_attempts
}

#[test]
fn test_retry_state_success_clears_error() {
    let mut state = RetryState::with_defaults();
    state.record_failure("Connection refused");
    assert!(state.last_error().is_some());

    state.record_success();
    assert!(state.last_error().is_none());
}
