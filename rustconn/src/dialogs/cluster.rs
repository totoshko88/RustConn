//! Cluster dialog for managing connection clusters
//!
//! Provides a GTK4 dialog for creating, editing, and managing clusters
//! with connection selection and broadcast mode configuration.
//!
//! Updated for GTK 4.10+ compatibility using Window instead of Dialog.
//! Migrated to libadwaita components for GNOME HIG compliance.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow,
};
use libadwaita as adw;
use rustconn_core::cluster::Cluster;
use rustconn_core::models::Connection;
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for cluster dialog callback
pub type ClusterCallback = Rc<RefCell<Option<Box<dyn Fn(Option<Cluster>)>>>>;

/// Cluster dialog for managing clusters
pub struct ClusterDialog {
    window: adw::Window,
    name_entry: gtk4::Entry,
    broadcast_row: adw::SwitchRow,
    connections_list: ListBox,
    connection_rows: Rc<RefCell<Vec<ConnectionSelectionRow>>>,
    editing_id: Rc<RefCell<Option<Uuid>>>,
    on_save: ClusterCallback,
}

/// Represents a connection selection row in the cluster dialog
#[allow(dead_code)] // Fields kept for GTK widget lifecycle and future use
struct ConnectionSelectionRow {
    /// The row widget
    row: ListBoxRow,
    /// Checkbox for selection
    selected_check: CheckButton,
    /// Connection ID
    connection_id: Uuid,
    /// Connection name (for display)
    connection_name: String,
}

impl ClusterDialog {
    /// Creates a new cluster dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title(i18n("New Cluster"))
            .modal(true)
            .default_width(500)
            .default_height(400)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(320, 280);

        // Header bar (GNOME HIG)
        let (header, close_btn, save_btn) =
            crate::dialogs::widgets::dialog_header("Close", "Create");

        // Close button handler
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Scrollable content with clamp
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.append(&header);
        main_box.append(&scrolled);
        window.set_content(Some(&main_box));

        // === Cluster Details ===
        let details_group = adw::PreferencesGroup::builder()
            .title(i18n("Cluster Details"))
            .build();

        // Name entry row using builder
        let (name_row, name_entry) = super::widgets::EntryRowBuilder::new("Name")
            .placeholder("Enter cluster name")
            .build();
        name_row.set_activatable_widget(Some(&name_entry));
        details_group.add(&name_row);

        // Broadcast mode switch row
        let broadcast_row = super::widgets::SwitchRowBuilder::new("Broadcast mode")
            .subtitle("Send keyboard input to all cluster sessions simultaneously")
            .active(false)
            .build();
        details_group.add(&broadcast_row);

        content.append(&details_group);

        // Connections selection section
        let (connections_group, connections_list) = Self::create_connections_section();
        content.append(&connections_group);

        let on_save: ClusterCallback = Rc::new(RefCell::new(None));
        let connection_rows: Rc<RefCell<Vec<ConnectionSelectionRow>>> =
            Rc::new(RefCell::new(Vec::new()));
        let editing_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));

        // Connect save button
        let window_clone = window.clone();
        let on_save_clone = on_save.clone();
        let name_entry_clone = name_entry.clone();
        let broadcast_row_clone = broadcast_row.clone();
        let connection_rows_clone = connection_rows.clone();
        let editing_id_clone = editing_id.clone();
        save_btn.connect_clicked(move |_| {
            let name = name_entry_clone.text().trim().to_string();
            if name.is_empty() {
                crate::toast::show_toast_on_window(
                    &window_clone,
                    &i18n("Cluster name cannot be empty"),
                    crate::toast::ToastType::Warning,
                );
                return;
            }

            // Collect selected connections
            let selected_ids: Vec<Uuid> = connection_rows_clone
                .borrow()
                .iter()
                .filter(|row| row.selected_check.is_active())
                .map(|row| row.connection_id)
                .collect();

            if selected_ids.is_empty() {
                crate::toast::show_toast_on_window(
                    &window_clone,
                    &i18n("Please select at least one connection"),
                    crate::toast::ToastType::Warning,
                );
                return;
            }

            // Create or update cluster
            let mut cluster = if let Some(id) = *editing_id_clone.borrow() {
                Cluster::with_id(id, name)
            } else {
                Cluster::new(name)
            };

            cluster.broadcast_enabled = broadcast_row_clone.is_active();
            for conn_id in selected_ids {
                cluster.add_connection(conn_id);
            }

            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(Some(cluster));
            }
            window_clone.close();
        });

        Self {
            window,
            name_entry,
            broadcast_row,
            connections_list,
            connection_rows,
            editing_id,
            on_save,
        }
    }

    /// Creates the connections selection section
    fn create_connections_section() -> (adw::PreferencesGroup, ListBox) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Connections"))
            .description(i18n("Select connections to include in this cluster"))
            .vexpand(true)
            .build();

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(200)
            .vexpand(true)
            .build();

        let connections_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&connections_list));

        group.add(&scrolled);

        // Select all / Deselect all buttons
        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);
        button_box.set_margin_top(12);

        let select_all_btn = Button::builder().label(i18n("Select All")).build();
        let deselect_all_btn = Button::builder().label(i18n("Deselect All")).build();
        button_box.append(&select_all_btn);
        button_box.append(&deselect_all_btn);

        group.add(&button_box);

        (group, connections_list)
    }

    /// Creates a connection selection row widget
    fn create_connection_row(connection: &Connection) -> ConnectionSelectionRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        let selected_check = CheckButton::new();

        // Protocol icon
        let icon_name = match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(_) => "utilities-terminal-symbolic",
            rustconn_core::models::ProtocolConfig::Rdp(_) => "computer-symbolic",
            rustconn_core::models::ProtocolConfig::Vnc(_) => "video-display-symbolic",
            rustconn_core::models::ProtocolConfig::Spice(_) => "video-display-symbolic",
            rustconn_core::models::ProtocolConfig::ZeroTrust(_) => "security-high-symbolic",
            rustconn_core::models::ProtocolConfig::Telnet(_) => "call-start-symbolic",
            rustconn_core::models::ProtocolConfig::Serial(_) => "modem-symbolic",
            rustconn_core::models::ProtocolConfig::Sftp(_) => "folder-remote-symbolic",
            rustconn_core::models::ProtocolConfig::Kubernetes(_) => {
                "application-x-executable-symbolic"
            }
        };
        let icon = gtk4::Image::from_icon_name(icon_name);

        let label = Label::builder()
            .label(&format!("{} ({})", connection.name, connection.host))
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();

        hbox.append(&selected_check);
        hbox.append(&icon);
        hbox.append(&label);

        let row = ListBoxRow::builder().child(&hbox).build();

        ConnectionSelectionRow {
            row,
            selected_check,
            connection_id: connection.id,
            connection_name: connection.name.clone(),
        }
    }

    /// Sets the available connections for selection
    pub fn set_connections(&self, connections: &[Connection]) {
        // Clear existing rows
        while let Some(row) = self.connections_list.row_at_index(0) {
            self.connections_list.remove(&row);
        }
        self.connection_rows.borrow_mut().clear();

        // Add rows for each connection
        for conn in connections {
            let conn_row = Self::create_connection_row(conn);
            self.connections_list.append(&conn_row.row);
            self.connection_rows.borrow_mut().push(conn_row);
        }

        // Wire up select all / deselect all buttons
        // Find the buttons in the frame
        if let Some(frame) = self.connections_list.parent()
            && let Some(scrolled) = frame.parent()
            && let Some(vbox) = scrolled.parent()
            && let Some(button_box) = vbox.last_child()
            && let Some(button_box) = button_box.downcast_ref::<GtkBox>()
        {
            let connection_rows_clone = self.connection_rows.clone();
            if let Some(select_all) = button_box.first_child()
                && let Some(select_all_btn) = select_all.downcast_ref::<Button>()
            {
                let rows = connection_rows_clone.clone();
                select_all_btn.connect_clicked(move |_| {
                    for row in rows.borrow().iter() {
                        row.selected_check.set_active(true);
                    }
                });
            }
            if let Some(deselect_all) = button_box.last_child()
                && let Some(deselect_all_btn) = deselect_all.downcast_ref::<Button>()
            {
                let rows = connection_rows_clone;
                deselect_all_btn.connect_clicked(move |_| {
                    for row in rows.borrow().iter() {
                        row.selected_check.set_active(false);
                    }
                });
            }
        }
    }

    /// Sets the cluster to edit (for editing existing clusters)
    pub fn set_cluster(&self, cluster: &Cluster) {
        *self.editing_id.borrow_mut() = Some(cluster.id);
        self.window.set_title(Some(&i18n("Edit Cluster")));
        self.name_entry.set_text(&cluster.name);
        self.broadcast_row.set_active(cluster.broadcast_enabled);

        // Select the connections that are in the cluster
        for row in self.connection_rows.borrow().iter() {
            let is_selected = cluster.contains_connection(row.connection_id);
            row.selected_check.set_active(is_selected);
        }
    }

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<Cluster>) + 'static>(&self, cb: F) {
        *self.on_save.borrow_mut() = Some(Box::new(cb));
        self.window.present();
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }
}

/// Cluster list dialog for managing all clusters
pub struct ClusterListDialog {
    window: adw::Window,
    clusters_list: ListBox,
    cluster_rows: Rc<RefCell<Vec<ClusterListRow>>>,
    on_connect: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_edit: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_delete: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_new: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Callback to get current clusters for refresh
    clusters_provider: Rc<RefCell<Option<Box<dyn Fn() -> Vec<Cluster>>>>>,
}

/// Represents a cluster row in the list dialog
#[allow(dead_code)] // Fields kept for GTK widget lifecycle and future dynamic updates
struct ClusterListRow {
    /// The row widget
    row: ListBoxRow,
    /// Cluster ID
    cluster_id: Uuid,
    /// Cluster name label
    name_label: Label,
    /// Connection count label
    count_label: Label,
    /// Connect button
    connect_button: Button,
    /// Edit button
    edit_button: Button,
    /// Delete button
    delete_button: Button,
}

impl ClusterListDialog {
    /// Creates a new cluster list dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title(i18n("Manage Clusters"))
            .modal(true)
            .default_width(500)
            .default_height(400)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(320, 280);

        // Header bar (GNOME HIG)
        let (header, close_btn, new_btn) =
            crate::dialogs::widgets::dialog_header("Close", "Create");

        // Close button handler
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        // Create main content area with clamp
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        clamp.set_child(Some(&content));

        // Use ToolbarView for adw::Window
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.append(&header);
        main_box.append(&clamp);
        window.set_content(Some(&main_box));

        // Info label
        let info_label = Label::builder()
            .label(i18n("Clusters allow you to connect to multiple servers simultaneously and optionally broadcast input to all sessions."))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        content.append(&info_label);

        // Clusters list
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(250)
            .vexpand(true)
            .build();

        let clusters_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        scrolled.set_child(Some(&clusters_list));

        content.append(&scrolled);

        let on_connect: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>> = Rc::new(RefCell::new(None));
        let on_edit: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>> = Rc::new(RefCell::new(None));
        let on_delete: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>> = Rc::new(RefCell::new(None));
        let on_new: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let cluster_rows: Rc<RefCell<Vec<ClusterListRow>>> = Rc::new(RefCell::new(Vec::new()));
        let clusters_provider: Rc<RefCell<Option<Box<dyn Fn() -> Vec<Cluster>>>>> =
            Rc::new(RefCell::new(None));

        // Connect new button
        let on_new_clone = on_new.clone();
        new_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_new_clone.borrow() {
                cb();
            }
        });

        Self {
            window,
            clusters_list,
            cluster_rows,
            on_connect,
            on_edit,
            on_delete,
            on_new,
            clusters_provider,
        }
    }

    /// Creates a cluster row widget
    fn create_cluster_row(cluster: &Cluster) -> ClusterListRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        // Cluster icon
        let icon = gtk4::Image::from_icon_name("network-workgroup-symbolic");
        hbox.append(&icon);

        // Info box
        let info_box = GtkBox::new(Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let name_label = Label::builder()
            .label(&cluster.name)
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();

        let broadcast_indicator = if cluster.broadcast_enabled {
            " (broadcast enabled)"
        } else {
            ""
        };
        let count_label = Label::builder()
            .label(&format!(
                "{} connections{}",
                cluster.connection_count(),
                broadcast_indicator
            ))
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "caption"])
            .build();

        info_box.append(&name_label);
        info_box.append(&count_label);
        hbox.append(&info_box);

        // Action buttons
        let connect_button = Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text(&i18n("Connect to cluster"))
            .css_classes(["flat"])
            .build();

        let edit_button = Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(&i18n("Edit cluster"))
            .css_classes(["flat"])
            .build();

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(&i18n("Delete cluster"))
            .css_classes(["flat", "destructive-action"])
            .build();

        hbox.append(&connect_button);
        hbox.append(&edit_button);
        hbox.append(&delete_button);

        let row = ListBoxRow::builder().child(&hbox).build();

        ClusterListRow {
            row,
            cluster_id: cluster.id,
            name_label,
            count_label,
            connect_button,
            edit_button,
            delete_button,
        }
    }

    /// Sets the clusters to display
    pub fn set_clusters(&self, clusters: &[Cluster]) {
        // Clear existing rows
        while let Some(row) = self.clusters_list.row_at_index(0) {
            self.clusters_list.remove(&row);
        }
        self.cluster_rows.borrow_mut().clear();

        if clusters.is_empty() {
            // Show empty state
            let empty_label = Label::builder()
                .label(i18n(
                    "No clusters defined. Click 'New Cluster' to create one.",
                ))
                .css_classes(["dim-label"])
                .margin_top(20)
                .margin_bottom(20)
                .build();
            let empty_row = ListBoxRow::builder()
                .child(&empty_label)
                .selectable(false)
                .build();
            self.clusters_list.append(&empty_row);
            return;
        }

        // Add rows for each cluster
        for cluster in clusters {
            let cluster_row = Self::create_cluster_row(cluster);

            // Wire up buttons
            let cluster_id = cluster.id;

            let on_connect_clone = self.on_connect.clone();
            cluster_row.connect_button.connect_clicked(move |_| {
                if let Some(ref cb) = *on_connect_clone.borrow() {
                    cb(cluster_id);
                }
            });

            let on_edit_clone = self.on_edit.clone();
            cluster_row.edit_button.connect_clicked(move |_| {
                if let Some(ref cb) = *on_edit_clone.borrow() {
                    cb(cluster_id);
                }
            });

            let on_delete_clone = self.on_delete.clone();
            cluster_row.delete_button.connect_clicked(move |_| {
                if let Some(ref cb) = *on_delete_clone.borrow() {
                    cb(cluster_id);
                }
            });

            self.clusters_list.append(&cluster_row.row);
            self.cluster_rows.borrow_mut().push(cluster_row);
        }
    }

    /// Sets the callback for connecting to a cluster
    pub fn set_on_connect<F: Fn(Uuid) + 'static>(&self, cb: F) {
        *self.on_connect.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for editing a cluster
    pub fn set_on_edit<F: Fn(Uuid) + 'static>(&self, cb: F) {
        *self.on_edit.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for deleting a cluster
    pub fn set_on_delete<F: Fn(Uuid) + 'static>(&self, cb: F) {
        *self.on_delete.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for creating a new cluster
    pub fn set_on_new<F: Fn() + 'static>(&self, cb: F) {
        *self.on_new.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the clusters provider callback for refresh operations
    ///
    /// This callback is called when `refresh_list()` is invoked to get the
    /// current list of clusters from the application state.
    pub fn set_clusters_provider<F: Fn() -> Vec<Cluster> + 'static>(&self, provider: F) {
        *self.clusters_provider.borrow_mut() = Some(Box::new(provider));
    }

    /// Refreshes the cluster list from the clusters provider
    ///
    /// This method retrieves the current clusters using the provider callback
    /// and updates the list display. If no provider is set, this is a no-op.
    ///
    /// Call this method after cluster operations (create, edit, delete) to
    /// ensure the list reflects the current state.
    pub fn refresh_list(&self) {
        if let Some(ref provider) = *self.clusters_provider.borrow() {
            let clusters = provider();
            self.set_clusters(&clusters);
        }
    }

    /// Shows the dialog
    pub fn show(&self) {
        self.window.present();
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }
}
