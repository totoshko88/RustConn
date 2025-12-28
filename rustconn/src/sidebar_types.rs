//! Type definitions for connection sidebar
//!
//! This module contains types, enums, and helper structs used by the sidebar widget.

use gtk4::prelude::*;
use gtk4::{
    gdk, gio, CssProvider, MultiSelection, Orientation, Separator, SingleSelection, TreeListModel,
    Widget,
};
use std::cell::RefCell;
use std::collections::HashSet;
use uuid::Uuid;

/// Tree state for preservation across refreshes
///
/// Captures the current state of the connection tree including which groups
/// are expanded, the scroll position, and the currently selected item.
/// This allows the tree to be refreshed while maintaining the user's view.
#[derive(Debug, Clone, Default)]
pub struct TreeState {
    /// IDs of groups that are currently expanded
    pub expanded_groups: HashSet<Uuid>,
    /// Vertical scroll position (adjustment value)
    pub scroll_position: f64,
    /// ID of the currently selected item
    pub selected_id: Option<Uuid>,
}

/// Session status information for a connection
///
/// Tracks the current status and number of active sessions for a connection.
/// This allows proper status management when multiple sessions are opened
/// for the same connection.
#[derive(Debug, Clone, Default)]
pub struct SessionStatusInfo {
    /// Current status (connected, connecting, failed, disconnected)
    pub status: String,
    /// Number of active sessions for this connection
    pub active_count: usize,
}

/// Drop position relative to a target item
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPosition {
    /// Drop before the target item
    Before,
    /// Drop after the target item
    After,
    /// Drop into the target item (for groups)
    Into,
}

/// Data for a drag-drop operation
///
/// This struct is used by `invoke_drag_drop()` and `set_drag_drop_callback()` methods
/// to pass drag-drop operation details to registered callbacks.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used by drag-drop callback system
pub struct DragDropData {
    /// Type of the dragged item ("conn" or "group")
    pub item_type: String,
    /// ID of the dragged item
    pub item_id: String,
    /// ID of the target item
    pub target_id: String,
    /// Whether the target is a group
    pub target_is_group: bool,
}

/// Visual indicator for drag-and-drop operations
///
/// Shows a horizontal line between items or highlights groups
/// to indicate where a dragged item will be placed.
/// Uses CSS classes on row widgets for precise positioning.
#[derive(Debug, Clone)]
pub struct DropIndicator {
    /// The separator widget (kept for overlay fallback, hidden by default)
    indicator: Separator,
    /// Current drop position type
    position: RefCell<Option<DropPosition>>,
    /// Target row index for the drop
    target_index: RefCell<Option<u32>>,
    /// Currently highlighted group index (for drop-into visual)
    highlighted_group_index: RefCell<Option<u32>>,
    /// Currently highlighted widget (for CSS class management)
    current_widget: RefCell<Option<Widget>>,
}

impl DropIndicator {
    /// Creates a new drop indicator widget
    #[must_use]
    pub fn new() -> Self {
        let indicator = Separator::new(Orientation::Horizontal);
        indicator.add_css_class("drop-indicator");
        indicator.set_visible(false);
        indicator.set_height_request(3);
        indicator.set_can_target(false);
        indicator.set_hexpand(true);
        indicator.set_valign(gtk4::Align::Start);

        // Load CSS for the drop indicator
        Self::load_css();

        Self {
            indicator,
            position: RefCell::new(None),
            target_index: RefCell::new(None),
            highlighted_group_index: RefCell::new(None),
            current_widget: RefCell::new(None),
        }
    }

    /// Sets the highlighted group index
    pub fn set_highlighted_group(&self, index: Option<u32>) {
        *self.highlighted_group_index.borrow_mut() = index;
    }

    /// Returns the highlighted group index
    ///
    /// Note: Part of drag-drop API, used internally by drop target handlers.
    #[must_use]
    #[allow(dead_code)]
    pub fn highlighted_group_index(&self) -> Option<u32> {
        *self.highlighted_group_index.borrow()
    }

    /// Clears CSS classes from the currently highlighted widget
    pub fn clear_current_widget(&self) {
        if let Some(widget) = self.current_widget.borrow().as_ref() {
            widget.remove_css_class("drop-target-before");
            widget.remove_css_class("drop-target-after");
            widget.remove_css_class("drop-target-into");
        }
        *self.current_widget.borrow_mut() = None;
    }

    /// Sets the current widget and applies the appropriate CSS class
    pub fn set_current_widget(&self, widget: Option<Widget>, position: DropPosition) {
        // Clear previous widget
        self.clear_current_widget();

        // Set new widget with CSS class
        if let Some(ref w) = widget {
            match position {
                DropPosition::Before => w.add_css_class("drop-target-before"),
                DropPosition::After => w.add_css_class("drop-target-after"),
                DropPosition::Into => w.add_css_class("drop-target-into"),
            }
        }
        *self.current_widget.borrow_mut() = widget;
    }

    /// Loads the CSS styling for the drop indicator
    fn load_css() {
        let provider = CssProvider::new();
        provider.load_from_string(
            r"
            /* Hide the overlay indicator - we use CSS borders instead */
            .drop-indicator {
                background-color: #aa4400;
                min-height: 3px;
                margin-left: 8px;
                margin-right: 8px;
                opacity: 1;
            }
            
            /* Disable GTK's default drop frame/border on ALL elements */
            *:drop(active) {
                background: none;
                background-color: transparent;
                background-image: none;
                border: none;
                border-color: transparent;
                border-width: 0;
                outline: none;
                outline-width: 0;
                box-shadow: none;
            }
            
            /* Specifically target list view elements */
            listview:drop(active),
            listview row:drop(active),
            listview > row:drop(active),
            .navigation-sidebar:drop(active),
            .navigation-sidebar row:drop(active),
            .navigation-sidebar > row:drop(active),
            treeexpander:drop(active),
            treeexpander > *:drop(active),
            row:drop(active),
            row > *:drop(active),
            box:drop(active) {
                background: none;
                background-color: transparent;
                background-image: none;
                border: none;
                border-color: transparent;
                border-width: 0;
                outline: none;
                outline-width: 0;
                box-shadow: none;
            }
            
            /* Drop indicator line BEFORE this row (line at top) */
            .drop-target-before {
                border-top: 3px solid #aa4400;
                margin-top: 4px;
                padding-top: 4px;
            }
            
            /* Drop indicator line AFTER this row (line at bottom) */
            .drop-target-after {
                border-bottom: 3px solid #aa4400;
                margin-bottom: 4px;
                padding-bottom: 4px;
            }

            /* Status icons */
            .status-connected {
                color: #2ec27e; /* Green */
            }
            .status-connecting {
                color: #f5c211; /* Yellow */
            }
            .status-failed {
                color: #e01b24; /* Red */
            }
            
            /* Group highlight for drop-into */
            .drop-target-into {
                background-color: alpha(#aa4400, 0.2);
                border: 2px solid #aa4400;
                border-radius: 6px;
            }
            
            /* Legacy classes for compatibility */
            .drop-highlight {
                background-color: alpha(@accent_bg_color, 0.3);
                border: 2px solid @accent_bg_color;
                border-radius: 6px;
            }
            
            .drop-into-group {
                background-color: alpha(@accent_bg_color, 0.15);
            }
            .drop-into-group row:selected {
                background-color: alpha(@accent_bg_color, 0.4);
                border-radius: 6px;
            }
            
            .status-connected {
                color: #2ec27e;
            }
            .status-connecting {
                color: #e5a50a;
            }
            ",
        );

        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().expect("Could not get default display"),
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );
    }

    /// Returns the indicator widget
    #[must_use]
    pub const fn widget(&self) -> &Separator {
        &self.indicator
    }

    /// Shows the indicator at the specified position
    pub fn show(&self, position: DropPosition, target_index: u32) {
        *self.position.borrow_mut() = Some(position);
        *self.target_index.borrow_mut() = Some(target_index);
        // Keep overlay indicator hidden - we use CSS classes now
        self.indicator.set_visible(false);
    }

    /// Hides the indicator and clears CSS classes
    pub fn hide(&self) {
        *self.position.borrow_mut() = None;
        *self.target_index.borrow_mut() = None;
        self.indicator.set_visible(false);
        // Clear CSS classes from current widget
        self.clear_current_widget();
    }

    /// Returns the current widget
    pub fn current_widget(&self) -> Option<Widget> {
        self.current_widget.borrow().clone()
    }

    /// Returns the current drop position
    #[must_use]
    pub fn position(&self) -> Option<DropPosition> {
        *self.position.borrow()
    }

    /// Returns the current target index
    ///
    /// Note: Part of drag-drop API for determining drop position.
    #[must_use]
    #[allow(dead_code)]
    pub fn target_index(&self) -> Option<u32> {
        *self.target_index.borrow()
    }

    /// Returns whether the indicator is currently visible
    ///
    /// Note: Part of drag-drop API for visual feedback state.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.indicator.is_visible()
    }
}

impl Default for DropIndicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper to switch between selection models
/// Supports switching between `SingleSelection` and `MultiSelection` modes
pub enum SelectionModelWrapper {
    /// Single selection mode (default)
    Single(SingleSelection),
    /// Multi-selection mode for group operations
    Multi(MultiSelection),
}

impl SelectionModelWrapper {
    /// Creates a new single selection wrapper
    #[must_use]
    pub fn new_single(model: TreeListModel) -> Self {
        Self::Single(SingleSelection::new(Some(model)))
    }

    /// Creates a new multi-selection wrapper
    #[must_use]
    pub fn new_multi(model: TreeListModel) -> Self {
        Self::Multi(MultiSelection::new(Some(model)))
    }

    /// Returns true if in multi-selection mode
    #[must_use]
    pub const fn is_multi(&self) -> bool {
        matches!(self, Self::Multi(_))
    }

    /// Gets all selected item positions
    #[must_use]
    pub fn get_selected_positions(&self) -> Vec<u32> {
        match self {
            Self::Single(s) => {
                let selected = s.selected();
                if selected == gtk4::INVALID_LIST_POSITION {
                    vec![]
                } else {
                    vec![selected]
                }
            }
            Self::Multi(m) => {
                let selection = m.selection();
                let mut positions = Vec::new();
                // Iterate through the bitset using nth() which returns the nth set bit
                let size = selection.size();
                for i in 0..size {
                    #[allow(clippy::cast_possible_truncation)]
                    let pos = selection.nth(i as u32);
                    if pos != u32::MAX {
                        positions.push(pos);
                    }
                }
                positions
            }
        }
    }

    /// Selects all items (only works in multi-selection mode)
    pub fn select_all(&self) {
        if let Self::Multi(m) = self {
            m.select_all();
        }
    }

    /// Clears all selections
    pub fn clear_selection(&self) {
        match self {
            Self::Single(s) => {
                s.set_selected(gtk4::INVALID_LIST_POSITION);
            }
            Self::Multi(m) => {
                m.unselect_all();
            }
        }
    }

    /// Gets the underlying model
    #[must_use]
    pub fn model(&self) -> Option<gio::ListModel> {
        match self {
            Self::Single(s) => s.model(),
            Self::Multi(m) => m.model(),
        }
    }
}

/// Maximum number of search history entries to keep
pub const MAX_SEARCH_HISTORY: usize = 10;
