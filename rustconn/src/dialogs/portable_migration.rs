//! Migration wizard for the portable encrypted file backend.
//!
//! Offers to bulk-transfer credentials from the machine-bound encrypted file
//! into the portable, passphrase-protected one (issue #293), so switching
//! backends does not leave every stored password behind.
//!
//! Only one direction is offered. Going back is not a wizard: the machine-bound
//! store is always readable on this machine, so "return these to the local file"
//! is the fallback chain's job, not a migration the user has to run.
//!
//! GNOME HIG: `AdwAlertDialog` for the decision, `AdwToast` for the outcome.

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::i18n::{i18n, i18n_f};

/// What the user chose in the migration wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationResponse {
    /// Copy every credential into the portable file.
    Transfer,
    /// Leave them where they are.
    Skip,
}

/// Asks whether to copy existing credentials into the portable file.
///
/// `entry_count` is shown so the user knows the size of what is about to move;
/// the caller has already established it is non-zero.
///
/// The default response is Cancel, not Transfer: this writes every stored
/// password into a new file, usually one inside a cloud-synced folder, so it is
/// not the action a stray Return should trigger.
///
/// # Arguments
/// * `parent` — parent widget for the dialog
/// * `entry_count` — number of credentials in the machine-bound store
/// * `callback` — called with the user's choice
pub fn show_migration_wizard<F>(parent: &impl IsA<gtk4::Widget>, entry_count: usize, callback: F)
where
    F: Fn(MigrationResponse) + 'static,
{
    let heading = i18n("Copy credentials to the portable file?");
    // Written on one line on purpose. `xgettext --language=C` does not honour
    // Rust's `\<newline>` continuation, so a wrapped literal reaches the POT with
    // the source indentation baked in while the runtime string has it collapsed —
    // the two never match and the translation is silently never used.
    let body = i18n_f(
        "{} stored passwords are currently protected by a key that only works on this computer. Copying them into the portable file re-encrypts them with your passphrase, so they can be opened on your other devices.\n\nThe originals are kept, so nothing is lost if you change your mind.",
        &[&entry_count.to_string()],
    );

    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("transfer", &i18n("Copy Credentials"));
    dialog.set_response_appearance("transfer", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let callback = std::rc::Rc::new(std::cell::RefCell::new(Some(callback)));
    dialog.connect_response(None, move |_dialog, response| {
        if let Some(cb) = callback.borrow_mut().take() {
            cb(if response == "transfer" {
                MigrationResponse::Transfer
            } else {
                MigrationResponse::Skip
            });
        }
    });

    dialog.present(Some(parent));
}

/// Copies credentials into the portable file on a worker thread.
///
/// Each entry costs an Argon2 pass on the machine-key side, so a store with a
/// few dozen credentials takes seconds. Running that on the GTK main thread
/// freezes every open session for the duration, which is why the work goes to
/// `gio::spawn_blocking` and the result comes back as a toast.
///
/// # Arguments
/// * `source_path` — the machine-bound `credentials.enc`
/// * `dest_path` — the portable store
/// * `passphrase` — protects the portable store
/// * `prefs_dialog` — where the outcome is reported
/// * `on_finished` — run once the copy has ended, however it ended, so the
///   caller can re-read what is now in the file. It runs after a failure too: a
///   partial copy still changed the destination, and a row showing the count
///   from before would be wrong in the case that most needs to be accurate.
pub fn run_migration<F>(
    source_path: std::path::PathBuf,
    dest_path: std::path::PathBuf,
    passphrase: secrecy::SecretString,
    prefs_dialog: adw::PreferencesDialog,
    on_finished: F,
) where
    F: Fn() + 'static,
{
    glib::spawn_future_local(async move {
        let result = gtk4::gio::spawn_blocking(move || {
            rustconn_core::secret::migration::migrate_encrypted_to_portable(
                &source_path,
                &dest_path,
                &passphrase,
                // Never delete the source. The machine-bound copy is this
                // machine's safety net if the passphrase is lost, and the
                // fallback chain still reads it.
                false,
            )
        })
        .await;

        // A toast only for the case where nothing needs acting on. Anything else
        // is a user-triggered failure that leaves a half-finished transfer, which
        // the project's HIG rule puts in a dialog: a 5-second toast saying "3
        // could not be read" tells the user something is wrong and then takes
        // away the only copy of *which* three.
        match result {
            Ok(Ok(ref res)) if res.is_complete() => {
                let toast = crate::toast::plain_toast(&i18n_f(
                    "Copied {} credentials to the portable file",
                    &[&res.migrated.to_string()],
                ));
                toast.set_timeout(5);
                prefs_dialog.add_toast(toast);
            }
            Ok(Ok(ref res)) => {
                show_partial_failure(&prefs_dialog, res);
            }
            Ok(Err(ref e)) => {
                crate::alert::show_error(
                    &prefs_dialog,
                    &i18n("Could Not Copy Credentials"),
                    &i18n_f("The transfer did not start: {}", &[&e.to_string()]),
                );
            }
            // A panic in the worker carries no `Display`, so there is nothing
            // more specific to show than that it did not finish.
            Err(_panic) => {
                crate::alert::show_error(
                    &prefs_dialog,
                    &i18n("Could Not Copy Credentials"),
                    &i18n(
                        "The transfer did not finish. The portable file may hold only some of your credentials; run the copy again to complete it.",
                    ),
                );
            }
        }

        on_finished();
    });
}

/// Reports a transfer that copied some credentials and could not read others.
///
/// Names the entries. `MigrationResult` carries the failing keys precisely so
/// this can: the user's next step is to open those connections and re-enter the
/// password, and a count does not tell them which ones. The list is capped
/// because an `AdwAlertDialog` body does not scroll — past a dozen the useful
/// message is the count plus "check the log", which is what the remainder line
/// says.
fn show_partial_failure(
    prefs_dialog: &adw::PreferencesDialog,
    result: &rustconn_core::secret::migration::MigrationResult,
) {
    /// How many failing entries to name before switching to a summary line.
    const MAX_NAMED: usize = 12;

    let mut named: Vec<&str> = result
        .failures
        .iter()
        .map(|(key, _)| key.as_str())
        .take(MAX_NAMED)
        .collect();
    named.sort_unstable();

    let mut body = i18n_f(
        "Copied {} credentials. {} could not be read and were left in the machine-bound file:",
        &[
            &result.migrated.to_string(),
            &result.failures.len().to_string(),
        ],
    );
    for key in named {
        body.push_str("\n• ");
        body.push_str(key);
    }
    if result.failures.len() > MAX_NAMED {
        body.push_str("\n\n");
        body.push_str(&i18n_f(
            "…and {} more. The full list is in the application log.",
            &[&(result.failures.len() - MAX_NAMED).to_string()],
        ));
    }
    body.push_str("\n\n");
    body.push_str(&i18n(
        "These connections still work on this computer. To use them on another device, open each one and save its password again.",
    ));

    // The keys are connection identifiers, not secrets, so logging them is what
    // makes the capped list above recoverable.
    tracing::warn!(
        failures = ?result.failures.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        migrated = result.migrated,
        "Portable migration could not read every credential"
    );

    crate::alert::show_error(
        prefs_dialog,
        &i18n("Some Credentials Were Not Copied"),
        &body,
    );
}
