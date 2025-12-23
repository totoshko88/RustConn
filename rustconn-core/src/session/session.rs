//! Session data structures
//!
//! This module defines the Session struct and related types for tracking
//! active connection sessions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Child;
use tracing::{debug, info_span};
use uuid::Uuid;

use crate::tracing::span_names;

/// Represents the current state of a session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SessionState {
    /// Session is starting up
    #[default]
    Starting,
    /// Session is active and connected
    Active,
    /// Session is disconnecting
    Disconnecting,
    /// Session has been terminated
    Terminated,
    /// Session encountered an error
    Error,
}

/// Type of session based on protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionType {
    /// SSH session displayed in embedded terminal
    Embedded,
    /// RDP/VNC session running in external window
    External,
}

/// Represents an active connection session
pub struct Session {
    /// Unique identifier for this session
    pub id: Uuid,
    /// ID of the connection this session is for
    pub connection_id: Uuid,
    /// Name of the connection (for display)
    pub connection_name: String,
    /// Protocol being used (ssh, rdp, vnc)
    pub protocol: String,
    /// Current state of the session
    pub state: SessionState,
    /// Type of session (embedded terminal or external window)
    pub session_type: SessionType,
    /// Timestamp when the session was started
    pub started_at: DateTime<Utc>,
    /// Timestamp when the session ended (if terminated)
    pub ended_at: Option<DateTime<Utc>>,
    /// Path to the log file for this session
    pub log_file: Option<PathBuf>,
    /// The child process handle (if running)
    process: Option<Child>,
}

impl Session {
    /// Creates a new session
    #[must_use]
    pub fn new(
        connection_id: Uuid,
        connection_name: String,
        protocol: String,
        session_type: SessionType,
    ) -> Self {
        let _span = info_span!(
            span_names::SESSION_START,
            connection_id = %connection_id,
            protocol = %protocol,
            session_type = ?session_type
        )
        .entered();

        debug!(
            connection_name = %connection_name,
            "Creating new session"
        );

        Self {
            id: Uuid::new_v4(),
            connection_id,
            connection_name,
            protocol,
            state: SessionState::Starting,
            session_type,
            started_at: Utc::now(),
            ended_at: None,
            log_file: None,
            process: None,
        }
    }

    /// Sets the process handle for this session
    pub fn set_process(&mut self, process: Child) {
        self.process = Some(process);
        self.state = SessionState::Active;
    }

    /// Sets the log file path for this session
    pub fn set_log_file(&mut self, path: PathBuf) {
        self.log_file = Some(path);
    }

    /// Returns the process ID if the session has a running process
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().map(std::process::Child::id)
    }

    /// Checks if the process is still running
    #[must_use]
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut process) = self.process {
            match process.try_wait() {
                Ok(Some(_)) => {
                    // Process has exited
                    self.state = SessionState::Terminated;
                    self.ended_at = Some(Utc::now());
                    false
                }
                Ok(None) => {
                    // Process is still running
                    true
                }
                Err(_) => {
                    // Error checking process status
                    self.state = SessionState::Error;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Terminates the session process
    ///
    /// # Errors
    /// Returns an error if the process cannot be terminated
    pub fn terminate(&mut self) -> std::io::Result<()> {
        self.state = SessionState::Disconnecting;

        if let Some(ref mut process) = self.process {
            // Use kill() which sends SIGKILL on Unix
            // For a more graceful shutdown, the terminal widget should handle SIGTERM
            process.kill()?;

            // Wait for process to exit
            let _ = process.wait();
        }

        self.state = SessionState::Terminated;
        self.ended_at = Some(Utc::now());
        Ok(())
    }

    /// Force kills the session process
    ///
    /// # Errors
    /// Returns an error if the process cannot be killed
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.state = SessionState::Disconnecting;

        if let Some(ref mut process) = self.process {
            process.kill()?;
            let _ = process.wait();
        }

        self.state = SessionState::Terminated;
        self.ended_at = Some(Utc::now());
        Ok(())
    }

    /// Takes ownership of the process handle
    pub const fn take_process(&mut self) -> Option<Child> {
        self.process.take()
    }

    /// Returns a reference to the process if available
    #[must_use]
    pub const fn process(&self) -> Option<&Child> {
        self.process.as_ref()
    }

    /// Returns a mutable reference to the process if available
    pub const fn process_mut(&mut self) -> Option<&mut Child> {
        self.process.as_mut()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("connection_id", &self.connection_id)
            .field("connection_name", &self.connection_name)
            .field("protocol", &self.protocol)
            .field("state", &self.state)
            .field("session_type", &self.session_type)
            .field("started_at", &self.started_at)
            .field("ended_at", &self.ended_at)
            .field("log_file", &self.log_file)
            .field("pid", &self.process.as_ref().map(std::process::Child::id))
            .finish()
    }
}
