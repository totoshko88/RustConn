//! Terminal notebook area using adw::TabView
//!
//! This module provides the tabbed terminal interface using VTE4
//! for SSH sessions and native GTK widgets for VNC/RDP/SPICE connections.
//!
//! # Module Structure
//!
//! - `types` - Data structures for sessions
//! - `config` - Terminal appearance and behavior configuration

mod config;
mod types;

pub use types::{SessionWidgetStorage, TerminalSession};

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, Widget, gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use regex::Regex;
use rustconn_core::models::AutomationConfig;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;
use vte4::prelude::*;
use vte4::{PtyFlags, Terminal};

use crate::automation::{AutomationSession, Trigger};
use crate::embedded_rdp::EmbeddedRdpWidget;
use crate::embedded_spice::EmbeddedSpiceWidget;
use crate::i18n::i18n;
use crate::session::{SessionState, SessionWidget, VncSessionWidget};
use crate::split_view::TabSplitManager;
use rustconn_core::split::TabId;
use rustconn_core::split::tab_groups::TabGroupManager;

/// Terminal notebook widget for managing multiple terminal sessions
/// Now using adw::TabView for modern GNOME HIG compliance
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle
pub struct TerminalNotebook {
    /// Main container with TabView and TabBar
    container: GtkBox,
    /// The adw::TabView for managing tabs
    tab_view: adw::TabView,
    /// The adw::TabBar for displaying tabs
    tab_bar: adw::TabBar,
    /// Map of session IDs to their TabPage
    sessions: Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
    /// Callback for when a page is closed (session_id, connection_id)
    on_page_closed: Rc<RefCell<Option<Box<dyn Fn(Uuid, Uuid)>>>>,
    /// Callback for split view cleanup when a page is about to close (session_id)
    on_split_cleanup: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    /// Map of session IDs to terminal widgets (for SSH sessions)
    terminals: Rc<RefCell<HashMap<Uuid, Terminal>>>,
    /// Map of session IDs to session widgets (for VNC/RDP/SPICE sessions)
    session_widgets: Rc<RefCell<HashMap<Uuid, SessionWidgetStorage>>>,
    /// Map of session IDs to automation sessions
    automation_sessions: Rc<RefCell<HashMap<Uuid, AutomationSession>>>,
    /// Session metadata
    session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    /// Tab split manager for managing split layouts per tab
    /// Requirements 3.1, 3.3, 3.4: Each tab maintains its own split container
    split_manager: Rc<RefCell<TabSplitManager>>,
    /// Map of session IDs to their TabId (for split layout tracking)
    session_tab_ids: Rc<RefCell<HashMap<Uuid, TabId>>>,
    /// Whether to color tab indicators by protocol type
    color_tabs_by_protocol: Rc<RefCell<bool>>,
    /// Tab group manager for assigning colors to named groups
    tab_group_manager: Rc<RefCell<TabGroupManager>>,
}

impl TerminalNotebook {
    /// Creates a new terminal notebook using adw::TabView
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);

        // Create TabView - content visibility controlled dynamically
        // For SSH: TabView hidden, content in split_view
        // For RDP/VNC/SPICE: TabView visible, content in TabView pages
        let tab_view = adw::TabView::new();
        tab_view.set_hexpand(true);
        tab_view.set_vexpand(true); // Will expand when visible for RDP/VNC/SPICE

        // Create TabBar - this is what we show
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        tab_bar.set_autohide(false);
        tab_bar.set_expand_tabs(false);
        tab_bar.set_inverted(false);

        // Enable drag-and-drop for reordering tabs within the bar
        // but NOT to external targets (we handle that separately)
        tab_bar.set_extra_drag_preload(false);

        // Only add TabBar to container - TabView is hidden but still manages tabs
        container.append(&tab_bar);
        // TabView must be in widget tree for TabBar to work, but hidden
        container.append(&tab_view);

        // Add a welcome page
        let welcome = Self::create_welcome_tab();
        let welcome_page = tab_view.append(&welcome);
        welcome_page.set_title(&i18n("Welcome"));
        welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));

        let term_notebook = Self {
            container,
            tab_view,
            tab_bar,
            sessions: Rc::new(RefCell::new(HashMap::new())),
            on_page_closed: Rc::new(RefCell::new(None)),
            on_split_cleanup: Rc::new(RefCell::new(None)),
            terminals: Rc::new(RefCell::new(HashMap::new())),
            session_widgets: Rc::new(RefCell::new(HashMap::new())),
            automation_sessions: Rc::new(RefCell::new(HashMap::new())),
            session_info: Rc::new(RefCell::new(HashMap::new())),
            split_manager: Rc::new(RefCell::new(TabSplitManager::new())),
            session_tab_ids: Rc::new(RefCell::new(HashMap::new())),
            color_tabs_by_protocol: Rc::new(RefCell::new(false)),
            tab_group_manager: Rc::new(RefCell::new(TabGroupManager::new())),
        };

        term_notebook.setup_tab_view_signals();
        term_notebook.setup_tab_context_menu();
        term_notebook
    }

    /// Sets up TabView signals for close requests
    fn setup_tab_view_signals(&self) {
        let sessions = self.sessions.clone();
        let terminals = self.terminals.clone();
        let session_widgets = self.session_widgets.clone();
        let session_info = self.session_info.clone();
        let tab_view = self.tab_view.clone();
        let split_manager = self.split_manager.clone();
        let session_tab_ids = self.session_tab_ids.clone();
        let on_page_closed = self.on_page_closed.clone();
        let on_split_cleanup = self.on_split_cleanup.clone();

        // Handle create-window signal - we must connect this to prevent the default
        // behavior which causes CRITICAL warnings. Returning None cancels the tearoff.
        // Note: libadwaita will still show a CRITICAL warning, but this is unavoidable
        // without implementing multi-window support.
        self.tab_view.connect_create_window(|_| {
            // Log instead of letting libadwaita complain
            tracing::debug!("Tab tearoff attempted but not supported - cancelling");
            // Return None to cancel the operation
            // The CRITICAL warning from libadwaita is unavoidable
            None
        });

        // Handle close-page signal
        self.tab_view.connect_close_page(move |view, page| {
            // Find session ID for this page
            let (session_id, connection_id) = {
                let sessions_ref = sessions.borrow();
                let info_ref = session_info.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == page)
                    .map(|(id, _)| {
                        let conn_id = info_ref.get(id).map(|i| i.connection_id);
                        (*id, conn_id)
                    })
                    .unwrap_or((Uuid::nil(), None))
            };

            if !session_id.is_nil() {
                // Call the on_split_cleanup callback FIRST to clear split view panels
                // This must happen before on_page_closed to ensure proper cleanup
                if let Some(ref callback) = *on_split_cleanup.borrow() {
                    callback(session_id);
                }

                // Call the on_page_closed callback to update sidebar status
                if let Some(conn_id) = connection_id
                    && let Some(ref callback) = *on_page_closed.borrow()
                {
                    callback(session_id, conn_id);
                }

                // Clean up split layout for this session's tab
                // Requirement 3.4: Split_Container is destroyed when tab is closed
                if let Some(tab_id) = session_tab_ids.borrow_mut().remove(&session_id) {
                    split_manager.borrow_mut().remove(tab_id);
                }

                // Clean up session data
                sessions.borrow_mut().remove(&session_id);
                terminals.borrow_mut().remove(&session_id);

                // Disconnect embedded widgets before removing
                if let Some(widget_storage) = session_widgets.borrow_mut().remove(&session_id) {
                    match widget_storage {
                        SessionWidgetStorage::EmbeddedRdp(widget) => widget.disconnect(),
                        SessionWidgetStorage::EmbeddedSpice(widget) => widget.disconnect(),
                        SessionWidgetStorage::Vnc(widget) => widget.disconnect(),
                    }
                }

                session_info.borrow_mut().remove(&session_id);
            }

            // Confirm close
            view.close_page_finish(page, true);

            // If no more sessions, show welcome page
            if sessions.borrow().is_empty() && tab_view.n_pages() == 0 {
                let welcome = Self::create_welcome_tab();
                let welcome_page = tab_view.append(&welcome);
                welcome_page.set_title(&i18n("Welcome"));
                welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));
            }

            glib::Propagation::Stop
        });
    }

    /// Sets up the tab context menu with group management actions.
    ///
    /// The menu is shown on right-click via `adw::TabView::set_menu_model`.
    /// The `setup-menu` signal stores the target page so actions can find it.
    fn setup_tab_context_menu(&self) {
        // Build the GMenu model for tab context menu
        let menu = gio::Menu::new();

        let group_section = gio::Menu::new();
        group_section.append(Some(&i18n("Set Group...")), Some("tab.set-group"));
        group_section.append(Some(&i18n("Remove from Group")), Some("tab.remove-group"));
        menu.append_section(None, &group_section);

        let close_section = gio::Menu::new();
        close_section.append(Some(&i18n("Close Tab")), Some("tab.close"));
        menu.append_section(None, &close_section);

        self.tab_view.set_menu_model(Some(&menu));

        // Shared cell to store the page that was right-clicked
        let context_page: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));

        // When the context menu is about to show, store the target page
        let context_page_setup = context_page.clone();
        self.tab_view.connect_setup_menu(move |_tab_view, page| {
            *context_page_setup.borrow_mut() = page.cloned();
        });

        // Create action group
        let action_group = gio::SimpleActionGroup::new();

        // "Set Group..." action — shows an entry dialog
        let set_group_action = gio::SimpleAction::new("set-group", None);
        let context_page_set = context_page.clone();
        let session_info = self.session_info.clone();
        let sessions = self.sessions.clone();
        let tab_group_manager = self.tab_group_manager.clone();
        let split_manager = self.split_manager.clone();
        let session_tab_ids = self.session_tab_ids.clone();

        set_group_action.connect_activate(move |_, _| {
            let target_page = context_page_set.borrow().clone();
            let Some(target_page) = target_page else {
                return;
            };
            let session_id = {
                let sessions_ref = sessions.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == &target_page)
                    .map(|(id, _)| *id)
            };
            let Some(session_id) = session_id else {
                return;
            };

            // Build the group chooser dialog
            let dialog = adw::AlertDialog::builder()
                .heading(i18n("Set Tab Group"))
                .body(i18n("Enter a group name for this tab"))
                .build();

            let entry = gtk4::Entry::builder()
                .placeholder_text(i18n("e.g. Production, Staging"))
                .hexpand(true)
                .build();

            // Pre-fill with current group if any
            if let Some(info) = session_info.borrow().get(&session_id)
                && let Some(ref group) = info.tab_group
            {
                entry.set_text(group);
            }

            dialog.set_extra_child(Some(&entry));
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("apply", &i18n("Apply"));
            dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("apply"));
            dialog.set_close_response("cancel");

            // Enter key triggers "apply" via set_default_response above

            let session_info_clone = session_info.clone();
            let tab_group_manager_clone = tab_group_manager.clone();
            let sessions_clone = sessions.clone();
            let split_manager_clone = split_manager.clone();
            let session_tab_ids_clone = session_tab_ids.clone();

            dialog.connect_response(None, move |_dialog, response| {
                if response != "apply" {
                    return;
                }
                let group_name = entry.text().trim().to_string();
                if group_name.is_empty() {
                    return;
                }

                let color_index = tab_group_manager_clone
                    .borrow_mut()
                    .get_or_assign_color(&group_name);

                if let Some(info) = session_info_clone.borrow_mut().get_mut(&session_id) {
                    info.tab_group = Some(group_name.clone());
                    info.tab_color_index = Some(color_index);
                }

                // Apply visual indicator (check split color priority)
                let has_split_color = session_tab_ids_clone
                    .borrow()
                    .get(&session_id)
                    .and_then(|tab_id| split_manager_clone.borrow().get_tab_color(*tab_id))
                    .is_some();

                if !has_split_color
                    && let Some(page) = sessions_clone.borrow().get(&session_id)
                    && let Some((r, g, b)) = TabGroupManager::color_rgb(color_index)
                    && let Some(icon) = Self::create_protocol_color_icon(r, g, b, 16)
                {
                    page.set_indicator_icon(Some(&icon));
                    page.set_indicator_activatable(false);
                }

                tracing::debug!(
                    session_id = %session_id,
                    group = group_name,
                    color_index,
                    "Tab assigned to group via context menu"
                );
            });

            // Present the dialog
            if let Some(root) = target_page.child().root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                dialog.present(Some(window));
            }
        });
        action_group.add_action(&set_group_action);

        // "Remove from Group" action
        let remove_group_action = gio::SimpleAction::new("remove-group", None);
        let context_page_remove = context_page.clone();
        let session_info = self.session_info.clone();
        let sessions = self.sessions.clone();
        let color_tabs_by_protocol = self.color_tabs_by_protocol.clone();
        let split_manager = self.split_manager.clone();
        let session_tab_ids = self.session_tab_ids.clone();

        remove_group_action.connect_activate(move |_, _| {
            let target_page = context_page_remove.borrow().clone();
            let Some(target_page) = target_page else {
                return;
            };
            let session_id = {
                let sessions_ref = sessions.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == &target_page)
                    .map(|(id, _)| *id)
            };
            let Some(session_id) = session_id else {
                return;
            };

            // Clear group from session info
            let protocol = {
                let mut info_ref = session_info.borrow_mut();
                if let Some(info) = info_ref.get_mut(&session_id) {
                    info.tab_group = None;
                    info.tab_color_index = None;
                    Some(info.protocol.clone())
                } else {
                    None
                }
            };

            // Restore appropriate indicator
            let has_split_color = session_tab_ids
                .borrow()
                .get(&session_id)
                .and_then(|tab_id| split_manager.borrow().get_tab_color(*tab_id))
                .is_some();

            if !has_split_color && let Some(page) = sessions.borrow().get(&session_id) {
                if *color_tabs_by_protocol.borrow() {
                    if let Some(ref proto) = protocol {
                        let (r, g, b) = rustconn_core::get_protocol_color_rgb(proto);
                        if let Some(icon) = Self::create_protocol_color_icon(r, g, b, 16) {
                            page.set_indicator_icon(Some(&icon));
                            page.set_indicator_activatable(false);
                        }
                    }
                } else {
                    page.set_indicator_icon(gio::Icon::NONE);
                }
            }

            tracing::debug!(session_id = %session_id, "Tab removed from group via context menu");
        });
        action_group.add_action(&remove_group_action);

        // "Close Tab" action
        let close_action = gio::SimpleAction::new("close", None);
        let context_page_close = context_page;
        let tab_view_clone = self.tab_view.clone();
        close_action.connect_activate(move |_, _| {
            if let Some(page) = context_page_close.borrow().clone() {
                tab_view_clone.close_page(&page);
            }
        });
        action_group.add_action(&close_action);

        // Attach action group to the TabView widget
        self.tab_view
            .insert_action_group("tab", Some(&action_group));
    }

    /// Creates the welcome tab content - uses the full welcome screen with features
    fn create_welcome_tab() -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Use the full welcome content from SplitViewBridge for consistency
        let status_page = crate::split_view::SplitViewBridge::create_welcome_content_static();
        container.append(&status_page);
        container
    }

    /// Gets the icon name for a protocol
    fn get_protocol_icon(protocol: &str) -> &'static str {
        rustconn_core::get_protocol_icon_by_name(protocol)
    }

    /// Removes the welcome page if it exists
    fn remove_welcome_page(&self) {
        if self.sessions.borrow().is_empty() && self.tab_view.n_pages() > 0 {
            // Find and remove welcome page
            for i in 0..self.tab_view.n_pages() {
                let page = self.tab_view.nth_page(i);
                if page.title() == i18n("Welcome") {
                    self.tab_view.close_page(&page);
                    break;
                }
            }
        }
    }

    /// Creates a new terminal tab for an SSH session with default settings
    #[allow(dead_code)]
    pub fn create_terminal_tab(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
    ) -> Uuid {
        self.create_terminal_tab_with_settings(
            connection_id,
            title,
            protocol,
            automation,
            &rustconn_core::config::TerminalSettings::default(),
        )
    }

    /// Creates a new terminal tab with specific settings
    pub fn create_terminal_tab_with_settings(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
        settings: &rustconn_core::config::TerminalSettings,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        self.remove_welcome_page();

        let terminal = Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        // Setup automation if configured
        if let Some(cfg) = automation
            && !cfg.expect_rules.is_empty()
        {
            let mut triggers = Vec::new();
            for rule in &cfg.expect_rules {
                if !rule.enabled {
                    continue;
                }
                if let Ok(regex) = Regex::new(&rule.pattern) {
                    triggers.push(Trigger {
                        pattern: regex,
                        response: rule.response.clone(),
                        one_shot: true,
                    });
                }
            }

            if !triggers.is_empty() {
                let session = AutomationSession::new(terminal.clone(), triggers);
                self.automation_sessions
                    .borrow_mut()
                    .insert(session_id, session);
            }
        }

        // Apply user settings
        config::configure_terminal_with_settings(&terminal, settings);

        // Create scrolled window for terminal
        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&terminal)
            .build();

        // Create container for TabView page with terminal inside
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&scrolled);

        // Add page to TabView
        let page = self.tab_view.append(&container);
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            protocol,
        ))));
        page.set_tooltip(title);

        // Store session data
        self.sessions.borrow_mut().insert(session_id, page.clone());
        self.terminals.borrow_mut().insert(session_id, terminal);

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
            },
        );

        // Select the new page
        self.tab_view.set_selected_page(&page);

        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, protocol);
        }

        session_id
    }

    /// Creates a new VNC session tab
    pub fn create_vnc_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_vnc_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new VNC session tab with host information
    pub fn create_vnc_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        self.remove_welcome_page();

        let vnc_widget = Rc::new(VncSessionWidget::new());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(vnc_widget.widget());

        let page = self.tab_view.append(&container);
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(
            "video-joined-displays-symbolic",
        )));
        let tooltip = if host.is_empty() {
            title.to_string()
        } else {
            format!("{title}\n{host}")
        };
        page.set_tooltip(&tooltip);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::Vnc(vnc_widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "vnc".to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "vnc");
        }
        session_id
    }

    /// Creates a new SPICE session tab
    pub fn create_spice_session_tab(&self, connection_id: Uuid, title: &str) -> Uuid {
        self.create_spice_session_tab_with_host(connection_id, title, "")
    }

    /// Creates a new SPICE session tab with host information
    pub fn create_spice_session_tab_with_host(
        &self,
        connection_id: Uuid,
        title: &str,
        host: &str,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        self.remove_welcome_page();

        let spice_widget = Rc::new(EmbeddedSpiceWidget::new());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(spice_widget.widget());

        let page = self.tab_view.append(&container);
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(
            "preferences-desktop-remote-desktop-symbolic",
        )));
        let tooltip = if host.is_empty() {
            title.to_string()
        } else {
            format!("{title}\n{host}")
        };
        page.set_tooltip(&tooltip);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        self.session_widgets.borrow_mut().insert(
            session_id,
            SessionWidgetStorage::EmbeddedSpice(spice_widget),
        );

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "spice".to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "spice");
        }
        session_id
    }

    /// Adds an embedded RDP tab with the EmbeddedRdpWidget
    pub fn add_embedded_rdp_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<EmbeddedRdpWidget>,
    ) {
        self.remove_welcome_page();

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(widget.widget());

        let page = self.tab_view.append(&container);
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new("computer-symbolic")));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());
        self.session_widgets
            .borrow_mut()
            .insert(session_id, SessionWidgetStorage::EmbeddedRdp(widget));

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id,
                name: title.to_string(),
                protocol: "rdp".to_string(),
                is_embedded: true,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, "rdp");
        }
    }

    /// Adds an embedded session tab (for RDP/VNC external processes)
    pub fn add_embedded_session_tab(
        &self,
        session_id: Uuid,
        title: &str,
        protocol: &str,
        widget: &GtkBox,
    ) {
        self.remove_welcome_page();

        let page = self.tab_view.append(widget);
        page.set_title(title);
        page.set_icon(Some(&gio::ThemedIcon::new(Self::get_protocol_icon(
            protocol,
        ))));
        page.set_tooltip(title);

        self.sessions.borrow_mut().insert(session_id, page.clone());

        self.session_info.borrow_mut().insert(
            session_id,
            TerminalSession {
                id: session_id,
                connection_id: session_id,
                name: title.to_string(),
                protocol: protocol.to_string(),
                is_embedded: false,
                log_file: None,
                history_entry_id: None,
                tab_group: None,
                tab_color_index: None,
            },
        );

        self.tab_view.set_selected_page(&page);
        // Apply protocol color indicator if enabled
        if *self.color_tabs_by_protocol.borrow() {
            self.apply_protocol_color(session_id, protocol);
        }
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
    pub fn get_rdp_widget(&self, session_id: Uuid) -> Option<Rc<EmbeddedRdpWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::EmbeddedRdp(widget)) => Some(widget.clone()),
            _ => None,
        }
    }

    /// Queues a redraw for an RDP widget
    pub fn queue_rdp_redraw(&self, session_id: Uuid) {
        if let Some(widget) = self.get_rdp_widget(session_id) {
            widget.queue_draw();
        }
    }

    /// Gets the SPICE session widget for a session
    #[must_use]
    pub fn get_spice_widget(&self, session_id: Uuid) -> Option<Rc<EmbeddedSpiceWidget>> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::EmbeddedSpice(widget)) => Some(widget.clone()),
            _ => None,
        }
    }

    /// Gets the session widget (VNC) for a session
    #[must_use]
    #[allow(dead_code)]
    pub fn get_session_widget(&self, session_id: Uuid) -> Option<SessionWidget> {
        let widgets = self.session_widgets.borrow();
        if let Some(SessionWidgetStorage::Vnc(_)) = widgets.get(&session_id) {
            Some(SessionWidget::Vnc(VncSessionWidget::new()))
        } else {
            drop(widgets);
            if let Some(terminal) = self.terminals.borrow().get(&session_id) {
                Some(SessionWidget::Ssh(terminal.clone()))
            } else {
                None
            }
        }
    }

    /// Gets the GTK widget for a session (for display in split view)
    #[must_use]
    #[allow(dead_code)]
    pub fn get_session_display_widget(&self, session_id: Uuid) -> Option<Widget> {
        let widgets = self.session_widgets.borrow();
        if let Some(storage) = widgets.get(&session_id) {
            return match storage {
                SessionWidgetStorage::Vnc(widget) => Some(widget.widget().clone()),
                SessionWidgetStorage::EmbeddedRdp(widget) => Some(widget.widget().clone().upcast()),
                SessionWidgetStorage::EmbeddedSpice(widget) => {
                    Some(widget.widget().clone().upcast())
                }
            };
        }
        drop(widgets);

        self.terminals
            .borrow()
            .get(&session_id)
            .map(|t| t.clone().upcast())
    }

    /// Gets the session state for a VNC session
    #[must_use]
    #[allow(dead_code)]
    pub fn get_session_state(&self, session_id: Uuid) -> Option<SessionState> {
        let widgets = self.session_widgets.borrow();
        match widgets.get(&session_id) {
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.state()),
            _ => None,
        }
    }

    /// Spawns a command in the terminal
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

        let argv_gstr: Vec<glib::GString> = argv.iter().map(|s| glib::GString::from(*s)).collect();
        let argv_refs: Vec<&str> = argv_gstr.iter().map(gtk4::glib::GString::as_str).collect();

        // Inherit the current process environment so that child
        // processes see SSH_AUTH_SOCK, HOME, TERM, DISPLAY, etc.
        // Then override PATH with our extended version (Flatpak CLI
        // tools) and layer any caller-provided variables on top.
        let extended_path = rustconn_core::cli_download::get_extended_path();

        let mut env_vec: Vec<glib::GString> = Vec::new();

        // Start with the full parent environment
        for (key, value) in std::env::vars() {
            if key == "PATH" {
                // Replace PATH with our extended version
                env_vec.push(glib::GString::from(format!("PATH={extended_path}")));
            } else {
                env_vec.push(glib::GString::from(format!("{key}={value}")));
            }
        }

        // If PATH wasn't in the parent env, add it explicitly
        if std::env::var("PATH").is_err() {
            env_vec.push(glib::GString::from(format!("PATH={extended_path}")));
        }

        // Inject SSH agent env from OnceLock if RustConn started its
        // own agent (Rust 2024 forbids set_var, so the process env may
        // not contain the correct SSH_AUTH_SOCK).
        if let Some(agent_info) = rustconn_core::sftp::get_agent_info() {
            env_vec.retain(|e| !e.starts_with("SSH_AUTH_SOCK="));
            env_vec.push(glib::GString::from(format!(
                "SSH_AUTH_SOCK={}",
                agent_info.socket_path
            )));
            if let Some(ref pid) = agent_info.pid {
                env_vec.retain(|e| !e.starts_with("SSH_AGENT_PID="));
                env_vec.push(glib::GString::from(format!("SSH_AGENT_PID={pid}")));
            }
        }

        // Layer caller-provided variables (override parent values)
        if let Some(user_env) = envv {
            for e in user_env {
                // Remove any existing entry with the same key
                if let Some(eq_pos) = e.find('=') {
                    let key_prefix = &e[..=eq_pos];
                    env_vec.retain(|existing| !existing.starts_with(key_prefix));
                }
                env_vec.push(glib::GString::from(*e));
            }
        }

        let env_refs: Vec<&str> = env_vec.iter().map(gtk4::glib::GString::as_str).collect();

        terminal.spawn_async(
            PtyFlags::DEFAULT,
            working_directory,
            &argv_refs,
            &env_refs,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            gio::Cancellable::NONE,
            |result| {
                if let Err(e) = result {
                    tracing::error!(%e, "Failed to spawn command");
                }
            },
        );

        true
    }

    /// Spawns an SSH command in the terminal
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_ssh(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        username: Option<&str>,
        identity_file: Option<&str>,
        extra_args: &[&str],
        use_waypipe: bool,
    ) -> bool {
        let mut argv = if use_waypipe {
            vec!["waypipe", "ssh"]
        } else {
            vec!["ssh"]
        };

        let port_str;
        if port != 22 {
            port_str = port.to_string();
            argv.push("-p");
            argv.push(&port_str);
        }

        if let Some(key) = identity_file {
            argv.push("-i");
            argv.push(key);
        }

        argv.extend(extra_args);

        let destination = if let Some(user) = username {
            format!("{user}@{host}")
        } else {
            host.to_string()
        };
        argv.push(&destination);

        self.spawn_command(session_id, &argv, None, None)
    }

    /// Spawns a Telnet command in the terminal
    ///
    /// Supports configurable backspace/delete key behavior via VTE
    /// `EraseBinding`. Settings are applied directly on the terminal
    /// widget before spawning the telnet process.
    pub fn spawn_telnet(
        &self,
        session_id: Uuid,
        host: &str,
        port: u16,
        extra_args: &[&str],
        backspace_sends: rustconn_core::models::TelnetBackspaceSends,
        delete_sends: rustconn_core::models::TelnetDeleteSends,
    ) -> bool {
        use rustconn_core::models::{TelnetBackspaceSends, TelnetDeleteSends};
        use vte4::EraseBinding;

        // Apply keyboard bindings directly on the VTE terminal
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            match backspace_sends {
                TelnetBackspaceSends::Automatic => {
                    terminal.set_backspace_binding(EraseBinding::Auto);
                }
                TelnetBackspaceSends::Backspace => {
                    terminal.set_backspace_binding(EraseBinding::AsciiBackspace);
                }
                TelnetBackspaceSends::Delete => {
                    terminal.set_backspace_binding(EraseBinding::AsciiDelete);
                }
            }
            match delete_sends {
                TelnetDeleteSends::Automatic => {
                    terminal.set_delete_binding(EraseBinding::Auto);
                }
                TelnetDeleteSends::Backspace => {
                    terminal.set_delete_binding(EraseBinding::AsciiBackspace);
                }
                TelnetDeleteSends::Delete => {
                    terminal.set_delete_binding(EraseBinding::AsciiDelete);
                }
            }
        }

        // Spawn telnet directly — no shell wrapper needed
        let mut argv = vec!["telnet"];
        argv.extend(extra_args);
        argv.push(host);
        let port_str = port.to_string();
        argv.push(&port_str);
        self.spawn_command(session_id, &argv, None, None)
    }

    /// Spawns a serial connection using picocom in the terminal tab.
    ///
    /// Builds the picocom command from the `SerialConfig` and spawns it
    /// directly in the VTE terminal (no shell wrapper).
    pub fn spawn_serial(&self, session_id: Uuid, command: &[String]) -> bool {
        let argv: Vec<&str> = command.iter().map(String::as_str).collect();
        self.spawn_command(session_id, &argv, None, None)
    }

    /// Closes a terminal tab by session ID
    pub fn close_tab(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id).cloned() {
            self.tab_view.close_page(&page);
        }
    }

    /// Marks a tab as disconnected (changes indicator)
    pub fn mark_tab_disconnected(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
            page.set_indicator_activatable(false);
        }
    }

    /// Marks a tab as connected (removes indicator)
    pub fn mark_tab_connected(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);
        }
    }

    /// Sets a color indicator on a tab to show it's in a split pane
    /// Applies a colored left border to the tab's title in the TabBar
    pub fn set_tab_split_color(&self, session_id: Uuid, color_index: usize) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Remove any existing tab color classes from the page's child
            for (_, tab_class) in crate::split_view::SPLIT_PANE_COLORS {
                page.child().remove_css_class(tab_class);
            }
            // Remove old indicator classes
            for i in 0..6 {
                page.child()
                    .remove_css_class(&format!("split-indicator-{}", i));
            }

            // Add the new tab color class to the page's child
            let tab_class = crate::split_view::get_tab_color_class(color_index);
            page.child().add_css_class(tab_class);

            // Add indicator class for potential CSS styling
            let indicator_class = format!("split-indicator-{}", color_index);
            page.child().add_css_class(&indicator_class);

            // Create a colored circle icon for the indicator
            // This provides a visible colored indicator in the tab header
            if let Some(icon) = crate::split_view::create_colored_circle_icon(color_index, 16) {
                page.set_indicator_icon(Some(&icon));
            } else {
                // Fallback to symbolic icon if colored icon creation fails
                let icon = gio::ThemedIcon::new("media-record-symbolic");
                page.set_indicator_icon(Some(&icon));
            }
            page.set_indicator_activatable(false);
        }
    }

    /// Sets a color indicator on a tab using the new ColorId system.
    ///
    /// This method is used by the new split view system to show color indicators
    /// on tabs that contain split containers.
    ///
    /// # Requirements
    /// - 6.2: Tab header shows color indicator when tab contains Split_Container
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session UUID
    /// * `color_id` - The ColorId from the split layout model
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn set_tab_split_color_id(
        &self,
        session_id: Uuid,
        color_id: rustconn_core::split::ColorId,
    ) {
        self.set_tab_split_color(session_id, color_id.index() as usize);
    }

    /// Updates the tab color indicator based on the session's split state.
    ///
    /// This method checks if the session's tab has a split layout and updates
    /// the color indicator accordingly. If the tab is split, it shows the
    /// assigned color; otherwise, it clears the indicator.
    ///
    /// # Requirements
    /// - 6.2: Tab header shows color indicator when tab contains Split_Container
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session UUID
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn update_tab_color_indicator(&self, session_id: Uuid) {
        if let Some(color_index) = self.get_session_split_color(session_id) {
            self.set_tab_split_color(session_id, color_index);
        } else {
            self.clear_tab_split_color(session_id);
        }
    }

    /// Removes the split color indicator from a tab
    pub fn clear_tab_split_color(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);

            // Remove all tab color classes and indicator classes from the page's child
            let child = page.child();
            for (_, tab_class) in crate::split_view::SPLIT_PANE_COLORS {
                child.remove_css_class(tab_class);
            }
            // Remove indicator classes
            for i in 0..6 {
                child.remove_css_class(&format!("split-indicator-{}", i));
            }
        }
    }

    /// Sets whether tabs should be colored by protocol type
    pub fn set_color_tabs_by_protocol(&self, enabled: bool) {
        *self.color_tabs_by_protocol.borrow_mut() = enabled;
        // Apply or remove protocol colors on all existing sessions
        let sessions: Vec<(Uuid, String)> = self
            .session_info
            .borrow()
            .iter()
            .map(|(id, info)| (*id, info.protocol.clone()))
            .collect();
        for (session_id, protocol) in sessions {
            if enabled {
                self.apply_protocol_color(session_id, &protocol);
            } else {
                self.clear_protocol_color(session_id);
            }
        }
    }

    /// Applies protocol-based color indicator to a tab
    fn apply_protocol_color(&self, session_id: Uuid, protocol: &str) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Don't override split colors — split takes priority
            if self.get_session_split_color(session_id).is_some() {
                return;
            }
            // Don't override group colors — group takes priority over protocol
            if self
                .session_info
                .borrow()
                .get(&session_id)
                .and_then(|i| i.tab_group.as_ref())
                .is_some()
            {
                return;
            }
            let (r, g, b) = rustconn_core::get_protocol_color_rgb(protocol);
            if let Some(icon) = Self::create_protocol_color_icon(r, g, b, 16) {
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_activatable(false);
            }
        }
    }

    /// Removes protocol color indicator from a tab
    fn clear_protocol_color(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Don't clear if split color is active
            if self.get_session_split_color(session_id).is_some() {
                return;
            }
            // Don't clear if group color is active
            if self
                .session_info
                .borrow()
                .get(&session_id)
                .and_then(|i| i.tab_group.as_ref())
                .is_some()
            {
                return;
            }
            page.set_indicator_icon(gio::Icon::NONE);
        }
    }

    /// Creates a colored circle icon for protocol tab indicators
    fn create_protocol_color_icon(r: u8, g: u8, b: u8, size: u32) -> Option<gio::Icon> {
        // Reuse the same circle-drawing logic as split colors
        let mut rgba_data = vec![0u8; (size * size * 4) as usize];
        let center = size as f32 / 2.0;
        let radius = center - 1.0;

        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let distance = dx.hypot(dy);
                let idx = ((y * size + x) * 4) as usize;

                if distance <= radius {
                    let alpha = if distance > radius - 1.0 {
                        ((radius - distance + 1.0) * 255.0) as u8
                    } else {
                        255
                    };
                    rgba_data[idx] = r;
                    rgba_data[idx + 1] = g;
                    rgba_data[idx + 2] = b;
                    rgba_data[idx + 3] = alpha;
                }
            }
        }

        let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
            &glib::Bytes::from(&rgba_data),
            gtk4::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            size as i32,
            size as i32,
            (size * 4) as i32,
        );
        let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
        Some(texture.upcast::<gio::Icon>())
    }

    /// Gets the terminal widget for a session
    #[must_use]
    pub fn get_terminal(&self, session_id: Uuid) -> Option<Terminal> {
        self.terminals.borrow().get(&session_id).cloned()
    }

    /// Gets the cursor row of a terminal session
    pub fn get_terminal_cursor_row(&self, session_id: Uuid) -> Option<i64> {
        self.get_terminal(session_id).map(|t| t.cursor_position().0)
    }

    /// Gets session info for a session
    #[must_use]
    pub fn get_session_info(&self, session_id: Uuid) -> Option<TerminalSession> {
        self.session_info.borrow().get(&session_id).cloned()
    }

    /// Gets the page container widget for a session
    ///
    /// Returns the `GtkBox` that holds the terminal scrolled window.
    /// Used by monitoring to prepend the monitoring bar above the terminal.
    #[must_use]
    pub fn get_session_container(&self, session_id: Uuid) -> Option<GtkBox> {
        let sessions = self.sessions.borrow();
        let page = sessions.get(&session_id)?;
        page.child().downcast::<GtkBox>().ok()
    }

    /// Gets all active sessions
    #[must_use]
    #[allow(dead_code)]
    pub fn get_all_sessions(&self) -> Vec<TerminalSession> {
        self.session_info.borrow().values().cloned().collect()
    }

    /// Sets the log file path for a session
    pub fn set_log_file(&self, session_id: Uuid, log_file: PathBuf) {
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.log_file = Some(log_file);
        }
    }

    /// Sets the history entry ID for a session
    pub fn set_history_entry_id(&self, session_id: Uuid, history_entry_id: Uuid) {
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.history_entry_id = Some(history_entry_id);
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
        let selected_page = self.tab_view.selected_page()?;
        let sessions = self.sessions.borrow();

        for (session_id, page) in sessions.iter() {
            if page == &selected_page {
                return self.terminals.borrow().get(session_id).cloned();
            }
        }
        None
    }

    /// Gets the session ID for the currently active tab
    #[must_use]
    pub fn get_active_session_id(&self) -> Option<Uuid> {
        let selected_page = self.tab_view.selected_page()?;
        let sessions = self.sessions.borrow();

        for (session_id, page) in sessions.iter() {
            if page == &selected_page {
                return Some(*session_id);
            }
        }
        None
    }

    /// Gets the session ID for a specific page number
    #[must_use]
    pub fn get_session_id_for_page(&self, page_num: u32) -> Option<Uuid> {
        if page_num >= self.tab_view.n_pages() as u32 {
            return None;
        }
        let page = self.tab_view.nth_page(page_num as i32);
        let sessions = self.sessions.borrow();

        for (session_id, stored_page) in sessions.iter() {
            if stored_page == &page {
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

    /// Returns the TabView widget
    #[must_use]
    pub fn tab_view(&self) -> &adw::TabView {
        &self.tab_view
    }

    /// Returns the number of open tabs
    #[must_use]
    #[allow(dead_code)]
    pub fn tab_count(&self) -> u32 {
        self.tab_view.n_pages() as u32
    }

    /// Returns the number of active sessions (excluding Welcome tab)
    #[must_use]
    #[allow(dead_code)]
    pub fn session_count(&self) -> usize {
        self.sessions.borrow().len()
    }

    /// Switches to a specific tab by session ID
    pub fn switch_to_tab(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id).cloned() {
            self.tab_view.set_selected_page(&page);
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

    /// Connects a callback for user input (commit signal - data sent to PTY)
    pub fn connect_commit<F>(&self, session_id: Uuid, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.connect_commit(move |_terminal, text, _size| {
                callback(text);
            });
        }
    }

    /// Gets the current terminal text content for transcript logging
    #[must_use]
    pub fn get_terminal_text(&self, session_id: Uuid) -> Option<String> {
        self.get_terminal(session_id).map(|terminal| {
            let row_count = terminal.row_count();
            let col_count = terminal.column_count();
            let (text, _len) =
                terminal.text_range_format(vte4::Format::Text, 0, 0, row_count, col_count);
            text.map_or_else(String::new, |g| g.to_string())
        })
    }

    /// Applies terminal settings to all existing terminals
    pub fn apply_settings(&self, settings: &rustconn_core::config::TerminalSettings) {
        let terminals = self.terminals.borrow();
        for terminal in terminals.values() {
            config::configure_terminal_with_settings(terminal, settings);
        }
    }

    /// Moves terminal back to its TabView page container
    /// Call this when session exits split view and returns to TabView display
    pub fn reparent_terminal_to_tab(&self, session_id: Uuid) {
        let Some(terminal) = self.terminals.borrow().get(&session_id).cloned() else {
            return;
        };
        let Some(page) = self.sessions.borrow().get(&session_id).cloned() else {
            return;
        };

        // Get the page's child (container box)
        let child = page.child();
        let Some(container) = child.downcast_ref::<GtkBox>() else {
            return;
        };

        // Check if terminal is already in this container (via scrolled window)
        if let Some(parent) = terminal.parent()
            && let Some(grandparent) = parent.parent()
            && grandparent == child
        {
            return; // Already in place
        }

        // Remove terminal from current parent (if any)
        if let Some(parent) = terminal.parent() {
            if let Some(scrolled) = parent.downcast_ref::<gtk4::ScrolledWindow>() {
                scrolled.set_child(None::<&Widget>);
            } else if let Some(box_widget) = parent.downcast_ref::<GtkBox>() {
                box_widget.remove(&terminal);
            }
        }

        // Create new scrolled window and add terminal
        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&terminal)
            .build();

        // Clear container and add scrolled window
        while let Some(existing) = container.first_child() {
            container.remove(&existing);
        }
        container.append(&scrolled);
        terminal.set_visible(true);
    }

    /// Shows TabView content area (for RDP/VNC/SPICE sessions)
    /// Call this when switching to a non-SSH session that displays in TabView
    pub fn show_tab_view_content(&self) {
        self.tab_view.set_visible(true);
        self.tab_view.set_vexpand(true);
    }

    /// Hides TabView content area (for SSH sessions that display in split_view)
    /// Call this when switching to an SSH session
    pub fn hide_tab_view_content(&self) {
        self.tab_view.set_visible(false);
        self.tab_view.set_vexpand(false);
    }

    /// Returns whether the TabView content is currently visible
    #[must_use]
    #[allow(dead_code)]
    pub fn is_tab_view_content_visible(&self) -> bool {
        self.tab_view.is_visible()
    }

    // ========================================================================
    // Split Layout Management
    // ========================================================================

    /// Returns a reference to the split manager.
    ///
    /// The split manager handles tab-scoped split layouts, allowing each tab
    /// to have its own independent panel configuration.
    ///
    /// # Requirements
    /// - 3.1: Each Root_Tab maintains its own Split_Container
    #[must_use]
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn split_manager(&self) -> Rc<RefCell<TabSplitManager>> {
        Rc::clone(&self.split_manager)
    }

    /// Gets or creates a TabId for a session.
    ///
    /// This associates a session with a TabId for split layout tracking.
    /// If the session doesn't have a TabId yet, one is created.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session UUID
    ///
    /// # Returns
    ///
    /// The TabId associated with this session
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn get_or_create_tab_id(&self, session_id: Uuid) -> TabId {
        let mut tab_ids = self.session_tab_ids.borrow_mut();
        *tab_ids.entry(session_id).or_default()
    }

    /// Gets the TabId for a session if it exists.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session UUID
    ///
    /// # Returns
    ///
    /// The TabId if the session has one, None otherwise
    #[must_use]
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn get_tab_id(&self, session_id: Uuid) -> Option<TabId> {
        self.session_tab_ids.borrow().get(&session_id).copied()
    }

    /// Checks if a session's tab has a split layout.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session UUID
    ///
    /// # Returns
    ///
    /// `true` if the session's tab has splits, `false` otherwise
    #[must_use]
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn is_session_split(&self, session_id: Uuid) -> bool {
        if let Some(tab_id) = self.get_tab_id(session_id) {
            self.split_manager.borrow().is_split(tab_id)
        } else {
            false
        }
    }

    /// Gets the color for a session's split container.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session UUID
    ///
    /// # Returns
    ///
    /// The color index if the session has a split container with a color
    #[must_use]
    #[allow(dead_code)] // Will be used in window integration tasks
    pub fn get_session_split_color(&self, session_id: Uuid) -> Option<usize> {
        if let Some(tab_id) = self.get_tab_id(session_id) {
            self.split_manager
                .borrow()
                .get_tab_color(tab_id)
                .map(|c| c.index() as usize)
        } else {
            None
        }
    }

    // ========================================================================
    // Tab Group Management
    // ========================================================================

    /// Assigns a session to a named tab group.
    ///
    /// The group is assigned a color from the palette. The tab indicator is
    /// updated to show the group color (unless a split color is active).
    #[allow(dead_code)] // Public API for window-level tab group operations
    pub fn set_tab_group(&self, session_id: Uuid, group_name: &str) {
        let color_index = self
            .tab_group_manager
            .borrow_mut()
            .get_or_assign_color(group_name);

        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.tab_group = Some(group_name.to_owned());
            info.tab_color_index = Some(color_index);
        }

        // Apply visual indicator (group color takes priority over protocol color,
        // but split color still takes priority over group color)
        if self.get_session_split_color(session_id).is_none() {
            self.apply_group_color(session_id, color_index);
        }

        tracing::debug!(session_id = %session_id, group = group_name, color_index, "Tab assigned to group");
    }

    /// Removes a session from its tab group.
    #[allow(dead_code)] // Public API for window-level tab group operations
    pub fn remove_tab_group(&self, session_id: Uuid) {
        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.tab_group = None;
            info.tab_color_index = None;
        }

        // Restore protocol color or clear indicator
        if self.get_session_split_color(session_id).is_none() {
            if *self.color_tabs_by_protocol.borrow() {
                if let Some(protocol) = self
                    .session_info
                    .borrow()
                    .get(&session_id)
                    .map(|i| i.protocol.clone())
                {
                    self.apply_protocol_color(session_id, &protocol);
                }
            } else {
                self.clear_group_color(session_id);
            }
        }

        tracing::debug!(session_id = %session_id, "Tab removed from group");
    }

    /// Returns the group name for a session, if any.
    #[must_use]
    #[allow(dead_code)] // Public API for window-level tab group operations
    pub fn get_tab_group(&self, session_id: Uuid) -> Option<String> {
        self.session_info
            .borrow()
            .get(&session_id)
            .and_then(|i| i.tab_group.clone())
    }

    /// Returns all known group names from the tab group manager.
    #[must_use]
    #[allow(dead_code)] // Public API for window-level tab group operations
    pub fn known_group_names(&self) -> Vec<String> {
        self.tab_group_manager.borrow().group_names()
    }

    /// Applies a group color indicator to a tab.
    fn apply_group_color(&self, session_id: Uuid, color_index: usize) {
        if let Some(page) = self.sessions.borrow().get(&session_id)
            && let Some((r, g, b)) = TabGroupManager::color_rgb(color_index)
            && let Some(icon) = Self::create_protocol_color_icon(r, g, b, 16)
        {
            page.set_indicator_icon(Some(&icon));
            page.set_indicator_activatable(false);
        }
    }

    /// Clears a group color indicator from a tab.
    fn clear_group_color(&self, session_id: Uuid) {
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);
        }
    }

    /// Sets the callback to be invoked when a page is closed.
    ///
    /// The callback receives the session ID and connection ID of the closed page.
    /// This is used to update the sidebar status when SSH tabs are closed via TabView.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that takes (session_id, connection_id) as parameters
    pub fn set_on_page_closed<F>(&self, callback: F)
    where
        F: Fn(Uuid, Uuid) + 'static,
    {
        *self.on_page_closed.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback to be invoked for split view cleanup when a page is about to close.
    ///
    /// The callback receives the session ID of the page being closed.
    /// This is used to clear the session from split view panels before the tab is closed.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that takes session_id as parameter
    pub fn set_on_split_cleanup<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_split_cleanup.borrow_mut() = Some(Box::new(callback));
    }
}

impl Default for TerminalNotebook {
    fn default() -> Self {
        Self::new()
    }
}
