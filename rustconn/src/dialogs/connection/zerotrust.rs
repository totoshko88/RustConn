//! Zero Trust protocol options for the connection dialog
//!
//! This module provides the Zero Trust-specific UI components including:
//! - Provider selection (AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, etc.)
//! - Provider-specific configuration fields
//! - Custom arguments for CLI commands

// These functions are prepared for future refactoring when dialog.rs is further modularized
#![allow(dead_code)]
// OCI Bastion has target_id and target_ip fields which are semantically different
#![allow(clippy::similar_names)]

use super::protocol_layout::ProtocolLayoutBuilder;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DropDown, Entry, Orientation, Stack, StringList};
use libadwaita as adw;

/// Return type for Zero Trust options creation
#[allow(clippy::type_complexity)]
pub type ZeroTrustOptionsWidgets = (
    GtkBox,
    DropDown,      // provider_dropdown
    Stack,         // provider_stack
    adw::EntryRow, // aws_target
    adw::EntryRow, // aws_profile
    adw::EntryRow, // aws_region
    adw::EntryRow, // gcp_instance
    adw::EntryRow, // gcp_zone
    adw::EntryRow, // gcp_project
    adw::EntryRow, // azure_bastion_resource_id
    adw::EntryRow, // azure_bastion_rg
    adw::EntryRow, // azure_bastion_name
    adw::EntryRow, // azure_ssh_vm
    adw::EntryRow, // azure_ssh_rg
    adw::EntryRow, // oci_bastion_id
    adw::EntryRow, // oci_target_id
    adw::EntryRow, // oci_target_ip
    adw::EntryRow, // cf_hostname
    adw::EntryRow, // teleport_host
    adw::EntryRow, // teleport_cluster
    adw::EntryRow, // tailscale_host
    adw::EntryRow, // boundary_target
    adw::EntryRow, // boundary_addr
    adw::EntryRow, // generic_command
    Entry,         // custom_args_entry
);

/// Creates the Zero Trust options panel using libadwaita components following GNOME HIG.
#[must_use]
pub fn create_zerotrust_options() -> ZeroTrustOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Provider Selection Group ===
    let (provider_group, provider_dropdown) = create_provider_selection();
    content.append(&provider_group);

    // Provider-specific stack
    let provider_stack = Stack::new();
    provider_stack.set_vexpand(true);

    // AWS SSM options
    let (aws_box, aws_target, aws_profile, aws_region) = create_aws_ssm_fields();
    provider_stack.add_named(&aws_box, Some("aws_ssm"));

    // GCP IAP options
    let (gcp_box, gcp_instance, gcp_zone, gcp_project) = create_gcp_iap_fields();
    provider_stack.add_named(&gcp_box, Some("gcp_iap"));

    // Azure Bastion options
    let (azure_bastion_box, azure_bastion_resource_id, azure_bastion_rg, azure_bastion_name) =
        create_azure_bastion_fields();
    provider_stack.add_named(&azure_bastion_box, Some("azure_bastion"));

    // Azure SSH options
    let (azure_ssh_box, azure_ssh_vm, azure_ssh_rg) = create_azure_ssh_fields();
    provider_stack.add_named(&azure_ssh_box, Some("azure_ssh"));

    // OCI Bastion options
    let (oci_box, oci_bastion_id, oci_target_id, oci_target_ip) = create_oci_bastion_fields();
    provider_stack.add_named(&oci_box, Some("oci_bastion"));

    // Cloudflare Access options
    let (cf_box, cf_hostname) = create_cloudflare_fields();
    provider_stack.add_named(&cf_box, Some("cloudflare"));

    // Teleport options
    let (teleport_box, teleport_host, teleport_cluster) = create_teleport_fields();
    provider_stack.add_named(&teleport_box, Some("teleport"));

    // Tailscale SSH options
    let (tailscale_box, tailscale_host) = create_tailscale_fields();
    provider_stack.add_named(&tailscale_box, Some("tailscale"));

    // Boundary options
    let (boundary_box, boundary_target, boundary_addr) = create_boundary_fields();
    provider_stack.add_named(&boundary_box, Some("boundary"));

    // Generic command options
    let (generic_box, generic_command) = create_generic_fields();
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
    let (advanced_group, custom_args_entry) = create_advanced_group();
    content.append(&advanced_group);

    (
        container,
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

/// Creates the provider selection group
fn create_provider_selection() -> (adw::PreferencesGroup, DropDown) {
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

    (provider_group, provider_dropdown)
}

/// Creates the advanced options group
fn create_advanced_group() -> (adw::PreferencesGroup, Entry) {
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

    (advanced_group, custom_args_entry)
}

/// Creates AWS SSM provider fields
fn create_aws_ssm_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("AWS Session Manager")
        .description("Connect via AWS Systems Manager")
        .build();

    let target_row = adw::EntryRow::builder().title("Instance ID").build();
    target_row.set_text("");
    group.add(&target_row);

    let profile_row = adw::EntryRow::builder().title("AWS Profile").build();
    profile_row.set_text("default");
    group.add(&profile_row);

    let region_row = adw::EntryRow::builder().title("Region").build();
    region_row.set_text("");
    group.add(&region_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, target_row, profile_row, region_row)
}

/// Creates GCP IAP provider fields
fn create_gcp_iap_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("GCP IAP Tunnel")
        .description("Connect via Identity-Aware Proxy")
        .build();

    let instance_row = adw::EntryRow::builder().title("Instance Name").build();
    instance_row.set_text("");
    group.add(&instance_row);

    let zone_row = adw::EntryRow::builder().title("Zone").build();
    zone_row.set_text("");
    group.add(&zone_row);

    let project_row = adw::EntryRow::builder().title("Project").build();
    project_row.set_text("");
    group.add(&project_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, instance_row, zone_row, project_row)
}

/// Creates Azure Bastion provider fields
fn create_azure_bastion_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Azure Bastion")
        .description("Connect via Azure Bastion service")
        .build();

    let resource_id_row = adw::EntryRow::builder().title("Target Resource ID").build();
    resource_id_row.set_text("");
    group.add(&resource_id_row);

    let rg_row = adw::EntryRow::builder().title("Resource Group").build();
    rg_row.set_text("");
    group.add(&rg_row);

    let name_row = adw::EntryRow::builder().title("Bastion Name").build();
    name_row.set_text("");
    group.add(&name_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, resource_id_row, rg_row, name_row)
}

/// Creates Azure SSH (AAD) provider fields
fn create_azure_ssh_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Azure SSH (AAD)")
        .description("Connect via Azure AD authentication")
        .build();

    let vm_row = adw::EntryRow::builder().title("VM Name").build();
    vm_row.set_text("");
    group.add(&vm_row);

    let rg_row = adw::EntryRow::builder().title("Resource Group").build();
    rg_row.set_text("");
    group.add(&rg_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, vm_row, rg_row)
}

/// Creates OCI Bastion provider fields
fn create_oci_bastion_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("OCI Bastion")
        .description("Connect via Oracle Cloud Bastion")
        .build();

    let bastion_id_row = adw::EntryRow::builder().title("Bastion OCID").build();
    bastion_id_row.set_text("");
    group.add(&bastion_id_row);

    let target_id_row = adw::EntryRow::builder().title("Target OCID").build();
    target_id_row.set_text("");
    group.add(&target_id_row);

    let target_ip_row = adw::EntryRow::builder().title("Target IP").build();
    target_ip_row.set_text("");
    group.add(&target_ip_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, bastion_id_row, target_id_row, target_ip_row)
}

/// Creates Cloudflare Access provider fields
fn create_cloudflare_fields() -> (GtkBox, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Cloudflare Access")
        .description("Connect via Cloudflare Zero Trust")
        .build();

    let hostname_row = adw::EntryRow::builder().title("Hostname").build();
    hostname_row.set_text("");
    group.add(&hostname_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, hostname_row)
}

/// Creates Teleport provider fields
fn create_teleport_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Teleport")
        .description("Connect via Gravitational Teleport")
        .build();

    let host_row = adw::EntryRow::builder().title("Node Name").build();
    host_row.set_text("");
    group.add(&host_row);

    let cluster_row = adw::EntryRow::builder().title("Cluster").build();
    cluster_row.set_text("");
    group.add(&cluster_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, host_row, cluster_row)
}

/// Creates Tailscale SSH provider fields
fn create_tailscale_fields() -> (GtkBox, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Tailscale SSH")
        .description("Connect via Tailscale network")
        .build();

    let host_row = adw::EntryRow::builder().title("Tailscale Host").build();
    host_row.set_text("");
    group.add(&host_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, host_row)
}

/// Creates HashiCorp Boundary provider fields
fn create_boundary_fields() -> (GtkBox, adw::EntryRow, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("HashiCorp Boundary")
        .description("Connect via Boundary proxy")
        .build();

    let target_row = adw::EntryRow::builder().title("Target ID").build();
    target_row.set_text("");
    group.add(&target_row);

    let addr_row = adw::EntryRow::builder().title("Controller Address").build();
    addr_row.set_text("");
    group.add(&addr_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, target_row, addr_row)
}

/// Creates Generic Zero Trust provider fields
fn create_generic_fields() -> (GtkBox, adw::EntryRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Generic Command")
        .description("Custom command for unsupported providers")
        .build();

    let command_row = adw::EntryRow::builder().title("Command Template").build();
    command_row.set_text("");
    group.add(&command_row);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&group);

    (vbox, command_row)
}
