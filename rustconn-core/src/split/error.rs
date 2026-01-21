//! Error types for split view operations
//!
//! This module defines the error types and result enums used throughout
//! the split view system.

use super::types::{PanelId, SessionId};

/// Errors that can occur during split view operations.
#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    /// No panel is currently focused.
    #[error("no panel is currently focused")]
    NoFocusedPanel,

    /// The specified panel was not found.
    #[error("panel not found: {0}")]
    PanelNotFound(PanelId),

    /// Cannot remove the last panel in a layout.
    #[error("cannot remove the last panel")]
    CannotRemoveLastPanel,

    /// Invalid split position (must be between 0.0 and 1.0).
    #[error("invalid split position: {0} (must be between 0.0 and 1.0)")]
    InvalidPosition(f64),

    /// The specified session was not found.
    #[error("session not found: {0}")]
    SessionNotFound(SessionId),
}

/// Result of placing a session in a panel.
///
/// When a session is placed in a panel, it either fills an empty panel
/// or evicts an existing session from an occupied panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropResult {
    /// Session was placed in an empty panel.
    Placed,
    /// Session was placed, existing session was evicted.
    Evicted {
        /// The session that was displaced.
        evicted_session: SessionId,
    },
}

impl DropResult {
    /// Returns true if a session was evicted.
    #[must_use]
    pub const fn is_evicted(&self) -> bool {
        matches!(self, Self::Evicted { .. })
    }

    /// Returns the evicted session ID, if any.
    #[must_use]
    pub const fn evicted_session(&self) -> Option<SessionId> {
        match self {
            Self::Placed => None,
            Self::Evicted { evicted_session } => Some(*evicted_session),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_result_placed_is_not_evicted() {
        let result = DropResult::Placed;
        assert!(!result.is_evicted());
        assert!(result.evicted_session().is_none());
    }

    #[test]
    fn drop_result_evicted_is_evicted() {
        let session = SessionId::new();
        let result = DropResult::Evicted {
            evicted_session: session,
        };
        assert!(result.is_evicted());
        assert_eq!(result.evicted_session(), Some(session));
    }

    #[test]
    fn split_error_display_no_focused_panel() {
        let err = SplitError::NoFocusedPanel;
        assert_eq!(format!("{err}"), "no panel is currently focused");
    }

    #[test]
    fn split_error_display_panel_not_found() {
        let id = PanelId::new();
        let err = SplitError::PanelNotFound(id);
        assert!(format!("{err}").contains("panel not found"));
    }

    #[test]
    fn split_error_display_cannot_remove_last_panel() {
        let err = SplitError::CannotRemoveLastPanel;
        assert_eq!(format!("{err}"), "cannot remove the last panel");
    }

    #[test]
    fn split_error_display_invalid_position() {
        let err = SplitError::InvalidPosition(1.5);
        assert!(format!("{err}").contains("invalid split position"));
        assert!(format!("{err}").contains("1.5"));
    }

    #[test]
    fn split_error_display_session_not_found() {
        let id = SessionId::new();
        let err = SplitError::SessionNotFound(id);
        assert!(format!("{err}").contains("session not found"));
    }
}
