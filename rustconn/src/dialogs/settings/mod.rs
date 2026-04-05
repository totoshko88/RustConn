//! Settings dialog using libadwaita PreferencesDialog
//!
//! This module contains the settings dialog using modern Adwaita components
//! for a native GNOME look and feel.
//!
//! Migrated to `PreferencesDialog` (libadwaita 1.5+) from deprecated `PreferencesWindow`.

mod clients_tab;
mod keybindings_tab;
mod logging_tab;
mod monitoring_tab;
mod secrets_tab;
mod ssh_agent_tab;
mod terminal_tab;
mod ui_tab;

pub use clients_tab::*;
pub use keybindings_tab::*;
pub use logging_tab::*;
pub use monitoring_tab::*;
pub use secrets_tab::*;
pub use ssh_agent_tab::*;
pub use terminal_tab::*;
pub use ui_tab::*;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, PasswordEntry, SpinButton};
use libadwaita as adw;
use rustconn_core::config::AppSettings;
use rustconn_core::models::Connection;
use rustconn_core::ssh_agent::SshAgentManager;
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::i18n;

/// Callback type for settings save
pub type SettingsCallback = Option<Rc<dyn Fn(AppSettings)>>;

/// Moves all `PreferencesGroup` children from `source` page to `target` page.
fn move_groups(source: &adw::PreferencesPage, target: &adw::PreferencesPage) {
    // PreferencesPage stores groups inside an internal GtkBox/ListBox.
    // We walk the widget tree to find PreferencesGroup children.
    let mut groups: Vec<adw::PreferencesGroup> = Vec::new();
    let mut child = source.first_child();
    while let Some(widget) = child {
        collect_groups(&widget, &mut groups);
        child = widget.next_sibling();
    }
    for group in groups {
        source.remove(&group);
        target.add(&group);
    }
}

/// Recursively collects `PreferencesGroup` widgets from a widget tree.
fn collect_groups(widget: &gtk4::Widget, groups: &mut Vec<adw::PreferencesGroup>) {
    if let Some(group) = widget.downcast_ref::<adw::PreferencesGroup>() {
        groups.push(group.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        collect_groups(&w, groups);
        child = w.next_sibling();
    }
}

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
    sftp_use_mc_check: CheckButton,
    // Logging settings
    logging_enabled_row: adw::SwitchRow,
    log_dir_entry: Entry,
    retention_spin: SpinButton,
    log_activity_check: CheckButton,
    log_input_check: CheckButton,
    log_output_check: CheckButton,
    log_timestamps_check: CheckButton,
    // Secret settings - now using SecretsPageWidgets struct
    secrets_widgets: SecretsPageWidgets,
    // UI settings
    color_scheme_box: GtkBox,
    language_dropdown: DropDown,
    remember_geometry: CheckButton,
    enable_tray_icon: CheckButton,
    minimize_to_tray: CheckButton,
    // Session restore settings
    session_restore_enabled: CheckButton,
    prompt_on_restore: CheckButton,
    max_age_row: adw::SpinRow,
    // Startup action
    startup_action_dropdown: DropDown,
    // Tab coloring
    color_tabs_by_protocol: CheckButton,
    // SSH Agent settings
    ssh_agent_status_label: Label,
    ssh_agent_socket_label: Label,
    ssh_agent_start_button: Button,
    ssh_agent_keys_list: gtk4::ListBox,
    ssh_agent_add_key_button: Button,
    ssh_agent_loading_spinner: gtk4::Widget,
    ssh_agent_error_label: Label,
    ssh_agent_refresh_button: Button,
    ssh_agent_available_keys_list: gtk4::ListBox,
    ssh_agent_custom_socket_entry: adw::EntryRow,
    ssh_agent_manager: Rc<RefCell<SshAgentManager>>,
    // Monitoring settings
    monitoring_widgets: MonitoringPageWidgets,
    // Keybinding settings
    keybindings_overrides: Rc<RefCell<rustconn_core::config::keybindings::KeybindingSettings>>,
    keybindings_page: adw::PreferencesPage,
    // Global highlight rules
    highlight_rules_list: gtk4::ListBox,
    highlight_rules: Rc<RefCell<Vec<rustconn_core::models::HighlightRule>>>,
    // Current settings
    settings: Rc<RefCell<AppSettings>>,
    // Connections list for startup action dropdown
    connections: Rc<RefCell<Vec<Connection>>>,
    // Callback
    on_save: SettingsCallback,
}

impl SettingsDialog {
    /// Creates a new settings dialog using AdwPreferencesDialog
    #[must_use]
    pub fn new(_parent: Option<&gtk4::Window>) -> Self {
        let dialog = adw::PreferencesDialog::builder()
            .search_enabled(true)
            .content_width(700)
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
            sftp_use_mc_check,
        ) = create_terminal_page();

        let (
            logging_page,
            logging_enabled_row,
            log_dir_entry,
            retention_spin,
            log_activity_check,
            log_input_check,
            log_output_check,
            log_timestamps_check,
        ) = create_logging_page();

        let secrets_widgets = create_secrets_page();

        let (
            ui_page,
            color_scheme_box,
            language_dropdown,
            remember_geometry,
            enable_tray_icon,
            minimize_to_tray,
            session_restore_enabled,
            prompt_on_restore,
            max_age_row,
            startup_action_dropdown,
            color_tabs_by_protocol,
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
            ssh_agent_custom_socket_entry,
        ) = create_ssh_agent_page();

        let clients_page = create_clients_page();

        let (keybindings_page, keybindings_overrides) = create_keybindings_page();

        let monitoring_widgets = MonitoringPageWidgets::new();

        // === GNOME HIG: 4 combined pages ===
        //
        // 1. Terminal   = Terminal + Logging
        // 2. Interface  = UI + Keybindings
        // 3. Secrets    = Secrets + SSH Agent
        // 4. Connection = Clients + Monitoring

        // 1. Terminal page already has terminal groups; add logging groups
        move_groups(&logging_page, &terminal_page);

        // Add global highlight rules group to terminal page
        let hl_group = adw::PreferencesGroup::builder()
            .title(i18n("Highlight Rules"))
            .description(i18n("Global regex-based text highlighting rules"))
            .build();

        let hl_scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .min_content_height(120)
            .build();

        let highlight_rules_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        highlight_rules_list
            .set_placeholder(Some(&gtk4::Label::new(Some(&i18n("No highlight rules")))));
        hl_scrolled.set_child(Some(&highlight_rules_list));
        hl_group.add(&hl_scrolled);

        let hl_btn_box = GtkBox::new(gtk4::Orientation::Horizontal, 8);
        hl_btn_box.set_halign(gtk4::Align::End);
        hl_btn_box.set_margin_top(8);

        let add_hl_button = Button::builder()
            .label(&i18n("Add Rule"))
            .css_classes(["suggested-action"])
            .build();
        hl_btn_box.append(&add_hl_button);
        hl_group.add(&hl_btn_box);
        terminal_page.add(&hl_group);

        let highlight_rules: Rc<RefCell<Vec<rustconn_core::models::HighlightRule>>> =
            Rc::new(RefCell::new(Vec::new()));

        // Wire up add highlight rule button for settings
        {
            let list_clone = highlight_rules_list.clone();
            let rules_clone = highlight_rules.clone();
            add_hl_button.connect_clicked(move |_| {
                let new_rule =
                    rustconn_core::models::HighlightRule::new(String::new(), String::new());
                let rule_id = new_rule.id;
                rules_clone.borrow_mut().push(new_rule.clone());

                let row = create_settings_highlight_rule_row(Some(&new_rule));
                wire_settings_highlight_rule_row(&row, rule_id, &list_clone, &rules_clone);
                list_clone.append(&row);
            });
        }

        // 2. UI page already has UI groups; add keybinding groups
        move_groups(&keybindings_page, &ui_page);

        // 3. Secrets page already has secrets groups; add SSH agent groups
        move_groups(&ssh_agent_page, &secrets_widgets.page);

        // 4. Create a combined Connection page for clients + monitoring
        let connection_page = adw::PreferencesPage::builder()
            .title(i18n("Connection"))
            .icon_name("network-server-symbolic")
            .build();
        move_groups(&clients_page, &connection_page);
        move_groups(&monitoring_widgets.page, &connection_page);

        // Add only the 4 combined pages
        dialog.add(&terminal_page);
        dialog.add(&ui_page);
        dialog.add(&secrets_widgets.page);
        dialog.add(&connection_page);

        // Initialize settings
        let settings: Rc<RefCell<AppSettings>> = Rc::new(RefCell::new(AppSettings::default()));

        // Initialize SSH Agent manager from environment
        let ssh_agent_manager = Rc::new(RefCell::new(SshAgentManager::from_env()));

        // === Backup / Restore group on the UI page ===
        let backup_group = adw::PreferencesGroup::builder()
            .title(gtk4::glib::markup_escape_text(&i18n("Backup & Restore")))
            .description(i18n("Export or import all settings as a ZIP archive"))
            .build();

        let backup_btn = Button::builder()
            .label(i18n("Backup Settings…"))
            .tooltip_text(i18n("Save all configuration files to a ZIP archive"))
            .build();
        let restore_btn = Button::builder()
            .label(i18n("Restore Settings…"))
            .tooltip_text(i18n(
                "Load configuration from a ZIP archive (restart required)",
            ))
            .css_classes(["destructive-action"])
            .build();

        let btn_box = GtkBox::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .halign(gtk4::Align::Center)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        btn_box.append(&backup_btn);
        btn_box.append(&restore_btn);
        backup_group.add(&btn_box);
        ui_page.add(&backup_group);

        // Backup handler
        let dialog_weak = dialog.downgrade();
        backup_btn.connect_clicked(move |_| {
            let Some(dlg) = dialog_weak.upgrade() else {
                return;
            };
            let Some(root) = dlg.root() else { return };
            let Some(win) = root.downcast_ref::<gtk4::Window>() else {
                return;
            };
            let file_dialog = gtk4::FileDialog::builder()
                .title(i18n("Save Backup"))
                .initial_name("rustconn-backup.zip")
                .build();
            let filter = gtk4::FileFilter::new();
            filter.add_pattern("*.zip");
            filter.set_name(Some("ZIP archives"));
            let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            file_dialog.set_filters(Some(&filters));

            let win_clone = win.clone();
            file_dialog.save(Some(win), None::<&gtk4::gio::Cancellable>, move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                match rustconn_core::config::ConfigManager::new() {
                    Ok(mgr) => match mgr.backup_to_archive(&path) {
                        Ok(count) => {
                            let msg = crate::i18n::i18n_f(
                                "Backup saved ({} files)",
                                &[&count.to_string()],
                            );
                            crate::toast::show_toast_on_window(
                                &win_clone,
                                &msg,
                                crate::toast::ToastType::Success,
                            );
                        }
                        Err(e) => {
                            tracing::error!(?e, "Settings backup failed");
                            crate::alert::show_error(
                                &win_clone,
                                &crate::i18n::i18n("Backup Error"),
                                &e.to_string(),
                            );
                        }
                    },
                    Err(e) => {
                        tracing::error!(?e, "Cannot create ConfigManager for backup");
                    }
                }
            });
        });

        // Restore handler
        let dialog_weak = dialog.downgrade();
        restore_btn.connect_clicked(move |_| {
            let Some(dlg) = dialog_weak.upgrade() else {
                return;
            };
            let Some(root) = dlg.root() else { return };
            let Some(win) = root.downcast_ref::<gtk4::Window>() else {
                return;
            };
            let file_dialog = gtk4::FileDialog::builder()
                .title(i18n("Open Backup"))
                .build();
            let filter = gtk4::FileFilter::new();
            filter.add_pattern("*.zip");
            filter.set_name(Some("ZIP archives"));
            let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            file_dialog.set_filters(Some(&filters));

            let win_clone = win.clone();
            file_dialog.open(Some(win), None::<&gtk4::gio::Cancellable>, move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };

                // Confirm before overwriting
                let confirm = adw::AlertDialog::new(
                    Some(&crate::i18n::i18n("Restore Settings?")),
                    Some(&crate::i18n::i18n(
                        "This will overwrite current settings. A restart is required to apply changes.",
                    )),
                );
                confirm.add_response("cancel", &crate::i18n::i18n("Cancel"));
                confirm.add_response("restore", &crate::i18n::i18n("Restore"));
                confirm.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
                confirm.set_default_response(Some("cancel"));
                confirm.set_close_response("cancel");

                let win_inner = win_clone.clone();
                let path_clone = path.clone();
                confirm.connect_response(None, move |_, response| {
                    if response != "restore" {
                        return;
                    }
                    match rustconn_core::config::ConfigManager::new() {
                        Ok(mgr) => match mgr.restore_from_archive(&path_clone) {
                            Ok(count) => {
                                let msg = crate::i18n::i18n_f(
                                    "Restored {} files. Restart to apply.",
                                    &[&count.to_string()],
                                );
                                crate::toast::show_toast_on_window(
                                    &win_inner,
                                    &msg,
                                    crate::toast::ToastType::Success,
                                );
                            }
                            Err(e) => {
                                tracing::error!(?e, "Settings restore failed");
                                crate::alert::show_error(
                                    &win_inner,
                                    &crate::i18n::i18n("Restore Error"),
                                    &e.to_string(),
                                );
                            }
                        },
                        Err(e) => {
                            tracing::error!(?e, "Cannot create ConfigManager for restore");
                        }
                    }
                });

                let widget = win_clone.upcast_ref::<gtk4::Widget>();
                confirm.present(Some(widget));
            });
        });

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
            sftp_use_mc_check,
            logging_enabled_row,
            log_dir_entry,
            retention_spin,
            log_activity_check,
            log_input_check,
            log_output_check,
            log_timestamps_check,
            secrets_widgets,
            color_scheme_box,
            language_dropdown,
            remember_geometry,
            enable_tray_icon,
            minimize_to_tray,
            session_restore_enabled,
            prompt_on_restore,
            max_age_row,
            startup_action_dropdown,
            color_tabs_by_protocol,
            ssh_agent_status_label,
            ssh_agent_socket_label,
            ssh_agent_start_button,
            ssh_agent_keys_list,
            ssh_agent_add_key_button,
            ssh_agent_loading_spinner,
            ssh_agent_error_label,
            ssh_agent_refresh_button,
            ssh_agent_available_keys_list,
            ssh_agent_custom_socket_entry,
            ssh_agent_manager,
            monitoring_widgets,
            keybindings_overrides,
            keybindings_page,
            highlight_rules_list,
            highlight_rules,
            settings,
            connections: Rc::new(RefCell::new(Vec::new())),
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

    /// Sets the connections list for the startup action dropdown
    pub fn set_connections(&self, connections: Vec<Connection>) {
        *self.connections.borrow_mut() = connections;
    }

    /// Shows the dialog and loads current settings
    pub fn run<F>(&self, parent: Option<&impl IsA<gtk4::Widget>>, callback: F)
    where
        F: Fn(Option<AppSettings>) + 'static,
    {
        // Present the dialog immediately so the window appears without delay.
        // Settings loading and async operations populate widgets afterwards.
        self.dialog.present(parent);

        // Setup close handler - auto-save on close for PreferencesDialog
        let callback_rc = Rc::new(callback);
        self.setup_close_handler(callback_rc);

        // Load settings into UI (sync widget properties + async background tasks)
        let settings = self.settings.borrow().clone();
        self.load_settings(&settings);

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

        // Connect SSH Agent Start button handler
        {
            let manager_clone = self.ssh_agent_manager.clone();
            let keys_list_clone = self.ssh_agent_keys_list.clone();
            let status_label_clone = self.ssh_agent_status_label.clone();
            let socket_label_clone = self.ssh_agent_socket_label.clone();
            let available_keys_list_clone = self.ssh_agent_available_keys_list.clone();

            self.ssh_agent_start_button.connect_clicked(move |_| {
                // Try to start the agent
                match SshAgentManager::start_agent() {
                    Ok(socket_path) => {
                        tracing::info!("SSH agent started with socket: {socket_path}");
                        // Store agent info globally so that child processes
                        // receive SSH_AUTH_SOCK via apply_agent_env().
                        rustconn_core::sftp::set_agent_info(rustconn_core::sftp::SshAgentInfo {
                            socket_path: socket_path.clone(),
                            pid: None,
                        });
                        // Update the manager with the new socket path
                        manager_clone
                            .borrow_mut()
                            .set_socket_path(Some(socket_path));
                        // Refresh the UI
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
                    }
                    Err(e) => {
                        tracing::error!("Failed to start SSH agent: {e}");
                        status_label_clone.set_text(&i18n("Failed to start"));
                        status_label_clone.remove_css_class("success");
                        status_label_clone.remove_css_class("dim-label");
                        status_label_clone.add_css_class("error");
                    }
                }
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
            &self.sftp_use_mc_check,
            &settings.terminal,
        );

        // Load logging settings
        load_logging_settings(
            &self.logging_enabled_row,
            &self.log_dir_entry,
            &self.retention_spin,
            &self.log_activity_check,
            &self.log_input_check,
            &self.log_output_check,
            &self.log_timestamps_check,
            &settings.logging,
            settings.terminal.log_timestamps,
        );

        // Load secret settings
        load_secret_settings(&self.secrets_widgets, &settings.secrets);

        // Load UI settings
        let conn_list = self.connections.borrow();
        let conn_refs: Vec<&Connection> = conn_list.iter().collect();
        load_ui_settings(
            &self.color_scheme_box,
            &self.language_dropdown,
            &self.remember_geometry,
            &self.enable_tray_icon,
            &self.minimize_to_tray,
            &self.session_restore_enabled,
            &self.prompt_on_restore,
            &self.max_age_row,
            &self.startup_action_dropdown,
            &self.color_tabs_by_protocol,
            &settings.ui,
            &conn_refs,
        );
        drop(conn_refs);
        drop(conn_list);

        // Load SSH agent settings
        load_ssh_agent_settings(
            &self.ssh_agent_status_label,
            &self.ssh_agent_socket_label,
            &self.ssh_agent_keys_list,
            &self.ssh_agent_manager,
        );

        // Load custom SSH agent socket path
        if let Some(ref socket_path) = settings.ssh_agent_socket {
            self.ssh_agent_custom_socket_entry.set_text(socket_path);
        } else {
            self.ssh_agent_custom_socket_entry.set_text("");
        }

        // Populate available keys list with working buttons
        populate_available_keys_list(
            &self.ssh_agent_available_keys_list,
            &self.ssh_agent_manager,
            &self.ssh_agent_keys_list,
            &self.ssh_agent_status_label,
            &self.ssh_agent_socket_label,
        );

        // Load keybinding settings
        load_keybinding_settings(
            &self.keybindings_page,
            &self.keybindings_overrides,
            &settings.keybindings,
        );

        // Load monitoring settings
        self.monitoring_widgets.load(&settings.monitoring);

        // Load activity monitor defaults
        self.monitoring_widgets
            .load_activity_monitor(&settings.activity_monitor);

        // Load global highlight rules
        self.load_highlight_rules(&settings.highlight_rules);
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
        let sftp_use_mc_check_clone = self.sftp_use_mc_check.clone();

        // Logging controls
        let logging_enabled_row_clone = self.logging_enabled_row.clone();
        let log_dir_entry_clone = self.log_dir_entry.clone();
        let retention_spin_clone = self.retention_spin.clone();
        let log_activity_check_clone = self.log_activity_check.clone();
        let log_input_check_clone = self.log_input_check.clone();
        let log_output_check_clone = self.log_output_check.clone();
        let log_timestamps_check_clone = self.log_timestamps_check.clone();

        // Secret controls - clone individual widgets from secrets_widgets
        let secret_backend_dropdown_clone = self.secrets_widgets.secret_backend_dropdown.clone();
        let enable_fallback_clone = self.secrets_widgets.enable_fallback.clone();
        let kdbx_path_entry_clone = self.secrets_widgets.kdbx_path_entry.clone();
        let kdbx_enabled_row_clone = self.secrets_widgets.kdbx_enabled_row.clone();
        let kdbx_password_entry_clone = self.secrets_widgets.kdbx_password_entry.clone();
        let kdbx_save_password_check_clone = self.secrets_widgets.kdbx_save_password_check.clone();
        let kdbx_key_file_entry_clone = self.secrets_widgets.kdbx_key_file_entry.clone();
        let kdbx_use_key_file_check_clone = self.secrets_widgets.kdbx_use_key_file_check.clone();
        let kdbx_use_password_check_clone = self.secrets_widgets.kdbx_use_password_check.clone();
        let bitwarden_password_entry_clone = self.secrets_widgets.bitwarden_password_entry.clone();
        let bitwarden_save_password_check_clone =
            self.secrets_widgets.bitwarden_save_password_check.clone();
        let bitwarden_save_to_keyring_check_clone =
            self.secrets_widgets.bitwarden_save_to_keyring_check.clone();
        let bitwarden_use_api_key_check_clone =
            self.secrets_widgets.bitwarden_use_api_key_check.clone();
        let bitwarden_client_id_entry_clone =
            self.secrets_widgets.bitwarden_client_id_entry.clone();
        let bitwarden_client_secret_entry_clone =
            self.secrets_widgets.bitwarden_client_secret_entry.clone();

        // UI controls
        let color_scheme_box_clone = self.color_scheme_box.clone();
        let language_dropdown_clone = self.language_dropdown.clone();
        let remember_geometry_clone = self.remember_geometry.clone();
        let enable_tray_icon_clone = self.enable_tray_icon.clone();
        let minimize_to_tray_clone = self.minimize_to_tray.clone();
        let session_restore_enabled_clone = self.session_restore_enabled.clone();
        let prompt_on_restore_clone = self.prompt_on_restore.clone();
        let max_age_row_clone = self.max_age_row.clone();
        let startup_action_dropdown_clone = self.startup_action_dropdown.clone();
        let color_tabs_by_protocol_clone = self.color_tabs_by_protocol.clone();
        let connections_clone = self.connections.clone();
        let keybindings_overrides_clone = self.keybindings_overrides.clone();

        // Monitoring controls
        let monitoring_widgets_clone = self.monitoring_widgets.clone();

        // Highlight rules
        let highlight_rules_clone = self.highlight_rules.clone();

        // SSH agent custom socket entry
        let ssh_agent_custom_socket_entry_clone = self.ssh_agent_custom_socket_entry.clone();

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
                &sftp_use_mc_check_clone,
                log_timestamps_check_clone.is_active(),
            );

            // Collect logging settings
            let logging = collect_logging_settings(
                &logging_enabled_row_clone,
                &log_dir_entry_clone,
                &retention_spin_clone,
                &log_activity_check_clone,
                &log_input_check_clone,
                &log_output_check_clone,
            );

            // Collect secret settings - build temporary struct for collect function
            let secrets_widgets_for_collect = SecretsPageWidgets {
                page: adw::PreferencesPage::new(), // dummy, not used in collect
                secret_backend_dropdown: secret_backend_dropdown_clone.clone(),
                enable_fallback: enable_fallback_clone.clone(),
                kdbx_path_entry: kdbx_path_entry_clone.clone(),
                kdbx_password_entry: kdbx_password_entry_clone.clone(),
                kdbx_enabled_row: kdbx_enabled_row_clone.clone(),
                kdbx_save_password_check: kdbx_save_password_check_clone.clone(),
                kdbx_status_label: Label::new(None), // dummy, not used in collect
                kdbx_browse_button: Button::new(),   // dummy, not used in collect
                kdbx_check_button: Button::new(),    // dummy, not used in collect
                keepassxc_status_container: GtkBox::new(gtk4::Orientation::Vertical, 0),
                kdbx_key_file_entry: kdbx_key_file_entry_clone.clone(),
                kdbx_key_file_browse_button: Button::new(), // dummy
                kdbx_use_key_file_check: kdbx_use_key_file_check_clone.clone(),
                kdbx_use_password_check: kdbx_use_password_check_clone.clone(),
                kdbx_group: adw::PreferencesGroup::new(), // dummy
                auth_group: adw::PreferencesGroup::new(), // dummy
                status_group: adw::PreferencesGroup::new(), // dummy
                password_row: adw::ActionRow::new(),      // dummy
                save_password_row: adw::ActionRow::new(), // dummy
                key_file_row: adw::ActionRow::new(),      // dummy
                bitwarden_group: adw::PreferencesGroup::new(), // dummy
                bitwarden_status_label: Label::new(None), // dummy
                bitwarden_unlock_button: Button::new(),   // dummy
                bitwarden_password_entry: bitwarden_password_entry_clone.clone(),
                bitwarden_save_password_check: bitwarden_save_password_check_clone.clone(),
                bitwarden_save_to_keyring_check: bitwarden_save_to_keyring_check_clone.clone(),
                bitwarden_use_api_key_check: bitwarden_use_api_key_check_clone.clone(),
                bitwarden_client_id_entry: bitwarden_client_id_entry_clone.clone(),
                bitwarden_client_secret_entry: bitwarden_client_secret_entry_clone.clone(),
                bitwarden_cmd: Rc::new(RefCell::new(String::new())), // dummy, not used in collect
                onepassword_group: adw::PreferencesGroup::new(),     // dummy
                onepassword_status_label: Label::new(None),          // dummy
                onepassword_signin_button: Button::new(),            // dummy
                onepassword_cmd: Rc::new(RefCell::new(String::new())), // dummy, not used in collect
                passbolt_group: adw::PreferencesGroup::new(),        // dummy
                passbolt_status_label: Label::new(None),             // dummy
                passbolt_server_url_entry: Entry::new(),             // dummy
                passbolt_open_vault_button: Button::new(),           // dummy
                passbolt_passphrase_entry: PasswordEntry::new(),     // dummy
                passbolt_save_password_check: CheckButton::new(),    // dummy
                passbolt_save_to_keyring_check: CheckButton::new(),  // dummy
                kdbx_save_to_keyring_check: CheckButton::new(),      // dummy
                onepassword_token_entry: PasswordEntry::new(),       // dummy
                onepassword_save_password_check: CheckButton::new(), // dummy
                onepassword_save_to_keyring_check: CheckButton::new(), // dummy
                secret_tool_available: Rc::new(RefCell::new(None)),  // dummy
                pass_group: adw::PreferencesGroup::new(),            // dummy
                pass_store_dir_entry: Entry::new(),                  // dummy
                pass_store_dir_browse_button: Button::new(),         // dummy
                pass_status_label: Label::new(None),                 // dummy
            };
            let secrets = collect_secret_settings(&secrets_widgets_for_collect, &settings_clone);

            // Collect UI settings
            let conn_list = connections_clone.borrow();
            let conn_refs: Vec<&Connection> = conn_list.iter().collect();
            let ui = collect_ui_settings(
                &color_scheme_box_clone,
                &language_dropdown_clone,
                &remember_geometry_clone,
                &enable_tray_icon_clone,
                &minimize_to_tray_clone,
                &session_restore_enabled_clone,
                &prompt_on_restore_clone,
                &max_age_row_clone,
                &startup_action_dropdown_clone,
                &color_tabs_by_protocol_clone,
                &conn_refs,
            );
            drop(conn_refs);
            drop(conn_list);

            // Create new settings
            let new_settings = AppSettings {
                terminal,
                logging,
                secrets,
                ui,
                connection: settings_clone.borrow().connection.clone(),
                global_variables: settings_clone.borrow().global_variables.clone(),
                history: settings_clone.borrow().history.clone(),
                keybindings: collect_keybinding_settings(&keybindings_overrides_clone),
                monitoring: monitoring_widgets_clone.collect(),
                activity_monitor: monitoring_widgets_clone.collect_activity_monitor(),
                highlight_rules: highlight_rules_clone
                    .borrow()
                    .iter()
                    .filter(|r| !r.pattern.is_empty())
                    .cloned()
                    .collect(),
                smart_folders: settings_clone.borrow().smart_folders.clone(),
                ssh_agent_socket: {
                    let text = ssh_agent_custom_socket_entry_clone.text();
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                },
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

    /// Loads global highlight rules into the settings dialog
    fn load_highlight_rules(&self, rules: &[rustconn_core::models::HighlightRule]) {
        // Clear existing rows
        while let Some(row) = self.highlight_rules_list.row_at_index(0) {
            self.highlight_rules_list.remove(&row);
        }
        self.highlight_rules.borrow_mut().clear();

        for rule in rules {
            self.highlight_rules.borrow_mut().push(rule.clone());
            let row = create_settings_highlight_rule_row(Some(rule));
            wire_settings_highlight_rule_row(
                &row,
                rule.id,
                &self.highlight_rules_list,
                &self.highlight_rules,
            );
            self.highlight_rules_list.append(&row);
        }
    }

    /// Returns a reference to the dialog for toast notifications
    pub fn dialog(&self) -> &adw::PreferencesDialog {
        &self.dialog
    }
}

/// Creates a highlight rule row for the settings dialog ListBox.
fn create_settings_highlight_rule_row(
    rule: Option<&rustconn_core::models::HighlightRule>,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .build();

    let hbox = GtkBox::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    let name_entry = gtk4::Entry::builder()
        .placeholder_text(i18n("Name"))
        .width_chars(12)
        .tooltip_text(i18n("Rule name"))
        .build();
    if let Some(r) = rule {
        name_entry.set_text(&r.name);
    }
    name_entry.set_widget_name("hl_name");

    let pattern_entry = gtk4::Entry::builder()
        .placeholder_text(i18n("Pattern (regex)"))
        .hexpand(true)
        .tooltip_text(i18n("Regex pattern to match"))
        .build();
    if let Some(r) = rule {
        pattern_entry.set_text(&r.pattern);
    }
    pattern_entry.set_widget_name("hl_pattern");

    let enabled_check = CheckButton::builder()
        .active(rule.is_none_or(|r| r.enabled))
        .tooltip_text(i18n("Enable rule"))
        .valign(gtk4::Align::Center)
        .build();
    enabled_check.set_widget_name("hl_enabled");

    let delete_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Delete rule"))
        .valign(gtk4::Align::Center)
        .build();
    delete_button.set_widget_name("hl_delete");

    hbox.append(&name_entry);
    hbox.append(&pattern_entry);
    hbox.append(&enabled_check);
    hbox.append(&delete_button);

    row.set_child(Some(&hbox));
    row
}

/// Wires up a settings highlight rule row's signals to update the rules data.
fn wire_settings_highlight_rule_row(
    row: &gtk4::ListBoxRow,
    rule_id: uuid::Uuid,
    list: &gtk4::ListBox,
    rules: &Rc<RefCell<Vec<rustconn_core::models::HighlightRule>>>,
) {
    // Find child widgets by name
    let Some(hbox_widget) = row.child() else {
        return;
    };
    let Some(hbox) = hbox_widget.downcast_ref::<GtkBox>() else {
        return;
    };

    let mut child = hbox.first_child();
    while let Some(widget) = child {
        let name = widget.widget_name();
        if name == "hl_name" {
            if let Some(entry) = widget.downcast_ref::<gtk4::Entry>() {
                let rules_clone = rules.clone();
                let id = rule_id;
                entry.connect_changed(move |e| {
                    let text = e.text().to_string();
                    let mut r = rules_clone.borrow_mut();
                    if let Some(rule) = r.iter_mut().find(|r| r.id == id) {
                        rule.name = text;
                    }
                });
            }
        } else if name == "hl_pattern" {
            if let Some(entry) = widget.downcast_ref::<gtk4::Entry>() {
                let rules_clone = rules.clone();
                let id = rule_id;
                entry.connect_changed(move |e| {
                    let text = e.text().to_string();
                    let mut r = rules_clone.borrow_mut();
                    if let Some(rule) = r.iter_mut().find(|r| r.id == id) {
                        rule.pattern = text;
                    }
                });
            }
        } else if name == "hl_enabled" {
            if let Some(check) = widget.downcast_ref::<CheckButton>() {
                let rules_clone = rules.clone();
                let id = rule_id;
                check.connect_toggled(move |c| {
                    let active = c.is_active();
                    let mut r = rules_clone.borrow_mut();
                    if let Some(rule) = r.iter_mut().find(|r| r.id == id) {
                        rule.enabled = active;
                    }
                });
            }
        } else if name == "hl_delete"
            && let Some(button) = widget.downcast_ref::<Button>()
        {
            let list_clone = list.clone();
            let rules_clone = rules.clone();
            let row_clone = row.clone();
            let id = rule_id;
            button.connect_clicked(move |_| {
                list_clone.remove(&row_clone);
                rules_clone.borrow_mut().retain(|r| r.id != id);
            });
        }
        child = widget.next_sibling();
    }
}
