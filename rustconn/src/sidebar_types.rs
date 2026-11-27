//! Type definitions for connection sidebar
//!
//! This module contains types, enums, and helper structs used by the sidebar widget.

use std::cell::RefCell;
use std::collections::HashSet;

use gtk4::prelude::*;
use gtk4::{
    CssProvider, MultiSelection, Orientation, Separator, SingleSelection, TreeListModel, Widget,
    gio,
};
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
    /// The sessions open for this connection, by session id.
    ///
    /// A set, not a counter: one session ending is reported by several paths
    /// (the close-tab action, the notebook's page-closed hook, the child-exit
    /// handler, a protocol's Disconnected state), and with a counter each extra
    /// report closed another session's slot — closing one of two tabs cleared
    /// the connection's "connected" mark while the other was still open.
    pub sessions: std::collections::HashSet<uuid::Uuid>,
}

/// Row decorations that have to survive a sidebar rebuild.
///
/// `rebuild_sidebar_sorted` throws the whole tree away and builds new
/// `ConnectionItem`s, so any state that only ever arrived through a property
/// setter is gone with the old objects. The connected/connecting/failed status
/// already had [`SessionStatusInfo`] to be read back from; these three had
/// nothing, so every reload — a rename, a duplicate, a pin toggle, a re-sort, a
/// drag-drop — silently cleared the recording dot, the external-viewer emblem
/// and the split marker, for pinned and unpinned connections alike.
///
/// Kept apart from [`SessionStatusInfo`] on purpose: that one is session
/// bookkeeping (how many sessions are open), this one is what the row draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowIndicators {
    /// A session of this connection is being recorded.
    pub recording: bool,
    /// The connection has at least one external-viewer session (issue #209).
    pub external_session: bool,
    /// Split palette index, or `-1` when the session is not in a split (R6.2).
    pub split_color: i32,
}

impl Default for RowIndicators {
    fn default() -> Self {
        Self {
            recording: false,
            external_session: false,
            // Matches the `split-color` property's "not in a split" value; a
            // derived `Default` would give 0, i.e. the first palette colour.
            split_color: -1,
        }
    }
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

/// The fraction of a group row's height taken by its top (before) and bottom
/// (after) bands; the middle is the group's "into" zone. A third leaves equal
/// before/after bands and a wider middle band than a half-width split, so the
/// three targets are symmetric and easy to line up on.
const GROUP_BAND_RATIO: f64 = 1.0 / 3.0;

/// Compute where a dragged item lands from the pointer's Y within the target row.
///
/// `y` and `height` are both in the row's real allocated pixels. The zones are
/// symmetric and cover the whole row with no gaps: a plain connection splits
/// into a top (before) and bottom (after) half; a group keeps a top and bottom
/// `before`/`after` band of `height * GROUP_BAND_RATIO` and a middle `into` band.
/// An import group takes no "into" drop, so it uses the connection halves.
pub fn drop_position(y: f64, height: f64, is_group: bool, is_import_group: bool) -> DropPosition {
    if is_group && !is_import_group {
        let band = height * GROUP_BAND_RATIO;
        if y < band {
            DropPosition::Before
        } else if y > height - band {
            DropPosition::After
        } else {
            DropPosition::Into
        }
    } else if y < height / 2.0 {
        DropPosition::Before
    } else {
        DropPosition::After
    }
}

#[cfg(test)]
mod drop_position_tests {
    use super::{DropPosition, drop_position};

    /// The row heights we actually render at, short to tall.
    const HEIGHTS: [f64; 3] = [36.0, 44.0, 60.0];

    /// A plain connection is split into a top (before) and bottom (after) half,
    /// at every row height we render at.
    #[test]
    fn connection_split_at_half() {
        for h in HEIGHTS {
            assert_eq!(drop_position(0.0, h, false, false), DropPosition::Before);
            assert_eq!(
                drop_position(h * 0.25, h, false, false),
                DropPosition::Before
            );
            assert_eq!(drop_position(h / 2.0, h, false, false), DropPosition::After);
            assert_eq!(
                drop_position(h * 0.75, h, false, false),
                DropPosition::After
            );
            assert_eq!(drop_position(h - 0.5, h, false, false), DropPosition::After);
        }
    }

    /// A group keeps a top and bottom before/after band and a middle into band.
    #[test]
    fn group_bands_top_bottom_and_into() {
        for h in HEIGHTS {
            let band = h / 3.0;
            assert_eq!(drop_position(0.0, h, true, false), DropPosition::Before);
            assert_eq!(
                drop_position(band / 2.0, h, true, false),
                DropPosition::Before
            );
            assert_eq!(drop_position(band, h, true, false), DropPosition::Into);
            assert_eq!(drop_position(h / 2.0, h, true, false), DropPosition::Into);
            assert_eq!(drop_position(h - band, h, true, false), DropPosition::Into);
            assert_eq!(
                drop_position(h - band / 2.0, h, true, false),
                DropPosition::After
            );
            assert_eq!(drop_position(h - 0.5, h, true, false), DropPosition::After);
        }
    }

    /// An import group takes no into drop — it behaves as a plain connection.
    #[test]
    fn import_group_has_no_into() {
        for h in HEIGHTS {
            assert_eq!(drop_position(0.0, h, true, true), DropPosition::Before);
            assert_eq!(drop_position(h / 2.0, h, true, true), DropPosition::After);
        }
    }

    /// The zones are symmetric: a point near the top that lands before has a
    /// mirror near the bottom that lands after, and the row is covered with no
    /// gap (every y is before, into or after).
    #[test]
    fn zones_are_symmetric_and_cover_the_row() {
        for h in HEIGHTS {
            assert_eq!(drop_position(h * 0.1, h, true, false), DropPosition::Before);
            assert_eq!(drop_position(h * 0.9, h, true, false), DropPosition::After);
            // Sweep: the before band and the after band are equal thirds.
            assert_eq!(
                drop_position(h / 3.0 - 1e-3, h, true, false),
                DropPosition::Before
            );
            assert_eq!(
                drop_position(h - h / 3.0 + 1e-3, h, true, false),
                DropPosition::After
            );
        }
    }
}

// `DragDropData` used to sit here, marked `#[expect(dead_code, reason = "Fields
// used by drag-drop callback system")]`. No callback system ever referenced it —
// the drag payload is encoded as a string in `sidebar::drag_drop` — so the reason
// described an intention rather than the code, and the attribute is what kept the
// compiler from saying so.

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
    /// Currently highlighted widget (for CSS class management)
    ///
    /// This, plus the `drop-target-*` CSS classes, is the whole of the drop
    /// visual. A `target_index` and a `highlighted_group_index` used to be
    /// tracked alongside it, left over from the pre-CSS approach: both were
    /// written and never read, `set_highlighted_group` was only ever called with
    /// `None`, and `show()` took an index its one caller passed as `0` under a
    /// comment saying it was unused.
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
            current_widget: RefCell::new(None),
        }
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
                background-color: @accent_bg_color;
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
            
            /* Drop indicator line BEFORE this row (line at top). box-shadow +
               background keep the row's own margin/padding, so it does not resize
               under the cursor — a layout shift would flip the pointer between
               before and into and flicker. The 4px inset line is the target. */
            .drop-target-before {
                box-shadow: inset 0 4px 0 0 @accent_bg_color;
                background-color: alpha(@accent_bg_color, 0.14);
            }

            /* Drop indicator line AFTER this row (line at bottom) */
            .drop-target-after {
                box-shadow: inset 0 -4px 0 0 @accent_bg_color;
                background-color: alpha(@accent_bg_color, 0.14);
            }

            /* Status icons - using Adwaita semantic colors */
            .status-connected {
                color: @success_color;
            }
            .status-connecting {
                color: @warning_color;
            }
            .status-failed {
                color: @error_color;
            }
            
            /* Group highlight for drop-into */
            .drop-target-into {
                box-shadow: inset 0 0 0 2px @accent_bg_color;
                background-color: alpha(@accent_bg_color, 0.2);
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

            /* Split-membership marker (R6.2): a small filled square shown in the
               sidebar row while a session is part of a split. The square shape
               is the orthogonal (color-independent) cue; the per-index color
               only mirrors the split pane and matches SPLIT_COLOR_VALUES in
               split_view/bridge.rs. Sized no larger than the 10px status icon. */
            .split-marker {
                min-width: 9px;
                min-height: 9px;
                border-radius: 2px;
            }
            .split-marker.sidebar-split-0 { background-color: #3584e4; } /* Blue 3 */
            .split-marker.sidebar-split-1 { background-color: #26a269; } /* Green 5 */
            .split-marker.sidebar-split-2 { background-color: #e66100; } /* Orange 4 */
            .split-marker.sidebar-split-3 { background-color: #c061cb; } /* Purple 2 */
            .split-marker.sidebar-split-4 { background-color: #2190a4; } /* Teal */
            .split-marker.sidebar-split-5 { background-color: #ed333b; } /* Red 2 */

            ",
        );

        // Use safe display access
        crate::utils::add_css_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_USER + 1);
    }

    /// Returns the indicator widget
    #[must_use]
    pub const fn widget(&self) -> &Separator {
        &self.indicator
    }

    /// Records the drop position.
    ///
    /// Takes no row index: the visual comes from the `drop-target-*` CSS class
    /// on the row widget, so there is nothing an index could position.
    pub fn show(&self, position: DropPosition) {
        *self.position.borrow_mut() = Some(position);
        // Keep overlay indicator hidden - we use CSS classes now
        self.indicator.set_visible(false);
    }

    /// Hides the indicator and clears CSS classes
    pub fn hide(&self) {
        *self.position.borrow_mut() = None;
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

    // `target_index()`, `is_visible()` and `highlighted_group_index()` were
    // removed here. All three were unreachable and all three carried the reason
    // "kept alive for GTK widget lifecycle" — which cannot apply to a method:
    // only a *field* keeps a widget alive. `position()` above is kept because
    // `sidebar::mod` genuinely calls it.
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
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "value range fits the target type by construction in this code path"
                    )]
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
pub const MAX_SEARCH_HISTORY: usize = 20;
