//! Variable setup dialog for Cloud Sync credential resolution.
//!
//! Shown when a connection references a variable that has no value on
//! this device. The user enters the value and selects a secret backend.
//!
//! GNOME HIG: `AdwAlertDialog` with `extra_child` widget.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::i18n::{i18n, i18n_f};

/// Response from the variable setup dialog.
#[derive(Debug, Clone)]
pub enum VariableSetupResponse {
    /// User cancelled the dialog.
    Cancel,
    /// User entered a value and chose a backend.
    Save {
        /// The secret value entered by the user.
        value: secrecy::SecretString,
        /// Index of the selected backend in the combo row.
        backend_index: u32,
    },
}

/// Shows the "Variable Not Configured" dialog.
///
/// Presents an `AdwAlertDialog` with:
/// - heading: "Variable Not Configured"
/// - body: "Connection '%s' requires variable '%s'"
/// - extra child: `AdwPreferencesGroup` with `AdwPasswordEntryRow` + `AdwComboRow`
/// - responses: Cancel / Save & Connect
///
/// # Arguments
/// * `parent` — parent widget for the dialog
/// * `connection_name` — display name of the connection
/// * `variable_name` — name of the missing variable
/// * `description` — optional human-readable description
/// * `backend_names` — list of available backend display names for the combo row
/// * `callback` — called with the user's response
pub fn show_variable_setup_dialog<F>(
    parent: &impl IsA<gtk4::Widget>,
    connection_name: &str,
    variable_name: &str,
    description: Option<&str>,
    backend_names: &[&str],
    callback: F,
) where
    F: Fn(VariableSetupResponse) + 'static,
{
    let heading = i18n("Variable Not Configured");

    let body = if let Some(desc) = description {
        i18n_f(
            "Connection ‘{}’ requires variable ‘{}’\n({})",
            &[connection_name, variable_name, desc],
        )
    } else {
        i18n_f(
            "Connection ‘{}’ requires variable ‘{}’",
            &[connection_name, variable_name],
        )
    };

    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));

    // Build extra child: AdwPreferencesGroup with value entry + backend combo
    let prefs_group = adw::PreferencesGroup::new();

    let value_row = adw::PasswordEntryRow::new();
    value_row.set_title(&i18n("Value"));
    prefs_group.add(&value_row);

    let backend_list = gtk4::StringList::new(backend_names);
    let backend_row = adw::ComboRow::new();
    backend_row.set_title(&i18n("Store in"));
    backend_row.set_model(Some(&backend_list));
    prefs_group.add(&backend_row);

    dialog.set_extra_child(Some(&prefs_group));

    // Responses
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("save", &i18n("Save & Connect"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    // Capture widget references for the response handler
    let value_row_ref = value_row.clone();
    let backend_row_ref = backend_row.clone();

    dialog.connect_response(None, move |_, response| {
        if response == "save" {
            // Capture directly into SecretString so plaintext does not live
            // as a plain String in this closure (M-PUBLIC-DEBUG).
            let value = secrecy::SecretString::from(value_row_ref.text().to_string());
            let backend_index = backend_row_ref.selected();
            callback(VariableSetupResponse::Save {
                value,
                backend_index,
            });
        } else {
            callback(VariableSetupResponse::Cancel);
        }
    });

    dialog.present(Some(parent));
}
