//! Connection dialog implementation
//!
//! This is the main dialog file. Protocol-specific UI is in submodules:
//! - `super::ssh` - SSH options

// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use super::logging_tab;
use adw::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, ColorDialogButton, DrawingArea, DropDown, Entry, Label,
    ListBox, ListBoxRow, PasswordEntry, SpinButton, Stack, TextView,
};
use libadwaita as adw;
use rustconn_core::automation::ExpectRule;
use rustconn_core::models::{CustomProperty, HighlightRule, SharedFolder};
use rustconn_core::variables::Variable;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

/// Keyboard layout KLID values matching the dropdown order.
/// Index 0 = Auto (None), rest map to specific Windows KLIDs.
pub(super) const KEYBOARD_LAYOUT_KLIDS: &[u32] = &[
    0x0000, // Auto (placeholder, not used)
    0x0409, // US English
    0x0407, // German
    0x040C, // French
    0x040A, // Spanish
    0x0410, // Italian
    0x0816, // Portuguese
    0x0416, // Portuguese - Brazil
    0x0809, // English - UK
    0x0807, // German - Switzerland
    0x0C07, // German - Austria
    0x080C, // French - Belgium
    0x0413, // Dutch
    0x041D, // Swedish
    0x0414, // Norwegian
    0x0406, // Danish
    0x040B, // Finnish
    0x0415, // Polish
    0x0405, // Czech
    0x041B, // Slovak
    0x040E, // Hungarian
    0x0418, // Romanian
    0x041A, // Croatian
    0x0424, // Slovenian
    0x081A, // Serbian
    0x0402, // Bulgarian
    0x0419, // Russian
    0x0422, // Ukrainian
    0x041F, // Turkish
    0x0408, // Greek
    0x0411, // Japanese
    0x0412, // Korean
];

/// Converts a dropdown index to an `Option<u32>` KLID.
/// Index 0 = Auto (returns `None`).
pub(super) fn dropdown_index_to_klid(index: u32) -> Option<u32> {
    if index == 0 {
        return None;
    }
    KEYBOARD_LAYOUT_KLIDS.get(index as usize).copied()
}

/// Converts a KLID to a dropdown index. Returns 0 (Auto) if not found.
fn klid_to_dropdown_index(klid: u32) -> u32 {
    KEYBOARD_LAYOUT_KLIDS
        .iter()
        .position(|&k| k == klid)
        .map_or(0, |i| i as u32)
}

/// Connection dialog for creating/editing connections
#[expect(
    dead_code,
    reason = "Many fields kept for GTK widget lifecycle and signal handlers"
)]
pub struct ConnectionDialog {
    dialog: adw::Dialog,
    parent: Option<gtk4::Widget>,
    /// Header bar save button - stored for potential future use
    /// (e.g., enabling/disabling based on validation state)
    save_button: Button,
    /// Test connection button
    test_button: Button,
    /// Shared application state for vault operations
    state: crate::state::SharedAppState,
    // Basic fields
    name_entry: Entry,
    icon_entry: Entry,
    description_view: TextView,
    host_entry: Entry,
    port_spin: SpinButton,
    username_entry: Entry,
    domain_entry: Entry,
    tags_entry: Entry,
    protocol_dropdown: DropDown,
    protocol_stack: Stack,
    // Password source selection
    password_source_dropdown: DropDown,
    // Password entry and visibility toggle
    password_entry: Entry,
    password_visibility_button: Button,
    password_load_button: Button,
    vault_test_button: Button,
    password_row: GtkBox,
    // Variable name dropdown (for Variable password source)
    variable_dropdown: DropDown,
    variable_row: GtkBox,
    // Script command entry (for Script password source)
    script_command_entry: Entry,
    script_test_button: Button,
    script_row: GtkBox,
    // Group selection
    group_dropdown: DropDown,
    groups_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    // SSH fields
    ssh_auth_dropdown: DropDown,
    ssh_key_source_dropdown: DropDown,
    ssh_key_source_row: adw::ActionRow,
    ssh_key_entry: Entry,
    ssh_key_button: Button,
    ssh_agent_key_dropdown: DropDown,
    ssh_agent_keys: Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>>,
    /// Pending agent key selection (fingerprint, comment) to restore after refresh
    pending_agent_selection: Rc<RefCell<Option<(String, String)>>>,
    ssh_jump_host_dropdown: DropDown,
    ssh_proxy_entry: Entry,
    ssh_proxy_command_entry: Entry,
    ssh_identities_only: CheckButton,
    ssh_control_master: CheckButton,
    ssh_agent_forwarding: CheckButton,
    ssh_waypipe: CheckButton,
    ssh_x11_forwarding: CheckButton,
    ssh_compression: CheckButton,
    ssh_verbose: CheckButton,
    ssh_startup_entry: Entry,
    ssh_options_entry: Entry,
    ssh_agent_socket_entry: adw::EntryRow,
    ssh_pkcs11_entry: adw::EntryRow,
    ssh_remote_path_entry: adw::EntryRow,
    ssh_keep_alive_interval: adw::SpinRow,
    ssh_keep_alive_count_max: adw::SpinRow,
    ssh_port_forwards: Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
    ssh_port_forwards_list: gtk4::ListBox,
    // RDP fields
    rdp_client_mode_dropdown: DropDown,
    rdp_performance_mode_dropdown: DropDown,
    rdp_width_spin: SpinButton,
    rdp_height_spin: SpinButton,
    rdp_color_dropdown: DropDown,
    rdp_scale_override_dropdown: DropDown,
    rdp_audio_check: adw::SwitchRow,
    rdp_printer_check: adw::SwitchRow,
    rdp_gateway_entry: Entry,
    rdp_gateway_port_spin: SpinButton,
    rdp_gateway_username_entry: Entry,
    rdp_disable_nla_check: adw::SwitchRow,
    rdp_security_layer_dropdown: DropDown,
    rdp_tls_security_level_spin: SpinButton,
    rdp_ignore_certificate_check: adw::SwitchRow,
    rdp_clipboard_check: adw::SwitchRow,
    rdp_show_local_cursor_check: adw::SwitchRow,
    rdp_jiggler_check: adw::SwitchRow,
    rdp_jiggler_interval_spin: gtk4::SpinButton,
    rdp_autotype_delay_spin: gtk4::SpinButton,
    rdp_autotype_initial_delay_spin: gtk4::SpinButton,
    rdp_reconnect_on_resize_check: adw::SwitchRow,
    rdp_jump_host_dropdown: DropDown,
    rdp_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    rdp_shared_folders: Rc<RefCell<Vec<SharedFolder>>>,
    rdp_shared_folders_list: gtk4::ListBox,
    rdp_custom_args_entry: Entry,
    rdp_keyboard_layout_dropdown: DropDown,
    rdp_remote_app_program_entry: Entry,
    rdp_remote_app_args_entry: Entry,
    rdp_remote_app_name_entry: Entry,
    rdp_graphics_mode_dropdown: DropDown,
    // VNC fields
    vnc_client_mode_dropdown: DropDown,
    vnc_performance_mode_dropdown: DropDown,
    vnc_encoding_dropdown: DropDown,
    vnc_compression_spin: SpinButton,
    vnc_quality_spin: SpinButton,
    vnc_view_only_check: adw::SwitchRow,
    vnc_scaling_check: adw::SwitchRow,
    vnc_clipboard_check: adw::SwitchRow,
    vnc_show_local_cursor_check: adw::SwitchRow,
    vnc_scale_override_dropdown: DropDown,
    vnc_custom_args_entry: Entry,
    vnc_jump_host_dropdown: DropDown,
    vnc_accept_certificate_check: adw::SwitchRow,
    vnc_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    // SPICE fields
    spice_tls_check: adw::SwitchRow,
    spice_ca_cert_entry: Entry,
    spice_skip_verify_check: adw::SwitchRow,
    spice_usb_check: adw::SwitchRow,
    spice_clipboard_check: adw::SwitchRow,
    spice_show_local_cursor_check: adw::SwitchRow,
    spice_compression_dropdown: DropDown,
    spice_proxy_entry: Entry,
    spice_shared_folders: Rc<RefCell<Vec<SharedFolder>>>,
    spice_shared_folders_list: gtk4::ListBox,
    spice_jump_host_dropdown: DropDown,
    spice_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    spice_unix_socket_check: adw::SwitchRow,
    spice_socket_path_entry: Entry,
    // Zero Trust fields
    zt_provider_dropdown: DropDown,
    zt_provider_stack: Stack,
    // AWS SSM fields
    zt_aws_target_entry: adw::EntryRow,
    zt_aws_profile_entry: adw::EntryRow,
    zt_aws_region_entry: adw::EntryRow,
    // GCP IAP fields
    zt_gcp_instance_entry: adw::EntryRow,
    zt_gcp_zone_entry: adw::EntryRow,
    zt_gcp_project_entry: adw::EntryRow,
    // Azure Bastion fields
    zt_azure_bastion_resource_id_entry: adw::EntryRow,
    zt_azure_bastion_rg_entry: adw::EntryRow,
    zt_azure_bastion_name_entry: adw::EntryRow,
    // Azure SSH fields
    zt_azure_ssh_vm_entry: adw::EntryRow,
    zt_azure_ssh_rg_entry: adw::EntryRow,
    // OCI Bastion fields
    zt_oci_bastion_id_entry: adw::EntryRow,
    zt_oci_target_id_entry: adw::EntryRow,
    zt_oci_target_ip_entry: adw::EntryRow,
    zt_oci_ssh_key_entry: adw::EntryRow,
    zt_oci_session_ttl_spin: adw::SpinRow,
    // Cloudflare Access fields
    zt_cf_hostname_entry: adw::EntryRow,
    // Teleport fields
    zt_teleport_host_entry: adw::EntryRow,
    zt_teleport_cluster_entry: adw::EntryRow,
    // Tailscale SSH fields
    zt_tailscale_host_entry: adw::EntryRow,
    // Boundary fields
    zt_boundary_target_entry: adw::EntryRow,
    zt_boundary_addr_entry: adw::EntryRow,
    // Hoop.dev fields
    zt_hoop_connection_name_entry: adw::EntryRow,
    zt_hoop_gateway_url_entry: adw::EntryRow,
    zt_hoop_grpc_url_entry: adw::EntryRow,
    // Generic fields
    zt_generic_command_entry: adw::EntryRow,
    // Custom args for all providers
    zt_custom_args_entry: Entry,
    // Telnet fields
    telnet_custom_args_entry: Entry,
    telnet_backspace_dropdown: DropDown,
    telnet_delete_dropdown: DropDown,
    // MOSH fields (embedded in SSH tab, visible only when MOSH protocol selected)
    mosh_port_range_entry: Entry,
    mosh_predict_dropdown: DropDown,
    mosh_server_binary_entry: Entry,
    // Serial fields
    serial_device_entry: Entry,
    serial_baud_dropdown: DropDown,
    serial_data_bits_dropdown: DropDown,
    serial_stop_bits_dropdown: DropDown,
    serial_parity_dropdown: DropDown,
    serial_flow_control_dropdown: DropDown,
    serial_custom_args_entry: Entry,
    // Kubernetes fields
    k8s_kubeconfig_entry: Entry,
    k8s_context_entry: Entry,
    k8s_namespace_entry: Entry,
    k8s_pod_entry: Entry,
    k8s_container_entry: Entry,
    k8s_shell_dropdown: DropDown,
    k8s_busybox_check: CheckButton,
    k8s_busybox_image_entry: Entry,
    k8s_custom_args_entry: Entry,
    // Web fields
    web_browser_entry: Entry,
    web_private_mode_switch: adw::SwitchRow,
    // Variables fields
    variables_list: ListBox,
    variables_rows: Rc<RefCell<Vec<LocalVariableRow>>>,
    /// Button to add new variables - wired up in `wire_add_variable_button()`
    add_variable_button: Button,
    global_variables: Rc<RefCell<Vec<Variable>>>,
    // Logging tab
    logging_tab: logging_tab::LoggingTab,
    // Expect rules fields
    expect_rules_list: ListBox,
    expect_rules: Rc<RefCell<Vec<ExpectRule>>>,
    /// Button to add new expect rules - wired up in `wire_add_expect_rule_button()`
    add_expect_rule_button: Button,
    /// Entry for testing expect patterns - wired up in `wire_pattern_tester()`
    expect_pattern_test_entry: Entry,
    /// Label showing pattern test results - wired up in `wire_pattern_tester()`
    expect_test_result_label: Label,
    // Connection tasks fields
    pre_connect_enabled_switch: adw::SwitchRow,
    pre_connect_command_entry: Entry,
    pre_connect_timeout_spin: SpinButton,
    pre_connect_abort_switch: adw::SwitchRow,
    pre_connect_first_only_switch: adw::SwitchRow,
    post_disconnect_enabled_switch: adw::SwitchRow,
    post_disconnect_command_entry: Entry,
    post_disconnect_timeout_spin: SpinButton,
    post_disconnect_last_only_switch: adw::SwitchRow,
    // Custom properties fields
    custom_properties_list: ListBox,
    custom_properties: Rc<RefCell<Vec<CustomProperty>>>,
    /// Button to add custom properties - wired up in `wire_add_custom_property_button()`
    add_custom_property_button: Button,
    // WOL fields
    wol_enabled_check: CheckButton,
    wol_mac_entry: Entry,
    wol_broadcast_entry: Entry,
    wol_port_spin: SpinButton,
    wol_wait_spin: SpinButton,
    // Terminal theme fields
    theme_bg_button: ColorDialogButton,
    theme_fg_button: ColorDialogButton,
    theme_cursor_button: ColorDialogButton,
    theme_reset_button: Button,
    theme_preview: DrawingArea,
    // Remote monitoring override field
    monitoring_toggle: adw::SwitchRow,
    // Session recording field
    recording_toggle: adw::SwitchRow,
    // Highlight rules fields
    highlight_rules_list: ListBox,
    highlight_rules: Rc<RefCell<Vec<HighlightRule>>>,
    /// Button to add highlight rules
    add_highlight_rule_button: Button,
    // Activity monitor fields
    activity_mode_combo: adw::ComboRow,
    activity_quiet_period_spin: adw::SpinRow,
    activity_silence_timeout_spin: adw::SpinRow,
    // Retry config fields
    retry_enabled_toggle: adw::SwitchRow,
    retry_max_attempts_spin: adw::SpinRow,
    retry_initial_delay_spin: adw::SpinRow,
    retry_max_delay_spin: adw::SpinRow,
    // Skip pre-connect TCP port check for this connection
    skip_port_check_toggle: adw::SwitchRow,
    // Port knock sequence entry
    knock_sequence_entry: gtk4::Entry,
    // SPA (fwknop) fields
    spa_enabled_toggle: adw::SwitchRow,
    spa_rij_key_entry: adw::PasswordEntryRow,
    spa_hmac_key_entry: adw::PasswordEntryRow,
    spa_access_entry: adw::EntryRow,
    spa_port_spin: adw::SpinRow,
    spa_allow_ip_combo: adw::ComboRow,
    // State
    editing_id: Rc<RefCell<Option<Uuid>>>,
    // Callback
    on_save: super::ConnectionCallback,
    connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    full_groups_data: Rc<RefCell<HashMap<Uuid, rustconn_core::models::ConnectionGroup>>>,
}

/// Represents a local variable row in the connection dialog
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
struct LocalVariableRow {
    /// The row widget
    row: ListBoxRow,
    /// Entry for variable name
    name_entry: Entry,
    /// Entry for variable value (regular)
    value_entry: Entry,
    /// Entry for secret value (password)
    secret_entry: PasswordEntry,
    /// Checkbox for secret flag
    is_secret_check: CheckButton,
    /// Entry for description
    description_entry: Entry,
    /// Delete button
    delete_button: Button,
    /// Whether this is an inherited global variable (read-only name)
    is_inherited: bool,
}

/// Represents an expect rule row in the connection dialog
struct ExpectRuleRow {
    /// The row widget
    row: ListBoxRow,
    /// The rule ID
    id: Uuid,
    /// Entry for regex pattern
    pattern_entry: Entry,
    /// Entry for response
    response_entry: Entry,
    /// Spin button for priority
    priority_spin: SpinButton,
    /// Spin button for timeout (ms)
    timeout_spin: SpinButton,
    /// Checkbox for enabled state
    enabled_check: CheckButton,
    /// Checkbox for one-shot mode
    one_shot_check: CheckButton,
    /// Delete button
    delete_button: Button,
    /// Move up button
    move_up_button: Button,
    /// Move down button
    move_down_button: Button,
}

/// Represents a custom property row in the connection dialog
struct CustomPropertyRow {
    /// The row widget
    row: ListBoxRow,
    /// Entry for property name
    name_entry: Entry,
    /// Dropdown for property type
    type_dropdown: DropDown,
    /// Entry for property value (regular)
    value_entry: Entry,
    /// Entry for secret value (password)
    secret_entry: PasswordEntry,
    /// Delete button
    delete_button: Button,
}

impl ConnectionDialog {
    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<super::ConnectionDialogResult>) + 'static>(&self, cb: F) {
        // Store callback - the save button handler was connected in the constructor
        // and will invoke this callback when clicked
        *self.on_save.borrow_mut() = Some(Box::new(cb));

        // Refresh agent keys before showing the dialog
        self.refresh_agent_keys();

        self.dialog.present(self.parent.as_ref());
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}

mod agent_variables;
mod build;
mod construction;
mod passwords;
mod populate;
mod rows;
mod save;
