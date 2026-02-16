//! Flatpak Components Dialog
//!
//! This dialog allows users to download and manage external CLI components
//! when running in Flatpak sandbox. It is only visible in Flatpak environment.
//!
//! Features:
//! - Download progress with percentage
//! - Cancel button for long downloads
//! - User-friendly error messages via toast
//! - SHA256 checksum verification for security

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, PolicyType, ScrolledWindow, Spinner};
use libadwaita as adw;
use rustconn_core::cli_download::{
    get_components_by_category, get_installation_status, get_user_friendly_error,
    install_component, uninstall_component, update_component, ComponentCategory,
    DownloadCancellation, DownloadableComponent,
};
use rustconn_core::is_flatpak;
use std::cell::RefCell;
use std::rc::Rc;

use crate::async_utils::spawn_async;

/// Dialog for managing Flatpak components
///
/// Note: `component_rows` field is kept alive for GTK widget lifecycle.
/// The rows contain GTK widgets that must remain valid while the dialog is open.
pub struct FlatpakComponentsDialog {
    /// The dialog window
    window: adw::Window,
    /// Toast overlay for notifications
    toast_overlay: adw::ToastOverlay,
    /// List of component rows for updating status
    /// Note: This field appears unused but is required to keep GTK widgets alive
    #[allow(dead_code)]
    component_rows: Rc<RefCell<Vec<ComponentRow>>>,
}

struct ComponentRow {
    component_id: &'static str,
    status_label: Label,
    action_button: Button,
    update_button: Button,
    cancel_button: Button,
    spinner: Spinner,
    cancel_token: Rc<RefCell<Option<DownloadCancellation>>>,
}

impl FlatpakComponentsDialog {
    /// Create a new Flatpak components dialog
    ///
    /// Returns `None` if not running in Flatpak
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Option<Self> {
        if !is_flatpak() {
            return None;
        }

        let window = adw::Window::builder()
            .title("Flatpak Components")
            .default_width(500)
            .default_height(500)
            .modal(true)
            .build();

        if let Some(parent) = parent {
            window.set_transient_for(Some(parent));
        }

        window.set_size_request(320, 280);

        let toast_overlay = adw::ToastOverlay::new();
        let component_rows = Rc::new(RefCell::new(Vec::new()));

        let content = Self::build_content(&component_rows);
        toast_overlay.set_child(Some(&content));

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&Self::build_header_bar(&window));
        toolbar_view.set_content(Some(&toast_overlay));

        window.set_content(Some(&toolbar_view));

        Some(Self {
            window,
            toast_overlay,
            component_rows,
        })
    }

    fn build_header_bar(window: &adw::Window) -> adw::HeaderBar {
        let header = adw::HeaderBar::new();

        let close_button = Button::with_label("Close");
        close_button.connect_clicked(glib::clone!(
            #[weak]
            window,
            move |_| {
                window.close();
            }
        ));
        header.pack_end(&close_button);

        header
    }

    fn build_content(component_rows: &Rc<RefCell<Vec<ComponentRow>>>) -> GtkBox {
        let content = GtkBox::new(Orientation::Vertical, 0);

        let scrolled = ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(PolicyType::Never)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        let inner = GtkBox::new(Orientation::Vertical, 24);

        // Info banner
        let info_banner = adw::Banner::new(
            "Showing sandbox-compatible components only. \
             Downloads are verified with SHA256 checksums.",
        );
        info_banner.set_revealed(true);
        inner.append(&info_banner);

        // Protocol clients section — may be empty in Flatpak
        // since xfreerdp/vncviewer need host display access
        let protocol_group = Self::build_category_group(
            "Protocol Clients",
            "Optional for external RDP/VNC/SPICE connections. \
             Embedded clients (IronRDP, vnc-rs) are preferred.",
            ComponentCategory::ProtocolClient,
            component_rows,
        );
        if Self::category_has_visible_components(ComponentCategory::ProtocolClient) {
            inner.append(&protocol_group);
        }

        // Zero Trust section
        let zerotrust_group = Self::build_category_group(
            "Zero Trust CLIs",
            "Required for Zero Trust connections (AWS SSM, GCP IAP, Azure Bastion, etc.)",
            ComponentCategory::ZeroTrust,
            component_rows,
        );
        if Self::category_has_visible_components(ComponentCategory::ZeroTrust) {
            inner.append(&zerotrust_group);
        }

        // Password managers section
        let password_group = Self::build_category_group(
            "Password Manager CLIs",
            "Required for Bitwarden and 1Password integration",
            ComponentCategory::PasswordManager,
            component_rows,
        );
        if Self::category_has_visible_components(ComponentCategory::PasswordManager) {
            inner.append(&password_group);
        }

        // Container orchestration section
        let k8s_group = Self::build_category_group(
            "Container Orchestration",
            "Required for Kubernetes pod shell connections",
            ComponentCategory::ContainerOrchestration,
            component_rows,
        );
        if Self::category_has_visible_components(ComponentCategory::ContainerOrchestration) {
            inner.append(&k8s_group);
        }

        clamp.set_child(Some(&inner));
        scrolled.set_child(Some(&clamp));
        content.append(&scrolled);

        content
    }

    /// Returns `true` if a category has any sandbox-compatible,
    /// downloadable components to display.
    fn category_has_visible_components(category: ComponentCategory) -> bool {
        get_components_by_category(category)
            .into_iter()
            .any(|c| c.works_in_sandbox && c.is_downloadable())
    }

    fn build_category_group(
        title: &str,
        description: &str,
        category: ComponentCategory,
        component_rows: &Rc<RefCell<Vec<ComponentRow>>>,
    ) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(title)
            .description(description)
            .build();

        let components: Vec<_> = get_components_by_category(category)
            .into_iter()
            .filter(|c| c.works_in_sandbox)
            .collect();
        let status = get_installation_status();

        for component in components {
            // Skip components that are not downloadable (e.g., FreeRDP)
            if !component.is_downloadable() {
                continue;
            }

            let is_installed = status
                .iter()
                .find(|(c, _)| c.id == component.id)
                .is_some_and(|(_, installed)| *installed);

            let row = Self::build_component_row(component, is_installed, component_rows);
            group.add(&row);
        }

        group
    }

    fn build_component_row(
        component: &'static DownloadableComponent,
        is_installed: bool,
        component_rows: &Rc<RefCell<Vec<ComponentRow>>>,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(component.name)
            .subtitle(component.description)
            .build();

        // Size hint label
        let size_label = Label::builder()
            .label(component.size_hint)
            .css_classes(["dim-label"])
            .build();
        row.add_suffix(&size_label);

        // Status label
        let status_label = Label::builder()
            .label(if is_installed { "Installed" } else { "" })
            .css_classes(if is_installed {
                vec!["success"]
            } else {
                vec![]
            })
            .build();
        row.add_suffix(&status_label);

        // Spinner (hidden by default)
        let spinner = Spinner::builder()
            .valign(Align::Center)
            .visible(false)
            .build();
        row.add_suffix(&spinner);

        // Cancel button (hidden by default)
        let cancel_button = Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text("Cancel")
            .valign(Align::Center)
            .visible(false)
            .build();
        cancel_button.add_css_class("flat");
        row.add_suffix(&cancel_button);

        // Update button (visible only when installed and downloadable)
        let update_button = Button::builder()
            .icon_name("emblem-synchronizing-symbolic")
            .tooltip_text("Update")
            .valign(Align::Center)
            .visible(is_installed && component.is_downloadable())
            .build();
        update_button.add_css_class("flat");
        row.add_suffix(&update_button);

        // Action button (Install/Remove)
        let action_button = Button::builder().valign(Align::Center).build();

        if is_installed {
            action_button.set_label("Remove");
            action_button.add_css_class("destructive-action");
        } else if component.is_downloadable() {
            action_button.set_label("Install");
            action_button.add_css_class("suggested-action");
        } else {
            action_button.set_label("N/A");
            action_button.set_sensitive(false);
            action_button.set_tooltip_text(Some("Not available for download"));
        }

        // Store row info for updates
        let cancel_token: Rc<RefCell<Option<DownloadCancellation>>> = Rc::new(RefCell::new(None));

        let row_info = ComponentRow {
            component_id: component.id,
            status_label: status_label.clone(),
            action_button: action_button.clone(),
            update_button: update_button.clone(),
            cancel_button: cancel_button.clone(),
            spinner: spinner.clone(),
            cancel_token: cancel_token.clone(),
        };
        component_rows.borrow_mut().push(row_info);

        // Connect cancel button
        let token_for_cancel = cancel_token.clone();
        cancel_button.connect_clicked(move |_| {
            if let Some(token) = token_for_cancel.borrow().as_ref() {
                token.cancel();
            }
        });

        // Connect action button click (Install/Remove)
        let rows_clone = component_rows.clone();
        action_button.connect_clicked(move |button| {
            let is_currently_installed = button.label().is_some_and(|l| l == "Remove");
            Self::handle_action_click(component, is_currently_installed, false, &rows_clone);
        });

        // Connect update button click
        let rows_for_update = component_rows.clone();
        update_button.connect_clicked(move |_| {
            Self::handle_action_click(component, false, true, &rows_for_update);
        });

        row.add_suffix(&action_button);
        row
    }

    fn handle_action_click(
        component: &'static DownloadableComponent,
        is_uninstall: bool,
        is_update: bool,
        rows: &Rc<RefCell<Vec<ComponentRow>>>,
    ) {
        // Find our row info and update UI
        {
            let rows_ref = rows.borrow();
            if let Some(info) = rows_ref.iter().find(|r| r.component_id == component.id) {
                info.action_button.set_sensitive(false);
                info.update_button.set_sensitive(false);
                info.status_label.set_label("...");

                if !is_uninstall {
                    // Show spinner and cancel button for install/update
                    info.spinner.set_visible(true);
                    info.spinner.start();
                    info.cancel_button.set_visible(true);

                    // Create new cancellation token
                    let token = DownloadCancellation::new();
                    *info.cancel_token.borrow_mut() = Some(token);
                }
            }
        }

        let rows_for_callback = rows.clone();

        if is_uninstall {
            // Uninstall
            spawn_async(async move {
                let result = uninstall_component(component).await;
                glib::idle_add_local_once(move || {
                    Self::update_row_after_action(&rows_for_callback, component.id, result, false);
                });
            });
        } else if is_update {
            // Update
            let component_id = component.id;
            let cancel_token = {
                let rows_ref = rows.borrow();
                rows_ref
                    .iter()
                    .find(|r| r.component_id == component_id)
                    .and_then(|info| info.cancel_token.borrow().clone())
                    .unwrap_or_default()
            };

            spawn_async(async move {
                let result = update_component(component, None, cancel_token).await;
                glib::idle_add_local_once(move || {
                    Self::update_row_after_action(
                        &rows_for_callback,
                        component.id,
                        result.map(|_| ()),
                        true,
                    );
                });
            });
        } else {
            // Install
            let component_id = component.id;
            let cancel_token = {
                let rows_ref = rows.borrow();
                rows_ref
                    .iter()
                    .find(|r| r.component_id == component_id)
                    .and_then(|info| info.cancel_token.borrow().clone())
                    .unwrap_or_default()
            };

            spawn_async(async move {
                let result = install_component(component, None, cancel_token).await;
                glib::idle_add_local_once(move || {
                    Self::update_row_after_action(
                        &rows_for_callback,
                        component.id,
                        result.map(|_| ()),
                        true,
                    );
                });
            });
        }
    }

    fn update_row_after_action(
        rows: &Rc<RefCell<Vec<ComponentRow>>>,
        component_id: &str,
        result: Result<(), rustconn_core::cli_download::CliDownloadError>,
        was_install: bool,
    ) {
        let rows_ref = rows.borrow();
        let row_info = rows_ref.iter().find(|r| r.component_id == component_id);

        // Get component to check if downloadable
        let component = rustconn_core::cli_download::get_component(component_id);

        if let Some(info) = row_info {
            // Hide progress UI
            info.spinner.stop();
            info.spinner.set_visible(false);
            info.cancel_button.set_visible(false);
            info.action_button.set_sensitive(true);
            info.update_button.set_sensitive(true);

            // Clear cancel token
            *info.cancel_token.borrow_mut() = None;

            match result {
                Ok(()) => {
                    let is_now_installed = was_install;
                    info.status_label
                        .set_label(if is_now_installed { "Installed" } else { "" });
                    info.status_label.remove_css_class("error");

                    if is_now_installed {
                        info.status_label.add_css_class("success");
                        info.action_button.set_label("Remove");
                        info.action_button.remove_css_class("suggested-action");
                        info.action_button.add_css_class("destructive-action");
                        // Show update button if component is downloadable
                        let is_downloadable = component.is_some_and(|c| c.is_downloadable());
                        info.update_button.set_visible(is_downloadable);
                    } else {
                        info.status_label.remove_css_class("success");
                        info.action_button.set_label("Install");
                        info.action_button.remove_css_class("destructive-action");
                        info.action_button.add_css_class("suggested-action");
                        // Hide update button when not installed
                        info.update_button.set_visible(false);
                    }
                }
                Err(ref error) => {
                    // Log technical details
                    tracing::error!(?error, "Component {} action failed", component_id);

                    // Show user-friendly message
                    let user_msg = get_user_friendly_error(error);
                    info.status_label.set_label("Failed");
                    info.status_label.remove_css_class("success");
                    info.status_label.add_css_class("error");

                    // Show toast with user-friendly error
                    // Note: We can't access toast_overlay here, so we just update the label
                    info.status_label.set_tooltip_text(Some(&user_msg));
                }
            }
        }
    }

    /// Show the dialog
    pub fn present(&self) {
        self.window.present();
    }

    /// Show a toast message
    pub fn show_toast(&self, message: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(message));
    }

    /// Get toast overlay for external error display
    #[must_use]
    pub fn toast_overlay(&self) -> &adw::ToastOverlay {
        &self.toast_overlay
    }
}

/// Check if Flatpak components menu should be visible
#[must_use]
pub fn should_show_flatpak_components_menu() -> bool {
    is_flatpak()
}
