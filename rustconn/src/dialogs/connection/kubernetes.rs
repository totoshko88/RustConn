//! Kubernetes protocol options for the connection dialog
//!
//! UI panel for Kubernetes pod shell connections via `kubectl exec`.

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::EntryRowBuilder;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, CheckButton, DropDown, Entry, StringList};
use libadwaita as adw;

/// Return type for Kubernetes options creation
///
/// Contains:
/// - Container box
/// - Kubeconfig entry
/// - Context entry
/// - Namespace entry
/// - Pod entry
/// - Container entry
/// - Shell dropdown
/// - Busybox toggle
/// - Busybox image entry
/// - Custom args entry
#[allow(clippy::type_complexity)]
pub type KubernetesOptionsWidgets = (
    GtkBox,
    Entry,
    Entry,
    Entry,
    Entry,
    Entry,
    DropDown,
    CheckButton,
    Entry,
    Entry,
);

/// Creates the Kubernetes options panel using libadwaita components.
#[must_use]
pub fn create_kubernetes_options() -> KubernetesOptionsWidgets {
    let (container, content) = ProtocolLayoutBuilder::new().build();

    // === Connection Group ===
    let connection_group = adw::PreferencesGroup::builder()
        .title("Kubernetes")
        .description("Connect to pod shell via kubectl exec")
        .build();

    let (kubeconfig_row, kubeconfig_entry) = EntryRowBuilder::new("Kubeconfig")
        .subtitle("Path to kubeconfig file (default if empty)")
        .placeholder("~/.kube/config")
        .build();
    connection_group.add(&kubeconfig_row);

    let (context_row, context_entry) = EntryRowBuilder::new("Context")
        .subtitle("Kubernetes context (current-context if empty)")
        .placeholder("my-cluster")
        .build();
    connection_group.add(&context_row);

    let (namespace_row, namespace_entry) = EntryRowBuilder::new("Namespace")
        .subtitle("Target namespace (default if empty)")
        .placeholder("default")
        .build();
    connection_group.add(&namespace_row);

    let (pod_row, pod_entry) = EntryRowBuilder::new("Pod")
        .subtitle("Pod name to exec into")
        .placeholder("my-pod-abc123")
        .build();
    connection_group.add(&pod_row);

    let (container_row, container_entry) = EntryRowBuilder::new("Container")
        .subtitle("Container name (optional for single-container)")
        .placeholder("app")
        .build();
    connection_group.add(&container_row);

    // Shell dropdown
    let shell_model = StringList::new(&["/bin/sh", "/bin/bash", "/bin/ash", "/bin/zsh"]);
    let shell_dropdown = DropDown::builder().model(&shell_model).selected(0).build();
    let shell_row = adw::ActionRow::builder()
        .title("Shell")
        .subtitle("Shell to use inside the container")
        .build();
    shell_row.add_suffix(&shell_dropdown);
    shell_row.set_activatable_widget(Some(&shell_dropdown));
    connection_group.add(&shell_row);

    content.append(&connection_group);

    // === Busybox Group ===
    let busybox_group = adw::PreferencesGroup::builder()
        .title("Temporary Pod")
        .description("Run a temporary pod instead of exec into existing")
        .build();

    let busybox_check = CheckButton::builder().build();
    let busybox_row = adw::ActionRow::builder()
        .title("Busybox Mode")
        .subtitle("Creates a temporary pod with kubectl run")
        .activatable_widget(&busybox_check)
        .build();
    busybox_row.add_suffix(&busybox_check);
    busybox_group.add(&busybox_row);

    let (busybox_image_row, busybox_image_entry) = EntryRowBuilder::new("Image")
        .subtitle("Container image for temporary pod")
        .placeholder("busybox:latest")
        .build();
    busybox_group.add(&busybox_image_row);

    content.append(&busybox_group);

    // === Advanced Group ===
    let advanced_group = adw::PreferencesGroup::builder().title("Advanced").build();

    let (custom_args_row, custom_args_entry) = EntryRowBuilder::new("Custom Arguments")
        .subtitle("Additional kubectl arguments")
        .placeholder("--request-timeout=30s")
        .build();
    advanced_group.add(&custom_args_row);

    content.append(&advanced_group);

    (
        container,
        kubeconfig_entry,
        context_entry,
        namespace_entry,
        pod_entry,
        container_entry,
        shell_dropdown,
        busybox_check,
        busybox_image_entry,
        custom_args_entry,
    )
}
