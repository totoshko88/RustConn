//! Split view model for terminal pane management
//!
//! This module provides a pure data model for split terminal views,
//! allowing property-based testing without GTK dependencies.

use std::collections::HashMap;
use uuid::Uuid;

/// Represents a split direction for terminal panes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Split horizontally (top and bottom panes)
    Horizontal,
    /// Split vertically (left and right panes)
    Vertical,
}

/// A pane in the split view model
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneModel {
    /// Unique identifier for this pane
    pub id: Uuid,
    /// Currently displayed session in this pane (if any)
    pub current_session: Option<Uuid>,
}

impl PaneModel {
    /// Creates a new pane model
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            current_session: None,
        }
    }

    /// Creates a new pane model with a specific ID
    #[must_use]
    pub const fn with_id(id: Uuid) -> Self {
        Self {
            id,
            current_session: None,
        }
    }
}

impl Default for PaneModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Session info for the split view model
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Session UUID
    pub id: Uuid,
    /// Connection ID this session is for
    pub connection_id: Uuid,
    /// Connection name for display
    pub name: String,
}

impl SessionInfo {
    /// Creates a new session info
    #[must_use]
    pub fn new(connection_id: Uuid, name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            connection_id,
            name,
        }
    }
}

/// Pure data model for split terminal views
///
/// This model contains the logic for managing split panes without
/// any GTK dependencies, making it suitable for property-based testing.
#[derive(Debug, Clone)]
pub struct SplitViewModel {
    /// All panes in the view
    panes: Vec<PaneModel>,
    /// Currently focused pane ID
    focused_pane: Option<Uuid>,
    /// Shared sessions map (`session_id` -> `SessionInfo`)
    sessions: HashMap<Uuid, SessionInfo>,
}

impl SplitViewModel {
    /// Creates a new split view model with one initial pane
    #[must_use]
    pub fn new() -> Self {
        let initial_pane = PaneModel::new();
        let initial_pane_id = initial_pane.id;

        Self {
            panes: vec![initial_pane],
            focused_pane: Some(initial_pane_id),
            sessions: HashMap::new(),
        }
    }

    /// Returns the number of panes
    #[must_use]
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// Returns the focused pane ID
    #[must_use]
    pub const fn focused_pane_id(&self) -> Option<Uuid> {
        self.focused_pane
    }

    /// Returns all pane IDs
    #[must_use]
    pub fn pane_ids(&self) -> Vec<Uuid> {
        self.panes.iter().map(|p| p.id).collect()
    }

    /// Returns the number of sessions
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Returns all session IDs
    #[must_use]
    pub fn session_ids(&self) -> Vec<Uuid> {
        self.sessions.keys().copied().collect()
    }

    /// Adds a session to the shared session list
    pub fn add_session(&mut self, session: SessionInfo) {
        self.sessions.insert(session.id, session);
    }

    /// Removes a session from the shared session list
    pub fn remove_session(&mut self, session_id: Uuid) {
        self.sessions.remove(&session_id);
        // Clear session from any panes that were showing it
        for pane in &mut self.panes {
            if pane.current_session == Some(session_id) {
                pane.current_session = None;
            }
        }
    }

    /// Gets session info by ID
    #[must_use]
    pub fn get_session(&self, session_id: Uuid) -> Option<&SessionInfo> {
        self.sessions.get(&session_id)
    }

    /// Splits the focused pane in the given direction
    ///
    /// Returns the ID of the new pane, or None if there's no focused pane.
    pub fn split(&mut self, _direction: SplitDirection) -> Option<Uuid> {
        let focused_id = self.focused_pane?;

        // Verify focused pane exists
        if !self.panes.iter().any(|p| p.id == focused_id) {
            return None;
        }

        // Create new pane
        let new_pane = PaneModel::new();
        let new_pane_id = new_pane.id;
        self.panes.push(new_pane);

        Some(new_pane_id)
    }

    /// Closes the focused pane
    ///
    /// Returns true if a pane was closed, false if there's only one pane
    /// or no focused pane.
    pub fn close_pane(&mut self) -> bool {
        // Can't close if only one pane
        if self.panes.len() <= 1 {
            return false;
        }

        let Some(focused_id) = self.focused_pane else {
            return false;
        };

        // Find and remove the focused pane
        let Some(index) = self.panes.iter().position(|p| p.id == focused_id) else {
            return false;
        };

        self.panes.remove(index);

        // Update focus to another pane
        if self.panes.is_empty() {
            self.focused_pane = None;
        } else {
            // Focus the pane at the same index, or the last one if we removed the last
            let new_index = index.min(self.panes.len() - 1);
            self.focused_pane = Some(self.panes[new_index].id);
        }

        true
    }

    /// Cycles focus to the next pane
    ///
    /// Returns the ID of the newly focused pane, or None if there are no panes.
    pub fn focus_next_pane(&mut self) -> Option<Uuid> {
        if self.panes.is_empty() {
            return None;
        }

        let current_index = self
            .focused_pane
            .and_then(|id| self.panes.iter().position(|p| p.id == id))
            .unwrap_or(0);

        let next_index = (current_index + 1) % self.panes.len();
        let next_id = self.panes[next_index].id;
        self.focused_pane = Some(next_id);

        Some(next_id)
    }

    /// Sets the focused pane by ID
    ///
    /// Returns true if the pane was found and focused.
    pub fn set_focused_pane(&mut self, pane_id: Uuid) -> bool {
        if self.panes.iter().any(|p| p.id == pane_id) {
            self.focused_pane = Some(pane_id);
            true
        } else {
            false
        }
    }

    /// Shows a session in the focused pane
    ///
    /// Returns true if the session was shown successfully.
    pub fn show_session(&mut self, session_id: Uuid) -> bool {
        let Some(focused_id) = self.focused_pane else {
            return false;
        };

        // Verify session exists
        if !self.sessions.contains_key(&session_id) {
            return false;
        }

        // Find focused pane and set its session
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == focused_id) {
            pane.current_session = Some(session_id);
            true
        } else {
            false
        }
    }

    /// Gets the focused pane's current session
    #[must_use]
    pub fn get_focused_session(&self) -> Option<Uuid> {
        let focused_id = self.focused_pane?;
        self.panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.current_session)
    }
}

impl Default for SplitViewModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_split_view_has_one_pane() {
        let view = SplitViewModel::new();
        assert_eq!(view.pane_count(), 1);
        assert!(view.focused_pane_id().is_some());
    }

    #[test]
    fn test_split_increases_pane_count() {
        let mut view = SplitViewModel::new();
        let initial_count = view.pane_count();

        view.split(SplitDirection::Horizontal);

        assert_eq!(view.pane_count(), initial_count + 1);
    }

    #[test]
    fn test_close_pane_decreases_count() {
        let mut view = SplitViewModel::new();
        view.split(SplitDirection::Horizontal);
        let count_after_split = view.pane_count();

        view.close_pane();

        assert_eq!(view.pane_count(), count_after_split - 1);
    }

    #[test]
    fn test_cannot_close_last_pane() {
        let mut view = SplitViewModel::new();
        assert_eq!(view.pane_count(), 1);

        let closed = view.close_pane();

        assert!(!closed);
        assert_eq!(view.pane_count(), 1);
    }

    #[test]
    fn test_focus_cycling() {
        let mut view = SplitViewModel::new();
        view.split(SplitDirection::Horizontal);
        view.split(SplitDirection::Vertical);

        let initial_focus = view.focused_pane_id();
        let pane_ids = view.pane_ids();

        // Cycle through all panes
        for _ in 0..pane_ids.len() {
            view.focus_next_pane();
        }

        // Should be back to initial focus
        assert_eq!(view.focused_pane_id(), initial_focus);
    }
}
