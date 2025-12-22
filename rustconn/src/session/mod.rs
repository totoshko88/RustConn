//! Session widget module for native protocol embedding
//!
//! This module provides session widgets for different protocols (SSH, VNC, RDP, SPICE)
//! that can be embedded as native GTK4 widgets within the application.
//!
//! # Architecture
//!
//! Each protocol has its own session widget implementation that wraps the underlying
//! display widget (VTE4 for SSH, gtk-vnc for VNC, etc.) and provides:
//! - Connection lifecycle management
//! - Floating overlay controls
//! - State tracking and error handling
//!
//! # Requirements Coverage
//!
//! - Requirement 2.1: Native VNC embedding as GTK widget
//! - Requirement 2.5: Connection state management and error handling
//! - Requirement 3.1: Native RDP embedding
//! - Requirement 4.2: Native SPICE embedding

pub mod rdp;
pub mod spice;
pub mod vnc;

pub use rdp::RdpSessionWidget;
pub use spice::SpiceSessionWidget;
pub use vnc::VncSessionWidget;

use gtk4::prelude::*;
use std::fmt;
use thiserror::Error;

/// Session widget enum that wraps protocol-specific display widgets
///
/// This enum provides a unified interface for all session types,
/// allowing the terminal notebook to handle different protocols uniformly.
#[derive(Debug)]
pub enum SessionWidget {
    /// SSH session using VTE4 terminal
    Ssh(vte4::Terminal),
    /// VNC session using gtk-vnc
    Vnc(VncSessionWidget),
    /// RDP session using gtk-frdp
    Rdp(RdpSessionWidget),
    /// SPICE session using spice-gtk
    Spice(SpiceSessionWidget),
}

impl SessionWidget {
    /// Returns the underlying GTK widget for embedding in containers
    #[must_use]
    pub fn widget(&self) -> gtk4::Widget {
        match self {
            Self::Ssh(terminal) => terminal.clone().upcast(),
            Self::Vnc(vnc_widget) => vnc_widget.widget().clone(),
            Self::Rdp(rdp_widget) => rdp_widget.widget().clone(),
            Self::Spice(spice_widget) => spice_widget.widget().clone(),
        }
    }

    /// Returns the current session state
    #[must_use]
    pub fn state(&self) -> SessionState {
        match self {
            Self::Ssh(_) => {
                // SSH terminals are always "connected" once created
                // The actual connection state is managed by the shell process
                SessionState::Connected
            }
            Self::Vnc(vnc_widget) => vnc_widget.state(),
            Self::Rdp(rdp_widget) => rdp_widget.state(),
            Self::Spice(spice_widget) => spice_widget.state(),
        }
    }

    /// Returns whether this is an SSH session
    #[must_use]
    pub const fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh(_))
    }

    /// Returns whether this is a VNC session
    #[must_use]
    pub const fn is_vnc(&self) -> bool {
        matches!(self, Self::Vnc(_))
    }

    /// Returns whether this is an RDP session
    #[must_use]
    pub const fn is_rdp(&self) -> bool {
        matches!(self, Self::Rdp(_))
    }

    /// Returns whether this is a SPICE session
    #[must_use]
    pub const fn is_spice(&self) -> bool {
        matches!(self, Self::Spice(_))
    }
}

/// Session state machine for tracking connection lifecycle
///
/// This enum represents the possible states of a remote session.
/// State transitions follow a defined pattern:
/// - Disconnected → Connecting → (Authenticating →)? Connected
/// - Any state → Disconnected or Error
///
/// # Requirements Coverage
///
/// - Requirement 2.5: Connection state management
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SessionState {
    /// Not connected to any remote host
    #[default]
    Disconnected,

    /// Connection attempt in progress
    Connecting,

    /// Waiting for authentication credentials
    Authenticating,

    /// Successfully connected and displaying remote content
    Connected,

    /// Connection failed with an error
    Error(SessionError),
}

impl SessionState {
    /// Returns whether the session is in a connected state
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Returns whether the session is in a connecting state
    #[must_use]
    pub const fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting)
    }

    /// Returns whether the session is in an authenticating state
    #[must_use]
    pub const fn is_authenticating(&self) -> bool {
        matches!(self, Self::Authenticating)
    }

    /// Returns whether the session is disconnected
    #[must_use]
    pub const fn is_disconnected(&self) -> bool {
        matches!(self, Self::Disconnected)
    }

    /// Returns whether the session is in an error state
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Checks if a transition to the target state is valid
    ///
    /// Valid transitions:
    /// - Disconnected → Connecting
    /// - Connecting → Authenticating, Connected, Disconnected, Error
    /// - Authenticating → Connected, Disconnected, Error
    /// - Connected → Disconnected, Error
    /// - Error → Disconnected, Connecting
    #[must_use]
    pub const fn can_transition_to(&self, target: &Self) -> bool {
        match (self, target) {
            // From Disconnected
            (Self::Disconnected, Self::Connecting) => true,
            (Self::Disconnected, Self::Disconnected) => true,

            // From Connecting
            (Self::Connecting, Self::Authenticating) => true,
            (Self::Connecting, Self::Connected) => true,
            (Self::Connecting, Self::Disconnected) => true,
            (Self::Connecting, Self::Error(_)) => true,

            // From Authenticating
            (Self::Authenticating, Self::Connected) => true,
            (Self::Authenticating, Self::Disconnected) => true,
            (Self::Authenticating, Self::Error(_)) => true,

            // From Connected
            (Self::Connected, Self::Disconnected) => true,
            (Self::Connected, Self::Error(_)) => true,

            // From Error - can retry or disconnect
            (Self::Error(_), Self::Disconnected) => true,
            (Self::Error(_), Self::Connecting) => true,

            // All other transitions are invalid
            _ => false,
        }
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Authenticating => write!(f, "Authenticating"),
            Self::Connected => write!(f, "Connected"),
            Self::Error(err) => write!(f, "Error: {err}"),
        }
    }
}

/// Session error types for connection failures
///
/// These errors represent the various failure modes that can occur
/// during session establishment and maintenance.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    /// Connection to remote host failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication with remote host failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Session was disconnected unexpectedly
    #[error("Disconnected: {0}")]
    Disconnected(String),

    /// Protocol-specific error occurred
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// Widget creation or initialization failed
    #[error("Widget error: {0}")]
    WidgetError(String),
}

impl SessionError {
    /// Creates a connection failed error
    #[must_use]
    pub fn connection_failed(msg: impl Into<String>) -> Self {
        Self::ConnectionFailed(msg.into())
    }

    /// Creates an authentication failed error
    #[must_use]
    pub fn authentication_failed(msg: impl Into<String>) -> Self {
        Self::AuthenticationFailed(msg.into())
    }

    /// Creates a disconnected error
    #[must_use]
    pub fn disconnected(msg: impl Into<String>) -> Self {
        Self::Disconnected(msg.into())
    }

    /// Creates a protocol error
    #[must_use]
    pub fn protocol_error(msg: impl Into<String>) -> Self {
        Self::ProtocolError(msg.into())
    }

    /// Creates a widget error
    #[must_use]
    pub fn widget_error(msg: impl Into<String>) -> Self {
        Self::WidgetError(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_default() {
        let state: SessionState = SessionState::default();
        assert_eq!(state, SessionState::Disconnected);
    }

    #[test]
    fn test_session_state_display() {
        assert_eq!(SessionState::Disconnected.to_string(), "Disconnected");
        assert_eq!(SessionState::Connecting.to_string(), "Connecting");
        assert_eq!(SessionState::Authenticating.to_string(), "Authenticating");
        assert_eq!(SessionState::Connected.to_string(), "Connected");
        assert_eq!(
            SessionState::Error(SessionError::connection_failed("timeout")).to_string(),
            "Error: Connection failed: timeout"
        );
    }

    #[test]
    fn test_session_state_predicates() {
        assert!(SessionState::Connected.is_connected());
        assert!(!SessionState::Disconnected.is_connected());

        assert!(SessionState::Connecting.is_connecting());
        assert!(!SessionState::Connected.is_connecting());

        assert!(SessionState::Authenticating.is_authenticating());
        assert!(!SessionState::Connected.is_authenticating());

        assert!(SessionState::Disconnected.is_disconnected());
        assert!(!SessionState::Connected.is_disconnected());

        assert!(SessionState::Error(SessionError::connection_failed("test")).is_error());
        assert!(!SessionState::Connected.is_error());
    }

    #[test]
    fn test_valid_state_transitions() {
        // From Disconnected
        assert!(SessionState::Disconnected.can_transition_to(&SessionState::Connecting));
        assert!(!SessionState::Disconnected.can_transition_to(&SessionState::Connected));

        // From Connecting
        assert!(SessionState::Connecting.can_transition_to(&SessionState::Authenticating));
        assert!(SessionState::Connecting.can_transition_to(&SessionState::Connected));
        assert!(SessionState::Connecting.can_transition_to(&SessionState::Disconnected));
        assert!(
            SessionState::Connecting.can_transition_to(&SessionState::Error(
                SessionError::connection_failed("test")
            ))
        );

        // From Authenticating
        assert!(SessionState::Authenticating.can_transition_to(&SessionState::Connected));
        assert!(SessionState::Authenticating.can_transition_to(&SessionState::Disconnected));
        assert!(!SessionState::Authenticating.can_transition_to(&SessionState::Connecting));

        // From Connected
        assert!(SessionState::Connected.can_transition_to(&SessionState::Disconnected));
        assert!(!SessionState::Connected.can_transition_to(&SessionState::Connecting));

        // From Error
        let error_state = SessionState::Error(SessionError::connection_failed("test"));
        assert!(error_state.can_transition_to(&SessionState::Disconnected));
        assert!(error_state.can_transition_to(&SessionState::Connecting));
        assert!(!error_state.can_transition_to(&SessionState::Connected));
    }

    #[test]
    fn test_session_error_constructors() {
        let err = SessionError::connection_failed("timeout");
        assert!(matches!(err, SessionError::ConnectionFailed(_)));
        assert_eq!(err.to_string(), "Connection failed: timeout");

        let err = SessionError::authentication_failed("invalid password");
        assert!(matches!(err, SessionError::AuthenticationFailed(_)));

        let err = SessionError::disconnected("server closed");
        assert!(matches!(err, SessionError::Disconnected(_)));

        let err = SessionError::protocol_error("unsupported encoding");
        assert!(matches!(err, SessionError::ProtocolError(_)));

        let err = SessionError::widget_error("failed to create display");
        assert!(matches!(err, SessionError::WidgetError(_)));
    }
}
