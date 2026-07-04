//! Step 1: Connection & Name page for the Tunnel Builder wizard
//!
//! Provides SSH connection selection, tunnel name input, jump host override,
//! and a live `TunnelPathDiagram` preview. Filters connections by name/host
//! with 150ms debounce.

use crate::i18n::i18n;
use crate::state::SharedAppState;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, StringList};
use libadwaita as adw;
use rustconn_core::models::{Connection, ProtocolConfig, ProtocolType};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use super::TunnelPathDiagram;

/// Step 1 page — Connection & Name selection
///
/// Displays tunnel name entry, SSH connection combo with search filter,
/// jump host override, "New SSH Connection" button, and a live path diagram.
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct StepConnectionPage {
    pub page: adw::NavigationPage,
    state: SharedAppState,
    // Widgets
    name_row: adw::EntryRow,
    connection_row: adw::ComboRow,
    search_entry: gtk4::SearchEntry,
    jump_host_row: adw::ComboRow,
    new_connection_btn: gtk4::Button,
    next_button: gtk4::Button,
    diagram: TunnelPathDiagram,
    // Empty state
    empty_state: adw::StatusPage,
    content_box: GtkBox,
    // Data
    /// Filtered SSH connection IDs (matches current combo model order)
    filtered_connection_ids: Rc<RefCell<Vec<Uuid>>>,
    /// All SSH connections (cached on page creation / refresh)
    all_ssh_connections: Rc<RefCell<Vec<Connection>>>,
    /// Jump host IDs (first entry = None for "(None)")
    jump_host_ids: Rc<RefCell<Vec<Option<Uuid>>>>,
    // Callbacks
    on_next: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_new_connection: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    // Debounce
    search_timeout_id: Rc<RefCell<Option<glib::SourceId>>>,
}

impl StepConnectionPage {
    /// Creates the Step 1 page
    #[must_use]
    pub fn new(state: SharedAppState) -> Self {
        let on_next: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_new_connection: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let filtered_connection_ids: Rc<RefCell<Vec<Uuid>>> = Rc::new(RefCell::new(Vec::new()));
        let all_ssh_connections: Rc<RefCell<Vec<Connection>>> = Rc::new(RefCell::new(Vec::new()));
        let jump_host_ids: Rc<RefCell<Vec<Option<Uuid>>>> = Rc::new(RefCell::new(Vec::new()));
        let search_timeout_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        // Main content
        let content_box = GtkBox::new(Orientation::Vertical, 12);
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(12);
        content_box.set_margin_start(12);
        content_box.set_margin_end(12);

        // === General group (tunnel name) ===
        let general_group = adw::PreferencesGroup::builder()
            .title(i18n("General"))
            .build();

        let name_row = adw::EntryRow::builder().title(i18n("Tunnel Name")).build();
        general_group.add(&name_row);
        content_box.append(&general_group);

        // === SSH Connection group ===
        let connection_group = adw::PreferencesGroup::builder()
            .title(i18n("SSH Connection"))
            .build();

        let connection_row = adw::ComboRow::builder().title(i18n("Connection")).build();
        connection_group.add(&connection_row);

        let search_entry = gtk4::SearchEntry::builder()
            .placeholder_text(i18n("Filter connections…"))
            .build();
        connection_group.add(&search_entry);
        content_box.append(&connection_group);

        // Jump host override
        let jump_host_group = adw::PreferencesGroup::builder()
            .title(i18n("Jump Host"))
            .description(i18n("Override bastion/proxy host"))
            .build();

        let jump_host_row = adw::ComboRow::builder()
            .title(i18n("Jump Host"))
            .subtitle(i18n("Connect via intermediate server"))
            .build();
        jump_host_group.add(&jump_host_row);

        // "New SSH Connection" button
        let new_connection_btn = gtk4::Button::builder()
            .label(i18n("New SSH Connection"))
            .css_classes(["flat"])
            .build();
        new_connection_btn.set_tooltip_text(Some(&i18n("Create a new SSH connection")));
        new_connection_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Create a new SSH connection",
        ))]);

        let btn_box = GtkBox::new(Orientation::Horizontal, 0);
        btn_box.set_margin_top(6);
        btn_box.set_halign(gtk4::Align::Start);
        btn_box.append(&new_connection_btn);

        content_box.append(&jump_host_group);
        content_box.append(&btn_box);

        // === Path Preview ===
        let diagram = TunnelPathDiagram::new();
        diagram.hide_status();

        let diagram_box = GtkBox::new(Orientation::Vertical, 0);
        diagram_box.set_margin_top(12);
        diagram_box.append(diagram.widget());
        content_box.append(&diagram_box);

        // === Empty state (shown when no SSH connections exist) ===
        let empty_state = adw::StatusPage::builder()
            .icon_name("network-server-symbolic")
            .title(i18n("No SSH connections available"))
            .description(i18n("Create an SSH connection first to set up a tunnel"))
            .vexpand(true)
            .visible(false)
            .build();

        let empty_btn = gtk4::Button::builder()
            .label(i18n("New SSH Connection"))
            .css_classes(["suggested-action", "pill"])
            .halign(gtk4::Align::Center)
            .build();
        empty_state.set_child(Some(&empty_btn));

        // === Layout assembly ===
        let outer_box = GtkBox::new(Orientation::Vertical, 0);
        outer_box.append(&content_box);
        outer_box.append(&empty_state);

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&outer_box)
            .build();

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&clamp)
            .vexpand(true)
            .build();

        // Footer with Next button
        let footer = GtkBox::new(Orientation::Horizontal, 12);
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let next_button = gtk4::Button::with_label(&i18n("Next"));
        next_button.add_css_class("suggested-action");
        next_button.set_receives_default(true);
        next_button.set_sensitive(false);
        footer.append(&next_button);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&adw::HeaderBar::new());
        toolbar_view.set_content(Some(&scrolled));
        toolbar_view.add_bottom_bar(&footer);

        let page = adw::NavigationPage::builder()
            .title(i18n("New Tunnel"))
            .child(&toolbar_view)
            .build();

        // === Wire signals ===

        // Next button
        let on_next_clone = on_next.clone();
        next_button.connect_clicked(move |_| {
            if let Some(ref cb) = *on_next_clone.borrow() {
                cb();
            }
        });

        // "New SSH Connection" button (main)
        let on_new_conn_clone = on_new_connection.clone();
        new_connection_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_new_conn_clone.borrow() {
                cb();
            }
        });

        // "New SSH Connection" button (empty state)
        let on_new_conn_clone2 = on_new_connection.clone();
        empty_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_new_conn_clone2.borrow() {
                cb();
            }
        });

        let mut page_obj = Self {
            page,
            state,
            name_row,
            connection_row,
            search_entry,
            jump_host_row,
            new_connection_btn,
            next_button,
            diagram,
            empty_state,
            content_box,
            filtered_connection_ids,
            all_ssh_connections,
            jump_host_ids,
            on_next,
            on_new_connection,
            search_timeout_id,
        };

        // Load initial data
        page_obj.refresh_connections();

        // Wire validation on name change
        let next_btn_v = page_obj.next_button.clone();
        let conn_ids_v = page_obj.filtered_connection_ids.clone();
        let conn_row_v = page_obj.connection_row.clone();
        let name_row_v = page_obj.name_row.clone();
        page_obj.name_row.connect_changed(move |_| {
            let name_valid = Self::validate_name(&name_row_v);
            let conn_valid = Self::validate_connection(&conn_row_v, &conn_ids_v);
            next_btn_v.set_sensitive(name_valid && conn_valid);
        });

        // Wire validation + diagram update on connection selection change
        let next_btn_c = page_obj.next_button.clone();
        let conn_ids_c = page_obj.filtered_connection_ids.clone();
        let name_row_c = page_obj.name_row.clone();
        let diagram_c = page_obj.diagram.clone();
        let state_c = page_obj.state.clone();
        let jump_host_ids_c = page_obj.jump_host_ids.clone();
        let jump_host_row_c = page_obj.jump_host_row.clone();
        page_obj.connection_row.connect_selected_notify(move |row| {
            let name_valid = Self::validate_name(&name_row_c);
            let conn_valid = Self::validate_connection(row, &conn_ids_c);
            next_btn_c.set_sensitive(name_valid && conn_valid);

            // Update diagram with new connection
            let bastion = Self::resolve_bastion_from_widgets(
                &state_c,
                &jump_host_row_c,
                &jump_host_ids_c,
                row,
                &conn_ids_c,
            );
            let target = Self::resolve_target_from_widgets(&state_c, row, &conn_ids_c);
            diagram_c.update(None, bastion.as_deref(), target.as_deref(), None, None);
        });

        // Wire diagram update on jump host selection change
        let diagram_j = page_obj.diagram.clone();
        let state_j = page_obj.state.clone();
        let jump_host_ids_j = page_obj.jump_host_ids.clone();
        let conn_row_j = page_obj.connection_row.clone();
        let conn_ids_j = page_obj.filtered_connection_ids.clone();
        page_obj
            .jump_host_row
            .connect_selected_notify(move |jh_row| {
                let bastion = Self::resolve_bastion_from_widgets(
                    &state_j,
                    jh_row,
                    &jump_host_ids_j,
                    &conn_row_j,
                    &conn_ids_j,
                );
                let target = Self::resolve_target_from_widgets(&state_j, &conn_row_j, &conn_ids_j);
                diagram_j.update(None, bastion.as_deref(), target.as_deref(), None, None);
            });

        // Wire search filter with 150ms debounce
        let conn_row_s = page_obj.connection_row.clone();
        let conn_ids_s = page_obj.filtered_connection_ids.clone();
        let all_conns_s = page_obj.all_ssh_connections.clone();
        let timeout_id_s = page_obj.search_timeout_id.clone();
        page_obj.search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            let conn_row_inner = conn_row_s.clone();
            let conn_ids_inner = conn_ids_s.clone();
            let all_conns_inner = all_conns_s.clone();

            // Cancel previous timeout
            if let Some(id) = timeout_id_s.borrow_mut().take() {
                id.remove();
            }

            // Debounce 150ms
            let timeout_id_inner = timeout_id_s.clone();
            let source_id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                    Self::apply_filter(&query, &all_conns_inner, &conn_ids_inner, &conn_row_inner);
                    *timeout_id_inner.borrow_mut() = None;
                });
            *timeout_id_s.borrow_mut() = Some(source_id);
        });

        page_obj
    }

    /// Registers a callback for the "Next" button
    pub fn connect_next<F: Fn() + 'static>(&self, f: F) {
        *self.on_next.borrow_mut() = Some(Box::new(f));
    }

    /// Registers a callback for the "New SSH Connection" button
    pub fn connect_new_connection<F: Fn() + 'static>(&self, f: F) {
        *self.on_new_connection.borrow_mut() = Some(Box::new(f));
    }

    /// Pre-populates the page with an existing connection (edit mode)
    pub fn set_connection(&self, conn: &Connection) {
        // Find the connection in the filtered list and select it
        let ids = self.filtered_connection_ids.borrow();
        if let Some(idx) = ids.iter().position(|id| *id == conn.id) {
            self.connection_row.set_selected(idx as u32);
        }
    }

    /// Sets the tunnel name
    pub fn set_tunnel_name(&self, name: &str) {
        self.name_row.set_text(name);
    }

    /// Sets the page title (e.g., "Edit Tunnel" in edit mode)
    pub fn set_title(&self, title: &str) {
        self.page.set_title(title);
    }

    /// Returns the selected SSH connection ID, if any
    #[must_use]
    pub fn selected_connection_id(&self) -> Option<Uuid> {
        let idx = self.connection_row.selected() as usize;
        let ids = self.filtered_connection_ids.borrow();
        ids.get(idx).copied()
    }

    /// Returns the entered tunnel name
    #[must_use]
    pub fn tunnel_name(&self) -> String {
        self.name_row.text().trim().to_string()
    }

    /// Returns the selected bastion/jump host connection, if any
    #[must_use]
    pub fn bastion_connection(&self) -> Option<Connection> {
        // First check manual jump host override
        let jump_idx = self.jump_host_row.selected() as usize;
        let jh_ids_ref = self.jump_host_ids.borrow();
        if let Some(Some(bastion_id)) = jh_ids_ref.get(jump_idx) {
            let state_ref = self.state.borrow();
            return state_ref.get_connection(*bastion_id).cloned();
        }

        // Then check selected connection's jump_host_id
        if let Some(conn_id) = self.selected_connection_id() {
            let state_ref = self.state.borrow();
            if let Some(conn) = state_ref.get_connection(conn_id)
                && let ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config
                && let Some(jh_id) = ssh_cfg.jump_host_id
            {
                return state_ref.get_connection(jh_id).cloned();
            }
        }

        None
    }

    /// Refreshes the connection lists from state
    pub fn refresh_connections(&mut self) {
        let ssh_connections = {
            let state_ref = self.state.borrow();
            let mut conns: Vec<Connection> = state_ref
                .list_connections()
                .into_iter()
                .filter(|c| c.protocol == ProtocolType::Ssh)
                .cloned()
                .collect();
            conns.sort_by_key(|c| c.name.to_lowercase());
            conns
        };

        let is_empty = ssh_connections.is_empty();
        *self.all_ssh_connections.borrow_mut() = ssh_connections;

        // Show/hide empty state. The footer's Next button is hidden together
        // with the content so the empty-state CTA is the page's only visible
        // suggested action (GNOME HIG: one primary action per page).
        self.empty_state.set_visible(is_empty);
        self.content_box.set_visible(!is_empty);
        self.next_button.set_visible(!is_empty);

        // Apply current filter (or show all)
        let query = self.search_entry.text().to_string();
        Self::apply_filter(
            &query,
            &self.all_ssh_connections,
            &self.filtered_connection_ids,
            &self.connection_row,
        );

        // Populate jump host list
        self.populate_jump_hosts();
    }

    /// Populates the jump host ComboRow with "(None)" + all SSH connections
    fn populate_jump_hosts(&self) {
        let all_conns = self.all_ssh_connections.borrow();
        let mut ids: Vec<Option<Uuid>> = vec![None];
        let mut names: Vec<String> = vec![i18n("(None)")];

        for conn in all_conns.iter() {
            ids.push(Some(conn.id));
            names.push(format!("{} ({})", conn.name, conn.host));
        }

        let strings: Vec<&str> = names.iter().map(String::as_str).collect();
        let model = StringList::new(&strings);
        self.jump_host_row.set_model(Some(&model));
        self.jump_host_row.set_selected(0);
        *self.jump_host_ids.borrow_mut() = ids;
    }

    /// Applies the search filter to the connection combo
    fn apply_filter(
        query: &str,
        all_conns: &Rc<RefCell<Vec<Connection>>>,
        filtered_ids: &Rc<RefCell<Vec<Uuid>>>,
        combo_row: &adw::ComboRow,
    ) {
        let all = all_conns.borrow();
        let query_lower = query.to_lowercase();

        let filtered: Vec<&Connection> = if query_lower.is_empty() {
            all.iter().collect()
        } else {
            all.iter()
                .filter(|c| {
                    c.name.to_lowercase().contains(&query_lower)
                        || c.host.to_lowercase().contains(&query_lower)
                })
                .collect()
        };

        let mut ids = Vec::with_capacity(filtered.len());
        let mut names = Vec::with_capacity(filtered.len());

        for conn in &filtered {
            ids.push(conn.id);
            let display = if let Some(ref user) = conn.username {
                format!("{} ({}@{})", conn.name, user, conn.host)
            } else {
                format!("{} ({})", conn.name, conn.host)
            };
            names.push(display);
        }

        let strings: Vec<&str> = names.iter().map(String::as_str).collect();
        let model = StringList::new(&strings);
        combo_row.set_model(Some(&model));

        if !ids.is_empty() {
            combo_row.set_selected(0);
        }

        *filtered_ids.borrow_mut() = ids;
    }

    /// Validates the tunnel name (1–128 chars)
    fn validate_name(name_row: &adw::EntryRow) -> bool {
        let text = name_row.text();
        let trimmed = text.trim();
        let len = trimmed.len();
        let valid = (1..=128).contains(&len);

        if !valid && !trimmed.is_empty() {
            name_row.add_css_class("error");
        } else {
            name_row.remove_css_class("error");
        }

        valid
    }

    /// Validates that a connection is selected
    fn validate_connection(
        combo_row: &adw::ComboRow,
        filtered_ids: &Rc<RefCell<Vec<Uuid>>>,
    ) -> bool {
        let ids = filtered_ids.borrow();
        let idx = combo_row.selected() as usize;
        idx < ids.len()
    }

    /// Updates the embedded diagram with current selections
    pub fn update_diagram(&self) {
        let bastion_label = self.resolve_bastion_label();
        let target_label = self.resolve_target_label();

        self.diagram.update(
            None, // local_port not known at step 1
            bastion_label.as_deref(),
            target_label.as_deref(),
            None, // target_port not known at step 1
            None, // direction not known at step 1
        );
    }

    /// Resolves the bastion host label from jump host override or connection's jump_host_id/proxy_jump
    fn resolve_bastion_label(&self) -> Option<String> {
        // First check manual jump host override
        let jump_idx = self.jump_host_row.selected() as usize;
        let jh_ids_ref = self.jump_host_ids.borrow();
        if let Some(Some(bastion_id)) = jh_ids_ref.get(jump_idx) {
            let state_ref = self.state.borrow();
            if let Some(bastion_conn) = state_ref.get_connection(*bastion_id) {
                return Some(format!("{} ({})", bastion_conn.name, bastion_conn.host));
            }
        }

        // Then check selected connection's jump_host_id or proxy_jump
        if let Some(conn_id) = self.selected_connection_id() {
            let state_ref = self.state.borrow();
            if let Some(conn) = state_ref.get_connection(conn_id)
                && let ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config
            {
                // Check jump_host_id first
                if let Some(jh_id) = ssh_cfg.jump_host_id
                    && let Some(jh_conn) = state_ref.get_connection(jh_id)
                {
                    return Some(format!("{} ({})", jh_conn.name, jh_conn.host));
                }
                // Fall back to proxy_jump string
                if let Some(ref pj) = ssh_cfg.proxy_jump {
                    return Some(pj.clone());
                }
            }
        }

        None
    }

    /// Resolves the target host label from the selected connection
    fn resolve_target_label(&self) -> Option<String> {
        if let Some(conn_id) = self.selected_connection_id() {
            let state_ref = self.state.borrow();
            if let Some(conn) = state_ref.get_connection(conn_id) {
                return Some(conn.host.clone());
            }
        }
        None
    }

    /// Sets the jump host override to a specific connection ID
    pub fn set_jump_host(&self, connection_id: Option<Uuid>) {
        let jump_ids = self.jump_host_ids.borrow();
        if let Some(target_id) = connection_id {
            if let Some(idx) = jump_ids.iter().position(|id| *id == Some(target_id)) {
                self.jump_host_row.set_selected(idx as u32);
            }
        } else {
            self.jump_host_row.set_selected(0);
        }
    }

    /// Resolves bastion label from widget state (static version for signal closures)
    fn resolve_bastion_from_widgets(
        state: &SharedAppState,
        jump_host_row: &adw::ComboRow,
        jump_host_ids: &Rc<RefCell<Vec<Option<Uuid>>>>,
        conn_row: &adw::ComboRow,
        conn_ids: &Rc<RefCell<Vec<Uuid>>>,
    ) -> Option<String> {
        // First check manual jump host override
        let jump_idx = jump_host_row.selected() as usize;
        let jh_ids = jump_host_ids.borrow();
        if let Some(Some(bastion_id)) = jh_ids.get(jump_idx) {
            let state_ref = state.borrow();
            if let Some(bastion_conn) = state_ref.get_connection(*bastion_id) {
                return Some(bastion_conn.host.clone());
            }
        }

        // Then check selected connection's jump_host_id or proxy_jump
        let idx = conn_row.selected() as usize;
        let ids = conn_ids.borrow();
        if let Some(conn_id) = ids.get(idx) {
            let state_ref = state.borrow();
            if let Some(conn) = state_ref.get_connection(*conn_id)
                && let ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config
            {
                if let Some(jh_id) = ssh_cfg.jump_host_id
                    && let Some(jh_conn) = state_ref.get_connection(jh_id)
                {
                    return Some(jh_conn.host.clone());
                }
                if let Some(ref pj) = ssh_cfg.proxy_jump {
                    return Some(pj.clone());
                }
            }
        }

        None
    }

    /// Resolves target host label from widget state (static version for signal closures)
    fn resolve_target_from_widgets(
        state: &SharedAppState,
        conn_row: &adw::ComboRow,
        conn_ids: &Rc<RefCell<Vec<Uuid>>>,
    ) -> Option<String> {
        let idx = conn_row.selected() as usize;
        let ids = conn_ids.borrow();
        if let Some(conn_id) = ids.get(idx) {
            let state_ref = state.borrow();
            if let Some(conn) = state_ref.get_connection(*conn_id) {
                return Some(conn.host.clone());
            }
        }
        None
    }
}
