//! Main application window
//!
//! This module provides the main window implementation for `RustConn`,
//! including the header bar, sidebar, terminal area, and action handling.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{gio, glib, Orientation, Paned};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;
use vte4::prelude::*;

use crate::alert;
use crate::toast::ToastOverlay;
use crate::window_clusters as clusters;
use crate::window_document_actions as doc_actions;
use crate::window_edit_dialogs as edit_dialogs;
use crate::window_groups as groups;
use crate::window_operations as operations;
use crate::window_protocols as protocols;
use crate::window_rdp_vnc as rdp_vnc;
use crate::window_sessions as sessions;
use crate::window_snippets as snippets;
use crate::window_templates as templates;
use crate::window_types::{
    get_protocol_string, SessionSplitBridges, SharedExternalWindowManager, SharedNotebook,
    SharedSidebar, SharedSplitView, SharedTabSplitManager,
};
use crate::window_ui;

use crate::dialogs::{ExportDialog, SettingsDialog};
use crate::external_window::ExternalWindowManager;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::split_view::{SplitDirection, SplitViewBridge, TabSplitManager};
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use rustconn_core::split::ColorPool;

/// Shared color pool type for global color allocation across all split containers
type SharedColorPool = Rc<RefCell<ColorPool>>;

/// Shared toast overlay reference
pub type SharedToastOverlay = Rc<ToastOverlay>;

/// Main application window wrapper
///
/// Provides access to the main window and its components.
#[allow(dead_code)] // Fields kept for GTK widget lifecycle and future use
pub struct MainWindow {
    window: adw::ApplicationWindow,
    sidebar: SharedSidebar,
    terminal_notebook: SharedNotebook,
    split_view: SharedSplitView,
    /// New tab-scoped split manager (for migration to new split view system)
    tab_split_manager: SharedTabSplitManager,
    /// Per-session split bridges - each session that has been split gets its own bridge
    /// Requirement 3: Each tab maintains its own independent split layout
    session_split_bridges: SessionSplitBridges,
    /// Global color pool shared across all split containers
    /// Ensures different split containers get different colors
    global_color_pool: SharedColorPool,
    /// Container for split views - we swap which bridge is visible based on active session
    split_container: gtk4::Box,
    state: SharedAppState,
    paned: Paned,
    external_window_manager: SharedExternalWindowManager,
    toast_overlay: SharedToastOverlay,
}

impl MainWindow {
    /// Creates a new main window for the application
    #[must_use]
    pub fn new(app: &adw::Application, state: SharedAppState) -> Self {
        // Register custom icon from assets before creating window
        Self::register_app_icon();

        // Create the main window
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("RustConn")
            .default_width(1200)
            .default_height(800)
            .icon_name("io.github.totoshko88.RustConn")
            .build();

        // Apply saved window geometry if available
        {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            if settings.ui.remember_window_geometry {
                if let (Some(width), Some(height)) =
                    (settings.ui.window_width, settings.ui.window_height)
                {
                    if width > 0 && height > 0 {
                        window.set_default_size(width, height);
                    }
                }
            }
        }

        // Create header bar
        let header_bar = window_ui::create_header_bar();

        // Create the main layout with paned container
        let paned = Paned::new(Orientation::Horizontal);

        // Apply saved sidebar width (with reasonable limits)
        {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let sidebar_width = settings.ui.sidebar_width.unwrap_or(280).clamp(150, 500);
            paned.set_position(sidebar_width);
        }

        // Sidebar should not resize when window resizes - only content area should
        paned.set_resize_start_child(false);
        paned.set_resize_end_child(true);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);

        // Create sidebar
        let sidebar = Rc::new(ConnectionSidebar::new());
        paned.set_start_child(Some(sidebar.widget()));

        // Load persisted search history
        {
            let state_ref = state.borrow();
            let search_history = &state_ref.settings().ui.search_history;
            sidebar.load_search_history(search_history);
        }

        // Create global color pool shared across all split containers
        // This ensures different split containers get different colors
        let global_color_pool: SharedColorPool = Rc::new(RefCell::new(ColorPool::new()));

        // Create split terminal view as the main terminal container
        // Uses the global color pool for consistent color allocation
        let split_view = Rc::new(SplitViewBridge::with_color_pool(Rc::clone(
            &global_color_pool,
        )));

        // Create new tab-scoped split manager (for migration to new split view system)
        let tab_split_manager: SharedTabSplitManager =
            Rc::new(RefCell::new(TabSplitManager::new()));

        // Create per-session split bridges map
        // Requirement 3: Each tab maintains its own independent split layout
        let session_split_bridges: SessionSplitBridges =
            Rc::new(RefCell::new(std::collections::HashMap::new()));

        // Create container for split views - we swap which bridge is visible based on active session
        let split_container = gtk4::Box::new(Orientation::Vertical, 0);
        split_container.set_vexpand(true);
        split_container.set_hexpand(true);

        // Create terminal notebook for tab management (using adw::TabView)
        let terminal_notebook = Rc::new(TerminalNotebook::new());

        // TabView/TabBar configuration is handled internally
        // Don't let notebook expand - it should only show tabs
        terminal_notebook.widget().set_vexpand(false);
        // Ensure notebook is visible (TabBar)
        terminal_notebook.widget().set_visible(true);
        // Hide TabView content initially - split_view shows welcome content
        terminal_notebook.hide_tab_view_content();

        // Create a container for the terminal area
        let terminal_container = gtk4::Box::new(Orientation::Vertical, 0);
        terminal_container.set_vexpand(true);
        terminal_container.set_hexpand(true);

        // Add notebook tabs at top for session switching (tabs only, content hidden by size)
        terminal_container.append(terminal_notebook.widget());

        // Add split view as the main content area - takes full space
        split_view.widget().set_vexpand(true);
        split_view.widget().set_hexpand(true);
        terminal_container.append(split_view.widget());

        // Add split_container for per-session split views (initially hidden)
        split_container.set_visible(false);
        terminal_container.append(&split_container);

        // Note: drag-and-drop is set up in connect_signals after we have access to notebook

        paned.set_end_child(Some(&terminal_container));

        // Create toast overlay and wrap the paned container
        let toast_overlay = Rc::new(ToastOverlay::new());
        toast_overlay.set_child(Some(&paned));

        // Create main layout using adw::ToolbarView for proper libadwaita integration
        // This provides better responsive behavior and follows GNOME HIG
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header_bar);
        toolbar_view.set_content(Some(toast_overlay.widget()));

        window.set_content(Some(&toolbar_view));

        // Create external window manager
        let external_window_manager = Rc::new(ExternalWindowManager::new());

        let main_window = Self {
            window,
            sidebar,
            terminal_notebook,
            split_view,
            tab_split_manager,
            session_split_bridges,
            global_color_pool,
            split_container,
            state,
            paned,
            external_window_manager,
            toast_overlay,
        };

        // Set up window actions
        main_window.setup_actions();

        // Load initial data
        main_window.load_connections();

        // Initialize KeePass button status
        main_window.update_keepass_button_status();

        // Connect signals
        main_window.connect_signals();

        main_window
    }

    /// Sets up window actions
    fn setup_actions(&self) {
        let window = &self.window;
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();

        // Set up action groups
        self.setup_connection_actions(window, &state, &sidebar, &terminal_notebook);
        self.setup_edit_actions(window, &state, &sidebar);
        self.setup_terminal_actions(window, &terminal_notebook, &sidebar, &state);
        self.setup_navigation_actions(window, &terminal_notebook, &sidebar);
        self.setup_group_operations_actions(window, &state, &sidebar);
        self.setup_snippet_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_cluster_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_template_actions(window, &state, &sidebar);
        self.setup_split_view_actions(window);
        self.setup_document_actions(window, &state, &sidebar);
        self.setup_history_actions(window, &state);
        self.setup_misc_actions(window, &state, &sidebar, &terminal_notebook);
    }

    /// Sets up connection-related actions (new, import, settings)
    fn setup_connection_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
    ) {
        // New connection action
        let new_conn_action = gio::SimpleAction::new("new-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_conn_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_new_connection_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&new_conn_action);

        // New group action
        let new_group_action = gio::SimpleAction::new("new-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_new_group_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&new_group_action);

        // Import action
        let import_action = gio::SimpleAction::new("import", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        import_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_import_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&import_action);

        // Settings action
        let settings_action = gio::SimpleAction::new("settings", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        settings_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_settings_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                );
            }
        });
        window.add_action(&settings_action);

        // Open password vault action - opens the configured password manager
        let open_keepass_action = gio::SimpleAction::new("open-keepass", None);
        let state_clone = state.clone();
        open_keepass_action.connect_activate(move |_, _| {
            let state_ref = state_clone.borrow();
            let settings = state_ref.settings();
            let backend = settings.secrets.preferred_backend;
            drop(state_ref);

            // Open the password manager for the configured backend
            if let Err(e) = rustconn_core::secret::open_password_manager(&backend) {
                eprintln!("Failed to open password manager: {e}");
            }
        });
        // Enable based on backend type - always enabled for libsecret/bitwarden,
        // for KeePassXC/KdbxFile requires kdbx_enabled and valid path
        let settings = state.borrow().settings().clone();
        let action_enabled = match settings.secrets.preferred_backend {
            rustconn_core::config::SecretBackendType::LibSecret
            | rustconn_core::config::SecretBackendType::Bitwarden => true,
            rustconn_core::config::SecretBackendType::KeePassXc
            | rustconn_core::config::SecretBackendType::KdbxFile => {
                settings.secrets.kdbx_enabled
                    && settings
                        .secrets
                        .kdbx_path
                        .as_ref()
                        .is_some_and(|p| p.exists())
            }
        };
        open_keepass_action.set_enabled(action_enabled);
        window.add_action(&open_keepass_action);

        // Export action
        let export_action = gio::SimpleAction::new("export", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        export_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_export_dialog(win.upcast_ref(), state_clone.clone());
            }
        });
        window.add_action(&export_action);
    }

    /// Sets up edit-related actions (edit, delete, duplicate, move)
    fn setup_edit_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Connect action
        let connect_action = gio::SimpleAction::new("connect", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = self.terminal_notebook.clone();
        connect_action.connect_activate(move |_, _| {
            Self::connect_selected(&state_clone, &sidebar_clone, &notebook_clone);
        });
        window.add_action(&connect_action);

        // Edit connection action
        let edit_action = gio::SimpleAction::new("edit-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        edit_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::edit_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&edit_action);

        // Delete connection action
        let delete_action = gio::SimpleAction::new("delete-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        delete_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::delete_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&delete_action);

        // Duplicate connection action
        let duplicate_action = gio::SimpleAction::new("duplicate-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        duplicate_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::duplicate_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&duplicate_action);

        // Move to group action
        let move_to_group_action = gio::SimpleAction::new("move-to-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        move_to_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                groups::show_move_to_group_dialog(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&move_to_group_action);

        // View details action
        let view_details_action = gio::SimpleAction::new("view-details", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let split_view_clone = self.split_view.clone();
        view_details_action.connect_activate(move |_, _| {
            Self::show_connection_details(&state_clone, &sidebar_clone, &split_view_clone);
        });
        window.add_action(&view_details_action);

        // Rename item action (works for both connections and groups)
        let rename_action = gio::SimpleAction::new("rename-item", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        rename_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::rename_selected_item(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&rename_action);

        // Copy connection action
        let copy_connection_action = gio::SimpleAction::new("copy-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        copy_connection_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::copy_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&copy_connection_action);

        // Paste connection action
        let paste_connection_action = gio::SimpleAction::new("paste-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        paste_connection_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::paste_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&paste_connection_action);
    }

    /// Sets up terminal-related actions (copy, paste, close tab)
    fn setup_terminal_actions(
        &self,
        window: &adw::ApplicationWindow,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        state: &SharedAppState,
    ) {
        // Search action
        let search_action = gio::SimpleAction::new("search", None);
        let sidebar_clone = sidebar.clone();
        search_action.connect_activate(move |_, _| {
            sidebar_clone.search_entry().grab_focus();
        });
        window.add_action(&search_action);

        // Copy action - works with split view's focused session for SSH
        let copy_action = gio::SimpleAction::new("copy", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        copy_action.connect_activate(move |_, _| {
            // Try split view's focused session first (for SSH in split panes)
            if let Some(session_id) = split_view_clone.get_focused_session() {
                if let Some(terminal) = notebook_clone.get_terminal(session_id) {
                    terminal.copy_clipboard_format(vte4::Format::Text);
                    return;
                }
            }
            // Fall back to TabView's active terminal (for RDP/VNC/SPICE)
            notebook_clone.copy_to_clipboard();
        });
        window.add_action(&copy_action);

        // Paste action - works with split view's focused session for SSH
        let paste_action = gio::SimpleAction::new("paste", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        paste_action.connect_activate(move |_, _| {
            // Try split view's focused session first (for SSH in split panes)
            if let Some(session_id) = split_view_clone.get_focused_session() {
                if let Some(terminal) = notebook_clone.get_terminal(session_id) {
                    terminal.paste_clipboard();
                    return;
                }
            }
            // Fall back to TabView's active terminal (for RDP/VNC/SPICE)
            notebook_clone.paste_from_clipboard();
        });
        window.add_action(&paste_action);

        // Terminal search action
        let terminal_search_action = gio::SimpleAction::new("terminal-search", None);
        let notebook_clone = terminal_notebook.clone();
        let window_weak = window.downgrade();
        terminal_search_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_terminal_search_dialog(win.upcast_ref(), &notebook_clone);
            }
        });
        window.add_action(&terminal_search_action);

        // Close tab action - closes the currently active session tab
        let close_tab_action = gio::SimpleAction::new("close-tab", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = self.sidebar.clone();
        close_tab_action.connect_activate(move |_, _| {
            if let Some(session_id) = notebook_clone.get_active_session_id() {
                // Get connection ID before closing
                let connection_id = notebook_clone
                    .get_session_info(session_id)
                    .map(|info| info.connection_id);

                // Clear from split view first
                split_view_clone.clear_session_from_panes(session_id);
                // Then close the tab
                notebook_clone.close_tab(session_id);
                // Decrement session count in sidebar if we have a connection ID
                if let Some(conn_id) = connection_id {
                    sidebar_clone.decrement_session_count(&conn_id.to_string(), false);
                }

                // After closing, the selected-page handler will take care of
                // showing the correct content for the new active session.
                // We only need to handle RDP redraw here since it's not handled
                // by the selected-page handler.
                if let Some(new_session_id) = notebook_clone.get_active_session_id() {
                    if let Some(info) = notebook_clone.get_session_info(new_session_id) {
                        if info.protocol == "rdp" {
                            // Trigger redraw for RDP widget
                            notebook_clone.queue_rdp_redraw(new_session_id);
                        }
                    }
                }
            }
        });
        window.add_action(&close_tab_action);

        // Close tab by ID action - closes a specific session tab without switching first
        let close_tab_by_id_action =
            gio::SimpleAction::new("close-tab-by-id", Some(glib::VariantTy::STRING));
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = self.sidebar.clone();
        close_tab_by_id_action.connect_activate(move |_, param| {
            if let Some(param) = param {
                if let Some(session_id_str) = param.get::<String>() {
                    if let Ok(session_id) = uuid::Uuid::parse_str(&session_id_str) {
                        // Get the currently active session BEFORE closing
                        // This is important because page numbers will shift after removal
                        let current_active_session = notebook_clone.get_active_session_id();
                        let is_closing_active = current_active_session == Some(session_id);

                        // Get connection ID before closing
                        let connection_id = notebook_clone
                            .get_session_info(session_id)
                            .map(|info| info.connection_id);

                        // Clear tab color indicator
                        notebook_clone.clear_tab_split_color(session_id);

                        // Close session from split view with auto-cleanup
                        // Returns true if split view should be hidden
                        let should_hide_split =
                            split_view_clone.close_session_from_panes(session_id);

                        // Then close the tab
                        notebook_clone.close_tab(session_id);

                        // Decrement session count in sidebar if we have a connection ID
                        if let Some(conn_id) = connection_id {
                            sidebar_clone.decrement_session_count(&conn_id.to_string(), false);
                        }

                        // Hide split view if no sessions remain in panes
                        if should_hide_split {
                            split_view_clone.widget().set_visible(false);
                            split_view_clone.widget().set_vexpand(false);
                            notebook_clone.widget().set_vexpand(true);
                            notebook_clone.show_tab_view_content();
                        }

                        // The selected-page handler will take care of showing the correct
                        // content for the new active session. We only need to handle
                        // special cases here.
                        let notebook_for_idle = notebook_clone.clone();
                        if is_closing_active {
                            // We closed the active tab - selected-page handler will fire
                            // Just handle RDP redraw
                            glib::idle_add_local_once(move || {
                                if let Some(new_session_id) =
                                    notebook_for_idle.get_active_session_id()
                                {
                                    if let Some(info) =
                                        notebook_for_idle.get_session_info(new_session_id)
                                    {
                                        if info.protocol == "rdp" {
                                            notebook_for_idle.queue_rdp_redraw(new_session_id);
                                        }
                                    }
                                }
                            });
                        } else if let Some(active_id) = current_active_session {
                            // We closed a non-active tab, ensure we stay on the active tab
                            // Defer to next main loop iteration to override switch-page effects
                            glib::idle_add_local_once(move || {
                                notebook_for_idle.switch_to_tab(active_id);
                                if let Some(info) = notebook_for_idle.get_session_info(active_id) {
                                    if info.protocol == "rdp" {
                                        notebook_for_idle.queue_rdp_redraw(active_id);
                                    } else if info.protocol == "vnc" {
                                        if let Some(vnc_widget) =
                                            notebook_for_idle.get_vnc_widget(active_id)
                                        {
                                            vnc_widget.widget().queue_draw();
                                        }
                                    } else if info.protocol == "spice" {
                                        if let Some(spice_widget) =
                                            notebook_for_idle.get_spice_widget(active_id)
                                        {
                                            spice_widget.widget().queue_draw();
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            }
        });
        window.add_action(&close_tab_by_id_action);

        // Local shell action
        let local_shell_action = gio::SimpleAction::new("local-shell", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let state_clone = state.clone();
        local_shell_action.connect_activate(move |_, _| {
            Self::open_local_shell_with_split(
                &notebook_clone,
                &split_view_clone,
                Some(&state_clone),
            );
        });
        window.add_action(&local_shell_action);

        // Quick connect action
        let quick_connect_action = gio::SimpleAction::new("quick-connect", None);
        let window_weak = window.downgrade();
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = self.sidebar.clone();
        quick_connect_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_quick_connect_dialog(
                    win.upcast_ref(),
                    notebook_clone.clone(),
                    split_view_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&quick_connect_action);
    }

    /// Sets up navigation actions (focus, tabs)
    fn setup_navigation_actions(
        &self,
        window: &adw::ApplicationWindow,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // Focus sidebar action
        let focus_sidebar_action = gio::SimpleAction::new("focus-sidebar", None);
        let sidebar_clone = sidebar.clone();
        focus_sidebar_action.connect_activate(move |_, _| {
            sidebar_clone.list_view().grab_focus();
        });
        window.add_action(&focus_sidebar_action);

        // Focus terminal action
        let focus_terminal_action = gio::SimpleAction::new("focus-terminal", None);
        let notebook_clone = terminal_notebook.clone();
        focus_terminal_action.connect_activate(move |_, _| {
            if let Some(terminal) = notebook_clone.get_active_terminal() {
                terminal.grab_focus();
            }
        });
        window.add_action(&focus_terminal_action);

        // Next tab action
        let next_tab_action = gio::SimpleAction::new("next-tab", None);
        let notebook_clone = terminal_notebook.clone();
        next_tab_action.connect_activate(move |_, _| {
            let tab_view = notebook_clone.tab_view();
            let n_pages = tab_view.n_pages();
            if n_pages > 0 {
                if let Some(selected) = tab_view.selected_page() {
                    let current_pos = tab_view.page_position(&selected);
                    let next_pos = (current_pos + 1) % n_pages;
                    let next_page = tab_view.nth_page(next_pos);
                    tab_view.set_selected_page(&next_page);
                }
            }
        });
        window.add_action(&next_tab_action);

        // Previous tab action
        let prev_tab_action = gio::SimpleAction::new("prev-tab", None);
        let notebook_clone = terminal_notebook.clone();
        prev_tab_action.connect_activate(move |_, _| {
            let tab_view = notebook_clone.tab_view();
            let n_pages = tab_view.n_pages();
            if n_pages > 0 {
                if let Some(selected) = tab_view.selected_page() {
                    let current_pos = tab_view.page_position(&selected);
                    let prev_pos = if current_pos == 0 {
                        n_pages - 1
                    } else {
                        current_pos - 1
                    };
                    let prev_page = tab_view.nth_page(prev_pos);
                    tab_view.set_selected_page(&prev_page);
                }
            }
        });
        window.add_action(&prev_tab_action);

        // Toggle fullscreen action
        let toggle_fullscreen_action = gio::SimpleAction::new("toggle-fullscreen", None);
        let window_weak = window.downgrade();
        toggle_fullscreen_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                if win.is_fullscreen() {
                    win.unfullscreen();
                } else {
                    win.fullscreen();
                }
            }
        });
        window.add_action(&toggle_fullscreen_action);
    }

    /// Sets up group operations actions (select all, delete selected, etc.)
    fn setup_group_operations_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Group operations action (toggle mode)
        let group_ops_action =
            gio::SimpleAction::new_stateful("group-operations", None, &false.to_variant());
        let sidebar_clone = sidebar.clone();
        group_ops_action.connect_activate(move |action, _| {
            let current = action
                .state()
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false);
            action.set_state(&(!current).to_variant());
            Self::toggle_group_operations_mode(&sidebar_clone, !current);
        });
        window.add_action(&group_ops_action);

        // Select all action
        let select_all_action = gio::SimpleAction::new("select-all", None);
        let sidebar_clone = sidebar.clone();
        select_all_action.connect_activate(move |_, _| {
            if sidebar_clone.is_group_operations_mode() {
                sidebar_clone.select_all();
            }
        });
        window.add_action(&select_all_action);

        // Clear selection action
        let clear_selection_action = gio::SimpleAction::new("clear-selection", None);
        let sidebar_clone = sidebar.clone();
        clear_selection_action.connect_activate(move |_, _| {
            sidebar_clone.clear_selection();
        });
        window.add_action(&clear_selection_action);

        // Delete selected action
        let delete_selected_action = gio::SimpleAction::new("delete-selected", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        delete_selected_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::delete_selected_connections(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&delete_selected_action);

        // Move selected to group action
        let move_selected_action = gio::SimpleAction::new("move-selected-to-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        move_selected_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_move_selected_to_group_dialog(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                );
            }
        });
        window.add_action(&move_selected_action);

        // Sort connections action
        let sort_action = gio::SimpleAction::new("sort-connections", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sort_action.connect_activate(move |_, _| {
            Self::sort_connections(&state_clone, &sidebar_clone);
        });
        window.add_action(&sort_action);

        // Sort recent action
        let sort_recent_action = gio::SimpleAction::new("sort-recent", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sort_recent_action.connect_activate(move |_, _| {
            Self::sort_recent(&state_clone, &sidebar_clone);
        });
        window.add_action(&sort_recent_action);
    }

    /// Sets up snippet-related actions
    fn setup_snippet_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // New snippet action
        let new_snippet_action = gio::SimpleAction::new("new-snippet", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        new_snippet_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                snippets::show_new_snippet_dialog(win.upcast_ref(), state_clone.clone());
            }
        });
        window.add_action(&new_snippet_action);

        // Manage snippets action
        let manage_snippets_action = gio::SimpleAction::new("manage-snippets", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        manage_snippets_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                snippets::show_snippets_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                );
            }
        });
        window.add_action(&manage_snippets_action);

        // Execute snippet action
        let execute_snippet_action = gio::SimpleAction::new("execute-snippet", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        execute_snippet_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                snippets::show_snippet_picker(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                );
            }
        });
        window.add_action(&execute_snippet_action);

        // Run snippet for selected connection (from context menu)
        // First connects to the selected connection, then shows snippet picker
        let run_snippet_for_conn_action =
            gio::SimpleAction::new("run-snippet-for-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        let split_view_clone = self.split_view.clone();
        run_snippet_for_conn_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                // Get selected connection from sidebar
                if let Some(item) = sidebar_clone.get_selected_item() {
                    if item.is_group() {
                        return; // Can't run snippet on a group
                    }

                    // Parse UUID from item id string
                    let id_str = item.id();
                    let Ok(id) = Uuid::parse_str(&id_str) else {
                        return;
                    };

                    // Check if connection is already connected (has active session)
                    let has_active_session = notebook_clone
                        .get_all_sessions()
                        .iter()
                        .any(|s| s.connection_id == id);

                    if has_active_session {
                        // Already connected, just show snippet picker
                        snippets::show_snippet_picker(
                            win.upcast_ref(),
                            state_clone.clone(),
                            notebook_clone.clone(),
                        );
                    } else {
                        // Need to connect first, then show snippet picker
                        // Start connection
                        Self::start_connection_with_split(
                            &state_clone,
                            &notebook_clone,
                            &split_view_clone,
                            &sidebar_clone,
                            id,
                        );

                        // Show snippet picker after a short delay to allow connection to establish
                        let win_for_timeout = win.clone();
                        let state_for_timeout = state_clone.clone();
                        let notebook_for_timeout = notebook_clone.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(500),
                            move || {
                                snippets::show_snippet_picker(
                                    win_for_timeout.upcast_ref(),
                                    state_for_timeout,
                                    notebook_for_timeout,
                                );
                            },
                        );
                    }
                }
            }
        });
        window.add_action(&run_snippet_for_conn_action);

        // Show sessions action
        let show_sessions_action = gio::SimpleAction::new("show-sessions", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        show_sessions_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                sessions::show_sessions_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&show_sessions_action);
    }

    /// Sets up cluster-related actions
    fn setup_cluster_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // New cluster action
        let new_cluster_action = gio::SimpleAction::new("new-cluster", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        new_cluster_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                clusters::show_new_cluster_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                );
            }
        });
        window.add_action(&new_cluster_action);

        // Manage clusters action
        let manage_clusters_action = gio::SimpleAction::new("manage-clusters", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        manage_clusters_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                clusters::show_clusters_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&manage_clusters_action);
    }

    /// Sets up template-related actions
    fn setup_template_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Manage templates action
        let manage_templates_action = gio::SimpleAction::new("manage-templates", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        manage_templates_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                templates::show_templates_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&manage_templates_action);
    }

    /// Sets up history and statistics actions
    fn setup_history_actions(&self, window: &adw::ApplicationWindow, state: &SharedAppState) {
        use crate::dialogs::{show_password_generator_dialog, HistoryDialog, StatisticsDialog};

        // Show history action
        let show_history_action = gio::SimpleAction::new("show-history", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let sidebar_clone = self.sidebar.clone();
        let split_view_clone = self.split_view.clone();
        show_history_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                let entries = state_ref.history_entries().to_vec();
                drop(state_ref);

                let dialog = HistoryDialog::new(Some(&win));
                dialog.set_entries(entries);

                // Connect callback for reconnecting from history
                let state_for_connect = state_clone.clone();
                let notebook_for_connect = notebook_clone.clone();
                let sidebar_for_connect = sidebar_clone.clone();
                let split_view_for_connect = split_view_clone.clone();
                dialog.connect_on_connect(move |entry| {
                    if entry.is_quick_connect() {
                        tracing::warn!("Cannot reconnect to quick connect from history");
                    } else {
                        tracing::info!(
                            "Reconnecting to {} (id: {}) from history",
                            entry.connection_name,
                            entry.connection_id
                        );
                        Self::start_connection_with_split(
                            &state_for_connect,
                            &notebook_for_connect,
                            &split_view_for_connect,
                            &sidebar_for_connect,
                            entry.connection_id,
                        );
                    }
                });

                dialog.present();
            }
        });
        window.add_action(&show_history_action);

        // Show statistics action
        let show_statistics_action = gio::SimpleAction::new("show-statistics", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        show_statistics_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                let all_stats = state_ref.get_all_statistics();
                drop(state_ref);

                let dialog = StatisticsDialog::new(Some(&win));
                dialog.set_overview_statistics(&all_stats);

                // Connect clear statistics callback
                let state_for_clear = state_clone.clone();
                dialog.connect_on_clear(move || {
                    if let Ok(mut state_mut) = state_for_clear.try_borrow_mut() {
                        state_mut.clear_all_statistics();
                        tracing::info!("All connection statistics cleared");
                    }
                });

                dialog.present();
            }
        });
        window.add_action(&show_statistics_action);

        // Password generator action
        let password_generator_action = gio::SimpleAction::new("password-generator", None);
        let window_weak = window.downgrade();
        password_generator_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                show_password_generator_dialog(Some(&win));
            }
        });
        window.add_action(&password_generator_action);
    }

    /// Sets up split view actions
    fn setup_split_view_actions(&self, window: &adw::ApplicationWindow) {
        // Helper function to get or create a split bridge for a session
        // Requirement 3: Each tab maintains its own independent split layout
        // A session gets its own bridge when it initiates a split.
        // If the session is already displayed in another bridge, we still create
        // a new bridge for it (the session will be moved to the new bridge).
        fn get_or_create_session_bridge(
            session_id: Uuid,
            session_split_bridges: &SessionSplitBridges,
            color_pool: &SharedColorPool,
        ) -> Rc<SplitViewBridge> {
            let mut bridges = session_split_bridges.borrow_mut();
            // Check if this session already owns a bridge
            if let Some(bridge) = bridges.get(&session_id) {
                // Session already has its own bridge - use it
                tracing::debug!(
                    "get_or_create_session_bridge: REUSING existing bridge for session {:?}, \
                     pool_ptr={:p}, pool_allocated={}",
                    session_id,
                    &*color_pool.borrow(),
                    color_pool.borrow().allocated_count()
                );
                bridge.clone()
            } else {
                // Create a new bridge for this session with the shared color pool
                // This ensures different split containers get different colors
                tracing::debug!(
                    "get_or_create_session_bridge: CREATING new bridge for session {:?}, \
                     pool_ptr={:p}, pool_allocated={}",
                    session_id,
                    &*color_pool.borrow(),
                    color_pool.borrow().allocated_count()
                );
                let new_bridge = Rc::new(SplitViewBridge::with_color_pool(Rc::clone(color_pool)));
                bridges.insert(session_id, new_bridge.clone());
                new_bridge
            }
        }

        // Split horizontal action
        let split_horizontal_action = gio::SimpleAction::new("split-horizontal", None);
        let session_bridges = self.session_split_bridges.clone();
        let notebook_for_split_h = self.terminal_notebook.clone();
        let split_container_h = self.split_container.clone();
        let global_split_view_h = self.split_view.clone();
        let color_pool_h = self.global_color_pool.clone();
        split_horizontal_action.connect_activate(move |_, _| {
            // Get current active session before splitting
            let Some(current_session) = notebook_for_split_h.get_active_session_id() else {
                return; // No active session to split
            };

            tracing::debug!("split-horizontal: splitting session {:?}", current_session);

            // Get or create a split bridge for this session (with shared color pool)
            let split_view =
                get_or_create_session_bridge(current_session, &session_bridges, &color_pool_h);

            // Check if this is the first split (bridge has only 1 panel)
            // If bridge already has multiple panels, we don't need to show the current session
            // because restore_panel_contents() already restored all terminals
            let is_first_split = split_view.pane_count() == 1;

            // Clone for close callback
            let sv_for_close = split_view.clone();
            if let Some((new_pane_id, new_color_index, original_color_index)) = split_view
                .split_with_close_callback(SplitDirection::Horizontal, move || {
                    let _ = sv_for_close.close_pane();
                })
            {
                tracing::debug!(
                    "split-horizontal: session {:?} got original_color={}, new_color={}, \
                     is_first_split={}",
                    current_session,
                    original_color_index,
                    new_color_index,
                    is_first_split
                );

                let notebook = notebook_for_split_h.clone();
                let notebook_for_drop = notebook_for_split_h.clone();
                let sv_for_click = split_view.clone();
                let nb_for_click = notebook.clone();

                // Per spec: Split transforms current tab into Container Tab
                // Only show current session in the original pane if this is the FIRST split
                // For subsequent splits, restore_panel_contents() already restored all terminals
                if is_first_split {
                    // Ensure session is registered in split_view
                    if let Some(info) = notebook_for_split_h.get_session_info(current_session) {
                        let terminal = notebook_for_split_h.get_terminal(current_session);
                        split_view.add_session(info, terminal);
                    }
                    // Show in the focused (original) pane
                    let _ = split_view.show_session(current_session);

                    // Use the original pane's color (properly allocated during split)
                    split_view.set_session_color(current_session, original_color_index);
                    notebook_for_split_h.set_tab_split_color(current_session, original_color_index);
                    tracing::debug!(
                        "split-horizontal: applied color {} to tab for session {:?}",
                        original_color_index,
                        current_session
                    );
                }

                // Swap visible split view in container
                // Remove old split view widget if any
                while let Some(child) = split_container_h.first_child() {
                    split_container_h.remove(&child);
                }
                // Add this session's split view
                split_view.widget().set_vexpand(true);
                split_view.widget().set_hexpand(true);
                split_container_h.append(split_view.widget());

                // Make split view visible and hide TabView content
                split_view.widget().set_visible(true);
                split_container_h.set_visible(true);
                notebook_for_split_h.widget().set_vexpand(false);
                notebook_for_split_h.hide_tab_view_content();

                // Also hide global split view (we're using per-session now)
                global_split_view_h.widget().set_visible(false);

                // Setup drop target for the new (empty) pane
                let sv_for_drop = split_view.clone();
                split_view.setup_pane_drop_target_with_callbacks(
                    new_pane_id,
                    move |session_id| {
                        let info = notebook.get_session_info(session_id)?;
                        let terminal = notebook.get_terminal(session_id);
                        Some((info, terminal))
                    },
                    move |session_id, color_index| {
                        // Store session color in split_view for tracking
                        sv_for_drop.set_session_color(session_id, color_index);
                        // Set tab color indicator when session is dropped into pane
                        notebook_for_drop.set_tab_split_color(session_id, color_index);
                    },
                );

                // Setup click handlers for ALL panes (both original and new)
                // This ensures focus rectangle moves correctly when clicking any pane
                let sv_for_focus = sv_for_click.clone();
                let panes_clone = sv_for_click.panes_ref_clone();
                let nb_for_click_clone = nb_for_click.clone();
                let sv_for_terminal = sv_for_click.clone();
                sv_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                    // Update the bridge's focused pane state (handles all focus styling)
                    sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                    // Get session_id from the clicked pane
                    let session_to_switch = {
                        let panes_ref = panes_clone.borrow();
                        panes_ref
                            .iter()
                            .find(|p| p.id() == clicked_pane_uuid)
                            .and_then(|p| p.current_session())
                    };
                    // Switch to the tab if there's a session in this pane
                    if let Some(session_id) = session_to_switch {
                        nb_for_click_clone.switch_to_tab(session_id);
                        // Grab focus on the terminal (click event is claimed, so we must do this)
                        if let Some(terminal) = sv_for_terminal.get_terminal(session_id) {
                            terminal.grab_focus();
                        }
                    }
                });

                // Setup select tab callback for this per-session bridge
                let split_view_for_select = split_view.clone();
                let notebook_for_select = notebook_for_split_h.clone();
                let notebook_for_provider = notebook_for_split_h.clone();
                let notebook_for_terminal = notebook_for_split_h.clone();
                // Clone session_bridges so we can register the new session in the map
                let session_bridges_for_select = session_bridges.clone();
                // Clone for clearing from previous split
                let session_bridges_for_clear = session_bridges.clone();
                // Clone for provider closure
                let split_view_for_provider = split_view.clone();
                split_view.setup_select_tab_callback_with_provider(
                    move || {
                        // Get all sessions from the notebook, excluding those already in THIS split
                        // Only show VTE-based sessions (SSH, ZeroTrust) - RDP/VNC/SPICE not supported
                        notebook_for_provider
                            .get_all_sessions()
                            .into_iter()
                            .filter(|s| {
                                s.protocol == "ssh" || s.protocol.starts_with("zerotrust")
                            })
                            .map(|s| (s.id, s.name))
                            .filter(|(id, _)| !split_view_for_provider.is_session_displayed(*id))
                            .collect()
                    },
                    move |panel_uuid, session_id| {
                        tracing::debug!(
                            "Select Tab callback (horizontal): moving session {} to panel {}",
                            session_id,
                            panel_uuid
                        );

                        // First, clear this session from any previous split view
                        {
                            let bridges = session_bridges_for_clear.borrow();
                            for (other_session_id, other_bridge) in bridges.iter() {
                                // Skip if this is the same bridge we're adding to
                                if Rc::ptr_eq(other_bridge, &split_view_for_select) {
                                    continue;
                                }
                                // Check if this session is displayed in another bridge
                                if other_bridge.is_session_displayed(session_id) {
                                    tracing::debug!(
                                        "Select Tab callback (horizontal): clearing session {} \
                                         from previous split (owner: {})",
                                        session_id,
                                        other_session_id
                                    );
                                    other_bridge.clear_session_from_panes(session_id);
                                    // Clear the old tab color
                                    notebook_for_select.clear_tab_split_color(session_id);
                                    break;
                                }
                            }
                        }

                        // Get terminal from notebook (not from bridge's internal map)
                        let Some(terminal) = notebook_for_terminal.get_terminal(session_id) else {
                            tracing::warn!(
                            "Select Tab callback (horizontal): no terminal found for session {}",
                            session_id
                        );
                            return;
                        };

                        // Move the session to the panel with the terminal
                        // This returns the color index on success
                        match split_view_for_select
                            .move_session_to_panel_with_terminal(panel_uuid, session_id, &terminal)
                        {
                            Ok(color_index) => {
                                // Register this session in session_split_bridges
                                session_bridges_for_select
                                    .borrow_mut()
                                    .insert(session_id, split_view_for_select.clone());

                                // Set tab color indicator using the color from the panel
                                notebook_for_select.set_tab_split_color(session_id, color_index);

                                tracing::debug!(
                                    "Select Tab callback (horizontal): moved session {} to panel {} with color {}",
                                    session_id,
                                    panel_uuid,
                                    color_index
                                );
                            }
                            Err(e) => {
                                tracing::warn!("Failed to move session to panel: {}", e);
                            }
                        }

                        // Note: Do NOT call switch_to_tab() here - the terminal should be
                        // displayed in the split panel, not switched to as the active tab
                    },
                );

                // Setup close panel callback for empty panel close buttons
                let split_view_for_close = split_view.clone();
                split_view.setup_close_panel_callback(move |pane_uuid| {
                    // Focus the pane first so close_pane() closes the correct one
                    split_view_for_close.set_focused_pane(Some(pane_uuid));

                    // Update focus styling via the adapter
                    if let Some(panel_id) = split_view_for_close.get_panel_id_for_uuid(pane_uuid) {
                        if let Err(e) = split_view_for_close.adapter_set_focus(panel_id) {
                            tracing::warn!("Failed to set focus on panel: {}", e);
                        }
                    }
                });
            }
        });
        window.add_action(&split_horizontal_action);

        // Split vertical action
        let split_vertical_action = gio::SimpleAction::new("split-vertical", None);
        let session_bridges_v = self.session_split_bridges.clone();
        let notebook_for_split_v = self.terminal_notebook.clone();
        let split_container_v = self.split_container.clone();
        let global_split_view_v = self.split_view.clone();
        let color_pool_v = self.global_color_pool.clone();
        split_vertical_action.connect_activate(move |_, _| {
            // Get current active session before splitting
            let Some(current_session) = notebook_for_split_v.get_active_session_id() else {
                return; // No active session to split
            };

            tracing::debug!("split-vertical: splitting session {:?}", current_session);

            // Get or create a split bridge for this session (with shared color pool)
            let split_view =
                get_or_create_session_bridge(current_session, &session_bridges_v, &color_pool_v);

            // Check if this is the first split (bridge has only 1 panel)
            // If bridge already has multiple panels, we don't need to show the current session
            // because restore_panel_contents() already restored all terminals
            let is_first_split = split_view.pane_count() == 1;

            // Clone for close callback
            let sv_for_close = split_view.clone();
            if let Some((new_pane_id, new_color_index, original_color_index)) = split_view
                .split_with_close_callback(SplitDirection::Vertical, move || {
                    let _ = sv_for_close.close_pane();
                })
            {
                tracing::debug!(
                    "split-vertical: session {:?} got original_color={}, new_color={}, \
                     is_first_split={}",
                    current_session,
                    original_color_index,
                    new_color_index,
                    is_first_split
                );

                let notebook = notebook_for_split_v.clone();
                let notebook_for_drop = notebook_for_split_v.clone();
                let sv_for_click = split_view.clone();
                let nb_for_click = notebook.clone();

                // Per spec: Split transforms current tab into Container Tab
                // Only show current session in the original pane if this is the FIRST split
                // For subsequent splits, restore_panel_contents() already restored all terminals
                if is_first_split {
                    // Ensure session is registered in split_view
                    if let Some(info) = notebook_for_split_v.get_session_info(current_session) {
                        let terminal = notebook_for_split_v.get_terminal(current_session);
                        split_view.add_session(info, terminal);
                    }
                    // Show in the focused (original) pane
                    let _ = split_view.show_session(current_session);

                    // Use the original pane's color (properly allocated during split)
                    split_view.set_session_color(current_session, original_color_index);
                    notebook_for_split_v.set_tab_split_color(current_session, original_color_index);
                    tracing::debug!(
                        "split-vertical: applied color {} to tab for session {:?}",
                        original_color_index,
                        current_session
                    );
                }

                // Swap visible split view in container
                // Remove old split view widget if any
                while let Some(child) = split_container_v.first_child() {
                    split_container_v.remove(&child);
                }
                // Add this session's split view
                split_view.widget().set_vexpand(true);
                split_view.widget().set_hexpand(true);
                split_container_v.append(split_view.widget());

                // Make split view visible and hide TabView content
                split_view.widget().set_visible(true);
                split_container_v.set_visible(true);
                notebook_for_split_v.widget().set_vexpand(false);
                notebook_for_split_v.hide_tab_view_content();

                // Also hide global split view (we're using per-session now)
                global_split_view_v.widget().set_visible(false);

                // Setup drop target for the new (empty) pane
                let sv_for_drop = split_view.clone();
                split_view.setup_pane_drop_target_with_callbacks(
                    new_pane_id,
                    move |session_id| {
                        let info = notebook.get_session_info(session_id)?;
                        let terminal = notebook.get_terminal(session_id);
                        Some((info, terminal))
                    },
                    move |session_id, color_index| {
                        // Store session color in split_view for tracking
                        sv_for_drop.set_session_color(session_id, color_index);
                        // Set tab color indicator when session is dropped into pane
                        notebook_for_drop.set_tab_split_color(session_id, color_index);
                    },
                );

                // Setup click handlers for ALL panes (both original and new)
                // This ensures focus rectangle moves correctly when clicking any pane
                let sv_for_focus = sv_for_click.clone();
                let panes_clone = sv_for_click.panes_ref_clone();
                let nb_for_click_clone = nb_for_click.clone();
                let sv_for_terminal = sv_for_click.clone();
                sv_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                    // Update the bridge's focused pane state (handles all focus styling)
                    sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                    // Get session_id from the clicked pane
                    let session_to_switch = {
                        let panes_ref = panes_clone.borrow();
                        panes_ref
                            .iter()
                            .find(|p| p.id() == clicked_pane_uuid)
                            .and_then(|p| p.current_session())
                    };
                    // Switch to the tab if there's a session in this pane
                    if let Some(session_id) = session_to_switch {
                        nb_for_click_clone.switch_to_tab(session_id);
                        // Grab focus on the terminal (click event is claimed, so we must do this)
                        if let Some(terminal) = sv_for_terminal.get_terminal(session_id) {
                            terminal.grab_focus();
                        }
                    }
                });

                // Setup select tab callback for this per-session bridge
                let split_view_for_select = split_view.clone();
                let notebook_for_select = notebook_for_split_v.clone();
                let notebook_for_provider = notebook_for_split_v.clone();
                let notebook_for_terminal = notebook_for_split_v.clone();
                // Clone session_bridges so we can register the new session in the map
                let session_bridges_for_select = session_bridges_v.clone();
                // Clone for clearing from previous split
                let session_bridges_for_clear = session_bridges_v.clone();
                // Clone for provider closure
                let split_view_for_provider = split_view.clone();
                split_view.setup_select_tab_callback_with_provider(
                    move || {
                        // Get all sessions from the notebook, excluding those already in THIS split
                        // Only show VTE-based sessions (SSH, ZeroTrust) - RDP/VNC/SPICE not supported
                        notebook_for_provider
                            .get_all_sessions()
                            .into_iter()
                            .filter(|s| {
                                s.protocol == "ssh" || s.protocol.starts_with("zerotrust")
                            })
                            .map(|s| (s.id, s.name))
                            .filter(|(id, _)| !split_view_for_provider.is_session_displayed(*id))
                            .collect()
                    },
                    move |panel_uuid, session_id| {
                        tracing::debug!(
                            "Select Tab callback (vertical): moving session {} to panel {}",
                            session_id,
                            panel_uuid
                        );

                        // First, clear this session from any previous split view
                        {
                            let bridges = session_bridges_for_clear.borrow();
                            for (other_session_id, other_bridge) in bridges.iter() {
                                // Skip if this is the same bridge we're adding to
                                if Rc::ptr_eq(other_bridge, &split_view_for_select) {
                                    continue;
                                }
                                // Check if this session is displayed in another bridge
                                if other_bridge.is_session_displayed(session_id) {
                                    tracing::debug!(
                                        "Select Tab callback (vertical): clearing session {} \
                                         from previous split (owner: {})",
                                        session_id,
                                        other_session_id
                                    );
                                    other_bridge.clear_session_from_panes(session_id);
                                    // Clear the old tab color
                                    notebook_for_select.clear_tab_split_color(session_id);
                                    break;
                                }
                            }
                        }

                        // Get terminal from notebook (not from bridge's internal map)
                        let Some(terminal) = notebook_for_terminal.get_terminal(session_id) else {
                            tracing::warn!(
                                "Select Tab callback (vertical): no terminal found for session {}",
                                session_id
                            );
                            return;
                        };

                        // Move the session to the panel with the terminal
                        // This returns the color index on success
                        match split_view_for_select
                            .move_session_to_panel_with_terminal(panel_uuid, session_id, &terminal)
                        {
                            Ok(color_index) => {
                                // Register this session in session_split_bridges
                                session_bridges_for_select
                                    .borrow_mut()
                                    .insert(session_id, split_view_for_select.clone());

                                // Set tab color indicator using the color from the panel
                                notebook_for_select.set_tab_split_color(session_id, color_index);

                                tracing::debug!(
                                    "Select Tab callback (vertical): moved session {} to panel {} with color {}",
                                    session_id,
                                    panel_uuid,
                                    color_index
                                );
                            }
                            Err(e) => {
                                tracing::warn!("Failed to move session to panel: {}", e);
                            }
                        }

                        // Note: Do NOT call switch_to_tab() here - the terminal should be
                        // displayed in the split panel, not switched to as the active tab
                    },
                );

                // Setup close panel callback for empty panel close buttons
                let split_view_for_close = split_view.clone();
                split_view.setup_close_panel_callback(move |pane_uuid| {
                    // Focus the pane first so close_pane() closes the correct one
                    split_view_for_close.set_focused_pane(Some(pane_uuid));

                    // Update focus styling via the adapter
                    if let Some(panel_id) = split_view_for_close.get_panel_id_for_uuid(pane_uuid) {
                        if let Err(e) = split_view_for_close.adapter_set_focus(panel_id) {
                            tracing::warn!("Failed to set focus on panel: {}", e);
                        }
                    }
                });
            }
        });
        window.add_action(&split_vertical_action);

        // Close pane action
        let close_pane_action = gio::SimpleAction::new("close-pane", None);
        let session_bridges_close = self.session_split_bridges.clone();
        let notebook_for_close = self.terminal_notebook.clone();
        let split_view_for_close = self.split_view.clone();
        let split_container_close = self.split_container.clone();
        close_pane_action.connect_activate(move |_, _| {
            // Find the bridge for the current session and close its focused pane
            if let Some(session_id) = notebook_for_close.get_active_session_id() {
                let bridges = session_bridges_close.borrow();
                if let Some(bridge) = bridges.get(&session_id) {
                    // Get the session in the focused pane before closing
                    let focused_session = bridge.get_focused_session();

                    tracing::debug!(
                        "close-pane: closing focused pane, focused_session={:?}, \
                         pane_count_before={}",
                        focused_session,
                        bridge.pane_count()
                    );

                    match bridge.close_pane() {
                        Ok(should_close_split) => {
                            // Clear tab color for the session that was in the closed pane
                            if let Some(sess_id) = focused_session {
                                notebook_for_close.clear_tab_split_color(sess_id);
                            }

                            // Check if we should close the split view
                            // This happens when: no panels remain, no sessions remain,
                            // or only one panel with one session remains
                            let remaining_sessions: Vec<Uuid> = bridge
                                .pane_ids()
                                .iter()
                                .filter_map(|&pane_id| bridge.get_pane_session(pane_id))
                                .collect();

                            let pane_count = bridge.pane_count();
                            let is_empty = bridge.is_empty();

                            tracing::debug!(
                                "close-pane: after close - should_close_split={}, pane_count={}, \
                                 is_empty={}, remaining_sessions={:?}",
                                should_close_split,
                                pane_count,
                                is_empty,
                                remaining_sessions
                            );

                            let should_unsplit = should_close_split
                                || is_empty
                                || (pane_count == 1 && remaining_sessions.len() == 1);

                            tracing::debug!(
                                "close-pane: should_unsplit={} (should_close_split={} || \
                                 is_empty={} || (pane_count==1 && remaining==1)={})",
                                should_unsplit,
                                should_close_split,
                                is_empty,
                                pane_count == 1 && remaining_sessions.len() == 1
                            );

                            if should_unsplit {
                                // Close split view and show remaining session as regular tab
                                tracing::debug!(
                                    "close-pane: closing split view for session {}, \
                                     remaining_sessions={:?}",
                                    session_id,
                                    remaining_sessions
                                );

                                // Clear tab colors for all remaining sessions
                                for sess_id in &remaining_sessions {
                                    notebook_for_close.clear_tab_split_color(*sess_id);
                                    // Reparent terminal back to TabView
                                    notebook_for_close.reparent_terminal_to_tab(*sess_id);
                                }

                                // Hide split view and show TabView
                                bridge.widget().set_visible(false);
                                split_view_for_close.widget().set_visible(false);
                                split_container_close.set_visible(false);
                                notebook_for_close.widget().set_vexpand(true);
                                notebook_for_close.show_tab_view_content();

                                // Clear tab color for the main session too
                                notebook_for_close.clear_tab_split_color(session_id);
                            } else {
                                // Multiple panels remain - restore terminal content
                                bridge.restore_panel_contents();
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to close pane: {}", e);
                        }
                    }
                }
            }
        });
        window.add_action(&close_pane_action);

        // Focus next pane action
        let focus_next_pane_action = gio::SimpleAction::new("focus-next-pane", None);
        let session_bridges_focus = self.session_split_bridges.clone();
        let notebook_for_focus = self.terminal_notebook.clone();
        focus_next_pane_action.connect_activate(move |_, _| {
            if let Some(session_id) = notebook_for_focus.get_active_session_id() {
                let bridges = session_bridges_focus.borrow();
                if let Some(bridge) = bridges.get(&session_id) {
                    let _ = bridge.focus_next_pane();
                }
            }
        });
        window.add_action(&focus_next_pane_action);

        // Unsplit session action - moves session from split pane to its own tab
        let unsplit_session_action =
            gio::SimpleAction::new("unsplit-session", Some(glib::VariantTy::STRING));
        let session_bridges_unsplit = self.session_split_bridges.clone();
        let notebook_for_unsplit = self.terminal_notebook.clone();
        let split_container_unsplit = self.split_container.clone();
        unsplit_session_action.connect_activate(move |_, param| {
            if let Some(param) = param {
                if let Some(session_id_str) = param.get::<String>() {
                    if let Ok(session_id) = Uuid::parse_str(&session_id_str) {
                        // Find the bridge containing this session
                        let bridges = session_bridges_unsplit.borrow();
                        for bridge in bridges.values() {
                            if bridge.is_session_displayed(session_id) {
                                // Clear session from split pane
                                bridge.clear_session_from_panes(session_id);

                                // Move terminal back to TabView
                                notebook_for_unsplit.reparent_terminal_to_tab(session_id);

                                // Clear tab color indicator
                                notebook_for_unsplit.clear_tab_split_color(session_id);

                                // Check if any sessions remain in this split view
                                let has_sessions_in_split = bridge
                                    .pane_ids()
                                    .iter()
                                    .any(|&pane_id| bridge.get_pane_session(pane_id).is_some());

                                if !has_sessions_in_split {
                                    // No sessions in split view - hide it and show TabView
                                    bridge.widget().set_visible(false);
                                    split_container_unsplit.set_visible(false);
                                    notebook_for_unsplit.widget().set_vexpand(true);
                                    notebook_for_unsplit.show_tab_view_content();
                                }
                                break;
                            }
                        }
                    }
                }
            }
        });
        window.add_action(&unsplit_session_action);
    }

    /// Sets up document management actions
    fn setup_document_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // adw::ApplicationWindow extends gtk4::ApplicationWindow, so we can use upcast_ref
        let gtk_app_window: &gtk4::ApplicationWindow = window.upcast_ref();
        doc_actions::setup_document_actions(gtk_app_window, state, sidebar);
    }

    /// Sets up miscellaneous actions (drag-drop)
    fn setup_misc_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        _terminal_notebook: &SharedNotebook,
    ) {
        // Drag-drop item action for reordering connections
        let drag_drop_action =
            gio::SimpleAction::new("drag-drop-item", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        drag_drop_action.connect_activate(move |_, param| {
            if let Some(data) = param.and_then(gtk4::glib::Variant::get::<String>) {
                Self::handle_drag_drop(&state_clone, &sidebar_clone, &data);
            }
        });
        window.add_action(&drag_drop_action);

        // Hide drop indicator action - called when drag ends or drop completes
        let hide_drop_indicator_action = gio::SimpleAction::new("hide-drop-indicator", None);
        let sidebar_clone = sidebar.clone();
        hide_drop_indicator_action.connect_activate(move |_, _| {
            sidebar_clone.hide_drop_indicator();
        });
        window.add_action(&hide_drop_indicator_action);
    }

    /// Connects UI signals
    #[allow(clippy::too_many_lines)]
    fn connect_signals(&self) {
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();
        let split_view = self.split_view.clone();
        let paned = self.paned.clone();
        let window = self.window.clone();

        // Set up "Select Tab" callback for empty panel placeholders
        // This provides an alternative to drag-and-drop for moving sessions to split panels
        {
            let split_view_for_select = split_view.clone();
            let notebook_for_select = terminal_notebook.clone();
            let notebook_for_provider = terminal_notebook.clone();
            let notebook_for_terminal = terminal_notebook.clone();
            split_view.setup_select_tab_callback_with_provider(
                move || {
                    // Get all sessions from the notebook
                    // Only show VTE-based sessions (SSH, ZeroTrust) - RDP/VNC/SPICE not supported
                    notebook_for_provider
                        .get_all_sessions()
                        .into_iter()
                        .filter(|s| s.protocol == "ssh" || s.protocol.starts_with("zerotrust"))
                        .map(|s| (s.id, s.name))
                        .collect()
                },
                move |panel_uuid, session_id| {
                    tracing::debug!(
                        "Select Tab callback: moving session {} to panel {}",
                        session_id,
                        panel_uuid
                    );

                    // Get terminal from notebook (not from bridge's internal map)
                    let Some(terminal) = notebook_for_terminal.get_terminal(session_id) else {
                        tracing::warn!(
                            "Select Tab callback (global): no terminal found for session {}",
                            session_id
                        );
                        return;
                    };

                    // Move the session to the panel with the terminal
                    if let Err(e) = split_view_for_select
                        .move_session_to_panel_with_terminal(panel_uuid, session_id, &terminal)
                    {
                        tracing::warn!("Failed to move session to panel: {}", e);
                        return;
                    }

                    // Get color for this pane using the new method
                    let color_index = split_view_for_select.get_pane_color(panel_uuid);

                    tracing::debug!(
                        "Select Tab callback (global): panel {} has color {:?}",
                        panel_uuid,
                        color_index
                    );

                    // Set tab color indicator
                    if let Some(color) = color_index {
                        notebook_for_select.set_tab_split_color(session_id, color);
                        split_view_for_select.set_session_color(session_id, color);
                        tracing::debug!(
                            "Select Tab callback (global): applied color {} to session {}",
                            color,
                            session_id
                        );
                    } else {
                        tracing::warn!(
                            "Select Tab callback (global): no color found for panel {}",
                            panel_uuid
                        );
                    }

                    // Note: Do NOT call switch_to_tab() here - the terminal should be
                    // displayed in the split panel, not switched to as the active tab
                },
            );

            // Setup close panel callback for empty panel close buttons
            let split_view_for_close = split_view.clone();
            split_view.setup_close_panel_callback(move |pane_uuid| {
                // Focus the pane first so close_pane() closes the correct one
                split_view_for_close.set_focused_pane(Some(pane_uuid));

                // Update focus styling via the adapter
                if let Some(panel_id) = split_view_for_close.get_panel_id_for_uuid(pane_uuid) {
                    if let Err(e) = split_view_for_close.adapter_set_focus(panel_id) {
                        tracing::warn!("Failed to set focus on panel: {}", e);
                    }
                }
            });
        }

        // Set up drag-and-drop for initial pane with notebook lookup
        if let Some(initial_pane_id) = split_view.pane_ids().first().copied() {
            let notebook_for_drop = terminal_notebook.clone();
            let notebook_for_color = terminal_notebook.clone();
            split_view.setup_pane_drop_target_with_callbacks(
                initial_pane_id,
                move |session_id| {
                    let info = notebook_for_drop.get_session_info(session_id)?;
                    let terminal = notebook_for_drop.get_terminal(session_id);
                    Some((info, terminal))
                },
                move |session_id, color_index| {
                    // Set tab color indicator when session is dropped into pane
                    notebook_for_color.set_tab_split_color(session_id, color_index);
                },
            );
        }

        // Set up click handlers for focus management on global split view
        // Note: This is for the global split view; per-session bridges set up their own handlers
        {
            let split_view_for_click = split_view.clone();
            let notebook_for_click = terminal_notebook.clone();
            let sv_for_focus = split_view_for_click.clone();
            let panes_clone = split_view_for_click.panes_ref_clone();
            let notebook_clone = notebook_for_click.clone();
            let sv_for_terminal = split_view_for_click.clone();

            split_view_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                // Update the bridge's focused pane state (handles all focus styling)
                sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                // Get session_id from the clicked pane
                let session_to_switch = {
                    let panes_ref = panes_clone.borrow();
                    panes_ref
                        .iter()
                        .find(|p| p.id() == clicked_pane_uuid)
                        .and_then(|p| p.current_session())
                };
                // Switch to the tab if there's a session in this pane
                if let Some(session_id) = session_to_switch {
                    notebook_clone.switch_to_tab(session_id);
                    // Grab focus on the terminal (click event is claimed, so we must do this)
                    if let Some(terminal) = sv_for_terminal.get_terminal(session_id) {
                        terminal.grab_focus();
                    }
                }
            });
        }

        // Connect sidebar search with debouncing
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sidebar.search_entry().connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            // Save pre-search state on first keystroke
            if !query.is_empty() {
                sidebar_clone.save_pre_search_state();
            }

            // Check if we should debounce
            let debouncer = sidebar_clone.search_debouncer();
            if debouncer.should_proceed() {
                // Immediate search - hide spinner and filter
                sidebar_clone.hide_search_pending();
                Self::filter_connections(&state_clone, &sidebar_clone, &query);

                // Restore state if search cleared
                if query.is_empty() {
                    sidebar_clone.restore_pre_search_state();
                }
            } else {
                // Debounced - show spinner and schedule search
                sidebar_clone.show_search_pending();
                sidebar_clone.set_pending_search_query(Some(query.clone()));

                // Schedule delayed search using glib timeout
                let state_for_timeout = state_clone.clone();
                let sidebar_for_timeout = sidebar_clone.clone();
                let delay_ms = debouncer.delay().as_millis() as u32;

                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(u64::from(delay_ms)),
                    move || {
                        // Only proceed if this is still the pending query
                        if let Some(pending) = sidebar_for_timeout.pending_search_query() {
                            if pending == query {
                                sidebar_for_timeout.hide_search_pending();
                                sidebar_for_timeout.set_pending_search_query(None);
                                Self::filter_connections(
                                    &state_for_timeout,
                                    &sidebar_for_timeout,
                                    &pending,
                                );

                                // Restore state if search cleared
                                if pending.is_empty() {
                                    sidebar_for_timeout.restore_pre_search_state();
                                }
                            }
                        }
                    },
                );
            }
        });

        // Add to search history when user presses Enter or stops searching
        let sidebar_for_history = sidebar.clone();
        let state_for_history = state.clone();
        sidebar.search_entry().connect_activate(move |entry| {
            let query = entry.text().to_string();
            if !query.is_empty() {
                sidebar_for_history.add_to_search_history(&query);
                // Persist to settings
                if let Ok(mut state_mut) = state_for_history.try_borrow_mut() {
                    state_mut.settings_mut().ui.add_search_history(&query);
                    let _ = state_mut.save_settings();
                }
            }
        });

        // Also add to history when search entry loses focus with non-empty query
        let sidebar_for_focus = sidebar.clone();
        let state_for_focus = state.clone();
        sidebar
            .search_entry()
            .connect_has_focus_notify(move |entry| {
                if !entry.has_focus() {
                    let query = entry.text().to_string();
                    if !query.is_empty() {
                        sidebar_for_focus.add_to_search_history(&query);
                        // Persist to settings
                        if let Ok(mut state_mut) = state_for_focus.try_borrow_mut() {
                            state_mut.settings_mut().ui.add_search_history(&query);
                            let _ = state_mut.save_settings();
                        }
                    }
                }
            });

        // Connect sidebar double-click to connect
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = split_view.clone();
        sidebar.list_view().connect_activate(move |_, position| {
            Self::connect_at_position_with_split(
                &state_clone,
                &sidebar_clone,
                &notebook_clone,
                &split_view_clone,
                position,
            );
        });

        // Connect TabView page selection - per spec, do NOT auto-fill split panes
        // Split view is shown ONLY when selected session is displayed in a split pane
        // Requirement 3: Each tab maintains its own independent split layout
        let session_bridges_for_tab = self.session_split_bridges.clone();
        let split_container_for_tab = self.split_container.clone();
        let global_split_view = split_view.clone();
        let notebook_clone = terminal_notebook.clone();
        terminal_notebook.tab_view().connect_notify_local(
            Some("selected-page"),
            move |tab_view, _| {
                let Some(selected_page) = tab_view.selected_page() else {
                    return;
                };
                let page_num = tab_view.page_position(&selected_page) as u32;

                // Get session ID for this page
                if let Some(session_id) = notebook_clone.get_session_id_for_page(page_num) {
                    // Check if this is a VNC, RDP, or SPICE session - they display differently
                    if let Some(info) = notebook_clone.get_session_info(session_id) {
                        if info.protocol == "vnc"
                            || info.protocol == "rdp"
                            || info.protocol == "spice"
                        {
                            // For VNC/RDP/SPICE: hide split view, show TabView content
                            global_split_view.widget().set_visible(false);
                            split_container_for_tab.set_visible(false);
                            notebook_clone.widget().set_vexpand(true);
                            notebook_clone.show_tab_view_content();
                            return;
                        }
                    }

                    // For SSH: check if this session has its own split bridge
                    let bridges = session_bridges_for_tab.borrow();
                    if let Some(bridge) = bridges.get(&session_id) {
                        // Session has its own split bridge - show it
                        // Swap visible split view in container
                        while let Some(child) = split_container_for_tab.first_child() {
                            split_container_for_tab.remove(&child);
                        }
                        bridge.widget().set_vexpand(true);
                        bridge.widget().set_hexpand(true);
                        split_container_for_tab.append(bridge.widget());

                        // Show split container, hide TabView content
                        bridge.widget().set_visible(true);
                        split_container_for_tab.set_visible(true);
                        global_split_view.widget().set_visible(false);
                        notebook_clone.widget().set_vexpand(false);
                        notebook_clone.hide_tab_view_content();

                        // Reparent terminal to split pane if needed
                        let has_split_color = bridge.get_session_color(session_id).is_some();
                        let is_displayed = bridge.is_session_displayed(session_id);
                        if has_split_color && !is_displayed {
                            let _ = bridge.reparent_terminal_to_split(session_id);
                        }

                        // Update tab colors for ALL sessions in this split view
                        // Each tab should have the color of its pane
                        for pane_id in bridge.pane_ids() {
                            if let Some(pane_session_id) = bridge.get_pane_session(pane_id) {
                                if let Some(pane_color) = bridge.get_pane_color(pane_id) {
                                    notebook_clone.set_tab_split_color(pane_session_id, pane_color);
                                    bridge.set_session_color(pane_session_id, pane_color);
                                }
                            }
                        }

                        // Focus the pane containing the selected session
                        for pane_id in bridge.pane_ids() {
                            if bridge.get_pane_session(pane_id) == Some(session_id) {
                                let _ = bridge.focus_pane(pane_id);
                                if let Some(terminal) = bridge.get_terminal(session_id) {
                                    terminal.grab_focus();
                                }
                                break;
                            }
                        }
                    } else {
                        // Session NOT in any split view - show in TabView directly
                        // Move terminal back to TabView page if it was in split view
                        notebook_clone.reparent_terminal_to_tab(session_id);

                        // Clear any split color indicator
                        notebook_clone.clear_tab_split_color(session_id);

                        // Hide split views, show TabView content
                        global_split_view.widget().set_visible(false);
                        split_container_for_tab.set_visible(false);
                        notebook_clone.widget().set_vexpand(true);
                        notebook_clone.show_tab_view_content();
                    }
                } else {
                    // Welcome tab (page 0) - show TabView welcome
                    global_split_view.widget().set_visible(false);
                    split_container_for_tab.set_visible(false);
                    notebook_clone.widget().set_vexpand(true);
                    notebook_clone.show_tab_view_content();
                }
            },
        );

        // Save window state on close and handle minimize to tray
        let state_clone = state.clone();
        let paned_clone = paned;
        let sidebar_clone = sidebar.clone();
        window.connect_close_request(move |win| {
            // Save window geometry and expanded groups state
            let (width, height) = win.default_size();
            let sidebar_width = paned_clone.position();

            // Save expanded groups state
            let expanded = sidebar_clone.get_expanded_groups();

            if let Ok(mut state) = state_clone.try_borrow_mut() {
                // Update expanded groups
                let _ = state.update_expanded_groups(expanded);

                let mut settings = state.settings().clone();
                if settings.ui.remember_window_geometry {
                    settings.ui.window_width = Some(width);
                    settings.ui.window_height = Some(height);
                    settings.ui.sidebar_width = Some(sidebar_width);
                    let _ = state.update_settings(settings.clone());
                }

                // Check if we should minimize to tray instead of closing
                if settings.ui.minimize_to_tray && settings.ui.enable_tray_icon {
                    // Hide the window instead of closing
                    win.set_visible(false);
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });
    }

    /// Loads connections into the sidebar
    fn load_connections(&self) {
        let expanded_groups = self.state.borrow().expanded_groups().clone();

        // Use sorted rebuild to ensure alphabetical order by default
        crate::window_sorting::rebuild_sidebar_sorted(&self.state, &self.sidebar);

        // Apply expanded state after populating
        self.sidebar.apply_expanded_groups(&expanded_groups);
    }

    /// Updates the password vault button status in the sidebar based on current settings
    fn update_keepass_button_status(&self) {
        let state_ref = self.state.borrow();
        let settings = state_ref.settings();
        let backend = settings.secrets.preferred_backend;

        // For libsecret and Bitwarden, always enabled (no database file needed)
        // For KeePassXC/KdbxFile, check if enabled and database exists
        let (enabled, database_exists) = match backend {
            rustconn_core::config::SecretBackendType::LibSecret
            | rustconn_core::config::SecretBackendType::Bitwarden => (true, true),
            rustconn_core::config::SecretBackendType::KeePassXc
            | rustconn_core::config::SecretBackendType::KdbxFile => {
                let kdbx_enabled = settings.secrets.kdbx_enabled;
                let db_exists = settings
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p| p.exists());
                (kdbx_enabled, db_exists)
            }
        };
        drop(state_ref);

        self.sidebar.update_keepass_status(enabled, database_exists);
    }

    /// Public method to refresh KeePass button status (called after settings change)
    #[allow(dead_code)] // Part of KeePass integration API, called from settings dialog
    pub fn refresh_keepass_status(&self) {
        self.update_keepass_button_status();
    }

    /// Builds context menu for a split view pane
    ///
    /// Note: This is currently unused as the adapter handles context menus via
    /// `setup_panel_context_menu`. Kept for potential future use.
    #[allow(dead_code)]
    fn build_pane_context_menu(
        pane_id: Uuid,
        panes: &Rc<RefCell<Vec<crate::split_view::TerminalPane>>>,
        sessions: &crate::split_view::SharedSessions,
    ) -> gio::Menu {
        let menu = gio::Menu::new();

        // Get session in this pane
        let panes_ref = panes.borrow();
        let session_id = panes_ref
            .iter()
            .find(|p| p.id() == pane_id)
            .and_then(|p| p.current_session());
        let pane_count = panes_ref.len();
        drop(panes_ref);

        // Clipboard actions section (always available)
        // Note: No keyboard shortcuts shown - they don't work in VTE context menu
        let clipboard_section = gio::Menu::new();
        clipboard_section.append(Some("Copy"), Some("win.copy"));
        clipboard_section.append(Some("Paste"), Some("win.paste"));
        menu.append_section(None, &clipboard_section);

        if let Some(sid) = session_id {
            // Get session name for display
            let sessions_ref = sessions.borrow();
            let session_name = sessions_ref
                .get(&sid)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Session".to_string());
            drop(sessions_ref);

            // Session actions section
            let session_section = gio::Menu::new();

            // Close session - use MenuItem with target for proper parameter passing
            let close_item = gio::MenuItem::new(Some(&format!("Close \"{}\"", session_name)), None);
            close_item.set_action_and_target_value(
                Some("win.close-tab-by-id"),
                Some(&sid.to_string().to_variant()),
            );
            session_section.append_item(&close_item);

            // Move to separate tab (unsplit) - only if in split view
            if pane_count > 1 {
                let unsplit_item = gio::MenuItem::new(Some("Move to Separate Tab"), None);
                unsplit_item.set_action_and_target_value(
                    Some("win.unsplit-session"),
                    Some(&sid.to_string().to_variant()),
                );
                session_section.append_item(&unsplit_item);
            }

            menu.append_section(None, &session_section);
        }

        // Pane actions section (only if multiple panes)
        if pane_count > 1 {
            let pane_section = gio::Menu::new();
            pane_section.append(Some("Close Pane"), Some("win.close-pane"));
            menu.append_section(None, &pane_section);
        }

        // Split actions section
        let split_section = gio::Menu::new();
        split_section.append(Some("Split Horizontal"), Some("win.split-horizontal"));
        split_section.append(Some("Split Vertical"), Some("win.split-vertical"));
        menu.append_section(None, &split_section);

        menu
    }

    /// Filters connections based on search query
    fn filter_connections(state: &SharedAppState, sidebar: &SharedSidebar, query: &str) {
        use rustconn_core::search::SearchEngine;

        if query.is_empty() {
            // Restore full hierarchy when search is cleared
            Self::reload_sidebar(state, sidebar);
            // Restore the tree state that was saved before search started
            sidebar.restore_pre_search_state();
            return;
        }

        // Save tree state before first search keystroke
        sidebar.save_pre_search_state();

        let store = sidebar.store();
        store.remove_all();

        let state_ref = state.borrow();

        // Get connections and groups for search
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        let groups: Vec<_> = state_ref.list_groups().iter().cloned().cloned().collect();

        // Check for special multiple protocol filter syntax
        if let Some(protocols_str) = query.strip_prefix("protocols:") {
            // Handle multiple protocol filters with OR logic
            let protocol_names: Vec<&str> = protocols_str.split(',').collect();
            let mut filtered_connections = Vec::new();

            for conn in &connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                let protocol_lower = protocol.to_lowercase();

                // Check if connection matches any of the selected protocols
                if protocol_names
                    .iter()
                    .any(|p| p.to_lowercase() == protocol_lower)
                {
                    filtered_connections.push(conn);
                }
            }

            // Display filtered connections
            for conn in filtered_connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                let item = ConnectionItem::new_connection(
                    &conn.id.to_string(),
                    &conn.name,
                    &protocol,
                    &conn.host,
                );
                store.append(&item);
            }
        } else {
            // Use standard search engine for other queries
            let search_engine = SearchEngine::new();
            let parsed_query = match SearchEngine::parse_query(query) {
                Ok(q) => q,
                Err(_) => {
                    // Fall back to simple text search on parse error
                    rustconn_core::search::SearchQuery::with_text(query)
                }
            };

            // Perform search with ranking
            let results = search_engine.search(&parsed_query, &connections, &groups);

            // Display results sorted by relevance
            for result in results {
                if let Some(conn) = connections.iter().find(|c| c.id == result.connection_id) {
                    let protocol = get_protocol_string(&conn.protocol_config);

                    // Create display name with relevance indicator
                    let display_name = if result.score >= 0.9 {
                        format!("★★★ {}", conn.name) // High relevance
                    } else if result.score >= 0.7 {
                        format!("★★ {}", conn.name) // Medium relevance
                    } else if result.score >= 0.5 {
                        format!("★ {}", conn.name) // Low relevance
                    } else {
                        conn.name.clone() // Very low relevance
                    };

                    let item = ConnectionItem::new_connection(
                        &conn.id.to_string(),
                        &display_name,
                        &protocol,
                        &conn.host,
                    );
                    store.append(&item);
                }
            }
        }
    }

    /// Connects to the selected connection
    fn connect_selected(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
    ) {
        // Get selected item from sidebar using the sidebar's method
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Only connect if it's not a group
        if conn_item.is_group() {
            return;
        }

        let id_str = conn_item.id();
        if let Ok(conn_id) = Uuid::parse_str(&id_str) {
            Self::start_connection(state, notebook, sidebar, conn_id);
        }
    }

    /// Connects to a connection at a specific position with split view support
    fn connect_at_position_with_split(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        position: u32,
    ) {
        // Get the item at position from the tree model (not the flat store)
        let tree_model = sidebar.tree_model();
        if let Some(item) = tree_model.item(position) {
            // TreeListModel returns TreeListRow, need to get the actual item
            if let Some(row) = item.downcast_ref::<gtk4::TreeListRow>() {
                if let Some(conn_item) =
                    row.item().and_then(|i| i.downcast::<ConnectionItem>().ok())
                {
                    if !conn_item.is_group() {
                        let id_str = conn_item.id();
                        if let Ok(conn_id) = Uuid::parse_str(&id_str) {
                            Self::start_connection_with_credential_resolution(
                                state.clone(),
                                notebook.clone(),
                                split_view.clone(),
                                sidebar.clone(),
                                conn_id,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Starts a connection with credential resolution
    ///
    /// This method implements the credential resolution flow:
    /// 1. Check the connection's `password_source` setting
    /// 2. Try to resolve credentials from configured backends (`KeePass`, Keyring)
    /// 3. Fall back to cached credentials if available
    /// 4. Prompt user if no credentials found and required
    ///
    /// Uses async credential resolution to avoid blocking the GTK main thread.
    fn start_connection_with_credential_resolution(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
    ) {
        // Get connection info and cached credentials (fast, non-blocking)
        let (protocol_type, cached_credentials) = {
            let Ok(state_ref) = state.try_borrow() else {
                tracing::warn!("Could not borrow state for credential resolution");
                return;
            };

            let conn = match state_ref.get_connection(connection_id) {
                Some(c) => c,
                None => return,
            };

            let protocol_type = conn.protocol_config.protocol_type();

            let cached = state_ref.get_cached_credentials(connection_id).map(|c| {
                use secrecy::ExposeSecret;
                (
                    c.username.clone(),
                    c.password.expose_secret().to_string(),
                    c.domain.clone(),
                )
            });

            (protocol_type, cached)
        };

        // If we have cached credentials, use them immediately (no async needed)
        if let Some((username, password, domain)) = cached_credentials {
            Self::handle_resolved_credentials(
                state,
                notebook,
                split_view,
                sidebar,
                connection_id,
                protocol_type,
                Some(rustconn_core::Credentials::with_password(
                    &username, &password,
                )),
                Some((username, password, domain)),
            );
            return;
        }

        // Resolve credentials asynchronously to avoid blocking GTK main thread
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let split_view_clone = split_view.clone();
        let sidebar_clone = sidebar.clone();

        {
            let Ok(state_ref) = state.try_borrow() else {
                tracing::warn!("Could not borrow state for async credential resolution");
                return;
            };

            state_ref.resolve_credentials_gtk(connection_id, move |result| {
                let resolved_credentials = match result {
                    Ok(creds) => creds,
                    Err(e) => {
                        tracing::warn!("Failed to resolve credentials: {e}");
                        None
                    }
                };

                Self::handle_resolved_credentials(
                    state_clone,
                    notebook_clone,
                    split_view_clone,
                    sidebar_clone,
                    connection_id,
                    protocol_type,
                    resolved_credentials,
                    None, // No cached credentials
                );
            });
        }
    }

    /// Handles resolved credentials and starts the appropriate connection
    ///
    /// This is called either immediately (if cached credentials exist) or
    /// from the async callback (after credential resolution completes).
    #[allow(clippy::too_many_arguments)]
    fn handle_resolved_credentials(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        protocol_type: rustconn_core::ProtocolType,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, String, String)>,
    ) {
        use rustconn_core::ProtocolType;

        match protocol_type {
            ProtocolType::Rdp => {
                Self::handle_rdp_credentials(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    connection_id,
                    resolved_credentials,
                    cached_credentials,
                );
            }
            ProtocolType::Vnc => {
                Self::handle_vnc_credentials(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    connection_id,
                    resolved_credentials,
                    cached_credentials,
                );
            }
            ProtocolType::Ssh | ProtocolType::Spice | ProtocolType::ZeroTrust => {
                // For SSH/SPICE, cache credentials if available and start connection
                if let Some(ref creds) = resolved_credentials {
                    if let (Some(username), Some(password)) =
                        (&creds.username, creds.expose_password())
                    {
                        if let Ok(mut state_mut) = state.try_borrow_mut() {
                            state_mut.cache_credentials(connection_id, username, password, "");
                        }
                    }
                }
                Self::start_connection_with_split(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar,
                    connection_id,
                );
            }
        }
    }

    /// Handles RDP credential resolution and connection start
    #[allow(clippy::too_many_arguments)]
    fn handle_rdp_credentials(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, String, String)>,
    ) {
        // Check if port check is needed BEFORE prompting for credentials
        let (should_check, host, port, timeout) = {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let conn = state_ref.get_connection(connection_id);
            if let Some(conn) = conn {
                let should = settings.connection.pre_connect_port_check && !conn.skip_port_check;
                (
                    should,
                    conn.host.clone(),
                    conn.port,
                    settings.connection.port_check_timeout_secs,
                )
            } else {
                return;
            }
        };

        if should_check {
            // Run port check in background thread BEFORE showing password dialog
            let state_clone = state.clone();
            let notebook_clone = notebook.clone();
            let split_view_clone = split_view.clone();
            let sidebar_clone = sidebar.clone();

            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::check_port(&host, port, timeout),
                move |result| {
                    match result {
                        Ok(_) => {
                            // Port is open, proceed with credential handling
                            Self::handle_rdp_credentials_internal(
                                state_clone,
                                notebook_clone,
                                split_view_clone,
                                sidebar_clone,
                                connection_id,
                                resolved_credentials,
                                cached_credentials,
                            );
                        }
                        Err(e) => {
                            // Port check failed, show error and update sidebar
                            tracing::warn!("Port check failed for RDP connection: {e}");
                            sidebar_clone
                                .update_connection_status(&connection_id.to_string(), "failed");
                            if let Some(root) = notebook_clone.widget().root() {
                                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                                    crate::alert::show_error(
                                        window,
                                        "Connection Failed",
                                        &format!(
                                            "{e}\n\n\
                                            The host may be offline or the port may be blocked."
                                        ),
                                    );
                                }
                            }
                        }
                    }
                },
            );
        } else {
            // Port check disabled, proceed directly
            Self::handle_rdp_credentials_internal(
                state,
                notebook,
                split_view,
                sidebar,
                connection_id,
                resolved_credentials,
                cached_credentials,
            );
        }
    }

    /// Internal RDP credential handling (after port check)
    #[allow(clippy::too_many_arguments)]
    fn handle_rdp_credentials_internal(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, String, String)>,
    ) {
        // Use resolved credentials if available
        if let Some(ref creds) = resolved_credentials {
            if let (Some(username), Some(password)) = (&creds.username, creds.expose_password()) {
                Self::start_rdp_session_with_credentials(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar,
                    connection_id,
                    username,
                    password,
                    "",
                );
                return;
            }
        }

        // Use cached credentials if available
        if let Some((username, password, domain)) = cached_credentials {
            Self::start_rdp_session_with_credentials(
                &state,
                &notebook,
                &split_view,
                &sidebar,
                connection_id,
                &username,
                &password,
                &domain,
            );
            return;
        }

        // Need to prompt for credentials
        if let Some(window) = notebook
            .widget()
            .ancestor(adw::ApplicationWindow::static_type())
        {
            if let Some(app_window) = window.downcast_ref::<adw::ApplicationWindow>() {
                Self::start_rdp_with_password_dialog(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    connection_id,
                    app_window,
                );
            }
        }
    }

    /// Handles VNC credential resolution and connection start
    #[allow(clippy::too_many_arguments)]
    fn handle_vnc_credentials(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, String, String)>,
    ) {
        // Check if port check is needed BEFORE prompting for credentials
        let (should_check, host, port, timeout) = {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let conn = state_ref.get_connection(connection_id);
            if let Some(conn) = conn {
                let should = settings.connection.pre_connect_port_check && !conn.skip_port_check;
                (
                    should,
                    conn.host.clone(),
                    conn.port,
                    settings.connection.port_check_timeout_secs,
                )
            } else {
                return;
            }
        };

        if should_check {
            // Run port check in background thread BEFORE showing password dialog
            let state_clone = state.clone();
            let notebook_clone = notebook.clone();
            let split_view_clone = split_view.clone();
            let sidebar_clone = sidebar.clone();

            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::check_port(&host, port, timeout),
                move |result| {
                    match result {
                        Ok(_) => {
                            // Port is open, proceed with credential handling
                            Self::handle_vnc_credentials_internal(
                                state_clone,
                                notebook_clone,
                                split_view_clone,
                                sidebar_clone,
                                connection_id,
                                resolved_credentials,
                                cached_credentials,
                            );
                        }
                        Err(e) => {
                            // Port check failed, show error and update sidebar
                            tracing::warn!("Port check failed for VNC connection: {e}");
                            sidebar_clone
                                .update_connection_status(&connection_id.to_string(), "failed");
                            if let Some(root) = notebook_clone.widget().root() {
                                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                                    crate::alert::show_error(
                                        window,
                                        "Connection Failed",
                                        &format!(
                                            "{e}\n\n\
                                            The host may be offline or the port may be blocked."
                                        ),
                                    );
                                }
                            }
                        }
                    }
                },
            );
        } else {
            // Port check disabled, proceed directly
            Self::handle_vnc_credentials_internal(
                state,
                notebook,
                split_view,
                sidebar,
                connection_id,
                resolved_credentials,
                cached_credentials,
            );
        }
    }

    /// Internal VNC credential handling (after port check)
    #[allow(clippy::too_many_arguments)]
    fn handle_vnc_credentials_internal(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        resolved_credentials: Option<rustconn_core::Credentials>,
        cached_credentials: Option<(String, String, String)>,
    ) {
        // Use resolved credentials if available (VNC only needs password)
        if let Some(ref creds) = resolved_credentials {
            if let Some(password) = creds.expose_password() {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(connection_id, "", password, "");
                }
                Self::start_connection_with_split(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar,
                    connection_id,
                );
                return;
            }
        }

        // Use cached credentials if available
        if cached_credentials.is_some() {
            Self::start_connection_with_split(
                &state,
                &notebook,
                &split_view,
                &sidebar,
                connection_id,
            );
            return;
        }

        // Need to prompt for VNC password
        if let Some(window) = notebook
            .widget()
            .ancestor(adw::ApplicationWindow::static_type())
        {
            if let Some(app_window) = window.downcast_ref::<adw::ApplicationWindow>() {
                Self::start_vnc_with_password_dialog(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    connection_id,
                    app_window,
                );
            }
        }
    }

    /// Starts an RDP connection with password dialog
    fn start_rdp_with_password_dialog(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        window: &adw::ApplicationWindow,
    ) {
        rdp_vnc::start_rdp_with_password_dialog(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            window.upcast_ref(),
        );
    }

    /// Starts RDP session with provided credentials
    #[allow(clippy::too_many_arguments)]
    fn start_rdp_session_with_credentials(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
        username: &str,
        password: &str,
        domain: &str,
    ) {
        rdp_vnc::start_rdp_session_with_credentials(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            username,
            password,
            domain,
        );
    }

    /// Starts a VNC connection with password dialog
    fn start_vnc_with_password_dialog(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        window: &adw::ApplicationWindow,
    ) {
        rdp_vnc::start_vnc_with_password_dialog(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            window.upcast_ref(),
        );
    }

    /// Starts a connection with split view integration
    pub fn start_connection_with_split(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
    ) -> Option<Uuid> {
        // Update status to connecting
        sidebar.update_connection_status(&connection_id.to_string(), "connecting");

        let session_id = Self::start_connection(state, notebook, sidebar, connection_id)?;

        // Get session info to check protocol
        if let Some(info) = notebook.get_session_info(session_id) {
            // VNC, RDP, and SPICE sessions are displayed directly in notebook tab
            if info.protocol == "vnc" || info.protocol == "rdp" || info.protocol == "spice" {
                // Hide split view and expand notebook for VNC/RDP/SPICE
                split_view.widget().set_visible(false);
                split_view.widget().set_vexpand(false);
                notebook.widget().set_vexpand(true);
                notebook.show_tab_view_content();
                return Some(session_id);
            }

            // For SSH: register session info for potential drag-and-drop
            // Per spec: new connections ALWAYS open in a new tab, never in split pane
            // Don't pass terminal - it stays in TabView page
            split_view.add_session(info.clone(), None);

            // Per spec: new connections always show in TabView (as a new tab)
            // Hide split view, show TabView content
            split_view.widget().set_visible(false);
            split_view.widget().set_vexpand(false);
            notebook.widget().set_vexpand(true);
            notebook.show_tab_view_content();

            // For SSH and Zero Trust, we assume connected for now
            if info.protocol == "ssh" || info.protocol.starts_with("zerotrust") {
                // Set status to connecting initially (only if not already connected)
                if sidebar
                    .get_connection_status(&connection_id.to_string())
                    .is_none()
                {
                    sidebar.update_connection_status(&connection_id.to_string(), "connecting");
                }

                // Monitor terminal content changes to detect successful connection
                // If content changes (e.g. prompt appears), mark as connected
                let sidebar_clone = sidebar.clone();
                let notebook_clone = notebook.clone();
                let connection_id_str = connection_id.to_string();

                // Track whether this specific session has been marked as connected
                let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
                let session_connected_clone = session_connected.clone();

                // Clone protocol for use in closure
                let protocol_for_closure = info.protocol.clone();

                notebook.connect_contents_changed(session_id, move || {
                    // Only increment once per session
                    if !session_connected_clone.get() {
                        // Check if content indicates actual output from the process
                        // For SSH: initial header is 2 lines, wait for more output
                        // For Zero Trust (AWS SSM etc.): any output indicates success
                        if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) {
                            let threshold = if protocol_for_closure.starts_with("zerotrust") {
                                0
                            } else {
                                2
                            };
                            if row > threshold {
                                sidebar_clone.increment_session_count(&connection_id_str);
                                session_connected_clone.set(true);
                            }
                        }
                    }
                });
            }
        }

        Some(session_id)
    }

    /// Starts a connection and returns the `session_id`
    pub fn start_connection(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
    ) -> Option<Uuid> {
        let state_ref = state.borrow();

        let conn = state_ref.get_connection(connection_id)?;
        let protocol = get_protocol_string(&conn.protocol_config);
        let logging_enabled = state_ref.settings().logging.enabled;

        // Clone connection data before dropping borrow
        let conn_clone = conn.clone();
        drop(state_ref);

        match protocol.as_str() {
            "ssh" => protocols::start_ssh_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
                logging_enabled,
            ),
            "vnc" => protocols::start_vnc_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
            ),
            "rdp" => {
                // RDP connections are handled by start_rdp_session_with_credentials
                // which is called from start_connection_with_credential_resolution
                eprintln!(
                    "Warning: RDP connection reached start_connection without credentials. \
                     Use start_connection_with_credential_resolution instead."
                );
                None
            }
            "spice" => protocols::start_spice_connection(
                state,
                notebook,
                sidebar,
                connection_id,
                &conn_clone,
            ),
            p if p == "zerotrust" || p.starts_with("zerotrust:") => {
                protocols::start_zerotrust_connection(
                    state,
                    notebook,
                    sidebar,
                    connection_id,
                    &conn_clone,
                    logging_enabled,
                )
            }
            _ => {
                // Unknown protocol
                None
            }
        }
    }

    /// Sets up session logging for a terminal session
    ///
    /// Directory creation and file opening are performed asynchronously
    /// to avoid blocking the GTK main thread on slow storage.
    pub fn setup_session_logging(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        session_id: Uuid,
        connection_id: Uuid,
        connection_name: &str,
    ) {
        // Get the log directory and logging modes from settings
        let (log_dir, log_activity, log_input, log_output) =
            if let Ok(state_ref) = state.try_borrow() {
                let settings = state_ref.settings();
                let dir = if settings.logging.log_directory.is_absolute() {
                    settings.logging.log_directory.clone()
                } else {
                    state_ref
                        .config_manager()
                        .config_dir()
                        .join(&settings.logging.log_directory)
                };
                (
                    dir,
                    settings.logging.log_activity,
                    settings.logging.log_input,
                    settings.logging.log_output,
                )
            } else {
                return;
            };

        // Create log file path with timestamp
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let sanitized_name: String = connection_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect();
        let log_filename = format!("{}_{}.log", sanitized_name, timestamp);
        let log_path = log_dir.join(&log_filename);

        // Clone data for the background thread (must be owned/static)
        let connection_name_for_header = connection_name.to_string();
        let connection_name_for_callback = connection_name.to_string();
        let log_dir_clone = log_dir.clone();
        let log_path_clone = log_path.clone();

        // Clone notebook for the callback
        let notebook_clone = notebook.clone();

        // Perform directory creation and file opening in background thread
        crate::utils::spawn_blocking_with_callback(
            move || {
                // Ensure log directory exists
                if let Err(e) = std::fs::create_dir_all(&log_dir_clone) {
                    return Err(format!(
                        "Failed to create log directory '{}': {}",
                        log_dir_clone.display(),
                        e
                    ));
                }

                // Create the log file and write header
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path_clone)
                {
                    Ok(mut file) => {
                        use std::io::Write;
                        let header = format!(
                            "=== Session Log ===\nConnection: {}\nConnection ID: {}\nSession ID: {}\nStarted: {}\n\n",
                            connection_name_for_header,
                            connection_id,
                            session_id,
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                        );
                        if let Err(e) = file.write_all(header.as_bytes()) {
                            return Err(format!("Failed to write log header: {}", e));
                        }
                        Ok(log_path_clone)
                    }
                    Err(e) => Err(format!(
                        "Failed to create log file '{}': {}",
                        log_path_clone.display(),
                        e
                    )),
                }
            },
            move |result: Result<std::path::PathBuf, String>| {
                match result {
                    Ok(log_path) => {
                        eprintln!(
                            "Session logging enabled for '{}': {}",
                            connection_name_for_callback,
                            log_path.display()
                        );

                        // Store log file path in session info
                        notebook_clone.set_log_file(session_id, log_path.clone());

                        // Set up logging handlers based on settings
                        Self::setup_logging_handlers(
                            &notebook_clone,
                            session_id,
                            &log_path,
                            log_activity,
                            log_input,
                            log_output,
                        );
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                    }
                }
            },
        );
    }

    /// Sets up the child exited handler for session cleanup
    pub fn setup_child_exited_handler(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        session_id: Uuid,
        connection_id: Uuid,
    ) {
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();

        notebook.connect_child_exited(session_id, move |exit_status| {
            // Get history entry ID before session info is removed
            let history_entry_id = notebook_clone
                .get_session_info(session_id)
                .and_then(|info| info.history_entry_id);

            // Update session status in state manager
            // This also closes the session logger and finalizes the log file
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let _ = state_mut.terminate_session(session_id);
            }

            // Check if session still exists in notebook
            // If it doesn't, the tab was closed by user
            if notebook_clone.get_session_info(session_id).is_none() {
                // Record connection end in history (user closed tab)
                if let Some(entry_id) = history_entry_id {
                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        state_mut.record_connection_end(entry_id);
                    }
                }
                // Decrement session count - status changes only if no other sessions active
                sidebar_clone.decrement_session_count(&connection_id_str, false);
                return;
            }

            // Parse waitpid status to determine if exit was a failure or intentional kill
            // WIFSIGNALED: (status & 0x7f) != 0
            // WTERMSIG: status & 0x7f
            // WIFEXITED: (status & 0x7f) == 0
            // WEXITSTATUS: (status >> 8) & 0xff

            let term_sig = exit_status & 0x7f;
            let is_signaled = term_sig != 0;
            let exit_code = (exit_status >> 8) & 0xff;

            // Consider it a failure if:
            // 1. Killed by a signal that isn't a standard termination signal (HUP, INT, KILL, TERM)
            // 2. Exited normally with non-zero code, UNLESS that code indicates a standard signal kill (128+N)
            let is_failure = if is_signaled {
                !matches!(term_sig, 1 | 2 | 9 | 15)
            } else {
                exit_code != 0 && !matches!(exit_code, 129 | 130 | 137 | 143)
            };

            // Record connection end/failure in history
            if let Some(entry_id) = history_entry_id {
                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    if is_failure {
                        let error_msg =
                            format!("Exit status: {exit_status} (Signal: {term_sig}, Code: {exit_code})");
                        state_mut.record_connection_failed(entry_id, &error_msg);
                    } else {
                        state_mut.record_connection_end(entry_id);
                    }
                }
            }

            if is_failure {
                eprintln!("Session {session_id} exited with status: {exit_status} (Signal: {term_sig}, Code: {exit_code})");
            }

            // Decrement session count - status changes only if no other sessions active
            sidebar_clone.decrement_session_count(&connection_id_str, is_failure);
        });
    }

    /// Sets up logging handlers for a terminal session based on settings
    ///
    /// Supports three logging modes:
    /// - Activity: logs change counts (default, lightweight)
    /// - Input: logs user commands sent to terminal
    /// - Output: logs full terminal transcript
    #[allow(clippy::too_many_arguments)]
    fn setup_logging_handlers(
        notebook: &SharedNotebook,
        session_id: Uuid,
        log_path: &std::path::Path,
        log_activity: bool,
        log_input: bool,
        log_output: bool,
    ) {
        use std::cell::RefCell;
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::rc::Rc;

        // Create a shared writer for the log file
        let log_writer: Rc<RefCell<Option<std::io::BufWriter<std::fs::File>>>> =
            Rc::new(RefCell::new(None));

        // Open the log file for appending
        match OpenOptions::new().append(true).open(log_path) {
            Ok(file) => {
                *log_writer.borrow_mut() = Some(std::io::BufWriter::new(file));
            }
            Err(e) => {
                eprintln!(
                    "Failed to open log file '{}' for session logging: {}",
                    log_path.display(),
                    e
                );
                return;
            }
        }

        // Set up activity logging (change counts)
        if log_activity {
            let log_writer_clone = log_writer.clone();
            let change_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
            let last_log_time: Rc<RefCell<std::time::Instant>> =
                Rc::new(RefCell::new(std::time::Instant::now()));
            let flush_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

            notebook.connect_contents_changed(session_id, move || {
                let mut counter = change_counter.borrow_mut();
                *counter += 1;

                let mut flush_count = flush_counter.borrow_mut();
                *flush_count += 1;

                let now = std::time::Instant::now();
                let elapsed = now.duration_since(*last_log_time.borrow());

                if *counter >= 100 || elapsed.as_secs() >= 5 {
                    if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                        if let Some(ref mut writer) = *writer_opt {
                            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                            let _ = writeln!(
                                writer,
                                "[{}] Terminal activity ({} changes)",
                                timestamp, *counter
                            );

                            if *flush_count >= 10 || elapsed.as_secs() >= 30 {
                                let _ = writer.flush();
                                *flush_count = 0;
                            }
                        }
                    }

                    *counter = 0;
                    *last_log_time.borrow_mut() = now;
                }
            });
        }

        // Set up input logging (user commands)
        if log_input {
            let log_writer_clone = log_writer.clone();
            let input_buffer: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
            let last_flush: Rc<RefCell<std::time::Instant>> =
                Rc::new(RefCell::new(std::time::Instant::now()));

            notebook.connect_commit(session_id, move |text| {
                let mut buffer = input_buffer.borrow_mut();

                // Handle special characters
                for ch in text.chars() {
                    match ch {
                        '\r' | '\n' => {
                            // End of command - log it
                            if !buffer.is_empty() {
                                if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                                    if let Some(ref mut writer) = *writer_opt {
                                        let timestamp =
                                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                                        let _ =
                                            writeln!(writer, "[{}] INPUT: {}", timestamp, *buffer);
                                        let _ = writer.flush();
                                    }
                                }
                                buffer.clear();
                            }
                        }
                        '\x7f' | '\x08' => {
                            // Backspace - remove last char
                            buffer.pop();
                        }
                        '\x03' => {
                            // Ctrl+C
                            if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                                if let Some(ref mut writer) = *writer_opt {
                                    let timestamp =
                                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                                    let _ = writeln!(writer, "[{}] INPUT: ^C", timestamp);
                                    let _ = writer.flush();
                                }
                            }
                            buffer.clear();
                        }
                        '\x04' => {
                            // Ctrl+D
                            if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                                if let Some(ref mut writer) = *writer_opt {
                                    let timestamp =
                                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                                    let _ = writeln!(writer, "[{}] INPUT: ^D", timestamp);
                                    let _ = writer.flush();
                                }
                            }
                        }
                        _ if ch.is_control() => {
                            // Skip other control characters
                        }
                        _ => {
                            buffer.push(ch);
                        }
                    }
                }

                // Periodic flush for long-running commands
                let now = std::time::Instant::now();
                if now.duration_since(*last_flush.borrow()).as_secs() >= 30 && !buffer.is_empty() {
                    if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                        if let Some(ref mut writer) = *writer_opt {
                            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                            let _ =
                                writeln!(writer, "[{}] INPUT (partial): {}", timestamp, *buffer);
                            let _ = writer.flush();
                        }
                    }
                    *last_flush.borrow_mut() = now;
                }
            });
        }

        // Set up output logging (full transcript)
        if log_output {
            let log_writer_clone = log_writer.clone();
            let notebook_clone = notebook.clone();
            let last_content: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
            let last_log_time: Rc<RefCell<std::time::Instant>> =
                Rc::new(RefCell::new(std::time::Instant::now()));

            notebook.connect_contents_changed(session_id, move || {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(*last_log_time.borrow());

                // Only capture transcript every 5 seconds to avoid performance issues
                if elapsed.as_secs() >= 5 {
                    if let Some(current_text) = notebook_clone.get_terminal_text(session_id) {
                        let mut last = last_content.borrow_mut();

                        // Only log if content changed
                        if current_text != *last {
                            // Find new content (simple diff - just log new lines)
                            let new_lines: Vec<&str> =
                                current_text.lines().skip(last.lines().count()).collect();

                            if !new_lines.is_empty() {
                                if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                                    if let Some(ref mut writer) = *writer_opt {
                                        let timestamp =
                                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                                        let _ = writeln!(writer, "[{}] OUTPUT:", timestamp);
                                        for line in new_lines {
                                            let _ = writeln!(writer, "  {}", line);
                                        }
                                        let _ = writer.flush();
                                    }
                                }
                            }

                            *last = current_text;
                        }
                    }

                    *last_log_time.borrow_mut() = now;
                }
            });
        }
    }

    /// Shows the new connection dialog with optional template selection
    fn show_new_connection_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        crate::window_connection_dialogs::show_new_connection_dialog(
            window.upcast_ref(),
            state,
            sidebar,
        );
    }

    /// Shows the new group dialog with optional parent selection
    fn show_new_group_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        crate::window_connection_dialogs::show_new_group_dialog(
            window.upcast_ref(),
            state,
            sidebar,
        );
    }

    /// Shows the import dialog
    fn show_import_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        crate::window_connection_dialogs::show_import_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the settings dialog
    fn show_settings_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
    ) {
        let mut dialog = SettingsDialog::new(None);

        // Load current settings
        {
            let state_ref = state.borrow();
            dialog.set_settings(state_ref.settings().clone());
        }

        let window_clone = window.clone();
        dialog.run(Some(window), move |result| {
            if let Some(settings) = result {
                // Capture backend and KeePass state for action update
                let backend = settings.secrets.preferred_backend;
                let keepass_enabled = settings.secrets.kdbx_enabled;
                let kdbx_path_exists = settings
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p: &std::path::PathBuf| p.exists());

                // Apply terminal settings to existing terminals
                notebook.apply_settings(&settings.terminal);

                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    if let Err(e) = state_mut.update_settings(settings) {
                        eprintln!("Failed to save settings: {e}");
                    } else {
                        // Update open-keepass action enabled state based on backend
                        if let Some(action) = window_clone.lookup_action("open-keepass") {
                            if let Some(simple_action) = action.downcast_ref::<gio::SimpleAction>()
                            {
                                let action_enabled = match backend {
                                    rustconn_core::config::SecretBackendType::LibSecret
                                    | rustconn_core::config::SecretBackendType::Bitwarden => true,
                                    rustconn_core::config::SecretBackendType::KeePassXc
                                    | rustconn_core::config::SecretBackendType::KdbxFile => {
                                        keepass_enabled && kdbx_path_exists
                                    }
                                };
                                simple_action.set_enabled(action_enabled);
                            }
                        }
                    }
                } else {
                    eprintln!("Failed to borrow state for settings update");
                }
            }
        });
    }

    /// Edits the selected connection or group
    fn edit_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        edit_dialogs::edit_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Shows connection details in the main content area (Info view)
    fn show_connection_details(
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        split_view: &SharedSplitView,
    ) {
        // Get selected item
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Only show details for connections, not groups
        if conn_item.is_group() {
            return;
        }

        let id_str = conn_item.id();
        let Ok(id) = uuid::Uuid::parse_str(&id_str) else {
            return;
        };

        let state_ref = state.borrow();
        let Some(conn) = state_ref.get_connection(id).cloned() else {
            return;
        };
        drop(state_ref);

        // Show Info content in split_view (replaces Welcome)
        split_view.show_info_content(&conn);
    }

    /// Renames the selected connection or group
    fn rename_selected_item(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        edit_dialogs::rename_selected_item(window.upcast_ref(), state, sidebar);
    }

    /// Deletes the selected connection or group
    fn delete_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::delete_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Deletes all selected connections (bulk delete for group operations mode)
    fn delete_selected_connections(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::delete_selected_connections(window.upcast_ref(), state, sidebar);
    }

    /// Shows dialog to move selected items to a group
    fn show_move_selected_to_group_dialog(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::show_move_selected_to_group_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Duplicates the selected connection
    fn duplicate_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::duplicate_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Copies the selected connection to the internal clipboard
    fn copy_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::copy_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Pastes a connection from the internal clipboard
    fn paste_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::paste_connection(window.upcast_ref(), state, sidebar);
    }

    /// Reloads the sidebar with current data (preserving hierarchy)
    fn reload_sidebar(state: &SharedAppState, sidebar: &SharedSidebar) {
        operations::reload_sidebar(state, sidebar);
    }

    /// Reloads the sidebar while preserving tree state
    ///
    /// This method saves the current expanded groups, scroll position, and selection,
    /// reloads the sidebar data, and then restores the state. Use this when editing
    /// connections to maintain the user's view.
    pub fn reload_sidebar_preserving_state(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Save current tree state
        let tree_state = sidebar.save_state();

        // Perform the reload
        Self::reload_sidebar(state, sidebar);

        // Restore tree state
        sidebar.restore_state(&tree_state);
    }

    /// Presents the window to the user
    pub fn present(&self) {
        self.window.present();
    }

    /// Returns a reference to the underlying GTK window
    #[must_use]
    pub const fn gtk_window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// Returns a reference to the toast overlay
    #[must_use]
    #[allow(dead_code)]
    pub fn toast_overlay(&self) -> &SharedToastOverlay {
        &self.toast_overlay
    }

    /// Shows an info toast message
    #[allow(dead_code)]
    pub fn show_toast(&self, message: &str) {
        self.toast_overlay.show_toast(message);
    }

    /// Shows a success toast message
    #[allow(dead_code)]
    pub fn show_success(&self, message: &str) {
        self.toast_overlay.show_success(message);
    }

    /// Shows a warning toast message
    #[allow(dead_code)]
    pub fn show_warning(&self, message: &str) {
        self.toast_overlay.show_warning(message);
    }

    /// Shows an error toast message
    #[allow(dead_code)]
    pub fn show_error(&self, message: &str) {
        self.toast_overlay.show_error(message);
    }

    /// Registers the application icon in the icon theme
    fn register_app_icon() {
        if let Some(display) = gtk4::gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);

            // Add multiple icon search paths for different installation scenarios
            // 1. Development path (cargo run)
            let dev_icons_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons");
            icon_theme.add_search_path(dev_icons_path);

            // 2. System installation paths
            let system_paths = [
                "/usr/share/icons",
                "/usr/local/share/icons",
                "/app/share/icons", // Flatpak
            ];
            for path in &system_paths {
                if std::path::Path::new(path).exists() {
                    icon_theme.add_search_path(path);
                }
            }

            // 3. User local installation path
            if let Some(data_dir) = dirs::data_dir() {
                let user_icons = data_dir.join("icons");
                if user_icons.exists() {
                    icon_theme.add_search_path(user_icons.to_string_lossy().as_ref());
                }
            }
        }
    }

    /// Returns a reference to the connection sidebar
    ///
    /// Note: Part of public API for accessing sidebar from external code.
    #[must_use]
    #[allow(dead_code)]
    pub fn sidebar(&self) -> &ConnectionSidebar {
        &self.sidebar
    }

    /// Returns a clone of the shared sidebar Rc
    #[must_use]
    pub fn sidebar_rc(&self) -> Rc<ConnectionSidebar> {
        self.sidebar.clone()
    }

    /// Returns a reference to the terminal notebook
    ///
    /// Note: Part of public API for accessing notebook from external code.
    #[must_use]
    #[allow(dead_code)]
    pub fn terminal_notebook(&self) -> &TerminalNotebook {
        &self.terminal_notebook
    }

    /// Saves the current expanded groups state to settings
    ///
    /// Note: Part of tree state persistence API.
    #[allow(dead_code)]
    pub fn save_expanded_groups(&self) {
        let expanded = self.sidebar.get_expanded_groups();
        if let Ok(mut state) = self.state.try_borrow_mut() {
            let _ = state.update_expanded_groups(expanded);
        }
    }

    /// Opens a local shell terminal with split view integration
    fn open_local_shell_with_split(
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        state: Option<&SharedAppState>,
    ) {
        // Get terminal settings from state if available
        let terminal_settings = state
            .and_then(|s| s.try_borrow().ok())
            .map(|s| s.settings().terminal.clone())
            .unwrap_or_default();

        let session_id = notebook.create_terminal_tab_with_settings(
            Uuid::nil(),
            "Local Shell",
            "local",
            None,
            &terminal_settings,
        );

        // Get user's default shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        notebook.spawn_command(session_id, &[&shell], None, None);

        // Per spec (Requirement 1): New connections ALWAYS create independent Root_Tabs
        // Register session for potential drag-and-drop, but don't show in split pane
        if let Some(info) = notebook.get_session_info(session_id) {
            // Don't pass terminal - it stays in TabView page
            split_view.add_session(info, None);
        }

        // Hide split view, show TabView content for the new tab
        split_view.widget().set_visible(false);
        split_view.widget().set_vexpand(false);
        notebook.widget().set_vexpand(true);
        notebook.show_tab_view_content();

        // Note: The switch_page signal handler will handle visibility
        // based on whether the session has a split_color assigned
    }

    /// Shows the quick connect dialog with protocol selection
    fn show_quick_connect_dialog(
        window: &adw::ApplicationWindow,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
    ) {
        edit_dialogs::show_quick_connect_dialog(window.upcast_ref(), notebook, split_view, sidebar);
    }

    /// Toggles group operations mode for multi-select
    fn toggle_group_operations_mode(sidebar: &SharedSidebar, enabled: bool) {
        crate::window_sorting::toggle_group_operations_mode(sidebar, enabled);
    }

    /// Sorts connections alphabetically and updates `sort_order`
    fn sort_connections(state: &SharedAppState, sidebar: &SharedSidebar) {
        crate::window_sorting::sort_connections(state, sidebar);
    }

    /// Sorts connections by recent usage (most recently used first)
    fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
        crate::window_sorting::sort_recent(state, sidebar);
    }

    /// Handles drag-drop operations for reordering connections
    fn handle_drag_drop(state: &SharedAppState, sidebar: &SharedSidebar, data: &str) {
        crate::window_sorting::handle_drag_drop(state, sidebar, data);
    }

    /// Shows the export dialog
    ///
    /// Displays a dialog for exporting connections to various formats:
    /// - Ansible Inventory (INI/YAML)
    /// - SSH Config
    /// - Remmina (.remmina files)
    /// - Asbru-CM (YAML)
    ///
    /// Requirements: 3.1, 4.1, 5.1, 6.1
    fn show_export_dialog(window: &adw::ApplicationWindow, state: SharedAppState) {
        let dialog = ExportDialog::new(Some(&window.clone().upcast()));

        // Get connections and groups from state
        let state_ref = state.borrow();
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .map(|c| (*c).clone())
            .collect();
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();
        drop(state_ref);

        // Set data for export
        dialog.set_connections(connections);
        dialog.set_groups(groups);

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(export_result) = result {
                // Optionally open the output location on success
                if !export_result.output_files.is_empty() {
                    if let Some(first_file) = export_result.output_files.first() {
                        ExportDialog::open_output_location(first_file);
                    }
                }

                // Show success notification
                alert::show_success(
                    &window_clone,
                    "Export Complete",
                    &format!(
                        "Successfully exported {} connection(s).\n{} skipped.",
                        export_result.exported_count, export_result.skipped_count
                    ),
                );
            }
        });
    }

    /// Shows the terminal search dialog
    fn show_terminal_search_dialog(window: &adw::ApplicationWindow, notebook: &SharedNotebook) {
        if let Some(terminal) = notebook.get_active_terminal() {
            let dialog =
                crate::dialogs::TerminalSearchDialog::new(Some(&window.clone().upcast()), terminal);
            dialog.show();
        }
    }
}
