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

// Re-export types for external use
pub use crate::sidebar_types::{
    DragDropData, DropIndicator, DropPosition, SelectionModelWrapper, SessionStatusInfo, TreeState,
    MAX_SEARCH_HISTORY,
};
use crate::sidebar_ui;

use gtk4::prelude::*;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use gtk4::{
    gdk, gio, glib, Box as GtkBox, Button, DragSource, DropTarget, EventControllerKey,
    GestureClick, Label, ListItem, ListView, MultiSelection, Orientation, PolicyType,
    ScrolledWindow, SearchEntry, SignalListItemFactory, SingleSelection, TreeExpander,
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

/// Sidebar widget for connection tree display
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle
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
    /// Map of connection IDs to their session status info
    /// Tracks status and active session count for proper multi-session handling
    connection_statuses: Rc<RefCell<std::collections::HashMap<String, SessionStatusInfo>>>,
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
        let help_popover_for_key = help_popover.clone();
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
        // autoexpand=false so we can control which groups are expanded via saved state
        let tree_model = TreeListModel::new(store.clone(), false, false, |item| {
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

        // Map to store signal handlers: ListItem -> SignalHandlerId
        let signal_handlers: Rc<
            RefCell<std::collections::HashMap<ListItem, glib::SignalHandlerId>>,
        > = Rc::new(RefCell::new(std::collections::HashMap::new()));
        let signal_handlers_bind = signal_handlers.clone();
        let signal_handlers_unbind = signal_handlers.clone();

        factory.connect_setup(move |factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::setup_list_item(factory, list_item, *group_ops_mode_clone.borrow());
            }
        });
        factory.connect_bind(move |factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::bind_list_item(factory, list_item, &signal_handlers_bind);
            }
        });
        factory.connect_unbind(move |factory, obj| {
            if let Some(list_item) = obj.downcast_ref::<ListItem>() {
                Self::unbind_list_item(factory, list_item, &signal_handlers_unbind);
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

        // Handle drop on the list view
        let drop_indicator_drop = drop_indicator.clone();
        list_view_drop_target.connect_drop(move |target, value, _x, _y| {
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

            // Get target info from indicator state
            let position = match drop_indicator_drop.position() {
                Some(p) => p,
                None => return false,
            };

            let target_widget = match drop_indicator_drop.current_widget() {
                Some(w) => w,
                None => return false,
            };

            let target_item = match Self::get_item_from_widget(&target_widget) {
                Some(item) => item,
                None => return false,
            };

            let target_id = target_item.id();
            let target_is_group = target_item.is_group();

            // Don't allow dropping on self
            if item_id == target_id {
                return false;
            }

            // Determine effective target type based on drop position
            // If dropping INTO a group, target is the group
            // If dropping BEFORE/AFTER, target is a sibling (so treat as non-group for placement)
            let effective_is_group = match position {
                DropPosition::Into => target_is_group,
                _ => false,
            };

            // Activate the drag-drop action with the data
            // Format: "item_type:item_id:target_id:target_is_group"
            let action_data = format!("{item_type}:{item_id}:{target_id}:{effective_is_group}");

            if let Some(widget) = target.widget() {
                // Hide drop indicator before processing the drop
                let _ = widget.activate_action("win.hide-drop-indicator", None);
                let _ =
                    widget.activate_action("win.drag-drop-item", Some(&action_data.to_variant()));
            }

            true
        });

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
            connection_statuses: Rc::new(RefCell::new(std::collections::HashMap::new())),
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
    ///
    /// Note: Part of drag-drop callback API for external handlers.
    #[allow(dead_code)]
    pub fn set_drag_drop_callback<F>(&self, callback: F)
    where
        F: Fn(DragDropData) + 'static,
    {
        *self.drag_drop_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Invokes the drag-drop callback if set
    ///
    /// Note: Part of drag-drop callback API for external handlers.
    #[allow(dead_code)]
    pub fn invoke_drag_drop(&self, data: DragDropData) {
        if let Some(ref callback) = *self.drag_drop_callback.borrow() {
            callback(data);
        }
    }

    /// Creates the bulk actions toolbar for group operations mode
    fn create_bulk_actions_bar() -> GtkBox {
        sidebar_ui::create_bulk_actions_bar()
    }

    /// Creates the button box at the bottom of the sidebar
    fn create_button_box() -> GtkBox {
        sidebar_ui::create_button_box()
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

        let status_icon = gtk4::Image::from_icon_name("emblem-default-symbolic");
        status_icon.set_pixel_size(10);
        status_icon.set_visible(false);
        status_icon.add_css_class("status-icon");
        content_box.append(&status_icon);

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
            let bytes = glib::Bytes::from(drag_data.as_bytes());

            Some(gdk::ContentProvider::for_bytes("text/plain", &bytes))
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
    fn bind_list_item(
        _factory: &SignalListItemFactory,
        list_item: &ListItem,
        handlers: &Rc<RefCell<std::collections::HashMap<ListItem, glib::SignalHandlerId>>>,
    ) {
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

        // Update status icon
        if let Some(status_icon) = content_box
            .first_child()
            .and_then(|c| c.next_sibling())
            .and_downcast::<gtk4::Image>()
        {
            // Helper to update icon state
            let update_icon = |icon: &gtk4::Image, status: &str| {
                icon.remove_css_class("status-connected");
                icon.remove_css_class("status-connecting");
                icon.remove_css_class("status-failed");

                if status == "connected" {
                    icon.set_icon_name(Some("emblem-default-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-connected");
                } else if status == "connecting" {
                    icon.set_icon_name(Some("network-transmit-receive-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-connecting");
                } else if status == "failed" {
                    icon.set_icon_name(Some("dialog-error-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-failed");
                } else {
                    icon.set_visible(false);
                }
            };

            // Initial update
            update_icon(&status_icon, &item.status());

            // Connect to notify::status
            let status_icon_clone = status_icon.clone();
            let handler_id = item.connect_notify_local(Some("status"), move |item, _| {
                update_icon(&status_icon_clone, &item.status());
            });

            // Store handler ID on list_item for cleanup
            handlers.borrow_mut().insert(list_item.clone(), handler_id);
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

    /// Unbinds data from a list item
    fn unbind_list_item(
        _factory: &SignalListItemFactory,
        list_item: &ListItem,
        handlers: &Rc<RefCell<std::collections::HashMap<ListItem, glib::SignalHandlerId>>>,
    ) {
        let Some(row) = list_item.item().and_downcast::<TreeListRow>() else {
            return;
        };
        let Some(item) = row.item().and_downcast::<ConnectionItem>() else {
            return;
        };

        // Retrieve and disconnect handler
        if let Some(handler_id) = handlers.borrow_mut().remove(list_item) {
            item.disconnect(handler_id);
        }
    }

    /// Returns the appropriate icon name for a protocol string
    ///
    /// For ZeroTrust connections, the protocol string may include provider info
    /// in the format "zerotrust:provider" (e.g., "zerotrust:aws", "zerotrust:gcloud").
    /// This allows showing provider-specific icons for cloud CLI connections.
    fn get_protocol_icon(protocol: &str) -> &'static str {
        sidebar_ui::get_protocol_icon(protocol)
    }

    /// Shows the context menu for a connection item
    ///
    /// Note: Context menu shown via `show_context_menu_for_item` with group awareness.
    #[allow(dead_code)]
    fn show_context_menu(widget: &impl IsA<Widget>, x: f64, y: f64) {
        sidebar_ui::show_context_menu(widget, x, y);
    }

    /// Shows the context menu for a connection item with group awareness
    fn show_context_menu_for_item(widget: &impl IsA<Widget>, x: f64, y: f64, is_group: bool) {
        sidebar_ui::show_context_menu_for_item(widget, x, y, is_group);
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
    ///
    /// Note: Spinner is typically accessed via `show_search_pending`/`hide_search_pending`.
    #[must_use]
    #[allow(dead_code)]
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
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[must_use]
    #[allow(dead_code)]
    pub fn lazy_loader(&self) -> std::cell::Ref<'_, LazyGroupLoader> {
        self.lazy_loader.borrow()
    }

    /// Returns a mutable reference to the lazy group loader
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[allow(dead_code)]
    pub fn lazy_loader_mut(&self) -> std::cell::RefMut<'_, LazyGroupLoader> {
        self.lazy_loader.borrow_mut()
    }

    /// Checks if a group needs to be loaded
    ///
    /// Returns true if the group's children have not been loaded yet.
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[must_use]
    #[allow(dead_code)]
    pub fn needs_group_loading(&self, group_id: Uuid) -> bool {
        self.lazy_loader.borrow().needs_loading(group_id)
    }

    /// Marks a group as loaded
    ///
    /// Call this after loading a group's children to prevent re-loading.
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[allow(dead_code)]
    pub fn mark_group_loaded(&self, group_id: Uuid) {
        self.lazy_loader.borrow_mut().mark_group_loaded(group_id);
    }

    /// Marks root items as loaded
    ///
    /// Call this after the initial sidebar population.
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[allow(dead_code)]
    pub fn mark_root_loaded(&self) {
        self.lazy_loader.borrow_mut().mark_root_loaded();
    }

    /// Checks if root items have been loaded
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_root_loaded(&self) -> bool {
        self.lazy_loader.borrow().is_root_loaded()
    }

    /// Resets the lazy loading state
    ///
    /// Call this when the connection database is reloaded.
    ///
    /// Note: Part of lazy loading API for large connection lists.
    #[allow(dead_code)]
    pub fn reset_lazy_loading(&self) {
        self.lazy_loader.borrow_mut().reset();
    }

    // ========== Virtual Scrolling Methods ==========

    /// Initializes virtual scrolling if the item count exceeds the threshold
    ///
    /// Call this after populating the sidebar to enable virtual scrolling
    /// for large connection lists.
    ///
    /// Note: Part of virtual scrolling API for performance optimization.
    #[allow(clippy::cast_lossless)]
    #[allow(dead_code)]
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
    ///
    /// Note: Part of virtual scrolling API for performance optimization.
    #[allow(dead_code)]
    pub fn update_viewport_height(&self, height: f64) {
        if let Some(ref mut scroller) = *self.virtual_scroller.borrow_mut() {
            scroller.set_viewport_height(height);
        }
    }

    /// Updates the virtual scroller when scrolling occurs
    ///
    /// Note: Part of virtual scrolling API for performance optimization.
    #[allow(dead_code)]
    pub fn update_scroll_offset(&self, offset: f64) {
        if let Some(ref mut scroller) = *self.virtual_scroller.borrow_mut() {
            scroller.set_scroll_offset(offset);
        }
    }

    /// Returns the visible range of items for virtual scrolling
    ///
    /// Note: Part of virtual scrolling API for performance optimization.
    #[must_use]
    #[allow(dead_code)]
    pub fn visible_range(&self) -> Option<(usize, usize)> {
        self.virtual_scroller
            .borrow()
            .as_ref()
            .map(VirtualScroller::visible_range)
    }

    /// Returns whether virtual scrolling is currently enabled
    ///
    /// Note: Part of virtual scrolling API for performance optimization.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_virtual_scrolling_enabled(&self) -> bool {
        self.virtual_scroller.borrow().is_some()
    }

    /// Returns a reference to the selection state for virtual scrolling
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[must_use]
    #[allow(dead_code)]
    pub fn selection_state(&self) -> std::cell::Ref<'_, CoreSelectionState> {
        self.selection_state.borrow()
    }

    /// Returns a mutable reference to the selection state
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[allow(dead_code)]
    pub fn selection_state_mut(&self) -> std::cell::RefMut<'_, CoreSelectionState> {
        self.selection_state.borrow_mut()
    }

    /// Selects an item by ID in the persistent selection state
    ///
    /// This preserves the selection across virtual scrolling operations.
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[allow(dead_code)]
    pub fn persist_selection(&self, id: Uuid) {
        self.selection_state.borrow_mut().select(id);
    }

    /// Deselects an item by ID from the persistent selection state
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[allow(dead_code)]
    pub fn unpersist_selection(&self, id: Uuid) {
        self.selection_state.borrow_mut().deselect(id);
    }

    /// Toggles selection for an item in the persistent selection state
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[allow(dead_code)]
    pub fn toggle_persisted_selection(&self, id: Uuid) {
        self.selection_state.borrow_mut().toggle(id);
    }

    /// Checks if an item is in the persistent selection state
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_selection_persisted(&self, id: Uuid) -> bool {
        self.selection_state.borrow().is_selected(id)
    }

    /// Clears all selections
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[allow(dead_code)]
    pub fn clear_selection_state(&self) {
        self.selection_state.borrow_mut().clear();
    }

    /// Returns the count of selected items
    ///
    /// Note: Part of selection state API for virtual scrolling.
    #[must_use]
    #[allow(dead_code)]
    pub fn selection_count(&self) -> usize {
        self.selection_state.borrow().selection_count()
    }

    /// Populates the sidebar with documents and their contents
    ///
    /// This method clears the current store and repopulates it with
    /// document headers followed by their groups and connections.
    ///
    /// Note: Part of document-based sidebar API.
    #[allow(dead_code)]
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
                // Check if we have a stored status for this connection
                let status = self
                    .connection_statuses
                    .borrow()
                    .get(&conn_id.to_string())
                    .map(|info| info.status.clone())
                    .unwrap_or_else(|| "disconnected".to_string());

                let conn_item = ConnectionItem::new_connection_with_status(
                    &conn_id.to_string(),
                    conn_name,
                    protocol,
                    host,
                    &status,
                );

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
    ///
    /// Note: Part of document-based sidebar API.
    #[allow(dead_code)]
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

    /// Updates the status of a connection item
    ///
    /// This method updates the visual status in the sidebar tree.
    /// For proper session counting, use `increment_session_count` when opening
    /// a session and `decrement_session_count` when closing.
    pub fn update_connection_status(&self, id: &str, status: &str) {
        // Update the status in the map
        {
            let mut statuses = self.connection_statuses.borrow_mut();
            if let Some(info) = statuses.get_mut(id) {
                info.status = status.to_string();
            } else {
                statuses.insert(
                    id.to_string(),
                    SessionStatusInfo {
                        status: status.to_string(),
                        active_count: 0,
                    },
                );
            }
        }

        // Update the visual status in the tree
        Self::update_item_status_recursive(self.store.upcast_ref::<gio::ListModel>(), id, status);
    }

    /// Increments the session count for a connection and sets status to connected
    ///
    /// Call this when opening a new session for a connection.
    pub fn increment_session_count(&self, id: &str) {
        let status = {
            let mut statuses = self.connection_statuses.borrow_mut();
            let info = statuses.entry(id.to_string()).or_default();
            info.active_count += 1;
            info.status = "connected".to_string();
            info.status.clone()
        };

        Self::update_item_status_recursive(self.store.upcast_ref::<gio::ListModel>(), id, &status);
    }

    /// Decrements the session count for a connection
    ///
    /// Call this when closing a session. Status changes to disconnected only
    /// when the last session is closed (active_count reaches 0).
    ///
    /// Returns the new status after decrement.
    pub fn decrement_session_count(&self, id: &str, failed: bool) -> String {
        let status = {
            let mut statuses = self.connection_statuses.borrow_mut();
            if let Some(info) = statuses.get_mut(id) {
                info.active_count = info.active_count.saturating_sub(1);
                if info.active_count == 0 {
                    info.status = if failed {
                        "failed".to_string()
                    } else {
                        "disconnected".to_string()
                    };
                }
                // If still has active sessions, keep "connected" status
                info.status.clone()
            } else {
                "disconnected".to_string()
            }
        };

        Self::update_item_status_recursive(self.store.upcast_ref::<gio::ListModel>(), id, &status);
        status
    }

    /// Gets the active session count for a connection
    ///
    /// Note: Part of session status tracking API.
    #[must_use]
    #[allow(dead_code)]
    pub fn get_session_count(&self, id: &str) -> usize {
        self.connection_statuses
            .borrow()
            .get(id)
            .map_or(0, |info| info.active_count)
    }

    /// Helper to recursively find and update item status in the tree
    fn update_item_status_recursive(model: &gio::ListModel, id: &str, status: &str) -> bool {
        let n_items = model.n_items();
        for i in 0..n_items {
            if let Some(item) = model.item(i).and_downcast::<ConnectionItem>() {
                if item.id() == id {
                    item.set_status(status);
                    return true;
                }

                // Check children if it's a group or document
                if item.is_group() || item.is_document() {
                    if let Some(children) = item.children() {
                        if Self::update_item_status_recursive(&children, id, status) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Gets the status of a connection item
    pub fn get_connection_status(&self, id: &str) -> Option<String> {
        self.connection_statuses
            .borrow()
            .get(id)
            .map(|info| info.status.clone())
    }

    /// Gets the document ID for a selected item
    ///
    /// Traverses up the tree to find the parent document
    ///
    /// Note: Part of document-based sidebar API.
    #[allow(dead_code)]
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
    ///
    /// Note: Part of document-based sidebar API.
    #[allow(dead_code)]
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
    ///
    /// Note: Part of group operations mode API.
    #[must_use]
    #[allow(dead_code)]
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
    ///
    /// Note: Part of selection model API.
    #[allow(dead_code)]
    pub fn selection_model(&self) -> Rc<RefCell<SelectionModelWrapper>> {
        self.selection_model.clone()
    }

    /// Returns the drop indicator
    ///
    /// Note: Part of drag-drop API.
    #[must_use]
    #[allow(dead_code)]
    pub fn drop_indicator(&self) -> Rc<DropIndicator> {
        self.drop_indicator.clone()
    }

    /// Returns the scrolled window containing the list view
    ///
    /// Note: Part of scroll management API.
    #[must_use]
    #[allow(dead_code)]
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
            .map(|p| f64::from(p.y()))
            .unwrap_or(y);

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
        if let Some(item) = Self::get_item_from_widget(row_widget) {
            return item.is_group();
        }
        false
    }

    /// Helper to get ConnectionItem from a widget in the list view
    fn get_item_from_widget(widget: &Widget) -> Option<ConnectionItem> {
        // Walk up to find TreeExpander
        let mut current = Some(widget.clone());
        while let Some(w) = current {
            if let Some(expander) = w.downcast_ref::<TreeExpander>() {
                if let Some(row) = expander.list_row() {
                    return row.item().and_then(|i| i.downcast::<ConnectionItem>().ok());
                }
            }
            current = w.parent();
        }
        None
    }

    /// Highlights a group row to indicate drop-into action
    /// Now handled by CSS classes on the row widget itself
    ///
    /// Note: Part of drag-drop visual feedback API.
    #[allow(dead_code)]
    fn highlight_group_at_index(_list_view: &ListView, drop_indicator: &DropIndicator, index: u32) {
        drop_indicator.set_highlighted_group(Some(index));
    }

    /// Clears highlight from all group rows
    /// CSS classes are now managed by DropIndicator
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
        sidebar_ui::create_search_help_popover()
    }

    /// Sets up search entry hints for operator autocomplete and history navigation
    #[allow(clippy::needless_pass_by_value)]
    fn setup_search_entry_hints(
        search_entry: &SearchEntry,
        search_entry_clone: &SearchEntry,
        history_popover: &gtk4::Popover,
        search_history: &Rc<RefCell<Vec<String>>>,
    ) {
        sidebar_ui::setup_search_entry_hints(
            search_entry,
            search_entry_clone,
            history_popover,
            search_history,
        );
    }

    /// Creates the search history popover
    fn create_history_popover(
        search_entry: &SearchEntry,
        search_history: Rc<RefCell<Vec<String>>>,
    ) -> gtk4::Popover {
        sidebar_ui::create_history_popover(search_entry, search_history)
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
    ///
    /// Note: Part of search history API.
    #[must_use]
    #[allow(dead_code)]
    pub fn get_search_history(&self) -> Vec<String> {
        self.search_history.borrow().clone()
    }

    /// Clears the search history
    ///
    /// Note: Part of search history API.
    #[allow(dead_code)]
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
    ///
    /// Note: Part of search state preservation API.
    #[must_use]
    #[allow(dead_code)]
    pub fn has_pre_search_state(&self) -> bool {
        self.pre_search_state.borrow().is_some()
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
    /// Returns a HashSet of group UUIDs that are currently expanded
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
    /// Groups in the provided set will be expanded, others will remain collapsed
    /// This method handles nested groups by expanding from root to leaves
    pub fn apply_expanded_groups(&self, expanded: &HashSet<Uuid>) {
        if expanded.is_empty() {
            return;
        }

        let tree_model = self.tree_model.clone();
        let expanded = expanded.clone();

        // Use idle_add to ensure tree model is ready
        // We need multiple passes because expanding a group reveals its children
        glib::idle_add_local_once(move || {
            Self::apply_expanded_state_recursive(&tree_model, &expanded);
        });
    }

    /// Recursively applies expanded state to the tree
    /// Makes multiple passes to handle nested groups
    fn apply_expanded_state_recursive(tree_model: &TreeListModel, expanded: &HashSet<Uuid>) {
        // We need multiple passes because expanding a parent reveals children
        // Maximum depth to prevent infinite loops
        const MAX_PASSES: usize = 10;

        for _ in 0..MAX_PASSES {
            let mut expanded_any = false;
            let n_items = tree_model.n_items();

            for i in 0..n_items {
                if let Some(row) = tree_model
                    .item(i)
                    .and_then(|o| o.downcast::<TreeListRow>().ok())
                {
                    if row.is_expandable() && !row.is_expanded() {
                        if let Some(item) =
                            row.item().and_then(|o| o.downcast::<ConnectionItem>().ok())
                        {
                            if item.is_group() || item.is_document() {
                                if let Ok(id) = Uuid::parse_str(&item.id()) {
                                    if expanded.contains(&id) {
                                        row.set_expanded(true);
                                        expanded_any = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // If we didn't expand anything in this pass, we're done
            if !expanded_any {
                break;
            }
        }
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
    ///
    /// Note: Part of tree state preservation API.
    #[allow(dead_code)]
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
        #[property(get, set)]
        status: RefCell<String>,
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
            .property("status", "disconnected")
            .build()
    }

    /// Creates a new connection item with status
    #[must_use]
    pub fn new_connection_with_status(
        id: &str,
        name: &str,
        protocol: &str,
        host: &str,
        status: &str,
    ) -> Self {
        glib::Object::builder()
            .property("id", id)
            .property("name", name)
            .property("protocol", protocol)
            .property("is-group", false)
            .property("is-document", false)
            .property("is-dirty", false)
            .property("host", host)
            .property("status", status)
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
