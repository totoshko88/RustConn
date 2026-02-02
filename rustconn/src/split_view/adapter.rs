//! Split view adapter bridging core models to GTK widgets
//!
//! This module provides the `SplitViewAdapter` which bridges the core
//! `SplitLayoutModel` from `rustconn-core` with GTK4/libadwaita widgets.
//! It maintains synchronization between the data model and the widget tree.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, DropTarget, Orientation, Overlay, Paned};
use libadwaita as adw;

use rustconn_core::split::{
    DropResult, PanelId, PanelNode, SessionId, SplitDirection, SplitError, SplitLayoutModel,
    SplitNode,
};

use super::types::{DropOutcome, DropSource, EvictionAction, SourceCleanup};

/// Callback type for "Select Tab" button clicks in empty panels.
///
/// When the user clicks "Select Tab" in an empty panel, this callback is invoked
/// with the panel ID. The UI layer (bridge) should then show a popover with
/// available sessions to choose from.
pub type SelectTabCallback = Rc<dyn Fn(PanelId)>;

/// Adapts `SplitLayoutModel` to GTK widgets.
///
/// This struct bridges the core data model with GTK4 widgets, maintaining
/// synchronization between the model state and the widget tree.
///
/// # Drop Handling
///
/// The adapter provides two ways to handle drops:
///
/// 1. **Direct method call**: Use `handle_drop()` or `handle_drop_session()` when
///    you have direct access to the adapter (e.g., from a custom drop handler).
///    These methods return a `DropOutcome` directly.
///
/// 2. **Built-in drop target**: Use `setup_drop_target()` to configure automatic
///    drop handling on panel widgets. After a drop, call `take_last_drop_outcome()`
///    to retrieve the outcome and handle source cleanup and eviction.
pub struct SplitViewAdapter {
    /// The underlying data model (wrapped for callback access)
    model: Rc<RefCell<SplitLayoutModel>>,
    /// Root GTK container
    root_widget: GtkBox,
    /// Map of panel IDs to their GTK containers
    panel_widgets: Rc<RefCell<HashMap<PanelId, GtkBox>>>,
    /// Paned widgets for splits (stored to prevent premature deallocation)
    paned_widgets: Vec<Paned>,
    /// Flag to track if a rebuild is needed (set by close button callbacks)
    needs_rebuild: Rc<RefCell<bool>>,
    /// The last drop outcome from a drop target callback.
    ///
    /// This is set by the drop target's `connect_drop` callback and can be
    /// retrieved via `take_last_drop_outcome()` to handle source cleanup
    /// and eviction in the UI layer.
    last_drop_outcome: Rc<RefCell<Option<DropOutcome>>>,
    /// Callback for "Select Tab" button clicks in empty panels.
    ///
    /// This allows the bridge to show a session selection popover when
    /// the user clicks the "Select Tab" button instead of dragging.
    select_tab_callback: Rc<RefCell<Option<SelectTabCallback>>>,
    /// Callback for close button clicks in empty panels.
    ///
    /// This allows the bridge to focus the panel and trigger the close action
    /// when the user clicks the close button on an empty panel.
    close_panel_callback: Rc<RefCell<Option<SelectTabCallback>>>,
}

impl std::fmt::Debug for SplitViewAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitViewAdapter")
            .field("model", &self.model)
            .field("root_widget", &self.root_widget)
            .field("panel_widgets", &self.panel_widgets)
            .field("paned_widgets", &self.paned_widgets)
            .field("needs_rebuild", &self.needs_rebuild)
            .field("last_drop_outcome", &self.last_drop_outcome)
            .field("select_tab_callback", &"<callback>")
            .field("close_panel_callback", &"<callback>")
            .finish()
    }
}

impl SplitViewAdapter {
    /// Creates a new adapter with an empty layout.
    #[must_use]
    pub fn new() -> Self {
        let model = Rc::new(RefCell::new(SplitLayoutModel::new()));
        let root_widget = GtkBox::new(Orientation::Vertical, 0);
        root_widget.set_hexpand(true);
        root_widget.set_vexpand(true);

        let mut adapter = Self {
            model,
            root_widget,
            panel_widgets: Rc::new(RefCell::new(HashMap::new())),
            paned_widgets: Vec::new(),
            needs_rebuild: Rc::new(RefCell::new(false)),
            last_drop_outcome: Rc::new(RefCell::new(None)),
            select_tab_callback: Rc::new(RefCell::new(None)),
            close_panel_callback: Rc::new(RefCell::new(None)),
        };

        adapter.rebuild_widgets();
        adapter
    }

    /// Creates a new adapter with a session in the initial panel.
    #[must_use]
    pub fn with_session(session: SessionId) -> Self {
        let model = Rc::new(RefCell::new(SplitLayoutModel::with_session(session)));
        let root_widget = GtkBox::new(Orientation::Vertical, 0);
        root_widget.set_hexpand(true);
        root_widget.set_vexpand(true);

        let mut adapter = Self {
            model,
            root_widget,
            panel_widgets: Rc::new(RefCell::new(HashMap::new())),
            paned_widgets: Vec::new(),
            needs_rebuild: Rc::new(RefCell::new(false)),
            last_drop_outcome: Rc::new(RefCell::new(None)),
            select_tab_callback: Rc::new(RefCell::new(None)),
            close_panel_callback: Rc::new(RefCell::new(None)),
        };

        adapter.rebuild_widgets();
        adapter
    }

    /// Returns the root widget for embedding in the UI.
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.root_widget
    }

    /// Takes the last drop outcome, if any.
    ///
    /// This method retrieves and clears the drop outcome that was stored by
    /// the drop target callback. Call this after a drop operation to handle
    /// source cleanup and eviction.
    ///
    /// # Returns
    ///
    /// Returns `Some(DropOutcome)` if a drop occurred since the last call,
    /// or `None` if no drop has occurred.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // After a drop event is signaled (e.g., via needs_rebuild flag)
    /// if let Some(outcome) = adapter.take_last_drop_outcome() {
    ///     // Handle source cleanup
    ///     match outcome.source_cleanup {
    ///         SourceCleanup::RemoveTab { session_id } => {
    ///             tab_manager.close_tab(session_id);
    ///         }
    ///         SourceCleanup::ClearPanel { source_tab_id, panel_id } => {
    ///             // Clear the source panel
    ///         }
    ///         SourceCleanup::None => {}
    ///     }
    ///
    ///     // Handle eviction
    ///     if let EvictionAction::CreateTab { evicted_session } = outcome.eviction {
    ///         tab_manager.create_tab_for_session(evicted_session);
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn take_last_drop_outcome(&self) -> Option<DropOutcome> {
        self.last_drop_outcome.borrow_mut().take()
    }

    /// Sets the callback for "Select Tab" button clicks in empty panels.
    ///
    /// When the user clicks the "Select Tab" button in an empty panel,
    /// this callback is invoked with the panel ID. The callback should
    /// show a popover or dialog allowing the user to select which session
    /// to display in the panel.
    ///
    /// This provides an alternative to drag-and-drop for moving sessions
    /// to split panels, which is useful since `AdwTabBar` intercepts drag
    /// events and prevents direct tab-to-panel dragging.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that receives the `PanelId` when the button is clicked
    pub fn set_select_tab_callback<F>(&self, callback: F)
    where
        F: Fn(PanelId) + 'static,
    {
        *self.select_tab_callback.borrow_mut() = Some(Rc::new(callback));
    }

    /// Sets a callback for close button clicks in empty panels.
    ///
    /// When the user clicks the close button (X) in an empty panel,
    /// this callback is invoked with the panel ID. The callback should
    /// focus the panel and trigger the close action.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that receives the `PanelId` when the close button is clicked
    pub fn set_close_panel_callback<F>(&self, callback: F)
    where
        F: Fn(PanelId) + 'static,
    {
        *self.close_panel_callback.borrow_mut() = Some(Rc::new(callback));
    }

    /// Returns a reference to the underlying model.
    ///
    /// Note: This returns a clone of the `Rc<RefCell<SplitLayoutModel>>` for
    /// external access. Use `with_model()` for internal operations.
    #[must_use]
    pub fn model(&self) -> Rc<RefCell<SplitLayoutModel>> {
        Rc::clone(&self.model)
    }

    /// Returns true if this layout has splits.
    #[must_use]
    pub fn is_split(&self) -> bool {
        self.model.borrow().is_split()
    }

    /// Returns the total number of panels in the layout.
    #[must_use]
    pub fn panel_count(&self) -> usize {
        self.model.borrow().panel_count()
    }

    /// Returns all panel IDs in the layout.
    #[must_use]
    pub fn panel_ids(&self) -> Vec<PanelId> {
        self.model.borrow().panel_ids()
    }

    /// Returns the ID of the currently focused panel.
    #[must_use]
    pub fn get_focused_panel(&self) -> Option<PanelId> {
        self.model.borrow().get_focused_panel()
    }

    /// Sets focus to a specific panel.
    pub fn set_focus(&mut self, panel_id: PanelId) -> Result<(), SplitError> {
        self.model.borrow_mut().set_focus(panel_id)?;
        self.update_focus_styling();
        Ok(())
    }

    /// Returns the session in a panel (if any).
    #[must_use]
    pub fn get_panel_session(&self, panel_id: PanelId) -> Option<SessionId> {
        self.model.borrow().get_panel_session(panel_id)
    }

    /// Returns the widget for a specific panel.
    #[must_use]
    pub fn get_panel_widget(&self, panel_id: PanelId) -> Option<GtkBox> {
        self.panel_widgets.borrow().get(&panel_id).cloned()
    }

    /// Splits the focused panel in the given direction.
    pub fn split(&mut self, direction: SplitDirection) -> Result<PanelId, SplitError> {
        let new_panel_id = self.model.borrow_mut().split(direction)?;
        self.rebuild_widgets();
        Ok(new_panel_id)
    }

    /// Places a session in the specified panel.
    pub fn place_in_panel(
        &mut self,
        panel_id: PanelId,
        session_id: SessionId,
    ) -> Result<DropResult, SplitError> {
        self.model.borrow_mut().place_in_panel(panel_id, session_id)
    }

    /// Handles a session disconnect event.
    ///
    /// When a session disconnects (server closes connection, user exits, etc.),
    /// this method finds the panel containing that session and removes it from
    /// the split container.
    ///
    /// # Requirements
    /// - 13.1: Panel is removed when connection is closed
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session that disconnected
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if the session was found and removed, `Ok(false)` if
    /// the session was not found in any panel, or an error if removal failed.
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn handle_session_disconnect(&mut self, session_id: SessionId) -> Result<bool, SplitError> {
        // Find the panel containing this session
        let panel_id = {
            let model = self.model.borrow();
            model
                .panel_ids()
                .into_iter()
                .find(|&pid| model.get_panel_session(pid) == Some(session_id))
        };

        if let Some(panel_id) = panel_id {
            tracing::debug!("Session {session_id} disconnected, removing panel {panel_id}");
            self.remove_panel(panel_id)?;
            Ok(true)
        } else {
            tracing::debug!("Session {session_id} disconnected but not found in any panel");
            Ok(false)
        }
    }

    /// Clears a session from its panel without removing the panel.
    ///
    /// This is useful when a session disconnects but you want to keep the
    /// panel as an empty drop target.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session to clear
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if the session was found and cleared, `Ok(false)` if
    /// the session was not found in any panel.
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn clear_session(&mut self, session_id: SessionId) -> Result<bool, SplitError> {
        // Find the panel containing this session
        let panel_id = {
            let model = self.model.borrow();
            model
                .panel_ids()
                .into_iter()
                .find(|&pid| model.get_panel_session(pid) == Some(session_id))
        };

        if let Some(panel_id) = panel_id {
            tracing::debug!("Clearing session {session_id} from panel {panel_id}");
            // Clear the panel content and show empty placeholder
            self.clear_panel(panel_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Handles a drop operation on a panel.
    ///
    /// This method processes a drag-and-drop operation, placing a session in the
    /// target panel and returning a comprehensive `DropOutcome` that describes:
    /// - The result from the core model (`DropResult::Placed` or `DropResult::Evicted`)
    /// - What cleanup is needed at the drag source
    /// - What action is needed for any evicted session
    ///
    /// # Drop Source Handling
    ///
    /// ## `RootTab` (Requirements 9.1, 9.2, 10.1, 10.2)
    /// - Session is moved from the tab to the panel
    /// - Source tab should be removed from the tab bar
    /// - If panel was occupied, evicted session goes to a new root tab
    ///
    /// ## `SplitPane` (Requirements 9.3, 10.3)
    /// - Session is moved from source panel to target panel
    /// - Source panel should be cleared or removed
    /// - If panel was occupied, evicted session goes to a new root tab
    ///
    /// ## `SidebarItem` (Requirements 9.4, 10.4)
    /// - A new session is created for the connection
    /// - No source cleanup needed (sidebar item remains)
    /// - If panel was occupied, evicted session goes to a new root tab
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The target panel to drop onto
    /// * `source` - The source of the drag operation
    /// * `session_factory` - A closure that creates a new session for sidebar items.
    ///   This is called only for `SidebarItem` sources. The closure receives the
    ///   `ConnectionId` and should return a new `SessionId`.
    ///
    /// # Returns
    ///
    /// Returns a `DropOutcome` containing all information needed to complete the
    /// drop operation in the UI layer.
    ///
    /// # Errors
    ///
    /// Returns `SplitError::PanelNotFound` if the target panel doesn't exist.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let outcome = adapter.handle_drop(
    ///     panel_id,
    ///     &DropSource::root_tab(session_id),
    ///     |_conn_id| SessionId::new(), // Not used for RootTab
    /// )?;
    ///
    /// // Handle source cleanup
    /// if let SourceCleanup::RemoveTab { session_id } = outcome.source_cleanup {
    ///     tab_manager.close_tab(session_id);
    /// }
    ///
    /// // Handle eviction
    /// if let EvictionAction::CreateTab { evicted_session } = outcome.eviction {
    ///     tab_manager.create_tab_for_session(evicted_session);
    /// }
    /// ```
    pub fn handle_drop<F>(
        &mut self,
        panel_id: PanelId,
        source: &DropSource,
        session_factory: F,
    ) -> Result<DropOutcome, SplitError>
    where
        F: FnOnce(&super::types::ConnectionId) -> SessionId,
    {
        // Determine the session to place and the source cleanup action
        let (session_to_place, source_cleanup) = match source {
            DropSource::RootTab { session_id } => {
                // Requirement 9.2: Remove source tab from tab bar
                let cleanup = SourceCleanup::RemoveTab {
                    session_id: *session_id,
                };
                (*session_id, cleanup)
            }
            DropSource::SplitPane {
                source_tab_id,
                panel_id: source_panel_id,
                session_id,
            } => {
                // Requirement 9.3: Move connection from another split container
                let cleanup = SourceCleanup::ClearPanel {
                    source_tab_id: *source_tab_id,
                    panel_id: *source_panel_id,
                };
                (*session_id, cleanup)
            }
            DropSource::SidebarItem { connection_id } => {
                // Requirement 9.4: Create new connection from sidebar item
                // No cleanup needed - sidebar item remains in place
                let new_session = session_factory(connection_id);
                (new_session, SourceCleanup::None)
            }
        };

        // Place the session in the panel
        let drop_result = self
            .model
            .borrow_mut()
            .place_in_panel(panel_id, session_to_place)?;

        // Determine eviction action based on drop result
        let eviction = match &drop_result {
            DropResult::Placed => EvictionAction::None,
            DropResult::Evicted { evicted_session } => {
                // Requirements 10.2, 10.3, 10.4: Evicted connection goes to new Root_Tab
                EvictionAction::CreateTab {
                    evicted_session: *evicted_session,
                }
            }
        };

        // Rebuild widgets to reflect the new state
        self.rebuild_widgets();

        Ok(DropOutcome::new(
            drop_result,
            source_cleanup,
            eviction,
            session_to_place,
        ))
    }

    /// Handles a drop operation with a pre-existing session ID.
    ///
    /// This is a convenience method for cases where the session ID is already
    /// known (e.g., from serialized drag data). It's equivalent to calling
    /// `handle_drop()` with a `RootTab` source.
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The target panel to drop onto
    /// * `session_id` - The session ID to place in the panel
    ///
    /// # Returns
    ///
    /// Returns a `DropOutcome` with `SourceCleanup::RemoveTab` indicating the
    /// source tab should be removed.
    pub fn handle_drop_session(
        &mut self,
        panel_id: PanelId,
        session_id: SessionId,
    ) -> Result<DropOutcome, SplitError> {
        let source = DropSource::RootTab { session_id };
        self.handle_drop(panel_id, &source, |_| {
            // This closure is never called for RootTab sources
            unreachable!("session_factory should not be called for RootTab source")
        })
    }

    /// Sets up a drop target on a panel widget for drag-and-drop operations.
    ///
    /// This method configures a `gtk4::DropTarget` on the given panel widget to:
    /// - Accept string data (serialized session ID or connection ID)
    /// - Provide visual feedback when a draggable item enters/leaves the panel
    /// - Handle the drop operation by calling `handle_drop()`
    ///
    /// # Visual Feedback
    ///
    /// The drop target provides different visual feedback based on panel state:
    /// - **Empty panels**: Use `drop-target-empty` CSS class (accent color, inviting)
    /// - **Occupied panels**: Use `drop-target-occupied` CSS class (warning color, indicates eviction)
    ///
    /// # Requirements
    /// - 8.1: Highlight target zone with focus border when drag enters
    /// - 8.2: Remove highlight when drag leaves
    /// - 8.3: Distinguish between Empty_Panel and Occupied_Panel drop targets
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The ID of the panel to set up the drop target for
    /// * `widget` - The GTK widget (panel container) to attach the drop target to
    pub fn setup_drop_target(&self, panel_id: PanelId, widget: &GtkBox) {
        // Create drop target that accepts string data (session ID as string)
        let drop_target = DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);

        // Clone references for the enter callback
        let model_for_enter = Rc::clone(&self.model);
        let widget_for_enter = widget.clone();

        // Connect enter signal for highlight feedback
        // Requirement 8.1: Highlight target zone with focus border when drag enters
        // Requirement 8.3: Distinguish between Empty_Panel and Occupied_Panel
        drop_target.connect_enter(move |_target, _x, _y| {
            // Determine if panel is empty or occupied to apply appropriate styling
            let is_occupied = model_for_enter
                .borrow()
                .get_panel_session(panel_id)
                .is_some();

            if is_occupied {
                widget_for_enter.add_css_class("drop-target-occupied");
            } else {
                widget_for_enter.add_css_class("drop-target-empty");
            }

            // Also add the generic highlight class for backward compatibility
            widget_for_enter.add_css_class("drop-target-highlight");

            gdk::DragAction::MOVE
        });

        // Clone reference for the leave callback
        let widget_for_leave = widget.clone();

        // Connect leave signal to remove highlight
        // Requirement 8.2: Remove highlight when drag leaves
        drop_target.connect_leave(move |_target| {
            widget_for_leave.remove_css_class("drop-target-highlight");
            widget_for_leave.remove_css_class("drop-target-empty");
            widget_for_leave.remove_css_class("drop-target-occupied");
        });

        // Clone references for the drop callback
        let model_for_drop = Rc::clone(&self.model);
        let needs_rebuild_for_drop = Rc::clone(&self.needs_rebuild);
        let last_drop_outcome_for_drop = Rc::clone(&self.last_drop_outcome);
        let widget_for_drop = widget.clone();

        // Connect drop signal to handle the drop operation
        // Requirements 9.1, 9.2, 9.4, 10.1, 10.2, 10.4: Handle drops from tabs and sidebar
        drop_target.connect_drop(move |_target, value, _x, _y| {
            // Remove highlight classes first
            widget_for_drop.remove_css_class("drop-target-highlight");
            widget_for_drop.remove_css_class("drop-target-empty");
            widget_for_drop.remove_css_class("drop-target-occupied");

            // Try to extract the drag data from the drop value
            let drag_data = match value.get::<String>() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to get string from drop value: {e}");
                    return false;
                }
            };

            // Parse the drag data to determine the source type
            // Format 1: "conn:uuid" - Connection from sidebar (Requirement 7.3)
            // Format 2: "uuid" - Session ID from tab or split pane (Requirements 7.1, 7.2)
            // Format 3: "group:uuid" - Group from sidebar (not droppable to panels)
            let (source_cleanup, session_id) = if let Some(conn_id_str) = drag_data.strip_prefix("conn:") {
                // Sidebar connection item - Requirement 9.4
                // Parse the connection ID
                let conn_uuid = match uuid::Uuid::parse_str(conn_id_str) {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        tracing::warn!("Failed to parse connection ID from sidebar drag data '{conn_id_str}': {e}");
                        return false;
                    }
                };

                // For sidebar items, we need to create a new session
                // The UI layer will handle this via the DropOutcome
                // For now, we create a placeholder session ID that the UI layer
                // will replace with a real session when it creates the connection
                let placeholder_session = SessionId::new();

                tracing::debug!(
                    "Sidebar connection {conn_uuid} dropped on panel {panel_id}, \
                     placeholder session {placeholder_session}"
                );

                // No source cleanup needed for sidebar items - they stay in the sidebar
                (SourceCleanup::None, placeholder_session)
            } else if drag_data.starts_with("group:") {
                // Group from sidebar - cannot be dropped on panels
                tracing::debug!("Groups cannot be dropped on split panels");
                return false;
            } else {
                // Session ID from tab or split pane - Requirements 9.1, 9.2, 10.1, 10.2
                let session_uuid = match uuid::Uuid::parse_str(&drag_data) {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        tracing::warn!("Failed to parse session ID from drop data '{drag_data}': {e}");
                        return false;
                    }
                };
                let session_id = SessionId::from_uuid(session_uuid);

                // Requirement 9.2: Source tab should be removed
                (SourceCleanup::RemoveTab { session_id }, session_id)
            };

            // Perform the drop operation on the model
            match model_for_drop.borrow_mut().place_in_panel(panel_id, session_id) {
                Ok(drop_result) => {
                    // Determine eviction action based on drop result
                    let eviction = match &drop_result {
                        DropResult::Placed => {
                            tracing::debug!(
                                "Session {session_id} placed in panel {panel_id}"
                            );
                            EvictionAction::None
                        }
                        DropResult::Evicted { evicted_session } => {
                            tracing::debug!(
                                "Session {session_id} placed in panel {panel_id}, \
                                 evicted session {evicted_session}"
                            );
                            // Requirement 10.2: Evicted connection goes to new Root_Tab
                            EvictionAction::CreateTab {
                                evicted_session: *evicted_session,
                            }
                        }
                    };

                    // Create the drop outcome
                    let outcome = DropOutcome::new(
                        drop_result,
                        source_cleanup,
                        eviction,
                        session_id,
                    );

                    // Store the outcome for retrieval by the UI layer
                    *last_drop_outcome_for_drop.borrow_mut() = Some(outcome);

                    // Signal that a rebuild may be needed to update the UI
                    *needs_rebuild_for_drop.borrow_mut() = true;
                    true
                }
                Err(e) => {
                    tracing::warn!("Failed to place session in panel {panel_id}: {e}");
                    false
                }
            }
        });

        // Add the drop target controller to the widget
        widget.add_controller(drop_target);
    }

    /// Removes a panel from the layout.
    pub fn remove_panel(&mut self, panel_id: PanelId) -> Result<Option<SessionId>, SplitError> {
        let session = self.model.borrow_mut().remove_panel(panel_id)?;
        self.rebuild_widgets();
        Ok(session)
    }

    /// Sets the content widget for a panel.
    pub fn set_panel_content(&self, panel_id: PanelId, widget: &impl IsA<gtk4::Widget>) {
        if let Some(panel_widget) = self.panel_widgets.borrow().get(&panel_id).cloned() {
            tracing::debug!(
                "set_panel_content: found panel widget for panel_id={}, clearing and setting content",
                panel_id
            );
            while let Some(child) = panel_widget.first_child() {
                panel_widget.remove(&child);
            }
            // Only unparent if the widget has a parent to avoid GTK-CRITICAL warning
            if widget.parent().is_some() {
                widget.unparent();
            }
            widget.set_hexpand(true);
            widget.set_vexpand(true);
            panel_widget.append(widget);
        } else {
            tracing::warn!(
                "set_panel_content: panel_id={} NOT FOUND in panel_widgets (available: {:?})",
                panel_id,
                self.panel_widgets.borrow().keys().collect::<Vec<_>>()
            );
        }
    }

    /// Clears the content of a panel and shows the empty placeholder.
    pub fn clear_panel(&self, panel_id: PanelId) {
        if let Some(panel_widget) = self.panel_widgets.borrow().get(&panel_id).cloned() {
            while let Some(child) = panel_widget.first_child() {
                panel_widget.remove(&child);
            }
            let placeholder = self.create_empty_placeholder(panel_id);
            panel_widget.append(&placeholder);
        }
    }

    /// Checks if a rebuild is needed and performs it.
    ///
    /// This is called after close button callbacks have requested a rebuild.
    /// Returns true if a rebuild was performed.
    pub fn check_and_rebuild(&mut self) -> bool {
        if *self.needs_rebuild.borrow() {
            *self.needs_rebuild.borrow_mut() = false;
            self.rebuild_widgets();
            true
        } else {
            false
        }
    }

    /// Rebuilds the widget tree from the model.
    pub fn rebuild_widgets(&mut self) {
        while let Some(child) = self.root_widget.first_child() {
            self.root_widget.remove(&child);
        }
        self.panel_widgets.borrow_mut().clear();
        self.paned_widgets.clear();

        let model = self.model.borrow();
        if model.is_split() {
            let root_node = model.root().cloned();
            drop(model); // Release borrow before building widgets
            if let Some(node) = root_node {
                let widget = self.build_node_widget(&node);
                self.root_widget.append(&widget);
            }
        } else {
            let panel_id = model.panel_ids()[0];
            drop(model); // Release borrow before building widgets
            let panel_widget = self.create_panel_widget(panel_id);
            self.panel_widgets
                .borrow_mut()
                .insert(panel_id, panel_widget.clone());
            self.root_widget.append(&panel_widget);
        }

        self.update_focus_styling();
    }

    fn build_node_widget(&mut self, node: &PanelNode) -> gtk4::Widget {
        match node {
            PanelNode::Leaf(leaf) => {
                let panel_widget = self.create_panel_widget(leaf.id);
                self.panel_widgets
                    .borrow_mut()
                    .insert(leaf.id, panel_widget.clone());
                panel_widget.upcast()
            }
            PanelNode::Split(split) => self.build_split_widget(split),
        }
    }

    fn build_split_widget(&mut self, split: &SplitNode) -> gtk4::Widget {
        let orientation = match split.direction {
            SplitDirection::Horizontal => Orientation::Vertical,
            SplitDirection::Vertical => Orientation::Horizontal,
        };

        let paned = Paned::new(orientation);
        paned.set_hexpand(true);
        paned.set_vexpand(true);

        // Set resize behavior for equal split
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        // Shrink both children equally when resizing
        paned.set_shrink_start_child(true);
        paned.set_shrink_end_child(true);

        let first_widget = self.build_node_widget(&split.first);
        let second_widget = self.build_node_widget(&split.second);

        paned.set_start_child(Some(&first_widget));
        paned.set_end_child(Some(&second_widget));

        // Set position to 50% when the widget is mapped and has a valid size
        // Using connect_map ensures the widget has been allocated space
        let paned_weak = paned.downgrade();
        paned.connect_map(move |_| {
            let paned_weak_inner = paned_weak.clone();
            // Use idle_add to ensure layout is complete after mapping
            glib::idle_add_local_once(move || {
                if let Some(p) = paned_weak_inner.upgrade() {
                    let size = if p.orientation() == Orientation::Horizontal {
                        p.width()
                    } else {
                        p.height()
                    };
                    if size > 0 {
                        p.set_position(size / 2);
                    }
                }
            });
        });

        self.paned_widgets.push(paned.clone());
        paned.upcast()
    }

    /// Creates a panel widget with proper expansion and color border styling.
    ///
    /// The panel widget is a `GtkBox` container that:
    /// - Expands horizontally and vertically to fill available space
    /// - Has a CSS class `split-panel` for base styling
    /// - Has a color-specific CSS class based on the layout's `ColorId` (e.g., `split-panel-color-0`)
    /// - Contains either an empty placeholder or occupied placeholder based on panel state
    /// - Has a drop target configured for drag-and-drop operations
    /// - Has a drag source configured for occupied panels (Requirement 7.2)
    ///
    /// # Color Border Styling
    ///
    /// When the layout has a `ColorId` assigned, the panel receives a CSS class
    /// `split-panel-color-N` where N is the color index (0-5). This allows CSS
    /// to apply colored borders to visually identify panels belonging to the
    /// same split container.
    ///
    /// # Requirements
    /// - 6.3: Panel borders within the Split_Container painted using the assigned Color_ID
    /// - 7.2: Occupied_Panel can be dragged from Split_Container
    /// - 8.1, 8.2, 8.3: Drop target with visual feedback
    fn create_panel_widget(&self, panel_id: PanelId) -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 0);

        // Set up proper expansion to fill available space
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Apply base panel styling
        container.add_css_class("split-panel");

        // Apply color border styling based on ColorId
        // The CSS class format is `split-panel-color-N` where N is the color index
        if let Some(color_id) = self.model.borrow().color_id() {
            let color_class = format!("split-panel-color-{}", color_id.index());
            container.add_css_class(&color_class);
        }

        // Set up drop target for drag-and-drop operations
        // Requirements 8.1, 8.2, 8.3: Drop target with visual feedback
        self.setup_drop_target(panel_id, &container);

        // Handle both empty and occupied panel states
        if let Some(session_id) = self.model.borrow().get_panel_session(panel_id) {
            // Set up drag source for occupied panels
            // Requirement 7.2: Occupied_Panel can be dragged from Split_Container
            self.setup_drag_source(panel_id, session_id, &container);

            // Set up context menu for occupied panels
            // Requirement 13.4: Right-click context menu with Close/Move options
            self.setup_panel_context_menu(panel_id, session_id, &container);

            let placeholder = self.create_occupied_placeholder();
            container.append(&placeholder);
        } else {
            let placeholder = self.create_empty_placeholder(panel_id);
            container.append(&placeholder);
        }

        container
    }

    /// Sets up a drag source on an occupied panel widget.
    ///
    /// This method configures a `gtk4::DragSource` on the given panel widget to:
    /// - Provide the session ID as drag data (serialized as string)
    /// - Add visual feedback during drag (CSS class `dragging`)
    /// - Handle removal from source after successful drop
    ///
    /// # Requirements
    /// - 7.2: Occupied_Panel can be dragged from Split_Container
    /// - 7.4: Visual feedback during drag
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The ID of the panel being dragged
    /// * `session_id` - The session ID in the panel
    /// * `widget` - The GTK widget (panel container) to attach the drag source to
    fn setup_drag_source(&self, panel_id: PanelId, session_id: SessionId, widget: &GtkBox) {
        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);

        // Prepare the drag data - session ID as string
        let session_str = session_id.to_string();
        drag_source.connect_prepare(move |_source, _x, _y| {
            let value = glib::Value::from(&session_str);
            let content = gdk::ContentProvider::for_value(&value);
            Some(content)
        });

        // Visual feedback: add CSS class when drag starts
        // Requirement 7.4: Visual feedback during drag
        let widget_for_begin = widget.clone();
        drag_source.connect_drag_begin(move |_source, _drag| {
            widget_for_begin.add_css_class("dragging");
            tracing::debug!("Started dragging panel {panel_id} with session {session_id}");
        });

        // Remove CSS class when drag ends and handle source cleanup
        let widget_for_end = widget.clone();
        let model_for_end = Rc::clone(&self.model);
        let needs_rebuild_for_end = Rc::clone(&self.needs_rebuild);
        drag_source.connect_drag_end(move |_source, _drag, delete_data| {
            widget_for_end.remove_css_class("dragging");

            // If delete_data is true, the drop was successful and we should clear the source panel
            if delete_data {
                tracing::debug!("Drag ended successfully, clearing source panel {panel_id}");
                // Clear the session from the source panel
                // Note: The actual removal is handled by the drop target, but we need to
                // signal a rebuild to update the UI
                if model_for_end.borrow().get_panel_session(panel_id).is_some() {
                    // The session is still in this panel, which means the drop was to a
                    // different location. We don't need to do anything here as the drop
                    // handler will have already updated the model.
                    *needs_rebuild_for_end.borrow_mut() = true;
                }
            }
        });

        // Set tooltip to indicate draggability
        widget.set_tooltip_text(Some("Drag to move this session to another panel or tab"));

        widget.add_controller(drag_source);
    }

    /// Sets up a right-click context menu on an occupied panel widget.
    ///
    /// The context menu provides options for:
    /// - "Close Connection": Removes the panel from the split container
    /// - "Move to New Tab": Extracts the session to a new root tab
    ///
    /// # Requirements
    /// - 13.4: Right-click context menu with Close Connection and Move to New Tab options
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The ID of the panel
    /// * `session_id` - The session ID in the panel
    /// * `widget` - The GTK widget (panel container) to attach the context menu to
    fn setup_panel_context_menu(&self, panel_id: PanelId, session_id: SessionId, widget: &GtkBox) {
        // Create action group for the panel
        let action_group = gio::SimpleActionGroup::new();

        // Create "close" action
        let close_action = gio::SimpleAction::new("close", None);
        let model_for_close = Rc::clone(&self.model);
        let needs_rebuild_for_close = Rc::clone(&self.needs_rebuild);
        close_action.connect_activate(move |_, _| {
            tracing::debug!("Context menu: Close Connection for panel {panel_id}");
            match model_for_close.borrow_mut().remove_panel(panel_id) {
                Ok(session) => {
                    if let Some(session) = session {
                        tracing::debug!("Removed panel {panel_id} with session {session}");
                    } else {
                        tracing::debug!("Removed empty panel {panel_id}");
                    }
                    *needs_rebuild_for_close.borrow_mut() = true;
                }
                Err(e) => {
                    tracing::warn!("Failed to close panel {panel_id}: {e}");
                }
            }
        });
        action_group.add_action(&close_action);

        // Create "move-to-tab" action
        let move_action = gio::SimpleAction::new("move-to-tab", None);
        let model_for_move = Rc::clone(&self.model);
        let needs_rebuild_for_move = Rc::clone(&self.needs_rebuild);
        let last_drop_outcome_for_move = Rc::clone(&self.last_drop_outcome);
        move_action.connect_activate(move |_, _| {
            tracing::debug!(
                "Context menu: Move to New Tab for panel {panel_id} with session {session_id}"
            );

            // Remove the panel and get the session
            match model_for_move.borrow_mut().remove_panel(panel_id) {
                Ok(Some(removed_session)) => {
                    tracing::debug!(
                        "Extracted session {removed_session} from panel {panel_id} for new tab"
                    );

                    // Create a drop outcome to signal that a new tab should be created
                    // The UI layer will handle this via take_last_drop_outcome()
                    let outcome = super::types::DropOutcome::new(
                        rustconn_core::split::DropResult::Placed,
                        super::types::SourceCleanup::None,
                        super::types::EvictionAction::CreateTab {
                            evicted_session: removed_session,
                        },
                        removed_session,
                    );
                    *last_drop_outcome_for_move.borrow_mut() = Some(outcome);
                    *needs_rebuild_for_move.borrow_mut() = true;
                }
                Ok(None) => {
                    tracing::warn!("Panel {panel_id} was empty, nothing to move");
                }
                Err(e) => {
                    tracing::warn!("Failed to extract session from panel {panel_id}: {e}");
                }
            }
        });
        action_group.add_action(&move_action);

        // Insert the action group into the widget
        widget.insert_action_group("panel", Some(&action_group));

        // Set up right-click gesture to show the context menu
        // Create popover dynamically on each right-click to avoid GTK popup grabbing issues
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gdk::BUTTON_SECONDARY);

        let widget_for_gesture = widget.clone();
        gesture.connect_pressed(move |gesture, _n_press, x, y| {
            // Create the context menu model
            let menu = gio::Menu::new();
            menu.append(Some("Close Connection"), Some("panel.close"));
            menu.append(Some("Move to New Tab"), Some("panel.move-to-tab"));

            // Create popover dynamically for this click
            let popover = gtk4::PopoverMenu::from_model(Some(&menu));
            popover.set_parent(&widget_for_gesture);
            popover.set_has_arrow(true);
            popover.set_autohide(true);

            // Position the popover at the click location
            let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.popup();
            gesture.set_state(gtk4::EventSequenceState::Claimed);

            // Clean up popover when closed
            let widget_weak = widget_for_gesture.downgrade();
            popover.connect_closed(move |pop| {
                if widget_weak.upgrade().is_some() {
                    pop.unparent();
                }
            });
        });

        widget.add_controller(gesture);
    }

    /// Creates the empty panel placeholder widget with close button and select tab button.
    ///
    /// The placeholder consists of:
    /// - An `adw::StatusPage` with instructions for selecting a tab
    /// - A "Select Tab" button below the status page for choosing a session
    /// - A close button (X) in the top-right corner using `gtk4::Overlay`
    /// - The close button triggers panel removal when clicked
    ///
    /// # Requirements
    /// - 4.1: Empty panel displays placeholder text
    /// - 4.5: Empty panel displays a close button (X icon) in the top-right corner
    /// - 4.6: When close button clicked, panel is removed from Split_Container
    fn create_empty_placeholder(&self, panel_id: PanelId) -> Overlay {
        // Create the status page with placeholder content
        // Use tab-symbolic icon to indicate this is for selecting tabs
        let status_page = adw::StatusPage::builder()
            .icon_name("tab-new-symbolic")
            .title("Empty Panel")
            .description("Select an existing tab to display in this panel")
            .hexpand(true)
            .vexpand(true)
            .build();

        status_page.add_css_class("empty-panel-placeholder");

        // Create "Select Tab" button as an alternative to drag-and-drop
        // This is useful because AdwTabBar intercepts drag events
        let select_button = Button::builder()
            .label("Select Tab...")
            .tooltip_text("Choose an open tab to display in this panel")
            .halign(Align::Center)
            .build();
        select_button.add_css_class("suggested-action");
        select_button.add_css_class("pill");

        // Connect select button to callback
        let callback_ref = Rc::clone(&self.select_tab_callback);
        select_button.connect_clicked(move |_| {
            if let Some(ref callback) = *callback_ref.borrow() {
                callback(panel_id);
            } else {
                tracing::debug!(
                    "Select Tab button clicked for panel {panel_id}, but no callback set"
                );
            }
        });

        // Set the button as the child of the status page
        status_page.set_child(Some(&select_button));

        // Create the close button
        let close_button = Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Close panel")
            .halign(Align::End)
            .valign(Align::Start)
            .margin_top(6)
            .margin_end(6)
            .build();

        close_button.add_css_class("flat");
        close_button.add_css_class("circular");
        close_button.add_css_class("panel-close-button");

        // Create overlay to position close button over status page
        let overlay = Overlay::new();
        overlay.set_child(Some(&status_page));
        overlay.add_overlay(&close_button);
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);

        // Connect close button to callback which will focus the panel and trigger close action
        // This ensures the correct panel is focused before the close-pane action runs
        let close_callback_ref = Rc::clone(&self.close_panel_callback);
        close_button.connect_clicked(move |_| {
            if let Some(ref callback) = *close_callback_ref.borrow() {
                tracing::debug!("Close button clicked for panel {panel_id}");
                callback(panel_id);
            } else {
                tracing::debug!("Close button clicked for panel {panel_id}, but no callback set");
            }
        });

        overlay
    }

    fn create_occupied_placeholder(&self) -> adw::StatusPage {
        adw::StatusPage::builder()
            .icon_name("content-loading-symbolic")
            .title("Loading...")
            .hexpand(true)
            .vexpand(true)
            .build()
    }

    fn update_focus_styling(&self) {
        let focused_id = self.model.borrow().get_focused_panel();

        for (panel_id, widget) in self.panel_widgets.borrow().iter() {
            if Some(*panel_id) == focused_id {
                widget.add_css_class("focused-panel");
            } else {
                widget.remove_css_class("focused-panel");
            }
        }
    }

    /// Sets up a click handler on a panel widget for focus management.
    ///
    /// When the panel is clicked, the callback is invoked with the panel ID.
    /// This allows the bridge to update focus state and switch tabs.
    ///
    /// The click event is claimed to prevent it from propagating to the terminal,
    /// which would cause unwanted text selection. However, clicks on buttons
    /// (like "Select Tab") are allowed to propagate so the button can handle them.
    ///
    /// # Arguments
    ///
    /// * `panel_id` - The ID of the panel
    /// * `callback` - A closure that receives the `PanelId` when the panel is clicked
    pub fn setup_panel_click_handler<F>(&self, panel_id: PanelId, callback: F)
    where
        F: Fn(PanelId) + 'static,
    {
        let Some(widget) = self.panel_widgets.borrow().get(&panel_id).cloned() else {
            tracing::warn!("Cannot set up click handler: panel {} not found", panel_id);
            return;
        };

        let click = gtk4::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.set_propagation_phase(gtk4::PropagationPhase::Capture);

        click.connect_pressed(move |gesture, _, x, y| {
            // Check if the click is on a button widget - if so, let it propagate
            // to allow the button to handle the click (e.g., "Select Tab" button)
            if let Some(gesture_widget) = gesture.widget() {
                if let Some(target_widget) = gesture_widget.pick(x, y, gtk4::PickFlags::DEFAULT) {
                    // Walk up the widget tree to check if we clicked on a button
                    let mut current: Option<gtk4::Widget> = Some(target_widget);
                    while let Some(widget) = current {
                        if widget.downcast_ref::<Button>().is_some() {
                            // Click is on a button - don't claim, let it propagate
                            tracing::debug!(
                                "Panel click handler: click on button in panel {}, not claiming",
                                panel_id
                            );
                            gesture.set_state(gtk4::EventSequenceState::None);
                            return;
                        }
                        current = widget.parent();
                    }
                }
            }

            tracing::debug!("Panel click handler: clicked on panel {}", panel_id);
            callback(panel_id);
            // Claim the event to prevent propagation to terminal (avoids text selection)
            // The callback should manually focus the terminal if needed
            gesture.set_state(gtk4::EventSequenceState::Claimed);
        });

        widget.add_controller(click);
    }
}

impl Default for SplitViewAdapter {
    fn default() -> Self {
        Self::new()
    }
}
