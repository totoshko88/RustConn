//! Dialog construction: `ConnectionDialog::new`
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use crate::alert;
use crate::dialogs::connection::logging_tab;
use crate::dialogs::connection::ssh;
use crate::i18n::{i18n, i18n_f};
use gtk4::ScrolledWindow;
use gtk4::prelude::*;
use rustconn_core::automation::ExpectRule;
use rustconn_core::models::{CustomProperty, HighlightRule};
use rustconn_core::variables::Variable;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

use super::{ConnectionDialog, LocalVariableRow};

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
        let basic = crate::dialogs::connection::general_tab::create_basic_tab();
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
        let ssh_pkcs11_entry = ssh_widgets.ssh_pkcs11_entry;
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
            rdp_printer_check,
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
        ) = crate::dialogs::connection::rdp::create_rdp_options();
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
            vnc_accept_certificate_check,
        ) = crate::dialogs::connection::vnc::create_vnc_options();
        protocol_stack.add_named(&vnc_box, Some("vnc"));

        // SPICE options
        let (
            spice_box,
            spice_tls_check,
            spice_ca_cert_entry,
            spice_skip_verify_check,
            spice_usb_check,
            spice_clipboard_check,
            spice_compression_dropdown,
            spice_proxy_entry,
            spice_show_local_cursor_check,
            spice_shared_folders,
            spice_shared_folders_list,
            spice_jump_host_dropdown,
            spice_unix_socket_check,
            spice_socket_path_entry,
        ) = crate::dialogs::connection::spice::create_spice_options();
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
        ) = crate::dialogs::connection::zerotrust::create_zerotrust_options();
        protocol_stack.add_named(&zt_box, Some("zerotrust"));

        // Telnet options
        let (
            telnet_box,
            telnet_custom_args_entry,
            telnet_backspace_dropdown,
            telnet_delete_dropdown,
        ) = crate::dialogs::connection::telnet::create_telnet_options();
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
        ) = crate::dialogs::connection::serial::create_serial_options();
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
        ) = crate::dialogs::connection::kubernetes::create_kubernetes_options();
        protocol_stack.add_named(&k8s_box, Some("kubernetes"));

        // Web bookmark options page
        let (web_box, web_browser_entry, web_private_mode_switch) =
            crate::dialogs::connection::web::create_web_options();
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
        ) = crate::dialogs::connection::data_tab::create_data_tab();
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
        let automation_widgets =
            crate::dialogs::connection::automation_tab::create_automation_combined_tab();
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
            knock_sequence_entry,
            spa_enabled_toggle,
            spa_rij_key_entry,
            spa_hmac_key_entry,
            spa_access_entry,
            spa_port_spin,
            spa_allow_ip_combo,
        ) = crate::dialogs::connection::advanced_tab::create_advanced_tab();
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

        let on_save: crate::dialogs::connection::ConnectionCallback = Rc::new(RefCell::new(None));
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
            &ssh_pkcs11_entry,
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
            &rdp_printer_check,
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
            &vnc_accept_certificate_check,
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
            &spice_unix_socket_check,
            &spice_socket_path_entry,
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
            &knock_sequence_entry,
            &spa_enabled_toggle,
            &spa_rij_key_entry,
            &spa_hmac_key_entry,
            &spa_access_entry,
            &spa_port_spin,
            &spa_allow_ip_combo,
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
            ssh_pkcs11_entry,
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
            rdp_printer_check,
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
            vnc_accept_certificate_check,
            vnc_connections_data,
            spice_tls_check,
            variables_list,
            variables_rows,
            add_variable_button,
            global_variables,
            logging_tab: logging_tab_struct,
            spice_ca_cert_entry,
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
            spice_unix_socket_check,
            spice_socket_path_entry,
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
            knock_sequence_entry,
            spa_enabled_toggle,
            spa_rij_key_entry,
            spa_hmac_key_entry,
            spa_access_entry,
            spa_port_spin,
            spa_allow_ip_combo,
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
                    &i18n("Fill in the required fields (name and host)"),
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
                            format!("Runtime error: {e}"),
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
                                    "Connection successful. Latency: {} ms",
                                    &[&latency.to_string()],
                                ),
                            );
                        }
                        Some(test_result) => {
                            let error = test_result
                                .error
                                .unwrap_or_else(|| {
                                    i18n("The test failed without a specific error. Check the host address and port, then try again.")
                                });
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
}
