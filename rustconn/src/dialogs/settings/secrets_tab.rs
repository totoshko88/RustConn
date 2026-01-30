//! Secrets settings tab using libadwaita components

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, FileFilter, Label,
    Orientation, PasswordEntry, StringList, Switch,
};
use libadwaita as adw;
use rustconn_core::config::{SecretBackendType, SecretSettings};
use std::cell::RefCell;
use std::rc::Rc;

/// Return type for secrets page - contains all widgets needed for dynamic visibility
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct SecretsPageWidgets {
    pub page: adw::PreferencesPage,
    pub secret_backend_dropdown: DropDown,
    pub enable_fallback: CheckButton,
    pub kdbx_path_entry: Entry,
    pub kdbx_password_entry: PasswordEntry,
    pub kdbx_enabled_switch: Switch,
    pub kdbx_save_password_check: CheckButton,
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
    pub password_row: adw::ActionRow,
    pub save_password_row: adw::ActionRow,
    pub key_file_row: adw::ActionRow,
    // Bitwarden widgets
    pub bitwarden_group: adw::PreferencesGroup,
    pub bitwarden_status_label: Label,
    pub bitwarden_unlock_button: Button,
    pub bitwarden_password_entry: PasswordEntry,
    pub bitwarden_save_password_check: CheckButton,
    pub bitwarden_save_to_keyring_check: CheckButton,
    pub bitwarden_use_api_key_check: Switch,
    pub bitwarden_client_id_entry: Entry,
    pub bitwarden_client_secret_entry: PasswordEntry,
    // 1Password widgets
    pub onepassword_group: adw::PreferencesGroup,
    pub onepassword_status_label: Label,
    pub onepassword_signin_button: Button,
}

/// Creates the secrets settings page using AdwPreferencesPage
#[allow(clippy::type_complexity)]
pub fn create_secrets_page() -> SecretsPageWidgets {
    let page = adw::PreferencesPage::builder()
        .title("Secrets")
        .icon_name("dialog-password-symbolic")
        .build();

    // === Secret Backend Group ===
    let backend_group = adw::PreferencesGroup::builder()
        .title("Secret Backend")
        .description("Choose how passwords are stored")
        .build();

    // Simplified: KeePassXC, libsecret, Bitwarden, 1Password
    let backend_strings = StringList::new(&["KeePassXC", "libsecret", "Bitwarden", "1Password"]);
    let secret_backend_dropdown = DropDown::builder()
        .model(&backend_strings)
        .selected(0)
        .valign(gtk4::Align::Center)
        .build();
    let backend_row = adw::ActionRow::builder()
        .title("Backend")
        .subtitle("Primary password storage method")
        .build();
    backend_row.add_suffix(&secret_backend_dropdown);
    backend_row.set_activatable_widget(Some(&secret_backend_dropdown));
    backend_group.add(&backend_row);

    // Version info row - shows version of selected backend
    let version_label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();
    let version_row = adw::ActionRow::builder().title("Version").build();
    version_row.add_suffix(&version_label);
    backend_group.add(&version_row);

    let enable_fallback = CheckButton::builder()
        .valign(gtk4::Align::Center)
        .active(true)
        .build();
    let fallback_row = adw::ActionRow::builder()
        .title("Enable fallback")
        .subtitle("Use libsecret if primary backend unavailable")
        .activatable_widget(&enable_fallback)
        .build();
    fallback_row.add_prefix(&enable_fallback);
    backend_group.add(&fallback_row);

    page.add(&backend_group);

    // Detect installed tools
    let keepassxc_installed = std::process::Command::new("which")
        .arg("keepassxc-cli")
        .output()
        .is_ok_and(|output| output.status.success());
    let keepassxc_version = if keepassxc_installed {
        get_cli_version("keepassxc-cli", &["--version"])
    } else {
        None
    };

    // Bitwarden CLI status - check multiple paths
    let bw_paths = ["bw", "/snap/bin/bw", "/usr/local/bin/bw"];
    let mut bitwarden_installed = false;
    let mut bitwarden_cmd = "bw".to_string();
    for path in &bw_paths {
        if std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            bitwarden_installed = true;
            bitwarden_cmd = (*path).to_string();
            break;
        }
    }
    // Also check via which
    if !bitwarden_installed {
        if let Ok(output) = std::process::Command::new("which").arg("bw").output() {
            if output.status.success() {
                bitwarden_installed = true;
                bitwarden_cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }
    let bitwarden_version = if bitwarden_installed {
        get_cli_version(&bitwarden_cmd, &["--version"])
    } else {
        None
    };

    // 1Password CLI status - check multiple paths
    let op_paths = ["op", "/usr/local/bin/op"];
    let mut onepassword_installed = false;
    let mut onepassword_cmd = "op".to_string();
    for path in &op_paths {
        if std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            onepassword_installed = true;
            onepassword_cmd = (*path).to_string();
            break;
        }
    }
    // Also check via which
    if !onepassword_installed {
        if let Ok(output) = std::process::Command::new("which").arg("op").output() {
            if output.status.success() {
                onepassword_installed = true;
                onepassword_cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }
    let onepassword_version = if onepassword_installed {
        get_cli_version(&onepassword_cmd, &["--version"])
    } else {
        None
    };

    // === Bitwarden Configuration Group ===
    let bitwarden_group = adw::PreferencesGroup::builder()
        .title("Bitwarden")
        .description("Configure Bitwarden CLI integration")
        .build();

    // Password entry for unlocking
    let bitwarden_password_entry = PasswordEntry::builder()
        .placeholder_text("Master password")
        .hexpand(true)
        .show_peek_icon(true)
        .valign(gtk4::Align::Center)
        .build();
    let bw_password_row = adw::ActionRow::builder()
        .title("Master Password")
        .subtitle("Required to unlock vault")
        .build();
    bw_password_row.add_suffix(&bitwarden_password_entry);
    bw_password_row.set_activatable_widget(Some(&bitwarden_password_entry));
    bitwarden_group.add(&bw_password_row);

    // Save password checkbox for Bitwarden (encrypted in settings file)
    let bitwarden_save_password_check = CheckButton::builder().valign(gtk4::Align::Center).build();
    let bw_save_password_row = adw::ActionRow::builder()
        .title("Save password")
        .subtitle("Encrypted storage (machine-specific)")
        .activatable_widget(&bitwarden_save_password_check)
        .build();
    bw_save_password_row.add_prefix(&bitwarden_save_password_check);
    bitwarden_group.add(&bw_save_password_row);

    // Save to system keyring checkbox (libsecret)
    let bitwarden_save_to_keyring_check =
        CheckButton::builder().valign(gtk4::Align::Center).build();
    let bw_save_to_keyring_row = adw::ActionRow::builder()
        .title("Save to system keyring")
        .subtitle("Store in GNOME Keyring / KDE Wallet (recommended)")
        .activatable_widget(&bitwarden_save_to_keyring_check)
        .build();
    bw_save_to_keyring_row.add_prefix(&bitwarden_save_to_keyring_check);
    bitwarden_group.add(&bw_save_to_keyring_row);

    // API Key authentication switch
    let bitwarden_use_api_key_check = Switch::builder().valign(gtk4::Align::Center).build();
    let bw_use_api_key_row = adw::ActionRow::builder()
        .title("Use API key authentication")
        .subtitle("For automation or 2FA methods not supported by CLI (FIDO2, Duo)")
        .build();
    bw_use_api_key_row.add_suffix(&bitwarden_use_api_key_check);
    bw_use_api_key_row.set_activatable_widget(Some(&bitwarden_use_api_key_check));
    bitwarden_group.add(&bw_use_api_key_row);

    // API Client ID entry
    let bitwarden_client_id_entry = Entry::builder()
        .placeholder_text("client_id")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let bw_client_id_row = adw::ActionRow::builder()
        .title("Client ID")
        .subtitle("From Bitwarden web vault → Settings → Security → Keys")
        .build();
    bw_client_id_row.add_suffix(&bitwarden_client_id_entry);
    bw_client_id_row.set_activatable_widget(Some(&bitwarden_client_id_entry));
    bitwarden_group.add(&bw_client_id_row);

    // API Client Secret entry
    let bitwarden_client_secret_entry = PasswordEntry::builder()
        .placeholder_text("client_secret")
        .hexpand(true)
        .show_peek_icon(true)
        .valign(gtk4::Align::Center)
        .build();
    let bw_client_secret_row = adw::ActionRow::builder()
        .title("Client Secret")
        .subtitle("Keep this secret safe")
        .build();
    bw_client_secret_row.add_suffix(&bitwarden_client_secret_entry);
    bw_client_secret_row.set_activatable_widget(Some(&bitwarden_client_secret_entry));
    bitwarden_group.add(&bw_client_secret_row);

    // Setup visibility for API key fields
    let bw_client_id_row_clone = bw_client_id_row.clone();
    let bw_client_secret_row_clone = bw_client_secret_row.clone();
    bitwarden_use_api_key_check.connect_state_set(move |_, state| {
        bw_client_id_row_clone.set_visible(state);
        bw_client_secret_row_clone.set_visible(state);
        glib::Propagation::Proceed
    });

    // Initial visibility - hide API key fields by default
    bw_client_id_row.set_visible(false);
    bw_client_secret_row.set_visible(false);

    let bitwarden_status_label = Label::builder()
        .label(if bitwarden_installed {
            "Checking status..."
        } else {
            "Not installed"
        })
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    let bitwarden_unlock_button = Button::builder()
        .label("Unlock")
        .valign(gtk4::Align::Center)
        .sensitive(bitwarden_installed)
        .tooltip_text("Unlock Bitwarden vault")
        .build();

    let bw_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    bw_status_box.append(&bitwarden_status_label);
    bw_status_box.append(&bitwarden_unlock_button);

    let bw_status_row = adw::ActionRow::builder()
        .title("Vault Status")
        .subtitle("Login with 'bw login' in terminal first")
        .build();
    bw_status_row.add_suffix(&bw_status_box);
    bitwarden_group.add(&bw_status_row);

    // Connect unlock button
    {
        let status_label = bitwarden_status_label.clone();
        let password_entry = bitwarden_password_entry.clone();
        let bw_cmd = bitwarden_cmd.clone();
        bitwarden_unlock_button.connect_clicked(move |button| {
            let password = password_entry.text();
            if password.is_empty() {
                update_status_label(&status_label, "Enter password", "warning");
                return;
            }

            button.set_sensitive(false);
            status_label.set_text("Unlocking...");
            update_status_label(&status_label, "Unlocking...", "dim-label");

            // Run unlock with password via environment variable
            let result = std::process::Command::new(&bw_cmd)
                .arg("unlock")
                .arg("--passwordenv")
                .arg("BW_PASSWORD")
                .env("BW_PASSWORD", password.as_str())
                .output();

            match result {
                Ok(output) => {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        // Extract session key from output
                        if let Some(session_key) = extract_session_key(&stdout) {
                            // Set session key in environment for future commands
                            std::env::set_var("BW_SESSION", &session_key);
                            update_status_label(&status_label, "Unlocked", "success");
                            password_entry.set_text("");
                        } else {
                            update_status_label(&status_label, "Unlocked", "success");
                        }
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let msg = if stderr.contains("Invalid master password") {
                            "Invalid password"
                        } else if stderr.contains("not logged in") {
                            "Not logged in"
                        } else {
                            "Unlock failed"
                        };
                        update_status_label(&status_label, msg, "error");
                    }
                }
                Err(_) => {
                    update_status_label(&status_label, "Command failed", "error");
                }
            }

            button.set_sensitive(true);
        });
    }

    // Check Bitwarden status synchronously (runs in idle callback to not block UI)
    if bitwarden_installed {
        let status_label = bitwarden_status_label.clone();
        let bw_cmd_clone = bitwarden_cmd.clone();
        glib::idle_add_local_once(move || {
            let status = check_bitwarden_status_sync(&bw_cmd_clone);
            status_label.set_text(&status.0);
            status_label.remove_css_class("dim-label");
            status_label.remove_css_class("success");
            status_label.remove_css_class("warning");
            status_label.remove_css_class("error");
            status_label.add_css_class(status.1);
        });
    }

    page.add(&bitwarden_group);

    // === 1Password Configuration Group ===
    let onepassword_group = adw::PreferencesGroup::builder()
        .title("1Password")
        .description("Configure 1Password CLI integration")
        .build();

    let onepassword_status_label = Label::builder()
        .label(if onepassword_installed {
            "Checking status..."
        } else {
            "Not installed"
        })
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    let onepassword_signin_button = Button::builder()
        .label("Sign In")
        .valign(gtk4::Align::Center)
        .sensitive(onepassword_installed)
        .tooltip_text("Sign in to 1Password (opens terminal)")
        .build();

    let op_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    op_status_box.append(&onepassword_status_label);
    op_status_box.append(&onepassword_signin_button);

    let op_status_row = adw::ActionRow::builder()
        .title("Account Status")
        .subtitle("Sign in with 'op signin' in terminal or use biometric unlock")
        .build();
    op_status_row.add_suffix(&op_status_box);
    onepassword_group.add(&op_status_row);

    // Connect signin button - opens terminal for interactive signin
    {
        let status_label = onepassword_status_label.clone();
        let op_cmd = onepassword_cmd.clone();
        onepassword_signin_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            update_status_label(&status_label, "Opening terminal...", "dim-label");

            // Try to open a terminal with op signin
            // This requires user interaction for biometric or password
            let xfce_cmd = format!("{op_cmd} signin");
            let terminal_cmds: [(&str, Vec<&str>); 4] = [
                ("gnome-terminal", vec!["--", &op_cmd, "signin"]),
                ("konsole", vec!["-e", &op_cmd, "signin"]),
                ("xfce4-terminal", vec!["-e", &xfce_cmd]),
                ("xterm", vec!["-e", &op_cmd, "signin"]),
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
                    update_status_label(&status_label, "Check terminal", "warning");
                    break;
                }
            }

            if !launched {
                update_status_label(&status_label, "No terminal found", "error");
            }

            button.set_sensitive(true);
        });
    }

    // Check 1Password status synchronously
    if onepassword_installed {
        let status_label = onepassword_status_label.clone();
        let op_cmd_clone = onepassword_cmd.clone();
        glib::idle_add_local_once(move || {
            let status = check_onepassword_status_sync(&op_cmd_clone);
            status_label.set_text(&status.0);
            status_label.remove_css_class("dim-label");
            status_label.remove_css_class("success");
            status_label.remove_css_class("warning");
            status_label.remove_css_class("error");
            status_label.add_css_class(status.1);
        });
    }

    page.add(&onepassword_group);

    // === KeePass Database Group ===
    let kdbx_group = adw::PreferencesGroup::builder()
        .title("KeePass Database")
        .description("Configure KDBX file integration (works with KeePassXC, GNOME Secrets, etc.)")
        .build();

    let kdbx_enabled_switch = Switch::builder().valign(gtk4::Align::Center).build();
    let kdbx_enabled_row = adw::ActionRow::builder()
        .title("KDBX Integration")
        .subtitle("Enable direct database access")
        .build();
    kdbx_enabled_row.add_suffix(&kdbx_enabled_switch);
    kdbx_enabled_row.set_activatable_widget(Some(&kdbx_enabled_switch));
    kdbx_group.add(&kdbx_enabled_row);

    // Database path with browse button
    let kdbx_path_entry = Entry::builder()
        .placeholder_text("Select .kdbx file")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let kdbx_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text("Browse for database file")
        .build();
    let kdbx_path_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    kdbx_path_box.append(&kdbx_path_entry);
    kdbx_path_box.append(&kdbx_browse_button);

    let kdbx_path_row = adw::ActionRow::builder().title("Database File").build();
    kdbx_path_row.add_suffix(&kdbx_path_box);
    kdbx_group.add(&kdbx_path_row);

    page.add(&kdbx_group);

    // === Authentication Group ===
    let auth_group = adw::PreferencesGroup::builder()
        .title("Authentication")
        .description("Database unlock methods")
        .build();

    // Use password switch
    let kdbx_use_password_check = Switch::builder()
        .active(true)
        .valign(gtk4::Align::Center)
        .build();
    let use_password_row = adw::ActionRow::builder().title("Use password").build();
    use_password_row.add_suffix(&kdbx_use_password_check);
    use_password_row.set_activatable_widget(Some(&kdbx_use_password_check));
    auth_group.add(&use_password_row);

    // Password entry
    let kdbx_password_entry = PasswordEntry::builder()
        .placeholder_text("Database password")
        .hexpand(true)
        .show_peek_icon(true)
        .valign(gtk4::Align::Center)
        .build();
    let password_row = adw::ActionRow::builder().title("Password").build();
    password_row.add_suffix(&kdbx_password_entry);
    password_row.set_activatable_widget(Some(&kdbx_password_entry));
    auth_group.add(&password_row);

    // Save password checkbox
    let kdbx_save_password_check = CheckButton::builder().valign(gtk4::Align::Center).build();
    let save_password_row = adw::ActionRow::builder()
        .title("Save password")
        .subtitle("Encrypted storage")
        .activatable_widget(&kdbx_save_password_check)
        .build();
    save_password_row.add_prefix(&kdbx_save_password_check);
    auth_group.add(&save_password_row);

    // Use key file switch
    let kdbx_use_key_file_check = Switch::builder().valign(gtk4::Align::Center).build();
    let use_key_file_row = adw::ActionRow::builder().title("Use key file").build();
    use_key_file_row.add_suffix(&kdbx_use_key_file_check);
    use_key_file_row.set_activatable_widget(Some(&kdbx_use_key_file_check));
    auth_group.add(&use_key_file_row);

    // Key file path with browse button
    let kdbx_key_file_entry = Entry::builder()
        .placeholder_text("Select .keyx or .key file")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let kdbx_key_file_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text("Browse for key file")
        .build();
    let key_file_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    key_file_box.append(&kdbx_key_file_entry);
    key_file_box.append(&kdbx_key_file_browse_button);

    let key_file_row = adw::ActionRow::builder().title("Key File").build();
    key_file_row.add_suffix(&key_file_box);
    auth_group.add(&key_file_row);

    page.add(&auth_group);

    // === Status Group ===
    let status_group = adw::PreferencesGroup::builder()
        .title("KDBX Status")
        .build();

    // Check connection button
    let kdbx_check_button = Button::builder()
        .label("Check")
        .valign(gtk4::Align::Center)
        .tooltip_text("Test database connection")
        .build();

    let kdbx_status_label = Label::builder()
        .label("Not connected")
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    let status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    status_box.append(&kdbx_status_label);
    status_box.append(&kdbx_check_button);

    let status_row = adw::ActionRow::builder().title("Connection Status").build();
    status_row.add_suffix(&status_box);
    status_group.add(&status_row);

    page.add(&status_group);

    // Setup visibility connections for password fields
    let password_row_clone = password_row.clone();
    let save_password_row_clone = save_password_row.clone();
    kdbx_use_password_check.connect_state_set(move |_, state| {
        password_row_clone.set_visible(state);
        save_password_row_clone.set_visible(state);
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
    kdbx_enabled_switch.connect_state_set(move |_, state| {
        auth_group_clone.set_visible(state);
        status_group_clone.set_visible(state);
        glib::Propagation::Proceed
    });

    // Setup visibility for Bitwarden and 1Password groups based on backend selection
    // Indices: 0=KeePassXC, 1=libsecret, 2=Bitwarden, 3=1Password
    let bitwarden_group_clone = bitwarden_group.clone();
    let onepassword_group_clone = onepassword_group.clone();
    let kdbx_group_clone = kdbx_group.clone();
    let auth_group_clone2 = auth_group.clone();
    let status_group_clone2 = status_group.clone();
    let kdbx_enabled_switch_clone = kdbx_enabled_switch.clone();
    let version_label_clone = version_label.clone();
    let version_row_clone = version_row.clone();
    let keepassxc_version_clone = keepassxc_version.clone();
    let bitwarden_version_clone = bitwarden_version.clone();
    let onepassword_version_clone = onepassword_version.clone();
    secret_backend_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        // Show Bitwarden group only when Bitwarden is selected (index 2)
        bitwarden_group_clone.set_visible(selected == 2);
        // Show 1Password group only when 1Password is selected (index 3)
        onepassword_group_clone.set_visible(selected == 3);
        // Show KDBX groups only when KeePassXC is selected (index 0)
        let show_kdbx = selected == 0;
        kdbx_group_clone.set_visible(show_kdbx);
        // Auth and status groups depend on both backend selection and kdbx_enabled
        let kdbx_enabled = kdbx_enabled_switch_clone.is_active();
        auth_group_clone2.set_visible(show_kdbx && kdbx_enabled);
        status_group_clone2.set_visible(show_kdbx && kdbx_enabled);

        // Update version label based on selected backend
        match selected {
            0 => {
                // KeePassXC
                version_row_clone.set_visible(true);
                if let Some(ref ver) = keepassxc_version_clone {
                    version_label_clone.set_text(&format!("v{ver}"));
                    version_label_clone.remove_css_class("error");
                    version_label_clone.add_css_class("success");
                } else {
                    version_label_clone.set_text("Not installed");
                    version_label_clone.remove_css_class("success");
                    version_label_clone.add_css_class("error");
                }
            }
            1 => {
                // libsecret - don't show version
                version_row_clone.set_visible(false);
            }
            2 => {
                // Bitwarden
                version_row_clone.set_visible(true);
                if let Some(ref ver) = bitwarden_version_clone {
                    version_label_clone.set_text(&format!("v{ver}"));
                    version_label_clone.remove_css_class("error");
                    version_label_clone.add_css_class("success");
                } else {
                    version_label_clone.set_text("Not installed");
                    version_label_clone.remove_css_class("success");
                    version_label_clone.add_css_class("error");
                }
            }
            3 => {
                // 1Password
                version_row_clone.set_visible(true);
                if let Some(ref ver) = onepassword_version_clone {
                    version_label_clone.set_text(&format!("v{ver}"));
                    version_label_clone.remove_css_class("error");
                    version_label_clone.add_css_class("success");
                } else {
                    version_label_clone.set_text("Not installed");
                    version_label_clone.remove_css_class("success");
                    version_label_clone.add_css_class("error");
                }
            }
            _ => {
                version_row_clone.set_visible(false);
            }
        }
    });

    // Initial visibility based on default states (KeePassXC selected by default)
    key_file_row.set_visible(false);
    password_row.set_visible(true);
    save_password_row.set_visible(true);
    auth_group.set_visible(false);
    status_group.set_visible(false);
    bitwarden_group.set_visible(false);
    onepassword_group.set_visible(false);

    // Set initial version display for KeePassXC (default selection)
    if let Some(ref ver) = keepassxc_version {
        version_label.set_text(&format!("v{ver}"));
        version_label.add_css_class("success");
    } else {
        version_label.set_text("Not installed");
        version_label.add_css_class("error");
    }

    // Setup browse button for database file
    let kdbx_path_entry_clone = kdbx_path_entry.clone();
    kdbx_browse_button.connect_clicked(move |button| {
        let entry = kdbx_path_entry_clone.clone();
        let dialog = FileDialog::builder()
            .title("Select KeePass Database")
            .modal(true)
            .build();

        let filter = FileFilter::new();
        filter.add_pattern("*.kdbx");
        filter.set_name(Some("KeePass Database (*.kdbx)"));

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
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        entry.set_text(&path.display().to_string());
                    }
                }
            },
        );
    });

    // Setup browse button for key file
    let kdbx_key_file_entry_clone = kdbx_key_file_entry.clone();
    kdbx_key_file_browse_button.connect_clicked(move |button| {
        let entry = kdbx_key_file_entry_clone.clone();
        let dialog = FileDialog::builder()
            .title("Select Key File")
            .modal(true)
            .build();

        let filter = FileFilter::new();
        filter.add_pattern("*.keyx");
        filter.add_pattern("*.key");
        filter.set_name(Some("Key Files (*.keyx, *.key)"));

        let all_filter = FileFilter::new();
        all_filter.add_pattern("*");
        all_filter.set_name(Some("All Files"));

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
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        entry.set_text(&path.display().to_string());
                    }
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
            update_status_label(&kdbx_status_label_check, "No database selected", "warning");
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

        let result = rustconn_core::secret::KeePassStatus::verify_kdbx_credentials(
            kdbx_path,
            password.as_deref(),
            key_file.as_deref(),
        );

        match result {
            Ok(()) => {
                update_status_label(&kdbx_status_label_check, "Connected", "success");
            }
            Err(e) => {
                update_status_label(&kdbx_status_label_check, &e, "error");
            }
        }
    });

    let keepassxc_status_container = GtkBox::new(Orientation::Vertical, 6);

    SecretsPageWidgets {
        page,
        secret_backend_dropdown,
        enable_fallback,
        kdbx_path_entry,
        kdbx_password_entry,
        kdbx_enabled_switch,
        kdbx_save_password_check,
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
        save_password_row,
        key_file_row,
        bitwarden_group,
        bitwarden_status_label,
        bitwarden_unlock_button,
        bitwarden_password_entry,
        bitwarden_save_password_check,
        bitwarden_save_to_keyring_check,
        bitwarden_use_api_key_check,
        bitwarden_client_id_entry,
        bitwarden_client_secret_entry,
        onepassword_group,
        onepassword_status_label,
        onepassword_signin_button,
    }
}

/// Gets CLI version from command output
fn get_cli_version(command: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(command)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let output = String::from_utf8_lossy(&o.stdout);
            parse_version(&output)
        })
}

/// Parses version from output string
fn parse_version(output: &str) -> Option<String> {
    // Try to find version pattern like "1.2.3" or "v1.2.3"
    let re = regex::Regex::new(r"v?(\d+\.\d+(?:\.\d+)?)").ok()?;
    re.captures(output)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Checks Bitwarden vault status synchronously
fn check_bitwarden_status_sync(bw_cmd: &str) -> (String, &'static str) {
    let output = std::process::Command::new(bw_cmd).arg("status").output();

    match output {
        Ok(o) if o.status.success() => {
            let status_str = String::from_utf8_lossy(&o.stdout);
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&status_str) {
                if let Some(status_val) = status.get("status").and_then(|v| v.as_str()) {
                    return match status_val {
                        "unlocked" => ("Unlocked".to_string(), "success"),
                        "locked" => ("Locked".to_string(), "warning"),
                        "unauthenticated" => ("Not logged in".to_string(), "error"),
                        _ => (format!("Status: {status_val}"), "dim-label"),
                    };
                }
            }
            ("Unknown".to_string(), "dim-label")
        }
        _ => ("Error checking status".to_string(), "error"),
    }
}

/// Checks 1Password account status synchronously
fn check_onepassword_status_sync(op_cmd: &str) -> (String, &'static str) {
    let output = std::process::Command::new(op_cmd)
        .args(["whoami", "--format", "json"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if let Ok(whoami) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if let Some(email) = whoami.get("email").and_then(|v| v.as_str()) {
                    return (format!("Signed in: {email}"), "success");
                }
            }
            ("Signed in".to_string(), "success")
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("not signed in") || stderr.contains("sign in") {
                ("Not signed in".to_string(), "error")
            } else if stderr.contains("session expired") {
                ("Session expired".to_string(), "warning")
            } else {
                ("Not signed in".to_string(), "error")
            }
        }
        Err(_) => ("Error checking status".to_string(), "error"),
    }
}

/// Extracts session key from `bw unlock` output
fn extract_session_key(output: &str) -> Option<String> {
    // Output format: export BW_SESSION="<session_key>"
    // or: $ export BW_SESSION="<session_key>"
    for line in output.lines() {
        if line.contains("BW_SESSION=") {
            // Extract the value between quotes
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if end > start {
                        return Some(line[start + 1..end].to_string());
                    }
                }
            }
            // Try without quotes (BW_SESSION=value)
            if let Some(pos) = line.find("BW_SESSION=") {
                let value_start = pos + "BW_SESSION=".len();
                let value = line[value_start..].trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Updates the status label with text and CSS class
fn update_status_label(label: &Label, text: &str, css_class: &str) {
    label.set_text(text);
    label.remove_css_class("success");
    label.remove_css_class("warning");
    label.remove_css_class("error");
    label.remove_css_class("dim-label");
    label.add_css_class(css_class);
}

/// Loads secret settings into UI controls
#[allow(clippy::too_many_arguments)]
pub fn load_secret_settings(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    // Indices: 0=KeePassXC, 1=libsecret, 2=Bitwarden, 3=1Password
    let backend_index = match settings.preferred_backend {
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => 0,
        SecretBackendType::LibSecret => 1,
        SecretBackendType::Bitwarden => 2,
        SecretBackendType::OnePassword => 3,
    };
    widgets.secret_backend_dropdown.set_selected(backend_index);
    widgets.enable_fallback.set_active(settings.enable_fallback);
    widgets
        .kdbx_enabled_switch
        .set_active(settings.kdbx_enabled);

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
    widgets
        .kdbx_save_password_check
        .set_active(settings.kdbx_password_encrypted.is_some());

    // Load Bitwarden save password state
    widgets
        .bitwarden_save_password_check
        .set_active(settings.bitwarden_password_encrypted.is_some());

    // Load Bitwarden keyring and API key settings
    widgets
        .bitwarden_save_to_keyring_check
        .set_active(settings.bitwarden_save_to_keyring);
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

    // Update visibility based on loaded settings
    // Show KDBX groups only when KeePassXC is selected (index 0)
    let show_kdbx = backend_index == 0;
    widgets.kdbx_group.set_visible(show_kdbx);
    widgets
        .auth_group
        .set_visible(show_kdbx && settings.kdbx_enabled);
    widgets
        .status_group
        .set_visible(show_kdbx && settings.kdbx_enabled);
    // Show Bitwarden group only when Bitwarden is selected (index 2)
    widgets.bitwarden_group.set_visible(backend_index == 2);
    // Show 1Password group only when 1Password is selected (index 3)
    widgets.onepassword_group.set_visible(backend_index == 3);
    widgets.password_row.set_visible(settings.kdbx_use_password);
    widgets
        .save_password_row
        .set_visible(settings.kdbx_use_password);
    widgets.key_file_row.set_visible(settings.kdbx_use_key_file);

    let status_text = if settings.kdbx_enabled {
        if settings.kdbx_path.is_some() {
            "Configured"
        } else {
            "Database path required"
        }
    } else {
        "Disabled"
    };

    widgets.kdbx_status_label.set_text(status_text);

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
}

/// Collects secret settings from UI controls
pub fn collect_secret_settings(
    widgets: &SecretsPageWidgets,
    settings: &Rc<RefCell<rustconn_core::config::AppSettings>>,
) -> SecretSettings {
    // Indices: 0=KeePassXC, 1=libsecret, 2=Bitwarden, 3=1Password
    let preferred_backend = match widgets.secret_backend_dropdown.selected() {
        0 => SecretBackendType::KeePassXc,
        1 => SecretBackendType::LibSecret,
        2 => SecretBackendType::Bitwarden,
        3 => SecretBackendType::OnePassword,
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

    let (kdbx_password, kdbx_password_encrypted) = if widgets.kdbx_save_password_check.is_active() {
        let password_text = widgets.kdbx_password_entry.text();
        if password_text.is_empty() {
            (None, None)
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
    } else {
        (None, None)
    };

    // Collect Bitwarden password if save is enabled
    let (bitwarden_password, bitwarden_password_encrypted) =
        if widgets.bitwarden_save_password_check.is_active() {
            let password_text = widgets.bitwarden_password_entry.text();
            if password_text.is_empty() {
                // Keep existing encrypted password if field is empty but save is checked
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
                // Mark for encryption (will be encrypted when settings are saved)
                let encrypted = settings
                    .borrow()
                    .secrets
                    .bitwarden_password_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_password_placeholder".to_string()));
                (Some(password), encrypted)
            }
        } else {
            (None, None)
        };

    // Collect Bitwarden API key settings
    let bitwarden_use_api_key = widgets.bitwarden_use_api_key_check.is_active();
    let bitwarden_save_to_keyring = widgets.bitwarden_save_to_keyring_check.is_active();

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

    SecretSettings {
        preferred_backend,
        enable_fallback: widgets.enable_fallback.is_active(),
        kdbx_path,
        kdbx_enabled: widgets.kdbx_enabled_switch.is_active(),
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
    }
}
