//! Connection tree sidebar
//!
//! This module provides the sidebar widget for displaying and managing
//! the connection hierarchy with drag-and-drop support.
//!
//! ## Lazy Loading
//!
//! For large connection databases, the sidebar supports lazy loading of
//! connection groups. When enabled, only root-level groups and ungrouped
//! connections are loaded initially. Child groups and connections are
//! loaded on demand when a group is expanded.

// Allow items_after_statements for const definitions inside functions
#![allow(clippy::items_after_statements)]

use gtk4::prelude::*;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use gtk4::{
    gdk, gio, glib, Box as GtkBox, Button, CssProvider, DragSource, DropTarget, EventControllerKey,
    GestureClick, Label, ListItem, ListView, MultiSelection, Orientation, PolicyType,
    ScrolledWindow, SearchEntry, Separator, SignalListItemFactory, SingleSelection, TreeExpander,
    TreeListModel, TreeListRow, Widget,
};
use rustconn_core::{
    Debouncer, LazyGroupLoader, SelectionState as CoreSelectionState, VirtualScrollConfig,
    VirtualScroller,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
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
    #[must_use]
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

    /// Returns the current drop position
    #[must_use]
    pub fn position(&self) -> Option<DropPosition> {
        *self.position.borrow()
    }

    /// Returns the current target index
    #[must_use]
    pub fn target_index(&self) -> Option<u32> {
        *self.target_index.borrow()
    }

    /// Returns whether the indicator is currently visible
    #[must_use]
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

    /// Returns the underlying selection model as a `SelectionModel`
    ///
    /// Note: This method only works in single selection mode. In multi-selection
    /// mode, it will panic. Use `is_multi()` to check the mode first.
    #[must_use]
    pub fn as_selection_model(&self) -> &impl IsA<gtk4::SelectionModel> {
        match self {
            Self::Single(s) => s,
            Self::Multi(_) => panic!("Cannot return MultiSelection as SelectionModel reference"),
        }
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

/// Data for a drag-drop operation
///
/// This struct is used by `invoke_drag_drop()` and `set_drag_drop_callback()` methods
/// to pass drag-drop operation details to registered callbacks.
#[derive(Debug, Clone)]
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

/// Maximum number of search history entries to keep
const MAX_SEARCH_HISTORY: usize = 10;

/// Sidebar widget for connection tree display
pub struct ConnectionSidebar {
    container: GtkBox,
    search_entry: SearchEntry,
    list_view: ListView,
    /// Store for connection data - will be populated from `ConnectionManager`
    store: gio::ListStore,
    /// Tree list model for hierarchical display
    tree_model: TreeListModel,
    /// Selection model - switches between Single and Multi
    selection_model: Rc<RefCell<SelectionModelWrapper>>,
    /// Bulk actions toolbar (visible in group ops mode)
    bulk_actions_bar: GtkBox,
    /// Current mode
    group_ops_mode: Rc<RefCell<bool>>,
    /// Callback for drag-drop operations
    ///
    /// Used by `set_drag_drop_callback()` and `invoke_drag_drop()` methods
    /// to handle drag-drop events from the connection tree.
    drag_drop_callback: Rc<RefCell<Option<Box<dyn Fn(DragDropData)>>>>,
    /// Search history
    search_history: Rc<RefCell<Vec<String>>>,
    /// Search history popover
    history_popover: gtk4::Popover,
    /// Drop indicator for drag-and-drop visual feedback
    drop_indicator: Rc<DropIndicator>,
    /// Scrolled window containing the list view
    scrolled_window: ScrolledWindow,
    /// Lazy group loader for on-demand loading of connection groups
    lazy_loader: Rc<RefCell<LazyGroupLoader>>,
    /// Virtual scroller for efficient rendering of large lists
    virtual_scroller: Rc<RefCell<Option<VirtualScroller>>>,
    /// Virtual scroll configuration
    virtual_scroll_config: VirtualScrollConfig,
    /// Selection state for preserving selections across virtual scrolling
    selection_state: Rc<RefCell<CoreSelectionState>>,
    /// Debouncer for rate-limiting search operations (100ms delay)
    search_debouncer: Rc<Debouncer>,
    /// Spinner widget to show search is pending during debounce
    search_spinner: gtk4::Spinner,
    /// Pending search query during debounce period
    pending_search_query: Rc<RefCell<Option<String>>>,
    /// Saved tree state before search (for restoration when search is cleared)
    pre_search_state: Rc<RefCell<Option<TreeState>>>,
}

impl ConnectionSidebar {
    /// Creates a new connection sidebar
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_width_request(250);
        container.add_css_class("sidebar");

        // Search box with entry and help button
        let search_box = GtkBox::new(Orientation::Horizontal, 4);
        search_box.set_margin_start(8);
        search_box.set_margin_end(8);
        search_box.set_margin_top(8);
        search_box.set_margin_bottom(8);

        // Search entry
        let search_entry = SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search... (? for help)"));
        search_entry.set_hexpand(true);
        // Accessibility: set label for screen readers
        search_entry.update_property(&[gtk4::accessible::Property::Label("Search connections")]);
        search_box.append(&search_entry);

        // Search pending spinner (hidden by default)
        let search_spinner = gtk4::Spinner::new();
        search_spinner.set_visible(false);
        search_spinner.set_tooltip_text(Some("Search pending..."));
        search_box.append(&search_spinner);

        // Help button with popover
        let help_button = Button::from_icon_name("dialog-question-symbolic");
        help_button.set_tooltip_text(Some("Search syntax help"));
        help_button.add_css_class("flat");

        // Create search help popover
        let help_popover = Self::create_search_help_popover();
        help_popover.set_parent(&help_button);

        let help_popover_clone = help_popover.clone();
        help_button.connect_clicked(move |_| {
            help_popover_clone.popup();
        });

        search_box.append(&help_button);
        container.append(&search_box);

        // Create search history storage and popover
        let search_history: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let history_popover = Self::create_history_popover(&search_entry, search_history.clone());
        history_popover.set_parent(&search_entry);

        // Show help popover when user types '?'
        let help_popover_for_key = help_popover;
        let search_entry_clone = search_entry.clone();
        let search_history_clone = search_history.clone();
        let history_popover_clone = history_popover.clone();
        search_entry.connect_search_changed(move |entry| {
            let text = entry.text();
            if text.as_str() == "?" {
                entry.set_text("");
                help_popover_for_key.popup();
            }
        });

        // Show history dropdown when search entry is focused and empty
        let history_popover_for_focus = history_popover.clone();
        let search_history_for_focus = search_history.clone();
        search_entry.connect_has_focus_notify(move |entry| {
            if entry.has_focus() && entry.text().is_empty() {
                let history = search_history_for_focus.borrow();
                if !history.is_empty() {
                    history_popover_for_focus.popup();
                }
            }
        });

        // Setup search entry key handler for operator hints and history navigation
        Self::setup_search_entry_hints(
            &search_entry,
            &search_entry_clone,
            &history_popover_clone,
            &search_history_clone,
        );

        // Create bulk actions toolbar (hidden by default)
        let bulk_actions_bar = Self::create_bulk_actions_bar();
        bulk_actions_bar.set_visible(false);
        container.append(&bulk_actions_bar);

        // Create the list store for connection items
        let store = gio::ListStore::new::<ConnectionItem>();

        // Create tree list model for hierarchical display
        let tree_model = TreeListModel::new(store.clone(), false, true, |item| {
            item.downcast_ref::<ConnectionItem>()
                .and_then(ConnectionItem::children)
        });

        // Create selection model (starts in single selection mode)
        let selection_wrapper = SelectionModelWrapper::new_single(tree_model.clone());
        let selection_model = Rc::new(RefCell::new(selection_wrapper));

        // Create the factory for list items
        let factory = SignalListItemFactory::new();
        let group_ops_mode = Rc::new(RefCell::new(false));
        let group_ops_mode_clone = group_ops_mode.clone();
        factory.connect_setup(move |factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::setup_list_item(factory, list_item, *group_ops_mode_clone.borrow());
            }
        });
        factory.connect_bind(|factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::bind_list_item(factory, list_item);
            }
        });

        // Create the list view with single selection initially
        let list_view = {
            let sel = selection_model.borrow();
            match &*sel {
                SelectionModelWrapper::Single(s) => ListView::new(Some(s.clone()), Some(factory)),
                SelectionModelWrapper::Multi(m) => ListView::new(Some(m.clone()), Some(factory)),
            }
        };
        list_view.add_css_class("navigation-sidebar");

        // Set accessibility properties
        list_view.update_property(&[gtk4::accessible::Property::Label("Connection list")]);
        list_view.set_focusable(true);
        list_view.set_can_focus(true);

        // Set up keyboard navigation
        let selection_model_clone = selection_model.clone();
        let key_controller = EventControllerKey::new();
        key_controller.connect_key_pressed(move |_controller, key, _code, state| {
            // Use is_multi() to check if we're in multi-selection mode
            let is_multi_mode = selection_model_clone.borrow().is_multi();

            // Handle keyboard navigation
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    // Activate selected item - handled by ListView's activate signal
                    glib::Propagation::Stop
                }
                gdk::Key::Delete => {
                    // Delete selected item - will be handled by window action
                    glib::Propagation::Proceed
                }
                gdk::Key::a | gdk::Key::A
                    if state.contains(gdk::ModifierType::CONTROL_MASK) && is_multi_mode =>
                {
                    // Ctrl+A: Select all in multi-selection mode
                    selection_model_clone.borrow().select_all();
                    glib::Propagation::Stop
                }
                gdk::Key::Escape if is_multi_mode => {
                    // Escape: Clear selection in multi-selection mode
                    selection_model_clone.borrow().clear_selection();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        list_view.add_controller(key_controller);

        // Create drop indicator for drag-and-drop visual feedback
        let drop_indicator = Rc::new(DropIndicator::new());

        // Create an overlay to position the drop indicator over the list
        let overlay = gtk4::Overlay::new();

        // Wrap in scrolled window
        let scrolled_window = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&list_view)
            .build();

        overlay.set_child(Some(&scrolled_window));

        // Add drop indicator as overlay - it will be positioned via margin_top
        let indicator_widget = drop_indicator.widget();
        overlay.add_overlay(indicator_widget);
        // Don't let the overlay affect the size measurement
        overlay.set_measure_overlay(indicator_widget, false);
        // Ensure indicator is clipped to overlay bounds
        overlay.set_clip_overlay(indicator_widget, true);

        // Set up drop target on the list view for motion tracking
        let list_view_drop_target = DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);

        // Track motion during drag for visual feedback
        let drop_indicator_motion = drop_indicator.clone();
        let list_view_for_motion = list_view.clone();
        let tree_model_for_motion = tree_model.clone();
        list_view_drop_target.connect_motion(move |_target, x, y| {
            Self::update_drop_indicator(
                &drop_indicator_motion,
                &list_view_for_motion,
                &tree_model_for_motion,
                x,
                y,
            )
        });

        // Hide indicator when drag leaves
        let drop_indicator_leave = drop_indicator.clone();
        let list_view_for_leave = list_view.clone();
        list_view_drop_target.connect_leave(move |_target| {
            // Hide the line indicator
            drop_indicator_leave.hide();
            // Clear the highlighted group tracking
            drop_indicator_leave.set_highlighted_group(None);
            // Remove all drop-related CSS classes
            list_view_for_leave.remove_css_class("drop-active");
            list_view_for_leave.remove_css_class("drop-into-group");
        });

        // The actual drop is handled by per-item drop targets, so we reject here
        list_view_drop_target.connect_drop(|_target, _value, _x, _y| false);

        list_view.add_controller(list_view_drop_target);

        container.append(&overlay);

        // Add buttons at the bottom
        let button_box = Self::create_button_box();
        container.append(&button_box);

        // Create debouncer for search with 100ms delay
        let search_debouncer = Rc::new(Debouncer::for_search());

        Self {
            container,
            search_entry,
            list_view,
            store,
            tree_model,
            selection_model,
            bulk_actions_bar,
            group_ops_mode,
            drag_drop_callback: Rc::new(RefCell::new(None)),
            search_history,
            history_popover,
            drop_indicator,
            scrolled_window,
            lazy_loader: Rc::new(RefCell::new(LazyGroupLoader::new())),
            virtual_scroller: Rc::new(RefCell::new(None)),
            virtual_scroll_config: VirtualScrollConfig::default(),
            selection_state: Rc::new(RefCell::new(CoreSelectionState::new())),
            search_debouncer,
            search_spinner,
            pending_search_query: Rc::new(RefCell::new(None)),
            pre_search_state: Rc::new(RefCell::new(None)),
        }
    }

    /// Sets the callback for drag-drop operations
    pub fn set_drag_drop_callback<F>(&self, callback: F)
    where
        F: Fn(DragDropData) + 'static,
    {
        *self.drag_drop_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Invokes the drag-drop callback if set
    pub fn invoke_drag_drop(&self, data: DragDropData) {
        if let Some(ref callback) = *self.drag_drop_callback.borrow() {
            callback(data);
        }
    }

    /// Creates the bulk actions toolbar for group operations mode
    fn create_bulk_actions_bar() -> GtkBox {
        let bar = GtkBox::new(Orientation::Horizontal, 4);
        bar.set_margin_start(8);
        bar.set_margin_end(8);
        bar.set_margin_top(4);
        bar.set_margin_bottom(4);
        bar.add_css_class("bulk-actions-bar");

        // Delete Selected button
        let delete_button = Button::with_label("Delete Selected");
        delete_button.set_tooltip_text(Some("Delete all selected items"));
        delete_button.set_action_name(Some("win.delete-selected"));
        delete_button.add_css_class("destructive-action");
        delete_button.update_property(&[gtk4::accessible::Property::Label(
            "Delete selected connections",
        )]);
        bar.append(&delete_button);

        // Move to Group dropdown button
        let move_button = Button::with_label("Move to Group...");
        move_button.set_tooltip_text(Some("Move selected items to a group"));
        move_button.set_action_name(Some("win.move-selected-to-group"));
        move_button.update_property(&[gtk4::accessible::Property::Label(
            "Move selected connections to group",
        )]);
        bar.append(&move_button);

        // Select All button
        let select_all_button = Button::with_label("Select All");
        select_all_button.set_tooltip_text(Some("Select all items (Ctrl+A)"));
        select_all_button.set_action_name(Some("win.select-all"));
        select_all_button
            .update_property(&[gtk4::accessible::Property::Label("Select all connections")]);
        bar.append(&select_all_button);

        // Clear Selection button
        let clear_button = Button::with_label("Clear");
        clear_button.set_tooltip_text(Some("Clear selection (Escape)"));
        clear_button.set_action_name(Some("win.clear-selection"));
        clear_button.update_property(&[gtk4::accessible::Property::Label("Clear selection")]);
        bar.append(&clear_button);

        bar
    }

    /// Creates the button box at the bottom of the sidebar
    fn create_button_box() -> GtkBox {
        let button_box = GtkBox::new(Orientation::Horizontal, 4);
        button_box.set_margin_start(8);
        button_box.set_margin_end(8);
        button_box.set_margin_top(8);
        button_box.set_margin_bottom(8);
        button_box.set_halign(gtk4::Align::Center);

        // Add connection button
        let add_button = Button::from_icon_name("list-add-symbolic");
        add_button.set_tooltip_text(Some("Add Connection (Ctrl+N)"));
        add_button.set_action_name(Some("win.new-connection"));
        add_button.update_property(&[gtk4::accessible::Property::Label("Add new connection")]);
        button_box.append(&add_button);

        // Delete button
        let delete_button = Button::from_icon_name("list-remove-symbolic");
        delete_button.set_tooltip_text(Some("Delete Selected (Delete)"));
        delete_button.set_action_name(Some("win.delete-connection"));
        delete_button.update_property(&[gtk4::accessible::Property::Label(
            "Delete selected connection or group",
        )]);
        button_box.append(&delete_button);

        // Add group button
        let add_group_button = Button::from_icon_name("folder-new-symbolic");
        add_group_button.set_tooltip_text(Some("Add Group (Ctrl+Shift+N)"));
        add_group_button.set_action_name(Some("win.new-group"));
        add_group_button.update_property(&[gtk4::accessible::Property::Label("Add new group")]);
        button_box.append(&add_group_button);

        // Quick connect button
        let quick_connect_button = Button::from_icon_name("network-transmit-symbolic");
        quick_connect_button.set_tooltip_text(Some("Quick Connect (without saving)"));
        quick_connect_button.set_action_name(Some("win.quick-connect"));
        quick_connect_button.update_property(&[gtk4::accessible::Property::Label(
            "Quick connect without saving",
        )]);
        button_box.append(&quick_connect_button);

        // Group operations button
        let group_ops_button = Button::from_icon_name("view-list-symbolic");
        group_ops_button.set_tooltip_text(Some("Group Operations Mode"));
        group_ops_button.set_action_name(Some("win.group-operations"));
        group_ops_button.update_property(&[gtk4::accessible::Property::Label(
            "Enable group operations mode for multi-select",
        )]);
        button_box.append(&group_ops_button);

        // Sort button
        let sort_button = Button::from_icon_name("view-sort-ascending-symbolic");
        sort_button.set_tooltip_text(Some("Sort Alphabetically"));
        sort_button.set_action_name(Some("win.sort-connections"));
        sort_button.update_property(&[gtk4::accessible::Property::Label(
            "Sort connections alphabetically",
        )]);
        button_box.append(&sort_button);

        // Sort Recent button
        let sort_recent_button = Button::from_icon_name("document-open-recent-symbolic");
        sort_recent_button.set_tooltip_text(Some("Sort by Recent Usage"));
        sort_recent_button.set_action_name(Some("win.sort-recent"));
        sort_recent_button.update_property(&[gtk4::accessible::Property::Label(
            "Sort connections by recent usage",
        )]);
        button_box.append(&sort_recent_button);

        // Import button
        let import_button = Button::from_icon_name("document-open-symbolic");
        import_button.set_tooltip_text(Some("Import Connections (Ctrl+I)"));
        import_button.set_action_name(Some("win.import"));
        import_button.update_property(&[gtk4::accessible::Property::Label(
            "Import connections from external sources",
        )]);
        button_box.append(&import_button);

        // KeePass button - opens KeePassXC with configured database
        let keepass_button = Button::from_icon_name("dialog-password-symbolic");
        keepass_button.set_tooltip_text(Some("Open KeePass Database"));
        keepass_button.set_action_name(Some("win.open-keepass"));
        keepass_button.set_sensitive(false); // Disabled by default, enabled when integration is active
        keepass_button.update_property(&[gtk4::accessible::Property::Label(
            "Open KeePassXC password database",
        )]);
        button_box.append(&keepass_button);

        // Export button
        let export_button = Button::from_icon_name("document-save-symbolic");
        export_button.set_tooltip_text(Some("Export Configuration"));
        export_button.set_action_name(Some("win.export"));
        export_button.update_property(&[gtk4::accessible::Property::Label(
            "Export configuration to file",
        )]);
        button_box.append(&export_button);

        button_box
    }

    /// Sets up a list item widget
    #[allow(clippy::too_many_lines)]
    fn setup_list_item(
        _factory: &SignalListItemFactory,
        list_item: &ListItem,
        _group_ops_mode: bool,
    ) {
        let expander = TreeExpander::new();

        let content_box = GtkBox::new(Orientation::Horizontal, 8);
        content_box.set_margin_start(4);
        content_box.set_margin_end(4);
        content_box.set_margin_top(4);
        content_box.set_margin_bottom(4);

        let icon = gtk4::Image::from_icon_name("network-server-symbolic");
        content_box.append(&icon);

        let label = Label::new(None);
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        content_box.append(&label);

        expander.set_child(Some(&content_box));
        list_item.set_child(Some(&expander));

        // Set up drag source for reorganization
        let drag_source = DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);

        // Store list_item reference for drag prepare
        let list_item_weak_drag = list_item.downgrade();
        drag_source.connect_prepare(move |_source, _x, _y| {
            // Get the item from the list item
            let list_item = list_item_weak_drag.upgrade()?;
            let row = list_item.item()?.downcast::<TreeListRow>().ok()?;
            let item = row.item()?.downcast::<ConnectionItem>().ok()?;

            // Encode item type and ID in drag data: "type:id"
            let item_type = if item.is_group() { "group" } else { "conn" };
            let drag_data = format!("{}:{}", item_type, item.id());

            Some(gdk::ContentProvider::for_value(&drag_data.to_value()))
        });

        // Clean up drop indicator when drag ends
        drag_source.connect_drag_end(|source, _drag, _delete_data| {
            // Find the sidebar and hide the drop indicator
            if let Some(widget) = source.widget() {
                if let Some(list_view) = widget.ancestor(ListView::static_type()) {
                    // Remove all drop-related CSS classes
                    list_view.remove_css_class("drop-active");
                    list_view.remove_css_class("drop-into-group");
                }
            }
        });

        expander.add_controller(drag_source);

        // Set up drop target
        let drop_target = DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        // Disable the default prelight (frame) effect - we use our own indicator
        drop_target.set_preload(false);

        // Store list_item reference for drop target
        let list_item_weak_drop = list_item.downgrade();
        drop_target.connect_drop(move |target, value, _x, _y| {
            // Parse drag data
            let drag_data = match value.get::<String>() {
                Ok(data) => data,
                Err(_) => return false,
            };

            let parts: Vec<&str> = drag_data.split(':').collect();
            if parts.len() != 2 {
                return false;
            }

            let item_type = parts[0];
            let item_id = parts[1];

            // Get target item info
            let list_item = match list_item_weak_drop.upgrade() {
                Some(li) => li,
                None => return false,
            };

            let row = match list_item
                .item()
                .and_then(|i| i.downcast::<TreeListRow>().ok())
            {
                Some(r) => r,
                None => return false,
            };

            let target_item = match row.item().and_then(|i| i.downcast::<ConnectionItem>().ok()) {
                Some(item) => item,
                None => return false,
            };

            let target_id = target_item.id();
            let target_is_group = target_item.is_group();

            // Don't allow dropping on self
            if item_id == target_id {
                return false;
            }

            // Activate the drag-drop action with the data
            // Format: "item_type:item_id:target_id:target_is_group"
            let action_data = format!("{item_type}:{item_id}:{target_id}:{target_is_group}");

            if let Some(widget) = target.widget() {
                // Hide drop indicator before processing the drop
                let _ = widget.activate_action("win.hide-drop-indicator", None);
                let _ =
                    widget.activate_action("win.drag-drop-item", Some(&action_data.to_variant()));
            }

            true
        });
        expander.add_controller(drop_target);

        // Set up right-click context menu
        // Note: is_group will be determined at bind time via list_item data
        let gesture = GestureClick::new();
        gesture.set_button(gdk::BUTTON_SECONDARY);
        let list_item_weak = list_item.downgrade();
        gesture.connect_pressed(move |gesture, _n_press, x, y| {
            if let Some(widget) = gesture.widget() {
                // First, select this item so context menu actions work on it
                if let Some(list_item) = list_item_weak.upgrade() {
                    // Get the position of this item and select it
                    let position = list_item.position();
                    if let Some(list_view) = widget.ancestor(ListView::static_type()) {
                        if let Some(list_view) = list_view.downcast_ref::<ListView>() {
                            if let Some(model) = list_view.model() {
                                if let Some(selection) = model.downcast_ref::<SingleSelection>() {
                                    selection.set_selected(position);
                                } else if let Some(selection) =
                                    model.downcast_ref::<MultiSelection>()
                                {
                                    // In multi-selection mode, select only this item for context menu
                                    selection.unselect_all();
                                    selection.select_item(position, false);
                                }
                            }
                        }
                    }
                }

                // Check if this is a group by looking at the icon
                let is_group = widget
                    .first_child()
                    .and_then(|c| c.first_child())
                    .and_then(|c| c.downcast::<gtk4::Image>().ok())
                    .is_some_and(|img| {
                        img.icon_name()
                            .is_some_and(|n| n.as_str() == "folder-symbolic")
                    });
                Self::show_context_menu_for_item(&widget, x, y, is_group);
            }
        });
        expander.add_controller(gesture);
    }

    /// Binds data to a list item
    fn bind_list_item(_factory: &SignalListItemFactory, list_item: &ListItem) {
        let Some(expander) = list_item.child().and_downcast::<TreeExpander>() else {
            return;
        };

        let Some(row) = list_item.item().and_downcast::<TreeListRow>() else {
            return;
        };

        expander.set_list_row(Some(&row));

        let Some(item) = row.item().and_downcast::<ConnectionItem>() else {
            return;
        };

        let Some(content_box) = expander.child().and_downcast::<GtkBox>() else {
            return;
        };

        // Update icon based on item type
        if let Some(icon) = content_box.first_child().and_downcast::<gtk4::Image>() {
            if item.is_document() {
                icon.set_icon_name(Some("x-office-document-symbolic"));
            } else if item.is_group() {
                icon.set_icon_name(Some("folder-symbolic"));
            } else {
                let protocol = item.protocol();
                let icon_name = Self::get_protocol_icon(&protocol);
                icon.set_icon_name(Some(icon_name));
            }
        }

        // Update label with dirty indicator for documents
        if let Some(label) = content_box.last_child().and_downcast::<Label>() {
            let name = item.name();
            if item.is_document() && item.is_dirty() {
                label.set_text(&format!("• {name}"));
            } else {
                label.set_text(&name);
            }
        }
    }

    /// Returns the appropriate icon name for a protocol string
    ///
    /// For `ZeroTrust` connections, the protocol string may include provider info
    /// in the format "zerotrust:provider" (e.g., "zerotrust:aws", "zerotrust:gcloud").
    /// This allows showing provider-specific icons for cloud CLI connections.
    ///
    /// Note: We use standard GTK symbolic icons that are guaranteed to exist
    /// in all icon themes. Provider-specific icons (aws-symbolic, etc.) are not
    /// available in standard themes, so we use semantic alternatives.
    fn get_protocol_icon(protocol: &str) -> &'static str {
        // Check for ZeroTrust with provider info (format: "zerotrust:provider")
        if let Some(provider) = protocol.strip_prefix("zerotrust:") {
            // Use standard GTK/Adwaita icons that are guaranteed to exist
            // Each provider has a unique icon - no duplicates with SSH or other protocols
            return match provider {
                "aws" | "aws_ssm" => "network-workgroup-symbolic", // AWS - workgroup
                "gcloud" | "gcp_iap" => "weather-overcast-symbolic", // GCP - cloud
                "azure" | "azure_bastion" => "weather-few-clouds-symbolic", // Azure - clouds
                "azure_ssh" => "weather-showers-symbolic",         // Azure SSH - showers
                "oci" | "oci_bastion" => "drive-harddisk-symbolic", // OCI - harddisk
                "cloudflare" | "cloudflare_access" => "security-high-symbolic", // Cloudflare
                "teleport" => "emblem-system-symbolic",            // Teleport - system/gear
                "tailscale" | "tailscale_ssh" => "network-vpn-symbolic", // Tailscale - VPN
                "boundary" => "dialog-password-symbolic",          // Boundary - password/lock
                "generic" => "system-run-symbolic",                // Generic - run command
                _ => "folder-remote-symbolic",                     // Unknown - remote folder
            };
        }

        // Standard protocol icons - each protocol has a distinct icon
        match protocol {
            "ssh" => "network-server-symbolic",
            "rdp" => "computer-symbolic",
            "vnc" => "video-display-symbolic",
            "spice" => "video-x-generic-symbolic",
            "zerotrust" => "folder-remote-symbolic",
            _ => "network-server-symbolic",
        }
    }

    /// Shows the context menu for a connection item
    fn show_context_menu(widget: &impl IsA<Widget>, x: f64, y: f64) {
        Self::show_context_menu_for_item(widget, x, y, false);
    }

    /// Shows the context menu for a connection item with group awareness
    fn show_context_menu_for_item(widget: &impl IsA<Widget>, x: f64, y: f64, is_group: bool) {
        // Get the root window to access actions
        let Some(root) = widget.root() else { return };
        let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
            return;
        };

        // Create a custom popover with buttons instead of PopoverMenu
        // This ensures actions are properly activated
        let popover = gtk4::Popover::new();

        let menu_box = GtkBox::new(Orientation::Vertical, 0);
        menu_box.set_margin_top(6);
        menu_box.set_margin_bottom(6);
        menu_box.set_margin_start(6);
        menu_box.set_margin_end(6);

        // Helper to create menu button
        let create_menu_button = |label: &str| -> Button {
            let btn = Button::with_label(label);
            btn.set_has_frame(false);
            btn.add_css_class("flat");
            btn.set_halign(gtk4::Align::Start);
            btn
        };

        let popover_ref = popover.downgrade();

        // Use lookup_action and activate on the window (which implements ActionMap)
        let window_clone = window.clone();

        if !is_group {
            let connect_btn = create_menu_button("Connect");
            let win = window_clone.clone();
            let popover_c = popover_ref.clone();
            connect_btn.connect_clicked(move |_| {
                if let Some(p) = popover_c.upgrade() {
                    p.popdown();
                }
                if let Some(action) = win.lookup_action("connect") {
                    action.activate(None);
                }
            });
            menu_box.append(&connect_btn);
        }

        let edit_btn = create_menu_button("Edit");
        let win = window_clone.clone();
        let popover_c = popover_ref.clone();
        edit_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("edit-connection") {
                action.activate(None);
            }
        });
        menu_box.append(&edit_btn);

        // View Details option (only for connections, not groups)
        if !is_group {
            let details_btn = create_menu_button("View Details");
            let win = window_clone.clone();
            let popover_c = popover_ref.clone();
            details_btn.connect_clicked(move |_| {
                if let Some(p) = popover_c.upgrade() {
                    p.popdown();
                }
                if let Some(action) = win.lookup_action("view-details") {
                    action.activate(None);
                }
            });
            menu_box.append(&details_btn);
        }

        if !is_group {
            let duplicate_btn = create_menu_button("Duplicate");
            let win = window_clone.clone();
            let popover_c = popover_ref.clone();
            duplicate_btn.connect_clicked(move |_| {
                if let Some(p) = popover_c.upgrade() {
                    p.popdown();
                }
                if let Some(action) = win.lookup_action("duplicate-connection") {
                    action.activate(None);
                }
            });
            menu_box.append(&duplicate_btn);

            let move_btn = create_menu_button("Move to Group...");
            let win = window_clone.clone();
            let popover_c = popover_ref.clone();
            move_btn.connect_clicked(move |_| {
                if let Some(p) = popover_c.upgrade() {
                    p.popdown();
                }
                if let Some(action) = win.lookup_action("move-to-group") {
                    action.activate(None);
                }
            });
            menu_box.append(&move_btn);
        }

        let delete_btn = create_menu_button("Delete");
        delete_btn.add_css_class("destructive-action");
        let win = window_clone;
        let popover_c = popover_ref;
        delete_btn.connect_clicked(move |_| {
            if let Some(p) = popover_c.upgrade() {
                p.popdown();
            }
            if let Some(action) = win.lookup_action("delete-connection") {
                action.activate(None);
            }
        });
        menu_box.append(&delete_btn);

        popover.set_child(Some(&menu_box));

        // Attach popover to the window
        popover.set_parent(window);

        // Calculate absolute position for the popover
        let widget_bounds = widget.compute_bounds(window);
        let (popup_x, popup_y) = if let Some(bounds) = widget_bounds {
            (bounds.x() as i32 + x as i32, bounds.y() as i32 + y as i32)
        } else {
            (x as i32, y as i32)
        };

        popover.set_pointing_to(Some(&gdk::Rectangle::new(popup_x, popup_y, 1, 1)));
        popover.set_autohide(true);
        popover.set_has_arrow(true);

        // Connect to closed signal to unparent the popover
        popover.connect_closed(|p| {
            p.unparent();
        });

        popover.popup();
    }

    /// Returns the main widget for this sidebar
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the search entry widget
    #[must_use]
    pub const fn search_entry(&self) -> &SearchEntry {
        &self.search_entry
    }

    /// Returns the search debouncer
    #[must_use]
    pub fn search_debouncer(&self) -> Rc<Debouncer> {
        Rc::clone(&self.search_debouncer)
    }

    /// Returns the search spinner widget
    #[must_use]
    pub const fn search_spinner(&self) -> &gtk4::Spinner {
        &self.search_spinner
    }

    /// Shows the search pending indicator
    pub fn show_search_pending(&self) {
        self.search_spinner.set_visible(true);
        self.search_spinner.start();
    }

    /// Hides the search pending indicator
    pub fn hide_search_pending(&self) {
        self.search_spinner.stop();
        self.search_spinner.set_visible(false);
    }

    /// Sets the pending search query
    pub fn set_pending_search_query(&self, query: Option<String>) {
        *self.pending_search_query.borrow_mut() = query;
    }

    /// Gets the pending search query
    #[must_use]
    pub fn pending_search_query(&self) -> Option<String> {
        self.pending_search_query.borrow().clone()
    }

    /// Returns the list view widget
    #[must_use]
    pub const fn list_view(&self) -> &ListView {
        &self.list_view
    }

    /// Returns the underlying store
    #[must_use]
    pub const fn store(&self) -> &gio::ListStore {
        &self.store
    }

    /// Returns the tree list model
    #[must_use]
    pub const fn tree_model(&self) -> &TreeListModel {
        &self.tree_model
    }

    /// Returns a reference to the lazy group loader
    #[must_use]
    pub fn lazy_loader(&self) -> std::cell::Ref<'_, LazyGroupLoader> {
        self.lazy_loader.borrow()
    }

    /// Returns a mutable reference to the lazy group loader
    pub fn lazy_loader_mut(&self) -> std::cell::RefMut<'_, LazyGroupLoader> {
        self.lazy_loader.borrow_mut()
    }

    /// Checks if a group needs to be loaded
    ///
    /// Returns true if the group's children have not been loaded yet.
    #[must_use]
    pub fn needs_group_loading(&self, group_id: Uuid) -> bool {
        self.lazy_loader.borrow().needs_loading(group_id)
    }

    /// Marks a group as loaded
    ///
    /// Call this after loading a group's children to prevent re-loading.
    pub fn mark_group_loaded(&self, group_id: Uuid) {
        self.lazy_loader.borrow_mut().mark_group_loaded(group_id);
    }

    /// Marks root items as loaded
    ///
    /// Call this after the initial sidebar population.
    pub fn mark_root_loaded(&self) {
        self.lazy_loader.borrow_mut().mark_root_loaded();
    }

    /// Checks if root items have been loaded
    #[must_use]
    pub fn is_root_loaded(&self) -> bool {
        self.lazy_loader.borrow().is_root_loaded()
    }

    /// Resets the lazy loading state
    ///
    /// Call this when the connection database is reloaded.
    pub fn reset_lazy_loading(&self) {
        self.lazy_loader.borrow_mut().reset();
    }

    // ========== Virtual Scrolling Methods ==========

    /// Initializes virtual scrolling if the item count exceeds the threshold
    ///
    /// Call this after populating the sidebar to enable virtual scrolling
    /// for large connection lists.
    #[allow(clippy::cast_lossless)]
    pub fn setup_virtual_scrolling(&self, item_count: usize) {
        if self.virtual_scroll_config.should_enable(item_count) {
            let viewport_height = f64::from(self.scrolled_window.height());
            let scroller = VirtualScroller::new(
                item_count,
                self.virtual_scroll_config.item_height,
                viewport_height,
            )
            .with_overscan(self.virtual_scroll_config.overscan);

            *self.virtual_scroller.borrow_mut() = Some(scroller);
        } else {
            *self.virtual_scroller.borrow_mut() = None;
        }
    }

    /// Updates the virtual scroller when the viewport is resized
    pub fn update_viewport_height(&self, height: f64) {
        if let Some(ref mut scroller) = *self.virtual_scroller.borrow_mut() {
            scroller.set_viewport_height(height);
        }
    }

    /// Updates the virtual scroller when scrolling occurs
    pub fn update_scroll_offset(&self, offset: f64) {
        if let Some(ref mut scroller) = *self.virtual_scroller.borrow_mut() {
            scroller.set_scroll_offset(offset);
        }
    }

    /// Returns the visible range of items for virtual scrolling
    #[must_use]
    pub fn visible_range(&self) -> Option<(usize, usize)> {
        self.virtual_scroller
            .borrow()
            .as_ref()
            .map(VirtualScroller::visible_range)
    }

    /// Returns whether virtual scrolling is currently enabled
    #[must_use]
    pub fn is_virtual_scrolling_enabled(&self) -> bool {
        self.virtual_scroller.borrow().is_some()
    }

    /// Returns a reference to the selection state for virtual scrolling
    #[must_use]
    pub fn selection_state(&self) -> std::cell::Ref<'_, CoreSelectionState> {
        self.selection_state.borrow()
    }

    /// Returns a mutable reference to the selection state
    pub fn selection_state_mut(&self) -> std::cell::RefMut<'_, CoreSelectionState> {
        self.selection_state.borrow_mut()
    }

    /// Selects an item by ID in the persistent selection state
    ///
    /// This preserves the selection across virtual scrolling operations.
    pub fn persist_selection(&self, id: Uuid) {
        self.selection_state.borrow_mut().select(id);
    }

    /// Deselects an item by ID from the persistent selection state
    pub fn unpersist_selection(&self, id: Uuid) {
        self.selection_state.borrow_mut().deselect(id);
    }

    /// Toggles selection for an item in the persistent selection state
    pub fn toggle_persisted_selection(&self, id: Uuid) {
        self.selection_state.borrow_mut().toggle(id);
    }

    /// Checks if an item is in the persistent selection state
    #[must_use]
    pub fn is_selection_persisted(&self, id: Uuid) -> bool {
        self.selection_state.borrow().is_selected(id)
    }

    /// Clears all selections
    pub fn clear_selection_state(&self) {
        self.selection_state.borrow_mut().clear();
    }

    /// Returns the count of selected items
    #[must_use]
    pub fn selection_count(&self) -> usize {
        self.selection_state.borrow().selection_count()
    }

    /// Populates the sidebar with documents and their contents
    ///
    /// This method clears the current store and repopulates it with
    /// document headers followed by their groups and connections.
    pub fn populate_with_documents(
        &self,
        documents: &[(Uuid, String, bool)], // (id, name, is_dirty)
        get_document_contents: impl Fn(
            Uuid,
        ) -> (
            Vec<(Uuid, String)>,
            Vec<(Uuid, String, String, String, Option<Uuid>)>,
        ),
        // Returns (groups, connections) where connections are (id, name, protocol, host, group_id)
    ) {
        self.store.remove_all();

        for (doc_id, doc_name, is_dirty) in documents {
            let doc_item = ConnectionItem::new_document(&doc_id.to_string(), doc_name, *is_dirty);

            let (groups, connections) = get_document_contents(*doc_id);

            // Create a map of group items for nesting connections
            let mut group_items: std::collections::HashMap<Uuid, ConnectionItem> =
                std::collections::HashMap::new();

            // Add groups to document
            for (group_id, group_name) in &groups {
                let group_item = ConnectionItem::new_group(&group_id.to_string(), group_name);
                group_items.insert(*group_id, group_item.clone());
                doc_item.add_child(&group_item);
            }

            // Add connections to their groups or directly to document
            for (conn_id, conn_name, protocol, host, group_id) in &connections {
                let conn_item =
                    ConnectionItem::new_connection(&conn_id.to_string(), conn_name, protocol, host);

                if let Some(gid) = group_id {
                    if let Some(group_item) = group_items.get(gid) {
                        group_item.add_child(&conn_item);
                    } else {
                        // Group not found, add to document root
                        doc_item.add_child(&conn_item);
                    }
                } else {
                    // No group, add to document root
                    doc_item.add_child(&conn_item);
                }
            }

            self.store.append(&doc_item);
        }
    }

    /// Updates the dirty indicator for a document in the sidebar
    pub fn update_document_dirty_state(&self, doc_id: Uuid, is_dirty: bool) {
        let n_items = self.store.n_items();
        for i in 0..n_items {
            if let Some(item) = self.store.item(i).and_downcast::<ConnectionItem>() {
                if item.is_document() && item.id() == doc_id.to_string() {
                    item.set_dirty(is_dirty);
                    // Trigger a refresh by notifying the model
                    self.store.items_changed(i, 1, 1);
                    break;
                }
            }
        }
    }

    /// Gets the document ID for a selected item
    ///
    /// Traverses up the tree to find the parent document
    pub fn get_document_for_item(&self, item_id: Uuid) -> Option<Uuid> {
        let n_items = self.store.n_items();
        for i in 0..n_items {
            if let Some(doc_item) = self.store.item(i).and_downcast::<ConnectionItem>() {
                if doc_item.is_document() {
                    // Check if this document contains the item
                    if doc_item.id() == item_id.to_string() {
                        return Uuid::parse_str(&doc_item.id()).ok();
                    }
                    // Check children
                    if let Some(children) = doc_item.children() {
                        if Self::find_item_in_children(&children, &item_id.to_string()) {
                            return Uuid::parse_str(&doc_item.id()).ok();
                        }
                    }
                }
            }
        }
        None
    }

    /// Helper to find an item in children recursively
    fn find_item_in_children(model: &gio::ListModel, item_id: &str) -> bool {
        let n_items = model.n_items();
        for i in 0..n_items {
            if let Some(item) = model.item(i).and_downcast::<ConnectionItem>() {
                if item.id() == item_id {
                    return true;
                }
                if let Some(children) = item.children() {
                    if Self::find_item_in_children(&children, item_id) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns the bulk actions bar
    /// Returns the bulk actions bar
    #[must_use]
    pub const fn bulk_actions_bar(&self) -> &GtkBox {
        &self.bulk_actions_bar
    }

    /// Returns whether group operations mode is active
    #[must_use]
    pub fn is_group_operations_mode(&self) -> bool {
        *self.group_ops_mode.borrow()
    }

    /// Toggles group operations mode
    /// Switches between `SingleSelection` and `MultiSelection` models
    pub fn set_group_operations_mode(&self, enabled: bool) {
        // Update mode flag
        *self.group_ops_mode.borrow_mut() = enabled;

        // Show/hide bulk actions toolbar
        self.bulk_actions_bar.set_visible(enabled);

        // Create new selection model
        let new_wrapper = if enabled {
            SelectionModelWrapper::new_multi(self.tree_model.clone())
        } else {
            SelectionModelWrapper::new_single(self.tree_model.clone())
        };

        // Update the list view with new selection model
        match &new_wrapper {
            SelectionModelWrapper::Single(s) => {
                self.list_view.set_model(Some(s));
            }
            SelectionModelWrapper::Multi(m) => {
                self.list_view.set_model(Some(m));
            }
        }

        // Store the new wrapper
        *self.selection_model.borrow_mut() = new_wrapper;

        // Update CSS class for visual feedback
        if enabled {
            self.list_view.add_css_class("group-operations-mode");
        } else {
            self.list_view.remove_css_class("group-operations-mode");
        }
    }

    /// Gets all selected connection/group IDs
    #[must_use]
    pub fn get_selected_ids(&self) -> Vec<Uuid> {
        let selection = self.selection_model.borrow();
        let positions = selection.get_selected_positions();

        let mut ids = Vec::new();
        for pos in positions {
            if let Some(model) = selection.model() {
                if let Some(item) = model.item(pos) {
                    // Handle TreeListRow wrapping
                    let conn_item = if let Some(row) = item.downcast_ref::<TreeListRow>() {
                        row.item().and_downcast::<ConnectionItem>()
                    } else {
                        item.downcast::<ConnectionItem>().ok()
                    };

                    if let Some(conn_item) = conn_item {
                        if let Ok(uuid) = Uuid::parse_str(&conn_item.id()) {
                            ids.push(uuid);
                        }
                    }
                }
            }
        }
        ids
    }

    /// Gets the first selected `ConnectionItem` (works in both single and multi-selection modes)
    #[must_use]
    pub fn get_selected_item(&self) -> Option<ConnectionItem> {
        let selection = self.selection_model.borrow();
        let positions = selection.get_selected_positions();

        if let Some(&pos) = positions.first() {
            if let Some(model) = selection.model() {
                if let Some(item) = model.item(pos) {
                    // Handle TreeListRow wrapping
                    return if let Some(row) = item.downcast_ref::<TreeListRow>() {
                        row.item().and_downcast::<ConnectionItem>()
                    } else {
                        item.downcast::<ConnectionItem>().ok()
                    };
                }
            }
        }
        None
    }

    /// Selects all visible items (only works in group operations mode)
    pub fn select_all(&self) {
        self.selection_model.borrow().select_all();
    }

    /// Clears all selections
    pub fn clear_selection(&self) {
        self.selection_model.borrow().clear_selection();
    }

    /// Returns the selection model wrapper
    pub fn selection_model(&self) -> Rc<RefCell<SelectionModelWrapper>> {
        self.selection_model.clone()
    }

    /// Returns the drop indicator
    #[must_use]
    pub fn drop_indicator(&self) -> Rc<DropIndicator> {
        self.drop_indicator.clone()
    }

    /// Returns the scrolled window containing the list view
    #[must_use]
    pub const fn scrolled_window(&self) -> &ScrolledWindow {
        &self.scrolled_window
    }

    /// Updates the drop indicator position based on drag coordinates
    ///
    /// This method calculates whether the drop should be before, after, or into
    /// a target item based on the Y coordinate of the drag.
    /// Uses CSS classes on row widgets for precise visual feedback.
    fn update_drop_indicator(
        drop_indicator: &DropIndicator,
        list_view: &ListView,
        _tree_model: &TreeListModel,
        x: f64,
        y: f64,
    ) -> gdk::DragAction {
        // Try to find the widget at the current position using pick()
        // This gives us the exact widget under the cursor
        let picked_widget = list_view.pick(x, y, gtk4::PickFlags::DEFAULT);

        // Find the TreeExpander ancestor of the picked widget
        let row_widget = picked_widget.and_then(|w| {
            // Walk up the widget tree to find TreeExpander
            let mut current: Option<Widget> = Some(w);
            while let Some(widget) = current {
                if widget.type_().name() == "GtkTreeExpander" {
                    return Some(widget);
                }
                // Also check for the content box inside TreeExpander
                if let Some(parent) = widget.parent() {
                    if parent.type_().name() == "GtkTreeExpander" {
                        return Some(parent);
                    }
                }
                current = widget.parent();
            }
            None
        });

        // If we couldn't find a row widget, hide the indicator
        let Some(row_widget) = row_widget else {
            drop_indicator.hide();
            return gdk::DragAction::empty();
        };

        // Get the row widget's allocation to determine position within it
        let (_, row_height) = row_widget.preferred_size();
        let row_height = f64::from(row_height.height().max(36));

        // Get the Y position relative to the row widget
        // Use compute_point for GTK4.12+ compatibility
        let point = gtk4::graphene::Point::new(x as f32, y as f32);
        let y_in_widget = list_view
            .compute_point(&row_widget, &point)
            .map_or(y, |p| f64::from(p.y()));

        // Determine drop position based on Y within the row
        // Increased ratio for easier targeting (40% top/bottom zones)
        const DROP_ZONE_RATIO: f64 = 0.4;
        let drop_zone_size = row_height * DROP_ZONE_RATIO;

        // Try to get the item to check if it's a group
        let is_group_or_document = Self::is_row_widget_group_or_document(list_view, &row_widget);

        let position = if is_group_or_document {
            // For groups/documents: top zone = before, middle = into, bottom = after
            if y_in_widget < drop_zone_size {
                DropPosition::Before
            } else if y_in_widget > row_height - drop_zone_size {
                DropPosition::After
            } else {
                DropPosition::Into
            }
        } else {
            // For connections: top half = before, bottom half = after
            if y_in_widget < row_height / 2.0 {
                DropPosition::Before
            } else {
                DropPosition::After
            }
        };

        // Update visual feedback using CSS classes
        drop_indicator.show(position, 0); // Index not used for CSS approach
        drop_indicator.set_current_widget(Some(row_widget), position);

        // Clear legacy group highlights
        Self::clear_group_highlights(list_view, drop_indicator);

        gdk::DragAction::MOVE
    }

    /// Checks if a row widget represents a group or document
    fn is_row_widget_group_or_document(_list_view: &ListView, row_widget: &Widget) -> bool {
        // Try to find the item by checking the widget's data or walking the model
        // For now, check if the widget has a folder icon
        if let Some(content_box) = row_widget.first_child() {
            if let Some(first_child) = content_box.first_child() {
                if let Some(image) = first_child.downcast_ref::<gtk4::Image>() {
                    if let Some(icon_name) = image.icon_name() {
                        return icon_name.contains("folder") || icon_name.contains("document");
                    }
                }
            }
        }
        false
    }

    /// Highlights a group row to indicate drop-into action
    /// Now handled by CSS classes on the row widget itself
    fn highlight_group_at_index(_list_view: &ListView, drop_indicator: &DropIndicator, index: u32) {
        drop_indicator.set_highlighted_group(Some(index));
    }

    /// Clears highlight from all group rows
    /// CSS classes are now managed by `DropIndicator`
    fn clear_group_highlights(_list_view: &ListView, drop_indicator: &DropIndicator) {
        drop_indicator.set_highlighted_group(None);
    }

    /// Hides the drop indicator (called on drag end or leave)
    pub fn hide_drop_indicator(&self) {
        self.drop_indicator.hide();
        self.drop_indicator.set_highlighted_group(None);
    }

    /// Creates the search help popover with syntax documentation
    fn create_search_help_popover() -> gtk4::Popover {
        let popover = gtk4::Popover::new();
        popover.set_autohide(true);

        let content = GtkBox::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Title
        let title = Label::builder()
            .label("<b>Search Syntax</b>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .build();
        content.append(&title);

        // Description
        let desc = Label::builder()
            .label("Use operators to filter connections:")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        content.append(&desc);

        // Operators list
        let operators = [
            ("protocol:ssh", "Filter by protocol (ssh, rdp, vnc, spice)"),
            ("tag:production", "Filter by tag"),
            ("group:servers", "Filter by group name"),
            ("prop:environment", "Filter by custom property"),
        ];

        let grid = gtk4::Grid::builder()
            .row_spacing(4)
            .column_spacing(12)
            .margin_top(8)
            .build();

        for (i, (operator, description)) in operators.iter().enumerate() {
            let op_label = Label::builder()
                .label(format!("<tt>{operator}</tt>"))
                .use_markup(true)
                .halign(gtk4::Align::Start)
                .build();
            let desc_label = Label::builder()
                .label(*description)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            #[allow(clippy::cast_possible_wrap)]
            {
                grid.attach(&op_label, 0, i as i32, 1, 1);
                grid.attach(&desc_label, 1, i as i32, 1, 1);
            }
        }
        content.append(&grid);

        // Examples section
        let examples_title = Label::builder()
            .label("<b>Examples</b>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .margin_top(8)
            .build();
        content.append(&examples_title);

        let examples = [
            "protocol:ssh web",
            "tag:prod server",
            "group:aws protocol:rdp",
        ];

        for example in examples {
            let example_label = Label::builder()
                .label(format!("<tt>{example}</tt>"))
                .use_markup(true)
                .halign(gtk4::Align::Start)
                .margin_start(8)
                .build();
            content.append(&example_label);
        }

        popover.set_child(Some(&content));
        popover
    }

    /// Sets up search entry hints for operator autocomplete and history navigation
    #[allow(clippy::needless_pass_by_value)]
    fn setup_search_entry_hints(
        search_entry: &SearchEntry,
        _search_entry_clone: &SearchEntry,
        history_popover: &gtk4::Popover,
        _search_history: &Rc<RefCell<Vec<String>>>,
    ) {
        // Show history on down arrow when empty
        let history_popover_clone = history_popover.clone();
        let key_controller = EventControllerKey::new();
        let search_entry_clone = search_entry.clone();
        key_controller.connect_key_pressed(move |_controller, key, _code, _state| {
            if key == gdk::Key::Down && search_entry_clone.text().is_empty() {
                history_popover_clone.popup();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        search_entry.add_controller(key_controller);
    }

    /// Creates the search history popover
    fn create_history_popover(
        search_entry: &SearchEntry,
        search_history: Rc<RefCell<Vec<String>>>,
    ) -> gtk4::Popover {
        let popover = gtk4::Popover::new();
        popover.set_autohide(true);

        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(8);
        content.set_margin_end(8);

        // Title
        let title = Label::builder()
            .label("<b>Recent Searches</b>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .build();
        content.append(&title);

        // History list container
        let history_list = GtkBox::new(Orientation::Vertical, 2);
        history_list.set_margin_top(4);
        content.append(&history_list);

        // Update history list when popover is shown
        let search_entry_clone = search_entry.clone();
        let history_list_clone = history_list;
        let search_history_clone = search_history;
        let popover_clone = popover.clone();
        popover.connect_show(move |_| {
            // Clear existing items
            while let Some(child) = history_list_clone.first_child() {
                history_list_clone.remove(&child);
            }

            // Add history items
            let history = search_history_clone.borrow();
            if history.is_empty() {
                let empty_label = Label::builder()
                    .label("No recent searches")
                    .css_classes(["dim-label"])
                    .build();
                history_list_clone.append(&empty_label);
            } else {
                for query in history.iter() {
                    let button = Button::builder()
                        .label(query)
                        .css_classes(["flat"])
                        .halign(gtk4::Align::Start)
                        .build();

                    let search_entry_for_btn = search_entry_clone.clone();
                    let query_clone = query.clone();
                    let popover_for_btn = popover_clone.clone();
                    button.connect_clicked(move |_| {
                        search_entry_for_btn.set_text(&query_clone);
                        popover_for_btn.popdown();
                    });

                    history_list_clone.append(&button);
                }
            }
        });

        popover.set_child(Some(&content));
        popover
    }

    /// Adds a search query to the history
    pub fn add_to_search_history(&self, query: &str) {
        if query.trim().is_empty() {
            return;
        }

        let mut history = self.search_history.borrow_mut();

        // Remove if already exists (to move to front)
        history.retain(|q| q != query);

        // Add to front
        history.insert(0, query.to_string());

        // Trim to max size
        history.truncate(MAX_SEARCH_HISTORY);
    }

    /// Gets the search history
    #[must_use]
    pub fn get_search_history(&self) -> Vec<String> {
        self.search_history.borrow().clone()
    }

    /// Clears the search history
    pub fn clear_search_history(&self) {
        self.search_history.borrow_mut().clear();
    }

    /// Saves the current tree state before starting a search
    /// Call this when the user starts typing in the search box
    pub fn save_pre_search_state(&self) {
        // Only save if we don't already have a saved state (first search keystroke)
        if self.pre_search_state.borrow().is_none() {
            *self.pre_search_state.borrow_mut() = Some(self.save_state());
        }
    }

    /// Restores the tree state saved before search and clears the saved state
    /// Call this when the search box is cleared
    pub fn restore_pre_search_state(&self) {
        if let Some(state) = self.pre_search_state.borrow_mut().take() {
            self.restore_state(&state);
        }
    }

    /// Checks if there is a saved pre-search state
    #[must_use]
    pub fn has_pre_search_state(&self) -> bool {
        self.pre_search_state.borrow().is_some()
    }

    /// Gets the IDs of all collapsed groups in the tree
    /// Returns a `HashSet` of group UUIDs that are currently collapsed
    /// Note: This iterates through all visible rows in the `TreeListModel`
    #[must_use]
    pub fn get_collapsed_groups(&self) -> std::collections::HashSet<Uuid> {
        let mut collapsed = std::collections::HashSet::new();
        self.collect_collapsed_groups_recursive(&mut collapsed);
        collapsed
    }

    /// Recursively collects collapsed group IDs from the tree model
    fn collect_collapsed_groups_recursive(&self, collapsed: &mut std::collections::HashSet<Uuid>) {
        let n_items = self.tree_model.n_items();

        for i in 0..n_items {
            if let Some(row) = self
                .tree_model
                .item(i)
                .and_then(|o| o.downcast::<TreeListRow>().ok())
            {
                if let Some(item) = row.item().and_then(|o| o.downcast::<ConnectionItem>().ok()) {
                    if item.is_group() {
                        // Check if this row is expandable and NOT expanded
                        if row.is_expandable() && !row.is_expanded() {
                            if let Ok(id) = Uuid::parse_str(&item.id()) {
                                collapsed.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Applies collapsed state to groups after populating the sidebar
    /// Groups in the provided set will be collapsed, others will be expanded
    /// This needs to be called after the tree is populated and uses `idle_add`
    /// to ensure the tree model is fully initialized
    pub fn apply_collapsed_groups(&self, collapsed: &std::collections::HashSet<Uuid>) {
        if collapsed.is_empty() {
            return;
        }

        let tree_model = self.tree_model.clone();
        let collapsed = collapsed.clone();

        // Use idle_add to ensure tree model is ready
        glib::idle_add_local_once(move || {
            Self::apply_collapsed_state(&tree_model, &collapsed);
        });
    }

    /// Internal function to apply collapsed state
    fn apply_collapsed_state(
        tree_model: &TreeListModel,
        collapsed: &std::collections::HashSet<Uuid>,
    ) {
        let n_items = tree_model.n_items();

        for i in 0..n_items {
            if let Some(row) = tree_model
                .item(i)
                .and_then(|o| o.downcast::<TreeListRow>().ok())
            {
                if row.is_expandable() {
                    if let Some(item) = row.item().and_then(|o| o.downcast::<ConnectionItem>().ok())
                    {
                        if item.is_group() {
                            if let Ok(id) = Uuid::parse_str(&item.id()) {
                                // Collapse if in collapsed set, expand otherwise
                                row.set_expanded(!collapsed.contains(&id));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Saves the current tree state for later restoration
    ///
    /// Captures expanded groups, scroll position, and selected item.
    /// Use this before refresh operations to preserve user's view.
    #[must_use]
    pub fn save_state(&self) -> TreeState {
        // Collect expanded groups (inverse of collapsed)
        let expanded_groups = self.get_expanded_groups();

        // Save scroll position from the scrolled window's vertical adjustment
        let adj = self.scrolled_window.vadjustment();
        let scroll_position = adj.value();

        // Save selected item ID
        let selected_id = self
            .get_selected_item()
            .and_then(|item| Uuid::parse_str(&item.id()).ok());

        TreeState {
            expanded_groups,
            scroll_position,
            selected_id,
        }
    }

    /// Restores tree state after a refresh operation
    ///
    /// Expands the previously expanded groups, restores scroll position,
    /// and re-selects the previously selected item.
    pub fn restore_state(&self, state: &TreeState) {
        // Restore expanded groups
        self.apply_expanded_groups(&state.expanded_groups);

        // Restore scroll position using idle_add to ensure tree is ready
        let scroll_position = state.scroll_position;
        let scrolled_window = self.scrolled_window.clone();
        glib::idle_add_local_once(move || {
            let adj = scrolled_window.vadjustment();
            adj.set_value(scroll_position);
        });

        // Restore selection
        if let Some(selected_id) = state.selected_id {
            self.select_item_by_id(selected_id);
        }
    }

    /// Gets the IDs of all expanded groups in the tree
    /// Returns a `HashSet` of group UUIDs that are currently expanded
    #[must_use]
    pub fn get_expanded_groups(&self) -> HashSet<Uuid> {
        let mut expanded = HashSet::new();
        let n_items = self.tree_model.n_items();

        for i in 0..n_items {
            if let Some(row) = self
                .tree_model
                .item(i)
                .and_then(|o| o.downcast::<TreeListRow>().ok())
            {
                if let Some(item) = row.item().and_then(|o| o.downcast::<ConnectionItem>().ok()) {
                    // Include both groups and documents that are expanded
                    if (item.is_group() || item.is_document()) && row.is_expanded() {
                        if let Ok(id) = Uuid::parse_str(&item.id()) {
                            expanded.insert(id);
                        }
                    }
                }
            }
        }

        expanded
    }

    /// Applies expanded state to groups after populating the sidebar
    /// Groups in the provided set will be expanded, others will be collapsed
    pub fn apply_expanded_groups(&self, expanded: &HashSet<Uuid>) {
        let tree_model = self.tree_model.clone();
        let expanded = expanded.clone();

        // Use idle_add to ensure tree model is ready
        glib::idle_add_local_once(move || {
            let n_items = tree_model.n_items();

            for i in 0..n_items {
                if let Some(row) = tree_model
                    .item(i)
                    .and_then(|o| o.downcast::<TreeListRow>().ok())
                {
                    if row.is_expandable() {
                        if let Some(item) =
                            row.item().and_then(|o| o.downcast::<ConnectionItem>().ok())
                        {
                            if item.is_group() || item.is_document() {
                                if let Ok(id) = Uuid::parse_str(&item.id()) {
                                    // Expand if in expanded set
                                    row.set_expanded(expanded.contains(&id));
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    /// Selects an item by its UUID
    ///
    /// Searches through the tree model to find and select the item with the given ID.
    pub fn select_item_by_id(&self, item_id: Uuid) {
        let tree_model = self.tree_model.clone();
        let selection_model = self.selection_model.clone();
        let item_id_str = item_id.to_string();

        // Use idle_add to ensure tree model is ready
        glib::idle_add_local_once(move || {
            let n_items = tree_model.n_items();

            for i in 0..n_items {
                if let Some(row) = tree_model
                    .item(i)
                    .and_then(|o| o.downcast::<TreeListRow>().ok())
                {
                    if let Some(item) = row.item().and_then(|o| o.downcast::<ConnectionItem>().ok())
                    {
                        if item.id() == item_id_str {
                            // Found the item, select it
                            let sel = selection_model.borrow();
                            match &*sel {
                                SelectionModelWrapper::Single(s) => {
                                    s.set_selected(i);
                                }
                                SelectionModelWrapper::Multi(m) => {
                                    m.unselect_all();
                                    m.select_item(i, false);
                                }
                            }
                            return;
                        }
                    }
                }
            }
        });
    }

    /// Refreshes the tree while preserving the current state
    ///
    /// This is a convenience method that saves the current state,
    /// calls the provided refresh function, and then restores the state.
    /// Use this when you need to refresh the tree contents but want to
    /// maintain the user's expanded groups, scroll position, and selection.
    ///
    /// # Arguments
    /// * `refresh_fn` - A closure that performs the actual refresh operation
    ///
    /// # Example
    /// ```ignore
    /// sidebar.refresh_preserving_state(|| {
    ///     sidebar.populate_with_documents(&documents, get_contents);
    /// });
    /// ```
    pub fn refresh_preserving_state<F>(&self, refresh_fn: F)
    where
        F: FnOnce(),
    {
        // Save current state before refresh
        let state = self.save_state();

        // Perform the refresh
        refresh_fn();

        // Restore state after refresh
        self.restore_state(&state);
    }
}

impl Default for ConnectionSidebar {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Connection Item GObject wrapper
// ============================================================================

mod imp {
    use super::{gio, glib};
    use glib::prelude::*;
    use glib::subclass::prelude::*;
    use glib::Properties;
    use std::cell::RefCell;

    #[derive(Default, Properties)]
    #[properties(wrapper_type = super::ConnectionItem)]
    pub struct ConnectionItem {
        #[property(get, set)]
        id: RefCell<String>,
        #[property(get, set)]
        name: RefCell<String>,
        #[property(get, set)]
        protocol: RefCell<String>,
        #[property(get, set)]
        is_group: RefCell<bool>,
        #[property(get, set)]
        is_document: RefCell<bool>,
        #[property(get, set)]
        is_dirty: RefCell<bool>,
        #[property(get, set)]
        host: RefCell<String>,
        pub(super) children: RefCell<Option<gio::ListStore>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConnectionItem {
        const NAME: &'static str = "RustConnConnectionItem";
        type Type = super::ConnectionItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ConnectionItem {}
}

glib::wrapper! {
    /// A GObject wrapper for connection/group items in the tree view
    pub struct ConnectionItem(ObjectSubclass<imp::ConnectionItem>);
}

impl ConnectionItem {
    /// Creates a new connection item
    #[must_use]
    pub fn new_connection(id: &str, name: &str, protocol: &str, host: &str) -> Self {
        glib::Object::builder()
            .property("id", id)
            .property("name", name)
            .property("protocol", protocol)
            .property("is-group", false)
            .property("is-document", false)
            .property("is-dirty", false)
            .property("host", host)
            .build()
    }

    /// Creates a new group item
    #[must_use]
    pub fn new_group(id: &str, name: &str) -> Self {
        let item: Self = glib::Object::builder()
            .property("id", id)
            .property("name", name)
            .property("protocol", "")
            .property("is-group", true)
            .property("is-document", false)
            .property("is-dirty", false)
            .property("host", "")
            .build();

        // Initialize children store for groups
        *item.imp().children.borrow_mut() = Some(gio::ListStore::new::<Self>());

        item
    }

    /// Creates a new document item
    #[must_use]
    pub fn new_document(id: &str, name: &str, is_dirty: bool) -> Self {
        let item: Self = glib::Object::builder()
            .property("id", id)
            .property("name", name)
            .property("protocol", "")
            .property("is-group", false)
            .property("is-document", true)
            .property("is-dirty", is_dirty)
            .property("host", "")
            .build();

        // Initialize children store for documents (they contain groups and connections)
        *item.imp().children.borrow_mut() = Some(gio::ListStore::new::<Self>());

        item
    }

    /// Returns the children list store for groups/documents
    pub fn children(&self) -> Option<gio::ListModel> {
        self.imp()
            .children
            .borrow()
            .as_ref()
            .map(|store| store.clone().upcast())
    }

    /// Adds a child item to this group/document
    pub fn add_child(&self, child: &Self) {
        if let Some(ref store) = *self.imp().children.borrow() {
            store.append(child);
        }
    }

    /// Sets the dirty flag for this item
    pub fn set_dirty(&self, dirty: bool) {
        self.set_is_dirty(dirty);
    }
}

impl Default for ConnectionItem {
    fn default() -> Self {
        Self::new_connection("", "Unnamed", "ssh", "")
    }
}
