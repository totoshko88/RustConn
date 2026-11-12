//! Unlock dialog for an encrypted SSH key a connect needs.
//!
//! Shown when a connection passes an encrypted OpenSSH key that the SSH agent
//! does not hold. The passphrase the user enters is moved into a background
//! `ssh-add` (12 h lifetime, recorded as one of RustConn's own keys) and
//! dropped there when the add returns. It is never stored, never sent to a
//! vault and never logged; the entry is cleared as soon as it is read, and
//! again whenever the dialog closes.
//!
//! GNOME HIG: `AdwAlertDialog` with an `extra_child`, as `portable_unlock.rs`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use libadwaita as adw;
use rustconn_core::ssh_agent::{
    AfterWrongPassphrase, AgentError, SshAgentManager, after_wrong_passphrase,
};

use crate::i18n::{i18n, i18n_f};

/// How the unlock dialog ended. The connect goes on in every case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyUnlockOutcome {
    /// The key is in the agent now.
    Unlocked,
    /// The user closed the dialog; ssh asks for the passphrase as before.
    Cancelled,
    /// Too many wrong passphrases; the key was not added.
    GaveUp,
    /// The agent refused the key (an agent such as 1Password's).
    Refused,
    /// The add failed for another reason (untranslated detail).
    Failed(String),
}

/// Shows the "Unlock SSH Key" dialog for the key at `key_path`, adding it to
/// the agent at `socket_path`, and calls `on_done` once when it closes.
pub fn show_key_unlock_dialog(
    parent: &impl IsA<gtk4::Widget>,
    key_path: &Path,
    socket_path: &str,
    on_done: impl FnOnce(KeyUnlockOutcome) + 'static,
) {
    let body = i18n_f(
        "Enter the passphrase for “{}”. The key is added to the SSH agent for up to 12 hours and removed when you quit RustConn. The passphrase is not saved.",
        &[&key_path.display().to_string()],
    );
    let dialog = adw::AlertDialog::new(Some(&i18n("Unlock SSH Key")), Some(&body));

    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let prefs_group = adw::PreferencesGroup::new();
    let passphrase_row = adw::PasswordEntryRow::new();
    passphrase_row.set_title(&i18n("Passphrase"));
    prefs_group.add(&passphrase_row);
    content_box.append(&prefs_group);

    // Wrong-passphrase and progress text (hidden until needed).
    let status_label = gtk4::Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    content_box.append(&status_label);
    dialog.set_extra_child(Some(&content_box));

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("unlock", &i18n("Unlock"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
    // A wrong passphrase keeps the dialog up with an inline error, so every
    // exit is an explicit `force_close` (see `portable_unlock.rs`).
    dialog.set_can_close(false);

    dialog.set_response_enabled("unlock", false);
    let busy = Rc::new(Cell::new(false));
    // The row's handlers hold the dialog weakly: the dialog owns the row, so a
    // strong reference would keep both, and any text typed, alive.
    {
        let dialog = dialog.downgrade();
        let busy = busy.clone();
        passphrase_row.connect_changed(move |row| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.set_response_enabled("unlock", !busy.get() && !row.text().is_empty());
            }
        });
    }
    {
        let dialog = dialog.downgrade();
        passphrase_row.connect_entry_activated(move |row| {
            if let Some(dialog) = dialog.upgrade()
                && !row.text().is_empty()
                && dialog.is_response_enabled("unlock")
            {
                dialog.emit_by_name::<()>("response", &[&"unlock"]);
            }
        });
    }

    let on_done: Rc<RefCell<Option<Box<dyn FnOnce(KeyUnlockOutcome)>>>> =
        Rc::new(RefCell::new(Some(Box::new(on_done))));
    let finish = {
        let on_done = on_done.clone();
        let passphrase_row = passphrase_row.downgrade();
        move |dialog: &adw::AlertDialog, outcome: KeyUnlockOutcome| {
            // A cancelled or unfinished passphrase does not stay in the entry.
            if let Some(row) = passphrase_row.upgrade() {
                row.set_text("");
            }
            dialog.force_close();
            // Taken into a local, so no borrow is held while it runs.
            let callback = on_done.borrow_mut().take();
            if let Some(callback) = callback {
                callback(outcome);
            }
        }
    };

    let busy_on_close = busy.clone();
    let attempts = Rc::new(Cell::new(0u8));
    let key_path: PathBuf = key_path.to_path_buf();
    let socket_path = socket_path.to_owned();
    {
        let finish = finish.clone();
        dialog.connect_response(None, move |dialog, response| {
            if busy.get() {
                return;
            }
            if response != "unlock" {
                tracing::debug!(?key_path, "SSH key unlock cancelled");
                finish(dialog, KeyUnlockOutcome::Cancelled);
                return;
            }
            let text = passphrase_row.text();
            if text.is_empty() {
                return;
            }
            let passphrase = secrecy::SecretString::from(text.as_str());
            drop(text);
            // The entry does not keep it while the add runs, nor after.
            passphrase_row.set_text("");

            busy.set(true);
            status_label.set_text(&i18n("Unlocking…"));
            status_label.remove_css_class("error");
            status_label.add_css_class("dim-label");
            status_label.set_visible(true);
            dialog.set_response_enabled("unlock", false);
            dialog.set_response_enabled("cancel", false);

            let key = key_path.clone();
            let socket = socket_path.clone();
            let dialog = dialog.clone();
            let finish = finish.clone();
            let busy = busy.clone();
            let attempts = attempts.clone();
            let passphrase_row = passphrase_row.clone();
            let status_label = status_label.clone();
            let key_path = key_path.clone();
            crate::utils::spawn_blocking_with_callback(
                // The passphrase moves into the worker and drops there when
                // the add returns.
                move || {
                    SshAgentManager::new(Some(socket))
                        .add_own_key_with_passphrase(&key, &passphrase)
                },
                move |result| {
                    busy.set(false);
                    attempts.set(attempts.get().saturating_add(u8::from(matches!(
                        result,
                        Err(AgentError::WrongPassphrase)
                    ))));
                    match &result {
                        Ok(()) => {
                            tracing::info!(?key_path, "SSH key unlocked and added to the agent");
                        }
                        Err(AgentError::AddRefused(e)) => {
                            tracing::info!(?key_path, error = %e, "the SSH agent refused the key");
                        }
                        Err(AgentError::WrongPassphrase) => {
                            tracing::debug!(?key_path, "wrong SSH key passphrase");
                        }
                        Err(_) => {}
                    }
                    if let Some(outcome) = next_step(result, attempts.get()) {
                        if outcome == KeyUnlockOutcome::GaveUp {
                            tracing::info!(
                                ?key_path,
                                "too many wrong SSH key passphrases; not adding the key"
                            );
                        }
                        finish(&dialog, outcome);
                    } else {
                        status_label.set_text(&i18n("Incorrect passphrase"));
                        status_label.remove_css_class("dim-label");
                        status_label.add_css_class("error");
                        status_label.set_visible(true);
                        dialog.set_response_enabled("cancel", true);
                        dialog.set_response_enabled("unlock", !passphrase_row.text().is_empty());
                        passphrase_row.grab_focus();
                    }
                },
            );
        });
    }

    // Escape and the close button arrive here while `can-close` is off. While
    // an add is running the dialog stays: its result decides the outcome.
    dialog.connect_close_attempt(move |dialog| {
        if !busy_on_close.get() {
            finish(dialog, KeyUnlockOutcome::Cancelled);
        }
    });

    dialog.present(Some(parent));
}

/// How the dialog ends after an add attempt: `None` asks for the passphrase
/// again, `Some` closes it. `attempts` counts wrong passphrases so far,
/// including this one.
fn next_step(result: Result<(), AgentError>, attempts: u8) -> Option<KeyUnlockOutcome> {
    match result {
        Ok(()) => Some(KeyUnlockOutcome::Unlocked),
        Err(AgentError::WrongPassphrase) => match after_wrong_passphrase(attempts) {
            AfterWrongPassphrase::Retry => None,
            AfterWrongPassphrase::GiveUp => Some(KeyUnlockOutcome::GaveUp),
        },
        Err(AgentError::AddRefused(_)) => Some(KeyUnlockOutcome::Refused),
        Err(e) => Some(KeyUnlockOutcome::Failed(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_passphrase_retries_until_the_limit() {
        assert_eq!(next_step(Err(AgentError::WrongPassphrase), 1), None);
        // The limit itself gives up; find it from the core rule, not a copy.
        let limit = (1..=u8::MAX)
            .find(|n| after_wrong_passphrase(*n) == AfterWrongPassphrase::GiveUp)
            .expect("a limit exists");
        assert_eq!(next_step(Err(AgentError::WrongPassphrase), limit - 1), None);
        assert_eq!(
            next_step(Err(AgentError::WrongPassphrase), limit),
            Some(KeyUnlockOutcome::GaveUp)
        );
    }

    #[test]
    fn every_other_result_closes_the_dialog() {
        assert_eq!(next_step(Ok(()), 0), Some(KeyUnlockOutcome::Unlocked));
        assert_eq!(
            next_step(Err(AgentError::AddRefused("1password".into())), 0),
            Some(KeyUnlockOutcome::Refused)
        );
        assert_eq!(
            next_step(Err(AgentError::NotRunning), 0),
            Some(KeyUnlockOutcome::Failed("SSH agent not running".into()))
        );
    }
}
