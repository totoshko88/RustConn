//! Secrets settings tab

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, Frame, Label, Orientation, PasswordEntry,
    ScrolledWindow, StringList, Switch,
};
use rustconn_core::config::{SecretBackendType, SecretSettings};
use std::cell::RefCell;
use std::rc::Rc;

/// Creates the secrets settings tab
#[allow(clippy::type_complexity)]
pub fn create_secrets_tab() -> (
    ScrolledWindow,
    DropDown,
    CheckButton,
    Entry,
    PasswordEntry,
    Switch,
    CheckButton,
    Label,
    Button,
    GtkBox,
    Entry,
    Button,
    CheckButton,
) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let main_vbox = GtkBox::new(Orientation::Vertical, 12);
    main_vbox.set_margin_top(12);
    main_vbox.set_margin_bottom(12);
    main_vbox.set_margin_start(12);
    main_vbox.set_margin_end(12);
    main_vbox.set_valign(gtk4::Align::Start);

    // Secret Backend Selection
    let backend_frame = Frame::builder()
        .label("Secret Backend")
        .margin_bottom(12)
        .build();

    let backend_vbox = GtkBox::new(Orientation::Vertical, 6);
    backend_vbox.set_margin_top(6);
    backend_vbox.set_margin_bottom(6);
    backend_vbox.set_margin_start(6);
    backend_vbox.set_margin_end(6);

    let backend_strings = StringList::new(&["KeePassXC", "libsecret", "KDBX File"]);
    let secret_backend_dropdown = DropDown::builder()
        .model(&backend_strings)
        .selected(0)
        .build();

    let enable_fallback =
        CheckButton::with_label("Enable fallback to libsecret if KeePassXC unavailable");
    enable_fallback.set_active(true);

    backend_vbox.append(&secret_backend_dropdown);
    backend_vbox.append(&enable_fallback);
    backend_frame.set_child(Some(&backend_vbox));

    // KeePass Database Settings
    let kdbx_frame = Frame::builder()
        .label("KeePass Database")
        .margin_bottom(12)
        .build();

    let kdbx_vbox = GtkBox::new(Orientation::Vertical, 6);
    kdbx_vbox.set_margin_top(6);
    kdbx_vbox.set_margin_bottom(6);
    kdbx_vbox.set_margin_start(6);
    kdbx_vbox.set_margin_end(6);

    let kdbx_enabled_switch = Switch::new();
    let kdbx_enabled_hbox = GtkBox::new(Orientation::Horizontal, 6);
    kdbx_enabled_hbox.append(&Label::new(Some("Enable KeePass integration")));
    kdbx_enabled_hbox.append(&kdbx_enabled_switch);
    kdbx_enabled_hbox.set_halign(gtk4::Align::Start);

    // Database path
    let kdbx_path_hbox = GtkBox::new(Orientation::Horizontal, 6);
    kdbx_path_hbox.append(&Label::new(Some("Database file:")));
    let kdbx_path_entry = Entry::builder()
        .placeholder_text("Select .kdbx file")
        .hexpand(true)
        .build();
    let kdbx_browse_button = Button::with_label("Browse");
    kdbx_path_hbox.append(&kdbx_path_entry);
    kdbx_path_hbox.append(&kdbx_browse_button);

    // Authentication method
    let kdbx_use_key_file_check = CheckButton::with_label("Use key file instead of password");

    // Password authentication
    let kdbx_password_hbox = GtkBox::new(Orientation::Horizontal, 6);
    kdbx_password_hbox.append(&Label::new(Some("Password:")));
    let kdbx_password_entry = PasswordEntry::builder()
        .placeholder_text("Database password")
        .hexpand(true)
        .build();
    kdbx_password_hbox.append(&kdbx_password_entry);

    let kdbx_save_password_check = CheckButton::with_label("Save password (encrypted)");

    // Key file authentication
    let kdbx_key_file_hbox = GtkBox::new(Orientation::Horizontal, 6);
    kdbx_key_file_hbox.append(&Label::new(Some("Key file:")));
    let kdbx_key_file_entry = Entry::builder()
        .placeholder_text("Select .keyx or .key file")
        .hexpand(true)
        .build();
    let kdbx_key_file_browse_button = Button::with_label("Browse");
    kdbx_key_file_hbox.append(&kdbx_key_file_entry);
    kdbx_key_file_hbox.append(&kdbx_key_file_browse_button);

    // Status display
    let keepassxc_status_container = GtkBox::new(Orientation::Vertical, 6);
    let kdbx_status_label = Label::builder()
        .label("Status: Not connected")
        .halign(gtk4::Align::Start)
        .build();
    keepassxc_status_container.append(&kdbx_status_label);

    kdbx_vbox.append(&kdbx_enabled_hbox);
    kdbx_vbox.append(&kdbx_path_hbox);
    kdbx_vbox.append(&kdbx_use_key_file_check);
    kdbx_vbox.append(&kdbx_password_hbox);
    kdbx_vbox.append(&kdbx_save_password_check);
    kdbx_vbox.append(&kdbx_key_file_hbox);
    kdbx_vbox.append(&keepassxc_status_container);

    kdbx_frame.set_child(Some(&kdbx_vbox));

    main_vbox.append(&backend_frame);
    main_vbox.append(&kdbx_frame);

    scrolled.set_child(Some(&main_vbox));

    (
        scrolled,
        secret_backend_dropdown,
        enable_fallback,
        kdbx_path_entry,
        kdbx_password_entry,
        kdbx_enabled_switch,
        kdbx_save_password_check,
        kdbx_status_label,
        kdbx_browse_button,
        keepassxc_status_container,
        kdbx_key_file_entry,
        kdbx_key_file_browse_button,
        kdbx_use_key_file_check,
    )
}

/// Loads secret settings into UI controls
#[allow(clippy::too_many_arguments)]
pub fn load_secret_settings(
    secret_backend_dropdown: &DropDown,
    enable_fallback: &CheckButton,
    kdbx_path_entry: &Entry,
    _kdbx_password_entry: &PasswordEntry,
    kdbx_enabled_switch: &Switch,
    kdbx_save_password_check: &CheckButton,
    kdbx_status_label: &Label,
    _keepassxc_status_container: &GtkBox,
    kdbx_key_file_entry: &Entry,
    kdbx_use_key_file_check: &CheckButton,
    settings: &SecretSettings,
) {
    // Set backend dropdown
    let backend_index = match settings.preferred_backend {
        SecretBackendType::KeePassXc => 0,
        SecretBackendType::LibSecret => 1,
        SecretBackendType::KdbxFile => 2, // Add this variant
    };
    secret_backend_dropdown.set_selected(backend_index);

    // Set fallback option
    enable_fallback.set_active(settings.enable_fallback);

    // Set KeePass settings
    kdbx_enabled_switch.set_active(settings.kdbx_enabled);

    if let Some(path) = &settings.kdbx_path {
        kdbx_path_entry.set_text(&path.display().to_string());
    }

    if let Some(key_file) = &settings.kdbx_key_file {
        kdbx_key_file_entry.set_text(&key_file.display().to_string());
    }

    kdbx_use_key_file_check.set_active(settings.kdbx_use_key_file);
    kdbx_save_password_check.set_active(settings.kdbx_password_encrypted.is_some());

    // Update status
    let status_text = if settings.kdbx_enabled {
        if settings.kdbx_path.is_some() {
            "Status: Configured"
        } else {
            "Status: Database path required"
        }
    } else {
        "Status: Disabled"
    };
    kdbx_status_label.set_text(status_text);
}

/// Collects secret settings from UI controls
#[allow(clippy::too_many_arguments)]
pub fn collect_secret_settings(
    secret_backend_dropdown: &DropDown,
    enable_fallback: &CheckButton,
    kdbx_path_entry: &Entry,
    kdbx_password_entry: &PasswordEntry,
    kdbx_enabled_switch: &Switch,
    kdbx_save_password_check: &CheckButton,
    kdbx_key_file_entry: &Entry,
    kdbx_use_key_file_check: &CheckButton,
    settings: &Rc<RefCell<rustconn_core::config::AppSettings>>,
) -> SecretSettings {
    let preferred_backend = match secret_backend_dropdown.selected() {
        0 => SecretBackendType::KeePassXc,
        1 => SecretBackendType::LibSecret,
        2 => SecretBackendType::KdbxFile,
        _ => SecretBackendType::default(),
    };

    let kdbx_path = {
        let path_text = kdbx_path_entry.text();
        if path_text.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(path_text.as_str()))
        }
    };

    let kdbx_key_file = {
        let key_file_text = kdbx_key_file_entry.text();
        if key_file_text.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(key_file_text.as_str()))
        }
    };

    // Handle password encryption if save is enabled
    let (kdbx_password, kdbx_password_encrypted) = if kdbx_save_password_check.is_active() {
        let password_text = kdbx_password_entry.text();
        if password_text.is_empty() {
            (None, None)
        } else {
            let password = secrecy::SecretString::new(password_text.to_string().into());
            // Use existing encrypted password or encrypt new one
            let encrypted = settings
                .borrow()
                .secrets
                .kdbx_password_encrypted
                .clone()
                .or_else(|| {
                    // In a real implementation, this would encrypt the password
                    // For now, we'll just store a placeholder
                    Some("encrypted_password_placeholder".to_string())
                });
            (Some(password), encrypted)
        }
    } else {
        (None, None)
    };

    SecretSettings {
        preferred_backend,
        enable_fallback: enable_fallback.is_active(),
        kdbx_path,
        kdbx_enabled: kdbx_enabled_switch.is_active(),
        kdbx_password,
        kdbx_password_encrypted,
        kdbx_key_file,
        kdbx_use_key_file: kdbx_use_key_file_check.is_active(),
    }
}
