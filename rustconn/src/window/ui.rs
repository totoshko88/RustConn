//! Window UI components
//!
//! This module contains UI creation functions for the main window,
//! including header bar and application menu construction.

use crate::i18n::i18n;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{Button, Label, MenuButton};
use libadwaita as adw;

/// Creates the header bar with title and controls
///
/// Layout:
/// - Left side (pack_start): Quick Connect, Add, Remove, Add Group
/// - Center: Title + Spinner
/// - Right side (pack_end): Menu, Settings, Split Vertical, Split Horizontal
///
/// Returns the header bar and the busy spinner widget (initially hidden).
#[must_use]
pub fn create_header_bar() -> (adw::HeaderBar, gtk4::Spinner) {
    let header_bar = adw::HeaderBar::new();

    // Title area: label + spinner in a horizontal box
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    title_box.set_halign(gtk4::Align::Center);
    title_box.set_valign(gtk4::Align::Center);

    let title = Label::new(Some("RustConn"));
    title.add_css_class("title");
    title_box.append(&title);

    let busy_spinner = gtk4::Spinner::new();
    busy_spinner.set_visible(false);
    busy_spinner.set_tooltip_text(Some(&i18n("Operation in progress")));
    busy_spinner.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Operation in progress",
    ))]);
    title_box.append(&busy_spinner);

    header_bar.set_title_widget(Some(&title_box));

    // === Left side (pack_start) - Primary connection actions ===
    // Order: Sidebar Toggle, Quick Connect, Add, Remove, Add Group

    // Sidebar toggle button
    let sidebar_toggle = Button::from_icon_name("sidebar-show-symbolic");
    sidebar_toggle.set_tooltip_text(Some(&i18n("Toggle Sidebar (F9)")));
    sidebar_toggle.set_action_name(Some("win.toggle-sidebar"));
    sidebar_toggle.update_property(&[gtk4::accessible::Property::Label(&i18n("Toggle Sidebar"))]);
    header_bar.pack_start(&sidebar_toggle);

    // Quick connect button
    let quick_connect_button = Button::from_icon_name("go-jump-symbolic");
    quick_connect_button.set_tooltip_text(Some(&i18n("Quick Connect (Ctrl+Shift+Q)")));
    quick_connect_button.set_action_name(Some("win.quick-connect"));
    quick_connect_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Quick Connect"))]);
    header_bar.pack_start(&quick_connect_button);

    // Add connection button
    let add_button = Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some(&i18n("New Connection (Ctrl+N)")));
    add_button.set_action_name(Some("win.new-connection"));
    add_button.update_property(&[gtk4::accessible::Property::Label(&i18n("New Connection"))]);
    header_bar.pack_start(&add_button);

    // Remove button (sensitive only when item selected)
    let remove_button = Button::from_icon_name("list-remove-symbolic");
    remove_button.set_tooltip_text(Some(&i18n("Delete Selected (Delete)")));
    remove_button.set_action_name(Some("win.delete-connection"));
    remove_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete Selected"))]);
    header_bar.pack_start(&remove_button);

    // Add group button
    let add_group_button = Button::from_icon_name("folder-new-symbolic");
    add_group_button.set_tooltip_text(Some(&i18n("New Group (Ctrl+Shift+N)")));
    add_group_button.set_action_name(Some("win.new-group"));
    add_group_button.update_property(&[gtk4::accessible::Property::Label(&i18n("New Group"))]);
    header_bar.pack_start(&add_group_button);

    // === Right side (pack_end) - Secondary actions ===

    // Add menu button (rightmost)
    let menu_button = MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(i18n("Menu (F10)"))
        .build();
    menu_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Menu"))]);

    let menu = create_app_menu();
    menu_button.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_button);

    // Add settings button
    let settings_button = Button::from_icon_name("preferences-system-symbolic");
    settings_button.set_tooltip_text(Some(&i18n("Settings (Ctrl+,)")));
    settings_button.set_action_name(Some("win.settings"));
    settings_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Settings"))]);
    header_bar.pack_end(&settings_button);

    // Add split view buttons
    let split_vertical_button = Button::from_icon_name("object-flip-horizontal-symbolic");
    split_vertical_button.set_tooltip_text(Some(&i18n("Split Vertical (Ctrl+Shift+S)")));
    split_vertical_button.set_action_name(Some("win.split-vertical"));
    split_vertical_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Split Vertical"))]);
    header_bar.pack_end(&split_vertical_button);

    let split_horizontal_button = Button::from_icon_name("object-flip-vertical-symbolic");
    split_horizontal_button.set_tooltip_text(Some(&i18n("Split Horizontal (Ctrl+Shift+H)")));
    split_horizontal_button.set_action_name(Some("win.split-horizontal"));
    split_horizontal_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Split Horizontal"))]);
    header_bar.pack_end(&split_horizontal_button);

    // Shell button — prominent, icon + label, accent color, leftmost in right group
    let shell_button = Button::new();
    let shell_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let shell_icon = gtk4::Image::from_icon_name("utilities-terminal-symbolic");
    shell_icon.set_pixel_size(16);
    let shell_label = Label::new(Some(&i18n("Shell")));
    shell_box.append(&shell_icon);
    shell_box.append(&shell_label);
    shell_button.set_child(Some(&shell_box));
    shell_button.set_tooltip_text(Some(&i18n("Local Shell (Ctrl+Shift+T)")));
    shell_button.set_action_name(Some("win.local-shell"));
    shell_button.add_css_class("suggested-action");
    shell_button.add_css_class("pill");
    shell_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Open Local Shell"))]);
    header_bar.pack_end(&shell_button);

    (header_bar, busy_spinner)
}

/// Creates the application menu
///
/// Menu sections:
/// 1. Connections: New Connection, New Group, Quick Connect, Local Shell
/// 2. Tools: Snippets, Clusters, Templates, Variables, Sessions, History, Statistics,
///    Password Generator, Wake On LAN, Recordings, SSH Tunnels
/// 3. File: Import, Export
/// 4. Edit: Copy Connection, Paste Connection
/// 5. App: Settings, Flatpak Components (if Flatpak), Keyboard Shortcuts, About, Quit
#[must_use]
pub fn create_app_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // Connections section
    let conn_section = gio::Menu::new();
    conn_section.append(Some(&i18n("New Connection")), Some("win.new-connection"));
    conn_section.append(
        Some(&i18n("New Connection (Advanced)\u{2026}")),
        Some("win.new-connection-advanced"),
    );
    conn_section.append(Some(&i18n("New Group")), Some("win.new-group"));
    conn_section.append(Some(&i18n("Quick Connect")), Some("win.quick-connect"));
    conn_section.append(Some(&i18n("Local Shell")), Some("win.local-shell"));
    menu.append_section(None, &conn_section);

    // Tools section — Managers
    let managers_section = gio::Menu::new();
    managers_section.append(Some(&i18n("Snippets...")), Some("win.manage-snippets"));
    managers_section.append(Some(&i18n("Clusters...")), Some("win.manage-clusters"));
    managers_section.append(Some(&i18n("Templates...")), Some("win.manage-templates"));
    managers_section.append(Some(&i18n("Variables...")), Some("win.manage-variables"));
    menu.append_section(None, &managers_section);

    // Tools section — Monitoring & History
    let monitoring_section = gio::Menu::new();
    monitoring_section.append(Some(&i18n("Active Sessions...")), Some("win.show-sessions"));
    monitoring_section.append(
        Some(&i18n("Connection History...")),
        Some("win.show-history"),
    );
    monitoring_section.append(Some(&i18n("Statistics...")), Some("win.show-statistics"));
    monitoring_section.append(Some(&i18n("Recordings...")), Some("win.manage-recordings"));
    menu.append_section(None, &monitoring_section);

    // Tools section — Security & Network
    let security_section = gio::Menu::new();
    security_section.append(
        Some(&i18n("Password Generator...")),
        Some("win.password-generator"),
    );
    security_section.append(
        Some(&i18n("Wake On LAN...")),
        Some("win.wake-on-lan-dialog"),
    );
    security_section.append(Some(&i18n("SSH Tunnels...")), Some("win.ssh-tunnels"));
    menu.append_section(None, &security_section);

    // File section (import/export connections)
    let file_section = gio::Menu::new();
    file_section.append(Some(&i18n("Import Connections...")), Some("win.import"));
    file_section.append(Some(&i18n("Export Connections...")), Some("win.export"));
    menu.append_section(None, &file_section);

    // Edit section
    let edit_section = gio::Menu::new();
    edit_section.append(Some(&i18n("Copy Connection")), Some("win.copy-connection"));
    edit_section.append(
        Some(&i18n("Paste Connection")),
        Some("win.paste-connection"),
    );
    menu.append_section(None, &edit_section);

    // Settings section (separated from app meta per GNOME HIG)
    let settings_section = gio::Menu::new();
    settings_section.append(Some(&i18n("Settings...")), Some("win.settings"));
    // Flatpak Components menu item - only visible in Flatpak environment
    // The action is always registered but does nothing outside Flatpak
    if rustconn_core::flatpak::is_flatpak() {
        settings_section.append(
            Some(&i18n("Flatpak Components...")),
            Some("win.flatpak-components"),
        );
    }
    menu.append_section(None, &settings_section);

    // App meta section (GNOME HIG: Fullscreen, Shortcuts, About, Quit)
    let app_section = gio::Menu::new();
    app_section.append(Some(&i18n("Fullscreen")), Some("win.toggle-fullscreen"));
    app_section.append(Some(&i18n("Keyboard Shortcuts...")), Some("app.shortcuts"));
    app_section.append(Some(&i18n("About RustConn")), Some("app.about"));
    app_section.append(Some(&i18n("Quit")), Some("app.quit"));
    menu.append_section(None, &app_section);

    menu
}
