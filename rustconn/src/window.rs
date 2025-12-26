//! Main application window
//!
//! This module provides the main window implementation for `RustConn`,
//! including the header bar, sidebar, terminal area, and action handling.

use gtk4::prelude::*;
use gtk4::{
    gio, glib, Application, ApplicationWindow, Button, HeaderBar, Label, MenuButton, Orientation,
    Paned, ScrolledWindow,
};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Returns the protocol string for a connection, including provider info for ZeroTrust
///
/// For ZeroTrust connections, returns "zerotrust:provider" format to enable
/// provider-specific icons in the sidebar.
///
/// Uses the provider enum to determine the provider type for icon display.
fn get_protocol_string(config: &rustconn_core::ProtocolConfig) -> String {
    match config {
        rustconn_core::ProtocolConfig::Ssh(_) => "ssh".to_string(),
        rustconn_core::ProtocolConfig::Rdp(_) => "rdp".to_string(),
        rustconn_core::ProtocolConfig::Vnc(_) => "vnc".to_string(),
        rustconn_core::ProtocolConfig::Spice(_) => "spice".to_string(),
        rustconn_core::ProtocolConfig::ZeroTrust(zt) => {
            // Use provider enum to determine the provider type
            let provider = match zt.provider {
                rustconn_core::models::ZeroTrustProvider::AwsSsm => "aws",
                rustconn_core::models::ZeroTrustProvider::GcpIap => "gcloud",
                rustconn_core::models::ZeroTrustProvider::AzureBastion => "azure",
                rustconn_core::models::ZeroTrustProvider::AzureSsh => "azure_ssh",
                rustconn_core::models::ZeroTrustProvider::OciBastion => "oci",
                rustconn_core::models::ZeroTrustProvider::CloudflareAccess => "cloudflare",
                rustconn_core::models::ZeroTrustProvider::Teleport => "teleport",
                rustconn_core::models::ZeroTrustProvider::TailscaleSsh => "tailscale",
                rustconn_core::models::ZeroTrustProvider::Boundary => "boundary",
                rustconn_core::models::ZeroTrustProvider::Generic => "generic",
            };
            format!("zerotrust:{provider}")
        }
    }
}

use crate::dialogs::{
    ClusterDialog, ClusterListDialog, ConnectionDialog, ExportDialog, ImportDialog, PasswordDialog,
    SettingsDialog, SnippetDialog,
};
use crate::embedded::{EmbeddedSessionTab, RdpLauncher};
use crate::external_window::ExternalWindowManager;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::split_view::{SplitDirection, SplitTerminalView};
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;

/// Shared sidebar type
type SharedSidebar = Rc<ConnectionSidebar>;
/// Shared terminal notebook type
type SharedNotebook = Rc<TerminalNotebook>;
/// Shared split view type
type SharedSplitView = Rc<SplitTerminalView>;

/// Shared external window manager type
type SharedExternalWindowManager = Rc<ExternalWindowManager>;

/// Main application window wrapper
///
/// Provides access to the main window and its components.
pub struct MainWindow {
    window: ApplicationWindow,
    sidebar: SharedSidebar,
    terminal_notebook: SharedNotebook,
    split_view: SharedSplitView,
    state: SharedAppState,
    paned: Paned,
    external_window_manager: SharedExternalWindowManager,
}

impl MainWindow {
    /// Creates a new main window for the application
    #[must_use]
    pub fn new(app: &Application, state: SharedAppState) -> Self {
        // Register custom icon from assets before creating window
        Self::register_app_icon();

        // Create the main window
        let window = ApplicationWindow::builder()
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
        let header_bar = Self::create_header_bar();
        window.set_titlebar(Some(&header_bar));

        // Create the main layout with paned container
        let paned = Paned::new(Orientation::Horizontal);

        // Apply saved sidebar width (with reasonable limits)
        {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let sidebar_width = settings.ui.sidebar_width.unwrap_or(280).clamp(150, 500);
            paned.set_position(sidebar_width);
        }

        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);

        // Create sidebar
        let sidebar = Rc::new(ConnectionSidebar::new());
        paned.set_start_child(Some(sidebar.widget()));

        // Create split terminal view as the main terminal container
        let split_view = Rc::new(SplitTerminalView::new());

        // Create terminal notebook for tab management
        let terminal_notebook = Rc::new(TerminalNotebook::new());

        // Configure notebook to show tabs
        // Content is displayed in split view panes, not in notebook
        terminal_notebook.notebook().set_show_tabs(true);
        terminal_notebook.notebook().set_show_border(false);
        // Don't let notebook expand - it should only show tabs
        terminal_notebook.widget().set_vexpand(false);
        // Ensure notebook is visible
        terminal_notebook.widget().set_visible(true);

        // Create a container for the terminal area
        let terminal_container = gtk4::Box::new(Orientation::Vertical, 0);

        // Add notebook tabs at top for session switching (tabs only, content hidden by size)
        terminal_container.append(terminal_notebook.widget());

        // Add split view as the main content area - takes full space
        split_view.widget().set_vexpand(true);
        split_view.widget().set_hexpand(true);
        terminal_container.append(split_view.widget());

        // Note: drag-and-drop is set up in connect_signals after we have access to notebook

        paned.set_end_child(Some(&terminal_container));

        window.set_child(Some(&paned));

        // Create external window manager
        let external_window_manager = Rc::new(ExternalWindowManager::new());

        let main_window = Self {
            window,
            sidebar,
            terminal_notebook,
            split_view,
            state,
            paned,
            external_window_manager,
        };

        // Set up window actions
        main_window.setup_actions();

        // Load initial data
        main_window.load_connections();

        // Connect signals
        main_window.connect_signals();

        main_window
    }

    /// Creates the header bar with title and controls
    fn create_header_bar() -> HeaderBar {
        let header_bar = HeaderBar::new();
        header_bar.set_show_title_buttons(true);

        // Add title
        let title = Label::new(Some("RustConn"));
        title.add_css_class("title");
        header_bar.set_title_widget(Some(&title));

        // Local shell button (leftmost)
        let local_shell_button = Button::from_icon_name("utilities-terminal-symbolic");
        local_shell_button.set_tooltip_text(Some("Local Shell"));
        local_shell_button.set_action_name(Some("win.local-shell"));
        header_bar.pack_start(&local_shell_button);

        // Add connection button
        let add_button = Button::from_icon_name("list-add-symbolic");
        add_button.set_tooltip_text(Some("New Connection (Ctrl+N)"));
        add_button.set_action_name(Some("win.new-connection"));
        header_bar.pack_start(&add_button);

        // Add import button
        let import_button = Button::from_icon_name("document-open-symbolic");
        import_button.set_tooltip_text(Some("Import Connections (Ctrl+I)"));
        import_button.set_action_name(Some("win.import"));
        header_bar.pack_start(&import_button);

        // Add menu button
        let menu_button = MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Menu")
            .build();

        let menu = Self::create_app_menu();
        menu_button.set_menu_model(Some(&menu));
        header_bar.pack_end(&menu_button);

        // Add settings button
        let settings_button = Button::from_icon_name("emblem-system-symbolic");
        settings_button.set_tooltip_text(Some("Settings (Ctrl+,)"));
        settings_button.set_action_name(Some("win.settings"));
        header_bar.pack_end(&settings_button);

        // Add split view buttons (before settings, so they appear to the left)
        let split_vertical_button = Button::from_icon_name("view-dual-symbolic");
        split_vertical_button.set_tooltip_text(Some("Split Vertical (Ctrl+Shift+S)"));
        split_vertical_button.set_action_name(Some("win.split-vertical"));
        header_bar.pack_end(&split_vertical_button);

        let split_horizontal_button = Button::from_icon_name("view-paged-symbolic");
        split_horizontal_button.set_tooltip_text(Some("Split Horizontal (Ctrl+Shift+H)"));
        split_horizontal_button.set_action_name(Some("win.split-horizontal"));
        header_bar.pack_end(&split_horizontal_button);

        header_bar
    }

    /// Creates the application menu
    fn create_app_menu() -> gio::Menu {
        let menu = gio::Menu::new();

        // Connections section
        let conn_section = gio::Menu::new();
        conn_section.append(Some("New Connection"), Some("win.new-connection"));
        conn_section.append(Some("New Group"), Some("win.new-group"));
        conn_section.append(Some("Quick Connect"), Some("win.quick-connect"));
        conn_section.append(Some("Local Shell"), Some("win.local-shell"));
        menu.append_section(None, &conn_section);

        // Tools section (managers)
        let tools_section = gio::Menu::new();
        tools_section.append(Some("Snippets..."), Some("win.manage-snippets"));
        tools_section.append(Some("Clusters..."), Some("win.manage-clusters"));
        tools_section.append(Some("Templates..."), Some("win.manage-templates"));
        tools_section.append(Some("Active Sessions"), Some("win.show-sessions"));
        menu.append_section(None, &tools_section);

        // File section (import/export connections)
        let file_section = gio::Menu::new();
        file_section.append(Some("Import Connections..."), Some("win.import"));
        file_section.append(Some("Export Connections..."), Some("win.export"));
        menu.append_section(None, &file_section);

        // Edit section
        let edit_section = gio::Menu::new();
        edit_section.append(Some("Copy Connection"), Some("win.copy-connection"));
        edit_section.append(Some("Paste Connection"), Some("win.paste-connection"));
        menu.append_section(None, &edit_section);

        // App section
        let app_section = gio::Menu::new();
        app_section.append(Some("Settings"), Some("win.settings"));
        app_section.append(Some("About"), Some("app.about"));
        app_section.append(Some("Quit"), Some("app.quit"));
        menu.append_section(None, &app_section);

        menu
    }

    /// Sets up window actions
    fn setup_actions(&self) {
        let window = &self.window;
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();

        // Set up action groups
        self.setup_connection_actions(window, &state, &sidebar);
        self.setup_edit_actions(window, &state, &sidebar);
        self.setup_terminal_actions(window, &terminal_notebook, &sidebar);
        self.setup_navigation_actions(window, &terminal_notebook, &sidebar);
        self.setup_group_operations_actions(window, &state, &sidebar);
        self.setup_snippet_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_cluster_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_template_actions(window, &state, &sidebar);
        self.setup_split_view_actions(window);
        self.setup_document_actions(window, &state, &sidebar);
        self.setup_misc_actions(window, &state, &sidebar, &terminal_notebook);
    }

    /// Sets up connection-related actions (new, import, settings)
    fn setup_connection_actions(
        &self,
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // New connection action
        let new_conn_action = gio::SimpleAction::new("new-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_conn_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_new_connection_dialog(&win, state_clone.clone(), sidebar_clone.clone());
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
                Self::show_new_group_dialog(&win, state_clone.clone(), sidebar_clone.clone());
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
                Self::show_import_dialog(&win, state_clone.clone(), sidebar_clone.clone());
            }
        });
        window.add_action(&import_action);

        // Settings action
        let settings_action = gio::SimpleAction::new("settings", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        settings_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_settings_dialog(&win, state_clone.clone());
            }
        });
        window.add_action(&settings_action);

        // Open KeePass action - opens KeePassXC with configured database
        let open_keepass_action = gio::SimpleAction::new("open-keepass", None);
        let state_clone = state.clone();
        open_keepass_action.connect_activate(move |_, _| {
            let state_ref = state_clone.borrow();
            let settings = state_ref.settings();

            if settings.secrets.kdbx_enabled {
                if let Some(ref kdbx_path) = settings.secrets.kdbx_path {
                    if kdbx_path.exists() {
                        // Open KeePassXC with the database file
                        if let Err(e) = std::process::Command::new("keepassxc")
                            .arg(kdbx_path)
                            .spawn()
                        {
                            eprintln!("Failed to open KeePassXC: {e}");
                        }
                    } else {
                        eprintln!("KeePass database file not found: {}", kdbx_path.display());
                    }
                }
            }
        });
        // Initially disabled, will be enabled when integration is active
        open_keepass_action.set_enabled(
            state.borrow().settings().secrets.kdbx_enabled
                && state
                    .borrow()
                    .settings()
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p| p.exists()),
        );
        window.add_action(&open_keepass_action);

        // Export action
        let export_action = gio::SimpleAction::new("export", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        export_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_export_dialog(&win, state_clone.clone());
            }
        });
        window.add_action(&export_action);
    }

    /// Sets up edit-related actions (edit, delete, duplicate, move)
    fn setup_edit_actions(
        &self,
        window: &ApplicationWindow,
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
                Self::edit_selected_connection(&win, &state_clone, &sidebar_clone);
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
                Self::delete_selected_connection(&win, &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&delete_action);

        // Duplicate connection action
        let duplicate_action = gio::SimpleAction::new("duplicate-connection", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        duplicate_action.connect_activate(move |_, _| {
            Self::duplicate_selected_connection(&state_clone, &sidebar_clone);
        });
        window.add_action(&duplicate_action);

        // Move to group action
        let move_to_group_action = gio::SimpleAction::new("move-to-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        move_to_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_move_to_group_dialog(&win, &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&move_to_group_action);

        // View details action
        let view_details_action = gio::SimpleAction::new("view-details", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        view_details_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_connection_details(&win, &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&view_details_action);

        // Copy connection action
        let copy_connection_action = gio::SimpleAction::new("copy-connection", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        copy_connection_action.connect_activate(move |_, _| {
            Self::copy_selected_connection(&state_clone, &sidebar_clone);
        });
        window.add_action(&copy_connection_action);

        // Paste connection action
        let paste_connection_action = gio::SimpleAction::new("paste-connection", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        paste_connection_action.connect_activate(move |_, _| {
            Self::paste_connection(&state_clone, &sidebar_clone);
        });
        window.add_action(&paste_connection_action);
    }

    /// Sets up terminal-related actions (copy, paste, close tab)
    fn setup_terminal_actions(
        &self,
        window: &ApplicationWindow,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // Search action
        let search_action = gio::SimpleAction::new("search", None);
        let sidebar_clone = sidebar.clone();
        search_action.connect_activate(move |_, _| {
            sidebar_clone.search_entry().grab_focus();
        });
        window.add_action(&search_action);

        // Copy action
        let copy_action = gio::SimpleAction::new("copy", None);
        let notebook_clone = terminal_notebook.clone();
        copy_action.connect_activate(move |_, _| {
            notebook_clone.copy_to_clipboard();
        });
        window.add_action(&copy_action);

        // Paste action
        let paste_action = gio::SimpleAction::new("paste", None);
        let notebook_clone = terminal_notebook.clone();
        paste_action.connect_activate(move |_, _| {
            notebook_clone.paste_from_clipboard();
        });
        window.add_action(&paste_action);

        // Close tab action - placeholder
        let close_tab_action = gio::SimpleAction::new("close-tab", None);
        window.add_action(&close_tab_action);

        // Local shell action
        let local_shell_action = gio::SimpleAction::new("local-shell", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        local_shell_action.connect_activate(move |_, _| {
            Self::open_local_shell_with_split(&notebook_clone, &split_view_clone);
        });
        window.add_action(&local_shell_action);

        // Quick connect action
        let quick_connect_action = gio::SimpleAction::new("quick-connect", None);
        let window_weak = window.downgrade();
        let notebook_clone = terminal_notebook.clone();
        quick_connect_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_quick_connect_dialog(&win, notebook_clone.clone());
            }
        });
        window.add_action(&quick_connect_action);
    }

    /// Sets up navigation actions (focus, tabs)
    fn setup_navigation_actions(
        &self,
        window: &ApplicationWindow,
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
            let notebook = notebook_clone.notebook();
            let current = notebook.current_page().unwrap_or(0);
            let total = notebook_clone.tab_count();
            if total > 0 {
                let next = (current + 1) % total;
                notebook.set_current_page(Some(next));
            }
        });
        window.add_action(&next_tab_action);

        // Previous tab action
        let prev_tab_action = gio::SimpleAction::new("prev-tab", None);
        let notebook_clone = terminal_notebook.clone();
        prev_tab_action.connect_activate(move |_, _| {
            let notebook = notebook_clone.notebook();
            let current = notebook.current_page().unwrap_or(0);
            let total = notebook_clone.tab_count();
            if total > 0 {
                let prev = if current == 0 { total - 1 } else { current - 1 };
                notebook.set_current_page(Some(prev));
            }
        });
        window.add_action(&prev_tab_action);
    }

    /// Sets up group operations actions (select all, delete selected, etc.)
    fn setup_group_operations_actions(
        &self,
        window: &ApplicationWindow,
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
                Self::delete_selected_connections(&win, &state_clone, &sidebar_clone);
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
                Self::show_move_selected_to_group_dialog(&win, &state_clone, &sidebar_clone);
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
        window: &ApplicationWindow,
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
                Self::show_new_snippet_dialog(&win, state_clone.clone());
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
                Self::show_snippets_manager(&win, state_clone.clone(), notebook_clone.clone());
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
                Self::show_snippet_picker(&win, state_clone.clone(), notebook_clone.clone());
            }
        });
        window.add_action(&execute_snippet_action);

        // Show sessions action
        let show_sessions_action = gio::SimpleAction::new("show-sessions", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        show_sessions_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_sessions_manager(
                    &win,
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
        window: &ApplicationWindow,
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
                Self::show_new_cluster_dialog(&win, state_clone.clone(), notebook_clone.clone());
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
                Self::show_clusters_manager(
                    &win,
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
        window: &ApplicationWindow,
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
                Self::show_templates_manager(&win, state_clone.clone(), sidebar_clone.clone());
            }
        });
        window.add_action(&manage_templates_action);
    }

    /// Sets up split view actions
    fn setup_split_view_actions(&self, window: &ApplicationWindow) {
        // Split horizontal action
        let split_horizontal_action = gio::SimpleAction::new("split-horizontal", None);
        let split_view_clone = self.split_view.clone();
        let split_view_for_close = self.split_view.clone();
        let notebook_for_split_h = self.terminal_notebook.clone();
        split_horizontal_action.connect_activate(move |_, _| {
            let sv = split_view_for_close.clone();
            if let Some(new_pane_id) =
                split_view_clone.split_with_close_callback(SplitDirection::Horizontal, move || {
                    let _ = sv.close_pane();
                })
            {
                let notebook = notebook_for_split_h.clone();
                split_view_clone.setup_pane_drop_target_with_callback(
                    new_pane_id,
                    move |session_id| {
                        let info = notebook.get_session_info(session_id)?;
                        let terminal = notebook.get_terminal(session_id);
                        Some((info, terminal))
                    },
                );
            }
        });
        window.add_action(&split_horizontal_action);

        // Split vertical action
        let split_vertical_action = gio::SimpleAction::new("split-vertical", None);
        let split_view_clone = self.split_view.clone();
        let split_view_for_close = self.split_view.clone();
        let notebook_for_split_v = self.terminal_notebook.clone();
        split_vertical_action.connect_activate(move |_, _| {
            let sv = split_view_for_close.clone();
            if let Some(new_pane_id) =
                split_view_clone.split_with_close_callback(SplitDirection::Vertical, move || {
                    let _ = sv.close_pane();
                })
            {
                let notebook = notebook_for_split_v.clone();
                split_view_clone.setup_pane_drop_target_with_callback(
                    new_pane_id,
                    move |session_id| {
                        let info = notebook.get_session_info(session_id)?;
                        let terminal = notebook.get_terminal(session_id);
                        Some((info, terminal))
                    },
                );
            }
        });
        window.add_action(&split_vertical_action);

        // Close pane action
        let close_pane_action = gio::SimpleAction::new("close-pane", None);
        let split_view_clone = self.split_view.clone();
        close_pane_action.connect_activate(move |_, _| {
            let _ = split_view_clone.close_pane();
        });
        window.add_action(&close_pane_action);

        // Focus next pane action
        let focus_next_pane_action = gio::SimpleAction::new("focus-next-pane", None);
        let split_view_clone = self.split_view.clone();
        focus_next_pane_action.connect_activate(move |_, _| {
            let _ = split_view_clone.focus_next_pane();
        });
        window.add_action(&focus_next_pane_action);
    }

    /// Sets up document management actions
    fn setup_document_actions(
        &self,
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        use crate::dialogs::{
            CloseDocumentDialog, DocumentDialogResult, NewDocumentDialog, OpenDocumentDialog,
            SaveDocumentDialog,
        };

        // New document action
        let new_doc_action = gio::SimpleAction::new("new-document", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_doc_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let dialog = NewDocumentDialog::new(Some(&win.clone().upcast()));
                let state_for_cb = state_clone.clone();
                let _sidebar_for_cb = sidebar_clone.clone();
                dialog.set_callback(move |result| {
                    if let Some(DocumentDialogResult::Create { name, password: _ }) = result {
                        let mut state_ref = state_for_cb.borrow_mut();
                        let _doc_id = state_ref.create_document(name);
                        drop(state_ref);
                        // Refresh sidebar - would need to call load_connections or similar
                    }
                });
                dialog.present();
            }
        });
        window.add_action(&new_doc_action);

        // Open document action
        let open_doc_action = gio::SimpleAction::new("open-document", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        open_doc_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let dialog = OpenDocumentDialog::new();
                let state_for_cb = state_clone.clone();
                let _sidebar_for_cb = sidebar_clone.clone();
                let win_for_cb = win.clone();
                dialog.set_callback(move |result| {
                    if let Some(DocumentDialogResult::Open { path, password }) = result {
                        let mut state_ref = state_for_cb.borrow_mut();
                        match state_ref.open_document(&path, password.as_deref()) {
                            Ok(_doc_id) => {
                                drop(state_ref);
                                // Refresh sidebar
                            }
                            Err(e) => {
                                drop(state_ref);
                                Self::show_error_toast(
                                    &win_for_cb,
                                    &format!("Failed to open document: {e}"),
                                );
                            }
                        }
                    }
                });
                dialog.present(Some(&win.clone().upcast()));
            }
        });
        window.add_action(&open_doc_action);

        // Save document action
        let save_doc_action = gio::SimpleAction::new("save-document", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        save_doc_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                if let Some(doc_id) = state_ref.active_document_id() {
                    if let Some(doc) = state_ref.get_document(doc_id) {
                        let doc_name = doc.name.clone();
                        let existing_path =
                            state_ref.get_document_path(doc_id).map(|p| p.to_path_buf());
                        drop(state_ref);

                        if let Some(path) = existing_path {
                            // Save to existing path
                            let mut state_ref = state_clone.borrow_mut();
                            if let Err(e) = state_ref.save_document(doc_id, &path, None) {
                                drop(state_ref);
                                Self::show_error_toast(
                                    &win,
                                    &format!("Failed to save document: {e}"),
                                );
                            }
                        } else {
                            // Show save dialog
                            let dialog = SaveDocumentDialog::new();
                            let state_for_cb = state_clone.clone();
                            let win_for_cb = win.clone();
                            dialog.set_callback(move |result| {
                                if let Some(DocumentDialogResult::Save { id, path, password }) =
                                    result
                                {
                                    let mut state_ref = state_for_cb.borrow_mut();
                                    if let Err(e) =
                                        state_ref.save_document(id, &path, password.as_deref())
                                    {
                                        drop(state_ref);
                                        Self::show_error_toast(
                                            &win_for_cb,
                                            &format!("Failed to save document: {e}"),
                                        );
                                    }
                                }
                            });
                            dialog.present(Some(&win.clone().upcast()), doc_id, &doc_name);
                        }
                    }
                }
            }
        });
        window.add_action(&save_doc_action);

        // Close document action
        let close_doc_action = gio::SimpleAction::new("close-document", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        close_doc_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                if let Some(doc_id) = state_ref.active_document_id() {
                    let is_dirty = state_ref.is_document_dirty(doc_id);
                    let doc_name = state_ref
                        .get_document(doc_id)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| "Untitled".to_string());
                    drop(state_ref);

                    if is_dirty {
                        // Show save prompt
                        let dialog = CloseDocumentDialog::new();
                        let state_for_cb = state_clone.clone();
                        let _sidebar_for_cb = sidebar_clone.clone();
                        let _win_for_cb = win.clone();
                        dialog.set_callback(move |result| {
                            match result {
                                Some(DocumentDialogResult::Close { id, save: true }) => {
                                    // Save then close
                                    let state_ref = state_for_cb.borrow();
                                    let existing_path =
                                        state_ref.get_document_path(id).map(|p| p.to_path_buf());
                                    drop(state_ref);

                                    if let Some(path) = existing_path {
                                        let mut state_ref = state_for_cb.borrow_mut();
                                        let _ = state_ref.save_document(id, &path, None);
                                        let _ = state_ref.close_document(id);
                                    }
                                    // Refresh sidebar
                                }
                                Some(DocumentDialogResult::Close { id, save: false }) => {
                                    // Close without saving
                                    let mut state_ref = state_for_cb.borrow_mut();
                                    let _ = state_ref.close_document(id);
                                    // Refresh sidebar
                                }
                                _ => {}
                            }
                        });
                        dialog.present(Some(&win.clone().upcast()), doc_id, &doc_name);
                    } else {
                        // Close directly
                        let mut state_ref = state_clone.borrow_mut();
                        let _ = state_ref.close_document(doc_id);
                        // Refresh sidebar
                    }
                }
            }
        });
        window.add_action(&close_doc_action);

        // Export document action
        let export_doc_action = gio::SimpleAction::new("export-document", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        export_doc_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                if let Some(doc_id) = state_ref.active_document_id() {
                    if let Some(doc) = state_ref.get_document(doc_id) {
                        let doc_name = doc.name.clone();
                        drop(state_ref);

                        // Show file save dialog for export
                        let filter = gtk4::FileFilter::new();
                        filter.add_pattern("*.json");
                        filter.add_pattern("*.yaml");
                        filter.set_name(Some("Document Files"));

                        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                        filters.append(&filter);

                        let dialog = gtk4::FileDialog::builder()
                            .title("Export Document")
                            .filters(&filters)
                            .initial_name(format!("{doc_name}.json"))
                            .modal(true)
                            .build();

                        let state_for_cb = state_clone.clone();
                        let win_for_cb = win.clone();

                        dialog.save(
                            Some(&win.clone().upcast::<gtk4::Window>()),
                            gtk4::gio::Cancellable::NONE,
                            move |result| {
                                if let Ok(file) = result {
                                    if let Some(path) = file.path() {
                                        let state_ref = state_for_cb.borrow();
                                        if let Err(e) = state_ref.export_document(doc_id, &path) {
                                            drop(state_ref);
                                            Self::show_error_toast(
                                                &win_for_cb,
                                                &format!("Failed to export document: {e}"),
                                            );
                                        }
                                    }
                                }
                            },
                        );
                    }
                }
            }
        });
        window.add_action(&export_doc_action);

        // Import document action
        let import_doc_action = gio::SimpleAction::new("import-document", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        import_doc_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                // Show file open dialog for import
                let filter = gtk4::FileFilter::new();
                filter.add_pattern("*.json");
                filter.add_pattern("*.yaml");
                filter.add_pattern("*.yml");
                filter.add_pattern("*.rcdb");
                filter.set_name(Some("Document Files"));

                let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);

                let dialog = gtk4::FileDialog::builder()
                    .title("Import Document")
                    .filters(&filters)
                    .modal(true)
                    .build();

                let state_for_cb = state_clone.clone();
                let _sidebar_for_cb = sidebar_clone.clone();
                let win_for_cb = win.clone();

                dialog.open(
                    Some(&win.clone().upcast::<gtk4::Window>()),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                let mut state_ref = state_for_cb.borrow_mut();
                                match state_ref.import_document(&path) {
                                    Ok(_doc_id) => {
                                        drop(state_ref);
                                        // Refresh sidebar
                                    }
                                    Err(e) => {
                                        drop(state_ref);
                                        Self::show_error_toast(
                                            &win_for_cb,
                                            &format!("Failed to import document: {e}"),
                                        );
                                    }
                                }
                            }
                        }
                    },
                );
            }
        });
        window.add_action(&import_doc_action);
    }

    /// Sets up miscellaneous actions (drag-drop)
    fn setup_misc_actions(
        &self,
        window: &ApplicationWindow,
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

        // Set up drag-and-drop for initial pane with notebook lookup
        if let Some(initial_pane_id) = split_view.pane_ids().first().copied() {
            let notebook_for_drop = terminal_notebook.clone();
            split_view.setup_pane_drop_target_with_callback(initial_pane_id, move |session_id| {
                let info = notebook_for_drop.get_session_info(session_id)?;
                let terminal = notebook_for_drop.get_terminal(session_id);
                Some((info, terminal))
            });
        }

        // Connect sidebar search
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sidebar.search_entry().connect_search_changed(move |entry| {
            let query = entry.text();
            Self::filter_connections(&state_clone, &sidebar_clone, &query);
        });

        // Add to search history when user presses Enter or stops searching
        let sidebar_for_history = sidebar.clone();
        sidebar.search_entry().connect_activate(move |entry| {
            let query = entry.text().to_string();
            if !query.is_empty() {
                sidebar_for_history.add_to_search_history(&query);
            }
        });

        // Also add to history when search entry loses focus with non-empty query
        let sidebar_for_focus = sidebar.clone();
        sidebar
            .search_entry()
            .connect_has_focus_notify(move |entry| {
                if !entry.has_focus() {
                    let query = entry.text().to_string();
                    if !query.is_empty() {
                        sidebar_for_focus.add_to_search_history(&query);
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

        // Connect notebook tab switch to show session in split view
        let split_view_clone = split_view.clone();
        let notebook_clone = terminal_notebook.clone();
        let notebook_container = terminal_notebook.widget().clone();
        let notebook_widget = terminal_notebook.notebook().clone();
        terminal_notebook
            .notebook()
            .connect_switch_page(move |_notebook_widget, _, page_num| {
                // Get session ID for this page number
                if let Some(session_id) = notebook_clone.get_session_id_for_page(page_num) {
                    // Check if this is a VNC, RDP, or SPICE session - they display in notebook
                    if let Some(info) = notebook_clone.get_session_info(session_id) {
                        if info.protocol == "vnc"
                            || info.protocol == "rdp"
                            || info.protocol == "spice"
                        {
                            // For VNC/RDP/SPICE: hide split view, expand notebook to show content
                            split_view_clone.widget().set_visible(false);
                            split_view_clone.widget().set_vexpand(false);
                            notebook_container.set_vexpand(true);
                            notebook_widget.set_vexpand(true);
                            // Show notebook page content
                            notebook_clone.show_page_content(page_num);
                            return;
                        }
                    }

                    // For SSH sessions: show split view, collapse notebook tabs area
                    split_view_clone.widget().set_visible(true);
                    split_view_clone.widget().set_vexpand(true);
                    // Collapse notebook - only show tabs, not content
                    notebook_container.set_vexpand(false);
                    notebook_widget.set_vexpand(false);
                    // Hide all notebook page content for SSH sessions
                    notebook_clone.hide_all_page_content_except(None);

                    // Check if this session is already shown in any pane
                    let pane_ids = split_view_clone.pane_ids();
                    let mut found_pane = None;

                    for pane_id in &pane_ids {
                        let pane_session = split_view_clone.get_pane_session(*pane_id);
                        if pane_session == Some(session_id) {
                            found_pane = Some(*pane_id);
                            break;
                        }
                    }

                    if let Some(pane_id) = found_pane {
                        // Session already shown in this pane - just focus it
                        let _ = split_view_clone.focus_pane(pane_id);
                        // Also grab focus on the terminal
                        if let Some(terminal) = split_view_clone.get_terminal(session_id) {
                            terminal.grab_focus();
                        }
                    } else {
                        // Session not shown in any pane - find best pane to show it
                        // Prefer: 1) empty pane, 2) focused pane, 3) first pane
                        let mut target_pane = None;

                        // First, look for an empty pane (no session)
                        for pane_id in &pane_ids {
                            if split_view_clone.get_pane_session(*pane_id).is_none() {
                                target_pane = Some(*pane_id);
                                break;
                            }
                        }

                        // If no empty pane, use focused pane or first pane
                        if target_pane.is_none() {
                            target_pane = split_view_clone
                                .focused_pane_id()
                                .or_else(|| pane_ids.first().copied());
                        }

                        if let Some(pane_id) = target_pane {
                            let _ = split_view_clone.focus_pane(pane_id);

                            // Always ensure session and terminal are in split_view
                            if let Some(info) = notebook_clone.get_session_info(session_id) {
                                let terminal = notebook_clone.get_terminal(session_id);
                                split_view_clone.add_session(info, terminal);
                            }

                            let _ = split_view_clone.show_session(session_id);
                        }
                    }
                } else {
                    // Welcome tab (page 0) - show welcome content in focused pane
                    split_view_clone.show_welcome_in_focused_pane();
                }
            });

        // Connect close-tab action with split_view cleanup
        let notebook_for_close = terminal_notebook;
        let split_view_for_close = split_view;
        let sidebar_for_close = sidebar.clone();
        if let Some(action) = window.lookup_action("close-tab") {
            if let Some(simple_action) = action.downcast_ref::<gio::SimpleAction>() {
                simple_action.connect_activate(move |_, _| {
                    if let Some(session_id) = notebook_for_close.get_active_session_id() {
                        // Get connection ID to update sidebar
                        if let Some(info) = notebook_for_close.get_session_info(session_id) {
                            sidebar_for_close.update_connection_status(
                                &info.connection_id.to_string(),
                                "disconnected",
                            );
                        }

                        // First clear from split view panes
                        split_view_for_close.clear_session_from_panes(session_id);
                        // Then close the tab
                        notebook_for_close.close_tab(session_id);

                        // After closing, if there's an active session, ensure it's shown in split view
                        if let Some(new_session_id) = notebook_for_close.get_active_session_id() {
                            if let Some(info) = notebook_for_close.get_session_info(new_session_id)
                            {
                                if info.protocol == "ssh" || info.protocol.starts_with("zerotrust")
                                {
                                    let _ = split_view_for_close.show_session(new_session_id);
                                }
                            }
                        }
                    }
                });
            }
        }

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
        let state = self.state.borrow();
        let store = self.sidebar.store();
        let expanded_groups = state.expanded_groups().clone();

        // Clear existing items
        store.remove_all();

        // Add root groups
        for group in state.get_root_groups() {
            let group_item = ConnectionItem::new_group(&group.id.to_string(), &group.name);
            self.add_group_children(&state, &group_item, group.id);
            store.append(&group_item);
        }

        // Add ungrouped connections
        for conn in state.get_ungrouped_connections() {
            let protocol = get_protocol_string(&conn.protocol_config);
            let status = self
                .sidebar
                .get_connection_status(&conn.id.to_string())
                .unwrap_or_else(|| "disconnected".to_string());
            let item = ConnectionItem::new_connection_with_status(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
                &status,
            );
            store.append(&item);
        }

        drop(state);

        // Apply expanded state after populating
        self.sidebar.apply_expanded_groups(&expanded_groups);
    }

    /// Recursively adds group children
    #[allow(clippy::self_only_used_in_recursion)]
    fn add_group_children(
        &self,
        state: &std::cell::Ref<crate::state::AppState>,
        parent_item: &ConnectionItem,
        group_id: Uuid,
    ) {
        // Add child groups
        for child_group in state.get_child_groups(group_id) {
            let child_item =
                ConnectionItem::new_group(&child_group.id.to_string(), &child_group.name);
            self.add_group_children(state, &child_item, child_group.id);
            parent_item.add_child(&child_item);
        }

        // Add connections in this group
        for conn in state.get_connections_by_group(group_id) {
            let protocol = get_protocol_string(&conn.protocol_config);
            let status = self
                .sidebar
                .get_connection_status(&conn.id.to_string())
                .unwrap_or_else(|| "disconnected".to_string());
            let item = ConnectionItem::new_connection_with_status(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
                &status,
            );
            parent_item.add_child(&item);
        }
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

        // Parse query and use SearchEngine for advanced search
        let search_engine = SearchEngine::new();
        let parsed_query = match SearchEngine::parse_query(query) {
            Ok(q) => q,
            Err(_) => {
                // Fall back to simple text search on parse error
                rustconn_core::search::SearchQuery::with_text(query)
            }
        };

        // Get connections and groups for search
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        let groups: Vec<_> = state_ref.list_groups().iter().cloned().cloned().collect();

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
    fn start_connection_with_credential_resolution(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
    ) {
        // Get connection info and determine credential handling
        let (is_rdp, _password_source, _needs_prompt) = {
            let state_ref = state.borrow();
            let conn = match state_ref.get_connection(connection_id) {
                Some(c) => c,
                None => return,
            };

            let is_rdp = matches!(conn.protocol_config, rustconn_core::ProtocolConfig::Rdp(_));
            let password_source = conn.password_source;
            let needs_prompt = state_ref.should_prompt_for_credentials(conn);

            (is_rdp, password_source, needs_prompt)
        };

        // Try to resolve credentials from backends
        let resolved_credentials = {
            let state_ref = state.borrow();
            match state_ref.resolve_credentials_for_connection(connection_id) {
                Ok(creds) => creds,
                Err(e) => {
                    eprintln!("Warning: Failed to resolve credentials: {e}");
                    None
                }
            }
        };

        // Check for cached credentials
        let cached_credentials = {
            let state_ref = state.borrow();
            state_ref.get_cached_credentials(connection_id).map(|c| {
                use secrecy::ExposeSecret;
                (
                    c.username.clone(),
                    c.password.expose_secret().to_string(),
                    c.domain.clone(),
                )
            })
        };

        // Determine if we have usable credentials (for future use in enhanced prompting)
        let _has_credentials = resolved_credentials.is_some() || cached_credentials.is_some();

        // For RDP connections that need credentials
        if is_rdp {
            // Use resolved credentials if available
            if let Some(ref creds) = resolved_credentials {
                if let (Some(username), Some(password)) = (&creds.username, creds.expose_password())
                {
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

            // Check if we can skip password dialog (verified credentials from KeePass)
            let can_skip = {
                let state_ref = state.borrow();
                state_ref.can_skip_password_dialog(connection_id)
            };

            if can_skip {
                // Try to resolve credentials one more time (they should be available)
                let state_ref = state.borrow();
                if let Ok(Some(creds)) = state_ref.resolve_credentials_for_connection(connection_id)
                {
                    if let (Some(username), Some(password)) =
                        (&creds.username, creds.expose_password())
                    {
                        drop(state_ref);
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
            }

            // Need to prompt for credentials
            if let Some(window) = notebook.widget().ancestor(ApplicationWindow::static_type()) {
                if let Some(app_window) = window.downcast_ref::<ApplicationWindow>() {
                    Self::start_rdp_with_password_dialog(
                        state,
                        notebook,
                        split_view,
                        sidebar,
                        connection_id,
                        app_window,
                    );
                    return;
                }
            }
        }

        // Check if this is a VNC connection
        let is_vnc = {
            let state_ref = state.borrow();
            state_ref
                .get_connection(connection_id)
                .is_some_and(|c| matches!(c.protocol_config, rustconn_core::ProtocolConfig::Vnc(_)))
        };

        // For VNC connections - check if we have password
        if is_vnc {
            // Use resolved credentials if available (VNC only needs password)
            if let Some(ref creds) = resolved_credentials {
                if creds.expose_password().is_some() {
                    // Cache the password for use in start_connection
                    if let Some(password) = creds.expose_password() {
                        if let Ok(mut state_mut) = state.try_borrow_mut() {
                            state_mut.cache_credentials(connection_id, "", password, "");
                        }
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

            // Check if we can skip password dialog (verified credentials from KeePass)
            let can_skip = {
                let state_ref = state.borrow();
                state_ref.can_skip_password_dialog(connection_id)
            };

            if can_skip {
                // Try to resolve credentials one more time (they should be available)
                let state_ref = state.borrow();
                if let Ok(Some(creds)) = state_ref.resolve_credentials_for_connection(connection_id)
                {
                    if let Some(password) = creds.expose_password() {
                        // Cache the password for use in start_connection
                        drop(state_ref);
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
            }

            // Need to prompt for VNC password
            if let Some(window) = notebook.widget().ancestor(ApplicationWindow::static_type()) {
                if let Some(app_window) = window.downcast_ref::<ApplicationWindow>() {
                    Self::start_vnc_with_password_dialog(
                        state,
                        notebook,
                        split_view,
                        sidebar,
                        connection_id,
                        app_window,
                    );
                    return;
                }
            }
        }

        // For SSH connections
        // SSH typically uses key-based auth, but we can pass credentials if available
        if let Some(ref creds) = resolved_credentials {
            // Store resolved credentials in cache for potential use
            if let (Some(username), Some(password)) = (&creds.username, creds.expose_password()) {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(connection_id, username, password, "");
                }
            }
        }

        // Start SSH connection
        Self::start_connection_with_split(&state, &notebook, &split_view, &sidebar, connection_id);
    }

    /// Starts an RDP connection with password dialog
    fn start_rdp_with_password_dialog(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        window: &ApplicationWindow,
    ) {
        // Check if we can skip password dialog (verified credentials from KeePass)
        let can_skip = {
            let state_ref = state.borrow();
            state_ref.can_skip_password_dialog(connection_id)
        };

        if can_skip {
            // Try to resolve credentials from backends (KeePass)
            let state_ref = state.borrow();
            if let Ok(Some(creds)) = state_ref.resolve_credentials_for_connection(connection_id) {
                if let (Some(username), Some(password)) = (&creds.username, creds.expose_password())
                {
                    drop(state_ref);
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
        }

        // Check if we have cached credentials
        let cached = {
            let state_ref = state.borrow();
            state_ref.get_cached_credentials(connection_id).map(|c| {
                use secrecy::ExposeSecret;
                (
                    c.username.clone(),
                    c.password.expose_secret().to_string(),
                    c.domain.clone(),
                )
            })
        };

        if let Some((username, password, domain)) = cached {
            // Use cached credentials directly
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

        // Get connection info for dialog
        let (conn_name, username, domain) = {
            let state_ref = state.borrow();
            if let Some(conn) = state_ref.get_connection(connection_id) {
                (
                    conn.name.clone(),
                    conn.username.clone().unwrap_or_default(),
                    conn.domain.clone().unwrap_or_default(),
                )
            } else {
                return;
            }
        };

        // Create and show password dialog
        let dialog = PasswordDialog::new(Some(window));
        dialog.set_connection_name(&conn_name);
        dialog.set_username(&username);
        dialog.set_domain(&domain);

        let sidebar_clone = sidebar.clone();
        dialog.show(move |result| {
            if let Some(creds) = result {
                // Cache credentials if requested
                if creds.save_credentials {
                    if let Ok(mut state_mut) = state.try_borrow_mut() {
                        state_mut.cache_credentials(
                            connection_id,
                            &creds.username,
                            &creds.password,
                            &creds.domain,
                        );
                    }
                }

                // Start RDP with credentials
                Self::start_rdp_session_with_credentials(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar_clone,
                    connection_id,
                    &creds.username,
                    &creds.password,
                    &creds.domain,
                );
            }
        });
    }

    /// Starts RDP session with provided credentials
    ///
    /// Starts an RDP session using xfreerdp with provided credentials
    ///
    /// Respects the `client_mode` setting in `RdpConfig`:
    /// - `Embedded`: Uses native RDP embedding with dynamic resolution (default)
    /// - `External`: Uses external xfreerdp window
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
        use rustconn_core::models::RdpClientMode;

        let state_ref = state.borrow();

        if let Some(conn) = state_ref.get_connection(connection_id) {
            let conn_name = conn.name.clone();
            let host = conn.host.clone();
            let port = conn.port;
            let window_mode = conn.window_mode;

            // Get RDP-specific options
            let rdp_config =
                if let rustconn_core::ProtocolConfig::Rdp(config) = &conn.protocol_config {
                    config.clone()
                } else {
                    rustconn_core::models::RdpConfig::default()
                };

            drop(state_ref);

            // Check client mode - if Embedded, use EmbeddedRdpWidget with fallback to external
            if rdp_config.client_mode == RdpClientMode::Embedded {
                use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};

                // Create embedded RDP widget
                let embedded_widget = EmbeddedRdpWidget::new();

                // Build embedded RDP config with dynamic resolution
                // Calculate initial resolution from saved window geometry
                // Content area = window_width - sidebar_width, window_height - toolbar/tabs
                let state_ref = state.borrow();
                let settings = state_ref.settings();
                let content_width = settings
                    .ui
                    .window_width
                    .unwrap_or(1200)
                    .saturating_sub(settings.ui.sidebar_width.unwrap_or(250));
                let content_height = settings.ui.window_height.unwrap_or(800).saturating_sub(100);
                drop(state_ref);

                #[allow(clippy::cast_sign_loss)]
                let initial_resolution = if content_width > 100 && content_height > 100 {
                    (content_width as u32, content_height as u32)
                } else {
                    // Fallback to config resolution or default
                    rdp_config
                        .resolution
                        .as_ref()
                        .map_or((1920, 1080), |r| (r.width, r.height))
                };

                let mut embedded_config = EmbeddedRdpConfig::new(&host)
                    .with_port(port)
                    .with_resolution(initial_resolution.0, initial_resolution.1)
                    .with_clipboard(true);

                if !username.is_empty() {
                    embedded_config = embedded_config.with_username(username);
                }
                if !password.is_empty() {
                    embedded_config = embedded_config.with_password(password);
                }
                if !domain.is_empty() {
                    embedded_config = embedded_config.with_domain(domain);
                }

                // Add extra args
                if !rdp_config.custom_args.is_empty() {
                    embedded_config =
                        embedded_config.with_extra_args(rdp_config.custom_args.clone());
                }

                // Add shared folders for drive redirection
                if !rdp_config.shared_folders.is_empty() {
                    use crate::embedded_rdp::EmbeddedSharedFolder;
                    let folders: Vec<EmbeddedSharedFolder> = rdp_config
                        .shared_folders
                        .iter()
                        .map(|f| EmbeddedSharedFolder {
                            local_path: f.local_path.clone(),
                            share_name: f.share_name.clone(),
                        })
                        .collect();
                    embedded_config = embedded_config.with_shared_folders(folders);
                }

                // Wrap in Rc to keep widget alive in notebook
                let embedded_widget = Rc::new(embedded_widget);

                // Connect using embedded widget (will fallback to external window if needed)
                if let Err(e) = embedded_widget.connect(&embedded_config) {
                    eprintln!("RDP connection failed for '{}': {}", conn_name, e);
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                } else {
                    sidebar.update_connection_status(&connection_id.to_string(), "connecting");
                }

                // Add embedded widget to notebook - shows connection status
                // If using external window, widget shows "Session running in external window"
                // The Rc<EmbeddedRdpWidget> is stored in notebook to keep it alive
                let session_id = Uuid::new_v4();

                // Connect state change callback to mark tab as disconnected when session ends
                let notebook_for_state = notebook.clone();
                let sidebar_for_state = sidebar.clone();
                embedded_widget.connect_state_changed(move |state| match state {
                    crate::embedded_rdp::RdpConnectionState::Disconnected => {
                        notebook_for_state.mark_tab_disconnected(session_id);
                        // If tab is still open, mark as failed (red) instead of disconnected (gray)
                        // This indicates an unexpected disconnection
                        if notebook_for_state.get_session_info(session_id).is_some() {
                            sidebar_for_state
                                .update_connection_status(&connection_id.to_string(), "failed");
                        } else {
                            sidebar_for_state.update_connection_status(
                                &connection_id.to_string(),
                                "disconnected",
                            );
                        }
                    }
                    crate::embedded_rdp::RdpConnectionState::Connected => {
                        notebook_for_state.mark_tab_connected(session_id);
                        sidebar_for_state
                            .update_connection_status(&connection_id.to_string(), "connected");
                    }
                    _ => {}
                });

                // Connect reconnect callback
                let widget_for_reconnect = embedded_widget.clone();
                embedded_widget.connect_reconnect(move || {
                    if let Err(e) = widget_for_reconnect.reconnect() {
                        eprintln!("RDP reconnect failed: {}", e);
                    }
                });

                notebook.add_embedded_rdp_tab(
                    session_id,
                    connection_id,
                    &conn_name,
                    embedded_widget,
                );

                // Show notebook for RDP session tab - hide split view, expand notebook
                split_view.widget().set_visible(false);
                split_view.widget().set_vexpand(false);
                notebook.widget().set_vexpand(true);
                notebook.notebook().set_vexpand(true);

                // If Fullscreen mode, maximize the window
                if matches!(window_mode, rustconn_core::models::WindowMode::Fullscreen) {
                    if let Some(window) = notebook
                        .widget()
                        .ancestor(gtk4::ApplicationWindow::static_type())
                    {
                        if let Some(app_window) = window.downcast_ref::<gtk4::ApplicationWindow>() {
                            app_window.maximize();
                        }
                    }
                }

                // Update last_connected timestamp
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let _ = state_mut.update_last_connected(connection_id);
                }
                return;
            }

            // External mode - use xfreerdp in external window
            let (tab, _is_embedded) = EmbeddedSessionTab::new(connection_id, &conn_name, "rdp");
            let session_id = tab.id();

            // Get resolution from RDP config
            let resolution = rdp_config.resolution.as_ref().map(|r| (r.width, r.height));

            // Get extra args from RDP config
            let extra_args = rdp_config.custom_args.clone();

            // Prepare domain (use None if empty)
            let domain_opt = if domain.is_empty() {
                None
            } else {
                Some(domain)
            };

            // Convert shared folders to (share_name, local_path) tuples for external RDP
            let shared_folders: Vec<(String, std::path::PathBuf)> = rdp_config
                .shared_folders
                .iter()
                .map(|f| (f.share_name.clone(), f.local_path.clone()))
                .collect();

            // Start RDP connection using xfreerdp with shared folders
            if let Err(e) = RdpLauncher::start_with_geometry(
                &tab,
                &host,
                port,
                Some(username),
                Some(password),
                domain_opt,
                resolution,
                &extra_args,
                None,  // window_geometry
                false, // remember_window_position
                &shared_folders,
            ) {
                eprintln!("Failed to start RDP session '{}': {}", conn_name, e);
                sidebar.update_connection_status(&connection_id.to_string(), "failed");
            } else {
                sidebar.update_connection_status(&connection_id.to_string(), "connected");
            }

            // Add tab widget to notebook
            notebook.add_embedded_session_tab(session_id, &conn_name, tab.widget());

            // For RDP/VNC sessions running in external windows, we add to split_view
            // but don't show them - they only show when user explicitly switches to the tab.
            // This prevents the placeholder from taking up space in split view.
            if let Some(info) = notebook.get_session_info(session_id) {
                split_view.add_session(info, None);
                // Don't call show_session for external sessions - let tab switch handle it
            }

            // Update last_connected
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                let _ = state_mut.update_last_connected(connection_id);
            }
        }
    }

    /// Starts a VNC connection with password dialog
    fn start_vnc_with_password_dialog(
        state: SharedAppState,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        connection_id: Uuid,
        window: &ApplicationWindow,
    ) {
        // Check if we can skip password dialog (verified credentials from KeePass)
        let can_skip = {
            let state_ref = state.borrow();
            state_ref.can_skip_password_dialog(connection_id)
        };

        if can_skip {
            // Try to resolve credentials from backends (KeePass)
            let state_ref = state.borrow();
            if let Ok(Some(creds)) = state_ref.resolve_credentials_for_connection(connection_id) {
                if let Some(password) = creds.expose_password() {
                    // Cache the password for use in start_connection
                    drop(state_ref);
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
        }

        // Get connection info for dialog
        let conn_name = {
            let state_ref = state.borrow();
            if let Some(conn) = state_ref.get_connection(connection_id) {
                conn.name.clone()
            } else {
                return;
            }
        };

        // Create and show password dialog
        let dialog = PasswordDialog::new(Some(window));
        dialog.set_connection_name(&conn_name);
        // VNC typically only needs password, hide username/domain by leaving them empty
        // The dialog will focus on password field

        // Try to load password from KeePass
        {
            use secrecy::ExposeSecret;
            let state_ref = state.borrow();
            let settings = state_ref.settings();

            if settings.secrets.kdbx_enabled {
                if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
                    let db_password = settings
                        .secrets
                        .kdbx_password
                        .as_ref()
                        .map(|p| p.expose_secret());
                    let key_file = settings.secrets.kdbx_key_file.as_deref();

                    eprintln!(
                        "DEBUG VNC: trying to load password for '{}' from KeePass",
                        conn_name
                    );

                    if let Ok(Some(password)) =
                        rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                            kdbx_path,
                            db_password,
                            key_file,
                            &conn_name,
                        )
                    {
                        eprintln!("DEBUG VNC: loaded password");
                        dialog.set_password(&password);
                    }
                }
            }
        }

        let sidebar_clone = sidebar.clone();
        dialog.show(move |result| {
            if let Some(creds) = result {
                // Cache credentials if requested (VNC only uses password)
                if creds.save_credentials {
                    if let Ok(mut state_mut) = state.try_borrow_mut() {
                        // For VNC, we store empty username/domain but save the password
                        state_mut.cache_credentials(connection_id, "", &creds.password, "");
                    }
                }

                // Start VNC with password
                Self::start_vnc_session_with_password(
                    &state,
                    &notebook,
                    &split_view,
                    &sidebar_clone,
                    connection_id,
                    &creds.password,
                );
            }
        });
    }

    /// Starts VNC session with provided password
    fn start_vnc_session_with_password(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
        password: &str,
    ) {
        let state_ref = state.borrow();

        if let Some(conn) = state_ref.get_connection(connection_id) {
            let conn_name = conn.name.clone();
            let host = conn.host.clone();
            let port = conn.port;

            // Get VNC-specific configuration
            let vnc_config =
                if let rustconn_core::ProtocolConfig::Vnc(config) = &conn.protocol_config {
                    config.clone()
                } else {
                    rustconn_core::models::VncConfig::default()
                };

            drop(state_ref);

            // Create VNC session tab with native widget
            let session_id = notebook.create_vnc_session_tab(connection_id, &conn_name);

            // Get the VNC widget and initiate connection with config
            if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
                // Connect state change callback to mark tab as disconnected when session ends
                let notebook_for_state = notebook.clone();
                let sidebar_for_state = sidebar.clone();
                vnc_widget.connect_state_changed(move |vnc_state| {
                    if vnc_state == crate::session::SessionState::Disconnected {
                        notebook_for_state.mark_tab_disconnected(session_id);
                        sidebar_for_state
                            .update_connection_status(&connection_id.to_string(), "disconnected");
                    } else if vnc_state == crate::session::SessionState::Connected {
                        notebook_for_state.mark_tab_connected(session_id);
                        sidebar_for_state
                            .update_connection_status(&connection_id.to_string(), "connected");
                    }
                });

                // Connect reconnect callback
                let widget_for_reconnect = vnc_widget.clone();
                vnc_widget.connect_reconnect(move || {
                    if let Err(e) = widget_for_reconnect.reconnect() {
                        eprintln!("VNC reconnect failed: {}", e);
                    }
                });

                // Initiate connection with VNC config (respects client_mode setting)
                if let Err(e) =
                    vnc_widget.connect_with_config(&host, port, Some(password), &vnc_config)
                {
                    eprintln!("Failed to connect VNC session '{}': {}", conn_name, e);
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                } else {
                    sidebar.update_connection_status(&connection_id.to_string(), "connecting");
                }
            }

            // VNC displays in notebook tab - hide split view and expand notebook
            split_view.widget().set_visible(false);
            split_view.widget().set_vexpand(false);
            notebook.widget().set_vexpand(true);
            notebook.notebook().set_vexpand(true);

            // Update last_connected timestamp
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                let _ = state_mut.update_last_connected(connection_id);
            }
        }
    }

    /// Starts a connection with split view integration
    fn start_connection_with_split(
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
                notebook.notebook().set_vexpand(true);
                return Some(session_id);
            }

            // For SSH, show terminal in split view
            // For external sessions, show placeholder
            let terminal = notebook.get_terminal(session_id);
            split_view.add_session(info.clone(), terminal.clone());
            let _ = split_view.show_session(session_id);

            // Ensure split view is visible and expanded for SSH
            split_view.widget().set_visible(true);
            split_view.widget().set_vexpand(true);
            notebook.widget().set_vexpand(false);
            notebook.notebook().set_vexpand(false);

            // For SSH and Zero Trust, we assume connected for now
            if info.protocol == "ssh" || info.protocol.starts_with("zerotrust") {
                // Set status to connecting initially
                sidebar.update_connection_status(&connection_id.to_string(), "connecting");

                // Monitor terminal content changes to detect successful connection
                // If content changes (e.g. prompt appears), mark as connected
                let sidebar_clone = sidebar.clone();
                let notebook_clone = notebook.clone();
                let connection_id_str = connection_id.to_string();

                notebook.connect_contents_changed(session_id, move || {
                    // Only update if currently "connecting"
                    if sidebar_clone.get_connection_status(&connection_id_str)
                        == Some("connecting".to_string())
                    {
                        // Check if content indicates actual output from the process
                        // The initial header is 2 lines. If we have more, it means the process produced output.
                        if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) {
                            if row > 2 {
                                sidebar_clone
                                    .update_connection_status(&connection_id_str, "connected");
                            }
                        }
                    }
                });
            }
        }

        Some(session_id)
    }

    /// Starts a connection and returns the `session_id`
    #[allow(clippy::too_many_lines)]
    fn start_connection(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
    ) -> Option<Uuid> {
        let state_ref = state.borrow();

        if let Some(conn) = state_ref.get_connection(connection_id) {
            let protocol = get_protocol_string(&conn.protocol_config);

            // Check if logging is enabled
            let logging_enabled = state_ref.settings().logging.enabled;
            let conn_name = conn.name.clone();

            if protocol == "ssh" {
                use rustconn_core::protocol::{format_command_message, format_connection_message};

                // Create terminal tab for SSH
                let session_id = notebook.create_terminal_tab(connection_id, &conn.name, &protocol);

                // Build and spawn SSH command
                let port = conn.port;
                let host = conn.host.clone();
                let username = conn.username.clone();

                // Get SSH-specific options
                let (identity_file, extra_args) =
                    if let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
                        let key = ssh_config
                            .key_path
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string());
                        let mut args = Vec::new();

                        if let Some(proxy) = &ssh_config.proxy_jump {
                            args.push("-J".to_string());
                            args.push(proxy.clone());
                        }

                        if ssh_config.use_control_master {
                            args.push("-o".to_string());
                            args.push("ControlMaster=auto".to_string());
                        }

                        for (k, v) in &ssh_config.custom_options {
                            args.push("-o".to_string());
                            args.push(format!("{k}={v}"));
                        }

                        (key, args)
                    } else {
                        (None, Vec::new())
                    };

                drop(state_ref);

                // Update last_connected timestamp
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let _ = state_mut.update_last_connected(connection_id);
                }

                // Set up session logging if enabled
                if logging_enabled {
                    Self::setup_session_logging(
                        state,
                        notebook,
                        session_id,
                        connection_id,
                        &conn_name,
                    );
                }

                // Wire up child exited callback for session cleanup
                Self::setup_child_exited_handler(
                    state,
                    notebook,
                    sidebar,
                    session_id,
                    connection_id,
                );

                // Build SSH command string for display
                let mut ssh_cmd_parts = vec!["ssh".to_string()];
                if port != 22 {
                    ssh_cmd_parts.push("-p".to_string());
                    ssh_cmd_parts.push(port.to_string());
                }
                if let Some(ref key) = identity_file {
                    ssh_cmd_parts.push("-i".to_string());
                    ssh_cmd_parts.push(key.clone());
                }
                ssh_cmd_parts.extend(extra_args.clone());
                let destination = if let Some(ref user) = username {
                    format!("{user}@{host}")
                } else {
                    host.clone()
                };
                ssh_cmd_parts.push(destination);
                let ssh_command = ssh_cmd_parts.join(" ");

                // Display CLI output feedback before executing command
                let conn_msg = format_connection_message("SSH", &host);
                let cmd_msg = format_command_message(&ssh_command);
                let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
                notebook.display_output(session_id, &feedback);

                // Spawn SSH
                let extra_refs: Vec<&str> =
                    extra_args.iter().map(std::string::String::as_str).collect();
                notebook.spawn_ssh(
                    session_id,
                    &host,
                    port,
                    username.as_deref(),
                    identity_file.as_deref(),
                    &extra_refs,
                );

                // Wire up child exited callback for session cleanup
                Self::setup_child_exited_handler(
                    state,
                    notebook,
                    sidebar,
                    session_id,
                    connection_id,
                );

                return Some(session_id);
            }

            // Handle VNC connections with native embedding or external client
            if protocol == "vnc" {
                let conn_name = conn.name.clone();
                let host = conn.host.clone();
                let port = conn.port;

                // Get VNC-specific configuration
                let vnc_config =
                    if let rustconn_core::ProtocolConfig::Vnc(config) = &conn.protocol_config {
                        config.clone()
                    } else {
                        rustconn_core::models::VncConfig::default()
                    };

                // Get password from cached credentials (set by credential resolution flow)
                let password: Option<String> =
                    state_ref.get_cached_credentials(connection_id).map(|c| {
                        use secrecy::ExposeSecret;
                        tracing::debug!("[VNC] Found cached credentials for connection");
                        c.password.expose_secret().to_string()
                    });

                tracing::debug!(
                    "[VNC] Password available: {}",
                    if password.is_some() { "yes" } else { "no" }
                );

                drop(state_ref);

                // Create VNC session tab with native widget
                let session_id = notebook.create_vnc_session_tab(connection_id, &conn_name);

                // Get the VNC widget and initiate connection with config
                if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
                    // Connect state change callback to mark tab as disconnected when session ends
                    let notebook_for_state = notebook.clone();
                    let sidebar_for_state = sidebar.clone();
                    vnc_widget.connect_state_changed(move |vnc_state| {
                        if vnc_state == crate::session::SessionState::Disconnected {
                            notebook_for_state.mark_tab_disconnected(session_id);
                            sidebar_for_state.update_connection_status(
                                &connection_id.to_string(),
                                "disconnected",
                            );
                        } else if vnc_state == crate::session::SessionState::Connected {
                            notebook_for_state.mark_tab_connected(session_id);
                            sidebar_for_state
                                .update_connection_status(&connection_id.to_string(), "connected");
                        }
                    });

                    // Connect reconnect callback
                    let widget_for_reconnect = vnc_widget.clone();
                    vnc_widget.connect_reconnect(move || {
                        if let Err(e) = widget_for_reconnect.reconnect() {
                            eprintln!("VNC reconnect failed: {}", e);
                        }
                    });

                    // Initiate connection with VNC config (respects client_mode setting)
                    if let Err(e) = vnc_widget.connect_with_config(
                        &host,
                        port,
                        password.as_deref(),
                        &vnc_config,
                    ) {
                        eprintln!("Failed to connect VNC session '{}': {}", conn_name, e);
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                    } else {
                        sidebar.update_connection_status(&connection_id.to_string(), "connecting");
                    }
                }

                // Update last_connected timestamp
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let _ = state_mut.update_last_connected(connection_id);
                }

                return Some(session_id);
            }

            // RDP connections are handled by start_rdp_session_with_credentials
            // which is called from start_connection_with_credential_resolution
            if protocol == "rdp" {
                // This branch should not be reached as RDP is handled earlier
                // with credential resolution. If we get here, fall through to
                // create a basic session without credentials.
                eprintln!(
                    "Warning: RDP connection reached start_connection without credentials. \
                     Use start_connection_with_credential_resolution instead."
                );
                return None;
            }

            // Handle SPICE connections with native embedding
            if protocol == "spice" {
                let conn_name = conn.name.clone();
                let host = conn.host.clone();
                let port = conn.port;

                // Get SPICE-specific options from connection config
                let spice_opts =
                    if let rustconn_core::ProtocolConfig::Spice(config) = &conn.protocol_config {
                        Some(config.clone())
                    } else {
                        None
                    };

                drop(state_ref);

                // Create SPICE session tab with native widget
                let session_id = notebook.create_spice_session_tab(connection_id, &conn_name);

                // Get the SPICE widget and initiate connection
                if let Some(spice_widget) = notebook.get_spice_widget(session_id) {
                    // Build connection config using SpiceClientConfig from spice_client module
                    use rustconn_core::SpiceClientConfig;
                    let mut config = SpiceClientConfig::new(&host).with_port(port);

                    // Apply SPICE-specific settings if available
                    if let Some(opts) = spice_opts {
                        // Configure TLS
                        config = config.with_tls(opts.tls_enabled);
                        if let Some(ca_path) = &opts.ca_cert_path {
                            config = config.with_ca_cert(ca_path);
                        }
                        config = config.with_skip_cert_verify(opts.skip_cert_verify);

                        // Configure USB redirection
                        config = config.with_usb_redirection(opts.usb_redirection);

                        // Configure clipboard
                        config = config.with_clipboard(opts.clipboard_enabled);
                    }

                    // Connect state change callback to mark tab as disconnected
                    let notebook_for_state = notebook.clone();
                    spice_widget.connect_state_changed(move |spice_state| {
                        use crate::embedded_spice::SpiceConnectionState;
                        if spice_state == SpiceConnectionState::Disconnected
                            || spice_state == SpiceConnectionState::Error
                        {
                            notebook_for_state.mark_tab_disconnected(session_id);
                        }
                    });

                    // Connect reconnect callback
                    let widget_for_reconnect = spice_widget.clone();
                    spice_widget.connect_reconnect(move || {
                        if let Err(e) = widget_for_reconnect.reconnect() {
                            eprintln!("SPICE reconnect failed: {}", e);
                        }
                    });

                    // Initiate connection
                    if let Err(e) = spice_widget.connect(&config) {
                        eprintln!("Failed to connect SPICE session '{conn_name}': {e}");
                    }
                }

                // Update last_connected timestamp
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let _ = state_mut.update_last_connected(connection_id);
                }

                return Some(session_id);
            }

            // Handle Zero Trust connections (protocol format: "zerotrust" or "zerotrust:provider")
            if protocol == "zerotrust" || protocol.starts_with("zerotrust:") {
                use rustconn_core::protocol::{format_command_message, format_connection_message};

                let conn_name = conn.name.clone();
                let username = conn.username.clone();

                // Get Zero Trust config and build command
                let (program, args, provider_name, provider_key) =
                    if let rustconn_core::ProtocolConfig::ZeroTrust(zt_config) =
                        &conn.protocol_config
                    {
                        let (prog, args) = zt_config.build_command(username.as_deref());
                        let provider = zt_config.provider.display_name();
                        // Get provider key for icon matching
                        let key = match zt_config.provider {
                            rustconn_core::models::ZeroTrustProvider::AwsSsm => "aws",
                            rustconn_core::models::ZeroTrustProvider::GcpIap => "gcloud",
                            rustconn_core::models::ZeroTrustProvider::AzureBastion => "azure",
                            rustconn_core::models::ZeroTrustProvider::AzureSsh => "azure_ssh",
                            rustconn_core::models::ZeroTrustProvider::OciBastion => "oci",
                            rustconn_core::models::ZeroTrustProvider::CloudflareAccess => {
                                "cloudflare"
                            }
                            rustconn_core::models::ZeroTrustProvider::Teleport => "teleport",
                            rustconn_core::models::ZeroTrustProvider::TailscaleSsh => "tailscale",
                            rustconn_core::models::ZeroTrustProvider::Boundary => "boundary",
                            rustconn_core::models::ZeroTrustProvider::Generic => "generic",
                        };
                        (prog, args, provider, key)
                    } else {
                        return None;
                    };

                drop(state_ref);

                // Create terminal tab for Zero Trust with provider-specific protocol
                let tab_protocol = format!("zerotrust:{provider_key}");
                let session_id =
                    notebook.create_terminal_tab(connection_id, &conn_name, &tab_protocol);

                // Update last_connected timestamp
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let _ = state_mut.update_last_connected(connection_id);
                }

                // Set up session logging if enabled
                if logging_enabled {
                    Self::setup_session_logging(
                        state,
                        notebook,
                        session_id,
                        connection_id,
                        &conn_name,
                    );
                }

                // Wire up child exited callback for session cleanup
                Self::setup_child_exited_handler(
                    state,
                    notebook,
                    sidebar,
                    session_id,
                    connection_id,
                );

                // Build the full command string for display
                let full_command = std::iter::once(program.as_str())
                    .chain(args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");

                // Display CLI output feedback before executing command
                let conn_msg = format_connection_message(provider_name, &conn_name);
                let cmd_msg = format_command_message(&full_command);
                let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
                notebook.display_output(session_id, &feedback);

                // Spawn the Zero Trust command through shell to use full PATH
                // This is needed because VTE doesn't see snap/flatpak paths
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
                notebook.spawn_command(session_id, &[&shell, "-c", &full_command], None, None);

                return Some(session_id);
            }

            // Unknown protocol - should not happen
            drop(state_ref);
            return None;
        }
        None
    }

    /// Sets up session logging for a terminal session
    fn setup_session_logging(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        session_id: Uuid,
        connection_id: Uuid,
        connection_name: &str,
    ) {
        // Get the log directory from settings
        let log_dir = if let Ok(state_ref) = state.try_borrow() {
            let settings = state_ref.settings();
            if settings.logging.log_directory.is_absolute() {
                settings.logging.log_directory.clone()
            } else {
                state_ref
                    .config_manager()
                    .config_dir()
                    .join(&settings.logging.log_directory)
            }
        } else {
            return;
        };

        // Ensure log directory exists
        if let Err(e) = std::fs::create_dir_all(&log_dir) {
            eprintln!(
                "Failed to create log directory '{}': {}",
                log_dir.display(),
                e
            );
            return;
        }

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

        // Create the log file and write header
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let header = format!(
                    "=== Session Log ===\nConnection: {}\nConnection ID: {}\nSession ID: {}\nStarted: {}\n\n",
                    connection_name,
                    connection_id,
                    session_id,
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                );
                if let Err(e) = file.write_all(header.as_bytes()) {
                    eprintln!("Failed to write log header: {}", e);
                    return;
                }

                eprintln!(
                    "Session logging enabled for '{}': {}",
                    connection_name,
                    log_path.display()
                );

                // Store log file path in session info
                notebook.set_log_file(session_id, log_path.clone());

                // Set up the contents changed handler to write terminal output
                Self::setup_contents_changed_handler(notebook, session_id, &log_path);
            }
            Err(e) => {
                eprintln!("Failed to create log file '{}': {}", log_path.display(), e);
            }
        }
    }

    /// Sets up the child exited handler for session cleanup
    fn setup_child_exited_handler(
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
            // Update session status in state manager
            // This also closes the session logger and finalizes the log file
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let _ = state_mut.terminate_session(session_id);
            }

            // Check if session still exists in notebook
            // If it doesn't, the tab was closed by user, so we should clear the status
            if notebook_clone.get_session_info(session_id).is_none() {
                sidebar_clone.update_connection_status(&connection_id_str, "disconnected");
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

            if is_failure {
                eprintln!("Session {session_id} exited with status: {exit_status} (Signal: {term_sig}, Code: {exit_code})");
                sidebar_clone.update_connection_status(&connection_id_str, "failed");
            } else {
                sidebar_clone.update_connection_status(&connection_id_str, "disconnected");
            }
        });
    }

    /// Sets up the contents changed handler for session logging
    ///
    /// Note: VTE4's Rust bindings have limited support for extracting terminal text.
    /// This handler logs activity timestamps when terminal content changes.
    /// For full session logging, consider using the script command or terminal recording.
    fn setup_contents_changed_handler(
        notebook: &SharedNotebook,
        session_id: Uuid,
        log_path: &std::path::Path,
    ) {
        use std::cell::RefCell;
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::rc::Rc;

        // Create a shared writer for the log file
        let log_writer: Rc<RefCell<Option<std::io::BufWriter<std::fs::File>>>> =
            Rc::new(RefCell::new(None));

        // Debounce counter to avoid excessive logging
        let change_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let last_log_time: Rc<RefCell<std::time::Instant>> =
            Rc::new(RefCell::new(std::time::Instant::now()));
        let flush_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

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

        let log_writer_clone = log_writer;
        let counter_clone = change_counter;
        let last_time_clone = last_log_time;
        let flush_counter_clone = flush_counter;

        notebook.connect_contents_changed(session_id, move || {
            // Increment change counter
            let mut counter = counter_clone.borrow_mut();
            *counter += 1;

            // Increment flush counter
            let mut flush_count = flush_counter_clone.borrow_mut();
            *flush_count += 1;

            // Debounce: only log every 100 changes or every 5 seconds
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(*last_time_clone.borrow());

            if *counter >= 100 || elapsed.as_secs() >= 5 {
                if let Ok(mut writer_opt) = log_writer_clone.try_borrow_mut() {
                    if let Some(ref mut writer) = *writer_opt {
                        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                        let _ = writeln!(
                            writer,
                            "[{}] Terminal activity ({} changes)",
                            timestamp, *counter
                        );

                        // Flush periodically (every 10 log entries or 30 seconds)
                        if *flush_count >= 10 || elapsed.as_secs() >= 30 {
                            let _ = writer.flush();
                            *flush_count = 0;
                        }
                    }
                }

                // Reset counter and time
                *counter = 0;
                *last_time_clone.borrow_mut() = now;
            }
        });
    }

    /// Shows the new connection dialog with optional template selection
    fn show_new_connection_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        // Check if there are templates available
        let templates = {
            let state_ref = state.borrow();
            state_ref.get_all_templates()
        };

        if templates.is_empty() {
            // No templates, show regular connection dialog
            Self::show_new_connection_dialog_internal(window, state, sidebar, None);
        } else {
            // Show template picker first
            Self::show_template_picker_for_new_connection(window, state, sidebar, templates);
        }
    }

    /// Shows a template picker dialog before creating a new connection
    fn show_template_picker_for_new_connection(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
        templates: Vec<rustconn_core::models::ConnectionTemplate>,
    ) {
        let picker_window = gtk4::Window::builder()
            .title("Create Connection")
            .transient_for(window)
            .modal(true)
            .default_width(400)
            .default_height(350)
            .build();

        // Create header bar
        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        header.pack_start(&cancel_btn);
        picker_window.set_titlebar(Some(&header));

        // Create content
        let content = gtk4::Box::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let title_label = Label::builder()
            .label("Choose how to create your connection:")
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        content.append(&title_label);

        // Blank connection option
        let blank_btn = Button::builder().label("Start from scratch").build();
        let blank_box = gtk4::Box::new(Orientation::Vertical, 4);
        blank_box.append(&blank_btn);
        let blank_desc = Label::builder()
            .label("Create a new connection with default settings")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        blank_box.append(&blank_desc);
        content.append(&blank_box);

        // Separator
        let separator = gtk4::Separator::new(Orientation::Horizontal);
        separator.set_margin_top(8);
        separator.set_margin_bottom(8);
        content.append(&separator);

        // Template section
        let template_label = Label::builder()
            .label("Or use a template:")
            .halign(gtk4::Align::Start)
            .build();
        content.append(&template_label);

        // Templates list
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let templates_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();

        for template in &templates {
            let hbox = gtk4::Box::new(Orientation::Horizontal, 8);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);
            hbox.set_margin_start(8);
            hbox.set_margin_end(8);

            // Protocol icon
            let icon_name = match template.protocol {
                rustconn_core::models::ProtocolType::Ssh => "utilities-terminal-symbolic",
                rustconn_core::models::ProtocolType::Rdp => "computer-symbolic",
                rustconn_core::models::ProtocolType::Vnc => "video-display-symbolic",
                rustconn_core::models::ProtocolType::Spice => "video-display-symbolic",
                rustconn_core::models::ProtocolType::ZeroTrust => "cloud-symbolic",
            };
            let icon = gtk4::Image::from_icon_name(icon_name);
            hbox.append(&icon);

            // Template info
            let info_box = gtk4::Box::new(Orientation::Vertical, 2);
            info_box.set_hexpand(true);

            let name_label = Label::builder()
                .label(&template.name)
                .halign(gtk4::Align::Start)
                .build();
            info_box.append(&name_label);

            if let Some(ref desc) = template.description {
                let desc_label = Label::builder()
                    .label(desc)
                    .halign(gtk4::Align::Start)
                    .css_classes(["dim-label"])
                    .build();
                info_box.append(&desc_label);
            }

            hbox.append(&info_box);

            let row = gtk4::ListBoxRow::builder().child(&hbox).build();
            row.set_widget_name(&format!("template-{}", template.id));
            templates_list.append(&row);
        }

        scrolled.set_child(Some(&templates_list));
        content.append(&scrolled);

        // Use template button
        let use_template_btn = Button::builder()
            .label("Use Selected Template")
            .sensitive(false)
            .css_classes(["suggested-action"])
            .build();
        content.append(&use_template_btn);

        picker_window.set_child(Some(&content));

        // Connect selection changed
        let use_btn_clone = use_template_btn.clone();
        templates_list.connect_row_selected(move |_, row| {
            use_btn_clone.set_sensitive(row.is_some());
        });

        // Connect cancel button
        let picker_clone = picker_window.clone();
        cancel_btn.connect_clicked(move |_| {
            picker_clone.close();
        });

        // Connect blank button
        let picker_clone = picker_window.clone();
        let window_clone = window.clone();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        blank_btn.connect_clicked(move |_| {
            picker_clone.close();
            Self::show_new_connection_dialog_internal(
                &window_clone,
                state_clone.clone(),
                sidebar_clone.clone(),
                None,
            );
        });

        // Connect use template button
        let picker_clone = picker_window.clone();
        let window_clone = window.clone();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let templates_clone = templates.clone();
        let list_clone = templates_list.clone();
        use_template_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        if let Some(template) = templates_clone.iter().find(|t| t.id == id) {
                            picker_clone.close();
                            Self::show_new_connection_dialog_internal(
                                &window_clone,
                                state_clone.clone(),
                                sidebar_clone.clone(),
                                Some(template.clone()),
                            );
                        }
                    }
                }
            }
        });

        // Double-click on template row
        let picker_clone = picker_window.clone();
        let window_clone = window.clone();
        let state_clone = state;
        let sidebar_clone = sidebar;
        let templates_clone = templates;
        templates_list.connect_row_activated(move |_, row| {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("template-") {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    if let Some(template) = templates_clone.iter().find(|t| t.id == id) {
                        picker_clone.close();
                        Self::show_new_connection_dialog_internal(
                            &window_clone,
                            state_clone.clone(),
                            sidebar_clone.clone(),
                            Some(template.clone()),
                        );
                    }
                }
            }
        });

        picker_window.present();
    }

    /// Internal function to show the new connection dialog with optional template
    fn show_new_connection_dialog_internal(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
        template: Option<rustconn_core::models::ConnectionTemplate>,
    ) {
        let dialog = ConnectionDialog::new(Some(&window.clone().upcast()));
        dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

        // Set KeePass enabled state from settings
        {
            let state_ref = state.borrow();
            let keepass_enabled = state_ref.settings().secrets.kdbx_enabled;
            dialog.set_keepass_enabled(keepass_enabled);
        }

        // If template provided, pre-populate the dialog
        if let Some(ref tmpl) = template {
            let connection = tmpl.apply(None);
            dialog.set_connection(&connection);
            dialog
                .window()
                .set_title(Some("New Connection from Template"));
        }

        // Connect save to KeePass callback
        let window_for_keepass = window.clone();
        let state_for_save = state.clone();
        dialog.connect_save_to_keepass(move |name, host, username, password, protocol| {
            use secrecy::ExposeSecret;

            let state_ref = state_for_save.borrow();
            let settings = state_ref.settings();

            if !settings.secrets.kdbx_enabled {
                let alert = gtk4::AlertDialog::builder()
                    .message("KeePass Not Enabled")
                    .detail("Please enable KeePass integration in Settings first.")
                    .modal(true)
                    .build();
                alert.show(Some(&window_for_keepass));
                return;
            }

            let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() else {
                let alert = gtk4::AlertDialog::builder()
                    .message("KeePass Database Not Configured")
                    .detail("Please select a KeePass database file in Settings.")
                    .modal(true)
                    .build();
                alert.show(Some(&window_for_keepass));
                return;
            };

            let lookup_key = if name.trim().is_empty() {
                host.to_string()
            } else {
                name.to_string()
            };

            // Get credentials - password and key file can be used together
            let db_password = settings
                .secrets
                .kdbx_password
                .as_ref()
                .map(|p| p.expose_secret());

            // Key file is optional additional authentication
            let key_file = settings.secrets.kdbx_key_file.as_deref();

            // Debug: check what credentials we have
            eprintln!(
                "DEBUG save_to_keepass: has_db_password={}, has_key_file={}, kdbx_path={}",
                db_password.is_some(),
                key_file.is_some(),
                kdbx_path.display()
            );

            // Check if we have at least one credential
            if db_password.is_none() && key_file.is_none() {
                let alert = gtk4::AlertDialog::builder()
                    .message("KeePass Credentials Required")
                    .detail("Please enter the database password or select a key file in Settings.")
                    .modal(true)
                    .build();
                alert.show(Some(&window_for_keepass));
                return;
            }

            // Build URL for the entry with correct protocol
            let url = format!("{}://{}", protocol, host);

            // Save to KeePass
            match rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                kdbx_path,
                db_password,
                key_file,
                &lookup_key,
                username,
                password,
                Some(&url),
            ) {
                Ok(()) => {
                    let alert = gtk4::AlertDialog::builder()
                        .message("Password Saved")
                        .detail(format!("Password for '{lookup_key}' saved to KeePass."))
                        .modal(true)
                        .build();
                    alert.show(Some(&window_for_keepass));
                }
                Err(e) => {
                    let alert = gtk4::AlertDialog::builder()
                        .message("Failed to Save Password")
                        .detail(format!("Error: {e}"))
                        .modal(true)
                        .build();
                    alert.show(Some(&window_for_keepass));
                }
            }
        });

        // Connect load from KeePass callback
        let state_for_load = state.clone();
        dialog.connect_load_from_keepass(move |name, host, _protocol| {
            use secrecy::ExposeSecret;

            let state_ref = state_for_load.borrow();
            let settings = state_ref.settings();

            if !settings.secrets.kdbx_enabled {
                return None;
            }

            let kdbx_path = settings.secrets.kdbx_path.as_ref()?;

            let lookup_key = if name.trim().is_empty() {
                host.to_string()
            } else {
                name.to_string()
            };

            // Get credentials - password and key file can be used together
            let db_password = settings
                .secrets
                .kdbx_password
                .as_ref()
                .map(|p| p.expose_secret());

            // Key file is optional additional authentication
            let key_file = settings.secrets.kdbx_key_file.as_deref();

            eprintln!(
                "DEBUG load_from_keepass: lookup_key='{}', has_password={}, has_key_file={}",
                lookup_key,
                db_password.is_some(),
                key_file.is_some()
            );

            match rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                kdbx_path,
                db_password,
                key_file,
                &lookup_key,
            ) {
                Ok(password) => password,
                Err(e) => {
                    eprintln!("Failed to load password from KeePass: {e}");
                    None
                }
            }
        });

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(conn) = result {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    match state_mut.create_connection(conn) {
                        Ok(_) => {
                            // Reload sidebar preserving tree state
                            drop(state_mut);
                            Self::reload_sidebar_preserving_state(&state, &sidebar);
                        }
                        Err(e) => {
                            // Show error in UI dialog with proper transient parent
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Creating Connection")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        });
    }

    /// Shows the new group dialog with optional parent selection
    fn show_new_group_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        Self::show_new_group_dialog_with_parent(window, state, sidebar, None);
    }

    /// Shows the new group dialog with parent group selection
    // SharedAppState is Rc<RefCell<...>> - cheap to clone and needed for closure ownership
    #[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
    fn show_new_group_dialog_with_parent(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
        preselected_parent: Option<Uuid>,
    ) {
        // Using Window instead of deprecated Dialog
        let entry = gtk4::Entry::new();
        entry.set_placeholder_text(Some("Group name"));

        let group_window = gtk4::Window::builder()
            .title("New Group")
            .transient_for(window)
            .modal(true)
            .default_width(350)
            .build();

        // Create header bar with Cancel/Create buttons
        let header = gtk4::HeaderBar::new();
        let cancel_btn = gtk4::Button::builder().label("Cancel").build();
        let create_btn = gtk4::Button::builder()
            .label("Create")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&create_btn);
        group_window.set_titlebar(Some(&header));

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Group name
        let name_label = Label::new(Some("Group name:"));
        name_label.set_halign(gtk4::Align::Start);
        content.append(&name_label);
        content.append(&entry);

        // Parent group dropdown
        let parent_label = Label::new(Some("Parent group (optional):"));
        parent_label.set_halign(gtk4::Align::Start);
        parent_label.set_margin_top(8);
        content.append(&parent_label);

        let parent_dropdown = gtk4::DropDown::from_strings(&["(None - Root Level)"]);

        // Populate parent dropdown with existing groups
        let state_ref = state.borrow();
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();
        drop(state_ref);

        let mut group_ids: Vec<Option<Uuid>> = vec![None];
        let mut strings: Vec<String> = vec!["(None - Root Level)".to_string()];
        let mut preselected_index = 0u32;

        for group in &groups {
            let state_ref = state.borrow();
            let path = state_ref
                .get_group_path(group.id)
                .unwrap_or_else(|| group.name.clone());
            drop(state_ref);

            strings.push(path);
            group_ids.push(Some(group.id));

            if preselected_parent == Some(group.id) {
                #[allow(clippy::cast_possible_truncation)]
                {
                    preselected_index = (group_ids.len() - 1) as u32;
                }
            }
        }

        let string_list = gtk4::StringList::new(
            &strings
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        parent_dropdown.set_model(Some(&string_list));
        parent_dropdown.set_selected(preselected_index);

        content.append(&parent_dropdown);
        group_window.set_child(Some(&content));

        // Connect cancel button
        let window_clone = group_window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect create button
        let state_clone = state.clone();
        let sidebar_clone = sidebar;
        let window_clone = group_window.clone();
        let entry_clone = entry;
        let dropdown_clone = parent_dropdown;
        create_btn.connect_clicked(move |_| {
            let name = entry_clone.text().to_string();
            if name.trim().is_empty() {
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail("Group name cannot be empty")
                    .modal(true)
                    .build();
                alert.show(Some(&window_clone));
                return;
            }

            let selected_idx = dropdown_clone.selected() as usize;
            let parent_id = if selected_idx < group_ids.len() {
                group_ids[selected_idx]
            } else {
                None
            };

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let result = if let Some(pid) = parent_id {
                    state_mut.create_group_with_parent(name, pid)
                } else {
                    state_mut.create_group(name)
                };

                match result {
                    Ok(_) => {
                        drop(state_mut);
                        Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        window_clone.close();
                    }
                    Err(e) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error")
                            .detail(&e)
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                    }
                }
            }
        });

        group_window.present();
    }

    /// Shows the import dialog
    fn show_import_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        let dialog = ImportDialog::new(Some(&window.clone().upcast()));

        let window_clone = window.clone();
        dialog.run_with_source(move |result, source_name| {
            if let Some(import_result) = result {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    match state_mut.import_connections_with_source(&import_result, &source_name) {
                        Ok(count) => {
                            drop(state_mut);
                            Self::reload_sidebar_preserving_state(&state, &sidebar);
                            // Show success message with proper transient parent
                            let alert = gtk4::AlertDialog::builder()
                                .message("Import Successful")
                                .detail(format!(
                                    "Imported {count} connections to '{source_name}' group"
                                ))
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Import Failed")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        });
    }

    /// Shows the settings dialog
    fn show_settings_dialog(window: &ApplicationWindow, state: SharedAppState) {
        let dialog = SettingsDialog::new(Some(&window.clone().upcast()));

        // Load current settings
        {
            let state_ref = state.borrow();
            dialog.set_settings(state_ref.settings());
        }

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(settings) = result {
                // Capture KeePass state for action update
                let keepass_enabled = settings.secrets.kdbx_enabled;
                let kdbx_path_exists = settings
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p| p.exists());

                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    if let Err(e) = state_mut.update_settings(settings) {
                        eprintln!("Failed to save settings: {e}");
                    } else {
                        // Update open-keepass action enabled state
                        if let Some(action) = window_clone.lookup_action("open-keepass") {
                            if let Some(simple_action) = action.downcast_ref::<gio::SimpleAction>()
                            {
                                simple_action.set_enabled(keepass_enabled && kdbx_path_exists);
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
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Get selected item using sidebar's method (works in both single and multi-selection modes)
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        let id_str = conn_item.id();
        let Ok(id) = Uuid::parse_str(&id_str) else {
            return;
        };

        if conn_item.is_group() {
            // Edit group - show simple rename dialog
            Self::show_edit_group_dialog(window, state.clone(), sidebar.clone(), id);
        } else {
            // Edit connection
            let state_ref = state.borrow();
            let Some(conn) = state_ref.get_connection(id).cloned() else {
                return;
            };
            drop(state_ref);

            let dialog = ConnectionDialog::new(Some(&window.clone().upcast()));
            dialog.setup_key_file_chooser(Some(&window.clone().upcast()));
            dialog.set_connection(&conn);

            // Set KeePass enabled state from settings
            {
                let state_ref = state.borrow();
                let keepass_enabled = state_ref.settings().secrets.kdbx_enabled;
                dialog.set_keepass_enabled(keepass_enabled);
            }

            // Connect save to KeePass callback
            let window_for_keepass = window.clone();
            let state_for_save = state.clone();
            let conn_name = conn.name.clone();
            let conn_host = conn.host.clone();
            dialog.connect_save_to_keepass(move |name, host, username, password, protocol| {
                use secrecy::ExposeSecret;

                let state_ref = state_for_save.borrow();
                let settings = state_ref.settings();

                if !settings.secrets.kdbx_enabled {
                    let alert = gtk4::AlertDialog::builder()
                        .message("KeePass Not Enabled")
                        .detail("Please enable KeePass integration in Settings first.")
                        .modal(true)
                        .build();
                    alert.show(Some(&window_for_keepass));
                    return;
                }

                let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() else {
                    let alert = gtk4::AlertDialog::builder()
                        .message("KeePass Database Not Configured")
                        .detail("Please select a KeePass database file in Settings.")
                        .modal(true)
                        .build();
                    alert.show(Some(&window_for_keepass));
                    return;
                };

                // Use connection name/host for lookup key
                let lookup_key = if !name.trim().is_empty() {
                    name.to_string()
                } else if !host.trim().is_empty() {
                    host.to_string()
                } else if !conn_name.is_empty() {
                    conn_name.clone()
                } else {
                    conn_host.clone()
                };

                // Get credentials - password and key file can be used together
                let db_password = settings
                    .secrets
                    .kdbx_password
                    .as_ref()
                    .map(|p| p.expose_secret());

                // Key file is optional additional authentication
                let key_file = settings.secrets.kdbx_key_file.as_deref();

                // Debug: check what credentials we have
                eprintln!(
                    "DEBUG save_to_keepass (edit): has_db_password={}, has_key_file={}, kdbx_path={}",
                    db_password.is_some(),
                    key_file.is_some(),
                    kdbx_path.display()
                );

                // Check if we have at least one credential
                if db_password.is_none() && key_file.is_none() {
                    let alert = gtk4::AlertDialog::builder()
                        .message("KeePass Credentials Required")
                        .detail("Please enter the database password or select a key file in Settings.")
                        .modal(true)
                        .build();
                    alert.show(Some(&window_for_keepass));
                    return;
                }

                // Use protocol from callback parameter
                let url = format!("{}://{}", protocol, if host.is_empty() { &conn_host } else { host });

                match rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx_path,
                    db_password,
                    key_file,
                    &lookup_key,
                    username,
                    password,
                    Some(&url),
                ) {
                    Ok(()) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Password Saved")
                            .detail(format!("Password for '{lookup_key}' saved to KeePass."))
                            .modal(true)
                            .build();
                        alert.show(Some(&window_for_keepass));
                    }
                    Err(e) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Failed to Save Password")
                            .detail(format!("Error: {e}"))
                            .modal(true)
                            .build();
                        alert.show(Some(&window_for_keepass));
                    }
                }
            });

            // Connect load from KeePass callback
            let state_for_load = state.clone();
            dialog.connect_load_from_keepass(move |name, host, _protocol| {
                use secrecy::ExposeSecret;

                let state_ref = state_for_load.borrow();
                let settings = state_ref.settings();

                if !settings.secrets.kdbx_enabled {
                    return None;
                }

                let kdbx_path = settings.secrets.kdbx_path.as_ref()?;

                let lookup_key = if name.trim().is_empty() {
                    host.to_string()
                } else {
                    name.to_string()
                };

                // Get credentials - password and key file can be used together
                let db_password = settings
                    .secrets
                    .kdbx_password
                    .as_ref()
                    .map(|p| p.expose_secret());

                // Key file is optional additional authentication
                let key_file = settings.secrets.kdbx_key_file.as_deref();

                eprintln!(
                    "DEBUG load_from_keepass (edit): lookup_key='{}', has_password={}, has_key_file={}",
                    lookup_key,
                    db_password.is_some(),
                    key_file.is_some()
                );

                match rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                    kdbx_path,
                    db_password,
                    key_file,
                    &lookup_key,
                ) {
                    Ok(password) => password,
                    Err(e) => {
                        eprintln!("Failed to load password from KeePass: {e}");
                        None
                    }
                }
            });

            let state_clone = state.clone();
            let sidebar_clone = sidebar.clone();
            let window_clone = window.clone();
            dialog.run(move |result| {
                if let Some(updated_conn) = result {
                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        match state_mut.update_connection(id, updated_conn) {
                            Ok(()) => {
                                drop(state_mut);
                                // Preserve tree state when editing connections
                                Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                            }
                            Err(e) => {
                                let alert = gtk4::AlertDialog::builder()
                                    .message("Error Updating Connection")
                                    .detail(&e)
                                    .modal(true)
                                    .build();
                                alert.show(Some(&window_clone));
                            }
                        }
                    }
                }
            });
        }
    }

    /// Shows connection details in a popover
    fn show_connection_details(
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
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
        let Ok(id) = Uuid::parse_str(&id_str) else {
            return;
        };

        let state_ref = state.borrow();
        let Some(conn) = state_ref.get_connection(id).cloned() else {
            return;
        };
        drop(state_ref);

        // Create details popover
        let popover = gtk4::Popover::new();
        popover.set_autohide(true);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_width_request(300);

        // Connection name header
        let name_label = Label::builder()
            .label(&conn.name)
            .css_classes(["title-2"])
            .halign(gtk4::Align::Start)
            .build();
        content.append(&name_label);

        // Basic info
        let info_grid = gtk4::Grid::builder()
            .row_spacing(4)
            .column_spacing(8)
            .build();

        let mut row = 0;

        // Protocol
        let protocol_label = Label::builder()
            .label("Protocol:")
            .halign(gtk4::Align::End)
            .css_classes(["dim-label"])
            .build();
        let protocol_value = Label::builder()
            .label(&format!("{:?}", conn.protocol))
            .halign(gtk4::Align::Start)
            .build();
        info_grid.attach(&protocol_label, 0, row, 1, 1);
        info_grid.attach(&protocol_value, 1, row, 1, 1);
        row += 1;

        // Host
        let host_label = Label::builder()
            .label("Host:")
            .halign(gtk4::Align::End)
            .css_classes(["dim-label"])
            .build();
        let host_value = Label::builder()
            .label(&format!("{}:{}", conn.host, conn.port))
            .halign(gtk4::Align::Start)
            .selectable(true)
            .build();
        info_grid.attach(&host_label, 0, row, 1, 1);
        info_grid.attach(&host_value, 1, row, 1, 1);
        row += 1;

        // Username if present
        if let Some(ref username) = conn.username {
            let user_label = Label::builder()
                .label("Username:")
                .halign(gtk4::Align::End)
                .css_classes(["dim-label"])
                .build();
            let user_value = Label::builder()
                .label(username)
                .halign(gtk4::Align::Start)
                .selectable(true)
                .build();
            info_grid.attach(&user_label, 0, row, 1, 1);
            info_grid.attach(&user_value, 1, row, 1, 1);
            row += 1;
        }

        // Tags if present
        if !conn.tags.is_empty() {
            let tags_label = Label::builder()
                .label("Tags:")
                .halign(gtk4::Align::End)
                .css_classes(["dim-label"])
                .build();
            let tags_value = Label::builder()
                .label(&conn.tags.join(", "))
                .halign(gtk4::Align::Start)
                .wrap(true)
                .build();
            info_grid.attach(&tags_label, 0, row, 1, 1);
            info_grid.attach(&tags_value, 1, row, 1, 1);
        }

        content.append(&info_grid);

        // Custom properties section
        if !conn.custom_properties.is_empty() {
            let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            separator.set_margin_top(8);
            separator.set_margin_bottom(8);
            content.append(&separator);

            let props_header = Label::builder()
                .label("Custom Properties")
                .css_classes(["title-4"])
                .halign(gtk4::Align::Start)
                .build();
            content.append(&props_header);

            let props_grid = gtk4::Grid::builder()
                .row_spacing(4)
                .column_spacing(8)
                .build();

            for (idx, prop) in conn.custom_properties.iter().enumerate() {
                let prop_name = Label::builder()
                    .label(&format!("{}:", prop.name))
                    .halign(gtk4::Align::End)
                    .css_classes(["dim-label"])
                    .build();

                #[allow(clippy::cast_possible_truncation)]
                let row_idx = idx as i32;

                if prop.is_protected() {
                    // Show masked value for protected properties
                    let prop_value = Label::builder()
                        .label("••••••••")
                        .halign(gtk4::Align::Start)
                        .build();
                    props_grid.attach(&prop_name, 0, row_idx, 1, 1);
                    props_grid.attach(&prop_value, 1, row_idx, 1, 1);
                } else if prop.is_url() {
                    // Create clickable link for URL properties
                    let link_button = gtk4::LinkButton::builder()
                        .uri(&prop.value)
                        .label(&prop.value)
                        .halign(gtk4::Align::Start)
                        .build();
                    props_grid.attach(&prop_name, 0, row_idx, 1, 1);
                    props_grid.attach(&link_button, 1, row_idx, 1, 1);
                } else {
                    // Regular text property
                    let prop_value = Label::builder()
                        .label(&prop.value)
                        .halign(gtk4::Align::Start)
                        .selectable(true)
                        .wrap(true)
                        .build();
                    props_grid.attach(&prop_name, 0, row_idx, 1, 1);
                    props_grid.attach(&prop_value, 1, row_idx, 1, 1);
                }
            }

            content.append(&props_grid);
        }

        popover.set_child(Some(&content));
        popover.set_parent(window);

        // Position the popover near the sidebar
        popover.set_position(gtk4::PositionType::Right);

        // Connect to closed signal to unparent
        popover.connect_closed(|p| {
            p.unparent();
        });

        popover.popup();
    }

    /// Shows dialog to edit a group name
    // SharedAppState is Rc<RefCell<...>> - cheap to clone and needed for closure ownership
    #[allow(clippy::needless_pass_by_value)]
    fn show_edit_group_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
        group_id: Uuid,
    ) {
        let state_ref = state.borrow();
        let Some(group) = state_ref.get_group(group_id).cloned() else {
            return;
        };
        drop(state_ref);

        let entry = gtk4::Entry::new();
        entry.set_text(&group.name);

        let group_window = gtk4::Window::builder()
            .title("Edit Group")
            .transient_for(window)
            .modal(true)
            .default_width(300)
            .build();

        let header = gtk4::HeaderBar::new();
        let cancel_btn = gtk4::Button::builder().label("Cancel").build();
        let save_btn = gtk4::Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&save_btn);
        group_window.set_titlebar(Some(&header));

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let label = Label::new(Some("Group name:"));
        content.append(&label);
        content.append(&entry);
        group_window.set_child(Some(&content));

        let window_clone = group_window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        let state_clone = state.clone();
        let sidebar_clone = sidebar;
        let window_clone = group_window.clone();
        let entry_clone = entry;
        let old_name = group.name;
        save_btn.connect_clicked(move |_| {
            let new_name = entry_clone.text().to_string();
            if new_name.trim().is_empty() {
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail("Group name cannot be empty")
                    .modal(true)
                    .build();
                alert.show(Some(&window_clone));
                return;
            }

            // Check for duplicate name (but allow keeping same name)
            if new_name != old_name {
                let state_ref = state_clone.borrow();
                if state_ref.group_exists_by_name(&new_name) {
                    drop(state_ref);
                    let alert = gtk4::AlertDialog::builder()
                        .message("Validation Error")
                        .detail(format!("Group with name '{new_name}' already exists"))
                        .modal(true)
                        .build();
                    alert.show(Some(&window_clone));
                    return;
                }
                drop(state_ref);
            }

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                if let Some(existing) = state_mut.get_group(group_id).cloned() {
                    let mut updated = existing;
                    updated.name = new_name;
                    if let Err(e) = state_mut
                        .connection_manager()
                        .update_group(group_id, updated)
                    {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error")
                            .detail(format!("{e}"))
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                        return;
                    }
                }
                drop(state_mut);
                // Preserve tree state when editing groups
                Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                window_clone.close();
            }
        });

        group_window.present();
    }

    /// Deletes the selected connection or group
    fn delete_selected_connection(
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Get selected item using sidebar's method (works in both single and multi-selection modes)
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        let id_str = conn_item.id();
        let Ok(id) = Uuid::parse_str(&id_str) else {
            return;
        };
        let name = conn_item.name();
        let is_group = conn_item.is_group();

        // Show confirmation dialog with connection count for groups
        let item_type = if is_group { "group" } else { "connection" };
        let detail = if is_group {
            let state_ref = state.borrow();
            let connection_count = state_ref.count_connections_in_group(id);
            drop(state_ref);

            if connection_count > 0 {
                format!(
                    "Are you sure you want to delete the group '{name}'?\n\nThis will also delete {connection_count} connection(s) in this group."
                )
            } else {
                format!("Are you sure you want to delete the empty group '{name}'?")
            }
        } else {
            format!("Are you sure you want to delete the connection '{name}'?")
        };

        let alert = gtk4::AlertDialog::builder()
            .message(format!("Delete {item_type}?"))
            .detail(&detail)
            .buttons(["Cancel", "Delete"])
            .default_button(0)
            .cancel_button(0)
            .modal(true)
            .build();

        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_clone = window.clone();
        alert.choose(Some(window), gio::Cancellable::NONE, move |result| {
            if result == Ok(1) {
                // "Delete" button
                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    let delete_result = if is_group {
                        // Use cascade delete to remove group and all its connections
                        state_mut.delete_group_cascade(id)
                    } else {
                        state_mut.delete_connection(id)
                    };

                    match delete_result {
                        Ok(()) => {
                            drop(state_mut);
                            // Preserve tree state when deleting (scroll position, other expanded groups)
                            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        }
                        Err(e) => {
                            let error_alert = gtk4::AlertDialog::builder()
                                .message("Error Deleting")
                                .detail(&e)
                                .modal(true)
                                .build();
                            error_alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        });
    }

    /// Deletes all selected connections (bulk delete for group operations mode)
    #[allow(clippy::too_many_lines)]
    fn delete_selected_connections(
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        let selected_ids = sidebar.get_selected_ids();

        if selected_ids.is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("No Selection")
                .detail("Please select one or more items to delete.")
                .modal(true)
                .build();
            alert.show(Some(window));
            return;
        }

        // Build list of items to delete for confirmation
        let state_ref = state.borrow();
        let mut item_names: Vec<String> = Vec::new();
        let mut connection_count = 0;
        let mut group_count = 0;

        for id in &selected_ids {
            if let Some(conn) = state_ref.get_connection(*id) {
                item_names.push(format!("• {} (connection)", conn.name));
                connection_count += 1;
            } else if let Some(group) = state_ref.get_group(*id) {
                item_names.push(format!("• {} (group)", group.name));
                group_count += 1;
            }
        }
        drop(state_ref);

        let summary = match (connection_count, group_count) {
            (c, 0) => format!("{c} connection(s)"),
            (0, g) => format!("{g} group(s)"),
            (c, g) => format!("{c} connection(s) and {g} group(s)"),
        };

        // Create custom dialog with scrolling for large lists
        let dialog = gtk4::Window::builder()
            .title("Delete Selected Items?")
            .transient_for(window)
            .modal(true)
            .default_width(400)
            .default_height(if item_names.len() > 10 { 400 } else { 250 })
            .build();

        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let delete_btn = Button::builder()
            .label("Delete All")
            .css_classes(["destructive-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&delete_btn);
        dialog.set_titlebar(Some(&header));

        let content = gtk4::Box::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Summary label
        let summary_label = Label::builder()
            .label(format!("Are you sure you want to delete {summary}?"))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        content.append(&summary_label);

        // Scrolled list of items
        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(100)
            .max_content_height(250)
            .vexpand(true)
            .build();

        let items_label = Label::builder()
            .label(item_names.join("\n"))
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Start)
            .wrap(true)
            .selectable(true)
            .build();
        scrolled.set_child(Some(&items_label));
        content.append(&scrolled);

        // Warning label
        let warning_label = Label::builder()
            .label("Connections in deleted groups will become ungrouped.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        content.append(&warning_label);

        dialog.set_child(Some(&content));

        // Connect cancel button
        let dialog_weak = dialog.downgrade();
        cancel_btn.connect_clicked(move |_| {
            if let Some(d) = dialog_weak.upgrade() {
                d.close();
            }
        });

        // Connect delete button
        let dialog_weak = dialog.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_clone = window.clone();
        delete_btn.connect_clicked(move |_| {
            if let Some(d) = dialog_weak.upgrade() {
                d.close();
            }

            let mut success_count = 0;
            let mut failures: Vec<String> = Vec::new();

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                for id in &selected_ids {
                    // Try to delete as connection first, then as group
                    let delete_result = state_mut
                        .delete_connection(*id)
                        .or_else(|_| state_mut.delete_group(*id));

                    match delete_result {
                        Ok(()) => success_count += 1,
                        Err(e) => failures.push(format!("{id}: {e}")),
                    }
                }
            }

            // Reload sidebar preserving state
            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);

            // Show results
            if failures.is_empty() {
                let success_alert = gtk4::AlertDialog::builder()
                    .message("Deletion Complete")
                    .detail(format!("Successfully deleted {success_count} item(s)."))
                    .modal(true)
                    .build();
                success_alert.show(Some(&window_clone));
            } else {
                let error_alert = gtk4::AlertDialog::builder()
                    .message("Deletion Partially Complete")
                    .detail(format!(
                        "Deleted {} item(s).\n\nFailed to delete {} item(s):\n{}",
                        success_count,
                        failures.len(),
                        failures.join("\n")
                    ))
                    .modal(true)
                    .build();
                error_alert.show(Some(&window_clone));
            }
        });

        dialog.present();
    }

    /// Shows dialog to move selected items to a group
    fn show_move_selected_to_group_dialog(
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        let selected_ids = sidebar.get_selected_ids();

        if selected_ids.is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("No Selection")
                .detail("Please select one or more connections to move.")
                .modal(true)
                .build();
            alert.show(Some(window));
            return;
        }

        // Filter to only connections (not groups)
        let state_ref = state.borrow();
        let connection_ids: Vec<Uuid> = selected_ids
            .iter()
            .filter(|id| state_ref.get_connection(**id).is_some())
            .copied()
            .collect();
        drop(state_ref);

        if connection_ids.is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("No Connections Selected")
                .detail("Only connections can be moved to groups. Please select at least one connection.")
                .modal(true)
                .build();
            alert.show(Some(window));
            return;
        }

        // Build group selection dialog
        let state_ref = state.borrow();
        let groups = state_ref.list_groups();
        let mut group_names: Vec<String> = vec!["(No Group)".to_string()];
        let mut group_ids: Vec<Option<Uuid>> = vec![None];

        for group in groups {
            group_names.push(group.name.clone());
            group_ids.push(Some(group.id));
        }
        drop(state_ref);

        let alert = gtk4::AlertDialog::builder()
            .message("Move to Group")
            .detail(format!(
                "Select a group for {} connection(s):",
                connection_ids.len()
            ))
            .buttons(group_names)
            .default_button(0)
            .cancel_button(-1)
            .modal(true)
            .build();

        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_clone = window.clone();
        alert.choose(Some(window), gio::Cancellable::NONE, move |result| {
            if let Ok(choice) = result {
                #[allow(clippy::cast_sign_loss)]
                let choice_idx = choice as usize;
                if choice_idx < group_ids.len() {
                    let target_group = group_ids[choice_idx];
                    let mut success_count = 0;
                    let mut failures: Vec<String> = Vec::new();

                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        for conn_id in &connection_ids {
                            match state_mut.move_connection_to_group(*conn_id, target_group) {
                                Ok(()) => success_count += 1,
                                Err(e) => failures.push(format!("{conn_id}: {e}")),
                            }
                        }
                    }

                    // Reload sidebar preserving state
                    Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);

                    // Show results if there were failures
                    if !failures.is_empty() {
                        let error_alert = gtk4::AlertDialog::builder()
                            .message("Move Partially Complete")
                            .detail(format!(
                                "Moved {} connection(s).\n\nFailed to move {} connection(s):\n{}",
                                success_count,
                                failures.len(),
                                failures.join("\n")
                            ))
                            .modal(true)
                            .build();
                        error_alert.show(Some(&window_clone));
                    }
                }
            }
        });
    }

    /// Duplicates the selected connection
    fn duplicate_selected_connection(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Get selected item using sidebar's method (works in both single and multi-selection modes)
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Can only duplicate connections, not groups
        if conn_item.is_group() {
            return;
        }

        let id_str = conn_item.id();
        let Ok(id) = Uuid::parse_str(&id_str) else {
            return;
        };

        let state_ref = state.borrow();
        let Some(conn) = state_ref.get_connection(id).cloned() else {
            return;
        };

        // Generate unique name for duplicate
        let new_name = state_ref
            .generate_unique_connection_name(&format!("{} (copy)", conn.name), conn.protocol);
        drop(state_ref);

        // Create duplicate with new ID and name
        let mut duplicate = conn;
        duplicate.id = Uuid::new_v4();
        duplicate.name = new_name;
        duplicate.created_at = chrono::Utc::now();
        duplicate.updated_at = chrono::Utc::now();

        if let Ok(mut state_mut) = state.try_borrow_mut() {
            match state_mut
                .connection_manager()
                .create_connection_from(duplicate)
            {
                Ok(_) => {
                    drop(state_mut);
                    // Preserve tree state when duplicating
                    Self::reload_sidebar_preserving_state(state, sidebar);
                }
                Err(e) => {
                    eprintln!("Failed to duplicate connection: {e}");
                }
            }
        }
    }

    /// Copies the selected connection to the internal clipboard
    fn copy_selected_connection(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Get selected item using sidebar's method
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Can only copy connections, not groups
        if conn_item.is_group() {
            return;
        }

        let id_str = conn_item.id();
        let Ok(id) = Uuid::parse_str(&id_str) else {
            return;
        };

        if let Ok(mut state_mut) = state.try_borrow_mut() {
            if let Err(e) = state_mut.copy_connection(id) {
                eprintln!("Failed to copy connection: {e}");
            }
        }
    }

    /// Pastes a connection from the internal clipboard
    fn paste_connection(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Check if clipboard has content
        {
            let state_ref = state.borrow();
            if !state_ref.has_clipboard_content() {
                return;
            }
        }

        if let Ok(mut state_mut) = state.try_borrow_mut() {
            match state_mut.paste_connection() {
                Ok(_) => {
                    drop(state_mut);
                    // Preserve tree state when pasting
                    Self::reload_sidebar_preserving_state(state, sidebar);
                }
                Err(e) => {
                    eprintln!("Failed to paste connection: {e}");
                }
            }
        }
    }

    /// Reloads the sidebar with current data (preserving hierarchy)
    fn reload_sidebar(state: &SharedAppState, sidebar: &SharedSidebar) {
        let store = sidebar.store();
        store.remove_all();

        let state_ref = state.borrow();

        // Add root groups with their children
        for group in state_ref.get_root_groups() {
            let group_item = ConnectionItem::new_group(&group.id.to_string(), &group.name);
            Self::add_group_children_static(&state_ref, sidebar, &group_item, group.id);
            store.append(&group_item);
        }

        // Add ungrouped connections
        for conn in state_ref.get_ungrouped_connections() {
            let protocol = get_protocol_string(&conn.protocol_config);
            let status = sidebar
                .get_connection_status(&conn.id.to_string())
                .unwrap_or_else(|| "disconnected".to_string());
            let item = ConnectionItem::new_connection_with_status(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
                &status,
            );
            store.append(&item);
        }
    }

    /// Recursively adds group children (static version for reload)
    fn add_group_children_static(
        state: &std::cell::Ref<crate::state::AppState>,
        sidebar: &SharedSidebar,
        parent_item: &ConnectionItem,
        group_id: Uuid,
    ) {
        // Add child groups
        for child_group in state.get_child_groups(group_id) {
            let child_item =
                ConnectionItem::new_group(&child_group.id.to_string(), &child_group.name);
            Self::add_group_children_static(state, sidebar, &child_item, child_group.id);
            parent_item.add_child(&child_item);
        }

        // Add connections in this group
        for conn in state.get_connections_by_group(group_id) {
            let protocol = get_protocol_string(&conn.protocol_config);
            let status = sidebar
                .get_connection_status(&conn.id.to_string())
                .unwrap_or_else(|| "disconnected".to_string());
            let item = ConnectionItem::new_connection_with_status(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
                &status,
            );
            parent_item.add_child(&item);
        }
    }

    /// Reloads the sidebar while preserving tree state
    ///
    /// This method saves the current expanded groups, scroll position, and selection,
    /// reloads the sidebar data, and then restores the state. Use this when editing
    /// connections to maintain the user's view.
    fn reload_sidebar_preserving_state(state: &SharedAppState, sidebar: &SharedSidebar) {
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
    pub const fn gtk_window(&self) -> &ApplicationWindow {
        &self.window
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
    #[must_use]
    pub fn sidebar(&self) -> &ConnectionSidebar {
        &self.sidebar
    }

    /// Returns a clone of the shared sidebar Rc
    #[must_use]
    pub fn sidebar_rc(&self) -> Rc<ConnectionSidebar> {
        self.sidebar.clone()
    }

    /// Returns a reference to the terminal notebook
    #[must_use]
    pub fn terminal_notebook(&self) -> &TerminalNotebook {
        &self.terminal_notebook
    }

    /// Saves the current expanded groups state to settings
    pub fn save_expanded_groups(&self) {
        let expanded = self.sidebar.get_expanded_groups();
        if let Ok(mut state) = self.state.try_borrow_mut() {
            let _ = state.update_expanded_groups(expanded);
        }
    }

    /// Opens a local shell terminal with split view integration
    fn open_local_shell_with_split(notebook: &SharedNotebook, split_view: &SharedSplitView) {
        let session_id = notebook.create_terminal_tab(Uuid::nil(), "Local Shell", "local");

        // Get user's default shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        notebook.spawn_command(session_id, &[&shell], None, None);

        // Explicitly show the session in split view
        // This is needed because switch_page signal may fire before session data is ready
        if let Some(info) = notebook.get_session_info(session_id) {
            let terminal = notebook.get_terminal(session_id);
            split_view.add_session(info, terminal);
            let _ = split_view.show_session(session_id);
        }

        // Ensure split view is visible and expanded for local shell
        split_view.widget().set_visible(true);
        split_view.widget().set_vexpand(true);
        notebook.widget().set_vexpand(false);
        notebook.notebook().set_vexpand(false);

        // Note: The switch_page signal handler will automatically show the session
        // in the split view when the notebook switches to the new tab
    }

    /// Shows the quick connect dialog with protocol selection
    #[allow(clippy::too_many_lines)]
    fn show_quick_connect_dialog(window: &ApplicationWindow, notebook: SharedNotebook) {
        // Create a quick connect window
        let quick_window = gtk4::Window::builder()
            .title("Quick Connect")
            .transient_for(window)
            .modal(true)
            .default_width(550)
            .default_height(550)
            .build();

        // Create header bar with Cancel/Quick Connect buttons
        let header = gtk4::HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let connect_btn = Button::builder()
            .label("Quick Connect")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&connect_btn);
        quick_window.set_titlebar(Some(&header));

        // Create content
        let content = gtk4::Box::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Info label
        let info_label = Label::new(Some("⚠ This connection will not be saved"));
        info_label.add_css_class("dim-label");
        content.append(&info_label);

        // Basic fields
        let grid = gtk4::Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .build();

        // Protocol dropdown (SSH, RDP, VNC)
        let protocol_label = Label::builder()
            .label("Protocol:")
            .halign(gtk4::Align::End)
            .build();
        let protocol_list = gtk4::StringList::new(&["SSH", "RDP", "VNC"]);
        let protocol_dropdown = gtk4::DropDown::builder().model(&protocol_list).build();
        protocol_dropdown.set_selected(0); // Default to SSH
        grid.attach(&protocol_label, 0, 0, 1, 1);
        grid.attach(&protocol_dropdown, 1, 0, 2, 1);

        let host_label = Label::builder()
            .label("Host:")
            .halign(gtk4::Align::End)
            .build();
        let host_entry = gtk4::Entry::builder()
            .hexpand(true)
            .placeholder_text("hostname or IP")
            .build();
        grid.attach(&host_label, 0, 1, 1, 1);
        grid.attach(&host_entry, 1, 1, 2, 1);

        let port_label = Label::builder()
            .label("Port:")
            .halign(gtk4::Align::End)
            .build();
        let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_spin = gtk4::SpinButton::builder()
            .adjustment(&port_adj)
            .climb_rate(1.0)
            .digits(0)
            .build();
        grid.attach(&port_label, 0, 2, 1, 1);
        grid.attach(&port_spin, 1, 2, 1, 1);

        let user_label = Label::builder()
            .label("Username:")
            .halign(gtk4::Align::End)
            .build();
        let user_entry = gtk4::Entry::builder()
            .hexpand(true)
            .placeholder_text("(optional)")
            .build();
        grid.attach(&user_label, 0, 3, 1, 1);
        grid.attach(&user_entry, 1, 3, 2, 1);

        content.append(&grid);
        quick_window.set_child(Some(&content));

        // Track if port was manually changed
        let port_manually_changed = Rc::new(RefCell::new(false));

        // Connect port spin value-changed to track manual changes
        let port_manually_changed_clone = port_manually_changed.clone();
        port_spin.connect_value_changed(move |_| {
            *port_manually_changed_clone.borrow_mut() = true;
        });

        // Connect protocol change to port update
        let port_spin_clone = port_spin.clone();
        let port_manually_changed_clone = port_manually_changed;
        protocol_dropdown.connect_selected_notify(move |dropdown| {
            // Only update port if it wasn't manually changed
            if !*port_manually_changed_clone.borrow() {
                let default_port = match dropdown.selected() {
                    1 => 3389.0, // RDP
                    2 => 5900.0, // VNC
                    _ => 22.0,   // SSH (0) and any other value
                };
                port_spin_clone.set_value(default_port);
            }
            // Reset the flag after protocol change so next protocol change updates port
            *port_manually_changed_clone.borrow_mut() = false;
        });

        // Connect cancel
        let window_clone = quick_window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect quick connect button
        let window_clone = quick_window.clone();
        let host_clone = host_entry;
        let port_clone = port_spin;
        let user_clone = user_entry;
        let protocol_clone = protocol_dropdown;
        connect_btn.connect_clicked(move |_| {
            let host = host_clone.text().to_string();
            if host.trim().is_empty() {
                return;
            }

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let port = port_clone.value() as u16;
            let username = {
                let text = user_clone.text();
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text.to_string())
                }
            };

            let protocol_idx = protocol_clone.selected();

            match protocol_idx {
                0 => {
                    // SSH - use terminal tab
                    let session_id =
                        notebook.create_terminal_tab(Uuid::nil(), &format!("Quick: {host}"), "ssh");

                    notebook.spawn_ssh(session_id, &host, port, username.as_deref(), None, &[]);
                }
                1 => {
                    // RDP - native embedding not yet implemented, show placeholder
                    let _session_id =
                        notebook.create_terminal_tab(Uuid::nil(), &format!("Quick: {host}"), "rdp");

                    eprintln!(
                        "Note: Native RDP embedding not yet implemented. \
                         Quick connect to '{host}' shown as placeholder."
                    );
                }
                2 => {
                    // VNC - native embedding not yet implemented, show placeholder
                    let _session_id =
                        notebook.create_terminal_tab(Uuid::nil(), &format!("Quick: {host}"), "vnc");

                    eprintln!(
                        "Note: Native VNC embedding not yet implemented. \
                         Quick connect to '{host}' shown as placeholder."
                    );
                }
                _ => {
                    // Default to SSH
                    let session_id =
                        notebook.create_terminal_tab(Uuid::nil(), &format!("Quick: {host}"), "ssh");

                    notebook.spawn_ssh(session_id, &host, port, username.as_deref(), None, &[]);
                }
            }

            window_clone.close();
        });

        quick_window.present();
    }

    /// Toggles group operations mode for multi-select
    fn toggle_group_operations_mode(sidebar: &SharedSidebar, enabled: bool) {
        sidebar.set_group_operations_mode(enabled);
    }

    /// Sorts connections alphabetically and updates `sort_order`
    ///
    /// If a group is selected, only sorts connections within that group.
    /// Otherwise, sorts all groups and connections globally.
    fn sort_connections(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Check if a group is selected
        let selected_group_id = sidebar.get_selected_item().and_then(|item| {
            if item.is_group() {
                Uuid::parse_str(&item.id()).ok()
            } else {
                None
            }
        });

        // Perform the appropriate sort operation
        if let Some(group_id) = selected_group_id {
            // Sort only the selected group
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                if let Err(e) = state_mut.sort_group(group_id) {
                    eprintln!("Failed to sort group: {e}");
                }
            }
        } else {
            // Sort all groups and connections
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                if let Err(e) = state_mut.sort_all() {
                    eprintln!("Failed to sort all: {e}");
                }
            }
        }

        // Rebuild the sidebar to reflect the new sort order
        Self::rebuild_sidebar_sorted(state, sidebar);
    }

    /// Sorts connections by recent usage (most recently used first)
    fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
        // Sort all connections by last_connected timestamp
        if let Ok(mut state_mut) = state.try_borrow_mut() {
            if let Err(e) = state_mut.sort_by_recent() {
                eprintln!("Failed to sort by recent: {e}");
            }
        }

        // Rebuild the sidebar to reflect the new sort order
        Self::rebuild_sidebar_sorted(state, sidebar);
    }

    /// Rebuilds the sidebar with sorted items
    fn rebuild_sidebar_sorted(state: &SharedAppState, sidebar: &SharedSidebar) {
        let store = sidebar.store();
        let state_ref = state.borrow();

        // Get and sort groups by sort_order, then by name
        let mut groups: Vec<_> = state_ref
            .get_root_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();
        groups.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
            std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            other => other,
        });

        // Get and sort ungrouped connections by sort_order, then by name
        let mut ungrouped: Vec<_> = state_ref
            .get_ungrouped_connections()
            .iter()
            .map(|c| (*c).clone())
            .collect();
        ungrouped.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
            std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            other => other,
        });

        drop(state_ref);

        // Rebuild store with sorted items (groups first, then ungrouped)
        store.remove_all();

        let state_ref = state.borrow();

        // Add sorted groups with their sorted children
        for group in &groups {
            let group_item = ConnectionItem::new_group(&group.id.to_string(), &group.name);
            Self::add_sorted_group_children(&state_ref, &group_item, group.id);
            store.append(&group_item);
        }

        // Add sorted ungrouped connections
        for conn in &ungrouped {
            let protocol = get_protocol_string(&conn.protocol_config);
            let item = ConnectionItem::new_connection(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
            );
            store.append(&item);
        }
    }

    /// Recursively adds sorted group children
    fn add_sorted_group_children(
        state: &std::cell::Ref<crate::state::AppState>,
        parent_item: &ConnectionItem,
        group_id: Uuid,
    ) {
        // Get and sort child groups by sort_order, then by name
        let mut child_groups: Vec<_> = state
            .get_child_groups(group_id)
            .iter()
            .map(|g| (*g).clone())
            .collect();
        child_groups.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
            std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            other => other,
        });

        for child_group in &child_groups {
            let child_item =
                ConnectionItem::new_group(&child_group.id.to_string(), &child_group.name);
            Self::add_sorted_group_children(state, &child_item, child_group.id);
            parent_item.add_child(&child_item);
        }

        // Get and sort connections in this group by sort_order, then by name
        let mut connections: Vec<_> = state
            .get_connections_by_group(group_id)
            .iter()
            .map(|c| (*c).clone())
            .collect();
        connections.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
            std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            other => other,
        });

        for conn in &connections {
            let protocol = get_protocol_string(&conn.protocol_config);
            let item = ConnectionItem::new_connection(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
            );
            parent_item.add_child(&item);
        }
    }

    /// Handles drag-drop operations for reordering connections
    ///
    /// Data format: "`item_type:item_id:target_id:target_is_group`"
    fn handle_drag_drop(state: &SharedAppState, sidebar: &SharedSidebar, data: &str) {
        let parts: Vec<&str> = data.split(':').collect();
        if parts.len() != 4 {
            return;
        }

        let item_type = parts[0];
        let item_id = parts[1];
        let target_id = parts[2];
        let target_is_group = parts[3] == "true";

        // Parse UUIDs
        let Ok(item_uuid) = Uuid::parse_str(item_id) else {
            return;
        };
        let Ok(target_uuid) = Uuid::parse_str(target_id) else {
            return;
        };

        // Handle based on item type
        match item_type {
            "conn" => {
                // Moving a connection
                if target_is_group {
                    // Move connection to the target group
                    if let Ok(mut state_mut) = state.try_borrow_mut() {
                        if let Err(e) =
                            state_mut.move_connection_to_group(item_uuid, Some(target_uuid))
                        {
                            eprintln!("Failed to move connection to group: {e}");
                            return;
                        }
                    }
                } else {
                    // Reorder connection relative to target connection
                    // Get the target connection's group and position
                    let target_group_id = {
                        let state_ref = state.borrow();
                        state_ref
                            .get_connection(target_uuid)
                            .and_then(|c| c.group_id)
                    };

                    if let Ok(mut state_mut) = state.try_borrow_mut() {
                        // First move to the same group as target
                        if let Err(e) =
                            state_mut.move_connection_to_group(item_uuid, target_group_id)
                        {
                            eprintln!("Failed to move connection: {e}");
                            return;
                        }

                        // Then reorder within the group
                        if let Err(e) = state_mut.reorder_connection(item_uuid, target_uuid) {
                            eprintln!("Failed to reorder connection: {e}");
                            return;
                        }
                    }
                }
            }
            "group" => {
                // Moving a group - reorder among groups
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    if let Err(e) = state_mut.reorder_group(item_uuid, target_uuid) {
                        eprintln!("Failed to reorder group: {e}");
                        return;
                    }
                }
            }
            _ => return,
        }

        // Save tree state before rebuild
        let tree_state = sidebar.save_state();

        // Rebuild sidebar to reflect changes
        Self::rebuild_sidebar_sorted(state, sidebar);

        // Restore tree state after rebuild
        sidebar.restore_state(&tree_state);
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
    fn show_export_dialog(window: &ApplicationWindow, state: SharedAppState) {
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
                let alert = gtk4::AlertDialog::builder()
                    .message("Export Complete")
                    .detail(format!(
                        "Successfully exported {} connection(s).\n{} skipped.",
                        export_result.exported_count, export_result.skipped_count
                    ))
                    .modal(true)
                    .build();
                alert.show(Some(&window_clone));
            }
        });
    }

    // ========== Snippet Management Methods ==========

    /// Shows the new snippet dialog
    fn show_new_snippet_dialog(window: &ApplicationWindow, state: SharedAppState) {
        let dialog = SnippetDialog::new(Some(&window.clone().upcast()));

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(snippet) = result {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    match state_mut.create_snippet(snippet) {
                        Ok(_) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Snippet Created")
                                .detail("Snippet has been saved successfully.")
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Creating Snippet")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        });
    }

    /// Shows the snippets manager window
    #[allow(clippy::too_many_lines)]
    fn show_snippets_manager(
        window: &ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
    ) {
        let manager_window = gtk4::Window::builder()
            .title("Manage Snippets")
            .transient_for(window)
            .modal(true)
            .default_width(600)
            .default_height(500)
            .build();

        // Create header bar
        let header = HeaderBar::new();
        let close_btn = Button::builder().label("Close").build();
        let new_btn = Button::builder()
            .label("New Snippet")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&new_btn);
        manager_window.set_titlebar(Some(&header));

        // Create main content
        let content = gtk4::Box::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Search entry
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search snippets..."));
        content.append(&search_entry);

        // Snippets list
        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let snippets_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&snippets_list));
        content.append(&scrolled);

        // Action buttons
        let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);

        let edit_btn = Button::builder().label("Edit").sensitive(false).build();
        let delete_btn = Button::builder().label("Delete").sensitive(false).build();
        let execute_btn = Button::builder()
            .label("Execute")
            .sensitive(false)
            .css_classes(["suggested-action"])
            .build();

        button_box.append(&edit_btn);
        button_box.append(&delete_btn);
        button_box.append(&execute_btn);
        content.append(&button_box);

        manager_window.set_child(Some(&content));

        // Populate snippets list
        Self::populate_snippets_list(&state, &snippets_list, "");

        // Connect search
        let state_clone = state.clone();
        let list_clone = snippets_list.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            Self::populate_snippets_list(&state_clone, &list_clone, &query);
        });

        // Connect selection changed
        let edit_clone = edit_btn.clone();
        let delete_clone = delete_btn.clone();
        let execute_clone = execute_btn.clone();
        snippets_list.connect_row_selected(move |_, row| {
            let has_selection = row.is_some();
            edit_clone.set_sensitive(has_selection);
            delete_clone.set_sensitive(has_selection);
            execute_clone.set_sensitive(has_selection);
        });

        // Connect close button
        let window_clone = manager_window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect new button
        let state_clone = state.clone();
        let list_clone = snippets_list.clone();
        let manager_clone = manager_window.clone();
        new_btn.connect_clicked(move |_| {
            let dialog = SnippetDialog::new(Some(&manager_clone.clone().upcast()));
            let state_inner = state_clone.clone();
            let list_inner = list_clone.clone();
            dialog.run(move |result| {
                if let Some(snippet) = result {
                    if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                        let _ = state_mut.create_snippet(snippet);
                        drop(state_mut);
                        Self::populate_snippets_list(&state_inner, &list_inner, "");
                    }
                }
            });
        });

        // Connect edit button
        let state_clone = state.clone();
        let list_clone = snippets_list.clone();
        let manager_clone = manager_window.clone();
        edit_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        let state_ref = state_clone.borrow();
                        if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                            drop(state_ref);
                            let dialog = SnippetDialog::new(Some(&manager_clone.clone().upcast()));
                            dialog.set_snippet(&snippet);
                            let state_inner = state_clone.clone();
                            let list_inner = list_clone.clone();
                            dialog.run(move |result| {
                                if let Some(updated) = result {
                                    if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                                        let _ = state_mut.update_snippet(id, updated);
                                        drop(state_mut);
                                        Self::populate_snippets_list(&state_inner, &list_inner, "");
                                    }
                                }
                            });
                        }
                    }
                }
            }
        });

        // Connect delete button
        let state_clone = state.clone();
        let list_clone = snippets_list.clone();
        let manager_clone = manager_window.clone();
        delete_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Delete Snippet?")
                            .detail("Are you sure you want to delete this snippet?")
                            .buttons(["Cancel", "Delete"])
                            .default_button(0)
                            .cancel_button(0)
                            .modal(true)
                            .build();

                        let state_inner = state_clone.clone();
                        let list_inner = list_clone.clone();
                        alert.choose(
                            Some(&manager_clone),
                            gio::Cancellable::NONE,
                            move |result| {
                                if result == Ok(1) {
                                    if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                                        let _ = state_mut.delete_snippet(id);
                                        drop(state_mut);
                                        Self::populate_snippets_list(&state_inner, &list_inner, "");
                                    }
                                }
                            },
                        );
                    }
                }
            }
        });

        // Connect execute button
        let state_clone = state;
        let list_clone = snippets_list;
        let notebook_clone = notebook;
        let manager_clone = manager_window.clone();
        execute_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        let state_ref = state_clone.borrow();
                        if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                            drop(state_ref);
                            Self::execute_snippet(&manager_clone, &notebook_clone, &snippet);
                        }
                    }
                }
            }
        });

        manager_window.present();
    }

    /// Populates the snippets list with filtered results
    fn populate_snippets_list(state: &SharedAppState, list: &gtk4::ListBox, query: &str) {
        // Clear existing rows
        while let Some(row) = list.row_at_index(0) {
            list.remove(&row);
        }

        let state_ref = state.borrow();
        let snippets = if query.is_empty() {
            state_ref.list_snippets()
        } else {
            state_ref.search_snippets(query)
        };

        for snippet in snippets {
            let row = gtk4::ListBoxRow::new();
            row.set_widget_name(&format!("snippet-{}", snippet.id));

            let hbox = gtk4::Box::new(Orientation::Horizontal, 12);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);

            let vbox = gtk4::Box::new(Orientation::Vertical, 4);
            vbox.set_hexpand(true);

            let name_label = Label::builder()
                .label(&snippet.name)
                .halign(gtk4::Align::Start)
                .css_classes(["heading"])
                .build();
            vbox.append(&name_label);

            let cmd_preview = if snippet.command.len() > 50 {
                format!("{}...", &snippet.command[..50])
            } else {
                snippet.command.clone()
            };
            let cmd_label = Label::builder()
                .label(&cmd_preview)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label", "monospace"])
                .build();
            vbox.append(&cmd_label);

            if let Some(ref cat) = snippet.category {
                let cat_label = Label::builder()
                    .label(cat)
                    .halign(gtk4::Align::Start)
                    .css_classes(["dim-label"])
                    .build();
                vbox.append(&cat_label);
            }

            hbox.append(&vbox);
            row.set_child(Some(&hbox));
            list.append(&row);
        }
    }

    /// Shows a snippet picker for quick execution
    fn show_snippet_picker(
        window: &ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
    ) {
        let picker_window = gtk4::Window::builder()
            .title("Execute Snippet")
            .transient_for(window)
            .modal(true)
            .default_width(400)
            .default_height(400)
            .build();

        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        header.pack_start(&cancel_btn);
        picker_window.set_titlebar(Some(&header));

        let content = gtk4::Box::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search snippets..."));
        content.append(&search_entry);

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let snippets_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&snippets_list));
        content.append(&scrolled);

        picker_window.set_child(Some(&content));

        Self::populate_snippets_list(&state, &snippets_list, "");

        // Connect search
        let state_clone = state.clone();
        let list_clone = snippets_list.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            Self::populate_snippets_list(&state_clone, &list_clone, &query);
        });

        // Connect cancel
        let window_clone = picker_window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect row activation (double-click or Enter)
        let state_clone = state;
        let notebook_clone = notebook;
        let window_clone = picker_window.clone();
        snippets_list.connect_row_activated(move |_, row| {
            if let Some(id_str) = row.widget_name().as_str().strip_prefix("snippet-") {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    let state_ref = state_clone.borrow();
                    if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                        drop(state_ref);
                        Self::execute_snippet(&window_clone, &notebook_clone, &snippet);
                        window_clone.close();
                    }
                }
            }
        });

        picker_window.present();
    }

    /// Executes a snippet in the active terminal
    fn execute_snippet(
        parent: &impl IsA<gtk4::Window>,
        notebook: &SharedNotebook,
        snippet: &rustconn_core::Snippet,
    ) {
        // Check if there's an active terminal
        if notebook.get_active_terminal().is_none() {
            let alert = gtk4::AlertDialog::builder()
                .message("No Active Terminal")
                .detail("Please open a terminal session first before executing a snippet.")
                .modal(true)
                .build();
            alert.show(Some(parent));
            return;
        }

        // Check if snippet has variables that need values
        let variables = rustconn_core::SnippetManager::extract_variables(&snippet.command);

        if variables.is_empty() {
            // No variables, execute directly
            notebook.send_text(&format!("{}\n", snippet.command));
        } else {
            // Show variable input dialog
            Self::show_variable_input_dialog(parent, notebook, snippet);
        }
    }

    /// Shows a dialog to input variable values for a snippet
    fn show_variable_input_dialog(
        parent: &impl IsA<gtk4::Window>,
        notebook: &SharedNotebook,
        snippet: &rustconn_core::Snippet,
    ) {
        let var_window = gtk4::Window::builder()
            .title("Enter Variable Values")
            .transient_for(parent)
            .modal(true)
            .default_width(400)
            .build();

        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let execute_btn = Button::builder()
            .label("Execute")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&execute_btn);
        var_window.set_titlebar(Some(&header));

        let content = gtk4::Box::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let grid = gtk4::Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .build();

        let mut entries: Vec<(String, gtk4::Entry)> = Vec::new();
        let variables = rustconn_core::SnippetManager::extract_variables(&snippet.command);

        for (i, var_name) in variables.iter().enumerate() {
            let label = Label::builder()
                .label(format!("{var_name}:"))
                .halign(gtk4::Align::End)
                .build();

            let entry = gtk4::Entry::builder().hexpand(true).build();

            // Set default value if available
            if let Some(var_def) = snippet.variables.iter().find(|v| &v.name == var_name) {
                if let Some(ref default) = var_def.default_value {
                    entry.set_text(default);
                }
                if let Some(ref desc) = var_def.description {
                    entry.set_placeholder_text(Some(desc));
                }
            }

            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let row_idx = i as i32;
            grid.attach(&label, 0, row_idx, 1, 1);
            grid.attach(&entry, 1, row_idx, 1, 1);
            entries.push((var_name.clone(), entry));
        }

        content.append(&grid);
        var_window.set_child(Some(&content));

        // Connect cancel
        let window_clone = var_window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect execute
        let window_clone = var_window.clone();
        let notebook_clone = notebook.clone();
        let command = snippet.command.clone();
        execute_btn.connect_clicked(move |_| {
            let mut values = std::collections::HashMap::new();
            for (name, entry) in &entries {
                values.insert(name.clone(), entry.text().to_string());
            }

            let substituted =
                rustconn_core::SnippetManager::substitute_variables(&command, &values);
            notebook_clone.send_text(&format!("{substituted}\n"));
            window_clone.close();
        });

        var_window.present();
    }

    // ========== Session Management Methods ==========

    /// Shows the sessions manager window
    #[allow(clippy::too_many_lines)]
    fn show_sessions_manager(
        window: &ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
        sidebar: SharedSidebar,
    ) {
        let manager_window = gtk4::Window::builder()
            .title("Active Sessions")
            .transient_for(window)
            .modal(true)
            .default_width(500)
            .default_height(400)
            .build();

        // Create header bar
        let header = HeaderBar::new();
        let close_btn = Button::builder().label("Close").build();
        let refresh_btn = Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh")
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&refresh_btn);
        manager_window.set_titlebar(Some(&header));

        // Create main content
        let content = gtk4::Box::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Session count label
        let count_label = Label::builder()
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        content.append(&count_label);

        // Sessions list
        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let sessions_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&sessions_list));
        content.append(&scrolled);

        // Action buttons
        let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);

        let switch_btn = Button::builder()
            .label("Switch To")
            .sensitive(false)
            .build();
        let send_text_btn = Button::builder()
            .label("Send Text")
            .sensitive(false)
            .build();
        let terminate_btn = Button::builder()
            .label("Terminate")
            .sensitive(false)
            .css_classes(["destructive-action"])
            .build();

        button_box.append(&switch_btn);
        button_box.append(&send_text_btn);
        button_box.append(&terminate_btn);
        content.append(&button_box);

        manager_window.set_child(Some(&content));

        // Populate sessions list
        Self::populate_sessions_list(&state, &notebook, &sessions_list, &count_label);

        // Connect selection changed
        let switch_clone = switch_btn.clone();
        let send_text_clone = send_text_btn.clone();
        let terminate_clone = terminate_btn.clone();
        sessions_list.connect_row_selected(move |_, row| {
            let has_selection = row.is_some();
            switch_clone.set_sensitive(has_selection);
            send_text_clone.set_sensitive(has_selection);
            terminate_clone.set_sensitive(has_selection);
        });

        // Connect close button
        let window_clone = manager_window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect refresh button
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let list_clone = sessions_list.clone();
        let count_clone = count_label.clone();
        refresh_btn.connect_clicked(move |_| {
            Self::populate_sessions_list(&state_clone, &notebook_clone, &list_clone, &count_clone);
        });

        // Connect switch button
        let notebook_clone = notebook.clone();
        let list_clone = sessions_list.clone();
        let window_clone = manager_window.clone();
        switch_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("session-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        notebook_clone.switch_to_tab(id);
                        window_clone.close();
                    }
                }
            }
        });

        // Connect send text button - uses send_text_to_session to send text to specific session
        let notebook_clone = notebook.clone();
        let list_clone = sessions_list.clone();
        let manager_clone = manager_window.clone();
        send_text_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("session-") {
                    if let Ok(session_id) = Uuid::parse_str(id_str) {
                        // Show a simple text input dialog
                        Self::show_send_text_dialog(&manager_clone, &notebook_clone, session_id);
                    }
                }
            }
        });

        // Connect terminate button
        let state_clone = state;
        let notebook_clone = notebook;
        let list_clone = sessions_list;
        let count_clone = count_label;
        let manager_clone = manager_window.clone();
        let sidebar_clone = sidebar;
        terminate_btn.connect_clicked(move |_| {
            if let Some(row) = list_clone.selected_row() {
                if let Some(id_str) = row.widget_name().as_str().strip_prefix("session-") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Terminate Session?")
                            .detail("Are you sure you want to terminate this session?")
                            .buttons(["Cancel", "Terminate"])
                            .default_button(0)
                            .cancel_button(0)
                            .modal(true)
                            .build();

                        let state_inner = state_clone.clone();
                        let notebook_inner = notebook_clone.clone();
                        let list_inner = list_clone.clone();
                        let count_inner = count_clone.clone();
                        let sidebar_inner = sidebar_clone.clone();
                        alert.choose(
                            Some(&manager_clone),
                            gio::Cancellable::NONE,
                            move |result| {
                                if result == Ok(1) {
                                    // Terminate session in state manager
                                    if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                                        let _ = state_mut.terminate_session(id);
                                    }

                                    // Update sidebar status
                                    if let Some(info) = notebook_inner.get_session_info(id) {
                                        sidebar_inner.update_connection_status(
                                            &info.connection_id.to_string(),
                                            "disconnected",
                                        );
                                    }

                                    // Close the tab
                                    notebook_inner.close_tab(id);
                                    // Refresh the list
                                    Self::populate_sessions_list(
                                        &state_inner,
                                        &notebook_inner,
                                        &list_inner,
                                        &count_inner,
                                    );
                                }
                            },
                        );
                    }
                }
            }
        });

        manager_window.present();
    }

    /// Populates the sessions list
    fn populate_sessions_list(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        list: &gtk4::ListBox,
        count_label: &Label,
    ) {
        // Clear existing rows
        while let Some(row) = list.row_at_index(0) {
            list.remove(&row);
        }

        // Get sessions from notebook (UI sessions)
        let session_ids = notebook.session_ids();
        let session_count = session_ids.len();

        // Also get active sessions from state manager for additional info
        let state_ref = state.borrow();
        let active_sessions = state_ref.active_sessions();
        let state_session_count = active_sessions.len();
        drop(state_ref);

        // Show both UI sessions and state-tracked sessions
        count_label.set_text(&format!(
            "{session_count} UI session(s), {state_session_count} tracked session(s)"
        ));

        for session_id in session_ids {
            if let Some(info) = notebook.get_session_info(session_id) {
                let row = gtk4::ListBoxRow::new();
                row.set_widget_name(&format!("session-{session_id}"));

                let hbox = gtk4::Box::new(Orientation::Horizontal, 12);
                hbox.set_margin_top(8);
                hbox.set_margin_bottom(8);
                hbox.set_margin_start(12);
                hbox.set_margin_end(12);

                // Protocol icon
                let icon_name = match info.protocol.as_str() {
                    "ssh" | "local" => "utilities-terminal-symbolic",
                    "rdp" => "computer-symbolic",
                    "vnc" => "video-display-symbolic",
                    _ => "network-server-symbolic",
                };
                let icon = gtk4::Image::from_icon_name(icon_name);
                hbox.append(&icon);

                let vbox = gtk4::Box::new(Orientation::Vertical, 4);
                vbox.set_hexpand(true);

                let name_label = Label::builder()
                    .label(&info.name)
                    .halign(gtk4::Align::Start)
                    .css_classes(["heading"])
                    .build();
                vbox.append(&name_label);

                // Get connection info if available
                let state_ref = state.borrow();
                let connection_info = if info.connection_id == Uuid::nil() {
                    Some(info.protocol.to_uppercase().clone())
                } else {
                    state_ref
                        .get_connection(info.connection_id)
                        .map(|c| format!("{} ({})", c.host, info.protocol.to_uppercase()))
                };
                drop(state_ref);

                if let Some(conn_info) = connection_info {
                    let info_label = Label::builder()
                        .label(&conn_info)
                        .halign(gtk4::Align::Start)
                        .css_classes(["dim-label"])
                        .build();
                    vbox.append(&info_label);
                }

                // Session type indicator - show protocol
                let type_label = Label::builder()
                    .label(info.protocol.to_uppercase())
                    .halign(gtk4::Align::Start)
                    .css_classes(["dim-label"])
                    .build();
                vbox.append(&type_label);

                hbox.append(&vbox);
                row.set_child(Some(&hbox));
                list.append(&row);
            }
        }
    }

    /// Shows a dialog to send text to a specific session
    fn show_send_text_dialog(parent: &gtk4::Window, notebook: &SharedNotebook, session_id: Uuid) {
        let dialog = gtk4::Window::builder()
            .title("Send Text to Session")
            .transient_for(parent)
            .modal(true)
            .default_width(400)
            .build();

        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let send_btn = Button::builder()
            .label("Send")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&send_btn);
        dialog.set_titlebar(Some(&header));

        let content = gtk4::Box::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let label = Label::builder()
            .label("Enter text to send to the session:")
            .halign(gtk4::Align::Start)
            .build();
        content.append(&label);

        let entry = gtk4::Entry::builder()
            .placeholder_text("Text to send...")
            .hexpand(true)
            .build();
        content.append(&entry);

        let newline_check = gtk4::CheckButton::builder()
            .label("Append newline (press Enter)")
            .active(true)
            .build();
        content.append(&newline_check);

        dialog.set_child(Some(&content));

        // Connect cancel button
        let dialog_clone = dialog.clone();
        cancel_btn.connect_clicked(move |_| {
            dialog_clone.close();
        });

        // Connect send button - uses send_text_to_session
        let notebook_clone = notebook.clone();
        let dialog_clone = dialog.clone();
        let entry_clone = entry.clone();
        let newline_clone = newline_check.clone();
        send_btn.connect_clicked(move |_| {
            let text = entry_clone.text().to_string();
            if !text.is_empty() {
                let text_to_send = if newline_clone.is_active() {
                    format!("{text}\n")
                } else {
                    text
                };
                // Use send_text_to_session to send to the specific session
                notebook_clone.send_text_to_session(session_id, &text_to_send);
            }
            dialog_clone.close();
        });

        // Also send on Enter key
        let notebook_clone = notebook.clone();
        let dialog_clone = dialog.clone();
        let newline_clone = newline_check;
        entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if !text.is_empty() {
                let text_to_send = if newline_clone.is_active() {
                    format!("{text}\n")
                } else {
                    text
                };
                notebook_clone.send_text_to_session(session_id, &text_to_send);
            }
            dialog_clone.close();
        });

        dialog.present();
    }

    // ========== Group Hierarchy Methods ==========

    /// Shows the move to group dialog for the selected connection
    #[allow(clippy::too_many_lines)]
    fn show_move_to_group_dialog(
        window: &ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Get selected item using sidebar's method (works in both single and multi-selection modes)
        let Some(conn_item) = sidebar.get_selected_item() else {
            return;
        };

        // Can only move connections, not groups
        if conn_item.is_group() {
            let alert = gtk4::AlertDialog::builder()
                .message("Cannot Move Group")
                .detail("Use drag and drop to reorganize groups.")
                .modal(true)
                .build();
            alert.show(Some(window));
            return;
        }

        let id_str = conn_item.id();
        let Ok(connection_id) = Uuid::parse_str(&id_str) else {
            return;
        };
        let connection_name = conn_item.name();

        // Get current group
        let state_ref = state.borrow();
        let current_group_id = state_ref
            .get_connection(connection_id)
            .and_then(|c| c.group_id);
        drop(state_ref);

        // Create dialog
        let move_window = gtk4::Window::builder()
            .title("Move Connection")
            .transient_for(window)
            .modal(true)
            .default_width(350)
            .build();

        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let move_btn = Button::builder()
            .label("Move")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&move_btn);
        move_window.set_titlebar(Some(&header));

        let content = gtk4::Box::new(Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let info_label = Label::builder()
            .label(format!("Move '{connection_name}' to:"))
            .halign(gtk4::Align::Start)
            .build();
        content.append(&info_label);

        // Group dropdown
        let state_ref = state.borrow();
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();
        drop(state_ref);

        let mut group_ids: Vec<Option<Uuid>> = vec![None];
        let mut strings: Vec<String> = vec!["(Ungrouped)".to_string()];
        let mut current_index = 0u32;

        for group in &groups {
            let state_ref = state.borrow();
            let path = state_ref
                .get_group_path(group.id)
                .unwrap_or_else(|| group.name.clone());
            drop(state_ref);

            strings.push(path);
            group_ids.push(Some(group.id));

            if current_group_id == Some(group.id) {
                #[allow(clippy::cast_possible_truncation)]
                {
                    current_index = (group_ids.len() - 1) as u32;
                }
            }
        }

        let string_list = gtk4::StringList::new(
            &strings
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        let group_dropdown = gtk4::DropDown::builder()
            .model(&string_list)
            .selected(current_index)
            .build();

        content.append(&group_dropdown);
        move_window.set_child(Some(&content));

        // Connect cancel
        let window_clone = move_window.clone();
        cancel_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Connect move
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_clone = move_window.clone();
        move_btn.connect_clicked(move |_| {
            let selected_idx = group_dropdown.selected() as usize;
            let target_group_id = if selected_idx < group_ids.len() {
                group_ids[selected_idx]
            } else {
                None
            };

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                match state_mut.move_connection_to_group(connection_id, target_group_id) {
                    Ok(()) => {
                        drop(state_mut);
                        // Preserve tree state when moving connections
                        Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        window_clone.close();
                    }
                    Err(e) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error Moving Connection")
                            .detail(&e)
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                    }
                }
            }
        });

        move_window.present();
    }

    /// Shows an error toast/notification
    fn show_error_toast(window: &ApplicationWindow, message: &str) {
        let alert = gtk4::AlertDialog::builder()
            .message("Error")
            .detail(message)
            .modal(true)
            .build();
        alert.show(Some(window));
    }

    // ========== Template Management Methods ==========

    /// Shows the templates manager window
    fn show_templates_manager(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        use crate::dialogs::{TemplateDialog, TemplateManagerDialog};

        let manager_dialog = TemplateManagerDialog::new(Some(&window.clone().upcast()));

        // Load templates from config file and active document
        let templates = {
            let state_ref = state.borrow();
            state_ref.get_all_templates()
        };
        manager_dialog.set_templates(templates);

        // Get references for closures
        let templates_list = manager_dialog.templates_list().clone();
        let state_templates = manager_dialog.state_templates().clone();
        let manager_window = manager_dialog.window().clone();

        // Connect filter dropdown
        if let Some(content) = manager_window.child() {
            if let Some(vbox) = content.downcast_ref::<gtk4::Box>() {
                if let Some(filter_box) = vbox.first_child() {
                    if let Some(hbox) = filter_box.downcast_ref::<gtk4::Box>() {
                        if let Some(dropdown_widget) = hbox.last_child() {
                            if let Some(filter_dropdown) =
                                dropdown_widget.downcast_ref::<gtk4::DropDown>()
                            {
                                let list_clone = templates_list.clone();
                                let templates_clone = state_templates.clone();
                                filter_dropdown.connect_selected_notify(move |dropdown| {
                                    let selected = dropdown.selected();
                                    let filter = match selected {
                                        1 => Some(rustconn_core::models::ProtocolType::Ssh),
                                        2 => Some(rustconn_core::models::ProtocolType::Rdp),
                                        3 => Some(rustconn_core::models::ProtocolType::Vnc),
                                        4 => Some(rustconn_core::models::ProtocolType::Spice),
                                        _ => None,
                                    };
                                    Self::refresh_templates_list(
                                        &list_clone,
                                        &templates_clone,
                                        filter,
                                    );
                                });
                            }
                        }
                    }
                }
            }
        }

        // Set up "New Template" callback
        {
            let state_clone = state.clone();
            let templates_clone = state_templates.clone();
            let list_clone = templates_list.clone();
            let manager_clone = manager_window.clone();
            manager_dialog.set_on_new(move || {
                let dialog = TemplateDialog::new(Some(&manager_clone.clone().upcast()));
                let state_inner = state_clone.clone();
                let templates_inner = templates_clone.clone();
                let list_inner = list_clone.clone();
                let manager_inner = manager_clone.clone();
                dialog.run(move |result| {
                    if let Some(template) = result {
                        // Add to state templates (local cache)
                        templates_inner.borrow_mut().push(template.clone());
                        // Save to config file and active document
                        if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                            if let Err(e) = state_mut.add_template(template) {
                                let alert = gtk4::AlertDialog::builder()
                                    .message("Error Saving Template")
                                    .detail(&e)
                                    .modal(true)
                                    .build();
                                alert.show(Some(&manager_inner));
                            }
                        }
                        // Refresh list
                        Self::refresh_templates_list(&list_inner, &templates_inner, None);
                    }
                });
            });
        }

        // Set up "Edit" callback
        {
            let state_clone = state.clone();
            let templates_clone = state_templates.clone();
            let list_clone = templates_list.clone();
            let manager_clone = manager_window.clone();
            manager_dialog.set_on_edit(move |template| {
                let id = template.id;
                let dialog = TemplateDialog::new(Some(&manager_clone.clone().upcast()));
                dialog.set_template(&template);
                let state_inner = state_clone.clone();
                let templates_inner = templates_clone.clone();
                let list_inner = list_clone.clone();
                let manager_inner = manager_clone.clone();
                dialog.run(move |result| {
                    if let Some(updated) = result {
                        // Update in state templates (local cache)
                        let mut templates = templates_inner.borrow_mut();
                        if let Some(pos) = templates.iter().position(|t| t.id == id) {
                            templates[pos] = updated.clone();
                        }
                        drop(templates);
                        // Update in config file and active document
                        if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                            if let Err(e) = state_mut.update_template(updated) {
                                let alert = gtk4::AlertDialog::builder()
                                    .message("Error Saving Template")
                                    .detail(&e)
                                    .modal(true)
                                    .build();
                                alert.show(Some(&manager_inner));
                            }
                        }
                        // Refresh list
                        Self::refresh_templates_list(&list_inner, &templates_inner, None);
                    }
                });
            });
        }

        // Set up "Delete" callback
        {
            let state_clone = state.clone();
            let templates_clone = state_templates.clone();
            let list_clone = templates_list.clone();
            let manager_clone = manager_window.clone();
            manager_dialog.set_on_delete(move |id| {
                let alert = gtk4::AlertDialog::builder()
                    .message("Delete Template?")
                    .detail("Are you sure you want to delete this template?")
                    .buttons(["Cancel", "Delete"])
                    .default_button(0)
                    .cancel_button(0)
                    .modal(true)
                    .build();

                let state_inner = state_clone.clone();
                let templates_inner = templates_clone.clone();
                let list_inner = list_clone.clone();
                alert.choose(
                    Some(&manager_clone),
                    gio::Cancellable::NONE,
                    move |result| {
                        if result == Ok(1) {
                            // Remove from state templates (local cache)
                            templates_inner.borrow_mut().retain(|t| t.id != id);
                            // Remove from config file and active document
                            if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                                if let Err(e) = state_mut.delete_template(id) {
                                    eprintln!("Failed to delete template: {e}");
                                }
                            }
                            // Refresh list
                            Self::refresh_templates_list(&list_inner, &templates_inner, None);
                        }
                    },
                );
            });
        }

        // Set up "Use Template" callback
        {
            let state_clone = state.clone();
            let manager_clone = manager_window.clone();
            let sidebar_clone = sidebar.clone();
            manager_dialog.set_on_template_selected(move |template_opt| {
                if let Some(template) = template_opt {
                    // Create connection from template
                    Self::show_new_connection_from_template(
                        &manager_clone,
                        state_clone.clone(),
                        sidebar_clone.clone(),
                        &template,
                    );
                }
            });
        }

        manager_dialog.present();
    }

    /// Refreshes the templates list with optional protocol filter
    fn refresh_templates_list(
        list: &gtk4::ListBox,
        templates: &std::rc::Rc<std::cell::RefCell<Vec<rustconn_core::models::ConnectionTemplate>>>,
        protocol_filter: Option<rustconn_core::models::ProtocolType>,
    ) {
        use rustconn_core::models::ProtocolType;

        // Clear existing rows
        while let Some(row) = list.row_at_index(0) {
            list.remove(&row);
        }

        let templates_ref = templates.borrow();

        // Group templates by protocol
        let mut ssh_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();
        let mut rdp_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();
        let mut vnc_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();
        let mut spice_templates: Vec<&rustconn_core::models::ConnectionTemplate> = Vec::new();

        for template in templates_ref.iter() {
            if let Some(filter) = protocol_filter {
                if template.protocol != filter {
                    continue;
                }
            }
            match template.protocol {
                ProtocolType::Ssh | ProtocolType::ZeroTrust => ssh_templates.push(template),
                ProtocolType::Rdp => rdp_templates.push(template),
                ProtocolType::Vnc => vnc_templates.push(template),
                ProtocolType::Spice => spice_templates.push(template),
            }
        }

        // Helper to add section header
        let add_section_header = |list: &gtk4::ListBox, title: &str| {
            let label = Label::builder()
                .label(title)
                .halign(gtk4::Align::Start)
                .css_classes(["heading"])
                .margin_top(8)
                .margin_bottom(4)
                .margin_start(8)
                .build();
            let row = gtk4::ListBoxRow::builder()
                .child(&label)
                .selectable(false)
                .activatable(false)
                .build();
            list.append(&row);
        };

        // Helper to add template row
        let add_template_row =
            |list: &gtk4::ListBox, template: &rustconn_core::models::ConnectionTemplate| {
                let hbox = gtk4::Box::new(Orientation::Horizontal, 8);
                hbox.set_margin_top(8);
                hbox.set_margin_bottom(8);
                hbox.set_margin_start(8);
                hbox.set_margin_end(8);

                // Protocol icon
                let icon_name = match template.protocol {
                    ProtocolType::Ssh => "utilities-terminal-symbolic",
                    ProtocolType::Rdp => "computer-symbolic",
                    ProtocolType::Vnc => "video-display-symbolic",
                    ProtocolType::Spice => "video-display-symbolic",
                    ProtocolType::ZeroTrust => "cloud-symbolic",
                };
                let icon = gtk4::Image::from_icon_name(icon_name);
                hbox.append(&icon);

                // Template info
                let info_box = gtk4::Box::new(Orientation::Vertical, 2);
                info_box.set_hexpand(true);

                let name_label = Label::builder()
                    .label(&template.name)
                    .halign(gtk4::Align::Start)
                    .css_classes(["heading"])
                    .build();
                info_box.append(&name_label);

                let details = if let Some(ref desc) = template.description {
                    desc.clone()
                } else {
                    let mut parts = Vec::new();
                    if !template.host.is_empty() {
                        parts.push(format!("Host: {}", template.host));
                    }
                    parts.push(format!("Port: {}", template.port));
                    if let Some(ref user) = template.username {
                        parts.push(format!("User: {user}"));
                    }
                    parts.join(" | ")
                };

                let details_label = Label::builder()
                    .label(&details)
                    .halign(gtk4::Align::Start)
                    .css_classes(["dim-label"])
                    .build();
                info_box.append(&details_label);

                hbox.append(&info_box);

                let row = gtk4::ListBoxRow::builder().child(&hbox).build();
                row.set_widget_name(&format!("template-{}", template.id));
                list.append(&row);
            };

        // Add SSH templates
        if !ssh_templates.is_empty() && protocol_filter.is_none() {
            add_section_header(list, "SSH Templates");
        }
        for template in ssh_templates {
            add_template_row(list, template);
        }

        // Add RDP templates
        if !rdp_templates.is_empty() && protocol_filter.is_none() {
            add_section_header(list, "RDP Templates");
        }
        for template in rdp_templates {
            add_template_row(list, template);
        }

        // Add VNC templates
        if !vnc_templates.is_empty() && protocol_filter.is_none() {
            add_section_header(list, "VNC Templates");
        }
        for template in vnc_templates {
            add_template_row(list, template);
        }

        // Add SPICE templates
        if !spice_templates.is_empty() && protocol_filter.is_none() {
            add_section_header(list, "SPICE Templates");
        }
        for template in spice_templates {
            add_template_row(list, template);
        }
    }

    /// Shows the new connection dialog pre-populated from a template
    fn show_new_connection_from_template(
        window: &gtk4::Window,
        state: SharedAppState,
        sidebar: SharedSidebar,
        template: &rustconn_core::models::ConnectionTemplate,
    ) {
        use crate::dialogs::ConnectionDialog;

        // Create connection from template
        let connection = template.apply(None);

        let dialog = ConnectionDialog::new(Some(window));
        dialog.setup_key_file_chooser(Some(window));

        // Set KeePass enabled state from settings
        {
            let state_ref = state.borrow();
            let keepass_enabled = state_ref.settings().secrets.kdbx_enabled;
            dialog.set_keepass_enabled(keepass_enabled);
        }

        // Pre-populate with template values
        dialog.set_connection(&connection);
        // Reset the title since we're creating a new connection
        dialog
            .window()
            .set_title(Some("New Connection from Template"));

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(conn) = result {
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    match state_mut.create_connection(conn) {
                        Ok(_) => {
                            // Reload sidebar preserving tree state
                            drop(state_mut);
                            Self::reload_sidebar_preserving_state(&state, &sidebar);
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Creating Connection")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        });
    }

    /// Shows the new cluster dialog
    fn show_new_cluster_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
    ) {
        let dialog = ClusterDialog::new(Some(&window.clone().upcast()));

        // Populate available connections
        {
            let state_ref = state.borrow();
            let connections: Vec<_> = state_ref
                .list_connections()
                .iter()
                .cloned()
                .cloned()
                .collect();
            dialog.set_connections(&connections);
        }

        let window_clone = window.clone();
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        dialog.run(move |result| {
            if let Some(cluster) = result {
                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    match state_mut.create_cluster(cluster) {
                        Ok(_) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Cluster Created")
                                .detail("Cluster has been saved successfully.")
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Creating Cluster")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
            // Keep notebook reference alive
            let _ = &notebook_clone;
        });
    }

    /// Shows the clusters manager dialog
    fn show_clusters_manager(
        window: &ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
        sidebar: SharedSidebar,
    ) {
        let dialog = ClusterListDialog::new(Some(&window.clone().upcast()));

        // Set up clusters provider for refresh operations
        let state_for_provider = state.clone();
        dialog.set_clusters_provider(move || {
            if let Ok(state_ref) = state_for_provider.try_borrow() {
                state_ref
                    .get_all_clusters()
                    .iter()
                    .cloned()
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            }
        });

        // Wrap dialog in Rc for shared access across callbacks
        let dialog_ref = std::rc::Rc::new(dialog);

        // Initial population of clusters - refresh after provider is set
        // Use show callback to ensure dialog is ready
        let dialog_for_refresh = dialog_ref.clone();
        dialog_ref.window().connect_show(move |_| {
            dialog_for_refresh.refresh_list();
        });

        // Helper to refresh the cluster list using the dialog's refresh_list method
        let create_refresh_callback = |dialog_ref: std::rc::Rc<ClusterListDialog>| {
            move || {
                dialog_ref.refresh_list();
            }
        };

        // Set up callbacks
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let window_clone = window.clone();
        let sidebar_clone = sidebar.clone();
        dialog_ref.set_on_connect(move |cluster_id| {
            Self::connect_cluster(
                &state_clone,
                &notebook_clone,
                &window_clone,
                &sidebar_clone,
                cluster_id,
            );
        });

        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let dialog_window = dialog_ref.window().clone();
        let dialog_ref_edit = dialog_ref.clone();
        let refresh_after_edit = create_refresh_callback(dialog_ref_edit.clone());
        dialog_ref.set_on_edit(move |cluster_id| {
            Self::edit_cluster(
                &dialog_window,
                &state_clone,
                &notebook_clone,
                cluster_id,
                Box::new(refresh_after_edit.clone()),
            );
        });

        let state_clone = state.clone();
        let dialog_window = dialog_ref.window().clone();
        let dialog_ref_delete = dialog_ref.clone();
        let refresh_after_delete = create_refresh_callback(dialog_ref_delete.clone());
        dialog_ref.set_on_delete(move |cluster_id| {
            Self::delete_cluster(
                &dialog_window,
                &state_clone,
                cluster_id,
                Box::new(refresh_after_delete.clone()),
            );
        });

        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let dialog_window = dialog_ref.window().clone();
        let dialog_ref_new = dialog_ref.clone();
        let refresh_after_new = create_refresh_callback(dialog_ref_new.clone());
        dialog_ref.set_on_new(move || {
            Self::show_new_cluster_dialog_from_manager(
                &dialog_window,
                state_clone.clone(),
                notebook_clone.clone(),
                Box::new(refresh_after_new.clone()),
            );
        });

        dialog_ref.show();
    }

    /// Shows new cluster dialog from the manager
    fn show_new_cluster_dialog_from_manager(
        parent: &gtk4::Window,
        state: SharedAppState,
        _notebook: SharedNotebook,
        on_created: Box<dyn Fn() + 'static>,
    ) {
        let dialog = ClusterDialog::new(Some(parent));

        // Populate available connections
        {
            let state_ref = state.borrow();
            let connections: Vec<_> = state_ref
                .list_connections()
                .iter()
                .cloned()
                .cloned()
                .collect();
            dialog.set_connections(&connections);
        }

        let state_clone = state.clone();
        let parent_clone = parent.clone();
        dialog.run(move |result| {
            if let Some(cluster) = result {
                let create_result = if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    state_mut.create_cluster(cluster)
                } else {
                    Err("Could not access application state".to_string())
                };

                match create_result {
                    Ok(_) => {
                        // Refresh the cluster list in the parent dialog
                        on_created();
                    }
                    Err(e) => {
                        // Show error dialog
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error Creating Cluster")
                            .detail(&format!("Failed to save cluster: {e}"))
                            .modal(true)
                            .build();
                        alert.show(Some(&parent_clone));
                    }
                }
            }
        });
    }

    /// Connects to all connections in a cluster
    fn connect_cluster(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        _window: &ApplicationWindow,
        sidebar: &SharedSidebar,
        cluster_id: Uuid,
    ) {
        let state_ref = state.borrow();
        let Some(cluster) = state_ref.get_cluster(cluster_id) else {
            return;
        };

        let connection_ids: Vec<Uuid> = cluster.connection_ids.clone();
        drop(state_ref);

        // Connect to each connection in the cluster using existing start_connection
        for conn_id in connection_ids {
            Self::start_connection(state, notebook, sidebar, conn_id);
        }
    }

    /// Edits a cluster
    fn edit_cluster(
        parent: &gtk4::Window,
        state: &SharedAppState,
        _notebook: &SharedNotebook,
        cluster_id: Uuid,
        on_updated: Box<dyn Fn() + 'static>,
    ) {
        let state_ref = state.borrow();
        let Some(cluster) = state_ref.get_cluster(cluster_id).cloned() else {
            return;
        };
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .cloned()
            .cloned()
            .collect();
        drop(state_ref);

        let dialog = ClusterDialog::new(Some(parent));
        dialog.set_connections(&connections);
        dialog.set_cluster(&cluster);

        let state_clone = state.clone();
        let parent_clone = parent.clone();
        dialog.run(move |result| {
            if let Some(updated) = result {
                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    match state_mut.update_cluster(updated) {
                        Ok(()) => {
                            // Refresh the cluster list
                            on_updated();
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Updating Cluster")
                                .detail(&format!("Failed to save cluster: {e}"))
                                .modal(true)
                                .build();
                            alert.show(Some(&parent_clone));
                        }
                    }
                } else {
                    let alert = gtk4::AlertDialog::builder()
                        .message("Error")
                        .detail("Could not access application state")
                        .modal(true)
                        .build();
                    alert.show(Some(&parent_clone));
                }
            }
        });
    }

    /// Deletes a cluster
    fn delete_cluster(
        parent: &gtk4::Window,
        state: &SharedAppState,
        cluster_id: Uuid,
        on_deleted: Box<dyn Fn() + 'static>,
    ) {
        let state_ref = state.borrow();
        let Some(cluster) = state_ref.get_cluster(cluster_id) else {
            return;
        };
        let cluster_name = cluster.name.clone();
        drop(state_ref);

        let alert = gtk4::AlertDialog::builder()
            .message("Delete Cluster?")
            .detail(&format!(
                "Are you sure you want to delete the cluster '{cluster_name}'?\n\
                This will not delete the connections in the cluster."
            ))
            .buttons(["Cancel", "Delete"])
            .default_button(0)
            .cancel_button(0)
            .modal(true)
            .build();

        let state_clone = state.clone();
        let parent_clone = parent.clone();
        alert.choose(Some(parent), None::<&gio::Cancellable>, move |result| {
            if result == Ok(1) {
                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    match state_mut.delete_cluster(cluster_id) {
                        Ok(()) => {
                            // Refresh the cluster list
                            on_deleted();
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Deleting Cluster")
                                .detail(&format!("Failed to delete cluster: {e}"))
                                .modal(true)
                                .build();
                            alert.show(Some(&parent_clone));
                        }
                    }
                } else {
                    let alert = gtk4::AlertDialog::builder()
                        .message("Error")
                        .detail("Could not access application state")
                        .modal(true)
                        .build();
                    alert.show(Some(&parent_clone));
                }
            }
        });
    }
}
