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
use gtk4::{gio, glib, Box as GtkBox, Orientation, Widget};
use libadwaita as adw;
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
use crate::session::{SessionState, SessionWidget, VncSessionWidget};
use crate::split_view::TabSplitManager;
use rustconn_core::split::TabId;

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
        welcome_page.set_title("Welcome");
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
        };

        term_notebook.setup_tab_view_signals();
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
                if let Some(conn_id) = connection_id {
                    if let Some(ref callback) = *on_page_closed.borrow() {
                        callback(session_id, conn_id);
                    }
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
                welcome_page.set_title("Welcome");
                welcome_page.set_icon(Some(&gio::ThemedIcon::new("go-home-symbolic")));
            }

            glib::Propagation::Stop
        });
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
        // Handle zerotrust:provider format
        if let Some(provider) = protocol.strip_prefix("zerotrust:") {
            return match provider {
                "aws" | "aws_ssm" => "network-workgroup-symbolic",
                "gcloud" | "gcp_iap" => "weather-overcast-symbolic",
                "azure" | "azure_bastion" => "weather-few-clouds-symbolic",
                "azure_ssh" => "weather-showers-symbolic",
                "oci" | "oci_bastion" => "drive-harddisk-symbolic",
                "cloudflare" | "cloudflare_access" => "security-high-symbolic",
                "teleport" => "emblem-system-symbolic",
                "tailscale" | "tailscale_ssh" => "network-vpn-symbolic",
                "boundary" => "dialog-password-symbolic",
                "generic" => "system-run-symbolic",
                _ => "folder-remote-symbolic",
            };
        }

        match protocol.to_lowercase().as_str() {
            "ssh" => "network-server-symbolic",
            "rdp" => "computer-symbolic",
            "vnc" => "video-display-symbolic",
            "spice" => "video-x-generic-symbolic",
            "zerotrust" => "folder-remote-symbolic",
            _ => "network-server-symbolic",
        }
    }

    /// Removes the welcome page if it exists
    fn remove_welcome_page(&self) {
        if self.sessions.borrow().is_empty() && self.tab_view.n_pages() > 0 {
            // Find and remove welcome page
            for i in 0..self.tab_view.n_pages() {
                let page = self.tab_view.nth_page(i);
                if page.title() == "Welcome" {
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
        if let Some(cfg) = automation {
            if !cfg.expect_rules.is_empty() {
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
            },
        );

        // Select the new page
        self.tab_view.set_selected_page(&page);

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
        page.set_icon(Some(&gio::ThemedIcon::new("video-display-symbolic")));
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
            },
        );

        self.tab_view.set_selected_page(&page);
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
        page.set_icon(Some(&gio::ThemedIcon::new("video-x-generic-symbolic")));
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
            },
        );

        self.tab_view.set_selected_page(&page);
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
            },
        );

        self.tab_view.set_selected_page(&page);
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
            },
        );

        self.tab_view.set_selected_page(&page);
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

        let envv_gstr: Option<Vec<glib::GString>> =
            envv.map(|e| e.iter().map(|s| glib::GString::from(*s)).collect());
        let envv_refs: Option<Vec<&str>> = envv_gstr
            .as_ref()
            .map(|e| e.iter().map(gtk4::glib::GString::as_str).collect());

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
                if let Err(e) = result {
                    eprintln!("Failed to spawn command: {e}");
                }
            },
        );

        true
    }

    /// Spawns an SSH command in the terminal
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
        if let Some(parent) = terminal.parent() {
            if let Some(grandparent) = parent.parent() {
                if grandparent == child {
                    return; // Already in place
                }
            }
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
