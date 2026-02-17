//! Connection Dashboard
//!
//! This module provides a dashboard view for monitoring active sessions,
//! displaying session statistics, and providing quick actions for session management.
//!
//! **Validates: Requirements 13.1, 13.2, 13.3, 13.5**

use chrono::{DateTime, Duration, Utc};
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, FlowBox, Frame, Label, Orientation, PolicyType,
    ScrolledWindow, SelectionMode, StringList,
};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use rustconn_core::session::{Session, SessionState};

/// Session statistics for dashboard display
/// **Validates: Requirements 13.2**
#[derive(Debug, Clone)]
pub struct SessionStats {
    /// Session ID
    pub session_id: Uuid,
    /// Connection ID
    pub connection_id: Uuid,
    /// Connection name
    pub connection_name: String,
    /// Protocol (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Session state
    pub state: SessionState,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// Bytes sent (tracked from terminal output)
    pub bytes_sent: u64,
    /// Bytes received (tracked from terminal input)
    pub bytes_received: u64,
    /// Host address
    pub host: String,
    /// Group ID (if any)
    pub group_id: Option<Uuid>,
}

impl SessionStats {
    /// Updates the bytes sent counter
    pub fn add_bytes_sent(&mut self, bytes: u64) {
        self.bytes_sent = self.bytes_sent.saturating_add(bytes);
    }

    /// Updates the bytes received counter
    pub fn add_bytes_received(&mut self, bytes: u64) {
        self.bytes_received = self.bytes_received.saturating_add(bytes);
    }
}

impl SessionStats {
    /// Creates session stats from a Session
    #[must_use]
    pub fn from_session(session: &Session, host: String, group_id: Option<Uuid>) -> Self {
        Self {
            session_id: session.id,
            connection_id: session.connection_id,
            connection_name: session.connection_name.clone(),
            protocol: session.protocol.clone(),
            state: session.state,
            started_at: session.started_at,
            bytes_sent: 0,
            bytes_received: 0,
            host,
            group_id,
        }
    }

    /// Returns the connection duration
    #[must_use]
    pub fn duration(&self) -> Duration {
        Utc::now().signed_duration_since(self.started_at)
    }

    /// Formats the duration as a human-readable string
    #[must_use]
    pub fn format_duration(&self) -> String {
        let duration = self.duration();
        let total_seconds = duration.num_seconds();

        if total_seconds < 0 {
            return "0s".to_string();
        }

        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{hours}h {minutes}m {seconds}s")
        } else if minutes > 0 {
            format!("{minutes}m {seconds}s")
        } else {
            format!("{seconds}s")
        }
    }

    /// Formats bytes as human-readable string
    #[must_use]
    pub fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB", bytes as f64 / KB as f64)
        } else {
            format!("{bytes} B")
        }
    }

    /// Returns the state as a display string
    #[must_use]
    pub const fn state_display(&self) -> &'static str {
        match self.state {
            SessionState::Starting => "Starting",
            SessionState::Active => "Connected",
            SessionState::Disconnecting => "Disconnecting",
            SessionState::Terminated => "Disconnected",
            SessionState::Error => "Error",
        }
    }

    /// Returns the CSS class for the state indicator
    #[must_use]
    pub const fn state_css_class(&self) -> &'static str {
        match self.state {
            SessionState::Starting => "session-starting",
            SessionState::Active => "session-active",
            SessionState::Disconnecting => "session-disconnecting",
            SessionState::Terminated => "session-terminated",
            SessionState::Error => "session-error",
        }
    }
}

/// Dashboard filter criteria
/// **Validates: Requirements 13.5**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardFilter {
    /// Filter by protocol (None = all protocols)
    pub protocol: Option<String>,
    /// Filter by group ID (None = all groups)
    pub group_id: Option<Uuid>,
    /// Filter by status (None = all statuses)
    pub status: Option<SessionState>,
}

impl DashboardFilter {
    /// Creates a new empty filter (shows all sessions)
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the protocol filter
    #[must_use]
    pub fn with_protocol(mut self, protocol: Option<String>) -> Self {
        self.protocol = protocol;
        self
    }

    /// Sets the group filter
    #[must_use]
    pub fn with_group(mut self, group_id: Option<Uuid>) -> Self {
        self.group_id = group_id;
        self
    }

    /// Sets the status filter
    #[must_use]
    pub fn with_status(mut self, status: Option<SessionState>) -> Self {
        self.status = status;
        self
    }

    /// Checks if a session matches this filter
    /// **Validates: Requirements 13.5**
    #[must_use]
    pub fn matches(&self, stats: &SessionStats) -> bool {
        // Check protocol filter
        if let Some(ref protocol) = self.protocol {
            if &stats.protocol != protocol {
                return false;
            }
        }

        // Check group filter
        if let Some(group_id) = self.group_id {
            if stats.group_id != Some(group_id) {
                return false;
            }
        }

        // Check status filter
        if let Some(status) = self.status {
            if stats.state != status {
                return false;
            }
        }

        true
    }

    /// Filters a list of session stats
    #[must_use]
    pub fn apply(&self, sessions: &[SessionStats]) -> Vec<SessionStats> {
        sessions
            .iter()
            .filter(|s| self.matches(s))
            .cloned()
            .collect()
    }
}

/// Callback type for session focus action
pub type FocusCallback = Box<dyn Fn(Uuid)>;
/// Callback type for session disconnect action
pub type DisconnectCallback = Box<dyn Fn(Uuid)>;

/// Connection Dashboard widget
/// **Validates: Requirements 13.1**
pub struct ConnectionDashboard {
    /// Main container widget
    container: GtkBox,
    /// Flow box for session cards
    flow_box: FlowBox,
    /// Current filter
    filter: Rc<RefCell<DashboardFilter>>,
    /// Protocol filter dropdown
    protocol_filter: DropDown,
    /// Status filter dropdown
    status_filter: DropDown,
    /// Session stats cache
    sessions: Rc<RefCell<Vec<SessionStats>>>,
    /// Callback for focusing a session
    focus_callback: Rc<RefCell<Option<FocusCallback>>>,
    /// Callback for disconnecting a session
    disconnect_callback: Rc<RefCell<Option<DisconnectCallback>>>,
}

impl ConnectionDashboard {
    /// Creates a new connection dashboard
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.set_margin_start(12);
        container.set_margin_end(12);
        container.set_margin_top(12);
        container.set_margin_bottom(12);

        // Create filter bar
        let filter_bar = Self::create_filter_bar();
        container.append(&filter_bar.0);

        let protocol_filter = filter_bar.1;
        let status_filter = filter_bar.2;

        // Create scrolled window for session cards
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .build();

        // Create flow box for session cards
        let flow_box = FlowBox::new();
        flow_box.set_valign(gtk4::Align::Start);
        flow_box.set_max_children_per_line(4);
        flow_box.set_min_children_per_line(1);
        flow_box.set_selection_mode(SelectionMode::None);
        flow_box.set_homogeneous(true);
        flow_box.set_column_spacing(12);
        flow_box.set_row_spacing(12);

        scrolled.set_child(Some(&flow_box));
        container.append(&scrolled);

        let filter = Rc::new(RefCell::new(DashboardFilter::new()));
        let sessions = Rc::new(RefCell::new(Vec::new()));

        let dashboard = Self {
            container,
            flow_box,
            filter,
            protocol_filter,
            status_filter,
            sessions,
            focus_callback: Rc::new(RefCell::new(None)),
            disconnect_callback: Rc::new(RefCell::new(None)),
        };

        dashboard.connect_filter_signals();
        dashboard
    }

    /// Creates the filter bar with dropdowns
    fn create_filter_bar() -> (GtkBox, DropDown, DropDown) {
        let filter_bar = GtkBox::new(Orientation::Horizontal, 8);
        filter_bar.set_margin_bottom(8);

        // Protocol filter
        let protocol_label = Label::new(Some("Protocol:"));
        filter_bar.append(&protocol_label);

        let protocol_items = StringList::new(&["All Protocols", "SSH", "RDP", "VNC", "SPICE"]);
        let protocol_filter = DropDown::builder()
            .model(&protocol_items)
            .selected(0)
            .build();
        filter_bar.append(&protocol_filter);

        // Status filter
        let status_label = Label::new(Some("Status:"));
        status_label.set_margin_start(16);
        filter_bar.append(&status_label);

        let status_items = StringList::new(&["All Statuses", "Connected", "Starting", "Error"]);
        let status_filter = DropDown::builder().model(&status_items).selected(0).build();
        filter_bar.append(&status_filter);

        // Spacer
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        filter_bar.append(&spacer);

        // Refresh button
        let refresh_button = Button::from_icon_name("view-refresh-symbolic");
        refresh_button.set_tooltip_text(Some("Refresh Dashboard"));
        refresh_button.update_property(&[gtk4::accessible::Property::Label("Refresh Dashboard")]);
        filter_bar.append(&refresh_button);

        (filter_bar, protocol_filter, status_filter)
    }

    /// Connects filter change signals
    fn connect_filter_signals(&self) {
        let filter = self.filter.clone();
        let flow_box = self.flow_box.clone();
        let sessions = self.sessions.clone();
        let focus_cb = self.focus_callback.clone();
        let disconnect_cb = self.disconnect_callback.clone();

        // Protocol filter change
        let filter_clone = filter.clone();
        let flow_box_clone = flow_box.clone();
        let sessions_clone = sessions.clone();
        let focus_cb_clone = focus_cb.clone();
        let disconnect_cb_clone = disconnect_cb.clone();
        self.protocol_filter
            .connect_selected_notify(move |dropdown| {
                let protocol = match dropdown.selected() {
                    0 => None, // All Protocols
                    1 => Some("ssh".to_string()),
                    2 => Some("rdp".to_string()),
                    3 => Some("vnc".to_string()),
                    4 => Some("spice".to_string()),
                    _ => None,
                };
                filter_clone.borrow_mut().protocol = protocol;
                Self::refresh_display(
                    &flow_box_clone,
                    &sessions_clone.borrow(),
                    &filter_clone.borrow(),
                    &focus_cb_clone,
                    &disconnect_cb_clone,
                );
            });

        // Status filter change
        let filter_clone = filter;
        let flow_box_clone = flow_box;
        let sessions_clone = sessions;
        self.status_filter.connect_selected_notify(move |dropdown| {
            let status = match dropdown.selected() {
                0 => None, // All Statuses
                1 => Some(SessionState::Active),
                2 => Some(SessionState::Starting),
                3 => Some(SessionState::Error),
                _ => None,
            };
            filter_clone.borrow_mut().status = status;
            Self::refresh_display(
                &flow_box_clone,
                &sessions_clone.borrow(),
                &filter_clone.borrow(),
                &focus_cb,
                &disconnect_cb,
            );
        });
    }

    /// Sets the callback for focusing a session
    /// **Validates: Requirements 13.3**
    pub fn set_focus_callback<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.focus_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback for disconnecting a session
    /// **Validates: Requirements 13.3**
    pub fn set_disconnect_callback<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.disconnect_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Updates the dashboard with new session data
    /// **Validates: Requirements 13.1**
    pub fn update_sessions(&self, sessions: Vec<SessionStats>) {
        *self.sessions.borrow_mut() = sessions;
        Self::refresh_display(
            &self.flow_box,
            &self.sessions.borrow(),
            &self.filter.borrow(),
            &self.focus_callback,
            &self.disconnect_callback,
        );
    }

    /// Refreshes the display with current filter
    fn refresh_display(
        flow_box: &FlowBox,
        sessions: &[SessionStats],
        filter: &DashboardFilter,
        focus_callback: &Rc<RefCell<Option<FocusCallback>>>,
        disconnect_callback: &Rc<RefCell<Option<DisconnectCallback>>>,
    ) {
        // Clear existing children
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        // Filter and display sessions
        let filtered = filter.apply(sessions);

        if filtered.is_empty() {
            let empty_label = Label::new(Some("No active sessions"));
            empty_label.add_css_class("dim-label");
            flow_box.append(&empty_label);
        } else {
            for stats in &filtered {
                let card = Self::create_session_card(stats, focus_callback, disconnect_callback);
                flow_box.append(&card);
            }
        }
    }

    /// Creates a session card widget
    /// **Validates: Requirements 13.1, 13.2, 13.3**
    fn create_session_card(
        stats: &SessionStats,
        focus_callback: &Rc<RefCell<Option<FocusCallback>>>,
        disconnect_callback: &Rc<RefCell<Option<DisconnectCallback>>>,
    ) -> Frame {
        let frame = Frame::new(None);
        frame.set_margin_start(4);
        frame.set_margin_end(4);
        frame.set_margin_top(4);
        frame.set_margin_bottom(4);

        let card_box = GtkBox::new(Orientation::Vertical, 4);
        card_box.set_margin_start(8);
        card_box.set_margin_end(8);
        card_box.set_margin_top(8);
        card_box.set_margin_bottom(8);

        // Header with name and status indicator
        let header_box = GtkBox::new(Orientation::Horizontal, 8);

        // Protocol icon
        let icon_name = match stats.protocol.as_str() {
            "ssh" => "utilities-terminal-symbolic",
            "rdp" => "computer-symbolic",
            "vnc" => "video-display-symbolic",
            "spice" => "video-display-symbolic",
            _ => "network-server-symbolic",
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        header_box.append(&icon);

        // Connection name
        let name_label = Label::new(Some(&stats.connection_name));
        name_label.set_hexpand(true);
        name_label.set_halign(gtk4::Align::Start);
        name_label.add_css_class("heading");
        header_box.append(&name_label);

        // Status indicator
        let status_label = Label::new(Some(stats.state_display()));
        status_label.add_css_class(stats.state_css_class());
        header_box.append(&status_label);

        card_box.append(&header_box);

        // Host info
        let host_label = Label::new(Some(&format!("Host: {}", stats.host)));
        host_label.set_halign(gtk4::Align::Start);
        host_label.add_css_class("dim-label");
        card_box.append(&host_label);

        // Duration
        let duration_label = Label::new(Some(&format!("Duration: {}", stats.format_duration())));
        duration_label.set_halign(gtk4::Align::Start);
        duration_label.add_css_class("dim-label");
        card_box.append(&duration_label);

        // Data transfer stats
        let data_label = Label::new(Some(&format!(
            "↑ {} / ↓ {}",
            SessionStats::format_bytes(stats.bytes_sent),
            SessionStats::format_bytes(stats.bytes_received)
        )));
        data_label.set_halign(gtk4::Align::Start);
        data_label.add_css_class("dim-label");
        card_box.append(&data_label);

        // Action buttons
        let button_box = GtkBox::new(Orientation::Horizontal, 4);
        button_box.set_margin_top(8);
        button_box.set_halign(gtk4::Align::End);

        // Focus button
        let focus_button = Button::from_icon_name("go-jump-symbolic");
        focus_button.set_tooltip_text(Some("Focus Session"));
        focus_button.update_property(&[gtk4::accessible::Property::Label("Focus Session")]);
        let session_id = stats.session_id;
        let focus_cb = focus_callback.clone();
        focus_button.connect_clicked(move |_| {
            if let Some(ref callback) = *focus_cb.borrow() {
                callback(session_id);
            }
        });
        button_box.append(&focus_button);

        // Disconnect button
        let disconnect_button = Button::from_icon_name("window-close-symbolic");
        disconnect_button.set_tooltip_text(Some("Disconnect"));
        disconnect_button.add_css_class("destructive-action");
        disconnect_button.update_property(&[gtk4::accessible::Property::Label("Disconnect")]);
        let session_id = stats.session_id;
        let disconnect_cb = disconnect_callback.clone();
        disconnect_button.connect_clicked(move |_| {
            if let Some(ref callback) = *disconnect_cb.borrow() {
                callback(session_id);
            }
        });
        button_box.append(&disconnect_button);

        card_box.append(&button_box);

        frame.set_child(Some(&card_box));
        frame
    }

    /// Returns the main widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the current filter
    #[must_use]
    pub fn filter(&self) -> DashboardFilter {
        self.filter.borrow().clone()
    }

    /// Sets the filter
    pub fn set_filter(&self, filter: DashboardFilter) {
        // Update dropdowns
        let protocol_idx = match filter.protocol.as_deref() {
            None => 0,
            Some("ssh") => 1,
            Some("rdp") => 2,
            Some("vnc") => 3,
            Some("spice") => 4,
            _ => 0,
        };
        self.protocol_filter.set_selected(protocol_idx);

        let status_idx = match filter.status {
            None => 0,
            Some(SessionState::Active) => 1,
            Some(SessionState::Starting) => 2,
            Some(SessionState::Error) => 3,
            _ => 0,
        };
        self.status_filter.set_selected(status_idx);

        *self.filter.borrow_mut() = filter;
        Self::refresh_display(
            &self.flow_box,
            &self.sessions.borrow(),
            &self.filter.borrow(),
            &self.focus_callback,
            &self.disconnect_callback,
        );
    }
}

impl Default for ConnectionDashboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Shows the dashboard dialog
/// **Validates: Requirements 13.1**
pub fn show_dashboard_dialog(
    parent: &gtk4::ApplicationWindow,
    sessions: Vec<SessionStats>,
    focus_callback: impl Fn(Uuid) + 'static,
    disconnect_callback: impl Fn(Uuid) + 'static,
) {
    let dialog = adw::Window::builder()
        .title("Connection Dashboard")
        .transient_for(parent)
        .modal(false)
        .default_width(800)
        .default_height(600)
        .build();

    dialog.set_size_request(350, 300);

    let dashboard = ConnectionDashboard::new();
    dashboard.set_focus_callback(move |id| {
        focus_callback(id);
    });
    dashboard.set_disconnect_callback(disconnect_callback);
    dashboard.update_sessions(sessions);

    dialog.set_content(Some(dashboard.widget()));
    dialog.present();
}
