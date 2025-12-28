//! Main application window
//!
//! This module provides the main window implementation for `RustConn`,
//! including the header bar, sidebar, terminal area, and action handling.

use gtk4::prelude::*;
use gtk4::{
    gio, glib, Application, ApplicationWindow, Button, HeaderBar, Label, MenuButton, Orientation,
    Paned,
};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use crate::window_clusters as clusters;
use crate::window_groups as groups;
use crate::window_sessions as sessions;
use crate::window_snippets as snippets;
use crate::window_templates as templates;
use crate::window_types::{
    get_protocol_string, SharedExternalWindowManager, SharedNotebook, SharedSidebar,
    SharedSplitView,
};

use crate::dialogs::{ConnectionDialog, ExportDialog, PasswordDialog, SettingsDialog};
use crate::embedded::{EmbeddedSessionTab, RdpLauncher};
use crate::external_window::ExternalWindowManager;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::split_view::{SplitDirection, SplitTerminalView};
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;

/// Main application window wrapper
///
/// Provides access to the main window and its components.
#[allow(dead_code)] // Fields kept for GTK widget lifecycle and future use
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
                groups::show_move_to_group_dialog(&win, &state_clone, &sidebar_clone);
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
                snippets::show_new_snippet_dialog(&win, state_clone.clone());
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
                snippets::show_snippets_manager(&win, state_clone.clone(), notebook_clone.clone());
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
                snippets::show_snippet_picker(&win, state_clone.clone(), notebook_clone.clone());
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
                sessions::show_sessions_manager(
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
                clusters::show_new_cluster_dialog(
                    &win,
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
                templates::show_templates_manager(&win, state_clone.clone(), sidebar_clone.clone());
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
                                groups::show_error_toast(
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
                                groups::show_error_toast(
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
                                        groups::show_error_toast(
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
                                            groups::show_error_toast(
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
                                        groups::show_error_toast(
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
                            // Decrement session count - status changes only if no other sessions
                            sidebar_for_close
                                .decrement_session_count(&info.connection_id.to_string(), false);
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
                        // Decrement session count - status changes only if no other sessions active
                        sidebar_for_state.decrement_session_count(
                            &connection_id.to_string(),
                            // If tab is still open, it's an unexpected disconnection (failure)
                            notebook_for_state.get_session_info(session_id).is_some(),
                        );
                    }
                    crate::embedded_rdp::RdpConnectionState::Connected => {
                        notebook_for_state.mark_tab_connected(session_id);
                        sidebar_for_state.increment_session_count(&connection_id.to_string());
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
                // External RDP window started - increment session count
                sidebar.increment_session_count(&connection_id.to_string());
            }

            // Add tab widget to notebook
            notebook.add_embedded_session_tab(session_id, &conn_name, "rdp", tab.widget());

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
                            .decrement_session_count(&connection_id.to_string(), false);
                    } else if vnc_state == crate::session::SessionState::Connected {
                        notebook_for_state.mark_tab_connected(session_id);
                        sidebar_for_state.increment_session_count(&connection_id.to_string());
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

                notebook.connect_contents_changed(session_id, move || {
                    // Only increment once per session
                    if !session_connected_clone.get() {
                        // Check if content indicates actual output from the process
                        // The initial header is 2 lines. If we have more, it means the process produced output.
                        if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) {
                            if row > 2 {
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
    #[allow(clippy::too_many_lines)]
    pub fn start_connection(
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
                let session_id = notebook.create_terminal_tab(
                    connection_id,
                    &conn.name,
                    &protocol,
                    Some(&conn.automation),
                );

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
                            sidebar_for_state
                                .decrement_session_count(&connection_id.to_string(), false);
                        } else if vnc_state == crate::session::SessionState::Connected {
                            notebook_for_state.mark_tab_connected(session_id);
                            sidebar_for_state.increment_session_count(&connection_id.to_string());
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
                    let sidebar_for_state = sidebar.clone();
                    spice_widget.connect_state_changed(move |spice_state| {
                        use crate::embedded_spice::SpiceConnectionState;
                        if spice_state == SpiceConnectionState::Disconnected
                            || spice_state == SpiceConnectionState::Error
                        {
                            notebook_for_state.mark_tab_disconnected(session_id);
                            sidebar_for_state.decrement_session_count(
                                &connection_id.to_string(),
                                spice_state == SpiceConnectionState::Error,
                            );
                        } else if spice_state == SpiceConnectionState::Connected {
                            notebook_for_state.mark_tab_connected(session_id);
                            sidebar_for_state.increment_session_count(&connection_id.to_string());
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

                let automation_config = conn.automation.clone();
                drop(state_ref);

                // Create terminal tab for Zero Trust with provider-specific protocol
                let tab_protocol = format!("zerotrust:{provider_key}");
                let session_id = notebook.create_terminal_tab(
                    connection_id,
                    &conn_name,
                    &tab_protocol,
                    Some(&automation_config),
                );

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
            // If it doesn't, the tab was closed by user
            if notebook_clone.get_session_info(session_id).is_none() {
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

            if is_failure {
                eprintln!("Session {session_id} exited with status: {exit_status} (Signal: {term_sig}, Code: {exit_code})");
            }

            // Decrement session count - status changes only if no other sessions active
            sidebar_clone.decrement_session_count(&connection_id_str, is_failure);
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
        crate::window_connection_dialogs::show_new_connection_dialog(window, state, sidebar);
    }

    /// Shows the new group dialog with optional parent selection
    fn show_new_group_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        crate::window_connection_dialogs::show_new_group_dialog(window, state, sidebar);
    }

    /// Shows the import dialog
    fn show_import_dialog(
        window: &ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        crate::window_connection_dialogs::show_import_dialog(window, state, sidebar);
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
    fn open_local_shell_with_split(notebook: &SharedNotebook, split_view: &SharedSplitView) {
        let session_id = notebook.create_terminal_tab(Uuid::nil(), "Local Shell", "local", None);

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
                    let session_id = notebook.create_terminal_tab(
                        Uuid::nil(),
                        &format!("Quick: {host}"),
                        "ssh",
                        None,
                    );

                    notebook.spawn_ssh(session_id, &host, port, username.as_deref(), None, &[]);
                }
                1 => {
                    // RDP - native embedding not yet implemented, show placeholder
                    let _session_id = notebook.create_terminal_tab(
                        Uuid::nil(),
                        &format!("Quick: {host}"),
                        "rdp",
                        None,
                    );

                    eprintln!(
                        "Note: Native RDP embedding not yet implemented. \
                         Quick connect to '{host}' shown as placeholder."
                    );
                }
                2 => {
                    // VNC - native embedding not yet implemented, show placeholder
                    let _session_id = notebook.create_terminal_tab(
                        Uuid::nil(),
                        &format!("Quick: {host}"),
                        "vnc",
                        None,
                    );

                    eprintln!(
                        "Note: Native VNC embedding not yet implemented. \
                         Quick connect to '{host}' shown as placeholder."
                    );
                }
                _ => {
                    // Default to SSH
                    let session_id = notebook.create_terminal_tab(
                        Uuid::nil(),
                        &format!("Quick: {host}"),
                        "ssh",
                        None,
                    );

                    notebook.spawn_ssh(session_id, &host, port, username.as_deref(), None, &[]);
                }
            }

            window_clone.close();
        });

        quick_window.present();
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
}
