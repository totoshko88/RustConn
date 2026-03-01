//! Connection dialog implementation
//!
//! This is the main dialog file. Protocol-specific UI is in submodules:
//! - `super::ssh` - SSH options

// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(clippy::similar_names)]

use super::logging_tab;
use super::ssh;
use crate::alert;
use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, Grid, Label, ListBox,
    ListBoxRow, Orientation, PasswordEntry, ScrolledWindow, SpinButton, Stack, StringList,
    TextView, WrapMode,
};
use libadwaita as adw;
use rustconn_core::automation::{ConnectionTask, ExpectRule, TaskCondition, builtin_templates};
use rustconn_core::models::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    Connection, CustomProperty, GcpIapConfig, GenericZeroTrustConfig, OciBastionConfig,
    PasswordSource, PropertyType, ProtocolConfig, RdpClientMode, RdpConfig, RdpPerformanceMode,
    Resolution, ScaleOverride, SharedFolder, SpiceConfig, SpiceImageCompression, SshAuthMethod,
    SshConfig, SshKeySource, TailscaleSshConfig, TeleportConfig, VncClientMode, VncConfig,
    VncPerformanceMode, WindowMode, ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
};
use rustconn_core::session::LogConfig;
use rustconn_core::variables::Variable;
use rustconn_core::wol::{
    DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS, MacAddress, WolConfig,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;

/// Keyboard layout KLID values matching the dropdown order.
/// Index 0 = Auto (None), rest map to specific Windows KLIDs.
const KEYBOARD_LAYOUT_KLIDS: &[u32] = &[
    0x0000, // Auto (placeholder, not used)
    0x0409, // US English
    0x0407, // German
    0x040C, // French
    0x040A, // Spanish
    0x0410, // Italian
    0x0816, // Portuguese
    0x0416, // Brazilian
    0x0809, // UK English
    0x0807, // Swiss German
    0x0C07, // Austrian
    0x080C, // Belgian
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
fn dropdown_index_to_klid(index: u32) -> Option<u32> {
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
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle and signal handlers
pub struct ConnectionDialog {
    window: adw::Window,
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
    password_row: GtkBox,
    // Variable name dropdown (for Variable password source)
    variable_dropdown: DropDown,
    variable_row: GtkBox,
    // Group selection
    group_dropdown: DropDown,
    groups_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    // SSH fields
    ssh_auth_dropdown: DropDown,
    ssh_key_source_dropdown: DropDown,
    ssh_key_entry: Entry,
    ssh_key_button: Button,
    ssh_agent_key_dropdown: DropDown,
    ssh_agent_keys: Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>>,
    ssh_jump_host_dropdown: DropDown,
    ssh_proxy_entry: Entry,
    ssh_identities_only: CheckButton,
    ssh_control_master: CheckButton,
    ssh_agent_forwarding: CheckButton,
    ssh_waypipe: CheckButton,
    ssh_x11_forwarding: CheckButton,
    ssh_compression: CheckButton,
    ssh_startup_entry: Entry,
    ssh_options_entry: Entry,
    ssh_port_forwards: Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
    ssh_port_forwards_list: gtk4::ListBox,
    // RDP fields
    rdp_client_mode_dropdown: DropDown,
    rdp_performance_mode_dropdown: DropDown,
    rdp_width_spin: SpinButton,
    rdp_height_spin: SpinButton,
    rdp_color_dropdown: DropDown,
    rdp_scale_override_dropdown: DropDown,
    rdp_audio_check: CheckButton,
    rdp_gateway_entry: Entry,
    rdp_shared_folders: Rc<RefCell<Vec<SharedFolder>>>,
    rdp_shared_folders_list: gtk4::ListBox,
    rdp_custom_args_entry: Entry,
    rdp_keyboard_layout_dropdown: DropDown,
    // VNC fields
    vnc_client_mode_dropdown: DropDown,
    vnc_performance_mode_dropdown: DropDown,
    vnc_encoding_entry: Entry,
    vnc_compression_spin: SpinButton,
    vnc_quality_spin: SpinButton,
    vnc_view_only_check: CheckButton,
    vnc_scaling_check: CheckButton,
    vnc_clipboard_check: CheckButton,
    vnc_scale_override_dropdown: DropDown,
    vnc_custom_args_entry: Entry,
    // SPICE fields
    spice_tls_check: CheckButton,
    spice_ca_cert_entry: Entry,
    spice_ca_cert_button: Button,
    spice_skip_verify_check: CheckButton,
    spice_usb_check: CheckButton,
    spice_clipboard_check: CheckButton,
    spice_compression_dropdown: DropDown,
    spice_shared_folders: Rc<RefCell<Vec<SharedFolder>>>,
    spice_shared_folders_list: gtk4::ListBox,
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
    // Generic fields
    zt_generic_command_entry: adw::EntryRow,
    // Custom args for all providers
    zt_custom_args_entry: Entry,
    // Telnet fields
    telnet_custom_args_entry: Entry,
    telnet_backspace_dropdown: DropDown,
    telnet_delete_dropdown: DropDown,
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
    pre_connect_enabled_check: CheckButton,
    pre_connect_command_entry: Entry,
    pre_connect_timeout_spin: SpinButton,
    pre_connect_abort_check: CheckButton,
    pre_connect_first_only_check: CheckButton,
    post_disconnect_enabled_check: CheckButton,
    post_disconnect_command_entry: Entry,
    post_disconnect_timeout_spin: SpinButton,
    post_disconnect_last_only_check: CheckButton,
    // Window mode fields
    window_mode_dropdown: DropDown,
    remember_position_check: CheckButton,
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
    // State
    editing_id: Rc<RefCell<Option<Uuid>>>,
    // Callback
    on_save: super::ConnectionCallback,
    connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    full_groups_data: Rc<RefCell<HashMap<Uuid, rustconn_core::models::ConnectionGroup>>>,
}

/// Represents a local variable row in the connection dialog
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
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
    #[allow(clippy::too_many_lines)]
    pub fn new(parent: Option<&gtk4::Window>, state: crate::state::SharedAppState) -> Self {
        let (window, header, save_btn, test_btn) = Self::create_window_with_header(parent);
        let view_stack = Self::create_view_stack(&window, &header);

        // === Basic Tab ===
        let (
            basic_grid,
            name_entry,
            icon_entry,
            description_view,
            host_entry,
            host_label,
            port_spin,
            port_label,
            username_entry,
            username_label,
            domain_entry,
            _domain_label,
            tags_entry,
            tags_label,
            protocol_dropdown,
            password_source_dropdown,
            password_source_label,
            password_entry,
            _password_entry_label,
            password_visibility_button,
            password_load_button,
            password_row,
            variable_dropdown,
            variable_row,
            group_dropdown,
            username_load_button,
            domain_load_button,
        ) = Self::create_basic_tab();
        // Wrap basic grid in ScrolledWindow for consistent styling
        let basic_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&basic_grid)
            .build();
        view_stack
            .add_titled(&basic_scrolled, Some("basic"), &i18n("Basic"))
            .set_icon_name(Some("document-properties-symbolic"));

        // === Protocol-specific Tab ===
        let protocol_stack = Self::create_protocol_stack(&view_stack);

        // Storage for agent keys (populated when dialog is shown)
        let ssh_agent_keys: Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>> =
            Rc::new(RefCell::new(Vec::new()));

        // Storage for port forwarding rules (created before SSH options so we can
        // append the port forwarding group to the SSH panel)
        let ssh_port_forwards: Rc<RefCell<Vec<rustconn_core::models::PortForward>>> =
            Rc::new(RefCell::new(Vec::new()));
        let ssh_port_forwards_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();

        // SSH options
        let (
            ssh_box,
            ssh_auth_dropdown,
            ssh_key_source_dropdown,
            ssh_key_entry,
            ssh_key_button,
            ssh_agent_key_dropdown,
            ssh_jump_host_dropdown,
            ssh_proxy_entry,
            ssh_identities_only,
            ssh_control_master,
            ssh_agent_forwarding,
            ssh_waypipe,
            ssh_x11_forwarding,
            ssh_compression,
            ssh_startup_entry,
            ssh_options_entry,
        ) = ssh::create_ssh_options();

        // Add port forwarding group to SSH options panel
        // Navigate: ssh_box → ScrolledWindow → Clamp → content Box
        if let Some(scrolled) = ssh_box.first_child()
            && let Some(scrolled_win) = scrolled.downcast_ref::<ScrolledWindow>()
            && let Some(clamp) = scrolled_win.child()
            && let Some(adw_clamp) = clamp.downcast_ref::<adw::Clamp>()
            && let Some(content) = adw_clamp.child()
            && let Some(content_box) = content.downcast_ref::<GtkBox>()
        {
            let pf_group =
                ssh::create_port_forwarding_group(&ssh_port_forwards_list, &ssh_port_forwards);
            content_box.append(&pf_group);
        }

        protocol_stack.add_named(&ssh_box, Some("ssh"));

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
            rdp_shared_folders,
            rdp_shared_folders_list,
            rdp_custom_args_entry,
            rdp_keyboard_layout_dropdown,
        ) = Self::create_rdp_options();
        protocol_stack.add_named(&rdp_box, Some("rdp"));

        // VNC options
        let (
            vnc_box,
            vnc_client_mode_dropdown,
            vnc_performance_mode_dropdown,
            vnc_encoding_entry,
            vnc_compression_spin,
            vnc_quality_spin,
            vnc_view_only_check,
            vnc_scaling_check,
            vnc_clipboard_check,
            vnc_scale_override_dropdown,
            vnc_custom_args_entry,
        ) = Self::create_vnc_options();
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
            spice_shared_folders,
            spice_shared_folders_list,
        ) = Self::create_spice_options();
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
            zt_cf_hostname_entry,
            zt_teleport_host_entry,
            zt_teleport_cluster_entry,
            zt_tailscale_host_entry,
            zt_boundary_target_entry,
            zt_boundary_addr_entry,
            zt_generic_command_entry,
            zt_custom_args_entry,
        ) = Self::create_zerotrust_options();
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
        );

        // === Data Tab (Variables + Custom Properties) ===
        let (
            data_tab,
            variables_list,
            add_variable_button,
            custom_properties_list,
            add_custom_property_button,
        ) = Self::create_data_tab();
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
        let (
            automation_tab,
            expect_rules_list,
            add_expect_rule_button,
            template_list_box,
            expect_pattern_test_entry,
            expect_test_result_label,
            pre_connect_enabled_check,
            pre_connect_command_entry,
            pre_connect_timeout_spin,
            pre_connect_abort_check,
            pre_connect_first_only_check,
            post_disconnect_enabled_check,
            post_disconnect_command_entry,
            post_disconnect_timeout_spin,
            post_disconnect_last_only_check,
        ) = Self::create_automation_combined_tab();
        view_stack
            .add_titled(&automation_tab, Some("automation"), &i18n("Automation"))
            .set_icon_name(Some("system-run-symbolic"));

        let expect_rules: Rc<RefCell<Vec<ExpectRule>>> = Rc::new(RefCell::new(Vec::new()));

        // === Advanced Tab (Display + WOL) ===
        let (
            advanced_tab,
            window_mode_dropdown,
            remember_position_check,
            wol_enabled_check,
            wol_mac_entry,
            wol_broadcast_entry,
            wol_port_spin,
            wol_wait_spin,
        ) = Self::create_advanced_tab();
        view_stack
            .add_titled(&advanced_tab, Some("advanced"), &i18n("Advanced"))
            .set_icon_name(Some("preferences-system-symbolic"));

        // Wire up add variable button
        Self::wire_add_variable_button(&add_variable_button, &variables_list, &variables_rows);

        // Wire up add expect rule button
        Self::wire_add_expect_rule_button(
            &add_expect_rule_button,
            &expect_rules_list,
            &expect_rules,
        );

        // Wire up template picker buttons
        Self::wire_template_buttons(&template_list_box, &expect_rules_list, &expect_rules);

        // Wire up pattern tester
        Self::wire_pattern_tester(
            &expect_pattern_test_entry,
            &expect_test_result_label,
            &expect_rules,
        );

        // Wire up add custom property button
        Self::wire_add_custom_property_button(
            &add_custom_property_button,
            &custom_properties_list,
            &custom_properties,
        );

        let on_save: super::ConnectionCallback = Rc::new(RefCell::new(None));
        let editing_id: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));
        let groups_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(Root)".to_string())]));
        let connections_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
            Rc::new(RefCell::new(vec![(None, "(None)".to_string())]));

        // Connect save button handler
        Self::connect_save_button(
            &save_btn,
            &window,
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
            &ssh_identities_only,
            &ssh_control_master,
            &ssh_agent_forwarding,
            &ssh_waypipe,
            &ssh_x11_forwarding,
            &ssh_compression,
            &ssh_startup_entry,
            &ssh_options_entry,
            &ssh_port_forwards,
            &rdp_client_mode_dropdown,
            &rdp_performance_mode_dropdown,
            &rdp_width_spin,
            &rdp_height_spin,
            &rdp_color_dropdown,
            &rdp_scale_override_dropdown,
            &rdp_audio_check,
            &rdp_gateway_entry,
            &rdp_shared_folders,
            &rdp_custom_args_entry,
            &rdp_keyboard_layout_dropdown,
            &vnc_client_mode_dropdown,
            &vnc_performance_mode_dropdown,
            &vnc_encoding_entry,
            &vnc_compression_spin,
            &vnc_quality_spin,
            &vnc_view_only_check,
            &vnc_scaling_check,
            &vnc_clipboard_check,
            &vnc_scale_override_dropdown,
            &vnc_custom_args_entry,
            &spice_tls_check,
            &spice_ca_cert_entry,
            &spice_skip_verify_check,
            &spice_usb_check,
            &spice_clipboard_check,
            &spice_compression_dropdown,
            &spice_shared_folders,
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
            &zt_cf_hostname_entry,
            &zt_teleport_host_entry,
            &zt_teleport_cluster_entry,
            &zt_tailscale_host_entry,
            &zt_boundary_target_entry,
            &zt_boundary_addr_entry,
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
            &variables_rows,
            &logging_tab_struct,
            &expect_rules,
            &pre_connect_enabled_check,
            &pre_connect_command_entry,
            &pre_connect_timeout_spin,
            &pre_connect_abort_check,
            &pre_connect_first_only_check,
            &post_disconnect_enabled_check,
            &post_disconnect_command_entry,
            &post_disconnect_timeout_spin,
            &post_disconnect_last_only_check,
            &window_mode_dropdown,
            &remember_position_check,
            &custom_properties,
            &wol_enabled_check,
            &wol_mac_entry,
            &wol_broadcast_entry,
            &wol_port_spin,
            &wol_wait_spin,
            &connections_data,
        );

        let result = Self {
            window,
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
            password_row,
            variable_dropdown,
            variable_row,
            group_dropdown,
            groups_data: Rc::new(RefCell::new(vec![(None, "(Root)".to_string())])),
            full_groups_data: Rc::new(RefCell::new(HashMap::new())),
            ssh_auth_dropdown,
            ssh_key_source_dropdown,
            ssh_key_entry,
            ssh_key_button,
            ssh_agent_key_dropdown,
            ssh_agent_keys,
            ssh_jump_host_dropdown,
            ssh_proxy_entry,
            ssh_identities_only,
            ssh_control_master,
            ssh_agent_forwarding,
            ssh_waypipe,
            ssh_x11_forwarding,
            ssh_compression,
            ssh_startup_entry,
            ssh_options_entry,
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
            rdp_shared_folders,
            rdp_shared_folders_list,
            rdp_custom_args_entry,
            rdp_keyboard_layout_dropdown,
            vnc_client_mode_dropdown,
            vnc_performance_mode_dropdown,
            vnc_encoding_entry,
            vnc_compression_spin,
            vnc_quality_spin,
            vnc_view_only_check,
            vnc_scaling_check,
            vnc_clipboard_check,
            vnc_scale_override_dropdown,
            vnc_custom_args_entry,
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
            spice_compression_dropdown,
            spice_shared_folders,
            spice_shared_folders_list,
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
            zt_cf_hostname_entry,
            zt_teleport_host_entry,
            zt_teleport_cluster_entry,
            zt_tailscale_host_entry,
            zt_boundary_target_entry,
            zt_boundary_addr_entry,
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
            expect_rules_list,
            expect_rules,
            add_expect_rule_button,
            expect_pattern_test_entry,
            expect_test_result_label,
            pre_connect_enabled_check,
            pre_connect_command_entry,
            pre_connect_timeout_spin,
            pre_connect_abort_check,
            pre_connect_first_only_check,
            post_disconnect_enabled_check,
            post_disconnect_command_entry,
            post_disconnect_timeout_spin,
            post_disconnect_last_only_check,
            window_mode_dropdown,
            remember_position_check,
            custom_properties_list,
            custom_properties,
            add_custom_property_button,
            wol_enabled_check,
            wol_mac_entry,
            wol_broadcast_entry,
            wol_port_spin,
            wol_wait_spin,
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
            let window = result.window.clone();

            // Helper to update button sensitivity
            let user_btn_for_update = username_load_button.clone();
            let domain_btn_for_update = domain_load_button.clone();
            let update_buttons = Rc::new(move |selected_idx: u32| {
                let sensitive = selected_idx > 0; // 0 is Root
                #[allow(clippy::cast_possible_truncation)]
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
            let window_clone = window.clone();

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
                            crate::toast::show_toast_on_window(
                                &window_clone,
                                "Username loaded from group",
                                crate::toast::ToastType::Success,
                            );
                            return;
                        }
                        crate::toast::show_toast_on_window(
                            &window_clone,
                            "Group has no username defined",
                            crate::toast::ToastType::Info,
                        );
                    }
                }
            });

            // Connect Domain Load Button
            let group_dropdown_clone = group_dropdown.clone();
            let groups_data_clone = groups_data.clone();
            let full_groups_data_clone = full_groups_data.clone();
            let domain_entry_clone = domain_entry.clone();
            let window_clone = window.clone();

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
                            crate::toast::show_toast_on_window(
                                &window_clone,
                                "Domain loaded from group",
                                crate::toast::ToastType::Success,
                            );
                            return;
                        }
                        crate::toast::show_toast_on_window(
                            &window_clone,
                            "Group has no domain defined",
                            crate::toast::ToastType::Info,
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
        let window = result.window.clone();

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
            #[allow(clippy::cast_sign_loss)]
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
                                "Connection Test Successful",
                                &format!("Connection successful! Latency: {}ms", latency),
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

    /// Creates the main window with header bar containing Save button
    fn create_window_with_header(
        parent: Option<&gtk4::Window>,
    ) -> (adw::Window, adw::HeaderBar, Button, Button) {
        let window = adw::Window::builder()
            .title(i18n("New Connection"))
            .modal(true)
            .default_width(600)
            .default_height(500)
            .build();
        window.set_size_request(350, 300);

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar with Close/Test/Create buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let close_btn = Button::builder().label(i18n("Close")).build();
        let test_btn = Button::builder()
            .label(i18n("Test"))
            .tooltip_text(i18n("Test connection"))
            .build();
        let save_btn = Button::builder()
            .label(i18n("Create"))
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&save_btn);
        header.pack_end(&test_btn);

        // Close button handler
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        (window, header, save_btn, test_btn)
    }

    /// Creates the view stack widget and adds it to the window with view switcher bar
    fn create_view_stack(window: &adw::Window, header: &adw::HeaderBar) -> adw::ViewStack {
        let view_stack = adw::ViewStack::new();

        // Create view switcher bar for the bottom of the window
        let view_switcher_bar = adw::ViewSwitcherBar::builder()
            .stack(&view_stack)
            .reveal(true)
            .build();

        // Create scrolled window for the stack content
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .child(&view_stack)
            .build();

        // Use GtkBox with HeaderBar, content, and ViewSwitcherBar at bottom
        let main_box = GtkBox::new(Orientation::Vertical, 0);
        main_box.append(header);
        main_box.append(&scrolled);
        main_box.append(&view_switcher_bar);
        window.set_content(Some(&main_box));

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
    #[allow(clippy::too_many_arguments)]
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
            ];
            let selected = dropdown.selected() as usize;
            if selected < protocols.len() {
                let protocol_id = protocols[selected];
                // SFTP reuses SSH config tab
                let stack_name = if protocol_id == "sftp" {
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
                let hide_network = is_zerotrust || is_serial || is_kubernetes;
                let visible = !hide_network;

                host_entry.set_visible(visible);
                host_label.set_visible(visible);
                port_clone.set_visible(visible);
                port_label.set_visible(visible);
                username_entry.set_visible(visible);
                username_label.set_visible(visible);
                tags_entry.set_visible(!is_zerotrust);
                tags_label.set_visible(!is_zerotrust);
                password_source_dropdown.set_visible(visible);
                password_source_label.set_visible(visible);
                // Password row visibility controlled by password_source_dropdown
                if hide_network {
                    password_row.set_visible(false);
                }
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
            _ => 22.0, // ssh, sftp
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
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn connect_save_button(
        save_btn: &Button,
        window: &adw::Window,
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
        ssh_identities_only: &CheckButton,
        ssh_control_master: &CheckButton,
        ssh_agent_forwarding: &CheckButton,
        ssh_waypipe: &CheckButton,
        ssh_x11_forwarding: &CheckButton,
        ssh_compression: &CheckButton,
        ssh_startup_entry: &Entry,
        ssh_options_entry: &Entry,
        ssh_port_forwards: &Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
        rdp_client_mode_dropdown: &DropDown,
        rdp_performance_mode_dropdown: &DropDown,
        rdp_width_spin: &SpinButton,
        rdp_height_spin: &SpinButton,
        rdp_color_dropdown: &DropDown,
        rdp_scale_override_dropdown: &DropDown,
        rdp_audio_check: &CheckButton,
        rdp_gateway_entry: &Entry,
        rdp_shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
        rdp_custom_args_entry: &Entry,
        rdp_keyboard_layout_dropdown: &DropDown,
        vnc_client_mode_dropdown: &DropDown,
        vnc_performance_mode_dropdown: &DropDown,
        vnc_encoding_entry: &Entry,
        vnc_compression_spin: &SpinButton,
        vnc_quality_spin: &SpinButton,
        vnc_view_only_check: &CheckButton,
        vnc_scaling_check: &CheckButton,
        vnc_clipboard_check: &CheckButton,
        vnc_scale_override_dropdown: &DropDown,
        vnc_custom_args_entry: &Entry,
        spice_tls_check: &CheckButton,
        spice_ca_cert_entry: &Entry,
        spice_skip_verify_check: &CheckButton,
        spice_usb_check: &CheckButton,
        spice_clipboard_check: &CheckButton,
        spice_compression_dropdown: &DropDown,
        spice_shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
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
        zt_cf_hostname_entry: &adw::EntryRow,
        zt_teleport_host_entry: &adw::EntryRow,
        zt_teleport_cluster_entry: &adw::EntryRow,
        zt_tailscale_host_entry: &adw::EntryRow,
        zt_boundary_target_entry: &adw::EntryRow,
        zt_boundary_addr_entry: &adw::EntryRow,
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
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
        logging_tab: &logging_tab::LoggingTab,
        expect_rules: &Rc<RefCell<Vec<ExpectRule>>>,
        pre_connect_enabled_check: &CheckButton,
        pre_connect_command_entry: &Entry,
        pre_connect_timeout_spin: &SpinButton,
        pre_connect_abort_check: &CheckButton,
        pre_connect_first_only_check: &CheckButton,
        post_disconnect_enabled_check: &CheckButton,
        post_disconnect_command_entry: &Entry,
        post_disconnect_timeout_spin: &SpinButton,
        post_disconnect_last_only_check: &CheckButton,
        window_mode_dropdown: &DropDown,
        remember_position_check: &CheckButton,
        custom_properties: &Rc<RefCell<Vec<CustomProperty>>>,
        wol_enabled_check: &CheckButton,
        wol_mac_entry: &Entry,
        wol_broadcast_entry: &Entry,
        wol_port_spin: &SpinButton,
        wol_wait_spin: &SpinButton,
        connections_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    ) {
        let window = window.clone();
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
        let ssh_identities_only = ssh_identities_only.clone();
        let ssh_control_master = ssh_control_master.clone();
        let ssh_agent_forwarding = ssh_agent_forwarding.clone();
        let ssh_waypipe = ssh_waypipe.clone();
        let ssh_x11_forwarding = ssh_x11_forwarding.clone();
        let ssh_compression = ssh_compression.clone();
        let ssh_startup_entry = ssh_startup_entry.clone();
        let ssh_options_entry = ssh_options_entry.clone();
        let ssh_port_forwards = ssh_port_forwards.clone();
        let rdp_client_mode_dropdown = rdp_client_mode_dropdown.clone();
        let rdp_width_spin = rdp_width_spin.clone();
        let rdp_height_spin = rdp_height_spin.clone();
        let rdp_color_dropdown = rdp_color_dropdown.clone();
        let rdp_scale_override_dropdown = rdp_scale_override_dropdown.clone();
        let rdp_audio_check = rdp_audio_check.clone();
        let rdp_gateway_entry = rdp_gateway_entry.clone();
        let rdp_shared_folders = rdp_shared_folders.clone();
        let rdp_custom_args_entry = rdp_custom_args_entry.clone();
        let rdp_keyboard_layout_dropdown = rdp_keyboard_layout_dropdown.clone();
        let rdp_performance_mode_dropdown = rdp_performance_mode_dropdown.clone();
        let vnc_client_mode_dropdown = vnc_client_mode_dropdown.clone();
        let vnc_encoding_entry = vnc_encoding_entry.clone();
        let vnc_compression_spin = vnc_compression_spin.clone();
        let vnc_quality_spin = vnc_quality_spin.clone();
        let vnc_view_only_check = vnc_view_only_check.clone();
        let vnc_scaling_check = vnc_scaling_check.clone();
        let vnc_clipboard_check = vnc_clipboard_check.clone();
        let vnc_scale_override_dropdown = vnc_scale_override_dropdown.clone();
        let vnc_custom_args_entry = vnc_custom_args_entry.clone();
        let vnc_performance_mode_dropdown = vnc_performance_mode_dropdown.clone();
        let spice_tls_check = spice_tls_check.clone();
        let spice_ca_cert_entry = spice_ca_cert_entry.clone();
        let spice_skip_verify_check = spice_skip_verify_check.clone();
        let spice_usb_check = spice_usb_check.clone();
        let spice_clipboard_check = spice_clipboard_check.clone();
        let spice_compression_dropdown = spice_compression_dropdown.clone();
        let spice_shared_folders = spice_shared_folders.clone();
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
        let zt_cf_hostname_entry = zt_cf_hostname_entry.clone();
        let zt_teleport_host_entry = zt_teleport_host_entry.clone();
        let zt_teleport_cluster_entry = zt_teleport_cluster_entry.clone();
        let zt_tailscale_host_entry = zt_tailscale_host_entry.clone();
        let zt_boundary_target_entry = zt_boundary_target_entry.clone();
        let zt_boundary_addr_entry = zt_boundary_addr_entry.clone();
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
        let variables_rows = variables_rows.clone();
        let logging_enabled_check = logging_tab.enabled_check.clone();
        let logging_path_entry = logging_tab.path_entry.clone();
        let logging_timestamp_dropdown = logging_tab.timestamp_dropdown.clone();
        let logging_max_size_spin = logging_tab.max_size_spin.clone();
        let logging_retention_spin = logging_tab.retention_spin.clone();
        let expect_rules = expect_rules.clone();
        let pre_connect_enabled_check = pre_connect_enabled_check.clone();
        let pre_connect_command_entry = pre_connect_command_entry.clone();
        let pre_connect_timeout_spin = pre_connect_timeout_spin.clone();
        let pre_connect_abort_check = pre_connect_abort_check.clone();
        let pre_connect_first_only_check = pre_connect_first_only_check.clone();
        let post_disconnect_enabled_check = post_disconnect_enabled_check.clone();
        let post_disconnect_command_entry = post_disconnect_command_entry.clone();
        let post_disconnect_timeout_spin = post_disconnect_timeout_spin.clone();
        let post_disconnect_last_only_check = post_disconnect_last_only_check.clone();
        let window_mode_dropdown = window_mode_dropdown.clone();
        let remember_position_check = remember_position_check.clone();
        let custom_properties = custom_properties.clone();
        let wol_enabled_check = wol_enabled_check.clone();
        let wol_mac_entry = wol_mac_entry.clone();
        let wol_broadcast_entry = wol_broadcast_entry.clone();
        let wol_port_spin = wol_port_spin.clone();
        let wol_wait_spin = wol_wait_spin.clone();
        let editing_id = editing_id.clone();
        let connections_data = connections_data.clone();

        save_btn.connect_clicked(move |_| {
            let local_variables = Self::collect_local_variables(&variables_rows);
            let collected_expect_rules = expect_rules.borrow().clone();
            let collected_custom_properties = custom_properties.borrow().clone();
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
                ssh_identities_only: &ssh_identities_only,
                ssh_control_master: &ssh_control_master,
                ssh_agent_forwarding: &ssh_agent_forwarding,
                ssh_waypipe: &ssh_waypipe,
                ssh_x11_forwarding: &ssh_x11_forwarding,
                ssh_compression: &ssh_compression,
                ssh_startup_entry: &ssh_startup_entry,
                ssh_options_entry: &ssh_options_entry,
                ssh_port_forwards: &ssh_port_forwards,
                rdp_client_mode_dropdown: &rdp_client_mode_dropdown,
                rdp_width_spin: &rdp_width_spin,
                rdp_height_spin: &rdp_height_spin,
                rdp_color_dropdown: &rdp_color_dropdown,
                rdp_scale_override_dropdown: &rdp_scale_override_dropdown,
                rdp_audio_check: &rdp_audio_check,
                rdp_gateway_entry: &rdp_gateway_entry,
                rdp_shared_folders: &rdp_shared_folders,
                rdp_custom_args_entry: &rdp_custom_args_entry,
                rdp_keyboard_layout_dropdown: &rdp_keyboard_layout_dropdown,
                vnc_client_mode_dropdown: &vnc_client_mode_dropdown,
                vnc_encoding_entry: &vnc_encoding_entry,
                vnc_compression_spin: &vnc_compression_spin,
                vnc_quality_spin: &vnc_quality_spin,
                vnc_view_only_check: &vnc_view_only_check,
                vnc_scaling_check: &vnc_scaling_check,
                vnc_clipboard_check: &vnc_clipboard_check,
                vnc_scale_override_dropdown: &vnc_scale_override_dropdown,
                vnc_custom_args_entry: &vnc_custom_args_entry,
                spice_tls_check: &spice_tls_check,
                spice_ca_cert_entry: &spice_ca_cert_entry,
                spice_skip_verify_check: &spice_skip_verify_check,
                spice_usb_check: &spice_usb_check,
                spice_clipboard_check: &spice_clipboard_check,
                spice_compression_dropdown: &spice_compression_dropdown,
                spice_shared_folders: &spice_shared_folders,
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
                zt_cf_hostname_entry: &zt_cf_hostname_entry,
                zt_teleport_host_entry: &zt_teleport_host_entry,
                zt_teleport_cluster_entry: &zt_teleport_cluster_entry,
                zt_tailscale_host_entry: &zt_tailscale_host_entry,
                zt_boundary_target_entry: &zt_boundary_target_entry,
                zt_boundary_addr_entry: &zt_boundary_addr_entry,
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
                local_variables: &local_variables,
                logging_tab: &logging_tab::LoggingTab {
                    enabled_check: logging_enabled_check.clone(),
                    path_entry: logging_path_entry.clone(),
                    timestamp_dropdown: logging_timestamp_dropdown.clone(),
                    max_size_spin: logging_max_size_spin.clone(),
                    retention_spin: logging_retention_spin.clone(),
                },
                expect_rules: &collected_expect_rules,
                pre_connect_enabled_check: &pre_connect_enabled_check,
                pre_connect_command_entry: &pre_connect_command_entry,
                pre_connect_timeout_spin: &pre_connect_timeout_spin,
                pre_connect_abort_check: &pre_connect_abort_check,
                pre_connect_first_only_check: &pre_connect_first_only_check,
                post_disconnect_enabled_check: &post_disconnect_enabled_check,
                post_disconnect_command_entry: &post_disconnect_command_entry,
                post_disconnect_timeout_spin: &post_disconnect_timeout_spin,
                post_disconnect_last_only_check: &post_disconnect_last_only_check,
                window_mode_dropdown: &window_mode_dropdown,
                remember_position_check: &remember_position_check,
                custom_properties: &collected_custom_properties,
                wol_enabled_check: &wol_enabled_check,
                wol_mac_entry: &wol_mac_entry,
                wol_broadcast_entry: &wol_broadcast_entry,
                wol_port_spin: &wol_port_spin,
                wol_wait_spin: &wol_wait_spin,
                rdp_performance_mode_dropdown: &rdp_performance_mode_dropdown,
                vnc_performance_mode_dropdown: &vnc_performance_mode_dropdown,
                editing_id: &editing_id,
            };

            if let Err(err) = data.validate() {
                crate::toast::show_toast_on_window(&window, &err, crate::toast::ToastType::Warning);
                return;
            }

            if let Some(result) = data.build_connection() {
                // Password saving is handled by the caller (edit_dialogs,
                // connection_dialogs, templates) after the on_save callback
                // to avoid duplicate vault writes.

                if let Some(ref cb) = *on_save.borrow() {
                    cb(Some(result));
                }
                window.close();
            }
        });
    }

    #[allow(clippy::type_complexity)]
    fn create_basic_tab() -> (
        GtkBox,
        Entry,
        Entry,
        TextView,
        Entry,
        Label,
        SpinButton,
        Label,
        Entry,
        Label,
        Entry,
        Label,
        Entry,
        Label,
        DropDown,
        DropDown,
        Label,
        Entry,
        Label,
        Button,
        Button,
        GtkBox,
        DropDown,
        GtkBox,
        DropDown,
        Button,
        Button,
    ) {
        let vbox = GtkBox::new(Orientation::Vertical, 8);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);

        let grid = Grid::builder().row_spacing(8).column_spacing(12).build();
        vbox.append(&grid);

        let mut row = 0;

        // Name
        let name_label = Label::builder()
            .label(i18n("Name:"))
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .placeholder_text(i18n("Connection name"))
            .hexpand(true)
            .build();
        grid.attach(&name_label, 0, row, 1, 1);
        grid.attach(&name_entry, 1, row, 2, 1);
        row += 1;

        // Icon (emoji or GTK icon name)
        let icon_label = Label::builder()
            .label(i18n("Icon:"))
            .halign(gtk4::Align::End)
            .build();
        let icon_entry = Entry::builder()
            .placeholder_text(i18n("Emoji or icon name (optional)"))
            .hexpand(true)
            .max_width_chars(30)
            .build();
        icon_entry.set_tooltip_text(Some(&i18n(
            "Enter an emoji (e.g. 🇺🇦) or GTK icon name (e.g. starred-symbolic)",
        )));
        grid.attach(&icon_label, 0, row, 1, 1);
        grid.attach(&icon_entry, 1, row, 2, 1);
        row += 1;

        // Protocol
        let protocol_label_grid = Label::builder()
            .label(i18n("Protocol:"))
            .halign(gtk4::Align::End)
            .build();
        let protocol_items: Vec<String> = vec![
            "SSH".to_string(),
            "RDP".to_string(),
            "VNC".to_string(),
            "SPICE".to_string(),
            i18n("Zero Trust"),
            "Telnet".to_string(),
            "Serial".to_string(),
            "SFTP".to_string(),
            "Kubernetes".to_string(),
        ];
        let protocol_strs: Vec<&str> = protocol_items.iter().map(String::as_str).collect();
        let protocol_list = StringList::new(&protocol_strs);
        let protocol_dropdown = DropDown::builder().model(&protocol_list).build();
        protocol_dropdown.set_selected(0);
        grid.attach(&protocol_label_grid, 0, row, 1, 1);
        grid.attach(&protocol_dropdown, 1, row, 2, 1);
        row += 1;

        // Host
        let host_label = Label::builder()
            .label(i18n("Host:"))
            .halign(gtk4::Align::End)
            .build();
        let host_entry = Entry::builder()
            .placeholder_text(i18n("hostname or IP"))
            .hexpand(true)
            .build();
        grid.attach(&host_label, 0, row, 1, 1);
        grid.attach(&host_entry, 1, row, 2, 1);
        row += 1;

        // Port with description
        let port_label = Label::builder()
            .label(i18n("Port:"))
            .halign(gtk4::Align::End)
            .build();
        let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_spin = SpinButton::builder()
            .adjustment(&port_adj)
            .climb_rate(1.0)
            .digits(0)
            .build();
        let port_desc = Label::builder()
            .label(i18n("SSH, Well-Known"))
            .css_classes(["dim-label"])
            .build();
        let port_box = GtkBox::new(Orientation::Horizontal, 8);
        port_box.append(&port_spin);
        port_box.append(&port_desc);
        grid.attach(&port_label, 0, row, 1, 1);
        grid.attach(&port_box, 1, row, 2, 1);
        row += 1;

        // Update port description when port changes
        let port_desc_clone = port_desc.clone();
        port_spin.connect_value_changed(move |spin| {
            #[allow(clippy::cast_sign_loss)]
            let port = spin.value() as u16;
            let desc = Self::get_port_description(port);
            port_desc_clone.set_label(&desc);
        });

        // Username
        let username_label = Label::builder()
            .label(i18n("Username:"))
            .halign(gtk4::Align::End)
            .build();
        let current_user = std::env::var("USER").unwrap_or_default();
        let username_entry = Entry::builder()
            .placeholder_text(&format!("(default: {current_user})"))
            .hexpand(true)
            .build();
        grid.attach(&username_label, 0, row, 1, 1);

        let username_load_button = Button::builder()
            .icon_name("folder-download-symbolic")
            .tooltip_text(i18n("Load from selected group"))
            .sensitive(false)
            .build();
        let username_box = GtkBox::new(Orientation::Horizontal, 4);
        username_box.append(&username_entry);
        username_box.append(&username_load_button);

        grid.attach(&username_box, 1, row, 2, 1);
        row += 1;

        // Domain (for RDP/Windows authentication)
        let domain_label = Label::builder()
            .label(i18n("Domain:"))
            .halign(gtk4::Align::End)
            .build();
        let domain_entry = Entry::builder()
            .placeholder_text(i18n("Optional (e.g., WORKGROUP)"))
            .hexpand(true)
            .build();
        grid.attach(&domain_label, 0, row, 1, 1);

        let domain_load_button = Button::builder()
            .icon_name("folder-download-symbolic")
            .tooltip_text(i18n("Load from selected group"))
            .sensitive(false)
            .build();
        let domain_box = GtkBox::new(Orientation::Horizontal, 4);
        domain_box.append(&domain_entry);
        domain_box.append(&domain_load_button);

        grid.attach(&domain_box, 1, row, 2, 1);
        row += 1;

        // Password Source
        let password_source_label = Label::builder()
            .label(i18n("Password:"))
            .halign(gtk4::Align::End)
            .build();
        let pw_src_items: Vec<String> = vec![
            i18n("Prompt"),
            i18n("Vault"),
            i18n("Variable"),
            i18n("Inherit"),
            i18n("None"),
        ];
        let pw_src_strs: Vec<&str> = pw_src_items.iter().map(String::as_str).collect();
        let password_source_list = StringList::new(&pw_src_strs);
        let password_source_dropdown = DropDown::builder().model(&password_source_list).build();
        password_source_dropdown.set_selected(0);
        grid.attach(&password_source_label, 0, row, 1, 1);
        grid.attach(&password_source_dropdown, 1, row, 2, 1);
        row += 1;

        // Password with visibility toggle - use grid row for proper alignment
        let password_entry_label = Label::builder()
            .label(i18n("Value:"))
            .halign(gtk4::Align::End)
            .build();
        let password_entry = Entry::builder()
            .placeholder_text(i18n("Password value"))
            .hexpand(true)
            .visibility(false)
            .build();
        let password_visibility_button = Button::builder()
            .icon_name("view-reveal-symbolic")
            .tooltip_text(i18n("Show/hide password"))
            .build();
        let password_load_button = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(i18n("Load password from vault"))
            .build();
        let password_box = GtkBox::new(Orientation::Horizontal, 4);
        password_box.append(&password_entry);
        password_box.append(&password_visibility_button);
        password_box.append(&password_load_button);
        password_box.set_hexpand(true);

        // Password row container - wraps label and entry box for show/hide
        let password_row = GtkBox::new(Orientation::Horizontal, 0);
        password_row.set_visible(false);
        // Attach label and password box to grid for proper alignment
        grid.attach(&password_entry_label, 0, row, 1, 1);
        grid.attach(&password_box, 1, row, 2, 1);
        // Bind visibility of label and box to password_row visibility
        password_row
            .bind_property("visible", &password_entry_label, "visible")
            .sync_create()
            .build();
        password_row
            .bind_property("visible", &password_box, "visible")
            .sync_create()
            .build();
        row += 1;

        // Variable name dropdown — shown when password source is Variable
        let variable_label = Label::builder()
            .label(i18n("Variable:"))
            .halign(gtk4::Align::End)
            .build();
        let variable_name_list = StringList::new(&[]);
        let variable_dropdown = DropDown::builder().model(&variable_name_list).build();
        let variable_row = GtkBox::new(Orientation::Horizontal, 0);
        variable_row.set_visible(false);
        grid.attach(&variable_label, 0, row, 1, 1);
        grid.attach(&variable_dropdown, 1, row, 2, 1);
        variable_row
            .bind_property("visible", &variable_label, "visible")
            .sync_create()
            .build();
        variable_row
            .bind_property("visible", &variable_dropdown, "visible")
            .sync_create()
            .build();
        row += 1;

        // Tags
        let tags_label = Label::builder()
            .label(i18n("Tags:"))
            .halign(gtk4::Align::End)
            .build();
        let tags_entry = Entry::builder()
            .placeholder_text(i18n("tag1, tag2, ..."))
            .hexpand(true)
            .build();
        grid.attach(&tags_label, 0, row, 1, 1);
        grid.attach(&tags_entry, 1, row, 2, 1);
        row += 1;

        // Group
        let group_label = Label::builder()
            .label(i18n("Group:"))
            .halign(gtk4::Align::End)
            .build();
        let group_items: Vec<String> = vec![i18n("(Root)")];
        let group_strs: Vec<&str> = group_items.iter().map(String::as_str).collect();
        let group_list = StringList::new(&group_strs);
        let group_dropdown = DropDown::builder().model(&group_list).build();
        grid.attach(&group_label, 0, row, 1, 1);
        grid.attach(&group_dropdown, 1, row, 2, 1);
        row += 1;

        // Description
        let desc_label = Label::builder()
            .label(i18n("Description:"))
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Start)
            .build();
        let description_view = TextView::builder()
            .hexpand(true)
            .vexpand(false)
            .wrap_mode(WrapMode::Word)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();
        let desc_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(144)
            .hexpand(true)
            .child(&description_view)
            .build();
        grid.attach(&desc_label, 0, row, 1, 1);
        grid.attach(&desc_scrolled, 1, row, 2, 1);

        // Accessible label relations for screen readers (A11Y-01)
        icon_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            icon_label.upcast_ref()
        ])]);
        name_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            name_label.upcast_ref()
        ])]);
        protocol_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            protocol_label_grid.upcast_ref(),
        ])]);
        host_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            host_label.upcast_ref()
        ])]);
        port_spin.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            port_label.upcast_ref()
        ])]);
        username_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            username_label.upcast_ref(),
        ])]);
        domain_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            domain_label.upcast_ref()
        ])]);
        password_source_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            password_source_label.upcast_ref(),
        ])]);
        password_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            password_entry_label.upcast_ref(),
        ])]);
        variable_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            variable_label.upcast_ref(),
        ])]);
        tags_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            tags_label.upcast_ref()
        ])]);
        group_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            group_label.upcast_ref()
        ])]);
        description_view.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
            desc_label.upcast_ref()
        ])]);

        (
            vbox,
            name_entry,
            icon_entry,
            description_view,
            host_entry,
            host_label,
            port_spin,
            port_label,
            username_entry,
            username_label,
            domain_entry,
            domain_label,
            tags_entry,
            tags_label,
            protocol_dropdown,
            password_source_dropdown,
            password_source_label,
            password_entry,
            password_entry_label,
            password_visibility_button,
            password_load_button,
            password_row,
            variable_dropdown,
            variable_row,
            group_dropdown,
            username_load_button,
            domain_load_button,
        )
    }

    /// Returns a description for the given port number
    fn get_port_description(port: u16) -> String {
        // Well-known service ports
        let service = match port {
            22 => "SSH",
            23 => "Telnet",
            25 => "SMTP",
            53 => "DNS",
            80 => "HTTP",
            110 => "POP3",
            143 => "IMAP",
            443 => "HTTPS",
            445 => "SMB",
            993 => "IMAPS",
            995 => "POP3S",
            3306 => "MySQL",
            3389 => "RDP",
            5432 => "PostgreSQL",
            5900 => "VNC",
            5901..=5909 => "VNC",
            5985 => "WinRM HTTP",
            5986 => "WinRM HTTPS",
            6379 => "Redis",
            8080 => "HTTP Alt",
            8443 => "HTTPS Alt",
            27017 => "MongoDB",
            _ => "",
        };

        // Port range category
        let range = if port <= 1023 {
            "Well-Known"
        } else if port <= 49151 {
            "Registered"
        } else {
            "Dynamic"
        };

        if service.is_empty() {
            range.to_string()
        } else {
            format!("{service}, {range}")
        }
    }

    fn create_rdp_options() -> (
        GtkBox,
        DropDown,
        DropDown,
        SpinButton,
        SpinButton,
        DropDown,
        DropDown,
        CheckButton,
        Entry,
        Rc<RefCell<Vec<SharedFolder>>>,
        gtk4::ListBox,
        Entry,
        DropDown,
    ) {
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

        // === Display Group ===
        let display_group = adw::PreferencesGroup::builder()
            .title(i18n("Display"))
            .build();

        // Client mode dropdown
        let client_mode_items: Vec<String> = vec![
            i18n(RdpClientMode::Embedded.display_name()),
            i18n(RdpClientMode::External.display_name()),
        ];
        let client_mode_strs: Vec<&str> = client_mode_items.iter().map(String::as_str).collect();
        let client_mode_list = StringList::new(&client_mode_strs);
        let client_mode_dropdown = DropDown::builder()
            .model(&client_mode_list)
            .valign(gtk4::Align::Center)
            .build();

        let client_mode_row = adw::ActionRow::builder()
            .title(i18n("Client Mode"))
            .subtitle(i18n(
                "Embedded renders in tab, External opens separate window",
            ))
            .build();
        client_mode_row.add_suffix(&client_mode_dropdown);
        display_group.add(&client_mode_row);

        // Performance mode dropdown
        let perf_items: Vec<String> = vec![
            i18n(RdpPerformanceMode::Quality.display_name()),
            i18n(RdpPerformanceMode::Balanced.display_name()),
            i18n(RdpPerformanceMode::Speed.display_name()),
        ];
        let perf_strs: Vec<&str> = perf_items.iter().map(String::as_str).collect();
        let performance_mode_list = StringList::new(&perf_strs);
        let performance_mode_dropdown = DropDown::builder()
            .model(&performance_mode_list)
            .valign(gtk4::Align::Center)
            .build();
        performance_mode_dropdown.set_selected(1); // Default to Balanced

        let performance_mode_row = adw::ActionRow::builder()
            .title(i18n("Performance Mode"))
            .subtitle(i18n("Quality/speed tradeoff for image rendering"))
            .build();
        performance_mode_row.add_suffix(&performance_mode_dropdown);
        display_group.add(&performance_mode_row);

        // Resolution
        let res_box = GtkBox::new(Orientation::Horizontal, 4);
        res_box.set_valign(gtk4::Align::Center);
        let width_adj = gtk4::Adjustment::new(1920.0, 640.0, 7680.0, 1.0, 100.0, 0.0);
        let width_spin = SpinButton::builder()
            .adjustment(&width_adj)
            .climb_rate(1.0)
            .digits(0)
            .build();
        let x_label = Label::new(Some("×"));
        let height_adj = gtk4::Adjustment::new(1080.0, 480.0, 4320.0, 1.0, 100.0, 0.0);
        let height_spin = SpinButton::builder()
            .adjustment(&height_adj)
            .climb_rate(1.0)
            .digits(0)
            .build();
        res_box.append(&width_spin);
        res_box.append(&x_label);
        res_box.append(&height_spin);

        let resolution_row = adw::ActionRow::builder()
            .title(i18n("Resolution"))
            .subtitle(i18n("Width × Height in pixels"))
            .build();
        resolution_row.add_suffix(&res_box);
        display_group.add(&resolution_row);

        // Color depth
        let color_items: Vec<String> = vec![
            i18n("32-bit (True Color)"),
            i18n("24-bit"),
            i18n("16-bit (High Color)"),
            i18n("15-bit"),
            i18n("8-bit"),
        ];
        let color_strs: Vec<&str> = color_items.iter().map(String::as_str).collect();
        let color_list = StringList::new(&color_strs);
        let color_dropdown = DropDown::new(Some(color_list), gtk4::Expression::NONE);
        color_dropdown.set_selected(0);
        color_dropdown.set_valign(gtk4::Align::Center);

        let color_row = adw::ActionRow::builder()
            .title(i18n("Color Depth"))
            .subtitle(i18n("Higher values provide better quality"))
            .build();
        color_row.add_suffix(&color_dropdown);
        display_group.add(&color_row);

        // Scale override dropdown (for embedded mode)
        let scale_items: Vec<&str> = ScaleOverride::all()
            .iter()
            .map(|s| s.display_name())
            .collect();
        let scale_list = StringList::new(&scale_items);
        let scale_override_dropdown = DropDown::builder()
            .model(&scale_list)
            .valign(gtk4::Align::Center)
            .build();
        let scale_row = adw::ActionRow::builder()
            .title(i18n("Display Scale"))
            .subtitle(i18n("Override HiDPI scaling for embedded viewer"))
            .build();
        scale_row.add_suffix(&scale_override_dropdown);
        display_group.add(&scale_row);

        // Connect client mode dropdown to show/hide resolution/color/scale rows
        // Embedded (0) - hide resolution and color depth (dynamic resolution)
        // External (1) - show resolution and color depth
        let resolution_row_clone = resolution_row.clone();
        let color_row_clone = color_row.clone();
        let scale_row_clone = scale_row.clone();
        client_mode_dropdown.connect_selected_notify(move |dropdown| {
            let is_embedded = dropdown.selected() == 0;
            resolution_row_clone.set_visible(!is_embedded);
            color_row_clone.set_visible(!is_embedded);
            scale_row_clone.set_visible(is_embedded);
        });

        // Set initial state (Embedded - hide resolution/color, show scale)
        resolution_row.set_visible(false);
        color_row.set_visible(false);
        scale_row.set_visible(true);

        content.append(&display_group);

        // === Features Group ===
        let features_group = adw::PreferencesGroup::builder()
            .title(i18n("Features"))
            .build();

        // Audio redirect
        let audio_check = CheckButton::new();
        let audio_row = adw::ActionRow::builder()
            .title(i18n("Audio Redirection"))
            .subtitle(i18n("Play remote audio locally"))
            .activatable_widget(&audio_check)
            .build();
        audio_row.add_suffix(&audio_check);
        features_group.add(&audio_row);

        // Gateway
        let gateway_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("gateway.example.com"))
            .valign(gtk4::Align::Center)
            .build();

        let gateway_row = adw::ActionRow::builder()
            .title(i18n("RDP Gateway"))
            .subtitle(i18n("Remote Desktop Gateway server"))
            .build();
        gateway_row.add_suffix(&gateway_entry);
        features_group.add(&gateway_row);

        content.append(&features_group);

        // === Shared Folders Group ===
        let folders_group = adw::PreferencesGroup::builder()
            .title(i18n("Shared Folders"))
            .description(i18n("Local folders accessible from remote session"))
            .build();

        let folders_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        folders_list.set_placeholder(Some(&Label::new(Some(&i18n("No shared folders")))));

        let folders_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(80)
            .max_content_height(120)
            .child(&folders_list)
            .build();
        folders_group.add(&folders_scrolled);

        let folders_buttons = GtkBox::new(Orientation::Horizontal, 8);
        folders_buttons.set_halign(gtk4::Align::End);
        folders_buttons.set_margin_top(8);
        let add_folder_btn = Button::builder()
            .label(i18n("Add"))
            .css_classes(["suggested-action"])
            .build();
        let remove_folder_btn = Button::builder()
            .label(i18n("Remove"))
            .sensitive(false)
            .build();
        folders_buttons.append(&add_folder_btn);
        folders_buttons.append(&remove_folder_btn);
        folders_group.add(&folders_buttons);

        content.append(&folders_group);

        let shared_folders: Rc<RefCell<Vec<SharedFolder>>> = Rc::new(RefCell::new(Vec::new()));

        // Connect add folder button
        Self::connect_add_folder_button(&add_folder_btn, &folders_list, &shared_folders);

        // Connect remove folder button
        Self::connect_remove_folder_button(&remove_folder_btn, &folders_list, &shared_folders);

        // Enable/disable remove button based on selection
        let remove_btn_for_selection = remove_folder_btn;
        folders_list.connect_row_selected(move |_, row| {
            remove_btn_for_selection.set_sensitive(row.is_some());
        });

        // === Advanced Group ===
        let advanced_group = adw::PreferencesGroup::builder()
            .title(i18n("Advanced"))
            .build();

        // Keyboard layout dropdown
        let kb_items: Vec<String> = vec![
            i18n("Auto (detect)"),
            i18n("US English"),
            i18n("German (de)"),
            i18n("French (fr)"),
            i18n("Spanish (es)"),
            i18n("Italian (it)"),
            i18n("Portuguese (pt)"),
            i18n("Brazilian (br)"),
            i18n("UK English (gb)"),
            i18n("Swiss German (ch)"),
            i18n("Austrian (at)"),
            i18n("Belgian (be)"),
            i18n("Dutch (nl)"),
            i18n("Swedish (se)"),
            i18n("Norwegian (no)"),
            i18n("Danish (dk)"),
            i18n("Finnish (fi)"),
            i18n("Polish (pl)"),
            i18n("Czech (cz)"),
            i18n("Slovak (sk)"),
            i18n("Hungarian (hu)"),
            i18n("Romanian (ro)"),
            i18n("Croatian (hr)"),
            i18n("Slovenian (si)"),
            i18n("Serbian (rs)"),
            i18n("Bulgarian (bg)"),
            i18n("Russian (ru)"),
            i18n("Ukrainian (ua)"),
            i18n("Turkish (tr)"),
            i18n("Greek (gr)"),
            i18n("Japanese (jp)"),
            i18n("Korean (kr)"),
        ];
        let kb_strs: Vec<&str> = kb_items.iter().map(String::as_str).collect();
        let kb_layout_list = StringList::new(&kb_strs);
        let kb_layout_dropdown = DropDown::builder()
            .model(&kb_layout_list)
            .valign(gtk4::Align::Center)
            .build();
        let kb_layout_row = adw::ActionRow::builder()
            .title(i18n("Keyboard Layout"))
            .subtitle(i18n("Layout sent to RDP server (Auto uses system setting)"))
            .build();
        kb_layout_row.add_suffix(&kb_layout_dropdown);
        advanced_group.add(&kb_layout_row);

        let args_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Additional command-line arguments"))
            .valign(gtk4::Align::Center)
            .build();

        let args_row = adw::ActionRow::builder()
            .title(i18n("Custom Arguments"))
            .subtitle(i18n("Extra FreeRDP command-line options"))
            .build();
        args_row.add_suffix(&args_entry);
        advanced_group.add(&args_row);

        content.append(&advanced_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            client_mode_dropdown,
            performance_mode_dropdown,
            width_spin,
            height_spin,
            color_dropdown,
            scale_override_dropdown,
            audio_check,
            gateway_entry,
            shared_folders,
            folders_list,
            args_entry,
            kb_layout_dropdown,
        )
    }

    /// Connects the add folder button to show file dialog and add folder
    fn connect_add_folder_button(
        add_btn: &Button,
        folders_list: &gtk4::ListBox,
        shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
    ) {
        let folders_list_clone = folders_list.clone();
        let shared_folders_clone = shared_folders.clone();
        add_btn.connect_clicked(move |btn| {
            let file_dialog = FileDialog::builder()
                .title(i18n("Select Folder to Share"))
                .modal(true)
                .build();

            let folders_list = folders_list_clone.clone();
            let shared_folders = shared_folders_clone.clone();
            let parent = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

            file_dialog.select_folder(
                parent.as_ref(),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        let share_name = path.file_name().map_or_else(
                            || "Share".to_string(),
                            |n| n.to_string_lossy().to_string(),
                        );

                        let folder = SharedFolder {
                            local_path: path.clone(),
                            share_name: share_name.clone(),
                        };

                        shared_folders.borrow_mut().push(folder);
                        Self::add_folder_row_to_list(&folders_list, &path, &share_name);
                    }
                },
            );
        });
    }

    /// Adds a folder row to the list UI
    fn add_folder_row_to_list(
        folders_list: &gtk4::ListBox,
        path: &std::path::Path,
        share_name: &str,
    ) {
        let row_box = GtkBox::new(Orientation::Horizontal, 8);
        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);

        let path_label = Label::builder()
            .label(path.to_string_lossy().as_ref())
            .hexpand(true)
            .halign(gtk4::Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .build();
        let name_label = Label::builder()
            .label(format!("→ {share_name}"))
            .halign(gtk4::Align::End)
            .build();

        row_box.append(&path_label);
        row_box.append(&name_label);
        folders_list.append(&row_box);
    }

    /// Connects the remove folder button
    fn connect_remove_folder_button(
        remove_btn: &Button,
        folders_list: &gtk4::ListBox,
        shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
    ) {
        let folders_list_clone = folders_list.clone();
        let shared_folders_clone = shared_folders.clone();
        remove_btn.connect_clicked(move |_| {
            if let Some(selected_row) = folders_list_clone.selected_row()
                && let Ok(index) = usize::try_from(selected_row.index())
                && index < shared_folders_clone.borrow().len()
            {
                shared_folders_clone.borrow_mut().remove(index);
                folders_list_clone.remove(&selected_row);
            }
        });
    }

    /// Creates the VNC options panel using libadwaita components following GNOME HIG.
    #[allow(clippy::type_complexity)]
    fn create_vnc_options() -> (
        GtkBox,
        DropDown,
        DropDown,
        Entry,
        SpinButton,
        SpinButton,
        CheckButton,
        CheckButton,
        CheckButton,
        DropDown,
        Entry,
    ) {
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

        // === Display Group ===
        let display_group = adw::PreferencesGroup::builder()
            .title(i18n("Display"))
            .build();

        // Client mode dropdown
        let vnc_mode_items: Vec<String> = vec![
            i18n(VncClientMode::Embedded.display_name()),
            i18n(VncClientMode::External.display_name()),
        ];
        let vnc_mode_strs: Vec<&str> = vnc_mode_items.iter().map(String::as_str).collect();
        let client_mode_list = StringList::new(&vnc_mode_strs);
        let client_mode_dropdown = DropDown::builder()
            .model(&client_mode_list)
            .valign(gtk4::Align::Center)
            .build();

        let client_mode_row = adw::ActionRow::builder()
            .title(i18n("Client Mode"))
            .subtitle(i18n(
                "Embedded renders in tab, External opens separate window",
            ))
            .build();
        client_mode_row.add_suffix(&client_mode_dropdown);
        display_group.add(&client_mode_row);

        // Performance mode dropdown
        let vnc_perf_items: Vec<String> = vec![
            i18n(VncPerformanceMode::Quality.display_name()),
            i18n(VncPerformanceMode::Balanced.display_name()),
            i18n(VncPerformanceMode::Speed.display_name()),
        ];
        let vnc_perf_strs: Vec<&str> = vnc_perf_items.iter().map(String::as_str).collect();
        let performance_mode_list = StringList::new(&vnc_perf_strs);
        let performance_mode_dropdown = DropDown::builder()
            .model(&performance_mode_list)
            .valign(gtk4::Align::Center)
            .build();
        performance_mode_dropdown.set_selected(1); // Default to Balanced

        let performance_mode_row = adw::ActionRow::builder()
            .title(i18n("Performance Mode"))
            .subtitle(i18n("Quality/speed tradeoff for image rendering"))
            .build();
        performance_mode_row.add_suffix(&performance_mode_dropdown);
        display_group.add(&performance_mode_row);

        // Encoding
        let encoding_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("tight, zrle, hextile")
            .valign(gtk4::Align::Center)
            .build();

        let encoding_row = adw::ActionRow::builder()
            .title(i18n("Encoding"))
            .subtitle(i18n("Preferred encoding methods (comma-separated)"))
            .build();
        encoding_row.add_suffix(&encoding_entry);
        display_group.add(&encoding_row);

        // Scale override dropdown (for embedded mode)
        let scale_items: Vec<&str> = ScaleOverride::all()
            .iter()
            .map(|s| s.display_name())
            .collect();
        let scale_list = StringList::new(&scale_items);
        let scale_override_dropdown = DropDown::builder()
            .model(&scale_list)
            .valign(gtk4::Align::Center)
            .build();
        let scale_row = adw::ActionRow::builder()
            .title(i18n("Display Scale"))
            .subtitle(i18n("Override HiDPI scaling for embedded viewer"))
            .build();
        scale_row.add_suffix(&scale_override_dropdown);
        display_group.add(&scale_row);

        // Show scale row only in embedded mode
        let scale_row_clone = scale_row.clone();
        client_mode_dropdown.connect_selected_notify(move |dropdown| {
            let is_embedded = dropdown.selected() == 0;
            scale_row_clone.set_visible(is_embedded);
        });
        scale_row.set_visible(true); // Default: embedded

        content.append(&display_group);

        // === Quality Group ===
        let quality_group = adw::PreferencesGroup::builder()
            .title(i18n("Quality"))
            .build();

        // Compression
        let compression_adj = gtk4::Adjustment::new(6.0, 0.0, 9.0, 1.0, 1.0, 0.0);
        let compression_spin = SpinButton::builder()
            .adjustment(&compression_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .build();

        let compression_row = adw::ActionRow::builder()
            .title(i18n("Compression"))
            .subtitle(i18n("0 (none) to 9 (maximum)"))
            .build();
        compression_row.add_suffix(&compression_spin);
        quality_group.add(&compression_row);

        // Quality
        let quality_adj = gtk4::Adjustment::new(6.0, 0.0, 9.0, 1.0, 1.0, 0.0);
        let quality_spin = SpinButton::builder()
            .adjustment(&quality_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .build();

        let quality_row = adw::ActionRow::builder()
            .title(i18n("Quality"))
            .subtitle(i18n("0 (lowest) to 9 (highest)"))
            .build();
        quality_row.add_suffix(&quality_spin);
        quality_group.add(&quality_row);

        content.append(&quality_group);

        // === Features Group ===
        let features_group = adw::PreferencesGroup::builder()
            .title(i18n("Features"))
            .build();

        // View-only mode
        let view_only_check = CheckButton::new();
        let view_only_row = adw::ActionRow::builder()
            .title(i18n("View-Only Mode"))
            .subtitle(i18n("Disable keyboard and mouse input"))
            .activatable_widget(&view_only_check)
            .build();
        view_only_row.add_suffix(&view_only_check);
        features_group.add(&view_only_row);

        // Scaling
        let scaling_check = CheckButton::new();
        scaling_check.set_active(true);
        let scaling_row = adw::ActionRow::builder()
            .title(i18n("Scale Display"))
            .subtitle(i18n("Fit remote desktop to window size"))
            .activatable_widget(&scaling_check)
            .build();
        scaling_row.add_suffix(&scaling_check);
        features_group.add(&scaling_row);

        // Clipboard sharing
        let clipboard_check = CheckButton::new();
        clipboard_check.set_active(true);
        let clipboard_row = adw::ActionRow::builder()
            .title(i18n("Clipboard Sharing"))
            .subtitle(i18n("Synchronize clipboard with remote"))
            .activatable_widget(&clipboard_check)
            .build();
        clipboard_row.add_suffix(&clipboard_check);
        features_group.add(&clipboard_row);

        content.append(&features_group);

        // === Advanced Group ===
        let advanced_group = adw::PreferencesGroup::builder()
            .title(i18n("Advanced"))
            .build();

        let custom_args_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Additional arguments for external client"))
            .valign(gtk4::Align::Center)
            .build();

        let args_row = adw::ActionRow::builder()
            .title(i18n("Custom Arguments"))
            .subtitle(i18n("Extra command-line options for vncviewer"))
            .build();
        args_row.add_suffix(&custom_args_entry);
        advanced_group.add(&args_row);

        content.append(&advanced_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            client_mode_dropdown,
            performance_mode_dropdown,
            encoding_entry,
            compression_spin,
            quality_spin,
            view_only_check,
            scaling_check,
            clipboard_check,
            scale_override_dropdown,
            custom_args_entry,
        )
    }

    /// Creates the SPICE options panel using libadwaita components following GNOME HIG.
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn create_spice_options() -> (
        GtkBox,
        CheckButton,
        Entry,
        Button,
        CheckButton,
        CheckButton,
        CheckButton,
        DropDown,
        Rc<RefCell<Vec<SharedFolder>>>,
        gtk4::ListBox,
    ) {
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

        // === Security Group ===
        let security_group = adw::PreferencesGroup::builder()
            .title(i18n("Security"))
            .build();

        // TLS enabled
        let tls_check = CheckButton::new();
        let tls_row = adw::ActionRow::builder()
            .title(i18n("TLS Encryption"))
            .subtitle(i18n("Encrypt connection with TLS"))
            .activatable_widget(&tls_check)
            .build();
        tls_row.add_suffix(&tls_check);
        security_group.add(&tls_row);

        // CA certificate path
        let ca_cert_box = GtkBox::new(Orientation::Horizontal, 4);
        ca_cert_box.set_valign(gtk4::Align::Center);
        let ca_cert_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Path to CA certificate"))
            .build();
        let ca_cert_button = Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(i18n("Browse for certificate"))
            .build();
        ca_cert_box.append(&ca_cert_entry);
        ca_cert_box.append(&ca_cert_button);

        let ca_cert_row = adw::ActionRow::builder()
            .title(i18n("CA Certificate"))
            .subtitle(i18n("Certificate authority for TLS verification"))
            .build();
        ca_cert_row.add_suffix(&ca_cert_box);
        security_group.add(&ca_cert_row);

        // Skip certificate verification
        let skip_verify_check = CheckButton::new();
        let skip_verify_row = adw::ActionRow::builder()
            .title(i18n("Skip Verification"))
            .subtitle(i18n("Disable certificate verification (insecure)"))
            .activatable_widget(&skip_verify_check)
            .build();
        skip_verify_row.add_suffix(&skip_verify_check);
        security_group.add(&skip_verify_row);

        content.append(&security_group);

        // === Features Group ===
        let features_group = adw::PreferencesGroup::builder()
            .title(i18n("Features"))
            .build();

        // USB redirection
        let usb_check = CheckButton::new();
        let usb_row = adw::ActionRow::builder()
            .title(i18n("USB Redirection"))
            .subtitle(i18n("Forward USB devices to remote"))
            .activatable_widget(&usb_check)
            .build();
        usb_row.add_suffix(&usb_check);
        features_group.add(&usb_row);

        // Clipboard sharing
        let clipboard_check = CheckButton::new();
        clipboard_check.set_active(true);
        let clipboard_row = adw::ActionRow::builder()
            .title(i18n("Clipboard Sharing"))
            .subtitle(i18n("Synchronize clipboard with remote"))
            .activatable_widget(&clipboard_check)
            .build();
        clipboard_row.add_suffix(&clipboard_check);
        features_group.add(&clipboard_row);

        // Image compression
        let comp_items: Vec<String> = vec![
            i18n("Auto"),
            i18n("Off"),
            "GLZ".to_string(),
            "LZ".to_string(),
            "QUIC".to_string(),
        ];
        let comp_strs: Vec<&str> = comp_items.iter().map(String::as_str).collect();
        let compression_list = StringList::new(&comp_strs);
        let compression_dropdown = DropDown::new(Some(compression_list), gtk4::Expression::NONE);
        compression_dropdown.set_selected(0);
        compression_dropdown.set_valign(gtk4::Align::Center);

        let compression_row = adw::ActionRow::builder()
            .title(i18n("Image Compression"))
            .subtitle(i18n("Algorithm for image data"))
            .build();
        compression_row.add_suffix(&compression_dropdown);
        features_group.add(&compression_row);

        content.append(&features_group);

        // === Shared Folders Group ===
        let folders_group = adw::PreferencesGroup::builder()
            .title(i18n("Shared Folders"))
            .description(i18n("Local folders accessible from remote session"))
            .build();

        let folders_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        folders_list.set_placeholder(Some(&Label::new(Some(&i18n("No shared folders")))));

        let folders_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(80)
            .max_content_height(120)
            .child(&folders_list)
            .build();
        folders_group.add(&folders_scrolled);

        let folders_buttons = GtkBox::new(Orientation::Horizontal, 8);
        folders_buttons.set_halign(gtk4::Align::End);
        folders_buttons.set_margin_top(8);
        let add_folder_btn = Button::builder()
            .label(i18n("Add"))
            .css_classes(["suggested-action"])
            .build();
        let remove_folder_btn = Button::builder()
            .label(i18n("Remove"))
            .sensitive(false)
            .build();
        folders_buttons.append(&add_folder_btn);
        folders_buttons.append(&remove_folder_btn);
        folders_group.add(&folders_buttons);

        content.append(&folders_group);

        let shared_folders: Rc<RefCell<Vec<SharedFolder>>> = Rc::new(RefCell::new(Vec::new()));

        // Connect add folder button
        Self::connect_add_folder_button(&add_folder_btn, &folders_list, &shared_folders);

        // Connect remove folder button
        Self::connect_remove_folder_button(&remove_folder_btn, &folders_list, &shared_folders);

        // Enable/disable remove button based on selection
        let remove_btn_for_selection = remove_folder_btn;
        folders_list.connect_row_selected(move |_, row| {
            remove_btn_for_selection.set_sensitive(row.is_some());
        });

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            tls_check,
            ca_cert_entry,
            ca_cert_button,
            skip_verify_check,
            usb_check,
            clipboard_check,
            compression_dropdown,
            shared_folders,
            folders_list,
        )
    }

    /// Creates the Zero Trust options panel with provider-specific fields using libadwaita.
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn create_zerotrust_options() -> (
        GtkBox,
        DropDown,
        Stack,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        adw::EntryRow,
        Entry,
    ) {
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

        // === Provider Selection Group ===
        let provider_group = adw::PreferencesGroup::builder()
            .title(i18n("Provider"))
            .build();

        let zt_items: Vec<String> = vec![
            i18n("AWS Session Manager"),
            i18n("GCP IAP Tunnel"),
            i18n("Azure Bastion"),
            i18n("Azure SSH (AAD)"),
            i18n("OCI Bastion"),
            i18n("Cloudflare Access"),
            i18n("Teleport"),
            i18n("Tailscale SSH"),
            i18n("HashiCorp Boundary"),
            i18n("Generic Command"),
        ];
        let zt_strs: Vec<&str> = zt_items.iter().map(String::as_str).collect();
        let provider_list = StringList::new(&zt_strs);
        let provider_dropdown = DropDown::new(Some(provider_list), gtk4::Expression::NONE);
        provider_dropdown.set_selected(0);
        provider_dropdown.set_valign(gtk4::Align::Center);

        let provider_row = adw::ActionRow::builder()
            .title(i18n("Zero Trust Provider"))
            .subtitle(i18n("Select your identity-aware proxy service"))
            .build();
        provider_row.add_suffix(&provider_dropdown);
        provider_group.add(&provider_row);

        content.append(&provider_group);

        // Provider-specific stack
        let provider_stack = Stack::new();
        provider_stack.set_vexpand(true);

        // AWS SSM options
        let (aws_box, aws_target, aws_profile, aws_region) = Self::create_aws_ssm_fields_adw();
        provider_stack.add_named(&aws_box, Some("aws_ssm"));

        // GCP IAP options
        let (gcp_box, gcp_instance, gcp_zone, gcp_project) = Self::create_gcp_iap_fields_adw();
        provider_stack.add_named(&gcp_box, Some("gcp_iap"));

        // Azure Bastion options
        let (azure_bastion_box, azure_bastion_resource_id, azure_bastion_rg, azure_bastion_name) =
            Self::create_azure_bastion_fields_adw();
        provider_stack.add_named(&azure_bastion_box, Some("azure_bastion"));

        // Azure SSH options
        let (azure_ssh_box, azure_ssh_vm, azure_ssh_rg) = Self::create_azure_ssh_fields_adw();
        provider_stack.add_named(&azure_ssh_box, Some("azure_ssh"));

        // OCI Bastion options
        let (oci_box, oci_bastion_id, oci_target_id, oci_target_ip) =
            Self::create_oci_bastion_fields_adw();
        provider_stack.add_named(&oci_box, Some("oci_bastion"));

        // Cloudflare Access options
        let (cf_box, cf_hostname) = Self::create_cloudflare_fields_adw();
        provider_stack.add_named(&cf_box, Some("cloudflare"));

        // Teleport options
        let (teleport_box, teleport_host, teleport_cluster) = Self::create_teleport_fields_adw();
        provider_stack.add_named(&teleport_box, Some("teleport"));

        // Tailscale SSH options
        let (tailscale_box, tailscale_host) = Self::create_tailscale_fields_adw();
        provider_stack.add_named(&tailscale_box, Some("tailscale"));

        // Boundary options
        let (boundary_box, boundary_target, boundary_addr) = Self::create_boundary_fields_adw();
        provider_stack.add_named(&boundary_box, Some("boundary"));

        // Generic command options
        let (generic_box, generic_command) = Self::create_generic_zt_fields_adw();
        provider_stack.add_named(&generic_box, Some("generic"));

        // Set initial view
        provider_stack.set_visible_child_name("aws_ssm");

        content.append(&provider_stack);

        // Connect provider dropdown to stack
        let stack_clone = provider_stack.clone();
        provider_dropdown.connect_selected_notify(move |dropdown| {
            let providers = [
                "aws_ssm",
                "gcp_iap",
                "azure_bastion",
                "azure_ssh",
                "oci_bastion",
                "cloudflare",
                "teleport",
                "tailscale",
                "boundary",
                "generic",
            ];
            let selected = dropdown.selected() as usize;
            if selected < providers.len() {
                stack_clone.set_visible_child_name(providers[selected]);
            }
        });

        // === Advanced Group ===
        let advanced_group = adw::PreferencesGroup::builder()
            .title(i18n("Advanced"))
            .build();

        let custom_args_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Additional command-line arguments"))
            .valign(gtk4::Align::Center)
            .build();

        let args_row = adw::ActionRow::builder()
            .title(i18n("Custom Arguments"))
            .subtitle(i18n("Extra CLI options for the provider command"))
            .build();
        args_row.add_suffix(&custom_args_entry);
        advanced_group.add(&args_row);

        content.append(&advanced_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            provider_dropdown,
            provider_stack,
            aws_target,
            aws_profile,
            aws_region,
            gcp_instance,
            gcp_zone,
            gcp_project,
            azure_bastion_resource_id,
            azure_bastion_rg,
            azure_bastion_name,
            azure_ssh_vm,
            azure_ssh_rg,
            oci_bastion_id,
            oci_target_id,
            oci_target_ip,
            cf_hostname,
            teleport_host,
            teleport_cluster,
            tailscale_host,
            boundary_target,
            boundary_addr,
            generic_command,
            custom_args_entry,
        )
    }

    /// Creates AWS SSM provider fields using libadwaita
    fn create_aws_ssm_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("AWS Session Manager"))
            .description(i18n("Connect via AWS Systems Manager"))
            .build();

        let target_row = adw::EntryRow::builder().title(i18n("Instance ID")).build();
        group.add(&target_row);

        let profile_row = adw::EntryRow::builder().title(i18n("AWS Profile")).build();
        profile_row.set_text("default");
        group.add(&profile_row);

        let region_row = adw::EntryRow::builder().title(i18n("Region")).build();
        group.add(&region_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, target_row, profile_row, region_row)
    }

    /// Creates GCP IAP provider fields using libadwaita
    fn create_gcp_iap_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("GCP IAP Tunnel"))
            .description(i18n("Connect via Identity-Aware Proxy"))
            .build();

        let instance_row = adw::EntryRow::builder()
            .title(i18n("Instance Name"))
            .build();
        group.add(&instance_row);

        let zone_row = adw::EntryRow::builder().title(i18n("Zone")).build();
        group.add(&zone_row);

        let project_row = adw::EntryRow::builder().title(i18n("Project")).build();
        group.add(&project_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, instance_row, zone_row, project_row)
    }

    /// Creates Azure Bastion provider fields using libadwaita
    fn create_azure_bastion_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Azure Bastion"))
            .description(i18n("Connect via Azure Bastion service"))
            .build();

        let resource_id_row = adw::EntryRow::builder()
            .title(i18n("Target Resource ID"))
            .build();
        group.add(&resource_id_row);

        let rg_row = adw::EntryRow::builder()
            .title(i18n("Resource Group"))
            .build();
        group.add(&rg_row);

        let name_row = adw::EntryRow::builder().title(i18n("Bastion Name")).build();
        group.add(&name_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, resource_id_row, rg_row, name_row)
    }

    /// Creates Azure SSH (AAD) provider fields using libadwaita
    fn create_azure_ssh_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Azure SSH (AAD)"))
            .description(i18n("Connect via Azure AD authentication"))
            .build();

        let vm_row = adw::EntryRow::builder().title(i18n("VM Name")).build();
        group.add(&vm_row);

        let rg_row = adw::EntryRow::builder()
            .title(i18n("Resource Group"))
            .build();
        group.add(&rg_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, vm_row, rg_row)
    }

    /// Creates OCI Bastion provider fields using libadwaita
    fn create_oci_bastion_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("OCI Bastion"))
            .description(i18n("Connect via Oracle Cloud Bastion"))
            .build();

        let bastion_id_row = adw::EntryRow::builder().title(i18n("Bastion OCID")).build();
        group.add(&bastion_id_row);

        let target_id_row = adw::EntryRow::builder().title(i18n("Target OCID")).build();
        group.add(&target_id_row);

        let target_ip_row = adw::EntryRow::builder().title(i18n("Target IP")).build();
        group.add(&target_ip_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, bastion_id_row, target_id_row, target_ip_row)
    }

    /// Creates Cloudflare Access provider fields using libadwaita
    fn create_cloudflare_fields_adw() -> (GtkBox, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Cloudflare Access"))
            .description(i18n("Connect via Cloudflare Zero Trust"))
            .build();

        let hostname_row = adw::EntryRow::builder().title(i18n("Hostname")).build();
        group.add(&hostname_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, hostname_row)
    }

    /// Creates Teleport provider fields using libadwaita
    fn create_teleport_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Teleport"))
            .description(i18n("Connect via Gravitational Teleport"))
            .build();

        let host_row = adw::EntryRow::builder().title(i18n("Node Name")).build();
        group.add(&host_row);

        let cluster_row = adw::EntryRow::builder().title(i18n("Cluster")).build();
        group.add(&cluster_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, host_row, cluster_row)
    }

    /// Creates Tailscale SSH provider fields using libadwaita
    fn create_tailscale_fields_adw() -> (GtkBox, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Tailscale SSH"))
            .description(i18n("Connect via Tailscale network"))
            .build();

        let host_row = adw::EntryRow::builder()
            .title(i18n("Tailscale Host"))
            .build();
        group.add(&host_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, host_row)
    }

    /// Creates HashiCorp Boundary provider fields using libadwaita
    fn create_boundary_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("HashiCorp Boundary"))
            .description(i18n("Connect via Boundary proxy"))
            .build();

        let target_row = adw::EntryRow::builder().title(i18n("Target ID")).build();
        group.add(&target_row);

        let addr_row = adw::EntryRow::builder()
            .title(i18n("Controller Address"))
            .build();
        group.add(&addr_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, target_row, addr_row)
    }

    /// Creates Generic Zero Trust provider fields using libadwaita
    fn create_generic_zt_fields_adw() -> (GtkBox, adw::EntryRow) {
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Generic Command"))
            .description(i18n("Custom command for unsupported providers"))
            .build();

        let command_row = adw::EntryRow::builder()
            .title(i18n("Command Template"))
            .build();
        group.add(&command_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, command_row)
    }

    /// Creates the Data tab combining Variables and Custom Properties
    ///
    /// Uses libadwaita components following GNOME HIG.
    #[allow(clippy::type_complexity)]
    fn create_data_tab() -> (GtkBox, ListBox, Button, ListBox, Button) {
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

        // === Variables Section ===
        let variables_group = adw::PreferencesGroup::builder()
            .title(i18n("Local Variables"))
            .description(i18n("Use ${variable_name} syntax in connection fields"))
            .build();

        let variables_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(150)
            .build();

        let variables_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        variables_list.set_placeholder(Some(&Label::new(Some(&i18n("No variables defined")))));
        variables_scrolled.set_child(Some(&variables_list));

        variables_group.add(&variables_scrolled);

        let var_button_box = GtkBox::new(Orientation::Horizontal, 8);
        var_button_box.set_halign(gtk4::Align::End);
        var_button_box.set_margin_top(8);

        let add_variable_button = Button::builder()
            .label(&i18n("Add Variable"))
            .css_classes(["suggested-action"])
            .build();
        var_button_box.append(&add_variable_button);

        variables_group.add(&var_button_box);
        content.append(&variables_group);

        // === Custom Properties Section ===
        let properties_group = adw::PreferencesGroup::builder()
            .title(i18n("Custom Properties"))
            .description(i18n(
                "Text, URL (clickable), or Protected (masked) metadata",
            ))
            .build();

        let properties_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(150)
            .build();

        let properties_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        properties_list.set_placeholder(Some(&Label::new(Some(&i18n("No custom properties")))));
        properties_scrolled.set_child(Some(&properties_list));

        properties_group.add(&properties_scrolled);

        let prop_button_box = GtkBox::new(Orientation::Horizontal, 8);
        prop_button_box.set_halign(gtk4::Align::End);
        prop_button_box.set_margin_top(8);

        let add_property_button = Button::builder()
            .label(&i18n("Add Property"))
            .css_classes(["suggested-action"])
            .build();
        prop_button_box.append(&add_property_button);

        properties_group.add(&prop_button_box);
        content.append(&properties_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            variables_list,
            add_variable_button,
            properties_list,
            add_property_button,
        )
    }

    /// Creates the combined Automation tab (Expect Rules + Tasks)
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn create_automation_combined_tab() -> (
        GtkBox,
        ListBox,
        Button,
        GtkBox,
        Entry,
        Label,
        CheckButton,
        Entry,
        SpinButton,
        CheckButton,
        CheckButton,
        CheckButton,
        Entry,
        SpinButton,
        CheckButton,
    ) {
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

        // === Expect Rules Section ===
        let rules_group = adw::PreferencesGroup::builder()
            .title(i18n("Expect Rules"))
            .description(i18n("Auto-respond to terminal patterns (priority order)"))
            .build();

        let rules_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(120)
            .build();

        let expect_rules_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        expect_rules_list.set_placeholder(Some(&Label::new(Some(&i18n("No expect rules")))));
        rules_scrolled.set_child(Some(&expect_rules_list));

        rules_group.add(&rules_scrolled);

        let rules_button_box = GtkBox::new(Orientation::Horizontal, 8);
        rules_button_box.set_halign(gtk4::Align::End);
        rules_button_box.set_margin_top(8);

        let template_menu_button = gtk4::MenuButton::builder()
            .label(&i18n("From Template"))
            .tooltip_text(i18n("Add rules from a built-in template"))
            .build();

        let template_popover = gtk4::Popover::new();
        let template_list_box = GtkBox::new(Orientation::Vertical, 4);
        template_list_box.set_margin_top(8);
        template_list_box.set_margin_bottom(8);
        template_list_box.set_margin_start(8);
        template_list_box.set_margin_end(8);

        for template in builtin_templates() {
            let btn = Button::builder()
                .label(template.name)
                .css_classes(["flat"])
                .tooltip_text(template.description)
                .build();
            template_list_box.append(&btn);
        }
        template_popover.set_child(Some(&template_list_box));
        template_menu_button.set_popover(Some(&template_popover));

        let add_rule_button = Button::builder()
            .label(&i18n("Add Rule"))
            .css_classes(["suggested-action"])
            .build();
        rules_button_box.append(&template_menu_button);
        rules_button_box.append(&add_rule_button);

        rules_group.add(&rules_button_box);
        content.append(&rules_group);

        // Pattern tester
        let tester_group = adw::PreferencesGroup::builder()
            .title(i18n("Pattern Tester"))
            .build();

        let test_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text(&i18n("Test text against patterns"))
            .build();

        let test_row = adw::ActionRow::builder().title(i18n("Test Input")).build();
        test_row.add_suffix(&test_entry);
        tester_group.add(&test_row);

        let result_label = Label::builder()
            .label(&i18n("Enter text to test"))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();

        let result_row = adw::ActionRow::builder().title(i18n("Result")).build();
        result_row.add_suffix(&result_label);
        tester_group.add(&result_row);

        content.append(&tester_group);

        // === Pre-Connect Task Section ===
        let (
            pre_connect_group,
            pre_connect_enabled_check,
            pre_connect_command_entry,
            pre_connect_timeout_spin,
            pre_connect_abort_check,
            pre_connect_first_only_check,
        ) = Self::create_task_section(&i18n("Pre-Connect Task"), true);
        content.append(&pre_connect_group);

        // === Post-Disconnect Task Section ===
        let (
            post_disconnect_group,
            post_disconnect_enabled_check,
            post_disconnect_command_entry,
            post_disconnect_timeout_spin,
            _post_disconnect_abort_check,
            post_disconnect_last_only_check,
        ) = Self::create_task_section(&i18n("Post-Disconnect Task"), false);
        content.append(&post_disconnect_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            expect_rules_list,
            add_rule_button,
            template_list_box,
            test_entry,
            result_label,
            pre_connect_enabled_check,
            pre_connect_command_entry,
            pre_connect_timeout_spin,
            pre_connect_abort_check,
            pre_connect_first_only_check,
            post_disconnect_enabled_check,
            post_disconnect_command_entry,
            post_disconnect_timeout_spin,
            post_disconnect_last_only_check,
        )
    }

    /// Creates a task section (pre-connect or post-disconnect)
    ///
    /// Uses libadwaita components following GNOME HIG.
    fn create_task_section(
        title: &str,
        is_pre_connect: bool,
    ) -> (
        adw::PreferencesGroup,
        CheckButton,
        Entry,
        SpinButton,
        CheckButton,
        CheckButton,
    ) {
        let description = if is_pre_connect {
            i18n("Run command before connecting. Supports ${variable} substitution.")
        } else {
            i18n("Run command after disconnecting. Supports ${variable} substitution.")
        };

        let group = adw::PreferencesGroup::builder()
            .title(title)
            .description(description)
            .build();

        // Enable checkbox
        let enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let enable_row = adw::ActionRow::builder()
            .title(i18n("Enable Task"))
            .activatable_widget(&enabled_check)
            .build();
        enable_row.add_suffix(&enabled_check);
        group.add(&enable_row);

        // Command entry
        let command_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text(i18n("e.g., /path/to/script.sh or vpn-connect ${host}"))
            .sensitive(false)
            .build();

        let command_row = adw::ActionRow::builder()
            .title(i18n("Command"))
            .subtitle(i18n(
                "Shell command to execute (supports ${variable} syntax)",
            ))
            .build();
        command_row.add_suffix(&command_entry);
        group.add(&command_row);

        // Timeout
        let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 300_000.0, 1000.0, 5000.0, 0.0);
        let timeout_spin = SpinButton::builder()
            .adjustment(&timeout_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let timeout_row = adw::ActionRow::builder()
            .title(i18n("Timeout (ms)"))
            .subtitle(i18n("0 = no timeout"))
            .build();
        timeout_row.add_suffix(&timeout_spin);
        group.add(&timeout_row);

        // Abort on failure (pre-connect only)
        let abort_check = CheckButton::builder()
            .valign(gtk4::Align::Center)
            .active(true)
            .sensitive(false)
            .build();

        if is_pre_connect {
            let abort_row = adw::ActionRow::builder()
                .title(i18n("Abort on Failure"))
                .subtitle(i18n("Cancel connection if this task fails"))
                .activatable_widget(&abort_check)
                .build();
            abort_row.add_suffix(&abort_check);
            group.add(&abort_row);
        }

        // Condition checkbox
        let condition_check = CheckButton::builder()
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let (condition_title, condition_subtitle) = if is_pre_connect {
            (
                i18n("First Connection Only"),
                i18n(
                    "Only run when opening the first connection in a folder (useful for VPN setup)",
                ),
            )
        } else {
            (
                i18n("Last Connection Only"),
                i18n("Only run when closing the last connection in a folder (useful for cleanup)"),
            )
        };

        let condition_row = adw::ActionRow::builder()
            .title(condition_title)
            .subtitle(condition_subtitle)
            .activatable_widget(&condition_check)
            .build();
        condition_row.add_suffix(&condition_check);
        group.add(&condition_row);

        // Connect enabled checkbox to enable/disable other fields
        let command_entry_clone = command_entry.clone();
        let timeout_spin_clone = timeout_spin.clone();
        let abort_check_clone = abort_check.clone();
        let condition_check_clone = condition_check.clone();
        enabled_check.connect_toggled(move |check| {
            let enabled = check.is_active();
            command_entry_clone.set_sensitive(enabled);
            timeout_spin_clone.set_sensitive(enabled);
            abort_check_clone.set_sensitive(enabled);
            condition_check_clone.set_sensitive(enabled);
        });

        (
            group,
            enabled_check,
            command_entry,
            timeout_spin,
            abort_check,
            condition_check,
        )
    }

    /// Creates the Advanced tab combining Display and WOL settings
    ///
    /// Uses libadwaita components following GNOME HIG.
    #[allow(clippy::type_complexity)]
    fn create_advanced_tab() -> (
        GtkBox,
        DropDown,
        CheckButton,
        CheckButton,
        Entry,
        Entry,
        SpinButton,
        SpinButton,
    ) {
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

        // === Window Mode Section ===
        let mode_group = adw::PreferencesGroup::builder()
            .title(i18n("Window Mode"))
            .build();

        let mode_list = StringList::new(&[
            &i18n("Embedded"),
            &i18n("External Window"),
            &i18n("Fullscreen"),
        ]);
        let mode_dropdown = DropDown::new(Some(mode_list), gtk4::Expression::NONE);
        mode_dropdown.set_selected(0);
        mode_dropdown.set_valign(gtk4::Align::Center);

        let mode_row = adw::ActionRow::builder()
            .title(i18n("Display Mode"))
            .subtitle(i18n("Embedded • External • Fullscreen"))
            .build();
        mode_row.add_suffix(&mode_dropdown);
        mode_group.add(&mode_row);

        let remember_check = CheckButton::builder()
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let remember_row = adw::ActionRow::builder()
            .title(i18n("Remember Position"))
            .subtitle(i18n("Save window geometry (External mode only)"))
            .activatable_widget(&remember_check)
            .build();
        remember_row.add_suffix(&remember_check);
        mode_group.add(&remember_row);

        let remember_check_clone = remember_check.clone();
        let remember_row_clone = remember_row.clone();
        mode_dropdown.connect_selected_notify(move |dropdown| {
            let is_external = dropdown.selected() == 1;
            remember_check_clone.set_sensitive(is_external);
            remember_row_clone.set_sensitive(is_external);
            if !is_external {
                remember_check_clone.set_active(false);
            }
        });

        content.append(&mode_group);

        // === Wake On LAN Section ===
        let wol_group = adw::PreferencesGroup::builder()
            .title(i18n("Wake On LAN"))
            .build();

        let wol_enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let wol_enable_row = adw::ActionRow::builder()
            .title(i18n("Enable WOL"))
            .subtitle(i18n("Send magic packet before connecting"))
            .activatable_widget(&wol_enabled_check)
            .build();
        wol_enable_row.add_suffix(&wol_enabled_check);
        wol_group.add(&wol_enable_row);

        content.append(&wol_group);

        // WOL Settings group
        let wol_settings_group = adw::PreferencesGroup::builder()
            .title(i18n("WOL Settings"))
            .sensitive(false)
            .build();

        let mac_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text(i18n("AA:BB:CC:DD:EE:FF"))
            .build();

        let mac_row = adw::ActionRow::builder().title(i18n("MAC Address")).build();
        mac_row.add_suffix(&mac_entry);
        wol_settings_group.add(&mac_row);

        let broadcast_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .text(DEFAULT_BROADCAST_ADDRESS)
            .build();

        let broadcast_row = adw::ActionRow::builder()
            .title(i18n("Broadcast Address"))
            .build();
        broadcast_row.add_suffix(&broadcast_entry);
        wol_settings_group.add(&broadcast_row);

        let port_adjustment =
            gtk4::Adjustment::new(f64::from(DEFAULT_WOL_PORT), 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_spin = SpinButton::builder()
            .adjustment(&port_adjustment)
            .digits(0)
            .valign(gtk4::Align::Center)
            .build();

        let port_row = adw::ActionRow::builder()
            .title(i18n("UDP Port"))
            .subtitle(i18n("Default: 9"))
            .build();
        port_row.add_suffix(&port_spin);
        wol_settings_group.add(&port_row);

        let wait_adjustment = gtk4::Adjustment::new(
            f64::from(DEFAULT_WOL_WAIT_SECONDS),
            0.0,
            300.0,
            1.0,
            10.0,
            0.0,
        );
        let wait_spin = SpinButton::builder()
            .adjustment(&wait_adjustment)
            .digits(0)
            .valign(gtk4::Align::Center)
            .build();

        let wait_row = adw::ActionRow::builder()
            .title(i18n("Wait Time (sec)"))
            .subtitle(i18n("Time to wait for boot"))
            .build();
        wait_row.add_suffix(&wait_spin);
        wol_settings_group.add(&wait_row);

        content.append(&wol_settings_group);

        // Connect WOL enabled checkbox
        let wol_settings_group_clone = wol_settings_group.clone();
        wol_enabled_check.connect_toggled(move |check| {
            wol_settings_group_clone.set_sensitive(check.is_active());
        });

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            mode_dropdown,
            remember_check,
            wol_enabled_check,
            mac_entry,
            broadcast_entry,
            port_spin,
            wait_spin,
        )
    }

    /// Creates a custom property row widget
    fn create_custom_property_row(property: Option<&CustomProperty>) -> CustomPropertyRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(8);
        main_box.set_margin_bottom(8);
        main_box.set_margin_start(8);
        main_box.set_margin_end(8);

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
    #[allow(clippy::too_many_lines)]
    fn create_expect_rule_row(rule: Option<&ExpectRule>) -> ExpectRuleRow {
        let main_box = GtkBox::new(Orientation::Vertical, 8);
        main_box.set_margin_top(8);
        main_box.set_margin_bottom(8);
        main_box.set_margin_start(8);
        main_box.set_margin_end(8);

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .hexpand(true)
            .build();

        // Row 0: Pattern and action buttons
        let pattern_label = Label::builder()
            .label(i18n("Pattern:"))
            .halign(gtk4::Align::End)
            .build();
        let pattern_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Regex pattern (e.g., password:\\s*$)"))
            .tooltip_text(i18n("Regular expression to match against terminal output"))
            .build();

        let button_box = GtkBox::new(Orientation::Horizontal, 4);
        let move_up_button = Button::builder()
            .icon_name("go-up-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Move up (higher priority)"))
            .build();
        let move_down_button = Button::builder()
            .icon_name("go-down-symbolic")
            .css_classes(["flat"])
            .tooltip_text(i18n("Move down (lower priority)"))
            .build();
        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text(i18n("Delete rule"))
            .build();
        button_box.append(&move_up_button);
        button_box.append(&move_down_button);
        button_box.append(&delete_button);

        grid.attach(&pattern_label, 0, 0, 1, 1);
        grid.attach(&pattern_entry, 1, 0, 1, 1);
        grid.attach(&button_box, 2, 0, 1, 1);

        // Row 1: Response
        let response_label = Label::builder()
            .label(i18n("Response:"))
            .halign(gtk4::Align::End)
            .build();
        let response_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text(i18n("Text to send when pattern matches"))
            .tooltip_text(i18n("Response to send (supports ${variable} syntax)"))
            .build();

        grid.attach(&response_label, 0, 1, 1, 1);
        grid.attach(&response_entry, 1, 1, 2, 1);

        // Row 2: Priority and Timeout
        let priority_label = Label::builder()
            .label(i18n("Priority:"))
            .halign(gtk4::Align::End)
            .build();
        let priority_adj = gtk4::Adjustment::new(0.0, -1000.0, 1000.0, 1.0, 10.0, 0.0);
        let priority_spin = SpinButton::builder()
            .adjustment(&priority_adj)
            .climb_rate(1.0)
            .digits(0)
            .tooltip_text(i18n("Higher priority rules are checked first"))
            .build();

        let timeout_label = Label::builder()
            .label(i18n("Timeout (ms):"))
            .halign(gtk4::Align::End)
            .build();
        let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 60000.0, 100.0, 1000.0, 0.0);
        let timeout_spin = SpinButton::builder()
            .adjustment(&timeout_adj)
            .climb_rate(1.0)
            .digits(0)
            .tooltip_text(i18n("Timeout in milliseconds (0 = no timeout)"))
            .build();

        let settings_box = GtkBox::new(Orientation::Horizontal, 12);
        let priority_box = GtkBox::new(Orientation::Horizontal, 4);
        priority_box.append(&priority_label);
        priority_box.append(&priority_spin);
        let timeout_box = GtkBox::new(Orientation::Horizontal, 4);
        timeout_box.append(&timeout_label);
        timeout_box.append(&timeout_spin);
        settings_box.append(&priority_box);
        settings_box.append(&timeout_box);

        grid.attach(&settings_box, 1, 2, 2, 1);

        // Row 3: Enabled and One-shot checkboxes
        let enabled_check = CheckButton::builder()
            .label(i18n("Enabled"))
            .active(true)
            .build();

        let one_shot_check = CheckButton::builder()
            .label(i18n("One-shot"))
            .active(true)
            .tooltip_text(i18n("Fire only once, then remove the rule"))
            .build();

        let checks_box = GtkBox::new(Orientation::Horizontal, 12);
        checks_box.append(&enabled_check);
        checks_box.append(&one_shot_check);

        grid.attach(&checks_box, 1, 3, 2, 1);

        // Row 4: Regex validation label
        let validation_label = Label::builder()
            .halign(gtk4::Align::Start)
            .css_classes(["error"])
            .visible(false)
            .build();
        grid.attach(&validation_label, 1, 4, 2, 1);

        main_box.append(&grid);

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
            #[allow(clippy::cast_possible_truncation)]
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
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
        #[allow(clippy::cast_sign_loss)]
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
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        if index < 0 || index >= (rules_len as i32 - 1) {
            return;
        }

        // Remove and re-insert the row
        list.remove(row);
        let new_index = index + 1;
        list.insert(row, new_index);

        // Update the rules vector
        #[allow(clippy::cast_sign_loss)]
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
                result_clone.set_text("Enter text above to test patterns");
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
            sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

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
                result_clone.set_text("No patterns matched");
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
        main_box.set_margin_top(8);
        main_box.set_margin_bottom(8);
        main_box.set_margin_start(8);
        main_box.set_margin_end(8);

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
                "Remove override"
            } else {
                "Delete variable"
            })
            .build();

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
                        entry.set_text(&path.to_string_lossy());
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
        self.window.set_title(Some(&i18n("Edit Connection")));
        self.save_button.set_label(&i18n("Save"));
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

        // Protocol and protocol-specific fields
        match &conn.protocol_config {
            ProtocolConfig::Ssh(ssh) => {
                self.protocol_dropdown.set_selected(0); // SSH
                self.protocol_stack.set_visible_child_name("ssh");
                self.set_ssh_config(ssh);
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
            }
            ProtocolConfig::Kubernetes(k8s) => {
                self.protocol_dropdown.set_selected(8); // Kubernetes
                self.protocol_stack.set_visible_child_name("kubernetes");
                self.set_kubernetes_config(k8s);
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

        // Set window mode
        self.window_mode_dropdown
            .set_selected(conn.window_mode.index());
        self.remember_position_check
            .set_active(conn.remember_window_position);
        // Enable remember position checkbox only for External mode
        let is_external = matches!(conn.window_mode, WindowMode::External);
        self.remember_position_check.set_sensitive(is_external);

        // Set custom properties
        self.set_custom_properties(&conn.custom_properties);

        // Set WOL config
        self.set_wol_config(conn.wol_config.as_ref());
    }

    /// Sets the available groups for the group dropdown
    ///
    /// Groups are displayed in a flat list with hierarchy indicated by indentation.
    /// The first item is always "(Root)" for connections without a group.
    #[allow(clippy::items_after_statements)]
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
        let mut groups_data: Vec<(Option<Uuid>, String)> = vec![(None, "(Root)".to_string())];

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
        sorted_conns.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        for conn in sorted_conns {
            connections_data.push((Some(conn.id), conn.name.clone()));
        }

        *self.connections_data.borrow_mut() = connections_data.clone();

        let display_strings: Vec<&str> = connections_data
            .iter()
            .map(|(_, name)| name.as_str())
            .collect();
        let model = StringList::new(&display_strings);
        self.ssh_jump_host_dropdown.set_model(Some(&model));
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
            #[allow(clippy::cast_possible_truncation)]
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
            self.pre_connect_enabled_check.set_active(true);
            self.pre_connect_command_entry.set_text(&task.command);
            self.pre_connect_command_entry.set_sensitive(true);
            self.pre_connect_timeout_spin
                .set_value(f64::from(task.timeout_ms.unwrap_or(0)));
            self.pre_connect_timeout_spin.set_sensitive(true);
            self.pre_connect_abort_check
                .set_active(task.abort_on_failure);
            self.pre_connect_abort_check.set_sensitive(true);
            self.pre_connect_first_only_check
                .set_active(task.condition.only_first_in_folder);
            self.pre_connect_first_only_check.set_sensitive(true);
        } else {
            self.pre_connect_enabled_check.set_active(false);
            self.pre_connect_command_entry.set_text("");
            self.pre_connect_command_entry.set_sensitive(false);
            self.pre_connect_timeout_spin.set_value(0.0);
            self.pre_connect_timeout_spin.set_sensitive(false);
            self.pre_connect_abort_check.set_active(true);
            self.pre_connect_abort_check.set_sensitive(false);
            self.pre_connect_first_only_check.set_active(false);
            self.pre_connect_first_only_check.set_sensitive(false);
        }
    }

    /// Sets the post-disconnect task fields
    fn set_post_disconnect_task(&self, task: Option<&ConnectionTask>) {
        if let Some(task) = task {
            self.post_disconnect_enabled_check.set_active(true);
            self.post_disconnect_command_entry.set_text(&task.command);
            self.post_disconnect_command_entry.set_sensitive(true);
            self.post_disconnect_timeout_spin
                .set_value(f64::from(task.timeout_ms.unwrap_or(0)));
            self.post_disconnect_timeout_spin.set_sensitive(true);
            self.post_disconnect_last_only_check
                .set_active(task.condition.only_last_in_folder);
            self.post_disconnect_last_only_check.set_sensitive(true);
        } else {
            self.post_disconnect_enabled_check.set_active(false);
            self.post_disconnect_command_entry.set_text("");
            self.post_disconnect_command_entry.set_sensitive(false);
            self.post_disconnect_timeout_spin.set_value(0.0);
            self.post_disconnect_timeout_spin.set_sensitive(false);
            self.post_disconnect_last_only_check.set_active(false);
            self.post_disconnect_last_only_check.set_sensitive(false);
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
                // Try to select the matching agent key in the dropdown
                self.select_agent_key_by_fingerprint(fingerprint, comment);
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
        self.ssh_identities_only.set_active(ssh.identities_only);
        self.ssh_control_master.set_active(ssh.use_control_master);
        self.ssh_agent_forwarding.set_active(ssh.agent_forwarding);
        self.ssh_waypipe.set_active(ssh.waypipe);
        self.ssh_x11_forwarding.set_active(ssh.x11_forwarding);
        self.ssh_compression.set_active(ssh.compression);
        if let Some(ref cmd) = ssh.startup_command {
            self.ssh_startup_entry.set_text(cmd);
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
                #[allow(clippy::cast_possible_truncation)]
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
        if let Some(ref gw) = rdp.gateway {
            self.rdp_gateway_entry.set_text(&gw.hostname);
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
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);
            row_box.set_margin_start(8);
            row_box.set_margin_end(8);

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

        // Set keyboard layout dropdown
        if let Some(klid) = rdp.keyboard_layout {
            let index = klid_to_dropdown_index(klid);
            self.rdp_keyboard_layout_dropdown.set_selected(index);
        } else {
            self.rdp_keyboard_layout_dropdown.set_selected(0); // Auto
        }
    }

    fn set_vnc_config(&self, vnc: &VncConfig) {
        // Set client mode dropdown
        self.vnc_client_mode_dropdown
            .set_selected(vnc.client_mode.index());

        // Set performance mode dropdown
        self.vnc_performance_mode_dropdown
            .set_selected(vnc.performance_mode.index());

        let encoding_text = vnc.encoding.as_deref().unwrap_or("");
        self.vnc_encoding_entry.set_text(encoding_text);

        if let Some(comp) = vnc.compression {
            self.vnc_compression_spin.set_value(f64::from(comp));
        }
        if let Some(qual) = vnc.quality {
            self.vnc_quality_spin.set_value(f64::from(qual));
        }

        self.vnc_view_only_check.set_active(vnc.view_only);
        self.vnc_scaling_check.set_active(vnc.scaling);
        self.vnc_clipboard_check.set_active(vnc.clipboard_enabled);
        self.vnc_scale_override_dropdown
            .set_selected(vnc.scale_override.index());

        if !vnc.custom_args.is_empty() {
            self.vnc_custom_args_entry
                .set_text(&vnc.custom_args.join(" "));
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

        // Populate shared folders
        self.spice_shared_folders.borrow_mut().clear();
        while let Some(row) = self.spice_shared_folders_list.row_at_index(0) {
            self.spice_shared_folders_list.remove(&row);
        }
        for folder in &spice.shared_folders {
            self.spice_shared_folders.borrow_mut().push(folder.clone());
            Self::add_folder_row_to_list(
                &self.spice_shared_folders_list,
                &folder.local_path,
                &folder.share_name,
            );
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
            ZeroTrustProvider::Generic => 9,
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

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<super::ConnectionDialogResult>) + 'static>(&self, cb: F) {
        // Store callback - the save button handler was connected in the constructor
        // and will invoke this callback when clicked
        *self.on_save.borrow_mut() = Some(Box::new(cb));

        // Refresh agent keys before showing the dialog
        self.refresh_agent_keys();

        self.window.present();
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
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

        self.password_source_dropdown
            .connect_selected_notify(move |dropdown| {
                let selected = dropdown.selected();
                // Show password row for Vault(1) only
                password_row.set_visible(selected == 1);
                // Show variable row for Variable(2) only
                variable_row.set_visible(selected == 2);
            });
    }

    /// Connects password load button to load password from vault (KeePass or Keyring)
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
    pub fn connect_password_load_button(
        &self,
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<String>,
        kdbx_key_file: Option<std::path::PathBuf>,
        secret_settings: rustconn_core::config::SecretSettings,
    ) {
        // Call the extended version with empty groups (legacy behavior)
        self.connect_password_load_button_with_groups(
            kdbx_enabled,
            kdbx_path,
            kdbx_password,
            kdbx_key_file,
            Vec::new(),
            secret_settings,
        );
    }

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
    #[allow(clippy::too_many_arguments)]
    pub fn connect_password_load_button_with_groups(
        &self,
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<String>,
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
        let window = self.window.clone();

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

            // Flat lookup key for Keyring/Bitwarden — matches the format
            // used by resolve_from_keyring and import credential storage:
            // "{name} ({protocol})"
            let flat_lookup_key = {
                let sanitized = base_name.trim().replace('/', "-");
                let sanitized = rustconn_core::import::sanitize_imported_value(&sanitized);
                format!("{sanitized} ({protocol_suffix})")
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
                            let db_password = kdbx_password
                                .as_ref()
                                .map(|p| secrecy::SecretString::from(p.clone()));
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

    /// Refreshes the agent keys dropdown with keys from the SSH agent
    ///
    /// This should be called before showing the dialog to populate the agent key dropdown
    /// with the currently loaded keys from the SSH agent.
    pub fn refresh_agent_keys(&self) {
        use rustconn_core::ssh_agent::SshAgentManager;

        let manager = SshAgentManager::from_env();
        let keys = match manager.get_status() {
            Ok(status) if status.running => status.keys,
            _ => Vec::new(),
        };

        // Update the stored keys
        *self.ssh_agent_keys.borrow_mut() = keys.clone();

        // Build the dropdown items with shortened display
        let items: Vec<String> = if keys.is_empty() {
            vec!["(No keys loaded)".to_string()]
        } else {
            keys.iter()
                .map(|key| Self::format_agent_key_short(key))
                .collect()
        };

        // Create new StringList and set it on the dropdown
        let string_list = StringList::new(&items.iter().map(String::as_str).collect::<Vec<_>>());
        self.ssh_agent_key_dropdown.set_model(Some(&string_list));
        self.ssh_agent_key_dropdown.set_selected(0);

        // Update sensitivity based on whether keys are available
        let has_keys = !keys.is_empty();
        if self.ssh_key_source_dropdown.selected() == 2 {
            // Agent source is selected
            self.ssh_agent_key_dropdown.set_sensitive(has_keys);
        }
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
}

/// Helper struct for validation and building in the response callback
struct ConnectionDialogData<'a> {
    name_entry: &'a Entry,
    icon_entry: &'a Entry,
    description_view: &'a TextView,
    host_entry: &'a Entry,
    port_spin: &'a SpinButton,
    username_entry: &'a Entry,
    domain_entry: &'a Entry,
    tags_entry: &'a Entry,
    protocol_dropdown: &'a DropDown,
    password_source_dropdown: &'a DropDown,
    password_entry: &'a Entry,
    variable_dropdown: &'a DropDown,
    group_dropdown: &'a DropDown,
    groups_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
    ssh_auth_dropdown: &'a DropDown,
    ssh_key_source_dropdown: &'a DropDown,
    ssh_key_entry: &'a Entry,
    ssh_agent_key_dropdown: &'a DropDown,
    ssh_agent_keys: &'a Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>>,
    ssh_proxy_entry: &'a Entry,
    ssh_identities_only: &'a CheckButton,
    ssh_control_master: &'a CheckButton,
    ssh_agent_forwarding: &'a CheckButton,
    ssh_waypipe: &'a CheckButton,
    ssh_x11_forwarding: &'a CheckButton,
    ssh_compression: &'a CheckButton,
    ssh_startup_entry: &'a Entry,
    ssh_options_entry: &'a Entry,
    ssh_port_forwards: &'a Rc<RefCell<Vec<rustconn_core::models::PortForward>>>,
    rdp_client_mode_dropdown: &'a DropDown,
    rdp_performance_mode_dropdown: &'a DropDown,
    rdp_width_spin: &'a SpinButton,
    rdp_height_spin: &'a SpinButton,
    rdp_color_dropdown: &'a DropDown,
    rdp_scale_override_dropdown: &'a DropDown,
    rdp_audio_check: &'a CheckButton,
    rdp_gateway_entry: &'a Entry,
    rdp_shared_folders: &'a Rc<RefCell<Vec<SharedFolder>>>,
    rdp_custom_args_entry: &'a Entry,
    rdp_keyboard_layout_dropdown: &'a DropDown,
    vnc_client_mode_dropdown: &'a DropDown,
    vnc_performance_mode_dropdown: &'a DropDown,
    vnc_encoding_entry: &'a Entry,
    vnc_compression_spin: &'a SpinButton,
    vnc_quality_spin: &'a SpinButton,
    vnc_view_only_check: &'a CheckButton,
    vnc_scaling_check: &'a CheckButton,
    vnc_clipboard_check: &'a CheckButton,
    vnc_scale_override_dropdown: &'a DropDown,
    vnc_custom_args_entry: &'a Entry,
    spice_tls_check: &'a CheckButton,
    spice_ca_cert_entry: &'a Entry,
    spice_skip_verify_check: &'a CheckButton,
    spice_usb_check: &'a CheckButton,
    spice_clipboard_check: &'a CheckButton,
    spice_compression_dropdown: &'a DropDown,
    spice_shared_folders: &'a Rc<RefCell<Vec<SharedFolder>>>,
    // Zero Trust fields
    zt_provider_dropdown: &'a DropDown,
    zt_aws_target_entry: &'a adw::EntryRow,
    zt_aws_profile_entry: &'a adw::EntryRow,
    zt_aws_region_entry: &'a adw::EntryRow,
    zt_gcp_instance_entry: &'a adw::EntryRow,
    zt_gcp_zone_entry: &'a adw::EntryRow,
    zt_gcp_project_entry: &'a adw::EntryRow,
    zt_azure_bastion_resource_id_entry: &'a adw::EntryRow,
    zt_azure_bastion_rg_entry: &'a adw::EntryRow,
    zt_azure_bastion_name_entry: &'a adw::EntryRow,
    zt_azure_ssh_vm_entry: &'a adw::EntryRow,
    zt_azure_ssh_rg_entry: &'a adw::EntryRow,
    zt_oci_bastion_id_entry: &'a adw::EntryRow,
    zt_oci_target_id_entry: &'a adw::EntryRow,
    zt_oci_target_ip_entry: &'a adw::EntryRow,
    zt_cf_hostname_entry: &'a adw::EntryRow,
    zt_teleport_host_entry: &'a adw::EntryRow,
    zt_teleport_cluster_entry: &'a adw::EntryRow,
    zt_tailscale_host_entry: &'a adw::EntryRow,
    zt_boundary_target_entry: &'a adw::EntryRow,
    zt_boundary_addr_entry: &'a adw::EntryRow,
    zt_generic_command_entry: &'a adw::EntryRow,
    zt_custom_args_entry: &'a Entry,
    // Telnet fields
    telnet_custom_args_entry: &'a Entry,
    telnet_backspace_dropdown: &'a DropDown,
    telnet_delete_dropdown: &'a DropDown,
    // Serial fields
    serial_device_entry: &'a Entry,
    serial_baud_dropdown: &'a DropDown,
    serial_data_bits_dropdown: &'a DropDown,
    serial_stop_bits_dropdown: &'a DropDown,
    serial_parity_dropdown: &'a DropDown,
    serial_flow_control_dropdown: &'a DropDown,
    serial_custom_args_entry: &'a Entry,
    // Kubernetes fields
    k8s_kubeconfig_entry: &'a Entry,
    k8s_context_entry: &'a Entry,
    k8s_namespace_entry: &'a Entry,
    k8s_pod_entry: &'a Entry,
    k8s_container_entry: &'a Entry,
    k8s_shell_dropdown: &'a DropDown,
    k8s_busybox_check: &'a CheckButton,
    k8s_busybox_image_entry: &'a Entry,
    k8s_custom_args_entry: &'a Entry,
    local_variables: &'a HashMap<String, Variable>,
    logging_tab: &'a logging_tab::LoggingTab,
    expect_rules: &'a Vec<ExpectRule>,
    // Task fields
    pre_connect_enabled_check: &'a CheckButton,
    pre_connect_command_entry: &'a Entry,
    pre_connect_timeout_spin: &'a SpinButton,
    pre_connect_abort_check: &'a CheckButton,
    pre_connect_first_only_check: &'a CheckButton,
    post_disconnect_enabled_check: &'a CheckButton,
    post_disconnect_command_entry: &'a Entry,
    post_disconnect_timeout_spin: &'a SpinButton,
    post_disconnect_last_only_check: &'a CheckButton,
    // Window mode fields
    window_mode_dropdown: &'a DropDown,
    remember_position_check: &'a CheckButton,
    // Custom properties
    custom_properties: &'a Vec<CustomProperty>,
    // WOL fields
    wol_enabled_check: &'a CheckButton,
    wol_mac_entry: &'a Entry,
    wol_broadcast_entry: &'a Entry,
    wol_port_spin: &'a SpinButton,
    wol_wait_spin: &'a SpinButton,
    editing_id: &'a Rc<RefCell<Option<Uuid>>>,
    // Jump Host fields
    ssh_jump_host_dropdown: &'a DropDown,
    connections_data: &'a Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
}

impl ConnectionDialogData<'_> {
    fn validate(&self) -> Result<(), String> {
        let name = self.name_entry.text();
        if name.trim().is_empty() {
            return Err("Connection name is required".to_string());
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
                return Err("Host is required".to_string());
            }

            let host_str = host.trim();
            if host_str.contains(' ') {
                return Err("Host cannot contain spaces".to_string());
            }

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let port = self.port_spin.value() as u16;
            if port == 0 {
                return Err("Port must be greater than 0".to_string());
            }
        }

        // Serial requires a device path
        if is_serial {
            let device = self.serial_device_entry.text();
            if device.trim().is_empty() {
                return Err("Device path is required for serial connections".to_string());
            }
        }
        if protocol_idx == 0 {
            // SSH
            let auth_idx = self.ssh_auth_dropdown.selected();
            if auth_idx == 1 {
                // Public Key
                let key_path = self.ssh_key_entry.text();
                if key_path.trim().is_empty() {
                    return Err(
                        "SSH key path is required for public key authentication".to_string()
                    );
                }
            }
        }
        // RDP (1) and VNC (2) use native embedding, no client validation needed

        // WOL validation
        if self.wol_enabled_check.is_active() {
            let mac_text = self.wol_mac_entry.text();
            if mac_text.trim().is_empty() {
                return Err("MAC address is required when WOL is enabled".to_string());
            }
            // Validate MAC address format
            if MacAddress::parse(mac_text.trim()).is_err() {
                return Err(
                    "Invalid MAC address format. Use AA:BB:CC:DD:EE:FF or AA-BB-CC-DD-EE-FF"
                        .to_string(),
                );
            }
        }

        // Icon validation
        let icon_text = self.icon_entry.text();
        rustconn_core::dialog_utils::validate_icon(icon_text.trim())?;

        Ok(())
    }

    fn build_connection(&self) -> Option<super::ConnectionDialogResult> {
        let name = self.name_entry.text().trim().to_string();
        let buffer = self.description_view.buffer();
        let description_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let description = if description_text.trim().is_empty() {
            None
        } else {
            Some(description_text.trim().to_string())
        };
        let host = self.host_entry.text().trim().to_string();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
        // Dropdown order: Prompt(0), Vault(1), Variable(2), Inherit(3), None(4)
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

        // Set window mode
        conn.window_mode = WindowMode::from_index(self.window_mode_dropdown.selected());
        conn.remember_window_position = self.remember_position_check.is_active();

        // Set custom properties (filter out empty names)
        conn.custom_properties = self
            .custom_properties
            .iter()
            .filter(|p| !p.name.trim().is_empty())
            .cloned()
            .collect();

        // Set WOL config if enabled
        conn.wol_config = self.build_wol_config();

        // Set group from dropdown
        let selected_idx = self.group_dropdown.selected() as usize;
        let groups_data = self.groups_data.borrow();
        if selected_idx < groups_data.len() {
            conn.group_id = groups_data[selected_idx].0;
        }

        if let Some(id) = *self.editing_id.borrow() {
            conn.id = id;
        }

        // Extract password if user entered one (for Vault source only)
        let password_source_idx = self.password_source_dropdown.selected();
        let password = if password_source_idx == 1 {
            let pwd = self.password_entry.text().to_string();
            if pwd.is_empty() {
                None
            } else {
                Some(secrecy::SecretString::from(pwd))
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

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let port = self.wol_port_spin.value() as u16;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wait_seconds = self.wol_wait_spin.value() as u32;

        Some(WolConfig {
            mac_address,
            broadcast_address,
            port,
            wait_seconds,
        })
    }

    fn build_pre_connect_task(&self) -> Option<ConnectionTask> {
        if !self.pre_connect_enabled_check.is_active() {
            return None;
        }

        let command = self.pre_connect_command_entry.text().trim().to_string();
        if command.is_empty() {
            return None;
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let timeout_ms = self.pre_connect_timeout_spin.value() as u32;
        let timeout = if timeout_ms > 0 {
            Some(timeout_ms)
        } else {
            None
        };

        let condition = TaskCondition {
            only_first_in_folder: self.pre_connect_first_only_check.is_active(),
            only_last_in_folder: false,
        };

        let mut task = ConnectionTask::new_pre_connect(command)
            .with_condition(condition)
            .with_abort_on_failure(self.pre_connect_abort_check.is_active());

        if let Some(t) = timeout {
            task = task.with_timeout(t);
        }

        Some(task)
    }

    fn build_post_disconnect_task(&self) -> Option<ConnectionTask> {
        if !self.post_disconnect_enabled_check.is_active() {
            return None;
        }

        let command = self.post_disconnect_command_entry.text().trim().to_string();
        if command.is_empty() {
            return None;
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let timeout_ms = self.post_disconnect_timeout_spin.value() as u32;
        let timeout = if timeout_ms > 0 {
            Some(timeout_ms)
        } else {
            None
        };

        let condition = TaskCondition {
            only_first_in_folder: false,
            only_last_in_folder: self.post_disconnect_last_only_check.is_active(),
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
                    ssh_public_key_file: PathBuf::new(),
                    session_ttl: 1800,
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

    fn build_ssh_config(&self) -> SshConfig {
        let auth_method = match self.ssh_auth_dropdown.selected() {
            1 => SshAuthMethod::PublicKey,
            2 => SshAuthMethod::KeyboardInteractive,
            3 => SshAuthMethod::Agent,
            4 => SshAuthMethod::SecurityKey,
            _ => SshAuthMethod::Password, // 0 and any other value default to Password
        };

        // Build key_source based on dropdown selection
        let (key_source, key_path, agent_key_fingerprint) =
            match self.ssh_key_source_dropdown.selected() {
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
                2 => {
                    // Agent source
                    let keys = self.ssh_agent_keys.borrow();
                    let selected_idx = self.ssh_agent_key_dropdown.selected() as usize;
                    if selected_idx < keys.len() {
                        let key = &keys[selected_idx];
                        (
                            SshKeySource::Agent {
                                fingerprint: key.fingerprint.clone(),
                                comment: key.comment.clone(),
                            },
                            None,
                            Some(key.fingerprint.clone()),
                        )
                    } else {
                        (SshKeySource::Default, None, None)
                    }
                }
                _ => {
                    // Default (0) or any other value
                    (SshKeySource::Default, None, None)
                }
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

        let custom_options = Self::parse_custom_options(&self.ssh_options_entry.text());

        SshConfig {
            auth_method,
            key_path,
            key_source,
            agent_key_fingerprint,
            identities_only: self.ssh_identities_only.is_active(),
            proxy_jump: proxy_jump_opt,
            jump_host_id, // Add this field
            use_control_master: self.ssh_control_master.is_active(),
            agent_forwarding: self.ssh_agent_forwarding.is_active(),
            waypipe: self.ssh_waypipe.is_active(),
            x11_forwarding: self.ssh_x11_forwarding.is_active(),
            compression: self.ssh_compression.is_active(),
            custom_options,
            startup_command,
            sftp_enabled: true,
            port_forwards: self.ssh_port_forwards.borrow().clone(),
        }
    }

    fn build_rdp_config(&self) -> RdpConfig {
        let client_mode = RdpClientMode::from_index(self.rdp_client_mode_dropdown.selected());
        let performance_mode =
            RdpPerformanceMode::from_index(self.rdp_performance_mode_dropdown.selected());

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
                Some(rustconn_core::models::RdpGateway {
                    hostname: text.trim().to_string(),
                    port: 443,
                    username: None,
                })
            }
        };

        let custom_args = Self::parse_args(&self.rdp_custom_args_entry.text());

        let shared_folders = self.rdp_shared_folders.borrow().clone();

        RdpConfig {
            client_mode,
            performance_mode,
            resolution,
            color_depth,
            audio_redirect: self.rdp_audio_check.is_active(),
            gateway,
            shared_folders,
            custom_args,
            keyboard_layout: dropdown_index_to_klid(self.rdp_keyboard_layout_dropdown.selected()),
            scale_override: ScaleOverride::from_index(self.rdp_scale_override_dropdown.selected()),
        }
    }

    fn build_vnc_config(&self) -> VncConfig {
        let client_mode = VncClientMode::from_index(self.vnc_client_mode_dropdown.selected());
        let performance_mode =
            VncPerformanceMode::from_index(self.vnc_performance_mode_dropdown.selected());

        let encoding = {
            let text = self.vnc_encoding_entry.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_string())
            }
        };

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let compression = Some(self.vnc_compression_spin.value() as u8);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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

        SpiceConfig {
            tls_enabled: self.spice_tls_check.is_active(),
            ca_cert_path,
            skip_cert_verify: self.spice_skip_verify_check.is_active(),
            usb_redirection: self.spice_usb_check.is_active(),
            shared_folders: self.spice_shared_folders.borrow().clone(),
            clipboard_enabled: self.spice_clipboard_check.is_active(),
            image_compression,
            proxy: None,
        }
    }

    fn parse_custom_options(text: &str) -> HashMap<String, String> {
        let mut options = HashMap::new();
        if text.trim().is_empty() {
            return options;
        }
        for part in text.split(',') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim().to_string();
                if !key.is_empty() {
                    options.insert(key, value);
                }
            }
        }
        options
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
