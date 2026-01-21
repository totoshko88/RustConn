//! Tab split manager for managing split layouts across tabs
//!
//! This module provides `TabSplitManager` which manages the relationship between
//! tabs and their split layouts. Each tab can have its own independent split
//! configuration, and the manager handles creation, retrieval, and cleanup.
//!
//! # Architecture
//!
//! The `TabSplitManager` maintains:
//! - A map of `TabId` to `SplitViewAdapter` for each tab's layout
//! - A shared `ColorPool` for allocating unique colors to split containers
//!
//! # Requirements
//! - 3.1: Each Root_Tab maintains its own Split_Container
//! - 3.3: Split_Container is created when first split operation occurs
//! - 3.4: Split_Container is destroyed when tab is closed

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rustconn_core::split::{ColorId, ColorPool, SessionId, TabId};

use super::adapter::SplitViewAdapter;

/// Manages split layouts for all tabs.
///
/// This struct maintains the relationship between tabs and their split layouts,
/// ensuring each tab has its own independent split configuration while sharing
/// a global color pool for visual identification.
///
/// # Requirements
/// - 3.1: Each Root_Tab maintains its own Split_Container
/// - 3.3: Split_Container is created when first split operation occurs
/// - 3.4: Split_Container is destroyed when tab is closed
///
/// # Example
///
/// ```ignore
/// use rustconn::split_view::TabSplitManager;
/// use rustconn_core::split::{TabId, SplitDirection};
///
/// let mut manager = TabSplitManager::new();
///
/// // Get or create a layout for a tab
/// let tab_id = TabId::new();
/// let adapter = manager.get_or_create(tab_id);
///
/// // Split the tab's layout
/// adapter.split(SplitDirection::Vertical).unwrap();
///
/// // Get the tab's color (if it's a split container)
/// if let Some(color) = manager.get_tab_color(tab_id) {
///     println!("Tab has color: {}", color.index());
/// }
///
/// // Clean up when tab is closed
/// manager.remove(tab_id);
/// ```
#[derive(Debug)]
pub struct TabSplitManager {
    /// Map of tab IDs to their split adapters
    layouts: HashMap<TabId, SplitViewAdapter>,
    /// Global color pool shared across all tabs
    color_pool: Rc<RefCell<ColorPool>>,
}

impl TabSplitManager {
    /// Creates a new tab split manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            layouts: HashMap::new(),
            color_pool: Rc::new(RefCell::new(ColorPool::new())),
        }
    }

    /// Gets or creates a split adapter for a tab.
    ///
    /// If the tab doesn't have a layout yet, a new one is created with an
    /// empty single panel. The layout is stored and returned for further
    /// operations.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to get or create a layout for
    ///
    /// # Returns
    ///
    /// A mutable reference to the tab's `SplitViewAdapter`
    ///
    /// # Requirements
    /// - 3.3: Split_Container is created when first split operation occurs
    pub fn get_or_create(&mut self, tab_id: TabId) -> &mut SplitViewAdapter {
        self.layouts.entry(tab_id).or_default()
    }

    /// Gets or creates a split adapter for a tab with an initial session.
    ///
    /// Similar to `get_or_create`, but initializes the layout with a session
    /// in the first panel if the layout is newly created.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab
    /// * `session_id` - The session to place in the initial panel
    ///
    /// # Returns
    ///
    /// A mutable reference to the tab's `SplitViewAdapter`
    pub fn get_or_create_with_session(
        &mut self,
        tab_id: TabId,
        session_id: SessionId,
    ) -> &mut SplitViewAdapter {
        self.layouts
            .entry(tab_id)
            .or_insert_with(|| SplitViewAdapter::with_session(session_id))
    }

    /// Gets an existing split adapter for a tab.
    ///
    /// Returns `None` if the tab doesn't have a layout.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to get the layout for
    ///
    /// # Returns
    ///
    /// An optional mutable reference to the tab's `SplitViewAdapter`
    #[must_use]
    pub fn get(&self, tab_id: TabId) -> Option<&SplitViewAdapter> {
        self.layouts.get(&tab_id)
    }

    /// Gets a mutable reference to an existing split adapter for a tab.
    ///
    /// Returns `None` if the tab doesn't have a layout.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to get the layout for
    ///
    /// # Returns
    ///
    /// An optional mutable reference to the tab's `SplitViewAdapter`
    pub fn get_mut(&mut self, tab_id: TabId) -> Option<&mut SplitViewAdapter> {
        self.layouts.get_mut(&tab_id)
    }

    /// Removes a tab's layout.
    ///
    /// This should be called when a tab is closed to clean up resources
    /// and release the color back to the pool.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to remove
    ///
    /// # Returns
    ///
    /// The removed `SplitViewAdapter` if it existed
    ///
    /// # Requirements
    /// - 3.4: Split_Container is destroyed when tab is closed
    pub fn remove(&mut self, tab_id: TabId) -> Option<SplitViewAdapter> {
        let adapter = self.layouts.remove(&tab_id);

        // Release the color back to the pool if the adapter had one
        if let Some(ref adapter) = adapter {
            if let Some(color_id) = adapter.model().borrow().color_id() {
                self.color_pool.borrow_mut().release(color_id);
            }
        }

        adapter
    }

    /// Returns the color for a tab's split container.
    ///
    /// Returns `None` if the tab doesn't have a layout or if the layout
    /// is not a split container (single panel).
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to get the color for
    ///
    /// # Returns
    ///
    /// The `ColorId` if the tab has a split container, `None` otherwise
    ///
    /// # Requirements
    /// - 6.2: Tab header shows color indicator when tab contains Split_Container
    #[must_use]
    pub fn get_tab_color(&self, tab_id: TabId) -> Option<ColorId> {
        self.layouts
            .get(&tab_id)
            .and_then(|adapter| adapter.model().borrow().color_id())
    }

    /// Allocates a color for a tab's split container.
    ///
    /// This should be called when a tab becomes a split container (first split).
    /// The color is allocated from the shared pool and assigned to the tab's
    /// layout model.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to allocate a color for
    ///
    /// # Returns
    ///
    /// The allocated `ColorId`, or `None` if the tab doesn't have a layout
    ///
    /// # Requirements
    /// - 2.5: Split_Container is assigned a unique Color_ID from ColorPool
    pub fn allocate_color(&mut self, tab_id: TabId) -> Option<ColorId> {
        if let Some(adapter) = self.layouts.get(&tab_id) {
            // Only allocate if the adapter doesn't already have a color
            if adapter.model().borrow().color_id().is_none() {
                let color_id = self.color_pool.borrow_mut().allocate();
                adapter.model().borrow_mut().set_color_id(color_id);
                return Some(color_id);
            }
            // Return existing color
            return adapter.model().borrow().color_id();
        }
        None
    }

    /// Returns true if the tab has a split layout (more than one panel).
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to check
    ///
    /// # Returns
    ///
    /// `true` if the tab has splits, `false` otherwise
    #[must_use]
    pub fn is_split(&self, tab_id: TabId) -> bool {
        self.layouts
            .get(&tab_id)
            .is_some_and(SplitViewAdapter::is_split)
    }

    /// Returns the number of panels in a tab's layout.
    ///
    /// # Arguments
    ///
    /// * `tab_id` - The ID of the tab to check
    ///
    /// # Returns
    ///
    /// The number of panels, or 0 if the tab doesn't have a layout
    #[must_use]
    pub fn panel_count(&self, tab_id: TabId) -> usize {
        self.layouts
            .get(&tab_id)
            .map_or(0, SplitViewAdapter::panel_count)
    }

    /// Returns all tab IDs that have layouts.
    #[must_use]
    pub fn tab_ids(&self) -> Vec<TabId> {
        self.layouts.keys().copied().collect()
    }

    /// Returns the number of tabs with layouts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    /// Returns true if there are no tabs with layouts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    /// Returns a reference to the shared color pool.
    ///
    /// This is useful for external components that need to query color
    /// availability or statistics.
    #[must_use]
    pub fn color_pool(&self) -> Rc<RefCell<ColorPool>> {
        Rc::clone(&self.color_pool)
    }
}

impl Default for TabSplitManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustconn_core::split::SplitDirection;

    // Note: Most tests for TabSplitManager require GTK initialization because
    // SplitViewAdapter creates GTK widgets. These tests are marked with #[ignore]
    // and should be run manually with GTK available, or tested through integration
    // tests.
    //
    // The core logic is tested through rustconn-core's property tests which don't
    // require GTK.

    #[test]
    fn new_manager_is_empty() {
        let manager = TabSplitManager::new();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let removed = manager.remove(tab_id);
        assert!(removed.is_none());
    }

    // The following tests require GTK initialization and are ignored by default.
    // Run with: cargo test -p rustconn manager -- --ignored

    #[test]
    #[ignore = "requires GTK initialization"]
    fn get_or_create_creates_new_layout() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let adapter = manager.get_or_create(tab_id);
        assert_eq!(adapter.panel_count(), 1);
        assert!(!adapter.is_split());

        assert_eq!(manager.len(), 1);
        assert!(!manager.is_empty());
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn get_or_create_returns_existing_layout() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        // Create initial layout
        let _ = manager.get_or_create(tab_id);

        // Split the layout
        {
            let adapter = manager.get_mut(tab_id).unwrap();
            adapter.split(SplitDirection::Vertical).unwrap();
        }

        // Get again - should return the same layout with 2 panels
        let adapter = manager.get_or_create(tab_id);
        assert_eq!(adapter.panel_count(), 2);
        assert!(adapter.is_split());
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn get_or_create_with_session_initializes_session() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();
        let session_id = SessionId::new();

        let adapter = manager.get_or_create_with_session(tab_id, session_id);
        let panel_ids = adapter.panel_ids();
        assert_eq!(panel_ids.len(), 1);

        let panel_session = adapter.get_panel_session(panel_ids[0]);
        assert_eq!(panel_session, Some(session_id));
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn remove_cleans_up_layout() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let _ = manager.get_or_create(tab_id);
        assert_eq!(manager.len(), 1);

        let removed = manager.remove(tab_id);
        assert!(removed.is_some());
        assert!(manager.is_empty());
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn get_tab_color_returns_none_for_non_split() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let _ = manager.get_or_create(tab_id);
        assert!(manager.get_tab_color(tab_id).is_none());
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn allocate_color_assigns_color() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let _ = manager.get_or_create(tab_id);
        let color = manager.allocate_color(tab_id);
        assert!(color.is_some());

        // Should return the same color on subsequent calls
        let color2 = manager.allocate_color(tab_id);
        assert_eq!(color, color2);
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn is_split_returns_correct_state() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let _ = manager.get_or_create(tab_id);
        assert!(!manager.is_split(tab_id));

        manager
            .get_mut(tab_id)
            .unwrap()
            .split(SplitDirection::Vertical)
            .unwrap();
        assert!(manager.is_split(tab_id));
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn panel_count_returns_correct_count() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        assert_eq!(manager.panel_count(tab_id), 0); // No layout yet

        let _ = manager.get_or_create(tab_id);
        assert_eq!(manager.panel_count(tab_id), 1);

        manager
            .get_mut(tab_id)
            .unwrap()
            .split(SplitDirection::Vertical)
            .unwrap();
        assert_eq!(manager.panel_count(tab_id), 2);
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn tab_ids_returns_all_tabs() {
        let mut manager = TabSplitManager::new();
        let tab1 = TabId::new();
        let tab2 = TabId::new();

        let _ = manager.get_or_create(tab1);
        let _ = manager.get_or_create(tab2);

        let ids = manager.tab_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&tab1));
        assert!(ids.contains(&tab2));
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn multiple_tabs_have_independent_layouts() {
        let mut manager = TabSplitManager::new();
        let tab1 = TabId::new();
        let tab2 = TabId::new();

        // Create layouts for both tabs
        let _ = manager.get_or_create(tab1);
        let _ = manager.get_or_create(tab2);

        // Split only tab1
        manager
            .get_mut(tab1)
            .unwrap()
            .split(SplitDirection::Vertical)
            .unwrap();

        // Verify independence
        assert!(manager.is_split(tab1));
        assert!(!manager.is_split(tab2));
        assert_eq!(manager.panel_count(tab1), 2);
        assert_eq!(manager.panel_count(tab2), 1);
    }

    #[test]
    #[ignore = "requires GTK initialization"]
    fn color_released_on_remove() {
        let mut manager = TabSplitManager::new();
        let tab_id = TabId::new();

        let _ = manager.get_or_create(tab_id);
        let color1 = manager.allocate_color(tab_id).unwrap();

        // Remove the tab (should release color)
        manager.remove(tab_id);

        // Create a new tab and allocate color - should get the same color back
        let tab_id2 = TabId::new();
        let _ = manager.get_or_create(tab_id2);
        let color2 = manager.allocate_color(tab_id2).unwrap();

        assert_eq!(color1, color2);
    }
}
