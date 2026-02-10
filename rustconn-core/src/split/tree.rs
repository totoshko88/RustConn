//! Panel tree structure for split layouts
//!
//! This module provides the binary tree structure used to represent
//! split panel layouts. Each node is either a leaf panel (containing
//! an optional session) or a split node (containing two children).
//!
//! # Tree Structure
//!
//! ```text
//! Split(Vertical)
//! ├── Leaf(A, session_1)
//! └── Split(Horizontal)
//!     ├── Leaf(B, None)
//!     └── Leaf(C, None)
//! ```
//!
//! The tree supports arbitrary nesting depth and maintains proper
//! parent-child relationships for all operations.

use super::types::{PanelId, SessionId, SplitDirection};

/// Default split position (50% of available space).
pub const DEFAULT_SPLIT_POSITION: f64 = 0.5;

/// Minimum valid split position.
pub const MIN_SPLIT_POSITION: f64 = 0.0;

/// Maximum valid split position.
pub const MAX_SPLIT_POSITION: f64 = 1.0;

/// A node in the panel tree.
///
/// The panel tree is a binary tree where each node is either:
/// - A `Leaf` containing a panel that can display a session
/// - A `Split` containing two child nodes arranged in a direction
#[derive(Debug, Clone, PartialEq)]
pub enum PanelNode {
    /// A leaf panel that can contain a session.
    Leaf(LeafPanel),
    /// A split containing two child nodes.
    Split(SplitNode),
}

/// A leaf panel in the tree.
///
/// Leaf panels are the actual display areas that can contain sessions.
/// An empty panel has `session: None` and displays a placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafPanel {
    /// Unique identifier for this panel.
    pub id: PanelId,
    /// Session currently displayed (None = empty panel).
    pub session: Option<SessionId>,
}

/// A split node containing two children.
///
/// Split nodes divide the available space between two child nodes,
/// arranged either horizontally (top/bottom) or vertically (left/right).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitNode {
    /// Split direction.
    pub direction: SplitDirection,
    /// First child (start/top for horizontal, left for vertical).
    pub first: Box<PanelNode>,
    /// Second child (end/bottom for horizontal, right for vertical).
    pub second: Box<PanelNode>,
    /// Split position (0.0 to 1.0, default 0.5).
    ///
    /// Represents the proportion of space allocated to the first child.
    pub position: f64,
}

impl LeafPanel {
    /// Creates a new empty leaf panel with a unique ID.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: PanelId::new(),
            session: None,
        }
    }

    /// Creates a new leaf panel with the given ID.
    #[must_use]
    pub fn with_id(id: PanelId) -> Self {
        Self { id, session: None }
    }

    /// Creates a new leaf panel with a session.
    #[must_use]
    pub fn with_session(session: SessionId) -> Self {
        Self {
            id: PanelId::new(),
            session: Some(session),
        }
    }

    /// Returns true if this panel has no session (is empty).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.session.is_none()
    }

    /// Returns true if this panel has a session (is occupied).
    #[must_use]
    pub const fn is_occupied(&self) -> bool {
        self.session.is_some()
    }
}

impl Default for LeafPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SplitNode {
    /// Creates a new split node with the given direction and children.
    ///
    /// Uses the default split position (0.5).
    #[must_use]
    pub fn new(direction: SplitDirection, first: PanelNode, second: PanelNode) -> Self {
        Self {
            direction,
            first: Box::new(first),
            second: Box::new(second),
            position: DEFAULT_SPLIT_POSITION,
        }
    }

    /// Creates a new split node with a custom position.
    ///
    /// # Arguments
    ///
    /// * `direction` - The split direction
    /// * `first` - The first child node
    /// * `second` - The second child node
    /// * `position` - Split position (0.0 to 1.0)
    ///
    /// # Panics
    ///
    /// Panics if position is not in the range [0.0, 1.0].
    #[must_use]
    pub fn with_position(
        direction: SplitDirection,
        first: PanelNode,
        second: PanelNode,
        position: f64,
    ) -> Self {
        assert!(
            (MIN_SPLIT_POSITION..=MAX_SPLIT_POSITION).contains(&position),
            "Split position must be between {MIN_SPLIT_POSITION} and {MAX_SPLIT_POSITION}"
        );
        Self {
            direction,
            first: Box::new(first),
            second: Box::new(second),
            position,
        }
    }
}

impl PanelNode {
    /// Creates a new leaf node with an empty panel.
    #[must_use]
    pub fn new_leaf() -> Self {
        Self::Leaf(LeafPanel::new())
    }

    /// Creates a new leaf node with the given panel.
    #[must_use]
    pub fn leaf(panel: LeafPanel) -> Self {
        Self::Leaf(panel)
    }

    /// Creates a new split node.
    #[must_use]
    pub fn split(direction: SplitDirection, first: Self, second: Self) -> Self {
        Self::Split(SplitNode::new(direction, first, second))
    }

    /// Returns true if this is a leaf node.
    #[must_use]
    pub const fn is_leaf(&self) -> bool {
        matches!(self, Self::Leaf(_))
    }

    /// Returns true if this is a split node.
    #[must_use]
    pub const fn is_split(&self) -> bool {
        matches!(self, Self::Split(_))
    }

    /// Returns the leaf panel if this is a leaf node.
    #[must_use]
    pub const fn as_leaf(&self) -> Option<&LeafPanel> {
        match self {
            Self::Leaf(panel) => Some(panel),
            Self::Split(_) => None,
        }
    }

    /// Returns a mutable reference to the leaf panel if this is a leaf node.
    #[must_use]
    pub fn as_leaf_mut(&mut self) -> Option<&mut LeafPanel> {
        match self {
            Self::Leaf(panel) => Some(panel),
            Self::Split(_) => None,
        }
    }

    /// Returns the split node if this is a split node.
    #[must_use]
    pub const fn as_split(&self) -> Option<&SplitNode> {
        match self {
            Self::Leaf(_) => None,
            Self::Split(split) => Some(split),
        }
    }

    /// Returns a mutable reference to the split node if this is a split node.
    #[must_use]
    pub fn as_split_mut(&mut self) -> Option<&mut SplitNode> {
        match self {
            Self::Leaf(_) => None,
            Self::Split(split) => Some(split),
        }
    }

    // ========================================================================
    // Tree Traversal Methods
    // ========================================================================

    /// Finds a panel by its ID.
    ///
    /// Returns a reference to the leaf panel if found.
    #[must_use]
    pub fn find_panel(&self, panel_id: PanelId) -> Option<&LeafPanel> {
        match self {
            Self::Leaf(panel) => {
                if panel.id == panel_id {
                    Some(panel)
                } else {
                    None
                }
            }
            Self::Split(split) => split
                .first
                .find_panel(panel_id)
                .or_else(|| split.second.find_panel(panel_id)),
        }
    }

    /// Finds a panel by its ID and returns a mutable reference.
    #[must_use]
    pub fn find_panel_mut(&mut self, panel_id: PanelId) -> Option<&mut LeafPanel> {
        match self {
            Self::Leaf(panel) => {
                if panel.id == panel_id {
                    Some(panel)
                } else {
                    None
                }
            }
            Self::Split(split) => {
                if let Some(panel) = split.first.find_panel_mut(panel_id) {
                    Some(panel)
                } else {
                    split.second.find_panel_mut(panel_id)
                }
            }
        }
    }

    /// Returns all panel IDs in the tree.
    ///
    /// Traverses the tree in pre-order (depth-first, left-to-right).
    #[must_use]
    pub fn panel_ids(&self) -> Vec<PanelId> {
        let mut ids = Vec::new();
        self.collect_panel_ids(&mut ids);
        ids
    }

    /// Helper method to collect panel IDs recursively.
    fn collect_panel_ids(&self, ids: &mut Vec<PanelId>) {
        match self {
            Self::Leaf(panel) => ids.push(panel.id),
            Self::Split(split) => {
                split.first.collect_panel_ids(ids);
                split.second.collect_panel_ids(ids);
            }
        }
    }

    /// Returns the depth of the tree.
    ///
    /// A single leaf has depth 0. Each level of splits adds 1 to the depth.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Leaf(_) => 0,
            Self::Split(split) => 1 + split.first.depth().max(split.second.depth()),
        }
    }

    /// Returns the total number of leaf panels in the tree.
    #[must_use]
    pub fn panel_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split(split) => split.first.panel_count() + split.second.panel_count(),
        }
    }

    /// Returns true if the tree contains a panel with the given ID.
    #[must_use]
    pub fn contains_panel(&self, panel_id: PanelId) -> bool {
        self.find_panel(panel_id).is_some()
    }

    /// Returns the first leaf panel in the tree (leftmost/topmost).
    #[must_use]
    pub fn first_panel(&self) -> &LeafPanel {
        match self {
            Self::Leaf(panel) => panel,
            Self::Split(split) => split.first.first_panel(),
        }
    }

    /// Returns a mutable reference to the first leaf panel.
    #[must_use]
    pub fn first_panel_mut(&mut self) -> &mut LeafPanel {
        match self {
            Self::Leaf(panel) => panel,
            Self::Split(split) => split.first.first_panel_mut(),
        }
    }

    // ========================================================================
    // Tree Mutation Methods
    // ========================================================================

    /// Splits a panel in the given direction.
    ///
    /// The panel with `panel_id` is replaced by a split node containing:
    /// - First child: the original panel (with its session)
    /// - Second child: a new empty panel
    ///
    /// Returns the ID of the newly created panel, or `None` if the panel
    /// was not found.
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The ID of the panel to split
    /// * `direction` - The direction to split (Horizontal or Vertical)
    #[must_use]
    pub fn insert_split(
        &mut self,
        panel_id: PanelId,
        direction: SplitDirection,
    ) -> Option<PanelId> {
        self.insert_split_internal(panel_id, direction)
    }

    /// Internal implementation of insert_split that handles the recursive search.
    fn insert_split_internal(
        &mut self,
        panel_id: PanelId,
        direction: SplitDirection,
    ) -> Option<PanelId> {
        match self {
            Self::Leaf(panel) => {
                if panel.id == panel_id {
                    // Found the panel to split
                    let new_panel = LeafPanel::new();
                    let new_panel_id = new_panel.id;

                    // Create the original panel as first child
                    let original = LeafPanel {
                        id: panel.id,
                        session: panel.session,
                    };

                    // Replace self with a split node
                    *self = Self::Split(SplitNode::new(
                        direction,
                        Self::Leaf(original),
                        Self::Leaf(new_panel),
                    ));

                    Some(new_panel_id)
                } else {
                    None
                }
            }
            Self::Split(split) => {
                // Try first child, then second
                split
                    .first
                    .insert_split_internal(panel_id, direction)
                    .or_else(|| split.second.insert_split_internal(panel_id, direction))
            }
        }
    }

    /// Updates the position of the split node whose first child contains
    /// the given panel ID.
    ///
    /// This is used to persist user-dragged divider positions back to the
    /// model. The `first_panel_id` identifies the split by matching the
    /// leftmost/topmost panel in its first child subtree.
    ///
    /// Returns `true` if the split was found and updated.
    pub fn update_split_position(&mut self, first_panel_id: PanelId, position: f64) -> bool {
        let clamped = position.clamp(MIN_SPLIT_POSITION, MAX_SPLIT_POSITION);
        match self {
            Self::Leaf(_) => false,
            Self::Split(split) => {
                if split.first.first_panel().id == first_panel_id {
                    split.position = clamped;
                    true
                } else {
                    split.first.update_split_position(first_panel_id, clamped)
                        || split.second.update_split_position(first_panel_id, clamped)
                }
            }
        }
    }

    /// Removes a panel from the tree.
    ///
    /// When a panel is removed:
    /// - If this is a leaf node with the matching ID, returns `RemoveResult::RemovedSelf`
    ///   indicating the caller should handle the removal
    /// - If the panel is in a child split, the sibling is promoted to replace the split
    ///
    /// Returns the session that was in the removed panel (if any), or `None` if
    /// the panel was not found.
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The ID of the panel to remove
    pub fn remove_panel(&mut self, panel_id: PanelId) -> RemoveResult {
        match self {
            Self::Leaf(panel) => {
                if panel.id == panel_id {
                    RemoveResult::RemovedSelf(panel.session)
                } else {
                    RemoveResult::NotFound
                }
            }
            Self::Split(split) => {
                // Check if the panel to remove is a direct child
                if let Self::Leaf(first_panel) = split.first.as_ref() {
                    if first_panel.id == panel_id {
                        // Remove first child, promote second
                        let session = first_panel.session;
                        let second =
                            std::mem::replace(split.second.as_mut(), Self::Leaf(LeafPanel::new()));
                        *self = second;
                        return RemoveResult::Removed(session);
                    }
                }

                if let Self::Leaf(second_panel) = split.second.as_ref() {
                    if second_panel.id == panel_id {
                        // Remove second child, promote first
                        let session = second_panel.session;
                        let first =
                            std::mem::replace(split.first.as_mut(), Self::Leaf(LeafPanel::new()));
                        *self = first;
                        return RemoveResult::Removed(session);
                    }
                }

                // Recursively search in children
                match split.first.remove_panel(panel_id) {
                    RemoveResult::NotFound => {}
                    result => return result,
                }

                split.second.remove_panel(panel_id)
            }
        }
    }
}

/// Result of a panel removal operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveResult {
    /// The panel was not found in the tree.
    NotFound,
    /// The panel was removed and the tree was restructured.
    /// Contains the session that was in the panel (if any).
    Removed(Option<SessionId>),
    /// The panel was the root leaf itself and needs to be handled by the caller.
    /// Contains the session that was in the panel (if any).
    RemovedSelf(Option<SessionId>),
}

impl RemoveResult {
    /// Returns true if the panel was found and removed.
    #[must_use]
    pub const fn is_removed(&self) -> bool {
        matches!(self, Self::Removed(_) | Self::RemovedSelf(_))
    }

    /// Returns the session that was in the removed panel, if any.
    #[must_use]
    pub const fn session(&self) -> Option<SessionId> {
        match self {
            Self::NotFound => None,
            Self::Removed(session) | Self::RemovedSelf(session) => *session,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // LeafPanel Tests
    // ========================================================================

    #[test]
    fn leaf_panel_new_creates_empty_panel() {
        let panel = LeafPanel::new();
        assert!(panel.is_empty());
        assert!(!panel.is_occupied());
        assert!(panel.session.is_none());
    }

    #[test]
    fn leaf_panel_with_id_creates_panel_with_specific_id() {
        let id = PanelId::new();
        let panel = LeafPanel::with_id(id);
        assert_eq!(panel.id, id);
        assert!(panel.is_empty());
    }

    #[test]
    fn leaf_panel_with_session_creates_occupied_panel() {
        let session = SessionId::new();
        let panel = LeafPanel::with_session(session);
        assert!(panel.is_occupied());
        assert!(!panel.is_empty());
        assert_eq!(panel.session, Some(session));
    }

    // ========================================================================
    // SplitNode Tests
    // ========================================================================

    #[test]
    fn split_node_new_uses_default_position() {
        let split = SplitNode::new(
            SplitDirection::Vertical,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
        );
        assert!((split.position - DEFAULT_SPLIT_POSITION).abs() < f64::EPSILON);
    }

    #[test]
    fn split_node_with_position_sets_custom_position() {
        let split = SplitNode::with_position(
            SplitDirection::Horizontal,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
            0.3,
        );
        assert!((split.position - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "Split position must be between")]
    fn split_node_with_position_rejects_negative() {
        let _ = SplitNode::with_position(
            SplitDirection::Vertical,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
            -0.1,
        );
    }

    #[test]
    #[should_panic(expected = "Split position must be between")]
    fn split_node_with_position_rejects_greater_than_one() {
        let _ = SplitNode::with_position(
            SplitDirection::Vertical,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
            1.1,
        );
    }

    // ========================================================================
    // PanelNode Basic Tests
    // ========================================================================

    #[test]
    fn panel_node_new_leaf_creates_leaf() {
        let node = PanelNode::new_leaf();
        assert!(node.is_leaf());
        assert!(!node.is_split());
    }

    #[test]
    fn panel_node_split_creates_split() {
        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
        );
        assert!(node.is_split());
        assert!(!node.is_leaf());
    }

    #[test]
    fn panel_node_as_leaf_returns_some_for_leaf() {
        let node = PanelNode::new_leaf();
        assert!(node.as_leaf().is_some());
        assert!(node.as_split().is_none());
    }

    #[test]
    fn panel_node_as_split_returns_some_for_split() {
        let node = PanelNode::split(
            SplitDirection::Horizontal,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
        );
        assert!(node.as_split().is_some());
        assert!(node.as_leaf().is_none());
    }

    // ========================================================================
    // Tree Traversal Tests
    // ========================================================================

    #[test]
    fn find_panel_finds_leaf_in_single_node() {
        let panel = LeafPanel::new();
        let id = panel.id;
        let node = PanelNode::Leaf(panel);

        assert!(node.find_panel(id).is_some());
        assert_eq!(node.find_panel(id).unwrap().id, id);
    }

    #[test]
    fn find_panel_returns_none_for_unknown_id() {
        let node = PanelNode::new_leaf();
        let unknown_id = PanelId::new();
        assert!(node.find_panel(unknown_id).is_none());
    }

    #[test]
    fn find_panel_finds_panel_in_nested_tree() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let panel3 = LeafPanel::new();
        let id2 = panel2.id;

        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::Leaf(panel2),
                PanelNode::Leaf(panel3),
            ),
        );

        assert!(node.find_panel(id2).is_some());
        assert_eq!(node.find_panel(id2).unwrap().id, id2);
    }

    #[test]
    fn panel_ids_returns_single_id_for_leaf() {
        let panel = LeafPanel::new();
        let id = panel.id;
        let node = PanelNode::Leaf(panel);

        let ids = node.panel_ids();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], id);
    }

    #[test]
    fn panel_ids_returns_all_ids_in_tree() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let panel3 = LeafPanel::new();
        let id1 = panel1.id;
        let id2 = panel2.id;
        let id3 = panel3.id;

        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::Leaf(panel2),
                PanelNode::Leaf(panel3),
            ),
        );

        let ids = node.panel_ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
    }

    #[test]
    fn depth_is_zero_for_leaf() {
        let node = PanelNode::new_leaf();
        assert_eq!(node.depth(), 0);
    }

    #[test]
    fn depth_is_one_for_single_split() {
        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
        );
        assert_eq!(node.depth(), 1);
    }

    #[test]
    fn depth_reflects_maximum_nesting() {
        // Create a tree with depth 3 on one side, depth 1 on the other
        let deep_side = PanelNode::split(
            SplitDirection::Horizontal,
            PanelNode::split(
                SplitDirection::Vertical,
                PanelNode::new_leaf(),
                PanelNode::new_leaf(),
            ),
            PanelNode::new_leaf(),
        );

        let node = PanelNode::split(SplitDirection::Vertical, deep_side, PanelNode::new_leaf());

        assert_eq!(node.depth(), 3);
    }

    #[test]
    fn panel_count_is_one_for_leaf() {
        let node = PanelNode::new_leaf();
        assert_eq!(node.panel_count(), 1);
    }

    #[test]
    fn panel_count_is_two_for_single_split() {
        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::new_leaf(),
            PanelNode::new_leaf(),
        );
        assert_eq!(node.panel_count(), 2);
    }

    #[test]
    fn panel_count_equals_splits_plus_one() {
        // 3 splits should give 4 panels
        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::new_leaf(),
                PanelNode::new_leaf(),
            ),
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::new_leaf(),
                PanelNode::new_leaf(),
            ),
        );
        assert_eq!(node.panel_count(), 4);
    }

    #[test]
    fn contains_panel_returns_true_for_existing_panel() {
        let panel = LeafPanel::new();
        let id = panel.id;
        let node = PanelNode::Leaf(panel);
        assert!(node.contains_panel(id));
    }

    #[test]
    fn contains_panel_returns_false_for_unknown_panel() {
        let node = PanelNode::new_leaf();
        assert!(!node.contains_panel(PanelId::new()));
    }

    #[test]
    fn first_panel_returns_leftmost_panel() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let id1 = panel1.id;

        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::Leaf(panel2),
        );

        assert_eq!(node.first_panel().id, id1);
    }

    #[test]
    fn first_panel_traverses_nested_splits() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let panel3 = LeafPanel::new();
        let id1 = panel1.id;

        let node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::Leaf(panel1),
                PanelNode::Leaf(panel2),
            ),
            PanelNode::Leaf(panel3),
        );

        assert_eq!(node.first_panel().id, id1);
    }

    // ========================================================================
    // Tree Mutation Tests
    // ========================================================================

    #[test]
    fn insert_split_creates_split_from_leaf() {
        let panel = LeafPanel::new();
        let original_id = panel.id;
        let mut node = PanelNode::Leaf(panel);

        let new_id = node.insert_split(original_id, SplitDirection::Vertical);

        assert!(new_id.is_some());
        assert!(node.is_split());
        assert_eq!(node.panel_count(), 2);

        // Original panel should be first child
        let split = node.as_split().unwrap();
        assert_eq!(split.first.first_panel().id, original_id);
    }

    #[test]
    fn insert_split_preserves_session() {
        let session = SessionId::new();
        let panel = LeafPanel::with_session(session);
        let original_id = panel.id;
        let mut node = PanelNode::Leaf(panel);

        let _ = node.insert_split(original_id, SplitDirection::Horizontal);

        // Session should be in first child
        let first_panel = node.first_panel();
        assert_eq!(first_panel.session, Some(session));
    }

    #[test]
    fn insert_split_creates_empty_second_panel() {
        let panel = LeafPanel::new();
        let original_id = panel.id;
        let mut node = PanelNode::Leaf(panel);

        let new_id = node
            .insert_split(original_id, SplitDirection::Vertical)
            .unwrap();

        // New panel should be empty
        let new_panel = node.find_panel(new_id).unwrap();
        assert!(new_panel.is_empty());
    }

    #[test]
    fn insert_split_returns_none_for_unknown_panel() {
        let mut node = PanelNode::new_leaf();
        let unknown_id = PanelId::new();

        let result = node.insert_split(unknown_id, SplitDirection::Vertical);
        assert!(result.is_none());
    }

    #[test]
    fn insert_split_works_on_nested_panel() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let id2 = panel2.id;

        let mut node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::Leaf(panel2),
        );

        let initial_count = node.panel_count();
        let new_id = node.insert_split(id2, SplitDirection::Horizontal);

        assert!(new_id.is_some());
        assert_eq!(node.panel_count(), initial_count + 1);
    }

    #[test]
    fn insert_split_sets_correct_direction() {
        let panel = LeafPanel::new();
        let id = panel.id;
        let mut node = PanelNode::Leaf(panel);

        let _ = node.insert_split(id, SplitDirection::Horizontal);

        let split = node.as_split().unwrap();
        assert_eq!(split.direction, SplitDirection::Horizontal);
    }

    // ========================================================================
    // Remove Panel Tests
    // ========================================================================

    #[test]
    fn remove_panel_returns_removed_self_for_root_leaf() {
        let session = SessionId::new();
        let panel = LeafPanel::with_session(session);
        let id = panel.id;
        let mut node = PanelNode::Leaf(panel);

        let result = node.remove_panel(id);
        assert!(matches!(result, RemoveResult::RemovedSelf(Some(s)) if s == session));
    }

    #[test]
    fn remove_panel_returns_not_found_for_unknown_id() {
        let mut node = PanelNode::new_leaf();
        let unknown_id = PanelId::new();

        let result = node.remove_panel(unknown_id);
        assert!(matches!(result, RemoveResult::NotFound));
    }

    #[test]
    fn remove_panel_promotes_sibling_when_first_child_removed() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let id1 = panel1.id;
        let id2 = panel2.id;

        let mut node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::Leaf(panel2),
        );

        let result = node.remove_panel(id1);

        assert!(result.is_removed());
        assert!(node.is_leaf());
        assert_eq!(node.as_leaf().unwrap().id, id2);
    }

    #[test]
    fn remove_panel_promotes_sibling_when_second_child_removed() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let id1 = panel1.id;
        let id2 = panel2.id;

        let mut node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::Leaf(panel2),
        );

        let result = node.remove_panel(id2);

        assert!(result.is_removed());
        assert!(node.is_leaf());
        assert_eq!(node.as_leaf().unwrap().id, id1);
    }

    #[test]
    fn remove_panel_returns_session_from_removed_panel() {
        let session = SessionId::new();
        let panel1 = LeafPanel::with_session(session);
        let panel2 = LeafPanel::new();
        let id1 = panel1.id;

        let mut node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::Leaf(panel2),
        );

        let result = node.remove_panel(id1);
        assert_eq!(result.session(), Some(session));
    }

    #[test]
    fn remove_panel_works_on_nested_tree() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let panel3 = LeafPanel::new();
        let id2 = panel2.id;

        let mut node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::Leaf(panel2),
                PanelNode::Leaf(panel3),
            ),
        );

        let initial_count = node.panel_count();
        let result = node.remove_panel(id2);

        assert!(result.is_removed());
        assert_eq!(node.panel_count(), initial_count - 1);
    }

    #[test]
    fn remove_panel_collapses_nested_split() {
        let panel1 = LeafPanel::new();
        let panel2 = LeafPanel::new();
        let panel3 = LeafPanel::new();
        let id1 = panel1.id;
        let id2 = panel2.id;
        let id3 = panel3.id;

        let mut node = PanelNode::split(
            SplitDirection::Vertical,
            PanelNode::Leaf(panel1),
            PanelNode::split(
                SplitDirection::Horizontal,
                PanelNode::Leaf(panel2),
                PanelNode::Leaf(panel3),
            ),
        );

        // Remove panel2, panel3 should be promoted
        node.remove_panel(id2);

        // Tree should now be: Split(Vertical, panel1, panel3)
        assert!(node.is_split());
        assert_eq!(node.panel_count(), 2);
        assert!(node.contains_panel(id1));
        assert!(node.contains_panel(id3));
    }

    // ========================================================================
    // RemoveResult Tests
    // ========================================================================

    #[test]
    fn remove_result_is_removed_returns_true_for_removed() {
        let result = RemoveResult::Removed(None);
        assert!(result.is_removed());
    }

    #[test]
    fn remove_result_is_removed_returns_true_for_removed_self() {
        let result = RemoveResult::RemovedSelf(None);
        assert!(result.is_removed());
    }

    #[test]
    fn remove_result_is_removed_returns_false_for_not_found() {
        let result = RemoveResult::NotFound;
        assert!(!result.is_removed());
    }

    #[test]
    fn remove_result_session_returns_session_when_present() {
        let session = SessionId::new();
        let result = RemoveResult::Removed(Some(session));
        assert_eq!(result.session(), Some(session));
    }

    #[test]
    fn remove_result_session_returns_none_for_not_found() {
        let result = RemoveResult::NotFound;
        assert!(result.session().is_none());
    }
}
