//! Session restore functionality
//!
//! This module provides structures and functions for persisting and restoring
//! session state across application restarts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::session::SessionType;

/// Data needed to restore a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRestoreData {
    /// Connection ID to reconnect to
    pub connection_id: Uuid,
    /// Connection name (for display during restore)
    pub connection_name: String,
    /// Protocol type
    pub protocol: String,
    /// Session type (embedded or external)
    pub session_type: SessionType,
    /// When the session was originally started
    pub original_start_time: DateTime<Utc>,
    /// When the session state was saved
    pub saved_at: DateTime<Utc>,
    /// Optional panel ID for split view restoration
    pub panel_id: Option<String>,
    /// Tab index in the notebook (for ordering)
    pub tab_index: Option<usize>,
}

impl SessionRestoreData {
    /// Creates new session restore data
    #[must_use]
    pub fn new(
        connection_id: Uuid,
        connection_name: String,
        protocol: String,
        session_type: SessionType,
    ) -> Self {
        Self {
            connection_id,
            connection_name,
            protocol,
            session_type,
            original_start_time: Utc::now(),
            saved_at: Utc::now(),
            panel_id: None,
            tab_index: None,
        }
    }

    /// Sets the panel ID for split view restoration
    #[must_use]
    pub fn with_panel_id(mut self, panel_id: impl Into<String>) -> Self {
        self.panel_id = Some(panel_id.into());
        self
    }

    /// Sets the tab index
    #[must_use]
    pub const fn with_tab_index(mut self, index: usize) -> Self {
        self.tab_index = Some(index);
        self
    }

    /// Updates the saved_at timestamp
    pub fn touch(&mut self) {
        self.saved_at = Utc::now();
    }
}

/// Split panel restore data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelRestoreData {
    /// Panel identifier
    pub panel_id: String,
    /// Session in this panel (if any)
    pub session: Option<SessionRestoreData>,
    /// Panel position (0.0 to 1.0 for split ratio)
    pub position: f64,
}

/// Split view layout restore data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitLayoutRestoreData {
    /// Whether the view is split
    pub is_split: bool,
    /// Split orientation (true = horizontal, false = vertical)
    pub horizontal: bool,
    /// Split ratio (0.0 to 1.0)
    pub split_ratio: f64,
    /// Panels in the split view
    pub panels: Vec<PanelRestoreData>,
}

impl Default for SplitLayoutRestoreData {
    fn default() -> Self {
        Self {
            is_split: false,
            horizontal: true,
            split_ratio: 0.5,
            panels: Vec::new(),
        }
    }
}

impl SplitLayoutRestoreData {
    /// Creates a new empty split layout
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a split layout with the given orientation
    #[must_use]
    pub fn split(horizontal: bool, ratio: f64) -> Self {
        Self {
            is_split: true,
            horizontal,
            split_ratio: ratio.clamp(0.1, 0.9),
            panels: Vec::new(),
        }
    }

    /// Adds a panel to the layout
    pub fn add_panel(&mut self, panel: PanelRestoreData) {
        self.panels.push(panel);
    }
}

/// Complete session restore state
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionRestoreState {
    /// Version for forward compatibility
    pub version: u32,
    /// When the state was saved
    pub saved_at: DateTime<Utc>,
    /// Active sessions to restore
    pub sessions: Vec<SessionRestoreData>,
    /// Split view layout
    pub split_layout: Option<SplitLayoutRestoreData>,
    /// ID of the focused/active session
    pub active_session_id: Option<Uuid>,
    /// Window geometry (x, y, width, height)
    pub window_geometry: Option<(i32, i32, i32, i32)>,
    /// Whether the window was maximized
    pub window_maximized: bool,
}

/// Current version of the restore state format
pub const RESTORE_STATE_VERSION: u32 = 1;

impl SessionRestoreState {
    /// Creates a new empty restore state
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: RESTORE_STATE_VERSION,
            saved_at: Utc::now(),
            sessions: Vec::new(),
            split_layout: None,
            active_session_id: None,
            window_geometry: None,
            window_maximized: false,
        }
    }

    /// Adds a session to restore
    pub fn add_session(&mut self, session: SessionRestoreData) {
        self.sessions.push(session);
    }

    /// Sets the split layout
    pub fn set_split_layout(&mut self, layout: SplitLayoutRestoreData) {
        self.split_layout = Some(layout);
    }

    /// Sets the active session ID
    pub fn set_active_session(&mut self, session_id: Uuid) {
        self.active_session_id = Some(session_id);
    }

    /// Sets the window geometry
    pub fn set_window_geometry(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.window_geometry = Some((x, y, width, height));
    }

    /// Sets whether the window was maximized
    pub fn set_window_maximized(&mut self, maximized: bool) {
        self.window_maximized = maximized;
    }

    /// Returns the number of sessions to restore
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Checks if there are any sessions to restore
    #[must_use]
    pub fn has_sessions(&self) -> bool {
        !self.sessions.is_empty()
    }

    /// Updates the saved_at timestamp
    pub fn touch(&mut self) {
        self.saved_at = Utc::now();
    }

    /// Clears all sessions
    pub fn clear(&mut self) {
        self.sessions.clear();
        self.split_layout = None;
        self.active_session_id = None;
    }

    /// Serializes the state to JSON
    ///
    /// # Errors
    /// Returns an error if serialization fails
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserializes the state from JSON
    ///
    /// # Errors
    /// Returns an error if deserialization fails
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Saves the state to a file
    ///
    /// # Errors
    /// Returns an error if writing fails
    pub fn save_to_file(&self, path: &PathBuf) -> Result<(), SessionRestoreError> {
        let json = self.to_json().map_err(SessionRestoreError::Serialization)?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(SessionRestoreError::Io)?;
        }

        std::fs::write(path, json).map_err(SessionRestoreError::Io)
    }

    /// Loads the state from a file
    ///
    /// # Errors
    /// Returns an error if reading or parsing fails
    pub fn load_from_file(path: &PathBuf) -> Result<Self, SessionRestoreError> {
        let json = std::fs::read_to_string(path).map_err(SessionRestoreError::Io)?;
        Self::from_json(&json).map_err(SessionRestoreError::Deserialization)
    }
}

/// Errors that can occur during session restore operations
#[derive(Debug, thiserror::Error)]
pub enum SessionRestoreError {
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(serde_json::Error),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    Deserialization(serde_json::Error),

    /// Version mismatch
    #[error("Incompatible restore state version: expected {expected}, got {actual}")]
    VersionMismatch {
        /// Expected version
        expected: u32,
        /// Actual version found
        actual: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_restore_data_new() {
        let data = SessionRestoreData::new(
            Uuid::new_v4(),
            "Test Server".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
        );
        assert_eq!(data.connection_name, "Test Server");
        assert_eq!(data.protocol, "ssh");
        assert!(data.panel_id.is_none());
    }

    #[test]
    fn test_session_restore_data_builder() {
        let data = SessionRestoreData::new(
            Uuid::new_v4(),
            "Test".to_string(),
            "rdp".to_string(),
            SessionType::External,
        )
        .with_panel_id("panel-1")
        .with_tab_index(2);

        assert_eq!(data.panel_id, Some("panel-1".to_string()));
        assert_eq!(data.tab_index, Some(2));
    }

    #[test]
    fn test_split_layout_default() {
        let layout = SplitLayoutRestoreData::default();
        assert!(!layout.is_split);
        assert!(layout.panels.is_empty());
    }

    #[test]
    fn test_split_layout_split() {
        let layout = SplitLayoutRestoreData::split(true, 0.3);
        assert!(layout.is_split);
        assert!(layout.horizontal);
        assert!((layout.split_ratio - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_split_layout_ratio_clamping() {
        let layout = SplitLayoutRestoreData::split(false, 0.05);
        assert!((layout.split_ratio - 0.1).abs() < f64::EPSILON);

        let layout = SplitLayoutRestoreData::split(false, 0.95);
        assert!((layout.split_ratio - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_session_restore_state_new() {
        let state = SessionRestoreState::new();
        assert_eq!(state.version, RESTORE_STATE_VERSION);
        assert!(state.sessions.is_empty());
        assert!(!state.has_sessions());
    }

    #[test]
    fn test_session_restore_state_add_session() {
        let mut state = SessionRestoreState::new();
        let session = SessionRestoreData::new(
            Uuid::new_v4(),
            "Server".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
        );
        state.add_session(session);

        assert_eq!(state.session_count(), 1);
        assert!(state.has_sessions());
    }

    #[test]
    fn test_session_restore_state_clear() {
        let mut state = SessionRestoreState::new();
        state.add_session(SessionRestoreData::new(
            Uuid::new_v4(),
            "Server".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
        ));
        state.set_active_session(Uuid::new_v4());

        state.clear();

        assert!(!state.has_sessions());
        assert!(state.active_session_id.is_none());
    }

    #[test]
    fn test_session_restore_state_serialization() {
        let mut state = SessionRestoreState::new();
        state.add_session(SessionRestoreData::new(
            Uuid::new_v4(),
            "Test Server".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
        ));
        state.set_window_geometry(100, 100, 800, 600);
        state.set_window_maximized(false);

        let json = state.to_json().expect("serialization should succeed");
        let restored =
            SessionRestoreState::from_json(&json).expect("deserialization should succeed");

        assert_eq!(restored.session_count(), 1);
        assert_eq!(restored.window_geometry, Some((100, 100, 800, 600)));
        assert!(!restored.window_maximized);
    }

    #[test]
    fn test_session_restore_state_file_roundtrip() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let path = temp_dir.path().join("sessions.json");

        let mut state = SessionRestoreState::new();
        state.add_session(SessionRestoreData::new(
            Uuid::new_v4(),
            "File Test".to_string(),
            "vnc".to_string(),
            SessionType::External,
        ));

        state.save_to_file(&path).expect("save should succeed");
        let loaded = SessionRestoreState::load_from_file(&path).expect("load should succeed");

        assert_eq!(loaded.session_count(), 1);
        assert_eq!(loaded.sessions[0].connection_name, "File Test");
    }

    #[test]
    fn test_panel_restore_data() {
        let panel = PanelRestoreData {
            panel_id: "main".to_string(),
            session: Some(SessionRestoreData::new(
                Uuid::new_v4(),
                "Panel Session".to_string(),
                "ssh".to_string(),
                SessionType::Embedded,
            )),
            position: 0.5,
        };

        assert_eq!(panel.panel_id, "main");
        assert!(panel.session.is_some());
    }
}
