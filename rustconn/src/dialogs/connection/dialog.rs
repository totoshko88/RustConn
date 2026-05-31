//! Connection dialog implementation
//!
//! This is the main dialog file. Protocol-specific UI is in submodules:
//! - `super::ssh` - SSH options

// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use super::builders::ConnectionDialogData;
use super::logging_tab;
use super::ssh;
use crate::alert;
use crate::i18n::{i18n, i18n_f};
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, ColorDialogButton, DrawingArea, DropDown, Entry,
    FileDialog, Grid, Label, ListBox, ListBoxRow, Orientation, PasswordEntry, ScrolledWindow,
    SpinButton, Stack, StringList, TextView,
};
use libadwaita as adw;
use rustconn_core::activity_monitor::MonitorMode;
use rustconn_core::automation::{ConnectionTask, ExpectRule, builtin_templates};
use rustconn_core::models::{
    Connection, CustomProperty, HighlightRule, PasswordSource, PropertyType, ProtocolConfig,
    RdpConfig, SharedFolder, SpiceConfig, SpiceImageCompression, SshAuthMethod, SshConfig,
    SshKeySource, VncConfig, ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
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
#[allow(
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
    vnc_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    // SPICE fields
    spice_tls_check: adw::SwitchRow,
    spice_ca_cert_entry: Entry,
    spice_ca_cert_button: Button,
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
    // State
    editing_id: Rc<RefCell<Option<Uuid>>>,
    // Callback
    on_save: super::ConnectionCallback,
    connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    full_groups_data: Rc<RefCell<HashMap<Uuid, rustconn_core::models::ConnectionGroup>>>,
}

/// Represents a local variable row in the connection dialog
#[allow(dead_code, reason = "Fields kept for GTK widget lifecycle")]
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
    /// Creates a new connection dialog
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    pub fn new(parent: Option<&gtk4::Window>, state: crate::state::SharedAppState) -> Self {
        let (dialog, header, save_btn, test_btn) = Self::create_window_with_header(parent);
        let view_stack = Self::create_view_stack(&dialog, &header);

        // === Basic Tab ===
        let basic = super::general_tab::create_basic_tab();
        let basic_grid = &basic.container;
        let name_entry = basic.name_entry.clone();
        let icon_entry = basic.icon_entry.clone();
        let description_view = basic.description_view.clone();
        let host_entry = basic.host_entry.clone();
        let host_label = basic.host_label.clone();
        let port_spin = basic.port_spin.clone();
        let port_label = basic.port_label.clone();
        let username_entry = basic.username_entry.clone();
        let username_label = basic.username_label.clone();
        let domain_entry = basic.domain_entry.clone();
        let domain_label = basic.domain_label.clone();
        let tags_entry = basic.tags_entry.clone();
        let tags_label = basic.tags_label.clone();
        let protocol_dropdown = basic.protocol_dropdown.clone();
        let password_source_dropdown = basic.password_source_dropdown.clone();
        let password_source_label = basic.password_source_label.clone();
        let password_entry = basic.password_entry.clone();
        let password_visibility_button = basic.password_visibility_button.clone();
        let password_load_button = basic.password_load_button.clone();
        let vault_test_button = basic.vault_test_button.clone();
        let password_row = basic.password_row.clone();
        let variable_dropdown = basic.variable_dropdown.clone();
        let variable_row = basic.variable_row.clone();
        let group_dropdown = basic.group_dropdown.clone();
        let username_load_button = basic.username_load_button.clone();
        let domain_load_button = basic.domain_load_button.clone();
        let script_command_entry = basic.script_command_entry.clone();
        let script_test_button = basic.script_test_button.clone();
        let script_row = basic.script_row.clone();
        // Wrap basic grid in ScrolledWindow for consistent styling
        let basic_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(basic_grid)
            .build();
        view_stack
            .add_titled(&basic_scrolled, Some("basic"), &i18n("Basic"))
            .set_icon_name(Some("document-properties-symbolic"));

        // === Protocol-specific Tab ===
        let protocol_stack = Self::create_protocol_stack(&view_stack);

        // Storage for agent keys (populated when dialog is shown)
        let ssh_agent_keys: Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>> =
            Rc::new(RefCell::new(Vec::new()));

        // Pending agent key selection to restore after refresh_agent_keys()
        let pending_agent_selection: Rc<RefCell<Option<(String, String)>>> =
            Rc::new(RefCell::new(None));

        // Storage for port forwarding rules (created before SSH options so we can
        // append the port forwarding group to the SSH panel)
        let ssh_port_forwards: Rc<RefCell<Vec<rustconn_core::models::PortForward>>> =
            Rc::new(RefCell::new(Vec::new()));
        let ssh_port_forwards_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();

        // SSH options
        let ssh_widgets = ssh::create_ssh_options();
        let ssh_auth_dropdown = ssh_widgets.auth_dropdown;
        let ssh_key_source_dropdown = ssh_widgets.key_source_dropdown;
        let ssh_key_source_row = ssh_widgets.key_source_row;
        let ssh_key_entry = ssh_widgets.key_entry;
        let ssh_key_button = ssh_widgets.key_button;
        let ssh_agent_key_dropdown = ssh_widgets.agent_key_dropdown;
        let ssh_jump_host_dropdown = ssh_widgets.jump_host_dropdown;
        let ssh_proxy_entry = ssh_widgets.proxy_entry;
        let ssh_proxy_command_entry = ssh_widgets.proxy_command_entry;
        let ssh_identities_only = ssh_widgets.identities_only;
        let ssh_control_master = ssh_widgets.control_master;
        let ssh_agent_forwarding = ssh_widgets.agent_forwarding;
        let ssh_waypipe = ssh_widgets.waypipe;
        let ssh_x11_forwarding = ssh_widgets.x11_forwarding;
        let ssh_compression = ssh_widgets.compression;
        let ssh_verbose = ssh_widgets.verbose;
        let ssh_startup_entry = ssh_widgets.startup_entry;
        let ssh_options_entry = ssh_widgets.options_entry;
        let mosh_settings_group = ssh_widgets.mosh_group;
        let mosh_port_range_entry = ssh_widgets.mosh_port_range_entry;
        let mosh_predict_dropdown = ssh_widgets.mosh_predict_dropdown;
        let mosh_server_binary_entry = ssh_widgets.mosh_server_binary_entry;
        let ssh_agent_socket_entry = ssh_widgets.ssh_agent_socket_entry;
        let ssh_keep_alive_interval = ssh_widgets.keep_alive_interval;
        let ssh_keep_alive_count_max = ssh_widgets.keep_alive_count_max;

        // Add port forwarding group to SSH options panel
        {
            let pf_group =
                ssh::create_port_forwarding_group(&ssh_port_forwards_list, &ssh_port_forwards);
            ssh_widgets.content.append(&pf_group);
        }

        protocol_stack.add_named(&ssh_widgets.container, Some("ssh"));

        // RDP options
        let (
            rdp_box,
            rdp_client_mode_dropdown,
            rdp_performance_mode_dropdown,
            rdp_width_spin,
            rdp_height_spin,
            rdp_color_dropdown,
            rdp_scale_override_dropdown,
            rdp_audio_check,
            rdp_gateway_entry,
            rdp_gateway_port_spin,
            rdp_gateway_username_entry,
            rdp_disable_nla_check,
            rdp_security_layer_dropdown,
            rdp_tls_security_level_spin,
            ignore_certificate_check,
            rdp_clipboard_check,
            rdp_show_local_cursor_check,
            rdp_jiggler_check,
            rdp_jiggler_interval_spin,
            rdp_autotype_delay_spin,
            rdp_autotype_initial_delay_spin,
            rdp_reconnect_on_resize_check,
            rdp_jump_host_dropdown,
            rdp_shared_folders,
            rdp_shared_folders_list,
            rdp_custom_args_entry,
            rdp_keyboard_layout_dropdown,
            rdp_remote_app_program_entry,
            rdp_remote_app_args_entry,
            rdp_remote_app_name_entry,
        ) = super::rdp::create_rdp_options();
        protocol_stack.add_named(&rdp_box, Some("rdp"));

        // VNC options
        let (
            vnc_box,
            vnc_client_mode_dropdown,
            vnc_performance_mode_dropdown,
            vnc_encoding_dropdown,
            vnc_compression_spin,
            vnc_quality_spin,
            vnc_view_only_check,
            vnc_scaling_check,
            vnc_clipboard_check,
            vnc_show_local_cursor_check,
            vnc_scale_override_dropdown,
            vnc_custom_args_entry,
            vnc_jump_host_dropdown,
        ) = super::vnc::create_vnc_options();
        protocol_stack.add_named(&vnc_box, Some("vnc"));

        // SPICE options
        let (
            spice_box,
            spice_tls_check,
            spice_ca_cert_entry,
            spice_ca_cert_button,
            spice_skip_verify_check,
            spice_usb_check,
            spice_clipboard_check,
            spice_compression_dropdown,
            spice_proxy_entry,
            spice_show_local_cursor_check,
            spice_shared_folders,
            spice_shared_folders_list,
            spice_jump_host_dropdown,
        ) = super::spice::create_spice_options();
        protocol_stack.add_named(&spice_box, Some("spice"));

        // Zero Trust options
        let (
            zt_box,
            zt_provider_dropdown,
            zt_provider_stack,
            zt_aws_target_entry,
            zt_aws_profile_entry,
            zt_aws_region_entry,
            zt_gcp_instance_entry,
            zt_gcp_zone_entry,
            zt_gcp_project_entry,
            zt_azure_bastion_resource_id_entry,
            zt_azure_bastion_rg_entry,
            zt_azure_bastion_name_entry,
            zt_azure_ssh_vm_entry,
            zt_azure_ssh_rg_entry,
            zt_oci_bastion_id_entry,
            zt_oci_target_id_entry,
            zt_oci_target_ip_entry,
            zt_oci_ssh_key_entry,
            zt_oci_session_ttl_spin,
            zt_cf_hostname_entry,
            zt_teleport_host_entry,
            zt_teleport_cluster_entry,
            zt_tailscale_host_entry,
            zt_boundary_target_entry,
            zt_boundary_addr_entry,
            zt_hoop_connection_name_entry,
            zt_hoop_gateway_url_entry,
            zt_hoop_grpc_url_entry,
            zt_generic_command_entry,
            zt_custom_args_entry,
        ) = super::zerotrust::create_zerotrust_options();
        protocol_stack.add_named(&zt_box, Some("zerotrust"));

        // Telnet options
        let (
            telnet_box,
            telnet_custom_args_entry,
            telnet_backspace_dropdown,
            telnet_delete_dropdown,
        ) = super::telnet::create_telnet_options();
        protocol_stack.add_named(&telnet_box, Some("telnet"));

        // Serial options
        let (
            serial_box,
            serial_device_entry,
            serial_baud_dropdown,
            serial_data_bits_dropdown,
            serial_stop_bits_dropdown,
            serial_parity_dropdown,
            serial_flow_control_dropdown,
            serial_custom_args_entry,
        ) = super::serial::create_serial_options();
        protocol_stack.add_named(&serial_box, Some("serial"));

        // Kubernetes options
        let (
            k8s_box,
            k8s_kubeconfig_entry,
            k8s_context_entry,
            k8s_namespace_entry,
            k8s_pod_entry,
            k8s_container_entry,
            k8s_shell_dropdown,
            k8s_busybox_check,
            k8s_busybox_image_entry,
            k8s_custom_args_entry,
        ) = super::kubernetes::create_kubernetes_options();
        protocol_stack.add_named(&k8s_box, Some("kubernetes"));

        // Web bookmark options page
        let (web_box, web_browser_entry, web_private_mode_switch) =
            super::web::create_web_options();
        protocol_stack.add_named(&web_box, Some("web"));

        // MOSH now uses SSH tab with additional MOSH settings group
        // (mosh_port_range_entry, mosh_predict_dropdown, mosh_server_binary_entry
        //  are already created in ssh::create_ssh_options above)

        // Set initial protocol view
        protocol_stack.set_visible_child_name("ssh");

        // Connect protocol dropdown to stack
        Self::connect_protocol_dropdown(
            &protocol_dropdown,
            &protocol_stack,
            &port_spin,
            &host_entry,
            &host_label,
            &port_label,
            &username_entry,
            &username_label,
            &tags_entry,
            &tags_label,
            &password_source_dropdown,
            &password_source_label,
            &password_row,
            &domain_entry,
            &domain_label,
            &mosh_settings_group,
        );

        // === Data Tab (Variables + Custom Properties) ===
        let (
            data_tab,
            variables_list,
            add_variable_button,
            custom_properties_list,
            add_custom_property_button,
        ) = super::data_tab::create_data_tab();
        view_stack
            .add_titled(&data_tab, Some("data"), &i18n("Data"))
            .set_icon_name(Some("accessories-text-editor-symbolic"));

        let variables_rows: Rc<RefCell<Vec<LocalVariableRow>>> = Rc::new(RefCell::new(Vec::new()));
        let global_variables: Rc<RefCell<Vec<Variable>>> = Rc::new(RefCell::new(Vec::new()));
        let custom_properties: Rc<RefCell<Vec<CustomProperty>>> = Rc::new(RefCell::new(Vec::new()));

        // === Logging Tab ===
        let (logging_tab_box, logging_tab_struct) = logging_tab::LoggingTab::new();
        view_stack
            .add_titled(&logging_tab_box, Some("logging"), &i18n("Logging"))
            .set_icon_name(Some("document-save-symbolic"));

        // === Automation Tab (Expect Rules + Tasks) ===
        let automation_widgets = super::automation_tab::create_automation_combined_tab();
        view_stack
            .add_titled(
                &automation_widgets.container,
                Some("automation"),
                &i18n("Automation"),
            )
            .set_icon_name(Some("system-run-symbolic"));

        let expect_rules: Rc<RefCell<Vec<ExpectRule>>> = Rc::new(RefCell::new(Vec::new()));

        // === Advanced Tab (Display + WOL) ===
        let (
            advanced_tab,
            wol_enabled_check,
            wol_mac_entry,
            wol_broadcast_entry,
            wol_port_spin,
            wol_wait_spin,
            theme_bg_button,
            theme_fg_button,
            theme_cursor_button,
            theme_reset_button,
            theme_preview,
            monitoring_toggle,
            recording_toggle,
            highlight_rules_list,
            add_highlight_rule_button,
            _theme_preset_dropdown,
            activity_mode_combo,
            activity_quiet_period_spin,
            activity_silence_timeout_spin,
            retry_enabled_toggle,
            retry_max_attempts_spin,
            retry_initial_delay_spin,
            retry_max_delay_spin,
            skip_port_check_toggle,
        ) = super::advanced_tab::create_advanced_tab();
        view_stack
            .add_titled(&advanced_tab, Some("advanced"), &i18n("Advanced"))
            .set_icon_name(Some("preferences-system-symbolic"));

        let highlight_rules: Rc<RefCell<Vec<HighlightRule>>> = Rc::new(RefCell::new(Vec::new()));

        // Wire up add variable button
        Self::wire_add_variable_button(&add_variable_button, &variables_list, &variables_rows);

        // Wire up add expect rule button
        Self::wire_add_expect_rule_button(
            &automation_widgets.add_expect_rule_button,
            &automation_widgets.expect_rules_list,
            &expect_rules,
        );

        // Wire up template picker buttons
        Self::wire_template_buttons(
            &automation_widgets.template_list_box,
            &automation_widgets.expect_rules_list,
            &expect_rules,
        );

        // Wire up pattern tester
        Self::wire_pattern_tester(
            &automation_widgets.expect_pattern_test_entry,
            &automation_widgets.expect_test_result_label,
            &expect_rules,
        );

        // Wire up add custom property button
        Self::wire_add_custom_property_button(
            &add_custom_property_button,
            &custom_properties_list,
            &custom_properties,
        );

        // Wire up add highlight rule button
        Self::wire_add_highlight_rule_button(
            &add_highlight_rule_button,
            &highlight_rules_list,
            &highlight_rules,
        );

        let on_save: super::ConnectionCallback = Rc::new(RefCell::new(None));
        let editing_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));
        let groups_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(Root)".to_string())]));
        let connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(None)".to_string())]));
        let rdp_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(None)".to_string())]));
        let vnc_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(None)".to_string())]));
        let spice_connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(None)".to_string())]));

        // Connect save button handler
        Self::connect_save_button(
            &save_btn,
            &dialog,
            &on_save,
            &state,
            &editing_id,
            &name_entry,
            &icon_entry,
            &description_view,
            &host_entry,
            &port_spin,
            &username_entry,
            &domain_entry,
            &tags_entry,
            &protocol_dropdown,
            &password_source_dropdown,
            &password_entry,
            &variable_dropdown,
            &group_dropdown,
            &groups_data,
            &ssh_auth_dropdown,
            &ssh_key_source_dropdown,
            &ssh_key_entry,
            &ssh_agent_key_dropdown,
            &ssh_agent_keys,
            &ssh_jump_host_dropdown,
            &ssh_proxy_entry,
            &ssh_proxy_command_entry,
            &ssh_identities_only,
            &ssh_control_master,
            &ssh_agent_forwarding,
            &ssh_waypipe,
            &ssh_x11_forwarding,
            &ssh_compression,
            &ssh_verbose,
            &ssh_startup_entry,
            &ssh_options_entry,
            &ssh_agent_socket_entry,
            &ssh_keep_alive_interval,
            &ssh_keep_alive_count_max,
            &ssh_port_forwards,
            &rdp_client_mode_dropdown,
            &rdp_performance_mode_dropdown,
            &rdp_width_spin,
            &rdp_height_spin,
            &rdp_color_dropdown,
            &rdp_scale_override_dropdown,
            &rdp_audio_check,
            &rdp_gateway_entry,
            &rdp_gateway_port_spin,
            &rdp_gateway_username_entry,
            &rdp_disable_nla_check,
            &rdp_security_layer_dropdown,
            &rdp_tls_security_level_spin,
            &ignore_certificate_check,
            &rdp_clipboard_check,
            &rdp_show_local_cursor_check,
            &rdp_jiggler_check,
            &rdp_jiggler_interval_spin,
            &rdp_autotype_delay_spin,
            &rdp_autotype_initial_delay_spin,
            &rdp_reconnect_on_resize_check,
            &rdp_jump_host_dropdown,
            &rdp_connections_data,
            &rdp_shared_folders,
            &rdp_custom_args_entry,
            &rdp_keyboard_layout_dropdown,
            &rdp_remote_app_program_entry,
            &rdp_remote_app_args_entry,
            &rdp_remote_app_name_entry,
            &vnc_client_mode_dropdown,
            &vnc_performance_mode_dropdown,
            &vnc_encoding_dropdown,
            &vnc_compression_spin,
            &vnc_quality_spin,
            &vnc_view_only_check,
            &vnc_scaling_check,
            &vnc_clipboard_check,
            &vnc_show_local_cursor_check,
            &vnc_scale_override_dropdown,
            &vnc_custom_args_entry,
            &vnc_jump_host_dropdown,
            &vnc_connections_data,
            &spice_tls_check,
            &spice_ca_cert_entry,
            &spice_skip_verify_check,
            &spice_usb_check,
            &spice_clipboard_check,
            &spice_show_local_cursor_check,
            &spice_compression_dropdown,
            &spice_proxy_entry,
            &spice_shared_folders,
            &spice_jump_host_dropdown,
            &spice_connections_data,
            &zt_provider_dropdown,
            &zt_aws_target_entry,
            &zt_aws_profile_entry,
            &zt_aws_region_entry,
            &zt_gcp_instance_entry,
            &zt_gcp_zone_entry,
            &zt_gcp_project_entry,
            &zt_azure_bastion_resource_id_entry,
            &zt_azure_bastion_rg_entry,
            &zt_azure_bastion_name_entry,
            &zt_azure_ssh_vm_entry,
            &zt_azure_ssh_rg_entry,
            &zt_oci_bastion_id_entry,
            &zt_oci_target_id_entry,
            &zt_oci_target_ip_entry,
            &zt_oci_ssh_key_entry,
            &zt_oci_session_ttl_spin,
            &zt_cf_hostname_entry,
            &zt_teleport_host_entry,
            &zt_teleport_cluster_entry,
            &zt_tailscale_host_entry,
            &zt_boundary_target_entry,
            &zt_boundary_addr_entry,
            &zt_hoop_connection_name_entry,
            &zt_hoop_gateway_url_entry,
            &zt_hoop_grpc_url_entry,
            &zt_generic_command_entry,
            &zt_custom_args_entry,
            &telnet_custom_args_entry,
            &telnet_backspace_dropdown,
            &telnet_delete_dropdown,
            &serial_device_entry,
            &serial_baud_dropdown,
            &serial_data_bits_dropdown,
            &serial_stop_bits_dropdown,
            &serial_parity_dropdown,
            &serial_flow_control_dropdown,
            &serial_custom_args_entry,
            &k8s_kubeconfig_entry,
            &k8s_context_entry,
            &k8s_namespace_entry,
            &k8s_pod_entry,
            &k8s_container_entry,
            &k8s_shell_dropdown,
            &k8s_busybox_check,
            &k8s_busybox_image_entry,
            &k8s_custom_args_entry,
            &mosh_port_range_entry,
            &mosh_predict_dropdown,
            &mosh_server_binary_entry,
            &web_browser_entry,
            &web_private_mode_switch,
            &variables_rows,
            &logging_tab_struct,
            &expect_rules,
            &automation_widgets.pre_connect_enabled_switch,
            &automation_widgets.pre_connect_command_entry,
            &automation_widgets.pre_connect_timeout_spin,
            &automation_widgets.pre_connect_abort_switch,
            &automation_widgets.pre_connect_first_only_switch,
            &automation_widgets.post_disconnect_enabled_switch,
            &automation_widgets.post_disconnect_command_entry,
            &automation_widgets.post_disconnect_timeout_spin,
            &automation_widgets.post_disconnect_last_only_switch,
            &custom_properties,
            &wol_enabled_check,
            &wol_mac_entry,
            &wol_broadcast_entry,
            &wol_port_spin,
            &wol_wait_spin,
            &theme_bg_button,
            &theme_fg_button,
            &theme_cursor_button,
            &connections_data,
            &script_command_entry,
            &monitoring_toggle,
            &recording_toggle,
            &highlight_rules,
            &activity_mode_combo,
            &activity_quiet_period_spin,
            &activity_silence_timeout_spin,
            &retry_enabled_toggle,
            &retry_max_attempts_spin,
            &retry_initial_delay_spin,
            &retry_max_delay_spin,
            &skip_port_check_toggle,
        );

        let result = Self {
            dialog,
            parent: parent.map(|p| p.clone().upcast::<gtk4::Widget>()),
            save_button: save_btn,
            test_button: test_btn,
            state,
            name_entry,
            icon_entry,
            description_view,
            host_entry,
            port_spin,
            username_entry,
            domain_entry,
            tags_entry,
            protocol_dropdown,
            protocol_stack,
            password_source_dropdown,
            password_entry,
            password_visibility_button,
            password_load_button,
            vault_test_button,
            password_row,
            variable_dropdown,
            variable_row,
            script_command_entry,
            script_test_button,
            script_row,
            group_dropdown,
            groups_data,
            full_groups_data: Rc::new(RefCell::new(HashMap::new())),
            ssh_auth_dropdown,
            ssh_key_source_dropdown,
            ssh_key_source_row,
            ssh_key_entry,
            ssh_key_button,
            ssh_agent_key_dropdown,
            ssh_agent_keys,
            pending_agent_selection,
            ssh_jump_host_dropdown,
            ssh_proxy_entry,
            ssh_proxy_command_entry,
            ssh_identities_only,
            ssh_control_master,
            ssh_agent_forwarding,
            ssh_waypipe,
            ssh_x11_forwarding,
            ssh_compression,
            ssh_verbose,
            ssh_startup_entry,
            ssh_options_entry,
            ssh_agent_socket_entry,
            ssh_keep_alive_interval,
            ssh_keep_alive_count_max,
            ssh_port_forwards,
            ssh_port_forwards_list,
            rdp_client_mode_dropdown,
            rdp_performance_mode_dropdown,
            rdp_width_spin,
            rdp_height_spin,
            rdp_color_dropdown,
            rdp_scale_override_dropdown,
            rdp_audio_check,
            rdp_gateway_entry,
            rdp_gateway_port_spin,
            rdp_gateway_username_entry,
            rdp_disable_nla_check,
            rdp_security_layer_dropdown,
            rdp_tls_security_level_spin,
            rdp_ignore_certificate_check: ignore_certificate_check,
            rdp_clipboard_check,
            rdp_show_local_cursor_check,
            rdp_jiggler_check,
            rdp_jiggler_interval_spin,
            rdp_autotype_delay_spin,
            rdp_autotype_initial_delay_spin,
            rdp_reconnect_on_resize_check,
            rdp_jump_host_dropdown,
            rdp_connections_data,
            rdp_shared_folders,
            rdp_shared_folders_list,
            rdp_custom_args_entry,
            rdp_keyboard_layout_dropdown,
            rdp_remote_app_program_entry,
            rdp_remote_app_args_entry,
            rdp_remote_app_name_entry,
            vnc_client_mode_dropdown,
            vnc_performance_mode_dropdown,
            vnc_encoding_dropdown,
            vnc_compression_spin,
            vnc_quality_spin,
            vnc_view_only_check,
            vnc_scaling_check,
            vnc_clipboard_check,
            vnc_show_local_cursor_check,
            vnc_scale_override_dropdown,
            vnc_custom_args_entry,
            vnc_jump_host_dropdown,
            vnc_connections_data,
            spice_tls_check,
            variables_list,
            variables_rows,
            add_variable_button,
            global_variables,
            logging_tab: logging_tab_struct,
            spice_ca_cert_entry,
            spice_ca_cert_button,
            spice_skip_verify_check,
            spice_usb_check,
            spice_clipboard_check,
            spice_show_local_cursor_check,
            spice_compression_dropdown,
            spice_proxy_entry,
            spice_shared_folders,
            spice_shared_folders_list,
            spice_jump_host_dropdown,
            spice_connections_data,
            zt_provider_dropdown,
            zt_provider_stack,
            zt_aws_target_entry,
            zt_aws_profile_entry,
            zt_aws_region_entry,
            zt_gcp_instance_entry,
            zt_gcp_zone_entry,
            zt_gcp_project_entry,
            zt_azure_bastion_resource_id_entry,
            zt_azure_bastion_rg_entry,
            zt_azure_bastion_name_entry,
            zt_azure_ssh_vm_entry,
            zt_azure_ssh_rg_entry,
            zt_oci_bastion_id_entry,
            zt_oci_target_id_entry,
            zt_oci_target_ip_entry,
            zt_oci_ssh_key_entry,
            zt_oci_session_ttl_spin,
            zt_cf_hostname_entry,
            zt_teleport_host_entry,
            zt_teleport_cluster_entry,
            zt_tailscale_host_entry,
            zt_boundary_target_entry,
            zt_boundary_addr_entry,
            zt_hoop_connection_name_entry,
            zt_hoop_gateway_url_entry,
            zt_hoop_grpc_url_entry,
            zt_generic_command_entry,
            zt_custom_args_entry,
            telnet_custom_args_entry,
            telnet_backspace_dropdown,
            telnet_delete_dropdown,
            serial_device_entry,
            serial_baud_dropdown,
            serial_data_bits_dropdown,
            serial_stop_bits_dropdown,
            serial_parity_dropdown,
            serial_flow_control_dropdown,
            serial_custom_args_entry,
            k8s_kubeconfig_entry,
            k8s_context_entry,
            k8s_namespace_entry,
            k8s_pod_entry,
            k8s_container_entry,
            k8s_shell_dropdown,
            k8s_busybox_check,
            k8s_busybox_image_entry,
            k8s_custom_args_entry,
            web_browser_entry,
            web_private_mode_switch,
            mosh_port_range_entry,
            mosh_predict_dropdown,
            mosh_server_binary_entry,
            expect_rules_list: automation_widgets.expect_rules_list,
            expect_rules,
            add_expect_rule_button: automation_widgets.add_expect_rule_button,
            expect_pattern_test_entry: automation_widgets.expect_pattern_test_entry,
            expect_test_result_label: automation_widgets.expect_test_result_label,
            pre_connect_enabled_switch: automation_widgets.pre_connect_enabled_switch,
            pre_connect_command_entry: automation_widgets.pre_connect_command_entry,
            pre_connect_timeout_spin: automation_widgets.pre_connect_timeout_spin,
            pre_connect_abort_switch: automation_widgets.pre_connect_abort_switch,
            pre_connect_first_only_switch: automation_widgets.pre_connect_first_only_switch,
            post_disconnect_enabled_switch: automation_widgets.post_disconnect_enabled_switch,
            post_disconnect_command_entry: automation_widgets.post_disconnect_command_entry,
            post_disconnect_timeout_spin: automation_widgets.post_disconnect_timeout_spin,
            post_disconnect_last_only_switch: automation_widgets.post_disconnect_last_only_switch,
            custom_properties_list,
            custom_properties,
            add_custom_property_button,
            wol_enabled_check,
            wol_mac_entry,
            wol_broadcast_entry,
            wol_port_spin,
            wol_wait_spin,
            theme_bg_button,
            theme_fg_button,
            theme_cursor_button,
            theme_reset_button,
            theme_preview,
            monitoring_toggle,
            recording_toggle,
            highlight_rules_list,
            highlight_rules,
            add_highlight_rule_button,
            activity_mode_combo,
            activity_quiet_period_spin,
            activity_silence_timeout_spin,
            retry_enabled_toggle,
            retry_max_attempts_spin,
            retry_initial_delay_spin,
            retry_max_delay_spin,
            skip_port_check_toggle,
            editing_id,
            on_save,
            connections_data,
        };

        // Wire up inline validation for required fields
        Self::setup_inline_validation_for(&result);

        // Wire up Group Inheritance
        {
            let group_dropdown = result.group_dropdown.clone();
            let username_load_button = username_load_button.clone();
            let domain_load_button = domain_load_button.clone();
            let username_entry = result.username_entry.clone();
            let domain_entry = result.domain_entry.clone();
            let groups_data = Rc::clone(&result.groups_data);
            let full_groups_data = Rc::clone(&result.full_groups_data);

            // Helper to update button sensitivity
            let user_btn_for_update = username_load_button.clone();
            let domain_btn_for_update = domain_load_button.clone();
            let update_buttons = Rc::new(move |selected_idx: u32| {
                let sensitive = selected_idx > 0; // 0 is Root
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "value range fits the target type by construction in this code path"
                )]
                {
                    user_btn_for_update.set_sensitive(sensitive);
                    domain_btn_for_update.set_sensitive(sensitive);
                }
            });

            // Connect dropdown change
            let update_buttons_clone = update_buttons.clone();
            let groups_data_clone = groups_data.clone();
            let full_groups_data_clone = full_groups_data.clone();
            let username_entry_clone = username_entry.clone();
            let domain_entry_clone = domain_entry.clone();

            group_dropdown.connect_selected_notify(move |dropdown| {
                let idx = dropdown.selected();
                update_buttons_clone(idx);

                // Auto-populate if fields are empty
                if idx > 0 {
                    let groups = groups_data_clone.borrow();
                    if let Some((Some(group_id), _)) = groups.get(idx as usize)
                        && let Some(group) = full_groups_data_clone.borrow().get(group_id)
                    {
                        if username_entry_clone.text().is_empty()
                            && let Some(username) = &group.username
                            && !username.is_empty()
                        {
                            username_entry_clone.set_text(username);
                        }
                        if domain_entry_clone.text().is_empty()
                            && let Some(domain) = &group.domain
                            && !domain.is_empty()
                        {
                            domain_entry_clone.set_text(domain);
                        }
                    }
                }
            });

            // Connect Username Load Button
            let group_dropdown_clone = group_dropdown.clone();
            let groups_data_clone = groups_data.clone();
            let full_groups_data_clone = full_groups_data.clone();
            let username_entry_clone = username_entry.clone();
            let dialog_clone = result.dialog.clone();

            username_load_button.connect_clicked(move |_| {
                let idx = group_dropdown_clone.selected();
                if idx > 0 {
                    let groups = groups_data_clone.borrow();
                    if let Some((Some(group_id), _)) = groups.get(idx as usize)
                        && let Some(group) = full_groups_data_clone.borrow().get(group_id)
                    {
                        if let Some(username) = &group.username
                            && !username.is_empty()
                        {
                            username_entry_clone.set_text(username);
                            alert::show_alert(
                                &dialog_clone,
                                &i18n("Username Loaded"),
                                &i18n("Username loaded from group"),
                            );
                            return;
                        }
                        alert::show_alert(
                            &dialog_clone,
                            &i18n("No Username"),
                            &i18n("Group has no username defined"),
                        );
                    }
                }
            });

            // Connect Domain Load Button
            let group_dropdown_clone = group_dropdown.clone();
            let groups_data_clone = groups_data.clone();
            let full_groups_data_clone = full_groups_data.clone();
            let domain_entry_clone = domain_entry.clone();
            let dialog_clone = result.dialog.clone();

            domain_load_button.connect_clicked(move |_| {
                let idx = group_dropdown_clone.selected();
                if idx > 0 {
                    let groups = groups_data_clone.borrow();
                    if let Some((Some(group_id), _)) = groups.get(idx as usize)
                        && let Some(group) = full_groups_data_clone.borrow().get(group_id)
                    {
                        if let Some(domain) = &group.domain
                            && !domain.is_empty()
                        {
                            domain_entry_clone.set_text(domain);
                            alert::show_alert(
                                &dialog_clone,
                                &i18n("Domain Loaded"),
                                &i18n("Domain loaded from group"),
                            );
                            return;
                        }
                        alert::show_alert(
                            &dialog_clone,
                            &i18n("No Domain"),
                            &i18n("Group has no domain defined"),
                        );
                    }
                }
            });
        }

        // Set up test button handler
        let test_button = result.test_button.clone();
        let name_entry = result.name_entry.clone();
        let host_entry = result.host_entry.clone();
        let port_spin = result.port_spin.clone();
        let protocol_dropdown = result.protocol_dropdown.clone();
        let _username_entry = result.username_entry.clone();
        let window = result.dialog.clone();
        let window_for_script = window.clone();

        test_button.connect_clicked(move |btn| {
            // Validate required fields
            let name = name_entry.text();
            let host = host_entry.text();
            let protocol_index = protocol_dropdown.selected();

            // Zero Trust connections have different validation requirements
            if protocol_index == 4 {
                // Zero Trust - show info message about testing
                alert::show_alert(
                    &window,
                    &i18n("Zero Trust Connection Test"),
                    &i18n("Zero Trust connections require provider-specific authentication.\n\nTo test the connection:\n1. Save the connection first\n2. Use the Connect button to initiate the connection\n3. The provider CLI will handle authentication"),
                );
                return;
            }

            if name.trim().is_empty() || host.trim().is_empty() {
                alert::show_error(
                    &window,
                    &i18n("Connection Test Failed"),
                    &i18n(&i18n("Please fill in required fields (name and host)")),
                );
                return;
            }

            // Create a minimal connection for testing
            #[expect(
    clippy::cast_sign_loss,
    reason = "value is non-negative by construction in this code path"
)]
            let port = port_spin.value().max(0.0) as u16;

            let protocol_config = match protocol_index {
                0 => rustconn_core::models::ProtocolConfig::Ssh(
                    rustconn_core::models::SshConfig::default(),
                ),
                1 => rustconn_core::models::ProtocolConfig::Rdp(
                    rustconn_core::models::RdpConfig::default(),
                ),
                2 => rustconn_core::models::ProtocolConfig::Vnc(
                    rustconn_core::models::VncConfig::default(),
                ),
                3 => rustconn_core::models::ProtocolConfig::Spice(
                    rustconn_core::models::SpiceConfig::default(),
                ),
                8 => rustconn_core::models::ProtocolConfig::Kubernetes(
                    rustconn_core::models::KubernetesConfig::default(),
                ),
                _ => rustconn_core::models::ProtocolConfig::Ssh(
                    rustconn_core::models::SshConfig::default(),
                ),
            };

            let connection = rustconn_core::models::Connection::new(
                name.to_string(),
                host.to_string(),
                port,
                protocol_config,
            );

            // Show testing status
            btn.set_sensitive(false);
            btn.set_label(&i18n("Testing..."));

            // Clone data needed for the test (not GTK widgets)
            let host = connection.host.clone();
            let port = connection.port;
            let conn_id = connection.id;
            let conn_name = connection.name.clone();
            let protocol = connection.protocol;

            // Perform the test in a background thread with tokio runtime
            let test_button_clone = btn.clone();
            let window_clone = window.clone();

            // Use spawn_blocking_with_timeout utility for cleaner async handling
            crate::utils::spawn_blocking_with_timeout(
                move || {
                    let tester = rustconn_core::testing::ConnectionTester::new();

                    // Create a minimal connection for testing
                    let protocol_config = match protocol {
                        rustconn_core::models::ProtocolType::Ssh => {
                            rustconn_core::models::ProtocolConfig::Ssh(
                                rustconn_core::models::SshConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Rdp => {
                            rustconn_core::models::ProtocolConfig::Rdp(
                                rustconn_core::models::RdpConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Vnc => {
                            rustconn_core::models::ProtocolConfig::Vnc(
                                rustconn_core::models::VncConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Spice => {
                            rustconn_core::models::ProtocolConfig::Spice(
                                rustconn_core::models::SpiceConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::ZeroTrust => {
                            rustconn_core::models::ProtocolConfig::Ssh(
                                rustconn_core::models::SshConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Telnet => {
                            rustconn_core::models::ProtocolConfig::Telnet(
                                rustconn_core::models::TelnetConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Serial => {
                            rustconn_core::models::ProtocolConfig::Serial(
                                rustconn_core::models::SerialConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Sftp => {
                            rustconn_core::models::ProtocolConfig::Sftp(
                                rustconn_core::models::SshConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Kubernetes => {
                            rustconn_core::models::ProtocolConfig::Kubernetes(
                                rustconn_core::models::KubernetesConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Mosh => {
                            rustconn_core::models::ProtocolConfig::Mosh(
                                rustconn_core::models::MoshConfig::default(),
                            )
                        }
                        rustconn_core::models::ProtocolType::Web => {
                            rustconn_core::models::ProtocolConfig::Web(
                                rustconn_core::models::WebConfig::default(),
                            )
                        }
                    };
                    let mut test_conn = rustconn_core::models::Connection::new(
                        conn_name.clone(),
                        host,
                        port,
                        protocol_config,
                    );
                    test_conn.id = conn_id;

                    match crate::async_utils::with_runtime(|rt| {
                        rt.block_on(tester.test_connection(&test_conn))
                    }) {
                        Ok(result) => result,
                        Err(e) => rustconn_core::testing::TestResult::failure(
                            conn_id,
                            conn_name,
                            &format!("Runtime error: {e}"),
                        ),
                    }
                },
                std::time::Duration::from_secs(15),
                move |result: Option<rustconn_core::testing::TestResult>| {
                    // Update UI
                    test_button_clone.set_sensitive(true);
                    test_button_clone.set_label(&i18n("Test"));

                    match result {
                        Some(test_result) if test_result.is_success() => {
                            let latency = test_result.latency_ms.unwrap_or(0);
                            alert::show_success(
                                &window_clone,
                                &i18n("Connection Test Successful"),
                                &i18n_f(
                                    "Connection successful! Latency: {}ms",
                                    &[&latency.to_string()],
                                ),
                            );
                        }
                        Some(test_result) => {
                            let error = test_result
                                .error
                                .unwrap_or_else(|| i18n("Unknown error"));
                            alert::show_error(&window_clone, &i18n("Connection Test Failed"), &error);
                        }
                        None => {
                            alert::show_error(
                                &window_clone,
                                &i18n("Connection Test Failed"),
                                &i18n("Test timed out"),
                            );
                        }
                    }
                },
            );
        });

        // Script credentials test button
        {
            let script_entry = result.script_command_entry.clone();
            let script_btn = result.script_test_button.clone();
            let window = window_for_script;
            result.script_test_button.connect_clicked(move |_| {
                let cmd_text = script_entry.text().to_string();
                if cmd_text.trim().is_empty() {
                    alert::show_error(
                        &window,
                        &i18n("Script Test Failed"),
                        &i18n("Script command is empty"),
                    );
                    return;
                }

                let args = match shell_words::split(&cmd_text) {
                    Ok(a) if a.is_empty() => {
                        alert::show_error(
                            &window,
                            &i18n("Script Test Failed"),
                            &i18n("Script command is empty after parsing"),
                        );
                        return;
                    }
                    Ok(a) => a,
                    Err(e) => {
                        alert::show_error(
                            &window,
                            &i18n("Script Test Failed"),
                            &format!("{}: {e}", i18n("Failed to parse command")),
                        );
                        return;
                    }
                };

                script_btn.set_sensitive(false);
                script_btn.set_label(&i18n("Testing…"));
                let btn_clone = script_btn.clone();
                let window_clone = window.clone();

                crate::utils::spawn_blocking_with_timeout(
                    move || {
                        let output = std::process::Command::new(&args[0])
                            .args(&args[1..])
                            .stdin(std::process::Stdio::null())
                            .output();
                        match output {
                            Ok(o) => (
                                o.status.success(),
                                String::from_utf8_lossy(&o.stdout).trim().to_string(),
                                String::from_utf8_lossy(&o.stderr).trim().to_string(),
                                o.status.code(),
                            ),
                            Err(e) => (false, String::new(), e.to_string(), None),
                        }
                    },
                    std::time::Duration::from_secs(30),
                    move |result: Option<(bool, String, String, Option<i32>)>| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_label(&i18n("Test Script"));

                        match result {
                            Some((true, stdout, _, _)) => {
                                let preview = if stdout.len() > 40 {
                                    format!("{}…", &stdout[..40])
                                } else if stdout.is_empty() {
                                    i18n("(empty output)")
                                } else {
                                    "●".repeat(stdout.len())
                                };
                                alert::show_success(
                                    &window_clone,
                                    &i18n("Script Test Successful"),
                                    &format!("{}: {preview}", i18n("Output")),
                                );
                            }
                            Some((false, _, stderr, code)) => {
                                let code_str =
                                    code.map(|c| format!(" (exit {c})")).unwrap_or_default();
                                let msg = if stderr.is_empty() {
                                    format!("{}{code_str}", i18n("Command failed"))
                                } else {
                                    let preview = if stderr.len() > 120 {
                                        format!("{}…", &stderr[..120])
                                    } else {
                                        stderr
                                    };
                                    format!("{preview}{code_str}")
                                };
                                alert::show_error(&window_clone, &i18n("Script Test Failed"), &msg);
                            }
                            None => {
                                alert::show_error(
                                    &window_clone,
                                    &i18n("Script Test Failed"),
                                    &i18n("Script timed out after 30 seconds"),
                                );
                            }
                        }
                    },
                );
            });
        }

        // Reverse sync: when SSH auth switches to Password(0) but password_source is None(4),
        // auto-switch password_source to Prompt(0)
        {
            let password_source_dropdown = result.password_source_dropdown.clone();
            let protocol_dropdown = result.protocol_dropdown.clone();
            let password_row = result.password_row.clone();
            let variable_row = result.variable_row.clone();
            result
                .ssh_auth_dropdown
                .connect_selected_notify(move |dropdown| {
                    if dropdown.selected() == 0
                        && protocol_dropdown.selected() == 0
                        && password_source_dropdown.selected() == 4
                    {
                        password_source_dropdown.set_selected(0);
                        // Update visibility to match Prompt(0)
                        password_row.set_visible(false);
                        variable_row.set_visible(false);
                    }
                });
        }

        result
    }

    /// Sets up inline validation for required fields
    fn setup_inline_validation_for(dialog: &Self) {
        // Name entry validation
        dialog.name_entry.connect_changed(move |entry| {
            let text = entry.text();
            if text.trim().is_empty() {
                entry.add_css_class(crate::validation::ERROR_CSS_CLASS);
            } else {
                entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
            }
        });

        // Host entry validation (only when not Zero Trust)
        let protocol_dropdown = dialog.protocol_dropdown.clone();
        dialog.host_entry.connect_changed(move |entry| {
            // Skip validation for Zero Trust (index 4)
            if protocol_dropdown.selected() == 4 {
                entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
                return;
            }

            let text = entry.text();
            let is_invalid = text.trim().is_empty() || text.contains(' ');
            if is_invalid {
                entry.add_css_class(crate::validation::ERROR_CSS_CLASS);
            } else {
                entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
            }
        });

        // Clear host validation when switching to Zero Trust
        let host_entry = dialog.host_entry.clone();
        dialog
            .protocol_dropdown
            .connect_notify_local(Some("selected"), move |dropdown, _| {
                if dropdown.selected() == 4 {
                    host_entry.remove_css_class(crate::validation::ERROR_CSS_CLASS);
                }
            });
    }

    /// Creates the main dialog with header bar containing Save button
    fn create_window_with_header(
        _parent: Option<&gtk4::Window>,
    ) -> (adw::Dialog, adw::HeaderBar, Button, Button) {
        let dialog = adw::Dialog::builder()
            .title(i18n("New Connection"))
            .content_width(600)
            .content_height(730)
            .build();
        // Set minimum size on the dialog widget to suppress AdwDialog warnings
        dialog.set_width_request(360);
        dialog.set_height_request(400);

        // Header bar with Test icon and Create icon button (GNOME HIG)
        let header = adw::HeaderBar::new();
        let test_btn = Button::from_icon_name("network-transmit-receive-symbolic");
        test_btn.set_tooltip_text(Some(&i18n("Test Connection")));
        test_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Test connection"))]);
        let save_btn = Button::from_icon_name("list-add-symbolic");
        save_btn.set_tooltip_text(Some(&i18n("Create")));
        save_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Create"))]);
        save_btn.add_css_class("suggested-action");
        header.pack_start(&test_btn);
        header.pack_start(&save_btn);

        (dialog, header, save_btn, test_btn)
    }

    /// Creates the view stack widget and adds it to the dialog with a bottom
    /// tab bar, following the GNOME HIG pattern for multi-page dialogs
    /// (similar to GNOME Settings / Preferences).
    fn create_view_stack(dialog: &adw::Dialog, header: &adw::HeaderBar) -> adw::ViewStack {
        let view_stack = adw::ViewStack::new();

        // Bottom tab bar — always visible (GNOME HIG for dialogs with many pages)
        let view_switcher_bar = adw::ViewSwitcherBar::builder()
            .stack(&view_stack)
            .reveal(true)
            .build();

        // Header bar shows the dialog title, no switcher
        header.set_title_widget(None::<&gtk4::Widget>);

        // Each tab provides its own ScrolledWindow, so the ViewStack sits
        // directly in the layout — no outer ScrolledWindow that would steal
        // height allocation from the per-tab scrollers.
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.set_width_request(360);
        main_box.set_height_request(400);
        main_box.append(header);
        view_stack.set_vexpand(true);
        main_box.append(&view_stack);
        main_box.append(&view_switcher_bar);
        dialog.set_child(Some(&main_box));

        view_stack
    }

    /// Creates the protocol stack and adds it to the view stack
    fn create_protocol_stack(view_stack: &adw::ViewStack) -> Stack {
        let protocol_stack = Stack::new();
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&protocol_stack)
            .build();
        view_stack
            .add_titled(&scrolled, Some("protocol"), &i18n("Protocol"))
            .set_icon_name(Some("network-server-symbolic"));
        protocol_stack
    }

    /// Connects the protocol dropdown to update the stack and port
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    fn connect_protocol_dropdown(
        dropdown: &DropDown,
        stack: &Stack,
        port_spin: &SpinButton,
        host_entry: &Entry,
        host_label: &Label,
        port_label: &Label,
        username_entry: &Entry,
        username_label: &Label,
        tags_entry: &Entry,
        tags_label: &Label,
        password_source_dropdown: &DropDown,
        password_source_label: &Label,
        password_row: &GtkBox,
        domain_entry: &Entry,
        domain_label: &Label,
        mosh_settings_group: &adw::PreferencesGroup,
    ) {
        let stack_clone = stack.clone();
        let port_clone = port_spin.clone();
        let host_entry = host_entry.clone();
        let host_label = host_label.clone();
        let port_label = port_label.clone();
        let username_entry = username_entry.clone();
        let username_label = username_label.clone();
        let tags_entry = tags_entry.clone();
        let tags_label = tags_label.clone();
        let password_source_dropdown = password_source_dropdown.clone();
        let password_source_label = password_source_label.clone();
        let password_row = password_row.clone();
        let domain_entry = domain_entry.clone();
        let domain_label = domain_label.clone();
        let mosh_group = mosh_settings_group.clone();

        dropdown.connect_selected_notify(move |dropdown| {
            let protocols = [
                "ssh",
                "rdp",
                "vnc",
                "spice",
                "zerotrust",
                "telnet",
                "serial",
                "sftp",
                "kubernetes",
                "mosh",
                "web",
            ];
            let selected = dropdown.selected() as usize;
            if selected < protocols.len() {
                let protocol_id = protocols[selected];
                // SFTP and MOSH reuse SSH config tab
                let stack_name = if protocol_id == "sftp" || protocol_id == "mosh" {
                    "ssh"
                } else {
                    protocol_id
                };
                stack_clone.set_visible_child_name(stack_name);
                let default_port = Self::get_default_port(protocol_id);
                if Self::is_default_port(port_clone.value()) {
                    port_clone.set_value(default_port);
                }

                let is_zerotrust = protocol_id == "zerotrust";
                let is_serial = protocol_id == "serial";
                let is_kubernetes = protocol_id == "kubernetes";
                let is_web = protocol_id == "web";
                let hide_network = is_zerotrust || is_serial || is_kubernetes;
                let visible = !hide_network;

                host_entry.set_visible(visible || is_web);
                host_label.set_visible(visible || is_web);
                port_clone.set_visible(visible && !is_web);
                port_label.set_visible(visible && !is_web);
                username_entry.set_visible(visible);
                username_label.set_visible(visible);

                // Update host field label and placeholder for Web protocol
                if is_web {
                    host_label.set_text(&crate::i18n::i18n("URL"));
                    host_entry
                        .set_placeholder_text(Some(&crate::i18n::i18n("https://example.com")));
                } else {
                    host_label.set_text(&crate::i18n::i18n("Host"));
                    host_entry.set_placeholder_text(Some(&crate::i18n::i18n("hostname or IP")));
                }
                tags_entry.set_visible(!is_zerotrust);
                tags_label.set_visible(!is_zerotrust);

                // Password source only relevant for protocols that use credentials:
                // SSH, SFTP, RDP, VNC, SPICE, Web. Hidden for Telnet, Serial, MOSH,
                // Kubernetes, Zero Trust — they don't use stored passwords.
                let uses_password = matches!(
                    protocol_id,
                    "ssh" | "sftp" | "rdp" | "vnc" | "spice" | "web"
                );
                password_source_dropdown.set_visible(uses_password);
                password_source_label.set_visible(uses_password);
                // Password row visibility controlled by password_source_dropdown
                if !uses_password {
                    password_row.set_visible(false);
                }

                // Domain only relevant for RDP (GEN-2)
                let is_rdp = protocol_id == "rdp";
                domain_entry.set_visible(is_rdp);
                domain_label.set_visible(is_rdp);

                // MOSH settings group visible only when MOSH is selected
                mosh_group.set_visible(protocol_id == "mosh");
            }
        });
    }

    /// Returns the default port for a protocol
    fn get_default_port(protocol_id: &str) -> f64 {
        match protocol_id {
            "rdp" => 3389.0,
            "vnc" | "spice" => 5900.0,
            "zerotrust" | "serial" | "kubernetes" => 0.0,
            "telnet" => 23.0,
            _ => 22.0, // ssh, sftp, mosh
        }
    }

    /// Checks if the port value is one of the default ports
    fn is_default_port(port: f64) -> bool {
        const EPSILON: f64 = 0.5;
        (port - 22.0).abs() < EPSILON
            || (port - 23.0).abs() < EPSILON
            || (port - 3389.0).abs() < EPSILON
            || (port - 5900.0).abs() < EPSILON
            || port.abs() < EPSILON
    }

    /// Connects the save button to validate and save the connection
    #[expect(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "long match dispatch with many flat parameters; restructuring would only move the parameter list elsewhere"
    )]
    fn connect_save_button(
        save_btn: &Button,
        dialog: &adw::Dialog,
        on_save: &super::ConnectionCallback,
        state: &crate::state::SharedAppState,
        editing_id: &Rc<RefCell<Option<Uuid>>>,
        name_entry: &Entry,
        icon_entry: &Entry,
        description_view: &TextView,
        host_entry: &Entry,
        port_spin: &SpinButton,
        username_entry: &Entry,
        domain_entry: &Entry,
        tags_entry: &Entry,
        protocol_dropdown: &DropDown,
        password_source_dropdown: &DropDown,
        password_entry: &Entry,
        variable_dropdown: &DropDown,
        group_dropdown: &DropDown,
        groups_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
        ssh_auth_dropdown: &DropDown,
        ssh_key_source_dropdown: &DropDown,
        ssh_key_entry: &Entry,
        ssh_agent_key_dropdown: &DropDown,
        ssh_agent_keys: &Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>>,
        ssh_jump_host_dropdown: &DropDown,
        ssh_proxy_entry: &Entry,
        ssh_proxy_command_entry: &Entry,
        ssh_identities_only: &CheckButton,
        ssh_control_master: &CheckButton,
        ssh_agent_forwarding: &CheckButton,
        ssh_waypipe: &CheckButton,
        ssh_x11_forwarding: &CheckButton,
        ssh_compression: &CheckButton,
        ssh_verbose: &CheckButton,
        ssh_startup_entry: &Entry,
        ssh_options_entry: &Entry,
        ssh_agent_socket_entry: &adw::EntryRow,
        ssh_keep_alive_interval: &adw::SpinRow,
        ssh_keep_alive_count_max: &adw::SpinRow,
        ssh_port_forwards: &Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
        rdp_client_mode_dropdown: &DropDown,
        rdp_performance_mode_dropdown: &DropDown,
        rdp_width_spin: &SpinButton,
        rdp_height_spin: &SpinButton,
        rdp_color_dropdown: &DropDown,
        rdp_scale_override_dropdown: &DropDown,
        rdp_audio_check: &adw::SwitchRow,
        rdp_gateway_entry: &Entry,
        rdp_gateway_port_spin: &SpinButton,
        rdp_gateway_username_entry: &Entry,
        rdp_disable_nla_check: &adw::SwitchRow,
        rdp_security_layer_dropdown: &DropDown,
        rdp_tls_security_level_spin: &SpinButton,
        rdp_ignore_certificate_check: &adw::SwitchRow,
        rdp_clipboard_check: &adw::SwitchRow,
        rdp_show_local_cursor_check: &adw::SwitchRow,
        rdp_jiggler_check: &adw::SwitchRow,
        rdp_jiggler_interval_spin: &SpinButton,
        rdp_autotype_delay_spin: &SpinButton,
        rdp_autotype_initial_delay_spin: &SpinButton,
        rdp_reconnect_on_resize_check: &adw::SwitchRow,
        rdp_jump_host_dropdown: &DropDown,
        rdp_connections_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
        rdp_shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
        rdp_custom_args_entry: &Entry,
        rdp_keyboard_layout_dropdown: &DropDown,
        rdp_remote_app_program_entry: &Entry,
        rdp_remote_app_args_entry: &Entry,
        rdp_remote_app_name_entry: &Entry,
        vnc_client_mode_dropdown: &DropDown,
        vnc_performance_mode_dropdown: &DropDown,
        vnc_encoding_dropdown: &DropDown,
        vnc_compression_spin: &SpinButton,
        vnc_quality_spin: &SpinButton,
        vnc_view_only_check: &adw::SwitchRow,
        vnc_scaling_check: &adw::SwitchRow,
        vnc_clipboard_check: &adw::SwitchRow,
        vnc_show_local_cursor_check: &adw::SwitchRow,
        vnc_scale_override_dropdown: &DropDown,
        vnc_custom_args_entry: &Entry,
        vnc_jump_host_dropdown: &DropDown,
        vnc_connections_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
        spice_tls_check: &adw::SwitchRow,
        spice_ca_cert_entry: &Entry,
        spice_skip_verify_check: &adw::SwitchRow,
        spice_usb_check: &adw::SwitchRow,
        spice_clipboard_check: &adw::SwitchRow,
        spice_show_local_cursor_check: &adw::SwitchRow,
        spice_compression_dropdown: &DropDown,
        spice_proxy_entry: &Entry,
        spice_shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
        spice_jump_host_dropdown: &DropDown,
        spice_connections_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
        zt_provider_dropdown: &DropDown,
        zt_aws_target_entry: &adw::EntryRow,
        zt_aws_profile_entry: &adw::EntryRow,
        zt_aws_region_entry: &adw::EntryRow,
        zt_gcp_instance_entry: &adw::EntryRow,
        zt_gcp_zone_entry: &adw::EntryRow,
        zt_gcp_project_entry: &adw::EntryRow,
        zt_azure_bastion_resource_id_entry: &adw::EntryRow,
        zt_azure_bastion_rg_entry: &adw::EntryRow,
        zt_azure_bastion_name_entry: &adw::EntryRow,
        zt_azure_ssh_vm_entry: &adw::EntryRow,
        zt_azure_ssh_rg_entry: &adw::EntryRow,
        zt_oci_bastion_id_entry: &adw::EntryRow,
        zt_oci_target_id_entry: &adw::EntryRow,
        zt_oci_target_ip_entry: &adw::EntryRow,
        zt_oci_ssh_key_entry: &adw::EntryRow,
        zt_oci_session_ttl_spin: &adw::SpinRow,
        zt_cf_hostname_entry: &adw::EntryRow,
        zt_teleport_host_entry: &adw::EntryRow,
        zt_teleport_cluster_entry: &adw::EntryRow,
        zt_tailscale_host_entry: &adw::EntryRow,
        zt_boundary_target_entry: &adw::EntryRow,
        zt_boundary_addr_entry: &adw::EntryRow,
        zt_hoop_connection_name_entry: &adw::EntryRow,
        zt_hoop_gateway_url_entry: &adw::EntryRow,
        zt_hoop_grpc_url_entry: &adw::EntryRow,
        zt_generic_command_entry: &adw::EntryRow,
        zt_custom_args_entry: &Entry,
        telnet_custom_args_entry: &Entry,
        telnet_backspace_dropdown: &DropDown,
        telnet_delete_dropdown: &DropDown,
        serial_device_entry: &Entry,
        serial_baud_dropdown: &DropDown,
        serial_data_bits_dropdown: &DropDown,
        serial_stop_bits_dropdown: &DropDown,
        serial_parity_dropdown: &DropDown,
        serial_flow_control_dropdown: &DropDown,
        serial_custom_args_entry: &Entry,
        k8s_kubeconfig_entry: &Entry,
        k8s_context_entry: &Entry,
        k8s_namespace_entry: &Entry,
        k8s_pod_entry: &Entry,
        k8s_container_entry: &Entry,
        k8s_shell_dropdown: &DropDown,
        k8s_busybox_check: &CheckButton,
        k8s_busybox_image_entry: &Entry,
        k8s_custom_args_entry: &Entry,
        mosh_port_range_entry: &Entry,
        mosh_predict_dropdown: &DropDown,
        mosh_server_binary_entry: &Entry,
        web_browser_entry: &Entry,
        web_private_mode_switch: &adw::SwitchRow,
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
        logging_tab: &logging_tab::LoggingTab,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
        pre_connect_enabled_switch: &adw::SwitchRow,
        pre_connect_command_entry: &Entry,
        pre_connect_timeout_spin: &SpinButton,
        pre_connect_abort_switch: &adw::SwitchRow,
        pre_connect_first_only_switch: &adw::SwitchRow,
        post_disconnect_enabled_switch: &adw::SwitchRow,
        post_disconnect_command_entry: &Entry,
        post_disconnect_timeout_spin: &SpinButton,
        post_disconnect_last_only_switch: &adw::SwitchRow,
        custom_properties: &Rc<RefCell<Vec<CustomProperty>>>,
        wol_enabled_check: &CheckButton,
        wol_mac_entry: &Entry,
        wol_broadcast_entry: &Entry,
        wol_port_spin: &SpinButton,
        wol_wait_spin: &SpinButton,
        theme_bg_button: &ColorDialogButton,
        theme_fg_button: &ColorDialogButton,
        theme_cursor_button: &ColorDialogButton,
        connections_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
        script_command_entry: &Entry,
        monitoring_toggle: &adw::SwitchRow,
        recording_toggle: &adw::SwitchRow,
        highlight_rules: &Rc<RefCell<Vec<HighlightRule>>>,
        activity_mode_combo: &adw::ComboRow,
        activity_quiet_period_spin: &adw::SpinRow,
        activity_silence_timeout_spin: &adw::SpinRow,
        retry_enabled_toggle: &adw::SwitchRow,
        retry_max_attempts_spin: &adw::SpinRow,
        retry_initial_delay_spin: &adw::SpinRow,
        retry_max_delay_spin: &adw::SpinRow,
        skip_port_check_toggle: &adw::SwitchRow,
    ) {
        let dialog = dialog.clone();
        let on_save = on_save.clone();
        let _state = state.clone();
        let name_entry = name_entry.clone();
        let icon_entry = icon_entry.clone();
        let description_view = description_view.clone();
        let host_entry = host_entry.clone();
        let port_spin = port_spin.clone();
        let username_entry = username_entry.clone();
        let domain_entry = domain_entry.clone();
        let tags_entry = tags_entry.clone();
        let protocol_dropdown = protocol_dropdown.clone();
        let password_source_dropdown = password_source_dropdown.clone();
        let password_entry = password_entry.clone();
        let variable_dropdown = variable_dropdown.clone();
        let group_dropdown = group_dropdown.clone();
        let groups_data = groups_data.clone();
        let ssh_auth_dropdown = ssh_auth_dropdown.clone();
        let ssh_key_source_dropdown = ssh_key_source_dropdown.clone();
        let ssh_key_entry = ssh_key_entry.clone();
        let ssh_agent_key_dropdown = ssh_agent_key_dropdown.clone();
        let ssh_agent_keys = ssh_agent_keys.clone();
        let ssh_jump_host_dropdown = ssh_jump_host_dropdown.clone();
        let ssh_proxy_entry = ssh_proxy_entry.clone();
        let ssh_proxy_command_entry = ssh_proxy_command_entry.clone();
        let ssh_identities_only = ssh_identities_only.clone();
        let ssh_control_master = ssh_control_master.clone();
        let ssh_agent_forwarding = ssh_agent_forwarding.clone();
        let ssh_waypipe = ssh_waypipe.clone();
        let ssh_x11_forwarding = ssh_x11_forwarding.clone();
        let ssh_compression = ssh_compression.clone();
        let ssh_verbose = ssh_verbose.clone();
        let ssh_startup_entry = ssh_startup_entry.clone();
        let ssh_options_entry = ssh_options_entry.clone();
        let ssh_agent_socket_entry = ssh_agent_socket_entry.clone();
        let ssh_keep_alive_interval = ssh_keep_alive_interval.clone();
        let ssh_keep_alive_count_max = ssh_keep_alive_count_max.clone();
        let ssh_port_forwards = ssh_port_forwards.clone();
        let rdp_client_mode_dropdown = rdp_client_mode_dropdown.clone();
        let rdp_width_spin = rdp_width_spin.clone();
        let rdp_height_spin = rdp_height_spin.clone();
        let rdp_color_dropdown = rdp_color_dropdown.clone();
        let rdp_scale_override_dropdown = rdp_scale_override_dropdown.clone();
        let rdp_audio_check = rdp_audio_check.clone();
        let rdp_gateway_entry = rdp_gateway_entry.clone();
        let rdp_gateway_port_spin = rdp_gateway_port_spin.clone();
        let rdp_gateway_username_entry = rdp_gateway_username_entry.clone();
        let rdp_disable_nla_check = rdp_disable_nla_check.clone();
        let rdp_security_layer_dropdown = rdp_security_layer_dropdown.clone();
        let rdp_tls_security_level_spin = rdp_tls_security_level_spin.clone();
        let rdp_ignore_certificate_check = rdp_ignore_certificate_check.clone();
        let rdp_clipboard_check = rdp_clipboard_check.clone();
        let rdp_show_local_cursor_check = rdp_show_local_cursor_check.clone();
        let rdp_jiggler_check = rdp_jiggler_check.clone();
        let rdp_jiggler_interval_spin = rdp_jiggler_interval_spin.clone();
        let rdp_autotype_delay_spin = rdp_autotype_delay_spin.clone();
        let rdp_autotype_initial_delay_spin = rdp_autotype_initial_delay_spin.clone();
        let rdp_reconnect_on_resize_check = rdp_reconnect_on_resize_check.clone();
        let rdp_jump_host_dropdown = rdp_jump_host_dropdown.clone();
        let rdp_connections_data = rdp_connections_data.clone();
        let rdp_shared_folders = rdp_shared_folders.clone();
        let rdp_custom_args_entry = rdp_custom_args_entry.clone();
        let rdp_keyboard_layout_dropdown = rdp_keyboard_layout_dropdown.clone();
        let rdp_remote_app_program_entry = rdp_remote_app_program_entry.clone();
        let rdp_remote_app_args_entry = rdp_remote_app_args_entry.clone();
        let rdp_remote_app_name_entry = rdp_remote_app_name_entry.clone();
        let rdp_performance_mode_dropdown = rdp_performance_mode_dropdown.clone();
        let vnc_client_mode_dropdown = vnc_client_mode_dropdown.clone();
        let vnc_encoding_dropdown = vnc_encoding_dropdown.clone();
        let vnc_compression_spin = vnc_compression_spin.clone();
        let vnc_quality_spin = vnc_quality_spin.clone();
        let vnc_view_only_check = vnc_view_only_check.clone();
        let vnc_scaling_check = vnc_scaling_check.clone();
        let vnc_clipboard_check = vnc_clipboard_check.clone();
        let vnc_show_local_cursor_check = vnc_show_local_cursor_check.clone();
        let vnc_scale_override_dropdown = vnc_scale_override_dropdown.clone();
        let vnc_custom_args_entry = vnc_custom_args_entry.clone();
        let vnc_performance_mode_dropdown = vnc_performance_mode_dropdown.clone();
        let vnc_jump_host_dropdown = vnc_jump_host_dropdown.clone();
        let vnc_connections_data = vnc_connections_data.clone();
        let spice_tls_check = spice_tls_check.clone();
        let spice_ca_cert_entry = spice_ca_cert_entry.clone();
        let spice_skip_verify_check = spice_skip_verify_check.clone();
        let spice_usb_check = spice_usb_check.clone();
        let spice_clipboard_check = spice_clipboard_check.clone();
        let spice_show_local_cursor_check = spice_show_local_cursor_check.clone();
        let spice_compression_dropdown = spice_compression_dropdown.clone();
        let spice_proxy_entry = spice_proxy_entry.clone();
        let spice_shared_folders = spice_shared_folders.clone();
        let spice_jump_host_dropdown = spice_jump_host_dropdown.clone();
        let spice_connections_data = spice_connections_data.clone();
        let zt_provider_dropdown = zt_provider_dropdown.clone();
        let zt_aws_target_entry = zt_aws_target_entry.clone();
        let zt_aws_profile_entry = zt_aws_profile_entry.clone();
        let zt_aws_region_entry = zt_aws_region_entry.clone();
        let zt_gcp_instance_entry = zt_gcp_instance_entry.clone();
        let zt_gcp_zone_entry = zt_gcp_zone_entry.clone();
        let zt_gcp_project_entry = zt_gcp_project_entry.clone();
        let zt_azure_bastion_resource_id_entry = zt_azure_bastion_resource_id_entry.clone();
        let zt_azure_bastion_rg_entry = zt_azure_bastion_rg_entry.clone();
        let zt_azure_bastion_name_entry = zt_azure_bastion_name_entry.clone();
        let zt_azure_ssh_vm_entry = zt_azure_ssh_vm_entry.clone();
        let zt_azure_ssh_rg_entry = zt_azure_ssh_rg_entry.clone();
        let zt_oci_bastion_id_entry = zt_oci_bastion_id_entry.clone();
        let zt_oci_target_id_entry = zt_oci_target_id_entry.clone();
        let zt_oci_target_ip_entry = zt_oci_target_ip_entry.clone();
        let zt_oci_ssh_key_entry = zt_oci_ssh_key_entry.clone();
        let zt_oci_session_ttl_spin = zt_oci_session_ttl_spin.clone();
        let zt_cf_hostname_entry = zt_cf_hostname_entry.clone();
        let zt_teleport_host_entry = zt_teleport_host_entry.clone();
        let zt_teleport_cluster_entry = zt_teleport_cluster_entry.clone();
        let zt_tailscale_host_entry = zt_tailscale_host_entry.clone();
        let zt_boundary_target_entry = zt_boundary_target_entry.clone();
        let zt_boundary_addr_entry = zt_boundary_addr_entry.clone();
        let zt_hoop_connection_name_entry = zt_hoop_connection_name_entry.clone();
        let zt_hoop_gateway_url_entry = zt_hoop_gateway_url_entry.clone();
        let zt_hoop_grpc_url_entry = zt_hoop_grpc_url_entry.clone();
        let zt_generic_command_entry = zt_generic_command_entry.clone();
        let zt_custom_args_entry = zt_custom_args_entry.clone();
        let telnet_custom_args_entry = telnet_custom_args_entry.clone();
        let telnet_backspace_dropdown = telnet_backspace_dropdown.clone();
        let telnet_delete_dropdown = telnet_delete_dropdown.clone();
        let serial_device_entry = serial_device_entry.clone();
        let serial_baud_dropdown = serial_baud_dropdown.clone();
        let serial_data_bits_dropdown = serial_data_bits_dropdown.clone();
        let serial_stop_bits_dropdown = serial_stop_bits_dropdown.clone();
        let serial_parity_dropdown = serial_parity_dropdown.clone();
        let serial_flow_control_dropdown = serial_flow_control_dropdown.clone();
        let serial_custom_args_entry = serial_custom_args_entry.clone();
        let k8s_kubeconfig_entry = k8s_kubeconfig_entry.clone();
        let k8s_context_entry = k8s_context_entry.clone();
        let k8s_namespace_entry = k8s_namespace_entry.clone();
        let k8s_pod_entry = k8s_pod_entry.clone();
        let k8s_container_entry = k8s_container_entry.clone();
        let k8s_shell_dropdown = k8s_shell_dropdown.clone();
        let k8s_busybox_check = k8s_busybox_check.clone();
        let k8s_busybox_image_entry = k8s_busybox_image_entry.clone();
        let k8s_custom_args_entry = k8s_custom_args_entry.clone();
        let mosh_port_range_entry = mosh_port_range_entry.clone();
        let mosh_predict_dropdown = mosh_predict_dropdown.clone();
        let mosh_server_binary_entry = mosh_server_binary_entry.clone();
        let web_browser_entry = web_browser_entry.clone();
        let web_private_mode_switch = web_private_mode_switch.clone();
        let variables_rows = variables_rows.clone();
        let logging_enabled_switch = logging_tab.enabled_switch.clone();
        let logging_path_entry = logging_tab.path_entry.clone();
        let logging_timestamp_dropdown = logging_tab.timestamp_dropdown.clone();
        let logging_max_size_spin = logging_tab.max_size_spin.clone();
        let logging_retention_spin = logging_tab.retention_spin.clone();
        let logging_activity_switch = logging_tab.log_activity_switch.clone();
        let logging_input_switch = logging_tab.log_input_switch.clone();
        let logging_output_switch = logging_tab.log_output_switch.clone();
        let logging_timestamps_switch = logging_tab.log_timestamps_switch.clone();
        let expect_rules = expect_rules.clone();
        let pre_connect_enabled_switch = pre_connect_enabled_switch.clone();
        let pre_connect_command_entry = pre_connect_command_entry.clone();
        let pre_connect_timeout_spin = pre_connect_timeout_spin.clone();
        let pre_connect_abort_switch = pre_connect_abort_switch.clone();
        let pre_connect_first_only_switch = pre_connect_first_only_switch.clone();
        let post_disconnect_enabled_switch = post_disconnect_enabled_switch.clone();
        let post_disconnect_command_entry = post_disconnect_command_entry.clone();
        let post_disconnect_timeout_spin = post_disconnect_timeout_spin.clone();
        let post_disconnect_last_only_switch = post_disconnect_last_only_switch.clone();
        let custom_properties = custom_properties.clone();
        let wol_enabled_check = wol_enabled_check.clone();
        let wol_mac_entry = wol_mac_entry.clone();
        let wol_broadcast_entry = wol_broadcast_entry.clone();
        let wol_port_spin = wol_port_spin.clone();
        let wol_wait_spin = wol_wait_spin.clone();
        let theme_bg_button = theme_bg_button.clone();
        let theme_fg_button = theme_fg_button.clone();
        let theme_cursor_button = theme_cursor_button.clone();
        let editing_id = editing_id.clone();
        let connections_data = connections_data.clone();
        let script_command_entry = script_command_entry.clone();
        let monitoring_toggle = monitoring_toggle.clone();
        let recording_toggle = recording_toggle.clone();
        let highlight_rules = highlight_rules.clone();
        let activity_mode_combo = activity_mode_combo.clone();
        let activity_quiet_period_spin = activity_quiet_period_spin.clone();
        let activity_silence_timeout_spin = activity_silence_timeout_spin.clone();
        let retry_enabled_toggle = retry_enabled_toggle.clone();
        let retry_max_attempts_spin = retry_max_attempts_spin.clone();
        let retry_initial_delay_spin = retry_initial_delay_spin.clone();
        let retry_max_delay_spin = retry_max_delay_spin.clone();
        let skip_port_check_toggle = skip_port_check_toggle.clone();

        save_btn.connect_clicked(move |_| {
            let local_variables = Self::collect_local_variables(&variables_rows);
            let collected_expect_rules = expect_rules.borrow().clone();
            let collected_custom_properties = custom_properties.borrow().clone();
            let collected_highlight_rules = highlight_rules.borrow().clone();
            let data = ConnectionDialogData {
                name_entry: &name_entry,
                icon_entry: &icon_entry,
                description_view: &description_view,
                host_entry: &host_entry,
                port_spin: &port_spin,
                username_entry: &username_entry,
                domain_entry: &domain_entry,
                tags_entry: &tags_entry,
                protocol_dropdown: &protocol_dropdown,
                password_source_dropdown: &password_source_dropdown,
                password_entry: &password_entry,
                variable_dropdown: &variable_dropdown,
                group_dropdown: &group_dropdown,
                groups_data: &groups_data,
                connections_data: &connections_data,
                ssh_auth_dropdown: &ssh_auth_dropdown,
                ssh_key_source_dropdown: &ssh_key_source_dropdown,
                ssh_key_entry: &ssh_key_entry,
                ssh_agent_key_dropdown: &ssh_agent_key_dropdown,
                ssh_agent_keys: &ssh_agent_keys,
                ssh_jump_host_dropdown: &ssh_jump_host_dropdown,
                ssh_proxy_entry: &ssh_proxy_entry,
                ssh_proxy_command_entry: &ssh_proxy_command_entry,
                ssh_identities_only: &ssh_identities_only,
                ssh_control_master: &ssh_control_master,
                ssh_agent_forwarding: &ssh_agent_forwarding,
                ssh_waypipe: &ssh_waypipe,
                ssh_x11_forwarding: &ssh_x11_forwarding,
                ssh_compression: &ssh_compression,
                ssh_verbose: &ssh_verbose,
                ssh_startup_entry: &ssh_startup_entry,
                ssh_options_entry: &ssh_options_entry,
                ssh_agent_socket_entry: &ssh_agent_socket_entry,
                ssh_keep_alive_interval: &ssh_keep_alive_interval,
                ssh_keep_alive_count_max: &ssh_keep_alive_count_max,
                ssh_port_forwards: &ssh_port_forwards,
                rdp_client_mode_dropdown: &rdp_client_mode_dropdown,
                rdp_width_spin: &rdp_width_spin,
                rdp_height_spin: &rdp_height_spin,
                rdp_color_dropdown: &rdp_color_dropdown,
                rdp_scale_override_dropdown: &rdp_scale_override_dropdown,
                rdp_audio_check: &rdp_audio_check,
                rdp_gateway_entry: &rdp_gateway_entry,
                rdp_gateway_port_spin: &rdp_gateway_port_spin,
                rdp_gateway_username_entry: &rdp_gateway_username_entry,
                rdp_disable_nla_check: &rdp_disable_nla_check,
                rdp_security_layer_dropdown: &rdp_security_layer_dropdown,
                rdp_tls_security_level_spin: &rdp_tls_security_level_spin,
                rdp_ignore_certificate_check: &rdp_ignore_certificate_check,
                rdp_clipboard_check: &rdp_clipboard_check,
                rdp_show_local_cursor_check: &rdp_show_local_cursor_check,
                rdp_jiggler_check: &rdp_jiggler_check,
                rdp_jiggler_interval_spin: &rdp_jiggler_interval_spin,
                rdp_autotype_delay_spin: &rdp_autotype_delay_spin,
                rdp_autotype_initial_delay_spin: &rdp_autotype_initial_delay_spin,
                rdp_reconnect_on_resize_check: &rdp_reconnect_on_resize_check,
                rdp_jump_host_dropdown: &rdp_jump_host_dropdown,
                rdp_connections_data: &rdp_connections_data,
                rdp_shared_folders: &rdp_shared_folders,
                rdp_custom_args_entry: &rdp_custom_args_entry,
                rdp_keyboard_layout_dropdown: &rdp_keyboard_layout_dropdown,
                rdp_remote_app_program_entry: &rdp_remote_app_program_entry,
                rdp_remote_app_args_entry: &rdp_remote_app_args_entry,
                rdp_remote_app_name_entry: &rdp_remote_app_name_entry,
                vnc_client_mode_dropdown: &vnc_client_mode_dropdown,
                vnc_encoding_dropdown: &vnc_encoding_dropdown,
                vnc_compression_spin: &vnc_compression_spin,
                vnc_quality_spin: &vnc_quality_spin,
                vnc_view_only_check: &vnc_view_only_check,
                vnc_scaling_check: &vnc_scaling_check,
                vnc_clipboard_check: &vnc_clipboard_check,
                vnc_show_local_cursor_check: &vnc_show_local_cursor_check,
                vnc_scale_override_dropdown: &vnc_scale_override_dropdown,
                vnc_custom_args_entry: &vnc_custom_args_entry,
                vnc_jump_host_dropdown: &vnc_jump_host_dropdown,
                vnc_connections_data: &vnc_connections_data,
                spice_tls_check: &spice_tls_check,
                spice_ca_cert_entry: &spice_ca_cert_entry,
                spice_skip_verify_check: &spice_skip_verify_check,
                spice_usb_check: &spice_usb_check,
                spice_clipboard_check: &spice_clipboard_check,
                spice_show_local_cursor_check: &spice_show_local_cursor_check,
                spice_compression_dropdown: &spice_compression_dropdown,
                spice_proxy_entry: &spice_proxy_entry,
                spice_shared_folders: &spice_shared_folders,
                spice_jump_host_dropdown: &spice_jump_host_dropdown,
                spice_connections_data: &spice_connections_data,
                zt_provider_dropdown: &zt_provider_dropdown,
                zt_aws_target_entry: &zt_aws_target_entry,
                zt_aws_profile_entry: &zt_aws_profile_entry,
                zt_aws_region_entry: &zt_aws_region_entry,
                zt_gcp_instance_entry: &zt_gcp_instance_entry,
                zt_gcp_zone_entry: &zt_gcp_zone_entry,
                zt_gcp_project_entry: &zt_gcp_project_entry,
                zt_azure_bastion_resource_id_entry: &zt_azure_bastion_resource_id_entry,
                zt_azure_bastion_rg_entry: &zt_azure_bastion_rg_entry,
                zt_azure_bastion_name_entry: &zt_azure_bastion_name_entry,
                zt_azure_ssh_vm_entry: &zt_azure_ssh_vm_entry,
                zt_azure_ssh_rg_entry: &zt_azure_ssh_rg_entry,
                zt_oci_bastion_id_entry: &zt_oci_bastion_id_entry,
                zt_oci_target_id_entry: &zt_oci_target_id_entry,
                zt_oci_target_ip_entry: &zt_oci_target_ip_entry,
                zt_oci_ssh_key_entry: &zt_oci_ssh_key_entry,
                zt_oci_session_ttl_spin: &zt_oci_session_ttl_spin,
                zt_cf_hostname_entry: &zt_cf_hostname_entry,
                zt_teleport_host_entry: &zt_teleport_host_entry,
                zt_teleport_cluster_entry: &zt_teleport_cluster_entry,
                zt_tailscale_host_entry: &zt_tailscale_host_entry,
                zt_boundary_target_entry: &zt_boundary_target_entry,
                zt_boundary_addr_entry: &zt_boundary_addr_entry,
                zt_hoop_connection_name_entry: &zt_hoop_connection_name_entry,
                zt_hoop_gateway_url_entry: &zt_hoop_gateway_url_entry,
                zt_hoop_grpc_url_entry: &zt_hoop_grpc_url_entry,
                zt_generic_command_entry: &zt_generic_command_entry,
                zt_custom_args_entry: &zt_custom_args_entry,
                telnet_custom_args_entry: &telnet_custom_args_entry,
                telnet_backspace_dropdown: &telnet_backspace_dropdown,
                telnet_delete_dropdown: &telnet_delete_dropdown,
                serial_device_entry: &serial_device_entry,
                serial_baud_dropdown: &serial_baud_dropdown,
                serial_data_bits_dropdown: &serial_data_bits_dropdown,
                serial_stop_bits_dropdown: &serial_stop_bits_dropdown,
                serial_parity_dropdown: &serial_parity_dropdown,
                serial_flow_control_dropdown: &serial_flow_control_dropdown,
                serial_custom_args_entry: &serial_custom_args_entry,
                k8s_kubeconfig_entry: &k8s_kubeconfig_entry,
                k8s_context_entry: &k8s_context_entry,
                k8s_namespace_entry: &k8s_namespace_entry,
                k8s_pod_entry: &k8s_pod_entry,
                k8s_container_entry: &k8s_container_entry,
                k8s_shell_dropdown: &k8s_shell_dropdown,
                k8s_busybox_check: &k8s_busybox_check,
                k8s_busybox_image_entry: &k8s_busybox_image_entry,
                k8s_custom_args_entry: &k8s_custom_args_entry,
                mosh_port_range_entry: &mosh_port_range_entry,
                mosh_predict_dropdown: &mosh_predict_dropdown,
                mosh_server_binary_entry: &mosh_server_binary_entry,
                web_browser_entry: &web_browser_entry,
                web_private_mode_switch: &web_private_mode_switch,
                local_variables: &local_variables,
                logging_tab: &logging_tab::LoggingTab {
                    enabled_switch: logging_enabled_switch.clone(),
                    path_entry: logging_path_entry.clone(),
                    timestamp_dropdown: logging_timestamp_dropdown.clone(),
                    max_size_spin: logging_max_size_spin.clone(),
                    retention_spin: logging_retention_spin.clone(),
                    log_activity_switch: logging_activity_switch.clone(),
                    log_input_switch: logging_input_switch.clone(),
                    log_output_switch: logging_output_switch.clone(),
                    log_timestamps_switch: logging_timestamps_switch.clone(),
                },
                expect_rules: &collected_expect_rules,
                pre_connect_enabled_switch: &pre_connect_enabled_switch,
                pre_connect_command_entry: &pre_connect_command_entry,
                pre_connect_timeout_spin: &pre_connect_timeout_spin,
                pre_connect_abort_switch: &pre_connect_abort_switch,
                pre_connect_first_only_switch: &pre_connect_first_only_switch,
                post_disconnect_enabled_switch: &post_disconnect_enabled_switch,
                post_disconnect_command_entry: &post_disconnect_command_entry,
                post_disconnect_timeout_spin: &post_disconnect_timeout_spin,
                post_disconnect_last_only_switch: &post_disconnect_last_only_switch,
                custom_properties: &collected_custom_properties,
                wol_enabled_check: &wol_enabled_check,
                wol_mac_entry: &wol_mac_entry,
                wol_broadcast_entry: &wol_broadcast_entry,
                wol_port_spin: &wol_port_spin,
                wol_wait_spin: &wol_wait_spin,
                theme_bg_button: &theme_bg_button,
                theme_fg_button: &theme_fg_button,
                theme_cursor_button: &theme_cursor_button,
                rdp_performance_mode_dropdown: &rdp_performance_mode_dropdown,
                vnc_performance_mode_dropdown: &vnc_performance_mode_dropdown,
                editing_id: &editing_id,
                script_command_entry: &script_command_entry,
                monitoring_toggle: &monitoring_toggle,
                recording_toggle: &recording_toggle,
                highlight_rules: &collected_highlight_rules,
                activity_mode_combo: &activity_mode_combo,
                activity_quiet_period_spin: &activity_quiet_period_spin,
                activity_silence_timeout_spin: &activity_silence_timeout_spin,
                retry_enabled_toggle: &retry_enabled_toggle,
                retry_max_attempts_spin: &retry_max_attempts_spin,
                retry_initial_delay_spin: &retry_initial_delay_spin,
                retry_max_delay_spin: &retry_max_delay_spin,
                skip_port_check_toggle: &skip_port_check_toggle,
            };

            if let Err(err) = data.validate() {
                alert::show_error(&dialog, &i18n("Validation Error"), &err);
                return;
            }

            if let Some(result) = data.build_connection() {
                // Password saving is handled by the caller (edit_dialogs,
                // connection_dialogs, templates) after the on_save callback
                // to avoid duplicate vault writes.

                if let Some(ref cb) = *on_save.borrow() {
                    cb(Some(result));
                }
                dialog.close();
            }
        });
    }

    /// Creates a custom property row widget
    fn create_custom_property_row(property: Option<&CustomProperty>) -> CustomPropertyRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(12);
        main_box.set_margin_bottom(12);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Row 0: Name and delete button
        let name_label = Label::builder()
            .label(i18n("Name:"))
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Property name (e.g., asset_tag, docs_url)"))
            .build();

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(i18n("Delete property"))
            .build();
        delete_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Delete property"))]);

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Type dropdown
        let type_label = Label::builder()
            .label(i18n("Type:"))
            .halign(gtk4::Align::End)
            .build();
        let type_list = StringList::new(&[&i18n("Text"), &i18n("URL"), &i18n("Protected")]);
        let type_dropdown = DropDown::new(Some(type_list), gtk4::Expression::NONE);
        type_dropdown.set_selected(0);
        type_dropdown.set_tooltip_text(Some(&i18n(
            "Text: Plain text\nURL: Clickable link\nProtected: Masked/encrypted value",
        )));

        grid.attach(&type_label, 0, 1, 1, 1);
        grid.attach(&type_dropdown, 1, 1, 2, 1);

        // Row 2: Value (regular entry)
        let value_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Property value"))
            .build();

        // Row 2: Value (password entry for protected type)
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Protected value (masked)"))
            .show_peek_icon(true)
            .build();

        grid.attach(&value_label, 0, 2, 1, 1);
        grid.attach(&value_entry, 1, 2, 2, 1);
        // Secret entry is hidden initially, will be shown when type is Protected
        grid.attach(&secret_entry, 1, 3, 2, 1);
        secret_entry.set_visible(false);

        // Connect type dropdown to show/hide appropriate value entry
        let value_entry_clone = value_entry.clone();
        let secret_entry_clone = secret_entry.clone();
        type_dropdown.connect_selected_notify(move |dropdown| {
            let is_protected = dropdown.selected() == 2; // Protected is index 2
            value_entry_clone.set_visible(!is_protected);
            secret_entry_clone.set_visible(is_protected);
        });

        main_box.append(&grid);

        // Populate from existing property if provided
        if let Some(prop) = property {
            name_entry.set_text(&prop.name);
            let type_idx = match prop.property_type {
                PropertyType::Text => 0,
                PropertyType::Url => 1,
                PropertyType::Protected => 2,
            };
            type_dropdown.set_selected(type_idx);

            if prop.is_protected() {
                secret_entry.set_text(&prop.value);
                value_entry.set_visible(false);
                secret_entry.set_visible(true);
            } else {
                value_entry.set_text(&prop.value);
            }
        }

        let row = ListBoxRow::builder().child(&main_box).build();

        CustomPropertyRow {
            row,
            name_entry,
            type_dropdown,
            value_entry,
            secret_entry,
            delete_button,
        }
    }

    /// Wires up the add custom property button
    fn wire_add_custom_property_button(
        add_button: &Button,
        properties_list: &ListBox,
        custom_properties: &Rc<RefCell<Vec<CustomProperty>>>,
    ) {
        let list_clone = properties_list.clone();
        let props_clone = custom_properties.clone();

        add_button.connect_clicked(move |_| {
            let prop_row = Self::create_custom_property_row(None);

            // Add a new empty property to the list
            let new_prop = CustomProperty::new_text("", "");
            props_clone.borrow_mut().push(new_prop);
            let prop_index = props_clone.borrow().len() - 1;

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let props_for_delete = props_clone.clone();
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
            Self::connect_custom_property_changes(&prop_row, &props_clone, prop_index);

            list_clone.append(&prop_row.row);
        });
    }

    /// Connects entry changes to update the custom property in the list
    fn connect_custom_property_changes(
        prop_row: &CustomPropertyRow,
        custom_properties: &Rc<RefCell<Vec<CustomProperty>>>,
        initial_index: usize,
    ) {
        // We need to track the row to find its current index
        let row_widget = prop_row.row.clone();

        // Name entry
        let props_for_name = custom_properties.clone();
        let row_for_name = row_widget.clone();
        prop_row.name_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_name.index())
                && let Some(prop) = props_for_name.borrow_mut().get_mut(idx)
            {
                prop.name = text;
            }
        });

        // Type dropdown
        let props_for_type = custom_properties.clone();
        let row_for_type = row_widget.clone();
        prop_row
            .type_dropdown
            .connect_selected_notify(move |dropdown| {
                let prop_type = match dropdown.selected() {
                    1 => PropertyType::Url,
                    2 => PropertyType::Protected,
                    _ => PropertyType::Text,
                };
                if let Ok(idx) = usize::try_from(row_for_type.index())
                    && let Some(prop) = props_for_type.borrow_mut().get_mut(idx)
                {
                    prop.property_type = prop_type;
                }
            });

        // Value entry (for Text and URL types)
        let props_for_value = custom_properties.clone();
        let row_for_value = row_widget.clone();
        prop_row.value_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_value.index())
                && let Some(prop) = props_for_value.borrow_mut().get_mut(idx)
                && !prop.is_protected()
            {
                prop.value = text;
            }
        });

        // Secret entry (for Protected type)
        let props_for_secret = custom_properties.clone();
        let row_for_secret = row_widget;
        prop_row.secret_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_secret.index())
                && let Some(prop) = props_for_secret.borrow_mut().get_mut(idx)
                && prop.is_protected()
            {
                prop.value = text;
            }
        });

        // Suppress unused variable warning
        let _ = initial_index;
    }

    /// Creates an expect rule row widget
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    fn create_expect_rule_row(rule: Option<&ExpectRule>) -> ExpectRuleRow {
        let main_box = GtkBox::new(Orientation::Vertical, 6);
        main_box.set_margin_top(12);
        main_box.set_margin_bottom(12);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        // Row 0: Action buttons (delete, move up/down) — top-right for visibility
        let action_box = GtkBox::new(Orientation::Horizontal, 4);
        action_box.set_halign(gtk4::Align::End);

        let move_up_button = Button::builder()
            .icon_name("go-up-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Move up (higher priority)"))
            .build();
        move_up_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Move rule up"))]);
        let move_down_button = Button::builder()
            .icon_name("go-down-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Move down (lower priority)"))
            .build();
        move_down_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Move rule down"))]);
        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Delete rule"))
            .build();
        delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete rule"))]);
        action_box.append(&move_up_button);
        action_box.append(&move_down_button);
        action_box.append(&delete_button);
        main_box.append(&action_box);

        // Row 1: Pattern entry (full width)
        let pattern_box = GtkBox::new(Orientation::Horizontal, 6);
        let pattern_label = Label::builder()
            .label(i18n("Pattern:"))
            .halign(gtk4::Align::End)
            .width_chars(10)
            .build();
        let pattern_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Regex pattern (e.g., password:\\s*$)"))
            .tooltip_text(i18n("Regular expression to match against terminal output"))
            .build();
        pattern_box.append(&pattern_label);
        pattern_box.append(&pattern_entry);
        main_box.append(&pattern_box);

        // Row 2: Response entry + "Insert Variable" button
        let response_box = GtkBox::new(Orientation::Horizontal, 6);
        let response_label = Label::builder()
            .label(i18n("Response:"))
            .halign(gtk4::Align::End)
            .width_chars(10)
            .build();
        let response_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Text to send (e.g., ${password}\\n)"))
            .tooltip_text(i18n(
                "Response to send when pattern matches. Use ${password}, ${username}, or ${VAR_NAME} for variables.",
            ))
            .build();

        // "Insert Variable" button with popover
        let var_menu_button = gtk4::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Insert a variable placeholder"))
            .build();
        var_menu_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Insert variable"))]);

        let var_popover = gtk4::Popover::new();
        var_popover.set_size_request(220, -1);
        let var_list = GtkBox::new(Orientation::Vertical, 2);
        var_list.set_margin_top(6);
        var_list.set_margin_bottom(6);
        var_list.set_margin_start(6);
        var_list.set_margin_end(6);

        let builtin_header = Label::builder()
            .label(i18n("Built-in"))
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "caption"])
            .build();
        var_list.append(&builtin_header);

        for (var_name, var_desc) in [
            ("${password}", i18n("Connection password")),
            ("${username}", i18n("Connection username")),
            ("${host}", i18n("Connection host")),
            ("${port}", i18n("Connection port")),
        ] {
            let btn = Button::builder()
                .label(var_name)
                .css_classes(["flat"])
                .tooltip_text(&var_desc)
                .build();
            let entry_clone = response_entry.clone();
            let var = var_name.to_string();
            btn.connect_clicked(move |btn| {
                let pos = entry_clone.position();
                entry_clone.insert_text(&var, &mut pos.clone());
                #[expect(
    clippy::cast_possible_wrap,
    reason = "value range fits the target signed type by construction in this code path"
)]
                entry_clone.set_position(pos + var.len() as i32);
                if let Some(popover) = btn
                    .ancestor(gtk4::Popover::static_type())
                    .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                {
                    popover.popdown();
                }
            });
            var_list.append(&btn);
        }

        let special_header = Label::builder()
            .label(i18n("Special"))
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label", "caption"])
            .margin_top(4)
            .build();
        var_list.append(&special_header);

        let newline_btn = Button::builder()
            .label("\\n")
            .css_classes(["flat"])
            .tooltip_text(i18n("Newline (Enter key)"))
            .build();
        {
            let entry_clone = response_entry.clone();
            newline_btn.connect_clicked(move |btn| {
                let pos = entry_clone.position();
                entry_clone.insert_text("\\n", &mut pos.clone());
                #[allow(
    clippy::cast_possible_wrap,
    reason = "value range fits the target signed type by construction in this code path"
)]
                entry_clone.set_position(pos + 2);
                if let Some(popover) = btn
                    .ancestor(gtk4::Popover::static_type())
                    .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                {
                    popover.popdown();
                }
            });
        }
        var_list.append(&newline_btn);

        var_popover.set_child(Some(&var_list));
        var_menu_button.set_popover(Some(&var_popover));

        response_box.append(&response_label);
        response_box.append(&response_entry);
        response_box.append(&var_menu_button);
        main_box.append(&response_box);

        // Row 3: Priority, Timeout, Enabled, One-shot — compact horizontal row
        let settings_box = GtkBox::new(Orientation::Horizontal, 8);
        settings_box.set_halign(gtk4::Align::Start);

        let priority_label = Label::builder()
            .label(i18n("Priority:"))
            .css_classes(["dim-label", "caption"])
            .build();
        let priority_adj = gtk4::Adjustment::new(0.0, -1000.0, 1000.0, 1.0, 10.0, 0.0);
        let priority_spin = SpinButton::builder()
            .adjustment(&priority_adj)
            .climb_rate(1.0)
            .digits(0)
            .width_chars(5)
            .tooltip_text(i18n("Higher priority rules are checked first"))
            .build();

        let timeout_label = Label::builder()
            .label(i18n("Timeout:"))
            .css_classes(["dim-label", "caption"])
            .build();
        let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 60000.0, 100.0, 1000.0, 0.0);
        let timeout_spin = SpinButton::builder()
            .adjustment(&timeout_adj)
            .climb_rate(1.0)
            .digits(0)
            .width_chars(6)
            .tooltip_text(i18n("Timeout in milliseconds (0 = no timeout)"))
            .build();

        let enabled_check = CheckButton::builder()
            .label(i18n("Enabled"))
            .active(true)
            .build();

        let one_shot_check = CheckButton::builder()
            .label(i18n("One-shot"))
            .active(true)
            .tooltip_text(i18n("Fire only once, then remove the rule"))
            .build();

        settings_box.append(&priority_label);
        settings_box.append(&priority_spin);
        settings_box.append(&timeout_label);
        settings_box.append(&timeout_spin);
        settings_box.append(&enabled_check);
        settings_box.append(&one_shot_check);
        main_box.append(&settings_box);

        // Row 4: Regex validation label
        let validation_label = Label::builder()
            .halign(gtk4::Align::Start)
            .css_classes(["error"])
            .visible(false)
            .build();
        main_box.append(&validation_label);

        // Wire regex validation on pattern entry
        let validation_label_clone = validation_label.clone();
        pattern_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if text.is_empty() {
                validation_label_clone.set_visible(false);
                entry.remove_css_class("error");
            } else {
                match regex::Regex::new(&text) {
                    Ok(_) => {
                        validation_label_clone.set_visible(false);
                        entry.remove_css_class("error");
                    }
                    Err(e) => {
                        validation_label_clone.set_text(&e.to_string());
                        validation_label_clone.set_visible(true);
                        entry.add_css_class("error");
                    }
                }
            }
        });

        // Populate from existing rule if provided
        let id = rule.map_or_else(Uuid::new_v4, |r| {
            pattern_entry.set_text(&r.pattern);
            response_entry.set_text(&r.response);
            priority_spin.set_value(f64::from(r.priority));
            timeout_spin.set_value(f64::from(r.timeout_ms.unwrap_or(0)));
            enabled_check.set_active(r.enabled);
            one_shot_check.set_active(r.one_shot);
            r.id
        });

        let row = ListBoxRow::builder().child(&main_box).build();

        ExpectRuleRow {
            row,
            id,
            pattern_entry,
            response_entry,
            priority_spin,
            timeout_spin,
            enabled_check,
            one_shot_check,
            delete_button,
            move_up_button,
            move_down_button,
        }
    }

    /// Wires up the add expect rule button
    fn wire_add_expect_rule_button(
        add_button: &Button,
        expect_rules_list: &ListBox,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let list_clone = expect_rules_list.clone();
        let rules_clone = expect_rules.clone();

        add_button.connect_clicked(move |_| {
            let rule_row = Self::create_expect_rule_row(None);
            let rule_id = rule_row.id;

            // Add a new empty rule to the list
            let new_rule = ExpectRule::with_id(rule_id, "", "");
            rules_clone.borrow_mut().push(new_rule);

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let rules_for_delete = rules_clone.clone();
            let row_widget = rule_row.row.clone();
            let delete_id = rule_id;
            rule_row.delete_button.connect_clicked(move |_| {
                list_for_delete.remove(&row_widget);
                rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
            });

            // Connect move up button
            let list_for_up = list_clone.clone();
            let rules_for_up = rules_clone.clone();
            let row_for_up = rule_row.row.clone();
            let up_id = rule_id;
            rule_row.move_up_button.connect_clicked(move |_| {
                Self::move_rule_up(&list_for_up, &rules_for_up, &row_for_up, up_id);
            });

            // Connect move down button
            let list_for_down = list_clone.clone();
            let rules_for_down = rules_clone.clone();
            let row_for_down = rule_row.row.clone();
            let down_id = rule_id;
            rule_row.move_down_button.connect_clicked(move |_| {
                Self::move_rule_down(&list_for_down, &rules_for_down, &row_for_down, down_id);
            });

            // Connect entry changes to update the rule
            Self::connect_rule_entry_changes(&rule_row, &rules_clone);

            list_clone.append(&rule_row.row);
        });
    }

    /// Wires up template picker buttons to add preset rules
    fn wire_template_buttons(
        template_list_box: &GtkBox,
        expect_rules_list: &ListBox,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let templates = builtin_templates();
        let mut child = template_list_box.first_child();
        let mut idx = 0;

        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Some(btn) = widget.downcast_ref::<Button>()
                && idx < templates.len()
            {
                let list_clone = expect_rules_list.clone();
                let rules_clone = expect_rules.clone();
                let template_idx = idx;

                btn.connect_clicked(move |btn| {
                    let templates = builtin_templates();
                    if template_idx >= templates.len() {
                        return;
                    }
                    let template = &templates[template_idx];
                    let new_rules = template.rules();

                    for rule in &new_rules {
                        let rule_row = Self::create_expect_rule_row(Some(rule));

                        // Connect delete button
                        let list_for_delete = list_clone.clone();
                        let rules_for_delete = rules_clone.clone();
                        let row_widget = rule_row.row.clone();
                        let delete_id = rule_row.id;
                        rule_row.delete_button.connect_clicked(move |_| {
                            list_for_delete.remove(&row_widget);
                            rules_for_delete.borrow_mut().retain(|r| r.id != delete_id);
                        });

                        // Connect move buttons
                        let list_for_up = list_clone.clone();
                        let rules_for_up = rules_clone.clone();
                        let row_for_up = rule_row.row.clone();
                        let up_id = rule_row.id;
                        rule_row.move_up_button.connect_clicked(move |_| {
                            Self::move_rule_up(&list_for_up, &rules_for_up, &row_for_up, up_id);
                        });

                        let list_for_down = list_clone.clone();
                        let rules_for_down = rules_clone.clone();
                        let row_for_down = rule_row.row.clone();
                        let down_id = rule_row.id;
                        rule_row.move_down_button.connect_clicked(move |_| {
                            Self::move_rule_down(
                                &list_for_down,
                                &rules_for_down,
                                &row_for_down,
                                down_id,
                            );
                        });

                        Self::connect_rule_entry_changes(&rule_row, &rules_clone);

                        rules_clone.borrow_mut().push(rule.clone());
                        list_clone.append(&rule_row.row);
                    }

                    // Close the popover
                    if let Some(popover) = btn
                        .ancestor(gtk4::Popover::static_type())
                        .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                    {
                        popover.popdown();
                    }
                });

                idx += 1;
            }
            child = next;
        }
    }

    /// Connects entry changes to update the rule in the list
    fn connect_rule_entry_changes(
        rule_row: &ExpectRuleRow,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let rule_id = rule_row.id;

        // Pattern entry
        let rules_for_pattern = expect_rules.clone();
        let pattern_id = rule_id;
        rule_row.pattern_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Some(rule) = rules_for_pattern
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == pattern_id)
            {
                rule.pattern = text;
            }
        });

        // Response entry
        let rules_for_response = expect_rules.clone();
        let response_id = rule_id;
        rule_row.response_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Some(rule) = rules_for_response
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == response_id)
            {
                rule.response = text;
            }
        });

        // Priority spin
        let rules_for_priority = expect_rules.clone();
        let priority_id = rule_id;
        rule_row.priority_spin.connect_value_changed(move |spin| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "value range fits the target type by construction in this code path"
            )]
            let value = spin.value() as i32;
            if let Some(rule) = rules_for_priority
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == priority_id)
            {
                rule.priority = value;
            }
        });

        // Timeout spin
        let rules_for_timeout = expect_rules.clone();
        let timeout_id = rule_id;
        rule_row.timeout_spin.connect_value_changed(move |spin| {
            #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
            let value = spin.value() as u32;
            if let Some(rule) = rules_for_timeout
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == timeout_id)
            {
                rule.timeout_ms = if value == 0 { None } else { Some(value) };
            }
        });

        // Enabled checkbox
        let rules_for_enabled = expect_rules.clone();
        let enabled_id = rule_id;
        rule_row.enabled_check.connect_toggled(move |check| {
            let enabled = check.is_active();
            if let Some(rule) = rules_for_enabled
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == enabled_id)
            {
                rule.enabled = enabled;
            }
        });

        // One-shot checkbox
        let rules_for_one_shot = expect_rules.clone();
        let one_shot_id = rule_id;
        rule_row.one_shot_check.connect_toggled(move |check| {
            let one_shot = check.is_active();
            if let Some(rule) = rules_for_one_shot
                .borrow_mut()
                .iter_mut()
                .find(|r| r.id == one_shot_id)
            {
                rule.one_shot = one_shot;
            }
        });
    }

    /// Moves a rule up in the list (increases priority)
    fn move_rule_up(
        list: &ListBox,
        rules: &Rc<RefCell<Vec<ExpectRule>>>,
        row: &ListBoxRow,
        _rule_id: Uuid,
    ) {
        let index = row.index();
        if index <= 0 {
            return;
        }

        // Remove and re-insert the row
        list.remove(row);
        let new_index = index - 1;
        list.insert(row, new_index);

        // Update the rules vector
        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let idx = index as usize;
        let mut rules_vec = rules.borrow_mut();
        if idx < rules_vec.len() {
            rules_vec.swap(idx, idx - 1);
        }
    }

    /// Moves a rule down in the list (decreases priority)
    fn move_rule_down(
        list: &ListBox,
        rules: &Rc<RefCell<Vec<ExpectRule>>>,
        row: &ListBoxRow,
        _rule_id: Uuid,
    ) {
        let index = row.index();
        let rules_len = rules.borrow().len();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "value range fits both signed and unsigned target types by construction in this code path"
        )]
        if index < 0 || index >= (rules_len as i32 - 1) {
            return;
        }

        // Remove and re-insert the row
        list.remove(row);
        let new_index = index + 1;
        list.insert(row, new_index);

        // Update the rules vector
        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let idx = index as usize;
        let mut rules_vec = rules.borrow_mut();
        if idx + 1 < rules_vec.len() {
            rules_vec.swap(idx, idx + 1);
        }
    }

    /// Wires up the pattern tester
    fn wire_pattern_tester(
        test_entry: &Entry,
        result_label: &Label,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
    ) {
        let rules_clone = expect_rules.clone();
        let result_clone = result_label.clone();

        test_entry.connect_changed(move |entry| {
            let test_text = entry.text().to_string();
            if test_text.is_empty() {
                result_clone.set_text(&i18n("Enter text above to test patterns"));
                result_clone.remove_css_class("success");
                result_clone.remove_css_class("error");
                result_clone.add_css_class("dim-label");
                return;
            }

            let rules = rules_clone.borrow();
            let mut matched = false;

            // Sort rules by priority (highest first) for testing
            let mut sorted_rules: Vec<_> = rules
                .iter()
                .filter(|r| r.enabled && !r.pattern.is_empty())
                .collect();
            sorted_rules.sort_by_key(|b| std::cmp::Reverse(b.priority));

            for rule in sorted_rules {
                match regex::Regex::new(&rule.pattern) {
                    Ok(re) => {
                        if re.is_match(&test_text) {
                            result_clone.set_text(&format!(
                                "✓ Matched pattern: \"{}\"\n  Response: \"{}\"",
                                rule.pattern, rule.response
                            ));
                            result_clone.remove_css_class("dim-label");
                            result_clone.remove_css_class("error");
                            result_clone.add_css_class("success");
                            matched = true;
                            break;
                        }
                    }
                    Err(e) => {
                        result_clone
                            .set_text(&format!("✗ Invalid pattern \"{}\": {}", rule.pattern, e));
                        result_clone.remove_css_class("dim-label");
                        result_clone.remove_css_class("success");
                        result_clone.add_css_class("error");
                        return;
                    }
                }
            }

            if !matched {
                result_clone.set_text(&i18n("No patterns matched"));
                result_clone.remove_css_class("success");
                result_clone.remove_css_class("error");
                result_clone.add_css_class("dim-label");
            }
        });
    }

    /// Creates a local variable row widget
    fn create_local_variable_row(
        variable: Option<&Variable>,
        is_inherited: bool,
    ) -> LocalVariableRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(12);
        main_box.set_margin_bottom(12);
        main_box.set_margin_start(12);
        main_box.set_margin_end(12);

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Row 0: Name and Delete button
        let name_label = Label::builder()
            .label(i18n("Name:"))
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("variable_name")
            .editable(!is_inherited)
            .build();

        if is_inherited {
            name_entry.add_css_class("dim-label");
        }

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(if is_inherited {
                i18n("Remove override")
            } else {
                i18n("Delete variable")
            })
            .build();
        delete_button.update_property(&[gtk4::accessible::Property::Label(&if is_inherited {
            i18n("Remove variable override")
        } else {
            i18n("Delete variable")
        })]);

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Value (regular entry)
        let value_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Variable value"))
            .build();

        grid.attach(&value_label, 0, 1, 1, 1);
        grid.attach(&value_entry, 1, 1, 2, 1);

        // Row 2: Secret value (password entry, initially hidden)
        let secret_label = Label::builder()
            .label(i18n("Secret Value:"))
            .halign(gtk4::Align::End)
            .visible(false)
            .build();
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Secret value (masked)"))
            .show_peek_icon(true)
            .visible(false)
            .build();

        grid.attach(&secret_label, 0, 2, 1, 1);
        grid.attach(&secret_entry, 1, 2, 2, 1);

        // Row 3: Is Secret checkbox
        let is_secret_check = CheckButton::builder()
            .label(i18n("Secret (mask value)"))
            .build();

        grid.attach(&is_secret_check, 1, 3, 2, 1);

        // Row 4: Description
        let desc_label = Label::builder()
            .label(i18n("Description:"))
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Optional description"))
            .build();

        grid.attach(&desc_label, 0, 4, 1, 1);
        grid.attach(&description_entry, 1, 4, 2, 1);

        // Add inherited indicator if applicable
        if is_inherited {
            let inherited_label = Label::builder()
                .label(i18n("(Inherited from global - override value below)"))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            grid.attach(&inherited_label, 1, 5, 2, 1);
        }

        main_box.append(&grid);

        // Connect is_secret checkbox to toggle value/secret entry visibility
        let value_entry_clone = value_entry.clone();
        let secret_entry_clone = secret_entry.clone();
        let value_label_clone = value_label.clone();
        let secret_label_clone = secret_label.clone();
        is_secret_check.connect_toggled(move |check| {
            let is_secret = check.is_active();
            value_label_clone.set_visible(!is_secret);
            value_entry_clone.set_visible(!is_secret);
            secret_label_clone.set_visible(is_secret);
            secret_entry_clone.set_visible(is_secret);

            // Transfer value between entries when toggling
            if is_secret {
                let value = value_entry_clone.text();
                secret_entry_clone.set_text(&value);
                value_entry_clone.set_text("");
            } else {
                let value = secret_entry_clone.text();
                value_entry_clone.set_text(&value);
                secret_entry_clone.set_text("");
            }
        });

        // Populate from existing variable if provided
        if let Some(var) = variable {
            name_entry.set_text(&var.name);
            if var.is_secret {
                is_secret_check.set_active(true);
                secret_entry.set_text(&var.value);
            } else {
                value_entry.set_text(&var.value);
            }
            if let Some(ref desc) = var.description {
                description_entry.set_text(desc);
            }
        }

        let row = ListBoxRow::builder().child(&main_box).build();

        LocalVariableRow {
            row,
            name_entry,
            value_entry,
            secret_entry,
            is_secret_check,
            description_entry,
            delete_button,
            is_inherited,
        }
    }

    /// Wires up the add variable button
    fn wire_add_variable_button(
        add_button: &Button,
        variables_list: &ListBox,
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
    ) {
        let list_clone = variables_list.clone();
        let rows_clone = variables_rows.clone();

        add_button.connect_clicked(move |_| {
            let var_row = Self::create_local_variable_row(None, false);

            // Connect delete button
            let list_for_delete = list_clone.clone();
            let rows_for_delete = rows_clone.clone();
            let row_widget = var_row.row.clone();
            var_row.delete_button.connect_clicked(move |_| {
                list_for_delete.remove(&row_widget);
                let mut rows = rows_for_delete.borrow_mut();
                rows.retain(|r| r.row != row_widget);
            });

            list_clone.append(&var_row.row);
            rows_clone.borrow_mut().push(var_row);
        });
    }

    /// Collects all local variables from the dialog
    fn collect_local_variables(
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
    ) -> HashMap<String, Variable> {
        let rows = variables_rows.borrow();
        let mut vars = HashMap::new();

        for row in rows.iter() {
            let name = row.name_entry.text().trim().to_string();
            if name.is_empty() {
                continue;
            }

            let is_secret = row.is_secret_check.is_active();
            let value = if is_secret {
                row.secret_entry.text().to_string()
            } else {
                row.value_entry.text().to_string()
            };

            let desc = row.description_entry.text();
            let description = if desc.trim().is_empty() {
                None
            } else {
                Some(desc.trim().to_string())
            };

            let mut var = Variable::new(name.clone(), value);
            var.is_secret = is_secret;
            var.description = description;
            vars.insert(name, var);
        }

        vars
    }

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

    /// Sets up the file chooser button for SPICE CA certificate selection using portal
    pub fn setup_ca_cert_file_chooser(&self, parent_window: Option<&gtk4::Window>) {
        let ca_cert_entry = self.spice_ca_cert_entry.clone();
        let parent = parent_window.cloned();

        self.spice_ca_cert_button.connect_clicked(move |_| {
            let file_dialog = FileDialog::builder()
                .title(i18n("Select CA Certificate"))
                .modal(true)
                .build();

            let entry = ca_cert_entry.clone();
            file_dialog.open(
                parent.as_ref(),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        entry.set_text(&path.to_string_lossy());
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
                && let Some(rgba) = super::advanced_tab::hex_to_rgba(bg)
            {
                self.theme_bg_button.set_rgba(&rgba);
            }
            if let Some(ref fg) = theme.foreground
                && let Some(rgba) = super::advanced_tab::hex_to_rgba(fg)
            {
                self.theme_fg_button.set_rgba(&rgba);
            }
            if let Some(ref cur) = theme.cursor
                && let Some(rgba) = super::advanced_tab::hex_to_rgba(cur)
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

    fn set_groups_list(&self, groups_data: &[(Option<Uuid>, String)]) {
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
    fn set_wol_config(&self, config: Option<&WolConfig>) {
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
    fn set_custom_properties(&self, properties: &[CustomProperty]) {
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
    fn add_custom_property_row(&self, property: Option<&CustomProperty>) {
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
    fn set_pre_connect_task(&self, task: Option<&ConnectionTask>) {
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
    fn set_post_disconnect_task(&self, task: Option<&ConnectionTask>) {
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
    fn set_expect_rules(&self, rules: &[ExpectRule]) {
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
    fn add_expect_rule_row(&self, rule: Option<&ExpectRule>) {
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
    fn set_highlight_rules(&self, rules: &[HighlightRule]) {
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
    fn add_highlight_rule_row(&self, rule: Option<&HighlightRule>) {
        let hl_row = super::advanced_tab::create_highlight_rule_row(rule);
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
    fn wire_add_highlight_rule_button(
        add_button: &Button,
        highlight_rules_list: &ListBox,
        highlight_rules: &Rc<RefCell<Vec<HighlightRule>>>,
    ) {
        let list_clone = highlight_rules_list.clone();
        let rules_clone = highlight_rules.clone();

        add_button.connect_clicked(move |_| {
            let new_rule = HighlightRule::new(String::new(), String::new());
            let hl_row = super::advanced_tab::create_highlight_rule_row(Some(&new_rule));
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
    fn connect_highlight_rule_changes(
        hl_row: &super::advanced_tab::HighlightRuleRow,
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
    fn set_log_config(&self, log_config: Option<&LogConfig>) {
        self.logging_tab.set(log_config);
    }

    /// Sets the local variables for this connection
    fn set_local_variables(&self, local_vars: &HashMap<String, Variable>) {
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
    fn add_local_variable_row(&self, variable: Option<&Variable>, is_inherited: bool) {
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

    fn set_ssh_config(&self, ssh: &SshConfig) {
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
    fn update_ssh_inherit_subtitle(&self, group_id: Option<uuid::Uuid>) {
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
    fn refresh_port_forwards_list(&self) {
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
    fn select_agent_key_by_fingerprint(&self, fingerprint: &str, comment: &str) {
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

    fn set_rdp_config(&self, rdp: &RdpConfig) {
        // Set client mode dropdown
        self.rdp_client_mode_dropdown
            .set_selected(rdp.client_mode.index());

        // Set performance mode dropdown
        self.rdp_performance_mode_dropdown
            .set_selected(rdp.performance_mode.index());

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

    fn set_vnc_config(&self, vnc: &VncConfig) {
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
    }

    fn set_spice_config(&self, spice: &SpiceConfig) {
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
            super::shared_folders::add_folder_row_to_list(
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
    }

    fn set_zerotrust_config(&self, zt: &ZeroTrustConfig) {
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

    fn set_kubernetes_config(&self, k8s: &rustconn_core::models::KubernetesConfig) {
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

    fn set_mosh_config(&self, mosh: &rustconn_core::models::MoshConfig) {
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

    fn set_web_config(&self, web: &rustconn_core::models::WebConfig) {
        if let Some(ref browser) = web.browser {
            self.web_browser_entry.set_text(browser);
        }
        self.web_private_mode_switch.set_active(web.private_mode);
    }

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

    /// Updates password row visibility based on password source
    /// Shows for: Vault(1)
    /// Hides for: Prompt(0), Variable(2), Inherit(3), None(4)
    pub fn update_password_row_visibility(&self) {
        let selected = self.password_source_dropdown.selected();
        // Show password row for Vault(1) only
        self.password_row.set_visible(selected == 1);
        // Show variable row for Variable(2) only
        self.variable_row.set_visible(selected == 2);
        // Show script row for Script(5) only
        self.script_row.set_visible(selected == 5);
    }

    /// Connects password visibility toggle button
    pub fn connect_password_visibility_toggle(&self) {
        use std::cell::Cell;

        let password_entry = self.password_entry.clone();
        // Track visibility state - starts hidden (false)
        let is_visible = Rc::new(Cell::new(false));

        self.password_visibility_button.connect_clicked(move |btn| {
            let currently_visible = is_visible.get();
            let new_visible = !currently_visible;
            is_visible.set(new_visible);
            password_entry.set_visibility(new_visible);
            // Update icon
            if new_visible {
                btn.set_icon_name("view-conceal-symbolic");
            } else {
                btn.set_icon_name("view-reveal-symbolic");
            }
        });
    }

    /// Connects password source dropdown to update password row visibility
    pub fn connect_password_source_visibility(&self) {
        let password_row = self.password_row.clone();
        let variable_row = self.variable_row.clone();
        let script_row = self.script_row.clone();
        let ssh_auth_dropdown = self.ssh_auth_dropdown.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();

        self.password_source_dropdown
            .connect_selected_notify(move |dropdown| {
                let selected = dropdown.selected();
                // Show password row for Vault(1) only
                password_row.set_visible(selected == 1);
                // Show variable row for Variable(2) only
                variable_row.set_visible(selected == 2);
                // Show script row for Script(5) only
                script_row.set_visible(selected == 5);

                // Sync: when password source is None(4) and protocol is SSH(0),
                // auto-switch SSH auth from Password(0) to Public Key(1)
                if selected == 4
                    && protocol_dropdown.selected() == 0
                    && ssh_auth_dropdown.selected() == 0
                {
                    ssh_auth_dropdown.set_selected(1);
                }
            });

        // Reverse sync: when SSH auth changes to Password(0) while
        // password source is None(4), auto-switch password source to Prompt(0)
        let password_source_dropdown = self.password_source_dropdown.clone();
        let protocol_dropdown2 = self.protocol_dropdown.clone();
        self.ssh_auth_dropdown
            .connect_selected_notify(move |dropdown| {
                let is_ssh = protocol_dropdown2.selected() == 0;
                if is_ssh && dropdown.selected() == 0 && password_source_dropdown.selected() == 4 {
                    password_source_dropdown.set_selected(0); // Prompt
                }
            });
    }

    /// Connects password load button to load password from vault (KeePass or Keyring)
    ///
    /// This method sets up the click handler for the password load button.
    /// Connects password load button with group hierarchy support
    ///
    /// This method sets up the click handler for the password load button.
    /// When clicked, it loads the password from the appropriate backend based on
    /// the selected password source (KeePass or Keyring).
    ///
    /// # Arguments
    /// * `kdbx_enabled` - Whether KeePass is enabled
    /// * `kdbx_path` - Path to the KeePass database
    /// * `kdbx_password` - Password for the KeePass database
    /// * `kdbx_key_file` - Key file for the KeePass database
    /// * `groups` - List of connection groups for building hierarchical paths
    /// * `secret_settings` - Secret backend settings for backend dispatch
    #[allow(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn connect_password_load_button_with_groups(
        &self,
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<&secrecy::SecretString>,
        kdbx_key_file: Option<std::path::PathBuf>,
        groups: Vec<rustconn_core::models::ConnectionGroup>,
        secret_settings: rustconn_core::config::SecretSettings,
    ) {
        use crate::utils::spawn_blocking_with_callback;

        let password_source_dropdown = self.password_source_dropdown.clone();
        let password_entry = self.password_entry.clone();
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let group_dropdown = self.group_dropdown.clone();
        let groups_data = self.groups_data.clone();
        let window = self.dialog.clone();
        let kdbx_password = kdbx_password.cloned();

        // Clone groups for use in closure
        let groups = Rc::new(groups);

        self.password_load_button.connect_clicked(move |btn| {
            let selected = password_source_dropdown.selected();

            // Get connection name for lookup key
            let conn_name = name_entry.text().to_string();
            let conn_host = host_entry.text().to_string();
            let protocol_index = protocol_dropdown.selected();

            // Build lookup key with protocol for uniqueness
            let base_name = if conn_name.trim().is_empty() {
                conn_host.clone()
            } else {
                conn_name.clone()
            };

            if base_name.trim().is_empty() {
                alert::show_error(
                    &window,
                    &i18n("Cannot Load Password"),
                    &i18n("Please enter a connection name or host first."),
                );
                return;
            }

            let protocol_suffix = match protocol_index {
                0 => "ssh",
                1 => "rdp",
                2 => "vnc",
                3 => "spice",
                4 => "zerotrust",
                _ => "ssh",
            };

            // Build hierarchical lookup key for KeePass
            let lookup_key = if groups.is_empty() {
                // Legacy behavior: sanitize name and use flat path
                let sanitized_name = base_name.replace('/', "-");
                format!("{sanitized_name} ({protocol_suffix})")
            } else {
                // Build hierarchical path using selected group
                let selected_group_idx = group_dropdown.selected() as usize;
                let groups_data_ref = groups_data.borrow();
                let group_id = if selected_group_idx < groups_data_ref.len() {
                    groups_data_ref[selected_group_idx].0
                } else {
                    None
                };
                drop(groups_data_ref);

                // Build path from group hierarchy
                let group_path = if let Some(gid) = group_id {
                    rustconn_core::secret::KeePassHierarchy::resolve_group_path(gid, &groups)
                } else {
                    Vec::new()
                };

                if group_path.is_empty() {
                    format!("{base_name} ({protocol_suffix})")
                } else {
                    let path = group_path.join("/");
                    format!("{path}/{base_name} ({protocol_suffix})")
                }
            };

            // Flat lookup key — must match the format used by
            // `generate_store_key` so that store and retrieve are consistent.
            // LibSecret uses "{name} ({protocol})", while Bitwarden and other
            // backends use "rustconn/{name}".
            let flat_lookup_key = {
                let backend_type =
                    crate::state::select_backend_for_load(&secret_settings);
                crate::state::generate_store_key(
                    &conn_name,
                    &conn_host,
                    protocol_suffix,
                    backend_type,
                )
            };

            match selected {
                1 => {
                    // Vault — delegate to configured backend
                    let password_entry = password_entry.clone();
                    let window = window.clone();
                    let btn = btn.clone();
                    let kdbx_enabled = kdbx_enabled;
                    let kdbx_path = kdbx_path.clone();
                    let kdbx_password = kdbx_password.clone();
                    let kdbx_key_file = kdbx_key_file.clone();
                    let lookup_key = lookup_key.clone();
                    let flat_lookup_key = flat_lookup_key.clone();

                    btn.set_sensitive(false);
                    btn.set_icon_name("content-loading-symbolic");

                    // Try KeePass first if enabled, then fall back to
                    // Keyring/Bitwarden/etc.
                    if kdbx_enabled
                        && matches!(
                            secret_settings.preferred_backend,
                            rustconn_core::config::SecretBackendType::KeePassXc
                                | rustconn_core::config::SecretBackendType::KdbxFile
                        )
                    {
                        if let Some(ref kdbx_path) = kdbx_path {
                            let kdbx_path = kdbx_path.clone();
                            let db_password = kdbx_password.clone();
                            let key_file = kdbx_key_file.clone();

                            spawn_blocking_with_callback(
                                move || {
                                    rustconn_core::secret::KeePassStatus
                                        ::get_password_from_kdbx_with_key(
                                            &kdbx_path,
                                            db_password.as_ref(),
                                            key_file.as_deref(),
                                            &lookup_key,
                                            None,
                                        )
                                },
                                move |result: rustconn_core::error::SecretResult<
                                    Option<secrecy::SecretString>,
                                >| {
                                    btn.set_sensitive(true);
                                    btn.set_icon_name("document-open-symbolic");

                                    match result {
                                        Ok(Some(password)) => {
                                            use secrecy::ExposeSecret;
                                            password_entry.set_text(password.expose_secret());
                                        }
                                        Ok(None) => {
                                            alert::show_error(
                                                &window,
                                                &i18n("Password Not Found"),
                                                &i18n("No password found in vault for this connection."),
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to load password from vault: {e}"
                                            );
                                            alert::show_error(
                                                &window,
                                                &i18n("Failed to Load Password"),
                                                &i18n("Could not load password from vault."),
                                            );
                                        }
                                    }
                                },
                            );
                        } else {
                            btn.set_sensitive(true);
                            btn.set_icon_name("document-open-symbolic");
                            alert::show_error(
                                &window,
                                &i18n("Vault Not Configured"),
                                &i18n("Please configure a secret backend in Settings → Secrets."),
                            );
                        }
                    } else {
                        // Non-KeePass backend — dispatch based on
                        // preferred_backend
                        let secret_settings = secret_settings.clone();
                        spawn_blocking_with_callback(
                            move || {
                                use rustconn_core::config::SecretBackendType;
                                use rustconn_core::secret::SecretBackend;

                                let backend_type =
                                    crate::state::select_backend_for_load(&secret_settings);

                                match backend_type {
                                    SecretBackendType::Bitwarden => {
                                        crate::async_utils::with_runtime(|rt| {
                                            let backend = rt
                                                .block_on(rustconn_core::secret::auto_unlock(
                                                    &secret_settings,
                                                ))
                                                .map_err(|e| format!("{e}"))?;
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::OnePassword => {
                                        let backend =
                                            rustconn_core::secret::OnePasswordBackend::new();
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::Passbolt => {
                                        let backend = rustconn_core::secret::PassboltBackend::new();
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::Pass => {
                                        let backend =
                                            rustconn_core::secret::PassBackend::from_secret_settings(
                                                &secret_settings,
                                            );
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::LibSecret
                                    | SecretBackendType::MacOsKeychain
                                    | SecretBackendType::KeePassXc
                                    | SecretBackendType::KdbxFile => {
                                        let backend = rustconn_core::secret::LibSecretBackend::new(
                                            "rustconn",
                                        );
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                }
                            },
                            move |result: Result<
                                Option<rustconn_core::models::Credentials>,
                                String,
                            >| {
                                btn.set_sensitive(true);
                                btn.set_icon_name("document-open-symbolic");

                                match result {
                                    Ok(Some(creds)) => {
                                        if let Some(password) = creds.expose_password() {
                                            password_entry.set_text(password);
                                        } else {
                                            alert::show_error(
                                                &window,
                                                &i18n("Password Not Found"),
                                                &i18n("No password found in vault for this connection."),
                                            );
                                        }
                                    }
                                    Ok(None) => {
                                        alert::show_error(
                                            &window,
                                            &i18n("Password Not Found"),
                                            &i18n("No password found in vault for this connection."),
                                        );
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to load password from vault: {e}");
                                        alert::show_error(
                                            &window,
                                            &i18n("Failed to Load Password"),
                                            &i18n("Could not load password from vault."),
                                        );
                                    }
                                }
                            },
                        );
                    }
                }
                _ => {
                    // Prompt(0), Variable(2), Inherit(3), None(4)
                    alert::show_error(
                        &window,
                        &i18n("Cannot Load Password"),
                        &i18n("Password loading is only available for Vault source."),
                    );
                }
            }
        });
    }

    /// Wires up the "Test credential resolution" button.
    ///
    /// When clicked, performs a vault lookup using the current connection name,
    /// host, protocol, and group — then shows a success/failure dialog with
    /// the lookup key used. Helps users verify their vault configuration
    /// before connecting.
    #[allow(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn connect_vault_test_button(
        &self,
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<&secrecy::SecretString>,
        kdbx_key_file: Option<std::path::PathBuf>,
        groups: Vec<rustconn_core::models::ConnectionGroup>,
        secret_settings: rustconn_core::config::SecretSettings,
    ) {
        use crate::utils::spawn_blocking_with_callback;

        let password_source_dropdown = self.password_source_dropdown.clone();
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let group_dropdown = self.group_dropdown.clone();
        let groups_data = self.groups_data.clone();
        let window = self.dialog.clone();
        let kdbx_password = kdbx_password.cloned();
        let groups = Rc::new(groups);

        self.vault_test_button.connect_clicked(move |btn| {
            let selected = password_source_dropdown.selected();
            if selected != 1 {
                // Only test for Vault source
                alert::show_error(
                    &window,
                    &i18n("Test Not Available"),
                    &i18n("Credential test is only available when Password Source is set to Vault."),
                );
                return;
            }

            let conn_name = name_entry.text().to_string();
            let conn_host = host_entry.text().to_string();
            let protocol_index = protocol_dropdown.selected();

            let base_name = if conn_name.trim().is_empty() {
                conn_host.clone()
            } else {
                conn_name.clone()
            };

            if base_name.trim().is_empty() {
                alert::show_error(
                    &window,
                    &i18n("Cannot Test"),
                    &i18n("Please enter a connection name or host first."),
                );
                return;
            }

            let protocol_suffix = match protocol_index {
                0 => "ssh",
                1 => "rdp",
                2 => "vnc",
                3 => "spice",
                4 => "zerotrust",
                _ => "ssh",
            };

            // Build hierarchical lookup key for KeePass
            let lookup_key = if groups.is_empty() {
                let sanitized_name = base_name.replace('/', "-");
                format!("{sanitized_name} ({protocol_suffix})")
            } else {
                let selected_group_idx = group_dropdown.selected() as usize;
                let groups_data_ref = groups_data.borrow();
                let group_id = if selected_group_idx < groups_data_ref.len() {
                    groups_data_ref[selected_group_idx].0
                } else {
                    None
                };
                drop(groups_data_ref);

                let group_path = if let Some(gid) = group_id {
                    rustconn_core::secret::KeePassHierarchy::resolve_group_path(gid, &groups)
                } else {
                    Vec::new()
                };

                if group_path.is_empty() {
                    format!("{base_name} ({protocol_suffix})")
                } else {
                    let path = group_path.join("/");
                    format!("{path}/{base_name} ({protocol_suffix})")
                }
            };

            let flat_lookup_key = {
                let backend_type = crate::state::select_backend_for_load(&secret_settings);
                crate::state::generate_store_key(
                    &conn_name,
                    &conn_host,
                    protocol_suffix,
                    backend_type,
                )
            };

            btn.set_sensitive(false);
            btn.set_icon_name("content-loading-symbolic");

            let btn_clone = btn.clone();
            let window_clone = window.clone();
            let lookup_key_display = lookup_key.clone();
            let flat_key_display = flat_lookup_key.clone();

            if kdbx_enabled
                && matches!(
                    secret_settings.preferred_backend,
                    rustconn_core::config::SecretBackendType::KeePassXc
                        | rustconn_core::config::SecretBackendType::KdbxFile
                )
            {
                if let Some(ref kdbx_path) = kdbx_path {
                    let kdbx_path = kdbx_path.clone();
                    let db_password = kdbx_password.clone();
                    let key_file = kdbx_key_file.clone();

                    spawn_blocking_with_callback(
                        move || {
                            rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                                &kdbx_path,
                                db_password.as_ref(),
                                key_file.as_deref(),
                                &lookup_key,
                                None,
                            )
                        },
                        move |result: rustconn_core::error::SecretResult<
                            Option<secrecy::SecretString>,
                        >| {
                            btn_clone.set_sensitive(true);
                            btn_clone.set_icon_name("emblem-ok-symbolic");

                            match result {
                                Ok(Some(_)) => {
                                    alert::show_success(
                                        &window_clone,
                                        &i18n("Credential Test Passed"),
                                        &i18n_f(
                                            "Password found in vault.\nLookup key: {}",
                                            &[&lookup_key_display],
                                        ),
                                    );
                                }
                                Ok(None) => {
                                    alert::show_error(
                                        &window_clone,
                                        &i18n("Credential Test Failed"),
                                        &i18n_f(
                                            "No password found in vault.\nLookup key: {}",
                                            &[&lookup_key_display],
                                        ),
                                    );
                                }
                                Err(e) => {
                                    tracing::error!("Vault test failed: {e}");
                                    alert::show_error(
                                        &window_clone,
                                        &i18n("Credential Test Failed"),
                                        &i18n_f(
                                            "Vault error. Check your KeePass configuration.\nLookup key: {}",
                                            &[&lookup_key_display],
                                        ),
                                    );
                                }
                            }
                        },
                    );
                } else {
                    btn.set_sensitive(true);
                    btn.set_icon_name("emblem-ok-symbolic");
                    alert::show_error(
                        &window,
                        &i18n("Vault Not Configured"),
                        &i18n("Please configure a secret backend in Settings → Secrets."),
                    );
                }
            } else {
                // Non-KeePass backend
                let secret_settings = secret_settings.clone();
                spawn_blocking_with_callback(
                    move || {
                        crate::state::dispatch_vault_op(
                            &secret_settings,
                            &flat_lookup_key,
                            crate::state::VaultOp::Retrieve,
                        )
                    },
                    move |result: Result<
                        Option<rustconn_core::models::Credentials>,
                        String,
                    >| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("emblem-ok-symbolic");

                        match result {
                            Ok(Some(creds)) => {
                                let has_pw = creds.expose_password().is_some();
                                if has_pw {
                                    alert::show_success(
                                        &window_clone,
                                        &i18n("Credential Test Passed"),
                                        &i18n_f(
                                            "Password found in vault.\nLookup key: {}",
                                            &[&flat_key_display],
                                        ),
                                    );
                                } else {
                                    alert::show_error(
                                        &window_clone,
                                        &i18n("Credential Test Failed"),
                                        &i18n_f(
                                            "Entry found but contains no password.\nLookup key: {}",
                                            &[&flat_key_display],
                                        ),
                                    );
                                }
                            }
                            Ok(None) => {
                                alert::show_error(
                                    &window_clone,
                                    &i18n("Credential Test Failed"),
                                    &i18n_f(
                                        "No entry found in vault.\nLookup key: {}",
                                        &[&flat_key_display],
                                    ),
                                );
                            }
                            Err(e) => {
                                tracing::error!("Vault test failed: {e}");
                                alert::show_error(
                                    &window_clone,
                                    &i18n("Credential Test Failed"),
                                    &i18n_f(
                                        "Backend error. Check your vault configuration.\nLookup key: {}",
                                        &[&flat_key_display],
                                    ),
                                );
                            }
                        }
                    },
                );
            }
        });
    }

    /// Returns the password entry widget for external access
    #[must_use]
    pub const fn password_entry(&self) -> &Entry {
        &self.password_entry
    }

    /// Returns the password row widget for external access
    #[must_use]
    pub const fn password_row(&self) -> &GtkBox {
        &self.password_row
    }

    /// Refreshes the SSH agent key list asynchronously.
    ///
    /// Spawns the agent probe on a background thread with a 5-second timeout so
    /// the GTK main thread is never blocked — even if the system ssh-agent is
    /// broken or launchd-throttled (common on macOS).
    ///
    /// This should be called before showing the dialog to populate the agent key dropdown
    /// with the currently loaded keys from the SSH agent.
    pub fn refresh_agent_keys(&self) {
        use rustconn_core::ssh_agent::SshAgentManager;

        // 5-second timeout — enough for a healthy agent, prevents indefinite hang.
        const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        // Show a placeholder while loading
        let loading_items: Vec<String> = vec![i18n("Loading agent keys…")];
        let loading_list =
            StringList::new(&loading_items.iter().map(String::as_str).collect::<Vec<_>>());
        self.ssh_agent_key_dropdown.set_model(Some(&loading_list));
        self.ssh_agent_key_dropdown.set_selected(0);
        self.ssh_agent_key_dropdown.set_sensitive(false);

        // Clone the Rc fields needed to update UI after async completion
        let ssh_agent_keys = self.ssh_agent_keys.clone();
        let ssh_agent_key_dropdown = self.ssh_agent_key_dropdown.clone();
        let ssh_key_source_dropdown = self.ssh_key_source_dropdown.clone();
        let pending_agent_selection = self.pending_agent_selection.clone();

        // Read the socket path from environment now (cheap, no blocking)
        let manager = SshAgentManager::from_env();

        glib::spawn_future_local(async move {
            // Run the blocking agent probe on a background thread
            let keys = glib::spawn_future(async move {
                match manager.get_status_timeout(AGENT_TIMEOUT) {
                    Ok(status) if status.running => status.keys,
                    _ => Vec::new(),
                }
            })
            .await
            .unwrap_or_default();

            // Back on GTK main thread — update the UI
            *ssh_agent_keys.borrow_mut() = keys.clone();

            let items: Vec<String> = if keys.is_empty() {
                vec![i18n("No keys loaded")]
            } else {
                keys.iter()
                    .map(|key| Self::format_agent_key_short(key))
                    .collect()
            };

            let string_list =
                StringList::new(&items.iter().map(String::as_str).collect::<Vec<_>>());
            ssh_agent_key_dropdown.set_model(Some(&string_list));
            ssh_agent_key_dropdown.set_selected(0);

            // Update sensitivity based on whether keys are available
            let has_keys = !keys.is_empty();
            if ssh_key_source_dropdown.selected() == 2 {
                // Agent source is selected
                ssh_agent_key_dropdown.set_sensitive(has_keys);
            }

            // Restore pending agent key selection (set by set_ssh_config before keys were loaded)
            if let Some((ref fingerprint, ref comment)) = *pending_agent_selection.borrow() {
                let keys_ref = ssh_agent_keys.borrow();
                for (idx, key) in keys_ref.iter().enumerate() {
                    if key.fingerprint == *fingerprint || key.comment == *comment {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "agent key count always fits u32"
                        )]
                        ssh_agent_key_dropdown.set_selected(idx as u32);
                        break;
                    }
                }
            }
        });
    }

    /// Formats an agent key for short display in dropdown button
    /// Shows: "comment_start...comment_end (TYPE)"
    fn format_agent_key_short(key: &rustconn_core::ssh_agent::AgentKey) -> String {
        let comment = &key.comment;
        let max_comment_len = 24;

        let short_comment = if comment.len() > max_comment_len {
            // Show first 10 and last 10 chars with ellipsis
            let start = &comment[..10];
            let end = &comment[comment.len() - 10..];
            format!("{start}…{end}")
        } else {
            comment.clone()
        };

        format!("{short_comment} ({})", key.key_type)
    }

    /// Sets the global variables to display as inherited in the Variables tab
    ///
    /// This should be called before `set_connection` to properly show
    /// which variables are inherited from global scope.
    pub fn set_global_variables(&self, variables: &[Variable]) {
        *self.global_variables.borrow_mut() = variables.to_vec();

        // Populate variable dropdown with secret global variables
        if let Some(model) = self.variable_dropdown.model()
            && let Some(sl) = model.downcast_ref::<gtk4::StringList>()
        {
            // Clear existing items
            sl.splice(0, sl.n_items(), &[] as &[&str]);
            // Add secret variables only
            for var in variables {
                if var.is_secret {
                    sl.append(&var.name);
                }
            }
        }
    }

    /// Populates the Variables tab with inherited global variables
    ///
    /// Call this after `set_global_variables` to show global variables
    /// that can be overridden locally.
    pub fn populate_inherited_variables(&self) {
        // Clear existing rows first
        while let Some(row) = self.variables_list.row_at_index(0) {
            self.variables_list.remove(&row);
        }
        self.variables_rows.borrow_mut().clear();

        // Add rows for each global variable (as inherited, read-only name)
        let global_vars = self.global_variables.borrow();
        for var in global_vars.iter() {
            // Create a row showing the global variable with empty value
            // (user can fill in to override)
            let mut display_var = var.clone();
            display_var.value = String::new(); // Clear value so user can override
            self.add_local_variable_row(Some(&display_var), true);
        }
    }

    /// Sets the preferred secret backend and updates the password source dropdown
    ///
    /// This method sets the default selection in the password source dropdown
    /// based on the configured secret backend in Settings:
    /// - All backends map to Vault (index 1)
    ///
    /// The dropdown shows: Prompt(0), Vault(1), Variable(2), Inherit(3), None(4)
    pub fn set_preferred_backend(&self, _backend: rustconn_core::config::SecretBackendType) {
        // All backends map to Vault (index 1)
        self.password_source_dropdown.set_selected(1);

        // Update password row visibility based on new selection
        self.update_password_row_visibility();
    }

    /// Configures the dialog for Import group mode (Requirement 5.1, 5.2).
    ///
    /// Synced fields become read-only (insensitive) with a tooltip explaining
    /// they are managed by cloud sync. Local-only fields remain editable.
    ///
    /// This follows the GNOME HIG pattern of using insensitive widgets with
    /// descriptive tooltips rather than replacing the entire dialog layout.
    pub fn configure_import_group_mode(&self) {
        use crate::i18n::i18n;

        let managed_tooltip = i18n("Managed by cloud sync");

        // Synced fields → read-only (insensitive + tooltip)
        // Name, host, port, protocol, username, domain, tags, description
        self.name_entry.set_sensitive(false);
        self.name_entry.set_tooltip_text(Some(&managed_tooltip));

        self.host_entry.set_sensitive(false);
        self.host_entry.set_tooltip_text(Some(&managed_tooltip));

        self.port_spin.set_sensitive(false);
        self.port_spin.set_tooltip_text(Some(&managed_tooltip));

        self.protocol_dropdown.set_sensitive(false);
        self.protocol_dropdown
            .set_tooltip_text(Some(&managed_tooltip));

        self.username_entry.set_sensitive(false);
        self.username_entry.set_tooltip_text(Some(&managed_tooltip));

        self.domain_entry.set_sensitive(false);
        self.domain_entry.set_tooltip_text(Some(&managed_tooltip));

        self.tags_entry.set_sensitive(false);
        self.tags_entry.set_tooltip_text(Some(&managed_tooltip));

        self.description_view.set_sensitive(false);
        self.description_view
            .set_tooltip_text(Some(&managed_tooltip));

        self.icon_entry.set_sensitive(false);
        self.icon_entry.set_tooltip_text(Some(&managed_tooltip));

        // Group dropdown → read-only (can't move connection between groups)
        self.group_dropdown.set_sensitive(false);
        self.group_dropdown.set_tooltip_text(Some(&managed_tooltip));

        // Local-only fields remain editable:
        // password_source_dropdown, ssh_key_entry, ssh_key_source_dropdown
        // (these are already editable by default — no changes needed)

        // Update window title to indicate Import group mode
        self.dialog.set_title(&i18n("Edit Connection (Synced)"));
    }
}
