//! User-friendly error display utilities
//!
//! This module provides utilities for converting technical errors into
//! user-friendly messages and displaying them appropriately.

use crate::error::AppStateError;
use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{AlertDialog, Button, Expander, Label, Orientation, ScrolledWindow};
use libadwaita as adw;

/// Converts an `AppStateError` to a user-friendly message
///
/// Technical details are preserved but the primary message is written
/// in plain language that non-technical users can understand.
#[must_use]
pub fn user_friendly_message(error: &AppStateError) -> String {
    match error {
        AppStateError::InitializationFailed { component, .. } => {
            format!("Failed to start {component}. Please try restarting the application.")
        }
        AppStateError::ConnectionNotFound(_) => {
            "The connection could not be found. It may have been deleted or moved.".to_string()
        }
        AppStateError::GroupNotFound(_) => {
            "The group could not be found. It may have been deleted.".to_string()
        }
        AppStateError::SessionNotFound(_) => {
            "The session could not be found. It may have ended.".to_string()
        }
        AppStateError::SnippetNotFound(_) => {
            "The snippet could not be found. It may have been deleted.".to_string()
        }
        AppStateError::DocumentNotFound(_) => {
            "The document could not be found. It may have been closed or deleted.".to_string()
        }
        AppStateError::ClusterNotFound(_) => {
            "The cluster could not be found. It may have been deleted.".to_string()
        }
        AppStateError::TemplateNotFound(_) => {
            "The template could not be found. It may have been deleted.".to_string()
        }
        AppStateError::DuplicateName { entity_type, name } => {
            format!(
                "A {entity_type} named \"{name}\" already exists. Please choose a different name."
            )
        }
        AppStateError::CreateFailed { entity_type, .. } => {
            format!("Failed to create the {entity_type}. Please check your input and try again.")
        }
        AppStateError::UpdateFailed { entity_type, .. } => {
            format!("Failed to update the {entity_type}. Please try again.")
        }
        AppStateError::DeleteFailed { entity_type, .. } => {
            format!("Failed to delete the {entity_type}. It may be in use.")
        }
        AppStateError::CredentialError(_) => {
            "Could not access credentials. Please check your secret storage settings.".to_string()
        }
        AppStateError::ConfigError(_) => {
            "Configuration error. Please check your settings and try again.".to_string()
        }
        AppStateError::ImportError(_) => {
            "Import failed. Please check the source file format and try again.".to_string()
        }
        AppStateError::ExportError(_) => {
            "Export failed. Please check the destination path and try again.".to_string()
        }
        AppStateError::DocumentIoError { path, .. } => {
            format!(
                "Could not access the file at {}. Please check the path and permissions.",
                path.display()
            )
        }
        AppStateError::SessionError(_) => {
            "Session operation failed. Please try reconnecting.".to_string()
        }
        AppStateError::ClipboardEmpty => {
            "Nothing to paste. Please copy a connection first.".to_string()
        }
        AppStateError::RuntimeError(_) => {
            "An internal error occurred. Please try again or restart the application.".to_string()
        }
    }
}

/// Returns the technical details of an error for debugging
#[must_use]
pub fn technical_details(error: &AppStateError) -> String {
    error.to_string()
}

/// Shows an error dialog with user-friendly message and optional technical details
///
/// The dialog shows a user-friendly message by default, with an expandable
/// section for technical details that can help with debugging.
pub fn show_error_dialog(
    window: Option<&impl IsA<gtk4::Window>>,
    title: &str,
    error: &AppStateError,
) {
    let user_message = user_friendly_message(error);
    let tech_details = technical_details(error);

    // If the messages are the same, just show a simple dialog
    if user_message == tech_details {
        let alert = AlertDialog::builder()
            .message(title)
            .detail(&user_message)
            .modal(true)
            .build();
        alert.show(window);
        return;
    }

    // Create a custom dialog with expandable details
    let dialog = adw::Window::builder()
        .title(title)
        .modal(true)
        .default_width(450)
        .resizable(false)
        .build();

    if let Some(parent) = window {
        dialog.set_transient_for(Some(parent));
    }

    // Header bar with close button
    let header = adw::HeaderBar::new();
    let close_btn = Button::builder().label("Close").build();
    header.pack_end(&close_btn);

    // Content
    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    // Error icon and message
    let message_box = gtk4::Box::new(Orientation::Horizontal, 12);

    let icon = gtk4::Image::from_icon_name("dialog-error-symbolic");
    icon.set_pixel_size(48);
    icon.set_valign(gtk4::Align::Start);
    message_box.append(&icon);

    let message_label = Label::builder()
        .label(&user_message)
        .wrap(true)
        .max_width_chars(50)
        .halign(gtk4::Align::Start)
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    message_box.append(&message_label);

    content.append(&message_box);

    // Expandable technical details
    let expander = Expander::builder()
        .label("Technical Details")
        .margin_top(8)
        .build();

    let details_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .max_content_height(150)
        .build();

    let details_label = Label::builder()
        .label(&tech_details)
        .wrap(true)
        .selectable(true)
        .halign(gtk4::Align::Start)
        .css_classes(["monospace", "dim-label"])
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();

    details_scroll.set_child(Some(&details_label));
    expander.set_child(Some(&details_scroll));
    content.append(&expander);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    dialog.set_content(Some(&toolbar_view));

    // Connect close button
    let dialog_clone = dialog.clone();
    close_btn.connect_clicked(move |_| {
        dialog_clone.close();
    });

    dialog.present();
}

/// Shows a simple error dialog with just a message (no technical details)
pub fn show_simple_error(window: Option<&impl IsA<gtk4::Window>>, title: &str, message: &str) {
    let alert = AlertDialog::builder()
        .message(title)
        .detail(message)
        .modal(true)
        .build();
    alert.show(window);
}

/// Shows a warning dialog
pub fn show_warning(window: Option<&impl IsA<gtk4::Window>>, title: &str, message: &str) {
    let alert = AlertDialog::builder()
        .message(title)
        .detail(message)
        .modal(true)
        .build();
    alert.show(window);
}

/// Shows an info dialog
pub fn show_info(window: Option<&impl IsA<gtk4::Window>>, title: &str, message: &str) {
    let alert = AlertDialog::builder()
        .message(title)
        .detail(message)
        .modal(true)
        .build();
    alert.show(window);
}

/// Formats an error message for logging
///
/// Includes both user-friendly and technical details in a format
/// suitable for log files.
#[must_use]
pub fn format_for_log(error: &AppStateError) -> String {
    format!(
        "Error: {} | Details: {}",
        user_friendly_message(error),
        technical_details(error)
    )
}
