//! Cloud Sync preferences page for the Settings dialog.
//!
//! Implements the `AdwPreferencesPage` "Cloud Sync" with four groups:
//! - Setup: sync directory and device name
//! - Synced Groups: list of groups with sync mode
//! - Available in Cloud: unimported `.rcn` files
//! - Simple Sync: toggle for bidirectional sync

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::i18n::i18n;

/// Widgets from the Cloud Sync preferences page that need to be accessed
/// for loading/saving settings.
pub struct CloudSyncPageWidgets {
    /// The preferences page itself.
    pub page: adw::PreferencesPage,
    /// Sync directory entry row.
    pub sync_dir_row: adw::EntryRow,
    /// Device name entry row.
    pub device_name_row: adw::EntryRow,
    /// Container for synced group rows (dynamically populated).
    pub synced_groups_group: adw::PreferencesGroup,
    /// Container for available cloud file rows (dynamically populated).
    pub available_files_group: adw::PreferencesGroup,
    /// Simple Sync toggle.
    pub simple_sync_row: adw::SwitchRow,
}

/// Creates the Cloud Sync `AdwPreferencesPage`.
///
/// Returns the page and its interactive widgets for later data binding.
#[must_use]
pub fn create_cloud_sync_page() -> CloudSyncPageWidgets {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Cloud Sync"))
        .icon_name("emblem-synchronizing-symbolic")
        .build();

    // --- Setup group ---
    let setup_group = adw::PreferencesGroup::builder()
        .title(i18n("Setup"))
        .build();

    let sync_dir_row = adw::EntryRow::builder()
        .title(i18n("Sync Directory"))
        .show_apply_button(true)
        .build();

    // File chooser suffix button for sync directory
    let dir_button = gtk4::Button::from_icon_name("folder-open-symbolic");
    dir_button.set_valign(gtk4::Align::Center);
    dir_button.set_tooltip_text(Some(&i18n("Choose sync directory")));
    dir_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Choose sync directory",
    ))]);
    sync_dir_row.add_suffix(&dir_button);

    // Wire up file chooser button
    let sync_dir_row_clone = sync_dir_row.clone();
    dir_button.connect_clicked(move |btn| {
        let Some(root) = btn.root() else { return };
        let Some(win) = root.downcast_ref::<gtk4::Window>() else {
            return;
        };
        let file_dialog = gtk4::FileDialog::builder()
            .title(i18n("Select Sync Directory"))
            .build();
        let row_clone = sync_dir_row_clone.clone();
        let win_clone = win.clone();
        file_dialog.select_folder(Some(win), None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(file) = result
                && let Some(path) = file.path()
            {
                if rustconn_core::flatpak::is_flatpak()
                    && rustconn_core::flatpak::is_portal_path(&path)
                {
                    show_flatpak_sync_dir_portal_warning(&win_clone, &path);
                } else {
                    row_clone.set_text(&path.to_string_lossy());
                }
            }
        });
    });

    setup_group.add(&sync_dir_row);

    // Validate manually entered paths for Flatpak portal paths.
    // The FileDialog check above only catches paths selected via the chooser;
    // this handles direct text entry via the "Apply" button.
    sync_dir_row.connect_apply(move |row| {
        if rustconn_core::flatpak::is_flatpak() {
            let text = row.text();
            let path = std::path::Path::new(text.as_str());
            if rustconn_core::flatpak::is_portal_path(path) {
                // Clear the invalid path and show warning
                row.set_text("");
                if let Some(root) = row.root()
                    && let Some(win) = root.downcast_ref::<gtk4::Window>()
                {
                    show_flatpak_sync_dir_portal_warning(win, path);
                }
            }
        }
    });

    let device_name_row = adw::EntryRow::builder()
        .title(i18n("Device Name"))
        .show_apply_button(true)
        .build();
    setup_group.add(&device_name_row);

    page.add(&setup_group);

    // --- Synced Groups group ---
    let synced_groups_group = adw::PreferencesGroup::builder()
        .title(i18n("Synced Groups"))
        .build();
    page.add(&synced_groups_group);

    // --- Available in Cloud group ---
    let available_files_group = adw::PreferencesGroup::builder()
        .title(i18n("Available in Cloud"))
        .description(i18n("Files in sync directory not yet imported"))
        .build();
    page.add(&available_files_group);

    // --- Simple Sync group ---
    let simple_sync_group = adw::PreferencesGroup::builder()
        .title(i18n("Simple Sync"))
        .build();

    let simple_sync_row = adw::SwitchRow::builder()
        .title(i18n("Sync everything between your devices"))
        .build();
    simple_sync_group.add(&simple_sync_row);

    page.add(&simple_sync_group);

    CloudSyncPageWidgets {
        page,
        sync_dir_row,
        device_name_row,
        synced_groups_group,
        available_files_group,
        simple_sync_row,
    }
}

/// Adds a synced group row to the "Synced Groups" preferences group.
///
/// Displays the group name with a subtitle indicating its sync mode
/// (e.g., "Master · synced" or "Import · synced").
pub fn add_synced_group_row(group: &adw::PreferencesGroup, name: &str, sync_mode: &str) {
    let subtitle = match sync_mode {
        "master" => i18n("Master · synced"),
        "import" => i18n("Import · synced"),
        _ => i18n("Sync error"),
    };

    let row = adw::ActionRow::builder()
        .title(name)
        .subtitle(&subtitle)
        .build();
    group.add(&row);
}

/// Adds an available cloud file row with an "Import" suffix button.
///
/// The `on_import` callback is invoked with the filename when the user clicks "Import".
pub fn add_available_file_row(
    group: &adw::PreferencesGroup,
    filename: &str,
    on_import: impl Fn(&str) + 'static,
) {
    let row = adw::ActionRow::builder()
        .title(filename)
        .subtitle(&i18n("Available for import"))
        .build();

    let import_btn = gtk4::Button::builder()
        .label(i18n("Import"))
        .valign(gtk4::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    import_btn.set_tooltip_text(Some(&i18n("Import this file as a new group")));
    import_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Import sync file"))]);

    let filename_owned = filename.to_owned();
    import_btn.connect_clicked(move |_| {
        on_import(&filename_owned);
    });

    row.add_suffix(&import_btn);
    group.add(&row);
}

/// Shows a warning dialog when the user selects a sync directory that resolves
/// to a Flatpak document portal path.
///
/// Portal paths (`/run/user/<uid>/doc/<hash>/...`) are temporary FUSE mounts
/// that don't support inotify and become stale after app restart. The user
/// must grant direct filesystem access via `flatpak override` instead.
pub fn show_flatpak_sync_dir_portal_warning(parent: &gtk4::Window, portal_path: &std::path::Path) {
    use crate::i18n::i18n_f;

    let app_id = crate::app::APP_ID;

    // Try to extract a human-readable hint from the portal path.
    // Portal paths look like /run/user/1000/doc/<hash>/<dirname>
    // The last component is usually the original directory name.
    let dir_hint = portal_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Build the filesystem argument for the flatpak override command.
    // We use the directory name as a hint with ~/... prefix, but since
    // portal paths don't preserve the full original path, the user may
    // need to adjust it (e.g. ~/Dropbox/rustconn-sync instead of ~/rustconn-sync).
    let filesystem_arg = if dir_hint.is_empty() {
        "/path/to/your/sync/directory".to_string()
    } else {
        format!("~/{dir_hint}")
    };

    let body = i18n_f(
        "The Flatpak sandbox cannot directly access this directory. File monitoring will not work through the document portal.\n\nTo grant access, run in a terminal:\n\nflatpak override --user --filesystem={} {}\n\nAdjust the path if needed to match the actual location on your filesystem. Then restart RustConn and set the sync directory again.",
        &[&filesystem_arg, app_id],
    );

    let dialog = adw::AlertDialog::new(
        Some(&i18n("Flatpak: filesystem access required")),
        Some(&body),
    );
    dialog.add_response("close", &i18n("Close"));
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");

    // Add a "Copy Command" button for convenience
    dialog.add_response("copy", &i18n("Copy Command"));
    dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);

    let cmd = format!("flatpak override --user --filesystem={filesystem_arg} {app_id}");
    let parent_clone = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "copy" {
            let display = gtk4::prelude::WidgetExt::display(&parent_clone);
            display.clipboard().set_text(&cmd);
        }
    });

    dialog.present(Some(parent));
}
