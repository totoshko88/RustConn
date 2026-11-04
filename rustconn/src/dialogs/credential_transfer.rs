//! Bulk credential transfer between two secret backends.
//!
//! Answers "I already keep my passwords in KeePassXC / the system keyring — how
//! do I get them into the portable file?", which until now had no answer for any
//! pair of backends except the machine-bound file to the portable one.
//!
//! ## Why this is driven by the connection list
//!
//! [`SecretBackend`] cannot enumerate: six of the eight backends offer no way to
//! ask what they hold. The set of credentials therefore comes from what RustConn
//! knows it stored — every connection and group set to use the vault, plus every
//! secret variable it owns — and the per-backend key shapes are regenerated for
//! each side, because a key copied verbatim between backends of different shape
//! produces an entry the resolver never looks for again. See
//! [`crate::vault_ops::plan_credential_transfer`].
//!
//! ## What it does not do
//!
//! It never removes anything from the source. For a shared vault the entries may
//! not be RustConn's to delete, and for the machine-bound file the originals are
//! this machine's fallback — the same reason the portable-file wizard keeps them.
//!
//! [`SecretBackend`]: rustconn_core::secret::SecretBackend

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Label, StringList, glib};
use libadwaita as adw;
use rustconn_core::config::{AppSettings, SecretBackendType};
use rustconn_core::models::{Connection, ConnectionGroup};

use crate::i18n::{i18n, i18n_f};
use crate::vault_ops::{
    CredentialTransferReport, plan_credential_transfer, run_credential_transfer,
};

/// How many failing entries to name before switching to a summary line.
///
/// An `AdwAlertDialog` body does not scroll, so past a dozen the useful message
/// is the count plus "check the log" — the same ceiling the portable-file
/// migration report uses.
const MAX_NAMED_FAILURES: usize = 12;

/// Everything the dialog needs, snapshotted when it opens.
///
/// The settings are the *saved* ones rather than whatever the Settings dialog
/// currently has in its widgets: the transfer talks to real stores, and it needs
/// the configuration those stores were set up with (the KeePass database path,
/// the 1Password token, the `pass` store directory) — not an unsaved edit.
///
/// The portable file's path is the documented exception, overridden from the live
/// Settings row by `SettingsDialog::connect_credential_transfer`, because it is
/// the field most likely to be mid-edit when the button is pressed. Taking the
/// saved value there would send the passwords to the default location while the
/// row above still described the path the user had just typed.
pub struct TransferContext {
    /// Saved application settings, source of every backend's configuration.
    pub settings: AppSettings,
    /// All connections, for the credentials keyed per connection.
    pub connections: Vec<Connection>,
    /// All groups, needed both for group credentials and to resolve the group
    /// path that the keyring and KDBX key shapes embed.
    pub groups: Vec<ConnectionGroup>,
}

/// Opens the credential transfer dialog.
pub fn show_transfer_dialog(parent: &impl IsA<gtk4::Widget>, context: TransferContext) {
    let choices = crate::dialogs::settings::secrets_tab::backend_choices();
    let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
    let model = StringList::new(&labels);

    let preferred = crate::dialogs::settings::secrets_tab::backend_to_index(
        context.settings.secrets.preferred_backend,
    );

    let group = adw::PreferencesGroup::builder()
        .title(i18n("Copy Stored Passwords"))
        .description(i18n(
            "Copies the passwords of every connection, group and secret variable that uses the password store. The originals are left where they are.",
        ))
        .build();

    // Two separate models: one `StringList` shared between two `AdwComboRow`s
    // would be fine, but each row also needs its own selection, and giving them
    // separate models keeps a future per-side filter (a backend that cannot be a
    // destination, say) from having to be undone first.
    let source_row = adw::ComboRow::builder()
        .title(i18n("From"))
        .subtitle(i18n("Where the passwords are now"))
        .model(&model)
        .selected(preferred)
        .build();
    let destination_row = adw::ComboRow::builder()
        .title(i18n("To"))
        .subtitle(i18n("Where they should also be"))
        .model(&StringList::new(&labels))
        .selected(preferred)
        .build();
    group.add(&source_row);
    group.add(&destination_row);

    let count_label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();
    let count_row = adw::ActionRow::builder()
        .title(i18n("Entries to copy"))
        .activatable(false)
        .build();
    count_row.add_suffix(&count_label);
    group.add(&count_row);

    // The portable store is the one backend that can be configured and still be
    // unopenable, because its key is a passphrase rather than something the
    // machine or a running agent holds. Asked for here only when the settings do
    // not already have it, so a user who chose "remember it" is not asked twice.
    let passphrase_row = adw::PasswordEntryRow::builder()
        .title(i18n("Portable file passphrase"))
        .visible(false)
        .build();
    group.add(&passphrase_row);

    let status_label = Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    let status_row = adw::ActionRow::builder().activatable(false).build();
    status_row.add_prefix(&status_label);
    // Same binding as the Secrets page: hiding only the label leaves a
    // full-height empty band in the boxed list.
    status_label
        .bind_property("visible", &status_row, "visible")
        .sync_create()
        .build();
    group.add(&status_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);

    let copy_button = gtk4::Button::with_label(&i18n("Copy"));
    copy_button.add_css_class("suggested-action");
    let cancel_button = gtk4::Button::with_label(&i18n("Cancel"));

    // A second button rather than relabelling Cancel. `connect_clicked` adds a
    // handler, it does not replace one, so a repurposed Cancel would end the run
    // *and* close the dialog on the same click — and restoring it would mean
    // tracking signal handler ids across the worker's lifetime. Two buttons with
    // one visible at a time says the same thing to the user and cannot get into
    // that state.
    let stop_button = gtk4::Button::with_label(&i18n("Stop"));
    stop_button.set_visible(false);

    let header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new(&i18n("Transfer Credentials"), ""))
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    header.pack_start(&cancel_button);
    header.pack_start(&stop_button);
    header.pack_end(&copy_button);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));

    let dialog = adw::Dialog::builder()
        .content_width(480)
        .content_height(460)
        .child(&toolbar)
        .build();

    let context = std::rc::Rc::new(context);

    // Recomputes the plan whenever either side changes. Pure in-memory work over
    // the connection and group lists — no store is touched until Copy.
    let refresh = {
        let context = std::rc::Rc::clone(&context);
        let source_row = source_row.clone();
        let destination_row = destination_row.clone();
        let count_label = count_label.clone();
        let copy_button = copy_button.clone();
        let status_label = status_label.clone();
        let passphrase_row = passphrase_row.clone();
        move || {
            let source =
                crate::dialogs::settings::secrets_tab::index_to_backend(source_row.selected());
            let destination =
                crate::dialogs::settings::secrets_tab::index_to_backend(destination_row.selected());

            let portable_involved = matches!(source, SecretBackendType::PortableEncryptedFile)
                || matches!(destination, SecretBackendType::PortableEncryptedFile);
            passphrase_row.set_visible(
                portable_involved && context.settings.secrets.portable_passphrase.is_none(),
            );

            if source == destination {
                count_label.set_label("—");
                copy_button.set_sensitive(false);
                status_label.set_label(&i18n("Choose two different password stores"));
                status_label.remove_css_class("error");
                status_label.set_visible(true);
                return;
            }

            let plan = plan_credential_transfer(
                &context.settings,
                &context.connections,
                &context.groups,
                source,
                destination,
            );
            count_label.set_label(&plan.items.len().to_string());

            // A collision is refused rather than resolved: two entries share one
            // destination key, so one would overwrite the other and the report
            // would count both as copied. There is no second key to send the
            // loser to.
            if !plan.collisions.is_empty() {
                copy_button.set_sensitive(false);
                let mut message = i18n(
                    "These entries would end up in the same place in the destination, so one would overwrite the other. Rename one of them, or choose a destination that keeps groups apart.",
                );
                for labels in &plan.collisions {
                    message.push_str("\n• ");
                    message.push_str(&labels.join(" / "));
                }
                update_status(&status_label, &message, true);
                return;
            }

            copy_button.set_sensitive(!plan.items.is_empty());
            if plan.items.is_empty() {
                update_status(
                    &status_label,
                    &i18n(
                        "No connection, group or variable is set to use the password store, so there is nothing to copy.",
                    ),
                    false,
                );
            } else {
                status_label.set_visible(false);
            }
        }
    };
    refresh();
    {
        let refresh = refresh.clone();
        source_row.connect_selected_notify(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        destination_row.connect_selected_notify(move |_| refresh());
    }

    {
        let dialog = dialog.clone();
        cancel_button.connect_clicked(move |_| {
            dialog.close();
        });
    }

    // The stop flag outlives any one run so this handler can be attached once
    // rather than per copy; each run clears it before starting.
    let stop_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop_requested = std::sync::Arc::clone(&stop_requested);
        let status_label = status_label.clone();
        stop_button.connect_clicked(move |button| {
            stop_requested.store(true, std::sync::atomic::Ordering::Relaxed);
            button.set_sensitive(false);
            // The entry in flight still finishes — a credential is one backend
            // call and abandoning it mid-write would leave the destination in a
            // state the report could not describe. Saying so is the difference
            // between "it ignored me" and "it is stopping".
            update_status(
                &status_label,
                &i18n("Stopping after the entry in progress…"),
                false,
            );
        });
    }

    {
        let context = std::rc::Rc::clone(&context);
        let source_row = source_row.clone();
        let destination_row = destination_row.clone();
        let passphrase_row = passphrase_row.clone();
        let status_label = status_label.clone();
        let dialog_clone = dialog.clone();
        let cancel_for_copy = cancel_button.clone();
        let stop_for_copy = stop_button.clone();
        let stop_requested_for_copy = std::sync::Arc::clone(&stop_requested);
        copy_button.connect_clicked(move |button| {
            let source =
                crate::dialogs::settings::secrets_tab::index_to_backend(source_row.selected());
            let destination =
                crate::dialogs::settings::secrets_tab::index_to_backend(destination_row.selected());

            // The transfer runs against its own copy of the settings so a
            // passphrase typed here unlocks the portable store for this operation
            // without being written to the configuration behind the user's back.
            let mut settings = context.settings.clone();
            if passphrase_row.is_visible() {
                let typed = passphrase_row.text();
                if typed.is_empty() {
                    update_status(
                        &status_label,
                        &i18n("Enter the passphrase that protects the portable file"),
                        true,
                    );
                    passphrase_row.grab_focus();
                    return;
                }
                settings.secrets.portable_passphrase =
                    Some(secrecy::SecretString::from(typed.to_string()));
            }

            let plan = plan_credential_transfer(
                &settings,
                &context.connections,
                &context.groups,
                source,
                destination,
            );
            if !plan.collisions.is_empty() {
                // `refresh` already disabled the button and named them; this only
                // guards the path where the plan changed underneath.
                return;
            }

            confirm_and_run(
                button,
                &dialog_clone,
                &status_label,
                Run {
                    settings,
                    source,
                    destination,
                    items: plan.items,
                    lock: vec![
                        source_row.clone().upcast(),
                        destination_row.clone().upcast(),
                    ],
                    cancel: cancel_for_copy.clone(),
                    stop: stop_for_copy.clone(),
                    stop_requested: std::sync::Arc::clone(&stop_requested_for_copy),
                },
            );
        });
    }

    dialog.present(Some(parent));
}

/// The progress line: entries finished out of entries planned.
///
/// A count rather than a fraction of a `GtkProgressBar`: a CLI-backed vault takes
/// a second or more per entry, so what the user wants to know is whether it is
/// still moving and how much is left, which two numbers answer and a bar of
/// unknown scale does not.
fn copying_message(finished: usize, planned: usize) -> String {
    i18n_f(
        "Copying… {} of {}",
        &[&finished.to_string(), &planned.to_string()],
    )
}

/// Sets the inline status message.
fn update_status(label: &Label, message: &str, is_error: bool) {
    label.set_label(message);
    if is_error {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
    label.set_visible(true);
}

/// A confirmed transfer, ready to run.
struct Run {
    /// Settings the transfer talks to the stores with, including any passphrase
    /// typed in the dialog.
    settings: AppSettings,
    /// Where the credentials are read from.
    source: SecretBackendType,
    /// Where they are written to.
    destination: SecretBackendType,
    /// The entries to copy.
    items: Vec<crate::vault_ops::CredentialTransferItem>,
    /// Widgets to desensitise while the copy runs.
    ///
    /// The two selectors: their change handler re-arms Copy, so without this a
    /// second transfer could start over the first — two read-modify-write passes
    /// over the same file, which is the lost update the store's own mutex exists
    /// to prevent and cannot, across two instances.
    lock: Vec<gtk4::Widget>,
    /// Cancel, hidden for the duration and shown again afterwards.
    cancel: gtk4::Button,
    /// Stop, shown only while the copy runs.
    stop: gtk4::Button,
    /// Set by Stop, read by the worker before each entry.
    ///
    /// Owned by the dialog rather than created per run, so Stop's handler is
    /// attached once; each run clears it before starting.
    stop_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Asks for confirmation, then performs the transfer off the main thread.
///
/// Confirmation is not ceremony: this writes every stored password into a second
/// store, which for a vault backend means creating entries in something the user
/// shares with other tools. The default response is Cancel for the same reason.
fn confirm_and_run(anchor: &gtk4::Button, dialog: &adw::Dialog, status_label: &Label, run: Run) {
    let choices = crate::dialogs::settings::secrets_tab::backend_choices();
    let name_of = |backend: SecretBackendType| -> String {
        let index = crate::dialogs::settings::secrets_tab::backend_to_index(backend) as usize;
        choices
            .get(index)
            .map_or_else(String::new, |choice| choice.label.clone())
    };

    // The resolved destination path, not just the backend's name, when the
    // destination is a file backend. For the portable file that path comes from
    // the Settings row as it stands right now, unsaved edits included, so naming
    // it here is the only place the user can see which file is about to be
    // written before agreeing to it.
    let mut body = i18n_f(
        "{} entries will be read from {} and written to {}. Nothing is removed from {}.",
        &[
            &run.items.len().to_string(),
            &name_of(run.source),
            &name_of(run.destination),
            &name_of(run.source),
        ],
    );
    if matches!(
        run.destination,
        SecretBackendType::PortableEncryptedFile | SecretBackendType::EncryptedFile
    ) {
        let path = if matches!(run.destination, SecretBackendType::PortableEncryptedFile) {
            rustconn_core::secret::resolve_portable_store_path(
                run.settings.secrets.portable_file_path.as_deref(),
            )
        } else {
            rustconn_core::secret::default_encrypted_store_path()
        };
        body.push_str("\n\n");
        body.push_str(&i18n_f(
            "Destination file: {}",
            &[&path.display().to_string()],
        ));
    }

    let confirm = adw::AlertDialog::new(Some(&i18n("Copy stored passwords?")), Some(&body));
    confirm.add_response("cancel", &i18n("Cancel"));
    confirm.add_response("copy", &i18n("Copy"));
    confirm.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    confirm.set_default_response(Some("cancel"));
    confirm.set_close_response("cancel");

    let anchor = anchor.clone();
    let presented_on = anchor.clone();
    let dialog = dialog.clone();
    let status_label = status_label.clone();
    let run = std::rc::Rc::new(run);
    confirm.connect_response(None, move |_, response| {
        if response != "copy" {
            return;
        }

        let planned = run.items.len();
        update_status(&status_label, &copying_message(0, planned), false);
        anchor.set_sensitive(false);
        for widget in &run.lock {
            widget.set_sensitive(false);
        }
        // Cancel out, Stop in. Cancel closes the dialog, which is not what the
        // user wants mid-run, and leaving it live would abandon the worker with
        // nowhere to report.
        run.cancel.set_visible(false);
        run.stop.set_visible(true);
        run.stop.set_sensitive(true);
        // Fresh for each run: the flag lives as long as the dialog so its handler
        // can be attached once, so it has to be cleared here rather than created.
        run.stop_requested
            .store(false, std::sync::atomic::Ordering::Relaxed);

        // Unbounded: the worker must never block on a busy main loop.
        let (progress_tx, progress_rx) = async_channel::unbounded::<usize>();
        {
            let status_label = status_label.clone();
            let stop_requested = std::sync::Arc::clone(&run.stop_requested);
            glib::spawn_future_local(async move {
                // Ends when the worker drops the sender, so this future does not
                // outlive the run it reports on.
                while let Ok(finished) = progress_rx.recv().await {
                    // A stop request has its own message. Overwriting it with the
                    // count of the entry that was already in flight would read as
                    // the request having been ignored.
                    if stop_requested.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    update_status(&status_label, &copying_message(finished, planned), false);
                }
            });
        }

        let settings = run.settings.clone();
        let items = run.items.clone();
        let (source, destination) = (run.source, run.destination);
        let cancel_for_worker = std::sync::Arc::clone(&run.stop_requested);
        let run = std::rc::Rc::clone(&run);
        let anchor = anchor.clone();
        let dialog = dialog.clone();
        let status_label = status_label.clone();
        glib::spawn_future_local(async move {
            let outcome = gtk4::gio::spawn_blocking(move || {
                // Verify a portable side before the loop rather than discovering
                // it N times: a wrong passphrase would otherwise produce one
                // identical failure per entry and bury the actual cause.
                if let Some(error) = verify_portable_side(&settings, source, destination) {
                    return Err(error);
                }
                let control = crate::vault_ops::TransferControl {
                    progress: progress_tx,
                    cancel: cancel_for_worker,
                };
                run_credential_transfer(&settings, source, destination, &items, &control)
            })
            .await;

            // Passwords may now exist where the connect path recorded that none
            // did, so drop every one of those records (#307). Unconditionally,
            // including on error and on panic: a run that failed part way through
            // still copied the entries it got to, which is what the panic message
            // below tells the user, and clearing too much only costs a lookup.
            // `forget_all` rather than per connection because the transfer works
            // on vault keys, and mapping those back to connection ids would be a
            // second derivation of something the plan already discarded.
            crate::vault_miss_cache::forget_all();

            anchor.set_sensitive(true);
            for widget in &run.lock {
                widget.set_sensitive(true);
            }
            run.stop.set_visible(false);
            run.cancel.set_visible(true);
            match outcome {
                Ok(Ok(report)) => {
                    status_label.set_visible(false);
                    show_report(&dialog, &report, planned);
                }
                Ok(Err(message)) => update_status(&status_label, &message, true),
                Err(_panic) => update_status(
                    &status_label,
                    &i18n("The copy did not finish. Some entries may already have been copied; running it again completes it."),
                    true,
                ),
            }
        });
    });

    confirm.present(Some(&presented_on));
}

/// Checks the portable store's passphrase when it is one of the two sides.
///
/// Returns the message to show, or `None` when there is nothing to check or the
/// check passed.
fn verify_portable_side(
    settings: &AppSettings,
    source: SecretBackendType,
    destination: SecretBackendType,
) -> Option<String> {
    if !matches!(source, SecretBackendType::PortableEncryptedFile)
        && !matches!(destination, SecretBackendType::PortableEncryptedFile)
    {
        return None;
    }
    let Some(passphrase) = settings.secrets.portable_passphrase.as_ref() else {
        return Some(i18n("Enter the passphrase that protects the portable file"));
    };
    let path = rustconn_core::secret::resolve_portable_store_path(
        settings.secrets.portable_file_path.as_deref(),
    );
    // A missing file is accepted by `verify_portable_passphrase` on purpose:
    // there is nothing to check a passphrase against, and for a *destination* the
    // first write creates it under whatever was typed. As a *source* that same
    // acceptance is the wrong answer. Every read would return a clean miss, and
    // the report would say "had no stored password yet" for every entry — which
    // describes an empty store, not a path that points at nothing. The two
    // situations need the same fix from the user and no way to tell them apart.
    if matches!(source, SecretBackendType::PortableEncryptedFile) && !path.exists() {
        return Some(i18n_f(
            "There is no portable file at {}. Check the path in Settings, or wait for your cloud client to deliver it.",
            &[&path.display().to_string()],
        ));
    }
    match rustconn_core::secret::verify_portable_passphrase(&path, passphrase) {
        Ok(()) => None,
        Err(rustconn_core::error::SecretError::IncorrectPassphrase) => Some(i18n(
            "That passphrase does not open the portable file. Enter the passphrase the file was created with.",
        )),
        Err(e) => Some(i18n_f(
            "Cannot open the portable file: {}",
            &[&e.to_string()],
        )),
    }
}

/// Reports the outcome.
///
/// A dialog rather than a toast even on success: this is a bulk write the user
/// asked for, the counts are the only evidence it did what they wanted, and a
/// five-second toast that says "3 could not be read" takes away the only record
/// of which three.
fn show_report(dialog: &adw::Dialog, report: &CredentialTransferReport, planned: usize) {
    let mut body = i18n_f("Copied: {}", &[&report.transferred.to_string()]);
    if report.missing > 0 {
        body.push('\n');
        body.push_str(&i18n_f(
            "Had no stored password yet: {}",
            &[&report.missing.to_string()],
        ));
    }

    // Said before the counts are interpreted, because it changes what they mean:
    // seven copied out of forty planned is the expected result of stopping and an
    // alarming one otherwise, and no other line distinguishes the two.
    if report.cancelled {
        // Failures count as looked at. Leaving them out reported them a second
        // time, as entries that were never reached.
        let untouched =
            planned.saturating_sub(report.transferred + report.missing + report.failures.len());
        body.push_str("\n\n");
        body.push_str(&i18n_f(
            "You stopped the copy, so {} entries were not looked at. Everything already copied stays where it is, and running the copy again finishes the rest.",
            &[&untouched.to_string()],
        ));
        let alert = adw::AlertDialog::new(Some(&i18n("Copy Stopped")), Some(&body));
        alert.add_response("close", &i18n("Close"));
        alert.set_default_response(Some("close"));
        alert.set_close_response("close");
        alert.present(Some(dialog));
        return;
    }

    if report.is_complete() {
        let alert = adw::AlertDialog::new(Some(&i18n("Passwords Copied")), Some(&body));
        alert.add_response("close", &i18n("Close"));
        alert.set_default_response(Some("close"));
        alert.set_close_response("close");
        alert.present(Some(dialog));
        return;
    }

    if !report.failures.is_empty() {
        body.push('\n');
        body.push_str(&i18n_f(
            "Could not be copied: {}",
            &[&report.failures.len().to_string()],
        ));
    }
    body.push_str("\n\n");

    let mut named: Vec<&str> = report
        .failures
        .iter()
        .map(|(label, _)| label.as_str())
        .take(MAX_NAMED_FAILURES)
        .collect();
    named.sort_unstable();
    for label in named {
        body.push_str("\n• ");
        body.push_str(label);
    }
    if report.failures.len() > MAX_NAMED_FAILURES {
        body.push_str("\n\n");
        body.push_str(&i18n_f(
            "…and {} more. The full list is in the application log.",
            &[&(report.failures.len() - MAX_NAMED_FAILURES).to_string()],
        ));
    }

    // Named separately from the failures: the password did arrive, so calling
    // these failures would be wrong, and calling them successes would hide that
    // the entry lost something. A KeePass entry has no field for a domain,
    // which libsecret and both file backends do store.
    if !report.incomplete.is_empty() {
        body.push_str("\n\n");
        body.push_str(&i18n(
            "These arrived, but the destination has nowhere to keep part of what they held:",
        ));
        let mut partial: Vec<&(String, String)> =
            report.incomplete.iter().take(MAX_NAMED_FAILURES).collect();
        partial.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (label, fields) in partial {
            body.push_str("\n• ");
            body.push_str(label);
            body.push_str(" — ");
            body.push_str(fields);
        }
    }

    // Labels only, which is what makes the capped list above recoverable from the
    // log. The reasons are already logged per entry by `run_credential_transfer`,
    // where each one is attached to the entry it belongs to rather than to a
    // summary line.
    tracing::warn!(
        failures = ?report.failures.iter().map(|(label, _)| label).collect::<Vec<_>>(),
        incomplete = ?report.incomplete,
        transferred = report.transferred,
        "credential transfer did not copy every entry cleanly"
    );

    let alert = adw::AlertDialog::new(Some(&i18n("Some Passwords Were Not Copied")), Some(&body));
    alert.add_response("close", &i18n("Close"));
    alert.set_default_response(Some("close"));
    alert.set_close_response("close");
    alert.present(Some(dialog));
}
