//! Settings dialog using libadwaita PreferencesDialog
//!
//! This module contains the settings dialog using modern Adwaita components
//! for a native GNOME look and feel.
//!
//! Migrated to `PreferencesDialog` (libadwaita 1.5+) from deprecated `PreferencesWindow`.

mod clients_tab;
mod logging_tab;
mod secrets_tab;
mod ssh_agent_tab;
mod terminal_tab;
mod ui_tab;

pub use clients_tab::*;
pub use logging_tab::*;
pub use secrets_tab::*;
pub use ssh_agent_tab::*;
pub use terminal_tab::*;
pub use ui_tab::*;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, PasswordEntry, SpinButton, Spinner,
    Switch,
};
use libadwaita as adw;
use rustconn_core::config::AppSettings;
use rustconn_core::ssh_agent::SshAgentManager;
use std::cell::RefCell;
use std::rc::Rc;

/// Callback type for settings save
pub type SettingsCallback = Option<Rc<dyn Fn(AppSettings)>>;

/// Main settings dialog using AdwPreferencesDialog (libadwaita 1.5+)
#[allow(dead_code)] // Fields kept for GTK widget lifecycle
pub struct SettingsDialog {
    dialog: adw::PreferencesDialog,
    // Terminal settings
    font_family_entry: Entry,
    font_size_spin: SpinButton,
    scrollback_spin: SpinButton,
    color_theme_dropdown: DropDown,
    cursor_shape_buttons: GtkBox,
    cursor_blink_buttons: GtkBox,
    scroll_on_output_check: CheckButton,
    scroll_on_keystroke_check: CheckButton,
    allow_hyperlinks_check: CheckButton,
    mouse_autohide_check: CheckButton,
    audible_bell_check: CheckButton,
    // Logging settings
    logging_enabled_switch: Switch,
    log_dir_entry: Entry,
    retention_spin: SpinButton,
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
    kdbx_use_key_file_check: Switch,
    kdbx_use_password_check: Switch,
    // UI settings
    color_scheme_box: GtkBox,
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
    ssh_agent_keys_list: gtk4::ListBox,
    ssh_agent_add_key_button: Button,
    ssh_agent_loading_spinner: Spinner,
    ssh_agent_error_label: Label,
    ssh_agent_refresh_button: Button,
    ssh_agent_available_keys_list: gtk4::ListBox,
    ssh_agent_manager: Rc<RefCell<SshAgentManager>>,
    // Current settings
    settings: Rc<RefCell<AppSettings>>,
    // Callback
    on_save: SettingsCallback,
}

impl SettingsDialog {
    /// Creates a new settings dialog using AdwPreferencesDialog
    #[must_use]
    pub fn new(_parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::PreferencesDialog::builder()
            .search_enabled(true)
            .build();

        // Create all pages
        let (
            terminal_page,
            font_family_entry,
            font_size_spin,
            scrollback_spin,
            color_theme_dropdown,
            cursor_shape_buttons,
            cursor_blink_buttons,
            scroll_on_output_check,
            scroll_on_keystroke_check,
            allow_hyperlinks_check,
            mouse_autohide_check,
            audible_bell_check,
        ) = create_terminal_page();

        let (
            logging_page,
            logging_enabled_switch,
            log_dir_entry,
            retention_spin,
            log_activity_check,
            log_input_check,
            log_output_check,
        ) = create_logging_page();

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
            kdbx_use_password_check,
        ) = create_secrets_page();

        let (
            ui_page,
            color_scheme_box,
            remember_geometry,
            enable_tray_icon,
            minimize_to_tray,
            session_restore_enabled,
            prompt_on_restore,
            max_age_spin,
        ) = create_ui_page();

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
            ssh_agent_available_keys_list,
        ) = create_ssh_agent_page();

        let clients_page = create_clients_page();

        // Add pages to dialog
        dialog.add(&terminal_page);
        dialog.add(&logging_page);
        dialog.add(&secrets_page);
        dialog.add(&ui_page);
        dialog.add(&ssh_agent_page);
        dialog.add(&clients_page);

        // Initialize settings
        let settings: Rc<RefCell<AppSettings>> = Rc::new(RefCell::new(AppSettings::default()));

        // Initialize SSH Agent manager from environment
        let ssh_agent_manager = Rc::new(RefCell::new(SshAgentManager::from_env()));

        Self {
            dialog,
            font_family_entry,
            font_size_spin,
            scrollback_spin,
            color_theme_dropdown,
            cursor_shape_buttons,
            cursor_blink_buttons,
            scroll_on_output_check,
            scroll_on_keystroke_check,
            allow_hyperlinks_check,
            mouse_autohide_check,
            audible_bell_check,
            logging_enabled_switch,
            log_dir_entry,
            retention_spin,
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
            kdbx_use_password_check,
            color_scheme_box,
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
            ssh_agent_available_keys_list,
            ssh_agent_manager,
            settings,
            on_save: None,
        }
    }

    /// Sets the callback for when settings are saved
    pub fn set_on_save<F>(&mut self, callback: F)
    where
        F: Fn(AppSettings) + 'static,
    {
        self.on_save = Some(Rc::new(callback));
    }

    /// Sets the current settings
    pub fn set_settings(&mut self, settings: AppSettings) {
        *self.settings.borrow_mut() = settings;
    }

    /// Shows the dialog and loads current settings
    pub fn run<F>(&self, parent: Option<&impl IsA<gtk4::Widget>>, callback: F)
    where
        F: Fn(Option<AppSettings>) + 'static,
    {
        // Load settings into UI
        let settings = self.settings.borrow().clone();
        self.load_settings(&settings);

        // Setup close handler - auto-save on close for PreferencesDialog
        let callback_rc = Rc::new(callback);
        self.setup_close_handler(callback_rc);

        // Connect SSH Agent Add Key button handler
        {
            let manager_clone = self.ssh_agent_manager.clone();
            let keys_list_clone = self.ssh_agent_keys_list.clone();
            let status_label_clone = self.ssh_agent_status_label.clone();
            let socket_label_clone = self.ssh_agent_socket_label.clone();

            self.ssh_agent_add_key_button
                .connect_clicked(move |button| {
                    show_add_key_file_chooser(
                        button,
                        &manager_clone,
                        &keys_list_clone,
                        &status_label_clone,
                        &socket_label_clone,
                    );
                });
        }

        // Connect SSH Agent Refresh button handler
        {
            let manager_clone = self.ssh_agent_manager.clone();
            let keys_list_clone = self.ssh_agent_keys_list.clone();
            let status_label_clone = self.ssh_agent_status_label.clone();
            let socket_label_clone = self.ssh_agent_socket_label.clone();
            let available_keys_list_clone = self.ssh_agent_available_keys_list.clone();

            self.ssh_agent_refresh_button.connect_clicked(move |_| {
                load_ssh_agent_settings(
                    &status_label_clone,
                    &socket_label_clone,
                    &keys_list_clone,
                    &manager_clone,
                );
                populate_available_keys_list(
                    &available_keys_list_clone,
                    &manager_clone,
                    &keys_list_clone,
                    &status_label_clone,
                    &socket_label_clone,
                );
            });
        }

        // Present the dialog - PreferencesDialog uses present() with parent widget
        self.dialog.present(parent);
    }


    /// Loads settings into the UI controls
    fn load_settings(&self, settings: &AppSettings) {
        // Load terminal settings
        load_terminal_settings(
            &self.font_family_entry,
            &self.font_size_spin,
            &self.scrollback_spin,
            &self.color_theme_dropdown,
            &self.cursor_shape_buttons,
            &self.cursor_blink_buttons,
            &self.scroll_on_output_check,
            &self.scroll_on_keystroke_check,
            &self.allow_hyperlinks_check,
            &self.mouse_autohide_check,
            &self.audible_bell_check,
            &settings.terminal,
        );

        // Load logging settings
        load_logging_settings(
            &self.logging_enabled_switch,
            &self.log_dir_entry,
            &self.retention_spin,
            &self.log_activity_check,
            &self.log_input_check,
            &self.log_output_check,
            &settings.logging,
        );

        // Load secret settings
        load_secret_settings(
            &self.secret_backend_dropdown,
            &self.enable_fallback,
            &self.kdbx_path_entry,
            &self.kdbx_password_entry,
            &self.kdbx_enabled_switch,
            &self.kdbx_save_password_check,
            &self.kdbx_status_label,
            &self.keepassxc_status_container,
            &self.kdbx_key_file_entry,
            &self.kdbx_use_key_file_check,
            &self.kdbx_use_password_check,
            &settings.secrets,
        );

        // Load UI settings
        load_ui_settings(
            &self.color_scheme_box,
            &self.remember_geometry,
            &self.enable_tray_icon,
            &self.minimize_to_tray,
            &self.session_restore_enabled,
            &self.prompt_on_restore,
            &self.max_age_spin,
            &settings.ui,
        );

        // Load SSH agent settings
        load_ssh_agent_settings(
            &self.ssh_agent_status_label,
            &self.ssh_agent_socket_label,
            &self.ssh_agent_keys_list,
            &self.ssh_agent_manager,
        );

        // Populate available keys list with working buttons
        populate_available_keys_list(
            &self.ssh_agent_available_keys_list,
            &self.ssh_agent_manager,
            &self.ssh_agent_keys_list,
            &self.ssh_agent_status_label,
            &self.ssh_agent_socket_label,
        );
    }

    /// Sets up the close handler to collect and save settings
    fn setup_close_handler(&self, external_callback: Rc<dyn Fn(Option<AppSettings>)>) {
        let settings_clone = self.settings.clone();

        // Terminal controls
        let font_family_entry_clone = self.font_family_entry.clone();
        let font_size_spin_clone = self.font_size_spin.clone();
        let scrollback_spin_clone = self.scrollback_spin.clone();
        let color_theme_dropdown_clone = self.color_theme_dropdown.clone();
        let cursor_shape_buttons_clone = self.cursor_shape_buttons.clone();
        let cursor_blink_buttons_clone = self.cursor_blink_buttons.clone();
        let scroll_on_output_check_clone = self.scroll_on_output_check.clone();
        let scroll_on_keystroke_check_clone = self.scroll_on_keystroke_check.clone();
        let allow_hyperlinks_check_clone = self.allow_hyperlinks_check.clone();
        let mouse_autohide_check_clone = self.mouse_autohide_check.clone();
        let audible_bell_check_clone = self.audible_bell_check.clone();

        // Logging controls
        let logging_enabled_switch_clone = self.logging_enabled_switch.clone();
        let log_dir_entry_clone = self.log_dir_entry.clone();
        let retention_spin_clone = self.retention_spin.clone();
        let log_activity_check_clone = self.log_activity_check.clone();
        let log_input_check_clone = self.log_input_check.clone();
        let log_output_check_clone = self.log_output_check.clone();

        // Secret controls
        let secret_backend_dropdown_clone = self.secret_backend_dropdown.clone();
        let enable_fallback_clone = self.enable_fallback.clone();
        let kdbx_path_entry_clone = self.kdbx_path_entry.clone();
        let kdbx_enabled_switch_clone = self.kdbx_enabled_switch.clone();
        let kdbx_password_entry_clone = self.kdbx_password_entry.clone();
        let kdbx_save_password_check_clone = self.kdbx_save_password_check.clone();
        let kdbx_key_file_entry_clone = self.kdbx_key_file_entry.clone();
        let kdbx_use_key_file_check_clone = self.kdbx_use_key_file_check.clone();
        let kdbx_use_password_check_clone = self.kdbx_use_password_check.clone();

        // UI controls
        let color_scheme_box_clone = self.color_scheme_box.clone();
        let remember_geometry_clone = self.remember_geometry.clone();
        let enable_tray_icon_clone = self.enable_tray_icon.clone();
        let minimize_to_tray_clone = self.minimize_to_tray.clone();
        let session_restore_enabled_clone = self.session_restore_enabled.clone();
        let prompt_on_restore_clone = self.prompt_on_restore.clone();
        let max_age_spin_clone = self.max_age_spin.clone();

        // Store callback reference
        let on_save_callback = self.on_save.clone();

        // PreferencesDialog uses connect_closed signal (not connect_close_request)
        self.dialog.connect_closed(move |_| {
            // Collect terminal settings
            let terminal = collect_terminal_settings(
                &font_family_entry_clone,
                &font_size_spin_clone,
                &scrollback_spin_clone,
                &color_theme_dropdown_clone,
                &cursor_shape_buttons_clone,
                &cursor_blink_buttons_clone,
                &scroll_on_output_check_clone,
                &scroll_on_keystroke_check_clone,
                &allow_hyperlinks_check_clone,
                &mouse_autohide_check_clone,
                &audible_bell_check_clone,
            );

            // Collect logging settings
            let logging = collect_logging_settings(
                &logging_enabled_switch_clone,
                &log_dir_entry_clone,
                &retention_spin_clone,
                &log_activity_check_clone,
                &log_input_check_clone,
                &log_output_check_clone,
            );

            // Collect secret settings
            let secrets = collect_secret_settings(
                &secret_backend_dropdown_clone,
                &enable_fallback_clone,
                &kdbx_path_entry_clone,
                &kdbx_password_entry_clone,
                &kdbx_enabled_switch_clone,
                &kdbx_save_password_check_clone,
                &kdbx_key_file_entry_clone,
                &kdbx_use_key_file_check_clone,
                &kdbx_use_password_check_clone,
                &settings_clone,
            );

            // Collect UI settings
            let ui = collect_ui_settings(
                &color_scheme_box_clone,
                &remember_geometry_clone,
                &enable_tray_icon_clone,
                &minimize_to_tray_clone,
                &session_restore_enabled_clone,
                &prompt_on_restore_clone,
                &max_age_spin_clone,
            );

            // Create new settings
            let new_settings = AppSettings {
                terminal,
                logging,
                secrets,
                ui,
                global_variables: settings_clone.borrow().global_variables.clone(),
                history: settings_clone.borrow().history.clone(),
            };

            // Update stored settings
            *settings_clone.borrow_mut() = new_settings.clone();

            // Call internal callback if set
            if let Some(ref callback) = on_save_callback {
                callback(new_settings.clone());
            }

            // Call external callback with settings
            external_callback(Some(new_settings));
        });
    }

    /// Returns a reference to the dialog for toast notifications
    pub fn dialog(&self) -> &adw::PreferencesDialog {
        &self.dialog
    }
}
