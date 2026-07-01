//! Secrets settings tab using libadwaita components

mod detection;
mod keyring;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, FileFilter, Label,
    Orientation, StringList, Switch,
};
use libadwaita as adw;
use rustconn_core::config::{SecretBackendType, SecretSettings};
use rustconn_core::secret::{CredentialStorage, set_session_key};
use secrecy::SecretString;
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::i18n;

use self::detection::{
    check_bitwarden_status_sync, detect_secret_backends, extract_session_key,
    read_passbolt_server_url_sync,
};
use self::keyring::{
    get_bw_password_from_keyring, get_kdbx_password_from_keyring, get_op_token_from_keyring,
    get_pb_passphrase_from_keyring, save_bw_password_to_keyring, save_kdbx_password_to_keyring,
    save_op_token_to_keyring, save_pb_passphrase_to_keyring,
};

/// Return type for secrets page - contains all widgets needed for dynamic visibility
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct SecretsPageWidgets {
    pub page: adw::PreferencesPage,
    pub secret_backend_dropdown: DropDown,
    pub enable_fallback: CheckButton,
    pub kdbx_path_entry: Entry,
    pub kdbx_password_entry: adw::PasswordEntryRow,
    pub kdbx_enabled_row: adw::SwitchRow,
    /// 3-state credential storage selector for KeePassXC database password.
    pub kdbx_storage_combo: adw::ComboRow,
    pub kdbx_status_label: Label,
    pub kdbx_browse_button: Button,
    pub kdbx_check_button: Button,
    pub keepassxc_status_container: GtkBox,
    pub kdbx_key_file_entry: Entry,
    pub kdbx_key_file_browse_button: Button,
    pub kdbx_use_key_file_check: Switch,
    pub kdbx_use_password_check: Switch,
    // Additional rows for visibility control
    pub kdbx_group: adw::PreferencesGroup,
    pub auth_group: adw::PreferencesGroup,
    pub status_group: adw::PreferencesGroup,
    pub password_row: adw::PasswordEntryRow,
    pub key_file_row: adw::ActionRow,
    // Bitwarden widgets
    pub bitwarden_group: adw::PreferencesGroup,
    pub bitwarden_status_label: Label,
    pub bitwarden_unlock_button: Button,
    pub bitwarden_password_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for Bitwarden master password.
    pub bitwarden_storage_combo: adw::ComboRow,
    pub bitwarden_use_api_key_check: Switch,
    pub bitwarden_client_id_entry: Entry,
    pub bitwarden_client_secret_entry: adw::PasswordEntryRow,
    /// Detected Bitwarden CLI command path (updated async)
    pub bitwarden_cmd: Rc<RefCell<String>>,
    // 1Password widgets
    pub onepassword_group: adw::PreferencesGroup,
    pub onepassword_status_label: Label,
    pub onepassword_signin_button: Button,
    // Passbolt widgets
    pub passbolt_group: adw::PreferencesGroup,
    pub passbolt_status_label: Label,
    pub passbolt_server_url_entry: Entry,
    pub passbolt_open_vault_button: Button,
    pub passbolt_passphrase_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for Passbolt GPG passphrase.
    pub passbolt_storage_combo: adw::ComboRow,
    // 1Password credential widgets
    pub onepassword_token_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for 1Password service account token.
    pub onepassword_storage_combo: adw::ComboRow,
    /// Cached result of `which secret-tool` (populated by background detection)
    pub secret_tool_available: Rc<RefCell<Option<bool>>>,
    /// Detected 1Password CLI command path (updated async)
    pub onepassword_cmd: Rc<RefCell<String>>,
    // Pass widgets
    pub pass_group: adw::PreferencesGroup,
    pub pass_store_dir_entry: Entry,
    pub pass_store_dir_browse_button: Button,
    pub pass_status_label: Label,
}

/// Index in the storage `StringList` for [`CredentialStorage::None`].
const STORAGE_NONE_INDEX: u32 = 0;
/// Index in the storage `StringList` for [`CredentialStorage::EncryptedFile`].
const STORAGE_ENCRYPTED_INDEX: u32 = 1;
/// Index in the storage `StringList` for [`CredentialStorage::SystemKeyring`].
const STORAGE_KEYRING_INDEX: u32 = 2;

/// Maps a [`CredentialStorage`] to its `StringList` index.
const fn storage_to_index(storage: CredentialStorage) -> u32 {
    match storage {
        CredentialStorage::None => STORAGE_NONE_INDEX,
        CredentialStorage::EncryptedFile => STORAGE_ENCRYPTED_INDEX,
        CredentialStorage::SystemKeyring => STORAGE_KEYRING_INDEX,
    }
}

/// Maps a `StringList` index back to a [`CredentialStorage`]. Unknown indices
/// fall back to [`CredentialStorage::None`].
const fn index_to_storage(idx: u32) -> CredentialStorage {
    match idx {
        STORAGE_ENCRYPTED_INDEX => CredentialStorage::EncryptedFile,
        STORAGE_KEYRING_INDEX => CredentialStorage::SystemKeyring,
        _ => CredentialStorage::None,
    }
}

/// Builds an `AdwComboRow` with the canonical 3-state credential storage
/// choice: "Don't save" / "Encrypted file (machine-specific)" /
/// "System keyring (recommended)".
///
/// The combo enforces availability of `secret-tool` for the keyring option:
/// if the user picks "System keyring" while `secret_tool_available` is false,
/// the selection is reverted to the previous one and `status_label` shows a
/// warning.
fn make_storage_combo(
    title: &str,
    secret_tool_available: Rc<RefCell<Option<bool>>>,
    status_label: Label,
) -> adw::ComboRow {
    let model = StringList::new(&[
        i18n("Don't save").as_str(),
        i18n("Encrypted file (machine-specific)").as_str(),
        i18n("System keyring (recommended)").as_str(),
    ]);
    let combo = adw::ComboRow::builder()
        .title(title)
        .subtitle(i18n("How to persist the credential between sessions"))
        .model(&model)
        .selected(STORAGE_NONE_INDEX)
        .build();

    // Track previous selection so we can revert if the user picks keyring
    // while secret-tool is unavailable.
    let previous: Rc<RefCell<u32>> = Rc::new(RefCell::new(STORAGE_NONE_INDEX));
    {
        let combo_clone = combo.clone();
        let previous_clone = previous.clone();
        combo.connect_selected_notify(move |c| {
            let new_sel = c.selected();
            if new_sel == STORAGE_KEYRING_INDEX
                && !*secret_tool_available.borrow().as_ref().unwrap_or(&false)
            {
                let revert_to = *previous_clone.borrow();
                update_status_label(
                    &status_label,
                    &i18n("Install libsecret-tools for keyring"),
                    "warning",
                );
                tracing::warn!("secret-tool not found, cannot use system keyring");
                // Revert without re-triggering this handler infinitely:
                // selected_notify will still fire but the guard above is a
                // no-op for non-keyring indices.
                combo_clone.set_selected(revert_to);
                return;
            }
            *previous_clone.borrow_mut() = new_sel;
        });
    }

    combo
}

/// Reads the current [`CredentialStorage`] choice from a storage combo.
fn storage_combo_value(combo: &adw::ComboRow) -> CredentialStorage {
    index_to_storage(combo.selected())
}

/// Sets a storage combo to the given [`CredentialStorage`] without triggering
/// validation of `secret-tool` availability — load-time positions should
/// always succeed because they came from a previously-saved config.
fn set_storage_combo_value(combo: &adw::ComboRow, storage: CredentialStorage) {
    combo.set_selected(storage_to_index(storage));
}

/// Creates the secrets settings page using AdwPreferencesPage
pub fn create_secrets_page() -> SecretsPageWidgets {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Secrets"))
        .icon_name("dialog-password-symbolic")
        .build();

    // === Secret Backend Group ===
    let backend_group = adw::PreferencesGroup::builder()
        .title(i18n("Secret Backend"))
        .description(i18n("Choose how passwords are stored"))
        .build();

    // Simplified: KeePassXC, libsecret, Bitwarden, 1Password, Passbolt, Pass,
    // Encrypted file. Index 1 is the system keyring: libsecret on Linux/BSD, the
    // native Keychain on macOS (libsecret does not exist there). Index 6 is the
    // application-managed encrypted-file backend (no system keyring required).
    #[cfg(target_os = "macos")]
    let system_keyring_label = "macOS Keychain";
    #[cfg(not(target_os = "macos"))]
    let system_keyring_label = "libsecret";
    // Descriptive (non-brand) label — sentence case per GNOME HIG and i18n-wrapped.
    let encrypted_file_label = i18n("Encrypted file (no system keyring)");
    let backend_strings = StringList::new(&[
        "KeePassXC",
        system_keyring_label,
        "Bitwarden",
        "1Password",
        "Passbolt",
        "Pass",
        encrypted_file_label.as_str(),
    ]);
    let secret_backend_dropdown = DropDown::builder()
        .model(&backend_strings)
        .selected(0)
        .valign(gtk4::Align::Center)
        .build();
    let backend_row = adw::ActionRow::builder()
        .title(i18n("Backend"))
        .subtitle(i18n("Primary password storage method"))
        .build();
    backend_row.add_suffix(&secret_backend_dropdown);
    backend_row.set_activatable_widget(Some(&secret_backend_dropdown));
    backend_group.add(&backend_row);

    // Version info row - shows version of selected backend
    let version_label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();
    let version_row = adw::ActionRow::builder().title(i18n("Version")).build();
    version_row.add_suffix(&version_label);
    backend_group.add(&version_row);

    // Availability indicator for the system-keyring backend (R4.3). Shown only
    // when the system-keyring entry (index 1) is selected; the other backends
    // already display their own status rows. The text label always names the
    // state, so status is never conveyed by colour alone (GNOME HIG / WCAG).
    let availability_label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .label(i18n("Checking..."))
        .css_classes(["dim-label"])
        .build();
    let availability_row = adw::ActionRow::builder()
        .title(i18n("Availability"))
        .build();
    availability_row.add_suffix(&availability_label);
    availability_row.set_visible(false);
    backend_group.add(&availability_row);

    let enable_fallback = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .active(true)
        .build();
    let fallback_row = adw::ActionRow::builder()
        .title(i18n("Enable fallback"))
        .subtitle(i18n("Use libsecret if primary backend unavailable"))
        .activatable_widget(&enable_fallback)
        .build();
    fallback_row.add_prefix(&enable_fallback);
    backend_group.add(&fallback_row);

    page.add(&backend_group);

    // Version info label — will be populated by async detection
    // Use placeholder defaults; real values arrive from background thread.
    let keepassxc_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let bitwarden_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let onepassword_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let passbolt_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let pass_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Cached secret-tool availability — populated by background detection thread.
    // `None` = not yet checked, `Some(true/false)` = result known.
    let secret_tool_available: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(None));

    // Track whether async detection has completed
    let detection_complete: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

    // Cached fine-grained availability of the system-keyring backend, populated
    // by background detection. Used to render the per-backend availability
    // indicator when the system-keyring entry is selected (#201, R4.3).
    let keyring_availability: Rc<RefCell<Option<rustconn_core::secret::BackendAvailability>>> =
        Rc::new(RefCell::new(None));

    // Shared mutable command paths for callbacks (updated by async detection)
    let bitwarden_cmd: Rc<RefCell<String>> = Rc::new(RefCell::new("bw".to_string()));
    let onepassword_cmd: Rc<RefCell<String>> = Rc::new(RefCell::new("op".to_string()));

    // Initial version display — "Detecting..."
    version_label.set_text(&i18n("Detecting..."));
    version_label.add_css_class("dim-label");

    // === Bitwarden Configuration Group ===
    let bitwarden_group = adw::PreferencesGroup::builder()
        .title(i18n("Bitwarden"))
        .description(i18n("Configure Bitwarden CLI integration"))
        .build();

    // Password entry for unlocking (PasswordEntryRow: built-in peek icon,
    // caps-lock warning and focus on row click)
    let bitwarden_password_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Master Password"))
        .tooltip_text(i18n("Required to unlock vault"))
        .build();
    bitwarden_group.add(&bitwarden_password_entry);

    // Save password checkbox for Bitwarden (encrypted in settings file)
    let bitwarden_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector (replaces the previous pair of
    // "Save password" + "Save to system keyring" CheckButtons + mutual
    // exclusion logic). See `make_storage_combo` for behaviour.
    let bitwarden_storage_combo = make_storage_combo(
        &i18n("Save master password"),
        secret_tool_available.clone(),
        bitwarden_status_label.clone(),
    );
    bitwarden_group.add(&bitwarden_storage_combo);

    // API Key authentication switch
    let bitwarden_use_api_key_check = Switch::builder().valign(gtk4::Align::Center).build();
    let bw_use_api_key_row = adw::ActionRow::builder()
        .title(i18n("Use API key authentication"))
        .subtitle(i18n(
            "For automation or 2FA methods not supported by CLI (FIDO2, Duo)",
        ))
        .build();
    bw_use_api_key_row.add_suffix(&bitwarden_use_api_key_check);
    bw_use_api_key_row.set_activatable_widget(Some(&bitwarden_use_api_key_check));
    bitwarden_group.add(&bw_use_api_key_row);

    // API Client ID entry
    let bitwarden_client_id_entry = Entry::builder()
        .placeholder_text(i18n("client_id"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let bw_client_id_row = adw::ActionRow::builder()
        .title(i18n("Client ID"))
        .subtitle(i18n(
            "From Bitwarden web vault → Settings → Security → Keys",
        ))
        .build();
    bw_client_id_row.add_suffix(&bitwarden_client_id_entry);
    bw_client_id_row.set_activatable_widget(Some(&bitwarden_client_id_entry));
    bitwarden_group.add(&bw_client_id_row);

    // API Client Secret entry
    let bitwarden_client_secret_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Client Secret"))
        .tooltip_text(i18n("Keep this secret safe"))
        .build();
    bitwarden_group.add(&bitwarden_client_secret_entry);

    // Setup visibility for API key fields
    let bw_client_id_row_clone = bw_client_id_row.clone();
    let bw_client_secret_entry_clone = bitwarden_client_secret_entry.clone();
    bitwarden_use_api_key_check.connect_state_set(move |_, state| {
        bw_client_id_row_clone.set_visible(state);
        bw_client_secret_entry_clone.set_visible(state);
        glib::Propagation::Proceed
    });

    // Initial visibility - hide API key fields by default
    bw_client_id_row.set_visible(false);
    bitwarden_client_secret_entry.set_visible(false);

    let bitwarden_unlock_button = Button::builder()
        .label(i18n("Unlock"))
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .tooltip_text(i18n("Unlock Bitwarden vault"))
        .build();

    let bw_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    bw_status_box.append(&bitwarden_status_label);
    bw_status_box.append(&bitwarden_unlock_button);

    let bw_status_row = adw::ActionRow::builder()
        .title(i18n("Vault Status"))
        .subtitle(i18n("Login with 'bw login' in terminal first"))
        .build();
    bw_status_row.add_suffix(&bw_status_box);
    bitwarden_group.add(&bw_status_row);

    // Connect unlock button
    {
        let status_label = bitwarden_status_label.clone();
        let password_entry = bitwarden_password_entry.clone();
        let bw_cmd = bitwarden_cmd.clone();
        let storage_combo = bitwarden_storage_combo.clone();
        bitwarden_unlock_button.connect_clicked(move |button| {
            let password_text = password_entry.text();
            let save_to_keyring =
                storage_combo_value(&storage_combo) == CredentialStorage::SystemKeyring;

            // Resolve password from keyring or text field, wrapping intermediate
            // plaintext copies in Zeroizing so they are wiped on drop
            // (M-PUBLIC-DEBUG / SecretString patterns).
            let password = if password_text.is_empty() && save_to_keyring {
                if let Some(val) = get_bw_password_from_keyring() {
                    use secrecy::ExposeSecret;
                    zeroize::Zeroizing::new(val.expose_secret().to_string())
                } else {
                    update_status_label(&status_label, &i18n("Enter password"), "warning");
                    return;
                }
            } else if password_text.is_empty() {
                update_status_label(&status_label, &i18n("Enter password"), "warning");
                return;
            } else {
                zeroize::Zeroizing::new(password_text.to_string())
            };

            button.set_sensitive(false);
            update_status_label(&status_label, &i18n("Unlocking..."), "dim-label");

            let bw_cmd_str = bw_cmd.borrow().clone();

            // Note: do not log password length — it leaks bruteforce metadata.
            tracing::debug!(
                bw_cmd = %bw_cmd_str,
                password_source = if password_text.is_empty() { "keyring" } else { "manual" },
                has_password = !password.is_empty(),
                "Bitwarden GUI: unlock button clicked"
            );

            // Run unlock with password via environment variable
            // Try --raw first, then verbose output parsing as fallback
            let raw_result = std::process::Command::new(&bw_cmd_str)
                .arg("unlock")
                .arg("--passwordenv")
                .arg("BW_PASSWORD")
                .arg("--raw")
                .env("BW_PASSWORD", password.as_str())
                .output();

            let (session_result, raw_stderr) = match raw_result {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let key = stdout.trim().to_string();
                    if key.is_empty() {
                        (None, String::new())
                    } else {
                        (Some(key), String::new())
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    (None, stderr)
                }
                Err(_) => (None, String::new()),
            };

            // Fallback: try without --raw and parse session key from verbose output
            let session_result = session_result.or_else(|| {
                let result = std::process::Command::new(&bw_cmd_str)
                    .arg("unlock")
                    .arg("--passwordenv")
                    .arg("BW_PASSWORD")
                    .env("BW_PASSWORD", password.as_str())
                    .output();
                match result {
                    Ok(output) if output.status.success() => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        extract_session_key(&stdout)
                    }
                    _ => None,
                }
            });

            if let Some(session_key) = session_result {
                tracing::info!(
                    session_key_len = session_key.len(),
                    "Bitwarden GUI: unlock succeeded"
                );
                set_session_key(SecretString::from(session_key));
                update_status_label(&status_label, &i18n("Unlocked"), "success");
                // Don't clear password_entry — it's a PasswordEntry (hidden),
                // and clearing it causes the encrypted settings to keep a stale
                // password when the user saves settings with an empty field.

                // Save to keyring if checkbox is active
                if save_to_keyring {
                    save_bw_password_to_keyring(password.as_str());
                }
            } else {
                tracing::warn!(
                    raw_stderr = %raw_stderr,
                    "Bitwarden GUI: unlock failed"
                );
                let msg = if raw_stderr.contains("Invalid master password") {
                    i18n("Invalid password")
                } else if raw_stderr.contains("not logged in") {
                    i18n("Not logged in")
                } else {
                    i18n("Unlock failed")
                };
                update_status_label(&status_label, &msg, "error");
            }

            button.set_sensitive(true);
        });
    }

    page.add(&bitwarden_group);

    // === 1Password Configuration Group ===
    let onepassword_group = adw::PreferencesGroup::builder()
        .title(i18n("1Password"))
        .description(i18n("Configure 1Password CLI integration"))
        .build();

    // Service account token entry
    let onepassword_token_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Service Account Token"))
        .tooltip_text(i18n(
            "For headless/automated access (OP_SERVICE_ACCOUNT_TOKEN)",
        ))
        .build();
    onepassword_group.add(&onepassword_token_entry);

    // Save password checkbox (encrypted in settings file)
    let onepassword_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector for the 1Password service account
    // token (replaces the previous "Save token" + "Save to system keyring"
    // CheckButton pair plus mutual-exclusion logic).
    let onepassword_storage_combo = make_storage_combo(
        &i18n("Save token"),
        secret_tool_available.clone(),
        onepassword_status_label.clone(),
    );
    onepassword_group.add(&onepassword_storage_combo);

    let onepassword_signin_button = Button::builder()
        .label(i18n("Sign In"))
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .tooltip_text(i18n("Sign in to 1Password (opens terminal)"))
        .build();

    let op_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    op_status_box.append(&onepassword_status_label);
    op_status_box.append(&onepassword_signin_button);

    let op_status_row = adw::ActionRow::builder()
        .title(i18n("Account Status"))
        .subtitle(i18n(
            "Sign in with 'op signin' in terminal or use biometric unlock",
        ))
        .build();
    op_status_row.add_suffix(&op_status_box);
    onepassword_group.add(&op_status_row);

    // Connect signin button - opens terminal for interactive signin
    {
        let status_label = onepassword_status_label.clone();
        let op_cmd = onepassword_cmd.clone();
        onepassword_signin_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            update_status_label(&status_label, &i18n("Opening terminal..."), "dim-label");

            // Try to open a terminal with op signin
            // This requires user interaction for biometric or password
            let op_cmd_str = op_cmd.borrow().clone();
            let xfce_cmd = format!("{op_cmd_str} signin");
            let terminal_cmds: [(&str, Vec<&str>); 4] = [
                ("gnome-terminal", vec!["--", &op_cmd_str, "signin"]),
                ("konsole", vec!["-e", &op_cmd_str, "signin"]),
                ("xfce4-terminal", vec!["-e", &xfce_cmd]),
                ("xterm", vec!["-e", &op_cmd_str, "signin"]),
            ];

            let mut launched = false;
            for (term, args) in &terminal_cmds {
                if std::process::Command::new("which")
                    .arg(term)
                    .output()
                    .is_ok_and(|o| o.status.success())
                    && std::process::Command::new(term)
                        .args(args.iter().copied())
                        .spawn()
                        .is_ok()
                {
                    launched = true;
                    update_status_label(&status_label, &i18n("Check terminal"), "warning");
                    break;
                }
            }

            if !launched {
                update_status_label(&status_label, &i18n("No terminal found"), "error");
            }

            button.set_sensitive(true);
        });
    }

    page.add(&onepassword_group);

    // === Passbolt Configuration Group ===
    let passbolt_group = adw::PreferencesGroup::builder()
        .title(i18n("Passbolt"))
        .description(i18n("Configure Passbolt CLI integration"))
        .build();

    // Server URL entry
    let passbolt_server_url_entry = Entry::builder()
        .placeholder_text("https://passbolt.example.org")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let pb_url_row = adw::ActionRow::builder()
        .title(i18n("Server URL"))
        .subtitle(i18n("Passbolt web vault address"))
        .build();
    pb_url_row.add_suffix(&passbolt_server_url_entry);
    pb_url_row.set_activatable_widget(Some(&passbolt_server_url_entry));
    passbolt_group.add(&pb_url_row);

    // GPG Passphrase entry
    let passbolt_passphrase_entry = adw::PasswordEntryRow::builder()
        .title(i18n("GPG Passphrase"))
        .tooltip_text(i18n("Required to decrypt credentials from Passbolt"))
        .build();
    passbolt_group.add(&passbolt_passphrase_entry);

    // Save passphrase checkbox (encrypted in settings file)
    let passbolt_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector for the Passbolt GPG passphrase
    // (replaces the previous pair of CheckButtons + mutual-exclusion logic).
    let passbolt_storage_combo = make_storage_combo(
        &i18n("Save passphrase"),
        secret_tool_available.clone(),
        passbolt_status_label.clone(),
    );
    passbolt_group.add(&passbolt_storage_combo);

    let passbolt_open_vault_button = Button::builder()
        .label(i18n("Open Vault"))
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .tooltip_text(i18n("Open Passbolt web vault in browser"))
        .build();

    let pb_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    pb_status_box.append(&passbolt_status_label);
    pb_status_box.append(&passbolt_open_vault_button);

    let pb_status_row = adw::ActionRow::builder()
        .title(i18n("Server Status"))
        .subtitle(i18n("Configure with 'passbolt configure' in terminal"))
        .build();
    pb_status_row.add_suffix(&pb_status_box);
    passbolt_group.add(&pb_status_row);

    // Connect Open Vault button
    {
        let url_entry = passbolt_server_url_entry.clone();
        let status_label = passbolt_status_label.clone();
        passbolt_open_vault_button.connect_clicked(move |_| {
            let url_text = url_entry.text();
            let url = if url_text.is_empty() {
                // Try reading from CLI config as fallback
                read_passbolt_server_url_sync()
            } else {
                Some(url_text.to_string())
            };

            if let Some(ref server_url) = url {
                let result = std::process::Command::new(rustconn_core::secret::url_open_command())
                    .arg(server_url)
                    .spawn();
                if result.is_err() {
                    update_status_label(&status_label, &i18n("Failed to open browser"), "error");
                }
            } else {
                update_status_label(
                    &status_label,
                    &i18n("Enter server URL or run 'passbolt configure'"),
                    "warning",
                );
            }
        });
    }

    page.add(&passbolt_group);

    // === Pass (Unix Password Manager) Group ===
    let pass_group = adw::PreferencesGroup::builder()
        .title(i18n("Pass"))
        .description(i18n("Configure Pass (passwordstore.org) integration"))
        .build();

    // Store directory entry with browse button
    let pass_store_dir_entry = Entry::builder()
        .placeholder_text(i18n("~/.password-store"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    let pass_store_dir_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Choose password store directory"))
        .build();
    pass_store_dir_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Choose password store directory",
    ))]);

    let pass_dir_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    pass_dir_box.append(&pass_store_dir_entry);
    pass_dir_box.append(&pass_store_dir_browse_button);

    let pass_dir_row = adw::ActionRow::builder()
        .title(i18n("Store Directory"))
        .subtitle(i18n("Location of password-store (leave empty for default)"))
        .build();
    pass_dir_row.add_suffix(&pass_dir_box);
    pass_group.add(&pass_dir_row);

    // Status label showing initialization status
    let pass_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    let pass_status_row = adw::ActionRow::builder()
        .title(i18n("Initialization Status"))
        .subtitle(i18n("Run 'pass init &lt;gpg-id&gt;' to initialize"))
        .build();
    pass_status_row.add_suffix(&pass_status_label);
    pass_group.add(&pass_status_row);

    // Setup browse button for pass store directory
    {
        let entry = pass_store_dir_entry.clone();
        pass_store_dir_browse_button.connect_clicked(move |button| {
            let entry_clone = entry.clone();
            let dialog = FileDialog::builder()
                .title(i18n("Select Password Store Directory"))
                .modal(true)
                .build();

            if let Some(window) = button
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok())
            {
                dialog.select_folder(Some(&window), gtk4::gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result {
                        let path = file.path();
                        if let Some(p) = path {
                            entry_clone.set_text(&p.to_string_lossy());
                        }
                    }
                });
            }
        });
    }

    page.add(&pass_group);

    // === KeePass Database Group ===
    let kdbx_group = adw::PreferencesGroup::builder()
        .title(i18n("KeePass Database"))
        .description(i18n(
            "Configure KDBX file integration (works with KeePassXC, GNOME Secrets, etc.)",
        ))
        .build();

    let kdbx_enabled_row = adw::SwitchRow::builder()
        .title(i18n("KDBX Integration"))
        .subtitle(i18n("Enable direct database access"))
        .build();
    kdbx_group.add(&kdbx_enabled_row);

    // Database path with browse button
    let kdbx_path_entry = Entry::builder()
        .placeholder_text(i18n("Select .kdbx file"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let kdbx_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Browse for database file"))
        .build();
    kdbx_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for KeePass database file",
    ))]);
    let kdbx_path_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    kdbx_path_box.append(&kdbx_path_entry);
    kdbx_path_box.append(&kdbx_browse_button);

    let kdbx_path_row = adw::ActionRow::builder()
        .title(i18n("Database File"))
        .build();
    kdbx_path_row.add_suffix(&kdbx_path_box);
    kdbx_group.add(&kdbx_path_row);

    page.add(&kdbx_group);

    // === Authentication Group ===
    let auth_group = adw::PreferencesGroup::builder()
        .title(i18n("Authentication"))
        .description(i18n("Database unlock methods"))
        .build();

    // Use password switch
    let kdbx_use_password_check = Switch::builder()
        .active(true)
        .valign(gtk4::Align::Center)
        .build();
    let use_password_row = adw::ActionRow::builder()
        .title(i18n("Use password"))
        .build();
    use_password_row.add_suffix(&kdbx_use_password_check);
    use_password_row.set_activatable_widget(Some(&kdbx_use_password_check));
    auth_group.add(&use_password_row);

    // Password entry (the row itself; `password_row` aliases it for the
    // visibility toggling driven by the "Use password" switch)
    let kdbx_password_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Database password"))
        .build();
    let password_row = kdbx_password_entry.clone();
    auth_group.add(&kdbx_password_entry);

    // Save password checkbox
    let kdbx_status_label = Label::builder()
        .label(i18n("Not connected"))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        // Long error strings (e.g. "Invalid database password or key file")
        // must not overflow the row next to the Check button (#182). Cap the
        // width and ellipsize; update_status_label sets the full text as a
        // tooltip so nothing is lost.
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector for the KeePassXC database
    // password (replaces the previous pair of CheckButtons + mutual-exclusion
    // logic).
    let kdbx_storage_combo = make_storage_combo(
        &i18n("Save password"),
        secret_tool_available.clone(),
        kdbx_status_label.clone(),
    );
    auth_group.add(&kdbx_storage_combo);

    // Use key file switch
    let kdbx_use_key_file_check = Switch::builder().valign(gtk4::Align::Center).build();
    let use_key_file_row = adw::ActionRow::builder()
        .title(i18n("Use key file"))
        .build();
    use_key_file_row.add_suffix(&kdbx_use_key_file_check);
    use_key_file_row.set_activatable_widget(Some(&kdbx_use_key_file_check));
    auth_group.add(&use_key_file_row);

    // Key file path with browse button
    let kdbx_key_file_entry = Entry::builder()
        .placeholder_text(i18n("Select .keyx or .key file"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let kdbx_key_file_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Browse for key file"))
        .build();
    kdbx_key_file_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for key file",
    ))]);
    let key_file_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    key_file_box.append(&kdbx_key_file_entry);
    key_file_box.append(&kdbx_key_file_browse_button);

    let key_file_row = adw::ActionRow::builder().title(i18n("Key File")).build();
    key_file_row.add_suffix(&key_file_box);
    auth_group.add(&key_file_row);

    page.add(&auth_group);

    // === Status Group ===
    let status_group = adw::PreferencesGroup::builder()
        .title(i18n("KDBX Status"))
        .build();

    // Check connection button
    let kdbx_check_button = Button::builder()
        .label(i18n("Check"))
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Test database connection"))
        .build();

    let status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    status_box.append(&kdbx_status_label);
    status_box.append(&kdbx_check_button);

    let status_row = adw::ActionRow::builder()
        .title(i18n("Connection Status"))
        .build();
    status_row.add_suffix(&status_box);
    status_group.add(&status_row);

    page.add(&status_group);

    // Setup visibility connections for password fields. The storage combo
    // tracks the password row, hidden when password auth is disabled.
    let password_row_clone = password_row.clone();
    let kdbx_storage_combo_clone = kdbx_storage_combo.clone();
    kdbx_use_password_check.connect_state_set(move |_, state| {
        password_row_clone.set_visible(state);
        kdbx_storage_combo_clone.set_visible(state);
        glib::Propagation::Proceed
    });

    // Setup visibility connections for key file fields
    let key_file_row_clone = key_file_row.clone();
    kdbx_use_key_file_check.connect_state_set(move |_, state| {
        key_file_row_clone.set_visible(state);
        glib::Propagation::Proceed
    });

    // Setup visibility for KeePass sections when integration is enabled/disabled
    let auth_group_clone = auth_group.clone();
    let status_group_clone = status_group.clone();
    kdbx_enabled_row.connect_active_notify(move |row| {
        let state = row.is_active();
        auth_group_clone.set_visible(state);
        status_group_clone.set_visible(state);
    });

    // Setup visibility for Bitwarden, 1Password, Passbolt, and Pass groups based on backend
    // Indices: 0=KeePassXC, 1=libsecret, 2=Bitwarden, 3=1Password, 4=Passbolt, 5=Pass
    let bitwarden_group_clone = bitwarden_group.clone();
    let onepassword_group_clone = onepassword_group.clone();
    let passbolt_group_clone = passbolt_group.clone();
    let pass_group_clone = pass_group.clone();
    let kdbx_group_clone = kdbx_group.clone();
    let auth_group_clone2 = auth_group.clone();
    let status_group_clone2 = status_group.clone();
    let kdbx_enabled_row_clone = kdbx_enabled_row.clone();
    let version_label_clone = version_label.clone();
    let version_row_clone = version_row.clone();
    let keepassxc_version_clone = keepassxc_version.clone();
    let bitwarden_version_clone = bitwarden_version.clone();
    let onepassword_version_clone = onepassword_version.clone();
    let passbolt_version_clone = passbolt_version.clone();
    let pass_version_clone = pass_version.clone();
    let detection_complete_clone = detection_complete.clone();
    let availability_row_clone = availability_row.clone();
    let availability_label_clone = availability_label.clone();
    let keyring_availability_clone = keyring_availability.clone();
    // Clones for on-demand keyring loading when user switches backend
    let bw_status_label_switch = bitwarden_status_label.clone();
    let op_token_entry_switch = onepassword_token_entry.clone();
    let op_status_label_switch = onepassword_status_label.clone();
    let pb_passphrase_entry_switch = passbolt_passphrase_entry.clone();
    let kdbx_password_entry_switch = kdbx_password_entry.clone();
    secret_backend_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        // Show Bitwarden group only when Bitwarden is selected (index 2)
        bitwarden_group_clone.set_visible(selected == 2);
        // Show 1Password group only when 1Password is selected (index 3)
        onepassword_group_clone.set_visible(selected == 3);
        // Show Passbolt group only when Passbolt is selected (index 4)
        passbolt_group_clone.set_visible(selected == 4);
        // Show Pass group only when Pass is selected (index 5)
        pass_group_clone.set_visible(selected == 5);
        // Show KDBX groups only when KeePassXC is selected (index 0)
        let show_kdbx = selected == 0;
        kdbx_group_clone.set_visible(show_kdbx);
        // Auth and status groups depend on both backend selection and kdbx_enabled
        let kdbx_enabled = kdbx_enabled_row_clone.is_active();
        auth_group_clone2.set_visible(show_kdbx && kdbx_enabled);
        status_group_clone2.set_visible(show_kdbx && kdbx_enabled);

        // System-keyring availability indicator: only the system-keyring entry
        // (index 1) uses the keyring probe; other backends show their own
        // status rows, so hide this indicator for them (R4.3).
        if selected == 1 {
            render_keyring_availability(
                &availability_row_clone,
                &availability_label_clone,
                keyring_availability_clone.borrow().as_ref(),
            );
        } else {
            availability_row_clone.set_visible(false);
        }

        // Helper to set version label text and style
        let detected = *detection_complete_clone.borrow();
        let set_ver = |ver: &Option<String>| {
            version_row_clone.set_visible(true);
            version_label_clone.remove_css_class("error");
            version_label_clone.remove_css_class("success");
            version_label_clone.remove_css_class("dim-label");
            if let Some(ref v) = *ver {
                version_label_clone.set_text(&format!("v{v}"));
                version_label_clone.add_css_class("success");
            } else if detected {
                version_label_clone.set_text(&i18n("Not installed"));
                version_label_clone.add_css_class("error");
            } else {
                version_label_clone.set_text(&i18n("Detecting..."));
                version_label_clone.add_css_class("dim-label");
            }
        };

        // Update version label based on selected backend
        match selected {
            0 => set_ver(&keepassxc_version_clone.borrow()),
            1 => version_row_clone.set_visible(false),
            2 => set_ver(&bitwarden_version_clone.borrow()),
            3 => set_ver(&onepassword_version_clone.borrow()),
            4 => set_ver(&passbolt_version_clone.borrow()),
            5 => set_ver(&pass_version_clone.borrow()),
            _ => version_row_clone.set_visible(false),
        }

        // On-demand keyring loading when user switches to a new backend
        match selected {
            2 => {
                // Bitwarden selected — trigger auto-unlock from keyring
                let status_label = bw_status_label_switch.clone();
                glib::spawn_future_local(async move {
                    let result = gtk4::gio::spawn_blocking(move || {
                        use secrecy::ExposeSecret;
                        let bw_cmd = rustconn_core::secret::get_bw_cmd();
                        let password = get_bw_password_from_keyring();
                        let password = password?;
                        let bw_status = check_bitwarden_status_sync(&bw_cmd);
                        if bw_status.0 != "Locked" {
                            return Some((bw_status.0, bw_status.1, None));
                        }
                        let unlock_result = std::process::Command::new(&bw_cmd)
                            .arg("unlock")
                            .arg("--passwordenv")
                            .arg("BW_PASSWORD")
                            .env("BW_PASSWORD", password.expose_secret())
                            .output();
                        if let Ok(output) = unlock_result
                            && output.status.success()
                        {
                            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                            if let Some(session_key) = extract_session_key(&stdout) {
                                return Some((
                                    "Unlocked".to_string(),
                                    "success",
                                    Some(session_key),
                                ));
                            }
                        }
                        Some(("Locked".to_string(), "warning", None))
                    })
                    .await
                    .ok()
                    .flatten();
                    if let Some((text, css, session_key)) = result {
                        if let Some(key) = session_key {
                            set_session_key(SecretString::from(key));
                        }
                        update_status_label(&status_label, &text, css);
                    }
                });
            }
            3 => {
                // 1Password selected — load token from keyring
                let token_entry = op_token_entry_switch.clone();
                let status_label = op_status_label_switch.clone();
                glib::spawn_future_local(async move {
                    let token = gtk4::gio::spawn_blocking(get_op_token_from_keyring)
                        .await
                        .ok()
                        .flatten();
                    if let Some(token) = token {
                        use secrecy::ExposeSecret;
                        token_entry.set_text(token.expose_secret());
                        update_status_label(
                            &status_label,
                            &i18n("Token loaded from keyring"),
                            "success",
                        );
                    }
                });
            }
            4 => {
                // Passbolt selected — load passphrase from keyring
                let passphrase_entry = pb_passphrase_entry_switch.clone();
                glib::spawn_future_local(async move {
                    let passphrase = gtk4::gio::spawn_blocking(get_pb_passphrase_from_keyring)
                        .await
                        .ok()
                        .flatten();
                    if let Some(passphrase) = passphrase {
                        use secrecy::ExposeSecret;
                        passphrase_entry.set_text(passphrase.expose_secret());
                    }
                });
            }
            0 => {
                // KeePassXC selected — load password from keyring
                let password_entry = kdbx_password_entry_switch.clone();
                glib::spawn_future_local(async move {
                    let password = gtk4::gio::spawn_blocking(get_kdbx_password_from_keyring)
                        .await
                        .ok()
                        .flatten();
                    if let Some(password) = password {
                        use secrecy::ExposeSecret;
                        password_entry.set_text(password.expose_secret());
                    }
                });
            }
            _ => {} // LibSecret, Pass, macOS Keychain — stateless
        }
    });

    // Initial visibility based on default states (KeePassXC selected by default)
    key_file_row.set_visible(false);
    password_row.set_visible(true);
    kdbx_storage_combo.set_visible(true);
    auth_group.set_visible(false);
    status_group.set_visible(false);
    bitwarden_group.set_visible(false);
    onepassword_group.set_visible(false);
    passbolt_group.set_visible(false);
    pass_group.set_visible(false);

    // Initial version display set above as "Detecting..."

    // Setup browse button for database file
    let kdbx_path_entry_clone = kdbx_path_entry.clone();
    kdbx_browse_button.connect_clicked(move |button| {
        let entry = kdbx_path_entry_clone.clone();
        let dialog = FileDialog::builder()
            .title(i18n("Select KeePass Database"))
            .modal(true)
            .build();

        let filter = FileFilter::new();
        filter.add_pattern("*.kdbx");
        filter.set_name(Some(&i18n("KeePass Database (*.kdbx)")));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let root = button.root();
        let window = root.and_then(|r| r.downcast::<gtk4::Window>().ok());

        dialog.open(
            window.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.display().to_string());
                }
            },
        );
    });

    // Setup browse button for key file
    let kdbx_key_file_entry_clone = kdbx_key_file_entry.clone();
    kdbx_key_file_browse_button.connect_clicked(move |button| {
        let entry = kdbx_key_file_entry_clone.clone();
        let dialog = FileDialog::builder()
            .title(i18n("Select Key File"))
            .modal(true)
            .build();

        let filter = FileFilter::new();
        filter.add_pattern("*.keyx");
        filter.add_pattern("*.key");
        filter.set_name(Some(&i18n("Key Files (*.keyx, *.key)")));

        let all_filter = FileFilter::new();
        all_filter.add_pattern("*");
        all_filter.set_name(Some(&i18n("All Files")));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);
        filters.append(&all_filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let root = button.root();
        let window = root.and_then(|r| r.downcast::<gtk4::Window>().ok());

        dialog.open(
            window.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.display().to_string());
                }
            },
        );
    });

    // Setup check connection button
    let kdbx_path_entry_check = kdbx_path_entry.clone();
    let kdbx_password_entry_check = kdbx_password_entry.clone();
    let kdbx_key_file_entry_check = kdbx_key_file_entry.clone();
    let kdbx_use_password_check_clone = kdbx_use_password_check.clone();
    let kdbx_use_key_file_check_clone = kdbx_use_key_file_check.clone();
    let kdbx_status_label_check = kdbx_status_label.clone();
    kdbx_check_button.connect_clicked(move |_| {
        let path_text = kdbx_path_entry_check.text();
        if path_text.is_empty() {
            update_status_label(
                &kdbx_status_label_check,
                &i18n("No database selected"),
                "warning",
            );
            return;
        }

        let kdbx_path = std::path::Path::new(path_text.as_str());

        let password = if kdbx_use_password_check_clone.is_active() {
            let pwd = kdbx_password_entry_check.text();
            if pwd.is_empty() {
                None
            } else {
                Some(pwd.to_string())
            }
        } else {
            None
        };

        let key_file = if kdbx_use_key_file_check_clone.is_active() {
            let kf = kdbx_key_file_entry_check.text();
            if kf.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(kf.as_str()))
            }
        } else {
            None
        };

        let password_secret = password.map(secrecy::SecretString::from);

        let result = rustconn_core::secret::KeePassStatus::verify_kdbx_credentials(
            kdbx_path,
            password_secret.as_ref(),
            key_file.as_deref(),
        );

        match result {
            Ok(()) => {
                update_status_label(&kdbx_status_label_check, &i18n("Connected"), "success");
            }
            Err(e) => {
                update_status_label(&kdbx_status_label_check, &e.to_string(), "error");
            }
        }
    });

    let keepassxc_status_container = GtkBox::new(Orientation::Vertical, 6);

    // Schedule async CLI detection on background thread
    {
        let version_label = version_label.clone();
        let version_row = version_row.clone();
        let bw_status_label = bitwarden_status_label.clone();
        let bw_unlock_btn = bitwarden_unlock_button.clone();
        let bw_cmd_rc = bitwarden_cmd.clone();
        let op_status_label = onepassword_status_label.clone();
        let op_signin_btn = onepassword_signin_button.clone();
        let op_cmd_rc = onepassword_cmd.clone();
        let dropdown = secret_backend_dropdown.clone();
        let pb_status_label = passbolt_status_label.clone();
        let pb_vault_btn = passbolt_open_vault_button.clone();
        let pb_open_button = passbolt_open_vault_button.clone();
        let pb_url_entry = passbolt_server_url_entry.clone();
        let pass_status_label = pass_status_label.clone();
        let kpxc_ver = keepassxc_version.clone();
        let bw_ver = bitwarden_version.clone();
        let op_ver = onepassword_version.clone();
        let pb_ver = passbolt_version.clone();
        let pass_ver = pass_version.clone();
        let det_complete = detection_complete.clone();
        let st_avail = secret_tool_available.clone();
        let availability_row_det = availability_row.clone();
        let availability_label_det = availability_label.clone();
        let keyring_avail_det = keyring_availability.clone();

        // Run detection on a real OS thread so the GTK main loop stays idle
        // and can render frames while detection runs in the background.
        // GTK widgets are not Send, so we use a channel to pass results back.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let det = detect_secret_backends();
            let _ = tx.send(det);
        });

        // Poll the channel from the main thread; GTK widgets stay here.
        // 50ms timeout instead of idle_add_local: an idle source would spin
        // the main loop at 100% CPU until the detection thread finishes.
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(det) => {
                    // Store detected command paths
                    *bw_cmd_rc.borrow_mut() = det.bitwarden_cmd.clone();
                    rustconn_core::secret::set_bw_cmd(&det.bitwarden_cmd);
                    *op_cmd_rc.borrow_mut() = det.onepassword_cmd;

                    // Store versions for dropdown callback
                    *kpxc_ver.borrow_mut() = det.keepassxc_version.clone();
                    *bw_ver.borrow_mut() = det.bitwarden_version.clone();
                    *op_ver.borrow_mut() = det.onepassword_version.clone();
                    *pb_ver.borrow_mut() = det.passbolt_version.clone();
                    *pass_ver.borrow_mut() = det.pass_version.clone();
                    *det_complete.borrow_mut() = true;
                    *st_avail.borrow_mut() = Some(det.secret_tool_available);
                    *keyring_avail_det.borrow_mut() = Some(det.system_keyring_availability.clone());

                    // Refresh the system-keyring availability indicator if that
                    // backend is currently selected (#201, R4.3).
                    if dropdown.selected() == 1 {
                        render_keyring_availability(
                            &availability_row_det,
                            &availability_label_det,
                            Some(&det.system_keyring_availability),
                        );
                    }

                    // Update version label for currently selected backend
                    let selected = dropdown.selected();
                    let cur_ver = match selected {
                        0 => &det.keepassxc_version,
                        2 => &det.bitwarden_version,
                        3 => &det.onepassword_version,
                        4 => &det.passbolt_version,
                        5 => &det.pass_version,
                        _ => &None,
                    };
                    version_label.remove_css_class("dim-label");
                    version_label.remove_css_class("error");
                    version_label.remove_css_class("success");
                    if selected == 1 {
                        version_row.set_visible(false);
                    } else if let Some(v) = cur_ver {
                        version_label.set_text(&format!("v{v}"));
                        version_label.add_css_class("success");
                    } else {
                        version_label.set_text(&i18n("Not installed"));
                        version_label.add_css_class("error");
                    }

                    // Update Bitwarden status
                    bw_unlock_btn.set_sensitive(det.bitwarden_installed);
                    if let Some((text, css)) = det.bitwarden_status {
                        update_status_label(&bw_status_label, &text, css);
                    } else {
                        update_status_label(&bw_status_label, &i18n("Not installed"), "error");
                    }

                    // Update 1Password status
                    op_signin_btn.set_sensitive(det.onepassword_installed);
                    if let Some((text, css)) = det.onepassword_status {
                        update_status_label(&op_status_label, &text, css);
                    } else {
                        update_status_label(&op_status_label, &i18n("Not installed"), "error");
                    }

                    // Update Passbolt status
                    pb_vault_btn.set_sensitive(det.passbolt_installed);
                    if let Some((text, css)) = det.passbolt_status {
                        update_status_label(&pb_status_label, &text, css);
                        if det.passbolt_installed {
                            pb_open_button.set_sensitive(true);
                        }
                    }

                    // Update Pass status label
                    if let Some((text, css)) = det.pass_status {
                        update_status_label(&pass_status_label, &text, css);
                    }

                    // Update Passbolt URL from detection if empty
                    if pb_url_entry.text().is_empty()
                        && let Some(ref url) = det.passbolt_server_url
                    {
                        pb_url_entry.set_text(url);
                    }

                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    SecretsPageWidgets {
        page,
        secret_backend_dropdown,
        enable_fallback,
        kdbx_path_entry,
        kdbx_password_entry,
        kdbx_enabled_row,
        kdbx_storage_combo,
        kdbx_status_label,
        kdbx_browse_button,
        kdbx_check_button,
        keepassxc_status_container,
        kdbx_key_file_entry,
        kdbx_key_file_browse_button,
        kdbx_use_key_file_check,
        kdbx_use_password_check,
        kdbx_group,
        auth_group,
        status_group,
        password_row,
        key_file_row,
        bitwarden_group,
        bitwarden_status_label,
        bitwarden_unlock_button,
        bitwarden_password_entry,
        bitwarden_storage_combo,
        bitwarden_use_api_key_check,
        bitwarden_client_id_entry,
        bitwarden_client_secret_entry,
        bitwarden_cmd,
        onepassword_group,
        onepassword_status_label,
        onepassword_signin_button,
        passbolt_group,
        passbolt_status_label,
        passbolt_server_url_entry,
        passbolt_open_vault_button,
        passbolt_passphrase_entry,
        passbolt_storage_combo,
        onepassword_token_entry,
        onepassword_storage_combo,
        secret_tool_available,
        onepassword_cmd,
        pass_group,
        pass_store_dir_entry,
        pass_store_dir_browse_button,
        pass_status_label,
    }
}

/// Gets CLI version from command output
fn update_status_label(label: &Label, text: &str, css_class: &str) {
    label.set_text(text);
    // Full text in a tooltip so ellipsized status (e.g. long errors) stays
    // readable on hover (#182).
    label.set_tooltip_text(Some(text));
    label.remove_css_class("success");
    label.remove_css_class("warning");
    label.remove_css_class("error");
    label.remove_css_class("dim-label");
    label.add_css_class(css_class);
}

/// Renders the system-keyring availability indicator row from a probe result.
///
/// Reveals `row` and sets `label` to a state name paired with a colour css
/// class (`None` = still probing). The text always names the state, so status
/// is never conveyed by colour alone (GNOME HIG / WCAG). See `availability()`
/// and issue #201.
fn render_keyring_availability(
    row: &adw::ActionRow,
    label: &Label,
    availability: Option<&rustconn_core::secret::BackendAvailability>,
) {
    use rustconn_core::secret::BackendAvailability;

    row.set_visible(true);
    let (text, css) = match availability {
        Some(BackendAvailability::Available) => (i18n("Available"), "success"),
        Some(BackendAvailability::ServiceUnavailable) => {
            (i18n("No keyring service responding"), "warning")
        }
        Some(BackendAvailability::ClientMissing) => (i18n("Keyring client not installed"), "error"),
        None => (i18n("Checking..."), "dim-label"),
    };
    update_status_label(label, &text, css);
}

pub fn load_secret_settings(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    // Indices: 0=KeePassXC, 1=libsecret/Keychain, 2=Bitwarden, 3=1Password,
    // 4=Passbolt, 5=Pass, 6=Encrypted file.
    let backend_index = match settings.preferred_backend {
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => 0,
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => 1,
        SecretBackendType::Bitwarden => 2,
        SecretBackendType::OnePassword => 3,
        SecretBackendType::Passbolt => 4,
        SecretBackendType::Pass => 5,
        SecretBackendType::EncryptedFile => 6,
    };
    widgets.secret_backend_dropdown.set_selected(backend_index);
    widgets.enable_fallback.set_active(settings.enable_fallback);
    widgets.kdbx_enabled_row.set_active(settings.kdbx_enabled);

    if let Some(path) = &settings.kdbx_path {
        widgets
            .kdbx_path_entry
            .set_text(&path.display().to_string());
    }

    if let Some(key_file) = &settings.kdbx_key_file {
        widgets
            .kdbx_key_file_entry
            .set_text(&key_file.display().to_string());
    }

    widgets
        .kdbx_use_password_check
        .set_active(settings.kdbx_use_password);
    widgets
        .kdbx_use_key_file_check
        .set_active(settings.kdbx_use_key_file);
    set_storage_combo_value(&widgets.kdbx_storage_combo, settings.kdbx_storage());

    // Load Bitwarden storage choice
    set_storage_combo_value(
        &widgets.bitwarden_storage_combo,
        settings.bitwarden_storage(),
    );

    // Load Bitwarden API key setting
    widgets
        .bitwarden_use_api_key_check
        .set_active(settings.bitwarden_use_api_key);

    // Load Bitwarden API credentials if available (from encrypted storage)
    if let Some(ref client_id) = settings.bitwarden_client_id {
        use secrecy::ExposeSecret;
        widgets
            .bitwarden_client_id_entry
            .set_text(client_id.expose_secret());
    }
    if let Some(ref client_secret) = settings.bitwarden_client_secret {
        use secrecy::ExposeSecret;
        widgets
            .bitwarden_client_secret_entry
            .set_text(client_secret.expose_secret());
    }

    // Load Passbolt server URL
    if let Some(ref url) = settings.passbolt_server_url {
        widgets.passbolt_server_url_entry.set_text(url);
    }

    // Load 1Password service account token if available
    if let Some(ref token) = settings.onepassword_service_account_token {
        use secrecy::ExposeSecret;
        widgets
            .onepassword_token_entry
            .set_text(token.expose_secret());
    }
    set_storage_combo_value(
        &widgets.onepassword_storage_combo,
        settings.onepassword_storage(),
    );

    // Load Passbolt storage choice
    set_storage_combo_value(&widgets.passbolt_storage_combo, settings.passbolt_storage());

    // Load Pass store directory
    if let Some(ref path) = settings.pass_store_dir {
        widgets
            .pass_store_dir_entry
            .set_text(&path.display().to_string());
    }

    // Show/hide groups based on selected backend
    let show_kdbx = backend_index == 0;
    widgets.kdbx_group.set_visible(show_kdbx);
    widgets
        .auth_group
        .set_visible(show_kdbx && settings.kdbx_enabled);
    widgets
        .status_group
        .set_visible(show_kdbx && settings.kdbx_enabled);
    widgets.bitwarden_group.set_visible(backend_index == 2);
    widgets.onepassword_group.set_visible(backend_index == 3);
    widgets.passbolt_group.set_visible(backend_index == 4);
    widgets.pass_group.set_visible(backend_index == 5);
    widgets.password_row.set_visible(settings.kdbx_use_password);
    widgets
        .kdbx_storage_combo
        .set_visible(settings.kdbx_use_password);
    widgets.key_file_row.set_visible(settings.kdbx_use_key_file);

    let status_text = if settings.kdbx_enabled {
        if settings.kdbx_path.is_some() {
            i18n("Configured")
        } else {
            i18n("Database path required")
        }
    } else {
        i18n("Disabled")
    };

    widgets.kdbx_status_label.set_text(&status_text);

    widgets.kdbx_status_label.remove_css_class("success");
    widgets.kdbx_status_label.remove_css_class("warning");
    widgets.kdbx_status_label.remove_css_class("error");
    widgets.kdbx_status_label.remove_css_class("dim-label");

    let status_css_class = if settings.kdbx_enabled {
        if settings.kdbx_path.is_some() {
            "success"
        } else {
            "warning"
        }
    } else {
        "dim-label"
    };
    widgets.kdbx_status_label.add_css_class(status_css_class);

    // Load credentials from keyring ONLY for the preferred backend (lazy init).
    // Other backends' credentials are loaded on-demand when the user switches
    // to them via the dropdown.
    match settings.preferred_backend {
        SecretBackendType::Bitwarden => {
            load_bitwarden_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::OnePassword => {
            load_onepassword_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::Passbolt => {
            load_passbolt_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
            load_kdbx_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::LibSecret
        | SecretBackendType::MacOsKeychain
        | SecretBackendType::Pass
        | SecretBackendType::EncryptedFile => {
            // Stateless backends — nothing to load from keyring.
            // (EncryptedFile keeps its entries in its own file; no settings-tab
            // credential fields to populate.)
        }
    }
}

/// Loads Bitwarden credentials from keyring and performs auto-unlock.
fn load_bitwarden_credentials_from_keyring(
    widgets: &SecretsPageWidgets,
    settings: &SecretSettings,
) {
    if !settings.bitwarden_save_to_keyring {
        return;
    }
    let status_label = widgets.bitwarden_status_label.clone();
    tracing::debug!("Scheduling Bitwarden auto-unlock from keyring (async)");
    glib::spawn_future_local({
        let status_label = status_label.clone();
        async move {
            let t_bw = std::time::Instant::now();
            let result = gtk4::gio::spawn_blocking(move || {
                use secrecy::ExposeSecret;
                let bw_cmd = rustconn_core::secret::get_bw_cmd();
                let password = get_bw_password_from_keyring();
                let password = if let Some(p) = password {
                    p
                } else {
                    tracing::debug!("No keyring password found for auto-unlock");
                    return None;
                };
                tracing::debug!(
                    bw_cmd = %bw_cmd,
                    "Got keyring password, checking vault status"
                );
                let bw_status = check_bitwarden_status_sync(&bw_cmd);
                if bw_status.0 != "Locked" {
                    return Some((bw_status.0, bw_status.1, None));
                }
                let unlock_result = std::process::Command::new(&bw_cmd)
                    .arg("unlock")
                    .arg("--passwordenv")
                    .arg("BW_PASSWORD")
                    .env("BW_PASSWORD", password.expose_secret())
                    .output();
                if let Ok(output) = unlock_result {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        if let Some(session_key) = extract_session_key(&stdout) {
                            return Some(("Unlocked".to_string(), "success", Some(session_key)));
                        }
                        tracing::warn!("bw unlock succeeded but no session key");
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!(
                            %stderr,
                            "bw unlock from keyring failed"
                        );
                    }
                }
                Some(("Locked".to_string(), "warning", None))
            })
            .await
            .ok()
            .flatten();
            tracing::debug!(
                elapsed_ms = t_bw.elapsed().as_millis(),
                "load_secret_settings — Bitwarden auto-unlock COMPLETED"
            );

            if let Some((text, css, session_key)) = result {
                if let Some(key) = session_key {
                    set_session_key(SecretString::from(key));
                    tracing::info!("Bitwarden auto-unlocked from keyring");
                }
                update_status_label(&status_label, &text, css);
            }
        }
    });
}

/// Loads 1Password service account token from keyring.
fn load_onepassword_credentials_from_keyring(
    widgets: &SecretsPageWidgets,
    settings: &SecretSettings,
) {
    if !settings.onepassword_save_to_keyring {
        return;
    }
    let token_entry = widgets.onepassword_token_entry.clone();
    let status_label = widgets.onepassword_status_label.clone();
    tracing::debug!("Scheduling 1Password token auto-load from keyring (async)");
    glib::spawn_future_local(async move {
        let t_op = std::time::Instant::now();
        let token = gtk4::gio::spawn_blocking(get_op_token_from_keyring)
            .await
            .ok()
            .flatten();
        tracing::debug!(
            elapsed_ms = t_op.elapsed().as_millis(),
            "load_secret_settings — 1Password keyring COMPLETED"
        );

        if let Some(token) = token {
            use secrecy::ExposeSecret;
            tracing::debug!("1Password token loaded from keyring");
            token_entry.set_text(token.expose_secret());
            update_status_label(&status_label, &i18n("Token loaded from keyring"), "success");
            tracing::info!("1Password token set from keyring");
        } else {
            tracing::debug!("No 1Password token found in keyring");
        }
    });
}

/// Loads Passbolt passphrase from keyring.
fn load_passbolt_credentials_from_keyring(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    if !settings.passbolt_save_to_keyring {
        return;
    }
    let passphrase_entry = widgets.passbolt_passphrase_entry.clone();
    tracing::debug!("Scheduling Passbolt passphrase auto-load (async)");
    glib::spawn_future_local(async move {
        let t_pb = std::time::Instant::now();
        let passphrase = gtk4::gio::spawn_blocking(get_pb_passphrase_from_keyring)
            .await
            .ok()
            .flatten();
        tracing::debug!(
            elapsed_ms = t_pb.elapsed().as_millis(),
            "load_secret_settings — Passbolt keyring COMPLETED"
        );

        if let Some(passphrase) = passphrase {
            use secrecy::ExposeSecret;
            tracing::debug!("Passbolt passphrase loaded from keyring");
            passphrase_entry.set_text(passphrase.expose_secret());
            tracing::info!("Passbolt passphrase restored from keyring");
        } else {
            tracing::debug!("No Passbolt passphrase found in keyring");
        }
    });
}

/// Loads KeePassXC password from keyring.
fn load_kdbx_credentials_from_keyring(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    if !settings.kdbx_save_to_keyring {
        return;
    }
    let password_entry = widgets.kdbx_password_entry.clone();
    tracing::debug!("Scheduling KDBX password auto-load (async)");
    glib::spawn_future_local(async move {
        let t_kdbx = std::time::Instant::now();
        let password = gtk4::gio::spawn_blocking(get_kdbx_password_from_keyring)
            .await
            .ok()
            .flatten();
        tracing::debug!(
            elapsed_ms = t_kdbx.elapsed().as_millis(),
            "load_secret_settings — KDBX keyring COMPLETED"
        );

        if let Some(password) = password {
            use secrecy::ExposeSecret;
            tracing::debug!("KDBX password loaded from keyring");
            password_entry.set_text(password.expose_secret());
            tracing::info!("KDBX password restored from keyring");
        } else {
            tracing::debug!("No KDBX password found in keyring");
        }
    });
}

/// Collects secret settings from UI controls
pub fn collect_secret_settings(
    widgets: &SecretsPageWidgets,
    settings: &Rc<RefCell<rustconn_core::config::AppSettings>>,
) -> SecretSettings {
    // Indices: 0=KeePassXC, 1=libsecret/Keychain, 2=Bitwarden, 3=1Password, 4=Passbolt, 5=Pass, 6=Encrypted file
    let preferred_backend = match widgets.secret_backend_dropdown.selected() {
        0 => SecretBackendType::KeePassXc,
        // Index 1 is the platform system keyring (see create_secrets_page).
        #[cfg(target_os = "macos")]
        1 => SecretBackendType::MacOsKeychain,
        #[cfg(not(target_os = "macos"))]
        1 => SecretBackendType::LibSecret,
        2 => SecretBackendType::Bitwarden,
        3 => SecretBackendType::OnePassword,
        4 => SecretBackendType::Passbolt,
        5 => SecretBackendType::Pass,
        6 => SecretBackendType::EncryptedFile,
        _ => SecretBackendType::default(),
    };

    let kdbx_path = {
        let path_text = widgets.kdbx_path_entry.text();
        if path_text.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(path_text.as_str()))
        }
    };

    let kdbx_key_file = {
        let key_file_text = widgets.kdbx_key_file_entry.text();
        if key_file_text.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(key_file_text.as_str()))
        }
    };

    let (kdbx_password, kdbx_password_encrypted) = {
        let storage = storage_combo_value(&widgets.kdbx_storage_combo);
        match storage {
            CredentialStorage::EncryptedFile => {
                let password_text = widgets.kdbx_password_entry.text();
                if password_text.is_empty() {
                    (
                        None,
                        settings.borrow().secrets.kdbx_password_encrypted.clone(),
                    )
                } else {
                    let password = secrecy::SecretString::new(password_text.to_string().into());
                    let encrypted = settings
                        .borrow()
                        .secrets
                        .kdbx_password_encrypted
                        .clone()
                        .or_else(|| Some("encrypted_password_placeholder".to_string()));
                    (Some(password), encrypted)
                }
            }
            // For System keyring or None: never write encrypted blob.
            CredentialStorage::SystemKeyring | CredentialStorage::None => (None, None),
        }
    };

    // Collect Bitwarden password if save is enabled
    let bitwarden_storage = storage_combo_value(&widgets.bitwarden_storage_combo);
    let (bitwarden_password, bitwarden_password_encrypted) = match bitwarden_storage {
        CredentialStorage::EncryptedFile => {
            let password_text = widgets.bitwarden_password_entry.text();
            if password_text.is_empty() {
                // Keep existing encrypted password if field is empty but
                // encrypted-file storage is selected.
                (
                    None,
                    settings
                        .borrow()
                        .secrets
                        .bitwarden_password_encrypted
                        .clone(),
                )
            } else {
                let password = secrecy::SecretString::new(password_text.to_string().into());
                let encrypted = settings
                    .borrow()
                    .secrets
                    .bitwarden_password_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_password_placeholder".to_string()));
                (Some(password), encrypted)
            }
        }
        CredentialStorage::SystemKeyring | CredentialStorage::None => (None, None),
    };

    // Collect Bitwarden API key settings
    let bitwarden_use_api_key = widgets.bitwarden_use_api_key_check.is_active();
    let bitwarden_save_to_keyring = bitwarden_storage == CredentialStorage::SystemKeyring;

    let (bitwarden_client_id, bitwarden_client_id_encrypted) = if bitwarden_use_api_key {
        let client_id_text = widgets.bitwarden_client_id_entry.text();
        if client_id_text.is_empty() {
            // Keep existing encrypted value if field is empty
            (
                None,
                settings
                    .borrow()
                    .secrets
                    .bitwarden_client_id_encrypted
                    .clone(),
            )
        } else {
            let client_id = secrecy::SecretString::new(client_id_text.to_string().into());
            let encrypted = settings
                .borrow()
                .secrets
                .bitwarden_client_id_encrypted
                .clone()
                .or_else(|| Some("encrypted_client_id_placeholder".to_string()));
            (Some(client_id), encrypted)
        }
    } else {
        (None, None)
    };

    let (bitwarden_client_secret, bitwarden_client_secret_encrypted) = if bitwarden_use_api_key {
        let client_secret_text = widgets.bitwarden_client_secret_entry.text();
        if client_secret_text.is_empty() {
            // Keep existing encrypted value if field is empty
            (
                None,
                settings
                    .borrow()
                    .secrets
                    .bitwarden_client_secret_encrypted
                    .clone(),
            )
        } else {
            let client_secret = secrecy::SecretString::new(client_secret_text.to_string().into());
            let encrypted = settings
                .borrow()
                .secrets
                .bitwarden_client_secret_encrypted
                .clone()
                .or_else(|| Some("encrypted_client_secret_placeholder".to_string()));
            (Some(client_secret), encrypted)
        }
    } else {
        (None, None)
    };

    // Collect 1Password service account token
    let onepassword_storage = storage_combo_value(&widgets.onepassword_storage_combo);
    let (onepassword_service_account_token, onepassword_service_account_token_encrypted) =
        match onepassword_storage {
            CredentialStorage::EncryptedFile => {
                let token_text = widgets.onepassword_token_entry.text();
                if token_text.is_empty() {
                    (
                        None,
                        settings
                            .borrow()
                            .secrets
                            .onepassword_service_account_token_encrypted
                            .clone(),
                    )
                } else {
                    let token = secrecy::SecretString::new(token_text.to_string().into());
                    let encrypted = settings
                        .borrow()
                        .secrets
                        .onepassword_service_account_token_encrypted
                        .clone()
                        .or_else(|| Some("encrypted_token_placeholder".to_string()));
                    (Some(token), encrypted)
                }
            }
            CredentialStorage::SystemKeyring | CredentialStorage::None => (None, None),
        };

    // Collect Passbolt passphrase
    let passbolt_storage = storage_combo_value(&widgets.passbolt_storage_combo);
    let (passbolt_passphrase, passbolt_passphrase_encrypted) = match passbolt_storage {
        CredentialStorage::EncryptedFile => {
            let passphrase_text = widgets.passbolt_passphrase_entry.text();
            if passphrase_text.is_empty() {
                (
                    None,
                    settings
                        .borrow()
                        .secrets
                        .passbolt_passphrase_encrypted
                        .clone(),
                )
            } else {
                let passphrase = secrecy::SecretString::new(passphrase_text.to_string().into());
                let encrypted = settings
                    .borrow()
                    .secrets
                    .passbolt_passphrase_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_passphrase_placeholder".to_string()));
                (Some(passphrase), encrypted)
            }
        }
        CredentialStorage::SystemKeyring | CredentialStorage::None => (None, None),
    };

    // Save credentials to keyring when SystemKeyring storage is selected
    let kdbx_storage = storage_combo_value(&widgets.kdbx_storage_combo);
    if bitwarden_storage == CredentialStorage::SystemKeyring {
        let pw = widgets.bitwarden_password_entry.text();
        if !pw.is_empty() {
            save_bw_password_to_keyring(&pw);
        }
    }
    if onepassword_storage == CredentialStorage::SystemKeyring {
        let token = widgets.onepassword_token_entry.text();
        if !token.is_empty() {
            save_op_token_to_keyring(&token);
        }
    }
    if passbolt_storage == CredentialStorage::SystemKeyring {
        let pp = widgets.passbolt_passphrase_entry.text();
        if !pp.is_empty() {
            save_pb_passphrase_to_keyring(&pp);
        }
    }
    if kdbx_storage == CredentialStorage::SystemKeyring {
        let pw = widgets.kdbx_password_entry.text();
        if !pw.is_empty() {
            save_kdbx_password_to_keyring(&pw);
        }
    }

    SecretSettings {
        preferred_backend,
        enable_fallback: widgets.enable_fallback.is_active(),
        kdbx_path,
        kdbx_enabled: widgets.kdbx_enabled_row.is_active(),
        kdbx_password,
        kdbx_password_encrypted,
        kdbx_key_file,
        kdbx_use_key_file: widgets.kdbx_use_key_file_check.is_active(),
        kdbx_use_password: widgets.kdbx_use_password_check.is_active(),
        bitwarden_password,
        bitwarden_password_encrypted,
        bitwarden_use_api_key,
        bitwarden_client_id,
        bitwarden_client_id_encrypted,
        bitwarden_client_secret,
        bitwarden_client_secret_encrypted,
        bitwarden_save_to_keyring,
        kdbx_save_to_keyring: kdbx_storage == CredentialStorage::SystemKeyring,
        onepassword_service_account_token,
        onepassword_service_account_token_encrypted,
        onepassword_save_to_keyring: onepassword_storage == CredentialStorage::SystemKeyring,
        passbolt_passphrase,
        passbolt_passphrase_encrypted,
        passbolt_save_to_keyring: passbolt_storage == CredentialStorage::SystemKeyring,
        passbolt_server_url: {
            let url_text = widgets.passbolt_server_url_entry.text();
            if url_text.is_empty() {
                None
            } else {
                Some(url_text.to_string())
            }
        },
        // Collect Pass store directory
        pass_store_dir: {
            let path_text = widgets.pass_store_dir_entry.text();
            if path_text.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(path_text.as_str()))
            }
        },
    }
}
