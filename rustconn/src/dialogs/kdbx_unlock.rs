//! On-demand KDBX unlock dialog for session-only master password.
//!
//! Shown when a connection requires credentials from a KeePass database but
//! the master password is not available in memory (i.e. "Save password = Don't
//! save" mode). The user enters the KDBX master password, which is verified
//! against the database and then kept only in memory for the remainder of the
//! RustConn process — never written to disk or keyring.
//!
//! GNOME HIG: `AdwAlertDialog` with `extra_child` widget.

use std::path::Path;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::i18n::{i18n, i18n_f};

/// Response from the KDBX unlock dialog.
#[derive(Debug)]
pub enum KdbxUnlockResponse {
    /// User cancelled the dialog — connection is not started.
    Cancel,
    /// User entered the correct KDBX master password.
    Unlocked {
        /// The verified master password (session-only, never persisted).
        password: secrecy::SecretString,
    },
}

/// Shows the "KeePass Database Locked" dialog.
///
/// Presents an `AdwAlertDialog` with:
/// - heading: "KeePass database is locked"
/// - body: filename of the KDBX database
/// - extra child: `AdwPreferencesGroup` with `AdwPasswordEntryRow`
/// - responses: Cancel / Unlock & Connect
///
/// The dialog verifies the password against the database before returning
/// `KdbxUnlockResponse::Unlocked`. Invalid passwords show an inline error
/// and keep the dialog open.
///
/// # Arguments
/// * `parent` — parent widget for the dialog
/// * `kdbx_path` — path to the KDBX database file (displayed for context)
/// * `key_file` — optional key file path for composite authentication
/// * `callback` — called with the user's response after verification
pub fn show_kdbx_unlock_dialog<F>(
    parent: &impl IsA<gtk4::Widget>,
    kdbx_path: &Path,
    key_file: Option<&Path>,
    callback: F,
) where
    F: Fn(KdbxUnlockResponse) + 'static,
{
    let heading = i18n("KeePass database is locked");

    let db_filename = kdbx_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("database.kdbx");

    let body = i18n_f(
        "Enter the master password to unlock '{}'.\nThe password will be kept in memory for this session only.",
        &[db_filename],
    );

    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));

    // Build extra child: password entry + status label
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

    let prefs_group = adw::PreferencesGroup::new();

    let password_row = adw::PasswordEntryRow::new();
    password_row.set_title(&i18n("Database password"));
    prefs_group.add(&password_row);

    content_box.append(&prefs_group);

    // Status label for verification errors (hidden initially)
    let status_label = gtk4::Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(["error"])
        .build();
    content_box.append(&status_label);

    dialog.set_extra_child(Some(&content_box));

    // Responses
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("unlock", &i18n("Unlock & Connect"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);

    // Disable "Unlock" until the user types something
    dialog.set_response_enabled("unlock", false);
    let dialog_enable = dialog.clone();
    password_row.connect_changed(move |row| {
        dialog_enable.set_response_enabled("unlock", !row.text().is_empty());
    });

    // Capture state for the response handler
    let kdbx_path_owned = kdbx_path.to_path_buf();
    let key_file_owned = key_file.map(Path::to_path_buf);
    let password_row_ref = password_row.clone();
    let status_label_ref = status_label.clone();
    let callback = std::rc::Rc::new(std::cell::RefCell::new(Some(callback)));

    dialog.connect_response(None, move |dialog, response| {
        if response == "unlock" {
            let password_text = password_row_ref.text().to_string();
            if password_text.is_empty() {
                return;
            }

            let password = secrecy::SecretString::from(password_text);
            let kdbx_path_verify = kdbx_path_owned.clone();
            let key_file_verify = key_file_owned.clone();
            let password_verify = password.clone();

            // Show verifying state
            status_label_ref.set_text(&i18n("Verifying…"));
            status_label_ref.remove_css_class("error");
            status_label_ref.add_css_class("dim-label");
            status_label_ref.set_visible(true);
            dialog.set_response_enabled("unlock", false);
            dialog.set_response_enabled("cancel", false);

            let status_async = status_label_ref.clone();
            let dialog_async = dialog.clone();
            let callback_clone = callback.clone();

            // Run verification in background thread (argon2 key derivation is CPU-heavy)
            glib::spawn_future_local(async move {
                let result = gtk4::gio::spawn_blocking(move || {
                    rustconn_core::secret::KeePassStatus::verify_kdbx_credentials(
                        &kdbx_path_verify,
                        Some(&password_verify),
                        key_file_verify.as_deref(),
                    )
                })
                .await;

                match result {
                    Ok(Ok(())) => {
                        // Verification succeeded — close dialog and notify caller
                        dialog_async.force_close();
                        if let Some(cb) = callback_clone.borrow_mut().take() {
                            cb(KdbxUnlockResponse::Unlocked { password });
                        }
                    }
                    Ok(Err(e)) => {
                        // Invalid password — show error and let the user retry
                        status_async.set_text(&e.to_string());
                        status_async.remove_css_class("dim-label");
                        status_async.add_css_class("error");
                        status_async.set_visible(true);
                        dialog_async.set_response_enabled("unlock", true);
                        dialog_async.set_response_enabled("cancel", true);
                    }
                    Err(_join_err) => {
                        status_async.set_text(&i18n("Verification failed"));
                        status_async.remove_css_class("dim-label");
                        status_async.add_css_class("error");
                        status_async.set_visible(true);
                        dialog_async.set_response_enabled("unlock", true);
                        dialog_async.set_response_enabled("cancel", true);
                    }
                }
            });
        } else {
            // Cancel
            if let Some(cb) = callback.borrow_mut().take() {
                cb(KdbxUnlockResponse::Cancel);
            }
        }
    });

    dialog.present(Some(parent));
}
