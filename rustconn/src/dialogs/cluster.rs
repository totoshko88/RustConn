//! Cluster dialog for managing connection clusters
//!
//! Provides an `adw::Dialog` for creating, editing, and managing clusters
//! with connection selection.
//!
//! Migrated to `adw::Dialog` for GNOME HIG compliance: bottom-sheet on narrow
//! screens, centered floating sheet on wide screens.

use crate::i18n::{i18n, i18n_f};
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
    dialog: adw::Dialog,
    name_entry: gtk4::Entry,
    connections_list: ListBox,
    connection_rows: Rc<RefCell<Vec<ConnectionSelectionRow>>>,
    editing_id: Rc<RefCell<Option<Uuid>>>,
    on_save: ClusterCallback,
    select_all_btn: Button,
    deselect_all_btn: Button,
    parent: Option<gtk4::Widget>,
}

/// Represents a connection selection row in the cluster dialog
#[allow(
    dead_code,
    reason = "Fields kept for GTK widget lifecycle and future use"
)]
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
        let dialog = adw::Dialog::builder()
            .title(i18n("New Cluster"))
            .content_width(600)
            .content_height(580)
            .build();

        // Header bar with Create icon button (GNOME HIG)
        let header = adw::HeaderBar::new();
        let save_btn = Button::from_icon_name("list-add-symbolic");
        save_btn.set_tooltip_text(Some(&i18n("Create")));
        save_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Create"))]);
        save_btn.add_css_class("suggested-action");
        header.pack_start(&save_btn);

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

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&scrolled));
        dialog.set_child(Some(&toolbar_view));

        // === Cluster Details ===
        let details_group = adw::PreferencesGroup::builder()
            .title(i18n("Cluster Details"))
            .build();

        // Name entry row using builder
        let (name_row, name_entry) = super::widgets::EntryRowBuilder::new(i18n("Name"))
            .placeholder(i18n("Enter cluster name"))
            .build();
        name_row.set_activatable_widget(Some(&name_entry));
        details_group.add(&name_row);

        content.append(&details_group);

        // Connections selection section
        let (connections_group, connections_list, select_all_btn, deselect_all_btn) =
            Self::create_connections_section();
        content.append(&connections_group);

        let on_save: ClusterCallback = Rc::new(RefCell::new(None));
        let connection_rows: Rc<RefCell<Vec<ConnectionSelectionRow>>> =
            Rc::new(RefCell::new(Vec::new()));
        let editing_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));

        // Connect save button
        let dialog_clone = dialog.clone();
        let on_save_clone = on_save.clone();
        let name_entry_clone = name_entry.clone();
        let connection_rows_clone = connection_rows.clone();
        let editing_id_clone = editing_id.clone();
        save_btn.connect_clicked(move |_| {
            let name = name_entry_clone.text().trim().to_string();
            if name.is_empty() {
                name_entry_clone.add_css_class("error");
                name_entry_clone.grab_focus();
                return;
            }
            name_entry_clone.remove_css_class("error");

            // Collect selected connections
            let selected_ids: Vec<Uuid> = connection_rows_clone
                .borrow()
                .iter()
                .filter(|row| row.selected_check.is_active())
                .map(|row| row.connection_id)
                .collect();

            if selected_ids.is_empty() {
                crate::toast::show_error_toast_on_active_window(&i18n(
                    "Please select at least one connection",
                ));
                return;
            }

            // Create or update cluster
            let mut cluster = if let Some(id) = *editing_id_clone.borrow() {
                Cluster::with_id(id, name)
            } else {
                Cluster::new(name)
            };

            // broadcast_enabled is left at its default (false). Cluster broadcast
            // was removed in 0.14.8 — broadcast is now a split-view feature.
            for conn_id in selected_ids {
                cluster.add_connection(conn_id);
            }

            if let Some(ref cb) = *on_save_clone.borrow() {
                cb(Some(cluster));
            }
            dialog_clone.close();
        });

        let parent_widget = parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        Self {
            dialog,
            name_entry,
            connections_list,
            connection_rows,
            editing_id,
            on_save,
            select_all_btn,
            deselect_all_btn,
            parent: parent_widget,
        }
    }

    /// Creates the connections selection section
    fn create_connections_section() -> (adw::PreferencesGroup, ListBox, Button, Button) {
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

        (group, connections_list, select_all_btn, deselect_all_btn)
    }

    /// Creates a connection selection row widget
    fn create_connection_row(connection: &Connection) -> ConnectionSelectionRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

        let selected_check = CheckButton::new();

        // Protocol icon
        let icon_name =
            rustconn_core::get_protocol_icon(connection.protocol_config.protocol_type());
        let icon = gtk4::Image::from_icon_name(icon_name);

        let label = Label::builder()
            .label(&format!("{} ({})", connection.name, connection.host))
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
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

        // Wire up select all / deselect all buttons using stored references
        {
            let rows = self.connection_rows.clone();
            self.select_all_btn.connect_clicked(move |_| {
                for row in rows.borrow().iter() {
                    row.selected_check.set_active(true);
                }
            });
        }
        {
            let rows = self.connection_rows.clone();
            self.deselect_all_btn.connect_clicked(move |_| {
                for row in rows.borrow().iter() {
                    row.selected_check.set_active(false);
                }
            });
        }
    }

    /// Sets the cluster to edit (for editing existing clusters)
    pub fn set_cluster(&self, cluster: &Cluster) {
        *self.editing_id.borrow_mut() = Some(cluster.id);
        self.dialog.set_title(&i18n("Edit Cluster"));
        self.name_entry.set_text(&cluster.name);

        // Select the connections that are in the cluster
        for row in self.connection_rows.borrow().iter() {
            let is_selected = cluster.contains_connection(row.connection_id);
            row.selected_check.set_active(is_selected);
        }
    }

    /// Pre-selects connections by their IDs (for creating cluster from sidebar selection)
    pub fn pre_select_connections(&self, selected_ids: &[Uuid]) {
        for row in self.connection_rows.borrow().iter() {
            let is_selected = selected_ids.contains(&row.connection_id);
            row.selected_check.set_active(is_selected);
        }
    }

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<Cluster>) + 'static>(&self, cb: F) {
        *self.on_save.borrow_mut() = Some(Box::new(cb));
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}

/// Cluster list dialog for managing all clusters
pub struct ClusterListDialog {
    dialog: adw::Dialog,
    clusters_list: ListBox,
    cluster_rows: Rc<RefCell<Vec<ClusterListRow>>>,
    on_connect: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_disconnect: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_edit: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_delete: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>,
    on_new: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Callback to get current clusters for refresh
    clusters_provider: Rc<RefCell<Option<Box<dyn Fn() -> Vec<Cluster>>>>>,
    parent: Option<gtk4::Widget>,
}

/// Represents a cluster row in the list dialog
#[allow(
    dead_code,
    reason = "Fields kept for GTK widget lifecycle and future dynamic updates"
)]
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
    /// Disconnect button
    disconnect_button: Button,
    /// Edit button
    edit_button: Button,
    /// Delete button
    delete_button: Button,
}

impl ClusterListDialog {
    /// Creates a new cluster list dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Manage Clusters"))
            .content_width(600)
            .content_height(500)
            .build();

        // Header bar with Add button and standard window buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        let new_btn = Button::from_icon_name("list-add-symbolic");
        new_btn.set_tooltip_text(Some(&i18n("New Cluster")));
        new_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("New Cluster"))]);
        header.pack_start(&new_btn);

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

        // Use ToolbarView for adw::Dialog (GNOME HIG)
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&clamp));
        dialog.set_child(Some(&toolbar_view));

        // Info label
        let info_label = Label::builder()
            .label(i18n("Clusters open multiple servers together. To mirror keystrokes across them, split the resulting tabs and use the Broadcast toggle in the header bar."))
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
        let on_disconnect: Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>> = Rc::new(RefCell::new(None));
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

        let parent_widget = parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        Self {
            dialog,
            clusters_list,
            cluster_rows,
            on_connect,
            on_disconnect,
            on_edit,
            on_delete,
            on_new,
            clusters_provider,
            parent: parent_widget,
        }
    }

    /// Creates a cluster row widget
    fn create_cluster_row(cluster: &Cluster) -> ClusterListRow {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(12);
        hbox.set_margin_bottom(12);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

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

        // Note: cluster.broadcast_enabled is kept on disk for backward
        // compatibility (CLI still reads/writes it) but no longer affects
        // anything in the GUI as of 0.14.8 — broadcast is a split-view
        // feature triggered from the header bar. Don't show a stale
        // "(broadcast enabled)" indicator here that promises behaviour
        // the cluster machinery no longer delivers.
        let count_label = Label::builder()
            .label(&i18n_f(
                "{} connections",
                &[&cluster.connection_count().to_string()],
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
        connect_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Connect to cluster",
        ))]);

        let disconnect_button = Button::builder()
            .icon_name("media-playback-stop-symbolic")
            .tooltip_text(&i18n("Disconnect all cluster sessions"))
            .css_classes(["flat"])
            .build();
        disconnect_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Disconnect all cluster sessions",
        ))]);

        let edit_button = Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(&i18n("Edit cluster"))
            .css_classes(["flat"])
            .build();
        edit_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Edit cluster"))]);

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(&i18n("Delete cluster"))
            .css_classes(["flat", "destructive-action"])
            .build();
        delete_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Delete cluster"))]);

        hbox.append(&connect_button);
        hbox.append(&disconnect_button);
        hbox.append(&edit_button);
        hbox.append(&delete_button);

        let row = ListBoxRow::builder().child(&hbox).build();

        ClusterListRow {
            row,
            cluster_id: cluster.id,
            name_label,
            count_label,
            connect_button,
            disconnect_button,
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

            let on_disconnect_clone = self.on_disconnect.clone();
            cluster_row.disconnect_button.connect_clicked(move |_| {
                if let Some(ref cb) = *on_disconnect_clone.borrow() {
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
            let dialog_weak = self.dialog.downgrade();
            cluster_row.delete_button.connect_clicked(move |_| {
                let Some(dlg) = dialog_weak.upgrade() else {
                    return;
                };
                let alert = adw::AlertDialog::builder()
                    .heading(i18n("Delete Cluster?"))
                    .body(i18n("This cluster will be permanently removed."))
                    .build();
                alert.add_response("cancel", &i18n("Cancel"));
                alert.add_response("delete", &i18n("Delete"));
                alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                alert.set_default_response(Some("cancel"));
                alert.set_close_response("cancel");
                let on_delete_inner = on_delete_clone.clone();
                alert.connect_response(None, move |_, response| {
                    if response == "delete"
                        && let Some(ref cb) = *on_delete_inner.borrow()
                    {
                        cb(cluster_id);
                    }
                });
                alert.present(Some(&dlg));
            });

            self.clusters_list.append(&cluster_row.row);
            self.cluster_rows.borrow_mut().push(cluster_row);
        }
    }

    /// Sets the callback for connecting to a cluster
    pub fn set_on_connect<F: Fn(Uuid) + 'static>(&self, cb: F) {
        *self.on_connect.borrow_mut() = Some(Box::new(cb));
    }

    /// Sets the callback for disconnecting all sessions in a cluster
    pub fn set_on_disconnect<F: Fn(Uuid) + 'static>(&self, cb: F) {
        *self.on_disconnect.borrow_mut() = Some(Box::new(cb));
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
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}
