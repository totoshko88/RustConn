//! Settings dialog for application preferences
//!
//! Provides a GTK4 dialog for configuring terminal settings, logging options,
//! and secret storage preferences.
//!
//! Updated for GTK 4.10+ compatibility using `DropDown` instead of `ComboBoxText`
//! and Window instead of Dialog.

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, FileDialog, FileFilter, Frame, Grid,
    HeaderBar, Label, ListBox, ListBoxRow, Notebook, Orientation, PasswordEntry, ScrolledWindow,
    SpinButton, Spinner, StringList, Switch, Window,
};
use rustconn_core::config::{
    AppSettings, LoggingSettings, SecretBackendType, SecretSettings, TerminalSettings, UiSettings,
};
use rustconn_core::secret::KeePassStatus;
use rustconn_core::ssh_agent::{AgentKey, SshAgentManager};
use rustconn_core::{detect_rdp_client, detect_ssh_client, detect_vnc_client, ClientInfo};
use secrecy::SecretString;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Settings dialog for application preferences
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct SettingsDialog {
    window: Window,
    save_button: Button,
    // Terminal settings
    font_family_entry: Entry,
    font_size_spin: SpinButton,
    scrollback_spin: SpinButton,
    // Logging settings
    logging_enabled_switch: Switch,
    log_dir_entry: Entry,
    retention_spin: SpinButton,
    open_logs_button: Button,
    log_activity_check: CheckButton,
    log_input_check: CheckButton,
    log_output_check: CheckButton,
    // Secret settings
    secret_backend_dropdown: DropDown,
    enable_fallback: CheckButton,
    // KeePass settings
    kdbx_path_entry: Entry,
    kdbx_password_entry: PasswordEntry,
    kdbx_enabled_switch: Switch,
    kdbx_save_password_check: CheckButton,
    kdbx_status_label: Label,
    kdbx_browse_button: Button,
    keepassxc_status_container: GtkBox,
    // KeePass key file settings
    kdbx_key_file_entry: Entry,
    kdbx_key_file_browse_button: Button,
    kdbx_use_key_file_check: CheckButton,
    // UI settings
    remember_geometry: CheckButton,
    enable_tray_icon: CheckButton,
    minimize_to_tray: CheckButton,
    // Session restore settings
    session_restore_enabled: CheckButton,
    prompt_on_restore: CheckButton,
    max_age_spin: SpinButton,
    // SSH Agent settings
    ssh_agent_status_label: Label,
    ssh_agent_socket_label: Label,
    ssh_agent_start_button: Button,
    ssh_agent_keys_list: ListBox,
    ssh_agent_add_key_button: Button,
    ssh_agent_loading_spinner: Spinner,
    ssh_agent_error_label: Label,
    ssh_agent_refresh_button: Button,
    ssh_agent_manager: Rc<RefCell<SshAgentManager>>,
    // Current settings
    settings: Rc<RefCell<AppSettings>>,
    // Callback
    on_save: super::SettingsCallback,
}

impl SettingsDialog {
    /// Creates a new settings dialog
    #[must_use]
    pub fn new(parent: Option<&Window>) -> Self {
        // Create window instead of deprecated Dialog
        let window = Window::builder()
            .title("Settings")
            .modal(true)
            .default_width(750)
            .default_height(600)
            .resizable(true)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create header bar (no Cancel button - window X is sufficient)
        let header = HeaderBar::new();
        let save_btn = Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&save_btn);
        window.set_titlebar(Some(&header));

        // Create main content area
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Create notebook for tabs - make it expand to fill available space
        let notebook = Notebook::new();
        notebook.set_vexpand(true);
        content.append(&notebook);
        window.set_child(Some(&content));

        // === Terminal Tab ===
        let (terminal_page, font_family_entry, font_size_spin, scrollback_spin) =
            Self::create_terminal_tab();
        notebook.append_page(&terminal_page, Some(&Label::new(Some("Terminal"))));

        // === Logging Tab ===
        let (
            logging_page,
            logging_enabled_switch,
            log_dir_entry,
            retention_spin,
            open_logs_button,
            log_activity_check,
            log_input_check,
            log_output_check,
        ) = Self::create_logging_tab();
        notebook.append_page(&logging_page, Some(&Label::new(Some("Logging"))));

        // === Secrets Tab ===
        let (
            secrets_page,
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
        ) = Self::create_secrets_tab();
        notebook.append_page(&secrets_page, Some(&Label::new(Some("Secrets"))));

        // === UI Tab ===
        let (
            ui_page,
            remember_geometry,
            enable_tray_icon,
            minimize_to_tray,
            session_restore_enabled,
            prompt_on_restore,
            max_age_spin,
        ) = Self::create_ui_tab();
        notebook.append_page(&ui_page, Some(&Label::new(Some("Interface"))));

        // === Clients Tab ===
        let clients_page = Self::create_clients_tab();
        notebook.append_page(&clients_page, Some(&Label::new(Some("Clients"))));

        // === SSH Agent Tab ===
        let (
            ssh_agent_page,
            ssh_agent_status_label,
            ssh_agent_socket_label,
            ssh_agent_start_button,
            ssh_agent_keys_list,
            ssh_agent_add_key_button,
            ssh_agent_loading_spinner,
            ssh_agent_error_label,
            ssh_agent_refresh_button,
        ) = Self::create_ssh_agent_tab();
        notebook.append_page(&ssh_agent_page, Some(&Label::new(Some("SSH Agent"))));

        let on_save: super::SettingsCallback = Rc::new(RefCell::new(None));
        let settings: Rc<RefCell<AppSettings>> = Rc::new(RefCell::new(AppSettings::default()));

        // Connect save button once in constructor (not in run())
        {
            let window_clone = window.clone();
            let on_save_clone = on_save.clone();
            let font_family_entry_clone = font_family_entry.clone();
            let font_size_spin_clone = font_size_spin.clone();
            let scrollback_spin_clone = scrollback_spin.clone();
            let logging_enabled_switch_clone = logging_enabled_switch.clone();
            let log_dir_entry_clone = log_dir_entry.clone();
            let retention_spin_clone = retention_spin.clone();
            let log_activity_check_clone = log_activity_check.clone();
            let log_input_check_clone = log_input_check.clone();
            let log_output_check_clone = log_output_check.clone();
            let secret_backend_dropdown_clone = secret_backend_dropdown.clone();
            let enable_fallback_clone = enable_fallback.clone();
            let kdbx_path_entry_clone = kdbx_path_entry.clone();
            let kdbx_password_entry_clone = kdbx_password_entry.clone();
            let kdbx_enabled_switch_clone = kdbx_enabled_switch.clone();
            let kdbx_save_password_check_clone = kdbx_save_password_check.clone();
            let kdbx_key_file_entry_clone = kdbx_key_file_entry.clone();
            let kdbx_use_key_file_check_clone = kdbx_use_key_file_check.clone();
            let remember_geometry_clone = remember_geometry.clone();
            let enable_tray_icon_clone = enable_tray_icon.clone();
            let minimize_to_tray_clone = minimize_to_tray.clone();
            let session_restore_enabled_clone = session_restore_enabled.clone();
            let prompt_on_restore_clone = prompt_on_restore.clone();
            let max_age_spin_clone = max_age_spin.clone();
            let settings_clone = settings.clone();

            save_btn.connect_clicked(move |_| {
                // SpinButton values are constrained by their adjustments to valid u32 ranges
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let terminal = TerminalSettings {
                    font_family: font_family_entry_clone.text().to_string(),
                    font_size: font_size_spin_clone.value() as u32,
                    scrollback_lines: scrollback_spin_clone.value() as u32,
                };

                // SpinButton values are constrained by their adjustments to valid u32 ranges
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let logging = LoggingSettings {
                    enabled: logging_enabled_switch_clone.is_active(),
                    log_directory: PathBuf::from(log_dir_entry_clone.text().to_string()),
                    retention_days: retention_spin_clone.value() as u32,
                    log_activity: log_activity_check_clone.is_active(),
                    log_input: log_input_check_clone.is_active(),
                    log_output: log_output_check_clone.is_active(),
                };

                // Map dropdown index to backend type
                let preferred_backend = match secret_backend_dropdown_clone.selected() {
                    0 => SecretBackendType::KeePassXc,
                    1 => SecretBackendType::KdbxFile,
                    _ => SecretBackendType::LibSecret,
                };

                // Get KDBX path from entry
                let kdbx_path_text = kdbx_path_entry_clone.text();
                let kdbx_path = if kdbx_path_text.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(kdbx_path_text.to_string()))
                };

                // Get password - handle placeholder for saved password
                let kdbx_password_text = kdbx_password_entry_clone.text();
                let save_password = kdbx_save_password_check_clone.is_active();

                // Borrow existing settings once to get all needed values
                let existing = settings_clone.borrow();

                // Check if this is a placeholder password (already saved)
                let is_placeholder = kdbx_password_text == "••••••••";

                let (kdbx_password, kdbx_password_encrypted) = if !save_password {
                    // Don't save password - clear everything
                    (None, None)
                } else if is_placeholder {
                    // Preserve existing encrypted password
                    (None, existing.secrets.kdbx_password_encrypted.clone())
                } else if kdbx_password_text.is_empty() {
                    (None, None)
                } else {
                    // New password entered - will be encrypted during save
                    (
                        Some(SecretString::from(kdbx_password_text.to_string())),
                        None,
                    )
                };

                // Get key file path
                let kdbx_key_file_text = kdbx_key_file_entry_clone.text();
                let kdbx_key_file = if kdbx_key_file_text.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(kdbx_key_file_text.to_string()))
                };

                let secrets = SecretSettings {
                    preferred_backend,
                    enable_fallback: enable_fallback_clone.is_active(),
                    kdbx_path,
                    kdbx_enabled: kdbx_enabled_switch_clone.is_active(),
                    kdbx_password,
                    kdbx_password_encrypted,
                    kdbx_key_file,
                    kdbx_use_key_file: kdbx_use_key_file_check_clone.is_active(),
                };

                // SpinButton value is constrained by adjustment to valid u32 range
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let session_restore = rustconn_core::config::SessionRestoreSettings {
                    enabled: session_restore_enabled_clone.is_active(),
                    prompt_on_restore: prompt_on_restore_clone.is_active(),
                    max_age_hours: max_age_spin_clone.value() as u32,
                    saved_sessions: existing.ui.session_restore.saved_sessions.clone(),
                };

                let ui = UiSettings {
                    remember_window_geometry: remember_geometry_clone.is_active(),
                    window_width: existing.ui.window_width,
                    window_height: existing.ui.window_height,
                    sidebar_width: existing.ui.sidebar_width,
                    enable_tray_icon: enable_tray_icon_clone.is_active(),
                    minimize_to_tray: minimize_to_tray_clone.is_active(),
                    expanded_groups: existing.ui.expanded_groups.clone(),
                    session_restore,
                };

                let new_settings = AppSettings {
                    terminal,
                    logging,
                    secrets,
                    ui,
                    global_variables: existing.global_variables.clone(),
                    history: existing.history.clone(),
                };
                drop(existing);

                if let Some(ref cb) = *on_save_clone.borrow() {
                    cb(Some(new_settings));
                }
                window_clone.close();
            });
        }

        // Initialize SSH Agent manager from environment
        let ssh_agent_manager = Rc::new(RefCell::new(SshAgentManager::from_env()));

        Self {
            window,
            save_button: save_btn,
            font_family_entry,
            font_size_spin,
            scrollback_spin,
            logging_enabled_switch,
            log_dir_entry,
            retention_spin,
            open_logs_button,
            log_activity_check,
            log_input_check,
            log_output_check,
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
            remember_geometry,
            enable_tray_icon,
            minimize_to_tray,
            session_restore_enabled,
            prompt_on_restore,
            max_age_spin,
            ssh_agent_status_label,
            ssh_agent_socket_label,
            ssh_agent_start_button,
            ssh_agent_keys_list,
            ssh_agent_add_key_button,
            ssh_agent_loading_spinner,
            ssh_agent_error_label,
            ssh_agent_refresh_button,
            ssh_agent_manager,
            settings,
            on_save,
        }
    }

    fn create_terminal_tab() -> (Frame, Entry, SpinButton, SpinButton) {
        let grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let mut row = 0;

        // Font family
        let font_label = Label::builder()
            .label("Font Family:")
            .halign(gtk4::Align::End)
            .build();
        let font_family_entry = Entry::builder().hexpand(true).text("Monospace").build();
        grid.attach(&font_label, 0, row, 1, 1);
        grid.attach(&font_family_entry, 1, row, 1, 1);
        row += 1;

        // Font size
        let size_label = Label::builder()
            .label("Font Size:")
            .halign(gtk4::Align::End)
            .build();
        let size_adj = gtk4::Adjustment::new(12.0, 6.0, 72.0, 1.0, 2.0, 0.0);
        let font_size_spin = SpinButton::builder()
            .adjustment(&size_adj)
            .climb_rate(1.0)
            .digits(0)
            .build();
        grid.attach(&size_label, 0, row, 1, 1);
        grid.attach(&font_size_spin, 1, row, 1, 1);
        row += 1;

        // Scrollback lines
        let scrollback_label = Label::builder()
            .label("Scrollback Lines:")
            .halign(gtk4::Align::End)
            .build();
        let scrollback_adj = gtk4::Adjustment::new(10000.0, 100.0, 1_000_000.0, 100.0, 1000.0, 0.0);
        let scrollback_spin = SpinButton::builder()
            .adjustment(&scrollback_adj)
            .climb_rate(100.0)
            .digits(0)
            .build();
        grid.attach(&scrollback_label, 0, row, 1, 1);
        grid.attach(&scrollback_spin, 1, row, 1, 1);

        let frame = Frame::builder()
            .label("Terminal Settings")
            .child(&grid)
            .margin_top(12)
            .valign(gtk4::Align::Start)
            .build();

        (frame, font_family_entry, font_size_spin, scrollback_spin)
    }

    #[allow(clippy::type_complexity)]
    fn create_logging_tab() -> (
        Frame,
        Switch,
        Entry,
        SpinButton,
        Button,
        CheckButton,
        CheckButton,
        CheckButton,
    ) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);

        // Enable logging switch row (similar to KeePass)
        let enable_row = GtkBox::new(Orientation::Horizontal, 12);
        let enable_label = Label::builder()
            .label("Persist logs")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        let logging_enabled_switch = Switch::builder().valign(gtk4::Align::Center).build();
        enable_row.append(&enable_label);
        enable_row.append(&logging_enabled_switch);
        vbox.append(&enable_row);

        // Logging mode checkboxes
        let mode_label = Label::builder()
            .label("Log content:")
            .halign(gtk4::Align::Start)
            .margin_top(8)
            .css_classes(["dim-label"])
            .build();
        vbox.append(&mode_label);

        let mode_box = GtkBox::new(Orientation::Vertical, 4);
        mode_box.set_margin_start(12);

        let log_activity_check = CheckButton::builder()
            .label("Activity (change counts)")
            .active(true)
            .sensitive(false)
            .build();
        let log_input_check = CheckButton::builder()
            .label("User input (commands)")
            .active(false)
            .sensitive(false)
            .build();
        let log_output_check = CheckButton::builder()
            .label("Terminal output (transcript)")
            .active(false)
            .sensitive(false)
            .build();

        mode_box.append(&log_activity_check);
        mode_box.append(&log_input_check);
        mode_box.append(&log_output_check);
        vbox.append(&mode_box);

        let grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(12)
            .margin_top(8)
            .build();

        let mut row = 0;

        // Log directory
        let dir_label = Label::builder()
            .label("Log Directory:")
            .halign(gtk4::Align::End)
            .build();
        let log_dir_entry = Entry::builder()
            .hexpand(true)
            .text("logs")
            .placeholder_text("Relative to config dir or absolute path")
            .sensitive(false)
            .build();
        grid.attach(&dir_label, 0, row, 1, 1);
        grid.attach(&log_dir_entry, 1, row, 1, 1);
        row += 1;

        // Retention days
        let retention_label = Label::builder()
            .label("Retention (days):")
            .halign(gtk4::Align::End)
            .build();
        let retention_adj = gtk4::Adjustment::new(30.0, 1.0, 365.0, 1.0, 7.0, 0.0);
        let retention_spin = SpinButton::builder()
            .adjustment(&retention_adj)
            .climb_rate(1.0)
            .digits(0)
            .sensitive(false)
            .build();
        grid.attach(&retention_label, 0, row, 1, 1);
        grid.attach(&retention_spin, 1, row, 1, 1);

        vbox.append(&grid);

        // Open logs directory button
        let open_logs_btn = Button::builder()
            .label("Open Logs Directory")
            .halign(gtk4::Align::Start)
            .margin_top(8)
            .sensitive(false)
            .build();

        let log_dir_entry_clone = log_dir_entry.clone();
        open_logs_btn.connect_clicked(move |_| {
            let log_dir = log_dir_entry_clone.text();
            let log_path = if log_dir.starts_with('/') {
                PathBuf::from(log_dir.as_str())
            } else {
                // Relative to config dir
                dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("rustconn")
                    .join(log_dir.as_str())
            };

            // Create directory if it doesn't exist
            if !log_path.exists() {
                if let Err(e) = std::fs::create_dir_all(&log_path) {
                    eprintln!("Failed to create logs directory: {e}");
                    return;
                }
            }

            // Open directory in file manager
            if let Err(e) = open::that(&log_path) {
                eprintln!("Failed to open logs directory: {e}");
            }
        });

        vbox.append(&open_logs_btn);

        // Connect switch to enable/disable other controls
        let dir_entry_clone = log_dir_entry.clone();
        let retention_clone = retention_spin.clone();
        let open_logs_btn_clone = open_logs_btn.clone();
        let log_activity_clone = log_activity_check.clone();
        let log_input_clone = log_input_check.clone();
        let log_output_clone = log_output_check.clone();
        logging_enabled_switch.connect_state_set(move |_, state| {
            dir_entry_clone.set_sensitive(state);
            retention_clone.set_sensitive(state);
            open_logs_btn_clone.set_sensitive(state);
            log_activity_clone.set_sensitive(state);
            log_input_clone.set_sensitive(state);
            log_output_clone.set_sensitive(state);
            gtk4::glib::Propagation::Proceed
        });

        let frame = Frame::builder()
            .label("Session Logging")
            .child(&vbox)
            .margin_top(12)
            .valign(gtk4::Align::Start)
            .build();

        (
            frame,
            logging_enabled_switch,
            log_dir_entry,
            retention_spin,
            open_logs_btn,
            log_activity_check,
            log_input_check,
            log_output_check,
        )
    }

    #[allow(clippy::type_complexity)]
    fn create_secrets_tab() -> (
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

        // KeePass Database Section
        let (
            kdbx_frame,
            kdbx_path_entry,
            kdbx_password_entry,
            kdbx_enabled_switch,
            kdbx_save_password_check,
            kdbx_status_label,
            browse_button,
            key_file_entry,
            key_file_browse_button,
            use_key_file_check,
        ) = Self::create_kdbx_section();
        main_vbox.append(&kdbx_frame);

        // KeePassXC Status Section
        let (keepassxc_frame, keepassxc_status_container) = Self::create_keepassxc_status_section();
        main_vbox.append(&keepassxc_frame);

        // Backend Selection Section
        let (backend_frame, secret_backend_dropdown, enable_fallback) =
            Self::create_backend_section();
        main_vbox.append(&backend_frame);

        scrolled.set_child(Some(&main_vbox));

        // Connect browse button for KDBX file
        let path_entry_clone = kdbx_path_entry.clone();
        browse_button.connect_clicked(move |btn| {
            Self::show_kdbx_file_dialog(btn, &path_entry_clone);
        });

        // Connect browse button for key file
        let key_file_entry_clone = key_file_entry.clone();
        key_file_browse_button.connect_clicked(move |btn| {
            Self::show_key_file_dialog(btn, &key_file_entry_clone);
        });

        // Connect enable switch to control sensitivity
        Self::connect_kdbx_enable_switch_extended(
            &kdbx_enabled_switch,
            &kdbx_path_entry,
            &kdbx_password_entry,
            &kdbx_save_password_check,
            &browse_button,
            &key_file_entry,
            &key_file_browse_button,
            &use_key_file_check,
        );

        // Connect use_key_file checkbox to toggle password/key file fields
        Self::connect_use_key_file_check(
            &use_key_file_check,
            &kdbx_password_entry,
            &kdbx_save_password_check,
            &key_file_entry,
            &key_file_browse_button,
        );

        (
            scrolled,
            secret_backend_dropdown,
            enable_fallback,
            kdbx_path_entry,
            kdbx_password_entry,
            kdbx_enabled_switch,
            kdbx_save_password_check,
            kdbx_status_label,
            browse_button,
            keepassxc_status_container,
            key_file_entry,
            key_file_browse_button,
            use_key_file_check,
        )
    }

    /// Creates the KDBX database section
    #[allow(clippy::type_complexity)]
    fn create_kdbx_section() -> (
        Frame,
        Entry,
        PasswordEntry,
        Switch,
        CheckButton,
        Label,
        Button,
        Entry,
        Button,
        CheckButton,
    ) {
        let kdbx_frame = Frame::builder().label("KeePass Database").build();
        let kdbx_vbox = GtkBox::new(Orientation::Vertical, 8);
        kdbx_vbox.set_margin_top(8);
        kdbx_vbox.set_margin_bottom(8);
        kdbx_vbox.set_margin_start(8);
        kdbx_vbox.set_margin_end(8);

        // Enable switch row
        let enable_row = GtkBox::new(Orientation::Horizontal, 12);
        let enable_label = Label::builder()
            .label("KeePass Integration")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        let kdbx_enabled_switch = Switch::builder().valign(gtk4::Align::Center).build();
        enable_row.append(&enable_label);
        enable_row.append(&kdbx_enabled_switch);
        kdbx_vbox.append(&enable_row);

        // KDBX file path and password grid
        let path_grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(8)
            .margin_top(8)
            .build();

        let path_label = Label::builder()
            .label("Database File:")
            .halign(gtk4::Align::End)
            .build();
        let kdbx_path_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Select a .kdbx file...")
            .editable(false)
            .sensitive(false)
            .build();
        let browse_button = Button::builder()
            .label("Browse...")
            .sensitive(false)
            .build();

        path_grid.attach(&path_label, 0, 0, 1, 1);
        path_grid.attach(&kdbx_path_entry, 1, 0, 1, 1);
        path_grid.attach(&browse_button, 2, 0, 1, 1);

        // Use key file checkbox (key file can be used with or without password)
        let use_key_file_check = CheckButton::builder()
            .label("Use key file (can be combined with password)")
            .sensitive(false)
            .build();
        path_grid.attach(&use_key_file_check, 1, 1, 2, 1);

        // Password row
        let password_label = Label::builder()
            .label("Password:")
            .halign(gtk4::Align::End)
            .build();
        let kdbx_password_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text("Enter database password")
            .show_peek_icon(true)
            .sensitive(false)
            .build();

        path_grid.attach(&password_label, 0, 2, 1, 1);
        path_grid.attach(&kdbx_password_entry, 1, 2, 2, 1);

        // Save password checkbox
        let kdbx_save_password_check = CheckButton::builder()
            .label("Save password")
            .sensitive(false)
            .build();
        path_grid.attach(&kdbx_save_password_check, 1, 3, 2, 1);

        // Key file row
        let key_file_label = Label::builder()
            .label("Key File:")
            .halign(gtk4::Align::End)
            .build();
        let key_file_entry = Entry::builder()
            .hexpand(true)
            .placeholder_text("Select a .keyx or .key file...")
            .editable(false)
            .sensitive(false)
            .build();
        let key_file_browse_button = Button::builder()
            .label("Browse...")
            .sensitive(false)
            .build();

        path_grid.attach(&key_file_label, 0, 4, 1, 1);
        path_grid.attach(&key_file_entry, 1, 4, 1, 1);
        path_grid.attach(&key_file_browse_button, 2, 4, 1, 1);

        kdbx_vbox.append(&path_grid);

        // Status label
        let status_row = GtkBox::new(Orientation::Horizontal, 12);
        status_row.set_margin_top(8);

        let kdbx_status_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .css_classes(["dim-label"])
            .build();

        status_row.append(&kdbx_status_label);
        kdbx_vbox.append(&status_row);

        kdbx_frame.set_child(Some(&kdbx_vbox));

        (
            kdbx_frame,
            kdbx_path_entry,
            kdbx_password_entry,
            kdbx_enabled_switch,
            kdbx_save_password_check,
            kdbx_status_label,
            browse_button,
            key_file_entry,
            key_file_browse_button,
            use_key_file_check,
        )
    }

    /// Creates the `KeePassXC` status section
    fn create_keepassxc_status_section() -> (Frame, GtkBox) {
        let keepassxc_frame = Frame::builder().label("KeePassXC Status").build();
        let keepassxc_status_container = GtkBox::new(Orientation::Vertical, 6);
        keepassxc_status_container.set_margin_top(8);
        keepassxc_status_container.set_margin_bottom(8);
        keepassxc_status_container.set_margin_start(8);
        keepassxc_status_container.set_margin_end(8);

        Self::populate_keepassxc_status(&keepassxc_status_container);
        keepassxc_frame.set_child(Some(&keepassxc_status_container));

        (keepassxc_frame, keepassxc_status_container)
    }

    /// Creates the backend selection section
    fn create_backend_section() -> (Frame, DropDown, CheckButton) {
        let backend_frame = Frame::builder().label("Secret Backend").build();
        let backend_vbox = GtkBox::new(Orientation::Vertical, 8);
        backend_vbox.set_margin_top(8);
        backend_vbox.set_margin_bottom(8);
        backend_vbox.set_margin_start(8);
        backend_vbox.set_margin_end(8);

        let backend_grid = Grid::builder().row_spacing(8).column_spacing(12).build();

        let backend_label = Label::builder()
            .label("Preferred Backend:")
            .halign(gtk4::Align::End)
            .build();
        let backend_list = StringList::new(&[
            "KeePassXC (Browser Integration)",
            "KDBX File (Direct Access)",
            "libsecret (System Keyring)",
        ]);
        let secret_backend_dropdown = DropDown::new(Some(backend_list), gtk4::Expression::NONE);
        secret_backend_dropdown.set_selected(0);

        backend_grid.attach(&backend_label, 0, 0, 1, 1);
        backend_grid.attach(&secret_backend_dropdown, 1, 0, 1, 1);
        backend_vbox.append(&backend_grid);

        let enable_fallback = CheckButton::builder()
            .label("Enable fallback to libsecret if primary backend unavailable")
            .active(true)
            .margin_top(4)
            .build();
        backend_vbox.append(&enable_fallback);

        let info = Label::builder()
            .label("KeePassXC requires browser integration enabled.\nKDBX File provides direct database access.\nlibsecret uses GNOME Keyring or KDE Wallet.")
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["dim-label"])
            .margin_top(8)
            .build();
        backend_vbox.append(&info);

        backend_frame.set_child(Some(&backend_vbox));

        (backend_frame, secret_backend_dropdown, enable_fallback)
    }

    /// Connects the KDBX enable switch with key file support
    #[allow(clippy::too_many_arguments)]
    fn connect_kdbx_enable_switch_extended(
        switch: &Switch,
        path_entry: &Entry,
        password_entry: &PasswordEntry,
        save_password_check: &CheckButton,
        browse_button: &Button,
        key_file_entry: &Entry,
        key_file_browse_button: &Button,
        use_key_file_check: &CheckButton,
    ) {
        let path_entry = path_entry.clone();
        let password_entry = password_entry.clone();
        let save_password_check = save_password_check.clone();
        let browse_button = browse_button.clone();
        let key_file_entry = key_file_entry.clone();
        let key_file_browse_button = key_file_browse_button.clone();
        let use_key_file_check = use_key_file_check.clone();

        switch.connect_state_set(move |_, state| {
            path_entry.set_sensitive(state);
            browse_button.set_sensitive(state);
            use_key_file_check.set_sensitive(state);

            // Password fields are sensitive only if enabled AND not using key file
            let use_key_file = use_key_file_check.is_active();
            password_entry.set_sensitive(state && !use_key_file);
            save_password_check.set_sensitive(state && !use_key_file);

            // Key file fields are sensitive only if enabled AND using key file
            key_file_entry.set_sensitive(state && use_key_file);
            key_file_browse_button.set_sensitive(state && use_key_file);

            gtk4::glib::Propagation::Proceed
        });
    }

    /// Connects the use key file checkbox to toggle key file fields
    /// Password fields remain enabled - password and key file can be used together
    fn connect_use_key_file_check(
        use_key_file_check: &CheckButton,
        _password_entry: &PasswordEntry,
        _save_password_check: &CheckButton,
        key_file_entry: &Entry,
        key_file_browse_button: &Button,
    ) {
        let key_file_entry = key_file_entry.clone();
        let key_file_browse_button = key_file_browse_button.clone();

        use_key_file_check.connect_toggled(move |check| {
            let use_key_file = check.is_active();
            // Only toggle if the parent switch is enabled
            let parent_enabled = check.is_sensitive();

            // Key file fields are enabled when checkbox is checked
            key_file_entry.set_sensitive(parent_enabled && use_key_file);
            key_file_browse_button.set_sensitive(parent_enabled && use_key_file);
            // Password fields remain enabled - both can be used together
        });
    }

    /// Shows file dialog for selecting key file
    fn show_key_file_dialog(button: &Button, entry: &Entry) {
        let dialog = FileDialog::builder()
            .title("Select Key File")
            .modal(true)
            .build();

        // Create filters - KeePassXC creates key files without extension by default
        let all_filter = FileFilter::new();
        all_filter.add_pattern("*");
        all_filter.set_name(Some("All Files (*)"));

        let key_filter = FileFilter::new();
        key_filter.add_pattern("*.keyx");
        key_filter.add_pattern("*.key");
        key_filter.set_name(Some("KeePass Key Files (*.keyx, *.key)"));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&all_filter);
        filters.append(&key_filter);
        dialog.set_filters(Some(&filters));

        let entry = entry.clone();
        let window = button.root().and_then(|r| r.downcast::<Window>().ok());

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
    }

    /// Populates the `KeePassXC` status container with current detection info
    fn populate_keepassxc_status(container: &GtkBox) {
        // Clear existing children
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }

        let status = KeePassStatus::detect();

        let grid = Grid::builder().row_spacing(6).column_spacing(12).build();

        let mut row = 0;

        // Installation status
        let status_label = Label::builder()
            .label("Status:")
            .halign(gtk4::Align::End)
            .build();
        let status_value = if status.keepassxc_installed {
            Label::builder()
                .label("✓ Installed")
                .halign(gtk4::Align::Start)
                .css_classes(["success"])
                .build()
        } else {
            Label::builder()
                .label("✗ Not installed")
                .halign(gtk4::Align::Start)
                .css_classes(["error"])
                .build()
        };
        grid.attach(&status_label, 0, row, 1, 1);
        grid.attach(&status_value, 1, row, 1, 1);
        row += 1;

        if status.keepassxc_installed {
            // Version
            if let Some(version) = &status.keepassxc_version {
                let version_label = Label::builder()
                    .label("Version:")
                    .halign(gtk4::Align::End)
                    .build();
                let version_value = Label::builder()
                    .label(version)
                    .halign(gtk4::Align::Start)
                    .selectable(true)
                    .build();
                grid.attach(&version_label, 0, row, 1, 1);
                grid.attach(&version_value, 1, row, 1, 1);
                row += 1;
            }

            // Path
            if let Some(path) = &status.keepassxc_path {
                let path_label = Label::builder()
                    .label("Path:")
                    .halign(gtk4::Align::End)
                    .build();
                let path_value = Label::builder()
                    .label(path.to_string_lossy().as_ref())
                    .halign(gtk4::Align::Start)
                    .selectable(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                    .build();
                grid.attach(&path_label, 0, row, 1, 1);
                grid.attach(&path_value, 1, row, 1, 1);
            }
        } else {
            // Installation hint
            let hint_label = Label::builder()
                .label("Install:")
                .halign(gtk4::Align::End)
                .build();
            let hint_value = Label::builder()
                .label("Install KeePassXC from your package manager\nor https://keepassxc.org")
                .halign(gtk4::Align::Start)
                .wrap(true)
                .css_classes(["dim-label"])
                .build();
            grid.attach(&hint_label, 0, row, 1, 1);
            grid.attach(&hint_value, 1, row, 1, 1);
        }

        container.append(&grid);

        // Refresh button
        let refresh_btn = Button::builder()
            .label("Refresh")
            .halign(gtk4::Align::Start)
            .margin_top(8)
            .build();

        let container_clone = container.clone();
        refresh_btn.connect_clicked(move |_| {
            Self::populate_keepassxc_status(&container_clone);
        });

        container.append(&refresh_btn);
    }

    /// Shows the KDBX file selection dialog
    fn show_kdbx_file_dialog(button: &Button, path_entry: &Entry) {
        let dialog = FileDialog::builder()
            .title("Select KeePass Database")
            .modal(true)
            .build();

        // Create filter for .kdbx files
        let filter = FileFilter::new();
        filter.add_pattern("*.kdbx");
        filter.add_pattern("*.KDBX");
        filter.set_name(Some("KeePass Database (*.kdbx)"));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let path_entry_clone = path_entry.clone();

        // Get the window from the button
        let window = button.root().and_then(|r| r.downcast::<Window>().ok());

        dialog.open(
            window.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        // Validate the path
                        match KeePassStatus::validate_kdbx_path(&path) {
                            Ok(()) => {
                                path_entry_clone.set_text(&path.to_string_lossy());
                            }
                            Err(e) => {
                                eprintln!("Invalid KDBX path: {e}");
                                // Could show an error dialog here
                            }
                        }
                    }
                }
            },
        );
    }

    fn create_ui_tab() -> (
        Frame,
        CheckButton,
        CheckButton,
        CheckButton,
        CheckButton,
        CheckButton,
        SpinButton,
    ) {
        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);

        // Window settings section
        let window_label = Label::builder()
            .label("<b>Window</b>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .build();
        vbox.append(&window_label);

        let remember_geometry = CheckButton::builder()
            .label("Remember window size and position")
            .active(true)
            .margin_start(12)
            .build();
        vbox.append(&remember_geometry);

        // Tray icon settings section
        let tray_label = Label::builder()
            .label("<b>System Tray</b>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .margin_top(12)
            .build();
        vbox.append(&tray_label);

        let enable_tray_icon = CheckButton::builder()
            .label("Show icon in system tray")
            .active(true)
            .margin_start(12)
            .build();
        vbox.append(&enable_tray_icon);

        let minimize_to_tray = CheckButton::builder()
            .label("Minimize to tray when closing window")
            .active(false)
            .margin_start(12)
            .build();
        vbox.append(&minimize_to_tray);

        // Make minimize_to_tray sensitive only when tray icon is enabled
        let minimize_to_tray_clone = minimize_to_tray.clone();
        enable_tray_icon.connect_toggled(move |check| {
            minimize_to_tray_clone.set_sensitive(check.is_active());
        });

        // Add note about tray icon requirements
        let tray_note = Label::builder()
            .label("<small>Note: Tray icon requires libdbus-1-dev to be installed.\nIf not available, the tray icon will be disabled.</small>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .margin_start(12)
            .margin_top(6)
            .css_classes(["dim-label"])
            .build();
        vbox.append(&tray_note);

        // Session restore settings section
        let restore_label = Label::builder()
            .label("<b>Session Restore</b>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .margin_top(12)
            .build();
        vbox.append(&restore_label);

        let session_restore_enabled = CheckButton::builder()
            .label("Restore sessions on startup")
            .active(false)
            .margin_start(12)
            .build();
        vbox.append(&session_restore_enabled);

        let prompt_on_restore = CheckButton::builder()
            .label("Ask before restoring sessions")
            .active(true)
            .margin_start(12)
            .sensitive(false)
            .build();
        vbox.append(&prompt_on_restore);

        // Max age setting
        let age_box = GtkBox::new(Orientation::Horizontal, 6);
        age_box.set_margin_start(12);

        let age_label = Label::builder()
            .label("Maximum session age (hours, 0 = no limit):")
            .halign(gtk4::Align::Start)
            .build();
        age_box.append(&age_label);

        let max_age_adj = gtk4::Adjustment::new(24.0, 0.0, 168.0, 1.0, 6.0, 0.0);
        let max_age_spin = SpinButton::builder()
            .adjustment(&max_age_adj)
            .climb_rate(1.0)
            .digits(0)
            .sensitive(false)
            .build();
        age_box.append(&max_age_spin);

        vbox.append(&age_box);

        // Connect session restore enabled to control other widgets
        let prompt_clone = prompt_on_restore.clone();
        let max_age_clone = max_age_spin.clone();
        session_restore_enabled.connect_toggled(move |check| {
            let active = check.is_active();
            prompt_clone.set_sensitive(active);
            max_age_clone.set_sensitive(active);
        });

        let restore_note = Label::builder()
            .label("<small>Sessions are saved when the application closes and restored on next startup.</small>")
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .margin_start(12)
            .margin_top(6)
            .css_classes(["dim-label"])
            .build();
        vbox.append(&restore_note);

        let frame = Frame::builder()
            .label("Interface Settings")
            .child(&vbox)
            .margin_top(12)
            .valign(gtk4::Align::Start)
            .build();

        (
            frame,
            remember_geometry,
            enable_tray_icon,
            minimize_to_tray,
            session_restore_enabled,
            prompt_on_restore,
            max_age_spin,
        )
    }

    /// Creates the Clients tab showing detected protocol clients
    fn create_clients_tab() -> ScrolledWindow {
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .build();

        let vbox = GtkBox::new(Orientation::Vertical, 12);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);
        vbox.set_valign(gtk4::Align::Start);

        // Create a container for client info that can be refreshed
        let clients_container = GtkBox::new(Orientation::Vertical, 12);
        Self::populate_clients_info(&clients_container);
        vbox.append(&clients_container);

        // Add refresh button
        let refresh_btn = Button::builder()
            .label("Refresh")
            .halign(gtk4::Align::Start)
            .margin_top(12)
            .build();

        let container_clone = clients_container;
        refresh_btn.connect_clicked(move |_| {
            // Clear existing children
            while let Some(child) = container_clone.first_child() {
                container_clone.remove(&child);
            }
            // Re-populate with fresh detection
            Self::populate_clients_info(&container_clone);
        });

        vbox.append(&refresh_btn);
        scrolled.set_child(Some(&vbox));
        scrolled
    }

    /// Populates the clients container with detected client information
    fn populate_clients_info(container: &GtkBox) {
        use rustconn_core::{
            detect_aws_cli, detect_azure_cli, detect_boundary, detect_cloudflared,
            detect_gcloud_cli, detect_oci_cli, detect_tailscale, detect_teleport,
        };

        // Detect standard protocol clients
        let ssh_info = detect_ssh_client();
        let rdp_info = detect_rdp_client();
        let vnc_info = detect_vnc_client();

        // Add SSH client info
        let ssh_frame = Self::create_client_frame("SSH Client", &ssh_info);
        container.append(&ssh_frame);

        // Add RDP client info
        let rdp_frame = Self::create_client_frame("RDP Client", &rdp_info);
        container.append(&rdp_frame);

        // Add VNC client info
        let vnc_frame = Self::create_client_frame("VNC Client", &vnc_info);
        container.append(&vnc_frame);

        // Zero Trust section header
        let zt_label = Label::builder()
            .label("Zero Trust Clients")
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .margin_top(16)
            .margin_bottom(8)
            .build();
        container.append(&zt_label);

        // Detect Zero Trust CLI tools
        let aws_info = detect_aws_cli();
        let gcloud_info = detect_gcloud_cli();
        let azure_info = detect_azure_cli();
        let oci_info = detect_oci_cli();
        let cloudflared_info = detect_cloudflared();
        let teleport_info = detect_teleport();
        let tailscale_info = detect_tailscale();
        let boundary_info = detect_boundary();

        // Add Zero Trust client info
        container.append(&Self::create_client_frame("AWS CLI (SSM)", &aws_info));
        container.append(&Self::create_client_frame(
            "Google Cloud CLI (IAP)",
            &gcloud_info,
        ));
        container.append(&Self::create_client_frame(
            "Azure CLI (Bastion/SSH)",
            &azure_info,
        ));
        container.append(&Self::create_client_frame("OCI CLI (Bastion)", &oci_info));
        container.append(&Self::create_client_frame(
            "Cloudflare Access",
            &cloudflared_info,
        ));
        container.append(&Self::create_client_frame("Teleport", &teleport_info));
        container.append(&Self::create_client_frame("Tailscale SSH", &tailscale_info));
        container.append(&Self::create_client_frame(
            "HashiCorp Boundary",
            &boundary_info,
        ));
    }

    /// Creates the SSH Agent tab for managing SSH keys
    #[allow(clippy::type_complexity)]
    fn create_ssh_agent_tab() -> (
        ScrolledWindow,
        Label,
        Label,
        Button,
        ListBox,
        Button,
        Spinner,
        Label,
        Button,
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

        // === Agent Status Section ===
        let status_frame = Frame::builder().label("Agent Status").build();
        let status_vbox = GtkBox::new(Orientation::Vertical, 8);
        status_vbox.set_margin_top(8);
        status_vbox.set_margin_bottom(8);
        status_vbox.set_margin_start(8);
        status_vbox.set_margin_end(8);

        let status_grid = Grid::builder().row_spacing(6).column_spacing(12).build();

        // Status row
        let status_title = Label::builder()
            .label("Status:")
            .halign(gtk4::Align::End)
            .build();
        let ssh_agent_status_label = Label::builder()
            .label("Checking...")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        status_grid.attach(&status_title, 0, 0, 1, 1);
        status_grid.attach(&ssh_agent_status_label, 1, 0, 1, 1);

        // Socket path row
        let socket_title = Label::builder()
            .label("Socket:")
            .halign(gtk4::Align::End)
            .build();
        let ssh_agent_socket_label = Label::builder()
            .label("-")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .selectable(true)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .css_classes(["dim-label"])
            .build();
        status_grid.attach(&socket_title, 0, 1, 1, 1);
        status_grid.attach(&ssh_agent_socket_label, 1, 1, 1, 1);

        status_vbox.append(&status_grid);

        // Start Agent button
        let ssh_agent_start_button = Button::builder()
            .label("Start Agent")
            .halign(gtk4::Align::Start)
            .margin_top(8)
            .css_classes(["suggested-action"])
            .build();
        status_vbox.append(&ssh_agent_start_button);

        status_frame.set_child(Some(&status_vbox));
        main_vbox.append(&status_frame);

        // === Loaded Keys Section ===
        let keys_frame = Frame::builder().label("Loaded Keys").build();
        let keys_vbox = GtkBox::new(Orientation::Vertical, 8);
        keys_vbox.set_margin_top(8);
        keys_vbox.set_margin_bottom(8);
        keys_vbox.set_margin_start(8);
        keys_vbox.set_margin_end(8);

        // Loading spinner (hidden by default)
        let ssh_agent_loading_spinner = Spinner::builder()
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        keys_vbox.append(&ssh_agent_loading_spinner);

        // Error label (hidden by default)
        let ssh_agent_error_label = Label::builder()
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["error"])
            .visible(false)
            .build();
        keys_vbox.append(&ssh_agent_error_label);

        // Keys list
        let ssh_agent_keys_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        keys_vbox.append(&ssh_agent_keys_list);

        // Add Key button row
        let button_row = GtkBox::new(Orientation::Horizontal, 8);
        button_row.set_margin_top(8);

        let ssh_agent_add_key_button = Button::builder()
            .label("Add Key...")
            .halign(gtk4::Align::Start)
            .build();
        button_row.append(&ssh_agent_add_key_button);

        // Refresh button
        let refresh_button = Button::builder()
            .label("Refresh")
            .halign(gtk4::Align::Start)
            .build();
        button_row.append(&refresh_button);

        keys_vbox.append(&button_row);

        keys_frame.set_child(Some(&keys_vbox));
        main_vbox.append(&keys_frame);

        // === Available Keys Section ===
        let available_frame = Frame::builder().label("Available Key Files").build();
        let available_vbox = GtkBox::new(Orientation::Vertical, 8);
        available_vbox.set_margin_top(8);
        available_vbox.set_margin_bottom(8);
        available_vbox.set_margin_start(8);
        available_vbox.set_margin_end(8);

        let available_info = Label::builder()
            .label("Key files found in ~/.ssh/")
            .halign(gtk4::Align::Start)
            .css_classes(["dim-label"])
            .build();
        available_vbox.append(&available_info);

        // List available key files
        let available_keys_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();

        // Populate available keys
        Self::populate_available_keys(&available_keys_list);

        available_vbox.append(&available_keys_list);
        available_frame.set_child(Some(&available_vbox));
        main_vbox.append(&available_frame);

        scrolled.set_child(Some(&main_vbox));

        (
            scrolled,
            ssh_agent_status_label,
            ssh_agent_socket_label,
            ssh_agent_start_button,
            ssh_agent_keys_list,
            ssh_agent_add_key_button,
            ssh_agent_loading_spinner,
            ssh_agent_error_label,
            refresh_button,
        )
    }

    /// Populates the available keys list with key files from ~/.ssh/
    fn populate_available_keys(list: &ListBox) {
        // Clear existing items
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        match SshAgentManager::list_key_files() {
            Ok(keys) => {
                if keys.is_empty() {
                    let row = ListBoxRow::new();
                    let label = Label::builder()
                        .label("No key files found in ~/.ssh/")
                        .halign(gtk4::Align::Start)
                        .margin_top(8)
                        .margin_bottom(8)
                        .margin_start(8)
                        .css_classes(["dim-label"])
                        .build();
                    row.set_child(Some(&label));
                    list.append(&row);
                } else {
                    for key_path in keys {
                        let row = ListBoxRow::new();
                        let hbox = GtkBox::new(Orientation::Horizontal, 8);
                        hbox.set_margin_top(6);
                        hbox.set_margin_bottom(6);
                        hbox.set_margin_start(8);
                        hbox.set_margin_end(8);

                        let file_name = key_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| key_path.to_string_lossy().to_string());

                        let label = Label::builder()
                            .label(&file_name)
                            .halign(gtk4::Align::Start)
                            .hexpand(true)
                            .build();
                        hbox.append(&label);

                        let path_label = Label::builder()
                            .label(key_path.to_string_lossy().as_ref())
                            .halign(gtk4::Align::End)
                            .css_classes(["dim-label"])
                            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                            .max_width_chars(30)
                            .build();
                        hbox.append(&path_label);

                        row.set_child(Some(&hbox));
                        list.append(&row);
                    }
                }
            }
            Err(e) => {
                let row = ListBoxRow::new();
                let label = Label::builder()
                    .label(&format!("Error listing keys: {e}"))
                    .halign(gtk4::Align::Start)
                    .margin_top(8)
                    .margin_bottom(8)
                    .margin_start(8)
                    .css_classes(["error"])
                    .build();
                row.set_child(Some(&label));
                list.append(&row);
            }
        }
    }

    /// Creates a row widget for a loaded SSH key
    fn create_key_row(
        key: &AgentKey,
        manager: &Rc<RefCell<SshAgentManager>>,
        keys_list: &ListBox,
        error_label: &Label,
    ) -> ListBoxRow {
        let row = ListBoxRow::new();
        let hbox = GtkBox::new(Orientation::Horizontal, 8);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);

        // Key info container
        let info_vbox = GtkBox::new(Orientation::Vertical, 2);
        info_vbox.set_hexpand(true);

        // Key type and bits
        let type_label = Label::builder()
            .label(&format!("{} ({} bits)", key.key_type, key.bits))
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        info_vbox.append(&type_label);

        // Fingerprint
        let fingerprint_label = Label::builder()
            .label(&key.fingerprint)
            .halign(gtk4::Align::Start)
            .selectable(true)
            .css_classes(["monospace", "dim-label"])
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .build();
        info_vbox.append(&fingerprint_label);

        // Comment
        if !key.comment.is_empty() {
            let comment_label = Label::builder()
                .label(&key.comment)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            info_vbox.append(&comment_label);
        }

        hbox.append(&info_vbox);

        // Remove button
        let remove_button = Button::builder()
            .label("Remove")
            .valign(gtk4::Align::Center)
            .css_classes(["destructive-action"])
            .build();

        // Connect remove button
        let key_comment = key.comment.clone();
        let manager_clone = manager.clone();
        let keys_list_clone = keys_list.clone();
        let error_label_clone = error_label.clone();
        let row_weak = row.downgrade();

        remove_button.connect_clicked(move |_| {
            // Try to remove the key using the comment (which is usually the file path)
            let key_path = PathBuf::from(&key_comment);
            let manager = manager_clone.borrow();

            match manager.remove_key(&key_path) {
                Ok(()) => {
                    // Remove the row from the list
                    if let Some(row) = row_weak.upgrade() {
                        keys_list_clone.remove(&row);
                    }
                    error_label_clone.set_visible(false);
                }
                Err(e) => {
                    error_label_clone.set_text(&format!("Failed to remove key: {e}"));
                    error_label_clone.set_visible(true);
                }
            }
        });

        hbox.append(&remove_button);
        row.set_child(Some(&hbox));
        row
    }

    /// Refreshes the SSH agent status and key list
    fn refresh_ssh_agent_status(&self) {
        let manager = self.ssh_agent_manager.borrow();
        let status = manager.get_status();

        match status {
            Ok(status) => {
                if status.running {
                    self.ssh_agent_status_label.set_text("✓ Running");
                    self.ssh_agent_status_label.remove_css_class("error");
                    self.ssh_agent_status_label.add_css_class("success");
                    self.ssh_agent_start_button.set_sensitive(false);
                    self.ssh_agent_start_button.set_label("Agent Running");
                    self.ssh_agent_add_key_button.set_sensitive(true);

                    if let Some(socket) = &status.socket_path {
                        self.ssh_agent_socket_label.set_text(socket);
                    } else {
                        self.ssh_agent_socket_label.set_text("-");
                    }

                    // Populate keys list
                    self.populate_keys_list(&status.keys);
                } else {
                    self.ssh_agent_status_label.set_text("✗ Not Running");
                    self.ssh_agent_status_label.remove_css_class("success");
                    self.ssh_agent_status_label.add_css_class("error");
                    self.ssh_agent_start_button.set_sensitive(true);
                    self.ssh_agent_start_button.set_label("Start Agent");
                    self.ssh_agent_add_key_button.set_sensitive(false);

                    if let Some(socket) = &status.socket_path {
                        self.ssh_agent_socket_label
                            .set_text(&format!("{socket} (not responding)"));
                    } else {
                        self.ssh_agent_socket_label.set_text("Not configured");
                    }

                    // Clear keys list
                    self.clear_keys_list();
                }
                self.ssh_agent_error_label.set_visible(false);
            }
            Err(e) => {
                self.ssh_agent_status_label.set_text("✗ Error");
                self.ssh_agent_status_label.remove_css_class("success");
                self.ssh_agent_status_label.add_css_class("error");
                self.ssh_agent_socket_label.set_text("-");
                self.ssh_agent_start_button.set_sensitive(true);
                self.ssh_agent_add_key_button.set_sensitive(false);
                self.ssh_agent_error_label.set_text(&format!("Error: {e}"));
                self.ssh_agent_error_label.set_visible(true);
                self.clear_keys_list();
            }
        }
    }

    /// Populates the keys list with loaded keys
    fn populate_keys_list(&self, keys: &[AgentKey]) {
        // Clear existing items
        self.clear_keys_list();

        if keys.is_empty() {
            let row = ListBoxRow::new();
            let label = Label::builder()
                .label("No keys loaded in agent")
                .halign(gtk4::Align::Start)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(8)
                .css_classes(["dim-label"])
                .build();
            row.set_child(Some(&label));
            self.ssh_agent_keys_list.append(&row);
        } else {
            for key in keys {
                let row = Self::create_key_row(
                    key,
                    &self.ssh_agent_manager,
                    &self.ssh_agent_keys_list,
                    &self.ssh_agent_error_label,
                );
                self.ssh_agent_keys_list.append(&row);
            }
        }
    }

    /// Clears the keys list
    fn clear_keys_list(&self) {
        while let Some(child) = self.ssh_agent_keys_list.first_child() {
            self.ssh_agent_keys_list.remove(&child);
        }
    }

    /// Shows the passphrase dialog for adding a key
    /// Returns the passphrase if entered, or None if cancelled
    fn show_passphrase_dialog(
        parent: &Window,
        key_path: &std::path::Path,
        manager: &Rc<RefCell<SshAgentManager>>,
        keys_list: &ListBox,
        error_label: &Label,
        loading_spinner: &Spinner,
    ) {
        // Create a modal dialog for passphrase entry
        let dialog = Window::builder()
            .title("Enter Passphrase")
            .modal(true)
            .transient_for(parent)
            .default_width(400)
            .resizable(false)
            .build();

        // Create header bar with Cancel/OK buttons
        let header = HeaderBar::new();
        let cancel_btn = Button::builder().label("Cancel").build();
        let ok_btn = Button::builder()
            .label("Add Key")
            .css_classes(["suggested-action"])
            .build();
        header.pack_start(&cancel_btn);
        header.pack_end(&ok_btn);
        dialog.set_titlebar(Some(&header));

        // Content
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Key file label
        let key_label = Label::builder()
            .label(&format!(
                "Enter passphrase for:\n{}",
                key_path.file_name().map_or_else(
                    || key_path.to_string_lossy().to_string(),
                    |n| n.to_string_lossy().to_string()
                )
            ))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        content.append(&key_label);

        // Passphrase entry
        let passphrase_entry = PasswordEntry::builder()
            .placeholder_text("Enter passphrase")
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        content.append(&passphrase_entry);

        // Error label (hidden by default)
        let dialog_error_label = Label::builder()
            .halign(gtk4::Align::Start)
            .wrap(true)
            .css_classes(["error"])
            .visible(false)
            .build();
        content.append(&dialog_error_label);

        dialog.set_child(Some(&content));

        // Connect cancel button
        let dialog_clone = dialog.clone();
        cancel_btn.connect_clicked(move |_| {
            dialog_clone.close();
        });

        // Connect OK button
        let dialog_clone = dialog.clone();
        let key_path = key_path.to_path_buf();
        let manager = manager.clone();
        let keys_list = keys_list.clone();
        let error_label = error_label.clone();
        let loading_spinner = loading_spinner.clone();
        let dialog_error_label_clone = dialog_error_label.clone();
        let passphrase_entry_clone = passphrase_entry.clone();

        ok_btn.connect_clicked(move |_| {
            let passphrase = passphrase_entry_clone.text();

            if passphrase.is_empty() {
                dialog_error_label_clone.set_text("Passphrase cannot be empty");
                dialog_error_label_clone.set_visible(true);
                return;
            }

            // Show loading
            loading_spinner.set_visible(true);
            loading_spinner.start();

            // Try to add the key with passphrase
            let manager_ref = manager.borrow();
            match manager_ref.add_key(&key_path, Some(&passphrase)) {
                Ok(()) => {
                    // Refresh the keys list
                    if let Ok(status) = manager_ref.get_status() {
                        // Clear and repopulate
                        while let Some(child) = keys_list.first_child() {
                            keys_list.remove(&child);
                        }

                        if status.keys.is_empty() {
                            let row = ListBoxRow::new();
                            let label = Label::builder()
                                .label("No keys loaded in agent")
                                .halign(gtk4::Align::Start)
                                .margin_top(8)
                                .margin_bottom(8)
                                .margin_start(8)
                                .css_classes(["dim-label"])
                                .build();
                            row.set_child(Some(&label));
                            keys_list.append(&row);
                        } else {
                            for key in &status.keys {
                                let row =
                                    Self::create_key_row(key, &manager, &keys_list, &error_label);
                                keys_list.append(&row);
                            }
                        }
                    }
                    error_label.set_visible(false);
                    dialog_clone.close();
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("bad passphrase")
                        || err_str.contains("incorrect passphrase")
                    {
                        dialog_error_label_clone.set_text("Incorrect passphrase");
                    } else {
                        dialog_error_label_clone.set_text(&format!("Failed to add key: {e}"));
                    }
                    dialog_error_label_clone.set_visible(true);
                }
            }

            loading_spinner.stop();
            loading_spinner.set_visible(false);
        });

        // Connect Enter key to submit
        let ok_btn_clone = ok_btn.clone();
        passphrase_entry.connect_activate(move |_| {
            ok_btn_clone.emit_clicked();
        });

        dialog.present();
    }

    /// Creates a frame displaying information about a single client
    fn create_client_frame(title: &str, info: &ClientInfo) -> Frame {
        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(8)
            .margin_end(8)
            .build();

        let mut row = 0;

        // Status row
        let status_label = Label::builder()
            .label("Status:")
            .halign(gtk4::Align::End)
            .build();
        let status_value = if info.installed {
            Label::builder()
                .label("✓ Installed")
                .halign(gtk4::Align::Start)
                .css_classes(["success"])
                .build()
        } else {
            Label::builder()
                .label("✗ Not installed")
                .halign(gtk4::Align::Start)
                .css_classes(["error"])
                .build()
        };
        grid.attach(&status_label, 0, row, 1, 1);
        grid.attach(&status_value, 1, row, 1, 1);
        row += 1;

        // Name row
        let name_label = Label::builder()
            .label("Name:")
            .halign(gtk4::Align::End)
            .build();
        let name_value = Label::builder()
            .label(&info.name)
            .halign(gtk4::Align::Start)
            .selectable(true)
            .build();
        grid.attach(&name_label, 0, row, 1, 1);
        grid.attach(&name_value, 1, row, 1, 1);
        row += 1;

        if info.installed {
            // Path row (only if installed)
            if let Some(path) = &info.path {
                let path_label = Label::builder()
                    .label("Path:")
                    .halign(gtk4::Align::End)
                    .build();
                let path_value = Label::builder()
                    .label(path.to_string_lossy().as_ref())
                    .halign(gtk4::Align::Start)
                    .selectable(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                    .build();
                grid.attach(&path_label, 0, row, 1, 1);
                grid.attach(&path_value, 1, row, 1, 1);
                row += 1;
            }

            // Version row (only if installed and version available)
            if let Some(version) = &info.version {
                let version_label = Label::builder()
                    .label("Version:")
                    .halign(gtk4::Align::End)
                    .build();
                let version_value = Label::builder()
                    .label(version)
                    .halign(gtk4::Align::Start)
                    .selectable(true)
                    .wrap(true)
                    .max_width_chars(50)
                    .build();
                grid.attach(&version_label, 0, row, 1, 1);
                grid.attach(&version_value, 1, row, 1, 1);
            }
        } else if let Some(hint) = &info.install_hint {
            // Installation hint (only if not installed)
            let hint_label = Label::builder()
                .label("Install:")
                .halign(gtk4::Align::End)
                .valign(gtk4::Align::Start)
                .build();
            let hint_value = Label::builder()
                .label(hint)
                .halign(gtk4::Align::Start)
                .selectable(true)
                .wrap(true)
                .max_width_chars(50)
                .css_classes(["dim-label"])
                .build();
            grid.attach(&hint_label, 0, row, 1, 1);
            grid.attach(&hint_value, 1, row, 1, 1);
        }

        Frame::builder().label(title).child(&grid).build()
    }

    /// Populates the dialog with existing settings
    pub fn set_settings(&self, settings: &AppSettings) {
        *self.settings.borrow_mut() = settings.clone();

        // Terminal settings
        self.font_family_entry
            .set_text(&settings.terminal.font_family);
        self.font_size_spin
            .set_value(f64::from(settings.terminal.font_size));
        self.scrollback_spin
            .set_value(f64::from(settings.terminal.scrollback_lines));

        // Logging settings
        self.logging_enabled_switch
            .set_state(settings.logging.enabled);
        self.logging_enabled_switch
            .set_active(settings.logging.enabled);
        self.log_dir_entry
            .set_text(&settings.logging.log_directory.to_string_lossy());
        self.retention_spin
            .set_value(f64::from(settings.logging.retention_days));
        self.log_dir_entry.set_sensitive(settings.logging.enabled);
        self.retention_spin.set_sensitive(settings.logging.enabled);
        self.open_logs_button
            .set_sensitive(settings.logging.enabled);
        // Logging mode checkboxes
        self.log_activity_check
            .set_active(settings.logging.log_activity);
        self.log_input_check.set_active(settings.logging.log_input);
        self.log_output_check
            .set_active(settings.logging.log_output);
        self.log_activity_check
            .set_sensitive(settings.logging.enabled);
        self.log_input_check.set_sensitive(settings.logging.enabled);
        self.log_output_check
            .set_sensitive(settings.logging.enabled);

        // Secret settings - using dropdown index
        let backend_idx = match settings.secrets.preferred_backend {
            SecretBackendType::KeePassXc => 0,
            SecretBackendType::KdbxFile => 1,
            SecretBackendType::LibSecret => 2,
        };
        self.secret_backend_dropdown.set_selected(backend_idx);
        self.enable_fallback
            .set_active(settings.secrets.enable_fallback);

        // KeePass settings
        self.kdbx_enabled_switch
            .set_state(settings.secrets.kdbx_enabled);
        self.kdbx_enabled_switch
            .set_active(settings.secrets.kdbx_enabled);
        if let Some(ref path) = settings.secrets.kdbx_path {
            self.kdbx_path_entry.set_text(&path.to_string_lossy());
        } else {
            self.kdbx_path_entry.set_text("");
        }

        // Check if we have a saved password
        let has_saved_password = settings.secrets.kdbx_password_encrypted.is_some();
        self.kdbx_save_password_check.set_active(has_saved_password);

        // Key file settings
        let use_key_file = settings.secrets.kdbx_use_key_file;
        self.kdbx_use_key_file_check.set_active(use_key_file);
        if let Some(ref path) = settings.secrets.kdbx_key_file {
            self.kdbx_key_file_entry.set_text(&path.to_string_lossy());
        } else {
            self.kdbx_key_file_entry.set_text("");
        }

        // Update sensitivity based on enabled state and key file mode
        let enabled = settings.secrets.kdbx_enabled;
        self.kdbx_path_entry.set_sensitive(enabled);
        self.kdbx_browse_button.set_sensitive(enabled);
        self.kdbx_use_key_file_check.set_sensitive(enabled);

        // Password fields sensitive only if enabled AND not using key file
        self.kdbx_password_entry
            .set_sensitive(enabled && !use_key_file);
        self.kdbx_save_password_check
            .set_sensitive(enabled && !use_key_file);

        // Key file fields sensitive only if enabled AND using key file
        self.kdbx_key_file_entry
            .set_sensitive(enabled && use_key_file);
        self.kdbx_key_file_browse_button
            .set_sensitive(enabled && use_key_file);

        // Update status label
        let has_key_file = settings.secrets.kdbx_key_file.is_some();
        if enabled && (has_saved_password || (use_key_file && has_key_file)) {
            self.kdbx_status_label.set_text("✅ Enabled");
            self.kdbx_status_label.remove_css_class("dim-label");
            self.kdbx_status_label.add_css_class("success");
            // Show placeholder for saved password if not using key file
            if !use_key_file && has_saved_password {
                self.kdbx_password_entry.set_text("••••••••");
            }
        } else {
            // Clear status for both enabled without saved password and disabled states
            self.kdbx_status_label.set_text("");
        }

        // Refresh KeePassXC status
        Self::populate_keepassxc_status(&self.keepassxc_status_container);

        // UI settings
        self.remember_geometry
            .set_active(settings.ui.remember_window_geometry);
        self.enable_tray_icon
            .set_active(settings.ui.enable_tray_icon);
        self.minimize_to_tray
            .set_active(settings.ui.minimize_to_tray);
        // Update sensitivity of minimize_to_tray based on enable_tray_icon
        self.minimize_to_tray
            .set_sensitive(settings.ui.enable_tray_icon);

        // Session restore settings
        self.session_restore_enabled
            .set_active(settings.ui.session_restore.enabled);
        self.prompt_on_restore
            .set_active(settings.ui.session_restore.prompt_on_restore);
        self.max_age_spin
            .set_value(f64::from(settings.ui.session_restore.max_age_hours));
        // Update sensitivity based on enabled state
        let restore_enabled = settings.ui.session_restore.enabled;
        self.prompt_on_restore.set_sensitive(restore_enabled);
        self.max_age_spin.set_sensitive(restore_enabled);
    }

    /// Runs the dialog and calls the callback with the result
    ///
    /// The save button handler is connected once in the constructor.
    /// This method just stores the callback and presents the dialog.
    pub fn run<F: Fn(Option<AppSettings>) + 'static>(&self, cb: F) {
        // Store callback - the save button handler (connected in constructor) will use this
        *self.on_save.borrow_mut() = Some(Box::new(cb));

        // Connect SSH Agent buttons
        self.connect_ssh_agent_buttons();

        // Refresh SSH Agent status on dialog open
        self.refresh_ssh_agent_status();

        self.window.present();
    }

    /// Connects the SSH Agent button functionality
    fn connect_ssh_agent_buttons(&self) {
        // Connect Start Agent button
        let manager = self.ssh_agent_manager.clone();
        let status_label = self.ssh_agent_status_label.clone();
        let socket_label = self.ssh_agent_socket_label.clone();
        let start_button = self.ssh_agent_start_button.clone();
        let add_key_button = self.ssh_agent_add_key_button.clone();
        let keys_list = self.ssh_agent_keys_list.clone();
        let error_label = self.ssh_agent_error_label.clone();

        self.ssh_agent_start_button.connect_clicked(move |_| {
            match SshAgentManager::start_agent() {
                Ok(socket_path) => {
                    // Update manager with new socket path
                    manager
                        .borrow_mut()
                        .set_socket_path(Some(socket_path.clone()));

                    // Update UI
                    status_label.set_text("✓ Running");
                    status_label.remove_css_class("error");
                    status_label.add_css_class("success");
                    socket_label.set_text(&socket_path);
                    start_button.set_sensitive(false);
                    start_button.set_label("Agent Running");
                    add_key_button.set_sensitive(true);
                    error_label.set_visible(false);

                    // Clear keys list (new agent has no keys)
                    while let Some(child) = keys_list.first_child() {
                        keys_list.remove(&child);
                    }
                    let row = ListBoxRow::new();
                    let label = Label::builder()
                        .label("No keys loaded in agent")
                        .halign(gtk4::Align::Start)
                        .margin_top(8)
                        .margin_bottom(8)
                        .margin_start(8)
                        .css_classes(["dim-label"])
                        .build();
                    row.set_child(Some(&label));
                    keys_list.append(&row);
                }
                Err(e) => {
                    error_label.set_text(&format!("Failed to start agent: {e}"));
                    error_label.set_visible(true);
                }
            }
        });

        // Connect Add Key button
        let manager = self.ssh_agent_manager.clone();
        let keys_list = self.ssh_agent_keys_list.clone();
        let error_label = self.ssh_agent_error_label.clone();
        let loading_spinner = self.ssh_agent_loading_spinner.clone();
        let window = self.window.clone();

        self.ssh_agent_add_key_button.connect_clicked(move |_| {
            let dialog = FileDialog::builder()
                .title("Select SSH Private Key")
                .modal(true)
                .build();

            // Set initial folder to ~/.ssh/
            if let Ok(home) = std::env::var("HOME") {
                let ssh_dir = PathBuf::from(home).join(".ssh");
                if ssh_dir.exists() {
                    if let Ok(file) = gtk4::gio::File::for_path(&ssh_dir).query_info(
                        "*",
                        gtk4::gio::FileQueryInfoFlags::NONE,
                        gtk4::gio::Cancellable::NONE,
                    ) {
                        let _ = file;
                        dialog.set_initial_folder(Some(&gtk4::gio::File::for_path(&ssh_dir)));
                    }
                }
            }

            let manager_clone = manager.clone();
            let keys_list_clone = keys_list.clone();
            let error_label_clone = error_label.clone();
            let loading_spinner_clone = loading_spinner.clone();

            dialog.open(Some(&window), gtk4::gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        // Show loading
                        loading_spinner_clone.set_visible(true);
                        loading_spinner_clone.start();

                        // Try to add the key without passphrase first
                        let manager = manager_clone.borrow();
                        match manager.add_key(&path, None) {
                            Ok(()) => {
                                // Refresh the keys list
                                if let Ok(status) = manager.get_status() {
                                    // Clear and repopulate
                                    while let Some(child) = keys_list_clone.first_child() {
                                        keys_list_clone.remove(&child);
                                    }

                                    if status.keys.is_empty() {
                                        let row = ListBoxRow::new();
                                        let label = Label::builder()
                                            .label("No keys loaded in agent")
                                            .halign(gtk4::Align::Start)
                                            .margin_top(8)
                                            .margin_bottom(8)
                                            .margin_start(8)
                                            .css_classes(["dim-label"])
                                            .build();
                                        row.set_child(Some(&label));
                                        keys_list_clone.append(&row);
                                    } else {
                                        for key in &status.keys {
                                            let row = Self::create_key_row(
                                                key,
                                                &manager_clone,
                                                &keys_list_clone,
                                                &error_label_clone,
                                            );
                                            keys_list_clone.append(&row);
                                        }
                                    }
                                }
                                error_label_clone.set_visible(false);
                            }
                            Err(e) => {
                                // Check if it needs a passphrase
                                let err_str = e.to_string();
                                if err_str.contains("passphrase")
                                    || err_str.contains("bad passphrase")
                                {
                                    // Show passphrase dialog
                                    drop(manager); // Release borrow before showing dialog
                                    loading_spinner_clone.stop();
                                    loading_spinner_clone.set_visible(false);

                                    // Get the window from the keys list
                                    if let Some(root) = keys_list_clone.root() {
                                        if let Some(window) = root.downcast_ref::<Window>() {
                                            Self::show_passphrase_dialog(
                                                window,
                                                &path,
                                                &manager_clone,
                                                &keys_list_clone,
                                                &error_label_clone,
                                                &loading_spinner_clone,
                                            );
                                        }
                                    }
                                    return;
                                }
                                error_label_clone.set_text(&format!("Failed to add key: {e}"));
                                error_label_clone.set_visible(true);
                            }
                        }

                        loading_spinner_clone.stop();
                        loading_spinner_clone.set_visible(false);
                    }
                }
            });
        });

        // Connect Refresh button
        let manager = self.ssh_agent_manager.clone();
        let status_label = self.ssh_agent_status_label.clone();
        let socket_label = self.ssh_agent_socket_label.clone();
        let start_button = self.ssh_agent_start_button.clone();
        let add_key_button = self.ssh_agent_add_key_button.clone();
        let keys_list = self.ssh_agent_keys_list.clone();
        let error_label = self.ssh_agent_error_label.clone();
        let loading_spinner = self.ssh_agent_loading_spinner.clone();

        self.ssh_agent_refresh_button.connect_clicked(move |_| {
            // Show loading
            loading_spinner.set_visible(true);
            loading_spinner.start();

            let manager_ref = manager.borrow();
            let status = manager_ref.get_status();

            match status {
                Ok(status) => {
                    if status.running {
                        status_label.set_text("✓ Running");
                        status_label.remove_css_class("error");
                        status_label.add_css_class("success");
                        start_button.set_sensitive(false);
                        start_button.set_label("Agent Running");
                        add_key_button.set_sensitive(true);

                        if let Some(socket) = &status.socket_path {
                            socket_label.set_text(socket);
                        } else {
                            socket_label.set_text("-");
                        }

                        // Clear and repopulate keys list
                        while let Some(child) = keys_list.first_child() {
                            keys_list.remove(&child);
                        }

                        if status.keys.is_empty() {
                            let row = ListBoxRow::new();
                            let label = Label::builder()
                                .label("No keys loaded in agent")
                                .halign(gtk4::Align::Start)
                                .margin_top(8)
                                .margin_bottom(8)
                                .margin_start(8)
                                .css_classes(["dim-label"])
                                .build();
                            row.set_child(Some(&label));
                            keys_list.append(&row);
                        } else {
                            for key in &status.keys {
                                let row =
                                    Self::create_key_row(key, &manager, &keys_list, &error_label);
                                keys_list.append(&row);
                            }
                        }
                        error_label.set_visible(false);
                    } else {
                        status_label.set_text("✗ Not Running");
                        status_label.remove_css_class("success");
                        status_label.add_css_class("error");
                        start_button.set_sensitive(true);
                        start_button.set_label("Start Agent");
                        add_key_button.set_sensitive(false);

                        if let Some(socket) = &status.socket_path {
                            socket_label.set_text(&format!("{socket} (not responding)"));
                        } else {
                            socket_label.set_text("Not configured");
                        }

                        // Clear keys list
                        while let Some(child) = keys_list.first_child() {
                            keys_list.remove(&child);
                        }
                    }
                }
                Err(e) => {
                    status_label.set_text("✗ Error");
                    status_label.remove_css_class("success");
                    status_label.add_css_class("error");
                    socket_label.set_text("-");
                    start_button.set_sensitive(true);
                    add_key_button.set_sensitive(false);
                    error_label.set_text(&format!("Error: {e}"));
                    error_label.set_visible(true);

                    // Clear keys list
                    while let Some(child) = keys_list.first_child() {
                        keys_list.remove(&child);
                    }
                }
            }

            loading_spinner.stop();
            loading_spinner.set_visible(false);
        });
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
    }
}
