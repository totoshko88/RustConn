//! Settings dialog using libadwaita PreferencesDialog
//!
//! This module contains the settings dialog using modern Adwaita components
//! for a native GNOME look and feel.
//!
//! Migrated to `PreferencesDialog` (libadwaita 1.5+) from deprecated `PreferencesWindow`.

mod clients_tab;
pub mod cloud_sync_tab;
mod keybindings_tab;
mod logging_tab;
mod monitoring_tab;
mod secrets_tab;
mod ssh_agent_tab;
mod terminal_tab;
mod ui_tab;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
pub use clients_tab::*;
pub use cloud_sync_tab::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DropDown, Entry, Label, SpinButton};
pub use keybindings_tab::*;
use libadwaita as adw;
pub use logging_tab::*;
pub use monitoring_tab::*;
use rustconn_core::config::AppSettings;
use rustconn_core::models::Connection;
use rustconn_core::ssh_agent::SshAgentManager;
pub use secrets_tab::*;
pub use ssh_agent_tab::*;
pub use terminal_tab::*;
pub use ui_tab::*;

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
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct SettingsDialog {
    dialog: adw::PreferencesDialog,
    // Terminal settings
    font_family_entry: Entry,
    font_size_spin: SpinButton,
    scrollback_spin: SpinButton,
    color_theme_dropdown: DropDown,
    cursor_shape_buttons: GtkBox,
    cursor_blink_buttons: GtkBox,
    scroll_on_output_check: adw::SwitchRow,
    scroll_on_keystroke_check: adw::SwitchRow,
    allow_hyperlinks_check: adw::SwitchRow,
    mouse_autohide_check: adw::SwitchRow,
    audible_bell_check: adw::SwitchRow,
    sftp_use_mc_check: adw::SwitchRow,
    copy_on_select_check: adw::SwitchRow,
    show_scrollbar_check: adw::SwitchRow,
    local_shell_command_entry: Entry,
    close_on_clean_exit_check: adw::SwitchRow,
    option_is_meta_check: adw::SwitchRow,
    // Logging settings
    logging_enabled_row: adw::SwitchRow,
    log_dir_entry: Entry,
    retention_spin: SpinButton,
    log_activity_check: adw::SwitchRow,
    log_input_check: adw::SwitchRow,
    log_output_check: adw::SwitchRow,
    log_timestamps_check: adw::SwitchRow,
    // Secret settings - now using SecretsPageWidgets struct
    secrets_widgets: SecretsPageWidgets,
    // UI settings
    color_scheme_box: GtkBox,
    language_dropdown: DropDown,
    remember_geometry: adw::SwitchRow,
    enable_tray_icon: adw::SwitchRow,
    minimize_to_tray: adw::SwitchRow,
    // Session restore settings
    session_restore_enabled: adw::SwitchRow,
    prompt_on_restore: adw::SwitchRow,
    max_age_row: adw::SpinRow,
    // Startup action
    startup_action_dropdown: DropDown,
    // Tab coloring
    color_tabs_by_protocol: adw::SwitchRow,
    // Protocol filter visibility
    show_protocol_filters: adw::SwitchRow,
    // Sidebar width setting
    sidebar_width_row: adw::SpinRow,
    // Compact interface toggle
    compact_ui: adw::SwitchRow,
    // Automatic compact interface on small windows
    compact_auto: adw::SwitchRow,
    // Send terminal control shortcuts to the session toggle (focus-based suspend)
    terminal_passthrough_ctrl: adw::SwitchRow,
    // Show active connection name in the window title (issue #211)
    window_title_shows_connection: adw::SwitchRow,
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
    keybindings_accel_labels: keybindings_tab::AccelLabels,
    keybindings_page: adw::PreferencesPage,
    // Global highlight rules
    highlight_rules_list: gtk4::ListBox,
    highlight_rules: Rc<RefCell<Vec<rustconn_core::models::HighlightRule>>>,
    // Cloud Sync settings
    cloud_sync_widgets: CloudSyncPageWidgets,
    // Current settings
    settings: Rc<RefCell<AppSettings>>,
    // Flag: restore was performed — skip save on close to avoid overwriting
    was_restored: Rc<Cell<bool>>,
    // Connections list for startup action dropdown
    connections: Rc<RefCell<Vec<Connection>>>,
    // Callback
    on_save: SettingsCallback,
}

impl SettingsDialog {
    /// Creates a new settings dialog using AdwPreferencesDialog
    #[must_use]
    pub fn new(_parent: Option<&gtk4::Window>) -> Self {
        // Timing instrumentation: the dialog builds ~6 pages synchronously,
        // and users report slow opening — keep per-page timings at debug
        // level so a `RUST_LOG=debug` run shows where the time goes.
        let build_started = std::time::Instant::now();
        let mut last_mark = build_started;
        let mut mark = move |section: &str| {
            let now = std::time::Instant::now();
            tracing::debug!(
                section,
                elapsed_ms = last_mark.elapsed().as_millis() as u64,
                "settings dialog build section"
            );
            last_mark = now;
        };
        // On macOS the header bar ViewSwitcher shares space with traffic lights,
        // so we need extra width for 6 tab labels to display without truncation.
        #[cfg(target_os = "macos")]
        let width = 1000;
        #[cfg(not(target_os = "macos"))]
        let width = 800;

        let dialog = adw::PreferencesDialog::builder()
            .search_enabled(true)
            .content_width(width)
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
            copy_on_select_check,
            show_scrollbar_check,
            local_shell_command_entry,
            close_on_clean_exit_check,
            option_is_meta_check,
        ) = create_terminal_page();
        mark("terminal_page");

        let (
            _logging_page,
            logging_enabled_row,
            log_dir_entry,
            retention_spin,
            log_activity_check,
            log_input_check,
            log_output_check,
            log_timestamps_check,
        ) = create_logging_page();
        mark("logging_page");

        let secrets_widgets = create_secrets_page();
        mark("secrets_page");

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
            show_protocol_filters,
            sidebar_width_row,
            compact_ui,
            compact_auto,
            terminal_passthrough_ctrl,
            window_title_shows_connection,
        ) = create_ui_page();
        mark("ui_page");

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
        mark("ssh_agent_page");

        let clients_page = create_clients_page();
        mark("clients_page");

        let (keybindings_page, keybindings_overrides, keybindings_accel_labels) =
            create_keybindings_page();
        mark("keybindings_page");

        let monitoring_widgets = MonitoringPageWidgets::new();
        mark("monitoring_page");

        // === GNOME HIG: 5 combined pages ===
        //
        // 1. Terminal    = Terminal + Logging (collapsible)
        // 2. Interface   = UI + Keybindings
        // 3. Secrets     = Secrets + SSH Agent
        // 4. Connection  = Clients
        // 5. Monitoring  = Monitoring + Activity Monitor

        // 1. Terminal page already has terminal groups; add logging as collapsible
        let logging_group = adw::PreferencesGroup::builder().build();

        let logging_expander = adw::ExpanderRow::builder()
            .title(i18n("Session Logging"))
            .subtitle(i18n("Configure session log persistence and content"))
            .show_enable_switch(false)
            .build();

        // Wrap logging controls as rows inside the expander
        logging_expander.add_row(&logging_enabled_row);

        let log_dir_action_row = adw::ActionRow::builder().title(i18n("Directory")).build();
        log_dir_action_row.add_suffix(&log_dir_entry);
        log_dir_action_row.set_activatable_widget(Some(&log_dir_entry));
        logging_expander.add_row(&log_dir_action_row);

        let retention_action_row = adw::ActionRow::builder()
            .title(i18n("Retention"))
            .subtitle(i18n("Days to keep logs"))
            .build();
        retention_action_row.add_suffix(&retention_spin);
        retention_action_row.set_activatable_widget(Some(&retention_spin));
        logging_expander.add_row(&retention_action_row);

        logging_expander.add_row(&log_activity_check);
        logging_expander.add_row(&log_input_check);
        logging_expander.add_row(&log_output_check);
        logging_expander.add_row(&log_timestamps_check);

        logging_group.add(&logging_expander);
        terminal_page.add(&logging_group);

        // Add global highlight rules group to terminal page. One ExpanderRow
        // per rule directly in the group's boxed list — no nested
        // ScrolledWindow (scrolling inside a scrollable preferences page is
        // a GNOME HIG anti-pattern); the page itself scrolls.
        let hl_group = adw::PreferencesGroup::builder()
            .title(i18n("Highlight Rules"))
            .description(i18n("Global regex-based text highlighting rules"))
            .build();

        let add_hl_button = Button::builder()
            .label(i18n("Add Rule"))
            .valign(gtk4::Align::Center)
            .build();
        hl_group.set_header_suffix(Some(&add_hl_button));

        let highlight_rules_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        highlight_rules_list
            .set_placeholder(Some(&gtk4::Label::new(Some(&i18n("No highlight rules")))));
        hl_group.add(&highlight_rules_list);
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
                rules_clone.borrow_mut().push(new_rule.clone());

                let row = build_highlight_rule_row(&new_rule, &list_clone, &rules_clone);
                list_clone.append(&row);
                row.set_expanded(true);
            });
        }

        // 2. UI page already has UI groups; add keybinding groups
        move_groups(&keybindings_page, &ui_page);

        // 3. Secrets page already has secrets groups; add SSH agent groups
        move_groups(&ssh_agent_page, &secrets_widgets.page);

        // 4. Connection page — only clients
        let connection_page = adw::PreferencesPage::builder()
            .title(i18n("Connection"))
            .icon_name("network-server-symbolic")
            .build();
        move_groups(&clients_page, &connection_page);

        // 5. Monitoring as its own page
        // (monitoring_widgets.page already has the correct title/icon)

        // Add all 5 combined pages + Cloud Sync
        dialog.add(&terminal_page);
        dialog.add(&ui_page);
        dialog.add(&secrets_widgets.page);
        dialog.add(&connection_page);
        dialog.add(&monitoring_widgets.page);

        // 6. Cloud Sync page
        let cloud_sync_widgets = create_cloud_sync_page();
        dialog.add(&cloud_sync_widgets.page);
        mark("cloud_sync_page");

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
            filter.set_name(Some(&i18n("ZIP archives")));
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
                                &crate::i18n::i18n("Backup failed"),
                                &e.to_string(),
                            );
                        }
                    },
                    Err(e) => {
                        tracing::error!(?e, "Cannot create ConfigManager for backup");
                        crate::alert::show_error(
                            &win_clone,
                            &crate::i18n::i18n("Backup failed"),
                            &crate::i18n::i18n_f(
                                "Cannot access the configuration directory: {}",
                                &[&e.to_string()],
                            ),
                        );
                    }
                }
            });
        });

        // Restore handler
        let dialog_weak = dialog.downgrade();
        let settings_for_restore = settings.clone();
        let was_restored: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let was_restored_for_restore = was_restored.clone();
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
            filter.set_name(Some(&i18n("ZIP archives")));
            let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            file_dialog.set_filters(Some(&filters));

            let win_clone = win.clone();
            let settings_for_restore = settings_for_restore.clone();
            let was_restored_clone = was_restored_for_restore.clone();
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
                let settings_for_restore_inner = settings_for_restore.clone();
                let was_restored_inner = was_restored_clone.clone();
                confirm.connect_response(None, move |_, response| {
                    if response != "restore" {
                        return;
                    }
                    match rustconn_core::config::ConfigManager::new() {
                        Ok(mgr) => match mgr.restore_from_archive(&path_clone) {
                            Ok(count) => {
                                // Reload settings from disk so the in-memory state
                                // reflects the restored config.toml (including
                                // global_variables). Without this, closing the dialog
                                // would overwrite the restored file with stale data.
                                match mgr.load_settings() {
                                    Ok(restored) => {
                                        *settings_for_restore_inner.borrow_mut() = restored;
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "Restored files but failed to reload settings"
                                        );
                                        crate::toast::show_toast_on_window(
                                            &win_inner,
                                            &crate::i18n::i18n(
                                                "Restored files but failed to reload settings. Please restart.",
                                            ),
                                            crate::toast::ToastType::Warning,
                                        );
                                    }
                                }
                                // Mark as restored so the close handler skips saving
                                // (UI widgets still show pre-restore values and would
                                // overwrite the restored config.toml).
                                was_restored_inner.set(true);

                                // "Restart required" is persistent state, not a
                                // transient result — a toast would vanish before
                                // the user reads it (GNOME HIG: toast vs dialog).
                                let done = adw::AlertDialog::new(
                                    Some(&crate::i18n::i18n("Settings restored")),
                                    Some(&crate::i18n::i18n_f(
                                        "Restored {} files. Restart RustConn to apply the restored settings.",
                                        &[&count.to_string()],
                                    )),
                                );
                                done.add_response("later", &crate::i18n::i18n("Restart later"));
                                done.add_response("quit", &crate::i18n::i18n("Quit now"));
                                done.set_response_appearance(
                                    "quit",
                                    adw::ResponseAppearance::Suggested,
                                );
                                done.set_default_response(Some("later"));
                                done.set_close_response("later");
                                let win_for_quit = win_inner.clone();
                                done.connect_response(Some("quit"), move |_, _| {
                                    let _ = gtk4::prelude::WidgetExt::activate_action(
                                        &win_for_quit,
                                        "app.quit",
                                        None,
                                    );
                                });
                                done.present(Some(win_inner.upcast_ref::<gtk4::Widget>()));
                            }
                            Err(e) => {
                                tracing::error!(?e, "Settings restore failed");
                                crate::alert::show_error(
                                    &win_inner,
                                    &crate::i18n::i18n("Restore failed"),
                                    &e.to_string(),
                                );
                            }
                        },
                        Err(e) => {
                            tracing::error!(?e, "Cannot create ConfigManager for restore");
                            crate::alert::show_error(
                                &win_inner,
                                &crate::i18n::i18n("Restore failed"),
                                &crate::i18n::i18n_f(
                                    "Cannot access the configuration directory: {}",
                                    &[&e.to_string()],
                                ),
                            );
                        }
                    }
                });

                let widget = win_clone.upcast_ref::<gtk4::Widget>();
                confirm.present(Some(widget));
            });
        });

        tracing::debug!(
            total_ms = build_started.elapsed().as_millis() as u64,
            "settings dialog built"
        );

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
            copy_on_select_check,
            show_scrollbar_check,
            local_shell_command_entry,
            close_on_clean_exit_check,
            option_is_meta_check,
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
            show_protocol_filters,
            sidebar_width_row,
            compact_ui,
            compact_auto,
            terminal_passthrough_ctrl,
            window_title_shows_connection,
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
            keybindings_accel_labels,
            keybindings_page,
            highlight_rules_list,
            highlight_rules,
            cloud_sync_widgets,
            settings,
            was_restored,
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

    /// Populates the Cloud Sync "Synced Groups" and "Available in Cloud" sections.
    ///
    /// Call this after `set_settings()` with the current groups and sync manager.
    pub fn populate_cloud_sync(
        &self,
        groups: &[rustconn_core::models::ConnectionGroup],
        sync_manager: &rustconn_core::sync::SyncManager,
        state: &crate::state::SharedAppState,
    ) {
        use rustconn_core::sync::settings::SyncMode;

        // Populate synced groups
        for group in groups {
            if group.sync_mode == SyncMode::None {
                continue;
            }
            add_synced_group_row(
                &self.cloud_sync_widgets.synced_groups_group,
                &group.name,
                match group.sync_mode {
                    SyncMode::Master => "master",
                    SyncMode::Import => "import",
                    SyncMode::None => "none",
                },
            );
        }

        // Populate available files with Import button
        if let Ok(files) = sync_manager.list_available_sync_files() {
            // Filter out files already imported by any group
            let imported_files: std::collections::HashSet<String> =
                groups.iter().filter_map(|g| g.sync_file.clone()).collect();

            for file_path in &files {
                if let Some(filename) = file_path.file_name().and_then(|f| f.to_str())
                    && !imported_files.contains(filename)
                {
                    let state_clone = state.clone();
                    add_available_file_row(
                        &self.cloud_sync_widgets.available_files_group,
                        filename,
                        move |fname| {
                            Self::import_cloud_file(&state_clone, fname);
                        },
                    );
                }
            }
        }
    }

    /// Imports a `.rcn` file from the sync directory by creating an Import group
    /// and triggering an immediate sync.
    fn import_cloud_file(state: &crate::state::SharedAppState, filename: &str) {
        use rustconn_core::sync::settings::SyncMode;

        let Ok(mut state_mut) = state.try_borrow_mut() else {
            tracing::warn!("Could not borrow state for cloud file import");
            return;
        };

        // Derive group name from filename (strip .rcn extension)
        let group_name = filename
            .strip_suffix(".rcn")
            .unwrap_or(filename)
            .to_string();

        // Create a new Import group
        let group_id = match state_mut.create_group(group_name) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, filename, "Failed to create group for cloud import");
                return;
            }
        };

        // Configure the group for Import sync
        if let Some(group) = state_mut.get_group(group_id).cloned() {
            let mut updated = group;
            updated.sync_mode = SyncMode::Import;
            updated.sync_file = Some(filename.to_owned());
            if let Err(e) = state_mut
                .connection_manager()
                .update_group(group_id, updated)
            {
                tracing::warn!(error = %e, "Failed to configure import group");
                return;
            }
        }

        // Trigger immediate sync to import connections
        match state_mut.sync_now_group(group_id) {
            Ok(report) => {
                tracing::info!(
                    filename,
                    added = report.connections_added,
                    updated = report.connections_updated,
                    "Cloud file imported successfully"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, filename, "Failed to import cloud file");
            }
        }
    }

    /// Shows the dialog and loads current settings
    pub fn run<F>(&self, parent: Option<&impl IsA<gtk4::Widget>>, callback: F)
    where
        F: Fn(Option<AppSettings>) + 'static,
    {
        // Present the dialog immediately so the window appears without delay.
        // Settings loading and async operations populate widgets afterwards.
        let present_at = std::time::Instant::now();
        self.dialog.connect_map(move |dialog| {
            tracing::debug!(
                elapsed_ms = present_at.elapsed().as_millis() as u64,
                "settings dialog mapped after present"
            );
            // One-shot frame-clock callback: logs when the first frame after
            // map is actually drawn — the moment the dialog becomes visible.
            // Distinguishes "GTK mapped the widget" from "compositor showed
            // it" when users report multi-second delays (flatpak).
            dialog.add_tick_callback(move |_, _| {
                tracing::debug!(
                    elapsed_ms = present_at.elapsed().as_millis() as u64,
                    "settings dialog first frame after present"
                );
                gtk4::glib::ControlFlow::Break
            });
        });
        self.dialog.present(parent);
        tracing::debug!(
            elapsed_ms = present_at.elapsed().as_millis() as u64,
            "settings dialog present() returned"
        );
        let load_at = std::time::Instant::now();

        // Setup close handler - auto-save on close for PreferencesDialog
        let callback_rc = Rc::new(callback);
        self.setup_close_handler(callback_rc);

        // Load settings into UI (sync widget properties + async background tasks)
        let settings = self.settings.borrow().clone();
        self.load_settings(&settings);
        tracing::debug!(
            elapsed_ms = load_at.elapsed().as_millis() as u64,
            "settings dialog load_settings done"
        );

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
            &self.copy_on_select_check,
            &self.show_scrollbar_check,
            &self.local_shell_command_entry,
            &self.close_on_clean_exit_check,
            &self.option_is_meta_check,
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
            &self.show_protocol_filters,
            &self.sidebar_width_row,
            &self.compact_ui,
            &self.compact_auto,
            &self.terminal_passthrough_ctrl,
            &self.window_title_shows_connection,
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
            &self.keybindings_accel_labels,
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

        // Load Cloud Sync settings
        if let Some(ref sync_dir) = settings.sync.sync_dir {
            self.cloud_sync_widgets
                .sync_dir_row
                .set_text(&sync_dir.to_string_lossy());
        }
        self.cloud_sync_widgets
            .device_name_row
            .set_text(&settings.sync.device_name);
        self.cloud_sync_widgets
            .simple_sync_row
            .set_active(settings.sync.simple_sync_enabled);
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
        let copy_on_select_check_clone = self.copy_on_select_check.clone();
        let show_scrollbar_check_clone = self.show_scrollbar_check.clone();
        let local_shell_command_entry_clone = self.local_shell_command_entry.clone();
        let close_on_clean_exit_check_clone = self.close_on_clean_exit_check.clone();
        let option_is_meta_check_clone = self.option_is_meta_check.clone();

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
        let kdbx_storage_combo_clone = self.secrets_widgets.kdbx_storage_combo.clone();
        let kdbx_key_file_entry_clone = self.secrets_widgets.kdbx_key_file_entry.clone();
        let kdbx_use_key_file_check_clone = self.secrets_widgets.kdbx_use_key_file_check.clone();
        let kdbx_use_password_check_clone = self.secrets_widgets.kdbx_use_password_check.clone();
        let bitwarden_password_entry_clone = self.secrets_widgets.bitwarden_password_entry.clone();
        let bitwarden_storage_combo_clone = self.secrets_widgets.bitwarden_storage_combo.clone();
        let bitwarden_use_api_key_check_clone =
            self.secrets_widgets.bitwarden_use_api_key_check.clone();
        let bitwarden_client_id_entry_clone =
            self.secrets_widgets.bitwarden_client_id_entry.clone();
        let bitwarden_client_secret_entry_clone =
            self.secrets_widgets.bitwarden_client_secret_entry.clone();
        // Passbolt, 1Password, Pass — collect-only widgets cloned for the
        // close-time settings collection.
        let passbolt_passphrase_entry_clone =
            self.secrets_widgets.passbolt_passphrase_entry.clone();
        let passbolt_storage_combo_clone = self.secrets_widgets.passbolt_storage_combo.clone();
        let passbolt_server_url_entry_clone =
            self.secrets_widgets.passbolt_server_url_entry.clone();
        let onepassword_token_entry_clone = self.secrets_widgets.onepassword_token_entry.clone();
        let onepassword_storage_combo_clone =
            self.secrets_widgets.onepassword_storage_combo.clone();
        let pass_store_dir_entry_clone = self.secrets_widgets.pass_store_dir_entry.clone();

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
        let show_protocol_filters_clone = self.show_protocol_filters.clone();
        let sidebar_width_row_clone = self.sidebar_width_row.clone();
        let compact_ui_clone = self.compact_ui.clone();
        let compact_auto_clone = self.compact_auto.clone();
        let terminal_passthrough_ctrl_clone = self.terminal_passthrough_ctrl.clone();
        let window_title_shows_connection_clone = self.window_title_shows_connection.clone();
        let connections_clone = self.connections.clone();
        let keybindings_overrides_clone = self.keybindings_overrides.clone();

        // Monitoring controls
        let monitoring_widgets_clone = self.monitoring_widgets.clone();

        // Highlight rules
        let highlight_rules_clone = self.highlight_rules.clone();

        // SSH agent custom socket entry
        let ssh_agent_custom_socket_entry_clone = self.ssh_agent_custom_socket_entry.clone();

        // Cloud Sync controls
        let cloud_sync_dir_row_clone = self.cloud_sync_widgets.sync_dir_row.clone();
        let cloud_sync_device_name_clone = self.cloud_sync_widgets.device_name_row.clone();
        let cloud_sync_simple_sync_clone = self.cloud_sync_widgets.simple_sync_row.clone();

        // Store callback reference
        let on_save_callback = self.on_save.clone();

        // was_restored flag — skip saving if a restore was performed
        let was_restored_clone = self.was_restored.clone();

        // PreferencesDialog uses connect_closed signal (not connect_close_request)
        self.dialog.connect_closed(move |_| {
            // Ensure accelerators are restored in case recording was in progress
            // when the dialog was closed (e.g. via Escape on the dialog level).
            if let Some(app) = gtk4::gio::Application::default()
                .and_then(|a| a.downcast::<gtk4::Application>().ok())
            {
                keybindings_tab::restore_accels_with_overrides(
                    &app,
                    &keybindings_overrides_clone.borrow(),
                );
            }

            // If settings were restored from backup, skip saving — the UI widgets
            // still show pre-restore values and would overwrite the restored file.
            if was_restored_clone.get() {
                tracing::info!("Settings restored from backup — skipping save on close");
                external_callback(None);
                return;
            }

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
                &copy_on_select_check_clone,
                &show_scrollbar_check_clone,
                &local_shell_command_entry_clone,
                &close_on_clean_exit_check_clone,
                &option_is_meta_check_clone,
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
                kdbx_storage_combo: kdbx_storage_combo_clone.clone(),
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
                password_row: adw::PasswordEntryRow::new(), // dummy
                key_file_row: adw::ActionRow::new(),      // dummy
                bitwarden_group: adw::PreferencesGroup::new(), // dummy
                bitwarden_status_label: Label::new(None), // dummy
                bitwarden_unlock_button: Button::new(),   // dummy
                bitwarden_password_entry: bitwarden_password_entry_clone.clone(),
                bitwarden_storage_combo: bitwarden_storage_combo_clone.clone(),
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
                passbolt_server_url_entry: passbolt_server_url_entry_clone.clone(),
                passbolt_open_vault_button: Button::new(), // dummy, не використовується при збиранні
                passbolt_passphrase_entry: passbolt_passphrase_entry_clone.clone(),
                passbolt_storage_combo: passbolt_storage_combo_clone.clone(),
                onepassword_token_entry: onepassword_token_entry_clone.clone(),
                onepassword_storage_combo: onepassword_storage_combo_clone.clone(),
                secret_tool_available: Rc::new(RefCell::new(None)), // dummy, не використовується при збиранні
                pass_group: adw::PreferencesGroup::new(), // dummy, не використовується при збиранні
                pass_store_dir_entry: pass_store_dir_entry_clone.clone(),
                pass_store_dir_browse_button: Button::new(), // dummy, не використовується при збиранні
                pass_status_label: Label::new(None), // dummy, не використовується при збиранні
            };
            let secrets = collect_secret_settings(&secrets_widgets_for_collect, &settings_clone);

            // Collect UI settings
            let conn_list = connections_clone.borrow();
            let conn_refs: Vec<&Connection> = conn_list.iter().collect();
            let mut ui = collect_ui_settings(
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
                &show_protocol_filters_clone,
                &sidebar_width_row_clone,
                &compact_ui_clone,
                &compact_auto_clone,
                &terminal_passthrough_ctrl_clone,
                &window_title_shows_connection_clone,
                &conn_refs,
            );
            // Preserve smart folders visibility (managed by toolbar toggle, not settings dialog)
            ui.show_smart_folders = settings_clone.borrow().ui.show_smart_folders;
            // Preserve runtime-managed state the dialog does not edit, so saving
            // Preferences never wipes it (#202): window geometry/maximized,
            // expanded sidebar groups, and search history.
            {
                let cur = settings_clone.borrow();
                ui.window_width = cur.ui.window_width;
                ui.window_height = cur.ui.window_height;
                ui.window_maximized = cur.ui.window_maximized;
                ui.expanded_groups = cur.ui.expanded_groups.clone();
                ui.search_history = cur.ui.search_history.clone();
                // A file-based startup action (an RDP/.vv file passed on the CLI)
                // has no dropdown slot — it shares index 0 with "Do nothing" — so
                // a Settings round-trip would otherwise collapse it to None.
                // Preserve it when the dropdown was left on that slot.
                if matches!(
                    ui.startup_action,
                    rustconn_core::config::StartupAction::None
                ) && matches!(
                    cur.ui.startup_action,
                    rustconn_core::config::StartupAction::RdpFile(_)
                        | rustconn_core::config::StartupAction::VvFile(_)
                ) {
                    ui.startup_action = cur.ui.startup_action.clone();
                }
            }
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
                sync: {
                    let mut sync_settings = settings_clone.borrow().sync.clone();
                    let dir_text = cloud_sync_dir_row_clone.text();
                    let dir_trimmed = dir_text.trim();
                    sync_settings.sync_dir = if dir_trimmed.is_empty() {
                        None
                    } else {
                        Some(std::path::PathBuf::from(dir_trimmed.to_string()))
                    };
                    let name_text = cloud_sync_device_name_clone.text();
                    let name_trimmed = name_text.trim();
                    if !name_trimmed.is_empty() {
                        name_trimmed.clone_into(&mut sync_settings.device_name);
                    }
                    sync_settings.simple_sync_enabled = cloud_sync_simple_sync_clone.is_active();
                    sync_settings
                },
                standalone_tunnels: settings_clone.borrow().standalone_tunnels.clone(),
                quick_connect_history: settings_clone.borrow().quick_connect_history.clone(),
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
            let row =
                build_highlight_rule_row(rule, &self.highlight_rules_list, &self.highlight_rules);
            self.highlight_rules_list.append(&row);
        }
    }

    /// Returns a reference to the dialog for toast notifications
    pub fn dialog(&self) -> &adw::PreferencesDialog {
        &self.dialog
    }
}

/// Builds a fully wired highlight-rule row: an `ExpanderRow` titled by the
/// rule name (pattern as subtitle), with an enable switch and delete button
/// as suffixes and `EntryRow`s for name/pattern inside.
fn build_highlight_rule_row(
    rule: &rustconn_core::models::HighlightRule,
    list: &gtk4::ListBox,
    rules: &Rc<RefCell<Vec<rustconn_core::models::HighlightRule>>>,
) -> adw::ExpanderRow {
    let rule_id = rule.id;
    let untitled = i18n("Untitled rule");

    let row = adw::ExpanderRow::builder()
        .show_enable_switch(false)
        .build();
    // Title/subtitle reflect user text — never interpret it as Pango markup.
    row.set_use_markup(false);
    row.set_title(if rule.name.is_empty() {
        &untitled
    } else {
        &rule.name
    });
    row.set_subtitle(&rule.pattern);

    let enabled_switch = gtk4::Switch::builder()
        .active(rule.enabled)
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Enable rule"))
        .build();
    enabled_switch.update_property(&[gtk4::accessible::Property::Label(&i18n("Enable rule"))]);
    {
        let rules_clone = rules.clone();
        enabled_switch.connect_active_notify(move |s| {
            let mut r = rules_clone.borrow_mut();
            if let Some(rule) = r.iter_mut().find(|r| r.id == rule_id) {
                rule.enabled = s.is_active();
            }
        });
    }
    row.add_suffix(&enabled_switch);

    let delete_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Delete rule"))
        .valign(gtk4::Align::Center)
        .build();
    delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Delete highlight rule",
    ))]);
    {
        let list_clone = list.clone();
        let rules_clone = rules.clone();
        let row_weak = row.downgrade();
        delete_button.connect_clicked(move |_| {
            if let Some(row) = row_weak.upgrade() {
                list_clone.remove(&row);
            }
            rules_clone.borrow_mut().retain(|r| r.id != rule_id);
        });
    }
    row.add_suffix(&delete_button);

    let name_row = adw::EntryRow::builder().title(i18n("Name")).build();
    name_row.set_text(&rule.name);
    {
        let rules_clone = rules.clone();
        let row_weak = row.downgrade();
        let untitled = untitled.clone();
        name_row.connect_changed(move |e| {
            let text = e.text().to_string();
            if let Some(row) = row_weak.upgrade() {
                row.set_title(if text.is_empty() { &untitled } else { &text });
            }
            let mut r = rules_clone.borrow_mut();
            if let Some(rule) = r.iter_mut().find(|r| r.id == rule_id) {
                rule.name = text;
            }
        });
    }
    row.add_row(&name_row);

    let pattern_row = adw::EntryRow::builder()
        .title(i18n("Pattern (regex)"))
        .build();
    pattern_row.set_text(&rule.pattern);
    {
        let rules_clone = rules.clone();
        let row_weak = row.downgrade();
        pattern_row.connect_changed(move |e| {
            let text = e.text().to_string();
            if let Some(row) = row_weak.upgrade() {
                row.set_subtitle(&text);
            }
            let mut r = rules_clone.borrow_mut();
            if let Some(rule) = r.iter_mut().find(|r| r.id == rule_id) {
                rule.pattern = text;
            }
        });
    }
    row.add_row(&pattern_row);

    row
}
