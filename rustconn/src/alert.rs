//! Alert dialog helpers using `adw::AlertDialog`
//!
//! This module provides helper functions for showing alert dialogs
//! using the modern libadwaita `AlertDialog` API.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::i18n::i18n;

/// Shows a simple info/error alert with OK button
pub fn show_alert(parent: &impl IsA<gtk4::Widget>, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("ok", &i18n("OK"));
    dialog.set_default_response(Some("ok"));
    dialog.present(Some(parent));
}

/// Shows a confirmation dialog with Cancel/Confirm buttons
/// Calls the callback with true if confirmed, false if cancelled
pub fn show_confirm<F>(
    parent: &impl IsA<gtk4::Widget>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    destructive: bool,
    callback: F,
) where
    F: Fn(bool) + 'static,
{
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("confirm", confirm_label);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    if destructive {
        dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    } else {
        dialog.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);
    }

    dialog.connect_response(None, move |_, response| {
        callback(response == "confirm");
    });

    dialog.present(Some(parent));
}

/// Shows an error alert
pub fn show_error(parent: &impl IsA<gtk4::Widget>, heading: &str, body: &str) {
    show_alert(parent, heading, body);
}

/// Shows a success alert
pub fn show_success(parent: &impl IsA<gtk4::Widget>, heading: &str, body: &str) {
    show_alert(parent, heading, body);
}

/// Shows a validation error alert
pub fn show_validation_error(parent: &impl IsA<gtk4::Widget>, body: &str) {
    show_alert(parent, &i18n("Validation Error"), body);
}

/// Response type for save changes dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveChangesResponse {
    /// User chose not to save
    DontSave,
    /// User cancelled the dialog
    Cancel,
    /// User chose to save
    Save,
}

/// Shows a "save changes" dialog with Don't Save/Cancel/Save buttons
/// Calls the callback with the user's choice
pub fn show_save_changes<F>(parent: &impl IsA<gtk4::Widget>, heading: &str, body: &str, callback: F)
where
    F: Fn(SaveChangesResponse) + 'static,
{
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("dont_save", &i18n("Don't Save"));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("save", &i18n("Save"));
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("dont_save", adw::ResponseAppearance::Destructive);

    dialog.connect_response(None, move |_, response| {
        let result = match response {
            "dont_save" => SaveChangesResponse::DontSave,
            "save" => SaveChangesResponse::Save,
            _ => SaveChangesResponse::Cancel,
        };
        callback(result);
    });

    dialog.present(Some(parent));
}
