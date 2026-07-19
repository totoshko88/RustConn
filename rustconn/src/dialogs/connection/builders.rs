//! Connection dialog data collection and building.
//!
//! This module contains `ConnectionDialogData` — a struct that collects references

// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]
//! to all dialog widgets and provides `validate()` and `build_connection()` methods
//! to produce a `ConnectionDialogResult` from the current widget state.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{CheckButton, ColorDialogButton, DropDown, Entry, SpinButton, TextView};
use libadwaita as adw;
use rustconn_core::activity_monitor::{ActivityMonitorConfig, MonitorMode};
use rustconn_core::automation::{ConnectionTask, ExpectRule, TaskCondition};
use rustconn_core::models::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    Connection, ConnectionThemeOverride, CustomProperty, GcpIapConfig, GenericZeroTrustConfig,
    HighlightRule, HoopDevConfig, OciBastionConfig, PasswordSource, ProtocolConfig, RdpClientMode,
    RdpConfig, RdpPerformanceMode, Resolution, ScaleOverride, SharedFolder, SpiceConfig,
    SpiceImageCompression, SshAuthMethod, SshConfig, SshKeySource, TailscaleSshConfig,
    TeleportConfig, VncClientMode, VncConfig, VncPerformanceMode, ZeroTrustConfig,
    ZeroTrustProvider, ZeroTrustProviderConfig,
};
use rustconn_core::session::LogConfig;
use rustconn_core::variables::Variable;
use rustconn_core::wol::{DEFAULT_BROADCAST_ADDRESS, MacAddress, WolConfig};
use uuid::Uuid;

use super::logging_tab;
use crate::i18n::i18n;

pub(super) struct ConnectionDialogData<'a> {
    pub name_entry: &'a Entry,
    pub icon_entry: &'a Entry,
    pub description_view: &'a TextView,
    pub host_entry: &'a Entry,
    pub port_spin: &'a SpinButton,
    pub username_entry: &'a Entry,
    pub domain_entry: &'a Entry,
    pub tags_entry: &'a Entry,
    pub protocol_dropdown: &'a DropDown,
    pub password_source_dropdown: &'a DropDown,
    pub password_entry: &'a Entry,
    pub variable_dropdown: &'a DropDown,
    pub group_dropdown: &'a DropDown,
    pub groups_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    pub ssh_auth_dropdown: &'a DropDown,
    pub ssh_key_source_dropdown: &'a DropDown,
    pub ssh_key_entry: &'a Entry,
    pub ssh_agent_key_dropdown: &'a DropDown,
    pub ssh_agent_keys: &'a Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>>,
    pub ssh_proxy_entry: &'a Entry,
    pub ssh_proxy_command_entry: &'a Entry,
    pub ssh_identities_only: &'a CheckButton,
    pub ssh_control_master: &'a CheckButton,
    pub ssh_agent_forwarding: &'a CheckButton,
    pub ssh_waypipe: &'a CheckButton,
    pub ssh_x11_forwarding: &'a CheckButton,
    pub ssh_compression: &'a CheckButton,
    pub ssh_verbose: &'a CheckButton,
    pub ssh_startup_entry: &'a Entry,
    pub ssh_options_entry: &'a Entry,
    pub ssh_agent_socket_entry: &'a adw::EntryRow,
    pub ssh_pkcs11_entry: &'a adw::EntryRow,
    pub ssh_remote_path_entry: &'a adw::EntryRow,
    pub ssh_keep_alive_interval: &'a adw::SpinRow,
    pub ssh_keep_alive_count_max: &'a adw::SpinRow,
    pub ssh_port_forwards: &'a Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
    pub rdp_client_mode_dropdown: &'a DropDown,
    pub rdp_performance_mode_dropdown: &'a DropDown,
    pub rdp_width_spin: &'a SpinButton,
    pub rdp_height_spin: &'a SpinButton,
    pub rdp_color_dropdown: &'a DropDown,
    pub rdp_scale_override_dropdown: &'a DropDown,
    pub rdp_audio_check: &'a adw::SwitchRow,
    pub rdp_printer_check: &'a adw::SwitchRow,
    pub rdp_gateway_entry: &'a Entry,
    pub rdp_gateway_port_spin: &'a SpinButton,
    pub rdp_gateway_username_entry: &'a Entry,
    pub rdp_disable_nla_check: &'a adw::SwitchRow,
    pub rdp_security_layer_dropdown: &'a DropDown,
    pub rdp_tls_security_level_spin: &'a SpinButton,
    pub rdp_ignore_certificate_check: &'a adw::SwitchRow,
    pub rdp_clipboard_check: &'a adw::SwitchRow,
    pub rdp_show_local_cursor_check: &'a adw::SwitchRow,
    pub rdp_jiggler_check: &'a adw::SwitchRow,
    pub rdp_jiggler_interval_spin: &'a SpinButton,
    pub rdp_autotype_delay_spin: &'a SpinButton,
    pub rdp_autotype_initial_delay_spin: &'a SpinButton,
    pub rdp_reconnect_on_resize_check: &'a adw::SwitchRow,
    pub rdp_jump_host_dropdown: &'a DropDown,
    pub rdp_connections_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    pub rdp_shared_folders: &'a Rc<RefCell<Vec<SharedFolder>>>,
    pub rdp_custom_args_entry: &'a Entry,
    pub rdp_keyboard_layout_dropdown: &'a DropDown,
    pub rdp_remote_app_program_entry: &'a Entry,
    pub rdp_remote_app_args_entry: &'a Entry,
    pub rdp_remote_app_name_entry: &'a Entry,
    pub rdp_graphics_mode_dropdown: &'a DropDown,
    pub vnc_client_mode_dropdown: &'a DropDown,
    pub vnc_performance_mode_dropdown: &'a DropDown,
    pub vnc_encoding_dropdown: &'a DropDown,
    pub vnc_compression_spin: &'a SpinButton,
    pub vnc_quality_spin: &'a SpinButton,
    pub vnc_view_only_check: &'a adw::SwitchRow,
    pub vnc_scaling_check: &'a adw::SwitchRow,
    pub vnc_clipboard_check: &'a adw::SwitchRow,
    pub vnc_show_local_cursor_check: &'a adw::SwitchRow,
    pub vnc_scale_override_dropdown: &'a DropDown,
    pub vnc_custom_args_entry: &'a Entry,
    pub vnc_jump_host_dropdown: &'a DropDown,
    pub vnc_accept_certificate_check: &'a adw::SwitchRow,
    pub vnc_connections_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    pub spice_tls_check: &'a adw::SwitchRow,
    pub spice_ca_cert_entry: &'a Entry,
    pub spice_skip_verify_check: &'a adw::SwitchRow,
    pub spice_usb_check: &'a adw::SwitchRow,
    pub spice_clipboard_check: &'a adw::SwitchRow,
    pub spice_show_local_cursor_check: &'a adw::SwitchRow,
    pub spice_compression_dropdown: &'a DropDown,
    pub spice_proxy_entry: &'a Entry,
    pub spice_shared_folders: &'a Rc<RefCell<Vec<SharedFolder>>>,
    pub spice_jump_host_dropdown: &'a DropDown,
    pub spice_connections_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    pub spice_unix_socket_check: &'a adw::SwitchRow,
    pub spice_socket_path_entry: &'a Entry,
    // Zero Trust fields
    pub zt_provider_dropdown: &'a DropDown,
    pub zt_aws_target_entry: &'a adw::EntryRow,
    pub zt_aws_profile_entry: &'a adw::EntryRow,
    pub zt_aws_region_entry: &'a adw::EntryRow,
    pub zt_gcp_instance_entry: &'a adw::EntryRow,
    pub zt_gcp_zone_entry: &'a adw::EntryRow,
    pub zt_gcp_project_entry: &'a adw::EntryRow,
    pub zt_azure_bastion_resource_id_entry: &'a adw::EntryRow,
    pub zt_azure_bastion_rg_entry: &'a adw::EntryRow,
    pub zt_azure_bastion_name_entry: &'a adw::EntryRow,
    pub zt_azure_ssh_vm_entry: &'a adw::EntryRow,
    pub zt_azure_ssh_rg_entry: &'a adw::EntryRow,
    pub zt_oci_bastion_id_entry: &'a adw::EntryRow,
    pub zt_oci_target_id_entry: &'a adw::EntryRow,
    pub zt_oci_target_ip_entry: &'a adw::EntryRow,
    pub zt_oci_ssh_key_entry: &'a adw::EntryRow,
    pub zt_oci_session_ttl_spin: &'a adw::SpinRow,
    pub zt_cf_hostname_entry: &'a adw::EntryRow,
    pub zt_teleport_host_entry: &'a adw::EntryRow,
    pub zt_teleport_cluster_entry: &'a adw::EntryRow,
    pub zt_tailscale_host_entry: &'a adw::EntryRow,
    pub zt_boundary_target_entry: &'a adw::EntryRow,
    pub zt_boundary_addr_entry: &'a adw::EntryRow,
    pub zt_hoop_connection_name_entry: &'a adw::EntryRow,
    pub zt_hoop_gateway_url_entry: &'a adw::EntryRow,
    pub zt_hoop_grpc_url_entry: &'a adw::EntryRow,
    pub zt_generic_command_entry: &'a adw::EntryRow,
    pub zt_custom_args_entry: &'a Entry,
    // Telnet fields
    pub telnet_custom_args_entry: &'a Entry,
    pub telnet_backspace_dropdown: &'a DropDown,
    pub telnet_delete_dropdown: &'a DropDown,
    // Serial fields
    pub serial_device_entry: &'a Entry,
    pub serial_baud_dropdown: &'a DropDown,
    pub serial_data_bits_dropdown: &'a DropDown,
    pub serial_stop_bits_dropdown: &'a DropDown,
    pub serial_parity_dropdown: &'a DropDown,
    pub serial_flow_control_dropdown: &'a DropDown,
    pub serial_custom_args_entry: &'a Entry,
    // Kubernetes fields
    pub k8s_kubeconfig_entry: &'a Entry,
    pub k8s_context_entry: &'a Entry,
    pub k8s_namespace_entry: &'a Entry,
    pub k8s_pod_entry: &'a Entry,
    pub k8s_container_entry: &'a Entry,
    pub k8s_shell_dropdown: &'a DropDown,
    pub k8s_busybox_check: &'a CheckButton,
    pub k8s_busybox_image_entry: &'a Entry,
    pub k8s_custom_args_entry: &'a Entry,
    // MOSH fields
    pub mosh_port_range_entry: &'a Entry,
    pub mosh_predict_dropdown: &'a DropDown,
    pub mosh_server_binary_entry: &'a Entry,
    // Web fields
    pub web_browser_entry: &'a Entry,
    pub web_private_mode_switch: &'a adw::SwitchRow,
    pub web_browser_mode_combo: &'a adw::ComboRow,
    pub web_javascript_switch: &'a adw::SwitchRow,
    pub web_user_agent_row: &'a adw::EntryRow,
    pub local_variables: &'a HashMap<String, Variable>,
    pub logging_tab: &'a logging_tab::LoggingTab,
    pub expect_rules: &'a Vec<ExpectRule>,
    // Task fields
    pub pre_connect_enabled_switch: &'a adw::SwitchRow,
    pub pre_connect_command_entry: &'a Entry,
    pub pre_connect_timeout_spin: &'a SpinButton,
    pub pre_connect_abort_switch: &'a adw::SwitchRow,
    pub pre_connect_first_only_switch: &'a adw::SwitchRow,
    pub post_disconnect_enabled_switch: &'a adw::SwitchRow,
    pub post_disconnect_command_entry: &'a Entry,
    pub post_disconnect_timeout_spin: &'a SpinButton,
    pub post_disconnect_last_only_switch: &'a adw::SwitchRow,
    // Custom properties
    pub custom_properties: &'a Vec<CustomProperty>,
    // WOL fields
    pub wol_enabled_check: &'a CheckButton,
    pub wol_mac_entry: &'a Entry,
    pub wol_broadcast_entry: &'a Entry,
    pub wol_port_spin: &'a SpinButton,
    pub wol_wait_spin: &'a SpinButton,
    // Terminal theme fields
    pub theme_bg_button: &'a ColorDialogButton,
    pub theme_fg_button: &'a ColorDialogButton,
    pub theme_cursor_button: &'a ColorDialogButton,
    pub editing_id: &'a Rc<RefCell<Option<Uuid>>>,
    // Jump Host fields
    pub ssh_jump_host_dropdown: &'a DropDown,
    pub connections_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    // Script credential fields
    pub script_command_entry: &'a Entry,
    // Remote monitoring override field
    pub monitoring_toggle: &'a adw::SwitchRow,
    // Session recording field
    pub recording_toggle: &'a adw::SwitchRow,
    // Highlight rules
    pub highlight_rules: &'a Vec<HighlightRule>,
    // Activity monitor fields
    pub activity_mode_combo: &'a adw::ComboRow,
    pub activity_quiet_period_spin: &'a adw::SpinRow,
    pub activity_silence_timeout_spin: &'a adw::SpinRow,
    // Retry config fields
    pub retry_enabled_toggle: &'a adw::SwitchRow,
    pub retry_max_attempts_spin: &'a adw::SpinRow,
    pub retry_initial_delay_spin: &'a adw::SpinRow,
    pub retry_max_delay_spin: &'a adw::SpinRow,
    // Skip pre-connect TCP port check for this connection
    pub skip_port_check_toggle: &'a adw::SwitchRow,
    pub knock_sequence_entry: &'a gtk4::Entry,
    // SPA (fwknop) fields
    pub spa_enabled_toggle: &'a adw::SwitchRow,
    pub spa_rij_key_entry: &'a adw::PasswordEntryRow,
    pub spa_hmac_key_entry: &'a adw::PasswordEntryRow,
    pub spa_access_entry: &'a adw::EntryRow,
    pub spa_port_spin: &'a adw::SpinRow,
    pub spa_allow_ip_combo: &'a adw::ComboRow,
}
impl ConnectionDialogData<'_> {
    pub(super) fn validate(&self) -> Result<(), String> {
        let name = self.name_entry.text();
        if name.trim().is_empty() {
            return Err(i18n("Connection name is required"));
        }

        // Protocol-specific validation using dropdown indices
        // 0=SSH, 1=RDP, 2=VNC, 3=SPICE, 4=Zero Trust, 5=Telnet, 6=Serial
        let protocol_idx = self.protocol_dropdown.selected();
        let is_zerotrust = protocol_idx == 4;
        let is_serial = protocol_idx == 6;
        let is_kubernetes = protocol_idx == 8;

        // Host and port are optional for Zero Trust, Serial, and Kubernetes
        if !is_zerotrust && !is_serial && !is_kubernetes {
            let host = self.host_entry.text();
            if host.trim().is_empty() {
                return Err(i18n("Host is required"));
            }

            let host_str = host.trim();
            if host_str.contains(' ') {
                return Err(i18n("Host cannot contain spaces"));
            }

            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "value range fits the target type and is non-negative by construction in this code path"
            )]
            let port = self.port_spin.value() as u16;
            if port == 0 {
                return Err(i18n("Port must be greater than 0"));
            }
        }

        // Serial requires a device path
        if is_serial {
            let device = self.serial_device_entry.text();
            if device.trim().is_empty() {
                return Err(i18n("Device path is required for serial connections"));
            }
        }

        // Zero Trust requires the selected provider's identifying field(s)
        if is_zerotrust {
            self.validate_zerotrust()?;
        }
        if protocol_idx == 0 {
            // SSH
            let auth_idx = self.ssh_auth_dropdown.selected();
            if auth_idx == 1 {
                // Public Key — key path is only required when Key Source is "File" (1).
                // "Default" (0) uses ~/.ssh/id_rsa, id_ed25519, id_ecdsa automatically.
                let key_source_idx = self.ssh_key_source_dropdown.selected();
                if key_source_idx == 1 {
                    let key_path = self.ssh_key_entry.text();
                    if key_path.trim().is_empty() {
                        return Err(i18n(
                            "SSH key path is required for public key authentication",
                        ));
                    }
                }
            }
            // SSH-1: Warn when auth=Password but password_source=None
            if auth_idx == 0 {
                // Password auth
                let pw_source_idx = self.password_source_dropdown.selected();
                if pw_source_idx == 4 {
                    // None
                    return Err(i18n(
                        "Password source is 'None' but auth method is Password. Set source to Prompt or Vault.",
                    ));
                }
            }
        }

        // K8S-1: Kubernetes pod validation
        if is_kubernetes && !self.k8s_busybox_check.is_active() {
            let pod = self.k8s_pod_entry.text();
            if pod.trim().is_empty() {
                return Err(i18n("Pod name is required when Busybox mode is disabled"));
            }
        }
        // RDP (1) and VNC (2) use native embedding, no client validation needed

        // WOL validation
        if self.wol_enabled_check.is_active() {
            let mac_text = self.wol_mac_entry.text();
            if mac_text.trim().is_empty() {
                return Err(i18n("MAC address is required when WOL is enabled"));
            }
            // Validate MAC address format
            if MacAddress::parse(mac_text.trim()).is_err() {
                return Err(i18n(
                    "Invalid MAC address format. Use AA:BB:CC:DD:EE:FF or AA-BB-CC-DD-EE-FF",
                ));
            }
        }

        // Icon validation
        let icon_text = self.icon_entry.text();
        rustconn_core::dialog_utils::validate_icon(icon_text.trim())
            .map_err(|e| i18n(&e.to_string()))?;

        // Web (10): Custom browser mode requires a non-empty browser command
        if protocol_idx == 10 {
            use crate::dialogs::connection::web::browser_mode_from_combo_index;
            let mode = browser_mode_from_combo_index(self.web_browser_mode_combo.selected());
            if mode == rustconn_core::models::WebBrowserMode::Custom {
                let browser_text = self.web_browser_entry.text();
                if browser_text.trim().is_empty() {
                    return Err(i18n(
                        "Browser command is required when Custom mode is selected",
                    ));
                }
            }
        }

        Ok(())
    }

    /// Validates that the selected Zero Trust provider has its required
    /// identifying field(s) filled in.
    ///
    /// Only the fields that form the provider's target identity are checked;
    /// genuinely optional fields (AWS profile/region, Teleport cluster, Hoop.dev
    /// URLs, …) are left to their defaults. Indices match `zt_provider_dropdown`
    /// and [`Self::build_zerotrust_config`].
    fn validate_zerotrust(&self) -> Result<(), String> {
        let filled = |row: &adw::EntryRow| !row.text().trim().is_empty();
        match self.zt_provider_dropdown.selected() {
            0 => {
                if !filled(self.zt_aws_target_entry) {
                    return Err(i18n("AWS SSM target instance ID is required"));
                }
            }
            1 => {
                if !filled(self.zt_gcp_instance_entry) || !filled(self.zt_gcp_zone_entry) {
                    return Err(i18n("GCP instance name and zone are required"));
                }
            }
            2 => {
                if !filled(self.zt_azure_bastion_resource_id_entry)
                    || !filled(self.zt_azure_bastion_rg_entry)
                    || !filled(self.zt_azure_bastion_name_entry)
                {
                    return Err(i18n(
                        "Azure Bastion resource ID, resource group, and bastion name are required",
                    ));
                }
            }
            3 => {
                if !filled(self.zt_azure_ssh_vm_entry) || !filled(self.zt_azure_ssh_rg_entry) {
                    return Err(i18n("Azure VM name and resource group are required"));
                }
            }
            4 => {
                if !filled(self.zt_oci_bastion_id_entry)
                    || !filled(self.zt_oci_target_id_entry)
                    || !filled(self.zt_oci_target_ip_entry)
                {
                    return Err(i18n(
                        "OCI bastion OCID, target OCID, and target private IP are required",
                    ));
                }
            }
            5 => {
                if !filled(self.zt_cf_hostname_entry) {
                    return Err(i18n("Cloudflare Access hostname is required"));
                }
            }
            6 => {
                if !filled(self.zt_teleport_host_entry) {
                    return Err(i18n("Teleport host is required"));
                }
            }
            7 => {
                if !filled(self.zt_tailscale_host_entry) {
                    return Err(i18n("Tailscale host is required"));
                }
            }
            8 => {
                if !filled(self.zt_boundary_target_entry) {
                    return Err(i18n("Boundary target is required"));
                }
            }
            9 => {
                if !filled(self.zt_hoop_connection_name_entry) {
                    return Err(i18n("Hoop.dev connection name is required"));
                }
            }
            _ => {
                if !filled(self.zt_generic_command_entry) {
                    return Err(i18n(
                        "Command template is required for the Generic provider",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn build_connection(&self) -> Option<super::ConnectionDialogResult> {
        let name = self.name_entry.text().trim().to_string();
        let buffer = self.description_view.buffer();
        let description_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let description = if description_text.trim().is_empty() {
            None
        } else {
            Some(description_text.trim().to_string())
        };
        let host = self.host_entry.text().trim().to_string();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let port = self.port_spin.value() as u16;

        let protocol_config = self.build_protocol_config()?;

        let mut conn = Connection::new(name, host, port, protocol_config);
        conn.description = description;

        // Set custom icon if provided
        let icon_text = self.icon_entry.text().trim().to_string();
        if !icon_text.is_empty() {
            conn.icon = Some(icon_text);
        }

        let username = self.username_entry.text();
        if !username.trim().is_empty() {
            conn.username = Some(username.trim().to_string());
        }

        let domain = self.domain_entry.text();
        if !domain.trim().is_empty() {
            conn.domain = Some(domain.trim().to_string());
        }

        let tags_text = self.tags_entry.text();
        if !tags_text.trim().is_empty() {
            conn.tags = tags_text
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                // Filter out desc: tags since we now have a dedicated description field
                .filter(|s| !s.starts_with("desc:"))
                .collect();
        }

        // Password source - map dropdown index to enum
        // Dropdown order: Prompt(0), Vault(1), Variable(2), Inherit(3), None(4), Script(5)
        conn.password_source = match self.password_source_dropdown.selected() {
            1 => PasswordSource::Vault,
            2 => {
                // Variable — get selected variable name from dropdown
                let selected = self.variable_dropdown.selected();
                let model = self.variable_dropdown.model();
                let var_name = model
                    .and_then(|m| {
                        m.downcast_ref::<gtk4::StringList>()
                            .and_then(|sl| sl.string(selected))
                    })
                    .map_or_else(String::new, |s| s.to_string());
                PasswordSource::Variable(var_name)
            }
            3 => PasswordSource::Inherit,
            4 => PasswordSource::None,
            5 => {
                let cmd = self.script_command_entry.text().trim().to_string();
                PasswordSource::Script(cmd)
            }
            _ => PasswordSource::Prompt, // 0 and any other value default to Prompt
        };

        // Set local variables
        conn.local_variables = self.local_variables.clone();

        // Set log config if enabled
        conn.log_config = self.build_log_config();

        // Set expect rules (filter out empty patterns)
        conn.automation.expect_rules = self
            .expect_rules
            .iter()
            .filter(|r| !r.pattern.is_empty())
            .cloned()
            .collect();

        // Set pre-connect task if enabled
        conn.pre_connect_task = self.build_pre_connect_task();

        // Set post-disconnect task if enabled
        conn.post_disconnect_task = self.build_post_disconnect_task();

        // Set custom properties (filter out empty names)
        conn.custom_properties = self
            .custom_properties
            .iter()
            .filter(|p| !p.name.trim().is_empty())
            .cloned()
            .collect();

        // Set WOL config if enabled
        conn.wol_config = self.build_wol_config();

        // Set terminal theme override
        {
            let bg_hex = super::advanced_tab::rgba_to_hex(&self.theme_bg_button.rgba());
            let fg_hex = super::advanced_tab::rgba_to_hex(&self.theme_fg_button.rgba());
            let cur_hex = super::advanced_tab::rgba_to_hex(&self.theme_cursor_button.rgba());

            // Only set if at least one color differs from default (black bg, white fg/cursor)
            let is_default_bg = bg_hex == "#000000";
            let is_default_fg = fg_hex == "#ffffff";
            let is_default_cur = cur_hex == "#ffffff";

            if !(is_default_bg && is_default_fg && is_default_cur) {
                let theme = ConnectionThemeOverride {
                    background: if is_default_bg { None } else { Some(bg_hex) },
                    foreground: if is_default_fg { None } else { Some(fg_hex) },
                    cursor: if is_default_cur { None } else { Some(cur_hex) },
                };
                if !theme.is_empty() {
                    conn.theme_override = Some(theme);
                }
            }
        }

        // Set remote monitoring override
        // When toggle is ON, store explicit enabled override so it works
        // even when global monitoring is disabled.
        // When toggle is OFF, store explicit disabled override.
        conn.monitoring_config = if self.monitoring_toggle.is_active() {
            Some(rustconn_core::monitoring::MonitoringConfig {
                enabled: Some(true),
                interval_secs: None,
            })
        } else {
            Some(rustconn_core::monitoring::MonitoringConfig {
                enabled: Some(false),
                interval_secs: None,
            })
        };

        // Set session recording
        conn.session_recording_enabled = self.recording_toggle.is_active();

        // Set skip-port-check override
        conn.skip_port_check = self.skip_port_check_toggle.is_active();

        // Set port knock sequence (parse from entry text)
        let knock_text = self.knock_sequence_entry.text().to_string();
        let knock_text = knock_text.trim();
        conn.knock_sequence = if knock_text.is_empty() {
            None
        } else {
            rustconn_core::connection::knock::KnockSequence::parse(knock_text).ok()
        };

        // Set SPA (fwknop) config if enabled
        conn.spa_config = if self.spa_enabled_toggle.is_active() {
            let rij_key = self.spa_rij_key_entry.text().to_string();
            let hmac_key = self.spa_hmac_key_entry.text().to_string();
            let access = self.spa_access_entry.text().trim().to_string();
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "port range 1..65535 fits u16"
            )]
            let dest_port = self.spa_port_spin.value() as u16;
            let allow_ip = match self.spa_allow_ip_combo.selected() {
                1 => rustconn_core::connection::knock::SpaAllowIp::ResolvePublic,
                2 => rustconn_core::connection::knock::SpaAllowIp::Explicit(String::new()),
                _ => rustconn_core::connection::knock::SpaAllowIp::SourceIp,
            };
            Some(rustconn_core::connection::knock::SpaConfig {
                rijndael_key_ref: if rij_key.is_empty() {
                    None
                } else {
                    Some(rij_key)
                },
                hmac_key_ref: if hmac_key.is_empty() {
                    None
                } else {
                    Some(hmac_key)
                },
                access: if access.is_empty() {
                    "tcp/22".to_string()
                } else {
                    access
                },
                dest_port,
                allow_ip,
            })
        } else {
            None
        };

        // Set highlight rules (filter out empty patterns)
        conn.highlight_rules = self
            .highlight_rules
            .iter()
            .filter(|r| !r.pattern.is_empty())
            .cloned()
            .collect();

        // Set activity monitor config
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        {
            let mode = match self.activity_mode_combo.selected() {
                1 => Some(MonitorMode::Activity),
                2 => Some(MonitorMode::Silence),
                _ => None, // Off or default
            };
            let quiet = self.activity_quiet_period_spin.value() as u32;
            let silence = self.activity_silence_timeout_spin.value() as u32;

            // Store None if all defaults (Off, 10, 30) to keep config clean
            let is_default = mode.is_none() && quiet == 10 && silence == 30;
            conn.activity_monitor_config = if is_default {
                None
            } else {
                Some(ActivityMonitorConfig {
                    mode,
                    quiet_period_secs: if quiet == 10 { None } else { Some(quiet) },
                    silence_timeout_secs: if silence == 30 { None } else { Some(silence) },
                })
            };
        }

        // Set retry config
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        {
            let enabled = self.retry_enabled_toggle.is_active();
            let max_attempts = self.retry_max_attempts_spin.value() as u32;
            let initial_delay_ms = self.retry_initial_delay_spin.value() as u64;
            let max_delay_ms = self.retry_max_delay_spin.value() as u64;

            // Store None if all defaults to keep config clean
            let is_default =
                enabled && max_attempts == 3 && initial_delay_ms == 1000 && max_delay_ms == 30_000;
            conn.retry_config = if is_default {
                None
            } else {
                Some(rustconn_core::RetryConfig {
                    max_attempts,
                    initial_delay_ms,
                    max_delay_ms,
                    backoff_multiplier: rustconn_core::connection::DEFAULT_BACKOFF_MULTIPLIER,
                    enabled,
                })
            };
        }

        // Set group from dropdown
        let selected_idx = self.group_dropdown.selected() as usize;
        let groups_data = self.groups_data.borrow();
        if selected_idx < groups_data.len() {
            conn.group_id = groups_data[selected_idx].0;
        }

        if let Some(id) = *self.editing_id.borrow() {
            conn.id = id;
        }

        // Extract password if user entered one (for Vault source only).
        // Capture directly into Zeroizing so the intermediate String is wiped
        // on drop instead of leaking a plaintext copy on the heap.
        let password_source_idx = self.password_source_dropdown.selected();
        let password = if password_source_idx == 1 {
            let pwd = zeroize::Zeroizing::new(self.password_entry.text().to_string());
            if pwd.is_empty() {
                None
            } else {
                Some(secrecy::SecretString::from(pwd.as_str()))
            }
        } else {
            None
        };

        Some(super::ConnectionDialogResult {
            connection: conn,
            password,
        })
    }

    fn build_wol_config(&self) -> Option<WolConfig> {
        if !self.wol_enabled_check.is_active() {
            return None;
        }

        let mac_text = self.wol_mac_entry.text();
        let mac_address = MacAddress::parse(mac_text.trim()).ok()?;

        let broadcast_address = self.wol_broadcast_entry.text().trim().to_string();
        let broadcast_address = if broadcast_address.is_empty() {
            DEFAULT_BROADCAST_ADDRESS.to_string()
        } else {
            broadcast_address
        };

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let port = self.wol_port_spin.value() as u16;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let wait_seconds = self.wol_wait_spin.value() as u32;

        Some(WolConfig {
            mac_address,
            broadcast_address,
            port,
            wait_seconds,
        })
    }

    fn build_pre_connect_task(&self) -> Option<ConnectionTask> {
        if !self.pre_connect_enabled_switch.is_active() {
            return None;
        }

        let command = self.pre_connect_command_entry.text().trim().to_string();
        if command.is_empty() {
            return None;
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let timeout_ms = self.pre_connect_timeout_spin.value() as u32;
        let timeout = if timeout_ms > 0 {
            Some(timeout_ms)
        } else {
            None
        };

        let condition = TaskCondition {
            only_first_in_folder: self.pre_connect_first_only_switch.is_active(),
            only_last_in_folder: false,
        };

        let mut task = ConnectionTask::new_pre_connect(command)
            .with_condition(condition)
            .with_abort_on_failure(self.pre_connect_abort_switch.is_active());

        if let Some(t) = timeout {
            task = task.with_timeout(t);
        }

        Some(task)
    }

    fn build_post_disconnect_task(&self) -> Option<ConnectionTask> {
        if !self.post_disconnect_enabled_switch.is_active() {
            return None;
        }

        let command = self.post_disconnect_command_entry.text().trim().to_string();
        if command.is_empty() {
            return None;
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let timeout_ms = self.post_disconnect_timeout_spin.value() as u32;
        let timeout = if timeout_ms > 0 {
            Some(timeout_ms)
        } else {
            None
        };

        let condition = TaskCondition {
            only_first_in_folder: false,
            only_last_in_folder: self.post_disconnect_last_only_switch.is_active(),
        };

        let mut task = ConnectionTask::new_post_disconnect(command).with_condition(condition);

        if let Some(t) = timeout {
            task = task.with_timeout(t);
        }

        Some(task)
    }

    fn build_log_config(&self) -> Option<LogConfig> {
        self.logging_tab.build()
    }

    fn build_protocol_config(&self) -> Option<ProtocolConfig> {
        let protocol_idx = self.protocol_dropdown.selected();

        match protocol_idx {
            0 => Some(ProtocolConfig::Ssh(self.build_ssh_config())),
            1 => Some(ProtocolConfig::Rdp(self.build_rdp_config())),
            2 => Some(ProtocolConfig::Vnc(self.build_vnc_config())),
            3 => Some(ProtocolConfig::Spice(self.build_spice_config())),
            4 => Some(ProtocolConfig::ZeroTrust(self.build_zerotrust_config())),
            5 => Some(ProtocolConfig::Telnet(self.build_telnet_config())),
            6 => Some(ProtocolConfig::Serial(self.build_serial_config())),
            7 => Some(ProtocolConfig::Sftp(self.build_ssh_config())),
            8 => Some(ProtocolConfig::Kubernetes(self.build_kubernetes_config())),
            9 => Some(ProtocolConfig::Mosh(self.build_mosh_config())),
            10 => Some(ProtocolConfig::Web(self.build_web_config())),
            _ => None,
        }
    }

    fn build_zerotrust_config(&self) -> ZeroTrustConfig {
        let provider_idx = self.zt_provider_dropdown.selected();
        let provider = match provider_idx {
            0 => ZeroTrustProvider::AwsSsm,
            1 => ZeroTrustProvider::GcpIap,
            2 => ZeroTrustProvider::AzureBastion,
            3 => ZeroTrustProvider::AzureSsh,
            4 => ZeroTrustProvider::OciBastion,
            5 => ZeroTrustProvider::CloudflareAccess,
            6 => ZeroTrustProvider::Teleport,
            7 => ZeroTrustProvider::TailscaleSsh,
            8 => ZeroTrustProvider::Boundary,
            9 => ZeroTrustProvider::HoopDev,
            _ => ZeroTrustProvider::Generic,
        };

        let provider_config = match provider {
            ZeroTrustProvider::AwsSsm => ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                target: self.zt_aws_target_entry.text().trim().to_string(),
                profile: {
                    let text = self.zt_aws_profile_entry.text();
                    if text.trim().is_empty() {
                        "default".to_string()
                    } else {
                        text.trim().to_string()
                    }
                },
                region: {
                    let text = self.zt_aws_region_entry.text();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                },
            }),
            ZeroTrustProvider::GcpIap => ZeroTrustProviderConfig::GcpIap(GcpIapConfig {
                instance: self.zt_gcp_instance_entry.text().trim().to_string(),
                zone: self.zt_gcp_zone_entry.text().trim().to_string(),
                project: {
                    let text = self.zt_gcp_project_entry.text();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                },
            }),
            ZeroTrustProvider::AzureBastion => {
                ZeroTrustProviderConfig::AzureBastion(AzureBastionConfig {
                    target_resource_id: self
                        .zt_azure_bastion_resource_id_entry
                        .text()
                        .trim()
                        .to_string(),
                    resource_group: self.zt_azure_bastion_rg_entry.text().trim().to_string(),
                    bastion_name: self.zt_azure_bastion_name_entry.text().trim().to_string(),
                })
            }
            ZeroTrustProvider::AzureSsh => ZeroTrustProviderConfig::AzureSsh(AzureSshConfig {
                vm_name: self.zt_azure_ssh_vm_entry.text().trim().to_string(),
                resource_group: self.zt_azure_ssh_rg_entry.text().trim().to_string(),
            }),
            ZeroTrustProvider::OciBastion => {
                ZeroTrustProviderConfig::OciBastion(OciBastionConfig {
                    bastion_id: self.zt_oci_bastion_id_entry.text().trim().to_string(),
                    target_resource_id: self.zt_oci_target_id_entry.text().trim().to_string(),
                    target_private_ip: self.zt_oci_target_ip_entry.text().trim().to_string(),
                    ssh_public_key_file: {
                        let text = self.zt_oci_ssh_key_entry.text();
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            default_ssh_pub_key_path()
                        } else {
                            PathBuf::from(trimmed)
                        }
                    },
                    session_ttl: self.zt_oci_session_ttl_spin.value() as u32,
                })
            }
            ZeroTrustProvider::CloudflareAccess => {
                ZeroTrustProviderConfig::CloudflareAccess(CloudflareAccessConfig {
                    hostname: self.zt_cf_hostname_entry.text().trim().to_string(),
                    username: None,
                })
            }
            ZeroTrustProvider::Teleport => ZeroTrustProviderConfig::Teleport(TeleportConfig {
                host: self.zt_teleport_host_entry.text().trim().to_string(),
                username: None,
                cluster: {
                    let text = self.zt_teleport_cluster_entry.text();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                },
            }),
            ZeroTrustProvider::TailscaleSsh => {
                ZeroTrustProviderConfig::TailscaleSsh(TailscaleSshConfig {
                    host: self.zt_tailscale_host_entry.text().trim().to_string(),
                    username: None,
                })
            }
            ZeroTrustProvider::Boundary => ZeroTrustProviderConfig::Boundary(BoundaryConfig {
                target: self.zt_boundary_target_entry.text().trim().to_string(),
                addr: {
                    let text = self.zt_boundary_addr_entry.text();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                },
            }),
            ZeroTrustProvider::HoopDev => ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: self.zt_hoop_connection_name_entry.text().trim().to_string(),
                gateway_url: {
                    let text = self.zt_hoop_gateway_url_entry.text();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                },
                grpc_url: {
                    let text = self.zt_hoop_grpc_url_entry.text();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                },
            }),
            ZeroTrustProvider::Generic => {
                ZeroTrustProviderConfig::Generic(GenericZeroTrustConfig {
                    command_template: self.zt_generic_command_entry.text().trim().to_string(),
                })
            }
        };

        let custom_args = Self::parse_args(&self.zt_custom_args_entry.text());

        // Build the config first to get the command for provider detection
        let mut config = ZeroTrustConfig {
            provider,
            provider_config,
            custom_args,
            detected_provider: None,
        };

        // Detect and persist the provider based on the generated command
        let (program, args) = config.build_command(None);
        let full_command = format!("{} {}", program, args.join(" "));
        let detected = rustconn_core::detect_provider(&full_command);
        config.detected_provider = Some(detected.icon_name().to_string());

        config
    }

    fn build_telnet_config(&self) -> rustconn_core::models::TelnetConfig {
        let custom_args_text = self.telnet_custom_args_entry.text();
        let custom_args: Vec<String> = if custom_args_text.trim().is_empty() {
            Vec::new()
        } else {
            custom_args_text
                .split_whitespace()
                .map(String::from)
                .collect()
        };
        let backspace_sends = rustconn_core::models::TelnetBackspaceSends::from_index(
            self.telnet_backspace_dropdown.selected(),
        );
        let delete_sends = rustconn_core::models::TelnetDeleteSends::from_index(
            self.telnet_delete_dropdown.selected(),
        );
        rustconn_core::models::TelnetConfig {
            custom_args,
            backspace_sends,
            delete_sends,
        }
    }

    fn build_serial_config(&self) -> rustconn_core::models::SerialConfig {
        let device = self.serial_device_entry.text().trim().to_string();
        let custom_args_text = self.serial_custom_args_entry.text();
        let custom_args: Vec<String> = if custom_args_text.trim().is_empty() {
            Vec::new()
        } else {
            custom_args_text
                .split_whitespace()
                .map(String::from)
                .collect()
        };
        rustconn_core::models::SerialConfig {
            device,
            baud_rate: rustconn_core::SerialBaudRate::from_index(
                self.serial_baud_dropdown.selected(),
            ),
            data_bits: rustconn_core::SerialDataBits::from_index(
                self.serial_data_bits_dropdown.selected(),
            ),
            stop_bits: rustconn_core::SerialStopBits::from_index(
                self.serial_stop_bits_dropdown.selected(),
            ),
            parity: rustconn_core::SerialParity::from_index(self.serial_parity_dropdown.selected()),
            flow_control: rustconn_core::SerialFlowControl::from_index(
                self.serial_flow_control_dropdown.selected(),
            ),
            custom_args,
        }
    }

    fn build_kubernetes_config(&self) -> rustconn_core::models::KubernetesConfig {
        let kubeconfig = {
            let text = self.k8s_kubeconfig_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(text.trim().to_string()))
            }
        };
        let context = {
            let text = self.k8s_context_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };
        let namespace = {
            let text = self.k8s_namespace_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };
        let pod = {
            let text = self.k8s_pod_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };
        let container = {
            let text = self.k8s_container_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };
        let shells = ["/bin/sh", "/bin/bash", "/bin/ash", "/bin/zsh"];
        let shell_idx = self.k8s_shell_dropdown.selected() as usize;
        let shell = shells.get(shell_idx).unwrap_or(&"/bin/sh").to_string();
        let busybox_image = {
            let text = self.k8s_busybox_image_entry.text();
            if text.trim().is_empty() {
                "busybox:latest".to_string()
            } else {
                text.trim().to_string()
            }
        };
        let custom_args = Self::parse_args(&self.k8s_custom_args_entry.text());
        rustconn_core::models::KubernetesConfig {
            kubeconfig,
            context,
            namespace,
            pod,
            container,
            shell,
            use_busybox: self.k8s_busybox_check.is_active(),
            busybox_image,
            custom_args,
        }
    }

    fn build_mosh_config(&self) -> rustconn_core::models::MoshConfig {
        // MOSH uses the main port spin for SSH port (general tab)
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let ssh_port_val = self.port_spin.value() as u16;
        let ssh_port = if ssh_port_val == 22 {
            None
        } else {
            Some(ssh_port_val)
        };
        let port_range = {
            let text = self.mosh_port_range_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };
        let predict_mode = match self.mosh_predict_dropdown.selected() {
            1 => rustconn_core::models::MoshPredictMode::Always,
            2 => rustconn_core::models::MoshPredictMode::Never,
            _ => rustconn_core::models::MoshPredictMode::Adaptive,
        };
        let server_binary = {
            let text = self.mosh_server_binary_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };
        rustconn_core::models::MoshConfig {
            ssh_port,
            port_range,
            server_binary,
            predict_mode,
            custom_args: Vec::new(),
        }
    }

    fn build_web_config(&self) -> rustconn_core::models::WebConfig {
        use crate::dialogs::connection::web::browser_mode_from_combo_index;

        let browser = {
            let text = self.web_browser_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        let browser_mode = browser_mode_from_combo_index(self.web_browser_mode_combo.selected());

        let user_agent = {
            let text = self.web_user_agent_row.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        rustconn_core::models::WebConfig {
            browser,
            private_mode: self.web_private_mode_switch.is_active(),
            browser_mode,
            javascript_enabled: self.web_javascript_switch.is_active(),
            user_agent,
            zoom_level: 1.0,
            accept_invalid_certs: false,
        }
    }

    /// Reads the currently selected agent key from the agent-key dropdown.
    ///
    /// Returns the `(key_source, key_path, agent_key_fingerprint)` triple used
    /// by [`Self::build_ssh_config`]; falls back to `Default` when the dropdown
    /// selection is out of range (e.g. no keys loaded from the agent).
    fn ssh_agent_key_selection(&self) -> (SshKeySource, Option<PathBuf>, Option<String>) {
        let keys = self.ssh_agent_keys.borrow();
        let selected_idx = self.ssh_agent_key_dropdown.selected() as usize;
        keys.get(selected_idx)
            .map_or((SshKeySource::Default, None, None), |key| {
                (
                    SshKeySource::Agent {
                        fingerprint: key.fingerprint.clone(),
                        comment: key.comment.clone(),
                    },
                    None,
                    Some(key.fingerprint.clone()),
                )
            })
    }

    fn build_ssh_config(&self) -> SshConfig {
        let auth_method = match self.ssh_auth_dropdown.selected() {
            1 => SshAuthMethod::PublicKey,
            2 => SshAuthMethod::KeyboardInteractive,
            3 => SshAuthMethod::Agent,
            4 => SshAuthMethod::SecurityKey,
            _ => SshAuthMethod::Password, // 0 and any other value default to Password
        };

        // Derive the key material. Most auth methods follow the explicit
        // key-source selector, but SSH Agent and Security Key (FIDO2) auth
        // replace that selector in the UI with their own row (the agent-key
        // dropdown / the key-file entry). Their key material must therefore be
        // read from those rows directly — the key-source dropdown is hidden for
        // them and stays at its stale Default value, so consulting it would
        // silently drop the selected agent key or the sk key path.
        let (key_source, key_path, agent_key_fingerprint) = match auth_method {
            SshAuthMethod::Agent => self.ssh_agent_key_selection(),
            SshAuthMethod::SecurityKey => {
                let text = self.ssh_key_entry.text();
                if text.trim().is_empty() {
                    (SshKeySource::Default, None, None)
                } else {
                    let path = PathBuf::from(text.trim().to_string());
                    (SshKeySource::File { path: path.clone() }, Some(path), None)
                }
            }
            _ => match self.ssh_key_source_dropdown.selected() {
                1 => {
                    // File source
                    let text = self.ssh_key_entry.text();
                    if text.trim().is_empty() {
                        (SshKeySource::Default, None, None)
                    } else {
                        let path = PathBuf::from(text.trim().to_string());
                        (SshKeySource::File { path: path.clone() }, Some(path), None)
                    }
                }
                2 => self.ssh_agent_key_selection(),
                3 => (SshKeySource::Inherit, None, None),
                // Default (0) or any other value
                _ => (SshKeySource::Default, None, None),
            },
        };

        let startup_command = {
            let text = self.ssh_startup_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        // Jump Host
        let jump_idx = self.ssh_jump_host_dropdown.selected() as usize;
        let connections = self.connections_data.borrow();
        let jump_host_id = if jump_idx < connections.len() {
            connections[jump_idx].0
        } else {
            None
        };

        // ProxyJump text entry
        let proxy_jump = self.ssh_proxy_entry.text();
        let proxy_jump_opt = if proxy_jump.trim().is_empty() {
            None
        } else {
            Some(proxy_jump.trim().to_string())
        };

        // ProxyCommand text entry
        let proxy_command = self.ssh_proxy_command_entry.text();
        let proxy_command_opt = if proxy_command.trim().is_empty() {
            None
        } else {
            Some(proxy_command.trim().to_string())
        };

        // PKCS#11 provider library path (hardware-token auth)
        let pkcs11_provider = {
            let text = self.ssh_pkcs11_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        // SFTP remote directory override (empty → auto-detect home, issue #212)
        let remote_path = {
            let text = self.ssh_remote_path_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        let custom_options = Self::parse_custom_options(&self.ssh_options_entry.text());

        let ssh_agent_socket = {
            let text = self.ssh_agent_socket_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let keep_alive_interval = {
            let val = self.ssh_keep_alive_interval.value() as u32;
            if val == 0 { None } else { Some(val) }
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let keep_alive_count_max = {
            let val = self.ssh_keep_alive_count_max.value() as u32;
            // Only store if keep-alive interval is set and count differs from default (3)
            if keep_alive_interval.is_some() && val != 3 {
                Some(val)
            } else if keep_alive_interval.is_some() {
                // Store default explicitly when interval is set
                Some(val)
            } else {
                None
            }
        };

        SshConfig {
            auth_method,
            key_path,
            key_source,
            agent_key_fingerprint,
            identities_only: self.ssh_identities_only.is_active(),
            proxy_jump: proxy_jump_opt,
            proxy_command: proxy_command_opt,
            pkcs11_provider,
            jump_host_id, // Add this field
            use_control_master: self.ssh_control_master.is_active(),
            agent_forwarding: self.ssh_agent_forwarding.is_active(),
            waypipe: self.ssh_waypipe.is_active(),
            x11_forwarding: self.ssh_x11_forwarding.is_active(),
            compression: self.ssh_compression.is_active(),
            verbose: self.ssh_verbose.is_active(),
            custom_options,
            startup_command,
            sftp_enabled: true,
            port_forwards: self.ssh_port_forwards.borrow().clone(),
            ssh_agent_socket,
            keep_alive_interval,
            keep_alive_count_max,
            remote_path,
        }
    }

    fn build_rdp_config(&self) -> RdpConfig {
        let client_mode = RdpClientMode::from_index(self.rdp_client_mode_dropdown.selected());
        let performance_mode =
            RdpPerformanceMode::from_index(self.rdp_performance_mode_dropdown.selected());

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let resolution = Some(Resolution::new(
            self.rdp_width_spin.value() as u32,
            self.rdp_height_spin.value() as u32,
        ));

        // Map dropdown index to color depth: 0->32, 1->24, 2->16, 3->15, 4->8
        let color_depth = Some(match self.rdp_color_dropdown.selected() {
            1 => 24,
            2 => 16,
            3 => 15,
            4 => 8,
            _ => 32, // 0 and any other value default to 32
        });

        let gateway = {
            let text = self.rdp_gateway_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let port = self.rdp_gateway_port_spin.value() as u16;
                let username_text = self.rdp_gateway_username_entry.text();
                let username = if username_text.trim().is_empty() {
                    None
                } else {
                    Some(username_text.trim().to_string())
                };
                Some(rustconn_core::models::RdpGateway {
                    hostname: text.trim().to_string(),
                    port,
                    username,
                })
            }
        };

        let custom_args = Self::parse_args(&self.rdp_custom_args_entry.text());

        let shared_folders = self.rdp_shared_folders.borrow().clone();

        RdpConfig {
            client_mode,
            performance_mode,
            graphics_mode: match self.rdp_graphics_mode_dropdown.selected() {
                1 => rustconn_core::rdp_client::graphics::GraphicsMode::Legacy,
                2 => rustconn_core::rdp_client::graphics::GraphicsMode::RemoteFx,
                _ => rustconn_core::rdp_client::graphics::GraphicsMode::Auto,
            },
            resolution,
            color_depth,
            audio_redirect: self.rdp_audio_check.is_active(),
            printer_enabled: self.rdp_printer_check.is_active(),
            gateway,
            shared_folders,
            custom_args,
            keyboard_layout: super::dialog::dropdown_index_to_klid(
                self.rdp_keyboard_layout_dropdown.selected(),
            ),
            scale_override: ScaleOverride::from_index(self.rdp_scale_override_dropdown.selected()),
            disable_nla: self.rdp_disable_nla_check.is_active(),
            security_layer: rustconn_core::models::RdpSecurityLayer::from_index(
                self.rdp_security_layer_dropdown.selected(),
            ),
            tls_security_level: {
                let level = self.rdp_tls_security_level_spin.value() as u8;
                // Only store if non-default (level != 2) to keep config clean
                if level == 2 { None } else { Some(level) }
            },
            ignore_certificate: self.rdp_ignore_certificate_check.is_active(),
            clipboard_enabled: self.rdp_clipboard_check.is_active(),
            show_local_cursor: self.rdp_show_local_cursor_check.is_active(),
            jiggler_enabled: self.rdp_jiggler_check.is_active(),
            jiggler_interval_secs: self.rdp_jiggler_interval_spin.value() as u32,
            jump_host_id: {
                let idx = self.rdp_jump_host_dropdown.selected() as usize;
                let conns = self.rdp_connections_data.borrow();
                if idx < conns.len() {
                    conns[idx].0
                } else {
                    None
                }
            },
            autotype_delay_ms: self.rdp_autotype_delay_spin.value() as u32,
            autotype_initial_delay_ms: self.rdp_autotype_initial_delay_spin.value() as u32,
            reconnect_on_resize: self.rdp_reconnect_on_resize_check.is_active(),
            script_paste_via_clipboard: true,
            remote_app_program: {
                let text = self.rdp_remote_app_program_entry.text();
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text.trim().to_string())
                }
            },
            remote_app_args: {
                let text = self.rdp_remote_app_args_entry.text();
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text.trim().to_string())
                }
            },
            remote_app_name: {
                let text = self.rdp_remote_app_name_entry.text();
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text.trim().to_string())
                }
            },
        }
    }

    fn build_vnc_config(&self) -> VncConfig {
        let client_mode = VncClientMode::from_index(self.vnc_client_mode_dropdown.selected());
        let performance_mode =
            VncPerformanceMode::from_index(self.vnc_performance_mode_dropdown.selected());

        // VNC-1: Map dropdown index to encoding string
        // Items: ["Auto", "Tight", "ZRLE", "Hextile", "Raw", "CopyRect"]
        let encoding = match self.vnc_encoding_dropdown.selected() {
            1 => Some("tight".to_string()),
            2 => Some("zrle".to_string()),
            3 => Some("hextile".to_string()),
            4 => Some("raw".to_string()),
            5 => Some("copyrect".to_string()),
            _ => None, // Auto = no override
        };

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let compression = Some(self.vnc_compression_spin.value() as u8);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let quality = Some(self.vnc_quality_spin.value() as u8);

        let custom_args = Self::parse_args(&self.vnc_custom_args_entry.text());

        VncConfig {
            client_mode,
            performance_mode,
            encoding,
            compression,
            quality,
            view_only: self.vnc_view_only_check.is_active(),
            scaling: self.vnc_scaling_check.is_active(),
            clipboard_enabled: self.vnc_clipboard_check.is_active(),
            custom_args,
            scale_override: ScaleOverride::from_index(self.vnc_scale_override_dropdown.selected()),
            show_local_cursor: self.vnc_show_local_cursor_check.is_active(),
            jump_host_id: {
                let idx = self.vnc_jump_host_dropdown.selected() as usize;
                let conns = self.vnc_connections_data.borrow();
                if idx < conns.len() {
                    conns[idx].0
                } else {
                    None
                }
            },
            accept_certificate: self.vnc_accept_certificate_check.is_active(),
        }
    }

    fn build_spice_config(&self) -> SpiceConfig {
        let ca_cert_path = {
            let text = self.spice_ca_cert_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(text.trim().to_string()))
            }
        };

        // Map dropdown index to compression mode: 0->Auto, 1->Off, 2->Glz, 3->Lz, 4->Quic
        let image_compression = match self.spice_compression_dropdown.selected() {
            1 => Some(SpiceImageCompression::Off),
            2 => Some(SpiceImageCompression::Glz),
            3 => Some(SpiceImageCompression::Lz),
            4 => Some(SpiceImageCompression::Quic),
            _ => Some(SpiceImageCompression::Auto), // 0 and any other value default to Auto
        };

        let unix_socket_path = if self.spice_unix_socket_check.is_active() {
            let text = self.spice_socket_path_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(text.trim().to_string()))
            }
        } else {
            None
        };

        SpiceConfig {
            tls_enabled: self.spice_tls_check.is_active(),
            ca_cert_path,
            skip_cert_verify: self.spice_skip_verify_check.is_active(),
            usb_redirection: self.spice_usb_check.is_active(),
            shared_folders: self.spice_shared_folders.borrow().clone(),
            clipboard_enabled: self.spice_clipboard_check.is_active(),
            image_compression,
            proxy: {
                let text = self.spice_proxy_entry.text();
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text.trim().to_string())
                }
            },
            show_local_cursor: self.spice_show_local_cursor_check.is_active(),
            jump_host_id: if unix_socket_path.is_some() {
                // Socket mode is local — jump host does not apply
                None
            } else {
                let idx = self.spice_jump_host_dropdown.selected() as usize;
                let conns = self.spice_connections_data.borrow();
                if idx < conns.len() {
                    conns[idx].0
                } else {
                    None
                }
            },
            unix_socket_path,
        }
    }

    fn parse_custom_options(text: &str) -> HashMap<String, String> {
        rustconn_core::dialog_utils::parse_custom_options(text)
    }

    fn parse_args(text: &str) -> Vec<String> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        text.split_whitespace()
            .map(std::string::ToString::to_string)
            .collect()
    }
}

/// Returns the default SSH public key path (~/.ssh/id_rsa.pub)
fn default_ssh_pub_key_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ssh/id_rsa.pub")
}
