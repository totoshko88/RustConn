//! Split layout model for managing tab-scoped panel layouts
//!
//! This module provides the `SplitLayoutModel` struct which manages the
//! panel tree structure for a single tab. Each tab can have its own
//! independent split layout.
//!
//! # Example
//!
//! ```
//! use rustconn_core::split::{SplitLayoutModel, SplitDirection, SessionId, DropResult};
//!
//! let mut layout = SplitLayoutModel::new();
//!
//! // Initially, there's one panel with no splits
//! assert!(!layout.is_split());
//! assert_eq!(layout.panel_count(), 1);
//!
//! // Split the focused panel vertically
//! let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();
//!
//! // Now we have two panels
//! assert!(layout.is_split());
//! assert_eq!(layout.panel_count(), 2);
//!
//! // Place a session in the new panel
//! let session = SessionId::new();
//! let result = layout.place_in_panel(new_panel_id, session).unwrap();
//! assert!(matches!(result, DropResult::Placed));
//! ```

use super::error::{DropResult, SplitError};
use super::tree::{LeafPanel, PanelNode, RemoveResult, SplitNode};
use super::types::{ColorId, PanelId, SessionId, SplitDirection};

/// Manages the split layout for a single tab.
///
/// Each tab in the application can have its own `SplitLayoutModel` that
/// tracks the panel tree structure, color identification, and focus state.
///
/// # Layout States
///
/// - **Single panel**: `root` is `None`, representing a single unsplit panel
/// - **Split container**: `root` is `Some(PanelNode)`, representing a tree of panels
///
/// # Focus Tracking
///
/// The model tracks which panel is currently focused. Split operations
/// act on the focused panel.
#[derive(Debug, Clone)]
pub struct SplitLayoutModel {
    /// Root of the panel tree (None = single panel, no splits).
    root: Option<PanelNode>,
    /// Unique color ID for this split container.
    color_id: Option<ColorId>,
    /// ID of the currently focused panel.
    focused_panel: Option<PanelId>,
    /// The single panel used when not split.
    single_panel: LeafPanel,
}

impl SplitLayoutModel {
    /// Creates a new layout with a single empty panel.
    ///
    /// The layout starts with no splits and a single panel that is
    /// automatically focused.
    #[must_use]
    pub fn new() -> Self {
        let single_panel = LeafPanel::new();
        let focused_panel = Some(single_panel.id);
        Self {
            root: None,
            color_id: None,
            focused_panel,
            single_panel,
        }
    }

    /// Creates a new layout with a session in the initial panel.
    ///
    /// This is useful when creating a split layout from an existing
    /// connection tab.
    #[must_use]
    pub fn with_session(session: SessionId) -> Self {
        let single_panel = LeafPanel::with_session(session);
        let focused_panel = Some(single_panel.id);
        Self {
            root: None,
            color_id: None,
            focused_panel,
            single_panel,
        }
    }

    /// Returns true if this layout has splits (is a split container).
    ///
    /// A layout without splits has a single panel and `is_split()` returns `false`.
    #[must_use]
    pub const fn is_split(&self) -> bool {
        self.root.is_some()
    }

    /// Returns the total number of panels in the layout.
    ///
    /// A layout always has at least one panel.
    #[must_use]
    pub fn panel_count(&self) -> usize {
        match &self.root {
            None => 1,
            Some(node) => node.panel_count(),
        }
    }

    /// Returns all panel IDs in the layout.
    ///
    /// The IDs are returned in tree traversal order (depth-first, left-to-right).
    #[must_use]
    pub fn panel_ids(&self) -> Vec<PanelId> {
        match &self.root {
            None => vec![self.single_panel.id],
            Some(node) => node.panel_ids(),
        }
    }

    /// Returns the color ID assigned to this split container.
    ///
    /// Returns `None` if no color has been assigned (typically when not split).
    #[must_use]
    pub const fn color_id(&self) -> Option<ColorId> {
        self.color_id
    }

    /// Sets the color ID for this split container.
    pub fn set_color_id(&mut self, color_id: ColorId) {
        self.color_id = Some(color_id);
    }

    /// Clears the color ID for this split container.
    pub fn clear_color_id(&mut self) {
        self.color_id = None;
    }

    /// Returns the ID of the currently focused panel.
    #[must_use]
    pub const fn get_focused_panel(&self) -> Option<PanelId> {
        self.focused_panel
    }

    /// Sets focus to a specific panel.
    ///
    /// # Errors
    ///
    /// Returns `SplitError::PanelNotFound` if the panel doesn't exist in this layout.
    pub fn set_focus(&mut self, panel_id: PanelId) -> Result<(), SplitError> {
        if self.contains_panel(panel_id) {
            self.focused_panel = Some(panel_id);
            Ok(())
        } else {
            Err(SplitError::PanelNotFound(panel_id))
        }
    }

    /// Returns true if the layout contains a panel with the given ID.
    #[must_use]
    pub fn contains_panel(&self, panel_id: PanelId) -> bool {
        match &self.root {
            None => self.single_panel.id == panel_id,
            Some(node) => node.contains_panel(panel_id),
        }
    }

    /// Returns the session in a panel (if any).
    ///
    /// Returns `None` if the panel is empty or doesn't exist.
    #[must_use]
    pub fn get_panel_session(&self, panel_id: PanelId) -> Option<SessionId> {
        match &self.root {
            None => {
                if self.single_panel.id == panel_id {
                    self.single_panel.session
                } else {
                    None
                }
            }
            Some(node) => node.find_panel(panel_id).and_then(|p| p.session),
        }
    }

    /// Splits the focused panel in the given direction.
    ///
    /// The focused panel is replaced by a split node containing:
    /// - First child: the original panel (with its session)
    /// - Second child: a new empty panel
    ///
    /// After the split, focus remains on the original panel.
    ///
    /// # Returns
    ///
    /// Returns the ID of the newly created panel.
    ///
    /// # Errors
    ///
    /// Returns `SplitError::NoFocusedPanel` if no panel is focused.
    pub fn split(&mut self, direction: SplitDirection) -> Result<PanelId, SplitError> {
        let focused_id = self.focused_panel.ok_or(SplitError::NoFocusedPanel)?;

        match &mut self.root {
            None => {
                // First split: convert single panel to split tree
                let new_panel = LeafPanel::new();
                let new_panel_id = new_panel.id;

                // Create the split node with original panel as first child
                let original_panel = LeafPanel {
                    id: self.single_panel.id,
                    session: self.single_panel.session,
                };

                self.root = Some(PanelNode::Split(SplitNode::new(
                    direction,
                    PanelNode::Leaf(original_panel),
                    PanelNode::Leaf(new_panel),
                )));

                Ok(new_panel_id)
            }
            Some(node) => {
                // Split an existing panel in the tree
                node.insert_split(focused_id, direction)
                    .ok_or(SplitError::PanelNotFound(focused_id))
            }
        }
    }

    /// Places a session in the specified panel.
    ///
    /// If the panel is empty, the session is placed directly.
    /// If the panel is occupied, the existing session is evicted.
    ///
    /// # Returns
    ///
    /// Returns `DropResult::Placed` if the panel was empty, or
    /// `DropResult::Evicted` with the displaced session if occupied.
    ///
    /// # Errors
    ///
    /// Returns `SplitError::PanelNotFound` if the panel doesn't exist.
    pub fn place_in_panel(
        &mut self,
        panel_id: PanelId,
        session_id: SessionId,
    ) -> Result<DropResult, SplitError> {
        let panel = self.find_panel_mut(panel_id)?;

        let result = if let Some(existing_session) = panel.session {
            DropResult::Evicted {
                evicted_session: existing_session,
            }
        } else {
            DropResult::Placed
        };

        panel.session = Some(session_id);
        Ok(result)
    }

    /// Removes a panel from the layout.
    ///
    /// When a panel is removed:
    /// - If it's the last panel, returns `CannotRemoveLastPanel` error
    /// - If it's in a split, the sibling is promoted to replace the split
    /// - The tree is collapsed as needed to maintain structure
    ///
    /// # Returns
    ///
    /// Returns the session that was in the removed panel (if any).
    ///
    /// # Errors
    ///
    /// - `SplitError::PanelNotFound` if the panel doesn't exist
    /// - `SplitError::CannotRemoveLastPanel` if this is the only panel
    pub fn remove_panel(&mut self, panel_id: PanelId) -> Result<Option<SessionId>, SplitError> {
        match &mut self.root {
            None => {
                // Single panel mode
                if self.single_panel.id == panel_id {
                    Err(SplitError::CannotRemoveLastPanel)
                } else {
                    Err(SplitError::PanelNotFound(panel_id))
                }
            }
            Some(node) => {
                let result = node.remove_panel(panel_id);

                match result {
                    RemoveResult::NotFound => Err(SplitError::PanelNotFound(panel_id)),
                    RemoveResult::RemovedSelf(_) => {
                        // The root node itself was the panel to remove
                        // This means we're down to a single panel
                        Err(SplitError::CannotRemoveLastPanel)
                    }
                    RemoveResult::Removed(session) => {
                        // Check if we're down to a single panel
                        if let Some(leaf) = node.as_leaf() {
                            // Collapse back to single panel mode
                            self.single_panel = LeafPanel {
                                id: leaf.id,
                                session: leaf.session,
                            };
                            self.root = None;

                            // Update focus if needed
                            if self.focused_panel == Some(panel_id) {
                                self.focused_panel = Some(self.single_panel.id);
                            }
                        } else {
                            // Update focus if the removed panel was focused
                            if self.focused_panel == Some(panel_id) {
                                // Focus the first available panel
                                self.focused_panel = Some(node.first_panel().id);
                            }
                        }

                        Ok(session)
                    }
                }
            }
        }
    }

    /// Returns the depth of the panel tree.
    ///
    /// A single panel has depth 0. Each level of splits adds 1.
    #[must_use]
    pub fn depth(&self) -> usize {
        match &self.root {
            None => 0,
            Some(node) => node.depth(),
        }
    }

    /// Returns a reference to the first panel in the layout.
    ///
    /// This is the leftmost/topmost panel in the tree.
    #[must_use]
    pub fn first_panel(&self) -> &LeafPanel {
        match &self.root {
            None => &self.single_panel,
            Some(node) => node.first_panel(),
        }
    }

    /// Returns a reference to the root panel node (if split).
    #[must_use]
    pub const fn root(&self) -> Option<&PanelNode> {
        self.root.as_ref()
    }

    // ========================================================================
    // Private Helper Methods
    // ========================================================================

    /// Finds a panel by ID and returns a mutable reference.
    fn find_panel_mut(&mut self, panel_id: PanelId) -> Result<&mut LeafPanel, SplitError> {
        match &mut self.root {
            None => {
                if self.single_panel.id == panel_id {
                    Ok(&mut self.single_panel)
                } else {
                    Err(SplitError::PanelNotFound(panel_id))
                }
            }
            Some(node) => node
                .find_panel_mut(panel_id)
                .ok_or(SplitError::PanelNotFound(panel_id)),
        }
    }
}

impl Default for SplitLayoutModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Construction Tests
    // ========================================================================

    #[test]
    fn new_creates_single_panel_layout() {
        let layout = SplitLayoutModel::new();
        assert!(!layout.is_split());
        assert_eq!(layout.panel_count(), 1);
        assert!(layout.get_focused_panel().is_some());
    }

    #[test]
    fn with_session_creates_layout_with_session() {
        let session = SessionId::new();
        let layout = SplitLayoutModel::with_session(session);

        assert!(!layout.is_split());
        let panel_id = layout.panel_ids()[0];
        assert_eq!(layout.get_panel_session(panel_id), Some(session));
    }

    #[test]
    fn default_creates_same_as_new() {
        let layout1 = SplitLayoutModel::new();
        let layout2 = SplitLayoutModel::default();

        assert_eq!(layout1.is_split(), layout2.is_split());
        assert_eq!(layout1.panel_count(), layout2.panel_count());
    }

    // ========================================================================
    // Split Tests
    // ========================================================================

    #[test]
    fn split_creates_two_panels() {
        let mut layout = SplitLayoutModel::new();
        let result = layout.split(SplitDirection::Vertical);

        assert!(result.is_ok());
        assert!(layout.is_split());
        assert_eq!(layout.panel_count(), 2);
    }

    #[test]
    fn split_preserves_original_session() {
        let session = SessionId::new();
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        layout.split(SplitDirection::Horizontal).unwrap();

        // Original session should be in first panel
        assert_eq!(layout.get_panel_session(original_panel_id), Some(session));
    }

    #[test]
    fn split_creates_empty_second_panel() {
        let session = SessionId::new();
        let mut layout = SplitLayoutModel::with_session(session);

        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        // New panel should be empty
        assert!(layout.get_panel_session(new_panel_id).is_none());
    }

    #[test]
    fn split_increases_panel_count_by_one() {
        let mut layout = SplitLayoutModel::new();
        let initial_count = layout.panel_count();

        layout.split(SplitDirection::Vertical).unwrap();

        assert_eq!(layout.panel_count(), initial_count + 1);
    }

    #[test]
    fn multiple_splits_increase_panel_count() {
        let mut layout = SplitLayoutModel::new();

        layout.split(SplitDirection::Vertical).unwrap();
        assert_eq!(layout.panel_count(), 2);

        // Focus the new panel and split it
        let panels = layout.panel_ids();
        layout.set_focus(panels[1]).unwrap();
        layout.split(SplitDirection::Horizontal).unwrap();
        assert_eq!(layout.panel_count(), 3);
    }

    #[test]
    fn split_without_focus_returns_error() {
        let mut layout = SplitLayoutModel::new();
        layout.focused_panel = None;

        let result = layout.split(SplitDirection::Vertical);
        assert!(matches!(result, Err(SplitError::NoFocusedPanel)));
    }

    // ========================================================================
    // Focus Tests
    // ========================================================================

    #[test]
    fn set_focus_updates_focused_panel() {
        let mut layout = SplitLayoutModel::new();
        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        layout.set_focus(new_panel_id).unwrap();

        assert_eq!(layout.get_focused_panel(), Some(new_panel_id));
    }

    #[test]
    fn set_focus_returns_error_for_unknown_panel() {
        let mut layout = SplitLayoutModel::new();
        let unknown_id = PanelId::new();

        let result = layout.set_focus(unknown_id);
        assert!(matches!(result, Err(SplitError::PanelNotFound(_))));
    }

    #[test]
    fn initial_focus_is_on_single_panel() {
        let layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        assert_eq!(layout.get_focused_panel(), Some(panel_id));
    }

    // ========================================================================
    // Place in Panel Tests
    // ========================================================================

    #[test]
    fn place_in_empty_panel_returns_placed() {
        let mut layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];
        let session = SessionId::new();

        let result = layout.place_in_panel(panel_id, session).unwrap();

        assert!(matches!(result, DropResult::Placed));
        assert_eq!(layout.get_panel_session(panel_id), Some(session));
    }

    #[test]
    fn place_in_occupied_panel_returns_evicted() {
        let old_session = SessionId::new();
        let mut layout = SplitLayoutModel::with_session(old_session);
        let panel_id = layout.panel_ids()[0];
        let new_session = SessionId::new();

        let result = layout.place_in_panel(panel_id, new_session).unwrap();

        assert!(matches!(
            result,
            DropResult::Evicted { evicted_session } if evicted_session == old_session
        ));
        assert_eq!(layout.get_panel_session(panel_id), Some(new_session));
    }

    #[test]
    fn place_in_unknown_panel_returns_error() {
        let mut layout = SplitLayoutModel::new();
        let unknown_id = PanelId::new();
        let session = SessionId::new();

        let result = layout.place_in_panel(unknown_id, session);
        assert!(matches!(result, Err(SplitError::PanelNotFound(_))));
    }

    #[test]
    fn place_does_not_affect_other_panels() {
        let session1 = SessionId::new();
        let mut layout = SplitLayoutModel::with_session(session1);
        let panel1_id = layout.panel_ids()[0];

        let panel2_id = layout.split(SplitDirection::Vertical).unwrap();
        let session2 = SessionId::new();

        layout.place_in_panel(panel2_id, session2).unwrap();

        // Panel 1 should still have its original session
        assert_eq!(layout.get_panel_session(panel1_id), Some(session1));
        assert_eq!(layout.get_panel_session(panel2_id), Some(session2));
    }

    // ========================================================================
    // Remove Panel Tests
    // ========================================================================

    #[test]
    fn remove_last_panel_returns_error() {
        let mut layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        let result = layout.remove_panel(panel_id);
        assert!(matches!(result, Err(SplitError::CannotRemoveLastPanel)));
    }

    #[test]
    fn remove_panel_decreases_count() {
        let mut layout = SplitLayoutModel::new();
        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        let initial_count = layout.panel_count();
        layout.remove_panel(new_panel_id).unwrap();

        assert_eq!(layout.panel_count(), initial_count - 1);
    }

    #[test]
    fn remove_panel_returns_session() {
        let session = SessionId::new();
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        layout.split(SplitDirection::Vertical).unwrap();
        let result = layout.remove_panel(original_panel_id).unwrap();

        assert_eq!(result, Some(session));
    }

    #[test]
    fn remove_empty_panel_returns_none() {
        let mut layout = SplitLayoutModel::new();
        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        let result = layout.remove_panel(new_panel_id).unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn remove_panel_collapses_to_single_panel() {
        let mut layout = SplitLayoutModel::new();
        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        layout.remove_panel(new_panel_id).unwrap();

        assert!(!layout.is_split());
        assert_eq!(layout.panel_count(), 1);
    }

    #[test]
    fn remove_unknown_panel_returns_error() {
        let mut layout = SplitLayoutModel::new();
        layout.split(SplitDirection::Vertical).unwrap();
        let unknown_id = PanelId::new();

        let result = layout.remove_panel(unknown_id);
        assert!(matches!(result, Err(SplitError::PanelNotFound(_))));
    }

    #[test]
    fn remove_focused_panel_updates_focus() {
        let mut layout = SplitLayoutModel::new();
        let original_panel_id = layout.panel_ids()[0];
        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        // Focus the new panel
        layout.set_focus(new_panel_id).unwrap();

        // Remove the focused panel
        layout.remove_panel(new_panel_id).unwrap();

        // Focus should move to remaining panel
        assert_eq!(layout.get_focused_panel(), Some(original_panel_id));
    }

    // ========================================================================
    // Color ID Tests
    // ========================================================================

    #[test]
    fn new_layout_has_no_color() {
        let layout = SplitLayoutModel::new();
        assert!(layout.color_id().is_none());
    }

    #[test]
    fn set_color_id_stores_color() {
        let mut layout = SplitLayoutModel::new();
        let color = ColorId::new(3);

        layout.set_color_id(color);

        assert_eq!(layout.color_id(), Some(color));
    }

    #[test]
    fn clear_color_id_removes_color() {
        let mut layout = SplitLayoutModel::new();
        layout.set_color_id(ColorId::new(5));

        layout.clear_color_id();

        assert!(layout.color_id().is_none());
    }

    // ========================================================================
    // Panel Query Tests
    // ========================================================================

    #[test]
    fn contains_panel_returns_true_for_existing_panel() {
        let layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        assert!(layout.contains_panel(panel_id));
    }

    #[test]
    fn contains_panel_returns_false_for_unknown_panel() {
        let layout = SplitLayoutModel::new();
        let unknown_id = PanelId::new();

        assert!(!layout.contains_panel(unknown_id));
    }

    #[test]
    fn panel_ids_returns_all_panels() {
        let mut layout = SplitLayoutModel::new();
        layout.split(SplitDirection::Vertical).unwrap();

        let panels = layout.panel_ids();
        assert_eq!(panels.len(), 2);
    }

    #[test]
    fn get_panel_session_returns_none_for_empty_panel() {
        let layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        assert!(layout.get_panel_session(panel_id).is_none());
    }

    #[test]
    fn get_panel_session_returns_none_for_unknown_panel() {
        let layout = SplitLayoutModel::new();
        let unknown_id = PanelId::new();

        assert!(layout.get_panel_session(unknown_id).is_none());
    }

    // ========================================================================
    // Depth Tests
    // ========================================================================

    #[test]
    fn single_panel_has_depth_zero() {
        let layout = SplitLayoutModel::new();
        assert_eq!(layout.depth(), 0);
    }

    #[test]
    fn single_split_has_depth_one() {
        let mut layout = SplitLayoutModel::new();
        layout.split(SplitDirection::Vertical).unwrap();

        assert_eq!(layout.depth(), 1);
    }

    #[test]
    fn nested_split_increases_depth() {
        let mut layout = SplitLayoutModel::new();
        let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();

        layout.set_focus(new_panel_id).unwrap();
        layout.split(SplitDirection::Horizontal).unwrap();

        assert_eq!(layout.depth(), 2);
    }

    // ========================================================================
    // First Panel Tests
    // ========================================================================

    #[test]
    fn first_panel_returns_single_panel() {
        let layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        assert_eq!(layout.first_panel().id, panel_id);
    }

    #[test]
    fn first_panel_returns_leftmost_after_split() {
        let mut layout = SplitLayoutModel::new();
        let original_panel_id = layout.panel_ids()[0];

        layout.split(SplitDirection::Vertical).unwrap();

        assert_eq!(layout.first_panel().id, original_panel_id);
    }
}
