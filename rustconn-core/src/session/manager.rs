//! Session manager for `RustConn`
//!
//! This module provides the `SessionManager` which handles the lifecycle
//! of active connection sessions, including starting, terminating,
//! and tracking sessions.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::error::{SessionError, SessionResult};
use crate::models::Connection;
use crate::protocol::ProtocolRegistry;

use super::logger::{LogConfig, LogContext, SessionLogger};
use super::session::{Session, SessionState, SessionType};

/// Default health check interval in seconds
pub const DEFAULT_HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Health check result for a session
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// Session is healthy and running
    Healthy,
    /// Session is unhealthy (process terminated unexpectedly)
    Unhealthy(String),
    /// Session state is unknown (cannot determine)
    Unknown,
    /// Session is intentionally terminated
    Terminated,
}

/// Health check event for callbacks
#[derive(Debug, Clone)]
pub struct HealthCheckEvent {
    /// Session ID
    pub session_id: Uuid,
    /// Connection ID
    pub connection_id: Uuid,
    /// Session name
    pub session_name: String,
    /// Previous health status
    pub previous_status: HealthStatus,
    /// Current health status
    pub current_status: HealthStatus,
    /// Timestamp of the check
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

/// Configuration for health checks
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Whether health checks are enabled
    pub enabled: bool,
    /// Interval between health checks
    pub interval: Duration,
    /// Whether to auto-remove terminated sessions
    pub auto_cleanup: bool,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: Duration::from_secs(DEFAULT_HEALTH_CHECK_INTERVAL_SECS),
            auto_cleanup: false,
        }
    }
}

impl HealthCheckConfig {
    /// Creates a new health check config with default settings
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a disabled health check config
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Sets the check interval
    #[must_use]
    pub const fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Sets the check interval in seconds
    #[must_use]
    pub const fn with_interval_secs(mut self, secs: u64) -> Self {
        self.interval = Duration::from_secs(secs);
        self
    }

    /// Enables auto-cleanup of terminated sessions
    #[must_use]
    pub const fn with_auto_cleanup(mut self, enabled: bool) -> Self {
        self.auto_cleanup = enabled;
        self
    }
}

/// Manages active connection sessions
///
/// The `SessionManager` is responsible for:
/// - Starting new sessions for connections
/// - Tracking active sessions
/// - Terminating sessions
/// - Managing session logging
/// - Performing health checks on active sessions
pub struct SessionManager {
    /// Active sessions indexed by session ID
    sessions: HashMap<Uuid, Session>,
    /// Protocol registry for validation
    protocol_registry: ProtocolRegistry,
    /// Session loggers indexed by session ID
    session_loggers: HashMap<Uuid, SessionLogger>,
    /// Default log configuration for new sessions
    default_log_config: Option<LogConfig>,
    /// Whether logging is enabled globally
    logging_enabled: bool,
    /// Health check configuration
    health_check_config: HealthCheckConfig,
    /// Last health check timestamp
    last_health_check: Option<Instant>,
    /// Health status cache for detecting changes
    health_status_cache: HashMap<Uuid, HealthStatus>,
}

impl SessionManager {
    /// Creates a new `SessionManager`
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            protocol_registry: ProtocolRegistry::new(),
            session_loggers: HashMap::new(),
            default_log_config: None,
            logging_enabled: false,
            health_check_config: HealthCheckConfig::default(),
            last_health_check: None,
            health_status_cache: HashMap::new(),
        }
    }

    /// Creates a new `SessionManager` with logging enabled
    ///
    /// # Arguments
    ///
    /// * `log_dir` - Base directory for log files
    ///
    /// # Errors
    /// Returns an error if the log directory cannot be created
    pub fn with_logging(log_dir: &Path) -> SessionResult<Self> {
        // Ensure the log directory exists
        if !log_dir.exists() {
            std::fs::create_dir_all(log_dir).map_err(|e| {
                SessionError::LoggingError(format!(
                    "Failed to create log directory '{}': {}",
                    log_dir.display(),
                    e
                ))
            })?;
        }

        // Create a default log config using the provided directory
        let path_template = log_dir
            .join("${connection_name}_${date}.log")
            .to_string_lossy()
            .to_string();

        let config = LogConfig::new(path_template).with_enabled(true);

        Ok(Self {
            sessions: HashMap::new(),
            protocol_registry: ProtocolRegistry::new(),
            session_loggers: HashMap::new(),
            default_log_config: Some(config),
            logging_enabled: true,
            health_check_config: HealthCheckConfig::default(),
            last_health_check: None,
            health_status_cache: HashMap::new(),
        })
    }

    /// Enables or disables session logging
    pub const fn set_logging_enabled(&mut self, enabled: bool) {
        self.logging_enabled = enabled;
    }

    /// Sets the default log configuration for new sessions
    pub fn set_default_log_config(&mut self, config: LogConfig) {
        self.default_log_config = Some(config);
    }

    /// Starts a new session for a connection
    ///
    /// This creates a session record for tracking. The actual connection
    /// is handled by the GUI layer (VTE4 for SSH, native widgets for RDP/VNC/SPICE).
    ///
    /// # Errors
    /// Returns an error if the session cannot be started
    pub fn start_session(&mut self, connection: &Connection) -> SessionResult<Uuid> {
        // Get the protocol handler
        let protocol = self
            .protocol_registry
            .get(connection.protocol.as_str())
            .ok_or_else(|| {
                SessionError::StartFailed(format!("Unknown protocol: {}", connection.protocol))
            })?;

        // Validate the connection
        protocol.validate_connection(connection).map_err(|e| {
            SessionError::StartFailed(format!("Invalid connection configuration: {e}"))
        })?;

        // Determine session type based on protocol
        let session_type = match connection.protocol.as_str() {
            "ssh" => SessionType::Embedded,
            _ => SessionType::External, // RDP, VNC, SPICE will use native widgets
        };

        // Create the session
        let mut session = Session::new(
            connection.id,
            connection.name.clone(),
            protocol.protocol_id().to_string(),
            session_type,
        );

        let session_id = session.id;

        // Set up logging if enabled
        if self.logging_enabled {
            if let Some(ref config) = self.default_log_config {
                let context = LogContext::new(&connection.name, connection.protocol.as_str());
                match SessionLogger::new(config.clone(), &context, None) {
                    Ok(logger) => {
                        let log_path = logger.log_path().to_path_buf();
                        tracing::info!(
                            connection = %connection.name,
                            path = %log_path.display(),
                            "Session logging enabled"
                        );
                        session.set_log_file(log_path);
                        self.session_loggers.insert(session_id, logger);
                    }
                    Err(e) => {
                        tracing::warn!(
                            %e,
                            connection = %connection.name,
                            path_template = %config.path_template,
                            "Failed to create session logger"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    connection = %connection.name,
                    "Logging enabled but no log config set for session"
                );
            }
        }

        self.sessions.insert(session_id, session);

        Ok(session_id)
    }

    /// Sets the process handle for a session
    ///
    /// This is called by the GUI layer after spawning the process.
    ///
    /// # Errors
    /// Returns an error if the session is not found
    pub fn set_session_process(
        &mut self,
        session_id: Uuid,
        process: std::process::Child,
    ) -> SessionResult<()> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;

        session.set_process(process);
        Ok(())
    }

    /// Terminates a session
    ///
    /// # Errors
    /// Returns an error if the session cannot be terminated
    pub fn terminate_session(&mut self, session_id: Uuid) -> SessionResult<()> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;

        // Terminate the process
        session.terminate().map_err(|e| {
            SessionError::TerminateFailed(format!("Failed to terminate process: {e}"))
        })?;

        // Close the session logger (this will finalize the log file)
        if let Some(mut logger) = self.session_loggers.remove(&session_id) {
            if let Err(e) = logger.close() {
                tracing::warn!(%e, "Failed to close session logger");
            }
        }

        Ok(())
    }

    /// Force kills a session
    ///
    /// # Errors
    /// Returns an error if the session cannot be killed
    pub fn kill_session(&mut self, session_id: Uuid) -> SessionResult<()> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;

        session
            .kill()
            .map_err(|e| SessionError::TerminateFailed(format!("Failed to kill process: {e}")))?;

        // Close the session logger (this will finalize the log file)
        if let Some(mut logger) = self.session_loggers.remove(&session_id) {
            if let Err(e) = logger.close() {
                tracing::warn!(%e, "Failed to close session logger");
            }
        }

        Ok(())
    }

    /// Removes a terminated session from tracking
    pub fn remove_session(&mut self, session_id: Uuid) -> Option<Session> {
        self.sessions.remove(&session_id)
    }

    /// Gets a reference to a session
    #[must_use]
    pub fn get_session(&self, session_id: Uuid) -> Option<&Session> {
        self.sessions.get(&session_id)
    }

    /// Gets a mutable reference to a session
    pub fn get_session_mut(&mut self, session_id: Uuid) -> Option<&mut Session> {
        self.sessions.get_mut(&session_id)
    }

    /// Returns all active sessions
    #[must_use]
    pub fn active_sessions(&self) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.state == SessionState::Active || s.state == SessionState::Starting)
            .collect()
    }

    /// Returns all sessions for a specific connection
    #[must_use]
    pub fn sessions_for_connection(&self, connection_id: Uuid) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.connection_id == connection_id)
            .collect()
    }

    /// Returns the number of active sessions
    #[must_use]
    pub fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.state == SessionState::Active || s.state == SessionState::Starting)
            .count()
    }

    /// Checks and updates the state of all sessions
    ///
    /// This should be called periodically to detect terminated processes.
    pub fn refresh_session_states(&mut self) {
        for session in self.sessions.values_mut() {
            if session.state == SessionState::Active {
                let _ = session.is_running();
            }
        }
    }

    /// Cleans up terminated sessions
    ///
    /// Removes sessions that have been terminated from tracking.
    pub fn cleanup_terminated_sessions(&mut self) {
        self.sessions.retain(|_, session| {
            session.state != SessionState::Terminated && session.state != SessionState::Error
        });
    }

    /// Terminates all active sessions
    ///
    /// # Errors
    /// Returns the first error encountered, but attempts to terminate all sessions
    pub fn terminate_all(&mut self) -> SessionResult<()> {
        let session_ids: Vec<Uuid> = self.sessions.keys().copied().collect();
        let mut first_error: Option<SessionError> = None;

        for session_id in session_ids {
            if let Err(e) = self.terminate_session(session_id) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    /// Returns a reference to a session's logger
    #[must_use]
    pub fn session_logger(&self, session_id: Uuid) -> Option<&SessionLogger> {
        self.session_loggers.get(&session_id)
    }

    /// Returns a mutable reference to a session's logger
    pub fn session_logger_mut(&mut self, session_id: Uuid) -> Option<&mut SessionLogger> {
        self.session_loggers.get_mut(&session_id)
    }

    /// Writes data to a session's log
    ///
    /// # Errors
    /// Returns an error if writing fails
    pub fn write_to_session_log(&mut self, session_id: Uuid, data: &[u8]) -> SessionResult<()> {
        if let Some(logger) = self.session_loggers.get_mut(&session_id) {
            logger
                .write(data)
                .map_err(|e| SessionError::LoggingError(format!("Failed to write to log: {e}")))?;
        }
        Ok(())
    }

    /// Flushes a session's log to disk
    ///
    /// # Errors
    /// Returns an error if flushing fails
    pub fn flush_session_log(&mut self, session_id: Uuid) -> SessionResult<()> {
        if let Some(logger) = self.session_loggers.get_mut(&session_id) {
            logger
                .flush()
                .map_err(|e| SessionError::LoggingError(format!("Failed to flush log: {e}")))?;
        }
        Ok(())
    }

    /// Checks if logging is enabled for a session
    #[must_use]
    pub fn is_logging_enabled_for_session(&self, session_id: Uuid) -> bool {
        self.session_loggers.contains_key(&session_id)
    }

    /// Returns whether logging is globally enabled
    #[must_use]
    pub const fn is_logging_enabled(&self) -> bool {
        self.logging_enabled
    }

    // ========== Health Check Methods ==========

    /// Sets the health check configuration
    pub fn set_health_check_config(&mut self, config: HealthCheckConfig) {
        self.health_check_config = config;
    }

    /// Returns the health check configuration
    #[must_use]
    pub const fn health_check_config(&self) -> &HealthCheckConfig {
        &self.health_check_config
    }

    /// Enables or disables health checks
    pub fn set_health_check_enabled(&mut self, enabled: bool) {
        self.health_check_config.enabled = enabled;
    }

    /// Returns whether health checks are enabled
    #[must_use]
    pub const fn is_health_check_enabled(&self) -> bool {
        self.health_check_config.enabled
    }

    /// Checks if a health check is due based on the configured interval
    #[must_use]
    pub fn is_health_check_due(&self) -> bool {
        if !self.health_check_config.enabled {
            return false;
        }

        self.last_health_check
            .is_none_or(|last| last.elapsed() >= self.health_check_config.interval)
    }

    /// Gets the health status of a specific session
    #[must_use]
    pub fn get_session_health(&self, session_id: Uuid) -> HealthStatus {
        let Some(session) = self.sessions.get(&session_id) else {
            return HealthStatus::Unknown;
        };

        match session.state {
            SessionState::Active => {
                // Check if process is still running
                if session.process().is_some() {
                    HealthStatus::Healthy
                } else {
                    // No process handle - might be embedded session managed by GUI
                    HealthStatus::Healthy
                }
            }
            SessionState::Starting => HealthStatus::Healthy,
            SessionState::Disconnecting => HealthStatus::Healthy, // Still transitioning
            SessionState::Terminated => HealthStatus::Terminated,
            SessionState::Error => HealthStatus::Unhealthy("Session in error state".to_string()),
        }
    }

    /// Performs a health check on all active sessions
    ///
    /// Returns a list of health check events for sessions whose status changed.
    pub fn perform_health_check(&mut self) -> Vec<HealthCheckEvent> {
        if !self.health_check_config.enabled {
            return Vec::new();
        }

        let now = chrono::Utc::now();
        let mut events = Vec::new();

        // First, refresh session states to detect terminated processes
        self.refresh_session_states();

        // Check each session
        let session_ids: Vec<Uuid> = self.sessions.keys().copied().collect();

        for session_id in session_ids {
            let Some(session) = self.sessions.get(&session_id) else {
                continue;
            };

            let current_status = self.get_session_health(session_id);
            let previous_status = self
                .health_status_cache
                .get(&session_id)
                .cloned()
                .unwrap_or(HealthStatus::Unknown);

            // Check if status changed
            if current_status != previous_status {
                events.push(HealthCheckEvent {
                    session_id,
                    connection_id: session.connection_id,
                    session_name: session.connection_name.clone(),
                    previous_status: previous_status.clone(),
                    current_status: current_status.clone(),
                    checked_at: now,
                });

                // Update cache
                self.health_status_cache
                    .insert(session_id, current_status.clone());
            }
        }

        // Update last check timestamp
        self.last_health_check = Some(Instant::now());

        // Auto-cleanup if enabled
        if self.health_check_config.auto_cleanup {
            self.cleanup_terminated_sessions();
        }

        events
    }

    /// Performs a health check only if the interval has elapsed
    ///
    /// Returns `Some(events)` if a check was performed, `None` otherwise.
    pub fn maybe_perform_health_check(&mut self) -> Option<Vec<HealthCheckEvent>> {
        if self.is_health_check_due() {
            Some(self.perform_health_check())
        } else {
            None
        }
    }

    /// Returns all sessions with unhealthy status
    #[must_use]
    pub fn unhealthy_sessions(&self) -> Vec<(Uuid, &Session, HealthStatus)> {
        self.sessions
            .iter()
            .filter_map(|(&id, session)| {
                let status = self.get_session_health(id);
                match status {
                    HealthStatus::Unhealthy(_) => Some((id, session, status)),
                    _ => None,
                }
            })
            .collect()
    }

    /// Clears the health status cache for a session
    ///
    /// Call this when a session is restarted to reset its health tracking.
    pub fn clear_health_cache(&mut self, session_id: Uuid) {
        self.health_status_cache.remove(&session_id);
    }

    /// Clears all health status caches
    pub fn clear_all_health_caches(&mut self) {
        self.health_status_cache.clear();
        self.last_health_check = None;
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_manager_creation() {
        let manager = SessionManager::new();
        assert_eq!(manager.active_session_count(), 0);
    }

    #[test]
    fn test_session_not_found() {
        let mut manager = SessionManager::new();
        let result = manager.terminate_session(Uuid::new_v4());
        assert!(result.is_err());
    }

    #[test]
    fn test_health_check_config_default() {
        let config = HealthCheckConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval, Duration::from_secs(30));
        assert!(!config.auto_cleanup);
    }

    #[test]
    fn test_health_check_config_disabled() {
        let config = HealthCheckConfig::disabled();
        assert!(!config.enabled);
    }

    #[test]
    fn test_health_check_config_builder() {
        let config = HealthCheckConfig::new()
            .with_interval_secs(60)
            .with_auto_cleanup(true);
        assert!(config.enabled);
        assert_eq!(config.interval, Duration::from_secs(60));
        assert!(config.auto_cleanup);
    }

    #[test]
    fn test_health_status_equality() {
        assert_eq!(HealthStatus::Healthy, HealthStatus::Healthy);
        assert_eq!(HealthStatus::Terminated, HealthStatus::Terminated);
        assert_eq!(HealthStatus::Unknown, HealthStatus::Unknown);
        assert_ne!(HealthStatus::Healthy, HealthStatus::Terminated);
    }

    #[test]
    fn test_health_check_due_initially() {
        let manager = SessionManager::new();
        assert!(manager.is_health_check_due());
    }

    #[test]
    fn test_health_check_disabled() {
        let mut manager = SessionManager::new();
        manager.set_health_check_enabled(false);
        assert!(!manager.is_health_check_due());
    }

    #[test]
    fn test_perform_health_check_empty() {
        let mut manager = SessionManager::new();
        let events = manager.perform_health_check();
        assert!(events.is_empty());
    }

    #[test]
    fn test_get_session_health_unknown() {
        let manager = SessionManager::new();
        let status = manager.get_session_health(Uuid::new_v4());
        assert_eq!(status, HealthStatus::Unknown);
    }

    #[test]
    fn test_unhealthy_sessions_empty() {
        let manager = SessionManager::new();
        let unhealthy = manager.unhealthy_sessions();
        assert!(unhealthy.is_empty());
    }

    #[test]
    fn test_clear_health_cache() {
        let mut manager = SessionManager::new();
        let session_id = Uuid::new_v4();
        manager
            .health_status_cache
            .insert(session_id, HealthStatus::Healthy);
        manager.clear_health_cache(session_id);
        assert!(!manager.health_status_cache.contains_key(&session_id));
    }

    #[test]
    fn test_clear_all_health_caches() {
        let mut manager = SessionManager::new();
        manager
            .health_status_cache
            .insert(Uuid::new_v4(), HealthStatus::Healthy);
        manager
            .health_status_cache
            .insert(Uuid::new_v4(), HealthStatus::Terminated);
        manager.last_health_check = Some(Instant::now());

        manager.clear_all_health_caches();

        assert!(manager.health_status_cache.is_empty());
        assert!(manager.last_health_check.is_none());
    }

    #[test]
    fn test_maybe_perform_health_check() {
        let mut manager = SessionManager::new();
        // First call should perform check
        let result = manager.maybe_perform_health_check();
        assert!(result.is_some());

        // Immediate second call should not perform check (interval not elapsed)
        let result = manager.maybe_perform_health_check();
        assert!(result.is_none());
    }
}
