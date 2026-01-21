//! Bridge module providing legacy-compatible API over new split view system
//!
//! This module provides `SplitViewBridge` which implements the same API as the
//! legacy `SplitTerminalView` but uses the new `SplitViewAdapter` and
//! `TabSplitManager` internally.
//!
//! It also contains `SplitDirection` and `TerminalPane` types that were previously
//! in the legacy module, kept here for backward compatibility with existing code.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;
use vte4::Terminal;

use rustconn_core::split::{ColorPool, PanelId, SessionId};

use super::adapter::SplitViewAdapter;
use crate::terminal::TerminalSession;

/// Color palette for split panes
pub const SPLIT_PANE_COLORS: &[(&str, &str)] = &[
    ("split-color-blue", "tab-split-blue"),
    ("split-color-green", "tab-split-green"),
    ("split-color-orange", "tab-split-orange"),
    ("split-color-purple", "tab-split-purple"),
    ("split-color-cyan", "tab-split-cyan"),
    ("split-color-pink", "tab-split-pink"),
];

/// Returns the CSS class for a given color index
#[must_use]
pub fn get_split_color_class(color_index: usize) -> &'static str {
    SPLIT_PANE_COLORS
        .get(color_index % SPLIT_PANE_COLORS.len())
        .map_or("split-color-blue", |(class, _)| class)
}

/// Returns the CSS class for tab coloring
#[must_use]
pub fn get_tab_color_class(color_index: usize) -> &'static str {
    SPLIT_PANE_COLORS
        .get(color_index % SPLIT_PANE_COLORS.len())
        .map_or("tab-split-blue", |(_, tab_class)| tab_class)
}

/// Color values for split pane indicators (RGB)
/// These match the CSS colors defined in app.rs
pub const SPLIT_COLOR_VALUES: &[(u8, u8, u8)] = &[
    (0x35, 0x84, 0xe4), // Blue (#3584e4)
    (0x33, 0xd1, 0x7a), // Green (#33d17a)
    (0xff, 0x78, 0x00), // Orange (#ff7800)
    (0x91, 0x41, 0xac), // Purple (#9141ac)
    (0x00, 0xb4, 0xd8), // Cyan (#00b4d8)
    (0xf6, 0x61, 0x51), // Pink (#f66151)
];

/// Creates a colored circle icon for tab indicators
///
/// This function generates a small colored circle as a `gio::BytesIcon` that can
/// be used as a tab indicator icon. The color is determined by the color index.
///
/// # Arguments
///
/// * `color_index` - Index into the color palette (0-5, wraps around)
/// * `size` - Size of the icon in pixels (typically 16 for tab indicators)
///
/// # Returns
///
/// A `gio::Icon` containing a colored circle, or `None` if creation fails
#[must_use]
pub fn create_colored_circle_icon(color_index: usize, size: u32) -> Option<gtk4::gio::Icon> {
    let (r, g, b) = SPLIT_COLOR_VALUES
        .get(color_index % SPLIT_COLOR_VALUES.len())
        .copied()
        .unwrap_or((0x35, 0x84, 0xe4)); // Default to blue

    // Create RGBA pixel data for a filled circle
    let mut rgba_data = vec![0u8; (size * size * 4) as usize];
    let center = size as f32 / 2.0;
    let radius = center - 1.0; // Leave 1px margin

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = dx.hypot(dy);

            let idx = ((y * size + x) * 4) as usize;

            if distance <= radius {
                // Inside the circle - use the color
                // Apply anti-aliasing at the edge
                let alpha = if distance > radius - 1.0 {
                    ((radius - distance + 1.0) * 255.0) as u8
                } else {
                    255
                };
                rgba_data[idx] = r;
                rgba_data[idx + 1] = g;
                rgba_data[idx + 2] = b;
                rgba_data[idx + 3] = alpha;
            } else {
                // Outside the circle - transparent
                rgba_data[idx] = 0;
                rgba_data[idx + 1] = 0;
                rgba_data[idx + 2] = 0;
                rgba_data[idx + 3] = 0;
            }
        }
    }

    // Create GdkPixbuf from RGBA data
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
        &gtk4::glib::Bytes::from(&rgba_data),
        gtk4::gdk_pixbuf::Colorspace::Rgb,
        true,              // has_alpha
        8,                 // bits_per_sample
        size as i32,       // width
        size as i32,       // height
        (size * 4) as i32, // rowstride
    );

    // Save pixbuf to PNG bytes
    let png_bytes = pixbuf.save_to_bufferv("png", &[]).ok()?;

    // Create a BytesIcon from the PNG data
    let bytes = gtk4::glib::Bytes::from(&png_bytes);
    Some(gtk4::gio::BytesIcon::new(&bytes).upcast())
}

/// Returns the CSS indicator class for a given color index
#[must_use]
pub fn get_split_indicator_class(color_index: usize) -> String {
    let color_class = get_split_color_class(color_index);
    color_class.replace("split-color-", "split-indicator-")
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

    /// Converts to core split direction
    #[must_use]
    pub const fn to_core(self) -> rustconn_core::split::SplitDirection {
        match self {
            Self::Horizontal => rustconn_core::split::SplitDirection::Horizontal,
            Self::Vertical => rustconn_core::split::SplitDirection::Vertical,
        }
    }
}

/// A pane in the split terminal view
///
/// This struct is kept for backward compatibility with existing code that
/// uses the `panes_ref()` API for click handlers and context menus.
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

    /// Creates a new terminal pane with a specific ID
    #[must_use]
    pub fn new_with_id(id: Uuid) -> Self {
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
    #[must_use]
    pub fn with_color(color_index: usize) -> Self {
        let id = Uuid::new_v4();
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        Self {
            id,
            container,
            current_session: None,
            color_index: Some(color_index),
        }
    }

    /// Creates a new terminal pane with a specific ID and color index
    #[must_use]
    pub fn with_id_and_color(id: Uuid, color_index: usize) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        Self {
            id,
            container,
            current_session: None,
            color_index: Some(color_index),
        }
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

    /// Returns the color index for this pane
    #[must_use]
    pub const fn color_index(&self) -> Option<usize> {
        self.color_index
    }

    /// Sets the color index for this pane
    pub fn set_color_index(&mut self, index: usize) {
        self.color_index = Some(index);
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
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_halign(gtk4::Align::Fill);
        widget.set_valign(gtk4::Align::Fill);
        self.container.append(widget);
    }

    /// Sets up click handler for focus management using capture phase
    pub fn setup_click_handler<F>(&self, on_click: F)
    where
        F: Fn(Uuid) + 'static,
    {
        let click = gtk4::GestureClick::new();
        click.set_button(1); // Left click
        click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let pane_id = self.id;
        click.connect_pressed(move |gesture, _, _, _| {
            tracing::debug!("Pane click handler: clicked on pane {}", pane_id);
            on_click(pane_id);
            gesture.set_state(gtk4::EventSequenceState::None);
        });
        self.container.add_controller(click);
    }

    /// Sets up context menu (right-click) for the pane
    pub fn setup_context_menu<F>(&self, menu_builder: F)
    where
        F: Fn(Uuid) -> gtk4::gio::Menu + 'static,
    {
        let click = gtk4::GestureClick::new();
        click.set_button(3); // Right click
        click.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        let pane_id = self.id;
        let container = self.container.clone();

        click.connect_pressed(move |gesture, _, x, y| {
            let menu = menu_builder(pane_id);
            let popover = gtk4::PopoverMenu::from_model(Some(&menu));
            popover.set_parent(&container);
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.set_has_arrow(false);

            if let Some(root) = container.root() {
                if let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() {
                    let action_group = gtk4::gio::SimpleActionGroup::new();
                    let window_weak = window.downgrade();

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

                    popover.insert_action_group("win", Some(&action_group));
                }
            }

            popover.popup();
            gesture.set_state(gtk4::EventSequenceState::Claimed);

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
        F: Fn(Uuid, Uuid) + 'static,
    {
        let drop_target =
            gtk4::DropTarget::new(gtk4::glib::Type::STRING, gtk4::gdk::DragAction::MOVE);

        let pane_id = self.id;
        let container = self.container.clone();
        let container_for_enter = self.container.clone();
        let container_for_leave = self.container.clone();

        drop_target.connect_enter(move |_target, _x, _y| {
            container_for_enter.add_css_class("drop-target-highlight");
            gtk4::gdk::DragAction::MOVE
        });

        drop_target.connect_leave(move |_target| {
            container_for_leave.remove_css_class("drop-target-highlight");
        });

        drop_target.connect_drop(move |_target, value, _x, _y| {
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
}

impl Default for TerminalPane {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared sessions type
pub type SharedSessions = Rc<RefCell<HashMap<Uuid, TerminalSession>>>;
/// Shared terminals type
pub type SharedTerminals = Rc<RefCell<HashMap<Uuid, Terminal>>>;
/// Session color map type
pub type SessionColorMap = Rc<RefCell<HashMap<Uuid, usize>>>;
/// Shared color pool type for global color allocation across all split containers
pub type SharedColorPool = Rc<RefCell<ColorPool>>;
/// Container color type - the single color assigned to this split container
pub type ContainerColor = Rc<RefCell<Option<usize>>>;

/// Shared panel UUID map type for sharing between bridge and callbacks
type SharedPanelUuidMap = Rc<RefCell<HashMap<PanelId, Uuid>>>;

/// Bridge providing legacy-compatible API over new split view system
pub struct SplitViewBridge {
    /// The underlying adapter
    adapter: Rc<RefCell<SplitViewAdapter>>,
    /// Root container widget
    root: GtkBox,
    /// Shared sessions map
    sessions: SharedSessions,
    /// Shared terminals map
    terminals: SharedTerminals,
    /// Session color map
    session_colors: SessionColorMap,
    /// Color pool for allocation (shared across all split containers)
    color_pool: SharedColorPool,
    /// Container color - the single color assigned to this entire split container
    /// All panels/sessions in this container share this color
    container_color: ContainerColor,
    /// Panel ID to Uuid mapping (for legacy compatibility)
    /// Wrapped in Rc for sharing with callbacks
    panel_uuid_map: SharedPanelUuidMap,
    /// Uuid to Panel ID mapping
    uuid_panel_map: Rc<RefCell<HashMap<Uuid, PanelId>>>,
    /// Focused pane UUID (legacy compatibility)
    focused_pane_uuid: RefCell<Option<Uuid>>,
    /// Legacy panes for backward compatibility with panes_ref() API
    panes: Rc<RefCell<Vec<TerminalPane>>>,
}

impl SplitViewBridge {
    /// Creates a new split view bridge with its own color pool
    #[must_use]
    pub fn new() -> Self {
        Self::with_color_pool(Rc::new(RefCell::new(ColorPool::new())))
    }

    /// Creates a new split view bridge with a shared color pool
    ///
    /// This ensures that different split containers get different colors
    /// from the same pool, providing visual distinction between them.
    #[must_use]
    pub fn with_color_pool(color_pool: SharedColorPool) -> Self {
        let adapter = SplitViewAdapter::new();
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        // Add adapter widget to root
        root.append(adapter.widget());

        // Get initial panel ID and create UUID mapping
        let panel_ids = adapter.panel_ids();
        let mut panel_uuid_map = HashMap::new();
        let mut uuid_panel_map = HashMap::new();
        let mut panes = Vec::new();

        let focused_uuid = if let Some(&panel_id) = panel_ids.first() {
            let uuid = Uuid::new_v4();
            panel_uuid_map.insert(panel_id, uuid);
            uuid_panel_map.insert(uuid, panel_id);

            // Create initial legacy pane for backward compatibility
            let pane = TerminalPane::new_with_id(uuid);
            panes.push(pane);

            // Show welcome content in the initial panel
            let welcome = Self::create_welcome_content();
            adapter.set_panel_content(panel_id, &welcome);

            Some(uuid)
        } else {
            None
        };

        Self {
            adapter: Rc::new(RefCell::new(adapter)),
            root,
            sessions: Rc::new(RefCell::new(HashMap::new())),
            terminals: Rc::new(RefCell::new(HashMap::new())),
            session_colors: Rc::new(RefCell::new(HashMap::new())),
            color_pool,
            container_color: Rc::new(RefCell::new(None)),
            panel_uuid_map: Rc::new(RefCell::new(panel_uuid_map)),
            uuid_panel_map: Rc::new(RefCell::new(uuid_panel_map)),
            focused_pane_uuid: RefCell::new(focused_uuid),
            panes: Rc::new(RefCell::new(panes)),
        }
    }

    /// Creates with shared state
    #[must_use]
    pub fn with_shared_state(sessions: SharedSessions, terminals: SharedTerminals) -> Self {
        let mut bridge = Self::new();
        bridge.sessions = sessions;
        bridge.terminals = terminals;
        bridge
    }

    /// Creates with shared state and color pool
    #[must_use]
    pub fn with_shared_state_and_color_pool(
        sessions: SharedSessions,
        terminals: SharedTerminals,
        color_pool: SharedColorPool,
    ) -> Self {
        let mut bridge = Self::with_color_pool(color_pool);
        bridge.sessions = sessions;
        bridge.terminals = terminals;
        bridge
    }

    /// Returns the root widget
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.root
    }

    /// Returns the session color map
    #[must_use]
    pub fn session_colors(&self) -> SessionColorMap {
        Rc::clone(&self.session_colors)
    }

    /// Gets the color index for a session
    #[must_use]
    pub fn get_session_color(&self, session_id: Uuid) -> Option<usize> {
        self.session_colors.borrow().get(&session_id).copied()
    }

    /// Sets the color for a session
    pub fn set_session_color(&self, session_id: Uuid, color_index: usize) {
        self.session_colors
            .borrow_mut()
            .insert(session_id, color_index);
    }

    /// Clears the color for a session
    pub fn clear_session_color(&self, session_id: Uuid) {
        self.session_colors.borrow_mut().remove(&session_id);
    }

    /// Returns the container color for this split view
    ///
    /// All panels/sessions in this container share this single color.
    /// Returns `None` if no color has been allocated yet (before first split).
    #[must_use]
    pub fn get_container_color(&self) -> Option<usize> {
        *self.container_color.borrow()
    }

    /// Allocates and sets the container color if not already set
    ///
    /// This should be called on the first split to assign a unique color
    /// to this container. All subsequent panels will use this same color.
    fn ensure_container_color(&self) -> usize {
        let mut container_color = self.container_color.borrow_mut();
        if let Some(color) = *container_color {
            color
        } else {
            let color = usize::from(self.color_pool.borrow_mut().allocate().index());
            *container_color = Some(color);
            tracing::debug!(
                "ensure_container_color: allocated container color {} for this bridge",
                color
            );
            color
        }
    }

    /// Returns shared sessions reference
    #[must_use]
    pub fn shared_sessions(&self) -> SharedSessions {
        Rc::clone(&self.sessions)
    }

    /// Returns shared terminals reference
    #[must_use]
    pub fn shared_terminals(&self) -> SharedTerminals {
        Rc::clone(&self.terminals)
    }

    /// Returns the number of panes
    #[must_use]
    pub fn pane_count(&self) -> usize {
        self.adapter.borrow().panel_count()
    }

    /// Returns all pane UUIDs (legacy compatibility)
    #[must_use]
    pub fn pane_ids(&self) -> Vec<Uuid> {
        self.panel_uuid_map.borrow().values().copied().collect()
    }

    /// Returns the focused pane UUID
    #[must_use]
    pub fn focused_pane_id(&self) -> Option<Uuid> {
        *self.focused_pane_uuid.borrow()
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

    /// Returns true if any pane has an active session
    #[must_use]
    pub fn has_active_sessions(&self) -> bool {
        let adapter = self.adapter.borrow();
        adapter
            .panel_ids()
            .iter()
            .any(|&pid| adapter.get_panel_session(pid).is_some())
    }

    /// Adds a session to the shared session list
    pub fn add_session(&self, session: TerminalSession, terminal: Option<Terminal>) {
        let session_id = session.id;
        tracing::debug!(
            "add_session: session_id={}, has_terminal={}",
            session_id,
            terminal.is_some()
        );
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

    /// Returns the focused pane's current session
    #[must_use]
    pub fn get_focused_session(&self) -> Option<Uuid> {
        let focused_uuid = (*self.focused_pane_uuid.borrow())?;
        let panel_id = *self.uuid_panel_map.borrow().get(&focused_uuid)?;
        let adapter = self.adapter.borrow();
        adapter.get_panel_session(panel_id).map(|sid| sid.as_uuid())
    }

    /// Gets the session displayed in a specific pane
    ///
    /// This uses the pane's `current_session` field which is updated when
    /// sessions are moved to panels via `move_session_to_panel_with_terminal`
    /// or `show_session`.
    #[must_use]
    pub fn get_pane_session(&self, pane_uuid: Uuid) -> Option<Uuid> {
        self.panes
            .borrow()
            .iter()
            .find(|p| p.id() == pane_uuid)
            .and_then(|p| p.current_session())
    }

    /// Gets the color index for a specific pane by its UUID
    #[must_use]
    pub fn get_pane_color(&self, pane_uuid: Uuid) -> Option<usize> {
        self.panes
            .borrow()
            .iter()
            .find(|p| p.id() == pane_uuid)
            .and_then(|p| p.color_index())
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

    /// Returns true if a session is displayed in any pane
    #[must_use]
    pub fn is_session_displayed(&self, session_id: Uuid) -> bool {
        let adapter = self.adapter.borrow();
        let session = SessionId::from_uuid(session_id);
        adapter
            .panel_ids()
            .iter()
            .any(|&pid| adapter.get_panel_session(pid) == Some(session))
    }

    /// Splits the focused pane
    ///
    /// Returns (new_pane_uuid, new_pane_color_index, original_pane_color_index)
    #[must_use]
    pub fn split(&self, direction: SplitDirection) -> Option<(Uuid, usize, usize)> {
        self.split_with_close_callback(direction, || {})
    }

    /// Splits with a close callback
    ///
    /// Returns (new_pane_uuid, container_color_index, container_color_index)
    /// Both color indices are the same - the container's color.
    /// All panels in this split container share the same color.
    pub fn split_with_close_callback<F>(
        &self,
        direction: SplitDirection,
        _on_close: F,
    ) -> Option<(Uuid, usize, usize)>
    where
        F: Fn() + 'static,
    {
        // Get the focused (original) pane before splitting
        let original_uuid = (*self.focused_pane_uuid.borrow())?;
        let _original_panel_id = *self.uuid_panel_map.borrow().get(&original_uuid)?;

        let mut adapter = self.adapter.borrow_mut();
        let new_panel_id = adapter.split(direction.to_core()).ok()?;
        drop(adapter); // Release borrow before modifying other state

        // Ensure container has a color allocated (only allocates once on first split)
        let container_color = self.ensure_container_color();

        tracing::debug!(
            "split_with_close_callback: using container color {} for all panels in this bridge",
            container_color
        );

        // Set the container color on all panes (both existing and new)
        {
            let mut panes = self.panes.borrow_mut();
            for pane in panes.iter_mut() {
                pane.set_color_index(container_color);
            }
        }

        // Create UUID mapping for new panel
        let new_uuid = Uuid::new_v4();
        self.panel_uuid_map
            .borrow_mut()
            .insert(new_panel_id, new_uuid);
        self.uuid_panel_map
            .borrow_mut()
            .insert(new_uuid, new_panel_id);

        // Create legacy pane for backward compatibility with the container color
        let new_pane = TerminalPane::with_id_and_color(new_uuid, container_color);
        self.panes.borrow_mut().push(new_pane);

        // Focus stays on original pane (where session will be displayed)
        // This is already the case since we didn't change focused_pane_uuid

        // Restore terminal content in all panels after rebuild_widgets()
        // This is critical: adapter.split() calls rebuild_widgets() which recreates
        // all panel widgets with "Loading..." placeholders for occupied panels.
        // We need to restore the actual terminal content from self.terminals.
        self.restore_panel_contents();

        // Return the same color for both - all panels share the container color
        Some((new_uuid, container_color, container_color))
    }

    /// Shows a session in the focused pane
    ///
    /// This method places a session in the currently focused pane and sets
    /// the session's color to match the pane's color for consistent tab coloring.
    pub fn show_session(&self, session_id: Uuid) -> Result<(), String> {
        tracing::debug!("show_session: session_id={}", session_id);

        let focused_uuid = self.focused_pane_uuid.borrow().ok_or("No focused pane")?;
        let panel_id = *self
            .uuid_panel_map
            .borrow()
            .get(&focused_uuid)
            .ok_or("Panel not found")?;

        tracing::debug!(
            "show_session: focused_uuid={}, panel_id={}",
            focused_uuid,
            panel_id
        );

        let session = SessionId::from_uuid(session_id);
        self.adapter
            .borrow_mut()
            .place_in_panel(panel_id, session)
            .map_err(|e| e.to_string())?;

        // Update the pane's current_session tracking and get pane color
        let pane_color = {
            let mut panes = self.panes.borrow_mut();
            if let Some(pane) = panes.iter_mut().find(|p| p.id() == focused_uuid) {
                pane.set_current_session(Some(session_id));
                pane.color_index()
            } else {
                None
            }
        };

        // Set session color to match pane color for consistent tab coloring
        if let Some(color_index) = pane_color {
            self.set_session_color(session_id, color_index);
        }

        // Set terminal content if available
        if let Some(terminal) = self.terminals.borrow().get(&session_id).cloned() {
            tracing::debug!(
                "show_session: found terminal for session {}, setting panel content",
                session_id
            );
            // Remove terminal from any previous parent first
            // This is critical - GTK widgets can only have one parent
            Self::detach_terminal_from_parent(&terminal);

            let scrolled = Self::create_terminal_scrolled_window(&terminal);
            self.adapter.borrow().set_panel_content(panel_id, &scrolled);

            // Ensure terminal is visible
            terminal.set_visible(true);
        } else {
            tracing::warn!(
                "show_session: NO terminal found for session {} in self.terminals (available: {:?})",
                session_id,
                self.terminals.borrow().keys().collect::<Vec<_>>()
            );
        }

        Ok(())
    }

    /// Reparents a terminal from TabView back to its split pane
    ///
    /// This is called when switching to a tab that has a split_color assigned,
    /// meaning the session belongs to a split view. The terminal needs to be
    /// moved from the TabView page back into the appropriate split pane.
    pub fn reparent_terminal_to_split(&self, session_id: Uuid) -> Result<(), String> {
        let session = SessionId::from_uuid(session_id);

        // First, check if session is already displayed in a panel
        let adapter = self.adapter.borrow();
        let existing_panel = adapter
            .panel_ids()
            .into_iter()
            .find(|&pid| adapter.get_panel_session(pid) == Some(session));
        drop(adapter);

        let panel_id = if let Some(pid) = existing_panel {
            // Session is already in a panel, just need to reparent terminal
            pid
        } else {
            // Session is not in any panel - need to find panel by color and place session
            let color_index = self
                .get_session_color(session_id)
                .ok_or("Session has no color assigned")?;

            // Find pane with matching color
            let panes = self.panes.borrow();
            let pane_uuid = panes
                .iter()
                .find(|p| p.color_index() == Some(color_index))
                .map(|p| p.id())
                .ok_or("No pane found with matching color")?;
            drop(panes);

            // Get panel ID from UUID
            let panel_id = *self
                .uuid_panel_map
                .borrow()
                .get(&pane_uuid)
                .ok_or("Panel not found for pane UUID")?;

            // Place session in the panel
            self.adapter
                .borrow_mut()
                .place_in_panel(panel_id, session)
                .map_err(|e| e.to_string())?;

            panel_id
        };

        // Get terminal for this session
        let terminal = self
            .terminals
            .borrow()
            .get(&session_id)
            .cloned()
            .ok_or("Terminal not found")?;

        // Detach from current parent
        Self::detach_terminal_from_parent(&terminal);

        // Create scrolled window and set as panel content
        let scrolled = Self::create_terminal_scrolled_window(&terminal);
        self.adapter.borrow().set_panel_content(panel_id, &scrolled);

        // Ensure terminal is visible
        terminal.set_visible(true);

        Ok(())
    }

    /// Detaches a terminal from its current parent widget
    fn detach_terminal_from_parent(terminal: &Terminal) {
        if let Some(parent) = terminal.parent() {
            if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                scrolled.set_child(None::<&gtk4::Widget>);
            } else if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                box_widget.remove(terminal);
            }
        }
    }

    /// Creates a scrolled window for a terminal
    fn create_terminal_scrolled_window(terminal: &Terminal) -> gtk4::ScrolledWindow {
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(terminal)
            .build()
    }

    /// Clears a session from all panes
    pub fn clear_session_from_panes(&self, session_id: Uuid) {
        let session = SessionId::from_uuid(session_id);
        let adapter = self.adapter.borrow_mut();

        // Find panel with this session and clear it
        for panel_id in adapter.panel_ids() {
            if adapter.get_panel_session(panel_id) == Some(session) {
                adapter.clear_panel(panel_id);

                // Also clear current_session on the corresponding pane
                if let Some(&pane_uuid) = self.panel_uuid_map.borrow().get(&panel_id) {
                    let mut panes = self.panes.borrow_mut();
                    if let Some(pane) = panes.iter_mut().find(|p| p.id() == pane_uuid) {
                        pane.set_current_session(None);
                    }
                }
                break;
            }
        }

        self.clear_session_color(session_id);
        self.remove_session(session_id);
    }

    /// Closes a session from panes with auto-cleanup
    ///
    /// Returns `true` if the split view should be closed (no panels remain or
    /// this was the last session and the split should be closed per Requirement 13.3).
    #[must_use]
    pub fn close_session_from_panes(&self, session_id: Uuid) -> bool {
        let session = SessionId::from_uuid(session_id);
        let mut adapter = self.adapter.borrow_mut();

        // Find panel with this session
        let panel_to_remove = adapter
            .panel_ids()
            .into_iter()
            .find(|&pid| adapter.get_panel_session(pid) == Some(session));

        self.clear_session_color(session_id);
        self.remove_session(session_id);

        if let Some(panel_id) = panel_to_remove {
            // Try to remove the panel
            let _ = adapter.remove_panel(panel_id);

            // Remove from UUID maps
            if let Some(uuid) = self.panel_uuid_map.borrow_mut().remove(&panel_id) {
                self.uuid_panel_map.borrow_mut().remove(&uuid);
                // Remove from panes list
                self.panes.borrow_mut().retain(|p| p.id() != uuid);
            }
        }

        // Check remaining state
        let remaining_panels = adapter.panel_ids();
        let no_panels = remaining_panels.is_empty();
        let no_sessions = !remaining_panels
            .iter()
            .any(|&pid| adapter.get_panel_session(pid).is_some());

        // Release container color only when closing the entire split view
        if no_panels || no_sessions {
            if let Some(color) = *self.container_color.borrow() {
                self.color_pool
                    .borrow_mut()
                    .release(rustconn_core::split::ColorId::new(color as u8));
                *self.container_color.borrow_mut() = None;
            }
        }

        // Per Requirement 13.3: When the last remaining Panel in a Split_Container
        // is closed, close the parent Root_Tab. We signal this by returning true
        // when no panels remain OR when no sessions remain (the split should close).
        no_panels || no_sessions
    }

    /// Closes the focused pane
    ///
    /// Returns `Ok(true)` if the split view should be closed (no panels remain),
    /// `Ok(false)` if there are still panels remaining.
    pub fn close_pane(&self) -> Result<bool, String> {
        let focused_uuid = self.focused_pane_uuid.borrow().ok_or("No focused pane")?;
        let panel_id = *self
            .uuid_panel_map
            .borrow()
            .get(&focused_uuid)
            .ok_or("Panel not found")?;

        // Get the session in this pane before closing (for color cleanup)
        let session_in_pane = {
            let adapter = self.adapter.borrow();
            adapter.get_panel_session(panel_id).map(|s| s.as_uuid())
        };

        // Clear session color if there was a session
        if let Some(session_id) = session_in_pane {
            self.clear_session_color(session_id);
        }

        let mut adapter = self.adapter.borrow_mut();
        adapter.remove_panel(panel_id).map_err(|e| e.to_string())?;

        // Remove from UUID maps
        self.panel_uuid_map.borrow_mut().remove(&panel_id);
        self.uuid_panel_map.borrow_mut().remove(&focused_uuid);

        // Remove from panes list
        self.panes.borrow_mut().retain(|p| p.id() != focused_uuid);

        // Check if this was the last panel
        let remaining_panels = adapter.panel_ids();
        let should_close_split = remaining_panels.is_empty();

        // Release container color only when closing the entire split view
        if should_close_split {
            if let Some(color) = *self.container_color.borrow() {
                self.color_pool
                    .borrow_mut()
                    .release(rustconn_core::split::ColorId::new(color as u8));
                *self.container_color.borrow_mut() = None;
            }
        }

        // Update focused pane to first available
        if let Some(&new_panel_id) = remaining_panels.first() {
            if let Some(&new_uuid) = self.panel_uuid_map.borrow().get(&new_panel_id) {
                *self.focused_pane_uuid.borrow_mut() = Some(new_uuid);
            }
        } else {
            *self.focused_pane_uuid.borrow_mut() = None;
        }

        Ok(should_close_split)
    }

    /// Restores terminal content to all panels that have sessions
    ///
    /// This is called after closing a pane to ensure remaining panels
    /// display their terminal content properly instead of placeholders.
    pub fn restore_panel_contents(&self) {
        let adapter = self.adapter.borrow();
        let panel_ids = adapter.panel_ids();

        tracing::debug!(
            "restore_panel_contents: restoring {} panels, terminals count: {}",
            panel_ids.len(),
            self.terminals.borrow().len()
        );

        for panel_id in panel_ids {
            if let Some(session_id) = adapter.get_panel_session(panel_id) {
                let session_uuid = session_id.as_uuid();

                tracing::debug!(
                    "restore_panel_contents: panel {} has session {}, looking for terminal",
                    panel_id,
                    session_uuid
                );

                // Get terminal for this session
                if let Some(terminal) = self.terminals.borrow().get(&session_uuid).cloned() {
                    // Detach from current parent
                    Self::detach_terminal_from_parent(&terminal);

                    // Create scrolled window and set as panel content
                    let scrolled = Self::create_terminal_scrolled_window(&terminal);
                    adapter.set_panel_content(panel_id, &scrolled);

                    // Ensure terminal is visible
                    terminal.set_visible(true);

                    tracing::debug!(
                        "restore_panel_contents: restored terminal for session {} in panel {}",
                        session_uuid,
                        panel_id
                    );
                } else {
                    tracing::warn!(
                        "restore_panel_contents: no terminal found for session {} in panel {}",
                        session_uuid,
                        panel_id
                    );
                }
            } else {
                tracing::debug!("restore_panel_contents: panel {} has no session", panel_id);
            }
        }
    }

    /// Returns true if the split view has no active sessions in any pane
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let adapter = self.adapter.borrow();
        !adapter
            .panel_ids()
            .iter()
            .any(|&pid| adapter.get_panel_session(pid).is_some())
    }

    /// Returns true if the split view has only one panel (not actually split)
    #[must_use]
    pub fn is_single_panel(&self) -> bool {
        self.adapter.borrow().panel_count() <= 1
    }

    /// Focuses the next pane
    ///
    /// Cycles through panes and updates both internal tracking and visual styling.
    pub fn focus_next_pane(&self) -> Result<(), String> {
        let pane_uuids: Vec<Uuid> = self.pane_ids();
        if pane_uuids.is_empty() {
            return Err("No panes".to_string());
        }

        let current = self.focused_pane_uuid.borrow();
        let current_idx = current
            .and_then(|uuid| pane_uuids.iter().position(|&u| u == uuid))
            .unwrap_or(0);

        let next_idx = (current_idx + 1) % pane_uuids.len();
        let next_uuid = pane_uuids[next_idx];

        // Use focus_pane to update both internal state and visual styling
        drop(current); // Release borrow before calling focus_pane
        self.focus_pane(next_uuid)
    }

    /// Focuses a specific pane
    ///
    /// Updates both the internal focused pane tracking and the adapter's
    /// visual focus styling (adds `focused-panel` CSS class).
    pub fn focus_pane(&self, pane_uuid: Uuid) -> Result<(), String> {
        let panel_id = *self
            .uuid_panel_map
            .borrow()
            .get(&pane_uuid)
            .ok_or("Pane not found")?;

        *self.focused_pane_uuid.borrow_mut() = Some(pane_uuid);

        // Update adapter focus styling
        self.adapter
            .borrow_mut()
            .set_focus(panel_id)
            .map_err(|e| e.to_string())
    }

    /// Sets up drop target for a pane with callbacks
    ///
    /// When a session is dropped onto this pane:
    /// 1. `get_session_info` is called to get the session info and terminal
    /// 2. The session is placed in the panel
    /// 3. `on_drop` is called with the session ID and the pane's color index
    ///
    /// The pane's color index is used so that the tab color matches the pane color.
    pub fn setup_pane_drop_target_with_callbacks<F, G>(
        &self,
        pane_uuid: Uuid,
        get_session_info: F,
        on_drop: G,
    ) where
        F: Fn(Uuid) -> Option<(TerminalSession, Option<Terminal>)> + 'static,
        G: Fn(Uuid, usize) + 'static,
    {
        let panel_id = match self.uuid_panel_map.borrow().get(&pane_uuid) {
            Some(&id) => id,
            None => return,
        };

        let Some(widget) = self.adapter.borrow().get_panel_widget(panel_id) else {
            return;
        };

        // Create drop target that accepts string data (session ID as string)
        let drop_target =
            gtk4::DropTarget::new(gtk4::glib::Type::STRING, gtk4::gdk::DragAction::MOVE);

        // Clone references for callbacks
        let widget_for_enter = widget.clone();
        let widget_for_leave = widget.clone();
        let widget_for_drop = widget.clone();
        let adapter_rc = Rc::clone(&self.adapter);
        let panes_rc = Rc::clone(&self.panes);
        let sessions_rc = Rc::clone(&self.sessions);
        let terminals_rc = Rc::clone(&self.terminals);
        let focused_pane_uuid = Rc::new(RefCell::new(pane_uuid));

        // Visual feedback on enter
        drop_target.connect_enter(move |_target, _x, _y| {
            widget_for_enter.add_css_class("drop-target-highlight");
            gtk4::gdk::DragAction::MOVE
        });

        // Remove highlight on leave
        drop_target.connect_leave(move |_target| {
            widget_for_leave.remove_css_class("drop-target-highlight");
        });

        // Handle the drop
        let get_session_info = Rc::new(get_session_info);
        let on_drop = Rc::new(on_drop);

        drop_target.connect_drop(move |_target, value, _x, _y| {
            widget_for_drop.remove_css_class("drop-target-highlight");

            // Parse session ID from drop data
            let drag_data = match value.get::<String>() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to get string from drop value: {e}");
                    return false;
                }
            };

            // Skip group drops
            if drag_data.starts_with("group:") {
                tracing::debug!("Groups cannot be dropped on split panels");
                return false;
            }

            // Parse session ID (format: "uuid" or "conn:uuid" for sidebar items)
            let session_uuid = if let Some(conn_id_str) = drag_data.strip_prefix("conn:") {
                // Sidebar connection - we can't handle this here, need to create session first
                tracing::debug!(
                    "Sidebar connection {} dropped - not handled by this drop target",
                    conn_id_str
                );
                return false;
            } else {
                match uuid::Uuid::parse_str(&drag_data) {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        tracing::warn!("Failed to parse session ID from drop data: {e}");
                        return false;
                    }
                }
            };

            // Get session info and terminal using the callback
            let Some((session_info, terminal_opt)) = get_session_info(session_uuid) else {
                tracing::warn!("Failed to get session info for {}", session_uuid);
                return false;
            };

            // Add session to our internal maps
            sessions_rc.borrow_mut().insert(session_uuid, session_info);
            if let Some(ref terminal) = terminal_opt {
                terminals_rc
                    .borrow_mut()
                    .insert(session_uuid, terminal.clone());
            }

            // Place session in panel
            let session = rustconn_core::split::SessionId::from_uuid(session_uuid);
            if let Err(e) = adapter_rc.borrow_mut().place_in_panel(panel_id, session) {
                tracing::warn!("Failed to place session in panel: {e}");
                return false;
            }

            // Display terminal in panel if available
            if let Some(terminal) = terminal_opt {
                Self::detach_terminal_from_parent(&terminal);
                let scrolled = Self::create_terminal_scrolled_window(&terminal);
                adapter_rc.borrow().set_panel_content(panel_id, &scrolled);
                terminal.set_visible(true);
            }

            // Update pane's current_session
            {
                let mut panes = panes_rc.borrow_mut();
                if let Some(pane) = panes
                    .iter_mut()
                    .find(|p| p.id() == *focused_pane_uuid.borrow())
                {
                    pane.set_current_session(Some(session_uuid));
                }
            }

            // Get the pane's color index and call the on_drop callback
            let color_index = {
                let panes = panes_rc.borrow();
                panes
                    .iter()
                    .find(|p| p.id() == *focused_pane_uuid.borrow())
                    .and_then(|p| p.color_index())
            };

            if let Some(color) = color_index {
                tracing::debug!(
                    "Drop: session {} placed in pane {} with color {}",
                    session_uuid,
                    *focused_pane_uuid.borrow(),
                    color
                );
                on_drop(session_uuid, color);
            } else {
                tracing::warn!(
                    "Drop: pane {} has no color assigned",
                    *focused_pane_uuid.borrow()
                );
            }

            true
        });

        widget.add_controller(drop_target);
    }

    /// Shows info content in the focused pane
    pub fn show_info_content(&self, connection: &rustconn_core::Connection) {
        if let Some(focused_uuid) = *self.focused_pane_uuid.borrow() {
            if let Some(&panel_id) = self.uuid_panel_map.borrow().get(&focused_uuid) {
                let adapter = self.adapter.borrow();
                let info_content = Self::create_info_content(connection);
                adapter.set_panel_content(panel_id, &info_content);
            }
        }
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
        let basic_frame = adw::PreferencesGroup::builder()
            .title("Basic Information")
            .build();

        // Protocol row
        let protocol_row = adw::ActionRow::builder()
            .title("Protocol")
            .subtitle(&format!("{:?}", connection.protocol))
            .build();
        basic_frame.add(&protocol_row);

        // Host row
        let host_row = adw::ActionRow::builder()
            .title("Host")
            .subtitle(&format!("{}:{}", connection.host, connection.port))
            .build();
        basic_frame.add(&host_row);

        // Username row
        if let Some(ref username) = connection.username {
            let username_row = adw::ActionRow::builder()
                .title("Username")
                .subtitle(username)
                .build();
            basic_frame.add(&username_row);
        }

        content.append(&basic_frame);

        // Wrap in scrolled window
        scroll.set_child(Some(&content));

        let wrapper = GtkBox::new(Orientation::Vertical, 0);
        wrapper.set_hexpand(true);
        wrapper.set_vexpand(true);
        wrapper.append(&scroll);
        wrapper
    }

    /// Resets the split view to initial state
    #[must_use]
    pub fn reset(&self) -> Vec<Uuid> {
        let displayed_sessions: Vec<Uuid> = self
            .adapter
            .borrow()
            .panel_ids()
            .iter()
            .filter_map(|&pid| {
                self.adapter
                    .borrow()
                    .get_panel_session(pid)
                    .map(|s| s.as_uuid())
            })
            .collect();

        // Release container color back to pool
        if let Some(color) = *self.container_color.borrow() {
            self.color_pool
                .borrow_mut()
                .release(rustconn_core::split::ColorId::new(color as u8));
        }

        // Clear all state
        *self.container_color.borrow_mut() = None;
        self.session_colors.borrow_mut().clear();
        self.sessions.borrow_mut().clear();
        self.terminals.borrow_mut().clear();
        self.panel_uuid_map.borrow_mut().clear();
        self.uuid_panel_map.borrow_mut().clear();
        self.panes.borrow_mut().clear();

        // Create new adapter
        let new_adapter = SplitViewAdapter::new();

        // Remove old widget and add new one
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        self.root.append(new_adapter.widget());

        // Set up initial panel mapping and pane
        if let Some(&panel_id) = new_adapter.panel_ids().first() {
            let uuid = Uuid::new_v4();
            self.panel_uuid_map.borrow_mut().insert(panel_id, uuid);
            self.uuid_panel_map.borrow_mut().insert(uuid, panel_id);
            *self.focused_pane_uuid.borrow_mut() = Some(uuid);

            // Create initial legacy pane
            let pane = TerminalPane::new_with_id(uuid);
            self.panes.borrow_mut().push(pane);
        }

        *self.adapter.borrow_mut() = new_adapter;

        displayed_sessions
    }

    /// Returns a reference to panes for external setup
    #[must_use]
    pub fn panes_ref(&self) -> std::cell::Ref<'_, Vec<TerminalPane>> {
        self.panes.borrow()
    }

    /// Returns a clone of panes Rc for callbacks
    #[must_use]
    pub fn panes_ref_clone(&self) -> Rc<RefCell<Vec<TerminalPane>>> {
        Rc::clone(&self.panes)
    }

    /// Returns the shared focused pane reference for callbacks
    ///
    /// This returns a clone of the Rc, so callbacks can update the actual
    /// focused pane state in the bridge.
    #[must_use]
    pub fn focused_pane_ref(&self) -> Rc<RefCell<Option<Uuid>>> {
        // Return a clone of the Rc to share the actual state
        // Note: focused_pane_uuid is stored as RefCell, we need to wrap it in Rc
        // For now, we create a new Rc but this should be refactored to store as Rc<RefCell<>>
        // TODO: Refactor focused_pane_uuid to be Rc<RefCell<Option<Uuid>>> for proper sharing
        Rc::new(RefCell::new(*self.focused_pane_uuid.borrow()))
    }

    /// Updates the focused pane UUID and visual styling
    ///
    /// This should be called by click handlers to update the bridge's focus state.
    /// It also updates the visual focus styling on the adapter's panel widgets.
    ///
    /// Note: This method handles all focus styling updates. Click handlers should
    /// NOT manually add/remove CSS classes - just call this method.
    pub fn set_focused_pane(&self, pane_uuid: Option<Uuid>) {
        *self.focused_pane_uuid.borrow_mut() = pane_uuid;

        // Update the adapter's focus styling (this updates the actual visible widgets)
        if let Some(uuid) = pane_uuid {
            if let Some(&panel_id) = self.uuid_panel_map.borrow().get(&uuid) {
                if let Err(e) = self.adapter.borrow_mut().set_focus(panel_id) {
                    tracing::warn!("Failed to set adapter focus: {}", e);
                }
            }
        }
    }

    /// Returns the panel widget for a pane UUID
    ///
    /// This returns the actual visible GTK widget from the adapter, not the
    /// legacy `TerminalPane` container. Use this for focus styling updates.
    #[must_use]
    pub fn get_panel_widget(&self, pane_uuid: Uuid) -> Option<gtk4::Box> {
        let panel_id = *self.uuid_panel_map.borrow().get(&pane_uuid)?;
        self.adapter.borrow().get_panel_widget(panel_id)
    }

    /// Gets the panel ID for a given pane UUID.
    ///
    /// This is useful when you need to interact with the adapter using panel IDs
    /// but only have the pane UUID.
    #[must_use]
    pub fn get_panel_id_for_uuid(&self, pane_uuid: Uuid) -> Option<PanelId> {
        self.uuid_panel_map.borrow().get(&pane_uuid).copied()
    }

    /// Sets focus on a panel via the adapter.
    ///
    /// This updates the visual focus styling in the adapter.
    ///
    /// # Errors
    ///
    /// Returns an error if the panel is not found.
    pub fn adapter_set_focus(&self, panel_id: PanelId) -> Result<(), String> {
        self.adapter
            .borrow_mut()
            .set_focus(panel_id)
            .map_err(|e| e.to_string())
    }

    /// Sets up click handlers for ALL panels in the split view.
    ///
    /// This should be called after a split operation to ensure all panels
    /// (both original and new) have click handlers for focus management.
    ///
    /// The click handler:
    /// 1. Updates the bridge's focused pane state
    /// 2. Updates the adapter's visual focus styling
    /// 3. Optionally switches to the tab containing the clicked pane's session
    ///
    /// # Arguments
    ///
    /// * `on_click` - A closure that receives the pane UUID when a panel is clicked.
    ///   The closure should handle tab switching if needed.
    pub fn setup_all_panel_click_handlers<F>(&self, on_click: F)
    where
        F: Fn(Uuid) + Clone + 'static,
    {
        let adapter = self.adapter.borrow();
        let panel_uuid_map = self.panel_uuid_map.borrow();

        for panel_id in adapter.panel_ids() {
            if panel_uuid_map.get(&panel_id).is_some() {
                let uuid_panel_map = Rc::clone(&self.uuid_panel_map);
                let on_click_clone = on_click.clone();

                adapter.setup_panel_click_handler(panel_id, move |clicked_panel_id| {
                    // Find the pane UUID for this panel
                    let pane_uuid = {
                        let map = uuid_panel_map.borrow();
                        // Reverse lookup: find UUID for panel_id
                        map.iter()
                            .find(|(_, &pid)| pid == clicked_panel_id)
                            .map(|(&uuid, _)| uuid)
                    };

                    if let Some(uuid) = pane_uuid {
                        // Call the user's callback - it handles focus updates via set_focused_pane()
                        on_click_clone(uuid);
                    }
                });
            }
        }
    }

    /// Creates welcome content with full feature showcase
    fn create_welcome_content() -> adw::StatusPage {
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

        let new_conn_btn = gtk4::Button::builder()
            .label("New Connection")
            .css_classes(["suggested-action", "pill"])
            .action_name("win.new-connection")
            .build();
        actions.append(&new_conn_btn);

        let quick_btn = gtk4::Button::builder()
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
            let label = gtk4::Label::builder()
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
        let hint = gtk4::Label::builder()
            .label("Double-click a connection in the sidebar to get started")
            .css_classes(["dim-label"])
            .margin_top(12)
            .build();
        content.append(&hint);

        status_page.set_child(Some(&content));
        status_page
    }

    /// Load embedded SVG logo and render to GdkPixbuf
    fn load_embedded_logo(size: u32) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
        // Embedded SVG icon data
        const ICON_SVG: &[u8] = include_bytes!(
            "../../assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"
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

    /// Sets up the "Select Tab" callback for empty panel placeholders.
    ///
    /// When the user clicks the "Select Tab" button in an empty panel,
    /// this callback shows a popover with available sessions to choose from.
    /// Only sessions NOT already displayed in this split view are shown.
    /// This provides an alternative to drag-and-drop for moving sessions
    /// to split panels.
    ///
    /// # Arguments
    ///
    /// * `on_session_selected` - Callback invoked when a session is selected.
    ///   Receives (panel_uuid, session_id) and should move the session to the panel.
    #[allow(dead_code)]
    pub fn setup_select_tab_callback<F>(&self, on_session_selected: F)
    where
        F: Fn(Uuid, Uuid) + Clone + 'static,
    {
        let sessions = Rc::clone(&self.sessions);
        // Use the provider-based version with a closure that reads from internal sessions
        self.setup_select_tab_callback_with_provider(
            move || {
                sessions
                    .borrow()
                    .iter()
                    .map(|(id, session)| (*id, session.name.clone()))
                    .collect()
            },
            on_session_selected,
        );
    }

    /// Sets up the "Select Tab" callback with an external session provider.
    ///
    /// This version allows the caller to provide a function that returns all
    /// available sessions (e.g., from `TerminalNotebook`), rather than using
    /// the internal sessions map which may not contain all open tabs.
    ///
    /// # Arguments
    ///
    /// * `session_provider` - Function that returns all available sessions as (id, name) pairs
    /// * `on_session_selected` - Callback invoked when a session is selected.
    ///   Receives (panel_uuid, session_id) and should move the session to the panel.
    pub fn setup_select_tab_callback_with_provider<P, F>(
        &self,
        session_provider: P,
        on_session_selected: F,
    ) where
        P: Fn() -> Vec<(Uuid, String)> + Clone + 'static,
        F: Fn(Uuid, Uuid) + Clone + 'static,
    {
        // Share the actual panel_uuid_map reference instead of cloning
        // This ensures new panels added during splits are visible to the callback
        let panel_uuid_map = Rc::clone(&self.panel_uuid_map);
        let adapter = Rc::clone(&self.adapter);
        let root = self.root.clone();

        self.adapter
            .borrow()
            .set_select_tab_callback(move |panel_id| {
                // Get the panel UUID for this panel_id
                let Some(panel_uuid) = panel_uuid_map.borrow().get(&panel_id).copied() else {
                    tracing::warn!("No UUID found for panel {panel_id}");
                    return;
                };

                // Get sessions already displayed in this split view using the adapter
                // This is more reliable than using pane.current_session() which may not be updated
                let sessions_in_split: std::collections::HashSet<Uuid> = {
                    let adapter_ref = adapter.borrow();
                    adapter_ref
                        .panel_ids()
                        .iter()
                        .filter_map(|&pid| adapter_ref.get_panel_session(pid))
                        .map(|sid| sid.as_uuid())
                        .collect()
                };

                // Get all sessions from the provider
                let all_sessions = session_provider();

                // Filter to only those NOT already in this split view
                let available_sessions: Vec<(Uuid, String)> = all_sessions
                    .into_iter()
                    .filter(|(id, _)| !sessions_in_split.contains(id))
                    .collect();

                if available_sessions.is_empty() {
                    tracing::debug!("No sessions available to select (all already in split view)");
                    // Show a toast or message that all sessions are already displayed
                    let popover = gtk4::Popover::new();
                    popover.set_parent(&root);
                    popover.set_autohide(true);

                    let content = GtkBox::new(Orientation::Vertical, 6);
                    content.set_margin_top(12);
                    content.set_margin_bottom(12);
                    content.set_margin_start(12);
                    content.set_margin_end(12);

                    let label = gtk4::Label::builder()
                        .label("All tabs are already displayed in this split view")
                        .css_classes(["dim-label"])
                        .build();
                    content.append(&label);

                    popover.set_child(Some(&content));

                    let width = root.width();
                    let height = root.height();
                    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                        width / 2,
                        height / 2,
                        1,
                        1,
                    )));

                    popover.popup();

                    let root_weak = root.downgrade();
                    popover.connect_closed(move |pop| {
                        if root_weak.upgrade().is_some() {
                            pop.unparent();
                        }
                    });
                    return;
                }

                // Create a popover with session list
                let popover = gtk4::Popover::new();
                popover.set_parent(&root);
                popover.set_autohide(true);

                let content = GtkBox::new(Orientation::Vertical, 6);
                content.set_margin_top(12);
                content.set_margin_bottom(12);
                content.set_margin_start(12);
                content.set_margin_end(12);

                let title = gtk4::Label::builder()
                    .label("Select a tab to display")
                    .css_classes(["heading"])
                    .halign(gtk4::Align::Start)
                    .build();
                content.append(&title);

                let list_box = gtk4::ListBox::builder()
                    .selection_mode(gtk4::SelectionMode::None)
                    .css_classes(["boxed-list"])
                    .build();

                for (session_id, session_name) in available_sessions {
                    let row = adw::ActionRow::builder()
                        .title(&session_name)
                        .activatable(true)
                        .build();
                    row.add_prefix(&gtk4::Image::from_icon_name("utilities-terminal-symbolic"));

                    let callback = on_session_selected.clone();
                    let popover_weak = popover.downgrade();
                    row.connect_activated(move |_| {
                        callback(panel_uuid, session_id);
                        if let Some(pop) = popover_weak.upgrade() {
                            pop.popdown();
                        }
                    });

                    list_box.append(&row);
                }

                content.append(&list_box);
                popover.set_child(Some(&content));

                // Position popover in center of root widget
                let width = root.width();
                let height = root.height();
                popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                    width / 2,
                    height / 2,
                    1,
                    1,
                )));

                popover.popup();

                // Clean up popover when closed
                let root_weak = root.downgrade();
                popover.connect_closed(move |pop| {
                    if root_weak.upgrade().is_some() {
                        pop.unparent();
                    }
                });
            });
    }

    /// Sets up the close panel callback for empty panel close buttons.
    ///
    /// When the user clicks the close button (X) on an empty panel, this callback:
    /// 1. Focuses the panel (so `close_pane()` closes the correct one)
    /// 2. Activates the `win.close-pane` action to properly close the panel
    ///
    /// # Arguments
    ///
    /// * `on_focus` - Callback to focus the panel, receives the pane UUID
    pub fn setup_close_panel_callback<F>(&self, on_focus: F)
    where
        F: Fn(Uuid) + Clone + 'static,
    {
        let panel_uuid_map = Rc::clone(&self.panel_uuid_map);
        let root = self.root.clone();

        self.adapter
            .borrow()
            .set_close_panel_callback(move |panel_id| {
                // Find the pane UUID for this panel
                let pane_uuid = {
                    let map = panel_uuid_map.borrow();
                    map.get(&panel_id).copied()
                };

                let Some(uuid) = pane_uuid else {
                    tracing::warn!("Close button: no UUID found for panel {panel_id}");
                    return;
                };

                tracing::debug!("Close button: focusing panel {panel_id} (uuid={uuid})");

                // Focus the panel first
                on_focus(uuid);

                // Activate the close-pane action on the window
                // This ensures proper cleanup: color release, terminal reparenting, etc.
                if let Some(window) = root
                    .root()
                    .and_then(|r| r.downcast::<gtk4::ApplicationWindow>().ok())
                {
                    tracing::debug!("Close button: activating close-pane action");
                    gtk4::prelude::ActionGroupExt::activate_action(&window, "close-pane", None);
                } else {
                    tracing::warn!("Close button: could not find ApplicationWindow");
                }
            });
    }

    /// Moves a session to a specific panel by UUID.
    ///
    /// This is called when the user selects a session from the "Select Tab" popover.
    /// It places the session in the specified panel and updates the terminal display.
    ///
    /// Note: This method tries to get the terminal from the internal `terminals` map,
    /// which may be empty if terminals are stored in `TerminalNotebook`. For reliable
    /// terminal display, use `move_session_to_panel_with_terminal` instead.
    ///
    /// # Arguments
    ///
    /// * `panel_uuid` - The UUID of the target panel
    /// * `session_id` - The UUID of the session to move
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error message on failure.
    pub fn move_session_to_panel(&self, panel_uuid: Uuid, session_id: Uuid) -> Result<(), String> {
        let panel_id = *self
            .uuid_panel_map
            .borrow()
            .get(&panel_uuid)
            .ok_or("Panel not found")?;

        let session = rustconn_core::split::SessionId::from_uuid(session_id);

        // Place session in panel
        self.adapter
            .borrow_mut()
            .place_in_panel(panel_id, session)
            .map_err(|e| e.to_string())?;

        // Set terminal content if available from internal map
        if let Some(terminal) = self.terminals.borrow().get(&session_id).cloned() {
            Self::detach_terminal_from_parent(&terminal);
            let scrolled = Self::create_terminal_scrolled_window(&terminal);
            self.adapter.borrow().set_panel_content(panel_id, &scrolled);
            terminal.set_visible(true);
        }

        // Update pane's current_session for filtering in Select Tab
        {
            let mut panes = self.panes.borrow_mut();
            if let Some(pane) = panes.iter_mut().find(|p| p.id() == panel_uuid) {
                pane.set_current_session(Some(session_id));
            }
        }

        // Update focused pane UUID
        *self.focused_pane_uuid.borrow_mut() = Some(panel_uuid);

        // Update focus styling in the adapter
        if let Err(e) = self.adapter.borrow_mut().set_focus(panel_id) {
            tracing::warn!("Failed to set focus on panel {}: {}", panel_id, e);
        }

        Ok(())
    }

    /// Moves a session to a specific panel by UUID, with an externally provided terminal.
    ///
    /// This is the preferred method when the terminal is stored in `TerminalNotebook`
    /// rather than in the bridge's internal `terminals` map.
    ///
    /// The session's color is set to match the container's color, ensuring
    /// consistent tab coloring for all sessions in this split container.
    ///
    /// # Arguments
    ///
    /// * `panel_uuid` - The UUID of the target panel
    /// * `session_id` - The UUID of the session to move
    /// * `terminal` - The VTE terminal widget to display in the panel
    ///
    /// # Returns
    ///
    /// `Ok(color_index)` with the container's color on success, or an error message on failure.
    pub fn move_session_to_panel_with_terminal(
        &self,
        panel_uuid: Uuid,
        session_id: Uuid,
        terminal: &Terminal,
    ) -> Result<usize, String> {
        tracing::debug!(
            "move_session_to_panel_with_terminal: panel_uuid={}, session_id={}",
            panel_uuid,
            session_id
        );

        let panel_id = *self
            .uuid_panel_map
            .borrow()
            .get(&panel_uuid)
            .ok_or_else(|| {
                let available = self
                    .uuid_panel_map
                    .borrow()
                    .keys()
                    .copied()
                    .collect::<Vec<_>>();
                format!(
                    "Panel not found: uuid={}, available uuids={:?}",
                    panel_uuid, available
                )
            })?;

        tracing::debug!(
            "move_session_to_panel_with_terminal: resolved panel_id={} for uuid={}",
            panel_id,
            panel_uuid
        );

        let session = rustconn_core::split::SessionId::from_uuid(session_id);

        // Place session in panel
        self.adapter
            .borrow_mut()
            .place_in_panel(panel_id, session)
            .map_err(|e| e.to_string())?;

        // Store terminal in bridge's map for later restoration after rebuild_widgets()
        // This is critical: when another split happens, rebuild_widgets() recreates all
        // panel widgets with "Loading..." placeholders, and restore_panel_contents()
        // needs to find the terminal in self.terminals to restore it.
        self.terminals
            .borrow_mut()
            .insert(session_id, terminal.clone());

        // Detach terminal from any previous parent and display in panel
        Self::detach_terminal_from_parent(terminal);
        let scrolled = Self::create_terminal_scrolled_window(terminal);
        self.adapter.borrow().set_panel_content(panel_id, &scrolled);
        terminal.set_visible(true);

        // Update pane's current_session for filtering in Select Tab
        {
            let mut panes = self.panes.borrow_mut();
            if let Some(pane) = panes.iter_mut().find(|p| p.id() == panel_uuid) {
                pane.set_current_session(Some(session_id));
            } else {
                tracing::warn!(
                    "move_session_to_panel_with_terminal: pane not found for uuid={}",
                    panel_uuid
                );
            }
        }

        // Use the container's color for consistent tab coloring
        let color_index = self.get_container_color().ok_or_else(|| {
            format!(
                "Container has no color assigned (panel_uuid={})",
                panel_uuid
            )
        })?;
        self.set_session_color(session_id, color_index);

        // Update focused pane UUID
        *self.focused_pane_uuid.borrow_mut() = Some(panel_uuid);

        // Update focus styling in the adapter
        if let Err(e) = self.adapter.borrow_mut().set_focus(panel_id) {
            tracing::warn!("Failed to set focus on panel {}: {}", panel_id, e);
        }

        tracing::debug!(
            "move_session_to_panel_with_terminal: SUCCESS - session {} in panel {} with \
             container color {}",
            session_id,
            panel_id,
            color_index
        );

        Ok(color_index)
    }

    /// Creates welcome content - static version for use by other modules
    /// This provides a unified welcome screen across the application
    #[must_use]
    pub fn create_welcome_content_static() -> adw::StatusPage {
        Self::create_welcome_content()
    }
}

impl Default for SplitViewBridge {
    fn default() -> Self {
        Self::new()
    }
}
