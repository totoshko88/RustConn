//! Session manager for `RustConn`
//!
//! This module provides the `SessionManager` which handles the lifecycle
//! of active connection sessions, including starting, terminating,
//! and tracking sessions.

use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

use crate::error::SessionError;
use crate::models::Connection;
use crate::protocol::ProtocolRegistry;

use super::logger::{LogConfig, LogContext, SessionLogger};
use super::session::{Session, SessionState, SessionType};

/// Result type for session operations
pub type SessionResult<T> = Result<T, SessionError>;

/// Manages active connection sessions
///
/// The `SessionManager` is responsible for:
/// - Starting new sessions for connections
/// - Tracking active sessions
/// - Terminating sessions
/// - Managing session logging
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
        })
    }

    /// Enables or disables session logging
    pub fn set_logging_enabled(&mut self, enabled: bool) {
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
                        eprintln!(
                            "Session logging enabled for '{}': {}",
                            connection.name,
                            log_path.display()
                        );
                        session.set_log_file(log_path);
                        self.session_loggers.insert(session_id, logger);
                    }
                    Err(e) => {
                        // Log detailed error for debugging
                        eprintln!(
                            "Warning: Failed to create session logger for '{}': {}",
                            connection.name, e
                        );
                        eprintln!("  Log config path template: {}", config.path_template);
                    }
                }
            } else {
                eprintln!(
                    "Warning: Logging enabled but no log config set for session '{}'",
                    connection.name
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
                eprintln!("Warning: Failed to close session logger: {e}");
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
                eprintln!("Warning: Failed to close session logger: {e}");
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
}
