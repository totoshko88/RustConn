//! Populating the dialog from an existing `Connection`
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use crate::dialogs::connection::ssh;
use crate::i18n::{i18n, i18n_f};
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, FileDialog, Label, ListBox, Orientation, StringList};
use libadwaita as adw;
use rustconn_core::activity_monitor::MonitorMode;
use rustconn_core::automation::{ConnectionTask, ExpectRule};
use rustconn_core::models::{
    Connection, CustomProperty, HighlightRule, PasswordSource, ProtocolConfig, RdpConfig,
    SpiceConfig, SpiceImageCompression, SshAuthMethod, SshConfig, SshKeySource, VncConfig,
    ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
};
use rustconn_core::session::LogConfig;
use rustconn_core::variables::Variable;
use rustconn_core::wol::{
    DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS, WolConfig,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;

use super::{ConnectionDialog, klid_to_dropdown_index};

impl ConnectionDialog {
    /// Sets up the file chooser button for SSH key selection using portal
    pub fn setup_key_file_chooser(&self, parent_window: Option<&gtk4::Window>) {
        let key_entry = self.ssh_key_entry.clone();
        let parent = parent_window.cloned();

        self.ssh_key_button.connect_clicked(move |_| {
            let file_dialog = FileDialog::builder()
                .title(i18n("Select SSH Key"))
                .modal(true)
                .build();

            // Set initial folder to ~/.ssh if it exists
            if let Some(home) = std::env::var_os("HOME") {
                let ssh_dir = PathBuf::from(home).join(".ssh");
                if ssh_dir.exists() {
                    let gio_file = gtk4::gio::File::for_path(&ssh_dir);
                    file_dialog.set_initial_folder(Some(&gio_file));
                }
            }

            let entry = key_entry.clone();
            file_dialog.open(
                parent.as_ref(),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        // In Flatpak, the file chooser returns document portal paths
                        // like /run/user/1000/doc/XXXXXXXX/key.pem which become stale
                        // after rebuilds. Copy the key to a stable location.
                        let stable_path = if rustconn_core::is_flatpak()
                            && rustconn_core::is_portal_path(&path)
                        {
                            rustconn_core::copy_key_to_flatpak_ssh(&path)
                                .unwrap_or_else(|| path.clone())
                        } else {
                            path
                        };
                        entry.set_text(&stable_path.to_string_lossy());
                    }
                },
            );
        });
    }

    /// Populates the dialog with an existing connection for editing
    pub fn set_connection(&self, conn: &Connection) {
        self.dialog.set_title(&i18n("Edit Connection"));
        // Switch from Create icon to Save icon for edit mode
        self.save_button.set_label("");
        self.save_button.set_icon_name("media-floppy-symbolic");
        self.save_button.set_tooltip_text(Some(&i18n("Save")));
        self.save_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Save"))]);
        *self.editing_id.borrow_mut() = Some(conn.id);

        // Basic fields
        self.name_entry.set_text(&conn.name);
        self.icon_entry.set_text(conn.icon.as_deref().unwrap_or(""));
        if let Some(ref description) = conn.description {
            self.description_view.buffer().set_text(description);
        } else {
            self.description_view.buffer().set_text("");
        }
        self.host_entry.set_text(&conn.host);
        self.port_spin.set_value(f64::from(conn.port));
        if let Some(ref username) = conn.username {
            self.username_entry.set_text(username);
        }
        if let Some(ref domain) = conn.domain {
            self.domain_entry.set_text(domain);
        }
        // Filter out desc: tags for backward compatibility with old imports
        let display_tags: Vec<&str> = conn
            .tags
            .iter()
            .filter(|t| !t.starts_with("desc:"))
            .map(String::as_str)
            .collect();
        self.tags_entry.set_text(&display_tags.join(", "));

        // If connection has desc: tag but no description field, extract it
        if conn.description.is_none()
            && let Some(desc_tag) = conn.tags.iter().find(|t| t.starts_with("desc:"))
        {
            self.description_view
                .buffer()
                .set_text(desc_tag.strip_prefix("desc:").unwrap_or(""));
        }

        // Set group selection
        if let Some(group_id) = conn.group_id {
            let groups_data = self.groups_data.borrow();
            if let Some(idx) = groups_data.iter().position(|(id, _)| *id == Some(group_id)) {
                self.group_dropdown.set_selected(idx as u32);
            }
        } else {
            self.group_dropdown.set_selected(0); // Root
        }

        // Password source - map enum to dropdown index
        // Dropdown order: Prompt(0), Vault(1), Variable(2), Inherit(3), None(4)
        let password_source_idx = match conn.password_source {
            PasswordSource::Prompt => 0,
            PasswordSource::Vault => 1,
            PasswordSource::Variable(_) => 2,
            PasswordSource::Inherit => 3,
            PasswordSource::Script(_) => 5,
            PasswordSource::None => 4,
        };
        self.password_source_dropdown
            .set_selected(password_source_idx);

        // If Variable source, select the matching variable in dropdown
        if let PasswordSource::Variable(ref var_name) = conn.password_source
            && let Some(model) = self.variable_dropdown.model()
            && let Some(sl) = model.downcast_ref::<gtk4::StringList>()
        {
            for i in 0..sl.n_items() {
                if sl.string(i).is_some_and(|s| s == *var_name) {
                    self.variable_dropdown.set_selected(i);
                    break;
                }
            }
        }

        // If Script source, populate the command entry
        if let PasswordSource::Script(ref cmd) = conn.password_source {
            self.script_command_entry.set_text(cmd);
        }

        // Protocol and protocol-specific fields
        match &conn.protocol_config {
            ProtocolConfig::Ssh(ssh) => {
                self.protocol_dropdown.set_selected(0); // SSH
                self.protocol_stack.set_visible_child_name("ssh");
                self.set_ssh_config(ssh);
                self.update_ssh_inherit_subtitle(conn.group_id);
            }
            ProtocolConfig::Rdp(rdp) => {
                self.protocol_dropdown.set_selected(1); // RDP
                self.protocol_stack.set_visible_child_name("rdp");
                self.set_rdp_config(rdp);
            }
            ProtocolConfig::Vnc(vnc) => {
                self.protocol_dropdown.set_selected(2); // VNC
                self.protocol_stack.set_visible_child_name("vnc");
                self.set_vnc_config(vnc);
            }
            ProtocolConfig::Spice(spice) => {
                self.protocol_dropdown.set_selected(3); // SPICE
                self.protocol_stack.set_visible_child_name("spice");
                self.set_spice_config(spice);
            }
            ProtocolConfig::ZeroTrust(zt) => {
                self.protocol_dropdown.set_selected(4); // Zero Trust
                self.protocol_stack.set_visible_child_name("zerotrust");
                self.set_zerotrust_config(zt);
            }
            ProtocolConfig::Telnet(telnet_config) => {
                self.protocol_dropdown.set_selected(5); // Telnet
                self.protocol_stack.set_visible_child_name("telnet");
                // Load custom args
                let args_text = telnet_config.custom_args.join(" ");
                self.telnet_custom_args_entry.set_text(&args_text);
                // Load keyboard settings
                self.telnet_backspace_dropdown
                    .set_selected(telnet_config.backspace_sends.index());
                self.telnet_delete_dropdown
                    .set_selected(telnet_config.delete_sends.index());
            }
            ProtocolConfig::Serial(serial_config) => {
                self.protocol_dropdown.set_selected(6); // Serial
                self.protocol_stack.set_visible_child_name("serial");
                self.serial_device_entry.set_text(&serial_config.device);
                self.serial_baud_dropdown
                    .set_selected(serial_config.baud_rate.index());
                self.serial_data_bits_dropdown
                    .set_selected(serial_config.data_bits.index());
                self.serial_stop_bits_dropdown
                    .set_selected(serial_config.stop_bits.index());
                self.serial_parity_dropdown
                    .set_selected(serial_config.parity.index());
                self.serial_flow_control_dropdown
                    .set_selected(serial_config.flow_control.index());
                let args_text = serial_config.custom_args.join(" ");
                self.serial_custom_args_entry.set_text(&args_text);
            }
            ProtocolConfig::Sftp(ssh) => {
                self.protocol_dropdown.set_selected(7); // SFTP
                self.protocol_stack.set_visible_child_name("ssh");
                self.set_ssh_config(ssh);
                self.update_ssh_inherit_subtitle(conn.group_id);
            }
            ProtocolConfig::Kubernetes(k8s) => {
                self.protocol_dropdown.set_selected(8); // Kubernetes
                self.protocol_stack.set_visible_child_name("kubernetes");
                self.set_kubernetes_config(k8s);
            }
            ProtocolConfig::Mosh(mosh_config) => {
                self.protocol_dropdown.set_selected(9); // MOSH
                // MOSH uses SSH tab — protocol dropdown handler shows mosh_settings_group
                self.set_mosh_config(mosh_config);
            }
            ProtocolConfig::Web(web_config) => {
                self.protocol_dropdown.set_selected(10); // Web
                self.protocol_stack.set_visible_child_name("web");
                self.set_web_config(web_config);
            }
        }

        // Set local variables
        self.set_local_variables(&conn.local_variables);

        // Set log config
        self.set_log_config(conn.log_config.as_ref());

        // Set expect rules
        self.set_expect_rules(&conn.automation.expect_rules);

        // Set connection tasks
        self.set_pre_connect_task(conn.pre_connect_task.as_ref());
        self.set_post_disconnect_task(conn.post_disconnect_task.as_ref());

        // Set custom properties
        self.set_custom_properties(&conn.custom_properties);

        // Set WOL config
        self.set_wol_config(conn.wol_config.as_ref());

        // Set terminal theme override
        if let Some(ref theme) = conn.theme_override {
            if let Some(ref bg) = theme.background
                && let Some(rgba) = crate::dialogs::connection::advanced_tab::hex_to_rgba(bg)
            {
                self.theme_bg_button.set_rgba(&rgba);
            }
            if let Some(ref fg) = theme.foreground
                && let Some(rgba) = crate::dialogs::connection::advanced_tab::hex_to_rgba(fg)
            {
                self.theme_fg_button.set_rgba(&rgba);
            }
            if let Some(ref cur) = theme.cursor
                && let Some(rgba) = crate::dialogs::connection::advanced_tab::hex_to_rgba(cur)
            {
                self.theme_cursor_button.set_rgba(&rgba);
            }
            self.theme_preview.queue_draw();
        }

        // Set remote monitoring toggle
        // If monitoring_config has enabled=Some(false), toggle is OFF.
        // Otherwise (None or enabled=Some(true)), toggle is ON.
        let mon_enabled = conn
            .monitoring_config
            .as_ref()
            .and_then(|mc| mc.enabled)
            .unwrap_or(true);
        self.monitoring_toggle.set_active(mon_enabled);

        // Set session recording toggle
        self.recording_toggle
            .set_active(conn.session_recording_enabled);

        // Set skip-port-check toggle (per-connection override)
        self.skip_port_check_toggle.set_active(conn.skip_port_check);

        // Set port knock sequence entry
        if let Some(ref knock_seq) = conn.knock_sequence {
            self.knock_sequence_entry.set_text(&knock_seq.display());
        } else {
            self.knock_sequence_entry.set_text("");
        }

        // Set SPA (fwknop) config
        if let Some(ref spa_cfg) = conn.spa_config {
            self.spa_enabled_toggle.set_active(true);
            if let Some(ref rij) = spa_cfg.rijndael_key_ref {
                self.spa_rij_key_entry.set_text(rij);
            }
            if let Some(ref hmac) = spa_cfg.hmac_key_ref {
                self.spa_hmac_key_entry.set_text(hmac);
            }
            self.spa_access_entry.set_text(&spa_cfg.access);
            self.spa_port_spin.set_value(f64::from(spa_cfg.dest_port));
            let allow_ip_idx = match &spa_cfg.allow_ip {
                rustconn_core::connection::knock::SpaAllowIp::SourceIp => 0,
                rustconn_core::connection::knock::SpaAllowIp::ResolvePublic => 1,
                rustconn_core::connection::knock::SpaAllowIp::Explicit(_) => 2,
            };
            self.spa_allow_ip_combo.set_selected(allow_ip_idx);
        } else {
            self.spa_enabled_toggle.set_active(false);
        }

        // Set highlight rules
        self.set_highlight_rules(&conn.highlight_rules);

        // Set activity monitor config
        if let Some(ref config) = conn.activity_monitor_config {
            let mode_idx = match config.mode {
                Some(MonitorMode::Activity) => 1,
                Some(MonitorMode::Silence) => 2,
                _ => 0,
            };
            self.activity_mode_combo.set_selected(mode_idx);
            if let Some(quiet) = config.quiet_period_secs {
                self.activity_quiet_period_spin.set_value(f64::from(quiet));
            }
            if let Some(silence) = config.silence_timeout_secs {
                self.activity_silence_timeout_spin
                    .set_value(f64::from(silence));
            }
        } else {
            self.activity_mode_combo.set_selected(0);
            self.activity_quiet_period_spin.set_value(10.0);
            self.activity_silence_timeout_spin.set_value(30.0);
        }

        // Set retry config
        if let Some(ref config) = conn.retry_config {
            self.retry_enabled_toggle.set_active(config.enabled);
            self.retry_max_attempts_spin
                .set_value(f64::from(config.max_attempts));
            #[expect(
                clippy::cast_precision_loss,
                reason = "f64 conversion is intentional for display/UI arithmetic where sub-integer precision is irrelevant"
            )]
            self.retry_initial_delay_spin
                .set_value(config.initial_delay_ms as f64);
            #[expect(
                clippy::cast_precision_loss,
                reason = "f64 conversion is intentional for display/UI arithmetic where sub-integer precision is irrelevant"
            )]
            self.retry_max_delay_spin
                .set_value(config.max_delay_ms as f64);
        } else {
            self.retry_enabled_toggle.set_active(true);
            self.retry_max_attempts_spin.set_value(3.0);
            self.retry_initial_delay_spin.set_value(1000.0);
            self.retry_max_delay_spin.set_value(30_000.0);
        }
    }

    /// Sets the available groups for the group dropdown
    ///
    /// Groups are displayed in a flat list with hierarchy indicated by indentation.
    /// The first item is always "(Root)" for connections without a group.
    #[expect(
        clippy::items_after_statements,
        reason = "local helper introduced inline next to its only call site; hoisting would scatter related logic"
    )]
    pub fn set_groups(&self, groups: &[rustconn_core::models::ConnectionGroup]) {
        use rustconn_core::models::ConnectionGroup;

        // Populate full_groups_data
        {
            let mut full_map = self.full_groups_data.borrow_mut();
            full_map.clear();
            for group in groups {
                full_map.insert(group.id, group.clone());
            }
        }

        // Build hierarchical group list
        let mut groups_data: Vec<(Option<Uuid>, String)> = vec![(None, i18n("(Root)"))];

        // Helper to add groups recursively with indentation
        fn add_group_recursive(
            group: &ConnectionGroup,
            all_groups: &[ConnectionGroup],
            groups_data: &mut Vec<(Option<Uuid>, String)>,
            depth: usize,
        ) {
            let indent = "  ".repeat(depth);
            groups_data.push((Some(group.id), format!("{}{}", indent, group.name)));

            // Find and add children
            let children: Vec<_> = all_groups
                .iter()
                .filter(|g| g.parent_id == Some(group.id))
                .collect();
            for child in children {
                add_group_recursive(child, all_groups, groups_data, depth + 1);
            }
        }

        // Start with root groups (no parent)
        let root_groups: Vec<_> = groups.iter().filter(|g| g.parent_id.is_none()).collect();
        for group in root_groups {
            add_group_recursive(group, groups, &mut groups_data, 0);
        }

        self.set_groups_list(&groups_data);
    }

    /// Sets the available connections for the jump host dropdown
    pub fn set_connections(&self, connections: &[Connection]) {
        use rustconn_core::models::ProtocolType;

        let mut connections_data: Vec<(Option<Uuid>, String)> = vec![(None, "(None)".to_string())];

        let mut sorted_conns: Vec<&Connection> = connections
            .iter()
            .filter(|c| c.protocol == ProtocolType::Ssh)
            .collect();
        sorted_conns.sort_by_key(|a| a.name.to_lowercase());

        for conn in sorted_conns {
            // Avoid duplicating the host when the connection name IS the host
            let label = if conn.name == conn.host {
                conn.name.clone()
            } else {
                format!("{} ({})", conn.name, conn.host)
            };
            // Truncate long labels to prevent the dropdown from stretching the dialog
            let label = if label.chars().count() > 50 {
                let truncated: String = label.chars().take(49).collect();
                format!("{truncated}…")
            } else {
                label
            };
            connections_data.push((Some(conn.id), label));
        }

        *self.connections_data.borrow_mut() = connections_data.clone();

        let display_strings: Vec<&str> = connections_data
            .iter()
            .map(|(_, name)| name.as_str())
            .collect();
        let model = StringList::new(&display_strings);
        self.ssh_jump_host_dropdown.set_model(Some(&model));

        // Also populate the RDP jump host dropdown with the same SSH connections
        *self.rdp_connections_data.borrow_mut() = connections_data.clone();
        let rdp_model = StringList::new(&display_strings);
        self.rdp_jump_host_dropdown.set_model(Some(&rdp_model));

        *self.vnc_connections_data.borrow_mut() = connections_data.clone();
        let vnc_model = StringList::new(&display_strings);
        self.vnc_jump_host_dropdown.set_model(Some(&vnc_model));

        *self.spice_connections_data.borrow_mut() = connections_data.clone();
        let spice_model = StringList::new(&display_strings);
        self.spice_jump_host_dropdown.set_model(Some(&spice_model));
    }

    pub(super) fn set_groups_list(&self, groups_data: &[(Option<Uuid>, String)]) {
        // Update dropdown model
        let names: Vec<&str> = groups_data.iter().map(|(_, name)| name.as_str()).collect();
        let string_list = StringList::new(&names);
        self.group_dropdown.set_model(Some(&string_list));

        // Store groups data for later lookup
        *self.groups_data.borrow_mut() = groups_data.to_vec();
    }

    /// Pre-selects a group in the group dropdown by its UUID
    pub fn set_selected_group(&self, group_id: Uuid) {
        let groups_data = self.groups_data.borrow();
        if let Some(idx) = groups_data.iter().position(|(id, _)| *id == Some(group_id)) {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "value range fits the target type by construction in this code path"
            )]
            self.group_dropdown.set_selected(idx as u32);
        }
    }

    /// Sets the WOL configuration fields
    pub(super) fn set_wol_config(&self, config: Option<&WolConfig>) {
        // Note: individual widget sensitivity is controlled by wol_settings_group
        // via the connect_toggled handler on wol_enabled_check.
        // Do NOT set_sensitive on individual widgets here — it conflicts with
        // the group-level sensitivity and leaves widgets disabled after toggling.
        if let Some(wol) = config {
            self.wol_enabled_check.set_active(true);
            self.wol_mac_entry.set_text(&wol.mac_address.to_string());
            self.wol_broadcast_entry.set_text(&wol.broadcast_address);
            self.wol_port_spin.set_value(f64::from(wol.port));
            self.wol_wait_spin.set_value(f64::from(wol.wait_seconds));
        } else {
            self.wol_enabled_check.set_active(false);
            self.wol_mac_entry.set_text("");
            self.wol_broadcast_entry.set_text(DEFAULT_BROADCAST_ADDRESS);
            self.wol_port_spin.set_value(f64::from(DEFAULT_WOL_PORT));
            self.wol_wait_spin
                .set_value(f64::from(DEFAULT_WOL_WAIT_SECONDS));
        }
    }

    /// Sets the custom properties for this connection
    pub(super) fn set_custom_properties(&self, properties: &[CustomProperty]) {
        // Clear existing rows
        while let Some(row) = self.custom_properties_list.row_at_index(0) {
            self.custom_properties_list.remove(&row);
        }
        self.custom_properties.borrow_mut().clear();

        // Add rows for each property
        for property in properties {
            self.add_custom_property_row(Some(property));
        }
    }

    /// Adds a custom property row to the list
    pub(super) fn add_custom_property_row(&self, property: Option<&CustomProperty>) {
        let prop_row = Self::create_custom_property_row(property);

        // Add the property to the list
        let new_prop = if let Some(p) = property {
            p.clone()
        } else {
            CustomProperty::new_text("", "")
        };
        self.custom_properties.borrow_mut().push(new_prop);
        let prop_index = self.custom_properties.borrow().len() - 1;

        // Connect delete button
        let list_for_delete = self.custom_properties_list.clone();
        let props_for_delete = self.custom_properties.clone();
        let row_widget = prop_row.row.clone();
        prop_row.delete_button.connect_clicked(move |_| {
            // Find and remove the property by matching the row index
            if let Ok(idx) = usize::try_from(row_widget.index())
                && idx < props_for_delete.borrow().len()
            {
                props_for_delete.borrow_mut().remove(idx);
            }
            list_for_delete.remove(&row_widget);
        });

        // Connect entry changes to update the property
        Self::connect_custom_property_changes(&prop_row, &self.custom_properties, prop_index);

        self.custom_properties_list.append(&prop_row.row);
    }

    /// Sets the pre-connect task fields
    pub(super) fn set_pre_connect_task(&self, task: Option<&ConnectionTask>) {
        if let Some(task) = task {
            self.pre_connect_enabled_switch.set_active(true);
            self.pre_connect_command_entry.set_text(&task.command);
            self.pre_connect_command_entry.set_sensitive(true);
            self.pre_connect_timeout_spin
                .set_value(f64::from(task.timeout_ms.unwrap_or(0)));
            self.pre_connect_timeout_spin.set_sensitive(true);
            self.pre_connect_abort_switch
                .set_active(task.abort_on_failure);
            self.pre_connect_abort_switch.set_sensitive(true);
            self.pre_connect_first_only_switch
                .set_active(task.condition.only_first_in_folder);
            self.pre_connect_first_only_switch.set_sensitive(true);
        } else {
            self.pre_connect_enabled_switch.set_active(false);
            self.pre_connect_command_entry.set_text("");
            self.pre_connect_command_entry.set_sensitive(false);
            self.pre_connect_timeout_spin.set_value(0.0);
            self.pre_connect_timeout_spin.set_sensitive(false);
            self.pre_connect_abort_switch.set_active(true);
            self.pre_connect_abort_switch.set_sensitive(false);
            self.pre_connect_first_only_switch.set_active(false);
            self.pre_connect_first_only_switch.set_sensitive(false);
        }
    }

    /// Sets the post-disconnect task fields
    pub(super) fn set_post_disconnect_task(&self, task: Option<&ConnectionTask>) {
        if let Some(task) = task {
            self.post_disconnect_enabled_switch.set_active(true);
            self.post_disconnect_command_entry.set_text(&task.command);
            self.post_disconnect_command_entry.set_sensitive(true);
            self.post_disconnect_timeout_spin
                .set_value(f64::from(task.timeout_ms.unwrap_or(0)));
            self.post_disconnect_timeout_spin.set_sensitive(true);
            self.post_disconnect_last_only_switch
                .set_active(task.condition.only_last_in_folder);
            self.post_disconnect_last_only_switch.set_sensitive(true);
        } else {
            self.post_disconnect_enabled_switch.set_active(false);
            self.post_disconnect_command_entry.set_text("");
            self.post_disconnect_command_entry.set_sensitive(false);
            self.post_disconnect_timeout_spin.set_value(0.0);
            self.post_disconnect_timeout_spin.set_sensitive(false);
            self.post_disconnect_last_only_switch.set_active(false);
            self.post_disconnect_last_only_switch.set_sensitive(false);
        }
    }

    /// Sets the expect rules for this connection
    pub(super) fn set_expect_rules(&self, rules: &[ExpectRule]) {
        // Clear existing rows
        while let Some(row) = self.expect_rules_list.row_at_index(0) {
            self.expect_rules_list.remove(&row);
        }
        self.expect_rules.borrow_mut().clear();

        // Add rows for each rule
        for rule in rules {
            self.add_expect_rule_row(Some(rule));
        }
    }

    /// Adds an expect rule row to the list
    pub(super) fn add_expect_rule_row(&self, rule: Option<&ExpectRule>) {
        let rule_row = Self::create_expect_rule_row(rule);
        let rule_id = rule_row.id;

        // If we have an existing rule, use its ID; otherwise create a new one
        let new_rule = if let Some(r) = rule {
            r.clone()
        } else {
            ExpectRule::with_id(rule_id, "", "")
        };
        self.expect_rules.borrow_mut().push(new_rule);

        // Connect delete button
        let list_for_delete = self.expect_rules_list.clone();
        let rules_for_delete = self.expect_rules.clone();
        let row_widget = rule_row.row.clone();
        let delete_id = rule_id;
        rule_row.delete_button.connect_clicked(move |_| {
            list_for_delete.remove(&row_widget);
            rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
        });

        // Connect move up button
        let list_for_up = self.expect_rules_list.clone();
        let rules_for_up = self.expect_rules.clone();
        let row_for_up = rule_row.row.clone();
        let up_id = rule_id;
        rule_row.move_up_button.connect_clicked(move |_| {
            Self::move_rule_up(&list_for_up, &rules_for_up, &row_for_up, up_id);
        });

        // Connect move down button
        let list_for_down = self.expect_rules_list.clone();
        let rules_for_down = self.expect_rules.clone();
        let row_for_down = rule_row.row.clone();
        let down_id = rule_id;
        rule_row.move_down_button.connect_clicked(move |_| {
            Self::move_rule_down(&list_for_down, &rules_for_down, &row_for_down, down_id);
        });

        // Connect entry changes to update the rule
        Self::connect_rule_entry_changes(&rule_row, &self.expect_rules);

        self.expect_rules_list.append(&rule_row.row);
    }

    /// Sets the highlight rules for this connection
    pub(super) fn set_highlight_rules(&self, rules: &[HighlightRule]) {
        // Clear existing rows
        while let Some(row) = self.highlight_rules_list.row_at_index(0) {
            self.highlight_rules_list.remove(&row);
        }
        self.highlight_rules.borrow_mut().clear();

        // Add rows for each rule
        for rule in rules {
            self.add_highlight_rule_row(Some(rule));
        }
    }

    /// Adds a highlight rule row to the list
    pub(super) fn add_highlight_rule_row(&self, rule: Option<&HighlightRule>) {
        let hl_row = crate::dialogs::connection::advanced_tab::create_highlight_rule_row(rule);
        let rule_id = hl_row.id;

        let new_rule = if let Some(r) = rule {
            r.clone()
        } else {
            HighlightRule::new(String::new(), String::new())
        };
        // Ensure the stored rule has the same ID as the row
        let mut stored_rule = new_rule;
        stored_rule.id = rule_id;
        self.highlight_rules.borrow_mut().push(stored_rule);

        // Connect delete button
        let list_for_delete = self.highlight_rules_list.clone();
        let rules_for_delete = self.highlight_rules.clone();
        let row_widget = hl_row.row.clone();
        let delete_id = rule_id;
        hl_row.delete_button.connect_clicked(move |_| {
            list_for_delete.remove(&row_widget);
            rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
        });

        // Connect entry changes
        Self::connect_highlight_rule_changes(&hl_row, &self.highlight_rules);

        self.highlight_rules_list.append(&hl_row.row);
    }

    /// Wires up the add highlight rule button
    pub(super) fn wire_add_highlight_rule_button(
        add_button: &Button,
        highlight_rules_list: &ListBox,
        highlight_rules: &Rc<RefCell<Vec<HighlightRule>>>,
    ) {
        let list_clone = highlight_rules_list.clone();
        let rules_clone = highlight_rules.clone();

        add_button.connect_clicked(move |_| {
            let new_rule = HighlightRule::new(String::new(), String::new());
            let hl_row = crate::dialogs::connection::advanced_tab::create_highlight_rule_row(Some(
                &new_rule,
            ));
            let rule_id = new_rule.id;

            rules_clone.borrow_mut().push(new_rule);

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let rules_for_delete = rules_clone.clone();
            let row_widget = hl_row.row.clone();
            let delete_id = rule_id;
            hl_row.delete_button.connect_clicked(move |_| {
                list_for_delete.remove(&row_widget);
                rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
            });

            // Connect entry changes
            Self::connect_highlight_rule_changes(&hl_row, &rules_clone);

            list_clone.append(&hl_row.row);
        });
    }

    /// Connects highlight rule row entry changes to update the rule data
    pub(super) fn connect_highlight_rule_changes(
        hl_row: &crate::dialogs::connection::advanced_tab::HighlightRuleRow,
        highlight_rules: &Rc<RefCell<Vec<HighlightRule>>>,
    ) {
        let rule_id = hl_row.id;

        // Name entry
        let rules_for_name = highlight_rules.clone();
        let name_id = rule_id;
        hl_row.name_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let mut rules = rules_for_name.borrow_mut();
            if let Some(r) = rules.iter_mut().find(|r| r.id == name_id) {
                r.name = text;
            }
        });

        // Pattern entry
        let rules_for_pattern = highlight_rules.clone();
        let pattern_id = rule_id;
        hl_row.pattern_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let mut rules = rules_for_pattern.borrow_mut();
            if let Some(r) = rules.iter_mut().find(|r| r.id == pattern_id) {
                r.pattern = text;
            }
        });

        // Enabled checkbox
        let rules_for_enabled = highlight_rules.clone();
        let enabled_id = rule_id;
        hl_row.enabled_check.connect_toggled(move |check| {
            let active = check.is_active();
            let mut rules = rules_for_enabled.borrow_mut();
            if let Some(r) = rules.iter_mut().find(|r| r.id == enabled_id) {
                r.enabled = active;
            }
        });
    }

    /// Sets the log configuration for this connection
    pub(super) fn set_log_config(&self, log_config: Option<&LogConfig>) {
        self.logging_tab.set(log_config);
    }

    /// Sets the local variables for this connection
    pub(super) fn set_local_variables(&self, local_vars: &HashMap<String, Variable>) {
        // Clear existing rows
        while let Some(row) = self.variables_list.row_at_index(0) {
            self.variables_list.remove(&row);
        }
        self.variables_rows.borrow_mut().clear();

        // First, add inherited global variables that are overridden
        let global_vars = self.global_variables.borrow();
        for global_var in global_vars.iter() {
            if let Some(local_var) = local_vars.get(&global_var.name) {
                // This global variable is overridden locally
                self.add_local_variable_row(Some(local_var), true);
            }
        }

        // Then add local-only variables (not overriding globals)
        for (name, var) in local_vars {
            let is_global_override = global_vars.iter().any(|g| &g.name == name);
            if !is_global_override {
                self.add_local_variable_row(Some(var), false);
            }
        }
    }

    /// Adds a local variable row to the list
    pub(super) fn add_local_variable_row(&self, variable: Option<&Variable>, is_inherited: bool) {
        let var_row = Self::create_local_variable_row(variable, is_inherited);

        // Connect delete button
        let list_clone = self.variables_list.clone();
        let rows_clone = self.variables_rows.clone();
        let row_widget = var_row.row.clone();
        var_row.delete_button.connect_clicked(move |_| {
            list_clone.remove(&row_widget);
            let mut rows = rows_clone.borrow_mut();
            rows.retain(|r| r.row != row_widget);
        });

        self.variables_list.append(&var_row.row);
        self.variables_rows.borrow_mut().push(var_row);
    }

    pub(super) fn set_ssh_config(&self, ssh: &SshConfig) {
        let auth_idx = match ssh.auth_method {
            SshAuthMethod::Password => 0,
            SshAuthMethod::PublicKey => 1,
            SshAuthMethod::KeyboardInteractive => 2,
            SshAuthMethod::Agent => 3,
            SshAuthMethod::SecurityKey => 4,
        };
        self.ssh_auth_dropdown.set_selected(auth_idx);

        // Set key source dropdown and related fields
        match &ssh.key_source {
            SshKeySource::Default => {
                self.ssh_key_source_dropdown.set_selected(0);
                self.ssh_key_entry.set_sensitive(false);
                self.ssh_key_button.set_sensitive(false);
                self.ssh_agent_key_dropdown.set_sensitive(false);
            }
            SshKeySource::File { path } => {
                self.ssh_key_source_dropdown.set_selected(1);
                self.ssh_key_entry.set_text(&path.to_string_lossy());
                self.ssh_key_entry.set_sensitive(true);
                self.ssh_key_button.set_sensitive(true);
                self.ssh_agent_key_dropdown.set_sensitive(false);
            }
            SshKeySource::Agent {
                fingerprint,
                comment,
            } => {
                self.ssh_key_source_dropdown.set_selected(2);
                self.ssh_key_entry.set_sensitive(false);
                self.ssh_key_button.set_sensitive(false);
                self.ssh_agent_key_dropdown.set_sensitive(true);
                // Store pending selection for restore after refresh_agent_keys()
                *self.pending_agent_selection.borrow_mut() =
                    Some((fingerprint.clone(), comment.clone()));
                // Try to select the matching agent key in the dropdown
                self.select_agent_key_by_fingerprint(fingerprint, comment);
            }
            SshKeySource::Inherit => {
                // Inherit from parent group — index 3
                self.ssh_key_source_dropdown.set_selected(3);
                self.ssh_key_entry.set_sensitive(false);
                self.ssh_key_button.set_sensitive(false);
                self.ssh_agent_key_dropdown.set_sensitive(false);
            }
        }

        // Also set key_path for backward compatibility
        if let Some(ref key_path) = ssh.key_path
            && matches!(ssh.key_source, SshKeySource::Default)
        {
            // If key_source is Default but key_path is set, use File source
            self.ssh_key_source_dropdown.set_selected(1);
            self.ssh_key_entry.set_text(&key_path.to_string_lossy());
            self.ssh_key_entry.set_sensitive(true);
            self.ssh_key_button.set_sensitive(true);
        }

        if let Some(agent_fingerprint) = &ssh.agent_key_fingerprint {
            let keys = self.ssh_agent_keys.borrow();
            if let Some(pos) = keys
                .iter()
                .position(|k| k.fingerprint == *agent_fingerprint)
            {
                self.ssh_agent_key_dropdown.set_selected(pos as u32);
            }
        }

        // Set jump host dropdown
        if let Some(jump_id) = ssh.jump_host_id {
            let connections = self.connections_data.borrow();
            if let Some(pos) = connections.iter().position(|(id, _)| *id == Some(jump_id)) {
                self.ssh_jump_host_dropdown.set_selected(pos as u32);
            } else {
                self.ssh_jump_host_dropdown.set_selected(0);
            }
        } else {
            self.ssh_jump_host_dropdown.set_selected(0);
        }

        self.ssh_proxy_entry
            .set_text(ssh.proxy_jump.as_deref().unwrap_or(""));
        self.ssh_proxy_command_entry
            .set_text(ssh.proxy_command.as_deref().unwrap_or(""));
        self.ssh_identities_only.set_active(ssh.identities_only);
        self.ssh_control_master.set_active(ssh.use_control_master);
        self.ssh_agent_forwarding.set_active(ssh.agent_forwarding);
        self.ssh_waypipe.set_active(ssh.waypipe);
        self.ssh_x11_forwarding.set_active(ssh.x11_forwarding);
        self.ssh_compression.set_active(ssh.compression);
        self.ssh_verbose.set_active(ssh.verbose);
        if let Some(ref cmd) = ssh.startup_command {
            self.ssh_startup_entry.set_text(cmd);
        }

        // Load per-connection SSH agent socket
        if let Some(ref socket) = ssh.ssh_agent_socket {
            self.ssh_agent_socket_entry.set_text(socket);
        }

        // Load PKCS#11 provider library path
        if let Some(ref provider) = ssh.pkcs11_provider {
            self.ssh_pkcs11_entry.set_text(provider);
        }

        // Load SFTP remote directory override
        if let Some(ref remote_path) = ssh.remote_path {
            self.ssh_remote_path_entry.set_text(remote_path);
        }

        // Load keep-alive settings
        if let Some(interval) = ssh.keep_alive_interval {
            self.ssh_keep_alive_interval.set_value(f64::from(interval));
        } else {
            self.ssh_keep_alive_interval.set_value(0.0);
        }
        if let Some(count) = ssh.keep_alive_count_max {
            self.ssh_keep_alive_count_max.set_value(f64::from(count));
        } else {
            self.ssh_keep_alive_count_max.set_value(3.0);
        }

        // Format custom options as "Key=Value, Key2=Value2"
        if !ssh.custom_options.is_empty() {
            let opts: Vec<String> = ssh
                .custom_options
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            self.ssh_options_entry.set_text(&opts.join(", "));
        }

        // Populate port forwarding rules
        {
            let mut pf_list = self.ssh_port_forwards.borrow_mut();
            pf_list.clear();
            pf_list.extend(ssh.port_forwards.clone());
        }
        self.refresh_port_forwards_list();
    }

    /// Updates the SSH Key Source row subtitle to show the resolved inherited value
    /// when "Inherit from group" is selected.
    pub(super) fn update_ssh_inherit_subtitle(&self, group_id: Option<uuid::Uuid>) {
        use rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path;

        if self.ssh_key_source_dropdown.selected() == 3 {
            // Inherit is selected — resolve the inherited key path
            if let Some(gid) = group_id {
                let full_groups = self.full_groups_data.borrow();
                let groups: Vec<rustconn_core::models::ConnectionGroup> =
                    full_groups.values().cloned().collect();

                // Build a minimal connection to resolve the key path
                let mut tmp_conn =
                    rustconn_core::models::Connection::new_ssh("tmp".into(), "tmp".into(), 22);
                tmp_conn.group_id = Some(gid);
                if let rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) =
                    tmp_conn.protocol_config
                {
                    cfg.key_source = rustconn_core::models::SshKeySource::Inherit;
                }

                if let Some(resolved_path) = resolve_ssh_key_path(&tmp_conn, &groups) {
                    let resolved_str = resolved_path.to_string_lossy();
                    let subtitle = i18n_f("Inherited: {}", &[&resolved_str]);
                    self.ssh_key_source_row.set_subtitle(&subtitle);
                } else {
                    self.ssh_key_source_row
                        .set_subtitle(&i18n("Inherited from parent group"));
                }
            } else {
                self.ssh_key_source_row
                    .set_subtitle(&i18n("Inherited from parent group"));
            }
        } else {
            // Not Inherit — restore default subtitle
            self.ssh_key_source_row.set_subtitle(&i18n(
                "Default tries ~/.ssh/id_rsa, id_ed25519, id_ecdsa automatically",
            ));
        }
    }

    /// Refreshes the port forwarding list UI from the stored rules
    pub(super) fn refresh_port_forwards_list(&self) {
        // Remove all existing rows
        while let Some(child) = self.ssh_port_forwards_list.first_child() {
            self.ssh_port_forwards_list.remove(&child);
        }

        let forwards = self.ssh_port_forwards.borrow();
        for (idx, pf) in forwards.iter().enumerate() {
            ssh::add_port_forward_row_to_list(
                &self.ssh_port_forwards_list,
                &self.ssh_port_forwards,
                idx,
                pf,
            );
        }
    }

    /// Selects an agent key in the dropdown by fingerprint
    pub(super) fn select_agent_key_by_fingerprint(&self, fingerprint: &str, comment: &str) {
        let keys = self.ssh_agent_keys.borrow();
        for (idx, key) in keys.iter().enumerate() {
            if key.fingerprint == fingerprint || key.comment == comment {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value range fits the target type by construction in this code path"
                )]
                self.ssh_agent_key_dropdown.set_selected(idx as u32);
                return;
            }
        }
        // If not found, keep the first item selected (will show warning on connect)
    }

    pub(super) fn set_rdp_config(&self, rdp: &RdpConfig) {
        // Set client mode dropdown
        self.rdp_client_mode_dropdown
            .set_selected(rdp.client_mode.index());

        // Set performance mode dropdown
        self.rdp_performance_mode_dropdown
            .set_selected(rdp.performance_mode.index());

        // Set graphics pipeline mode dropdown
        // 0=Auto, 1=Legacy, 2=RemoteFx
        self.rdp_graphics_mode_dropdown
            .set_selected(match rdp.graphics_mode {
                rustconn_core::rdp_client::graphics::GraphicsMode::Legacy => 1,
                rustconn_core::rdp_client::graphics::GraphicsMode::RemoteFx => 2,
                _ => 0, // Auto and any GFX variant → Auto
            });

        if let Some(ref res) = rdp.resolution {
            self.rdp_width_spin.set_value(f64::from(res.width));
            self.rdp_height_spin.set_value(f64::from(res.height));
        }
        if let Some(depth) = rdp.color_depth {
            // Map color depth to dropdown index: 32->0, 24->1, 16->2, 15->3, 8->4
            let idx = match depth {
                24 => 1,
                16 => 2,
                15 => 3,
                8 => 4,
                _ => 0, // 32 and any other value default to 0
            };
            self.rdp_color_dropdown.set_selected(idx);
        }
        self.rdp_scale_override_dropdown
            .set_selected(rdp.scale_override.index());
        self.rdp_audio_check.set_active(rdp.audio_redirect);
        self.rdp_printer_check.set_active(rdp.printer_enabled);
        self.rdp_clipboard_check.set_active(rdp.clipboard_enabled);
        self.rdp_show_local_cursor_check
            .set_active(rdp.show_local_cursor);
        self.rdp_jiggler_check.set_active(rdp.jiggler_enabled);
        self.rdp_jiggler_interval_spin
            .set_value(f64::from(rdp.jiggler_interval_secs));
        self.rdp_jiggler_interval_spin
            .set_sensitive(rdp.jiggler_enabled);
        self.rdp_autotype_delay_spin
            .set_value(f64::from(rdp.autotype_delay_ms));
        self.rdp_autotype_initial_delay_spin
            .set_value(f64::from(rdp.autotype_initial_delay_ms));
        self.rdp_reconnect_on_resize_check
            .set_active(rdp.reconnect_on_resize);
        self.rdp_disable_nla_check.set_active(rdp.disable_nla);
        self.rdp_security_layer_dropdown
            .set_selected(rdp.security_layer.index());
        if let Some(level) = rdp.tls_security_level {
            self.rdp_tls_security_level_spin.set_value(f64::from(level));
        } else {
            self.rdp_tls_security_level_spin.set_value(2.0); // Default
        }
        self.rdp_ignore_certificate_check
            .set_active(rdp.ignore_certificate);
        if let Some(ref gw) = rdp.gateway {
            self.rdp_gateway_entry.set_text(&gw.hostname);
            self.rdp_gateway_port_spin.set_value(f64::from(gw.port));
            if let Some(ref username) = gw.username {
                self.rdp_gateway_username_entry.set_text(username);
            }
        }

        // Populate shared folders
        self.rdp_shared_folders.borrow_mut().clear();
        // Clear existing list items
        while let Some(row) = self.rdp_shared_folders_list.row_at_index(0) {
            self.rdp_shared_folders_list.remove(&row);
        }
        for folder in &rdp.shared_folders {
            self.rdp_shared_folders.borrow_mut().push(folder.clone());

            // Add to UI
            let row_box = GtkBox::new(Orientation::Horizontal, 8);
            row_box.set_margin_top(6);
            row_box.set_margin_bottom(6);
            row_box.set_margin_start(12);
            row_box.set_margin_end(12);

            let path_label = Label::builder()
                .label(folder.local_path.to_string_lossy().as_ref())
                .hexpand(true)
                .halign(gtk4::Align::Start)
                .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                .build();
            let name_label = Label::builder()
                .label(format!("→ {}", folder.share_name))
                .halign(gtk4::Align::End)
                .build();

            row_box.append(&path_label);
            row_box.append(&name_label);
            self.rdp_shared_folders_list.append(&row_box);
        }

        if !rdp.custom_args.is_empty() {
            self.rdp_custom_args_entry
                .set_text(&rdp.custom_args.join(" "));
        }

        // Set RemoteApp fields
        if let Some(ref program) = rdp.remote_app_program {
            self.rdp_remote_app_program_entry.set_text(program);
        }
        if let Some(ref args) = rdp.remote_app_args {
            self.rdp_remote_app_args_entry.set_text(args);
        }
        if let Some(ref name) = rdp.remote_app_name {
            self.rdp_remote_app_name_entry.set_text(name);
        }

        // Set keyboard layout dropdown
        if let Some(klid) = rdp.keyboard_layout {
            let index = klid_to_dropdown_index(klid);
            self.rdp_keyboard_layout_dropdown.set_selected(index);
        } else {
            self.rdp_keyboard_layout_dropdown.set_selected(0); // Auto
        }

        // Set jump host dropdown
        if let Some(jump_id) = rdp.jump_host_id {
            let conns = self.rdp_connections_data.borrow();
            if let Some(idx) = conns.iter().position(|(id, _)| *id == Some(jump_id)) {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value range fits the target type by construction in this code path"
                )]
                self.rdp_jump_host_dropdown.set_selected(idx as u32);
            }
        }
    }

    pub(super) fn set_vnc_config(&self, vnc: &VncConfig) {
        // Set client mode dropdown
        self.vnc_client_mode_dropdown
            .set_selected(vnc.client_mode.index());

        // Set performance mode dropdown
        self.vnc_performance_mode_dropdown
            .set_selected(vnc.performance_mode.index());

        // VNC-1: Map encoding string to dropdown index
        // Items: ["Auto", "Tight", "ZRLE", "Hextile", "Raw", "CopyRect"]
        let encoding_idx = match vnc.encoding.as_deref() {
            Some("tight") => 1,
            Some("zrle") => 2,
            Some("hextile") => 3,
            Some("raw") => 4,
            Some("copyrect") => 5,
            _ => 0, // Auto
        };
        self.vnc_encoding_dropdown.set_selected(encoding_idx);

        if let Some(comp) = vnc.compression {
            self.vnc_compression_spin.set_value(f64::from(comp));
        }
        if let Some(qual) = vnc.quality {
            self.vnc_quality_spin.set_value(f64::from(qual));
        }

        self.vnc_view_only_check.set_active(vnc.view_only);
        self.vnc_scaling_check.set_active(vnc.scaling);
        self.vnc_clipboard_check.set_active(vnc.clipboard_enabled);
        self.vnc_show_local_cursor_check
            .set_active(vnc.show_local_cursor);
        self.vnc_scale_override_dropdown
            .set_selected(vnc.scale_override.index());

        if !vnc.custom_args.is_empty() {
            self.vnc_custom_args_entry
                .set_text(&vnc.custom_args.join(" "));
        }

        // Set jump host dropdown
        if let Some(jump_id) = vnc.jump_host_id {
            let conns = self.vnc_connections_data.borrow();
            if let Some(idx) = conns.iter().position(|(id, _)| *id == Some(jump_id)) {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value range fits the target type by construction in this code path"
                )]
                self.vnc_jump_host_dropdown.set_selected(idx as u32);
            }
        }

        self.vnc_accept_certificate_check
            .set_active(vnc.accept_certificate);
    }

    pub(super) fn set_spice_config(&self, spice: &SpiceConfig) {
        self.spice_tls_check.set_active(spice.tls_enabled);
        if let Some(ref path) = spice.ca_cert_path {
            self.spice_ca_cert_entry.set_text(&path.to_string_lossy());
        }
        self.spice_skip_verify_check
            .set_active(spice.skip_cert_verify);
        self.spice_usb_check.set_active(spice.usb_redirection);
        self.spice_clipboard_check
            .set_active(spice.clipboard_enabled);
        self.spice_show_local_cursor_check
            .set_active(spice.show_local_cursor);

        // Map compression mode to dropdown index
        let compression_idx = match spice.image_compression {
            Some(SpiceImageCompression::Off) => 1,
            Some(SpiceImageCompression::Glz) => 2,
            Some(SpiceImageCompression::Lz) => 3,
            Some(SpiceImageCompression::Quic) => 4,
            _ => 0, // Auto or None
        };
        self.spice_compression_dropdown
            .set_selected(compression_idx);

        // Set proxy
        if let Some(ref proxy) = spice.proxy {
            self.spice_proxy_entry.set_text(proxy);
        }

        // Populate shared folders
        self.spice_shared_folders.borrow_mut().clear();
        while let Some(row) = self.spice_shared_folders_list.row_at_index(0) {
            self.spice_shared_folders_list.remove(&row);
        }
        for folder in &spice.shared_folders {
            self.spice_shared_folders.borrow_mut().push(folder.clone());
            crate::dialogs::connection::shared_folders::add_folder_row_to_list(
                &self.spice_shared_folders_list,
                &folder.local_path,
                &folder.share_name,
            );
        }

        // Set jump host dropdown
        if let Some(jump_id) = spice.jump_host_id {
            let conns = self.spice_connections_data.borrow();
            if let Some(idx) = conns.iter().position(|(id, _)| *id == Some(jump_id)) {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value range fits the target type by construction in this code path"
                )]
                self.spice_jump_host_dropdown.set_selected(idx as u32);
            }
        }

        // Set unix socket path
        if let Some(ref socket_path) = spice.unix_socket_path {
            self.spice_unix_socket_check.set_active(true);
            self.spice_socket_path_entry
                .set_text(&socket_path.to_string_lossy());
        } else {
            self.spice_unix_socket_check.set_active(false);
            self.spice_socket_path_entry.set_text("");
        }
    }

    pub(super) fn set_zerotrust_config(&self, zt: &ZeroTrustConfig) {
        // Set provider dropdown
        let provider_idx = match zt.provider {
            ZeroTrustProvider::AwsSsm => 0,
            ZeroTrustProvider::GcpIap => 1,
            ZeroTrustProvider::AzureBastion => 2,
            ZeroTrustProvider::AzureSsh => 3,
            ZeroTrustProvider::OciBastion => 4,
            ZeroTrustProvider::CloudflareAccess => 5,
            ZeroTrustProvider::Teleport => 6,
            ZeroTrustProvider::TailscaleSsh => 7,
            ZeroTrustProvider::Boundary => 8,
            ZeroTrustProvider::HoopDev => 9,
            ZeroTrustProvider::Generic => 10,
        };
        self.zt_provider_dropdown.set_selected(provider_idx);

        // Set provider stack view
        let stack_name = match zt.provider {
            ZeroTrustProvider::AwsSsm => "aws_ssm",
            ZeroTrustProvider::GcpIap => "gcp_iap",
            ZeroTrustProvider::AzureBastion => "azure_bastion",
            ZeroTrustProvider::AzureSsh => "azure_ssh",
            ZeroTrustProvider::OciBastion => "oci_bastion",
            ZeroTrustProvider::CloudflareAccess => "cloudflare",
            ZeroTrustProvider::Teleport => "teleport",
            ZeroTrustProvider::TailscaleSsh => "tailscale",
            ZeroTrustProvider::Boundary => "boundary",
            ZeroTrustProvider::HoopDev => "hoop_dev",
            ZeroTrustProvider::Generic => "generic",
        };
        self.zt_provider_stack.set_visible_child_name(stack_name);

        // Set provider-specific fields
        match &zt.provider_config {
            ZeroTrustProviderConfig::AwsSsm(cfg) => {
                self.zt_aws_target_entry.set_text(&cfg.target);
                self.zt_aws_profile_entry.set_text(&cfg.profile);
                if let Some(ref region) = cfg.region {
                    self.zt_aws_region_entry.set_text(region);
                }
            }
            ZeroTrustProviderConfig::GcpIap(cfg) => {
                self.zt_gcp_instance_entry.set_text(&cfg.instance);
                self.zt_gcp_zone_entry.set_text(&cfg.zone);
                if let Some(ref project) = cfg.project {
                    self.zt_gcp_project_entry.set_text(project);
                }
            }
            ZeroTrustProviderConfig::AzureBastion(cfg) => {
                self.zt_azure_bastion_resource_id_entry
                    .set_text(&cfg.target_resource_id);
                self.zt_azure_bastion_rg_entry.set_text(&cfg.resource_group);
                self.zt_azure_bastion_name_entry.set_text(&cfg.bastion_name);
            }
            ZeroTrustProviderConfig::AzureSsh(cfg) => {
                self.zt_azure_ssh_vm_entry.set_text(&cfg.vm_name);
                self.zt_azure_ssh_rg_entry.set_text(&cfg.resource_group);
            }
            ZeroTrustProviderConfig::OciBastion(cfg) => {
                self.zt_oci_bastion_id_entry.set_text(&cfg.bastion_id);
                self.zt_oci_target_id_entry
                    .set_text(&cfg.target_resource_id);
                self.zt_oci_target_ip_entry.set_text(&cfg.target_private_ip);
                self.zt_oci_ssh_key_entry
                    .set_text(&cfg.ssh_public_key_file.to_string_lossy());
                self.zt_oci_session_ttl_spin
                    .set_value(f64::from(cfg.session_ttl));
            }
            ZeroTrustProviderConfig::CloudflareAccess(cfg) => {
                self.zt_cf_hostname_entry.set_text(&cfg.hostname);
            }
            ZeroTrustProviderConfig::Teleport(cfg) => {
                self.zt_teleport_host_entry.set_text(&cfg.host);
                if let Some(ref cluster) = cfg.cluster {
                    self.zt_teleport_cluster_entry.set_text(cluster);
                }
            }
            ZeroTrustProviderConfig::TailscaleSsh(cfg) => {
                self.zt_tailscale_host_entry.set_text(&cfg.host);
            }
            ZeroTrustProviderConfig::Boundary(cfg) => {
                self.zt_boundary_target_entry.set_text(&cfg.target);
                if let Some(ref addr) = cfg.addr {
                    self.zt_boundary_addr_entry.set_text(addr);
                }
            }
            ZeroTrustProviderConfig::HoopDev(cfg) => {
                self.zt_hoop_connection_name_entry
                    .set_text(&cfg.connection_name);
                if let Some(ref url) = cfg.gateway_url {
                    self.zt_hoop_gateway_url_entry.set_text(url);
                }
                if let Some(ref url) = cfg.grpc_url {
                    self.zt_hoop_grpc_url_entry.set_text(url);
                }
            }
            ZeroTrustProviderConfig::Generic(cfg) => {
                self.zt_generic_command_entry
                    .set_text(&cfg.command_template);
            }
        }

        // Set custom args
        if !zt.custom_args.is_empty() {
            self.zt_custom_args_entry
                .set_text(&zt.custom_args.join(" "));
        }
    }

    pub(super) fn set_kubernetes_config(&self, k8s: &rustconn_core::models::KubernetesConfig) {
        if let Some(ref path) = k8s.kubeconfig {
            self.k8s_kubeconfig_entry.set_text(&path.to_string_lossy());
        }
        if let Some(ref ctx) = k8s.context {
            self.k8s_context_entry.set_text(ctx);
        }
        if let Some(ref ns) = k8s.namespace {
            self.k8s_namespace_entry.set_text(ns);
        }
        if let Some(ref pod) = k8s.pod {
            self.k8s_pod_entry.set_text(pod);
        }
        if let Some(ref container) = k8s.container {
            self.k8s_container_entry.set_text(container);
        }
        let shell_idx = match k8s.shell.as_str() {
            "/bin/sh" => 0,
            "/bin/bash" => 1,
            "/bin/ash" => 2,
            "/bin/zsh" => 3,
            _ => 0,
        };
        self.k8s_shell_dropdown.set_selected(shell_idx);
        self.k8s_busybox_check.set_active(k8s.use_busybox);
        self.k8s_busybox_image_entry.set_text(&k8s.busybox_image);
        if !k8s.custom_args.is_empty() {
            self.k8s_custom_args_entry
                .set_text(&k8s.custom_args.join(" "));
        }
    }

    pub(super) fn set_mosh_config(&self, mosh: &rustconn_core::models::MoshConfig) {
        // MOSH uses the main port spin for SSH port (general tab)
        if let Some(ref port_range) = mosh.port_range {
            self.mosh_port_range_entry.set_text(port_range);
        }
        let predict_idx = match mosh.predict_mode {
            rustconn_core::models::MoshPredictMode::Adaptive => 0,
            rustconn_core::models::MoshPredictMode::Always => 1,
            rustconn_core::models::MoshPredictMode::Never => 2,
        };
        self.mosh_predict_dropdown.set_selected(predict_idx);
        if let Some(ref server_binary) = mosh.server_binary {
            self.mosh_server_binary_entry.set_text(server_binary);
        }
    }

    pub(super) fn set_web_config(&self, web: &rustconn_core::models::WebConfig) {
        if let Some(ref browser) = web.browser {
            self.web_browser_entry.set_text(browser);
        }
        self.web_private_mode_switch.set_active(web.private_mode);
    }
}
