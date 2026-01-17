//! Connection dialog for creating and editing connections
//!
//! Provides a GTK4 dialog with protocol-specific fields, input validation,
//! and portal integration for file selection (SSH keys).
//!
//! Updated for GTK 4.10+ compatibility using `DropDown` instead of `ComboBoxText`
//! and Window instead of Dialog.
//!
//! Uses `adw::ViewStack` with `adw::ViewSwitcher` for proper libadwaita theming.

// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(clippy::similar_names)]

use crate::alert;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, Grid, Label, ListBox,
    ListBoxRow, Orientation, PasswordEntry, ScrolledWindow, SpinButton, Stack, StringList,
    TextView, WrapMode,
};
use libadwaita as adw;
use rustconn_core::automation::{ConnectionTask, ExpectRule, TaskCondition};
use rustconn_core::models::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    Connection, CustomProperty, GcpIapConfig, GenericZeroTrustConfig, OciBastionConfig,
    PasswordSource, PropertyType, ProtocolConfig, RdpClientMode, RdpConfig, RdpPerformanceMode,
    Resolution, SharedFolder, SpiceConfig, SpiceImageCompression, SshAuthMethod, SshConfig,
    SshKeySource, TailscaleSshConfig, TeleportConfig, VncClientMode, VncConfig, VncPerformanceMode,
    WindowMode, ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
};
use rustconn_core::session::LogConfig;
use rustconn_core::variables::Variable;
use rustconn_core::wol::{
    MacAddress, WolConfig, DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;

/// Connection dialog for creating/editing connections
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle and signal handlers
pub struct ConnectionDialog {
    window: adw::Window,
    /// Header bar save button - stored for potential future use
    /// (e.g., enabling/disabling based on validation state)
    save_button: Button,
    /// Test connection button
    test_button: Button,
    // Basic fields
    name_entry: Entry,
    description_view: TextView,
    host_entry: Entry,
    port_spin: SpinButton,
    username_entry: Entry,
    tags_entry: Entry,
    protocol_dropdown: DropDown,
    protocol_stack: Stack,
    // Password source selection
    password_source_dropdown: DropDown,
    // Password entry and KeePass buttons
    password_entry: Entry,
    save_to_keepass_button: Button,
    load_from_keepass_button: Button,
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
    ssh_startup_entry: Entry,
    ssh_options_entry: Entry,
    // RDP fields
    rdp_client_mode_dropdown: DropDown,
    rdp_performance_mode_dropdown: DropDown,
    rdp_width_spin: SpinButton,
    rdp_height_spin: SpinButton,
    rdp_color_dropdown: DropDown,
    rdp_audio_check: CheckButton,
    rdp_gateway_entry: Entry,
    rdp_shared_folders: Rc<RefCell<Vec<SharedFolder>>>,
    rdp_shared_folders_list: gtk4::ListBox,
    rdp_custom_args_entry: Entry,
    // VNC fields
    vnc_client_mode_dropdown: DropDown,
    vnc_performance_mode_dropdown: DropDown,
    vnc_encoding_entry: Entry,
    vnc_compression_spin: SpinButton,
    vnc_quality_spin: SpinButton,
    vnc_view_only_check: CheckButton,
    vnc_scaling_check: CheckButton,
    vnc_clipboard_check: CheckButton,
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
    zt_aws_target_entry: Entry,
    zt_aws_profile_entry: Entry,
    zt_aws_region_entry: Entry,
    // GCP IAP fields
    zt_gcp_instance_entry: Entry,
    zt_gcp_zone_entry: Entry,
    zt_gcp_project_entry: Entry,
    // Azure Bastion fields
    zt_azure_bastion_resource_id_entry: Entry,
    zt_azure_bastion_rg_entry: Entry,
    zt_azure_bastion_name_entry: Entry,
    // Azure SSH fields
    zt_azure_ssh_vm_entry: Entry,
    zt_azure_ssh_rg_entry: Entry,
    // OCI Bastion fields
    zt_oci_bastion_id_entry: Entry,
    zt_oci_target_id_entry: Entry,
    zt_oci_target_ip_entry: Entry,
    // Cloudflare Access fields
    zt_cf_hostname_entry: Entry,
    // Teleport fields
    zt_teleport_host_entry: Entry,
    zt_teleport_cluster_entry: Entry,
    // Tailscale SSH fields
    zt_tailscale_host_entry: Entry,
    // Boundary fields
    zt_boundary_target_entry: Entry,
    zt_boundary_addr_entry: Entry,
    // Generic fields
    zt_generic_command_entry: Entry,
    // Custom args for all providers
    zt_custom_args_entry: Entry,
    // Variables fields
    variables_list: ListBox,
    variables_rows: Rc<RefCell<Vec<LocalVariableRow>>>,
    /// Button to add new variables - wired up in `wire_add_variable_button()`
    add_variable_button: Button,
    global_variables: Rc<RefCell<Vec<Variable>>>,
    // Logging fields
    logging_enabled_check: CheckButton,
    logging_path_entry: Entry,
    logging_timestamp_dropdown: DropDown,
    logging_max_size_spin: SpinButton,
    logging_retention_spin: SpinButton,
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
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let (window, header, save_btn, test_btn) = Self::create_window_with_header(parent);
        let view_stack = Self::create_view_stack(&window, &header);

        // === Basic Tab ===
        let (
            basic_grid,
            name_entry,
            description_view,
            host_entry,
            host_label,
            port_spin,
            port_label,
            username_entry,
            username_label,
            tags_entry,
            tags_label,
            protocol_dropdown,
            password_source_dropdown,
            password_source_label,
            password_entry,
            password_entry_label,
            load_from_keepass_button,
            save_to_keepass_button,
            group_dropdown,
        ) = Self::create_basic_tab();
        // Wrap basic grid in ScrolledWindow for consistent styling
        let basic_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&basic_grid)
            .build();
        view_stack
            .add_titled(&basic_scrolled, Some("basic"), "Basic")
            .set_icon_name(Some("document-properties-symbolic"));

        // === Protocol-specific Tab ===
        let protocol_stack = Self::create_protocol_stack(&view_stack);

        // SSH options
        let (
            ssh_box,
            ssh_auth_dropdown,
            ssh_key_source_dropdown,
            ssh_key_entry,
            ssh_key_button,
            ssh_agent_key_dropdown,
            ssh_jump_host_dropdown, // New field
            ssh_proxy_entry,
            ssh_identities_only,
            ssh_control_master,
            ssh_agent_forwarding,
            ssh_startup_entry,
            ssh_options_entry,
        ) = Self::create_ssh_options();
        protocol_stack.add_named(&ssh_box, Some("ssh"));

        // Storage for agent keys (populated when dialog is shown)
        let ssh_agent_keys: Rc<RefCell<Vec<rustconn_core::ssh_agent::AgentKey>>> =
            Rc::new(RefCell::new(Vec::new()));

        // RDP options
        let (
            rdp_box,
            rdp_client_mode_dropdown,
            rdp_performance_mode_dropdown,
            rdp_width_spin,
            rdp_height_spin,
            rdp_color_dropdown,
            rdp_audio_check,
            rdp_gateway_entry,
            rdp_shared_folders,
            rdp_shared_folders_list,
            rdp_custom_args_entry,
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
            &password_entry,
            &password_entry_label,
            &load_from_keepass_button,
            &save_to_keepass_button,
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
            .add_titled(&data_tab, Some("data"), "Data")
            .set_icon_name(Some("accessories-text-editor-symbolic"));

        let variables_rows: Rc<RefCell<Vec<LocalVariableRow>>> = Rc::new(RefCell::new(Vec::new()));
        let global_variables: Rc<RefCell<Vec<Variable>>> = Rc::new(RefCell::new(Vec::new()));
        let custom_properties: Rc<RefCell<Vec<CustomProperty>>> = Rc::new(RefCell::new(Vec::new()));

        // === Logging Tab ===
        let (
            logging_tab,
            logging_enabled_check,
            logging_path_entry,
            logging_timestamp_dropdown,
            logging_max_size_spin,
            logging_retention_spin,
        ) = Self::create_logging_tab();
        view_stack
            .add_titled(&logging_tab, Some("logging"), "Logging")
            .set_icon_name(Some("document-save-symbolic"));

        // === Automation Tab (Expect Rules + Tasks) ===
        let (
            automation_tab,
            expect_rules_list,
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
        ) = Self::create_automation_combined_tab();
        view_stack
            .add_titled(&automation_tab, Some("automation"), "Automation")
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
            .add_titled(&advanced_tab, Some("advanced"), "Advanced")
            .set_icon_name(Some("preferences-system-symbolic"));

        // Wire up add variable button
        Self::wire_add_variable_button(&add_variable_button, &variables_list, &variables_rows);

        // Wire up add expect rule button
        Self::wire_add_expect_rule_button(
            &add_expect_rule_button,
            &expect_rules_list,
            &expect_rules,
        );

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
            &editing_id,
            &name_entry,
            &description_view,
            &host_entry,
            &port_spin,
            &username_entry,
            &tags_entry,
            &protocol_dropdown,
            &password_source_dropdown,
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
            &ssh_startup_entry,
            &ssh_options_entry,
            &rdp_client_mode_dropdown,
            &rdp_performance_mode_dropdown,
            &rdp_width_spin,
            &rdp_height_spin,
            &rdp_color_dropdown,
            &rdp_audio_check,
            &rdp_gateway_entry,
            &rdp_shared_folders,
            &rdp_custom_args_entry,
            &vnc_client_mode_dropdown,
            &vnc_performance_mode_dropdown,
            &vnc_encoding_entry,
            &vnc_compression_spin,
            &vnc_quality_spin,
            &vnc_view_only_check,
            &vnc_scaling_check,
            &vnc_clipboard_check,
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
            &variables_rows,
            &logging_enabled_check,
            &logging_path_entry,
            &logging_timestamp_dropdown,
            &logging_max_size_spin,
            &logging_retention_spin,
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
            name_entry,
            description_view,
            host_entry,
            port_spin,
            username_entry,
            tags_entry,
            protocol_dropdown,
            protocol_stack,
            password_source_dropdown,
            password_entry,
            save_to_keepass_button,
            load_from_keepass_button,
            group_dropdown,
            groups_data: Rc::new(RefCell::new(vec![(None, "(Root)".to_string())])),
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
            ssh_startup_entry,
            ssh_options_entry,
            rdp_client_mode_dropdown,
            rdp_performance_mode_dropdown,
            rdp_width_spin,
            rdp_height_spin,
            rdp_color_dropdown,
            rdp_audio_check,
            rdp_gateway_entry,
            rdp_shared_folders,
            rdp_shared_folders_list,
            rdp_custom_args_entry,
            vnc_client_mode_dropdown,
            vnc_performance_mode_dropdown,
            vnc_encoding_entry,
            vnc_compression_spin,
            vnc_quality_spin,
            vnc_view_only_check,
            vnc_scaling_check,
            vnc_clipboard_check,
            vnc_custom_args_entry,
            spice_tls_check,
            variables_list,
            variables_rows,
            add_variable_button,
            global_variables,
            logging_enabled_check,
            logging_path_entry,
            logging_timestamp_dropdown,
            logging_max_size_spin,
            logging_retention_spin,
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
                    "Zero Trust Connection Test",
                    "Zero Trust connections require provider-specific authentication.\n\n\
                     To test the connection:\n\
                     1. Save the connection first\n\
                     2. Use the Connect button to initiate the connection\n\
                     3. The provider CLI will handle authentication",
                );
                return;
            }

            if name.trim().is_empty() || host.trim().is_empty() {
                alert::show_error(
                    &window,
                    "Connection Test Failed",
                    "Please fill in required fields (name and host)",
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
            btn.set_label("Testing...");

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
                    let Ok(rt) = tokio::runtime::Runtime::new() else {
                        return rustconn_core::testing::TestResult::failure(
                            conn_id,
                            conn_name.clone(),
                            "Failed to create tokio runtime",
                        );
                    };
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
                    };
                    let mut test_conn = rustconn_core::models::Connection::new(
                        conn_name,
                        host,
                        port,
                        protocol_config,
                    );
                    test_conn.id = conn_id;

                    rt.block_on(tester.test_connection(&test_conn))
                },
                std::time::Duration::from_secs(15),
                move |result: Option<rustconn_core::testing::TestResult>| {
                    // Update UI
                    test_button_clone.set_sensitive(true);
                    test_button_clone.set_label("Test");

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
                                .unwrap_or_else(|| "Unknown error".to_string());
                            alert::show_error(&window_clone, "Connection Test Failed", &error);
                        }
                        None => {
                            alert::show_error(
                                &window_clone,
                                "Connection Test Failed",
                                "Test timed out",
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
            .title("New Connection")
            .modal(true)
            .default_width(750)
            .default_height(650)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar with Close/Test/Create buttons (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        let close_btn = Button::builder().label("Close").build();
        let test_btn = Button::builder()
            .label("Test")
            .tooltip_text("Test connection")
            .build();
        let save_btn = Button::builder()
            .label("Create")
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
            .add_titled(&scrolled, Some("protocol"), "Protocol")
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
        password_entry: &Entry,
        password_label: &Label,
        load_from_keepass_button: &Button,
        save_to_keepass_button: &Button,
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
        let password_entry = password_entry.clone();
        let password_label = password_label.clone();
        let load_from_keepass_button = load_from_keepass_button.clone();
        let save_to_keepass_button = save_to_keepass_button.clone();

        dropdown.connect_selected_notify(move |dropdown| {
            let protocols = ["ssh", "rdp", "vnc", "spice", "zerotrust"];
            let selected = dropdown.selected() as usize;
            if selected < protocols.len() {
                let protocol_id = protocols[selected];
                stack_clone.set_visible_child_name(protocol_id);
                let default_port = Self::get_default_port(protocol_id);
                if Self::is_default_port(port_clone.value()) {
                    port_clone.set_value(default_port);
                }

                let is_zerotrust = protocol_id == "zerotrust";
                let visible = !is_zerotrust;

                host_entry.set_visible(visible);
                host_label.set_visible(visible);
                port_clone.set_visible(visible);
                port_label.set_visible(visible);
                username_entry.set_visible(visible);
                username_label.set_visible(visible);
                tags_entry.set_visible(visible);
                tags_label.set_visible(visible);
                password_source_dropdown.set_visible(visible);
                password_source_label.set_visible(visible);
                password_entry.set_visible(visible);
                password_label.set_visible(visible);
                load_from_keepass_button.set_visible(visible);
                save_to_keepass_button.set_visible(visible);
            }
        });
    }

    /// Returns the default port for a protocol
    fn get_default_port(protocol_id: &str) -> f64 {
        match protocol_id {
            "rdp" => 3389.0,
            "vnc" | "spice" => 5900.0,
            "zerotrust" => 0.0,
            _ => 22.0,
        }
    }

    /// Checks if the port value is one of the default ports
    fn is_default_port(port: f64) -> bool {
        const EPSILON: f64 = 0.5;
        (port - 22.0).abs() < EPSILON
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
        editing_id: &Rc<RefCell<Option<Uuid>>>,
        name_entry: &Entry,
        description_view: &TextView,
        host_entry: &Entry,
        port_spin: &SpinButton,
        username_entry: &Entry,
        tags_entry: &Entry,
        protocol_dropdown: &DropDown,
        password_source_dropdown: &DropDown,
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
        ssh_startup_entry: &Entry,
        ssh_options_entry: &Entry,
        rdp_client_mode_dropdown: &DropDown,
        rdp_performance_mode_dropdown: &DropDown,
        rdp_width_spin: &SpinButton,
        rdp_height_spin: &SpinButton,
        rdp_color_dropdown: &DropDown,
        rdp_audio_check: &CheckButton,
        rdp_gateway_entry: &Entry,
        rdp_shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
        rdp_custom_args_entry: &Entry,
        vnc_client_mode_dropdown: &DropDown,
        vnc_performance_mode_dropdown: &DropDown,
        vnc_encoding_entry: &Entry,
        vnc_compression_spin: &SpinButton,
        vnc_quality_spin: &SpinButton,
        vnc_view_only_check: &CheckButton,
        vnc_scaling_check: &CheckButton,
        vnc_clipboard_check: &CheckButton,
        vnc_custom_args_entry: &Entry,
        spice_tls_check: &CheckButton,
        spice_ca_cert_entry: &Entry,
        spice_skip_verify_check: &CheckButton,
        spice_usb_check: &CheckButton,
        spice_clipboard_check: &CheckButton,
        spice_compression_dropdown: &DropDown,
        spice_shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
        zt_provider_dropdown: &DropDown,
        zt_aws_target_entry: &Entry,
        zt_aws_profile_entry: &Entry,
        zt_aws_region_entry: &Entry,
        zt_gcp_instance_entry: &Entry,
        zt_gcp_zone_entry: &Entry,
        zt_gcp_project_entry: &Entry,
        zt_azure_bastion_resource_id_entry: &Entry,
        zt_azure_bastion_rg_entry: &Entry,
        zt_azure_bastion_name_entry: &Entry,
        zt_azure_ssh_vm_entry: &Entry,
        zt_azure_ssh_rg_entry: &Entry,
        zt_oci_bastion_id_entry: &Entry,
        zt_oci_target_id_entry: &Entry,
        zt_oci_target_ip_entry: &Entry,
        zt_cf_hostname_entry: &Entry,
        zt_teleport_host_entry: &Entry,
        zt_teleport_cluster_entry: &Entry,
        zt_tailscale_host_entry: &Entry,
        zt_boundary_target_entry: &Entry,
        zt_boundary_addr_entry: &Entry,
        zt_generic_command_entry: &Entry,
        zt_custom_args_entry: &Entry,
        variables_rows: &Rc<RefCell<Vec<LocalVariableRow>>>,
        logging_enabled_check: &CheckButton,
        logging_path_entry: &Entry,
        logging_timestamp_dropdown: &DropDown,
        logging_max_size_spin: &SpinButton,
        logging_retention_spin: &SpinButton,
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
        let name_entry = name_entry.clone();
        let description_view = description_view.clone();
        let host_entry = host_entry.clone();
        let port_spin = port_spin.clone();
        let username_entry = username_entry.clone();
        let tags_entry = tags_entry.clone();
        let protocol_dropdown = protocol_dropdown.clone();
        let password_source_dropdown = password_source_dropdown.clone();
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
        let ssh_startup_entry = ssh_startup_entry.clone();
        let ssh_options_entry = ssh_options_entry.clone();
        let rdp_client_mode_dropdown = rdp_client_mode_dropdown.clone();
        let rdp_width_spin = rdp_width_spin.clone();
        let rdp_height_spin = rdp_height_spin.clone();
        let rdp_color_dropdown = rdp_color_dropdown.clone();
        let rdp_audio_check = rdp_audio_check.clone();
        let rdp_gateway_entry = rdp_gateway_entry.clone();
        let rdp_shared_folders = rdp_shared_folders.clone();
        let rdp_custom_args_entry = rdp_custom_args_entry.clone();
        let rdp_performance_mode_dropdown = rdp_performance_mode_dropdown.clone();
        let vnc_client_mode_dropdown = vnc_client_mode_dropdown.clone();
        let vnc_encoding_entry = vnc_encoding_entry.clone();
        let vnc_compression_spin = vnc_compression_spin.clone();
        let vnc_quality_spin = vnc_quality_spin.clone();
        let vnc_view_only_check = vnc_view_only_check.clone();
        let vnc_scaling_check = vnc_scaling_check.clone();
        let vnc_clipboard_check = vnc_clipboard_check.clone();
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
        let variables_rows = variables_rows.clone();
        let logging_enabled_check = logging_enabled_check.clone();
        let logging_path_entry = logging_path_entry.clone();
        let logging_timestamp_dropdown = logging_timestamp_dropdown.clone();
        let logging_max_size_spin = logging_max_size_spin.clone();
        let logging_retention_spin = logging_retention_spin.clone();
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
                description_view: &description_view,
                host_entry: &host_entry,
                port_spin: &port_spin,
                username_entry: &username_entry,
                tags_entry: &tags_entry,
                protocol_dropdown: &protocol_dropdown,
                password_source_dropdown: &password_source_dropdown,
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
                ssh_startup_entry: &ssh_startup_entry,
                ssh_options_entry: &ssh_options_entry,
                rdp_client_mode_dropdown: &rdp_client_mode_dropdown,
                rdp_width_spin: &rdp_width_spin,
                rdp_height_spin: &rdp_height_spin,
                rdp_color_dropdown: &rdp_color_dropdown,
                rdp_audio_check: &rdp_audio_check,
                rdp_gateway_entry: &rdp_gateway_entry,
                rdp_shared_folders: &rdp_shared_folders,
                rdp_custom_args_entry: &rdp_custom_args_entry,
                vnc_client_mode_dropdown: &vnc_client_mode_dropdown,
                vnc_encoding_entry: &vnc_encoding_entry,
                vnc_compression_spin: &vnc_compression_spin,
                vnc_quality_spin: &vnc_quality_spin,
                vnc_view_only_check: &vnc_view_only_check,
                vnc_scaling_check: &vnc_scaling_check,
                vnc_clipboard_check: &vnc_clipboard_check,
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
                local_variables: &local_variables,
                logging_enabled_check: &logging_enabled_check,
                logging_path_entry: &logging_path_entry,
                logging_timestamp_dropdown: &logging_timestamp_dropdown,
                logging_max_size_spin: &logging_max_size_spin,
                logging_retention_spin: &logging_retention_spin,
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

            if let Some(conn) = data.build_connection() {
                if let Some(ref cb) = *on_save.borrow() {
                    cb(Some(conn));
                }
                window.close();
            }
        });
    }

    #[allow(clippy::type_complexity)]
    fn create_basic_tab() -> (
        GtkBox,
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
        DropDown,
        DropDown,
        Label,
        Entry,
        Label,
        Button,
        Button,
        DropDown,
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
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .placeholder_text("Connection name")
            .hexpand(true)
            .build();
        grid.attach(&name_label, 0, row, 1, 1);
        grid.attach(&name_entry, 1, row, 2, 1);
        row += 1;

        // Protocol
        let protocol_label_grid = Label::builder()
            .label("Protocol:")
            .halign(gtk4::Align::End)
            .build();
        let protocol_list = StringList::new(&["SSH", "RDP", "VNC", "SPICE", "Zero Trust"]);
        let protocol_dropdown = DropDown::builder().model(&protocol_list).build();
        protocol_dropdown.set_selected(0);
        grid.attach(&protocol_label_grid, 0, row, 1, 1);
        grid.attach(&protocol_dropdown, 1, row, 2, 1);
        row += 1;

        // Host
        let host_label = Label::builder()
            .label("Host:")
            .halign(gtk4::Align::End)
            .build();
        let host_entry = Entry::builder()
            .placeholder_text("hostname or IP")
            .hexpand(true)
            .build();
        grid.attach(&host_label, 0, row, 1, 1);
        grid.attach(&host_entry, 1, row, 2, 1);
        row += 1;

        // Port with description
        let port_label = Label::builder()
            .label("Port:")
            .halign(gtk4::Align::End)
            .build();
        let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_spin = SpinButton::builder()
            .adjustment(&port_adj)
            .climb_rate(1.0)
            .digits(0)
            .build();
        let port_desc = Label::builder()
            .label("SSH, Well-Known")
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
            .label("Username:")
            .halign(gtk4::Align::End)
            .build();
        let current_user = std::env::var("USER").unwrap_or_default();
        let username_entry = Entry::builder()
            .placeholder_text(&format!("(default: {current_user})"))
            .hexpand(true)
            .build();
        grid.attach(&username_label, 0, row, 1, 1);
        grid.attach(&username_entry, 1, row, 2, 1);
        row += 1;

        // Password Source
        let password_source_label = Label::builder()
            .label("Password:")
            .halign(gtk4::Align::End)
            .build();
        let password_source_list =
            StringList::new(&["Prompt", "Stored", "KeePass", "Keyring", "Inherit", "None"]);
        let password_source_dropdown = DropDown::builder().model(&password_source_list).build();
        password_source_dropdown.set_selected(0);
        grid.attach(&password_source_label, 0, row, 1, 1);
        grid.attach(&password_source_dropdown, 1, row, 2, 1);
        row += 1;

        // Password with KeePass buttons
        let password_entry_label = Label::builder()
            .label("Password:")
            .halign(gtk4::Align::End)
            .build();
        let password_entry = Entry::builder()
            .placeholder_text("Password")
            .hexpand(true)
            .visibility(false)
            .build();
        let save_to_keepass_button = Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text("Save password to vault")
            .build();
        let load_from_keepass_button = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Load password from vault")
            .build();
        let password_box = GtkBox::new(Orientation::Horizontal, 4);
        password_box.append(&password_entry);
        password_box.append(&save_to_keepass_button);
        password_box.append(&load_from_keepass_button);
        grid.attach(&password_entry_label, 0, row, 1, 1);
        grid.attach(&password_box, 1, row, 2, 1);
        row += 1;

        // Tags
        let tags_label = Label::builder()
            .label("Tags:")
            .halign(gtk4::Align::End)
            .build();
        let tags_entry = Entry::builder()
            .placeholder_text("tag1, tag2, ...")
            .hexpand(true)
            .build();
        grid.attach(&tags_label, 0, row, 1, 1);
        grid.attach(&tags_entry, 1, row, 2, 1);
        row += 1;

        // Group
        let group_label = Label::builder()
            .label("Group:")
            .halign(gtk4::Align::End)
            .build();
        let group_list = StringList::new(&["(Root)"]);
        let group_dropdown = DropDown::builder().model(&group_list).build();
        grid.attach(&group_label, 0, row, 1, 1);
        grid.attach(&group_dropdown, 1, row, 2, 1);
        row += 1;

        // Description
        let desc_label = Label::builder()
            .label("Description:")
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
            .min_content_height(60)
            .hexpand(true)
            .child(&description_view)
            .build();
        grid.attach(&desc_label, 0, row, 1, 1);
        grid.attach(&desc_scrolled, 1, row, 2, 1);

        (
            vbox,
            name_entry,
            description_view,
            host_entry,
            host_label,
            port_spin,
            port_label,
            username_entry,
            username_label,
            tags_entry,
            tags_label,
            protocol_dropdown,
            password_source_dropdown,
            password_source_label,
            password_entry,
            password_entry_label,
            load_from_keepass_button,
            save_to_keepass_button,
            group_dropdown,
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

    /// Creates the SSH options panel using libadwaita components following GNOME HIG.
    ///
    /// Layout:
    /// - Authentication group: Auth Method, Key Source, Key File, Agent Key
    /// - Connection group: ProxyJump, IdentitiesOnly, ControlMaster
    /// - Session group: Agent Forwarding, Startup Command, Custom Options
    #[allow(clippy::type_complexity)]
    fn create_ssh_options() -> (
        GtkBox,
        DropDown,
        DropDown,
        Entry,
        Button,
        DropDown,
        DropDown, // Jump Host DropDown
        Entry,
        CheckButton,
        CheckButton,
        CheckButton,
        Entry,
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

        // === Authentication Group ===
        let auth_group = adw::PreferencesGroup::builder()
            .title("Authentication")
            .build();

        // Auth method dropdown
        let auth_list = StringList::new(&[
            "Password",
            "Public Key",
            "Keyboard Interactive",
            "SSH Agent",
        ]);
        let auth_dropdown = DropDown::new(Some(auth_list), gtk4::Expression::NONE);
        auth_dropdown.set_selected(0);

        let auth_row = adw::ActionRow::builder()
            .title("Method")
            .subtitle("How to authenticate with the server")
            .build();
        auth_row.add_suffix(&auth_dropdown);
        auth_group.add(&auth_row);

        // Key source dropdown
        let key_source_list = StringList::new(&["Default", "File", "Agent"]);
        let key_source_dropdown = DropDown::new(Some(key_source_list), gtk4::Expression::NONE);
        key_source_dropdown.set_selected(0);

        let key_source_row = adw::ActionRow::builder()
            .title("Key Source")
            .subtitle("Where to get the SSH key from")
            .build();
        key_source_row.add_suffix(&key_source_dropdown);
        auth_group.add(&key_source_row);

        // Key file entry with browse button
        let key_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Path to SSH key")
            .valign(gtk4::Align::Center)
            .build();
        let key_button = Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Browse for key file")
            .valign(gtk4::Align::Center)
            .build();

        let key_file_row = adw::ActionRow::builder()
            .title("Key File")
            .subtitle("Path to private key file")
            .build();
        key_file_row.add_suffix(&key_entry);
        key_file_row.add_suffix(&key_button);
        auth_group.add(&key_file_row);

        // Agent key dropdown
        let agent_key_list = StringList::new(&["(No keys loaded)"]);
        let agent_key_dropdown = DropDown::new(Some(agent_key_list), gtk4::Expression::NONE);
        agent_key_dropdown.set_selected(0);
        agent_key_dropdown.set_sensitive(false);
        agent_key_dropdown.set_hexpand(false);

        let agent_key_row = adw::ActionRow::builder()
            .title("Key")
            .subtitle("Select from SSH agent")
            .build();
        agent_key_row.add_suffix(&agent_key_dropdown);
        auth_group.add(&agent_key_row);

        content.append(&auth_group);

        // Connect key source dropdown to show/hide appropriate fields
        let key_file_row_clone = key_file_row.clone();
        let agent_key_row_clone = agent_key_row.clone();
        let key_entry_clone = key_entry.clone();
        let key_button_clone = key_button.clone();
        let agent_key_dropdown_clone = agent_key_dropdown.clone();
        key_source_dropdown.connect_selected_notify(move |dropdown| {
            let selected = dropdown.selected();
            match selected {
                0 => {
                    // Default - hide both rows
                    key_file_row_clone.set_visible(false);
                    agent_key_row_clone.set_visible(false);
                    key_entry_clone.set_sensitive(false);
                    key_button_clone.set_sensitive(false);
                    agent_key_dropdown_clone.set_sensitive(false);
                }
                1 => {
                    // File - show file row, hide agent row
                    key_file_row_clone.set_visible(true);
                    agent_key_row_clone.set_visible(false);
                    key_entry_clone.set_sensitive(true);
                    key_button_clone.set_sensitive(true);
                    agent_key_dropdown_clone.set_sensitive(false);
                }
                2 => {
                    // Agent - hide file row, show agent row
                    key_file_row_clone.set_visible(false);
                    agent_key_row_clone.set_visible(true);
                    key_entry_clone.set_sensitive(false);
                    key_button_clone.set_sensitive(false);
                    agent_key_dropdown_clone.set_sensitive(true);
                }
                _ => {}
            }
        });

        // Set initial state (Default selected - hide both key rows)
        key_file_row.set_visible(false);
        agent_key_row.set_visible(false);
        key_entry.set_sensitive(false);
        key_button.set_sensitive(false);
        agent_key_dropdown.set_sensitive(false);

        // Connect auth method dropdown to show/hide key-related rows
        // Password (0) - hide all key rows
        // Public Key (1) - show key source
        // Keyboard Interactive (2) - show key source
        // SSH Agent (3) - hide key source, show agent key directly
        let key_source_row_clone = key_source_row.clone();
        let key_file_row_for_auth = key_file_row.clone();
        let agent_key_row_for_auth = agent_key_row.clone();
        let agent_key_dropdown_for_auth = agent_key_dropdown.clone();
        auth_dropdown.connect_selected_notify(move |dropdown| {
            let selected = dropdown.selected();
            match selected {
                0 => {
                    // Password - hide all key-related rows
                    key_source_row_clone.set_visible(false);
                    key_file_row_for_auth.set_visible(false);
                    agent_key_row_for_auth.set_visible(false);
                }
                3 => {
                    // SSH Agent - hide key source, show agent key directly
                    key_source_row_clone.set_visible(false);
                    key_file_row_for_auth.set_visible(false);
                    agent_key_row_for_auth.set_visible(true);
                    agent_key_dropdown_for_auth.set_sensitive(true);
                }
                _ => {
                    // Public Key, Keyboard Interactive - show key source row
                    key_source_row_clone.set_visible(true);
                    // Key file/agent rows visibility is controlled by key_source_dropdown
                }
            }
        });

        // Set initial state for auth method (Password selected - hide key source)
        key_source_row.set_visible(false);

        // === Connection Options Group ===
        let connection_group = adw::PreferencesGroup::builder().title("Connection").build();

        // Jump Host dropdown
        let jump_host_list = StringList::new(&["(None)"]);
        let jump_host_dropdown = DropDown::new(Some(jump_host_list), gtk4::Expression::NONE);
        jump_host_dropdown.set_selected(0);

        let jump_host_row = adw::ActionRow::builder()
            .title("Jump Host")
            .subtitle("Connect via another SSH connection")
            .build();
        jump_host_row.add_suffix(&jump_host_dropdown);
        connection_group.add(&jump_host_row);

        // ProxyJump entry
        let proxy_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("user@jumphost")
            .valign(gtk4::Align::Center)
            .build();

        let proxy_row = adw::ActionRow::builder()
            .title("ProxyJump")
            .subtitle("Jump host for tunneling (-J)")
            .build();
        proxy_row.add_suffix(&proxy_entry);
        connection_group.add(&proxy_row);

        // IdentitiesOnly switch
        let identities_only = CheckButton::new();
        let identities_row = adw::ActionRow::builder()
            .title("Use Only Specified Key")
            .subtitle("Prevents trying other keys (IdentitiesOnly)")
            .activatable_widget(&identities_only)
            .build();
        identities_row.add_suffix(&identities_only);
        connection_group.add(&identities_row);

        // ControlMaster switch
        let control_master = CheckButton::new();
        let control_master_row = adw::ActionRow::builder()
            .title("Connection Multiplexing")
            .subtitle("Reuse connections (ControlMaster)")
            .activatable_widget(&control_master)
            .build();
        control_master_row.add_suffix(&control_master);
        connection_group.add(&control_master_row);

        content.append(&connection_group);

        // === Session Group ===
        let session_group = adw::PreferencesGroup::builder().title("Session").build();

        // Agent Forwarding switch
        let agent_forwarding = CheckButton::new();
        let agent_forwarding_row = adw::ActionRow::builder()
            .title("Agent Forwarding")
            .subtitle("Forward SSH agent to remote host (-A)")
            .activatable_widget(&agent_forwarding)
            .build();
        agent_forwarding_row.add_suffix(&agent_forwarding);
        session_group.add(&agent_forwarding_row);

        // Startup command entry
        let startup_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Command to run on connect")
            .valign(gtk4::Align::Center)
            .build();

        let startup_row = adw::ActionRow::builder()
            .title("Startup Command")
            .subtitle("Execute after connection established")
            .build();
        startup_row.add_suffix(&startup_entry);
        session_group.add(&startup_row);

        // Custom options entry
        let options_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("-o Key=Value")
            .valign(gtk4::Align::Center)
            .build();

        let options_row = adw::ActionRow::builder()
            .title("Custom Options")
            .subtitle("Additional SSH command-line options")
            .build();
        options_row.add_suffix(&options_entry);
        session_group.add(&options_row);

        content.append(&session_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            auth_dropdown,
            key_source_dropdown,
            key_entry,
            key_button,
            agent_key_dropdown,
            jump_host_dropdown,
            proxy_entry,
            identities_only,
            control_master,
            agent_forwarding,
            startup_entry,
            options_entry,
        )
    }

    /// Creates the RDP options panel using libadwaita components following GNOME HIG.
    #[allow(clippy::type_complexity)]
    fn create_rdp_options() -> (
        GtkBox,
        DropDown,
        DropDown,
        SpinButton,
        SpinButton,
        DropDown,
        CheckButton,
        Entry,
        Rc<RefCell<Vec<SharedFolder>>>,
        gtk4::ListBox,
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
        let display_group = adw::PreferencesGroup::builder().title("Display").build();

        // Client mode dropdown
        let client_mode_list = StringList::new(&[
            RdpClientMode::Embedded.display_name(),
            RdpClientMode::External.display_name(),
        ]);
        let client_mode_dropdown = DropDown::builder()
            .model(&client_mode_list)
            .valign(gtk4::Align::Center)
            .build();

        let client_mode_row = adw::ActionRow::builder()
            .title("Client Mode")
            .subtitle("Embedded renders in tab, External opens separate window")
            .build();
        client_mode_row.add_suffix(&client_mode_dropdown);
        display_group.add(&client_mode_row);

        // Performance mode dropdown
        let performance_mode_list = StringList::new(&[
            RdpPerformanceMode::Quality.display_name(),
            RdpPerformanceMode::Balanced.display_name(),
            RdpPerformanceMode::Speed.display_name(),
        ]);
        let performance_mode_dropdown = DropDown::builder()
            .model(&performance_mode_list)
            .valign(gtk4::Align::Center)
            .build();
        performance_mode_dropdown.set_selected(1); // Default to Balanced

        let performance_mode_row = adw::ActionRow::builder()
            .title("Performance Mode")
            .subtitle("Quality/speed tradeoff for image rendering")
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
            .title("Resolution")
            .subtitle("Width × Height in pixels")
            .build();
        resolution_row.add_suffix(&res_box);
        display_group.add(&resolution_row);

        // Color depth
        let color_list = StringList::new(&[
            "32-bit (True Color)",
            "24-bit",
            "16-bit (High Color)",
            "15-bit",
            "8-bit",
        ]);
        let color_dropdown = DropDown::new(Some(color_list), gtk4::Expression::NONE);
        color_dropdown.set_selected(0);
        color_dropdown.set_valign(gtk4::Align::Center);

        let color_row = adw::ActionRow::builder()
            .title("Color Depth")
            .subtitle("Higher values provide better quality")
            .build();
        color_row.add_suffix(&color_dropdown);
        display_group.add(&color_row);

        // Connect client mode dropdown to show/hide resolution/color rows
        // Embedded (0) - hide resolution and color depth (dynamic resolution)
        // External (1) - show resolution and color depth
        let resolution_row_clone = resolution_row.clone();
        let color_row_clone = color_row.clone();
        client_mode_dropdown.connect_selected_notify(move |dropdown| {
            let is_embedded = dropdown.selected() == 0;
            resolution_row_clone.set_visible(!is_embedded);
            color_row_clone.set_visible(!is_embedded);
        });

        // Set initial state (Embedded - hide resolution/color)
        resolution_row.set_visible(false);
        color_row.set_visible(false);

        content.append(&display_group);

        // === Features Group ===
        let features_group = adw::PreferencesGroup::builder().title("Features").build();

        // Audio redirect
        let audio_check = CheckButton::new();
        let audio_row = adw::ActionRow::builder()
            .title("Audio Redirection")
            .subtitle("Play remote audio locally")
            .activatable_widget(&audio_check)
            .build();
        audio_row.add_suffix(&audio_check);
        features_group.add(&audio_row);

        // Gateway
        let gateway_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("gateway.example.com")
            .valign(gtk4::Align::Center)
            .build();

        let gateway_row = adw::ActionRow::builder()
            .title("RDP Gateway")
            .subtitle("Remote Desktop Gateway server")
            .build();
        gateway_row.add_suffix(&gateway_entry);
        features_group.add(&gateway_row);

        content.append(&features_group);

        // === Shared Folders Group ===
        let folders_group = adw::PreferencesGroup::builder()
            .title("Shared Folders")
            .description("Local folders accessible from remote session")
            .build();

        let folders_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        folders_list.set_placeholder(Some(&Label::new(Some("No shared folders"))));

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
            .label("Add")
            .css_classes(["suggested-action"])
            .build();
        let remove_folder_btn = Button::builder().label("Remove").sensitive(false).build();
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
        let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

        let args_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Additional command-line arguments")
            .valign(gtk4::Align::Center)
            .build();

        let args_row = adw::ActionRow::builder()
            .title("Custom Arguments")
            .subtitle("Extra FreeRDP command-line options")
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
            audio_check,
            gateway_entry,
            shared_folders,
            folders_list,
            args_entry,
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
                .title("Select Folder to Share")
                .modal(true)
                .build();

            let folders_list = folders_list_clone.clone();
            let shared_folders = shared_folders_clone.clone();
            let parent = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

            file_dialog.select_folder(
                parent.as_ref(),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
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
            if let Some(selected_row) = folders_list_clone.selected_row() {
                if let Ok(index) = usize::try_from(selected_row.index()) {
                    if index < shared_folders_clone.borrow().len() {
                        shared_folders_clone.borrow_mut().remove(index);
                        folders_list_clone.remove(&selected_row);
                    }
                }
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
        let display_group = adw::PreferencesGroup::builder().title("Display").build();

        // Client mode dropdown
        let client_mode_list = StringList::new(&[
            VncClientMode::Embedded.display_name(),
            VncClientMode::External.display_name(),
        ]);
        let client_mode_dropdown = DropDown::builder()
            .model(&client_mode_list)
            .valign(gtk4::Align::Center)
            .build();

        let client_mode_row = adw::ActionRow::builder()
            .title("Client Mode")
            .subtitle("Embedded renders in tab, External opens separate window")
            .build();
        client_mode_row.add_suffix(&client_mode_dropdown);
        display_group.add(&client_mode_row);

        // Performance mode dropdown
        let performance_mode_list = StringList::new(&[
            VncPerformanceMode::Quality.display_name(),
            VncPerformanceMode::Balanced.display_name(),
            VncPerformanceMode::Speed.display_name(),
        ]);
        let performance_mode_dropdown = DropDown::builder()
            .model(&performance_mode_list)
            .valign(gtk4::Align::Center)
            .build();
        performance_mode_dropdown.set_selected(1); // Default to Balanced

        let performance_mode_row = adw::ActionRow::builder()
            .title("Performance Mode")
            .subtitle("Quality/speed tradeoff for image rendering")
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
            .title("Encoding")
            .subtitle("Preferred encoding methods (comma-separated)")
            .build();
        encoding_row.add_suffix(&encoding_entry);
        display_group.add(&encoding_row);

        content.append(&display_group);

        // === Quality Group ===
        let quality_group = adw::PreferencesGroup::builder().title("Quality").build();

        // Compression
        let compression_adj = gtk4::Adjustment::new(6.0, 0.0, 9.0, 1.0, 1.0, 0.0);
        let compression_spin = SpinButton::builder()
            .adjustment(&compression_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .build();

        let compression_row = adw::ActionRow::builder()
            .title("Compression")
            .subtitle("0 (none) to 9 (maximum)")
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
            .title("Quality")
            .subtitle("0 (lowest) to 9 (highest)")
            .build();
        quality_row.add_suffix(&quality_spin);
        quality_group.add(&quality_row);

        content.append(&quality_group);

        // === Features Group ===
        let features_group = adw::PreferencesGroup::builder().title("Features").build();

        // View-only mode
        let view_only_check = CheckButton::new();
        let view_only_row = adw::ActionRow::builder()
            .title("View-Only Mode")
            .subtitle("Disable keyboard and mouse input")
            .activatable_widget(&view_only_check)
            .build();
        view_only_row.add_suffix(&view_only_check);
        features_group.add(&view_only_row);

        // Scaling
        let scaling_check = CheckButton::new();
        scaling_check.set_active(true);
        let scaling_row = adw::ActionRow::builder()
            .title("Scale Display")
            .subtitle("Fit remote desktop to window size")
            .activatable_widget(&scaling_check)
            .build();
        scaling_row.add_suffix(&scaling_check);
        features_group.add(&scaling_row);

        // Clipboard sharing
        let clipboard_check = CheckButton::new();
        clipboard_check.set_active(true);
        let clipboard_row = adw::ActionRow::builder()
            .title("Clipboard Sharing")
            .subtitle("Synchronize clipboard with remote")
            .activatable_widget(&clipboard_check)
            .build();
        clipboard_row.add_suffix(&clipboard_check);
        features_group.add(&clipboard_row);

        content.append(&features_group);

        // === Advanced Group ===
        let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

        let custom_args_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Additional arguments for external client")
            .valign(gtk4::Align::Center)
            .build();

        let args_row = adw::ActionRow::builder()
            .title("Custom Arguments")
            .subtitle("Extra command-line options for vncviewer")
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
        let security_group = adw::PreferencesGroup::builder().title("Security").build();

        // TLS enabled
        let tls_check = CheckButton::new();
        let tls_row = adw::ActionRow::builder()
            .title("TLS Encryption")
            .subtitle("Encrypt connection with TLS")
            .activatable_widget(&tls_check)
            .build();
        tls_row.add_suffix(&tls_check);
        security_group.add(&tls_row);

        // CA certificate path
        let ca_cert_box = GtkBox::new(Orientation::Horizontal, 4);
        ca_cert_box.set_valign(gtk4::Align::Center);
        let ca_cert_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Path to CA certificate")
            .build();
        let ca_cert_button = Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Browse for certificate")
            .build();
        ca_cert_box.append(&ca_cert_entry);
        ca_cert_box.append(&ca_cert_button);

        let ca_cert_row = adw::ActionRow::builder()
            .title("CA Certificate")
            .subtitle("Certificate authority for TLS verification")
            .build();
        ca_cert_row.add_suffix(&ca_cert_box);
        security_group.add(&ca_cert_row);

        // Skip certificate verification
        let skip_verify_check = CheckButton::new();
        let skip_verify_row = adw::ActionRow::builder()
            .title("Skip Verification")
            .subtitle("Disable certificate verification (insecure)")
            .activatable_widget(&skip_verify_check)
            .build();
        skip_verify_row.add_suffix(&skip_verify_check);
        security_group.add(&skip_verify_row);

        content.append(&security_group);

        // === Features Group ===
        let features_group = adw::PreferencesGroup::builder().title("Features").build();

        // USB redirection
        let usb_check = CheckButton::new();
        let usb_row = adw::ActionRow::builder()
            .title("USB Redirection")
            .subtitle("Forward USB devices to remote")
            .activatable_widget(&usb_check)
            .build();
        usb_row.add_suffix(&usb_check);
        features_group.add(&usb_row);

        // Clipboard sharing
        let clipboard_check = CheckButton::new();
        clipboard_check.set_active(true);
        let clipboard_row = adw::ActionRow::builder()
            .title("Clipboard Sharing")
            .subtitle("Synchronize clipboard with remote")
            .activatable_widget(&clipboard_check)
            .build();
        clipboard_row.add_suffix(&clipboard_check);
        features_group.add(&clipboard_row);

        // Image compression
        let compression_list = StringList::new(&["Auto", "Off", "GLZ", "LZ", "QUIC"]);
        let compression_dropdown = DropDown::new(Some(compression_list), gtk4::Expression::NONE);
        compression_dropdown.set_selected(0);
        compression_dropdown.set_valign(gtk4::Align::Center);

        let compression_row = adw::ActionRow::builder()
            .title("Image Compression")
            .subtitle("Algorithm for image data")
            .build();
        compression_row.add_suffix(&compression_dropdown);
        features_group.add(&compression_row);

        content.append(&features_group);

        // === Shared Folders Group ===
        let folders_group = adw::PreferencesGroup::builder()
            .title("Shared Folders")
            .description("Local folders accessible from remote session")
            .build();

        let folders_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        folders_list.set_placeholder(Some(&Label::new(Some("No shared folders"))));

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
            .label("Add")
            .css_classes(["suggested-action"])
            .build();
        let remove_folder_btn = Button::builder().label("Remove").sensitive(false).build();
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
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
        Entry,
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
        let provider_group = adw::PreferencesGroup::builder().title("Provider").build();

        let provider_list = StringList::new(&[
            "AWS Session Manager",
            "GCP IAP Tunnel",
            "Azure Bastion",
            "Azure SSH (AAD)",
            "OCI Bastion",
            "Cloudflare Access",
            "Teleport",
            "Tailscale SSH",
            "HashiCorp Boundary",
            "Generic Command",
        ]);
        let provider_dropdown = DropDown::new(Some(provider_list), gtk4::Expression::NONE);
        provider_dropdown.set_selected(0);
        provider_dropdown.set_valign(gtk4::Align::Center);

        let provider_row = adw::ActionRow::builder()
            .title("Zero Trust Provider")
            .subtitle("Select your identity-aware proxy service")
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
        let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

        let custom_args_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Additional command-line arguments")
            .valign(gtk4::Align::Center)
            .build();

        let args_row = adw::ActionRow::builder()
            .title("Custom Arguments")
            .subtitle("Extra CLI options for the provider command")
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
    fn create_aws_ssm_fields_adw() -> (GtkBox, Entry, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("AWS Session Manager")
            .description("Connect via AWS Systems Manager")
            .build();

        let target_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("i-0123456789abcdef0")
            .valign(gtk4::Align::Center)
            .build();
        let target_row = adw::ActionRow::builder()
            .title("Instance ID")
            .subtitle("EC2 instance or managed instance ID")
            .build();
        target_row.add_suffix(&target_entry);
        group.add(&target_row);

        let profile_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("default")
            .text("default")
            .valign(gtk4::Align::Center)
            .build();
        let profile_row = adw::ActionRow::builder()
            .title("AWS Profile")
            .subtitle("Named profile from ~/.aws/credentials")
            .build();
        profile_row.add_suffix(&profile_entry);
        group.add(&profile_row);

        let region_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("us-east-1")
            .valign(gtk4::Align::Center)
            .build();
        let region_row = adw::ActionRow::builder()
            .title("Region")
            .subtitle("AWS region (optional, uses profile default)")
            .build();
        region_row.add_suffix(&region_entry);
        group.add(&region_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, target_entry, profile_entry, region_entry)
    }

    /// Creates GCP IAP provider fields using libadwaita
    fn create_gcp_iap_fields_adw() -> (GtkBox, Entry, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("GCP IAP Tunnel")
            .description("Connect via Identity-Aware Proxy")
            .build();

        let instance_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("my-instance")
            .valign(gtk4::Align::Center)
            .build();
        let instance_row = adw::ActionRow::builder()
            .title("Instance Name")
            .subtitle("Compute Engine instance name")
            .build();
        instance_row.add_suffix(&instance_entry);
        group.add(&instance_row);

        let zone_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("us-central1-a")
            .valign(gtk4::Align::Center)
            .build();
        let zone_row = adw::ActionRow::builder()
            .title("Zone")
            .subtitle("Compute Engine zone")
            .build();
        zone_row.add_suffix(&zone_entry);
        group.add(&zone_row);

        let project_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("my-project")
            .valign(gtk4::Align::Center)
            .build();
        let project_row = adw::ActionRow::builder()
            .title("Project")
            .subtitle("GCP project ID")
            .build();
        project_row.add_suffix(&project_entry);
        group.add(&project_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, instance_entry, zone_entry, project_entry)
    }

    /// Creates Azure Bastion provider fields using libadwaita
    fn create_azure_bastion_fields_adw() -> (GtkBox, Entry, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("Azure Bastion")
            .description("Connect via Azure Bastion service")
            .build();

        let resource_id_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("/subscriptions/.../resourceGroups/.../providers/...")
            .valign(gtk4::Align::Center)
            .build();
        let resource_id_row = adw::ActionRow::builder()
            .title("Target Resource ID")
            .subtitle("Full Azure resource ID of the target VM")
            .build();
        resource_id_row.add_suffix(&resource_id_entry);
        group.add(&resource_id_row);

        let rg_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("my-resource-group")
            .valign(gtk4::Align::Center)
            .build();
        let rg_row = adw::ActionRow::builder()
            .title("Resource Group")
            .subtitle("Bastion host resource group")
            .build();
        rg_row.add_suffix(&rg_entry);
        group.add(&rg_row);

        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("my-bastion")
            .valign(gtk4::Align::Center)
            .build();
        let name_row = adw::ActionRow::builder()
            .title("Bastion Name")
            .subtitle("Azure Bastion host name")
            .build();
        name_row.add_suffix(&name_entry);
        group.add(&name_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, resource_id_entry, rg_entry, name_entry)
    }

    /// Creates Azure SSH (AAD) provider fields using libadwaita
    fn create_azure_ssh_fields_adw() -> (GtkBox, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("Azure SSH (AAD)")
            .description("Connect via Azure AD authentication")
            .build();

        let vm_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("my-vm")
            .valign(gtk4::Align::Center)
            .build();
        let vm_row = adw::ActionRow::builder()
            .title("VM Name")
            .subtitle("Azure virtual machine name")
            .build();
        vm_row.add_suffix(&vm_entry);
        group.add(&vm_row);

        let rg_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("my-resource-group")
            .valign(gtk4::Align::Center)
            .build();
        let rg_row = adw::ActionRow::builder()
            .title("Resource Group")
            .subtitle("VM resource group")
            .build();
        rg_row.add_suffix(&rg_entry);
        group.add(&rg_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, vm_entry, rg_entry)
    }

    /// Creates OCI Bastion provider fields using libadwaita
    fn create_oci_bastion_fields_adw() -> (GtkBox, Entry, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("OCI Bastion")
            .description("Connect via Oracle Cloud Bastion")
            .build();

        let bastion_id_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("ocid1.bastion.oc1...")
            .valign(gtk4::Align::Center)
            .build();
        let bastion_id_row = adw::ActionRow::builder()
            .title("Bastion OCID")
            .subtitle("Oracle Cloud bastion identifier")
            .build();
        bastion_id_row.add_suffix(&bastion_id_entry);
        group.add(&bastion_id_row);

        let target_id_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("ocid1.instance.oc1...")
            .valign(gtk4::Align::Center)
            .build();
        let target_id_row = adw::ActionRow::builder()
            .title("Target OCID")
            .subtitle("Target instance identifier")
            .build();
        target_id_row.add_suffix(&target_id_entry);
        group.add(&target_id_row);

        let target_ip_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("10.0.0.1")
            .valign(gtk4::Align::Center)
            .build();
        let target_ip_row = adw::ActionRow::builder()
            .title("Target IP")
            .subtitle("Private IP address of target")
            .build();
        target_ip_row.add_suffix(&target_ip_entry);
        group.add(&target_ip_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, bastion_id_entry, target_id_entry, target_ip_entry)
    }

    /// Creates Cloudflare Access provider fields using libadwaita
    fn create_cloudflare_fields_adw() -> (GtkBox, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("Cloudflare Access")
            .description("Connect via Cloudflare Zero Trust")
            .build();

        let hostname_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("ssh.example.com")
            .valign(gtk4::Align::Center)
            .build();
        let hostname_row = adw::ActionRow::builder()
            .title("Hostname")
            .subtitle("Cloudflare Access protected hostname")
            .build();
        hostname_row.add_suffix(&hostname_entry);
        group.add(&hostname_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, hostname_entry)
    }

    /// Creates Teleport provider fields using libadwaita
    fn create_teleport_fields_adw() -> (GtkBox, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("Teleport")
            .description("Connect via Gravitational Teleport")
            .build();

        let host_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("node-name")
            .valign(gtk4::Align::Center)
            .build();
        let host_row = adw::ActionRow::builder()
            .title("Node Name")
            .subtitle("Teleport node hostname")
            .build();
        host_row.add_suffix(&host_entry);
        group.add(&host_row);

        let cluster_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("teleport.example.com")
            .valign(gtk4::Align::Center)
            .build();
        let cluster_row = adw::ActionRow::builder()
            .title("Cluster")
            .subtitle("Teleport cluster address (optional)")
            .build();
        cluster_row.add_suffix(&cluster_entry);
        group.add(&cluster_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, host_entry, cluster_entry)
    }

    /// Creates Tailscale SSH provider fields using libadwaita
    fn create_tailscale_fields_adw() -> (GtkBox, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("Tailscale SSH")
            .description("Connect via Tailscale network")
            .build();

        let host_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("hostname or 100.x.x.x")
            .valign(gtk4::Align::Center)
            .build();
        let host_row = adw::ActionRow::builder()
            .title("Tailscale Host")
            .subtitle("Machine name or Tailscale IP")
            .build();
        host_row.add_suffix(&host_entry);
        group.add(&host_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, host_entry)
    }

    /// Creates HashiCorp Boundary provider fields using libadwaita
    fn create_boundary_fields_adw() -> (GtkBox, Entry, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("HashiCorp Boundary")
            .description("Connect via Boundary proxy")
            .build();

        let target_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("ttcp_1234567890")
            .valign(gtk4::Align::Center)
            .build();
        let target_row = adw::ActionRow::builder()
            .title("Target ID")
            .subtitle("Boundary target identifier")
            .build();
        target_row.add_suffix(&target_entry);
        group.add(&target_row);

        let addr_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("https://boundary.example.com")
            .valign(gtk4::Align::Center)
            .build();
        let addr_row = adw::ActionRow::builder()
            .title("Controller Address")
            .subtitle("Boundary controller URL (optional)")
            .build();
        addr_row.add_suffix(&addr_entry);
        group.add(&addr_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, target_entry, addr_entry)
    }

    /// Creates Generic Zero Trust provider fields using libadwaita
    fn create_generic_zt_fields_adw() -> (GtkBox, Entry) {
        let group = adw::PreferencesGroup::builder()
            .title("Generic Command")
            .description("Custom command for unsupported providers")
            .build();

        let command_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("ssh -o ProxyCommand='...' %h")
            .valign(gtk4::Align::Center)
            .build();
        let command_row = adw::ActionRow::builder()
            .title("Command Template")
            .subtitle("Use %h for host, %p for port, %u for user")
            .build();
        command_row.add_suffix(&command_entry);
        group.add(&command_row);

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&group);

        (vbox, command_entry)
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
            .title("Local Variables")
            .description("Use ${variable_name} syntax in connection fields")
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
        variables_list.set_placeholder(Some(&Label::new(Some("No variables defined"))));
        variables_scrolled.set_child(Some(&variables_list));

        variables_group.add(&variables_scrolled);

        let var_button_box = GtkBox::new(Orientation::Horizontal, 8);
        var_button_box.set_halign(gtk4::Align::End);
        var_button_box.set_margin_top(8);

        let add_variable_button = Button::builder()
            .label("Add Variable")
            .css_classes(["suggested-action"])
            .build();
        var_button_box.append(&add_variable_button);

        variables_group.add(&var_button_box);
        content.append(&variables_group);

        // === Custom Properties Section ===
        let properties_group = adw::PreferencesGroup::builder()
            .title("Custom Properties")
            .description("Text, URL (clickable), or Protected (masked) metadata")
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
        properties_list.set_placeholder(Some(&Label::new(Some("No custom properties"))));
        properties_scrolled.set_child(Some(&properties_list));

        properties_group.add(&properties_scrolled);

        let prop_button_box = GtkBox::new(Orientation::Horizontal, 8);
        prop_button_box.set_halign(gtk4::Align::End);
        prop_button_box.set_margin_top(8);

        let add_property_button = Button::builder()
            .label("Add Property")
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

    /// Creates the Logging tab for session logging configuration
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn create_logging_tab() -> (GtkBox, CheckButton, Entry, DropDown, SpinButton, SpinButton) {
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

        // Enable logging group
        let enable_group = adw::PreferencesGroup::builder()
            .title("Session Logging")
            .description("Record terminal output to files")
            .build();

        let enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let enable_row = adw::ActionRow::builder()
            .title("Enable Logging")
            .subtitle("Record session output to log files")
            .activatable_widget(&enabled_check)
            .build();
        enable_row.add_suffix(&enabled_check);
        enable_group.add(&enable_row);

        content.append(&enable_group);

        // Log settings group
        let settings_group = adw::PreferencesGroup::builder()
            .title("Log Settings")
            .build();

        // Path template
        let path_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text("${HOME}/.local/share/rustconn/logs/${connection_name}_${date}.log")
            .sensitive(false)
            .build();

        let path_row = adw::ActionRow::builder()
            .title("Log Path")
            .subtitle("Variables: ${connection_name}, ${protocol}, ${date}, ${time}, ${datetime}, ${HOME}")
            .build();
        path_row.add_suffix(&path_entry);
        settings_group.add(&path_row);

        // Timestamp format
        let timestamp_list = StringList::new(&[
            "%Y-%m-%d %H:%M:%S",     // 2024-01-15 14:30:45
            "%H:%M:%S",              // 14:30:45
            "%Y-%m-%d %H:%M:%S%.3f", // 2024-01-15 14:30:45.123
            "[%Y-%m-%d %H:%M:%S]",   // [2024-01-15 14:30:45]
            "%d/%m/%Y %H:%M:%S",     // 15/01/2024 14:30:45
        ]);
        let timestamp_dropdown = DropDown::new(Some(timestamp_list), gtk4::Expression::NONE);
        timestamp_dropdown.set_selected(0);
        timestamp_dropdown.set_valign(gtk4::Align::Center);
        timestamp_dropdown.set_sensitive(false);

        let timestamp_row = adw::ActionRow::builder()
            .title("Timestamp Format")
            .subtitle("Format for timestamps in log entries")
            .build();
        timestamp_row.add_suffix(&timestamp_dropdown);
        settings_group.add(&timestamp_row);

        // Max size
        let size_adj = gtk4::Adjustment::new(10.0, 0.0, 1000.0, 1.0, 10.0, 0.0);
        let size_spin = SpinButton::builder()
            .adjustment(&size_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let size_row = adw::ActionRow::builder()
            .title("Max Size (MB)")
            .subtitle("Maximum log file size (0 = no limit)")
            .build();
        size_row.add_suffix(&size_spin);
        settings_group.add(&size_row);

        // Retention days
        let retention_adj = gtk4::Adjustment::new(30.0, 0.0, 365.0, 1.0, 7.0, 0.0);
        let retention_spin = SpinButton::builder()
            .adjustment(&retention_adj)
            .climb_rate(1.0)
            .digits(0)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let retention_row = adw::ActionRow::builder()
            .title("Retention (days)")
            .subtitle("Days to keep old log files (0 = keep forever)")
            .build();
        retention_row.add_suffix(&retention_spin);
        settings_group.add(&retention_row);

        content.append(&settings_group);

        // Connect enabled checkbox to enable/disable other fields
        let path_entry_clone = path_entry.clone();
        let timestamp_dropdown_clone = timestamp_dropdown.clone();
        let size_spin_clone = size_spin.clone();
        let retention_spin_clone = retention_spin.clone();
        let settings_group_clone = settings_group.clone();
        enabled_check.connect_toggled(move |check| {
            let enabled = check.is_active();
            path_entry_clone.set_sensitive(enabled);
            timestamp_dropdown_clone.set_sensitive(enabled);
            size_spin_clone.set_sensitive(enabled);
            retention_spin_clone.set_sensitive(enabled);
            settings_group_clone.set_sensitive(enabled);
        });

        // Initially disable settings group since logging is off by default
        settings_group.set_sensitive(false);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            enabled_check,
            path_entry,
            timestamp_dropdown,
            size_spin,
            retention_spin,
        )
    }

    /// Creates the combined Automation tab (Expect Rules + Tasks)
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn create_automation_combined_tab() -> (
        GtkBox,
        ListBox,
        Button,
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
            .title("Expect Rules")
            .description("Auto-respond to terminal patterns (priority order)")
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
        expect_rules_list.set_placeholder(Some(&Label::new(Some("No expect rules"))));
        rules_scrolled.set_child(Some(&expect_rules_list));

        rules_group.add(&rules_scrolled);

        let rules_button_box = GtkBox::new(Orientation::Horizontal, 8);
        rules_button_box.set_halign(gtk4::Align::End);
        rules_button_box.set_margin_top(8);

        let add_rule_button = Button::builder()
            .label("Add Rule")
            .css_classes(["suggested-action"])
            .build();
        rules_button_box.append(&add_rule_button);

        rules_group.add(&rules_button_box);
        content.append(&rules_group);

        // Pattern tester
        let tester_group = adw::PreferencesGroup::builder()
            .title("Pattern Tester")
            .build();

        let test_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text("Test text against patterns")
            .build();

        let test_row = adw::ActionRow::builder().title("Test Input").build();
        test_row.add_suffix(&test_entry);
        tester_group.add(&test_row);

        let result_label = Label::builder()
            .label("Enter text to test")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();

        let result_row = adw::ActionRow::builder().title("Result").build();
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
        ) = Self::create_task_section("Pre-Connect Task", true);
        content.append(&pre_connect_group);

        // === Post-Disconnect Task Section ===
        let (
            post_disconnect_group,
            post_disconnect_enabled_check,
            post_disconnect_command_entry,
            post_disconnect_timeout_spin,
            _post_disconnect_abort_check,
            post_disconnect_last_only_check,
        ) = Self::create_task_section("Post-Disconnect Task", false);
        content.append(&post_disconnect_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            expect_rules_list,
            add_rule_button,
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

    /// Creates the Automation tab for expect rules configuration (legacy, kept for reference)
    #[allow(dead_code, clippy::type_complexity)]
    fn create_automation_tab() -> (GtkBox, ListBox, Button, Entry, Label) {
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

        // Expect rules group
        let rules_group = adw::PreferencesGroup::builder()
            .title("Expect Rules")
            .description(
                "Automatically respond to terminal patterns.\n\
                 Rules are matched in priority order (highest first).",
            )
            .build();

        let inner_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(200)
            .vexpand(true)
            .build();

        let expect_rules_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        expect_rules_list.set_placeholder(Some(&Label::new(Some("No expect rules configured"))));
        inner_scrolled.set_child(Some(&expect_rules_list));

        rules_group.add(&inner_scrolled);

        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);
        button_box.set_margin_top(12);

        let add_button = Button::builder()
            .label("Add Rule")
            .css_classes(["suggested-action"])
            .build();
        button_box.append(&add_button);

        rules_group.add(&button_box);

        content.append(&rules_group);

        // Pattern tester group
        let tester_group = adw::PreferencesGroup::builder()
            .title("Pattern Tester")
            .description("Test your patterns against sample input")
            .build();

        let test_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text("Enter text to test against patterns")
            .build();

        let test_row = adw::ActionRow::builder().title("Test Input").build();
        test_row.add_suffix(&test_entry);
        tester_group.add(&test_row);

        let result_label = Label::builder()
            .label("Enter text above to test patterns")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();

        let result_row = adw::ActionRow::builder().title("Result").build();
        result_row.add_suffix(&result_label);
        tester_group.add(&result_row);

        content.append(&tester_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            expect_rules_list,
            add_button,
            test_entry,
            result_label,
        )
    }

    /// Creates the Tasks tab for pre/post connection task configuration (legacy)
    #[allow(dead_code, clippy::type_complexity)]
    fn create_tasks_tab() -> (
        GtkBox,
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

        // Pre-connect task section
        let (
            pre_connect_group,
            pre_connect_enabled_check,
            pre_connect_command_entry,
            pre_connect_timeout_spin,
            pre_connect_abort_check,
            pre_connect_first_only_check,
        ) = Self::create_task_section("Pre-Connect Task", true);
        content.append(&pre_connect_group);

        // Post-disconnect task section
        let (
            post_disconnect_group,
            post_disconnect_enabled_check,
            post_disconnect_command_entry,
            post_disconnect_timeout_spin,
            _post_disconnect_abort_check, // Not used for post-disconnect
            post_disconnect_last_only_check,
        ) = Self::create_task_section("Post-Disconnect Task", false);
        content.append(&post_disconnect_group);

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
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
            "Run command before connecting. Supports ${variable} substitution."
        } else {
            "Run command after disconnecting. Supports ${variable} substitution."
        };

        let group = adw::PreferencesGroup::builder()
            .title(title)
            .description(description)
            .build();

        // Enable checkbox
        let enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let enable_row = adw::ActionRow::builder()
            .title("Enable Task")
            .activatable_widget(&enabled_check)
            .build();
        enable_row.add_suffix(&enabled_check);
        group.add(&enable_row);

        // Command entry
        let command_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text("e.g., /path/to/script.sh or vpn-connect ${host}")
            .sensitive(false)
            .build();

        let command_row = adw::ActionRow::builder()
            .title("Command")
            .subtitle("Shell command to execute (supports ${variable} syntax)")
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
            .title("Timeout (ms)")
            .subtitle("0 = no timeout")
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
                .title("Abort on Failure")
                .subtitle("Cancel connection if this task fails")
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
                "First Connection Only",
                "Only run when opening the first connection in a folder (useful for VPN setup)",
            )
        } else {
            (
                "Last Connection Only",
                "Only run when closing the last connection in a folder (useful for cleanup)",
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
            .title("Window Mode")
            .build();

        let mode_list = StringList::new(&["Embedded", "External Window", "Fullscreen"]);
        let mode_dropdown = DropDown::new(Some(mode_list), gtk4::Expression::NONE);
        mode_dropdown.set_selected(0);
        mode_dropdown.set_valign(gtk4::Align::Center);

        let mode_row = adw::ActionRow::builder()
            .title("Display Mode")
            .subtitle("Embedded • External • Fullscreen")
            .build();
        mode_row.add_suffix(&mode_dropdown);
        mode_group.add(&mode_row);

        let remember_check = CheckButton::builder()
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let remember_row = adw::ActionRow::builder()
            .title("Remember Position")
            .subtitle("Save window geometry (External mode only)")
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
            .title("Wake On LAN")
            .build();

        let wol_enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let wol_enable_row = adw::ActionRow::builder()
            .title("Enable WOL")
            .subtitle("Send magic packet before connecting")
            .activatable_widget(&wol_enabled_check)
            .build();
        wol_enable_row.add_suffix(&wol_enabled_check);
        wol_group.add(&wol_enable_row);

        content.append(&wol_group);

        // WOL Settings group
        let wol_settings_group = adw::PreferencesGroup::builder()
            .title("WOL Settings")
            .sensitive(false)
            .build();

        let mac_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text("AA:BB:CC:DD:EE:FF")
            .build();

        let mac_row = adw::ActionRow::builder().title("MAC Address").build();
        mac_row.add_suffix(&mac_entry);
        wol_settings_group.add(&mac_row);

        let broadcast_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .text(DEFAULT_BROADCAST_ADDRESS)
            .build();

        let broadcast_row = adw::ActionRow::builder().title("Broadcast Address").build();
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
            .title("UDP Port")
            .subtitle("Default: 9")
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
            .title("Wait Time (sec)")
            .subtitle("Time to wait for boot")
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
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Property name (e.g., asset_tag, docs_url)")
            .build();

        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text("Delete property")
            .build();

        grid.attach(&name_label, 0, 0, 1, 1);
        grid.attach(&name_entry, 1, 0, 1, 1);
        grid.attach(&delete_button, 2, 0, 1, 1);

        // Row 1: Type dropdown
        let type_label = Label::builder()
            .label("Type:")
            .halign(gtk4::Align::End)
            .build();
        let type_list = StringList::new(&["Text", "URL", "Protected"]);
        let type_dropdown = DropDown::new(Some(type_list), gtk4::Expression::NONE);
        type_dropdown.set_selected(0);
        type_dropdown.set_tooltip_text(Some(
            "Text: Plain text\nURL: Clickable link\nProtected: Masked/encrypted value",
        ));

        grid.attach(&type_label, 0, 1, 1, 1);
        grid.attach(&type_dropdown, 1, 1, 2, 1);

        // Row 2: Value (regular entry)
        let value_label = Label::builder()
            .label("Value:")
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Property value")
            .build();

        // Row 2: Value (password entry for protected type)
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text("Protected value (masked)")
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
                if let Ok(idx) = usize::try_from(row_widget.index()) {
                    if idx < props_for_delete.borrow().len() {
                        props_for_delete.borrow_mut().remove(idx);
                    }
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
            if let Ok(idx) = usize::try_from(row_for_name.index()) {
                if let Some(prop) = props_for_name.borrow_mut().get_mut(idx) {
                    prop.name = text;
                }
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
                if let Ok(idx) = usize::try_from(row_for_type.index()) {
                    if let Some(prop) = props_for_type.borrow_mut().get_mut(idx) {
                        prop.property_type = prop_type;
                    }
                }
            });

        // Value entry (for Text and URL types)
        let props_for_value = custom_properties.clone();
        let row_for_value = row_widget.clone();
        prop_row.value_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_value.index()) {
                if let Some(prop) = props_for_value.borrow_mut().get_mut(idx) {
                    if !prop.is_protected() {
                        prop.value = text;
                    }
                }
            }
        });

        // Secret entry (for Protected type)
        let props_for_secret = custom_properties.clone();
        let row_for_secret = row_widget;
        prop_row.secret_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if let Ok(idx) = usize::try_from(row_for_secret.index()) {
                if let Some(prop) = props_for_secret.borrow_mut().get_mut(idx) {
                    if prop.is_protected() {
                        prop.value = text;
                    }
                }
            }
        });

        // Suppress unused variable warning
        let _ = initial_index;
    }

    /// Creates the WOL (Wake On LAN) tab for configuring wake-on-lan settings (legacy)
    ///
    /// Uses libadwaita components following GNOME HIG.
    #[allow(dead_code, clippy::type_complexity)]
    fn create_wol_tab() -> (GtkBox, CheckButton, Entry, Entry, SpinButton, SpinButton) {
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

        // Enable WOL group
        let enable_group = adw::PreferencesGroup::builder()
            .title("Wake On LAN")
            .description("Send magic packet to wake sleeping machines before connecting")
            .build();

        let enabled_check = CheckButton::builder().valign(gtk4::Align::Center).build();

        let enable_row = adw::ActionRow::builder()
            .title("Enable Wake On LAN")
            .subtitle("Send WOL packet before connecting")
            .activatable_widget(&enabled_check)
            .build();
        enable_row.add_suffix(&enabled_check);
        enable_group.add(&enable_row);

        content.append(&enable_group);

        // WOL Settings group
        let settings_group = adw::PreferencesGroup::builder()
            .title("WOL Settings")
            .sensitive(false)
            .build();

        // MAC Address
        let mac_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .placeholder_text("AA:BB:CC:DD:EE:FF")
            .build();

        let mac_row = adw::ActionRow::builder()
            .title("MAC Address")
            .subtitle("Hardware address (format: AA:BB:CC:DD:EE:FF or AA-BB-CC-DD-EE-FF)")
            .build();
        mac_row.add_suffix(&mac_entry);
        settings_group.add(&mac_row);

        // Broadcast Address
        let broadcast_entry = Entry::builder()
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .text(DEFAULT_BROADCAST_ADDRESS)
            .placeholder_text("255.255.255.255")
            .build();

        let broadcast_row = adw::ActionRow::builder()
            .title("Broadcast Address")
            .subtitle("Network broadcast address for the magic packet")
            .build();
        broadcast_row.add_suffix(&broadcast_entry);
        settings_group.add(&broadcast_row);

        // UDP Port
        let port_adjustment =
            gtk4::Adjustment::new(f64::from(DEFAULT_WOL_PORT), 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_spin = SpinButton::builder()
            .adjustment(&port_adjustment)
            .digits(0)
            .valign(gtk4::Align::Center)
            .build();

        let port_row = adw::ActionRow::builder()
            .title("UDP Port")
            .subtitle("Default: 9 (discard protocol)")
            .build();
        port_row.add_suffix(&port_spin);
        settings_group.add(&port_row);

        // Wait Time
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
            .title("Wait Time (seconds)")
            .subtitle("Time to wait for machine to boot before connecting")
            .build();
        wait_row.add_suffix(&wait_spin);
        settings_group.add(&wait_row);

        content.append(&settings_group);

        // Status info group
        let status_group = adw::PreferencesGroup::builder()
            .title("Status")
            .description(
                "WOL packets will be sent automatically when connecting.\n\
                 Status feedback will be shown in the connection progress dialog.",
            )
            .build();

        content.append(&status_group);

        // Connect enabled checkbox to enable/disable settings group
        let settings_group_clone = settings_group.clone();
        enabled_check.connect_toggled(move |check| {
            settings_group_clone.set_sensitive(check.is_active());
        });

        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));

        let vbox = GtkBox::new(Orientation::Vertical, 0);
        vbox.append(&scrolled);

        (
            vbox,
            enabled_check,
            mac_entry,
            broadcast_entry,
            port_spin,
            wait_spin,
        )
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
            .label("Pattern:")
            .halign(gtk4::Align::End)
            .build();
        let pattern_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Regex pattern (e.g., password:\\s*$)")
            .tooltip_text("Regular expression to match against terminal output")
            .build();

        let button_box = GtkBox::new(Orientation::Horizontal, 4);
        let move_up_button = Button::builder()
            .icon_name("go-up-symbolic")
            .css_classes(["flat"])
            .tooltip_text("Move up (higher priority)")
            .build();
        let move_down_button = Button::builder()
            .icon_name("go-down-symbolic")
            .css_classes(["flat"])
            .tooltip_text("Move down (lower priority)")
            .build();
        let delete_button = Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["destructive-action", "flat"])
            .tooltip_text("Delete rule")
            .build();
        button_box.append(&move_up_button);
        button_box.append(&move_down_button);
        button_box.append(&delete_button);

        grid.attach(&pattern_label, 0, 0, 1, 1);
        grid.attach(&pattern_entry, 1, 0, 1, 1);
        grid.attach(&button_box, 2, 0, 1, 1);

        // Row 1: Response
        let response_label = Label::builder()
            .label("Response:")
            .halign(gtk4::Align::End)
            .build();
        let response_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Text to send when pattern matches")
            .tooltip_text("Response to send (supports ${variable} syntax)")
            .build();

        grid.attach(&response_label, 0, 1, 1, 1);
        grid.attach(&response_entry, 1, 1, 2, 1);

        // Row 2: Priority and Timeout
        let priority_label = Label::builder()
            .label("Priority:")
            .halign(gtk4::Align::End)
            .build();
        let priority_adj = gtk4::Adjustment::new(0.0, -1000.0, 1000.0, 1.0, 10.0, 0.0);
        let priority_spin = SpinButton::builder()
            .adjustment(&priority_adj)
            .climb_rate(1.0)
            .digits(0)
            .tooltip_text("Higher priority rules are checked first")
            .build();

        let timeout_label = Label::builder()
            .label("Timeout (ms):")
            .halign(gtk4::Align::End)
            .build();
        let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 60000.0, 100.0, 1000.0, 0.0);
        let timeout_spin = SpinButton::builder()
            .adjustment(&timeout_adj)
            .climb_rate(1.0)
            .digits(0)
            .tooltip_text("Timeout in milliseconds (0 = no timeout)")
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

        // Row 3: Enabled checkbox
        let enabled_check = CheckButton::builder().label("Enabled").active(true).build();

        grid.attach(&enabled_check, 1, 3, 2, 1);

        main_box.append(&grid);

        // Populate from existing rule if provided
        let id = rule.map_or_else(Uuid::new_v4, |r| {
            pattern_entry.set_text(&r.pattern);
            response_entry.set_text(&r.response);
            priority_spin.set_value(f64::from(r.priority));
            timeout_spin.set_value(f64::from(r.timeout_ms.unwrap_or(0)));
            enabled_check.set_active(r.enabled);
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
            .label("Name:")
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
            .label("Value:")
            .halign(gtk4::Align::End)
            .build();
        let value_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Variable value")
            .build();

        grid.attach(&value_label, 0, 1, 1, 1);
        grid.attach(&value_entry, 1, 1, 2, 1);

        // Row 2: Secret value (password entry, initially hidden)
        let secret_label = Label::builder()
            .label("Secret Value:")
            .halign(gtk4::Align::End)
            .visible(false)
            .build();
        let secret_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text("Secret value (masked)")
            .show_peek_icon(true)
            .visible(false)
            .build();

        grid.attach(&secret_label, 0, 2, 1, 1);
        grid.attach(&secret_entry, 1, 2, 2, 1);

        // Row 3: Is Secret checkbox
        let is_secret_check = CheckButton::builder().label("Secret (mask value)").build();

        grid.attach(&is_secret_check, 1, 3, 2, 1);

        // Row 4: Description
        let desc_label = Label::builder()
            .label("Description:")
            .halign(gtk4::Align::End)
            .build();
        let description_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Optional description")
            .build();

        grid.attach(&desc_label, 0, 4, 1, 1);
        grid.attach(&description_entry, 1, 4, 2, 1);

        // Add inherited indicator if applicable
        if is_inherited {
            let inherited_label = Label::builder()
                .label("(Inherited from global - override value below)")
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
                .title("Select SSH Key")
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
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.to_string_lossy());
                        }
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
                .title("Select CA Certificate")
                .modal(true)
                .build();

            let entry = ca_cert_entry.clone();
            file_dialog.open(
                parent.as_ref(),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.to_string_lossy());
                        }
                    }
                },
            );
        });
    }

    /// Populates the dialog with an existing connection for editing
    pub fn set_connection(&self, conn: &Connection) {
        self.window.set_title(Some("Edit Connection"));
        self.save_button.set_label("Save");
        *self.editing_id.borrow_mut() = Some(conn.id);

        // Basic fields
        self.name_entry.set_text(&conn.name);
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
        // Filter out desc: tags for backward compatibility with old imports
        let display_tags: Vec<&str> = conn
            .tags
            .iter()
            .filter(|t| !t.starts_with("desc:"))
            .map(String::as_str)
            .collect();
        self.tags_entry.set_text(&display_tags.join(", "));

        // If connection has desc: tag but no description field, extract it
        if conn.description.is_none() {
            if let Some(desc_tag) = conn.tags.iter().find(|t| t.starts_with("desc:")) {
                self.description_view
                    .buffer()
                    .set_text(desc_tag.strip_prefix("desc:").unwrap_or(""));
            }
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
        // Dropdown order: Prompt(0), Stored(1), KeePass(2), Keyring(3), Inherit(4), None(5)
        let password_source_idx = match conn.password_source {
            PasswordSource::Prompt => 0,
            PasswordSource::Stored => 1,
            PasswordSource::KeePass => 2,
            PasswordSource::Keyring => 3,
            PasswordSource::Inherit => 4,
            PasswordSource::None => 5,
        };
        self.password_source_dropdown
            .set_selected(password_source_idx);

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

    /// Sets the WOL configuration fields
    fn set_wol_config(&self, config: Option<&WolConfig>) {
        if let Some(wol) = config {
            self.wol_enabled_check.set_active(true);
            self.wol_mac_entry.set_text(&wol.mac_address.to_string());
            self.wol_mac_entry.set_sensitive(true);
            self.wol_broadcast_entry.set_text(&wol.broadcast_address);
            self.wol_broadcast_entry.set_sensitive(true);
            self.wol_port_spin.set_value(f64::from(wol.port));
            self.wol_port_spin.set_sensitive(true);
            self.wol_wait_spin.set_value(f64::from(wol.wait_seconds));
            self.wol_wait_spin.set_sensitive(true);
        } else {
            self.wol_enabled_check.set_active(false);
            self.wol_mac_entry.set_text("");
            self.wol_mac_entry.set_sensitive(false);
            self.wol_broadcast_entry.set_text(DEFAULT_BROADCAST_ADDRESS);
            self.wol_broadcast_entry.set_sensitive(false);
            self.wol_port_spin.set_value(f64::from(DEFAULT_WOL_PORT));
            self.wol_port_spin.set_sensitive(false);
            self.wol_wait_spin
                .set_value(f64::from(DEFAULT_WOL_WAIT_SECONDS));
            self.wol_wait_spin.set_sensitive(false);
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
            if let Ok(idx) = usize::try_from(row_widget.index()) {
                if idx < props_for_delete.borrow().len() {
                    props_for_delete.borrow_mut().remove(idx);
                }
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
        if let Some(config) = log_config {
            self.logging_enabled_check.set_active(config.enabled);
            self.logging_path_entry.set_text(&config.path_template);

            // Map timestamp format to dropdown index
            let timestamp_formats = [
                "%Y-%m-%d %H:%M:%S",
                "%H:%M:%S",
                "%Y-%m-%d %H:%M:%S%.3f",
                "[%Y-%m-%d %H:%M:%S]",
                "%d/%m/%Y %H:%M:%S",
            ];
            let idx = timestamp_formats
                .iter()
                .position(|&f| f == config.timestamp_format)
                .unwrap_or(0);
            self.logging_timestamp_dropdown.set_selected(idx as u32);

            self.logging_max_size_spin
                .set_value(f64::from(config.max_size_mb));
            self.logging_retention_spin
                .set_value(f64::from(config.retention_days));

            // Enable fields if logging is enabled
            let enabled = config.enabled;
            self.logging_path_entry.set_sensitive(enabled);
            self.logging_timestamp_dropdown.set_sensitive(enabled);
            self.logging_max_size_spin.set_sensitive(enabled);
            self.logging_retention_spin.set_sensitive(enabled);
        } else {
            // Reset to defaults
            self.logging_enabled_check.set_active(false);
            self.logging_path_entry.set_text("");
            self.logging_timestamp_dropdown.set_selected(0);
            self.logging_max_size_spin.set_value(10.0);
            self.logging_retention_spin.set_value(30.0);

            // Disable fields
            self.logging_path_entry.set_sensitive(false);
            self.logging_timestamp_dropdown.set_sensitive(false);
            self.logging_max_size_spin.set_sensitive(false);
            self.logging_retention_spin.set_sensitive(false);
        }
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
        if let Some(ref key_path) = ssh.key_path {
            if matches!(ssh.key_source, SshKeySource::Default) {
                // If key_source is Default but key_path is set, use File source
                self.ssh_key_source_dropdown.set_selected(1);
                self.ssh_key_entry.set_text(&key_path.to_string_lossy());
                self.ssh_key_entry.set_sensitive(true);
                self.ssh_key_button.set_sensitive(true);
            }
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

    /// Runs the dialog and calls the callback with the result
    pub fn run<F: Fn(Option<Connection>) + 'static>(&self, cb: F) {
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

    /// Sets whether `KeePass` integration is enabled
    ///
    /// This controls the sensitivity of the `KeePass` buttons.
    /// When `KeePass` is not enabled, the buttons are disabled.
    pub fn set_keepass_enabled(&self, enabled: bool) {
        self.save_to_keepass_button.set_sensitive(enabled);
        self.load_from_keepass_button.set_sensitive(enabled);
    }

    /// Sets up the callback for the "Save to `KeePass`" button
    ///
    /// The callback receives the connection name, host, username, password, protocol, and dialog window.
    pub fn connect_save_to_keepass<F: Fn(&str, &str, &str, &str, &str, adw::Window) + 'static>(
        &self,
        callback: F,
    ) {
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let username_entry = self.username_entry.clone();
        let password_entry = self.password_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let window = self.window.clone();

        self.save_to_keepass_button.connect_clicked(move |_| {
            let name = name_entry.text();
            let host = host_entry.text();
            let username = username_entry.text();
            let password = password_entry.text();

            // Get selected protocol
            let protocol = match protocol_dropdown.selected() {
                0 => "ssh",
                1 => "rdp",
                2 => "vnc",
                3 => "spice",
                _ => "ssh",
            };

            if password.is_empty() {
                crate::toast::show_toast_on_window(
                    &window,
                    "Please enter a password to save",
                    crate::toast::ToastType::Warning,
                );
                return;
            }

            if name.trim().is_empty() && host.trim().is_empty() {
                crate::toast::show_toast_on_window(
                    &window,
                    "Please enter a connection name or host first",
                    crate::toast::ToastType::Warning,
                );
                return;
            }

            callback(&name, &host, &username, &password, protocol, window.clone());
        });
    }

    /// Sets up the callback for the "Load from `KeePass`" button
    ///
    /// The callback receives the connection name, host, protocol, password entry, and window.
    /// The callback should handle the async loading and update the password entry when done.
    pub fn connect_load_from_keepass<F: Fn(&str, &str, &str, Entry, gtk4::Window) + 'static>(
        &self,
        callback: F,
    ) {
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let password_entry = self.password_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let window = self.window.clone();

        self.load_from_keepass_button.connect_clicked(move |_| {
            let name = name_entry.text();
            let host = host_entry.text();

            // Get current protocol from dropdown
            let protocol = match protocol_dropdown.selected() {
                0 => "ssh",
                1 => "rdp",
                2 => "vnc",
                3 => "spice",
                4 => "zerotrust",
                _ => "ssh",
            };

            if name.trim().is_empty() && host.trim().is_empty() {
                crate::toast::show_toast_on_window(
                    &window,
                    "Please enter a connection name or host to look up",
                    crate::toast::ToastType::Warning,
                );
                return;
            }

            callback(
                &name,
                &host,
                protocol,
                password_entry.clone(),
                window.clone().into(),
            );
        });
    }

    /// Returns the password entry widget for external access
    #[must_use]
    pub const fn password_entry(&self) -> &Entry {
        &self.password_entry
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
}

/// Helper struct for validation and building in the response callback
struct ConnectionDialogData<'a> {
    name_entry: &'a Entry,
    description_view: &'a TextView,
    host_entry: &'a Entry,
    port_spin: &'a SpinButton,
    username_entry: &'a Entry,
    tags_entry: &'a Entry,
    protocol_dropdown: &'a DropDown,
    password_source_dropdown: &'a DropDown,
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
    ssh_startup_entry: &'a Entry,
    ssh_options_entry: &'a Entry,
    rdp_client_mode_dropdown: &'a DropDown,
    rdp_performance_mode_dropdown: &'a DropDown,
    rdp_width_spin: &'a SpinButton,
    rdp_height_spin: &'a SpinButton,
    rdp_color_dropdown: &'a DropDown,
    rdp_audio_check: &'a CheckButton,
    rdp_gateway_entry: &'a Entry,
    rdp_shared_folders: &'a Rc<RefCell<Vec<SharedFolder>>>,
    rdp_custom_args_entry: &'a Entry,
    vnc_client_mode_dropdown: &'a DropDown,
    vnc_performance_mode_dropdown: &'a DropDown,
    vnc_encoding_entry: &'a Entry,
    vnc_compression_spin: &'a SpinButton,
    vnc_quality_spin: &'a SpinButton,
    vnc_view_only_check: &'a CheckButton,
    vnc_scaling_check: &'a CheckButton,
    vnc_clipboard_check: &'a CheckButton,
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
    zt_aws_target_entry: &'a Entry,
    zt_aws_profile_entry: &'a Entry,
    zt_aws_region_entry: &'a Entry,
    zt_gcp_instance_entry: &'a Entry,
    zt_gcp_zone_entry: &'a Entry,
    zt_gcp_project_entry: &'a Entry,
    zt_azure_bastion_resource_id_entry: &'a Entry,
    zt_azure_bastion_rg_entry: &'a Entry,
    zt_azure_bastion_name_entry: &'a Entry,
    zt_azure_ssh_vm_entry: &'a Entry,
    zt_azure_ssh_rg_entry: &'a Entry,
    zt_oci_bastion_id_entry: &'a Entry,
    zt_oci_target_id_entry: &'a Entry,
    zt_oci_target_ip_entry: &'a Entry,
    zt_cf_hostname_entry: &'a Entry,
    zt_teleport_host_entry: &'a Entry,
    zt_teleport_cluster_entry: &'a Entry,
    zt_tailscale_host_entry: &'a Entry,
    zt_boundary_target_entry: &'a Entry,
    zt_boundary_addr_entry: &'a Entry,
    zt_generic_command_entry: &'a Entry,
    zt_custom_args_entry: &'a Entry,
    local_variables: &'a HashMap<String, Variable>,
    logging_enabled_check: &'a CheckButton,
    logging_path_entry: &'a Entry,
    logging_timestamp_dropdown: &'a DropDown,
    logging_max_size_spin: &'a SpinButton,
    logging_retention_spin: &'a SpinButton,
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
        // 0=SSH, 1=RDP, 2=VNC, 3=SPICE, 4=Zero Trust
        let protocol_idx = self.protocol_dropdown.selected();
        let is_zerotrust = protocol_idx == 4;

        // Host and port are optional for Zero Trust (defined in provider config)
        if !is_zerotrust {
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

        Ok(())
    }

    fn build_connection(&self) -> Option<Connection> {
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

        let username = self.username_entry.text();
        if !username.trim().is_empty() {
            conn.username = Some(username.trim().to_string());
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
        // Dropdown order: Prompt(0), Stored(1), KeePass(2), Keyring(3), Inherit(4), None(5)
        conn.password_source = match self.password_source_dropdown.selected() {
            1 => PasswordSource::Stored,
            2 => PasswordSource::KeePass,
            3 => PasswordSource::Keyring,
            4 => PasswordSource::Inherit,
            5 => PasswordSource::None,
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

        Some(conn)
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
        let enabled = self.logging_enabled_check.is_active();

        // If not enabled, return None
        if !enabled {
            return None;
        }

        let path_template = self.logging_path_entry.text().trim().to_string();

        // Use default path if empty
        let path_template = if path_template.is_empty() {
            "${HOME}/.local/share/rustconn/logs/${connection_name}_${date}.log".to_string()
        } else {
            path_template
        };

        // Map dropdown index to timestamp format
        let timestamp_formats = [
            "%Y-%m-%d %H:%M:%S",
            "%H:%M:%S",
            "%Y-%m-%d %H:%M:%S%.3f",
            "[%Y-%m-%d %H:%M:%S]",
            "%d/%m/%Y %H:%M:%S",
        ];
        let timestamp_idx = self.logging_timestamp_dropdown.selected() as usize;
        let timestamp_format = timestamp_formats
            .get(timestamp_idx)
            .unwrap_or(&timestamp_formats[0])
            .to_string();

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_size_mb = self.logging_max_size_spin.value() as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let retention_days = self.logging_retention_spin.value() as u32;

        Some(LogConfig {
            enabled,
            path_template,
            timestamp_format,
            max_size_mb,
            retention_days,
            log_activity: true,
            log_input: false,
            log_output: false,
        })
    }

    fn build_protocol_config(&self) -> Option<ProtocolConfig> {
        let protocol_idx = self.protocol_dropdown.selected();

        match protocol_idx {
            0 => Some(ProtocolConfig::Ssh(self.build_ssh_config())),
            1 => Some(ProtocolConfig::Rdp(self.build_rdp_config())),
            2 => Some(ProtocolConfig::Vnc(self.build_vnc_config())),
            3 => Some(ProtocolConfig::Spice(self.build_spice_config())),
            4 => Some(ProtocolConfig::ZeroTrust(self.build_zerotrust_config())),
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

    fn build_ssh_config(&self) -> SshConfig {
        let auth_method = match self.ssh_auth_dropdown.selected() {
            1 => SshAuthMethod::PublicKey,
            2 => SshAuthMethod::KeyboardInteractive,
            3 => SshAuthMethod::Agent,
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
            custom_options,
            startup_command,
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
