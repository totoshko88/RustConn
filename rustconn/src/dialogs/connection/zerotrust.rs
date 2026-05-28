//! Zero Trust protocol options for the connection dialog
//!
//! This module provides the Zero Trust-specific UI components including:
//! - Provider selection (AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, etc.)
//! - Provider-specific configuration fields
//! - Custom arguments for CLI commands

// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, Orientation, ScrolledWindow, Stack, StringList};
use libadwaita as adw;

use crate::i18n::i18n;

/// Creates the Zero Trust options panel with provider-specific fields using libadwaita.
#[expect(
    clippy::too_many_lines,
    clippy::type_complexity,
    reason = "long match dispatch returning a documented tuple; aliasing would obscure the data flow"
)]
pub(super) fn create_zerotrust_options() -> (
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
    adw::EntryRow, // oci_ssh_key
    adw::SpinRow,  // oci_session_ttl
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow, // hoop_connection_name
    adw::EntryRow, // hoop_gateway_url
    adw::EntryRow, // hoop_grpc_url
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
        i18n("Hoop.dev"),
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
    let (aws_box, aws_target, aws_profile, aws_region) = create_aws_ssm_fields_adw();
    provider_stack.add_named(&aws_box, Some("aws_ssm"));

    // GCP IAP options
    let (gcp_box, gcp_instance, gcp_zone, gcp_project) = create_gcp_iap_fields_adw();
    provider_stack.add_named(&gcp_box, Some("gcp_iap"));

    // Azure Bastion options
    let (azure_bastion_box, azure_bastion_resource_id, azure_bastion_rg, azure_bastion_name) =
        create_azure_bastion_fields_adw();
    provider_stack.add_named(&azure_bastion_box, Some("azure_bastion"));

    // Azure SSH options
    let (azure_ssh_box, azure_ssh_vm, azure_ssh_rg) = create_azure_ssh_fields_adw();
    provider_stack.add_named(&azure_ssh_box, Some("azure_ssh"));

    // OCI Bastion options
    let (oci_box, oci_bastion_id, oci_target_id, oci_target_ip, oci_ssh_key, oci_session_ttl) =
        create_oci_bastion_fields_adw();
    provider_stack.add_named(&oci_box, Some("oci_bastion"));

    // Cloudflare Access options
    let (cf_box, cf_hostname) = create_cloudflare_fields_adw();
    provider_stack.add_named(&cf_box, Some("cloudflare"));

    // Teleport options
    let (teleport_box, teleport_host, teleport_cluster) = create_teleport_fields_adw();
    provider_stack.add_named(&teleport_box, Some("teleport"));

    // Tailscale SSH options
    let (tailscale_box, tailscale_host) = create_tailscale_fields_adw();
    provider_stack.add_named(&tailscale_box, Some("tailscale"));

    // Boundary options
    let (boundary_box, boundary_target, boundary_addr) = create_boundary_fields_adw();
    provider_stack.add_named(&boundary_box, Some("boundary"));

    // Hoop.dev options
    let (hoop_box, hoop_connection_name, hoop_gateway_url, hoop_grpc_url) =
        create_hoop_dev_fields_adw();
    provider_stack.add_named(&hoop_box, Some("hoop_dev"));

    // Generic command options
    let (generic_box, generic_command) = create_generic_zt_fields_adw();
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
            "hoop_dev",
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
        oci_ssh_key,
        oci_session_ttl,
        cf_hostname,
        teleport_host,
        teleport_cluster,
        tailscale_host,
        boundary_target,
        boundary_addr,
        hoop_connection_name,
        hoop_gateway_url,
        hoop_grpc_url,
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
fn create_oci_bastion_fields_adw() -> (
    GtkBox,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::SpinRow,
) {
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

    // ZT-2: SSH Public Key file path
    let ssh_key_row = adw::EntryRow::builder()
        .title(i18n("SSH Public Key"))
        .build();
    ssh_key_row.set_text("~/.ssh/id_rsa.pub");
    group.add(&ssh_key_row);

    // ZT-2: Session TTL
    let ttl_adj = gtk4::Adjustment::new(1800.0, 300.0, 10800.0, 300.0, 600.0, 0.0);
    let ttl_row = adw::SpinRow::builder()
        .title(i18n("Session TTL"))
        .subtitle(i18n("Session duration in seconds (default: 1800)"))
        .adjustment(&ttl_adj)
        .build();
    group.add(&ttl_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (
        vbox,
        bastion_id_row,
        target_id_row,
        target_ip_row,
        ssh_key_row,
        ttl_row,
    )
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

/// Creates Hoop.dev provider fields using libadwaita
fn create_hoop_dev_fields_adw() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title(i18n("Hoop.dev"))
        .description(i18n("Connect via Hoop.dev zero-trust gateway"))
        .build();

    let connection_name_row = adw::EntryRow::builder()
        .title(i18n("Connection Name"))
        .build();
    group.add(&connection_name_row);

    let gateway_url_row = adw::EntryRow::builder().title(i18n("Gateway URL")).build();
    group.add(&gateway_url_row);

    let grpc_url_row = adw::EntryRow::builder().title(i18n("gRPC URL")).build();
    group.add(&grpc_url_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, connection_name_row, gateway_url_row, grpc_url_row)
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
