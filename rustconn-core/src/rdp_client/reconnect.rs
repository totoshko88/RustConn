//! RDP reconnection logic
//!
//! This module provides automatic reconnection capabilities for RDP sessions.
//! When a connection is lost unexpectedly, the client can attempt to reconnect
//! with configurable retry policies.
//!
//! # Features
//!
//! - Exponential backoff with jitter
//! - Configurable retry limits
//! - Session state preservation hints
//! - Connection quality monitoring

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Reconnection policy configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReconnectPolicy {
    /// Whether automatic reconnection is enabled
    pub enabled: bool,
    /// Maximum number of reconnection attempts (0 = unlimited)
    pub max_attempts: u32,
    /// Initial delay before first reconnection attempt
    pub initial_delay: Duration,
    /// Maximum delay between reconnection attempts
    pub max_delay: Duration,
    /// Backoff multiplier (e.g., 2.0 for exponential backoff)
    pub backoff_multiplier: f64,
    /// Whether to add random jitter to delays
    pub use_jitter: bool,
    /// Timeout for each reconnection attempt
    pub attempt_timeout: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            use_jitter: true,
            attempt_timeout: Duration::from_secs(30),
        }
    }
}

impl ReconnectPolicy {
    /// Creates a policy that never reconnects
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            max_attempts: 0,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            use_jitter: false,
            attempt_timeout: Duration::from_secs(30),
        }
    }

    /// Creates an aggressive reconnection policy for unstable networks
    #[must_use]
    pub const fn aggressive() -> Self {
        Self {
            enabled: true,
            max_attempts: 10,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 1.5,
            use_jitter: true,
            attempt_timeout: Duration::from_secs(15),
        }
    }

    /// Creates a conservative policy for stable networks
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            enabled: true,
            max_attempts: 3,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 3.0,
            use_jitter: true,
            attempt_timeout: Duration::from_secs(45),
        }
    }

    /// Calculates the delay for a given attempt number
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return self.initial_delay;
        }

        let base_delay = self.initial_delay.as_secs_f64()
            * self
                .backoff_multiplier
                .powi(attempt.saturating_sub(1) as i32);

        let delay_secs = base_delay.min(self.max_delay.as_secs_f64());

        let final_delay = if self.use_jitter {
            // Add up to 25% jitter
            let jitter = delay_secs * 0.25 * rand_jitter();
            delay_secs + jitter
        } else {
            delay_secs
        };

        Duration::from_secs_f64(final_delay)
    }

    /// Returns whether another attempt should be made
    #[must_use]
    pub const fn should_retry(&self, attempt: u32) -> bool {
        self.enabled && (self.max_attempts == 0 || attempt < self.max_attempts)
    }
}

/// Simple pseudo-random jitter (0.0 to 1.0)
/// Uses a basic LCG seeded from current time
fn rand_jitter() -> f64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345);

    // Simple LCG
    let next = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (next as f64) / (u64::MAX as f64)
}

/// Reconnection state tracker
#[derive(Debug, Clone, Default)]
pub struct ReconnectState {
    /// Current attempt number (0 = first attempt)
    pub attempt: u32,
    /// Total reconnection attempts made in this session
    pub total_attempts: u32,
    /// Last error message
    pub last_error: Option<String>,
    /// Whether currently attempting to reconnect
    pub reconnecting: bool,
    /// Time of last successful connection
    pub last_connected: Option<std::time::Instant>,
    /// Time of last disconnection
    pub last_disconnected: Option<std::time::Instant>,
}

impl ReconnectState {
    /// Creates a new reconnection state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            attempt: 0,
            total_attempts: 0,
            last_error: None,
            reconnecting: false,
            last_connected: None,
            last_disconnected: None,
        }
    }

    /// Records a successful connection
    pub fn on_connected(&mut self) {
        self.attempt = 0;
        self.reconnecting = false;
        self.last_error = None;
        self.last_connected = Some(std::time::Instant::now());
    }

    /// Records a disconnection
    pub fn on_disconnected(&mut self, error: Option<String>) {
        self.last_error = error;
        self.last_disconnected = Some(std::time::Instant::now());
    }

    /// Records a reconnection attempt
    pub fn on_attempt(&mut self) {
        self.attempt += 1;
        self.total_attempts += 1;
        self.reconnecting = true;
    }

    /// Records a failed reconnection attempt
    pub fn on_attempt_failed(&mut self, error: String) {
        self.last_error = Some(error);
    }

    /// Resets the state for a new connection
    pub fn reset(&mut self) {
        self.attempt = 0;
        self.reconnecting = false;
        self.last_error = None;
    }

    /// Returns the uptime since last connection
    #[must_use]
    pub fn uptime(&self) -> Option<Duration> {
        self.last_connected.map(|t| t.elapsed())
    }

    /// Returns the downtime since last disconnection
    #[must_use]
    pub fn downtime(&self) -> Option<Duration> {
        self.last_disconnected.map(|t| t.elapsed())
    }
}

/// Disconnect reason classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// User requested disconnect
    UserRequested,
    /// Network error (potentially recoverable)
    NetworkError,
    /// Server closed connection
    ServerClosed,
    /// Authentication failure (not recoverable)
    AuthenticationFailed,
    /// Protocol error (potentially recoverable)
    ProtocolError,
    /// Timeout
    Timeout,
    /// Unknown reason
    Unknown,
}

impl DisconnectReason {
    /// Returns whether this disconnect reason is potentially recoverable
    #[must_use]
    pub const fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::NetworkError | Self::ServerClosed | Self::ProtocolError | Self::Timeout
        )
    }

    /// Classifies an error message into a disconnect reason
    #[must_use]
    pub fn from_error(error: &str) -> Self {
        let error_lower = error.to_lowercase();

        if error_lower.contains("authentication")
            || error_lower.contains("credential")
            || error_lower.contains("password")
            || error_lower.contains("logon")
        {
            return Self::AuthenticationFailed;
        }

        if error_lower.contains("timeout") || error_lower.contains("timed out") {
            return Self::Timeout;
        }

        if error_lower.contains("network")
            || error_lower.contains("connection")
            || error_lower.contains("socket")
            || error_lower.contains("tcp")
            || error_lower.contains("io error")
        {
            return Self::NetworkError;
        }

        if error_lower.contains("server")
            || error_lower.contains("disconnect")
            || error_lower.contains("closed")
        {
            return Self::ServerClosed;
        }

        if error_lower.contains("protocol") || error_lower.contains("pdu") {
            return Self::ProtocolError;
        }

        Self::Unknown
    }
}

/// Connection quality metrics
#[derive(Debug, Clone, Default)]
pub struct ConnectionQuality {
    /// Round-trip time in milliseconds
    pub rtt_ms: Option<u32>,
    /// Packet loss percentage (0-100)
    pub packet_loss: Option<f32>,
    /// Bandwidth estimate in Kbps
    pub bandwidth_kbps: Option<u32>,
    /// Number of frame updates per second
    pub fps: Option<f32>,
    /// Quality rating (0-100)
    pub quality_score: Option<u8>,
}

impl ConnectionQuality {
    /// Creates a new connection quality tracker
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rtt_ms: None,
            packet_loss: None,
            bandwidth_kbps: None,
            fps: None,
            quality_score: None,
        }
    }

    /// Updates RTT measurement
    pub fn update_rtt(&mut self, rtt_ms: u32) {
        self.rtt_ms = Some(rtt_ms);
        self.recalculate_score();
    }

    /// Updates FPS measurement
    pub fn update_fps(&mut self, fps: f32) {
        self.fps = Some(fps);
        self.recalculate_score();
    }

    /// Recalculates the overall quality score
    fn recalculate_score(&mut self) {
        let mut score = 100u8;

        // Penalize high RTT
        if let Some(rtt) = self.rtt_ms {
            if rtt > 200 {
                score = score.saturating_sub(30);
            } else if rtt > 100 {
                score = score.saturating_sub(15);
            } else if rtt > 50 {
                score = score.saturating_sub(5);
            }
        }

        // Penalize low FPS
        if let Some(fps) = self.fps {
            if fps < 10.0 {
                score = score.saturating_sub(40);
            } else if fps < 20.0 {
                score = score.saturating_sub(20);
            } else if fps < 30.0 {
                score = score.saturating_sub(10);
            }
        }

        // Penalize packet loss
        if let Some(loss) = self.packet_loss {
            if loss > 5.0 {
                score = score.saturating_sub(30);
            } else if loss > 1.0 {
                score = score.saturating_sub(15);
            } else if loss > 0.1 {
                score = score.saturating_sub(5);
            }
        }

        self.quality_score = Some(score);
    }

    /// Returns a human-readable quality description
    #[must_use]
    pub fn quality_description(&self) -> &'static str {
        match self.quality_score {
            Some(score) if score >= 80 => "Excellent",
            Some(score) if score >= 60 => "Good",
            Some(score) if score >= 40 => "Fair",
            Some(score) if score >= 20 => "Poor",
            Some(_) => "Very Poor",
            None => "Unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_policy_default() {
        let policy = ReconnectPolicy::default();
        assert!(policy.enabled);
        assert_eq!(policy.max_attempts, 5);
    }

    #[test]
    fn test_reconnect_policy_disabled() {
        let policy = ReconnectPolicy::disabled();
        assert!(!policy.enabled);
        assert!(!policy.should_retry(0));
    }

    #[test]
    fn test_should_retry() {
        let policy = ReconnectPolicy {
            enabled: true,
            max_attempts: 3,
            ..Default::default()
        };

        assert!(policy.should_retry(0));
        assert!(policy.should_retry(1));
        assert!(policy.should_retry(2));
        assert!(!policy.should_retry(3));
    }

    #[test]
    fn test_delay_for_attempt() {
        let policy = ReconnectPolicy {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            use_jitter: false,
            ..Default::default()
        };

        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(2));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_secs(4));
    }

    #[test]
    fn test_delay_capped_at_max() {
        let policy = ReconnectPolicy {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 10.0,
            use_jitter: false,
            ..Default::default()
        };

        // Should be capped at 5 seconds
        assert_eq!(policy.delay_for_attempt(5), Duration::from_secs(5));
    }

    #[test]
    fn test_reconnect_state() {
        let mut state = ReconnectState::new();

        state.on_connected();
        assert_eq!(state.attempt, 0);
        assert!(!state.reconnecting);

        state.on_disconnected(Some("Network error".to_string()));
        assert_eq!(state.last_error, Some("Network error".to_string()));

        state.on_attempt();
        assert_eq!(state.attempt, 1);
        assert!(state.reconnecting);

        state.on_attempt();
        assert_eq!(state.attempt, 2);
        assert_eq!(state.total_attempts, 2);
    }

    #[test]
    fn test_disconnect_reason_classification() {
        assert_eq!(
            DisconnectReason::from_error("Authentication failed"),
            DisconnectReason::AuthenticationFailed
        );
        assert_eq!(
            DisconnectReason::from_error("Connection timeout"),
            DisconnectReason::Timeout
        );
        assert_eq!(
            DisconnectReason::from_error("Network error: connection reset"),
            DisconnectReason::NetworkError
        );
        assert_eq!(
            DisconnectReason::from_error("Server disconnected"),
            DisconnectReason::ServerClosed
        );
    }

    #[test]
    fn test_disconnect_reason_recoverable() {
        assert!(DisconnectReason::NetworkError.is_recoverable());
        assert!(DisconnectReason::Timeout.is_recoverable());
        assert!(!DisconnectReason::AuthenticationFailed.is_recoverable());
        assert!(!DisconnectReason::UserRequested.is_recoverable());
    }

    #[test]
    fn test_connection_quality() {
        let mut quality = ConnectionQuality::new();

        quality.update_rtt(50);
        assert!(quality.quality_score.unwrap() >= 90);

        quality.update_rtt(250);
        assert!(quality.quality_score.unwrap() < 80);
    }

    #[test]
    fn test_quality_description() {
        let mut quality = ConnectionQuality::new();

        quality.quality_score = Some(90);
        assert_eq!(quality.quality_description(), "Excellent");

        quality.quality_score = Some(50);
        assert_eq!(quality.quality_description(), "Fair");

        quality.quality_score = Some(10);
        assert_eq!(quality.quality_description(), "Very Poor");
    }
}
