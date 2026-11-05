//! Kubernetes protocol options for the connection dialog
//!
//! UI panel for Kubernetes pod shell connections via `kubectl exec`.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CheckButton, DropDown, Entry, Label, ListBox, MenuButton, Popover,
    ScrolledWindow, StringList,
};
use libadwaita as adw;

use super::protocol_layout::ProtocolLayoutBuilder;
use super::widgets::EntryRowBuilder;
use crate::i18n::i18n;

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
        .title(i18n("Kubernetes"))
        .description(i18n("Connect to pod shell via kubectl exec"))
        .build();

    let (kubeconfig_row, kubeconfig_entry) = EntryRowBuilder::new(i18n("Kubeconfig"))
        .subtitle(i18n("Path to kubeconfig file (default if empty)"))
        .placeholder("~/.kube/config")
        .build();
    connection_group.add(&kubeconfig_row);

    let (context_row, context_entry) = EntryRowBuilder::new(i18n("Context"))
        .subtitle(i18n("Kubernetes context (current-context if empty)"))
        .placeholder("my-cluster")
        .build();
    // A button that lists the contexts found in the kubeconfig the user named
    // (or the default one) so a context can be picked instead of typed. The
    // Context field stays a free-text entry — a context not yet in the file, or
    // a kubeconfig the agent cannot read, can still be entered by hand (W5).
    let context_picker = build_context_picker(&kubeconfig_entry, &context_entry);
    context_row.add_suffix(&context_picker);
    connection_group.add(&context_row);

    let (namespace_row, namespace_entry) = EntryRowBuilder::new(i18n("Namespace"))
        .subtitle(i18n("Target namespace (default if empty)"))
        .placeholder("default")
        .build();
    connection_group.add(&namespace_row);

    let (pod_row, pod_entry) = EntryRowBuilder::new(i18n("Pod"))
        .subtitle(i18n("Pod name to exec into"))
        .placeholder("my-pod-abc123")
        .build();
    connection_group.add(&pod_row);

    let (container_row, container_entry) = EntryRowBuilder::new(i18n("Container"))
        .subtitle(i18n("Container name (optional for single-container)"))
        .placeholder("app")
        .build();
    connection_group.add(&container_row);

    // Shell dropdown
    let shell_model = StringList::new(&["/bin/sh", "/bin/bash", "/bin/ash", "/bin/zsh"]);
    let shell_dropdown = DropDown::builder().model(&shell_model).selected(0).build();
    let shell_row = adw::ActionRow::builder()
        .title(i18n("Shell"))
        .subtitle(i18n("Shell to use inside the container"))
        .build();
    shell_row.add_suffix(&shell_dropdown);
    shell_row.set_activatable_widget(Some(&shell_dropdown));
    connection_group.add(&shell_row);

    content.append(&connection_group);

    // === Busybox Group ===
    let busybox_group = adw::PreferencesGroup::builder()
        .title(i18n("Temporary Pod"))
        .description(i18n("Run a temporary pod instead of exec into existing"))
        .build();

    let busybox_check = CheckButton::builder().build();
    let busybox_row = adw::ActionRow::builder()
        .title(i18n("Busybox Mode"))
        .subtitle(i18n("Creates a temporary pod with kubectl run"))
        .activatable_widget(&busybox_check)
        .build();
    busybox_row.add_suffix(&busybox_check);
    busybox_group.add(&busybox_row);

    let (busybox_image_row, busybox_image_entry) = EntryRowBuilder::new(i18n("Image"))
        .subtitle(i18n("Container image for temporary pod"))
        .placeholder("busybox:latest")
        .build();
    busybox_image_entry.set_sensitive(false);
    busybox_group.add(&busybox_image_row);

    // Wire busybox toggle to image entry and pod sensitivity
    let busybox_image_entry_clone = busybox_image_entry.clone();
    let pod_entry_clone = pod_entry.clone();
    let container_entry_clone = container_entry.clone();
    busybox_check.connect_toggled(move |check| {
        let on = check.is_active();
        busybox_image_entry_clone.set_sensitive(on);
        // When busybox is on, pod/container are not needed
        pod_entry_clone.set_sensitive(!on);
        container_entry_clone.set_sensitive(!on);
    });

    content.append(&busybox_group);

    // === Advanced Group ===
    let advanced_group = adw::PreferencesGroup::builder()
        .title(i18n("Advanced"))
        .build();

    let (custom_args_row, custom_args_entry) = EntryRowBuilder::new(i18n("Custom Arguments"))
        .subtitle(i18n("Additional kubectl arguments"))
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

/// Builds the "pick a context" button shown as a suffix on the Context row.
///
/// Clicking it opens a popover listing the contexts in whatever kubeconfig the
/// Kubeconfig field currently names (or the default one when it is empty). The
/// list is rebuilt every time the popover opens, so editing the Kubeconfig path
/// first and then opening this shows that file's contexts. Choosing one fills
/// the Context entry. The current-context is marked, and an empty or unreadable
/// kubeconfig shows a short "no contexts" note rather than an empty popover.
fn build_context_picker(kubeconfig_entry: &Entry, context_entry: &Entry) -> MenuButton {
    let popover = Popover::new();
    let menu_button = MenuButton::builder()
        .icon_name("view-list-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Choose a context from the kubeconfig"))
        .popover(&popover)
        .build();
    menu_button.add_css_class("flat");
    menu_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Choose a context from the kubeconfig",
    ))]);

    let kubeconfig_entry = kubeconfig_entry.clone();
    let context_entry = context_entry.clone();
    let popover_rebuild = popover.clone();
    // Rebuild on each open so a changed Kubeconfig path is reflected.
    menu_button.connect_active_notify(move |button| {
        if !button.is_active() {
            return;
        }
        let kubeconfig_text = kubeconfig_entry.text().to_string();
        let path = resolve_kubeconfig_entry_path(&kubeconfig_text);
        let contexts = rustconn_core::protocol::read_kubeconfig_contexts(path.as_deref());
        popover_rebuild.set_child(Some(&build_context_list(
            &contexts,
            &context_entry,
            &popover_rebuild,
        )));
    });

    menu_button
}

/// Turns a Kubeconfig entry value into a path to read, or `None` to let the core
/// fall back to `$KUBECONFIG` / `~/.kube/config`. Expands a leading `~`.
fn resolve_kubeconfig_entry_path(text: &str) -> Option<std::path::PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return Some(home.join(rest));
    }
    Some(std::path::PathBuf::from(trimmed))
}

/// Builds the popover body listing `contexts`; clicking a row fills `entry`.
fn build_context_list(
    contexts: &rustconn_core::protocol::KubeconfigContexts,
    entry: &Entry,
    popover: &Popover,
) -> gtk4::Widget {
    if contexts.names.is_empty() {
        return Label::builder()
            .label(i18n("No contexts found in the kubeconfig"))
            .css_classes(["dim-label"])
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build()
            .upcast();
    }

    let list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    for name in &contexts.names {
        let row = adw::ActionRow::builder()
            .title(name)
            .activatable(true)
            .build();
        if contexts.current.as_deref() == Some(name.as_str()) {
            // Mark the current-context with a check, so the default is obvious.
            let check = gtk4::Image::from_icon_name("object-select-symbolic");
            row.add_suffix(&check);
            row.set_subtitle(&i18n("Current context"));
        }
        let entry = entry.clone();
        let popover = popover.clone();
        let name = name.clone();
        row.connect_activated(move |_| {
            entry.set_text(&name);
            popover.popdown();
        });
        list.append(&row);
    }

    ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_width(240)
        .max_content_height(320)
        .propagate_natural_height(true)
        .child(&list)
        .build()
        .upcast()
}
