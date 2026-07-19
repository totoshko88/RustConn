//! Save button handling: validation and result assembly
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, CheckButton, ColorDialogButton, DropDown, Entry, SpinButton, TextView};
use libadwaita as adw;
use rustconn_core::automation::ExpectRule;
use rustconn_core::models::{CustomProperty, HighlightRule, SharedFolder};
use uuid::Uuid;

use super::{ConnectionDialog, LocalVariableRow};
use crate::alert;
use crate::dialogs::connection::builders::ConnectionDialogData;
use crate::dialogs::connection::logging_tab;
use crate::i18n::i18n;

impl ConnectionDialog {
    /// Connects the save button to validate and save the connection
    #[expect(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "long match dispatch with many flat parameters; restructuring would only move the parameter list elsewhere"
    )]
    pub(super) fn connect_save_button(
        save_btn: &Button,
        dialog: &adw::Dialog,
        on_save: &crate::dialogs::connection::ConnectionCallback,
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
        ssh_pkcs11_entry: &adw::EntryRow,
        ssh_remote_path_entry: &adw::EntryRow,
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
        rdp_printer_check: &adw::SwitchRow,
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
        rdp_graphics_mode_dropdown: &DropDown,
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
        vnc_accept_certificate_check: &adw::SwitchRow,
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
        spice_unix_socket_check: &adw::SwitchRow,
        spice_socket_path_entry: &Entry,
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
        web_browser_mode_combo: &adw::ComboRow,
        web_javascript_switch: &adw::SwitchRow,
        web_user_agent_row: &adw::EntryRow,
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
        knock_sequence_entry: &gtk4::Entry,
        spa_enabled_toggle: &adw::SwitchRow,
        spa_rij_key_entry: &adw::PasswordEntryRow,
        spa_hmac_key_entry: &adw::PasswordEntryRow,
        spa_access_entry: &adw::EntryRow,
        spa_port_spin: &adw::SpinRow,
        spa_allow_ip_combo: &adw::ComboRow,
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
        let ssh_pkcs11_entry = ssh_pkcs11_entry.clone();
        let ssh_remote_path_entry = ssh_remote_path_entry.clone();
        let ssh_keep_alive_interval = ssh_keep_alive_interval.clone();
        let ssh_keep_alive_count_max = ssh_keep_alive_count_max.clone();
        let ssh_port_forwards = ssh_port_forwards.clone();
        let rdp_client_mode_dropdown = rdp_client_mode_dropdown.clone();
        let rdp_width_spin = rdp_width_spin.clone();
        let rdp_height_spin = rdp_height_spin.clone();
        let rdp_color_dropdown = rdp_color_dropdown.clone();
        let rdp_scale_override_dropdown = rdp_scale_override_dropdown.clone();
        let rdp_audio_check = rdp_audio_check.clone();
        let rdp_printer_check = rdp_printer_check.clone();
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
        let rdp_graphics_mode_dropdown = rdp_graphics_mode_dropdown.clone();
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
        let vnc_accept_certificate_check = vnc_accept_certificate_check.clone();
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
        let spice_unix_socket_check = spice_unix_socket_check.clone();
        let spice_socket_path_entry = spice_socket_path_entry.clone();
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
        let web_browser_mode_combo = web_browser_mode_combo.clone();
        let web_javascript_switch = web_javascript_switch.clone();
        let web_user_agent_row = web_user_agent_row.clone();
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
        let knock_sequence_entry = knock_sequence_entry.clone();
        let spa_enabled_toggle = spa_enabled_toggle.clone();
        let spa_rij_key_entry = spa_rij_key_entry.clone();
        let spa_hmac_key_entry = spa_hmac_key_entry.clone();
        let spa_access_entry = spa_access_entry.clone();
        let spa_port_spin = spa_port_spin.clone();
        let spa_allow_ip_combo = spa_allow_ip_combo.clone();

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
                ssh_pkcs11_entry: &ssh_pkcs11_entry,
                ssh_remote_path_entry: &ssh_remote_path_entry,
                ssh_keep_alive_interval: &ssh_keep_alive_interval,
                ssh_keep_alive_count_max: &ssh_keep_alive_count_max,
                ssh_port_forwards: &ssh_port_forwards,
                rdp_client_mode_dropdown: &rdp_client_mode_dropdown,
                rdp_width_spin: &rdp_width_spin,
                rdp_height_spin: &rdp_height_spin,
                rdp_color_dropdown: &rdp_color_dropdown,
                rdp_scale_override_dropdown: &rdp_scale_override_dropdown,
                rdp_audio_check: &rdp_audio_check,
                rdp_printer_check: &rdp_printer_check,
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
                rdp_graphics_mode_dropdown: &rdp_graphics_mode_dropdown,
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
                vnc_accept_certificate_check: &vnc_accept_certificate_check,
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
                spice_unix_socket_check: &spice_unix_socket_check,
                spice_socket_path_entry: &spice_socket_path_entry,
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
                web_browser_mode_combo: &web_browser_mode_combo,
                web_javascript_switch: &web_javascript_switch,
                web_user_agent_row: &web_user_agent_row,
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
                knock_sequence_entry: &knock_sequence_entry,
                spa_enabled_toggle: &spa_enabled_toggle,
                spa_rij_key_entry: &spa_rij_key_entry,
                spa_hmac_key_entry: &spa_hmac_key_entry,
                spa_access_entry: &spa_access_entry,
                spa_port_spin: &spa_port_spin,
                spa_allow_ip_combo: &spa_allow_ip_combo,
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
}
