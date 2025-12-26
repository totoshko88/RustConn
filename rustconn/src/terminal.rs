//! Terminal notebook area
//!
//! This module provides the tabbed terminal interface using VTE4
//! for SSH sessions and native GTK widgets for VNC/RDP/SPICE connections.
//!
//! # Requirements Coverage
//!
//! - Requirement 2.1: Native VNC embedding as GTK widget
//! - Requirement 2.6: Multiple VNC sessions in separate tabs with proper isolation
//!
//! # Adaptive Tab Display
//!
//! The notebook supports adaptive tab display for handling many open sessions:
//! - Full mode: Protocol icon + full connection name
//! - Compact mode: Protocol icon + truncated name (when space is limited)
//! - Icon mode: Protocol icon only (when very limited space)
//! - All tabs have tooltips showing full name and host

use gtk4::prelude::*;
use gtk4::{
    gdk, gio, glib, Box as GtkBox, Button, Image, Label, MenuButton, Notebook, Orientation,
    Popover, ScrolledWindow, Widget,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;
use vte4::prelude::*;
use vte4::{PtyFlags, Terminal};

use crate::session::{
    RdpSessionWidget, SessionState, SessionWidget, SpiceSessionWidget, VncSessionWidget,
};

/// Tab display mode based on available space
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabDisplayMode {
    /// Full mode: icon + full name (default)
    #[default]
    Full,
    /// Compact mode: icon + truncated name (max 12 chars)
    Compact,
    /// Icon mode: icon only
    IconOnly,
}

/// Terminal session information
#[derive(Debug, Clone)]
pub struct TerminalSession {
    /// Session UUID for session management
    pub id: Uuid,
    /// Connection ID this session is for
    pub connection_id: Uuid,
    /// Connection name for display
    pub name: String,
    /// Protocol type (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Whether this is an embedded terminal or external window
    pub is_embedded: bool,
    /// Log file path if logging is enabled
    pub log_file: Option<PathBuf>,
}

use crate::embedded_rdp::EmbeddedRdpWidget;
use crate::embedded_spice::EmbeddedSpiceWidget;

/// Session widget storage for non-SSH sessions
enum SessionWidgetStorage {
    /// VNC session widget
    Vnc(Rc<VncSessionWidget>),
    /// RDP session widget
    Rdp(Rc<RdpSessionWidget>),
    /// SPICE session widget (legacy)
    Spice(Rc<SpiceSessionWidget>),
    /// Embedded RDP widget (with dynamic resolution)
    EmbeddedRdp(Rc<EmbeddedRdpWidget>),
    /// Embedded SPICE widget (native spice-client)
    EmbeddedSpice(Rc<EmbeddedSpiceWidget>),
}

/// Terminal notebook widget for managing multiple terminal sessions
pub struct TerminalNotebook {
    /// Main container with notebook and overflow button
    container: GtkBox,
    notebook: Notebook,
    /// Overflow menu button (shown when tabs don't fit)
    overflow_button: MenuButton,
    /// Overflow popover content
    overflow_box: GtkBox,
    /// Map of session IDs to their tab indices
    sessions: Rc<RefCell<HashMap<Uuid, u32>>>,
    /// Map of session IDs to terminal widgets (for SSH sessions)
    terminals: Rc<RefCell<HashMap<Uuid, Terminal>>>,
    /// Map of session IDs to session widgets (for VNC/RDP/SPICE sessions)
    session_widgets: Rc<RefCell<HashMap<Uuid, SessionWidgetStorage>>>,
    /// Session metadata
    session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    /// Current tab display mode
    display_mode: Rc<Cell<TabDisplayMode>>,
    /// Map of session IDs to their tab label widgets (for updating display mode)
    tab_labels: Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
}

/// Widgets that make up a tab label (for updating display mode)
struct TabLabelWidgets {
    container: GtkBox,
    icon: Image,
    label: Label,
    full_name: String,
}

impl TerminalNotebook {
    /// Creates a new terminal notebook
    #[must_use]
    pub fn new() -> Self {
        // Main container
        let container = GtkBox::new(Orientation::Vertical, 0);

        let notebook = Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_show_border(false);
        notebook.set_tab_pos(gtk4::PositionType::Top);
        notebook.set_hexpand(true);
        notebook.set_vexpand(true);

        // Create overflow menu button
        let overflow_button = MenuButton::new();
        overflow_button.set_icon_name("view-more-symbolic");
        overflow_button.add_css_class("flat");
        overflow_button.set_tooltip_text(Some("All tabs"));
        overflow_button.set_visible(false);

        // Create overflow popover
        let overflow_popover = Popover::new();
        let overflow_scroll = ScrolledWindow::new();
        overflow_scroll.set_max_content_height(400);
        overflow_scroll.set_propagate_natural_height(true);
        overflow_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        let overflow_box = GtkBox::new(Orientation::Vertical, 2);
        overflow_box.set_margin_start(4);
        overflow_box.set_margin_end(4);
        overflow_box.set_margin_top(4);
        overflow_box.set_margin_bottom(4);
        overflow_scroll.set_child(Some(&overflow_box));
        overflow_popover.set_child(Some(&overflow_scroll));
        overflow_button.set_popover(Some(&overflow_popover));

        // Header box with notebook tabs and overflow button
        let header_box = GtkBox::new(Orientation::Horizontal, 0);
        header_box.append(&notebook);
        header_box.append(&overflow_button);

        container.append(&header_box);

        // Add a welcome tab
        let welcome = Self::create_welcome_tab();
        let welcome_label = Label::new(Some("Welcome"));
        notebook.append_page(&welcome, Some(&welcome_label));

        let display_mode = Rc::new(Cell::new(TabDisplayMode::Full));
        let tab_labels = Rc::new(RefCell::new(HashMap::new()));

        let term_notebook = Self {
            container,
            notebook,
            overflow_button,
            overflow_box,
            sessions: Rc::new(RefCell::new(HashMap::new())),
            terminals: Rc::new(RefCell::new(HashMap::new())),
            session_widgets: Rc::new(RefCell::new(HashMap::new())),
            session_info: Rc::new(RefCell::new(HashMap::new())),
            display_mode,
            tab_labels,
        };

        // Setup resize handler for adaptive tabs
        term_notebook.setup_adaptive_tabs();

        term_notebook
    }

    /// Sets up the adaptive tab display handler
    fn setup_adaptive_tabs(&self) {
        let notebook = self.notebook.clone();
        let display_mode = self.display_mode.clone();
        let tab_labels = self.tab_labels.clone();
        let overflow_button = self.overflow_button.clone();

        // Monitor notebook size changes
        notebook.connect_notify_local(Some("width-request"), move |nb, _| {
            Self::update_tab_display_mode(nb, &display_mode, &tab_labels, &overflow_button);
        });

        // Also update on page changes
        let display_mode2 = self.display_mode.clone();
        let tab_labels2 = self.tab_labels.clone();
        let overflow_button2 = self.overflow_button.clone();
        let notebook2 = self.notebook.clone();

        self.notebook.connect_page_added(move |_, _, _| {
            Self::update_tab_display_mode(
                &notebook2,
                &display_mode2,
                &tab_labels2,
                &overflow_button2,
            );
        });

        let display_mode3 = self.display_mode.clone();
        let tab_labels3 = self.tab_labels.clone();
        let overflow_button3 = self.overflow_button.clone();
        let notebook3 = self.notebook.clone();

        self.notebook.connect_page_removed(move |_, _, _| {
            Self::update_tab_display_mode(
                &notebook3,
                &display_mode3,
                &tab_labels3,
                &overflow_button3,
            );
        });
    }

    /// Updates tab display mode based on available space
    fn update_tab_display_mode(
        notebook: &Notebook,
        display_mode: &Rc<Cell<TabDisplayMode>>,
        tab_labels: &Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
        overflow_button: &MenuButton,
    ) {
        let available_width = notebook.width();
        if available_width <= 0 {
            return;
        }

        let tab_count = tab_labels.borrow().len();
        if tab_count == 0 {
            overflow_button.set_visible(false);
            return;
        }

        // Estimate tab widths for each mode
        // Full: ~150px, Compact: ~80px, Icon: ~40px
        let min_full = 150;
        let min_compact = 80;
        let min_icon = 40;

        let tab_count_i32 = tab_count as i32;

        let new_mode = if available_width >= tab_count_i32 * min_full {
            TabDisplayMode::Full
        } else if available_width >= tab_count_i32 * min_compact {
            TabDisplayMode::Compact
        } else {
            TabDisplayMode::IconOnly
        };

        // Show overflow button when even icon mode doesn't fit well
        let need_overflow = available_width < tab_count_i32 * min_icon;
        overflow_button.set_visible(need_overflow);

        // Update tabs if mode changed
        if new_mode != display_mode.get() {
            display_mode.set(new_mode);

            for widgets in tab_labels.borrow().values() {
                Self::apply_display_mode_to_tab(widgets, new_mode);
            }
        }
    }

    /// Applies display mode to a single tab
    fn apply_display_mode_to_tab(widgets: &TabLabelWidgets, mode: TabDisplayMode) {
        match mode {
            TabDisplayMode::Full => {
                widgets.label.set_visible(true);
                widgets.label.set_text(&widgets.full_name);
                widgets.label.set_max_width_chars(20);
            }
            TabDisplayMode::Compact => {
                widgets.label.set_visible(true);
                let truncated = Self::truncate_name(&widgets.full_name, 10);
                widgets.label.set_text(&truncated);
                widgets.label.set_max_width_chars(10);
            }
            TabDisplayMode::IconOnly => {
                widgets.label.set_visible(false);
            }
        }
    }

    /// Truncates a name to max_chars with ellipsis
    fn truncate_name(name: &str, max_chars: usize) -> String {
        if name.chars().count() <= max_chars {
            name.to_string()
        } else {
            let truncated: String = name.chars().take(max_chars - 1).collect();
            format!("{truncated}…")
        }
    }

    /// Creates the welcome tab content (minimal - actual content shown in split view)
    fn create_welcome_tab() -> GtkBox {
        // Create empty container - actual welcome content is in split view
        let container = GtkBox::new(Orientation::Vertical, 0);
        // Add an invisible spacer to ensure the page has some content
        let spacer = gtk4::DrawingArea::new();
        spacer.set_content_width(1);
        spacer.set_content_height(1);
        container.append(&spacer);
        container
    }

    /// Creates a new terminal tab for an SSH session
    ///
    /// This creates the terminal widget and prepares it for spawning a command.
    /// Returns the session UUID for tracking.
    pub fn create_terminal_tab(&self, connection_id: Uuid, title: &str, protocol: &str) -> Uuid {
        let session_id = Uuid::new_v4();
        let is_first_session = self.sessions.borrow().is_empty();

        // Remove Welcome tab if this is the first session
        // Welcome tab is page 0 and has no session_id
        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        // Create VTE terminal
        let terminal = Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        // Configure terminal appearance
        Self::configure_terminal(&terminal);

        // Create empty placeholder for notebook page (terminal shown in split view)
        let placeholder = GtkBox::new(Orientation::Vertical, 0);
        // Add an invisible spacer to ensure the page has some content
        let spacer = gtk4::DrawingArea::new();
        spacer.set_content_width(1);
        spacer.set_content_height(1);
        placeholder.append(&spacer);

        // Create tab label with close button
        let tab_label = Self::create_tab_label(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            &self.tab_labels,
            &self.overflow_box,
        );

        // Add the page with empty placeholder (terminal is NOT added to notebook)
        let page_num = self.notebook.append_page(&placeholder, Some(&tab_label));

        // Store session mapping BEFORE switching page
        // This ensures switch_page handler can find the session
        self.sessions.borrow_mut().insert(session_id, page_num);
        // Store terminal separately - it will be shown in split view
        self.terminals.borrow_mut().insert(session_id, terminal);

        // Store session info
        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: true,
                log_file: None,
            },
        );

        // Make the tab reorderable
        self.notebook.set_tab_reorderable(&placeholder, true);

        // Switch to the new tab AFTER all data is stored
        // This triggers switch_page signal which will show the session in split view
        self.notebook.set_current_page(Some(page_num));

        session_id
    }

    /// Creates a new VNC session tab
    ///
    /// This creates a VNC session widget and prepares it for connection.
    /// Returns the session UUID for tracking.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 2.1: Native VNC embedding as GTK widget
    /// - Requirement 2.6: Multiple VNC sessions in separate tabs
    pub fn create_vnc_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_vnc_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new VNC session tab with host information
    ///
    /// Extended version that includes host for tooltip display.
    pub fn create_vnc_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        let is_first_session = self.sessions.borrow().is_empty();

        // Remove Welcome tab if this is the first session
        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        // Create VNC session widget
        let vnc_widget = Rc::new(VncSessionWidget::new());

        // Create container for VNC widget in notebook tab
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        // Add the VNC widget to the container
        container.append(vnc_widget.widget());

        // Create tab label with protocol icon
        let tab_label = Self::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "vnc",
            host,
            &self.tab_labels,
            &self.overflow_box,
        );

        // Add the page with VNC widget container
        let page_num = self.notebook.append_page(&container, Some(&tab_label));

        // Store session mapping
        self.sessions.borrow_mut().insert(session_id, page_num);

        // Store VNC widget
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::Vnc(vnc_widget));

        // Store session info
        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "vnc".to_string(),
                is_embedded: true,
                log_file: None,
            },
        );

        // Make the tab reorderable
        self.notebook.set_tab_reorderable(&container, true);

        // Switch to the new tab
        self.notebook.set_current_page(Some(page_num));

        session_id
    }

    /// Creates a new RDP session tab
    ///
    /// This creates an RDP session widget and prepares it for connection.
    /// Returns the session UUID for tracking.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 3.1: Native RDP embedding as GTK widget
    pub fn create_rdp_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_rdp_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new RDP session tab with host information
    ///
    /// Extended version that includes host for tooltip display.
    pub fn create_rdp_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        let is_first_session = self.sessions.borrow().is_empty();

        // Remove Welcome tab if this is the first session
        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        // Create RDP session widget
        let rdp_widget = Rc::new(RdpSessionWidget::new());

        // Create empty placeholder for notebook page (RDP widget shown in split view)
        let placeholder = GtkBox::new(Orientation::Vertical, 0);
        let spacer = gtk4::DrawingArea::new();
        spacer.set_content_width(1);
        spacer.set_content_height(1);
        placeholder.append(&spacer);

        // Create tab label with protocol icon
        let tab_label = Self::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "rdp",
            host,
            &self.tab_labels,
            &self.overflow_box,
        );

        // Add the page with empty placeholder
        let page_num = self.notebook.append_page(&placeholder, Some(&tab_label));

        // Store session mapping
        self.sessions.borrow_mut().insert(session_id, page_num);

        // Store RDP widget
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::Rdp(rdp_widget));

        // Store session info
        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "rdp".to_string(),
                is_embedded: true,
                log_file: None,
            },
        );

        // Make the tab reorderable
        self.notebook.set_tab_reorderable(&placeholder, true);

        // Switch to the new tab
        self.notebook.set_current_page(Some(page_num));

        session_id
    }

    /// Creates a new SPICE session tab
    ///
    /// This creates a SPICE session widget and prepares it for connection.
    /// Returns the session UUID for tracking.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 4.2: Native SPICE embedding as GTK widget
    pub fn create_spice_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_spice_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new SPICE session tab with host information
    ///
    /// Extended version that includes host for tooltip display.
    pub fn create_spice_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        let is_first_session = self.sessions.borrow().is_empty();

        // Remove Welcome tab if this is the first session
        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        // Create embedded SPICE widget (uses native spice-client)
        let spice_widget = Rc::new(EmbeddedSpiceWidget::new());

        // Create container for the SPICE widget
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(spice_widget.widget());

        // Create tab label with protocol icon
        let tab_label = Self::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "spice",
            host,
            &self.tab_labels,
            &self.overflow_box,
        );

        // Add the page with SPICE widget
        let page_num = self.notebook.append_page(&container, Some(&tab_label));

        // Store session mapping
        self.sessions.borrow_mut().insert(session_id, page_num);

        // Store embedded SPICE widget
        self.session_widgets.borrow_mut().insert(
            session_id,
            SessionWidgetStorage::EmbeddedSpice(spice_widget),
        );

        // Store session info
        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "spice".to_string(),
                is_embedded: true,
                log_file: None,
            },
        );

        // Make the tab reorderable
        self.notebook.set_tab_reorderable(&container, true);

        // Switch to the new tab
        self.notebook.set_current_page(Some(page_num));

        session_id
    }

    /// Gets the VNC session widget for a session
    #[must_use]
    pub fn get_vnc_widget(&self, session_id: Uuid) -> Option<Rc<VncSessionWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.clone()),
            _ => None,
        }
    }

    /// Gets the RDP session widget for a session
    #[must_use]
    pub fn get_rdp_widget(&self, session_id: Uuid) -> Option<Rc<RdpSessionWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Rdp(widget)) => Some(widget.clone()),
            _ => None,
        }
    }

    /// Gets the SPICE session widget for a session
    #[must_use]
    pub fn get_spice_widget(&self, session_id: Uuid) -> Option<Rc<EmbeddedSpiceWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::EmbeddedSpice(widget)) => Some(widget.clone()),
            Some(SessionWidgetStorage::Spice(_)) => None, // Legacy widget, not supported
            _ => None,
        }
    }

    /// Gets the session widget (VNC/RDP/SPICE) for a session
    #[must_use]
    pub fn get_session_widget(&self, session_id: Uuid) -> Option<SessionWidget> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Vnc(_)) => {
                // Return a new VncSessionWidget wrapper
                // Note: The actual widget is stored separately and accessed via get_vnc_widget
                Some(SessionWidget::Vnc(VncSessionWidget::new()))
            }
            Some(SessionWidgetStorage::Rdp(_)) => {
                // Return a new RdpSessionWidget wrapper
                // Note: The actual widget is stored separately and accessed via get_rdp_widget
                Some(SessionWidget::Rdp(RdpSessionWidget::new()))
            }
            Some(SessionWidgetStorage::Spice(_)) => {
                // Return a new SpiceSessionWidget wrapper
                // Note: The actual widget is stored separately and accessed via get_spice_widget
                Some(SessionWidget::Spice(SpiceSessionWidget::new()))
            }
            _ => {
                drop(widgets);
                // Check if it's an SSH terminal
                if let Some(terminal) = self.terminals.borrow().get(&session_id) {
                    Some(SessionWidget::Ssh(terminal.clone()))
                } else {
                    None
                }
            }
        }
    }

    /// Gets the GTK widget for a session (for display in split view)
    ///
    /// Returns the appropriate widget based on session type:
    /// - SSH: VTE Terminal widget
    /// - VNC: VncSessionWidget overlay
    /// - RDP: RdpSessionWidget overlay
    /// - SPICE: SpiceSessionWidget overlay or EmbeddedSpiceWidget
    #[must_use]
    pub fn get_session_display_widget(&self, session_id: Uuid) -> Option<Widget> {
        // Check for VNC/RDP/SPICE session widgets first
        let widgets = self.session_widgets.borrow();
        if let Some(storage) = widgets.get(&session_id) {
            return match storage {
                SessionWidgetStorage::Vnc(widget) => Some(widget.widget().clone()),
                SessionWidgetStorage::Rdp(widget) => Some(widget.widget().clone()),
                SessionWidgetStorage::Spice(widget) => Some(widget.widget().clone()),
                SessionWidgetStorage::EmbeddedRdp(widget) => Some(widget.widget().clone().upcast()),
                SessionWidgetStorage::EmbeddedSpice(widget) => {
                    Some(widget.widget().clone().upcast())
                }
            };
        }
        drop(widgets);

        // Fall back to SSH terminal
        self.terminals
            .borrow()
            .get(&session_id)
            .map(|t| t.clone().upcast())
    }

    /// Gets the session state for a VNC/RDP/SPICE session
    #[must_use]
    pub fn get_session_state(&self, session_id: Uuid) -> Option<SessionState> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.state()),
            Some(SessionWidgetStorage::Rdp(widget)) => Some(widget.state()),
            Some(SessionWidgetStorage::Spice(widget)) => Some(widget.state()),
            _ => None,
        }
    }

    /// Configures terminal appearance and behavior
    fn configure_terminal(terminal: &Terminal) {
        // Cursor settings
        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::On);
        terminal.set_cursor_shape(vte4::CursorShape::Block);

        // Scrolling behavior
        terminal.set_scroll_on_output(false);
        terminal.set_scroll_on_keystroke(true);
        terminal.set_scrollback_lines(10000);

        // Input handling
        terminal.set_input_enabled(true);
        terminal.set_allow_hyperlink(true);
        terminal.set_mouse_autohide(true);

        // Set up terminal colors (dark theme)
        let bg_color = gdk::RGBA::new(0.1, 0.1, 0.1, 1.0);
        let fg_color = gdk::RGBA::new(0.9, 0.9, 0.9, 1.0);
        terminal.set_color_background(&bg_color);
        terminal.set_color_foreground(&fg_color);

        // Set up palette colors (standard 16-color palette)
        let palette: [gdk::RGBA; 16] = [
            gdk::RGBA::new(0.0, 0.0, 0.0, 1.0), // Black
            gdk::RGBA::new(0.8, 0.0, 0.0, 1.0), // Red
            gdk::RGBA::new(0.0, 0.8, 0.0, 1.0), // Green
            gdk::RGBA::new(0.8, 0.8, 0.0, 1.0), // Yellow
            gdk::RGBA::new(0.0, 0.0, 0.8, 1.0), // Blue
            gdk::RGBA::new(0.8, 0.0, 0.8, 1.0), // Magenta
            gdk::RGBA::new(0.0, 0.8, 0.8, 1.0), // Cyan
            gdk::RGBA::new(0.8, 0.8, 0.8, 1.0), // White
            gdk::RGBA::new(0.4, 0.4, 0.4, 1.0), // Bright Black
            gdk::RGBA::new(1.0, 0.0, 0.0, 1.0), // Bright Red
            gdk::RGBA::new(0.0, 1.0, 0.0, 1.0), // Bright Green
            gdk::RGBA::new(1.0, 1.0, 0.0, 1.0), // Bright Yellow
            gdk::RGBA::new(0.0, 0.0, 1.0, 1.0), // Bright Blue
            gdk::RGBA::new(1.0, 0.0, 1.0, 1.0), // Bright Magenta
            gdk::RGBA::new(0.0, 1.0, 1.0, 1.0), // Bright Cyan
            gdk::RGBA::new(1.0, 1.0, 1.0, 1.0), // Bright White
        ];
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        terminal.set_colors(Some(&fg_color), Some(&bg_color), &palette_refs);

        // Font settings
        let font_desc = gtk4::pango::FontDescription::from_string("Monospace 11");
        terminal.set_font(Some(&font_desc));
    }

    /// Spawns a command in the terminal
    ///
    /// # Arguments
    /// * `session_id` - The session ID to spawn the command in
    /// * `argv` - The command and arguments to execute
    /// * `envv` - Optional environment variables
    /// * `working_directory` - Optional working directory
    ///
    /// # Returns
    /// `true` if the command was spawned successfully
    pub fn spawn_command(
        &self,
        session_id: Uuid,
        argv: &[&str],
        envv: Option<&[&str]>,
        working_directory: Option<&str>,
    ) -> bool {
        let terminals = self.terminals.borrow();
        let Some(terminal) = terminals.get(&session_id) else {
            return false;
        };

        // Convert argv to the format VTE expects
        let argv_gstr: Vec<glib::GString> = argv.iter().map(|s| glib::GString::from(*s)).collect();
        let argv_refs: Vec<&str> = argv_gstr.iter().map(gtk4::glib::GString::as_str).collect();

        // Convert envv if provided
        let envv_gstr: Option<Vec<glib::GString>> =
            envv.map(|e| e.iter().map(|s| glib::GString::from(*s)).collect());
        let envv_refs: Option<Vec<&str>> = envv_gstr
            .as_ref()
            .map(|e| e.iter().map(gtk4::glib::GString::as_str).collect());

        // Spawn the command asynchronously
        terminal.spawn_async(
            PtyFlags::DEFAULT,
            working_directory,
            &argv_refs,
            envv_refs.as_deref().unwrap_or(&[]),
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            gio::Cancellable::NONE,
            |result| {
                match result {
                    Ok(_pid) => {
                        // Command spawned successfully
                    }
                    Err(e) => {
                        eprintln!("Failed to spawn command: {e}");
                    }
                }
            },
        );

        true
    }

    /// Spawns an SSH command in the terminal
    ///
    /// This is a convenience method for spawning SSH connections.
    pub fn spawn_ssh(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        extra_args: &[&str],
    ) -> bool {
        let mut argv = vec!["ssh"];

        // Add port if not default
        let port_str;
        if port != 22 {
            port_str = port.to_string();
            argv.push("-p");
            argv.push(&port_str);
        }

        // Add identity file if specified
        if let Some(key) = identity_file {
            argv.push("-i");
            argv.push(key);
        }

        // Add extra arguments
        argv.extend(extra_args);

        // Add destination
        let destination = if let Some(user) = username {
            format!("{user}@{host}")
        } else {
            host.to_string()
        };
        argv.push(&destination);

        self.spawn_command(session_id, &argv, None, None)
    }

    /// Creates a tab label with icon, title, close button, and drag source
    ///
    /// The tab label adapts to available space:
    /// - Shows protocol icon for quick identification
    /// - Label truncates with ellipsis when space is limited
    /// - Tooltip shows full name and host information
    fn create_tab_label(
        title: &str,
        session_id: Uuid,
        notebook: &Notebook,
        sessions: &Rc<RefCell<HashMap<Uuid, u32>>>,
        tab_labels: &Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
        overflow_box: &GtkBox,
    ) -> GtkBox {
        Self::create_tab_label_with_protocol(
            title,
            session_id,
            notebook,
            sessions,
            "ssh",
            "",
            tab_labels,
            overflow_box,
        )
    }

    /// Creates a tab label with protocol icon, title, close button, and drag source
    ///
    /// Extended version that includes protocol icon and host for tooltip.
    #[allow(clippy::too_many_arguments)]
    fn create_tab_label_with_protocol(
        title: &str,
        session_id: Uuid,
        notebook: &Notebook,
        sessions: &Rc<RefCell<HashMap<Uuid, u32>>>,
        protocol: &str,
        host: &str,
        tab_labels: &Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
        overflow_box: &GtkBox,
    ) -> GtkBox {
        let tab_box = GtkBox::new(Orientation::Horizontal, 4);
        tab_box.add_css_class("session-tab");

        // Protocol icon - handle zerotrust:provider format
        // Icons must match sidebar.rs get_protocol_icon() for consistency
        let icon_name = if let Some(provider) = protocol.strip_prefix("zerotrust:") {
            // ZeroTrust provider-specific icons (unique, no duplicates with base protocols)
            match provider {
                "aws" | "aws_ssm" => "network-workgroup-symbolic", // AWS - workgroup
                "gcloud" | "gcp_iap" => "weather-overcast-symbolic", // GCP - cloud
                "azure" | "azure_bastion" => "weather-few-clouds-symbolic", // Azure - clouds
                "azure_ssh" => "weather-showers-symbolic",         // Azure SSH - showers
                "oci" | "oci_bastion" => "drive-harddisk-symbolic", // OCI - harddisk
                "cloudflare" | "cloudflare_access" => "security-high-symbolic", // Cloudflare
                "teleport" => "emblem-system-symbolic",            // Teleport - gear
                "tailscale" | "tailscale_ssh" => "network-vpn-symbolic", // Tailscale - VPN
                "boundary" => "dialog-password-symbolic",          // Boundary - lock
                "generic" => "system-run-symbolic",                // Generic - run
                _ => "folder-remote-symbolic",                     // Unknown - remote
            }
        } else {
            // Base protocol icons
            match protocol.to_lowercase().as_str() {
                "ssh" => "network-server-symbolic",
                "rdp" => "computer-symbolic",
                "vnc" => "video-display-symbolic",
                "spice" => "video-x-generic-symbolic",
                "zerotrust" => "folder-remote-symbolic",
                _ => "network-server-symbolic",
            }
        };
        let icon = Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        icon.add_css_class("tab-icon");
        tab_box.append(&icon);

        // Label with ellipsis
        let label = Label::new(Some(title));
        label.set_hexpand(true);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(20);
        label.add_css_class("tab-label");
        tab_box.append(&label);

        // Close button
        let close_button = Button::from_icon_name("window-close-symbolic");
        close_button.add_css_class("flat");
        close_button.add_css_class("circular");
        close_button.add_css_class("tab-close-button");
        close_button.set_tooltip_text(Some("Close tab"));

        // Connect close button - switch to this tab first, then trigger close-tab action
        let notebook_weak = notebook.downgrade();
        let sessions_clone = sessions.clone();
        close_button.connect_clicked(move |button| {
            if let Some(notebook) = notebook_weak.upgrade() {
                let sessions = sessions_clone.borrow();
                if let Some(&page_num) = sessions.get(&session_id) {
                    // Switch to this tab first so close-tab action closes the right one
                    notebook.set_current_page(Some(page_num));
                    drop(sessions);
                    // Trigger the close-tab action
                    if let Some(root) = button.root() {
                        if let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() {
                            gtk4::prelude::ActionGroupExt::activate_action(
                                window,
                                "close-tab",
                                None,
                            );
                        }
                    }
                }
            }
        });

        tab_box.append(&close_button);

        // Add drag source for dragging sessions to split panes
        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);

        // Provide the session ID as drag data
        let session_id_str = session_id.to_string();
        drag_source.connect_prepare(move |_source, _x, _y| {
            let value = glib::Value::from(&session_id_str);
            let content = gdk::ContentProvider::for_value(&value);
            Some(content)
        });

        tab_box.add_controller(drag_source);

        // Set tooltip with full name and host
        let tooltip = if host.is_empty() {
            format!("{title}\nDrag to split pane")
        } else {
            format!("{title}\n{host}\nDrag to split pane")
        };
        tab_box.set_tooltip_text(Some(&tooltip));

        // Store tab label widgets for adaptive display
        tab_labels.borrow_mut().insert(
            session_id,
            TabLabelWidgets {
                container: tab_box.clone(),
                icon: icon.clone(),
                label: label.clone(),
                full_name: title.to_string(),
            },
        );

        // Add to overflow menu
        Self::add_to_overflow_menu(
            overflow_box,
            session_id,
            title,
            host,
            icon_name,
            notebook,
            sessions,
        );

        tab_box
    }

    /// Adds a session entry to the overflow menu
    fn add_to_overflow_menu(
        overflow_box: &GtkBox,
        session_id: Uuid,
        title: &str,
        host: &str,
        icon_name: &str,
        notebook: &Notebook,
        sessions: &Rc<RefCell<HashMap<Uuid, u32>>>,
    ) {
        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.add_css_class("overflow-item");
        row.set_margin_start(4);
        row.set_margin_end(4);
        row.set_margin_top(2);
        row.set_margin_bottom(2);

        let icon = Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        row.append(&icon);

        let label = Label::new(Some(title));
        label.set_hexpand(true);
        label.set_halign(gtk4::Align::Start);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&label);

        // Set tooltip
        let tooltip = if host.is_empty() {
            title.to_string()
        } else {
            format!("{title}\n{host}")
        };
        row.set_tooltip_text(Some(&tooltip));

        // Make clickable - switch to this tab
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gdk::BUTTON_PRIMARY);
        let notebook_weak = notebook.downgrade();
        let sessions_clone = sessions.clone();
        gesture.connect_released(move |gesture, _, _, _| {
            if let Some(notebook) = notebook_weak.upgrade() {
                let sessions = sessions_clone.borrow();
                if let Some(&page_num) = sessions.get(&session_id) {
                    notebook.set_current_page(Some(page_num));
                }
            }
            // Close popover
            if let Some(widget) = gesture.widget() {
                if let Some(popover) = widget.ancestor(Popover::static_type()) {
                    if let Some(popover) = popover.downcast_ref::<Popover>() {
                        popover.popdown();
                    }
                }
            }
        });
        row.add_controller(gesture);

        // Store session_id in widget name for removal
        row.set_widget_name(&session_id.to_string());

        overflow_box.append(&row);
    }

    /// Removes a session from the overflow menu
    fn remove_from_overflow_menu(overflow_box: &GtkBox, session_id: Uuid) {
        let session_str = session_id.to_string();
        let mut child = overflow_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if widget.widget_name() == session_str {
                overflow_box.remove(&widget);
                break;
            }
            child = next;
        }
    }

    /// Closes a terminal tab by session ID
    pub fn close_tab(&self, session_id: Uuid) {
        // Get page_num and remove from sessions map first
        let page_num = self.sessions.borrow_mut().remove(&session_id);

        // Clean up terminal and session info before removing page
        // (to avoid issues with switch_page signal)
        self.terminals.borrow_mut().remove(&session_id);
        self.session_widgets.borrow_mut().remove(&session_id);
        self.session_info.borrow_mut().remove(&session_id);

        // Remove from tab_labels and overflow menu
        self.tab_labels.borrow_mut().remove(&session_id);
        Self::remove_from_overflow_menu(&self.overflow_box, session_id);

        // Now remove the page (this may trigger switch_page signal)
        if let Some(page_num) = page_num {
            self.notebook.remove_page(Some(page_num));

            // Update page numbers for remaining sessions
            // (pages after the removed one shift down by 1)
            let mut sessions = self.sessions.borrow_mut();
            for (_, num) in sessions.iter_mut() {
                if *num > page_num {
                    *num -= 1;
                }
            }
            let is_empty = sessions.is_empty();
            drop(sessions);

            // Restore Welcome tab if no sessions remain
            if is_empty {
                let welcome = Self::create_welcome_tab();
                let welcome_label = Label::new(Some("Welcome"));
                self.notebook.append_page(&welcome, Some(&welcome_label));
                self.notebook.set_current_page(Some(0));
            }
        }
    }

    /// Marks a tab as disconnected (changes label color to red)
    pub fn mark_tab_disconnected(&self, session_id: Uuid) {
        if let Some(widgets) = self.tab_labels.borrow().get(&session_id) {
            widgets.label.remove_css_class("tab-label");
            widgets.label.add_css_class("tab-label-disconnected");
        }
    }

    /// Marks a tab as connected (restores normal label color)
    pub fn mark_tab_connected(&self, session_id: Uuid) {
        if let Some(widgets) = self.tab_labels.borrow().get(&session_id) {
            widgets.label.remove_css_class("tab-label-disconnected");
            widgets.label.add_css_class("tab-label");
        }
    }

    /// Gets the terminal widget for a session
    #[must_use]
    pub fn get_terminal(&self, session_id: Uuid) -> Option<Terminal> {
        self.terminals.borrow().get(&session_id).cloned()
    }

    /// Gets the cursor row of a terminal session
    pub fn get_terminal_cursor_row(&self, session_id: Uuid) -> Option<i64> {
        if let Some(terminal) = self.get_terminal(session_id) {
            let (row, _col) = terminal.cursor_position();
            return Some(row);
        }
        None
    }

    /// Gets session info for a session
    #[must_use]
    pub fn get_session_info(&self, session_id: Uuid) -> Option<TerminalSession> {
        self.session_info.borrow().get(&session_id).cloned()
    }

    /// Sets the log file path for a session
    pub fn set_log_file(&self, session_id: Uuid, log_file: PathBuf) {
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.log_file = Some(log_file);
        }
    }

    /// Copies selected text from the active terminal to clipboard
    pub fn copy_to_clipboard(&self) {
        if let Some(terminal) = self.get_active_terminal() {
            terminal.copy_clipboard_format(vte4::Format::Text);
        }
    }

    /// Pastes text from clipboard to the active terminal
    pub fn paste_from_clipboard(&self) {
        if let Some(terminal) = self.get_active_terminal() {
            terminal.paste_clipboard();
        }
    }

    /// Gets the terminal for the currently active tab
    #[must_use]
    pub fn get_active_terminal(&self) -> Option<Terminal> {
        let current_page = self.notebook.current_page()?;
        let sessions = self.sessions.borrow();

        // Find the session ID for the current page
        for (session_id, &page_num) in sessions.iter() {
            if page_num == current_page {
                return self.terminals.borrow().get(session_id).cloned();
            }
        }
        None
    }

    /// Gets the session ID for the currently active tab
    #[must_use]
    pub fn get_active_session_id(&self) -> Option<Uuid> {
        let current_page = self.notebook.current_page()?;
        self.get_session_id_for_page(current_page)
    }

    /// Gets the session ID for a specific page number
    #[must_use]
    pub fn get_session_id_for_page(&self, page_num: u32) -> Option<Uuid> {
        let sessions = self.sessions.borrow();
        for (session_id, &stored_page) in sessions.iter() {
            if stored_page == page_num {
                return Some(*session_id);
            }
        }
        None
    }

    /// Sends text to the active terminal
    pub fn send_text(&self, text: &str) {
        if let Some(terminal) = self.get_active_terminal() {
            terminal.feed_child(text.as_bytes());
        }
    }

    /// Sends text to a specific terminal session
    pub fn send_text_to_session(&self, session_id: Uuid, text: &str) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed_child(text.as_bytes());
        }
    }

    /// Displays output text in a specific terminal session
    ///
    /// Unlike `send_text_to_session`, this displays text as terminal output
    /// rather than sending it as input to the running process.
    /// Useful for displaying feedback messages before command execution.
    pub fn display_output(&self, session_id: Uuid, text: &str) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed(text.as_bytes());
        }
    }

    /// Returns the main container widget for this notebook
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the notebook widget (for internal use)
    #[must_use]
    pub fn notebook(&self) -> &Notebook {
        &self.notebook
    }

    /// Returns the number of open tabs
    #[must_use]
    pub fn tab_count(&self) -> u32 {
        self.notebook.n_pages()
    }

    /// Returns the number of active sessions (excluding Welcome tab)
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.borrow().len()
    }

    /// Sets vexpand for all notebook page contents
    ///
    /// This is used to control whether notebook pages expand vertically.
    /// When showing SSH sessions in split view, we want notebook pages collapsed.
    /// When showing VNC/RDP/SPICE sessions, we want the active page expanded.
    pub fn set_pages_vexpand(&self, _expand: bool) {
        // Don't modify individual page vexpand - it causes issues when switching
        // between different session types. Instead, control the notebook itself.
    }

    /// Shows only the specified page content, hides all others
    ///
    /// This ensures that inactive VNC/RDP pages don't affect layout sizing.
    pub fn show_only_current_page(&self) {
        // No-op - hiding pages causes RDP disconnection issues
    }

    /// Shows all page contents (for VNC/RDP/SPICE mode)
    pub fn show_all_pages(&self) {
        // No-op - all pages are always visible
    }

    /// Switches to a specific tab by session ID
    pub fn switch_to_tab(&self, session_id: Uuid) {
        let sessions = self.sessions.borrow();
        if let Some(&page_num) = sessions.get(&session_id) {
            self.notebook.set_current_page(Some(page_num));
        }
    }

    /// Returns all session IDs
    #[must_use]
    pub fn session_ids(&self) -> Vec<Uuid> {
        self.sessions.borrow().keys().copied().collect()
    }

    /// Connects a callback for when a terminal child exits
    pub fn connect_child_exited<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn(i32) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_child_exited(move |_terminal, status| {
                callback(status);
            });
        }
    }

    /// Connects a callback for terminal output (for logging)
    pub fn connect_contents_changed<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn() + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_contents_changed(move |_terminal| {
                callback();
            });
        }
    }

    /// Adds an embedded session tab (for RDP/VNC external processes)
    ///
    /// This method adds a pre-created widget to the notebook for sessions
    /// that use external processes (like xfreerdp) instead of native embedding.
    pub fn add_embedded_session_tab(&self, session_id: Uuid, title: &str, widget: &GtkBox) {
        let is_first_session = self.sessions.borrow().is_empty();

        // Remove Welcome tab if this is the first session
        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        // Create tab label with close button
        let tab_label = Self::create_tab_label(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            &self.tab_labels,
            &self.overflow_box,
        );

        // Add the page
        let page_num = self.notebook.append_page(widget, Some(&tab_label));

        // Store session mapping
        self.sessions.borrow_mut().insert(session_id, page_num);

        // Store session info
        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id: session_id, // Will be updated by caller if needed
                name: title.to_string(),
                protocol: "embedded".to_string(),
                is_embedded: false, // External process
                log_file: None,
            },
        );

        // Make the tab reorderable
        self.notebook.set_tab_reorderable(widget, true);

        // Switch to the new tab
        self.notebook.set_current_page(Some(page_num));
    }

    /// Adds an embedded RDP tab with the EmbeddedRdpWidget
    ///
    /// This method adds a pre-created EmbeddedRdpWidget to the notebook for
    /// embedded RDP sessions with dynamic resolution support.
    ///
    /// The `EmbeddedRdpWidget` is stored in `session_widgets` to keep it alive
    /// for the duration of the session. This is important because the widget
    /// contains `Rc<RefCell<...>>` fields that are captured by the draw function
    /// closure.
    pub fn add_embedded_rdp_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<EmbeddedRdpWidget>,
    ) {
        let is_first_session = self.sessions.borrow().is_empty();

        // Remove Welcome tab if this is the first session
        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        // Create container for the widget
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(widget.widget());

        // Create tab label with RDP protocol icon
        let tab_label = Self::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "rdp",
            "",
            &self.tab_labels,
            &self.overflow_box,
        );

        // Add the page
        let page_num = self.notebook.append_page(&container, Some(&tab_label));

        // Store session mapping
        self.sessions.borrow_mut().insert(session_id, page_num);

        // Store the widget to keep it alive
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::EmbeddedRdp(widget));

        // Store session info
        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "rdp".to_string(),
                is_embedded: true, // Embedded RDP widget
                log_file: None,
            },
        );

        // Make the tab reorderable
        self.notebook.set_tab_reorderable(&container, true);

        // Switch to the new tab
        self.notebook.set_current_page(Some(page_num));
    }

    /// Hides content of all notebook pages except the specified one
    ///
    /// This is used to prevent VNC/RDP/SPICE page content from taking space
    /// when the notebook is constrained to show only tabs (for terminal sessions).
    /// When switching to a terminal session, we hide all embedded session content
    /// so the notebook can properly shrink to tabs-only height.
    ///
    /// Note: We hide the children of the page container, not the page itself,
    /// to keep the tabs visible.
    pub fn hide_all_page_content_except(&self, except_page: Option<u32>) {
        let n_pages = self.notebook.n_pages();
        for page_num in 0..n_pages {
            if let Some(page_widget) = self.notebook.nth_page(Some(page_num)) {
                let should_show = except_page == Some(page_num);
                // Hide children of the page, not the page itself (to keep tabs visible)
                if let Some(container) = page_widget.downcast_ref::<GtkBox>() {
                    let mut child = container.first_child();
                    while let Some(widget) = child {
                        widget.set_visible(should_show);
                        child = widget.next_sibling();
                    }
                }
            }
        }
    }

    /// Shows content of a specific notebook page
    ///
    /// Used when switching to a VNC/RDP/SPICE session to show its content.
    pub fn show_page_content(&self, page_num: u32) {
        if let Some(page_widget) = self.notebook.nth_page(Some(page_num)) {
            // Show children of the page container
            if let Some(container) = page_widget.downcast_ref::<GtkBox>() {
                let mut child = container.first_child();
                while let Some(widget) = child {
                    widget.set_visible(true);
                    child = widget.next_sibling();
                }
            }
        }
    }

    /// Hides content of all notebook pages
    ///
    /// Used when switching to a terminal session to ensure notebook
    /// only shows tabs without any page content taking space.
    pub fn hide_all_page_content(&self) {
        let n_pages = self.notebook.n_pages();
        for page_num in 0..n_pages {
            if let Some(page_widget) = self.notebook.nth_page(Some(page_num)) {
                // Hide children of the page, not the page itself (to keep tabs visible)
                if let Some(container) = page_widget.downcast_ref::<GtkBox>() {
                    let mut child = container.first_child();
                    while let Some(widget) = child {
                        widget.set_visible(false);
                        child = widget.next_sibling();
                    }
                }
            }
        }
    }
}

impl Default for TerminalNotebook {
    fn default() -> Self {
        Self::new()
    }
}
