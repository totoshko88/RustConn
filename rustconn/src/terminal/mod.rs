//! Terminal notebook area
//!
//! This module provides the tabbed terminal interface using VTE4
//! for SSH sessions and native GTK widgets for VNC/RDP/SPICE connections.
//!
//! # Module Structure
//!
//! - `types` - Data structures for sessions and tabs
//! - `config` - Terminal appearance and behavior configuration
//! - `tabs` - Tab creation and management

mod config;
mod tabs;
mod types;

pub use types::{SessionWidgetStorage, TabDisplayMode, TabLabelWidgets, TerminalSession};

use gtk4::prelude::*;
use gtk4::{
    gio, glib, Box as GtkBox, Label, MenuButton, Notebook, Orientation, Popover, ScrolledWindow,
    Widget,
};
use std::cell::{Cell, RefCell};
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
use regex::Regex;
use rustconn_core::models::AutomationConfig;

/// Terminal notebook widget for managing multiple terminal sessions
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle
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
    /// Map of session IDs to automation sessions
    automation_sessions: Rc<RefCell<HashMap<Uuid, AutomationSession>>>,
    /// Session metadata
    session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    /// Current tab display mode
    display_mode: Rc<Cell<TabDisplayMode>>,
    /// Map of session IDs to their tab label widgets (for updating display mode)
    tab_labels: Rc<RefCell<HashMap<Uuid, TabLabelWidgets>>>,
}

impl TerminalNotebook {
    /// Creates a new terminal notebook
    #[must_use]
    pub fn new() -> Self {
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
            automation_sessions: Rc::new(RefCell::new(HashMap::new())),
            session_info: Rc::new(RefCell::new(HashMap::new())),
            display_mode,
            tab_labels,
        };

        term_notebook.setup_adaptive_tabs();
        term_notebook
    }

    /// Sets up the adaptive tab display handler
    fn setup_adaptive_tabs(&self) {
        let notebook = self.notebook.clone();
        let display_mode = self.display_mode.clone();
        let tab_labels = self.tab_labels.clone();
        let overflow_button = self.overflow_button.clone();

        notebook.connect_notify_local(Some("width-request"), move |nb, _| {
            tabs::update_tab_display_mode(nb, &display_mode, &tab_labels, &overflow_button);
        });

        let display_mode2 = self.display_mode.clone();
        let tab_labels2 = self.tab_labels.clone();
        let overflow_button2 = self.overflow_button.clone();
        let notebook2 = self.notebook.clone();

        self.notebook.connect_page_added(move |_, _, _| {
            tabs::update_tab_display_mode(
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
            tabs::update_tab_display_mode(
                &notebook3,
                &display_mode3,
                &tab_labels3,
                &overflow_button3,
            );
        });
    }

    /// Creates the welcome tab content
    fn create_welcome_tab() -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 0);
        let spacer = gtk4::DrawingArea::new();
        spacer.set_content_width(1);
        spacer.set_content_height(1);
        container.append(&spacer);
        container
    }

    /// Creates a new terminal tab for an SSH session
    pub fn create_terminal_tab(
        &self,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        automation: Option<&AutomationConfig>,
    ) -> Uuid {
        let session_id = Uuid::new_v4();
        let is_first_session = self.sessions.borrow().is_empty();

        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

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

        config::configure_terminal(&terminal);

        let placeholder = GtkBox::new(Orientation::Vertical, 0);
        let spacer = gtk4::DrawingArea::new();
        spacer.set_content_width(1);
        spacer.set_content_height(1);
        placeholder.append(&spacer);

        let tab_label = tabs::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            protocol,
            "",
            &self.tab_labels,
            &self.overflow_box,
        );

        let page_num = self.notebook.append_page(&placeholder, Some(&tab_label));

        self.sessions.borrow_mut().insert(session_id, page_num);
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

        self.notebook.set_tab_reorderable(&placeholder, true);
        self.notebook.set_current_page(Some(page_num));

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
        let is_first_session = self.sessions.borrow().is_empty();

        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        let vnc_widget = Rc::new(VncSessionWidget::new());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(vnc_widget.widget());

        let tab_label = tabs::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "vnc",
            host,
            &self.tab_labels,
            &self.overflow_box,
        );

        let page_num = self.notebook.append_page(&container, Some(&tab_label));

        self.sessions.borrow_mut().insert(session_id, page_num);
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

        self.notebook.set_tab_reorderable(&container, true);
        self.notebook.set_current_page(Some(page_num));

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
        let is_first_session = self.sessions.borrow().is_empty();

        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        let spice_widget = Rc::new(EmbeddedSpiceWidget::new());

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(spice_widget.widget());

        let tab_label = tabs::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "spice",
            host,
            &self.tab_labels,
            &self.overflow_box,
        );

        let page_num = self.notebook.append_page(&container, Some(&tab_label));

        self.sessions.borrow_mut().insert(session_id, page_num);
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

        self.notebook.set_tab_reorderable(&container, true);
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
        let page_num = self.sessions.borrow_mut().remove(&session_id);

        self.terminals.borrow_mut().remove(&session_id);
        self.session_widgets.borrow_mut().remove(&session_id);
        self.session_info.borrow_mut().remove(&session_id);

        self.tab_labels.borrow_mut().remove(&session_id);
        tabs::remove_from_overflow_menu(&self.overflow_box, session_id);

        if let Some(page_num) = page_num {
            self.notebook.remove_page(Some(page_num));

            let mut sessions = self.sessions.borrow_mut();
            for (_, num) in sessions.iter_mut() {
                if *num > page_num {
                    *num -= 1;
                }
            }
            let is_empty = sessions.is_empty();
            drop(sessions);

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
        self.get_terminal(session_id).map(|t| t.cursor_position().0)
    }

    /// Gets session info for a session
    #[must_use]
    pub fn get_session_info(&self, session_id: Uuid) -> Option<TerminalSession> {
        self.session_info.borrow().get(&session_id).cloned()
    }

    /// Gets all active sessions
    ///
    /// Note: Part of session restore API - used to collect sessions for saving.
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
        let current_page = self.notebook.current_page()?;
        let sessions = self.sessions.borrow();

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
    #[allow(dead_code)]
    pub fn session_count(&self) -> usize {
        self.sessions.borrow().len()
    }

    /// Sets vexpand for all notebook page contents
    #[allow(dead_code)]
    pub fn set_pages_vexpand(&self, _expand: bool) {
        // Don't modify individual page vexpand - it causes issues
    }

    /// Shows only the specified page content, hides all others
    #[allow(dead_code)]
    pub fn show_only_current_page(&self) {
        // No-op - hiding pages causes RDP disconnection issues
    }

    /// Shows all page contents (for VNC/RDP/SPICE mode)
    #[allow(dead_code)]
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
            // Get terminal dimensions for text range
            let row_count = terminal.row_count();
            let col_count = terminal.column_count();

            // Use text_range_format to get all visible text
            // Start from row 0, col 0 to end row, end col
            let (text, _len) =
                terminal.text_range_format(vte4::Format::Text, 0, 0, row_count, col_count);
            text.map_or_else(String::new, |g| g.to_string())
        })
    }

    /// Adds an embedded session tab (for RDP/VNC external processes)
    pub fn add_embedded_session_tab(
        &self,
        session_id: Uuid,
        title: &str,
        protocol: &str,
        widget: &GtkBox,
    ) {
        let is_first_session = self.sessions.borrow().is_empty();

        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        let tab_label = tabs::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            protocol,
            "",
            &self.tab_labels,
            &self.overflow_box,
        );

        let page_num = self.notebook.append_page(widget, Some(&tab_label));

        self.sessions.borrow_mut().insert(session_id, page_num);

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

        self.notebook.set_tab_reorderable(widget, true);
        self.notebook.set_current_page(Some(page_num));
    }

    /// Adds an embedded RDP tab with the EmbeddedRdpWidget
    pub fn add_embedded_rdp_tab(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        widget: Rc<EmbeddedRdpWidget>,
    ) {
        let is_first_session = self.sessions.borrow().is_empty();

        if is_first_session && self.notebook.n_pages() > 0 {
            self.notebook.remove_page(Some(0));
        }

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(widget.widget());

        let tab_label = tabs::create_tab_label_with_protocol(
            title,
            session_id,
            &self.notebook,
            &self.sessions,
            "rdp",
            "",
            &self.tab_labels,
            &self.overflow_box,
        );

        let page_num = self.notebook.append_page(&container, Some(&tab_label));

        self.sessions.borrow_mut().insert(session_id, page_num);

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

        self.notebook.set_tab_reorderable(&container, true);
        self.notebook.set_current_page(Some(page_num));
    }

    /// Hides content of all notebook pages except the specified one
    pub fn hide_all_page_content_except(&self, except_page: Option<u32>) {
        let n_pages = self.notebook.n_pages();
        for page_num in 0..n_pages {
            if let Some(page_widget) = self.notebook.nth_page(Some(page_num)) {
                let should_show = except_page == Some(page_num);
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
    pub fn show_page_content(&self, page_num: u32) {
        if let Some(page_widget) = self.notebook.nth_page(Some(page_num)) {
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
    #[allow(dead_code)]
    pub fn hide_all_page_content(&self) {
        let n_pages = self.notebook.n_pages();
        for page_num in 0..n_pages {
            if let Some(page_widget) = self.notebook.nth_page(Some(page_num)) {
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
