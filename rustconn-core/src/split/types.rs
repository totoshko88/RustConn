//! Core type definitions for the split view redesign
//!
//! This module contains the fundamental identifier types and enums used
//! throughout the split view system.

use std::fmt;
use uuid::Uuid;

/// Unique identifier for a panel within a split layout.
///
/// Each panel in a split container has a unique ID that persists
/// throughout its lifetime, even as the tree structure changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PanelId(pub Uuid);

impl PanelId {
    /// Creates a new random panel ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PanelId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PanelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Panel({})", self.0)
    }
}

/// Unique identifier for a tab.
///
/// Each root tab in the tab bar has a unique ID that identifies it
/// and its associated split layout (if any).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub Uuid);

impl TabId {
    /// Creates a new random tab ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TabId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tab({})", self.0)
    }
}

/// Unique identifier for a session.
///
/// A session represents an active connection displayed in a panel.
/// Sessions can be moved between panels and tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Creates a new random session ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a session ID from an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Returns the inner UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Session({})", self.0)
    }
}

/// A color identifier (index into palette).
///
/// Each split container is assigned a unique color from a predefined
/// palette for visual identification. The color is displayed in both
/// the tab header and panel borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorId(pub u8);

impl ColorId {
    /// Creates a new color ID with the given index.
    #[must_use]
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    /// Returns the color index.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.0
    }
}

impl fmt::Display for ColorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Color({})", self.0)
    }
}

/// Split direction for dividing panels.
///
/// When a panel is split, it is divided into two child panels
/// arranged either horizontally (top/bottom) or vertically (left/right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Split horizontally, creating top and bottom panels.
    Horizontal,
    /// Split vertically, creating left and right panels.
    Vertical,
}

impl fmt::Display for SplitDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Horizontal => write!(f, "Horizontal"),
            Self::Vertical => write!(f, "Vertical"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_id_new_creates_unique_ids() {
        let id1 = PanelId::new();
        let id2 = PanelId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn panel_id_equality() {
        let uuid = Uuid::new_v4();
        let id1 = PanelId(uuid);
        let id2 = PanelId(uuid);
        assert_eq!(id1, id2);
    }

    #[test]
    fn tab_id_new_creates_unique_ids() {
        let id1 = TabId::new();
        let id2 = TabId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_id_new_creates_unique_ids() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn color_id_index() {
        let color = ColorId::new(5);
        assert_eq!(color.index(), 5);
    }

    #[test]
    fn split_direction_display() {
        assert_eq!(format!("{}", SplitDirection::Horizontal), "Horizontal");
        assert_eq!(format!("{}", SplitDirection::Vertical), "Vertical");
    }

    #[test]
    fn panel_id_display() {
        let uuid = Uuid::nil();
        let id = PanelId(uuid);
        assert!(format!("{id}").contains("Panel("));
    }

    #[test]
    fn tab_id_display() {
        let uuid = Uuid::nil();
        let id = TabId(uuid);
        assert!(format!("{id}").contains("Tab("));
    }

    #[test]
    fn session_id_display() {
        let uuid = Uuid::nil();
        let id = SessionId(uuid);
        assert!(format!("{id}").contains("Session("));
    }

    #[test]
    fn color_id_display() {
        let color = ColorId::new(3);
        assert_eq!(format!("{color}"), "Color(3)");
    }
}
