//! Split-screen terminal views
//!
//! This module provides split-screen functionality for terminal views,
//! allowing users to view multiple sessions simultaneously while maintaining
//! a single unified tab list.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, Orientation, Paned};
use libadwaita as adw;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;
use vte4::Terminal;

use crate::terminal::TerminalSession;

/// Color palette for split panes - neutral colors that work in both light and dark themes
/// Each color has a CSS class name for panes and a separate class for tabs
pub const SPLIT_PANE_COLORS: &[(&str, &str)] = &[
    ("split-color-blue", "tab-split-blue"),   // Blue - first split
    ("split-color-green", "tab-split-green"), // Green - second split
    ("split-color-orange", "tab-split-orange"), // Orange - third split
    ("split-color-purple", "tab-split-purple"), // Purple - fourth split
    ("split-color-cyan", "tab-split-cyan"),   // Cyan - fifth split
    ("split-color-pink", "tab-split-pink"),   // Pink - sixth split
];

/// Returns the CSS class for a given color index (for pane containers)
#[must_use]
pub fn get_split_color_class(color_index: usize) -> &'static str {
    SPLIT_PANE_COLORS
        .get(color_index % SPLIT_PANE_COLORS.len())
        .map_or("split-color-blue", |(class, _)| class)
}

/// Returns the CSS class for tab coloring for a given color index
#[must_use]
pub fn get_tab_color_class(color_index: usize) -> &'static str {
    SPLIT_PANE_COLORS
        .get(color_index % SPLIT_PANE_COLORS.len())
        .map_or("tab-split-blue", |(_, tab_class)| tab_class)
}

/// Returns the CSS indicator class for a given color index (for tab indicators)
#[must_use]
pub fn get_split_indicator_class(color_index: usize) -> String {
    let color_class = get_split_color_class(color_index);
    color_class.replace("split-color-", "split-indicator-")
}

/// Creates a scrolled window containing a terminal with proper expansion settings
fn create_terminal_scrolled_window(terminal: &Terminal) -> gtk4::ScrolledWindow {
    // Ensure terminal fills available space
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .propagate_natural_height(false)
        .propagate_natural_width(false)
        .child(terminal)
        .build();

    // Ensure the scrolled window takes all available space
    scrolled.set_valign(gtk4::Align::Fill);
    scrolled.set_halign(gtk4::Align::Fill);

    scrolled
}

/// Represents a split direction for terminal panes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Split horizontally (top and bottom panes)
    Horizontal,
    /// Split vertically (left and right panes)
    Vertical,
}

impl SplitDirection {
    /// Converts to GTK orientation
    #[must_use]
    pub const fn to_orientation(self) -> Orientation {
        match self {
            Self::Horizontal => Orientation::Vertical, // Vertical orientation = horizontal split
            Self::Vertical => Orientation::Horizontal, // Horizontal orientation = vertical split
        }
    }
}

/// A pane in the split terminal view
#[derive(Debug)]
pub struct TerminalPane {
    /// Unique identifier for this pane
    id: Uuid,
    /// Container widget for this pane's content
    container: GtkBox,
    /// Currently displayed session in this pane (if any)
    current_session: Option<Uuid>,
    /// Color index for this pane (used for tab coloring)
    color_index: Option<usize>,
}

impl TerminalPane {
    /// Creates a new terminal pane with drag-and-drop support
    #[must_use]
    pub fn new() -> Self {
        let id = Uuid::new_v4();
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        Self {
            id,
            container,
            current_session: None,
            color_index: None,
        }
    }

    /// Creates a new terminal pane with a specific color index
    /// Note: The color is used for tracking which pane a session belongs to,
    /// but the visual indicator is shown on the TAB, not the pane container
    #[must_use]
    pub fn with_color(color_index: usize) -> Self {
        let id = Uuid::new_v4();
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Don't add color class to container - color indicator is on tabs only

        Self {
            id,
            container,
            current_session: None,
            color_index: Some(color_index),
        }
    }

    /// Returns the color index for this pane
    #[must_use]
    pub const fn color_index(&self) -> Option<usize> {
        self.color_index
    }

    /// Sets the color index for this pane
    /// Note: The color is used for tracking, visual indicator is on tabs
    pub fn set_color_index(&mut self, index: usize) {
        self.color_index = Some(index);
    }

    /// Sets up click handler for focus management using capture phase
    /// This ensures we catch clicks even when they're on child widgets like terminals
    pub fn setup_click_handler<F>(&self, on_click: F)
    where
        F: Fn(Uuid) + 'static,
    {
        let click = gtk4::GestureClick::new();
        click.set_button(1); // Left click
                             // Use capture phase to intercept clicks before child widgets
        click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let pane_id = self.id;
        click.connect_pressed(move |gesture, _, _, _| {
            on_click(pane_id);
            // Don't stop propagation - let the terminal still receive the click
            gesture.set_state(gtk4::EventSequenceState::None);
        });
        self.container.add_controller(click);
    }

    /// Sets up context menu (right-click) for the pane
    /// The menu_builder returns a Menu with action names like "win.copy", "win.paste", etc.
    /// These actions are looked up from the window's action group.
    /// Note: This only shows when clicking on empty pane areas, not on terminals
    /// (terminals have their own context menu that takes priority)
    pub fn setup_context_menu<F>(&self, menu_builder: F)
    where
        F: Fn(Uuid) -> gtk4::gio::Menu + 'static,
    {
        let click = gtk4::GestureClick::new();
        click.set_button(3); // Right click
                             // Use bubble phase so terminal's context menu takes priority
        click.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        let pane_id = self.id;
        let container = self.container.clone();

        click.connect_pressed(move |gesture, _, x, y| {
            let menu = menu_builder(pane_id);
            let popover = gtk4::PopoverMenu::from_model(Some(&menu));
            popover.set_parent(&container);
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.set_has_arrow(false);

            // GTK4 PopoverMenu doesn't automatically inherit action groups from ancestors.
            // We need to create proxy actions that forward to the window's actions
            // and insert them into the POPOVER (not container) so menu items can find them.
            if let Some(root) = container.root() {
                if let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() {
                    let action_group = gtk4::gio::SimpleActionGroup::new();
                    let window_weak = window.downgrade();

                    // Create proxy actions for simple (no parameter) actions
                    let simple_actions = [
                        "copy",
                        "paste",
                        "close-tab",
                        "close-pane",
                        "split-horizontal",
                        "split-vertical",
                    ];

                    for name in simple_actions {
                        let win = window_weak.clone();
                        let action_name = name.to_string();
                        let action = gtk4::gio::SimpleAction::new(name, None);
                        action.connect_activate(move |_, _| {
                            if let Some(w) = win.upgrade() {
                                if let Some(a) = w.lookup_action(&action_name) {
                                    a.activate(None);
                                }
                            }
                        });
                        action_group.add_action(&action);
                    }

                    // Create proxy actions for string parameter actions
                    let string_actions = ["close-tab-by-id", "unsplit-session"];

                    for name in string_actions {
                        let win = window_weak.clone();
                        let action_name = name.to_string();
                        let action =
                            gtk4::gio::SimpleAction::new(name, Some(gtk4::glib::VariantTy::STRING));
                        action.connect_activate(move |_, param| {
                            if let Some(w) = win.upgrade() {
                                if let Some(a) = w.lookup_action(&action_name) {
                                    a.activate(param);
                                }
                            }
                        });
                        action_group.add_action(&action);
                    }

                    // Insert action group into the POPOVER so menu items can find them
                    popover.insert_action_group("win", Some(&action_group));
                }
            }

            popover.popup();

            // Claim the gesture to prevent terminal from getting the click
            gesture.set_state(gtk4::EventSequenceState::Claimed);

            // Clean up popover when closed
            let container_weak = container.downgrade();
            popover.connect_closed(move |pop| {
                if container_weak.upgrade().is_some() {
                    pop.unparent();
                }
            });
        });
        self.container.add_controller(click);
    }

    /// Sets up drag-and-drop for this pane with visual feedback
    pub fn setup_drop_target<F>(&self, on_drop: F)
    where
        F: Fn(Uuid, Uuid) + 'static, // (pane_id, session_id)
    {
        let drop_target =
            gtk4::DropTarget::new(gtk4::glib::Type::STRING, gtk4::gdk::DragAction::MOVE);

        let pane_id = self.id;
        let container = self.container.clone();
        let container_for_enter = self.container.clone();
        let container_for_leave = self.container.clone();

        // Visual feedback when drag enters the pane
        drop_target.connect_enter(move |_target, _x, _y| {
            container_for_enter.add_css_class("drop-target-highlight");
            gtk4::gdk::DragAction::MOVE
        });

        // Remove highlight when drag leaves
        drop_target.connect_leave(move |_target| {
            container_for_leave.remove_css_class("drop-target-highlight");
        });

        // Handle the drop
        drop_target.connect_drop(move |_target, value, _x, _y| {
            // Remove highlight
            container.remove_css_class("drop-target-highlight");

            if let Ok(session_str) = value.get::<String>() {
                if let Ok(session_id) = Uuid::parse_str(&session_str) {
                    on_drop(pane_id, session_id);
                    return true;
                }
            }
            false
        });

        self.container.add_controller(drop_target);
    }

    /// Returns the pane's unique identifier
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the pane's container widget
    #[must_use]
    pub const fn container(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the currently displayed session ID
    #[must_use]
    pub const fn current_session(&self) -> Option<Uuid> {
        self.current_session
    }

    /// Sets the currently displayed session
    pub fn set_current_session(&mut self, session_id: Option<Uuid>) {
        self.current_session = session_id;
    }

    /// Clears the pane's content
    pub fn clear(&self) {
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }
    }

    /// Sets the content widget for this pane
    pub fn set_content(&self, widget: &impl IsA<gtk4::Widget>) {
        self.clear();
        // Ensure the widget fills the pane
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_halign(gtk4::Align::Fill);
        widget.set_valign(gtk4::Align::Fill);
        self.container.append(widget);
    }
}

impl Default for TerminalPane {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared sessions type for use with `TerminalNotebook`
pub type SharedSessions = Rc<RefCell<HashMap<Uuid, TerminalSession>>>;
/// Shared terminals type for use with `TerminalNotebook`
pub type SharedTerminals = Rc<RefCell<HashMap<Uuid, Terminal>>>;

/// Mapping of session IDs to their split pane color index
pub type SessionColorMap = Rc<RefCell<HashMap<Uuid, usize>>>;

/// Manages split terminal views with a unified session list
pub struct SplitTerminalView {
    /// Root container widget
    root: GtkBox,
    /// All panes in the view
    panes: Rc<RefCell<Vec<TerminalPane>>>,
    /// Currently focused pane ID
    focused_pane: Rc<RefCell<Option<Uuid>>>,
    /// Shared sessions map (`session_id` -> `TerminalSession`)
    sessions: SharedSessions,
    /// Shared terminals map (`session_id` -> Terminal widget)
    terminals: SharedTerminals,
    /// Paned widgets for managing splits (stored for cleanup and preventing premature deallocation)
    paned_widgets: Rc<RefCell<Vec<Paned>>>,
    /// Next color index to assign to new split panes
    next_color_index: Rc<RefCell<usize>>,
    /// Mapping of session IDs to their color index (for tab coloring)
    session_colors: SessionColorMap,
}

impl SplitTerminalView {
    /// Creates a new split terminal view
    #[must_use]
    pub fn new() -> Self {
        Self::with_shared_state(
            Rc::new(RefCell::new(HashMap::new())),
            Rc::new(RefCell::new(HashMap::new())),
        )
    }

    /// Creates a new split terminal view with shared session and terminal state
    ///
    /// This allows sharing the session list with `TerminalNotebook` for unified tab management.
    #[must_use]
    pub fn with_shared_state(sessions: SharedSessions, terminals: SharedTerminals) -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        // Create initial pane (no color - it's the main pane)
        let initial_pane = TerminalPane::new();
        let initial_pane_id = initial_pane.id();

        // Set welcome content for initial pane
        let welcome = Self::create_welcome_content();
        initial_pane.set_content(&welcome);

        // Apply focused-pane CSS class to initial pane
        initial_pane.container().add_css_class("focused-pane");

        root.append(initial_pane.container());

        let panes = Rc::new(RefCell::new(vec![initial_pane]));
        let focused_pane = Rc::new(RefCell::new(Some(initial_pane_id)));

        Self {
            root,
            panes,
            focused_pane,
            sessions,
            terminals,
            paned_widgets: Rc::new(RefCell::new(Vec::new())),
            next_color_index: Rc::new(RefCell::new(0)),
            session_colors: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// Returns the session color map for external use (e.g., by TerminalNotebook)
    #[must_use]
    pub fn session_colors(&self) -> SessionColorMap {
        self.session_colors.clone()
    }

    /// Gets the color index for a session, if it's in a split pane
    #[must_use]
    pub fn get_session_color(&self, session_id: Uuid) -> Option<usize> {
        self.session_colors.borrow().get(&session_id).copied()
    }

    /// Sets the color for a session (called when session is added to a split pane)
    pub fn set_session_color(&self, session_id: Uuid, color_index: usize) {
        self.session_colors
            .borrow_mut()
            .insert(session_id, color_index);
    }

    /// Removes the color for a session (called when session is removed from split pane)
    pub fn clear_session_color(&self, session_id: Uuid) {
        self.session_colors.borrow_mut().remove(&session_id);
    }

    /// Allocates the next color index for a new split pane
    fn allocate_next_color(&self) -> usize {
        let mut index = self.next_color_index.borrow_mut();
        let color = *index;
        *index = (*index + 1) % SPLIT_PANE_COLORS.len();
        color
    }

    /// Creates welcome content for the initial pane
    fn create_welcome_content() -> adw::StatusPage {
        // Use the same content as placeholder for consistency
        Self::create_placeholder()
    }

    /// Returns the shared sessions reference for use with `TerminalNotebook`
    #[must_use]
    pub fn shared_sessions(&self) -> SharedSessions {
        self.sessions.clone()
    }

    /// Returns the shared terminals reference for use with `TerminalNotebook`
    #[must_use]
    pub fn shared_terminals(&self) -> SharedTerminals {
        self.terminals.clone()
    }

    /// Returns a reference to the panes for external setup
    #[must_use]
    pub fn panes_ref(&self) -> std::cell::Ref<'_, Vec<TerminalPane>> {
        self.panes.borrow()
    }

    /// Returns a clone of the panes Rc for callbacks
    #[must_use]
    pub fn panes_ref_clone(&self) -> Rc<RefCell<Vec<TerminalPane>>> {
        self.panes.clone()
    }

    /// Returns a clone of the focused_pane Rc for callbacks
    #[must_use]
    pub fn focused_pane_ref(&self) -> Rc<RefCell<Option<Uuid>>> {
        self.focused_pane.clone()
    }

    /// Returns the root widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.root
    }

    /// Returns the number of panes
    #[must_use]
    pub fn pane_count(&self) -> usize {
        self.panes.borrow().len()
    }

    /// Returns the focused pane ID
    #[must_use]
    pub fn focused_pane_id(&self) -> Option<Uuid> {
        *self.focused_pane.borrow()
    }

    /// Returns all pane IDs
    #[must_use]
    pub fn pane_ids(&self) -> Vec<Uuid> {
        self.panes.borrow().iter().map(TerminalPane::id).collect()
    }

    /// Returns all session IDs
    #[must_use]
    pub fn session_ids(&self) -> Vec<Uuid> {
        self.sessions.borrow().keys().copied().collect()
    }

    /// Returns the number of sessions
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.borrow().len()
    }

    /// Returns true if any pane has an active session displayed
    #[must_use]
    pub fn has_active_sessions(&self) -> bool {
        self.panes
            .borrow()
            .iter()
            .any(|p| p.current_session().is_some())
    }

    /// Resets the split view to initial state with a single pane
    /// Returns list of session IDs that were displayed in panes (for reparenting back to TabView)
    #[must_use]
    pub fn reset(&self) -> Vec<Uuid> {
        // Collect session IDs that were displayed in panes
        let displayed_sessions: Vec<Uuid> = self
            .panes
            .borrow()
            .iter()
            .filter_map(TerminalPane::current_session)
            .collect();

        // Clear session colors
        self.session_colors.borrow_mut().clear();

        // Reset color index
        *self.next_color_index.borrow_mut() = 0;

        // Remove all children from root
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }

        // Clear paned widgets
        self.paned_widgets.borrow_mut().clear();

        // Clear sessions and terminals (they'll be re-added when needed)
        self.sessions.borrow_mut().clear();
        self.terminals.borrow_mut().clear();

        // Create new initial pane
        let initial_pane = TerminalPane::new();
        let initial_pane_id = initial_pane.id();

        // Set welcome content
        let welcome = Self::create_welcome_content();
        initial_pane.set_content(&welcome);
        initial_pane.container().add_css_class("focused-pane");

        self.root.append(initial_pane.container());

        // Reset panes to single initial pane
        *self.panes.borrow_mut() = vec![initial_pane];
        *self.focused_pane.borrow_mut() = Some(initial_pane_id);

        displayed_sessions
    }

    /// Returns true if a specific session is displayed in any pane
    #[must_use]
    pub fn is_session_displayed(&self, session_id: Uuid) -> bool {
        self.panes
            .borrow()
            .iter()
            .any(|p| p.current_session() == Some(session_id))
    }

    /// Gets the color index of the pane displaying a session
    #[must_use]
    pub fn get_pane_color_for_session(&self, session_id: Uuid) -> Option<usize> {
        self.panes
            .borrow()
            .iter()
            .find(|p| p.current_session() == Some(session_id))
            .and_then(|p| p.color_index())
    }

    /// Gets session info by ID
    #[must_use]
    pub fn get_session_info(&self, session_id: Uuid) -> Option<TerminalSession> {
        self.sessions.borrow().get(&session_id).cloned()
    }

    /// Gets terminal by session ID
    #[must_use]
    pub fn get_terminal(&self, session_id: Uuid) -> Option<Terminal> {
        self.terminals.borrow().get(&session_id).cloned()
    }

    /// Adds a session to the shared session list
    pub fn add_session(&self, session: TerminalSession, terminal: Option<Terminal>) {
        let session_id = session.id;
        self.sessions.borrow_mut().insert(session_id, session);
        if let Some(term) = terminal {
            self.terminals.borrow_mut().insert(session_id, term);
        }
    }

    /// Removes a session from the shared session list
    pub fn remove_session(&self, session_id: Uuid) {
        self.sessions.borrow_mut().remove(&session_id);
        self.terminals.borrow_mut().remove(&session_id);
    }

    /// Clears a session from all panes that display it
    /// Shows a placeholder in panes that were displaying this session
    /// Used for "Move to Tab" action - keeps empty panes with "Drag Tab Here"
    pub fn clear_session_from_panes(&self, session_id: Uuid) {
        {
            let mut panes = self.panes.borrow_mut();
            for pane in panes.iter_mut() {
                if pane.current_session() == Some(session_id) {
                    // Show "Drag Tab Here" placeholder instead
                    let placeholder = Self::create_drag_here_placeholder();
                    pane.set_content(&placeholder);
                    pane.set_current_session(None);
                    // Clear session color
                    self.clear_session_color(session_id);
                }
            }
        }
        // Also remove from sessions and terminals
        self.remove_session(session_id);

        // NOTE: We do NOT auto-collapse empty panes for "Move to Tab"
        // Empty panes show "Drag Tab Here" and wait for drop
    }

    /// Closes a session from panes with auto-cleanup
    /// If the session was the only one in a pane, the pane is closed
    /// Used for "Close Connection" action
    /// Returns true if split view should be hidden (all panes closed or no sessions remain)
    #[must_use]
    pub fn close_session_from_panes(&self, session_id: Uuid) -> bool {
        // Find pane containing this session and set focus to it for close_pane
        let pane_to_close = {
            let panes = self.panes.borrow();
            panes
                .iter()
                .find(|p| p.current_session() == Some(session_id))
                .map(|p| p.id())
        };

        // Clear session color
        self.clear_session_color(session_id);

        // Remove from sessions and terminals
        self.remove_session(session_id);

        // If session was in a pane, close that pane (auto-cleanup per spec)
        if let Some(pane_id) = pane_to_close {
            // Set focus to the pane we want to close
            *self.focused_pane.borrow_mut() = Some(pane_id);

            // Clear the pane content first
            {
                let mut panes = self.panes.borrow_mut();
                if let Some(pane) = panes.iter_mut().find(|p| p.id() == pane_id) {
                    pane.set_current_session(None);
                }
            }

            // Try to close the pane (will fail if it's the only pane)
            let _ = self.close_pane();
        }

        // Return true if no sessions remain in any pane (should hide split view)
        let panes = self.panes.borrow();
        !panes.iter().any(|p| p.current_session().is_some())
    }

    /// Auto-collapse is DISABLED per spec
    /// Empty panes show "Drag Tab Here" and persist until explicitly closed
    #[allow(dead_code)]
    fn auto_collapse_empty_panes(&self) {
        // Intentionally empty - per spec, empty panes should NOT auto-collapse
        // They show "Drag Tab Here" placeholder and wait for user to drop a tab
    }

    /// Shows welcome content in the focused pane
    /// Used when switching to the Welcome tab
    pub fn show_welcome_in_focused_pane(&self) {
        let focused_id = match *self.focused_pane.borrow() {
            Some(id) => id,
            None => return,
        };

        let mut panes = self.panes.borrow_mut();
        if let Some(pane) = panes.iter_mut().find(|p| p.id() == focused_id) {
            let welcome = Self::create_welcome_content();
            pane.set_content(&welcome);
            pane.set_current_session(None);
        }
    }

    /// Sets the focused pane by ID
    pub fn set_focused_pane(&self, pane_id: Uuid) {
        let panes = self.panes.borrow();
        if panes.iter().any(|p| p.id() == pane_id) {
            *self.focused_pane.borrow_mut() = Some(pane_id);
        }
    }

    /// Gets the focused pane's current session
    #[must_use]
    pub fn get_focused_session(&self) -> Option<Uuid> {
        let focused_id = (*self.focused_pane.borrow())?;
        let panes = self.panes.borrow();
        panes
            .iter()
            .find(|p| p.id() == focused_id)
            .and_then(TerminalPane::current_session)
    }

    /// Gets the session displayed in a specific pane
    #[must_use]
    pub fn get_pane_session(&self, pane_id: Uuid) -> Option<Uuid> {
        let panes = self.panes.borrow();
        panes
            .iter()
            .find(|p| p.id() == pane_id)
            .and_then(TerminalPane::current_session)
    }

    /// Splits the focused pane in the given direction
    ///
    /// Creates a new Paned widget with the correct orientation,
    /// moves the current content to the first child, and creates
    /// a new pane for the second child.
    ///
    /// Returns the ID of the new pane and its color index, or None if there's no focused pane.
    #[must_use]
    pub fn split(&self, direction: SplitDirection) -> Option<(Uuid, usize)> {
        self.split_internal(direction, None)
    }

    /// Splits the focused pane with a close callback for the new pane
    ///
    /// The close callback is called when the close button on the placeholder is clicked.
    /// Returns the ID of the new pane and its color index, or None if there's no focused pane.
    pub fn split_with_close_callback<F>(
        &self,
        direction: SplitDirection,
        on_close: F,
    ) -> Option<(Uuid, usize)>
    where
        F: Fn() + 'static,
    {
        self.split_internal(direction, Some(Rc::new(on_close)))
    }

    /// Internal split implementation with optional close callback
    /// This creates a new pane at the ROOT level, not nested inside existing panes
    fn split_internal(
        &self,
        direction: SplitDirection,
        close_callback: Option<Rc<dyn Fn()>>,
    ) -> Option<(Uuid, usize)> {
        // Check if this is the first split (only one pane exists without color)
        let is_first_split = {
            let panes = self.panes.borrow();
            panes.len() == 1 && panes[0].color_index().is_none()
        };

        // If first split, assign color to the original pane
        if is_first_split {
            let original_color = self.allocate_next_color();
            let mut panes = self.panes.borrow_mut();
            if let Some(pane) = panes.first_mut() {
                pane.set_color_index(original_color);
            }
        }

        // Allocate a color for the new pane
        let color_index = self.allocate_next_color();

        // Get the root container
        let root = &self.root;

        // Get the current content of root (either a pane container or a Paned)
        let current_content = root.first_child()?;

        // Remove current content from root
        root.remove(&current_content);

        // Create a new Paned widget at the root level
        let new_paned = Paned::new(direction.to_orientation());
        new_paned.set_hexpand(true);
        new_paned.set_vexpand(true);

        // Set the current content as the first child
        new_paned.set_start_child(Some(&current_content));

        // Create a new pane for the second child with assigned color
        let new_pane = TerminalPane::with_color(color_index);
        let new_pane_id = new_pane.id();

        // Add placeholder content to new pane with close button
        let placeholder: gtk4::Widget = if let Some(cb) = close_callback {
            Self::create_placeholder_with_close(move || cb()).upcast()
        } else {
            Self::create_placeholder().upcast()
        };
        new_pane.set_content(&placeholder);

        new_paned.set_end_child(Some(new_pane.container()));

        // Set resize behavior for equal split
        new_paned.set_resize_start_child(true);
        new_paned.set_resize_end_child(true);

        // Add the paned to root
        root.append(&new_paned);

        // Set position to 50% after a short delay to ensure layout is complete
        let paned_weak = new_paned.downgrade();
        gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            if let Some(p) = paned_weak.upgrade() {
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

        // Store the paned widget
        self.paned_widgets.borrow_mut().push(new_paned);

        // Add the new pane to our collection
        self.panes.borrow_mut().push(new_pane);

        Some((new_pane_id, color_index))
    }

    /// Creates a placeholder widget for empty panes with close button
    fn create_placeholder_with_close<F>(on_close: F) -> GtkBox
    where
        F: Fn() + 'static,
    {
        let outer = GtkBox::new(Orientation::Vertical, 0);
        outer.set_hexpand(true);
        outer.set_vexpand(true);

        // Header with close button (only visible when there are multiple panes)
        let header = GtkBox::new(Orientation::Horizontal, 0);
        header.set_halign(gtk4::Align::End);
        header.set_margin_top(6);
        header.set_margin_end(6);

        let close_button = Button::from_icon_name("window-close-symbolic");
        close_button.add_css_class("flat");
        close_button.add_css_class("circular");
        close_button.set_tooltip_text(Some("Close pane (Ctrl+Shift+W)"));
        close_button.connect_clicked(move |_| {
            on_close();
        });
        header.append(&close_button);
        outer.append(&header);

        // Use AdwStatusPage for center content - "Drag Tab Here" per spec
        let status_page = adw::StatusPage::builder()
            .icon_name("insert-object-symbolic")
            .title("Drag Tab Here")
            .vexpand(true)
            .build();

        outer.append(&status_page);
        outer
    }

    /// Creates a "Drag Tab Here" placeholder for empty panes (per spec)
    fn create_drag_here_placeholder() -> adw::StatusPage {
        adw::StatusPage::builder()
            .icon_name("insert-object-symbolic")
            .title("Drag Tab Here")
            .vexpand(true)
            .build()
    }

    /// Creates a placeholder widget for empty panes (simple version)
    fn create_placeholder() -> adw::StatusPage {
        let status_page = adw::StatusPage::new();

        // Try to set logo image as icon
        if let Some(pixbuf) = Self::load_embedded_logo(96) {
            let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
            let paintable = texture.upcast::<gtk4::gdk::Paintable>();
            status_page.set_paintable(Some(&paintable));
        } else {
            status_page.set_icon_name(Some("network-server-symbolic"));
        }

        status_page.set_title("RustConn");
        status_page.set_description(Some(
            "Modern Connection Manager for Linux • SSH • RDP • VNC • SPICE • Zero Trust",
        ));

        // Create content box for additional elements
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_halign(gtk4::Align::Fill);
        content.set_hexpand(true);
        content.set_margin_top(6);
        content.set_margin_bottom(12);
        content.set_margin_start(24);
        content.set_margin_end(24);

        // Quick actions as buttons
        let actions = GtkBox::new(Orientation::Horizontal, 12);
        actions.set_halign(gtk4::Align::Center);

        let new_conn_btn = Button::builder()
            .label("New Connection")
            .css_classes(["suggested-action", "pill"])
            .action_name("win.new-connection")
            .build();
        actions.append(&new_conn_btn);

        let quick_btn = Button::builder()
            .label("Quick Connect")
            .css_classes(["pill"])
            .action_name("win.quick-connect")
            .build();
        actions.append(&quick_btn);

        content.append(&actions);

        // Three-column layout
        let columns = GtkBox::new(Orientation::Horizontal, 18);
        columns.set_halign(gtk4::Align::Fill);
        columns.set_hexpand(true);
        columns.set_homogeneous(true);
        columns.set_margin_top(12);

        // Column 1 - Features
        let col1 = GtkBox::new(Orientation::Vertical, 6);
        col1.set_valign(gtk4::Align::Start);
        col1.set_hexpand(true);

        let features_group = adw::PreferencesGroup::builder().title("Features").build();

        let features = [
            ("utilities-terminal-symbolic", "Embedded SSH terminals"),
            ("channel-secure-symbolic", "Secure credential storage"),
            ("dialog-password-symbolic", "Password Generator"),
            ("view-refresh-symbolic", "Session Restore"),
            ("system-run-symbolic", "Expect automation"),
            ("folder-symbolic", "Groups and tags"),
            ("network-workgroup-symbolic", "Zero Trust tunnels"),
            ("preferences-system-symbolic", "Customizable settings"),
            (
                "application-x-executable-symbolic",
                "Embedded &amp; external clients",
            ),
        ];

        for (icon, description) in features {
            let row = adw::ActionRow::builder().title(description).build();
            row.add_prefix(&gtk4::Image::from_icon_name(icon));
            features_group.add(&row);
        }
        col1.append(&features_group);
        columns.append(&col1);

        // Column 2 - Keyboard shortcuts
        let col2 = GtkBox::new(Orientation::Vertical, 6);
        col2.set_valign(gtk4::Align::Start);
        col2.set_hexpand(true);

        let shortcuts_group = adw::PreferencesGroup::builder()
            .title("Keyboard Shortcuts")
            .build();

        let shortcuts = [
            ("Ctrl+N", "New connection"),
            ("Ctrl+Shift+N", "New group"),
            ("Ctrl+Shift+T", "Local shell"),
            ("Ctrl+Shift+Q", "Quick connect"),
            ("Ctrl+F", "Search"),
            ("Ctrl+Shift+S", "Split vertical"),
            ("Ctrl+Shift+H", "Split horizontal"),
            ("Ctrl+W", "Close tab"),
            ("Ctrl+Tab", "Next tab"),
        ];

        for (shortcut, description) in shortcuts {
            let row = adw::ActionRow::builder().title(description).build();
            let label = Label::builder()
                .label(shortcut)
                .css_classes(["dim-label", "monospace"])
                .build();
            row.add_suffix(&label);
            shortcuts_group.add(&row);
        }
        col2.append(&shortcuts_group);
        columns.append(&col2);

        // Column 3 - Performance & Import
        let col3 = GtkBox::new(Orientation::Vertical, 6);
        col3.set_valign(gtk4::Align::Start);
        col3.set_hexpand(true);

        let perf_group = adw::PreferencesGroup::builder()
            .title("Performance")
            .build();

        let perf_features = [
            ("edit-find-symbolic", "Smart search caching"),
            ("folder-symbolic", "Lazy loading for trees"),
            ("view-list-symbolic", "Virtual scrolling"),
            ("video-display-symbolic", "Embedded VNC/RDP/SPICE"),
        ];

        for (icon, description) in perf_features {
            let row = adw::ActionRow::builder().title(description).build();
            row.add_prefix(&gtk4::Image::from_icon_name(icon));
            perf_group.add(&row);
        }
        col3.append(&perf_group);

        // Import formats
        let formats_group = adw::PreferencesGroup::builder()
            .title("Import Formats")
            .margin_top(6)
            .build();

        let formats = [
            "Remmina",
            "Asbru-CM / Royal TS / MobaXterm",
            "SSH Config",
            "Ansible Inventory",
        ];

        for format in formats {
            let row = adw::ActionRow::builder().title(format).build();
            row.add_prefix(&gtk4::Image::from_icon_name("document-open-symbolic"));
            formats_group.add(&row);
        }
        col3.append(&formats_group);
        columns.append(&col3);

        content.append(&columns);

        // Hint at the bottom
        let hint = Label::builder()
            .label("Double-click a connection in the sidebar to get started")
            .css_classes(["dim-label"])
            .margin_top(12)
            .build();
        content.append(&hint);

        status_page.set_child(Some(&content));
        status_page
    }

    /// Creates logo image or fallback text title (kept for compatibility)
    #[allow(dead_code)]
    fn create_logo_or_title() -> gtk4::Widget {
        // Try to load embedded SVG icon using GdkPixbuf
        if let Some(pixbuf) = Self::load_embedded_logo(64) {
            let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
            let image = gtk4::Image::from_paintable(Some(&texture));
            image.set_pixel_size(64);
            image.set_margin_bottom(8);

            let hbox = GtkBox::new(Orientation::Horizontal, 12);
            hbox.set_halign(gtk4::Align::Center);
            hbox.append(&image);

            let title = Label::new(Some("RustConn"));
            title.add_css_class("title-1");
            hbox.append(&title);

            return hbox.upcast();
        }

        // Fallback to text-only title with icon
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_halign(gtk4::Align::Center);

        let icon = gtk4::Image::from_icon_name("network-server-symbolic");
        icon.set_pixel_size(48);
        hbox.append(&icon);

        let title = gtk4::Label::new(Some("RustConn"));
        title.add_css_class("title-1");
        hbox.append(&title);

        hbox.upcast()
    }

    /// Load embedded SVG logo and render to GdkPixbuf
    fn load_embedded_logo(size: u32) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
        // Embedded SVG icon data
        const ICON_SVG: &[u8] = include_bytes!(
            "../assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"
        );

        // Parse SVG using resvg
        let tree = resvg::usvg::Tree::from_data(ICON_SVG, &resvg::usvg::Options::default()).ok()?;

        // Create pixmap
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;

        // Calculate transform to fit SVG into target size
        let svg_size = tree.size();
        let scale = (size as f32 / svg_size.width()).min(size as f32 / svg_size.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);

        // Render SVG to pixmap
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Convert from premultiplied RGBA to straight RGBA for GdkPixbuf
        let premultiplied = pixmap.data();
        let mut rgba_data = Vec::with_capacity(premultiplied.len());
        for chunk in premultiplied.chunks_exact(4) {
            let a = chunk[3];
            if a == 0 {
                rgba_data.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                // Un-premultiply: color = premultiplied_color * 255 / alpha
                let r = (u16::from(chunk[0]) * 255 / u16::from(a)) as u8;
                let g = (u16::from(chunk[1]) * 255 / u16::from(a)) as u8;
                let b = (u16::from(chunk[2]) * 255 / u16::from(a)) as u8;
                rgba_data.extend_from_slice(&[r, g, b, a]);
            }
        }

        Some(gtk4::gdk_pixbuf::Pixbuf::from_bytes(
            &gtk4::glib::Bytes::from(&rgba_data),
            gtk4::gdk_pixbuf::Colorspace::Rgb,
            true, // has_alpha
            8,    // bits_per_sample
            size as i32,
            size as i32,
            (size * 4) as i32, // rowstride
        ))
    }

    /// Closes the focused pane
    ///
    /// Removes the pane from the collection and merges the remaining content
    /// if only one pane is left. Updates focus to the remaining pane.
    ///
    /// Returns true if a pane was closed, false if there's only one pane
    /// or no focused pane.
    #[must_use]
    pub fn close_pane(&self) -> bool {
        // Can't close if only one pane
        if self.panes.borrow().len() <= 1 {
            return false;
        }

        let focused_id = match *self.focused_pane.borrow() {
            Some(id) => id,
            None => return false,
        };

        // Find the focused pane
        let mut panes = self.panes.borrow_mut();
        let Some(index) = panes.iter().position(|p| p.id() == focused_id) else {
            return false;
        };

        let focused_pane = &panes[index];
        let focused_container = focused_pane.container().clone();

        // Get the parent (should be a Paned widget)
        let Some(parent) = focused_container.parent() else {
            return false;
        };

        if let Some(parent_paned) = parent.downcast_ref::<Paned>() {
            // Determine which child we are and get the sibling
            let is_start = parent_paned
                .start_child()
                .is_some_and(|c| c == focused_container);

            let sibling = if is_start {
                parent_paned.end_child()
            } else {
                parent_paned.start_child()
            };

            // Get the grandparent
            let grandparent = parent_paned.parent();

            // Clear focus before removing children to avoid GTK warning
            if let Some(root) = parent_paned.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    gtk4::prelude::GtkWindowExt::set_focus(window, None::<&gtk4::Widget>);
                }
            }

            // Remove both children from the paned
            parent_paned.set_start_child(None::<&gtk4::Widget>);
            parent_paned.set_end_child(None::<&gtk4::Widget>);

            // Replace the paned with the sibling in the grandparent
            if let Some(gp) = grandparent {
                if let Some(gp_box) = gp.downcast_ref::<GtkBox>() {
                    gp_box.remove(parent_paned);
                    if let Some(sib) = sibling {
                        gp_box.append(&sib);
                    }
                } else if let Some(gp_paned) = gp.downcast_ref::<Paned>() {
                    let is_start_in_gp = gp_paned.start_child().is_some_and(|c| c == *parent_paned);

                    if is_start_in_gp {
                        gp_paned.set_start_child(sibling.as_ref());
                    } else {
                        gp_paned.set_end_child(sibling.as_ref());
                    }
                }
            }

            // Remove the pane from our collection
            panes.remove(index);

            // Update focus to another pane
            if panes.is_empty() {
                *self.focused_pane.borrow_mut() = None;
            } else {
                let new_index = index.min(panes.len() - 1);
                *self.focused_pane.borrow_mut() = Some(panes[new_index].id());
            }

            true
        } else {
            false
        }
    }

    /// Cycles focus to the next pane
    ///
    /// Cycles through panes in order and updates the visual focus indicator.
    ///
    /// Returns the ID of the newly focused pane, or None if there are no panes.
    #[must_use]
    pub fn focus_next_pane(&self) -> Option<Uuid> {
        let panes = self.panes.borrow();
        if panes.is_empty() {
            return None;
        }

        let current_index = self
            .focused_pane
            .borrow()
            .and_then(|id| panes.iter().position(|p| p.id() == id))
            .unwrap_or(0);

        let next_index = (current_index + 1) % panes.len();
        let next_id = panes[next_index].id();

        // Update visual focus indicator
        self.update_focus_indicator(&panes, Some(next_id));

        drop(panes);
        *self.focused_pane.borrow_mut() = Some(next_id);

        Some(next_id)
    }

    /// Updates the visual focus indicator for panes
    fn update_focus_indicator(&self, panes: &[TerminalPane], focused_id: Option<Uuid>) {
        for pane in panes {
            let container = pane.container();
            if Some(pane.id()) == focused_id {
                container.add_css_class("focused-pane");
                container.remove_css_class("unfocused-pane");
            } else {
                container.remove_css_class("focused-pane");
                container.add_css_class("unfocused-pane");
            }
        }
    }

    /// Sets focus to a specific pane and updates visual indicator
    #[must_use]
    pub fn focus_pane(&self, pane_id: Uuid) -> bool {
        let panes = self.panes.borrow();
        if panes.iter().any(|p| p.id() == pane_id) {
            self.update_focus_indicator(&panes, Some(pane_id));
            drop(panes);
            *self.focused_pane.borrow_mut() = Some(pane_id);
            true
        } else {
            false
        }
    }

    /// Displays a session in the focused pane
    ///
    /// Shows the terminal for the selected session in the currently focused pane
    /// and updates the pane's `current_session`.
    ///
    /// Returns true if the session was shown successfully.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn show_session(&self, session_id: Uuid) -> bool {
        let focused_id = match *self.focused_pane.borrow() {
            Some(id) => id,
            None => return false,
        };

        // Verify session exists
        if !self.sessions.borrow().contains_key(&session_id) {
            return false;
        }

        // Find focused pane and update its content
        let mut panes = self.panes.borrow_mut();

        // Get the session currently shown in focused pane (if any)
        let focused_current_session = panes
            .iter()
            .find(|p| p.id() == focused_id)
            .and_then(TerminalPane::current_session);

        // If focused pane already shows this session, nothing to do
        if focused_current_session == Some(session_id) {
            return true;
        }

        // Find which pane (if any) currently shows the session we want to display
        let source_pane_id = panes
            .iter()
            .find(|p| p.current_session() == Some(session_id))
            .map(TerminalPane::id);

        // Strategy: swap sessions between panes if possible
        // If focused pane has a session and source pane exists, swap them
        // Otherwise, just move the session and show placeholder in source pane

        if let Some(source_id) = source_pane_id {
            if source_id != focused_id {
                // Session is in another pane - need to handle the swap
                if let Some(swap_session) = focused_current_session {
                    // Focused pane has a session - swap with source pane
                    // First, detach both terminals from their parents
                    let terminals_ref = self.terminals.borrow();
                    let source_terminal = terminals_ref.get(&session_id).cloned();
                    let swap_terminal = terminals_ref.get(&swap_session).cloned();
                    drop(terminals_ref);

                    // Detach source terminal
                    if let Some(ref term) = source_terminal {
                        if let Some(parent) = term.parent() {
                            if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                                scrolled.set_child(None::<&gtk4::Widget>);
                            } else if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                                box_widget.remove(term);
                            }
                        }
                    }

                    // Detach swap terminal
                    if let Some(ref term) = swap_terminal {
                        if let Some(parent) = term.parent() {
                            if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                                scrolled.set_child(None::<&gtk4::Widget>);
                            } else if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                                box_widget.remove(term);
                            }
                        }
                    }

                    // Now place swap_terminal in source pane
                    if let Some(source_pane) = panes.iter_mut().find(|p| p.id() == source_id) {
                        if let Some(term) = swap_terminal {
                            let scrolled = create_terminal_scrolled_window(&term);
                            source_pane.set_content(&scrolled);
                            term.set_visible(true);
                        } else {
                            // External session - show placeholder
                            let sessions_ref = self.sessions.borrow();
                            let info = sessions_ref.get(&swap_session);
                            let name = info.map_or("Unknown", |s| &s.name);
                            let protocol = info.map_or("unknown", |s| &s.protocol);
                            let placeholder =
                                Self::create_external_session_placeholder(name, protocol);
                            drop(sessions_ref);
                            source_pane.set_content(&placeholder);
                        }
                        source_pane.set_current_session(Some(swap_session));
                    }

                    // Place source_terminal in focused pane
                    if let Some(focused_pane) = panes.iter_mut().find(|p| p.id() == focused_id) {
                        if let Some(term) = source_terminal {
                            let scrolled = create_terminal_scrolled_window(&term);
                            focused_pane.set_content(&scrolled);
                            term.set_visible(true);
                            term.grab_focus();
                        } else {
                            // External session
                            let sessions_ref = self.sessions.borrow();
                            let info = sessions_ref.get(&session_id);
                            let name = info.map_or("Unknown", |s| &s.name);
                            let protocol = info.map_or("unknown", |s| &s.protocol);
                            let placeholder =
                                Self::create_external_session_placeholder(name, protocol);
                            drop(sessions_ref);
                            focused_pane.set_content(&placeholder);
                        }
                        focused_pane.set_current_session(Some(session_id));
                    }

                    return true;
                }

                // Focused pane is empty - just move session there, show placeholder in source
                let terminals_ref = self.terminals.borrow();
                let terminal = terminals_ref.get(&session_id).cloned();
                drop(terminals_ref);

                // Detach terminal
                if let Some(ref term) = terminal {
                    if let Some(parent) = term.parent() {
                        if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                            scrolled.set_child(None::<&gtk4::Widget>);
                        } else if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                            box_widget.remove(term);
                        }
                    }
                }

                // Show placeholder in source pane
                if let Some(source_pane) = panes.iter_mut().find(|p| p.id() == source_id) {
                    let placeholder = Self::create_placeholder();
                    source_pane.set_content(&placeholder);
                    source_pane.set_current_session(None);
                }

                // Show terminal in focused pane
                if let Some(focused_pane) = panes.iter_mut().find(|p| p.id() == focused_id) {
                    if let Some(term) = terminal {
                        let scrolled = create_terminal_scrolled_window(&term);
                        focused_pane.set_content(&scrolled);
                        term.set_visible(true);
                        term.grab_focus();
                    } else {
                        // External session
                        let sessions_ref = self.sessions.borrow();
                        let info = sessions_ref.get(&session_id);
                        let name = info.map_or("Unknown", |s| &s.name);
                        let protocol = info.map_or("unknown", |s| &s.protocol);
                        let placeholder = Self::create_external_session_placeholder(name, protocol);
                        drop(sessions_ref);
                        focused_pane.set_content(&placeholder);
                    }
                    focused_pane.set_current_session(Some(session_id));
                }

                return true;
            }
        }

        // Session is not shown in any pane - just show it in focused pane
        let Some(pane) = panes.iter_mut().find(|p| p.id() == focused_id) else {
            return false;
        };

        // Get the terminal for this session
        let terminals_ref = self.terminals.borrow();
        if let Some(terminal) = terminals_ref.get(&session_id) {
            // Clone terminal reference before dropping borrow
            let terminal = terminal.clone();
            drop(terminals_ref);

            // Remove terminal from any previous parent first
            // This is critical - GTK widgets can only have one parent
            if let Some(parent) = terminal.parent() {
                if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                    scrolled.set_child(None::<&gtk4::Widget>);
                } else if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                    box_widget.remove(&terminal);
                }
            }

            // Create a scrolled window for the terminal
            let scrolled = create_terminal_scrolled_window(&terminal);
            pane.set_content(&scrolled);

            // Ensure terminal is visible and can receive input
            terminal.set_visible(true);
            terminal.grab_focus();
        } else {
            drop(terminals_ref);
            // No terminal (external session), show placeholder
            let session_info = self.sessions.borrow();
            let session = session_info.get(&session_id);
            let name = session.map_or("Unknown", |s| &s.name);
            let protocol = session.map_or("unknown", |s| &s.protocol);

            let placeholder = Self::create_external_session_placeholder(name, protocol);
            pane.set_content(&placeholder);
        }

        pane.set_current_session(Some(session_id));
        true
    }

    /// Creates a placeholder for external sessions (RDP/VNC)
    fn create_external_session_placeholder(name: &str, protocol: &str) -> adw::StatusPage {
        let icon_name = match protocol {
            "rdp" => "computer-symbolic",
            "vnc" => "preferences-desktop-remote-desktop-symbolic",
            "spice" => "preferences-desktop-remote-desktop-symbolic",
            _ => "network-server-symbolic",
        };

        adw::StatusPage::builder()
            .icon_name(icon_name)
            .title(name)
            .description(format!(
                "{} session running in external window",
                protocol.to_uppercase()
            ))
            .build()
    }

    /// Gets the active terminal in the focused pane
    #[must_use]
    pub fn get_active_terminal(&self) -> Option<Terminal> {
        let session_id = self.get_focused_session()?;
        self.terminals.borrow().get(&session_id).cloned()
    }

    /// Sets up drag-and-drop for all panes
    ///
    /// When a session is dropped on a pane, it will be shown in that pane.
    pub fn setup_drag_and_drop(&self) {
        // Setup drop target for initial pane
        self.setup_pane_drop_target_by_index(0);
    }

    /// Sets up click handlers for focus management on all panes
    pub fn setup_click_handlers(&self) {
        self.setup_pane_click_handler_by_index(0);
    }

    /// Sets up click handler for a specific pane by index
    fn setup_pane_click_handler_by_index(&self, index: usize) {
        let panes = self.panes.borrow();
        if index >= panes.len() {
            return;
        }
        let pane_id = panes[index].id();
        drop(panes);
        self.setup_pane_click_handler(pane_id);
    }

    /// Sets up click handler for a specific pane by ID
    pub fn setup_pane_click_handler(&self, pane_id: Uuid) {
        let panes = self.panes.borrow();
        let Some(pane) = panes.iter().find(|p| p.id() == pane_id) else {
            return;
        };

        let focused_pane = self.focused_pane.clone();
        let panes_clone = self.panes.clone();

        pane.setup_click_handler(move |clicked_pane_id| {
            // Update focused pane
            *focused_pane.borrow_mut() = Some(clicked_pane_id);

            // Update visual focus indicator
            let panes_ref = panes_clone.borrow();
            for p in panes_ref.iter() {
                let container = p.container();
                if p.id() == clicked_pane_id {
                    container.add_css_class("focused-pane");
                    container.remove_css_class("unfocused-pane");
                } else {
                    container.remove_css_class("focused-pane");
                    container.add_css_class("unfocused-pane");
                }
            }
        });
    }

    /// Sets up context menu for all panes with callbacks for actions
    #[allow(clippy::too_many_arguments)]
    pub fn setup_context_menus<F1, F2, F3>(
        &self,
        on_close_pane: F1,
        on_close_tab: F2,
        on_detach: F3,
    ) where
        F1: Fn(Uuid) + Clone + 'static,
        F2: Fn(Uuid) + Clone + 'static,
        F3: Fn(Uuid) + Clone + 'static,
    {
        self.setup_pane_context_menu_by_index(0, on_close_pane, on_close_tab, on_detach);
    }

    /// Sets up context menu for a specific pane by index
    fn setup_pane_context_menu_by_index<F1, F2, F3>(
        &self,
        index: usize,
        on_close_pane: F1,
        on_close_tab: F2,
        on_detach: F3,
    ) where
        F1: Fn(Uuid) + Clone + 'static,
        F2: Fn(Uuid) + Clone + 'static,
        F3: Fn(Uuid) + Clone + 'static,
    {
        let panes = self.panes.borrow();
        if index >= panes.len() {
            return;
        }
        let pane_id = panes[index].id();
        drop(panes);
        self.setup_pane_context_menu(pane_id, on_close_pane, on_close_tab, on_detach);
    }

    /// Sets up context menu for a specific pane by ID
    pub fn setup_pane_context_menu<F1, F2, F3>(
        &self,
        pane_id: Uuid,
        on_close_pane: F1,
        on_close_tab: F2,
        on_detach: F3,
    ) where
        F1: Fn(Uuid) + Clone + 'static,
        F2: Fn(Uuid) + Clone + 'static,
        F3: Fn(Uuid) + Clone + 'static,
    {
        let panes = self.panes.borrow();
        let Some(pane) = panes.iter().find(|p| p.id() == pane_id) else {
            return;
        };

        let panes_clone = self.panes.clone();
        let sessions_clone = self.sessions.clone();

        pane.setup_context_menu(move |ctx_pane_id| {
            let menu = gtk4::gio::Menu::new();

            // Get session in this pane
            let panes_ref = panes_clone.borrow();
            let session_id = panes_ref
                .iter()
                .find(|p| p.id() == ctx_pane_id)
                .and_then(|p| p.current_session());
            drop(panes_ref);

            if let Some(sid) = session_id {
                // Get session name for display
                let sessions_ref = sessions_clone.borrow();
                let session_name = sessions_ref
                    .get(&sid)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "Session".to_string());
                drop(sessions_ref);

                // Close tab action
                let close_tab_section = gtk4::gio::Menu::new();
                close_tab_section.append(
                    Some(&format!("Close \"{}\"", session_name)),
                    Some(&format!("win.close-tab-by-id('{}')", sid)),
                );
                menu.append_section(None, &close_tab_section);
            }

            // Close pane action (only if multiple panes)
            if panes_clone.borrow().len() > 1 {
                let pane_section = gtk4::gio::Menu::new();
                pane_section.append(Some("Close Pane"), Some("win.close-pane"));
                menu.append_section(None, &pane_section);
            }

            menu
        });

        // Store callbacks for later use (they're triggered via window actions)
        let _ = (on_close_pane, on_close_tab, on_detach);
    }

    /// Sets up drop target for a specific pane by index
    fn setup_pane_drop_target_by_index(&self, index: usize) {
        let panes = self.panes.borrow();
        if index >= panes.len() {
            return;
        }

        let pane_id = panes[index].id();
        drop(panes);

        self.setup_pane_drop_target(pane_id);
    }

    /// Sets up drop target for a specific pane by ID
    pub fn setup_pane_drop_target(&self, pane_id: Uuid) {
        let panes = self.panes.clone();
        let sessions = self.sessions.clone();
        let terminals = self.terminals.clone();
        let focused_pane = self.focused_pane.clone();

        let panes_ref = panes.borrow();
        let Some(pane) = panes_ref.iter().find(|p| p.id() == pane_id) else {
            return;
        };

        let panes_clone = panes.clone();
        let sessions_clone = sessions;
        let terminals_clone = terminals;
        let focused_pane_clone = focused_pane;

        pane.setup_drop_target(move |target_pane_id, session_id| {
            // Set focus to the target pane
            *focused_pane_clone.borrow_mut() = Some(target_pane_id);

            // Verify session exists in our local sessions
            let sessions_ref = sessions_clone.borrow();
            if !sessions_ref.contains_key(&session_id) {
                // Session not in split_view yet - this is expected for drag from notebook
                // The external callback should handle this case
                drop(sessions_ref);
                return;
            }

            // Get session info for external sessions
            let session_info = sessions_ref.get(&session_id).cloned();
            drop(sessions_ref);

            // Find the pane and show the session
            let mut panes_ref = panes_clone.borrow_mut();
            if let Some(pane) = panes_ref.iter_mut().find(|p| p.id() == target_pane_id) {
                // Get the terminal for this session
                let terminals_ref = terminals_clone.borrow();
                if let Some(terminal) = terminals_ref.get(&session_id) {
                    // Remove terminal from any previous parent
                    if let Some(parent) = terminal.parent() {
                        if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                            scrolled.set_child(None::<&gtk4::Widget>);
                        }
                    }

                    let scrolled = create_terminal_scrolled_window(terminal);
                    pane.set_content(&scrolled);
                    pane.set_current_session(Some(session_id));
                } else if let Some(info) = session_info {
                    // External session - show placeholder
                    let placeholder =
                        Self::create_external_session_placeholder(&info.name, &info.protocol);
                    pane.set_content(&placeholder);
                    pane.set_current_session(Some(session_id));
                }
            }
        });
    }

    /// Sets up drop target for a pane with external session lookup
    ///
    /// This version accepts a callback that can look up sessions from an external source
    /// (like `TerminalNotebook`) when the session is not found in the local sessions map.
    pub fn setup_pane_drop_target_with_callback<F>(&self, pane_id: Uuid, session_lookup: F)
    where
        F: Fn(Uuid) -> Option<(TerminalSession, Option<Terminal>)> + 'static,
    {
        self.setup_pane_drop_target_with_callbacks(pane_id, session_lookup, |_, _| {});
    }

    /// Sets up drop target for a pane with external session lookup and on_drop callback
    ///
    /// This version accepts two callbacks:
    /// - `session_lookup`: looks up sessions from an external source (like `TerminalNotebook`)
    /// - `on_drop`: called after a session is successfully dropped with (session_id, color_index)
    pub fn setup_pane_drop_target_with_callbacks<F, G>(
        &self,
        pane_id: Uuid,
        session_lookup: F,
        on_drop: G,
    ) where
        F: Fn(Uuid) -> Option<(TerminalSession, Option<Terminal>)> + 'static,
        G: Fn(Uuid, usize) + 'static,
    {
        let panes = self.panes.clone();
        let sessions = self.sessions.clone();
        let terminals = self.terminals.clone();
        let focused_pane = self.focused_pane.clone();

        let panes_ref = panes.borrow();
        let Some(pane) = panes_ref.iter().find(|p| p.id() == pane_id) else {
            return;
        };

        // Get the color index for this pane
        let pane_color_index = pane.color_index().unwrap_or(0);

        let panes_clone = panes.clone();
        let sessions_clone = sessions;
        let terminals_clone = terminals;
        let focused_pane_clone = focused_pane;
        let session_lookup = Rc::new(session_lookup);
        let on_drop = Rc::new(on_drop);

        pane.setup_drop_target(move |target_pane_id, session_id| {
            // Set focus to the target pane
            *focused_pane_clone.borrow_mut() = Some(target_pane_id);

            // Try to get session from local sessions first
            let mut session_info: Option<TerminalSession> = None;
            let mut terminal_widget: Option<Terminal> = None;

            {
                let sessions_ref = sessions_clone.borrow();
                if let Some(info) = sessions_ref.get(&session_id) {
                    session_info = Some(info.clone());
                }
            }

            {
                let terminals_ref = terminals_clone.borrow();
                if let Some(term) = terminals_ref.get(&session_id) {
                    terminal_widget = Some(term.clone());
                }
            }

            // If not found locally, try the external lookup (from notebook)
            if session_info.is_none() {
                if let Some((info, term)) = session_lookup(session_id) {
                    // Add to local sessions for future use
                    sessions_clone.borrow_mut().insert(session_id, info.clone());
                    if let Some(t) = &term {
                        terminals_clone.borrow_mut().insert(session_id, t.clone());
                    }
                    session_info = Some(info);
                    terminal_widget = term;
                }
            }

            // If still no session found, return
            let Some(info) = session_info else {
                return;
            };

            // Find the pane and show the session
            let mut panes_ref = panes_clone.borrow_mut();
            if let Some(pane) = panes_ref.iter_mut().find(|p| p.id() == target_pane_id) {
                if let Some(terminal) = terminal_widget {
                    // Remove terminal from any previous parent
                    if let Some(parent) = terminal.parent() {
                        if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                            scrolled.set_child(None::<&gtk4::Widget>);
                        }
                    }

                    let scrolled = create_terminal_scrolled_window(&terminal);
                    pane.set_content(&scrolled);
                    pane.set_current_session(Some(session_id));

                    // Call on_drop callback with session_id and color_index
                    on_drop(session_id, pane_color_index);
                } else {
                    // External session - show placeholder
                    let placeholder =
                        Self::create_external_session_placeholder(&info.name, &info.protocol);
                    pane.set_content(&placeholder);
                    pane.set_current_session(Some(session_id));

                    // Call on_drop callback with session_id and color_index
                    on_drop(session_id, pane_color_index);
                }
            }
        });
    }

    /// Shows a session in a specific pane (for drag-and-drop)
    #[must_use]
    pub fn show_session_in_pane(&self, pane_id: Uuid, session_id: Uuid) -> bool {
        // Verify session exists
        if !self.sessions.borrow().contains_key(&session_id) {
            return false;
        }

        // Find the pane and update its content
        let mut panes = self.panes.borrow_mut();
        let Some(pane) = panes.iter_mut().find(|p| p.id() == pane_id) else {
            return false;
        };

        // Get the terminal for this session
        let terminals_ref = self.terminals.borrow();
        if let Some(terminal) = terminals_ref.get(&session_id) {
            // Clone terminal reference before dropping borrow
            let terminal = terminal.clone();
            drop(terminals_ref);

            // Remove terminal from any previous parent first
            if let Some(parent) = terminal.parent() {
                if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                    scrolled.set_child(None::<&gtk4::Widget>);
                }
            }

            let scrolled = create_terminal_scrolled_window(&terminal);
            pane.set_content(&scrolled);
        } else {
            drop(terminals_ref);
            // No terminal (external session), show placeholder
            let session_info = self.sessions.borrow();
            let session = session_info.get(&session_id);
            let name = session.map_or("Unknown", |s| &s.name);
            let protocol = session.map_or("unknown", |s| &s.protocol);

            let placeholder = Self::create_external_session_placeholder(name, protocol);
            pane.set_content(&placeholder);
        }

        pane.set_current_session(Some(session_id));
        *self.focused_pane.borrow_mut() = Some(pane_id);
        true
    }

    /// Shows connection info content in the focused pane
    ///
    /// Displays detailed connection information in the main content area,
    /// replacing the Welcome screen. The info view will be replaced when
    /// switching to another session tab.
    pub fn show_info_content(&self, connection: &rustconn_core::Connection) {
        let focused_id = match *self.focused_pane.borrow() {
            Some(id) => id,
            None => return,
        };

        let mut panes = self.panes.borrow_mut();
        let Some(pane) = panes.iter_mut().find(|p| p.id() == focused_id) else {
            return;
        };

        let info_content = Self::create_info_content(connection);
        pane.set_content(&info_content);
        pane.set_current_session(None); // Info is not a session
    }

    /// Creates info content widget for a connection
    fn create_info_content(connection: &rustconn_core::Connection) -> GtkBox {
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_halign(gtk4::Align::Center);
        content.set_valign(gtk4::Align::Start);
        content.set_width_request(600);

        // Connection name header
        let name_label = gtk4::Label::builder()
            .label(&connection.name)
            .css_classes(["title-1"])
            .halign(gtk4::Align::Start)
            .build();
        content.append(&name_label);

        // Basic info section
        let basic_frame = Self::create_info_section("Basic Information");
        let basic_grid = gtk4::Grid::builder()
            .row_spacing(8)
            .column_spacing(16)
            .build();

        let mut row = 0;

        // Protocol
        Self::add_info_row(
            &basic_grid,
            row,
            "Protocol",
            &format!("{:?}", connection.protocol),
        );
        row += 1;

        // Host
        Self::add_info_row(
            &basic_grid,
            row,
            "Host",
            &format!("{}:{}", connection.host, connection.port),
        );
        row += 1;

        // Username
        if let Some(ref username) = connection.username {
            Self::add_info_row(&basic_grid, row, "Username", username);
            row += 1;
        }

        // Description - check both new field and legacy desc: tag
        let description: Option<&str> = connection.description.as_deref().or_else(|| {
            connection
                .tags
                .iter()
                .find(|t| t.starts_with("desc:"))
                .and_then(|t| t.strip_prefix("desc:"))
        });
        if let Some(desc) = description {
            if !desc.trim().is_empty() {
                Self::add_info_row(&basic_grid, row, "Description", desc);
                row += 1;
            }
        }

        // Tags (filter out desc: tags for backward compatibility)
        let display_tags: Vec<&str> = connection
            .tags
            .iter()
            .filter(|t| !t.starts_with("desc:"))
            .map(String::as_str)
            .collect();
        if !display_tags.is_empty() {
            Self::add_info_row(&basic_grid, row, "Tags", &display_tags.join(", "));
        }

        basic_frame.set_child(Some(&basic_grid));
        content.append(&basic_frame);

        // Protocol-specific section
        let protocol_frame = Self::create_info_section("Protocol Settings");
        let protocol_grid = gtk4::Grid::builder()
            .row_spacing(8)
            .column_spacing(16)
            .build();

        Self::add_protocol_info(&protocol_grid, &connection.protocol_config);

        protocol_frame.set_child(Some(&protocol_grid));
        content.append(&protocol_frame);

        // Custom properties section
        if !connection.custom_properties.is_empty() {
            let custom_frame = Self::create_info_section("Custom Properties");
            let custom_grid = gtk4::Grid::builder()
                .row_spacing(8)
                .column_spacing(16)
                .build();

            for (idx, prop) in connection.custom_properties.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                Self::add_info_row(&custom_grid, idx as i32, &prop.name, &prop.value);
            }

            custom_frame.set_child(Some(&custom_grid));
            content.append(&custom_frame);
        }

        // Timestamps section
        let time_frame = Self::create_info_section("Timestamps");
        let time_grid = gtk4::Grid::builder()
            .row_spacing(8)
            .column_spacing(16)
            .build();

        Self::add_info_row(
            &time_grid,
            0,
            "Created",
            &connection
                .created_at
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        );
        Self::add_info_row(
            &time_grid,
            1,
            "Modified",
            &connection
                .updated_at
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        );
        if let Some(last_connected) = connection.last_connected {
            Self::add_info_row(
                &time_grid,
                2,
                "Last Connected",
                &last_connected.format("%Y-%m-%d %H:%M:%S").to_string(),
            );
        }

        time_frame.set_child(Some(&time_grid));
        content.append(&time_frame);

        scroll.set_child(Some(&content));

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&scroll);
        container
    }

    /// Creates a framed section for info display
    fn create_info_section(title: &str) -> gtk4::Frame {
        let frame = gtk4::Frame::new(Some(title));
        frame.set_margin_top(8);
        if let Some(label) = frame.label_widget().and_downcast::<gtk4::Label>() {
            label.add_css_class("heading");
        }
        frame
    }

    /// Adds a label-value row to an info grid
    fn add_info_row(grid: &gtk4::Grid, row: i32, label_text: &str, value_text: &str) {
        let label = gtk4::Label::builder()
            .label(&format!("{label_text}:"))
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        let value = gtk4::Label::builder()
            .label(value_text)
            .halign(gtk4::Align::Start)
            .selectable(true)
            .wrap(true)
            .build();
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(&value, 1, row, 1, 1);
    }

    /// Adds protocol-specific information to the grid
    fn add_protocol_info(grid: &gtk4::Grid, config: &rustconn_core::ProtocolConfig) {
        use rustconn_core::ProtocolConfig;

        let mut row = 0;

        match config {
            ProtocolConfig::Ssh(ssh) => {
                Self::add_info_row(grid, row, "Auth Method", &format!("{:?}", ssh.auth_method));
                row += 1;
                if let Some(ref key_path) = ssh.key_path {
                    Self::add_info_row(grid, row, "Key Path", &key_path.display().to_string());
                    row += 1;
                }
                if let Some(ref proxy) = ssh.proxy_jump {
                    Self::add_info_row(grid, row, "Proxy Jump", proxy);
                    row += 1;
                }
                if ssh.agent_forwarding {
                    Self::add_info_row(grid, row, "Agent Forwarding", "Enabled");
                    row += 1;
                }
                if let Some(ref cmd) = ssh.startup_command {
                    Self::add_info_row(grid, row, "Startup Command", cmd);
                }
            }
            ProtocolConfig::Rdp(rdp) => {
                Self::add_info_row(grid, row, "Client Mode", &format!("{:?}", rdp.client_mode));
                row += 1;
                if let Some(ref res) = rdp.resolution {
                    Self::add_info_row(
                        grid,
                        row,
                        "Resolution",
                        &format!("{}x{}", res.width, res.height),
                    );
                    row += 1;
                }
                if let Some(depth) = rdp.color_depth {
                    Self::add_info_row(grid, row, "Color Depth", &format!("{depth} bit"));
                    row += 1;
                }
                if rdp.audio_redirect {
                    Self::add_info_row(grid, row, "Audio", "Enabled");
                }
            }
            ProtocolConfig::Vnc(vnc) => {
                Self::add_info_row(grid, row, "Client Mode", &format!("{:?}", vnc.client_mode));
                row += 1;
                if vnc.view_only {
                    Self::add_info_row(grid, row, "View Only", "Yes");
                    row += 1;
                }
                if let Some(ref encoding) = vnc.encoding {
                    Self::add_info_row(grid, row, "Encoding", encoding);
                }
            }
            ProtocolConfig::Spice(spice) => {
                if spice.tls_enabled {
                    Self::add_info_row(grid, row, "TLS", "Enabled");
                    row += 1;
                }
                if spice.usb_redirection {
                    Self::add_info_row(grid, row, "USB Redirection", "Enabled");
                    row += 1;
                }
                if spice.clipboard_enabled {
                    Self::add_info_row(grid, row, "Clipboard", "Enabled");
                }
            }
            ProtocolConfig::ZeroTrust(zt) => {
                Self::add_info_row(grid, row, "Provider", &format!("{:?}", zt.provider));
            }
        }
    }
}

impl Default for SplitTerminalView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_direction_to_orientation() {
        assert_eq!(
            SplitDirection::Horizontal.to_orientation(),
            Orientation::Vertical
        );
        assert_eq!(
            SplitDirection::Vertical.to_orientation(),
            Orientation::Horizontal
        );
    }

    #[test]
    fn test_terminal_pane_creation() {
        // Note: This test requires GTK to be initialized
        // In actual tests, we'd need to initialize GTK first
    }
}
